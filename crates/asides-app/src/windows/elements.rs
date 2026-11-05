//! What's under a pin on Windows, as `elements.rs` finds it on a Mac. The window list (taken when
//! the screen freezes) names the app and the window; UI Automation names the element itself: its
//! role and label, its frame, and in web pages (Chromium's and Firefox's) the DOM id and classes
//! and the page's address. Windows asks no permission for either.
//!
//! UI Automation's control types are named as macOS's accessibility roles (`AXButton`), so what's
//! made of them (`inspect::Element`) reads the same on both: `button "Save"`, `a#buy.cta`.
//! Chromium gives a web element's `id` as its automation id and its `class` as its class name.
//!
//! What's under a point is looked for in the window found there before asides opened, from it down
//! (its frozen screen is over everything now). An app can take a moment to answer: UI Automation
//! runs on a thread of its own (COM's multithreaded apartment), and a question not answered in time
//! gets the window alone.
//! Coordinates are global points (`asides_core::display`).

use std::collections::HashMap;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use asides_core::display::{self, Monitor};
use asides_core::inspect::{Element, Node, PtRect};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
use windows::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetClassNameW, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
};
use windows::core::{BOOL, HSTRING, Interface};

/// How long an app may take to answer UI Automation, and how long asides waits for the answer.
const UIA_TIMEOUT: Duration = Duration::from_millis(400);
const ANSWER_WAIT: Duration = Duration::from_millis(700);
/// How many ancestors to read above the element.
const MAX_DEPTH: usize = 14;
/// A window above the others (the taskbar, an always-on-top window): macOS's layers above 0.
const ABOVE: i64 = 25;

/// A window on screen, front to back.
#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub pid: i32,
    pub app: String,
    pub title: Option<String>,
    pub bounds: PtRect,
    pub layer: i64,
    /// Its handle: what's under a pin is looked for in it, not in asides' windows over it.
    pub handle: isize,
}

/// The monitors, for the coordinates.
fn monitors() -> Vec<Monitor> {
    display::monitors().into_iter().map(|l| l.monitor).collect()
}

/// The windows on screen now, frontmost first, without this app's or process `without`'s (the
/// overlay's).
pub fn windows(without: Option<i32>) -> Vec<WindowInfo> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the vector passed below, alive during the enumeration.
        let found = unsafe { &mut *(found.0 as *mut Vec<HWND>) };
        found.push(window);
        true.into()
    }
    let mut handles: Vec<HWND> = Vec::new();
    // SAFETY: the callback only touches `handles`, which outlives the call.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut handles as *mut Vec<HWND> as isize));
    }
    let monitors = monitors();
    let me = std::process::id() as i32;
    let mut names: HashMap<i32, String> = HashMap::new();
    handles
        .into_iter()
        .filter_map(|window| {
            // SAFETY: plain queries of a window that may have gone (they then fail).
            unsafe {
                if !IsWindowVisible(window).as_bool() || IsIconic(window).as_bool() || cloaked(window) {
                    return None;
                }
                let mut pid = 0;
                GetWindowThreadProcessId(window, Some(&mut pid));
                let pid = pid as i32;
                if pid == me || Some(pid) == without || pid == 0 {
                    return None;
                }
                let mut r = RECT::default();
                DwmGetWindowAttribute(
                    window,
                    DWMWA_EXTENDED_FRAME_BOUNDS,
                    (&mut r as *mut RECT).cast(),
                    size_of::<RECT>() as u32,
                )
                .ok()?;
                let (w, h) = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
                if w < 1.0 || h < 1.0 {
                    return None;
                }
                let style = GetWindowLongW(window, GWL_EXSTYLE) as u32;
                let above = style & (WS_EX_TOPMOST.0 | WS_EX_TOOLWINDOW.0) != 0 || class(window) == "Shell_TrayWnd";
                let app = names.entry(pid).or_insert_with(|| app_name(pid)).clone();
                Some(WindowInfo {
                    pid,
                    app,
                    title: title(window),
                    bounds: display::rect_to_points(&monitors, (r.left as f64, r.top as f64, w, h)),
                    layer: if above { ABOVE } else { 0 },
                    handle: window.0 as isize,
                })
            }
        })
        .collect()
}

