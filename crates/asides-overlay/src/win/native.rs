//! What GPUI's windows need to be an overlay on Windows, as `native.rs` does for macOS: borderless,
//! above everything (the taskbar too), exactly over their screen, and no taskbar buttons.
//!
//! The windows are given by their handles (`HWND`), as macOS's are by their views. Frames are in
//! the coordinates the overlay shares with macOS (AppKit's: y up from the main screen's bottom,
//! points), each screen at its own scale (`asides_core::display`).
//!
//! - **The keyboard.** Windows lets a process bring its window to the front only when it was given
//!   the right: the app hands it on as its shortcut is pressed (`AllowSetForegroundWindow`).
//! - **The pill** never takes the keyboard (`WS_EX_NOACTIVATE`), and is left out of captures
//!   (`WDA_EXCLUDEFROMCAPTURE`), so freezing again over the live screen doesn't take it with it. A
//!   frozen screen brought forward goes just below it (`below_pill`).
//! - **Frozen**, the screens' windows are above the taskbar; what Windows lets an app hold back
//!   stays back while they're up (`kiosk`): the Windows key, Alt+Tab, Alt+Esc and Ctrl+Esc, through
//!   a keyboard hook (Windows asks no permission for it). Ctrl+Alt+Del stays, a way out.
//! - **The glass**: Windows blurs a whole window or nothing; the pill is its tint over what's
//!   behind it (`set_glass`). The asides window is Windows 11's acrylic (`window_glass`).

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use asides_core::display::{self, Monitor};
use asides_core::inspect::PtRect;
use gpui::Window;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute};
use windows::Win32::System::Threading::AttachThreadInput;
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, SetFocus, VK_CONTROL, VK_ESCAPE, VK_LBUTTON, VK_LWIN, VK_RBUTTON, VK_RWIN, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The overlay's own windows: the screens' and the pill's, by their handles.
static SCREENS: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

fn hwnd(window: usize) -> Option<HWND> {
    (window != 0).then_some(HWND(window as *mut core::ffi::c_void))
}

fn monitors() -> Vec<Monitor> {
    display::monitors().into_iter().map(|l| l.monitor).collect()
}

/// Windows 11 2022 Update (build 22621): DWM's system backdrops.
const SYSTEM_BACKDROPS: u32 = 22621;

/// Windows' build number (22000 is Windows 11), once asked.
fn windows_build() -> u32 {
    use windows::Wdk::System::SystemServices::RtlGetVersion;
    use windows::Win32::System::SystemInformation::OSVERSIONINFOW;
    static BUILD: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *BUILD.get_or_init(|| {
        let mut info = OSVERSIONINFOW { dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32, ..Default::default() };
        // SAFETY: `info` has its size filled in, as RtlGetVersion takes (it says the real version,
        // whatever the program's manifest).
        match unsafe { RtlGetVersion(&mut info) }.ok() {
            Ok(()) => info.dwBuildNumber,
            Err(_) => 0,
        }
    })
}

/// The asides window's glass. From the 2022 Update, the system's acrylic, set by `dress` (as
/// helio's window): GPUI asks for no accent of its own and draws see-through where it draws
/// nothing (it clears each frame so). GPUI's own blur, the older acrylic
/// (`SetWindowCompositionAttribute`), thins out where the window meets the screen's edge; it's
/// the nearest Windows 10 and the first Windows 11 have.
pub fn window_glass() -> gpui::WindowBackgroundAppearance {
    match windows_build() >= SYSTEM_BACKDROPS {
        true => gpui::WindowBackgroundAppearance::Opaque,
        false => gpui::WindowBackgroundAppearance::Blurred,
    }
}

/// The font of Windows' caption buttons' glyphs: Windows 11's, or Windows 10's (the same code
/// points); missing, they'd show as boxes.
pub fn caption_font() -> &'static str {
    match windows_build() >= 22000 {
        true => "Segoe Fluent Icons",
        false => "Segoe MDL2 Assets",
    }
}

