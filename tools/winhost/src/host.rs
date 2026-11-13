#![allow(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_ALL, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber;
use windows_sys::Win32::System::StationsAndDesktops::{
    CloseDesktop, CloseWindowStation, CreateDesktopW, CreateWindowStationW,
    GetProcessWindowStation, GetUserObjectInformationW, SetProcessWindowStation, UOI_NAME,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateMutexW, CreateProcessW, GetExitCodeProcess,
    INFINITE, PROCESS_INFORMATION, ReleaseMutex, STARTF_USESTDHANDLES, STARTUPINFOW,
    TerminateProcess, WaitForSingleObject,
};

/// Set to `1` in the environment of a process that winhost starts.
pub const CHILD_ENV: &str = "UC_WINHOST_CHILD";
/// The name of the private station, set in the child's environment. The child checks it.
pub const STATION_ENV: &str = "UC_WINHOST_STATION";

const DESKTOP: &str = "uc-winhost";
/// `WINSTA_ALL_ACCESS | STANDARD_RIGHTS_REQUIRED`.
const WINSTA_ALL: u32 = 0x000F_037F;

/// Name of the session-wide lock that serializes winhost runs (see [`SessionLock`]).
const SESSION_LOCK: &str = "Local\\UniversalClipboard.winhost";

/// Switching the process window station is process-wide, and handle inheritance too. One
/// launch at a time.
static LAUNCH: Mutex<()> = Mutex::new(());
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// What the child did.
#[derive(Clone, Debug)]
pub struct Output {
    /// The exit code (1 if it was killed after the timeout).
    pub exit_code: u32,
    /// stdout and stderr together.
    pub output: String,
    /// The private station's name, for example `Service-0x0-26a5251$`.
    pub station: String,
    /// The child was killed because it ran too long.
    pub timed_out: bool,
}

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn object_name(h: HANDLE) -> io::Result<String> {
    let mut buf = [0u16; 256];
    let mut need = 0u32;
    // SAFETY: the buffer and its size in bytes match; `h` is a station or desktop handle.
    let ok = unsafe {
        GetUserObjectInformationW(
            h,
            UOI_NAME,
            buf.as_mut_ptr().cast(),
            (buf.len() * 2) as u32,
            &mut need,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Ok(String::from_utf16_lossy(&buf[..n]))
}

/// The name of this process's window station (`WinSta0` in a normal user session).
pub fn current_station() -> io::Result<String> {
    // SAFETY: no preconditions; the handle is not to be closed.
    object_name(unsafe { GetProcessWindowStation() })
}

/// The clipboard sequence number of this process's station. Reads no content.
pub fn host_sequence_number() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetClipboardSequenceNumber() }
}

/// Quote one argument the way `CommandLineToArgvW` and the Rust runtime read it back.
fn quote(arg: &OsStr, out: &mut Vec<u16>) {
    let a: Vec<u16> = arg.encode_wide().collect();
    let needs = a.is_empty()
        || a.iter()
            .any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
    if !needs {
        out.extend_from_slice(&a);
        return;
    }
    out.push(b'"' as u16);
    let mut backslashes = 0usize;
    for &c in &a {
        if c == b'\\' as u16 {
            backslashes += 1;
        } else {
            if c == b'"' as u16 {
                out.extend(std::iter::repeat_n(b'\\' as u16, backslashes + 1));
            }
            backslashes = 0;
        }
        out.push(c);
    }
    out.extend(std::iter::repeat_n(b'\\' as u16, backslashes));
    out.push(b'"' as u16);
}

fn command_line(exe: &Path, args: &[OsString]) -> Vec<u16> {
    let mut out = Vec::new();
    quote(exe.as_os_str(), &mut out);
    for a in args {
        out.push(b' ' as u16);
        quote(a, &mut out);
    }
    out.push(0);
    out
}

