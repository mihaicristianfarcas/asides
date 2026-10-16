//! Talking to the app: `Event`s as JSON lines on standard output, `Command`s from standard input
//! (see `asides_core::helper`).

use std::io::{BufRead, Write};
use std::sync::mpsc::{Receiver, channel};

use asides_core::helper::{self, Command, Event};

pub fn emit(event: &Event) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", helper::line(event));
    let _ = out.flush();
}

/// The app's commands, read on a thread. When the app goes away (standard input ends), the
/// overlay closes, unless `close_on_end` is off (a development run).
pub fn commands(close_on_end: bool) -> Receiver<Command> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            match serde_json::from_str::<Command>(&line) {
                Ok(command) => {
                    if tx.send(command).is_err() {
                        return;
                    }
                }
                Err(err) => eprintln!("asides-overlay: ignored {line:?}: {err}"),
            }
        }
        if close_on_end {
            let _ = tx.send(Command::Close);
        }
    });
    rx
}
