//! Device-independent bitmaps (`CF_DIB`, `CF_DIBV5`) to and from RGBA pixels and PNG.
//!
//! The DIB bytes come from other programs, so the parser treats them as hostile: every offset
//! and size is checked before use, and the pixel count is limited before any allocation.
//! This module has no OS calls and is tested on every OS.

use std::io::Cursor;

/// At most this many pixels per side (same limit as `uc_core::image`).
pub const MAX_SIDE: u32 = 16_384;
/// At most this many pixels in total (same limit as `uc_core::image`).
pub const MAX_PIXELS: u64 = 100_000_000;

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;
/// Size of `BITMAPV5HEADER`.
pub const V5_HEADER: usize = 124;

/// Why a DIB or PNG could not be converted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DibError {
    /// The data ends before the header, the masks, the palette or the pixels.
    Truncated,
    /// A header field has an impossible value.
    BadHeader,
    /// A valid but unsupported variant (RLE, JPEG or PNG inside a DIB, an unknown header).
    Unsupported,
    /// Over [`MAX_SIDE`] or [`MAX_PIXELS`].
    TooLarge,
    /// The PNG could not be decoded or encoded.
    Png,
}

impl std::fmt::Display for DibError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DibError::Truncated => "bitmap data is truncated",
            DibError::BadHeader => "bitmap header is invalid",
            DibError::Unsupported => "bitmap variant is not supported",
            DibError::TooLarge => "bitmap is over the size limit",
            DibError::Png => "PNG conversion failed",
        })
    }
}

impl std::error::Error for DibError {}

/// An image as straight (not premultiplied) RGBA8, rows from top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes.
    pub pixels: Vec<u8>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn check_size(width: u32, height: u32) -> Result<(), DibError> {
    if width == 0 || height == 0 {
        return Err(DibError::BadHeader);
    }
    if width > MAX_SIDE || height > MAX_SIDE || u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(DibError::TooLarge);
    }
    Ok(())
}

/// One color channel described by a bit mask.
#[derive(Clone, Copy)]
struct Channel {
    mask: u32,
    shift: u32,
    max: u32,
}

impl Channel {
    fn new(mask: u32) -> Channel {
        if mask == 0 {
            return Channel {
                mask: 0,
                shift: 0,
                max: 0,
            };
        }
        let shift = mask.trailing_zeros();
        Channel {
            mask,
            shift,
            max: mask >> shift,
        }
    }

    /// The channel value scaled to 0..=255, or `None` when the mask is empty.
    fn get(self, v: u32) -> Option<u8> {
        if self.max == 0 {
            return None;
        }
        let raw = u64::from((v & self.mask) >> self.shift);
        // raw <= max, so the result is <= 255.
        Some((raw * 255 / u64::from(self.max)) as u8)
    }
}

