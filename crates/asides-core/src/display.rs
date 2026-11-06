//! Windows' screens in the coordinates the app and the overlay share (`helper::SpecScreen`), which
//! are macOS's: global points from the top left of the main screen (CoreGraphics', y down) and
//! frames from its bottom left (AppKit's, y up).
//!
//! On Linux, the X server's monitors are read the same way (`monitors`, through RandR), Xwayland's on
//! a Wayland desktop: its pixels are the desktop's points there.
//!
//! Windows lays its monitors out in physical pixels, each at a scale of its own (per-monitor DPI).
//! Points can't be global the way macOS's are: dividing every monitor's position by its own scale
//! would overlap a 100 % monitor left of a 150 % one. So a monitor keeps its corner where Windows
//! puts it (in pixels) and is its size in points from there: a point on it is its corner plus how
//! far in, in points. Monitors never overlap that way (a scale is at least 1), and each point maps
//! back to one pixel.

use crate::inspect::PtRect;

/// A monitor as Windows lays it out: its rectangle in physical pixels (virtual-screen coordinates)
/// and how many pixels a point is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub left: i32,
    pub top: i32,
    pub width: i32,
    pub height: i32,
    pub scale: f64,
    /// The primary monitor (at 0, 0): the main screen.
    pub primary: bool,
}

impl Monitor {
    /// Its size in points.
    pub fn size(&self) -> (f64, f64) {
        (self.width as f64 / self.scale, self.height as f64 / self.scale)
    }

    /// Its top-left corner in global points (CoreGraphics' way, y down).
    pub fn origin(&self) -> (f64, f64) {
        (self.left as f64, self.top as f64)
    }

    fn contains_pixel(&self, (x, y): (f64, f64)) -> bool {
        x >= self.left as f64
            && x < (self.left + self.width) as f64
            && y >= self.top as f64
            && y < (self.top + self.height) as f64
    }

    fn contains_point(&self, (x, y): (f64, f64)) -> bool {
        let (w, h) = self.size();
        let (ox, oy) = self.origin();
        x >= ox && x < ox + w && y >= oy && y < oy + h
    }
}

/// The main screen's height in points: AppKit's frames count up from its bottom.
fn main_height(monitors: &[Monitor]) -> f64 {
    monitors.iter().find(|m| m.primary).or(monitors.first()).map_or(0.0, |m| m.size().1)
}

/// The monitor's frame in AppKit's coordinates (from the main screen's bottom left, y up).
pub fn frame(monitors: &[Monitor], monitor: &Monitor) -> PtRect {
    let (w, h) = monitor.size();
    let (x, y) = monitor.origin();
    PtRect::new(x, main_height(monitors) - y - h, w, h)
}

/// A point in physical pixels (virtual-screen coordinates) in global points (y down); a pixel off
/// every monitor is taken as on the nearest.
pub fn to_points(monitors: &[Monitor], (x, y): (f64, f64)) -> (f64, f64) {
    match nearest(monitors, |m| m.contains_pixel((x, y)), (x, y)) {
        Some(m) => (m.left as f64 + (x - m.left as f64) / m.scale, m.top as f64 + (y - m.top as f64) / m.scale),
        None => (x, y),
    }
}

/// A point in global points (y down) in physical pixels.
pub fn to_pixels(monitors: &[Monitor], (x, y): (f64, f64)) -> (f64, f64) {
    match nearest(monitors, |m| m.contains_point((x, y)), (x, y)) {
        Some(m) => (m.left as f64 + (x - m.left as f64) * m.scale, m.top as f64 + (y - m.top as f64) * m.scale),
        None => (x, y),
    }
}

/// A rectangle in physical pixels in global points, by the monitor its top-left corner is on.
pub fn rect_to_points(monitors: &[Monitor], (x, y, w, h): (f64, f64, f64, f64)) -> PtRect {
    let scale = nearest(monitors, |m| m.contains_pixel((x, y)), (x, y)).map_or(1.0, |m| m.scale);
    let (px, py) = to_points(monitors, (x, y));
    PtRect::new(px, py, w / scale, h / scale)
}

/// AppKit's y (up from the main screen's bottom) and global points' (down from its top) for a
/// rectangle `height` tall.
pub fn flip_y(monitors: &[Monitor], y: f64, height: f64) -> f64 {
    main_height(monitors) - y - height
}

/// A frame in AppKit's coordinates as a rectangle in physical pixels (`x, y, width, height`), by
/// the monitor its top-left corner is on.
pub fn frame_to_pixels(monitors: &[Monitor], frame: PtRect) -> (i32, i32, i32, i32) {
    let top = flip_y(monitors, frame.y, frame.height);
    let scale = nearest(monitors, |m| m.contains_point((frame.x, top)), (frame.x, top)).map_or(1.0, |m| m.scale);
    let (x, y) = to_pixels(monitors, (frame.x, top));
    (x.round() as i32, y.round() as i32, (frame.width * scale).round() as i32, (frame.height * scale).round() as i32)
}

