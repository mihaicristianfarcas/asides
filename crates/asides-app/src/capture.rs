//! Freezing the screen for asides: one screenshot per display, taken with ScreenCaptureKit, in sRGB
//! (so colors read like CSS colors). Needs the Screen Recording permission. And watching what an
//! app shows as it changes, for a clip (`watch`).

use std::sync::{Arc, Mutex};

use asides_core::inspect::PtRect;
use asides_core::measure::Frame;
use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, define_class, msg_send};
use objc2_app_kit::{NSBitmapImageRep, NSColorRenderingIntent, NSColorSpace};
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    CGDataProvider, CGDisplayBounds, CGImage, CGImageAlphaInfo, CGImageByteOrderInfo, CGPreflightScreenCaptureAccess,
    CGRequestScreenCaptureAccess, kCGColorSpaceSRGB,
};
use objc2_core_media::{CMSampleBuffer, CMTime};
use objc2_core_video::{
    CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetPixelFormatType,
    CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress,
    kCVPixelFormatType_32BGRA,
};
use objc2_foundation::{NSArray, NSError};
use objc2_screen_capture_kit::{
    SCContentFilter, SCRunningApplication, SCScreenshotConfiguration, SCScreenshotDynamicRange, SCScreenshotManager,
    SCScreenshotOutput, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamOutput, SCStreamOutputType,
    SCWindow,
};

/// A display as AppKit and CoreGraphics know it.
pub struct ScreenInfo {
    pub display: u32,
    pub name: Option<String>,
    /// In AppKit's coordinates (origin at the bottom-left of the main screen), points.
    pub frame: PtRect,
    pub scale: f64,
    /// Top-left corner in CoreGraphics' global coordinates (y down), points.
    pub origin: (f64, f64),
    /// How far down the menu bar reaches, and the notch (points).
    pub menu_bar: f64,
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
            let (frame, visible) = (screen.frame(), screen.visibleFrame());
            let menu_bar = frame.origin.y + frame.size.height - visible.origin.y - visible.size.height;
            Some(ScreenInfo {
                display,
                name: Some(screen.localizedName().to_string()),
                frame: PtRect::new(frame.origin.x, frame.origin.y, frame.size.width, frame.size.height),
                scale: screen.backingScaleFactor(),
                origin: (bounds.origin.x, bounds.origin.y),
                menu_bar: menu_bar.max(screen.safeAreaInsets().top),
            })
        })
        .collect()
}

/// A display, frozen: its pixels.
pub struct Capture {
    pub display: u32,
    pub frame: Frame,
}

/// Whether asides may capture the screen (without asking).
pub fn permitted() -> bool {
    CGPreflightScreenCaptureAccess()
}

/// Asks for the Screen Recording permission: the first time, macOS shows its prompt; after that,
/// the user has to allow it in System Settings.
pub fn request() -> bool {
    CGRequestScreenCaptureAccess()
}

