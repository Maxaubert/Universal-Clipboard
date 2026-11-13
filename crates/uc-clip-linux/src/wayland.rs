//! Wayland backend: the data-control protocol, in its two versions.
//!
//! `ext-data-control-v1` (the standard) and `wlr-data-control-unstable-v1` (older wlroots and
//! KDE) have the same requests and events with other names. One macro writes the backend for
//! each, so the logic exists once.
//!
//! An event thread dispatches the Wayland queue: it tracks the current selection offer and
//! its types, and serves our sources. Requests (read, write) are sent from the caller's
//! thread, which wayland-client allows.

use std::io::Read;
use std::time::{Duration, Instant};

use uc_clip::ClipboardError;

use crate::backend::{READ_TIMEOUT, poll_in};
use crate::mime::MAX_READ;

fn other(code: &str, detail: impl std::fmt::Display) -> ClipboardError {
    ClipboardError::Other(format!("{code}: {detail}"))
}

/// Read a pipe until the writer closes it. `READ_TIMEOUT` is the longest wait with no data.
fn read_pipe(mut reader: std::io::PipeReader) -> Result<Vec<u8>, ClipboardError> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left == Duration::ZERO {
            return Err(ClipboardError::Other("wl-timeout".into()));
        }
        if !poll_in(&reader, left).map_err(|e| other("wl-poll", e))? {
            continue;
        }
        match reader.read(&mut buf) {
            Ok(0) => return Ok(out),
            Ok(n) => {
                if out.len() + n > MAX_READ {
                    return Err(ClipboardError::Other("wl-too-large".into()));
                }
                out.extend_from_slice(&buf[..n]);
                deadline = Instant::now() + READ_TIMEOUT;
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(other("wl-read", e)),
        }
    }
}

