//! The overlay's windows: one per screen, borderless, above everything, showing the frozen screen
//! and what's on it, with one toolbar for the four tools (Measure, Annotate, Draw, Shot), the
//! tolerance, the units, talking, copying, and adding it all to the aside.
//!
//! `Overlay` holds the session (`asides_core::overlay::Session`) and everything around it; each
//! window's `ScreenView` draws its screen from it and feeds it the mouse and keyboard.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use asides_core::config::OverlayKeys;
use asides_core::helper::{Command, Event, Item, Spec, SpecScreen};
use asides_core::inspect::{Annotation, Axis, Element, Intent, Measurement, PtRect, Said, Severity, Shape, num};
use asides_core::keys::{self, Chord, Mods};
use asides_core::measure::{Frame, Tolerance};
use asides_core::overlay::{MARKER, Part, Placed, Saved, Session, Tool, lookups};
use gpui::{
    AnyElement, AnyWindowHandle, App, BorderStyle, Bounds, Context, CursorStyle, DispatchPhase, Div, Entity,
    FocusHandle, Focusable, FontWeight, Hitbox, HitboxBehavior, Hsla, ImageSource, KeyDownEvent, KeyUpEvent, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point,
    Render, RenderImage, Rgba, SharedString, Window, canvas, deferred, div, img, point, prelude::*, px, quad, size,
    svg, transparent_black,
};

use crate::export::{self, Exporter};
use crate::input::TextInput;
use crate::windows::TOOLBAR_LIFT;
use crate::{finish, io, style};

/// How often the overlay looks at the app's commands, the clock and the hover.
const TICK: Duration = Duration::from_millis(33);
/// ...and, while the frozen screens give way, whether the system is back with the user's app.
const HANDOFF_TICK: Duration = Duration::from_millis(8);
/// Annotate: how long the cursor rests before the element under it is looked up.
const HOVER_REST: Duration = Duration::from_millis(90);
const TOAST: Duration = Duration::from_millis(1600);
/// A recording about an annotation ended: shown as being transcribed this long at most, until the
/// app says it is (it doesn't for one too short to keep).
const AWAIT_TRANSCRIPT: Duration = Duration::from_millis(1500);
/// Esc pressed again within this closes without adding anything.
const CLOSE_AGAIN: Duration = Duration::from_millis(2500);
/// In a card, Space held this long talks about its annotation; shorter, it types a space.
const SPACE_HOLD: Duration = Duration::from_millis(300);
const NOTICE: Duration = Duration::from_millis(5000);
const CARD_WIDTH: f64 = 440.0;
/// The card's trash button and the gap before it.
const TRASH_WIDTH: f64 = 34.0;
/// The card opens above its annotation when there's less than this below it.
const CARD_ROOM: f64 = 240.0;
const MENU_WIDTH: f64 = 260.0;

/// A screen's window: where it is and what part of the screen it shows (all of it, unless
/// windowed for development).
pub struct ScreenWindow {
    pub handle: Option<AnyWindowHandle>,
    pub focus: Option<FocusHandle>,
    /// The screen it's on (AppKit's coordinates).
    pub screen: PtRect,
    /// The screen point at the window's top-left corner.
    pub offset: (f64, f64),
    pub size: (f64, f64),
    /// AppKit's window number, for screenshots of it (development).
    pub number: isize,
    /// Its view, for development replays.
    pub view: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Cmd {
    Tool(Tool),
    HoldH,
    HoldV,
    GuideH,
    GuideV,
    TolUp,
    TolDown,
    TolHigh,
    Units,
    CopyDims,
    CopyImage,
    CopyMarkdown,
    Undo,
    Clear,
    Hide,
    Toolbar,
    Restore,
    Help,
    Done,
    DoneWhole,
    Send,
    Live,
    Close,
}

/// The overlay's keys, parsed.
struct Keys {
    list: Vec<(Vec<Chord>, Cmd)>,
    talk: Vec<Chord>,
}

impl Keys {
    fn new(k: &OverlayKeys) -> Keys {
        let p = keys::parse;
        Keys {
            list: vec![
                (p(&k.measure), Cmd::Tool(Tool::Measure)),
                (p(&k.annotate), Cmd::Tool(Tool::Annotate)),
                (p(&k.draw), Cmd::Tool(Tool::Draw)),
                (p(&k.shot), Cmd::Tool(Tool::Shot)),
                (p(&k.guide_horizontal), Cmd::GuideH),
                (p(&k.guide_vertical), Cmd::GuideV),
                (p(&k.hold_horizontal), Cmd::HoldH),
                (p(&k.hold_vertical), Cmd::HoldV),
                (p(&k.tolerance_up), Cmd::TolUp),
                (p(&k.tolerance_down), Cmd::TolDown),
                (p(&k.tolerance_high), Cmd::TolHigh),
                (p(&k.units), Cmd::Units),
                (p(&k.copy_markdown), Cmd::CopyMarkdown),
                (p(&k.copy_dimensions), Cmd::CopyDims),
                (p(&k.copy_image), Cmd::CopyImage),
                (p(&k.undo), Cmd::Undo),
                (p(&k.clear), Cmd::Clear),
                (p(&k.hide), Cmd::Hide),
                (p(&k.toolbar), Cmd::Toolbar),
                (p(&k.restore), Cmd::Restore),
                (p(&k.help), Cmd::Help),
                (p(&k.send), Cmd::Send),
                (p(&k.done_whole_screen), Cmd::DoneWhole),
                (p(&k.done), Cmd::Done),
                (p(&k.live), Cmd::Live),
                (p(&k.close), Cmd::Close),
            ],
            talk: p(&k.talk),
        }
    }

    fn command(&self, key: &str, mods: Mods) -> Option<Cmd> {
        self.list.iter().find(|(chords, _)| keys::matches(chords, key, mods)).map(|(_, c)| *c)
    }

    fn label(&self, cmd: Cmd) -> String {
        self.list.iter().find(|(_, c)| *c == cmd).map(|(chords, _)| keys::label(chords)).unwrap_or_default()
    }
}

struct Card {
    screen: usize,
    index: usize,
    number: u32,
    /// The annotation's key: what's recorded while the card is open is about it.
    key: u32,
    input: Entity<TextInput>,
    intent: Option<Intent>,
    severity: Option<Severity>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuItem {
    Cmd(Cmd),
    Tool(Tool),
    RemoveAnnotation(usize),
    RemoveHeld(usize),
    AnnotateHeld(usize),
    AnnotateHere,
    ShotOfHeld(usize),
    EditAnnotation(usize),
}

struct Menu {
    screen: usize,
    at: (f64, f64),
    items: Vec<(String, String, MenuItem)>,
    /// The row under the pointer (its text turns dark on the UI color).
    hovered: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
enum Lookup {
    Annotation { screen: usize, index: usize, number: u32, part: Part },
    Hover { screen: usize },
}

struct Hovered {
    screen: usize,
    frame: PtRect,
    name: String,
}

/// A freeze's work, for the aside: drawing its images takes a moment, done once the live screen
/// shows.
struct Keeping {
    session: Session,
    frames: Vec<Arc<Frame>>,
    exporter: Option<Exporter>,
    shots: Vec<(f32, String)>,
    /// Which freeze it was, and the screen under the pointer.
    freeze: usize,
    screen: usize,
}

pub struct Overlay {
    pub session: Session,
    spec: Spec,
    keys: Keys,
    accent: u32,
    images: Vec<Arc<RenderImage>>,
    frames: Vec<Arc<Frame>>,
    exporter: Option<Exporter>,
    pub windows: Vec<ScreenWindow>,
    dir: PathBuf,
    opened: Instant,
    card: Option<Card>,
    menu: Option<Menu>,
    /// A short message at the bottom, and until when.
    toast: Option<(SharedString, Instant)>,
    /// The keys, shown with `?` (or while the pointer is on the toolbar's `?`).
    help: bool,
    help_hover: bool,
    /// The toolbar is shown (`Cmd::Toolbar` hides it; every action has a key).
    toolbar: bool,
    /// Seconds recorded when the app last said so, and when it did.
    recording: Option<(f32, Instant)>,
    /// The annotation that recording is about (started from its card).
    recording_for: Option<u32>,
    /// Recordings about annotations being transcribed: (annotation key, take).
    transcribing: Vec<(u32, u64)>,
    /// A recording about this annotation just ended, and when (until the app says it's being
    /// transcribed): the card shows it as such, without a gap.
    awaiting: Option<(u32, Instant)>,
    shown_seconds: u32,
    /// Where the cursor last moved and when, for Annotate's hover lookup.
    rested: Option<(usize, f64, f64, Instant)>,
    hover_asked: Option<(usize, f64, f64)>,
    hovered: Option<Hovered>,
    pending: HashMap<u64, Lookup>,
    next_id: u64,
    talking: bool,
    /// A toolbar button's name while the pointer is on it.
    hint: Option<SharedString>,
    /// The card's dot under the pointer, named above the card.
    card_tip: Option<SharedString>,
    /// Screenshots taken: when, and the file in the session directory.
    shots: Vec<(f32, String)>,
    shot_count: usize,
    commands: Receiver<Command>,
    done: bool,
    /// Over the live screen: only the toolbar shows, in a window of its own, and the user's apps
    /// take the mouse and the keyboard. Frozen, the screens show, and take them.
    pub live: bool,
    /// The app was asked to freeze the screens, and the tool to pick then.
    freezing: Option<Option<Tool>>,
    /// The toolbar's window over the live screen, and the screen it's on.
    pub toolbar_window: Option<ScreenWindow>,
    pub toolbar_screen: usize,
    /// Which windows show: the toolbar's (`Some(true)`, live) or the screens' (`Some(false)`).
    pub shown: Option<bool>,
    /// Where the user put the toolbar (its top-left corner, as a fraction of the screen), and
    /// folded to its handle and microphone.
    pub toolbar_at: Option<(f64, f64)>,
    pub collapsed: bool,
    /// The toolbar's bounds in its window, as last drawn: on a frozen screen, and over the live one.
    pub toolbar_bounds: Option<Bounds<Pixels>>,
    pub live_toolbar_bounds: Option<Bounds<Pixels>>,
    /// The toolbar being moved: where it was grabbed, from its top-left corner.
    pub moving_toolbar: Option<(f64, f64)>,
    /// The keys still to show, the first time a screen freezes.
    tips: bool,
    /// Space down in a card: since when, and whether it's talking yet (`SPACE_HOLD`).
    space_hold: Option<(Instant, bool)>,
    /// Something was said in the session (there's something to lose by closing it).
    talked: bool,
    /// Esc pressed once, with something to lose: when.
    close_armed: Option<Instant>,
    /// What earlier freezes add to the aside, and how many there were.
    kept: Vec<Item>,
    freezes: usize,
    /// The last freeze, still to be kept (`Keeping`).
    keeping: Option<Keeping>,
    /// The last freeze's guides and held measurements, for next time.
    saved: Saved,
    /// The app the user was in when the screens froze, for the keyboard to go back to.
    pub previous_app: Option<i32>,
    /// The last freeze's screens, to drop from the windows once the new ones are drawn.
    pub stale_images: Vec<Arc<RenderImage>>,
    /// The part of each screen the Dock and the menu bar leave (AppKit's coordinates), measured
    /// over the live screen: the toolbar sits as high above the Dock, frozen or live, and the frozen
    /// screens give way once they're back (`windows::Handoff`).
    pub visible: Vec<PtRect>,
    /// The frozen screens are giving way (to the live screen, or closing): the system goes back to
    /// the app the user was in first.
    pub handoff: Option<crate::windows::Handoff>,
    /// Done or closed while frozen: the overlay quits once it has handed the system back.
    pub closing: bool,
    /// The live toolbar's window (AppKit's coordinates) while the frozen screens give way: they draw
    /// the toolbar there too, so it shows in the same frame as they lose theirs (`windows::show_live`).
    pub live_toolbar_frame: Option<PtRect>,
    /// The frozen screens just showed: the system is the overlay's once they're on screen (taken
    /// at once, the menu bar hid a frame before they covered it), and since when.
    pub taking_over: Option<Instant>,
    /// Windows opened for the screens before the session (`windows::Spare`).
    pub spares: Vec<crate::windows::Spare>,
}

impl Overlay {
    pub fn new(spec: Spec, frames: Vec<Frame>, dir: PathBuf, commands: Receiver<Command>) -> Overlay {
        // The session's pixels: shared, a screen's are tens of megabytes.
        let frames: Vec<Arc<Frame>> = frames.into_iter().map(Arc::new).collect();
        let screens = spec.screens.iter().zip(&frames).map(|(s, f)| Session::from_spec(s, f.clone())).collect();
        let mut session = Session::new(screens, Tolerance(spec.tolerance.min(100)), spec.units);
        session.tool = Tool::Measure;
        // Opened over the live screen, there are no frozen screens yet.
        session.cursor = pointer(&spec).filter(|_| !spec.live);
        let images = frames.iter().map(|f| render_image(f)).collect();
        let visible = visible_frames(&spec.screens);
        let accent = style::rgb_hex(style::parse_color(&spec.color));
        let recording = spec.recording.map(|s| (s, Instant::now()));
        Overlay {
            session,
            keys: Keys::new(&spec.keys),
            accent,
            images,
            frames,
            exporter: None,
            windows: Vec::new(),
            dir,
            opened: Instant::now(),
            card: None,
            menu: None,
            toast: None,
            help: spec.tips && !spec.live,
            help_hover: false,
            toolbar: spec.toolbar,
            recording,
            recording_for: None,
            transcribing: Vec::new(),
            awaiting: None,
            shown_seconds: 0,
            rested: None,
            hover_asked: None,
            hovered: None,
            pending: HashMap::new(),
            next_id: 1,
            talking: false,
            hint: None,
            card_tip: None,
            shots: Vec::new(),
            shot_count: 0,
            commands,
            done: false,
            live: spec.live,
            freezing: None,
            toolbar_window: None,
            toolbar_screen: 0,
            shown: None,
            toolbar_at: spec.toolbar_at,
            collapsed: spec.toolbar_collapsed,
            toolbar_bounds: None,
            live_toolbar_bounds: None,
            moving_toolbar: None,
            tips: spec.tips && spec.live,
            space_hold: None,
            talked: spec.recording.is_some(),
            close_armed: None,
            kept: Vec::new(),
            freezes: 0,
            keeping: None,
            saved: Saved::default(),
            previous_app: None,
            stale_images: Vec::new(),
            visible,
            handoff: None,
            closing: false,
            live_toolbar_frame: None,
            taking_over: None,
            spares: Vec::new(),
            spec,
        }
    }

