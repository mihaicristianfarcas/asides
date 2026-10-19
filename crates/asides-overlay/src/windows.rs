//! The overlay's windows: one per screen while it's frozen, and the pill's, above them and over the
//! live screen, the same window both ways (it stays put, and looks the same). They open hidden; the
//! clock (`sync`) shows, hides and moves them, between GPUI's updates: a window can't change from
//! inside an event GPUI is handling for it, and what AppKit tells GPUI about a change (a new size,
//! the keyboard) is dropped while GPUI is updating. Each is drawn before it shows, so it never shows
//! what it showed last (an earlier freeze, where the pointer was then).

use std::time::{Duration, Instant};

use asides_core::helper::SpecScreen;
use asides_core::inspect::PtRect;
use gpui::{
    AnyWindowHandle, App, AppContext, AsyncApp, Bounds, Context, DisplayId, Entity, FontWeight, IntoElement,
    ParentElement, Render, Styled, Window, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, div,
    point, px, size,
};

use crate::native;
use crate::ui::{
    KEYS_WIDTH, LIST_GAP, LIST_ROWS_HEIGHT, LIST_WIDTH, Overlay, PILL_TOP, ScreenView, ScreenWindow, TOOLBAR_MARGIN,
    ToolbarView,
};

/// Rounded rectangles in a window's points, their radii and how much of each shows: where the
/// pill's glass is.
pub type Shapes = Vec<(PtRect, f64, f64)>;

/// The pill's window before it's first drawn, big enough to lay the pill out at its size (it then
/// fits it), and about how big the pill is until then.
const TOOLBAR_GUESS: (f64, f64) = (640.0, 120.0);
const PILL_GUESS: (f64, f64) = (300.0, 40.0);
/// Back in front, an app redraws as the active one (macOS draws one in the background
/// differently): the frozen screens stay up this much longer...
const HANDOFF_SETTLE: Duration = Duration::from_millis(50);
/// ...or, with the Dock showing, this much: it slides back in and only then drops the overlay's
/// icon (it was a regular app while frozen), about half a second in all (measured on macOS 26).
const HANDOFF_SETTLE_DOCK: Duration = Duration::from_millis(520);
/// The frozen screens give way after this at most, whatever came back.
const HANDOFF_TIMEOUT: Duration = Duration::from_millis(1000);
/// Shown, the frozen screens take the system after this at most, on screen or not.
const TAKEOVER_TIMEOUT: Duration = Duration::from_millis(200);

/// The frozen screens giving way: the system went back to the app the user was in (when, and
/// when it was seen back: in front, with the Dock and the menu bar where they were).
pub struct Handoff {
    since: Instant,
    back: Option<Instant>,
    /// The frozen screens show just the screen now, under the pill.
    swapped: bool,
}

impl Handoff {
    pub fn swapped(&self) -> bool {
        self.swapped
    }
}

/// The screen (index into the spec's) under the pointer.
pub fn pointer_screen(screens: &[SpecScreen]) -> Option<usize> {
    let (mx, my) = native::mouse();
    screens.iter().position(|s| {
        let f = s.frame;
        mx >= f.x && mx < f.x + f.width && my >= f.y && my < f.y + f.height
    })
}

/// A screen's window opened while standing by, hidden. Opening a window takes 25 ms (Metal compiles
/// GPUI's shaders for each, longer once its compiler has gone idle), and GPUI draws it as it opens:
/// the frozen screens take these instead.
pub struct Spare {
    handle: AnyWindowHandle,
    display: u32,
    frame: PtRect,
    number: isize,
    view: usize,
}

/// What a spare window draws as it opens (hidden): text in the sizes and weights the overlay's
/// labels and pill use, so the frozen screen it becomes finds the font matched and the glyphs
/// drawn in its atlas (its first draw took 7 ms longer).
struct WarmUp;

