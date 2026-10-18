//! Menu-bar icons: the asides mark, the letter a in brackets. Every status keeps the brackets and
//! changes what's between them. The app icon (`scripts/make-icon.swift`) and the website
//! (`docs/favicon.svg`, `docs/index.html`) draw the same mark.

use std::cell::RefCell;
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

/// What goes between the brackets. One mark says it all, and nothing goes beside the brackets but
/// a recording's time or a download's progress.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    /// The a: ready to record.
    Letter,
    /// Recording (red).
    Dot,
    /// Under way: transcribing, an agent starting or on an aside.
    Ellipsis,
    /// Done: sent, resolved, an image added.
    Check,
    /// Added to the note as entry N.
    Number(u32),
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

/// Every mark's icon, drawn once; a number's when it's shown (the last one kept).
pub struct Icons {
    marks: Vec<(Mark, StatusIcon)>,
    number: RefCell<Option<(u32, StatusIcon)>>,
}

impl Icons {
    pub fn load() -> Icons {
        let marks = Mark::ALL
            .iter()
            .map(|&mark| {
                let color = (mark == Mark::Dot).then_some(RED);
                (mark, StatusIcon { icon: icon(mark, color), template: color.is_none() })
            })
            .collect();
        Icons { marks, number: RefCell::new(None) }
    }

    pub fn get(&self, mark: Mark) -> StatusIcon {
        if let Mark::Number(n) = mark {
            let mut number = self.number.borrow_mut();
            if number.as_ref().is_none_or(|(shown, _)| *shown != n) {
                *number = Some((n, StatusIcon { icon: icon(mark, None), template: true }));
            }
            return number.as_ref().expect("just drawn").1.clone();
        }
        self.marks.iter().find(|(m, _)| *m == mark).expect("every mark is drawn").1.clone()
    }
}

fn icon(mark: Mark, color: Option<[u8; 3]>) -> Icon {
    let (width, height) = ((width(mark) * SCALE) as u32, (CANVAS_PT * SCALE) as u32);
    Icon::from_rgba(draw(mark, color), width, height).expect("valid status icon")
}

/// A digit's box, in points, and the space between two (its strokes reach half theirs past it).
const DIGIT: (f64, f64) = (4.4, 7.6);
const DIGIT_GAP: f64 = 2.5;
/// How wide a number fits between the brackets where they are: a digit. A longer one moves the
/// right bracket over, so the number keeps the margins the a has.
const NUMBER_ROOM: f64 = DIGIT.0;

fn digits(n: u32) -> Vec<u32> {
    n.to_string().chars().filter_map(|c| c.to_digit(10)).collect()
}

fn number_width(n: u32) -> f64 {
    let count = digits(n).len() as f64;
    count * DIGIT.0 + (count - 1.0) * DIGIT_GAP
}

/// The icon's width in points: the canvas's, or wider for a number of three digits or more.
fn width(mark: Mark) -> f64 {
    match mark {
        Mark::Number(n) => CANVAS_PT + (number_width(n) - NUMBER_ROOM).max(0.0),
        _ => CANVAS_PT,
    }
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
    let right = width(mark) - CANVAS_PT;
    let mut shapes = vec![
        Line((5.4, 3.8), (2.8, 3.8)),
        Line((2.8, 3.8), (2.8, 18.2)),
        Line((2.8, 18.2), (5.4, 18.2)),
        Line((16.6 + right, 3.8), (19.2 + right, 3.8)),
        Line((19.2 + right, 3.8), (19.2 + right, 18.2)),
        Line((19.2 + right, 18.2), (16.6 + right, 18.2)),
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
        Mark::Number(n) => {
            let mut x = (width(mark) - number_width(n)) / 2.0;
            let mut strokes = Vec::new();
            for d in digits(n) {
                strokes.extend(digit(d, x, 11.0 - DIGIT.1 / 2.0));
                x += DIGIT.0 + DIGIT_GAP;
            }
            strokes
        }
        Mark::Nothing => vec![],
    });
    shapes
}

