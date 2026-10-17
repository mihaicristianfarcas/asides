//! What the user measured, annotated and drew on screen with Measure & Annotate, and how it reads in an
//! aside. Measurements are PixelSnap's (distances between edges, the bounds of an object, guides);
//! annotations are numbered, on an element, an area or a drawing (an arrow, a sketch), each with optional
//! typed feedback, an intent and a severity, as in Agentation, and with what the user said around
//! it. Positions are in points, origin at the screen's top-left corner, and shown in points or
//! pixels as the user chose.
//!
//! An element under a marker is identified from the accessibility tree when the user allows it
//! (`Node`s, innermost first): its role and label, the window it's in, and for web pages the URL
//! and a CSS-like selector built from the DOM ids and classes browsers expose.

use serde::{Deserialize, Serialize};

/// How much of what was captured goes into the aside.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// One line per annotation and per measurement group.
    Compact,
    /// Element, location, page and feedback.
    #[default]
    Standard,
    /// Also positions, sizes and nearby text.
    Detailed,
    /// Also colors, padding and accessibility details.
    Forensic,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Units {
    /// Logical points, what CSS pixels and AppKit coordinates use.
    #[default]
    Pt,
    /// Physical pixels on the screen (points × the screen's scale).
    Px,
}

impl Units {
    pub fn label(self) -> &'static str {
        match self {
            Units::Pt => "pt",
            Units::Px => "px",
        }
    }
}

/// The screen a session was on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Screen {
    pub name: Option<String>,
    /// Size in points.
    pub width: f64,
    pub height: f64,
    /// Pixels per point (2 on Retina screens).
    pub scale: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PtRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl PtRect {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> PtRect {
        PtRect { x, y, width, height }
    }

    pub fn right(&self) -> f64 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f64 {
        self.y + self.height
    }

    pub fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &PtRect) -> PtRect {
        let (x, y) = (self.x.min(other.x), self.y.min(other.y));
        PtRect { x, y, width: self.right().max(other.right()) - x, height: self.bottom().max(other.bottom()) - y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    /// A vertical guide, positioned by its x.
    X,
    /// A horizontal guide, positioned by its y.
    Y,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub axis: Axis,
    pub position: f64,
}

/// A run along one axis, `start..end` in points; `open_start`/`open_end` when it reached the
/// screen's edge instead of an edge on screen.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub open_start: bool,
    #[serde(default)]
    pub open_end: bool,
}

