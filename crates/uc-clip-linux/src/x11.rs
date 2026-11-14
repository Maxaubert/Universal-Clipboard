//! X11 backend: XFIXES selection-owner events on `CLIPBOARD`, and a selection owner that
//! serves our writes (ICCCM, with `INCR` for large data).
//!
//! Two connections:
//! - the **owner** connection has a hidden window. Its thread receives the XFIXES events and
//!   answers `SelectionRequest`s for our writes;
//! - the **reader** connection has its own hidden window and is used only by the caller's
//!   thread to read the selection. So a read never waits on the owner thread, also when we
//!   read our own selection.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uc_clip::ClipboardError;
use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::xfixes::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _,
    CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
    SelectionRequestEvent, Timestamp, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, COPY_FROM_PARENT, CURRENT_TIME, NONE};

use crate::backend::{Backend, Events, READ_TIMEOUT, WRITE_CONFIRM, poll_in};
use crate::mime::{Item, MAX_READ};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        TIMESTAMP,
        INCR,
        UTF8_STRING,
        TEXT,
        UCLIP_SEL,
        UCLIP_WAKE,
    }
}

/// An `INCR` transfer older than this is dropped (the requestor went away).
const TRANSFER_TTL: Duration = Duration::from_secs(60);

fn other(code: &'static str) -> impl FnOnce(String) -> ClipboardError {
    move |detail| ClipboardError::Other(format!("{code}: {detail}"))
}

macro_rules! x11err {
    ($code:literal) => {
        |e| other($code)(e.to_string())
    };
}

/// What the owner thread serves.
#[derive(Default)]
struct Owner {
    /// Served targets and their bytes. Empty when we do not own the selection.
    items: Vec<(Atom, Arc<Vec<u8>>)>,
    /// The time we became owner (from the XFIXES event).
    time: Timestamp,
}

pub(crate) struct Clip {
    owner_conn: Arc<RustConnection>,
    owner_win: Window,
    owner: Arc<Mutex<Owner>>,
    events: Arc<Events>,
    stop: Arc<AtomicBool>,
    reader: Reader,
    names: HashMap<Atom, String>,
    ids: HashMap<String, Atom>,
}

impl Clip {
    pub(crate) fn open(display: Option<&str>) -> Result<Self, ClipboardError> {
        let (conn, screen) = x11rb::connect(display).map_err(x11err!("x11-connect"))?;
        if conn
            .extension_information(xfixes::X11_EXTENSION_NAME)
            .map_err(x11err!("x11-ext"))?
            .is_none()
        {
            return Err(ClipboardError::Other("x11-no-xfixes".into()));
        }
        conn.xfixes_query_version(5, 0)
            .map_err(x11err!("x11-xfixes"))?
            .reply()
            .map_err(x11err!("x11-xfixes"))?;
        let atoms = Atoms::new(&conn)
            .map_err(x11err!("x11-atoms"))?
            .reply()
            .map_err(x11err!("x11-atoms"))?;
        let win = hidden_window(&conn, screen)?;
        conn.xfixes_select_selection_input(
            win,
            atoms.CLIPBOARD,
            xfixes::SelectionEventMask::SET_SELECTION_OWNER
                | xfixes::SelectionEventMask::SELECTION_WINDOW_DESTROY
                | xfixes::SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )
        .map_err(x11err!("x11-select"))?;
        conn.flush().map_err(x11err!("x11-flush"))?;

        let reader = Reader::open(display)?;
        let conn = Arc::new(conn);
        let owner = Arc::new(Mutex::new(Owner::default()));
        let events = Arc::new(Events::default());
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (conn, owner, events, stop) =
                (conn.clone(), owner.clone(), events.clone(), stop.clone());
            std::thread::Builder::new()
                .name("uclip-x11".into())
                .spawn(move || owner_loop(&conn, win, atoms, &owner, &events, &stop))
                .map_err(|e| other("x11-thread")(e.to_string()))?;
        }
        Ok(Self {
            owner_conn: conn,
            owner_win: win,
            owner,
            events,
            stop,
            reader,
            names: HashMap::new(),
            ids: HashMap::new(),
        })
    }

    fn intern(&mut self, name: &str) -> Result<Atom, ClipboardError> {
        if let Some(a) = self.ids.get(name) {
            return Ok(*a);
        }
        let a = self
            .reader
            .conn
            .intern_atom(false, name.as_bytes())
            .map_err(x11err!("x11-intern"))?
            .reply()
            .map_err(x11err!("x11-intern"))?
            .atom;
        self.ids.insert(name.to_owned(), a);
        self.names.insert(a, name.to_owned());
        Ok(a)
    }

    fn names_of(&mut self, atoms: &[Atom]) -> Result<Vec<String>, ClipboardError> {
        let unknown: Vec<Atom> = atoms
            .iter()
            .copied()
            .filter(|a| *a != NONE && !self.names.contains_key(a))
            .collect();
        let mut cookies = Vec::with_capacity(unknown.len());
        for a in &unknown {
            cookies.push(
                self.reader
                    .conn
                    .get_atom_name(*a)
                    .map_err(x11err!("x11-atom-name"))?,
            );
        }
        for (a, c) in unknown.into_iter().zip(cookies) {
            // An atom without a name (a broken owner) is skipped.
            if let Ok(r) = c.reply() {
                let name = String::from_utf8_lossy(&r.name).into_owned();
                self.ids.insert(name.clone(), a);
                self.names.insert(a, name);
            }
        }
        Ok(atoms
            .iter()
            .filter_map(|a| self.names.get(a).cloned())
            .collect())
    }
}

