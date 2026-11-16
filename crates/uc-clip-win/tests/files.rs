//! File copies on the real clipboard (file sync, spec 6 and 7): `CF_HDROP` read and write, the
//! marker, the drop effect and the image companion. Each test runs inside a private window
//! station (winhost), so the user's clipboard is never touched. The paths need not exist: the
//! clipboard only carries names.
#![cfg(windows)]
#![allow(unsafe_code)]

use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use uc_clip::{Clipboard, ClipboardError, Format};
use uc_clip_win::dib::{self, Rgba};
use uc_clip_win::files;
use uc_clip_win::formats::{self, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use uc_clip_win::{RawFormat, WinClipboard, cloud_only};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_ATTRIBUTE_NORMAL, FILE_ATTRIBUTE_OFFLINE, SetFileAttributesW,
};

macro_rules! real_test {
    ($name:ident, $body:block) => {
        #[test]
        fn $name() {
            uc_winhost::isolated_test(stringify!($name), || $body);
        }
    };
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().collect()
}

fn name(n: &str) -> RawFormat {
    RawFormat::Name(n.into())
}

/// Put `paths` on the clipboard the way Explorer would (no marker) and read them back.
fn round_trip_foreign(cb: &mut WinClipboard, paths: &[PathBuf]) -> uc_clip::Snapshot {
    let units: Vec<Vec<u16>> = paths.iter().map(|p| wide(p)).collect();
    cb.write_raw(&[
        (RawFormat::Id(CF_HDROP), files::build_hdrop(&units).unwrap()),
        (
            name(formats::PREFERRED_DROP_EFFECT),
            5u32.to_le_bytes().to_vec(),
        ),
    ])
    .unwrap();
    cb.read().unwrap()
}

fn png_sample() -> Vec<u8> {
    let pixels = (0..12u32)
        .flat_map(|i| [(i * 20) as u8, 0, 255 - (i * 20) as u8, 255])
        .collect();
    dib::rgba_to_png(&Rgba {
        width: 4,
        height: 3,
        pixels,
    })
    .unwrap()
}

real_test!(paths_with_spaces_are_exact, {
    let mut cb = WinClipboard::new().unwrap();
    let paths = vec![
        PathBuf::from(r"C:\Users\Max\My Documents\a  b .txt"),
        PathBuf::from(r"\\server\share name\folder with spaces"),
        PathBuf::from(r"D:\x"),
    ];
    let snap = round_trip_foreign(&mut cb, &paths);
    assert!(snap.has_files && !snap.files_virtual && !snap.has_marker && !snap.private);
    assert_eq!(snap.files, paths);
    assert!(snap.parts.is_empty());
});

real_test!(non_bmp_and_lone_surrogate_names_are_exact, {
    let mut cb = WinClipboard::new().unwrap();
    // Emoji and a musical symbol (surrogate pairs), Norwegian letters, and a name with a
    // lone surrogate, which only OsString::from_wide keeps.
    let mut odd = wide(Path::new(r"C:\odd-"));
    odd.push(0xD800);
    odd.extend(wide(Path::new(".txt")));
    let paths = vec![
        PathBuf::from(r"C:\😀 bilder\𝄞 æøå.png"),
        PathBuf::from(std::ffi::OsString::from_wide(&odd)),
    ];
    let snap = round_trip_foreign(&mut cb, &paths);
    assert_eq!(snap.files, paths);
    assert_eq!(wide(&snap.files[1]), odd);
});

real_test!(many_paths_are_read_in_order, {
    let mut cb = WinClipboard::new().unwrap();
    let paths: Vec<PathBuf> = (0..5_000)
        .map(|i| PathBuf::from(format!(r"C:\many\file {i:05}.dat")))
        .collect();
    let snap = round_trip_foreign(&mut cb, &paths);
    assert_eq!(snap.files, paths);
});

