//! Help's demonstrations, and setup's How it works: the overlay's own pieces (`faces.rs`) on a plain
//! stage beside a small app drawn in code, and what happens to them in turn: a key pressed, the
//! pointer gliding to what it clicks or drags, what's said. Each beat is a scene; going into it, the
//! scene before gives way to it as the overlay's pieces come and go. A demonstration plays from its
//! first beat, round and round; the window is drawn every frame only while something moves, and else
//! again when the next change is due.

use std::rc::Rc;
use std::time::{Duration, Instant};

use asides_core::inspect::{Intent, PtRect};
use asides_core::keys;
use asides_core::measure::{Frame, Rect as Pixels};
use asides_core::motion;
use asides_core::overlay::{MARKER, Tool};
use gpui::{
    AnyElement, Div, FontWeight, HighlightStyle, Hsla, StyledText, TextRun, Window, canvas, deferred, div, prelude::*,
    px,
};

use crate::controls::{self, Radius, Theme, ease, hex};
use crate::faces::{
    self, Prim, Still, ToolSlot, agent_segment, agents_track, card_camera, card_cluster, card_field, card_surface,
    card_tool, choose_folder_content, folder_content, intent_dot, keys_sheet_face, label_at, list_hairline,
    list_heading, list_lines, list_panel, list_row_face, list_rows, marker_face, new_session_row, plain_glass,
    still_pill, talk_face, tool_slot_width,
};
use crate::style;
use crate::ui::{CAMERA_WIDTH, CARD_WIDTH, Keys, LIST_GAP};

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
/// How long a flash on the app shows: a few frames, too fast to read.
const FLASH: Duration = Duration::from_millis(100);
/// Typed into a card, a character this often.
const TYPE_EACH: Duration = Duration::from_millis(45);
/// Said, a character's worth of time each (spaces too): every caption at the same pace, each word
/// showing once those before it have had theirs. Then all of it stays this long, to be read.
const SAID_EACH: Duration = Duration::from_millis(42);
const SAID_READ: Duration = Duration::from_millis(1600);

/// How many of `text`'s words are said `held` in: the first at once, each next when the time for
/// the characters before it has gone by.
fn words_said(text: &str, held: Duration) -> usize {
    let chars = held.as_millis() / SAID_EACH.as_millis().max(1);
    1 + text.chars().enumerate().filter(|&(i, c)| c == ' ' && (i as u128) < chars).count()
}

/// How long a beat saying `text` holds (ms): until its last word, then `SAID_READ`.
fn saying(text: &str) -> u64 {
    let last = text.chars().enumerate().filter(|&(_, c)| c == ' ').last().map_or(0, |(i, _)| i + 1);
    (SAID_EACH * last as u32 + SAID_READ).as_millis() as u64
}
/// The pill's top on the stage.
const PILL_TOP: f32 = 0.0;
/// The pill's height, and how far left of its middle where the aside goes is (its buttons on either
/// side are always as wide: 5 + 28 + 28 + 28 + 9 before the name, 9 + 2 + 21 + 2 + 99 + 5 after).
const PILL_HEIGHT: f32 = 38.0;
const NAME_LEFT_OF_MIDDLE: f32 = 20.0;

/// A demonstration: its size (laid out that wide, in the middle of what it's given), where the app
/// and the agent's chat are on its stage, its beats, and the clip the aside brings into the chat.
pub struct Demo {
    pub size: (f32, f32),
    pub app: Option<Rect>,
    pub chat: Option<Rect>,
    pub beats: Vec<Beat>,
    pub clip: Option<Clip>,
}

/// A clip, as the agent gets it: its frames, and what's said after them (`motion`'s own caption).
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub frames: Vec<ClipFrame>,
    pub caption: String,
}

/// A frame of a clip: what the app's button showed (`declined`: its error), and when, in seconds into
/// the clip.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipFrame {
    pub declined: bool,
    pub time: f32,
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
    /// The screen frozen under the pill, the microphone listening, a clip recording.
    pub frozen: bool,
    pub listening: bool,
    pub clip: bool,
    /// A note under the pill (`ui::toast`'s).
    pub note: Option<&'static str>,
    /// The app's button says its payment was declined: for an instant (`FLASH`) as the beat comes in,
    /// then no longer.
    pub declined: bool,
    /// Where the aside goes, as the pill names it.
    pub target: &'static str,
    /// The pill's list open, and the session lit in it.
    pub list: Option<Option<usize>>,
    /// The tool, when it isn't Annotate.
    pub tool: Option<Tool>,
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

/// The demonstration called `name` (`basics`, `target`, `card`, `tools` or `clip`), laid out `width`
/// wide.
pub fn demo(name: &str, width: f32) -> Option<Demo> {
    match name {
        "basics" => Some(basics(width)),
        "target" => Some(target()),
        "card" => Some(card(width)),
        "tools" => Some(tools(width)),
        "clip" => Some(clip(width)),
        _ => None,
    }
}

// --- The app on the stage ---

/// The app's border (`border_1`): what's on it starts inside, and so do the marks on it (from its
/// outside, they were a point off).
const APP_BORDER: f32 = 1.0;
/// Its parts, from the top left inside its border: its rows (cramped, as the basics say: they touch),
/// the line above the total (what's under it), the total's top, and the button to pay.
const APP_ROW_TOPS: [f32; 2] = [42.0, 66.0];
const APP_ROW_HEIGHT: f32 = 24.0;
const APP_RULE: f32 = 102.0;
const APP_TOTAL_TOP: f32 = 110.0;
const APP_BUTTON: Rect = (16.0, 140.0, 92.0, 42.0);
/// How tall what's on it is, with room under the button.
const APP_CONTENT: f32 = 198.0;

/// Where the app's part at `(x, y, w, h)` is on the stage, the app at `app`.
fn on_app(app: Rect, (x, y, w, h): Rect) -> Rect {
    (app.0 + APP_BORDER + x, app.1 + APP_BORDER + y, w, h)
}

/// Its rows, a little around them (what a drag over them pins).
fn app_rows(app: Rect) -> Rect {
    let top = APP_ROW_TOPS[0] - 3.0;
    on_app(app, (12.0, top, app.2 - 2.0 * APP_BORDER - 24.0, APP_ROW_TOPS[1] + APP_ROW_HEIGHT + 3.0 - top))
}

fn app_button(app: Rect) -> Rect {
    on_app(app, APP_BUTTON)
}

