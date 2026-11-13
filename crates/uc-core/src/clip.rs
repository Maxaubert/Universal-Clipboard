//! The clip model: one clip has 1 to 3 parts (plan 3.8).

use serde::{Deserialize, Serialize};
use uc_clip::{Format, RawPart};

use crate::caps;
use crate::ids::{ClipId, ContentHash};
use crate::image::{self, ImageError};
use crate::text;

/// Size and hash of one part, as listed in a clip header.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartMeta {
    /// The format.
    pub format: Format,
    /// The size in bytes.
    pub size: u32,
    /// BLAKE3 of the bytes.
    pub hash: [u8; 32],
}

/// What a clip holds, for "Last activity". Never the content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipKind {
    /// Text (maybe with HTML).
    Text,
    /// An image.
    Image,
    /// Text and an image.
    TextAndImage,
}

/// Why a set of parts is not a valid clip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipError {
    /// No parts, or more than 3.
    PartCount,
    /// The same format appears twice.
    DuplicateFormat,
    /// HTML without a plain text part.
    HtmlWithoutText,
    /// A part is over its cap.
    TooLarge(Format),
    /// Text or HTML is not UTF-8.
    NotUtf8,
    /// The image failed a safety check or did not decode.
    Image(ImageError),
    /// A part does not match its header (size, hash, order), or the wire ID is wrong.
    Mismatch,
}

impl std::fmt::Display for ClipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClipError::PartCount => f.write_str("a clip needs 1 to 3 parts"),
            ClipError::DuplicateFormat => f.write_str("a format appears twice"),
            ClipError::HtmlWithoutText => f.write_str("HTML needs a plain text part"),
            ClipError::TooLarge(fmt) => write!(f, "{} part is over its cap", fmt.mime()),
            ClipError::NotUtf8 => f.write_str("text is not UTF-8"),
            ClipError::Image(e) => write!(f, "image: {e}"),
            ClipError::Mismatch => f.write_str("parts do not match the header"),
        }
    }
}

impl std::error::Error for ClipError {}

fn format_tag(f: Format) -> u8 {
    match f {
        Format::TextPlain => 1,
        Format::TextHtml => 2,
        Format::ImagePng => 3,
    }
}

/// Compute the wire ID from the part list. It covers format, size and hash of every part, in
/// canonical order, so it is a hash over the parts exactly as sent.
pub fn wire_id(metas: &[PartMeta]) -> ClipId {
    let mut h = blake3::Hasher::new();
    h.update(b"uclip/wire-id/1");
    for m in metas {
        h.update(&[format_tag(m.format)]);
        h.update(&u64::from(m.size).to_le_bytes());
        h.update(&m.hash);
    }
    ClipId(*h.finalize().as_bytes())
}

/// A valid clip, with its wire ID and content hash computed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Clip {
    parts: Vec<RawPart>,
    metas: Vec<PartMeta>,
    id: ClipId,
    content: ContentHash,
}

impl Clip {
    /// Check the parts and build a clip. Parts are put in canonical order. Text must already be
    /// in wire form (see [`text::to_wire`]). Every part must be within its cap.
    pub fn from_parts(mut parts: Vec<RawPart>) -> Result<Clip, ClipError> {
        if parts.is_empty() || parts.len() > 3 {
            return Err(ClipError::PartCount);
        }
        parts.sort_by_key(|(f, _)| *f);
        if parts.windows(2).any(|w| w[0].0 == w[1].0) {
            return Err(ClipError::DuplicateFormat);
        }
        let has_text = parts.iter().any(|(f, _)| *f == Format::TextPlain);
        if !has_text && parts.iter().any(|(f, _)| *f == Format::TextHtml) {
            return Err(ClipError::HtmlWithoutText);
        }
        let mut content = blake3::Hasher::new();
        content.update(b"uclip/content/1");
        let mut metas = Vec::with_capacity(parts.len());
        for (format, data) in &parts {
            if data.len() > caps::cap(*format) {
                return Err(ClipError::TooLarge(*format));
            }
            match format {
                Format::TextPlain => {
                    let s = std::str::from_utf8(data).map_err(|_| ClipError::NotUtf8)?;
                    let n = text::normalize_for_hash(s);
                    content.update(b"T");
                    content.update(&(n.len() as u64).to_le_bytes());
                    content.update(n.as_bytes());
                }
                Format::TextHtml => {
                    // Not in the content hash (plan 3.8), but it must be text.
                    std::str::from_utf8(data).map_err(|_| ClipError::NotUtf8)?;
                }
                Format::ImagePng => {
                    content.update(b"I");
                    image::hash_pixels(data, &mut content).map_err(ClipError::Image)?;
                }
            }
            metas.push(PartMeta {
                format: *format,
                // Caps keep every part far below u32::MAX.
                size: u32::try_from(data.len()).map_err(|_| ClipError::TooLarge(*format))?,
                hash: *blake3::hash(data).as_bytes(),
            });
        }
        Ok(Clip {
            id: wire_id(&metas),
            content: ContentHash(*content.finalize().as_bytes()),
            parts,
            metas,
        })
    }

