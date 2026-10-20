//! The overlay's windows: one per screen, borderless, above everything, showing the frozen screen
//! and what's on it, with the pill: the microphone, where the aside goes, send and close. The tools
//! (Annotate, the default; Measure, Draw, Shot) are on keys and in the right-click menu.
//!
//! `Overlay` holds the session (`asides_core::overlay::Session`) and everything around it; each
//! window's `ScreenView` draws its screen from it and feeds it the mouse and keyboard.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use asides_core::agents::AgentKind;
use asides_core::config::OverlayKeys;
use asides_core::helper::{Command, Event, Item, Spec, SpecScreen, Target};
use asides_core::inspect::{Annotation, Axis, Element, Intent, Measurement, PtRect, Said, Severity, Shape, num};
use asides_core::keys::{self, Chord, Mods};
use asides_core::measure::{Frame, Tolerance};
use asides_core::overlay::{MARKER, Part, Placed, Saved, Session, Tool, lookups, move_lookup};
use futures::StreamExt;
use gpui::{
    AnyElement, AnyWindowHandle, App, BorderStyle, Bounds, Context, CursorStyle, DispatchPhase, Div, Entity,
    FocusHandle, Focusable, FontWeight, Hitbox, HitboxBehavior, Hsla, ImageSource, KeyDownEvent, KeyUpEvent, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point,
    Render, RenderImage, Rgba, ScrollHandle, SharedString, Window, canvas, deferred, div, img, point, prelude::*, px,
    quad, size, svg, transparent_black,
};

use crate::controls::{Fade, Glide, Slot, fade_in};
use crate::export::{self, Exporter};
use crate::input::TextInput;
use crate::{finish, glass, io, style};

const TOAST: Duration = Duration::from_millis(1600);
/// A recording about an annotation ended: shown as being transcribed this long at most, until the
/// app says it is (it doesn't for one too short to keep).
const AWAIT_TRANSCRIPT: Duration = Duration::from_millis(1500);
/// Esc pressed again within this closes without adding anything.
const CLOSE_AGAIN: Duration = Duration::from_millis(2500);
/// In a card, Space held this long talks about its annotation; shorter, it types a space.
const SPACE_HOLD: Duration = Duration::from_millis(300);
const NOTICE: Duration = Duration::from_millis(5000);
/// The pill's list, opened by pointing at where the aside goes, stays this long after the pointer
/// leaves the name and the list.
const LIST_LINGER: Duration = Duration::from_millis(200);
/// Tab held moves to the next session this often, slower than keys repeat: each name can be read.
const TARGET_REPEAT: Duration = Duration::from_millis(240);
const CARD_WIDTH: f64 = 440.0;
/// The card's trash button and the gap before it.
const TRASH_WIDTH: f64 = 34.0;
/// The card opens above its annotation when there's less than this below it.
const CARD_ROOM: f64 = 240.0;
const MENU_WIDTH: f64 = 260.0;
/// The pill's place, unless the user moved it: this far below the menu bar, in the middle.
pub const PILL_TOP: f64 = 10.0;

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
    Restore,
    Help,
    Target,
    Done,
    DoneWhole,
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
                (p(&k.restore), Cmd::Restore),
                (p(&k.help), Cmd::Help),
                (p(&k.target), Cmd::Target),
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
    /// The row under the pointer (a glass pill behind it).
    hovered: Option<usize>,
}

/// An annotation whose element the app is looking up.
#[derive(Clone, Copy, Debug)]
struct Lookup {
    screen: usize,
    index: usize,
    number: u32,
    part: Part,
}

/// How loud what the microphone hears is (0 to 1), as the pill shows it: from where it was when
/// the app last said (every 50 ms), easing toward what it said, quickly up and gently down.
#[derive(Clone, Copy)]
struct Level {
    from: f32,
    to: f32,
    at: Instant,
}

impl Default for Level {
    fn default() -> Level {
        Level { from: 0.0, to: 0.0, at: Instant::now() }
    }
}

impl Level {
    fn now(&self) -> f32 {
        let tau = if self.to > self.from { 0.05 } else { 0.2 };
        self.to + (self.from - self.to) * (-self.at.elapsed().as_secs_f32() / tau).exp()
    }

    fn set(&mut self, to: f32) {
        *self = Level { from: self.now(), to, at: Instant::now() };
    }

    fn settled(&self) -> bool {
        (self.now() - self.to).abs() < 0.002
    }
}

/// A freeze's work, for the aside: drawing its images takes a moment, done once the live screen
/// shows.
struct Keeping {
    session: Session,
    frames: Vec<Arc<Frame>>,
    exporter: Option<Exporter>,
    shots: Vec<(f32, String)>,
    /// Which freeze it was, when (seconds into the session), and the screen under the pointer.
    freeze: usize,
    frozen_at: f32,
    screen: usize,
}

