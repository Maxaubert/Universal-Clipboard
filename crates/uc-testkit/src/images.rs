//! Small PNG helpers for tests.

/// Encode RGBA8 pixels as a PNG.
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8], compression: png::Compression) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, width, height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(compression);
        let mut w = enc.write_header().expect("png header");
        w.write_image_data(rgba).expect("png data");
    }
    out
}

/// A test image whose pixels depend on `seed`.
pub fn image(width: u32, height: u32, seed: u8) -> Vec<u8> {
    let rgba: Vec<u8> = (0..width * height * 4)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect();
    encode_rgba(width, height, &rgba, png::Compression::Fast)
}

/// A PNG that does not compress, so it is about `4 * width * height` bytes.
pub fn big_image(width: u32, height: u32, seed: u8) -> Vec<u8> {
    let mut x = u32::from(seed) | 1;
    let rgba: Vec<u8> = (0..width * height * 4)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            x as u8
        })
        .collect();
    encode_rgba(width, height, &rgba, png::Compression::NoCompression)
}

/// Decode and encode again with other settings: same pixels, other bytes.
pub fn reencode(data: &[u8]) -> Vec<u8> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let Ok(mut r) = dec.read_info() else {
        return data.to_vec();
    };
    let Some(size) = r.output_buffer_size() else {
        return data.to_vec();
    };
    let mut buf = vec![0u8; size];
    let Ok(info) = r.next_frame(&mut buf) else {
        return data.to_vec();
    };
    if info.color_type != png::ColorType::Rgba {
        return data.to_vec();
    }
    let out = encode_rgba(
        info.width,
        info.height,
        &buf[..info.buffer_size()],
        png::Compression::High,
    );
    if out == data {
        encode_rgba(
            info.width,
            info.height,
            &buf[..info.buffer_size()],
            png::Compression::NoCompression,
        )
    } else {
        out
    }
}
