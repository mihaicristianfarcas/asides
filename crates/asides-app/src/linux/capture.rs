//! Freezing the screen on Linux. On X11 each monitor's pixels are read from the root window. On a
//! Wayland desktop only the compositor can see the screen: its Screenshot portal, or on KDE KWin
//! itself (`kwin_picture`), gives one picture of all of them, which is cut up by monitor (as
//! Xwayland's RandR lays them out, in points). The overlay's windows, moved off the screen while
//! it's taken, stay out of it.
//!
//! Monitors are numbered as `asides_core::display::monitors` lists them; the overlay's windows use
//! the same list.

use std::collections::HashMap;
use std::io::Read;
use std::os::fd::AsFd;
use std::time::Duration;

use asides_core::display::{self, Monitor, monitors};
use asides_core::inspect::PtRect;
use asides_core::measure::Frame;
use asides_core::portal::{self, Answer};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConfigureWindowAux, ConnectionExt as _, ImageFormat, Window};
use zbus::zvariant::{Fd, OwnedValue, Value};

use super::system::MainThread;

/// How long a Wayland desktop may take to answer: the first time, it may ask the user.
const PORTAL_WAIT: Duration = Duration::from_secs(120);
/// Nothing says when a compositor has shown an X11 window's move: two frames at 60 Hz, and a margin.
const SHOWN: Duration = Duration::from_millis(50);
/// Off every screen.
const AWAY: i32 = -32000;

/// A display as the desktop lays it out, in the coordinates the overlay shares with macOS
/// (`asides_core::display`).
pub struct ScreenInfo {
    pub display: u32,
    pub name: Option<String>,
    /// AppKit's coordinates (y up from the main screen's bottom), points.
    pub frame: PtRect,
    pub scale: f64,
    /// Top-left corner in global points (y down).
    pub origin: (f64, f64),
    /// The overlay covers the panels too: it keeps nothing clear along the top.
    pub menu_bar: f64,
}

/// The displays, numbered as the overlay numbers them.
pub fn screens(_: MainThread) -> Vec<ScreenInfo> {
    let listed = monitors();
    let all: Vec<Monitor> = listed.iter().map(|l| l.monitor).collect();
    listed
        .iter()
        .enumerate()
        .map(|(i, l)| ScreenInfo {
            display: i as u32,
            name: Some(l.device.clone()),
            frame: display::frame(&all, &l.monitor),
            scale: l.monitor.scale,
            origin: l.monitor.origin(),
            menu_bar: 0.0,
        })
        .collect()
}

/// A display, frozen: its pixels.
pub struct Capture {
    pub display: u32,
    pub frame: Frame,
}

/// Whether the desktop lets asides see the screen. Only GNOME on Wayland must be asked first
/// (`portal::asks_for_screenshots`); without its store, a capture will say what's wrong.
pub fn permitted() -> bool {
    !portal::asks_for_screenshots() || !matches!(portal::screenshot_answer(), Ok(Answer::No | Answer::NotAsked))
}

/// Asks the desktop, and waits for the user's answer: a screenshot, thrown away. For setup's button,
/// with asides' window in front: GNOME shows its question only for the focused app.
pub fn request() -> bool {
    if !portal::asks_for_screenshots() {
        return true;
    }
    if portal::screenshot_answer() == Ok(Answer::No) {
        portal::forget_screenshot_answer();
    }
    match portal_picture() {
        Ok(_) => true,
        Err(why) => {
            eprintln!("asides: asking for screenshots: {why}");
            false
        }
    }
}

/// Whether the desktop is Wayland's: X11 clients see only each other's windows there.
pub fn on_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland")
}

/// Captures `displays` (number, width and height in pixels) on a thread of its own; `done` runs
/// once there. `without`: asides is open over the live screen, and its windows mustn't be in it.
pub fn capture(
    displays: Vec<(u32, usize, usize)>,
    without: Option<i32>,
    done: impl Fn(Result<Vec<Capture>, String>) + Send + Sync + 'static,
) {
    let spawned = std::thread::Builder::new().name("asides-capture".into()).spawn(move || {
        if displays.is_empty() {
            return done(Err("No screen to capture.".into()));
        }
        let chosen: Vec<u32> = displays.iter().map(|d| d.0).collect();
        let moved = without.map(|_| OutOfTheWay::now());
        let result = if on_wayland() { from_compositor(&chosen) } else { from_x11(&chosen) };
        drop(moved);
        done(result);
    });
    if let Err(err) = spawned {
        eprintln!("asides: could not start capturing: {err}");
    }
}

