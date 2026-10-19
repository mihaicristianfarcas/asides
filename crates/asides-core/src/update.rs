//! Updating asides: the feed every release publishes (`appcast.json`, an asset of the GitHub
//! release), which release to offer, what the app tells the asides window about it (`state.json`'s
//! `update`), and the release notes What's New shows. Downloading, checking the signature and
//! swapping the bundle are the app's (`asides-app`'s `update.rs`).
//!
//! ```json
//! {"releases": [{
//!   "version": "0.8.0",
//!   "url": "https://github.com/mihaicristianfarcas/asides/releases/download/v0.8.0/asides-0.8.0.zip",
//!   "size": 8656001,
//!   "sha256": "2e106c04…",
//!   "minimum_system_version": "14.0",
//!   "published": "2026-10-20",
//!   "notes": "**asides updates itself.** …"
//! }]}
//! ```

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The newest release's feed: every release carries one, and `latest` is always the newest.
pub const FEED_URL: &str = "https://github.com/mihaicristianfarcas/asides/releases/latest/download/appcast.json";
/// Where every download must come from (GitHub then redirects to its file servers).
pub const RELEASES_HOST: &str = "github.com";
/// Development: another feed (a staging one) instead.
pub const FEED_ENV: &str = "ASIDES_UPDATE_FEED";
/// Development: lets a copy that isn't notarized (`make install`'s) update itself, to try it.
pub const UNNOTARIZED_ENV: &str = "ASIDES_UPDATE_UNNOTARIZED";
/// The first check after launch, and how often after that (with automatic updates on).
pub const FIRST_CHECK: Duration = Duration::from_secs(20);
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// The largest download accepted (a release is about 9 MB), and the largest feed.
pub const MAX_SIZE: u64 = 256 * 1024 * 1024;
pub const MAX_FEED: u64 = 1024 * 1024;

/// A version, `major.minor.patch` (and macOS's, `15.5`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// `0.8.0`, `v0.8.0`, `15.5` or `26`; `None` for anything else (`0.8.0-beta`, `latest`), so a
    /// bad row in the feed is skipped rather than read as 0.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let parts: Vec<&str> = text.strip_prefix('v').unwrap_or(text).split('.').collect();
        if parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
            return None;
        }
        let number = |i: usize| parts.get(i).map_or(Some(0), |p| p.parse().ok());
        Some(Version(number(0)?, number(1)?, number(2)?))
    }

    /// This copy's.
    pub fn current() -> Version {
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_default()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// One release in the feed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    pub version: String,
    /// A zip of the notarized, stapled asides.app.
    pub url: String,
    pub size: u64,
    pub sha256: String,
    /// The oldest macOS it runs on.
    pub minimum_system_version: Option<String>,
    /// When it came out (`2026-10-20`).
    pub published: Option<String>,
    /// What's new in it, in Markdown (the GitHub release's notes).
    pub notes: String,
}

impl Release {
    /// It can be downloaded and checked: from GitHub, of a known size and SHA-256.
    fn complete(&self) -> bool {
        valid_url(&self.url)
            && (1..=MAX_SIZE).contains(&self.size)
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    }

    fn runs_on(&self, system: Version) -> bool {
        self.minimum_system_version.as_deref().and_then(Version::parse).is_none_or(|oldest| oldest <= system)
    }

