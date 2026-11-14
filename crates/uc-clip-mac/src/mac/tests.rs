//! Real pasteboard tests. Each test uses its own private named pasteboard, never the general
//! one, so the user's clipboard is untouched and no prompt appears.

use super::*;
use crate::poll::Poller;
use std::sync::atomic::{AtomicU32, Ordering};

/// A private pasteboard that is freed when the test ends.
struct Board(Option<MacClipboard>);

impl Board {
    fn new() -> Self {
        static N: AtomicU32 = AtomicU32::new(0);
        let name = format!(
            "io.github.maxaubert.uclip.test.{}.{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        );
        let cb = MacClipboard::named(&name);
        assert_eq!(cb.name(), name);
        assert_ne!(
            cb.name(),
            GENERAL_NAME,
            "a test must never use the general pasteboard"
        );
        Self(Some(cb))
    }
}

impl std::ops::Deref for Board {
    type Target = MacClipboard;
    fn deref(&self) -> &MacClipboard {
        self.0.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for Board {
    fn deref_mut(&mut self) -> &mut MacClipboard {
        self.0.as_mut().unwrap()
    }
}

impl Drop for Board {
    fn drop(&mut self) {
        if let Some(cb) = self.0.take() {
            cb.release_globally();
        }
    }
}

fn png_rgba(width: u32, height: u32, seed: u8) -> Vec<u8> {
    // Opaque pixels: AppKit may premultiply alpha, which would change see-through pixels.
    let rgba: Vec<u8> = (0..width * height)
        .flat_map(|i| {
            let v = (i as u8).wrapping_mul(37).wrapping_add(seed);
            [v, v.wrapping_add(80), v.wrapping_mul(3), 255]
        })
        .collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut w = enc.write_header().unwrap();
        w.write_image_data(&rgba).unwrap();
    }
    out
}

/// The content hash that the sync engine uses for images (width, height, RGBA8 pixels).
fn pixels(png: &[u8]) -> blake3::Hash {
    let mut h = blake3::Hasher::new();
    uc_core::image::hash_pixels(png, &mut h).expect("decodable PNG");
    h.finalize()
}

fn part(snap: &Snapshot, f: Format) -> Option<&[u8]> {
    snap.parts
        .iter()
        .find(|(x, _)| *x == f)
        .map(|(_, b)| b.as_slice())
}

#[test]
fn general_name_constant_matches_appkit() {
    // Reads only the name constant from AppKit; the general pasteboard is not opened.
    // SAFETY: `NSPasteboardNameGeneral` is an immutable string constant from AppKit.
    let name = unsafe { objc2_app_kit::NSPasteboardNameGeneral }.to_string();
    assert_eq!(name, GENERAL_NAME);
}

#[test]
fn from_env_uses_the_named_test_pasteboard() {
    let b = Board::new();
    let name = b.name();
    // SAFETY of the test: nextest runs each test in its own process, and `cargo test` callers
    // do not read this variable anywhere else.
    unsafe { std::env::set_var(TEST_PASTEBOARD_ENV, &name) };
    let cb = MacClipboard::from_env();
    unsafe { std::env::remove_var(TEST_PASTEBOARD_ENV) };
    assert_eq!(cb.name(), name);
}

#[test]
fn empty_pasteboard_reads_as_empty() {
    let mut b = Board::new();
    b.set_raw(&[]);
    let s = b.read().unwrap();
    assert!(s.parts.is_empty());
    assert!(!s.has_marker && !s.private && !s.has_files && !s.remote);
    assert_eq!(s.seq, b.change_count());
}

#[test]
fn text_from_another_app() {
    let mut b = Board::new();
    let text = "Hei verden\n\u{e6}\u{f8}\u{e5} \u{1F600}";
    let seq = b.set_raw(&[(types::TEXT, text.as_bytes())]);
    let s = b.read().unwrap();
    assert_eq!(s.seq, seq);
    assert!(!s.has_marker);
    assert_eq!(part(&s, Format::TextPlain), Some(text.as_bytes()));
}

#[test]
fn utf16_text_is_converted() {
    let mut b = Board::new();
    let text = "gr\u{f8}t";
    let utf16: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
    b.set_raw(&[("public.utf16-plain-text", &utf16)]);
    let s = b.read().unwrap();
    assert_eq!(part(&s, Format::TextPlain), Some(text.as_bytes()));
}

#[test]
fn html_next_to_text() {
    let mut b = Board::new();
    b.set_raw(&[(types::TEXT, b"bold"), (types::HTML, b"<b>bold</b>")]);
    let s = b.read().unwrap();
    assert_eq!(part(&s, Format::TextPlain), Some(&b"bold"[..]));
    assert_eq!(part(&s, Format::TextHtml), Some(&b"<b>bold</b>"[..]));
}

#[test]
fn our_write_carries_marker_and_current_host_only() {
    let mut b = Board::new();
    let before = b.change_count();
    let seq = b
        .write(&[
            (Format::TextPlain, b"from pc".to_vec()),
            (Format::TextHtml, b"<i>from pc</i>".to_vec()),
        ])
        .unwrap();
    assert_ne!(seq, before);
    assert_eq!(seq, b.change_count());
    let t = b.types().unwrap();
    assert!(t.iter().any(|x| x == types::MARKER), "{t:?}");
    assert_eq!(b.data(types::TEXT).unwrap(), b"from pc");
    // AppKit needs the charset tag to read UTF-8 HTML (types::with_charset_meta).
    assert_eq!(
        b.data(types::HTML).unwrap(),
        b"<meta charset=\"utf-8\"><i>from pc</i>"
    );

    // The read sees the marker and the counter, and does not read the content.
    let s = b.read().unwrap();
    assert!(s.has_marker);
    assert_eq!(s.seq, seq);
    assert!(s.parts.is_empty());
}

#[test]
fn write_replaces_everything() {
    let mut b = Board::new();
    b.set_raw(&[(types::TEXT, b"old"), (types::PRIVATE[0], b"")]);
    b.write(&[(Format::TextPlain, b"new".to_vec())]).unwrap();
    let t = b.types().unwrap();
    assert!(!t.iter().any(|x| x == types::PRIVATE[0]), "{t:?}");
    assert_eq!(b.data(types::TEXT).unwrap(), b"new");
}

#[test]
fn write_refuses_text_that_is_not_utf8() {
    let mut b = Board::new();
    let r = b.write(&[(Format::TextPlain, vec![0xC3, 0x28])]);
    assert!(matches!(r, Err(ClipboardError::Other(_))), "{r:?}");
}

#[test]
fn each_private_marker_skips_content() {
    for marker in types::PRIVATE {
        let mut b = Board::new();
        b.set_raw(&[(types::TEXT, b"hunter2"), (marker, b"")]);
        let s = b.read().unwrap();
        assert!(s.private, "{marker}");
        assert!(s.parts.is_empty(), "{marker}: content must not be read");
    }
}

#[test]
fn file_reference_skips_the_clip() {
    let mut b = Board::new();
    b.set_raw(&[
        ("public.file-url", b"file:///Users/test/report.pdf"),
        (types::TEXT, b"report.pdf"),
    ]);
    let s = b.read().unwrap();
    assert!(s.has_files);
    assert!(s.parts.is_empty());
}

#[test]
fn remote_clipboard_is_flagged() {
    let mut b = Board::new();
    b.set_raw(&[
        (types::REMOTE_CLIPBOARD, b""),
        (types::TEXT, b"from iphone"),
    ]);
    let s = b.read().unwrap();
    assert!(s.remote && !s.private && !s.has_marker);
    assert_eq!(part(&s, Format::TextPlain), Some(&b"from iphone"[..]));

    b.set_read_remote_content(false);
    let s = b.read().unwrap();
    assert!(s.remote);
    assert!(s.parts.is_empty());
}

#[test]
fn png_from_another_app_is_passed_as_is() {
    let mut b = Board::new();
    let png = png_rgba(17, 9, 1);
    b.set_raw(&[(types::PNG, &png)]);
    let s = b.read().unwrap();
    assert_eq!(part(&s, Format::ImagePng), Some(png.as_slice()));
}

#[test]
fn tiff_only_becomes_png_with_the_same_pixels() {
    let mut b = Board::new();
    let png = png_rgba(23, 11, 2);
    let tiff = png_to_tiff(&png).expect("tiff");
    assert!(tiff.starts_with(b"MM") || tiff.starts_with(b"II"));
    b.set_raw(&[(types::TIFF, &tiff)]);
    let s = b.read().unwrap();
    let out = part(&s, Format::ImagePng).expect("image part");
    assert!(out.starts_with(b"\x89PNG"));
    assert_eq!(pixels(out), pixels(&png));
}

#[test]
fn broken_tiff_gives_no_image() {
    let mut b = Board::new();
    b.set_raw(&[(types::TIFF, b"II*\0garbage"), (types::TEXT, b"t")]);
    let s = b.read().unwrap();
    assert_eq!(part(&s, Format::ImagePng), None);
    assert_eq!(part(&s, Format::TextPlain), Some(&b"t"[..]));
}

#[test]
fn image_write_puts_png_and_tiff() {
    let mut b = Board::new();
    let png = png_rgba(8, 8, 3);
    b.write(&[(Format::ImagePng, png.clone())]).unwrap();
    assert_eq!(b.data(types::PNG).unwrap(), png);
    let tiff = b.data(types::TIFF).expect("tiff copy");
    assert_eq!(pixels(&tiff_to_png(&tiff).unwrap()), pixels(&png));
}

#[test]
fn poller_sees_every_change() {
    let mut b = Board::new();
    let mut p = Poller::starting_at(b.change_count(), 0);
    assert_eq!(p.on_poll(250, b.change_count()).changed, None);
    let seq = b.set_raw(&[(types::TEXT, b"a")]);
    assert_eq!(p.on_poll(500, b.change_count()).changed, Some(seq));
    let seq = b.write(&[(Format::TextPlain, b"b".to_vec())]).unwrap();
    assert_eq!(p.on_poll(750, b.change_count()).changed, Some(seq));
}

#[test]
fn engine_sends_a_copy_and_ignores_its_own_write() {
    use uc_core::{Config, DeviceId, DeviceKind, DeviceRecord, Engine, Event, Role, SkipReason};
    let mut b = Board::new();
    b.set_raw(&[(types::TEXT, b"copied on the mac")]);
    let rec = DeviceRecord {
        id: DeviceId([7; 32]),
        name: "test mac".into(),
        kind: DeviceKind::Mac,
    };
    let mut e = Engine::new(Config::new(rec, Role::Full));
    let ev = e.on_local_change(&mut *b, 1_000);
    assert!(ev.iter().any(|x| matches!(x, Event::Sent { .. })), "{ev:?}");
    b.write(&[(Format::TextPlain, b"from a peer".to_vec())])
        .unwrap();
    let ev = e.on_local_change(&mut *b, 2_000);
    assert!(
        ev.iter()
            .any(|x| matches!(x, Event::Skipped(SkipReason::Marker))),
        "{ev:?}"
    );
}

#[test]
fn access_behavior_of_a_named_pasteboard_never_asks() {
    let b = Board::new();
    // Before macOS 15.4 the call does not exist.
    if let Some(a) = b.access_behavior() {
        assert!(
            !matches!(a, AccessBehavior::Ask | AccessBehavior::AlwaysDeny),
            "{a:?}"
        );
    }
}
