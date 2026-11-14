//! PNG safety checks and the pixel part of the content hash (plan 3.8, 3.11).

use std::io::Cursor;

/// At most this many pixels per side.
pub const MAX_SIDE: u32 = 16_384;
/// At most this many pixels in total (100 megapixels).
pub const MAX_PIXELS: u64 = 100_000_000;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];

/// Why an image was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// Not a PNG, or the header is broken.
    BadHeader,
    /// Width or height is zero, or above [`MAX_SIDE`], or the area is above [`MAX_PIXELS`].
    TooManyPixels { width: u32, height: u32 },
    /// The pixel data could not be decoded.
    Decode,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::BadHeader => f.write_str("not a valid PNG header"),
            ImageError::TooManyPixels { width, height } => {
                write!(f, "image size {width}x{height} is over the limit")
            }
            ImageError::Decode => f.write_str("PNG data could not be decoded"),
        }
    }
}

impl std::error::Error for ImageError {}

/// Check the PNG signature and the IHDR size **before** any decoding. Returns (width, height).
pub fn check_header(png: &[u8]) -> Result<(u32, u32), ImageError> {
    // signature (8) + IHDR length (4) + "IHDR" (4) + width (4) + height (4)
    if png.len() < 24 || png[..8] != SIGNATURE || &png[12..16] != b"IHDR" {
        return Err(ImageError::BadHeader);
    }
    if u32::from_be_bytes([png[8], png[9], png[10], png[11]]) != 13 {
        return Err(ImageError::BadHeader);
    }
    let width = u32::from_be_bytes([png[16], png[17], png[18], png[19]]);
    let height = u32::from_be_bytes([png[20], png[21], png[22], png[23]]);
    if width == 0
        || height == 0
        || width > MAX_SIDE
        || height > MAX_SIDE
        || u64::from(width) * u64::from(height) > MAX_PIXELS
    {
        return Err(ImageError::TooManyPixels { width, height });
    }
    Ok((width, height))
}

