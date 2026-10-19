//! Updating asides itself (`asides_core::update`). A thread looks at the feed 20 s after launch
//! and every 6 hours (with automatic updates on) or when asked, downloads a release, checks it and
//! stages it; when the app quits, `install` hands the swap to a script that waits for it to exit.
//! `Flow` is the app's side: what the updater says and the user asks in, the status out.
//!
//! The trust is Apple's, as in diri: there's no key of our own to keep or lose. A download is
//! taken only if its size and SHA-256 are the feed's, `codesign --verify --deep --strict` passes,
//! its team and bundle identifier are the running copy's, Gatekeeper accepts it (notarized and
//! stapled), and its version is the one promised. The feed only ever offers newer versions, from
//! github.com. A copy that isn't notarized (built from source) doesn't update itself, so
//! `make install` never has its build replaced.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use asides_core::history;
use asides_core::paths;
use asides_core::update::{
    self, EVERY, FEED_ENV, FEED_URL, Feed, MAX_FEED, Notes, Phase, Release, Status, Step, UNNOTARIZED_ENV, Version,
};
use sha2::{Digest, Sha256};

/// How long the helper waits for asides to quit before it gives up, changing nothing.
const QUIT_WAIT_SECS: u32 = 60;
/// The most an update may unpack to, in files and bytes.
const MAX_FILES: usize = 100_000;
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;

/// What the app asks of the updater.
#[derive(Debug, PartialEq)]
pub enum Request {
    Check,
    Download(Release),
    /// Automatic updates turned on or off.
    Automatic(bool),
}

/// What the updater says back.
pub enum Reply {
    /// This copy can't update itself, and why.
    Off(String),
    /// The newest release worth offering (if any), and the notes of those up to this copy.
    Checked(Result<(Option<Release>, Vec<Notes>), String>),
    Progress(f32),
    Staged(Result<Staged, String>),
}

/// A release downloaded, checked and unpacked: what `install` swaps in.
#[derive(Debug, Clone, PartialEq)]
pub struct Staged {
    pub release: Release,
    app: PathBuf,
    dir: PathBuf,
    /// The asides.app it replaces.
    bundle: PathBuf,
}

/// What the app does next about an update.
#[derive(Debug, PartialEq)]
pub enum Next {
    Ask(Request),
    /// Quit, swap the update in, and open asides again.
    Restart,
}

/// An update's way through the app: what the updater said and the user asked in; the status the
/// app publishes, and what it does next, out.
#[derive(Default)]
pub struct Flow {
    pub status: Status,
    /// The newer release found, and the one downloaded and checked, ready to swap in.
    found: Option<Release>,
    pub staged: Option<Staged>,
    /// The user asked for the update: once it's downloaded, asides restarts into it.
    restart_when_ready: bool,
}

impl Flow {
    pub fn new(status: Status) -> Flow {
        Flow { status, ..Flow::default() }
    }

    /// What the updater said; `automatic`: updates download on their own.
    pub fn reply(&mut self, reply: Reply, automatic: bool) -> Vec<Next> {
        let status = &mut self.status;
        match reply {
            Reply::Off(why) => {
                status.phase = Phase::Off;
                status.detail = Some(why);
            }
            Reply::Checked(Ok((found, notes))) => {
                status.checked_ms = Some(history::now_ms());
                if !notes.is_empty() {
                    status.notes = notes;
                }
                match found {
                    Some(release) if self.staged.as_ref().is_some_and(|s| s.release.version == release.version) => {
                        status.phase = Phase::Ready;
                    }
                    Some(release) => {
                        status.release = Some(release.notes());
                        status.phase = Phase::Available;
                        self.found = Some(release.clone());
                        if automatic || self.restart_when_ready {
                            return vec![self.download(release)];
                        }
                    }
                    None if self.staged.is_some() => status.phase = Phase::Ready,
                    None => {
                        self.found = None;
                        status.release = None;
                        status.phase = Phase::UpToDate;
                        self.restart_when_ready = false;
                    }
                }
            }
            Reply::Checked(Err(why)) => {
                status.checked_ms = Some(history::now_ms());
                status.detail = Some(why);
                self.restart_when_ready = false;
                // An update found earlier still stands.
                self.settle(Phase::Failed);
            }
            Reply::Progress(progress) => status.progress = progress,
            Reply::Staged(Ok(staged)) => {
                self.staged = Some(staged);
                status.phase = Phase::Ready;
                if std::mem::take(&mut self.restart_when_ready) {
                    return vec![Next::Restart];
                }
            }
            Reply::Staged(Err(why)) => {
                // A download that fails says so, even one started in the background.
                status.phase = Phase::Failed;
                status.detail = Some(why);
                status.asked = true;
                self.restart_when_ready = false;
            }
        }
        Vec::new()
    }

