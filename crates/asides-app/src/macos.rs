//! Small AppKit/AVFoundation helpers: sounds, the microphone and Automation permissions, apps in
//! front, the command link, opening at login.

use std::cell::RefCell;
use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::c_void;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use std::ptr::NonNull;

use asides_core::state::LoginItem;
use block2::RcBlock;
use objc2::AllocAnyThread;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_app_kit::{
    NSApplicationActivationOptions, NSRunningApplication, NSSound, NSWorkspace, NSWorkspaceApplicationKey,
    NSWorkspaceDidActivateApplicationNotification, NSWorkspaceDidDeactivateApplicationNotification,
    NSWorkspaceDidLaunchApplicationNotification,
};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_foundation::{NSData, NSDistributedNotificationCenter, NSNotification, NSString};
use objc2_service_management::{SMAppService, SMAppServiceStatus};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mic {
    Granted,
    Denied,
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

pub fn mic_status() -> Mic {
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else { return Mic::Undetermined };
    match unsafe { AVCaptureDevice::authorizationStatusForMediaType(audio) } {
        AVAuthorizationStatus::Authorized => Mic::Granted,
        AVAuthorizationStatus::NotDetermined => Mic::Undetermined,
        _ => Mic::Denied,
    }
}

/// Shows the system microphone prompt; `done` runs on an arbitrary thread with the answer.
pub fn request_mic(done: impl Fn(bool) + 'static) {
    let Some(audio) = (unsafe { AVMediaTypeAudio }) else { return };
    let block = RcBlock::new(move |granted: Bool| done(granted.as_bool()));
    unsafe { AVCaptureDevice::requestAccessForMediaType_completionHandler(audio, &block) };
}

/// Whether asides may send Apple events to an app (Automation: how it types into Terminal and
/// Ghostty).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Automation {
    Granted,
    Denied,
    Undetermined,
    /// macOS tells only for an app that runs.
    NotRunning,
}

impl Automation {
    pub fn as_str(self) -> &'static str {
        match self {
            Automation::Granted => "granted",
            Automation::Denied => "denied",
            Automation::Undetermined => "undetermined",
            Automation::NotRunning => "not running",
        }
    }
}

#[repr(C)]
struct AEDesc {
    descriptor_type: u32,
    data_handle: *mut c_void,
}

#[link(name = "CoreServices", kind = "framework")]
unsafe extern "C" {
    fn AECreateDesc(type_code: u32, data: *const c_void, size: isize, result: *mut AEDesc) -> i16;
    fn AEDisposeDesc(desc: *mut AEDesc) -> i16;
    fn AEDeterminePermissionToAutomateTarget(target: *const AEDesc, class: u32, id: u32, ask: u8) -> i32;
}

/// `typeApplicationBundleID`, and `typeWildCard`: any event.
const TYPE_BUNDLE_ID: u32 = u32::from_be_bytes(*b"bund");
const WILDCARD: u32 = u32::from_be_bytes(*b"****");

/// Whether asides may automate the app `bundle` (an ID). With `ask`, macOS asks the user if they
/// haven't answered: it blocks until they do (off the main thread).
pub fn automation(bundle: &str, ask: bool) -> Automation {
    let mut desc = AEDesc { descriptor_type: 0, data_handle: std::ptr::null_mut() };
    // SAFETY: the bytes outlive the call, which copies them into the descriptor; it's disposed of.
    let status = unsafe {
        if AECreateDesc(TYPE_BUNDLE_ID, bundle.as_ptr().cast(), bundle.len() as isize, &mut desc) != 0 {
            return Automation::NotRunning;
        }
        let status = AEDeterminePermissionToAutomateTarget(&desc, WILDCARD, WILDCARD, u8::from(ask));
        AEDisposeDesc(&mut desc);
        status
    };
    match status {
        0 => Automation::Granted,
        // errAEEventNotPermitted
        -1743 => Automation::Denied,
        // errAEEventWouldRequireUserConsent
        -1744 => Automation::Undetermined,
        // procNotFound, and anything else macOS can't tell
        _ => Automation::NotRunning,
    }
}

