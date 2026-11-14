//! Local IPC between the agent and its clients (the `uc` CLI, the settings window, tests),
//! plan 3.2: JSON lines with a request, a reply and an event stream.
//!
//! Hardening (plan 3.2, threat 10):
//! - **Windows:** named pipe `\\.\pipe\uclip-<user SID>[-profile]`, created with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE` (no other program can take the name first), a DACL for
//!   this user only, remote clients refused. The agent checks that each client process runs
//!   under the same SID, and the client checks the same of the server before it sends
//!   anything. (The check that the server is *our signed* agent needs code signing, M9.)
//! - **macOS, Linux:** Unix socket, mode `0600`, in a `0700` folder. Both sides check the
//!   peer's user ID.
//!
//! Wire format, one JSON object per line:
//! - request `{"id":1,"call":"status","params":{}}`
//! - reply `{"id":1,"ok":true,"result":{...}}` or `{"id":1,"ok":false,"error":{"code":"...","message":"..."}}`
//! - event (after `subscribe`) `{"event":"pair.added","data":{...}}`

use std::collections::VecDeque;
use std::io;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

use crate::profile::Profile;

/// The calls of plan 3.2, plus `setup.*` (for `uc setup`) and `version`.
pub const CALLS: &[&str] = &[
    "status",
    "subscribe",
    "pair.start",
    "pair.join",
    "pair.cancel",
    "devices.list",
    "devices.rename",
    "devices.remove",
    "pause.set",
    "settings.get",
    "settings.set",
    "diagnostics",
    "leave",
    "setup.status",
    "setup.step",
    "version",
];

/// Calls for the installers and the updater (not window actions, so no `uc` command).
pub const SERVICE_CALLS: &[&str] = &["quit"];

/// Calls that exist only in test profiles (the real app refuses them).
pub const DEBUG_CALLS: &[&str] = &[
    "debug.copy",
    "debug.clipboard",
    "debug.shutdown",
    "debug.wake",
];

/// Longest line accepted (1 MiB).
pub const MAX_LINE: usize = 1 << 20;

/// A request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Request {
    /// Chosen by the client; the reply carries it back.
    pub id: u64,
    /// One of [`CALLS`].
    pub call: String,
    /// Arguments.
    #[serde(default)]
    pub params: Value,
}

/// An error in a reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpcError {
    /// A stable code for scripts, for example `not-found`.
    pub code: String,
    /// A plain sentence for people.
    pub message: String,
}

impl IpcError {
    /// Build one.
    pub fn new(code: &str, message: impl Into<String>) -> IpcError {
        IpcError {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IpcError {}

/// A reply.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Reply {
    /// The request's ID.
    pub id: u64,
    /// Success?
    pub ok: bool,
    /// The result on success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// The error on failure.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<IpcError>,
}

impl Reply {
    /// From a result.
    pub fn from_result(id: u64, r: Result<Value, IpcError>) -> Reply {
        match r {
            Ok(v) => Reply {
                id,
                ok: true,
                result: Some(v),
                error: None,
            },
            Err(e) => Reply {
                id,
                ok: false,
                result: None,
                error: Some(e),
            },
        }
    }
}

/// An event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    /// The name, for example `pair.added`.
    pub event: String,
    /// Details.
    #[serde(default)]
    pub data: Value,
}

/// One request handed to the agent's core.
#[derive(Debug)]
pub struct Incoming {
    /// The request.
    pub req: Request,
    /// Where the reply goes. The core may keep it and answer later (`pair.join`).
    pub reply: oneshot::Sender<Reply>,
    /// The connection's event channel, for `subscribe`.
    pub events: mpsc::UnboundedSender<Event>,
}

/// Why the agent cannot open its IPC endpoint.
#[derive(Debug)]
pub enum BindError {
    /// Another agent of this user and profile owns it.
    AlreadyRunning,
    /// Anything else.
    Io(io::Error),
}

impl std::fmt::Display for BindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BindError::AlreadyRunning => f.write_str("the agent is already running"),
            BindError::Io(e) => write!(f, "IPC: {e}"),
        }
    }
}

