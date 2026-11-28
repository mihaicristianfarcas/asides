//! How the overlay's pieces look: the pill and its buttons, its list, the keys at a glance, a pin's
//! card, pins and labels, and what's painted on a frozen screen. The overlay (`ui.rs`) adds what
//! they do; Help (`help.rs`) draws them as they are, in its demonstrations, so they look the same.

use gpui::{
    AnyElement, BorderStyle, Bounds, Div, FontWeight, Hsla, PathBuilder, SharedString, Window, div, point, prelude::*,
    px, quad, size, svg, transparent_black,
};

use asides_core::inspect::{Intent, PtRect};
use asides_core::keys;
use asides_core::overlay::{MARKER, Tool};

use crate::controls::{Theme, hex};
use crate::style;
use crate::ui::{Cmd, KEYS_WIDTH, Keys, LIST_WIDTH};

/// Between the pill's buttons.
pub const PILL_GAP: f32 = 2.0;
/// The narrowest a label gets: a few digits and a unit.
pub const LABEL_MIN_WIDTH: f32 = 44.0;

/// How a toolbar button stands out: not at all, selected (a glass pill: the list open, a tool), or
/// the main action (asides' soft blue).
#[derive(Clone, Copy, PartialEq)]
pub enum Emphasis {
    Plain,
    On,
    Primary,
}

impl Emphasis {
    /// Its text and icons.
    pub fn ink(self, t: Theme) -> Hsla {
        match self {
            Emphasis::Plain => t.primary.opacity(0.86),
            Emphasis::On => t.primary,
            Emphasis::Primary => t.on_accent,
        }
    }
}

pub fn icon(name: &str, ink: Hsla) -> impl IntoElement + use<> {
    svg().path(format!("icons/{name}.svg")).size(px(15.)).text_color(ink)
}

pub fn tool_icon(tool: Tool) -> &'static str {
    match tool {
        Tool::Measure => "measure",
        Tool::Annotate => "annotate",
        Tool::Draw => "draw",
        Tool::Shot => "shot",
    }
}

// --- The pill ---

/// A surface of the pill (the pill, its list): rounded, the glass behind it (macOS blurs what's
/// behind the window there, `native::set_glass`) under the asides window's sidebar's tint, a
/// hairline around it. No shadow: GPUI fills one under the whole surface too, between the blur and
/// the tint, and it darkened the glass to near black. The overlay makes it take the pointer
/// (`occlude`); in Help, the page scrolls under it.
pub fn glass_panel(t: Theme) -> Div {
    div().relative().rounded(px(12.)).border_1().border_color(t.card_stroke).bg(pill_tint(t))
}

/// The pill's tint. Only macOS blurs behind the pill's window (`windows.rs`): elsewhere it's nearly
/// solid, as Windows' own acrylic is without its blur, or a page reads through the list.
pub fn pill_tint(t: Theme) -> Hsla {
    match cfg!(target_os = "macos") {
        false => Hsla { a: 0.95, ..t.sidebar },
        true => t.sidebar,
    }
}

/// The pill's buttons in a row, inside its glass.
pub fn pill_row(items: Vec<AnyElement>) -> Div {
    div().flex().items_center().gap(px(PILL_GAP)).p(px(4.)).children(items)
}

/// A button of the pill, without what it shows: `boxed`, a selected one draws its own glass pill
/// (the tool's is its slot's, easing with it).
pub fn button_face(emphasis: Emphasis, boxed: bool, t: Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .h(px(28.))
        .min_w(px(30.))
        .px(px(8.))
        .rounded(px(8.))
        .border_1()
        .border_color(transparent_black())
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(emphasis.ink(t))
        .when(emphasis == Emphasis::On && boxed, |d| d.bg(t.selected).border_color(t.selected_stroke))
        .when(emphasis == Emphasis::Primary, |d| d.bg(t.accent))
}

/// The pill's first button, the keys at a glance (`on`: showing).
pub fn help_face(on: bool, t: Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(8.))
        .when(on, |d| d.bg(t.selected))
        .child(icon("help", if on { Emphasis::On.ink(t) } else { Emphasis::Plain.ink(t) }))
}

