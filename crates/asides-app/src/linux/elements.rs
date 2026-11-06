//! What's under a pin on Linux, as `elements.rs` finds it on a Mac. On X11 the window manager's list
//! (taken when the screen freezes) names the app and the window. AT-SPI, the desktop's accessibility
//! bus, names the element itself: its role and label, its frame, and in web pages the DOM id and
//! classes and the page's address. It asks no permission; apps answer if their toolkit does (GTK
//! always; Qt, Firefox and Chromium when the desktop's accessibility is on).
//!
//! A Wayland desktop says nowhere where windows are, to X11 or AT-SPI: there a pin is its point and
//! the screenshot, and `place` still names the window in front, which AT-SPI marks active.
//!
//! AT-SPI's role names are given as macOS's accessibility roles (`AXButton`), so what's made of them
//! (`inspect::Element`) reads the same everywhere. An app can take a moment to answer: AT-SPI runs on
//! a thread of its own, and a question not answered in time gets the window alone.
//! Coordinates are global points (`asides_core::display`).

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use asides_core::display::{self, Monitor};
use asides_core::inspect::{Element, Node, PtRect};
use x11rb::connection::Connection as _;
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, Window};
use zbus::blocking::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue};

use crate::capture::on_wayland;
use crate::system::{self, FRONT_UNKNOWN};

/// How long an app may take to answer a question, and how long asides waits for all of them.
const CALL_TIMEOUT: Duration = Duration::from_millis(300);
const ANSWER_WAIT: Duration = Duration::from_millis(700);
/// How many ancestors to read above the element, and how deep the search under a point goes.
const MAX_DEPTH: usize = 14;
const MAX_SEARCH: usize = 48;
/// A window above the others (a panel): macOS's layers above 0.
const ABOVE: i64 = 25;

const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
const COMPONENT: &str = "org.a11y.atspi.Component";
const ROOT: &str = "/org/a11y/atspi/accessible/root";
const NULL: &str = "/org/a11y/atspi/null";
/// `ATSPI_COORD_TYPE_SCREEN`.
const SCREEN: u32 = 0;
/// `ATSPI_STATE_ACTIVE`.
const ACTIVE: u32 = 1;

/// A window on screen, front to back.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub pid: i32,
    pub app: String,
    pub title: Option<String>,
    pub bounds: PtRect,
    pub layer: i64,
}

/// The monitors, for the coordinates.
fn monitors() -> Vec<Monitor> {
    display::monitors().into_iter().map(|l| l.monitor).collect()
}

/// The windows on screen now, frontmost first, without this app's or process `without`'s (the
/// overlay's). On a Wayland desktop, Xwayland's windows lie about what's in front, and the others
/// aren't there: only an X11 window that's the active one can be named.
pub fn windows(without: Option<i32>) -> Vec<WindowInfo> {
    let Some((connection, screen)) = display::connect_x11() else { return Vec::new() };
    let Some(root) = connection.setup().roots.get(screen).map(|s| s.root) else { return Vec::new() };
    let atom = |name: &[u8]| -> Option<Atom> { connection.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom) };
    let Some(list) = atom(b"_NET_CLIENT_LIST_STACKING") else { return Vec::new() };
    let Some(reply) = connection
        .get_property(false, root, list, AtomEnum::WINDOW, 0, 4096)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    else {
        return Vec::new();
    };
    let Some(stack) = reply.value32() else { return Vec::new() };
    let stack: Vec<Window> = stack.collect();
    let active = atom(b"_NET_ACTIVE_WINDOW").and_then(|a| {
        let cookie = connection.get_property(false, root, a, AtomEnum::WINDOW, 0, 1).ok()?;
        cookie.reply().ok()?.value32()?.next()
    });
    let names = Names {
        title: atom(b"_NET_WM_NAME"),
        utf8: atom(b"UTF8_STRING"),
        kind: atom(b"_NET_WM_WINDOW_TYPE"),
        dock: atom(b"_NET_WM_WINDOW_TYPE_DOCK"),
        desktop: atom(b"_NET_WM_WINDOW_TYPE_DESKTOP"),
        state: atom(b"_NET_WM_STATE"),
        hidden: atom(b"_NET_WM_STATE_HIDDEN"),
        frame: atom(b"_GTK_FRAME_EXTENTS"),
    };
    let wayland = on_wayland();
    let all = display::monitors_on(&connection, screen).into_iter().map(|l| l.monitor).collect::<Vec<_>>();
    let own = std::process::id() as i32;
    stack
        .into_iter()
        .rev()
        .filter(|&w| !wayland || Some(w) == active)
        .filter_map(|window| {
            let pid = system::window_pid(&connection, window).unwrap_or(0);
            (pid != own && Some(pid) != without).then(|| window_info(&connection, root, window, pid, &names, &all))?
        })
        .collect()
}