impl Render for WarmUp {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let text = "Claude Code () Codex OpenCode Measure Annotate Draw Shot 0123456789:‘’“” px pt × · \
                    abcdefghijklmnopqrstuvwxyz ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let line = |size: f32, weight: FontWeight| div().text_size(px(size)).font_weight(weight).child(text);
        div()
            .child(line(11., FontWeight::NORMAL))
            .child(line(12., FontWeight::NORMAL))
            .child(line(13., FontWeight::NORMAL))
            .child(line(12., FontWeight::SEMIBOLD))
            .child(line(12., FontWeight::BOLD))
    }
}

/// A spare window for the pill.
pub fn open_spare_toolbar(cx: &mut App) -> Option<Spare> {
    let frame = PtRect::new(0.0, 0.0, TOOLBAR_GUESS.0, TOOLBAR_GUESS.1);
    let mut made = (0, 0);
    let opened = cx.open_window(toolbar_options(None, frame), |window, cx| {
        made = native::make_toolbar(window, frame);
        native::set_glass(made.1, &[]);
        cx.new(|_| WarmUp)
    });
    match opened {
        Ok(handle) => Some(Spare { handle: handle.into(), display: 0, frame, number: made.0, view: made.1 }),
        Err(err) => {
            eprintln!("asides-overlay: no spare pill window: {err:#}");
            None
        }
    }
}

/// A spare window for each screen.
pub fn open_spares(cx: &mut App) -> Vec<Spare> {
    native::screens()
        .into_iter()
        .filter_map(|(display, frame)| {
            let display_id = cx.displays().into_iter().find(|d| u32::from(d.id()) == display).map(|d| d.id());
            let mut made = (0, 0);
            let opened = cx.open_window(screen_options(display_id, (frame.width, frame.height)), |window, cx| {
                made = native::make_overlay(window, frame);
                cx.new(|_| WarmUp)
            });
            match opened {
                Ok(handle) => Some(Spare { handle: handle.into(), display, frame, number: made.0, view: made.1 }),
                Err(err) => {
                    eprintln!("asides-overlay: no spare window: {err:#}");
                    None
                }
            }
        })
        .collect()
}

/// A frozen screen's window, `w` × `h` points (hidden until `sync` shows it).
fn screen_options(display_id: Option<DisplayId>, (w, h): (f64, f64)) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(w as f32), px(h as f32)),
        ))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        // Not opaque, though the screenshot covers every pixel: an opaque window occludes the
        // apps under it, and many (web views, Electron, terminals) stop drawing while occluded,
        // then show a stale, blurred frame for a moment when it goes.
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    }
}

