//! Real clipboard tests for programs that behave badly: a delayed-render owner that hangs or
//! fails, a "PNG" that is not a PNG, the legacy private marker. Each test runs inside a private
//! window station (winhost), so the user's clipboard is never touched. Review 2026-11-15.
#![cfg(windows)]
#![allow(unsafe_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use uc_clip::{Clipboard, ClipboardError, Format};
use uc_clip_win::dib::{self, Rgba};
use uc_clip_win::formats::{self, CF_DIB, CF_UNICODETEXT};
use uc_clip_win::{RawFormat, WinClipboard};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG,
    RegisterClassExW, TranslateMessage, WM_RENDERFORMAT, WNDCLASSEXW,
};

macro_rules! real_test {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            uc_winhost::isolated_test(stringify!($name), || $body);
        }
    };
}

fn utf16(s: &str) -> Vec<u8> {
    formats::utf8_to_utf16z(s.as_bytes())
}

fn sample(w: u32, h: u32) -> Rgba {
    let pixels = (0..w * h)
        .flat_map(|i| [(i * 5) as u8, (i * 11) as u8, (i * 17) as u8, 255])
        .collect();
    Rgba {
        width: w,
        height: h,
        pixels,
    }
}

fn part(snap: &uc_clip::Snapshot, f: Format) -> Option<&[u8]> {
    snap.parts
        .iter()
        .find(|(g, _)| *g == f)
        .map(|(_, b)| b.as_slice())
}

/// What the owner window does when asked to render: 0 fail, 1 hang 6 s then fail.
static MODE: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn owner_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_RENDERFORMAT {
        if MODE.load(Ordering::SeqCst) == 1 {
            std::thread::sleep(Duration::from_secs(6));
        }
        return 0;
    }
    // SAFETY: default handling with the arguments we were given.
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// A message-only owner window on its own thread. Returns its HWND as usize.
fn start_owner() -> usize {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // SAFETY: plain Win32 calls with valid arguments; the window lives on this thread.
        unsafe {
            let class: Vec<u16> = "UcHostileOwner\0".encode_utf16().collect();
            let inst = GetModuleHandleW(std::ptr::null());
            let mut wc: WNDCLASSEXW = std::mem::zeroed();
            wc.cbSize = std::mem::size_of::<WNDCLASSEXW>() as u32;
            wc.lpfnWndProc = Some(owner_proc);
            wc.hInstance = inst;
            wc.lpszClassName = class.as_ptr();
            RegisterClassExW(&wc);
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
                inst,
                std::ptr::null(),
            );
            tx.send(hwnd as usize).unwrap();
            let mut m: MSG = std::mem::zeroed();
            while GetMessageW(&mut m, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&m);
                DispatchMessageW(&m);
            }
        }
    });
    rx.recv().unwrap()
}

/// Put `delayed` formats (rendered on request by `owner`) and `immediate` ones on the clipboard.
fn set_delayed(owner: usize, delayed: &[u32], immediate: &[(u32, Vec<u8>)]) {
    // SAFETY: the clipboard is opened by `owner`, filled with fresh global blocks (the system
    // takes them) and null handles for delayed formats, then closed.
    unsafe {
        assert!(OpenClipboard(owner as HWND) != 0);
        EmptyClipboard();
        for (f, b) in immediate {
            let h = GlobalAlloc(GMEM_MOVEABLE, b.len().max(1));
            let p = GlobalLock(h) as *mut u8;
            std::ptr::copy_nonoverlapping(b.as_ptr(), p, b.len());
            GlobalUnlock(h);
            SetClipboardData(*f, h);
        }
        for f in delayed {
            SetClipboardData(*f, std::ptr::null_mut());
        }
        CloseClipboard();
    }
}

real_test!(a_hung_owner_does_not_block_the_read, {
    let mut cb = WinClipboard::new().unwrap();
    let owner = start_owner();
    MODE.store(1, Ordering::SeqCst);
    set_delayed(owner, &[CF_UNICODETEXT], &[]);
    let t0 = Instant::now();
    let r = cb.read();
    let took = t0.elapsed();
    assert!(r.is_err(), "the read must give up, got {r:?}");
    assert!(
        took < uc_clip_win::READ_TIMEOUT + Duration::from_millis(800),
        "read took {took:?}"
    );
    // While the first read is still stuck, the next one answers at once.
    let t1 = Instant::now();
    assert_eq!(cb.read().unwrap_err(), ClipboardError::Busy);
    assert!(t1.elapsed() < Duration::from_millis(200));
    // Once the owner gives up, reads work again (poll, so a slow machine has time).
    MODE.store(0, Ordering::SeqCst);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if cb.write(&[(Format::TextPlain, b"fine".to_vec())]).is_ok()
            && let Ok(s) = cb.read()
            && part(&s, Format::TextPlain) == Some(&b"fine"[..])
        {
            break;
        }
        assert!(Instant::now() < deadline, "reads never recovered");
        std::thread::sleep(Duration::from_millis(250));
    }
});

real_test!(unreadable_text_keeps_a_good_image, {
    let mut cb = WinClipboard::new().unwrap();
    let owner = start_owner();
    MODE.store(0, Ordering::SeqCst);
    let png_id = uc_clip_win::register_format(formats::PNG).unwrap();
    let png = dib::rgba_to_png(&sample(12, 12)).unwrap();
    set_delayed(owner, &[CF_UNICODETEXT], &[(png_id, png)]);
    let s = cb.read().unwrap();
    assert!(part(&s, Format::TextPlain).is_none());
    assert!(
        part(&s, Format::ImagePng).is_some(),
        "the image must survive"
    );
});

real_test!(a_fake_png_falls_back_to_the_dib, {
    let mut cb = WinClipboard::new().unwrap();
    // CF_DIB accepts a V5 header too.
    let dibv = dib::rgba_to_dibv5(&sample(10, 8)).unwrap();
    cb.write_raw(&[
        (
            RawFormat::Name(formats::PNG.into()),
            b"not a png at all".to_vec(),
        ),
        (RawFormat::Id(CF_DIB), dibv),
    ])
    .unwrap();
    let s = cb.read().unwrap();
    let png = part(&s, Format::ImagePng).expect("the DIB must be used");
    assert_eq!(dib::png_to_rgba(png).unwrap().width, 10);
});

real_test!(the_legacy_viewer_ignore_marker_is_private, {
    let mut cb = WinClipboard::new().unwrap();
    cb.write_raw(&[
        (RawFormat::Id(CF_UNICODETEXT), utf16("hunter2")),
        (RawFormat::Name(formats::VIEWER_IGNORE.into()), Vec::new()),
    ])
    .unwrap();
    let s = cb.read().unwrap();
    assert!(s.private);
    assert!(s.parts.is_empty(), "a private copy is never read");
});
