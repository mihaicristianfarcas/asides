//! `asides://` links: how the CLI talks to the running menu-bar app (via `open`).

use std::path::PathBuf;

use url::Url;

use crate::paths::URL_SCHEME;

#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// Bind the app to `dir`. With `new_note`, the next recording starts a new note.
    Bind {
        dir: PathBuf,
        new_note: bool,
        title: Option<String>,
        request: Option<String>,
    },
    /// Make the session `id` (`sessions::Session`) the target: the user's asides go to it, and the
    /// app binds to its repo. Sent when a session is launched or starts listening.
    Session {
        id: String,
        request: Option<String>,
    },
    Quit,
    /// Send to Agent: hands the entries recorded so far to agents listening in batch mode.
    Send {
        request: Option<String>,
    },
    /// Development hook: process `wav` as if it had just been recorded.
    Simulate {
        wav: PathBuf,
        new_note: bool,
        image: Option<PathBuf>,
        image_at: f32,
        request: Option<String>,
    },
    /// Opens Measure & Annotate, as the shortcut does.
    Measure {
        request: Option<String>,
    },
    /// Development hook: runs a Measure & Annotate session on `image` (instead of the screen),
    /// driven by `script` (a JSON file of actions), and adds the result like the user would.
    SimulateMeasure {
        image: PathBuf,
        script: PathBuf,
        request: Option<String>,
    },
}

impl Link {
    pub fn to_url(&self) -> String {
        let mut url = Url::parse(&format!("{URL_SCHEME}://{}", self.action())).expect("valid base URL");
        {
            let mut q = url.query_pairs_mut();
            match self {
                Link::Bind { dir, new_note, title, request } => {
                    q.append_pair("dir", &dir.to_string_lossy());
                    if *new_note {
                        q.append_pair("new", "1");
                    }
                    if let Some(title) = title {
                        q.append_pair("title", title);
                    }
                    if let Some(request) = request {
                        q.append_pair("req", request);
                    }
                }
                Link::Session { id, request } => {
                    q.append_pair("id", id);
                    if let Some(request) = request {
                        q.append_pair("req", request);
                    }
                }
                Link::Quit => {}
                Link::Send { request } => {
                    if let Some(request) = request {
                        q.append_pair("req", request);
                    }
                }
                Link::Simulate { wav, new_note, image, image_at, request } => {
                    q.append_pair("wav", &wav.to_string_lossy());
                    if *new_note {
                        q.append_pair("new", "1");
                    }
                    if let Some(image) = image {
                        q.append_pair("image", &image.to_string_lossy());
                        q.append_pair("at", &image_at.to_string());
                    }
                    if let Some(request) = request {
                        q.append_pair("req", request);
                    }
                }
                Link::Measure { request } => {
                    if let Some(request) = request {
                        q.append_pair("req", request);
                    }
                }
                Link::SimulateMeasure { image, script, request } => {
                    q.append_pair("image", &image.to_string_lossy());
                    q.append_pair("script", &script.to_string_lossy());
                    if let Some(request) = request {
                        q.append_pair("req", request);
                    }
                }
            }
        }
        url.to_string().trim_end_matches('?').to_string()
    }

    pub fn parse(url: &Url) -> Option<Link> {
        if url.scheme() != URL_SCHEME {
            return None;
        }
        let get = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
        let flag = |key: &str| get(key).is_some_and(|v| v == "1" || v == "true");
        match url.host_str()? {
            "bind" => Some(Link::Bind {
                dir: get("dir")?.into(),
                new_note: flag("new"),
                title: get("title").filter(|t| !t.trim().is_empty()),
                request: get("req"),
            }),
            "session" => Some(Link::Session { id: get("id").filter(|id| !id.is_empty())?, request: get("req") }),
            "quit" => Some(Link::Quit),
            "send" => Some(Link::Send { request: get("req") }),
            "simulate" => Some(Link::Simulate {
                wav: get("wav")?.into(),
                new_note: flag("new"),
                image: get("image").map(PathBuf::from),
                image_at: get("at").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                request: get("req"),
            }),
            "measure" => Some(Link::Measure { request: get("req") }),
            "simulate-measure" => Some(Link::SimulateMeasure {
                image: get("image")?.into(),
                script: get("script")?.into(),
                request: get("req"),
            }),
            _ => None,
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Link::Bind { .. } => "bind",
            Link::Session { .. } => "session",
            Link::Quit => "quit",
            Link::Send { .. } => "send",
            Link::Simulate { .. } => "simulate",
            Link::Measure { .. } => "measure",
            Link::SimulateMeasure { .. } => "simulate-measure",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(link: Link) {
        let url = link.to_url();
        assert_eq!(Link::parse(&Url::parse(&url).unwrap()), Some(link), "{url}");
    }

    #[test]
    fn links_round_trip() {
        round_trip(Link::Bind {
            dir: "/Users/me/My Code/shop & co".into(),
            new_note: true,
            title: Some("Pagina de setări: bug #3".into()),
            request: Some("r1".into()),
        });
        round_trip(Link::Bind { dir: "/x".into(), new_note: false, title: None, request: None });
        round_trip(Link::Session { id: "a1b2c3d4".into(), request: Some("r4".into()) });
        round_trip(Link::Quit);
        round_trip(Link::Send { request: Some("r2".into()) });
        round_trip(Link::Send { request: None });
        round_trip(Link::Simulate {
            wav: "/tmp/a b.wav".into(),
            new_note: false,
            image: Some("/tmp/i.png".into()),
            image_at: 2.5,
            request: None,
        });
        round_trip(Link::Measure { request: Some("r3".into()) });
        round_trip(Link::SimulateMeasure {
            image: "/tmp/shot 1.png".into(),
            script: "/tmp/script.json".into(),
            request: None,
        });
        assert_eq!(Link::Quit.to_url(), "asides://quit");
    }

    #[test]
    fn rejects_foreign_links() {
        assert_eq!(Link::parse(&Url::parse("https://bind?dir=/x").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://nope").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://bind").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://session?id=").unwrap()), None);
    }
}