pub struct Overlay {
    pub session: Session,
    spec: Spec,
    keys: Keys,
    accent: u32,
    images: Vec<Arc<RenderImage>>,
    /// Each frozen screen, small and blurred: the glass of what's drawn on it.
    backdrops: Vec<Arc<glass::Backdrop>>,
    /// The parts of them drawn under surfaces (by screen and part), as images.
    glass: glass::Pieces,
    frames: Vec<Arc<Frame>>,
    exporter: Option<Exporter>,
    pub windows: Vec<ScreenWindow>,
    dir: PathBuf,
    opened: Instant,
    card: Option<Card>,
    menu: Option<Menu>,
    /// A short message by the pill, and until when. It waits for the pill to be drawn as it is now
    /// (`toast_waits`), and its window to fit it: placed under the pill as it was, it jumped. The
    /// one shown (`toast_shown`) stays until then, and the new one takes its place.
    pub toast: Option<(SharedString, Instant)>,
    pub toast_waits: bool,
    pub toast_shown: Option<SharedString>,
    /// When the next session was last chosen by key (`TARGET_REPEAT`).
    target_switched: Option<Instant>,
    /// The pill's window is asked to follow the pill at once (`windows::follow_now`).
    pub following: bool,
    /// Put somewhere by the user, the pill's kept corner moved as its width eased: kept again once
    /// the width settles (`windows::follow_toolbar`).
    pub recentered: bool,
    /// On the pill, where the aside goes (Tab, the list) and the tool (none for Annotate, or
    /// live; the gap before it included): their widths ease to what they show, the pill's with
    /// them. Each step shows as the pill's window follows (`take_next`), which takes the pill's
    /// width as it's drawn: drawn first in the old window, a longer name was cut off for a frame,
    /// and the glass showed past a shorter one.
    pub label: Slot<SharedString>,
    pub badge: Slot<Tool>,
    /// The keys under the pill: shown (`?`, its button, the first freeze), or while the pointer is
    /// on its button; fading in and out.
    help: bool,
    help_hover: bool,
    pub keys_sheet: Fade,
    /// Where the aside goes, and whether the user picked it (the app's choice no longer applies).
    target: Option<Target>,
    chose_target: bool,
    /// The pill's list of sessions: open, or fading in or out.
    pub list: Fade,
    /// It opened as the pointer came onto where the aside goes (not clicked): it closes once the
    /// pointer leaves that and the list (since when it's away).
    list_hovered: bool,
    list_left_at: Option<Instant>,
    /// Where its rows are scrolled to (its bar), past `LIST_ROWS_HEIGHT`.
    list_scroll: ScrollHandle,
    /// The agent a new session starts (the list's chips).
    new_agent: AgentKind,
    /// Choose Folder… was clicked (the clock opens the panel, after the click), or its panel shows.
    pub folder_asked: bool,
    pub folder_panel: bool,
    /// The app was asked to download the speech model.
    asked_model: bool,
    /// Seconds recorded when the app last said so, and when it did.
    recording: Option<(f32, Instant)>,
    /// How loud what the microphone hears is, as the pill's microphone fills.
    level: Level,
    /// The annotation that recording is about (started from its card).
    recording_for: Option<u32>,
    /// Recordings about annotations being transcribed: (annotation key, take).
    transcribing: Vec<(u32, u64)>,
    /// A recording about this annotation just ended, and when (until the app says it's being
    /// transcribed): the card shows it as such, without a gap.
    awaiting: Option<(u32, Instant)>,
    shown_seconds: u32,
    pending: HashMap<u64, Lookup>,
    next_id: u64,
    talking: bool,
    /// A toolbar button's name while the pointer is on it.
    hint: Option<(&'static str, SharedString)>,
    /// The card's dot under the pointer, named above the card.
    card_tip: Option<SharedString>,
    /// Screenshots taken: when, and the file in the session directory.
    shots: Vec<(f32, String)>,
    shot_count: usize,
    commands: Receiver<Command>,
    done: bool,
    /// Over the live screen: only the pill shows, and the user's apps take the mouse and the
    /// keyboard. Frozen, the screens show under it, and take them.
    pub live: bool,
    /// The app was asked to freeze the screens, and the tool to pick then.
    freezing: Option<Option<Tool>>,
    /// The pill's window, frozen and live, and the screen it's on.
    pub toolbar_window: Option<ScreenWindow>,
    pub toolbar_screen: usize,
    /// Which windows show: the pill's alone (`Some(true)`, live) or over the screens' (`Some(false)`).
    pub shown: Option<bool>,
    /// Where the user put the pill (its top-left corner, as a fraction of the screen).
    pub toolbar_at: Option<(f64, f64)>,
    /// In the pill's window, as last drawn: the pill with its list, the pill, the list. Its window
    /// fits them, and macOS blurs what's behind the pill and the list (`windows::follow_toolbar`):
    /// the shapes it last did, with the window's size.
    pub toolbar_bounds: Option<Bounds<Pixels>>,
    pub pill_bounds: Option<Bounds<Pixels>>,
    pub list_bounds: Option<Bounds<Pixels>>,
    /// Where the aside goes, on the pill: pointed at, the list opens.
    pub target_bounds: Option<Bounds<Pixels>>,
    pub keys_bounds: Option<Bounds<Pixels>>,
    pub glass_shapes: Option<(crate::windows::Shapes, (f64, f64))>,
    /// How much of the list and of the keys showed then (fading, the glass behind them fades too).
    pub glass_fades: (f32, f32),
    /// Where the pill is: its screen, and its frame there (points from the screen's top left), for
    /// what goes with it on the frozen screen (`ScreenView::notes`).
    pub pill_rect: Option<(usize, PtRect)>,
    /// Which way the list opens: to the pill's left (near the screen's right edge) and above it
    /// (near the bottom).
    pub list_left: bool,
    pub list_up: bool,
    /// The pill is hidden with the rest (`Cmd::Hide`).
    pub pill_hidden: bool,
    /// The pill being moved: where it was grabbed, from its top-left corner; and let go (macOS may
    /// still say the button's down as the mouse-up comes).
    pub moving_toolbar: Option<(f64, f64)>,
    pub let_go: bool,
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
    /// When the screens last froze, in seconds into the session: where its screenshots go.
    frozen_at: f32,
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
    /// The frozen screens just showed: the system is the overlay's once they're on screen (taken
    /// at once, the menu bar hid a frame before they covered it), and since when.
    pub taking_over: Option<Instant>,
    /// Windows opened for the screens and the pill before the session (`windows::Spare`).
    pub spares: Vec<crate::windows::Spare>,
    pub spare_toolbar: Option<crate::windows::Spare>,
}

impl Overlay {
    pub fn new(spec: Spec, frames: Vec<Frame>, dir: PathBuf, commands: Receiver<Command>) -> Overlay {
        // The session's pixels: shared, a screen's are tens of megabytes.
        let frames: Vec<Arc<Frame>> = frames.into_iter().map(Arc::new).collect();
        let screens = spec.screens.iter().zip(&frames).map(|(s, f)| Session::from_spec(s, f.clone())).collect();
        let mut session = Session::new(screens, Tolerance(spec.tolerance.min(100)), spec.units);
        // A click pins what's under it; Measure is a key away.
        session.tool = Tool::Annotate;
        // Opened over the live screen, there are no frozen screens yet.
        session.cursor = pointer(&spec).filter(|_| !spec.live);
        let images = frames.iter().map(|f| render_image(f)).collect();
        let backdrops = frames.iter().map(|f| Arc::new(glass::Backdrop::of(f))).collect();
        let visible = visible_frames(&spec.screens);
        let accent = style::rgb_hex(style::parse_color(&spec.color));
        let recording = spec.recording.map(|s| (s, Instant::now()));
        Overlay {
            session,
            keys: Keys::new(&spec.keys),
            accent,
            images,
            backdrops,
            glass: Vec::new(),
            frames,
            exporter: None,
            windows: Vec::new(),
            dir,
            opened: Instant::now(),
            card: None,
            menu: None,
            toast: None,
            toast_waits: false,
            toast_shown: None,
            target_switched: None,
            following: false,
            recentered: false,
            label: Slot::new(spec.targets.label(spec.targets.chosen.as_ref()).into()),
            badge: Slot::empty(),
            help: spec.tips && !spec.live,
            help_hover: false,
            keys_sheet: Fade::default(),
            target: spec.targets.chosen.clone(),
            chose_target: false,
            list: Fade::default(),
            list_hovered: false,
            list_left_at: None,
            list_scroll: ScrollHandle::new(),
            folder_asked: false,
            folder_panel: false,
            new_agent: spec.targets.agents.first().copied().unwrap_or(AgentKind::Claude),
            asked_model: false,
            recording,
            level: Level::default(),
            recording_for: None,
            transcribing: Vec::new(),
            awaiting: None,
            shown_seconds: 0,
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
            toolbar_at: spec.pill_at,
            toolbar_bounds: None,
            pill_bounds: None,
            list_bounds: None,
            target_bounds: None,
            keys_bounds: None,
            glass_shapes: None,
            glass_fades: (0.0, 0.0),
            pill_rect: None,
            list_left: false,
            list_up: false,
            pill_hidden: false,
            moving_toolbar: None,
            let_go: false,
            tips: spec.tips && spec.live,
            space_hold: None,
            talked: spec.recording.is_some(),
            close_armed: None,
            kept: Vec::new(),
            freezes: 0,
            frozen_at: 0.0,
            keeping: None,
            saved: Saved::default(),
            previous_app: None,
            stale_images: Vec::new(),
            visible,
            handoff: None,
            closing: false,
            taking_over: None,
            spares: Vec::new(),
            spare_toolbar: None,
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
        self.backdrops = frames.iter().map(|f| Arc::new(glass::Backdrop::of(f))).collect();
        self.stale_images.extend(self.glass.drain(..).map(|(_, image)| image));
        self.frames = frames;
        self.exporter = None;
        // Over the live screen still: the Dock is where the user keeps it.
        self.visible = visible_frames(&screens);
        self.spec.screens = screens;
        // Measure's lines start where the pointer is, not where it was when the screen last froze.
        self.session.cursor = pointer(&self.spec);
        self.live = false;
        self.frozen_at = self.elapsed();
        self.help = std::mem::take(&mut self.tips);
        self.hint = None;
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
            frozen_at: self.frozen_at,
            screen: self.cursor_screen(),
        };
        self.saved = keeping.session.saved();
        self.freezes += 1;
        keeping
    }

    /// The last freeze, kept for the aside if it isn't yet.
    pub fn keep_last(&mut self) {
        if let Some(keeping) = self.keeping.take()
            && let Err(err) = self.keep(keeping, false)
        {
            eprintln!("could not keep the frozen screen: {err:#}");
        }
    }

    /// What was done on a freeze, kept for the aside: only that, unless `whole`, all of the screen
    /// (even with nothing on it).
    fn keep(&mut self, k: Keeping, whole: bool) -> anyhow::Result<()> {
        if k.session.inspections().is_empty() && k.shots.is_empty() && !whole {
            return Ok(());
        }
        let exporter = k.exporter.unwrap_or_else(|| {
            let frames: Vec<&Frame> = k.frames.iter().map(|f| &**f).collect();
            Exporter::new(&frames, style::parse_color(&self.spec.color))
        });
        let freeze = (k.freeze, k.frozen_at);
        let items = finish::items(&k.session, &exporter, &self.spec, &self.dir, whole, &k.shots, k.screen, freeze)?;
        self.kept.extend(items);
        Ok(())
    }

    /// The pill, pressed at `at` (in its window) anywhere but its buttons: it follows the pointer
    /// until let go (`windows::follow_toolbar`), its buttons and names quiet meanwhile.
    fn grab_toolbar(&mut self, at: Point<Pixels>) {
        let origin = self.pill_bounds.map_or(at, |b| b.origin);
        self.moving_toolbar = Some((f64::from(f32::from(at.x - origin.x)), f64::from(f32::from(at.y - origin.y))));
        (self.let_go, self.hint) = (false, None);
    }

    /// Opens or closes the pill's list, fading.
    pub fn show_list(&mut self, open: bool) {
        self.list.set(open);
        if !open {
            (self.list_hovered, self.list_left_at) = (false, None);
        }
    }

    /// The pointer came onto where the aside goes: the list opens, as the keys do on `?`, until the
    /// pointer leaves it (`tick`). A click then keeps it open.
    fn hover_list(&mut self) {
        if !self.list.is_open() && self.moving_toolbar.is_none() {
            self.show_list(true);
            (self.list_hovered, self.help_hover) = (true, false);
        }
    }

    /// Where the aside goes, clicked: the list opened by pointing stays; otherwise it opens or
    /// closes.
    fn click_list(&mut self) {
        match self.list_hovered {
            true => (self.list_hovered, self.list_left_at) = (false, None),
            false => self.show_list(!self.list.is_open()),
        }
    }

    /// The keys under the pill, as `help` and `help_hover` say (not over its list): whether that
    /// changed.
    fn show_keys(&mut self) -> bool {
        let open = (self.help || self.help_hover) && !self.list.is_open() && !self.pill_hidden;
        let changed = open != self.keys_sheet.is_open();
        self.keys_sheet.set(open);
        changed
    }

    /// The name of the pill's button `id` under the pointer, by the pill. Leaving a button clears
    /// only its own name (whichever of the two GPUI tells first): going straight to the next one, the
    /// name changes without a gap. A press clears it, as a tooltip goes.
    fn show_hint(&mut self, id: &'static str, name: SharedString, hovered: bool, cx: &mut Context<Self>) {
        if hovered && self.moving_toolbar.is_none() {
            self.hint = Some((id, name));
        } else if self.hint.as_ref().is_some_and(|(shown, _)| *shown == id) {
            self.hint = None;
        }
        // Where the aside goes opens the list; another button, opened so, closes it.
        match (hovered, id) {
            (true, "target") => self.hover_list(),
            (true, _) if self.list_hovered => self.show_list(false),
            _ => {}
        }
        cx.notify();
    }

    /// The pill, just drawn (`bounds`, in its window): what goes with it waits for this.
    pub fn drawn_pill(&mut self, bounds: Bounds<Pixels>) {
        self.pill_bounds = Some(bounds);
    }

