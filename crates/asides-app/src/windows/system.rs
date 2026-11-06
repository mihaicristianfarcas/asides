//! What asides needs of Windows itself, as `macos.rs` is for macOS: the microphone's privacy
//! setting, sounds, the app in front and leaving it, opening at login (the user's Run key, which
//! Settings → Apps → Startup lists), the `asides` command on the user's PATH, the system's version
//! for the log, and the main thread's watchdog.
//!
//! Windows asks no permission to see the screen, name what's under a pin (UI Automation) or type
//! into a console: the microphone is the one privacy setting asides depends on, and Windows asks
//! desktop apps nothing about it; it's on or off in Settings → Privacy & security → Microphone.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::Duration;

use asides_core::state::LoginItem;
use asides_core::watchdog::{Limits, Watchdog};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_NOTIFY, REG_EXPAND_SZ, REG_NOTIFY_CHANGE_LAST_SET, REG_SZ,
    RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteKeyValueW, RegGetValueW,
    RegNotifyChangeKeyValue, RegOpenKeyExW, RegSetKeyValueW,
};
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    WaitForSingleObject,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, EnumWindows, GW_OWNER, GetForegroundWindow, GetWindow, GetWindowThreadProcessId,
    HWND_BROADCAST, IsIconic, IsWindowVisible, SMTO_ABORTIFHUNG, SW_RESTORE, SW_SHOWNORMAL, SendMessageTimeoutW,
    SetForegroundWindow, ShowWindow, WINEVENT_OUTOFCONTEXT, WM_SETTINGCHANGE,
};
use windows::core::{BOOL, HSTRING, PCWSTR, PWSTR, w};

/// The proof of being on the main thread that macOS's calls take; Windows' need none.
#[derive(Clone, Copy)]
pub struct MainThread;

impl MainThread {
    pub fn new() -> Option<MainThread> {
        Some(MainThread)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mic {
    Granted,
    Denied,
    #[expect(dead_code, reason = "macOS's: Windows asks nothing before a desktop app hears the microphone")]
    Undetermined,
}

impl Mic {
    pub fn as_str(self) -> &'static str {
        match self {
            Mic::Granted => "granted",
            Mic::Denied => "denied",
            Mic::Undetermined => "undetermined",
        }
    }
}

/// Windows' microphone privacy settings: for the device, for apps, and for desktop apps (where
/// asides is). Off in any of them, asides hears nothing.
const MICROPHONE: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore\microphone";

/// What asides says when the microphone is off for it.
pub const MIC_DENIED: &str = "The microphone is off for asides: turn on Settings → Privacy & security → Microphone → Let desktop apps access \
     your microphone.";

pub fn mic_status() -> Mic {
    let desktop = format!(r"{MICROPHONE}\NonPackaged");
    let settings = [(HKEY_LOCAL_MACHINE, MICROPHONE), (HKEY_CURRENT_USER, MICROPHONE), (HKEY_CURRENT_USER, &desktop)];
    let denied = settings.iter().any(|(root, key)| registry_string(*root, key, "Value").as_deref() == Some("Deny"));
    if denied { Mic::Denied } else { Mic::Granted }
}

/// Windows has no prompt for a desktop app's microphone: off, its settings open, and `done` is
/// told how it is now.
pub fn request_mic(done: impl Fn(bool) + 'static) {
    let status = mic_status();
    if status == Mic::Denied {
        open("ms-settings:privacy-microphone");
    }
    done(status == Mic::Granted);
}

/// Whether asides may send Apple events to an app: macOS's, which Windows has nothing like.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Automation {
    Granted,
}

impl Automation {
    pub fn as_str(self) -> &'static str {
        "granted"
    }
}

pub fn automation(_bundle: &str, _ask: bool) -> Automation {
    Automation::Granted
}

pub fn ask_automation(_bundle: &str) -> Automation {
    Automation::Granted
}

/// The user's Run key: what Windows starts as they sign in; Settings → Apps → Startup turns each
/// on or off (in `StartupApproved`, which only Settings and Task Manager change).
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const STARTUP_APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const RUN_NAME: &str = "asides";

/// How asides is started at login: this copy, quoted.
fn run_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