/// Once the asides window is open: the system's acrylic behind it (see `window_glass`).
pub fn dress(window: &Window) {
    use windows::Win32::Graphics::Dwm::{DWMSBT_TRANSIENTWINDOW, DWMWA_SYSTEMBACKDROP_TYPE};
    let Some(handle) = hwnd(view_pointer(window)) else { return };
    if windows_build() < SYSTEM_BACKDROPS {
        return;
    }
    let backdrop = DWMSBT_TRANSIENTWINDOW;
    // SAFETY: the window is this process's, open; the attribute takes a DWM_SYSTEMBACKDROP_TYPE,
    // passed with its size.
    let set = unsafe {
        DwmSetWindowAttribute(
            handle,
            DWMWA_SYSTEMBACKDROP_TYPE,
            &backdrop as *const _ as *const core::ffi::c_void,
            size_of_val(&backdrop) as u32,
        )
    };
    if let Err(err) = set {
        eprintln!("asides-overlay: the window has no acrylic: {err}");
    }
}

/// Linux's: Windows draws no window's title bar for it.
pub fn draws_its_title_bar(_window: &Window) -> bool {
    false
}

/// macOS's Dock icon: Windows' tool windows have no taskbar button.
pub fn accessory() {}

pub fn stay_accessory_at_launch() {}

/// The window's handle: what the native calls here take.
pub fn view_pointer(window: &Window) -> usize {
    match HasWindowHandle::window_handle(window).map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => h.hwnd.get() as usize,
        _ => 0,
    }
}

/// A frame (AppKit's coordinates) as Windows' rectangle in pixels.
fn pixels(frame: PtRect) -> (i32, i32, i32, i32) {
    display::frame_to_pixels(&monitors(), frame)
}

/// The whole window is GPUI's to draw: GPUI keeps a resizable frame's thickness around a window
/// without a title bar (about 8 pixels left, right and below), which would cut the pill and move a
/// frozen screen off its pixels.
unsafe extern "system" fn whole_client(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if message == WM_NCCALCSIZE && wparam.0 != 0 {
        return LRESULT(0);
    }
    if message == WM_WINDOWPOSCHANGING {
        below_pill(window, lparam);
    }
    // SAFETY: the rest goes on to GPUI's own handling.
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(window, message, wparam, lparam) }
}

/// A frozen screen brought forward goes just below the pill. Windows raises the window it activates
/// (a click on a screen, after one on the pill made the pill's window the active one) to the top,
/// over the pill, which then stayed hidden behind the screen until the next freeze. Windows says
/// where a window goes as the window it goes behind, `HWND_TOPMOST` already worked out.
fn below_pill(window: HWND, lparam: LPARAM) {
    let Some(pill) = hwnd(PILL.load(Ordering::Relaxed) as usize).filter(|&p| p != window) else { return };
    // SAFETY: WM_WINDOWPOSCHANGING's `lparam` is the change about to be made, ours to adjust.
    let change = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
    let lowered = change.hwndInsertAfter == HWND_BOTTOM || change.hwndInsertAfter == HWND_NOTOPMOST;
    // SAFETY: a plain query.
    if !change.flags.contains(SWP_NOZORDER) && !lowered && unsafe { IsWindowVisible(pill) }.as_bool() {
        change.hwndInsertAfter = pill;
    }
}

fn style(window: HWND, extended: WINDOW_EX_STYLE) {
    // SAFETY: plain style changes on a window of this process; the subclass lives as long as it.
    unsafe {
        let _ = windows::Win32::UI::Shell::SetWindowSubclass(window, Some(whole_client), 1, 0);
        SetWindowLongPtrW(window, GWL_STYLE, (WS_POPUP.0 | WS_CLIPCHILDREN.0) as isize);
        let before = GetWindowLongPtrW(window, GWL_EXSTYLE) as u32;
        SetWindowLongPtrW(window, GWL_EXSTYLE, (before | extended.0) as isize);
        // No rounded corners or border: the screen's window covers it exactly.
        let corner = DWMWCP_DONOTROUND;
        let _ = DwmSetWindowAttribute(
            window,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const _ as *const core::ffi::c_void,
            size_of_val(&corner) as u32,
        );
    }
}

