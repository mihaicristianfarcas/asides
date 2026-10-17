//! OpenCode sessions asides pushes asides into. OpenCode 2 runs every session in a shared
//! background service, which its TUI is a client of. Asides talks to it the same way (HTTP on
//! localhost, with the address and password the service registers in
//! `~/.local/state/opencode/service.json`): it creates the session in the repo (or worktree), gives
//! that directory a session-tied `asides` MCP server, and admits each aside as the session's next
//! input (`delivery: "queue"`), so OpenCode never waits in a tool call. The terminal then opens the
//! session (`opencode --session`). OpenCode's free models refuse `opencode run`, but not this.
//!
//! The server replaces the user's own `asides` server for that directory while the session runs
//! (two would offer every tool twice); when it ends, theirs is put back.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// How long one request may take (the first one in a directory loads it, and starts its MCP servers).
const TIMEOUT: Duration = Duration::from_secs(60);
/// How long a service asides starts may take to register.
const STARTING: Duration = Duration::from_secs(20);
/// Screenshots attached to one pushed message.
const MAX_IMAGES: usize = 6;
/// The MCP server's name, as users configure it.
const SERVER: &str = "asides";

pub struct NewSession<'a> {
    /// Where the session works: the repo, or its worktree.
    pub dir: &'a Path,
    pub name: &'a str,
    /// The session's `asides mcp` command line.
    pub server: Vec<String>,
    pub prompt: &'a str,
}

/// Starts a session in OpenCode's service, with its MCP server, sends it its first prompt, and
/// returns its id. The service runs it whether or not a TUI has it open.
pub fn start_session(new: &NewSession) -> Result<String> {
    let service = Service::connect()?;
    let config = json!({ "config": { "type": "local", "command": new.server } });
    service.call("PUT", &format!("/api/experimental/mcp/{SERVER}?{}", location(new.dir)), Some(&config))?;
    let created = service.call(
        "POST",
        "/api/session",
        Some(&json!({ "title": new.name, "location": { "directory": new.dir } })),
    )?;
    let id = created
        .pointer("/data/id")
        .and_then(Value::as_str)
        .context("OpenCode didn't say which session it started")?
        .to_string();
    service.prompt(&id, new.prompt, &[])?;
    Ok(id)
}

/// Hands `session` a message, with screenshots: OpenCode runs it after what it's doing.
pub fn prompt(session: &str, text: &str, images: &[PathBuf]) -> Result<()> {
    Service::running().context("OpenCode's background service isn't running")?.prompt(session, text, images)
}

/// Puts the user's own `asides` server back where `session` works (or removes the session's, when
/// they have none), once asides stops pushing into it.
pub fn release(session: &str) -> Result<()> {
    let service = Service::running().context("OpenCode's background service isn't running")?;
    let info = service.call("GET", &format!("/api/session/{session}"), None)?;
    let dir = info.pointer("/data/location/directory").and_then(Value::as_str).context("no session directory")?;
    let path = format!("/api/experimental/mcp/{SERVER}?{}", location(Path::new(dir)));
    let configured = service.call("GET", &format!("/api/config?{}", location(Path::new(dir))), None)?;
    match configured_server(&configured) {
        Some(config) => service.call("PUT", &path, Some(&json!({ "config": config }))),
        None => service.call("DELETE", &path, None),
    }
    .map(drop)
}

/// The `asides` server the user configured, from `GET /api/config`'s documents (lowest priority
/// first): `mcp.servers.asides`, or `mcp.asides` as OpenCode 1 kept it.
fn configured_server(documents: &Value) -> Option<Value> {
    let documents = documents.get("data").unwrap_or(documents).as_array()?;
    documents.iter().rev().find_map(|doc| {
        let mcp = doc.pointer("/info/mcp")?;
        mcp.pointer(&format!("/servers/{SERVER}")).or_else(|| mcp.get(SERVER)).filter(|s| s.is_object()).cloned()
    })
}