/// Whether asides opens at login: this copy is in the Run key, and Settings hasn't turned it off.
pub fn login_item() -> LoginItem {
    let Some(command) = registry_string(HKEY_CURRENT_USER, RUN, RUN_NAME) else { return LoginItem::Off };
    if Some(command) != run_command() {
        return LoginItem::Off;
    }
    // A disabled entry's first byte is odd (3); an enabled one's even (2), or it isn't there.
    match registry_bytes(HKEY_CURRENT_USER, STARTUP_APPROVED, RUN_NAME).and_then(|b| b.first().copied()) {
        Some(flag) if flag & 1 == 1 => LoginItem::NeedsApproval,
        _ => LoginItem::On,
    }
}

/// Opens asides at login, or stops. Turned off in Settings, it stays so (`NeedsApproval`): only
/// there can it be turned on again. Returns how it is now.
pub fn set_open_at_login(open: bool) -> Result<LoginItem, String> {
    let done = match open {
        true => {
            let command = run_command().ok_or("asides doesn't know where it is")?;
            set_registry_string(HKEY_CURRENT_USER, RUN, RUN_NAME, &command, false)
        }
        false => {
            // SAFETY: plain strings, nul-terminated by `HSTRING`.
            let deleted =
                unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(RUN), &HSTRING::from(RUN_NAME)) };
            match deleted.is_ok() || deleted.0 == 2 {
                true => Ok(()),
                false => Err(format!("Windows said {}", deleted.0)),
            }
        }
    };
    done.map(|()| login_item())
}

/// Settings → Apps → Startup.
pub fn open_login_items() {
    open("ms-settings:startupapps");
}

/// Calls `changed` when what Windows lets asides do may have changed: the microphone's privacy
/// setting, as Windows says (a registry key's notification), on a thread of its own.
pub fn on_permission_changes(changed: impl Fn() + Send + 'static) {
    let spawned = std::thread::Builder::new().name("asides-privacy".into()).spawn(move || {
        let mut key = HKEY::default();
        // SAFETY: the key and the event are this thread's, for as long as it runs.
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, &HSTRING::from(MICROPHONE), None, KEY_NOTIFY, &mut key).is_err() {
                return;
            }
            let Ok(event) = CreateEventW(None, false, false, None) else { return };
            loop {
                if RegNotifyChangeKeyValue(key, true, REG_NOTIFY_CHANGE_LAST_SET, Some(event), true).is_err() {
                    break;
                }
                WaitForSingleObject(event, u32::MAX);
                changed();
            }
            let _ = CloseHandle(event);
            let _ = RegCloseKey(key);
        }
    });
    if let Err(err) = spawned {
        eprintln!("asides: not watching the microphone's setting: {err}");
    }
}

/// What `on_app_left` calls, and the app in front last (its program's file name).
type Left = (Box<dyn Fn(&str)>, String);

thread_local! {
    static LEFT: RefCell<Option<Left>> = const { RefCell::new(None) };
}

/// Calls `left` with the program (`WindowsTerminal.exe`) of each app that stops being the one in
/// front. Windows says so on the main thread, as its messages are handled.
pub fn on_app_left(left: impl Fn(&str) + 'static) {
    let front = front_program().unwrap_or_default();
    LEFT.with_borrow_mut(|slot| *slot = Some((Box::new(left), front)));
    unsafe extern "system" fn foreground(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
        let now = front_program().unwrap_or_default();
        LEFT.with_borrow_mut(|slot| {
            let Some((left, before)) = slot.as_mut() else { return };
            if *before != now {
                let gone = std::mem::replace(before, now);
                left(&gone);
            }
        });
    }
    // SAFETY: the callback is a plain function, called on this thread while it handles messages.
    let hook = unsafe {
        SetWinEventHook(
            EVENT_SYSTEM_FOREGROUND,
            EVENT_SYSTEM_FOREGROUND,
            None,
            Some(foreground),
            0,
            0,
            WINEVENT_OUTOFCONTEXT,
        )
    };
    if hook.is_invalid() {
        eprintln!("asides: not told which app is in front");
    }
}

