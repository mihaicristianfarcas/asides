//! Measure & Annotate's overlay runs in its own process, `asides-overlay` (drawn with GPUI, which
//! needs its own event loop), next to the app's binary. This module gives it a session directory
//! (the frozen screens and the settings, see `asides_core::helper`), relays its requests (what's
//! under a mark, talking) to the app, and reads what it adds to the aside. Main thread.

use std::io::{BufRead, BufReader, Write};
#[cfg(feature = "dev-hooks")]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command as Process, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use anyhow::Context as _;
use asides_core::helper::{Command, Event, Outcome, RESULT, SPEC, Spec};
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

pub struct Overlay {
    child: Child,
    stdin: ChildStdin,
    pub dir: PathBuf,
    /// The windows on screen when it opened, to name what's under a mark.
    pub windows: Vec<WindowInfo>,
    /// When its windows came up: marks carry the seconds since.
    pub opened: Option<Instant>,
    /// A recording was started from it (it ends with it).
    pub started_recording: bool,
    /// It finished or was cancelled (the process is ending).
    pub over: bool,
    /// The app that was frontmost, to give the focus back to.
    pub previous_app: Option<i32>,
}

/// `asides-overlay`, beside the app's binary (`ASIDES_OVERLAY` overrides it, for development).
pub fn helper() -> PathBuf {
    if let Some(path) = std::env::var_os("ASIDES_OVERLAY") {
        return PathBuf::from(path);
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.with_file_name("asides-overlay")
}

/// Starts the overlay on these screens (`spec.screens[i].pixels` is filled in), telling `notify`
/// what it says.
pub fn start(
    mut spec: Spec,
    frames: &[Frame],
    windows: Vec<WindowInfo>,
    notify: impl Fn(Message) + Send + 'static,
) -> anyhow::Result<Overlay> {
    static SESSIONS: AtomicU32 = AtomicU32::new(0);
    let n = SESSIONS.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("asides-overlay-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).context("no session directory")?;
    for (i, (screen, frame)) in spec.screens.iter_mut().zip(frames).enumerate() {
        screen.pixels = format!("screen-{i}.rgba");
        screen.width = frame.width;
        screen.height = frame.height;
        std::fs::write(dir.join(&screen.pixels), &frame.rgba).context("could not save the screen")?;
    }
    std::fs::write(dir.join(SPEC), serde_json::to_string(&spec)?)?;
    let mut child = Process::new(helper())
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("could not start {}", helper().display()))?;
    let stdin = child.stdin.take().context("no stdin")?;
    let stdout = child.stdout.take().context("no stdout")?;
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Event>(&line) {
                Ok(event) => notify(Message::Event(event)),
                Err(err) => eprintln!("overlay said {line:?}: {err}"),
            }
        }
        notify(Message::Exited);
    });
    Ok(Overlay { child, stdin, dir, windows, opened: None, started_recording: false, over: false, previous_app: None })
}

impl Overlay {
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

    /// Seconds from the start of a recording (started at `started`) to when the overlay opened;
    /// negative when the recording started after.
    pub fn offset_from(&self, started: Instant) -> f32 {
        let opened = self.opened.unwrap_or_else(Instant::now);
        match opened.checked_duration_since(started) {
            Some(after) => after.as_secs_f32(),
            None => -started.duration_since(opened).as_secs_f32(),
        }
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

/// An image file's pixels, for simulated sessions.
#[cfg(feature = "dev-hooks")]
pub fn load_image(path: &Path) -> Option<Frame> {
    let data = NSData::with_bytes(&std::fs::read(path).ok()?);
    let rep = NSBitmapImageRep::imageRepWithData(&data)?;
    let image: CFRetained<CGImage> = rep.CGImage()?.into();
    crate::capture::to_frame(&image)
}
