//! How the overlay looks, shared by the windows (GPUI) and the images it exports (AppKit), after
//! PixelSnap: thin lines in the accent color, sizes in dark pills, blue guides, tinted areas.

use asides_core::inspect::{Annotation, Intent, PtRect};

/// Pills behind sizes and labels.
pub const LABEL: u32 = 0x1c1c1e;
pub const LABEL_ALPHA: f32 = 0.92;
pub const GUIDE: u32 = 0x0a84ff;
/// Markers on an element or a point.
pub const ANNOTATION_BLUE: u32 = 0x3b82f6;
/// Markers on an area.
pub const ANNOTATION_GREEN: u32 = 0x22c55e;
/// Fill of a held or dragged area, over the accent color.
pub const AREA_TINT: f32 = 0.14;
/// Arrows and sketches.
pub const DRAW_WIDTH: f32 = 3.0;
/// The UI's own color ("soft blue cloud"): the main action (Send) and the caret; what's selected is a
/// glass pill instead. Measuring and annotations keep their own, high-contrast colors.
pub const UI: u32 = 0xc2d6f6;
pub const UI_LIGHT: u32 = 0xd1dff6;
pub const UI_DEEP: u32 = 0x92b6f0;
/// Text and icons on the UI color.
pub const UI_INK: u32 = 0x16213a;
/// What an annotation asks for: its card's dots, and its marker once chosen (apart from the
/// kinds' blue and green).
pub const INTENT_FIX: u32 = 0xef4444;
pub const INTENT_CHANGE: u32 = 0xf59e0b;
pub const INTENT_QUESTION: u32 = 0x8b5cf6;
pub const INTENT_APPROVE: u32 = 0x14b8a6;
/// The overlay's surfaces (the pill and its list, tips, cards, menus) are the asides window's glass,
/// in the system's appearance (`controls::Theme`'s `sidebar`). Help's clips caption what's said on
/// a dark glass of this color.
pub const GLASS: u32 = 0x1c1b1a;
pub const RECORDING: u32 = 0xff3b30;
/// The pill's microphone while it listens: a softer red.
pub const RECORDING_SOFT: u32 = 0xec6a5e;

pub fn intent_color(intent: Intent) -> u32 {
    match intent {
        Intent::Fix => INTENT_FIX,
        Intent::Change => INTENT_CHANGE,
        Intent::Question => INTENT_QUESTION,
        Intent::Approve => INTENT_APPROVE,
    }
}

/// `#RRGGBB` as 0..1 channels; the default accent when it doesn't parse.
pub fn parse_color(hex: &str) -> (f64, f64, f64) {
    let hex = hex.trim().trim_start_matches('#');
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok().map(|v| v as f64 / 255.0);
    match (hex.len(), channel(0), channel(2), channel(4)) {
        (6, Some(r), Some(g), Some(b)) => (r, g, b),
        _ => (1.0, 0.176, 0.333),
    }
}

pub fn rgb_hex((r, g, b): (f64, f64, f64)) -> u32 {
    let c = |v: f64| ((v * 255.0).round() as u32).min(255);
    (c(r) << 16) | (c(g) << 8) | c(b)
}

fn channels(value: u32) -> (f64, f64, f64) {
    let c = |shift: u32| ((value >> shift) & 0xff) as f64 / 255.0;
    (c(16), c(8), c(0))
}

/// An annotation's color and the alpha of its fill: its `intent`'s, else drawings in the accent
/// color, areas green, elements and points blue.
pub fn annotation_color(a: &Annotation, intent: Option<Intent>, accent: u32) -> (f64, f64, f64, f32) {
    let value = match (intent, &a.shape, a.area) {
        (Some(intent), ..) => intent_color(intent),
        (None, Some(_), _) => accent,
        (None, None, Some(_)) => ANNOTATION_GREEN,
        (None, None, None) => ANNOTATION_BLUE,
    };
    let (r, g, b) = channels(value);
    (r, g, b, 0.1)
}

/// Where an area's size goes: centered in it, or below it (above, near the screen's bottom) when
/// it doesn't fit inside.
pub fn area_label(rect: PtRect, label_width: f64, screen_height: f64) -> (f64, f64) {
    let x = rect.x + rect.width / 2.0;
    if rect.width >= label_width + 12.0 && rect.height >= 30.0 {
        return (x, rect.y + rect.height / 2.0);
    }
    if rect.bottom() + 30.0 > screen_height { (x, rect.y - 14.0) } else { (x, rect.bottom() + 14.0) }
}

/// An arrow's head: its tip and the two corners of its base.
pub fn arrow_head(from: (f64, f64), to: (f64, f64)) -> [(f64, f64); 3] {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let length = dx.hypot(dy).max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let size = (length * 0.5).min(14.0);
    let base = (to.0 - ux * size, to.1 - uy * size);
    let half = size * 0.55;
    [to, (base.0 - uy * half, base.1 + ux * half), (base.0 + uy * half, base.1 - ux * half)]
}

/// A move's arrow: from where `from`'s edge faces `to` to where `to`'s edge faces back (middle to
/// middle when they overlap).
pub fn move_arrow(from: PtRect, to: PtRect) -> ((f64, f64), (f64, f64)) {
    let middle = |r: PtRect| (r.x + r.width / 2.0, r.y + r.height / 2.0);
    let (a, b) = (middle(from), middle(to));
    // Where the line from `r`'s middle toward `p` leaves `r`.
    let edge = |r: PtRect, (cx, cy): (f64, f64), (px, py): (f64, f64)| {
        let (dx, dy) = (px - cx, py - cy);
        let t = [(r.width / 2.0, dx), (r.height / 2.0, dy)]
            .into_iter()
            .filter(|(_, d)| d.abs() > f64::EPSILON)
            .map(|(half, d)| half / d.abs())
            .fold(f64::INFINITY, f64::min)
            .min(1.0);
        (cx + dx * t, cy + dy * t)
    };
    let (start, end) = (edge(from, a, b), edge(to, b, a));
    let ahead = (end.0 - start.0) * (b.0 - a.0) + (end.1 - start.1) * (b.1 - a.1) > 0.0;
    if ahead { (start, end) } else { (a, b) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_labels_and_arrows() {
        assert_eq!(parse_color("#FF0000"), (1.0, 0.0, 0.0));
        assert_eq!(parse_color("nope"), (1.0, 0.176, 0.333));
        assert_eq!(rgb_hex((1.0, 0.0, 0.5)), 0xff0080);
        let small = PtRect::new(10.0, 10.0, 20.0, 10.0);
        assert_eq!(area_label(small, 50.0, 500.0), (20.0, 34.0));
        assert_eq!(area_label(small, 50.0, 40.0), (20.0, -4.0));
        let big = PtRect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(area_label(big, 50.0, 500.0), (100.0, 50.0));
        // A move's arrow, edge to edge; middle to middle when they overlap.
        let (from, to) = (PtRect::new(0.0, 0.0, 20.0, 10.0), PtRect::new(100.0, 0.0, 20.0, 10.0));
        assert_eq!(move_arrow(from, to), ((20.0, 5.0), (100.0, 5.0)));
        assert_eq!(move_arrow(from, PtRect::new(5.0, 2.0, 20.0, 10.0)), ((10.0, 5.0), (15.0, 7.0)));
        let [tip, left, right] = arrow_head((0.0, 0.0), (100.0, 0.0));
        assert_eq!(tip, (100.0, 0.0));
        assert!((left.0 - 86.0).abs() < 1e-9 && left.1 > 0.0 && right.1 < 0.0);
    }
}