    pub fn spec(&self) -> &Spec {
        &self.spec
    }

    // --- Live and frozen ---

    /// Freezes the screens to work on them (the app captures them), picking `tool` then.
    pub fn freeze(&mut self, tool: Option<Tool>) {
        if !self.live {
            if let Some(tool) = tool {
                self.session.set_tool(tool);
            }
            return;
        }
        if self.freezing.is_none() {
            io::emit(&Event::Freeze);
        }
        self.freezing = Some(tool.or(self.freezing.flatten()));
    }

    /// The screens, frozen: the work goes on on them, numbered on from the last freeze.
    fn frozen(&mut self, screens: Vec<SpecScreen>) {
        self.keep_last();
        let tool = self.freezing.take().flatten();
        let mut frames = Vec::new();
        for s in &screens {
            match io::pixels(&self.dir, s) {
                Ok(frame) => frames.push(Arc::new(frame)),
                Err(_) => return self.notice("Could not freeze the screen."),
            }
        }
        let states = screens.iter().zip(&frames).map(|(s, f)| Session::from_spec(s, f.clone())).collect();
        self.session = self.session.refrozen(states);
        if let Some(tool) = tool {
            self.session.set_tool(tool);
        }
        let images = frames.iter().map(|f| render_image(f)).collect();
        self.stale_images.extend(std::mem::replace(&mut self.images, images));
        self.frames = frames;
        self.exporter = None;
        // Over the live screen still: the Dock is where the user keeps it.
        self.visible = visible_frames(&screens);
        self.spec.screens = screens;
        // Measure's lines start where the pointer is, not where it was when the screen last froze.
        self.session.cursor = pointer(&self.spec);
        (self.rested, self.hover_asked) = (None, None);
        self.live = false;
        self.help = std::mem::take(&mut self.tips);
        self.hint = None;
        self.set_hovered(None);
    }

    /// Back to the live screen: what was done on the frozen one goes in the aside, in order (once
    /// the live screen shows: `keep_last`).
    fn go_live(&mut self, cx: &mut Context<Self>) {
        if self.live || self.done {
            return;
        }
        self.put_card_away(cx);
        (self.menu, self.help, self.hint) = (None, false, None);
        self.keeping = Some(self.this_freeze());
        self.live = true;
        io::emit(&Event::Live);
    }

    /// This freeze's work, taken to be kept.
    fn this_freeze(&mut self) -> Keeping {
        let keeping = Keeping {
            session: self.session.clone(),
            frames: std::mem::take(&mut self.frames),
            exporter: self.exporter.take(),
            shots: std::mem::take(&mut self.shots),
            freeze: self.freezes,
            screen: self.cursor_screen(),
        };
        self.saved = keeping.session.saved();
        self.freezes += 1;
        keeping
    }

    /// The last freeze, kept for the aside if it isn't yet.
    fn keep_last(&mut self) {
        if let Some(keeping) = self.keeping.take()
            && let Err(err) = self.keep(keeping, false, false)
        {
            eprintln!("could not keep the frozen screen: {err:#}");
        }
    }

    /// What was done on a freeze, kept for the aside (`alone`: the screen even with nothing on it;
    /// `whole`: all of it, not just around what's there).
    fn keep(&mut self, k: Keeping, alone: bool, whole: bool) -> anyhow::Result<()> {
        if k.session.inspections().is_empty() && k.shots.is_empty() && !alone {
            return Ok(());
        }
        let exporter = k.exporter.unwrap_or_else(|| {
            let frames: Vec<&Frame> = k.frames.iter().map(|f| &**f).collect();
            Exporter::new(&frames, style::parse_color(&self.spec.color))
        });
        let items =
            finish::items(&k.session, &exporter, &self.spec, &self.dir, whole, &k.shots, k.screen, k.freeze, alone)?;
        self.kept.extend(items);
        Ok(())
    }

    /// The toolbar's handle, pressed at `at` (in its window): it follows the pointer until let go.
    fn grab_toolbar(&mut self, at: Point<Pixels>, live: bool) {
        let bounds = if live { self.live_toolbar_bounds } else { self.toolbar_bounds };
        let origin = bounds.map_or(at, |b| b.origin);
        self.moving_toolbar = Some((f64::from(f32::from(at.x - origin.x)), f64::from(f32::from(at.y - origin.y))));
    }

    /// The moved or folded toolbar, remembered for next time.
    pub fn placed_toolbar(&mut self) {
        io::emit(&Event::ToolbarPlaced { at: self.toolbar_at, collapsed: self.collapsed });
    }

    pub fn start(&mut self) {
        self.opened = Instant::now();
    }

    fn elapsed(&self) -> f32 {
        self.opened.elapsed().as_secs_f32()
    }

    fn exporter(&mut self) -> &Exporter {
        if self.exporter.is_none() {
            let frames: Vec<&Frame> = self.frames.iter().map(|f| &**f).collect();
            self.exporter = Some(Exporter::new(&frames, style::parse_color(&self.spec.color)));
        }
        self.exporter.as_ref().unwrap()
    }

    fn cursor_screen(&self) -> usize {
        self.session.cursor.map_or(0, |c| c.0)
    }

    fn toast(&mut self, text: impl Into<SharedString>) {
        self.toast = Some((text.into(), Instant::now() + TOAST));
    }

    /// A message the user must read (an error from the app): it stays longer.
    fn notice(&mut self, text: impl Into<SharedString>) {
        self.toast = Some((text.into(), Instant::now() + NOTICE));
    }

    /// Without a bound directory there's nowhere to record into or add the aside to: say so
    /// instead of trying.
    fn unbound(&mut self) -> bool {
        if self.spec.bound.is_none() {
            self.notice("Not bound to a repo: run `asides` in your project first. ⇧⌘C copies this as Markdown.");
            return true;
        }
        false
    }

    /// Measure's modifiers. With ⌃ down, they're part of one of Asides' shortcuts (⌃⌥P, ⌃⌥Space),
    /// not for Measure: its lines don't change while it's pressed.
    fn set_modifiers(&mut self, m: &Modifiers) {
        let measure = !m.control;
        self.session.crosshair = m.shift && measure;
        self.session.snapping = !(m.platform && measure);
        self.session.alternate = m.alt && measure;
    }

    // --- Mouse ---

    fn mouse_down(
        &mut self,
        screen: usize,
        (x, y): (f64, f64),
        e: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        crate::native::debug(&format!("mouse down at {x},{y}"));
        // Live, the frozen screens are only still up while they give way: what's done is kept.
        if self.done || self.live {
            return;
        }
        self.session.clock = self.elapsed();
        self.set_modifiers(&e.modifiers);
        if self.menu.take().is_some() || self.help {
            self.help = false;
            return cx.notify();
        }
        // Closing the card, the click goes on: to the next annotation, or whatever it's on. What
        // was said about this one is still being transcribed meanwhile, for it.
        if self.card.is_some() {
            self.close_card(window, cx);
        }
        // Control-click is a right-click.
        let right = e.button == MouseButton::Right || (e.button == MouseButton::Left && e.modifiers.control);
        if e.button != MouseButton::Left && e.button != MouseButton::Right {
            return;
        }
        self.session.mouse_down(screen, x, y, right);
        cx.notify();
    }

    fn mouse_move(&mut self, screen: usize, (x, y): (f64, f64), e: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.set_modifiers(&e.modifiers);
        if let Some((gx, gy)) = self.moving_toolbar {
            let s = &self.spec.screens[screen].screen;
            self.toolbar_at = Some((((x - gx) / s.width).clamp(0.0, 1.0), ((y - gy) / s.height).clamp(0.0, 1.0)));
            return cx.notify();
        }
        self.session.move_to(screen, x, y);
        self.rested = Some((screen, x, y, Instant::now()));
        if self.hovered.as_ref().is_some_and(|h| h.screen != screen || !h.frame.contains(x, y)) {
            self.set_hovered(None);
        }
        cx.notify();
    }

    fn mouse_up(
        &mut self,
        screen: usize,
        (x, y): (f64, f64),
        e: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.moving_toolbar.take().is_some() {
            self.placed_toolbar();
            return cx.notify();
        }
        if self.done || self.live || self.session.drag.is_none() {
            return;
        }
        self.set_modifiers(&e.modifiers);
        let right = self.session.drag.as_ref().is_some_and(|d| !d.snap);
        let click = self.session.drag.as_ref().is_some_and(|d| d.is_click());
        if right && click {
            self.session.drag = None;
            self.open_menu(screen, x, y);
            return cx.notify();
        }
        if right && self.session.tool != Tool::Measure {
            // A right-drag only means something to Measure (an area as drawn).
            self.session.drag = None;
            return cx.notify();
        }
        let placed = self.session.mouse_up(screen, x, y, self.elapsed());
        self.placed(placed, window, cx);
        cx.notify();
    }

    fn placed(&mut self, placed: Placed, window: &mut Window, cx: &mut Context<Self>) {
        match placed {
            Placed::Annotation { screen, index } => {
                self.look_up(screen, index);
                // While the user talks, an annotation is enough: what they say goes with it. Drawings
                // come in quick strokes: their card opens on a click on their number.
                let drawing = self.session.screens[screen].annotations.get(index).is_some_and(|a| a.shape.is_some());
                if self.recording.is_none() && !drawing {
                    self.open_card(screen, index, window, cx);
                }
            }
            Placed::Edit { screen, index } => self.open_card(screen, index, window, cx),
            Placed::Shot { screen, rect } => self.take_shot(screen, rect),
            Placed::Measurement { .. } | Placed::Nothing => {}
        }
    }

    /// Asks the app what a new annotation is on.
    fn look_up(&mut self, screen: usize, index: usize) {
        let Some(a) = self.session.screens[screen].annotations.get(index) else { return };
        let (ox, oy) = self.spec.screens[screen].origin;
        for (part, x, y) in lookups(a) {
            let id = self.next_id;
            self.next_id += 1;
            self.pending.insert(id, Lookup::Annotation { screen, index, number: a.number, part });
            io::emit(&Event::Identify { id, x: ox + x, y: oy + y });
        }
    }