    pub fn notes(&self) -> Notes {
        Notes { version: self.version.clone(), published: self.published.clone(), notes: self.notes.clone() }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Feed {
    pub releases: Vec<Release>,
}

impl Feed {
    pub fn parse(json: &str) -> Result<Feed, String> {
        serde_json::from_str(json).map_err(|err| format!("the update feed is malformed ({err})"))
    }

    /// The newest release worth offering: newer than `current` (never older, so a tampered feed
    /// can't take anyone back), for this macOS, complete. A row that doesn't parse is skipped.
    pub fn newest(&self, current: Version, system: Version) -> Option<&Release> {
        self.releases
            .iter()
            .filter_map(|r| Some((Version::parse(&r.version)?, r)))
            .filter(|(version, r)| *version > current && r.runs_on(system) && r.complete())
            .max_by_key(|(version, _)| *version)
            .map(|(_, r)| r)
    }

    /// What's new up to `current`: the notes of the releases it includes, newest first.
    pub fn notes_up_to(&self, current: Version) -> Vec<Notes> {
        let mut found: Vec<(Version, Notes)> = self
            .releases
            .iter()
            .filter(|r| !r.notes.trim().is_empty())
            .filter_map(|r| Some((Version::parse(&r.version)?, r.notes())))
            .filter(|(version, _)| *version <= current)
            .collect();
        found.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
        found.dedup_by_key(|(version, _)| *version);
        found.into_iter().map(|(_, notes)| notes).collect()
    }
}

/// `https://github.com/…`, with nothing that could make the host read as another.
pub fn valid_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
    host == RELEASES_HOST && !url.chars().any(|c| c.is_control() || c.is_whitespace() || matches!(c, '"' | '\\'))
}

/// A release's notes, for What's New.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Notes {
    pub version: String,
    pub published: Option<String>,
    pub notes: String,
}

/// Where an update is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// This copy can't update itself (`detail` says why): one built from source, a test build.
    Off,
    #[default]
    Idle,
    Checking,
    UpToDate,
    /// A newer release is out (`release`), not downloaded yet.
    Available,
    Downloading,
    /// Downloaded and checked: asides restarts into it, or swaps it in when it quits.
    Ready,
    /// asides is quitting to swap it in.
    Installing,
    Failed,
}

/// Updates, as the app publishes them for the asides window (`state.json`'s `update`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Status {
    pub phase: Phase,
    /// Why updates are off, or what went wrong.
    pub detail: Option<String>,
    /// The user asked (Check Now, an update's button): how it went shows, even "up to date".
    pub asked: bool,
    /// The release offered, from the check that found it until it's installed.
    pub release: Option<Notes>,
    /// How much of it is downloaded, 0 to 1.
    pub progress: f32,
    /// What's new: the notes of the releases up to this one, newest first, as last found.
    pub notes: Vec<Notes>,
    /// When asides last looked (ms since the epoch).
    pub checked_ms: Option<u64>,
}

impl Status {
    /// The window's footer says it: an update found, or how a check the user asked for went. A
    /// check in the background that found nothing says nothing.
    pub fn shows(&self) -> bool {
        match self.phase {
            Phase::Available | Phase::Downloading | Phase::Ready | Phase::Installing => true,
            Phase::Checking | Phase::UpToDate | Phase::Failed => self.asked,
            Phase::Idle | Phase::Off => false,
        }
    }

    /// What it says, in a line.
    pub fn summary(&self) -> String {
        let version = self.release.as_ref().map_or("", |r| r.version.as_str());
        let current = env!("CARGO_PKG_VERSION");
        match self.phase {
            Phase::Off => match &self.detail {
                Some(why) => format!("Updates are off ({why})"),
                None => "Updates are off".into(),
            },
            Phase::Idle => format!("asides {current}"),
            Phase::Checking => "Checking for updates…".into(),
            Phase::UpToDate => format!("asides {current} is up to date"),
            Phase::Available => format!("Update to {version}"),
            Phase::Downloading => format!("Downloading {version}… {}%", (self.progress * 100.0).floor() as u32),
            Phase::Ready => format!("Restart to update to {version}"),
            Phase::Installing => "Restarting…".into(),
            Phase::Failed => self.detail.clone().unwrap_or_else(|| "The update failed".into()),
        }
    }

    /// What clicking it does.
    pub fn click(&self) -> Option<Step> {
        match self.phase {
            Phase::Available | Phase::Ready => Some(Step::Install),
            Phase::UpToDate | Phase::Failed => Some(Step::Dismiss),
            _ => None,
        }
    }

    /// What's new as the copy before this one left it (the notes it had, and the release it
    /// installed): this copy's What's New until it checks.
    pub fn handed_over(&self) -> Vec<Notes> {
        let current = Version::current();
        let mut notes: Vec<(Version, Notes)> = self
            .release
            .iter()
            .chain(&self.notes)
            .filter(|n| !n.notes.trim().is_empty())
            .filter_map(|n| Some((Version::parse(&n.version)?, n.clone())))
            .filter(|(version, _)| *version <= current)
            .collect();
        notes.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
        notes.dedup_by_key(|(version, _)| *version);
        notes.into_iter().map(|(_, notes)| notes).collect()
    }