/// The overlay's windows (`_ASIDES_OVERLAY`) moved off the screens, until it's dropped.
struct OutOfTheWay {
    connection: Option<x11rb::rust_connection::RustConnection>,
    moved: Vec<Moved>,
}

impl OutOfTheWay {
    fn now() -> OutOfTheWay {
        let Ok((connection, screen)) = x11rb::connect(None) else {
            return OutOfTheWay { connection: None, moved: Vec::new() };
        };
        let moved = move_overlay(&connection, screen).unwrap_or_default();
        if !moved.is_empty() {
            // Answered once the server has done all that came before.
            let _ = connection.get_input_focus().map(|cookie| cookie.reply());
            if on_wayland() {
                std::thread::sleep(SHOWN);
            }
        }
        OutOfTheWay { connection: Some(connection), moved }
    }
}

impl Drop for OutOfTheWay {
    fn drop(&mut self) {
        let Some(connection) = &self.connection else { return };
        for &(window, x, y) in &self.moved {
            let _ = connection.configure_window(window, &ConfigureWindowAux::new().x(x).y(y));
        }
        // Answered once the server has done them: closed right after a flush, the moves were lost.
        let _ = connection.get_input_focus().map(|cookie| cookie.reply());
    }
}

/// A window, and where it was.
type Moved = (Window, i32, i32);

/// Moves the overlay's windows that are on a screen off it; returns where they were.
fn move_overlay(connection: &impl Connection, screen: usize) -> Result<Vec<Moved>, Box<dyn std::error::Error>> {
    let root = connection.setup().roots.get(screen).ok_or("no such screen")?.root;
    let mark = connection.intern_atom(false, b"_ASIDES_OVERLAY")?.reply()?.atom;
    let mut moved = Vec::new();
    for window in connection.query_tree(root)?.reply()?.children {
        let marked = connection.get_property(false, window, mark, AtomEnum::ANY, 0, 1)?.reply();
        if !marked.is_ok_and(|p| p.value_len > 0) {
            continue;
        }
        let Ok(place) = connection.get_geometry(window)?.reply() else { continue };
        if i32::from(place.x) <= AWAY / 2 {
            continue;
        }
        connection.configure_window(window, &ConfigureWindowAux::new().x(AWAY).y(AWAY))?;
        moved.push((window, i32::from(place.x), i32::from(place.y)));
    }
    Ok(moved)
}

/// Each display's pixels, from the X server.
fn from_x11(displays: &[u32]) -> Result<Vec<Capture>, String> {
    let (connection, screen) = x11rb::connect(None).map_err(|err| format!("Could not reach the X server: {err}"))?;
    let root = connection.setup().roots.get(screen).ok_or("The X server has no such screen.")?.root;
    let listed = display::monitors_on(&connection, screen);
    let mut captures = Vec::new();
    let mut error = None;
    for &display in displays {
        let grabbed = listed
            .get(display as usize)
            .ok_or("That screen is gone.".to_string())
            .and_then(|l| grab(&connection, root, &l.monitor));
        match grabbed {
            Ok(frame) => captures.push(Capture { display, frame }),
            Err(why) => error = Some(why),
        }
    }
    if captures.is_empty() { Err(error.unwrap_or_default()) } else { Ok(captures) }
}

/// A monitor's pixels, as RGBA.
fn grab(connection: &impl Connection, root: Window, monitor: &Monitor) -> Result<Frame, String> {
    let (w, h) = (monitor.width as u16, monitor.height as u16);
    let image = connection
        .get_image(ImageFormat::Z_PIXMAP, root, monitor.left as i16, monitor.top as i16, w, h, !0)
        .map_err(|err| format!("Could not capture the screen: {err}"))?
        .reply()
        .map_err(|err| format!("Could not capture the screen: {err}"))?;
    if image.data.len() < w as usize * h as usize * 4 {
        return Err("The X server's screen isn't 32 bits a pixel.".into());
    }
    let rgba = image.data.as_chunks::<4>().0.iter().flat_map(|&[b, g, r, _]| [r, g, b, 255]).collect();
    Frame::new(w as u32, h as u32, rgba).ok_or_else(|| "Could not capture the screen.".to_string())
}

