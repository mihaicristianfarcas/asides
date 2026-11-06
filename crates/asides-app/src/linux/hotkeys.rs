//! Global shortcuts on Linux. Each desktop has its own way, and the first that applies is used:
//!
//! - On X11, the key is grabbed on the root window (`XGrabKey`): Cinnamon, MATE, Xfce, any GNOME or
//!   Plasma on Xorg.
//! - On GNOME on Wayland, a custom keyboard shortcut in its settings (Settings → Keyboard → Custom
//!   Shortcuts lists them) runs `asides open` or `asides live`. Every version has it; only GNOME 48
//!   and later have the portal.
//! - On KDE on Wayland, `kglobalaccel`, as its own apps have them (System Settings → Shortcuts lists
//!   them): nothing to approve, where its portal opens that page and waits.
//! - On other Wayland desktops, the GlobalShortcuts portal: the desktop asks the user once.
//!
//! Elsewhere (Sway, say, with no portal for this) the user binds a key to those two commands
//! themselves; the error says so.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use asides_core::config::Shortcuts;
use asides_core::paths::BUNDLE_ID;
use asides_core::portal;
use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::xkb::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask, Window};
use x11rb::rust_connection::RustConnection;
use zbus::zvariant::{OwnedObjectPath, Value};

use crate::capture::on_wayland;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Opens asides frozen; while it's open, sends the aside.
    Asides,
    /// Freezes the screen, or goes back to live; closed, opens asides live.
    Freeze,
}

impl Action {
    /// Its number: the id of its events.
    fn id(self) -> u32 {
        match self {
            Action::Asides => 1,
            Action::Freeze => 2,
        }
    }

