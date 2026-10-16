//! What a finished session adds to the aside (`result.json` and its images).

use std::path::Path;

use anyhow::Context as _;
use asides_core::helper::{Item, Outcome, RESULT, Spec};
use asides_core::overlay::{Session, markdown};

use crate::export::{self, Exporter};

/// Each screen with something on it, as an image (with what was measured) at the start, each
/// mark's Markdown at the moment it was made, the screenshots taken when they were taken. With
/// nothing at all, the screen alone, like a screenshot.
#[allow(clippy::too_many_arguments)]
pub fn outcome(
    session: &Session,
    exporter: &Exporter,
    spec: &Spec,
    dir: &Path,
    whole: bool,
    shots: &[(f32, String)],
    cursor_screen: usize,
    send: bool,
) -> anyhow::Result<Outcome> {
    let mut items = Vec::new();
    let inspections = session.inspections();
    let image = |screen: usize, text: Option<String>, items: &mut Vec<Item>| -> anyhow::Result<()> {
        let crop = export::crop(session, screen, spec.padding.max(0.0), whole);
        let png = exporter
            .render_within(session, screen, crop, export::ASIDE_MAX_SIDE)
            .context("could not draw the screen")?;
        let name = format!("screen-{}.png", screen + 1);
        std::fs::write(dir.join(&name), png)?;
        items.push(Item { time: 0.0, image: Some(name), text });
        Ok(())
    };
    if inspections.is_empty() && shots.is_empty() {
        image(cursor_screen.min(session.screens.len().saturating_sub(1)), None, &mut items)?;
    }
    for (screen, inspection) in &inspections {
        let (measured, marks) = markdown(inspection, spec.detail);
        image(*screen, measured, &mut items)?;
        items.extend(marks.into_iter().map(|(time, text)| Item { time, image: None, text: Some(text) }));
    }
    items.extend(shots.iter().map(|(time, name)| Item { time: *time, image: Some(name.clone()), text: None }));
    items.sort_by(|a, b| a.time.total_cmp(&b.time));
    Ok(Outcome { send, saved: session.saved(), items })
}

pub fn write(dir: &Path, outcome: &Outcome) -> anyhow::Result<()> {
    std::fs::write(dir.join(RESULT), serde_json::to_string_pretty(outcome)?)?;
    Ok(())
}