    fn take_shot(&mut self, screen: usize, rect: PtRect) {
        let time = self.elapsed();
        let session = self.session.clone();
        // Full size on the clipboard; for the aside, no larger than the agent reads.
        let Some(png) = self.exporter().render(&session, screen, rect) else { return };
        let small = self.exporter().render_within(&session, screen, rect, export::ASIDE_MAX_SIDE);
        self.shot_count += 1;
        let name = format!("shot-{}.png", self.shot_count);
        if std::fs::write(self.dir.join(&name), small.as_deref().unwrap_or(&png)).is_ok() {
            self.shots.push((time, name));
        }
        export::copy_png(&png);
        io::emit(&Event::Copied);
        self.toast(format!("Screenshot copied · {} × {}", self.n(screen, rect.width), self.n(screen, rect.height)));
    }

    fn n(&self, screen: usize, v: f64) -> String {
        num(v, self.session.screens[screen].screen.scale, self.session.units)
    }

    // --- The context menu (right-click) ---

    fn open_menu(&mut self, screen: usize, x: f64, y: f64) {
        let k = |c: Cmd| self.keys.label(c);
        let mut items = Vec::new();
        let marker = self.session.marker_at(screen, x, y);
        if let Some(index) = marker {
            let number = self.session.screens[screen].annotations[index].number;
            items.push((format!("Edit Annotation {number}"), String::new(), MenuItem::EditAnnotation(index)));
            items.push((format!("Remove Annotation {number}"), String::new(), MenuItem::RemoveAnnotation(index)));
        }
        // On a measured section, annotating means that section; elsewhere, what's under the pointer.
        match self.session.held_at(screen, x, y) {
            Some(index) => items.extend([
                ("Annotate This".to_string(), String::new(), MenuItem::AnnotateHeld(index)),
                ("Screenshot of This".into(), String::new(), MenuItem::ShotOfHeld(index)),
                ("Remove This Measurement".into(), String::new(), MenuItem::RemoveHeld(index)),
            ]),
            None if marker.is_none() => items.push(("Annotate Here".into(), String::new(), MenuItem::AnnotateHere)),
            None => {}
        }
        items.extend([
            ("Add Horizontal Guide".to_string(), k(Cmd::GuideH), MenuItem::Cmd(Cmd::GuideH)),
            ("Add Vertical Guide".into(), k(Cmd::GuideV), MenuItem::Cmd(Cmd::GuideV)),
            ("Hold Horizontal Distance".into(), k(Cmd::HoldH), MenuItem::Cmd(Cmd::HoldH)),
            ("Hold Vertical Distance".into(), k(Cmd::HoldV), MenuItem::Cmd(Cmd::HoldV)),
            ("Take a Screenshot".into(), k(Cmd::Tool(Tool::Shot)), MenuItem::Tool(Tool::Shot)),
            ("Copy as Markdown".into(), k(Cmd::CopyMarkdown), MenuItem::Cmd(Cmd::CopyMarkdown)),
        ]);
        self.menu = Some(Menu { screen, at: (x, y), items, hovered: None });
    }

    fn menu_item(&mut self, item: MenuItem, window: &mut Window, cx: &mut Context<Self>) {
        let Some(menu) = self.menu.take() else { return };
        let (screen, (x, y)) = (menu.screen, menu.at);
        self.session.clock = self.elapsed();
        // Guides and holds go where the menu was opened.
        self.session.move_to(screen, x, y);
        match item {
            MenuItem::Cmd(cmd) => self.command(cmd, window, cx),
            MenuItem::Tool(tool) => self.session.set_tool(tool),
            MenuItem::RemoveAnnotation(index) => self.session.remove_annotation(screen, index),
            MenuItem::RemoveHeld(index) => self.session.remove_held(screen, index),
            MenuItem::EditAnnotation(index) => self.open_card(screen, index, window, cx),
            MenuItem::ShotOfHeld(index) => self.shot_of_held(screen, index),
            MenuItem::AnnotateHeld(index) => {
                let placed = self.session.annotate_held(screen, index, self.elapsed());
                self.placed(placed, window, cx);
            }
            MenuItem::AnnotateHere => {
                let placed = self.session.annotate_at(screen, x, y, self.elapsed());
                self.placed(placed, window, cx);
            }
        }
        cx.notify();
    }

    fn shot_of_held(&mut self, screen: usize, index: usize) {
        let padding = self.spec.padding.clamp(0.0, 24.0);
        if let Placed::Shot { screen, rect } = self.session.shot_of_held(screen, index, padding, self.elapsed()) {
            self.take_shot(screen, rect);
        }
    }

    // --- The card (an annotation's feedback) ---

    fn open_card(&mut self, screen: usize, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = self.session.screens[screen].annotations.get(index) else { return };
        let (comment, number, key, intent, severity) = (a.comment.clone(), a.number, a.key, a.intent, a.severity);
        let input = cx.new(|cx| TextInput::new(&comment, "What should change?", cx));
        // The card opens where the click was: `window` is that screen's.
        window.focus(&input.read(cx).focus_handle(cx));
        self.card = Some(Card { screen, index, number, key, input, intent, severity });
    }

    /// Closes the card, keeping what was typed; the annotation stays, and a click on its number
    /// opens the card again. A recording for it ends with it.
    fn close_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(screen) = self.put_card_away(cx) else { return };
        if let Some(focus) = self.windows.get(screen).and_then(|w| w.focus.clone()) {
            window.focus(&focus);
        }
    }

    /// The card closed (see `close_card`); the screen it was on.
    fn put_card_away(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let card = self.card.take()?;
        let still_there =
            self.session.screens[card.screen].annotations.get(card.index).is_some_and(|a| a.number == card.number);
        if still_there {
            let text = card.input.read(cx).text();
            self.session.set_feedback(card.screen, card.index, &text, card.intent, card.severity);
        }
        if self.recording.is_some() && self.recording_for == Some(card.key) {
            io::emit(&Event::Stop);
            // Stopped: the next annotation's card opens right away.
            (self.talking, self.recording, self.recording_for) = (false, None, None);
        }
        Some(card.screen)
    }

    fn annotation(&mut self, key: u32) -> Option<&mut Annotation> {
        self.session.screens.iter_mut().flat_map(|s| s.annotations.iter_mut()).find(|a| a.key == key)
    }

    /// The card's trash: what was said about the annotation goes, to say it again; still being said,
    /// that recording goes too.
    fn unsay(&mut self, key: u32) {
        if let Some(a) = self.annotation(key) {
            a.said.clear();
        }
        self.transcribing.retain(|&(k, _)| k != key);
        self.awaiting = self.awaiting.filter(|&(k, _)| k != key);
        if self.recording_for == Some(key) {
            (self.talking, self.recording, self.recording_for) = (false, None, None);
        }
        io::emit(&Event::Unsay { annotation: key });
    }

    // --- Keys ---

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        crate::native::debug(&format!("key down {:?} card={}", e.keystroke, self.card.is_some()));
        if self.done || self.live {
            return;
        }
        let key = keys::normalize(&e.keystroke.key);
        let m = &e.keystroke.modifiers;
        let mods = Mods { cmd: m.platform, shift: m.shift, alt: m.alt, ctrl: m.control };
        self.session.clock = self.elapsed();
        if self.card.is_some() {
            // Space held talks about the annotation; tapped, it's a space (typed when it's let go,
            // or when the next key comes first).
            let plain = !m.platform && !m.alt && !m.control;
            if key == "space" && plain && (self.space_hold.is_some() || self.recording.is_none()) {
                if !e.is_held && self.space_hold.is_none() {
                    self.space_hold = Some((Instant::now(), false));
                }
                cx.stop_propagation();
                return;
            }
            self.type_held_space(cx);
            // ⌘1–⌘4: what the annotation asks for (the card's dots).
            let intent = (m.platform && !m.shift && !m.alt)
                .then(|| key.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| Intent::ALL.get(i)))
                .flatten();
            match key.as_str() {
                "escape" => self.close_card(window, cx),
                "enter" if !m.shift => self.close_card(window, cx),
                _ if intent.is_some() => self.choose_intent(*intent.unwrap()),
                _ => return,
            }
            cx.stop_propagation();
            return cx.notify();
        }
        if key == "escape" && (self.menu.is_some() || self.help) {
            self.menu = None;
            self.help = false;
            return cx.notify();
        }
        // Space while dragging moves the area; otherwise it's the talk key.
        if key == "space" && self.session.drag.is_some() {
            self.session.moving = true;
            return;
        }
        if keys::matches(&self.keys.talk, &key, mods) {
            if !e.is_held && !self.talking && !self.unbound() {
                self.talking = true;
                io::emit(&Event::Talk { down: true, annotation: None });
            }
            return cx.notify();
        }
        if let Some(delta) = arrow(&key) {
            let step = if m.shift { 10.0 } else { 1.0 / self.session.screens[self.cursor_screen()].screen.scale };
            self.session.nudge_guide(delta * step);
            return cx.notify();
        }
        if let Some(cmd) = self.keys.command(&key, mods) {
            self.help = self.help && cmd == Cmd::Help;
            match cmd {
                Cmd::Close => self.close_by_key(cx),
                _ => self.command(cmd, window, cx),
            }
            cx.notify();
        }
    }

    fn key_up(&mut self, e: &KeyUpEvent, cx: &mut Context<Self>) {
        let key = keys::normalize(&e.keystroke.key);
        if key == "space"
            && let Some((_, talking)) = self.space_hold
        {
            if talking {
                self.space_hold = None;
                self.talking = false;
                io::emit(&Event::Stop);
            } else {
                self.type_held_space(cx);
            }
            return cx.notify();
        }
        if key == "space" {
            self.session.moving = false;
        }
        if self.talking && self.keys.talk.iter().any(|c| c.key == key) {
            self.talking = false;
            io::emit(&Event::Talk { down: false, annotation: None });
        }
        cx.notify();
    }

    fn command(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        let s = &mut self.session;
        match cmd {
            Cmd::Tool(tool) => {
                s.set_tool(tool);
                self.set_hovered(None);
            }
            Cmd::HoldH => drop(s.hold_axis(Axis::X)),
            Cmd::HoldV => drop(s.hold_axis(Axis::Y)),
            // A horizontal guide is a line across, placed by its y.
            Cmd::GuideH => drop(s.add_guide(Axis::Y)),
            Cmd::GuideV => drop(s.add_guide(Axis::X)),
            Cmd::TolUp => s.raise_tolerance(),
            Cmd::TolDown => s.lower_tolerance(),
            Cmd::TolHigh => s.toggle_high_tolerance(),
            Cmd::Units => s.toggle_units(),
            Cmd::CopyDims => {
                if let Some(text) = s.dimensions() {
                    export::copy_text(&text);
                    io::emit(&Event::Copied);
                    self.toast(format!("Copied {text}"));
                }
            }
            Cmd::CopyImage => {
                let screen = self.cursor_screen();
                let crop = export::crop(&self.session, screen, self.spec.padding, false);
                let session = self.session.clone();
                if let Some(png) = self.exporter().render(&session, screen, crop) {
                    export::copy_png(&png);
                    io::emit(&Event::Copied);
                    self.toast("Image copied");
                }
            }
            Cmd::CopyMarkdown => {
                let text = self.session.markdown(self.spec.detail);
                if !text.is_empty() {
                    export::copy_text(&text);
                    io::emit(&Event::Copied);
                    self.toast("Markdown copied");
                }
            }
            // On a marker or a held measurement: that one; elsewhere, the last thing added.
            Cmd::Undo => match s.cursor {
                Some((screen, x, y)) if s.marker_at(screen, x, y).is_some() => {
                    let index = s.marker_at(screen, x, y).unwrap_or_default();
                    s.remove_annotation(screen, index);
                }
                Some((screen, x, y)) if s.tool == Tool::Measure && s.held_at(screen, x, y).is_some() => {
                    let index = s.held_at(screen, x, y).unwrap_or_default();
                    s.remove_held(screen, index);
                }
                _ => drop(s.undo()),
            },
            Cmd::Clear => {
                s.clear();
                self.shots.clear();
            }
            Cmd::Hide => s.hidden = !s.hidden,
            Cmd::Toolbar => {
                self.toolbar = !self.toolbar;
                io::emit(&Event::Toolbar { shown: self.toolbar });
                self.toast(if self.toolbar { "Toolbar shown" } else { "Toolbar hidden · ⇧Tab shows it" });
            }
            Cmd::Restore => {
                let saved = self.spec.saved.clone();
                if self.session.restore(&saved) {
                    self.toast("Restored the last guides and measurements");
                }
            }
            Cmd::Help => self.help = !self.help,
            Cmd::Done | Cmd::DoneWhole | Cmd::Send if self.unbound() => {}
            Cmd::Done => self.finish(false, false, window, cx),
            Cmd::DoneWhole => self.finish(true, false, window, cx),
            // Without batches, agents get the aside once it's written anyway.
            Cmd::Send => self.finish(false, self.spec.batch, window, cx),
            Cmd::Live => self.go_live(cx),
            Cmd::Close => self.cancel(cx),
        }
    }