impl Backend for Clip {
    fn offered(&mut self) -> Result<(u64, Vec<String>), ClipboardError> {
        let seq = self.events.seq()?;
        let targets = self.reader.atoms.TARGETS;
        let Some(bytes) = self.reader.convert(targets)? else {
            return Ok((seq, Vec::new()));
        };
        let atoms: Vec<Atom> = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_ne_bytes(*c))
            .collect();
        Ok((seq, self.names_of(&atoms)?))
    }

    fn fetch(&mut self, mime: &str) -> Result<Option<Vec<u8>>, ClipboardError> {
        let atom = self.intern(mime)?;
        self.reader.convert(atom)
    }

    fn set(&mut self, items: Vec<Item>) -> Result<u64, ClipboardError> {
        let mut served = Vec::with_capacity(items.len());
        for (m, b) in items {
            served.push((self.intern(&m)?, b));
        }
        let before = self.events.seq()?;
        self.owner.lock().unwrap_or_else(|e| e.into_inner()).items = served;
        let clipboard = self.reader.atoms.CLIPBOARD;
        let c = &self.owner_conn;
        c.set_selection_owner(self.owner_win, clipboard, CURRENT_TIME)
            .map_err(x11err!("x11-own"))?;
        let now = c
            .get_selection_owner(clipboard)
            .map_err(x11err!("x11-own"))?
            .reply()
            .map_err(x11err!("x11-own"))?
            .owner;
        if now != self.owner_win {
            return Err(ClipboardError::Busy);
        }
        self.events.wait_above(before, WRITE_CONFIRM)
    }

    fn events(&self) -> &Events {
        &self.events
    }
}

impl Drop for Clip {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the owner thread with a message to our own window.
        let wake =
            ClientMessageEvent::new(32, self.owner_win, self.reader.atoms.UCLIP_WAKE, [0u32; 5]);
        let _ = self
            .owner_conn
            .send_event(false, self.owner_win, EventMask::NO_EVENT, wake);
        let _ = self.owner_conn.flush();
    }
}

