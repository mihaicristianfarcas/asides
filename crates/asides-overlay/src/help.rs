//! The Help page's clips, and setup's How it works: frames of the real overlay on the website's demo
//! (`help/`, made by `scripts/make-help.py`), and what happens between them (`help/clips.json`): a
//! key pressed, the pointer gliding to what it clicks or drags, what's said. A clip plays them in
//! turn from its first frame, round and round. Each frame stays for its time: the window is drawn
//! every frame only while something moves, and else again when the next change is due.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use gpui::{AnyElement, FontWeight, HighlightStyle, SharedString, StyledText, div, img, prelude::*, px};
use serde::Deserialize;

use crate::controls::{Radius, Theme, ease, hex, icon};
use crate::style;

type Point = (f32, f32);

/// How long each part of going to the next frame takes: a key pressed, the pointer gliding, a drag,
/// a click, the frames crossfading.
const KEY: Duration = Duration::from_millis(380);
const GLIDE: Duration = Duration::from_millis(620);
const DRAG: Duration = Duration::from_millis(900);
const CLICK: Duration = Duration::from_millis(300);
const FADE: Duration = Duration::from_millis(220);
/// A key pressed shows until this long after its frame is in, and pops in and fades out in `POP`.
const KEY_SHOWN: Duration = Duration::from_millis(1100);
const POP: Duration = Duration::from_millis(150);

#[derive(Debug, Deserialize)]
pub struct Clip {
    /// Its size, in points (its images are twice that, in pixels).
    pub size: (f32, f32),
    /// Its corners' radius, when it isn't a card's (the pill's).
    #[serde(default)]
    pub radius: Option<f32>,
    /// Shown as it is, without an outline (the pill, cut out).
    #[serde(default)]
    pub bare: bool,
    pub frames: Vec<Frame>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Frame {
    /// Its image, in `help/`.
    pub image: String,
    /// How long it stays, once in (ms).
    pub hold: u64,
    /// What happens before it shows, in this order: a key pressed (the overlay's action, as in the
    /// settings, or `open` and `send`: the shortcut)...
    pub key: Option<String>,
    /// ...the pointer dragged along this...
    pub path: Vec<Point>,
    /// ...or going here (after a drag, once the frame shows)...
    pub pointer: Option<Point>,
    /// ...and clicking.
    pub click: bool,
    /// What's being said while it shows, pins' numbers as `[1]`.
    pub say: Option<String>,
    /// The step of the basics it shows.
    pub step: Option<usize>,
}

/// The clip called `name` in `help/clips.json`.
pub fn clip(name: &str) -> Option<&'static Clip> {
    static CLIPS: OnceLock<HashMap<String, Clip>> = OnceLock::new();
    CLIPS
        .get_or_init(|| {
            let json = crate::assets::help_file("clips.json").unwrap_or(b"{}");
            serde_json::from_slice(json).unwrap_or_default()
        })
        .get(name)
}

/// What a clip shows at a moment.
#[derive(Debug, PartialEq)]
pub struct Shown<'a> {
    /// The frame in front, how far it has faded in (1: all of it), and the one under it.
    pub frame: usize,
    pub fade: f32,
    pub under: usize,
    pub pointer: Option<Point>,
    /// The pointer held down (dragging, clicking).
    pub pressed: bool,
    /// A click's ring, spreading out: how far (0 to 1).
    pub ripple: Option<f32>,
    /// The key pressed, and how much of it shows.
    pub key: Option<(&'a str, f32)>,
    pub say: Option<&'a str>,
    pub step: Option<usize>,
    /// Something's moving: draw it again next frame. Else, when something next changes.
    pub moving: bool,
    pub next: Option<Duration>,
}

/// Going into a frame: how long each part takes.
struct Parts {
    key: Duration,
    reach: Duration,
    drag: Duration,
    click: Duration,
    hold: Duration,
    /// After a drag, the pointer going on to `pointer` (as the frame stays).
    after: Duration,
}

impl Parts {
    /// Until the frame starts fading in.
    fn before(&self) -> Duration {
        self.key + self.reach + self.drag + self.click
    }

    fn total(&self) -> Duration {
        self.before() + FADE + self.hold
    }
}

fn ms(d: Duration) -> f32 {
    d.as_secs_f32() * 1000.0
}

/// Quick in the middle, gentle at both ends: the pointer, as a hand moves it.
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

