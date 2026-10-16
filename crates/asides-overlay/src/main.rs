//! `asides-overlay`: Asides' Measure & Annotate overlay. The app starts it with a session
//! directory (see `asides_core::helper`); it shows the frozen screens, lets the user measure,
//! mark, draw and take screenshots while they talk, and writes what to add to the aside.
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

use std::path::{Path, PathBuf};

use anyhow::{Context as _, bail};
use asides_core::helper::{Event, SPEC, Spec};
use asides_core::measure::{Frame, Tolerance};
use asides_core::overlay::{Placed, Saved, Script, Session};
use gpui::{
    App, AppContext, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, point,
    px, size,
};

use crate::ui::{Overlay, ScreenView, ScreenWindow};

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
    let frames = spec
        .screens
        .iter()
        .map(|s| {
            let rgba = std::fs::read(dir.join(&s.pixels)).with_context(|| format!("no pixels in {}", s.pixels))?;
            if rgba.len() < (s.width * s.height * 4) as usize {
                bail!("{} is too short for {} × {}", s.pixels, s.width, s.height);
            }
            Ok(Frame { width: s.width, height: s.height, rgba })
        })
        .collect::<anyhow::Result<Vec<Frame>>>()?;
    match spec.script.clone() {
        Some(script) if spec.replay.is_none() => headless(dir, &spec, frames, &script),
        _ => {
            gui(dir.to_path_buf(), spec, frames);
            Ok(())
        }
    }
}

/// Plays a script, without windows, and writes the result.
fn headless(dir: &Path, spec: &Spec, frames: Vec<Frame>, script: &Script) -> anyhow::Result<()> {
    let screens = spec.screens.iter().zip(&frames).map(|(s, f)| Session::screen(s.screen.clone(), f.clone())).collect();
    let mut session = Session::new(screens, Tolerance(spec.tolerance.min(100)), spec.units);
    let refs: Vec<&Frame> = frames.iter().collect();
    let exporter = export::Exporter::new(&refs, style::parse_color(&spec.color));
    let mut shots = Vec::new();
    for action in &script.actions {
        if let Placed::Shot { screen, rect } = session.apply(action)
            && let Some(png) = exporter.render_within(&session, screen, rect, export::ASIDE_MAX_SIDE)
        {
            let name = format!("shot-{}.png", shots.len() + 1);
            std::fs::write(dir.join(&name), png)?;
            shots.push((0.0, name));
        }
    }
    let outcome = finish::outcome(&session, &exporter, spec, dir, false, &shots, 0, false)?;
    finish::write(dir, &outcome)?;
    io::emit(&Event::Finished);
    Ok(())
}

/// The overlay's windows, one per screen.
fn gui(dir: PathBuf, spec: Spec, frames: Vec<Frame>) {
    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        native::accessory();
        input::bind_keys(cx);
        let commands = io::commands(spec.replay.is_none());
        let windows: Vec<ScreenWindow> = spec
            .screens
            .iter()
            .map(|s| {
                let (offset, size) = match spec.windowed {
                    Some(r) => ((r.x, r.y), (r.width, r.height)),
                    None => ((0.0, 0.0), (s.screen.width, s.screen.height)),
                };
                ScreenWindow { handle: None, focus: None, offset, size, number: 0, view: 0 }
            })
            .collect();
        // The screen with the mouse on it gets the toolbar and the keyboard.
        let (mx, my) = native::mouse();
        let cursor = spec.screens.iter().enumerate().find_map(|(i, s)| {
            let f = s.frame;
            let inside = mx >= f.x && mx < f.x + f.width && my >= f.y && my < f.y + f.height;
            inside.then_some((i, mx - f.x, f.y + f.height - my))
        });
        let focused = cursor.map_or(0, |c| c.0);
        let overlay = cx.new(|_| {
            let mut o = Overlay::new(spec.clone(), frames, dir.clone(), commands);
            o.windows = windows;
            o
        });
        for (i, s) in spec.screens.iter().enumerate() {
            let (offset, (w, h)) = {
                let o = overlay.read(cx);
                (o.windows[i].offset, o.windows[i].size)
            };
            // AppKit's coordinates: from the bottom-left corner of the main screen.
            let frame = match spec.windowed {
                Some(_) => asides_core::inspect::PtRect::new(
                    s.frame.x + offset.0,
                    s.frame.y + s.frame.height - offset.1 - h,
                    w,
                    h,
                ),
                None => s.frame,
            };
            let display_id = cx.displays().into_iter().find(|d| u32::from(d.id()) == s.display).map(|d| d.id());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(0.), px(0.)),
                    size(px(w as f32), px(h as f32)),
                ))),
                titlebar: None,
                focus: i == focused,
                show: true,
                kind: WindowKind::PopUp,
                is_movable: false,
                is_resizable: false,
                is_minimizable: false,
                display_id,
                window_background: WindowBackgroundAppearance::Opaque,
                ..Default::default()
            };
            let o = overlay.clone();
            let opened = cx.open_window(options, move |window, cx| {
                let (number, view_pointer) = native::make_overlay(window, frame);
                let view = cx.new(|cx| ScreenView::new(o.clone(), i, cx));
                let focus = view.read(cx).focus_handle();
                window.focus(&focus);
                o.update(cx, |o, _| {
                    o.windows[i].focus = Some(focus);
                    o.windows[i].number = number;
                    o.windows[i].view = view_pointer;
                });
                view
            });
            match opened {
                Ok(handle) => overlay.update(cx, |o, _| o.windows[i].handle = Some(handle.into())),
                Err(err) => eprintln!("asides-overlay: no window for screen {i}: {err:#}"),
            }
        }
        cx.activate(true);
        if let Some(handle) = overlay.read(cx).windows[focused].handle {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        }
        overlay.update(cx, |o, _| {
            o.start();
            if spec.windowed.is_none() {
                o.session.cursor = cursor;
            }
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