fn hidden_window(conn: &RustConnection, screen: usize) -> Result<Window, ClipboardError> {
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or_else(|| ClipboardError::Other("x11-no-screen".into()))?
        .root;
    let win = conn.generate_id().map_err(x11err!("x11-id"))?;
    conn.create_window(
        COPY_DEPTH_FROM_PARENT,
        win,
        root,
        -10,
        -10,
        1,
        1,
        0,
        WindowClass::INPUT_OUTPUT,
        COPY_FROM_PARENT,
        &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .map_err(x11err!("x11-window"))?;
    Ok(win)
}

/// One `INCR` transfer to a requestor.
struct Transfer {
    requestor: Window,
    property: Atom,
    ty: Atom,
    data: Arc<Vec<u8>>,
    offset: usize,
    started: Instant,
}

fn owner_loop(
    conn: &RustConnection,
    win: Window,
    atoms: Atoms,
    owner: &Mutex<Owner>,
    events: &Events,
    stop: &AtomicBool,
) {
    // Data above this size goes in INCR chunks of this size.
    let chunk = (conn.maximum_request_bytes() / 4).clamp(4096, 256 * 1024);
    let mut transfers: Vec<Transfer> = Vec::new();
    loop {
        let ev = match conn.wait_for_event() {
            Ok(ev) => ev,
            Err(e) => {
                events.kill(format!("x11-connection-lost: {e}"));
                return;
            }
        };
        if stop.load(Ordering::SeqCst) {
            return;
        }
        match ev {
            Event::XfixesSelectionNotify(e) if e.selection == atoms.CLIPBOARD => {
                if e.owner == win {
                    owner.lock().unwrap_or_else(|e| e.into_inner()).time = e.selection_timestamp;
                }
                events.bump();
            }
            Event::SelectionRequest(req) => {
                serve(conn, &atoms, owner, &req, chunk, &mut transfers);
            }
            Event::SelectionClear(e) if e.selection == atoms.CLIPBOARD => {
                // A write may have taken the selection back already; ask the server.
                let still_ours = conn
                    .get_selection_owner(atoms.CLIPBOARD)
                    .ok()
                    .and_then(|c| c.reply().ok())
                    .is_some_and(|r| r.owner == win);
                if !still_ours {
                    owner
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .items
                        .clear();
                }
            }
            Event::PropertyNotify(e) if e.state == Property::DELETE => {
                if let Some(i) = transfers
                    .iter()
                    .position(|t| t.requestor == e.window && t.property == e.atom)
                {
                    let t = &mut transfers[i];
                    let end = (t.offset + chunk).min(t.data.len());
                    let _ = conn.change_property8(
                        PropMode::REPLACE,
                        t.requestor,
                        t.property,
                        t.ty,
                        &t.data[t.offset..end],
                    );
                    if t.offset == t.data.len() {
                        // That was the empty closing chunk.
                        transfers.swap_remove(i);
                    } else {
                        t.offset = end;
                    }
                    let _ = conn.flush();
                }
            }
            _ => {}
        }
        transfers.retain(|t| t.started.elapsed() < TRANSFER_TTL);
    }
}

fn serve(
    conn: &RustConnection,
    atoms: &Atoms,
    owner: &Mutex<Owner>,
    req: &SelectionRequestEvent,
    chunk: usize,
    transfers: &mut Vec<Transfer>,
) {
    let (items, time) = {
        let o = owner.lock().unwrap_or_else(|e| e.into_inner());
        (o.items.clone(), o.time)
    };
    // Obsolete clients send property None: use the target as the property.
    let property = if req.property == NONE {
        req.target
    } else {
        req.property
    };
    let ok = if req.selection != atoms.CLIPBOARD || items.is_empty() {
        false
    } else if req.target == atoms.TARGETS {
        let mut list = vec![atoms.TARGETS, atoms.TIMESTAMP];
        list.extend(items.iter().map(|(a, _)| *a));
        conn.change_property32(
            PropMode::REPLACE,
            req.requestor,
            property,
            AtomEnum::ATOM,
            &list,
        )
        .is_ok()
    } else if req.target == atoms.TIMESTAMP {
        conn.change_property32(
            PropMode::REPLACE,
            req.requestor,
            property,
            AtomEnum::INTEGER,
            &[time],
        )
        .is_ok()
    } else if let Some((_, data)) = items.iter().find(|(a, _)| *a == req.target) {
        let ty = if req.target == atoms.TEXT {
            atoms.UTF8_STRING
        } else {
            req.target
        };
        if data.len() > chunk {
            let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
            let r1 = conn.change_window_attributes(
                req.requestor,
                &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
            );
            let r2 = conn.change_property32(
                PropMode::REPLACE,
                req.requestor,
                property,
                atoms.INCR,
                &[len],
            );
            transfers.retain(|t| !(t.requestor == req.requestor && t.property == property));
            transfers.push(Transfer {
                requestor: req.requestor,
                property,
                ty,
                data: data.clone(),
                offset: 0,
                started: Instant::now(),
            });
            r1.is_ok() && r2.is_ok()
        } else {
            conn.change_property8(PropMode::REPLACE, req.requestor, property, ty, data)
                .is_ok()
        }
    } else {
        // MULTIPLE and unknown targets are refused.
        false
    };
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: req.time,
        requestor: req.requestor,
        selection: req.selection,
        target: req.target,
        property: if ok { property } else { NONE },
    };
    let _ = conn.send_event(false, req.requestor, EventMask::NO_EVENT, notify);
    let _ = conn.flush();
}