/// Convert a packed DIB (`BITMAPINFOHEADER` or a later header, then masks, palette and
/// pixels) into RGBA.
///
/// Supported: 1, 4 and 8 bit with a palette; 16 and 32 bit with `BI_RGB` or bit fields;
/// 24 bit. Bottom-up and top-down rows. A 32-bit image whose alpha bytes are all zero is
/// taken as opaque, because many programs leave that byte at zero.
pub fn dib_to_rgba(dib: &[u8]) -> Result<Rgba, DibError> {
    let header = u32_at(dib, 0).ok_or(DibError::Truncated)? as usize;
    if !matches!(header, 40 | 52 | 56 | 108 | 124) {
        return Err(DibError::Unsupported);
    }
    if dib.len() < header {
        return Err(DibError::Truncated);
    }
    let width = i32::from_le_bytes(dib[4..8].try_into().map_err(|_| DibError::Truncated)?);
    let height = i32::from_le_bytes(dib[8..12].try_into().map_err(|_| DibError::Truncated)?);
    let bpp = u16_at(dib, 14).ok_or(DibError::Truncated)?;
    let compression = u32_at(dib, 16).ok_or(DibError::Truncated)?;
    let colors_used = u32_at(dib, 32).ok_or(DibError::Truncated)?;
    if width <= 0 || height == 0 || height == i32::MIN {
        return Err(DibError::BadHeader);
    }
    let w = width.unsigned_abs();
    let h = height.unsigned_abs();
    let top_down = height < 0;
    check_size(w, h)?;

    let mut offset = header;
    let mut masks: Option<[u32; 4]> = None;
    match compression {
        BI_RGB => {}
        BI_BITFIELDS | BI_ALPHABITFIELDS => {
            if bpp != 16 && bpp != 32 {
                return Err(DibError::BadHeader);
            }
            let with_alpha = compression == BI_ALPHABITFIELDS;
            if header == 40 {
                // The masks follow a plain BITMAPINFOHEADER.
                let n = if with_alpha { 4 } else { 3 };
                let mut m = [0u32; 4];
                for (i, slot) in m.iter_mut().enumerate().take(n) {
                    *slot = u32_at(dib, 40 + i * 4).ok_or(DibError::Truncated)?;
                }
                offset += n * 4;
                masks = Some(m);
            } else {
                // Later headers hold the masks (alpha from 56 bytes on).
                let alpha = if header >= 56 {
                    u32_at(dib, 52).ok_or(DibError::Truncated)?
                } else {
                    0
                };
                masks = Some([
                    u32_at(dib, 40).ok_or(DibError::Truncated)?,
                    u32_at(dib, 44).ok_or(DibError::Truncated)?,
                    u32_at(dib, 48).ok_or(DibError::Truncated)?,
                    alpha,
                ]);
            }
        }
        _ => return Err(DibError::Unsupported),
    }

    let mut palette: Vec<[u8; 4]> = Vec::new();
    match bpp {
        1 | 4 | 8 => {
            if compression != BI_RGB {
                return Err(DibError::Unsupported);
            }
            let max = 1u32 << bpp;
            let n = if colors_used == 0 { max } else { colors_used };
            if n > max {
                return Err(DibError::BadHeader);
            }
            let n = n as usize;
            let bytes = dib.get(offset..offset + n * 4).ok_or(DibError::Truncated)?;
            palette = bytes
                .as_chunks::<4>()
                .0
                .iter()
                .map(|c| [c[2], c[1], c[0], 255])
                .collect();
            offset += n * 4;
        }
        16 | 24 | 32 => {}
        _ => return Err(DibError::Unsupported),
    }

    let stride = (u64::from(w) * u64::from(bpp)).div_ceil(32) * 4;
    let need = stride * u64::from(h);
    let pixels_len = (dib.len() - offset.min(dib.len())) as u64;
    if offset > dib.len() || pixels_len < need {
        return Err(DibError::Truncated);
    }
    let stride = stride as usize;
    let data = &dib[offset..];

    let (w, h) = (w as usize, h as usize);
    let mut out = vec![0u8; w * h * 4];
    let channels = masks.map(|m| m.map(Channel::new));
    let default16 = [0x7C00, 0x03E0, 0x001F, 0].map(Channel::new);
    let mut any_alpha = false;

    for y in 0..h {
        let src_y = if top_down { y } else { h - 1 - y };
        let row = &data[src_y * stride..src_y * stride + stride];
        let dst = &mut out[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let px: [u8; 4] = match bpp {
                1 | 4 | 8 => {
                    let bits = bpp as usize;
                    let bit = x * bits;
                    let byte = row[bit / 8];
                    let shift = 8 - bits - (bit % 8);
                    let index = (byte >> shift) as usize & ((1 << bits) - 1);
                    // An index past a short palette is black, as GDI does.
                    palette.get(index).copied().unwrap_or([0, 0, 0, 255])
                }
                16 => {
                    let v = u32::from(u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]));
                    let ch = channels.unwrap_or(default16);
                    let a = ch[3].get(v);
                    any_alpha |= a.is_some_and(|a| a != 0);
                    [
                        ch[0].get(v).unwrap_or(0),
                        ch[1].get(v).unwrap_or(0),
                        ch[2].get(v).unwrap_or(0),
                        a.unwrap_or(255),
                    ]
                }
                24 => {
                    let p = &row[x * 3..x * 3 + 3];
                    [p[2], p[1], p[0], 255]
                }
                _ => {
                    let p = &row[x * 4..x * 4 + 4];
                    match channels {
                        None => {
                            any_alpha |= p[3] != 0;
                            [p[2], p[1], p[0], p[3]]
                        }
                        Some(ch) => {
                            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
                            let a = ch[3].get(v);
                            any_alpha |= a.is_some_and(|a| a != 0);
                            [
                                ch[0].get(v).unwrap_or(0),
                                ch[1].get(v).unwrap_or(0),
                                ch[2].get(v).unwrap_or(0),
                                a.unwrap_or(255),
                            ]
                        }
                    }
                }
            };
            dst[x * 4..x * 4 + 4].copy_from_slice(&px);
        }
    }

    // 32-bit (and 16-bit with an alpha mask) images with all alpha at zero: opaque.
    let alpha_channel = match bpp {
        32 => channels.is_none_or(|ch| ch[3].max != 0),
        16 => channels.is_some_and(|ch| ch[3].max != 0),
        _ => false,
    };
    if alpha_channel && !any_alpha {
        for px in out.as_chunks_mut::<4>().0.iter_mut() {
            px[3] = 255;
        }
    }
    Ok(Rgba {
        width: w as u32,
        height: h as u32,
        pixels: out,
    })
}