    /// What the user asked (in the window or the menu). `Step::Seen` is the app's.
    pub fn step(&mut self, step: Step) -> Vec<Next> {
        match step {
            Step::Check | Step::Install if self.status.phase == Phase::Off => Vec::new(),
            Step::Check => {
                self.status.asked = true;
                if self.staged.is_some() {
                    self.status.phase = Phase::Ready;
                    Vec::new()
                } else if matches!(self.status.phase, Phase::Checking | Phase::Downloading) {
                    Vec::new()
                } else {
                    self.status.phase = Phase::Checking;
                    vec![Next::Ask(Request::Check)]
                }
            }
            Step::Install if self.staged.is_some() => vec![Next::Restart],
            Step::Install => {
                self.status.asked = true;
                self.restart_when_ready = true;
                match self.found.clone() {
                    Some(_) if self.status.phase == Phase::Downloading => Vec::new(),
                    Some(release) => vec![self.download(release)],
                    None => {
                        self.status.phase = Phase::Checking;
                        vec![Next::Ask(Request::Check)]
                    }
                }
            }
            Step::Dismiss => {
                self.status.asked = false;
                self.settle(Phase::Idle);
                Vec::new()
            }
            Step::Seen => Vec::new(),
        }
    }

    /// Automatic updates turned on or off: with an update found, on gets it ready.
    pub fn automatic(&mut self, on: bool) -> Vec<Next> {
        let mut next = vec![Next::Ask(Request::Automatic(on))];
        if let (true, Phase::Available, Some(release)) = (on, self.status.phase, self.found.clone()) {
            next.push(self.download(release));
        }
        next
    }

    /// Downloading because the user asked to restart into it (the menu says how far it is).
    pub fn restarting_after_download(&self) -> bool {
        self.restart_when_ready && self.status.phase == Phase::Downloading
    }

    /// Back to what's known: the update downloaded, or the one found, or `otherwise`.
    fn settle(&mut self, otherwise: Phase) {
        self.status.phase = match (&self.staged, &self.found) {
            (Some(_), _) => Phase::Ready,
            (None, Some(_)) => Phase::Available,
            (None, None) => otherwise,
        };
    }

    fn download(&mut self, release: Release) -> Next {
        self.status.release = Some(release.notes());
        self.status.phase = Phase::Downloading;
        self.status.progress = 0.0;
        Next::Ask(Request::Download(release))
    }
}

pub struct Updater {
    requests: Sender<Request>,
}

impl Updater {
    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }
}

/// Starts the updater: it checks `first_check` after launch, then every 6 hours, while `automatic`.
pub fn spawn(automatic: bool, first_check: Duration, report: impl Fn(Reply) -> bool + Send + 'static) -> Updater {
    let (requests, asked) = channel();
    std::thread::Builder::new()
        .name("asides-update".into())
        .spawn(move || run(asked, automatic, first_check, report))
        .expect("spawn the updater thread");
    Updater { requests }
}

fn run(asked: Receiver<Request>, mut automatic: bool, first_check: Duration, report: impl Fn(Reply) -> bool) {
    let running = match Running::find() {
        Ok(running) => running,
        Err(why) => {
            report(Reply::Off(why));
            return;
        }
    };
    running.clean_cache();
    let mut due = Instant::now() + first_check;
    loop {
        let reply = match asked.recv_timeout(due.saturating_duration_since(Instant::now())) {
            Ok(Request::Automatic(on)) => {
                automatic = on;
                continue;
            }
            Ok(Request::Check) => {
                due = Instant::now() + EVERY;
                Reply::Checked(running.check())
            }
            Ok(Request::Download(release)) => Reply::Staged(running.download(&release, |p| {
                report(Reply::Progress(p));
            })),
            Err(RecvTimeoutError::Timeout) => {
                due = Instant::now() + EVERY;
                if !automatic {
                    continue;
                }
                Reply::Checked(running.check())
            }
            Err(RecvTimeoutError::Disconnected) => return,
        };
        if !report(reply) {
            return;
        }
    }
}

/// The asides.app running, as far as updating it goes.
struct Running {
    bundle: PathBuf,
    signature: Signature,
    version: Version,
}

impl Running {
    fn find() -> Result<Running, String> {
        if cfg!(feature = "dev-hooks") {
            return Err("this is a test build".into());
        }
        let exe = std::env::current_exe().map_err(|err| err.to_string())?;
        let bundle = enclosing_bundle(&exe.canonicalize().unwrap_or(exe)).ok_or("this copy isn't in an app")?;
        let signature =
            signature(&bundle).filter(|s| s.developer_id).ok_or("this copy isn't signed by its developer")?;
        if std::env::var(UNNOTARIZED_ENV).as_deref() != Ok("1") && !gatekeeper_accepts(&bundle) {
            return Err("this copy isn't notarized".into());
        }
        Ok(Running { bundle, signature, version: Version::current() })
    }