fn place(window: HWND, frame: PtRect, flags: SET_WINDOW_POS_FLAGS) {
    let (x, y, w, h) = pixels(frame);
    // SAFETY: a plain call on a window of this process.
    let _ = unsafe { SetWindowPos(window, Some(HWND_TOPMOST), x, y, w, h, flags | SWP_NOACTIVATE | SWP_FRAMECHANGED) };
}

/// Makes `window` a frozen screen's at `frame`: topmost, no frame, no taskbar button. Returns its
/// handle twice (macOS's window number and view).
pub fn make_overlay(window: &Window, frame: PtRect) -> (isize, usize) {
    let view = view_pointer(window);
    let Some(handle) = hwnd(view) else { return (0, 0) };
    style(handle, WS_EX_TOPMOST | WS_EX_TOOLWINDOW);
    place(handle, frame, SWP_NOOWNERZORDER);
    SCREENS.lock().unwrap().push(view as isize);
    (view as isize, view)
}

/// Makes `window` the pill's at `frame`: topmost, above the frozen screens, never taking the
/// keyboard, and out of captures. Returns its handle twice.
pub fn make_toolbar(window: &Window, frame: PtRect) -> (isize, usize) {
    let view = view_pointer(window);
    let Some(handle) = hwnd(view) else { return (0, 0) };
    style(handle, WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE);
    // SAFETY: a plain call on a window of this process.
    let _ = unsafe { SetWindowDisplayAffinity(handle, WDA_EXCLUDEFROMCAPTURE) };
    place(handle, frame, SWP_NOOWNERZORDER);
    PILL.store(view as isize, Ordering::Relaxed);
    (view as isize, view)
}

/// The pill's window.
static PILL: AtomicIsize = AtomicIsize::new(0);

/// Draws these windows now, as GPUI would their next frames: GPUI draws on `WM_PAINT`, sent here
/// even to hidden ones, so they show it when they're shown, not what they showed last.
pub fn draw_now(views: &[usize]) {
    for window in views.iter().filter_map(|&v| hwnd(v)) {
        // SAFETY: a message GPUI handles on this thread.
        unsafe { SendMessageW(window, WM_PAINT, None, None) };
    }
}

/// macOS commits what's moved and drawn together; Windows shows each as it's done.
pub fn transaction<R>(f: impl FnOnce() -> R) -> R {
    f()
}

/// Shows the window, above the others; with `key`, it takes the keyboard too.
pub fn show(pointer: usize, key: bool) {
    let Some(window) = hwnd(pointer) else { return };
    // SAFETY: plain calls on a window of this process.
    unsafe {
        let _ = SetWindowPos(
            window,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE,
        );
        if key {
            take_keyboard(window);
        }
    }
    // The pill stays above the frozen screens (`below_pill` keeps it there after).
    if let Some(pill) = hwnd(PILL.load(Ordering::Relaxed) as usize).filter(|p| *p != window) {
        // SAFETY: as above.
        let _ = unsafe { SetWindowPos(pill, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE) };
    }
}

/// Brings `window` to the front with the keyboard: allowed since the app handed the right on. If
/// Windows still declines, the thread in front shares its input with this one for the moment it
/// takes (what Windows itself does for a window the user clicks).
unsafe fn take_keyboard(window: HWND) {
    unsafe {
        if SetForegroundWindow(window).as_bool() {
            let _ = SetFocus(Some(window));
            return;
        }
        let front = GetForegroundWindow();
        let theirs = GetWindowThreadProcessId(front, None);
        let ours = windows::Win32::System::Threading::GetCurrentThreadId();
        let attached = theirs != 0 && theirs != ours && AttachThreadInput(ours, theirs, true).as_bool();
        let _ = BringWindowToTop(window);
        let _ = SetForegroundWindow(window);
        let _ = SetFocus(Some(window));
        if attached {
            let _ = AttachThreadInput(ours, theirs, false);
        }
    }
}

