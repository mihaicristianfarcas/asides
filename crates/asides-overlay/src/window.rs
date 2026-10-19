//! The asides window (`asides-overlay --window`): History (every aside: what was sent where, sent
//! again, copied, revealed, deleted), Sessions (the agent sessions asides go to, and where new ones
//! start) and the settings (General, Keys, Voice, Pins), every one of them; its foot says what
//! asides is doing, or offers an update (one click installs it), then What's New. The app starts it
//! from its menu (and `asides`); a line `show` on standard input brings it back. Its look is
//! `controls.rs`'.
//!
//! It reads what the app publishes (`state.json`), the history (`~/.asides/history`) and the
//! settings (`config.toml`, which the app reloads when it changes); what only the app can do (send,
//! download a voice model) it asks for with `asides://` links.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use asides_core::agents::AgentKind;
use asides_core::config::{Config, OverlayKeys, join_args, parse_languages, pretty_shortcut, split_args};
use asides_core::deliver;
use asides_core::history::{self, Aside};
use asides_core::inspect::{Detail, Intent, Units};
use asides_core::keys::{self, Chord, Mods};
use asides_core::link::Link;
use asides_core::markdown::Part;
use asides_core::models::{MODELS, ModelSpec};
use asides_core::paths::{self, tilde};
use asides_core::sessions::{self, AgentSession};
use asides_core::state::AppState;
use asides_core::update::{self, Block, Notes, Phase, Span, Step};
use chrono::{Local, TimeZone};
use gpui::{
    AnyElement, App, AppContext, Application, Bounds, Context, Entity, FocusHandle, Focusable, FontWeight, Hsla,
    ImageSource, KeyBinding, KeyDownEvent, Menu, MenuItem, MouseButton, PathPromptOptions, Render, ScrollHandle,
    SharedString, TitlebarOptions, Window, WindowAppearance, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    actions, div, img, point, prelude::*, px, size,
};

actions!(
    asides_window,
    [ShowHistory, ShowSessions, ShowSettings, CheckForUpdates, QuitAsides, HideAsides, Minimize, CloseWindow]
);

use crate::controls::{
    Choice, Glide, Metrics, Radius, Theme, Type, button, chip, fade_in, floating, font, footnote, hex, icon,
    icon_button, key_recorder, label_stack, page_header, rows, scrollbar, section, segmented, select, setting, stepper,
    swatches, switch, toolbar_row,
};
use crate::input::{Look, TextInput};
use crate::{assets, native, style};

/// How often the window looks at what changed (the history, the app's state, the settings).
const TICK: Duration = Duration::from_millis(500);
const TOAST: Duration = Duration::from_millis(2500);
/// History's list of asides.
const LIST: f32 = 300.0;

#[derive(Clone, Copy, PartialEq)]
enum Page {
    History,
    Sessions,
    General,
    Keys,
    Voice,
    Pins,
    /// What's new in this asides and the update found (not in the sidebar: its foot leads here).
    WhatsNew,
}

impl Page {
    fn named(name: &str) -> Option<Page> {
        Some(match name {
            "history" => Page::History,
            "sessions" => Page::Sessions,
            "general" => Page::General,
            "keys" => Page::Keys,
            "voice" => Page::Voice,
            "pins" => Page::Pins,
            "whats-new" => Page::WhatsNew,
            _ => return None,
        })
    }
}

/// What a key press records, while one is being recorded.
#[derive(Clone, Copy, PartialEq)]
enum Recording {
    /// A global shortcut.
    Asides,
    Freeze,
    /// One of the frozen screen's keys (its name in the settings).
    Key(&'static str),
}

/// The frozen screen's keys, as the Keys page groups and names them (by their names in the
/// settings, `OverlayKeys::NAMES`).
const KEY_GROUPS: &[(&str, &[(&str, &str)])] = &[
    ("Tools", &[("annotate", "Annotate"), ("measure", "Measure"), ("draw", "Draw"), ("shot", "Screenshot")]),
    (
        "Measuring",
        &[
            ("hold_horizontal", "Hold the horizontal distance"),
            ("hold_vertical", "Hold the vertical distance"),
            ("guide_horizontal", "Horizontal guide"),
            ("guide_vertical", "Vertical guide"),
            ("tolerance_down", "Lower tolerance"),
            ("tolerance_up", "Higher tolerance"),
            ("tolerance_high", "High tolerance, on or off"),
            ("units", "Points or pixels"),
            ("restore", "The last guides and measurements again"),
        ],
    ),
    (
        "Copying",
        &[("copy_dimensions", "The dimensions"), ("copy_image", "The image"), ("copy_markdown", "As Markdown")],
    ),
    ("Editing", &[("undo", "Undo"), ("clear", "Clear everything"), ("hide", "Hide everything, or show it")]),
    (
        "The aside",
        &[
            ("talk", "Talk while muted (held)"),
            ("target", "The next session"),
            ("done", "Send"),
            ("done_whole_screen", "Send, with the whole screen"),
            ("live", "The live screen"),
            ("help", "Show the keys"),
            ("close", "Close without sending"),
        ],
    ),
];

/// What the Keys page calls the action named `name`.
fn key_name(name: &str) -> &'static str {
    KEY_GROUPS.iter().flat_map(|(_, keys)| keys.iter()).find(|(n, _)| *n == name).map_or("another action", |(_, l)| l)
}

/// The colors the pins and measurements can be drawn in.
const COLORS: [u32; 7] = [0xff2d55, 0xff3b30, 0xff9500, 0xffcc00, 0x34c759, 0x007aff, 0xaf52de];

/// `#RRGGBB` as typed (with or without `#`), uppercase.
fn color_hex(text: &str) -> Option<String> {
    let digits = text.trim().trim_start_matches('#');
    (digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit())).then(|| format!("#{}", digits.to_uppercase()))
}

/// The field for an agent's arguments.
fn args_field(kind: AgentKind) -> &'static str {
    match kind {
        AgentKind::Claude => "args-claude",
        AgentKind::Codex => "args-codex",
        AgentKind::OpenCode => "args-opencode",
        AgentKind::Gemini => "args-gemini",
        AgentKind::Copilot => "args-copilot",
        AgentKind::Qwen => "args-qwen",
    }
}

/// Where History sends an aside: a session running (its id), or a new one in a folder.
#[derive(Clone, Debug, PartialEq)]
enum Destination {
    Session(String),
    New(PathBuf),
}

/// The folders New session in… offers, at most.
const NEW_SESSION_FOLDERS: usize = 5;

pub struct AsidesWindow {
    page: Page,
    asides: Vec<Aside>,
    selected: Option<String>,
    /// Where History sends to: the session typed into last until another is picked, or a new one.
    send_to: Option<Destination>,
    /// The agent a new session starts (the first installed until another is picked), and folders
    /// chosen for one here (Choose Folder…), before the recent ones.
    new_agent: Option<AgentKind>,
    chosen_dirs: Vec<PathBuf>,
    state: AppState,
    config: Config,
    /// When the settings file was last read.
    config_read: Option<SystemTime>,
    toast: Option<(SharedString, Instant)>,
    commands: Receiver<String>,
    theme: Theme,
    /// The window's width, last drawn: screenshots in History fit their column.
    width: f32,
    /// Where its keys go (⌘, and the pages' shortcuts).
    focus: FocusHandle,
    /// The glass pills behind the page and the aside selected, sliding to the next.
    nav_glide: Glide,
    list_glide: Glide,
    /// Where History's list, the aside shown and a settings page are scrolled to (their bars).
    list_scroll: ScrollHandle,
    aside_scroll: ScrollHandle,
    page_scroll: ScrollHandle,
    /// Settings typed in (an agent's arguments, the languages, a color), by setting, and what each
    /// last showed of the settings: saved on Return or when the field loses the keyboard (`commit`).
    fields: HashMap<&'static str, (Entity<TextInput>, String)>,
    /// The shortcut or key being recorded: the next key pressed.
    recording: Option<Recording>,
    /// What's New was read, up to this version (told to the app once).
    told_seen: Option<String>,
}

