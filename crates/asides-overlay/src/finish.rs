//! What a finished session adds to the aside (`result.json` and its images).

use std::path::Path;

use anyhow::Context as _;
use asides_core::helper::{Item, Outcome, RESULT, Spec, Target};
use asides_core::overlay::{Saved, Session, markdown};

use crate::export::{self, Exporter};
use crate::style;

/// What one freeze adds to the aside: each screen with something on it, as an image around what's
/// on it (all of it when `whole`) at the moment it froze, with where and in what units it was
/// measured, then each measurement and annotation at the moment it was made, and the screenshots
/// taken when they were taken. With nothing on it, nothing (what was said goes alone), unless the
/// whole screen was asked for: the one under the pointer. Images are named after the freeze
/// (`snapshot`: which, and when, in seconds into the session), so a session's freezes don't
/// overwrite each other's.
#[allow(clippy::too_many_arguments)]
pub fn items(
    session: &Session,
    exporter: &Exporter,
    spec: &Spec,
    dir: &Path,
    whole: bool,
    shots: &[(f32, String)],
    cursor_screen: usize,
    (snapshot, frozen_at): (usize, f32),
) -> anyhow::Result<Vec<Item>> {
    let mut items = Vec::new();
    let inspections = session.inspections();
    let image = |screen: usize, text: Option<String>, time: f32, items: &mut Vec<Item>| -> anyhow::Result<()> {
        let crop = export::crop(session, screen, spec.padding.max(0.0), whole);
        let png = exporter
            .render_within(session, screen, crop, export::ASIDE_MAX_SIDE)
            .context("could not draw the screen")?;
        let name = format!("screen-{}-{}.png", snapshot + 1, screen + 1);
        std::fs::write(dir.join(&name), png)?;
        items.push(Item { time, image: Some(name), text, ..Item::default() });
        Ok(())
    };
    if inspections.is_empty() && shots.is_empty() && whole {
        image(cursor_screen.min(session.screens.len().saturating_sub(1)), None, frozen_at, &mut items)?;
    }
    let accent = style::rgb_hex(style::parse_color(&spec.color));
    for (screen, inspection) in &inspections {
        let (header, notes) = markdown(inspection, spec.detail);
        let first = notes.iter().map(|n| n.time).fold(frozen_at, f32::min);
        image(*screen, header, first, &mut items)?;
        items.extend(notes.into_iter().map(|n| {
            let (annotation, said) = n.annotation.unzip();
            let color = inspection.annotations.iter().find(|a| Some(a.key) == annotation).map(|a| {
                let (r, g, b, _) = style::annotation_color(a, a.intent, accent);
                format!("#{:06X}", style::rgb_hex((r, g, b)))
            });
            Item {
                time: n.time,
                text: Some(n.text),
                annotation,
                said: said.unwrap_or_default(),
                color,
                ..Item::default()
            }
        }));
    }
    items.extend(shots.iter().map(|(time, name)| Item { time: *time, image: Some(name.clone()), ..Item::default() }));
    // Stable: a screenshot comes before what was first done on it.
    items.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(items)
}

pub fn outcome(items: Vec<Item>, saved: Saved, target: Option<Target>) -> Outcome {
    Outcome { target, saved, items }
}

pub fn write(dir: &Path, outcome: &Outcome) -> anyhow::Result<()> {
    std::fs::write(dir.join(RESULT), serde_json::to_string_pretty(outcome)?)?;
    Ok(())
}
