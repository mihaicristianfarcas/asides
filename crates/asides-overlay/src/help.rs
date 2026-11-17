//! Help's demonstrations, and setup's How it works: the overlay's own pieces (`faces.rs`) on a plain
//! stage beside a small app drawn in code, and what happens to them in turn: a key pressed, the
//! pointer gliding to what it clicks or drags, what's said. Each beat is a scene; going into it, the
//! scene before gives way to it as the overlay's pieces come and go. A demonstration plays from its
//! first beat, round and round; the window is drawn every frame only while something moves, and else
//! again when the next change is due.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::Duration;

use asides_core::inspect::{Intent, PtRect};
use asides_core::keys;
use asides_core::overlay::{MARKER, Tool};
use gpui::{
    AnyElement, Div, FontWeight, HighlightStyle, Hsla, StyledText, TextRun, Window, canvas, div, prelude::*, px,
};

use crate::controls::{self, Radius, Theme, ease, hex};
use crate::faces::{
    self, Prim, Still, agent_segment, agents_track, card_cluster, card_field, card_surface, card_tool,
    choose_folder_content, folder_content, intent_dot, keys_sheet_face, label_at, list_hairline, list_heading,
    list_lines, list_panel, list_row_face, list_rows, marker_face, new_session_row, plain_glass, still_pill, talk_face,
};
use crate::style;
use crate::ui::{CARD_WIDTH, Keys, LIST_GAP};

type Point = (f32, f32);
/// A rectangle on the stage: x, y, width, height.
type Rect = (f32, f32, f32, f32);

/// How long each part of going to the next beat takes: a key pressed, the pointer gliding, a drag,
/// a click, the scenes giving way.
const KEY: Duration = Duration::from_millis(380);
const GLIDE: Duration = Duration::from_millis(620);
const DRAG: Duration = Duration::from_millis(900);
const CLICK: Duration = Duration::from_millis(300);
const FADE: Duration = Duration::from_millis(220);
/// A key pressed shows until this long after its beat is in, and pops in and fades out in `POP`.
const KEY_SHOWN: Duration = Duration::from_millis(1100);
const POP: Duration = Duration::from_millis(150);
/// Typed into a card, a character this often; said, a word.
const TYPE_EACH: Duration = Duration::from_millis(45);
const WORD_EACH: Duration = Duration::from_millis(190);
/// The pill's top on the stage.
const PILL_TOP: f32 = 12.0;
/// The pill's height, and how far left of its middle where the aside goes is (its buttons on either
/// side are always as wide: 5 + 28 + 28 + 8 before the name, 8 + 2 + 21 + 90 + 5 after).
const PILL_HEIGHT: f32 = 38.0;
const NAME_LEFT_OF_MIDDLE: f32 = 28.5;

/// A demonstration: its size (laid out that wide, in the middle of what it's given), where the app
/// is on its stage (and the agent's chat), and its beats.
pub struct Demo {
    pub size: (f32, f32),
    pub app: Option<Point>,
    pub chat: Option<Rect>,
    pub beats: Vec<Beat>,
}

#[derive(Clone, Default)]
pub struct Beat {
    /// How long it stays, once in (ms).
    pub hold: u64,
    /// What happens before it shows, in this order: a key pressed (the overlay's action, as in the
    /// settings, or `open` and `send`: the shortcut, or a key as it's named)...
    pub key: Option<String>,
    /// ...the pointer dragged along this (drawing `drag` as it goes)...
    pub path: Vec<Point>,
    pub drag: Option<Drag>,
    /// ...or going here (after a drag, once the beat is in)...
    pub pointer: Option<Point>,
    /// ...and clicking.
    pub click: bool,
    /// What's being said while it shows, pins' numbers as `[1]`, a word at a time.
    pub say: Option<&'static str>,
    /// The step of the basics it shows.
    pub step: Option<usize>,
    /// What's on the stage once it's in.
    pub scene: Scene,
}

/// What a drag draws as it goes: an area pinned or measured, or a line drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Pin,
    Measure,
    Sketch,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    /// The screen frozen under the pill, the microphone listening.
    pub frozen: bool,
    pub listening: bool,
    /// Where the aside goes, as the pill names it.
    pub target: &'static str,
    /// The pill's list open, and the session lit in it.
    pub list: Option<Option<usize>>,
    /// The tool, when it isn't Annotate.
    pub tool: Option<Tool>,
    /// What a click would pin, under the pointer.
    pub hover: Option<Rect>,
    pub pins: Vec<Pin>,
    /// A pin's card open: its pin's number, what's typed (typing in, a character at a time, when
    /// `typing`), what it asks for.
    pub card: Option<Card>,
    pub measured: Vec<Measured>,
    /// The aside typed into the agent's chat.
    pub sent: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub number: u32,
    pub at: Point,
    pub on: On,
    pub intent: Option<Intent>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum On {
    Element(Rect),
    Area(Rect),
    Sketch(Vec<Point>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub pin: u32,
    pub text: &'static str,
    pub typing: bool,
    pub intent: Option<Intent>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Measured {
    /// From the pointer, across (from x to x, at its y) and down (from y to y, at its x).
    Distance {
        at: Point,
        across: (f32, f32),
        down: (f32, f32),
    },
    Area(Rect),
}

/// The demonstration called `name`: `basics`, `target`, `card` or `tools`.
pub fn demo(name: &str) -> Option<&'static Demo> {
    static DEMOS: OnceLock<HashMap<&'static str, Demo>> = OnceLock::new();
    DEMOS
        .get_or_init(|| {
            HashMap::from([("basics", basics()), ("target", target()), ("card", card()), ("tools", tools())])
        })
        .get(name)
}

// --- The app on the stage ---

/// Its size, and where its parts are, from its top left.
const APP: (f32, f32) = (290.0, 218.0);
const APP_ROWS: Rect = (12.0, 40.0, 266.0, 68.0);
const APP_BUTTON: Rect = (16.0, 156.0, 92.0, 42.0);
/// Under the line above the total, and its right edge, inside.
const APP_RULE: f32 = 117.0;
const APP_TOTAL: Point = (246.0, 135.0);

fn on_app(app: Point, (x, y, w, h): Rect) -> Rect {
    (app.0 + x, app.1 + y, w, h)
}

/// A circle drawn by hand around `(x, y)`: a little past where it started.
fn circle_around((x, y): Point, (rx, ry): (f32, f32)) -> Vec<Point> {
    (0..=26)
        .map(|i| {
            let a = -2.4 + i as f32 / 24.0 * std::f32::consts::TAU;
            (x + rx * a.cos(), y + ry * a.sin())
        })
        .collect()
}

fn session(i: usize) -> &'static str {
    ["Claude Code (shop)", "Codex (api)", "OpenCode (blog)"][i]
}

// --- The demonstrations ---