/// Opens the window; it quits when it's closed.
pub fn run() {
    // While it's open, the window is an app of its own (in the Dock and ⌘Tab): it quits when closed.
    let app = Application::new().with_assets(assets::Assets);
    // Its Dock icon clicked, minimized: back.
    app.on_reopen(|cx| {
        for handle in cx.windows() {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
    });
    app.run(|cx: &mut App| {
        // A Mac app's keys and menus: ⌘, opens the settings, ⌘1 and ⌘2 History and Sessions; ⌘Q
        // quits asides (the menu bar's too), ⌘W closes the window.
        cx.bind_keys([
            KeyBinding::new("cmd-,", ShowSettings, None),
            KeyBinding::new("cmd-1", ShowHistory, None),
            KeyBinding::new("cmd-2", ShowSessions, None),
            KeyBinding::new("cmd-q", QuitAsides, None),
            KeyBinding::new("cmd-h", HideAsides, None),
            KeyBinding::new("cmd-m", Minimize, None),
            KeyBinding::new("cmd-w", CloseWindow, None),
        ]);
        cx.on_action(|_: &QuitAsides, cx| {
            let _ = send_link(&Link::Quit);
            cx.quit();
        });
        cx.on_action(|_: &HideAsides, cx| cx.hide());
        cx.on_action(|_: &CheckForUpdates, _| {
            let _ = send_link(&Link::Update { step: Step::Check, request: None });
        });
        cx.set_menus(vec![
            Menu {
                name: "asides".into(),
                items: vec![
                    MenuItem::action("Settings…", ShowSettings),
                    MenuItem::action("Check for Updates…", CheckForUpdates),
                    MenuItem::separator(),
                    MenuItem::action("Hide asides", HideAsides),
                    MenuItem::separator(),
                    MenuItem::action("Quit asides", QuitAsides),
                ],
            },
            Menu {
                name: "Window".into(),
                items: vec![
                    MenuItem::action("Minimize", Minimize),
                    MenuItem::action("Close", CloseWindow),
                    MenuItem::separator(),
                    MenuItem::action("History", ShowHistory),
                    MenuItem::action("Sessions", ShowSessions),
                ],
            },
        ]);
        // The settings' fields edit as the card's does.
        crate::input::bind_keys(cx);
        let (tx, commands) = channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tx.send(line.trim().to_string()).is_err() {
                    return;
                }
            }
        });
        let bounds = Bounds::centered(None, size(px(1240.), px(780.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("asides".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(16.), px(16.))),
            }),
            focus: true,
            show: true,
            window_min_size: Some(size(px(920.), px(600.))),
            // Glass, as the overlay's surfaces: the window blurs what's behind it (`Theme`'s tints).
            window_background: WindowBackgroundAppearance::Blurred,
            ..WindowOptions::default()
        };
        // Development: one appearance or the other, whatever the system's.
        let forced = match std::env::var("ASIDES_WINDOW_APPEARANCE").as_deref() {
            Ok("light") => Some(WindowAppearance::Light),
            Ok("dark") => Some(WindowAppearance::Dark),
            _ => None,
        };
        let opened = cx.open_window(options, move |window, cx| {
            let theme = Theme::of(forced.unwrap_or(window.appearance()));
            cx.new(|cx| {
                cx.observe_window_appearance(window, move |this: &mut AsidesWindow, window, cx| {
                    this.theme = Theme::of(forced.unwrap_or(window.appearance()));
                    cx.notify();
                })
                .detach();
                let focus = cx.focus_handle();
                window.focus(&focus);
                let glides = (Glide::new(cx), Glide::new(cx));
                AsidesWindow::new(commands, theme, focus, glides)
            })
        });
        let Ok(handle) = opened else {
            eprintln!("asides-overlay: could not open the asides window");
            return cx.quit();
        };
        cx.on_window_closed(|cx| cx.quit()).detach();
        native::activate();
        if let Some(dir) = std::env::var_os("ASIDES_WINDOW_CAPTURE") {
            return capture_pages(handle, PathBuf::from(dir), cx);
        }
        cx.spawn(async move |cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = handle.update(cx, |this, window, cx| this.tick(window, cx)).is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();
    });
}

/// Development (`ASIDES_WINDOW_CAPTURE=<dir>`): each page in each appearance, captured to
/// `<dir>/<page>-<dark|light>.png`, then it quits.
fn capture_pages(handle: gpui::WindowHandle<AsidesWindow>, dir: PathBuf, cx: &mut App) {
    let pages = [
        (Page::History, "history"),
        (Page::Sessions, "sessions"),
        (Page::General, "general"),
        (Page::Keys, "keys"),
        (Page::Voice, "voice"),
        (Page::Pins, "pins"),
        (Page::WhatsNew, "whats-new"),
    ];
    cx.spawn(async move |cx| {
        for (theme, look) in [(Theme::dark(), "dark"), (Theme::light(), "light")] {
            for (page, name) in pages {
                let _ = handle.update(cx, |this, _, cx| {
                    (this.page, this.theme) = (page, theme);
                    cx.notify();
                });
                cx.background_executor().timer(Duration::from_millis(500)).await;
                // Where the window is on screen, as it shows there: glass shows what's behind it.
                let Ok(b) = handle.update(cx, |_, window, _| window.bounds()) else { continue };
                let p = |v: gpui::Pixels| f32::from(v);
                let rect = format!("{},{},{},{}", p(b.origin.x), p(b.origin.y), p(b.size.width), p(b.size.height));
                let path = dir.join(format!("{name}-{look}.png"));
                let _ = Command::new("screencapture").args(["-x", "-o", "-R", &rect]).arg(&path).status();
            }
        }
        let _ = cx.update(|cx| cx.quit());
    })
    .detach();
}

impl AsidesWindow {
    fn new(commands: Receiver<String>, theme: Theme, focus: FocusHandle, glides: (Glide, Glide)) -> AsidesWindow {
        let mut this = AsidesWindow {
            page: Page::History,
            asides: Vec::new(),
            selected: None,
            send_to: None,
            new_agent: None,
            chosen_dirs: Vec::new(),
            state: AppState::default(),
            config: Config::default(),
            config_read: None,
            toast: None,
            commands,
            theme,
            width: 1240.,
            focus,
            nav_glide: glides.0,
            list_glide: glides.1,
            list_scroll: ScrollHandle::new(),
            aside_scroll: ScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            fields: HashMap::new(),
            recording: None,
            told_seen: None,
        };
        this.reload();
        this.selected = this.asides.first().map(|a| a.id.clone());
        // Development: where it opens (to capture each page).
        // Where it opens: the menu's History… or Settings…, or a page to capture.
        this.page = std::env::var("ASIDES_WINDOW_PAGE").ok().and_then(|p| Page::named(&p)).unwrap_or(Page::History);
        // Development: a shortcut or key recording, or a field focused, when it opens (to try them
        // with posted keys).
        this.recording = match std::env::var("ASIDES_WINDOW_FOCUS").as_deref() {
            Ok("asides") => Some(Recording::Asides),
            Ok("freeze") => Some(Recording::Freeze),
            Ok(name) => OverlayKeys::NAMES.into_iter().find(|n| *n == name).map(Recording::Key),
            Err(_) => None,
        };
        if let Ok(id) = std::env::var("ASIDES_WINDOW_ASIDE") {
            this.selected = Some(id);
        }
        this
    }

