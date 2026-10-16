//! Which directory a session is bound to, and the git facts recorded when a note is created.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::paths::NOTES_DIR;

/// The repo root when `start` is inside a git work tree, otherwise `start` itself.
pub fn resolve_bind_dir(start: &Path) -> io::Result<PathBuf> {
    let start = start.canonicalize()?;
    Ok(git(&start, &["rev-parse", "--show-toplevel"]).map(PathBuf::from).unwrap_or(start))
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct GitInfo {
    pub remote: Option<String>,
    pub branch: Option<String>,
    pub commit: Option<String>,
    pub commit_subject: Option<String>,
    pub uncommitted_files: usize,
}

/// The branch checked out in `dir`, if it's a git work tree on a branch.
pub fn current_branch(dir: &Path) -> Option<String> {
    git(dir, &["branch", "--show-current"]).filter(|b| !b.is_empty())
}

/// `None` when `dir` is not inside a git work tree.
pub fn git_info(dir: &Path) -> Option<GitInfo> {
    if git(dir, &["rev-parse", "--is-inside-work-tree"]).as_deref() != Some("true") {
        return None;
    }
    let remote = git(dir, &["remote", "get-url", "origin"]).or_else(|| {
        let first = git(dir, &["remote"])?.lines().next()?.to_string();
        git(dir, &["remote", "get-url", &first])
    });
    // Unlike `rev-parse --abbrev-ref HEAD`, this also names a branch with no commits yet.
    let branch = git(dir, &["branch", "--show-current"]);
    let uncommitted_files = git(dir, &["status", "--porcelain"]).map(|s| s.lines().count()).unwrap_or(0);
    Some(GitInfo {
        remote,
        branch,
        commit: git(dir, &["rev-parse", "--short", "HEAD"]),
        commit_subject: git(dir, &["log", "-1", "--format=%s"]),
        uncommitted_files,
    })
}

/// Adds `.asides/` to the repo's `.git/info/exclude` (a local-only ignore list that never
/// touches `.gitignore`). Returns `true` if the line was added.
pub fn ensure_git_exclude(dir: &Path) -> io::Result<bool> {
    let Some(rel) = git(dir, &["rev-parse", "--git-path", "info/exclude"]) else {
        return Ok(false);
    };
    let path = dir.join(rel);
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let pattern = format!("{NOTES_DIR}/");
    let already = existing.lines().map(str::trim).any(|line| {
        line == pattern || line == NOTES_DIR || line == format!("/{pattern}") || line == format!("/{NOTES_DIR}")
    });
    if already {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::OpenOptions::new().create(true).append(true).open(&path)?;
    let sep = if existing.is_empty() || existing.ends_with('\n') { "" } else { "\n" };
    write!(file, "{sep}# asides notes (local only)\n{pattern}\n")?;
    Ok(true)
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(dir: &Path, args: &[&str]) {
        let ok = Command::new("git").arg("-C").arg(dir).args(args).output().unwrap().status.success();
        assert!(ok, "git {args:?} failed");
    }

    fn repo() -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        run(tmp.path(), &["init", "-q", "-b", "main"]);
        run(tmp.path(), &["config", "user.email", "t@example.com"]);
        run(tmp.path(), &["config", "user.name", "T"]);
        fs::write(tmp.path().join("a.txt"), "a").unwrap();
        run(tmp.path(), &["add", "."]);
        run(tmp.path(), &["commit", "-q", "-m", "First commit"]);
        tmp
    }

    #[test]
    fn binds_to_repo_root_from_subdirectory() {
        let tmp = repo();
        let sub = tmp.path().join("src/deep");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(resolve_bind_dir(&sub).unwrap(), tmp.path().canonicalize().unwrap());
    }

    #[test]
    fn binds_to_plain_directory_outside_git() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_bind_dir(tmp.path()).unwrap(), tmp.path().canonicalize().unwrap());
        assert_eq!(git_info(tmp.path()), None);
        assert!(!ensure_git_exclude(tmp.path()).unwrap());
    }

    #[test]
    fn collects_git_info() {
        let tmp = repo();
        run(tmp.path(), &["remote", "add", "origin", "git@github.com:acme/shop.git"]);
        fs::write(tmp.path().join("b.txt"), "b").unwrap();
        fs::write(tmp.path().join("a.txt"), "changed").unwrap();
        let info = git_info(tmp.path()).unwrap();
        assert_eq!(info.remote.as_deref(), Some("git@github.com:acme/shop.git"));
        assert_eq!(info.branch.as_deref(), Some("main"));
        assert_eq!(info.commit_subject.as_deref(), Some("First commit"));
        assert_eq!(info.commit.as_ref().map(|c| c.len() >= 7), Some(true));
        assert_eq!(info.uncommitted_files, 2);
    }

    #[test]
    fn names_the_branch_of_a_repo_without_commits() {
        let tmp = tempfile::tempdir().unwrap();
        run(tmp.path(), &["init", "-q", "-b", "trunk"]);
        fs::write(tmp.path().join("a.txt"), "a").unwrap();
        let info = git_info(tmp.path()).unwrap();
        assert_eq!(info.branch.as_deref(), Some("trunk"));
        assert_eq!((info.commit, info.commit_subject, info.uncommitted_files), (None, None, 1));
    }

    #[test]
    fn git_exclude_is_added_once_and_hides_notes() {
        let tmp = repo();
        assert!(ensure_git_exclude(tmp.path()).unwrap());
        assert!(!ensure_git_exclude(tmp.path()).unwrap());
        fs::create_dir_all(tmp.path().join(".asides/x")).unwrap();
        fs::write(tmp.path().join(".asides/x/notes.md"), "hi").unwrap();
        assert_eq!(git_info(tmp.path()).unwrap().uncommitted_files, 0);
    }
}
