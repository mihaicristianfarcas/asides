//! `asides://` links: how the CLI and the asides window talk to the running menu-bar app (via
//! `open`).

use std::path::PathBuf;

use url::Url;

use crate::agents::AgentKind;
use crate::paths::URL_SCHEME;
use crate::state::Permission;
use crate::update::Step;

#[derive(Debug, Clone, PartialEq)]
pub enum Link {
    /// Opens asides frozen, as `⌃⌥Space` does; while it's open, sends the aside.
    Open {
        request: Option<String>,
    },
    /// Opens the asides window (History, Sessions, Settings).
    Window {
        request: Option<String>,
    },
    /// Sends aside `id` from the history to session `to` (a session's id), or to the default.
    Send {
        id: String,
        to: Option<String>,
        request: Option<String>,
    },
    /// Starts a new `agent` session in `dir` with aside `id` as its first message.
    NewSession {
        id: String,
        agent: AgentKind,
        dir: PathBuf,
        request: Option<String>,
    },
    /// Downloads speech model `model` (`models::MODELS`).
    Download {
        model: String,
        request: Option<String>,
    },
    /// asides' own update: look for one, install it (restarting), or what was shown, dismissed.
    Update {
        step: Step,
        request: Option<String>,
    },
    Quit,
    /// Asks macOS for a permission (setup's buttons): its prompt, or its settings.
    Permission {
        name: Permission,
        request: Option<String>,
    },
    /// How far setup got (`step`, a page of it), or `done` (finished, or skipped).
    Setup {
        step: String,
        request: Option<String>,
    },
    /// Opens asides at login, or stops (General's and setup's switch).
    Login {
        open: bool,
        request: Option<String>,
    },
    /// Looks for the agent sessions running again (the asides window coming forward).
    Sessions {
        request: Option<String>,
    },
    /// Development hook: process `wav` as if it had just been recorded (outside asides, an aside of
    /// its own, sent to session `to`, or kept in the history only).
    Simulate {
        wav: PathBuf,
        image: Option<PathBuf>,
        image_at: f32,
        /// Said about this annotation of the open asides session (its key).
        annotation: Option<u32>,
        to: Option<String>,
        request: Option<String>,
    },
    /// Development hook: runs an asides session on `image` (instead of the screen), driven by
    /// `script` (a JSON file of actions), and sends the result to session `to` (or keeps it in the
    /// history only), as the user would.
    SimulateMeasure {
        image: PathBuf,
        script: PathBuf,
        to: Option<String>,
        request: Option<String>,
    },
}

impl Link {
    pub fn to_url(&self) -> String {
        let mut url = Url::parse(&format!("{URL_SCHEME}://{}", self.action())).expect("valid base URL");
        {
            let mut q = url.query_pairs_mut();
            match self {
                Link::Open { .. } | Link::Window { .. } | Link::Sessions { .. } | Link::Quit => {}
                Link::Send { id, to, .. } => {
                    q.append_pair("id", id);
                    if let Some(to) = to {
                        q.append_pair("to", to);
                    }
                }
                Link::NewSession { id, agent, dir, .. } => {
                    q.append_pair("id", id);
                    q.append_pair("agent", agent.program());
                    q.append_pair("dir", &dir.to_string_lossy());
                }
                Link::Download { model, .. } => {
                    q.append_pair("model", model);
                }
                Link::Update { step, .. } => {
                    q.append_pair("step", step.as_str());
                }
                Link::Permission { name, .. } => {
                    q.append_pair("name", &name.as_str());
                }
                Link::Setup { step, .. } => {
                    q.append_pair("step", step);
                }
                Link::Login { open, .. } => {
                    q.append_pair("open", if *open { "true" } else { "false" });
                }
                Link::Simulate { wav, image, image_at, annotation, to, .. } => {
                    q.append_pair("wav", &wav.to_string_lossy());
                    if let Some(to) = to {
                        q.append_pair("to", to);
                    }
                    if let Some(annotation) = annotation {
                        q.append_pair("annotation", &annotation.to_string());
                    }
                    if let Some(image) = image {
                        q.append_pair("image", &image.to_string_lossy());
                        q.append_pair("at", &image_at.to_string());
                    }
                }
                Link::SimulateMeasure { image, script, to, .. } => {
                    q.append_pair("image", &image.to_string_lossy());
                    q.append_pair("script", &script.to_string_lossy());
                    if let Some(to) = to {
                        q.append_pair("to", to);
                    }
                }
            }
            if let Some(request) = self.request() {
                q.append_pair("req", request);
            }
        }
        url.to_string().trim_end_matches('?').to_string()
    }