    /// The history, the app's state and the settings, as they are now.
    fn reload(&mut self) {
        self.asides = history::list(&history::dir());
        if let Some(state) = AppState::read(&paths::state_path()) {
            self.state = state;
        }
        let modified = std::fs::metadata(paths::config_path()).and_then(|m| m.modified()).ok();
        if modified != self.config_read || modified.is_none() {
            self.config = Config::load(&paths::config_path()).unwrap_or_default();
            self.config_read = modified;
        }
        if self.selected.as_ref().is_some_and(|id| !self.asides.iter().any(|a| &a.id == id)) {
            self.selected = self.asides.first().map(|a| a.id.clone());
        }
    }

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        while let Ok(command) = self.commands.try_recv() {
            // `show`, or `show general`: forward, at that page.
            if let Some(page) = command.strip_prefix("show") {
                if let Some(page) = Page::named(page.trim()) {
                    self.page = page;
                }
                native::activate();
                window.activate_window();
            }
        }
        if self.toast.as_ref().is_some_and(|(_, until)| Instant::now() > *until) {
            self.toast = None;
        }
        self.reload();
        // What's New open: it's read.
        let newest = self.state.update.unseen(self.state.whats_new_seen.as_deref()).first().map(|n| n.version.clone());
        if self.page == Page::WhatsNew && newest.is_some() && newest != self.told_seen {
            let _ = send_link(&Link::Update { step: Step::Seen, request: None });
            self.told_seen = newest;
        }
        cx.notify();
    }

    fn go(&mut self, page: Page, cx: &mut Context<Self>) {
        if page != self.page {
            self.page_scroll.set_offset(point(px(0.), px(0.)));
        }
        self.page = page;
        cx.notify();
    }

    fn toast(&mut self, text: impl Into<SharedString>) {
        self.toast = Some((text.into(), Instant::now() + TOAST));
    }

    /// Changes the settings (read fresh, so nothing the app wrote meanwhile is lost).
    fn change(&mut self, edit: impl FnOnce(&mut Config)) {
        let mut config = Config::load(&paths::config_path()).unwrap_or_default();
        edit(&mut config);
        match config.save(&paths::config_path()) {
            Ok(()) => {
                self.config = config;
                self.config_read = std::fs::metadata(paths::config_path()).and_then(|m| m.modified()).ok();
            }
            Err(err) => self.toast(format!("Could not save the settings: {err}")),
        }
    }

    fn selected(&self) -> Option<&Aside> {
        self.selected.as_ref().and_then(|id| self.asides.iter().find(|a| &a.id == id))
    }

    /// The sessions the app last found that asides can be typed into, typed into last first.
    fn reachable(&self) -> Vec<(AgentSession, String)> {
        let mut found: Vec<AgentSession> = self.state.sessions.clone();
        found.sort_by_key(|s| std::cmp::Reverse(s.last_input));
        let labels = sessions::labels(&found);
        found.into_iter().zip(labels).filter(|(s, _)| s.host.reachable()).collect()
    }

    /// ⌘1, ⌘2 and ⌘, go to a page, unless keys are being recorded: then they're the keys.
    fn page_or_keys(&mut self, page: Page, keys: &str, cx: &mut Context<Self>) {
        match (self.recording, keys::parse(keys).first()) {
            (Some(_), Some(c)) => {
                self.record(&c.key, Mods { cmd: c.cmd, shift: c.shift, alt: c.alt, ctrl: c.ctrl }, cx)
            }
            _ => self.go(page, cx),
        }
    }

    /// The keys pressed while recording, for what's recording.
    fn record(&mut self, key: &str, mods: Mods, cx: &mut Context<Self>) {
        let (Some(recording), Some(chord)) = (self.recording.take(), Chord::pressed(key, mods)) else { return };
        match recording {
            Recording::Asides | Recording::Freeze => match chord.to_global() {
                Some(text) => self.change(|c| match recording {
                    Recording::Asides => c.shortcuts.asides = text,
                    _ => c.shortcuts.freeze = text,
                }),
                None => self.toast("A shortcut needs ⌃ or ⌘: macOS doesn't take ⌥ alone."),
            },
            Recording::Key(name) => {
                let keys = &self.config.measure.keys;
                let other = OverlayKeys::NAMES
                    .into_iter()
                    .filter(|n| *n != name)
                    .find(|n| keys.get(n).is_some_and(|k| keys::parse(k).contains(&chord)));
                let text = chord.to_config();
                self.change(|c| {
                    if let Some(keys) = c.measure.keys.get_mut(name) {
                        *keys = text;
                    }
                });
                if let Some(other) = other {
                    self.toast(format!("{} also does: {}", chord.label(), key_name(other)));
                }
            }
        }
        cx.notify();
    }

    /// Keycaps for `labels` that a click records anew, as `recording`.
    fn recorder(
        &self,
        id: &'static str,
        labels: Vec<String>,
        recording: Recording,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        key_recorder(id, labels, self.recording == Some(recording), self.theme)
            .on_click(cx.listener(move |this, _, window, cx| {
                this.recording = Some(recording);
                // The keys come to the window, not a field.
                window.focus(&this.focus);
                cx.notify();
            }))
            .into_any_element()
    }

    /// The field for setting `id`, `width` wide, showing `value` (as typed, while it has the
    /// keyboard). Return or leaving it saves it (`commit`); Escape puts it back.
    fn field(
        &mut self,
        id: &'static str,
        value: String,
        placeholder: &str,
        width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let t = self.theme;
        let look = Look {
            text: t.primary,
            placeholder: t.quaternary,
            border: t.control_stroke,
            background: t.control,
            caret: t.accent,
            selection: t.accent.opacity(0.35),
        };
        let input = match self.fields.get(id) {
            Some((input, _)) => input.clone(),
            None => {
                let input = cx.new(|cx| TextInput::new(&value, placeholder, cx));
                let focus = input.read(cx).focus_handle(cx);
                cx.on_blur(&focus, window, move |this, _, cx| this.commit(id, cx)).detach();
                if std::env::var("ASIDES_WINDOW_FOCUS").is_ok_and(|f| f == id) {
                    window.defer(cx, move |window, _| window.focus(&focus));
                }
                self.fields.insert(id, (input.clone(), value.clone()));
                input
            }
        };
        input.update(cx, |input, _| input.look = look);
        // The setting changed elsewhere: shown, unless it's being edited.
        if let Some((_, shown)) = self.fields.get_mut(id)
            && *shown != value
            && input.read(cx).text() == *shown
        {
            input.update(cx, |input, cx| input.set_text(&value, cx));
            *shown = value.clone();
        }
        let (field, root) = (input.clone(), self.focus.clone());
        div()
            .w(px(width))
            .on_key_down(cx.listener(move |this, e: &KeyDownEvent, window, cx| match e.keystroke.key.as_str() {
                "enter" => {
                    this.commit(id, cx);
                    window.focus(&root);
                    cx.stop_propagation();
                }
                "escape" => {
                    field.update(cx, |field, cx| field.set_text(&value, cx));
                    window.focus(&root);
                    cx.stop_propagation();
                }
                _ => {}
            }))
            .child(input)
            .into_any_element()
    }

    /// Saves what's typed in setting `id`'s field; what doesn't make sense, it says, and the
    /// field shows the setting again.
    fn commit(&mut self, id: &'static str, cx: &mut Context<Self>) {
        let Some((input, _)) = self.fields.get(id).cloned() else { return };
        let text = input.read(cx).text();
        match id {
            "languages" => {
                let codes = parse_languages(&text);
                if codes != self.config.languages {
                    self.change(|c| c.languages = codes);
                }
            }
            "color" => match color_hex(&text) {
                Some(color) if color != self.config.measure.color => self.change(|c| c.measure.color = color),
                Some(_) => {}
                None => self.toast("A color is written #RRGGBB, like #FF2D55."),
            },
            _ => {
                if let Some(kind) = AgentKind::ALL.into_iter().find(|k| args_field(*k) == id) {
                    let args = split_args(&text);
                    if args != self.config.agents.args(kind) {
                        self.change(|c| *c.agents.args_mut(kind) = args);
                    }
                }
            }
        }
        // The setting as saved, written as the field shows it.
        let saved = match id {
            "languages" => self.config.languages.join(", "),
            "color" => self.config.measure.color.clone(),
            _ => AgentKind::ALL
                .into_iter()
                .find(|k| args_field(*k) == id)
                .map(|kind| join_args(self.config.agents.args(kind)))
                .unwrap_or_default(),
        };
        input.update(cx, |input, cx| input.set_text(&saved, cx));
        if let Some((_, shown)) = self.fields.get_mut(id) {
            *shown = saved;
        }
        cx.notify();
    }

    /// A listener that changes the settings with `edit` and redraws.
    fn setter<T: 'static>(
        &self,
        cx: &mut Context<Self>,
        edit: impl Fn(&mut Config, T) + 'static,
    ) -> impl Fn(T, &mut Window, &mut App) + 'static {
        let this = cx.weak_entity();
        let edit = std::rc::Rc::new(edit);
        move |value, _, cx| {
            let edit = edit.clone();
            let _ = this.update(cx, |this, cx| {
                this.change(|c| edit(c, value));
                cx.notify();
            });
        }
    }
}

/// Hands a link to the app (the copy this window ships in), without bringing it forward.
fn send_link(link: &Link) -> std::io::Result<()> {
    let mut open = Command::new("open");
    open.arg("-g");
    let bundle = std::env::current_exe().ok().and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf));
    match bundle.filter(|b| b.extension().is_some_and(|e| e == "app")) {
        Some(app) => open.arg("-a").arg(app),
        None => open.args(["-b", paths::BUNDLE_ID]),
    };
    open.arg(link.to_url()).status().map(drop)
}

fn open_url(url: &str) {
    let _ = Command::new("open").arg(url).status();
}

fn ago(at: SystemTime) -> String {
    let secs = SystemTime::now().duration_since(at).unwrap_or_default().as_secs();
    match secs {
        0..60 => "just now".into(),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86400 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86400),
    }
}

/// A release in What's New: its version and date (and a button), then its notes.
fn release_card(t: Theme, notes: &Notes, action: Option<AnyElement>) -> AnyElement {
    let date = notes
        .published
        .as_deref()
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .map(|d| SharedString::from(d.format("%-d %B %Y").to_string()));
    let head = div()
        .px(px(12.))
        .py(px(10.))
        .flex()
        .items_center()
        .gap(px(16.))
        .child(label_stack(format!("asides {}", notes.version), date, t))
        .children(action);
    let mut body = div().px(px(14.)).py(px(12.)).flex().flex_col().gap(px(8.)).line_height(px(19.));
    for block in update::blocks(&notes.notes) {
        body = body.child(match block {
            Block::Heading(text) => div().pt(px(2.)).font_weight(FontWeight::SEMIBOLD).child(text),
            Block::Paragraph(spans) => div().child(styled(spans, t)),
            Block::Item(spans) => div()
                .flex()
                .gap(px(8.))
                .child(div().flex_none().text_color(t.tertiary).child("•"))
                .child(div().flex_1().min_w_0().child(styled(spans, t))),
        });
    }
    rows([head.into_any_element(), body.into_any_element()], t).into_any_element()
}

/// Release notes' runs: bold as bold, code on a quiet fill.
fn styled(spans: Vec<Span>, t: Theme) -> gpui::StyledText {
    let mut text = String::new();
    let mut highlights = Vec::new();
    for span in spans {
        let start = text.len();
        text.push_str(&span.text);
        if span.bold || span.code {
            let style = gpui::HighlightStyle {
                font_weight: span.bold.then_some(FontWeight::SEMIBOLD),
                background_color: span.code.then_some(t.control),
                ..Default::default()
            };
            highlights.push((start..text.len(), style));
        }
    }
    gpui::StyledText::new(text).with_highlights(highlights)
}

/// What asides is doing, as the sidebar's foot says it, and its color (an error stands out).
fn status(state: &AppState, t: Theme) -> (String, Hsla) {
    let detail = || state.detail.clone().unwrap_or_default();
    match state.status.as_str() {
        "" => ("asides isn't running".into(), t.quaternary),
        "idle" => (format!("asides {}", env!("CARGO_PKG_VERSION")), t.tertiary),
        "open" => ("On screen".into(), t.tertiary),
        "recording" => ("Listening".into(), t.tertiary),
        "transcribing" => ("Transcribing…".into(), t.tertiary),
        "sending" => ("Sending…".into(), t.tertiary),
        "downloading" => (state.detail.clone().unwrap_or_else(|| "Downloading…".into()), t.tertiary),
        "error" => (detail(), t.danger),
        _ => (detail(), t.tertiary),
    }
}

impl Render for AsidesWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        self.width = f32::from(window.viewport_size().width);
        let content = match self.page {
            Page::History => self.history(cx),
            Page::Sessions => self.sessions(window, cx),
            Page::General => self.general(cx),
            Page::Keys => self.keys(cx),
            Page::Voice => self.voice(window, cx),
            Page::Pins => self.pins(window, cx),
            Page::WhatsNew => self.whats_new(),
        };
        div()
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ShowHistory, _, cx| this.page_or_keys(Page::History, "cmd+1", cx)))
            .on_action(cx.listener(|this, _: &ShowSessions, _, cx| this.page_or_keys(Page::Sessions, "cmd+2", cx)))
            .on_action(cx.listener(|this, _: &ShowSettings, _, cx| this.page_or_keys(Page::General, "cmd+comma", cx)))
            .on_action(|_: &Minimize, window, _| window.minimize_window())
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            // Recording, the next key pressed is it.
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if this.recording.is_some() {
                    let m = &e.keystroke.modifiers;
                    this.record(
                        &e.keystroke.key,
                        Mods { cmd: m.platform, shift: m.shift, alt: m.alt, ctrl: m.control },
                        cx,
                    );
                    cx.stop_propagation();
                }
            }))
            // A click makes asides the app in front, so its keys (⌘,) come here, not to another app;
            // and anywhere but on a recorder, it stops recording.
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if !native::is_active() {
                        native::activate();
                    }
                    if this.recording.take().is_some() {
                        cx.notify();
                    }
                }),
            )
            .size_full()
            .relative()
            .flex()
            .font_family(font())
            .text_color(t.primary)
            .text_size(px(Type::BODY))
            .child(self.sidebar(cx))
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().bg(t.background).child(content))
            .children(self.toast.as_ref().map(|(text, _)| {
                div().absolute().bottom(px(18.)).left_0().right_0().flex().justify_center().child(fade_in(
                    floating(t)
                        .flex_row()
                        .px(px(12.))
                        .py(px(7.))
                        .rounded(px(Radius::ROW))
                        .text_size(px(12.))
                        .child(text.clone()),
                    "toast",
                    6.,
                ))
            }))
    }
}

