//! What's under a Measure & Annotate marker. The window list (taken when the screen freezes) names
//! the app and window; with the Accessibility permission, the accessibility tree names the element
//! itself: its role and label, its frame, and in browsers the DOM id and classes and the page's URL.
//! Coordinates here are global, in points, origin at the top-left of the main screen.

use std::ffi::c_void;
use std::ptr;

use asides_core::inspect::{Element, Node, PtRect};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2_core_foundation::{CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGWindowListCopyWindowInfo, CGWindowListOption, kCGNullWindowID};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL};

/// How long to wait for an app to answer an accessibility query, in seconds.
const AX_TIMEOUT: f32 = 0.4;
/// How many ancestors to read above the element.
const MAX_DEPTH: usize = 14;

type AXUIElementRef = *const c_void;
type CFTypeRef = *const c_void;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> u8;
    fn AXIsProcessTrustedWithOptions(options: CFTypeRef) -> u8;
    fn AXUIElementCreateApplication(pid: i32) -> AXUIElementRef;
    fn AXUIElementCopyElementAtPosition(app: AXUIElementRef, x: f32, y: f32, element: *mut AXUIElementRef) -> i32;
    fn AXUIElementCopyAttributeValue(element: AXUIElementRef, attribute: CFTypeRef, value: *mut CFTypeRef) -> i32;
    fn AXUIElementSetAttributeValue(element: AXUIElementRef, attribute: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, out: *mut c_void) -> u8;
}

/// `kAXValueTypeCGPoint`, `…CGSize`, `…CGRect`.
const AX_POINT: u32 = 1;
const AX_SIZE: u32 = 2;
const AX_RECT: u32 = 3;

/// A window on screen, front to back, as the window server lists it.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub pid: i32,
    pub app: String,
    pub title: Option<String>,
    pub bounds: PtRect,
    pub layer: i64,
}

/// The windows on screen now, frontmost first, without this app's or process `without`'s (the
/// overlay's).
pub fn windows(without: Option<i32>) -> Vec<WindowInfo> {
    let options = CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let Some(list) = CGWindowListCopyWindowInfo(options, kCGNullWindowID) else { return Vec::new() };
    // CFArray of CFDictionary is toll-free bridged with NSArray of NSDictionary.
    let array: &NSArray<NSDictionary<NSString, AnyObject>> =
        unsafe { &*(CFRetained::as_ptr(&list).as_ptr() as *const NSArray<NSDictionary<NSString, AnyObject>>) };
    let me = std::process::id() as i32;
    let mut out = Vec::new();
    for dict in array.iter() {
        let get = |key: &str| dict.objectForKey(&NSString::from_str(key));
        let number =
            |o: Option<Retained<AnyObject>>| o.and_then(|o| o.downcast::<NSNumber>().ok()).map(|n| n.doubleValue());
        let text = |key: &str| {
            get(key).and_then(|o| o.downcast::<NSString>().ok()).map(|s| s.to_string()).filter(|s| !s.is_empty())
        };
        let Some(pid) = number(get("kCGWindowOwnerPID")) else { continue };
        if pid as i32 == me || Some(pid as i32) == without || number(get("kCGWindowAlpha")).is_some_and(|a| a <= 0.0) {
            continue;
        }
        let Some(bounds) = get("kCGWindowBounds").and_then(|o| o.downcast::<NSDictionary>().ok()) else { continue };
        let b = |key: &str| {
            let key = NSString::from_str(key);
            number(bounds.objectForKey(&key))
        };
        let (Some(x), Some(y), Some(w), Some(h)) = (b("X"), b("Y"), b("Width"), b("Height")) else { continue };
        out.push(WindowInfo {
            pid: pid as i32,
            app: text("kCGWindowOwnerName").unwrap_or_else(|| "?".into()),
            title: text("kCGWindowName"),
            bounds: PtRect::new(x, y, w, h),
            layer: number(get("kCGWindowLayer")).unwrap_or(0.0) as i64,
        });
    }
    out
}

