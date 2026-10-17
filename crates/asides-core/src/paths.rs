use std::path::{Path, PathBuf};

pub const APP_NAME: &str = "Asides";
pub const BUNDLE_ID: &str = "io.github.mihaicristianfarcas.asides";
pub const URL_SCHEME: &str = "asides";
/// Folder created inside the bound directory that holds one sub-folder per note.
pub const NOTES_DIR: &str = ".asides";
pub const NOTE_FILE: &str = "notes.md";

/// `~/Library/Application Support/Asides`
pub fn support_dir() -> PathBuf {
    std::env::home_dir().expect("HOME is not set").join("Library/Application Support").join(APP_NAME)
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

/// One record per agent session listening through `asides mcp` (see `agents::Listener`).
pub fn listeners_dir() -> PathBuf {
    support_dir().join("listeners")
}

/// One record per agent session (see `sessions::Session`).
pub fn sessions_dir() -> PathBuf {
    support_dir().join("sessions")
}

/// `/Users/me/code/shop` → `~/code/shop`.
pub fn tilde(path: &Path) -> String {
    match std::env::home_dir().and_then(|home| path.strip_prefix(&home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
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
        assert_eq!(tilde(&home.join("code/shop")), "~/code/shop");
        assert_eq!(tilde(Path::new("/tmp/x")), "/tmp/x");
    }
}
