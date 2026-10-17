//! Background polling: Wispr Flow's dictation history, the agent sessions and the processes
//! listening for them, and what agents say about the user's asides (`threads.jsonl`). Like the
//! config watcher, these threads only report back; the main thread acts.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use asides_core::agents::Listener;
use asides_core::notes;
use asides_core::sessions::Session;
use asides_core::threads::{self, Event, THREADS_FILE};
use asides_core::wispr::{Dictation, History};
use chrono::Utc;

const WISPR_POLL: Duration = Duration::from_millis(700);
const LISTENERS_POLL: Duration = Duration::from_secs(2);
const THREADS_POLL: Duration = Duration::from_secs(1);

/// Reports Wispr Flow dictations finished from now on, until the returned flag is set. Each
/// distinct error is reported once.
pub fn wispr_flow(db: PathBuf, report: impl Fn(Result<Dictation, String>) -> bool + Send + 'static) -> Arc<AtomicBool> {
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::Builder::new()
        .name("asides-wispr-flow".into())
        .spawn(move || {
            let mut history = History::new(db, Utc::now());
            let mut last_error: Option<String> = None;
            while !stopped.load(Ordering::Relaxed) {
                match history.poll() {
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
                std::thread::sleep(WISPR_POLL);
            }
        })
        .expect("spawn Wispr Flow thread");
    stop
}

/// Reports the agent sessions and the processes listening for them (`asides mcp`, `asides
/// listen`) every couple of seconds, so the app also notices a session that came and went between
/// two looks (an agent that failed to start). Records left behind by processes and sessions that
/// ended without cleaning up are removed.
pub fn sessions(
    listeners_dir: PathBuf,
    sessions_dir: PathBuf,
    report: impl Fn(Vec<Listener>, Vec<Session>) -> bool + Send + 'static,
) {
    std::thread::Builder::new()
        .name("asides-sessions".into())
        .spawn(move || {
            loop {
                let mut listeners = Vec::new();
                for listener in Listener::read_all(&listeners_dir) {
                    if is_running(listener.pid) {
                        listeners.push(listener);
                    } else {
                        let _ = std::fs::remove_file(Listener::file(&listeners_dir, listener.pid));
                    }
                }
                let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                let mut sessions = Vec::new();
                for session in Session::read_all(&sessions_dir) {
                    let ended = match session.pid {
                        0 => session.expired(&listeners, now),
                        pid => !is_running(pid),
                    };
                    if ended {
                        Session::remove(&sessions_dir, &session.id);
                    } else {
                        sessions.push(session);
                    }
                }
                if !report(listeners, sessions) {
                    return;
                }
                std::thread::sleep(LISTENERS_POLL);
            }
        })
        .expect("spawn sessions thread");
}

/// Reports the events of the latest note in the directory `bound` holds whenever they change (an
/// agent resolved or replied to an aside), and once for each note it starts watching.
pub fn threads(bound: Arc<Mutex<Option<PathBuf>>>, report: impl Fn(PathBuf, Vec<Event>) -> bool + Send + 'static) {
    std::thread::Builder::new()
        .name("asides-threads".into())
        .spawn(move || {
            let mut last: Option<(PathBuf, Option<(u64, std::time::SystemTime)>)> = None;
            loop {
                std::thread::sleep(THREADS_POLL);
                let dir = bound.lock().unwrap().clone();
                let Some(note) = dir.as_deref().and_then(notes::latest_note) else { continue };
                let stamp = std::fs::metadata(note.dir.join(THREADS_FILE))
                    .ok()
                    .and_then(|m| Some((m.len(), m.modified().ok()?)));
                let now = (note.dir.clone(), stamp);
                if last.as_ref() != Some(&now) {
                    last = Some(now);
                    if !report(note.dir.clone(), threads::read(&note.dir)) {
                        return;
                    }
                }
            }
        })
        .expect("spawn threads watcher");
}

fn is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else { return false };
    // Signal 0 checks that the process exists; EPERM means it does but belongs to someone else.
    pid > 0
        && (unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}