/// Freeze, click what's wrong, say why, send: an app and the agent's chat beside it.
fn basics() -> Demo {
    let app = (20.0, 62.0);
    let button = on_app(app, APP_BUTTON);
    let rows = on_app(app, APP_ROWS);
    let click = (button.0 + button.2 - 12.0, button.1 + 9.0);
    let rest = (250.0, 40.0);
    let frozen = Scene { frozen: true, listening: true, target: session(0), ..Scene::default() };
    let pin1 = Pin { number: 1, at: click, on: On::Element(button), intent: None };
    let pin2 = Pin { number: 2, at: (rows.0, rows.1), on: On::Area(rows), intent: None };
    let pinned = Scene { pins: vec![pin1.clone()], ..frozen.clone() };
    let both = Scene { pins: vec![pin1, pin2], ..frozen.clone() };
    Demo {
        size: (536.0, 340.0),
        app: Some(app),
        chat: Some((324.0, 62.0, 192.0, 218.0)),
        beats: vec![
            Beat {
                hold: 900,
                pointer: Some(rest),
                step: Some(0),
                scene: Scene { target: session(0), ..Scene::default() },
                ..Beat::default()
            },
            Beat { hold: 1100, key: Some("open".into()), step: Some(0), scene: frozen.clone(), ..Beat::default() },
            Beat {
                hold: 380,
                pointer: Some(click),
                step: Some(1),
                scene: Scene { hover: Some(button), ..frozen },
                ..Beat::default()
            },
            Beat { hold: 650, pointer: Some(click), click: true, step: Some(1), scene: pinned, ..Beat::default() },
            Beat {
                hold: 700,
                path: vec![(rows.0, rows.1), (150.0, 140.0), (rows.0 + rows.2, rows.1 + rows.3)],
                drag: Some(Drag::Pin),
                step: Some(1),
                scene: both.clone(),
                ..Beat::default()
            },
            Beat {
                hold: 3400,
                pointer: Some(rest),
                say: Some("This button [1] wraps, and these rows [2] are cramped."),
                step: Some(2),
                scene: both,
                ..Beat::default()
            },
            Beat {
                hold: 3200,
                key: Some("send".into()),
                step: Some(3),
                scene: Scene { target: session(0), sent: true, ..Scene::default() },
                ..Beat::default()
            },
        ],
    }
}

/// Where it goes: the next session, then the list, a session picked in it.
fn target() -> Demo {
    let middle = 210.0;
    let name = (middle - NAME_LEFT_OF_MIDDLE, PILL_TOP + PILL_HEIGHT / 2.0);
    let rest = (392.0, 116.0);
    let scene = |target, list| Scene { frozen: true, listening: true, target, list, ..Scene::default() };
    Demo {
        size: (420.0, 424.0),
        app: None,
        chat: None,
        beats: vec![
            Beat { hold: 1100, pointer: Some(rest), scene: scene(session(0), None), ..Beat::default() },
            Beat { hold: 1300, key: Some("target".into()), scene: scene(session(1), None), ..Beat::default() },
            Beat { hold: 900, pointer: Some(name), scene: scene(session(1), Some(None)), ..Beat::default() },
            Beat {
                hold: 450,
                pointer: Some((name.0 + 20.0, list_row_middle(2))),
                scene: scene(session(1), Some(Some(2))),
                ..Beat::default()
            },
            Beat {
                hold: 1800,
                pointer: Some((name.0 + 20.0, list_row_middle(2))),
                click: true,
                scene: scene(session(2), None),
                ..Beat::default()
            },
        ],
    }
}

/// A pin's card: its number clicked, what's typed, what it is.
fn card() -> Demo {
    let app = (20.0, 16.0);
    let button = on_app(app, APP_BUTTON);
    let at = (button.0 + 8.0, button.1 + button.3 - 6.0);
    let pin = |intent| Pin { number: 1, at, on: On::Element(button), intent };
    // The pill left out: the card is what's shown.
    let open = |text, typing, intent| Scene {
        pins: vec![pin(intent)],
        card: Some(Card { pin: 1, text, typing, intent }),
        ..Scene::default()
    };
    let text = "Keep the label on one line";
    Demo {
        size: (536.0, 322.0),
        app: Some(app),
        chat: None,
        beats: vec![
            Beat {
                hold: 1000,
                pointer: Some((440.0, 70.0)),
                scene: Scene { pins: vec![pin(None)], ..Scene::default() },
                ..Beat::default()
            },
            Beat { hold: 450, pointer: Some(at), click: true, scene: open("", false, None), ..Beat::default() },
            Beat { hold: 1900, scene: open(text, true, None), ..Beat::default() },
            Beat {
                hold: 2100,
                key: Some(format!("{}2", keys::COMMAND)),
                scene: open(text, false, Some(Intent::Change)),
                ..Beat::default()
            },
        ],
    }
}

/// Measure (the distances from the pointer, then the size of what's dragged over), then Draw.
fn tools() -> Demo {
    let app = (40.0, 62.0);
    let button = on_app(app, APP_BUTTON);
    let at = (app.0 + 200.0, app.1 + 177.0);
    let distance = Measured::Distance {
        at,
        across: (button.0 + button.2, app.0 + APP.0 - 1.0),
        down: (app.1 + APP_RULE, app.1 + APP.1 - 1.0),
    };
    let ring = circle_around((app.0 + APP_TOTAL.0, app.1 + APP_TOTAL.1), (40.0, 17.0));
    let scene = |tool, measured: Vec<Measured>, pins| Scene {
        frozen: true,
        listening: true,
        target: session(0),
        tool,
        measured,
        pins,
        ..Scene::default()
    };
    let size = Measured::Area(button);
    let drawn = Pin { number: 1, at: ring[0], on: On::Sketch(ring.clone()), intent: None };
    Demo {
        size: (536.0, 330.0),
        app: Some(app),
        chat: None,
        beats: vec![
            Beat { hold: 900, pointer: Some((440.0, 250.0)), scene: scene(None, vec![], vec![]), ..Beat::default() },
            Beat {
                hold: 600,
                key: Some("measure".into()),
                scene: scene(Some(Tool::Measure), vec![], vec![]),
                ..Beat::default()
            },
            Beat {
                hold: 1500,
                pointer: Some(at),
                scene: scene(Some(Tool::Measure), vec![distance], vec![]),
                ..Beat::default()
            },
            Beat {
                hold: 1300,
                path: vec![(button.0 - 7.0, button.1 - 6.0), (button.0 + button.2 + 7.0, button.1 + button.3 + 6.0)],
                drag: Some(Drag::Measure),
                pointer: Some((button.0 + button.2 + 40.0, button.1 + 30.0)),
                scene: scene(Some(Tool::Measure), vec![size.clone()], vec![]),
                ..Beat::default()
            },
            Beat {
                hold: 500,
                key: Some("draw".into()),
                scene: scene(Some(Tool::Draw), vec![size.clone()], vec![]),
                ..Beat::default()
            },
            Beat {
                hold: 2100,
                path: ring,
                drag: Some(Drag::Sketch),
                scene: scene(Some(Tool::Draw), vec![size], vec![drawn]),
                ..Beat::default()
            },
        ],
    }
}

// --- Playing ---

/// What a demonstration shows at a moment.
#[derive(Debug, PartialEq)]
pub struct Shown<'a> {
    /// The beat in front, how far it has come in (1: all of it), and the one before it.
    pub beat: usize,
    pub fade: f32,
    pub under: usize,
    /// How long the beat in front has been in.
    pub held: Duration,
    /// A beat's drag going on: which, and how far along.
    pub dragging: Option<(usize, f32)>,
    pub pointer: Option<Point>,
    /// The pointer held down (dragging, clicking).
    pub pressed: bool,
    /// A click's ring, spreading out: how far (0 to 1).
    pub ripple: Option<f32>,
    /// The key pressed, and how much of it shows.
    pub key: Option<(&'a str, f32)>,
    pub say: Option<&'a str>,
    pub step: Option<usize>,
    /// Since it started playing.
    pub time: Duration,
    /// Something's moving: draw it again next frame. Else, when something next changes.
    pub moving: bool,
    pub next: Option<Duration>,
}

