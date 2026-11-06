//! What's shared about typing into a terminal on Linux: a terminal holds its pty's master, and what
//! it writes there is what the keyboard typed. `asides-keys` (`main.rs`) writes there too, and the
//! program reading the terminal can't tell, whatever the terminal, the desktop or the display
//! server. Reaching another process's master needs `pidfd_getfd`, which the kernel allows as it
//! allows `ptrace` (Yama's `ptrace_scope`): at scope 0 (Fedora) any process of the user may; at 1
//! (Ubuntu, Debian, Arch…) `asides-keys` needs the capability to, and only it holds it.

#![cfg(target_os = "linux")]

use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

/// The helper's file name, beside the app's.
pub const HELPER: &str = "asides-keys";

/// `kernel.yama.ptrace_scope` (0 when there's no Yama).
pub fn ptrace_scope() -> u8 {
    std::fs::read_to_string("/proc/sys/kernel/yama/ptrace_scope")
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

/// The file has a capability set (`setcap`), `CAP_SYS_PTRACE` in its permitted set.
pub fn has_capability(file: &Path) -> bool {
    // `struct vfs_cap_data` in the file's `security.capability`: a magic word, then the permitted
    // and inheritable sets, low 32 bits first.
    const CAP_SYS_PTRACE: u32 = 19;
    let Ok(path) = CString::new(file.as_os_str().as_bytes()) else { return false };
    let mut data = [0u8; 24];
    // SAFETY: both strings are nul-terminated; the kernel writes at most `data.len()` bytes.
    let got =
        unsafe { libc::getxattr(path.as_ptr(), c"security.capability".as_ptr(), data.as_mut_ptr().cast(), data.len()) };
    if got < 8 {
        return false;
    }
    let permitted = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    permitted & (1 << CAP_SYS_PTRACE) != 0
}

/// `helper` can type into the user's terminals: the kernel lets it (scope 0), or it holds the
/// capability (scopes 1 and 2; at 3 nothing may, until a reboot).
pub fn may_type(helper: &Path) -> bool {
    match ptrace_scope() {
        0 => true,
        3 => false,
        _ => has_capability(helper),
    }
}

/// The master of pty `index` (`/dev/pts/<index>`) among what `fdinfo` says of a process's file
/// descriptors: the kernel's `tty-index` line.
pub fn holds_pty(fdinfo: &str, index: u32) -> bool {
    fdinfo.lines().filter_map(|l| l.strip_prefix("tty-index:")).any(|n| n.trim().parse() == Ok(index))
}

/// All three of a process's user ids (`/proc/<pid>/status`' `Uid:` line, real, effective, saved and
/// file system's) are `uid`.
pub fn status_is_of(status: &str, uid: u32) -> bool {
    let Some(line) = status.lines().find_map(|l| l.strip_prefix("Uid:")) else { return false };
    let ids: Vec<&str> = line.split_whitespace().collect();
    ids.len() == 4 && ids.iter().all(|id| id.parse() == Ok(uid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_pty_a_descriptor_is_the_master_of() {
        let info = "pos:\t0\nflags:\t02\nmnt_id:\t28\nino:\t173\ntty-index:\t4\n";
        assert!(holds_pty(info, 4));
        assert!(!holds_pty(info, 44));
        assert!(!holds_pty("pos:\t0\nflags:\t02\n", 4));
    }

    #[test]
    fn a_process_of_another_user_or_setuid_is_not_the_users() {
        assert!(status_is_of("Name:\tbash\nUid:\t1000\t1000\t1000\t1000\n", 1000));
        assert!(!status_is_of("Uid:\t1000\t0\t1000\t1000\n", 1000));
        assert!(!status_is_of("Uid:\t0\t0\t0\t0\n", 1000));
        assert!(!status_is_of("Name:\tbash\n", 1000));
    }

    #[test]
    fn a_file_without_a_capability_has_none() {
        assert!(!has_capability(Path::new("/proc/self/exe")));
        assert!(!has_capability(Path::new("/no/such/file")));
    }
}
