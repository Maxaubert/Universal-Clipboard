//! The Windows clipboard: a message-only listener window and the read and write paths.
#![allow(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use uc_clip::{Clipboard, ClipboardError, Format, LineEnding, RawPart, Snapshot};
use windows_sys::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND, LPARAM, LRESULT,
    WPARAM,
};
use windows_sys::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, EnumClipboardFormats,
    GetClipboardData, GetClipboardFormatNameW, GetClipboardSequenceNumber, OpenClipboard,
    RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, HWND_MESSAGE,
    MSG, PostMessageW, PostQuitMessage, RegisterClassExW, TranslateMessage, WM_CLIPBOARDUPDATE,
    WM_CLOSE, WM_DESTROY, WNDCLASSEXW,
};

use crate::{dib, formats, html};

/// No single clipboard format is copied out if it is larger than this (memory guard). A
/// 100-megapixel DIB is 400 MB, so this is above it.
const MAX_FORMAT_BYTES: usize = 512 << 20;

const CLASS_NAME: &str = "UniversalClipboard.Listener";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn os_error(what: &str) -> ClipboardError {
    // SAFETY: GetLastError has no preconditions.
    let code = unsafe { GetLastError() };
    ClipboardError::Other(format!("{what}:{code}"))
}

/// Register a clipboard format by name and return its ID (the same ID in every process).
pub fn register_format(name: &str) -> Result<u32, ClipboardError> {
    let w = wide(name);
    // SAFETY: `w` is a NUL-terminated UTF-16 string that lives for the call.
    let id = unsafe { RegisterClipboardFormatW(w.as_ptr()) };
    if id == 0 {
        Err(os_error("register-format"))
    } else {
        Ok(id)
    }
}

/// How long [`WinClipboard`] keeps trying when another program holds the clipboard open.
#[derive(Clone, Copy, Debug)]
pub struct Retry {
    /// Number of `OpenClipboard` attempts.
    pub tries: u32,
    /// Wait between attempts.
    pub wait: Duration,
}

