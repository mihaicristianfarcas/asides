//! The Asides window (`asides-overlay --window`): History (every aside: what was sent where, sent
//! again, copied, revealed, deleted), Sessions (the agent sessions asides can go to) and Settings.
//! The app starts it from its menu (and `asides`); a line `show` on standard input brings it back.
//!
//! It reads what the app publishes (`state.json`), the history (`~/.asides/history`) and the
//! settings (`config.toml`, which the app reloads when it changes); what only the app can do (send,
//! download a voice model) it asks for with `asides://` links. Soft blue accent; macOS's warm gray
//! in dark mode, white in light mode.

use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant, SystemTime};

use asides_core::agents::AgentKind;
use asides_core::config::Config;
use asides_core::deliver;
use asides_core::history::{self, Aside};
use asides_core::inspect::{Detail, Units};
use asides_core::link::Link;
use asides_core::markdown::Part;
use asides_core::models::{MODELS, ModelSpec};
use asides_core::paths::{self, tilde};
use asides_core::sessions::{self, AgentSession};
use asides_core::state::AppState;
use gpui::{
    AnyElement, App, AppContext, Application, Bounds, Context, FontWeight, Hsla, ImageSource, Render, SharedString,
    TitlebarOptions, Window, WindowAppearance, WindowBounds, WindowOptions, div, img, point, prelude::*, px, size,
};

use crate::{assets, native, style};

/// How often the window looks at what changed (the history, the app's state, the settings).
const TICK: Duration = Duration::from_millis(500);
const TOAST: Duration = Duration::from_millis(2500);
const SIDEBAR: f32 = 196.0;
const LIST: f32 = 300.0;

#[derive(Clone, Copy, PartialEq)]
enum Section {
    History,
    Sessions,
    Settings,
}

/// The window's colors, for the system's appearance.
#[derive(Clone, Copy)]
struct Theme {
    bg: Hsla,
    sidebar: Hsla,
    surface: Hsla,
    hover: Hsla,
    text: Hsla,
    secondary: Hsla,
    border: Hsla,
    accent: Hsla,
    accent_ink: Hsla,
    danger: Hsla,
}

fn hex(value: u32) -> Hsla {
    Hsla::from(gpui::rgb(value))
}

impl Theme {
    fn of(appearance: WindowAppearance) -> Theme {
        let (accent, accent_ink, danger) = (hex(style::UI), hex(style::UI_INK), hex(0xe5484d));
        match appearance {
            // macOS's warm grays.
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme {
                bg: hex(0x262422),
                sidebar: hex(0x1f1e1c),
                surface: hex(0x302e2b),
                hover: hex(0x3a3835),
                text: hex(0xeceae6),
                secondary: hex(0xa39f98),
                border: hex(0x3b3936),
                accent,
                accent_ink,
                danger,
            },
            WindowAppearance::Light | WindowAppearance::VibrantLight => Theme {
                bg: hex(0xffffff),
                sidebar: hex(0xf6f5f3),
                surface: hex(0xf3f2f0),
                hover: hex(0xebeae7),
                text: hex(0x1d1c1a),
                secondary: hex(0x6e6a64),
                border: hex(0xe6e4e0),
                accent,
                accent_ink,
                danger,
            },
        }
    }
}

pub struct AsidesWindow {
    section: Section,
    asides: Vec<Aside>,
    selected: Option<String>,
    state: AppState,
    config: Config,
    /// When the settings file was last read.
    config_read: Option<SystemTime>,
    /// The Send to… list is open.
    send_menu: bool,
    toast: Option<(SharedString, Instant)>,
    commands: Receiver<String>,
    theme: Theme,
}

/// Opens the window; it quits when it's closed.
pub fn run() {
    native::stay_accessory_at_launch();
    Application::new().with_assets(assets::Assets).run(|cx: &mut App| {
        native::accessory();
        let (tx, commands) = channel();
        std::thread::spawn(move || {
            for line in std::io::stdin().lock().lines().map_while(Result::ok) {
                if tx.send(line.trim().to_string()).is_err() {
                    return;
                }
            }
        });
        let bounds = Bounds::centered(None, size(px(920.), px(620.)), cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Asides".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.), px(14.))),
            }),
            focus: true,
            show: true,
            window_min_size: Some(size(px(680.), px(440.))),
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
                AsidesWindow::new(commands, theme)
            })
        });
        let Ok(handle) = opened else {
            eprintln!("asides-overlay: could not open the Asides window");
            return cx.quit();
        };
        cx.on_window_closed(|cx| cx.quit()).detach();
        native::activate();
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