/// The atoms a window is read by.
struct Names {
    title: Option<Atom>,
    utf8: Option<Atom>,
    kind: Option<Atom>,
    dock: Option<Atom>,
    desktop: Option<Atom>,
    state: Option<Atom>,
    hidden: Option<Atom>,
    frame: Option<Atom>,
}

fn window_info(
    connection: &impl x11rb::connection::Connection,
    root: Window,
    window: Window,
    pid: i32,
    names: &Names,
    monitors: &[Monitor],
) -> Option<WindowInfo> {
    let atoms = |name: Option<Atom>| -> Vec<u32> {
        name.and_then(|name| {
            let cookie = connection.get_property(false, window, name, AtomEnum::ANY, 0, 32).ok()?;
            Some(cookie.reply().ok()?.value32()?.collect())
        })
        .unwrap_or_default()
    };
    if atoms(names.state).iter().any(|a| Some(*a) == names.hidden) {
        return None;
    }
    let kinds = atoms(names.kind);
    if kinds.iter().any(|a| Some(*a) == names.desktop) {
        return None;
    }
    let layer = if kinds.iter().any(|a| Some(*a) == names.dock) { ABOVE } else { 0 };
    let size = connection.get_geometry(window).ok()?.reply().ok()?;
    let corner = connection.translate_coordinates(window, root, 0, 0).ok()?.reply().ok()?;
    // A GTK window's shadow is part of it: what it says it is, is inside that.
    let frame = atoms(names.frame);
    let [left, right, top, bottom] = frame[..] else {
        let rect = (corner.dst_x as f64, corner.dst_y as f64, size.width as f64, size.height as f64);
        return Some(info(connection, window, pid, names, monitors, rect, layer));
    };
    let rect = (
        corner.dst_x as f64 + left as f64,
        corner.dst_y as f64 + top as f64,
        (size.width as f64 - (left + right) as f64).max(0.0),
        (size.height as f64 - (top + bottom) as f64).max(0.0),
    );
    Some(info(connection, window, pid, names, monitors, rect, layer))
}

fn info(
    connection: &impl x11rb::connection::Connection,
    window: Window,
    pid: i32,
    names: &Names,
    monitors: &[Monitor],
    rect: (f64, f64, f64, f64),
    layer: i64,
) -> WindowInfo {
    let text = |name: Atom, kind: Atom| -> Option<String> {
        let cookie = connection.get_property(false, window, name, kind, 0, 1024).ok()?;
        let bytes = cookie.reply().ok()?.value;
        (!bytes.is_empty()).then(|| String::from_utf8_lossy(&bytes).into_owned())
    };
    let title = names
        .title
        .zip(names.utf8)
        .and_then(|(name, utf8)| text(name, utf8))
        .or_else(|| text(AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()));
    // `WM_CLASS` is an instance and a class, each nul-terminated: the class names the app.
    let class = text(AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())
        .and_then(|c| c.split('\0').rfind(|p| !p.is_empty()).map(String::from));
    WindowInfo {
        pid,
        app: class.unwrap_or_else(|| system::app_name(pid)),
        title,
        bounds: display::rect_to_points(monitors, rect),
        layer,
    }
}

