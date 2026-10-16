//! The images that leave the overlay: a frozen screen with what was measured, marked and drawn on
//! it, cut to a part of it, as a PNG at the screen's resolution (for the aside, a screenshot, the
//! clipboard). Drawn with AppKit into a bitmap, looking like the overlay. Main thread.

use asides_core::inspect::{Annotation, Axis, Measurement, PtRect, Shape, num};
use asides_core::measure::Frame;
use asides_core::overlay::{MARKER, Session};
use objc2::AnyThread;
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_app_kit::{
    NSBezierPath, NSBitmapImageFileType, NSBitmapImageRep, NSColor, NSColorSpace, NSCompositingOperation,
    NSDeviceRGBColorSpace, NSFont, NSFontAttributeName, NSFontWeightBold, NSFontWeightSemibold,
    NSForegroundColorAttributeName, NSGraphicsContext, NSLineCapStyle, NSLineJoinStyle, NSPasteboard,
    NSPasteboardTypePNG, NSPasteboardTypeString, NSStringDrawing,
};
use objc2_core_graphics::CGContext;
use objc2_foundation::{NSAttributedStringKey, NSData, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::style;

/// The frozen screens, ready to draw.
pub struct Exporter {
    reps: Vec<Option<Retained<NSBitmapImageRep>>>,
    accent: (f64, f64, f64),
}

impl Exporter {
    pub fn new(frames: &[&Frame], accent: (f64, f64, f64)) -> Exporter {
        Exporter { reps: frames.iter().map(|f| bitmap(f)).collect(), accent }
    }

    /// `screen` with what's on it, cut to `crop` (points), as a PNG.
    pub fn render(&self, session: &Session, screen: usize, crop: PtRect) -> Option<Vec<u8>> {
        let state = session.screens.get(screen)?;
        let source = self.reps.get(screen)?.as_ref()?;
        let scale = state.screen.scale;
        let (w, h) = ((crop.width * scale).round().max(1.0) as isize, (crop.height * scale).round().max(1.0) as isize);
        let rep = new_rep(w, h)?;
        rep.setSize(NSSize::new(crop.width, crop.height));
        let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
        let cg = context.CGContext();
        // y down, origin at the crop's top-left corner, in the screen's points.
        CGContext::translate_ctm(Some(&cg), 0.0, crop.height);
        CGContext::scale_ctm(Some(&cg), 1.0, -1.0);
        CGContext::translate_ctm(Some(&cg), -crop.x, -crop.y);
        let flipped = NSGraphicsContext::graphicsContextWithCGContext_flipped(&cg, true);
        NSGraphicsContext::saveGraphicsState_class();
        NSGraphicsContext::setCurrentContext(Some(&flipped));
        let full = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(state.screen.width, state.screen.height));
        unsafe {
            source.drawInRect_fromRect_operation_fraction_respectFlipped_hints(
                full,
                NSRect::ZERO,
                NSCompositingOperation::Copy,
                1.0,
                true,
                None,
            )
        };
        Paint::new(self.accent).marks(session, screen);
        flipped.flushGraphics();
        NSGraphicsContext::restoreGraphicsState_class();
        let png = unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) }?;
        Some(png.to_vec())
    }
}

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

fn new_rep(w: isize, h: isize) -> Option<Retained<NSBitmapImageRep>> {
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            std::ptr::null_mut(),
            w,
            h,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            w * 4,
            32,
        )
    }?;
    // Screenshots are sRGB; keep them so (no conversion to a display profile).
    rep.bitmapImageRepByRetaggingWithColorSpace(&NSColorSpace::sRGBColorSpace())
}

/// A frame's pixels as a bitmap to draw from.
fn bitmap(frame: &Frame) -> Option<Retained<NSBitmapImageRep>> {
    let rep = new_rep(frame.width as isize, frame.height as isize)?;
    let len = (frame.width * frame.height * 4) as usize;
    if frame.rgba.len() < len {
        return None;
    }
    unsafe { std::ptr::copy_nonoverlapping(frame.rgba.as_ptr(), rep.bitmapData(), len) };
    Some(rep)
}

