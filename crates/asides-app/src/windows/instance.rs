//! One asides running per user, as Launch Services keeps one on a Mac. Windows starts the app
//! again for each `asides://` link (the URL scheme's command, and the CLI, run `asides-app.exe
//! <link>`): the copy started hands its links to the one running, through a named pipe the first
//! copy owns, and ends. Started again without a link (from the Start menu), it hands over
//! `asides://window`: the window comes forward.
//!
//! The pipe is the user's (its name has their name; Windows' default security lets only them and
//! administrators write to it) and takes no clients from other computers.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::{FromRawHandle, OwnedHandle};

use windows::Win32::Foundation::{ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows::core::HSTRING;

/// This copy is the one running: its pipe, and the links it was started with.
pub struct Instance {
    pipe: HANDLE,
    links: Vec<url::Url>,
}

// The pipe's handle is only used by the listening thread once it's handed over.
unsafe impl Send for Instance {}

fn pipe_name() -> String {
    let user = std::env::var("USERNAME").unwrap_or_default().replace(['\\', '/'], "_");
    format!(r"\\.\pipe\asides-{user}")
}

/// The `asides://` links among this copy's arguments.
fn links() -> Vec<url::Url> {
    std::env::args().skip(1).filter_map(|a| url::Url::parse(&a).ok()).filter(|u| u.scheme() == "asides").collect()
}

/// This copy, if it's the first: else its links go to the one running and it should end (`None`).
pub fn first() -> Option<Instance> {
    let links = links();
    for _ in 0..20 {
        // SAFETY: a name and flags; the handle is owned by the `Instance`.
        let pipe = unsafe { create(true) };
        if pipe != INVALID_HANDLE_VALUE {
            return Some(Instance { pipe, links });
        }
        // Another copy has it: tell it.
        let handed = match links.is_empty() {
            true => vec!["asides://window".to_string()],
            false => links.iter().map(url::Url::to_string).collect(),
        };
        if hand_over(&handed) {
            return None;
        }
        // It's between two clients, or just ending: try again in a moment.
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    eprintln!("asides: another copy holds the pipe but takes nothing; running anyway");
    // SAFETY: as above.
    let pipe = unsafe { create(false) };
    Some(Instance { pipe, links })
}

unsafe fn create(first: bool) -> HANDLE {
    let flags = if first { PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE } else { PIPE_ACCESS_INBOUND };
    unsafe {
        CreateNamedPipeW(
            &HSTRING::from(pipe_name()),
            flags,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            0,
            4096,
            0,
            None,
        )
    }
}

/// Writes the links to the copy running, a line each.
fn hand_over(links: &[String]) -> bool {
    let Ok(mut pipe) = std::fs::OpenOptions::new().write(true).open(pipe_name()) else { return false };
    let text: String = links.iter().map(|l| format!("{l}\n")).collect();
    pipe.write_all(text.as_bytes()).is_ok()
}

impl Instance {
    /// Hands each link to `handle`: the ones this copy was started with, then each a later copy
    /// hands over, from a thread of its own; until `handle` says to stop.
    pub fn listen(self, handle: impl Fn(url::Url) -> bool + Send + 'static) {
        let instance = self;
        let spawned = std::thread::Builder::new().name("asides-links".into()).spawn(move || {
            // Moved whole (it's `Send`), not field by field.
            let instance = instance;
            let Instance { mut pipe, links } = instance;
            for link in links {
                if !handle(link) {
                    return;
                }
            }
            loop {
                // SAFETY: the pipe is this thread's; a client that connected before the wait is
                // connected too (`ERROR_PIPE_CONNECTED`).
                let connected = unsafe { ConnectNamedPipe(pipe, None) };
                if connected.is_err() && connected.as_ref().err().map(|e| e.code()) != Some(ERROR_PIPE_CONNECTED.into())
                {
                    return;
                }
                // The next copy's instance, ready before this one is read: no client finds none.
                // SAFETY: as in `first`.
                let next = unsafe { create(false) };
                // SAFETY: the handle is this thread's alone; reading it to its end, then dropping
                // the reader, leaves the pipe open (it's duplicated first).
                let read: Vec<String> = unsafe {
                    let mut copy = HANDLE::default();
                    let process = windows::Win32::System::Threading::GetCurrentProcess();
                    let duplicated = windows::Win32::Foundation::DuplicateHandle(
                        process,
                        pipe,
                        process,
                        &mut copy,
                        0,
                        false,
                        windows::Win32::Foundation::DUPLICATE_SAME_ACCESS,
                    );
                    match duplicated {
                        Ok(()) => {
                            let file = std::fs::File::from(OwnedHandle::from_raw_handle(copy.0));
                            BufReader::new(file).lines().map_while(Result::ok).collect()
                        }
                        Err(_) => Vec::new(),
                    }
                };
                // SAFETY: done with this client; its instance is closed, the next one waits.
                unsafe {
                    let _ = DisconnectNamedPipe(pipe);
                    let _ = windows::Win32::Foundation::CloseHandle(pipe);
                }
                pipe = next;
                for line in read {
                    if let Ok(link) = url::Url::parse(line.trim())
                        && link.scheme() == "asides"
                        && !handle(link)
                    {
                        return;
                    }
                }
                if pipe == INVALID_HANDLE_VALUE {
                    return;
                }
            }
        });
        if let Err(err) = spawned {
            eprintln!("asides: not taking links from other copies: {err}");
        }
    }
}
