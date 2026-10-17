//! Codex sessions asides pushes asides into. The session is a thread in Codex's app-server daemon,
//! which runs it whether or not a Codex window has it open; asides hands it each aside as a
//! message, so Codex never waits in a tool call. The thread is started through the daemon's
//! protocol (JSON-RPC over WebSocket, on the socket `codex app-server proxy` connects stdio to):
//! `thread/start` with the session's MCP server, `thread/name/set`, and the first prompt. Each aside
//! is a turn started the same way when Codex is idle (its screenshots attached), or queued with
//! `codex queue` while it's busy.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// How long one request to the daemon may take (starting a thread starts its MCP servers).
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Screenshots attached to one pushed message.
const MAX_IMAGES: usize = 6;

pub struct NewThread<'a> {
    pub cwd: &'a Path,
    pub name: &'a str,
    /// Configuration overrides, like `-c` (the asides MCP server for the session).
    pub config: Value,
    pub prompt: &'a str,
}

/// Starts a thread in the daemon, sends it its first prompt, and returns its id. The daemon keeps
/// running it after this disconnects.
pub fn start_thread(new: &NewThread) -> Result<String> {
    // A proxy that exits at once (no daemon) must be an error, not SIGPIPE, which the CLI otherwise
    // takes as a reason to exit quietly. The agent this process becomes gets the default back.
    let previous = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let started = talk_to_daemon(new);
    unsafe { libc::signal(libc::SIGPIPE, previous) };
    started
}

fn talk_to_daemon(new: &NewThread) -> Result<String> {
    let mut daemon = Daemon::connect()?;
    daemon.call("initialize", json!({ "clientInfo": { "name": "asides", "version": env!("CARGO_PKG_VERSION") } }))?;
    daemon.notify("initialized")?;
    let started = daemon.call("thread/start", json!({ "cwd": new.cwd, "config": new.config }))?;
    let thread = started
        .pointer("/thread/id")
        .and_then(Value::as_str)
        .context("Codex didn't say which thread it started")?
        .to_string();
    daemon.call("thread/name/set", json!({ "threadId": thread, "name": new.name }))?;
    daemon.call("turn/start", json!({ "threadId": thread, "input": [{ "type": "text", "text": new.prompt }] }))?;
    Ok(thread)
}

/// Hands `thread` an aside. Idle, Codex starts on it at once with its screenshots attached (a turn
/// started through the daemon, which a window that has the thread open shows too); busy, it's
/// queued for after the turn in progress (`codex queue`, which takes no images, so the text says
/// where they are). A thread the daemon has let go of (no window has it open) is loaded again
/// first, with `config`: its asides MCP server.
pub fn push(thread: &str, text: &str, images: &[PathBuf], config: &Value) -> Result<()> {
    let previous = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    let pushed = push_to_daemon(thread, text, images, config);
    unsafe { libc::signal(libc::SIGPIPE, previous) };
    pushed
}

fn push_to_daemon(thread: &str, text: &str, images: &[PathBuf], config: &Value) -> Result<()> {
    let images: Vec<&PathBuf> = images.iter().filter(|i| i.is_file()).take(MAX_IMAGES).collect();
    let mut daemon = Daemon::connect()?;
    daemon.call("initialize", json!({ "clientInfo": { "name": "asides", "version": env!("CARGO_PKG_VERSION") } }))?;
    daemon.notify("initialized")?;
    // The daemon lets a thread go once no client has it open (and asks for it to be loaded again).
    let status = match daemon.call("thread/read", json!({ "threadId": thread })) {
        Ok(read) => read.pointer("/thread/status/type").and_then(Value::as_str).unwrap_or_default().to_string(),
        Err(err) if format!("{err:#}").contains("not loaded") => "notLoaded".into(),
        Err(err) => return Err(err),
    };
    match status.as_str() {
        "active" => return queue(thread, &with_image_paths(text, &images)),
        "notLoaded" => drop(daemon.call("thread/resume", json!({ "threadId": thread, "config": config }))?),
        _ => {}
    }
    let mut input = vec![json!({ "type": "text", "text": text })];
    input.extend(images.iter().map(|path| json!({ "type": "localImage", "path": path })));
    daemon.call("turn/start", json!({ "threadId": thread, "input": input }))?;
    Ok(())
}