type Done = Arc<dyn Fn(Result<Vec<Capture>, String>) + Send + Sync>;
/// A display's screenshot, or why there's none.
type Shot = Arc<dyn Fn(Result<Frame, String>) + Send + Sync>;
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
    if displays.is_empty() {
        return done(Err("No screen to capture.".into()));
    }
    // When nothing is left out, macOS 26 captures a part of the screen without listing what's
    // shareable first (30 to 50 ms of opening asides). Its quickest way (20 ms quicker than with a
    // configuration) has windows' shadows and leaves the pointer out; a picture that isn't 8 bits
    // a channel at the screen's size (the screen showing HDR) is taken again the other way.
    if without.is_none() && AnyClass::get(c"SCScreenshotConfiguration").is_some() {
        let ids: Vec<u32> = displays.iter().map(|d| d.0).collect();
        for ((id, width, height), shot) in displays.into_iter().zip(shots(&ids, done)) {
            let rect = CGDisplayBounds(id);
            let handler = RcBlock::new(move |image: *mut CGImage, _: *mut NSError| {
                let image = unsafe { image.as_ref() }
                    .filter(|i| (CGImage::width(Some(i)), CGImage::height(Some(i))) == (width, height));
                match image.and_then(srgb_frame) {
                    Some(frame) => shot(Ok(frame)),
                    None => screenshot_in(rect, width, height, shot.clone()),
                }
            });
            unsafe { SCScreenshotManager::captureImageInRect_completionHandler(rect, Some(&handler)) };
        }
        return;
    }
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
        let left_out: Vec<objc2::rc::Retained<SCWindow>> = match without {
            Some(pid) => unsafe { content.windows() }
                .iter()
                .filter(|w| unsafe { w.owningApplication() }.is_some_and(|a| unsafe { a.processID() } == pid))
                .collect(),
            None => Vec::new(),
        };
        let left_out = NSArray::from_retained_slice(&left_out);
        let ids: Vec<u32> = wanted.iter().map(|w| w.0).collect();
        for ((_, width, height, display), shot) in wanted.into_iter().zip(shots(&ids, done.clone())) {
            let filter = unsafe {
                SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &left_out)
            };
            if AnyClass::get(c"SCScreenshotConfiguration").is_some() {
                let handler = RcBlock::new(move |output: *mut SCScreenshotOutput, error: *mut NSError| {
                    shot(screenshot(output, error));
                });
                unsafe {
                    SCScreenshotManager::captureScreenshotWithFilter_configuration_completionHandler(
                        &filter,
                        &screenshot_configuration(width, height),
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
                shot(match unsafe { image.as_ref() } {
                    Some(image) => to_frame(image).ok_or_else(|| UNEXPECTED_FORMAT.into()),
                    None => Err(describe(error, "Could not capture the screen")),
                });
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

/// Captures window `window` (its `CGWindowID`) alone, whatever is over it, without its shadow:
/// its pixels and how many there are to a point. For an agent looking again at what a pin was on
/// (`asides_core::look`): nothing of the windows around it is in the picture. `done` runs once, on
/// a background queue.
pub fn capture_window(window: u32, done: impl Fn(Result<(Frame, f64), String>) + Send + Sync + 'static) {
    let done = Arc::new(done);
    let block = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let Some(content) = (unsafe { content.as_ref() }) else {
            return done(Err(describe(error, "Could not list the windows to look at")));
        };
        let Some(found) = unsafe { content.windows() }.iter().find(|w| unsafe { w.windowID() } == window) else {
            return done(Err("That window isn't on screen now.".into()));
        };
        let filter = unsafe { SCContentFilter::initWithDesktopIndependentWindow(SCContentFilter::alloc(), &found) };
        let (frame, scale) = unsafe { (found.frame(), f64::from(filter.pointPixelScale()).max(1.0)) };
        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth((frame.size.width * scale).round() as usize);
            config.setHeight((frame.size.height * scale).round() as usize);
            config.setShowsCursor(false);
            config.setIgnoreShadowsSingleWindow(true);
            config.setColorSpaceName(kCGColorSpaceSRGB);
        }
        let done = done.clone();
        let handler = RcBlock::new(move |image: *mut CGImage, error: *mut NSError| {
            done(match unsafe { image.as_ref() } {
                Some(image) => to_frame(image).map(|frame| (frame, scale)).ok_or_else(|| UNEXPECTED_FORMAT.into()),
                None => Err(describe(error, "Could not capture the window")),
            });
        });
        unsafe {
            SCScreenshotManager::captureImageWithFilter_configuration_completionHandler(
                &filter,
                &config,
                Some(&handler),
            )
        };
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
}

/// A Mac says what an app shows as it changes (`watch`): clips can be recorded.
pub fn watches() -> bool {
    true
}

/// What an app shows on a display, watched: dropped, it's watched no more.
pub struct Watch(Arc<Mutex<Watching>>);

/// The stream, once it runs, with what takes its frames; or that it's no longer wanted.
#[derive(Default)]
struct Watching {
    stream: Option<(Retained<SCStream>, Retained<FrameOutput>)>,
    stopped: bool,
}

// SAFETY: a stream is made, started and stopped from any thread (ScreenCaptureKit's own callbacks
// come on its queues), and what takes its frames is only kept alive here.
unsafe impl Send for Watching {}

impl Drop for Watch {
    fn drop(&mut self) {
        let mut watching = self.0.lock().unwrap();
        watching.stopped = true;
        let Some((stream, output)) = watching.stream.take() else { return };
        // What takes its frames lives until the stream has stopped handing them over.
        let (held, kept) = (stream.clone(), Mutex::new(Some(Held(output))));
        let stopped = RcBlock::new(move |_: *mut NSError| drop(kept.lock().unwrap().take()));
        unsafe { held.stopCaptureWithCompletionHandler(Some(&stopped)) };
    }
}

/// A frame output on its way to being let go, from whichever queue the stream stops on.
struct Held(#[allow(dead_code)] Retained<FrameOutput>);

// SAFETY: only dropped there; its callback is `Send + Sync`.
unsafe impl Send for Held {}

/// Frames a second, at most: every frame a 60 Hz screen shows (a flash can be one); of a part of the
/// screen (fewer pixels), every frame a 120 Hz one shows.
const FRAMES_A_SECOND: i32 = 60;
const AREA_FRAMES_A_SECOND: i32 = 120;

/// What a clip watches on a display.
pub enum Watched {
    /// An app's windows alone (its process): menus, popovers and sheets with them, no other app's.
    App(i32),
    /// A part of the display, in its points from its top-left corner: whatever shows there, but the
    /// windows of these processes (asides' own).
    Area(PtRect, Vec<i32>),
}

/// Watches what shows on `display` (`watched`), without the pointer, `width` × `height` pixels, in
/// sRGB: each frame it shows goes to `frame`, on a queue of the stream's own, until the `Watch` is
/// dropped. Nothing comes while nothing changes. `failed` says why it couldn't start, or stopped.
pub fn watch(
    display: u32,
    watched: Watched,
    (width, height): (usize, usize),
    frame: impl Fn(Frame) + Send + Sync + 'static,
    failed: impl Fn(String) + Send + Sync + 'static,
) -> Watch {
    let watching = Arc::new(Mutex::new(Watching::default()));
    let (state, frame) = (watching.clone(), Mutex::new(Some(Box::new(frame) as Box<dyn Fn(Frame) + Send + Sync>)));
    let failed = Arc::new(failed);
    let block = RcBlock::new(move |content: *mut SCShareableContent, error: *mut NSError| {
        let Some(content) = (unsafe { content.as_ref() }) else {
            return failed(describe(error, "Could not list what's on the screen"));
        };
        let Some(shown) = unsafe { content.displays() }.iter().find(|d| unsafe { d.displayID() } == display) else {
            return failed("That screen is gone.".into());
        };
        let apps = |pids: &[i32]| -> Vec<Retained<SCRunningApplication>> {
            unsafe { content.applications() }.iter().filter(|a| pids.contains(&unsafe { a.processID() })).collect()
        };
        let filter = match &watched {
            Watched::App(app) => {
                let running = apps(&[*app]);
                if running.is_empty() {
                    return failed("The app in front shows no window to record.".into());
                }
                let running = NSArray::from_retained_slice(&running);
                let alloc = SCContentFilter::alloc();
                unsafe {
                    SCContentFilter::initWithDisplay_includingApplications_exceptingWindows(
                        alloc,
                        &shown,
                        &running,
                        &NSArray::new(),
                    )
                }
            }
            Watched::Area(_, without) => {
                let ours = NSArray::from_retained_slice(&apps(without));
                let alloc = SCContentFilter::alloc();
                unsafe {
                    SCContentFilter::initWithDisplay_excludingApplications_exceptingWindows(
                        alloc,
                        &shown,
                        &ours,
                        &NSArray::new(),
                    )
                }
            }
        };
        let Some(frame) = frame.lock().unwrap().take() else { return };
        let config = unsafe { SCStreamConfiguration::new() };
        unsafe {
            config.setWidth(width);
            config.setHeight(height);
            config.setShowsCursor(false);
            config.setColorSpaceName(kCGColorSpaceSRGB);
            config.setPixelFormat(kCVPixelFormatType_32BGRA);
            match &watched {
                Watched::App(_) => config.setMinimumFrameInterval(CMTime::new(1, FRAMES_A_SECOND)),
                Watched::Area(rect, _) => {
                    let origin = CGPoint { x: rect.x, y: rect.y };
                    config.setSourceRect(CGRect { origin, size: CGSize { width: rect.width, height: rect.height } });
                    config.setMinimumFrameInterval(CMTime::new(1, AREA_FRAMES_A_SECOND));
                }
            }
        }
        let output = FrameOutput::new(frame);
        let stream =
            unsafe { SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, None) };
        let queue = DispatchQueue::new("asides.clip", DispatchQueueAttr::SERIAL);
        let added = unsafe {
            stream.addStreamOutput_type_sampleHandlerQueue_error(
                ProtocolObject::from_ref(&*output),
                SCStreamOutputType::Screen,
                Some(&queue),
            )
        };
        if let Err(error) = added {
            return failed(format!("Could not record the screen: {}", error.localizedDescription()));
        }
        // Dropped meanwhile: it never starts.
        let mut watching = state.lock().unwrap();
        if watching.stopped {
            return;
        }
        let failed = failed.clone();
        let started = RcBlock::new(move |error: *mut NSError| {
            if !error.is_null() {
                failed(describe(error, "Could not record the screen"));
            }
        });
        unsafe { stream.startCaptureWithCompletionHandler(Some(&started)) };
        watching.stream = Some((stream, output));
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&block) };
    Watch(watching)
}

/// What takes a stream's frames.
struct Taker {
    frame: Box<dyn Fn(Frame) + Send + Sync>,
}

define_class!(
    // SAFETY: `NSObject` asks nothing of its subclasses, and `FrameOutput` has no `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "AsidesFrameOutput"]
    #[ivars = Taker]
    struct FrameOutput;

    unsafe impl NSObjectProtocol for FrameOutput {}

    unsafe impl SCStreamOutput for FrameOutput {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn stream_did_output(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            // A sample without pixels says the screen didn't change.
            if kind == SCStreamOutputType::Screen
                && let Some(frame) = pixels(sample)
            {
                (self.ivars().frame)(frame);
            }
        }
    }
);

impl FrameOutput {
    fn new(frame: Box<dyn Fn(Frame) + Send + Sync>) -> Retained<FrameOutput> {
        let this = FrameOutput::alloc().set_ivars(Taker { frame });
        unsafe { msg_send![super(this), init] }
    }
}

/// A sample's pixels (BGRA, as `watch` asks for them) as a frame.
fn pixels(sample: &CMSampleBuffer) -> Option<Frame> {
    let buffer = unsafe { sample.image_buffer() }?;
    if CVPixelBufferGetPixelFormatType(&buffer) != kCVPixelFormatType_32BGRA {
        return None;
    }
    if unsafe { CVPixelBufferLockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) } != 0 {
        return None;
    }
    let (width, height) = (CVPixelBufferGetWidth(&buffer), CVPixelBufferGetHeight(&buffer));
    let (row, base) = (CVPixelBufferGetBytesPerRow(&buffer), CVPixelBufferGetBaseAddress(&buffer) as *const u8);
    let frame = (!base.is_null() && row >= width * 4)
        .then(|| {
            let mut rgba = vec![0; width * height * 4];
            for (y, line) in rgba.chunks_exact_mut(width * 4).enumerate() {
                // SAFETY: locked, the buffer has `height` rows of `row` bytes from `base`.
                let from = unsafe { std::slice::from_raw_parts(base.add(y * row), width * 4) };
                for (&[b, g, r, _], to) in from.as_chunks::<4>().0.iter().zip(line.as_chunks_mut::<4>().0) {
                    *to = [r, g, b, 255];
                }
            }
            Frame::new(width as u32, height as u32, rgba)
        })
        .flatten();
    unsafe { CVPixelBufferUnlockBaseAddress(&buffer, CVPixelBufferLockFlags::ReadOnly) };
    frame
}

const UNEXPECTED_FORMAT: &str = "The screenshot had an unexpected pixel format.";

/// One `Shot` per display in `ids`: `done` gets them all once the last is in.
fn shots(ids: &[u32], done: Done) -> Vec<Shot> {
    let results: Results = Arc::new(Mutex::new((ids.len(), Vec::new(), None)));
    ids.iter()
        .map(|&id| {
            let (results, done) = (results.clone(), done.clone());
            Arc::new(move |shot: Result<Frame, String>| {
                let mut state = results.lock().unwrap();
                state.0 -= 1;
                match shot {
                    Ok(frame) => state.1.push(Capture { display: id, frame }),
                    Err(error) => state.2 = Some(error),
                }
                if state.0 == 0 {
                    let (_, captures, error) = std::mem::take(&mut *state);
                    drop(state);
                    done(if captures.is_empty() { Err(error.unwrap_or_default()) } else { Ok(captures) });
                }
            }) as Shot
        })
        .collect()
}

/// The part of the screen at `rect` (CoreGraphics' coordinates) as macOS 26 takes a screenshot,
/// `width` × `height` pixels.
fn screenshot_in(rect: CGRect, width: usize, height: usize, shot: Shot) {
    let handler = RcBlock::new(move |output: *mut SCScreenshotOutput, error: *mut NSError| {
        shot(screenshot(output, error));
    });
    unsafe {
        SCScreenshotManager::captureScreenshotWithRect_configuration_completionHandler(
            rect,
            &screenshot_configuration(width, height),
            Some(&handler),
        )
    };
}

/// macOS 26's screenshots have windows' shadows, as the screen shows them; the older ones leave
/// them out, so a frozen screen was lighter under every window than the live one.
fn screenshot_configuration(width: usize, height: usize) -> objc2::rc::Retained<SCScreenshotConfiguration> {
    let config = unsafe { SCScreenshotConfiguration::new() };
    unsafe {
        config.setWidth(width as isize);
        config.setHeight(height as isize);
        config.setShowsCursor(false);
        config.setIgnoreShadows(false);
        config.setDynamicRange(SCScreenshotDynamicRange::SDR);
    }
    config
}

/// A macOS 26 screenshot's pixels. Its images are in the display's color space: in sRGB, colors
/// read like CSS.
fn screenshot(output: *mut SCScreenshotOutput, error: *mut NSError) -> Result<Frame, String> {
    match unsafe { output.as_ref() }.and_then(|o| unsafe { o.sdrImage() }) {
        Some(image) => srgb_frame(&image).ok_or_else(|| UNEXPECTED_FORMAT.into()),
        None => Err(describe(error, "Could not capture the screen")),
    }
}

/// `image`'s pixels in sRGB, as `to_frame` gives them. Converting a screen takes ColorSync 25 ms,
/// so each thread converts a strip of it: a pixel's color doesn't depend on its neighbors', so
/// they're the same as converted whole.
fn srgb_frame(image: &CGImage) -> Option<Frame> {
    let (width, height) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).clamp(1, 8);
    let rows = height.div_ceil(threads).max(1);
    let mut rgba = vec![0; width * height * 4];
    let converted = std::thread::scope(|scope| {
        let strips: Vec<_> = rgba
            .chunks_mut(rows * width * 4)
            .enumerate()
            .map(|(i, out)| {
                let size = CGSize::new(width as f64, (out.len() / (width * 4)) as f64);
                let strip =
                    CGImage::with_image_in_rect(Some(image), CGRect::new(CGPoint::new(0.0, (i * rows) as f64), size));
                scope.spawn(move || {
                    objc2::rc::autoreleasepool(|_| strip.and_then(|s| srgb(&s)).and_then(|s| copy_rgba(&s, out)))
                })
            })
            .collect();
        strips.into_iter().all(|s| s.join().ok().flatten().is_some())
    });
    if converted { Frame::new(width as u32, height as u32, rgba) } else { None }
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
    let (width, height) = (CGImage::width(Some(image)), CGImage::height(Some(image)));
    let mut rgba = vec![0; width * height * 4];
    copy_rgba(image, &mut rgba)?;
    Frame::new(width as u32, height as u32, rgba)
}

