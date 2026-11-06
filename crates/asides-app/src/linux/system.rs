//! What asides needs of Linux itself, as `macos.rs` is for macOS: sounds, the app in front and
//! leaving it, opening at login (the XDG autostart folder), the launcher entry and the `asides`
//! command when no package installed them, the system's version for the log, and the main thread's
//! watchdog.
//!
//! Linux asks no permission to hear the microphone. Typing into a terminal's session may need one
//! (`Automation` here, `asides_keys`): the kernel's ptrace rules decide.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use asides_core::paths::BUNDLE_ID;
use asides_core::state::LoginItem;
use asides_core::watchdog::{Limits, Watchdog};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _, EventMask, Window,
};
use x11rb::rust_connection::RustConnection;

use crate::capture::on_wayland;

/// The proof of being on the main thread that macOS's calls take; Linux's need none.
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
    /// macOS's: Linux has no setting for it.
    Denied,
    /// macOS's: Linux asks nothing before an app hears the microphone.
    #[expect(dead_code, reason = "macOS's: Linux asks nothing before an app hears the microphone")]
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

/// macOS's and Windows' message: nothing on Linux turns the microphone off for one app.
pub const MIC_DENIED: &str = "asides can't hear the microphone: check your desktop's sound settings.";

pub fn mic_status() -> Mic {
    Mic::Granted
}

pub fn request_mic(done: impl Fn(bool) + 'static) {
    done(true);
}

/// Whether asides may type into a terminal's session (`asides_keys::may_type`): the one thing on
/// Linux that may need allowing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Automation {
    Granted,
    Undetermined,
}

impl Automation {
    pub fn as_str(self) -> &'static str {
        match self {
            Automation::Granted => "granted",
            Automation::Undetermined => "undetermined",
        }
    }
}

pub fn automation(_bundle: &str, _ask: bool) -> Automation {
    match asides_core::sessions::linux::helper_may_type() {
        true => Automation::Granted,
        false => Automation::Undetermined,
    }
}

/// Asks the desktop's password prompt to let `asides-keys` read another process's pty
/// (`setcap cap_sys_ptrace+ep`). Waits for the answer.
pub fn ask_automation(bundle: &str) -> Automation {
    let helper = asides_core::sessions::linux::helper();
    let done = std::process::Command::new("pkexec").args(["setcap", "cap_sys_ptrace+ep"]).arg(&helper).status();
    match done {
        Ok(status) if status.success() => {}
        Ok(status) => eprintln!("asides: pkexec setcap on {} said {status}", helper.display()),
        Err(err) => eprintln!("asides: could not run pkexec: {err}"),
    }
    automation(bundle, false)
}

/// The XDG autostart entry's path.
fn autostart_file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let home = std::env::home_dir().unwrap_or_default();
    config.unwrap_or_else(|| home.join(".config")).join("autostart").join(format!("{BUNDLE_ID}.desktop"))
}

/// This copy's desktop entry: `extra` goes after the program on its `Exec` line.
fn desktop_entry(extra: &str, more: &str) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!(
        "[Desktop Entry]\nType=Application\nName=asides\nComment=Freeze the screen, click what's wrong, say why; it \
         goes to your coding agent\nExec={}{extra}\nIcon=asides\nTerminal=false\nCategories=Development;\n\
         X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2\n{more}",
        desktop_quote(&exe.to_string_lossy()),
    ))
}

/// A word on a desktop entry's `Exec` line.
fn desktop_quote(word: &str) -> String {
    if word.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        return word.to_string();
    }
    let mut quoted = String::from("\"");
    for c in word.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted.replace('%', "%%")
}

/// Whether asides opens at login: an autostart entry starts this copy.
pub fn login_item() -> LoginItem {
    let Ok(entry) = std::fs::read_to_string(autostart_file()) else { return LoginItem::Off };
    let hidden = entry.lines().any(|l| l.trim().eq_ignore_ascii_case("Hidden=true"));
    if hidden { LoginItem::Off } else { LoginItem::On }
}