/// Digit `d` in its box ([`DIGIT`]), top-left corner at `(x, y)`, in the marks' strokes.
fn digit(d: u32, x: f64, y: f64) -> Vec<Shape> {
    use Shape::*;
    let (w, h) = DIGIT;
    let (cx, r) = (x + w / 2.0, w / 2.0);
    match d {
        0 => vec![
            Arc((cx, y + r), r, PI, TAU),
            Arc((cx, y + h - r), r, 0.0, PI),
            Line((x, y + r), (x, y + h - r)),
            Line((x + w, y + r), (x + w, y + h - r)),
        ],
        1 => vec![Line((x + w * 0.62, y), (x + w * 0.62, y + h)), Line((x + w * 0.62, y), (x + w * 0.1, y + 1.9))],
        2 => vec![
            Arc((cx, y + r), r, PI, 2.25 * PI),
            Line((cx + r * 0.707, y + r + r * 0.707), (x, y + h)),
            Line((x, y + h), (x + w, y + h)),
        ],
        3 => vec![Arc((cx, y + 1.9), 1.9, 1.1 * PI, 2.5 * PI), Arc((cx, y + h - 1.9), 1.9, 1.5 * PI, 2.9 * PI)],
        4 => vec![
            Line((x + 3.2, y), (x, y + 5.0)),
            Line((x, y + 5.0), (x + w, y + 5.0)),
            Line((x + 3.2, y), (x + 3.2, y + h)),
        ],
        5 => vec![
            Line((x + w, y), (x + 0.5, y)),
            Line((x + 0.5, y), (x + 0.3, y + 3.6)),
            Arc((cx, y + h - 2.3), 2.3, 1.15 * PI, 2.85 * PI),
        ],
        6 => vec![Ring((cx, y + h - 2.3), 2.3), Line((x + 3.4, y), (x + 0.2, y + 4.4))],
        7 => vec![Line((x, y), (x + w, y)), Line((x + w, y), (x + 1.3, y + h))],
        8 => vec![Ring((cx, y + 1.75), 1.75), Ring((cx, y + h - 2.05), 2.05)],
        _ => vec![Ring((cx, y + 2.3), 2.3), Line((x + w - 0.2, y + 3.0), (x + 1.0, y + h))],
    }
}

/// Straight-alpha RGBA pixels of the mark on the status-item canvas, black unless `color` is given.
fn draw(mark: Mark, color: Option<[u8; 3]>) -> Vec<u8> {
    let shapes = shapes(mark);
    let [r, g, b] = color.unwrap_or([0, 0, 0]);
    let (width, height) = ((width(mark) * SCALE) as usize, (CANVAS_PT * SCALE) as usize);
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
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
    fn numbers_go_between_the_brackets() {
        for n in [1, 7, 42, 108, 2026, 1_234_567_890] {
            let rgba = draw(Mark::Number(n), None);
            let width = (width(Mark::Number(n)) * SCALE) as usize;
            assert_eq!(rgba.len(), width * 44 * 4, "{n}");
            assert_eq!(width, if n < 10 { 44 } else { 44 + ((number_width(n) - NUMBER_ROOM) * SCALE) as usize }, "{n}");
            let at = |(x, y): (f64, f64)| rgba[((y * SCALE) as usize * width + (x * SCALE) as usize) * 4 + 3];
            let right = width as f64 / SCALE - 2.8;
            assert_eq!((at((2.8, 11.0)), at((right, 11.0))), (255, 255), "{n}: both brackets");
            // Every digit drawn, each in its own place.
            let inked = |x0: f64, x1: f64| {
                (0..44).any(|y| {
                    ((x0 * SCALE) as usize..(x1 * SCALE) as usize).any(|x| rgba[(y * width + x) * 4 + 3] > 128)
                })
            };
            let mut x = (width as f64 / SCALE - number_width(n)) / 2.0;
            for _ in digits(n) {
                assert!(inked(x, x + DIGIT.0), "{n}: a digit at {x}");
                x += DIGIT.0 + DIGIT_GAP;
            }
            if let Some(dir) = std::env::var_os("ASIDES_ICON_DUMP") {
                std::fs::create_dir_all(&dir).unwrap();
                std::fs::write(std::path::Path::new(&dir).join(format!("number-{n}-{width}x44.rgba")), &rgba).unwrap();
            }
        }
        assert_eq!(digits(2026), [2, 0, 2, 6]);
        let _ = Icons::load().get(Mark::Number(12));
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
