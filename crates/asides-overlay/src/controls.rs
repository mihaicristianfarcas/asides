//! The asides window's look, after pingpong's (`pingpong-ui`): quiet surfaces, one hairline weight,
//! a glass pill behind what's selected, settings as cards of rows that each say what they do, color
//! kept for status, and asides' soft blue for the one main action. Warm grays in dark mode, white in
//! light mode. Each control takes the [`Theme`] it paints with.

use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Animation, AnimationElement, AnimationExt, AnyElement, AnyView, App, BorderStyle, Bounds, BoxShadow, ClickEvent,
    DispatchPhase, Div, ElementId, Entity, FontWeight, HitboxBehavior, Hsla, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, Pixels, ScrollHandle, SharedString, Stateful, Window, WindowAppearance, anchored, canvas, deferred,
    div, point, prelude::*, px, quad, size, svg,
};

use crate::style;

/// What a control calls with its new value.
type Handler<T> = Rc<dyn Fn(T, &mut Window, &mut App)>;
type OnClick = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Type sizes (points).
pub struct Type;

impl Type {
    pub const META: f32 = 11.0;
    pub const BODY: f32 = 13.0;
    pub const TITLE: f32 = 15.0;
    pub const DISPLAY: f32 = 22.0;
}

pub struct Radius;

impl Radius {
    pub const CHIP: f32 = 5.0;
    pub const CONTROL: f32 = 6.0;
    pub const ROW: f32 = 7.0;
    pub const CARD: f32 = 10.0;
    pub const PANEL: f32 = 12.0;
}

pub struct Metrics;

impl Metrics {
    /// The strip along the top of a column: the traffic lights over the sidebar, a page's actions.
    pub const TOOLBAR: f32 = 44.0;
    pub const SIDEBAR: f32 = 216.0;
    pub const NAV_ROW: f32 = 28.0;
    pub const SETTINGS_ROW: f32 = 50.0;
    pub const CONTROL: f32 = 26.0;
    /// A page's column.
    pub const FORM: f32 = 640.0;
}

/// What floats over the window paints in this order (`deferred` priorities).
pub const MENU_LAYER: usize = 3;

// --- Transitions ---

/// How long what opens and closes (menus, lists, pages, cards, tips) takes to fade.
pub const FADE: Duration = Duration::from_millis(140);

/// Eased out: quick to start, gentle to settle.
pub fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// `element` fades in when it first shows under `id`, rising `rise` points into place (its top
/// margin: none of its own).
pub fn fade_in<E: IntoElement + Styled + 'static>(
    element: E,
    id: impl Into<ElementId>,
    rise: f32,
) -> AnimationElement<E> {
    element
        .with_animation(id, Animation::new(FADE).with_easing(ease), move |el, d| el.opacity(d).mt(px(rise * (1.0 - d))))
}

/// A selection whose glass pill slides from one item to the next: each item notes where it's laid
/// out (`Glide::item`), and a layer behind them paints the pill there (`Glide::layer`), after.
#[derive(Clone)]
pub struct Glide(Entity<GlideState>);

#[derive(Default)]
pub struct GlideState {
    /// Where each item is in the window, this frame.
    items: HashMap<usize, Bounds<Pixels>>,
    selected: Option<usize>,
    /// Where the pill slides from, and since when; where it was last drawn.
    from: Option<(Bounds<Pixels>, Instant)>,
    last: Option<Bounds<Pixels>>,
}

impl Glide {
    pub fn new(cx: &mut App) -> Glide {
        Glide(cx.new(|_| GlideState::default()))
    }