/// Writes the image's pixels into `out` (its width × height × 4 bytes) as `to_frame` gives them.
fn copy_rgba(image: &CGImage, out: &mut [u8]) -> Option<()> {
    let image = Some(image);
    let (width, height) = (CGImage::width(image), CGImage::height(image));
    if CGImage::bits_per_pixel(image) != 32
        || CGImage::bits_per_component(image) != 8
        || out.len() != width * height * 4
    {
        return None;
    }
    let row = CGImage::bytes_per_row(image);
    let data = CGDataProvider::data(CGImage::data_provider(image).as_deref())?;
    // SAFETY: the data is ours (a copy) and isn't changed while it's read.
    let data = unsafe { data.as_bytes_unchecked() };
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
    for (y, line) in out.chunks_exact_mut(width * 4).enumerate() {
        let pixels = data[y * row..y * row + width * 4].as_chunks::<4>().0;
        for (px, to) in pixels.iter().zip(line.as_chunks_mut::<4>().0) {
            *to = [px[r], px[g], px[b], 255];
        }
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_core_foundation::CFData;
    use objc2_core_graphics::{CGBitmapInfo, CGColorRenderingIntent, CGColorSpace, kCGColorSpaceDisplayP3};

    /// A screenshot as macOS 26 gives one: Display P3, BGRA, alpha first and premultiplied.
    fn display_p3(width: usize, height: usize) -> CFRetained<CGImage> {
        let mut seed = 7u32;
        let bytes: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                // Smooth ramps (like a screen) with noise on them, saturated colors included.
                let (x, y) = ((i % width) as u32, (i / width) as u32);
                let noise = (seed >> 16) as u8;
                [(x * 255 / width as u32) as u8 ^ (noise & 7), (y * 255 / height as u32) as u8, noise, 255]
            })
            .collect();
        let provider = CGDataProvider::with_cf_data(Some(&CFData::from_bytes(&bytes))).unwrap();
        let space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceDisplayP3 })).unwrap();
        let info = CGBitmapInfo(CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0);
        unsafe {
            CGImage::new(
                width,
                height,
                8,
                32,
                width * 4,
                Some(&space),
                info,
                Some(&provider),
                std::ptr::null(),
                false,
                CGColorRenderingIntent::RenderingIntentDefault,
            )
        }
        .unwrap()
    }

    #[test]
    fn converts_in_strips_as_whole() {
        // 333 rows don't split evenly: the last strip is shorter.
        let image = display_p3(301, 333);
        let whole = to_frame(&srgb(&image).unwrap()).unwrap();
        let strips = srgb_frame(&image).unwrap();
        assert_eq!((strips.width, strips.height), (301, 333));
        assert!(strips.rgba == whole.rgba, "converted in strips, the pixels changed");
        // It did convert: P3's saturated colors aren't sRGB's.
        let raw = to_frame(&image).unwrap();
        assert!(raw.rgba != whole.rgba);
    }

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

    /// Records a clip of app `$ASIDES_CLIP_APP` (its process id) on the main display, as the pill's
    /// button does, or with `$ASIDES_CLIP_AREA` (`x,y,width,height`, the display's points) of that
    /// part of it, as a pin's card's does (at the display's pixels, asides' own windows left out),
    /// for `$ASIDES_CLIP_SECONDS` (3), into `$ASIDES_CAPTURE_DUMP/clip-1.png` (and on) and what's
    /// said after them, `clip.txt`. `--ignored`; needs the Screen Recording permission.
    #[test]
    #[ignore]
    fn records_a_clip() {
        let dir = std::path::PathBuf::from(std::env::var("ASIDES_CAPTURE_DUMP").expect("ASIDES_CAPTURE_DUMP"));
        let seconds: f64 = std::env::var("ASIDES_CLIP_SECONDS").map_or(3.0, |s| s.parse().unwrap());
        let display = objc2_core_graphics::CGMainDisplayID();
        let area = std::env::var("ASIDES_CLIP_AREA").ok().map(|area| {
            let n: Vec<f64> = area.split(',').map(|n| n.trim().parse().unwrap()).collect();
            PtRect::new(n[0], n[1], n[2], n[3])
        });
        let (watched, size, pid, pin, place) = match area {
            Some(rect) => {
                use objc2_core_graphics::{CGDisplayCopyDisplayMode, CGDisplayMode};
                let mode = CGDisplayCopyDisplayMode(display);
                let scale =
                    CGDisplayMode::pixel_width(mode.as_deref()) as f64 / CGDisplayMode::width(mode.as_deref()) as f64;
                let size = ((rect.width * scale) as usize, (rect.height * scale) as usize);
                (Watched::Area(rect, vec![std::process::id() as i32]), size, 0, Some(1), "in pin 1's area")
            }
            None => {
                let app: i32 = std::env::var("ASIDES_CLIP_APP").expect("ASIDES_CLIP_APP").parse().unwrap();
                let bounds = objc2_core_graphics::CGDisplayBounds(display);
                let size = (bounds.size.width as usize, bounds.size.height as usize);
                (Watched::App(app), size, app, None, "in the test's app")
            }
        };
        let recording = crate::clip::Recording::start(pid, None, pin, |take| {
            watch(display, watched, size, move |frame| take(crate::clip::Shown::Frame(frame)), |why| panic!("{why}"))
        });
        std::thread::sleep(std::time::Duration::from_secs_f64(seconds));
        let made = recording.finish().expect("no frame came");
        for (i, png) in made.pngs.iter().enumerate() {
            std::fs::write(dir.join(format!("clip-{}.png", i + 1)), png).unwrap();
        }
        std::fs::write(dir.join("clip.txt"), made.summary.caption(place)).unwrap();
    }
}