/// The pill's microphone: listening (a soft red, filled up to `level`) or not (crossed out).
pub fn mic_face(listening: bool, level: f32, t: Theme) -> Div {
    let red = hex(style::RECORDING_SOFT, 1.0);
    // Listening, over the red fill: white on dark glass, the ink on light.
    let ink = match listening {
        true if t.dark => gpui::white(),
        true => t.primary,
        false => Emphasis::Plain.ink(t),
    };
    div()
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(8.))
        .when(listening, |d| d.bg(red.opacity(0.32)))
        // The fill: the button's whole shape, cut at the level (a short one rounded at its own
        // height overflowed the corners).
        .when(listening, |d| {
            d.child(
                div()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .w_full()
                    .h(gpui::relative(level.clamp(0.0, 1.0)))
                    .overflow_hidden()
                    .child(div().absolute().bottom_0().left_0().size(px(28.)).rounded(px(8.)).bg(red)),
            )
        })
        .child(icon(if listening { "mic" } else { "mic-off" }, ink))
}

/// The pill's camera: a clip of what happens next, recorded live (a soft red while recording, its
/// stop square).
pub fn clip_face(recording: bool, t: Theme) -> Div {
    let red = hex(style::RECORDING_SOFT, 1.0);
    // Recording, over the red: white on dark glass, the ink on light (as the microphone).
    let ink = match recording {
        true if t.dark => gpui::white(),
        true => t.primary,
        false => Emphasis::Plain.ink(t),
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .size(px(28.))
        .rounded(px(8.))
        .when(recording, |d| d.bg(red.opacity(0.32)))
        .child(icon(if recording { "clip-stop" } else { "clip" }, ink))
}

/// Buttons side by side as one control.
pub fn group(buttons: Vec<AnyElement>, t: Theme) -> AnyElement {
    div().flex().rounded(px(8.)).bg(t.hover).children(buttons).into_any_element()
}

/// The line between the pill's groups, with room to grab the pill by around it.
pub fn sep(t: Theme) -> AnyElement {
    div().w(px(1.)).h(px(18.)).mx(px(10.)).bg(t.hairline).into_any_element()
}

/// Where the aside goes, as the pill names it.
pub fn target_name(name: Option<SharedString>) -> Div {
    div().max_w(px(240.)).truncate().children(name)
}

/// A tool's icon and name, in its button.
pub fn tool_face(tool: Tool, t: Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(5.))
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(Emphasis::On.ink(t))
        .child(icon(tool_icon(tool), Emphasis::On.ink(t)))
        .child(tool.label())
}

/// The glass pill under the tool's button.
pub fn tool_box(t: Theme) -> Div {
    div().rounded(px(8.)).border_1().bg(t.selected).border_color(t.selected_stroke)
}

/// The pill as it stands, for Help: the keys at a glance (`help`, lit), the microphone (listening,
/// filled to `level`), the camera (recording a clip), where the aside goes (`list_open`, lit), the
/// tool unless it's Annotate (its slot, easing), start over, send and close. The overlay's own is
/// `ui::toolbar`.
pub struct Still {
    pub help: bool,
    pub listening: bool,
    pub level: f32,
    pub recording_clip: bool,
    pub target: AnyElement,
    pub list_open: bool,
    pub tool: Option<ToolSlot>,
}

/// The tool's slot on the pill, as the overlay's eases (`ui::toolbar`'s `slot`): as wide as it's drawn
/// (the gap before it its own: from nothing, it grows in), how much of it shows (fading as it grows in
/// or shrinks away), its box as wide, and the tools' names in it, how much each shows (one fading into
/// the other), in its middle, cut off on both sides while it's narrower.
pub struct ToolSlot {
    pub width: f32,
    pub shown: f32,
    pub tools: Vec<(Tool, f32)>,
}

/// How wide a tool's slot is, its name `name_width` wide: the gap before it, the button's border and
/// sides, the icon and the gap after it.
pub fn tool_slot_width(name_width: f32) -> f32 {
    PILL_GAP + 2.0 + 16.0 + 15.0 + 5.0 + name_width
}

pub fn still_pill(pill: Still, t: Theme) -> Div {
    still_pill_with(pill, t, &|_, button| button.into_any_element())
}