/// A window per screen, to show it frozen (hidden until `sync` shows them).
fn open_screens(overlay: &Entity<Overlay>, cx: &mut App) {
    let spec = overlay.read(cx).spec().clone();
    let windows: Vec<ScreenWindow> = spec
        .screens
        .iter()
        .map(|s| {
            let (offset, size) = match spec.windowed {
                Some(r) => ((r.x, r.y), (r.width, r.height)),
                None => ((0.0, 0.0), (s.screen.width, s.screen.height)),
            };
            ScreenWindow { handle: None, focus: None, screen: s.frame, offset, size, number: 0, view: 0 }
        })
        .collect();
    let mut spares = overlay.update(cx, |o, _| {
        o.windows = windows;
        std::mem::take(&mut o.spares)
    });
    for (i, s) in spec.screens.iter().enumerate() {
        if spec.windowed.is_none()
            && let Some(at) = spares.iter().position(|w| w.display == s.display && w.frame == s.frame)
            && take_spare(overlay, i, spares.swap_remove(at), cx)
        {
            continue;
        }
        let (offset, (w, h)) = {
            let o = overlay.read(cx);
            (o.windows[i].offset, o.windows[i].size)
        };
        // AppKit's coordinates: from the bottom-left corner of the main screen.
        let frame = match spec.windowed {
            Some(_) => PtRect::new(s.frame.x + offset.0, s.frame.y + s.frame.height - offset.1 - h, w, h),
            None => s.frame,
        };
        let display_id = cx.displays().into_iter().find(|d| u32::from(d.id()) == s.display).map(|d| d.id());
        let o = overlay.clone();
        let opened = cx.open_window(screen_options(display_id, (w, h)), move |window, cx| {
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
    // The screens changed since they were opened.
    for spare in spares {
        let _ = spare.handle.update(cx, |_, window, _| window.remove_window());
    }
}

/// Makes a spare window screen `i`'s; false if it's gone.
fn take_spare(overlay: &Entity<Overlay>, i: usize, spare: Spare, cx: &mut App) -> bool {
    let o = overlay.clone();
    let focus = spare.handle.update(cx, |_, window, cx| {
        let view = window.replace_root(cx, |_, cx| ScreenView::new(o, i, cx));
        let focus = view.read(cx).focus_handle();
        window.focus(&focus);
        focus
    });
    let Ok(focus) = focus else { return false };
    overlay.update(cx, |o, _| {
        let w = &mut o.windows[i];
        (w.handle, w.focus, w.number, w.view) = (Some(spare.handle), Some(focus), spare.number, spare.view);
    });
    true
}

/// Where the pill goes on `screen`, `w` × `h` (its frame, in points from the screen's top left):
/// where the user put it, or at the top in the middle; always on the screen (in the development
/// window, on its part), below the menu bar and the notch.
fn pill_place(screen: &SpecScreen, windowed: Option<PtRect>, at: Option<(f64, f64)>, (w, h): (f64, f64)) -> PtRect {
    let f = screen.frame;
    let area = windowed.unwrap_or(PtRect::new(0.0, 0.0, f.width, f.height));
    let top = screen.menu_bar.max(area.y);
    let (x, y) = match at {
        Some((fx, fy)) => (fx * f.width, fy * f.height),
        None => (area.x + (area.width - w) / 2.0, top + PILL_TOP),
    };
    let x = x.clamp(area.x, (area.x + area.width - w).max(area.x)).round();
    let y = y.clamp(top, (area.y + area.height - h).max(top)).round();
    PtRect::new(x, y, w, h)
}

/// The pill's window on `screen` (AppKit's coordinates): the pill at `pill`, and its list (`size`
/// is both, as drawn) under it, or above it where there's more room there than below; to its right,
/// or to its left near the right edge; the margin around them. And which ways the list goes.
fn toolbar_frame(
    screen: &SpecScreen,
    windowed: Option<PtRect>,
    pill: PtRect,
    (w, h): (f64, f64),
) -> (PtRect, bool, bool) {
    let f = screen.frame;
    let area = windowed.unwrap_or(PtRect::new(0.0, 0.0, f.width, f.height));
    let list = LIST_GAP + LIST_ROWS_HEIGHT + 2.0;
    let left = pill.x + LIST_WIDTH.max(KEYS_WIDTH) > area.x + area.width;
    let (above, below) = (pill.y - screen.menu_bar.max(area.y), area.y + area.height - pill.y - pill.height);
    let up = below < list && above > below;
    let x = if left { pill.x + pill.width - w } else { pill.x } - TOOLBAR_MARGIN;
    let y = if up { pill.y + pill.height - h } else { pill.y } - TOOLBAR_MARGIN;
    let (width, height) = ((w + 2.0 * TOOLBAR_MARGIN).ceil(), (h + 2.0 * TOOLBAR_MARGIN).ceil());
    (PtRect::new(f.x + x, f.y + f.height - y - height, width, height), left, up)
}

/// The pill's window: a panel that never takes the keyboard, `frame` big (hidden until `sync`
/// shows it).
fn toolbar_options(display_id: Option<DisplayId>, frame: PtRect) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(frame.width as f32), px(frame.height as f32)),
        ))),
        titlebar: None,
        focus: false,
        show: false,
        kind: WindowKind::PopUp,
        is_movable: false,
        is_resizable: false,
        is_minimizable: false,
        display_id,
        // Glass, as Diri's sidebar: macOS blurs what's behind, only behind the pill's shapes.
        window_background: WindowBackgroundAppearance::Blurred,
        ..Default::default()
    }
}

