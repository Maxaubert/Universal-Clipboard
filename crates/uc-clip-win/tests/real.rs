//! Real clipboard tests. Each test re-runs itself inside a private window station made by
//! `tools/winhost` (spike S3), so the user's clipboard and screen are never touched.
//!
//! The format sets below copy what real programs put on the clipboard (M4 accept list):
//! Notepad-like text, a browser copy, a screenshot-like bitmap, an Explorer file copy.
#![cfg(windows)]

use std::time::Duration;

use uc_clip::{Clipboard, ClipboardError, Format, LineEnding};
use uc_clip_win::dib::{self, Rgba};
use uc_clip_win::formats::{self, CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use uc_clip_win::{RawFormat, Retry, WinClipboard};

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

fn name(n: &str) -> RawFormat {
    RawFormat::Name(n.into())
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

/// Wait for the change event with this sequence number (or a later one).
fn wait_for_event(cb: &WinClipboard, seq: u64) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        match cb.changes().recv_timeout(left) {
            Ok(s) if s >= seq => return,
            Ok(_) => continue,
            Err(_) => panic!("no WM_CLIPBOARDUPDATE for seq {seq}"),
        }
    }
}

real_test!(runs_in_private_station, {
    let station = uc_winhost::current_station().unwrap();
    assert_ne!(station.to_ascii_lowercase(), "winsta0");
    let mut cb = WinClipboard::new().unwrap();
    // The station is shared by runs one after another, so empty it first.
    cb.write_raw(&[]).unwrap();
    let snap = cb.read().unwrap();
    assert!(snap.parts.is_empty());
    assert!(!snap.has_marker && !snap.private && !snap.has_files);
    assert_eq!(cb.line_ending(), LineEnding::CrLf);
});

real_test!(our_text_write_round_trips_with_marker, {
    let mut cb = WinClipboard::new().unwrap();
    let text = "line one\r\nlinje to æøå 😀";
    let seq = cb
        .write(&[(Format::TextPlain, text.as_bytes().to_vec())])
        .unwrap();
    wait_for_event(&cb, seq);
    let snap = cb.read().unwrap();
    assert_eq!(snap.seq, seq);
    assert!(snap.has_marker);
    assert!(
        !snap.private,
        "our own CanUploadToCloudClipboard=0 must not mark it private"
    );
    assert_eq!(part(&snap, Format::TextPlain), Some(text.as_bytes()));
    // The cloud flag is really there and 0.
    let cloud = cb
        .read_raw(name(formats::CAN_UPLOAD_TO_CLOUD))
        .unwrap()
        .unwrap();
    assert_eq!(&cloud[..4], &[0, 0, 0, 0]);
    // We do not opt out of history (our clips stay in Win+V).
    assert!(
        cb.read_raw(name(formats::CAN_INCLUDE_IN_HISTORY))
            .unwrap()
            .is_none()
    );
});

real_test!(notepad_like_text, {
    let mut cb = WinClipboard::new().unwrap();
    // Notepad sets CF_UNICODETEXT; Windows adds CF_TEXT, CF_OEMTEXT and CF_LOCALE.
    let seq = cb
        .write_raw(&[(RawFormat::Id(CF_UNICODETEXT), utf16("Hei\r\nverden"))])
        .unwrap();
    let snap = cb.read().unwrap();
    assert_eq!(snap.seq, seq);
    assert!(!snap.has_marker && !snap.private && !snap.has_files);
    assert_eq!(
        snap.parts,
        vec![(Format::TextPlain, b"Hei\r\nverden".to_vec())]
    );
    // The core turns it into a valid clip.
    let wire = uc_core::text::to_wire(part(&snap, Format::TextPlain).unwrap()).unwrap();
    assert_eq!(wire, "Hei\nverden");
});

real_test!(ansi_only_text_is_converted_by_windows, {
    let mut cb = WinClipboard::new().unwrap();
    cb.write_raw(&[(RawFormat::Id(1 /* CF_TEXT */), b"plain ascii\0".to_vec())])
        .unwrap();
    let snap = cb.read().unwrap();
    assert_eq!(part(&snap, Format::TextPlain), Some(&b"plain ascii"[..]));
});

