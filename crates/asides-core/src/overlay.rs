//! The state of a Measure & Annotate session, apart from any UI so it can be tested: the overlay
//! feeds it mouse and key events (in points, per screen) and draws what it holds.
//!
//! Four tools share one session:
//! - **Measure** (PixelSnap): moving shows the distances from the cursor to the nearest edges, a
//!   click holds them (H or V holds one axis), a drag measures an area, snapped to the object
//!   inside (right-drag or ⌘ doesn't snap; ⌥-drag holds each object inside). Guides (⇧H, ⇧V) go
//!   through the cursor, snapped to held objects and to the nearest edge; distances stop at guides
//!   and at held objects.
//! - **Annotate**: a click puts a numbered marker on the element under it, or annotates the held
//!   area (a measured section) it's in; a drag annotates an area.
//! - **Draw**: a drag sketches (circle or underline something), a ⇧-drag draws an arrow, numbered
//!   like markers.
//! - **Shot**: a drag takes a screenshot of that part of the screen, then the tool goes back.
//!
//! Annotations and measurements carry when they were made, so they can sit among what the user
//! said.

use serde::{Deserialize, Serialize};

use crate::inspect::{
    Annotation, Axis, Detail, Element, Guide, Inspection, Intent, Look, Measurement, PtRect, Run, Screen, Severity,
    Shape, Units,
};
use crate::measure::{self, Frame, Rect, Stops, Tolerance};

/// A drag shorter than this (in points) is a click.
const CLICK_SLOP: f64 = 3.0;
/// How far (in points) a guide looks for an edge to snap to.
const GUIDE_SNAP: f64 = 6.0;
/// Points closer than this in a sketch are dropped.
const SKETCH_STEP: f64 = 2.0;
/// Objects closer than this (in points) are one object for ⌥-drag.
const OBJECT_GAP: f64 = 4.0;
/// Markers are circles of this diameter (points).
pub const MARKER: f64 = 22.0;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    #[default]
    Measure,
    Annotate,
    Draw,
    Shot,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Measure, Tool::Annotate, Tool::Draw, Tool::Shot];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Measure => "Measure",
            Tool::Annotate => "Annotate",
            Tool::Draw => "Draw",
            Tool::Shot => "Shot",
        }
    }
}

/// One screen: its frozen frame and what's on it.
#[derive(Debug, Clone)]
pub struct ScreenState {
    pub screen: Screen,
    pub frame: Frame,
    /// Where the user was on it when it froze (`helper::SpecScreen::place`).
    pub place: Option<String>,
    pub guides: Vec<Guide>,
    pub held: Vec<Measurement>,
    /// When each held measurement was made, in seconds into the session.
    pub held_times: Vec<f32>,
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Drag {
    pub screen: usize,
    pub start: (f64, f64),
    pub current: (f64, f64),
    /// Snap the area to the object inside (not for a right-drag).
    pub snap: bool,
    /// Draw: the sketch so far.
    pub points: Vec<(f64, f64)>,
}

impl Drag {
    pub fn rect(&self) -> PtRect {
        let (x0, x1) = (self.start.0.min(self.current.0), self.start.0.max(self.current.0));
        let (y0, y1) = (self.start.1.min(self.current.1), self.start.1.max(self.current.1));
        PtRect::new(x0, y0, x1 - x0, y1 - y0)
    }

    pub fn is_click(&self) -> bool {
        (self.current.0 - self.start.0).abs() < CLICK_SLOP && (self.current.1 - self.start.1).abs() < CLICK_SLOP
    }
}

/// A screenshot taken during the session (Shot tool, or a held object's camera).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Shot {
    pub screen: usize,
    pub rect: PtRect,
    /// Seconds into the session.
    pub time: f32,
}

/// What a mouse-up did, for the overlay to follow up on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Placed {
    Nothing,
    Measurement {
        screen: usize,
    },
    /// A new annotation: identify what's under it and (unless the user is talking) ask for feedback.
    Annotation {
        screen: usize,
        index: usize,
    },
    /// A click on an existing marker: edit its feedback.
    Edit {
        screen: usize,
        index: usize,
    },
    /// A screenshot to take (the last of `shots`).
    Shot {
        screen: usize,
        rect: PtRect,
    },
}

/// Which part of an annotation an identified element describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Part {
    /// What it's on (a marker, an arrow's start, what a sketch or an area surrounds).
    Element,
    /// Where an arrow points.
    Target,
}

/// Where to look up the element for a new annotation: `(part, x, y)`, in the screen's points.
pub fn lookups(annotation: &Annotation) -> Vec<(Part, f64, f64)> {
    match (&annotation.shape, annotation.area) {
        (Some(Shape::Arrow { from, to }), _) => vec![(Part::Element, from.0, from.1), (Part::Target, to.0, to.1)],
        (Some(shape), _) => {
            let r = shape.bounds();
            vec![(Part::Element, r.x + r.width / 2.0, r.y + r.height / 2.0)]
        }
        (None, Some(r)) => vec![(Part::Element, r.x + r.width / 2.0, r.y + r.height / 2.0)],
        (None, None) => vec![(Part::Element, annotation.at.0, annotation.at.1)],
    }
}

/// Something added, for undo (Delete removes the last one).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Added {
    Held(usize),
    Guide(usize),
    Annotation(usize),
}

/// Guides and held measurements, kept from one session to restore in the next (R).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    pub guides: Vec<Vec<Guide>>,
    pub held: Vec<Vec<Measurement>>,
}

impl Saved {
    pub fn is_empty(&self) -> bool {
        self.guides.iter().all(Vec::is_empty) && self.held.iter().all(Vec::is_empty)
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub screens: Vec<ScreenState>,
    pub tool: Tool,
    /// The tool before Shot, to go back to after the screenshot.
    pub previous_tool: Tool,
    pub tolerance: Tolerance,
    /// The tolerance before T switched to high, to switch back.
    pub normal_tolerance: Tolerance,
    pub units: Units,
    /// Screen and position in points.
    pub cursor: Option<(usize, f64, f64)>,
    /// Shift: a crosshair, a square area, a straight arrow.
    pub crosshair: bool,
    /// ⌘ held: measure without snapping.
    pub snapping: bool,
    /// ⌥ held: drag holds each object inside; screenshots without padding.
    pub alternate: bool,
    pub drag: Option<Drag>,
    /// Space held while dragging: move the area instead of resizing it.
    pub moving: bool,
    /// Tab: everything hidden, to see the frozen screen as it was.
    pub hidden: bool,
    /// Annotate: the element found under the pointer (Identify UI Elements), in its screen's
    /// points, so a click annotates it rather than a larger section around it.
    pub element: Option<(usize, PtRect)>,
    pub shots: Vec<Shot>,
    /// Seconds into the session as of what the user last did: what's added is stamped with it.
    pub clock: f32,
    added: Vec<(usize, Added)>,
    next_number: u32,
    /// Annotations' keys: unlike their numbers, never reused.
    next_key: u32,
}

impl Session {
    pub fn new(screens: Vec<ScreenState>, tolerance: Tolerance, units: Units) -> Session {
        Session {
            screens,
            tool: Tool::Measure,
            previous_tool: Tool::Measure,
            tolerance,
            normal_tolerance: tolerance,
            units,
            cursor: None,
            crosshair: false,
            snapping: true,
            alternate: false,
            drag: None,
            moving: false,
            hidden: false,
            element: None,
            shots: Vec::new(),
            clock: 0.0,
            added: Vec::new(),
            next_number: 1,
            next_key: 1,
        }
    }

    /// The next freeze of the same session: the screens as they are now, nothing on them yet;
    /// the tool, the settings, the clock and the numbering go on.
    pub fn refrozen(&self, screens: Vec<ScreenState>) -> Session {
        Session {
            tool: if self.tool == Tool::Shot { self.previous_tool } else { self.tool },
            previous_tool: self.previous_tool,
            tolerance: self.tolerance,
            normal_tolerance: self.normal_tolerance,
            units: self.units,
            clock: self.clock,
            next_number: self.next_number,
            next_key: self.next_key,
            ..Session::new(screens, self.tolerance, self.units)
        }
    }