/// Going into a beat: how long each part takes.
struct Parts {
    key: Duration,
    reach: Duration,
    drag: Duration,
    click: Duration,
    hold: Duration,
    /// After a drag, the pointer going on to `pointer` (as the beat stays).
    after: Duration,
    /// Something keeps moving while it's held (typing, talking): until then.
    lively: Duration,
}

impl Parts {
    /// Until the beat starts coming in.
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

fn length(a: Point, b: Point) -> f32 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// Where along `path` a pointer is, `t` of the way (by length).
fn along(path: &[Point], t: f32) -> Point {
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

/// `path` as far as `t` of the way along it.
fn path_to(path: &[Point], t: f32) -> Vec<Point> {
    let total: f32 = path.windows(2).map(|w| length(w[0], w[1])).sum();
    let mut left = total * t.clamp(0.0, 1.0);
    let mut drawn = path.first().copied().into_iter().collect::<Vec<_>>();
    for w in path.windows(2) {
        let part = length(w[0], w[1]);
        if left >= part {
            drawn.push(w[1]);
            left -= part;
        } else {
            if part > 0.0 {
                drawn.push(lerp(w[0], w[1], left / part));
            }
            break;
        }
    }
    drawn
}

impl Beat {
    /// Where the pointer goes first: where a drag starts, or where it's going.
    fn target(&self) -> Option<Point> {
        self.path.first().copied().or(self.pointer)
    }

    /// Where the pointer is once the beat is in.
    fn end(&self) -> Option<Point> {
        self.pointer.or(self.path.last().copied())
    }

    /// How long something keeps moving once it's in: a card's text typing, the words said.
    fn lively(&self) -> Duration {
        let typing = match &self.scene.card {
            Some(card) if card.typing => TYPE_EACH * card.text.chars().count() as u32,
            _ => Duration::ZERO,
        };
        // Talking: the microphone's level moves the whole time.
        let talking = if self.say.is_some() { Duration::from_millis(self.hold) } else { Duration::ZERO };
        typing.max(talking)
    }
}

impl Demo {
    /// Where the pointer is once beat `i` is in (it stays where it was, without one of its own).
    fn rest(&self, i: usize) -> Option<Point> {
        let n = self.beats.len();
        (0..n).find_map(|back| self.beats[(i + n - back) % n].end())
    }

    fn parts(&self, i: usize) -> Parts {
        let n = self.beats.len();
        let b = &self.beats[i];
        let from = self.rest((i + n - 1) % n);
        let hold = Duration::from_millis(b.hold);
        let moves = |a: Option<Point>, b: Option<Point>| matches!((a, b), (Some(a), Some(b)) if apart(a, b));
        let after = b.path.last().copied().filter(|_| moves(b.path.last().copied(), b.pointer));
        Parts {
            key: if b.key.is_some() { KEY } else { Duration::ZERO },
            reach: if moves(from, b.target()) { GLIDE } else { Duration::ZERO },
            drag: if b.path.len() > 1 { DRAG } else { Duration::ZERO },
            click: if b.click { CLICK } else { Duration::ZERO },
            hold,
            after: if after.is_some() { GLIDE.min(hold) } else { Duration::ZERO },
            lively: b.lively().min(hold),
        }
    }

    /// When, after it started playing, it starts going into step `step`'s first beat (to watch
    /// that step).
    pub fn time_of_step(&self, step: usize) -> Option<Duration> {
        let beat = self.beats.iter().position(|b| b.step == Some(step))?;
        let parts: Vec<Parts> = (0..self.beats.len()).map(|i| self.parts(i)).collect();
        let cycle: Duration = parts.iter().map(Parts::total).sum();
        let start = parts[0].before() + FADE;
        let going: Duration = parts[..beat].iter().map(Parts::total).sum();
        Some(Duration::from_nanos(((going + cycle - start).as_nanos() % cycle.as_nanos().max(1)) as u64))
    }

    /// What it shows `t` after it started playing: beat 0 just in, then each in turn, round again.
    pub fn at(&self, t: Duration) -> Shown<'_> {
        let n = self.beats.len();
        let parts: Vec<Parts> = (0..n).map(|i| self.parts(i)).collect();
        let cycle: Duration = parts.iter().map(Parts::total).sum();
        // From beat 0 just in.
        let start = parts[0].before() + FADE;
        let mut left = Duration::from_nanos(((t + start).as_nanos() % cycle.as_nanos().max(1)) as u64);
        for (i, p) in parts.iter().enumerate() {
            if left < p.total() {
                return self.going_into(i, p, left, t);
            }
            left -= p.total();
        }
        self.going_into(0, &parts[0], Duration::ZERO, t)
    }

    /// Beat `i` coming in, `s` into it.
    fn going_into(&self, i: usize, p: &Parts, s: Duration, time: Duration) -> Shown<'_> {
        let n = self.beats.len();
        let prev = (i + n - 1) % n;
        let (b, before) = (&self.beats[i], &self.beats[prev]);
        let from = self.rest(prev);
        let target = b.target().or(from);
        // The parts' ends, from the start.
        let reached = p.key + p.reach;
        let dragged = reached + p.drag;
        let clicked = dragged + p.click;
        let faded = clicked + FADE;
        let held = faded + p.hold;
        let part = |from: Duration, len: Duration| ms(s.saturating_sub(from)) / ms(len).max(1.0);
        let (mut pointer, mut pressed, mut ripple, mut dragging) = (from, false, None, None);
        if s >= p.key && s < reached {
            pointer = match (from, target) {
                (Some(a), Some(b)) => Some(lerp(a, b, ease_in_out(part(p.key, p.reach)))),
                _ => target,
            };
        } else if s >= reached && s < dragged {
            let done = ease_in_out(part(reached, p.drag));
            (pointer, pressed, dragging) = (Some(along(&b.path, done)), true, Some((i, done)));
        } else if s >= dragged && s < clicked {
            let spread = part(dragged, p.click);
            (pointer, pressed, ripple) = (target, spread < 0.5, Some(spread));
        } else if s >= clicked {
            let done = b.path.last().copied().or(target);
            pointer = match (done, b.pointer) {
                (Some(a), Some(c)) if p.after > Duration::ZERO => Some(lerp(a, c, ease_in_out(part(faded, p.after)))),
                _ => b.end().or(from),
            };
        }
        let fade = if s < clicked { 0.0 } else { ease(part(clicked, FADE)) };
        let showing = if s < clicked { before } else { b };
        // The step coming, as soon as it does.
        let step = b.step.or(showing.step);
        // The key pressed, from when it's pressed until a while after the beat is in.
        let shown_until = (faded + KEY_SHOWN).min(held);
        let key = b.key.as_deref().filter(|_| s < shown_until).map(|key| {
            let into = part(Duration::ZERO, POP).min(1.0);
            let out = (ms(shown_until.saturating_sub(s)) / ms(POP)).min(1.0);
            (key, into.min(out))
        });
        // Moving: a key popping in or fading out, the pointer, a click, the scenes giving way, the
        // pointer going on after a drag, typing or talking.
        let spans = [
            (Duration::ZERO, if b.key.is_some() { POP } else { Duration::ZERO }),
            (p.key, faded),
            (faded, faded + p.after.max(p.lively)),
            (if b.key.is_some() { shown_until.saturating_sub(POP) } else { held }, shown_until),
        ];
        let moving = spans.iter().any(|&(a, b)| s >= a && s < b);
        let next = spans.iter().map(|&(a, _)| a).filter(|&a| a > s).chain([held]).min().map(|at| at - s);
        Shown {
            beat: if s < clicked { prev } else { i },
            fade: if s < clicked { 1.0 } else { fade },
            under: prev,
            held: if s < clicked { held_before(self, prev) } else { s.saturating_sub(faded) },
            dragging,
            pointer,
            pressed,
            ripple,
            key,
            say: showing.say,
            step,
            time,
            moving,
            next: if moving { None } else { next },
        }
    }
}