impl Run {
    pub fn len(&self) -> f64 {
        self.end - self.start
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Measurement {
    /// From a point, the distance to the edges across and down.
    Distance { x: f64, y: f64, across: Run, down: Run },
    /// A dragged area; `snapped` when it was shrunk to the object it surrounded.
    Area { rect: PtRect, snapped: bool },
}

impl Measurement {
    /// The part of the screen it covers.
    pub fn bounds(&self) -> PtRect {
        match self {
            Measurement::Distance { x, y, across, down } => {
                let h = PtRect::new(across.start, *y, across.len(), 1.0);
                let v = PtRect::new(*x, down.start, 1.0, down.len());
                h.union(&v)
            }
            Measurement::Area { rect, .. } => *rect,
        }
    }

    /// `24 × 16` (across × down), one length when only one axis was held, or an area's `W × H`,
    /// for the clipboard.
    pub fn dimensions(&self, scale: f64, units: Units) -> String {
        let n = |v: f64| num(v, scale, units);
        match self {
            Measurement::Distance { across, down, .. } if down.is_empty() => n(across.len()),
            Measurement::Distance { across, down, .. } if across.is_empty() => n(down.len()),
            Measurement::Distance { across, down, .. } => format!("{} × {}", n(across.len()), n(down.len())),
            Measurement::Area { rect, .. } => format!("{} × {}", n(rect.width), n(rect.height)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Intent {
    Fix,
    Change,
    Question,
    Approve,
}

impl Intent {
    pub const ALL: [Intent; 4] = [Intent::Fix, Intent::Change, Intent::Question, Intent::Approve];

    pub fn as_str(self) -> &'static str {
        match self {
            Intent::Fix => "fix",
            Intent::Change => "change",
            Intent::Question => "question",
            Intent::Approve => "approve",
        }
    }

    pub fn parse(s: &str) -> Option<Intent> {
        Intent::ALL.into_iter().find(|i| i.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Blocking,
    Important,
    Suggestion,
}

impl Severity {
    pub const ALL: [Severity; 3] = [Severity::Blocking, Severity::Important, Severity::Suggestion];

    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Blocking => "blocking",
            Severity::Important => "important",
            Severity::Suggestion => "suggestion",
        }
    }

    pub fn parse(s: &str) -> Option<Severity> {
        Severity::ALL.into_iter().find(|i| i.as_str().eq_ignore_ascii_case(s.trim()))
    }
}

/// An annotation drawn with the Draw tool: an arrow (from what to where) or a freehand sketch (circling
/// or underlining something). Points are in the screen's points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Shape {
    Arrow { from: (f64, f64), to: (f64, f64) },
    Sketch { points: Vec<(f64, f64)> },
}

impl Shape {
    /// The part of the screen it covers.
    pub fn bounds(&self) -> PtRect {
        let points: Vec<(f64, f64)> = match self {
            Shape::Arrow { from, to } => vec![*from, *to],
            Shape::Sketch { points } => points.clone(),
        };
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for (x, y) in points {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
        if x0 > x1 {
            return PtRect::default();
        }
        PtRect::new(x0, y0, x1 - x0, y1 - y0)
    }
}

/// How an element looks, measured from the screenshot: colors as `#RRGGBB`, padding in points
/// (top, right, bottom, left).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Look {
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub padding: Option<[f64; 4]>,
}

/// One node of the accessibility tree, as the app reads it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Node {
    /// `AXButton`, `AXGroup`, `AXWebArea`, …
    pub role: String,
    pub subrole: Option<String>,
    /// The localized role name, e.g. "button".
    pub role_description: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub value: Option<String>,
    pub placeholder: Option<String>,
    pub identifier: Option<String>,
    /// Web content: the element's DOM id and classes.
    pub dom_id: Option<String>,
    pub dom_classes: Vec<String>,
    /// Web content: heading level (h1–h6).
    pub level: Option<u32>,
    /// `AXWebArea` (and documents): the page's address.
    pub url: Option<String>,
    pub frame: Option<PtRect>,
    /// Text the user had selected in it.
    pub selected_text: Option<String>,
}

/// What a marker was put on, for the agent to find it in the code.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Element {
    /// The app, e.g. "Safari".
    pub app: Option<String>,
    /// The window's title.
    pub window: Option<String>,
    /// What the element is, by its role and label: `button "Save"`.
    pub name: Option<String>,
    /// Native apps: the chain of labelled ancestors, outermost first.
    pub path: Option<String>,
    /// Web pages: a CSS-like path from the closest ancestors, e.g. `div.actions > button#save.btn`.
    pub selector: Option<String>,
    pub url: Option<String>,
    pub role: Option<String>,
    pub identifier: Option<String>,
    pub frame: Option<PtRect>,
    /// The labels of the element's ancestors, for context.
    pub nearby: Option<String>,
    /// Text selected in it when the screen froze (a typo to fix, a sentence to rewrite).
    pub selected_text: Option<String>,
}

impl Element {
    /// Identifies an element from its accessibility node and its ancestors (innermost first).
    pub fn from_nodes(app: Option<String>, window: Option<String>, nodes: &[Node]) -> Element {
        // Text or an image inside a control stands for the control (elements are named by what
        // they are, a button "Save", not the text in them).
        let promote = nodes.len() > 1
            && matches!(nodes[0].role.as_str(), "AXStaticText" | "AXImage")
            && CONTROLS.contains(&nodes[1].role.as_str());
        let nodes = if promote { &nodes[1..] } else { nodes };
        let Some(hit) = nodes.first() else { return Element { app, window, ..Element::default() } };
        let web_depth = nodes.iter().position(|n| n.role == "AXWebArea");
        let url = web_depth.and_then(|i| nodes[i].url.clone()).or_else(|| nodes.iter().find_map(|n| n.url.clone()));
        let selector = web_depth.filter(|&d| d > 0).map(|d| {
            let mut parts: Vec<String> = nodes[..d].iter().take(4).map(css_part).collect();
            parts.reverse();
            parts.join(" > ")
        });
        let ancestors = &nodes[1..web_depth.unwrap_or(nodes.len())];
        let labelled: Vec<String> = ancestors
            .iter()
            .filter(|n| n.role != "AXWindow" && n.role != "AXApplication")
            .filter_map(|n| label(n).map(|l| format!("{} {}", role_name(n), quote(&l))))
            .take(4)
            .collect();
        let path = (web_depth.is_none()).then(|| {
            let mut chain: Vec<String> = labelled.iter().rev().cloned().collect();
            chain.push(describe(hit));
            chain.join(" › ")
        });
        let nearby = (!labelled.is_empty() && web_depth.is_some()).then(|| labelled.join(" · "));
        Element {
            app,
            window,
            name: Some(describe(hit)),
            path,
            selector,
            url,
            role: Some(hit.role.clone()).filter(|r| !r.is_empty()),
            identifier: hit.identifier.clone().or_else(|| hit.dom_id.clone()),
            frame: hit.frame,
            nearby,
            selected_text: nodes.iter().find_map(|n| n.selected_text.clone()).filter(|t| !t.trim().is_empty()),
        }
    }
}

/// Roles that name what's inside them.
const CONTROLS: &[&str] = &[
    "AXButton",
    "AXLink",
    "AXHeading",
    "AXMenuItem",
    "AXMenuButton",
    "AXPopUpButton",
    "AXCheckBox",
    "AXRadioButton",
    "AXTab",
    "AXCell",
    "AXDisclosureTriangle",
];

/// `button "Save"`, or just the role when the element has no label.
fn describe(node: &Node) -> String {
    match label(node) {
        Some(l) => format!("{} {}", role_name(node), quote(&l)),
        None => role_name(node),
    }
}

fn label(node: &Node) -> Option<String> {
    [&node.title, &node.description, &node.value, &node.placeholder]
        .into_iter()
        .flatten()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .find(|s| !s.is_empty())
        .map(|s| truncate(&s, 60))
}

/// The role in words: the localized description when the app gives one, else derived from AXRole.
fn role_name(node: &Node) -> String {
    if let Some(desc) = node.role_description.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        return desc.to_string();
    }
    let role = node.role.strip_prefix("AX").unwrap_or(&node.role);
    let mut words = String::new();
    for (i, c) in role.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            words.push(' ');
        }
        words.extend(c.to_lowercase());
    }
    if words.is_empty() { "element".into() } else { words }
}

