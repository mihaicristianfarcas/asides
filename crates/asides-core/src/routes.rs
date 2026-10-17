//! Which session each entry went to: `routes.jsonl` beside the note, one line per entry, only
//! ever appended to. The app's worker writes an entry's line just before the entry itself, so an
//! agent that sees the entry also sees whether it's for it. Several sessions can listen in one
//! repo; each gets only the asides the user spoke to it.
//!
//! An entry without a line was written while no session was around (or by an older version):
//! every agent listening there may have it. A line without a session means the user chose Notes
//! Only: no agent gets it.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

pub const ROUTES_FILE: &str = "routes.jsonl";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Route {
    pub entry: u32,
    /// The session it went to; `None` for Notes Only.
    pub session: Option<String>,
    /// That session's agent, for people reading the file.
    pub agent: Option<String>,
}

impl Route {
    /// Whether an agent listening as `session` gets the entry.
    pub fn reaches(route: Option<&Route>, session: Option<&str>) -> bool {
        route.is_none_or(|r| r.session.as_deref() == session)
    }
}

/// Appends `route` in one write, like `threads::append`.
pub fn append(note_dir: &Path, route: &Route) -> io::Result<()> {
    let mut line = serde_json::to_string(route).map_err(io::Error::other)?;
    line.push('\n');
    OpenOptions::new().create(true).append(true).open(note_dir.join(ROUTES_FILE))?.write_all(line.as_bytes())
}

/// Each entry's route; when an entry has several lines (a write that failed and was retried),
/// the last one.
pub fn read(note_dir: &Path) -> BTreeMap<u32, Route> {
    let Ok(text) = fs::read_to_string(note_dir.join(ROUTES_FILE)) else { return BTreeMap::new() };
    text.lines().filter_map(|line| serde_json::from_str::<Route>(line).ok()).map(|r| (r.entry, r)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_reads_the_last_route_of_each_entry() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read(tmp.path()).is_empty());
        let to = |entry, session: Option<&str>| Route { entry, session: session.map(Into::into), agent: None };
        for route in [to(1, Some("a")), to(2, None), to(2, Some("b"))] {
            append(tmp.path(), &route).unwrap();
        }
        let routes = read(tmp.path());
        assert_eq!(routes.values().cloned().collect::<Vec<_>>(), [to(1, Some("a")), to(2, Some("b"))]);

        assert!(Route::reaches(routes.get(&1), Some("a")));
        assert!(!Route::reaches(routes.get(&1), Some("b")));
        assert!(Route::reaches(routes.get(&9), Some("b")), "an entry without a route reaches everyone");
        assert!(!Route::reaches(Some(&to(3, None)), Some("a")), "Notes Only reaches nobody");
    }
}
