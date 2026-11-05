//! xdg-desktop-portal, the way Wayland desktops let an app do what only the compositor can: take a
//! screenshot, bind global shortcuts, open a dialog above no window of the app's. A call answers
//! with a request object, whose `Response` signal brings the result, once the user has answered if
//! they're asked.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use crate::paths::BUNDLE_ID;
use serde::Serialize;
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{DynamicType, OwnedValue, Value};

pub const DESKTOP: &str = "org.freedesktop.portal.Desktop";
pub const OBJECT: &str = "/org/freedesktop/portal/desktop";

/// Whether the portal knows asides by its ID (newer ones do); older ones don't take it.
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// The session bus connection portals know this app by. A desktop asks once per app, so it's the
/// same for every call; the app says who it is first (newer portals take that from its `.desktop`
/// file; older ones have no such call).
pub fn connection() -> Result<&'static Connection, String> {
    static CONNECTION: OnceLock<Result<Connection, String>> = OnceLock::new();
    let connection = CONNECTION.get_or_init(|| {
        let connection = Connection::session().map_err(|err| format!("no session bus: {err}"))?;
        let no_options: HashMap<&str, Value> = HashMap::new();
        let registered = connection.call_method(
            Some(DESKTOP),
            OBJECT,
            Some("org.freedesktop.host.portal.Registry"),
            "Register",
            &(BUNDLE_ID, no_options),
        );
        REGISTERED.store(registered.is_ok(), Ordering::Relaxed);
        Ok(connection)
    });
    connection.as_ref().map_err(Clone::clone)
}

/// What the desktop keeps of the user's answer about asides' screenshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Yes,
    No,
    NotAsked,
}

const STORE: &str = "org.freedesktop.impl.portal.PermissionStore";
const STORE_OBJECT: &str = "/org/freedesktop/impl/portal/PermissionStore";
/// The store's table and entry for screenshots.
const SCREENSHOTS: (&str, &str) = ("screenshot", "screenshot");

/// Whether the desktop asks before asides' first screenshot, and shows its question only for the app
/// in front ("Only the focused app is allowed to show a system access dialog"): GNOME on Wayland. A
/// screenshot asked for from the background fails there, so asides' window asks (setup's button).
pub fn asks_for_screenshots() -> bool {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty())
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|s| s == "wayland");
    wayland && std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|desktop| is_gnome(&desktop))
}

/// `XDG_CURRENT_DESKTOP` lists desktops: `GNOME`, `ubuntu:GNOME`, `GNOME-Classic:GNOME`.
fn is_gnome(desktop: &str) -> bool {
    desktop.split(':').any(|name| name.eq_ignore_ascii_case("GNOME"))
}

/// The answer the desktop keeps for asides.
pub fn screenshot_answer() -> Result<Answer, String> {
    let connection = connection()?;
    match connection.call_method(Some(STORE), STORE_OBJECT, Some(STORE), "Lookup", &SCREENSHOTS) {
        Ok(reply) => {
            let (kept, _): (HashMap<String, Vec<String>>, OwnedValue) =
                reply.body().deserialize().map_err(|err| err.to_string())?;
            Ok(answer_in(&kept, REGISTERED.load(Ordering::Relaxed)))
        }
        // Nobody has been asked yet.
        Err(zbus::Error::MethodError(name, ..)) if name.as_str().ends_with(".NotFound") => Ok(Answer::NotAsked),
        Err(err) => Err(err.to_string()),
    }
}

/// The answer among all apps': under asides' ID, or, for a portal that doesn't take it, under the ID
/// it found itself (from how asides was started) or none.
fn answer_in(kept: &HashMap<String, Vec<String>>, registered: bool) -> Answer {
    let mine = |app: &str| app == BUNDLE_ID || (!registered && app.is_empty());
    let answers: Vec<&str> =
        kept.iter().filter(|(app, _)| mine(app)).flat_map(|(_, answer)| answer.iter().map(String::as_str)).collect();
    match () {
        _ if answers.contains(&"yes") => Answer::Yes,
        _ if answers.contains(&"no") => Answer::No,
        _ => Answer::NotAsked,
    }
}

