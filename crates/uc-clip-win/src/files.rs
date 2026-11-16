//! File copies on Windows: the `CF_HDROP` (`DROPFILES`) layout and the cloud placeholder rule.
//! No OS calls here, so these are tested on every OS. Paths are UTF-16 code units, the way
//! Windows stores them, so a name that is not valid Unicode (a lone surrogate) survives exactly.

/// Size of the `DROPFILES` header: `pFiles` (u32), `pt` (two i32), `fNC` (i32), `fWide` (i32).
pub const DROPFILES_SIZE: usize = 20;

/// `DROPEFFECT_COPY`: the value of `Preferred DropEffect` for a copy (not a cut).
pub const DROP_EFFECT_COPY: u32 = 1;

/// `FILE_ATTRIBUTE_OFFLINE`: the data is not on this disk (old HSM tools, some sync clients).
pub const FILE_ATTRIBUTE_OFFLINE: u32 = 0x1000;
/// `FILE_ATTRIBUTE_RECALL_ON_OPEN`: opening the item fetches it (cloud folders not yet listed).
pub const FILE_ATTRIBUTE_RECALL_ON_OPEN: u32 = 0x4_0000;
/// `FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS`: reading the data downloads it (OneDrive
/// "online-only" files and other cloud placeholders).
pub const FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

/// The file sync rule for cloud placeholders (spec 2.4): an item with any of these attributes is
/// not on disk, and reading it would download it. One such item skips the whole copy.
pub fn is_cloud_placeholder(attributes: u32) -> bool {
    attributes
        & (FILE_ATTRIBUTE_OFFLINE
            | FILE_ATTRIBUTE_RECALL_ON_OPEN
            | FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS)
        != 0
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    let c = b.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes([c[0], c[1], c[2], c[3]]))
}

/// Read the paths out of `CF_HDROP` memory (a `DROPFILES` header and a double-NUL-ended list).
///
/// Returns `None` when the header is cut short, `pFiles` points outside the block, or the list
/// is ANSI (`fWide = 0`; no current program writes that, and its code page is a guess). A path
/// that runs to the end of the block without its NUL may be cut, so it is left out. Never
/// panics on any input.
pub fn parse_hdrop(bytes: &[u8]) -> Option<Vec<Vec<u16>>> {
    let start = usize::try_from(u32_at(bytes, 0)?).ok()?;
    let wide = u32_at(bytes, 16)? != 0;
    if !wide || start < DROPFILES_SIZE || start > bytes.len() {
        return None;
    }
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for c in bytes[start..].as_chunks::<2>().0 {
        match u16::from_le_bytes(*c) {
            0 if cur.is_empty() => return Some(out),
            0 => out.push(std::mem::take(&mut cur)),
            u => cur.push(u),
        }
    }
    // No closing empty string: keep the complete paths, drop the cut one.
    Some(out)
}