    /// A microphone: with the card open, what's said is about its annotation.
    fn talk_button(&mut self, down: bool) {
        if down && !self.talking && self.unbound() {
            return;
        }
        if down != self.talking {
            self.talking = down;
            let annotation = self.card.as_ref().map(|c| c.key);
            io::emit(&Event::Talk { down, annotation });
        }
    }

    /// Space, down in a card but not held long enough to talk: a space after all.
    fn type_held_space(&mut self, cx: &mut Context<Self>) {
        if let Some((_, false)) = self.space_hold.take()
            && let Some(card) = &self.card
        {
            card.input.update(cx, |input, cx| input.insert(" ", cx));
        }
    }

    /// Space held in a card long enough: talking about its annotation until it's let go.
    fn space_talks(&mut self) {
        let Some((since, false)) = self.space_hold else { return };
        let Some(key) = self.card.as_ref().map(|c| c.key) else { return };
        if since.elapsed() >= SPACE_HOLD && self.recording.is_none() && !self.unbound() {
            self.space_hold = Some((since, true));
            self.talking = true;
            io::emit(&Event::Talk { down: true, annotation: Some(key) });
        }
    }

    /// The card's dots (⌘1–⌘4): a second click on the chosen one clears it.
    fn choose_intent(&mut self, intent: Intent) {
        if let Some(card) = &mut self.card {
            card.intent = if card.intent == Some(intent) { None } else { Some(intent) };
        }
    }

    // --- Finishing ---

    fn finish(&mut self, whole: bool, send: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.close_card(window, cx);
        self.finish_now(whole, send, cx);
    }

    fn finish_now(&mut self, whole: bool, send: bool, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        if let Some(card) = self.card.take() {
            let text = card.input.read(cx).text();
            self.session.set_feedback(card.screen, card.index, &text, card.intent, card.severity);
        }
        self.done = true;
        if self.talking {
            io::emit(&Event::Talk { down: false, annotation: None });
        }
        self.keep_last();
        // A frozen screen with nothing on it, and nothing before: the screen, like a screenshot.
        let kept = match self.live {
            true => Ok(()),
            false => {
                let alone = self.kept.is_empty();
                let keeping = self.this_freeze();
                self.keep(keeping, alone, whole)
            }
        };
        let outcome = finish::outcome(std::mem::take(&mut self.kept), self.saved.clone(), send);
        match kept.and_then(|()| finish::write(&self.dir, &outcome)) {
            Ok(()) => io::emit(&Event::Finished),
            Err(err) => {
                eprintln!("could not write the result: {err:#}");
                io::emit(&Event::Cancelled { saved: self.saved_now() });
            }
        }
        self.quit(cx);
    }

    /// Closes the overlay; with the frozen screens up (still giving way too), once the system is
    /// back with the app the user was in (`windows::sync`), so it shows as it was, not as an app in
    /// the background.
    fn quit(&mut self, cx: &mut Context<Self>) {
        match self.shown == Some(false) {
            true => self.closing = true,
            false => cx.quit(),
        }
    }

    /// How far above the screen's bottom edge its visible part starts (the Dock, when it shows).
    pub fn lift(&self, screen: usize) -> f64 {
        match (self.visible.get(screen), self.spec.screens.get(screen)) {
            (Some(v), Some(s)) => (v.y - s.frame.y).max(0.0),
            _ => 0.0,
        }
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        if self.talking {
            io::emit(&Event::Talk { down: false, annotation: None });
        }
        io::emit(&Event::Cancelled { saved: self.saved_now() });
        self.quit(cx);
    }

    /// Esc: closes without adding anything, at once with nothing to lose, otherwise on a second
    /// press within `CLOSE_AGAIN`.
    fn close_by_key(&mut self, cx: &mut Context<Self>) {
        let armed = self.close_armed.is_some_and(|at| at.elapsed() < CLOSE_AGAIN);
        let work = !self.session.is_empty()
            || !self.kept.is_empty()
            || !self.shots.is_empty()
            || self.talked
            || self.recording.is_some();
        if armed || !work {
            return self.cancel(cx);
        }
        self.close_armed = Some(Instant::now());
        self.toast(format!("{} again closes without adding anything", self.keys.label(Cmd::Close)));
    }

    /// The guides and held measurements to restore next time: the frozen screen's, or the last's.
    fn saved_now(&self) -> Saved {
        if self.live { self.saved.clone() } else { self.session.saved() }
    }

    // --- The clock: the app's commands, toasts, the recording's time, the hover lookup ---

    pub fn tick(&mut self, cx: &mut Context<Self>) {
        crate::native::debug_focus();
        let mut changed = false;
        while let Ok(command) = self.commands.try_recv() {
            changed = true;
            self.apply_command(command, cx);
        }
        if self.toast.as_ref().is_some_and(|(_, until)| Instant::now() > *until) {
            self.toast = None;
            changed = true;
        }
        let seconds = self.recording.map_or(0, |(s, at)| (s + at.elapsed().as_secs_f32()) as u32);
        if seconds != self.shown_seconds {
            self.shown_seconds = seconds;
            changed = true;
        }
        self.hover_lookup();
        if self.space_hold.is_some_and(|(_, talking)| !talking) {
            self.space_talks();
            changed = true;
        }
        if self.awaiting.is_some_and(|(_, at)| at.elapsed() >= AWAIT_TRANSCRIPT) {
            self.awaiting = None;
            changed = true;
        }
        if self.shown == Some(true) {
            self.keep_last();
        }
        if changed {
            cx.notify();
        }
    }

    pub fn apply_command(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Element { id, element } => self.identified(id, element.map(|e| *e)),
            Command::Recording { seconds, annotation } => {
                // What was said waits for Done, with the annotations: say so as it stops. What was
                // said about an annotation shows in its card.
                if self.recording.is_some() && self.recording_for.is_none() && seconds.is_none() && !self.done {
                    self.toast(format!("Kept for the aside · {} adds it", self.keys.label(Cmd::Done)));
                }
                if let (Some(key), None) = (self.recording_for, seconds) {
                    self.awaiting = Some((key, Instant::now()));
                }
                self.recording = seconds.map(|s| (s, Instant::now()));
                self.recording_for = seconds.and(annotation);
                self.talked |= seconds.is_some();
            }
            Command::Transcribing { annotation, take } => {
                self.awaiting = self.awaiting.filter(|&(key, _)| key != annotation);
                self.transcribing.push((annotation, take));
            }
            Command::Said { annotation, take, text } => {
                // Unless it was thrown away meanwhile.
                if let Some(i) = self.transcribing.iter().position(|&t| t == (annotation, take)) {
                    self.transcribing.remove(i);
                    if let Some(a) = self.annotation(annotation) {
                        a.said.push(Said { take, text });
                    }
                }
            }
            Command::Finish => self.finish_now(false, false, cx),
            Command::ToggleFreeze if self.live => self.freeze(None),
            Command::ToggleFreeze => self.go_live(cx),
            Command::Frozen { screens } => self.frozen(screens),
            Command::Close => self.cancel(cx),
            Command::Notice { text } => self.notice(text),
            // Already open.
            Command::Open { .. } => {}
        }
        cx.notify();
    }

    /// Development (replay): answers every pending lookup with `element`.
    pub fn answer_all(&mut self, element: Element, cx: &mut Context<Self>) {
        let ids: Vec<u64> = self.pending.keys().copied().collect();
        for id in ids {
            self.identified(id, Some(element.clone()));
        }
        cx.notify();
    }

    /// Development (replay): answers the pending lookups one element each, oldest first.
    pub fn answer_each(&mut self, elements: Vec<Element>, cx: &mut Context<Self>) {
        let mut ids: Vec<u64> = self.pending.keys().copied().collect();
        ids.sort();
        for (id, element) in ids.into_iter().zip(elements) {
            self.identified(id, Some(element));
        }
        cx.notify();
    }

    pub fn dir(&self) -> PathBuf {
        self.dir.clone()
    }

    /// Development (replay): the open card's annotation.
    pub fn card_key(&self) -> Option<u32> {
        self.card.as_ref().map(|c| c.key)
    }

    /// Annotate, cursor at rest: ask what's under it, to outline it before the click.
    fn hover_lookup(&mut self) {
        if !self.spec.identify
            || self.session.tool != Tool::Annotate
            || self.session.drag.is_some()
            || self.card.is_some()
        {
            return;
        }
        let Some((screen, x, y, at)) = self.rested else { return };
        if at.elapsed() < HOVER_REST || self.hover_asked == Some((screen, x, y)) {
            return;
        }
        if self.hovered.as_ref().is_some_and(|h| h.screen == screen && h.frame.contains(x, y)) {
            return;
        }
        self.hover_asked = Some((screen, x, y));
        let (ox, oy) = self.spec.screens[screen].origin;
        let id = self.next_id;
        self.next_id += 1;
        self.pending.retain(|_, l| !matches!(l, Lookup::Hover { .. }));
        self.pending.insert(id, Lookup::Hover { screen });
        io::emit(&Event::Identify { id, x: ox + x, y: oy + y });
    }

    fn identified(&mut self, id: u64, element: Option<Element>) {
        let Some(lookup) = self.pending.remove(&id) else { return };
        let Some(mut element) = element else { return };
        let screen = match lookup {
            Lookup::Annotation { screen, .. } | Lookup::Hover { screen } => screen,
        };
        let (ox, oy) = self.spec.screens[screen].origin;
        element.frame = element.frame.map(|f| PtRect::new(f.x - ox, f.y - oy, f.width, f.height));
        match lookup {
            Lookup::Annotation { screen, index, number, part } => {
                let same = self.session.screens[screen].annotations.get(index).is_some_and(|a| a.number == number);
                if same {
                    self.session.set_element(screen, index, part, element);
                }
            }
            Lookup::Hover { screen } => {
                let hovered = match (element.frame, element.name) {
                    (Some(frame), Some(name)) => Some(Hovered { screen, frame, name }),
                    _ => None,
                };
                self.set_hovered(hovered);
            }
        }
    }

    /// The element outlined under the pointer; the session knows it too, so a click annotates it rather
    /// than a larger section around it.
    fn set_hovered(&mut self, hovered: Option<Hovered>) {
        self.session.element = hovered.as_ref().map(|h| (h.screen, h.frame));
        self.hovered = hovered;
    }

    fn recording_seconds(&self) -> Option<u32> {
        self.recording.map(|(s, at)| (s + at.elapsed().as_secs_f32()) as u32)
    }
}

fn arrow(key: &str) -> Option<f64> {
    match key {
        "left" | "up" => Some(-1.0),
        "right" | "down" => Some(1.0),
        _ => None,
    }
}

/// A frame as GPUI wants it: BGRA, swapped by a few threads at once.
fn render_image(frame: &Frame) -> Arc<RenderImage> {
    let mut bgra = vec![0; frame.rgba.len()];
    let part = (bgra.len() / 4).div_ceil(io::threads()).max(1) * 4;
    std::thread::scope(|scope| {
        for (to, from) in bgra.chunks_mut(part).zip(frame.rgba.chunks(part)) {
            scope.spawn(move || {
                for (to, px) in to.as_chunks_mut::<4>().0.iter_mut().zip(from.as_chunks::<4>().0) {
                    *to = [px[2], px[1], px[0], px[3]];
                }
            });
        }
    });
    let buffer = image::RgbaImage::from_raw(frame.width, frame.height, bgra)
        .unwrap_or_else(|| image::RgbaImage::new(frame.width.max(1), frame.height.max(1)));
    Arc::new(RenderImage::new(vec![image::Frame::new(buffer)]))
}

