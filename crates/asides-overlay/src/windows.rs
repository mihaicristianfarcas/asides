//! The overlay's windows: one per screen while it's frozen, and the toolbar's over the live
//! screen. They open hidden; the clock (`sync`) shows, hides and moves them, between GPUI's
//! updates: a window can't change from inside an event GPUI is handling for it, and what AppKit
//! tells GPUI about a change (a new size, the keyboard) is dropped while GPUI is updating. Each is
//! drawn before it shows, so it never shows what it showed last (an earlier freeze, where the
//! pointer was then), and the frozen screens and the live toolbar take each other's place at once.

use std::time::{Duration, Instant};

use asides_core::helper::SpecScreen;
use asides_core::inspect::PtRect;
use gpui::{
    App, AppContext, AsyncApp, Bounds, Entity, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions,
    point, px, size,
};

use crate::native;
use crate::ui::{Overlay, ScreenView, ScreenWindow, TOOLBAR_MARGIN, ToolbarView};

/// The live toolbar's window before it's drawn (it then fits the toolbar).
const TOOLBAR_GUESS: (f64, f64) = (420.0, 56.0);
/// The live toolbar's default place: this far above the bottom of the screen's visible part.
pub const TOOLBAR_LIFT: f64 = 18.0;
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
    /// The live toolbar shows, over the frozen screens, which show just the screen now.
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
    overlay.update(cx, |o, _| o.windows = windows);
    for (i, s) in spec.screens.iter().enumerate() {
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
        let options = WindowOptions {
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
        };
        let o = overlay.clone();
        let opened = cx.open_window(options, move |window, cx| {
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
}

/// Where the live toolbar's window goes on `screen`, `w` × `h`: where the user put the toolbar, below
/// the menu bar, or at the bottom in the middle, `lift` above the screen's bottom edge (the Dock's
/// height, when it shows) (AppKit's coordinates).
fn toolbar_frame(screen: &SpecScreen, at: Option<(f64, f64)>, lift: f64, (w, h): (f64, f64)) -> PtRect {
    let f = screen.frame;
    match at {
        Some((fx, fy)) => {
            let x = (f.x + fx * f.width - TOOLBAR_MARGIN).clamp(f.x, (f.x + f.width - w).max(f.x));
            let top = f.y + f.height - fy * f.height + TOOLBAR_MARGIN;
            let y = (top - h).clamp(f.y, (below_menu_bar(screen) - h).max(f.y));
            PtRect::new(x, y, w, h)
        }
        None => PtRect::new(f.x + (f.width - w) / 2.0, f.y + lift + TOOLBAR_LIFT - TOOLBAR_MARGIN, w, h),
    }
}

/// The highest the top of the live toolbar's window goes on `screen`: its toolbar (inside its
/// margin) stays below the menu bar and the notch (AppKit's coordinates).
fn below_menu_bar(screen: &SpecScreen) -> f64 {
    screen.frame.y + screen.frame.height - (screen.menu_bar - TOOLBAR_MARGIN).max(0.0)
}

/// The toolbar over the live screen, in a window of its own on the screen under the pointer
/// (hidden until `sync` shows it).
fn open_toolbar(overlay: &Entity<Overlay>, cx: &mut App) {
    let (spec, at) = {
        let o = overlay.read(cx);
        (o.spec().clone(), o.toolbar_at)
    };
    let Some(index) = pointer_screen(&spec.screens).or((!spec.screens.is_empty()).then_some(0)) else { return };
    let screen = &spec.screens[index];
    let lift = overlay.read(cx).lift(index);
    let frame = toolbar_frame(screen, at, lift, TOOLBAR_GUESS);
    let display_id = cx.displays().into_iter().find(|d| u32::from(d.id()) == screen.display).map(|d| d.id());
    let options = WindowOptions {
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
        window_background: WindowBackgroundAppearance::Transparent,
        ..Default::default()
    };
    let o = overlay.clone();
    let screen_frame = screen.frame;
    let opened = cx.open_window(options, move |window, cx| {
        let (number, view) = native::make_toolbar(window, frame);
        o.update(cx, |o, _| {
            o.toolbar_screen = index;
            o.toolbar_window = Some(ScreenWindow {
                handle: None,
                focus: None,
                screen: screen_frame,
                offset: (0.0, 0.0),
                size: (frame.width, frame.height),
                number,
                view,
            });
        });
        cx.new(|cx| ToolbarView::new(o.clone(), cx))
    });
    match opened {
        Ok(handle) => overlay.update(cx, |o, _| {
            if let Some(t) = &mut o.toolbar_window {
                t.handle = Some(handle.into());
            }
        }),
        Err(err) => eprintln!("asides-overlay: no toolbar window: {err:#}"),
    }
}

/// The windows the overlay needs and doesn't have yet: the screens' while frozen (again, when the
/// screens changed since), the toolbar's over the live screen.
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
    if overlay.read(cx).live && overlay.read(cx).toolbar_window.is_none() {
        open_toolbar(overlay, cx);
    }
}

/// Puts the windows in line with the overlay: the screens' while frozen, the toolbar's over the
/// live screen (following its handle and fitting the toolbar). From the clock, outside updates.
pub fn sync(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (live, shown, closing, handing, taking_over) = cx.update(|cx| {
        let o = overlay.read(cx);
        if !o.closing {
            open_missing(overlay, cx);
        }
        let o = overlay.read(cx);
        (o.live, o.shown, o.closing, o.handoff.is_some(), o.taking_over)
    })?;
    if closing {
        if shown == Some(false) && !handed_back(overlay, cx)? {
            return Ok(());
        }
        return cx.update(|cx| cx.quit());
    }
    match (shown == Some(live), live) {
        (true, true) => follow_toolbar(overlay, cx),
        // Frozen again while still giving way: the system comes back to the overlay.
        (true, false) if handing => show_frozen(overlay, cx),
        (true, false) => match taking_over {
            Some(since) => take_over(overlay, since, cx),
            None => Ok(()),
        },
        (false, true) => show_live(overlay, cx),
        (false, false) => show_frozen(overlay, cx),
    }
}

/// The frozen screens, drawn, in place of the live toolbar; the one under the pointer takes the
/// keyboard, and the overlay's app the system (`native::kiosk`).
fn show_frozen(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (screens, key, toolbar, windowed) = cx.update(|cx| {
        let o = overlay.read(cx);
        let views: Vec<usize> = o.windows.iter().map(|w| w.view).collect();
        let key = pointer_screen(&o.spec().screens).unwrap_or(0).min(views.len().saturating_sub(1));
        (views, key, o.toolbar_window.as_ref().map(|t| t.view), o.spec().windowed.is_some())
    })?;
    // GPUI still has the pointer where it was when they last showed (development windows show only
    // part of the screen: the pointer stays out of them).
    if !windowed && let Some(&view) = screens.get(key) {
        native::pointer_moved(view);
    }
    native::draw_now(&screens);
    let previous = native::front_app();
    for (i, &view) in screens.iter().enumerate() {
        native::show(view, i == key);
    }
    if let Some(toolbar) = toolbar {
        native::hide(toolbar);
    }
    cx.update(|cx| {
        let (focus, handles, stale) = overlay.update(cx, |o, _| {
            o.shown = Some(false);
            o.handoff = None;
            o.taking_over = Some(Instant::now());
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

/// The live toolbar, drawn at its size, in place of the frozen screens. From frozen, it shows at
/// once, above them (they show just the screen now, as it was), and they give way once the system
/// is back with the app the user was in (`handed_back`): with the Dock always shown, that's about
/// half a second, which the user doesn't wait on to see ▶ worked.
fn show_live(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let (screens, toolbar, frozen, started) = cx.update(|cx| {
        let o = overlay.read(cx);
        let views: Vec<usize> = o.windows.iter().map(|w| w.view).collect();
        (views, o.toolbar_window.as_ref().map(|t| t.view), o.shown == Some(false), o.handoff.is_some())
    })?;
    // Drawn while hidden, the toolbar knows its size: the window fits it, and it's drawn again.
    let prepare_toolbar = |cx: &mut AsyncApp| -> anyhow::Result<()> {
        let Some(toolbar) = toolbar else { return Ok(()) };
        native::pointer_moved(toolbar);
        native::draw_now(&[toolbar]);
        follow_toolbar(overlay, cx)?;
        native::draw_now(&[toolbar]);
        Ok(())
    };
    let show_toolbar = || {
        if let Some(toolbar) = toolbar {
            native::show(toolbar, false);
        }
    };
    if frozen {
        if !started {
            // At once, while the overlay's app is still the active one (drawn once it's in the
            // background, the frozen screens didn't show it): they show just the screen
            // (`ScreenView`), under the live toolbar; then the system goes back to the user's app.
            cx.update(|cx| {
                overlay.update(cx, |o, cx| {
                    o.handoff = Some(Handoff { since: Instant::now(), back: None, swapped: true });
                    cx.notify();
                })
            })?;
            // In one redraw of the frozen screens, their toolbar goes and the live one shows, where
            // its window is: shown under them meanwhile, it's there when they go. (Ordering a window
            // in and drawing another don't reach the screen in the same frame.)
            prepare_toolbar(cx)?;
            let frame = toolbar.and_then(native::frame);
            cx.update(|cx| {
                overlay.update(cx, |o, cx| {
                    o.handoff = Some(Handoff { since: Instant::now(), back: None, swapped: true });
                    o.live_toolbar_frame = frame;
                    cx.notify();
                })
            })?;
            native::draw_now(&screens);
            show_toolbar();
            return begin_handoff(overlay, cx);
        }
        if !handed_back(overlay, cx)? {
            return Ok(());
        }
    } else {
        prepare_toolbar(cx)?;
        show_toolbar();
    }
    for view in screens {
        native::hide(view);
    }
    cx.update(|cx| {
        overlay.update(cx, |o, _| {
            o.shown = Some(true);
            o.handoff = None;
            o.live_toolbar_frame = None;
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

/// The live toolbar's window: as big as the toolbar, and under the pointer while it's moved.
fn follow_toolbar(overlay: &Entity<Overlay>, cx: &mut AsyncApp) -> anyhow::Result<()> {
    let state = cx.update(|cx| {
        let o = overlay.read(cx);
        let margin = TOOLBAR_MARGIN * 2.0;
        let fits = o
            .live_toolbar_bounds
            .map(|b| (f32::from(b.size.width) as f64 + margin, f32::from(b.size.height) as f64 + margin));
        let screens = o.spec().screens.clone();
        o.toolbar_window.as_ref().map(|t| (t.view, fits, o.moving_toolbar, o.toolbar_at.is_some(), screens))
    })?;
    let Some((view, fits, moving, placed, screens)) = state else { return Ok(()) };
    let Some(frame) = native::frame(view) else { return Ok(()) };
    let mut f = frame;
    if let Some((w, h)) = fits
        && ((w - f.width).abs() > 0.5 || (h - f.height).abs() > 0.5)
    {
        // Its top-left corner stays, or, where it was never moved, its bottom middle.
        if placed {
            f.y += f.height - h;
        } else {
            f.x += (f.width - w) / 2.0;
        }
        (f.width, f.height) = (w, h);
    }
    let (mut moved, mut let_go) = (None, false);
    if let Some((gx, gy)) = moving {
        if native::mouse_pressed() {
            let (mx, my) = native::mouse();
            f.x = mx - gx - TOOLBAR_MARGIN;
            f.y = my + gy + TOOLBAR_MARGIN - f.height;
            // Never over the menu bar of the screen under the pointer.
            if let Some(s) = screens.iter().find(|s| {
                let r = s.frame;
                mx >= r.x && mx < r.x + r.width && my >= r.y && my <= r.y + r.height
            }) {
                f.y = f.y.min(below_menu_bar(s) - f.height);
            }
            moved = Some(f);
        } else {
            let_go = true;
        }
    }
    if f != frame {
        native::set_frame(view, f);
    }
    if moved.is_none() && !let_go {
        return Ok(());
    }
    cx.update(|cx| {
        overlay.update(cx, |o, _| {
            if let Some(f) = moved {
                // The toolbar's top-left corner, as a fraction of the screen it's on.
                let (left, top) = (f.x + TOOLBAR_MARGIN, f.y + f.height - TOOLBAR_MARGIN);
                let screens = &o.spec().screens;
                let index = screens
                    .iter()
                    .position(|s| {
                        let r = s.frame;
                        left >= r.x && left < r.x + r.width && top > r.y && top <= r.y + r.height
                    })
                    .unwrap_or(o.toolbar_screen);
                if let Some(s) = screens.get(index) {
                    let r = s.frame;
                    o.toolbar_at = Some((
                        ((left - r.x) / r.width).clamp(0.0, 1.0),
                        ((r.y + r.height - top) / r.height).clamp(0.0, 1.0),
                    ));
                    o.toolbar_screen = index;
                }
            }
            if let_go {
                o.moving_toolbar = None;
                o.placed_toolbar();
            }
        })
    })
}
