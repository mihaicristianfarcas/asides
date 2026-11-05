//! The images that leave the overlay on Windows, as `export.rs` draws them with AppKit on a Mac: a
//! frozen screen with what was measured, annotated and drawn on it, cut to a part of it, as a PNG at
//! the screen's resolution. Drawn with tiny-skia, the labels in Windows' own typeface (Segoe UI),
//! looking like the overlay. Main thread.

#[cfg(windows)]
use std::borrow::Cow;

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use asides_core::inspect::{Annotation, Axis, Measurement, PtRect, Shape, Units, num};
use asides_core::measure::Frame;
use asides_core::overlay::{MARKER, Session};
use tiny_skia::{
    Color, FillRule, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, StrokeDash,
    Transform,
};

use crate::style;

/// The frozen screens, ready to draw.
pub struct Exporter {
    frames: Vec<Option<Pixmap>>,
    accent: (f64, f64, f64),
}

impl Exporter {
    pub fn new(frames: &[&Frame], accent: (f64, f64, f64)) -> Exporter {
        Exporter { frames: frames.iter().map(|f| pixmap(f)).collect(), accent }
    }

    /// `screen` with what's on it, cut to `crop` (points), as a PNG at the screen's resolution.
    pub fn render(&self, session: &Session, screen: usize, crop: PtRect) -> Option<Vec<u8>> {
        self.render_within(session, screen, crop, u32::MAX)
    }

    /// Like `render`, no larger than `max_side` pixels on its long side.
    pub fn render_within(&self, session: &Session, screen: usize, crop: PtRect, max_side: u32) -> Option<Vec<u8>> {
        let state = session.screens.get(screen)?;
        let source = self.frames.get(screen)?.as_ref()?;
        let long = crop.width.max(crop.height).max(1.0);
        let scale = state.screen.scale.min(max_side as f64 / long);
        let (w, h) = ((crop.width * scale).round().max(1.0) as u32, (crop.height * scale).round().max(1.0) as u32);
        let mut canvas = Canvas { pixmap: Pixmap::new(w, h)?, scale, crop, fonts: Fonts::load() };
        // The screen's pixels, then what's on it.
        let from_pixels = (scale / state.screen.scale) as f32;
        let place = Transform::from_translate((-crop.x * scale) as f32, (-crop.y * scale) as f32)
            .pre_scale(from_pixels, from_pixels);
        let paint = PixmapPaint { quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
        canvas.pixmap.draw_pixmap(0, 0, source.as_ref(), &paint, place, None);
        Painter { accent: self.accent }.annotations(&mut canvas, session, screen, source);
        canvas.pixmap.encode_png().ok()
    }
}

/// The longest side of an image added to an aside. Claude scales larger ones down to this before
/// reading them, so more pixels would only cost space, not detail the agent sees.
pub const ASIDE_MAX_SIDE: u32 = 1568;

/// The part of `screen` an image keeps: around what's on it (with `padding`), or all of it.
pub fn crop(session: &Session, screen: usize, padding: f64, whole: bool) -> PtRect {
    let state = &session.screens[screen];
    let full = PtRect::new(0.0, 0.0, state.screen.width, state.screen.height);
    if whole {
        return full;
    }
    match session.inspections().into_iter().find(|(i, _)| *i == screen) {
        Some((_, inspection)) => inspection.bounds(padding),
        None => full,
    }
}

/// A frame's pixels, ready to draw from.
fn pixmap(frame: &Frame) -> Option<Pixmap> {
    let len = (frame.width * frame.height * 4) as usize;
    let data = frame.rgba.get(..len)?.to_vec();
    Pixmap::from_vec(data, tiny_skia::IntSize::from_wh(frame.width, frame.height)?)
}

#[cfg(target_os = "linux")]
pub fn copy_png(png: &[u8]) {
    let dir = asides_core::paths::links_socket().parent().map(std::path::Path::to_path_buf).unwrap_or_default();
    let path = dir.join(format!("asides-copied-{}.png", std::process::id()));
    if std::fs::write(&path, png).is_ok() {
        crate::io::emit(&asides_core::helper::Event::Clipboard { text: None, png: Some(path) });
    }
}

#[cfg(target_os = "linux")]
pub fn copy_text(text: &str) {
    crate::io::emit(&asides_core::helper::Event::Clipboard { text: Some(text.to_string()), png: None });
}

#[cfg(windows)]
pub fn copy_png(png: &[u8]) {
    let Some((width, height, rgba)) = decode(png) else { return };
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_image(arboard::ImageData { width, height, bytes: Cow::Owned(rgba) });
    }
}