/// Asks for Automation of the app `bundle`: started in the background first if it isn't running
/// (macOS asks only about an app that runs; it says when one has started). Blocks until the user
/// answers: call it off the main thread.
pub fn ask_automation(bundle: &str) -> Automation {
    if automation(bundle, false) == Automation::NotRunning {
        let (launched, launches) = std::sync::mpsc::channel();
        let block = RcBlock::new(move |_: NonNull<NSNotification>| {
            let _ = launched.send(());
        });
        let center = NSWorkspace::sharedWorkspace().notificationCenter();
        let name = unsafe { NSWorkspaceDidLaunchApplicationNotification };
        let observer = unsafe { center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block) };
        let _ = Command::new("/usr/bin/open").args(["-g", "-j", "-b", bundle]).status();
        let deadline = Instant::now() + Duration::from_secs(10);
        while automation(bundle, false) == Automation::NotRunning {
            let Some(left) = deadline.checked_duration_since(Instant::now()) else { break };
            if launches.recv_timeout(left).is_err() {
                break;
            }
        }
        unsafe { center.removeObserver(observer.as_ref()) };
    }
    automation(bundle, true)
}

/// Whether asides opens at login (macOS's Login Items: this copy, where it is).
pub fn login_item() -> LoginItem {
    match unsafe { SMAppService::mainAppService().status() } {
        SMAppServiceStatus::Enabled => LoginItem::On,
        SMAppServiceStatus::RequiresApproval => LoginItem::NeedsApproval,
        _ => LoginItem::Off,
    }
}

/// Opens asides at login, or stops; macOS notifies the user and lists it in System Settings →
/// General → Login Items. Turned off there, it stays so (`NeedsApproval`): only there can it be
/// turned on again. Returns how it is now.
pub fn set_open_at_login(open: bool) -> Result<LoginItem, String> {
    let service = unsafe { SMAppService::mainAppService() };
    // Registering twice, or unregistering what isn't, is an error.
    let done = match (open, login_item()) {
        (true, LoginItem::Off | LoginItem::Unknown) => unsafe { service.registerAndReturnError() },
        (false, LoginItem::On | LoginItem::NeedsApproval) => unsafe { service.unregisterAndReturnError() },
        _ => Ok(()),
    };
    done.map(|()| login_item()).map_err(|err| err.localizedDescription().to_string())
}

/// System Settings → General → Login Items.
pub fn open_login_items() {
    unsafe { SMAppService::openSystemSettingsLoginItems() };
}

/// Calls `changed` when what macOS lets asides do may have changed: an app came to the front (back
/// from System Settings, or a permission prompt answered), or Accessibility was turned on or off
/// for an app. macOS tells nothing more: Screen Recording and Automation change in System
/// Settings, which the user leaves.
pub fn on_permission_changes(changed: impl Fn() + 'static) {
    let block = RcBlock::new(move |_: NonNull<NSNotification>| changed());
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let activated = unsafe { NSWorkspaceDidActivateApplicationNotification };
    std::mem::forget(unsafe {
        workspace.addObserverForName_object_queue_usingBlock(Some(activated), None, None, &block)
    });
    let distributed = NSDistributedNotificationCenter::defaultCenter();
    let accessibility = NSString::from_str("com.apple.accessibility.api");
    std::mem::forget(unsafe {
        distributed.addObserverForName_object_queue_usingBlock(Some(&accessibility), None, None, &block)
    });
}

/// Calls `left` with the bundle ID of each app that stops being the one in front.
pub fn on_app_left(left: impl Fn(&str) + 'static) {
    let block = RcBlock::new(move |note: NonNull<NSNotification>| {
        let info = unsafe { note.as_ref() }.userInfo();
        let app = info.and_then(|info| info.objectForKey(unsafe { NSWorkspaceApplicationKey }));
        let bundle = app.as_deref().and_then(|a| a.downcast_ref::<NSRunningApplication>()?.bundleIdentifier());
        if let Some(bundle) = bundle {
            left(&bundle.to_string());
        }
    });
    let workspace = NSWorkspace::sharedWorkspace().notificationCenter();
    let deactivated = unsafe { NSWorkspaceDidDeactivateApplicationNotification };
    std::mem::forget(unsafe {
        workspace.addObserverForName_object_queue_usingBlock(Some(deactivated), None, None, &block)
    });
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
            Sound::MicOn => include_bytes!("../sounds/mic-on.wav"),
            Sound::MicOff => include_bytes!("../sounds/mic-off.wav"),
            Sound::Sent => include_bytes!("../sounds/sent.wav"),
            Sound::Closed => include_bytes!("../sounds/closed.wav"),
            Sound::Error => include_bytes!("../sounds/error.wav"),
            Sound::Dictation => include_bytes!("../sounds/dictation.wav"),
        }
    }

    fn load(self) -> Option<Retained<NSSound>> {
        NSSound::initWithData(NSSound::alloc(), &NSData::with_bytes(self.wav()))
    }
}

thread_local! {
    /// Each sound, loaded the first time it plays (on the main thread, as the rest of AppKit).
    static SOUNDS: RefCell<HashMap<Sound, Retained<NSSound>>> = RefCell::default();
}