    /// A frozen screen as the app described it, and its pixels.
    pub fn from_spec(spec: &crate::helper::SpecScreen, frame: Frame) -> ScreenState {
        ScreenState { place: spec.place.clone(), ..Session::screen(spec.screen.clone(), frame) }
    }

    pub fn screen(screen: Screen, frame: Frame) -> ScreenState {
        ScreenState {
            screen,
            frame,
            place: None,
            guides: Vec::new(),
            held: Vec::new(),
            held_times: Vec::new(),
            annotations: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.screens.iter().all(|s| s.held.is_empty() && s.guides.is_empty() && s.annotations.is_empty())
    }

    pub fn set_tool(&mut self, tool: Tool) {
        if tool == Tool::Shot && self.tool != Tool::Shot {
            self.previous_tool = self.tool;
        }
        self.tool = tool;
        self.drag = None;
    }

    /// The pixel of the frame under a point (clamped to the frame).
    fn pixel(state: &ScreenState, x: f64, y: f64) -> (u32, u32) {
        let s = state.screen.scale;
        let px = ((x * s).floor().max(0.0) as u32).min(state.frame.width.saturating_sub(1));
        let py = ((y * s).floor().max(0.0) as u32).min(state.frame.height.saturating_sub(1));
        (px, py)
    }

    /// The distances from `(x, y)` on `screen` to the nearest edges, stopping at guides and at
    /// held objects.
    pub fn distance(&self, screen: usize, x: f64, y: f64) -> Option<Measurement> {
        let state = self.screens.get(screen)?;
        let s = state.screen.scale;
        let (px, py) = Session::pixel(state, x, y);
        let boundary = |p: f64| (p * s).round().max(0.0) as u32;
        let stops = Stops {
            x: state.guides.iter().filter(|g| g.axis == Axis::X).map(|g| boundary(g.position)).collect(),
            y: state.guides.iter().filter(|g| g.axis == Axis::Y).map(|g| boundary(g.position)).collect(),
        };
        let d = measure::distance_at(&state.frame, px, py, self.tolerance, &stops)?;
        let run = |span: measure::Span, limit: u32| {
            let (open_start, open_end) = span.open(limit);
            Run { start: span.start as f64 / s, end: span.end as f64 / s, open_start, open_end }
        };
        let (x, y) = (px as f64 / s, py as f64 / s);
        let mut across = run(d.horizontal, state.frame.width);
        let mut down = run(d.vertical, state.frame.height);
        for held in &state.held {
            let Measurement::Area { rect, .. } = held else { continue };
            stop_at(&mut across, x, y, rect.x, rect.right(), rect.y, rect.bottom());
            stop_at(&mut down, y, x, rect.y, rect.bottom(), rect.x, rect.right());
        }
        Some(Measurement::Distance { x, y, across, down })
    }

    /// What the cursor measures right now: the distances from it, with the Measure tool, while not
    /// dragging.
    pub fn hover(&self) -> Option<Measurement> {
        let (screen, x, y) = self.cursor?;
        (self.tool == Tool::Measure && self.drag.is_none() && !self.hidden).then(|| self.distance(screen, x, y))?
    }

    /// The area being dragged, snapped as it would be on release (Measure), or as drawn.
    pub fn drag_area(&self) -> Option<(usize, PtRect, bool)> {
        let drag = self.drag.as_ref()?;
        let rect = drag.rect();
        if drag.is_click() || self.tool == Tool::Draw {
            return None;
        }
        if !(drag.snap && self.snapping) || self.tool != Tool::Measure || self.alternate {
            return Some((drag.screen, rect, false));
        }
        Some(match self.snap(drag.screen, rect) {
            Some(snapped) => (drag.screen, snapped, true),
            None => (drag.screen, rect, false),
        })
    }

    /// ⌥-drag with the Measure tool: the objects inside the area being dragged.
    pub fn drag_objects(&self) -> Vec<PtRect> {
        match (&self.drag, self.tool, self.alternate) {
            (Some(drag), Tool::Measure, true) if !drag.is_click() => self.objects(drag.screen, drag.rect()),
            _ => Vec::new(),
        }
    }

    /// The separate objects inside `rect`, largest first.
    pub fn objects(&self, screen: usize, rect: PtRect) -> Vec<PtRect> {
        let Some(state) = self.screens.get(screen) else { return Vec::new() };
        let s = state.screen.scale;
        let (x0, y0) = Session::pixel(state, rect.x, rect.y);
        let (x1, y1) = Session::pixel(state, rect.right(), rect.bottom());
        let gap = (OBJECT_GAP * s).round() as u32;
        measure::objects(&state.frame, Rect::from_corners((x0, y0), (x1, y1)), self.tolerance, gap)
            .into_iter()
            .map(|r| px_rect_to_pt(r, s))
            .collect()
    }

    /// `rect` (points) shrunk to the object inside it, if there is one.
    fn snap(&self, screen: usize, rect: PtRect) -> Option<PtRect> {
        let state = self.screens.get(screen)?;
        let s = state.screen.scale;
        let (x0, y0) = Session::pixel(state, rect.x, rect.y);
        let (x1, y1) = Session::pixel(state, rect.right(), rect.bottom());
        let snapped = measure::snap_area(&state.frame, Rect::from_corners((x0, y0), (x1, y1)), self.tolerance)?;
        Some(px_rect_to_pt(snapped, s))
    }

    /// The arrow being drawn (⇧-drag with Draw): from where the drag started to the cursor.
    pub fn drawing_arrow(&self) -> bool {
        self.tool == Tool::Draw && self.crosshair
    }

    pub fn move_to(&mut self, screen: usize, x: f64, y: f64) {
        if self.moving
            && let (Some(drag), Some((_, cx, cy))) = (&mut self.drag, self.cursor)
        {
            let (dx, dy) = (x - cx, y - cy);
            drag.start = (drag.start.0 + dx, drag.start.1 + dy);
        }
        self.cursor = Some((screen, x, y));
        let square = self.crosshair && !self.moving && matches!(self.tool, Tool::Measure | Tool::Annotate | Tool::Shot);
        if let Some(drag) = &mut self.drag
            && drag.screen == screen
        {
            drag.current = (x, y);
            if self.tool == Tool::Draw {
                let far = drag.points.last().is_none_or(|p| (p.0 - x).hypot(p.1 - y) >= SKETCH_STEP);
                if far {
                    drag.points.push((x, y));
                }
            }
            // Shift while dragging: a perfect square.
            if square {
                let (dx, dy) = (x - drag.start.0, y - drag.start.1);
                let side = dx.abs().max(dy.abs());
                drag.current = (drag.start.0 + side.copysign(dx), drag.start.1 + side.copysign(dy));
            }
        }
    }

    pub fn mouse_down(&mut self, screen: usize, x: f64, y: f64, right: bool) {
        self.move_to(screen, x, y);
        self.drag = Some(Drag { screen, start: (x, y), current: (x, y), snap: !right, points: vec![(x, y)] });
    }

    /// Ends a click or drag, adding what it measured, annotated, drew or shot.
    pub fn mouse_up(&mut self, screen: usize, x: f64, y: f64, time: f32) -> Placed {
        self.clock = time;
        self.move_to(screen, x, y);
        let area = self.drag_area();
        let objects = self.drag_objects();
        let Some(drag) = self.drag.take() else { return Placed::Nothing };
        self.moving = false;
        let screen = drag.screen;
        // A sketch can end where it started (a circle): it's a click only if it stayed put.
        let click = match self.tool {
            Tool::Draw => {
                let r = Shape::Sketch { points: drag.points.clone() }.bounds().union(&drag.rect());
                r.width < CLICK_SLOP && r.height < CLICK_SLOP
            }
            _ => drag.is_click(),
        };
        match self.tool {
            Tool::Measure if click => match self.distance(screen, drag.start.0, drag.start.1) {
                Some(m) => self.hold(screen, m),
                None => Placed::Nothing,
            },
            Tool::Measure if self.alternate => {
                if objects.is_empty() {
                    return Placed::Nothing;
                }
                for rect in objects {
                    self.hold(screen, Measurement::Area { rect, snapped: true });
                }
                Placed::Measurement { screen }
            }
            Tool::Measure => match area {
                Some((_, rect, snapped)) if rect.width > 0.0 && rect.height > 0.0 => {
                    self.hold(screen, Measurement::Area { rect, snapped })
                }
                _ => Placed::Nothing,
            },
            Tool::Annotate | Tool::Draw if click => match self.marker_at(screen, drag.start.0, drag.start.1) {
                Some(index) => Placed::Edit { screen, index },
                None if self.tool == Tool::Annotate => self.annotate_at(screen, drag.start.0, drag.start.1, time),
                None => Placed::Nothing,
            },
            Tool::Annotate => self.annotate(screen, drag.start, Some(drag.rect()), None, time),
            Tool::Draw => {
                let shape = if self.crosshair {
                    Shape::Arrow { from: drag.start, to: drag.current }
                } else {
                    let mut points = drag.points;
                    if points.last() != Some(&drag.current) {
                        points.push(drag.current);
                    }
                    Shape::Sketch { points }
                };
                self.annotate(screen, drag.start, None, Some(shape), time)
            }
            Tool::Shot if click => Placed::Nothing,
            Tool::Shot => {
                let rect = drag.rect();
                self.tool = self.previous_tool;
                self.shot(screen, rect, time)
            }
        }
    }

    fn hold(&mut self, screen: usize, m: Measurement) -> Placed {
        let state = &mut self.screens[screen];
        state.held.push(m);
        state.held_times.push(self.clock);
        self.added.push((screen, Added::Held(state.held.len() - 1)));
        Placed::Measurement { screen }
    }

    /// H or V: holds one axis of the distances from the cursor.
    pub fn hold_axis(&mut self, axis: Axis) -> Placed {
        let Some((screen, _, _)) = self.cursor else { return Placed::Nothing };
        let Some(Measurement::Distance { x, y, mut across, mut down }) = self.hover() else { return Placed::Nothing };
        let empty = |r: &Run| Run { start: r.start, end: r.start, ..Run::default() };
        match axis {
            // A vertical guide's axis: the horizontal distance.
            Axis::X => down = empty(&down),
            Axis::Y => across = empty(&across),
        }
        if across.is_empty() && down.is_empty() {
            return Placed::Nothing;
        }
        self.hold(screen, Measurement::Distance { x, y, across, down })
    }

    /// Records a screenshot of `rect`, clamped to the screen.
    pub fn shot(&mut self, screen: usize, rect: PtRect, time: f32) -> Placed {
        let Some(state) = self.screens.get(screen) else { return Placed::Nothing };
        let x = rect.x.max(0.0);
        let y = rect.y.max(0.0);
        let rect =
            PtRect::new(x, y, rect.right().min(state.screen.width) - x, rect.bottom().min(state.screen.height) - y);
        if rect.width < 1.0 || rect.height < 1.0 {
            return Placed::Nothing;
        }
        self.shots.push(Shot { screen, rect, time });
        Placed::Shot { screen, rect }
    }

    /// The camera on a held object: a screenshot of it, with `padding` points around it unless ⌥
    /// is held.
    pub fn shot_of_held(&mut self, screen: usize, index: usize, padding: f64, time: f32) -> Placed {
        let Some(m) = self.screens.get(screen).and_then(|s| s.held.get(index)) else { return Placed::Nothing };
        let pad = if self.alternate { 0.0 } else { padding };
        let r = m.bounds();
        self.shot(screen, PtRect::new(r.x - pad, r.y - pad, r.width + 2.0 * pad, r.height + 2.0 * pad), time)
    }

    /// Annotates what's at `(x, y)`: the section there (see `section_at`), or else the element
    /// under it. A click with Annotate, or the context menu's Annotate Here.
    pub fn annotate_at(&mut self, screen: usize, x: f64, y: f64, time: f32) -> Placed {
        match self.section_at(screen, x, y) {
            Some(rect) => self.annotate_section(screen, rect, time),
            None => self.annotate(screen, (x, y), None, None, time),
        }
    }

    /// Annotates a held area (the context menu's Annotate This on a measured section).
    pub fn annotate_held(&mut self, screen: usize, index: usize, time: f32) -> Placed {
        match self.screens.get(screen).and_then(|s| s.held.get(index)) {
            Some(&Measurement::Area { rect, .. }) => self.annotate_section(screen, rect, time),
            _ => Placed::Nothing,
        }
    }

    /// An annotation on the area of a section, or, when it has one already, that one to edit.
    fn annotate_section(&mut self, screen: usize, rect: PtRect, time: f32) -> Placed {
        let list = &self.screens[screen].annotations;
        match list.iter().position(|a| a.shape.is_none() && a.area == Some(rect)) {
            Some(index) => Placed::Edit { screen, index },
            None => self.annotate(screen, (rect.x, rect.y), Some(rect), None, time),
        }
    }

    /// The measured section (a held area, snapped to an object or drawn) that Annotate annotates at
    /// `(x, y)`: the smallest one there, unless the element under the pointer (`element`) is
    /// clearly smaller. What's outlined under the pointer, and annotated, is the most specific.
    pub fn section_at(&self, screen: usize, x: f64, y: f64) -> Option<PtRect> {
        let size = |r: &PtRect| r.width * r.height;
        let section = self
            .screens
            .get(screen)?
            .held
            .iter()
            .filter_map(|m| match m {
                Measurement::Area { rect, .. } if rect.contains(x, y) => Some(*rect),
                _ => None,
            })
            .min_by(|a, b| size(a).total_cmp(&size(b)))?;
        match self.element {
            Some((s, frame)) if s == screen && frame.contains(x, y) && size(&frame) < size(&section) => {
                fits(frame, section).then_some(section)
            }
            _ => Some(section),
        }
    }

    fn annotate(
        &mut self,
        screen: usize,
        at: (f64, f64),
        area: Option<PtRect>,
        shape: Option<Shape>,
        time: f32,
    ) -> Placed {
        let at = match area {
            Some(r) => (r.x, r.y),
            None => at,
        };
        let key = self.next_key;
        let annotation = Annotation { number: self.next_number, key, at, area, shape, time, ..Annotation::default() };
        self.next_number += 1;
        self.next_key += 1;
        let list = &mut self.screens[screen].annotations;
        list.push(annotation);
        let index = list.len() - 1;
        self.added.push((screen, Added::Annotation(index)));
        if area.is_some() {
            self.describe(screen, index);
        }
        Placed::Annotation { screen, index }
    }

    /// The marker under `(x, y)`, if any.
    pub fn marker_at(&self, screen: usize, x: f64, y: f64) -> Option<usize> {
        let list = &self.screens.get(screen)?.annotations;
        let distance = |a: &Annotation| (a.at.0 - x).hypot(a.at.1 - y);
        (0..list.len())
            .filter(|&i| distance(&list[i]) <= MARKER / 2.0 + 1.0)
            .min_by(|&a, &b| distance(&list[a]).total_cmp(&distance(&list[b])))
    }

    /// The held measurement whose label or outline is under `(x, y)`, for its camera.
    pub fn held_at(&self, screen: usize, x: f64, y: f64) -> Option<usize> {
        let list = &self.screens.get(screen)?.held;
        list.iter().rposition(|m| match m {
            Measurement::Area { rect, .. } => rect.contains(x, y),
            Measurement::Distance { .. } => false,
        })
    }

    /// Records what an annotation is on (from the accessibility tree; its frame in this screen's points)
    /// and how that looks. For an area or a drawing, an element much smaller or larger than what
    /// was annotated only says which app and window it's in.
    pub fn set_element(&mut self, screen: usize, index: usize, part: Part, element: Element) {
        let Some(a) = self.screens.get_mut(screen).and_then(|s| s.annotations.get_mut(index)) else { return };
        let region = match (&a.shape, a.area) {
            (Some(Shape::Sketch { points }), _) => Some(Shape::Sketch { points: points.clone() }.bounds()),
            (_, Some(area)) => Some(area),
            _ => None,
        };
        let element = match (region, element.frame) {
            (Some(region), Some(frame)) if !fits(frame, region) => {
                Element { app: element.app, window: element.window, url: element.url, ..Element::default() }
            }
            (Some(_), None) => {
                Element { app: element.app, window: element.window, url: element.url, ..Element::default() }
            }
            _ => element,
        };
        match part {
            Part::Element => a.element = Some(element),
            Part::Target => a.target = Some(element),
        }
        if part == Part::Element && a.shape.is_none() {
            self.describe(screen, index);
        }
    }

    /// Measures the annotated element's colors and padding from the frame.
    fn describe(&mut self, screen: usize, index: usize) {
        let tolerance = self.tolerance;
        let Some(state) = self.screens.get_mut(screen) else { return };
        let Some(a) = state.annotations.get(index) else { return };
        let Some(rect) = a.area.or(a.element.as_ref().and_then(|e| e.frame)) else { return };
        let s = state.screen.scale;
        let (x0, y0) = Session::pixel(state, rect.x, rect.y);
        let (x1, y1) = Session::pixel(state, rect.right() - 1.0 / s, rect.bottom() - 1.0 / s);
        let look = measure::appearance(&state.frame, Rect::from_corners((x0, y0), (x1, y1)), tolerance).map(|l| Look {
            background: Some(measure::hex(l.background)),
            foreground: l.foreground.map(measure::hex),
            padding: l.insets.map(|i| [i.top as f64 / s, i.right as f64 / s, i.bottom as f64 / s, i.left as f64 / s]),
        });
        state.annotations[index].look = look;
    }

    /// Sets an annotation's typed feedback.
    pub fn set_feedback(
        &mut self,
        screen: usize,
        index: usize,
        comment: &str,
        intent: Option<Intent>,
        severity: Option<Severity>,
    ) {
        if let Some(a) = self.screens.get_mut(screen).and_then(|s| s.annotations.get_mut(index)) {
            a.comment = comment.trim().to_string();
            a.intent = intent;
            a.severity = severity;
        }
    }

    /// Adds a guide through the cursor, snapped to a held object's edges or middle, else to the
    /// nearest edge (unless ⌘ is held).
    pub fn add_guide(&mut self, axis: Axis) -> bool {
        let Some((screen, x, y)) = self.cursor else { return false };
        self.add_guide_at(screen, axis, x, y)
    }

    pub fn add_guide_at(&mut self, screen: usize, axis: Axis, x: f64, y: f64) -> bool {
        let tolerance = self.tolerance;
        let snapping = self.snapping;
        let Some(state) = self.screens.get_mut(screen) else { return false };
        let s = state.screen.scale;
        let at = if axis == Axis::X { x } else { y };
        let held = snapping
            .then(|| {
                state
                    .held
                    .iter()
                    .filter_map(|m| match m {
                        Measurement::Area { rect, .. } => Some(*rect),
                        _ => None,
                    })
                    .flat_map(|r| match axis {
                        Axis::X => [r.x, r.x + r.width / 2.0, r.right()],
                        Axis::Y => [r.y, r.y + r.height / 2.0, r.bottom()],
                    })
                    .filter(|p| (p - at).abs() <= GUIDE_SNAP)
                    .min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
            })
            .flatten();
        let (px, py) = Session::pixel(state, x, y);
        let edge = (snapping && held.is_none())
            .then(|| {
                let axis = if axis == Axis::X { measure::Axis::X } else { measure::Axis::Y };
                measure::nearest_edge(&state.frame, axis, px, py, (GUIDE_SNAP * s) as u32, tolerance)
            })
            .flatten();
        let position = match (held, edge) {
            (Some(p), _) => p,
            (None, Some(b)) => b as f64 / s,
            (None, None) => (at * s).round() / s,
        };
        state.guides.push(Guide { axis, position });
        self.added.push((screen, Added::Guide(state.guides.len() - 1)));
        true
    }

    /// Moves the guide added last by `points` (arrow keys).
    pub fn nudge_guide(&mut self, points: f64) -> bool {
        let Some(&(screen, Added::Guide(i))) = self.added.iter().rev().find(|(_, a)| matches!(a, Added::Guide(_)))
        else {
            return false;
        };
        let state = &mut self.screens[screen];
        let limit = if state.guides[i].axis == Axis::X { state.screen.width } else { state.screen.height };
        state.guides[i].position = (state.guides[i].position + points).clamp(0.0, limit);
        true
    }

    /// Puts back guides and held measurements from an earlier session, on the screens they were on.
    pub fn restore(&mut self, saved: &Saved) -> bool {
        let mut restored = false;
        for (screen, list) in saved.guides.iter().enumerate() {
            let Some(state) = self.screens.get_mut(screen) else { continue };
            for guide in list {
                if !state.guides.contains(guide) {
                    state.guides.push(*guide);
                    self.added.push((screen, Added::Guide(state.guides.len() - 1)));
                    restored = true;
                }
            }
        }
        for (screen, list) in saved.held.iter().enumerate() {
            let Some(state) = self.screens.get_mut(screen) else { continue };
            for m in list {
                if !state.held.contains(m) {
                    state.held.push(m.clone());
                    state.held_times.push(self.clock);
                    self.added.push((screen, Added::Held(state.held.len() - 1)));
                    restored = true;
                }
            }
        }
        restored
    }

    /// The guides and held measurements on each screen, to restore in a later session.
    pub fn saved(&self) -> Saved {
        Saved {
            guides: self.screens.iter().map(|s| s.guides.clone()).collect(),
            held: self.screens.iter().map(|s| s.held.clone()).collect(),
        }
    }

    /// The whole session as Markdown, as it would go in the aside.
    pub fn markdown(&self, detail: Detail) -> String {
        let mut blocks = Vec::new();
        for (_, inspection) in self.inspections() {
            let (header, notes) = markdown(&inspection, detail);
            blocks.extend(header);
            blocks.extend(notes.into_iter().map(|n| n.text));
        }
        crate::markdown::join_blocks(blocks)
    }

    /// Removes the last thing added.
    pub fn undo(&mut self) -> bool {
        let Some((screen, added)) = self.added.pop() else { return false };
        let state = &mut self.screens[screen];
        match added {
            Added::Held(i) if i < state.held.len() => {
                state.held.remove(i);
                state.held_times.remove(i);
            }
            Added::Guide(i) if i < state.guides.len() => drop(state.guides.remove(i)),
            Added::Annotation(i) if i < state.annotations.len() => {
                state.annotations.remove(i);
                self.renumber();
            }
            _ => {}
        }
        true
    }

    pub fn remove_annotation(&mut self, screen: usize, index: usize) {
        if self.screens.get(screen).is_some_and(|s| index < s.annotations.len()) {
            self.screens[screen].annotations.remove(index);
            self.added.retain(|&(s, a)| !(s == screen && a == Added::Annotation(index)));
            for (s, a) in &mut self.added {
                if let Added::Annotation(i) = a
                    && *s == screen
                    && *i > index
                {
                    *i -= 1;
                }
            }
            self.renumber();
        }
    }

    /// Removes a held measurement (right-click on it).
    pub fn remove_held(&mut self, screen: usize, index: usize) {
        if self.screens.get(screen).is_some_and(|s| index < s.held.len()) {
            self.screens[screen].held.remove(index);
            self.screens[screen].held_times.remove(index);
            self.added.retain(|&(s, a)| !(s == screen && a == Added::Held(index)));
            for (s, a) in &mut self.added {
                if let Added::Held(i) = a
                    && *s == screen
                    && *i > index
                {
                    *i -= 1;
                }
            }
        }
    }

    /// Annotations stay numbered 1, 2, 3… in the order they were put down.
    fn renumber(&mut self) {
        let mut all: Vec<(u32, usize, usize)> = Vec::new();
        for (s, state) in self.screens.iter().enumerate() {
            for (i, a) in state.annotations.iter().enumerate() {
                all.push((a.number, s, i));
            }
        }
        all.sort();
        for (n, (_, s, i)) in all.iter().enumerate() {
            self.screens[*s].annotations[*i].number = n as u32 + 1;
        }
        self.next_number = all.len() as u32 + 1;
    }

    pub fn clear(&mut self) {
        for state in &mut self.screens {
            state.held.clear();
            state.held_times.clear();
            state.guides.clear();
            state.annotations.clear();
        }
        self.shots.clear();
        self.added.clear();
        self.next_number = 1;
    }

    pub fn raise_tolerance(&mut self) {
        self.tolerance = self.tolerance.raised(2);
        self.normal_tolerance = self.tolerance;
    }

    pub fn lower_tolerance(&mut self) {
        self.tolerance = self.tolerance.lowered(2);
        self.normal_tolerance = self.tolerance;
    }

    /// T: switches between the tolerance in use and a high one that looks through shadows.
    pub fn toggle_high_tolerance(&mut self) {
        self.tolerance = if self.tolerance >= Tolerance::HIGH {
            self.normal_tolerance.min(Tolerance(Tolerance::HIGH.0 - 1))
        } else {
            Tolerance::HIGH
        };
    }

    pub fn toggle_units(&mut self) {
        self.units = if self.units == Units::Pt { Units::Px } else { Units::Pt };
    }

    /// The dimensions to copy (⌘C): what the cursor measures, or else the last thing held.
    pub fn dimensions(&self) -> Option<String> {
        let last_held = || {
            self.added.iter().rev().find_map(|(s, a)| match a {
                Added::Held(i) => self.screens[*s].held.get(*i).map(|m| (*s, m.clone())),
                _ => None,
            })
        };
        let screen = self.cursor.map(|c| c.0).unwrap_or(0);
        let (screen, m) = self.hover().map(|m| (screen, m)).or_else(last_held)?;
        Some(m.dimensions(self.screens[screen].screen.scale, self.units))
    }

    /// Everything measured and annotated on each screen that has something, as it goes in the aside.
    pub fn inspections(&self) -> Vec<(usize, Inspection)> {
        self.screens
            .iter()
            .enumerate()
            .filter(|(_, s)| !(s.held.is_empty() && s.guides.is_empty() && s.annotations.is_empty()))
            .map(|(i, s)| {
                let mut annotations = s.annotations.clone();
                annotations.sort_by_key(|a| a.number);
                (
                    i,
                    Inspection {
                        screen: s.screen.clone(),
                        place: s.place.clone(),
                        units: self.units,
                        measurements: s.held.clone(),
                        measurement_times: s.held_times.clone(),
                        guides: s.guides.clone(),
                        annotations,
                    },
                )
            })
            .collect()
    }

    /// Runs one scripted action (tests and `asides simulate-measure` drive sessions this way).
    pub fn apply(&mut self, action: &Action) -> Placed {
        match action {
            Action::Tool(tool) => self.set_tool(*tool),
            Action::Tolerance(t) => {
                self.tolerance = Tolerance(*t);
                self.normal_tolerance = self.tolerance;
            }
            Action::Units(units) => self.units = *units,
            // The player waits; the session's clock moves on.
            Action::Wait(seconds) => self.clock += *seconds as f32,
            // The player goes back to the live screen, or freezes it again.
            Action::Live | Action::Freeze => {}
            Action::Click { screen, at } => {
                self.mouse_down(*screen, at.0, at.1, false);
                return self.mouse_up(*screen, at.0, at.1, self.clock);
            }
            Action::Drag { screen, from, to, snap } => {
                self.mouse_down(*screen, from.0, from.1, !snap);
                self.move_to(*screen, to.0, to.1);
                return self.mouse_up(*screen, to.0, to.1, self.clock);
            }
            Action::Objects { screen, from, to } => {
                let (previous, alternate) = (self.tool, self.alternate);
                (self.tool, self.alternate) = (Tool::Measure, true);
                let placed = self.apply(&Action::Drag { screen: *screen, from: *from, to: *to, snap: true });
                (self.tool, self.alternate) = (previous, alternate);
                return placed;
            }
            Action::Hold { screen, axis, at } => {
                let previous = self.tool;
                self.tool = Tool::Measure;
                self.move_to(*screen, at.0, at.1);
                let placed = self.hold_axis(*axis);
                self.tool = previous;
                return placed;
            }
            Action::Guide { screen, axis, at } => {
                self.add_guide_at(*screen, *axis, at.0, at.1);
            }
            Action::Shot { screen, from, to } => {
                self.set_tool(Tool::Shot);
                return self.apply(&Action::Drag { screen: *screen, from: *from, to: *to, snap: false });
            }
            Action::Annotate { screen, at, area, element, comment, intent, severity } => {
                let previous = self.tool;
                self.tool = Tool::Annotate;
                let placed = match area {
                    Some(r) => self.apply(&Action::Drag {
                        screen: *screen,
                        from: (r.x, r.y),
                        to: (r.right(), r.bottom()),
                        snap: false,
                    }),
                    None => self.apply(&Action::Click { screen: *screen, at: *at }),
                };
                self.tool = previous;
                return self.feedback(placed, element.as_deref(), None, comment, *intent, *severity);
            }
            Action::Draw { screen, points, arrow, element, target, comment, intent, severity } => {
                let (Some(first), Some(last)) = (points.first(), points.last()) else { return Placed::Nothing };
                let (previous, crosshair) = (self.tool, self.crosshair);
                (self.tool, self.crosshair) = (Tool::Draw, *arrow);
                self.mouse_down(*screen, first.0, first.1, false);
                for p in &points[1..] {
                    self.move_to(*screen, p.0, p.1);
                }
                let placed = self.mouse_up(*screen, last.0, last.1, self.clock);
                (self.tool, self.crosshair) = (previous, crosshair);
                return self.feedback(placed, element.as_deref(), target.as_deref(), comment, *intent, *severity);
            }
        }
        Placed::Nothing
    }

    fn feedback(
        &mut self,
        placed: Placed,
        element: Option<&Element>,
        target: Option<&Element>,
        comment: &str,
        intent: Option<Intent>,
        severity: Option<Severity>,
    ) -> Placed {
        if let Placed::Annotation { screen, index } = placed {
            if let Some(element) = element {
                self.set_element(screen, index, Part::Element, element.clone());
            }
            if let Some(target) = target {
                self.set_element(screen, index, Part::Target, target.clone());
            }
            self.set_feedback(screen, index, comment, intent, severity);
        }
        placed
    }
}

/// Shortens `run` (along one axis, through `along` on it, `across` on the other) where a held
/// object `[start, end) × [lo, hi)` lies in its way, or to the object's edges when inside it.
fn stop_at(run: &mut Run, along: f64, across: f64, start: f64, end: f64, lo: f64, hi: f64) {
    if run.is_empty() || across < lo || across >= hi {
        return;
    }
    if along >= start && along < end {
        if start > run.start {
            (run.start, run.open_start) = (start, false);
        }
        if end < run.end {
            (run.end, run.open_end) = (end, false);
        }
    } else if end <= along && end > run.start {
        (run.start, run.open_start) = (end, false);
    } else if start > along && start < run.end {
        (run.end, run.open_end) = (start, false);
    }
}

/// Whether an element's frame is about the size of what was annotated (so it's what was annotated).
fn fits(frame: PtRect, region: PtRect) -> bool {
    let (a, b) = (frame.width * frame.height, region.width.max(1.0) * region.height.max(1.0));
    a >= b * 0.3 && a <= b * 3.0
}

/// A scripted step of a session, as JSON: `{"click": {"at": [100, 200]}}`, `{"tool": "annotate"}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Tool(Tool),
    Tolerance(u8),
    Units(Units),
    Click {
        #[serde(default)]
        screen: usize,
        at: (f64, f64),
    },
    Drag {
        #[serde(default)]
        screen: usize,
        from: (f64, f64),
        to: (f64, f64),
        #[serde(default = "yes")]
        snap: bool,
    },
    /// ⌥-drag: holds each object inside.
    Objects {
        #[serde(default)]
        screen: usize,
        from: (f64, f64),
        to: (f64, f64),
    },
    /// H (axis "x", the distance across) or V (axis "y", down) with the cursor at `at`.
    Hold {
        #[serde(default)]
        screen: usize,
        axis: Axis,
        at: (f64, f64),
    },
    Guide {
        #[serde(default)]
        screen: usize,
        axis: Axis,
        at: (f64, f64),
    },
    Shot {
        #[serde(default)]
        screen: usize,
        from: (f64, f64),
        to: (f64, f64),
    },
    Annotate {
        #[serde(default)]
        screen: usize,
        #[serde(default)]
        at: (f64, f64),
        #[serde(default)]
        area: Option<PtRect>,
        #[serde(default)]
        element: Option<Box<Element>>,
        #[serde(default)]
        comment: String,
        #[serde(default)]
        intent: Option<Intent>,
        #[serde(default)]
        severity: Option<Severity>,
    },
    /// A sketch through `points`, or with `arrow`, an arrow from the first to the last.
    Draw {
        #[serde(default)]
        screen: usize,
        points: Vec<(f64, f64)>,
        #[serde(default)]
        arrow: bool,
        #[serde(default)]
        element: Option<Box<Element>>,
        #[serde(default)]
        target: Option<Box<Element>>,
        #[serde(default)]
        comment: String,
        #[serde(default)]
        intent: Option<Intent>,
        #[serde(default)]
        severity: Option<Severity>,
    },
    /// Seconds to wait before the next step, with the session open (the end-to-end test talks
    /// meanwhile).
    Wait(f64),
    /// Back to the live screen: what's on the frozen one goes in the aside.
    Live,
    /// Freeze the screen again (the same image).
    Freeze,
}

fn yes() -> bool {
    true
}

/// A scripted session (`asides simulate-measure`): the screenshot's pixels per point, then the steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Script {
    #[serde(default = "one")]
    pub scale: f64,
    pub actions: Vec<Action>,
}

