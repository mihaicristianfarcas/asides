//! `asides mcp`: an MCP server (JSON-RPC 2.0 over stdio), optional. asides types the user's asides
//! into the agent's chat as their own messages; this server lets an agent read them again from the
//! history (with the screenshots inline, without asking to read files outside the repo), measure
//! screenshots, and look again at what a pin was on (`look_again`: the app captures, as far as
//! `asides_core::look` lets it). Every tool is read-only (Codex asks before each call to one that
//! isn't): none changes anything of the user's.
//!
//! Nothing but JSON-RPC goes to stdout.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::Result;
use asides_core::history::{self, Aside};
use asides_core::markdown::Part;
use asides_core::measure::{Rect, Tolerance};
use asides_core::search::{self, Query};
use serde_json::{Value, json};

/// Revisions answered with themselves; any other gets the newest.
const PROTOCOL_VERSIONS: &[&str] = &["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"];
/// Images larger than this are named, not sent (clients cap a message's size).
const MAX_INLINE_IMAGE: u64 = 3_750_000;
const MAX_INLINE_IMAGES: usize = 6;

const INSTRUCTIONS: &str = "asides lets the user freeze their screen, click what's wrong (numbered pins on elements and \
    areas), measure, draw and say why; asides then types it into this chat as their message, with a screenshot of \
    what they marked. Each aside is also kept in their history (~/.asides/history). get_aside reads one again (the newest by \
    default) with its screenshots; list_asides lists recent ones, or finds those that say some words. Transcripts are verbatim speech-to-text: expect \
    misheard words, especially names of projects and libraries. measure_image measures a screenshot (distances \
    between edges, an object's bounds and padding). look_again shows what a pin was on as it is now (only the \
    window the pin was in, for some minutes after the aside): use it to check your change before saying it's done.";

pub fn run() -> Result<()> {
    let stdout = io::stdout();
    let send = |message: &Value| {
        let mut out = stdout.lock();
        let _ = writeln!(out, "{message}");
        let _ = out.flush();
    };
    for line in io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let messages = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => batch,
            Ok(message) => vec![message],
            Err(err) => {
                send(&json!({
                    "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {err}") }
                }));
                continue;
            }
        };
        for message in messages {
            if let Some(reply) = dispatch(&message) {
                send(&reply);
            }
        }
    }
    Ok(())
}

/// The answer to `message`, or `None` for a notification.
fn dispatch(message: &Value) -> Option<Value> {
    let method = message.get("method").and_then(Value::as_str).unwrap_or_default();
    let params = message.get("params").cloned().unwrap_or(Value::Null);
    let id = message.get("id").cloned()?;
    let result = match method {
        "initialize" => Ok(initialize(&params)),
        "ping" | "logging/setLevel" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "prompts/list" => Ok(json!({ "prompts": [] })),
        "tools/call" => Ok(call_tool(&params)),
        // Including `server/discover` (a newer revision Claude Code probes): "not found" makes it
        // fall back to `initialize`.
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    Some(match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
    })
}

fn initialize(params: &Value) -> Value {
    let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or_default();
    let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).unwrap_or(PROTOCOL_VERSIONS.last().unwrap());
    json!({
        "protocolVersion": version,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "asides", "title": "asides", "version": env!("CARGO_PKG_VERSION") },
        "instructions": INSTRUCTIONS,
    })
}

fn call_tool(params: &Value) -> Value {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default();
    let args = params.get("arguments").cloned().unwrap_or(Value::Null);
    let history = history::dir();
    match name {
        "list_asides" => {
            let count = args.get("count").and_then(Value::as_u64).unwrap_or(10).clamp(1, 100) as usize;
            let words = args.get("query").and_then(Value::as_str).unwrap_or_default();
            tool_text(list_text(&history, words, count), false)
        }
        "get_aside" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("last");
            match history::find(&history, id) {
                Some(aside) => aside_content(&aside),
                None => tool_text(format!("There's no aside \"{id}\" in the history (list_asides lists them)."), true),
            }
        }
        "look_again" => {
            let id = args.get("id").and_then(Value::as_str).unwrap_or("last");
            let Some(pin) = args.get("pin").and_then(Value::as_u64) else {
                return tool_text("pin is required: the pin's number in the aside.".into(), true);
            };
            match crate::look(id, pin as u32) {
                Ok((what, image)) => looked_content(&what, &image),
                Err(err) => tool_text(format!("{err:#}"), true),
            }
        }
        "measure_image" => match measure_image(&args) {
            Ok(text) => tool_text(text, false),
            Err(err) => tool_text(format!("{err:#}"), true),
        },
        _ => tool_text(format!("Unknown tool {name}"), true),
    }
}

