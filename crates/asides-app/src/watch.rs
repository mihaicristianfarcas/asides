//! Watching on threads of their own: Wispr Flow's dictation history (read as its database
//! changes), and the agent sessions running (looked for when asked, and again as one starts or
//! ends). Like the config watcher, these threads only report back; the main thread acts.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender, channel};

use asides_core::sessions::{self, AgentSession, Starts};
use asides_core::watch::{self as files, Watch};
use asides_core::wispr::{Dictation, History};
use chrono::Utc;

/// Watching Wispr Flow's history, until it's dropped.
pub struct Wispr {
    _watch: Option<Watch>,
}

/// Reports Wispr Flow dictations finished from now on, each time its database changes, until the
/// returned watch is dropped. Each distinct error is reported once.
pub fn wispr_flow(db: PathBuf, report: impl Fn(Result<Dictation, String>) -> bool + Send + 'static) -> Wispr {
    let (changed, changes) = channel();
    // The database and its write-ahead log (`flow.sqlite-wal`), where Wispr Flow writes first.
    let name = db.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let watch = db.parent().and_then(|dir| {
        files::folder(dir, false, move |path| {
            if path.file_name().is_some_and(|f| f.to_string_lossy().starts_with(&name)) {
                let _ = changed.send(());
            }
        })
        .ok()
    });
    std::thread::Builder::new()
        .name("asides-wispr-flow".into())
        .spawn(move || {
            let mut history = History::new(db, Utc::now());
            let mut last_error: Option<String> = None;
            loop {
                match history.read_new() {
                    Ok(found) => {
                        last_error = None;
                        for dictation in found {
                            if !report(Ok(dictation)) {
                                return;
                            }
                        }
                    }
                    Err(err) if last_error.as_ref() != Some(&err) => {
                        last_error = Some(err.clone());
                        if !report(Err(err)) {
                            return;
                        }
                    }
                    Err(_) => {}
                }
                // Until it changes; not watched (no folder), or no longer: done.
                if changes.recv().is_err() {
                    return;
                }
                while changes.try_recv().is_ok() {}
            }
        })
        .expect("spawn Wispr Flow thread");
    Wispr { _watch: watch }
}

/// Reports the agent sessions running (`sessions::discover`) now, and each time something comes
/// on `asked` (asides opening, the window wanting them, an aside sent, the user leaving a terminal
/// app), one of them starts (the file its agent writes, `sessions::start_folders`) or one ends:
/// asides knows them before it opens.
pub fn sessions(
    home: &Path,
    asked: Receiver<()>,
    again: Sender<()>,
    report: impl Fn(Vec<AgentSession>) -> bool + Send + 'static,
) {
    let starts: Vec<Watch> = sessions::start_folders(home)
        .into_iter()
        .filter_map(|(dir, deep)| {
            let (again, starts) = (again.clone(), Mutex::new(Starts::default()));
            files::folder(&dir, deep, move |path| {
                if starts.lock().unwrap().started(path) {
                    let _ = again.send(());
                }
            })
            .ok()
        })
        .collect();
    let exits = Exits::new(again);
    std::thread::Builder::new()
        .name("asides-sessions".into())
        .spawn(move || {
            let _starts = starts;
            loop {
                let found = sessions::discover();
                if let Some(exits) = &exits {
                    exits.watch(found.iter().map(|s| s.pid));
                }
                if !report(found) || asked.recv().is_err() {
                    return;
                }
                while asked.try_recv().is_ok() {}
            }
        })
        .expect("spawn the sessions thread");
}

/// Processes ending: kqueue says when one does (`NOTE_EXIT`), and `ended` hears it.
struct Exits {
    queue: libc::c_int,
}

impl Exits {
    fn new(ended: Sender<()>) -> Option<Exits> {
        let queue = unsafe { libc::kqueue() };
        if queue < 0 {
            return None;
        }
        std::thread::Builder::new()
            .name("asides-session-exits".into())
            .spawn(move || {
                loop {
                    let mut event: libc::kevent = unsafe { std::mem::zeroed() };
                    let got = unsafe { libc::kevent(queue, std::ptr::null(), 0, &mut event, 1, std::ptr::null()) };
                    if got > 0 && ended.send(()).is_err() {
                        return;
                    }
                    if got < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
                        return;
                    }
                }
            })
            .ok()?;
        Some(Exits { queue })
    }

    /// Hears when any of these processes ends (once; asking again for one already heard is fine).
    fn watch(&self, pids: impl Iterator<Item = i32>) {
        for pid in pids {
            let event = libc::kevent {
                ident: pid as libc::uintptr_t,
                filter: libc::EVFILT_PROC,
                flags: libc::EV_ADD | libc::EV_ONESHOT,
                fflags: libc::NOTE_EXIT,
                data: 0,
                udata: std::ptr::null_mut(),
            };
            unsafe { libc::kevent(self.queue, &event, 1, std::ptr::null_mut(), 0, std::ptr::null()) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn looks_for_sessions_again_as_one_starts() {
        let home = tempfile::tempdir().unwrap();
        let home = home.path().canonicalize().unwrap();
        let folder = home.join(".claude/sessions");
        std::fs::create_dir_all(&folder).unwrap();
        let (asked, requests) = channel();
        let (looked, looks) = channel();
        sessions(&home, requests, asked, move |_| looked.send(()).is_ok());
        let wait = Duration::from_secs(5);
        // Once as it starts.
        looks.recv_timeout(wait).unwrap();
        // A session starting: its agent writes its file.
        std::fs::write(folder.join("70001.json"), r#"{"status":"idle"}"#).unwrap();
        looks.recv_timeout(wait).expect("a session starting is looked for");
        // The same session working: its file written again isn't a session starting.
        std::fs::write(folder.join("70001.json"), r#"{"status":"busy"}"#).unwrap();
        assert!(looks.recv_timeout(Duration::from_secs(1)).is_err());
    }
}