/// The pill as it stands, each button passed through `button` with its name (`help`, `mic`, `clip`,
/// `target`, `restart`, `done`, `close`): Help's pill to try adds what they do.
pub fn still_pill_with(pill: Still, t: Theme, button: &dyn Fn(&'static str, Div) -> AnyElement) -> Div {
    let plain = Emphasis::Plain.ink(t);
    let target = button_face(if pill.list_open { Emphasis::On } else { Emphasis::Plain }, true, t).child(pill.target);
    let first = group(
        vec![
            button("help", help_face(pill.help, t)),
            button("mic", mic_face(pill.listening, pill.level, t)),
            button("clip", clip_face(pill.recording_clip, t)),
            button("target", target),
        ],
        t,
    );
    let badge = pill.tool.filter(|slot| slot.width > 0.0).map(|slot| {
        let faces = slot.tools.into_iter().map(|(tool, shown)| {
            let face = button_face(Emphasis::On, false, t).flex_none().child(tool_face(tool, t));
            div().absolute().inset_0().flex().items_center().justify_center().opacity(shown).child(face)
        });
        div()
            .flex()
            .flex_none()
            .w(px(slot.width))
            .overflow_hidden()
            .opacity(slot.shown)
            .child(div().flex_none().w(px(PILL_GAP)))
            .child(
                div()
                    .relative()
                    .flex_none()
                    .w(px((slot.width - PILL_GAP).max(0.0)))
                    .h(px(28.))
                    .overflow_hidden()
                    .child(tool_box(t).absolute().inset_0())
                    .children(faces),
            )
    });
    let items = vec![
        div().flex().items_center().child(first).children(badge).into_any_element(),
        sep(t),
        group(
            vec![
                button("restart", button_face(Emphasis::Plain, true, t).child(icon("restart", plain))),
                button("done", button_face(Emphasis::Primary, true, t).child(icon("check", Emphasis::Primary.ink(t)))),
                button("close", button_face(Emphasis::Plain, true, t).child(icon("close", plain))),
            ],
            t,
        ),
    ];
    glass_panel(t).flex_none().child(pill_row(items))
}

// --- The keys at a glance ---

/// The keys under the pill: what asides does, and the keys for it, as `keys` has them;
/// `freeze` and `send` are the shortcuts, `live` says what the footer says.
pub fn keys_sheet_face(t: Theme, k: &Keys, freeze: &str, send: &str, live: bool) -> Div {
    let label = |c: Cmd| k.label(c);
    let where_it_goes = format!("Where it goes (↑ ↓, {})", keys::RETURN);
    let left: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "Pins",
            vec![
                ("Annotate (click, or drag)", label(Cmd::Tool(Tool::Annotate))),
                (
                    if cfg!(target_os = "macos") { "Draw (⇧ for an arrow)" } else { "Draw (Shift for an arrow)" },
                    label(Cmd::Tool(Tool::Draw)),
                ),
                ("Screenshot", label(Cmd::Tool(Tool::Shot))),
                ("Move it: drag a pin's number", String::new()),
                ("A pin's card (Tab: the next)", "1 9".into()),
                ("The last pin's type", format!("{c}1 {c}4", c = keys::COMMAND)),
                // Short: a longer name pushed the other column's keys past the sheet's edge.
                ("Follow up (again: hide)", label(Cmd::Restore)),
            ],
        ),
        (
            "Measuring",
            vec![
                ("Measure", label(Cmd::Tool(Tool::Measure))),
                ("Hold a distance", format!("{} {}", label(Cmd::HoldH), label(Cmd::HoldV))),
                ("Guide", format!("{} {}", label(Cmd::GuideH), label(Cmd::GuideV))),
                ("Tolerance", format!("{} {}", label(Cmd::TolDown), label(Cmd::TolUp))),
                ("Points or pixels", label(Cmd::Units)),
            ],
        ),
    ];
    let right: Vec<(&str, Vec<(&str, String)>)> = vec![
        (
            "Editing",
            vec![
                ("Undo", label(Cmd::Undo)),
                ("Redo", label(Cmd::Redo)),
                ("Clear", label(Cmd::Clear)),
                ("Hide everything", label(Cmd::Hide)),
                ("Copy as Markdown", label(Cmd::CopyMarkdown)),
            ],
        ),
        (
            "The aside",
            vec![
                ("Talk while muted (hold)", keys::label(&k.talk)),
                ("Mute or listen", label(Cmd::Mic)),
                ("Next session", label(Cmd::Target)),
                (&where_it_goes, label(Cmd::Sessions)),
                ("Live or frozen", freeze.to_string()),
                ("Record the whole screen, live", label(Cmd::Clip)),
                ("Record a pin's area (its card)", format!("{}K", keys::COMMAND)),
                ("Send", format!("{} {}", label(Cmd::Done), send)),
                ("Start over", label(Cmd::Restart)),
                ("Close", label(Cmd::Close)),
            ],
        ),
    ];
    let cap = |text: String| {
        div()
            .min_w(px(18.))
            .h(px(18.))
            .px(px(5.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .bg(t.selected)
            .border_1()
            .border_color(t.control_stroke)
            .text_size(px(11.))
            .child(text)
    };
    let column = |groups: Vec<(&str, Vec<(&str, String)>)>| {
        div().flex_1().flex().flex_col().gap(px(10.)).children(groups.into_iter().map(|(name, rows)| {
            div()
                .flex()
                .flex_col()
                .gap(px(5.))
                .child(
                    div()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.secondary)
                        .child(name.to_string()),
                )
                .children(rows.into_iter().map(|(label, keys)| {
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(px(12.))
                        .text_size(px(12.))
                        .child(div().text_color(t.primary).child(label.to_string()))
                        .child(
                            div()
                                .flex()
                                .gap(px(3.))
                                .children(keys.split(' ').filter(|k| !k.is_empty()).map(|k| cap(k.to_string()))),
                        )
                }))
        }))
    };
    let footer = match live {
        true => format!("Live, your apps have the keyboard: {freeze} freezes the screen again."),
        false => "Click a pin's number (or press it) to type about it, right-click for more.".into(),
    };
    glass_panel(t)
        // At least as wide: a wider font than a Mac's or Windows' (Linux's vary) makes it wider.
        .when(cfg!(target_os = "macos"), |d| d.w(px(KEYS_WIDTH as f32)))
        .when(!cfg!(target_os = "macos"), |d| d.min_w(px(KEYS_WIDTH as f32)))
        .p(px(14.))
        .flex()
        .flex_col()
        .gap(px(12.))
        .text_color(t.primary)
        .child(
            div()
                .text_size(px(12.))
                .text_color(t.secondary)
                .child("Click what's wrong or drag around it, say why, and send it."),
        )
        .child(div().flex().gap(px(22.)).child(column(left)).child(column(right)))
        // asides' soft blue reads on the dark glass only.
        .child(div().text_size(px(11.)).text_color(if t.dark { t.accent } else { t.secondary }).child(footer))
}

