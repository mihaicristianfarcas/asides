//! Measure & Annotate's overlay runs in its own process, `asides-overlay` (drawn with GPUI, which
//! needs its own event loop), next to the app's binary. This module gives it a session directory
//! (the frozen screens and the settings, see `asides_core::helper`), relays its requests (what's
//! under an annotation, talking) to the app, and reads what it adds to the aside. The next one is
//! started ahead of time (`Standby`). Main thread.

use std::ffi::OsStr;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use anyhow::Context as _;
use asides_core::helper::{Command, Event, Outcome, RESULT, SPEC, STANDBY, Spec, SpecScreen};
use asides_core::measure::Frame;
#[cfg(feature = "dev-hooks")]
use objc2_app_kit::NSBitmapImageRep;
#[cfg(feature = "dev-hooks")]
use objc2_core_foundation::CFRetained;
#[cfg(feature = "dev-hooks")]
use objc2_core_graphics::CGImage;
#[cfg(feature = "dev-hooks")]
use objc2_foundation::NSData;

use crate::elements::WindowInfo;

/// What the overlay's process says, as the app hears it.
pub enum Message {
    Event(Event),
    /// The process ended.
    Exited,
}

/// Tells the app what the overlay with this process id says (from a thread of its own).
pub trait Notify: Fn(u32, Message) + Send + 'static {}
impl<F: Fn(u32, Message) + Send + 'static> Notify for F {}

/// An overlay started ahead of time, waiting for a session: its GPUI started and a window open for
/// each screen. Opening Asides doesn't wait for them, about 120 ms. Dropped, it quits (its standard
/// input ends).
pub struct Standby {
    child: Child,
    stdin: ChildStdin,
}

impl Standby {
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Opens the session in `dir`: the process is the session's now. `None` if it quit.
    fn open(mut self, dir: &Path) -> Option<(Child, ChildStdin)> {
        let open = asides_core::helper::line(&Command::Open { dir: dir.to_path_buf() });
        if writeln!(self.stdin, "{open}").and_then(|_| self.stdin.flush()).is_err() {
            let _ = self.child.kill();
            let _ = self.child.wait();
            return None;
        }
        Some((self.child, self.stdin))
    }
}

/// Starts an overlay ahead of time, telling `notify` what it says.
pub fn standby(notify: impl Notify) -> anyhow::Result<Standby> {
    let (child, stdin) = spawn(STANDBY, notify)?;
    Ok(Standby { child, stdin })
}

/// Starts `asides-overlay` with `arg` (a session directory, or `STANDBY`), relaying what it says to
/// `notify`.
fn spawn(arg: impl AsRef<OsStr>, notify: impl Notify) -> anyhow::Result<(Child, ChildStdin)> {
    let mut child = Process::new(helper())
        .arg(arg)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("could not start {}", helper().display()))?;
    let stdin = child.stdin.take().context("no stdin")?;
    let stdout = child.stdout.take().context("no stdout")?;
    let pid = child.id();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Event>(&line) {
                Ok(event) => notify(pid, Message::Event(event)),
                Err(err) => eprintln!("overlay said {line:?}: {err}"),
            }
        }
        notify(pid, Message::Exited);
    });
    Ok((child, stdin))
}

pub struct Overlay {
    child: Child,
    stdin: ChildStdin,
    pub dir: PathBuf,
    /// The windows on screen when it opened, to name what's under an annotation.
    pub windows: Vec<WindowInfo>,
    /// When it was started, and when its windows came up: annotations carry the seconds since.
    pub created: Instant,
    pub opened: Option<Instant>,
    /// A recording was started from it (it ends with it).
    pub started_recording: bool,
    /// It finished or was cancelled (the process is ending).
    pub over: bool,
    /// The app that was frontmost (when it opened, or last froze), to give the focus back to.
    pub previous_app: Option<i32>,
    /// Its screens are frozen (not live), and how many times they were.
    pub frozen: bool,
    freezes: usize,
}

/// `asides-overlay`, beside the app's binary (`ASIDES_OVERLAY` overrides it, for development).
pub fn helper() -> PathBuf {
    if let Some(path) = std::env::var_os("ASIDES_OVERLAY") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.with_file_name("asides-overlay")
}