/// Hidden by the system though "visible": another virtual desktop's, or a suspended app's.
unsafe fn cloaked(window: HWND) -> bool {
    let mut cloaked = 0u32;
    // SAFETY: Windows writes the flag, its size given.
    unsafe { DwmGetWindowAttribute(window, DWMWA_CLOAKED, (&mut cloaked as *mut u32).cast(), 4) }.is_ok()
        && cloaked != 0
}

unsafe fn title(window: HWND) -> Option<String> {
    // SAFETY: Windows writes at most the buffer's length.
    unsafe {
        let len = GetWindowTextLengthW(window);
        if len <= 0 {
            return None;
        }
        let mut text = vec![0u16; len as usize + 1];
        let got = GetWindowTextW(window, &mut text);
        Some(String::from_utf16_lossy(&text[..got as usize])).filter(|t| !t.trim().is_empty())
    }
}

unsafe fn class(window: HWND) -> String {
    let mut text = [0u16; 128];
    // SAFETY: Windows writes at most the buffer's length.
    let got = unsafe { GetClassNameW(window, &mut text) };
    String::from_utf16_lossy(&text[..got.max(0) as usize])
}

/// An app's name: its program's description (`Google Chrome`), else its file name (`chrome`).
fn app_name(pid: i32) -> String {
    let Some(path) = super::system::process_path(pid) else { return format!("process {pid}") };
    description(&path)
        .or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| format!("process {pid}"))
}

/// A program's `FileDescription`, from its version information.
fn description(path: &std::path::Path) -> Option<String> {
    let path = HSTRING::from(path.as_os_str());
    // SAFETY: the version block is read into a buffer its size; the values found point into it.
    unsafe {
        let size = GetFileVersionInfoSizeW(&path, None);
        if size == 0 {
            return None;
        }
        let mut block = vec![0u8; size as usize];
        GetFileVersionInfoW(&path, None, size, block.as_mut_ptr().cast()).ok()?;
        let (mut found, mut len) = (std::ptr::null_mut(), 0u32);
        let mut language = 0x0409_04b0;
        if VerQueryValueW(block.as_ptr().cast(), &HSTRING::from(r"\VarFileInfo\Translation"), &mut found, &mut len)
            .as_bool()
            && len >= 4
        {
            let pair = std::slice::from_raw_parts(found as *const u16, 2);
            language = ((pair[0] as u32) << 16) | pair[1] as u32;
        }
        let key = HSTRING::from(format!(r"\StringFileInfo\{language:08x}\FileDescription"));
        if !VerQueryValueW(block.as_ptr().cast(), &key, &mut found, &mut len).as_bool() || len == 0 {
            return None;
        }
        let text = std::slice::from_raw_parts(found as *const u16, len as usize);
        let text = String::from_utf16_lossy(text).trim_end_matches('\0').trim().to_string();
        (!text.is_empty()).then_some(text)
    }
}

/// The frontmost window at a point: an app's own window if there is one, else anything (the
/// taskbar).
pub fn window_at(windows: &[WindowInfo], x: f64, y: f64) -> Option<&WindowInfo> {
    windows
        .iter()
        .find(|w| w.layer == 0 && w.bounds.contains(x, y))
        .or_else(|| windows.iter().find(|w| w.bounds.contains(x, y)))
}

/// UI Automation needs no permission.
pub fn trusted() -> bool {
    true
}

pub fn request_trust() -> bool {
    true
}

/// What's at the global point `(x, y)`: the window (always), and with `deep` the element under it,
/// its frame in global points.
pub fn identify(windows: &[WindowInfo], x: f64, y: f64, deep: bool) -> Option<Element> {
    let window = window_at(windows, x, y)?;
    let nodes = if deep { element_at(window.handle, x, y) } else { Vec::new() };
    Some(Element::from_nodes(Some(window.app.clone()), window.title.clone(), &nodes))
}

