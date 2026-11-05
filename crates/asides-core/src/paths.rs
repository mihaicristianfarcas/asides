use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "asides";
pub const BUNDLE_ID: &str = "io.github.mihaicristianfarcas.asides";
pub const URL_SCHEME: &str = "asides";

/// `~/Library/Application Support/asides`; on Windows `%LOCALAPPDATA%\asides` (the voice models
/// are large: not in the roaming profile).
pub fn support_dir() -> PathBuf {
    #[cfg(windows)]
    return local_app_data().join(APP_NAME);
    #[cfg(not(windows))]
    std::env::home_dir().expect("HOME is not set").join("Library/Application Support").join(APP_NAME)
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

pub fn config_path() -> PathBuf {
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
    #[cfg(not(windows))]
    std::env::home_dir().expect("HOME is not set").join("Library/Caches").join(APP_NAME).join("updates")
}

/// `~/Library/Logs/asides`: the log (`log::path`), where Console looks. On Windows
/// `%LOCALAPPDATA%\asides\Logs`.
pub fn logs_dir() -> PathBuf {
    #[cfg(windows)]
    return support_dir().join("Logs");
    #[cfg(not(windows))]
    std::env::home_dir().expect("HOME is not set").join("Library/Logs").join(APP_NAME)
}

/// Where the system keeps the crash reports of the user's apps: macOS's diagnostic reports;
/// Windows Error Reporting's local dumps, when they're turned on (`asides log` reads only macOS's).
pub fn crash_reports_dir() -> PathBuf {
    #[cfg(windows)]
    return local_app_data().join("CrashDumps");
    #[cfg(not(windows))]
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