#[cfg(windows)]
pub fn copy_text(text: &str) {
    if let Ok(mut clipboard) = arboard::Clipboard::new() {
        let _ = clipboard.set_text(text.to_string());
    }
}

/// A PNG of ours (RGBA, 8 bits) as its pixels.
#[cfg(windows)]
fn decode(png: &[u8]) -> Option<(usize, usize, Vec<u8>)> {
    let pixmap = Pixmap::decode_png(png).ok()?;
    let (w, h) = (pixmap.width() as usize, pixmap.height() as usize);
    let rgba = pixmap.pixels().iter().flat_map(|p| {
        let c = p.demultiply();
        [c.red(), c.green(), c.blue(), c.alpha()]
    });
    Some((w, h, rgba.collect()))
}

/// The labels' typefaces: Segoe UI's semibold and bold, as Windows has them.
struct Fonts {
    semibold: Option<Vec<u8>>,
    bold: Option<Vec<u8>>,
}

impl Fonts {
    #[cfg(windows)]
    fn load() -> Fonts {
        let fonts =
            std::env::var_os("WINDIR").map_or_else(|| "C:\\Windows".into(), std::path::PathBuf::from).join("Fonts");
        let read = |names: &[&str]| names.iter().find_map(|n| std::fs::read(fonts.join(n)).ok());
        Fonts { semibold: read(&["seguisb.ttf", "segoeui.ttf"]), bold: read(&["segoeuib.ttf", "seguisb.ttf"]) }
    }

    /// On Linux: what fontconfig finds for the desktop's sans-serif.
    #[cfg(target_os = "linux")]
    fn load() -> Fonts {
        let find = |style: &str| -> Option<Vec<u8>> {
            let out = std::process::Command::new("fc-match")
                .args(["-f", "%{file}", &format!("sans-serif:{style}")])
                .output()
                .ok()?;
            std::fs::read(String::from_utf8_lossy(&out.stdout).trim()).ok()
        };
        Fonts { semibold: find("semibold").or_else(|| find("medium")), bold: find("bold") }
    }
}

/// Where a pass draws: the image, how many of its pixels a point is, and the part of the screen
/// it shows.
struct Canvas {
    pixmap: Pixmap,
    scale: f64,
    crop: PtRect,
    fonts: Fonts,
}

impl Canvas {
    /// The screen's points to the image's pixels.
    fn transform(&self) -> Transform {
        let s = self.scale as f32;
        Transform::from_scale(s, s).pre_translate(-self.crop.x as f32, -self.crop.y as f32)
    }

    fn fill(&mut self, path: &tiny_skia::Path, color: Color) {
        let paint = paint(color);
        let transform = self.transform();
        self.pixmap.fill_path(path, &paint, FillRule::Winding, transform, None);
    }

    fn stroke(&mut self, path: &tiny_skia::Path, color: Color, width: f64, dash: Option<&[f32]>, round: bool) {
        let mut stroke = Stroke { width: width as f32, ..Stroke::default() };
        if round {
            (stroke.line_cap, stroke.line_join) = (LineCap::Round, LineJoin::Round);
        }
        stroke.dash = dash.and_then(|d| StrokeDash::new(d.to_vec(), 0.0));
        let transform = self.transform();
        self.pixmap.stroke_path(path, &paint(color), &stroke, transform, None);
    }