impl AsidesWindow {
    fn new(commands: Receiver<String>, theme: Theme) -> AsidesWindow {
        let mut this = AsidesWindow {
            section: Section::History,
            asides: Vec::new(),
            selected: None,
            state: AppState::default(),
            config: Config::default(),
            config_read: None,
            send_menu: false,
            toast: None,
            commands,
            theme,
        };
        this.reload();
        this.selected = this.asides.first().map(|a| a.id.clone());
        // Development: where it opens (to capture each part of it).
        match std::env::var("ASIDES_WINDOW_SECTION").as_deref() {
            Ok("sessions") => this.section = Section::Sessions,
            Ok("settings") => this.section = Section::Settings,
            _ => {}
        }
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
            if command == "show" {
                native::activate();
                window.activate_window();
            }
        }
        if self.toast.as_ref().is_some_and(|(_, until)| Instant::now() > *until) {
            self.toast = None;
        }
        self.reload();
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

/// An aside as Markdown with its screenshots' absolute paths, for the clipboard.
fn absolute_message(aside: &Aside) -> String {
    aside
        .parts()
        .into_iter()
        .map(|part| match part {
            Part::Text(text) => text,
            Part::Image(path) => format!("![Screenshot]({path})"),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
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

impl Render for AsidesWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let content = match self.section {
            Section::History => self.history(cx),
            Section::Sessions => self.sessions(cx),
            Section::Settings => self.settings(cx),
        };
        div()
            .size_full()
            .flex()
            .bg(t.bg)
            .text_color(t.text)
            .text_size(px(13.))
            .child(self.sidebar(cx))
            .child(div().flex_1().h_full().min_w_0().child(content))
            .children(self.toast.as_ref().map(|(text, _)| {
                div().absolute().bottom(px(16.)).left_0().right_0().flex().justify_center().child(
                    div()
                        .px(px(12.))
                        .py(px(6.))
                        .rounded(px(8.))
                        .bg(t.surface)
                        .border_1()
                        .border_color(t.border)
                        .shadow_md()
                        .child(text.clone()),
                )
            }))
    }
}

impl AsidesWindow {
    fn sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let item =
            |id: &'static str, label: &'static str, section: Section, count: Option<usize>, cx: &mut Context<Self>| {
                let on = self.section == section;
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .justify_between()
                    .h(px(30.))
                    .px(px(10.))
                    .rounded(px(7.))
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.accent).text_color(t.accent_ink).font_weight(FontWeight::MEDIUM))
                    .when(!on, |d| d.hover(|s| s.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.section = section;
                        this.send_menu = false;
                        cx.notify();
                    }))
                    .child(label)
                    .children(count.map(|n| div().text_size(px(11.5)).opacity(0.7).child(n.to_string())))
            };
        let status = match self.state.status.as_str() {
            "" => "Asides isn't running".to_string(),
            "idle" => "Ready".into(),
            "open" => "Asides is on screen".into(),
            "recording" => "Listening".into(),
            "transcribing" => "Transcribing…".into(),
            "sending" => "Sending…".into(),
            "downloading" => self.state.detail.clone().unwrap_or_else(|| "Downloading…".into()),
            _ => self.state.detail.clone().unwrap_or_default(),
        };
        let shortcut = asides_core::config::pretty_shortcut(&self.config.shortcuts.asides);
        div()
            .w(px(SIDEBAR))
            .h_full()
            .flex()
            .flex_col()
            .justify_between()
            .bg(t.sidebar)
            .border_r_1()
            .border_color(t.border)
            .p(px(10.))
            .pt(px(44.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(item("nav-history", "History", Section::History, Some(self.asides.len()), cx))
                    .child(item("nav-sessions", "Sessions", Section::Sessions, Some(self.state.sessions.len()), cx))
                    .child(item("nav-settings", "Settings", Section::Settings, None, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .text_size(px(11.5))
                    .text_color(t.secondary)
                    .child(status)
                    .child(format!("{shortcut} opens Asides")),
            )
            .into_any_element()
    }

    // --- History ---

    fn history(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        if self.asides.is_empty() {
            let shortcut = asides_core::config::pretty_shortcut(&self.config.shortcuts.asides);
            return empty(
                t,
                "No asides yet",
                &format!("Press {shortcut} over any app: click what's wrong, say why, and press {shortcut} again."),
            );
        }
        let rows: Vec<AnyElement> = self
            .asides
            .iter()
            .map(|aside| {
                let on = self.selected.as_ref() == Some(&aside.id);
                let id = aside.id.clone();
                let (where_, failed) = match aside.last_delivery() {
                    Some(d) if d.error.is_none() => (format!("→ {}", d.to), false),
                    Some(d) => (format!("Not sent to {}", d.to), true),
                    None => ("Kept in History".into(), false),
                };
                div()
                    .id(SharedString::from(format!("aside-{id}")))
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .px(px(12.))
                    .py(px(8.))
                    .border_b_1()
                    .border_color(t.border)
                    .cursor_pointer()
                    .when(on, |d| d.bg(t.accent).text_color(t.accent_ink))
                    .when(!on, |d| d.hover(|s| s.bg(t.hover)))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = Some(id.clone());
                        this.send_menu = false;
                        cx.notify();
                    }))
                    .child(div().truncate().font_weight(FontWeight::MEDIUM).child(aside.record.title.clone()))
                    .child(
                        div()
                            .flex()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .when(!on, |d| d.text_color(if failed { t.danger } else { t.secondary }))
                            .child(aside.created().format("%b %-d, %H:%M").to_string())
                            .child(div().truncate().child(where_)),
                    )
                    .into_any_element()
            })
            .collect();
        let detail = self.selected().cloned().map(|aside| self.aside_detail(&aside, cx));
        div()
            .size_full()
            .flex()
            .child(
                div()
                    .id("history-list")
                    .w(px(LIST))
                    .h_full()
                    .pt(px(38.))
                    .border_r_1()
                    .border_color(t.border)
                    .overflow_y_scroll()
                    .children(rows),
            )
            .child(div().flex_1().h_full().min_w_0().children(detail))
            .into_any_element()
    }

    fn aside_detail(&mut self, aside: &Aside, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let id = aside.id.clone();
        let reachable = self.reachable();
        let default = reachable.first().map(|(s, label)| (s.id(), label.clone()));
        let deliveries: Vec<AnyElement> = aside
            .record
            .deliveries
            .iter()
            .map(|d| {
                let when = history_time(d.at_ms);
                let text = match &d.error {
                    None => format!("{when} · sent to {} in {}", d.to, d.app),
                    Some(error) => format!("{when} · not sent to {}: {error}", d.to),
                };
                div()
                    .text_size(px(12.))
                    .text_color(if d.error.is_some() { t.danger } else { t.secondary })
                    .child(text)
                    .into_any_element()
            })
            .collect();
        let send_default = default.clone().map(|(session, label)| {
            let id = id.clone();
            button(
                t,
                "send-default",
                &format!("Send to {label}"),
                true,
                cx.listener(move |this, _, _, cx| {
                    let link = Link::Send { id: id.clone(), to: Some(session.clone()), request: None };
                    match send_link(&link) {
                        Ok(()) => this.toast("Sending…"),
                        Err(err) => this.toast(format!("Could not reach Asides: {err}")),
                    }
                    cx.notify();
                }),
            )
        });
        let more = (reachable.len() > 1).then(|| {
            button(
                t,
                "send-to",
                "Send to…",
                false,
                cx.listener(|this, _, _, cx| {
                    this.send_menu = !this.send_menu;
                    cx.notify();
                }),
            )
        });
        let menu = (self.send_menu).then(|| {
            let rows: Vec<AnyElement> = reachable
                .iter()
                .enumerate()
                .map(|(i, (session, label))| {
                    let (id, to) = (id.clone(), session.id());
                    let detail = format!("{} · {}", tilde(&session.dir), session.host.name());
                    div()
                        .id(SharedString::from(format!("send-{i}")))
                        .px(px(10.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.bg(t.accent).text_color(t.accent_ink))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let link = Link::Send { id: id.clone(), to: Some(to.clone()), request: None };
                            let _ = send_link(&link);
                            this.send_menu = false;
                            this.toast("Sending…");
                            cx.notify();
                        }))
                        .child(label.clone())
                        .child(div().text_size(px(11.)).opacity(0.65).child(detail))
                        .into_any_element()
                })
                .collect();
            div().mt(px(6.)).p(px(4.)).rounded(px(9.)).bg(t.surface).border_1().border_color(t.border).children(rows)
        });
        let copy = {
            let aside = aside.clone();
            button(
                t,
                "copy",
                "Copy",
                false,
                cx.listener(move |this, _, _, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(absolute_message(&aside)));
                    this.toast("Copied, with the screenshots' paths");
                    cx.notify();
                }),
            )
        };
        let reveal = {
            let dir = aside.dir.clone();
            button(
                t,
                "reveal",
                "Show in Finder",
                false,
                cx.listener(move |_, _, _, _| {
                    let _ = Command::new("open").arg("-R").arg(dir.join(history::MESSAGE_FILE)).status();
                }),
            )
        };
        let delete = {
            let aside = aside.clone();
            button(
                t,
                "delete",
                "Delete",
                false,
                cx.listener(move |this, _, _, cx| {
                    match history::delete(&aside) {
                        Ok(()) => this.toast("Deleted"),
                        Err(err) => this.toast(format!("Could not delete it: {err}")),
                    }
                    this.reload();
                    cx.notify();
                }),
            )
        };
        let body: Vec<AnyElement> = aside
            .parts()
            .into_iter()
            .map(|part| match part {
                Part::Text(text) => div()
                    .flex()
                    .flex_col()
                    .gap(px(3.))
                    .children(text.lines().map(|line| markdown_line(t, line)))
                    .into_any_element(),
                Part::Image(path) => {
                    // As wide as there's room for (at most its own size, in points), as tall as that makes it.
                    let (w, h) = png_size(Path::new(&path)).unwrap_or((1120, 560));
                    let width = (w as f32 / 2.0).min(560.0);
                    div()
                        .w(px(width))
                        .h(px(width * h as f32 / w.max(1) as f32))
                        .rounded(px(8.))
                        .overflow_hidden()
                        .border_1()
                        .border_color(t.border)
                        .child(img(ImageSource::from(PathBuf::from(path))).size_full())
                        .into_any_element()
                }
            })
            .collect();
        div()
            .id("aside")
            .size_full()
            .overflow_y_scroll()
            .p(px(20.))
            .pt(px(40.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(aside.created().format("%A %-d %B, %H:%M").to_string()),
                    )
                    .children(deliveries)
                    .when(aside.record.deliveries.is_empty(), |d| {
                        d.child(div().text_size(px(12.)).text_color(t.secondary).child("Kept in History, not sent"))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .children(send_default)
                    .children(more)
                    .child(copy)
                    .child(reveal)
                    .child(delete),
            )
            .children(menu)
            .when(default.is_none(), |d| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(t.secondary)
                        .child("No agent session that Asides can type into is running (Terminal, Ghostty or Diri)."),
                )
            })
            .child(div().h(px(1.)).bg(t.border))
            .children(body)
            .into_any_element()
    }

    // --- Sessions ---

    fn sessions(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let mut found = self.state.sessions.clone();
        found.sort_by_key(|s| std::cmp::Reverse(s.last_input));
        if found.is_empty() {
            return empty(
                t,
                "No agent sessions",
                "Start Claude Code, Codex or OpenCode in Terminal, Ghostty or Diri: Asides sends each aside to the \
                 session you typed into last. Asides can also start one: pick New session in… on its pill.",
            );
        }
        let default = sessions::latest(&found).map(AgentSession::id);
        let labels = sessions::labels(&found);
        let rows = found.iter().zip(labels).map(|(s, label)| {
            let note = match (s.host.reachable(), Some(s.id()) == default) {
                (false, _) => format!("In {}: Asides can't type there", s.host.name()),
                (true, true) => format!("In {} · asides go here (typed into last)", s.host.name()),
                (true, false) => format!("In {}", s.host.name()),
            };
            let typed = s.last_input.map(|at| format!("typed into {}", ago(at))).unwrap_or_default();
            div()
                .flex()
                .flex_col()
                .gap(px(3.))
                .p(px(12.))
                .rounded(px(10.))
                .bg(t.surface)
                .border_1()
                .border_color(if Some(s.id()) == default { t.accent } else { t.border })
                .when(!s.host.reachable(), |d| d.opacity(0.6))
                .child(div().font_weight(FontWeight::SEMIBOLD).child(label))
                .child(div().text_size(px(12.)).text_color(t.secondary).child(tilde(&s.dir)))
                .child(div().text_size(px(12.)).text_color(t.secondary).child(format!("{note} · {typed}")))
        });
        div()
            .id("sessions")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .pt(px(44.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(heading(
                t,
                "Sessions",
                "Asides sends each aside to the session you typed into last; its pill switches.",
            ))
            .children(rows)
            .into_any_element()
    }

    // --- Settings ---

    fn settings(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let t = self.theme;
        let c = self.config.clone();
        let downloading = (self.state.status == "downloading").then(|| self.state.detail.clone()).flatten();
        // Voice.
        let models = MODELS.iter().map(|m| {
            let id = m.id;
            let chosen = c.model == id || (ModelSpec::find(&c.model).id == id);
            let status: AnyElement = if m.downloaded() {
                div().text_size(px(12.)).text_color(t.secondary).child("Downloaded").into_any_element()
            } else if chosen && downloading.is_some() {
                div()
                    .text_size(px(12.))
                    .text_color(t.secondary)
                    .child(downloading.clone().unwrap_or_default())
                    .into_any_element()
            } else {
                button(
                    t,
                    &format!("download-{id}"),
                    "Download",
                    false,
                    cx.listener(move |this, _, _, cx| {
                        let _ = send_link(&Link::Download { model: id.into(), request: None });
                        this.toast("Downloading…");
                        cx.notify();
                    }),
                )
                .into_any_element()
            };
            div()
                .id(SharedString::from(format!("model-{id}")))
                .flex()
                .items_center()
                .justify_between()
                .gap(px(10.))
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .cursor_pointer()
                .when(chosen, |d| d.bg(t.surface))
                .hover(|s| s.bg(t.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(|c| c.model = id.into());
                    cx.notify();
                }))
                .child(div().flex().items_center().gap(px(8.)).child(radio(t, chosen)).child(m.label))
                .child(status)
        });
        let devices = std::iter::once(String::new()).chain(self.state.input_devices.clone()).map(|name| {
            let chosen = c.input_device == name;
            let label = if name.is_empty() { "System default".to_string() } else { name.clone() };
            div()
                .id(SharedString::from(format!("device-{label}")))
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(10.))
                .py(px(5.))
                .rounded(px(8.))
                .cursor_pointer()
                .hover(|s| s.bg(t.hover))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.change(|c| c.input_device = name.clone());
                    cx.notify();
                }))
                .child(radio(t, chosen))
                .child(label)
        });
        let terminals = deliver::installed_terminals();
        let terminal = if c.agents.terminal.is_empty() { "Terminal".to_string() } else { c.agents.terminal.clone() };
        let days = c.history_days;
        div()
            .id("settings")
            .size_full()
            .overflow_y_scroll()
            .p(px(24.))
            .pt(px(44.))
            .flex()
            .flex_col()
            .gap(px(20.))
            .child(heading(t, "Settings", "Asides picks up changes at once."))
            .child(group(t, "Voice", vec![
                div().flex().flex_col().gap(px(2.)).children(models).into_any_element(),
                toggle_row(t, "mic-on-open", "Listen as soon as Asides opens", "Its pill's microphone (and holding Space) still turn it on and off.", c.mic_on_open, cx.listener(|this, _, _, cx| {
                    this.change(|c| c.mic_on_open = !c.mic_on_open);
                    cx.notify();
                })),
                label_row(t, "Microphone"),
                div().flex().flex_col().gap(px(2.)).children(devices).into_any_element(),
            ]))
            .child(group(t, "On the screen", vec![
                toggle_row(t, "identify", "Name what's under each pin", "Its role, title and, in browsers, the page and a CSS selector. Needs the Accessibility permission.", c.measure.identify_elements, cx.listener(|this, _, _, cx| {
                    this.change(|c| c.measure.identify_elements = !c.measure.identify_elements);
                    cx.notify();
                })),
                choice_row(t, "detail", "How much each pin says", &[("Compact", c.annotation_detail == Detail::Compact), ("Standard", c.annotation_detail == Detail::Standard), ("Detailed", c.annotation_detail == Detail::Detailed), ("Forensic", c.annotation_detail == Detail::Forensic)], cx, |this, i| {
                    let detail = [Detail::Compact, Detail::Standard, Detail::Detailed, Detail::Forensic][i];
                    this.change(|c| c.annotation_detail = detail);
                }),
                choice_row(t, "units", "Measure in", &[("Points", c.measure.units == Units::Pt), ("Pixels", c.measure.units == Units::Px)], cx, |this, i| {
                    this.change(|c| c.measure.units = if i == 0 { Units::Pt } else { Units::Px });
                }),
            ]))
            .child(group(t, "New sessions", vec![
                choice_row(t, "terminal", "Open them in", &terminals.iter().map(|n| (*n, n.eq_ignore_ascii_case(&terminal))).collect::<Vec<_>>(), cx, move |this, i| {
                    let name = terminals[i];
                    this.change(|c| c.agents.terminal = if name == "Terminal" { String::new() } else { name.to_string() });
                }),
                choice_row(t, "agent", "Start", &AgentKind::ALL[..3].iter().map(|a| (a.name(), *a == c.agents.default_agent())).collect::<Vec<_>>(), cx, |this, i| {
                    let agent = AgentKind::ALL[i];
                    this.change(|c| c.agents.default = agent.program().into());
                }),
            ]))
            .child(group(t, "History", vec![
                choice_row(t, "keep", "Keep asides for", &[("7 days", days == 7), ("30 days", days == 30), ("90 days", days == 90), ("Always", days == 0)], cx, |this, i| {
                    let days = [7, 30, 90, 0][i];
                    this.change(|c| c.history_days = days);
                }),
                toggle_row(t, "keep-audio", "Keep what was said as audio", "An .m4a with each aside, in ~/.asides/history.", c.keep_audio, cx.listener(|this, _, _, cx| {
                    this.change(|c| c.keep_audio = !c.keep_audio);
                    cx.notify();
                })),
            ]))
            .child(group(t, "Other", vec![
                toggle_row(t, "sounds", "Sounds", "When Asides listens, sends, or something goes wrong.", c.sounds, cx.listener(|this, _, _, cx| {
                    this.change(|c| c.sounds = !c.sounds);
                    cx.notify();
                })),
                toggle_row(t, "wispr", "Wispr Flow dictations", "What you dictate with Wispr Flow while the screen is frozen joins the aside.", c.wispr_flow, cx.listener(|this, _, _, cx| {
                    this.change(|c| c.wispr_flow = !c.wispr_flow);
                    cx.notify();
                })),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .text_size(px(12.5))
                    .child(format!("{}  opens Asides, and sends what you made", asides_core::config::pretty_shortcut(&c.shortcuts.asides)))
                    .child(format!("{}  switches between the frozen and the live screen", asides_core::config::pretty_shortcut(&c.shortcuts.freeze)))
                    .into_any_element(),
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(button(t, "edit-config", "Edit shortcuts and more…", false, cx.listener(|_, _, _, _| {
                        let _ = Command::new("open").args(["-t"]).arg(paths::config_path()).status();
                    })))
                    .child(button(t, "perm-mic", "Microphone…", false, cx.listener(|_, _, _, _| open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"))))
                    .child(button(t, "perm-screen", "Screen Recording…", false, cx.listener(|_, _, _, _| open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"))))
                    .child(button(t, "perm-ax", "Accessibility…", false, cx.listener(|_, _, _, _| open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"))))
                    .child(button(t, "perm-auto", "Automation…", false, cx.listener(|_, _, _, _| open_url("x-apple.systempreferences:com.apple.preference.security?Privacy_Automation"))))
                    .into_any_element(),
            ]))
            .into_any_element()
    }
}

/// A line of an aside's Markdown, lightly: headings bold, list items with a bullet, no asterisks.
fn markdown_line(t: Theme, line: &str) -> AnyElement {
    let plain = |text: &str| text.replace("**", "").replace('`', "");
    let trimmed = line.trim_start();
    if trimmed.is_empty() {
        return div().h(px(6.)).into_any_element();
    }
    if let Some(heading) = trimmed.strip_prefix("### ").or_else(|| trimmed.strip_prefix("## ")) {
        return div().pt(px(4.)).font_weight(FontWeight::SEMIBOLD).child(plain(heading)).into_any_element();
    }
    if let Some(item) = trimmed.strip_prefix("- ") {
        let indent = (line.len() - trimmed.len()) as f32 * 6.0;
        return div()
            .flex()
            .gap(px(6.))
            .pl(px(indent))
            .child(div().text_color(t.secondary).child("•"))
            .child(div().flex_1().child(plain(item)))
            .into_any_element();
    }
    div().child(plain(line)).into_any_element()
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
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(ms as i64).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

fn empty(t: Theme, title: &str, text: &str) -> AnyElement {
    div()
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .p(px(40.))
        .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .child(div().max_w(px(420.)).text_color(t.secondary).text_center().child(text.to_string()))
        .into_any_element()
}

fn heading(t: Theme, title: &str, text: &str) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().text_size(px(20.)).font_weight(FontWeight::SEMIBOLD).child(title.to_string()))
        .child(div().text_color(t.secondary).child(text.to_string()))
        .into_any_element()
}

fn group(t: Theme, title: &str, rows: Vec<AnyElement>) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(10.))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.secondary)
                .child(title.to_uppercase()),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .p(px(14.))
                .rounded(px(12.))
                .border_1()
                .border_color(t.border)
                .children(rows),
        )
        .into_any_element()
}

fn label_row(t: Theme, text: &str) -> AnyElement {
    div().text_size(px(12.5)).text_color(t.secondary).child(text.to_string()).into_any_element()
}

type Listener = Box<dyn Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static>;

fn button(
    t: Theme,
    id: &str,
    label: &str,
    primary: bool,
    on: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> gpui::Stateful<gpui::Div> {
    let on: Listener = Box::new(on);
    div()
        .id(SharedString::from(id.to_string()))
        .px(px(11.))
        .py(px(5.))
        .rounded(px(7.))
        .text_size(px(12.5))
        .cursor_pointer()
        .when(primary, |d| d.bg(t.accent).text_color(t.accent_ink).font_weight(FontWeight::MEDIUM))
        .when(!primary, |d| d.bg(t.surface).border_1().border_color(t.border).hover(|s| s.bg(t.hover)))
        .on_click(on)
        .child(label.to_string())
}

fn radio(t: Theme, on: bool) -> AnyElement {
    div()
        .size(px(14.))
        .rounded_full()
        .border_1()
        .border_color(if on { t.accent } else { t.secondary })
        .flex()
        .items_center()
        .justify_center()
        .when(on, |d| d.child(div().size(px(8.)).rounded_full().bg(t.accent)))
        .into_any_element()
}

fn toggle_row(
    t: Theme,
    id: &str,
    title: &str,
    text: &str,
    on: bool,
    change: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    let knob = div().size(px(14.)).rounded_full().bg(gpui::white()).shadow_sm();
    div()
        .id(SharedString::from(id.to_string()))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .cursor_pointer()
        .on_click(change)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(title.to_string())
                .child(div().text_size(px(12.)).text_color(t.secondary).child(text.to_string())),
        )
        .child(
            div()
                .flex_none()
                .w(px(32.))
                .h(px(18.))
                .p(px(2.))
                .rounded_full()
                .flex()
                .bg(if on { t.accent } else { t.border })
                .when(on, |d| d.justify_end())
                .child(knob),
        )
        .into_any_element()
}

/// A row of choices (a segmented control); `pick` gets the index chosen.
fn choice_row(
    t: Theme,
    id: &str,
    title: &str,
    choices: &[(&str, bool)],
    cx: &mut Context<AsidesWindow>,
    pick: impl Fn(&mut AsidesWindow, usize) + Clone + 'static,
) -> AnyElement {
    let buttons = choices.iter().enumerate().map(|(i, (label, on))| {
        let pick = pick.clone();
        div()
            .id(SharedString::from(format!("{id}-{i}")))
            .px(px(10.))
            .py(px(4.))
            .rounded(px(6.))
            .text_size(px(12.5))
            .cursor_pointer()
            .when(*on, |d| d.bg(t.accent).text_color(t.accent_ink).font_weight(FontWeight::MEDIUM))
            .when(!on, |d| d.hover(|s| s.bg(t.hover)))
            .on_click(cx.listener(move |this, _, _, cx| {
                pick(this, i);
                cx.notify();
            }))
            .child(label.to_string())
    });
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(title.to_string())
        .child(div().flex().gap(px(2.)).p(px(2.)).rounded(px(8.)).bg(t.surface).children(buttons))
        .into_any_element()
}