/// Shows the pill's window, which never takes the keyboard.
pub fn appear(pointer: usize) {
    let Some(window) = hwnd(pointer) else { return };
    // SAFETY: a plain call on a window of this process.
    let _ = unsafe {
        SetWindowPos(window, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW | SWP_NOACTIVATE)
    };
}

/// The window is on screen.
pub fn on_screen(pointer: usize) -> bool {
    // SAFETY: a plain query.
    hwnd(pointer).is_some_and(|w| unsafe { IsWindowVisible(w) }.as_bool())
}

pub fn hide(pointer: usize) {
    if let Some(window) = hwnd(pointer) {
        // SAFETY: a plain call on a window of this process.
        let _ = unsafe { ShowWindow(window, SW_HIDE) };
    }
}

/// The window's frame, in AppKit's coordinates.
pub fn frame(pointer: usize) -> Option<PtRect> {
    let mut r = RECT::default();
    // SAFETY: Windows fills the rectangle.
    unsafe { GetWindowRect(hwnd(pointer)?, &mut r) }.ok()?;
    let size = ((r.right - r.left) as f64, (r.bottom - r.top) as f64);
    Some(display::pixels_to_frame(&monitors(), (r.left as f64, r.top as f64, size.0, size.1)))
}

/// Development (replays): what the screen shows in `frame`, every window, as a PNG at `path`.
pub fn capture(frame: PtRect, path: &Path) -> bool {
    use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleBitmap, CreateCompatibleDC,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SRCCOPY, SelectObject,
    };
    let (x, y, w, h) = pixels(frame);
    if w <= 0 || h <= 0 {
        return false;
    }
    let mut bgra = vec![0u8; w as usize * h as usize * 4];
    // SAFETY: GDI objects made here and released here; the bits are read into a buffer their size.
    let read = unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(dc, bitmap.into());
        let copied = BitBlt(dc, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT).is_ok();
        let mut info = BITMAPINFO {
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
        let lines = GetDIBits(dc, bitmap, 0, h as u32, Some(bgra.as_mut_ptr().cast()), &mut info, DIB_RGB_COLORS);
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        copied && lines == h
    };
    if !read {
        return false;
    }
    for pixel in bgra.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let size = tiny_skia::IntSize::from_wh(w as u32, h as u32);
    size.and_then(|size| tiny_skia::Pixmap::from_vec(bgra, size)).is_some_and(|image| image.save_png(path).is_ok())
}

/// Development (replays): the pill shows in captures, which leave it out (`make_toolbar`).
pub fn show_in_captures(pointer: usize) {
    if let Some(window) = hwnd(pointer) {
        // SAFETY: a plain call on a window of this process.
        let _ = unsafe { SetWindowDisplayAffinity(window, WDA_NONE) };
    }
}

/// The window's scale (pixels per point).
fn scale(window: HWND) -> f64 {
    // SAFETY: a plain query.
    let dpi = unsafe { windows::Win32::UI::HiDpi::GetDpiForWindow(window) };
    if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }
}

/// The pill's glass, kept to its shapes on macOS. Windows blurs a whole window or nothing, and
/// doesn't cut what GPUI draws to a window's region: the pill's window is see-through, the pill
/// its tint over what's behind (`windows::toolbar_options`).
pub fn set_glass(_pointer: usize, _shapes: &[(PtRect, f64, f64)]) {}