/// Build `CF_HDROP` memory for `paths`: a wide `DROPFILES` header with `pFiles = 20`, each path
/// with its NUL, and the closing NUL. Returns `None` for an empty list, an empty path or a path
/// with a NUL in it, because the list could not be read back as written.
pub fn build_hdrop(paths: &[Vec<u16>]) -> Option<Vec<u8>> {
    if paths.is_empty() || paths.iter().any(|p| p.is_empty() || p.contains(&0)) {
        return None;
    }
    let units: usize = paths.iter().map(|p| p.len() + 1).sum::<usize>() + 1;
    let mut out = Vec::with_capacity(DROPFILES_SIZE + units * 2);
    out.extend_from_slice(&(DROPFILES_SIZE as u32).to_le_bytes()); // pFiles
    out.extend_from_slice(&[0; 12]); // pt.x, pt.y, fNC
    out.extend_from_slice(&1u32.to_le_bytes()); // fWide
    for u in paths.iter().flat_map(|p| p.iter().copied().chain([0])) {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    Some(out)
}

/// Remove the `\\?\` prefix where a plain path says the same: `\\?\C:\x` becomes `C:\x` and
/// `\\?\UNC\server\share` becomes `\\server\share`. The spec says no prefix, because many apps
/// do not expect it in a pasted file list. Other forms (`\\?\Volume{...}\`) have no plain spelling and
/// stay as they are.
pub fn strip_verbatim(path: &[u16]) -> Vec<u16> {
    let w = |s: &str| s.encode_utf16().collect::<Vec<u16>>();
    let verbatim = w(r"\\?\");
    let Some(rest) = path.strip_prefix(verbatim.as_slice()) else {
        return path.to_vec();
    };
    let unc = w(r"UNC\");
    if rest.len() >= unc.len() && eq_ascii_ci(&rest[..unc.len()], &unc) {
        let mut out = w(r"\\");
        out.extend_from_slice(&rest[unc.len()..]);
        return out;
    }
    let drive = matches!(rest, [d, c, s, ..]
        if u8::try_from(*d).is_ok_and(|d| d.is_ascii_alphabetic())
            && *c == u16::from(b':')
            && *s == u16::from(b'\\'));
    if drive { rest.to_vec() } else { path.to_vec() }
}

/// ASCII case-insensitive compare of UTF-16 code units.
fn eq_ascii_ci(a: &[u16], b: &[u16]) -> bool {
    let low = |u: u16| match u8::try_from(u) {
        Ok(c) => u16::from(c.to_ascii_lowercase()),
        Err(_) => u,
    };
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| low(*x) == low(*y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn w(s: &str) -> Vec<u16> {
        s.encode_utf16().collect()
    }

    #[test]
    fn explorer_layout_parses() {
        // What Explorer writes: header, then "C:\a b.txt\0D:\😀\0\0".
        let mut b = Vec::new();
        b.extend_from_slice(&20u32.to_le_bytes());
        b.extend_from_slice(&[0; 12]);
        b.extend_from_slice(&1u32.to_le_bytes());
        for u in w("C:\\a b.txt\0D:\\😀\0\0") {
            b.extend_from_slice(&u.to_le_bytes());
        }
        assert_eq!(parse_hdrop(&b), Some(vec![w("C:\\a b.txt"), w("D:\\😀")]));
        assert_eq!(build_hdrop(&[w("C:\\a b.txt"), w("D:\\😀")]), Some(b));
    }

    #[test]
    fn larger_pfiles_and_trailing_bytes_are_fine() {
        // Some programs put extra bytes between the header and the list, or round the block up.
        let mut b = build_hdrop(&[w("C:\\x")]).unwrap();
        b.splice(20..20, [9, 9, 9, 9]);
        b[0] = 24;
        b.extend_from_slice(&[0x41, 0, 0x42, 0]);
        assert_eq!(parse_hdrop(&b), Some(vec![w("C:\\x")]));
    }

    #[test]
    fn bad_headers_are_refused() {
        let good = build_hdrop(&[w("C:\\x")]).unwrap();
        assert_eq!(parse_hdrop(&good[..19]), None, "cut header");
        assert_eq!(parse_hdrop(&[]), None);
        let mut ansi = good.clone();
        ansi[16] = 0;
        assert_eq!(parse_hdrop(&ansi), None, "ANSI list is skipped");
        let mut inside = good.clone();
        inside[0] = 4;
        assert_eq!(parse_hdrop(&inside), None, "pFiles inside the header");
        let mut outside = good.clone();
        outside[..4].copy_from_slice(&(good.len() as u32 + 1).to_le_bytes());
        assert_eq!(parse_hdrop(&outside), None, "pFiles past the end");
        let mut huge = good;
        huge[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse_hdrop(&huge), None);
    }

    #[test]
    fn unterminated_path_is_dropped_and_empty_list_is_empty() {
        let mut b = build_hdrop(&[w("C:\\a"), w("C:\\bcd")]).unwrap();
        b.truncate(b.len() - 6); // cut inside the second path
        assert_eq!(parse_hdrop(&b), Some(vec![w("C:\\a")]));
        let mut empty = build_hdrop(&[w("C:\\a")]).unwrap();
        empty.truncate(20);
        empty.extend_from_slice(&[0, 0]);
        assert_eq!(parse_hdrop(&empty), Some(vec![]));
        // An odd last byte is ignored.
        let mut odd = build_hdrop(&[w("C:\\a")]).unwrap();
        odd.push(7);
        assert_eq!(parse_hdrop(&odd), Some(vec![w("C:\\a")]));
    }

    #[test]
    fn build_refuses_what_cannot_round_trip() {
        assert_eq!(build_hdrop(&[]), None);
        assert_eq!(build_hdrop(&[vec![]]), None);
        assert_eq!(build_hdrop(&[vec![0x43, 0, 0x44]]), None);
    }

    #[test]
    fn verbatim_prefix_is_removed_where_safe() {
        assert_eq!(strip_verbatim(&w(r"\\?\C:\a\b")), w(r"C:\a\b"));
        assert_eq!(
            strip_verbatim(&w(r"\\?\UNC\srv\share\f")),
            w(r"\\srv\share\f")
        );
        assert_eq!(strip_verbatim(&w(r"\\?\unc\srv\s")), w(r"\\srv\s"));
        let vol = w(r"\\?\Volume{1234}\a");
        assert_eq!(strip_verbatim(&vol), vol);
        assert_eq!(strip_verbatim(&w(r"C:\plain")), w(r"C:\plain"));
        assert_eq!(strip_verbatim(&w(r"\\srv\s")), w(r"\\srv\s"));
        assert_eq!(strip_verbatim(&w(r"\\?\")), w(r"\\?\"));
    }

    #[test]
    fn cloud_placeholder_attributes() {
        assert!(!is_cloud_placeholder(0));
        assert!(!is_cloud_placeholder(0x20 | 0x10 | 0x8_0000)); // archive, directory, pinned
        assert!(is_cloud_placeholder(
            FILE_ATTRIBUTE_RECALL_ON_DATA_ACCESS | 0x20
        ));
        assert!(is_cloud_placeholder(FILE_ATTRIBUTE_RECALL_ON_OPEN | 0x10));
        assert!(is_cloud_placeholder(FILE_ATTRIBUTE_OFFLINE));
    }

    fn path_units() -> impl Strategy<Value = Vec<u16>> {
        // Any code unit but NUL, lone surrogates included.
        proptest::collection::vec(1u16..=u16::MAX, 1..300)
    }

    proptest! {
        #[test]
        fn hdrop_round_trip(paths in proptest::collection::vec(path_units(), 1..40)) {
            let b = build_hdrop(&paths).unwrap();
            prop_assert_eq!(parse_hdrop(&b), Some(paths));
        }

        #[test]
        fn random_bytes_never_panic(b in proptest::collection::vec(any::<u8>(), 0..400)) {
            let _ = parse_hdrop(&b);
        }

        #[test]
        fn random_bytes_after_a_wide_header_never_panic(
            start in 0u32..64,
            tail in proptest::collection::vec(any::<u8>(), 0..400),
        ) {
            let mut b = start.to_le_bytes().to_vec();
            b.extend_from_slice(&[0; 12]);
            b.extend_from_slice(&1u32.to_le_bytes());
            b.extend_from_slice(&tail);
            if let Some(paths) = parse_hdrop(&b) {
                prop_assert!(paths.iter().all(|p| !p.is_empty() && !p.contains(&0)));
            }
        }

        #[test]
        fn strip_verbatim_never_panics(p in proptest::collection::vec(any::<u16>(), 0..20)) {
            let _ = strip_verbatim(&p);
        }
    }
}
