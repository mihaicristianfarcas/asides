//! Global shortcuts (Carbon hot keys via `global-hotkey`; no Accessibility permission needed).
//! They keep working while Measure & Annotate is open, so the user can record while measuring.

use std::str::FromStr;

use asides_core::config::Shortcuts;
use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::HotKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Record,
    RecordNewNote,
    AttachImage,
    Cancel,
    SendToAgent,
    /// Measure & Annotate.
    Measure,
    /// Switch the target to the next agent session.
    NextTarget,
}

pub struct Hotkeys {
    manager: GlobalHotKeyManager,
    bindings: Vec<(HotKey, Action)>,
    cancel: Option<HotKey>,
    cancel_active: bool,
    /// Shortcuts that could not be parsed or registered (e.g. taken by another app).
    pub errors: Vec<String>,
}

impl Hotkeys {
    pub fn new(shortcuts: &Shortcuts, send: bool, sessions: bool) -> anyhow::Result<Hotkeys> {
        let mut hotkeys = Hotkeys {
            manager: GlobalHotKeyManager::new()?,
            bindings: Vec::new(),
            cancel: None,
            cancel_active: false,
            errors: Vec::new(),
        };
        hotkeys.apply(shortcuts, send, sessions);
        Ok(hotkeys)
    }

    /// (Re)registers all shortcuts. Cancel is only registered while recording, so Esc keeps
    /// working normally in other apps the rest of the time, Send to Agent only with `send`
    /// (batch delivery), and switching sessions only while there are `sessions`, so those
    /// shortcuts stay free for other apps otherwise.
    pub fn apply(&mut self, shortcuts: &Shortcuts, send: bool, sessions: bool) {
        for (hotkey, _) in self.bindings.drain(..) {
            let _ = self.manager.unregister(hotkey);
        }
        self.set_cancel_active(false);
        self.errors.clear();
        let send = send.then_some((&shortcuts.send_to_agent, Action::SendToAgent));
        let next = sessions.then_some((&shortcuts.next_target, Action::NextTarget));
        for (text, action) in [
            (&shortcuts.record, Action::Record),
            (&shortcuts.record_new_note, Action::RecordNewNote),
            (&shortcuts.attach_image, Action::AttachImage),
            (&shortcuts.measure, Action::Measure),
        ]
        .into_iter()
        .chain(send)
        .chain(next)
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
        self.cancel = match HotKey::from_str(&shortcuts.cancel) {
            Ok(hotkey) => Some(hotkey),
            Err(err) => {
                self.errors.push(format!("Shortcut {} is invalid: {err}", shortcuts.cancel));
                None
            }
        };
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        if self.cancel_active && self.cancel.is_some_and(|c| c.id() == id) {
            return Some(Action::Cancel);
        }
        self.bindings.iter().find(|(hotkey, _)| hotkey.id() == id).map(|(_, action)| *action)
    }

    pub fn set_cancel_active(&mut self, active: bool) {
        let Some(cancel) = self.cancel else { return };
        if active == self.cancel_active {
            return;
        }
        let result = if active { self.manager.register(cancel) } else { self.manager.unregister(cancel) };
        self.cancel_active = active && result.is_ok();
    }
}
