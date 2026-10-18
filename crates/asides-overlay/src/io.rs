//! Talking to the app: `Event`s as JSON lines on standard output, `Command`s from standard input
//! (see `asides_core::helper`).

use std::io::{BufRead, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};

use anyhow::{Context as _, bail};
use asides_core::helper::{self, Command, Event, SPEC, Spec, SpecScreen};
use asides_core::measure::Frame;
use futures::channel::oneshot;

pub fn emit(event: &Event) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", helper::line(event));
    let _ = out.flush();
}

/// The session in `dir`: its spec, and its frozen screens' pixels (none over the live screen).
pub fn session(dir: &Path) -> anyhow::Result<(Spec, Vec<Frame>)> {
    let text = std::fs::read_to_string(dir.join(SPEC)).with_context(|| format!("no {SPEC} in {}", dir.display()))?;
    let spec: Spec = serde_json::from_str(&text).context("could not read the session")?;
    if spec.screens.is_empty() {
        bail!("no screens");
    }
    // Opened over the live screen, there are no frozen screens yet.
    if spec.live {
        return Ok((spec, Vec::new()));
    }
    let frames = spec.screens.iter().map(|s| pixels(dir, s)).collect::<anyhow::Result<Vec<Frame>>>()?;
    Ok((spec, frames))
}

/// A frozen screen's pixels, read by a few threads at once: read by one, a screen's tens of
/// megabytes take 15 ms.
pub fn pixels(dir: &Path, screen: &SpecScreen) -> anyhow::Result<Frame> {
    let file =
        std::fs::File::open(dir.join(&screen.pixels)).with_context(|| format!("no pixels in {}", screen.pixels))?;
    let mut rgba = vec![0; screen.width as usize * screen.height as usize * 4];
    if file.metadata()?.len() < rgba.len() as u64 {
        bail!("{} is too short for {} × {}", screen.pixels, screen.width, screen.height);
    }
    let part = rgba.len().div_ceil(threads()).max(1);
    std::thread::scope(|scope| {
        let reads: Vec<_> = rgba
            .chunks_mut(part)
            .enumerate()
            .map(|(i, chunk)| {
                let file = &file;
                scope.spawn(move || file.read_exact_at(chunk, (i * part) as u64))
            })
            .collect();
        reads.into_iter().try_for_each(|read| read.join().unwrap_or_else(|_| Err(std::io::Error::other("panicked"))))
    })
    .with_context(|| format!("could not read {}", screen.pixels))?;
    Ok(Frame { width: screen.width, height: screen.height, rgba })
}

/// How many threads share copying a screen's pixels.
pub fn threads() -> usize {
    std::thread::available_parallelism().map_or(1, |n| n.get()).min(8)
}

/// The app's commands, read on a thread. When the app goes away (standard input ends), the
/// overlay closes, unless `close_on_end` is off (a development run).
pub fn commands(close_on_end: bool) -> Receiver<Command> {
    let (tx, rx) = channel();
    std::thread::spawn(move || forward(std::io::stdin().lock().lines(), &tx, close_on_end));
    rx
}

/// The session the app opens, with its directory.
pub type Opened = oneshot::Receiver<anyhow::Result<(PathBuf, Spec, Vec<Frame>)>>;

/// Started ahead of time: the session the app opens (`Command::Open`), read on the same thread, with
/// its pixels; then its commands, as `commands` gives them (a replay stays open). The session never
/// comes when the app goes away first.
pub fn standby() -> (Opened, Receiver<Command>) {
    let (opened, open) = oneshot::channel();
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let mut lines = std::io::stdin().lock().lines();
        let dir = loop {
            let Some(Ok(line)) = lines.next() else { return };
            match serde_json::from_str::<Command>(&line) {
                Ok(Command::Open { dir }) => break dir,
                // Nothing is open to take it.
                Ok(_) => {}
                Err(err) => eprintln!("asides-overlay: ignored {line:?}: {err}"),
            }
        };
        let session = session(&dir);
        let close_on_end = session.as_ref().map_or(true, |(spec, _)| spec.replay.is_none());
        let _ = opened.send(session.map(|(spec, frames)| (dir, spec, frames)));
        forward(lines, &tx, close_on_end);
    });
    (open, rx)
}

fn forward(lines: impl Iterator<Item = std::io::Result<String>>, tx: &Sender<Command>, close_on_end: bool) {
    for line in lines {
        let Ok(line) = line else { break };
        match serde_json::from_str::<Command>(&line) {
            Ok(command) => {
                if tx.send(command).is_err() {
                    return;
                }
            }
            Err(err) => eprintln!("asides-overlay: ignored {line:?}: {err}"),
        }
    }
    if close_on_end {
        let _ = tx.send(Command::Close);
    }
}
