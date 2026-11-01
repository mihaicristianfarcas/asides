//! The command's Windows parts: what the app runs it for, and how it reaches the app.
//!
//! - `asides type-into <pid>`: types what's on its standard input into the console of process
//!   `pid` (an agent's session), as keys pressed there (`WriteConsoleInput`), whichever app shows
//!   that console. The app runs it without a console of its own: a process can attach to one
//!   console at a time, and a console that closes while it's attached ends it.
//! - `asides launch <file>`: runs in a new terminal, starts the agent the file names in its folder
//!   with the aside as its first message, and waits for it (`asides_core::launch`).
//! - Links go to the app by starting it with them (`asides-app.exe asides://…`): the copy running
//!   takes them from the one started (`asides-app`'s `instance.rs`).

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use asides_core::link::Link;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, STILL_ACTIVE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, INPUT_RECORD, INPUT_RECORD_0, KEY_EVENT, KEY_EVENT_RECORD, KEY_EVENT_RECORD_0,
    SetConsoleCtrlHandler, WriteConsoleInputW,
};
use windows::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MAPVK_VK_TO_VSC, MapVirtualKeyW, VK_BACK, VK_ESCAPE, VK_PACKET, VK_RETURN, VK_TAB, VkKeyScanW,
};
use windows::core::{BOOL, HSTRING};

/// How many key records go to the console at once: a burst, which agents take for a paste.
const CHUNK: usize = 1024;

/// Whether process `pid` is running.
pub fn is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: the handle is closed below.
    unsafe {
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut code = 0u32;
        let alive = GetExitCodeProcess(process, &mut code).is_ok() && code == STILL_ACTIVE.0 as u32;
        let _ = CloseHandle(process);
        alive
    }
}

/// `asides-app.exe`, beside this command (`ASIDES_APP` names another, for development).
fn app() -> PathBuf {
    if let Some(app) = std::env::var_os("ASIDES_APP") {
        return PathBuf::from(app);
    }
    std::env::current_exe().unwrap_or_default().with_file_name("asides-app.exe")
}

/// Hands the link to the app: starts it with the link, which the copy running takes (or, if none
/// runs, it starts with it).
pub fn send(link: &Link) -> Result<()> {
    let app = app();
    if !app.is_file() {
        bail!("could not find asides ({}). Is it installed?", app.display());
    }
    std::process::Command::new(&app)
        .arg(link.to_url())
        .spawn()
        .with_context(|| format!("could not start {}", app.display()))?;
    Ok(())
}

/// A key pressed and let go, typing `unit` (a UTF-16 code unit): Return, Tab, Esc and Backspace
/// as those keys, anything else as the character it types.
fn key_records(unit: u16) -> [INPUT_RECORD; 2] {
    let special = match unit {
        0x0d => Some(VK_RETURN),
        0x09 => Some(VK_TAB),
        0x1b => Some(VK_ESCAPE),
        0x08 => Some(VK_BACK),
        _ => None,
    };
    // SAFETY: plain lookups in the keyboard layout. A character it has no key for is typed as
    // Windows types any character (an emoji, ă on an English keyboard): `VK_PACKET`.
    let key = special.map(|vk| vk.0).unwrap_or_else(|| match unsafe { VkKeyScanW(unit) } {
        -1 => VK_PACKET.0,
        found => (found & 0xff) as u16,
    });
    let scan = match key {
        0 => 0,
        k if k == VK_PACKET.0 => 0,
        k => (unsafe { MapVirtualKeyW(k as u32, MAPVK_VK_TO_VSC) }) as u16,
    };
    let record = |down: bool| INPUT_RECORD {
        EventType: KEY_EVENT as u16,
        Event: INPUT_RECORD_0 {
            KeyEvent: KEY_EVENT_RECORD {
                bKeyDown: down.into(),
                wRepeatCount: 1,
                wVirtualKeyCode: key,
                wVirtualScanCode: scan,
                uChar: KEY_EVENT_RECORD_0 { UnicodeChar: unit },
                dwControlKeyState: 0,
            },
        },
    };
    [record(true), record(false)]
}

/// Ctrl+C in the console attached to is the user's, for the agent: not this command's.
unsafe extern "system" fn ignore_ctrl(_: u32) -> BOOL {
    true.into()
}

/// `asides type-into <pid>`: what's on standard input, typed into process `pid`'s console.
pub fn type_into(pid: u32) -> Result<()> {
    let mut text = String::new();
    std::io::stdin().read_to_string(&mut text)?;
    let records: Vec<INPUT_RECORD> = text.encode_utf16().flat_map(key_records).collect();
    // SAFETY: this process leaves any console it has, attaches to the session's, writes key records
    // into its input (`CONIN$`, the input of the console attached to), and leaves it.
    unsafe {
        let _ = FreeConsole();
        let _ = SetConsoleCtrlHandler(Some(ignore_ctrl), true);
        AttachConsole(pid).map_err(|err| anyhow::anyhow!("its console isn't there ({err})"))?;
        let input = CreateFileW(
            &HSTRING::from("CONIN$"),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        );
        let written = input
            .as_ref()
            .map_err(|err| anyhow::anyhow!("couldn't open its console's input ({err})"))
            .and_then(|&input| {
                for chunk in records.chunks(CHUNK) {
                    let mut done = 0u32;
                    WriteConsoleInputW(input, chunk, &mut done)
                        .map_err(|err| anyhow::anyhow!("couldn't type into its console ({err})"))?;
                }
                Ok(())
            });
        if let Ok(input) = input {
            let _ = CloseHandle(input);
        }
        let _ = FreeConsole();
        written
    }
}

/// `asides launch <file>`: the agent the file names, started in its folder with its first message,
/// in this console. This command ends once it's started: the console (a Windows Terminal tab, or a
/// console window) stays while the agent uses it, and asides.exe isn't held open for the session's
/// life (an update could replace it).
pub fn launch(file: &Path) -> Result<()> {
    let text = std::fs::read_to_string(file).with_context(|| format!("could not read {}", file.display()))?;
    let _ = std::fs::remove_file(file);
    let spec: serde_json::Value = serde_json::from_str(&text)?;
    let words: Vec<String> =
        spec["words"].as_array().into_iter().flatten().filter_map(|w| w.as_str().map(String::from)).collect();
    let dir = PathBuf::from(spec["dir"].as_str().unwrap_or_default());
    let launch = spec["launch"].as_str().unwrap_or_default().to_string();
    let Some((name, args)) = words.split_first() else { bail!("nothing to start") };
    let path = asides_core::launch::user_path();
    let found = asides_core::launch::find_program(&path, name)
        .with_context(|| format!("{name} isn't installed (it isn't on your PATH)"))?;
    let (program, args) = asides_core::launch::program_command(&found, args, &path);
    std::process::Command::new(&program)
        .args(&args)
        .current_dir(&dir)
        .env(asides_core::agents::LAUNCH_VAR, launch)
        .spawn()
        .with_context(|| format!("could not start {}", program.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn types_return_tab_and_characters_as_keys() {
        // SAFETY: reading the union's key event, which `key_records` writes.
        let key = |record: &INPUT_RECORD| unsafe { record.Event.KeyEvent };
        let [down, up] = key_records(0x0d);
        assert_eq!((key(&down).wVirtualKeyCode, unsafe { key(&down).uChar.UnicodeChar }), (VK_RETURN.0, 0x0d));
        assert!(key(&down).bKeyDown.as_bool() && !key(&up).bKeyDown.as_bool());
        let [a, _] = key_records('é' as u16);
        assert_eq!(unsafe { key(&a).uChar.UnicodeChar }, 'é' as u16);
    }
}