/// `button#save.btn.primary`: the HTML tag a web element's role stands for, its id and classes.
fn css_part(node: &Node) -> String {
    let tag = match node.role.as_str() {
        "AXButton" => "button".to_string(),
        "AXLink" => "a".into(),
        "AXHeading" => node.level.map_or("h".to_string(), |l| format!("h{l}")),
        "AXImage" => "img".into(),
        "AXTextField" | "AXSearchField" | "AXCheckBox" | "AXRadioButton" | "AXSlider" => "input".into(),
        "AXTextArea" => "textarea".into(),
        "AXPopUpButton" | "AXComboBox" => "select".into(),
        "AXList" => "ul".into(),
        "AXTable" => "table".into(),
        "AXRow" => "tr".into(),
        "AXCell" => "td".into(),
        "AXStaticText" => "text".into(),
        "AXParagraph" => "p".into(),
        "AXForm" => "form".into(),
        "AXGroup" | "AXGenericElement" | "AXSection" => "div".into(),
        _ => match node.subrole.as_deref() {
            Some("AXLandmarkNavigation") => "nav".into(),
            Some("AXLandmarkMain") => "main".into(),
            Some("AXLandmarkBanner") => "header".into(),
            Some("AXLandmarkContentInfo") => "footer".into(),
            _ => role_name(node).replace(' ', "-"),
        },
    };
    let mut part = tag;
    if let Some(id) = node.dom_id.as_deref().filter(|id| !id.is_empty()) {
        part.push('#');
        part.push_str(id);
    }
    for class in node.dom_classes.iter().filter(|c| !c.is_empty()).take(4) {
        part.push('.');
        part.push_str(class);
    }
    part
}

/// One annotation: a numbered marker, on an element or around an area, with the user's feedback.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Annotation {
    pub number: u32,
    /// Which annotation this is for the rest of the session: numbers change when one is removed.
    pub key: u32,
    /// Where the marker is, in points.
    pub at: (f64, f64),
    /// For an area annotation (dragged), the area.
    pub area: Option<PtRect>,
    pub element: Option<Element>,
    /// Typed feedback.
    pub comment: String,
    /// What the user said about it, recorded from its card: verbatim transcripts.
    pub said: Vec<Said>,
    pub intent: Option<Intent>,
    pub severity: Option<Severity>,
    pub look: Option<Look>,
    /// Seconds into the session when it was put down, to place it among what was said.
    pub time: f32,
    /// Drawn with the Draw tool; `element` is what it starts on (an arrow) or circles (a sketch).
    pub shape: Option<Shape>,
    /// What an arrow points to.
    pub target: Option<Element>,
}

/// A recording of what the user said about one annotation (`take`, the app's id for it), and its
/// transcript.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Said {
    pub take: u64,
    pub text: String,
}

impl Annotation {
    /// What the annotation is on, for its heading.
    pub fn title(&self, scale: f64, units: Units) -> String {
        let name = self.element.as_ref().and_then(|e| e.name.clone());
        let point = |(x, y): (f64, f64)| format!("({}, {})", num(x, scale, units), num(y, scale, units));
        match &self.shape {
            Some(Shape::Arrow { from, to }) => {
                let start = name.unwrap_or_else(|| point(*from));
                let end = self.target.as_ref().and_then(|e| e.name.clone()).unwrap_or_else(|| point(*to));
                return format!("Arrow from {start} to {end}");
            }
            Some(Shape::Sketch { points }) => {
                let r = Shape::Sketch { points: points.clone() }.bounds();
                return match name {
                    Some(name) => format!("Drawn on {name}"),
                    None => format!(
                        "Drawn around {} × {} at ({}, {})",
                        num(r.width, scale, units),
                        num(r.height, scale, units),
                        num(r.x, scale, units),
                        num(r.y, scale, units)
                    ),
                };
            }
            None => {}
        }
        if let Some(name) = name {
            return name;
        }
        match self.area {
            Some(r) => format!(
                "Area {} × {} at ({}, {})",
                num(r.width, scale, units),
                num(r.height, scale, units),
                num(r.x, scale, units),
                num(r.y, scale, units)
            ),
            None => format!("Point {}", point(self.at)),
        }
    }

