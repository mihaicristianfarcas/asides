//! The status item and its menu: the asides mark (what's happening) and a short menu: what's
//! going on, Open Asides, the Asides window, Quit. Everything else is in the Asides window.

use std::str::FromStr;

use asides_core::config::Config;
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::icons::{Mark, StatusIcon};

#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    /// Opens Asides, or, open, sends the aside.
    Open,
    /// The Asides window (History, Sessions, Settings).
    Window,
    Quit,
}

pub struct Tray {
    icon: TrayIcon,
    /// Kept alive with the status item.
    _menu: Menu,
    shown: Option<(Mark, Option<String>)>,
    status: MenuItem,
    open: MenuItem,
    window: MenuItem,
    quit: MenuItem,
}

/// What the menu should show; computed by the app on every change.
pub struct View {
    pub mark: Mark,
    pub icon: StatusIcon,
    /// Beside the brackets: only a recording's time or a download's progress.
    pub title: Option<String>,
    pub status: String,
    /// Asides is open.
    pub open: bool,
}

impl Tray {
    pub fn new(config: &Config, icon: &StatusIcon) -> anyhow::Result<Tray> {
        let status = MenuItem::new("", false, None);
        let open = MenuItem::new("Open Asides", true, accelerator(&config.shortcuts.asides));
        let window = MenuItem::new("Asides Window…", true, Accelerator::from_str("cmd+Comma").ok());
        let quit = MenuItem::new("Quit Asides", true, Accelerator::from_str("cmd+KeyQ").ok());
        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &PredefinedMenuItem::separator(),
            &open,
            &window,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;
        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(true)
            .with_tooltip("Asides");
        let builder = if icon.template {
            builder.with_icon_templated(icon.icon.clone())
        } else {
            builder.with_icon(icon.icon.clone())
        };
        let icon = builder.build()?;
        Ok(Tray { icon, _menu: menu, shown: None, status, open, window, quit })
    }

    pub fn action(&self, id: &MenuId) -> Option<MenuAction> {
        if id == self.open.id() {
            Some(MenuAction::Open)
        } else if id == self.window.id() {
            Some(MenuAction::Window)
        } else if id == self.quit.id() {
            Some(MenuAction::Quit)
        } else {
            None
        }
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
        self.open.set_text(if view.open { "Send the Aside" } else { "Open Asides" });
    }

    /// Shortcuts changed in the settings.
    pub fn sync_config(&self, config: &Config) {
        self.open.set_accelerator(accelerator(&config.shortcuts.asides)).ok();
    }
}

/// muda drops a single `&` from menu titles (mnemonics): doubled, it shows.
fn menu_text(text: &str) -> String {
    text.replace('&', "&&")
}

fn accelerator(shortcut: &str) -> Option<Accelerator> {
    Accelerator::from_str(shortcut).ok()
}
