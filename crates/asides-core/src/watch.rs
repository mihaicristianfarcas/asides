//! Waiting on files without looking at them again and again: the system says when something in a
//! folder changes (FSEvents on macOS), and only then is it read.

use std::path::Path;
use std::sync::mpsc::{RecvTimeoutError, channel};
use std::time::Instant;

use notify::{RecursiveMode, Watcher as _};

/// A folder watched, until it's dropped.
pub struct Watch(#[allow(dead_code)] notify::RecommendedWatcher);

/// Calls `changed` with each path that changes in `dir` (and in the folders in it, with `deep`):
/// created, written, renamed or removed.
pub fn folder(dir: &Path, deep: bool, changed: impl Fn(&Path) + Send + 'static) -> notify::Result<Watch> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if let Ok(event) = event {
            for path in &event.paths {
                changed(path);
            }
        }
    })?;
    let mode = if deep { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    watcher.watch(dir, mode)?;
    Ok(Watch(watcher))
}

/// Calls `changed` each time the file at `path` changes (it may not exist yet): its folder is
/// watched, as files are often replaced rather than written in place.
pub fn file(path: &Path, changed: impl Fn() + Send + 'static) -> notify::Result<Watch> {
    let (dir, name) = match (path.parent(), path.file_name()) {
        (Some(dir), Some(name)) => (dir, name.to_owned()),
        _ => return Err(notify::Error::path_not_found()),
    };
    folder(dir, false, move |changed_path| {
        if changed_path.file_name() == Some(name.as_os_str()) {
            changed();
        }
    })
}

/// What `ready` gives, as soon as it gives something: it's asked now, then each time something in
/// `dir` (or the folders in it) changes, until `deadline` (then once more).
pub fn until<T>(dir: &Path, deadline: Instant, mut ready: impl FnMut() -> Option<T>) -> Option<T> {
    let (tx, rx) = channel();
    let watch = folder(dir, true, move |_| {
        let _ = tx.send(());
    });
    loop {
        if let Some(found) = ready() {
            return Some(found);
        }
        let left = deadline.checked_duration_since(Instant::now())?;
        match rx.recv_timeout(left) {
            Ok(()) => while rx.try_recv().is_ok() {},
            Err(RecvTimeoutError::Timeout) => return ready(),
            // Not watched (the folder isn't there): nothing will say, so it waits it out.
            Err(RecvTimeoutError::Disconnected) => {
                drop(watch);
                std::thread::sleep(left);
                return ready();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn says_when_a_file_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.json");
        let (tx, rx) = channel();
        let _watch = file(&path, move || {
            let _ = tx.send(());
        })
        .unwrap();
        std::fs::write(dir.path().join("other.json"), "{}").unwrap();
        let tmp = dir.path().join("state.json.tmp");
        std::fs::write(&tmp, "{}").unwrap();
        std::fs::rename(&tmp, &path).unwrap();
        assert!(rx.recv_timeout(Duration::from_secs(5)).is_ok(), "the replaced file was reported");
    }

    #[test]
    fn waits_until_ready() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("answer");
        let writer = {
            let path = path.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(200));
                std::fs::write(path, "42").unwrap();
            })
        };
        let started = Instant::now();
        let found = until(dir.path(), started + Duration::from_secs(10), || std::fs::read_to_string(&path).ok());
        writer.join().unwrap();
        assert_eq!(found.as_deref(), Some("42"));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