impl Default for Retry {
    /// 5 tries over about 500 ms (plan 3.8).
    fn default() -> Self {
        Retry {
            tries: 5,
            wait: Duration::from_millis(100),
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct Ids {
    png: u32,
    html: u32,
    origin: u32,
    exclude: u32,
    history: u32,
    cloud: u32,
    file_group: u32,
}

impl Ids {
    fn register() -> Result<Ids, ClipboardError> {
        Ok(Ids {
            png: register_format(formats::PNG)?,
            html: register_format(formats::HTML)?,
            origin: register_format(formats::ORIGIN)?,
            exclude: register_format(formats::EXCLUDE_MONITOR)?,
            history: register_format(formats::CAN_INCLUDE_IN_HISTORY)?,
            cloud: register_format(formats::CAN_UPLOAD_TO_CLOUD)?,
            file_group: register_format(formats::FILE_GROUP_DESCRIPTOR)?,
        })
    }
}

/// The Windows clipboard backend.
///
/// [`WinClipboard::new`] starts one thread with a message-only window. The window listens
/// with `AddClipboardFormatListener` (no polling) and owns our writes. Each
/// `WM_CLIPBOARDUPDATE` puts the clipboard sequence number on the [`WinClipboard::changes`]
/// channel. The agent waits for the settle time, then calls [`Clipboard::read`].
pub struct WinClipboard {
    hwnd: usize,
    ids: Ids,
    retry: Retry,
    changes: Receiver<u64>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for WinClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WinClipboard")
            .field("hwnd", &self.hwnd)
            .finish_non_exhaustive()
    }
}

thread_local! {
    static CHANGES: RefCell<Option<Sender<u64>>> = const { RefCell::new(None) };
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            // SAFETY: no preconditions.
            let seq = u64::from(unsafe { GetClipboardSequenceNumber() });
            CHANGES.with(|c| {
                if let Some(tx) = c.borrow().as_ref() {
                    let _ = tx.send(seq);
                }
            });
            0
        }
        WM_CLOSE => {
            // SAFETY: `hwnd` is our window, on this thread.
            unsafe { DestroyWindow(hwnd) };
            0
        }
        WM_DESTROY => {
            // SAFETY: `hwnd` is our window; the listener was added on it.
            unsafe {
                RemoveClipboardFormatListener(hwnd);
                PostQuitMessage(0);
            }
            0
        }
        // SAFETY: default handling with the arguments we were given.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

fn create_window() -> Result<HWND, ClipboardError> {
    let class = wide(CLASS_NAME);
    // SAFETY: plain Win32 calls with valid, live arguments. The class struct is fully set.
    unsafe {
        let instance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSEXW = std::mem::zeroed();
        wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
        wc.lpfnWndProc = Some(wndproc);
        wc.hInstance = instance;
        wc.lpszClassName = class.as_ptr();
        if RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(os_error("register-class"));
        }
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            std::ptr::null(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            std::ptr::null_mut(),
            instance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(os_error("create-window"));
        }
        Ok(hwnd)
    }
}

fn window_thread(tx: Sender<u64>, ready: Sender<Result<usize, ClipboardError>>) {
    let hwnd = match create_window() {
        Ok(h) => h,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    CHANGES.with(|c| *c.borrow_mut() = Some(tx));
    // SAFETY: `hwnd` is the window we just made on this thread.
    if unsafe { AddClipboardFormatListener(hwnd) } == 0 {
        let e = os_error("add-listener");
        // SAFETY: as above.
        unsafe { DestroyWindow(hwnd) };
        let _ = ready.send(Err(e));
        return;
    }
    let _ = ready.send(Ok(hwnd as usize));
    // SAFETY: a standard message loop on the thread that owns the window.
    unsafe {
        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// An open clipboard. Closes it on drop, on the thread that opened it.
pub struct Open(std::marker::PhantomData<*const ()>);

impl Drop for Open {
    fn drop(&mut self) {
        // SAFETY: only made after a successful OpenClipboard on this thread.
        unsafe { CloseClipboard() };
    }
}

/// Global memory we own until `SetClipboardData` takes it.
struct Global(HGLOBAL);

impl Global {
    fn new(data: &[u8]) -> Result<Global, ClipboardError> {
        // SAFETY: we allocate at least 1 byte, lock, copy `data.len()` bytes into a block at
        // least that large, and unlock.
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE, data.len().max(1));
            if h.is_null() {
                return Err(ClipboardError::Other("alloc".into()));
            }
            let g = Global(h);
            let p = GlobalLock(h) as *mut u8;
            if p.is_null() {
                return Err(os_error("lock"));
            }
            std::ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
            GlobalUnlock(h);
            Ok(g)
        }
    }

    /// The system took the memory. Do not free it.
    fn release(self) {
        std::mem::forget(self);
    }
}

impl Drop for Global {
    fn drop(&mut self) {
        // SAFETY: we still own the block.
        unsafe { GlobalFree(self.0) };
    }
}

/// Copy one format's bytes out of the open clipboard.
fn get_bytes(format: u32) -> Result<Vec<u8>, ClipboardError> {
    // SAFETY: the clipboard is open (caller holds `Open`). The handle is HGLOBAL for every
    // format we ask for. We copy at most GlobalSize bytes while the block is locked.
    unsafe {
        let h: HANDLE = GetClipboardData(format);
        if h.is_null() {
            return Err(os_error("get-data"));
        }
        let size = GlobalSize(h);
        if size > MAX_FORMAT_BYTES {
            return Err(ClipboardError::Other("format-too-large".into()));
        }
        let p = GlobalLock(h) as *const u8;
        if p.is_null() {
            return Err(os_error("lock"));
        }
        let out = std::slice::from_raw_parts(p, size).to_vec();
        GlobalUnlock(h);
        Ok(out)
    }
}

fn enum_formats() -> Vec<u32> {
    let mut out = Vec::new();
    let mut f = 0;
    loop {
        // SAFETY: the clipboard is open (caller holds `Open`).
        f = unsafe { EnumClipboardFormats(f) };
        if f == 0 {
            return out;
        }
        out.push(f);
    }
}

/// The name of a clipboard format, for diagnostics. Standard formats have no name.
pub fn format_name(format: u32) -> Option<String> {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer is 256 UTF-16 units long and we say so.
    let n = unsafe { GetClipboardFormatNameW(format, buf.as_mut_ptr(), buf.len() as i32) };
    (n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

/// One raw format for [`WinClipboard::write_raw`].
#[derive(Clone, Debug)]
pub enum RawFormat {
    /// A standard format ID, for example [`formats::CF_DIB`].
    Id(u32),
    /// A registered format name.
    Name(String),
}

struct Raw {
    seq: u64,
    formats: HashSet<u32>,
    history: formats::Dword,
    cloud: formats::Dword,
    text: Option<Vec<u8>>,
    html: Option<Vec<u8>>,
    png: Option<Vec<u8>>,
    dibv5: Option<Vec<u8>>,
    dib: Option<Vec<u8>>,
}

impl WinClipboard {
    /// Start the listener window and register our formats.
    pub fn new() -> Result<WinClipboard, ClipboardError> {
        Self::with_retry(Retry::default())
    }

    /// Like [`WinClipboard::new`] with a custom retry policy.
    pub fn with_retry(retry: Retry) -> Result<WinClipboard, ClipboardError> {
        let ids = Ids::register()?;
        let (tx, changes) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("uc-clip-win".into())
            .spawn(move || window_thread(tx, ready_tx))
            .map_err(|_| ClipboardError::Other("spawn".into()))?;
        let hwnd = ready_rx
            .recv()
            .map_err(|_| ClipboardError::Other("window-thread".into()))??;
        Ok(WinClipboard {
            hwnd,
            ids,
            retry,
            changes,
            thread: Some(thread),
        })
    }

    /// Clipboard change events: the sequence number at each `WM_CLIPBOARDUPDATE`. Our own
    /// writes show up here too; the engine skips them (echo layers 1 and 2).
    pub fn changes(&self) -> &Receiver<u64> {
        &self.changes
    }

    /// The current clipboard sequence number. Reads no content.
    pub fn sequence_number() -> u64 {
        // SAFETY: no preconditions.
        u64::from(unsafe { GetClipboardSequenceNumber() })
    }

    fn open(&self) -> Result<Open, ClipboardError> {
        for i in 0..self.retry.tries.max(1) {
            if i > 0 {
                thread::sleep(self.retry.wait);
            }
            // SAFETY: `hwnd` is our live window (the thread runs until drop).
            if unsafe { OpenClipboard(self.hwnd as HWND) } != 0 {
                return Ok(Open(std::marker::PhantomData));
            }
        }
        Err(ClipboardError::Busy)
    }

    /// Keep the clipboard open until the guard is dropped, like a slow program would. For
    /// tests of the busy path.
    #[doc(hidden)]
    pub fn hold_open(&self) -> Result<Open, ClipboardError> {
        self.open()
    }

    /// The IDs and names of every format on the clipboard (including the ones Windows
    /// makes by conversion). For diagnostics and tests; reads no content.
    pub fn list_formats(&mut self) -> Result<Vec<(u32, Option<String>)>, ClipboardError> {
        let _open = self.open()?;
        Ok(enum_formats()
            .into_iter()
            .map(|f| (f, format_name(f)))
            .collect())
    }

    /// The bytes of one format, as stored. For tests and diagnostics.
    pub fn read_raw(&mut self, format: RawFormat) -> Result<Option<Vec<u8>>, ClipboardError> {
        let id = self.format_id(&format)?;
        let _open = self.open()?;
        if !enum_formats().contains(&id) {
            return Ok(None);
        }
        get_bytes(id).map(Some)
    }

    /// Replace the clipboard with exactly these formats and **no** origin marker, the way
    /// another program would. For tests (recorded format sets). Returns the sequence number.
    pub fn write_raw(&mut self, items: &[(RawFormat, Vec<u8>)]) -> Result<u64, ClipboardError> {
        let mut prepared = Vec::with_capacity(items.len());
        for (f, data) in items {
            prepared.push((self.format_id(f)?, Global::new(data)?));
        }
        self.put(prepared)
    }

    fn format_id(&self, f: &RawFormat) -> Result<u32, ClipboardError> {
        match f {
            RawFormat::Id(id) => Ok(*id),
            RawFormat::Name(n) => register_format(n),
        }
    }

    /// Empty the clipboard and set the prepared formats, in one open and close.
    fn put(&self, items: Vec<(u32, Global)>) -> Result<u64, ClipboardError> {
        {
            let _open = self.open()?;
            // SAFETY: the clipboard is open, and our window is the owner.
            if unsafe { EmptyClipboard() } == 0 {
                return Err(os_error("empty"));
            }
            for (format, g) in items {
                // SAFETY: the clipboard is open; `g.0` is a valid moveable global block. On
                // success the system owns it, so we must not free it.
                if unsafe { SetClipboardData(format, g.0) }.is_null() {
                    return Err(os_error("set-data"));
                }
                g.release();
            }
        }
        Ok(Self::sequence_number())
    }

    /// Copy what we need out of the clipboard in one short open. Conversion happens later.
    fn read_raw_state(&self) -> Result<Raw, ClipboardError> {
        let ids = self.ids;
        let _open = self.open()?;
        let seq = Self::sequence_number();
        let formats: HashSet<u32> = enum_formats().into_iter().collect();
        let dword = |id: u32| {
            if formats.contains(&id) {
                formats::Dword::from_bytes(get_bytes(id).ok().as_deref())
            } else {
                formats::Dword::Absent
            }
        };
        let history = dword(ids.history);
        let cloud = dword(ids.cloud);
        let mut raw = Raw {
            seq,
            history,
            cloud,
            text: None,
            html: None,
            png: None,
            dibv5: None,
            dib: None,
            formats,
        };
        let private = formats::is_private(
            raw.formats.contains(&ids.exclude),
            raw.history,
            raw.cloud,
            raw.formats.contains(&ids.origin),
        );
        let files =
            raw.formats.contains(&formats::CF_HDROP) || raw.formats.contains(&ids.file_group);
        if private || files {
            // Never read the content of a private or file copy.
            return Ok(raw);
        }
        if raw.formats.contains(&formats::CF_UNICODETEXT) {
            raw.text = Some(get_bytes(formats::CF_UNICODETEXT)?);
            if raw.formats.contains(&ids.html) {
                raw.html = get_bytes(ids.html).ok();
            }
        }
        if raw.formats.contains(&ids.png) {
            raw.png = get_bytes(ids.png).ok();
        }
        if raw.png.is_none() {
            if raw.formats.contains(&formats::CF_DIBV5) {
                raw.dibv5 = get_bytes(formats::CF_DIBV5).ok();
            }
            if raw.formats.contains(&formats::CF_DIB) {
                raw.dib = get_bytes(formats::CF_DIB).ok();
            }
        }
        Ok(raw)
    }
}

impl Drop for WinClipboard {
    fn drop(&mut self) {
        // SAFETY: posting to our window; if it is already gone the call just fails.
        unsafe { PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0) };
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn png_signature(b: &[u8]) -> bool {
    b.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a])
}

impl Clipboard for WinClipboard {
    /// Read the clipboard: text, HTML (fragment only), and an image as PNG (the registered
    /// `PNG` format if present, else `CF_DIBV5`, else `CF_DIB`, converted). File lists and
    /// private copies come back with flags set and no parts.
    fn read(&mut self) -> Result<Snapshot, ClipboardError> {
        let raw = self.read_raw_state()?;
        let ids = self.ids;
        let has_marker = raw.formats.contains(&ids.origin);
        let private = formats::is_private(
            raw.formats.contains(&ids.exclude),
            raw.history,
            raw.cloud,
            has_marker,
        );
        let has_files =
            raw.formats.contains(&formats::CF_HDROP) || raw.formats.contains(&ids.file_group);
        let mut parts: Vec<RawPart> = Vec::new();
        if let Some(t) = &raw.text {
            parts.push((Format::TextPlain, formats::utf16_to_utf8(t)));
            if let Some(f) = raw.html.as_deref().and_then(html::fragment) {
                parts.push((Format::TextHtml, f.to_vec()));
            }
        }
        let image = raw
            .png
            .as_deref()
            .map(dib::trim_png)
            .filter(|p| png_signature(p))
            .map(<[u8]>::to_vec)
            .or_else(|| raw.dibv5.as_deref().and_then(|d| dib::dib_to_png(d).ok()))
            .or_else(|| raw.dib.as_deref().and_then(|d| dib::dib_to_png(d).ok()));
        if let Some(png) = image {
            parts.push((Format::ImagePng, png));
        }
        Ok(Snapshot {
            seq: raw.seq,
            parts,
            has_marker,
            private,
            has_files,
        })
    }

    /// Replace the clipboard: `CF_UNICODETEXT`, `PNG` plus `CF_DIBV5` (Windows makes
    /// `CF_DIB` and `CF_BITMAP` from it for older apps), our origin marker and
    /// `CanUploadToCloudClipboard = 0`. The marker goes first, so even a write that fails
    /// half way is never sent again.
    fn write(&mut self, parts: &[RawPart]) -> Result<u64, ClipboardError> {
        let ids = self.ids;
        let mut items = vec![
            (ids.origin, Global::new(&[1])?),
            (ids.cloud, Global::new(&0u32.to_le_bytes())?),
        ];
        for (format, data) in parts {
            match format {
                Format::TextPlain => {
                    items.push((
                        formats::CF_UNICODETEXT,
                        Global::new(&formats::utf8_to_utf16z(data))?,
                    ));
                }
                // TODO(M11): build CF_HTML (plan 3.11). Until then the plain text part
                // carries the clip on Windows.
                Format::TextHtml => {}
                Format::ImagePng => {
                    items.push((ids.png, Global::new(data)?));
                    if let Ok(dibv5) = dib::png_to_rgba(data).and_then(|i| dib::rgba_to_dibv5(&i)) {
                        items.push((formats::CF_DIBV5, Global::new(&dibv5)?));
                    }
                }
            }
        }
        self.put(items)
    }

    fn line_ending(&self) -> LineEnding {
        LineEnding::CrLf
    }
}