/// The programs that show the consoles asides types into: leaving one, the user may have started
/// a session there.
pub fn is_terminal(program: &str) -> bool {
    const TERMINALS: &[&str] =
        &["WindowsTerminal.exe", "conhost.exe", "OpenConsole.exe", "Code.exe", "wezterm-gui.exe"];
    TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(program))
}

/// The taskbar is light (Settings → Personalization → Colors: Windows mode).
pub fn light_taskbar() -> bool {
    let key = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    registry_value(HKEY_CURRENT_USER, key, "SystemUsesLightTheme", windows::Win32::System::Registry::RRF_RT_REG_DWORD.0)
        .is_some_and(|v| v.first().is_some_and(|&b| b != 0))
}

/// The file name of the program in front.
fn front_program() -> Option<String> {
    let path = process_path(frontmost_app()?)?;
    Some(path.file_name()?.to_string_lossy().into_owned())
}

/// asides' sounds (`sounds/`, made by `scripts/make-sounds.py`), in the binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    MicOn,
    MicOff,
    Sent,
    /// Closed without sending what was said.
    Closed,
    Error,
    /// A Wispr Flow dictation joined the aside.
    Dictation,
}

impl Sound {
    fn wav(self) -> &'static [u8] {
        match self {
            Sound::MicOn => include_bytes!("../../sounds/mic-on.wav"),
            Sound::MicOff => include_bytes!("../../sounds/mic-off.wav"),
            Sound::Sent => include_bytes!("../../sounds/sent.wav"),
            Sound::Closed => include_bytes!("../../sounds/closed.wav"),
            Sound::Error => include_bytes!("../../sounds/error.wav"),
            Sound::Dictation => include_bytes!("../../sounds/dictation.wav"),
        }
    }
}

/// Plays a sound, stopping the one playing.
pub fn play(sound: Sound) {
    // SAFETY: the sound's bytes are static: they outlive its playing, as `SND_ASYNC` needs.
    let _ = unsafe { PlaySoundW(PCWSTR(sound.wav().as_ptr().cast()), None, SND_MEMORY | SND_ASYNC | SND_NODEFAULT) };
}

/// Makes `asides://` links open this copy (the user's classes: no administrator needed). The
/// command and the window send their links this way (`instance.rs` hands them to the copy running).
pub fn register_links() {
    let Some(command) = run_command().map(|exe| format!("{exe} \"%1\"")) else { return };
    let root = r"Software\Classes\asides";
    let set = [
        (root, "", "URL:asides"),
        (root, "URL Protocol", ""),
        (r"Software\Classes\asides\shell\open\command", "", command.as_str()),
    ];
    for (key, name, value) in set {
        if registry_string(HKEY_CURRENT_USER, key, name).as_deref() != Some(value)
            && let Err(err) = set_registry_string(HKEY_CURRENT_USER, key, name, value, false)
        {
            eprintln!("asides: asides:// links won't open it: {err}");
        }
    }
}

/// Lets the overlay (process `pid`) take the keyboard: Windows lets only the app the user last
/// gave input to (here, through its shortcut) bring a window to the front, and hand that on.
pub fn allow_focus(pid: i32) {
    // SAFETY: a plain call.
    let _ = unsafe { windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(pid as u32) };
}

/// How putting the `asides` command on the PATH went.
pub enum CommandLink {
    Linked,
    /// The user declined (macOS's administrator prompt; Windows asks nothing).
    #[allow(dead_code)]
    Declined,
    Failed(String),
}

/// The folder to put on the user's PATH for the `asides` command, if it isn't there yet: the
/// app's own, which has `asides.exe`.
pub fn command_to_link() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.to_path_buf();
    if !dir.join("asides.exe").is_file() || on_user_path(&dir) {
        return None;
    }
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found = std::env::split_paths(&path).any(|d| d.join("asides.exe").is_file());
    (!found).then_some(dir)
}

fn on_user_path(dir: &Path) -> bool {
    let path = registry_expandable(HKEY_CURRENT_USER, "Environment", "Path").unwrap_or_default();
    let dir = dir.to_string_lossy().trim_end_matches('\\').to_lowercase();
    path.split(';').any(|d| d.trim().trim_end_matches('\\').to_lowercase() == dir)
}