    /// One kept in the window under `id` (for a control drawn anew each frame).
    pub fn keyed(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> Glide {
        Glide(window.use_keyed_state(id, cx, |_, _| GlideState::default()))
    }

    /// Goes in item `i`, which is `relative()`: notes where it is.
    pub fn item(&self, i: usize) -> impl IntoElement + use<> {
        let state = self.0.clone();
        canvas(
            move |bounds, _, cx| {
                state.update(cx, |s, _| {
                    s.items.insert(i, bounds);
                })
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    /// Goes behind the items (their container's first child): the pill behind item `selected`.
    pub fn layer(&self, selected: Option<usize>, fill: Hsla, stroke: Hsla, radius: f32) -> impl IntoElement + use<> {
        let state = self.0.clone();
        canvas(
            |_, _, _| (),
            move |_, _, window, cx| {
                let (rect, moving) = state.update(cx, |s, _| s.step(selected));
                if let Some(r) = rect {
                    window.paint_quad(quad(r, px(radius), fill, px(1.), stroke, BorderStyle::Solid));
                }
                if moving {
                    window.request_animation_frame();
                }
            },
        )
        // It paints where the items are: it takes no room (in a scroll area, none to scroll).
        .absolute()
        .top_0()
        .left_0()
        .size_0()
    }
}

impl GlideState {
    /// Where the pill is now, and whether it's still sliding. Selected anew, it slides from where
    /// it was; the same one moving (scrolled), it follows.
    fn step(&mut self, selected: Option<usize>) -> (Option<Bounds<Pixels>>, bool) {
        let target = selected.and_then(|i| self.items.get(&i).copied());
        if selected != self.selected {
            self.from = self.last.filter(|_| target.is_some()).map(|last| (last, Instant::now()));
            self.selected = selected;
        }
        let (rect, moving) = match (self.from, target) {
            (Some((from, since)), Some(to)) => {
                let t = since.elapsed().as_secs_f32() / FADE.as_secs_f32();
                (Some(lerp(from, to, ease(t))), t < 1.0)
            }
            _ => (target, false),
        };
        if !moving {
            self.from = None;
        }
        self.last = rect;
        // Measured again next frame.
        self.items.clear();
        (rect, moving)
    }
}

fn lerp(a: Bounds<Pixels>, b: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    let l = |x: Pixels, y: Pixels| x + (y - x) * t;
    Bounds::new(
        point(l(a.origin.x, b.origin.x), l(a.origin.y, b.origin.y)),
        size(l(a.size.width, b.size.width), l(a.size.height, b.size.height)),
    )
}

/// What fades in as it opens and out as it closes: open or not, and since when.
#[derive(Clone, Copy, Debug, Default)]
pub struct Fade {
    open: bool,
    since: Option<Instant>,
}

impl Fade {
    pub fn new(open: bool) -> Fade {
        Fade { open, since: None }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn set(&mut self, open: bool) {
        if self.open != open {
            *self = Fade { open, since: Some(Instant::now()) };
        }
    }

    /// Still fading: draw it again next frame.
    pub fn moving(&self) -> bool {
        self.since.is_some_and(|since| since.elapsed() < FADE)
    }

    /// How much of it shows, 0 to 1.
    pub fn shown(&self) -> f32 {
        let t = self.since.map_or(1.0, |since| since.elapsed().as_secs_f32() / FADE.as_secs_f32());
        if self.open { ease(t) } else { 1.0 - ease(t) }
    }
}

/// A scroll area's bar, beside it (in a parent with `relative()`, after it): a thin thumb as long as
/// the part that shows, when there's more than shows. Dragged, it scrolls; a click beside it jumps
/// there.
pub fn scrollbar(id: impl Into<ElementId>, handle: &ScrollHandle, t: Theme) -> impl IntoElement {
    let (handle, id) = (handle.clone(), id.into());
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let view = handle.bounds().size.height;
            let max = handle.max_offset().height;
            if max <= px(0.5) || view <= px(0.) {
                return;
            }
            let length = (view * (view / (view + max))).max(px(28.)).min(bounds.size.height);
            let room = bounds.size.height - length;
            let thumb = |offset: Pixels| bounds.origin.y + room * (-offset / max).clamp(0., 1.);
            // Where the thumb was grabbed, from its top, while it's dragged.
            let grab = window.use_keyed_state(id.clone(), cx, |_, _| None::<Pixels>);
            let dragging = grab.read(cx).is_some();
            let hovered = hitbox.is_hovered(window) || dragging;
            let top = thumb(handle.offset().y);
            let width = if hovered { px(7.) } else { px(5.) };
            let rect = Bounds::new(point(bounds.right() - width - px(3.), top + px(2.)), size(width, length - px(4.)));
            let ink = t.primary.opacity(if hovered { 0.38 } else { 0.22 });
            window.paint_quad(quad(rect, px(3.5), ink, px(0.), ink, BorderStyle::Solid));
            // Scrolled to where the thumb's top goes.
            let scroll_to = {
                let handle = handle.clone();
                move |thumb_top: Pixels| {
                    let along = ((thumb_top - bounds.origin.y) / room).clamp(0., 1.);
                    handle.set_offset(point(handle.offset().x, -(max * along)));
                }
            };
            let (h, g, to) = (hitbox.clone(), grab.clone(), scroll_to.clone());
            window.on_mouse_event(move |e: &MouseDownEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble || !h.is_hovered(window) {
                    return;
                }
                let y = e.position.y;
                // On the thumb, it's held where it was grabbed; beside it, the thumb comes there.
                let at = if y >= top && y <= top + length { y - top } else { length / 2. };
                if !(y >= top && y <= top + length) {
                    to(y - at);
                }
                g.update(cx, |grab, _| *grab = Some(at));
                cx.stop_propagation();
                window.refresh();
            });
            let (g, to) = (grab.clone(), scroll_to);
            window.on_mouse_event(move |e: &MouseMoveEvent, _, window, cx| {
                if let Some(at) = *g.read(cx) {
                    if e.pressed_button.is_none() {
                        g.update(cx, |grab, _| *grab = None);
                    } else {
                        to(e.position.y - at);
                    }
                    window.refresh();
                }
            });
            window.on_mouse_event(move |_: &MouseUpEvent, _, window, cx| {
                if grab.read(cx).is_some() {
                    grab.update(cx, |grab, _| *grab = None);
                    window.refresh();
                }
            });
        },
    )
    .absolute()
    .top_0()
    .right_0()
    .h_full()
    .w(px(13.))
}

pub fn hex(value: u32, alpha: f32) -> Hsla {
    let mut c = Hsla::from(gpui::rgb(value));
    c.a = alpha;
    c
}

fn white(alpha: f32) -> Hsla {
    hex(0xffffff, alpha)
}

fn black(alpha: f32) -> Hsla {
    hex(0x000000, alpha)
}

/// Every color a page needs, for one appearance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub dark: bool,
    /// Text and marks, from the most to the least prominent.
    pub primary: Hsla,
    pub secondary: Hsla,
    pub tertiary: Hsla,
    pub quaternary: Hsla,
    /// Glass, after Diri's: the window blurs what's behind it, under these tints. The work surface
    /// (what's read) is the densest; the sidebar lets more of the desktop through.
    pub background: Hsla,
    pub sidebar: Hsla,
    /// Menus and tooltips.
    pub floating: Hsla,
    pub floating_stroke: Hsla,
    /// One-point separators.
    pub hairline: Hsla,
    /// A card of rows: its fill and outline.
    pub card: Hsla,
    pub card_stroke: Hsla,
    pub hover: Hsla,
    /// The glass pill behind a selected row or segment.
    pub selected: Hsla,
    pub selected_stroke: Hsla,
    /// A quiet button.
    pub control: Hsla,
    pub control_hover: Hsla,
    pub control_stroke: Hsla,
    /// asides' soft blue: the main action (Send), as on the pill.
    pub accent: Hsla,
    pub on_accent: Hsla,
    /// Status: on and well, needs attention, wrong or destructive.
    pub fresh: Hsla,
    pub attention: Hsla,
    pub danger: Hsla,
}

impl Theme {
    pub fn dark() -> Theme {
        Theme {
            dark: true,
            primary: hex(0xedebe7, 1.0),
            secondary: white(0.62),
            tertiary: white(0.42),
            quaternary: white(0.26),
            background: hex(0x1d1c1a, 0.82),
            sidebar: hex(0x262422, 0.6),
            floating: hex(0x2c2a28, 1.0),
            floating_stroke: white(0.09),
            hairline: white(0.07),
            card: white(0.055),
            card_stroke: white(0.10),
            hover: white(0.06),
            selected: white(0.10),
            selected_stroke: white(0.09),
            // Controls are near solid: on glass, translucent ones disappear over a light desktop.
            control: hex(0x3a3836, 0.92),
            control_hover: hex(0x45423f, 0.95),
            control_stroke: white(0.12),
            accent: hex(style::UI, 1.0),
            on_accent: hex(style::UI_INK, 1.0),
            fresh: hex(0x34c759, 1.0),
            attention: hex(0xf5a623, 1.0),
            danger: hex(0xf5453a, 1.0),
        }
    }