/// The pill's window, on the screen under the pointer (the spare one, or a new one): hidden until
/// `sync` puts it where it goes and shows it.
fn open_toolbar(overlay: &Entity<Overlay>, cx: &mut App) {
    let (spec, spare) = overlay.update(cx, |o, _| (o.spec().clone(), o.spare_toolbar.take()));
    let Some(index) = pointer_screen(&spec.screens).or((!spec.screens.is_empty()).then_some(0)) else { return };
    let screen = &spec.screens[index];
    let window = |number, view, frame: PtRect| ScreenWindow {
        handle: None,
        focus: None,
        screen: screen.frame,
        offset: (0.0, 0.0),
        size: (frame.width, frame.height),
        number,
        view,
    };
    if let Some(spare) = spare {
        let o = overlay.clone();
        let taken = spare.handle.update(cx, |_, window, cx| {
            window.replace_root(cx, |_, cx| ToolbarView::new(o, cx));
        });
        if taken.is_ok() {
            let mut toolbar = window(spare.number, spare.view, spare.frame);
            toolbar.handle = Some(spare.handle);
            overlay.update(cx, |o, _| (o.toolbar_screen, o.toolbar_window) = (index, Some(toolbar)));
            return;
        }
    }
    let frame = PtRect::new(screen.frame.x, screen.frame.y, TOOLBAR_GUESS.0, TOOLBAR_GUESS.1);
    let display_id = cx.displays().into_iter().find(|d| u32::from(d.id()) == screen.display).map(|d| d.id());
    let o = overlay.clone();
    let opened = cx.open_window(toolbar_options(display_id, frame), move |w, cx| {
        let (number, view) = native::make_toolbar(w, frame);
        // Nothing blurred until the pill is drawn and its shapes are known (`follow_toolbar`).
        native::set_glass(view, &[]);
        o.update(cx, |o, _| (o.toolbar_screen, o.toolbar_window) = (index, Some(window(number, view, frame))));
        cx.new(|cx| ToolbarView::new(o.clone(), cx))
    });
    match opened {
        Ok(handle) => overlay.update(cx, |o, _| {
            if let Some(t) = &mut o.toolbar_window {
                t.handle = Some(handle.into());
            }
        }),
        Err(err) => eprintln!("asides-overlay: no pill window: {err:#}"),
    }
}

/// The windows the overlay needs and doesn't have yet: the screens' while frozen (again, when the
/// screens changed since), and the pill's.
fn open_missing(overlay: &Entity<Overlay>, cx: &mut App) {
    let o = overlay.read(cx);
    let screens = &o.spec().screens;
    let changed = o.windows.len() != screens.len() || o.windows.iter().zip(screens).any(|(w, s)| w.screen != s.frame);
    if !o.live && changed {
        let old: Vec<_> = o.windows.iter().filter_map(|w| w.handle).collect();
        for handle in old {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        open_screens(overlay, cx);
    }
    if overlay.read(cx).toolbar_window.is_none() {
        open_toolbar(overlay, cx);
    }
}

/// Puts the windows in line with the overlay: the screens' while frozen, the pill's always (where
/// it goes, fitting the pill and its list). From the clock, outside updates.
pub fn sync(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (live, shown, closing, handing, taking_over, toolbar) = cx.update(|cx| {
        let o = overlay.read(cx);
        if !o.closing {
            open_missing(overlay, cx);
        }
        let o = overlay.read(cx);
        let toolbar = o.toolbar_window.as_ref().map(|t| t.view);
        (o.live, o.shown, o.closing, o.handoff.is_some(), o.taking_over, toolbar)
    })?;
    if closing {
        // The aside is on its way: the pill goes at once, the frozen screens once the system is back.
        if let Some(toolbar) = toolbar {
            native::hide(toolbar);
        }
        if shown == Some(false) && !handed_back(overlay, cx)? {
            return Ok(());
        }
        return cx.update(|cx| cx.quit());
    }
    if shown.is_some() {
        hide_pill(overlay, cx)?;
        follow_toolbar(overlay, cx)?;
    }
    match (shown == Some(live), live) {
        (true, true) => Ok(()),
        // Frozen again while still giving way: the system comes back to the overlay.
        (true, false) if handing => show_frozen(overlay, cx),
        (true, false) => match taking_over {
            Some(since) => take_over(overlay, since, cx),
            None => drop_stale(overlay, cx),
        },
        (false, true) => show_live(overlay, cx),
        (false, false) => show_frozen(overlay, cx),
    }
}

/// Frozen, the glass under surfaces that moved since, out of the screens' windows.
fn drop_stale(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    cx.update(|cx| {
        let (handles, stale) = overlay.update(cx, |o, _| {
            let handles: Vec<_> = o.windows.iter().filter_map(|w| w.handle).collect();
            (handles, std::mem::take(&mut o.stale_images))
        });
        if stale.is_empty() {
            return;
        }
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| {
                for image in &stale {
                    let _ = window.drop_image(image.clone());
                }
            });
        }
    })
}

