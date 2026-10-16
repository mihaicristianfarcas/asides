//! What GPUI's windows need to be an overlay: borderless, above everything (the menu bar and
//! the Dock too), on every space, exactly over their screen; and an app without a Dock icon.

use asides_core::inspect::PtRect;
use gpui::Window;
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSEvent, NSScreenSaverWindowLevel, NSView, NSWindow,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Asides is a menu-bar app; GPUI makes its apps regular ones (a Dock icon, a menu bar).
pub fn accessory() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

fn native_window(window: &Window) -> Option<Retained<NSWindow>> {
    let handle = HasWindowHandle::window_handle(window).ok()?;
    let RawWindowHandle::AppKit(h) = handle.as_raw() else { return None };
    let view: &NSView = unsafe { h.ns_view.cast::<NSView>().as_ref() };
    view.window()
}

/// Makes `window` an overlay at `frame` (AppKit's coordinates); returns its window number and
/// its view (for development replays).
pub fn make_overlay(window: &Window, frame: PtRect) -> (isize, usize) {
    let view = view_pointer(window);
    let Some(ns) = native_window(window) else { return (0, view) };
    // Changing the style mask rebuilds the window's frame and resets its first responder to the
    // window: typed letters (which go only to the first responder, unlike Esc or ⌘ shortcuts)
    // would never reach GPUI. Give it back to GPUI's view below.
    ns.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    ns.setLevel(NSScreenSaverWindowLevel);
    ns.setHasShadow(false);
    ns.setMovable(false);
    ns.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    ns.setFrame_display(NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(frame.width, frame.height)), true);
    if view != 0 {
        let gpui_view: &NSView = unsafe { &*(view as *const NSView) };
        ns.makeFirstResponder(Some(gpui_view));
    }
    (ns.windowNumber(), view)
}

fn view_pointer(window: &Window) -> usize {
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::AppKit(h)) => h.ns_view.as_ptr() as usize,
        _ => 0,
    }
}

/// Development: with `asides-overlay-debug` in the temporary directory, notes what the overlay
/// gets (keys, focus) in `asides-overlay-debug.log` there, for when it runs under the app.
pub fn debug(message: &str) {
    use std::io::Write;
    let dir = std::env::temp_dir();
    if !dir.join("asides-overlay-debug").exists() {
        return;
    }
    let state = MainThreadMarker::new().map(|mtm| {
        let app = NSApplication::sharedApplication(mtm);
        let key = app.keyWindow().map(|w| w.windowNumber());
        format!("active={} key_window={key:?}", app.isActive())
    });
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("asides-overlay-debug.log"))
    {
        let _ = writeln!(f, "{message} [{}]", state.unwrap_or_default());
    }
}

/// Development: logs every key event AppKit hands the app (see `debug`), and who the key
/// window's first responder is.
pub fn monitor_keys() {
    if !std::env::temp_dir().join("asides-overlay-debug").exists() {
        return;
    }
    use objc2_app_kit::NSEventMask;
    let block = block2::RcBlock::new(|event: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
        let e = unsafe { event.as_ref() };
        let chars = e.characters().map(|c| c.to_string()).unwrap_or_default();
        let responder = MainThreadMarker::new()
            .and_then(|mtm| NSApplication::sharedApplication(mtm).keyWindow())
            .and_then(|w| w.firstResponder())
            .map(|r| r.class().name().to_string_lossy().into_owned())
            .unwrap_or_else(|| "none".into());
        debug(&format!("appkit {:?} {chars:?} code {} responder {responder}", e.r#type(), e.keyCode()));
        event.as_ptr()
    });
    let monitor = unsafe {
        NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown | NSEventMask::KeyUp, &block)
    };
    std::mem::forget(monitor);
}