    fn check(&self) -> Result<(Option<Release>, Vec<Notes>), String> {
        let url = std::env::var(FEED_ENV).unwrap_or_else(|_| FEED_URL.into());
        let text = match url.strip_prefix("file://") {
            Some(path) => fs::read_to_string(path).map_err(|err| err.to_string()),
            None => fetch_text(&url),
        }
        .map_err(|why| format!("Couldn't check for updates ({why})"))?;
        let feed = Feed::parse(&text).map_err(|why| format!("Couldn't check for updates ({why})"))?;
        Ok((feed.newest(self.version, system_version()).cloned(), feed.notes_up_to(self.version)))
    }

    fn download(&self, release: &Release, progress: impl Fn(f32)) -> Result<Staged, String> {
        let name = &release.version;
        // The feed's version names a folder: only a version's shape may.
        let version = Version::parse(name).ok_or_else(|| format!("{name} isn't a version"))?;
        writable(&self.bundle)?;
        if !update::valid_url(&release.url) {
            return Err(format!("The update feed points somewhere other than GitHub ({})", release.url));
        }
        let dir = paths::updates_dir().join(version.to_string());
        fs::create_dir_all(&dir).map_err(|err| format!("Couldn't download {name} ({err})"))?;
        let zip = dir.join("asides.zip");
        fetch_file(&release.url, &zip, release.size, &release.sha256, progress)
            .map_err(|why| format!("Couldn't download {name} ({why})"))?;
        let staged = unpack(&zip, &dir.join("staged"))
            .and_then(|app| self.verify(&app, version).map(|()| app))
            .map_err(|why| format!("The download of {name} failed its checks ({why})"));
        let _ = fs::remove_file(&zip);
        Ok(Staged { release: release.clone(), app: staged?, dir, bundle: self.bundle.clone() })
    }

    /// A notarized asides from the same developer, of the version promised (a real but older
    /// release must not stand in for the newer one).
    fn verify(&self, app: &Path, version: Version) -> Result<(), String> {
        let sealed = Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(app).output();
        if !sealed.as_ref().is_ok_and(|o| o.status.success()) {
            return Err("its signature doesn't hold".into());
        }
        let found = signature(app).ok_or("it isn't signed")?;
        if !found.developer_id || found.team != self.signature.team || found.identifier != self.signature.identifier {
            return Err(format!("it's signed by {} ({}), not asides' developer", found.team, found.identifier));
        }
        if !gatekeeper_accepts(app) {
            return Err("Gatekeeper rejects it".into());
        }
        let plist = app.join("Contents/Info.plist");
        let read = Command::new("/usr/bin/defaults").arg("read").arg(&plist).arg("CFBundleShortVersionString").output();
        let found = read.ok().and_then(|o| Version::parse(&String::from_utf8_lossy(&o.stdout)));
        if found != Some(version) {
            return Err(format!("it's {}, not {version}", found.map_or("no version".into(), |v| v.to_string())));
        }
        Ok(())
    }

    /// Removes the downloads of versions this copy already is (or is past).
    fn clean_cache(&self) {
        for entry in fs::read_dir(paths::updates_dir()).into_iter().flatten().flatten() {
            let name = entry.file_name();
            if name.to_str().and_then(Version::parse).is_some_and(|v| v <= self.version) {
                let _ = fs::remove_dir_all(entry.path());
            }
        }
    }
}

/// `…/asides.app/Contents/MacOS/asides-app` → `…/asides.app`.
fn enclosing_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS" && contents.file_name()? == "Contents" && app.extension()? == "app")
        .then(|| app.to_path_buf())
}

#[derive(Debug, Default, PartialEq)]
struct Signature {
    identifier: String,
    team: String,
    /// Signed with a Developer ID Application certificate (not Apple Development, nor ad hoc).
    developer_id: bool,
}

impl Signature {
    /// From what `codesign -dv --verbose=4` writes (on stderr).
    fn parse(text: &str) -> Signature {
        let mut found = Signature::default();
        for (key, value) in text.lines().filter_map(|l| l.split_once('=')) {
            match key {
                "Identifier" => found.identifier = value.into(),
                "TeamIdentifier" if value != "not set" => found.team = value.into(),
                "Authority" if value.starts_with("Developer ID Application:") => found.developer_id = true,
                _ => {}
            }
        }
        found.developer_id &= !found.team.is_empty();
        found
    }
}

fn signature(bundle: &Path) -> Option<Signature> {
    let output = Command::new("/usr/bin/codesign").args(["-dv", "--verbose=4"]).arg(bundle).output().ok()?;
    output.status.success().then(|| Signature::parse(&String::from_utf8_lossy(&output.stderr)))
}