/// Forgets a "no", so the desktop asks again: the user just pressed a button asking it to.
pub fn forget_screenshot_answer() {
    let Ok(connection) = connection() else { return };
    let app = if REGISTERED.load(Ordering::Relaxed) { BUNDLE_ID } else { "" };
    let _ = connection.call_method(
        Some(STORE),
        STORE_OBJECT,
        Some(STORE),
        "DeletePermission",
        &(SCREENSHOTS.0, SCREENSHOTS.1, app),
    );
}

/// A name for a request or a session, new each time.
pub fn token() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!("asides{}_{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed))
}

/// The object a request with this token answers on: the portal makes it from the caller's name.
fn request_path(connection: &Connection, token: &str) -> Result<String, String> {
    let name = connection.unique_name().ok_or("not on the session bus")?;
    let sender = name.trim_start_matches(':').replace('.', "_");
    Ok(format!("/org/freedesktop/portal/desktop/request/{sender}/{token}"))
}

/// What a portal request answered.
pub type Results = HashMap<String, OwnedValue>;

/// Calls `method` of a portal `interface` with `args`, made with the request's token, and waits up
/// to `wait` for its response. Listening starts before the call: the answer can't be missed.
pub fn request<B: Serialize + DynamicType>(
    interface: &str,
    method: &str,
    args: impl FnOnce(&str) -> B,
    wait: Duration,
) -> Result<Results, String> {
    let connection = connection()?;
    let token = token();
    let path = request_path(connection, &token)?;
    let answer = Proxy::new(connection, DESKTOP, path, "org.freedesktop.portal.Request")
        .map_err(|err| format!("the portal's request: {err}"))?;
    let mut responses = answer.receive_signal("Response").map_err(|err| format!("the portal's request: {err}"))?;
    let (answered, answer_came) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("asides-portal-request".into())
        .spawn(move || {
            if let Some(message) = responses.next() {
                let _ = answered.send(message.body().deserialize::<(u32, Results)>().map_err(|err| err.to_string()));
            }
        })
        .map_err(|err| err.to_string())?;
    let called = connection.call_method(Some(DESKTOP), OBJECT, Some(interface), method, &args(&token));
    called.map_err(|err| format!("the desktop's portal said: {err}"))?;
    let (code, results) = answer_came
        .recv_timeout(wait)
        .map_err(|_| "the desktop's portal didn't answer".to_string())?
        .map_err(|err| format!("the desktop's portal said: {err}"))?;
    match code {
        0 => Ok(results),
        1 => Err("declined".into()),
        _ => Err("the desktop's portal failed".into()),
    }
}

/// A string among a request's results.
pub fn text(results: &Results, key: &str) -> Option<String> {
    let value = results.get(key)?;
    let text: &str = value.downcast_ref().ok()?;
    Some(text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_gnome_under_its_other_names() {
        for desktop in ["GNOME", "ubuntu:GNOME", "GNOME-Classic:GNOME", "gnome"] {
            assert!(is_gnome(desktop), "{desktop}");
        }
        for desktop in ["KDE", "XFCE", "X-Cinnamon", "GNOME-Flashback", ""] {
            assert!(!is_gnome(desktop), "{desktop}");
        }
    }

    #[test]
    fn finds_asides_answer_among_all_apps() {
        let kept = |entries: &[(&str, &str)]| -> HashMap<String, Vec<String>> {
            entries.iter().map(|(app, answer)| (app.to_string(), vec![answer.to_string()])).collect()
        };
        assert_eq!(answer_in(&kept(&[]), true), Answer::NotAsked);
        assert_eq!(answer_in(&kept(&[(BUNDLE_ID, "yes")]), true), Answer::Yes);
        assert_eq!(answer_in(&kept(&[(BUNDLE_ID, "no")]), true), Answer::No);
        // Another app's answer, or one for apps without an ID, isn't asides'.
        assert_eq!(answer_in(&kept(&[("org.other.App", "yes"), ("", "yes")]), true), Answer::NotAsked);
        // A portal that doesn't take IDs keeps asides' under none, or one it found itself.
        assert_eq!(answer_in(&kept(&[("", "yes")]), false), Answer::Yes);
        assert_eq!(answer_in(&kept(&[(BUNDLE_ID, "yes")]), false), Answer::Yes);
        assert_eq!(answer_in(&kept(&[("org.other.App", "yes")]), false), Answer::NotAsked);
    }
}