/// The frontmost window at a point: an app's own window if there is one, else anything (a panel).
pub fn window_at(windows: &[WindowInfo], x: f64, y: f64) -> Option<&WindowInfo> {
    windows
        .iter()
        .find(|w| w.layer == 0 && w.bounds.contains(x, y))
        .or_else(|| windows.iter().find(|w| w.bounds.contains(x, y)))
}

/// AT-SPI needs no permission.
pub fn trusted() -> bool {
    true
}

pub fn request_trust() -> bool {
    true
}

/// What's at the global point `(x, y)`: the window (always), and with `deep` the element under it,
/// its frame in global points. On a Wayland desktop, where nothing says what's at a point: nothing.
pub fn identify(windows: &[WindowInfo], x: f64, y: f64, deep: bool) -> Option<Element> {
    let window = window_at(windows, x, y)?;
    let nodes = if deep { element_at(window.pid, x, y) } else { Vec::new() };
    Some(Element::from_nodes(Some(window.app.clone()), window.title.clone(), &nodes))
}

/// Where the user is in the app with process id `pid` (or, not knowing it, the app whose window is
/// active): its frontmost window, and with `deep` the page in it, as `**In:** Firefox · "Checkout —
/// Shop" · http://…`, and that window's frame.
pub fn place(pid: i32, deep: bool) -> Option<(String, PtRect)> {
    if pid == FRONT_UNKNOWN {
        return with_atspi(active_place).flatten();
    }
    let windows = windows(None);
    let window = windows.iter().find(|w| w.pid == pid && w.layer == 0)?;
    let b = window.bounds;
    let nodes = if deep { element_at(pid, b.x + b.width / 2.0, b.y + b.height / 2.0) } else { Vec::new() };
    let element = Element::from_nodes(Some(window.app.clone()), window.title.clone(), &nodes);
    let place = asides_core::inspect::place_markdown(&window.app, window.title.as_deref(), element.url.as_deref());
    Some((place, b))
}

/// Starts AT-SPI ahead of the first pin: its first answer can take longer than a pin waits for one.
pub fn warm_up() {
    let spawned = std::thread::Builder::new().name("asides-atspi-warm-up".into()).spawn(|| {
        with_atspi(|_| ());
    });
    if let Err(err) = spawned {
        eprintln!("asides: AT-SPI starts with the first pin: {err}");
    }
}

type Job = Box<dyn FnOnce(&Connection) + Send>;

/// The accessibility bus, as the desktop says where it is.
fn connect() -> Option<Connection> {
    let session = Connection::session().ok()?;
    let reply =
        session.call_method(Some("org.a11y.Bus"), "/org/a11y/bus", Some("org.a11y.Bus"), "GetAddress", &()).ok()?;
    let address: String = reply.body().deserialize().ok()?;
    zbus::blocking::connection::Builder::address(address.as_str()).ok()?.method_timeout(CALL_TIMEOUT).build().ok()
}

/// Runs `job` on AT-SPI's thread, and waits for its answer (`None` if it's late).
fn with_atspi<T: Send + 'static>(job: impl FnOnce(&Connection) -> T + Send + 'static) -> Option<T> {
    static THREAD: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let jobs = THREAD.get_or_init(|| {
        let (jobs, asked) = channel::<Job>();
        let _ = std::thread::Builder::new().name("asides-atspi".into()).spawn(move || {
            let Some(bus) = connect() else { return };
            for job in asked {
                job(&bus);
            }
        });
        Mutex::new(jobs)
    });
    let (answer, answered) = channel();
    let job: Job = Box::new(move |bus| {
        let _ = answer.send(job(bus));
    });
    jobs.lock().ok()?.send(job).ok()?;
    answered.recv_timeout(ANSWER_WAIT).ok()
}