/// Each display's pixels, from one picture of the desktop.
fn from_compositor(displays: &[u32]) -> Result<Vec<Capture>, String> {
    let listed = monitors();
    if listed.is_empty() {
        return Err("Could not reach Xwayland, which asides' windows need.".into());
    }
    let all: Vec<Monitor> = listed.iter().map(|l| l.monitor).collect();
    let on_kde = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.to_ascii_uppercase().contains("KDE"));
    let picture = match on_kde {
        true => kwin_picture().or_else(|why| {
            eprintln!("asides: KWin's screenshot: {why}; asking its portal");
            portal_picture()
        }),
        false => portal_picture(),
    }?;
    let captures: Vec<Capture> = displays
        .iter()
        .filter_map(|&display| {
            let monitor = all.get(display as usize)?;
            let frame = cut(&picture, &all, monitor)?;
            Some(Capture { display, frame })
        })
        .collect();
    if captures.is_empty() { Err("Could not capture the screen.".into()) } else { Ok(captures) }
}

/// A picture of the desktop, from its Screenshot portal.
fn portal_picture() -> Result<Picture, String> {
    let results = portal::request(
        "org.freedesktop.portal.Screenshot",
        "Screenshot",
        |token| {
            let mut options: HashMap<&str, Value> = HashMap::new();
            options.insert("handle_token", Value::from(token.to_string()));
            options.insert("interactive", Value::from(false));
            options.insert("modal", Value::from(false));
            ("", options)
        },
        PORTAL_WAIT,
    )
    .map_err(|err| match err.as_str() {
        "declined" => "The desktop declined asides' screenshot: allow it in its settings.".to_string(),
        _ => format!("Could not capture the screen: {err}"),
    })?;
    let uri = portal::text(&results, "uri").ok_or("The desktop gave no screenshot.")?;
    let path = url::Url::parse(&uri).ok().and_then(|u| u.to_file_path().ok()).ok_or("The screenshot isn't a file.")?;
    let bytes = std::fs::read(&path).map_err(|err| format!("Could not read the screenshot: {err}"));
    // The picture is asides' to dispose of: it goes where the user's own screenshots go.
    let _ = std::fs::remove_file(&path);
    decode(&bytes?).ok_or_else(|| "Could not read the screenshot.".to_string())
}

/// A picture of the desktop, from KWin. Its portal isn't always allowed to ask it (Plasma 5.27's
/// says it's allowed another interface), and KWin allows an app whose desktop entry names the
/// interface, as Spectacle's does and asides' (`system::desktop_entry`). It answers with the
/// picture's shape, and writes the pixels down a pipe.
fn kwin_picture() -> Result<Picture, String> {
    let connection = portal::connection()?;
    let (mut pixels, writer) = std::io::pipe().map_err(|err| err.to_string())?;
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("native-resolution", Value::from(true));
    options.insert("include-cursor", Value::from(false));
    let answer = connection
        .call_method(
            Some("org.kde.KWin"),
            "/org/kde/KWin/ScreenShot2",
            Some("org.kde.KWin.ScreenShot2"),
            "CaptureWorkspace",
            &(options, Fd::from(writer.as_fd())),
        )
        .map_err(|err| err.to_string())?;
    drop(writer);
    let shape: HashMap<String, OwnedValue> = answer.body().deserialize().map_err(|err| err.to_string())?;
    let number = |key: &str| shape.get(key).and_then(|v| u32::try_from(v).ok());
    let (width, height, stride, format) = (number("width"), number("height"), number("stride"), number("format"));
    let (Some(width), Some(height), Some(stride), Some(format)) = (width, height, stride, format) else {
        return Err("KWin didn't say what the picture is".into());
    };
    let mut bytes = Vec::with_capacity(stride as usize * height as usize);
    pixels.read_to_end(&mut bytes).map_err(|err| err.to_string())?;
    raw_picture(&bytes, width as usize, height as usize, stride as usize, format)
}