real_test!(garbage_dropfiles_never_panics, {
    let mut cb = WinClipboard::new().unwrap();
    let good = files::build_hdrop(&[wide(Path::new(r"C:\a")), wide(Path::new(r"C:\b"))]).unwrap();
    let mut ansi = good.clone();
    ansi[16] = 0;
    let mut past_end = good.clone();
    past_end[..4].copy_from_slice(&u32::MAX.to_le_bytes());
    // A list cut inside a path is tested in `src/files.rs` only: Windows rounds the block up,
    // and the bytes after the cut are not ours to choose here.
    let mut relative =
        files::build_hdrop(&[wide(Path::new("relative.txt")), wide(Path::new(r"C:\ok"))]).unwrap();
    relative.extend_from_slice(&[0xFF; 7]);
    let noise: Vec<u8> = (0..997u32).map(|i| (i * 31 + 7) as u8).collect();
    let cases: Vec<(Vec<u8>, Vec<PathBuf>)> = vec![
        (vec![], vec![]),
        (vec![1, 2, 3], vec![]),
        (good[..19].to_vec(), vec![]),
        (ansi, vec![]),
        (past_end, vec![]),
        (relative, vec![PathBuf::from(r"C:\ok")]),
        (noise, vec![]),
    ];
    for (i, (bytes, want)) in cases.into_iter().enumerate() {
        cb.write_raw(&[
            (RawFormat::Id(CF_HDROP), bytes),
            (RawFormat::Id(CF_UNICODETEXT), formats::utf8_to_utf16z(b"x")),
        ])
        .unwrap();
        let snap = cb.read().unwrap();
        assert!(snap.has_files, "case {i}");
        assert!(!snap.files_virtual, "case {i}");
        assert!(snap.parts.is_empty(), "case {i}");
        assert_eq!(snap.files, want, "case {i}");
    }
});

real_test!(write_files_sets_every_format_in_one_write, {
    let mut cb = WinClipboard::new().unwrap();
    let paths = vec![
        PathBuf::from(r"C:\Users\Max\AppData\Local\Temp\Universal Clipboard\ab12cd34\report.pdf"),
        PathBuf::from(r"C:\Users\Max\AppData\Local\Temp\Universal Clipboard\ab12cd34\Folder"),
    ];
    let seq = cb.write_files(&paths, None).unwrap();
    let snap = cb.read().unwrap();
    assert_eq!(snap.seq, seq, "one open and close: one sequence number");
    assert!(snap.has_marker, "the watcher must see our marker");
    assert!(
        !snap.private,
        "our CanUploadToCloudClipboard = 0 is not private"
    );
    assert!(snap.has_files && !snap.files_virtual);
    assert_eq!(snap.files, paths);
    assert!(snap.parts.is_empty());
    let cloud = cb
        .read_raw(name(formats::CAN_UPLOAD_TO_CLOUD))
        .unwrap()
        .unwrap();
    assert_eq!(&cloud[..4], &[0, 0, 0, 0]);
    let effect = cb
        .read_raw(name(formats::PREFERRED_DROP_EFFECT))
        .unwrap()
        .unwrap();
    assert_eq!(&effect[..4], &files::DROP_EFFECT_COPY.to_le_bytes());
    assert!(cb.read_raw(name(formats::ORIGIN)).unwrap().is_some());
    let hdrop = cb.read_raw(RawFormat::Id(CF_HDROP)).unwrap().unwrap();
    let units: Vec<Vec<u16>> = paths.iter().map(|p| wide(p)).collect();
    assert_eq!(files::parse_hdrop(&hdrop), Some(units));
    // No image was asked for.
    assert!(cb.read_raw(name(formats::PNG)).unwrap().is_none());
    assert!(
        cb.read_raw(name(formats::CAN_INCLUDE_IN_HISTORY))
            .unwrap()
            .is_none()
    );
});

real_test!(write_files_with_image_adds_png_and_dibv5, {
    let mut cb = WinClipboard::new().unwrap();
    let png = png_sample();
    let paths = vec![PathBuf::from(r"C:\t\photo.png")];
    cb.write_files(&paths, Some(&png)).unwrap();
    let snap = cb.read().unwrap();
    assert!(snap.has_marker);
    assert_eq!(snap.files, paths);
    // A file copy reads no parts, even our own image next to it.
    assert!(snap.parts.iter().all(|(f, _)| *f != Format::ImagePng));
    let stored = cb.read_raw(name(formats::PNG)).unwrap().unwrap();
    assert_eq!(dib::trim_png(&stored), png.as_slice());
    let v5 = cb.read_raw(RawFormat::Id(CF_DIBV5)).unwrap().unwrap();
    assert_eq!(
        dib::dib_to_rgba(&v5).unwrap(),
        dib::png_to_rgba(&png).unwrap()
    );
});

