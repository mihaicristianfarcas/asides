//! Freezing the screen for Asides: one screenshot per display, taken with ScreenCaptureKit, in sRGB
//! (so colors read like CSS colors). Needs the Screen Recording permission.

use std::ptr::NonNull;
use std::sync::{Arc, Mutex};

use asides_core::measure::Frame;
use block2::RcBlock;
use objc2::AnyThread;
use objc2::runtime::AnyClass;
use objc2_app_kit::{NSBitmapImageRep, NSColorRenderingIntent, NSColorSpace};
use objc2_core_foundation::CFRetained;
use objc2_core_graphics::{
    CGDataProvider, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGPreflightScreenCaptureAccess,
    CGRequestScreenCaptureAccess, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCScreenshotConfiguration, SCScreenshotDynamicRange, SCScreenshotManager, SCScreenshotOutput,
    SCShareableContent, SCStreamConfiguration, SCWindow,
};

/// A display as AppKit and CoreGraphics know it.
pub struct ScreenInfo {
    pub display: u32,
    pub name: Option<String>,
    /// In AppKit's coordinates (origin at the bottom-left of the main screen), points.
    pub frame: objc2_foundation::NSRect,
    pub scale: f64,
    /// Top-left corner in CoreGraphics' global coordinates (y down), points.
    pub origin: (f64, f64),
}

/// The displays, in AppKit's order (the main one first).
pub fn screens(mtm: objc2::MainThreadMarker) -> Vec<ScreenInfo> {
    let key = objc2_foundation::NSString::from_str("NSScreenNumber");
    objc2_app_kit::NSScreen::screens(mtm)
        .iter()
        .filter_map(|screen| {
            let number = screen.deviceDescription().objectForKey(&key)?;
            let display = number.downcast_ref::<objc2_foundation::NSNumber>()?.unsignedIntValue();
            let bounds = objc2_core_graphics::CGDisplayBounds(display);
            Some(ScreenInfo {
                display,
                name: Some(screen.localizedName().to_string()),
                frame: screen.frame(),
                scale: screen.backingScaleFactor(),
                origin: (bounds.origin.x, bounds.origin.y),
            })
        })
        .collect()
}

/// A display, frozen: its pixels.
pub struct Capture {
    pub display: u32,
    pub frame: Frame,
}

/// Whether Asides may capture the screen (without asking).
pub fn permitted() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// Asks for the Screen Recording permission: the first time, macOS shows its prompt; after that,
/// the user has to allow it in System Settings.
pub fn request() -> bool {
    CGRequestScreenCaptureAccess()
}

pub fn open_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture")
        .spawn();
}

type Done = Arc<dyn Fn(Result<Vec<Capture>, String>) + Send + Sync>;
/// Screens still to come, the captures so far, and the last error.
type Results = Arc<Mutex<(usize, Vec<Capture>, Option<String>)>>;

/// Captures `displays` (ID, width and height in pixels), leaving out the windows of process
/// `without` (the live toolbar's); `done` runs once, on a background queue.
pub fn capture(
    displays: Vec<(u32, usize, usize)>,
    without: Option<i32>,
    done: impl Fn(Result<Vec<Capture>, String>) + Send + Sync + 'static,
) {
    let done: Done = Arc::new(done);
    let block = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let Some(content) = (unsafe { content.as_ref() }) else {
            return done(Err(describe(error, "Could not list the screens to capture")));
        };
        let available = unsafe { content.displays() };
        let wanted: Vec<_> = displays
            .iter()
            .filter_map(|&(id, w, h)| available.iter().find(|d| unsafe { d.displayID() } == id).map(|d| (id, w, h, d)))
            .collect();
        if wanted.is_empty() {
            return done(Err("No screen to capture.".into()));
        }
        let results: Results = Arc::new(Mutex::new((wanted.len(), Vec::new(), None)));
        let left_out: Vec<objc2::rc::Retained<SCWindow>> = match without {
            Some(pid) => unsafe { content.windows() }
                .iter()
                .filter(|w| unsafe { w.owningApplication() }.is_some_and(|a| unsafe { a.processID() } == pid))
                .collect(),
            None => Vec::new(),
        };
        let left_out = NSArray::from_retained_slice(&left_out);
        for (id, width, height, display) in wanted {
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &left_out)
            };
            let (results, done) = (results.clone(), done.clone());
            let captured = Arc::new(move |image: Option<CFRetained<CGImage>>, error: *mut NSError| {
                let mut state = results.lock().unwrap();
                state.0 -= 1;
                match image {
                    Some(image) => match to_frame(&image) {
                        Some(frame) => state.1.push(Capture { display: id, frame }),
                        None => state.2 = Some("The screenshot had an unexpected pixel format.".into()),
                    },
                    None => state.2 = Some(describe(error, "Could not capture the screen")),
                }
                if state.0 == 0 {
                    let (_, captures, error) = std::mem::take(&mut *state);
                    drop(state);
                    done(if captures.is_empty() { Err(error.unwrap_or_default()) } else { Ok(captures) });
                }
            });
            // macOS 26's screenshots have windows' shadows, as the screen shows them; the older
            // ones leave them out, so a frozen screen was lighter under every window than the
            // live one. Its images are in the display's color space: in sRGB, colors read like CSS.
            if AnyClass::get(c"SCScreenshotConfiguration").is_some() {
                let config = unsafe { SCScreenshotConfiguration::new() };
                unsafe {
                    config.setWidth(width as isize);
                    config.setHeight(height as isize);
                    config.setShowsCursor(false);
                    config.setIgnoreShadows(false);
                    config.setDynamicRange(SCScreenshotDynamicRange::SDR);
                }
                let handler = RcBlock::new(move |output: *mut SCScreenshotOutput, error: *mut NSError| {
                    let image = unsafe { output.as_ref() }.and_then(|o| unsafe { o.sdrImage() }).and_then(|i| srgb(&i));
                    captured(image, error);
                });
                unsafe {
                    SCScreenshotManager::captureScreenshotWithFilter_configuration_completionHandler(
                        &filter,
                        &config,
                        Some(&handler),
                    )
                };
                continue;
            }
            let config = unsafe { SCStreamConfiguration::new() };
            unsafe {
                config.setWidth(width);
                config.setHeight(height);
                config.setShowsCursor(false);
                config.setColorSpaceName(kCGColorSpaceSRGB);
            }
            let handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
                captured(NonNull::new(image).map(|image| unsafe { CFRetained::retain(image) }), error);
            });
            unsafe {
                SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                    &filter,
                    &config,
                    Some(&handler),
                )
            };
        }
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
}

