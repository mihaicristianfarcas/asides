//! What GPUI's windows need to be an overlay: borderless, above everything (the menu bar and
//! the Dock too), on every space, exactly over their screen; and an app without a Dock icon.

use asides_core::inspect::PtRect;
use gpui::Window;
use objc2::MainThreadMarker;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSApplicationActivationPolicy, NSApplicationPresentationOptions,
    NSEvent, NSEventType, NSPanel, NSRunningApplication, NSScreen, NSScreenSaverWindowLevel, NSView, NSWindow,
    NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{NSNumber, NSPoint, NSRect, NSSize, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// Asides is a menu-bar app; GPUI makes its apps regular ones (a Dock icon, a menu bar).
pub fn accessory() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// GPUI makes its app a regular one as it finishes launching, and `accessory` turns it back. Made
/// regular again right after (frozen, `kiosk`), the app got a second Dock tile, which the Dock kept
/// over the live screen. This keeps the app an accessory through that first call; call it before
/// GPUI starts. (`setActivationPolicy:` is NSApplication's, which GPUI's application class keeps.)
pub fn stay_accessory_at_launch() {
    use objc2::runtime::{Bool, Imp, Sel};
    use std::sync::OnceLock;
    use std::sync::atomic::{AtomicBool, Ordering};

    type SetPolicy = unsafe extern "C-unwind" fn(&AnyObject, Sel, isize) -> Bool;
    static ORIGINAL: OnceLock<SetPolicy> = OnceLock::new();
    static LAUNCHED: AtomicBool = AtomicBool::new(false);

    unsafe extern "C-unwind" fn set_policy(app: &AnyObject, cmd: Sel, policy: isize) -> Bool {
        let policy = match LAUNCHED.swap(true, Ordering::Relaxed) {
            // GPUI's, at launch: an accessory still.
            false => NSApplicationActivationPolicy::Accessory.0,
            true => policy,
        };
        match ORIGINAL.get() {
            Some(original) => unsafe { original(app, cmd, policy) },
            None => Bool::NO,
        }
    }

    let Some(class) = AnyClass::get(c"NSApplication") else { return };
    let Some(method) = class.instance_method(objc2::sel!(setActivationPolicy:)) else { return };
    let ours: Imp = unsafe { std::mem::transmute::<SetPolicy, Imp>(set_policy) };
    let original = unsafe { method.set_implementation(ours) };
    let _ = ORIGINAL.set(unsafe { std::mem::transmute::<Imp, SetPolicy>(original) });
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
    // GPUI's pop-ups fade in and out, showing what they last showed while they do.
    ns.setAnimationBehavior(NSWindowAnimationBehavior::None);
    // Live again, they stay up until the user's app is back (`windows::handed_back`).
    ns.setHidesOnDeactivate(false);
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

/// Makes `window` the toolbar over the live screen at `frame` (AppKit's coordinates): a panel
/// above other apps' windows, on every space, that never takes the keyboard from the app the user
/// is working in. Returns its window number and view (for development replays).
pub fn make_toolbar(window: &Window, frame: PtRect) -> (isize, usize) {
    let view = view_pointer(window);
    let Some(ns) = native_window(window) else { return (0, view) };
    ns.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    ns.setAnimationBehavior(NSWindowAnimationBehavior::None);
    ns.setHasShadow(false);
    ns.setMovable(false);
    ns.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    // The overlay's app is never the active one while the screen is live: a panel would hide.
    ns.setHidesOnDeactivate(false);
    // Clicks on it don't make it the key window: keys stay with the app underneath.
    if let Some(panel) = ns.downcast_ref::<NSPanel>() {
        panel.setBecomesKeyOnlyIfNeeded(true);
    }
    ns.setFrame_display(NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(frame.width, frame.height)), true);
    (ns.windowNumber(), view)
}

// The windows, by their views (`make_overlay`, `make_toolbar`), changed outside GPUI's updates:
// AppKit tells GPUI about each change (a new size, the keyboard) as it happens, which GPUI can't
// take while it's updating, and drops.

/// GPUI's view (from `make_overlay` or `make_toolbar`); it lives as long as its window.
fn view<'a>(pointer: usize) -> Option<&'a NSView> {
    (pointer != 0).then(|| unsafe { &*(pointer as *const NSView) })
}

/// Draws these windows now, as GPUI would their next frames, and puts it on screen: hidden ones
/// show it when they're shown, not what they showed last. (Hidden, GPUI doesn't draw them.)
pub fn draw_now(views: &[usize]) {
    for v in views.iter().filter_map(|&v| view(v)) {
        // GPUI draws on `displayLayer:`, for whatever needs drawing.
        let _: () = unsafe { msg_send![v, displayLayer: std::ptr::null::<AnyObject>()] };
    }
    if let Some(transaction) = AnyClass::get(c"CATransaction") {
        let _: () = unsafe { msg_send![transaction, flush] };
    }
}

/// Shows the window; with `key`, it takes the keyboard too.
pub fn show(pointer: usize, key: bool) {
    let Some(window) = view(pointer).and_then(|v| v.window()) else { return };
    match key {
        true => window.makeKeyAndOrderFront(None),
        false => window.orderFrontRegardless(),
    }
}

/// The window is on screen, drawn (not just ordered in).
pub fn on_screen(pointer: usize) -> bool {
    view(pointer)
        .and_then(|v| v.window())
        .is_some_and(|w| w.occlusionState().contains(objc2_app_kit::NSWindowOcclusionState::Visible))
}

pub fn hide(pointer: usize) {
    if let Some(window) = view(pointer).and_then(|v| v.window()) {
        window.orderOut(None);
    }
}

/// The window's frame, in AppKit's coordinates.
pub fn frame(pointer: usize) -> Option<PtRect> {
    let f = view(pointer)?.window()?.frame();
    Some(PtRect::new(f.origin.x, f.origin.y, f.size.width, f.size.height))
}

pub fn set_frame(pointer: usize, frame: PtRect) {
    if let Some(window) = view(pointer).and_then(|v| v.window()) {
        let rect = NSRect::new(NSPoint::new(frame.x, frame.y), NSSize::new(frame.width, frame.height));
        window.setFrame_display(rect, true);
    }
}

/// Tells the window where the pointer is now (it moved while the window was hidden): GPUI drops
/// what was under the pointer when it was last shown, and Measure follows it from there.
pub fn pointer_moved(pointer: usize) {
    let Some(v) = view(pointer) else { return };
    let Some(window) = v.window() else { return };
    let at = window.mouseLocationOutsideOfEventStream();
    let Some(event) = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
        NSEventType::MouseMoved,
        at,
        NSEvent::modifierFlags_class(),
        0.0,
        window.windowNumber(),
        None,
        0,
        0,
        0.0,
    ) else {
        return;
    };
    let _: () = unsafe { msg_send![v, mouseMoved: &*event] };
}