/// Frozen, the pill hides with everything on the screen (`Cmd::Hide`), and shows again with it.
fn hide_pill(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (toolbar, hide, hidden) = cx.update(|cx| {
        let o = overlay.read(cx);
        (o.toolbar_window.as_ref().map(|t| t.view), !o.live && o.session.hidden, o.pill_hidden)
    })?;
    let Some(toolbar) = toolbar.filter(|_| hide != hidden) else { return Ok(()) };
    match hide {
        true => native::hide(toolbar),
        false => {
            native::draw_now(&[toolbar]);
            native::show(toolbar, false);
        }
    }
    cx.update(|cx| overlay.update(cx, |o, _| o.pill_hidden = hide))
}

/// The pill's window, drawn at its size where it goes, before it shows: drawn once, the pill knows
/// its size, and the window fits it (`follow_toolbar` draws it again).
fn prepare_toolbar(overlay: &Entity<Overlay>, toolbar: usize, cx: &mut AsyncApp) -> anyhow::Result<()> {
    native::pointer_moved(toolbar);
    native::draw_now(&[toolbar]);
    follow_toolbar(overlay, cx)
}

/// Choose Folder…: its panel, after the click that asked for it (`native::choose_folder`). Live,
/// the keyboard goes back to the user's app once it closes; frozen, to the screen under the pointer.
pub fn ask_folder(overlay: &Entity<Overlay>, cx: &mut AsyncApp) {
    let Ok(Some(start)) = cx.update(|cx| overlay.update(cx, |o, _| o.take_folder_request())) else { return };
    let live = cx.update(|cx| overlay.read(cx).live).unwrap_or_default();
    let front = if live { native::front_app() } else { None };
    let (chosen, folder) = futures::channel::oneshot::channel();
    native::choose_folder(Some(&start), move |dir| {
        let _ = chosen.send(dir);
    });
    let overlay = overlay.clone();
    cx.spawn(async move |cx| {
        let dir = folder.await.ok().flatten();
        let key = cx.update(|cx| {
            overlay.update(cx, |o, cx| {
                o.folder_chosen(dir);
                cx.notify();
                let key = pointer_screen(&o.spec().screens).unwrap_or(0);
                (!o.live).then(|| o.windows.get(key).map(|w| w.view)).flatten()
            })
        });
        match key {
            Ok(Some(view)) => native::show(view, true),
            _ if live => native::give_focus(front),
            _ => {}
        }
    })
    .detach();
}