async fn serve_conn<S>(stream: S, tx: mpsc::Sender<Incoming>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (r, mut w) = tokio::io::split(stream);
    let (line_tx, mut line_rx) = mpsc::unbounded_channel::<String>();
    let (ev_tx, mut ev_rx) = mpsc::unbounded_channel::<Event>();
    let writer = tokio::spawn(async move {
        loop {
            let line = tokio::select! {
                l = line_rx.recv() => match l { Some(l) => l, None => break },
                e = ev_rx.recv() => match e {
                    Some(e) => serde_json::to_string(&e).unwrap_or_default(),
                    None => break,
                },
            };
            if w.write_all(line.as_bytes()).await.is_err()
                || w.write_all(b"\n").await.is_err()
                || w.flush().await.is_err()
            {
                break;
            }
        }
    });
    let mut reader = BufReader::new(r);
    let mut buf = String::new();
    loop {
        buf.clear();
        match reader.read_line(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if buf.len() > MAX_LINE {
            break;
        }
        let req: Request = match serde_json::from_str(buf.trim()) {
            Ok(r) => r,
            Err(_) => {
                let r = Reply::from_result(0, Err(IpcError::new("bad-request", "Bad request.")));
                let _ = line_tx.send(serde_json::to_string(&r).unwrap_or_default());
                continue;
            }
        };
        let (rtx, rrx) = oneshot::channel();
        let id = req.id;
        if tx
            .send(Incoming {
                req,
                reply: rtx,
                events: ev_tx.clone(),
            })
            .await
            .is_err()
        {
            break;
        }
        let lt = line_tx.clone();
        tokio::spawn(async move {
            let reply = rrx.await.unwrap_or_else(|_| {
                Reply::from_result(id, Err(IpcError::new("stopped", "The agent stopped.")))
            });
            let _ = lt.send(serde_json::to_string(&reply).unwrap_or_default());
        });
    }
    drop(line_tx);
    drop(ev_tx);
    let _ = writer.await;
}

// ---------------------------------------------------------------- Windows

#[cfg(windows)]
mod sys {
    #![allow(unsafe_code)]
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    use super::*;
    use crate::win;

    /// The bound pipe.
    pub struct Listener {
        name: String,
        first: NamedPipeServer,
        sa: win::OwnerOnly,
    }

    fn create(name: &str, sa: &mut win::OwnerOnly, first: bool) -> io::Result<NamedPipeServer> {
        let mut o = ServerOptions::new();
        o.first_pipe_instance(first).reject_remote_clients(true);
        // SAFETY: `sa` holds valid SECURITY_ATTRIBUTES that outlive the call.
        unsafe { o.create_with_security_attributes_raw(name, sa.as_ptr()) }
    }

    pub fn bind(p: &Profile) -> Result<Listener, BindError> {
        let name = p.pipe_name().map_err(BindError::Io)?;
        let mut sa = win::OwnerOnly::new().map_err(BindError::Io)?;
        match create(&name, &mut sa, true) {
            Ok(first) => Ok(Listener { name, first, sa }),
            Err(e) if e.raw_os_error() == Some(5) || e.raw_os_error() == Some(231) => {
                Err(BindError::AlreadyRunning)
            }
            Err(e) => Err(BindError::Io(e)),
        }
    }

    impl Listener {
        pub fn name(&self) -> String {
            self.name.clone()
        }

        pub async fn run(mut self, tx: mpsc::Sender<Incoming>) {
            let me = win::current_user_sid().unwrap_or_default();
            let mut server = self.first;
            loop {
                if server.connect().await.is_err() {
                    match create(&self.name, &mut self.sa, false) {
                        Ok(s) => server = s,
                        Err(_) => return,
                    }
                    continue;
                }
                let client = server;
                server = match create(&self.name, &mut self.sa, false) {
                    Ok(s) => s,
                    Err(e) => {
                        tracing::error!(error = %e, "pipe instance");
                        return;
                    }
                };
                let same = win::pipe_peer_pid(&client, false)
                    .and_then(win::process_sid)
                    .is_ok_and(|s| s == me);
                if !same {
                    tracing::warn!("IPC client of another user refused");
                    continue;
                }
                tokio::spawn(serve_conn(client, tx.clone()));
            }
        }
    }

    pub type ClientStream = NamedPipeClient;

    pub async fn connect(p: &Profile) -> io::Result<ClientStream> {
        let name = p.pipe_name()?;
        let mut tries = 0;
        let c = loop {
            match ClientOptions::new().open(&name) {
                Ok(c) => break c,
                Err(e) if e.raw_os_error() == Some(231) && tries < 50 => {
                    tries += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
                Err(e) => return Err(e),
            }
        };
        let me = win::current_user_sid()?;
        let theirs = win::pipe_peer_pid(&c, true).and_then(win::process_sid)?;
        if theirs != me {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the pipe belongs to another user",
            ));
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------- Unix

#[cfg(unix)]
mod sys {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    use std::path::PathBuf;

    use tokio::net::{UnixListener, UnixStream};

    use super::*;

    #[allow(unsafe_code)]
    fn my_uid() -> u32 {
        // SAFETY: getuid has no preconditions and cannot fail.
        unsafe { libc::getuid() }
    }

    pub struct Listener {
        path: PathBuf,
        inner: UnixListener,
    }

    pub fn bind(p: &Profile) -> Result<Listener, BindError> {
        let path = p.socket_path();
        if let Some(dir) = path.parent() {
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(dir)
                .map_err(BindError::Io)?;
            let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
        }
        if path.exists() {
            if std::os::unix::net::UnixStream::connect(&path).is_ok() {
                return Err(BindError::AlreadyRunning);
            }
            let _ = std::fs::remove_file(&path);
        }
        let inner = UnixListener::bind(&path).map_err(BindError::Io)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(BindError::Io)?;
        Ok(Listener { path, inner })
    }

    impl Listener {
        pub fn name(&self) -> String {
            self.path.display().to_string()
        }

        pub async fn run(self, tx: mpsc::Sender<Incoming>) {
            let me = my_uid();
            loop {
                let Ok((s, _)) = self.inner.accept().await else {
                    continue;
                };
                if s.peer_cred().map(|c| c.uid()).ok() != Some(me) {
                    tracing::warn!("IPC client of another user refused");
                    continue;
                }
                tokio::spawn(serve_conn(s, tx.clone()));
            }
        }
    }

    impl Drop for Listener {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.path);
        }
    }

    pub type ClientStream = UnixStream;

    pub async fn connect(p: &Profile) -> io::Result<ClientStream> {
        let s = UnixStream::connect(p.socket_path()).await?;
        if s.peer_cred()?.uid() != my_uid() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the socket belongs to another user",
            ));
        }
        Ok(s)
    }
}

