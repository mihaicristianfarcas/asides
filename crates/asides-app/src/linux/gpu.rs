//! What GPUI draws with. A GPU's driver can crash it: Mesa's Venus (UTM's virtual GPU) aborts as
//! GPUI makes its shaders, taking the overlay or the window with it. Once one of them has crashed,
//! those started after draw with Mesa's software Vulkan (lavapipe) when it's there: slower, but on
//! screen.

use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

/// Mesa's software Vulkan, once asides draws with it.
static ON_CPU: OnceLock<PathBuf> = OnceLock::new();

/// Whether a GPUI process ended this way by itself: asides ends them by closing their input, or
/// with SIGKILL.
pub fn crashed(status: ExitStatus) -> bool {
    matches!(
        status.signal(),
        Some(libc::SIGSEGV | libc::SIGABRT | libc::SIGBUS | libc::SIGILL | libc::SIGFPE | libc::SIGTRAP)
    )
}

/// A GPUI process crashed on the GPU: those started from now on draw on the CPU, when Mesa's
/// software Vulkan is there. Whether they do.
pub fn draw_on_cpu() -> bool {
    if ON_CPU.get().is_some() {
        return true;
    }
    let Some(driver) = software_driver() else {
        asides_core::log::problem(format_args!("no software Vulkan (Mesa's lavapipe) to draw with instead"));
        return false;
    };
    asides_core::log::problem(format_args!("drawing on the CPU from now on ({})", driver.display()));
    let _ = ON_CPU.set(driver);
    true
}

/// A GPUI process about to start: once asides draws on the CPU, Vulkan offers it Mesa's software
/// driver alone. Whether it does.
pub fn configure(command: &mut Command) -> bool {
    let Some(driver) = ON_CPU.get() else { return false };
    // `VK_DRIVER_FILES` since Vulkan's loader 1.3.207 (2022), `VK_ICD_FILENAMES` before.
    command.env("VK_DRIVER_FILES", driver).env("VK_ICD_FILENAMES", driver);
    true
}

/// Calls `ended` from a thread of its own when the child process `pid` ends, leaving it for its
/// `Child` to collect.
pub fn when_ended(pid: u32, ended: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new().name("asides-window-exit".into()).spawn(move || {
        // SAFETY: `info` is written by the call; WNOWAIT leaves the child unreaped.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        unsafe { libc::waitid(libc::P_PID, pid, &mut info, libc::WEXITED | libc::WNOWAIT) };
        ended();
    });
    if let Err(err) = spawned {
        eprintln!("asides: not watching the window's process: {err}");
    }
}

/// lavapipe's manifest, where Vulkan's loader looks for drivers.
fn software_driver() -> Option<PathBuf> {
    let data = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty());
    let data = data.as_deref().unwrap_or("/usr/local/share:/usr/share");
    let dirs = std::iter::once(Path::new("/etc")).chain(data.split(':').filter(|d| !d.is_empty()).map(Path::new));
    dirs.map(|dir| dir.join("vulkan/icd.d")).find_map(|dir| lavapipe_in(&dir))
}

/// `lvp_icd.x86_64.json` (or `lvp_icd.json`) in `dir`: Mesa's software Vulkan, for this computer.
fn lavapipe_in(dir: &Path) -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.file_name().and_then(|n| n.to_str()).is_some_and(is_lavapipe))
        .collect();
    found.sort();
    found.into_iter().next()
}

fn is_lavapipe(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("lvp_icd.").and_then(|n| n.strip_suffix("json")) else { return false };
    rest.is_empty() || rest == format!("{}.", std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_lavapipe_for_this_computer() {
        let arch = std::env::consts::ARCH;
        assert!(is_lavapipe(&format!("lvp_icd.{arch}.json")));
        assert!(is_lavapipe("lvp_icd.json"));
        assert!(!is_lavapipe("lvp_icd.i686.json"));
        assert!(!is_lavapipe("virtio_icd.aarch64.json"));
        let dir = tempfile::tempdir().unwrap();
        for name in ["radeon_icd.json".to_string(), "lvp_icd.i686.json".into(), format!("lvp_icd.{arch}.json")] {
            std::fs::write(dir.path().join(name), "{}").unwrap();
        }
        assert_eq!(lavapipe_in(dir.path()), Some(dir.path().join(format!("lvp_icd.{arch}.json"))));
    }

    #[test]
    fn a_crash_is_a_signal_nobody_sent() {
        assert!(crashed(ExitStatus::from_raw(libc::SIGSEGV)));
        assert!(crashed(ExitStatus::from_raw(libc::SIGABRT | 0x80)));
        assert!(!crashed(ExitStatus::from_raw(libc::SIGKILL)));
        assert!(!crashed(ExitStatus::from_raw(libc::SIGTERM)));
        // Exit status 101: a panic, which no other driver would fix.
        assert!(!crashed(ExitStatus::from_raw(101 << 8)));
    }
}