/// The `location` query OpenCode's API takes (a deep object): `location[directory]=<dir>`.
fn location(dir: &Path) -> String {
    format!("location%5Bdirectory%5D={}", percent_encode(&dir.to_string_lossy()))
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// OpenCode's background service, as its registration gives it.
struct Service {
    /// `host:port`
    address: String,
    /// The `Authorization` header, when the service has a password.
    auth: Option<String>,
}

impl Service {
    /// The running service, or one started now (`opencode serve --service`, as its TUI does).
    fn connect() -> Result<Service> {
        if let Some(service) = Service::running() {
            return Ok(service);
        }
        let mut child = Command::new("opencode")
            .args(["serve", "--service"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .context("could not start OpenCode's background service")?;
        let started = Instant::now();
        while started.elapsed() < STARTING {
            if let Some(service) = Service::running() {
                return Ok(service);
            }
            // It exits at once when it can't serve (or another one registered first).
            if child.try_wait().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Service::running().context("OpenCode's background service didn't start")
    }

    /// The registered service, if it answers.
    fn running() -> Option<Service> {
        let registration: Value = serde_json::from_str(&std::fs::read_to_string(registration()).ok()?).ok()?;
        let address = registration["url"].as_str()?.strip_prefix("http://")?.trim_end_matches('/').to_string();
        let auth = registration["password"].as_str().map(|p| format!("Basic {}", base64(&format!("opencode:{p}"))));
        let service = Service { address, auth };
        service.call("GET", "/api/session?limit=1", None).ok()?;
        Some(service)
    }

    fn prompt(&self, session: &str, text: &str, images: &[PathBuf]) -> Result<()> {
        let files: Vec<Value> = images
            .iter()
            .filter(|i| i.is_file())
            .take(MAX_IMAGES)
            .map(|i| json!({ "uri": format!("file://{}", i.display()), "name": i.file_name().map(|n| n.to_string_lossy()) }))
            .collect();
        let mut body = json!({ "text": text, "delivery": "queue" });
        if !files.is_empty() {
            body["files"] = Value::from(files);
        }
        self.call("POST", &format!("/api/session/{session}/prompt"), Some(&body)).map(drop)
    }

    /// Sends a request and returns the JSON it answers with, or what went wrong.
    fn call(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value> {
        let (status, answer) = self.request(method, path, body)?;
        if !(200..300).contains(&status) {
            let message = answer.get("message").and_then(Value::as_str).map(str::to_string);
            bail!("OpenCode answered {method} {path} with {status}: {}", message.unwrap_or_else(|| answer.to_string()));
        }
        Ok(answer)
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>) -> Result<(u16, Value)> {
        let mut stream = TcpStream::connect(&self.address).context("could not reach OpenCode's background service")?;
        stream.set_read_timeout(Some(TIMEOUT))?;
        stream.set_write_timeout(Some(TIMEOUT))?;
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: {}\r\n",
            self.address,
            body.len()
        );
        if !body.is_empty() {
            head.push_str("Content-Type: application/json\r\n");
        }
        if let Some(auth) = &self.auth {
            head.push_str(&format!("Authorization: {auth}\r\n"));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes())?;
        stream.write_all(body.as_bytes())?;
        read_response(&mut stream)
    }
}

/// Where the service registers: `$XDG_STATE_HOME/opencode/service.json`, by default under
/// `~/.local/state`.
fn registration() -> PathBuf {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(".local/state"));
    state.join("opencode/service.json")
}

/// Reads an HTTP/1.1 response: its status and its JSON (a string when it isn't JSON, null when
/// empty). The body ends where its length says, its last chunk, or the connection.
fn read_response(input: &mut impl Read) -> Result<(u16, Value)> {
    let mut raw = Vec::new();
    let mut buf = [0u8; 16384];
    let head_end = loop {
        if let Some(at) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
            break at;
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            bail!("OpenCode's background service closed the connection");
        }
        raw.extend_from_slice(&buf[..n]);
    };
    let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
    let status = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).context("not an HTTP response")?;
    let header = |name: &str| {
        head.lines().find_map(|l| l.strip_prefix(name).and_then(|v| v.strip_prefix(':')).map(|v| v.trim().to_string()))
    };
    let length: Option<usize> = header("content-length").and_then(|v| v.parse().ok());
    let chunked = header("transfer-encoding").is_some_and(|v| v.contains("chunked"));
    let mut body = raw[head_end + 4..].to_vec();
    loop {
        let done = match (length, chunked) {
            (Some(n), _) => body.len() >= n,
            (None, true) => unchunk(&body).is_some(),
            (None, false) => false,
        };
        if done {
            break;
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    if let Some(n) = length {
        body.truncate(n);
    } else if chunked {
        body = unchunk(&body).context("the response ended mid-chunk")?;
    }
    let answer = if body.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(&body).unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned()))
    };
    Ok((status, answer))
}

/// A chunked body's content, once its last chunk has arrived.
fn unchunk(mut raw: &[u8]) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    loop {
        let line_end = raw.windows(2).position(|w| w == b"\r\n")?;
        let size = std::str::from_utf8(&raw[..line_end]).ok()?.split(';').next()?.trim();
        let size = usize::from_str_radix(size, 16).ok()?;
        let start = line_end + 2;
        if size == 0 {
            return Some(body);
        }
        if raw.len() < start + size + 2 {
            return None;
        }
        body.extend_from_slice(&raw[start..start + size]);
        raw = &raw[start + size + 2..];
    }
}

fn base64(text: &str) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in text.as_bytes().chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            out.push(if i <= chunk.len() { ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_what_the_api_takes() {
        assert_eq!(base64("opencode:pw"), "b3BlbmNvZGU6cHc=");
        assert_eq!(base64("ab"), "YWI=");
        assert_eq!(base64("abc"), "YWJj");
        assert_eq!(location(Path::new("/Users/sam/my shop")), "location%5Bdirectory%5D=%2FUsers%2Fsam%2Fmy%20shop");
    }

    #[test]
    fn reads_responses() {
        let plain = b"HTTP/1.1 200 OK\r\nContent-Length: 13\r\n\r\n{\"data\":true}".to_vec();
        assert_eq!(read_response(&mut plain.as_slice()).unwrap(), (200, json!({ "data": true })));
        let chunked =
            b"HTTP/1.1 404 Not Found\r\nTransfer-Encoding: chunked\r\n\r\n4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        assert_eq!(read_response(&mut chunked.as_slice()).unwrap(), (404, json!({ "a": 1 })));
        let empty = b"HTTP/1.1 204 No Content\r\n\r\n".to_vec();
        assert_eq!(read_response(&mut empty.as_slice()).unwrap(), (204, Value::Null));
    }

    #[test]
    fn finds_the_users_server() {
        let documents = json!([
            { "type": "document", "info": { "mcp": { "servers": { "asides": { "type": "local", "command": ["asides", "mcp"] } } } } },
            { "type": "document", "info": { "mcp": { "servers": { "other": {} } } } },
            { "type": "directory" }
        ]);
        assert_eq!(configured_server(&documents), Some(json!({ "type": "local", "command": ["asides", "mcp"] })));
        let old = json!({ "data": [{ "info": { "mcp": { "asides": { "type": "local", "command": ["x"] } } } }] });
        assert_eq!(configured_server(&old), Some(json!({ "type": "local", "command": ["x"] })));
        assert_eq!(configured_server(&json!([{ "type": "directory" }])), None);
    }
}
