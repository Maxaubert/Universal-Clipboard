//! Windows helpers: the user's SID, process checks on the pipe, the pipe ACL, the `Run`
//! registry value, and suspend/resume notifications.
#![allow(unsafe_code)]

use std::ffi::c_void;
use std::io;
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId};
use windows_sys::Win32::System::Power::{
    DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS, PowerRegisterSuspendResumeNotification,
};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
    RegCloseKey, RegCreateKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW,
    RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle we opened and own.
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn token_sid(token: HANDLE) -> io::Result<String> {
    let mut len = 0u32;
    // SAFETY: size query with a null buffer.
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: the buffer is `len` bytes long and 8-byte aligned.
    if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) } == 0
    {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call filled a TOKEN_USER at the start of the buffer.
    let user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let mut s: *mut u16 = std::ptr::null_mut();
    // SAFETY: the SID points into `buf`, which lives until the end of this function.
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut s) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `s` is a NUL-terminated string from the OS; freed with LocalFree below.
    let out = unsafe {
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let v = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        v
    };
    Ok(out)
}

/// The SID of the user that runs this process, as `S-1-5-21-...`.
pub fn current_user_sid() -> io::Result<String> {
    static SID: OnceLock<String> = OnceLock::new();
    if let Some(s) = SID.get() {
        return Ok(s.clone());
    }
    let mut t: HANDLE = std::ptr::null_mut();
    // SAFETY: the pseudo handle of this process; `t` is closed by `Handle`.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut t) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let t = Handle(t);
    let s = token_sid(t.0)?;
    let _ = SID.set(s.clone());
    Ok(s)
}

/// The user SID of another process.
pub fn process_sid(pid: u32) -> io::Result<String> {
    // SAFETY: plain call; the handle is closed by `Handle`.
    let p = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if p.is_null() {
        return Err(io::Error::last_os_error());
    }
    let p = Handle(p);
    let mut t: HANDLE = std::ptr::null_mut();
    // SAFETY: `p` is a live process handle.
    if unsafe { OpenProcessToken(p.0, TOKEN_QUERY, &mut t) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let t = Handle(t);
    token_sid(t.0)
}

/// The process ID at the other end of a pipe. `server` true asks for the server's ID (the
/// client checks the agent), false for the client's (the agent checks the caller).
pub fn pipe_peer_pid(
    pipe: &impl std::os::windows::io::AsRawHandle,
    server: bool,
) -> io::Result<u32> {
    let pipe = pipe.as_raw_handle() as HANDLE;
    let mut pid = 0u32;
    // SAFETY: `pipe` is a live pipe handle owned by the caller.
    let ok = unsafe {
        if server {
            GetNamedPipeServerProcessId(pipe, &mut pid)
        } else {
            GetNamedPipeClientProcessId(pipe, &mut pid)
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(pid)
}

/// Security attributes that give the current user, and nobody else, full access (plan 3.2).
pub struct OwnerOnly {
    sd: PSECURITY_DESCRIPTOR,
    /// The attributes, pointing at the descriptor.
    pub attrs: SECURITY_ATTRIBUTES,
}

// SAFETY: the descriptor is owned memory that is never changed after creation.
unsafe impl Send for OwnerOnly {}
// SAFETY: as above, read only.
unsafe impl Sync for OwnerOnly {}

impl OwnerOnly {
    /// `D:P(A;;GA;;;<SID>)`: a protected DACL with one entry, the user.
    pub fn new() -> io::Result<OwnerOnly> {
        let sddl = wide(&format!("D:P(A;;GA;;;{})", current_user_sid()?));
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: valid NUL-terminated SDDL; `sd` is freed in Drop.
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut sd,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(OwnerOnly {
            sd,
            attrs: SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            },
        })
    }

    /// Pointer for `create_with_security_attributes_raw`.
    pub fn as_ptr(&mut self) -> *mut c_void {
        (&mut self.attrs as *mut SECURITY_ATTRIBUTES).cast()
    }
}

impl Drop for OwnerOnly {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe { LocalFree(self.sd) };
    }
}

fn check(code: u32) -> io::Result<()> {
    if code == 0 {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code as i32))
    }
}

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: a key we opened.
        unsafe { RegCloseKey(self.0) };
    }
}

fn create_key(path: &str) -> io::Result<Key> {
    let p = wide(path);
    let mut k: HKEY = std::ptr::null_mut();
    // SAFETY: valid arguments; `k` is closed by `Key`.
    check(unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            p.as_ptr(),
            0,
            std::ptr::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            std::ptr::null(),
            &mut k,
            std::ptr::null_mut(),
        )
    })?;
    Ok(Key(k))
}