/// The backend body. Expects `Manager`, `Device`, `Source`, `Offer` and the modules
/// `device`, `source`, `offer` to be in scope, and takes a short name for error codes.
macro_rules! data_control_backend {
    ($name:literal) => {
        use std::io::Write as _;
        use std::os::fd::AsFd as _;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        use uc_clip::ClipboardError;
        use wayland_client::globals::{GlobalListContents, registry_queue_init};
        use wayland_client::protocol::{
            wl_callback::WlCallback, wl_registry::WlRegistry, wl_seat::WlSeat,
        };
        use wayland_client::{Connection, Dispatch, Proxy as _, QueueHandle, event_created_child};

        use crate::backend::{Backend, Events, WRITE_CONFIRM};
        use crate::mime::Item;
        use crate::wayland::{other, read_pipe};

        /// The current selection offer and its types.
        type Current = Arc<Mutex<Option<(Offer, Vec<String>)>>>;

        pub(crate) struct State {
            current: Current,
            events: Arc<Events>,
        }

        #[derive(Default)]
        pub(crate) struct OfferData {
            mimes: Mutex<Vec<String>>,
        }

        pub(crate) struct SourceData {
            items: Vec<Item>,
        }

        pub(crate) struct Clip {
            conn: Connection,
            qh: QueueHandle<State>,
            manager: Manager,
            device: Device,
            current: Current,
            events: Arc<Events>,
            stop: Arc<AtomicBool>,
        }

        impl Clip {
            pub(crate) fn open() -> Result<Self, ClipboardError> {
                let conn = Connection::connect_to_env().map_err(|e| other("wl-connect", e))?;
                let (globals, mut queue) =
                    registry_queue_init::<State>(&conn).map_err(|e| other("wl-registry", e))?;
                let qh = queue.handle();
                let manager: Manager = globals
                    .bind(&qh, 1..=1, ())
                    .map_err(|e| other(concat!($name, "-missing"), e))?;
                let seat: WlSeat = globals
                    .bind(&qh, 1..=1, ())
                    .map_err(|e| other("wl-no-seat", e))?;
                let device = manager.get_data_device(&seat, &qh, ());
                let current: Current = Arc::new(Mutex::new(None));
                let events = Arc::new(Events::default());
                let mut state = State {
                    current: current.clone(),
                    events: events.clone(),
                };
                // Receive the current selection before the first read.
                queue
                    .roundtrip(&mut state)
                    .map_err(|e| other("wl-roundtrip", e))?;
                let stop = Arc::new(AtomicBool::new(false));
                {
                    let stop = stop.clone();
                    let events = events.clone();
                    std::thread::Builder::new()
                        .name(concat!("uclip-", $name).into())
                        .spawn(move || {
                            while !stop.load(Ordering::SeqCst) {
                                if let Err(e) = queue.blocking_dispatch(&mut state) {
                                    events.kill(format!("wl-connection-lost: {e}"));
                                    return;
                                }
                            }
                        })
                        .map_err(|e| other("wl-thread", e))?;
                }
                Ok(Self {
                    conn,
                    qh,
                    manager,
                    device,
                    current,
                    events,
                    stop,
                })
            }
        }

        impl Backend for Clip {
            fn offered(&mut self) -> Result<(u64, Vec<String>), ClipboardError> {
                let seq = self.events.seq()?;
                let g = self.current.lock().unwrap_or_else(|e| e.into_inner());
                Ok((seq, g.as_ref().map(|(_, m)| m.clone()).unwrap_or_default()))
            }

            fn fetch(&mut self, mime: &str) -> Result<Option<Vec<u8>>, ClipboardError> {
                let offer = {
                    let g = self.current.lock().unwrap_or_else(|e| e.into_inner());
                    match &*g {
                        Some((o, mimes)) if mimes.iter().any(|m| m == mime) => o.clone(),
                        _ => return Ok(None),
                    }
                };
                let (reader, writer) = std::io::pipe().map_err(|e| other("wl-pipe", e))?;
                offer.receive(mime.to_owned(), writer.as_fd());
                self.conn.flush().map_err(|e| other("wl-flush", e))?;
                drop(writer);
                let bytes = read_pipe(reader)?;
                // A source that went away closes the pipe with no data.
                Ok((!bytes.is_empty()).then_some(bytes))
            }

            fn set(&mut self, items: Vec<Item>) -> Result<u64, ClipboardError> {
                let mimes: Vec<String> = items.iter().map(|(m, _)| m.clone()).collect();
                let source = self
                    .manager
                    .create_data_source(&self.qh, SourceData { items });
                for m in mimes {
                    source.offer(m);
                }
                let before = self.events.seq()?;
                self.device.set_selection(Some(&source));
                self.conn.flush().map_err(|e| other("wl-flush", e))?;
                self.events.wait_above(before, WRITE_CONFIRM)
            }

            fn events(&self) -> &Events {
                &self.events
            }
        }

        impl Drop for Clip {
            fn drop(&mut self) {
                self.stop.store(true, Ordering::SeqCst);
                // A sync callback wakes the event thread so it sees the stop flag.
                self.device.destroy();
                self.conn.display().sync(&self.qh, ());
                let _ = self.conn.flush();
            }
        }

        impl Dispatch<WlRegistry, GlobalListContents> for State {
            fn event(
                _: &mut Self,
                _: &WlRegistry,
                _: <WlRegistry as wayland_client::Proxy>::Event,
                _: &GlobalListContents,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<WlSeat, ()> for State {
            fn event(
                _: &mut Self,
                _: &WlSeat,
                _: <WlSeat as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<WlCallback, ()> for State {
            fn event(
                _: &mut Self,
                _: &WlCallback,
                _: <WlCallback as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<Manager, ()> for State {
            fn event(
                _: &mut Self,
                _: &Manager,
                _: <Manager as wayland_client::Proxy>::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
            }
        }

        impl Dispatch<Device, ()> for State {
            fn event(
                state: &mut Self,
                dev: &Device,
                event: device::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    device::Event::Selection { id } => {
                        let new = id.map(|o| {
                            let mimes = o
                                .data::<OfferData>()
                                .map(|d| d.mimes.lock().unwrap_or_else(|e| e.into_inner()).clone())
                                .unwrap_or_default();
                            (o, mimes)
                        });
                        let old = std::mem::replace(
                            &mut *state.current.lock().unwrap_or_else(|e| e.into_inner()),
                            new,
                        );
                        if let Some((o, _)) = old {
                            o.destroy();
                        }
                        state.events.bump();
                    }
                    // The primary selection (middle-click paste) is not synced.
                    device::Event::PrimarySelection { id: Some(o) } => o.destroy(),
                    device::Event::Finished => {
                        dev.destroy();
                        state.events.kill(concat!($name, "-device-finished").into());
                    }
                    _ => {}
                }
            }

            event_created_child!(State, Device, [
                device::EVT_DATA_OFFER_OPCODE => (Offer, OfferData::default())
            ]);
        }

        impl Dispatch<Offer, OfferData> for State {
            fn event(
                _: &mut Self,
                _: &Offer,
                event: offer::Event,
                data: &OfferData,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if let offer::Event::Offer { mime_type } = event {
                    data.mimes
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(mime_type);
                }
            }
        }

        impl Dispatch<Source, SourceData> for State {
            fn event(
                _: &mut Self,
                src: &Source,
                event: source::Event,
                data: &SourceData,
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    source::Event::Send { mime_type, fd } => {
                        // Write on a thread: the reader may be slow, or be ourselves.
                        if let Some((_, bytes)) = data.items.iter().find(|(m, _)| *m == mime_type) {
                            let bytes = bytes.clone();
                            let _ = std::thread::Builder::new()
                                .name("uclip-wl-send".into())
                                .spawn(move || {
                                    let mut f = std::fs::File::from(fd);
                                    let _ = f.write_all(&bytes);
                                });
                        }
                    }
                    source::Event::Cancelled => src.destroy(),
                    _ => {}
                }
            }
        }
    };
}

/// `ext-data-control-v1`.
pub(crate) mod ext {
    use wayland_protocols::ext::data_control::v1::client::{
        ext_data_control_device_v1::{self as device, ExtDataControlDeviceV1 as Device},
        ext_data_control_manager_v1::ExtDataControlManagerV1 as Manager,
        ext_data_control_offer_v1::{self as offer, ExtDataControlOfferV1 as Offer},
        ext_data_control_source_v1::{self as source, ExtDataControlSourceV1 as Source},
    };

    data_control_backend!("ext-data-control");
}

/// `wlr-data-control-unstable-v1`.
pub(crate) mod wlr {
    use wayland_protocols_wlr::data_control::v1::client::{
        zwlr_data_control_device_v1::{self as device, ZwlrDataControlDeviceV1 as Device},
        zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 as Manager,
        zwlr_data_control_offer_v1::{self as offer, ZwlrDataControlOfferV1 as Offer},
        zwlr_data_control_source_v1::{self as source, ZwlrDataControlSourceV1 as Source},
    };

    data_control_backend!("wlr-data-control");
}