pub fn play(sound: Sound) {
    SOUNDS.with_borrow_mut(|loaded| {
        let playing = match loaded.entry(sound) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => match sound.load() {
                Some(ready) => entry.insert(ready),
                None => return,
            },
        };
        playing.stop();
        playing.play();
    });
}

/// How putting the `asides` command on the PATH went.
pub enum CommandLink {
    Linked,
    /// The user cancelled the administrator prompt.
    Declined,
    Failed(String),
}

/// Links `cli` as `asides_core::install::LINK`. /usr/local/bin usually belongs to root, so unless the
/// user can write to it, macOS asks for an administrator's password. Blocks while the prompt shows.
pub fn link_command(cli: &Path) -> CommandLink {
    let link = Path::new(asides_core::install::LINK);
    let direct = link.parent().is_some_and(|dir| std::fs::create_dir_all(dir).is_ok())
        && std::os::unix::fs::symlink(cli, link).is_ok();
    if direct {
        return CommandLink::Linked;
    }
    let script = format!(
        "on run argv\n\
         do shell script \"/bin/mkdir -p /usr/local/bin && /bin/ln -sfn \" & quoted form of (item 1 of argv) & \" {}\" \
         with prompt \"asides wants to add the asides command to /usr/local/bin, so you can run it in any terminal.\" \
         with administrator privileges\n\
         end run",
        link.display()
    );
    match Command::new("/usr/bin/osascript").arg("-e").arg(script).arg(cli).output() {
        Ok(out) if out.status.success() => CommandLink::Linked,
        // errAEEventUserCanceled
        Ok(out) if String::from_utf8_lossy(&out.stderr).contains("-128") => CommandLink::Declined,
        Ok(out) => CommandLink::Failed(String::from_utf8_lossy(&out.stderr).trim().to_string()),
        Err(err) => CommandLink::Failed(err.to_string()),
    }
}

/// The app in front (its process id), to give the focus back to it later.
pub fn frontmost_app() -> Option<i32> {
    NSWorkspace::sharedWorkspace().frontmostApplication().map(|app| app.processIdentifier())
}

/// The name of the app with this process id, for the log.
pub fn app_name(pid: i32) -> String {
    NSRunningApplication::runningApplicationWithProcessIdentifier(pid)
        .and_then(|app| app.localizedName())
        .map_or_else(|| format!("process {pid}"), |name| name.to_string())
}

/// `26.5.2 (25F84)`, for the log.
pub fn system_version() -> String {
    let version = objc2_foundation::NSProcessInfo::processInfo().operatingSystemVersionString().to_string();
    version.trim_start_matches("Version ").replace("Build ", "")
}

/// Tells a watchdog (`asides_core::watchdog`) what the main thread does, from its run loop
/// (menus run loops of their own on it): its slow and stuck spells go in the log. It isn't ended
/// when stuck: only the menu bar waits on it, and it can be quit.
pub fn watch_main_thread() {
    use asides_core::watchdog::{Limits, Watchdog};
    use objc2_core_foundation::{CFRunLoop, CFRunLoopActivity, CFRunLoopObserver, kCFRunLoopCommonModes};
    let limits = Limits { slow: Duration::from_millis(250), stuck: Duration::from_secs(2), give_up: None };
    let watchdog = match Watchdog::start(limits, |note| note.log()) {
        Ok(watchdog) => watchdog,
        Err(err) => return eprintln!("asides: no watchdog: {err}"),
    };
    let block = RcBlock::new(move |_: *mut CFRunLoopObserver, activity: CFRunLoopActivity| {
        match activity.contains(CFRunLoopActivity::BeforeWaiting) {
            true => watchdog.waiting(),
            false => watchdog.working(),
        }
    });
    // SAFETY: an allocator and a block are given; the observer copies the block.
    let observer =
        unsafe { CFRunLoopObserver::with_handler(None, CFRunLoopActivity::AllActivities.0, true, 0, Some(&block)) };
    if let (Some(observer), Some(main)) = (observer, CFRunLoop::main()) {
        // SAFETY: a constant CoreFoundation defines.
        main.add_observer(Some(&observer), unsafe { kCFRunLoopCommonModes });
    }
}

/// Brings the app with this process id to the front again.
pub fn activate_app(pid: i32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sounds_load_and_are_short() {
        for sound in [Sound::MicOn, Sound::MicOff, Sound::Sent, Sound::Closed, Sound::Error, Sound::Dictation] {
            let loaded = sound.load().unwrap_or_else(|| panic!("{sound:?} doesn't load"));
            assert!(loaded.duration() <= 1.0, "{sound:?} lasts {} s: they play many times a day", loaded.duration());
        }
    }
}
