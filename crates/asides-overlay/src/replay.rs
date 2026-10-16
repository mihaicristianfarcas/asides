//! Development: plays mouse and key steps in the overlay's windows, as GPUI events, and takes
//! screenshots of the windows, to check what the user would see and do without anyone at the
//! keyboard. The events are AppKit's, sent to GPUI's view, so they take the path real ones take.
//! Steps (`Spec::replay`), in screen points:
//!
//! ```json
//! [{"move": [200, 140]}, {"down": [50, 160]}, {"drag": [175, 205]}, {"up": [175, 205]},
//!  {"key": "shift+h"}, {"type": "Too tight"}, {"keyup": "space"}, {"wait": 300},
//!  {"answer": {"name": "button \"Save\"", "app": "Safari"}}, {"capture": "after.png"}]
//! ```
//!
//! `shift`, `alt`, `cmd` and `right` modify mouse steps; `screen` picks the window. `answer`
//! answers the overlay's pending lookups as the app would (frames in global points), `answers`
//! answers them one each, oldest first (an arrow's two ends), and
//! `{"recording": 14}` says a recording is 14 s in.

use std::time::Duration;

use asides_core::helper::Command;
use asides_core::inspect::Element;
use asides_core::keys;
use gpui::{App, Entity};
use serde::Deserialize;

use crate::native::synthetic::{self, Mouse};
use crate::ui::Overlay;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct Step {
    screen: usize,
    #[serde(rename = "move")]
    to: Option<(f64, f64)>,
    down: Option<(f64, f64)>,
    drag: Option<(f64, f64)>,
    up: Option<(f64, f64)>,
    right: bool,
    shift: bool,
    alt: bool,
    cmd: bool,
    key: Option<String>,
    keyup: Option<String>,
    #[serde(rename = "type")]
    text: Option<String>,
    wait: Option<u64>,
    answer: Option<Element>,
    /// Answers the pending lookups one each, oldest first.
    answers: Option<Vec<Element>>,
    /// The app says it's recording, this many seconds in.
    recording: Option<f32>,
    capture: Option<String>,
}

pub fn run(overlay: Entity<Overlay>, steps: serde_json::Value, cx: &mut App) {
    let steps: Vec<Step> = match serde_json::from_value(steps) {
        Ok(steps) => steps,
        Err(err) => return eprintln!("asides-overlay: bad replay: {err}"),
    };
    cx.spawn(async move |cx| {
        // Let the windows draw first.
        cx.background_executor().timer(Duration::from_millis(400)).await;
        for step in steps {
            let wait = step.wait.unwrap_or(90);
            let Ok(Some((view, offset))) = cx.update(|cx| {
                let o = overlay.read(cx);
                o.windows.get(step.screen).map(|w| (w.view, w.offset))
            }) else {
                return;
            };
            // Outside any update: GPUI updates the window to handle each event.
            send(view, offset, &step);
            if cx.update(|cx| after(&overlay, &step, cx)).is_err() {
                return;
            }
            cx.background_executor().timer(Duration::from_millis(wait)).await;
        }
        cx.background_executor().timer(Duration::from_millis(300)).await;
        let _ = cx.update(|cx| overlay.update(cx, |o, cx| o.apply_command(Command::Close, cx)));
    })
    .detach();
}

fn send(view: usize, offset: (f64, f64), step: &Step) {
    let at = |(x, y): (f64, f64)| (x - offset.0, y - offset.1);
    let right = step.right;
    let mouse = |kind: Mouse, p: (f64, f64)| synthetic::mouse(view, kind, at(p), step.shift, step.alt, step.cmd);
    if let Some(p) = step.to {
        mouse(Mouse::Move, p);
    }
    if let Some(p) = step.down {
        mouse(Mouse::Move, p);
        mouse(Mouse::Down { right }, p);
    }
    if let Some(p) = step.drag {
        mouse(Mouse::Drag { right }, p);
    }
    if let Some(p) = step.up {
        mouse(Mouse::Up { right }, p);
    }
    if let Some(chord) = step.key.as_deref().and_then(|k| keys::parse(k).into_iter().next()) {
        synthetic::key(view, &chord, true);
    }
    if let Some(chord) = step.keyup.as_deref().and_then(|k| keys::parse(k).into_iter().next()) {
        synthetic::key(view, &chord, false);
    }
    if let Some(text) = &step.text {
        synthetic::text(view, text);
    }
}

fn after(overlay: &Entity<Overlay>, step: &Step, cx: &mut App) {
    if let Some(element) = &step.answer {
        overlay.update(cx, |o, cx| o.answer_all(element.clone(), cx));
    }
    if let Some(elements) = &step.answers {
        overlay.update(cx, |o, cx| o.answer_each(elements.clone(), cx));
    }
    if let Some(seconds) = step.recording {
        overlay.update(cx, |o, cx| o.apply_command(Command::Recording { seconds: Some(seconds) }, cx));
    }
    if let Some(path) = &step.capture {
        let number = overlay.read(cx).windows.get(step.screen).map_or(0, |w| w.number);
        let path = overlay.read(cx).dir().join(path);
        let status = std::process::Command::new("screencapture")
            .args(["-x", "-o", "-l", &number.to_string()])
            .arg(&path)
            .status();
        eprintln!("captured {} ({status:?})", path.display());
    }
}