/// Where the user is in the app with process id `pid`: its frontmost window, and with `deep` the
/// page in it, as `**In:** Edge · "Checkout — Shop" · http://…`, and that window's frame.
pub fn place(pid: i32, deep: bool) -> Option<(String, PtRect)> {
    let windows = windows(None);
    let window = windows.iter().find(|w| w.pid == pid && w.layer == 0)?;
    let b = window.bounds;
    let nodes = if deep { element_at(window.handle, b.x + b.width / 2.0, b.y + b.height / 2.0) } else { Vec::new() };
    let element = Element::from_nodes(Some(window.app.clone()), window.title.clone(), &nodes);
    let place = asides_core::inspect::place_markdown(&window.app, window.title.as_deref(), element.url.as_deref());
    Some((place, b))
}

type Job = Box<dyn FnOnce(&IUIAutomation) + Send>;

/// Starts UI Automation ahead of the first pin, from a thread of its own: its first answer (COM,
/// then the first calls into an app) can take longer than a pin waits for one, and the pin would be
/// only a point.
pub fn warm_up() {
    let spawned = std::thread::Builder::new().name("asides-automation-warm-up".into()).spawn(|| {
        // SAFETY: a plain query; the element is dropped at once.
        with_automation(|automation| unsafe { automation.GetRootElement() }.is_ok());
    });
    if let Err(err) = spawned {
        eprintln!("asides: UI Automation starts with the first pin: {err}");
    }
}

/// Runs `job` on UI Automation's thread, and waits for its answer (`None` if it's late).
fn with_automation<T: Send + 'static>(job: impl FnOnce(&IUIAutomation) -> T + Send + 'static) -> Option<T> {
    static THREAD: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();
    let jobs = THREAD.get_or_init(|| {
        let (jobs, asked) = channel::<Job>();
        let _ = std::thread::Builder::new().name("asides-automation".into()).spawn(move || {
            // SAFETY: this thread's apartment; the object lives as long as the thread.
            let automation: Option<IUIAutomation> = unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
            };
            let Some(automation) = automation else { return };
            if let Ok(timeouts) = automation.cast::<IUIAutomation2>() {
                // SAFETY: plain setters.
                unsafe {
                    let _ = timeouts.SetConnectionTimeout(UIA_TIMEOUT.as_millis() as u32);
                    let _ = timeouts.SetTransactionTimeout(UIA_TIMEOUT.as_millis() as u32);
                }
            }
            for job in asked {
                job(&automation);
            }
        });
        Mutex::new(jobs)
    });
    let (answer, answered) = channel();
    let job: Job = Box::new(move |automation| {
        let _ = answer.send(job(automation));
    });
    jobs.lock().ok()?.send(job).ok()?;
    answered.recv_timeout(ANSWER_WAIT).ok()
}

/// The element of window `handle` at the global point `(x, y)` and its ancestors up to the window,
/// innermost first.
pub fn element_at(handle: isize, x: f64, y: f64) -> Vec<Node> {
    let monitors = monitors();
    let (px, py) = display::to_pixels(&monitors, (x, y));
    let at = POINT { x: px as i32, y: py as i32 };
    with_automation(move |automation| nodes_at(automation, &monitors, handle, at)).unwrap_or_default()
}

/// How deep the search for the element under a point goes, and how many of an element's children
/// it looks at.
const MAX_SEARCH: usize = 48;
const MAX_CHILDREN: i32 = 2000;