/// An object on the accessibility bus: its owner, and its path there.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, zbus::zvariant::Type)]
struct Accessible(String, OwnedObjectPath);

impl Accessible {
    fn is_null(&self) -> bool {
        self.1.as_str() == NULL || self.0.is_empty()
    }

    fn call<R: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type>(
        &self,
        bus: &Connection,
        interface: &str,
        method: &str,
        args: &(impl serde::Serialize + zbus::zvariant::DynamicType),
    ) -> Option<R> {
        let reply = bus.call_method(Some(self.0.as_str()), self.1.as_str(), Some(interface), method, args).ok()?;
        reply.body().deserialize().ok()
    }

    fn property(&self, bus: &Connection, interface: &str, name: &str) -> Option<OwnedValue> {
        let reply = bus
            .call_method(
                Some(self.0.as_str()),
                self.1.as_str(),
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(interface, name),
            )
            .ok()?;
        reply.body().deserialize().ok()
    }

    fn text_property(&self, bus: &Connection, interface: &str, name: &str) -> Option<String> {
        let value = self.property(bus, interface, name)?;
        let text: &str = value.downcast_ref().ok()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_string())
    }

    fn children(&self, bus: &Connection) -> Vec<Accessible> {
        self.call(bus, ACCESSIBLE, "GetChildren", &()).unwrap_or_default()
    }

    fn parent(&self, bus: &Connection) -> Option<Accessible> {
        let value = self.property(bus, ACCESSIBLE, "Parent")?;
        let parent: Accessible = value
            .downcast_ref::<zbus::zvariant::Structure>()
            .ok()?
            .try_into()
            .ok()
            .map(|(owner, path): (String, OwnedObjectPath)| Accessible(owner, path))?;
        (!parent.is_null()).then_some(parent)
    }

    fn role_name(&self, bus: &Connection) -> String {
        self.call(bus, ACCESSIBLE, "GetRoleName", &()).unwrap_or_default()
    }

    fn has_state(&self, bus: &Connection, state: u32) -> bool {
        let states: Vec<u32> = self.call(bus, ACCESSIBLE, "GetState", &()).unwrap_or_default();
        states.get((state / 32) as usize).is_some_and(|word| word & (1 << (state % 32)) != 0)
    }

    fn extents(&self, bus: &Connection) -> Option<(f64, f64, f64, f64)> {
        let (x, y, w, h): (i32, i32, i32, i32) = self.call(bus, COMPONENT, "GetExtents", &(SCREEN,))?;
        (w > 0 && h > 0).then_some((x as f64, y as f64, w as f64, h as f64))
    }
}

/// The apps on the accessibility bus.
fn apps(bus: &Connection) -> Vec<Accessible> {
    let registry = match OwnedObjectPath::try_from(ROOT) {
        Ok(path) => Accessible("org.a11y.atspi.Registry".into(), path),
        Err(_) => return Vec::new(),
    };
    registry.children(bus)
}

/// The accessible app of process `pid`.
fn app_of(bus: &Connection, pid: i32) -> Option<Accessible> {
    apps(bus).into_iter().find(|app| {
        bus.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus"),
            "GetConnectionUnixProcessID",
            &(app.0.as_str(),),
        )
        .ok()
        .and_then(|reply| reply.body().deserialize::<u32>().ok())
            == Some(pid as u32)
    })
}

/// The window that's active on the accessibility bus, as a place, on the main screen: nothing says
/// where it is.
fn active_place(bus: &Connection) -> Option<(String, PtRect)> {
    let own = std::process::id();
    for app in apps(bus) {
        if app.children(bus).is_empty() {
            continue;
        }
        for window in app.children(bus) {
            if !window.has_state(bus, ACTIVE) {
                continue;
            }
            let pid: Option<u32> = bus
                .call_method(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    Some("org.freedesktop.DBus"),
                    "GetConnectionUnixProcessID",
                    &(app.0.as_str(),),
                )
                .ok()
                .and_then(|reply| reply.body().deserialize().ok());
            if pid == Some(own) {
                continue;
            }
            let name = app.text_property(bus, ACCESSIBLE, "Name").unwrap_or_else(|| "an app".into());
            let title = window.text_property(bus, ACCESSIBLE, "Name");
            return Some((
                asides_core::inspect::place_markdown(&name, title.as_deref(), None),
                PtRect::new(0.0, 0.0, 1.0, 1.0),
            ));
        }
    }
    None
}