/// How long beat `i` has been in while the next one is on its way: all of it (its typing done).
fn held_before(demo: &Demo, i: usize) -> Duration {
    Duration::from_millis(demo.beats[i].hold)
}

// --- Drawing ---

/// What a demonstration needs from the window: its theme, the measuring color, the keys as set
/// (`key` names one as the window shows it), and the shortcuts.
pub struct Look {
    pub t: Theme,
    pub accent: Hsla,
    pub key: Box<dyn Fn(&str) -> String>,
    pub keys: Keys,
    pub freeze: String,
    pub send: String,
}

/// The stage: a plain screen, as wide as it's given.
fn stage(width: f32, height: f32, t: Theme) -> Div {
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .h(px(height))
        .rounded(px(Radius::CARD))
        .border_1()
        .border_color(t.card_stroke)
        .bg(t.control_hover)
        .overflow_hidden()
}

/// How wide `text` is in the pill's name (12.5 points, medium).
fn name_width(text: &str, window: &Window) -> f32 {
    let font = gpui::Font { weight: FontWeight::MEDIUM, ..gpui::font(controls::font()) };
    let run = TextRun {
        len: text.len(),
        font,
        color: gpui::black(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    f32::from(window.text_system().shape_line(text.to_string().into(), px(12.5), &[run], None).width)
}

/// How much of what's in `from` and `to` shows, `f` of the way from one to the other.
fn between(from: bool, to: bool, f: f32) -> f32 {
    match (from, to) {
        (true, true) => 1.0,
        (false, false) => 0.0,
        (false, true) => f,
        (true, false) => 1.0 - f,
    }
}

/// A pin's color, as the overlay paints it (`style::annotation_color`).
fn pin_ink(pin: &Pin, accent: Hsla) -> Hsla {
    match (pin.intent, &pin.on) {
        (Some(intent), _) => hex(style::intent_color(intent), 1.0),
        (None, On::Sketch(_)) => accent,
        (None, On::Area(_)) => hex(style::ANNOTATION_GREEN, 1.0),
        (None, On::Element(_)) => hex(style::ANNOTATION_BLUE, 1.0),
    }
}

fn rect((x, y, w, h): Rect, dx: f32) -> PtRect {
    PtRect::new((x + dx) as f64, y as f64, w as f64, h as f64)
}

fn dot((x, y): Point, dx: f32) -> (f64, f64) {
    ((x + dx) as f64, y as f64)
}

fn faded(c: Hsla, by: f32) -> Hsla {
    Hsla { a: c.a * by, ..c }
}

/// `demo` as `shown`, `width` wide.
pub fn view(demo: &Demo, shown: &Shown, width: f32, look: &Look, window: &Window) -> AnyElement {
    let t = look.t;
    let (from, to) = (&demo.beats[shown.under].scene, &demo.beats[shown.beat].scene);
    let f = shown.fade;
    let dx = ((width - demo.size.0) / 2.0).max(0.0);
    let mut prims: Vec<Prim> = Vec::new();
    let mut labels: Vec<AnyElement> = Vec::new();
    let mut markers: Vec<AnyElement> = Vec::new();
    let label = |text: String, (x, y): Point, shown: f32| {
        div()
            .absolute()
            .inset_0()
            .opacity(shown)
            .child(label_at(plain_glass(t, 5.), t, text, ((x + dx) as f64, y as f64)))
            .into_any_element()
    };
    let size_of = |(_, _, w, h): Rect| format!("{} × {}", w.round(), h.round());

    // What a click would pin, under the pointer.
    for (hover, shows) in [(from.hover, between(true, false, f)), (to.hover, f)] {
        if let Some(r) = hover.filter(|_| shows > 0.0) {
            prims.push(Prim::Fill(rect(r, dx), hex(style::ANNOTATION_BLUE, 0.12 * shows)));
            prims.push(Prim::Border(rect(r, dx), hex(style::ANNOTATION_BLUE, shows), 1.5, false));
        }
    }

    // Measured: the distances from the pointer, and sizes held; those in both scenes stay, the others
    // come or go.
    let staying = to.measured.iter().map(|m| (m, if from.measured.contains(m) { 1.0 } else { f }));
    let going = from.measured.iter().filter(|m| !to.measured.contains(m)).map(|m| (m, 1.0 - f));
    for (m, shows) in staying.chain(going) {
        if shows <= 0.0 {
            continue;
        }
        let c = faded(look.accent, shows);
        match *m {
            Measured::Distance { at: (x, y), across: (x0, x1), down: (y0, y1) } => {
                let thin = 1.0;
                prims.push(Prim::Fill(rect((x0, y, x1 - x0, thin), dx), c));
                prims.push(Prim::Fill(rect((x, y0, thin, y1 - y0), dx), c));
                for edge in [x0, x1] {
                    prims.push(Prim::Fill(rect((edge, y - 4.0, thin, 8.0), dx), c));
                }
                for edge in [y0, y1] {
                    prims.push(Prim::Fill(rect((x - 4.0, edge, 8.0, thin), dx), c));
                }
                let text = format!("{} × {}", (x1 - x0).round(), (y1 - y0).round());
                let w = text.chars().count() as f32 * 6.6 + 12.0;
                labels.push(label(text, (x + 14.0 + w / 2.0, y + 22.0), shows));
            }
            Measured::Area(r) => {
                prims.push(Prim::Fill(rect(r, dx), faded(look.accent, style::AREA_TINT * shows)));
                prims.push(Prim::Border(rect(r, dx), c, 1.0, false));
                let text = size_of(r);
                let (x, y) = style::area_label(rect(r, 0.0), text.chars().count() as f64 * 6.6 + 12.0, 1e4);
                labels.push(label(text, (x as f32, y as f32), shows));
            }
        }
    }

    // Pins: those in both scenes stay, the others come or go.
    let mut pins: Vec<(&Pin, f32)> = Vec::new();
    for p in &to.pins {
        let stays = from.pins.iter().any(|q| q.number == p.number);
        pins.push((p, if stays { 1.0 } else { f }));
    }
    for p in from.pins.iter().filter(|p| !to.pins.iter().any(|q| q.number == p.number)) {
        pins.push((p, 1.0 - f));
    }
    for &(pin, shows) in pins.iter().filter(|(_, shows)| *shows > 0.0) {
        // Its color changing (what it asks for): the new one as the beat comes in.
        let ink = pin_ink(pin, look.accent);
        match &pin.on {
            On::Element(r) => prims.push(Prim::Border(rect(*r, dx), faded(ink, 0.9 * shows), 1.5, false)),
            On::Area(r) => {
                prims.push(Prim::Fill(rect(*r, dx), faded(ink, 0.1 * shows)));
                prims.push(Prim::Border(rect(*r, dx), faded(ink, shows), 1.5, true));
            }
            On::Sketch(points) => prims.push(Prim::Stroke(
                points.iter().map(|p| dot(*p, dx)).collect(),
                style::DRAW_WIDTH,
                faded(ink, shows),
            )),
        }
        let (x, y) = pin.at;
        let half = MARKER as f32 / 2.0;
        markers.push(
            marker_face(pin.number, ink)
                .absolute()
                .left(px(x + dx - half))
                .top(px(y - half))
                .opacity(shows)
                .into_any_element(),
        );
    }

    // A drag going on: what it draws, as far as it's gone.
    if let Some((i, done)) = shown.dragging {
        let beat = &demo.beats[i];
        let start = beat.path.first().copied().unwrap_or_default();
        let now = along(&beat.path, done);
        let area = (start.0.min(now.0), start.1.min(now.1), (now.0 - start.0).abs(), (now.1 - start.1).abs());
        match beat.drag {
            Some(Drag::Sketch) => {
                prims.push(Prim::Stroke(
                    path_to(&beat.path, done).into_iter().map(|p| dot(p, dx)).collect(),
                    style::DRAW_WIDTH,
                    look.accent,
                ));
            }
            Some(kind) => {
                let (fill, line, dashed) = match kind {
                    Drag::Pin => (hex(style::ANNOTATION_GREEN, 0.1), hex(style::ANNOTATION_GREEN, 1.0), true),
                    _ => (faded(look.accent, style::AREA_TINT), look.accent, false),
                };
                prims.push(Prim::Fill(rect(area, dx), fill));
                prims.push(Prim::Border(rect(area, dx), line, if dashed { 1.5 } else { 1.0 }, dashed));
                let text = size_of(area);
                let (x, y) = style::area_label(rect(area, 0.0), text.chars().count() as f64 * 6.6 + 12.0, 1e4);
                labels.push(label(text, (x as f32, y as f32), 1.0));
            }
            None => {}
        }
    }

    let app = demo.app.map(|(x, y)| app_view(t).absolute().left(px(x + dx)).top(px(y)));
    let chat = demo.chat.map(|r| chat_view(r, dx, between(from.sent, to.sent, f), shown, demo, t));
    let painted = canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let (ox, oy) = (f64::from(bounds.origin.x), f64::from(bounds.origin.y));
            let shift = |r: PtRect| PtRect::new(r.x + ox, r.y + oy, r.width, r.height);
            let shift_all = |points: Vec<(f64, f64)>| points.into_iter().map(|(x, y)| (x + ox, y + oy)).collect();
            for prim in prims {
                faces::paint(
                    match prim {
                        Prim::Fill(r, c) => Prim::Fill(shift(r), c),
                        Prim::Border(r, c, w, dashed) => Prim::Border(shift(r), c, w, dashed),
                        Prim::Stroke(points, w, c) => Prim::Stroke(shift_all(points), w, c),
                        Prim::Polygon(points, c) => Prim::Polygon(shift_all(points), c),
                    },
                    window,
                );
            }
        },
    )
    .absolute()
    .size_full();
    let card = card_view(from, to, f, shown, dx, look);
    let pill = pill_view(demo, from, to, f, shown, dx, look, window);
    let pointer = pointer_view(shown, to, dx);
    let captions = captions_view(shown, to, width, look);
    stage(width, demo.size.1, t)
        .children(app)
        .children(chat)
        .child(painted)
        .children(labels)
        .children(markers)
        .children(card)
        .children(pill)
        .children(pointer)
        .child(captions)
        .into_any_element()
}

