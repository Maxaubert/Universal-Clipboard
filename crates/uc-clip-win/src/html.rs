//! Read side of the Windows `HTML Format` (`CF_HTML`): take the fragment out of the header.
//!
//! The data comes from other programs, so every offset is checked. A bad header gives `None`
//! (the clip is then sent as plain text only). Writing `CF_HTML` is milestone M11
//! (rich text, plan 3.11), together with the full parser review there.
//!
//! Format (byte offsets count from the start of the data):
//! ```text
//! Version:0.9
//! StartHTML:0000000105
//! EndHTML:0000000199
//! StartFragment:0000000141
//! EndFragment:0000000163
//! <html><body><!--StartFragment-->...<!--EndFragment--></body></html>
//! ```

/// The header is short. Stop looking for header lines after this many bytes.
const HEADER_LIMIT: usize = 4096;

fn header_value(data: &[u8], key: &[u8]) -> Option<usize> {
    let head = &data[..data.len().min(HEADER_LIMIT)];
    for line in head.split(|&b| b == b'\n' || b == b'\r') {
        if line.first() == Some(&b'<') {
            break;
        }
        let Some(rest) = line.strip_prefix(key) else {
            continue;
        };
        let Some(value) = rest.strip_prefix(b":") else {
            continue;
        };
        let text = std::str::from_utf8(value).ok()?.trim();
        // "-1" means "not given".
        let n: i64 = text.parse().ok()?;
        return usize::try_from(n).ok();
    }
    None
}

fn slice(data: &[u8], start: Option<usize>, end: Option<usize>) -> Option<&[u8]> {
    let (start, end) = (start?, end?);
    if start > end || end > data.len() {
        return None;
    }
    Some(&data[start..end])
}

/// Take the HTML fragment out of `CF_HTML` data. Returns UTF-8 bytes, or `None` when the
/// header is missing or wrong, the offsets are out of range, or the result is not UTF-8.
///
/// Uses `StartFragment`/`EndFragment`, and falls back to `StartHTML`/`EndHTML`.
pub fn fragment(data: &[u8]) -> Option<&[u8]> {
    // Clipboard memory can carry a trailing NUL and padding.
    let data = match data.iter().position(|&b| b == 0) {
        Some(n) => &data[..n],
        None => data,
    };
    let out = slice(
        data,
        header_value(data, b"StartFragment"),
        header_value(data, b"EndFragment"),
    )
    .or_else(|| {
        slice(
            data,
            header_value(data, b"StartHTML"),
            header_value(data, b"EndHTML"),
        )
    })?;
    std::str::from_utf8(out).ok()?;
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// Build CF_HTML the way browsers do (10-digit offsets).
    pub(crate) fn build(fragment: &str) -> Vec<u8> {
        let pre = "<html><body>\r\n<!--StartFragment-->";
        let post = "<!--EndFragment-->\r\n</body></html>";
        let header_len = "Version:0.9\r\nStartHTML:0000000000\r\nEndHTML:0000000000\r\nStartFragment:0000000000\r\nEndFragment:0000000000\r\n".len();
        let start_html = header_len;
        let start_frag = start_html + pre.len();
        let end_frag = start_frag + fragment.len();
        let end_html = end_frag + post.len();
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\nStartFragment:{start_frag:010}\r\nEndFragment:{end_frag:010}\r\n{pre}{fragment}{post}"
        )
        .into_bytes()
    }

    #[test]
    fn browser_style_fragment() {
        let data = build("<b>Hei på deg</b>");
        assert_eq!(fragment(&data), Some("<b>Hei på deg</b>".as_bytes()));
    }

    #[test]
    fn trailing_nul_and_padding_ignored() {
        let mut data = build("<i>x</i>");
        data.extend_from_slice(&[0, 0, 0xFF]);
        assert_eq!(fragment(&data), Some(&b"<i>x</i>"[..]));
    }

    #[test]
    fn falls_back_to_html_range() {
        let body = "<p>a</p>";
        let head = "Version:1.0\r\nStartHTML:0000000055\r\nEndHTML:0000000063\r\n";
        let data = format!("{head}{body}");
        assert_eq!(head.len(), 55);
        assert_eq!(fragment(data.as_bytes()), Some(body.as_bytes()));
    }

    #[test]
    fn bad_headers_give_none() {
        assert_eq!(fragment(b""), None);
        assert_eq!(fragment(b"<html>no header</html>"), None);
        assert_eq!(
            fragment(b"Version:0.9\r\nStartFragment:10\r\nEndFragment:9999\r\n"),
            None
        );
        assert_eq!(
            fragment(b"Version:0.9\r\nStartFragment:30\r\nEndFragment:20\r\n"),
            None
        );
        assert_eq!(
            fragment(b"Version:0.9\r\nStartFragment:-1\r\nEndFragment:-1\r\n"),
            None
        );
        // Offsets that cut a UTF-8 character in half.
        let mut data = build("ø");
        let s = String::from_utf8(data.clone()).unwrap();
        let ef = s.find("EndFragment:").unwrap() + "EndFragment:".len();
        let old: usize = s[ef..ef + 10].parse().unwrap();
        data[ef..ef + 10].copy_from_slice(format!("{:010}", old - 1).as_bytes());
        assert_eq!(fragment(&data), None);
    }

    proptest! {
        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..500)) {
            let _ = fragment(&bytes);
        }

        #[test]
        fn random_offsets_never_panic(a in any::<i64>(), b in any::<i64>(), body in ".{0,40}") {
            let data = format!("Version:0.9\r\nStartFragment:{a}\r\nEndFragment:{b}\r\n{body}");
            if let Some(f) = fragment(data.as_bytes()) {
                prop_assert!(std::str::from_utf8(f).is_ok());
            }
        }

        #[test]
        fn build_then_parse(frag in "[^\u{0}]{1,60}") {
            let data = build(&frag);
            prop_assert_eq!(fragment(&data), Some(frag.as_bytes()));
        }
    }
}