    /// The command that does the same, and what a desktop's settings call it.
    fn command(self) -> (&'static str, &'static str) {
        match self {
            Action::Asides => ("open", "Open asides, or send the aside"),
            Action::Freeze => ("live", "Freeze the screen, or go live"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotKeyState {
    Pressed,
    Released,
}

/// A shortcut pressed or let go.
#[derive(Debug, Clone, Copy)]
pub struct GlobalHotKeyEvent {
    pub id: u32,
    pub state: HotKeyState,
}

type Handler = Box<dyn Fn(GlobalHotKeyEvent) + Send + Sync>;
static HANDLER: OnceLock<Handler> = OnceLock::new();

impl GlobalHotKeyEvent {
    pub fn set_event_handler<F: Fn(GlobalHotKeyEvent) + Send + Sync + 'static>(handler: Option<F>) {
        if let Some(handler) = handler {
            let _ = HANDLER.set(Box::new(handler));
        }
    }
}

fn say(id: u32, state: HotKeyState) {
    if let Some(handler) = HANDLER.get() {
        handler(GlobalHotKeyEvent { id, state });
    }
}

/// A shortcut as the settings write it (`ctrl+alt+Space`, `ctrl+alt+KeyP`): its modifiers and key.
#[derive(Debug, Clone, PartialEq)]
pub struct Parsed {
    ctrl: bool,
    alt: bool,
    shift: bool,
    logo: bool,
    /// As `global-hotkey` names keys (`Space`, `KeyP`, `Digit1`, `F5`).
    key: String,
}

impl Parsed {
    pub fn parse(text: &str) -> Option<Parsed> {
        let mut parsed = Parsed { ctrl: false, alt: false, shift: false, logo: false, key: String::new() };
        for word in text.split('+').map(str::trim) {
            match word.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => parsed.ctrl = true,
                "alt" | "option" => parsed.alt = true,
                "shift" => parsed.shift = true,
                "super" | "cmd" | "command" | "meta" => parsed.logo = true,
                _ if !parsed.key.is_empty() => return None,
                _ => parsed.key = word.to_string(),
            }
        }
        keysym(&parsed.key).map(|_| parsed)
    }

    fn mods(&self) -> ModMask {
        let mut mods = ModMask::from(0u16);
        for (on, mask) in [
            (self.ctrl, ModMask::CONTROL),
            (self.alt, ModMask::M1),
            (self.shift, ModMask::SHIFT),
            (self.logo, ModMask::M4),
        ] {
            if on {
                mods |= mask;
            }
        }
        mods
    }

    /// As GNOME's settings and the shortcuts portal write it: `<Control><Alt>space`.
    fn accelerator(&self) -> String {
        let mut text = String::new();
        for (on, name) in
            [(self.ctrl, "<Control>"), (self.alt, "<Alt>"), (self.shift, "<Shift>"), (self.logo, "<Super>")]
        {
            if on {
                text.push_str(name);
            }
        }
        text.push_str(&gtk_key(&self.key));
        text
    }

    /// As the shortcuts portal's `preferred_trigger` writes it: `CTRL+ALT+space`.
    fn trigger(&self) -> String {
        let mut text = String::new();
        for (on, name) in [(self.ctrl, "CTRL+"), (self.alt, "ALT+"), (self.shift, "SHIFT+"), (self.logo, "LOGO+")] {
            if on {
                text.push_str(name);
            }
        }
        text.push_str(&gtk_key(&self.key));
        text
    }

    /// As Qt numbers it, its modifiers in the high bits: how `kglobalaccel` takes a shortcut.
    fn qt_key(&self) -> Option<i32> {
        let key = match keysym(&self.key)? {
            sym @ 0x20..=0x7e => i32::from((sym as u8).to_ascii_uppercase()),
            sym @ 0xffbe..=0xffd5 => 0x0100_0030 + (sym - 0xffbe) as i32,
            0xff1b => 0x0100_0000,
            0xff09 => 0x0100_0001,
            0xff08 => 0x0100_0003,
            0xff0d => 0x0100_0004,
            0xffff => 0x0100_0007,
            0xff51 => 0x0100_0012,
            0xff52 => 0x0100_0013,
            0xff53 => 0x0100_0014,
            0xff54 => 0x0100_0015,
            _ => return None,
        };
        let mods =
            [(self.shift, 0x0200_0000), (self.ctrl, 0x0400_0000), (self.alt, 0x0800_0000), (self.logo, 0x1000_0000)];
        Some(mods.into_iter().filter(|(on, _)| *on).fold(key, |all, (_, bit)| all | bit))
    }
}

/// The key's X11 keysym name, which GNOME's settings and portals use too.
fn gtk_key(key: &str) -> String {
    if let Some(letter) = key.strip_prefix("Key") {
        return letter.to_ascii_lowercase();
    }
    if let Some(digit) = key.strip_prefix("Digit") {
        return digit.to_string();
    }
    let name = match key {
        "Space" => "space",
        "Enter" => "Return",
        "Escape" => "Escape",
        "Tab" => "Tab",
        "Backspace" => "BackSpace",
        "Delete" => "Delete",
        "ArrowLeft" => "Left",
        "ArrowRight" => "Right",
        "ArrowUp" => "Up",
        "ArrowDown" => "Down",
        "Equal" => "equal",
        "Minus" => "minus",
        "Slash" => "slash",
        "Backslash" => "backslash",
        "Comma" => "comma",
        "Period" => "period",
        "Semicolon" => "semicolon",
        "Quote" => "apostrophe",
        "BracketLeft" => "bracketleft",
        "BracketRight" => "bracketright",
        "Backquote" => "grave",
        other => other,
    };
    name.to_string()
}

/// The key's X11 keysym.
fn keysym(key: &str) -> Option<u32> {
    if let Some(letter) = key.strip_prefix("Key") {
        let [c] = letter.as_bytes() else { return None };
        return c.is_ascii_uppercase().then(|| c.to_ascii_lowercase() as u32);
    }
    if let Some(digit) = key.strip_prefix("Digit") {
        let [c] = digit.as_bytes() else { return None };
        return c.is_ascii_digit().then_some(*c as u32);
    }
    if let Some(n) = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()) {
        return (1..=24).contains(&n).then_some(0xffbe + n - 1);
    }
    Some(match key {
        "Space" => 0x20,
        "Enter" => 0xff0d,
        "Escape" => 0xff1b,
        "Tab" => 0xff09,
        "Backspace" => 0xff08,
        "Delete" => 0xffff,
        "ArrowLeft" => 0xff51,
        "ArrowUp" => 0xff52,
        "ArrowRight" => 0xff53,
        "ArrowDown" => 0xff54,
        "Equal" => 0x3d,
        "Minus" => 0x2d,
        "Slash" => 0x2f,
        "Backslash" => 0x5c,
        "Comma" => 0x2c,
        "Period" => 0x2e,
        "Semicolon" => 0x3b,
        "Quote" => 0x27,
        "BracketLeft" => 0x5b,
        "BracketRight" => 0x5d,
        "Backquote" => 0x60,
        _ => return None,
    })
}

/// Which way this desktop has for global shortcuts.
enum Backend {
    X11(X11Keys),
    Gnome,
    Kde(KdeKeys),
    Portal(PortalKeys),
    /// Nothing asides can use: the user binds the commands themselves.
    None,
}

pub struct Hotkeys {
    backend: Backend,
    /// Shortcuts that could not be parsed or registered (e.g. taken by another app).
    pub errors: Vec<String>,
}

impl Hotkeys {
    pub fn new(shortcuts: &Shortcuts) -> anyhow::Result<Hotkeys> {
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
        let backend = if !on_wayland() {
            X11Keys::new().map(Backend::X11).map_err(|err| anyhow::anyhow!("{err}"))?
        } else if desktop.split(':').any(|d| d.eq_ignore_ascii_case("GNOME")) {
            Backend::Gnome
        } else if let Some(keys) =
            desktop.split(':').any(|d| d.eq_ignore_ascii_case("KDE")).then(KdeKeys::new).flatten()
        {
            Backend::Kde(keys)
        } else {
            PortalKeys::new().map_or(Backend::None, Backend::Portal)
        };
        let mut hotkeys = Hotkeys { backend, errors: Vec::new() };
        hotkeys.apply(shortcuts);
        Ok(hotkeys)
    }