/// Adds `dir` to the user's PATH (`HKCU\Environment`), and tells running programs (Explorer, so
/// the terminals it starts next have it). No administrator needed.
pub fn link_command(dir: &Path) -> CommandLink {
    if on_user_path(dir) {
        return CommandLink::Linked;
    }
    let path = registry_expandable(HKEY_CURRENT_USER, "Environment", "Path").unwrap_or_default();
    let joined = match path.trim_end_matches(';') {
        "" => dir.display().to_string(),
        before => format!("{before};{}", dir.display()),
    };
    if let Err(err) = set_registry_string(HKEY_CURRENT_USER, "Environment", "Path", &joined, true) {
        return CommandLink::Failed(err);
    }
    // SAFETY: a broadcast with a static string; hung windows are skipped.
    unsafe {
        SendMessageTimeoutW(
            HWND_BROADCAST,
            WM_SETTINGCHANGE,
            WPARAM(0),
            LPARAM(w!("Environment").as_ptr() as isize),
            SMTO_ABORTIFHUNG,
            5000,
            None,
        )
    };
    CommandLink::Linked
}

/// The app in front (its process id), to give the focus back to it later.
pub fn frontmost_app() -> Option<i32> {
    // SAFETY: plain calls.
    let window = unsafe { GetForegroundWindow() };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
    (pid != 0).then_some(pid as i32)
}

/// A process's executable.
pub fn process_path(pid: i32) -> Option<PathBuf> {
    // SAFETY: the handle is closed below; Windows writes at most `len` characters.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid as u32).ok()?;
        let mut path = vec![0u16; 1024];
        let mut len = path.len() as u32;
        let read = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, PWSTR(path.as_mut_ptr()), &mut len);
        let _ = CloseHandle(process);
        read.ok()?;
        Some(PathBuf::from(String::from_utf16_lossy(&path[..len as usize])))
    }
}

/// The name of the app with this process id, for the log.
pub fn app_name(pid: i32) -> String {
    process_path(pid)
        .and_then(|p| Some(p.file_stem()?.to_string_lossy().into_owned()))
        .unwrap_or_else(|| format!("process {pid}"))
}

/// `Windows 11 24H2 (26100.4061)`, for the log.
pub fn system_version() -> String {
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let build = registry_string(HKEY_LOCAL_MACHINE, key, "CurrentBuild").unwrap_or_default();
    let release = registry_string(HKEY_LOCAL_MACHINE, key, "DisplayVersion").unwrap_or_default();
    // The product name still says 10 on Windows 11: its builds start at 22000.
    let windows = if build.parse::<u32>().is_ok_and(|b| b >= 22000) { "11" } else { "10" };
    format!("Windows {windows} {release} ({build})")
}

/// Tells a watchdog (`asides_core::watchdog`) what the main thread does: at work on each event the
/// event loop hands it, waiting once they're handled. Slow and stuck spells go in the log.
pub fn watch_main_thread() -> Option<Watchdog> {
    let limits = Limits { slow: Duration::from_millis(250), stuck: Duration::from_secs(2), give_up: None };
    match Watchdog::start(limits, |note| note.log()) {
        Ok(watchdog) => Some(watchdog),
        Err(err) => {
            eprintln!("asides: no watchdog: {err}");
            None
        }
    }
}

/// The event loop is at work on an event...
pub fn main_thread_working<T>(watchdog: &Option<Watchdog>, _: &tao::event::Event<'_, T>) {
    if let Some(watchdog) = watchdog {
        watchdog.working();
    }
}

/// ...and done with it: between events, the loop waits (or handles Windows' own messages).
pub fn main_thread_done(watchdog: &Option<Watchdog>) {
    if let Some(watchdog) = watchdog {
        watchdog.waiting();
    }
}

/// Brings the app with this process id to the front again: its main window.
pub fn activate_app(pid: i32) {
    struct Found {
        pid: u32,
        window: Option<HWND>,
    }
    unsafe extern "system" fn each(window: HWND, found: LPARAM) -> BOOL {
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
            if IsIconic(window).as_bool() {
                let _ = ShowWindow(window, SW_RESTORE);
            }
            let _ = SetForegroundWindow(window);
        }
    }
}