/// The frozen screens, drawn, under the pill; the one under the pointer takes the keyboard, and the
/// overlay's app the system (`native::kiosk`).
fn show_frozen(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (screens, key, toolbar, windowed, opening, hidden) = cx.update(|cx| {
        let o = overlay.read(cx);
        let views: Vec<usize> = o.windows.iter().map(|w| w.view).collect();
        let key = pointer_screen(&o.spec().screens).unwrap_or(0).min(views.len().saturating_sub(1));
        let toolbar = o.toolbar_window.as_ref().map(|t| t.view);
        (views, key, toolbar, o.spec().windowed.is_some(), o.shown.is_none(), o.session.hidden)
    })?;
    // GPUI still has the pointer where it was when they last showed (development windows show only
    // part of the screen: the pointer stays out of them).
    if !windowed && let Some(&view) = screens.get(key) {
        native::pointer_moved(view);
    }
    native::draw_now(&screens);
    // Opening, the pill first: over the live screen for a moment, it looks just the same.
    if opening && let Some(toolbar) = toolbar {
        prepare_toolbar(overlay, toolbar, cx)?;
        if !hidden {
            native::show(toolbar, false);
        }
    }
    let previous = native::front_app();
    for (i, &view) in screens.iter().enumerate() {
        native::show(view, i == key);
    }
    cx.update(|cx| {
        let (focus, handles, stale) = overlay.update(cx, |o, _| {
            o.shown = Some(false);
            o.handoff = None;
            o.taking_over = Some(Instant::now());
            if opening {
                o.pill_hidden = hidden;
            }
            // The app the user was in, for the keyboard to go back to.
            if previous.is_some() {
                o.previous_app = previous;
            }
            let focus = o.windows.get(key).and_then(|w| w.handle.zip(w.focus.clone()));
            let handles: Vec<_> = o.windows.iter().filter_map(|w| w.handle).collect();
            (focus, handles, std::mem::take(&mut o.stale_images))
        });
        if let Some((handle, focus)) = focus {
            let _ = handle.update(cx, |_, window, _| window.focus(&focus));
        }
        // The last freeze's screens, drawn no more.
        for handle in handles {
            let _ = handle.update(cx, |_, window, _| {
                for image in &stale {
                    let _ = window.drop_image(image.clone());
                }
            });
        }
    })
}

/// The frozen screens on screen: the system is the overlay's (`native::kiosk`), the Dock and the
/// menu bar hidden under them.
fn take_over(overlay: &Entity<Overlay>, since: Instant, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let screens = cx.update(|cx| overlay.read(cx).windows.iter().map(|w| w.view).collect::<Vec<usize>>())?;
    if !screens.iter().all(|&view| native::on_screen(view)) && since.elapsed() < TAKEOVER_TIMEOUT {
        return Ok(());
    }
    native::kiosk(true);
    cx.update(|cx| overlay.update(cx, |o, _| o.taking_over = None))
}

/// The live screen: the pill alone. From frozen, the frozen screens show just the screen at once
/// (the pill stays where it is, above them), and give way once the system is back with the app the
/// user was in (`handed_back`): with the Dock always shown, that's about half a second, which the
/// user doesn't wait on to see ▶ worked.
fn show_live(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (screens, toolbar, frozen, started, hidden) = cx.update(|cx| {
        let o = overlay.read(cx);
        let views: Vec<usize> = o.windows.iter().map(|w| w.view).collect();
        let toolbar = o.toolbar_window.as_ref().map(|t| t.view);
        (views, toolbar, o.shown == Some(false), o.handoff.is_some(), o.pill_hidden)
    })?;
    if frozen {
        if !started {
            // At once, while the overlay's app is still the active one (drawn once it's in the
            // background, the frozen screens didn't show it): they show just the screen
            // (`ScreenView`); then the system goes back to the user's app.
            cx.update(|cx| {
                overlay.update(cx, |o, cx| {
                    o.handoff = Some(Handoff { since: Instant::now(), back: None, swapped: true });
                    cx.notify();
                })
            })?;
            native::draw_now(&screens);
            return begin_handoff(overlay, cx);
        }
        if !handed_back(overlay, cx)? {
            return Ok(());
        }
    }
    // Opened live, or hidden with everything on the frozen screen.
    if let Some(toolbar) = toolbar.filter(|_| !frozen || hidden) {
        prepare_toolbar(overlay, toolbar, cx)?;
        native::show(toolbar, false);
    }
    for view in screens {
        native::hide(view);
    }
    cx.update(|cx| {
        overlay.update(cx, |o, _| {
            o.shown = Some(true);
            o.handoff = None;
            o.pill_hidden = false;
        })
    })
}

