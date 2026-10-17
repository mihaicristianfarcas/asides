//! `asides-overlay`: Asides' Measure & Annotate overlay. The app starts it with a session
//! directory (see `asides_core::helper`); it shows the frozen screens, lets the user measure,
//! annotate, draw and take screenshots while they talk, and writes what to add to the aside.
//!
//! With a script in the session (`asides simulate-measure`), it plays it without any window and
//! writes the result.

mod assets;
mod export;
mod finish;
mod input;
mod io;
mod native;
mod replay;
mod style;
mod ui;
mod windows;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use asides_core::helper::{Event, SPEC, Spec};
use asides_core::measure::{Frame, Tolerance};
use asides_core::overlay::{Action, Placed, Saved, Script, Session};
use gpui::{App, AppContext, Application};

use crate::ui::Overlay;

fn main() {
    let Some(dir) = std::env::args_os().nth(1).map(PathBuf::from) else {
        eprintln!("usage: asides-overlay <session directory>");
        std::process::exit(2);
    };
    if let Err(err) = run(&dir) {
        eprintln!("asides-overlay: {err:#}");
        io::emit(&Event::Cancelled { saved: Saved::default() });
        std::process::exit(1);
    }
}

fn run(dir: &Path) -> anyhow::Result<()> {
    let text = std::fs::read_to_string(dir.join(SPEC)).with_context(|| format!("no {SPEC} in {}", dir.display()))?;
    let spec: Spec = serde_json::from_str(&text).context("could not read the session")?;
    if spec.screens.is_empty() {
        bail!("no screens");
    }
    // Opened over the live screen, there are no frozen screens yet.
    let frames = if spec.live {
        Vec::new()
    } else {
        spec.screens
            .iter()
            .map(|s| {
                let rgba = std::fs::read(dir.join(&s.pixels)).with_context(|| format!("no pixels in {}", s.pixels))?;
                if rgba.len() < (s.width * s.height * 4) as usize {
                    bail!("{} is too short for {} × {}", s.pixels, s.width, s.height);
                }
                Ok(Frame { width: s.width, height: s.height, rgba })
            })
            .collect::<anyhow::Result<Vec<Frame>>>()?
    };
    match spec.script.clone() {
        Some(script) if spec.replay.is_none() => headless(dir, &spec, frames, &script),
        _ => {
            gui(dir.to_path_buf(), spec, frames);
            Ok(())
        }
    }
}

/// Plays a script, without windows, and writes the result. `live` and `freeze` steps go back to
/// the live screen and freeze it again (the same image).
fn headless(dir: &Path, spec: &Spec, frames: Vec<Frame>, script: &Script) -> anyhow::Result<()> {
    let screens = || spec.screens.iter().zip(&frames).map(|(s, f)| Session::from_spec(s, f.clone())).collect();
    let mut session = Session::new(screens(), Tolerance(spec.tolerance.min(100)), spec.units);
    let refs: Vec<&Frame> = frames.iter().collect();
    let exporter = export::Exporter::new(&refs, style::parse_color(&spec.color));
    let (mut shots, mut items, mut live, mut freezes) = (Vec::new(), Vec::new(), false, 0);
    for action in &script.actions {
        match action {
            Action::Wait(seconds) => std::thread::sleep(std::time::Duration::from_secs_f64(seconds.clamp(0.0, 60.0))),
            Action::Live if !live => {
                items.extend(finish::items(&session, &exporter, spec, dir, false, &shots, 0, freezes, false)?);
                (live, freezes) = (true, freezes + 1);
                shots.clear();
            }
            Action::Freeze if live => {
                session = session.refrozen(screens());
                live = false;
            }
            _ => {}
        }
        if let Placed::Shot { screen, rect } = session.apply(action)
            && let Some(png) = exporter.render_within(&session, screen, rect, export::ASIDE_MAX_SIDE)
        {
            let name = format!("shot-{}-{}.png", freezes + 1, shots.len() + 1);
            std::fs::write(dir.join(&name), png)?;
            shots.push((session.clock, name));
        }
    }
    if !live {
        let alone = items.is_empty();
        items.extend(finish::items(&session, &exporter, spec, dir, false, &shots, 0, freezes, alone)?);
    }
    finish::write(dir, &finish::outcome(items, session.saved(), false))?;
    io::emit(&Event::Finished);
    Ok(())
}

/// The overlay's windows: the toolbar's over the live screen, or one per screen, frozen (opened by
/// the clock, `windows::sync`).
fn gui(dir: PathBuf, spec: Spec, frames: Vec<Frame>) {
    // The app the user is in, before the overlay's takes over.
    let previous = native::front_app();
    native::stay_accessory_at_launch();
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        native::accessory();
        input::bind_keys(cx);
        let commands = io::commands(spec.replay.is_none());
        let overlay = cx.new(|_| Overlay::new(spec.clone(), frames, dir.clone(), commands));
        overlay.update(cx, |o, _| {
            o.previous_app = previous;
            o.start();
        });
        native::debug("ready");
        native::monitor_keys();
        io::emit(&Event::Ready);
        ui::run_clock(&overlay, cx);
        if let Some(steps) = spec.replay.clone() {
            replay::run(overlay.clone(), steps, cx);
        }
    });
}