impl AsidesWindow {
    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let current = self.page;
        let nav = |id: &'static str,
                   label: &'static str,
                   glyph: &'static str,
                   page: Page,
                   count: Option<usize>,
                   cx: &mut Context<Self>| {
            let on = current == page;
            div()
                .id(id)
                .h(px(Metrics::NAV_ROW))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(Radius::ROW))
                .border_1()
                .border_color(hex(0, 0.0))
                .text_color(if on { t.primary } else { t.primary.opacity(0.78) })
                .relative()
                .child(self.nav_glide.item(page as usize))
                .when(!on, |d| {
                    d.cursor_pointer().hover(|s| s.bg(t.hover)).on_click(cx.listener(move |this, _, _, cx| {
                        this.page = page;
                        cx.notify();
                    }))
                })
                .child(icon(glyph, 15., if on { t.primary } else { t.secondary }))
                .child(div().flex_1().child(label))
                .children(
                    count
                        .filter(|n| *n > 0)
                        .map(|n| div().text_size(px(Type::META)).text_color(t.tertiary).child(n.to_string())),
                )
        };
        let header = |label: &'static str| {
            div()
                .pt(px(14.))
                .pb(px(4.))
                .px(px(9.))
                .text_size(px(Type::META))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.tertiary)
                .child(label)
        };
        // What's New isn't in the sidebar.
        let selected = (current != Page::WhatsNew).then_some(current as usize);
        div()
            .flex_none()
            .w(px(Metrics::SIDEBAR))
            .h_full()
            .flex()
            .flex_col()
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.hairline)
            .child(div().flex_none().h(px(Metrics::TOOLBAR)))
            .child(
                div()
                    .flex_1()
                    .px(px(8.))
                    .pt(px(6.))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .relative()
                    .child(self.nav_glide.layer(selected, t.selected, t.selected_stroke, Radius::ROW))
                    .child(nav("nav-history", "History", "clock", Page::History, Some(self.asides.len()), cx))
                    .child(nav(
                        "nav-sessions",
                        "Sessions",
                        "terminal",
                        Page::Sessions,
                        Some(self.state.sessions.len()),
                        cx,
                    ))
                    .child(header("Settings"))
                    .child(nav("nav-general", "General", "sliders", Page::General, None, cx))
                    .child(nav("nav-keys", "Keys", "keyboard", Page::Keys, None, cx))
                    .child(nav("nav-voice", "Voice", "mic", Page::Voice, None, cx))
                    .child(nav("nav-pins", "Pins", "annotate", Page::Pins, None, cx)),
            )
            .child(self.foot(cx))
            .into_any_element()
    }

    /// The sidebar's foot: what asides is doing; waiting, an update (one click installs it), then
    /// What's New once it's in, else asides' version.
    fn foot(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let update = &self.state.update;
        let row = |id: &'static str| {
            div()
                .id(id)
                .flex_none()
                .mx(px(8.))
                .mb(px(8.))
                .h(px(Metrics::NAV_ROW))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(Radius::ROW))
                .text_size(px(12.))
        };
        let idle = self.state.status == "idle";
        if idle && update.shows() {
            let step = update.click();
            let (glyph, tint) = match update.phase {
                Phase::Available => ("download", t.fresh),
                Phase::Ready => ("restart", t.fresh),
                Phase::Failed => ("warning", t.danger),
                Phase::UpToDate => ("check", t.tertiary),
                Phase::Installing => ("restart", t.tertiary),
                _ => ("download", t.tertiary),
            };
            return row("update")
                .child(icon(glyph, 14., tint))
                .child(div().flex_1().min_w_0().truncate().text_color(tint).child(update.summary()))
                .when_some(step, |d, step| {
                    d.cursor_pointer().hover(|s| s.bg(t.hover)).on_click(move |_, _, _| {
                        let _ = send_link(&Link::Update { step, request: None });
                    })
                })
                .into_any_element();
        }
        if let (true, Some(newest)) = (idle, update.unseen(self.state.whats_new_seen.as_deref()).first()) {
            return row("whats-new")
                .group("whats-new")
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .on_click(cx.listener(|this, _, _, cx| this.go(Page::WhatsNew, cx)))
                .child(icon("sparkles", 14., t.accent))
                .child(div().flex_1().min_w_0().truncate().child(format!("What's new in {}", newest.version)))
                .child(
                    // Hovered, it can be put away unread.
                    div()
                        .id("whats-new-seen")
                        .flex_none()
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(Radius::CHIP))
                        .opacity(0.)
                        .group_hover("whats-new", |s| s.opacity(1.))
                        .hover(|s| s.bg(t.hover))
                        .child(icon("close", 11., t.tertiary))
                        .on_click(|_, _, cx| {
                            cx.stop_propagation();
                            let _ = send_link(&Link::Update { step: Step::Seen, request: None });
                        }),
                )
                .into_any_element();
        }
        // asides' version, or what it's doing when that's more than waiting.
        let (state, color) = status(&self.state, t);
        div()
            .flex_none()
            .px(px(17.))
            .pb(px(14.))
            .text_size(px(12.))
            .text_color(color)
            .truncate()
            .child(state)
            .into_any_element()
    }

    // --- History ---

    fn history(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        if self.asides.is_empty() {
            let shortcut = pretty_shortcut(&self.config.shortcuts.asides);
            return empty(
                t,
                "clock",
                "No asides yet",
                &format!("Press {shortcut} over any app, click what's wrong and say why, then press {shortcut} again."),
            );
        }
        let today = Local::now().date_naive();
        let mut list: Vec<AnyElement> = Vec::new();
        let mut day = None;
        for (index, aside) in self.asides.iter().enumerate() {
            let created = aside.created();
            if day != Some(created.date_naive()) {
                day = Some(created.date_naive());
                let label = match (today - created.date_naive()).num_days() {
                    0 => "Today".to_string(),
                    1 => "Yesterday".to_string(),
                    _ => created.format("%A %-d %B").to_string(),
                };
                list.push(
                    div()
                        .pt(px(if list.is_empty() { 4. } else { 14. }))
                        .pb(px(4.))
                        .px(px(10.))
                        .text_size(px(Type::META))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.tertiary)
                        .child(label)
                        .into_any_element(),
                );
            }
            let on = self.selected.as_ref() == Some(&aside.id);
            let id = aside.id.clone();
            let (glyph, where_, failed) = delivered(aside);
            list.push(
                div()
                    .id(SharedString::from(format!("aside-{id}")))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(10.))
                    .py(px(7.))
                    .rounded(px(Radius::ROW))
                    .border_1()
                    .border_color(hex(0, 0.0))
                    .relative()
                    .child(self.list_glide.item(index))
                    .when(!on, |d| d.cursor_pointer().hover(|s| s.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.selected.as_ref() != Some(&id) {
                            this.aside_scroll.set_offset(point(px(0.), px(0.)));
                        }
                        this.selected = Some(id.clone());
                        cx.notify();
                    }))
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(aside.record.title.clone()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(Type::META))
                            .text_color(if failed { t.danger } else { t.tertiary })
                            .child(div().flex_none().child(created.format("%H:%M").to_string()))
                            .child(icon(glyph, 11., if failed { t.danger } else { t.tertiary }))
                            .child(div().truncate().child(where_)),
                    )
                    .into_any_element(),
            );
        }
        let detail = self.selected().cloned().map(|aside| self.aside_detail(&aside, cx));
        let chosen = self.selected.as_ref().and_then(|id| self.asides.iter().position(|a| &a.id == id));
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .flex_none()
                    .w(px(LIST))
                    .h_full()
                    .flex()
                    .flex_col()
                    .border_r_1()
                    .border_color(t.hairline)
                    .child(toolbar_row(
                        Some(
                            div()
                                .pl(px(2.))
                                .flex()
                                .items_baseline()
                                .gap(px(8.))
                                .child(
                                    div().text_size(px(Type::TITLE)).font_weight(FontWeight::SEMIBOLD).child("History"),
                                )
                                .child(div().text_size(px(Type::META)).text_color(t.tertiary).child(format!(
                                    "kept {}",
                                    match self.config.history_days {
                                        0 => "always".to_string(),
                                        days => format!("{days} days"),
                                    }
                                )))
                                .into_any_element(),
                        ),
                        Vec::new(),
                    ))
                    .child(scrolled(
                        div()
                            .id("history-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.list_scroll)
                            .px(px(8.))
                            .pb(px(10.))
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .relative()
                            .child(self.list_glide.layer(chosen, t.selected, t.selected_stroke, Radius::ROW))
                            .children(list),
                        "history-bar".into(),
                        &self.list_scroll,
                        t,
                    )),
            )
            .child(div().flex_1().min_w_0().h_full().flex().flex_col().children(detail))
            .into_any_element()
    }

    fn aside_detail(&mut self, aside: &Aside, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        // The sessions running, then a new one in a recent folder, or in one chosen.
        let reachable = self.reachable();
        let mut folders: Vec<PathBuf> = Vec::new();
        for dir in self.chosen_dirs.iter().chain(&self.state.recent_dirs) {
            if !folders.contains(dir) && folders.len() < NEW_SESSION_FOLDERS {
                folders.push(dir.clone());
            }
        }
        let mut destinations: Vec<Destination> = reachable.iter().map(|(s, _)| Destination::Session(s.id())).collect();
        let mut choices: Vec<Choice> = reachable
            .iter()
            .map(|(s, label)| Choice::new(label.clone()).detail(format!("{} (in {})", tilde(&s.dir), s.host.name())))
            .collect();
        for (i, dir) in folders.iter().enumerate() {
            let choice = Choice::new(format!("New session in {}", sessions::repo_name(dir))).detail(tilde(dir));
            choices.push(if i == 0 && !reachable.is_empty() { choice.separated() } else { choice });
            destinations.push(Destination::New(dir.clone()));
        }
        let choose_folder = destinations.len();
        let choice = Choice::new("Choose Folder…").detail("A new session somewhere else");
        choices.push(if folders.is_empty() && !reachable.is_empty() { choice.separated() } else { choice });
        let chosen = self
            .send_to
            .as_ref()
            .and_then(|d| destinations.iter().position(|x| x == d))
            .or((!destinations.is_empty()).then_some(0));
        let target = chosen.map(|i| destinations[i].clone());
        // A new session's agent: the first installed, until another is picked.
        let agents = match self.state.agents.as_slice() {
            [] => vec![AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode],
            installed => installed.to_vec(),
        };
        let agent = self.new_agent.filter(|a| agents.contains(a)).unwrap_or(agents[0]);
        let mut actions: Vec<AnyElement> = Vec::new();
        if matches!(target, Some(Destination::New(_))) {
            let this = cx.weak_entity();
            let picked = agents.iter().position(|a| *a == agent);
            let names = agents.iter().map(|a| Choice::new(a.name())).collect();
            actions.push(
                select("new-agent", names, picked, t)
                    .width(130.)
                    .on_select(move |i, _, cx| {
                        let agent = agents[i];
                        let _ = this.update(cx, |this, cx| {
                            this.new_agent = Some(agent);
                            cx.notify();
                        });
                    })
                    .into_any_element(),
            );
        }
        let this = cx.weak_entity();
        actions.push(
            select("send-to", choices, chosen, t)
                .width(240.)
                .on_select(move |i, _, cx| {
                    let this = this.clone();
                    if i != choose_folder {
                        let to = destinations[i].clone();
                        let _ = this.update(cx, |this, cx| {
                            this.send_to = Some(to);
                            cx.notify();
                        });
                        return;
                    }
                    let folder = cx.prompt_for_paths(PathPromptOptions {
                        files: false,
                        directories: true,
                        multiple: false,
                        prompt: Some("Start Here".into()),
                    });
                    cx.spawn(async move |cx| {
                        let Ok(Ok(Some(paths))) = folder.await else { return };
                        let Some(dir) = paths.into_iter().next() else { return };
                        let _ = this.update(cx, |this, cx| {
                            this.chosen_dirs.retain(|d| *d != dir);
                            this.chosen_dirs.insert(0, dir.clone());
                            this.send_to = Some(Destination::New(dir));
                            cx.notify();
                        });
                    })
                    .detach();
                })
                .into_any_element(),
        );
        let id = aside.id.clone();
        actions.push(
            button("send", "Send", t)
                .accent()
                .icon("send")
                .disabled(target.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    let (link, doing) = match target.clone() {
                        Some(Destination::Session(to)) => {
                            (Link::Send { id: id.clone(), to: Some(to), request: None }, "Sending…".to_string())
                        }
                        Some(Destination::New(dir)) => {
                            let doing = format!("Starting {} in {}…", agent.name(), sessions::repo_name(&dir));
                            (Link::NewSession { id: id.clone(), agent, dir, request: None }, doing)
                        }
                        None => return,
                    };
                    match send_link(&link) {
                        Ok(()) => this.toast(doing),
                        Err(err) => this.toast(format!("Could not reach asides: {err}")),
                    }
                    cx.notify();
                }))
                .into_any_element(),
        );
        actions.push(div().w(px(6.)).into_any_element());
        let copied = aside.clone();
        actions.push(
            icon_button("copy", "copy", "Copy, with the screenshots' paths", t)
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(copied.markdown()));
                    this.toast("Copied, with the screenshots' paths");
                    cx.notify();
                }))
                .into_any_element(),
        );
        let dir = aside.dir.clone();
        actions.push(
            icon_button("reveal", "folder", "Show in Finder", t)
                .on_click(move |_, _, _| {
                    let _ = Command::new("open").arg("-R").arg(dir.join(history::MESSAGE_FILE)).status();
                })
                .into_any_element(),
        );
        let deleted = aside.clone();
        actions.push(
            icon_button("delete", "trash", "Delete", t)
                .on_click(cx.listener(move |this, _, _, cx| {
                    match history::delete(&deleted) {
                        Ok(()) => this.toast("Deleted"),
                        Err(err) => this.toast(format!("Could not delete it: {err}")),
                    }
                    this.reload();
                    cx.notify();
                }))
                .into_any_element(),
        );
        // Where it went, each time, or that it's only in History.
        let status = |text: String, color: Hsla| div().text_color(color).child(text).into_any_element();
        let mut deliveries: Vec<AnyElement> = aside
            .record
            .deliveries
            .iter()
            .map(|d| {
                let when = history_time(d.at_ms);
                match &d.error {
                    None => status(format!("Sent to {} in {} at {when}", d.to, d.app), t.secondary),
                    Some(error) => status(format!("Not sent to {} at {when}: {error}", d.to), t.danger),
                }
            })
            .collect();
        if deliveries.is_empty() {
            deliveries.push(status("Only saved in History, not sent".into(), t.tertiary));
        }
        // The reading column: the window less the sidebar, the list and the margins.
        let column = (self.width - Metrics::SIDEBAR - LIST - 58.).clamp(200., Metrics::FORM + 60.);
        let body = aside_body(t, aside, column);
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(toolbar_row(None, actions))
            .child(scrolled(
                div().id("aside").size_full().overflow_y_scroll().track_scroll(&self.aside_scroll).child(
                    div().w_full().flex().justify_center().px(px(28.)).pb(px(32.)).child(
                        div()
                            .w_full()
                            .min_w_0()
                            .max_w(px(Metrics::FORM + 60.))
                            .flex()
                            .flex_col()
                            .gap(px(16.))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(3.))
                                    .child(
                                        div()
                                            .text_size(px(Type::DISPLAY))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(aside.created().format("%A %-d %B, %H:%M").to_string()),
                                    )
                                    .children(deliveries),
                            )
                            .children(body),
                    ),
                ),
                "aside-bar".into(),
                &self.aside_scroll,
                t,
            ))
            .into_any_element()
    }

    // --- Sessions ---

    fn sessions(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let mut found = self.state.sessions.clone();
        found.sort_by_key(|s| std::cmp::Reverse(s.last_input));
        let default = sessions::latest(&found).map(AgentSession::id);
        let labels = sessions::labels(&found);
        let mut running: Vec<AnyElement> = found
            .iter()
            .zip(labels)
            .map(|(s, label)| {
                let typed = s.last_input.map(|at| format!(", typed into {}", ago(at))).unwrap_or_default();
                let detail = format!("{} (in {}{typed})", tilde(&s.dir), s.host.name());
                let mark = match (s.host.reachable(), Some(s.id()) == default) {
                    (false, _) => Some(chip("Can't type there", t.attention)),
                    (true, true) => Some(chip("Gets asides", t.fresh)),
                    (true, false) => None,
                };
                div()
                    .min_h(px(Metrics::SETTINGS_ROW))
                    .px(px(12.))
                    .py(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(tile(t, "terminal"))
                    .child(label_stack(label, Some(detail.into()), t))
                    .children(mark)
                    .into_any_element()
            })
            .collect();
        if running.is_empty() {
            running.push(setting(
                "No agent sessions running",
                Some("Start Claude Code, Codex or OpenCode in Terminal, Ghostty or Diri, or start one from the pill: New session in…"),
                div(),
                t,
            ));
        }
        let terminals = deliver::installed_terminals();
        let current =
            if self.config.agents.terminal.is_empty() { "Terminal" } else { self.config.agents.terminal.as_str() };
        let chosen_terminal = terminals.iter().position(|n| n.eq_ignore_ascii_case(current));
        let terminal_choices = terminals.iter().map(|n| Choice::new(*n)).collect();
        let pick_terminal = self.setter(cx, move |c, i: usize| {
            let name = terminals[i];
            c.agents.terminal = if name == "Terminal" { String::new() } else { name.to_string() };
        });
        let agents = &AgentKind::ALL[..3];
        let chosen_agent = agents.iter().position(|a| *a == self.config.agents.default_agent());
        let pick_agent = self.setter(cx, |c, i: usize| c.agents.default = AgentKind::ALL[i].program().into());
        // Each agent's arguments, for the sessions asides starts.
        let arguments: Vec<AnyElement> = AgentKind::ALL
            .into_iter()
            .map(|kind| {
                let value = join_args(self.config.agents.args(kind));
                let field = self.field(args_field(kind), value, "None", 240., window, cx);
                setting(kind.name(), None, field, t)
            })
            .collect();
        page(
            "sessions",
            &self.page_scroll,
            self.theme,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(page_header(
                    "Sessions",
                    Some("asides types each aside into the session you typed into last. Tab, or the pill's list, picks another.".into()),
                    t,
                ))
                .child(section("Running", t).child(rows(running, t)))
                .child(
                    section("New sessions", t).child(rows(
                        [
                            setting(
                                "Open them in",
                                Some("Where the pill's New session in… starts one."),
                                select("terminal", terminal_choices, chosen_terminal, t)
                                    .width(150.)
                                    .on_select(pick_terminal),
                                t,
                            ),
                            setting(
                                "Agent",
                                Some("It starts with the aside as its first message."),
                                segmented("agent", agents.iter().map(|a| a.name()), chosen_agent, t).on_select(pick_agent),
                                t,
                            ),
                        ],
                        t,
                    )),
                )
                .child(
                    section("Arguments", t)
                        .child(rows(arguments, t))
                        .child(footnote("Added to each new session's command, like --model opus.", t)),
                ),
        )
    }

    // --- Keys ---

    fn keys(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let defaults = OverlayKeys::default();
        let mut sections: Vec<AnyElement> = Vec::new();
        for (group, keys) in KEY_GROUPS {
            let rows_of: Vec<AnyElement> = keys
                .iter()
                .map(|&(name, label)| {
                    let current = self.config.measure.keys.get(name).cloned().unwrap_or_default();
                    let labels = keys::parse(&current).iter().map(Chord::label).collect();
                    let recorder = self.recorder(name, labels, Recording::Key(name), cx);
                    // Changed: a way back to what it was.
                    let default = defaults.get(name).cloned().unwrap_or_default();
                    let reset = (keys::parse(&current) != keys::parse(&default)).then(|| {
                        icon_button(SharedString::from(format!("reset-{name}")), "undo", "As it was", t).on_click(
                            cx.listener(move |this, _, _, cx| {
                                let default = default.clone();
                                this.change(|c| {
                                    if let Some(keys) = c.measure.keys.get_mut(name) {
                                        *keys = default;
                                    }
                                });
                                cx.notify();
                            }),
                        )
                    });
                    setting(label, None, div().flex().items_center().gap(px(4.)).children(reset).child(recorder), t)
                })
                .collect();
            sections.push(section(group, t).child(rows(rows_of, t)).into_any_element());
        }
        let changed = OverlayKeys::NAMES.iter().any(|n| self.config.measure.keys.get(n) != defaults.get(n));
        let reset_all = changed.then(|| {
            button("reset-keys", "All as they were", t)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.change(|c| c.measure.keys = OverlayKeys::default());
                    cx.notify();
                }))
                .into_any_element()
        });
        page(
            "keys",
            &self.page_scroll,
            self.theme,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(page_header("Keys", Some("On the frozen screen. Click one, then press the keys.".into()), t))
                .children(sections)
                .children(reset_all.map(|b| div().flex().justify_end().child(b))),
        )
    }

    // --- Settings ---

    fn general(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let c = &self.config;
        let keep = [7, 30, 90, 0];
        let keep_labels = ["7 days", "30 days", "90 days", "Always"];
        let pick_keep = self.setter(cx, move |c, i: usize| c.history_days = keep[i]);
        let audio = self.setter(cx, |c, on| c.keep_audio = on);
        let sounds = self.setter(cx, |c, on| c.sounds = on);
        let wispr = self.setter(cx, |c, on| c.wispr_flow = on);
        let auto_update = self.setter(cx, |c, on| c.auto_update = on);
        let shortcut = |s: &str| if s.trim().is_empty() { Vec::new() } else { vec![pretty_shortcut(s)] };
        let open = self.recorder("record-asides", shortcut(&c.shortcuts.asides), Recording::Asides, cx);
        let freeze = self.recorder("record-freeze", shortcut(&c.shortcuts.freeze), Recording::Freeze, cx);
        let c = &self.config;
        let permission = |id: &'static str, name: &'static str, what: &'static str, pane: &'static str| {
            setting(
                name,
                Some(what),
                button(id, "Open…", t).on_click(move |_, _, _| {
                    open_url(&format!("x-apple.systempreferences:com.apple.preference.security?Privacy_{pane}"))
                }),
                t,
            )
        };
        page(
            "general",
            &self.page_scroll,
            self.theme,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(page_header("General", Some("asides picks up changes at once.".into()), t))
                .child(
                    section("Shortcuts", t)
                        .child(rows(
                            [
                                setting(
                                    "Open asides, and send",
                                    Some("Freezes the screen; pressed again, sends what you made."),
                                    open,
                                    t,
                                ),
                                setting(
                                    "Frozen or live",
                                    Some("Switches between the frozen screen and the live one; with asides closed, opens it live."),
                                    freeze,
                                    t,
                                ),
                            ],
                            t,
                        ))
                        .child(footnote("Click one, then press the keys. The frozen screen's are under Keys.", t)),
                )
                .child(
                    section("History", t).child(rows(
                        [
                            setting(
                                "Keep asides for",
                                Some("In ~/.asides/history."),
                                select("keep", keep_labels.map(Choice::new).to_vec(), keep.iter().position(|d| *d == c.history_days), t)
                                    .width(120.)
                                    .on_select(pick_keep),
                                t,
                            ),
                            setting(
                                "Keep what was said as audio",
                                Some("An .m4a with each aside."),
                                switch("keep-audio", c.keep_audio, t).on_toggle(audio),
                                t,
                            ),
                        ],
                        t,
                    )),
                )
                .child(
                    section("Sounds and dictation", t).child(rows(
                        [
                            setting(
                                "Sounds",
                                Some("When asides listens, sends, or something goes wrong."),
                                switch("sounds", c.sounds, t).on_toggle(sounds),
                                t,
                            ),
                            setting(
                                "Wispr Flow dictations",
                                Some("What you dictate with Wispr Flow while the screen is frozen joins the aside."),
                                switch("wispr", c.wispr_flow, t).on_toggle(wispr),
                                t,
                            ),
                        ],
                        t,
                    )),
                )
                .child(
                    section("Updates", t).child(rows(
                        [
                            setting(
                                "Update automatically",
                                Some("Looks for a new asides every 6 hours, downloads it, and installs it when asides quits."),
                                switch("auto-update", c.auto_update, t).on_toggle(auto_update),
                                t,
                            ),
                            self.update_row(cx),
                        ],
                        t,
                    )),
                )
                .child(
                    section("Permissions", t)
                        .child(rows(
                            [
                                permission("perm-mic", "Microphone", "To hear what you say.", "Microphone"),
                                permission("perm-screen", "Screen Recording", "To freeze the screen.", "ScreenCapture"),
                                permission("perm-ax", "Accessibility", "To name what's under each pin.", "Accessibility"),
                                permission("perm-auto", "Automation", "To type into Terminal and Ghostty.", "Automation"),
                            ],
                            t,
                        ))
                        .child(footnote("macOS asks for each the first time asides needs it.", t)),
                ),
        )
    }

    /// asides' version, where its update is, and the button that moves it on.
    fn update_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let update = &self.state.update;
        let version = update.release.as_ref().map_or("", |r| r.version.as_str());
        let detail = match update.phase {
            Phase::Idle | Phase::UpToDate => match update.checked_ms {
                Some(ms) => {
                    format!("Up to date (checked {}).", ago(SystemTime::UNIX_EPOCH + Duration::from_millis(ms)))
                }
                None => "Not checked yet.".into(),
            },
            Phase::Available => format!("asides {version} is out."),
            Phase::Ready => format!("asides {version} is ready: restart to update."),
            _ => format!("{}.", update.summary().trim_end_matches('.')),
        };
        let step = |step: Step| {
            move |_: &gpui::ClickEvent, _: &mut Window, _: &mut App| {
                let _ = send_link(&Link::Update { step, request: None });
            }
        };
        let main = match update.phase {
            Phase::Available => button("update-now", "Update", t).accent().on_click(step(Step::Install)),
            Phase::Ready => button("update-now", "Restart to Update", t).accent().on_click(step(Step::Install)),
            phase => button("update-check", "Check Now", t)
                .disabled(matches!(phase, Phase::Off | Phase::Checking | Phase::Downloading | Phase::Installing))
                .on_click(step(Step::Check)),
        };
        let notes = !update.notes.is_empty() || update.release.is_some();
        let whats_new = notes.then(|| {
            button("whats-new", "What's New", t).on_click(cx.listener(|this, _, _, cx| this.go(Page::WhatsNew, cx)))
        });
        setting(
            format!("asides {}", env!("CARGO_PKG_VERSION")),
            Some(&detail),
            div().flex().gap(px(8.)).children(whats_new).child(main),
            t,
        )
    }

    // --- What's New ---

    /// The update found (with its button), then what came in this asides and the ones before.
    fn whats_new(&mut self) -> AnyElement {
        let t = self.theme;
        let update = &self.state.update;
        let pending = matches!(update.phase, Phase::Available | Phase::Downloading | Phase::Ready | Phase::Installing);
        let offered = update.release.as_ref().filter(|_| pending).map(|release| {
            let action = match update.phase {
                Phase::Available => button("update-now", "Update", t).accent(),
                Phase::Ready => button("update-now", "Restart to Update", t).accent(),
                _ => button("update-now", update.summary(), t).disabled(true),
            };
            let action = action.on_click(|_, _, _| {
                let _ = send_link(&Link::Update { step: Step::Install, request: None });
            });
            let title = if update.phase == Phase::Available { "Update available" } else { "Ready to install" };
            section(title, t).child(release_card(t, release, Some(action.into_any_element())))
        });
        let installed: Vec<AnyElement> = update
            .notes
            .iter()
            .enumerate()
            .map(|(i, notes)| {
                let title = if i == 0 && notes.version == env!("CARGO_PKG_VERSION") { "This version" } else { "" };
                section(title, t).child(release_card(t, notes, None)).into_any_element()
            })
            .collect();
        if offered.is_none() && installed.is_empty() {
            return empty(t, "sparkles", "No release notes yet", "They show here once asides has looked for updates.");
        }
        page(
            "whats-new",
            &self.page_scroll,
            t,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(page_header("What's new", Some(format!("asides {}", env!("CARGO_PKG_VERSION")).into()), t))
                .children(offered)
                .children(installed),
        )
    }

    fn voice(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let chosen = ModelSpec::find(&self.config.model);
        // Only the multilingual models choose between languages.
        let languages = (!chosen.english).then(|| {
            let codes = self.config.languages.join(", ");
            let field = self.field("languages", codes, "Any", 160., window, cx);
            setting(
                "Languages",
                Some("What you speak, as codes like en, ro: each recording is transcribed in the likeliest."),
                field,
                t,
            )
        });
        let c = &self.config;
        let downloading = (self.state.status == "downloading").then(|| self.state.detail.clone()).flatten();
        let choices = MODELS
            .iter()
            .map(|m| {
                let state = if m.downloaded() { "downloaded".to_string() } else { m.size_label() };
                Choice::new(model_name(m)).detail(state)
            })
            .collect();
        let pick_model = self.setter(cx, |c, i: usize| c.model = MODELS[i].id.into());
        let model_state: AnyElement = if chosen.downloaded() {
            chip("Downloaded", t.fresh).into_any_element()
        } else if let Some(progress) = downloading {
            div().text_size(px(12.)).text_color(t.tertiary).child(progress).into_any_element()
        } else {
            let id = chosen.id;
            button("download", format!("Download {}", chosen.size_label()), t)
                .accent()
                .on_click(cx.listener(move |this, _, _, cx| {
                    let _ = send_link(&Link::Download { model: id.into(), request: None });
                    this.toast("Downloading…");
                    cx.notify();
                }))
                .into_any_element()
        };
        let devices: Vec<String> = std::iter::once(String::new()).chain(self.state.input_devices.clone()).collect();
        let chosen_device = devices.iter().position(|d| *d == c.input_device);
        let device_choices = devices
            .iter()
            .map(|d| Choice::new(if d.is_empty() { "System default".to_string() } else { d.clone() }))
            .collect();
        let pick_device = self.setter(cx, move |c, i: usize| c.input_device = devices[i].clone());
        let listen = self.setter(cx, |c, on| c.mic_on_open = on);
        page(
            "voice",
            &self.page_scroll,
            self.theme,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(page_header(
                    "Voice",
                    Some("What you say is transcribed on this Mac by Whisper, word for word.".into()),
                    t,
                ))
                .child(
                    section("Model", t)
                        .child(rows(
                            [
                                setting(
                                    "Voice model",
                                    Some("English models never take your words for another language."),
                                    select("model", choices, MODELS.iter().position(|m| m.id == chosen.id), t)
                                        .width(220.)
                                        .on_select(pick_model),
                                    t,
                                ),
                                setting(model_name(chosen), Some(model_note(chosen)), model_state, t),
                            ]
                            .into_iter()
                            .chain(languages),
                            t,
                        ))
                        .child(footnote(
                            "Downloaded only when you ask, into ~/Library/Application Support/asides/models.",
                            t,
                        )),
                )
                .child(section("Microphone", t).child(rows(
                    [
                        setting(
                            "Input",
                            None,
                            select("device", device_choices, chosen_device, t).width(220.).on_select(pick_device),
                            t,
                        ),
                        setting(
                            "Listen as soon as asides opens",
                            Some("The pill's microphone, and holding Space, still turn it on and off."),
                            switch("mic-on-open", c.mic_on_open, t).on_toggle(listen),
                            t,
                        ),
                    ],
                    t,
                ))),
        )
    }

    fn pins(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let color = self.field("color", self.config.measure.color.clone(), "#FF2D55", 92., window, cx);
        let c = &self.config;
        let picked =
            COLORS.iter().position(|rgb| color_hex(&format!("{rgb:06x}")).as_deref() == Some(c.measure.color.as_str()));
        let pick_color = self.setter(cx, |c, i: usize| c.measure.color = format!("#{:06X}", COLORS[i]));
        let tolerance = self.setter(cx, |c, step: i32| {
            c.measure.tolerance = (c.measure.tolerance as i32 + step).clamp(0, 100) as u8;
        });
        let padding = self
            .setter(cx, |c, step: i32| c.measure.padding = (c.measure.padding + 8.0 * step as f64).clamp(0.0, 200.0));
        let details = [Detail::Compact, Detail::Standard, Detail::Detailed, Detail::Forensic];
        let identify = self.setter(cx, |c, on| c.measure.identify_elements = on);
        let pick_detail = self.setter(cx, move |c, i: usize| c.annotation_detail = details[i]);
        let pick_units = self.setter(cx, |c, i: usize| c.measure.units = if i == 0 { Units::Pt } else { Units::Px });
        page(
            "pins",
            &self.page_scroll,
            self.theme,
            div()
                .flex()
                .flex_col()
                .gap(px(20.))
                .child(page_header(
                    "Pins",
                    Some("Click to pin what's under the pointer; drag to pin an area.".into()),
                    t,
                ))
                .child(section("", t).child(rows(
                    [
                        setting(
                            "Name what's under each pin",
                            Some("Its role and label, and in browsers the page and a CSS selector. Needs Accessibility."),
                            switch("identify", c.measure.identify_elements, t).on_toggle(identify),
                            t,
                        ),
                        setting(
                            "How much each pin says",
                            None,
                            segmented(
                                "detail",
                                ["Compact", "Standard", "Detailed", "Forensic"],
                                details.iter().position(|d| *d == c.annotation_detail),
                                t,
                            )
                            .on_select(pick_detail),
                            t,
                        ),
                        setting(
                            "Measure in",
                            None,
                            segmented("units", ["Points", "Pixels"], Some(usize::from(c.measure.units == Units::Px)), t)
                                .on_select(pick_units),
                            t,
                        ),
                    ],
                    t,
                )))
                .child(section("Look", t).child(rows(
                    [
                        setting(
                            "Color",
                            Some("Of measurements, guides and drawings."),
                            div()
                                .flex()
                                .items_center()
                                .gap(px(10.))
                                .child(swatches("color", &COLORS, picked, t, pick_color))
                                .child(color),
                            t,
                        ),
                        setting(
                            "Room around a pin",
                            Some("How much of the screen around what you pinned its screenshot keeps."),
                            stepper("padding", format!("{} pt", c.measure.padding.round()), t, padding),
                            t,
                        ),
                    ],
                    t,
                )))
                .child(section("Measuring", t).child(rows(
                    [setting(
                        "Tolerance",
                        Some("How different two pixels must be to make an edge. - and = change it on the frozen screen."),
                        stepper("tolerance", format!("{}%", c.measure.tolerance), t, tolerance),
                        t,
                    )],
                    t,
                ))),
        )
    }
}

