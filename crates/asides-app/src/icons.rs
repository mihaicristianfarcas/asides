//! Menu-bar icons: the asides mark, the letter a in brackets. Every status keeps the brackets and
//! changes what's between them. The app icon (`scripts/make-icon.swift`) and the website
//! (`docs/favicon.svg`, `docs/index.html`) draw the same mark.

use tray_icon::Icon;

/// tray-icon shows icons at most 22 pt tall; we draw at 2× on a 22 pt canvas.
const CANVAS_PT: f64 = 22.0;
const SCALE: f64 = 2.0;
/// The width of every stroke, in points.
const STROKE: f64 = 1.75;
/// systemRed, for the one status that isn't a template.
const RED: [u8; 3] = [255, 59, 48];

#[derive(Clone)]
pub struct StatusIcon {
    pub icon: Icon,
    /// Template icons are tinted by the system (black/white); colored ones are not.
    pub template: bool,
}

pub struct Icons {
    pub idle: StatusIcon,
    pub unbound: StatusIcon,
    pub recording: StatusIcon,
    pub busy: StatusIcon,
    pub done: StatusIcon,
    pub error: StatusIcon,
    pub download: StatusIcon,
}

/// What goes between the brackets.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Inside {
    /// The a: ready to record.
    Letter,
    /// Recording.
    Dot,
    /// Transcribing.
    Ellipsis,
    /// Added to the note.
    Check,
    Exclamation,
    /// Downloading the speech model.
    Arrow,
    /// Not bound to a repo.
    Nothing,
}

impl Icons {
    pub fn load() -> Icons {
        let template = |inside| StatusIcon { icon: icon(inside, None), template: true };
        Icons {
            idle: template(Inside::Letter),
            unbound: template(Inside::Nothing),
            recording: StatusIcon { icon: icon(Inside::Dot, Some(RED)), template: false },
            busy: template(Inside::Ellipsis),
            done: template(Inside::Check),
            error: template(Inside::Exclamation),
            download: template(Inside::Arrow),
        }
    }
}

fn icon(inside: Inside, color: Option<[u8; 3]>) -> Icon {
    let size = (CANVAS_PT * SCALE) as u32;
    Icon::from_rgba(draw(inside, color), size, size).expect("valid status icon")
}

/// A stroke with round caps, a stroked circle, or a filled one, in points from the top left.
enum Shape {
    Line((f64, f64), (f64, f64)),
    Ring((f64, f64), f64),
    Disc((f64, f64), f64),
}

impl Shape {
    /// Signed distance from `p` to the shape's edge, in points (negative inside).
    fn distance(&self, p: (f64, f64)) -> f64 {
        let length = |x: f64, y: f64| x.hypot(y);
        match *self {
            Shape::Line(a, b) => {
                let (dx, dy) = (b.0 - a.0, b.1 - a.1);
                let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
                length(p.0 - a.0 - t * dx, p.1 - a.1 - t * dy) - STROKE / 2.0
            }
            Shape::Ring(c, r) => (length(p.0 - c.0, p.1 - c.1) - r).abs() - STROKE / 2.0,
            Shape::Disc(c, r) => length(p.0 - c.0, p.1 - c.1) - r,
        }
    }
}

fn shapes(inside: Inside) -> Vec<Shape> {
    use Shape::*;
    let mut shapes = vec![
        Line((5.4, 3.8), (2.8, 3.8)),
        Line((2.8, 3.8), (2.8, 18.2)),
        Line((2.8, 18.2), (5.4, 18.2)),
        Line((16.6, 3.8), (19.2, 3.8)),
        Line((19.2, 3.8), (19.2, 18.2)),
        Line((19.2, 18.2), (16.6, 18.2)),
    ];
    shapes.extend(match inside {
        // A geometric single-storey a: a circle with its stem along the right side.
        Inside::Letter => vec![Ring((11.0, 11.5), 2.9), Line((13.9, 8.6), (13.9, 14.4))],
        Inside::Dot => vec![Disc((11.0, 11.0), 3.0)],
        Inside::Ellipsis => [8.4, 11.0, 13.6].map(|x| Disc((x, 11.0), 1.1)).into(),
        Inside::Check => vec![Line((8.6, 11.2), (10.3, 13.0)), Line((10.3, 13.0), (13.5, 9.3))],
        Inside::Exclamation => vec![Line((11.0, 7.6), (11.0, 11.5)), Disc((11.0, 14.2), 1.05)],
        Inside::Arrow => {
            vec![Line((11.0, 7.4), (11.0, 14.0)), Line((8.4, 11.6), (11.0, 14.2)), Line((11.0, 14.2), (13.6, 11.6))]
        }
        Inside::Nothing => vec![],
    });
    shapes
}