    /// The pill and what's under it, just drawn (`bounds`, in its window): whether its window must
    /// follow at once (`windows::follow_now`). A name or a tool to show, or their widths to ease,
    /// the pill or what's under it drawn at a size the window isn't, the list or the keys fading
    /// (the glass behind them fades too), or a toast waits.
    pub fn drawn_toolbar(&mut self, bounds: Bounds<Pixels>) -> bool {
        self.toolbar_bounds = Some(bounds);
        let p = |v: Pixels| f64::from(f32::from(v));
        let size = |v: Pixels| (p(v) + 2.0 * TOOLBAR_MARGIN).ceil();
        let fits = self.glass_shapes.as_ref().is_some_and(|(_, (w, h))| {
            (w - size(bounds.size.width)).abs() < 0.5 && (h - size(bounds.size.height)).abs() < 0.5
        });
        let fading = (self.list.shown(), self.keys_sheet.shown()) != self.glass_fades;
        (self.pill_moving() || !self.pill_fits() || !fits || fading || self.toast_waits) && self.shown.is_some()
    }

    /// The tool whose name the pill shows: none for Annotate, or live.
    fn badge_wanted(&self) -> Option<Tool> {
        (!self.live && self.session.tool != Tool::Annotate).then_some(self.session.tool)
    }

    /// The name or the tool on the pill is still to change, or their widths to ease.
    pub fn pill_moving(&self) -> bool {
        self.label.moving(Some(&self.target_label())) || self.badge.moving(self.badge_wanted().as_ref())
    }

    /// The pill's screen's pixels to a point: the pill is drawn on them.
    pub fn pill_scale(&self) -> f32 {
        self.spec.screens.get(self.toolbar_screen).map_or(2.0, |s| s.screen.scale) as f32
    }

    /// Where the aside goes, as its name reads.
    fn target_label(&self) -> SharedString {
        self.spec.targets.label(self.target.as_ref()).into()
    }

    /// The name and the tool on the pill, a step on, show now, and the pill's width with them (for
    /// its window to take as it's drawn).
    pub fn take_next(&mut self, cx: &mut Context<Self>) -> Option<f64> {
        let (label, badge, scale) = (self.target_label(), self.badge_wanted(), self.pill_scale());
        let grew = [self.label.step(Some(&label), scale), self.badge.step(badge.as_ref(), scale)];
        if grew.iter().all(Option::is_none) {
            return None;
        }
        cx.notify();
        let drawn = self.pill_bounds.map(|b| f64::from(f32::from(b.size.width)))?;
        Some(drawn + grew.iter().flatten().map(|&g| f64::from(g)).sum::<f64>())
    }

    /// The pill's window fits the pill as last drawn, and what goes with it is placed for it.
    fn pill_fits(&self) -> bool {
        let p = |v: Pixels| f64::from(f32::from(v));
        match (self.pill_rect, self.pill_bounds) {
            (Some((_, r)), Some(b)) => {
                (r.width - p(b.size.width)).abs() < 0.5 && (r.height - p(b.size.height)).abs() < 0.5
            }
            _ => false,
        }
    }

    /// The moved pill, remembered for next time.
    pub fn placed_toolbar(&mut self) {
        io::emit(&Event::PillPlaced { at: self.toolbar_at });
    }

    /// The frozen screen `screen` under `bounds` (in a window `offset` points into it), blurred: the
    /// glass under a surface drawn on it. Its images stay while the surface stays put.
    pub fn glass_under(
        &mut self,
        screen: usize,
        (ox, oy): (f64, f64),
        bounds: Bounds<Pixels>,
    ) -> Option<Arc<RenderImage>> {
        let backdrop = self.backdrops.get(screen)?.clone();
        let scale = self.session.screens.get(screen)?.screen.scale;
        let p = |v: Pixels| f32::from(v) as f64;
        let (left, top) = (p(bounds.origin.x) + ox, p(bounds.origin.y) + oy);
        let (right, bottom) = (left + p(bounds.size.width), top + p(bounds.size.height));
        let key = (screen, backdrop.cover([left * scale, top * scale, right * scale, bottom * scale]));
        if let Some((_, image)) = self.glass.iter().find(|(k, _)| *k == key) {
            return Some(image.clone());
        }
        let image = backdrop.crop(key.1);
        // Surfaces at once (labels, a card, the keys, a menu): older parts go once the next frame
        // is drawn (`windows::drop_stale`).
        if self.glass.len() >= 32 {
            let (_, old) = self.glass.remove(0);
            self.stale_images.push(old);
        }
        self.glass.push((key, image.clone()));
        Some(image)
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
        self.toast_waits = true;
    }

    /// A message the user must read (an error from the app): it stays longer.
    fn notice(&mut self, text: impl Into<SharedString>) {
        self.toast = Some((text.into(), Instant::now() + NOTICE));
        self.toast_waits = true;
    }

    /// Without a speech model there's nothing to listen with: the app is asked to download one (the
    /// first time), and the user is told.
    fn needs_model(&mut self) -> bool {
        if self.spec.model.ready {
            return false;
        }
        if !std::mem::replace(&mut self.asked_model, true) {
            io::emit(&Event::DownloadModel);
        }
        let text = match self.spec.model.downloading {
            Some(p) => format!("Getting the voice model ready… {:.0}%", p * 100.0),
            None => format!("Downloading the voice model, {}…", self.spec.model.label),
        };
        self.notice(text);
        true
    }

    /// Where the aside goes now.
    fn choose_target(&mut self, target: Target) {
        self.target = Some(target.clone());
        self.chose_target = true;
        self.show_list(false);
        io::emit(&Event::Target { target });
    }

    /// Choose Folder… was clicked: where its panel opens (beside the latest folder). `None` unless
    /// it was just asked for, or while the panel shows.
    pub fn take_folder_request(&mut self) -> Option<PathBuf> {
        if !std::mem::take(&mut self.folder_asked) || self.folder_panel {
            return None;
        }
        self.folder_panel = true;
        let latest = self.spec.targets.folders.first().and_then(|f| f.path.parent().map(Path::to_path_buf));
        Some(latest.or_else(|| std::env::var_os("HOME").map(PathBuf::from)).unwrap_or_else(|| PathBuf::from("/")))
    }

    /// The folder chosen in the panel (`None`: cancelled): a new session there, with the agent picked.
    pub fn folder_chosen(&mut self, dir: Option<PathBuf>) {
        self.folder_panel = false;
        if let Some(dir) = dir {
            self.choose_target(Target::New { agent: self.new_agent, dir });
        }
    }

    /// Tab: the next session the aside can go to.
    fn next_target(&mut self) {
        self.target_switched = Some(Instant::now());
        let next = self.spec.targets.next(self.target.as_ref());
        let label = self.spec.targets.label(Some(&next));
        self.choose_target(next);
        self.toast(format!("Sends to {label}"));
    }

    /// The pill's microphone: listening, or not (a model first, when there's none).
    fn mic(&mut self) {
        if self.needs_model() {
            return;
        }
        io::emit(&Event::Mic { on: !self.listening() });
    }

    /// The pill's microphone is on: what's said goes in the aside (not in one pin's card).
    fn listening(&self) -> bool {
        self.recording.is_some() && self.recording_for.is_none()
    }

    /// Measure's modifiers. With ⌃ down, they're part of one of asides' shortcuts (⌃⌥P, ⌃⌥Space),
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
        // A click away from a menu, the keys or the pill's list closes them, and only that.
        if self.menu.take().is_some() || self.help || self.list.is_open() {
            self.help = false;
            self.show_list(false);
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
        self.session.move_to(screen, x, y);
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
            // A pin is enough: what the user says goes with it. A click on its number opens its card,
            // to type. With the microphone off, the user writes instead: the card opens at once.
            Placed::Annotation { screen, index } => {
                self.look_up(screen, index);
                if !self.listening() {
                    self.open_card(screen, index, window, cx);
                }
            }
            Placed::Edit { screen, index } => self.open_card(screen, index, window, cx),
            Placed::Moved { screen, index } => self.look_up_move(screen, index),
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
            self.pending.insert(id, Lookup { screen, index, number: a.number, part });
            io::emit(&Event::Identify { id, x: ox + x, y: oy + y });
        }
    }