    fn line(&mut self, a: (f64, f64), b: (f64, f64), color: Color, width: f64) {
        let mut path = PathBuilder::new();
        path.move_to(a.0 as f32, a.1 as f32);
        path.line_to(b.0 as f32, b.1 as f32);
        if let Some(path) = path.finish() {
            self.stroke(&path, color, width, None, false);
        }
    }

    /// The width of `text` in points, at `size`.
    fn text_width(&self, text: &str, size: f64, bold: bool) -> f64 {
        let Some(font) = self.font(bold) else { return text.chars().count() as f64 * size * 0.55 };
        let scaled = font.as_scaled(px_scale(&font, size));
        text.chars().map(|c| scaled.h_advance(font.glyph_id(c)) as f64).sum()
    }

    fn font(&self, bold: bool) -> Option<FontRef<'_>> {
        let bytes = if bold { self.fonts.bold.as_deref() } else { self.fonts.semibold.as_deref() }?;
        FontRef::try_from_slice(bytes).ok()
    }

    /// `text` with its top-left at `at` (points), `size` points tall, in `color`.
    fn text(&mut self, text: &str, at: (f64, f64), size: f64, bold: bool, color: Color) {
        let s = self.scale;
        let (ox, oy) = ((at.0 - self.crop.x) * s, (at.1 - self.crop.y) * s);
        let Some(bytes) = (if bold { self.fonts.bold.clone() } else { self.fonts.semibold.clone() }) else { return };
        let Ok(font) = FontRef::try_from_slice(&bytes) else { return };
        let scale = px_scale(&font, size * s);
        let scaled = font.as_scaled(scale);
        let baseline = oy as f32 + scaled.ascent();
        let mut x = ox as f32;
        let (w, h) = (self.pixmap.width() as i32, self.pixmap.height() as i32);
        let pixels = self.pixmap.pixels_mut();
        let (r, g, b, a) = (color.red(), color.green(), color.blue(), color.alpha());
        for c in text.chars() {
            let glyph = font.glyph_id(c).with_scale_and_position(scale, point(x, baseline));
            x += scaled.h_advance(font.glyph_id(c));
            let Some(outline) = font.outline_glyph(glyph) else { continue };
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
                if px < 0 || py < 0 || px >= w || py >= h {
                    return;
                }
                let alpha = (coverage * a).clamp(0.0, 1.0);
                let pixel = &mut pixels[(py * w + px) as usize];
                // Over what's there (premultiplied).
                let blend = |src: f32, dst: u8| ((src * alpha * 255.0) + dst as f32 * (1.0 - alpha)).round() as u8;
                let out =
                    [blend(r, pixel.red()), blend(g, pixel.green()), blend(b, pixel.blue()), blend(1.0, pixel.alpha())];
                if let Some(p) = tiny_skia::PremultipliedColorU8::from_rgba(
                    out[0],
                    out[1],
                    out[2],
                    out[3].max(out[0]).max(out[1]).max(out[2]),
                ) {
                    *pixel = p;
                }
            });
        }
    }
}

/// A font's scale for text `size` pixels to the em, as macOS sizes its fonts.
fn px_scale(font: &FontRef<'_>, size: f64) -> PxScale {
    let em = font.units_per_em().unwrap_or(2048.0);
    PxScale::from((size as f32) * font.height_unscaled() / em)
}

fn paint(color: Color) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color(color);
    paint.anti_alias = true;
    paint
}

fn color((r, g, b): (f64, f64, f64), a: f64) -> Color {
    Color::from_rgba(r as f32, g as f32, b as f32, a as f32).unwrap_or(Color::BLACK)
}

fn hex(value: u32, a: f64) -> Color {
    let c = |shift: u32| ((value >> shift) & 0xff) as f64 / 255.0;
    color((c(16), c(8), c(0)), a)
}

fn rect_path(r: PtRect) -> Option<tiny_skia::Path> {
    tiny_skia::Rect::from_xywh(r.x as f32, r.y as f32, r.width as f32, r.height as f32).map(PathBuilder::from_rect)
}