    pub fn light() -> Theme {
        Theme {
            dark: false,
            primary: hex(0x1c1b19, 1.0),
            secondary: black(0.58),
            tertiary: black(0.45),
            quaternary: black(0.26),
            background: hex(0xffffff, 0.86),
            sidebar: hex(0xf5f4f2, 0.74),
            floating: hex(0xffffff, 1.0),
            floating_stroke: black(0.10),
            hairline: black(0.08),
            card: hex(0xffffff, 0.6),
            card_stroke: black(0.09),
            hover: black(0.045),
            selected: hex(0xffffff, 0.92),
            selected_stroke: black(0.09),
            control: hex(0xffffff, 0.94),
            control_hover: hex(0xffffff, 1.0),
            control_stroke: black(0.12),
            accent: hex(style::UI, 1.0),
            on_accent: hex(style::UI_INK, 1.0),
            fresh: hex(0x248a3d, 1.0),
            attention: hex(0xb26a00, 1.0),
            danger: hex(0xd70015, 1.0),
        }
    }

    pub fn of(appearance: WindowAppearance) -> Theme {
        match appearance {
            WindowAppearance::Dark | WindowAppearance::VibrantDark => Theme::dark(),
            WindowAppearance::Light | WindowAppearance::VibrantLight => Theme::light(),
        }
    }
}

pub fn font() -> SharedString {
    ".SystemUIFont".into()
}

pub fn icon(name: &str, size: f32, color: Hsla) -> impl IntoElement + use<> {
    svg().path(format!("icons/{name}.svg")).flex_none().size(px(size)).text_color(color)
}

pub fn lift(alpha: f32) -> Vec<BoxShadow> {
    vec![BoxShadow { color: black(alpha), offset: point(px(0.), px(1.)), blur_radius: px(3.), spread_radius: px(0.) }]
}

/// Menus and tooltips: a raised panel.
pub fn floating(t: Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .rounded(px(Radius::PANEL))
        .bg(t.floating)
        .border_1()
        .border_color(t.floating_stroke)
        .shadow(vec![BoxShadow {
            color: black(if t.dark { 0.40 } else { 0.16 }),
            offset: point(px(0.), px(14.)),
            blur_radius: px(36.),
            spread_radius: px(0.),
        }])
        .font_family(font())
        .text_size(px(Type::BODY))
        .text_color(t.primary)
}

// --- Buttons ---

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ButtonKind {
    /// Bordered and quiet: most actions.
    #[default]
    Quiet,
    /// asides' soft blue: the one main action on a page.
    Accent,
    /// No chrome until hovered: icon buttons along a strip.
    Ghost,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<&'static str>,
    kind: ButtonKind,
    theme: Theme,
    disabled: bool,
    tooltip: Option<SharedString>,
    on_click: Option<OnClick>,
}

pub fn button(id: impl Into<ElementId>, label: impl Into<SharedString>, theme: Theme) -> Button {
    Button {
        id: id.into(),
        label: Some(label.into()),
        icon: None,
        kind: ButtonKind::Quiet,
        theme,
        disabled: false,
        tooltip: None,
        on_click: None,
    }
}

/// A square button with only an icon, and a tooltip saying what it does.
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    tooltip: impl Into<SharedString>,
    theme: Theme,
) -> Button {
    Button {
        id: id.into(),
        label: None,
        icon: Some(icon),
        kind: ButtonKind::Ghost,
        theme,
        disabled: false,
        tooltip: Some(tooltip.into()),
        on_click: None,
    }
}

