//! Menu-bar icons: the asides mark, the letter a in brackets. Every status keeps the brackets and
//! changes what's between them. The app icon (`scripts/make-icon.swift`) and the website
//! (`docs/favicon.svg`, `docs/index.html`) draw the same mark.

use std::f64::consts::{PI, TAU};

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

/// What goes between the brackets. One mark says it all: anything beside the brackets is a word
/// or a number (`#3`, `→ Codex`), never another symbol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// The a: ready to record.
    Letter,
    /// Recording (red).
    Dot,
    /// Under way: transcribing, an agent starting or on an aside.
    Ellipsis,
    /// Done: added to the note, sent, resolved.
    Check,
    /// Cancelled or ended: nothing added.
    Cross,
    /// Nothing: nothing heard, an aside dismissed.
    Dash,
    /// An agent asks something.
    Question,
    /// Something needs the user.
    Exclamation,
    /// Downloading the speech model.
    Arrow,
    /// Not bound to a repo.
    Nothing,
}

impl Mark {
    const ALL: [Mark; 10] = [
        Mark::Letter,
        Mark::Dot,
        Mark::Ellipsis,
        Mark::Check,
        Mark::Cross,
        Mark::Dash,
        Mark::Question,
        Mark::Exclamation,
        Mark::Arrow,
        Mark::Nothing,
    ];
}

/// Every mark's icon, drawn once.
pub struct Icons(Vec<(Mark, StatusIcon)>);

impl Icons {
    pub fn load() -> Icons {
        Icons(
            Mark::ALL
                .iter()
                .map(|&mark| {
                    let color = (mark == Mark::Dot).then_some(RED);
                    (mark, StatusIcon { icon: icon(mark, color), template: color.is_none() })
                })
                .collect(),
        )
    }

    pub fn get(&self, mark: Mark) -> &StatusIcon {
        &self.0.iter().find(|(m, _)| *m == mark).expect("every mark is drawn").1
    }
}

fn icon(mark: Mark, color: Option<[u8; 3]>) -> Icon {
    let size = (CANVAS_PT * SCALE) as u32;
    Icon::from_rgba(draw(mark, color), size, size).expect("valid status icon")
}

/// A stroke with round caps, a stroked circle or arc, or a filled circle, in points from the top
/// left. An arc runs clockwise on screen between two angles in radians (0 to the right, π/2 down).
enum Shape {
    Line((f64, f64), (f64, f64)),
    Ring((f64, f64), f64),
    Arc((f64, f64), f64, f64, f64),
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
            Shape::Arc(c, r, from, to) => {
                let angle = (p.1 - c.1).atan2(p.0 - c.0);
                if (angle - from).rem_euclid(TAU) <= to - from {
                    (length(p.0 - c.0, p.1 - c.1) - r).abs() - STROKE / 2.0
                } else {
                    let end = |a: f64| length(p.0 - c.0 - r * a.cos(), p.1 - c.1 - r * a.sin());
                    end(from).min(end(to)) - STROKE / 2.0
                }
            }
            Shape::Disc(c, r) => length(p.0 - c.0, p.1 - c.1) - r,
        }
    }
}

fn shapes(mark: Mark) -> Vec<Shape> {
    use Shape::*;
    let mut shapes = vec![
        Line((5.4, 3.8), (2.8, 3.8)),
        Line((2.8, 3.8), (2.8, 18.2)),
        Line((2.8, 18.2), (5.4, 18.2)),
        Line((16.6, 3.8), (19.2, 3.8)),
        Line((19.2, 3.8), (19.2, 18.2)),
        Line((19.2, 18.2), (16.6, 18.2)),
    ];
    shapes.extend(match mark {
        // A geometric single-storey a: a circle with its stem along the right side.
        Mark::Letter => vec![Ring((11.0, 11.5), 2.9), Line((13.9, 8.6), (13.9, 14.4))],
        Mark::Dot => vec![Disc((11.0, 11.0), 3.0)],
        Mark::Ellipsis => [8.4, 11.0, 13.6].map(|x| Disc((x, 11.0), 1.1)).into(),
        Mark::Check => vec![Line((8.6, 11.2), (10.3, 13.0)), Line((10.3, 13.0), (13.5, 9.3))],
        Mark::Cross => vec![Line((8.9, 8.9), (13.1, 13.1)), Line((13.1, 8.9), (8.9, 13.1))],
        Mark::Dash => vec![Line((8.7, 11.0), (13.3, 11.0))],
        // A hook over the top from the left, down to the middle, and the dot.
        Mark::Question => {
            vec![Arc((11.0, 9.4), 2.0, PI, 2.5 * PI), Line((11.0, 11.4), (11.0, 11.9)), Disc((11.0, 14.6), 1.05)]
        }
        Mark::Exclamation => vec![Line((11.0, 7.6), (11.0, 11.5)), Disc((11.0, 14.2), 1.05)],
        Mark::Arrow => {
            vec![Line((11.0, 7.4), (11.0, 14.0)), Line((8.4, 11.6), (11.0, 14.2)), Line((11.0, 14.2), (13.6, 11.6))]
        }
        Mark::Nothing => vec![],
    });
    shapes
}