/// Moves the window, the same size.
pub fn move_to(pointer: usize, (x, y): (f64, f64)) {
    let Some(window) = hwnd(pointer) else { return };
    let Some(f) = frame(pointer) else { return };
    let (px, py, _, _) = pixels(PtRect::new(x, y, f.width, f.height));
    // SAFETY: a plain call on a window of this process.
    let _ = unsafe { SetWindowPos(window, None, px, py, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// Moves and resizes the window.
pub fn set_frame(pointer: usize, frame: PtRect) {
    let Some(window) = hwnd(pointer) else { return };
    let (x, y, w, h) = pixels(frame);
    // SAFETY: a plain call on a window of this process.
    let _ = unsafe { SetWindowPos(window, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE) };
}

/// Tells the window where the pointer is now (it moved while the window was hidden): GPUI drops
/// what was under the pointer when it was last shown, and Measure follows it from there.
pub fn pointer_moved(pointer: usize) {
    let Some(window) = hwnd(pointer) else { return };
    let mut at = POINT::default();
    // SAFETY: plain calls; the move is handled by GPUI on this thread.
    unsafe {
        if GetCursorPos(&mut at).is_err() || !windows::Win32::Graphics::Gdi::ScreenToClient(window, &mut at).as_bool() {
            return;
        }
        let packed = ((at.y as u16 as u32) << 16 | at.x as u16 as u32) as isize;
        SendMessageW(window, WM_MOUSEMOVE, None, Some(LPARAM(packed)));
    }
}

/// The keyboard hook holding back what would take the user away from the frozen screens.
static KIOSK: AtomicBool = AtomicBool::new(false);
static KEYBOARD_HOOK: AtomicIsize = AtomicIsize::new(0);

unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && KIOSK.load(Ordering::Relaxed) {
        // SAFETY: Windows passes the key's details with each event.
        let key = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let down = |vk: u16| unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0;
        let alt = key.flags.0 & LLKHF_ALTDOWN.0 != 0;
        let vk = key.vkCode as u16;
        let held = vk == VK_LWIN.0
            || vk == VK_RWIN.0
            || (alt && (vk == VK_TAB.0 || vk == VK_ESCAPE.0))
            || (vk == VK_ESCAPE.0 && down(VK_CONTROL.0));
        if held {
            return LRESULT(1);
        }
    }
    // SAFETY: the next hook gets what this one passes on.
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// While the screens are frozen, nothing Windows lets an app hold back takes the user away: the
/// Windows key (Start, Task View), Alt+Tab, Alt+Esc and Ctrl+Esc. Live, the user's keys again.
pub fn kiosk(on: bool) {
    KIOSK.store(on, Ordering::Relaxed);
    if on && KEYBOARD_HOOK.load(Ordering::Relaxed) == 0 {
        // SAFETY: a low-level hook runs on this thread, which handles messages (GPUI's loop).
        let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), None, 0) };
        if let Ok(hook) = hook {
            KEYBOARD_HOOK.store(hook.0 as isize, Ordering::Relaxed);
        }
    }
    if on {
        activate();
    }
}

/// Brings the overlay's frozen screen to the front, with the keyboard; in the window's process,
/// the asides window. Windows lets a process come forward only when it may (the app hands that on,
/// `allow_focus`); `asides` run in a terminal can't, so it takes it from the app in front
/// (`take_keyboard`).
pub fn activate() {
    let screens = SCREENS.lock().unwrap().clone();
    // SAFETY: plain queries and calls on windows of this process.
    let shown = screens.into_iter().filter_map(|w| hwnd(w as usize)).find(|&w| unsafe { IsWindowVisible(w) }.as_bool());
    if let Some(window) = shown.or_else(main_window) {
        unsafe { take_keyboard(window) };
    }
}

/// This thread's visible window that has a taskbar button: the asides window.
fn main_window() -> Option<HWND> {
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> windows::core::BOOL {
        // SAFETY: `found` is the `Option<HWND>` below, alive during the enumeration.
        let found = unsafe { &mut *(found.0 as *mut Option<HWND>) };
        // SAFETY: plain queries.
        let tool = unsafe { GetWindowLongPtrW(window, GWL_EXSTYLE) } as u32 & WS_EX_TOOLWINDOW.0 != 0;
        if unsafe { IsWindowVisible(window) }.as_bool() && !tool {
            *found = Some(window);
            return false.into();
        }
        true.into()
    }
    let mut found: Option<HWND> = None;
    // SAFETY: the callback only writes `found`, which outlives the call.
    unsafe {
        let thread = windows::Win32::System::Threading::GetCurrentThreadId();
        let _ = EnumThreadWindows(thread, Some(each), LPARAM(&mut found as *mut _ as isize));
    }
    found
}