/// The total's middle.
fn app_total(app: Rect) -> Point {
    let (x, y, _, _) = on_app(app, (app.2 - 2.0 * APP_BORDER - 38.0, APP_TOTAL_TOP + 8.0, 0.0, 0.0));
    (x, y)
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

/// Under the pill, where the screen's windows start; between the app and the agent's chat; and under
/// what's on the app, room for what's said (it goes over the windows' bottom).
const SCREEN_TOP: f32 = PILL_TOP + PILL_HEIGHT + 12.0;
const SCREEN_GAP: f32 = 12.0;
const CAPTION_ROOM: f32 = 48.0;

/// What's said in the basics, and about a clip.
const BASICS_SAID: &str = "This button [1] wraps, and these rows [2] are cramped.";
const CLIP_SAID: &str = "When I pay it flashes Declined, too fast to read.";

/// Freeze, click what's wrong, say why, send: the app and the agent's chat beside it, as wide as
/// they're given.
fn basics(width: f32) -> Demo {
    let chat_width = (width * 0.4).clamp(200.0, 280.0);
    let app = (0.0, SCREEN_TOP, width - chat_width - SCREEN_GAP, APP_CONTENT + CAPTION_ROOM);
    let chat = (app.2 + SCREEN_GAP, SCREEN_TOP, chat_width, app.3);
    let button = app_button(app);
    let rows = app_rows(app);
    let click = (button.0 + button.2 - 12.0, button.1 + 9.0);
    let rest = (app.0 + app.2 * 0.72, button.1 + 20.0);
    let frozen = Scene { frozen: true, listening: true, target: session(0), ..Scene::default() };
    let pin1 = Pin { number: 1, at: click, on: On::Element(button), intent: None };
    let pin2 = Pin { number: 2, at: (rows.0, rows.1), on: On::Area(rows), intent: None };
    let pinned = Scene { pins: vec![pin1.clone()], ..frozen.clone() };
    let both = Scene { pins: vec![pin1, pin2], ..frozen.clone() };
    Demo {
        size: (width, SCREEN_TOP + app.3),
        app: Some(app),
        chat: Some(chat),
        clip: None,
        beats: vec![
            Beat {
                hold: 900,
                pointer: Some(rest),
                step: Some(0),
                scene: Scene { target: session(0), ..Scene::default() },
                ..Beat::default()
            },
            Beat { hold: 1100, key: Some("open".into()), step: Some(0), scene: frozen.clone(), ..Beat::default() },
            // The pointer goes to the button and clicks: its outline and its number come in together.
            Beat { hold: 650, pointer: Some(click), click: true, step: Some(1), scene: pinned, ..Beat::default() },
            Beat {
                hold: 700,
                path: vec![
                    (rows.0, rows.1),
                    (rows.0 + rows.2 * 0.5, rows.1 + rows.3 * 0.8),
                    (rows.0 + rows.2, rows.1 + rows.3),
                ],
                drag: Some(Drag::Pin),
                step: Some(1),
                scene: both.clone(),
                ..Beat::default()
            },
            Beat {
                hold: saying(BASICS_SAID),
                pointer: Some(rest),
                say: Some(BASICS_SAID),
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

/// Where it goes: pointed at, the list opens; a session picked there, it closes. One, then back:
/// the list shows most of the time (the room under the pill is its).
fn target() -> Demo {
    let middle = 210.0;
    let name = (middle - NAME_LEFT_OF_MIDDLE, PILL_TOP + PILL_HEIGHT / 2.0);
    let row = |i| (name.0 + 20.0, list_row_middle(i));
    let scene = |target, list| Scene { frozen: true, listening: true, target, list, ..Scene::default() };
    let pick = |from: usize, to: usize| {
        [
            Beat { hold: 1300, pointer: Some(name), scene: scene(session(from), Some(None)), ..Beat::default() },
            Beat { hold: 450, pointer: Some(row(to)), scene: scene(session(from), Some(Some(to))), ..Beat::default() },
            Beat { hold: 700, pointer: Some(row(to)), click: true, scene: scene(session(to), None), ..Beat::default() },
        ]
    };
    Demo { size: (420.0, 412.0), app: None, chat: None, beats: [pick(0, 2), pick(2, 0)].concat(), clip: None }
}

/// A pin's card: its number clicked, what's typed, what it is. The app as wide as it's given, with
/// room under its button for the card, and under that for the key pressed.
fn card(width: f32) -> Demo {
    let app = (0.0, 0.0, width, APP_CONTENT + 60.0 + CAPTION_ROOM);
    let button = app_button(app);
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
        size: (width, app.3),
        app: Some(app),
        chat: None,
        clip: None,
        beats: vec![
            Beat {
                hold: 1000,
                pointer: Some((width - 96.0, 70.0)),
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

/// Measure (the distances from the pointer, then the size of what's dragged over), then Draw. The
/// app as wide as it's given.
fn tools(width: f32) -> Demo {
    let app = (0.0, SCREEN_TOP, width, APP_CONTENT + 40.0);
    let button = app_button(app);
    let inside_right = app.0 + app.2 - APP_BORDER;
    let at = (button.0 + button.2 + 96.0, button.1 + 21.0);
    let distance = Measured::Distance {
        at,
        across: (button.0 + button.2, inside_right),
        down: (app.1 + APP_BORDER + APP_RULE, app.1 + app.3 - APP_BORDER),
    };
    let ring = circle_around(app_total(app), (40.0, 17.0));
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
    let rest = (width - 96.0, app.1 + app.3 - 50.0);
    Demo {
        size: (width, app.1 + app.3),
        app: Some(app),
        chat: None,
        clip: None,
        beats: vec![
            Beat { hold: 900, pointer: Some(rest), scene: scene(None, vec![], vec![]), ..Beat::default() },
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

/// How wide the pill names where the aside goes in the demonstrations ("Claude Code (shop)", 12.5
/// points, medium), near enough on each system's font for the pointer to land on the camera, and how
/// far left of that name the camera's middle is (5 + 28 + 28 + 9 before the name, 5 + 28 + 28 + 14 to
/// it).
const SESSION_NAME_WIDTH: f32 = 116.0;
const CAMERA_LEFT_OF_NAME: f32 = 23.0;
/// Under the chat's aside, room for the clip.
const CLIP_ROOM: f32 = 70.0;

/// A clip: K (the screen comes to life), the button to pay clicked, its error for an instant, the
/// camera's square clicked, why said, sent: in the agent's chat, the clip's picture where it was
/// recorded, what asides says of it, then what was said. The app and the chat beside it, as wide as
/// they're given.
fn clip(width: f32) -> Demo {
    let chat_width = (width * 0.4).clamp(200.0, 280.0);
    let app = (0.0, SCREEN_TOP, width - chat_width - SCREEN_GAP, APP_CONTENT + CAPTION_ROOM + CLIP_ROOM);
    let chat = (app.2 + SCREEN_GAP, SCREEN_TOP, chat_width, app.3);
    let button = app_button(app);
    let pay = (button.0 + button.2 * 0.62, button.1 + button.3 * 0.6);
    let rest = (app.0 + app.2 * 0.72, button.1 + 20.0);
    let name_left = width / 2.0 - NAME_LEFT_OF_MIDDLE - SESSION_NAME_WIDTH / 2.0;
    let camera = (name_left - CAMERA_LEFT_OF_NAME, PILL_TOP + PILL_HEIGHT / 2.0);
    // Live, only the pill stays on the screen: on the stage, as frozen (nothing's pinned).
    let pill = |clip, declined, note| Scene {
        frozen: true,
        listening: true,
        target: session(0),
        clip,
        declined,
        note,
        ..Scene::default()
    };
    // Recording from beat `ON` (K), the error at `FLASH_AT` (the click), stopped at `OFF` (the square).
    const ON: usize = 1;
    const FLASH_AT: usize = 2;
    const OFF: usize = 3;
    let mut demo = Demo {
        size: (width, SCREEN_TOP + app.3),
        app: Some(app),
        chat: Some(chat),
        clip: None,
        beats: vec![
            Beat { hold: 1000, pointer: Some(rest), scene: pill(false, false, None), ..Beat::default() },
            Beat { hold: 700, key: Some("clip".into()), scene: pill(true, false, None), ..Beat::default() },
            // Its error, for an instant, then back as it was.
            Beat { hold: 1000, pointer: Some(pay), click: true, scene: pill(true, true, None), ..Beat::default() },
            Beat {
                hold: 1500,
                pointer: Some(camera),
                click: true,
                scene: pill(false, false, Some("The clip is in the aside")),
                ..Beat::default()
            },
            Beat {
                hold: saying(CLIP_SAID),
                pointer: Some(rest),
                say: Some(CLIP_SAID),
                scene: pill(false, false, None),
                ..Beat::default()
            },
            Beat {
                hold: 3400,
                key: Some("send".into()),
                scene: Scene { target: session(0), sent: true, ..Scene::default() },
                ..Beat::default()
            },
        ],
    };
    // What the recorder keeps of it, at the times it plays: the first frame, the error, the frame
    // after it (back as it was, the last), and asides' caption for them.
    let since = |i: usize| (demo.comes_in(i) - demo.comes_in(ON)).as_secs_f32();
    let frames = vec![
        ClipFrame { declined: false, time: 0.0 },
        ClipFrame { declined: true, time: since(FLASH_AT) },
        ClipFrame { declined: false, time: since(FLASH_AT) + FLASH.as_secs_f32() },
    ];
    let kept = motion::Clip {
        frames: frames.iter().map(|f| (f.time, Frame::filled(1, 1, 0))).collect(),
        rect: Pixels::new(0, 0, 1, 1),
        seen: frames.len(),
        seconds: since(OFF),
        flashes: vec![1],
    };
    demo.clip = Some(Clip { frames, caption: kept.summary().caption("in Safari") });
    demo
}

// --- Playing ---

/// What a demonstration shows at a moment.
#[derive(Debug, PartialEq)]
pub struct Shown<'a> {
    /// The beat in front, how far it has come in (1: all of it), and the one before it.
    pub beat: usize,
    pub fade: f32,
    pub under: usize,
    /// How long the beat in front has been in, and since it started coming in.
    pub held: Duration,
    pub since_in: Duration,
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

    /// When beat `i` starts coming in (its scene giving way to the next), from the start of a round.
    fn comes_in(&self, i: usize) -> Duration {
        let before: Duration = (0..i).map(|j| self.parts(j).total()).sum();
        before + self.parts(i).before()
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
        // Moving: a key popping in or fading out, the pointer, a click, the scenes giving way, a flash,
        // the pointer going on after a drag, typing or talking.
        let flash = if b.scene.declined { FLASH } else { Duration::ZERO };
        let spans = [
            (Duration::ZERO, if b.key.is_some() { POP } else { Duration::ZERO }),
            (p.key, faded),
            (clicked, clicked + flash),
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
            since_in: if s < clicked { FADE + held_before(self, prev) } else { s - clicked },
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

/// The stage, as wide as it's given: nothing of its own, only the windows and the overlay's pieces on
/// it (a screen behind them looked pasted in).
fn stage(width: f32, height: f32) -> Div {
    div().relative().flex_none().w(px(width)).h(px(height))
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
    // What a click or a drag makes is there at once, as on screen: fading in, a drag's outline went
    // as the pointer let go, then came back.
    let made = &demo.beats[shown.beat];
    let arriving = if made.click || made.drag.is_some() { 1.0 } else { f };

    // Measured: the distances from the pointer, and sizes held; those in both scenes stay, the others
    // come or go.
    let staying = to.measured.iter().map(|m| (m, if from.measured.contains(m) { 1.0 } else { arriving }));
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
        pins.push((p, if stays { 1.0 } else { arriving }));
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

    // A flash: there, then gone, at once.
    let declined = to.declined && shown.since_in < FLASH;
    let app = demo.app.map(|r| app_view(r, declined, t).absolute().left(px(r.0 + dx)).top(px(r.1)));
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
    stage(width, demo.size.1)
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

/// The app's surface.
fn app_surface(t: Theme) -> Hsla {
    if t.dark { hex(0x262523, 1.0) } else { hex(0xffffff, 1.0) }
}

/// The app's button to pay, `scale` times its size: its label wrapping, or (`declined`) its error.
fn pay_button(declined: bool, scale: f32, t: Theme) -> Div {
    let (_, _, w, h) = APP_BUTTON;
    // Its ink inverted: dark on a light page, light on a dark one.
    let (button, on_button) =
        if t.dark { (hex(0xedebe7, 1.0), hex(0x1c1b19, 1.0)) } else { (hex(0x2b2a27, 1.0), hex(0xffffff, 1.0)) };
    let error = if t.dark { hex(0xd8584e, 1.0) } else { hex(0xc9463d, 1.0) };
    let face = |fill: Hsla, ink: Hsla| {
        div()
            .absolute()
            .inset_0()
            .rounded(px(8. * scale))
            .bg(fill)
            .text_color(ink)
            .text_size(px(12. * scale))
            .line_height(px(15. * scale))
            .font_weight(FontWeight::MEDIUM)
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
    };
    div()
        .relative()
        .flex_none()
        .w(px(w * scale))
        .h(px(h * scale))
        .when(!declined, |d| d.child(face(button, on_button).child("Pay now ·").child("$72.00")))
        .when(declined, |d| d.child(face(error, hex(0xffffff, 1.0)).child("Declined")))
}

/// The app on the stage, `(_, _, width, height)`: an order's summary (its rows cramped), its total, and
/// the button to pay (its label wrapping; `declined`, its error).
fn app_view((_, _, width, height): Rect, declined: bool, t: Theme) -> Div {
    let (sand, sage) =
        if t.dark { (hex(0x8c7d62, 1.0), hex(0x5f7a5a, 1.0)) } else { (hex(0xe6dcc6, 1.0), hex(0xa9c2a2, 1.0)) };
    let surface = app_surface(t);
    let inside = width - 2.0 * APP_BORDER - 32.0;
    let row = |top: f32, swatch: Hsla, name: &'static str, price: &'static str| {
        div()
            .absolute()
            .left(px(16.))
            .top(px(top))
            .w(px(inside))
            .h(px(APP_ROW_HEIGHT))
            .flex()
            .items_center()
            .gap(px(10.))
            .text_size(px(12.5))
            .child(div().flex_none().size(px(APP_ROW_HEIGHT)).rounded(px(5.)).bg(swatch))
            .child(div().flex_1().child(name))
            .child(div().text_color(t.secondary).child(price))
    };
    let (bx, by, _, _) = APP_BUTTON;
    div()
        .w(px(width))
        .h(px(height))
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
        .child(row(APP_ROW_TOPS[0], sand, "Linen tote", "$48.00"))
        .child(row(APP_ROW_TOPS[1], sage, "Notebook × 2", "$24.00"))
        .child(div().absolute().left(px(16.)).top(px(APP_RULE - 1.)).w(px(inside)).h(px(1.)).bg(t.hairline))
        .child(
            div()
                .absolute()
                .left(px(16.))
                .top(px(APP_TOTAL_TOP))
                .w(px(inside))
                .flex()
                .justify_between()
                .text_size(px(12.5))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Total")
                .child("$72.00"),
        )
        .child(pay_button(declined, 1.0, t).absolute().left(px(bx)).top(px(by)))
}

/// The chat's padding, its text's size, and how far a line under the prompt's `› ` starts.
const CHAT_PADDING: f32 = 12.0;
const CHAT_TEXT: f32 = 10.5;
const CHAT_INDENT: f32 = 2.0 * 0.6 * CHAT_TEXT;

/// The agent's chat beside the app: what was said before, and the aside, typed in once it's sent
/// (`sent`, how much of it shows): the screenshot, what was said, then its pins; or the clip's
/// picture, what asides says of it, then what was said.
fn chat_view((x, y, w, h): Rect, dx: f32, sent: f32, shown: &Shown, demo: &Demo, t: Theme) -> AnyElement {
    let surface = if t.dark { hex(0x1f1e1c, 1.0) } else { hex(0xffffff, 1.0) };
    let mono = |text: &'static str, color: Hsla| div().text_color(color).child(text);
    let blue = hex(style::ANNOTATION_BLUE, 1.0);
    let green = hex(style::ANNOTATION_GREEN, 1.0);
    let said = demo.beats.iter().find_map(|b| b.say).unwrap_or_default();
    let aside = div().opacity(sent).flex().flex_col().gap(px(6.));
    let image =
        |after: &str| div().child(said_text(&format!("› [Image #1]{after}"), usize::MAX, blue, green, t.primary));
    // Under the prompt's `› `, as the agent shows the rest of a message.
    let indented = |child: Div| div().pl(px(CHAT_INDENT)).child(child);
    let aside = match &demo.clip {
        None => aside
            .child(image(&format!(" {said}")))
            .child(mono("  ### 1. button \"Pay now · $72.00\"", t.secondary))
            .child(mono("  ### 2. list \"Items\"", t.secondary)),
        Some(clip) => {
            let inside = w - 2.0 * APP_BORDER - 2.0 * CHAT_PADDING - CHAT_INDENT;
            aside
                .child(image(""))
                .child(indented(clip_sheet(&clip.frames, inside, t)))
                .child(indented(div().text_color(t.secondary).child(clip.caption.clone())))
                .child(indented(div().text_color(t.primary).child(said)))
        }
    };
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
                .p(px(CHAT_PADDING))
                .flex()
                .flex_col()
                .gap(px(8.))
                .font_family(controls::mono_font())
                .text_size(px(CHAT_TEXT))
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

/// What a clip's frames show around what changed (the button), on the stage, and the least a side
/// of them is (`motion`'s 24 and 200 pixels, at 2×).
const CLIP_AROUND: f32 = 12.0;
const CLIP_SIDE: f32 = 100.0;

/// A clip as the agent gets it (`motion::pictures`), `width` wide: its frames side by side on a dark
/// picture, each under its number and its time, the part of the app that changed in each.
fn clip_sheet(frames: &[ClipFrame], width: f32, t: Theme) -> Div {
    let (gap, band, border) = (4.0, 12.0, 1.0);
    let n = frames.len().max(1) as f32;
    let tile = (width - 2.0 * border - gap * (n + 1.0)) / n;
    let picture = hex(0x1c1c1e, 1.0);
    let (_, _, w, h) = APP_BUTTON;
    let crop = ((w + 2.0 * CLIP_AROUND).max(CLIP_SIDE), (h + 2.0 * CLIP_AROUND).max(CLIP_SIDE));
    let scale = tile / crop.0;
    let frame = |(i, f): (usize, &ClipFrame)| {
        div()
            .flex_none()
            .w(px(tile))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(band))
                    .flex()
                    .items_center()
                    .text_size(px(8.5))
                    .text_color(hex(0xf2f2f7, 1.0))
                    .whitespace_nowrap()
                    .child(format!("{}  {:.2}s", i + 1, f.time)),
            )
            .child(
                div()
                    .h(px(crop.1 * scale))
                    .bg(app_surface(t))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(pay_button(f.declined, scale, t)),
            )
    };
    div()
        .w(px(width))
        .px(px(gap))
        .pb(px(gap))
        .flex()
        .gap(px(gap))
        .rounded(px(5.))
        .bg(picture)
        // On a dark chat, its edge.
        .border_1()
        .border_color(if t.dark { t.card_stroke } else { picture })
        .children(frames.iter().enumerate().map(frame))
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
    // The tool's slot easing from one's width to the other's (nothing's: none), their names fading
    // one into the other.
    let slot_width = |tool: Option<Tool>| tool.map_or(0.0, |tool| tool_slot_width(name_width(tool.label(), window)));
    let tool = match (from.tool, to.tool) {
        (None, None) => None,
        (a, b) => Some(ToolSlot {
            width: slot_width(a) + (slot_width(b) - slot_width(a)) * ease(f),
            shown: match (a, b) {
                (None, _) => f,
                (_, None) => 1.0 - f,
                _ => 1.0,
            },
            tools: match (a, b) {
                (Some(a), Some(b)) if a == b => vec![(a, 1.0)],
                _ => a.map(|a| (a, 1.0 - f)).into_iter().chain(b.map(|b| (b, f))).collect(),
            },
        }),
    };
    let pill = still_pill(
        Still {
            help: false,
            listening: to.listening || from.listening,
            level,
            // As the overlay's: at once.
            recording_clip: to.clip,
            target,
            list_open: to.list.is_some() && f > 0.5 || from.list.is_some() && f <= 0.5,
            tool,
        },
        t,
    );
    let list = match (from.list, to.list) {
        (None, None) => None,
        (a, b) => {
            let chosen = (0..3).find(|&i| session(i) == to.target).map_or(Tried::History, Tried::Session);
            let list = list_view(b.or(a).flatten(), chosen, 0, t, &|_, row| row.into_any_element());
            Some((list, between(a.is_some(), b.is_some(), f)))
        }
    };
    let below = list.map(|(list, shown)| {
        div()
            .absolute()
            .left_0()
            .top(px(PILL_HEIGHT + LIST_GAP as f32 - (1.0 - shown) * 6.0))
            .opacity(shown)
            .child(list)
    });
    // A note under it, in its middle.
    let note = to.note.or(from.note).map(|text| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .top(px(PILL_HEIGHT + 8.0))
            .flex()
            .justify_center()
            .opacity(between(from.note.is_some(), to.note.is_some(), f))
            .child(note_view(text.into(), t))
    });
    Some(
        div()
            .absolute()
            .left(px(dx))
            .top(px(PILL_TOP - (1.0 - shows) * 6.0))
            .w(px(demo.size.0))
            .flex()
            .justify_center()
            .opacity(shows)
            .child(div().relative().child(pill).children(below).children(note))
            .into_any_element(),
    )
}

/// The middle of the pill's list's row for session `i`, on the stage: under the pill, its border and
/// its rows' room, the Sessions heading, and the sessions before it (two lines each).
fn list_row_middle(i: usize) -> f32 {
    let (heading, session) = (26.8, 46.8);
    PILL_TOP + PILL_HEIGHT + LIST_GAP as f32 + 1.0 + 4.0 + heading + session * i as f32 + session / 2.0
}

/// The pill's list as it lists three sessions (`lit`, the one the pointer's on), two directories and
/// History only, `chosen` checked, a new session's agent `agent`. Each row and agent is passed
/// through `row` with what picking it does (the pill to try picks them).
fn list_view(
    lit: Option<usize>,
    chosen: Tried,
    agent: usize,
    t: Theme,
    row: &dyn Fn(Option<Try>, Div) -> AnyElement,
) -> AnyElement {
    let apps: [&str; 3] = match () {
        _ if cfg!(target_os = "macos") => ["Terminal", "Ghostty", "Diri"],
        _ if cfg!(windows) => ["Windows Terminal", "Windows Terminal", "Console"],
        _ => ["GNOME Terminal", "Ghostty", "Konsole"],
    };
    let home = if cfg!(windows) { "C:\\Users\\me\\code" } else { "~/Developer" };
    let separator = if cfg!(windows) { "\\" } else { "/" };
    let mut rows: Vec<AnyElement> = vec![list_heading("Sessions", t).into_any_element()];
    for (i, app) in apps.into_iter().enumerate() {
        let lines = list_lines(session(i).into(), Some(app.into()), t);
        let face = list_row_face(chosen == Tried::Session(i), lit == Some(i), true, lines, t);
        rows.push(row(Some(Try::Pick(Tried::Session(i))), face));
    }
    rows.push(list_hairline(t));
    let track = agents_track(t).children(AGENTS.into_iter().enumerate().map(|(i, name)| {
        let face =
            agent_segment(name, i == agent, t).when(i == agent, |d| d.bg(t.selected).border_color(t.selected_stroke));
        row(Some(Try::Agent(i)), face)
    }));
    rows.push(new_session_row(track, t));
    for (i, name) in FOLDERS.into_iter().enumerate() {
        let content = folder_content(name.into(), format!("{home}{separator}{name}"), t);
        let face = list_row_face(chosen == Tried::New(agent, i), false, true, content, t);
        rows.push(row(Some(Try::Pick(Tried::New(agent, i))), face));
    }
    rows.push(row(None, list_row_face(false, false, true, choose_folder_content(t), t)));
    rows.push(list_hairline(t));
    let history = list_lines("History only".into(), Some("Send it later from the asides window".into()), t);
    let face = list_row_face(chosen == Tried::History, false, true, history, t);
    rows.push(row(Some(Try::Pick(Tried::History)), face));
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
    // Its camera, as on a pin that's on more than a point.
    let tools = [card_tool(Tool::Draw, false, t), card_tool(Tool::Measure, false, t), card_camera(t)]
        .map(IntoElement::into_any_element);
    let row = vec![
        card_field(&typed, true, t),
        card_cluster(dots),
        card_cluster(tools),
        talk_face(false, t).into_any_element(),
    ];
    let half = MARKER as f32 / 2.0;
    Some(
        card_surface(plain_glass(t, 12.), (CARD_WIDTH + CAMERA_WIDTH) as f32, row, t)
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
        let words = words_said(text, shown.held);
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

/// The pill with its keys under it, in the middle of `width`.
pub fn still(width: f32, look: &Look) -> AnyElement {
    let t = look.t;
    let pill = still_pill(
        Still {
            help: true,
            listening: true,
            level: 0.0,
            recording_clip: false,
            target: faces::target_name(Some(session(0).into())).into_any_element(),
            list_open: false,
            tool: None,
        },
        t,
    );
    let sheet = keys_sheet_face(t, &look.keys, &look.freeze, &look.send, false);
    div()
        .flex_none()
        .w(px(width))
        .flex()
        .justify_center()
        .child(div().flex().flex_col().items_start().gap(px(LIST_GAP as f32)).child(pill).child(sheet))
        .into_any_element()
}

// --- The pill to try ---

/// A new session's agents, and the directories it can start in, in the pill's list.
const AGENTS: [&str; 3] = ["Claude Code", "Codex", "OpenCode"];
const FOLDERS: [&str; 2] = ["shop", "api"];
/// How long a note shows under the pill (`ui::TOAST`), and the pill is away once sent or closed.
pub const NOTE_SHOWN: Duration = Duration::from_millis(1600);
pub const AWAY: Duration = Duration::from_millis(2200);

/// Where the pill to try sends: a made-up session, a new one (its agent, its directory), History only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Tried {
    Session(usize),
    New(usize, usize),
    History,
}

impl Default for Tried {
    fn default() -> Self {
        Tried::Session(0)
    }
}

impl Tried {
    /// As the pill names it (`Targets::label`).
    pub fn label(self) -> String {
        match self {
            Tried::Session(i) => session(i).into(),
            Tried::New(agent, folder) => format!("New {} ({})", AGENTS[agent], FOLDERS[folder]),
            Tried::History => "History only".into(),
        }
    }
}

/// What's done to the pill to try: the pointer on a button (or off it), a button clicked, an agent
/// or where it goes picked in its list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Try {
    Point(&'static str, bool),
    Press(&'static str),
    Agent(usize),
    Pick(Tried),
    /// A click outside the list or the keys open under it.
    Dismiss,
}

/// Help's pill to try, as it is: nothing is sent, its sessions are made up.
#[derive(Default)]
pub struct TryPill {
    pub muted: bool,
    /// Recording a clip.
    pub clip: bool,
    pub keys: bool,
    pub list: bool,
    pub target: Tried,
    pub agent: usize,
    /// The button under the pointer: its name shows under the pill.
    pub pointed: Option<&'static str>,
    /// What a note under it says (Start over's), until when.
    pub note: Option<(String, Instant)>,
    /// Sent or closed: the pill away, what's said in its place, until when.
    pub away: Option<(String, Instant)>,
    /// Each time it comes back (it fades in again).
    pub back: u32,
    /// The list or the keys closed by a click outside them, by which button opens them, and when: the
    /// click on that button doesn't open them again.
    pub closed: Option<(&'static str, Instant)>,
}

/// Within this of a click outside the list or the keys, their button's click was that click.
const CLICKED_SHUT: Duration = Duration::from_millis(500);

impl TryPill {
    /// `done` to it now; when something it shows then should go (a note, the pill away), when.
    pub fn apply(&mut self, done: Try, now: Instant) -> Option<Instant> {
        match done {
            Try::Point(id, true) => self.pointed = Some(id),
            Try::Point(id, false) => {
                if self.pointed == Some(id) {
                    self.pointed = None;
                }
            }
            Try::Press(id @ ("help" | "target"))
                if self
                    .closed
                    .is_some_and(|(was, at)| was == id && now.saturating_duration_since(at) < CLICKED_SHUT) =>
            {
                self.closed = None;
            }
            Try::Press("help") => (self.keys, self.list) = (!self.keys, false),
            Try::Press("mic") => self.muted = !self.muted,
            Try::Press("clip") => {
                self.clip = !self.clip;
                if !self.clip {
                    self.note = Some(("The clip is in the aside".into(), now + NOTE_SHOWN));
                    return Some(now + NOTE_SHOWN);
                }
            }
            Try::Press("target") => (self.list, self.keys) = (!self.list, false),
            Try::Press("restart") => {
                (self.list, self.keys) = (false, false);
                self.note = Some(("Started over".into(), now + NOTE_SHOWN));
                return Some(now + NOTE_SHOWN);
            }
            Try::Press(id @ ("done" | "close")) => {
                let said = match id {
                    "done" => format!("Sent to {}", self.target.label()),
                    _ => "Closed without sending".into(),
                };
                (self.list, self.keys, self.pointed, self.note, self.clip) = (false, false, None, None, false);
                self.away = Some((said, now + AWAY));
                return Some(now + AWAY);
            }
            Try::Press(_) => {}
            Try::Agent(agent) => {
                self.agent = agent;
                if let Tried::New(_, folder) = self.target {
                    self.target = Tried::New(agent, folder);
                }
            }
            Try::Pick(target) => (self.target, self.list) = (target, false),
            Try::Dismiss => {
                let open = if self.list { Some("target") } else { self.keys.then_some("help") };
                self.closed = open.map(|id| (id, now));
                (self.list, self.keys) = (false, false);
            }
        }
        None
    }

    /// The note and the pill's absence, gone by `now`.
    pub fn expire(&mut self, now: Instant) {
        if self.note.as_ref().is_some_and(|(_, until)| *until <= now) {
            self.note = None;
        }
        if self.away.as_ref().is_some_and(|(_, until)| *until <= now) {
            (self.away, self.back) = (None, self.back + 1);
        }
    }
}

/// What hears what's done to the pill to try.
pub type Act = Rc<dyn Fn(Try, &mut Window, &mut gpui::App)>;

/// A note by the pill: the overlay's (`ui::notes_column`), on Help's page.
fn note_view(text: String, t: Theme) -> Div {
    plain_glass(t, 7.)
        .h(px(26.))
        .px(px(9.))
        .flex()
        .items_center()
        .text_color(t.primary.opacity(0.86))
        .text_size(px(12.))
        .line_height(px(16.))
        .child(div().relative().whitespace_nowrap().child(text))
}

/// Help's pill to try, in the middle of `width`: its buttons light up under the pointer, their names
/// under it; the microphone mutes, the keys and the list open under it, a session is picked there;
/// Start over says so; sent or closed, it's away a moment. `act` hears what's done to it.
pub fn try_pill(width: f32, pill: &TryPill, look: &Look, act: Act) -> AnyElement {
    let t = look.t;
    let label = pill.target.label();
    let k = |name: &str| (look.key)(name);
    let named = |name: &str, key: String| if key.is_empty() { name.to_string() } else { format!("{name} ({key})") };
    let name_of = |id: &str| match id {
        "help" => named("Every key", k("help")),
        "mic" if pill.muted => format!("Muted (click to listen, or hold {})", k("talk")),
        "mic" => named("Listening (click to mute)", k("mic")),
        "clip" if pill.clip => "Recording a clip (click to stop)".into(),
        "clip" => named("Record a clip of what happens next, live", k("clip")),
        "target" => format!("Where it goes ({} switches, {} lists)", k("target"), k("sessions")),
        "restart" => named("Start over without sending", k("restart")),
        "done" => format!("Send ({} or {})", look.send, k("done")),
        _ => named("Close without sending", k("close")),
    };
    let button = |id: &'static str, face: Div| -> AnyElement {
        let (hovered, pressed) = (act.clone(), act.clone());
        // Lit as the overlay's: the main action lighter, the others a glass pill, unless already on.
        let lit = match id {
            "done" => Some(hex(style::UI_LIGHT, 1.0)),
            "mic" if !pill.muted => None,
            "clip" if pill.clip => None,
            "target" if pill.list => None,
            _ => Some(t.hover),
        };
        face.id(id)
            .cursor_pointer()
            .when_some(lit, |d, lit| d.hover(move |s| s.bg(lit)))
            .on_hover(move |on, window, cx| hovered(Try::Point(id, *on), window, cx))
            .on_click(move |_, window, cx| pressed(Try::Press(id), window, cx))
            .into_any_element()
    };
    let shown: AnyElement = match &pill.away {
        Some((said, _)) => controls::fade_in(
            div().h(px(PILL_HEIGHT)).flex().items_center().child(note_view(said.clone(), t)),
            gpui::SharedString::from(format!("tried-away-{}", pill.back)),
            0.,
        )
        .into_any_element(),
        None => {
            let face = faces::still_pill_with(
                Still {
                    help: pill.keys,
                    listening: !pill.muted,
                    level: 0.0,
                    recording_clip: pill.clip,
                    target: faces::target_name(Some(label.clone().into())).into_any_element(),
                    list_open: pill.list,
                    tool: None,
                },
                t,
                &button,
            );
            controls::fade_in(face, gpui::SharedString::from(format!("tried-pill-{}", pill.back)), 0.)
                .into_any_element()
        }
    };
    let row = |picks: Option<Try>, face: Div| -> AnyElement {
        let Some(picks) = picks else { return face.into_any_element() };
        let id = gpui::SharedString::from(format!("tried-{picks:?}"));
        let pressed = act.clone();
        face.id(id)
            .cursor_pointer()
            .hover(|s| s.bg(t.selected).border_color(t.selected_stroke))
            .on_click(move |_, window, cx| pressed(picks, window, cx))
            .into_any_element()
    };
    // Under it, from its left: the list or the keys, over what's below (on the page's color, which
    // would show through the glass), until a click outside them; else a note.
    let under: Option<AnyElement> = match (pill.away.is_some(), pill.list, pill.keys) {
        (false, true, _) => Some(list_view(None, pill.target, pill.agent, t, &row)),
        (false, _, true) => Some(keys_sheet_face(t, &look.keys, &look.freeze, &look.send, false).into_any_element()),
        _ => None,
    };
    let open = under.is_some();
    let panel = under.map(|under| {
        let id = if pill.list { "tried-list" } else { "tried-keys" };
        let dismiss = act.clone();
        let surface = div().rounded(px(12.)).bg(t.control_hover).child(under);
        deferred(
            div()
                .absolute()
                .top(px(PILL_HEIGHT + LIST_GAP as f32))
                .left_0()
                .block_mouse_except_scroll()
                .child(controls::fade_in(surface, id, -4.))
                .on_mouse_down_out(move |_, window, cx| dismiss(Try::Dismiss, window, cx)),
        )
        .with_priority(1)
    });
    let said = pill.note.as_ref().map(|(text, _)| text.clone());
    let said = said.or_else(|| (pill.away.is_none() && !open).then(|| pill.pointed.map(name_of)).flatten());
    let note = div().h(px(34.)).pt(px(8.)).flex().justify_center().children(said.map(|text| note_view(text, t)));
    div()
        .flex_none()
        .w(px(width))
        .flex()
        .flex_col()
        .items_center()
        .child(div().relative().h(px(PILL_HEIGHT)).child(shown).children(panel))
        .child(note)
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
            clip: None,
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
            clip: None,
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
    fn every_caption_goes_at_the_same_pace() {
        // A word once the characters before it (spaces too) have had their time.
        let text = "a [1] bc d";
        assert_eq!(words_said(text, Duration::ZERO), 1);
        assert_eq!(words_said(text, SAID_EACH * 2 - ms(1)), 1);
        assert_eq!(words_said(text, SAID_EACH * 2), 2);
        assert_eq!(words_said(text, SAID_EACH * 6), 3);
        assert_eq!(words_said(text, SAID_EACH * 9), 4);
        assert_eq!(words_said(text, Duration::from_secs(60)), 4);
        assert_eq!(saying(text), (SAID_EACH * 9 + SAID_READ).as_millis() as u64);
        // In every demonstration: each held until its last word, then as long to be read.
        for name in ["basics", "target", "card", "tools", "clip"] {
            for beat in super::demo(name, 600.0).unwrap().beats {
                let Some(said) = beat.say else { continue };
                assert_eq!(beat.hold, saying(said), "{name}");
                let last = Duration::from_millis(beat.hold) - SAID_READ;
                assert_eq!(words_said(said, last), said.split(' ').count(), "{name}");
                assert!(words_said(said, last - ms(1)) < said.split(' ').count(), "{name}");
            }
        }
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
        for width in [536.0, 700.0] {
            for name in ["basics", "target", "card", "tools", "clip"] {
                let demo = super::demo(name, width).unwrap_or_else(|| panic!("no demonstration {name}"));
                assert!(demo.beats.len() > 1, "{name}");
                assert!(demo.size.0 <= width, "{name} at {width}");
                for beat in &demo.beats {
                    // A card is a pin's, and a drag says what it draws.
                    if let Some(card) = &beat.scene.card {
                        assert!(beat.scene.pins.iter().any(|p| p.number == card.pin), "{name}");
                    }
                    assert_eq!(beat.path.len() > 1, beat.drag.is_some(), "{name}");
                }
                // The windows fill it, side by side.
                let right = demo.chat.or(demo.app).map(|(x, _, w, _)| x + w);
                assert!(right.is_none_or(|right| (right - demo.size.0).abs() < 0.01), "{name} at {width}");
            }
        }
        let steps: Vec<_> = super::demo("basics", 600.0).unwrap().beats.iter().filter_map(|b| b.step).collect();
        assert_eq!((steps.first(), steps.last()), (Some(&0), Some(&3)));
        // The same timing at any width (a step is watched from its start).
        let (narrow, wide) = (super::demo("basics", 536.0).unwrap(), super::demo("basics", 700.0).unwrap());
        assert_eq!(narrow.time_of_step(2), wide.time_of_step(2));
    }

    #[test]
    fn a_clip_records_from_its_key_to_its_square_and_brings_its_flash() {
        let width = 600.0;
        let demo = super::demo("clip", width).unwrap();
        let on = demo.beats.iter().position(|b| b.scene.clip).unwrap();
        assert_eq!(demo.beats[on].key.as_deref(), Some("clip"));
        let off = on + demo.beats[on..].iter().position(|b| !b.scene.clip).unwrap();
        // Stopped by a click on the pill's camera, left of where it goes, which says it's kept.
        let stop = &demo.beats[off];
        assert!(stop.click && stop.scene.note.is_some());
        let (x, y) = stop.pointer.unwrap();
        assert!(y < PILL_TOP + PILL_HEIGHT && x < width / 2.0 - NAME_LEFT_OF_MIDDLE - SESSION_NAME_WIDTH / 2.0);
        // The error shows while it records, as its click comes in, for an instant.
        let flash = demo.beats.iter().position(|b| b.scene.declined).unwrap();
        assert!(flash > on && flash < off && demo.beats[flash].click);
        // Playing starts with beat 0 in.
        let at = demo.comes_in(flash) - demo.comes_in(0) - FADE;
        let declined = |t: Duration| {
            let shown = demo.at(t);
            demo.beats[shown.beat].scene.declined && shown.since_in < FLASH
        };
        assert!(!declined(at - ms(5)) && declined(at + ms(5)) && declined(at + FLASH - ms(5)));
        assert!(!declined(at + FLASH + ms(5)));
        assert!(demo.at(at + ms(50)).moving);
        // Its frames at the times they play: the first, the error, back as it was; then asides' words.
        let clip = demo.clip.as_ref().unwrap();
        let since = |i: usize| (demo.comes_in(i) - demo.comes_in(on)).as_secs_f32();
        let times: Vec<f32> = clip.frames.iter().map(|f| f.time).collect();
        assert_eq!(times, [0.0, since(flash), since(flash) + FLASH.as_secs_f32()]);
        assert_eq!(clip.frames.iter().map(|f| f.declined).collect::<Vec<_>>(), [false, true, false]);
        let caption = &clip.caption;
        assert!(caption.starts_with("**Clip:** 3 frames, what changed in Safari over "), "{caption}");
        assert!(caption.contains(&format!("over {:.1} s", since(off))), "{caption}");
        assert!(caption.ends_with("Frame 2 showed for only an instant."), "{caption}");
        // Said after, then sent.
        assert!(demo.beats[off..].iter().any(|b| b.say.is_some()));
        assert!(demo.beats.last().is_some_and(|b| b.scene.sent));
    }

    #[test]
    fn what_is_marked_on_the_app_is_inside_its_border() {
        let app = (10.0, 20.0, 300.0, 200.0);
        // Drawn inside its 1-point border: the button at (16, 140) from there.
        assert_eq!(app_button(app), (27.0, 161.0, 92.0, 42.0));
        let rows = app_rows(app);
        assert_eq!((rows.0, rows.2), (23.0, 300.0 - 2.0 - 24.0));
        // Around both rows, a little past them.
        assert!(rows.1 < 20.0 + 1.0 + APP_ROW_TOPS[0]);
        assert!(rows.1 + rows.3 > 20.0 + 1.0 + APP_ROW_TOPS[1] + APP_ROW_HEIGHT);
    }

    #[test]
    fn the_pill_to_try_does_what_the_overlays_does() {
        let now = Instant::now();
        let mut pill = TryPill::default();
        assert_eq!(pill.apply(Try::Press("mic"), now), None);
        assert!(pill.muted);
        // The list and the keys take turns under it.
        pill.apply(Try::Press("target"), now);
        pill.apply(Try::Press("help"), now);
        assert!(pill.keys && !pill.list);
        // A new session: the agent picked first, then the directory; a later agent follows.
        pill.apply(Try::Press("target"), now);
        pill.apply(Try::Agent(1), now);
        pill.apply(Try::Pick(Tried::New(1, 0)), now);
        assert!(!pill.list);
        assert_eq!(pill.target.label(), "New Codex (shop)");
        pill.apply(Try::Agent(2), now);
        assert_eq!(pill.target.label(), "New OpenCode (shop)");
        // Its buttons' names: only the one the pointer's still on.
        pill.apply(Try::Point("mic", true), now);
        pill.apply(Try::Point("restart", true), now);
        pill.apply(Try::Point("mic", false), now);
        assert_eq!(pill.pointed, Some("restart"));
        // A clip recorded, then stopped: it's in the aside.
        assert_eq!(pill.apply(Try::Press("clip"), now), None);
        assert!(pill.clip);
        let until = pill.apply(Try::Press("clip"), now).unwrap();
        assert!(!pill.clip);
        assert_eq!(pill.note.as_ref().map(|(text, _)| text.as_str()), Some("The clip is in the aside"));
        pill.expire(until);
        // Started over: a note, until it's gone.
        let until = pill.apply(Try::Press("restart"), now).unwrap();
        assert_eq!(pill.note.as_ref().map(|(text, _)| text.as_str()), Some("Started over"));
        pill.expire(until - ms(1));
        assert!(pill.note.is_some());
        pill.expire(until);
        assert!(pill.note.is_none());
        // A click outside the list closes it; that click on where it goes doesn't open it again, a
        // later one does.
        pill.apply(Try::Press("target"), now);
        pill.apply(Try::Dismiss, now);
        pill.apply(Try::Press("target"), now + ms(80));
        assert!(!pill.list);
        pill.apply(Try::Press("target"), now + ms(900));
        assert!(pill.list);
        pill.apply(Try::Pick(Tried::New(2, 0)), now);
        // Sent: away, saying where, then back (fading in again).
        let until = pill.apply(Try::Press("done"), now).unwrap();
        assert_eq!(pill.away.as_ref().map(|(text, _)| text.as_str()), Some("Sent to New OpenCode (shop)"));
        assert_eq!(pill.pointed, None);
        pill.expire(until);
        assert_eq!((pill.away.is_none(), pill.back), (true, 1));
    }
}
