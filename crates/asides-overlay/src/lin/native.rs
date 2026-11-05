//! What GPUI's windows need to be an overlay on Linux, as `native.rs` does for macOS: borderless,
//! above everything (the panels too), exactly over their screen, in no window list.
//!
//! One way for every desktop: X11, on a Wayland desktop through Xwayland (the app starts the overlay
//! so). An override-redirect window is one the window manager leaves alone, wherever it is, and
//! X11 lets it take the keyboard at once. Wayland lets a client none of these, and each compositor
//! has its own ways, or none. GNOME scales X11 windows up on a high-resolution screen, so there the
//! frozen screen is as sharp as X11 lets it be: its pictures are full size all the same.
//!
//! - **Showing and hiding.** An X11 window can't be drawn while it's unmapped, and GPUI's swapchain
//!   goes with its mapping: a hidden window stays mapped, off every screen (`AWAY`), and "where it
//!   is" here is where it'd be (`PLACES`).
//! - **The keyboard.** `XSetInputFocus` takes it. Going back, a focused X11 window is told to whom,
//!   and on Wayland a window that's gone gives it back: the compositor's the one who knows. KWin
//!   gives the keyboard only to its active window, which an override-redirect window can't be: a
//!   stand-in is (`keyboard_stand_in`), and the keyboard's grabbed from it.
//! - **Frozen**, what a desktop lets an app hold back is nothing: Alt+Tab can take the keyboard
//!   away behind the frozen screens, and a click takes it back (`ui.rs`).
//! - **The glass**: nothing blurs: the pill is its tint over what's behind (`set_glass`).
//! - Frames are in the coordinates the overlay shares with macOS (AppKit's: y up from the main
//!   screen's bottom, points), `asides_core::display`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use asides_core::display::{self, Monitor};
use asides_core::inspect::PtRect;
use gpui::{App, Window};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use x11rb::connection::Connection;
use x11rb::protocol::xinput::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _,
    CreateWindowAux, EventMask, GrabMode, GrabStatus, ImageFormat, InputFocus, PropMode, StackMode, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

/// Off every screen (X11 coordinates are 16 bits).
const AWAY: i32 = -32000;

/// The X server, for the overlay's own calls beside GPUI's.
struct Server {
    connection: RustConnection,
    screen: usize,
    root: u32,
}

fn server() -> Option<&'static Server> {
    static SERVER: OnceLock<Option<Server>> = OnceLock::new();
    SERVER
        .get_or_init(|| {
            let (connection, screen) = x11rb::connect(None).ok()?;
            let root = connection.setup().roots.get(screen)?.root;
            Some(Server { connection, screen, root })
        })
        .as_ref()
}

/// An overlay window: where it goes, and whether it's there.
#[derive(Clone, Copy)]
struct Place {
    frame: PtRect,
    shown: bool,
}

/// The overlay's windows, by their ids.
fn places() -> &'static Mutex<HashMap<usize, Place>> {
    static PLACES: OnceLock<Mutex<HashMap<usize, Place>>> = OnceLock::new();
    PLACES.get_or_init(Mutex::default)
}

/// The pill's window, which stays above the frozen screens.
static PILL: AtomicU32 = AtomicU32::new(0);
/// What had the keyboard when the overlay took it.
static BEFORE: AtomicU32 = AtomicU32::new(0);
/// The overlay's window that has grabbed the keyboard (`keyboard_stand_in`).
static GRABBED: AtomicU32 = AtomicU32::new(0);

/// The desktop's Wayland: X11 clients see only each other there.
fn on_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland")
}

fn monitors() -> Vec<Monitor> {
    let listed = match server() {
        Some(s) => display::monitors_on(&s.connection, s.screen),
        None => display::monitors(),
    };
    listed.into_iter().map(|l| l.monitor).collect()
}

/// A frame (AppKit's coordinates) in the X server's pixels.
fn pixels(frame: PtRect) -> (i32, i32, i32, i32) {
    display::frame_to_pixels(&monitors(), frame)
}

fn atom(server: &Server, name: &[u8]) -> Option<Atom> {
    server.connection.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
}

/// The asides window has no glass here: nothing would blur behind it.
pub fn window_glass() -> gpui::WindowBackgroundAppearance {
    gpui::WindowBackgroundAppearance::Opaque
}

/// Windows': Linux's caption buttons are icons.
#[allow(dead_code)]
pub fn caption_font() -> &'static str {
    ""
}

/// Once the asides window is open: its title, which GPUI leaves out on Wayland.
pub fn dress(window: &mut Window) {
    window.set_window_title("asides");
}