/// The innermost element of window `handle` at `at`: from the window down, at each level the
/// smallest child the point is in (UI Automation's own `ElementFromPoint` would find asides' frozen
/// screen, over everything). Each level's children and their frames come in one call.
unsafe fn element_under(automation: &IUIAutomation, handle: isize, at: POINT) -> Option<IUIAutomationElement> {
    // SAFETY: COM calls on this apartment's objects.
    unsafe {
        let window = HWND(handle as *mut core::ffi::c_void);
        let mut current = automation.ElementFromHandle(window).ok()?;
        let cache = automation.CreateCacheRequest().ok()?;
        cache.AddProperty(UIA_BoundingRectanglePropertyId).ok()?;
        let condition = automation.ControlViewCondition().ok()?;
        for _ in 0..MAX_SEARCH {
            let Ok(children) = current.FindAllBuildCache(TreeScope_Children, &condition, &cache) else { break };
            let count = children.Length().unwrap_or(0).min(MAX_CHILDREN);
            let mut best: Option<(i64, IUIAutomationElement)> = None;
            for i in 0..count {
                let Ok(child) = children.GetElement(i) else { continue };
                let Ok(r) = child.CachedBoundingRectangle() else { continue };
                let inside = at.x >= r.left && at.x < r.right && at.y >= r.top && at.y < r.bottom;
                let area = (r.right - r.left) as i64 * (r.bottom - r.top) as i64;
                if inside && best.as_ref().is_none_or(|(a, _)| area < *a) {
                    best = Some((area, child));
                }
            }
            match best {
                Some((_, child)) => current = child,
                None => break,
            }
        }
        Some(current)
    }
}

fn nodes_at(automation: &IUIAutomation, monitors: &[Monitor], handle: isize, at: POINT) -> Vec<Node> {
    // SAFETY: COM calls on this apartment's objects; each fails cleanly when the app doesn't answer.
    unsafe {
        let Some(hit) = element_under(automation, handle, at) else { return Vec::new() };
        let (Ok(walker), Ok(root)) = (automation.ControlViewWalker(), automation.GetRootElement()) else {
            return Vec::new();
        };
        let mut nodes = Vec::new();
        let mut current = Some(hit);
        while let Some(element) = current {
            if automation.CompareElements(&element, &root).is_ok_and(|same| same.as_bool()) {
                break;
            }
            nodes.push(node(&element, monitors));
            if nodes.len() >= MAX_DEPTH {
                break;
            }
            current = walker.GetParentElement(&element).ok();
        }
        nodes
    }
}

/// UI Automation's frameworks that draw web pages: their ids and classes are the DOM's.
fn web(framework: &str) -> bool {
    matches!(framework, "Chrome" | "Gecko")
}

unsafe fn node(element: &IUIAutomationElement, monitors: &[Monitor]) -> Node {
    let text = |value: windows::core::Result<windows::core::BSTR>| {
        value.ok().map(|b| b.to_string().trim().to_string()).filter(|s| !s.is_empty())
    };
    // SAFETY: COM property reads; a failure is a missing value.
    unsafe {
        let control = element.CurrentControlType().map(|c| c.0).unwrap_or(UIA_CustomControlTypeId.0);
        let framework = text(element.CurrentFrameworkId()).unwrap_or_default();
        let level = element
            .cast::<IUIAutomationElement8>()
            .ok()
            .and_then(|e| e.CurrentHeadingLevel().ok())
            .map(|l| l.0 - HeadingLevel_None.0)
            .filter(|l| (1..=9).contains(l))
            .map(|l| l as u32);
        let role = role(control, web(&framework), level);
        let value = element
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|p| text(p.CurrentValue()));
        let url = (role == "AXWebArea").then(|| value.clone()).flatten().filter(|v| v.contains("://"));
        let identifier = text(element.CurrentAutomationId());
        let classes = text(element.CurrentClassName()).unwrap_or_default();
        let is_web = web(&framework);
        let frame = element.CurrentBoundingRectangle().ok().filter(|r| r.right > r.left && r.bottom > r.top).map(|r| {
            let size = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
            display::rect_to_points(monitors, (r.left as f64, r.top as f64, size.0, size.1))
        });
        Node {
            role: role.to_string(),
            subrole: None,
            role_description: text(element.CurrentLocalizedControlType()),
            title: text(element.CurrentName()),
            description: None,
            value: value.filter(|_| role != "AXWebArea"),
            placeholder: (role == "AXTextField").then(|| text(element.CurrentHelpText())).flatten(),
            identifier: identifier.clone().filter(|_| !is_web),
            dom_id: identifier.filter(|_| is_web),
            dom_classes: if is_web { classes.split_whitespace().map(String::from).collect() } else { Vec::new() },
            level,
            url,
            frame,
            selected_text: None,
        }
    }
}