    /// The part of the screen it annotates: its area, its drawing, or the element it's on.
    pub fn region(&self) -> Option<PtRect> {
        self.area.or(self.shape.as_ref().map(Shape::bounds)).or(self.element.as_ref().and_then(|e| e.frame))
    }
}

/// Everything from one screen in one session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inspection {
    pub screen: Screen,
    /// Where the user was on the screen, said when the annotations don't say it.
    #[serde(default)]
    pub place: Option<String>,
    #[serde(default)]
    pub units: Units,
    #[serde(default)]
    pub measurements: Vec<Measurement>,
    /// When each measurement was made, in seconds into the session.
    #[serde(default)]
    pub measurement_times: Vec<f32>,
    #[serde(default)]
    pub guides: Vec<Guide>,
    #[serde(default)]
    pub annotations: Vec<Annotation>,
}

impl Inspection {
    pub fn is_empty(&self) -> bool {
        self.measurements.is_empty() && self.guides.is_empty() && self.annotations.is_empty()
    }

    /// The part of the screen holding everything, padded by `padding` points, within the screen;
    /// the whole screen when there's nothing (or only guides, which span it).
    pub fn bounds(&self, padding: f64) -> PtRect {
        let full = PtRect::new(0.0, 0.0, self.screen.width, self.screen.height);
        let mut parts = self.measurements.iter().map(Measurement::bounds).chain(self.annotations.iter().map(|a| {
            let marker = PtRect::new(a.at.0 - 12.0, a.at.1 - 12.0, 24.0, 24.0);
            let mut around = a.region().map_or(marker, |r| r.union(&marker));
            if let Some(frame) = a.target.as_ref().and_then(|e| e.frame) {
                around = around.union(&frame);
            }
            around
        }));
        let Some(first) = parts.next() else { return full };
        let all = parts.fold(first, |acc, r| acc.union(&r));
        let x = (all.x - padding).max(0.0);
        let y = (all.y - padding).max(0.0);
        PtRect::new(x, y, (all.right() + padding).min(full.width) - x, (all.bottom() + padding).min(full.height) - y)
    }

    /// What goes with the screenshot about the measurements: their units and screen (the screen's
    /// left out at compact detail), and the guides. Each measurement is a line of its own
    /// (`measurement_lines`), placed when it was made.
    pub fn measurements_markdown(&self, detail: Detail) -> Option<String> {
        if self.measurements.is_empty() && self.guides.is_empty() {
            return None;
        }
        let (scale, units) = (self.screen.scale, self.units);
        let mut text = if detail == Detail::Compact {
            format!("**Measured** in {}", units.label())
        } else {
            let mut screen =
                format!("{} × {} pt, @{}x", n_pt(self.screen.width), n_pt(self.screen.height), n_pt(scale));
            if let Some(name) = &self.screen.name {
                screen = format!("{name}, {screen}");
            }
            format!("**Measured** in {} (screen: {screen})", units.label())
        };
        let guides = guides_text(&self.guides, scale, units);
        if !guides.is_empty() {
            text.push_str(&format!("\n**Guides:** {guides}"));
        }
        Some(text)
    }

    /// Each measurement as a list item: `- Area 120 × 32 (15:4) at (380, 200)`.
    pub fn measurement_lines(&self) -> Vec<String> {
        let (scale, units) = (self.screen.scale, self.units);
        let n = |v: f64| num(v, scale, units);
        self.measurements
            .iter()
            .map(|m| match m {
                Measurement::Distance { x, y, across, down } => {
                    let run = |r: &Run, axis: &str, direction: &str, before: &str, after: &str| {
                        let start =
                            if r.open_start { format!("{before} edge") } else { format!("{axis} {}", n(r.start)) };
                        let end = if r.open_end { format!("{after} edge") } else { format!("{axis} {}", n(r.end)) };
                        format!("{} {direction} ({start} to {end})", n(r.len()))
                    };
                    let runs: Vec<String> = [
                        (!across.is_empty()).then(|| run(across, "x", "across", "screen's left", "screen's right")),
                        (!down.is_empty()).then(|| run(down, "y", "down", "screen's top", "screen's bottom")),
                    ]
                    .into_iter()
                    .flatten()
                    .collect();
                    format!("- Distance from ({}, {}): {}", n(*x), n(*y), runs.join(", "))
                }
                Measurement::Area { rect, snapped } => {
                    let ratio = crate::measure::aspect_ratio(rect.width, rect.height)
                        .filter(|r| r != "1:1" || rect.width >= 8.0)
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default();
                    format!(
                        "- Area {} × {}{ratio} at ({}, {}){}",
                        n(rect.width),
                        n(rect.height),
                        n(rect.x),
                        n(rect.y),
                        if *snapped { ", snapped to the object inside" } else { "" }
                    )
                }
            })
            .collect()
    }