/// A picture from a `QImage`'s rows: 32 bits a pixel, its colors in the order `format` says.
fn raw_picture(bytes: &[u8], width: usize, height: usize, stride: usize, format: u32) -> Result<Picture, String> {
    // `QImage::Format`: RGB32, ARGB32 and its premultiplied are BGRA in memory here; RGBX8888,
    // RGBA8888 and its premultiplied are as they say. Nothing shows through a screen.
    let blue_first = match format {
        4..=6 => true,
        16..=18 => false,
        other => return Err(format!("KWin's picture is in format {other}")),
    };
    if stride < width * 4 || bytes.len() < stride * height {
        return Err("KWin's picture is short".into());
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for row in bytes.chunks_exact(stride).take(height) {
        for pixel in row[..width * 4].as_chunks::<4>().0 {
            let [a, g, b, _] = *pixel;
            rgba.extend_from_slice(&if blue_first { [b, g, a, 255] } else { [a, g, b, 255] });
        }
    }
    Ok(Picture { width, height, rgba })
}

/// An image's pixels, as RGBA.
struct Picture {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

/// A PNG's pixels.
fn decode(bytes: &[u8]) -> Option<Picture> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let pixels = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => pixels.as_chunks::<4>().0.iter().flat_map(|&[r, g, b, _]| [r, g, b, 255]).collect(),
        png::ColorType::Rgb => pixels.as_chunks::<3>().0.iter().flat_map(|&[r, g, b]| [r, g, b, 255]).collect(),
        _ => return None,
    };
    Some(Picture { width: info.width as usize, height: info.height as usize, rgba })
}

/// The monitor's part of a picture of all of them. The picture is as many pixels a point as the
/// finest monitor has (the desktop's own scale), which X11 clients don't see.
fn cut(picture: &Picture, all: &[Monitor], monitor: &Monitor) -> Option<Frame> {
    let left = all.iter().map(|m| m.left).min()?;
    let top = all.iter().map(|m| m.top).min()?;
    let right = all.iter().map(|m| m.left + m.width).max()?;
    let ratio = picture.width as f64 / (right - left) as f64;
    let at = |v: i32| (v as f64 * ratio).round() as usize;
    let (x, y) = (at(monitor.left - left), at(monitor.top - top));
    let (w, h) = (at(monitor.width), at(monitor.height));
    if x + w > picture.width || y + h > picture.height {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for row in y..y + h {
        let start = (row * picture.width + x) * 4;
        rgba.extend_from_slice(&picture.rgba[start..start + w * 4]);
    }
    Frame::new(w as u32, h as u32, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn monitor(left: i32, top: i32, width: i32, height: i32) -> Monitor {
        Monitor { left, top, width, height, scale: 1.0, primary: left == 0 }
    }

    #[test]
    fn reads_kwins_rows_whatever_their_order_and_stride() {
        // Two pixels a row, four bytes of padding: red then blue, each format's way.
        let bgra = [0, 0, 255, 255, 255, 0, 0, 255, 9, 9, 9, 9];
        let picture = raw_picture(&bgra, 2, 1, 12, 6).unwrap();
        assert_eq!(picture.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        let rgba = [255, 0, 0, 7, 0, 0, 255, 7, 9, 9, 9, 9];
        let picture = raw_picture(&rgba, 2, 1, 12, 17).unwrap();
        assert_eq!(picture.rgba, [255, 0, 0, 255, 0, 0, 255, 255]);
        assert!(raw_picture(&bgra, 2, 1, 12, 3).is_err());
        assert!(raw_picture(&bgra[..8], 2, 1, 12, 6).is_err());
    }

    #[test]
    fn cuts_a_picture_of_the_whole_desktop_by_monitor() {
        // Two monitors side by side, the second lower, at 2 pixels a point in the picture.
        let all = [monitor(0, 0, 2, 2), monitor(2, 1, 1, 1)];
        let (width, height) = (6usize, 4usize);
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                rgba.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        let picture = Picture { width, height, rgba };
        let frame = cut(&picture, &all, &all[1]).unwrap();
        assert_eq!((frame.width, frame.height), (2, 2));
        assert_eq!(&frame.rgba[..4], &[4, 2, 0, 255]);
    }

    /// Takes each screen as asides would: `cargo test -p asides-app captures_every_screen --
    /// --ignored --nocapture`, on a desktop.
    #[test]
    #[ignore = "needs a desktop"]
    fn captures_every_screen() {
        let screens = screens(MainThread);
        assert!(!screens.is_empty());
        let chosen: Vec<u32> = screens.iter().map(|s| s.display).collect();
        let captures = if on_wayland() { from_compositor(&chosen) } else { from_x11(&chosen) }.unwrap();
        for c in &captures {
            println!("display {}: {}x{}", c.display, c.frame.width, c.frame.height);
        }
    }
}