/// Opens asides at login, or stops. Returns how it is now.
pub fn set_open_at_login(open: bool) -> Result<LoginItem, String> {
    let file = autostart_file();
    match open {
        true => {
            let entry = desktop_entry(" --background", "X-GNOME-Autostart-enabled=true\n")
                .ok_or("asides doesn't know where it is")?;
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
            }
            std::fs::write(&file, entry).map_err(|err| err.to_string())?;
        }
        false => match std::fs::remove_file(&file) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err.to_string()),
            _ => {}
        },
    }
    Ok(login_item())
}

/// macOS's: nothing here can turn it off behind asides' back.
pub fn open_login_items() {}

/// Calls `changed` when what asides may do changes: the helper's capability, as its file changes.
/// A password prompt's answer says so too; this hears a package or the user doing it themselves.
pub fn on_permission_changes(changed: impl Fn() + Send + 'static) {
    let helper = asides_core::sessions::linux::helper();
    static KEPT: OnceLock<Mutex<Option<asides_core::watch::Watch>>> = OnceLock::new();
    match asides_core::watch::file(&helper, changed) {
        Ok(watch) => *KEPT.get_or_init(Mutex::default).lock().unwrap() = Some(watch),
        Err(err) => eprintln!("asides: not watching {}: {err}", helper.display()),
    }
}

/// Calls `left` with the program (`gnome-terminal-server`) of each app that stops being the one in
/// front, as the window manager says. A Wayland desktop tells nobody; there, a new session is known
/// by the file its agent writes (`sessions::start_folders`).
pub fn on_app_left(left: impl Fn(&str) + Send + 'static) {
    if on_wayland() {
        return;
    }
    let spawned = std::thread::Builder::new().name("asides-front-app".into()).spawn(move || {
        let Ok((connection, screen)) = x11rb::connect(None) else { return };
        let Some(root) = connection.setup().roots.get(screen).map(|s| s.root) else { return };
        let Some(active) = atom(&connection, b"_NET_ACTIVE_WINDOW") else { return };
        let attributes = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        if connection.change_window_attributes(root, &attributes).is_err() || connection.flush().is_err() {
            return;
        }
        let mut before = front_on(&connection, root).and_then(program_of).unwrap_or_default();
        while let Ok(event) = connection.wait_for_event() {
            if let x11rb::protocol::Event::PropertyNotify(event) = event
                && event.atom == active
            {
                let now = front_on(&connection, root).and_then(program_of).unwrap_or_default();
                if now != before {
                    left(&std::mem::replace(&mut before, now));
                }
            }
        }
    });
    if let Err(err) = spawned {
        eprintln!("asides: not told which app is in front: {err}");
    }
}

/// The programs that show the terminals asides types into: leaving one, the user may have started a
/// session there.
pub fn is_terminal(program: &str) -> bool {
    const TERMINALS: &[&str] = &[
        "gnome-terminal-server",
        "gnome-terminal-",
        "ptyxis",
        "ptyxis-agent",
        "kgx",
        "konsole",
        "xterm",
        "alacritty",
        "kitty",
        "wezterm-gui",
        "foot",
        "footclient",
        "ghostty",
        "tilix",
        "terminator",
        "xfce4-terminal",
        "mate-terminal",
        "cosmic-term",
        "code",
    ];
    TERMINALS.iter().any(|t| t.eq_ignore_ascii_case(program))
}

/// The panel is light: Plasma's follows its color scheme, and its default is light. GNOME's, and
/// most others', are dark whatever the theme.
pub fn light_taskbar() -> bool {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !desktop.split(':').any(|d| d.eq_ignore_ascii_case("KDE")) {
        return false;
    }
    let home = std::env::home_dir().unwrap_or_default();
    let Ok(settings) = std::fs::read_to_string(home.join(".config/kdeglobals")) else { return true };
    settings.lines().find_map(|l| l.strip_prefix("ColorScheme=")).is_none_or(|scheme| !scheme.contains("Dark"))
}

fn atom(connection: &impl Connection, name: &[u8]) -> Option<Atom> {
    connection.intern_atom(false, name).ok()?.reply().ok().map(|r| r.atom)
}

