//! The overlay's windows: one per screen, borderless, above everything, showing the frozen screen
//! and what's on it, with one toolbar for the four tools (Measure, Mark, Draw, Shot), the
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
use asides_core::helper::{Command, Event, Spec};
use asides_core::inspect::{Axis, Element, Intent, Measurement, PtRect, Severity, Shape, Units, num};
use asides_core::keys::{self, Chord, Mods};
use asides_core::measure::{Frame, Tolerance};
use asides_core::overlay::{MARKER, Part, Placed, Session, Tool, lookups};
use gpui::{
    AnyElement, AnyWindowHandle, App, BorderStyle, Bounds, Context, CursorStyle, DispatchPhase, Entity, FocusHandle,
    Focusable, FontWeight, Hitbox, HitboxBehavior, Hsla, ImageSource, KeyDownEvent, KeyUpEvent, Modifiers,
    ModifiersChangedEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PathBuilder, Pixels, Point,
    Render, RenderImage, Rgba, SharedString, Window, canvas, deferred, div, img, point, prelude::*, px, quad, size,
    svg, transparent_black,
};

use crate::export::{self, Exporter};
use crate::input::TextInput;
use crate::{finish, io, style};

/// How often the overlay looks at the app's commands, the clock and the hover.
const TICK: Duration = Duration::from_millis(33);
/// Mark tool: how long the cursor rests before the element under it is looked up.
const HOVER_REST: Duration = Duration::from_millis(90);
const TOAST: Duration = Duration::from_millis(1600);
const CARD_WIDTH: f64 = 320.0;
const CARD_HEIGHT: f64 = 176.0;
const MENU_WIDTH: f64 = 260.0;

/// A screen's window: where it is and what part of the screen it shows (all of it, unless
/// windowed for development).
pub struct ScreenWindow {
    pub handle: Option<AnyWindowHandle>,
    pub focus: Option<FocusHandle>,
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
    Done,
    DoneWhole,
    Send,
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
                (p(&k.mark), Cmd::Tool(Tool::Mark)),
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
                (p(&k.send), Cmd::Send),
                (p(&k.done_whole_screen), Cmd::DoneWhole),
                (p(&k.done), Cmd::Done),
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
    input: Entity<TextInput>,
    intent: Option<Intent>,
    severity: Option<Severity>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum MenuItem {
    Cmd(Cmd),
    Tool(Tool),
    RemoveMark(usize),
    RemoveHeld(usize),
    ShotOfHeld(usize),
    EditMark(usize),
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
    Mark { screen: usize, index: usize, number: u32, part: Part },
    Hover { screen: usize },
}

struct Hovered {
    screen: usize,
    frame: PtRect,
    name: String,
}

pub struct Overlay {
    pub session: Session,
    spec: Spec,
    keys: Keys,
    accent: u32,
    images: Vec<Arc<RenderImage>>,
    frames: Vec<Frame>,
    exporter: Option<Exporter>,
    pub windows: Vec<ScreenWindow>,
    dir: PathBuf,
    opened: Instant,
    card: Option<Card>,
    menu: Option<Menu>,
    toast: Option<(SharedString, Instant)>,
    help: bool,
    /// Seconds recorded when the app last said so, and when it did.
    recording: Option<(f32, Instant)>,
    shown_seconds: u32,
    /// Where the cursor last moved and when, for the Mark tool's hover lookup.
    rested: Option<(usize, f64, f64, Instant)>,
    hover_asked: Option<(usize, f64, f64)>,
    hovered: Option<Hovered>,
    pending: HashMap<u64, Lookup>,
    next_id: u64,
    talking: bool,
    /// A toolbar button's description while the pointer is on it.
    hint: Option<SharedString>,
    /// Screenshots taken: when, and the file in the session directory.
    shots: Vec<(f32, String)>,
    commands: Receiver<Command>,
    done: bool,
}

impl Overlay {
    pub fn new(spec: Spec, frames: Vec<Frame>, dir: PathBuf, commands: Receiver<Command>) -> Overlay {
        let screens =
            spec.screens.iter().zip(&frames).map(|(s, f)| Session::screen(s.screen.clone(), f.clone())).collect();
        let mut session = Session::new(screens, Tolerance(spec.tolerance.min(100)), spec.units);
        session.tool = Tool::Measure;
        let images = frames.iter().map(render_image).collect();
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
            help: spec.tips,
            recording,
            shown_seconds: 0,
            rested: None,
            hover_asked: None,
            hovered: None,
            pending: HashMap::new(),
            next_id: 1,
            talking: false,
            hint: None,
            shots: Vec::new(),
            commands,
            done: false,
            spec,
        }
    }

    pub fn start(&mut self) {
        self.opened = Instant::now();
    }

