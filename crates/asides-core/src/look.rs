//! Looking again at what a pin was on (`asides look`, the MCP server's `look_again`): what the app
//! may show an agent, decided here so it can be tested. An agent checking its own fix shouldn't
//! need the user to freeze the screen again, but it mustn't get to see the screen at will either:
//!
//! - only what a pin of an aside was on (the pin's own part of the screen, with some of what's
//!   around it);
//! - only for `minutes` after that aside was last sent (or made), and never with the setting at 0;
//! - only in the window the pin was in, wherever that window is now (it's found again by its id,
//!   else by its app and title): a Mac captures that window alone, whatever is over it; Windows and
//!   Linux capture the screen there, so only while nothing else is over that part of it
//!   (`Plan::uncovered`).
//!
//! The app captures, flashes the menu bar and keeps the picture with the aside (`looks/`).

use std::path::PathBuf;

use crate::history::Aside;
use crate::inspect::{PtRect, Spot};

/// How long after an aside an agent may look again, unless the settings say otherwise.
pub const DEFAULT_MINUTES: u32 = 15;
/// Points kept around what a pin marked, at least: an element alone says little.
const MIN_PADDING: f64 = 24.0;
/// The longer side of the picture an agent gets, in pixels: as an aside's screenshots.
pub const MAX_SIDE: u32 = 1568;
/// The folder in an aside's own where what agents looked at is kept.
pub const FOLDER: &str = "looks";

/// A window on screen, as each system lists it (frontmost first).
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    /// The system's own id for it (`Element::window_id`).
    pub id: u64,
    pub app: String,
    pub title: Option<String>,
    /// In global points (y down).
    pub frame: PtRect,
}

/// What to capture, and what it shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The window to look at, as the system knows it, and where it is (global points).
    pub window: u64,
    pub frame: PtRect,
    /// The part of the screen to capture now, in global points: inside that window.
    pub rect: PtRect,
    /// The app whose window is over that part of it, if one is.
    pub covered_by: Option<String>,
    /// The pin, for what the agent is told: its number, and what it was on.
    pub pin: u32,
    pub title: String,
    /// The app the window is in.
    pub app: String,
    /// Where the picture is kept, in the aside's folder (`looks/2-1763300000000.png`).
    pub file: PathBuf,
}

/// What looking again at pin `pin` of `aside` captures, with `windows` on screen (frontmost
/// first), or why it can't: said to the agent as it is.
pub fn plan(
    aside: &Aside,
    pin: u32,
    minutes: u32,
    now_ms: u64,
    windows: &[Window],
    padding: f64,
) -> Result<Plan, String> {
    if minutes == 0 {
        return Err("Looking again is turned off in asides' settings (General, or look_again_minutes).".into());
    }
    let age = now_ms.saturating_sub(aside.last_sent_ms()) / 60_000;
    if age >= minutes as u64 {
        return Err(format!(
            "Aside {} is {age} minutes old: asides lets agents look again for {minutes} minutes after an aside. \
             Ask the user to send a new one.",
            aside.id
        ));
    }
    let numbers: Vec<String> = aside.record.pins.iter().map(|p| p.number.to_string()).collect();
    let Some(found) = aside.record.pins.iter().find(|p| p.number == pin) else {
        return Err(match numbers.is_empty() {
            true => format!("Aside {} has no pins to look at again.", aside.id),
            false => format!("Aside {} has no pin {pin} (its pins: {}).", aside.id, numbers.join(", ")),
        });
    };
    let Some(spot) = &found.spot else {
        return Err(format!("Where pin {pin} was on the screen wasn't kept with aside {}.", aside.id));
    };
    let Some(app) = &spot.app else {
        return Err(format!("Pin {pin} wasn't on a window (asides only looks again at the window a pin was in)."));
    };
    let Some((index, window)) = find(spot, app, windows) else {
        let title = spot.window.as_deref().filter(|t| !t.is_empty()).map(|t| format!(" (\"{t}\")")).unwrap_or_default();
        return Err(format!("The {app} window{title} pin {pin} was in isn't on screen now."));
    };
    // Where it was in its window is where it is now, wherever the window went.
    let view = spot.view(padding.max(MIN_PADDING));
    let then = spot.window_frame.unwrap_or(window.frame);
    let moved =
        PtRect::new(view.x - then.x + window.frame.x, view.y - then.y + window.frame.y, view.width, view.height);
    let Some(rect) = intersect(moved, window.frame) else {
        return Err(format!("The {app} window pin {pin} was in is smaller now: where the pin was is outside it."));
    };
    let covered_by = windows[..index].iter().find(|w| intersect(rect, w.frame).is_some()).map(|w| w.app.clone());
    let file = aside.dir.join(FOLDER).join(format!("{pin}-{now_ms}.png"));
    Ok(Plan {
        window: window.id,
        frame: window.frame,
        rect,
        covered_by,
        pin,
        title: spot.title.clone(),
        app: app.clone(),
        file,
    })
}