fn lerp(a: Point, b: Point, t: f32) -> Point {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

fn apart(a: Point, b: Point) -> bool {
    (a.0 - b.0).abs() > 0.5 || (a.1 - b.1).abs() > 0.5
}

/// Where along `path` a pointer is, `t` of the way (by length).
fn along(path: &[Point], t: f32) -> Point {
    let length = |a: Point, b: Point| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
    let total: f32 = path.windows(2).map(|w| length(w[0], w[1])).sum();
    let mut left = total * t.clamp(0.0, 1.0);
    for w in path.windows(2) {
        let part = length(w[0], w[1]);
        if left <= part && part > 0.0 {
            return lerp(w[0], w[1], left / part);
        }
        left -= part;
    }
    path.last().copied().unwrap_or_default()
}

impl Frame {
    /// Where the pointer goes first: where a drag starts, or where it's going.
    fn target(&self) -> Option<Point> {
        self.path.first().copied().or(self.pointer)
    }

    /// Where the pointer is once the frame is in.
    fn end(&self) -> Option<Point> {
        self.pointer.or(self.path.last().copied())
    }
}

impl Clip {
    /// Where the pointer is once frame `i` is in (it stays where it was, without one of its own).
    fn rest(&self, i: usize) -> Option<Point> {
        let n = self.frames.len();
        (0..n).find_map(|back| self.frames[(i + n - back) % n].end())
    }

    fn parts(&self, i: usize) -> Parts {
        let n = self.frames.len();
        let f = &self.frames[i];
        let from = self.rest((i + n - 1) % n);
        let hold = Duration::from_millis(f.hold);
        let moves = |a: Option<Point>, b: Option<Point>| matches!((a, b), (Some(a), Some(b)) if apart(a, b));
        let after = f.path.last().copied().filter(|_| moves(f.path.last().copied(), f.pointer));
        Parts {
            key: if f.key.is_some() { KEY } else { Duration::ZERO },
            reach: if moves(from, f.target()) { GLIDE } else { Duration::ZERO },
            drag: if f.path.len() > 1 { DRAG } else { Duration::ZERO },
            click: if f.click { CLICK } else { Duration::ZERO },
            hold,
            after: if after.is_some() { GLIDE.min(hold) } else { Duration::ZERO },
        }
    }

    /// When, after it started playing, the clip starts going into step `step`'s first frame (to
    /// watch that step).
    pub fn time_of_step(&self, step: usize) -> Option<Duration> {
        let frame = self.frames.iter().position(|f| f.step == Some(step))?;
        let parts: Vec<Parts> = (0..self.frames.len()).map(|i| self.parts(i)).collect();
        let cycle: Duration = parts.iter().map(Parts::total).sum();
        let start = parts[0].before() + FADE;
        let going: Duration = parts[..frame].iter().map(Parts::total).sum();
        Some(Duration::from_nanos(((going + cycle - start).as_nanos() % cycle.as_nanos().max(1)) as u64))
    }

    /// What it shows `t` after it started playing: frame 0 just in, then each in turn, round again.
    pub fn at(&self, t: Duration) -> Shown<'_> {
        let n = self.frames.len();
        if n < 2 {
            return Shown {
                frame: 0,
                fade: 1.0,
                under: 0,
                pointer: self.frames.first().and_then(Frame::end),
                pressed: false,
                ripple: None,
                key: None,
                say: self.frames.first().and_then(|f| f.say.as_deref()),
                step: self.frames.first().and_then(|f| f.step),
                moving: false,
                next: None,
            };
        }
        let parts: Vec<Parts> = (0..n).map(|i| self.parts(i)).collect();
        let cycle: Duration = parts.iter().map(Parts::total).sum();
        // From frame 0 just faded in.
        let start = parts[0].before() + FADE;
        let mut left = Duration::from_nanos(((t + start).as_nanos() % cycle.as_nanos().max(1)) as u64);
        for (i, p) in parts.iter().enumerate() {
            if left < p.total() {
                return self.going_into(i, p, left);
            }
            left -= p.total();
        }
        self.going_into(0, &parts[0], Duration::ZERO)
    }

    /// Frame `i` coming in, `s` into it.
    fn going_into(&self, i: usize, p: &Parts, s: Duration) -> Shown<'_> {
        let n = self.frames.len();
        let prev = (i + n - 1) % n;
        let (f, before) = (&self.frames[i], &self.frames[prev]);
        let from = self.rest(prev);
        let target = f.target().or(from);
        // The parts' ends, from the start.
        let reached = p.key + p.reach;
        let dragged = reached + p.drag;
        let clicked = dragged + p.click;
        let faded = clicked + FADE;
        let held = faded + p.hold;
        let part = |from: Duration, len: Duration| ms(s.saturating_sub(from)) / ms(len).max(1.0);
        let (mut pointer, mut pressed, mut ripple) = (from, false, None);
        if s >= p.key && s < reached {
            pointer = match (from, target) {
                (Some(a), Some(b)) => Some(lerp(a, b, ease_in_out(part(p.key, p.reach)))),
                _ => target,
            };
        } else if s >= reached && s < dragged {
            (pointer, pressed) = (Some(along(&f.path, ease_in_out(part(reached, p.drag)))), true);
        } else if s >= dragged && s < clicked {
            let spread = part(dragged, p.click);
            (pointer, pressed, ripple) = (target, spread < 0.5, Some(spread));
        } else if s >= clicked {
            let done = f.path.last().copied().or(target);
            pointer = match (done, f.pointer) {
                (Some(a), Some(b)) if p.after > Duration::ZERO => Some(lerp(a, b, ease_in_out(part(faded, p.after)))),
                _ => f.end().or(from),
            };
        }
        let fade = if s < clicked { 0.0 } else { ease(part(clicked, FADE)) };
        let showing = if s < clicked { before } else { f };
        // The step coming, as soon as it does.
        let step = f.step.or(showing.step);
        // The key pressed, from when it's pressed until a while after the frame is in.
        let shown_until = (faded + KEY_SHOWN).min(held);
        let key = f.key.as_deref().filter(|_| s < shown_until).map(|key| {
            let into = part(Duration::ZERO, POP).min(1.0);
            let out = (ms(shown_until.saturating_sub(s)) / ms(POP)).min(1.0);
            (key, into.min(out))
        });
        // Moving: a key popping in or fading out, the pointer, a click, the frames crossfading.
        let spans = [
            (Duration::ZERO, if f.key.is_some() { POP } else { Duration::ZERO }),
            (p.key, faded),
            (faded, faded + p.after),
            (if f.key.is_some() { shown_until.saturating_sub(POP) } else { held }, shown_until),
        ];
        let moving = spans.iter().any(|&(a, b)| s >= a && s < b);
        let next = spans.iter().map(|&(a, _)| a).filter(|&a| a > s).chain([held]).min().map(|at| at - s);
        Shown {
            frame: if s < clicked { prev } else { i },
            fade: if s < clicked { 1.0 } else { fade },
            under: prev,
            pointer,
            pressed,
            ripple,
            key,
            say: showing.say.as_deref(),
            step,
            moving,
            next: if moving { None } else { next },
        }
    }
}

