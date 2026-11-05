//! `asides-keys <pts index>`: writes standard input to the master of `/dev/pts/<index>`, as if typed
//! on its terminal's keyboard.
//!
//! This small program alone may hold `CAP_SYS_PTRACE`, on systems whose `ptrace_scope` needs it
//! (the packages set it, or setup does: `lib.rs`), so it does the one thing, for the user running it:
//! the pty must be the user's, the process holding its master must be entirely the user's, and what
//! it takes from that process must be that very pty's master. It gives the capability up as soon as
//! it has it, before reading a byte of what it's asked to type. Nothing else is touched.

use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::fs::MetadataExt;

use asides_keys::{holds_pty, status_is_of};

pub fn main() {
    let index = std::env::args().nth(1).and_then(|a| a.parse::<u32>().ok());
    let Some(index) = index else {
        eprintln!("usage: asides-keys <pts index> (what to type on standard input)");
        std::process::exit(2);
    };
    if let Err(message) = run(index) {
        eprintln!("asides-keys: {message}");
        std::process::exit(1);
    }
}

fn run(index: u32) -> Result<(), String> {
    // SAFETY: a plain query.
    let uid = unsafe { libc::getuid() };
    let device = std::fs::metadata(format!("/dev/pts/{index}")).map_err(|_| "that terminal has closed".to_string())?;
    if device.uid() != uid {
        return Err("that terminal is another user's".into());
    }
    let master = master(index, uid)?;
    drop_capabilities()?;
    let mut text = Vec::new();
    std::io::stdin().read_to_end(&mut text).map_err(|err| err.to_string())?;
    write_all(master.as_raw_fd(), &text)
}

/// The master of pty `index`, duplicated from the process of `uid` that holds it.
fn master(index: u32, uid: u32) -> Result<OwnedFd, String> {
    let mut declined = None;
    let processes = std::fs::read_dir("/proc").map_err(|err| err.to_string())?;
    for entry in processes.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<i32>().ok()) else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != uid {
            continue;
        }
        let Ok(fds) = std::fs::read_dir(format!("/proc/{pid}/fd")) else { continue };
        for fd in fds.flatten() {
            let Some(number) = fd.file_name().to_str().and_then(|n| n.parse::<RawFd>().ok()) else { continue };
            let is_ptmx = std::fs::read_link(fd.path()).is_ok_and(|to| to.ends_with("ptmx"));
            if !is_ptmx {
                continue;
            }
            let info = std::fs::read_to_string(format!("/proc/{pid}/fdinfo/{number}")).unwrap_or_default();
            if !holds_pty(&info, index) {
                continue;
            }
            match take(pid, number, index, uid) {
                Ok(Some(master)) => return Ok(master),
                Ok(None) => {}
                Err(why) => declined = Some(why),
            }
        }
    }
    Err(match declined {
        Some(Declined::NotAllowed) => "this system doesn't let asides type into terminals \
                                       (kernel.yama.ptrace_scope): allow it in asides' setup, or see its \
                                       troubleshooting guide"
            .into(),
        Some(Declined::NoSuchCall) => "this kernel can't lend asides a terminal: it needs Linux 5.6 or later".into(),
        None => "no process holds that terminal".into(),
    })
}

/// Why the kernel didn't hand over a terminal's master.
enum Declined {
    /// `ptrace_scope` keeps it from this process.
    NotAllowed,
    /// No `pidfd_getfd` (before Linux 5.6, or under an emulator).
    NoSuchCall,
}

/// Why a system call failed.
fn last_error() -> Declined {
    match std::io::Error::last_os_error().raw_os_error() {
        Some(libc::ENOSYS) => Declined::NoSuchCall,
        _ => Declined::NotAllowed,
    }
}

/// Process `pid`'s descriptor `fd`, if it's the master of pty `index` and the process is entirely
/// `uid`'s; `Err` when the kernel declined.
fn take(pid: i32, fd: RawFd, index: u32, uid: u32) -> Result<Option<OwnedFd>, Declined> {
    // SAFETY: plain system calls; each descriptor returned is owned at once.
    unsafe {
        let pidfd = libc::syscall(libc::SYS_pidfd_open, pid, 0) as RawFd;
        if pidfd < 0 {
            // Gone, unless there's no such call.
            return match last_error() {
                Declined::NoSuchCall => Err(Declined::NoSuchCall),
                Declined::NotAllowed => Ok(None),
            };
        }
        let pidfd = OwnedFd::from_raw_fd(pidfd);
        // Asked after the handle is held: a process that's still there now is the one that has it.
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap_or_default();
        if !status_is_of(&status, uid)
            || libc::syscall(libc::SYS_pidfd_send_signal, pidfd.as_raw_fd(), 0, 0usize, 0u32) != 0
        {
            return Ok(None);
        }
        let got = libc::syscall(libc::SYS_pidfd_getfd, pidfd.as_raw_fd(), fd, 0) as RawFd;
        if got < 0 {
            return Err(last_error());
        }
        let master = OwnedFd::from_raw_fd(got);
        let mut number: libc::c_uint = 0;
        let is_it = libc::ioctl(master.as_raw_fd(), libc::TIOCGPTN, &mut number) == 0 && number == index;
        Ok(is_it.then_some(master))
    }
}

/// Gives up every capability this process holds, for good.
fn drop_capabilities() -> Result<(), String> {
    #[repr(C)]
    struct Header {
        version: u32,
        pid: i32,
    }
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Data {
        effective: u32,
        permitted: u32,
        inheritable: u32,
    }
    /// `_LINUX_CAPABILITY_VERSION_3`.
    const VERSION_3: u32 = 0x2008_0522;
    let header = Header { version: VERSION_3, pid: 0 };
    let data = [Data { effective: 0, permitted: 0, inheritable: 0 }; 2];
    // SAFETY: the header and the two data structs version 3 takes.
    let done = unsafe { libc::syscall(libc::SYS_capset, &header, data.as_ptr()) };
    match done {
        0 => Ok(()),
        _ => Err(format!("could not give up its capability: {}", std::io::Error::last_os_error())),
    }
}

/// Writes all of `text`: a terminal's master is often non-blocking, and takes a few kilobytes at a
/// time, as the program reading the terminal reads them.
fn write_all(fd: RawFd, mut text: &[u8]) -> Result<(), String> {
    while !text.is_empty() {
        // SAFETY: at most `text.len()` bytes of `text` are read.
        let wrote = unsafe { libc::write(fd, text.as_ptr().cast(), text.len()) };
        if wrote >= 0 {
            text = &text[wrote as usize..];
            continue;
        }
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINTR) => {}
            Some(libc::EAGAIN) => {
                let mut ready = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                // SAFETY: one pollfd; waits until the terminal has room (or has closed).
                unsafe { libc::poll(&mut ready, 1, 10_000) };
                if ready.revents == 0 {
                    return Err("that terminal isn't reading what's typed".into());
                }
            }
            _ => return Err(format!("that terminal has closed: {err}")),
        }
    }
    Ok(())
}
