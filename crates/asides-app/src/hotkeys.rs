//! Global shortcuts (Carbon hot keys via `global-hotkey`; no Accessibility permission needed).
//! They work while Asides is open too, over the live screen as well as frozen.

use std::str::FromStr;

use asides_core::config::Shortcuts;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Opens Asides frozen; while it's open, sends the aside.
    Asides,
    /// Freezes the screen, or goes back to live; closed, opens Asides live.
    Freeze,
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    bindings: Vec<(HotKey, Action)>,
    /// Shortcuts that could not be parsed or registered (e.g. taken by another app).
    pub errors: Vec<String>,
}

impl Hotkeys {
    pub fn new(shortcuts: &Shortcuts) -> anyhow::Result<Hotkeys> {
        let mut hotkeys = Hotkeys { manager: GlobalHotKeyManager::new()?, bindings: Vec::new(), errors: Vec::new() };
        hotkeys.apply(shortcuts);
        Ok(hotkeys)
    }

    /// (Re)registers the shortcuts.
    pub fn apply(&mut self, shortcuts: &Shortcuts) {
        for (hotkey, _) in self.bindings.drain(..) {
            let _ = self.manager.unregister(hotkey);
        }
        self.errors.clear();
        for (text, action) in [(&shortcuts.asides, Action::Asides), (&shortcuts.freeze, Action::Freeze)]
            .into_iter()
            .filter(|(text, _)| !text.trim().is_empty())
        {
            match HotKey::from_str(text) {
                Ok(hotkey) => match self.manager.register(hotkey) {
                    Ok(()) => self.bindings.push((hotkey, action)),
                    Err(err) => self.errors.push(format!("Shortcut {text} is unavailable: {err}")),
                },
                Err(err) => self.errors.push(format!("Shortcut {text} is invalid: {err}")),
            }
        }
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        self.bindings.iter().find(|(hotkey, _)| hotkey.id() == id).map(|(_, action)| *action)
    }
}