fn env_block(extra: &[(OsString, OsString)]) -> Vec<u16> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| {
            !extra.iter().any(|(e, _)| {
                e.to_string_lossy()
                    .eq_ignore_ascii_case(&k.to_string_lossy())
            })
        })
        .collect();
    vars.extend(extra.iter().cloned());
    vars.sort_by_key(|(k, _)| k.to_string_lossy().to_uppercase());
    let mut out = Vec::new();
    for (k, v) in vars {
        out.extend(k.encode_wide());
        out.push(b'=' as u16);
        out.extend(v.encode_wide());
        out.push(0);
    }
    out.push(0);
    out
}

/// A NULL-name window station is the session's service station (`Service-0x0-<LUID>$`):
/// every winhost run in the same logon session gets the same one, and with it the same
/// clipboard. So runs take this named mutex, and only one child uses the clipboard at a time,
/// across all test processes.
struct SessionLock(HANDLE);

impl SessionLock {
    fn acquire() -> io::Result<SessionLock> {
        let name = wide(OsStr::new(SESSION_LOCK));
        // SAFETY: valid NUL-terminated name. The wait is on the handle we just got; an
        // abandoned mutex (a crashed run) is ours too and is fine to take over.
        unsafe {
            let h = CreateMutexW(std::ptr::null(), 0, name.as_ptr());
            if h.is_null() {
                return Err(io::Error::last_os_error());
            }
            WaitForSingleObject(h, INFINITE);
            Ok(SessionLock(h))
        }
    }
}