/// `clip` as `shown`, `width` wide: its frames (all of them, those not showing unseen, so each is
/// loaded before it's needed), the pointer, a click's ring, the key pressed, what's said. `key`
/// names a key as the window shows it.
pub fn view(clip: &Clip, shown: &Shown, width: f32, key: impl Fn(&str) -> String, t: Theme) -> AnyElement {
    let scale = width / clip.size.0.max(1.0);
    let height = clip.size.1 * scale;
    let radius = px(clip.radius.map_or(Radius::CARD, |r| r * scale));
    let image = |i: usize, opacity: f32| {
        let path = SharedString::from(format!("help/{}", clip.frames[i].image));
        img(path).absolute().top_0().left_0().size_full().rounded(radius).opacity(opacity).into_any_element()
    };
    // Unseen first, then the one fading out, then the one fading in.
    let mut frames: Vec<AnyElement> =
        (0..clip.frames.len()).filter(|&i| i != shown.frame && i != shown.under).map(|i| image(i, 0.0)).collect();
    if shown.under != shown.frame {
        frames.push(image(shown.under, 1.0));
    }
    frames.push(image(shown.frame, shown.fade));
    let blue = hex(style::GUIDE, 1.0);
    let at = |(x, y): Point| (x * scale, y * scale);
    let ripple = shown.ripple.zip(shown.pointer).map(|(spread, p)| {
        let (x, y) = at(p);
        let r = (5.0 + 17.0 * ease(spread)) * scale.max(0.6);
        div()
            .absolute()
            .left(px(x - r))
            .top(px(y - r))
            .size(px(2.0 * r))
            .rounded_full()
            .border_2()
            .border_color(blue.opacity(1.0 - spread))
    });
    // The pointer, its tip on the point (at 5, 3 of its 24): its edge, then it.
    let pointer = shown.pointer.map(|p| {
        let (x, y) = at(p);
        let size = 22.0 * scale.clamp(0.7, 1.0);
        let unit = size / 24.0;
        let layer = |name: &str, color| div().absolute().top_0().left_0().child(icon(name, size, color));
        div()
            .absolute()
            .left(px(x - 5.0 * unit))
            .top(px(y - 3.0 * unit))
            .size(px(size))
            .when(shown.pressed, |d| {
                d.child(
                    div()
                        .absolute()
                        .left(px(5.0 * unit - 8.0))
                        .top(px(3.0 * unit - 8.0))
                        .size(px(16.))
                        .rounded_full()
                        .bg(blue.opacity(0.35)),
                )
            })
            .child(layer("pointer-edge", hex(0xffffff, 1.0)))
            .child(layer("pointer", hex(0x000000, 1.0)))
    });
    // What's said, and the key pressed, along the bottom.
    let glass = |d: gpui::Div| {
        d.bg(hex(style::GLASS, 0.88)).border_1().border_color(hex(0xffffff, 0.12)).text_color(hex(0xffffff, 0.94))
    };
    let say = shown.say.map(|text| {
        glass(div())
            .max_w(px(width * 0.86))
            .px(px(12.))
            .py(px(7.))
            .rounded(px(10.))
            .flex()
            .items_start()
            .gap(px(8.))
            .text_size(px(13.5))
            .line_height(px(19.))
            .child(div().pt(px(3.)).child(icon("mic", 13., hex(style::RECORDING_SOFT, 1.0))))
            .child(div().flex_1().min_w_0().child(said(text)))
    });
    let key = shown.key.map(|(name, shows)| {
        glass(div())
            .opacity(shows)
            .px(px(12.))
            .py(px(6.))
            .rounded(px(9.))
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child(key(name))
    });
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .rounded(radius)
        .when(!clip.bare, |d| d.border_1().border_color(t.card_stroke))
        .children(frames)
        .children(ripple)
        .children(pointer)
        .child(
            div()
                .absolute()
                .bottom(px(14.))
                .left_0()
                .right_0()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .children(say)
                .children(key),
        )
        .into_any_element()
}