    /// Asks the app what's where a pin was moved to.
    fn look_up_move(&mut self, screen: usize, index: usize) {
        let Some(a) = self.session.screens[screen].annotations.get(index) else { return };
        let Some((x, y)) = move_lookup(a) else { return };
        let (ox, oy) = self.spec.screens[screen].origin;
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, Lookup { screen, index, number: a.number, part: Part::Target });
        io::emit(&Event::Identify { id, x: ox + x, y: oy + y });
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
        self.toast(format!("Screenshot copied ({} × {})", self.n(screen, rect.width), self.n(screen, rect.height)));
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
        // ⌘1–⌘4: what the annotation asks for (the card's dots; with no card, the last one's).
        let intent = (m.platform && !m.shift && !m.alt)
            .then(|| key.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| Intent::ALL.get(i)))
            .flatten()
            .copied();
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
            match (key.as_str(), intent) {
                ("escape", _) => self.close_card(window, cx),
                ("enter", _) if !m.shift => self.close_card(window, cx),
                (_, Some(intent)) => self.choose_intent(intent),
                _ => return,
            }
            cx.stop_propagation();
            return cx.notify();
        }
        if let Some(intent) = intent {
            if let Some((number, now)) = self.session.set_last_intent(intent) {
                let now = now.map_or("no type", |i| capitalized(i.as_str()));
                self.toast(format!("Pin {number} ({now})"));
            }
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
            // Held while the microphone is off, it listens until it's let go.
            let listening = self.recording.is_some() && !self.talking;
            if !e.is_held && !self.talking && !listening && !self.needs_model() {
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
            let soon = self.target_switched.is_some_and(|at| at.elapsed() < TARGET_REPEAT);
            if cmd == Cmd::Target && e.is_held && soon {
                return;
            }
            // Held, a tool's key would put the tool down and pick it up again.
            if matches!(cmd, Cmd::Tool(_)) && e.is_held {
                return;
            }
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
            Cmd::Tool(tool) => s.toggle_tool(tool),
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
            Cmd::Restore => {
                let saved = self.spec.saved.clone();
                if self.session.restore(&saved) {
                    self.toast("Restored the last guides and measurements");
                }
            }
            Cmd::Help => self.help = !self.help,
            Cmd::Target => self.next_target(),
            Cmd::Done => self.finish(false, window, cx),
            Cmd::DoneWhole => self.finish(true, window, cx),
            Cmd::Live => self.go_live(cx),
            Cmd::Close => self.cancel(cx),
        }
    }

    /// A microphone: with the card open, what's said is about its annotation.
    fn talk_button(&mut self, down: bool) {
        if down && !self.talking && self.needs_model() {
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
        if since.elapsed() >= SPACE_HOLD && self.recording.is_none() && !self.needs_model() {
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

    fn finish(&mut self, whole: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.close_card(window, cx);
        self.finish_now(whole, cx);
    }

    fn finish_now(&mut self, whole: bool, cx: &mut Context<Self>) {
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
        // Only what was done on the frozen screen: with nothing on it, what was said goes alone.
        let kept = match self.live {
            true => Ok(()),
            false => {
                let keeping = self.this_freeze();
                self.keep(keeping, whole)
            }
        };
        let outcome = finish::outcome(std::mem::take(&mut self.kept), self.saved.clone(), self.target.clone());
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
        self.toast(format!("{} again closes without sending anything", self.keys.label(Cmd::Close)));
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
            // How loud it is redraws only the pill (`Command::Level`).
            changed |= !matches!(command, Command::Level { .. });
            self.apply_command(command, cx);
        }
        if self.toast.as_ref().is_some_and(|(_, until)| Instant::now() > *until) {
            (self.toast, self.toast_shown) = (None, None);
            changed = true;
        }
        let seconds = self.recording.map_or(0, |(s, at)| (s + at.elapsed().as_secs_f32()) as u32);
        if seconds != self.shown_seconds {
            self.shown_seconds = seconds;
            changed = true;
        }
        if self.space_hold.is_some_and(|(_, talking)| !talking) {
            self.space_talks();
            changed = true;
        }
        if self.awaiting.is_some_and(|(_, at)| at.elapsed() >= AWAIT_TRANSCRIPT) {
            self.awaiting = None;
            changed = true;
        }
        // GPUI isn't told when the pointer leaves the pill's window: the keys go with it, and the
        // list opened by pointing, a moment after.
        if self.help_hover && !self.pointer_on_pill() {
            self.help_hover = false;
        }
        if self.list_hovered && self.list.is_open() {
            match self.pointer_on_list() {
                true => self.list_left_at = None,
                false => {
                    if self.list_left_at.get_or_insert_with(Instant::now).elapsed() >= LIST_LINGER {
                        self.show_list(false);
                        changed = true;
                    }
                }
            }
        }
        changed |= self.show_keys();
        crate::native::watch_pointer(self.help_hover || (self.list_hovered && self.list.is_open()));
        if changed {
            cx.notify();
        }
    }

    /// When what shows next changes with time (the clock runs then, `run_clock`): a toast ends, the
    /// recording's next second, Space held long enough to talk, a transcript waited for long enough,
    /// the list opened by pointing closes.
    pub fn due(&self) -> Option<Instant> {
        let toast = self.toast.as_ref().map(|(_, until)| *until);
        let second = self.recording.map(|(seconds, at)| {
            let next = (seconds + at.elapsed().as_secs_f32()).floor() + 1.0;
            at + Duration::from_secs_f32((next - seconds).max(0.0))
        });
        let space = self.space_hold.filter(|(_, talking)| !talking).map(|(since, _)| since + SPACE_HOLD);
        let awaiting = self.awaiting.map(|(_, at)| at + AWAIT_TRANSCRIPT);
        let linger = self.list_left_at.filter(|_| self.list_hovered).map(|at| at + LIST_LINGER);
        [toast, second, space, awaiting, linger].into_iter().flatten().min()
    }

    /// The pointer is on where the aside goes, the list, or between them (always, in development
    /// windows).
    fn pointer_on_list(&self) -> bool {
        if self.spec.windowed.is_some() {
            return true;
        }
        let view = self.toolbar_window.as_ref().map(|t| t.view);
        let (Some(target), Some(list), Some(f)) =
            (self.target_bounds, self.list_bounds, view.and_then(crate::native::frame))
        else {
            return false;
        };
        let (mx, my) = crate::native::mouse();
        // In the window's points, from its top left, as GPUI lays it out.
        let (x, y) = (mx - f.x, f.y + f.height - my);
        let p = |v: Pixels| f64::from(f32::from(v));
        let edges = |b: Bounds<Pixels>| (p(b.left()), p(b.top()), p(b.right()), p(b.bottom()));
        let (t, l) = (edges(target), edges(list));
        // Between them: the pill's padding and the gap, above the list or under it.
        let bridge = (t.0.min(l.0), t.3.min(l.3), t.2.max(l.2), t.1.max(l.1));
        let inside =
            |(left, top, right, bottom): (f64, f64, f64, f64)| x >= left && x <= right && y >= top && y <= bottom;
        inside(t) || inside(l) || inside(bridge)
    }

    /// The pointer is on the pill (always, in development windows: it's someone else's pointer).
    fn pointer_on_pill(&self) -> bool {
        let Some((screen, pill)) = self.pill_rect.filter(|_| self.spec.windowed.is_none()) else { return true };
        let Some(f) = self.spec.screens.get(screen).map(|s| s.frame) else { return true };
        let (mx, my) = crate::native::mouse();
        let (x, y) = (mx - f.x, f.y + f.height - my);
        x >= pill.x && x <= pill.x + pill.width && y >= pill.y && y <= pill.y + pill.height
    }

    pub fn apply_command(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Element { id, element } => self.identified(id, element.map(|e| *e)),
            Command::Recording { seconds, annotation } => {
                // What was said about an annotation shows in its card.
                if let (Some(key), None) = (self.recording_for, seconds) {
                    self.awaiting = Some((key, Instant::now()));
                }
                self.recording = seconds.map(|s| (s, Instant::now()));
                self.recording_for = seconds.and(annotation);
                self.talked |= seconds.is_some();
            }
            Command::Level { level } => {
                self.level.set(level);
                if let Some(handle) = self.toolbar_window.as_ref().and_then(|t| t.handle) {
                    let _ = handle.update(cx, |_, window, _| window.refresh());
                }
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
            Command::Finish => self.finish_now(false, cx),
            Command::Targets { targets } => {
                // The user's own pick stays while it's there; otherwise the app's.
                let gone = match &self.target {
                    Some(Target::Session { id }) => !targets.sessions.iter().any(|s| &s.id == id),
                    _ => false,
                };
                if !self.chose_target || gone {
                    self.target = targets.chosen.clone();
                }
                if !targets.agents.contains(&self.new_agent)
                    && let Some(&first) = targets.agents.first()
                {
                    self.new_agent = first;
                }
                self.spec.targets = targets;
            }
            Command::Model { model } => {
                let ready = model.ready && !self.spec.model.ready;
                self.spec.model = model;
                if ready {
                    self.toast("The voice model is ready: talk away");
                }
            }
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

    fn identified(&mut self, id: u64, element: Option<Element>) {
        let Some(Lookup { screen, index, number, part }) = self.pending.remove(&id) else { return };
        let Some(mut element) = element else { return };
        let (ox, oy) = self.spec.screens[screen].origin;
        element.frame = element.frame.map(|f| PtRect::new(f.x - ox, f.y - oy, f.width, f.height));
        let same = self.session.screens[screen].annotations.get(index).is_some_and(|a| a.number == number);
        if same {
            self.session.set_element(screen, index, part, element);
        }
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

/// A glass label with white text, centered on `(x, y)` (window points, in a window `offset` points
/// into screen `screen`). Its digits are all as wide, and it's never narrower than a short number:
/// a measurement changing as it's dragged doesn't make it jump.
fn pill(
    overlay: &Entity<Overlay>,
    screen: usize,
    offset: (f64, f64),
    text: impl Into<SharedString>,
    (x, y): (f64, f64),
) -> AnyElement {
    let mut label = frozen_glass(overlay, screen, offset, 5.)
        .min_w(px(LABEL_MIN_WIDTH))
        .flex()
        .justify_center()
        .px(px(6.))
        .py(px(1.5))
        .text_color(gpui::white())
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD);
    label.text_style().get_or_insert_with(Default::default).font_features =
        Some(gpui::FontFeatures(Arc::new(vec![("tnum".into(), 1)])));
    div()
        .absolute()
        .left(px(x as f32 - 150.))
        .top(px(y as f32 - 10.))
        .w(px(300.))
        .h(px(20.))
        .flex()
        .justify_center()
        .items_center()
        .child(label.child(text.into()))
        .into_any_element()
}

/// The narrowest a label gets: a few digits and a unit.
const LABEL_MIN_WIDTH: f32 = 44.0;

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
            return div()
                .id("screen")
                .size_full()
                .relative()
                .overflow_hidden()
                .bg(gpui::black())
                .child(screenshot())
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
        // What pins were moved by their numbers are on, again where they go.
        let mut ghosts: Vec<AnyElement> = Vec::new();
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
                        labels.push(pill(&overlay, screen, (ox, oy), n(gap), label(at)));
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
                            labels.push(pill(
                                &overlay,
                                screen,
                                (ox, oy),
                                n(across.len()),
                                label(((across.start + across.end) / 2.0, ly)),
                            ));
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
                            labels.push(pill(
                                &overlay,
                                screen,
                                (ox, oy),
                                text,
                                label((lx, (down.start + down.end) / 2.0)),
                            ));
                        }
                    }
                    Measurement::Area { rect: r, .. } => {
                        prims.push(Prim::Fill(rect(*r), color(accent, style::AREA_TINT)));
                        prims.push(Prim::Border(rect(*r), accent_c, thin as f32, false));
                        let text = format!("{} × {}", n(r.width), n(r.height));
                        let at = style::area_label(*r, pill_width(&text), sh);
                        labels.push(pill(&overlay, screen, (ox, oy), text, label(at)));
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
                // The open card's type shows as it's chosen.
                let intent = match &o.card {
                    Some(card) if card.key == a.key => card.intent,
                    _ => a.intent,
                };
                let (r, g, b, alpha) = style::annotation_color(a, intent, accent);
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
                // Moved by its number, or being dragged by it: a copy of what it's on where it goes,
                // and an arrow there.
                let dragged = session
                    .moving_pin()
                    .filter(|&(s, i, _)| s == screen && state.annotations.get(i).is_some_and(|m| m.key == a.key));
                let moved = match dragged {
                    Some((.., offset)) => Some(Annotation { moved: Some(offset), ..a.clone() }.moved_region()),
                    None => a.moved.map(|_| a.moved_region()),
                };
                if let Some(Some(dest)) = moved {
                    let from = a.region().unwrap_or(PtRect::new(a.at.0, a.at.1, 0.0, 0.0));
                    if from.width >= 1.0 && from.height >= 1.0 {
                        ghosts.push(
                            div()
                                .absolute()
                                .left(px((dest.x - ox) as f32))
                                .top(px((dest.y - oy) as f32))
                                .w(px(dest.width as f32))
                                .h(px(dest.height as f32))
                                .overflow_hidden()
                                .opacity(0.92)
                                .shadow_lg()
                                .child(
                                    img(ImageSource::Render(o.images[screen].clone()))
                                        .absolute()
                                        .left(px(-from.x as f32))
                                        .top(px(-from.y as f32))
                                        .w(px(sw as f32))
                                        .h(px(sh as f32)),
                                )
                                .into_any_element(),
                        );
                        prims.push(Prim::Border(rect(dest), ink, 1.5, false));
                    }
                    let (start, end) = style::move_arrow(from, dest);
                    arrow_prims(&mut prims, to(start), to(end), ink);
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
                        frozen_glass(&overlay, screen, (ox, oy), 6.)
                            .absolute()
                            .left(px((mx + MARKER / 2.0 + 6.0) as f32))
                            .top(px((my - 11.0) as f32))
                            .max_w(px(360.))
                            .px(px(8.))
                            .py(px(3.))
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
                    labels.push(pill(&overlay, screen, (ox, oy), text, label((lx, ly))));
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
                            labels.push(pill(&overlay, screen, (ox, oy), text, label(at)));
                        }
                    }
                }
            }
            // Annotate: what a click annotates, a measured section.
            let section = cursor
                .filter(|_| session.tool == Tool::Annotate && session.drag.is_none() && o.card.is_none())
                .filter(|&(x, y)| session.marker_at(screen, x, y).is_none())
                .and_then(|(x, y)| session.section_at(screen, x, y));
            if let Some(r) = section {
                prims.push(Prim::Fill(rect(r), color(style::ANNOTATION_BLUE, 0.12)));
                prims.push(Prim::Border(rect(r), color(style::ANNOTATION_BLUE, 1.0), 1.5, false));
            }
        }

        // A pin's number can be dragged (it moves what it's on).
        let on_marker = cursor.is_some_and(|(x, y)| session.marker_at(screen, x, y).is_some());
        let cursor_style = match session.tool {
            _ if session.moving_pin().is_some() => CursorStyle::ClosedHand,
            Tool::Annotate | Tool::Draw if on_marker && session.drag.is_none() => CursorStyle::OpenHand,
            Tool::Annotate => CursorStyle::Arrow,
            _ => CursorStyle::Crosshair,
        };
        // The pill (its own window, above) is on this screen: what goes with it is too.
        let pill_here = o.pill_rect.is_some_and(|(s, _)| s == screen);
        let card = o.card.as_ref().filter(|c| c.screen == screen).map(|c| self.card(o, c, (ox, oy), (ww, wh)));
        let menu = o.menu.as_ref().filter(|m| m.screen == screen).map(|m| self.menu(m, (ox, oy), (ww, wh), top));
        let notes = (pill_here && !session.hidden).then(|| self.notes(o, (ox, oy), wh)).flatten();
        let hidden_hint = (session.hidden && pill_here).then(|| {
            pill(
                &overlay,
                screen,
                (ox, oy),
                format!("Hidden ({} shows everything again)", o.keys.label(Cmd::Hide)),
                (ww / 2.0, top + 20.0),
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
            .children(ghosts)
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
            .children(notes)
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

/// How a toolbar button stands out: not at all, selected (a glass pill: the list open, a tool), or
/// the main action (asides' soft blue).
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
            Emphasis::Plain => color(0xffffff, 0.86),
            Emphasis::On => color(0xedebe7, 1.0),
            Emphasis::Primary => color(style::UI_INK, 1.0),
        }
    }
}