/// The element of the app of process `pid` at the global point `(x, y)` and its ancestors up to the
/// window, innermost first.
pub fn element_at(pid: i32, x: f64, y: f64) -> Vec<Node> {
    let monitors = monitors();
    let at = display::to_pixels(&monitors, (x, y));
    let at = (at.0 as i32, at.1 as i32);
    with_atspi(move |bus| nodes_at(bus, &monitors, pid, at)).unwrap_or_default()
}

fn nodes_at(bus: &Connection, monitors: &[Monitor], pid: i32, at: (i32, i32)) -> Vec<Node> {
    let Some(app) = app_of(bus, pid) else { return Vec::new() };
    let inside = |w: &Accessible| {
        w.extents(bus).is_some_and(|(x, y, width, height)| {
            let (px, py) = (at.0 as f64, at.1 as f64);
            px >= x && px < x + width && py >= y && py < y + height
        })
    };
    let windows = app.children(bus);
    let Some(window) = windows
        .iter()
        .find(|w| w.has_state(bus, ACTIVE) && inside(w))
        .or_else(|| windows.iter().find(|w| inside(w)))
        .cloned()
    else {
        return Vec::new();
    };
    let mut hit = window.clone();
    for _ in 0..MAX_SEARCH {
        let next: Option<Accessible> = hit.call(bus, COMPONENT, "GetAccessibleAtPoint", &(at.0, at.1, SCREEN));
        match next {
            Some(next) if !next.is_null() && next != hit => hit = next,
            _ => break,
        }
    }
    let mut nodes = Vec::new();
    let mut current = Some(hit);
    while let Some(element) = current {
        nodes.push(node(bus, &element, monitors));
        if nodes.len() >= MAX_DEPTH || element == window {
            break;
        }
        current = element.parent(bus);
    }
    nodes
}

fn node(bus: &Connection, element: &Accessible, monitors: &[Monitor]) -> Node {
    let name = element.role_name(bus);
    let attributes: HashMap<String, String> = element.call(bus, ACCESSIBLE, "GetAttributes", &()).unwrap_or_default();
    let level = attributes.get("level").and_then(|l| l.parse().ok()).filter(|l| (1..=9).contains(l));
    let role = role(&name, level);
    let is_web =
        attributes.contains_key("tag") || attributes.contains_key("xml-roles") || attributes.contains_key("class");
    let id = attributes.get("id").filter(|id| !id.is_empty()).cloned();
    let identifier = element.text_property(bus, ACCESSIBLE, "AccessibleId");
    let url: Option<String> = (role == "AXWebArea")
        .then(|| element.call(bus, "org.a11y.atspi.Document", "GetAttributeValue", &("DocURL",)))
        .flatten()
        .filter(|u: &String| u.contains("://"));
    let value: Option<String> = matches!(role, "AXTextField" | "AXTextArea")
        .then(|| element.call(bus, "org.a11y.atspi.Text", "GetText", &(0i32, -1i32)))
        .flatten()
        .map(|v: String| v.trim().to_string())
        .filter(|v| !v.is_empty());
    Node {
        role: role.to_string(),
        subrole: None,
        role_description: (!name.is_empty()).then_some(name),
        title: element.text_property(bus, ACCESSIBLE, "Name"),
        description: element.text_property(bus, ACCESSIBLE, "Description"),
        value,
        placeholder: attributes.get("placeholder-text").cloned().filter(|p| !p.is_empty()),
        identifier: identifier.or_else(|| id.clone().filter(|_| !is_web)),
        dom_id: id.filter(|_| is_web),
        dom_classes: attributes
            .get("class")
            .map(|c| c.split_whitespace().map(String::from).collect())
            .unwrap_or_default(),
        level,
        url,
        frame: element.extents(bus).map(|rect| display::rect_to_points(monitors, rect)),
        selected_text: None,
    }
}