/// What's said, pins' numbers bold, in asides' blue.
fn said(text: &str) -> StyledText {
    let mut highlights = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find('[').map(|i| from + i) {
        let Some(close) = text[open..].find(']').map(|i| open + i) else { break };
        let style = HighlightStyle {
            color: Some(hex(style::UI, 1.0)),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        highlights.push((open..close + 1, style));
        from = close + 1;
    }
    StyledText::new(text.to_string()).with_highlights(highlights)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(image: &str, hold: u64) -> Frame {
        Frame { image: image.into(), hold, ..Frame::default() }
    }

    /// Frame 0 with the pointer at rest; 1 after a key; 2 after a click elsewhere.
    fn clip() -> Clip {
        Clip {
            size: (100.0, 50.0),
            radius: None,
            bare: false,
            frames: vec![
                Frame { pointer: Some((10.0, 10.0)), ..frame("a", 1000) },
                Frame { key: Some("measure".into()), ..frame("b", 1000) },
                Frame { pointer: Some((90.0, 40.0)), click: true, ..frame("c", 1000) },
            ],
        }
    }

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn starts_on_its_first_frame_still_until_the_next_change() {
        let clip = clip();
        let shown = clip.at(Duration::ZERO);
        assert_eq!((shown.frame, shown.fade, shown.pointer), (0, 1.0, Some((10.0, 10.0))));
        assert!(!shown.moving);
        // Drawn again only when the next key is pressed, at the end of its hold.
        assert_eq!(shown.next, Some(ms(1000)));
        assert_eq!(clip.at(ms(999)).frame, 0);
    }

    #[test]
    fn a_key_shows_then_its_frame_fades_in() {
        let clip = clip();
        // Pressed: popping in, the frame before still showing.
        let pressing = clip.at(ms(1050));
        assert_eq!((pressing.frame, pressing.key.map(|(k, _)| k)), (0, Some("measure")));
        assert!(pressing.moving);
        // Popped in: nothing moves until the frames crossfade, when the key's time is up.
        let shown = clip.at(ms(1200));
        assert!(!shown.moving);
        assert_eq!(shown.next, Some(KEY - ms(200)));
        let fading = clip.at(ms(1000) + KEY + ms(110));
        assert_eq!((fading.frame, fading.under), (1, 0));
        assert!(fading.fade > 0.0 && fading.fade < 1.0 && fading.moving);
        // In, the key shows a while, then goes.
        let held = clip.at(ms(1000) + KEY + FADE + ms(10));
        assert_eq!((held.frame, held.fade), (1, 1.0));
        assert!(held.key.is_some());
        assert!(clip.at(ms(1000) + KEY + FADE + KEY_SHOWN + ms(10)).key.is_none());
    }

    #[test]
    fn the_pointer_glides_to_what_it_clicks_then_its_frame_comes() {
        let clip = clip();
        let start = ms(1000) + KEY + FADE + ms(1000);
        let halfway = clip.at(start + GLIDE / 2);
        let (x, y) = halfway.pointer.unwrap();
        assert!(x > 10.0 && x < 90.0 && y > 10.0 && y < 40.0, "{x}, {y}");
        assert_eq!(halfway.frame, 1);
        let clicking = clip.at(start + GLIDE + ms(50));
        assert_eq!(clicking.pointer, Some((90.0, 40.0)));
        assert!(clicking.ripple.is_some() && clicking.pressed);
        let after = clip.at(start + GLIDE + CLICK + FADE + ms(10));
        assert_eq!((after.frame, after.ripple), (2, None));
    }

    #[test]
    fn goes_round_again_from_its_first_frame() {
        let clip = clip();
        let cycle: Duration = (0..3).map(|i| clip.parts(i).total()).sum();
        assert_eq!(clip.at(cycle), clip.at(Duration::ZERO));
        // Back to frame 0 (playing starts once it's in), the pointer glides back to where it rests.
        let back = clip.at(cycle - FADE - GLIDE / 2);
        assert_eq!(back.frame, 2);
        assert!(back.moving && back.pointer != Some((90.0, 40.0)));
    }

    #[test]
    fn a_drag_follows_its_path_held_down() {
        let path = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(along(&path, 0.5), (10.0, 0.0));
        assert_eq!(along(&path, 0.75), (10.0, 5.0));
        let clip = Clip {
            size: (20.0, 20.0),
            radius: None,
            bare: false,
            frames: vec![
                Frame { pointer: Some((0.0, 0.0)), ..frame("a", 500) },
                Frame { path, pointer: Some((20.0, 20.0)), ..frame("b", 1000) },
            ],
        };
        let dragging = clip.at(ms(500) + DRAG / 2);
        assert!(dragging.pressed && dragging.frame == 0);
        // Let go: its frame fades in, then the pointer goes on.
        let after = clip.at(ms(500) + DRAG + FADE + GLIDE / 2);
        assert_eq!(after.frame, 1);
        assert!(after.moving && after.pointer.is_some_and(|(x, _)| x > 10.0 && x < 20.0));
    }

    #[test]
    fn a_step_can_be_watched_from_its_start() {
        let mut clip = clip();
        for (i, f) in clip.frames.iter_mut().enumerate() {
            f.step = Some(i);
        }
        // Step 2: going into frame 2 (the pointer setting off from where frame 1 left it), lit at once.
        let at = clip.time_of_step(2).unwrap();
        let shown = clip.at(at);
        assert_eq!((shown.frame, shown.step, shown.pointer), (1, Some(2), Some((10.0, 10.0))));
        assert!(shown.moving);
        // Step 0: the clip going round to its first frame.
        let shown = clip.at(clip.time_of_step(0).unwrap());
        assert_eq!((shown.frame, shown.step), (2, Some(0)));
        assert_eq!(clip.time_of_step(9), None);
    }

    #[test]
    fn every_clip_is_there_with_its_frames() {
        for name in ["basics", "pill", "target", "card", "tools", "keys"] {
            let clip = super::clip(name).unwrap_or_else(|| panic!("no clip {name} in help/clips.json"));
            for frame in &clip.frames {
                assert!(crate::assets::help_file(&frame.image).is_some(), "{name}: no help/{}", frame.image);
            }
        }
        assert_eq!(super::clip("basics").unwrap().frames.iter().filter_map(|f| f.step).max(), Some(3));
    }
}
