use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "asides";
pub const BUNDLE_ID: &str = "io.github.mihaicristianfarcas.asides";
pub const URL_SCHEME: &str = "asides";

/// What users read for a directory, in their system's word: a folder on Windows, a directory on
/// macOS and Linux. Only for what they read (the overlay, the asides window, the CLI, errors);
/// code and comments say what they like.
pub const DIRECTORY: &str = if cfg!(windows) { "folder" } else { "directory" };
/// [`DIRECTORY`] in a title or on a button (`Choose Directory…`).
pub const DIRECTORY_TITLE: &str = if cfg!(windows) { "Folder" } else { "Directory" };

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

/// The repository's bug form (`.github/ISSUE_TEMPLATE/bug.yml`), with asides' version and the
/// system filled in (`system`: its name and version, as the app's log names it), the processor
/// after it. GitHub fills a form's text fields from the link's query, by their ids (never a
/// dropdown). Only that goes in the link: the user writes the rest, and pastes what they choose of
/// `asides log`.
pub fn report_url(system: &str) -> String {
    let processor = match std::env::consts::ARCH {
        "aarch64" if cfg!(target_os = "macos") => "Apple silicon",
        "x86_64" if cfg!(target_os = "macos") => "Intel",
        "aarch64" => "Arm64",
        "x86_64" if cfg!(windows) => "x64",
        arch => arch,
    };
    let fields = [
        ("template", "bug.yml".to_string()),
        ("version", env!("CARGO_PKG_VERSION").to_string()),
        ("system", format!("{system}, {processor}")),
    ];
    url::Url::parse_with_params(&format!("{REPOSITORY}/issues/new"), &fields)
        .map_or_else(|_| format!("{REPOSITORY}/issues/new"), |url| url.to_string())
}

/// asides' repository on GitHub.
pub const REPOSITORY: &str = "https://github.com/mihaicristianfarcas/asides";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_fills_in_the_version_and_the_system() {
        let link = url::Url::parse(&report_url("macOS 26.2 (25C56)")).unwrap();
        assert_eq!(link.as_str().split('?').next(), Some("https://github.com/mihaicristianfarcas/asides/issues/new"));
        let field = |name: &str| link.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned());
        assert_eq!(field("template").as_deref(), Some("bug.yml"));
        assert_eq!(field("version").as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert!(field("system").is_some_and(|s| s.starts_with("macOS 26.2 (25C56), ")));
        // The template's fields, each with the type above its id: a field renamed there no longer
        // fills in, and GitHub leaves a dropdown empty whatever the link says.
        let form = include_str!("../../../.github/ISSUE_TEMPLATE/bug.yml");
        let kind = |id: &str| {
            let mut kind = None;
            for line in form.lines().map(str::trim) {
                if let Some(k) = line.strip_prefix("- type: ") {
                    kind = Some(k);
                } else if line == format!("id: {id}") {
                    return kind;
                }
            }
            None
        };
        for id in ["version", "system"] {
            assert!(matches!(kind(id), Some("input" | "textarea")), "bug.yml has no text field {id}");
        }
    }

    #[test]
    fn shortens_home_paths() {
        let home = std::env::home_dir().unwrap();
        assert_eq!(tilde(&home), "~");
        let shop = home.join("code").join("shop");
        assert_eq!(tilde(&shop), format!("~{0}code{0}shop", std::path::MAIN_SEPARATOR));
        assert_eq!(tilde(Path::new("/tmp/x")), "/tmp/x");
    }

    #[test]
    fn says_directory_as_the_system_does() {
        let (word, title) = if cfg!(windows) { ("folder", "Folder") } else { ("directory", "Directory") };
        assert_eq!((DIRECTORY, DIRECTORY_TITLE), (word, title));
        assert_eq!(format!("Choose {DIRECTORY_TITLE}…"), format!("Choose {title}…"));
    }
}