/// asides' entry in Settings > Apps, as the installer names it: its `AppId` with Inno Setup's
/// `_is1` (`packaging/windows/asides.iss`).
const INSTALLED_ENTRY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\io.github.mihaicristianfarcas.asides_is1";

/// The installer put asides in `dir` (its entry in Settings > Apps says so): a copy there is the
/// one an update replaces.
pub fn installed_in(dir: &Path) -> bool {
    let normal = |p: &str| p.trim_end_matches('\\').to_lowercase();
    registry_string(HKEY_CURRENT_USER, INSTALLED_ENTRY, "InstallLocation")
        .is_some_and(|found| normal(&found) == normal(&dir.to_string_lossy()))
}

/// Windows' build (`26100`), what releases name the oldest they run on by (`10.0.19041`).
pub fn build_number() -> Option<u32> {
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    registry_string(HKEY_LOCAL_MACHINE, key, "CurrentBuild")?.trim().parse().ok()
}

/// Opens a link (a Settings page) with what Windows opens it with.
pub fn open(link: &str) {
    // SAFETY: plain strings, nul-terminated by `HSTRING`.
    unsafe { ShellExecuteW(None, w!("open"), &HSTRING::from(link), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL) };
}

// --- The registry ---

fn registry_value(root: HKEY, key: &str, name: &str, flags: u32) -> Option<Vec<u8>> {
    let (key, name) = (HSTRING::from(key), HSTRING::from(name));
    let flags = windows::Win32::System::Registry::REG_ROUTINE_FLAGS(flags);
    let mut len = 0u32;
    // SAFETY: asked for its size first, then read into a buffer that size.
    unsafe {
        RegGetValueW(root, &key, &name, flags, None, None, Some(&mut len)).ok().ok()?;
        let mut data = vec![0u8; len as usize];
        RegGetValueW(root, &key, &name, flags, None, Some(data.as_mut_ptr().cast()), Some(&mut len)).ok().ok()?;
        data.truncate(len as usize);
        Some(data)
    }
}

fn wide_text(data: &[u8]) -> String {
    let wide: Vec<u16> = data.as_chunks::<2>().0.iter().map(|&c| u16::from_le_bytes(c)).collect();
    String::from_utf16_lossy(&wide).trim_end_matches('\0').to_string()
}

pub fn registry_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    registry_value(root, key, name, RRF_RT_REG_SZ.0).map(|d| wide_text(&d))
}

/// A `REG_EXPAND_SZ` (or plain) string as written, its `%VARIABLES%` kept.
fn registry_expandable(root: HKEY, key: &str, name: &str) -> Option<String> {
    registry_value(root, key, name, RRF_RT_REG_SZ.0 | RRF_RT_REG_EXPAND_SZ.0 | RRF_NOEXPAND.0).map(|d| wide_text(&d))
}

fn registry_bytes(root: HKEY, key: &str, name: &str) -> Option<Vec<u8>> {
    registry_value(root, key, name, windows::Win32::System::Registry::RRF_RT_REG_BINARY.0)
}

fn set_registry_string(root: HKEY, key: &str, name: &str, value: &str, expandable: bool) -> Result<(), String> {
    let data: Vec<u16> = value.encode_utf16().chain([0]).collect();
    let kind = if expandable { REG_EXPAND_SZ } else { REG_SZ };
    // SAFETY: the data is the value's characters and their nul, as many bytes as said.
    let set = unsafe {
        RegSetKeyValueW(
            root,
            &HSTRING::from(key),
            &HSTRING::from(name),
            kind.0,
            Some(data.as_ptr().cast()),
            (data.len() * 2) as u32,
        )
    };
    set.ok().map_err(|err| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_which_windows_it_is() {
        let version = system_version();
        assert!(version.starts_with("Windows 1"), "{version}");
    }

    #[test]
    fn knows_the_terminals() {
        assert!(is_terminal("WindowsTerminal.exe") && is_terminal("conhost.exe") && !is_terminal("explorer.exe"));
    }
}