/// The desktop leaves the window's frame to the window: GNOME on Wayland, the one that has no
/// decorations of its own to offer (GPUI patched: `third_party/gpui/PATCHES.md`).
pub fn draws_its_title_bar(window: &Window) -> bool {
    matches!(window.window_decorations(), gpui::Decorations::Client { .. })
}

/// macOS's Dock icon: an override-redirect window is in no list.
pub fn accessory() {}

pub fn stay_accessory_at_launch() {}

/// The window's id: what the native calls here take (on Wayland, the asides window's surface).
pub fn view_pointer(window: &Window) -> usize {
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Xcb(h)) => h.window.get() as usize,
        Ok(RawWindowHandle::Wayland(h)) => h.surface.as_ptr() as usize,
        _ => 0,
    }
}

/// Makes `id` the overlay's: nobody manages it, it's marked so the app's captures leave it out
/// (`capture.rs`), and it's mapped, away.
fn adopt(id: usize, frame: PtRect) {
    let Some(server) = server() else { return };
    let window = id as u32;
    let connection = &server.connection;
    let aux = ChangeWindowAttributesAux::new().override_redirect(1);
    let _ = connection.change_window_attributes(window, &aux);
    if let Some(mark) = atom(server, b"_ASIDES_OVERLAY") {
        let _ = connection.change_property32(PropMode::REPLACE, window, mark, AtomEnum::CARDINAL, &[1]);
    }
    let (_, _, w, h) = pixels(frame);
    let size = ConfigureWindowAux::new().x(AWAY).y(AWAY).width(w.max(1) as u32).height(h.max(1) as u32);
    let _ = connection.configure_window(window, &size);
    let _ = connection.map_window(window);
    let _ = connection.flush();
    places().lock().unwrap().insert(id, Place { frame, shown: false });
    watch_window(window);
}

/// Makes `window` a frozen screen's at `frame`. Returns its id twice (macOS's window number and view).
pub fn make_overlay(window: &Window, frame: PtRect) -> (isize, usize) {
    let view = view_pointer(window);
    if view != 0 {
        adopt(view, frame);
        if let Some(server) = server() {
            pill_after(server, view as u32);
            let _ = server.connection.flush();
        }
    }
    (view as isize, view)
}

/// Mutter stacks override-redirect windows as they were mapped, whatever X11 says: a frozen screen
/// mapped after the pill covers it from then on. The pill goes after it.
fn pill_after(server: &Server, window: u32) {
    let pill = PILL.load(Ordering::Relaxed);
    if on_wayland() && pill != 0 && pill != window {
        let _ = server.connection.unmap_window(pill);
        let _ = server.connection.map_window(pill);
    }
}

/// Makes `window` the pill's at `frame`. Returns its id twice.
pub fn make_toolbar(window: &Window, frame: PtRect) -> (isize, usize) {
    let view = view_pointer(window);
    if view != 0 {
        adopt(view, frame);
        PILL.store(view as u32, Ordering::Relaxed);
    }
    (view as isize, view)
}

/// Draws these windows now, as GPUI would their next frames, off the screen or not: they show it
/// when they're shown, not what they showed last.
pub fn draw_now(views: &[usize], cx: &mut App) {
    if views.is_empty() {
        return;
    }
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, cx| {
            if views.contains(&view_pointer(window)) {
                window.draw_now(cx);
            }
        });
    }
}

/// macOS commits what's moved and drawn together; X11 shows each as it's done.
pub fn transaction<R>(f: impl FnOnce() -> R) -> R {
    f()
}

/// Puts a window where it goes, above the others.
fn bring(server: &Server, id: usize) {
    let frame = {
        let mut places = places().lock().unwrap();
        let Some(place) = places.get_mut(&id) else { return };
        place.shown = true;
        place.frame
    };
    let (x, y, w, h) = pixels(frame);
    let aux =
        ConfigureWindowAux::new().x(x).y(y).width(w.max(1) as u32).height(h.max(1) as u32).stack_mode(StackMode::ABOVE);
    let _ = server.connection.configure_window(id as u32, &aux);
}

/// Shows the window, above the others; with `key`, it takes the keyboard too.
pub fn show(pointer: usize, key: bool) {
    let Some(server) = server() else { return };
    bring(server, pointer);
    if key {
        take_keyboard(server, pointer as u32);
    }
    // The pill stays above the frozen screens.
    let pill = PILL.load(Ordering::Relaxed);
    if pill != 0 && pill as usize != pointer {
        let above = ConfigureWindowAux::new().stack_mode(StackMode::ABOVE);
        let _ = server.connection.configure_window(pill, &above);
    }
    let _ = server.connection.flush();
}