impl Button {
    pub fn kind(mut self, kind: ButtonKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn accent(self) -> Self {
        self.kind(ButtonKind::Accent)
    }

    pub fn icon(mut self, name: &'static str) -> Self {
        self.icon = Some(name);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.theme;
        let icon_only = self.label.is_none();
        let clear = hex(0, 0.0);
        let (fill, hover, stroke, ink) = match self.kind {
            ButtonKind::Quiet => (t.control, t.control_hover, Some(t.control_stroke), t.primary),
            ButtonKind::Accent => (t.accent, hex(style::UI_LIGHT, 1.0), None, t.on_accent),
            ButtonKind::Ghost => (clear, t.hover, None, t.secondary),
        };
        let ink = if self.disabled { ink.opacity(0.45) } else { ink };
        let mut el = div()
            .id(self.id)
            .flex_none()
            .h(px(Metrics::CONTROL))
            .when(icon_only, |d| d.w(px(Metrics::CONTROL)).justify_center())
            .when(!icon_only, |d| d.px(px(10.)))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(Radius::CONTROL))
            .bg(fill)
            .when_some(stroke, |d, s| d.border_1().border_color(s))
            .text_size(px(12.))
            .font_weight(if self.kind == ButtonKind::Accent { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
            .text_color(ink)
            .whitespace_nowrap()
            .when_some(self.icon, |d, name| d.child(icon(name, if icon_only { 16. } else { 14. }, ink)))
            .when_some(self.label, |d, l| d.child(l));
        if !self.disabled {
            let on_click = self.on_click.clone();
            el = el
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .active(|s| s.opacity(0.82))
                .when_some(on_click, |d, f| d.on_click(move |e, w, cx| f(e, w, cx)));
        }
        if let Some(tip) = self.tooltip {
            el = el.tooltip(move |_, cx| tooltip(tip.clone(), cx));
        }
        el
    }
}

// --- Tooltips ---

pub struct Tooltip {
    text: SharedString,
}

impl Render for Tooltip {
    fn render(&mut self, window: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        let t = Theme::of(window.appearance());
        let tip = div()
            .font_family(font())
            .max_w(px(300.))
            .px(px(8.))
            .py(px(5.))
            .rounded(px(Radius::CONTROL))
            .bg(t.floating)
            .border_1()
            .border_color(t.floating_stroke)
            .shadow(lift(0.25))
            .text_size(px(Type::META + 0.5))
            .text_color(t.secondary)
            .child(self.text.clone());
        fade_in(tip, "tooltip", 0.)
    }
}

pub fn tooltip(text: impl Into<SharedString>, cx: &mut App) -> AnyView {
    let text = text.into();
    cx.new(|_| Tooltip { text }).into()
}

// --- Switch, segmented control, select ---

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    on: bool,
    theme: Theme,
    on_toggle: Option<Handler<bool>>,
}

pub fn switch(id: impl Into<ElementId>, on: bool, theme: Theme) -> Switch {
    Switch { id: id.into(), on, theme, on_toggle: None }
}

impl Switch {
    pub fn on_toggle(mut self, f: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Switch {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let t = self.theme;
        let on = self.on;
        // Off, a control's surface: on glass, a faint track disappears.
        let track = if on { t.fresh.opacity(if t.dark { 0.72 } else { 0.85 }) } else { t.control_hover };
        let knob = if t.dark { t.primary } else { hex(0xffffff, 1.0) };
        let toggle = self.on_toggle.clone();
        div()
            .id(self.id)
            .flex_none()
            .w(px(30.))
            .h(px(18.))
            .p(px(2.))
            .rounded(px(9.))
            .bg(track)
            .when(!on, |d| d.border_1().border_color(t.control_stroke).p(px(1.)))
            .flex()
            .when(on, |d| d.justify_end())
            .child(div().size(px(14.)).rounded(px(7.)).bg(knob).shadow(lift(0.18)))
            .cursor_pointer()
            .when_some(toggle, |d, f| d.on_click(move |_, w, cx| f(!on, w, cx)))
    }
}

#[derive(IntoElement)]
pub struct Segmented {
    id: SharedString,
    items: Vec<SharedString>,
    selected: Option<usize>,
    theme: Theme,
    on_select: Option<Handler<usize>>,
}

pub fn segmented(
    id: &str,
    items: impl IntoIterator<Item = impl Into<SharedString>>,
    selected: Option<usize>,
    theme: Theme,
) -> Segmented {
    Segmented {
        id: id.to_string().into(),
        items: items.into_iter().map(Into::into).collect(),
        selected,
        theme,
        on_select: None,
    }
}

impl Segmented {
    pub fn on_select(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Segmented {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = self.theme;
        let id = self.id.clone();
        // The glass pill slides to the segment picked.
        let glide = Glide::keyed(ElementId::Name(format!("{id}-glide").into()), window, cx);
        div()
            .id(ElementId::Name(self.id))
            .relative()
            .flex_none()
            .flex()
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(Radius::ROW))
            .bg(t.primary.opacity(0.06))
            .child(glide.layer(self.selected, t.selected, t.selected_stroke, Radius::CONTROL - 1.))
            .children(self.items.into_iter().enumerate().map(|(i, label)| {
                let on = self.selected == Some(i);
                let select = self.on_select.clone();
                div()
                    .id(ElementId::Name(format!("{id}-{i}").into()))
                    .relative()
                    .child(glide.item(i))
                    .h(px(22.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .rounded(px(Radius::CONTROL - 1.))
                    .border_1()
                    .border_color(hex(0, 0.0))
                    .text_size(px(12.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if on { t.primary } else { t.secondary })
                    .when(!on, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.text_color(t.primary))
                            .when_some(select, |d, f| d.on_click(move |_, w, cx| f(i, w, cx)))
                    })
                    .child(label)
            }))
    }
}

/// A choice in a select's menu: what it's called, and a line under it.
#[derive(Clone)]
pub struct Choice {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    /// A hairline above it: it starts a group of its own.
    pub separated: bool,
}

impl Choice {
    pub fn new(label: impl Into<SharedString>) -> Choice {
        Choice { label: label.into(), detail: None, separated: false }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Choice {
        self.detail = Some(detail.into());
        self
    }

    pub fn separated(mut self) -> Choice {
        self.separated = true;
        self
    }
}

/// Development: the select to open when it's first drawn (`ASIDES_WINDOW_OPEN`), to capture its menu.
fn opens_at_first(id: &str) -> bool {
    std::env::var("ASIDES_WINDOW_OPEN").is_ok_and(|open| open == id)
}

/// A button showing the choice made, which opens a menu of the choices.
#[derive(IntoElement)]
pub struct Select {
    id: SharedString,
    choices: Vec<Choice>,
    selected: Option<usize>,
    width: Option<f32>,
    theme: Theme,
    on_select: Option<Handler<usize>>,
}

pub fn select(id: &str, choices: Vec<Choice>, selected: Option<usize>, theme: Theme) -> Select {
    Select { id: id.to_string().into(), choices, selected, width: None, theme, on_select: None }
}

impl Select {
    pub fn width(mut self, w: f32) -> Self {
        self.width = Some(w);
        self
    }