/// `image` in sRGB.
fn srgb(image: &CGImage) -> Option<CFRetained<CGImage>> {
    let rep = NSBitmapImageRep::initWithCGImage(NSBitmapImageRep::alloc(), image);
    let converted = rep.bitmapImageRepByConvertingToColorSpace_renderingIntent(
        &NSColorSpace::sRGBColorSpace(),
        NSColorRenderingIntent::Default,
    )?;
    converted.CGImage().map(Into::into)
}

fn describe(error: *mut NSError, what: &str) -> String {
    match unsafe { error.as_ref() } {
        Some(error) => format!("{what}: {}", error.localizedDescription()),
        None => format!("{what}."),
    }
}

/// The image's pixels as straight RGBA (8-bit, 4 bytes per pixel), whatever their byte order.
pub fn to_frame(image: &CGImage) -> Option<Frame> {
    let image = Some(image);
    let (width, height) = (CGImage::width(image), CGImage::height(image));
    if CGImage::bits_per_pixel(image) != 32 || CGImage::bits_per_component(image) != 8 {
        return None;
    }
    let row = CGImage::bytes_per_row(image);
    let data = CGDataProvider::data(CGImage::data_provider(image).as_deref())?.to_vec();
    let little = CGImage::byte_order_info(image) == CGImageByteOrderInfo::Order32Little;
    let alpha = CGImage::alpha_info(image);
    let alpha_first = [CGImageAlphaInfo::PremultipliedFirst, CGImageAlphaInfo::First, CGImageAlphaInfo::NoneSkipFirst]
        .contains(&alpha);
    // Where red, green and blue sit in each pixel's 4 bytes.
    let (r, g, b) = match (little, alpha_first) {
        (true, true) => (2, 1, 0),
        (true, false) => (3, 2, 1),
        (false, true) => (1, 2, 3),
        (false, false) => (0, 1, 2),
    };
    if data.len() < row * height.saturating_sub(1) + width * 4 {
        return None;
    }
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for px in data[y * row..y * row + width * 4].as_chunks::<4>().0 {
            rgba.extend_from_slice(&[px[r], px[g], px[b], 255]);
        }
    }
    Frame::new(width as u32, height as u32, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Captures the main display as a freeze does (in sRGB, 2 pixels a point) into
    /// `$ASIDES_CAPTURE_DUMP/freeze-<width>x<height>.rgba`: the pixels Measure works on, which a
    /// screenshot in the display's colors isn't. `--ignored`; needs the Screen Recording permission.
    #[test]
    #[ignore]
    fn dumps_a_freeze() {
        let dir = std::path::PathBuf::from(std::env::var("ASIDES_CAPTURE_DUMP").expect("ASIDES_CAPTURE_DUMP"));
        let display = objc2_core_graphics::CGMainDisplayID();
        let bounds = objc2_core_graphics::CGDisplayBounds(display);
        let (w, h) = ((bounds.size.width * 2.0) as usize, (bounds.size.height * 2.0) as usize);
        let (tx, rx) = std::sync::mpsc::channel();
        capture(vec![(display, w, h)], None, move |r| {
            let _ = tx.send(r);
        });
        let captures = rx.recv().unwrap().unwrap();
        let f = &captures[0].frame;
        std::fs::write(dir.join(format!("freeze-{}x{}.rgba", f.width, f.height)), &f.rgba).unwrap();
    }
}