/// The window with the keyboard, as the X server says (0 for none, 1 for the one under the pointer).
fn focused(server: &Server) -> u32 {
    match GRABBED.load(Ordering::Relaxed) {
        0 => server.connection.get_input_focus().ok().and_then(|c| c.reply().ok()).map_or(0, |r| r.focus),
        grabbed => grabbed,
    }
}

/// On a Wayland desktop other than GNOME's, the compositor gives the keyboard to its active window
/// alone, and an override-redirect window is never that (GNOME gives it to the window X11 says
/// has it). So a window it manages stands in: tiny, under the frozen screens, in no list. Active, it
/// brings the keyboard to Xwayland, whose grab sends it to the overlay's window; gone, the
/// compositor activates what was active before.
fn keyboard_stand_in(server: &Server) -> Option<u32> {
    static STAND_IN: OnceLock<Option<u32>> = OnceLock::new();
    *STAND_IN.get_or_init(|| {
        let connection = &server.connection;
        let id = connection.generate_id().ok()?;
        let aux = CreateWindowAux::new().background_pixel(0);
        let class = WindowClass::INPUT_OUTPUT;
        connection.create_window(0, id, server.root, 0, 0, 1, 1, 0, class, 0, &aux).ok()?;
        let name = asides_core::paths::BUNDLE_ID.as_bytes();
        let _ = connection.change_property8(PropMode::REPLACE, id, AtomEnum::WM_NAME, AtomEnum::STRING, name);
        let class = [name, b"\0", name, b"\0"].concat();
        let _ = connection.change_property8(PropMode::REPLACE, id, AtomEnum::WM_CLASS, AtomEnum::STRING, &class);
        let state: Vec<Atom> = [&b"_NET_WM_STATE_SKIP_TASKBAR"[..], b"_NET_WM_STATE_SKIP_PAGER"]
            .iter()
            .filter_map(|n| atom(server, n))
            .collect();
        if let Some(key) = atom(server, b"_NET_WM_STATE") {
            let _ = connection.change_property32(PropMode::REPLACE, id, key, AtomEnum::ATOM, &state);
        }
        if let Some(hints) = atom(server, b"_MOTIF_WM_HINTS") {
            // Flags: these say which decorations. None.
            let _ = connection.change_property32(PropMode::REPLACE, id, hints, hints, &[2, 0, 0, 0, 0]);
        }
        Some(id)
    })
}

/// The compositor's keyboard comes only by way of a window it manages (`keyboard_stand_in`).
fn needs_stand_in() -> bool {
    on_wayland() && !std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("GNOME"))
}

/// A grab's keys come to the connection that grabbed: they go on to the window's, GPUI's. (Sent to no
/// event mask, an event goes to the client that made the window.)
fn forward_grabbed_keys(server: &'static Server) {
    static STARTED: std::sync::Once = std::sync::Once::new();
    STARTED.call_once(|| {
        let forward = move || {
            while let Ok(event) = server.connection.wait_for_event() {
                let to = GRABBED.load(Ordering::Relaxed);
                let sent = match event {
                    x11rb::protocol::Event::KeyPress(key) => {
                        server.connection.send_event(false, to, EventMask::NO_EVENT, key)
                    }
                    x11rb::protocol::Event::KeyRelease(key) => {
                        server.connection.send_event(false, to, EventMask::NO_EVENT, key)
                    }
                    _ => continue,
                };
                let _ = sent.map(|_| server.connection.flush());
            }
        };
        if let Err(err) = std::thread::Builder::new().name("asides-keys-forward".into()).spawn(forward) {
            eprintln!("asides-overlay: no keys on this desktop: {err}");
        }
    });
}

/// The stand-in active, and the keyboard's X11 events `window`'s.
fn grab_keyboard(server: &'static Server, window: u32) {
    let Some(stand_in) = keyboard_stand_in(server) else { return };
    forward_grabbed_keys(server);
    let connection = &server.connection;
    if GRABBED.load(Ordering::Relaxed) == 0 {
        let _ = connection.map_window(stand_in);
        if let Some(active) = atom(server, b"_NET_ACTIVE_WINDOW") {
            // Asking as a pager does (2): an app's own request may be declined as focus stealing.
            let ask = ClientMessageEvent::new(32, stand_in, active, [2u32, 0, 0, 0, 0]);
            let to = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
            let _ = connection.send_event(false, server.root, to, ask);
        }
    }
    let grab = connection.grab_keyboard(false, window, x11rb::CURRENT_TIME, GrabMode::ASYNC, GrabMode::ASYNC);
    let status = grab.ok().and_then(|c| c.reply().ok()).map(|r| r.status);
    if status == Some(GrabStatus::SUCCESS) {
        GRABBED.store(window, Ordering::Relaxed);
    } else {
        eprintln!("asides-overlay: could not grab the keyboard: {status:?}");
    }
    let _ = connection.flush();
}

