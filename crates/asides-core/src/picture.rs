//! Pictures made from frames without drawing anything: a part of one, a smaller copy, a PNG.
//! What `look_again` shows an agent is cut and encoded here; the images asides draws on (pins,
//! measurements) are the overlay's (`export.rs`).

use crate::measure::{Frame, Rect};

/// The part of `frame` in `rect` (clipped to it); `None` when nothing of it is inside.
pub fn crop(frame: &Frame, rect: Rect) -> Option<Frame> {
    let rect = rect.clamp_to(frame.width, frame.height);
    if rect.is_empty() {
        return None;
    }
    let (width, row) = (rect.width as usize * 4, frame.width as usize * 4);
    let mut rgba = Vec::with_capacity(width * rect.height as usize);
    for y in rect.y..rect.bottom() {
        let start = y as usize * row + rect.x as usize * 4;
        rgba.extend_from_slice(&frame.rgba[start..start + width]);
    }
    Frame::new(rect.width, rect.height, rgba)
}

/// `frame` no larger than `max_side` on its longer side: itself, or a smaller copy, each pixel the
/// mean of those it covers (a hairline fades instead of vanishing, as it would picking pixels).
pub fn within(frame: &Frame, max_side: u32) -> Frame {
    let longer = frame.width.max(frame.height);
    if longer <= max_side || max_side == 0 {
        return frame.clone();
    }
    let size = |side: u32| ((side as u64 * max_side as u64).div_ceil(longer as u64) as u32).max(1);
    scaled(frame, size(frame.width), size(frame.height))
}

/// `frame` at `width` × `height`, smaller than it is: each pixel the mean of those it covers.
pub fn scaled(frame: &Frame, width: u32, height: u32) -> Frame {
    let (width, height) = (width.clamp(1, frame.width.max(1)), height.clamp(1, frame.height.max(1)));
    let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
    // The source pixels a target pixel covers, along one axis: at least one.
    let span = |i: u32, from: u32, to: u32| {
        let start = (i as u64 * from as u64 / to as u64) as u32;
        let end = (((i + 1) as u64 * from as u64).div_ceil(to as u64) as u32).clamp(start + 1, from);
        start..end
    };
    for y in 0..height {
        let rows = span(y, frame.height, height);
        for x in 0..width {
            let columns = span(x, frame.width, width);
            let (mut sum, mut count) = ([0u32; 3], 0u32);
            for sy in rows.clone() {
                let row = sy as usize * frame.width as usize * 4;
                for sx in columns.clone() {
                    let p = &frame.rgba[row + sx as usize * 4..row + sx as usize * 4 + 3];
                    sum = [sum[0] + p[0] as u32, sum[1] + p[1] as u32, sum[2] + p[2] as u32];
                    count += 1;
                }
            }
            let mean = |s: u32| ((s + count / 2) / count.max(1)) as u8;
            rgba.extend_from_slice(&[mean(sum[0]), mean(sum[1]), mean(sum[2]), 255]);
        }
    }
    Frame { width, height, rgba }
}

/// `frame` as a PNG (RGB: a frame has no transparency).
pub fn png(frame: &Frame) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, frame.width, frame.height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let rgb: Vec<u8> = frame.rgba.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b]).collect();
    // Into memory: nothing fails but running out of it.
    if let Ok(mut writer) = encoder.write_header() {
        let _ = writer.write_image_data(&rgb);
    }
    bytes
}

/// A PNG's pixels.
pub fn from_png(bytes: &[u8]) -> Option<Frame> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b, 255]).collect(),
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        png::ColorType::Grayscale => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => pixels.as_chunks::<2>().0.iter().flat_map(|&[g, _]| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Frame::new(info.width, info.height, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_scales_and_encodes() {
        // White, a red 4 × 2 block at (2, 1).
        let mut frame = Frame::filled(8, 4, 0xffffff);
        frame.fill(Rect::new(2, 1, 4, 2), 0xff0000);
        let block = crop(&frame, Rect::new(2, 1, 4, 2)).unwrap();
        assert_eq!((block.width, block.height, block.rgb(0, 0), block.rgb(3, 1)), (4, 2, 0xff0000, 0xff0000));
        // Past the frame's edge it's clipped; outside it, there's nothing.
        let corner = crop(&frame, Rect::new(6, 3, 10, 10)).unwrap();
        assert_eq!((corner.width, corner.height), (2, 1));
        assert_eq!(crop(&frame, Rect::new(8, 4, 2, 2)), None);

        // Half the size: each pixel is the mean of the four it covers.
        let half = within(&frame, 4);
        assert_eq!((half.width, half.height), (4, 2));
        assert_eq!((half.rgb(0, 0), half.rgb(1, 0), half.rgb(1, 1)), (0xffffff, 0xff8080, 0xff8080));
        // Small enough already: as it is. An odd size rounds up, and every pixel is covered.
        assert_eq!(within(&frame, 8), frame);
        let third = within(&Frame::filled(7, 3, 0x336699), 3);
        assert_eq!((third.width, third.height, third.rgb(2, 1)), (3, 2, 0x336699));

        let again = from_png(&png(&frame)).unwrap();
        assert_eq!(again, frame);
        assert!(png(&frame).starts_with(&[0x89, b'P', b'N', b'G']));
    }
}
