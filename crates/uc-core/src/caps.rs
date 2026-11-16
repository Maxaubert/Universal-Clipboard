//! Size caps (plan 3.11).
//!
//! | Part  | Cap   | Over the cap |
//! |-------|-------|--------------|
//! | text  | 1 MiB | not synced (and its HTML twin goes with it) |
//! | HTML  | 2 MiB | the HTML part is dropped, the text is still sent |
//! | image | 25 MiB| not synced |
//!
//! In a mixed clip each part is checked on its own, so the parts within their caps are still
//! sent. A clip with no part left is not synced at all.

use uc_clip::{Format, RawPart};

/// Plain text cap (1 MiB).
pub const TEXT_MAX: usize = 1 << 20;
/// HTML cap (2 MiB).
pub const HTML_MAX: usize = 2 << 20;
/// PNG cap (25 MiB).
pub const IMAGE_MAX: usize = 25 << 20;
/// Clips up to this total size are pushed at once; larger ones are announced with `Latest`.
pub const PUSH_MAX: usize = 256 << 10;

/// Largest file copy that syncs automatically, by default (setting `file-limit-mb`, file sync
/// spec 5). Copies picked with "Copy files..." or `uc copy` have no byte limit.
pub const FILE_LIMIT_DEFAULT_MB: u64 = 100;
/// Smallest value of the `file-limit-mb` setting.
pub const FILE_LIMIT_MIN_MB: u64 = 1;
/// Largest value of the `file-limit-mb` setting (10 GiB).
pub const FILE_LIMIT_MAX_MB: u64 = 10_240;

/// The automatic file copy limit in bytes for a `file-limit-mb` setting, clamped to its range.
pub fn file_limit_bytes(mb: u64) -> u64 {
    mb.clamp(FILE_LIMIT_MIN_MB, FILE_LIMIT_MAX_MB) << 20
}

/// The cap for one format.
pub fn cap(format: Format) -> usize {
    match format {
        Format::TextPlain => TEXT_MAX,
        Format::TextHtml => HTML_MAX,
        Format::ImagePng => IMAGE_MAX,
    }
}

/// A part that was left out, for "Last activity" (never the content, only kind and size).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dropped {
    /// Which part.
    pub format: Format,
    /// Its size in bytes.
    pub size: usize,
}

/// Drop every part over its cap. HTML without its text twin is dropped too.
pub fn fit(parts: Vec<RawPart>) -> (Vec<RawPart>, Vec<Dropped>) {
    let mut dropped = Vec::new();
    let mut kept: Vec<RawPart> = Vec::with_capacity(parts.len());
    for (format, data) in parts {
        if data.len() > cap(format) {
            dropped.push(Dropped {
                format,
                size: data.len(),
            });
        } else {
            kept.push((format, data));
        }
    }
    let has_text = kept.iter().any(|(f, _)| *f == Format::TextPlain);
    if !has_text && let Some(i) = kept.iter().position(|(f, _)| *f == Format::TextHtml) {
        let (format, data) = kept.remove(i);
        dropped.push(Dropped {
            format,
            size: data.len(),
        });
    }
    (kept, dropped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_limit_is_clamped() {
        assert_eq!(file_limit_bytes(FILE_LIMIT_DEFAULT_MB), 100 << 20);
        assert_eq!(file_limit_bytes(0), 1 << 20);
        assert_eq!(file_limit_bytes(u64::MAX), 10_240 << 20);
    }

    fn part(f: Format, n: usize) -> RawPart {
        (f, vec![b'a'; n])
    }

    #[test]
    fn html_over_cap_is_dropped_text_kept() {
        let (kept, dropped) = fit(vec![
            part(Format::TextPlain, 10),
            part(Format::TextHtml, HTML_MAX + 1),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].0, Format::TextPlain);
        assert_eq!(
            dropped,
            vec![Dropped {
                format: Format::TextHtml,
                size: HTML_MAX + 1
            }]
        );
    }

    #[test]
    fn text_over_cap_takes_html_with_it() {
        let (kept, dropped) = fit(vec![
            part(Format::TextPlain, TEXT_MAX + 1),
            part(Format::TextHtml, 10),
        ]);
        assert!(kept.is_empty());
        assert_eq!(dropped.len(), 2);
    }

    #[test]
    fn exactly_at_cap_is_kept() {
        let (kept, dropped) = fit(vec![
            part(Format::TextPlain, TEXT_MAX),
            part(Format::ImagePng, IMAGE_MAX),
        ]);
        assert_eq!(kept.len(), 2);
        assert!(dropped.is_empty());
    }

    #[test]
    fn mixed_clip_keeps_the_parts_within_caps() {
        let (kept, dropped) = fit(vec![
            part(Format::TextPlain, 5),
            part(Format::ImagePng, IMAGE_MAX + 1),
        ]);
        assert_eq!(kept, vec![part(Format::TextPlain, 5)]);
        assert_eq!(dropped[0].format, Format::ImagePng);
    }
}
