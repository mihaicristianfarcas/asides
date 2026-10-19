//! The status item and its menu: the asides mark (what's happening) and a short menu: what's
//! going on, opening asides (frozen or live), copying the last aside, the settings wanted most
//! often, History and Settings in the asides window, Quit.

use std::str::FromStr;

use asides_core::config::Config;
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::icons::{Mark, StatusIcon};

#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// Opens asides, or, open, sends the aside.
    Open,
    /// Opens asides live, or, open, switches between frozen and live.
    Live,
    /// The last aside, as Markdown, on the clipboard.
    CopyLast,
    /// Turns a setting on or off.
    Listen,
    Sounds,
    Identify,
    /// The asides window, at History or the settings.
    History,
    Settings,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    /// Kept alive with the status item.
    _menu: Menu,
    shown: Option<(Mark, Option<String>)>,
    status: MenuItem,
    open: MenuItem,
    live: MenuItem,
    copy_last: MenuItem,
    listen: CheckMenuItem,
    sounds: CheckMenuItem,
    identify: CheckMenuItem,
    history: MenuItem,
    settings: MenuItem,
    quit: MenuItem,
}

/// What the menu should show; computed by the app on every change.
pub struct View {
    pub mark: Mark,
    pub icon: StatusIcon,
    /// Beside the brackets: only a recording's time or a download's progress.
    pub title: Option<String>,
    pub status: String,
    /// asides is open.
    pub open: bool,
    /// There's an aside to copy.
    pub has_last: bool,
}

impl Tray {
    pub fn new(config: &Config, icon: &StatusIcon) -> anyhow::Result<Tray> {
        let status = MenuItem::new("", false, None);
        let open = MenuItem::new("Open asides", true, accelerator(&config.shortcuts.asides));
        let live = MenuItem::new("Open Live", true, accelerator(&config.shortcuts.freeze));
        let copy_last = MenuItem::new("Copy the Last Aside", false, None);
        let listen = CheckMenuItem::new("Listen When asides Opens", true, config.mic_on_open, None);
        let sounds = CheckMenuItem::new("Sounds", true, config.sounds, None);
        let identify = CheckMenuItem::new("Name What's Pinned", true, config.measure.identify_elements, None);
        let history = MenuItem::new("History…", true, None);
        let settings = MenuItem::new("Settings…", true, Accelerator::from_str("cmd+Comma").ok());
        let quit = MenuItem::new("Quit asides", true, Accelerator::from_str("cmd+KeyQ").ok());
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &open,
            &live,
            &copy_last,
            &PredefinedMenuItem::separator(),
            &listen,
            &sounds,
            &identify,
            &PredefinedMenuItem::separator(),
            &history,
            &settings,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(true)
            .with_tooltip("asides");
        let builder = if icon.template {
            builder.with_icon_templated(icon.icon.clone())
        } else {
            builder.with_icon(icon.icon.clone())
        };
        let icon = builder.build()?;
        Ok(Tray {
            icon,
            _menu: menu,
            shown: None,
            status,
            open,
            live,
            copy_last,
            listen,
            sounds,
            identify,
            history,
            settings,
            quit,
        })
    }

    pub fn action(&self, id: &MenuId) -> Option<MenuAction> {
        [
            (self.open.id(), MenuAction::Open),
            (self.live.id(), MenuAction::Live),
            (self.copy_last.id(), MenuAction::CopyLast),
            (self.listen.id(), MenuAction::Listen),
            (self.sounds.id(), MenuAction::Sounds),
            (self.identify.id(), MenuAction::Identify),
            (self.history.id(), MenuAction::History),
            (self.settings.id(), MenuAction::Settings),
            (self.quit.id(), MenuAction::Quit),
        ]
        .into_iter()
        .find(|(item, _)| *item == id)
        .map(|(_, action)| action)
    }

    pub fn show(&mut self, view: &View) {
        let shown = (view.mark, view.title.clone());
        if self.shown.as_ref().map(|s| s.0) != Some(view.mark) {
            let icon = Some(view.icon.icon.clone());
            let _ = if view.icon.template { self.icon.set_icon_templated(icon) } else { self.icon.set_icon(icon) };
        }
        if self.shown.as_ref().map(|s| &s.1) != Some(&view.title) {
            self.icon.set_title(view.title.as_deref());
        }
        self.shown = Some(shown);
        self.status.set_text(menu_text(&view.status));
        self.open.set_text(if view.open { "Send the Aside" } else { "Open asides" });
        self.live.set_text(if view.open { "Frozen or Live" } else { "Open Live" });
        self.copy_last.set_enabled(view.has_last);
    }

    /// The settings changed: shortcuts, and what's on or off.
    pub fn sync_config(&self, config: &Config) {
        self.open.set_accelerator(accelerator(&config.shortcuts.asides)).ok();
        self.live.set_accelerator(accelerator(&config.shortcuts.freeze)).ok();
        self.listen.set_checked(config.mic_on_open);
        self.sounds.set_checked(config.sounds);
        self.identify.set_checked(config.measure.identify_elements);
    }
}

/// muda drops a single `&` from menu titles (mnemonics): doubled, it shows.
fn menu_text(text: &str) -> String {
    text.replace('&', "&&")
}

fn accelerator(shortcut: &str) -> Option<Accelerator> {
    Accelerator::from_str(shortcut).ok()
}