/// Set a `REG_SZ` value under `HKCU\<path>`.
pub fn reg_set(path: &str, name: &str, value: &str) -> io::Result<()> {
    let k = create_key(path)?;
    let n = wide(name);
    let v = wide(value);
    // SAFETY: `v` is a NUL-terminated UTF-16 buffer of the given byte length.
    check(unsafe {
        RegSetValueExW(
            k.0,
            n.as_ptr(),
            0,
            REG_SZ,
            v.as_ptr().cast(),
            (v.len() * 2) as u32,
        )
    })
}

/// Delete a value under `HKCU\<path>`. A missing key or value is fine.
pub fn reg_delete(path: &str, name: &str) -> io::Result<()> {
    let Some(k) = open_key(path)? else {
        return Ok(());
    };
    let n = wide(name);
    // SAFETY: plain call.
    match unsafe { RegDeleteValueW(k.0, n.as_ptr()) } {
        0 | 2 => Ok(()),
        e => Err(io::Error::from_raw_os_error(e as i32)),
    }
}

fn open_key(path: &str) -> io::Result<Option<Key>> {
    let p = wide(path);
    let mut k: HKEY = std::ptr::null_mut();
    // SAFETY: plain call.
    match unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            p.as_ptr(),
            0,
            KEY_SET_VALUE | KEY_QUERY_VALUE,
            &mut k,
        )
    } {
        0 => Ok(Some(Key(k))),
        2 => Ok(None),
        e => Err(io::Error::from_raw_os_error(e as i32)),
    }
}

/// True if the value exists under `HKCU\<path>`.
pub fn reg_exists(path: &str, name: &str) -> bool {
    let Ok(Some(k)) = open_key(path) else {
        return false;
    };
    let n = wide(name);
    let mut len = 0u32;
    // SAFETY: size query only.
    unsafe {
        RegQueryValueExW(
            k.0,
            n.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut len,
        ) == 0
    }
}

/// Delete `HKCU\<path>` and everything under it (test sandbox cleanup).
pub fn reg_delete_tree(path: &str) -> io::Result<()> {
    let p = wide(path);
    // SAFETY: plain call.
    match unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, p.as_ptr()) } {
        0 | 2 => Ok(()),
        e => Err(io::Error::from_raw_os_error(e as i32)),
    }
}

static RESUME: OnceLock<tokio::sync::mpsc::UnboundedSender<()>> = OnceLock::new();

unsafe extern "system" fn on_power(_ctx: *const c_void, kind: u32, _s: *const c_void) -> u32 {
    // PBT_APMRESUMESUSPEND (7) and PBT_APMRESUMEAUTOMATIC (0x12).
    if (kind == 7 || kind == 0x12)
        && let Some(tx) = RESUME.get()
    {
        let _ = tx.send(());
    }
    0
}

/// Ask Windows to tell us about resume from sleep. Each resume sends `()` on `tx`.
pub fn watch_resume(tx: tokio::sync::mpsc::UnboundedSender<()>) -> io::Result<()> {
    if RESUME.set(tx).is_err() {
        return Ok(());
    }
    // The parameters must live for the whole process.
    let params: &'static mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS =
        Box::leak(Box::new(DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS {
            Callback: Some(on_power),
            Context: std::ptr::null_mut(),
        }));
    let mut reg: *mut c_void = std::ptr::null_mut();
    // DEVICE_NOTIFY_CALLBACK = 2: `recipient` is the parameters block.
    // SAFETY: the block is leaked, so it outlives the registration.
    check(unsafe {
        PowerRegisterSuspendResumeNotification(
            2,
            (params as *mut DEVICE_NOTIFY_SUBSCRIBE_PARAMETERS).cast(),
            &mut reg,
        )
    })
}

/// Read a string value (`REG_SZ` or `REG_EXPAND_SZ`) under `HKCU\<path>`, with its type.
pub fn reg_get(path: &str, name: &str) -> io::Result<Option<(String, u32)>> {
    let Some(k) = open_key(path)? else {
        return Ok(None);
    };
    let n = wide(name);
    let mut ty = 0u32;
    let mut len = 0u32;
    // SAFETY: size query only.
    let r = unsafe {
        RegQueryValueExW(
            k.0,
            n.as_ptr(),
            std::ptr::null(),
            &mut ty,
            std::ptr::null_mut(),
            &mut len,
        )
    };
    if r == 2 {
        return Ok(None);
    }
    check(r)?;
    let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
    let mut len2 = (buf.len() * 2) as u32;
    // SAFETY: `buf` has `len2` bytes.
    check(unsafe {
        RegQueryValueExW(
            k.0,
            n.as_ptr(),
            std::ptr::null(),
            &mut ty,
            buf.as_mut_ptr().cast(),
            &mut len2,
        )
    })?;
    let chars = (len2 as usize / 2).min(buf.len());
    let s = String::from_utf16_lossy(&buf[..chars]);
    Ok(Some((s.trim_end_matches('\0').to_string(), ty)))
}

