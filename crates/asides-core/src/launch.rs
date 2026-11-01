//! Finding and starting an agent for a new session on Windows, without a shell between: the
//! aside, its first message, is an argument, which a shell (`cmd.exe` cuts it at a new line)
//! would mangle.
//!
//! Agents installed with npm are `.cmd` files (npm's shims) that run `node` on their script, or
//! the program the package ships (Claude Code's `bin\claude.exe`): those are run directly instead
//! (`program_command`). Native ones (`claude.exe`) are run as they are. The PATH is the user's as
//! the registry has it, as a new terminal gets it.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The extensions a program may have, in the order Windows tries them.
const EXTENSIONS: [&str; 3] = ["exe", "cmd", "bat"];

/// The program `name` in one of `path`'s folders: `name.exe`, `name.cmd` or `name.bat`.
pub fn find_program(path: &OsStr, name: &str) -> Option<PathBuf> {
    std::env::split_paths(path).find_map(|dir| {
        EXTENSIONS.iter().map(|ext| dir.join(format!("{name}.{ext}"))).find(|candidate| candidate.is_file())
    })
}

/// What an npm shim (`claude.cmd`) runs, relative to the shim's folder: a script it runs with
/// `node`, or a program; the last `"%dp0%\…"` it names that isn't `node.exe` (npm's `cmd-shim`).
pub fn npm_shim_script(shim: &str) -> Option<String> {
    shim.match_indices("\"%dp0%\\")
        .filter_map(|(at, marker)| {
            let rest = &shim[at + marker.len()..];
            let script = &rest[..rest.find('"')?];
            (!script.eq_ignore_ascii_case("node.exe") && !script.is_empty()).then(|| script.to_string())
        })
        .last()
}

/// How to run `program` with `args` without a shell: an npm shim's script under `node` (the one
/// beside the shim, else on the PATH), or the program it runs; anything else as it is.
pub fn program_command(program: &Path, args: &[String], path: &OsStr) -> (PathBuf, Vec<String>) {
    let shim = program.extension().is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let script = shim.then(|| std::fs::read_to_string(program).ok()).flatten().and_then(|t| npm_shim_script(&t));
    let runs_itself = |script: &str| Path::new(script).extension().is_some_and(|e| e.eq_ignore_ascii_case("exe"));
    match (script, program.parent()) {
        (Some(script), Some(dir)) if runs_itself(&script) => (dir.join(script), args.to_vec()),
        (Some(script), Some(dir)) => {
            let beside = dir.join("node.exe");
            let node = if beside.is_file() { Some(beside) } else { find_program(path, "node") };
            let mut words = vec![dir.join(script).to_string_lossy().into_owned()];
            words.extend(args.iter().cloned());
            (node.unwrap_or_else(|| "node".into()), words)
        }
        _ => (program.to_path_buf(), args.to_vec()),
    }
}

/// The user's PATH as a new terminal gets it: the system's, then the user's own (the registry's,
/// its `%VARIABLES%` expanded), or this process's elsewhere.
pub fn user_path() -> OsString {
    #[cfg(windows)]
    {
        let system = registry_path(true);
        let user = registry_path(false);
        let joined: Vec<String> = [system, user].into_iter().flatten().filter(|p| !p.is_empty()).collect();
        if !joined.is_empty() {
            return OsString::from(joined.join(";"));
        }
    }
    std::env::var_os("PATH").unwrap_or_default()
}

#[cfg(windows)]
fn registry_path(system: bool) -> Option<String> {
    use windows::Win32::System::Environment::ExpandEnvironmentStringsW;
    use windows::Win32::System::Registry::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_NOEXPAND, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
    };
    use windows::core::HSTRING;
    let (root, key) = match system {
        true => (HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment"),
        false => (HKEY_CURRENT_USER, "Environment"),
    };
    let (key, name) = (HSTRING::from(key), HSTRING::from("Path"));
    let flags =
        windows::Win32::System::Registry::REG_ROUTINE_FLAGS(RRF_RT_REG_SZ.0 | RRF_RT_REG_EXPAND_SZ.0 | RRF_NOEXPAND.0);
    let mut len = 0u32;
    // SAFETY: asked for its size, then read into a buffer that size; expanded the same way.
    unsafe {
        RegGetValueW(root, &key, &name, flags, None, None, Some(&mut len)).ok().ok()?;
        let mut data = vec![0u16; len as usize / 2 + 1];
        RegGetValueW(root, &key, &name, flags, None, Some(data.as_mut_ptr().cast()), Some(&mut len)).ok().ok()?;
        let raw: Vec<u16> = data.iter().copied().take_while(|&c| c != 0).chain([0]).collect();
        let needed = ExpandEnvironmentStringsW(windows::core::PCWSTR(raw.as_ptr()), None);
        let mut expanded = vec![0u16; needed as usize];
        let got = ExpandEnvironmentStringsW(windows::core::PCWSTR(raw.as_ptr()), Some(&mut expanded));
        let text = expanded.get(..(got as usize).saturating_sub(1)).unwrap_or_default();
        Some(String::from_utf16_lossy(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What npm writes for a package's command (`cmd-shim`).
    const SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL \
        :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \
        \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title \
        %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@anthropic-ai\\claude-code\\cli.js\" %*\r\n";

    /// What npm writes for a package whose command is a program (Claude Code from 2.1).
    const PROGRAM_SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\n\
        SETLOCAL\r\nCALL :find_dp0\r\n\"%dp0%\\..\\@anthropic-ai\\claude-code\\bin\\claude.exe\"   %*\r\n";

    #[test]
    fn finds_the_script_an_npm_shim_runs() {
        assert_eq!(npm_shim_script(SHIM).as_deref(), Some(r"node_modules\@anthropic-ai\claude-code\cli.js"));
        assert_eq!(npm_shim_script(PROGRAM_SHIM).as_deref(), Some(r"..\@anthropic-ai\claude-code\bin\claude.exe"));
        assert_eq!(npm_shim_script("@echo off\r\nclaude.exe %*"), None);
    }

    #[test]
    fn runs_the_program_a_shim_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let shim = tmp.path().join("claude.cmd");
        std::fs::write(&shim, PROGRAM_SHIM).unwrap();
        let args = vec!["--".to_string(), "fix it".to_string()];
        let (program, words) = program_command(&shim, &args, OsStr::new(""));
        assert_eq!(program, tmp.path().join(r"..\@anthropic-ai\claude-code\bin\claude.exe"));
        assert_eq!(words, args);
    }

    #[test]
    fn runs_shims_through_node_and_programs_as_they_are() {
        let tmp = tempfile::tempdir().unwrap();
        let shim = tmp.path().join("claude.cmd");
        std::fs::write(&shim, SHIM).unwrap();
        std::fs::write(tmp.path().join("node.exe"), "").unwrap();
        let args = vec!["--".to_string(), "fix the \"login\"\nplease".to_string()];
        let (program, words) = program_command(&shim, &args, OsStr::new(""));
        assert_eq!(program, tmp.path().join("node.exe"));
        assert_eq!(words[0], tmp.path().join(r"node_modules\@anthropic-ai\claude-code\cli.js").to_string_lossy());
        assert_eq!(&words[1..], &args[..]);
        let exe = tmp.path().join("codex.exe");
        assert_eq!(program_command(&exe, &args, OsStr::new("")), (exe.clone(), args.clone()));
        let path = std::env::join_paths([tmp.path()]).unwrap();
        assert_eq!(find_program(&path, "claude"), Some(shim));
        assert_eq!(find_program(&path, "opencode"), None);
    }
}