fn color(value: u32, alpha: f32) -> Hsla {
    let mut c: Rgba = gpui::rgb(value);
    c.a = alpha;
    c.into()
}

// --- Drawing ---

/// What the canvas paints, in window points.
enum Prim {
    Fill(PtRect, Hsla),
    Border(PtRect, Hsla, f32, bool),
    Stroke(Vec<(f64, f64)>, f32, Hsla),
    Polygon(Vec<(f64, f64)>, Hsla),
}

fn paint(prim: Prim, window: &mut Window) {
    let b =
        |r: PtRect| Bounds::new(point(px(r.x as f32), px(r.y as f32)), size(px(r.width as f32), px(r.height as f32)));
    let p = |(x, y): (f64, f64)| point(px(x as f32), px(y as f32));
    match prim {
        Prim::Fill(r, c) => window.paint_quad(gpui::fill(b(r), c)),
        Prim::Border(r, c, w, dashed) => window.paint_quad(quad(
            b(r),
            px(0.),
            transparent_black(),
            px(w),
            c,
            if dashed { BorderStyle::Dashed } else { BorderStyle::Solid },
        )),
        Prim::Stroke(points, w, c) => {
            if points.len() < 2 {
                return;
            }
            let mut builder = PathBuilder::stroke(px(w));
            builder.move_to(p(points[0]));
            for q in &points[1..] {
                builder.line_to(p(*q));
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, c);
            }
        }
        Prim::Polygon(points, c) => {
            let mut builder = PathBuilder::fill();
            builder.move_to(p(points[0]));
            for q in &points[1..] {
                builder.line_to(p(*q));
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, c);
            }
        }
    }
}

/// A dark pill with white text, centered on `(x, y)` (window points).
fn pill(text: impl Into<SharedString>, (x, y): (f64, f64)) -> AnyElement {
    div()
        .absolute()
        .left(px(x as f32 - 150.))
        .top(px(y as f32 - 10.))
        .w(px(300.))
        .h(px(20.))
        .flex()
        .justify_center()
        .items_center()
        .child(
            div()
                .px(px(6.))
                .py(px(1.5))
                .rounded(px(4.))
                .bg(color(style::LABEL, style::LABEL_ALPHA))
                .text_color(gpui::white())
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .child(text.into()),
        )
        .into_any_element()
}

/// About how wide a pill is, to keep it on screen.
fn pill_width(text: &str) -> f64 {
    text.chars().count() as f64 * 6.6 + 12.0
}

pub struct ScreenView {
    overlay: Entity<Overlay>,
    screen: usize,
    focus: FocusHandle,
}

impl ScreenView {
    pub fn new(overlay: Entity<Overlay>, screen: usize, cx: &mut Context<Self>) -> ScreenView {
        cx.observe(&overlay, |_, _, cx| cx.notify()).detach();
        ScreenView { overlay, screen, focus: cx.focus_handle() }
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ScreenView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = self.screen;
        let overlay = self.overlay.clone();
        let o = self.overlay.read(cx);
        let state = &o.session.screens[screen];
        let win = &o.windows[screen];
        let (ox, oy) = win.offset;
        let (ww, wh) = win.size;
        let (sw, sh) = (state.screen.width, state.screen.height);
        let screenshot = || {
            img(ImageSource::Render(o.images[screen].clone()))
                .absolute()
                .left(px(-ox as f32))
                .top(px(-oy as f32))
                .w(px(sw as f32))
                .h(px(sh as f32))
        };
        // Giving way (back to the live screen, or closing): the screen as it was, nothing on it, until
        // the user's app is back (`windows::handed_back`). It looks live at once.
        if o.handoff.as_ref().is_some_and(|h| h.swapped()) || o.closing {
            // The live toolbar, as its window draws it, where it is (on this screen).
            let f = o.spec.screens[screen].frame;
            let here = o.live_toolbar_frame.filter(|_| !o.closing).filter(|t| {
                let (cx, cy) = (t.x + t.width / 2.0, t.y + t.height / 2.0);
                cx >= f.x && cx < f.x + f.width && cy >= f.y && cy < f.y + f.height
            });
            let live_toolbar = here.map(|t| {
                div()
                    .absolute()
                    .left(px((t.x - f.x - ox) as f32))
                    .top(px((f.y + f.height - t.y - t.height - oy) as f32))
                    .w(px(t.width as f32))
                    .h(px(t.height as f32))
                    .overflow_hidden()
                    .flex()
                    .items_start()
                    .p(px(TOOLBAR_MARGIN as f32))
                    .child(toolbar(&overlay, o, true))
            });
            return div()
                .id("screen")
                .size_full()
                .relative()
                .overflow_hidden()
                .bg(gpui::black())
                .child(screenshot())
                .children(live_toolbar)
                .into_any_element();
        }
        let (scale, units) = (state.screen.scale, o.session.units);
        let n = |v: f64| num(v, scale, units);
        let accent = o.accent;
        let thin = (1.0 / window.scale_factor() as f64).max(0.5);
        let to = |(x, y): (f64, f64)| (x - ox, y - oy);
        // Where the menu bar ends (and the notch): nothing of the overlay's goes above it.
        let menu_bar = o.spec.screens[screen].menu_bar;
        let top = (menu_bar - oy).max(0.0);
        // A label's place, below the menu bar.
        let label = |at: (f64, f64)| {
            let (x, y) = to(at);
            (x, y.max(top + 12.0))
        };
        let rect = |r: PtRect| PtRect::new(r.x - ox, r.y - oy, r.width, r.height);
        let cursor = o.session.cursor.filter(|c| c.0 == screen).map(|c| (c.1, c.2));
        let session = &o.session;

        let mut prims: Vec<Prim> = Vec::new();
        let mut labels: Vec<AnyElement> = Vec::new();
        let mut markers: Vec<AnyElement> = Vec::new();
        let hline = |prims: &mut Vec<Prim>, x0: f64, x1: f64, y: f64, c: Hsla, w: f64| {
            prims.push(Prim::Fill(PtRect::new(x0 - ox, y - oy, x1 - x0, w), c))
        };
        let vline = |prims: &mut Vec<Prim>, y0: f64, y1: f64, x: f64, c: Hsla, w: f64| {
            prims.push(Prim::Fill(PtRect::new(x - ox, y0 - oy, w, y1 - y0), c))
        };
        let accent_c = color(accent, 1.0);

        if !session.hidden {
            // Guides, with the gaps between them at the screen's top or left edge.
            for axis in [Axis::X, Axis::Y] {
                let mut positions: Vec<f64> =
                    state.guides.iter().filter(|g| g.axis == axis).map(|g| g.position).collect();
                if positions.is_empty() {
                    continue;
                }
                positions.sort_by(f64::total_cmp);
                for &p in &positions {
                    match axis {
                        Axis::X => vline(&mut prims, 0.0, sh, p, color(style::GUIDE, 1.0), thin),
                        Axis::Y => hline(&mut prims, 0.0, sw, p, color(style::GUIDE, 1.0), thin),
                    }
                }
                let limit = if axis == Axis::X { sw } else { sh };
                let mut stops = vec![0.0];
                stops.extend(positions);
                stops.push(limit);
                for pair in stops.windows(2) {
                    let gap = pair[1] - pair[0];
                    if gap >= 16.0 {
                        let mid = (pair[0] + pair[1]) / 2.0;
                        let at = if axis == Axis::X { (mid, 14.0) } else { (28.0, mid) };
                        labels.push(pill(n(gap), label(at)));
                    }
                }
            }

            // What's held.
            for (i, m) in state.held.iter().enumerate() {
                match m {
                    Measurement::Distance { x, y, across, down } => {
                        if !across.is_empty() {
                            hline(&mut prims, across.start, across.end, *y, accent_c, thin);
                            for (edge, open) in [(across.start, across.open_start), (across.end, across.open_end)] {
                                if !open {
                                    vline(&mut prims, y - 4.0, y + 4.0, edge, accent_c, thin);
                                }
                            }
                            let ly = if *y < 30.0 { y + 14.0 } else { y - 14.0 };
                            labels.push(pill(n(across.len()), label(((across.start + across.end) / 2.0, ly))));
                        }
                        if !down.is_empty() {
                            vline(&mut prims, down.start, down.end, *x, accent_c, thin);
                            for (edge, open) in [(down.start, down.open_start), (down.end, down.open_end)] {
                                if !open {
                                    hline(&mut prims, x - 4.0, x + 4.0, edge, accent_c, thin);
                                }
                            }
                            let text = n(down.len());
                            let w = pill_width(&text);
                            let lx = if x + w + 16.0 > sw { x - w / 2.0 - 8.0 } else { x + w / 2.0 + 8.0 };
                            labels.push(pill(text, label((lx, (down.start + down.end) / 2.0))));
                        }
                    }
                    Measurement::Area { rect: r, .. } => {
                        prims.push(Prim::Fill(rect(*r), color(accent, style::AREA_TINT)));
                        prims.push(Prim::Border(rect(*r), accent_c, thin as f32, false));
                        let text = format!("{} × {}", n(r.width), n(r.height));
                        let at = style::area_label(*r, pill_width(&text), sh);
                        labels.push(pill(text, label(at)));
                        // Over a held object: its camera.
                        let over = cursor.is_some_and(|(x, y)| r.contains(x, y))
                            && session.drag.is_none()
                            && session.tool == Tool::Measure;
                        if over {
                            let (cx_, cy_) = to(at);
                            let o2 = overlay.clone();
                            labels.push(
                                div()
                                    .id(SharedString::from(format!("camera-{screen}-{i}")))
                                    .absolute()
                                    .left(px((cx_
                                        + pill_width(&format!("{} × {}", n(r.width), n(r.height))) / 2.0
                                        + 4.0) as f32))
                                    .top(px(cy_ as f32 - 11.))
                                    .size(px(22.))
                                    .rounded(px(11.))
                                    .bg(color(style::LABEL, style::LABEL_ALPHA))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .occlude()
                                    .cursor_pointer()
                                    .hover(|s| s.bg(color(style::UI_DEEP, 1.0)))
                                    .child(svg().path("icons/camera.svg").size(px(13.)).text_color(gpui::white()))
                                    .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                                        cx.stop_propagation();
                                        o2.update(cx, |o, cx| {
                                            o.shot_of_held(screen, i);
                                            cx.notify();
                                        });
                                    })
                                    .into_any_element(),
                            );
                        }
                    }
                }
            }