fn icon(name: &str, ink: Hsla) -> impl IntoElement {
    svg().path(format!("icons/{name}.svg")).size(px(15.)).text_color(ink)
}

/// The pill: the microphone, where the aside goes (a click lists the sessions and New session…), the
/// tool when it isn't Annotate (frozen), send and close. One window, frozen and live; it moves when
/// it's dragged anywhere but its buttons.
pub fn toolbar(overlay: &Entity<Overlay>, o: &Overlay, glide: &Glide) -> AnyElement {
    let session = &o.session;
    // Moving, the pointer runs over the buttons: they don't light up.
    let still = o.moving_toolbar.is_none();
    // An icon (or short text) button; its name shows by the pill while the pointer is on it.
    let button = |id: &'static str, name: String, emphasis: Emphasis, content: AnyElement, on: Box<OnPress>| {
        let (o1, o2, o3) = (overlay.clone(), overlay.clone(), overlay.clone());
        let name: SharedString = name.into();
        div()
            .id(id)
            // Where the aside goes: pointed at, the list opens, and stays while the pointer is on it.
            .when(id == "target", |d| {
                d.relative().child(measured(move |b, cx| o3.update(cx, |o, _| o.target_bounds = Some(b))))
            })
            .flex()
            .items_center()
            .justify_center()
            .gap(px(5.))
            .h(px(28.))
            .min_w(px(30.))
            .px(px(8.))
            .rounded(px(8.))
            .border_1()
            .border_color(color(0, 0.0))
            .text_size(px(12.5))
            .font_weight(FontWeight::MEDIUM)
            .text_color(emphasis.ink())
            .when(emphasis == Emphasis::On, |d| d.bg(color(0xffffff, 0.12)).border_color(color(0xffffff, 0.10)))
            .when(emphasis == Emphasis::Primary, |d| {
                d.bg(color(style::UI, 1.0)).when(still, |d| d.hover(|s| s.bg(color(style::UI_LIGHT, 1.0))))
            })
            .when(emphasis == Emphasis::Plain && still, |d| d.hover(|s| s.bg(color(0xffffff, 0.08))))
            .when(still, |d| d.cursor_pointer())
            .on_hover(move |hovered, _, cx| o1.update(cx, |o, cx| o.show_hint(id, name.clone(), *hovered, cx)))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                cx.stop_propagation();
                o2.update(cx, |o, cx| {
                    o.hint = None;
                    on(o, window, cx);
                    cx.notify();
                });
            })
            .child(content)
            .into_any_element()
    };
    // Room to grab the pill by, around the line.
    let sep = || div().w(px(1.)).h(px(18.)).mx(px(10.)).bg(color(0xffffff, 0.10)).into_any_element();
    // Buttons side by side as one control: the pointer goes from one to the next without leaving
    // them, and the name by the pill changes at once.
    let group = |buttons: Vec<AnyElement>| {
        div().flex().rounded(px(8.)).bg(color(0xffffff, 0.06)).children(buttons).into_any_element()
    };
    let plain = Emphasis::Plain.ink();
    let open = o.list.is_open();
    let target = if open { Emphasis::On } else { Emphasis::Plain };
    let (hover, toggle) = (overlay.clone(), overlay.clone());
    let help = o.help;
    // The keys at a glance: the pill's first button, one control with the microphone.
    let help_button = div()
        .id("help")
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(8.))
        .when(help, |d| d.bg(color(0xffffff, 0.12)))
        .when(still, |d| d.cursor_pointer().hover(|s| s.bg(color(0xffffff, 0.08))))
        .on_hover(move |hovered, _, cx| {
            let hovered = *hovered;
            hover.update(cx, |o, cx| {
                o.help_hover = hovered && o.moving_toolbar.is_none();
                if hovered && o.list_hovered {
                    o.show_list(false);
                    cx.notify();
                }
                if o.show_keys() {
                    cx.notify();
                }
            })
        })
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            toggle.update(cx, |o, cx| {
                o.help = !o.help;
                o.show_list(false);
                o.show_keys();
                cx.notify();
            });
        })
        .child(icon("help", if help { Emphasis::On.ink() } else { plain }))
        .into_any_element();
    // What a `Slot` shows, as wide as it's drawn (clipped while it grows, faded while it grows in or
    // shrinks away), noting the width it takes.
    let scale = o.pill_scale();
    let slot = |width: Option<f32>, shown: f32, content: Div, measure: fn(&mut Overlay, f32)| {
        let o4 = overlay.clone();
        div().flex().flex_none().when_some(width, |d, w| d.w(px(w)).overflow_hidden()).opacity(shown).child(
            content
                .relative()
                .flex_none()
                .child(measured(move |b, cx| o4.update(cx, |o, _| measure(o, f32::from(b.size.width))))),
        )
    };
    let first = group(vec![
        help_button,
        mic_button(overlay, o),
        button(
            "target",
            format!("Where it goes ({} switches)", o.keys.label(Cmd::Target)),
            target,
            slot(
                o.label.width(scale),
                o.label.shown(),
                div().max_w(px(240.)).truncate().children(o.label.shows().cloned()),
                |o, width| o.label.measured(width),
            )
            .into_any_element(),
            Box::new(|o, _, _| o.click_list()),
        ),
    ]);
    let badge = o.badge.shows().copied().map(|tool| {
        let back = if tool == Tool::Shot { session.previous_tool } else { Tool::Annotate };
        let name = format!(
            "{}: a click here, or {}, goes back to {}",
            tool.label(),
            o.keys.label(Cmd::Tool(tool)),
            back.label()
        );
        let badge = button(
            "tool",
            name,
            Emphasis::On,
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon(tool_icon(tool), Emphasis::On.ink()))
                .child(tool.label())
                .into_any_element(),
            // Shrinking away, it's already put down.
            Box::new(move |o, _, _| {
                if o.session.tool == tool {
                    o.session.toggle_tool(tool)
                }
            }),
        );
        slot(o.badge.width(scale), o.badge.shown(), div().flex().pl(px(PILL_GAP)).child(badge), |o, width| {
            o.badge.measured(width)
        })
    });
    // The tool goes by the first buttons, the gap before it its own, for it to grow in from
    // nothing.
    let mut items: Vec<AnyElement> = vec![div().flex().items_center().child(first).children(badge).into_any_element()];
    items.push(sep());
    items.push(group(vec![
        button(
            "done",
            format!("Send ({} or {})", o.spec.record_shortcut, o.keys.label(Cmd::Done)),
            Emphasis::Primary,
            icon("check", Emphasis::Primary.ink()).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Done, w, cx)),
        ),
        button(
            "close",
            "Close without sending".into(),
            Emphasis::Plain,
            icon("close", plain).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Close, w, cx)),
        ),
    ]));
    // Open, or still fading out; else the keys.
    let below = match (o.list.shown() > 0.0, o.keys_sheet.shown() > 0.0) {
        (true, _) => Some(targets_list(overlay, o, glide)),
        (false, true) => Some(keys_sheet(overlay, o)),
        _ => None,
    };
    toolbar_panel(overlay, o, items, below)
}