/// A page: the strip along the title bar, then its content in a column, scrolling.
fn page(id: &'static str, scroll: &ScrollHandle, t: Theme, content: impl IntoElement) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .child(div().flex_none().h(px(Metrics::TOOLBAR)))
        .child(scrolled(
            div().id(id).size_full().overflow_y_scroll().track_scroll(scroll).child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .px(px(28.))
                    .pb(px(32.))
                    .child(div().w_full().min_w_0().max_w(px(Metrics::FORM + 60.)).child(content)),
            ),
            SharedString::from(format!("{id}-bar")),
            scroll,
            t,
        ))
        .into_any_element()
}

/// A scroll area, filling what's left, with its bar beside it.
fn scrolled(area: impl IntoElement, bar: SharedString, scroll: &ScrollHandle, t: Theme) -> impl IntoElement {
    div().flex_1().min_h_0().relative().child(area).child(scrollbar(bar, scroll, t))
}

/// A page with nothing on it yet: what it'll show, and how.
fn empty(t: Theme, glyph: &str, title: &str, text: &str) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .p(px(40.))
        .child(tile(t, glyph))
        .child(div().pt(px(4.)).text_size(px(Type::TITLE)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .child(div().max_w(px(380.)).text_color(t.tertiary).text_center().child(text.to_string()))
        .into_any_element()
}

/// An icon on a quiet square.
fn tile(t: Theme, glyph: &str) -> impl IntoElement + use<> {
    div()
        .flex_none()
        .size(px(28.))
        .rounded(px(Radius::ROW))
        .bg(t.primary.opacity(0.06))
        .flex()
        .items_center()
        .justify_center()
        .child(icon(glyph, 15., t.secondary))
}

/// "English, more accurate" (the catalog's label without its size).
fn model_name(m: &ModelSpec) -> String {
    m.label.split(" (").next().unwrap_or(m.label).to_string()
}

fn model_note(m: &ModelSpec) -> &'static str {
    if m.english { "Transcribes English only." } else { "Transcribes any language it hears." }
}