/// A rectangle with corners rounded by `radius`.
fn rounded(r: PtRect, radius: f64) -> Option<tiny_skia::Path> {
    let (x, y, w, h) = (r.x as f32, r.y as f32, r.width as f32, r.height as f32);
    let k = radius.min(r.width / 2.0).min(r.height / 2.0) as f32;
    // How far along a quarter circle's tangent its cubic's control points sit.
    let c = k * 0.552_284_8;
    let mut p = PathBuilder::new();
    p.move_to(x + k, y);
    p.line_to(x + w - k, y);
    p.cubic_to(x + w - k + c, y, x + w, y + k - c, x + w, y + k);
    p.line_to(x + w, y + h - k);
    p.cubic_to(x + w, y + h - k + c, x + w - k + c, y + h, x + w - k, y + h);
    p.line_to(x + k, y + h);
    p.cubic_to(x + k - c, y + h, x, y + h - k + c, x, y + h - k);
    p.line_to(x, y + k);
    p.cubic_to(x, y + k - c, x + k - c, y, x + k, y);
    p.close();
    p.finish()
}

fn circle(center: (f64, f64), radius: f64) -> Option<tiny_skia::Path> {
    PathBuilder::from_circle(center.0 as f32, center.1 as f32, radius as f32)
}

/// The size labels are drawn at, as the overlay's.
const LABEL_SIZE: f64 = 11.0;

/// Colors for one drawing pass (the overlay's, see `style`).
struct Painter {
    accent: (f64, f64, f64),
}

impl Painter {
    /// A dark pill with white `text`, centered on `at`.
    fn label(&self, canvas: &mut Canvas, text: &str, at: (f64, f64)) {
        let width = canvas.text_width(text, LABEL_SIZE, false);
        let height = LABEL_SIZE * 1.3;
        let rect = PtRect::new(at.0 - width / 2.0 - 5.0, at.1 - height / 2.0 - 2.0, width + 10.0, height + 4.0);
        if let Some(path) = rounded(rect, 4.0) {
            canvas.fill(&path, hex(style::LABEL, style::LABEL_ALPHA as f64));
        }
        canvas.text(text, (rect.x + 5.0, rect.y + 2.0), LABEL_SIZE, false, Color::WHITE);
    }

    fn label_width(&self, canvas: &Canvas, text: &str) -> f64 {
        canvas.text_width(text, LABEL_SIZE, false) + 10.0
    }

    fn annotations(&self, canvas: &mut Canvas, session: &Session, screen: usize, source: &Pixmap) {
        let state = &session.screens[screen];
        let (width, height) = (state.screen.width, state.screen.height);
        let (scale, units) = (state.screen.scale, session.units);
        let n = |v: f64| num(v, scale, units);

        // Guides (blue, a pixel wide), with the gaps between them at the screen's edge.
        for axis in [Axis::X, Axis::Y] {
            let mut positions: Vec<f64> = state.guides.iter().filter(|g| g.axis == axis).map(|g| g.position).collect();
            if positions.is_empty() {
                continue;
            }
            positions.sort_by(f64::total_cmp);
            let px = 1.0 / scale.max(1.0);
            for &p in &positions {
                match axis {
                    Axis::X => canvas.line((p + px / 2.0, 0.0), (p + px / 2.0, height), hex(style::GUIDE, 1.0), px),
                    Axis::Y => canvas.line((0.0, p + px / 2.0), (width, p + px / 2.0), hex(style::GUIDE, 1.0), px),
                }
            }
            let limit = if axis == Axis::X { width } else { height };
            let mut stops = vec![0.0];
            stops.extend(positions);
            stops.push(limit);
            for pair in stops.windows(2) {
                let gap = pair[1] - pair[0];
                if gap < 16.0 {
                    continue;
                }
                let mid = (pair[0] + pair[1]) / 2.0;
                self.label(canvas, &n(gap), if axis == Axis::X { (mid, 14.0) } else { (28.0, mid) });
            }
        }

        for m in &state.held {
            self.measurement(canvas, m, scale, units, width, height);
        }
        for a in &state.annotations {
            self.moved(canvas, a, source, state.screen.scale);
        }
        for a in &state.annotations {
            self.annotation(canvas, a);
        }
    }