/// `window` takes the keyboard from whatever has it.
fn take_keyboard(server: &'static Server, window: u32) {
    if needs_stand_in() {
        return grab_keyboard(server, window);
    }
    let now = focused(server);
    if !places().lock().unwrap().contains_key(&(now as usize)) {
        BEFORE.store(now, Ordering::Relaxed);
    }
    let _ = server.connection.set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME);
    let _ = server.connection.flush();
}

/// The overlay's window `window` gives the keyboard back. To an X11 window it was taken from, it's
/// handed. A Wayland window's, the compositor gives back itself when the window that has it goes.
fn release_keyboard(server: &Server, window: u32) {
    let before = BEFORE.swap(0, Ordering::Relaxed);
    let connection = &server.connection;
    if GRABBED.swap(0, Ordering::Relaxed) != 0 {
        let _ = connection.ungrab_keyboard(x11rb::CURRENT_TIME);
        if let Some(stand_in) = keyboard_stand_in(server) {
            let _ = connection.unmap_window(stand_in);
        }
        let _ = connection.flush();
        return;
    }
    // On a Wayland desktop what has X11's keyboard while a Wayland window has the real one is the
    // compositor's own stand-in: handed back to that, nothing would have the keyboard.
    if before > 1 && (!on_wayland() || is_client(server, before)) {
        let done = connection.set_input_focus(InputFocus::POINTER_ROOT, before, x11rb::CURRENT_TIME);
        if done.is_ok_and(|cookie| cookie.check().is_ok()) {
            let _ = connection.flush();
            return;
        }
    }
    if on_wayland() {
        let _ = connection.unmap_window(window);
        let _ = connection.configure_window(window, &ConfigureWindowAux::new().x(AWAY).y(AWAY));
        let _ = connection.map_window(window);
        pill_after(server, window);
    } else {
        let _ = connection.set_input_focus(InputFocus::POINTER_ROOT, 1u32, x11rb::CURRENT_TIME);
    }
    let _ = connection.flush();
}

/// An app's X11 window, as the window manager lists them.
fn is_client(server: &Server, window: u32) -> bool {
    let Some(list) = atom(server, b"_NET_CLIENT_LIST") else { return false };
    let cookie = server.connection.get_property(false, server.root, list, AtomEnum::WINDOW, 0, u32::MAX);
    let reply = cookie.ok().and_then(|c| c.reply().ok());
    reply.is_some_and(|r| r.value32().is_some_and(|mut windows| windows.any(|w| w == window)))
}

/// Shows the pill's window, which doesn't take the keyboard.
pub fn appear(pointer: usize) {
    let Some(server) = server() else { return };
    bring(server, pointer);
    let _ = server.connection.flush();
}

/// The window is on screen.
pub fn on_screen(pointer: usize) -> bool {
    places().lock().unwrap().get(&pointer).is_some_and(|p| p.shown)
}

pub fn hide(pointer: usize) {
    let Some(server) = server() else { return };
    if focused(server) == pointer as u32 {
        release_keyboard(server, pointer as u32);
    }
    if let Some(place) = places().lock().unwrap().get_mut(&pointer) {
        place.shown = false;
    }
    let _ = server.connection.configure_window(pointer as u32, &ConfigureWindowAux::new().x(AWAY).y(AWAY));
    let _ = server.connection.flush();
}

/// The window's frame, in AppKit's coordinates: where an overlay's goes; another's where it is.
pub fn frame(pointer: usize) -> Option<PtRect> {
    if let Some(place) = places().lock().unwrap().get(&pointer) {
        return Some(place.frame);
    }
    let server = server()?;
    let size = server.connection.get_geometry(pointer as u32).ok()?.reply().ok()?;
    let at = server.connection.translate_coordinates(pointer as u32, server.root, 0, 0).ok()?.reply().ok()?;
    let rect = (at.dst_x as f64, at.dst_y as f64, size.width as f64, size.height as f64);
    Some(display::pixels_to_frame(&monitors(), rect))
}

/// Development (replays): what the X server shows in `frame`, as a PNG at `path`. On a Wayland
/// desktop that's only X11's windows (the overlay's).
pub fn capture(frame: PtRect, path: &Path) -> bool {
    let Some(server) = server() else { return false };
    let (x, y, w, h) = pixels(frame);
    if w <= 0 || h <= 0 {
        return false;
    }
    let image = server
        .connection
        .get_image(ImageFormat::Z_PIXMAP, server.root, x as i16, y as i16, w as u16, h as u16, !0)
        .ok()
        .and_then(|c| c.reply().ok());
    let Some(image) = image else { return false };
    let rgba: Vec<u8> = image.data.as_chunks::<4>().0.iter().flat_map(|&[b, g, r, _]| [r, g, b, 255]).collect();
    let size = tiny_skia::IntSize::from_wh(w as u32, h as u32);
    size.and_then(|size| tiny_skia::Pixmap::from_vec(rgba, size)).is_some_and(|image| image.save_png(path).is_ok())
}