/// The keys at a glance, under the pill (or above it, as its list): what asides does, and the keys
/// for it.
fn keys_sheet(overlay: &Entity<Overlay>, o: &Overlay) -> AnyElement {
    let k = |c: Cmd| o.keys.label(c);
    let left: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "Pins",
            vec![
                ("Annotate (click, or drag)", k(Cmd::Tool(Tool::Annotate))),
                ("Draw (⇧ for an arrow)", k(Cmd::Tool(Tool::Draw))),
                ("Screenshot", k(Cmd::Tool(Tool::Shot))),
                ("Move it: drag a pin's number", String::new()),
                ("The last pin's type", "⌘1 ⌘4".into()),
            ],
        ),
        (
            "Measuring",
            vec![
                ("Measure", k(Cmd::Tool(Tool::Measure))),
                ("Hold a distance", format!("{} {}", k(Cmd::HoldH), k(Cmd::HoldV))),
                ("Guide", format!("{} {}", k(Cmd::GuideH), k(Cmd::GuideV))),
                ("Tolerance", format!("{} {}", k(Cmd::TolDown), k(Cmd::TolUp))),
                ("Points or pixels", k(Cmd::Units)),
            ],
        ),
    ];
    let right: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "Editing",
            vec![
                ("Undo", k(Cmd::Undo)),
                ("Clear", k(Cmd::Clear)),
                ("Hide everything", k(Cmd::Hide)),
                ("Copy as Markdown", k(Cmd::CopyMarkdown)),
            ],
        ),
        (
            "The aside",
            vec![
                ("Talk while muted (hold)", keys::label(&o.keys.talk)),
                ("Next session", k(Cmd::Target)),
                ("Live or frozen", o.spec.freeze_shortcut.clone()),
                ("Send", format!("{} {}", k(Cmd::Done), o.spec.record_shortcut)),
                ("Close", k(Cmd::Close)),
            ],
        ),
    ];
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
    let column = |groups: Vec<(&str, Vec<(&str, String)>)>| {
        div().flex_1().flex().flex_col().gap(px(10.)).children(groups.into_iter().map(|(name, rows)| {
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(color(0xffffff, 0.55))
                        .child(name.to_string()),
                )
                .children(rows.into_iter().map(|(label, keys)| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .text_size(px(12.))
                        .child(div().text_color(color(0xffffff, 0.85)).child(label.to_string()))
                        .child(
                            div()
                                .flex()
                                .gap(px(3.))
                                .children(keys.split(' ').filter(|k| !k.is_empty()).map(|k| cap(k.to_string()))),
                        )
                }))
        }))
    };
    let footer = match o.live {
        true => format!("Live, your apps have the keyboard: {} freezes the screen again.", o.spec.freeze_shortcut),
        false => "Click a pin's number to type about it, right-click for more.".into(),
    };
    let measure = overlay.clone();
    let shown = o.keys_sheet.shown();
    let slide = px((1.0 - shown) * if o.list_up { 6.0 } else { -6.0 });
    glass_panel()
        .id("keys")
        .top(slide)
        .opacity(shown)
        .w(px(KEYS_WIDTH as f32))
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .text_color(gpui::white())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .text_size(px(12.))
                .text_color(color(0xffffff, 0.7))
                .child("Click what's wrong or drag around it, say why, and send it."),
        )
        .child(div().flex().gap(px(22.)).child(column(left)).child(column(right)))
        .child(div().text_size(px(11.)).text_color(color(style::UI, 1.0)).child(footer))
        .child(measured(move |bounds, cx| measure.update(cx, |o, _| o.keys_bounds = Some(bounds))))
        .into_any_element()
}

fn tool_icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Measure => "measure",
        Tool::Annotate => "annotate",
        Tool::Draw => "draw",
        Tool::Shot => "shot",
    }
}

/// What a row of the pill's list does.
#[derive(Clone)]
enum Pick {
    Target(Target),
    ChooseFolder,
}

