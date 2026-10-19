//! Putting the `asides` command on the PATH when the app was installed by hand, from the disk image
//! or the zip. The Homebrew cask and `make install` link it themselves.

use std::path::{Path, PathBuf};

/// Where the app links the command: a folder on every shell's default PATH (`/etc/paths`).
pub const LINK: &str = "/usr/local/bin/asides";

/// The folders where an `asides` command means it's already set up: the app's link, Homebrew's
/// (the cask) and `make install`'s.
pub fn command_dirs(home: &Path) -> Vec<PathBuf> {
    vec!["/usr/local/bin".into(), "/opt/homebrew/bin".into(), home.join(".local/bin")]
}

/// Whether one of `dirs` has an `asides` command that works (a file, or a link that resolves to one).
pub fn command_installed(dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|dir| dir.join("asides").is_file())
}

/// The CLI inside the bundle at `app`, if the bundle is installed in an Applications folder. Not
/// when it runs from the disk image, a build folder or App Translocation: a link there would break.
pub fn installed_cli(app: &Path, home: &Path) -> Option<PathBuf> {
    let parent = app.parent()?;
    let installed = parent == Path::new("/Applications") || parent == home.join("Applications");
    let cli = app.join("Contents/Helpers/asides");
    (installed && app.extension()? == "app" && cli.is_file()).then_some(cli)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_an_installed_command() {
        let tmp = tempfile::tempdir().unwrap();
        let (bin, other) = (tmp.path().join("bin"), tmp.path().join("other"));
        fs::create_dir_all(&bin).unwrap();
        assert!(!command_installed(&[bin.clone(), other.clone()]));
        // A link to an app that's gone doesn't count.
        std::os::unix::fs::symlink(tmp.path().join("gone"), bin.join("asides")).unwrap();
        assert!(!command_installed(&[bin.clone(), other.clone()]));
        fs::write(tmp.path().join("gone"), "").unwrap();
        assert!(command_installed(&[other, bin]));
    }

    #[test]
    fn links_only_from_an_applications_folder() {
        let home = tempfile::tempdir().unwrap();
        let bundle = |parent: &Path| {
            let app = parent.join("asides.app");
            fs::create_dir_all(app.join("Contents/Helpers")).unwrap();
            fs::write(app.join("Contents/Helpers/asides"), "").unwrap();
            app
        };
        let app = bundle(&home.path().join("Applications"));
        assert_eq!(installed_cli(&app, home.path()), Some(app.join("Contents/Helpers/asides")));
        assert_eq!(installed_cli(&bundle(&home.path().join("Downloads")), home.path()), None);
        assert_eq!(installed_cli(&home.path().join("Applications/Missing.app"), home.path()), None);
    }
}