impl Plan {
    /// For a system that captures the screen, not a window: only with nothing over that part of
    /// the window, or the agent would see what is.
    pub fn uncovered(self) -> Result<Plan, String> {
        match &self.covered_by {
            Some(over) => Err(format!(
                "Another window ({over}) is over what pin {} was on in {}: asides shows an agent only the window \
                 the pin was in. Ask the user to bring it to the front.",
                self.pin, self.app
            )),
            None => Ok(self),
        }
    }

    /// The part of a picture of the whole window (`scale` pixels a point) to keep: `rect`.
    pub fn in_window(&self, scale: f64) -> crate::measure::Rect {
        self.within(self.frame, scale)
    }

    /// The part of a picture of `of` (global points, `scale` pixels a point) to keep: `rect`.
    pub fn within(&self, of: PtRect, scale: f64) -> crate::measure::Rect {
        let px = |points: f64| (points * scale).round().max(0.0) as u32;
        crate::measure::Rect::new(
            px(self.rect.x - of.x),
            px(self.rect.y - of.y),
            px(self.rect.width).max(1),
            px(self.rect.height).max(1),
        )
    }
}

/// The window `spot` was in: the same one (its id, still that app's), else that app's window with
/// the same title, else that app's only window.
fn find<'a>(spot: &Spot, app: &str, windows: &'a [Window]) -> Option<(usize, &'a Window)> {
    let of_app = || windows.iter().enumerate().filter(|(_, w)| w.app == app);
    let by_id = spot.window_id.and_then(|id| of_app().find(|(_, w)| w.id == id));
    let title = spot.window.as_deref().filter(|t| !t.is_empty());
    let by_title = || title.and_then(|t| of_app().find(|(_, w)| w.title.as_deref() == Some(t)));
    let only = || (of_app().count() == 1).then(|| of_app().next()).flatten();
    by_id.or_else(by_title).or_else(only)
}

/// The part of `a` inside `b`, if any (at least a point wide and high).
fn intersect(a: PtRect, b: PtRect) -> Option<PtRect> {
    let (x0, y0) = (a.x.max(b.x), a.y.max(b.y));
    let (x1, y1) = (a.right().min(b.right()), a.bottom().min(b.bottom()));
    (x1 - x0 >= 1.0 && y1 - y0 >= 1.0).then(|| PtRect::new(x0, y0, x1 - x0, y1 - y0))
}

#[cfg(test)]
mod tests {
    use chrono::{Local, TimeZone};

    use super::*;
    use crate::history::{self, Delivery, Pin};

    fn window(id: u64, app: &str, title: &str, frame: PtRect) -> Window {
        Window { id, app: app.into(), title: Some(title.into()), frame }
    }

    /// An aside with pin 1 on a 120 × 36 button at (192, 460) of a Safari window at (100, 100),
    /// pin 2 on the desktop, and pin 3 from before places were kept.
    fn aside(tmp: &std::path::Path) -> Aside {
        let spot = Spot {
            display: 1,
            at: (200.0, 470.0),
            region: Some(PtRect::new(192.0, 460.0, 120.0, 36.0)),
            title: "button \"Save\"".into(),
            app: Some("Safari".into()),
            window: Some("Settings — Shop".into()),
            window_id: Some(41),
            window_frame: Some(PtRect::new(100.0, 100.0, 800.0, 600.0)),
            ..Spot::default()
        };
        let pins = vec![
            Pin { number: 1, color: "#3B82F6".into(), spot: Some(spot) },
            Pin { number: 2, color: "#3B82F6".into(), spot: Some(Spot { at: (5.0, 5.0), ..Spot::default() }) },
            Pin { number: 3, color: "#3B82F6".into(), spot: None },
        ];
        let at = Local.with_ymd_and_hms(2026, 11, 16, 14, 2, 0).unwrap();
        history::create(tmp, at, "Too low [1].\n\n### 1. button \"Save\"", &[], pins, None).unwrap()
    }

