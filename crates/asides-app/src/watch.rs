//! Background polling: Wispr Flow's dictation history, and the agent sessions running. Like the
//! config watcher, these threads only report back; the main thread acts.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Duration;

use asides_core::sessions::{self, AgentSession};
use asides_core::wispr::{Dictation, History};
use chrono::Utc;

const WISPR_POLL: Duration = Duration::from_millis(700);
/// How often the agent sessions are looked for, besides when the app asks (asides opening).
const SESSIONS_POLL: Duration = Duration::from_secs(10);

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

/// Reports the agent sessions running (`sessions::discover`), every `SESSIONS_POLL`, and at once
/// each time something comes on `asked` (asides opening, a window wanting them).
pub fn sessions(asked: Receiver<()>, report: impl Fn(Vec<AgentSession>) -> bool + Send + 'static) {
    std::thread::Builder::new()
        .name("asides-sessions".into())
        .spawn(move || {
            loop {
                if !report(sessions::discover()) {
                    return;
                }
                match asked.recv_timeout(SESSIONS_POLL) {
                    Ok(()) => while asked.try_recv().is_ok() {},
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => return,
                }
            }
        })
        .expect("spawn the sessions thread");
}