/// Development (replays): only Windows' captures leave the pill out.
pub fn show_in_captures(_pointer: usize) {}

/// macOS keeps its glass to the pill's shapes; nothing blurs here (`windows::toolbar_options`).
pub fn set_glass(_pointer: usize, _shapes: &[(PtRect, f64, f64)]) {}

/// Moves the window, the same size.
pub fn move_to(pointer: usize, (x, y): (f64, f64)) {
    let Some(f) = frame(pointer) else { return };
    set_frame(pointer, PtRect::new(x, y, f.width, f.height));
}

/// Moves and resizes the window.
pub fn set_frame(pointer: usize, frame: PtRect) {
    let Some(server) = server() else { return };
    let shown = {
        let mut places = places().lock().unwrap();
        let Some(place) = places.get_mut(&pointer) else { return };
        place.frame = frame;
        place.shown
    };
    let (x, y, w, h) = pixels(frame);
    let (x, y) = if shown { (x, y) } else { (AWAY, AWAY) };
    let aux = ConfigureWindowAux::new().x(x).y(y).width(w.max(1) as u32).height(h.max(1) as u32);
    let _ = server.connection.configure_window(pointer as u32, &aux);
    let _ = server.connection.flush();
}

/// Tells the window where the pointer is now (it moved while the window was hidden): X11 says so only
/// as the pointer next moves over it, and Measure follows it from there.
pub fn pointer_moved(pointer: usize, cx: &mut App) {
    let Some(frame) = frame(pointer) else { return };
    if on_wayland() && !POINTER_INSIDE.load(Ordering::Relaxed) {
        return;
    }
    let (mx, my) = mouse();
    let position = gpui::point(gpui::px((mx - frame.x) as f32), gpui::px((frame.y + frame.height - my) as f32));
    for handle in cx.windows() {
        let _ = handle.update(cx, |_, window, cx| {
            if view_pointer(window) == pointer {
                let event = gpui::MouseMoveEvent { position, pressed_button: None, modifiers: Default::default() };
                let _ = window.dispatch_event(gpui::PlatformInput::MouseMove(event), cx);
            }
        });
    }
}

/// Frozen: nothing a desktop lets an app hold back. The keyboard, though, comes here.
pub fn kiosk(on: bool) {
    if on {
        activate();
    }
}

/// Brings the overlay's frozen screen forward, with the keyboard. (The asides window's process has
/// none; it asks through GPUI.)
pub fn activate() {
    let Some(server) = server() else { return };
    let pill = PILL.load(Ordering::Relaxed) as usize;
    let shown = {
        let places = places().lock().unwrap();
        places.iter().find(|(id, p)| p.shown && **id != pill).map(|(id, _)| *id)
    };
    if let Some(window) = shown {
        take_keyboard(server, window as u32);
    }
}

/// A click on a frozen screen: unless it has the keyboard already, it takes it, or what's typed into
/// the card just opened there would go to the screen that has it.
pub fn click_takes_keyboard(pointer: usize) {
    let Some(server) = server() else { return };
    if focused(server) != pointer as u32 {
        take_keyboard(server, pointer as u32);
    }
}

/// The overlay has the keyboard. (A process with no overlay windows, the asides window's, is left to
/// GPUI.)
pub fn is_active() -> bool {
    let Some(server) = server() else { return true };
    let places = places().lock().unwrap();
    places.is_empty() || places.contains_key(&(focused(server) as usize))
}

/// The app in front (its process id), unless it's the overlay's: as the window manager says, if it's
/// an X11 window.
pub fn front_app() -> Option<i32> {
    let server = server()?;
    let active = atom(server, b"_NET_ACTIVE_WINDOW")?;
    let cookie = server.connection.get_property(false, server.root, active, AtomEnum::WINDOW, 0, 1).ok()?;
    let window = cookie.reply().ok()?.value32()?.next().filter(|&w| w != 0)?;
    let pid_atom = atom(server, b"_NET_WM_PID")?;
    let cookie = server.connection.get_property(false, window, pid_atom, AtomEnum::CARDINAL, 0, 1).ok()?;
    let pid = cookie.reply().ok()?.value32()?.next()?;
    (pid != std::process::id()).then_some(pid as i32)
}

/// Gives the keyboard back, if the overlay has it, to what had it (`release_keyboard`).
pub fn give_focus(_pid: Option<i32>) {
    let Some(server) = server() else { return };
    let now = focused(server);
    if places().lock().unwrap().contains_key(&(now as usize)) {
        release_keyboard(server, now);
    }
}