/// Build a `CF_DIBV5` (a `BITMAPV5HEADER` with 32-bit `BI_BITFIELDS` pixels, sRGB, straight
/// alpha, bottom-up rows) from RGBA.
pub fn rgba_to_dibv5(img: &Rgba) -> Result<Vec<u8>, DibError> {
    check_size(img.width, img.height)?;
    let (w, h) = (img.width as usize, img.height as usize);
    if img.pixels.len() != w * h * 4 {
        return Err(DibError::BadHeader);
    }
    let size = w * h * 4;
    let mut out = Vec::with_capacity(V5_HEADER + size);
    let put = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
    put(&mut out, V5_HEADER as u32); // bV5Size
    put(&mut out, img.width); // bV5Width
    put(&mut out, img.height); // bV5Height (positive: bottom-up)
    out.extend_from_slice(&1u16.to_le_bytes()); // bV5Planes
    out.extend_from_slice(&32u16.to_le_bytes()); // bV5BitCount
    put(&mut out, BI_BITFIELDS); // bV5Compression
    put(
        &mut out,
        u32::try_from(size).map_err(|_| DibError::TooLarge)?,
    ); // bV5SizeImage
    put(&mut out, 2835); // bV5XPelsPerMeter (72 dpi)
    put(&mut out, 2835); // bV5YPelsPerMeter
    put(&mut out, 0); // bV5ClrUsed
    put(&mut out, 0); // bV5ClrImportant
    put(&mut out, 0x00FF_0000); // red mask
    put(&mut out, 0x0000_FF00); // green mask
    put(&mut out, 0x0000_00FF); // blue mask
    put(&mut out, 0xFF00_0000); // alpha mask
    put(&mut out, 0x7352_4742); // bV5CSType = LCS_sRGB ('sRGB')
    out.extend_from_slice(&[0u8; 36]); // bV5Endpoints
    put(&mut out, 0); // gamma red
    put(&mut out, 0); // gamma green
    put(&mut out, 0); // gamma blue
    put(&mut out, 4); // bV5Intent = LCS_GM_IMAGES
    put(&mut out, 0); // bV5ProfileData
    put(&mut out, 0); // bV5ProfileSize
    put(&mut out, 0); // bV5Reserved
    debug_assert_eq!(out.len(), V5_HEADER);
    for y in (0..h).rev() {
        for px in img.pixels[y * w * 4..(y + 1) * w * 4]
            .as_chunks::<4>()
            .0
            .iter()
        {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    Ok(out)
}

/// Decode a PNG into RGBA. The size is checked from the header before the pixels are decoded.
pub fn png_to_rgba(png_bytes: &[u8]) -> Result<Rgba, DibError> {
    let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| DibError::Png)?;
    let (width, height) = (reader.info().width, reader.info().height);
    check_size(width, height)?;
    let size = reader.output_buffer_size().ok_or(DibError::Png)?;
    let mut buf = vec![0u8; size];
    let frame = reader.next_frame(&mut buf).map_err(|_| DibError::Png)?;
    if frame.bit_depth != png::BitDepth::Eight {
        return Err(DibError::Png);
    }
    let channels = match frame.color_type {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(DibError::Png),
    };
    let (w, h) = (width as usize, height as usize);
    let line = frame.line_size;
    let mut pixels = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = buf
            .get(y * line..y * line + w * channels)
            .ok_or(DibError::Png)?;
        for p in row.chunks_exact(channels) {
            match channels {
                1 => pixels.extend_from_slice(&[p[0], p[0], p[0], 255]),
                2 => pixels.extend_from_slice(&[p[0], p[0], p[0], p[1]]),
                3 => pixels.extend_from_slice(&[p[0], p[1], p[2], 255]),
                _ => pixels.extend_from_slice(p),
            }
        }
    }
    Ok(Rgba {
        width,
        height,
        pixels,
    })
}

