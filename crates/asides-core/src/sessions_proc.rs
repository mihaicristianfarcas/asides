//! What Linux's `/proc` says about a process: read there (`sessions_linux.rs`), and from Windows for
//! WSL's distributions (`sessions_wsl.rs`).

use std::collections::HashMap;

/// What `/proc/<pid>/stat` says.
#[derive(Debug, Clone, PartialEq)]
pub struct Stat {
    pub ppid: i32,
    /// `pts/3`, for a process with a pty.
    pub tty: Option<String>,
    /// When it started, in clock ticks since the computer did.
    pub start_ticks: u64,
}

/// Parses `/proc/<pid>/stat`: `pid (comm) state ppid pgrp session tty_nr …`, the name in parentheses
/// holding anything (spaces, parentheses).
pub fn parse_stat(text: &str) -> Option<Stat> {
    let rest = &text[text.rfind(')')? + 1..];
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // Field 3 of the file is the first here, so field n is `fields[n - 3]`.
    let ppid = fields.get(1)?.parse().ok()?;
    let tty = pts_name(fields.get(4)?.parse().ok()?);
    let start_ticks = fields.get(19)?.parse().ok()?;
    Some(Stat { ppid, tty, start_ticks })
}

/// `tty_nr` as a device name, for a pty (`/dev/pts/N`: majors 136 to 143).
fn pts_name(tty_nr: u32) -> Option<String> {
    let major = (tty_nr >> 8) & 0xfff;
    let minor = (tty_nr & 0xff) | ((tty_nr >> 12) & 0xfff00);
    (136..=143).contains(&major).then(|| format!("pts/{}", (major - 136) * 256 + minor))
}

/// `NUL`-separated strings (`cmdline`).
pub fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes.split(|&b| b == 0).filter(|w| !w.is_empty()).map(|w| String::from_utf8_lossy(w).into_owned()).collect()
}

/// `NAME=value` strings (`environ`), `NUL`-separated.
pub fn parse_environ(bytes: &[u8]) -> HashMap<String, String> {
    split_nul(bytes).into_iter().filter_map(|e| e.split_once('=').map(|(n, v)| (n.into(), v.into()))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stat_whatever_the_process_is_called() {
        let line = "4242 (node (claude) x) S 4000 4242 4000 34819 4242 4194560 1 2 3 4 5 6 7 8 20 0 11 0 987654 1 2";
        let stat = parse_stat(line).unwrap();
        assert_eq!(stat.ppid, 4000);
        assert_eq!(stat.tty.as_deref(), Some("pts/3"));
        assert_eq!(stat.start_ticks, 987654);
        // No terminal, and a console that isn't a pty.
        assert_eq!(parse_stat("1 (init) S 0 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 1 2").unwrap().tty, None);
        assert_eq!(parse_stat("1 (login) S 0 1 1 1025 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 5 1 2").unwrap().tty, None);
    }

    #[test]
    fn numbers_ptys_past_the_first_major() {
        // Major 137, minor 1: the 257th pty.
        assert_eq!(pts_name((137 << 8) | 1).as_deref(), Some("pts/257"));
    }

    #[test]
    fn splits_a_process_environment() {
        let vars = parse_environ(b"A=1\0B=x=y\0\0NOVALUE\0");
        assert_eq!(vars.get("A").map(String::as_str), Some("1"));
        assert_eq!(vars.get("B").map(String::as_str), Some("x=y"));
        assert_eq!(vars.len(), 2);
    }
}
