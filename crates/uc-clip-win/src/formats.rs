//! Clipboard format names and the rules that need no OS calls (tested on every OS).

/// Our origin marker (echo layer 1, plan 3.8). Every write of ours carries it.
pub const ORIGIN: &str = "UniversalClipboard.Origin";
/// Private marker: present means "do not record this copy".
pub const EXCLUDE_MONITOR: &str = "ExcludeClipboardContentFromMonitorProcessing";
/// Private marker when its DWORD value is 0. We never set it, so our writes stay in Win+V.
pub const CAN_INCLUDE_IN_HISTORY: &str = "CanIncludeInClipboardHistory";
/// Private marker when its DWORD value is 0. Our writes set it to 0 (plan 3.8).
pub const CAN_UPLOAD_TO_CLOUD: &str = "CanUploadToCloudClipboard";
/// The registered PNG format that browsers, Office and the Snipping Tool use.
pub const PNG: &str = "PNG";
/// `CF_HTML`.
pub const HTML: &str = "HTML Format";
/// Virtual files (for example an Outlook attachment). Treated like `CF_HDROP`.
pub const FILE_GROUP_DESCRIPTOR: &str = "FileGroupDescriptorW";

/// Standard format IDs used here.
pub const CF_BITMAP: u32 = 2;
/// `CF_DIB`.
pub const CF_DIB: u32 = 8;
/// `CF_UNICODETEXT`.
pub const CF_UNICODETEXT: u32 = 13;
/// `CF_HDROP` (a file list).
pub const CF_HDROP: u32 = 15;
/// `CF_DIBV5`.
pub const CF_DIBV5: u32 = 17;

/// What the value of a DWORD marker format was.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dword {
    /// The format is not on the clipboard.
    Absent,
    /// The value.
    Value(u32),
    /// The format is listed but could not be read, or is shorter than 4 bytes.
    Unreadable,
}

impl Dword {
    /// Read a DWORD marker from its bytes (`None`: the read failed).
    pub fn from_bytes(bytes: Option<&[u8]>) -> Dword {
        match bytes.and_then(|b| b.get(..4)) {
            Some(b) => Dword::Value(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
            None => Dword::Unreadable,
        }
    }

    fn says_private(self) -> bool {
        // A marker that cannot be read counts as private (plan 3.10).
        matches!(self, Dword::Value(0) | Dword::Unreadable)
    }
}

/// The private-copy rule (plan 3.10) for one clipboard state.
///
/// `CanUploadToCloudClipboard = 0` is ignored when our origin marker is present, because our
/// own writes set it. The other markers count in every case.
pub fn is_private(exclude_present: bool, history: Dword, cloud: Dword, has_origin: bool) -> bool {
    exclude_present || history.says_private() || (!has_origin && cloud.says_private())
}

/// `CF_UNICODETEXT` memory to UTF-8. Stops at the first NUL (the memory block is often
/// larger than the text). Lone surrogates become U+FFFD.
pub fn utf16_to_utf8(bytes: &[u8]) -> Vec<u8> {
    let units: Vec<u16> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units).into_bytes()
}

/// UTF-8 text to `CF_UNICODETEXT` memory, with the closing NUL. Invalid UTF-8 becomes U+FFFD.
pub fn utf8_to_utf16z(bytes: &[u8]) -> Vec<u8> {
    let text = String::from_utf8_lossy(bytes);
    let mut out = Vec::with_capacity(text.len() * 2 + 2);
    for u in text.encode_utf16().chain(std::iter::once(0)) {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn private_rules() {
        use Dword::*;
        assert!(!is_private(false, Absent, Absent, false));
        assert!(is_private(true, Absent, Absent, false));
        assert!(is_private(false, Value(0), Absent, false));
        assert!(!is_private(false, Value(1), Value(1), false));
        assert!(is_private(false, Absent, Value(0), false));
        assert!(is_private(false, Unreadable, Absent, false));
        assert!(is_private(false, Absent, Unreadable, false));
        // Our own write: cloud = 0 plus origin is not private.
        assert!(!is_private(false, Absent, Value(0), true));
        // But a history opt-out still is.
        assert!(is_private(false, Value(0), Value(0), true));
    }

    #[test]
    fn dword_from_bytes() {
        assert_eq!(Dword::from_bytes(Some(&[0, 0, 0, 0, 9])), Dword::Value(0));
        assert_eq!(Dword::from_bytes(Some(&[1, 0, 0, 0])), Dword::Value(1));
        assert_eq!(Dword::from_bytes(Some(&[1, 0])), Dword::Unreadable);
        assert_eq!(Dword::from_bytes(None), Dword::Unreadable);
    }

    #[test]
    fn utf16_stops_at_nul_and_keeps_crlf() {
        let mut b = utf8_to_utf16z("a\r\nø😀".as_bytes());
        b.extend_from_slice(&[0, 0, b'x', 0, 7]);
        assert_eq!(utf16_to_utf8(&b), "a\r\nø😀".as_bytes());
    }

    #[test]
    fn lone_surrogate_is_replaced() {
        let b = [0x00, 0xD8, b'a', 0];
        assert_eq!(utf16_to_utf8(&b), "\u{FFFD}a".as_bytes());
    }

    proptest! {
        #[test]
        fn text_round_trip(s in "[^\u{0}]{0,200}") {
            prop_assert_eq!(utf16_to_utf8(&utf8_to_utf16z(s.as_bytes())), s.as_bytes());
        }

        #[test]
        fn random_bytes_never_panic(b in proptest::collection::vec(any::<u8>(), 0..300)) {
            let _ = utf16_to_utf8(&b);
            let _ = utf8_to_utf16z(&b);
        }
    }
}