/// The pill's list, a menu under it: the sessions running (the ones asides can't type into, dimmed,
/// with why), a new session (the agent, then the folder, or Choose Folder…), and History only. Its
/// rows keep their height (`flex_none`): past `LIST_ROWS_HEIGHT` it scrolls (a truncated line can
/// shrink to nothing in a flex column, and with five sessions they overlapped).
fn targets_list(overlay: &Entity<Overlay>, o: &Overlay, glide: &Glide) -> AnyElement {
    let targets = &o.spec.targets;
    // Firmer than on a solid surface: what's behind shows through.
    let (primary, tertiary) = (color(0xedebe7, 1.0), color(0xffffff, 0.6));
    // A row: a check where it's chosen, then what it is; hovered, a glass pill.
    let row = |id: String, chosen: bool, enabled: bool, content: AnyElement, action: Option<Pick>| {
        let pick = overlay.clone();
        div()
            .id(SharedString::from(id))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_h(px(28.))
            .px(px(8.))
            .py(px(3.))
            .rounded(px(6.))
            .border_1()
            .border_color(color(0, 0.0))
            .when(enabled, |d| {
                d.cursor_pointer().hover(|s| s.bg(color(0xffffff, 0.10)).border_color(color(0xffffff, 0.09)))
            })
            .when(!enabled, |d| d.opacity(0.55))
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                cx.stop_propagation();
                match action.clone() {
                    Some(Pick::Target(target)) => pick.update(cx, |o, cx| {
                        o.choose_target(target);
                        cx.notify();
                    }),
                    Some(Pick::ChooseFolder) => pick.update(cx, |o, cx| {
                        o.show_list(false);
                        o.folder_asked = true;
                        cx.notify();
                    }),
                    None => {}
                }
            })
            .child(div().flex_none().w(px(14.)).when(chosen, |d| d.child(icon("check", primary))))
            .child(content)
            .into_any_element()
    };
    let lines = |title: SharedString, detail: Option<SharedString>| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .child(div().truncate().child(title))
            .children(detail.map(|d| div().text_size(px(11.)).text_color(tertiary).truncate().child(d)))
            .into_any_element()
    };
    let heading = |text: &str| {
        div()
            .flex_none()
            .px(px(8.))
            .pt(px(6.))
            .pb(px(3.))
            .text_size(px(11.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(tertiary)
            .child(text.to_string())
    };
    let hairline = || div().flex_none().h(px(1.)).mx(px(8.)).my(px(4.)).bg(color(0xffffff, 0.08)).into_any_element();
    let mut rows: Vec<AnyElement> = Vec::new();
    if !targets.sessions.is_empty() || targets.looking {
        rows.push(heading("Sessions").into_any_element());
    }
    for (i, s) in targets.sessions.iter().enumerate() {
        let target = Target::Session { id: s.id.clone() };
        let chosen = o.target.as_ref() == Some(&target);
        let detail = s.unreachable.clone().unwrap_or_else(|| s.app.clone());
        rows.push(row(
            format!("session-{i}"),
            chosen,
            s.unreachable.is_none(),
            lines(s.label.clone().into(), Some(detail.into())),
            s.unreachable.is_none().then_some(Pick::Target(target)),
        ));
    }
    if targets.looking {
        rows.push(row("looking".into(), false, false, lines("Looking for sessions…".into(), None), None));
    }
    if !targets.agents.is_empty() {
        if !rows.is_empty() {
            rows.push(hairline());
        }
        // The agent a new session starts: a segmented control beside the heading.
        // Its glass pill slides to the agent picked.
        let segments = targets.agents.iter().enumerate().map(|(i, &agent)| {
            let pick = overlay.clone();
            let on = agent == o.new_agent;
            div()
                .id(SharedString::from(format!("agent-{}", agent.program())))
                .relative()
                .child(glide.item(i))
                .h(px(20.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .border_1()
                .border_color(color(0, 0.0))
                .text_size(px(11.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(if on { primary } else { color(0xffffff, 0.68) })
                .when(!on, |d| d.cursor_pointer().hover(|s| s.text_color(primary)))
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    pick.update(cx, |o, cx| {
                        o.new_agent = agent;
                        cx.notify();
                    });
                })
                .child(agent.name())
        });
        rows.push(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .pr(px(4.))
                .child(heading("New session"))
                .child(
                    div()
                        .relative()
                        .flex()
                        .p(px(2.))
                        .gap(px(2.))
                        .rounded(px(7.))
                        .bg(color(0xffffff, 0.06))
                        .child(glide.layer(
                            targets.agents.iter().position(|&a| a == o.new_agent),
                            color(0xffffff, 0.12),
                            color(0xffffff, 0.10),
                            5.,
                        ))
                        .children(segments),
                )
                .into_any_element(),
        );
        // A folder chosen in the panel isn't among the app's: it leads.
        let picked = match &o.target {
            Some(Target::New { dir, .. }) if !targets.folders.iter().any(|f| &f.path == dir) => Some(dir.clone()),
            _ => None,
        };
        let folders = picked
            .iter()
            .map(|dir| (dir.clone(), dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()))
            .chain(targets.folders.iter().map(|f| (f.path.clone(), f.name.clone())))
            .take(LIST_FOLDERS);
        for (i, (dir, name)) in folders.enumerate() {
            let path = asides_core::paths::tilde(&dir);
            let target = Target::New { agent: o.new_agent, dir };
            let chosen = o.target.as_ref() == Some(&target);
            rows.push(row(
                format!("folder-{i}"),
                chosen,
                true,
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(icon("folder", tertiary))
                    .child(div().flex_none().child(name))
                    .child(div().min_w_0().text_size(px(11.)).text_color(tertiary).truncate().child(path))
                    .into_any_element(),
                Some(Pick::Target(target)),
            ));
        }
        rows.push(row(
            "choose-folder".into(),
            false,
            !o.folder_panel,
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(icon("plus", tertiary))
                .child("Choose Folder…")
                .into_any_element(),
            (!o.folder_panel).then_some(Pick::ChooseFolder),
        ));
    }
    rows.push(hairline());
    let history = Target::History;
    rows.push(row(
        "history".into(),
        o.target.as_ref() == Some(&history),
        true,
        lines("History only".into(), Some("Send it later from the asides window".into())),
        Some(Pick::Target(history)),
    ));
    let measure = overlay.clone();
    let shown = o.list.shown();
    let slide = px((1.0 - shown) * if o.list_up { 6.0 } else { -6.0 });
    glass_panel()
        .id("targets")
        .top(slide)
        .opacity(shown)
        .w(px(LIST_WIDTH as f32))
        .text_size(px(13.))
        .text_color(primary)
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            div()
                .id("targets-rows")
                .max_h(px(LIST_ROWS_HEIGHT as f32))
                .overflow_y_scroll()
                .track_scroll(&o.list_scroll)
                .p(px(4.))
                .flex()
                .flex_col()
                .children(rows),
        )
        .child(crate::controls::scrollbar("targets-bar", &o.list_scroll, crate::controls::Theme::dark()))
        .child(measured(move |bounds, cx| measure.update(cx, |o, _| o.list_bounds = Some(bounds))))
        .into_any_element()
}

/// A surface of the pill (the pill, its list): rounded, the glass behind it (macOS blurs what's
/// behind the window there, `native::set_glass`) under a tint, a hairline around it. No shadow:
/// GPUI fills one under the whole surface too, between the blur and the tint, and it darkened the
/// glass to near black.
fn glass_panel() -> Div {
    div()
        .relative()
        .occlude()
        .rounded(px(12.))
        .border_1()
        .border_color(color(0xffffff, style::PANEL_STROKE))
        .bg(color(style::GLASS, style::GLASS_ALPHA))
}

/// A surface on a frozen screen (a window `offset` points into screen `screen`), glass like the
/// pill: the screen behind it, blurred (`glass::frozen`), under the pill's tint, a hairline around
/// it. What's added to it goes on top.
fn frozen_glass(overlay: &Entity<Overlay>, screen: usize, offset: (f64, f64), radius: f32) -> Div {
    // The tint is a layer of its own, over the blur (a child draws over its parent's background).
    let tint = div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(radius - 1.))
        .bg(color(style::GLASS, style::GLASS_ALPHA));
    div()
        .relative()
        .rounded(px(radius))
        .border_1()
        .border_color(color(0xffffff, style::PANEL_STROKE))
        .child(glass::frozen(overlay, screen, offset, radius - 1.))
        .child(tint)
}

/// An element's bounds in its window, as drawn.
fn measured(store: impl Fn(Bounds<Pixels>, &mut App) + 'static) -> impl IntoElement {
    canvas(move |bounds, _, cx| store(bounds, cx), |_, _, _, _| {}).absolute().top_0().left_0().size_full()
}

/// The pill around its buttons, and its list under it when it's open (or above it, to its left:
/// `Overlay::list_left`, `list_up`). It notes where they're drawn, for its window to fit them and
/// macOS to blur behind them, and moves when it's dragged.
fn toolbar_panel(
    overlay: &Entity<Overlay>,
    o: &Overlay,
    items: Vec<AnyElement>,
    list: Option<AnyElement>,
) -> AnyElement {
    let (measure, measure_pill, grab) = (overlay.clone(), overlay.clone(), overlay.clone());
    div()
        .relative()
        .flex_none()
        .flex()
        .flex_col()
        .when(o.list_up, |d| d.flex_col_reverse())
        .when(o.list_left, |d| d.items_end())
        .when(!o.list_left, |d| d.items_start())
        .gap(px(LIST_GAP as f32))
        .font_family(".SystemUIFont")
        .child(
            glass_panel()
                .id("toolbar")
                .cursor(if o.moving_toolbar.is_some() { CursorStyle::ClosedHand } else { CursorStyle::OpenHand })
                .on_mouse_down(MouseButton::Left, move |e, _, cx| {
                    cx.stop_propagation();
                    grab.update(cx, |o, cx| {
                        o.grab_toolbar(e.position);
                        cx.notify();
                    });
                })
                // Grabbed anywhere but its buttons; they fill it, their corners inside its own.
                .child(div().flex().items_center().gap(px(PILL_GAP)).p(px(4.)).children(items))
                .child(measured(move |bounds, cx| measure_pill.update(cx, |o, _| o.drawn_pill(bounds)))),
        )
        .children(list)
        .child(measured(move |bounds, cx| {
            if measure.update(cx, |o, _| o.drawn_toolbar(bounds)) {
                follow_soon(&measure, cx);
            }
        }))
        .child(dragged(overlay))
        .into_any_element()
}

/// The pill's window follows the pill at once (`windows::follow_now`), unless it's about to.
fn follow_soon(overlay: &Entity<Overlay>, cx: &mut App) {
    if overlay.update(cx, |o, _| !std::mem::replace(&mut o.following, true)) {
        let overlay = overlay.clone();
        cx.spawn(async move |cx| crate::windows::follow_now(&overlay, cx)).detach();
    }
}

/// Grabbed, the pill moves with each move of the mouse, and stays where it's let go (macOS sends
/// them to the window it was grabbed in, wherever the pointer is).
fn dragged(overlay: &Entity<Overlay>) -> impl IntoElement {
    let overlay = overlay.clone();
    canvas(
        |_, _, _| (),
        move |_, _, window, _| {
            let (moved, let_go) = (overlay.clone(), overlay.clone());
            window.on_mouse_event(move |_: &MouseMoveEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && moved.read(cx).moving_toolbar.is_some() {
                    follow_soon(&moved, cx);
                }
            });
            window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && let_go.read(cx).moving_toolbar.is_some() {
                    let_go.update(cx, |o, _| o.let_go = true);
                    follow_soon(&let_go, cx);
                }
            });
        },
    )
    .absolute()
    .size_0()
}

/// The pill's microphone: listening (a soft red, filling with how loud it hears) or not (crossed
/// out); a click turns it on or off, and without a voice model, gets one. Always the same size.
fn mic_button(overlay: &Entity<Overlay>, o: &Overlay) -> AnyElement {
    let listening = o.listening();
    let (press, tip) = (overlay.clone(), overlay.clone());
    let name: SharedString = match (o.spec.model.ready, listening) {
        (false, _) => format!("Download the voice model ({})", o.spec.model.label).into(),
        (true, true) => "Listening (click to mute)".into(),
        (true, false) => format!("Muted (click to listen, or hold {})", keys::label(&o.keys.talk)).into(),
    };
    let red = color(style::RECORDING_SOFT, 1.0);
    let ink = if listening { gpui::white() } else { Emphasis::Plain.ink() };
    div()
        .id("mic")
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(8.))
        .when(o.moving_toolbar.is_none(), |d| d.cursor_pointer())
        .when(listening, |d| d.bg(red.opacity(0.32)))
        .when(!listening && o.moving_toolbar.is_none(), |d| d.hover(|s| s.bg(color(0xffffff, 0.08))))
        .on_hover(move |hovered, _, cx| tip.update(cx, |o, cx| o.show_hint("mic", name.clone(), *hovered, cx)))
        .on_mouse_down(MouseButton::Left, move |_, _, cx| {
            cx.stop_propagation();
            press.update(cx, |o, cx| {
                o.hint = None;
                o.mic();
                cx.notify();
            });
        })
        // The fill: the button's whole shape, cut at the level (a short one rounded at its own
        // height overflowed the corners).
        .when(listening, |d| {
            d.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w_full()
                    .h(gpui::relative(o.level.now().clamp(0.0, 1.0)))
                    .overflow_hidden()
                    .child(div().absolute().bottom_0().left_0().size(px(28.)).rounded(px(8.)).bg(red)),
            )
        })
        .child(icon(if listening { "mic" } else { "mic-off" }, ink))
        .into_any_element()
}

