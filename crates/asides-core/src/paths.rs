use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "asides";
pub const BUNDLE_ID: &str = "io.github.mihaicristianfarcas.asides";
pub const URL_SCHEME: &str = "asides";

/// `~/Library/Application Support/asides`; on Windows `%LOCALAPPDATA%\asides` (the voice models
/// are large: not in the roaming profile); on Linux `~/.local/share/asides` (`$XDG_DATA_HOME`).
pub fn support_dir() -> PathBuf {
    #[cfg(windows)]
    return local_app_data().join(APP_NAME);
    #[cfg(target_os = "linux")]
    return xdg("XDG_DATA_HOME", ".local/share").join(APP_NAME);
    #[cfg(target_os = "macos")]
    std::env::home_dir().expect("HOME is not set").join("Library/Application Support").join(APP_NAME)
}

/// A folder of the XDG base directories: the variable's, when it's an absolute path, or its default
/// under the user's folder.
#[cfg(target_os = "linux")]
fn xdg(variable: &str, default: &str) -> PathBuf {
    match std::env::var_os(variable).map(PathBuf::from).filter(|dir| dir.is_absolute()) {
        Some(dir) => dir,
        None => std::env::home_dir().expect("HOME is not set").join(default),
    }
}

/// `$XDG_RUNTIME_DIR/asides.sock`, where the running app takes `asides://` links (`instance.rs`),
/// or a folder of the user's own in the temporary directory, on a system without one.
#[cfg(target_os = "linux")]
pub fn links_socket() -> PathBuf {
    // A test's own history is a test's own app: it mustn't hand its links to the user's.
    if let Some(home) = std::env::var_os("ASIDES_HOME").map(PathBuf::from).filter(|dir| dir.is_absolute()) {
        return home.join("asides.sock");
    }
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|dir| dir.is_absolute()) {
        Some(dir) => dir.join("asides.sock"),
        None => std::env::temp_dir().join(format!("asides-{}.sock", unsafe { libc::getuid() })),
    }
}

/// `%LOCALAPPDATA%` (`C:\Users\me\AppData\Local`).
#[cfg(windows)]
pub fn local_app_data() -> PathBuf {
    match std::env::var_os("LOCALAPPDATA").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => std::env::home_dir().expect("the user's folder is not known").join("AppData").join("Local"),
    }
}

pub fn state_path() -> PathBuf {
    support_dir().join("state.json")
}

/// In the support folder; on Linux `~/.config/asides/config.toml` (`$XDG_CONFIG_HOME`).
pub fn config_path() -> PathBuf {
    #[cfg(target_os = "linux")]
    return xdg("XDG_CONFIG_HOME", ".config").join(APP_NAME).join("config.toml");
    #[cfg(not(target_os = "linux"))]
    support_dir().join("config.toml")
}

pub fn models_dir() -> PathBuf {
    support_dir().join("models")
}

/// Updates downloaded, and the logs of their installs: `~/Library/Caches/asides/updates/<version>`
/// (Windows: `%LOCALAPPDATA%\asides\updates`).
pub fn updates_dir() -> PathBuf {
    #[cfg(windows)]
    return support_dir().join("updates");
    #[cfg(target_os = "linux")]
    return xdg("XDG_CACHE_HOME", ".cache").join(APP_NAME).join("updates");
    #[cfg(target_os = "macos")]
    std::env::home_dir().expect("HOME is not set").join("Library/Caches").join(APP_NAME).join("updates")
}

/// `~/Library/Logs/asides`: the log (`log::path`), where Console looks. On Windows
/// `%LOCALAPPDATA%\asides\Logs`; on Linux `~/.local/state/asides` (`$XDG_STATE_HOME`).
pub fn logs_dir() -> PathBuf {
    #[cfg(windows)]
    return support_dir().join("Logs");
    #[cfg(target_os = "linux")]
    return xdg("XDG_STATE_HOME", ".local/state").join(APP_NAME);
    #[cfg(target_os = "macos")]
    std::env::home_dir().expect("HOME is not set").join("Library/Logs").join(APP_NAME)
}

/// Where the system keeps the crash reports of the user's apps: macOS's diagnostic reports;
/// Windows Error Reporting's local dumps, when they're turned on (`asides log` reads only macOS's);
/// on Linux a folder nothing writes to: crashes go to systemd's journal (`coredumpctl`).
pub fn crash_reports_dir() -> PathBuf {
    #[cfg(windows)]
    return local_app_data().join("CrashDumps");
    #[cfg(target_os = "linux")]
    return logs_dir().join("crashes");
    #[cfg(target_os = "macos")]
    std::env::home_dir().expect("HOME is not set").join("Library/Logs/DiagnosticReports")
}

/// Launch scripts for new sessions (each removes itself once it runs).
pub fn launch_dir() -> PathBuf {
    std::env::temp_dir().join("asides-sessions")
}

/// `/Users/me/code/shop` → `~/code/shop` (Windows: `C:\Users\me\code\shop` → `~\code\shop`).
pub fn tilde(path: &Path) -> String {
    match std::env::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortens_home_paths() {
        let home = std::env::home_dir().unwrap();
        assert_eq!(tilde(&home), "~");
        let shop = home.join("code").join("shop");
        assert_eq!(tilde(&shop), format!("~{0}code{0}shop", std::path::MAIN_SEPARATOR));
        assert_eq!(tilde(Path::new("/tmp/x")), "/tmp/x");
    }
}