    /// One annotation, as its own block: `### N. what it's on` and what the agent needs to find
    /// and fix it, at the chosen detail.
    /// Where every annotation is, when they're all in the same app, window and page: said once (in
    /// [`Inspection::context_markdown`]) instead of under each annotation.
    pub fn shared_context(&self) -> Option<(String, Option<String>, Option<String>)> {
        let mut places = self.annotations.iter().map(|a| {
            let e = a.element.as_ref()?;
            Some((e.app.clone()?, e.window.clone().filter(|w| !w.is_empty()), e.url.clone()))
        });
        let first = places.next()??;
        places.all(|p| p.as_ref() == Some(&first)).then_some(first)
    }

    /// `**In:** Safari · "Settings" · http://localhost:3000/settings`, when all annotations share it,
    /// or where the user was on the screen when it froze, when no annotation says otherwise.
    pub fn context_markdown(&self) -> Option<String> {
        match self.shared_context() {
            Some((app, window, url)) => Some(place_markdown(&app, window.as_deref(), url.as_deref())),
            None if self.annotations.iter().all(|a| a.element.as_ref().and_then(|e| e.app.as_ref()).is_none()) => {
                self.place.clone()
            }
            None => None,
        }
    }

    pub fn annotation_markdown(&self, annotation: &Annotation, detail: Detail) -> String {
        let (scale, units) = (self.screen.scale, self.units);
        let n = |v: f64| num(v, scale, units);
        let title = annotation.title(scale, units);
        let element = annotation.element.clone().unwrap_or_default();
        let comment = annotation.comment.trim();
        let said =
            annotation.said.iter().map(|s| s.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ");
        // Said once for all of them.
        let shared = self.shared_context().is_some();
        if detail == Detail::Compact {
            let mut line = format!("{}. **{}**", annotation.number, title);
            if let Some(app) = element.app.as_ref().filter(|_| !shared) {
                line.push_str(&format!(" ({app})"));
            }
            if !comment.is_empty() {
                line.push_str(&format!(": {comment}"));
            }
            return with_said(&line, &said, detail);
        }
        let mut lines = vec![format!("### {}. {title}", annotation.number)];
        let mut field = |label: &str, value: String| lines.push(format!("**{label}:** {value}"));
        match (&element.app, &element.window) {
            _ if shared => {}
            (Some(app), Some(window)) if !window.is_empty() => field("App", format!("{app} · {}", quote(window))),
            (Some(app), _) => field("App", app.clone()),
            _ => {}
        }
        if let Some(url) = element.url.as_ref().filter(|_| !shared) {
            field("Page", url.clone());
        }
        if let Some(location) = element.selector.clone().or(element.path.clone()) {
            field("Location", location);
        }
        // The title names both ends; this says where they are (and the target's app, if another).
        if let Some(Shape::Arrow { from, to }) = &annotation.shape {
            let other_app = annotation
                .target
                .as_ref()
                .and_then(|t| t.app.clone())
                .filter(|app| Some(app) != element.app.as_ref())
                .map(|app| format!(" in {app}"))
                .unwrap_or_default();
            field("From → to", format!("({}, {}) → ({}, {}){other_app}", n(from.0), n(from.1), n(to.0), n(to.1)));
        }
        if let Some(selected) = &element.selected_text {
            field("Selected text", quote(selected.trim()));
        }
        if matches!(detail, Detail::Detailed | Detail::Forensic) {
            if let Some(r) = annotation.region() {
                field(
                    "Position",
                    format!("{}, {} ({} × {} {})", n(r.x), n(r.y), n(r.width), n(r.height), units.label()),
                );
            }
            if let Some(nearby) = &element.nearby {
                field("Context", nearby.clone());
            }
        }
        if detail == Detail::Forensic {
            if let Some(look) = &annotation.look {
                let mut parts = Vec::new();
                if let Some(bg) = &look.background {
                    parts.push(format!("background {bg}"));
                }
                if let Some(fg) = &look.foreground {
                    parts.push(format!("content {fg}"));
                }
                if let Some([t, r, b, l]) = look.padding {
                    parts.push(format!("padding {} {} {} {} {}", n(t), n(r), n(b), n(l), units.label()));
                }
                if !parts.is_empty() {
                    field("Style", parts.join(", "));
                }
            }
            let mut ax = Vec::new();
            if let Some(role) = &element.role {
                ax.push(format!("role {role}"));
            }
            if let Some(id) = &element.identifier {
                ax.push(format!("identifier {}", quote(id)));
            }
            if !ax.is_empty() {
                field("Accessibility", ax.join(", "));
            }
            field("Marker at", format!("({}, {})", n(annotation.at.0), n(annotation.at.1)));
        }
        let mut tags = Vec::new();
        if let Some(intent) = annotation.intent {
            tags.push(format!("**Intent:** {}", intent.as_str()));
        }
        if let Some(severity) = annotation.severity {
            tags.push(format!("**Severity:** {}", severity.as_str()));
        }
        if !tags.is_empty() {
            lines.push(tags.join(" · "));
        }
        if !comment.is_empty() {
            lines.push(format!("**Feedback:** {comment}"));
        }
        with_said(&lines.join("\n"), &said, detail)
    }
}

/// An annotation's block with what the user said about it (a verbatim transcript) added:
/// `**Said:** …`, or at compact detail `(said: "…")` at the end of its line.
pub fn with_said(block: &str, said: &str, detail: Detail) -> String {
    let said = said.trim();
    if said.is_empty() {
        return block.to_string();
    }
    if detail == Detail::Compact {
        return format!("{block} (said: \"{said}\")");
    }
    // A second recording continues the first.
    match block.rsplit_once('\n').map_or(block, |(_, last)| last).starts_with("**Said:** ") {
        true => format!("{block} {said}"),
        false => format!("{block}\n**Said:** {said}"),
    }
}

/// `x 380 · x 700 (320 apart) · y 120`: guides by axis, with the distance between neighbours.
fn guides_text(guides: &[Guide], scale: f64, units: Units) -> String {
    let mut parts = Vec::new();
    for axis in [Axis::X, Axis::Y] {
        let mut positions: Vec<f64> = guides.iter().filter(|g| g.axis == axis).map(|g| g.position).collect();
        positions.sort_by(f64::total_cmp);
        let name = if axis == Axis::X { "x" } else { "y" };
        for (i, p) in positions.iter().enumerate() {
            let mut part = format!("{name} {}", num(*p, scale, units));
            if i > 0 {
                part.push_str(&format!(" ({} apart)", num(p - positions[i - 1], scale, units)));
            }
            parts.push(part);
        }
    }
    parts.join(" · ")
}

/// A length in the chosen units, rounded to a tenth (whole numbers without the decimal).
pub fn num(points: f64, scale: f64, units: Units) -> String {
    let value = match units {
        Units::Pt => points,
        Units::Px => points * scale,
    };
    n_pt(value)
}

fn n_pt(value: f64) -> String {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded.fract() == 0.0 { format!("{}", rounded as i64) } else { format!("{rounded:.1}") }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "'"))
}