/// The overlay has the keyboard: its window is in front.
pub fn is_active() -> bool {
    // SAFETY: plain queries.
    let pid = unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid
    };
    pid == std::process::id()
}

/// The app in front (its process id), unless it's the overlay's.
pub fn front_app() -> Option<i32> {
    // SAFETY: plain queries.
    let pid = unsafe {
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        pid
    };
    (pid != 0 && pid != std::process::id()).then_some(pid as i32)
}

/// Gives the keyboard back to the app with this process id, if the overlay has it: its main window
/// comes to the front (the overlay, in front, may bring it).
pub fn give_focus(pid: Option<i32>) {
    if !is_active() {
        return;
    }
    let Some(pid) = pid else { return };
    struct Found {
        pid: u32,
        window: Option<HWND>,
    }
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> windows::core::BOOL {
        // SAFETY: `found` is the `Found` passed below, alive during the enumeration.
        let found = unsafe { &mut *(found.0 as *mut Found) };
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
        let owned = unsafe { GetWindow(window, GW_OWNER) }.is_ok_and(|o| !o.is_invalid());
        if pid == found.pid && unsafe { IsWindowVisible(window) }.as_bool() && !owned {
            found.window = Some(window);
            return false.into();
        }
        true.into()
    }
    let mut found = Found { pid: pid as u32, window: None };
    // SAFETY: `found` outlives the enumeration, which calls back on this thread.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut found as *mut Found as isize));
        if let Some(window) = found.window {
            let _ = SetForegroundWindow(window);
        }
    }
}

/// The folder picker, above the overlay's windows and with the keyboard; `done` gets the folder, or
/// `None` when it's cancelled. Windows' dialog is modal: the overlay waits on it.
pub fn choose_folder(start: Option<&Path>, done: impl FnOnce(Option<PathBuf>) + 'static) {
    done(pick_folder(start));
}

fn pick_folder(start: Option<&Path>) -> Option<PathBuf> {
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
    use windows::Win32::UI::Shell::{
        FOS_FORCEFILESYSTEM, FOS_PICKFOLDERS, FileOpenDialog, IFileOpenDialog, IShellItem, SHCreateItemFromParsingName,
        SIGDN_FILESYSPATH,
    };
    use windows::core::HSTRING;
    let owner = SCREENS
        .lock()
        .unwrap()
        .iter()
        .filter_map(|&w| hwnd(w as usize))
        .find(|&w| unsafe { IsWindowVisible(w) }.as_bool());
    // SAFETY: COM calls on this thread's apartment (GPUI's, initialized for it); the path's text
    // is freed after it's copied.
    unsafe {
        let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
        let options = dialog.GetOptions().ok()?;
        dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM).ok()?;
        let _ = dialog.SetTitle(&HSTRING::from("The folder the new session starts in"));
        let _ = dialog.SetOkButtonLabel(&HSTRING::from("Start Here"));
        if let Some(start) = start
            && let Ok(item) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(start.as_os_str()), None)
        {
            let _ = dialog.SetFolder(&item);
        }
        dialog.Show(owner).ok()?;
        let item = dialog.GetResult().ok()?;
        let path = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let text = path.to_string().ok();
        windows::Win32::System::Com::CoTaskMemFree(Some(path.0 as *const core::ffi::c_void));
        text.map(PathBuf::from)
    }
}

/// The screens: each one's display (GPUI's id) and frame (AppKit's coordinates).
pub fn screens() -> Vec<(u32, PtRect)> {
    let monitors = monitors();
    monitors.iter().enumerate().map(|(i, m)| (i as u32, display::frame(&monitors, m))).collect()
}