/// A window's process, as it says (`_NET_WM_PID`).
pub fn window_pid(connection: &impl Connection, window: Window) -> Option<i32> {
    let name = atom(connection, b"_NET_WM_PID")?;
    let reply = connection.get_property(false, window, name, AtomEnum::CARDINAL, 0, 1).ok()?.reply().ok()?;
    reply.value32()?.next().map(|pid| pid as i32)
}

/// The process of the window in front, as the window manager says.
fn front_on(connection: &impl Connection, root: Window) -> Option<i32> {
    let active = atom(connection, b"_NET_ACTIVE_WINDOW")?;
    let reply = connection.get_property(false, root, active, AtomEnum::WINDOW, 0, 1).ok()?.reply().ok()?;
    let window = reply.value32()?.next().filter(|&w| w != 0)?;
    window_pid(connection, window)
}

/// A process's program, as `ps` names it.
fn program_of(pid: i32) -> Option<String> {
    Some(std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?.trim().to_string())
}

/// "Some app": a Wayland desktop doesn't say which is in front, or where its windows are. Where the
/// user is is looked for by accessibility (`elements::place`), and the desktop gives the keyboard
/// back by itself.
pub const FRONT_UNKNOWN: i32 = 0;

/// The app in front (its process id), to give the focus back to it later.
pub fn frontmost_app() -> Option<i32> {
    let found = asides_core::display::connect_x11().and_then(|(connection, screen)| {
        let root = connection.setup().roots.get(screen)?.root;
        front_on(&connection, root)
    });
    match found {
        Some(pid) => Some(pid),
        None if on_wayland() => Some(FRONT_UNKNOWN),
        None => None,
    }
}

/// The name of the app with this process id, for the log.
pub fn app_name(pid: i32) -> String {
    if pid == FRONT_UNKNOWN {
        return "an app".into();
    }
    program_of(pid).unwrap_or_else(|| format!("process {pid}"))
}

/// `Ubuntu 24.04.1 LTS, ubuntu:GNOME on wayland`, for the log.
pub fn system_version() -> String {
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    let name = release
        .lines()
        .find_map(|l| l.strip_prefix("PRETTY_NAME="))
        .map(|n| n.trim_matches('"').to_string())
        .unwrap_or_else(|| "Linux".into());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "no desktop".into());
    let session = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "an unknown session".into());
    format!("{name}, {desktop} on {session}")
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

/// A sound's samples: each channel's in turn, and how many there are of each a second.
fn decode(wav: &[u8]) -> Option<(Vec<f32>, u16, u32)> {
    let reader = hound::WavReader::new(std::io::Cursor::new(wav)).ok()?;
    let spec = reader.spec();
    let samples = match spec.sample_format {
        hound::SampleFormat::Float => reader.into_samples::<f32>().map_while(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let full = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader.into_samples::<i32>().map_while(Result::ok).map(|s| s as f32 / full).collect()
        }
    };
    Some((samples, spec.channels, spec.sample_rate))
}

/// A stream playing `sound`, and for how long. The device's own format: the sound is resampled to it
/// on the way, nearest sample being good enough for these short chimes.
fn start(sound: Sound) -> Option<(cpal::Stream, Duration)> {
    let (samples, channels, rate) = decode(sound.wav())?;
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    if config.sample_format() != cpal::SampleFormat::F32 && config.sample_format() != cpal::SampleFormat::I16 {
        return None;
    }
    let (out_channels, out_rate) = (config.channels() as usize, config.sample_rate() as f64);
    let frames = samples.len() / channels.max(1) as usize;
    let step = rate as f64 / out_rate;
    let channels = channels.max(1) as usize;
    let mut at = 0f64;
    let mut next = move || -> Vec<f32> {
        let frame = at as usize;
        at += step;
        (0..out_channels).map(|c| samples.get(frame * channels + c.min(channels - 1)).copied().unwrap_or(0.0)).collect()
    };
    let error = |err| eprintln!("asides: sound: {err}");
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device.build_output_stream(
            config.config(),
            move |out: &mut [f32], _: &cpal::OutputCallbackInfo| {
                for frame in out.chunks_mut(out_channels) {
                    frame.copy_from_slice(&next());
                }
            },
            error,
            None,
        ),
        _ => device.build_output_stream(
            config.config(),
            move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                for frame in out.chunks_mut(out_channels) {
                    for (o, s) in frame.iter_mut().zip(next()) {
                        *o = (s * i16::MAX as f32) as i16;
                    }
                }
            },
            error,
            None,
        ),
    }
    .ok()?;
    stream.play().ok()?;
    // A little more than the sound: the device's buffer plays out after the last sample's written.
    Some((stream, Duration::from_secs_f64(frames as f64 / rate as f64) + Duration::from_millis(200)))
}