/// Where an aside last went, for the list: its icon, its text, and whether it failed.
fn delivered(aside: &Aside) -> (&'static str, String, bool) {
    match aside.last_delivery() {
        Some(d) if d.error.is_none() => ("send", format!("Sent to {}", d.to), false),
        Some(d) => ("close", format!("Not sent to {}", d.to), true),
        None => ("clock", "Only saved in History".into(), false),
    }
}

/// What an aside says, piece by piece, as History shows it.
#[derive(Debug, PartialEq)]
enum Piece {
    /// Where the user was: the app, and its window and page.
    Place {
        app: String,
        window: Option<String>,
        page: Option<String>,
    },
    /// What was said, with the pins' numbers (`[2]`) where they were placed.
    Said(String),
    Image(String),
    /// A pin: its number, what it's on, and its details (label, value).
    Pin {
        number: u32,
        title: String,
        fields: Vec<(String, String)>,
    },
    /// How it was measured (label, value), and what.
    Measured {
        fields: Vec<(String, String)>,
        items: Vec<String>,
    },
}

fn plain(text: &str) -> String {
    text.replace("**", "").replace('`', "")
}

/// `**Label:** value` (or `**Measured** in pt`), as (label, value).
fn field_of(line: &str) -> Option<(String, String)> {
    let (label, value) = line.trim().strip_prefix("**")?.split_once("**")?;
    Some((label.trim_end_matches(':').to_string(), plain(value.trim())))
}