/// The frontmost window at a point: an app's own window if there is one, else anything (the menu
/// bar, the Dock).
pub fn window_at(windows: &[WindowInfo], x: f64, y: f64) -> Option<&WindowInfo> {
    windows
        .iter()
        .find(|w| w.layer == 0 && w.bounds.contains(x, y))
        .or_else(|| windows.iter().find(|w| w.bounds.contains(x, y)))
}

pub fn trusted() -> bool {
    unsafe { AXIsProcessTrusted() != 0 }
}

/// Shows the system's Accessibility prompt (or does nothing when already allowed).
pub fn request_trust() -> bool {
    let key = NSString::from_str("AXTrustedCheckOptionPrompt");
    let yes = NSNumber::new_bool(true);
    let options = NSDictionary::<NSString, NSNumber>::from_slices(&[&*key], &[&*yes]);
    unsafe { AXIsProcessTrustedWithOptions(Retained::as_ptr(&options).cast()) != 0 }
}

pub fn open_settings() {
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
        .spawn();
}

/// What's at the global point `(x, y)`: the window (always), and with `deep` (and the permission)
/// the element under it, its frame in global points.
pub fn identify(windows: &[WindowInfo], x: f64, y: f64, deep: bool) -> Option<Element> {
    let window = window_at(windows, x, y)?;
    let nodes = if deep && trusted() { element_at(window.pid, x, y) } else { Vec::new() };
    Some(Element::from_nodes(Some(window.app.clone()), window.title.clone(), &nodes))
}

/// Where the user is in the app with process id `pid`: its frontmost window, and with `deep` (and
/// the permission) the page in it, as `**In:** Safari · "Checkout — Shop" · http://…`, and that
/// window's frame. `None` when it has no window on screen. Window titles need the Screen Recording
/// permission.
pub fn place(pid: i32, deep: bool) -> Option<(String, PtRect)> {
    let windows = windows(None);
    let window = windows.iter().find(|w| w.pid == pid && w.layer == 0)?;
    let b = window.bounds;
    let nodes = if deep && trusted() { element_at(pid, b.x + b.width / 2.0, b.y + b.height / 2.0) } else { Vec::new() };
    let element = Element::from_nodes(Some(window.app.clone()), window.title.clone(), &nodes);
    let place = asides_core::inspect::place_markdown(&window.app, window.title.as_deref(), element.url.as_deref());
    Some((place, b))
}

/// An accessibility object we own (+1), released when dropped.
struct Ax(Retained<AnyObject>);

impl Ax {
    /// Takes ownership of a reference returned by a Create or Copy function.
    fn from_owned(ptr: CFTypeRef) -> Option<Ax> {
        unsafe { Retained::from_raw(ptr as *mut AnyObject) }.map(Ax)
    }

    fn ptr(&self) -> CFTypeRef {
        Retained::as_ptr(&self.0).cast()
    }

    fn attribute(&self, name: &str) -> Option<Retained<AnyObject>> {
        let name = NSString::from_str(name);
        let mut value: CFTypeRef = ptr::null();
        let err = unsafe { AXUIElementCopyAttributeValue(self.ptr(), Retained::as_ptr(&name).cast(), &mut value) };
        if err != 0 {
            return None;
        }
        unsafe { Retained::from_raw(value as *mut AnyObject) }
    }

    fn string(&self, name: &str) -> Option<String> {
        let value = self.attribute(name)?;
        let text = value.downcast_ref::<NSString>().map(|s| s.to_string());
        text.map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    }

    fn url(&self, name: &str) -> Option<String> {
        let value = self.attribute(name)?;
        if let Some(url) = value.downcast_ref::<NSURL>() {
            return url.absoluteString().map(|s| s.to_string());
        }
        value.downcast_ref::<NSString>().map(|s| s.to_string()).filter(|s| s.contains("://"))
    }

    fn strings(&self, name: &str) -> Vec<String> {
        let Some(value) = self.attribute(name) else { return Vec::new() };
        let Some(array) = value.downcast_ref::<NSArray>() else { return Vec::new() };
        array.iter().filter_map(|item| item.downcast_ref::<NSString>().map(|s| s.to_string())).collect()
    }