/// The system goes back to the app the user was in (the keyboard, the Dock and the menu bar), and
/// the overlay's app is an accessory again.
fn begin_handoff(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let previous = cx.update(|cx| {
        overlay.update(cx, |o, _| {
            let swapped = o.handoff.as_ref().is_some_and(|h| h.swapped);
            o.handoff = Some(Handoff { since: Instant::now(), back: None, swapped });
            o.taking_over = None;
            o.previous_app
        })
    })?;
    native::kiosk(false);
    native::give_focus(previous);
    Ok(())
}

/// Whether the frozen screens can give way: the first time, the system goes back to the app the
/// user was in (the keyboard, the Dock and the menu bar); then, once that app is in front with the
/// Dock and the menu bar where they were, and had a moment to draw itself as the active app. Until
/// then the frozen screens cover it all, so the user sees their app as they left it, at once.
fn handed_back(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<bool> {
    let (started, back, frames, visible) = cx.update(|cx| {
        let o = overlay.read(cx);
        let frames: Vec<PtRect> = o.spec().screens.iter().map(|s| s.frame).collect();
        let (started, back) = o.handoff.as_ref().map_or((None, None), |h| (Some(h.since), h.back));
        (started, back, frames, o.visible.clone())
    })?;
    let Some(since) = started else {
        begin_handoff(overlay, cx)?;
        return Ok(false);
    };
    if since.elapsed() >= HANDOFF_TIMEOUT {
        return Ok(true);
    }
    let same = |a: &PtRect, b: &PtRect| {
        (a.x - b.x).abs() < 1.0
            && (a.y - b.y).abs() < 1.0
            && (a.width - b.width).abs() < 1.0
            && (a.height - b.height).abs() < 1.0
    };
    let in_place = frames.iter().zip(&visible).all(|(f, v)| same(&native::visible_frame(*f), v));
    let is_back = !native::is_active() && in_place;
    // Where the Dock shows (the visible part is short of a side other than the top, the menu
    // bar's), it slides back in.
    let dock = frames
        .iter()
        .zip(&visible)
        .any(|(f, v)| v.y - f.y > 1.0 || v.x - f.x > 1.0 || (f.x + f.width) - (v.x + v.width) > 1.0);
    let settle = if dock { HANDOFF_SETTLE_DOCK } else { HANDOFF_SETTLE };
    match (is_back, back) {
        (true, Some(at)) => Ok(at.elapsed() >= settle),
        (true, None) => {
            cx.update(|cx| overlay.update(cx, |o, _| o.handoff.as_mut().map(|h| h.back = Some(Instant::now()))))?;
            Ok(false)
        }
        (false, _) => {
            cx.update(|cx| overlay.update(cx, |o, _| o.handoff.as_mut().map(|h| h.back = None)))?;
            Ok(false)
        }
    }
}

/// The pill's window: the pill where it goes (`pill_place`), under the pointer while it's moved,
/// and the window as big as the pill and its list as drawn (`toolbar_frame`). macOS blurs what's
/// behind them only (`native::set_glass`).
fn follow_toolbar(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let state = cx.update(|cx| {
        let o = overlay.read(cx);
        let p = |v: gpui::Pixels| f32::from(v) as f64;
        let size = |b: Option<gpui::Bounds<gpui::Pixels>>| b.map(|b| (p(b.size.width), p(b.size.height)));
        let (pill, whole) = (size(o.pill_bounds).unwrap_or(PILL_GUESS), size(o.toolbar_bounds));
        // The pill and, open, its list (fading with it): where macOS blurs what's behind (in the
        // window's points).
        let rect = |b: gpui::Bounds<gpui::Pixels>| {
            PtRect::new(p(b.origin.x), p(b.origin.y), p(b.size.width), p(b.size.height))
        };
        let (list, keys) = (o.list.shown(), o.keys_sheet.shown());
        let shapes: Shapes = o
            .pill_bounds
            .map(|b| (rect(b), 12.0, 1.0))
            .into_iter()
            .chain(o.list_bounds.filter(|_| list > 0.0).map(|b| (rect(b), 12.0, list as f64)))
            .chain(o.keys_bounds.filter(|_| keys > 0.0 && list == 0.0).map(|b| (rect(b), 12.0, keys as f64)))
            .collect();
        let (spec, sides) = (o.spec(), (o.list_left, o.list_up));
        o.toolbar_window.as_ref().map(|t| {
            let screens = spec.screens.clone();
            let place = (o.toolbar_at, o.toolbar_screen, o.pill_rect);
            (t.view, pill, whole.unwrap_or(pill), shapes, o.moving_toolbar, place, screens, spec.windowed, sides)
        })
    })?;
    let Some((view, pill, whole, shapes, moving, (mut at, mut index, last), screens, windowed, sides)) = state else {
        return Ok(());
    };
    let Some(frame) = native::frame(view) else { return Ok(()) };
    let (mut moved, mut let_go) = (false, false);
    if let Some((gx, gy)) = moving {
        if native::mouse_pressed() {
            // Its top-left corner, on the screen under the pointer.
            let (mx, my) = native::mouse();
            if let Some(i) = screens.iter().position(|s| {
                let r = s.frame;
                mx >= r.x && mx < r.x + r.width && my >= r.y && my <= r.y + r.height
            }) {
                let r = screens[i].frame;
                (at, index, moved) =
                    (Some(((mx - gx - r.x) / r.width, (r.y + r.height - my - gy) / r.height)), i, true);
            }
        } else {
            let_go = true;
        }
    }
    let Some(screen) = screens.get(index) else { return Ok(()) };
    let place = pill_place(screen, windowed, at, pill);
    let (f, left, up) = toolbar_frame(screen, windowed, place, whole);
    // Where it shows is where it's kept: let go past an edge, it comes back there next time too.
    if moved {
        at = Some((place.x / screen.frame.width, place.y / screen.frame.height));
    }
    let same = |a: PtRect, b: PtRect| {
        (a.x - b.x).abs() < 0.5
            && (a.y - b.y).abs() < 0.5
            && (a.width - b.width).abs() < 0.5
            && (a.height - b.height).abs() < 0.5
    };
    let resized = (f.width - frame.width).abs() >= 0.5 || (f.height - frame.height).abs() >= 0.5;
    if resized {
        native::set_frame(view, f);
    } else if !same(f, frame) {
        native::move_to(view, (f.x, f.y));
    }
    // The mask is drawn at the window's size: again when it's resized.
    let glass = cx.update(|cx| {
        let last = overlay.read(cx).glass_shapes.as_ref();
        (last.map(|(s, size)| (s, *size)) != Some((&shapes, (f.width, f.height)))).then_some(shapes)
    })?;
    if let Some(shapes) = &glass {
        native::set_glass(view, shapes);
    }
    // Moving, only the window moves; what goes with the pill catches up once it's let go.
    let changed = (left, up) != sides || (last != Some((index, place)) && (moving.is_none() || let_go));
    cx.update(|cx| {
        overlay.update(cx, |o, cx| {
            if let Some(shapes) = glass {
                o.glass_shapes = Some((shapes, (f.width, f.height)));
            }
            if moved {
                (o.toolbar_at, o.toolbar_screen) = (at, index);
            }
            if let_go {
                o.moving_toolbar = None;
                o.placed_toolbar();
            }
            if changed {
                (o.list_left, o.list_up, o.pill_rect) = (left, up, Some((index, place)));
                cx.notify();
            }
        })
    })?;
    if resized || changed {
        native::draw_now(&[view]);
    }
    Ok(())
}