pub fn copy_png(png: &[u8]) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setData_forType(Some(&NSData::with_bytes(png)), unsafe { NSPasteboardTypePNG });
}

pub fn copy_text(text: &str) {
    let pasteboard = NSPasteboard::generalPasteboard();
    pasteboard.clearContents();
    pasteboard.setString_forType(&NSString::from_str(text), unsafe { NSPasteboardTypeString });
}

fn ns(r: PtRect) -> NSRect {
    NSRect::new(NSPoint::new(r.x, r.y), NSSize::new(r.width, r.height))
}

fn color((r, g, b): (f64, f64, f64), a: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

fn hex(value: u32, a: f64) -> Retained<NSColor> {
    let c = |shift: u32| ((value >> shift) & 0xff) as f64 / 255.0;
    color((c(16), c(8), c(0)), a)
}

fn line(a: (f64, f64), b: (f64, f64), width: f64) {
    let path = NSBezierPath::new();
    path.moveToPoint(NSPoint::new(a.0, a.1));
    path.lineToPoint(NSPoint::new(b.0, b.1));
    path.setLineWidth(width);
    path.stroke();
}

fn stroke_rect(r: PtRect, width: f64, dash: Option<&[f64]>) {
    let path = NSBezierPath::bezierPathWithRect(ns(r));
    path.setLineWidth(width);
    if let Some(dash) = dash {
        unsafe { path.setLineDash_count_phase(dash.as_ptr(), dash.len() as isize, 0.0) };
    }
    path.stroke();
}

fn attributes(font: &NSFont, color: &NSColor) -> Retained<NSDictionary<NSAttributedStringKey, AnyObject>> {
    let keys: [&NSAttributedStringKey; 2] = unsafe { [NSFontAttributeName, NSForegroundColorAttributeName] };
    let values: [&AnyObject; 2] = [font, color];
    NSDictionary::from_slices(&keys, &values)
}

fn text_size(text: &str, font: &NSFont) -> NSSize {
    let attrs = attributes(font, &NSColor::whiteColor());
    unsafe { NSString::from_str(text).sizeWithAttributes(Some(&attrs)) }
}

fn draw_text(text: &str, at: NSPoint, font: &NSFont, color: &NSColor) {
    let attrs = attributes(font, color);
    unsafe { NSString::from_str(text).drawAtPoint_withAttributes(at, Some(&attrs)) };
}

/// Colors and fonts for one drawing pass (the overlay's, see `style`).
struct Paint {
    accent: (f64, f64, f64),
    font: Retained<NSFont>,
}

impl Paint {
    fn new(accent: (f64, f64, f64)) -> Paint {
        Paint { accent, font: NSFont::monospacedDigitSystemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold }) }
    }

    /// A dark pill with white `text`, centered on `at`.
    fn label(&self, text: &str, at: (f64, f64)) {
        let size = text_size(text, &self.font);
        let rect = NSRect::new(
            NSPoint::new(at.0 - size.width / 2.0 - 5.0, at.1 - size.height / 2.0 - 2.0),
            NSSize::new(size.width + 10.0, size.height + 4.0),
        );
        hex(style::LABEL, style::LABEL_ALPHA as f64).setFill();
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 4.0, 4.0).fill();
        draw_text(text, NSPoint::new(rect.origin.x + 5.0, rect.origin.y + 2.0), &self.font, &NSColor::whiteColor());
    }

    fn label_width(&self, text: &str) -> f64 {
        text_size(text, &self.font).width + 10.0
    }

    fn marks(&self, session: &Session, screen: usize) {
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
                hex(style::GUIDE, 1.0).setStroke();
                match axis {
                    Axis::X => line((p + px / 2.0, 0.0), (p + px / 2.0, height), px),
                    Axis::Y => line((0.0, p + px / 2.0), (width, p + px / 2.0), px),
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
                self.label(&n(gap), if axis == Axis::X { (mid, 14.0) } else { (28.0, mid) });
            }
        }

        for m in &state.held {
            self.measurement(m, scale, units, width, height);
        }
        for a in &state.annotations {
            self.annotation(a);
        }
    }

    fn measurement(&self, m: &Measurement, scale: f64, units: Units, width: f64, height: f64) {
        let n = |v: f64| num(v, scale, units);
        match m {
            Measurement::Distance { x, y, across, down } => {
                let (cx, cy) = (x + 0.5 / scale, y + 0.5 / scale);
                if !across.is_empty() {
                    color(self.accent, 1.0).setStroke();
                    line((across.start, cy), (across.end, cy), 1.0);
                    for (edge, open) in [(across.start, across.open_start), (across.end, across.open_end)] {
                        if !open {
                            line((edge, cy - 4.0), (edge, cy + 4.0), 1.0);
                        }
                    }
                    let ly = if cy < 30.0 { cy + 14.0 } else { cy - 14.0 };
                    self.label(&n(across.len()), ((across.start + across.end) / 2.0, ly));
                }
                if !down.is_empty() {
                    // Drawing a label's text changes the stroke color.
                    color(self.accent, 1.0).setStroke();
                    line((cx, down.start), (cx, down.end), 1.0);
                    for (edge, open) in [(down.start, down.open_start), (down.end, down.open_end)] {
                        if !open {
                            line((cx - 4.0, edge), (cx + 4.0, edge), 1.0);
                        }
                    }
                    let text = n(down.len());
                    let w = self.label_width(&text);
                    let lx = if cx + w + 16.0 > width { cx - w / 2.0 - 8.0 } else { cx + w / 2.0 + 8.0 };
                    self.label(&text, (lx, (down.start + down.end) / 2.0));
                }
            }
            Measurement::Area { rect, .. } => {
                color(self.accent, style::AREA_TINT as f64).setFill();
                NSBezierPath::fillRect(ns(*rect));
                color(self.accent, 1.0).setStroke();
                stroke_rect(*rect, 1.0, None);
                let text = format!("{} × {}", n(rect.width), n(rect.height));
                self.label(&text, style::area_label(*rect, self.label_width(&text), height));
            }
        }
    }

    fn annotation(&self, a: &Annotation) {
        let (r, g, b, alpha) = style::mark_color(a, self.accent_hex());
        let mark = color((r, g, b), 1.0);
        match (&a.shape, a.area) {
            (Some(Shape::Sketch { points }), _) => {
                let path = NSBezierPath::new();
                for (i, p) in points.iter().enumerate() {
                    let p = NSPoint::new(p.0, p.1);
                    if i == 0 { path.moveToPoint(p) } else { path.lineToPoint(p) }
                }
                path.setLineWidth(style::DRAW_WIDTH as f64);
                path.setLineCapStyle(NSLineCapStyle::Round);
                path.setLineJoinStyle(NSLineJoinStyle::Round);
                mark.setStroke();
                path.stroke();
            }
            (Some(Shape::Arrow { from, to }), _) => {
                let [tip, left, right] = style::arrow_head(*from, *to);
                let shaft = NSBezierPath::new();
                shaft.moveToPoint(NSPoint::new(from.0, from.1));
                shaft.lineToPoint(NSPoint::new((left.0 + right.0) / 2.0, (left.1 + right.1) / 2.0));
                shaft.setLineWidth(style::DRAW_WIDTH as f64);
                shaft.setLineCapStyle(NSLineCapStyle::Round);
                mark.setStroke();
                shaft.stroke();
                let head = NSBezierPath::new();
                head.moveToPoint(NSPoint::new(tip.0, tip.1));
                head.lineToPoint(NSPoint::new(left.0, left.1));
                head.lineToPoint(NSPoint::new(right.0, right.1));
                head.closePath();
                mark.setFill();
                head.fill();
            }
            (None, Some(area)) => {
                color((r, g, b), alpha as f64).setFill();
                NSBezierPath::fillRect(ns(area));
                mark.setStroke();
                stroke_rect(area, 1.5, Some(&[6.0, 4.0]));
            }
            (None, None) => {
                if let Some(frame) = a.element.as_ref().and_then(|e| e.frame) {
                    color((r, g, b), 0.9).setStroke();
                    stroke_rect(frame, 1.5, None);
                }
            }
        }
        let rect = NSRect::new(NSPoint::new(a.at.0 - MARKER / 2.0, a.at.1 - MARKER / 2.0), NSSize::new(MARKER, MARKER));
        mark.setFill();
        NSBezierPath::bezierPathWithOvalInRect(rect).fill();
        NSColor::whiteColor().setStroke();
        let ring = NSBezierPath::bezierPathWithOvalInRect(rect);
        ring.setLineWidth(2.0);
        ring.stroke();
        let font = NSFont::monospacedDigitSystemFontOfSize_weight(11.0, unsafe { NSFontWeightBold });
        let text = a.number.to_string();
        let size = text_size(&text, &font);
        draw_text(
            &text,
            NSPoint::new(a.at.0 - size.width / 2.0, a.at.1 - size.height / 2.0),
            &font,
            &NSColor::whiteColor(),
        );
    }

    fn accent_hex(&self) -> u32 {
        let c = |v: f64| ((v * 255.0).round() as u32).min(255);
        (c(self.accent.0) << 16) | (c(self.accent.1) << 8) | c(self.accent.2)
    }
}