/// Straight-alpha RGBA pixels of the mark on the status-item canvas, black unless `color` is given.
fn draw(mark: Mark, color: Option<[u8; 3]>) -> Vec<u8> {
    let shapes = shapes(mark);
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

    fn alpha(rgba: &[u8], (x, y): (f64, f64)) -> u8 {
        let size = (CANVAS_PT * SCALE) as usize;
        rgba[((y * SCALE) as usize * size + (x * SCALE) as usize) * 4 + 3]
    }

    #[test]
    fn renders_every_status_icon() {
        for mark in Mark::ALL {
            let name = format!("{mark:?}").to_lowercase();
            let red = mark == Mark::Dot;
            let rgba = draw(mark, red.then_some(RED));
            assert_eq!(rgba.len(), 44 * 44 * 4, "{name}");
            // The brackets are in every icon, and the centre between them is empty with nothing inside.
            assert_eq!(alpha(&rgba, (2.8, 11.0)), 255, "{name}: left bracket");
            assert_eq!(alpha(&rgba, (19.2, 11.0)), 255, "{name}: right bracket");
            assert_eq!(alpha(&rgba, (11.0, 2.0)), 0, "{name}: above the brackets");
            let opaque = rgba.chunks(4).filter(|px| px[3] > 128).count();
            let inner =
                rgba.chunks(4).enumerate().filter(|(i, px)| px[3] > 128 && (14..30).contains(&(i % 44))).count();
            assert_eq!(inner == 0, mark == Mark::Nothing, "{name}: {inner} pixels between the brackets");
            let reddish = rgba.chunks(4).filter(|px| px[3] > 128 && px[0] > 180 && px[1] < 120).count();
            assert_eq!(reddish == opaque, red, "{name}: color");
            // ASIDES_ICON_DUMP=<dir> writes the raw pixels for visual inspection.
            if let Some(dir) = std::env::var_os("ASIDES_ICON_DUMP") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("{name}-44x44.rgba")), &rgba).unwrap();
            }
            let _ = icon(mark, red.then_some(RED));
        }
    }

    #[test]
    fn the_letter_is_an_a() {
        let rgba = draw(Mark::Letter, None);
        // A closed bowl around an open counter, and the stem beside it.
        assert_eq!(alpha(&rgba, (11.0, 11.5)), 0, "counter");
        assert_eq!(alpha(&rgba, (8.1, 11.5)), 255, "bowl, left");
        assert_eq!(alpha(&rgba, (13.9, 9.0)), 255, "stem, top");
        assert_eq!(alpha(&rgba, (13.9, 14.0)), 255, "stem, bottom");
    }

    #[test]
    fn the_marks_are_what_they_say() {
        let cross = draw(Mark::Cross, None);
        assert_eq!(alpha(&cross, (11.0, 11.0)), 255, "cross: where the strokes meet");
        assert_eq!(alpha(&cross, (11.0, 8.6)), 0, "cross: open above");
        let dash = draw(Mark::Dash, None);
        assert_eq!(alpha(&dash, (11.0, 11.0)), 255, "dash");
        assert_eq!(alpha(&dash, (11.0, 9.5)), 0, "dash: one stroke");
        let question = draw(Mark::Question, None);
        assert_eq!(alpha(&question, (11.0, 7.4)), 255, "question: the hook's top");
        assert_eq!(alpha(&question, (9.0, 9.4)), 255, "question: the hook's left");
        assert_eq!(alpha(&question, (9.0, 11.6)), 0, "question: open below its left");
        assert_eq!(alpha(&question, (11.0, 14.6)), 255, "question: the dot");
        assert_eq!(alpha(&question, (11.0, 13.1)), 0, "question: the dot stands apart");
    }
}