/// The newest `count` asides in `history`, a line each; with `words`, those that say them, each
/// with the line it says them in (when its first words don't).
fn list_text(history: &Path, words: &str, count: usize) -> String {
    let query = Query::new(words);
    let asides: Vec<(Aside, Option<String>)> = match query.is_empty() {
        true => history::list(history).into_iter().take(count).map(|a| (a, None)).collect(),
        false => search::search(history, &query)
            .into_iter()
            .take(count)
            .map(|(aside, text)| {
                let excerpt = query.excerpt(&text, &aside.record.title).map(|e| e.text);
                (aside, excerpt)
            })
            .collect(),
    };
    if asides.is_empty() {
        return match query.is_empty() {
            true => "No asides yet.".into(),
            false if words.contains('"') => format!("No aside says {}.", words.trim()),
            false => format!("No aside says \"{}\".", words.trim()),
        };
    }
    asides
        .iter()
        .map(|(a, excerpt)| {
            let sent = match a.last_delivery() {
                Some(d) if d.error.is_none() => format!(" → {}", d.to),
                Some(d) => format!(" (not sent to {})", d.to),
                None => String::new(),
            };
            let shots = match a.record.images.len() {
                0 => String::new(),
                1 => ", 1 screenshot".into(),
                n => format!(", {n} screenshots"),
            };
            let found = excerpt.as_ref().map(|e| format!("\n  {e}")).unwrap_or_default();
            format!("- {} · {}{sent}{shots}: {}{found}", a.id, a.created().format("%Y-%m-%d %H:%M"), a.record.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// An aside's message, with its screenshots inline where they were made (their paths too).
fn aside_content(aside: &Aside) -> Value {
    let mut content = vec![
        json!({ "type": "text", "text": format!("Aside {} · {}", aside.id, aside.created().format("%Y-%m-%d %H:%M")) }),
    ];
    let mut images = 0;
    for part in aside.parts() {
        match part {
            Part::Text(text) => content.push(json!({ "type": "text", "text": text })),
            Part::Image(path) => {
                content.push(json!({ "type": "text", "text": format!("[Screenshot: {path}]") }));
                if images < MAX_INLINE_IMAGES
                    && let Some(image) = inline_image(Path::new(&path))
                {
                    content.push(image);
                    images += 1;
                }
            }
        }
    }
    json!({ "content": content, "isError": false })
}

/// What an agent looked at again: what it is, where the picture is kept, and the picture.
fn looked_content(what: &str, image: &Path) -> Value {
    let mut content = vec![json!({ "type": "text", "text": format!("{what}\n[Picture: {}]", image.display()) })];
    content.extend(inline_image(image));
    json!({ "content": content, "isError": false })
}

fn measure_image(args: &Value) -> Result<String> {
    use anyhow::Context;
    let path = args.get("path").and_then(Value::as_str).context("path is required")?;
    let path = std::env::current_dir()?.join(path);
    let frame = crate::measure::load(&path)?;
    let n = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).map(|v| v.max(0.0).round() as u32);
    let target = match (args.get("area"), n(args, "x"), n(args, "y")) {
        (Some(area), ..) => {
            let (Some(x), Some(y), Some(w), Some(h)) =
                (n(area, "x"), n(area, "y"), n(area, "width"), n(area, "height"))
            else {
                anyhow::bail!("area needs x, y, width and height");
            };
            crate::measure::Target::Area(Rect::new(x, y, w.max(1), h.max(1)))
        }
        (None, Some(x), Some(y)) => crate::measure::Target::Point(x, y),
        _ => anyhow::bail!("give a point (x and y) or an area"),
    };
    let tolerance = Tolerance(n(args, "tolerance").unwrap_or(Tolerance::DEFAULT.0 as u32).min(100) as u8);
    let scale = args.get("scale").and_then(Value::as_f64).filter(|s| *s > 0.0).unwrap_or(1.0);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    crate::measure::describe(&frame, &name, target, tolerance, scale)
}

fn inline_image(path: &Path) -> Option<Value> {
    let mime = match path.extension()?.to_str()?.to_ascii_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => return None,
    };
    if fs::metadata(path).ok()?.len() > MAX_INLINE_IMAGE {
        return None;
    }
    Some(json!({ "type": "image", "data": base64(&fs::read(path).ok()?), "mimeType": mime }))
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
}

fn tools() -> Value {
    let read_only = |title: &str| json!({ "title": title, "readOnlyHint": true, "openWorldHint": false });
    json!([
        {
            "name": "get_aside",
            "title": "Read an aside",
            "description": "Read one of the user's asides from their history: what they said, with the elements they \
                pinned, what they measured, and each screenshot inline where it was taken. The newest by default.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "string", "description": "Its id (list_asides), or \"last\"." } }
            },
            "annotations": read_only("Read an aside")
        },
        {
            "name": "list_asides",
            "title": "List asides",
            "description": "List the user's recent asides, newest first: id, when, where it was sent, its first words. \
                With a query, only those that say all its words (in any order, ignoring case and accents; \"in \
                quotes\" together), in what was said, the pins, where it was and where it was sent, each with the \
                line it says them in.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "count": { "type": "integer", "minimum": 1, "maximum": 100, "description": "How many (10)." },
                    "query": { "type": "string", "description": "Words to look for." }
                }
            },
            "annotations": read_only("List asides")
        },
        {
            "name": "look_again",
            "title": "Look again at a pin",
            "description": "See what a pin of the user's aside was on, as it is on their screen right now: a picture \
                of that part of the window the pin was in (found again wherever the window is), and nothing else of \
                their screen. Use it after changing what they pinned, to check the change before saying it's done, \
                and compare with the aside's screenshot (get_aside). asides allows it for some minutes after the \
                aside was sent and shows the user that you looked; when it can't look (the window is closed, the \
                aside is old), it says why.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "pin": { "type": "integer", "minimum": 1, "description": "The pin's number in the aside." },
                    "id": { "type": "string", "description": "The aside's id (list_asides), or \"last\" (the default)." }
                },
                "required": ["pin"]
            },
            "annotations": read_only("Look again at a pin")
        },
        {
            "name": "measure_image",
            "title": "Measure a screenshot",
            "description": "Measure a screenshot (PNG) like PixelSnap: from a point, the distances to the nearest \
                edges across and down; or, for an area, the bounds of the object inside it and the space around it \
                (padding), with its colors. Coordinates are the image's pixels; screenshots of Retina screens have \
                2 pixels per point (pass scale 2 to get points too). Use it to check spacing, alignment and sizes \
                in the user's screenshots or your own.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "The PNG, absolute or relative to the working directory." },
                    "x": { "type": "number", "description": "A point to measure from, in pixels." },
                    "y": { "type": "number" },
                    "area": {
                        "type": "object",
                        "description": "An area around an object, in pixels, to find the object's bounds.",
                        "properties": {
                            "x": { "type": "number" }, "y": { "type": "number" },
                            "width": { "type": "number" }, "height": { "type": "number" }
                        },
                        "required": ["x", "y", "width", "height"]
                    },
                    "tolerance": {
                        "type": "integer", "minimum": 0, "maximum": 100,
                        "description": "How much (in %) the color must change from one pixel to the next to count \
                            as an edge (gradients, soft shadows and the blur behind glass never do); raise it to look \
                            through borders and hard shadows. Default 2."
                    },
                    "scale": { "type": "number", "description": "Pixels per point, to also report points." }
                },
                "required": ["path"]
            },
            "annotations": read_only("Measure a screenshot")
        }
    ])
}

fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n =
            (chunk[0] as u32) << 16 | (*chunk.get(1).unwrap_or(&0) as u32) << 8 | *chunk.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Local, TimeZone};

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }

    #[test]
    fn answers_the_protocol_and_reads_asides_with_their_screenshots() {
        let call = |method: &str, params: Value| {
            dispatch(&json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }))
        };
        let init = call("initialize", json!({ "protocolVersion": "2025-06-18" })).unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(call("server/discover", Value::Null).unwrap()["error"]["code"], -32601);
        assert!(dispatch(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" })).is_none());
        let names: Vec<String> =
            tools().as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().into()).collect();
        assert_eq!(names, ["get_aside", "list_asides", "look_again", "measure_image"]);
        // A pin is needed; the picture comes with what it shows and where it's kept.
        let asked = call("tools/call", json!({ "name": "look_again", "arguments": {} })).unwrap();
        assert_eq!(asked["result"]["isError"], true);
        let picture = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        fs::write(picture.path(), [0x89, b'P', b'N', b'G']).unwrap();
        let looked = looked_content("Pin 2 of aside a (button \"Save\" in Safari), as it is now.", picture.path());
        let kinds: Vec<&str> =
            looked["content"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["text", "image"]);
        assert!(looked["content"][0]["text"].as_str().unwrap().contains("[Picture: "));
        assert!(tools().as_array().unwrap().iter().all(|t| t["annotations"]["readOnlyHint"] == true));

        let tmp = tempfile::tempdir().unwrap();
        let at = Local.with_ymd_and_hms(2026, 10, 18, 22, 15, 30).unwrap();
        let png = [0x89, b'P', b'N', b'G'];
        let message = "The button is too low.\n\n![Screenshot 1]({image:0})\n\n### 1. button";
        let aside = history::create(
            tmp.path(),
            at,
            message,
            &[history::Image { bytes: &png, extension: "png" }],
            Vec::new(),
            None,
        )
        .unwrap();
        let content = aside_content(&aside);
        let kinds: Vec<&str> =
            content["content"].as_array().unwrap().iter().map(|c| c["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, ["text", "text", "text", "image", "text"]);
        let row = "- 2026-10-18-221530 · 2026-10-18 22:15, 1 screenshot: The button is too low.";
        assert_eq!(list_text(tmp.path(), "", 10), row);
        // Found by what it says, with the line it says it in.
        assert_eq!(list_text(tmp.path(), "BUTTON low", 10), row);
        assert_eq!(list_text(tmp.path(), "low button", 10), row);
        history::create(tmp.path(), at, "Something else.\n\n### 1. link \"Șterge\"", &[], Vec::new(), None).unwrap();
        assert!(list_text(tmp.path(), "sterge", 10).ends_with(": Something else.\n  link \"Șterge\""));
        assert_eq!(list_text(tmp.path(), "green", 10), "No aside says \"green\".");
    }
}
