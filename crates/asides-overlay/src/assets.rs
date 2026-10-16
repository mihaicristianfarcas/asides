//! The toolbar's icons (24 × 24, drawn as masks in the text color).

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

const fn icon(body: &'static str) -> &'static str {
    body
}

const ICONS: &[(&str, &str)] = &[
    (
        "measure",
        icon(r#"<path d="M3.5 16.5 16.5 3.5l4 4-13 13z"/><path d="M7.5 12.5l2 2M10.5 9.5l2 2M13.5 6.5l2 2"/>"#),
    ),
    (
        "mark",
        icon(
            r#"<path d="M12 21s-6.5-5.8-6.5-10.5a6.5 6.5 0 0 1 13 0C18.5 15.2 12 21 12 21z"/><circle cx="12" cy="10.5" r="2.2"/>"#,
        ),
    ),
    ("draw", icon(r#"<path d="M4 20l1-4.5L15.5 5a2.1 2.1 0 0 1 3 3L8 18.5z"/><path d="M13.5 7l3 3"/>"#)),
    (
        "shot",
        icon(
            r#"<path d="M4 8V5.5A1.5 1.5 0 0 1 5.5 4H8M16 4h2.5A1.5 1.5 0 0 1 20 5.5V8M20 16v2.5a1.5 1.5 0 0 1-1.5 1.5H16M8 20H5.5A1.5 1.5 0 0 1 4 18.5V16"/>"#,
        ),
    ),
    (
        "camera",
        icon(
            r#"<path d="M4 8.5A1.5 1.5 0 0 1 5.5 7H8l1.5-2.5h5L16 7h2.5A1.5 1.5 0 0 1 20 8.5v9a1.5 1.5 0 0 1-1.5 1.5h-13A1.5 1.5 0 0 1 4 17.5z"/><circle cx="12" cy="13" r="3.2"/>"#,
        ),
    ),
    (
        "mic",
        icon(r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21"/>"#),
    ),
    ("undo", icon(r#"<path d="M9 14 4 9l5-5"/><path d="M4 9h10.5a5.5 5.5 0 0 1 0 11H11"/>"#)),
    (
        "copy",
        icon(
            r#"<rect x="8.5" y="8.5" width="11.5" height="11.5" rx="2"/><path d="M15.5 8.5V6A2 2 0 0 0 13.5 4H6a2 2 0 0 0-2 2v7.5a2 2 0 0 0 2 2h2.5"/>"#,
        ),
    ),
    (
        "help",
        icon(
            r#"<circle cx="12" cy="12" r="8.5"/><path d="M9.6 9.4a2.5 2.5 0 1 1 3.4 2.3c-.6.3-1 .8-1 1.5v.6"/><path d="M12 16.6v.4"/>"#,
        ),
    ),
    ("check", icon(r#"<path d="M5 12.5l4.5 4.5L19 7.5"/>"#)),
    ("send", icon(r#"<path d="M5 12h13M12.5 5.5 19 12l-6.5 6.5"/>"#)),
    ("close", icon(r#"<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>"#)),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        let name = path.strip_prefix("icons/").and_then(|p| p.strip_suffix(".svg"));
        Ok(ICONS.iter().find(|(n, _)| Some(*n) == name).map(|(_, body)| {
            let svg = format!(
                r##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">{body}</svg>"##
            );
            Cow::Owned(svg.into_bytes())
        }))
    }

    fn list(&self, _: &str) -> gpui::Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(n, _)| SharedString::from(format!("icons/{n}.svg"))).collect())
    }
}