pub use sys::Listener;

/// Open the IPC endpoint. Fails with [`BindError::AlreadyRunning`] if another agent of this
/// user and profile has it.
pub fn bind(p: &Profile) -> Result<Listener, BindError> {
    sys::bind(p)
}

/// A client: the CLI, the window and the tests use it.
pub struct Client {
    reader: BufReader<tokio::io::ReadHalf<sys::ClientStream>>,
    writer: tokio::io::WriteHalf<sys::ClientStream>,
    next_id: u64,
    events: VecDeque<Event>,
}

impl Client {
    /// Connect to the agent of this profile and check that it runs as this user.
    pub async fn connect(p: &Profile) -> io::Result<Client> {
        let s = sys::connect(p).await?;
        let (r, w) = tokio::io::split(s);
        Ok(Client {
            reader: BufReader::new(r),
            writer: w,
            next_id: 1,
            events: VecDeque::new(),
        })
    }

    async fn read_line(&mut self) -> io::Result<Value> {
        let mut buf = String::new();
        if self.reader.read_line(&mut buf).await? == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "the agent closed the connection",
            ));
        }
        serde_json::from_str(buf.trim()).map_err(io::Error::other)
    }

    /// Send a call and wait for its reply. Events that arrive meanwhile are kept for
    /// [`Client::next_event`].
    pub async fn call(&mut self, call: &str, params: Value) -> io::Result<Result<Value, IpcError>> {
        let id = self.next_id;
        self.next_id += 1;
        let line = serde_json::to_string(&Request {
            id,
            call: call.into(),
            params,
        })
        .map_err(io::Error::other)?;
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        loop {
            let v = self.read_line().await?;
            if v.get("event").is_some() {
                if let Ok(e) = serde_json::from_value(v) {
                    self.events.push_back(e);
                }
                continue;
            }
            let r: Reply = serde_json::from_value(v).map_err(io::Error::other)?;
            if r.id != id {
                continue;
            }
            return Ok(if r.ok {
                Ok(r.result.unwrap_or(Value::Null))
            } else {
                Err(r
                    .error
                    .unwrap_or_else(|| IpcError::new("unknown", "Unknown error.")))
            });
        }
    }

    /// The next event (after `subscribe`). `None` when the agent closed the connection.
    pub async fn next_event(&mut self) -> Option<Event> {
        if let Some(e) = self.events.pop_front() {
            return Some(e);
        }
        loop {
            let v = self.read_line().await.ok()?;
            if v.get("event").is_some() {
                return serde_json::from_value(v).ok();
            }
        }
    }
}
