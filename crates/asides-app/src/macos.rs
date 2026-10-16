//! Small AppKit/AVFoundation helpers: sounds, microphone permission, Finder, the command link.

use std::path::{Path, PathBuf};
use std::process::Command;

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::runtime::Bool;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationOptions, NSModalResponse, NSOpenPanel, NSRunningApplication, NSSound,
    NSWorkspace,
};
use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};
use objc2_foundation::{NSString, NSURL};

const MODAL_RESPONSE_OK: NSModalResponse = 1;

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

/// Shows the system folder picker (starting at `start`) and returns the chosen directory.
pub fn choose_directory(mtm: MainThreadMarker, start: Option<&Path>) -> Option<PathBuf> {
    let app = NSApplication::sharedApplication(mtm);
    // A menu-bar app must come to the front for its panel to get keyboard focus.
    app.activate();
    let panel = NSOpenPanel::openPanel(mtm);
    panel.setCanChooseDirectories(true);
    panel.setCanChooseFiles(false);
    panel.setAllowsMultipleSelection(false);
    panel.setCanCreateDirectories(true);
    panel.setPrompt(Some(&NSString::from_str("Use This Directory")));
    panel.setMessage(Some(&NSString::from_str("Recordings will be appended to notes in this repo or directory.")));
    if let Some(start) = start {
        panel.setDirectoryURL(Some(&NSURL::fileURLWithPath(&NSString::from_str(&start.to_string_lossy()))));
    }
    let chosen = (panel.runModal() == MODAL_RESPONSE_OK)
        .then(|| panel.URL()?.path())
        .flatten()
        .map(|path| PathBuf::from(path.to_string()));
    // Hand focus back to the app you were in (typically the browser).
    app.hide(None);
    chosen
}

pub fn play(sound: &str) {
    if let Some(sound) = NSSound::soundNamed(&NSString::from_str(sound)) {
        sound.stop();
        sound.play();
    }
}

pub fn open(path: &Path) {
    let _ = Command::new("open").arg(path).spawn();
}

pub fn open_in_text_editor(path: &Path) {
    let _ = Command::new("open").arg("-t").arg(path).spawn();
}

pub fn reveal(path: &Path) {
    let _ = Command::new("open").arg("-R").arg(path).spawn();
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
         with prompt \"Asides wants to add the asides command to /usr/local/bin, so you can run it in any terminal.\" \
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

pub fn open_microphone_settings() {
    let _ =
        Command::new("open").arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone").spawn();
}

/// The app in front (its process id), to give the focus back to it later.
pub fn frontmost_app() -> Option<i32> {
    NSWorkspace::sharedWorkspace().frontmostApplication().map(|app| app.processIdentifier())
}

/// Brings the app with this process id to the front again.
pub fn activate_app(pid: i32) {
    if let Some(app) = NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        app.activateWithOptions(NSApplicationActivationOptions::empty());
    }
}