/// The folder picker couldn't open.
static NO_PICKER: AtomicBool = AtomicBool::new(false);

/// Whether the last folder picker couldn't open (which looks like one cancelled).
pub fn no_folder_picker() -> bool {
    NO_PICKER.swap(false, Ordering::Relaxed)
}

/// The folder picker: the desktop's, through its portal. The overlay's windows step aside: they're
/// above everything, and the dialog is the compositor's to place. Its answer is waited for here.
pub fn choose_folder(start: Option<&Path>, done: impl FnOnce(Option<PathBuf>) + 'static) {
    let shown: Vec<usize> = places().lock().unwrap().iter().filter(|(_, p)| p.shown).map(|(id, _)| *id).collect();
    if let Some(server) = server() {
        for &id in &shown {
            let _ = server.connection.configure_window(id as u32, &ConfigureWindowAux::new().x(AWAY).y(AWAY));
        }
        let _ = server.connection.flush();
        let now = focused(server);
        if shown.iter().any(|&id| id as u32 == now) {
            release_keyboard(server, now);
        }
    }
    let picked = pick_folder(start);
    if let Some(server) = server() {
        for &id in &shown {
            bring(server, id);
        }
        let _ = server.connection.flush();
    }
    done(picked);
}

fn pick_folder(start: Option<&Path>) -> Option<PathBuf> {
    use asides_core::portal;
    use std::collections::HashMap;
    use std::os::unix::ffi::OsStrExt;
    use zbus::zvariant::{Array, Value};
    let wait = std::time::Duration::from_secs(3600);
    let results = portal::request(
        "org.freedesktop.portal.FileChooser",
        "OpenFile",
        |token| {
            let mut options: HashMap<&str, Value> = HashMap::new();
            options.insert("handle_token", Value::from(token.to_string()));
            options.insert("directory", Value::from(true));
            options.insert("accept_label", Value::from("Start Here"));
            if let Some(start) = start {
                let mut bytes = start.as_os_str().as_bytes().to_vec();
                bytes.push(0);
                options.insert("current_folder", Value::from(Array::from(bytes)));
            }
            ("", "The folder the new session starts in", options)
        },
        wait,
    );
    let results = match results {
        Ok(results) => results,
        Err(err) => {
            if err != "declined" {
                eprintln!("asides-overlay: no folder picker: {err}");
                NO_PICKER.store(true, Ordering::Relaxed);
            }
            return None;
        }
    };
    let uris = results.get("uris")?;
    let uris: Vec<String> = uris.try_clone().ok()?.try_into().ok()?;
    url::Url::parse(uris.first()?).ok()?.to_file_path().ok()
}

/// The screens: each one's number (the app's) and frame (AppKit's coordinates).
pub fn screens() -> Vec<(u32, PtRect)> {
    let monitors = monitors();
    monitors.iter().enumerate().map(|(i, m)| (i as u32, display::frame(&monitors, m))).collect()
}

/// The part of the screen at `frame` (AppKit's coordinates) that the panels leave.
pub fn visible_frame(frame: PtRect) -> PtRect {
    let listed = match server() {
        Some(s) => display::monitors_on(&s.connection, s.screen),
        None => display::monitors(),
    };
    let all: Vec<Monitor> = listed.iter().map(|l| l.monitor).collect();
    let found = listed.iter().find(|l| {
        let f = display::frame(&all, &l.monitor);
        (f.x - frame.x).abs() < 1.0 && (f.y - frame.y).abs() < 1.0
    });
    match found {
        Some(l) => {
            let (left, top, right, bottom) = l.work;
            display::pixels_to_frame(&all, (left as f64, top as f64, (right - left) as f64, (bottom - top) as f64))
        }
        None => frame,
    }
}

/// The left mouse button is down (anywhere X11 can see).
pub fn mouse_pressed() -> bool {
    let Some(server) = server() else { return false };
    let Some(pointer) = server.connection.query_pointer(server.root).ok().and_then(|c| c.reply().ok()) else {
        return false;
    };
    u16::from(pointer.mask) & u16::from(x11rb::protocol::xproto::KeyButMask::BUTTON1) != 0
}

/// macOS's run loop tells its watchdog what the main thread does; GPUI's loop here has no such
/// observer.
pub fn watch_main_thread(_give_up: Option<std::time::Duration>) {}

/// Development: with `asides-overlay-debug` in the temporary directory, notes what the overlay
/// gets (keys, focus) in `asides-overlay-debug.log` there, for when it runs under the app.
pub fn debug(message: &str) {
    use std::io::Write;
    let dir = std::env::temp_dir();
    if !dir.join("asides-overlay-debug").exists() {
        return;
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("asides-overlay-debug.log"))
    {
        let ms =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() % 100_000);
        let _ = writeln!(f, "{ms} {message} [active={}]", is_active());
    }
}