/// The app on the stage: an order's summary, its total, and the button to pay.
fn app_view(t: Theme) -> Div {
    let (sand, sage) =
        if t.dark { (hex(0x8c7d62, 1.0), hex(0x5f7a5a, 1.0)) } else { (hex(0xe6dcc6, 1.0), hex(0xa9c2a2, 1.0)) };
    let surface = if t.dark { hex(0x262523, 1.0) } else { hex(0xffffff, 1.0) };
    let row = |top: f32, swatch: Hsla, name: &'static str, price: &'static str| {
        div()
            .absolute()
            .left(px(16.))
            .top(px(top))
            .w(px(APP.0 - 32.))
            .h(px(26.))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(12.5))
            .child(div().flex_none().size(px(26.)).rounded(px(6.)).bg(swatch))
            .child(div().flex_1().child(name))
            .child(div().text_color(t.secondary).child(price))
    };
    let (bx, by, bw, bh) = APP_BUTTON;
    // Its ink inverted: dark on a light page, light on a dark one.
    let (button, on_button) =
        if t.dark { (hex(0xedebe7, 1.0), hex(0x1c1b19, 1.0)) } else { (hex(0x2b2a27, 1.0), hex(0xffffff, 1.0)) };
    div()
        .w(px(APP.0))
        .h(px(APP.1))
        .rounded(px(Radius::CARD))
        .border_1()
        .border_color(t.card_stroke)
        .bg(surface)
        .shadow(controls::lift(if t.dark { 0.25 } else { 0.06 }))
        .text_color(t.primary)
        .child(
            div()
                .absolute()
                .left(px(16.))
                .top(px(12.))
                .text_size(px(13.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Order summary"),
        )
        .child(row(44., sand, "Linen tote", "$48.00"))
        .child(row(78., sage, "Notebook × 2", "$24.00"))
        .child(div().absolute().left(px(16.)).top(px(APP_RULE - 1.)).w(px(APP.0 - 32.)).h(px(1.)).bg(t.hairline))
        .child(
            div()
                .absolute()
                .left(px(16.))
                .top(px(124.))
                .w(px(APP.0 - 32.))
                .flex()
                .justify_between()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Total")
                .child("$72.00"),
        )
        .child(
            div()
                .absolute()
                .left(px(bx))
                .top(px(by))
                .w(px(bw))
                .h(px(bh))
                .rounded(px(8.))
                .bg(button)
                .text_color(on_button)
                .text_size(px(12.))
                .line_height(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .child("Pay now ·")
                .child("$72.00"),
        )
}

/// The agent's chat beside the app: what was said before, and the aside, typed in once it's sent
/// (`sent`, how much of it shows).
fn chat_view((x, y, w, h): Rect, dx: f32, sent: f32, shown: &Shown, demo: &Demo, t: Theme) -> AnyElement {
    let surface = if t.dark { hex(0x1f1e1c, 1.0) } else { hex(0xffffff, 1.0) };
    let mono = |text: &'static str, color: Hsla| div().text_color(color).child(text);
    let blue = hex(style::ANNOTATION_BLUE, 1.0);
    let green = hex(style::ANNOTATION_GREEN, 1.0);
    let said = demo.beats.iter().find_map(|b| b.say).unwrap_or_default();
    let aside = div()
        .opacity(sent)
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(div().child(said_text(&format!("› [Image #1] {said}"), usize::MAX, blue, green, t.primary)))
        .child(mono("  ### 1. button \"Pay now · $72.00\"", t.secondary))
        .child(mono("  ### 2. list \"Items\"", t.secondary));
    let _ = shown;
    div()
        .absolute()
        .left(px(x + dx))
        .top(px(y))
        .w(px(w))
        .h(px(h))
        .rounded(px(Radius::CARD))
        .border_1()
        .border_color(t.card_stroke)
        .bg(surface)
        .shadow(controls::lift(if t.dark { 0.25 } else { 0.06 }))
        .overflow_hidden()
        .flex()
        .flex_col()
        .child(
            div()
                .flex_none()
                .h(px(28.))
                .px(px(12.))
                .flex()
                .items_center()
                .border_b_1()
                .border_color(t.hairline)
                .text_size(px(11.))
                .text_color(t.secondary)
                .child("Claude Code · shop"),
        )
        .child(
            div()
                .flex_1()
                .p(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .font_family(controls::mono_font())
                .text_size(px(10.5))
                .line_height(px(15.))
                .child(mono("› Run the checkout tests", t.secondary))
                .child(mono("● All 14 tests pass.", t.primary))
                .child(
                    div()
                        .relative()
                        .child(aside)
                        // The prompt, waiting, until the aside is in.
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .opacity(1.0 - sent)
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .child(mono("›", t.secondary))
                                .child(div().w(px(6.)).h(px(13.)).bg(t.tertiary)),
                        ),
                ),
        )
        .into_any_element()
}

/// The pill at the top of the stage, as it comes in or goes, its list or the keys under it.
#[allow(clippy::too_many_arguments)]
fn pill_view(
    demo: &Demo,
    from: &Scene,
    to: &Scene,
    f: f32,
    shown: &Shown,
    dx: f32,
    look: &Look,
    window: &Window,
) -> Option<AnyElement> {
    let t = look.t;
    let shows = between(from.frozen, to.frozen, f);
    if shows <= 0.0 {
        return None;
    }
    // Where it goes, the name easing from one to the other.
    let target = match from.target == to.target || f >= 1.0 {
        true => faces::target_name(Some(to.target.into())).into_any_element(),
        false => {
            let (a, b) = (name_width(from.target, window), name_width(to.target, window));
            let name = |text: &'static str, opacity: f32| {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .opacity(opacity)
                    .child(div().flex_none().whitespace_nowrap().child(text))
            };
            div()
                .relative()
                .flex_none()
                .w(px(a + (b - a) * f))
                .h(px(20.))
                .overflow_hidden()
                .child(name(from.target, 1.0 - f))
                .child(name(to.target, f))
                .into_any_element()
        }
    };
    // Talking, the microphone fills with how loud: a voice's rise and fall.
    let level = match shown.say {
        Some(_) if to.listening => {
            let s = shown.time.as_secs_f32();
            0.18 + 0.5 * ((s * 7.3).sin() * (s * 2.1 + 1.0).sin()).abs()
        }
        _ => 0.0,
    };
    let tool = match (from.tool, to.tool) {
        (None, None) => None,
        (Some(a), None) => Some((a, 1.0 - f)),
        (_, Some(b)) => Some((b, if from.tool.is_some() { 1.0 } else { f })),
    };
    let pill = still_pill(
        Still {
            help: false,
            listening: to.listening || from.listening,
            level,
            target,
            list_open: to.list.is_some() && f > 0.5 || from.list.is_some() && f <= 0.5,
            tool,
        },
        t,
    );
    let list = match (from.list, to.list) {
        (None, None) => None,
        (a, b) => Some((list_view(b.or(a).flatten(), to.target, t), between(a.is_some(), b.is_some(), f))),
    };
    let below = list.map(|(list, shown)| {
        div()
            .absolute()
            .left_0()
            .top(px(PILL_HEIGHT + LIST_GAP as f32 - (1.0 - shown) * 6.0))
            .opacity(shown)
            .child(list)
    });
    let _ = demo;
    Some(
        div()
            .absolute()
            .left(px(dx))
            .top(px(PILL_TOP - (1.0 - shows) * 6.0))
            .w(px(demo.size.0))
            .flex()
            .justify_center()
            .opacity(shows)
            .child(div().relative().child(pill).children(below))
            .into_any_element(),
    )
}

/// The middle of the pill's list's row for session `i`, on the stage: under the pill, its border and
/// its rows' room, the Sessions heading, and the sessions before it (two lines each).
fn list_row_middle(i: usize) -> f32 {
    let (heading, session) = (26.8, 46.8);
    PILL_TOP + PILL_HEIGHT + LIST_GAP as f32 + 1.0 + 4.0 + heading + session * i as f32 + session / 2.0
}

/// The pill's list as it lists three sessions (`lit`, the one the pointer's on; `chosen`'s checked),
/// two directories and History only.
fn list_view(lit: Option<usize>, chosen: &str, t: Theme) -> AnyElement {
    let apps: [&str; 3] = match () {
        _ if cfg!(target_os = "macos") => ["Terminal", "Ghostty", "Diri"],
        _ if cfg!(windows) => ["Windows Terminal", "Windows Terminal", "Console"],
        _ => ["GNOME Terminal", "Ghostty", "Konsole"],
    };
    let home = if cfg!(windows) { "C:\\Users\\me\\code" } else { "~/Developer" };
    let separator = if cfg!(windows) { "\\" } else { "/" };
    let mut rows: Vec<AnyElement> = vec![list_heading("Sessions", t).into_any_element()];
    for (i, app) in apps.into_iter().enumerate() {
        let name = session(i);
        let lines = list_lines(name.into(), Some(app.into()), t);
        rows.push(list_row_face(name == chosen, lit == Some(i), true, lines, t).into_any_element());
    }
    rows.push(list_hairline(t));
    let agents = ["Claude Code", "Codex", "OpenCode"];
    let track = agents_track(t).children(agents.into_iter().enumerate().map(|(i, name)| {
        agent_segment(name, i == 0, t).when(i == 0, |d| d.bg(t.selected).border_color(t.selected_stroke))
    }));
    rows.push(new_session_row(track, t));
    for name in ["shop", "api"] {
        let content = folder_content(name.into(), format!("{home}{separator}{name}"), t);
        rows.push(list_row_face(false, false, true, content, t).into_any_element());
    }
    rows.push(list_row_face(false, false, true, choose_folder_content(t), t).into_any_element());
    rows.push(list_hairline(t));
    let history = list_lines("History only".into(), Some("Send it later from the asides window".into()), t);
    rows.push(list_row_face(false, false, true, history, t).into_any_element());
    list_panel(t).child(list_rows().children(rows)).into_any_element()
}

/// A pin's card, under its number, as it opens or closes: what's typed, typing in.
fn card_view(from: &Scene, to: &Scene, f: f32, shown: &Shown, dx: f32, look: &Look) -> Option<AnyElement> {
    let t = look.t;
    let open = between(from.card.is_some(), to.card.is_some(), f);
    let card = to.card.as_ref().or(from.card.as_ref())?;
    if open <= 0.0 {
        return None;
    }
    let pin = to.pins.iter().chain(&from.pins).find(|p| p.number == card.pin)?;
    let typed = match card.typing {
        true => card.text.chars().take((shown.held.as_millis() / TYPE_EACH.as_millis().max(1)) as usize).collect(),
        false => card.text.to_string(),
    };
    let dots = Intent::ALL.map(|intent| intent_dot(intent, card.intent == Some(intent), t).into_any_element());
    let tools = [Tool::Draw, Tool::Measure].map(|tool| card_tool(tool, false, t).into_any_element());
    let row = vec![
        card_field(&typed, true, t),
        card_cluster(dots),
        card_cluster(tools),
        talk_face(false, t).into_any_element(),
    ];
    let half = MARKER as f32 / 2.0;
    Some(
        card_surface(plain_glass(t, 12.), CARD_WIDTH as f32, row, t)
            .absolute()
            .left(px(pin.at.0 + dx + half + 6.0))
            .top(px(pin.at.1 + half + 6.0 - 4.0 * (1.0 - open)))
            .opacity(open)
            .into_any_element(),
    )
}

/// The pointer, its tip on the point (at 5, 3 of its 24): an arrow, or a crosshair while measuring or
/// drawing; pressed, a soft dot under it; a click's ring spreading out.
fn pointer_view(shown: &Shown, scene: &Scene, dx: f32) -> Vec<AnyElement> {
    let Some((x, y)) = shown.pointer else { return Vec::new() };
    let (x, y) = (x + dx, y);
    let blue = hex(style::GUIDE, 1.0);
    let mut drawn = Vec::new();
    if let Some(spread) = shown.ripple {
        let r = 5.0 + 17.0 * ease(spread);
        drawn.push(
            div()
                .absolute()
                .left(px(x - r))
                .top(px(y - r))
                .size(px(2.0 * r))
                .rounded_full()
                .border_2()
                .border_color(blue.opacity(1.0 - spread))
                .into_any_element(),
        );
    }
    if shown.pressed {
        drawn.push(
            div()
                .absolute()
                .left(px(x - 8.0))
                .top(px(y - 8.0))
                .size(px(16.))
                .rounded_full()
                .bg(blue.opacity(0.35))
                .into_any_element(),
        );
    }
    let cross = matches!(scene.tool, Some(Tool::Measure | Tool::Draw));
    let (size, tip) = if cross { (20.0, (10.0, 10.0)) } else { (22.0, (5.0 * 22.0 / 24.0, 3.0 * 22.0 / 24.0)) };
    let (edge, ink) = if cross { ("cross-edge", "cross") } else { ("pointer-edge", "pointer") };
    let layer = |name: &str, color| div().absolute().top_0().left_0().child(controls::icon(name, size, color));
    drawn.push(
        div()
            .absolute()
            .left(px(x - tip.0))
            .top(px(y - tip.1))
            .size(px(size))
            .child(layer(edge, hex(0xffffff, 1.0)))
            .child(layer(ink, hex(0x000000, 1.0)))
            .into_any_element(),
    );
    drawn
}

/// The key pressed, and what's said, along the bottom, a word at a time, each pin's number in its
/// color.
fn captions_view(shown: &Shown, scene: &Scene, width: f32, look: &Look) -> AnyElement {
    let t = look.t;
    let ink = |number: u32| {
        scene
            .pins
            .iter()
            .find(|p| p.number == number)
            .map_or(hex(style::ANNOTATION_BLUE, 1.0), |p| pin_ink(p, look.accent))
    };
    // A key pressed takes the caption's place: the caption fades as it comes (stacked, it covered
    // the app).
    let key_shows = shown.key.map_or(0.0, |(_, shows)| shows);
    let say = shown.say.filter(|_| key_shows < 1.0).map(|text| {
        let words = (shown.held.as_millis() / WORD_EACH.as_millis().max(1)) as usize + 1;
        controls::floating(t)
            .flex_row()
            .opacity(1.0 - key_shows)
            .max_w(px(width * 0.86))
            .px(px(12.))
            .py(px(7.))
            .rounded(px(10.))
            .items_start()
            .gap(px(8.))
            .text_size(px(13.5))
            .line_height(px(19.))
            .child(div().pt(px(3.)).child(controls::icon("mic", 13., hex(style::RECORDING_SOFT, 1.0))))
            .child(div().flex_1().min_w_0().child(said_text(text, words, ink(1), ink(2), t.primary)))
    });
    let key = shown.key.map(|(name, shows)| {
        controls::floating(t)
            .flex_row()
            .opacity(shows)
            .px(px(12.))
            .py(px(5.))
            .rounded(px(9.))
            .text_size(px(15.))
            .font_weight(FontWeight::SEMIBOLD)
            .child((look.key)(name))
    });
    let at_bottom =
        |child: Div| div().absolute().bottom(px(14.)).left_0().right_0().flex().justify_center().child(child);
    div()
        .absolute()
        .top_0()
        .bottom_0()
        .left_0()
        .right_0()
        .children(say.map(at_bottom))
        .children(key.map(at_bottom))
        .into_any_element()
}

/// What's said, its first `words` shown (the rest laid out, unseen: nothing moves as they come),
/// pins' numbers bold in their colors (`one`, `two`).
fn said_text(text: &str, words: usize, one: Hsla, two: Hsla, ink: Hsla) -> StyledText {
    let mut highlights = Vec::new();
    // Where the words shown end.
    let shown_to = text.match_indices(' ').map(|(i, _)| i).nth(words.saturating_sub(1)).unwrap_or(text.len());
    let mut from = 0;
    while let Some(open) = text[from..].find('[').map(|i| from + i) {
        let Some(close) = text[open..].find(']').map(|i| open + i) else { break };
        let color = if &text[open..=close] == "[2]" { two } else { one };
        highlights.push((
            open..close + 1,
            HighlightStyle { color: Some(color), font_weight: Some(FontWeight::BOLD), ..Default::default() },
        ));
        from = close + 1;
    }
    let mut styles: Vec<(std::ops::Range<usize>, HighlightStyle)> = Vec::new();
    let mut at = 0;
    for (range, style) in highlights {
        if range.start > at {
            styles.push((at..range.start, HighlightStyle { color: Some(ink), ..Default::default() }));
        }
        styles.push((range.clone(), style));
        at = range.end;
    }
    if at < text.len() {
        styles.push((at..text.len(), HighlightStyle { color: Some(ink), ..Default::default() }));
    }
    // Not said yet: unseen (a highlight's color blends over the text's: only fading hides it).
    let hidden = HighlightStyle { fade_out: Some(1.0), ..Default::default() };
    let styles = styles
        .into_iter()
        .flat_map(|(range, style)| {
            if range.end <= shown_to {
                vec![(range, style)]
            } else if range.start >= shown_to {
                vec![(range, hidden)]
            } else {
                vec![(range.start..shown_to, style), (shown_to..range.end, hidden)]
            }
        })
        .collect::<Vec<_>>();
    StyledText::new(text.to_string()).with_highlights(styles)
}

// --- Still ---

/// The pill as it stands (`help`, its keys shown under it), on a stage as wide as it's given.
pub fn still(width: f32, help: bool, look: &Look) -> AnyElement {
    let t = look.t;
    let pill = still_pill(
        Still {
            help,
            listening: true,
            level: 0.0,
            target: faces::target_name(Some(session(0).into())).into_any_element(),
            list_open: false,
            tool: None,
        },
        t,
    );
    let sheet = help.then(|| keys_sheet_face(t, &look.keys, &look.freeze, &look.send, false));
    div()
        .relative()
        .flex_none()
        .w(px(width))
        .rounded(px(Radius::CARD))
        .border_1()
        .border_color(t.card_stroke)
        .bg(t.control_hover)
        .overflow_hidden()
        .py(px(PILL_TOP))
        .flex()
        .justify_center()
        .child(div().flex().flex_col().items_start().gap(px(LIST_GAP as f32)).child(pill).children(sheet))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beat(hold: u64) -> Beat {
        Beat { hold, ..Beat::default() }
    }

    /// Beat 0 with the pointer at rest; 1 after a key; 2 after a click elsewhere.
    fn demo() -> Demo {
        Demo {
            size: (100.0, 50.0),
            app: None,
            chat: None,
            beats: vec![
                Beat { pointer: Some((10.0, 10.0)), ..beat(1000) },
                Beat { key: Some("measure".into()), ..beat(1000) },
                Beat { pointer: Some((90.0, 40.0)), click: true, ..beat(1000) },
            ],
        }
    }

    const fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn starts_on_its_first_beat_still_until_the_next_change() {
        let demo = demo();
        let shown = demo.at(Duration::ZERO);
        assert_eq!((shown.beat, shown.fade, shown.pointer), (0, 1.0, Some((10.0, 10.0))));
        assert!(!shown.moving);
        // Drawn again only when the next key is pressed, at the end of its hold.
        assert_eq!(shown.next, Some(ms(1000)));
        assert_eq!(demo.at(ms(999)).beat, 0);
    }

    #[test]
    fn a_key_shows_then_its_beat_comes_in() {
        let demo = demo();
        // Pressed: popping in, the beat before still showing.
        let pressing = demo.at(ms(1050));
        assert_eq!((pressing.beat, pressing.key.map(|(k, _)| k)), (0, Some("measure")));
        assert!(pressing.moving);
        let coming = demo.at(ms(1000) + KEY + ms(110));
        assert_eq!((coming.beat, coming.under), (1, 0));
        assert!(coming.fade > 0.0 && coming.fade < 1.0 && coming.moving);
        // In, the key shows a while, then goes.
        let held = demo.at(ms(1000) + KEY + FADE + ms(10));
        assert_eq!((held.beat, held.fade), (1, 1.0));
        assert!(held.key.is_some());
        assert!(demo.at(ms(1000) + KEY + FADE + KEY_SHOWN + ms(10)).key.is_none());
    }

    #[test]
    fn the_pointer_glides_to_what_it_clicks_then_its_beat_comes() {
        let demo = demo();
        let start = ms(1000) + KEY + FADE + ms(1000);
        let halfway = demo.at(start + GLIDE / 2);
        let (x, y) = halfway.pointer.unwrap();
        assert!(x > 10.0 && x < 90.0 && y > 10.0 && y < 40.0, "{x}, {y}");
        assert_eq!(halfway.beat, 1);
        let clicking = demo.at(start + GLIDE + ms(50));
        assert_eq!(clicking.pointer, Some((90.0, 40.0)));
        assert!(clicking.ripple.is_some() && clicking.pressed);
        let after = demo.at(start + GLIDE + CLICK + FADE + ms(10));
        assert_eq!((after.beat, after.ripple), (2, None));
    }

    #[test]
    fn goes_round_again_from_its_first_beat() {
        let demo = demo();
        let cycle: Duration = (0..3).map(|i| demo.parts(i).total()).sum();
        let (again, first) = (demo.at(cycle), demo.at(Duration::ZERO));
        assert_eq!(
            (again.beat, again.fade, again.pointer, again.next),
            (first.beat, first.fade, first.pointer, first.next)
        );
        // Back to beat 0 (playing starts once it's in), the pointer glides back to where it rests.
        let back = demo.at(cycle - FADE - GLIDE / 2);
        assert_eq!(back.beat, 2);
        assert!(back.moving && back.pointer != Some((90.0, 40.0)));
    }

    #[test]
    fn a_drag_follows_its_path_held_down_and_draws_as_it_goes() {
        let path = vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)];
        assert_eq!(along(&path, 0.5), (10.0, 0.0));
        assert_eq!(along(&path, 0.75), (10.0, 5.0));
        assert_eq!(path_to(&path, 0.75), vec![(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
        let demo = Demo {
            size: (20.0, 20.0),
            app: None,
            chat: None,
            beats: vec![
                Beat { pointer: Some((0.0, 0.0)), ..beat(500) },
                Beat { path, drag: Some(Drag::Sketch), pointer: Some((20.0, 20.0)), ..beat(1000) },
            ],
        };
        let dragging = demo.at(ms(500) + DRAG / 2);
        assert!(dragging.pressed && dragging.beat == 0);
        assert!(dragging.dragging.is_some_and(|(i, done)| i == 1 && done > 0.0 && done < 1.0));
        // Let go: its beat comes in, then the pointer goes on.
        let after = demo.at(ms(500) + DRAG + FADE + GLIDE / 2);
        assert_eq!((after.beat, after.dragging), (1, None));
        assert!(after.moving && after.pointer.is_some_and(|(x, _)| x > 10.0 && x < 20.0));
    }

    #[test]
    fn typing_and_talking_move_while_they_last() {
        let mut demo = demo();
        demo.beats[1].scene.card = Some(Card { pin: 1, text: "abcd", typing: true, intent: None });
        let typing = demo.at(ms(1000) + KEY + FADE + ms(100));
        assert!(typing.moving && typing.held >= ms(99));
        assert!(!demo.at(ms(1000) + KEY + FADE + TYPE_EACH * 4 + ms(500)).moving);
        demo.beats[1].scene.card = None;
        demo.beats[1].say = Some("hello there");
        assert!(demo.at(ms(1000) + KEY + FADE + ms(900)).moving);
    }

    #[test]
    fn a_step_can_be_watched_from_its_start() {
        let mut demo = demo();
        for (i, b) in demo.beats.iter_mut().enumerate() {
            b.step = Some(i);
        }
        // Step 2: going into beat 2 (the pointer setting off from where beat 1 left it), lit at once.
        let at = demo.time_of_step(2).unwrap();
        let shown = demo.at(at);
        assert_eq!((shown.beat, shown.step, shown.pointer), (1, Some(2), Some((10.0, 10.0))));
        assert!(shown.moving);
        // Step 0: going round to its first beat.
        let shown = demo.at(demo.time_of_step(0).unwrap());
        assert_eq!((shown.beat, shown.step), (2, Some(0)));
        assert_eq!(demo.time_of_step(9), None);
    }

    #[test]
    fn what_is_said_shows_a_word_at_a_time_without_moving() {
        let ink = gpui::black();
        let colors = |text: StyledText| text.into_any_element();
        let _ = colors(said_text("a [1] b", 1, ink, ink, ink));
        // Not shown yet, still laid out: "a" shown, " [1] b" unseen.
        let text = "a [1] b";
        let shown_to = text.match_indices(' ').map(|(i, _)| i).next().unwrap();
        assert_eq!(&text[..shown_to], "a");
    }

    #[test]
    fn every_demonstration_is_there_and_makes_sense() {
        for name in ["basics", "target", "card", "tools"] {
            let demo = super::demo(name).unwrap_or_else(|| panic!("no demonstration {name}"));
            assert!(demo.beats.len() > 1, "{name}");
            for beat in &demo.beats {
                // A card is a pin's, and a drag says what it draws.
                if let Some(card) = &beat.scene.card {
                    assert!(beat.scene.pins.iter().any(|p| p.number == card.pin), "{name}");
                }
                assert_eq!(beat.path.len() > 1, beat.drag.is_some(), "{name}");
            }
        }
        let steps: Vec<_> = super::demo("basics").unwrap().beats.iter().filter_map(|b| b.step).collect();
        assert_eq!((steps.first(), steps.last()), (Some(&0), Some(&3)));
    }
}
