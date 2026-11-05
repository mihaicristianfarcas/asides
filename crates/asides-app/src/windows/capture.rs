//! Freezing the screen on Windows: each monitor's pixels as the desktop composes them, copied with
//! GDI (`BitBlt` from the screen, which reads what the compositor shows) into a bitmap at the
//! monitor's own resolution. Windows asks no permission for it. The overlay's pill marks its
//! window as left out of captures (`WDA_EXCLUDEFROMCAPTURE`), so freezing again over the live
//! screen doesn't take the pill with it.
//!
//! Monitors are numbered as `EnumDisplayMonitors` lists them, which is GPUI's display id on
//! Windows: the overlay finds each screen's monitor by it.

use asides_core::display::{self, Monitor, monitors};
use asides_core::inspect::PtRect;
use asides_core::measure::Frame;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, GetDC, ReleaseDC, SRCCOPY, SelectObject,
};

use super::system::MainThread;

/// A display as Windows lays it out, in the coordinates the overlay shares with macOS
/// (`asides_core::display`).
pub struct ScreenInfo {
    pub display: u32,
    pub name: Option<String>,
    /// AppKit's coordinates (y up from the main screen's bottom), points.
    pub frame: PtRect,
    pub scale: f64,
    /// Top-left corner in global points (y down).
    pub origin: (f64, f64),
    /// Windows has no menu bar along the top.
    pub menu_bar: f64,
}

/// The displays, numbered as GPUI numbers them.
pub fn screens(_: MainThread) -> Vec<ScreenInfo> {
    let listed = monitors();
    let all: Vec<Monitor> = listed.iter().map(|l| l.monitor).collect();
    listed
        .iter()
        .enumerate()
        .map(|(i, l)| ScreenInfo {
            display: i as u32,
            name: Some(friendly_name(&l.device)),
            frame: display::frame(&all, &l.monitor),
            scale: l.monitor.scale,
            origin: l.monitor.origin(),
            menu_bar: 0.0,
        })
        .collect()
}

/// `\\.\DISPLAY2` → `Display 2`.
fn friendly_name(device: &str) -> String {
    match device.trim_start_matches(r"\\.\").strip_prefix("DISPLAY") {
        Some(n) => format!("Display {n}"),
        None => device.to_string(),
    }
}

/// A display, frozen: its pixels.
pub struct Capture {
    pub display: u32,
    pub frame: Frame,
}

/// Windows asks no permission to see the screen.
pub fn permitted() -> bool {
    true
}

pub fn request() -> bool {
    true
}

/// Captures `displays` (number, width and height in pixels) on a thread of its own; `done` runs
/// once there. The overlay's windows that must not be in it keep themselves out (`without` is
/// macOS's way).
pub fn capture(
    displays: Vec<(u32, usize, usize)>,
    _without: Option<i32>,
    done: impl Fn(Result<Vec<Capture>, String>) + Send + Sync + 'static,
) {
    let spawned = std::thread::Builder::new().name("asides-capture".into()).spawn(move || {
        if displays.is_empty() {
            return done(Err("No screen to capture.".into()));
        }
        let listed = monitors();
        let mut captures = Vec::new();
        let mut error = None;
        for (display, _, _) in displays {
            match listed.get(display as usize).ok_or("That screen is gone.".to_string()).and_then(|l| grab(&l.monitor))
            {
                Ok(frame) => captures.push(Capture { display, frame }),
                Err(why) => error = Some(why),
            }
        }
        done(if captures.is_empty() { Err(error.unwrap_or_default()) } else { Ok(captures) })
    });
    if let Err(err) = spawned {
        eprintln!("asides: could not start capturing: {err}");
    }
}

/// A monitor's pixels, as RGBA.
fn grab(monitor: &Monitor) -> Result<Frame, String> {
    let (w, h) = (monitor.width, monitor.height);
    if w <= 0 || h <= 0 {
        return Err("The screen has no size.".into());
    }
    // SAFETY: every GDI object made here is deleted before returning; the bitmap's bits are read
    // while it lives, `w * h * 4` bytes, top-down (a negative height).
    unsafe {
        let screen = GetDC(None);
        let memory = CreateCompatibleDC(Some(screen));
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w,
                biHeight: -h,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bitmap = CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0);
        let result = match bitmap {
            Ok(bitmap) if !bits.is_null() => {
                let previous = SelectObject(memory, bitmap.into());
                let copied =
                    BitBlt(memory, 0, 0, w, h, Some(screen), monitor.left, monitor.top, SRCCOPY | CAPTUREBLT).is_ok();
                let frame = copied
                    .then(|| {
                        let bgra = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                        let rgba: Vec<u8> =
                            bgra.as_chunks::<4>().0.iter().flat_map(|&[b, g, r, _]| [r, g, b, 255]).collect();
                        Frame::new(w as u32, h as u32, rgba)
                    })
                    .flatten();
                SelectObject(memory, previous);
                let _ = DeleteObject(bitmap.into());
                frame.ok_or_else(|| "Could not capture the screen.".to_string())
            }
            _ => Err("Could not make a bitmap for the screenshot.".to_string()),
        };
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_displays() {
        assert_eq!(friendly_name(r"\\.\DISPLAY2"), "Display 2");
    }

    /// Takes each screen as asides would: `cargo test -p asides-app captures_every_screen --
    /// --ignored --nocapture` (on Windows, in the desktop's session).
    #[test]
    #[ignore = "needs a desktop"]
    fn captures_every_screen() {
        let screens = screens(MainThread);
        assert!(!screens.is_empty());
        let listed = monitors();
        for s in &screens {
            let m = &listed[s.display as usize].monitor;
            let frame = grab(m).unwrap();
            println!("{:?} {}x{} at {:?}, scale {}", s.name, frame.width, frame.height, s.origin, s.scale);
            assert_eq!((frame.width as i32, frame.height as i32), (m.width, m.height));
        }
    }
}