/// Gatekeeper would open it: for a Developer ID app, notarized (stapled, so no network needed).
fn gatekeeper_accepts(bundle: &Path) -> bool {
    Command::new("/usr/sbin/spctl")
        .args(["--assess", "--type", "execute"])
        .arg(bundle)
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// This Mac's macOS (`0.0.0` if unknown, which no release with an oldest macOS runs on).
fn system_version() -> Version {
    let output = Command::new("/usr/bin/sw_vers").arg("-productVersion").output();
    output.ok().and_then(|o| Version::parse(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default()
}

/// The swap renames the app and puts the update beside it: that needs its folder writable. Known
/// before downloading anything.
fn writable(bundle: &Path) -> Result<(), String> {
    let folder = bundle.parent().ok_or("asides has no folder")?;
    let probe = folder.join(format!(".asides-update-{}", std::process::id()));
    match File::create(&probe) {
        Ok(_) => {
            let _ = fs::remove_file(&probe);
            Ok(())
        }
        Err(_) => Err(format!("asides can't write to {} (download it from the website instead)", paths::tilde(folder))),
    }
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(timeout))
        .user_agent(format!("asides/{}", env!("CARGO_PKG_VERSION")))
        .build()
        .new_agent()
}

fn network(err: ureq::Error) -> String {
    match err {
        ureq::Error::StatusCode(code) => format!("GitHub answered {code}"),
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Timeout(_) | ureq::Error::Io(_) => {
            "couldn't reach GitHub".into()
        }
        other => other.to_string(),
    }
}

fn fetch_text(url: &str) -> Result<String, String> {
    let response = agent(Duration::from_secs(20)).get(url).call().map_err(network)?;
    response.into_body().with_config().limit(MAX_FEED).read_to_string().map_err(network)
}

/// Downloads `url` to `path`: exactly `size` bytes, of SHA-256 `sha256`.
fn fetch_file(url: &str, path: &Path, size: u64, sha256: &str, progress: impl Fn(f32)) -> Result<(), String> {
    let response = agent(Duration::from_secs(15 * 60)).get(url).call().map_err(network)?;
    let mut reader = response.into_body().into_with_config().limit(size + 1).reader();
    let mut file = File::create(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 16];
    let (mut done, mut told) = (0u64, 0.0f32);
    loop {
        let n = reader.read(&mut buf).map_err(|err| format!("the download stopped ({err})"))?;
        if n == 0 {
            break;
        }
        file.write_all(&buf[..n]).map_err(|err| err.to_string())?;
        hasher.update(&buf[..n]);
        done += n as u64;
        let fraction = (done as f32 / size as f32).min(1.0);
        if fraction - told >= 0.01 {
            told = fraction;
            progress(fraction);
        }
    }
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if done != size || !digest.eq_ignore_ascii_case(sha256) {
        let _ = fs::remove_file(path);
        return Err(format!("it isn't the file the feed names ({done} bytes, SHA-256 {digest})"));
    }
    Ok(())
}

/// Unpacks the update and finds the app in it. The zip is read first: nothing may land outside
/// the folder (no `..`, no links), nor unpack to more than it should.
fn unpack(zip: &Path, into: &Path) -> Result<PathBuf, String> {
    check_zip(zip)?;
    let _ = fs::remove_dir_all(into);
    fs::create_dir_all(into).map_err(|err| err.to_string())?;
    // ditto, as it was zipped: it keeps what the signature covers.
    let output = Command::new("/usr/bin/ditto").args(["-x", "-k"]).arg(zip).arg(into).output();
    match output {
        Ok(o) if o.status.success() => {}
        Ok(o) => return Err(format!("it doesn't unzip ({})", String::from_utf8_lossy(&o.stderr).trim())),
        Err(err) => return Err(err.to_string()),
    }
    let apps: Vec<PathBuf> = fs::read_dir(into)
        .map_err(|err| err.to_string())?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
        .collect();
    match apps.as_slice() {
        [app] => Ok(app.clone()),
        _ => Err(format!("it holds {} apps, not one", apps.len())),
    }
}

fn check_zip(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|err| err.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| format!("it isn't a zip ({err})"))?;
    if archive.len() > MAX_FILES {
        return Err(format!("it holds {} files", archive.len()));
    }
    let mut unpacked = 0u64;
    for i in 0..archive.len() {
        let entry = archive.by_index_raw(i).map_err(|err| err.to_string())?;
        if entry.enclosed_name().is_none() || entry.is_symlink() {
            return Err(format!("it holds {:?}, which could land outside its folder", entry.name()));
        }
        unpacked = unpacked.saturating_add(entry.size());
        if unpacked > MAX_UNPACKED {
            return Err("it unpacks to more than 1 GB".into());
        }
    }
    Ok(())
}

/// Starts the script that swaps the update in once asides has quit; the app must quit right after.
/// `reopen`: what to open then (`asides://window`, or `""` for asides alone); `None` leaves it
/// closed (an update installed as asides quits). The script's output goes to `install.log` in the
/// update's folder.
pub fn install(staged: &Staged, reopen: Option<&str>) -> Result<(), String> {
    writable(&staged.bundle)?;
    let script = staged.dir.join("install.sh");
    fs::write(&script, script_text(std::process::id(), &staged.app, &staged.bundle, reopen))
        .map_err(|err| err.to_string())?;
    let log = File::create(staged.dir.join("install.log")).map_err(|err| err.to_string())?;
    let errors = log.try_clone().map_err(|err| err.to_string())?;
    Command::new("/bin/sh")
        .arg(&script)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(errors)
        // Its own process group: what ends asides doesn't end it.
        .process_group(0)
        .spawn()
        .map(drop)
        .map_err(|err| format!("Couldn't start installing ({err})"))
}

/// The swap, as a script for `/bin/sh`. The update is copied beside the app while asides still
/// runs; once it has quit, the app's path only changes by renaming, so it's never missing or half
/// copied, and if anything fails the old app goes back.
fn script_text(pid: u32, staged: &Path, target: &Path, reopen: Option<&str>) -> String {
    let quote = |text: &str| format!("'{}'", text.replace('\'', r"'\''"));
    let staged = quote(&staged.to_string_lossy());
    let target = quote(&target.to_string_lossy());
    let reopen = match reopen {
        None => ":".to_string(),
        Some("") => "/usr/bin/open \"$target\"".to_string(),
        Some(link) => format!("/usr/bin/open -a \"$target\" {}", quote(link)),
    };
    let tries = QUIT_WAIT_SECS * 10;
    format!(
        r#"#!/bin/sh
# Written by asides' updater: swaps asides.app for the update once asides has quit.
set -u

pid={pid}
staged={staged}
target={target}
next="$target.asides-next"
backup="$target.asides-previous"

rm -rf "$next"
copied=1
if ! /usr/bin/ditto "$staged" "$next"; then
    echo "could not copy $staged; keeping the app as it is" >&2
    rm -rf "$next"
    copied=0
fi

waited=0
while kill -0 "$pid" 2>/dev/null; do
    waited=$((waited + 1))
    if [ "$waited" -gt {tries} ]; then
        echo "asides (pid $pid) still runs after {QUIT_WAIT_SECS} s; leaving $target as it is" >&2
        rm -rf "$next"
        exit 1
    fi
    sleep 0.1
done

if [ "$copied" -eq 1 ]; then
    rm -rf "$backup"
    if ! mv "$target" "$backup"; then
        echo "could not move $target aside; nothing changed" >&2
        rm -rf "$next"
    elif mv "$next" "$target"; then
        rm -rf "$backup"
        touch "$target"
        {reopen}
        exit 0
    else
        echo "could not put the update in place; putting the old app back" >&2
        rm -rf "$target"
        mv "$backup" "$target"
        rm -rf "$next"
    fi
fi

{reopen}
exit 1
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const TARGET: &str = "/Applications/asides.app";

    fn found(version: &str) -> Reply {
        let release = Release { version: version.into(), ..Release::default() };
        Reply::Checked(Ok((Some(release), Vec::new())))
    }

    fn staged(version: &str) -> Reply {
        let release = Release { version: version.into(), ..Release::default() };
        Reply::Staged(Ok(Staged { release, app: "a".into(), dir: "d".into(), bundle: TARGET.into() }))
    }

    fn download(version: &str) -> Vec<Next> {
        vec![Next::Ask(Request::Download(Release { version: version.into(), ..Release::default() }))]
    }

    #[test]
    fn found_in_the_background_it_downloads_and_waits_for_a_restart() {
        let mut flow = Flow::default();
        assert_eq!(flow.reply(found("0.9.0"), true), download("0.9.0"));
        assert!(flow.status.shows() && !flow.restarting_after_download());
        assert!(flow.reply(Reply::Progress(0.5), true).is_empty());
        assert!(flow.reply(staged("0.9.0"), true).is_empty(), "it doesn't restart uninvited");
        assert_eq!((flow.status.phase, flow.status.summary().as_str()), (Phase::Ready, "Restart to update to 0.9.0"));
        assert_eq!(flow.step(Step::Install), [Next::Restart]);
        // Checked again, it's still the one ready.
        assert!(flow.reply(found("0.9.0"), true).is_empty());
        assert_eq!(flow.status.phase, Phase::Ready);
    }

    #[test]
    fn without_automatic_updates_one_click_downloads_and_restarts() {
        let mut flow = Flow::default();
        assert!(flow.reply(found("0.9.0"), false).is_empty());
        assert_eq!(flow.status.phase, Phase::Available);
        assert_eq!(flow.step(Step::Install), download("0.9.0"));
        assert!(flow.restarting_after_download());
        assert!(flow.step(Step::Install).is_empty(), "clicked again while it downloads");
        assert_eq!(flow.reply(staged("0.9.0"), false), [Next::Restart]);
    }

    #[test]
    fn installing_with_nothing_found_looks_first() {
        let mut flow = Flow::default();
        assert_eq!(flow.step(Step::Install), [Next::Ask(Request::Check)]);
        assert_eq!(flow.reply(found("0.9.0"), false), download("0.9.0"));
        assert_eq!(flow.reply(staged("0.9.0"), false), [Next::Restart]);
    }

    #[test]
    fn a_check_asked_for_says_how_it_went() {
        let mut flow = Flow::default();
        assert_eq!(flow.step(Step::Check), [Next::Ask(Request::Check)]);
        assert!(flow.status.shows());
        assert!(flow.step(Step::Check).is_empty(), "one check at a time");
        flow.reply(Reply::Checked(Ok((None, Vec::new()))), true);
        assert!(flow.status.shows() && flow.status.phase == Phase::UpToDate);
        flow.step(Step::Dismiss);
        assert!(!flow.status.shows() && flow.status.phase == Phase::Idle);
        // In the background, an up-to-date check says nothing.
        flow.reply(Reply::Checked(Ok((None, Vec::new()))), true);
        assert!(!flow.status.shows());
    }

    #[test]
    fn failures_dont_hide_an_update_found() {
        let mut flow = Flow::default();
        flow.reply(Reply::Checked(Err("Couldn't check for updates (couldn't reach GitHub)".into())), true);
        assert!(flow.status.phase == Phase::Failed && !flow.status.shows(), "a background check fails quietly");
        flow.reply(found("0.9.0"), false);
        flow.reply(Reply::Checked(Err("offline".into())), false);
        assert_eq!(flow.status.phase, Phase::Available);
        // A download that fails says so; put away, the update is offered again.
        flow.step(Step::Install);
        flow.reply(Reply::Staged(Err("Couldn't download 0.9.0 (GitHub answered 503)".into())), false);
        assert!(flow.status.shows() && flow.status.summary().contains("503"));
        assert!(!flow.restarting_after_download());
        flow.step(Step::Dismiss);
        assert_eq!(flow.status.phase, Phase::Available);
        assert_eq!(flow.step(Step::Install), download("0.9.0"));
    }

    #[test]
    fn turning_automatic_updates_on_gets_the_update_ready() {
        let mut flow = Flow::default();
        flow.reply(found("0.9.0"), false);
        let mut next = vec![Next::Ask(Request::Automatic(true))];
        next.extend(download("0.9.0"));
        assert_eq!(flow.automatic(true), next);
        assert_eq!(flow.automatic(false), [Next::Ask(Request::Automatic(false))]);
    }

    #[test]
    fn a_copy_that_cant_update_does_nothing() {
        let mut flow = Flow::default();
        flow.reply(Reply::Off("this copy isn't notarized".into()), true);
        assert!(flow.step(Step::Check).is_empty() && flow.step(Step::Install).is_empty());
        assert_eq!(flow.status.summary(), "Updates are off (this copy isn't notarized)");
        assert!(!flow.status.shows());
    }

    #[test]
    fn finds_the_app_around_its_binary() {
        let found = enclosing_bundle(Path::new("/Applications/asides.app/Contents/MacOS/asides-app"));
        assert_eq!(found, Some(PathBuf::from(TARGET)));
        assert_eq!(enclosing_bundle(Path::new("/Users/me/asides/target/release/asides-app")), None);
        assert_eq!(enclosing_bundle(Path::new("/opt/asides/Contents/MacOS/asides-app")), None);
    }

    #[test]
    fn reads_signatures() {
        let developer_id = "Executable=/Applications/asides.app/Contents/MacOS/asides-app\n\
            Identifier=io.github.mihaicristianfarcas.asides\n\
            Authority=Developer ID Application: Mihai-Cristian Farcas (5DSNCS98LH)\n\
            Authority=Developer ID Certification Authority\n\
            Authority=Apple Root CA\n\
            TeamIdentifier=5DSNCS98LH\n";
        let found = Signature::parse(developer_id);
        assert_eq!(
            found,
            Signature {
                identifier: "io.github.mihaicristianfarcas.asides".into(),
                team: "5DSNCS98LH".into(),
                developer_id: true
            }
        );
        let ad_hoc = Signature::parse(
            "Identifier=io.github.mihaicristianfarcas.asides\nSignature=adhoc\nTeamIdentifier=not set\n",
        );
        assert!(!ad_hoc.developer_id && ad_hoc.team.is_empty());
        let development =
            Signature::parse("Authority=Apple Development: Someone (ABCDE12345)\nTeamIdentifier=ABCDE12345\n");
        assert!(!development.developer_id);
    }

    #[test]
    fn a_test_binary_doesnt_update_itself() {
        assert!(Running::find().is_err());
    }

    fn script(reopen: Option<&str>) -> String {
        script_text(
            4242,
            Path::new("/Users/me/Library/Caches/asides/updates/0.9.0/staged/asides.app"),
            Path::new(TARGET),
            reopen,
        )
    }

    #[test]
    fn the_script_copies_first_then_waits_then_renames() {
        let text = script(Some(""));
        let copy = text.find("/usr/bin/ditto \"$staged\" \"$next\"").unwrap();
        let wait = text.find("kill -0 \"$pid\"").unwrap();
        let aside = text.find("mv \"$target\" \"$backup\"").unwrap();
        let swap = text.find("mv \"$next\" \"$target\"").unwrap();
        assert!(copy < wait && wait < aside && aside < swap);
        assert_eq!(text.matches("ditto").count(), 1, "nothing is copied into the app's path");
        assert!(text.contains("pid=4242"));
        let check = Command::new("/bin/sh").args(["-n", "-c", &text]).output().unwrap();
        assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    }

    #[test]
    fn the_script_reopens_as_asked() {
        assert!(!script(None).contains("/usr/bin/open"));
        assert_eq!(script(Some("")).matches("/usr/bin/open \"$target\"").count(), 2);
        assert_eq!(
            script(Some("asides://window")).matches("/usr/bin/open -a \"$target\" 'asides://window'").count(),
            2
        );
        let quoted =
            script_text(1, Path::new("/tmp/it's here/asides.app"), Path::new("/Applications/My Apps/asides.app"), None);
        assert!(quoted.contains(r"staged='/tmp/it'\''s here/asides.app'"));
        assert!(quoted.contains("target='/Applications/My Apps/asides.app'"));
    }

    fn fake_app(dir: &Path, name: &str, marker: &str) -> PathBuf {
        let app = dir.join(name);
        fs::create_dir_all(app.join("Contents/MacOS")).unwrap();
        fs::write(app.join("Contents/MacOS/asides-app"), marker).unwrap();
        app
    }

    fn marker(app: &Path) -> String {
        fs::read_to_string(app.join("Contents/MacOS/asides-app")).unwrap()
    }

    /// The script as shipped, but saying it reopens rather than opening anything.
    fn run_script(pid: u32, staged: &Path, target: &Path) -> std::process::Output {
        let text = script_text(pid, staged, target, Some("")).replace("/usr/bin/open", "echo reopened");
        Command::new("/bin/sh").args(["-c", &text]).output().unwrap()
    }

    fn quit_pid() -> u32 {
        let mut child = Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    #[test]
    fn the_script_swaps_the_app() {
        let dir = tempfile::tempdir().unwrap();
        let staged = fake_app(dir.path(), "staged.app", "new");
        let target = fake_app(dir.path(), "asides.app", "old");
        let output = run_script(quit_pid(), &staged, &target);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(marker(&target), "new");
        assert!(String::from_utf8_lossy(&output.stdout).contains("reopened"));
        assert!(!dir.path().join("asides.app.asides-previous").exists());
        assert!(!dir.path().join("asides.app.asides-next").exists());
    }

    #[test]
    fn the_script_keeps_the_old_app_when_the_copy_fails() {
        let dir = tempfile::tempdir().unwrap();
        let target = fake_app(dir.path(), "asides.app", "old");
        let output = run_script(quit_pid(), &dir.path().join("missing.app"), &target);
        assert!(!output.status.success());
        assert_eq!(marker(&target), "old");
        assert!(String::from_utf8_lossy(&output.stdout).contains("reopened"), "the old one opens again");
    }

    #[test]
    fn the_script_never_touches_an_app_still_running() {
        let dir = tempfile::tempdir().unwrap();
        let staged = fake_app(dir.path(), "staged.app", "new");
        let target = fake_app(dir.path(), "asides.app", "old");
        let mut running = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let text = script_text(running.id(), &staged, &target, None).replace("-gt 600", "-gt 3");
        let output = Command::new("/bin/sh").args(["-c", &text]).output().unwrap();
        let _ = running.kill();
        let _ = running.wait();
        assert!(!output.status.success());
        assert_eq!(marker(&target), "old");
        assert!(!dir.path().join("asides.app.asides-next").exists());
    }

    /// While asides runs, its app stays whole; once it quits, the new one replaces it by renaming
    /// (never missing for the length of a copy).
    #[test]
    fn the_app_is_never_missing_for_a_copy() {
        let dir = tempfile::tempdir().unwrap();
        let staged = fake_app(dir.path(), "staged.app", "new");
        for i in 0..2000 {
            fs::write(staged.join(format!("Contents/r{i}")), [0u8; 512]).unwrap();
        }
        let target = fake_app(dir.path(), "asides.app", "old");
        let mut running = Command::new("/bin/sleep").arg("30").spawn().unwrap();
        let text = script_text(running.id(), &staged, &target, None);
        let mut helper = Command::new("/bin/sh").args(["-c", &text]).spawn().unwrap();
        let next = dir.path().join("asides.app.asides-next/Contents/r1999");
        let until = Instant::now() + Duration::from_secs(10);
        while !next.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(next.exists(), "the update is copied beside the app while asides runs");
        assert_eq!(marker(&target), "old");
        let _ = running.kill();
        let _ = running.wait();
        let (mut longest, mut missing) = (Duration::ZERO, None::<Instant>);
        let status = loop {
            let whole = target.join("Contents/MacOS/asides-app").exists();
            match (whole, missing) {
                (false, None) => missing = Some(Instant::now()),
                (true, Some(since)) => {
                    longest = longest.max(since.elapsed());
                    missing = None;
                }
                _ => {}
            }
            if let Some(status) = helper.try_wait().unwrap() {
                break status;
            }
        };
        assert!(status.success());
        assert!(longest < Duration::from_millis(20), "asides.app was missing for {longest:?}");
        assert_eq!(marker(&target), "new");
    }

    fn zip_of(dir: &Path, entries: &[(&str, Option<&str>)]) -> PathBuf {
        let path = dir.join("update.zip");
        let mut zip = zip::ZipWriter::new(File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default();
        for (name, link) in entries {
            match link {
                Some(target) => zip.add_symlink(*name, *target, options).unwrap(),
                None => {
                    zip.start_file(*name, options).unwrap();
                    zip.write_all(b"x").unwrap();
                }
            }
        }
        zip.finish().unwrap();
        path
    }

    #[test]
    fn unpacks_the_one_app_in_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let zip = zip_of(dir.path(), &[("asides.app/Contents/MacOS/asides-app", None)]);
        let app = unpack(&zip, &dir.path().join("staged")).unwrap();
        assert_eq!(app, dir.path().join("staged/asides.app"));
        assert!(app.join("Contents/MacOS/asides-app").is_file());
        let none = zip_of(dir.path(), &[("README.txt", None)]);
        assert!(unpack(&none, &dir.path().join("none")).is_err());
    }

    #[test]
    fn refuses_zips_that_reach_outside() {
        let dir = tempfile::tempdir().unwrap();
        for entries in [
            vec![("../evil.app/Contents/x", None)],
            vec![("/tmp/evil", None)],
            vec![("asides.app/Contents/link", Some("/etc"))],
        ] {
            let zip = zip_of(dir.path(), &entries);
            assert!(unpack(&zip, &dir.path().join("staged")).is_err(), "{entries:?}");
        }
        assert!(!dir.path().parent().unwrap().join("evil.app").exists());
    }

    #[test]
    fn the_folder_must_be_writable() {
        let dir = tempfile::tempdir().unwrap();
        assert!(writable(&dir.path().join("asides.app")).is_ok());
        assert!(writable(Path::new("/System/Applications/Notes.app")).is_err());
    }

    /// The whole of an update but the restart, on a real release: download 0.7.4 from GitHub, check
    /// it against the signature releases carry, and swap it over a copy of 0.7.3 in a temporary
    /// folder.
    #[test]
    #[ignore = "downloads two releases (17 MB) from GitHub"]
    fn updates_a_real_release() {
        let dir = tempfile::tempdir().unwrap();
        let old_zip = dir.path().join("old.zip");
        let old = "https://github.com/mihaicristianfarcas/asides/releases/download/v0.7.3/Asides-0.7.3.zip";
        let response = agent(Duration::from_secs(300)).get(old).call().unwrap();
        let mut body = Vec::new();
        response.into_body().into_with_config().limit(update::MAX_SIZE).reader().read_to_end(&mut body).unwrap();
        fs::write(&old_zip, body).unwrap();
        let installed = unpack(&old_zip, &dir.path().join("Applications")).unwrap();
        let running = Running {
            bundle: installed.clone(),
            signature: signature(&installed).expect("0.7.3 is signed"),
            version: Version(0, 7, 3),
        };
        assert!(running.signature.developer_id && gatekeeper_accepts(&installed));
        let release = Release {
            version: "0.7.4".into(),
            url: "https://github.com/mihaicristianfarcas/asides/releases/download/v0.7.4/Asides-0.7.4.zip".into(),
            size: 8_656_001,
            sha256: "2e106c04130ff63f4a3b0b137c6e651a20814a0aa2625a17fb8bf202ac98ba65".into(),
            ..Release::default()
        };
        let zip = dir.path().join("asides.zip");
        let reports = std::cell::Cell::new(0);
        fetch_file(&release.url, &zip, release.size, &release.sha256, |_| reports.set(reports.get() + 1)).unwrap();
        assert!(reports.get() > 10, "progress is told as it goes");
        let app = unpack(&zip, &dir.path().join("staged")).unwrap();
        running.verify(&app, Version(0, 7, 4)).unwrap();
        assert!(running.verify(&app, Version(0, 7, 5)).is_err(), "only the version promised");
        let mut bad = release.clone();
        bad.sha256 = "0".repeat(64);
        assert!(fetch_file(&bad.url, &zip, bad.size, &bad.sha256, |_| {}).is_err());

        let output = run_script(quit_pid(), &app, &installed);
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let plist = installed.join("Contents/Info.plist");
        let version =
            Command::new("/usr/bin/defaults").arg("read").arg(&plist).arg("CFBundleShortVersionString").output();
        assert_eq!(String::from_utf8_lossy(&version.unwrap().stdout).trim(), "0.7.4");
        assert!(gatekeeper_accepts(&installed), "still notarized after the swap");
    }
}