    /// What's new since `seen`, newest first: what this copy brought that the user hasn't read.
    pub fn unseen(&self, seen: Option<&str>) -> Vec<&Notes> {
        let Some(seen) = seen.and_then(Version::parse) else { return Vec::new() };
        let current = Version::current();
        self.notes.iter().filter(|n| Version::parse(&n.version).is_some_and(|v| v > seen && v <= current)).collect()
    }
}

/// What the asides window asks of the app's updater (`asides://update?step=…`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Look for an update now, and say how it went.
    Check,
    /// Download the update if it isn't yet, then restart into it.
    Install,
    /// Clear what the last check said.
    Dismiss,
    /// What's New was read.
    Seen,
}

impl Step {
    pub const ALL: [Step; 4] = [Step::Check, Step::Install, Step::Dismiss, Step::Seen];

    pub fn as_str(self) -> &'static str {
        match self {
            Step::Check => "check",
            Step::Install => "install",
            Step::Dismiss => "dismiss",
            Step::Seen => "seen",
        }
    }

    pub fn parse(name: &str) -> Option<Step> {
        Step::ALL.into_iter().find(|s| s.as_str() == name)
    }
}

/// Release notes, as What's New shows them: Markdown's headings, paragraphs and list items.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(String),
    Paragraph(Vec<Span>),
    Item(Vec<Span>),
}

/// A run of text, bold or code (links show their text).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub code: bool,
}

pub fn blocks(markdown: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut paragraph: Vec<&str> = Vec::new();
    // A list item's next lines (before a blank one) go on with it.
    let mut in_item = false;
    let flush = |paragraph: &mut Vec<&str>, blocks: &mut Vec<Block>| {
        if !paragraph.is_empty() {
            blocks.push(Block::Paragraph(spans(&paragraph.join(" "))));
            paragraph.clear();
        }
    };
    for line in markdown.lines().map(str::trim) {
        if line.is_empty() {
            flush(&mut paragraph, &mut blocks);
            in_item = false;
        } else if line.starts_with('#') {
            flush(&mut paragraph, &mut blocks);
            let text = spans(line.trim_start_matches('#').trim()).into_iter().map(|s| s.text).collect();
            blocks.push(Block::Heading(text));
            in_item = false;
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush(&mut paragraph, &mut blocks);
            blocks.push(Block::Item(spans(item)));
            in_item = true;
        } else if let (true, Some(Block::Item(item))) = (in_item, blocks.last_mut()) {
            let mut more = spans(&format!(" {line}"));
            item.append(&mut more);
        } else {
            paragraph.push(line);
        }
    }
    flush(&mut paragraph, &mut blocks);
    blocks
}

fn spans(text: &str) -> Vec<Span> {
    let text = link_texts(text);
    let mut spans = Vec::new();
    let mut span = Span::default();
    let mut rest = text.as_str();
    while let Some(c) = rest.chars().next() {
        let marker = if !span.code && rest.starts_with("**") {
            2
        } else if c == '`' {
            1
        } else {
            span.text.push(c);
            rest = &rest[c.len_utf8()..];
            continue;
        };
        let (bold, code) = (span.bold ^ (marker == 2), span.code ^ (marker == 1));
        let done = std::mem::replace(&mut span, Span { text: String::new(), bold, code });
        if !done.text.is_empty() {
            spans.push(done);
        }
        rest = &rest[marker..];
    }
    if !span.text.is_empty() {
        spans.push(span);
    }
    spans
}