/// Queues a message for `thread`'s next turn.
fn queue(thread: &str, text: &str) -> Result<()> {
    let out = Command::new("codex")
        .args(["queue", "--thread", thread, "--message", text])
        .stdin(Stdio::null())
        .output()
        .context("could not run codex")?;
    if !out.status.success() {
        bail!("codex queue failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(())
}

/// `text`, saying which files are its screenshots, for a message that can't attach them.
fn with_image_paths(text: &str, images: &[&PathBuf]) -> String {
    if images.is_empty() {
        return text.to_string();
    }
    let paths: Vec<String> = images.iter().map(|p| format!("- {}", p.display())).collect();
    format!("{text}\n\nIts screenshots (open them with view_image):\n{}", paths.join("\n"))
}

/// A connection to the daemon: `codex app-server proxy`'s stdio, spoken as a WebSocket client.
struct Daemon {
    proxy: Child,
    writer: Arc<Mutex<ChildStdin>>,
    messages: Receiver<Value>,
    next_id: u64,
}

impl Daemon {
    /// Connects to the daemon, starting it first if it isn't running (Codex starts it on demand,
    /// but only for its own windows).
    fn connect() -> Result<Daemon> {
        Daemon::connect_once().or_else(|_| {
            let started = Command::new("codex")
                .args(["app-server", "daemon", "start"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            match started {
                Ok(status) if status.success() => Daemon::connect_once(),
                _ => bail!("Codex's app-server daemon isn't running, and didn't start"),
            }
        })
    }

    fn connect_once() -> Result<Daemon> {
        let mut proxy = Command::new("codex")
            .args(["app-server", "proxy"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("could not run `codex app-server proxy`")?;
        let mut stdin = proxy.stdin.take().context("no stdin")?;
        let mut stdout = BufReader::new(proxy.stdout.take().context("no stdout")?);
        stdin.write_all(
            b"GET / HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
              Sec-WebSocket-Key: YXNpZGVzLWNvZGV4LXNlc3Npb24=\r\nSec-WebSocket-Version: 13\r\n\r\n",
        )?;
        stdin.flush()?;
        let mut status = String::new();
        stdout.read_line(&mut status)?;
        if !status.contains(" 101 ") {
            let _ = proxy.kill();
            bail!("Codex's app-server didn't accept the connection ({})", status.trim());
        }
        loop {
            let mut header = String::new();
            if stdout.read_line(&mut header)? == 0 || header.trim().is_empty() {
                break;
            }
        }
        let writer = Arc::new(Mutex::new(stdin));
        let (sender, messages) = mpsc::channel();
        let pong = writer.clone();
        std::thread::spawn(move || {
            while let Ok(Some(text)) = read_message(&mut stdout, &pong) {
                if let Ok(message) = serde_json::from_str(&text)
                    && sender.send(message).is_err()
                {
                    return;
                }
            }
        });
        Ok(Daemon { proxy, writer, messages, next_id: 0 })
    }

    fn send(&self, message: &Value) -> Result<()> {
        let mut writer = self.writer.lock().unwrap();
        write_frame(&mut *writer, 0x1, message.to_string().as_bytes())?;
        Ok(())
    }

    fn notify(&self, method: &str) -> Result<()> {
        self.send(&json!({ "jsonrpc": "2.0", "method": method }))
    }

    /// Sends a request and waits for its reply, skipping the daemon's notifications.
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.next_id += 1;
        let id = self.next_id;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        let deadline = Instant::now() + REPLY_TIMEOUT;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            let message = self.messages.recv_timeout(left).with_context(|| format!("Codex didn't answer {method}"))?;
            if message.get("id").and_then(Value::as_u64) != Some(id) || message.get("method").is_some() {
                continue;
            }
            if let Some(error) = message.get("error") {
                bail!("Codex refused {method}: {}", error.get("message").and_then(Value::as_str).unwrap_or("error"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = write_frame(&mut *writer, 0x8, &[]);
        }
        let _ = self.proxy.kill();
        let _ = self.proxy.wait();
    }
}

/// Writes one masked frame (clients must mask what they send).
fn write_frame(out: &mut impl Write, opcode: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => frame.push(0x80 | n as u8),
        n if n < 65536 => {
            frame.push(0x80 | 126);
            frame.extend((n as u16).to_be_bytes());
        }
        n => {
            frame.push(0x80 | 127);
            frame.extend((n as u64).to_be_bytes());
        }
    }
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().subsec_nanos();
    let mask = (nanos ^ std::process::id().rotate_left(16)).to_be_bytes();
    frame.extend(mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out.write_all(&frame)?;
    out.flush()
}

/// Reads the next text message (joining fragments, answering pings); `None` when the connection
/// closes.
fn read_message(input: &mut impl Read, writer: &Mutex<ChildStdin>) -> std::io::Result<Option<String>> {
    let mut message = Vec::new();
    loop {
        let mut head = [0u8; 2];
        if input.read_exact(&mut head).is_err() {
            return Ok(None);
        }
        let len = match head[1] & 0x7f {
            126 => {
                let mut n = [0u8; 2];
                input.read_exact(&mut n)?;
                u16::from_be_bytes(n) as u64
            }
            127 => {
                let mut n = [0u8; 8];
                input.read_exact(&mut n)?;
                u64::from_be_bytes(n)
            }
            n => n as u64,
        };
        let mut mask = [0u8; 4];
        if head[1] & 0x80 != 0 {
            input.read_exact(&mut mask)?;
        }
        let mut payload = vec![0u8; len as usize];
        input.read_exact(&mut payload)?;
        if head[1] & 0x80 != 0 {
            payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i % 4]);
        }
        match head[0] & 0x0f {
            0x8 => return Ok(None),
            0x9 => {
                let mut writer = writer.lock().unwrap();
                write_frame(&mut *writer, 0xA, &payload)?;
            }
            0xA => {}
            _ => {
                message.extend(payload);
                if head[0] & 0x80 != 0 {
                    return Ok(Some(String::from_utf8_lossy(&message).into_owned()));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_are_masked_and_sized() {
        let mut out = Vec::new();
        write_frame(&mut out, 0x1, b"hi").unwrap();
        assert_eq!((out[0], out[1]), (0x81, 0x82));
        let mask = &out[2..6];
        assert_eq!([out[6] ^ mask[0], out[7] ^ mask[1]], *b"hi");
        let mut long = Vec::new();
        write_frame(&mut long, 0x1, &[b'x'; 300]).unwrap();
        assert_eq!((long[1], u16::from_be_bytes([long[2], long[3]])), (0x80 | 126, 300));
    }

    #[test]
    fn a_queued_aside_says_where_its_screenshots_are() {
        assert_eq!(with_image_paths("An aside.", &[]), "An aside.");
        let shot = PathBuf::from("/r/.asides/n/images/1.png");
        let text = with_image_paths("An aside.", &[&shot]);
        assert!(text.starts_with("An aside.\n\n") && text.ends_with("view_image):\n- /r/.asides/n/images/1.png"));
    }
}