/// Development: logs when the app's activation, key window or the frontmost app change.
pub fn debug_focus() {
    use std::sync::Mutex;
    static LAST: Mutex<String> = Mutex::new(String::new());
    if !std::env::temp_dir().join("asides-overlay-debug").exists() {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let front = objc2_app_kit::NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map(|a| a.localizedName().map(|n| n.to_string()).unwrap_or_default());
    let now = format!(
        "active={} key={:?} front={front:?} me={}",
        app.isActive(),
        app.keyWindow().map(|w| w.windowNumber()),
        std::process::id()
    );
    let mut last = LAST.lock().unwrap();
    if *last != now {
        debug(&format!("focus {now}"));
        *last = now;
    }
}

/// The mouse, in AppKit's global coordinates.
pub fn mouse() -> (f64, f64) {
    let p = NSEvent::mouseLocation();
    (p.x, p.y)
}

/// Development (replay): AppKit events, sent to GPUI's view the way AppKit would. GPUI handles an
/// event by updating its window, so these must be sent from outside any GPUI update (a replay
/// sends them from its task, between updates).
pub mod synthetic {
    use asides_core::keys::Chord;
    use objc2::msg_send;
    use objc2_app_kit::{NSEvent, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::{NSPoint, NSString};

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mouse {
        Move,
        Down { right: bool },
        Drag { right: bool },
        Up { right: bool },
    }

    /// The window's view, from `make_overlay`. It lives as long as the window.
    fn view<'a>(pointer: usize) -> Option<&'a NSView> {
        (pointer != 0).then(|| unsafe { &*(pointer as *const NSView) })
    }

    fn flags(shift: bool, alt: bool, cmd: bool, ctrl: bool) -> NSEventModifierFlags {
        let mut f = NSEventModifierFlags::empty();
        for (on, flag) in [
            (shift, NSEventModifierFlags::Shift),
            (alt, NSEventModifierFlags::Option),
            (cmd, NSEventModifierFlags::Command),
            (ctrl, NSEventModifierFlags::Control),
        ] {
            if on {
                f |= flag;
            }
        }
        f
    }

    /// A mouse event at `(x, y)` (window points, y down).
    pub fn mouse(window: usize, kind: Mouse, (x, y): (f64, f64), shift: bool, alt: bool, cmd: bool) {
        let Some(view) = view(window) else { return };
        let height = view.window().map_or(0.0, |w| w.contentLayoutRect().size.height);
        let number = view.window().map_or(0, |w| w.windowNumber());
        let at = NSPoint::new(x, height - y);
        // Events made this way have no button number, so a right-click is sent as a
        // control-click (which GPUI, like macOS, takes for one).
        let right =
            matches!(kind, Mouse::Down { right: true } | Mouse::Drag { right: true } | Mouse::Up { right: true });
        let (ty, selector) = match kind {
            Mouse::Move => (NSEventType::MouseMoved, "mouseMoved:"),
            Mouse::Down { .. } => (NSEventType::LeftMouseDown, "mouseDown:"),
            Mouse::Drag { .. } => (NSEventType::LeftMouseDragged, "mouseDragged:"),
            Mouse::Up { .. } => (NSEventType::LeftMouseUp, "mouseUp:"),
        };
        let clicks = if matches!(kind, Mouse::Down { .. } | Mouse::Up { .. }) { 1 } else { 0 };

        let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
            ty,
            at,
            flags(shift, alt, cmd, right),
            0.0,
            number,
            None,
            0,
            clicks,
            if clicks > 0 { 1.0 } else { 0.0 },
        ) else {
            return;
        };
        unsafe {
            match selector {
                "mouseMoved:" => {
                    let _: () = msg_send![view, mouseMoved: &*event];
                }
                "mouseDown:" => {
                    let _: () = msg_send![view, mouseDown: &*event];
                }
                "mouseDragged:" => {
                    let _: () = msg_send![view, mouseDragged: &*event];
                }
                _ => {
                    let _: () = msg_send![view, mouseUp: &*event];
                }
            }
        }
    }

    /// The US key code of a key, and the character it types (without and with Shift).
    fn key_code(key: &str) -> Option<(u16, String, String)> {
        let special = |code: u16, c: char| Some((code, c.to_string(), c.to_string()));
        match key {
            "enter" | "\n" => return special(36, '\r'),
            "tab" => return special(48, '\t'),
            "space" | " " => return special(49, ' '),
            "backspace" => return special(51, '\u{7f}'),
            "escape" => return special(53, '\u{1b}'),
            "left" => return special(123, '\u{f702}'),
            "right" => return special(124, '\u{f703}'),
            "down" => return special(125, '\u{f701}'),
            "up" => return special(126, '\u{f700}'),
            "f1" => return special(122, '\u{f704}'),
            _ => {}
        }
        const LAYOUT: &[(&str, u16, &str)] = &[
            ("a", 0, "A"),
            ("s", 1, "S"),
            ("d", 2, "D"),
            ("f", 3, "F"),
            ("h", 4, "H"),
            ("g", 5, "G"),
            ("z", 6, "Z"),
            ("x", 7, "X"),
            ("c", 8, "C"),
            ("v", 9, "V"),
            ("b", 11, "B"),
            ("q", 12, "Q"),
            ("w", 13, "W"),
            ("e", 14, "E"),
            ("r", 15, "R"),
            ("y", 16, "Y"),
            ("t", 17, "T"),
            ("1", 18, "!"),
            ("2", 19, "@"),
            ("3", 20, "#"),
            ("4", 21, "$"),
            ("6", 22, "^"),
            ("5", 23, "%"),
            ("=", 24, "+"),
            ("9", 25, "("),
            ("7", 26, "&"),
            ("-", 27, "_"),
            ("8", 28, "*"),
            ("0", 29, ")"),
            ("]", 30, "}"),
            ("o", 31, "O"),
            ("u", 32, "U"),
            ("[", 33, "{"),
            ("i", 34, "I"),
            ("p", 35, "P"),
            ("l", 37, "L"),
            ("j", 38, "J"),
            ("'", 39, "\""),
            ("k", 40, "K"),
            (";", 41, ":"),
            ("\\", 42, "|"),
            (",", 43, "<"),
            ("/", 44, "?"),
            ("n", 45, "N"),
            ("m", 46, "M"),
            (".", 47, ">"),
            ("`", 50, "~"),
        ];
        LAYOUT
            .iter()
            .find(|(plain, _, shifted)| *plain == key || *shifted == key)
            .map(|(plain, code, shifted)| (*code, plain.to_string(), shifted.to_string()))
    }

    fn send_key(view: &NSView, number: isize, chord: &Chord, down: bool) {
        let Some((code, plain, shifted)) = key_code(&chord.key) else { return };
        let shift = chord.shift || (shifted == chord.key && plain != chord.key);
        let typed = if shift { shifted } else { plain.clone() };
        let ty = if down { NSEventType::KeyDown } else { NSEventType::KeyUp };
        let Some(event) = NSEvent::keyEventWithType_location_modifierFlags_timestamp_windowNumber_context_characters_charactersIgnoringModifiers_isARepeat_keyCode(
            ty,
            NSPoint::new(0.0, 0.0),
            flags(shift, chord.alt, chord.cmd, chord.ctrl),
            0.0,
            number,
            None,
            &NSString::from_str(&typed),
            &NSString::from_str(&plain),
            false,
            code,
        ) else {
            return;
        };
        unsafe {
            if down {
                let _: () = msg_send![view, keyDown: &*event];
            } else {
                let _: () = msg_send![view, keyUp: &*event];
            }
        }
    }

    /// A key going down or up (`chord` like `shift+h`).
    pub fn key(window: usize, chord: &Chord, down: bool) {
        let Some(view) = view(window) else { return };
        let number = view.window().map_or(0, |w| w.windowNumber());
        send_key(view, number, chord, down);
    }

    /// Typing `text`, a key at a time.
    pub fn text(window: usize, text: &str) {
        let Some(view) = view(window) else { return };
        let number = view.window().map_or(0, |w| w.windowNumber());
        for c in text.chars() {
            let key = c.to_string();
            let Some((_, plain, shifted)) = key_code(&key.to_lowercase()).or_else(|| key_code(&key)) else { continue };
            let chord = Chord {
                key: if c.is_uppercase() || shifted == key && plain != key { shifted } else { plain },
                ..Chord::default()
            };
            send_key(view, number, &chord, true);
            send_key(view, number, &chord, false);
        }
    }
}