    fn number(&self, name: &str) -> Option<u32> {
        self.attribute(name)?.downcast_ref::<NSNumber>().map(|n| n.unsignedIntValue())
    }

    fn frame(&self) -> Option<PtRect> {
        if let Some(value) = self.attribute("AXFrame") {
            let mut rect = CGRect::default();
            if unsafe { AXValueGetValue(Retained::as_ptr(&value).cast(), AX_RECT, (&mut rect as *mut CGRect).cast()) }
                != 0
            {
                return Some(PtRect::new(rect.origin.x, rect.origin.y, rect.size.width, rect.size.height));
            }
        }
        let (position, size) = (self.attribute("AXPosition")?, self.attribute("AXSize")?);
        let (mut p, mut s) = (CGPoint::default(), CGSize::default());
        let ok = unsafe {
            AXValueGetValue(Retained::as_ptr(&position).cast(), AX_POINT, (&mut p as *mut CGPoint).cast()) != 0
                && AXValueGetValue(Retained::as_ptr(&size).cast(), AX_SIZE, (&mut s as *mut CGSize).cast()) != 0
        };
        ok.then(|| PtRect::new(p.x, p.y, s.width, s.height))
    }

    fn node(&self) -> Node {
        Node {
            role: self.string("AXRole").unwrap_or_default(),
            subrole: self.string("AXSubrole"),
            role_description: self.string("AXRoleDescription"),
            title: self.string("AXTitle"),
            description: self.string("AXDescription"),
            value: self.string("AXValue"),
            placeholder: self.string("AXPlaceholderValue"),
            identifier: self.string("AXIdentifier"),
            dom_id: self.string("AXDOMIdentifier"),
            dom_classes: self.strings("AXDOMClassList"),
            level: self.heading_level(),
            url: self.url("AXURL").or_else(|| self.url("AXDocument")),
            frame: self.frame(),
            selected_text: self.string("AXSelectedText"),
        }
    }

    fn heading_level(&self) -> Option<u32> {
        (self.string("AXRole").as_deref() == Some("AXHeading")).then(|| self.number("AXValue")).flatten()
    }
}

/// The element of app `pid` at the global point `(x, y)` and its ancestors, innermost first.
pub fn element_at(pid: i32, x: f64, y: f64) -> Vec<Node> {
    let Some(app) = Ax::from_owned(unsafe { AXUIElementCreateApplication(pid) }) else { return Vec::new() };
    unsafe {
        AXUIElementSetMessagingTimeout(app.ptr(), AX_TIMEOUT);
        // Chromium and Electron apps only build their accessibility tree when asked to.
        let manual = NSString::from_str("AXManualAccessibility");
        let yes = NSNumber::new_bool(true);
        AXUIElementSetAttributeValue(app.ptr(), Retained::as_ptr(&manual).cast(), Retained::as_ptr(&yes).cast());
    }
    let mut hit: AXUIElementRef = ptr::null();
    if unsafe { AXUIElementCopyElementAtPosition(app.ptr(), x as f32, y as f32, &mut hit) } != 0 {
        return Vec::new();
    }
    let mut nodes = Vec::new();
    let mut current = Ax::from_owned(hit);
    while let Some(element) = current {
        let node = element.node();
        let top = node.role == "AXApplication";
        if !top {
            nodes.push(node);
        }
        if top || nodes.len() >= MAX_DEPTH {
            break;
        }
        current = element.attribute("AXParent").map(Ax);
    }
    nodes
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prints where the user is in the app in front (and in Safari, when it's open): run with
    /// `--ignored --nocapture`; titles need the Screen Recording permission, URLs Accessibility.
    #[test]
    #[ignore]
    fn prints_the_place_in_front() {
        let front = crate::macos::frontmost_app().unwrap();
        println!("front: {:?}", place(front, true));
        for app in objc2_app_kit::NSWorkspace::sharedWorkspace().runningApplications() {
            if app.bundleIdentifier().is_some_and(|id| id.to_string() == "com.apple.Safari") {
                println!("Safari: {:?}", place(app.processIdentifier(), true));
            }
        }
    }
}