/// A rectangle in physical pixels as a frame in AppKit's coordinates.
pub fn pixels_to_frame(monitors: &[Monitor], (x, y, w, h): (f64, f64, f64, f64)) -> PtRect {
    let r = rect_to_points(monitors, (x, y, w, h));
    PtRect::new(r.x, flip_y(monitors, r.y, r.height), r.width, r.height)
}

/// A monitor as the system lists it (Windows' `EnumDisplayMonitors`, whose order is GPUI's display
/// ids; RandR's on Linux).
#[cfg(any(windows, target_os = "linux"))]
#[derive(Debug, Clone)]
pub struct Listed {
    pub monitor: Monitor,
    /// `\\.\DISPLAY1`
    pub device: String,
    /// What the taskbar leaves of it, in physical pixels: left, top, right, bottom.
    pub work: (i32, i32, i32, i32),
}

/// The monitors, in `EnumDisplayMonitors`' order. Physical pixels only in a process that's aware
/// of each monitor's scale (the app is, through tao; the overlay, through GPUI's manifest).
#[cfg(windows)]
pub fn monitors() -> Vec<Listed> {
    use windows::Win32::Foundation::{LPARAM, RECT};
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    use windows::core::BOOL;
    /// `MONITORINFOF_PRIMARY`.
    const PRIMARY: u32 = 1;
    unsafe extern "system" fn each(handle: HMONITOR, _: HDC, _: *mut RECT, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the vector passed below, alive during the enumeration.
        let found = unsafe { &mut *(found.0 as *mut Vec<Listed>) };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        // SAFETY: the struct's size is set; Windows fills it.
        if !unsafe { GetMonitorInfoW(handle, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) }.as_bool() {
            return true.into();
        }
        let (mut dpi_x, mut dpi_y) = (96, 96);
        // SAFETY: plain call; on failure the default (100 %) stays.
        let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
        let (r, w) = (info.monitorInfo.rcMonitor, info.monitorInfo.rcWork);
        let len = info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len());
        found.push(Listed {
            monitor: Monitor {
                left: r.left,
                top: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
                scale: dpi_x.max(96) as f64 / 96.0,
                primary: info.monitorInfo.dwFlags & PRIMARY != 0,
            },
            device: String::from_utf16_lossy(&info.szDevice[..len]),
            work: (w.left, w.top, w.right, w.bottom),
        });
        true.into()
    }
    let mut found: Vec<Listed> = Vec::new();
    // SAFETY: the callback only touches `found`, which outlives the call.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut found as *mut Vec<Listed> as isize));
    }
    found
}

/// How long [`connect_x11`] waits for the X server.
#[cfg(target_os = "linux")]
const X_WAIT: std::time::Duration = std::time::Duration::from_secs(3);

/// A new connection to the X server, if it takes one in time. Connecting waits for as long as the
/// server doesn't answer, and on a Wayland desktop it may still be starting, or hung with the
/// compositor that starts it: a GUI's main thread mustn't wait with it. (One that comes too late
/// goes with its thread.)
#[cfg(target_os = "linux")]
pub fn connect_x11() -> Option<(x11rb::rust_connection::RustConnection, usize)> {
    let (connected, connection) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("asides-x-connect".into())
        .spawn(move || {
            let _ = connected.send(x11rb::connect(None).ok());
        })
        .ok()?;
    connection.recv_timeout(X_WAIT).ok().flatten()
}

/// The monitors, on a new connection to the X server (none without one).
#[cfg(target_os = "linux")]
pub fn monitors() -> Vec<Listed> {
    match connect_x11() {
        Some((connection, screen)) => monitors_on(&connection, screen),
        None => Vec::new(),
    }
}

/// The monitors as the X server lays them out (RandR), the primary one first, as GPUI's scale says
/// (`x11_scale`); a screen without RandR is one monitor.
#[cfg(target_os = "linux")]
pub fn monitors_on(connection: &impl x11rb::connection::Connection, screen: usize) -> Vec<Listed> {
    use x11rb::protocol::randr::ConnectionExt as _;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _};
    let Some(root) = connection.setup().roots.get(screen) else { return Vec::new() };
    let scale = x11_scale(connection, screen);
    let area: Option<Vec<u32>> =
        connection.intern_atom(false, b"_NET_WORKAREA").ok().and_then(|cookie| cookie.reply().ok()).and_then(|atom| {
            let cookie = connection.get_property(false, root.root, atom.atom, AtomEnum::CARDINAL, 0, 4).ok()?;
            cookie.reply().ok()?.value32().map(Iterator::collect)
        });
    let work = |left: i32, top: i32, width: i32, height: i32| match area.as_deref() {
        // One rectangle for the whole screen: what it leaves of this monitor.
        Some(&[x, y, w, h]) if w > 0 && h > 0 => {
            let (x, y, w, h) = (x as i32, y as i32, w as i32, h as i32);
            let (l, t) = (left.max(x), top.max(y));
            let (r, b) = ((left + width).min(x + w), (top + height).min(y + h));
            if r > l && b > t { (l, t, r, b) } else { (left, top, left + width, top + height) }
        }
        _ => (left, top, left + width, top + height),
    };
    let reply = connection.randr_get_monitors(root.root, true).ok().and_then(|cookie| cookie.reply().ok());
    let mut found: Vec<Listed> = reply
        .map(|r| r.monitors)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.width > 0 && m.height > 0)
        .map(|m| {
            let (left, top, width, height) = (m.x as i32, m.y as i32, m.width as i32, m.height as i32);
            let name = connection.get_atom_name(m.name).ok().and_then(|cookie| cookie.reply().ok());
            Listed {
                monitor: Monitor { left, top, width, height, scale, primary: m.primary },
                device: name.map_or_else(String::new, |n| String::from_utf8_lossy(&n.name).into_owned()),
                work: work(left, top, width, height),
            }
        })
        .collect();
    if found.is_empty() {
        let (width, height) = (root.width_in_pixels as i32, root.height_in_pixels as i32);
        found.push(Listed {
            monitor: Monitor { left: 0, top: 0, width, height, scale, primary: true },
            device: String::new(),
            work: work(0, 0, width, height),
        });
    }
    if !found.iter().any(|l| l.monitor.primary) {
        // One must be: the first at the origin, or the first.
        let at = found.iter().position(|l| l.monitor.left == 0 && l.monitor.top == 0).unwrap_or(0);
        found[at].monitor.primary = true;
    }
    found
}