    pub fn parse(url: &Url) -> Option<Link> {
        if url.scheme() != URL_SCHEME {
            return None;
        }
        let get = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
        let request = get("req");
        let nonempty = |key: &str| get(key).filter(|v| !v.trim().is_empty());
        match url.host_str()? {
            "open" => Some(Link::Open { request }),
            "window" => Some(Link::Window { request }),
            "send" => Some(Link::Send { id: nonempty("id")?, to: nonempty("to"), request }),
            "new-session" => Some(Link::NewSession {
                id: nonempty("id")?,
                agent: AgentKind::parse(&get("agent")?)?,
                dir: nonempty("dir")?.into(),
                request,
            }),
            "download" => Some(Link::Download { model: nonempty("model")?, request }),
            "update" => Some(Link::Update { step: Step::parse(&get("step")?)?, request }),
            "quit" => Some(Link::Quit),
            "permission" => Some(Link::Permission { name: Permission::parse(&get("name")?)?, request }),
            "setup" => Some(Link::Setup { step: nonempty("step")?, request }),
            "login" => Some(Link::Login { open: get("open")?.parse().ok()?, request }),
            "sessions" => Some(Link::Sessions { request }),
            "simulate" => Some(Link::Simulate {
                wav: get("wav")?.into(),
                image: get("image").map(PathBuf::from),
                image_at: get("at").and_then(|v| v.parse().ok()).unwrap_or(0.0),
                annotation: get("annotation").and_then(|v| v.parse().ok()),
                to: nonempty("to"),
                request,
            }),
            "simulate-measure" => Some(Link::SimulateMeasure {
                image: get("image")?.into(),
                script: get("script")?.into(),
                to: nonempty("to"),
                request,
            }),
            _ => None,
        }
    }

    fn action(&self) -> &'static str {
        match self {
            Link::Open { .. } => "open",
            Link::Window { .. } => "window",
            Link::Send { .. } => "send",
            Link::NewSession { .. } => "new-session",
            Link::Download { .. } => "download",
            Link::Update { .. } => "update",
            Link::Quit => "quit",
            Link::Permission { .. } => "permission",
            Link::Setup { .. } => "setup",
            Link::Login { .. } => "login",
            Link::Sessions { .. } => "sessions",
            Link::Simulate { .. } => "simulate",
            Link::SimulateMeasure { .. } => "simulate-measure",
        }
    }

    /// The request id the app echoes in `state.json` once it handled the link.
    pub fn request(&self) -> Option<&String> {
        match self {
            Link::Open { request }
            | Link::Window { request }
            | Link::Send { request, .. }
            | Link::NewSession { request, .. }
            | Link::Download { request, .. }
            | Link::Update { request, .. }
            | Link::Permission { request, .. }
            | Link::Setup { request, .. }
            | Link::Login { request, .. }
            | Link::Sessions { request }
            | Link::Simulate { request, .. }
            | Link::SimulateMeasure { request, .. } => request.as_ref(),
            Link::Quit => None,
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
        round_trip(Link::Open { request: Some("r1".into()) });
        round_trip(Link::Window { request: None });
        round_trip(Link::Send { id: "2026-10-18-221530".into(), to: Some("95393".into()), request: Some("r2".into()) });
        round_trip(Link::Send { id: "last".into(), to: None, request: None });
        round_trip(Link::NewSession {
            id: "a".into(),
            agent: AgentKind::Codex,
            dir: "/Users/me/My Code/shop & co".into(),
            request: None,
        });
        round_trip(Link::Download { model: "small.en".into(), request: None });
        for step in Step::ALL {
            round_trip(Link::Update { step, request: Some("r3".into()) });
        }
        round_trip(Link::Quit);
        round_trip(Link::Permission { name: Permission::Automation("Ghostty".into()), request: None });
        round_trip(Link::Permission { name: Permission::Screen, request: Some("r4".into()) });
        round_trip(Link::Setup { step: "voice".into(), request: None });
        round_trip(Link::Login { open: true, request: None });
        round_trip(Link::Login { open: false, request: Some("r6".into()) });
        round_trip(Link::Sessions { request: Some("r5".into()) });
        round_trip(Link::Simulate {
            wav: "/tmp/a b.wav".into(),
            image: Some("/tmp/i.png".into()),
            image_at: 2.5,
            annotation: Some(3),
            to: Some("4242".into()),
            request: None,
        });
        round_trip(Link::SimulateMeasure {
            image: "/tmp/shot 1.png".into(),
            script: "/tmp/script.json".into(),
            to: None,
            request: None,
        });
        assert_eq!(Link::Quit.to_url(), "asides://quit");
    }

    #[test]
    fn rejects_foreign_links() {
        assert_eq!(Link::parse(&Url::parse("https://open").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://nope").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://send").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://send?id=").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://update?step=reboot").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://login").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://login?open=yes").unwrap()), None);
        assert_eq!(Link::parse(&Url::parse("asides://new-session?id=a&agent=cursor&dir=/x").unwrap()), None);
    }
}
