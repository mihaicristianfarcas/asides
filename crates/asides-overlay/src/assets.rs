//! The pill's and the menus' icons (24 × 24, drawn as masks in the text color).

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
        "annotate",
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
    (
        "grip",
        icon(
            r#"<circle cx="9" cy="6.5" r="1"/><circle cx="15" cy="6.5" r="1"/><circle cx="9" cy="12" r="1"/><circle cx="15" cy="12" r="1"/><circle cx="9" cy="17.5" r="1"/><circle cx="15" cy="17.5" r="1"/>"#,
        ),
    ),
    ("fold", icon(r#"<path d="M14.5 6.5 9 12l5.5 5.5"/>"#)),
    ("unfold", icon(r#"<path d="M9.5 6.5 15 12l-5.5 5.5"/>"#)),
    ("live", icon(r#"<path d="M8 5.5v13L18.5 12z"/>"#)),
    ("pause", icon(r#"<path d="M9 6v12M15 6v12"/>"#)),
    (
        "mic-off",
        icon(
            r#"<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5.5 11a6.5 6.5 0 0 0 13 0M12 17.5V21M4 4l16 16"/>"#,
        ),
    ),
    ("chevron", icon(r#"<path d="M7 10l5 5 5-5"/>"#)),
    (
        "folder",
        icon(
            r#"<path d="M3.5 7.5A1.5 1.5 0 0 1 5 6h4l2 2h8a1.5 1.5 0 0 1 1.5 1.5v8A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z"/>"#,
        ),
    ),
    ("trash", icon(r#"<path d="M4.5 7h15M9.5 7V5h5v2M6.5 7l1 12.5h9l1-12.5"/><path d="M10.5 10.5v6M13.5 10.5v6"/>"#)),
    ("plus", icon(r#"<path d="M12 5.5v13M5.5 12h13"/>"#)),
    ("chevron-up-down", icon(r#"<path d="M8 9.5l4-4 4 4M8 14.5l4 4 4-4"/>"#)),
    ("clock", icon(r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>"#)),
    (
        "terminal",
        icon(r#"<rect x="3.5" y="5" width="17" height="14" rx="2"/><path d="M7.5 10l2.5 2-2.5 2M12.5 14.5h4"/>"#),
    ),
    (
        "sliders",
        icon(r#"<path d="M4 7h7M15 7h5M4 17h3M11 17h9"/><circle cx="13" cy="7" r="2"/><circle cx="9" cy="17" r="2"/>"#),
    ),
    (
        "keyboard",
        icon(r#"<rect x="3" y="6.5" width="18" height="11" rx="2"/><path d="M7 10h1M11.5 10h1M16 10h1M8 14h8"/>"#),
    ),
    ("minus", icon(r#"<path d="M5.5 12h13"/>"#)),
    // Updates: one to download, one to restart into, what's new in it, one that failed.
    ("download", icon(r#"<path d="M12 4.5v10M7.5 10.5 12 15l4.5-4.5M5 18.5h14"/>"#)),
    ("restart", icon(r#"<path d="M20.8 5.6v4.8H16"/><path d="M18.8 14.4a7.2 7.2 0 1 1-1.7-7.49l3.7 3.49"/>"#)),
    (
        "sparkles",
        icon(
            r#"<path d="M10.5 4.5c.6 3.9 2.6 5.9 6.5 6.5-3.9.6-5.9 2.6-6.5 6.5-.6-3.9-2.6-5.9-6.5-6.5 3.9-.6 5.9-2.6 6.5-6.5z"/><path d="M18 14.5c.2 1.3.9 2 2.2 2.2-1.3.2-2 .9-2.2 2.2-.2-1.3-.9-2-2.2-2.2 1.3-.2 2-.9 2.2-2.2z"/>"#,
        ),
    ),
    ("warning", icon(r#"<path d="M12 4.5 20.5 19h-17z"/><path d="M12 10v4M12 16.6v.1"/>"#)),
    // Setup's permissions: the screen, and Accessibility's figure.
    ("display", icon(r#"<rect x="3.5" y="4.5" width="17" height="11.5" rx="1.5"/><path d="M9 20h6M12 16v4"/>"#)),
    (
        "person",
        icon(
            r#"<circle cx="12" cy="5" r="1.6"/><path d="M5 8.5l7 1.5 7-1.5M12 10v4.5M12 14.5l-3 5.5M12 14.5l3 5.5"/>"#,
        ),
    ),
    // asides' mark: brackets around a point.
    ("mark", icon(r#"<path d="M8.5 4.5h-3v15h3M15.5 4.5h3v15h-3"/><circle cx="12" cy="12" r="1.6"/>"#)),
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