/// Plays a sound, stopping the one playing, on a thread of its own: audio streams stay where they're
/// made.
pub fn play(sound: Sound) {
    static PLAYER: OnceLock<Option<Mutex<Sender<Sound>>>> = OnceLock::new();
    let player = PLAYER.get_or_init(|| {
        let (asked, sounds) = channel::<Sound>();
        std::thread::Builder::new()
            .name("asides-sounds".into())
            .spawn(move || {
                let mut next = sounds.recv().ok();
                while let Some(sound) = next.take() {
                    let Some((stream, length)) = start(sound) else {
                        next = sounds.recv().ok();
                        continue;
                    };
                    next = match sounds.recv_timeout(length) {
                        Ok(another) => Some(another),
                        Err(RecvTimeoutError::Timeout) => {
                            drop(stream);
                            sounds.recv().ok()
                        }
                        Err(RecvTimeoutError::Disconnected) => None,
                    };
                }
            })
            .ok()
            .map(|_| Mutex::new(asked))
    });
    if let Some(Ok(player)) = player.as_ref().map(Mutex::lock) {
        let _ = player.send(sound);
    }
}

/// Installed from a release's archive, nothing else made the launcher's entry (with its icon and
/// `asides://` links): it goes in the user's own applications. A package has its own, and one left
/// there by an archive would come before it, and start what may be gone.
pub fn register_links() {
    if cfg!(feature = "dev-hooks") {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let data = std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).filter(|p| p.is_absolute());
    let data = data.unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(".local/share"));
    let file = data.join("applications").join(format!("{BUNDLE_ID}.desktop"));
    if exe.starts_with("/usr") {
        if std::fs::read_to_string(&file).is_ok_and(|entry| starts_another(&entry, &exe)) {
            let _ = std::fs::remove_file(&file);
        }
        return;
    }
    let more = format!("MimeType=x-scheme-handler/asides;\nStartupWMClass={BUNDLE_ID}\n");
    let Some(entry) = desktop_entry(" %u", &more) else {
        return;
    };
    if std::fs::read_to_string(&file).is_ok_and(|now| now == entry) {
        return;
    }
    let written = file.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|()| std::fs::write(&file, entry));
    if let Err(err) = written {
        eprintln!("asides: no launcher entry ({}): {err}", file.display());
        return;
    }
    // Quiet: on Plasma 6 it looks for Qt 5's `qtpaths`, and says so, though it works.
    let _ = std::process::Command::new("xdg-mime")
        .args(["default", &format!("{BUNDLE_ID}.desktop"), "x-scheme-handler/asides"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Whether this desktop entry starts another program than `exe`. (One the user changed to start this
/// one stays theirs.)
fn starts_another(entry: &str, exe: &Path) -> bool {
    let exe = exe.to_string_lossy();
    !entry.lines().any(|line| line.starts_with("Exec=") && line.contains(exe.as_ref()))
}

/// Windows' way of letting the overlay take the keyboard: on X11 it just takes it.
pub fn allow_focus(_pid: i32) {}

/// How putting the `asides` command on the PATH went.
pub enum CommandLink {
    Linked,
    /// macOS's administrator prompt; Linux asks nothing.
    #[allow(dead_code)]
    Declined,
    Failed(String),
}

/// The command to put on the user's PATH, if nothing like it is there yet: the one beside this app.
pub fn command_to_link() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let cli = exe.with_file_name("asides");
    if exe.starts_with("/usr") || !cli.is_file() {
        return None;
    }
    let home = std::env::home_dir()?;
    let found = asides_core::sessions::linux::find_program(&std::env::var_os("PATH")?, "asides").is_some();
    (!found && !home.join(".local/bin/asides").exists()).then_some(cli)
}

/// `~/.local/bin/asides` → the command beside the app.
pub fn link_command(cli: &Path) -> CommandLink {
    let Some(home) = std::env::home_dir() else { return CommandLink::Failed("no home folder".into()) };
    let bin = home.join(".local/bin");
    let done = std::fs::create_dir_all(&bin).and_then(|()| std::os::unix::fs::symlink(cli, bin.join("asides")));
    match done {
        Ok(()) => CommandLink::Linked,
        Err(err) => CommandLink::Failed(err.to_string()),
    }
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
pub fn main_thread_working<T>(watchdog: &Option<Watchdog>, _: &crate::tao::event::Event<'_, T>) {
    if let Some(watchdog) = watchdog {
        watchdog.working();
    }
}

/// ...and done with it: between events, the loop waits.
pub fn main_thread_done(watchdog: &Option<Watchdog>) {
    if let Some(watchdog) = watchdog {
        watchdog.waiting();
    }
}

/// Brings the app with this process id to the front again, as a pager asks a window manager. On a
/// Wayland desktop nobody may ask: closing asides' windows gives the keyboard back by itself.
pub fn activate_app(pid: i32) {
    if pid == FRONT_UNKNOWN {
        return;
    }
    let Some((connection, screen)) = asides_core::display::connect_x11() else { return };
    let _ = activate_on(&connection, screen, pid);
}

fn activate_on(connection: &RustConnection, screen: usize, pid: i32) -> Option<()> {
    let root = connection.setup().roots.get(screen)?.root;
    let (list, active) = (atom(connection, b"_NET_CLIENT_LIST_STACKING")?, atom(connection, b"_NET_ACTIVE_WINDOW")?);
    let windows = connection.get_property(false, root, list, AtomEnum::WINDOW, 0, 4096).ok()?.reply().ok()?;
    // The list runs from the bottom up: the app's window nearest the top.
    let window =
        windows.value32()?.collect::<Vec<_>>().into_iter().rev().find(|&w| window_pid(connection, w) == Some(pid))?;
    // Source 2: a pager, which a window manager obeys.
    let event = ClientMessageEvent::new(32, window, active, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    connection.send_event(false, root, mask, event).ok()?;
    connection.flush().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_which_linux_it_is() {
        assert!(!system_version().is_empty());
    }

    #[test]
    fn knows_the_terminals() {
        assert!(is_terminal("gnome-terminal-server") && is_terminal("konsole") && !is_terminal("firefox"));
    }

    #[test]
    fn an_entry_left_by_an_archive_starts_another_program() {
        let exe = Path::new("/usr/lib/asides/asides-app");
        assert!(starts_another("[Desktop Entry]\nExec=/home/me/asides/asides-app %u\n", exe));
        assert!(starts_another("[Desktop Entry]\nName=asides\n", exe));
        assert!(!starts_another("[Desktop Entry]\nExec=env A=1 /usr/lib/asides/asides-app %u\n", exe));
    }

    #[test]
    fn quotes_desktop_entry_words() {
        assert_eq!(desktop_quote("/usr/bin/asides-app"), "/usr/bin/asides-app");
        assert_eq!(desktop_quote("/home/a b/asides$1"), "\"/home/a b/asides\\$1\"");
    }

    #[test]
    fn decodes_each_sound() {
        for sound in [Sound::MicOn, Sound::MicOff, Sound::Sent, Sound::Closed, Sound::Error, Sound::Dictation] {
            let (samples, channels, rate) = decode(sound.wav()).unwrap();
            assert!(!samples.is_empty() && channels > 0 && rate > 0);
            assert!(samples.iter().all(|s| s.abs() <= 1.0));
        }
    }
}
