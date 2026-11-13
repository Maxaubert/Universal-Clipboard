//! The line-ending rule (plan 3.8).
//!
//! - Text on the wire uses LF.
//! - Reading from a clipboard: CRLF becomes LF, a lone CR becomes LF too (so the wire never
//!   holds a CR that a later CRLF step could join), and trailing NUL characters are removed.
//! - Writing to a CRLF clipboard (Windows): LF becomes CRLF.
//! - The content hash normalizes further: every CRLF and every lone CR become LF.

/// Turn text read from a clipboard into wire form. Returns `None` if it is not UTF-8.
pub fn to_wire(raw: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(raw).ok()?;
    Some(normalize_for_hash(s))
}

/// Turn wire text into the clipboard's line ending.
pub fn to_native(wire: &str, ending: uc_clip::LineEnding) -> String {
    match ending {
        uc_clip::LineEnding::Lf => wire.to_owned(),
        uc_clip::LineEnding::CrLf => wire.replace("\r\n", "\n").replace('\n', "\r\n"),
    }
}

/// The normalized form of text that goes into the content hash.
pub fn normalize_for_hash(text: &str) -> String {
    text.trim_end_matches('\0')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use uc_clip::LineEnding;

    #[test]
    fn crlf_to_lf_and_nul_trim() {
        assert_eq!(to_wire(b"a\r\nb\r\n\0\0").as_deref(), Some("a\nb\n"));
        assert_eq!(to_wire(b"a\rb").as_deref(), Some("a\nb"));
        assert_eq!(to_wire(b"\r\r\n").as_deref(), Some("\n\n"));
        assert_eq!(to_wire(&[0xff, 0xfe]), None);
    }

    #[test]
    fn lf_to_crlf_on_windows() {
        assert_eq!(to_native("a\nb", LineEnding::CrLf), "a\r\nb");
        assert_eq!(to_native("a\r\nb", LineEnding::CrLf), "a\r\nb");
        assert_eq!(to_native("a\nb", LineEnding::Lf), "a\nb");
    }

    #[test]
    fn hash_form_folds_all_endings() {
        assert_eq!(normalize_for_hash("a\r\nb\rc\n\0"), "a\nb\nc\n");
    }

    proptest! {
        #[test]
        fn windows_round_trip_is_stable(s in "[a-z\r\n\0 ]{0,40}") {
            let wire = to_wire(s.as_bytes()).unwrap();
            prop_assert!(!wire.contains('\r'));
            let native = to_native(&wire, LineEnding::CrLf);
            let back = to_wire(native.as_bytes()).unwrap();
            prop_assert_eq!(back, wire);
        }

        #[test]
        fn hash_form_is_the_same_on_both_os(s in "[a-z\r\n ]{0,40}") {
            let w = to_wire(s.as_bytes()).unwrap();
            prop_assert_eq!(
                normalize_for_hash(&to_native(&w, LineEnding::CrLf)),
                normalize_for_hash(&to_native(&w, LineEnding::Lf))
            );
        }
    }
}
