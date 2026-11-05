//! One asides running per user, as Launch Services keeps one on a Mac. The copy running owns a Unix
//! socket in the user's runtime folder (`paths::links_socket`, private to them): a copy started
//! later hands its `asides://` links to it there and ends, and so does the command. Started again
//! without a link (from the desktop's launcher), it hands over `asides://window`: the window comes
//! forward.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};

use asides_core::paths;

/// This copy is the one running: its socket, and the links it was started with.
pub struct Instance {
    socket: Option<UnixListener>,
    links: Vec<url::Url>,
}

/// The `asides://` links among this copy's arguments.
fn links() -> Vec<url::Url> {
    std::env::args().skip(1).filter_map(|a| url::Url::parse(&a).ok()).filter(|u| u.scheme() == "asides").collect()
}

/// This copy, if it's the first: else its links go to the one running and it should end (`None`).
pub fn first() -> Option<Instance> {
    let links = links();
    let path = paths::links_socket();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut running) = UnixStream::connect(&path) {
        let handed = match links.is_empty() {
            true => "asides://window\n".to_string(),
            false => links.iter().map(|l| format!("{l}\n")).collect(),
        };
        if running.write_all(handed.as_bytes()).is_ok() {
            return None;
        }
    }
    // Nobody listens: what's there is left by a copy that ended.
    let _ = std::fs::remove_file(&path);
    let socket = UnixListener::bind(&path);
    if let Err(err) = &socket {
        eprintln!("asides: not taking links ({}): {err}", path.display());
    }
    Some(Instance { socket: socket.ok(), links })
}

impl Instance {
    /// Hands each link to `handle`: the ones this copy was started with, then each a later copy or
    /// the command hands over, from a thread of its own; until `handle` says to stop.
    pub fn listen(self, handle: impl Fn(url::Url) -> bool + Send + 'static) {
        let Instance { socket, links } = self;
        let spawned = std::thread::Builder::new().name("asides-links".into()).spawn(move || {
            for link in links {
                if !handle(link) {
                    return;
                }
            }
            let Some(socket) = socket else { return };
            for client in socket.incoming() {
                let Ok(client) = client else { continue };
                for line in BufReader::new(client).lines().map_while(Result::ok) {
                    if let Ok(link) = url::Url::parse(line.trim())
                        && link.scheme() == "asides"
                        && !handle(link)
                    {
                        return;
                    }
                }
            }
        });
        if let Err(err) = spawned {
            eprintln!("asides: not taking links from other copies: {err}");
        }
    }
}