use asides_core::inspect::Units;

#[cfg(test)]
mod tests {
    use asides_core::inspect::{Element, Intent, Screen, Severity};
    use asides_core::measure::{Rect, Tolerance};
    use asides_core::overlay::Action;

    use super::*;

    #[test]
    fn renders_marks_onto_the_screen() {
        // A 400 × 240 pt screen at 2×: a card with a title, a line of text and a blue button.
        let mut frame = Frame::filled(800, 480, 0xffffff);
        frame.fill(Rect::new(80, 60, 640, 380), 0xf3f4f6);
        frame.fill(Rect::new(120, 100, 300, 28), 0x111827);
        frame.fill(Rect::new(120, 160, 480, 16), 0x6b7280);
        frame.fill(Rect::new(120, 340, 200, 48), 0x2563eb);
        frame.fill(Rect::new(160, 356, 120, 16), 0xffffff);
        let screen = Screen { name: None, width: 400.0, height: 240.0, scale: 2.0 };
        let mut session = Session::new(
            vec![Session::screen(screen, frame.clone())],
            Tolerance::DEFAULT,
            asides_core::inspect::Units::Pt,
        );
        let button = Element {
            app: Some("Safari".into()),
            name: Some("button \"Save\"".into()),
            frame: Some(PtRect::new(60.0, 170.0, 100.0, 24.0)),
            ..Element::default()
        };
        for action in [
            Action::Click { screen: 0, at: (100.0, 140.0) },
            Action::Drag { screen: 0, from: (50.0, 160.0), to: (175.0, 205.0), snap: true },
            Action::Guide { screen: 0, axis: Axis::X, at: (59.0, 50.0) },
            Action::Annotate {
                screen: 0,
                at: (110.0, 182.0),
                area: None,
                element: Some(Box::new(button)),
                comment: "Align it with the text above".into(),
                intent: Some(Intent::Fix),
                severity: Some(Severity::Important),
            },
            Action::Draw {
                screen: 0,
                points: vec![(250.0, 60.0), (330.0, 100.0)],
                arrow: true,
                element: None,
                target: None,
                comment: String::new(),
                intent: None,
                severity: None,
            },
        ] {
            session.apply(&action);
        }
        let exporter = Exporter::new(&[&frame], (1.0, 0.176, 0.333));
        let around = crop(&session, 0, 24.0, false);
        let png = exporter.render(&session, 0, around).expect("a PNG");
        let rep = NSBitmapImageRep::imageRepWithData(&NSData::with_bytes(&png)).expect("a PNG");
        let (w, h) = (rep.pixelsWide(), rep.pixelsHigh());
        assert!(w > 500 && h > 300 && w <= 800 && h <= 480, "{w}×{h}");
        assert_eq!(exporter.render(&session, 0, crop(&session, 0, 0.0, true)).map(|p| p.len() > 1000), Some(true));
        // ASIDES_OVERLAY_DUMP=<dir> writes it for a look.
        if let Some(dir) = std::env::var_os("ASIDES_OVERLAY_DUMP") {
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(std::path::Path::new(&dir).join("session.png"), png).unwrap();
        }
    }
}