    fn measurement(&self, canvas: &mut Canvas, m: &Measurement, scale: f64, units: Units, width: f64, height: f64) {
        let n = |v: f64| num(v, scale, units);
        let ink = color(self.accent, 1.0);
        match m {
            Measurement::Distance { x, y, across, down } => {
                let (cx, cy) = (x + 0.5 / scale, y + 0.5 / scale);
                if !across.is_empty() {
                    canvas.line((across.start, cy), (across.end, cy), ink, 1.0);
                    for (edge, open) in [(across.start, across.open_start), (across.end, across.open_end)] {
                        if !open {
                            canvas.line((edge, cy - 4.0), (edge, cy + 4.0), ink, 1.0);
                        }
                    }
                    let ly = if cy < 30.0 { cy + 14.0 } else { cy - 14.0 };
                    self.label(canvas, &n(across.len()), ((across.start + across.end) / 2.0, ly));
                }
                if !down.is_empty() {
                    canvas.line((cx, down.start), (cx, down.end), ink, 1.0);
                    for (edge, open) in [(down.start, down.open_start), (down.end, down.open_end)] {
                        if !open {
                            canvas.line((cx - 4.0, edge), (cx + 4.0, edge), ink, 1.0);
                        }
                    }
                    let text = n(down.len());
                    let w = self.label_width(canvas, &text);
                    let lx = if cx + w + 16.0 > width { cx - w / 2.0 - 8.0 } else { cx + w / 2.0 + 8.0 };
                    self.label(canvas, &text, (lx, (down.start + down.end) / 2.0));
                }
            }
            Measurement::Area { rect, .. } => {
                if let Some(path) = rect_path(*rect) {
                    canvas.fill(&path, color(self.accent, style::AREA_TINT as f64));
                    canvas.stroke(&path, ink, 1.0, None, false);
                }
                let text = format!("{} × {}", n(rect.width), n(rect.height));
                let w = self.label_width(canvas, &text);
                self.label(canvas, &text, style::area_label(*rect, w, height));
            }
        }
    }

    fn annotation(&self, canvas: &mut Canvas, a: &Annotation) {
        let (r, g, b, alpha) = style::annotation_color(a, a.intent, self.accent_hex());
        let ink = color((r, g, b), 1.0);
        match (&a.shape, a.area) {
            (Some(Shape::Sketch { points }), _) => {
                let mut path = PathBuilder::new();
                for (i, p) in points.iter().enumerate() {
                    if i == 0 { path.move_to(p.0 as f32, p.1 as f32) } else { path.line_to(p.0 as f32, p.1 as f32) }
                }
                if let Some(path) = path.finish() {
                    canvas.stroke(&path, ink, style::DRAW_WIDTH as f64, None, true);
                }
            }
            (Some(Shape::Arrow { from, to }), _) => self.arrow(canvas, *from, *to, ink),
            (None, Some(area)) => {
                if let Some(path) = rect_path(area) {
                    canvas.fill(&path, color((r, g, b), alpha as f64));
                    canvas.stroke(&path, ink, 1.5, Some(&[6.0, 4.0]), false);
                }
            }
            (None, None) => {
                if let Some(path) = a.element.as_ref().and_then(|e| e.frame).and_then(rect_path) {
                    canvas.stroke(&path, color((r, g, b), 0.9), 1.5, None, false);
                }
            }
        }
        if let Some(path) = circle(a.at, MARKER / 2.0) {
            canvas.fill(&path, ink);
            canvas.stroke(&path, Color::WHITE, 2.0, None, false);
        }
        let text = a.number.to_string();
        let width = canvas.text_width(&text, LABEL_SIZE, true);
        let height = LABEL_SIZE * 1.3;
        canvas.text(&text, (a.at.0 - width / 2.0, a.at.1 - height / 2.0), LABEL_SIZE, true, Color::WHITE);
    }