/// `Safari ("Settings — Shop", http://localhost:3000)`: the app, the window, the page.
fn place_of(text: &str) -> Piece {
    let (app, rest) = text.split_once(" (").map_or((text, ""), |(app, rest)| (app, rest.trim_end_matches(')')));
    let (window, page) = match rest.strip_prefix('"').and_then(|r| r.split_once('"')) {
        Some((window, page)) => (Some(window.to_string()), page.trim_start_matches(", ").to_string()),
        None => (None, rest.to_string()),
    };
    Piece::Place { app: app.to_string(), window, page: (!page.is_empty()).then_some(page) }
}

/// An aside's message, in pieces.
fn pieces(parts: Vec<Part>) -> Vec<Piece> {
    let mut pieces = Vec::new();
    for part in parts {
        let text = match part {
            Part::Image(path) => {
                pieces.push(Piece::Image(path));
                continue;
            }
            Part::Text(text) => text,
        };
        for block in text.split("\n\n").map(str::trim).filter(|b| !b.is_empty()) {
            let lines: Vec<&str> = block.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            if let Some(number) = asides_core::markdown::pin_number(block) {
                let head = lines[0].trim_start_matches("### ");
                let title = head.split_once(". ").map_or(head, |(_, title)| title);
                let fields = lines[1..].iter().filter_map(|l| field_of(l)).collect();
                pieces.push(Piece::Pin { number, title: plain(title), fields });
            } else if lines.iter().all(|l| l.starts_with("- ")) {
                let items = lines.iter().map(|l| plain(&l[2..])).collect();
                match pieces.last_mut() {
                    Some(Piece::Measured { items: list, .. }) => list.extend(items),
                    _ => pieces.push(Piece::Measured { fields: Vec::new(), items }),
                }
            } else if lines.iter().all(|l| field_of(l).is_some()) {
                let mut fields = Vec::new();
                for line in lines {
                    match line.strip_prefix(asides_core::inspect::PLACE) {
                        Some(place) => pieces.push(place_of(place)),
                        None => fields.extend(field_of(line)),
                    }
                }
                if !fields.is_empty() {
                    pieces.push(Piece::Measured { fields, items: Vec::new() });
                }
            } else {
                pieces.push(Piece::Said(plain(&lines.join(" "))));
            }
        }
    }
    pieces
}