    fn elapsed(&self) -> f32 {
        self.opened.elapsed().as_secs_f32()
    }

    fn exporter(&mut self) -> &Exporter {
        if self.exporter.is_none() {
            let frames: Vec<&Frame> = self.frames.iter().collect();
            self.exporter = Some(Exporter::new(&frames, style::parse_color(&self.spec.color)));
        }
        self.exporter.as_ref().unwrap()
    }

    fn cursor_screen(&self) -> usize {
        self.session.cursor.map_or(0, |c| c.0)
    }

    fn toast(&mut self, text: impl Into<SharedString>) {
        self.toast = Some((text.into(), Instant::now()));
    }

    fn set_modifiers(&mut self, m: &Modifiers) {
        self.session.crosshair = m.shift;
        self.session.snapping = !m.platform;
        self.session.alternate = m.alt;
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
        if self.done {
            return;
        }
        self.set_modifiers(&e.modifiers);
        if self.menu.take().is_some() || self.help {
            self.help = false;
            return cx.notify();
        }
        if self.card.is_some() {
            self.close_card(true, window, cx);
            return cx.notify();
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
        self.rested = Some((screen, x, y, Instant::now()));
        if self.hovered.as_ref().is_some_and(|h| h.screen != screen || !h.frame.contains(x, y)) {
            self.hovered = None;
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
        if self.done || self.session.drag.is_none() {
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
                // While the user talks, a mark is enough: what they say goes with it. Drawings
                // come in quick strokes: their card opens on a click on their number.
                if self.recording.is_none() && self.session.tool == Tool::Mark {
                    self.open_card(screen, index, window, cx);
                }
            }
            Placed::Edit { screen, index } => self.open_card(screen, index, window, cx),
            Placed::Shot { screen, rect } => self.take_shot(screen, rect),
            Placed::Measurement { .. } | Placed::Nothing => {}
        }
    }

    /// Asks the app what a new mark is on.
    fn look_up(&mut self, screen: usize, index: usize) {
        let Some(a) = self.session.screens[screen].annotations.get(index) else { return };
        let (ox, oy) = self.spec.screens[screen].origin;
        for (part, x, y) in lookups(a) {
            let id = self.next_id;
            self.next_id += 1;
            self.pending.insert(id, Lookup::Mark { screen, index, number: a.number, part });
            io::emit(&Event::Identify { id, x: ox + x, y: oy + y });
        }
    }

    fn take_shot(&mut self, screen: usize, rect: PtRect) {
        let time = self.elapsed();
        let png = {
            let session = self.session.clone();
            self.exporter().render(&session, screen, rect)
        };
        let Some(png) = png else { return };
        let name = format!("shot-{}.png", self.shots.len() + 1);
        if std::fs::write(self.dir.join(&name), &png).is_ok() {
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
        if let Some(index) = self.session.marker_at(screen, x, y) {
            let number = self.session.screens[screen].annotations[index].number;
            items.push((format!("Edit Mark {number}"), String::new(), MenuItem::EditMark(index)));
            items.push((format!("Remove Mark {number}"), String::new(), MenuItem::RemoveMark(index)));
        }
        if let Some(index) = self.session.held_at(screen, x, y) {
            items.push(("Screenshot of This".into(), String::new(), MenuItem::ShotOfHeld(index)));
            items.push(("Remove This Measurement".into(), String::new(), MenuItem::RemoveHeld(index)));
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
        // Guides and holds go where the menu was opened.
        self.session.move_to(screen, x, y);
        match item {
            MenuItem::Cmd(cmd) => self.command(cmd, window, cx),
            MenuItem::Tool(tool) => self.session.set_tool(tool),
            MenuItem::RemoveMark(index) => self.session.remove_annotation(screen, index),
            MenuItem::RemoveHeld(index) => self.session.remove_held(screen, index),
            MenuItem::EditMark(index) => self.open_card(screen, index, window, cx),
            MenuItem::ShotOfHeld(index) => self.shot_of_held(screen, index),
        }
        cx.notify();
    }

    fn shot_of_held(&mut self, screen: usize, index: usize) {
        let padding = self.spec.padding.clamp(0.0, 24.0);
        if let Placed::Shot { screen, rect } = self.session.shot_of_held(screen, index, padding, self.elapsed()) {
            self.take_shot(screen, rect);
        }
    }

    // --- The card (a mark's typed feedback) ---

    fn open_card(&mut self, screen: usize, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(a) = self.session.screens[screen].annotations.get(index) else { return };
        let (comment, number, intent, severity) = (a.comment.clone(), a.number, a.intent, a.severity);
        let input = cx.new(|cx| TextInput::new(&comment, "What should change? Type it, or just say it", cx));
        // The card opens where the click was: `window` is that screen's.
        window.focus(&input.read(cx).focus_handle(cx));
        self.card = Some(Card { screen, index, number, input, intent, severity });
    }

    fn close_card(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(card) = self.card.take() else { return };
        let still_there =
            self.session.screens[card.screen].annotations.get(card.index).is_some_and(|a| a.number == card.number);
        if save && still_there {
            let text = card.input.read(cx).text();
            self.session.set_feedback(card.screen, card.index, &text, card.intent, card.severity);
        }
        if let Some(focus) = self.windows.get(card.screen).and_then(|w| w.focus.clone()) {
            window.focus(&focus);
        }
    }

    fn delete_card_mark(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(card) = &self.card {
            let (screen, index) = (card.screen, card.index);
            self.close_card(false, window, cx);
            self.session.remove_annotation(screen, index);
        }
    }

    // --- Keys ---

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        let key = keys::normalize(&e.keystroke.key);
        let m = &e.keystroke.modifiers;
        let mods = Mods { cmd: m.platform, shift: m.shift, alt: m.alt, ctrl: m.control };
        if self.card.is_some() {
            match key.as_str() {
                "escape" => self.close_card(false, window, cx),
                "enter" if !m.shift => self.close_card(true, window, cx),
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
            if !e.is_held && !self.talking {
                self.talking = true;
                io::emit(&Event::Talk { down: true });
            }
            return;
        }
        if let Some(delta) = arrow(&key) {
            let step = if m.shift { 10.0 } else { 1.0 / self.session.screens[self.cursor_screen()].screen.scale };
            self.session.nudge_guide(delta * step);
            return cx.notify();
        }
        if let Some(cmd) = self.keys.command(&key, mods) {
            self.help = self.help && cmd == Cmd::Help;
            self.command(cmd, window, cx);
            cx.notify();
        }
    }

    fn key_up(&mut self, e: &KeyUpEvent, cx: &mut Context<Self>) {
        let key = keys::normalize(&e.keystroke.key);
        if key == "space" {
            self.session.moving = false;
        }
        if self.talking && self.keys.talk.iter().any(|c| c.key == key) {
            self.talking = false;
            io::emit(&Event::Talk { down: false });
        }
        cx.notify();
    }

    fn command(&mut self, cmd: Cmd, window: &mut Window, cx: &mut Context<Self>) {
        let s = &mut self.session;
        match cmd {
            Cmd::Tool(tool) => {
                s.set_tool(tool);
                self.hovered = None;
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
            Cmd::Undo => drop(s.undo()),
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
            Cmd::Done => self.finish(false, false, window, cx),
            Cmd::DoneWhole => self.finish(true, false, window, cx),
            // Without batches, agents get the aside once it's written anyway.
            Cmd::Send => self.finish(false, self.spec.batch, window, cx),
            Cmd::Close => self.cancel(cx),
        }
    }

    fn talk_button(&mut self, down: bool) {
        if down != self.talking {
            self.talking = down;
            io::emit(&Event::Talk { down });
        }
    }

    // --- Finishing ---

    fn finish(&mut self, whole: bool, send: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.close_card(true, window, cx);
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
            io::emit(&Event::Talk { down: false });
        }
        let screen = self.cursor_screen();
        let session = self.session.clone();
        let (spec, dir, shots) = (self.spec.clone(), self.dir.clone(), self.shots.clone());
        let outcome = finish::outcome(&session, self.exporter(), &spec, &dir, whole, &shots, screen, send);
        match outcome.and_then(|o| finish::write(&dir, &o)) {
            Ok(()) => io::emit(&Event::Finished),
            Err(err) => {
                eprintln!("could not write the result: {err:#}");
                io::emit(&Event::Cancelled { saved: self.session.saved() });
            }
        }
        cx.quit();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.done {
            return;
        }
        self.done = true;
        if self.talking {
            io::emit(&Event::Talk { down: false });
        }
        io::emit(&Event::Cancelled { saved: self.session.saved() });
        cx.quit();
    }

    // --- The clock: the app's commands, toasts, the recording's time, the hover lookup ---

    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        while let Ok(command) = self.commands.try_recv() {
            changed = true;
            self.apply_command(command, cx);
        }
        if self.toast.as_ref().is_some_and(|(_, at)| at.elapsed() > TOAST) {
            self.toast = None;
            changed = true;
        }
        let seconds = self.recording.map_or(0, |(s, at)| (s + at.elapsed().as_secs_f32()) as u32);
        if seconds != self.shown_seconds {
            self.shown_seconds = seconds;
            changed = true;
        }
        self.hover_lookup();
        if changed {
            cx.notify();
        }
    }

    pub fn apply_command(&mut self, command: Command, cx: &mut Context<Self>) {
        match command {
            Command::Element { id, element } => self.identified(id, element.map(|e| *e)),
            Command::Recording { seconds } => self.recording = seconds.map(|s| (s, Instant::now())),
            Command::Finish => self.finish_now(false, false, cx),
            Command::Close => self.cancel(cx),
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

    /// Mark tool, cursor at rest: ask what's under it, to outline it before the click.
    fn hover_lookup(&mut self) {
        if !self.spec.identify || self.session.tool != Tool::Mark || self.session.drag.is_some() || self.card.is_some()
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
            Lookup::Mark { screen, .. } | Lookup::Hover { screen } => screen,
        };
        let (ox, oy) = self.spec.screens[screen].origin;
        element.frame = element.frame.map(|f| PtRect::new(f.x - ox, f.y - oy, f.width, f.height));
        match lookup {
            Lookup::Mark { screen, index, number, part } => {
                let same = self.session.screens[screen].annotations.get(index).is_some_and(|a| a.number == number);
                if same {
                    self.session.set_element(screen, index, part, element);
                }
            }
            Lookup::Hover { screen } => {
                self.hovered = match (element.frame, element.name) {
                    (Some(frame), Some(name)) => Some(Hovered { screen, frame, name }),
                    _ => None,
                };
            }
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

/// A frame as GPUI wants it: BGRA.
fn render_image(frame: &Frame) -> Arc<RenderImage> {
    let mut bgra = frame.rgba.clone();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
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
        let (scale, units) = (state.screen.scale, o.session.units);
        let n = |v: f64| num(v, scale, units);
        let accent = o.accent;
        let thin = (1.0 / window.scale_factor() as f64).max(0.5);
        let to = |(x, y): (f64, f64)| (x - ox, y - oy);
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
                        labels.push(pill(n(gap), to(at)));
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
                            labels.push(pill(n(across.len()), to(((across.start + across.end) / 2.0, ly))));
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
                            labels.push(pill(text, to((lx, (down.start + down.end) / 2.0))));
                        }
                    }
                    Measurement::Area { rect: r, .. } => {
                        prims.push(Prim::Fill(rect(*r), color(accent, style::AREA_TINT)));
                        prims.push(Prim::Border(rect(*r), accent_c, thin as f32, false));
                        let text = format!("{} × {}", n(r.width), n(r.height));
                        let at = style::area_label(*r, pill_width(&text), sh);
                        labels.push(pill(text, to(at)));
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

            // Marks.
            for a in &state.annotations {
                let (r, g, b, alpha) = style::mark_color(a, accent);
                let mark = Hsla::from(Rgba { r: r as f32, g: g as f32, b: b as f32, a: 1.0 });
                match (&a.shape, a.area) {
                    (Some(Shape::Sketch { points }), _) => {
                        prims.push(Prim::Stroke(points.iter().map(|p| to(*p)).collect(), style::DRAW_WIDTH, mark));
                    }
                    (Some(Shape::Arrow { from, to: end }), _) => arrow_prims(&mut prims, to(*from), to(*end), mark),
                    (None, Some(area)) => {
                        prims.push(Prim::Fill(rect(area), Hsla { a: alpha, ..mark }));
                        prims.push(Prim::Border(rect(area), mark, 1.5, true));
                    }
                    (None, None) => {
                        if let Some(frame) = a.element.as_ref().and_then(|e| e.frame) {
                            prims.push(Prim::Border(rect(frame), Hsla { a: 0.9, ..mark }, 1.5, false));
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
                        .bg(mark)
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
                    labels.push(pill(text, to((lx, ly))));
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
                                Tool::Mark => (color(style::MARK_GREEN, 0.1), color(style::MARK_GREEN, 1.0)),
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
                            labels.push(pill(text, to(at)));
                        }
                    }
                }
            }
            if let Some(h) =
                o.hovered.as_ref().filter(|h| h.screen == screen && session.drag.is_none() && o.card.is_none())
            {
                prims.push(Prim::Fill(rect(h.frame), color(style::MARK_BLUE, 0.12)));
                prims.push(Prim::Border(rect(h.frame), color(style::MARK_BLUE, 1.0), 1.5, false));
                let name: String = h.name.chars().take(60).collect();
                let w = pill_width(&name);
                let x = (h.frame.x + h.frame.width / 2.0).clamp(w / 2.0 + 4.0, sw - w / 2.0 - 4.0);
                let y = if h.frame.y > 24.0 { h.frame.y - 12.0 } else { h.frame.bottom() + 12.0 };
                labels.push(pill(name, to((x, y))));
            }
        }

        let cursor_style = match session.tool {
            Tool::Mark => CursorStyle::Arrow,
            _ => CursorStyle::Crosshair,
        };
        let shows_toolbar = o.session.cursor.map_or(screen == 0, |c| c.0 == screen);
        let card = o.card.as_ref().filter(|c| c.screen == screen).map(|c| self.card(o, c, (ox, oy), (ww, wh)));
        let menu = o.menu.as_ref().filter(|m| m.screen == screen).map(|m| self.menu(m, (ox, oy), (ww, wh)));
        let toolbar = (shows_toolbar && !session.hidden).then(|| self.toolbar(o));
        let help = (o.help && shows_toolbar).then(|| self.help(o));
        let hidden_hint = (session.hidden && shows_toolbar)
            .then(|| pill(format!("Hidden · {} shows everything again", o.keys.label(Cmd::Hide)), (ww / 2.0, 20.0)));
        let image = o.images[screen].clone();

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
            .child(
                img(ImageSource::Render(image))
                    .absolute()
                    .left(px(-ox as f32))
                    .top(px(-oy as f32))
                    .w(px(sw as f32))
                    .h(px(sh as f32)),
            )
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
            .children(toolbar)
            // On top of everything else.
            .children(card.map(|c| deferred(c).with_priority(1)))
            .children(menu.map(|m| deferred(m).with_priority(2)))
            .children(help.map(|h| deferred(h).with_priority(3)))
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

impl ScreenView {
    fn toolbar(&self, o: &Overlay) -> AnyElement {
        let session = &o.session;
        let tolerance = session.tolerance.level();
        let hint: SharedString = o.hint.clone().unwrap_or_else(|| {
            let k = |c: Cmd| o.keys.label(c);
            match session.tool {
                Tool::Measure => format!(
                    "Click holds the distances · drag sizes what's inside (⌥ each object) · {} / {} hold one · {} / {} \
                     guides · right-click for more",
                    k(Cmd::HoldH),
                    k(Cmd::HoldV),
                    k(Cmd::GuideH),
                    k(Cmd::GuideV)
                ),
                Tool::Mark => "Click an element or drag over an area · type a note, or just say it".to_string(),
                Tool::Draw => {
                    "Drag to circle or underline · ⇧-drag for an arrow · say what you mean, or click its number to type"
                        .to_string()
                }
                Tool::Shot => "Drag over what to capture: it's copied, and added to the aside".to_string(),
            }
            .into()
        });
        let overlay = self.overlay.clone();
        let button = move |id: &str, hint: String, emphasis: Emphasis, content: AnyElement, on: Box<OnPress>| {
            let (o1, o2) = (overlay.clone(), overlay.clone());
            let hint: SharedString = hint.into();
            div()
                .id(SharedString::from(id.to_string()))
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(28.))
                .px(px(8.))
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
                    let hint = hint.clone();
                    o1.update(cx, |o, cx| {
                        o.hint = hovered.then_some(hint);
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
        let sep = || div().w(px(1.)).h(px(18.)).mx(px(4.)).bg(color(0xffffff, 0.15)).into_any_element();
        let k = |c: Cmd| o.keys.label(c);
        let mut items: Vec<AnyElement> = Vec::new();
        for (tool, name) in
            [(Tool::Measure, "measure"), (Tool::Mark, "mark"), (Tool::Draw, "draw"), (Tool::Shot, "shot")]
        {
            let key = k(Cmd::Tool(tool));
            let emphasis = if session.tool == tool { Emphasis::On } else { Emphasis::Plain };
            items.push(button(
                name,
                format!("{} ({key})", tool_help(tool)),
                emphasis,
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon(name, emphasis.ink()))
                    .child(tool.label())
                    .child(div().text_size(px(10.)).opacity(0.55).child(key))
                    .into_any_element(),
                Box::new(move |o, _, _| o.session.set_tool(tool)),
            ));
        }
        items.push(sep());
        items.push(button(
            "tolerance",
            format!(
                "Edge tolerance {}%: higher looks through shadows and gradients ({} / {}, {} high)",
                session.tolerance.0,
                k(Cmd::TolDown),
                k(Cmd::TolUp),
                k(Cmd::TolHigh)
            ),
            Emphasis::Plain,
            div().opacity(0.85).child(format!("Tolerance: {tolerance}")).into_any_element(),
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
            format!("Points or pixels ({})", k(Cmd::Units)),
            Emphasis::Plain,
            div().opacity(0.85).child(session.units.label()).into_any_element(),
            Box::new(|o, _, _| o.session.toggle_units()),
        ));
        items.push(sep());
        let recording = o.recording_seconds();
        let talk_hint = if o.spec.record_shortcut.is_empty() {
            format!("Talk: hold {} (a tap toggles)", keys::label(&o.keys.talk))
        } else {
            format!(
                "Talk: hold {} or {} (a tap toggles); what you say goes with your marks",
                keys::label(&o.keys.talk),
                o.spec.record_shortcut
            )
        };
        let (t1, t2) = (self.overlay.clone(), self.overlay.clone());
        let talk_hint: SharedString = talk_hint.into();
        let th = talk_hint.clone();
        let o3 = self.overlay.clone();
        items.push(
            div()
                .id("talk")
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(28.))
                .px(px(9.))
                .rounded(px(7.))
                .text_size(px(12.))
                .text_color(gpui::white())
                .cursor_pointer()
                .when(recording.is_some(), |d| d.bg(color(style::RECORDING, 1.0)))
                .when(recording.is_none(), |d| d.bg(color(0xffffff, 0.08)).hover(|s| s.bg(color(0xffffff, 0.16))))
                .on_hover(move |hovered, _, cx| {
                    let hint = th.clone();
                    o3.update(cx, |o, cx| {
                        o.hint = hovered.then_some(hint);
                        cx.notify();
                    });
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    t1.update(cx, |o, _| o.talk_button(true));
                })
                .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                    t2.update(cx, |o, _| o.talk_button(false));
                })
                .child(icon("mic", gpui::white()))
                .child(match recording {
                    Some(s) => format!("{}:{:02}", s / 60, s % 60),
                    None => format!("Talk {}", keys::label(&o.keys.talk)),
                })
                .into_any_element(),
        );
        items.push(sep());
        items.push(button(
            "undo",
            format!("Undo ({})", k(Cmd::Undo)),
            Emphasis::Plain,
            icon("undo", Emphasis::Plain.ink()).into_any_element(),
            Box::new(|o, _, _| {
                o.session.undo();
            }),
        ));
        items.push(button(
            "copy",
            format!(
                "Copy as Markdown ({}); {} copies the size, {} the image",
                k(Cmd::CopyMarkdown),
                k(Cmd::CopyDims),
                k(Cmd::CopyImage)
            ),
            Emphasis::Plain,
            icon("copy", Emphasis::Plain.ink()).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::CopyMarkdown, w, cx)),
        ));
        items.push(button(
            "help",
            format!("Keys ({})", k(Cmd::Help)),
            if o.help { Emphasis::On } else { Emphasis::Plain },
            icon("help", if o.help { Emphasis::On.ink() } else { Emphasis::Plain.ink() }).into_any_element(),
            Box::new(|o, _, _| o.help = !o.help),
        ));
        items.push(sep());
        items.push(button(
            "done",
            format!("Add to the aside ({}); {} with the whole screen", k(Cmd::Done), k(Cmd::DoneWhole)),
            Emphasis::Primary,
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .child(icon("check", Emphasis::Primary.ink()))
                .child("Done")
                .into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Done, w, cx)),
        ));
        if o.spec.batch {
            items.push(button(
                "send",
                format!("Add to the aside and send it to your agent ({})", k(Cmd::Send)),
                Emphasis::Plain,
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.))
                    .child(icon("send", Emphasis::Plain.ink()))
                    .child("Send")
                    .into_any_element(),
                Box::new(|o, w, cx| o.command(Cmd::Send, w, cx)),
            ));
        }
        items.push(button(
            "close",
            format!("Close without adding ({})", k(Cmd::Close)),
            Emphasis::Plain,
            icon("close", Emphasis::Plain.ink()).into_any_element(),
            Box::new(|o, w, cx| o.command(Cmd::Close, w, cx)),
        ));

        let toast = o.toast.as_ref().map(|(t, _)| t.clone());
        div()
            .absolute()
            .bottom(px(18.))
            .left_0()
            .right_0()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(8.))
            .children(toast.map(|t| {
                div()
                    .px(px(12.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .bg(color(style::PANEL, style::PANEL_ALPHA))
                    .text_color(gpui::white())
                    .text_size(px(12.))
                    .child(t)
            }))
            .child(
                div()
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(6.))
                    .bg(color(style::PANEL, 0.7))
                    .text_color(color(0xffffff, 0.8))
                    .text_size(px(11.))
                    .child(hint),
            )
            .child(
                div()
                    .id("toolbar")
                    .occlude()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .p(px(5.))
                    .rounded(px(12.))
                    .bg(color(style::PANEL, style::PANEL_ALPHA))
                    .border_1()
                    .border_color(color(0xffffff, 0.12))
                    .shadow_lg()
                    .children(items),
            )
            .into_any_element()
    }

    fn card(&self, o: &Overlay, card: &Card, (ox, oy): (f64, f64), (ww, wh): (f64, f64)) -> AnyElement {
        let state = &o.session.screens[card.screen];
        let Some(a) = state.annotations.get(card.index) else { return div().into_any_element() };
        let (scale, units) = (state.screen.scale, o.session.units);
        let x = (a.at.0 - ox + MARKER).clamp(8.0, (ww - CARD_WIDTH - 8.0).max(8.0));
        let y = (a.at.1 - oy + MARKER).clamp(8.0, (wh - CARD_HEIGHT - 80.0).max(8.0));
        let (r, g, b, _) = style::mark_color(a, o.accent);
        let mark = Hsla::from(Rgba { r: r as f32, g: g as f32, b: b as f32, a: 1.0 });
        let mut title = a.title(scale, units);
        if let Some(app) = a.element.as_ref().and_then(|e| e.app.clone()) {
            title = format!("{title} · {app}");
        }
        let title: String = title.chars().take(52).collect();
        let chip = |id: String, label: &'static str, on: bool, act: Box<dyn Fn(&mut Card)>| {
            let overlay = self.overlay.clone();
            div()
                .id(SharedString::from(id))
                .px(px(7.))
                .py(px(2.))
                .rounded(px(5.))
                .text_size(px(11.))
                .cursor_pointer()
                .when(on, |d| d.bg(color(style::UI, 1.0)).text_color(color(style::UI_INK, 1.0)))
                .when(!on, |d| {
                    d.bg(color(0xffffff, 0.08)).text_color(color(0xffffff, 0.75)).hover(|s| s.bg(color(0xffffff, 0.16)))
                })
                .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                    cx.stop_propagation();
                    overlay.update(cx, |o, cx| {
                        if let Some(card) = &mut o.card {
                            act(card);
                        }
                        cx.notify();
                    });
                })
                .child(label)
        };
        let intents = Intent::ALL.map(|i| {
            chip(
                format!("intent-{}", i.as_str()),
                capitalized(i.as_str()),
                card.intent == Some(i),
                Box::new(move |c: &mut Card| {
                    c.intent = if c.intent == Some(i) { None } else { Some(i) };
                }),
            )
        });
        let severities = Severity::ALL.map(|s| {
            chip(
                format!("severity-{}", s.as_str()),
                capitalized(s.as_str()),
                card.severity == Some(s),
                Box::new(move |c: &mut Card| {
                    c.severity = if c.severity == Some(s) { None } else { Some(s) };
                }),
            )
        });
        let (save, delete) = (self.overlay.clone(), self.overlay.clone());
        div()
            .id("card")
            .occlude()
            .absolute()
            .left(px(x as f32))
            .top(px(y as f32))
            .w(px(CARD_WIDTH as f32))
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.))
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
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .size(px(18.))
                            .rounded_full()
                            .bg(mark)
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(10.))
                            .font_weight(FontWeight::BOLD)
                            .child(a.number.to_string()),
                    )
                    .child(div().text_size(px(12.)).font_weight(FontWeight::SEMIBOLD).child(title)),
            )
            .child(card.input.clone())
            .child(div().flex().gap(px(4.)).children(intents))
            .child(div().flex().gap(px(4.)).children(severities))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_size(px(10.)).opacity(0.5).child("⏎ save · Esc close"))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .child(
                                div()
                                    .id("card-delete")
                                    .px(px(8.))
                                    .py(px(3.))
                                    .rounded(px(6.))
                                    .text_size(px(12.))
                                    .cursor_pointer()
                                    .hover(|s| s.bg(color(0xffffff, 0.12)))
                                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                        cx.stop_propagation();
                                        delete.update(cx, |o, cx| {
                                            o.delete_card_mark(window, cx);
                                            cx.notify();
                                        });
                                    })
                                    .child("Delete"),
                            )
                            .child(
                                div()
                                    .id("card-save")
                                    .px(px(10.))
                                    .py(px(3.))
                                    .rounded(px(6.))
                                    .text_size(px(12.))
                                    .bg(color(style::UI, 1.0))
                                    .text_color(color(style::UI_INK, 1.0))
                                    .hover(|s| s.bg(color(style::UI_LIGHT, 1.0)))
                                    .cursor_pointer()
                                    .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                        cx.stop_propagation();
                                        save.update(cx, |o, cx| {
                                            o.close_card(true, window, cx);
                                            cx.notify();
                                        });
                                    })
                                    .child("Save"),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn menu(&self, menu: &Menu, (ox, oy): (f64, f64), (ww, wh): (f64, f64)) -> AnyElement {
        let height = menu.items.len() as f64 * 26.0 + 10.0;
        let x = (menu.at.0 - ox).min(ww - MENU_WIDTH - 6.0).max(6.0);
        let y = if menu.at.1 - oy + height > wh - 6.0 { (menu.at.1 - oy - height).max(6.0) } else { menu.at.1 - oy };
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

    fn help(&self, o: &Overlay) -> AnyElement {
        let k = |c: Cmd| o.keys.label(c);
        let talk = keys::label(&o.keys.talk);
        let talk =
            if o.spec.record_shortcut.is_empty() { talk } else { format!("{talk} or {}", o.spec.record_shortcut) };
        let left = [
            ("Tools", String::new()),
            ("Measure: distances, sizes, guides", k(Cmd::Tool(Tool::Measure))),
            ("Mark an element or an area", k(Cmd::Tool(Tool::Mark))),
            ("Draw: circle, underline, ⇧ arrow", k(Cmd::Tool(Tool::Draw))),
            ("Screenshot of a part", k(Cmd::Tool(Tool::Shot))),
            ("Measuring", String::new()),
            ("Hold the horizontal · vertical distance", format!("{} · {}", k(Cmd::HoldH), k(Cmd::HoldV))),
            ("Horizontal · vertical guide", format!("{} · {}", k(Cmd::GuideH), k(Cmd::GuideV))),
            ("Move the last guide", "← → ↑ ↓".into()),
            ("Tolerance down · up · high", format!("{} · {} · {}", k(Cmd::TolDown), k(Cmd::TolUp), k(Cmd::TolHigh))),
            ("Points or pixels", k(Cmd::Units)),
            ("Square · no snapping · each object", "⇧ · ⌘ · ⌥ drag".into()),
            ("Move the area while dragging", "Space".into()),
            ("Last session's guides", k(Cmd::Restore)),
        ];
        let right = [
            ("Talking", String::new()),
            ("Hold to talk, tap to toggle", talk),
            ("Copy", String::new()),
            ("The size · the image", format!("{} · {}", k(Cmd::CopyDims), k(Cmd::CopyImage))),
            ("Everything as Markdown", k(Cmd::CopyMarkdown)),
            ("Session", String::new()),
            ("Undo · clear · hide", format!("{} · {} · {}", k(Cmd::Undo), k(Cmd::Clear), k(Cmd::Hide))),
            ("More, where you click", "Right-click".into()),
            ("Add to the aside", k(Cmd::Done)),
            ("… with the whole screen", k(Cmd::DoneWhole)),
            ("Close without adding", k(Cmd::Close)),
        ];
        let right: Vec<(&'static str, String)> =
            right.into_iter().chain(o.spec.batch.then(|| ("Add and send to your agent", k(Cmd::Send)))).collect();
        let column = |rows: &[(&'static str, String)]| {
            div().flex_1().flex().flex_col().gap(px(3.)).children(rows.iter().map(|(label, key)| {
                if key.is_empty() {
                    div()
                        .pt(px(6.))
                        .text_size(px(11.))
                        .text_color(color(style::UI, 1.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(*label)
                        .into_any_element()
                } else {
                    div()
                        .flex()
                        .justify_between()
                        .gap(px(16.))
                        .text_size(px(12.))
                        .child(*label)
                        .child(div().opacity(0.65).child(key.clone()))
                        .into_any_element()
                }
            }))
        };
        div()
            .absolute()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .id("help")
                    .occlude()
                    .w(px(660.))
                    .p(px(18.))
                    .rounded(px(14.))
                    .bg(color(style::PANEL, 0.98))
                    .border_1()
                    .border_color(color(0xffffff, 0.12))
                    .shadow_lg()
                    .text_color(gpui::white())
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(div().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child("Measure & Annotate"))
                    .child(div().text_size(px(12.)).opacity(0.7).child(
                        "Measure, mark and draw on the frozen screen, and talk while you do. Done adds it all to your \
                         aside, each mark where you were speaking.",
                    ))
                    .child(div().flex().gap(px(28.)).child(column(&left)).child(column(&right)))
                    .child(
                        div()
                            .pt(px(4.))
                            .text_size(px(11.))
                            .opacity(0.5)
                            .child(format!("{} shows this again · Esc closes it", k(Cmd::Help))),
                    ),
            )
            .into_any_element()
    }
}

fn tool_help(tool: Tool) -> &'static str {
    match tool {
        Tool::Measure => "Measure distances and sizes",
        Tool::Mark => "Mark an element or an area",
        Tool::Draw => "Draw: circle or underline, ⇧ for an arrow",
        Tool::Shot => "Take a screenshot of a part",
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

#[allow(dead_code)]
fn units_label(units: Units) -> &'static str {
    units.label()
}

/// The clock: every `TICK`, the app's commands and what changes with time.
pub fn run_clock(overlay: &Entity<Overlay>, cx: &mut App) {
    let weak = overlay.downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            if weak.update(cx, |o, cx| o.tick(cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}