    /// Rebuild a clip from a header's part list and the received bytes, and check that every
    /// size and hash and the wire ID match.
    pub fn from_wire(
        id: ClipId,
        metas: &[PartMeta],
        data: Vec<Vec<u8>>,
    ) -> Result<Clip, ClipError> {
        if metas.len() != data.len() {
            return Err(ClipError::Mismatch);
        }
        for (m, d) in metas.iter().zip(&data) {
            if m.size as usize != d.len() {
                return Err(ClipError::Mismatch);
            }
        }
        let parts = metas.iter().map(|m| m.format).zip(data).collect();
        let clip = Clip::from_parts(parts)?;
        if clip.metas != metas || clip.id != id {
            return Err(ClipError::Mismatch);
        }
        Ok(clip)
    }

    /// The parts in canonical order, text in wire form (LF).
    pub fn parts(&self) -> &[RawPart] {
        &self.parts
    }

    /// The part list for a header.
    pub fn metas(&self) -> &[PartMeta] {
        &self.metas
    }

    /// The wire ID.
    pub fn id(&self) -> ClipId {
        self.id
    }

    /// The content hash.
    pub fn content_hash(&self) -> ContentHash {
        self.content
    }

    /// Total size of all parts in bytes.
    pub fn size(&self) -> usize {
        self.parts.iter().map(|(_, d)| d.len()).sum()
    }

    /// The bytes of one part.
    pub fn part(&self, format: Format) -> Option<&[u8]> {
        self.parts
            .iter()
            .find(|(f, _)| *f == format)
            .map(|(_, d)| d.as_slice())
    }

    /// The kind, for "Last activity".
    pub fn kind(&self) -> ClipKind {
        kind_of(self.metas.iter().map(|m| m.format))
    }
}