/// How many X pixels a point is, as GPUI says for its windows: `GPUI_X11_SCALE_FACTOR`, or `Xft.dpi`
/// in the X resources over 96, or 1.
#[cfg(target_os = "linux")]
fn x11_scale(connection: &impl x11rb::connection::Connection, _screen: usize) -> f64 {
    if let Some(scale) = std::env::var("GPUI_X11_SCALE_FACTOR").ok().and_then(|v| v.parse::<f64>().ok()) {
        return scale;
    }
    let database = x11rb::resource_manager::new_from_default(connection).ok();
    let dpi = database.and_then(|db| db.get_value::<f64>("Xft.dpi", "Xft.dpi").ok().flatten());
    dpi.filter(|dpi| *dpi > 0.0).map_or(1.0, |dpi| dpi / 96.0)
}

/// The monitor `inside` holds, or else the one nearest to `(x, y)` (by its corner).
fn nearest(monitors: &[Monitor], inside: impl Fn(&Monitor) -> bool, (x, y): (f64, f64)) -> Option<&Monitor> {
    monitors.iter().find(|m| inside(m)).or_else(|| {
        monitors.iter().min_by(|a, b| {
            let d = |m: &Monitor| (m.left as f64 - x).hypot(m.top as f64 - y);
            d(a).total_cmp(&d(b))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 150 % laptop (the main screen) with a 100 % monitor on its left, a little higher.
    fn desk() -> Vec<Monitor> {
        vec![
            Monitor { left: 0, top: 0, width: 2880, height: 1800, scale: 1.5, primary: true },
            Monitor { left: -1920, top: -200, width: 1920, height: 1080, scale: 1.0, primary: false },
        ]
    }

    #[test]
    fn monitors_keep_their_corners_and_never_overlap() {
        let desk = desk();
        assert_eq!(desk[0].size(), (1920.0, 1200.0));
        assert_eq!(frame(&desk, &desk[0]), PtRect::new(0.0, 0.0, 1920.0, 1200.0));
        // AppKit's y: up from the main screen's bottom.
        assert_eq!(frame(&desk, &desk[1]), PtRect::new(-1920.0, 1200.0 + 200.0 - 1080.0, 1920.0, 1080.0));
        let left = frame(&desk, &desk[1]);
        assert!(left.x + left.width <= 0.0, "the left monitor ends where the main one begins");
    }

    #[test]
    fn maps_pixels_to_points_and_back() {
        let desk = desk();
        assert_eq!(to_points(&desk, (1500.0, 900.0)), (1000.0, 600.0));
        assert_eq!(to_pixels(&desk, (1000.0, 600.0)), (1500.0, 900.0));
        assert_eq!(to_points(&desk, (-100.0, 50.0)), (-100.0, 50.0));
        assert_eq!(to_pixels(&desk, (-100.0, 50.0)), (-100.0, 50.0));
        assert_eq!(rect_to_points(&desk, (300.0, 150.0, 600.0, 300.0)), PtRect::new(200.0, 100.0, 400.0, 200.0));
        assert_eq!(flip_y(&desk, 100.0, 200.0), 900.0);
        // A window's frame (AppKit's), on the main screen and on the other.
        let pill = PtRect::new(800.0, 1200.0 - 50.0 - 40.0, 300.0, 40.0);
        assert_eq!(frame_to_pixels(&desk, pill), (1200, 75, 450, 60));
        assert_eq!(pixels_to_frame(&desk, (1200.0, 75.0, 450.0, 60.0)), pill);
        let left = PtRect::new(-1000.0, 1200.0 - (-100.0) - 300.0, 400.0, 300.0);
        assert_eq!(frame_to_pixels(&desk, left), (-1000, -100, 400, 300));
    }
}