pub fn monitor_keys() {}

/// GPUI's Metal pipelines are macOS's.
pub fn blend_alpha_over() {}

/// The connection that hears what the overlay waits on.
static WATCHER: OnceLock<Arc<RustConnection>> = OnceLock::new();
/// Whether the pointer's moves matter (`watch_pointer`).
static POINTER_WATCHED: AtomicBool = AtomicBool::new(false);
/// The pointer is over one of the overlay's windows: on a Wayland desktop, the X server knows where
/// it is only then.
static POINTER_INSIDE: AtomicBool = AtomicBool::new(false);

/// This connection hears `window` too (X11 keeps what each client hears apart: GPUI's stays as it is).
fn watch_window(window: u32) {
    let Some(watcher) = WATCHER.get() else { return };
    let mask =
        EventMask::FOCUS_CHANGE | EventMask::ENTER_WINDOW | EventMask::LEAVE_WINDOW | EventMask::STRUCTURE_NOTIFY;
    let _ = watcher.change_window_attributes(window, &ChangeWindowAttributesAux::new().event_mask(mask));
    let _ = watcher.flush();
}

/// Calls `changed` when the X server says what the overlay waits on may have changed: one of its
/// windows showed, hid, or got or lost the keyboard, the window in front or the panels changed, or
/// the pointer moved while `watch_pointer` asks.
pub fn on_system_changes(changed: fn()) {
    let spawned = std::thread::Builder::new().name("asides-overlay-x11".into()).spawn(move || {
        let Ok((connection, screen)) = x11rb::connect(None) else { return };
        let Some(root) = connection.setup().roots.get(screen).map(|s| s.root) else { return };
        let connection = Arc::new(connection);
        let _ = connection.xinput_xi_query_version(2, 0).map(|c| c.reply());
        let _ = connection
            .change_window_attributes(root, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE));
        let _ = WATCHER.set(connection.clone());
        let known: Vec<usize> = places().lock().unwrap().keys().copied().collect();
        known.into_iter().for_each(|id| watch_window(id as u32));
        let _ = connection.flush();
        while let Ok(event) = connection.wait_for_event() {
            use x11rb::protocol::Event;
            match event {
                Event::EnterNotify(_) => POINTER_INSIDE.store(true, Ordering::Relaxed),
                Event::LeaveNotify(_) => POINTER_INSIDE.store(false, Ordering::Relaxed),
                Event::XinputRawMotion(_) if !POINTER_WATCHED.load(Ordering::Relaxed) => continue,
                _ => {}
            }
            changed();
        }
    });
    if let Err(err) = spawned {
        eprintln!("asides-overlay: not told what changes: {err}");
    }
}

/// Whether the pointer's moves wake the clock.
pub fn watch_pointer(watched: bool) {
    if POINTER_WATCHED.swap(watched, Ordering::Relaxed) == watched {
        return;
    }
    let (Some(watcher), Some(server)) = (WATCHER.get(), server()) else { return };
    let mask = if watched { vec![xinput::XIEventMask::RAW_MOTION] } else { vec![0u32.into()] };
    let events = xinput::EventMask { deviceid: 1, mask };
    let _ = watcher.xinput_xi_select_events(server.root, &[events]);
    let _ = watcher.flush();
}

pub fn debug_focus() {}

/// The mouse, in AppKit's global coordinates. On a Wayland desktop, off every screen unless it's over
/// one of the overlay's windows: the X server knows nothing better.
pub fn mouse() -> (f64, f64) {
    const NOWHERE: (f64, f64) = (-1e7, -1e7);
    let Some(server) = server() else { return NOWHERE };
    if on_wayland() && !POINTER_INSIDE.load(Ordering::Relaxed) {
        return NOWHERE;
    }
    let Some(pointer) = server.connection.query_pointer(server.root).ok().and_then(|c| c.reply().ok()) else {
        return NOWHERE;
    };
    let monitors = monitors();
    let (x, y) = display::to_points(&monitors, (pointer.root_x as f64, pointer.root_y as f64));
    (x, display::flip_y(&monitors, y, 0.0))
}

/// Development (replay): input sent through the X server (XTEST), as the mouse and keyboard would:
/// it comes to the window under the pointer, or with the keyboard.
pub mod synthetic {
    use asides_core::keys::Chord;
    use x11rb::connection::Connection as _;
    use x11rb::protocol::xproto::{
        BUTTON_PRESS_EVENT, BUTTON_RELEASE_EVENT, ConnectionExt as _, KEY_PRESS_EVENT, KEY_RELEASE_EVENT,
        MOTION_NOTIFY_EVENT,
    };
    use x11rb::protocol::xtest::ConnectionExt as _;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mouse {
        Move,
        Down { right: bool },
        Drag { right: bool },
        Up { right: bool },
    }