/// Where something was, in an app: `**In:** Safari · "Settings — Shop" · http://localhost:3000/settings`.
pub fn place_markdown(app: &str, window: Option<&str>, url: Option<&str>) -> String {
    let mut parts = vec![app.to_string()];
    parts.extend(window.filter(|w| !w.trim().is_empty()).map(quote));
    parts.extend(url.filter(|u| !u.is_empty()).map(String::from));
    format!("{PLACE}{}", parts.join(" · "))
}

/// How a place begins (see [`place_markdown`]).
pub const PLACE: &str = "**In:** ";

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen() -> Screen {
        Screen { name: Some("Built-in Retina Display".into()), width: 1512.0, height: 982.0, scale: 2.0 }
    }

    fn web_button() -> Vec<Node> {
        vec![
            Node {
                role: "AXButton".into(),
                role_description: Some("button".into()),
                title: Some("Save  changes".into()),
                dom_id: Some("save".into()),
                dom_classes: vec!["btn".into(), "btn-primary".into()],
                frame: Some(PtRect::new(380.0, 200.0, 120.0, 32.0)),
                ..Node::default()
            },
            Node { role: "AXGroup".into(), dom_classes: vec!["actions".into()], ..Node::default() },
            Node {
                role: "AXForm".into(),
                description: Some("Settings".into()),
                dom_classes: vec!["settings".into()],
                ..Node::default()
            },
            Node {
                role: "AXWebArea".into(),
                title: Some("Settings — Shop".into()),
                url: Some("http://localhost:3000/settings".into()),
                ..Node::default()
            },
            Node { role: "AXScrollArea".into(), ..Node::default() },
            Node { role: "AXWindow".into(), title: Some("Settings — Shop".into()), ..Node::default() },
        ]
    }

    #[test]
    fn identifies_web_elements_by_selector() {
        let e = Element::from_nodes(Some("Safari".into()), Some("Settings — Shop".into()), &web_button());
        assert_eq!(e.name.as_deref(), Some("button \"Save changes\""));
        assert_eq!(e.selector.as_deref(), Some("form.settings > div.actions > button#save.btn.btn-primary"));
        assert_eq!(e.url.as_deref(), Some("http://localhost:3000/settings"));
        assert_eq!(e.path, None);
        assert_eq!(e.nearby.as_deref(), Some("form \"Settings\""));
        assert_eq!(e.identifier.as_deref(), Some("save"));
        assert_eq!(e.selected_text, None);
        let mut typo = web_button();
        typo[1].selected_text = Some("Sve chnages".into());
        let e = Element::from_nodes(None, None, &typo);
        let i = Inspection { annotations: vec![], ..inspection() };
        let a = Annotation { number: 1, element: Some(e), ..Annotation::default() };
        assert!(i.annotation_markdown(&a, Detail::Standard).contains("**Selected text:** \"Sve chnages\""));
    }

    #[test]
    fn identifies_native_elements_by_their_labelled_ancestors() {
        let nodes = vec![
            Node { role: "AXCheckBox".into(), title: Some("Show hidden files".into()), ..Node::default() },
            Node { role: "AXGroup".into(), ..Node::default() },
            Node { role: "AXTabGroup".into(), title: Some("Advanced".into()), ..Node::default() },
            Node { role: "AXWindow".into(), title: Some("Settings".into()), ..Node::default() },
        ];
        let e = Element::from_nodes(Some("Finder".into()), Some("Settings".into()), &nodes);
        assert_eq!(e.name.as_deref(), Some("check box \"Show hidden files\""));
        assert_eq!(e.path.as_deref(), Some("tab group \"Advanced\" › check box \"Show hidden files\""));
        assert_eq!(e.selector, None);
        let text = Node { role: "AXStaticText".into(), value: Some("Save".into()), ..Node::default() };
        let link =
            Node { role: "AXLink".into(), title: Some("Save".into()), dom_id: Some("s".into()), ..Node::default() };
        let promoted = Element::from_nodes(None, None, &[text, link]);
        assert_eq!((promoted.name.as_deref(), promoted.identifier.as_deref()), (Some("link \"Save\""), Some("s")));
        let bare = Element::from_nodes(Some("App".into()), None, &[Node { role: "AXGroup".into(), ..Node::default() }]);
        assert_eq!(bare.name.as_deref(), Some("group"));
        assert_eq!(Element::from_nodes(None, None, &[]).name, None);
    }

    fn inspection() -> Inspection {
        Inspection {
            place: None,
            screen: screen(),
            units: Units::Pt,
            measurements: vec![
                Measurement::Distance {
                    x: 412.0,
                    y: 230.0,
                    across: Run { start: 400.0, end: 424.0, ..Run::default() },
                    down: Run { start: 0.0, end: 238.5, open_start: true, open_end: false },
                },
                Measurement::Area { rect: PtRect::new(380.0, 200.0, 320.0, 48.0), snapped: true },
            ],
            measurement_times: vec![1.0, 4.0],
            guides: vec![
                Guide { axis: Axis::X, position: 700.0 },
                Guide { axis: Axis::Y, position: 120.0 },
                Guide { axis: Axis::X, position: 380.0 },
            ],
            annotations: vec![Annotation {
                number: 1,
                at: (412.0, 216.0),
                element: Some(Element::from_nodes(
                    Some("Safari".into()),
                    Some("Settings — Shop".into()),
                    &web_button(),
                )),
                comment: " Line it up with the fields. ".into(),
                intent: Some(Intent::Fix),
                severity: Some(Severity::Important),
                look: Some(Look {
                    background: Some("#2563EB".into()),
                    foreground: Some("#FFFFFF".into()),
                    padding: Some([6.0, 12.0, 6.0, 12.0]),
                }),
                ..Annotation::default()
            }],
        }
    }

    #[test]
    fn writes_measurements() {
        let i = inspection();
        assert_eq!(
            i.measurements_markdown(Detail::Standard).unwrap(),
            "**Measured** in pt (screen: Built-in Retina Display, 1512 × 982 pt, @2x)\n\
             **Guides:** x 380 · x 700 (320 apart) · y 120"
        );
        assert_eq!(
            i.measurement_lines(),
            [
                "- Distance from (412, 230): 24 across (x 400 to x 424), 238.5 down (screen's top edge to y 238.5)",
                "- Area 320 × 48 (20:3) at (380, 200), snapped to the object inside"
            ]
        );
        let px = Inspection { units: Units::Px, ..inspection() };
        assert_eq!(
            px.measurements_markdown(Detail::Compact).unwrap(),
            "**Measured** in px\n**Guides:** x 760 · x 1400 (640 apart) · y 240"
        );
        assert_eq!(px.measurement_lines()[1], "- Area 640 × 96 (20:3) at (760, 400), snapped to the object inside");
        let nothing = Inspection { measurements: vec![], guides: vec![], ..inspection() };
        assert_eq!(nothing.measurements_markdown(Detail::Standard), None);
        assert_eq!(i.measurements[1].dimensions(2.0, Units::Px), "640 × 96");
    }

    #[test]
    fn writes_annotations_at_each_detail_level() {
        let i = inspection();
        let a = &i.annotations[0];
        // Every annotation is in the same app, window and page: that's said once, not under each.
        assert_eq!(
            i.context_markdown().as_deref(),
            Some("**In:** Safari · \"Settings — Shop\" · http://localhost:3000/settings")
        );
        assert_eq!(
            i.annotation_markdown(a, Detail::Compact),
            "1. **button \"Save changes\"**: Line it up with the fields."
        );
        assert_eq!(
            i.annotation_markdown(a, Detail::Standard),
            "### 1. button \"Save changes\"\n\
             **Location:** form.settings > div.actions > button#save.btn.btn-primary\n\
             **Intent:** fix · **Severity:** important\n\
             **Feedback:** Line it up with the fields."
        );
        // Annotations in different apps each say where they are.
        let mut finder = a.clone();
        finder.number = 2;
        finder.element = Some(Element { app: Some("Finder".into()), name: Some("row".into()), ..Element::default() });
        let mixed = Inspection { annotations: vec![a.clone(), finder.clone()], ..inspection() };
        assert_eq!(mixed.context_markdown(), None);
        // Where the user was, when no annotation says where it is.
        let placed = |annotations| Inspection {
            annotations,
            place: Some("**In:** Simulator · \"iPhone 17\"".into()),
            ..inspection()
        };
        assert_eq!(placed(vec![]).context_markdown().as_deref(), Some("**In:** Simulator · \"iPhone 17\""));
        assert_eq!(placed(vec![a.clone(), finder.clone()]).context_markdown(), None, "annotations in different apps");
        assert!(
            mixed.annotation_markdown(a, Detail::Standard).contains("**App:** Safari · \"Settings — Shop\"\n**Page:**")
        );
        assert_eq!(
            mixed.annotation_markdown(&finder, Detail::Compact),
            "2. **row** (Finder): Line it up with the fields."
        );
        let forensic = i.annotation_markdown(a, Detail::Forensic);
        assert!(
            forensic.contains("**Position:** 380, 200 (120 × 32 pt)\n**Context:** form \"Settings\""),
            "{forensic}"
        );
        assert!(
            forensic.contains("**Style:** background #2563EB, content #FFFFFF, padding 6 12 6 12 pt"),
            "{forensic}"
        );
        assert!(forensic.contains("**Accessibility:** role AXButton, identifier \"save\"\n**Marker at:** (412, 216)"));
        let spoken = Annotation {
            number: 2,
            at: (10.0, 20.0),
            area: Some(PtRect::new(5.0, 5.0, 100.5, 40.0)),
            ..Annotation::default()
        };
        assert_eq!(
            i.annotation_markdown(&spoken, Detail::Detailed),
            "### 2. Area 100.5 × 40 at (5, 5)\n**Position:** 5, 5 (100.5 × 40 pt)"
        );
        assert_eq!(Annotation { at: (1.0, 2.0), ..Annotation::default() }.title(2.0, Units::Px), "Point (2, 4)");
        // What was said about it, after what was typed; a later recording continues it.
        let said = Annotation {
            said: vec![Said { take: 4, text: " Make it bigger. ".into() }, Said { take: 6, text: "Much.".into() }],
            ..a.clone()
        };
        assert!(
            i.annotation_markdown(&said, Detail::Standard)
                .ends_with("**Feedback:** Line it up with the fields.\n**Said:** Make it bigger. Much.")
        );
        assert_eq!(
            i.annotation_markdown(&said, Detail::Compact),
            "1. **button \"Save changes\"**: Line it up with the fields. (said: \"Make it bigger. Much.\")"
        );
        assert_eq!(with_said("### 3. row", "  ", Detail::Standard), "### 3. row");
    }

    #[test]
    fn bounds_cover_everything_with_padding() {
        let i = inspection();
        let b = i.bounds(20.0);
        assert_eq!((b.x, b.y), (360.0, 0.0));
        assert_eq!((b.right(), b.bottom()), (720.0, 268.0));
        let empty = Inspection { measurements: vec![], annotations: vec![], ..inspection() };
        assert_eq!(empty.bounds(20.0), PtRect::new(0.0, 0.0, 1512.0, 982.0));
        assert_eq!(Intent::parse(" Question"), Some(Intent::Question));
        assert_eq!(Severity::parse("blocker"), None);
    }
}