    #[test]
    fn looks_only_at_the_window_a_pin_was_in() {
        let tmp = tempfile::tempdir().unwrap();
        let mut aside = aside(tmp.path());
        let made = aside.record.created_ms;
        let safari = window(41, "Safari", "Settings — Shop", PtRect::new(100.0, 100.0, 800.0, 600.0));
        let terminal = window(7, "Terminal", "claude", PtRect::new(1000.0, 100.0, 600.0, 600.0));
        let now = made + 60_000;
        let look = |windows: &[Window]| plan(&aside, 1, 15, now, windows, 24.0);

        // Where it was: the button with 24 points around it.
        let plan_ = look(&[terminal.clone(), safari.clone()]).unwrap();
        assert_eq!((plan_.window, plan_.rect), (41, PtRect::new(168.0, 436.0, 168.0, 84.0)));
        assert_eq!((plan_.frame, &plan_.covered_by), (safari.frame, &None));
        // In a picture of the window at 2 pixels a point, and in one of the screen it's on.
        assert_eq!(plan_.in_window(2.0), crate::measure::Rect::new(136, 672, 336, 168));
        assert_eq!(
            plan_.within(PtRect::new(0.0, 0.0, 1512.0, 945.0), 2.0),
            crate::measure::Rect::new(336, 872, 336, 168)
        );
        assert_eq!((plan_.pin, plan_.title.as_str(), plan_.app.as_str()), (1, "button \"Save\"", "Safari"));
        assert_eq!(plan_.file, aside.dir.join("looks").join(format!("1-{now}.png")));

        // The window moved: so did what's looked at. Near its edge, only what's in the window.
        let moved = Window { frame: PtRect::new(300.0, 50.0, 800.0, 600.0), ..safari.clone() };
        assert_eq!(look(&[moved]).unwrap().rect, PtRect::new(368.0, 386.0, 168.0, 84.0));
        let small = Window { frame: PtRect::new(100.0, 100.0, 800.0, 400.0), ..safari.clone() };
        assert_eq!(look(&[small]).unwrap().rect, PtRect::new(168.0, 436.0, 168.0, 64.0));
        let smaller = Window { frame: PtRect::new(100.0, 100.0, 800.0, 300.0), ..safari.clone() };
        assert!(look(&[smaller]).unwrap_err().contains("smaller now"));

        // Reopened (another id), it's found by its title; with another title, when it's the app's only one.
        let reopened = Window { id: 99, ..safari.clone() };
        assert_eq!(look(std::slice::from_ref(&reopened)).unwrap().window, 99);
        let renamed = Window { id: 99, title: Some("Saved — Shop".into()), ..safari.clone() };
        assert_eq!(look(std::slice::from_ref(&renamed)).unwrap().window, 99);
        let other = window(98, "Safari", "Docs", PtRect::new(0.0, 0.0, 400.0, 300.0));
        assert!(look(&[renamed, other]).unwrap_err().contains("isn't on screen now"));
        assert!(look(std::slice::from_ref(&terminal)).unwrap_err().contains("The Safari window (\"Settings — Shop\")"));
        // Another app's window with the same id isn't it.
        assert!(look(&[window(41, "Mail", "Inbox", safari.frame)]).is_err());

        // Something over it: where the screen is what's captured, nothing is shown. Over another
        // part of the window, or behind it, it's fine.
        let over = window(8, "1Password", "Vault", PtRect::new(150.0, 400.0, 300.0, 200.0));
        let covered = look(&[over, safari.clone()]).unwrap();
        assert_eq!(covered.covered_by.as_deref(), Some("1Password"));
        let said = covered.uncovered().unwrap_err();
        assert!(said.contains("Another window (1Password) is over"), "{said}");
        let beside = window(8, "Notes", "Note", PtRect::new(500.0, 100.0, 300.0, 200.0));
        assert!(look(&[beside, safari.clone()]).unwrap().uncovered().is_ok());
        let behind = window(8, "Notes", "Note", PtRect::new(150.0, 400.0, 300.0, 200.0));
        assert!(look(&[safari.clone(), behind]).unwrap().uncovered().is_ok());

        // Pins it can't look at, and why.
        let windows = [safari.clone()];
        assert!(plan(&aside, 2, 15, now, &windows, 24.0).unwrap_err().contains("wasn't on a window"));
        assert!(plan(&aside, 3, 15, now, &windows, 24.0).unwrap_err().contains("wasn't kept"));
        assert!(plan(&aside, 9, 15, now, &windows, 24.0).unwrap_err().contains("no pin 9 (its pins: 1, 2, 3)"));

        // For a while after the aside only: counted from when it was last sent. And never when off.
        let late = made + 16 * 60_000;
        assert!(plan(&aside, 1, 15, late, &windows, 24.0).unwrap_err().contains("is 16 minutes old"));
        aside.record_delivery(Delivery { at_ms: made + 10 * 60_000, ..Default::default() }).unwrap();
        assert!(plan(&aside, 1, 15, late, &windows, 24.0).is_ok());
        assert!(plan(&aside, 1, 0, now, &windows, 24.0).unwrap_err().contains("turned off"));
    }
}