/// The caller-side connection that reads the selection.
struct Reader {
    conn: RustConnection,
    win: Window,
    atoms: Atoms,
}

impl Reader {
    fn open(display: Option<&str>) -> Result<Self, ClipboardError> {
        let (conn, screen) = x11rb::connect(display).map_err(x11err!("x11-connect"))?;
        let atoms = Atoms::new(&conn)
            .map_err(x11err!("x11-atoms"))?
            .reply()
            .map_err(x11err!("x11-atoms"))?;
        let win = hidden_window(&conn, screen)?;
        conn.flush().map_err(x11err!("x11-flush"))?;
        Ok(Self { conn, win, atoms })
    }

    /// Wait for the first event that matches `pred`; other events are dropped.
    fn wait(
        &self,
        deadline: Instant,
        pred: impl Fn(&Event) -> bool,
    ) -> Result<Event, ClipboardError> {
        loop {
            while let Some(ev) = self.conn.poll_for_event().map_err(x11err!("x11-read"))? {
                if pred(&ev) {
                    return Ok(ev);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                return Err(ClipboardError::Other("x11-timeout".into()));
            }
            poll_in(self.conn.stream(), deadline - now).map_err(x11err!("x11-poll"))?;
        }
    }

    fn get(
        &self,
        delete: bool,
    ) -> Result<x11rb::protocol::xproto::GetPropertyReply, ClipboardError> {
        self.conn
            .get_property(
                delete,
                self.win,
                self.atoms.UCLIP_SEL,
                AtomEnum::ANY,
                0,
                u32::MAX / 4,
            )
            .map_err(x11err!("x11-get"))?
            .reply()
            .map_err(x11err!("x11-get"))
    }

    /// Ask the owner for `target` and read the answer, with `INCR` support.
    fn convert(&self, target: Atom) -> Result<Option<Vec<u8>>, ClipboardError> {
        // Drop events left from an earlier read that timed out.
        while self
            .conn
            .poll_for_event()
            .map_err(x11err!("x11-read"))?
            .is_some()
        {}
        let prop = self.atoms.UCLIP_SEL;
        self.conn
            .convert_selection(self.win, self.atoms.CLIPBOARD, target, prop, CURRENT_TIME)
            .map_err(x11err!("x11-convert"))?;
        self.conn.flush().map_err(x11err!("x11-flush"))?;
        let win = self.win;
        let ev = self.wait(
            Instant::now() + READ_TIMEOUT,
            |e| matches!(e, Event::SelectionNotify(n) if n.requestor == win && n.target == target),
        )?;
        let Event::SelectionNotify(n) = ev else {
            return Err(ClipboardError::Other("x11-event".into()));
        };
        if n.property == NONE {
            return Ok(None);
        }
        let first = self.get(false)?;
        if first.type_ != self.atoms.INCR {
            self.conn
                .delete_property(win, prop)
                .map_err(x11err!("x11-delete"))?;
            self.conn.flush().map_err(x11err!("x11-flush"))?;
            if first.type_ == NONE {
                return Ok(None);
            }
            return Ok(Some(first.value));
        }
        // INCR: deleting the property asks for the next chunk; an empty chunk ends it.
        self.conn
            .delete_property(win, prop)
            .map_err(x11err!("x11-delete"))?;
        self.conn.flush().map_err(x11err!("x11-flush"))?;
        let mut out = Vec::new();
        loop {
            self.wait(Instant::now() + READ_TIMEOUT, |e| {
                matches!(e, Event::PropertyNotify(p)
                    if p.window == win && p.atom == prop && p.state == Property::NEW_VALUE)
            })?;
            let r = self.get(true)?;
            self.conn.flush().map_err(x11err!("x11-flush"))?;
            if r.value.is_empty() {
                return Ok(Some(out));
            }
            if out.len() + r.value.len() > MAX_READ {
                return Err(ClipboardError::Other("x11-too-large".into()));
            }
            out.extend_from_slice(&r.value);
        }
    }
}