    const SHIFT: u32 = 0xffe1;
    const CONTROL: u32 = 0xffe3;
    const ALT: u32 = 0xffe9;

    /// The key code that types `sym`, and whether it needs Shift.
    fn code(server: &super::Server, sym: u32) -> Option<(u8, bool)> {
        let setup = server.connection.setup();
        let (first, count) = (setup.min_keycode, setup.max_keycode - setup.min_keycode + 1);
        let map = server.connection.get_keyboard_mapping(first, count).ok()?.reply().ok()?;
        let per = map.keysyms_per_keycode.max(1) as usize;
        map.keysyms.chunks(per).enumerate().find_map(|(i, syms)| {
            let level = syms.iter().take(2).position(|&s| s == sym)?;
            Some((first + i as u8, level == 1))
        })
    }

    fn fake(server: &super::Server, kind: u8, detail: u8) {
        let _ = server.connection.xtest_fake_input(kind, detail, 0, server.root, 0, 0, 0);
    }

    fn fake_key(server: &super::Server, sym: u32, down: bool) {
        if let Some((code, _)) = code(server, sym) {
            fake(server, if down { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT }, code);
        }
    }

    /// The modifiers held around `send`.
    fn holding(server: &super::Server, mods: &[(bool, u32)], send: impl FnOnce()) {
        mods.iter().filter(|(on, _)| *on).for_each(|(_, sym)| fake_key(server, *sym, true));
        send();
        mods.iter().filter(|(on, _)| *on).for_each(|(_, sym)| fake_key(server, *sym, false));
        let _ = server.connection.flush();
    }

    /// A mouse event at `(x, y)` (window points, y down).
    pub fn mouse(window: usize, kind: Mouse, (x, y): (f64, f64), shift: bool, alt: bool, cmd: bool) {
        let Some(server) = super::server() else { return };
        let Some(frame) = super::frame(window) else { return };
        let monitors = super::monitors();
        let scale = monitors.first().map_or(1.0, |m| m.scale);
        let (left, top, _, _) = super::pixels(frame);
        let (px, py) = (left + (x * scale).round() as i32, top + (y * scale).round() as i32);
        let button = |right: bool| if right { 3 } else { 1 };
        let _ = server.connection.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, 0, server.root, px as i16, py as i16, 0);
        holding(server, &[(shift, SHIFT), (alt, ALT), (cmd, CONTROL)], || match kind {
            Mouse::Move | Mouse::Drag { .. } => {}
            Mouse::Down { right } => fake(server, BUTTON_PRESS_EVENT, button(right)),
            Mouse::Up { right } => fake(server, BUTTON_RELEASE_EVENT, button(right)),
        });
    }

    /// The keysym of a key the replays name.
    fn keysym(key: &str) -> Option<u32> {
        Some(match key {
            "enter" | "\n" => 0xff0d,
            "tab" => 0xff09,
            "space" | " " => 0x20,
            "backspace" => 0xff08,
            "escape" => 0xff1b,
            "left" => 0xff51,
            "up" => 0xff52,
            "right" => 0xff53,
            "down" => 0xff54,
            "f1" => 0xffbe,
            "home" => 0xff50,
            "end" => 0xff57,
            "delete" => 0xffff,
            k if k.chars().count() == 1 => k.chars().next()? as u32,
            _ => return None,
        })
    }

    /// A key going down or up (`chord` like `shift+h`; ⌘ is Ctrl); `held`, a repeat of it going down.
    pub fn key(_window: usize, chord: &Chord, down: bool, _held: bool) {
        let Some(server) = super::server() else { return };
        let Some(sym) = keysym(&chord.key) else { return };
        let Some((code, needs_shift)) = code(server, sym) else { return };
        let mods = [(chord.shift || needs_shift, SHIFT), (chord.ctrl || chord.cmd, CONTROL), (chord.alt, ALT)];
        holding(server, &mods, || fake(server, if down { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT }, code));
    }

    /// Typing `text`, a key at a time.
    pub fn text(_window: usize, text: &str) {
        let Some(server) = super::server() else { return };
        for c in text.chars() {
            let Some((code, needs_shift)) = keysym(&c.to_string()).and_then(|s| code(server, s)) else { continue };
            holding(server, &[(needs_shift, SHIFT)], || {
                fake(server, KEY_PRESS_EVENT, code);
                fake(server, KEY_RELEASE_EVENT, code);
            });
        }
    }
}
