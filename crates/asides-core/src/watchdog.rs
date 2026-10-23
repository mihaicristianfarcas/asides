//! Watches a thread that must keep going: a GUI process's main thread, which draws and takes the
//! keyboard. Its run loop says when it works and when it waits for events (`working`, `waiting`).
//! A spell of work longer than `Limits::slow` is told when it ends; one that doesn't end is told
//! at `Limits::stuck`, while it lasts, and at `Limits::give_up`. The watchdog's thread sleeps until
//! the deadline of the spell going on, and with none, until one starts: a stuck thread can't say
//! it's stuck, so a deadline is the only way to know.
//!
//! `Spin` notices the opposite: something that runs again and again without waiting.

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

pub struct Limits {
    /// A spell this long is told when it ends (`Note::Slow`).
    pub slow: Duration,
    /// A spell this long is told while it lasts (`Note::Stuck`).
    pub stuck: Duration,
    /// And this long, once (`Note::GiveUp`): nothing is told after.
    pub give_up: Option<Duration>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Note {
    /// A spell of work ended, this long.
    Slow(Duration),
    /// A spell has gone on this long, and isn't over.
    Stuck(Duration),
    /// A spell told as stuck ended, this long.
    WentOn(Duration),
    /// A spell has gone on for `Limits::give_up`.
    GiveUp(Duration),
}

impl Note {
    /// The note as a line of the log (a problem from a second on).
    pub fn log(&self) {
        let ms = |d: &Duration| d.as_millis();
        match self {
            Note::Slow(d) if *d < Duration::from_secs(1) => {
                crate::log::write(format_args!("the main thread was busy for {} ms", ms(d)))
            }
            Note::Slow(d) => crate::log::problem(format_args!("the main thread was busy for {} ms", ms(d))),
            Note::Stuck(d) => crate::log::problem(format_args!("the main thread has been stuck for {} ms", ms(d))),
            Note::WentOn(d) => crate::log::problem(format_args!("the main thread went on after {} ms", ms(d))),
            Note::GiveUp(d) => {
                crate::log::problem(format_args!("the main thread has been stuck for {} ms: giving up", ms(d)))
            }
        }
    }
}

/// The thread watched says what it does through this.
#[derive(Clone)]
pub struct Watchdog(Arc<Shared>);

struct Shared {
    spell: Mutex<Spell>,
    /// A spell started, or one told as stuck ended.
    changed: Condvar,
    limits: Limits,
    tell: Box<dyn Fn(Note) + Send + Sync>,
}

#[derive(Default)]
struct Spell {
    since: Option<Instant>,
    /// Told as stuck (`Note::Stuck`).
    stuck: bool,
}

impl Watchdog {
    /// Starts watching, on a thread of its own; `tell` hears each note, from either thread.
    pub fn start(limits: Limits, tell: impl Fn(Note) + Send + Sync + 'static) -> std::io::Result<Watchdog> {
        let tell = Box::new(tell);
        let shared = Arc::new(Shared { spell: Mutex::default(), changed: Condvar::new(), limits, tell });
        let watched = shared.clone();
        std::thread::Builder::new().name("asides-watchdog".into()).spawn(move || watch(&watched))?;
        Ok(Watchdog(shared))
    }

    /// The thread is at work: a new spell (the run loop going round again without waiting starts
    /// one too: each must end in time).
    pub fn working(&self) {
        self.next(true);
    }

    /// The thread waits for events.
    pub fn waiting(&self) {
        self.next(false);
    }

