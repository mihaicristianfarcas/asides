//! `asides-overlay`: Asides' Measure & Annotate overlay. The app starts it with a session
//! directory (see `asides_core::helper`); it shows the frozen screens, lets the user measure,
//! annotate, draw and take screenshots while they talk, and writes what to add to the aside.
//!
//! Started with `--standby`, it starts GPUI and opens the screens' windows, then waits for the app
//! to open a session: the app keeps one waiting, so opening Asides doesn't wait for them.
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
use std::sync::mpsc::Receiver;

use asides_core::helper::{Command, Event, STANDBY, Spec};
use asides_core::measure::{Frame, Tolerance};
use asides_core::overlay::{Action, Placed, Saved, Script, Session};
use gpui::{App, AppContext, Application};

use crate::ui::Overlay;

fn main() {
    let Some(arg) = std::env::args_os().nth(1) else {
        eprintln!("usage: asides-overlay <session directory> | {STANDBY}");
        std::process::exit(2);
    };
    if arg == STANDBY {
        return standby();
    }
    if let Err(err) = run(Path::new(&arg)) {
        failed(&err);
        std::process::exit(1);
    }
}

fn failed(err: &anyhow::Error) {
    eprintln!("asides-overlay: {err:#}");
    io::emit(&Event::Cancelled { saved: Saved::default() });
}

fn run(dir: &Path) -> anyhow::Result<()> {
    let (spec, frames) = io::session(dir)?;
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
    native::stay_accessory_at_launch();
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        launched(cx);
        let commands = io::commands(spec.replay.is_none());
        open(dir, spec, frames, commands, Vec::new(), cx);
    });
}

/// Started ahead of time: GPUI starts, and a window opens for each screen (`windows::Spare`),
/// then it waits for the app to open a session. Opening Asides then doesn't wait for a process,
/// GPUI and its windows, about 120 ms.
fn standby() {
    native::stay_accessory_at_launch();
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        launched(cx);
        let spares = windows::open_spares(cx);
        let (opened, commands) = io::standby();
        cx.spawn(async move |cx| {
            let session = opened.await;
            cx.update(|cx| match session {
                Ok(Ok((dir, spec, frames))) => match spec.script.clone() {
                    Some(script) if spec.replay.is_none() => {
                        if let Err(err) = headless(&dir, &spec, frames, &script) {
                            failed(&err);
                        }
                        cx.quit();
                    }
                    _ => open(dir, spec, frames, commands, spares, cx),
                },
                Ok(Err(err)) => {
                    failed(&err);
                    cx.quit();
                }
                // The app went away first.
                Err(_) => cx.quit(),
            })
        })
        .detach();
    });
}

/// GPUI has started: the overlay's app is an accessory one, with its keys.
fn launched(cx: &mut App) {
    native::accessory();
    input::bind_keys(cx);
}

/// Opens the session: the clock opens its windows, or takes the spare ones.
fn open(
    dir: PathBuf,
    spec: Spec,
    frames: Vec<Frame>,
    commands: Receiver<Command>,
    spares: Vec<windows::Spare>,
    cx: &mut App,
) {
    // The app the user is in, before the overlay's takes over.
    let previous = native::front_app();
    let replay = spec.replay.clone();
    let overlay = cx.new(|_| Overlay::new(spec, frames, dir, commands));
    overlay.update(cx, |o, _| {
        o.previous_app = previous;
        o.spares = spares;
        o.start();
    });
    native::debug("ready");
    native::monitor_keys();
    io::emit(&Event::Ready);
    ui::run_clock(&overlay, cx);
    if let Some(steps) = replay {
        replay::run(overlay.clone(), steps, cx);
    }
}