/// Feed the decoded pixels (width, height, then RGBA8 rows) into `hasher`.
///
/// Two PNG files with the same pixels give the same bytes here, whatever their compression,
/// filters, bit depth or color type. Uses only the memory-safe `png` crate.
pub fn hash_pixels(png: &[u8], hasher: &mut blake3::Hasher) -> Result<(), ImageError> {
    let (width, height) = check_header(png)?;
    let mut decoder = png::Decoder::new(Cursor::new(png));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|_| ImageError::Decode)?;
    let info = reader.info();
    if info.width != width || info.height != height {
        return Err(ImageError::BadHeader);
    }
    let interlaced = info.interlaced;
    let (color, depth) = reader.output_color_type();
    if depth != png::BitDepth::Eight {
        return Err(ImageError::Decode);
    }
    let channels = match color {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return Err(ImageError::Decode),
    };
    let row_len = width as usize * channels;

    hasher.update(&width.to_le_bytes());
    hasher.update(&height.to_le_bytes());
    let mut rgba = Vec::with_capacity(width as usize * 4);
    let mut push_row = |row: &[u8], hasher: &mut blake3::Hasher| -> Result<(), ImageError> {
        if row.len() != row_len {
            return Err(ImageError::Decode);
        }
        rgba.clear();
        for px in row.chunks_exact(channels) {
            match channels {
                1 => rgba.extend_from_slice(&[px[0], px[0], px[0], 255]),
                2 => rgba.extend_from_slice(&[px[0], px[0], px[0], px[1]]),
                3 => rgba.extend_from_slice(&[px[0], px[1], px[2], 255]),
                _ => rgba.extend_from_slice(px),
            }
        }
        hasher.update(&rgba);
        Ok(())
    };

    if interlaced {
        // Interlaced rows come in passes, so decode the whole frame first.
        let size = reader.output_buffer_size().ok_or(ImageError::Decode)?;
        let mut buf = vec![0u8; size];
        let out = reader
            .next_frame(&mut buf)
            .map_err(|_| ImageError::Decode)?;
        let line = out.line_size;
        for y in 0..height as usize {
            let start = y * line;
            let row = buf.get(start..start + row_len).ok_or(ImageError::Decode)?;
            push_row(row, hasher)?;
        }
    } else {
        let mut rows = 0u32;
        while let Some(row) = reader.next_row().map_err(|_| ImageError::Decode)? {
            push_row(row.data(), hasher)?;
            rows += 1;
        }
        if rows != height {
            return Err(ImageError::Decode);
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Encode RGBA pixels as a PNG with the given settings.
    pub(crate) fn encode(
        w: u32,
        h: u32,
        rgba: &[u8],
        color: png::ColorType,
        compression: png::Compression,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(color);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_compression(compression);
            let mut wr = enc.write_header().unwrap();
            let data: Vec<u8> = match color {
                png::ColorType::Rgb => rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .flat_map(|p| [p[0], p[1], p[2]])
                    .collect(),
                _ => rgba.to_vec(),
            };
            wr.write_image_data(&data).unwrap();
        }
        out
    }

    fn pixel_hash(png: &[u8]) -> Result<[u8; 32], ImageError> {
        let mut h = blake3::Hasher::new();
        hash_pixels(png, &mut h)?;
        Ok(*h.finalize().as_bytes())
    }

    #[test]
    fn same_pixels_same_hash_across_encodings() {
        let rgba: Vec<u8> = (0..4 * 5 * 3)
            .map(|i| if i % 4 == 3 { 255 } else { i as u8 })
            .collect();
        let a = encode(
            5,
            3,
            &rgba,
            png::ColorType::Rgba,
            png::Compression::NoCompression,
        );
        let b = encode(5, 3, &rgba, png::ColorType::Rgba, png::Compression::High);
        let c = encode(5, 3, &rgba, png::ColorType::Rgb, png::Compression::Fast);
        assert_ne!(a, b);
        assert_eq!(pixel_hash(&a).unwrap(), pixel_hash(&b).unwrap());
        assert_eq!(pixel_hash(&a).unwrap(), pixel_hash(&c).unwrap());
    }

    #[test]
    fn different_pixels_different_hash() {
        let mut rgba = vec![255u8; 4 * 4];
        let a = encode(2, 2, &rgba, png::ColorType::Rgba, png::Compression::Fast);
        rgba[0] = 0;
        let b = encode(2, 2, &rgba, png::ColorType::Rgba, png::Compression::Fast);
        assert_ne!(pixel_hash(&a).unwrap(), pixel_hash(&b).unwrap());
    }

    fn header(w: u32, h: u32) -> Vec<u8> {
        let mut v = SIGNATURE.to_vec();
        v.extend_from_slice(&13u32.to_be_bytes());
        v.extend_from_slice(b"IHDR");
        v.extend_from_slice(&w.to_be_bytes());
        v.extend_from_slice(&h.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    #[test]
    fn header_limits() {
        assert_eq!(check_header(&header(16_384, 6_000)), Ok((16_384, 6_000)));
        assert!(matches!(
            check_header(&header(16_385, 1)),
            Err(ImageError::TooManyPixels { .. })
        ));
        assert!(matches!(
            check_header(&header(10_001, 10_000)),
            Err(ImageError::TooManyPixels { .. })
        ));
        assert!(matches!(
            check_header(&header(0, 10)),
            Err(ImageError::TooManyPixels { .. })
        ));
        assert_eq!(check_header(b"GIF89a"), Err(ImageError::BadHeader));
    }

    #[test]
    fn truncated_png_is_refused_not_panicking() {
        let rgba = vec![7u8; 4 * 8 * 8];
        let png = encode(8, 8, &rgba, png::ColorType::Rgba, png::Compression::Fast);
        for cut in 0..png.len() {
            let _ = pixel_hash(&png[..cut]);
        }
        assert!(pixel_hash(&png[..png.len() - 20]).is_err());
    }

    proptest::proptest! {
        #[test]
        fn random_bytes_never_panic(mut bytes in proptest::collection::vec(proptest::num::u8::ANY, 0..300)) {
            let _ = pixel_hash(&bytes);
            // Also with a valid header in front.
            let mut v = header(4, 4);
            v.append(&mut bytes);
            let _ = pixel_hash(&v);
        }
    }
}