            // Annotations.
            for a in &state.annotations {
                let (r, g, b, alpha) = style::annotation_color(a, accent);
                let ink = Hsla::from(Rgba { r: r as f32, g: g as f32, b: b as f32, a: 1.0 });
                match (&a.shape, a.area) {
                    (Some(Shape::Sketch { points }), _) => {
                        prims.push(Prim::Stroke(points.iter().map(|p| to(*p)).collect(), style::DRAW_WIDTH, ink));
                    }
                    (Some(Shape::Arrow { from, to: end }), _) => arrow_prims(&mut prims, to(*from), to(*end), ink),
                    (None, Some(area)) => {
                        prims.push(Prim::Fill(rect(area), Hsla { a: alpha, ..ink }));
                        prims.push(Prim::Border(rect(area), ink, 1.5, true));
                    }
                    (None, None) => {
                        if let Some(frame) = a.element.as_ref().and_then(|e| e.frame) {
                            prims.push(Prim::Border(rect(frame), Hsla { a: 0.9, ..ink }, 1.5, false));
                        }
                    }
                }
                let (mx, my) = to(a.at);
                let hovered_marker = cursor.is_some_and(|(x, y)| (a.at.0 - x).hypot(a.at.1 - y) <= MARKER / 2.0 + 1.0);
                markers.push(
                    div()
                        .absolute()
                        .left(px((mx - MARKER / 2.0) as f32))
                        .top(px((my - MARKER / 2.0) as f32))
                        .size(px(MARKER as f32))
                        .rounded_full()
                        .bg(ink)
                        .border_2()
                        .border_color(gpui::white())
                        .shadow_md()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(gpui::white())
                        .text_size(px(11.))
                        .font_weight(FontWeight::BOLD)
                        .child(a.number.to_string())
                        .into_any_element(),
                );
                if hovered_marker && o.card.is_none() {
                    let mut text = a.title(scale, units);
                    if !a.comment.is_empty() {
                        text = format!("{text}: {}", a.comment);
                    }
                    let text: String = text.chars().take(90).collect();
                    markers.push(
                        div()
                            .absolute()
                            .left(px((mx + MARKER / 2.0 + 6.0) as f32))
                            .top(px((my - 11.0) as f32))
                            .max_w(px(360.))
                            .px(px(8.))
                            .py(px(3.))
                            .rounded(px(6.))
                            .bg(color(style::PANEL, style::PANEL_ALPHA))
                            .text_color(gpui::white())
                            .text_size(px(12.))
                            .child(text)
                            .into_any_element(),
                    );
                }
            }
        }

        // Live: what the cursor measures, the drag, the drawing, the hover.
        if !session.hidden && o.menu.is_none() {
            if let (Some((x, y)), Some(m)) = (cursor, session.hover()) {
                if session.crosshair {
                    hline(&mut prims, 0.0, sw, y, color(accent, 0.6), thin);
                    vline(&mut prims, 0.0, sh, x, color(accent, 0.6), thin);
                }
                if let Measurement::Distance { x: px_, y: py_, across, down } = &m {
                    if !across.is_empty() {
                        hline(&mut prims, across.start, across.end, *py_, accent_c, thin);
                        for (edge, open) in [(across.start, across.open_start), (across.end, across.open_end)] {
                            if !open {
                                vline(&mut prims, py_ - 4.0, py_ + 4.0, edge, accent_c, thin);
                            }
                        }
                    }
                    if !down.is_empty() {
                        vline(&mut prims, down.start, down.end, *px_, accent_c, thin);
                        for (edge, open) in [(down.start, down.open_start), (down.end, down.open_end)] {
                            if !open {
                                hline(&mut prims, px_ - 4.0, px_ + 4.0, edge, accent_c, thin);
                            }
                        }
                    }
                    // One pill, at the cursor's lower right (PixelSnap's), kept on screen.
                    let text = m.dimensions(scale, units);
                    let w = pill_width(&text);
                    let lx = if x + 14.0 + w > sw { x - 14.0 - w / 2.0 } else { x + 14.0 + w / 2.0 };
                    let ly = if y + 34.0 > sh { y - 18.0 } else { y + 22.0 };
                    labels.push(pill(text, label((lx, ly))));
                }
            }
            if let Some(drag) = session.drag.as_ref().filter(|d| d.screen == screen) {
                match session.tool {
                    Tool::Draw => {
                        if session.drawing_arrow() {
                            arrow_prims(&mut prims, to(drag.start), to(drag.current), accent_c);
                        } else {
                            prims.push(Prim::Stroke(
                                drag.points.iter().map(|p| to(*p)).collect(),
                                style::DRAW_WIDTH,
                                accent_c,
                            ));
                        }
                    }
                    Tool::Measure if session.alternate => {
                        prims.push(Prim::Border(rect(drag.rect()), color(accent, 0.7), 1.0, true));
                        for r in session.drag_objects() {
                            prims.push(Prim::Fill(rect(r), color(accent, style::AREA_TINT)));
                            prims.push(Prim::Border(rect(r), accent_c, thin as f32, false));
                        }
                    }
                    tool => {
                        if let Some((_, r, snapped)) = session.drag_area() {
                            let (fill_c, line_c) = match tool {
                                Tool::Annotate => {
                                    (color(style::ANNOTATION_GREEN, 0.1), color(style::ANNOTATION_GREEN, 1.0))
                                }
                                Tool::Shot => (color(0xffffff, 0.12), color(0xffffff, 0.9)),
                                _ => (color(accent, style::AREA_TINT), accent_c),
                            };
                            if snapped {
                                prims.push(Prim::Border(rect(drag.rect()), color(accent, 0.5), 1.0, true));
                            }
                            prims.push(Prim::Fill(rect(r), fill_c));
                            prims.push(Prim::Border(
                                rect(r),
                                line_c,
                                if tool == Tool::Measure { thin as f32 } else { 1.5 },
                                tool != Tool::Measure,
                            ));
                            let text = format!("{} × {}", n(r.width), n(r.height));
                            let at = style::area_label(r, pill_width(&text), sh);
                            labels.push(pill(text, label(at)));
                        }
                    }
                }
            }
            // Annotate: what a click annotates, a measured section or the element under the pointer.
            let section = cursor
                .filter(|_| session.tool == Tool::Annotate && session.drag.is_none() && o.card.is_none())
                .filter(|&(x, y)| session.marker_at(screen, x, y).is_none())
                .and_then(|(x, y)| session.section_at(screen, x, y));
            if let Some(r) = section {
                prims.push(Prim::Fill(rect(r), color(style::ANNOTATION_BLUE, 0.12)));
                prims.push(Prim::Border(rect(r), color(style::ANNOTATION_BLUE, 1.0), 1.5, false));
            } else if let Some(h) =
                o.hovered.as_ref().filter(|h| h.screen == screen && session.drag.is_none() && o.card.is_none())
            {
                prims.push(Prim::Fill(rect(h.frame), color(style::ANNOTATION_BLUE, 0.12)));
                prims.push(Prim::Border(rect(h.frame), color(style::ANNOTATION_BLUE, 1.0), 1.5, false));
                let name: String = h.name.chars().take(60).collect();
                let w = pill_width(&name);
                let x = (h.frame.x + h.frame.width / 2.0).clamp(w / 2.0 + 4.0, sw - w / 2.0 - 4.0);
                let y = if h.frame.y > menu_bar + 24.0 { h.frame.y - 12.0 } else { h.frame.bottom() + 12.0 };
                labels.push(pill(name, label((x, y))));
            }
        }

        let cursor_style = match session.tool {
            Tool::Annotate => CursorStyle::Arrow,
            _ => CursorStyle::Crosshair,
        };
        let shows_toolbar = o.session.cursor.map_or(screen == 0, |c| c.0 == screen);
        let card = o.card.as_ref().filter(|c| c.screen == screen).map(|c| self.card(o, c, (ox, oy), (ww, wh)));
        let menu = o.menu.as_ref().filter(|m| m.screen == screen).map(|m| self.menu(m, (ox, oy), (ww, wh), top));
        let shows_help = (o.help || o.help_hover) && shows_toolbar;
        let bottom = (shows_toolbar && !session.hidden).then(|| self.bottom(o, shows_help, (ox, oy), (ww, wh)));
        let hidden_hint = (session.hidden && shows_toolbar).then(|| {
            pill(format!("Hidden · {} shows everything again", o.keys.label(Cmd::Hide)), (ww / 2.0, top + 20.0))
        });
        // Nowhere to add the aside: said when it opens, not after the work's done.
        let banner = (o.spec.bound.is_none() && shows_toolbar && !session.hidden).then(|| {
            div().absolute().top(px(top as f32 + 14.)).left_0().right_0().flex().justify_center().child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .px(px(11.))
                    .py(px(5.))
                    .rounded(px(9.))
                    .bg(color(style::PANEL, style::PANEL_ALPHA))
                    .text_color(gpui::white())
                    .text_size(px(12.))
                    .child(div().size(px(7.)).rounded_full().bg(color(style::INTENT_CHANGE, 1.0)))
                    .child("Not bound to a repo · run `asides` in your project to keep what you annotate"),
            )
        });
        let image = screenshot();

        let view = cx.entity();
        div()
            .id("screen")
            .size_full()
            .relative()
            .overflow_hidden()
            .bg(gpui::black())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| {
                this.overlay.update(cx, |o, cx| o.key_down(e, window, cx));
            }))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, _, cx| {
                this.overlay.update(cx, |o, cx| o.key_up(e, cx));
            }))
            .on_modifiers_changed(cx.listener(|this, e: &ModifiersChangedEvent, _, cx| {
                this.overlay.update(cx, |o, cx| {
                    o.set_modifiers(&e.modifiers);
                    cx.notify();
                });
            }))
            .child(image)
            .child(
                canvas(
                    move |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |_, hitbox: Hitbox, window, _| {
                        for prim in prims {
                            paint(prim, window);
                        }
                        window.set_cursor_style(cursor_style, &hitbox);
                        listen(window, view, overlay, screen, (ox, oy), hitbox);
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(labels)
            .children(markers)
            .children(hidden_hint)
            .children(banner)
            .children(bottom)
            // On top of everything else.
            .children(card.map(|c| deferred(c).with_priority(1)))
            .children(menu.map(|m| deferred(m).with_priority(2)))
            .into_any_element()
    }
}

/// The mouse, for the whole window: presses only where nothing (a toolbar, a card) is on top,
/// moves and releases everywhere (a drag can cross the toolbar).
fn listen(
    window: &mut Window,
    view: Entity<ScreenView>,
    overlay: Entity<Overlay>,
    screen: usize,
    (ox, oy): (f64, f64),
    hitbox: Hitbox,
) {
    let at = move |p: Point<Pixels>| (f64::from(p.x) + ox, f64::from(p.y) + oy);
    // Another app took over meanwhile (one macOS doesn't hold back, like Notification Center): a
    // press anywhere on the frozen screen takes the system back.
    window.on_mouse_event(|_: &MouseDownEvent, phase, _, _| {
        if phase == DispatchPhase::Capture && !crate::native::is_active() {
            crate::native::activate();
        }
    });
    let (o, h, v) = (overlay.clone(), hitbox.clone(), view.clone());
    window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !h.is_hovered(window) {
            return;
        }
        // The keyboard follows the screen being worked on.
        let focus = v.read(cx).focus_handle();
        if o.read(cx).card.is_none() {
            window.focus(&focus);
        }
        o.update(cx, |o, cx| o.mouse_down(screen, at(e.position), e, window, cx));
    });
    let o = overlay.clone();
    window.on_mouse_event(move |e: &MouseMoveEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble {
            o.update(cx, |o, cx| o.mouse_move(screen, at(e.position), e, cx));
        }
    });
    window.on_mouse_event(move |e: &MouseUpEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            overlay.update(cx, |o, cx| o.mouse_up(screen, at(e.position), e, window, cx));
        }
    });
}

fn arrow_prims(prims: &mut Vec<Prim>, from: (f64, f64), to: (f64, f64), c: Hsla) {
    if (to.0 - from.0).hypot(to.1 - from.1) < 2.0 {
        return;
    }
    let [tip, left, right] = style::arrow_head(from, to);
    let base = ((left.0 + right.0) / 2.0, (left.1 + right.1) / 2.0);
    prims.push(Prim::Stroke(vec![from, base], style::DRAW_WIDTH, c));
    prims.push(Prim::Polygon(vec![tip, left, right], c));
}

// --- The toolbar, the card, the menu, the cheat sheet ---

/// What a toolbar button does.
type OnPress = dyn Fn(&mut Overlay, &mut Window, &mut Context<Overlay>);

/// How a toolbar button stands out: not at all, selected (a tool, the keys), or the main action.
#[derive(Clone, Copy, PartialEq)]
enum Emphasis {
    Plain,
    On,
    Primary,
}

impl Emphasis {
    /// Its text and icons.
    fn ink(self) -> Hsla {
        match self {
            Emphasis::Plain => gpui::white(),
            Emphasis::On | Emphasis::Primary => color(style::UI_INK, 1.0),
        }
    }
}

fn icon(name: &str, ink: Hsla) -> impl IntoElement {
    svg().path(format!("icons/{name}.svg")).size(px(15.)).text_color(ink)
}