/// Encode RGBA as a PNG. A fully opaque image is stored as RGB, which is smaller.
pub fn rgba_to_png(img: &Rgba) -> Result<Vec<u8>, DibError> {
    check_size(img.width, img.height)?;
    let opaque = img.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255);
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let data: Vec<u8> = if opaque {
            enc.set_color(png::ColorType::Rgb);
            img.pixels
                .as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect()
        } else {
            enc.set_color(png::ColorType::Rgba);
            img.pixels.clone()
        };
        let mut writer = enc.write_header().map_err(|_| DibError::Png)?;
        writer.write_image_data(&data).map_err(|_| DibError::Png)?;
    }
    Ok(out)
}

/// Convert a DIB straight to PNG.
pub fn dib_to_png(dib: &[u8]) -> Result<Vec<u8>, DibError> {
    rgba_to_png(&dib_to_rgba(dib)?)
}

/// Cut a PNG after its `IEND` chunk. Windows rounds clipboard memory up, so a PNG read from
/// the clipboard can carry extra bytes at the end. Returns the input unchanged when the chunk
/// walk fails; the core checks the PNG later anyway.
pub fn trim_png(png: &[u8]) -> &[u8] {
    const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    if png.get(..8) != Some(&SIGNATURE[..]) {
        return png;
    }
    let mut at = 8usize;
    while let Some(len) = png.get(at..at + 4) {
        let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let Some(end) = at.checked_add(12).and_then(|v| v.checked_add(len)) else {
            return png;
        };
        if end > png.len() {
            return png;
        }
        if &png[at + 4..at + 8] == b"IEND" {
            return &png[..end];
        }
        at = end;
    }
    png
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A DIB with a 40-byte header and the given pixel bytes (rows already padded).
    pub(crate) fn info_dib(w: i32, h: i32, bpp: u16, compression: u32, extra: &[u8]) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&40u32.to_le_bytes());
        v.extend_from_slice(&w.to_le_bytes());
        v.extend_from_slice(&h.to_le_bytes());
        v.extend_from_slice(&1u16.to_le_bytes());
        v.extend_from_slice(&bpp.to_le_bytes());
        v.extend_from_slice(&compression.to_le_bytes());
        v.extend_from_slice(&[0u8; 20]);
        assert_eq!(v.len(), 40);
        v.extend_from_slice(extra);
        v
    }

    fn sample(w: u32, h: u32) -> Rgba {
        let pixels = (0..w * h)
            .flat_map(|i| [i as u8, (i * 3) as u8, (i * 7) as u8, (100 + i) as u8])
            .collect();
        Rgba {
            width: w,
            height: h,
            pixels,
        }
    }

    #[test]
    fn dibv5_round_trip_keeps_alpha() {
        let img = sample(5, 3);
        let dib = rgba_to_dibv5(&img).unwrap();
        assert_eq!(dib.len(), V5_HEADER + 5 * 3 * 4);
        assert_eq!(dib_to_rgba(&dib).unwrap(), img);
    }

    #[test]
    fn png_round_trip() {
        let img = sample(4, 4);
        let png = rgba_to_png(&img).unwrap();
        assert_eq!(png_to_rgba(&png).unwrap(), img);
        // The core accepts what we make.
        uc_core::Clip::from_parts(vec![(uc_clip::Format::ImagePng, png)]).unwrap();
    }

    #[test]
    fn opaque_png_is_rgb_and_same_pixels() {
        let mut img = sample(3, 2);
        for p in img.pixels.as_chunks_mut::<4>().0.iter_mut() {
            p[3] = 255;
        }
        let png = rgba_to_png(&img).unwrap();
        assert_eq!(png[25], 2, "color type RGB");
        assert_eq!(png_to_rgba(&png).unwrap(), img);
    }

    #[test]
    fn bottom_up_24_bit_with_padding() {
        // 2x2, rows are 6 bytes + 2 padding. Bottom row first.
        let mut px = Vec::new();
        px.extend_from_slice(&[1, 2, 3, 4, 5, 6, 0, 0]); // bottom: (3,2,1) (6,5,4)
        px.extend_from_slice(&[7, 8, 9, 10, 11, 12, 0, 0]); // top
        let dib = info_dib(2, 2, 24, BI_RGB, &px);
        let img = dib_to_rgba(&dib).unwrap();
        assert_eq!(
            img.pixels,
            vec![9, 8, 7, 255, 12, 11, 10, 255, 3, 2, 1, 255, 6, 5, 4, 255]
        );
    }

    #[test]
    fn top_down_32_bit_zero_alpha_is_opaque() {
        let px = [10, 20, 30, 0, 40, 50, 60, 0];
        let dib = info_dib(1, -2, 32, BI_RGB, &px);
        let img = dib_to_rgba(&dib).unwrap();
        assert_eq!(img.pixels, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn bi_rgb_32_bit_with_alpha_keeps_it() {
        let px = [10, 20, 30, 128, 40, 50, 60, 0];
        let dib = info_dib(1, -2, 32, BI_RGB, &px);
        let img = dib_to_rgba(&dib).unwrap();
        assert_eq!(img.pixels, vec![30, 20, 10, 128, 60, 50, 40, 0]);
    }

    #[test]
    fn bitfields_after_info_header() {
        // 32-bit, masks for BGR in a different order: R in the low byte.
        let mut extra = Vec::new();
        for m in [0x0000_00FFu32, 0x0000_FF00, 0x00FF_0000] {
            extra.extend_from_slice(&m.to_le_bytes());
        }
        extra.extend_from_slice(&[1, 2, 3, 99]);
        let dib = info_dib(1, 1, 32, BI_BITFIELDS, &extra);
        assert_eq!(dib_to_rgba(&dib).unwrap().pixels, vec![1, 2, 3, 255]);
    }

    #[test]
    fn rgb565_bitfields_scale_to_8_bit() {
        let mut extra = Vec::new();
        for m in [0xF800u32, 0x07E0, 0x001F] {
            extra.extend_from_slice(&m.to_le_bytes());
        }
        extra.extend_from_slice(&0xFFFFu16.to_le_bytes());
        extra.extend_from_slice(&[0, 0]); // row padding
        let dib = info_dib(1, 1, 16, BI_BITFIELDS, &extra);
        assert_eq!(dib_to_rgba(&dib).unwrap().pixels, vec![255, 255, 255, 255]);
    }

    #[test]
    fn rgb555_default() {
        // pure red in 5-5-5: 0x7C00
        let mut extra = 0x7C00u16.to_le_bytes().to_vec();
        extra.extend_from_slice(&[0, 0]);
        let dib = info_dib(1, 1, 16, BI_RGB, &extra);
        assert_eq!(dib_to_rgba(&dib).unwrap().pixels, vec![255, 0, 0, 255]);
    }

    #[test]
    fn eight_bit_palette() {
        let mut extra = Vec::new();
        // palette of 2 (colors used = 2 at offset 32)
        extra.extend_from_slice(&[0, 0, 255, 0, 0, 255, 0, 0]); // red, green (BGRx)
        extra.extend_from_slice(&[1, 0, 5, 0]); // indices 1, 0, 5 (past palette: black)
        let mut dib = info_dib(3, 1, 8, BI_RGB, &extra);
        dib[32..36].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(
            dib_to_rgba(&dib).unwrap().pixels,
            vec![0, 255, 0, 255, 255, 0, 0, 255, 0, 0, 0, 255]
        );
    }

    #[test]
    fn one_bit_palette() {
        let mut extra = vec![0, 0, 0, 0, 255, 255, 255, 0];
        extra.extend_from_slice(&[0b1010_0000, 0, 0, 0]);
        let dib = info_dib(3, 1, 1, BI_RGB, &extra);
        assert_eq!(
            dib_to_rgba(&dib).unwrap().pixels,
            vec![255, 255, 255, 255, 0, 0, 0, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn refuses_bad_input() {
        assert_eq!(dib_to_rgba(&[]), Err(DibError::Truncated));
        assert_eq!(
            dib_to_rgba(&info_dib(2, 2, 24, BI_RGB, &[0; 4])),
            Err(DibError::Truncated)
        );
        assert_eq!(
            dib_to_rgba(&info_dib(0, 2, 24, BI_RGB, &[])),
            Err(DibError::BadHeader)
        );
        assert_eq!(
            dib_to_rgba(&info_dib(20_000, 1, 24, BI_RGB, &[])),
            Err(DibError::TooLarge)
        );
        assert_eq!(
            dib_to_rgba(&info_dib(10_001, 10_000, 24, BI_RGB, &[])),
            Err(DibError::TooLarge)
        );
        assert_eq!(
            dib_to_rgba(&info_dib(1, 1, 8, 1 /* BI_RLE8 */, &[])),
            Err(DibError::Unsupported)
        );
        assert_eq!(
            dib_to_rgba(&info_dib(1, 1, 32, 5 /* BI_PNG */, &[])),
            Err(DibError::Unsupported)
        );
        assert_eq!(
            dib_to_rgba(&info_dib(1, i32::MIN, 32, BI_RGB, &[])),
            Err(DibError::BadHeader)
        );
    }

    #[test]
    fn trim_png_cuts_after_iend() {
        let png = rgba_to_png(&sample(2, 2)).unwrap();
        let mut padded = png.clone();
        padded.extend_from_slice(&[0u8; 13]);
        assert_eq!(trim_png(&padded), &png[..]);
        assert_eq!(trim_png(b"not a png"), b"not a png");
        assert_eq!(trim_png(&png[..png.len() - 3]), &png[..png.len() - 3]);
    }

    proptest! {
        #[test]
        fn random_dibs_never_panic(
            header in prop_oneof![Just(40u32), Just(108u32), Just(124u32), any::<u32>()],
            w in -40i32..40,
            h in -40i32..40,
            bpp in prop_oneof![Just(1u16), Just(4), Just(8), Just(16), Just(24), Just(32), any::<u16>()],
            compression in 0u32..8,
            colors in prop_oneof![Just(0u32), 0u32..300, any::<u32>()],
            tail in proptest::collection::vec(any::<u8>(), 0..2000),
        ) {
            let mut dib = Vec::new();
            dib.extend_from_slice(&header.to_le_bytes());
            dib.extend_from_slice(&w.to_le_bytes());
            dib.extend_from_slice(&h.to_le_bytes());
            dib.extend_from_slice(&1u16.to_le_bytes());
            dib.extend_from_slice(&bpp.to_le_bytes());
            dib.extend_from_slice(&compression.to_le_bytes());
            dib.extend_from_slice(&[0u8; 12]);
            dib.extend_from_slice(&colors.to_le_bytes());
            dib.extend_from_slice(&tail);
            if let Ok(img) = dib_to_rgba(&dib) {
                prop_assert_eq!(img.pixels.len(), img.width as usize * img.height as usize * 4);
            }
            for cut in [0, 3, 12, 39, 40, 60] {
                let _ = dib_to_rgba(&dib[..cut.min(dib.len())]);
            }
        }

        #[test]
        fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
            let _ = dib_to_rgba(&bytes);
            let _ = png_to_rgba(&bytes);
            let _ = trim_png(&bytes);
        }

        #[test]
        fn dibv5_round_trip_any(w in 1u32..12, h in 1u32..12, seed in any::<u8>()) {
            let pixels = (0..w * h * 4).map(|i| (i as u8).wrapping_mul(seed | 1)).collect();
            let img = Rgba { width: w, height: h, pixels };
            let back = dib_to_rgba(&rgba_to_dibv5(&img).unwrap()).unwrap();
            if img.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
                prop_assert!(back.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            } else {
                prop_assert_eq!(back, img);
            }
        }
    }
}