    /// (Re)registers the shortcuts.
    pub fn apply(&mut self, shortcuts: &Shortcuts) {
        self.errors.clear();
        let mut wanted = Vec::new();
        for (text, action) in [(&shortcuts.asides, Action::Asides), (&shortcuts.freeze, Action::Freeze)] {
            if text.trim().is_empty() {
                wanted.push((action, None));
                continue;
            }
            match Parsed::parse(text) {
                Some(parsed) => wanted.push((action, Some(parsed))),
                None => self.errors.push(format!("Shortcut {text} is invalid")),
            }
        }
        match &mut self.backend {
            Backend::X11(keys) => self.errors.extend(keys.apply(&wanted)),
            Backend::Gnome => self.errors.extend(gnome_apply(&wanted)),
            Backend::Kde(keys) => self.errors.extend(keys.apply(&wanted)),
            Backend::Portal(keys) => keys.apply(&wanted),
            Backend::None => self.errors.push(
                "This desktop has no global shortcuts for apps: bind a key to `asides open` and one to `asides live` \
                 in its settings."
                    .into(),
            ),
        }
    }

    pub fn action(&self, id: u32) -> Option<Action> {
        [Action::Asides, Action::Freeze].into_iter().find(|a| a.id() == id)
    }
}

// --- X11 ---

/// The keys grabbed, and the connection that hears them.
struct X11Keys {
    connection: Arc<RustConnection>,
    root: Window,
    grabbed: Arc<Mutex<Vec<(u8, u16, u32)>>>,
}

/// Caps Lock, Num Lock and both: a grab is for an exact set of modifiers, these mustn't matter.
const LOCKS: [u16; 4] = [0, 0x02, 0x10, 0x12];

impl X11Keys {
    fn new() -> Result<X11Keys, String> {
        let (connection, screen) = x11rb::connect(None).map_err(|err| format!("no X server: {err}"))?;
        let connection = Arc::new(connection);
        let root = connection.setup().roots.get(screen).ok_or("no such X screen")?.root;
        // Without this, a key held repeats as a release and a press: asides would open and send.
        if connection.extension_information(xkb::X11_EXTENSION_NAME).ok().flatten().is_some()
            && let Ok(cookie) = connection.xkb_use_extension(1, 0)
            && cookie.reply().is_ok()
        {
            let flag = xkb::PerClientFlag::DETECTABLE_AUTO_REPEAT;
            let _ = connection.xkb_per_client_flags(
                xkb::ID::USE_CORE_KBD.into(),
                flag,
                flag,
                xkb::BoolCtrl::from(0u32),
                xkb::BoolCtrl::from(0u32),
                xkb::BoolCtrl::from(0u32),
            );
        }
        let grabbed: Arc<Mutex<Vec<(u8, u16, u32)>>> = Arc::default();
        let (hears, keys) = (connection.clone(), grabbed.clone());
        std::thread::Builder::new()
            .name("asides-shortcuts".into())
            .spawn(move || {
                // What each key down pressed. Its release goes with that, whatever's still held:
                // the user may let go of the modifiers first.
                let mut down: HashMap<u8, u32> = HashMap::new();
                while let Ok(event) = hears.wait_for_event() {
                    match event {
                        x11rb::protocol::Event::KeyPress(e) => {
                            let state = u16::from(e.state) & !LOCKS[3];
                            let found =
                                keys.lock().unwrap().iter().find(|k| k.0 == e.detail && k.1 == state).map(|k| k.2);
                            if let Some(id) = found {
                                down.insert(e.detail, id);
                                say(id, HotKeyState::Pressed);
                            }
                        }
                        x11rb::protocol::Event::KeyRelease(e) => {
                            if let Some(id) = down.remove(&e.detail) {
                                say(id, HotKeyState::Released);
                            }
                        }
                        _ => {}
                    }
                }
            })
            .map_err(|err| err.to_string())?;
        Ok(X11Keys { connection, root, grabbed })
    }

