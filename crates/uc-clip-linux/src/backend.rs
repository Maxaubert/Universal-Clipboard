//! The public clipboard type and the small trait both backends implement.

use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use uc_clip::{Clipboard, ClipboardError, RawPart, Snapshot};

use crate::BackendKind;
use crate::mime::{self, Item};
use crate::session::{Candidate, Detection};

/// How long a read of one type may take before we give up on the owner.
pub(crate) const READ_TIMEOUT: Duration = Duration::from_secs(2);

/// How long a write waits for the change event of its own selection.
pub(crate) const WRITE_CONFIRM: Duration = Duration::from_secs(2);

/// Wait until `fd` is readable or `left` has passed. True when it is readable (or closed).
pub(crate) fn poll_in(
    fd: impl std::os::fd::AsFd,
    left: Duration,
) -> Result<bool, rustix::io::Errno> {
    let ts = rustix::event::Timespec::try_from(left).unwrap_or(rustix::event::Timespec {
        tv_sec: 1,
        tv_nsec: 0,
    });
    let mut fds = [rustix::event::PollFd::new(
        &fd,
        rustix::event::PollFlags::IN,
    )];
    match rustix::event::poll(&mut fds, Some(&ts)) {
        Ok(n) => Ok(n > 0),
        Err(rustix::io::Errno::INTR) => Ok(false),
        Err(e) => Err(e),
    }
}

/// What a backend does. Every call comes from the one thread that owns the
/// [`LinuxClipboard`]; each backend has its own event thread inside.
pub(crate) trait Backend: Send {
    /// The change counter and the types offered by the current owner.
    fn offered(&mut self) -> Result<(u64, Vec<String>), ClipboardError>;
    /// Read one type. `Ok(None)` when the owner refused it or the clipboard is empty.
    fn fetch(&mut self, mime: &str) -> Result<Option<Vec<u8>>, ClipboardError>;
    /// Become the owner and serve `items`. Returns the change counter of this write.
    fn set(&mut self, items: Vec<Item>) -> Result<u64, ClipboardError>;
    /// The change notifications.
    fn events(&self) -> &Events;
}

/// The change counter and its subscribers, shared by a backend and its event thread.
#[derive(Default)]
pub(crate) struct Events {
    state: Mutex<EventState>,
    cv: Condvar,
}

#[derive(Default)]
struct EventState {
    seq: u64,
    subs: Vec<Sender<u64>>,
    dead: Option<String>,
}

impl Events {
    /// A new selection owner. Returns the new counter.
    pub(crate) fn bump(&self) -> u64 {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.seq += 1;
        let seq = s.seq;
        s.subs.retain(|tx| tx.send(seq).is_ok());
        self.cv.notify_all();
        seq
    }

    /// The event thread stopped (the display went away).
    pub(crate) fn kill(&self, why: String) {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.dead.get_or_insert(why);
        s.subs.clear();
        self.cv.notify_all();
    }

    pub(crate) fn seq(&self) -> Result<u64, ClipboardError> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        match &s.dead {
            Some(why) => Err(ClipboardError::Other(why.clone())),
            None => Ok(s.seq),
        }
    }

    /// Wait until the counter is above `before`. Returns the counter.
    pub(crate) fn wait_above(&self, before: u64, timeout: Duration) -> Result<u64, ClipboardError> {
        let deadline = Instant::now() + timeout;
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(why) = &s.dead {
                return Err(ClipboardError::Other(why.clone()));
            }
            if s.seq > before {
                return Ok(s.seq);
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ClipboardError::Other("no-change-event".into()));
            }
            s = self
                .cv
                .wait_timeout(s, deadline - now)
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }

    fn subscribe(&self) -> Receiver<u64> {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if s.dead.is_none() {
            s.subs.push(tx);
        }
        rx
    }
}

/// Which data-control protocol to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataControl {
    /// `ext-data-control-v1` (the standard one).
    Ext,
    /// `wlr-data-control-unstable-v1` (older wlroots and KDE).
    Wlr,
}

/// No backend could be opened. The app shows "This desktop blocks background clipboard
/// access." (plan 2.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenError {
    /// One short reason per backend tried, for the diagnostic report. Never content.
    pub tried: Vec<String>,
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "This desktop blocks background clipboard access. ({})",
            if self.tried.is_empty() {
                "no graphical session".to_owned()
            } else {
                self.tried.join("; ")
            }
        )
    }
}