    pub fn on_select(mut self, f: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
}

impl RenderOnce for Select {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = self.theme;
        let first = opens_at_first(&self.id);
        let open = window.use_keyed_state(ElementId::Name(self.id.clone()), cx, move |_, _| Fade::new(first));
        // A press on the button while the menu is open closes it (as a press outside the menu), and
        // its click doesn't open it again: the button's bounds, and when that press was.
        let button = window
            .use_keyed_state(ElementId::Name(format!("{}-button", self.id).into()), cx, |_, _| None::<Bounds<Pixels>>);
        let dismissed = window
            .use_keyed_state(ElementId::Name(format!("{}-dismissed", self.id).into()), cx, |_, _| None::<Instant>);
        let fade = *open.read(cx);
        if fade.moving() {
            window.request_animation_frame();
        }
        let shown = self
            .selected
            .and_then(|i| self.choices.get(i))
            .map(|c| c.label.clone())
            .unwrap_or_else(|| "Choose…".into());
        let (toggle, just_closed) = (open.clone(), dismissed.clone());
        let mut el = div()
            .id(ElementId::Name(self.id.clone()))
            .relative()
            .flex_none()
            .h(px(Metrics::CONTROL))
            .when_some(self.width, |d, w| d.w(px(w)))
            .px(px(9.))
            .flex()
            .items_center()
            .gap(px(6.))
            .rounded(px(Radius::CONTROL))
            .bg(t.control)
            .border_1()
            .border_color(t.control_stroke)
            .text_size(px(12.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(t.primary)
            .cursor_pointer()
            .hover(move |s| s.bg(t.control_hover))
            .on_click(move |_, _, cx| {
                if just_closed.update(cx, |at, _| at.take()).is_some_and(|at| at.elapsed() < Duration::from_secs(1)) {
                    return;
                }
                toggle.update(cx, |o, cx| {
                    o.set(!o.is_open());
                    cx.notify();
                })
            })
            .child(div().flex_1().min_w_0().truncate().child(shown))
            .child(icon("chevron-up-down", 12., t.tertiary))
            .child({
                let button = button.clone();
                canvas(move |bounds, _, cx| button.update(cx, |b, _| *b = Some(bounds)), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
            });
        // Open, or still fading out.
        if fade.shown() > 0.0 {
            let rows = self.choices.into_iter().enumerate().flat_map(|(i, c)| {
                let (select, close) = (self.on_select.clone(), open.clone());
                let line = c.separated.then(|| div().mx(px(8.)).my(px(4.)).h(px(1.)).bg(t.hairline).into_any_element());
                let row = menu_row(ElementId::Name(format!("{}-choice-{i}", self.id).into()), t)
                    .child(
                        div()
                            .w(px(14.))
                            .flex_none()
                            .when(self.selected == Some(i), |d| d.child(icon("check", 13., t.primary))),
                    )
                    .child(div().flex().flex_col().min_w_0().child(div().truncate().child(c.label)).when_some(
                        c.detail,
                        |d, detail| {
                            d.child(div().text_size(px(Type::META)).text_color(t.tertiary).truncate().child(detail))
                        },
                    ))
                    .on_click(move |_, w, cx| {
                        close.update(cx, |o, cx| {
                            o.set(false);
                            cx.notify();
                        });
                        if let Some(f) = &select {
                            f(i, w, cx);
                        }
                    })
                    .into_any_element();
                line.into_iter().chain([row])
            });
            let close = open.clone();
            let shown = fade.shown();
            // Under the button, its left edge on the button's.
            let below = button.read(cx).map(|b| point(b.left(), b.bottom() + px(4.)));
            el = el.child(
                deferred(
                    anchored().when_some(below, |a, at| a.position(at)).snap_to_window_with_margin(px(8.)).child(
                        div()
                            .mt(px(if below.is_some() { 0. } else { 30. }) - px(4. * (1. - shown)))
                            .opacity(shown)
                            .occlude()
                            .child(
                                floating(t)
                                    .min_w(px(self.width.unwrap_or(180.).max(180.)))
                                    .max_w(px(420.))
                                    .p(px(4.))
                                    .children(rows),
                            )
                            .on_mouse_down_out(move |e, _, cx| {
                                let closed = close.update(cx, |o, cx| {
                                    let was = o.is_open();
                                    if was {
                                        o.set(false);
                                        cx.notify();
                                    }
                                    was
                                });
                                if closed && button.read(cx).is_some_and(|b| b.contains(&e.position)) {
                                    dismissed.update(cx, |at, _| *at = Some(Instant::now()));
                                }
                            }),
                    ),
                )
                .with_priority(MENU_LAYER),
            );
        }
        el
    }
}

/// A row in a floating menu: hovered, it lifts as a glass pill.
pub fn menu_row(id: impl Into<ElementId>, t: Theme) -> Stateful<Div> {
    div()
        .id(id)
        .min_h(px(26.))
        .px(px(8.))
        .py(px(3.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(Radius::CONTROL))
        .border_1()
        .border_color(hex(0, 0.0))
        .text_size(px(Type::BODY))
        .cursor_pointer()
        .hover(move |s| s.bg(t.selected).border_color(t.selected_stroke))
}

// --- Pages, sections and rows ---

/// A page's title and a line under it.
pub fn page_header(title: impl Into<SharedString>, subtitle: Option<SharedString>, t: Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(3.))
        .child(div().text_size(px(Type::DISPLAY)).font_weight(FontWeight::SEMIBOLD).child(title.into()))
        .when_some(subtitle, |d, s| d.child(div().text_color(t.tertiary).child(s)))
}

/// A titled group of rows.
pub fn section(title: &str, t: Theme) -> Div {
    div().flex().flex_col().gap(px(6.)).when(!title.is_empty(), |d| {
        d.child(
            div()
                .px(px(2.))
                .text_size(px(Type::META))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.tertiary)
                .child(title.to_string()),
        )
    })
}

/// A card of rows, hairlines between them.
pub fn rows(rows: impl IntoIterator<Item = AnyElement>, t: Theme) -> Div {
    let mut card = div()
        .flex()
        .flex_col()
        .rounded(px(Radius::ROW + 1.))
        .border_1()
        .border_color(t.card_stroke)
        .bg(t.card)
        .overflow_hidden();
    for (i, row) in rows.into_iter().enumerate() {
        if i > 0 {
            card = card.child(div().mx(px(12.)).h(px(1.)).bg(t.hairline));
        }
        card = card.child(row);
    }
    card
}

/// A setting: its name and a line on what it does, the control at the right.
pub fn setting(
    label: impl Into<SharedString>,
    detail: Option<&str>,
    control: impl IntoElement,
    t: Theme,
) -> AnyElement {
    div()
        .min_h(px(Metrics::SETTINGS_ROW))
        .px(px(12.))
        .py(px(8.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(16.))
        .child(label_stack(label, detail.map(|d| d.to_string().into()), t))
        .child(div().flex_none().flex().items_center().gap(px(8.)).child(control))
        .into_any_element()
}

pub fn label_stack(label: impl Into<SharedString>, detail: Option<SharedString>, t: Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .gap(px(2.))
        .child(div().font_weight(FontWeight::MEDIUM).child(label.into()))
        .when_some(detail, |d, detail| {
            d.child(div().text_size(px(Type::META)).line_height(px(15.)).text_color(t.tertiary).child(detail))
        })
}

/// Small print under a card.
pub fn footnote(text: impl Into<SharedString>, t: Theme) -> Div {
    div().px(px(2.)).text_size(px(Type::META)).line_height(px(15.)).text_color(t.tertiary).child(text.into())
}

/// A compact label in a quiet pill.
pub fn chip(label: impl Into<SharedString>, tint: Hsla) -> Div {
    div()
        .flex_none()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(Radius::CHIP))
        .bg(tint.opacity(0.14))
        .text_size(px(Type::META))
        .line_height(px(15.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(tint)
        .whitespace_nowrap()
        .child(label.into())
}

/// A number between − and + buttons; `step` gets -1 or 1.
pub fn stepper(
    id: &str,
    value: impl Into<SharedString>,
    t: Theme,
    step: impl Fn(i32, &mut Window, &mut App) + 'static,
) -> Div {
    let step = Rc::new(step);
    let side = |dir: i32, glyph: &'static str| {
        let step = step.clone();
        div()
            .id(ElementId::Name(format!("{id}-{glyph}").into()))
            .size(px(Metrics::CONTROL - 4.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(Radius::CONTROL - 1.))
            .cursor_pointer()
            .hover(move |s| s.bg(t.control_hover))
            .on_click(move |_, w, cx| step(dir, w, cx))
            .child(icon(glyph, 12., t.secondary))
    };
    let mut number = div()
        .min_w(px(56.))
        .flex()
        .justify_center()
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.primary)
        .child(value.into());
    number.text_style().get_or_insert_with(Default::default).font_features =
        Some(gpui::FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])));
    div()
        .flex_none()
        .h(px(Metrics::CONTROL))
        .flex()
        .items_center()
        .px(px(1.))
        .rounded(px(Radius::CONTROL))
        .bg(t.control)
        .border_1()
        .border_color(t.control_stroke)
        .child(side(-1, "minus"))
        .child(number)
        .child(side(1, "plus"))
}

/// Keys as keycaps (`labels`), which a click records anew: waiting, it asks for them; without
/// any, it says so.
pub fn key_recorder(id: impl Into<ElementId>, labels: Vec<String>, recording: bool, t: Theme) -> Stateful<Div> {
    let chips: Vec<AnyElement> = if recording {
        vec![kbd("Type the keys…", t).border_color(t.accent).text_color(t.primary).into_any_element()]
    } else if labels.is_empty() {
        vec![div().px(px(4.)).text_size(px(12.)).text_color(t.tertiary).child("None").into_any_element()]
    } else {
        labels.into_iter().map(|label| kbd(label, t).into_any_element()).collect()
    };
    div()
        .id(id)
        .flex_none()
        .flex()
        .items_center()
        .gap(px(4.))
        .p(px(3.))
        .rounded(px(Radius::CONTROL))
        .cursor_pointer()
        .hover(move |s| s.bg(t.hover))
        .children(chips)
}

/// Colors to pick from (`0xRRGGBB`): a ring around the one picked.
pub fn swatches(
    id: &str,
    colors: &[u32],
    selected: Option<usize>,
    t: Theme,
    pick: impl Fn(usize, &mut Window, &mut App) + 'static,
) -> Div {
    let pick = Rc::new(pick);
    div().flex_none().flex().items_center().gap(px(4.)).children(colors.iter().enumerate().map(|(i, &rgb)| {
        let pick = pick.clone();
        let on = selected == Some(i);
        div()
            .id(ElementId::Name(format!("{id}-{i}").into()))
            .size(px(22.))
            .p(px(2.))
            .rounded_full()
            .border_2()
            .border_color(if on { t.primary.opacity(0.75) } else { hex(0, 0.0) })
            .cursor_pointer()
            .on_click(move |_, w, cx| pick(i, w, cx))
            .child(div().size_full().rounded_full().bg(hex(rgb, 1.0)))
    }))
}

/// A key or chord, as a keycap.
pub fn kbd(keys: impl Into<SharedString>, t: Theme) -> Div {
    div()
        .flex_none()
        .h(px(20.))
        .px(px(6.))
        .flex()
        .items_center()
        .rounded(px(Radius::CHIP))
        .border_1()
        .border_color(t.control_stroke)
        .bg(t.control)
        .text_size(px(Type::META + 0.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.secondary)
        .child(keys.into())
}

/// A page's own strip along the title bar, its actions at the right.
pub fn toolbar_row(leading: Option<AnyElement>, trailing: Vec<AnyElement>) -> Div {
    div()
        .flex_none()
        .h(px(Metrics::TOOLBAR))
        .px(px(14.))
        .flex()
        .items_center()
        .justify_between()
        .gap(px(6.))
        .child(div().min_w_0().flex().items_center().children(leading))
        .child(div().flex_none().flex().items_center().gap(px(6.)).children(trailing))
}