    /// An arrow's shaft and head.
    fn arrow(&self, canvas: &mut Canvas, from: (f64, f64), to: (f64, f64), ink: Color) {
        let [tip, left, right] = style::arrow_head(from, to);
        canvas.line(from, ((left.0 + right.0) / 2.0, (left.1 + right.1) / 2.0), ink, style::DRAW_WIDTH as f64);
        let mut head = PathBuilder::new();
        head.move_to(tip.0 as f32, tip.1 as f32);
        head.line_to(left.0 as f32, left.1 as f32);
        head.line_to(right.0 as f32, right.1 as f32);
        head.close();
        if let Some(head) = head.finish() {
            canvas.fill(&head, ink);
        }
    }

    /// A pin moved by its number: a copy of what it's on (from `source`, `scale` pixels a point)
    /// where it goes, outlined, and an arrow there.
    fn moved(&self, canvas: &mut Canvas, a: &Annotation, source: &Pixmap, scale: f64) {
        let Some(dest) = a.moved_region() else { return };
        let (r, g, b, _) = style::annotation_color(a, a.intent, self.accent_hex());
        let ink = color((r, g, b), 1.0);
        let from = a.region().unwrap_or(PtRect::new(a.at.0, a.at.1, 0.0, 0.0));
        if from.width >= 1.0 && from.height >= 1.0 {
            let (sx, sy) = ((from.x * scale).round().max(0.0) as u32, (from.y * scale).round().max(0.0) as u32);
            let (sw, sh) =
                ((from.width * scale).round().max(1.0) as u32, (from.height * scale).round().max(1.0) as u32);
            if let Some(copy) =
                tiny_skia::IntRect::from_xywh(sx as i32, sy as i32, sw, sh).and_then(|r| source.clone_rect(r))
            {
                let (fx, fy) =
                    ((dest.width * canvas.scale) as f32 / sw as f32, (dest.height * canvas.scale) as f32 / sh as f32);
                let at = Transform::from_translate(
                    ((dest.x - canvas.crop.x) * canvas.scale) as f32,
                    ((dest.y - canvas.crop.y) * canvas.scale) as f32,
                )
                .pre_scale(fx, fy);
                let paint = PixmapPaint { opacity: 0.92, quality: FilterQuality::Bicubic, ..PixmapPaint::default() };
                canvas.pixmap.draw_pixmap(0, 0, copy.as_ref(), &paint, at, None);
            }
            if let Some(path) = rect_path(dest) {
                canvas.stroke(&path, ink, 1.5, None, false);
            }
        }
        let (start, end) = style::move_arrow(from, dest);
        self.arrow(canvas, start, end, ink);
    }

    fn accent_hex(&self) -> u32 {
        let c = |v: f64| ((v * 255.0).round() as u32).min(255);
        (c(self.accent.0) << 16) | (c(self.accent.1) << 8) | c(self.accent.2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_rectangles_and_draws_text() {
        let frame = Frame::new(40, 20, vec![255; 40 * 20 * 4]).unwrap();
        let exporter = Exporter::new(&[&frame], (0.23, 0.51, 0.96));
        assert_eq!(exporter.frames.len(), 1);
        assert!(rounded(PtRect::new(0.0, 0.0, 30.0, 10.0), 4.0).is_some());
        let mut canvas = Canvas {
            pixmap: Pixmap::new(80, 40).unwrap(),
            scale: 2.0,
            crop: PtRect::new(0.0, 0.0, 40.0, 20.0),
            fonts: Fonts::load(),
        };
        Painter { accent: (0.23, 0.51, 0.96) }.label(&mut canvas, "12 px", (20.0, 10.0));
        assert!(canvas.pixmap.pixels().iter().any(|p| p.alpha() > 0), "the label is drawn");
    }
}