// --- The pill's list ---

/// The list's surface, under (or above) the pill.
pub fn list_panel(t: Theme) -> Div {
    glass_panel(t).w(px(LIST_WIDTH as f32)).text_size(px(13.)).text_color(t.primary)
}

/// Its rows, in a column.
pub fn list_rows() -> Div {
    div().p(px(4.)).flex().flex_col()
}

/// A row: a check where it's chosen, then what it is; lit (hovered, or the arrows on it), a glass
/// pill; dimmed where it can't be picked.
pub fn list_row_face(chosen: bool, lit: bool, enabled: bool, content: AnyElement, t: Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .min_h(px(28.))
        .px(px(8.))
        .py(px(3.))
        .rounded(px(6.))
        .border_1()
        .border_color(transparent_black())
        .when(lit, |d| d.bg(t.selected).border_color(t.selected_stroke))
        .when(!enabled, |d| d.opacity(0.55))
        .child(div().flex_none().w(px(14.)).when(chosen, |d| d.child(icon("check", t.primary))))
        .child(content)
}

/// A row's name, and what it is under it.
pub fn list_lines(title: SharedString, detail: Option<SharedString>, t: Theme) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .child(div().truncate().child(title))
        .children(detail.map(|d| div().text_size(px(11.)).text_color(t.secondary).truncate().child(d)))
        .into_any_element()
}

pub fn list_heading(text: &str, t: Theme) -> Div {
    div()
        .flex_none()
        .px(px(8.))
        .pt(px(6.))
        .pb(px(3.))
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.secondary)
        .child(text.to_string())
}

pub fn list_hairline(t: Theme) -> AnyElement {
    div().flex_none().h(px(1.)).mx(px(8.)).my(px(4.)).bg(t.hairline).into_any_element()
}

/// New session's heading, the agents beside it (`agents`, a segmented control's track).
pub fn new_session_row(agents: Div, t: Theme) -> AnyElement {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(12.))
        .pr(px(4.))
        .child(list_heading("New session", t))
        .child(agents)
        .into_any_element()
}