real_test!(browser_copy_text_html_and_png, {
    let mut cb = WinClipboard::new().unwrap();
    let frag = "<b>Fet</b> tekst";
    let pre = "<html><body>\r\n<!--StartFragment-->";
    let post = "<!--EndFragment-->\r\n</body></html>";
    let head_len = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
    let sf = head_len + pre.len();
    let ef = sf + frag.len();
    let eh = ef + post.len();
    let cf_html = format!(
        "Version:0.9\r\nStartHTML:{head_len:010}\r\nEndHTML:{eh:010}\r\nStartFragment:{sf:010}\r\nEndFragment:{ef:010}\r\n{pre}{frag}{post}\0"
    );
    let png = dib::rgba_to_png(&sample(3, 2)).unwrap();
    cb.write_raw(&[
        (RawFormat::Id(CF_UNICODETEXT), utf16("Fet tekst")),
        (name(formats::HTML), cf_html.into_bytes()),
        (name(formats::PNG), png.clone()),
        (
            RawFormat::Id(CF_DIB),
            dib::rgba_to_dibv5(&sample(3, 2)).unwrap(),
        ),
    ])
    .unwrap();
    let snap = cb.read().unwrap();
    assert_eq!(part(&snap, Format::TextPlain), Some(&b"Fet tekst"[..]));
    assert_eq!(part(&snap, Format::TextHtml), Some(frag.as_bytes()));
    // The registered PNG wins over the DIB, byte for byte (trailing padding cut).
    assert_eq!(part(&snap, Format::ImagePng), Some(png.as_slice()));
    // And the whole snapshot is a valid clip for the core.
    uc_core::Clip::from_parts(vec![
        (Format::TextPlain, b"Fet tekst".to_vec()),
        (Format::TextHtml, frag.as_bytes().to_vec()),
        (Format::ImagePng, png),
    ])
    .unwrap();
});

real_test!(screenshot_like_dibv5_becomes_png, {
    let mut cb = WinClipboard::new().unwrap();
    let mut img = sample(7, 5);
    img.pixels[3] = 128; // one half-transparent pixel
    cb.write_raw(&[(RawFormat::Id(CF_DIBV5), dib::rgba_to_dibv5(&img).unwrap())])
        .unwrap();
    let snap = cb.read().unwrap();
    let png = part(&snap, Format::ImagePng).expect("image part");
    assert_eq!(dib::png_to_rgba(png).unwrap(), img);
    assert!(snap.parts.iter().all(|(f, _)| *f == Format::ImagePng));
});

real_test!(plain_24_bit_dib_becomes_png, {
    let mut cb = WinClipboard::new().unwrap();
    // 2x2 bottom-up 24-bit, the classic Print Screen shape.
    let mut d = Vec::new();
    d.extend_from_slice(&40u32.to_le_bytes());
    d.extend_from_slice(&2i32.to_le_bytes());
    d.extend_from_slice(&2i32.to_le_bytes());
    d.extend_from_slice(&1u16.to_le_bytes());
    d.extend_from_slice(&24u16.to_le_bytes());
    d.extend_from_slice(&[0u8; 24]);
    d.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0, 7, 8, 9, 10, 11, 12, 0, 0]);
    cb.write_raw(&[(RawFormat::Id(CF_DIB), d)]).unwrap();
    let snap = cb.read().unwrap();
    let img = dib::png_to_rgba(part(&snap, Format::ImagePng).unwrap()).unwrap();
    assert_eq!(
        img.pixels,
        vec![9, 8, 7, 255, 12, 11, 10, 255, 3, 2, 1, 255, 6, 5, 4, 255]
    );
});

real_test!(our_image_write_serves_png_and_dib, {
    let mut cb = WinClipboard::new().unwrap();
    let mut img = sample(6, 4);
    img.pixels[7] = 0; // a transparent pixel
    let png = dib::rgba_to_png(&img).unwrap();
    cb.write(&[(Format::ImagePng, png.clone())]).unwrap();
    // PNG-aware apps get our bytes.
    let snap = cb.read().unwrap();
    assert!(snap.has_marker);
    assert_eq!(part(&snap, Format::ImagePng), Some(png.as_slice()));
    // Older apps get CF_DIBV5, and Windows makes CF_DIB from it.
    let v5 = cb.read_raw(RawFormat::Id(CF_DIBV5)).unwrap().unwrap();
    assert_eq!(dib::dib_to_rgba(&v5).unwrap(), img);
    let plain = cb.read_raw(RawFormat::Id(CF_DIB)).unwrap().unwrap();
    let back = dib::dib_to_rgba(&plain).unwrap();
    assert_eq!((back.width, back.height), (6, 4));
    // Colors survive the conversion (alpha handling of CF_DIB is up to Windows).
    for (a, b) in back
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(img.pixels.as_chunks::<4>().0.iter())
    {
        if b[3] == 255 {
            assert_eq!(&a[..3], &b[..3]);
        }
    }
});

real_test!(text_and_image_write, {
    let mut cb = WinClipboard::new().unwrap();
    let png = dib::rgba_to_png(&sample(2, 2)).unwrap();
    cb.write(&[
        (Format::TextPlain, b"a\r\nb".to_vec()),
        (Format::TextHtml, b"<i>a</i>".to_vec()),
        (Format::ImagePng, png.clone()),
    ])
    .unwrap();
    let snap = cb.read().unwrap();
    assert_eq!(part(&snap, Format::TextPlain), Some(&b"a\r\nb"[..]));
    assert_eq!(part(&snap, Format::ImagePng), Some(png.as_slice()));
    // Writing HTML waits for M11.
    assert_eq!(part(&snap, Format::TextHtml), None);
});