/// Straight-alpha RGBA pixels of the mark on the status-item canvas, black unless `color` is given.
fn draw(inside: Inside, color: Option<[u8; 3]>) -> Vec<u8> {
    let shapes = shapes(inside);
    let [r, g, b] = color.unwrap_or([0, 0, 0]);
    let size = (CANVAS_PT * SCALE) as usize;
    let mut rgba = Vec::with_capacity(size * size * 4);
    for y in 0..size {
        for x in 0..size {
            let p = ((x as f64 + 0.5) / SCALE, (y as f64 + 0.5) / SCALE);
            let d = shapes.iter().map(|s| s.distance(p)).fold(f64::INFINITY, f64::min);
            // Coverage from the distance in pixels: a one-pixel ramp across the edge.
            let a = (0.5 - d * SCALE).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[r, g, b, (a * 255.0).round() as u8]);
        }
    }
    rgba
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(&str, Inside); 7] = [
        ("idle", Inside::Letter),
        ("unbound", Inside::Nothing),
        ("recording", Inside::Dot),
        ("busy", Inside::Ellipsis),
        ("done", Inside::Check),
        ("error", Inside::Exclamation),
        ("download", Inside::Arrow),
    ];

    fn alpha(rgba: &[u8], (x, y): (f64, f64)) -> u8 {
        let size = (CANVAS_PT * SCALE) as usize;
        rgba[((y * SCALE) as usize * size + (x * SCALE) as usize) * 4 + 3]
    }

    #[test]
    fn renders_every_status_icon() {
        for (name, inside) in ALL {
            let red = inside == Inside::Dot;
            let rgba = draw(inside, red.then_some(RED));
            assert_eq!(rgba.len(), 44 * 44 * 4, "{name}");
            // The brackets are in every icon, and the centre between them is empty with nothing inside.
            assert_eq!(alpha(&rgba, (2.8, 11.0)), 255, "{name}: left bracket");
            assert_eq!(alpha(&rgba, (19.2, 11.0)), 255, "{name}: right bracket");
            assert_eq!(alpha(&rgba, (11.0, 2.0)), 0, "{name}: above the brackets");
            let opaque = rgba.chunks(4).filter(|px| px[3] > 128).count();
            let inner =
                rgba.chunks(4).enumerate().filter(|(i, px)| px[3] > 128 && (14..30).contains(&(i % 44))).count();
            assert_eq!(inner == 0, inside == Inside::Nothing, "{name}: {inner} pixels between the brackets");
            let reddish = rgba.chunks(4).filter(|px| px[3] > 128 && px[0] > 180 && px[1] < 120).count();
            assert_eq!(reddish == opaque, red, "{name}: color");
            // ASIDES_ICON_DUMP=<dir> writes the raw pixels for visual inspection.
            if let Some(dir) = std::env::var_os("ASIDES_ICON_DUMP") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}-44x44.rgba")), &rgba).unwrap();
            }
            let _ = icon(inside, red.then_some(RED));
        }
    }

    #[test]
    fn the_letter_is_an_a() {
        let rgba = draw(Inside::Letter, None);
        // A closed bowl around an open counter, and the stem beside it.
        assert_eq!(alpha(&rgba, (11.0, 11.5)), 0, "counter");
        assert_eq!(alpha(&rgba, (8.1, 11.5)), 255, "bowl, left");
        assert_eq!(alpha(&rgba, (13.9, 9.0)), 255, "stem, top");
        assert_eq!(alpha(&rgba, (13.9, 14.0)), 255, "stem, bottom");
    }
}