/// The agents' segmented control, without its segments: a glass pill slides to the one picked.
pub fn agents_track(t: Theme) -> Div {
    div().relative().flex().p(px(2.)).gap(px(2.)).rounded(px(7.)).bg(t.hover)
}

/// An agent in it (`on`: picked; the overlay draws its glass pill as a layer that slides, Help on
/// it).
pub fn agent_segment(name: &'static str, on: bool, t: Theme) -> Div {
    div()
        .h(px(20.))
        .px(px(8.))
        .flex()
        .items_center()
        .rounded(px(5.))
        .border_1()
        .border_color(transparent_black())
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(if on { t.primary } else { t.primary.opacity(0.7) })
        .child(name)
}

/// A folder a new session can start in: its name and where it is.
pub fn folder_content(name: String, path: String, t: Theme) -> AnyElement {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(icon("folder", t.secondary))
        .child(div().flex_none().child(name))
        .child(div().min_w_0().text_size(px(11.)).text_color(t.secondary).truncate().child(path))
        .into_any_element()
}

pub fn choose_folder_content(t: Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .child(icon("plus", t.secondary))
        .child(format!("Choose {}…", asides_core::paths::DIRECTORY_TITLE))
        .into_any_element()
}

// --- A pin's card ---

/// What a pin asks for, a dot (`on`: chosen, ringed).
pub fn intent_dot(intent: Intent, on: bool, t: Theme) -> Div {
    div()
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .when(on, |d| d.border_2().border_color(t.primary))
        .child(div().size(px(if on { 10. } else { 11. })).rounded_full().bg(hex(style::intent_color(intent), 1.0)))
}

/// A button on a card (or the tooltip of a clip of its pin's area): its icon, as tall as the field.
pub fn card_button(name: &str, t: Theme) -> Div {
    div()
        .flex()
        .items_center()
        .justify_center()
        .w(px(28.))
        .h(px(FIELD_HEIGHT))
        .rounded(px(7.))
        .child(icon(name, t.primary))
}

/// Draw or Measure, in the card (`on`: the tool picked).
pub fn card_tool(tool: Tool, on: bool, t: Theme) -> Div {
    card_button(tool_icon(tool), t).when(on, |d| d.bg(t.selected))
}

/// The card's camera: a clip of its pin's part of the screen, live.
pub fn card_camera(t: Theme) -> Div {
    card_button("clip", t)
}

/// How tall a card's field is on one line (`input::TextInput`: its line, padding and hairline): the
/// microphone beside it, and the tooltip's buttons, are as tall.
pub const FIELD_HEIGHT: f32 = 32.0;

/// The card's microphone: red while it records.
pub fn talk_face(recording: bool, t: Theme) -> Div {
    // White on the red while recording.
    let ink = if recording { gpui::white() } else { t.primary };
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(5.))
        .h(px(FIELD_HEIGHT))
        .min_w(px(30.))
        .px(px(7.))
        .rounded(px(7.))
        .text_size(px(12.))
        .text_color(ink)
        .when(recording, |d| d.bg(hex(style::RECORDING, 1.0)))
        .child(icon("mic", ink))
}

/// The card's surface (on its glass), its field, dots, tools and microphone in a row.
pub fn card_surface(glass: Div, width: f32, row: Vec<AnyElement>, t: Theme) -> Div {
    glass
        .w(px(width))
        .p(px(6.))
        .flex()
        .flex_col()
        .gap(px(4.))
        .text_color(t.primary)
        .child(div().flex().items_start().gap(px(6.)).children(row))
}

/// The card's dots, or its tools, side by side.
pub fn card_cluster(items: impl IntoIterator<Item = AnyElement>) -> AnyElement {
    div().flex().items_center().h(px(FIELD_HEIGHT)).gap(px(1.)).children(items).into_any_element()
}

/// A text field on a card, as `input::TextInput` draws it with `ui::card_look`: what's typed, the
/// caret at its end (or the placeholder).
pub fn card_field(text: &str, caret: bool, t: Theme) -> AnyElement {
    field_face(text, "What should change?", caret, t)
}