/// AT-SPI's role name as macOS's accessibility role.
fn role(name: &str, level: Option<u32>) -> &'static str {
    match name {
        "push button" | "button" | "toggle button" | "push button menu" => "AXButton",
        "menu button" => "AXMenuButton",
        "check box" | "check menu item" | "switch" => "AXCheckBox",
        "radio button" | "radio menu item" => "AXRadioButton",
        "combo box" => "AXPopUpButton",
        "spin button" => "AXIncrementor",
        "entry" | "password text" | "editbar" | "search box" => "AXTextField",
        "text" => "AXTextArea",
        "link" | "hyperlink" => "AXLink",
        "image" | "icon" | "image map" | "canvas" => "AXImage",
        "list" | "list box" => "AXList",
        "list item" => "AXGroup",
        "menu" | "popup menu" => "AXMenu",
        "menu bar" => "AXMenuBar",
        "menu item" | "tearoff menu item" => "AXMenuItem",
        "progress bar" | "level bar" => "AXProgressIndicator",
        "scroll bar" => "AXScrollBar",
        "slider" => "AXSlider",
        "page tab list" => "AXTabGroup",
        "page tab" => "AXTab",
        "heading" => "AXHeading",
        "label" | "static" | "caption" | "paragraph" if level.is_some() => "AXHeading",
        "label" | "static" | "caption" | "paragraph" => "AXStaticText",
        "tool bar" => "AXToolbar",
        "tool tip" => "AXHelpTag",
        "tree" | "tree table" => "AXOutline",
        "tree item" | "table row" | "row" => "AXRow",
        "table" | "grid" => "AXTable",
        "table cell" | "cell" | "table column header" | "table row header" | "column header" | "row header" => "AXCell",
        "document web" | "document frame" => "AXWebArea",
        "document text" | "terminal" | "document email" | "document presentation" | "document spreadsheet" => {
            "AXTextArea"
        }
        "frame" | "window" | "dialog" | "alert" | "file chooser" => "AXWindow",
        "separator" => "AXSplitter",
        _ if level.is_some() => "AXHeading",
        _ => "AXGroup",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_roles_as_macos_does() {
        assert_eq!(role("push button", None), "AXButton");
        assert_eq!(role("document web", None), "AXWebArea");
        assert_eq!(role("heading", Some(2)), "AXHeading");
        assert_eq!(role("entry", None), "AXTextField");
        assert_eq!(role("spin button", None), "AXIncrementor");
        assert_eq!(role("panel", None), "AXGroup");
        assert_eq!(role("frame", None), "AXWindow");
    }

    #[test]
    fn finds_the_window_at_a_point_app_windows_first() {
        let window = |layer, x| WindowInfo {
            pid: 1,
            app: "a".into(),
            title: None,
            bounds: PtRect::new(x, 0.0, 10.0, 10.0),
            layer,
        };
        let windows = [window(ABOVE, 0.0), window(0, 0.0), window(0, 20.0)];
        assert_eq!(window_at(&windows, 5.0, 5.0).unwrap().layer, 0);
        assert!(window_at(&windows, 15.0, 5.0).is_none());
    }

    /// Prints the windows, and what's under the middle of the one in front: run with `--ignored
    /// --nocapture`, on a desktop.
    #[test]
    #[ignore = "needs a desktop"]
    fn prints_the_place_in_front() {
        for w in windows(None).iter().take(8) {
            println!("{} {:?} {:?} layer {}", w.app, w.title, w.bounds, w.layer);
        }
        let front = crate::system::frontmost_app().unwrap();
        println!("front: {:?}", place(front, true));
    }
}