/// Set a string value of type `ty` (`REG_SZ` 1 or `REG_EXPAND_SZ` 2) under `HKCU\<path>`.
pub fn reg_set_typed(path: &str, name: &str, value: &str, ty: u32) -> io::Result<()> {
    let k = create_key(path)?;
    let n = wide(name);
    let v = wide(value);
    // SAFETY: `v` is a NUL-terminated UTF-16 buffer of the given byte length.
    check(unsafe {
        RegSetValueExW(
            k.0,
            n.as_ptr(),
            0,
            ty,
            v.as_ptr().cast(),
            (v.len() * 2) as u32,
        )
    })
}

/// Tell running programs (Explorer) that the environment of the user changed, so new
/// terminals see the new PATH. Waits at most 2 s.
pub fn broadcast_env_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
    };
    let s = wide("Environment");
    let mut out = 0usize;
    // SAFETY: plain call with a NUL-terminated string.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            0,
            s.as_ptr() as isize,
            SMTO_ABORTIFHUNG,
            2000,
            &mut out,
        )
    };
}

/// True if this process runs elevated (as administrator).
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{TOKEN_ELEVATION, TokenElevation};
    let mut token: HANDLE = std::ptr::null_mut();
    // SAFETY: our own process token, closed below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let mut e = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut len = 0u32;
    // SAFETY: `e` has the size passed.
    let ok = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            (&mut e as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
    };
    // SAFETY: opened above.
    unsafe { CloseHandle(token) };
    ok != 0 && e.TokenIsElevated != 0
}

/// Run `exe args` elevated: Windows shows one UAC prompt. Waits for it and returns its exit
/// code. A "No" in the prompt is an error (`ERROR_CANCELLED`, 1223).
pub fn run_elevated(exe: &std::path::Path, args: &str) -> io::Result<u32> {
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    let verb = wide("runas");
    let file = wide(&exe.display().to_string());
    let params = wide(args);
    // SAFETY: zeroed is a valid SHELLEXECUTEINFOW; we fill the fields we use.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0; // SW_HIDE
    // SAFETY: `info` and the strings live through the call.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.hProcess.is_null() {
        return Err(io::Error::other("no process"));
    }
    let mut code = 0u32;
    // SAFETY: the process handle we asked for; closed below.
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    Ok(code)
}

/// True if this process runs from an MSIX package (the Microsoft Store build).
pub fn is_packaged() -> bool {
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    let mut len = 0u32;
    // SAFETY: size query; APPMODEL_ERROR_NO_PACKAGE (15700) means not packaged.
    let r = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
    r != 15700
}

/// Stop our standard handles from being inherited by programs we start. Rust gives a child
/// its own handles for the `Stdio` we choose, but Windows also passes every other inheritable
/// handle. So a detached agent started by `uc` would keep `uc`'s output pipe open, and a
/// script that reads `uc status` would wait for ever.
pub fn no_inherit_std_handles() {
    use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};
    use windows_sys::Win32::System::Console::{
        GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };
    for id in [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
        // SAFETY: plain calls; a null or invalid handle makes SetHandleInformation fail, which
        // is fine.
        unsafe {
            let h = GetStdHandle(id);
            if !h.is_null() {
                SetHandleInformation(h, HANDLE_FLAG_INHERIT, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sid_looks_right_and_matches_our_process() {
        let s = current_user_sid().unwrap();
        assert!(s.starts_with("S-1-"), "{s}");
        assert_eq!(process_sid(std::process::id()).unwrap(), s);
    }

    #[test]
    fn registry_round_trip_in_a_sandbox_key() {
        let path = format!(
            r"Software\UniversalClipboard-test\reg-{}",
            std::process::id()
        );
        reg_set(&path, "v", "x").unwrap();
        assert!(reg_exists(&path, "v"));
        reg_delete(&path, "v").unwrap();
        assert!(!reg_exists(&path, "v"));
        reg_delete_tree(&path).unwrap();
    }
}