    /// The key codes that type `sym` on the keyboard's layout.
    fn codes(&self, sym: u32) -> Vec<u8> {
        let setup = self.connection.setup();
        let (first, count) = (setup.min_keycode, setup.max_keycode - setup.min_keycode + 1);
        let Some(map) = self.connection.get_keyboard_mapping(first, count).ok().and_then(|c| c.reply().ok()) else {
            return Vec::new();
        };
        let per = map.keysyms_per_keycode.max(1) as usize;
        map.keysyms
            .chunks(per)
            .enumerate()
            .filter(|(_, syms)| syms.iter().take(2).any(|&s| s == sym))
            .map(|(i, _)| first + i as u8)
            .collect()
    }

    fn apply(&mut self, wanted: &[(Action, Option<Parsed>)]) -> Vec<String> {
        let mut errors = Vec::new();
        let mut keys = self.grabbed.lock().unwrap();
        for (code, mods, _) in keys.drain(..) {
            for lock in LOCKS {
                let _ = self.connection.ungrab_key(code, self.root, ModMask::from(mods | lock));
            }
        }
        for (action, parsed) in wanted {
            let Some(parsed) = parsed else { continue };
            let codes = keysym(&parsed.key).map(|sym| self.codes(sym)).unwrap_or_default();
            let Some(&code) = codes.first() else {
                errors.push(format!("Shortcut {} is on no key of this keyboard", parsed.key));
                continue;
            };
            let mods = u16::from(parsed.mods());
            let taken = LOCKS.iter().any(|lock| {
                self.connection
                    .grab_key(false, self.root, ModMask::from(mods | lock), code, GrabMode::ASYNC, GrabMode::ASYNC)
                    .map_or(true, |cookie| cookie.check().is_err())
            });
            if taken {
                errors.push(format!("Shortcut {} is taken by another app", parsed.accelerator()));
                for lock in LOCKS {
                    let _ = self.connection.ungrab_key(code, self.root, ModMask::from(mods | lock));
                }
                continue;
            }
            keys.push((code, mods, action.id()));
        }
        let _ = self.connection.flush();
        errors
    }
}

// --- GNOME's own custom shortcuts ---

const GNOME_LIST: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_KEY: &str = "custom-keybindings";
const GNOME_ENTRY: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";
const GNOME_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

fn gsettings(args: &[&str]) -> Result<String, String> {
    let out = std::process::Command::new("gsettings")
        .args(args)
        .output()
        .map_err(|err| format!("could not run gsettings: {err}"))?;
    match out.status.success() {
        true => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// A string as `gsettings` reads and prints one.
fn gvariant_text(text: &str) -> String {
    let (quote, escaped) = match text.contains('\'') {
        // GVariant prints such a string in double quotes.
        true => ('"', text.replace('\\', r"\\").replace('"', r#"\""#)),
        false => ('\'', text.replace('\\', r"\\")),
    };
    format!("{quote}{escaped}{quote}")
}

/// The paths in a printed `custom-keybindings` list (`['/a/', '/b/']`, or `@as []`).
fn parse_gsettings_list(text: &str) -> Vec<String> {
    let inner = text.trim().trim_start_matches("@as").trim().trim_start_matches('[').trim_end_matches(']');
    inner.split(',').map(|p| p.trim().trim_matches('\'').to_string()).filter(|p| !p.is_empty()).collect()
}

fn print_gsettings_list(paths: &[String]) -> String {
    let quoted: Vec<String> = paths.iter().map(|p| format!("'{p}'")).collect();
    format!("[{}]", quoted.join(", "))
}

/// A word of the command line GNOME runs (`g_shell_parse_argv`), quoted only when it must be: so it
/// reads well in Settings.
fn shell_word(word: &str) -> String {
    match word.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        true => word.to_string(),
        false => format!("'{}'", word.replace('\'', r"'\''")),
    }
}

/// Says in GNOME's settings what each shortcut runs; a shortcut with no key comes off the list.
fn gnome_apply(wanted: &[(Action, Option<Parsed>)]) -> Vec<String> {
    let cli = std::env::current_exe().unwrap_or_default().with_file_name("asides");
    let mut errors = Vec::new();
    let result = (|| -> Result<(), String> {
        let before = parse_gsettings_list(&gsettings(&["get", GNOME_LIST, GNOME_KEY])?);
        let mut list = before.clone();
        for (action, parsed) in wanted {
            let (command, name) = action.command();
            let path = format!("{GNOME_PATH}/asides-{command}/");
            list.retain(|p| *p != path);
            let Some(parsed) = parsed else { continue };
            list.push(path.clone());
            let schema = format!("{GNOME_ENTRY}:{path}");
            let run = format!("{} {command}", shell_word(&cli.display().to_string()));
            for (key, value) in
                [("name", format!("asides: {name}")), ("command", run), ("binding", parsed.accelerator())]
            {
                let value = gvariant_text(&value);
                if gsettings(&["get", &schema, key]).is_ok_and(|now| now == value) {
                    continue;
                }
                gsettings(&["set", &schema, key, &value])?;
            }
        }
        if list != before {
            gsettings(&["set", GNOME_LIST, GNOME_KEY, &print_gsettings_list(&list)])?;
        }
        Ok(())
    })();
    if let Err(err) = result {
        errors.push(format!("Could not set asides' shortcuts in GNOME's settings: {err}"));
    }
    errors
}

// --- The GlobalShortcuts portal ---

const SHORTCUTS: &str = "org.freedesktop.portal.GlobalShortcuts";

/// The portal's session, and what it was last asked to bind.
struct PortalKeys {
    session: OwnedObjectPath,
    bound: Option<Vec<(Action, Option<Parsed>)>>,
}

impl PortalKeys {
    /// A session with the portal, and a thread that hears its signals. `None` without the portal.
    fn new() -> Option<PortalKeys> {
        let connection = portal::connection().ok()?;
        let proxy = zbus::blocking::Proxy::new(connection, portal::DESKTOP, portal::OBJECT, SHORTCUTS).ok()?;
        proxy.get_property::<u32>("version").ok()?;
        // The same every time: KDE keeps a session's shortcuts by this name, and asks again for a
        // name it hasn't seen.
        let session_token = "asides";
        let results = portal::request(
            SHORTCUTS,
            "CreateSession",
            |token| {
                let mut options: HashMap<&str, Value> = HashMap::new();
                options.insert("handle_token", Value::from(token.to_string()));
                options.insert("session_handle_token", Value::from(session_token));
                (options,)
            },
            Duration::from_secs(10),
        )
        .ok()?;
        let session = OwnedObjectPath::try_from(portal::text(&results, "session_handle")?).ok()?;
        for (signal, state) in [("Activated", HotKeyState::Pressed), ("Deactivated", HotKeyState::Released)] {
            let mut signals = proxy.receive_signal(signal).ok()?;
            let _ = std::thread::Builder::new().name("asides-shortcuts".into()).spawn(move || {
                for message in &mut signals {
                    type Shortcut = (OwnedObjectPath, String, u64, HashMap<String, zbus::zvariant::OwnedValue>);
                    let Ok((_, id, ..)) = message.body().deserialize::<Shortcut>() else { continue };
                    let action = [Action::Asides, Action::Freeze].into_iter().find(|a| a.command().0 == id);
                    if let Some(action) = action {
                        say(action.id(), state);
                    }
                }
            });
        }
        Some(PortalKeys { session, bound: None })
    }

    /// Asks the desktop for these shortcuts: it shows the user what they'd be, the first time. Its
    /// answer can take that long, so it comes on a thread of its own.
    fn apply(&mut self, wanted: &[(Action, Option<Parsed>)]) {
        if self.bound.as_deref() == Some(wanted) {
            return;
        }
        self.bound = Some(wanted.to_vec());
        let session = self.session.clone();
        let wanted = wanted.to_vec();
        let _ = std::thread::Builder::new().name("asides-bind-shortcuts".into()).spawn(move || {
            let shortcuts: Vec<(String, HashMap<&str, Value>)> = wanted
                .iter()
                .filter_map(|(action, parsed)| {
                    let parsed = parsed.as_ref()?;
                    let (id, description) = action.command();
                    let mut options: HashMap<&str, Value> = HashMap::new();
                    options.insert("description", Value::from(description));
                    options.insert("preferred_trigger", Value::from(parsed.trigger()));
                    Some((id.to_string(), options))
                })
                .collect();
            let bound = portal::request(
                SHORTCUTS,
                "BindShortcuts",
                |token| {
                    let mut options: HashMap<&str, Value> = HashMap::new();
                    options.insert("handle_token", Value::from(token.to_string()));
                    (session.clone(), shortcuts.clone(), "", options)
                },
                Duration::from_secs(600),
            );
            match bound {
                Ok(results) => asides_core::log::write(format_args!("global shortcuts bound: {results:?}")),
                Err(err) => asides_core::log::problem(format_args!("{BUNDLE_ID}: global shortcuts: {err}")),
            }
        });
    }
}

// --- KDE ---

const KGLOBALACCEL: &str = "org.kde.kglobalaccel";

/// `kglobalaccel`, which keeps KDE's shortcuts and tells who owns one when it's pressed.
struct KdeKeys {
    accel: zbus::blocking::Proxy<'static>,
    listening: bool,
}

impl KdeKeys {
    fn new() -> Option<KdeKeys> {
        let connection = portal::connection().ok()?;
        let accel =
            zbus::blocking::Proxy::new(connection, KGLOBALACCEL, "/kglobalaccel", "org.kde.KGlobalAccel").ok()?;
        // Starts it, if it hasn't yet; a desktop without it has no such name.
        accel.call::<_, _, Vec<Vec<String>>>("allMainComponents", &()).ok()?;
        Some(KdeKeys { accel, listening: false })
    }

    /// What's pressed comes as a signal of its component, there once it has an action.
    fn listen(&mut self) -> Result<(), String> {
        let path: OwnedObjectPath = self.accel.call("getComponent", &(BUNDLE_ID,)).map_err(|err| err.to_string())?;
        let connection = portal::connection()?;
        let component = zbus::blocking::Proxy::new(connection, KGLOBALACCEL, path, "org.kde.kglobalaccel.Component")
            .map_err(|err| err.to_string())?;
        let signals = component.receive_signal("globalShortcutPressed").map_err(|err| err.to_string())?;
        std::thread::Builder::new()
            .name("asides-shortcuts".into())
            .spawn(move || {
                for message in signals {
                    let Ok((owner, name, _)) = message.body().deserialize::<(String, String, i64)>() else { continue };
                    let action = [Action::Asides, Action::Freeze].into_iter().find(|a| a.command().0 == name);
                    if let Some(action) = action.filter(|_| owner == BUNDLE_ID) {
                        // Only the press matters, and it doesn't repeat here.
                        say(action.id(), HotKeyState::Pressed);
                        say(action.id(), HotKeyState::Released);
                    }
                }
            })
            .map_err(|err| err.to_string())?;
        self.listening = true;
        Ok(())
    }

    fn apply(&mut self, wanted: &[(Action, Option<Parsed>)]) -> Vec<String> {
        // `KGlobalAccel`'s flags: the action is present, and these keys are meant, not what's saved.
        const SET_PRESENT_NO_AUTOLOADING: u32 = 2 | 4;
        let mut errors = Vec::new();
        for (action, parsed) in wanted {
            let (name, description) = action.command();
            let id = vec![BUNDLE_ID, name, "asides", description];
            if let Err(err) = self.accel.call::<_, _, ()>("doRegister", &(&id,)) {
                errors.push(format!("Could not set the shortcut: {err}"));
                continue;
            }
            let Some(key) = parsed.as_ref().map(Parsed::qt_key) else {
                let none: Vec<(Vec<i32>,)> = Vec::new();
                let cleared = self
                    .accel
                    .call::<_, _, Vec<(Vec<i32>,)>>("setShortcutKeys", &(&id, none, SET_PRESENT_NO_AUTOLOADING));
                if let Err(err) = cleared {
                    errors.push(format!("Could not clear the shortcut: {err}"));
                }
                continue;
            };
            let Some(key) = key else {
                errors.push(format!("Shortcut {} isn't one KDE can have", description));
                continue;
            };
            // A sequence of four keys, always: Qt reads four, and anything short aborts `kglobalaccel`
            // and, as it goes, KWin.
            let keys = vec![(vec![key, 0, 0, 0],)];
            let set =
                self.accel.call::<_, _, Vec<(Vec<i32>,)>>("setShortcutKeys", &(&id, keys, SET_PRESENT_NO_AUTOLOADING));
            match set {
                Ok(now) if now.iter().any(|(k,)| k.first() == Some(&key)) => {}
                Ok(_) => errors.push(format!("Another app has the shortcut for \"{description}\"")),
                Err(err) => errors.push(format!("Could not set the shortcut: {err}")),
            }
        }
        if !self.listening
            && let Err(err) = self.listen()
        {
            errors.push(format!("Could not hear the shortcuts: {err}"));
        }
        errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use asides_core::keys::{Chord, Mods};

    #[test]
    fn numbers_shortcuts_as_qt_does() {
        let qt = |text: &str| Parsed::parse(text).and_then(|p| p.qt_key());
        assert_eq!(qt("ctrl+alt+Space"), Some(0x0c00_0020));
        assert_eq!(qt("ctrl+alt+KeyP"), Some(0x0c00_0050));
        assert_eq!(qt("shift+super+Digit1"), Some(0x1200_0031));
        assert_eq!(qt("ctrl+F5"), Some(0x0400_0000 | 0x0100_0034));
        assert_eq!(qt("ctrl+ArrowLeft"), Some(0x0400_0000 | 0x0100_0012));
        for text in [&Shortcuts::default().asides, &Shortcuts::default().freeze] {
            assert!(qt(text).is_some(), "{text}");
        }
    }

    #[test]
    fn takes_shortcuts_the_window_records() {
        let mods = Mods { ctrl: true, alt: true, ..Mods::default() };
        for key in ["space", "p", "1", "f5", "/", "=", "enter", "left", "escape", "`"] {
            let text = Chord::pressed(key, mods).and_then(|c| c.to_global()).unwrap();
            assert!(Parsed::parse(&text).is_some(), "{text}");
        }
        let defaults = Shortcuts::default();
        for text in [&defaults.asides, &defaults.freeze] {
            assert!(Parsed::parse(text).is_some(), "{text}");
        }
    }

    #[test]
    fn writes_shortcuts_as_desktops_do() {
        let parsed = Parsed::parse("ctrl+alt+Space").unwrap();
        assert_eq!(parsed.accelerator(), "<Control><Alt>space");
        assert_eq!(parsed.trigger(), "CTRL+ALT+space");
        assert_eq!(Parsed::parse("ctrl+alt+KeyP").unwrap().accelerator(), "<Control><Alt>p");
        assert_eq!(Parsed::parse("shift+super+Digit1").unwrap().accelerator(), "<Shift><Super>1");
        assert_eq!(Parsed::parse("ctrl+Nothing"), None);
        assert_eq!(keysym("KeyP"), Some(b'p' as u32));
        assert_eq!(keysym("F5"), Some(0xffc2));
    }

    #[test]
    fn quotes_what_gsettings_reads() {
        assert_eq!(gvariant_text("/usr/bin/asides open"), "'/usr/bin/asides open'");
        assert_eq!(gvariant_text("<Control><Alt>space"), "'<Control><Alt>space'");
        assert_eq!(gvariant_text("'/a b/asides' open"), r#""'/a b/asides' open""#);
        assert_eq!(shell_word("/usr/bin/asides"), "/usr/bin/asides");
        assert_eq!(shell_word("/a b/asides"), "'/a b/asides'");
    }

    #[test]
    fn reads_and_writes_gnomes_list() {
        assert!(parse_gsettings_list("@as []").is_empty());
        let list = parse_gsettings_list("['/a/', '/b/']");
        assert_eq!(list, ["/a/", "/b/"]);
        assert_eq!(print_gsettings_list(&list), "['/a/', '/b/']");
    }
}
