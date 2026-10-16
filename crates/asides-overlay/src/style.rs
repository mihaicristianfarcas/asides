//! How the overlay looks, shared by the windows (GPUI) and the images it exports (AppKit), after
//! PixelSnap: thin lines in the accent color, sizes in dark pills, blue guides, tinted areas.

use asides_core::inspect::{Annotation, PtRect};

/// Pills behind sizes and labels.
pub const LABEL: u32 = 0x1c1c1e;
pub const LABEL_ALPHA: f32 = 0.92;
pub const GUIDE: u32 = 0x0a84ff;
/// Markers on an element or a point.
pub const MARK_BLUE: u32 = 0x3b82f6;
/// Markers on an area.
pub const MARK_GREEN: u32 = 0x22c55e;
/// Fill of a held or dragged area, over the accent color.
pub const AREA_TINT: f32 = 0.14;
/// Arrows and sketches.
pub const DRAW_WIDTH: f32 = 3.0;
/// The UI's own color ("soft blue cloud"): the selected tool, the main buttons, chosen chips, the
/// menu's highlight, the caret. Measuring and marks keep their own, high-contrast colors.
pub const UI: u32 = 0xc2d6f6;
pub const UI_LIGHT: u32 = 0xd1dff6;
pub const UI_DEEP: u32 = 0x92b6f0;
/// Text and icons on the UI color.
pub const UI_INK: u32 = 0x16213a;
/// What a mark asks for, as the card's dots show it.
pub const INTENT_FIX: u32 = 0xff6b6b;
pub const INTENT_CHANGE: u32 = 0xf5b14c;
pub const INTENT_QUESTION: u32 = UI_DEEP;
pub const INTENT_APPROVE: u32 = 0x4cd28b;
/// The toolbar, cards and menus.
pub const PANEL: u32 = 0x1c1c1e;
pub const PANEL_ALPHA: f32 = 0.98;
pub const RECORDING: u32 = 0xff3b30;

pub fn intent_color(intent: asides_core::inspect::Intent) -> u32 {
    use asides_core::inspect::Intent;
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

/// A mark's color and the alpha of its fill: drawings in the accent color, areas green, elements
/// and points blue.
pub fn mark_color(a: &Annotation, accent: u32) -> (f64, f64, f64, f32) {
    let value = match (&a.shape, a.area) {
        (Some(_), _) => accent,
        (None, Some(_)) => MARK_GREEN,
        (None, None) => MARK_BLUE,
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
        let [tip, left, right] = arrow_head((0.0, 0.0), (100.0, 0.0));
        assert_eq!(tip, (100.0, 0.0));
        assert!((left.0 - 86.0).abs() < 1e-9 && left.1 > 0.0 && right.1 < 0.0);
    }
}