/// Opens the overlay on these screens (`spec.screens[i].pixels` is filled in): in `standby`, or a
/// process of its own, telling `notify` what it says.
pub fn start(
    mut spec: Spec,
    frames: &[Frame],
    windows: Vec<WindowInfo>,
    standby: Option<Standby>,
    notify: impl Notify,
) -> anyhow::Result<Overlay> {
    static SESSIONS: AtomicU32 = AtomicU32::new(0);
    let n = SESSIONS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("asides-overlay-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).context("no session directory")?;
    // Over the live screen, the screens' pixels come when they freeze.
    if !spec.live {
        write_frames(&dir, 0, &mut spec.screens, frames)?;
    }
    let frozen = !spec.live;
    std::fs::write(dir.join(SPEC), serde_json::to_string(&spec)?)?;
    let (child, stdin) = match standby.and_then(|s| s.open(&dir)) {
        Some(process) => process,
        // None waiting, or it quit.
        None => spawn(&dir, notify)?,
    };
    Ok(Overlay {
        child,
        stdin,
        dir,
        windows,
        created: Instant::now(),
        opened: None,
        started_recording: false,
        over: false,
        previous_app: None,
        frozen,
        freezes: 1,
    })
}

/// Saves frozen screens' pixels in the session directory (the `freeze`th time), naming them in
/// `screens`.
fn write_frames(dir: &Path, freeze: usize, screens: &mut [SpecScreen], frames: &[Frame]) -> anyhow::Result<()> {
    for (i, (screen, frame)) in screens.iter_mut().zip(frames).enumerate() {
        screen.pixels = format!("screen-{freeze}-{i}.rgba");
        screen.width = frame.width;
        screen.height = frame.height;
        std::fs::write(dir.join(&screen.pixels), &frame.rgba).context("could not save the screen")?;
    }
    Ok(())
}

impl Overlay {
    /// Its process, whose windows screenshots and the window list leave out.
    pub fn pid(&self) -> i32 {
        self.child.id() as i32
    }

    /// The screens, frozen again: their pixels for the overlay.
    pub fn freeze(&mut self, mut screens: Vec<SpecScreen>, frames: &[Frame]) -> anyhow::Result<()> {
        write_frames(&self.dir, self.freezes, &mut screens, frames)?;
        self.freezes += 1;
        self.frozen = true;
        self.send(&Command::Frozen { screens });
        Ok(())
    }

    pub fn send(&mut self, command: &Command) {
        let _ = writeln!(self.stdin, "{}", asides_core::helper::line(command));
        let _ = self.stdin.flush();
    }

    /// What the finished session adds, with its images read.
    pub fn result(&self) -> anyhow::Result<(Outcome, Vec<Option<Vec<u8>>>)> {
        let text = std::fs::read_to_string(self.dir.join(RESULT)).context("no result")?;
        let outcome: Outcome = serde_json::from_str(&text)?;
        let images = outcome
            .items
            .iter()
            .map(|item| item.image.as_ref().and_then(|name| std::fs::read(self.dir.join(name)).ok()))
            .collect();
        Ok((outcome, images))
    }

    /// When its annotations' seconds count from: its windows coming up (or, without windows, its start).
    pub fn origin(&self) -> Instant {
        self.opened.unwrap_or(self.created)
    }

    /// Seconds from the start of a recording (started at `started`) to when the overlay opened;
    /// negative when the recording started after.
    pub fn offset_from(&self, started: Instant) -> f32 {
        seconds_between(started, self.origin())
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Seconds from `from` to `to`, negative when `to` is earlier.
pub fn seconds_between(from: Instant, to: Instant) -> f32 {
    match to.checked_duration_since(from) {
        Some(after) => after.as_secs_f32(),
        None => -from.duration_since(to).as_secs_f32(),
    }
}

/// An image file's pixels, for simulated sessions.
#[cfg(feature = "dev-hooks")]
pub fn load_image(path: &Path) -> Option<Frame> {
    let data = NSData::with_bytes(&std::fs::read(path).ok()?);
    let rep = NSBitmapImageRep::imageRepWithData(&data)?;
    let image: CFRetained<CGImage> = rep.CGImage()?.into();
    crate::capture::to_frame(&image)
}