/// `[text](url)` → `text`.
fn link_texts(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let link = after.find("](").and_then(|mid| after[mid + 2..].find(')').map(|end| (mid, mid + 2 + end)));
        match link {
            Some((mid, end)) => {
                out.push_str(&rest[..open]);
                out.push_str(&after[..mid]);
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[..=open]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYSTEM: Version = Version(26, 0, 0);

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            url: format!(
                "https://github.com/mihaicristianfarcas/asides/releases/download/v{version}/asides-{version}.zip"
            ),
            size: 9_000_000,
            sha256: "a".repeat(64),
            notes: format!("Notes for {version}."),
            ..Release::default()
        }
    }

    fn feed(releases: Vec<Release>) -> Feed {
        Feed { releases }
    }

    #[test]
    fn parses_versions() {
        assert_eq!(Version::parse("0.8.0"), Some(Version(0, 8, 0)));
        assert_eq!(Version::parse(" v1.2.3 "), Some(Version(1, 2, 3)));
        assert_eq!(Version::parse("15.5"), Some(Version(15, 5, 0)));
        assert_eq!(Version::parse("26"), Some(Version(26, 0, 0)));
        for bad in ["", "latest", "1.x.3", "1.2.3.4", "0.8.0-beta", "+1.0.0", "1..2"] {
            assert_eq!(Version::parse(bad), None, "{bad}");
        }
        assert!(Version(0, 10, 0) > Version(0, 9, 9));
        assert!(Version(1, 0, 0) > Version(0, 99, 99));
        assert_eq!(Version(15, 5, 0).to_string(), "15.5.0");
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn offers_the_newest_newer_release() {
        let found = feed(vec![release("0.7.9"), release("0.9.0"), release("0.8.1")]);
        assert_eq!(found.newest(Version(0, 8, 0), SYSTEM).map(|r| r.version.as_str()), Some("0.9.0"));
        assert_eq!(found.newest(Version(0, 9, 0), SYSTEM), None);
        assert_eq!(found.newest(Version(1, 0, 0), SYSTEM), None, "never an older one");
    }

    #[test]
    fn skips_releases_it_cant_install() {
        let mut newer_macos = release("0.9.3");
        newer_macos.minimum_system_version = Some("27.0".into());
        let mut elsewhere = release("0.9.2");
        elsewhere.url = "https://evil.test/asides.zip".into();
        let mut no_size = release("0.9.1");
        no_size.size = 0;
        let mut no_digest = release("0.9.0");
        no_digest.sha256 = "nope".into();
        let mut unversioned = release("0.9.4");
        unversioned.version = "nightly".into();
        let rows = vec![newer_macos, elsewhere, no_size, no_digest, unversioned, release("0.8.5")];
        assert_eq!(feed(rows).newest(Version(0, 8, 0), SYSTEM).map(|r| r.version.as_str()), Some("0.8.5"));
    }

    #[test]
    fn downloads_come_only_from_github() {
        assert!(valid_url("https://github.com/mihaicristianfarcas/asides/releases/download/v1/a.zip"));
        for bad in [
            "http://github.com/a.zip",
            "https://github.com.evil.test/a.zip",
            "https://github.com@evil.test/a.zip",
            "https://evil.test/github.com/a.zip",
            "https://github.com/a b.zip",
            "https://github.com/a\"b.zip",
            "file:///tmp/a.zip",
        ] {
            assert!(!valid_url(bad), "{bad}");
        }
    }

    #[test]
    fn reads_the_feed_the_release_script_writes() {
        let feed = Feed::parse(
            r#"{
  "releases": [
    {
      "version": "0.8.0",
      "url": "https://github.com/mihaicristianfarcas/asides/releases/download/v0.8.0/asides-0.8.0.zip",
      "size": 8656001,
      "sha256": "2e106c04130ff63f4a3b0b137c6e651a20814a0aa2625a17fb8bf202ac98ba65",
      "minimum_system_version": "14.0",
      "published": "2026-10-20",
      "notes": "**asides updates itself.**\n\n- Restart from the window."
    }
  ]
}"#,
        )
        .unwrap();
        let offered = feed.newest(Version(0, 7, 4), Version(14, 0, 0)).expect("0.8.0 is newer");
        assert_eq!((offered.size, offered.published.as_deref()), (8656001, Some("2026-10-20")));
        assert!(Feed::parse("{\"releases\": 3}").is_err());
    }

    #[test]
    fn notes_are_of_the_releases_this_copy_includes() {
        let mut quiet = release("0.8.1");
        quiet.notes = " ".into();
        let found = feed(vec![release("0.7.0"), release("0.9.0"), release("0.8.0"), quiet, release("0.8.0")]);
        let notes: Vec<String> = found.notes_up_to(Version(0, 8, 1)).into_iter().map(|n| n.version).collect();
        assert_eq!(notes, ["0.8.0", "0.7.0"]);
    }

    fn status(phase: Phase, asked: bool) -> Status {
        let release = Some(Notes { version: "0.9.0".into(), ..Notes::default() });
        Status { phase, asked, release, progress: 0.436, ..Status::default() }
    }

    #[test]
    fn the_footer_says_an_update_or_what_was_asked() {
        for phase in [Phase::Available, Phase::Downloading, Phase::Ready, Phase::Installing] {
            assert!(status(phase, false).shows(), "{phase:?}");
        }
        for phase in [Phase::Checking, Phase::UpToDate, Phase::Failed] {
            assert!(!status(phase, false).shows(), "a background {phase:?} stays quiet");
            assert!(status(phase, true).shows(), "{phase:?} asked for");
        }
        assert!(!status(Phase::Off, true).shows() && !status(Phase::Idle, true).shows());
        assert_eq!(status(Phase::Available, false).summary(), "Update to 0.9.0");
        assert_eq!(status(Phase::Downloading, false).summary(), "Downloading 0.9.0… 43%");
        assert_eq!(status(Phase::Ready, false).summary(), "Restart to update to 0.9.0");
        assert_eq!(status(Phase::Ready, false).click(), Some(Step::Install));
        assert_eq!(status(Phase::Available, false).click(), Some(Step::Install));
        assert_eq!(status(Phase::Failed, true).click(), Some(Step::Dismiss));
        assert_eq!(status(Phase::Downloading, true).click(), None);
    }

    #[test]
    fn whats_new_is_what_came_since_it_was_seen() {
        let current = Version::current();
        let version = |v: Version| Notes { version: v.to_string(), ..Notes::default() };
        let older = Version(0, 0, 1);
        let newer = Version(current.0, current.1 + 1, 0);
        let status = Status { notes: vec![version(newer), version(current), version(older)], ..Status::default() };
        let unseen = |seen: &str| status.unseen(Some(seen)).iter().map(|n| n.version.clone()).collect::<Vec<_>>();
        assert_eq!(unseen("0.0.0"), [current.to_string(), older.to_string()]);
        assert_eq!(unseen(&current.to_string()), Vec::<String>::new());
        assert!(status.unseen(None).is_empty());

        // The copy before this one installed it: its notes are this copy's What's New.
        let mut installed = version(current);
        installed.notes = "Installed.".into();
        let mut kept = version(older);
        kept.notes = "Kept.".into();
        let before = Status { release: Some(installed), notes: vec![kept, version(newer)], ..Status::default() };
        let handed: Vec<String> = before.handed_over().into_iter().map(|n| n.notes).collect();
        assert_eq!(handed, ["Installed.", "Kept."]);
    }

    #[test]
    fn steps_round_trip() {
        for step in Step::ALL {
            assert_eq!(Step::parse(step.as_str()), Some(step));
        }
        assert_eq!(Step::parse("nope"), None);
    }

    fn span(text: &str, bold: bool, code: bool) -> Span {
        Span { text: text.into(), bold, code }
    }

    #[test]
    fn reads_release_notes() {
        let notes = "## Highlights\n\n**Opens twice as fast.** About 110 ms,\ndown from 260.\n\n- **No wait.** One stands by.\n  It uses 50 MB.\n- Run `brew upgrade` or [download it](https://x.test/a.dmg).\n\nThe end.";
        assert_eq!(
            blocks(notes),
            [
                Block::Heading("Highlights".into()),
                Block::Paragraph(vec![
                    span("Opens twice as fast.", true, false),
                    span(" About 110 ms, down from 260.", false, false)
                ]),
                Block::Item(vec![
                    span("No wait.", true, false),
                    span(" One stands by.", false, false),
                    span(" It uses 50 MB.", false, false)
                ]),
                Block::Item(vec![
                    span("Run ", false, false),
                    span("brew upgrade", false, true),
                    span(" or download it.", false, false)
                ]),
                Block::Paragraph(vec![span("The end.", false, false)]),
            ]
        );
        assert_eq!(blocks("a [b] c"), [Block::Paragraph(vec![span("a [b] c", false, false)])]);
        assert_eq!(blocks("`**`"), [Block::Paragraph(vec![span("**", false, true)])]);
    }
}