impl Drop for SessionLock {
    fn drop(&mut self) {
        // SAFETY: we own the mutex (taken on this thread in `acquire`) and the handle.
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

struct Station {
    station: HANDLE,
    desktop: HANDLE,
}

impl Drop for Station {
    fn drop(&mut self) {
        // SAFETY: both handles are ours and still open.
        unsafe {
            CloseDesktop(self.desktop);
            CloseWindowStation(self.station);
        }
    }
}

fn make_station() -> io::Result<(Station, String)> {
    // SAFETY: Win32 calls with valid arguments. A NULL station name lets Windows pick a unique
    // name (a named one fails with access denied for a normal user, spike S3). The desktop is
    // made while the new station is the process station, then the old one is restored.
    unsafe {
        let station = CreateWindowStationW(std::ptr::null(), 0, WINSTA_ALL, std::ptr::null());
        if station.is_null() {
            return Err(io::Error::last_os_error());
        }
        let original = GetProcessWindowStation();
        if SetProcessWindowStation(station) == 0 {
            let e = io::Error::last_os_error();
            CloseWindowStation(station);
            return Err(e);
        }
        let name = wide(OsStr::new(DESKTOP));
        let desktop = CreateDesktopW(
            name.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            GENERIC_ALL,
            std::ptr::null(),
        );
        let err = io::Error::last_os_error();
        SetProcessWindowStation(original);
        if desktop.is_null() {
            CloseWindowStation(station);
            return Err(err);
        }
        let s = Station { station, desktop };
        let station_name = object_name(s.station)?;
        Ok((s, station_name))
    }
}

/// Start `exe args` inside a new private window station and desktop, wait for it (at most
/// `timeout`), and return its exit code and output. The child's environment gets
/// [`CHILD_ENV`]`=1`, [`STATION_ENV`] and `extra_env`.
pub fn run(
    exe: &Path,
    args: &[OsString],
    extra_env: &[(OsString, OsString)],
    timeout: Duration,
) -> io::Result<Output> {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let log_path = std::env::temp_dir().join(format!("uc-winhost-{}-{n}.log", std::process::id()));
    let log = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&log_path)?;
    let log_handle = log.as_raw_handle() as HANDLE;

    let session = SessionLock::acquire()?;
    let guard = LAUNCH.lock().unwrap_or_else(|e| e.into_inner());
    let (station, station_name) = make_station()?;
    let mut env = vec![
        (OsString::from(CHILD_ENV), OsString::from("1")),
        (OsString::from(STATION_ENV), OsString::from(&station_name)),
    ];
    env.extend(extra_env.iter().cloned());
    let env = env_block(&env);
    let mut desktop_path = wide(OsStr::new(&format!("{station_name}\\{DESKTOP}")));
    let mut cmd = command_line(exe, args);
    let app = wide(exe.as_os_str());

    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    // SAFETY: all buffers are NUL-terminated and live across the call. The log handle is
    // made inheritable only for this launch, under the launch lock.
    let created = unsafe {
        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = desktop_path.as_mut_ptr();
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdInput = std::ptr::null_mut();
        si.hStdOutput = log_handle;
        si.hStdError = log_handle;
        SetHandleInformation(log_handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        let ok = CreateProcessW(
            app.as_ptr(),
            cmd.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            env.as_ptr().cast(),
            std::ptr::null(),
            &si,
            &mut pi,
        );
        let err = io::Error::last_os_error();
        SetHandleInformation(log_handle, HANDLE_FLAG_INHERIT, 0);
        if ok == 0 { Err(err) } else { Ok(()) }
    };
    drop(guard);
    created?;

    let ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX - 1);
    let mut exit_code = 1u32;
    // SAFETY: the process and thread handles are ours; we close them once.
    let timed_out = unsafe {
        let timed_out = WaitForSingleObject(pi.hProcess, ms) == WAIT_TIMEOUT;
        if timed_out {
            TerminateProcess(pi.hProcess, 1);
            WaitForSingleObject(pi.hProcess, 10_000);
        }
        GetExitCodeProcess(pi.hProcess, &mut exit_code);
        CloseHandle(pi.hThread);
        CloseHandle(pi.hProcess);
        timed_out
    };
    drop(station);
    drop(session);
    drop(log);
    let output = std::fs::read(&log_path)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
        .unwrap_or_default();
    let _ = std::fs::remove_file(&log_path);
    Ok(Output {
        exit_code,
        output,
        station: station_name,
        timed_out,
    })
}

/// Panic unless this process runs inside the private station that winhost made for it.
/// Call it before any real clipboard use in a test.
pub fn require_isolated() {
    let expected = std::env::var(STATION_ENV).unwrap_or_default();
    let actual = current_station().unwrap_or_default();
    assert!(
        std::env::var(CHILD_ENV).as_deref() == Ok("1")
            && !expected.is_empty()
            && actual == expected
            && !actual.eq_ignore_ascii_case("WinSta0"),
        "refusing to touch the clipboard: not inside a winhost station (station {actual:?})"
    );
}

/// Run a real-clipboard test in isolation.
///
/// In the normal test process this re-runs the current test binary inside a new private
/// station (`--exact <name>`), prints its output and fails if the child failed. Inside that
/// station (the child) it checks [`require_isolated`] and runs `body`.
///
/// It also checks that the host clipboard sequence number did not change. In CI (`CI` set)
/// a change fails the test; locally it only prints a warning, because the user may copy
/// something while tests run.
pub fn isolated_test(name: &str, body: impl FnOnce()) {
    if std::env::var_os(CHILD_ENV).is_some() {
        require_isolated();
        body();
        return;
    }
    let exe = std::env::current_exe().expect("current exe");
    let before = host_sequence_number();
    let args: Vec<OsString> = ["--exact", name, "--nocapture", "--test-threads=1"]
        .iter()
        .map(OsString::from)
        .collect();
    let out = run(&exe, &args, &[], Duration::from_secs(120)).expect("winhost launch");
    let after = host_sequence_number();
    println!("--- winhost child in station {} ---", out.station);
    println!("{}", out.output);
    assert!(!out.timed_out, "child timed out");
    assert_eq!(out.exit_code, 0, "child failed");
    assert!(
        out.output.contains("1 passed"),
        "the child did not run test {name:?}"
    );
    if before != after {
        let msg = format!("host clipboard sequence changed: {before} -> {after}");
        if std::env::var_os("CI").is_some() {
            panic!("{msg}");
        }
        eprintln!("warning: {msg} (the user may have copied something)");
    }
}