/// UI Automation's control type as macOS's accessibility role.
fn role(control: i32, web: bool, level: Option<u32>) -> &'static str {
    let is = |id: UIA_CONTROLTYPE_ID| control == id.0;
    match () {
        _ if is(UIA_ButtonControlTypeId) => "AXButton",
        _ if is(UIA_SplitButtonControlTypeId) => "AXMenuButton",
        _ if is(UIA_CheckBoxControlTypeId) => "AXCheckBox",
        _ if is(UIA_RadioButtonControlTypeId) => "AXRadioButton",
        _ if is(UIA_ComboBoxControlTypeId) => "AXPopUpButton",
        _ if is(UIA_EditControlTypeId) => "AXTextField",
        _ if is(UIA_HyperlinkControlTypeId) => "AXLink",
        _ if is(UIA_ImageControlTypeId) => "AXImage",
        _ if is(UIA_ListControlTypeId) => "AXList",
        _ if is(UIA_ListItemControlTypeId) => "AXGroup",
        _ if is(UIA_MenuControlTypeId) => "AXMenu",
        _ if is(UIA_MenuBarControlTypeId) => "AXMenuBar",
        _ if is(UIA_MenuItemControlTypeId) => "AXMenuItem",
        _ if is(UIA_ProgressBarControlTypeId) => "AXProgressIndicator",
        _ if is(UIA_ScrollBarControlTypeId) => "AXScrollBar",
        _ if is(UIA_SliderControlTypeId) => "AXSlider",
        _ if is(UIA_SpinnerControlTypeId) => "AXIncrementor",
        _ if is(UIA_TabControlTypeId) => "AXTabGroup",
        _ if is(UIA_TabItemControlTypeId) => "AXTab",
        _ if is(UIA_TextControlTypeId) && level.is_some() => "AXHeading",
        _ if is(UIA_TextControlTypeId) => "AXStaticText",
        _ if is(UIA_ToolBarControlTypeId) => "AXToolbar",
        _ if is(UIA_ToolTipControlTypeId) => "AXHelpTag",
        _ if is(UIA_TreeControlTypeId) => "AXOutline",
        _ if is(UIA_TreeItemControlTypeId) | is(UIA_DataItemControlTypeId) => "AXRow",
        _ if is(UIA_DataGridControlTypeId) | is(UIA_TableControlTypeId) => "AXTable",
        _ if is(UIA_HeaderItemControlTypeId) => "AXCell",
        _ if is(UIA_DocumentControlTypeId) && web => "AXWebArea",
        _ if is(UIA_DocumentControlTypeId) => "AXTextArea",
        _ if is(UIA_WindowControlTypeId) => "AXWindow",
        _ if is(UIA_SeparatorControlTypeId) => "AXSplitter",
        _ if level.is_some() => "AXHeading",
        _ => "AXGroup",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_controls_as_macos_does() {
        assert_eq!(role(UIA_ButtonControlTypeId.0, false, None), "AXButton");
        assert_eq!(role(UIA_DocumentControlTypeId.0, true, None), "AXWebArea");
        assert_eq!(role(UIA_DocumentControlTypeId.0, false, None), "AXTextArea");
        assert_eq!(role(UIA_TextControlTypeId.0, true, Some(2)), "AXHeading");
        assert_eq!(role(UIA_PaneControlTypeId.0, false, None), "AXGroup");
    }

    /// Prints the windows, and what's under the middle of the one in front: run with `--ignored
    /// --nocapture`, on Windows in the desktop's session.
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