    fn next(&self, working: bool) {
        let now = Instant::now();
        let mut spell = self.0.lock();
        let ended = std::mem::replace(&mut *spell, Spell { since: working.then_some(now), stuck: false });
        if (working && ended.since.is_none()) || ended.stuck {
            self.0.changed.notify_one();
        }
        drop(spell);
        let Some(since) = ended.since else { return };
        let took = now - since;
        let note = match ended.stuck {
            true => Note::WentOn(took),
            false if took >= self.0.limits.slow => Note::Slow(took),
            false => return,
        };
        (self.0.tell)(note);
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, Spell> {
        self.spell.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn watch(shared: &Shared) {
    let mut spell = shared.lock();
    loop {
        let limit = match (spell.since, spell.stuck) {
            (None, _) => None,
            (Some(_), false) => Some(shared.limits.stuck),
            (Some(_), true) => shared.limits.give_up,
        };
        let (Some(since), Some(limit)) = (spell.since, limit) else {
            // Nothing to time: until a spell starts, or the stuck one ends.
            spell = shared.changed.wait(spell).unwrap_or_else(PoisonError::into_inner);
            continue;
        };
        let now = Instant::now();
        if now < since + limit {
            spell = shared.changed.wait_timeout(spell, since + limit - now).unwrap_or_else(PoisonError::into_inner).0;
            continue;
        }
        let stuck = std::mem::replace(&mut spell.stuck, true);
        drop(spell);
        if stuck {
            return (shared.tell)(Note::GiveUp(now - since));
        }
        (shared.tell)(Note::Stuck(now - since));
        spell = shared.lock();
    }
}

/// Notices something that runs again and again (the overlay's clock): `limit` times within a
/// second is a spin, told once until a second goes by below it.
pub struct Spin {
    limit: u32,
    second: Option<Instant>,
    count: u32,
    told: bool,
}

impl Spin {
    pub fn new(limit: u32) -> Spin {
        Spin { limit, second: None, count: 0, told: false }
    }

    /// It ran again: how many times in this second, when it's a spin not told yet.
    pub fn ran(&mut self, now: Instant) -> Option<u32> {
        match self.second {
            Some(start) if now - start < Duration::from_secs(1) => self.count += 1,
            _ => {
                self.told &= self.count >= self.limit;
                (self.second, self.count) = (Some(now), 1);
            }
        }
        (self.count >= self.limit && !std::mem::replace(&mut self.told, true)).then_some(self.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc::{Receiver, channel};

    const MS: Duration = Duration::from_millis(1);

    fn watchdog(give_up: Option<Duration>) -> (Watchdog, Receiver<Note>) {
        let (tx, rx) = channel();
        let tx = Mutex::new(tx);
        let limits = Limits { slow: 40 * MS, stuck: 150 * MS, give_up };
        let watchdog = Watchdog::start(limits, move |note| {
            let _ = tx.lock().unwrap().send(note);
        })
        .unwrap();
        (watchdog, rx)
    }

    #[test]
    fn tells_slow_spells_as_they_end() {
        let (watchdog, notes) = watchdog(None);
        watchdog.working();
        watchdog.waiting();
        watchdog.working();
        std::thread::sleep(60 * MS);
        watchdog.waiting();
        let Ok(Note::Slow(took)) = notes.try_recv() else { panic!("no slow spell told") };
        assert!(took >= 60 * MS);
        // Waiting is no spell: nothing is told, however long.
        std::thread::sleep(200 * MS);
        assert!(notes.try_recv().is_err());
    }

    #[test]
    fn tells_a_stuck_spell_while_it_lasts_then_when_it_ends() {
        let (watchdog, notes) = watchdog(None);
        watchdog.working();
        let Ok(Note::Stuck(took)) = notes.recv_timeout(2000 * MS) else { panic!("not told stuck") };
        assert!(took >= 150 * MS);
        watchdog.working();
        assert!(matches!(notes.try_recv(), Ok(Note::WentOn(d)) if d >= 150 * MS));
        // The next spell is watched again.
        assert!(matches!(notes.recv_timeout(2000 * MS), Ok(Note::Stuck(_))));
    }

    #[test]
    fn gives_up_on_a_spell_that_never_ends() {
        let (watchdog, notes) = watchdog(Some(300 * MS));
        watchdog.working();
        assert!(matches!(notes.recv_timeout(2000 * MS), Ok(Note::Stuck(_))));
        let Ok(Note::GiveUp(took)) = notes.recv_timeout(2000 * MS) else { panic!("didn't give up") };
        assert!(took >= 300 * MS);
    }

    #[test]
    fn spells_going_round_without_waiting_are_each_timed() {
        let (watchdog, notes) = watchdog(None);
        // Busy for longer than `stuck` in all, but never stuck in one spell.
        for _ in 0..10 {
            watchdog.working();
            std::thread::sleep(30 * MS);
        }
        watchdog.waiting();
        // A sleep can overrun into a slow spell, never a stuck one.
        assert!(notes.try_iter().all(|note| matches!(note, Note::Slow(_))));
    }

    #[test]
    fn a_spin_is_told_once_until_it_stops() {
        let mut spin = Spin::new(100);
        let start = Instant::now();
        let told: Vec<_> = (0..300).filter_map(|i| spin.ran(start + i * MS)).collect();
        assert_eq!(told, [100]);
        // A quiet second, then spinning again: told again.
        assert_eq!(spin.ran(start + 1500 * MS), None);
        assert_eq!(spin.ran(start + 2600 * MS), None);
        let told: Vec<_> = (0..300).filter_map(|i| spin.ran(start + 3000 * MS + i * MS)).collect();
        assert_eq!(told, [100]);
    }
}