/// The part of the screen at `frame` (AppKit's coordinates) that the taskbar leaves.
pub fn visible_frame(frame: PtRect) -> PtRect {
    let listed = display::monitors();
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

/// The left mouse button is down (anywhere), whichever button the user made primary.
pub fn mouse_pressed() -> bool {
    // SAFETY: plain queries.
    unsafe {
        let swapped = GetSystemMetrics(SM_SWAPBUTTON) != 0;
        let button = if swapped { VK_RBUTTON } else { VK_LBUTTON };
        GetAsyncKeyState(button.0 as i32) as u16 & 0x8000 != 0
    }
}

/// macOS's run loop tells its watchdog what the main thread does; GPUI's loop on Windows has no
/// such observer. A stuck overlay is shown by Windows as not responding, and can be closed then.
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

/// GPUI's Metal pipelines are macOS's; nothing to change in its Direct3D ones.
pub fn blend_alpha_over() {}

thread_local! {
    /// What `on_system_changes` calls.
    static CHANGED: Cell<Option<fn()>> = const { Cell::new(None) };
}

/// Whether the pointer's moves matter (`on_system_changes`).
static POINTER_WATCHED: AtomicBool = AtomicBool::new(false);

/// Calls `changed` when Windows says what the overlay waits on may have changed: one of its windows
/// showed or hid, an app came to the front, or the pointer moved while `watch_pointer` asks.
pub fn on_system_changes(changed: fn()) {
    CHANGED.with(|c| c.set(Some(changed)));
    unsafe extern "system" fn event(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
        if let Some(changed) = CHANGED.with(Cell::get) {
            changed();
        }
    }
    unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code == HC_ACTION as i32
            && POINTER_WATCHED.load(Ordering::Relaxed)
            && let Some(changed) = CHANGED.with(Cell::get)
        {
            changed();
        }
        // SAFETY: the next hook gets what this one passes on.
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
    let me = std::process::id();
    // SAFETY: the callbacks are plain functions, called on this thread as it handles messages.
    unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(event),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        );
        SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE, None, Some(event), me, 0, WINEVENT_OUTOFCONTEXT);
        let _ = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), None, 0);
    }
}

pub fn watch_pointer(watched: bool) {
    POINTER_WATCHED.store(watched, Ordering::Relaxed);
}

pub fn debug_focus() {}

/// The mouse, in AppKit's global coordinates.
pub fn mouse() -> (f64, f64) {
    let mut at = POINT::default();
    // SAFETY: Windows fills the point.
    let _ = unsafe { GetCursorPos(&mut at) };
    let monitors = monitors();
    let (x, y) = display::to_points(&monitors, (at.x as f64, at.y as f64));
    (x, display::flip_y(&monitors, y, 0.0))
}

/// Development (replay): input sent to GPUI's window the way Windows would.
pub mod synthetic {
    use asides_core::keys::Chord;
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        SendMessageW, WM_CHAR, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_RBUTTONDOWN,
        WM_RBUTTONUP,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Mouse {
        Move,
        Down { right: bool },
        Drag { right: bool },
        Up { right: bool },
    }

    /// A mouse event at `(x, y)` (window points, y down).
    pub fn mouse(window: usize, kind: Mouse, (x, y): (f64, f64), shift: bool, _alt: bool, cmd: bool) {
        let Some(handle) = super::hwnd(window) else { return };
        let s = super::scale(handle);
        let packed = (((y * s).round() as i32 as u16 as u32) << 16 | (x * s).round() as i32 as u16 as u32) as isize;
        // MK_LBUTTON, MK_RBUTTON, MK_SHIFT, MK_CONTROL.
        let mut keys = if shift { 0x4 } else { 0 } | if cmd { 0x8 } else { 0 };
        let message = match kind {
            Mouse::Move => WM_MOUSEMOVE,
            Mouse::Drag { right } => {
                keys |= if right { 0x2 } else { 0x1 };
                WM_MOUSEMOVE
            }
            Mouse::Down { right: false } => WM_LBUTTONDOWN,
            Mouse::Down { right: true } => WM_RBUTTONDOWN,
            Mouse::Up { right: false } => WM_LBUTTONUP,
            Mouse::Up { right: true } => WM_RBUTTONUP,
        };
        // SAFETY: a message GPUI handles on this thread.
        unsafe { SendMessageW(handle, message, Some(WPARAM(keys)), Some(LPARAM(packed))) };
    }