/// The toolbar: a handle to move it, the tools, the microphone, Done and close, and fold. Frozen,
/// also the tolerance, the units, undo, copy, the keys and Live; over the live screen (`live`), the
/// tools freeze the screen. Folded: the handle, the microphone and unfold.
pub fn toolbar(overlay: &Entity<Overlay>, o: &Overlay, live: bool) -> AnyElement {
    let session = &o.session;
    // An icon (or short text) button; its name shows above the toolbar while the pointer is on it.
    let button = |id: &str, name: String, emphasis: Emphasis, content: AnyElement, on: Box<OnPress>| {
        let (o1, o2) = (overlay.clone(), overlay.clone());
        let name: SharedString = name.into();
        div()
            .id(SharedString::from(id.to_string()))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .h(px(28.))
            .min_w(px(30.))
            .px(px(7.))
            .rounded(px(7.))
            .text_size(px(12.))
            .text_color(emphasis.ink())
            .when(emphasis == Emphasis::On, |d| d.bg(color(style::UI, 1.0)))
            .when(emphasis == Emphasis::Primary, |d| {
                d.bg(color(style::UI, 1.0)).hover(|s| s.bg(color(style::UI_LIGHT, 1.0)))
            })
            .when(emphasis == Emphasis::Plain, |d| d.hover(|s| s.bg(color(0xffffff, 0.12))))
            .cursor_pointer()
            .on_hover(move |hovered, _, cx| {
                let name = name.clone();
                o1.update(cx, |o, cx| {
                    o.hint = hovered.then_some(name);
                    cx.notify();
                });
            })
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                o2.update(cx, |o, cx| {
                    on(o, window, cx);
                    cx.notify();
                });
            })
            .child(content)
            .into_any_element()
    };
    let sep = || div().w(px(1.)).h(px(18.)).mx(px(3.)).bg(color(0xffffff, 0.15)).into_any_element();
    let plain = Emphasis::Plain.ink();
    let grab = overlay.clone();
    let mut items: Vec<AnyElement> = vec![
        div()
            .id("handle")
            .flex()
            .items_center()
            .justify_center()
            .h(px(28.))
            .w(px(16.))
            .cursor(CursorStyle::OpenHand)
            .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                cx.stop_propagation();
                grab.update(cx, |o, _| o.grab_toolbar(e.position, live));
            })
            .child(svg().path("icons/grip.svg").size(px(14.)).text_color(color(0xffffff, 0.45)))
            .into_any_element(),
    ];
    let fold = |folded: bool| {
        button(
            "fold",
            if folded { "Unfold the toolbar" } else { "Fold the toolbar" }.into(),
            Emphasis::Plain,
            icon(if folded { "unfold" } else { "fold" }, plain).into_any_element(),
            Box::new(|o, _, _| {
                o.collapsed = !o.collapsed;
                o.placed_toolbar();
            }),
        )
    };
    if o.collapsed {
        items.push(talk_button(overlay, o, "talk", true));
        items.push(fold(true));
        return toolbar_panel(overlay, items, live);
    }
    for (tool, name) in
        [(Tool::Measure, "measure"), (Tool::Annotate, "annotate"), (Tool::Draw, "draw"), (Tool::Shot, "shot")]
    {
        let emphasis = if !live && session.tool == tool { Emphasis::On } else { Emphasis::Plain };
        let label = if live { format!("{}: freezes the screen", tool.label()) } else { tool.label().to_string() };
        items.push(button(
            name,
            label,
            emphasis,
            icon(name, emphasis.ink()).into_any_element(),
            Box::new(move |o, _, _| o.freeze(Some(tool))),
        ));
    }
    if live {
        items.push(button(
            "pause",
            format!("Freeze the screen ({})", o.spec.freeze_shortcut),
            Emphasis::Plain,
            icon("pause", plain).into_any_element(),
            Box::new(|o, _, _| o.freeze(None)),
        ));
    }
    items.push(sep());
    if !live {
        let level = match session.tolerance.level() {
            "Medium" => "Med",
            other => other,
        };
        items.push(button(
            "tolerance",
            format!("Edge tolerance: {}", session.tolerance.level()),
            Emphasis::Plain,
            div().opacity(0.8).child(level).into_any_element(),
            Box::new(|o, _, _| {
                let next = match o.session.tolerance.level() {
                    "Low" => "Medium",
                    "Medium" => "High",
                    _ => "Low",
                };
                o.session.tolerance = Tolerance::of_level(next).unwrap_or_default();
                o.session.normal_tolerance = o.session.tolerance;
            }),
        ));
        items.push(button(
            "units",
            "Points or pixels".into(),
            Emphasis::Plain,
            div().opacity(0.8).child(session.units.label()).into_any_element(),
            Box::new(|o, _, _| o.session.toggle_units()),
        ));
        items.push(sep());
    }
    items.push(talk_button(overlay, o, "talk", true));
    items.push(sep());
    if !live {
        items.push(button(
            "undo",
            "Undo".into(),
            Emphasis::Plain,
            icon("undo", plain).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Undo, w, cx)),
        ));
        items.push(button(
            "copy",
            "Copy as Markdown".into(),
            Emphasis::Plain,
            icon("copy", plain).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::CopyMarkdown, w, cx)),
        ));
        // `?`: the keys while the pointer is on it; a click keeps them up.
        let (h1, h2) = (overlay.clone(), overlay.clone());
        let help_on = o.help || o.help_hover;
        let ink = if help_on { Emphasis::On.ink() } else { plain };
        items.push(
            div()
                .id("help")
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .cursor_pointer()
                .when(help_on, |d| d.bg(color(style::UI, 1.0)))
                .when(!help_on, |d| d.hover(|s| s.bg(color(0xffffff, 0.12))))
                .on_hover(move |hovered, _, cx| {
                    h1.update(cx, |o, cx| {
                        o.help_hover = *hovered;
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    h2.update(cx, |o, cx| {
                        o.help = !o.help;
                        cx.notify();
                    });
                })
                .child(icon("help", ink))
                .into_any_element(),
        );
        items.push(button(
            "live",
            format!("Back to the live screen ({})", o.spec.freeze_shortcut),
            Emphasis::Plain,
            icon("live", plain).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Live, w, cx)),
        ));
        items.push(sep());
    }
    let done_name = if o.spec.bound.is_some() { "Add it all to the aside" } else { "Not bound to a repo" };
    items.push(button(
        "done",
        done_name.into(),
        Emphasis::Primary,
        icon("check", Emphasis::Primary.ink()).into_any_element(),
        Box::new(|o, w, cx| o.command(Cmd::Done, w, cx)),
    ));
    if o.spec.batch {
        items.push(button(
            "send",
            "Add and send to your agent".into(),
            Emphasis::Plain,
            icon("send", plain).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Send, w, cx)),
        ));
    }
    items.push(button(
        "close",
        "Close without adding anything".into(),
        Emphasis::Plain,
        icon("close", plain).into_any_element(),
        Box::new(|o, w, cx| o.command(Cmd::Close, w, cx)),
    ));
    items.push(fold(false));
    toolbar_panel(overlay, items, live)
}