/// While the screens are frozen, what macOS lets an app hold back stays back, so nothing slides in
/// over the frozen screen or takes the user away: the Dock and the menu bar stay hidden; app
/// switching is off, and with it Mission Control, App Exposé, Show Desktop and switching spaces, by
/// keys, gestures or hot corners; the overlay can't be hidden, and the Apple menu is off. Force
/// Quit stays, a way out. They hold only while the overlay's app is active, and app switching only
/// for a regular app (an accessory one, without a Dock icon, still switches with ⌘Tab): it's one
/// while frozen, with the Dock hidden. Live, an accessory again, and the user's own settings.
pub fn kiosk(on: bool) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    if on {
        app.setPresentationOptions(
            NSApplicationPresentationOptions::HideDock
                | NSApplicationPresentationOptions::HideMenuBar
                | NSApplicationPresentationOptions::DisableProcessSwitching
                | NSApplicationPresentationOptions::DisableHideApplication
                | NSApplicationPresentationOptions::DisableAppleMenu,
        );
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        activate();
    } else {
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.setPresentationOptions(NSApplicationPresentationOptions::Default);
    }
}

/// Makes the overlay's app the active one (the keyboard, and the kiosk options with it). Asked
/// politely (`activate`, macOS 14's way), macOS declines while the user's app is in front.
#[allow(deprecated)]
pub fn activate() {
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

pub fn is_active() -> bool {
    MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
}

/// The app in front (its process id), unless it's the overlay's.
pub fn front_app() -> Option<i32> {
    let pid = NSWorkspace::sharedWorkspace().frontmostApplication()?.processIdentifier();
    (pid != std::process::id() as i32).then_some(pid)
}

/// Gives the keyboard back to the app with this process id, if the overlay has it: over the live
/// screen (or closed) it's the user's, who may have moved on to another app meanwhile.
pub fn give_focus(pid: Option<i32>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let ns = NSApplication::sharedApplication(mtm);
    if !ns.isActive() {
        return;
    }
    match pid.and_then(NSRunningApplication::runningApplicationWithProcessIdentifier) {
        // macOS activates another app at the active one's request.
        Some(app) => {
            ns.yieldActivationToApplication(&app);
            app.activateWithOptions(NSApplicationActivationOptions::empty());
        }
        // It quit meanwhile: macOS picks the next.
        None => ns.deactivate(),
    }
}

/// The screens: each one's display and frame (AppKit's coordinates), as the app describes them.
pub fn screens() -> Vec<(u32, PtRect)> {
    let Some(mtm) = MainThreadMarker::new() else { return Vec::new() };
    let key = NSString::from_str("NSScreenNumber");
    NSScreen::screens(mtm)
        .iter()
        .filter_map(|screen| {
            let number = screen.deviceDescription().objectForKey(&key)?;
            let display = number.downcast_ref::<NSNumber>()?.unsignedIntValue();
            let f = screen.frame();
            Some((display, PtRect::new(f.origin.x, f.origin.y, f.size.width, f.size.height)))
        })
        .collect()
}

/// The part of the screen at `frame` (AppKit's coordinates) that the Dock and the menu bar leave.
pub fn visible_frame(frame: PtRect) -> PtRect {
    let Some(mtm) = MainThreadMarker::new() else { return frame };
    NSScreen::screens(mtm)
        .iter()
        .find(|s| (s.frame().origin.x - frame.x).abs() < 1.0 && (s.frame().origin.y - frame.y).abs() < 1.0)
        .map_or(frame, |s| {
            let v = s.visibleFrame();
            PtRect::new(v.origin.x, v.origin.y, v.size.width, v.size.height)
        })
}

/// The left mouse button is down (anywhere).
pub fn mouse_pressed() -> bool {
    NSEvent::pressedMouseButtons() & 1 != 0
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
        let ms =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() % 100_000);
        let _ = writeln!(f, "{ms} {message} [{}]", state.unwrap_or_default());
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
        "active={} key={:?} front={front:?} me={} options={:?} system={:?}",
        app.isActive(),
        app.keyWindow().map(|w| w.windowNumber()),
        std::process::id(),
        app.presentationOptions(),
        app.currentSystemPresentationOptions(),
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