real_test!(explorer_file_copy_is_skipped, {
    let mut cb = WinClipboard::new().unwrap();
    // DROPFILES header (20 bytes, wide) + "C:\a.txt\0\0".
    let mut drop = Vec::new();
    drop.extend_from_slice(&20u32.to_le_bytes());
    drop.extend_from_slice(&[0u8; 12]);
    drop.extend_from_slice(&1u32.to_le_bytes());
    drop.extend_from_slice(&utf16("C:\\a.txt"));
    drop.extend_from_slice(&[0, 0]);
    cb.write_raw(&[
        (RawFormat::Id(CF_HDROP), drop),
        (name("FileNameW"), utf16("C:\\a.txt")),
        (name("Preferred DropEffect"), 5u32.to_le_bytes().to_vec()),
        (RawFormat::Id(CF_UNICODETEXT), utf16("a.txt")),
    ])
    .unwrap();
    let snap = cb.read().unwrap();
    assert!(snap.has_files);
    assert!(snap.parts.is_empty(), "content of a file copy is not read");
});

real_test!(virtual_file_copy_is_skipped, {
    let mut cb = WinClipboard::new().unwrap();
    cb.write_raw(&[
        (name(formats::FILE_GROUP_DESCRIPTOR), vec![0u8; 600]),
        (RawFormat::Id(CF_UNICODETEXT), utf16("mail.msg")),
    ])
    .unwrap();
    assert!(cb.read().unwrap().has_files);
});

real_test!(private_markers_are_honored, {
    let mut cb = WinClipboard::new().unwrap();
    let cases: Vec<(RawFormat, Vec<u8>, bool)> = vec![
        (name(formats::EXCLUDE_MONITOR), vec![0], true),
        (
            name(formats::CAN_INCLUDE_IN_HISTORY),
            0u32.to_le_bytes().to_vec(),
            true,
        ),
        (
            name(formats::CAN_UPLOAD_TO_CLOUD),
            0u32.to_le_bytes().to_vec(),
            true,
        ),
        (
            name(formats::CAN_INCLUDE_IN_HISTORY),
            1u32.to_le_bytes().to_vec(),
            false,
        ),
        (
            name(formats::CAN_UPLOAD_TO_CLOUD),
            1u32.to_le_bytes().to_vec(),
            false,
        ),
        // Too short to read: counts as private (plan 3.10).
        (name(formats::CAN_INCLUDE_IN_HISTORY), vec![], true),
    ];
    for (marker, value, private) in cases {
        cb.write_raw(&[
            (RawFormat::Id(CF_UNICODETEXT), utf16("hunter2")),
            (marker.clone(), value.clone()),
        ])
        .unwrap();
        let snap = cb.read().unwrap();
        assert_eq!(snap.private, private, "{marker:?} = {value:?}");
        if private {
            assert!(snap.parts.is_empty(), "private content is not read");
        } else {
            assert_eq!(part(&snap, Format::TextPlain), Some(&b"hunter2"[..]));
        }
    }
});

real_test!(change_events_are_pushed, {
    let cb = WinClipboard::new().unwrap();
    let mut writer = WinClipboard::new().unwrap();
    for i in 0..3 {
        let seq = writer
            .write_raw(&[(RawFormat::Id(CF_UNICODETEXT), utf16(&format!("n{i}")))])
            .unwrap();
        wait_for_event(&cb, seq);
    }
    // No change, no event.
    assert!(
        cb.changes()
            .recv_timeout(Duration::from_millis(300))
            .is_err()
    );
});

real_test!(busy_clipboard_gives_busy_then_recovers, {
    let mut cb = WinClipboard::with_retry(Retry {
        tries: 3,
        wait: Duration::from_millis(20),
    })
    .unwrap();
    let holder = WinClipboard::new().unwrap();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    // Another "program" holds the clipboard open.
    let t = std::thread::spawn(move || {
        let held = holder.hold_open().unwrap();
        locked_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        drop(held);
    });
    locked_rx.recv().unwrap();
    assert_eq!(cb.read(), Err(ClipboardError::Busy));
    assert_eq!(
        cb.write(&[(Format::TextPlain, b"x".to_vec())]),
        Err(ClipboardError::Busy)
    );
    release_tx.send(()).unwrap();
    t.join().unwrap();
    cb.write(&[(Format::TextPlain, b"x".to_vec())]).unwrap();
    assert_eq!(
        part(&cb.read().unwrap(), Format::TextPlain),
        Some(&b"x"[..])
    );
});

real_test!(engine_skips_its_own_write_and_sends_user_copy, {
    // The agent's view: a write of ours is skipped, a foreign copy is a clip.
    let mut cb = WinClipboard::new().unwrap();
    cb.write(&[(Format::TextPlain, b"from peer".to_vec())])
        .unwrap();
    assert!(cb.read().unwrap().has_marker);
    cb.write_raw(&[(RawFormat::Id(CF_UNICODETEXT), utf16("typed by user"))])
        .unwrap();
    let snap = cb.read().unwrap();
    assert!(!snap.has_marker);
    let text = uc_core::text::to_wire(part(&snap, Format::TextPlain).unwrap()).unwrap();
    uc_core::Clip::from_parts(vec![(Format::TextPlain, text.into_bytes())]).unwrap();
});