/// The toolbar's panel around its buttons; it notes where it's drawn, to be moved (and over the
/// live screen, for its window to fit it).
fn toolbar_panel(overlay: &Entity<Overlay>, items: Vec<AnyElement>, live: bool) -> AnyElement {
    let measure = overlay.clone();
    div()
        .id("toolbar")
        .occlude()
        .relative()
        .flex()
        .items_center()
        .gap(px(2.))
        .p(px(5.))
        .rounded(px(12.))
        .bg(color(style::PANEL, style::PANEL_ALPHA))
        .border_1()
        .border_color(color(0xffffff, 0.12))
        .shadow_lg()
        .children(items)
        .child(
            canvas(
                move |bounds, _, cx| {
                    measure.update(cx, |o, _| match live {
                        true => o.live_toolbar_bounds = Some(bounds),
                        false => o.toolbar_bounds = Some(bounds),
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
        .into_any_element()
}

/// The microphone: tap to start or stop recording, hold to talk. Red while recording, with the
/// time on the toolbar (`timer`); dimmed when there's no repo to record into.
fn talk_button(overlay: &Entity<Overlay>, o: &Overlay, id: &str, timer: bool) -> AnyElement {
    let recording = o.recording_seconds();
    let bound = o.spec.bound.is_some();
    let (t1, t2, t3) = (overlay.clone(), overlay.clone(), overlay.clone());
    let name: SharedString =
        if bound { "Talk: hold, or tap to start and stop".into() } else { "Not bound to a repo".into() };
    div()
        .id(SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .h(px(28.))
        .min_w(px(30.))
        .px(px(7.))
        .rounded(px(7.))
        .text_size(px(12.))
        .text_color(gpui::white())
        .cursor_pointer()
        .when(recording.is_some(), |d| d.bg(color(style::RECORDING, 1.0)))
        .when(recording.is_none(), |d| d.hover(|s| s.bg(color(0xffffff, 0.12))))
        .when(!bound, |d| d.opacity(0.4))
        .on_hover(move |hovered, _, cx| {
            let name = name.clone();
            t3.update(cx, |o, cx| {
                o.hint = hovered.then_some(name);
                cx.notify();
            });
        })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            t1.update(cx, |o, cx| {
                o.talk_button(true);
                cx.notify();
            });
        })
        .on_mouse_up(MouseButton::Left, move |_, _, cx| {
            t2.update(cx, |o, _| o.talk_button(false));
        })
        .child(icon("mic", gpui::white()))
        .children(recording.filter(|_| timer).map(|s| format!("{}:{:02}", s / 60, s % 60)))
        .into_any_element()
}

/// The toolbar's window over the live screen.
pub struct ToolbarView {
    overlay: Entity<Overlay>,
}

impl ToolbarView {
    pub fn new(overlay: Entity<Overlay>, cx: &mut Context<Self>) -> ToolbarView {
        cx.observe(&overlay, |_, _, cx| cx.notify()).detach();
        ToolbarView { overlay }
    }
}

/// Room around the live toolbar in its window, for its shadow.
pub const TOOLBAR_MARGIN: f64 = 8.0;

impl Render for ToolbarView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let o = self.overlay.read(cx);
        // As big as the toolbar: its window fits it.
        div().size_full().flex().items_start().p(px(TOOLBAR_MARGIN as f32)).child(toolbar(&self.overlay, o, true))
    }
}

impl ScreenView {
    /// What goes with the toolbar: a toast, the name of the button under the pointer, the keys
    /// (while on `?`), and the toolbar, at the bottom or where the user put it; hidden, just the
    /// recording's time.
    fn bottom(&self, o: &Overlay, shows_help: bool, (ox, oy): (f64, f64), (ww, wh): (f64, f64)) -> AnyElement {
        let recording = o.recording_seconds();
        let toast = o.toast.as_ref().map(|(t, _)| t.clone());
        let tip = (o.toolbar && !shows_help).then(|| o.hint.clone()).flatten();
        let bar = o.toolbar.then(|| toolbar(&self.overlay, o, false));
        // Hidden, the toolbar still says it's recording.
        let recording_only = (!o.toolbar).then_some(recording).flatten().map(|s| {
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .px(px(10.))
                .py(px(4.))
                .rounded(px(10.))
                .bg(color(style::RECORDING, 1.0))
                .text_color(gpui::white())
                .text_size(px(12.))
                .child(div().size(px(7.)).rounded_full().bg(gpui::white()))
                .child(format!("{}:{:02}", s / 60, s % 60))
        });
        let small = |text: SharedString| {
            div()
                .px(px(9.))
                .py(px(4.))
                .rounded(px(7.))
                .bg(color(style::PANEL, style::PANEL_ALPHA))
                .text_color(gpui::white())
                .text_size(px(12.))
                .child(text)
        };
        let notes = |d: Div| {
            d.flex()
                .flex_col()
                .items_center()
                .gap(px(8.))
                .children(shows_help.then(|| self.help(o)))
                .children(toast.clone().map(small))
                .children(tip.clone().map(small))
        };
        let spec = &o.spec.screens[self.screen];
        let (screen, top) = (&spec.screen, (spec.menu_bar - oy).max(0.0));
        match o.toolbar_at.filter(|_| o.toolbar) {
            // Where the user put it, kept on the screen below the menu bar, with what goes with it
            // toward the middle of the screen: above it, or below it in the top half.
            Some((fx, fy)) => {
                let (w, h) = o
                    .toolbar_bounds
                    .map_or((560.0, 40.0), |b| (f32::from(b.size.width) as f64, f32::from(b.size.height) as f64));
                let x = (fx * screen.width - ox).clamp(8.0, (ww - w - 8.0).max(8.0));
                let y = (fy * screen.height - oy).clamp(top + 8.0, (wh - h - 8.0).max(top + 8.0));
                let notes = notes(div()).absolute().left(px((x + w / 2.0 - 320.0) as f32)).w(px(640.));
                let notes = if y + h / 2.0 < wh / 2.0 {
                    notes.top(px((y + h + 8.0) as f32)).justify_start()
                } else {
                    notes.bottom(px((wh - y + 8.0) as f32)).justify_end()
                };
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .child(notes)
                    .child(div().absolute().left(px(x as f32)).top(px(y as f32)).children(bar))
                    .into_any_element()
            }
            None => notes(div())
                .absolute()
                .bottom(px((o.lift(self.screen) + TOOLBAR_LIFT) as f32))
                .left_0()
                .right_0()
                .children(bar)
                .children(recording_only)
                .into_any_element(),
        }
    }

    /// An annotation's card: what should change (typed, wrapping as it grows), what kind of request
    /// it is (the dots), the microphone, and, once something was said about it, what was said and a
    /// trash button to say it again.
    fn card(&self, o: &Overlay, card: &Card, (ox, oy): (f64, f64), (ww, wh): (f64, f64)) -> AnyElement {
        let state = &o.session.screens[card.screen];
        let Some(a) = state.annotations.get(card.index) else { return div().into_any_element() };
        let recording_here = o.recording.is_some() && o.recording_for == Some(card.key);
        let transcribing = o.transcribing.iter().any(|&(k, _)| k == card.key)
            || o.awaiting.is_some_and(|(k, at)| k == card.key && at.elapsed() < AWAIT_TRANSCRIPT);
        // Wider by its trash button from the moment something's said, until it's deleted: the text
        // field keeps its width, so what's typed doesn't wrap again. Placed for the wider card, it
        // doesn't move sideways when it widens.
        let has_said = !a.said.is_empty() || transcribing || recording_here;
        let width = CARD_WIDTH + if has_said { TRASH_WIDTH } else { 0.0 };
        let x = (a.at.0 - ox + MARKER / 2.0 + 6.0).clamp(8.0, (ww - CARD_WIDTH - TRASH_WIDTH - 8.0).max(8.0));
        let top = (o.spec.screens[card.screen].menu_bar - oy).max(0.0);
        let below = (a.at.1 - oy + MARKER / 2.0 + 6.0).max(top + 8.0);
        // Near the bottom, above the annotation, growing upward: what was said goes above the text
        // field, so the field stays put.
        let above = (wh - (a.at.1 - oy - MARKER / 2.0 - 6.0)).max(8.0);
        let up = below + CARD_ROOM > wh;
        let said = a.said.iter().map(|s| s.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        let status = match (recording_here, transcribing) {
            (true, _) => {
                let s = o.recording_seconds().unwrap_or_default();
                Some(format!("Listening… {}:{:02}", s / 60, s % 60))
            }
            (false, true) => Some("Transcribing…".to_string()),
            _ if a.said.iter().any(|s| s.text.trim().is_empty()) && said.is_empty() => Some("Nothing heard".into()),
            _ => None,
        };
        let said_row = (!said.is_empty() || status.is_some()).then(|| {
            let text = match (said.is_empty(), &status) {
                (false, Some(status)) => format!("“{said}” · {status}"),
                (false, None) => format!("“{said}”"),
                (true, status) => status.clone().unwrap_or_default(),
            };
            div()
                .px(px(8.))
                .pb(px(2.))
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(color(0xffffff, 0.72))
                .child(text)
        });
        let (unsay, tip) = (self.overlay.clone(), self.overlay.clone());
        let key = card.key;
        let trash = has_said.then(|| {
            div()
                .id("card-unsay")
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .cursor_pointer()
                .hover(|s| s.bg(color(0xffffff, 0.12)))
                .on_hover(move |hovered, _, cx| {
                    tip.update(cx, |o, cx| {
                        o.card_tip = hovered.then_some("Delete what you said, to say it again".into());
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    unsay.update(cx, |o, cx| {
                        o.unsay(key);
                        o.card_tip = None;
                        cx.notify();
                    });
                })
                .child(icon("trash", gpui::white()))
        });
        let dots = Intent::ALL.into_iter().enumerate().map(|(i, intent)| {
            let (pick, tip) = (self.overlay.clone(), self.overlay.clone());
            let on = card.intent == Some(intent);
            let name: SharedString = format!("{} ⌘{}", capitalized(intent.as_str()), i + 1).into();
            div()
                .id(SharedString::from(format!("intent-{}", intent.as_str())))
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .cursor_pointer()
                .when(on, |d| d.border_2().border_color(gpui::white()))
                .on_hover(move |hovered, _, cx| {
                    let name = name.clone();
                    tip.update(cx, |o, cx| {
                        o.card_tip = hovered.then_some(name);
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    pick.update(cx, |o, cx| {
                        o.choose_intent(intent);
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .size(px(if on { 10. } else { 11. }))
                        .rounded_full()
                        .bg(color(style::intent_color(intent), 1.0)),
                )
        });
        div()
            .id("card")
            .occlude()
            .absolute()
            .left(px(x as f32))
            .when(!up, |d| d.top(px(below as f32)))
            .when(up, |d| d.bottom(px(above as f32)))
            .w(px(width as f32))
            .p(px(6.))
            .flex()
            .when(!up, |d| d.flex_col())
            .when(up, |d| d.flex_col_reverse())
            .gap(px(4.))
            .rounded(px(12.))
            .bg(color(style::PANEL, style::PANEL_ALPHA))
            .border_1()
            .border_color(color(0xffffff, 0.12))
            .shadow_lg()
            .text_color(gpui::white())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .child(div().flex_1().child(card.input.clone()))
                    .child(div().flex().items_center().h(px(30.)).gap(px(1.)).children(dots))
                    .child(talk_button(&self.overlay, o, "card-talk", false))
                    .children(trash),
            )
            .children(said_row)
            .children(o.card_tip.clone().map(|tip| {
                div()
                    .absolute()
                    .bottom(px(-26.))
                    .right(px(30.))
                    .px(px(7.))
                    .py(px(2.))
                    .rounded(px(6.))
                    .bg(color(style::PANEL, style::PANEL_ALPHA))
                    .text_size(px(11.))
                    .child(tip)
            }))
            .into_any_element()
    }

    /// The right-click menu, at the pointer, on the window (`ww` × `wh`, below `top`).
    fn menu(&self, menu: &Menu, (ox, oy): (f64, f64), (ww, wh): (f64, f64), top: f64) -> AnyElement {
        let height = menu.items.len() as f64 * 26.0 + 10.0;
        let x = (menu.at.0 - ox).min(ww - MENU_WIDTH - 6.0).max(6.0);
        let y = if menu.at.1 - oy + height > wh - 6.0 { menu.at.1 - oy - height } else { menu.at.1 - oy };
        let y = y.max(top + 6.0);
        let rows = menu.items.iter().enumerate().map(|(i, (label, key, item))| {
            let (overlay, hover) = (self.overlay.clone(), self.overlay.clone());
            let item = *item;
            let lit = menu.hovered == Some(i);
            div()
                .id(SharedString::from(format!("menu-{i}")))
                .flex()
                .justify_between()
                .items_center()
                .h(px(26.))
                .px(px(10.))
                .rounded(px(6.))
                .text_size(px(13.))
                .cursor_pointer()
                .when(lit, |d| d.bg(color(style::UI, 1.0)).text_color(color(style::UI_INK, 1.0)))
                .on_hover(move |hovered, _, cx| {
                    hover.update(cx, |o, cx| {
                        if let Some(menu) = &mut o.menu {
                            if *hovered {
                                menu.hovered = Some(i);
                            } else if menu.hovered == Some(i) {
                                menu.hovered = None;
                            }
                        }
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    overlay.update(cx, |o, cx| o.menu_item(item, window, cx));
                })
                .child(label.clone())
                .child(div().opacity(0.55).child(key.clone()))
        });
        div()
            .id("menu")
            .occlude()
            .absolute()
            .left(px(x as f32))
            .top(px(y as f32))
            .w(px(MENU_WIDTH as f32))
            .p(px(5.))
            .rounded(px(9.))
            .bg(color(style::PANEL, style::PANEL_ALPHA))
            .border_1()
            .border_color(color(0xffffff, 0.12))
            .shadow_lg()
            .text_color(gpui::white())
            .children(rows)
            .into_any_element()
    }

    /// The keys, as key caps: what each one does in a word or two.
    fn help(&self, o: &Overlay) -> AnyElement {
        let k = |c: Cmd| o.keys.label(c);
        let talk = keys::label(&o.keys.talk);
        let left: Vec<(&str, String)> = vec![
            ("Measure", k(Cmd::Tool(Tool::Measure))),
            ("Annotate", k(Cmd::Tool(Tool::Annotate))),
            ("Draw (⇧ arrow)", k(Cmd::Tool(Tool::Draw))),
            ("Screenshot", k(Cmd::Tool(Tool::Shot))),
            ("Hold distance", format!("{} {}", k(Cmd::HoldH), k(Cmd::HoldV))),
            ("Guide", format!("{} {}", k(Cmd::GuideH), k(Cmd::GuideV))),
            ("Tolerance", format!("{} {}", k(Cmd::TolDown), k(Cmd::TolUp))),
            ("Points ↔ pixels", k(Cmd::Units)),
        ];
        let mut right: Vec<(&str, String)> = vec![
            ("Talk (hold)", talk),
            ("Undo", k(Cmd::Undo)),
            ("Clear", k(Cmd::Clear)),
            ("Hide all", k(Cmd::Hide)),
            ("Toolbar", k(Cmd::Toolbar)),
            ("Copy Markdown", k(Cmd::CopyMarkdown)),
            ("Live screen", o.spec.freeze_shortcut.clone()),
            ("Done", k(Cmd::Done)),
            ("Close", k(Cmd::Close)),
        ];
        if o.spec.batch {
            right.insert(8, ("Done and send", k(Cmd::Send)));
        }
        let cap = |text: String| {
            div()
                .min_w(px(18.))
                .h(px(18.))
                .px(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(4.))
                .bg(color(0xffffff, 0.1))
                .border_1()
                .border_color(color(0xffffff, 0.16))
                .text_size(px(11.))
                .child(text)
        };
        let column = |rows: Vec<(&str, String)>| {
            div().flex_1().flex().flex_col().gap(px(5.)).children(rows.into_iter().map(|(label, keys)| {
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(px(12.))
                    .text_size(px(12.))
                    .child(div().opacity(0.85).child(label.to_string()))
                    .child(
                        div()
                            .flex()
                            .gap(px(3.))
                            .children(keys.split(' ').filter(|k| !k.is_empty()).map(|k| cap(k.to_string()))),
                    )
            }))
        };
        div()
            .id("keys")
            .occlude()
            .w(px(420.))
            .p(px(14.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .rounded(px(12.))
            .bg(color(style::PANEL, 0.98))
            .border_1()
            .border_color(color(0xffffff, 0.12))
            .shadow_lg()
            .text_color(gpui::white())
            .child(div().flex().gap(px(22.)).child(column(left)).child(column(right)))
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(color(style::UI, 1.0))
                    .child("Drag: size · ⌥ each object · ⌘ as drawn · right-click: more"),
            )
            .into_any_element()
    }
}

fn capitalized(s: &'static str) -> &'static str {
    match s {
        "fix" => "Fix",
        "change" => "Change",
        "question" => "Question",
        "approve" => "Approve",
        "blocking" => "Blocking",
        "important" => "Important",
        "suggestion" => "Suggestion",
        other => other,
    }
}

/// The clock: every `TICK`, the app's commands and what changes with time, then the windows.
pub fn run_clock(overlay: &Entity<Overlay>, cx: &mut App) {
    // It keeps the overlay as long as the app runs (its windows open from here).
    let overlay = overlay.clone();
    cx.spawn(async move |cx| {
        loop {
            if let Err(err) = crate::windows::sync(&overlay, cx) {
                eprintln!("asides-overlay: {err:#}");
                break;
            }
            // Handing the system back, the frozen screens give way as soon as it's back.
            let handing = cx
                .update(|cx| {
                    let o = overlay.read(cx);
                    o.handoff.is_some() || o.taking_over.is_some()
                })
                .unwrap_or_default();
            cx.background_executor().timer(if handing { HANDOFF_TICK } else { TICK }).await;
            if overlay.update(cx, |o, cx| o.tick(cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// The pointer, as a session has it: the screen under it, and where on it (none in development
/// windows, which show only part of the screen).
fn pointer(spec: &Spec) -> Option<(usize, f64, f64)> {
    if spec.windowed.is_some() {
        return None;
    }
    let (mx, my) = crate::native::mouse();
    crate::windows::pointer_screen(&spec.screens).map(|i| {
        let f = spec.screens[i].frame;
        (i, mx - f.x, f.y + f.height - my)
    })
}

/// The part of each screen the Dock and the menu bar leave (see `Overlay::visible`).
fn visible_frames(screens: &[SpecScreen]) -> Vec<PtRect> {
    screens.iter().map(|s| crate::native::visible_frame(s.frame)).collect()
}