/// The kind of a list of formats.
pub fn kind_of(formats: impl Iterator<Item = Format>) -> ClipKind {
    let (mut text, mut img) = (false, false);
    for f in formats {
        match f {
            Format::TextPlain | Format::TextHtml => text = true,
            Format::ImagePng => img = true,
        }
    }
    match (text, img) {
        (_, false) => ClipKind::Text,
        (false, true) => ClipKind::Image,
        (true, true) => ClipKind::TextAndImage,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::tests::encode;
    use proptest::prelude::*;

    fn t(s: &str) -> RawPart {
        (Format::TextPlain, s.as_bytes().to_vec())
    }
    fn h(s: &str) -> RawPart {
        (Format::TextHtml, s.as_bytes().to_vec())
    }

    #[test]
    fn part_rules() {
        assert_eq!(Clip::from_parts(vec![]), Err(ClipError::PartCount));
        assert_eq!(
            Clip::from_parts(vec![t("a"), t("b")]),
            Err(ClipError::DuplicateFormat)
        );
        assert_eq!(
            Clip::from_parts(vec![h("<b>a</b>")]),
            Err(ClipError::HtmlWithoutText)
        );
        assert_eq!(
            Clip::from_parts(vec![(Format::TextPlain, vec![0xff])]),
            Err(ClipError::NotUtf8)
        );
        assert!(matches!(
            Clip::from_parts(vec![(Format::ImagePng, b"nope".to_vec())]),
            Err(ClipError::Image(_))
        ));
        assert_eq!(
            Clip::from_parts(vec![(Format::TextPlain, vec![b'a'; caps::TEXT_MAX + 1])]),
            Err(ClipError::TooLarge(Format::TextPlain))
        );
    }

    #[test]
    fn order_does_not_change_the_wire_id() {
        let a = Clip::from_parts(vec![t("a"), h("<i>a</i>")]).unwrap();
        let b = Clip::from_parts(vec![h("<i>a</i>"), t("a")]).unwrap();
        assert_eq!(a.id(), b.id());
        assert_eq!(a.parts()[0].0, Format::TextPlain);
    }

    #[test]
    fn content_hash_ignores_html_and_line_endings() {
        let a = Clip::from_parts(vec![t("a\nb")]).unwrap();
        let b = Clip::from_parts(vec![t("a\r\nb\0"), h("<p>a</p>")]).unwrap();
        assert_ne!(a.id(), b.id());
        assert_eq!(a.content_hash(), b.content_hash());
        let c = Clip::from_parts(vec![t("a b")]).unwrap();
        assert_ne!(a.content_hash(), c.content_hash());
    }

    #[test]
    fn re_encoded_image_keeps_content_hash() {
        let rgba: Vec<u8> = (0..4 * 6 * 6).map(|i| (i * 7) as u8).collect();
        let p1 = encode(
            6,
            6,
            &rgba,
            png::ColorType::Rgba,
            png::Compression::NoCompression,
        );
        let p2 = encode(6, 6, &rgba, png::ColorType::Rgba, png::Compression::High);
        let a = Clip::from_parts(vec![(Format::ImagePng, p1)]).unwrap();
        let b = Clip::from_parts(vec![(Format::ImagePng, p2)]).unwrap();
        assert_ne!(a.id(), b.id());
        assert_eq!(a.content_hash(), b.content_hash());
        assert_eq!(a.kind(), ClipKind::Image);
    }

    #[test]
    fn text_and_image_hash_separately() {
        let rgba = vec![1u8; 16];
        let png = encode(2, 2, &rgba, png::ColorType::Rgba, png::Compression::Fast);
        let both = Clip::from_parts(vec![t("x"), (Format::ImagePng, png.clone())]).unwrap();
        let img = Clip::from_parts(vec![(Format::ImagePng, png)]).unwrap();
        let txt = Clip::from_parts(vec![t("x")]).unwrap();
        assert_ne!(both.content_hash(), img.content_hash());
        assert_ne!(both.content_hash(), txt.content_hash());
        assert_eq!(both.kind(), ClipKind::TextAndImage);
    }

    #[test]
    fn from_wire_checks_everything() {
        let c = Clip::from_parts(vec![t("hello"), h("<b>hello</b>")]).unwrap();
        let data: Vec<Vec<u8>> = c.parts().iter().map(|(_, d)| d.clone()).collect();
        let ok = Clip::from_wire(c.id(), c.metas(), data.clone()).unwrap();
        assert_eq!(ok, c);

        let mut bad = data.clone();
        bad[0] = b"hellp".to_vec();
        assert_eq!(
            Clip::from_wire(c.id(), c.metas(), bad),
            Err(ClipError::Mismatch)
        );
        assert_eq!(
            Clip::from_wire(ClipId([9; 32]), c.metas(), data.clone()),
            Err(ClipError::Mismatch)
        );
        assert_eq!(
            Clip::from_wire(c.id(), &c.metas()[..1], data),
            Err(ClipError::Mismatch)
        );
    }

    proptest! {
        #[test]
        fn wire_id_is_deterministic_and_content_sensitive(a in ".{0,50}", b in ".{0,50}") {
            let ca = Clip::from_parts(vec![t(&a)]).unwrap();
            let ca2 = Clip::from_parts(vec![t(&a)]).unwrap();
            prop_assert_eq!(ca.id(), ca2.id());
            let cb = Clip::from_parts(vec![t(&b)]).unwrap();
            prop_assert_eq!(a == b, ca.id() == cb.id());
        }
    }
}