impl Script {
    pub fn parse(text: &str) -> Result<Script, String> {
        let script: Script = serde_json::from_str(text).map_err(|e| e.to_string())?;
        if script.scale <= 0.0 {
            return Err("scale must be positive".into());
        }
        Ok(script)
    }
}

fn one() -> f64 {
    1.0
}

fn px_rect_to_pt(r: Rect, scale: f64) -> PtRect {
    PtRect::new(r.x as f64 / scale, r.y as f64 / scale, r.width as f64 / scale, r.height as f64 / scale)
}

/// One measurement or annotation as it goes in the aside, with the moment it was made, so the app
/// can place it among what was said.
#[derive(Debug, Clone, PartialEq)]
pub struct Note {
    /// Seconds into the session.
    pub time: f32,
    pub text: String,
    /// The annotation's key, and the recordings of what was said about it that `text` has.
    pub annotation: Option<(u32, Vec<u64>)>,
}

/// The Markdown for one screen's inspection: what goes with its screenshot (where the annotations
/// are, when they share an app and a page; the screen and units of the measurements; the guides),
/// then each measurement and annotation, in the order they were made.
pub fn markdown(inspection: &Inspection, detail: Detail) -> (Option<String>, Vec<Note>) {
    let header: Vec<String> =
        [inspection.context_markdown(), inspection.measurements_markdown(detail)].into_iter().flatten().collect();
    let measured = inspection.measurement_lines().into_iter().enumerate().map(|(i, text)| Note {
        time: inspection.measurement_times.get(i).copied().unwrap_or_default(),
        text,
        annotation: None,
    });
    let annotated = inspection.annotations.iter().map(|a| Note {
        time: a.time,
        text: inspection.annotation_markdown(a, detail),
        annotation: Some((a.key, a.said.iter().map(|s| s.take).collect())),
    });
    let mut notes: Vec<Note> = measured.chain(annotated).collect();
    notes.sort_by(|a, b| a.time.total_cmp(&b.time));
    ((!header.is_empty()).then(|| header.join("\n")), notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 2× screen of 100 × 50 pt: white, with a blue 30 × 10 pt button at (20, 15).
    fn session() -> Session {
        let mut frame = Frame::filled(200, 100, 0xffffff);
        frame.fill(Rect::new(40, 30, 60, 20), 0x2563eb);
        frame.fill(Rect::new(52, 36, 36, 8), 0xffffff);
        let screen = Screen { name: None, width: 100.0, height: 50.0, scale: 2.0 };
        Session::new(vec![Session::screen(screen, frame)], Tolerance::DEFAULT, Units::Pt)
    }

    #[test]
    fn hovering_measures_and_clicking_holds() {
        let mut s = session();
        s.move_to(0, 5.0, 20.0);
        let Some(Measurement::Distance { across, down, .. }) = s.hover() else { panic!("no distance") };
        assert_eq!((across.start, across.end, across.open_start), (0.0, 20.0, true));
        assert_eq!((down.start, down.end), (0.0, 50.0));
        assert_eq!(s.dimensions().as_deref(), Some("20 × 50"));
        s.toggle_units();
        assert_eq!(s.dimensions().as_deref(), Some("40 × 100"));
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (5.0, 20.0) }), Placed::Measurement { screen: 0 });
        assert_eq!(s.screens[0].held.len(), 1);
        assert!(s.undo());
        assert!(s.is_empty());
    }

    #[test]
    fn holding_one_axis() {
        let mut s = session();
        s.toggle_units();
        assert_eq!(
            s.apply(&Action::Hold { screen: 0, axis: Axis::X, at: (5.0, 20.0) }),
            Placed::Measurement { screen: 0 }
        );
        assert_eq!(s.cursor, Some((0, 5.0, 20.0)));
        s.cursor = None;
        assert_eq!(s.dimensions().as_deref(), Some("40"));
        s.apply(&Action::Hold { screen: 0, axis: Axis::Y, at: (5.0, 20.0) });
        s.cursor = None;
        assert_eq!(s.dimensions().as_deref(), Some("100"));
    }

    #[test]
    fn dragging_snaps_to_the_object() {
        let mut s = session();
        s.apply(&Action::Drag { screen: 0, from: (10.0, 5.0), to: (70.0, 40.0), snap: true });
        assert_eq!(s.screens[0].held, [Measurement::Area { rect: PtRect::new(20.0, 15.0, 30.0, 10.0), snapped: true }]);
        // A right-drag keeps the area as dragged.
        s.apply(&Action::Drag { screen: 0, from: (10.0, 5.0), to: (70.0, 40.0), snap: false });
        assert_eq!(
            s.screens[0].held[1],
            Measurement::Area { rect: PtRect::new(10.0, 5.0, 60.0, 35.0), snapped: false }
        );
        // Space moves the area being dragged.
        s.mouse_down(0, 10.0, 10.0, true);
        s.move_to(0, 20.0, 20.0);
        s.moving = true;
        s.move_to(0, 25.0, 30.0);
        assert_eq!(s.drag.as_ref().unwrap().rect(), PtRect::new(15.0, 20.0, 10.0, 10.0));
        // Shift makes it square.
        s.moving = false;
        s.crosshair = true;
        s.move_to(0, 40.0, 25.0);
        assert_eq!(s.drag.as_ref().unwrap().rect(), PtRect::new(15.0, 20.0, 25.0, 25.0));
    }

    #[test]
    fn held_objects_stop_distances_and_option_holds_each_object() {
        let mut s = session();
        s.screens[0].frame.fill(Rect::new(140, 30, 20, 20), 0x111111);
        assert_eq!(
            s.apply(&Action::Objects { screen: 0, from: (1.0, 1.0), to: (99.0, 49.0) }),
            Placed::Measurement { screen: 0 }
        );
        assert_eq!(
            s.screens[0].held,
            [
                Measurement::Area { rect: PtRect::new(20.0, 15.0, 30.0, 10.0), snapped: true },
                Measurement::Area { rect: PtRect::new(70.0, 15.0, 10.0, 10.0), snapped: true },
            ]
        );
        // Below the objects the row is empty, but a held area (drawn, not snapped) stops it.
        let Some(Measurement::Distance { across, .. }) = s.distance(0, 55.0, 40.0) else { panic!() };
        assert_eq!((across.start, across.end, across.open_end), (0.0, 100.0, true));
        s.apply(&Action::Drag { screen: 0, from: (60.0, 30.0), to: (65.0, 50.0), snap: false });
        let Some(Measurement::Distance { across, .. }) = s.distance(0, 55.0, 40.0) else { panic!() };
        assert_eq!((across.start, across.end, across.open_end), (0.0, 60.0, false));
        // Inside it, its edges.
        let Some(Measurement::Distance { across, .. }) = s.distance(0, 62.0, 40.0) else { panic!() };
        assert_eq!((across.start, across.end), (60.0, 65.0));
        s.undo();
        // A guide near a held object's middle snaps to it.
        s.add_guide_at(0, Axis::X, 33.0, 40.0);
        assert_eq!(s.screens[0].guides[0].position, 35.0);
        // Saved for the next session, and restored once.
        let saved = s.saved();
        let mut next = session();
        assert!(next.restore(&saved) && !next.restore(&saved));
        assert_eq!((next.screens[0].held.len(), next.screens[0].guides.len()), (2, 1));
    }

    #[test]
    fn guides_snap_to_edges_and_stop_distances() {
        let mut s = session();
        s.move_to(0, 18.5, 20.0);
        assert!(s.add_guide(Axis::X));
        assert_eq!(s.screens[0].guides, [Guide { axis: Axis::X, position: 20.0 }]);
        assert!(s.nudge_guide(-10.0));
        assert_eq!(s.screens[0].guides[0].position, 10.0);
        s.move_to(0, 5.0, 20.0);
        let Some(Measurement::Distance { across, .. }) = s.hover() else { panic!() };
        assert_eq!((across.start, across.end), (0.0, 10.0));
        s.snapping = false;
        s.add_guide_at(0, Axis::Y, 3.0, 33.3);
        assert_eq!(s.screens[0].guides[1].position, 33.5);
        assert_eq!(s.markdown(Detail::Compact), "**Measured** in pt\n**Guides:** x 10 · y 33.5");
    }

    #[test]
    fn annotations_are_numbered_described_and_written() {
        let mut s = session();
        let element = Element {
            app: Some("Safari".into()),
            name: Some("button \"Pay\"".into()),
            frame: Some(PtRect::new(20.0, 15.0, 30.0, 10.0)),
            ..Element::default()
        };
        let placed = s.apply(&Action::Annotate {
            screen: 0,
            at: (25.0, 20.0),
            area: None,
            element: Some(Box::new(element)),
            comment: "Too tight".into(),
            intent: Some(Intent::Fix),
            severity: None,
        });
        assert_eq!(placed, Placed::Annotation { screen: 0, index: 0 });
        assert_eq!(lookups(&s.screens[0].annotations[0]), [(Part::Element, 25.0, 20.0)]);
        let a = &s.screens[0].annotations[0];
        assert_eq!((a.number, a.comment.as_str()), (1, "Too tight"));
        let look = a.look.clone().unwrap();
        assert_eq!((look.background.as_deref(), look.padding), (Some("#2563EB"), Some([3.0, 6.0, 3.0, 6.0])));
        s.apply(&Action::Annotate {
            screen: 0,
            at: (0.0, 0.0),
            area: Some(PtRect::new(60.0, 5.0, 30.0, 30.0)),
            element: None,
            comment: String::new(),
            intent: None,
            severity: Some(Severity::Suggestion),
        });
        s.tool = Tool::Annotate;
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (26.0, 21.0) }), Placed::Edit { screen: 0, index: 0 });
        s.remove_annotation(0, 0);
        assert_eq!(s.screens[0].annotations[0].number, 1);

        let inspections = s.inspections();
        let (measured, notes) = markdown(&inspections[0].1, Detail::Standard);
        assert_eq!(measured, None);
        let text = "### 1. Area 30 × 30 at (60, 5)\n**Severity:** suggestion".to_string();
        assert_eq!(notes, [Note { time: 0.0, text, annotation: Some((2, vec![])) }]);
        s.clear();
        assert!(s.is_empty() && s.inspections().is_empty());
    }

    #[test]
    fn freezing_again_goes_on_numbering() {
        let mut s = session();
        s.set_tool(Tool::Annotate);
        s.apply(&Action::Wait(3.0));
        s.apply(&Action::Click { screen: 0, at: (90.0, 45.0) });
        let next = s.refrozen(session().screens);
        assert!(next.is_empty());
        assert_eq!((next.tool, next.clock), (Tool::Annotate, 3.0));
        let mut next = next;
        next.apply(&Action::Click { screen: 0, at: (90.0, 45.0) });
        let a = &next.screens[0].annotations[0];
        assert_eq!((a.number, a.key, a.time), (2, 2, 3.0));
    }

    #[test]
    fn notes_keep_the_order_things_were_made() {
        let mut s = session();
        let script: Vec<Action> = serde_json::from_str(
            r#"[{"drag": {"from": [10, 5], "to": [70, 40]}}, {"hold": {"axis": "x", "at": [5, 20]}},
                {"wait": 2}, {"tool": "annotate"}, {"click": {"at": [90, 45]}},
                {"wait": 1.5}, {"hold": {"axis": "y", "at": [5, 20]}}]"#,
        )
        .unwrap();
        for action in &script {
            s.apply(action);
        }
        assert_eq!(s.screens[0].held_times, [0.0, 0.0, 3.5]);
        let (header, notes) = markdown(&s.inspections()[0].1, Detail::Compact);
        assert_eq!(header.as_deref(), Some("**Measured** in pt"));
        let order: Vec<(f32, &str)> = notes.iter().map(|n| (n.time, &n.text[..5])).collect();
        assert_eq!(order, [(0.0, "- Are"), (0.0, "- Dis"), (2.0, "1. **"), (3.5, "- Dis")]);
        assert_eq!(notes[2].annotation, Some((1, vec![])));
        // Measured one after another: one list.
        assert!(s.markdown(Detail::Compact).starts_with("**Measured** in pt\n\n- Area 30 × 10 (3:1)"));
        assert_eq!(s.markdown(Detail::Compact).matches("\n- Distance").count(), 2);
        // Undo takes the time away with the measurement.
        s.undo();
        assert_eq!(s.screens[0].held_times, [0.0, 0.0]);
    }

    #[test]
    fn annotating_measured_sections() {
        let mut s = session();
        // A section measured as drawn, around the button.
        s.apply(&Action::Drag { screen: 0, from: (10.0, 5.0), to: (70.0, 40.0), snap: false });
        let section = PtRect::new(10.0, 5.0, 60.0, 35.0);
        s.set_tool(Tool::Annotate);
        assert_eq!(s.section_at(0, 25.0, 20.0), Some(section));
        // A click in it annotates the section; another one edits that annotation.
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (60.0, 30.0) }), Placed::Annotation { screen: 0, index: 0 });
        assert_eq!(s.screens[0].annotations[0].area, Some(section));
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (65.0, 35.0) }), Placed::Edit { screen: 0, index: 0 });
        // The element under the pointer, much smaller than the section, is annotated instead.
        s.element = Some((0, PtRect::new(20.0, 15.0, 30.0, 10.0)));
        assert_eq!(s.section_at(0, 25.0, 20.0), None);
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (25.0, 20.0) }), Placed::Annotation { screen: 0, index: 1 });
        assert_eq!((s.screens[0].annotations[1].at, s.screens[0].annotations[1].area), ((25.0, 20.0), None));
        // A section snapped to the button (detected): the smallest section there, and about the
        // element's size, so it's the section. The context menu annotates a held one.
        s.set_tool(Tool::Measure);
        s.apply(&Action::Drag { screen: 0, from: (15.0, 12.0), to: (55.0, 30.0), snap: true });
        let button = PtRect::new(20.0, 15.0, 30.0, 10.0);
        assert_eq!(s.screens[0].held[1], Measurement::Area { rect: button, snapped: true });
        assert_eq!(s.section_at(0, 25.0, 20.0), Some(button));
        assert_eq!(s.annotate_held(0, 1, 2.0), Placed::Annotation { screen: 0, index: 2 });
        assert_eq!(s.screens[0].annotations[2].area, Some(button));
        assert_eq!(s.annotate_held(0, 1, 3.0), Placed::Edit { screen: 0, index: 2 });
        // Elsewhere, Annotate Here annotates the element under it.
        assert_eq!(s.annotate_at(0, 90.0, 45.0, 4.0), Placed::Annotation { screen: 0, index: 3 });
        assert_eq!(s.screens[0].annotations[3].area, None);
    }

    #[test]
    fn drawing_arrows_and_sketches() {
        let mut s = session();
        let save = Element {
            app: Some("Mail".into()),
            name: Some("button \"Save\"".into()),
            frame: Some(PtRect::new(20.0, 15.0, 30.0, 10.0)),
            ..Element::default()
        };
        let toolbar = Element { app: Some("Mail".into()), name: Some("toolbar".into()), ..Element::default() };
        let placed = s.apply(&Action::Draw {
            screen: 0,
            points: vec![(25.0, 20.0), (60.0, 30.0), (80.0, 40.0)],
            arrow: true,
            element: Some(Box::new(save.clone())),
            target: Some(Box::new(toolbar)),
            comment: "Move it here".into(),
            intent: Some(Intent::Change),
            severity: None,
        });
        assert_eq!(placed, Placed::Annotation { screen: 0, index: 0 });
        let arrow = &s.screens[0].annotations[0];
        assert_eq!(arrow.shape, Some(Shape::Arrow { from: (25.0, 20.0), to: (80.0, 40.0) }));
        assert_eq!(lookups(arrow), [(Part::Element, 25.0, 20.0), (Part::Target, 80.0, 40.0)]);
        // A sketch around the button names it; one around the whole screen only says the app.
        s.apply(&Action::Draw {
            screen: 0,
            points: vec![(18.0, 13.0), (52.0, 13.0), (52.0, 27.0), (18.0, 27.0), (18.0, 13.0)],
            arrow: false,
            element: Some(Box::new(save.clone())),
            target: None,
            comment: String::new(),
            intent: None,
            severity: None,
        });
        s.apply(&Action::Draw {
            screen: 0,
            points: vec![(1.0, 1.0), (99.0, 1.0), (99.0, 49.0)],
            arrow: false,
            element: Some(Box::new(save)),
            target: None,
            comment: String::new(),
            intent: None,
            severity: None,
        });
        let (header, notes) = markdown(&s.inspections()[0].1, Detail::Standard);
        // All in Mail: said once, with the screenshot.
        assert_eq!(header.as_deref(), Some("**In:** Mail"));
        assert_eq!(
            notes[0].text,
            "### 1. Arrow from button \"Save\" to toolbar\n**From → to:** (25, 20) → (80, 40)\n**Intent:** change\n\
             **Feedback:** Move it here"
        );
        assert_eq!(notes[1].text, "### 2. Drawn on button \"Save\"");
        assert_eq!(notes[2].text, "### 3. Drawn around 98 × 48 at (1, 1)");
        // A click with the Draw tool on a marker edits it; elsewhere it does nothing.
        s.set_tool(Tool::Draw);
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (25.0, 20.0) }), Placed::Edit { screen: 0, index: 0 });
        assert_eq!(s.apply(&Action::Click { screen: 0, at: (90.0, 10.0) }), Placed::Nothing);
    }

    #[test]
    fn screenshots_go_back_to_the_tool() {
        let mut s = session();
        s.set_tool(Tool::Annotate);
        let placed = s.apply(&Action::Shot { screen: 0, from: (90.0, 40.0), to: (120.0, 60.0) });
        assert_eq!(placed, Placed::Shot { screen: 0, rect: PtRect::new(90.0, 40.0, 10.0, 10.0) });
        assert_eq!(s.tool, Tool::Annotate);
        s.apply(&Action::Drag { screen: 0, from: (10.0, 5.0), to: (70.0, 40.0), snap: true });
        s.tool = Tool::Measure;
        s.apply(&Action::Drag { screen: 0, from: (10.0, 5.0), to: (70.0, 40.0), snap: true });
        assert_eq!(s.held_at(0, 25.0, 20.0), Some(0));
        assert_eq!(
            s.shot_of_held(0, 0, 8.0, 1.5),
            Placed::Shot { screen: 0, rect: PtRect::new(12.0, 7.0, 46.0, 26.0) }
        );
        s.alternate = true;
        assert_eq!(
            s.shot_of_held(0, 0, 8.0, 2.0),
            Placed::Shot { screen: 0, rect: PtRect::new(20.0, 15.0, 30.0, 10.0) }
        );
        assert_eq!(s.shots.len(), 3);
        s.remove_held(0, 0);
        assert!(s.screens[0].held.is_empty());
    }

    #[test]
    fn tolerance_steps_and_toggles() {
        let mut s = session();
        s.raise_tolerance();
        assert_eq!(s.tolerance, Tolerance(4));
        s.toggle_high_tolerance();
        assert_eq!(s.tolerance, Tolerance::HIGH);
        s.toggle_high_tolerance();
        assert_eq!(s.tolerance, Tolerance(4));
        let script: Vec<Action> = serde_json::from_str(
            r#"[{"tool": "annotate"}, {"tolerance": 10}, {"units": "px"}, {"click": {"at": [5, 20]}},
                {"drag": {"from": [10, 5], "to": [70, 40]}}, {"guide": {"axis": "y", "at": [5, 14]}},
                {"annotate": {"at": [25, 20], "comment": "Hi", "intent": "question"}},
                {"hold": {"axis": "x", "at": [5, 20]}}, {"draw": {"points": [[1, 1], [9, 9]], "arrow": true}},
                {"wait": 1.5}, "live", "freeze"]"#,
        )
        .unwrap();
        assert_eq!(script[0], Action::Tool(Tool::Annotate));
        assert_eq!(script[4], Action::Drag { screen: 0, from: (10.0, 5.0), to: (70.0, 40.0), snap: true });
        assert_eq!(script[9..], [Action::Wait(1.5), Action::Live, Action::Freeze]);
        let parsed = Script::parse(r#"{"actions": [{"tool": "measure"}]}"#).unwrap();
        assert_eq!((parsed.scale, parsed.actions), (1.0, vec![Action::Tool(Tool::Measure)]));
        assert!(Script::parse(r#"{"scale": 0, "actions": []}"#).is_err());
    }
}