/// A field as `card_field` draws it, saying `placeholder` while it's empty.
pub fn field_face(text: &str, placeholder: &str, caret: bool, t: Theme) -> AnyElement {
    let empty = text.is_empty();
    div()
        .flex_1()
        .child(
            div()
                .flex()
                .items_center()
                .w_full()
                .px_2()
                .py_1p5()
                .rounded_md()
                .border_1()
                .border_color(t.card_stroke)
                .text_size(px(13.))
                .line_height(px(18.))
                .text_color(if empty { t.tertiary } else { t.primary })
                .when(empty && caret, |d| d.child(div().w(px(2.)).h(px(18.)).bg(t.accent)))
                .child(if empty { placeholder.to_string() } else { text.to_string() })
                .when(!empty && caret, |d| d.child(div().w(px(2.)).h(px(18.)).bg(t.accent))),
        )
        .into_any_element()
}

// --- A clip of a pin's area ---

/// The tooltip beside a pin's area while a clip of it records (`ui::ClipTipView`), as the pin's card
/// (`card_surface`): what happens there (`field`, the pin's own), the microphone, then the ways to
/// stop (`stops`: staying live, or freezing again), what was said about it, then the recording
/// (`status`).
pub fn clip_tip_face(
    glass: Div,
    field: AnyElement,
    talk: AnyElement,
    stops: Vec<AnyElement>,
    said: Option<AnyElement>,
    status: AnyElement,
    t: Theme,
) -> Div {
    let row = vec![div().flex_1().min_w_0().child(field).into_any_element(), talk, card_cluster(stops)];
    card_surface(glass, CLIP_TIP_WIDTH as f32, row, t).children(said).child(status)
}

/// The recording, in the tooltip: a red dot and how long so far, then the name of what the pointer is
/// on there (`hint`), with room for all of it.
pub fn clip_tip_status(seconds: u32, hint: Option<SharedString>, t: Theme) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(CLIP_TIP_STATUS))
        .pl(px(4.))
        .text_size(px(12.))
        .child(div().flex_none().size(px(8.)).rounded_full().bg(hex(style::RECORDING, 1.0)))
        .child(tabular(div().flex_none().text_color(t.tertiary).child(format!("{}:{:02}", seconds / 60, seconds % 60))))
        .children(hint.map(|hint| div().pl(px(4.)).min_w_0().truncate().text_color(t.secondary).child(hint)))
        .into_any_element()
}

/// A way to end a clip of a pin's area, after its tooltip's microphone: stopping and staying live,
/// stopping and freezing again, or throwing it away, back on the frozen screen it was asked from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ClipStop {
    Live,
    Freeze,
    Discard,
}

impl ClipStop {
    pub const ALL: [ClipStop; 3] = [ClipStop::Live, ClipStop::Freeze, ClipStop::Discard];

    /// Its button's icon: the pill's stop square, a snowflake, the card's trash.
    pub fn icon(self) -> &'static str {
        match self {
            ClipStop::Live => "clip-stop",
            ClipStop::Freeze => "freeze",
            ClipStop::Discard => "trash",
        }
    }
}

/// Where the tooltip goes beside a pin's area (`area`, in its screen's points from the top left; the
/// screen `screen` big, its menu bar `top` tall), `tip` big: under the outline around the area, else
/// above it, else inside it at its bottom (a clip leaves asides' windows out where it can); its left
/// side with the outline's, on the screen.
pub fn clip_tip_place(area: PtRect, (width, height): (f64, f64), top: f64, (w, h): (f64, f64)) -> PtRect {
    const EDGE: f64 = 8.0;
    let room = CLIP_OUTLINE + CLIP_TIP_GAP;
    let x = (area.x - CLIP_OUTLINE).clamp(EDGE, (width - w - EDGE).max(EDGE));
    let (below, above) = (area.bottom() + room, area.y - room - h);
    let y = match () {
        _ if below + h <= height - EDGE => below,
        _ if above >= top + EDGE => above,
        _ => (area.bottom() - h - EDGE).clamp(top + EDGE, (height - h - EDGE).max(top + EDGE)),
    };
    PtRect::new(x.round(), y.round(), w, h)
}

/// How thick the outline around a pin's area is while a clip of it records (just outside it), the
/// tooltip's width, its height with nothing said yet (its hairline, padding, buttons and recording),
/// and its gap from the outline.
pub const CLIP_OUTLINE: f64 = 3.0;
pub const CLIP_TIP_WIDTH: f64 = 380.0;
pub const CLIP_TIP_HEIGHT: f64 = 2.0 + 12.0 + FIELD_HEIGHT as f64 + 4.0 + CLIP_TIP_STATUS as f64;
/// The tooltip's line with the recording on it: as tall as its text.
const CLIP_TIP_STATUS: f32 = 18.0;
pub const CLIP_TIP_GAP: f64 = 8.0;