/// A pin's color: as on its screenshot, else its type's, else an element's.
fn pin_color(aside: &Aside, number: u32, fields: &[(String, String)]) -> Hsla {
    let kept = aside.record.pins.iter().find(|p| p.number == number);
    let intent = fields.iter().find(|(l, _)| l == "Intent").and_then(|(_, v)| intent_named(v));
    let value = match (kept, intent) {
        (Some(pin), _) => style::rgb_hex(style::parse_color(&pin.color)),
        (None, Some(intent)) => style::intent_color(intent),
        _ => style::ANNOTATION_BLUE,
    };
    hex(value, 1.0)
}

fn intent_named(name: &str) -> Option<Intent> {
    [Intent::Fix, Intent::Change, Intent::Question, Intent::Approve].into_iter().find(|i| i.as_str() == name)
}

/// An aside as History shows it: where it was, what was said with its pins' numbers, the
/// screenshots, each pin's card and what was measured.
fn aside_body(t: Theme, aside: &Aside, column: f32) -> Vec<AnyElement> {
    let pieces = pieces(aside.parts());
    let colors: HashMap<u32, Hsla> = pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Pin { number, fields, .. } => Some((*number, pin_color(aside, *number, fields))),
            _ => None,
        })
        .collect();
    let mut body: Vec<AnyElement> = Vec::new();
    let mut pins: Vec<AnyElement> = Vec::new();
    let card = |children: Vec<AnyElement>| {
        div()
            .flex()
            .flex_col()
            .rounded(px(Radius::CARD))
            .border_1()
            .border_color(t.card_stroke)
            .bg(t.card)
            .children(children.into_iter().enumerate().map(|(i, row)| {
                div().flex().flex_col().when(i > 0, |d| d.border_t_1().border_color(t.hairline)).child(row)
            }))
            .into_any_element()
    };
    for piece in pieces {
        if !matches!(piece, Piece::Pin { .. }) && !pins.is_empty() {
            body.push(card(std::mem::take(&mut pins)));
        }
        match piece {
            Piece::Place { app, window, page } => body.push(card(vec![place(t, app, window, page)])),
            Piece::Said(text) => body.push(said(t, text, &colors)),
            Piece::Image(path) => body.push(screenshot(t, path, column)),
            Piece::Pin { number, title, fields } => {
                let color = colors.get(&number).copied().unwrap_or(hex(style::ANNOTATION_BLUE, 1.0));
                pins.push(pin_card(t, number, title, fields, color));
            }
            Piece::Measured { fields, items } => {
                let rows = fields
                    .into_iter()
                    .map(|(label, value)| detail_row(t, label, value).into_any_element())
                    .chain(items.into_iter().map(|item| {
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(div().flex_none().pt(px(2.)).child(icon("measure", 13., t.tertiary)))
                            .child(div().flex_1().min_w_0().text_color(t.secondary).child(item))
                            .into_any_element()
                    }))
                    .collect::<Vec<_>>();
                body.push(card(vec![
                    div().flex().flex_col().gap(px(6.)).px(px(12.)).py(px(10.)).children(rows).into_any_element(),
                ]));
            }
        }
    }
    if !pins.is_empty() {
        body.push(card(pins));
    }
    body
}

/// Where the user was: the app, its window and the page, as a pin's details.
fn place(t: Theme, app: String, window: Option<String>, page: Option<String>) -> AnyElement {
    let rows = [Some(("App", app)), window.map(|w| ("Window", w)), page.map(|p| ("Page", short_address(&p)))];
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(12.))
        .py(px(10.))
        .children(rows.into_iter().flatten().map(|(label, value)| detail_row(t, label.into(), value)))
        .into_any_element()
}

/// A page's address without its scheme or last slash, cut short (GPUI's own "…" doesn't fit text to
/// a width it learns later).
fn short_address(url: &str) -> String {
    let url = url.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/');
    match url.chars().count() > 90 {
        true => format!("{}…", url.chars().take(89).collect::<String>()),
        false => url.to_string(),
    }
}

/// What was said, its pins' numbers in their colors.
fn said(t: Theme, text: String, colors: &HashMap<u32, Hsla>) -> AnyElement {
    let mut highlights = Vec::new();
    let mut from = 0;
    while let Some(open) = text[from..].find('[').map(|i| from + i) {
        let Some(close) = text[open..].find(']').map(|i| open + i) else { break };
        if let Some(color) = text[open + 1..close].parse::<u32>().ok().and_then(|n| colors.get(&n)) {
            let style =
                gpui::HighlightStyle { color: Some(*color), font_weight: Some(FontWeight::BOLD), ..Default::default() };
            highlights.push((open..close + 1, style));
        }
        from = close + 1;
    }
    div()
        .text_size(px(14.))
        .line_height(px(22.))
        .text_color(t.primary)
        .child(gpui::StyledText::new(text).with_highlights(highlights))
        .into_any_element()
}

/// A screenshot: as wide as the column, at most its own size (in points); as tall as that makes it.
fn screenshot(t: Theme, path: String, column: f32) -> AnyElement {
    let (w, h) = png_size(Path::new(&path)).unwrap_or((1120, 560));
    let width = (w as f32 / 2.).min(column);
    div()
        .w(px(width))
        .h(px(width * h as f32 / w.max(1) as f32))
        .rounded(px(Radius::CARD))
        .overflow_hidden()
        .border_1()
        .border_color(t.card_stroke)
        .child(img(ImageSource::from(PathBuf::from(path))).size_full())
        .into_any_element()
}

/// One pin: its number in its color and what it's on, its type, what was typed or said about it,
/// then where it is.
fn pin_card(t: Theme, number: u32, title: String, fields: Vec<(String, String)>, color: Hsla) -> AnyElement {
    let mut tags = Vec::new();
    let mut notes = Vec::new();
    let mut details = Vec::new();
    for (label, value) in fields {
        match label.as_str() {
            "Intent" | "Severity" => tags.push(value),
            "Feedback" | "Said" => notes.push((label, value)),
            // Where it goes first: it's what's asked.
            "Move to" => details.insert(0, (label, value)),
            _ => details.push((label, value)),
        }
    }
    let badge = div()
        .flex_none()
        .size(px(20.))
        .rounded_full()
        .bg(color)
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .text_color(hex(0xffffff, 1.0))
        .child(number.to_string());
    // Its type with the type's dot (as on its card); its severity alone.
    let tag = |text: String| {
        div()
            .flex_none()
            .flex()
            .items_center()
            .gap(px(5.))
            .px(px(7.))
            .py(px(1.))
            .rounded(px(Radius::CHIP))
            .bg(t.hover)
            .text_size(px(Type::META))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.secondary)
            .children(
                intent_named(&text).map(|i| div().size(px(7.)).rounded_full().bg(hex(style::intent_color(i), 1.0))),
            )
            .child(capitalized(&text))
    };
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap(px(6.))
        .px(px(12.))
        .py(px(10.))
        .child(
            div()
                .w_full()
                .flex()
                .items_start()
                .gap(px(10.))
                .child(badge)
                .child(div().flex_1().min_w_0().pt(px(1.)).font_weight(FontWeight::SEMIBOLD).child(title))
                .children(tags.into_iter().map(tag)),
        )
        // What was typed, and said (the microphone under its number).
        .children(notes.into_iter().map(|(label, value)| {
            div()
                .w_full()
                .flex()
                .gap(px(10.))
                .text_color(t.primary)
                .child(
                    div()
                        .flex_none()
                        .w(px(20.))
                        .pt(px(3.))
                        .flex()
                        .justify_center()
                        .when(label == "Said", |d| d.child(icon("mic", 12., t.tertiary))),
                )
                .child(div().flex_1().min_w_0().child(format!("“{value}”")))
        }))
        .when(!details.is_empty(), |d| {
            d.child(
                div()
                    .pl(px(30.))
                    .pt(px(2.))
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .children(details.into_iter().map(|(label, value)| detail_row(t, label, value))),
            )
        })
        .into_any_element()
}

/// A detail: its label in a column of its own, the value wrapping beside it.
fn detail_row(t: Theme, label: String, value: String) -> impl IntoElement {
    div()
        .flex()
        .gap(px(10.))
        .text_size(px(12.))
        .line_height(px(17.))
        .child(div().flex_none().w(px(92.)).text_color(t.tertiary).child(label))
        .child(div().flex_1().min_w_0().text_color(t.secondary).child(value))
}

fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map(|c| c.to_uppercase().chain(chars).collect()).unwrap_or_default()
}

/// A PNG's size in pixels, from its header.
fn png_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut header = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut header).ok()?;
    let be = |at: usize| u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]]);
    (&header[..8] == b"\x89PNG\r\n\x1a\n").then(|| (be(16), be(20)))
}

fn history_time(ms: u64) -> String {
    Local.timestamp_millis_opt(ms as i64).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}