real_test!(long_path_of_300_characters_round_trips, {
    let mut cb = WinClipboard::new().unwrap();
    let mut s = String::from(r"C:\Users\Max\AppData\Local\Temp\Universal Clipboard\ab12cd34");
    while s.len() < 291 {
        s.push_str(r"\a long folder name");
    }
    s.truncate(291);
    if s.ends_with('\\') {
        s.pop();
        s.push('x');
    }
    s.push_str(r"\file.txt");
    assert_eq!(s.chars().count(), 300);
    let long = PathBuf::from(&s);
    // Written plain, and also when the caller hands a verbatim (\\?\) path.
    cb.write_files(std::slice::from_ref(&long), None).unwrap();
    assert_eq!(cb.read().unwrap().files, vec![long.clone()]);
    let verbatim = PathBuf::from(format!(r"\\?\{s}"));
    cb.write_files(&[verbatim], None).unwrap();
    assert_eq!(cb.read().unwrap().files, vec![long.clone()]);
    let unc = PathBuf::from(r"\\?\UNC\server\share\x.txt");
    cb.write_files(&[unc], None).unwrap();
    assert_eq!(
        cb.read().unwrap().files,
        vec![PathBuf::from(r"\\server\share\x.txt")]
    );
    // And a foreign 300-character entry reads back exact.
    assert_eq!(
        round_trip_foreign(&mut cb, std::slice::from_ref(&long)).files,
        vec![long]
    );
});

real_test!(
    write_files_refuses_bad_paths_without_touching_the_clipboard,
    {
        let mut cb = WinClipboard::new().unwrap();
        let before = cb
            .write_raw(&[(
                RawFormat::Id(CF_UNICODETEXT),
                formats::utf8_to_utf16z(b"keep"),
            )])
            .unwrap();
        let bad = ClipboardError::Other("bad-file-paths".into());
        assert_eq!(cb.write_files(&[], None), Err(bad.clone()));
        assert_eq!(
            cb.write_files(&[PathBuf::from("relative.txt")], None),
            Err(bad.clone())
        );
        let nul = PathBuf::from(std::ffi::OsString::from_wide(&[
            u16::from(b'C'),
            u16::from(b':'),
            u16::from(b'\\'),
            0,
            u16::from(b'x'),
        ]));
        assert_eq!(cb.write_files(&[nul], None), Err(bad));
        assert_eq!(WinClipboard::sequence_number(), before);
        let snap = cb.read().unwrap();
        assert!(!snap.has_files);
    }
);

/// Not a clipboard test: the cloud placeholder check reads file attributes only. OneDrive is
/// not needed: `FILE_ATTRIBUTE_OFFLINE` can be set on a plain temp file and counts the same.
#[test]
fn cloud_only_reads_the_placeholder_attributes() {
    let dir = std::env::temp_dir().join(format!("uc-clip-win-cloud-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let f = dir.join("online-only.txt");
    std::fs::write(&f, b"x").unwrap();
    assert!(!cloud_only(&f));
    assert!(!cloud_only(&dir));
    assert!(!cloud_only(&dir.join("missing")));
    let w: Vec<u16> = wide(&f).into_iter().chain([0]).collect();
    // SAFETY: `w` is a NUL-terminated path that lives for the calls.
    unsafe {
        assert_ne!(SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_OFFLINE), 0);
    }
    assert!(cloud_only(&f));
    // SAFETY: as above.
    unsafe {
        assert_ne!(SetFileAttributesW(w.as_ptr(), FILE_ATTRIBUTE_NORMAL), 0);
    }
    assert!(!cloud_only(&f));
    std::fs::remove_dir_all(&dir).unwrap();
}