/// An annotation's card's microphone: tap to start or stop, hold to talk about the annotation. Red
/// while recording.
fn talk_button(overlay: &Entity<Overlay>, o: &Overlay, id: &str, timer: bool) -> AnyElement {
    let recording = o.recording_seconds();
    let (t1, t2, t3) = (overlay.clone(), overlay.clone(), overlay.clone());
    let name: SharedString = "Say what should change: hold, or tap to start and stop".into();
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
        .on_hover(move |hovered, _, cx| {
            let name = name.clone();
            t3.update(cx, |o, cx| {
                o.card_tip = hovered.then_some(name);
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

/// The pill's window, above the frozen screens and over the live one.
pub struct ToolbarView {
    overlay: Entity<Overlay>,
    /// The glass pill behind the list's agent picked.
    glide: Glide,
}

impl ToolbarView {
    pub fn new(overlay: Entity<Overlay>, cx: &mut Context<Self>) -> ToolbarView {
        cx.observe(&overlay, |_, _, cx| cx.notify()).detach();
        ToolbarView { overlay, glide: Glide::new(cx) }
    }
}

/// Room around the pill in its window, for its edges' antialiasing.
pub const TOOLBAR_MARGIN: f64 = 1.0;
/// Between the pill's buttons.
const PILL_GAP: f32 = 2.0;
/// The pill's list: its width, the most its rows take before they scroll, and the gap above it.
pub const LIST_WIDTH: f64 = 360.0;
/// The keys under the pill: their width.
pub const KEYS_WIDTH: f64 = 460.0;
pub const LIST_ROWS_HEIGHT: f64 = 440.0;
pub const LIST_GAP: f64 = 6.0;
/// The folders a new session can start in, at most.
const LIST_FOLDERS: usize = 5;

impl Render for ToolbarView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let o = self.overlay.read(cx);
        // Fading or growing, it's drawn every frame.
        if o.list.moving() || o.keys_sheet.moving() || !o.level.settled() || o.pill_moving() {
            window.request_animation_frame();
        }
        // As big as the pill and its list: its window fits them. Until it does, the pill stays in
        // the corner the window grows from.
        div()
            .size_full()
            .flex()
            .p(px(TOOLBAR_MARGIN as f32))
            .when(o.list_left, |d| d.justify_end())
            .when(o.list_up, |d| d.items_end())
            .when(!o.list_up, |d| d.items_start())
            .child(toolbar(&self.overlay, o, &self.glide))
    }
}

impl ScreenView {
    /// What goes with the pill, by it (in its own window, above): a toast, the name of the button
    /// under the pointer, the keys. Under it, or above it in the lower half of the screen.
    fn notes(&self, o: &Overlay, (ox, oy): (f64, f64), wh: f64) -> Option<AnyElement> {
        // Moving, they'd trail behind the pill (another window); drawn at a new size, they wait for
        // its window to fit it, and a new toast for the pill to be drawn as it is now (the last one
        // shows until then).
        let (_, pill) = o.pill_rect.filter(|_| o.moving_toolbar.is_none() && o.pill_fits())?;
        let toast = o.toast_shown.clone();
        let tip = (!o.keys_sheet.is_open() && !o.list.is_open())
            .then(|| o.hint.as_ref().map(|(_, name)| name.clone()))
            .flatten();
        let small = |text: SharedString| {
            frozen_glass(&self.overlay, self.screen, (ox, oy), 7.)
                .px(px(9.))
                .py(px(5.))
                .font_family(".SystemUIFont")
                .text_color(color(0xffffff, 0.86))
                .text_size(px(12.))
                .child(text)
        };
        let (x, y) = (pill.x - ox, pill.y - oy);
        let notes = div()
            .absolute()
            .left(px((x + pill.width / 2.0 - 320.0) as f32))
            .w(px(640.))
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            // The tip keeps its place (and doesn't fade again) as it names the next button.
            .children(toast.map(|t| fade_in(small(t), "toast", 0.)))
            .children(tip.map(|t| fade_in(small(t), "tip", 0.)));
        let notes = match y + pill.height / 2.0 < wh / 2.0 {
            true => notes.top(px((y + pill.height + 8.0) as f32)).justify_start(),
            false => notes.bottom(px((wh - y + 8.0) as f32)).justify_end(),
        };
        Some(notes.into_any_element())
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
                (false, Some(status)) => format!("“{said}” ({status})"),
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
        // Draw and Measure, a click away: in the card, their keys type.
        let tools = [Tool::Draw, Tool::Measure].map(|tool| {
            let (pick, tip) = (self.overlay.clone(), self.overlay.clone());
            let on = o.session.tool == tool;
            let name: SharedString = format!("{} ({})", tool.label(), o.keys.label(Cmd::Tool(tool))).into();
            div()
                .id(SharedString::from(format!("card-{}", tool_icon(tool))))
                .flex()
                .items_center()
                .justify_center()
                .size(px(28.))
                .rounded(px(7.))
                .cursor_pointer()
                .when(on, |d| d.bg(color(0xffffff, 0.14)))
                .when(!on, |d| d.hover(|s| s.bg(color(0xffffff, 0.12))))
                .on_hover(move |hovered, _, cx| {
                    let name = name.clone();
                    tip.update(cx, |o, cx| {
                        o.card_tip = hovered.then_some(name);
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    pick.update(cx, |o, cx| {
                        o.card_tip = None;
                        o.close_card(window, cx);
                        o.session.set_tool(tool);
                        cx.notify();
                    });
                })
                .child(icon(tool_icon(tool), gpui::white()))
        });
        let surface = frozen_glass(&self.overlay, card.screen, (ox, oy), 12.)
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
            .text_color(gpui::white())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap(px(6.))
                    .child(div().flex_1().child(card.input.clone()))
                    .child(div().flex().items_center().h(px(30.)).gap(px(1.)).children(dots))
                    .child(div().flex().items_center().h(px(30.)).gap(px(1.)).children(tools))
                    .child(talk_button(&self.overlay, o, "card-talk", false))
                    .children(trash),
            )
            .children(said_row)
            .children(o.card_tip.clone().map(|tip| {
                frozen_glass(&self.overlay, card.screen, (ox, oy), 6.)
                    .absolute()
                    .bottom(px(-26.))
                    .right(px(30.))
                    .px(px(7.))
                    .py(px(2.))
                    .text_size(px(11.))
                    .child(tip)
            }));
        // Opening, it drops into place.
        fade_in(surface, SharedString::from(format!("card-{}", card.key)), -4.).into_any_element()
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
                .border_1()
                .border_color(color(0, 0.0))
                .text_size(px(13.))
                .cursor_pointer()
                .when(lit, |d| d.bg(color(0xffffff, 0.10)).border_color(color(0xffffff, 0.09)))
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
        let surface = frozen_glass(&self.overlay, menu.screen, (ox, oy), 12.)
            .id("menu")
            .occlude()
            .absolute()
            .left(px(x as f32))
            .top(px(y as f32))
            .w(px(MENU_WIDTH as f32))
            .p(px(4.))
            .font_family(".SystemUIFont")
            .text_color(color(0xedebe7, 1.0))
            .children(rows);
        fade_in(surface, SharedString::from(format!("menu-{}-{}", menu.at.0, menu.at.1)), -4.).into_any_element()
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

/// What wakes the clock before its tick (`wake_clock`).
static WAKE: std::sync::OnceLock<futures::channel::mpsc::UnboundedSender<()>> = std::sync::OnceLock::new();

/// Something the clock looks at happened: the overlay changed, a command came from the app, or
/// macOS says a window showed or hid, an app came to the front, the screens changed or the pointer
/// moved (`native::on_system_changes`).
pub fn wake_clock() {
    if let Some(wake) = WAKE.get() {
        let _ = wake.unbounded_send(());
    }
}

/// The clock: the app's commands and what changes with time, then the windows. It runs when
/// something happened (`wake_clock`) and when something is due (`Overlay::due`,
/// `windows::deadline`), never on a timer of its own.
pub fn run_clock(overlay: &Entity<Overlay>, cx: &mut App) {
    let (wake, mut woken) = futures::channel::mpsc::unbounded();
    let _ = WAKE.set(wake);
    crate::native::on_system_changes(wake_clock);
    // What changes the overlay may change its windows.
    cx.observe(overlay, |_, _| wake_clock()).detach();
    // It keeps the overlay as long as the app runs (its windows open from here).
    let overlay = overlay.clone();
    cx.spawn(async move |cx| {
        loop {
            if overlay.update(cx, |o, cx| o.tick(cx)).is_err() {
                break;
            }
            if let Err(err) = crate::windows::sync(&overlay, cx) {
                eprintln!("asides-overlay: {err:#}");
                break;
            }
            crate::windows::ask_folder(&overlay, cx);
            // Only what's still to come: what was due and didn't change waits for something else.
            let now = Instant::now();
            let due = cx
                .update(|cx| {
                    let o = overlay.read(cx);
                    [o.due(), crate::windows::deadline(o)].into_iter().flatten().filter(|at| *at > now).min()
                })
                .ok()
                .flatten();
            let woke = match due {
                Some(at) => {
                    let due = cx.background_executor().timer(at - now);
                    futures::future::select(due, woken.next()).await;
                    true
                }
                None => woken.next().await.is_some(),
            };
            if !woke {
                break;
            }
            while let Ok(()) = woken.try_recv() {}
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
