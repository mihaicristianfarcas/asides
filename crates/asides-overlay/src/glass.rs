//! Glass on a frozen screen, after Diri's: what's behind a surface, blurred, under a translucent
//! tint. The pill's window is glass by macOS (`WindowBackgroundAppearance::Blurred`, kept to its
//! shapes by `native::set_glass`). What's drawn on a frozen screen (the tips, the keys, cards,
//! menus) has a still picture behind it: the overlay blurs a small copy of it once ([`Backdrop`])
//! and draws the part under each surface ([`frozen`]).

use std::sync::Arc;

use asides_core::measure::Frame;
use gpui::{Bounds, Corners, Entity, IntoElement, Pixels, RenderImage, Styled, canvas, px};

use crate::ui::Overlay;

/// The parts of the frozen screens' copies drawn under surfaces (by screen and part), as images.
pub type Pieces = Vec<((usize, [u32; 4]), Arc<RenderImage>)>;

/// Screen pixels per pixel of the blurred copy (at 2×, 8 points).
const STEP: u32 = 16;
/// The box blur's radius, in the copy's pixels, and how many times it runs (three are close to a
/// Gaussian): about 20 points, as macOS blurs behind a window.
const RADIUS: usize = 2;
const PASSES: usize = 3;

/// A frozen screen, small and blurred, as BGRA.
pub struct Backdrop {
    width: u32,
    height: u32,
    bgra: Vec<u8>,
}

impl Backdrop {
    pub fn of(frame: &Frame) -> Backdrop {
        let (width, height) = (frame.width.div_ceil(STEP).max(1), frame.height.div_ceil(STEP).max(1));
        // Each pixel: the average of four samples of its block of the screen.
        let mut rgb = vec![[0f32; 3]; (width * height) as usize];
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0f32; 3];
                for (dx, dy) in [(1, 1), (3, 1), (1, 3), (3, 3)] {
                    let sx = (x * STEP + dx * STEP / 4).min(frame.width - 1);
                    let sy = (y * STEP + dy * STEP / 4).min(frame.height - 1);
                    let i = ((sy * frame.width + sx) * 4) as usize;
                    for (c, s) in sum.iter_mut().enumerate() {
                        *s += frame.rgba[i + c] as f32;
                    }
                }
                rgb[(y * width + x) as usize] = sum.map(|s| s / 4.0);
            }
        }
        for _ in 0..PASSES {
            rgb = blur(&rgb, width as usize, height as usize, true);
            rgb = blur(&rgb, width as usize, height as usize, false);
        }
        let bgra = rgb.iter().flat_map(|[r, g, b]| [*b as u8, *g as u8, *r as u8, 255]).collect();
        Backdrop { width, height, bgra }
    }

    /// The copy's pixels covering `rect` (screen pixels: left, top, right, bottom), the same for
    /// the same pixels: the image of them is kept while the surface stays put.
    pub fn cover(&self, rect: [f64; 4]) -> [u32; 4] {
        let to = |v: f64, limit: u32, up: bool| {
            let v = v / STEP as f64;
            (if up { v.ceil() } else { v.floor() }).clamp(0.0, limit as f64) as u32
        };
        let (x0, y0) = (to(rect[0], self.width - 1, false), to(rect[1], self.height - 1, false));
        let (x1, y1) = (to(rect[2], self.width, true).max(x0 + 1), to(rect[3], self.height, true).max(y0 + 1));
        [x0, y0, x1, y1]
    }

    /// Those pixels (see [`Backdrop::cover`]) as an image.
    pub fn crop(&self, [x0, y0, x1, y1]: [u32; 4]) -> Arc<RenderImage> {
        let mut pixels = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 4) as usize);
        for y in y0..y1 {
            let row = ((y * self.width + x0) * 4) as usize;
            pixels.extend_from_slice(&self.bgra[row..row + ((x1 - x0) * 4) as usize]);
        }
        let buffer =
            image::RgbaImage::from_raw(x1 - x0, y1 - y0, pixels).unwrap_or_else(|| image::RgbaImage::new(1, 1));
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
    }
}

/// One pass of a box blur, across or down (edges repeat).
fn blur(src: &[[f32; 3]], width: usize, height: usize, across: bool) -> Vec<[f32; 3]> {
    let (lines, length) = if across { (height, width) } else { (width, height) };
    let at = |line: usize, i: usize| if across { line * width + i } else { i * width + line };
    let mut out = vec![[0f32; 3]; src.len()];
    let n = (2 * RADIUS + 1) as f32;
    for line in 0..lines {
        for i in 0..length {
            let mut sum = [0f32; 3];
            for k in 0..=2 * RADIUS {
                let j = (i + k).saturating_sub(RADIUS).min(length - 1);
                let p = src[at(line, j)];
                for c in 0..3 {
                    sum[c] += p[c];
                }
            }
            out[at(line, i)] = sum.map(|s| s / n);
        }
    }
    out
}

/// The blurred frozen screen under a surface (its bounds in a window `offset` points into screen
/// `screen`), with its corners rounded by `radius`.
pub fn frozen(overlay: &Entity<Overlay>, screen: usize, offset: (f64, f64), radius: f32) -> impl IntoElement {
    let overlay = overlay.clone();
    canvas(
        |_, _, _| (),
        move |bounds: Bounds<Pixels>, _, window, cx| {
            let image = overlay.update(cx, |o, _| o.glass_under(screen, offset, bounds));
            if let Some(image) = image {
                let _ = window.paint_image(bounds, Corners::all(px(radius)), image, 0, false);
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blurs_a_small_copy() {
        let mut frame = Frame::filled(320, 160, 0xffffff);
        frame.fill(asides_core::measure::Rect::new(0, 0, 160, 160), 0x000000);
        let b = Backdrop::of(&frame);
        assert_eq!((b.width, b.height), (20, 10));
        let blue = |x: u32| b.bgra[((5 * b.width + x) * 4) as usize];
        // Black on the left, white on the right, a ramp between.
        assert!(blue(0) < 10 && blue(19) > 245, "{} {}", blue(0), blue(19));
        assert!(blue(9) > 40 && blue(9) < 215, "{}", blue(9));
        assert_eq!(b.cover([20.0, 20.0, 100.0, 60.0]), [1, 1, 7, 4]);
    }
}