impl std::error::Error for OpenError {}

/// The Linux clipboard. Implements [`Clipboard`].
///
/// Change events arrive on [`LinuxClipboard::subscribe`]; the agent waits the settle time
/// and then calls [`Clipboard::read`].
pub struct LinuxClipboard {
    backend: Box<dyn Backend>,
    kind: BackendKind,
}

impl LinuxClipboard {
    /// Open the right backend for this session (see [`crate::session`]).
    pub fn open() -> Result<Self, OpenError> {
        Self::open_detected(&crate::session::detect_from_env())
    }

    /// Open the first backend of `detection` that works.
    pub fn open_detected(detection: &Detection) -> Result<Self, OpenError> {
        let mut tried = Vec::new();
        for c in &detection.candidates {
            match c {
                Candidate::DataControl => {
                    for p in [DataControl::Ext, DataControl::Wlr] {
                        match Self::open_wayland(p) {
                            Ok(c) => return Ok(c),
                            Err(e) => tried.push(e.to_string()),
                        }
                    }
                }
                Candidate::X11 { xwayland } => match Self::open_x11(None) {
                    Ok(mut c) => {
                        if *xwayland {
                            c.kind = BackendKind::XWayland;
                        }
                        return Ok(c);
                    }
                    Err(e) => tried.push(e.to_string()),
                },
            }
        }
        Err(OpenError { tried })
    }

    /// Open the Wayland data-control backend on `$WAYLAND_DISPLAY`.
    pub fn open_wayland(protocol: DataControl) -> Result<Self, ClipboardError> {
        let (backend, kind): (Box<dyn Backend>, _) = match protocol {
            DataControl::Ext => (
                Box::new(crate::wayland::ext::Clip::open()?),
                BackendKind::ExtDataControl,
            ),
            DataControl::Wlr => (
                Box::new(crate::wayland::wlr::Clip::open()?),
                BackendKind::WlrDataControl,
            ),
        };
        Ok(Self { backend, kind })
    }

    /// Open the X11 backend on `display` (`None`: `$DISPLAY`).
    pub fn open_x11(display: Option<&str>) -> Result<Self, ClipboardError> {
        Ok(Self {
            backend: Box::new(crate::x11::Clip::open(display)?),
            kind: BackendKind::X11,
        })
    }

    /// The backend in use.
    pub fn kind(&self) -> BackendKind {
        self.kind
    }

    /// A new channel that receives the change counter on every new clipboard owner,
    /// including our own writes. It closes when the display goes away.
    pub fn subscribe(&self) -> Receiver<u64> {
        self.backend.events().subscribe()
    }

    /// The current change counter.
    pub fn seq(&self) -> Result<u64, ClipboardError> {
        self.backend.events().seq()
    }

    /// Put `items` (type, bytes) on the clipboard **without** our marker, as another app would.
    /// For tests and tools only; the agent uses [`Clipboard::write`].
    pub fn copy_raw(&mut self, items: &[(&str, &[u8])]) -> Result<u64, ClipboardError> {
        self.backend.set(mime::raw_items(items))
    }

    /// Read one type as it is, with no marker or private check. For tests and tools only.
    pub fn read_type(&mut self, mime: &str) -> Result<Option<Vec<u8>>, ClipboardError> {
        self.backend.fetch(mime)
    }

    /// The types offered by the current owner (for diagnostics and tests).
    pub fn offered(&mut self) -> Result<Vec<String>, ClipboardError> {
        Ok(self.backend.offered()?.1)
    }
}

impl Clipboard for LinuxClipboard {
    fn read(&mut self) -> Result<Snapshot, ClipboardError> {
        let (seq, offered) = self.backend.offered()?;
        let backend = &mut self.backend;
        Ok(mime::build_snapshot(seq, &offered, |m| backend.fetch(m)))
    }

    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError> {
        self.backend.set(mime::write_items(parts))
    }
}

impl std::fmt::Debug for LinuxClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxClipboard")
            .field("kind", &self.kind)
            .finish()
    }
}