    /// The virtual key of a key, the character it types, and whether the layout needs Shift for it
    /// (`?` is Shift and `/` on a US keyboard).
    fn key_code(key: &str) -> Option<(u16, Option<char>, bool)> {
        Some(match key {
            "enter" | "\n" => (0x0d, Some('\r'), false),
            "tab" => (0x09, Some('\t'), false),
            "space" | " " => (0x20, Some(' '), false),
            "backspace" => (0x08, Some('\u{8}'), false),
            "escape" => (0x1b, Some('\u{1b}'), false),
            "left" => (0x25, None, false),
            "up" => (0x26, None, false),
            "right" => (0x27, None, false),
            "down" => (0x28, None, false),
            "f1" => (0x70, None, false),
            "home" => (0x24, None, false),
            "end" => (0x23, None, false),
            "delete" => (0x2e, None, false),
            k if k.chars().count() == 1 => {
                let c = k.chars().next()?;
                // SAFETY: a plain query of the keyboard layout.
                let scan = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW(c as u16) };
                match scan {
                    -1 => (0, Some(c), false),
                    scan => ((scan & 0xff) as u16, Some(c), scan >> 8 & 1 != 0),
                }
            }
            _ => return None,
        })
    }

    /// The modifiers held, as GPUI reads them (the thread's keyboard state), for one message.
    fn holding<R>(shift: bool, ctrl: bool, alt: bool, send: impl FnOnce() -> R) -> R {
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardState, SetKeyboardState};
        let mut before = [0u8; 256];
        // SAFETY: the thread's own keyboard state, read and put back.
        unsafe {
            let _ = GetKeyboardState(&mut before);
            let mut state = before;
            for (on, keys) in [(shift, [0x10, 0xa0]), (ctrl, [0x11, 0xa2]), (alt, [0x12, 0xa4])] {
                for key in keys {
                    state[key] = if on { 0x80 } else { 0 };
                }
            }
            let _ = SetKeyboardState(&state);
            let sent = send();
            let _ = SetKeyboardState(&before);
            sent
        }
    }

    /// A key going down or up (`chord` like `shift+h`; ⌘ is Ctrl, as Windows' keys are); `held`, a
    /// repeat of it going down.
    pub fn key(window: usize, chord: &Chord, down: bool, held: bool) {
        let Some(handle) = super::hwnd(window) else { return };
        let Some((vk, typed, layout_shift)) = key_code(&chord.key) else { return };
        let repeat: isize = if held { 1 << 30 } else { 0 };
        let (shift, ctrl) = (chord.shift || layout_shift, chord.ctrl || chord.cmd);
        // SAFETY: messages GPUI handles on this thread.
        holding(shift, ctrl, chord.alt, || unsafe {
            SendMessageW(
                handle,
                if down { WM_KEYDOWN } else { WM_KEYUP },
                Some(WPARAM(vk as usize)),
                Some(LPARAM(repeat | 1)),
            );
            if down && let Some(c) = typed.filter(|_| !ctrl) {
                let c = if chord.shift { c.to_ascii_uppercase() } else { c };
                SendMessageW(handle, WM_CHAR, Some(WPARAM(c as usize)), Some(LPARAM(1)));
            }
        });
    }

    /// Typing `text`, a key at a time.
    pub fn text(window: usize, text: &str) {
        let Some(handle) = super::hwnd(window) else { return };
        for c in text.chars() {
            // SAFETY: a message GPUI handles on this thread.
            unsafe { SendMessageW(handle, WM_CHAR, Some(WPARAM(c as usize)), Some(LPARAM(1))) };
        }
    }
}