// --- On the frozen screen ---

/// A pin's number, in its color.
pub fn marker_face(number: u32, ink: Hsla) -> Div {
    div()
        .size(px(MARKER as f32))
        .rounded_full()
        .bg(ink)
        .border_2()
        .border_color(gpui::white())
        .shadow_md()
        .flex()
        .items_center()
        .justify_center()
        .text_color(gpui::white())
        .text_size(px(11.))
        .font_weight(FontWeight::BOLD)
        .child(number.to_string())
}

/// A glass label (`glass`, the overlay's over the frozen screen), centered on `(x, y)`. Its digits
/// are all as wide, and it's never narrower than a short number: a measurement changing as it's
/// dragged doesn't make it jump.
pub fn label_at(glass: Div, t: Theme, text: impl Into<SharedString>, (x, y): (f64, f64)) -> AnyElement {
    let label = glass
        .min_w(px(LABEL_MIN_WIDTH))
        .flex()
        .justify_center()
        .px(px(6.))
        .py(px(1.5))
        .text_color(t.primary)
        .text_size(px(11.))
        .font_weight(FontWeight::SEMIBOLD);
    div()
        .absolute()
        .left(px(x as f32 - 150.))
        .top(px(y as f32 - 10.))
        .w(px(300.))
        .h(px(20.))
        .flex()
        .justify_center()
        .items_center()
        .child(tabular(label).child(text.into()))
        .into_any_element()
}

/// Text with its figures all as wide (a label's digits don't jump as they change).
pub fn tabular(mut text: Div) -> Div {
    text.text_style().get_or_insert_with(Default::default).font_features =
        Some(gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));
    text
}

/// A surface on the frozen screen as Help draws it: the overlay's glass (`ui::frozen_glass`) over a
/// plain stage, where the blur behind it would change nothing.
pub fn plain_glass(t: Theme, radius: f32) -> Div {
    let tint = div().absolute().top_0().left_0().size_full().rounded(px(radius - 1.)).bg(t.sidebar);
    div().relative().rounded(px(radius)).border_1().border_color(t.card_stroke).child(tint)
}

/// What a frozen screen paints, in window points.
pub enum Prim {
    Fill(PtRect, Hsla),
    Border(PtRect, Hsla, f32, bool),
    Stroke(Vec<(f64, f64)>, f32, Hsla),
    Polygon(Vec<(f64, f64)>, Hsla),
}

pub fn paint(prim: Prim, window: &mut Window) {
    let b =
        |r: PtRect| Bounds::new(point(px(r.x as f32), px(r.y as f32)), size(px(r.width as f32), px(r.height as f32)));
    let p = |(x, y): (f64, f64)| point(px(x as f32), px(y as f32));
    match prim {
        Prim::Fill(r, c) => window.paint_quad(gpui::fill(b(r), c)),
        Prim::Border(r, c, w, dashed) => window.paint_quad(quad(
            b(r),
            px(0.),
            transparent_black(),
            px(w),
            c,
            if dashed { BorderStyle::Dashed } else { BorderStyle::Solid },
        )),
        Prim::Stroke(points, w, c) => {
            if points.len() < 2 {
                return;
            }
            let mut builder = PathBuilder::stroke(px(w));
            builder.move_to(p(points[0]));
            for q in &points[1..] {
                builder.line_to(p(*q));
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, c);
            }
        }
        Prim::Polygon(points, c) => {
            let mut builder = PathBuilder::fill();
            builder.move_to(p(points[0]));
            for q in &points[1..] {
                builder.line_to(p(*q));
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, c);
            }
        }
    }
}

pub fn arrow_prims(prims: &mut Vec<Prim>, from: (f64, f64), to: (f64, f64), c: Hsla) {
    if (to.0 - from.0).hypot(to.1 - from.1) < 2.0 {
        return;
    }
    let [tip, left, right] = style::arrow_head(from, to);
    let base = ((left.0 + right.0) / 2.0, (left.1 + right.1) / 2.0);
    prims.push(Prim::Stroke(vec![from, base], style::DRAW_WIDTH, c));
    prims.push(Prim::Polygon(vec![tip, left, right], c));
}
