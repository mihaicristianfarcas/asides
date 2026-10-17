//! Asides: menu-bar app that records spoken bug reports and appends them, transcribed locally,
//! to the latest note in the directory bound with the `asides` CLI. Measure & Annotate freezes the
//! screen to measure, mark, draw on and talk about it (its overlay is a separate process,
//! `asides-overlay`); what's marked lands among what was said. Each aside goes to one agent
//! session, the target (or to the notes only); New Agent Session starts one.

mod audio;
mod capture;
mod clipboard;
mod elements;
mod environment;
mod hotkeys;
mod icons;
mod macos;
mod overlay;
mod stt;
mod terminal;
mod tray;
mod watch;
mod worker;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use asides_core::agents::Listener;
use asides_core::config::{AgentDelivery, Config};
use asides_core::helper::{
    Command as OverlayCommand, Event as OverlayEvent, Outcome as OverlayOutcome, Spec, SpecScreen,
};
use asides_core::inspect::{PtRect, Screen, Units};
use asides_core::install;
use asides_core::launch::{self, AgentKind};
use asides_core::link::Link;
use asides_core::models::ModelSpec;
use asides_core::notes;
use asides_core::overlay::Saved;
use asides_core::paths::{self, tilde};
use asides_core::project;
use asides_core::routes::Route;
use asides_core::sessions::{self, Session, SessionState};
use asides_core::state::{self as app_state, AgentSend, AppState, PendingNew};
use asides_core::threads::{self, Event as ThreadEvent, Role, Status};
use asides_core::wispr::{self, Dictation};
use chrono::{DateTime, Local};
use global_hotkey::{GlobalHotKeyEvent, HotKeyState};
use objc2::MainThreadMarker;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
use tray_icon::TrayIconEvent;
use tray_icon::menu::MenuEvent;

use crate::capture::{Capture, ScreenInfo};
use crate::elements::WindowInfo;
use crate::environment::Environment;
use crate::hotkeys::{Action, Hotkeys};
use crate::icons::Icons;
use crate::macos::{CommandLink, Mic};
use crate::overlay::{Message as OverlayMessage, Overlay};
use crate::stt::{ModelManager, ModelState};
use crate::tray::{MenuAction, TargetRow, Tray, View};
use crate::worker::{CapturedImage, DictationJob, ImageJob, Job, Outcome, RecordingJob, ScreenJob};

/// A press shorter than this toggles recording; a longer one is push-to-talk.
const TAP_MAX: Duration = Duration::from_millis(350);
const TICK: Duration = Duration::from_millis(250);
const FLASH: Duration = Duration::from_secs(2);
const ERROR_ICON: Duration = Duration::from_secs(6);
/// Recordings shorter than this (without images) are treated as accidental and dropped.
const MIN_RECORDING_SECS: f32 = 0.3;
/// How long the outcome of a Send to Agent stays in the menu bar.
const SENT_FLASH: Duration = Duration::from_secs(4);
/// How many of the agents' latest replies the menu shows.
const MAX_REPLIES: usize = 8;
/// How long Measure & Annotate waits for the screenshots before it may ask again.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);

enum UserEvent {
    HotKey(GlobalHotKeyEvent),
    Menu(MenuEvent),
    Work(Outcome),
    Model(&'static str, ModelState),
    Mic(bool),
    ConfigFileChanged,
    Wispr(Result<Dictation, String>),
    /// The processes listening for asides, and the agent sessions.
    Sessions(Vec<Listener>, Vec<Session>),
    /// The coding agents installed, as the user's login shell finds them.
    Agents(Vec<AgentKind>),
    /// A background session started (what `asides claude --background` said), or what went wrong
    /// starting or stopping one.
    Launched(Result<String, String>),
    CommandLinked(CommandLink),
    /// The screens, frozen for Measure & Annotate.
    Captured(Result<Vec<Capture>, String>),
    /// What Measure & Annotate's overlay said.
    Overlay(OverlayMessage),
    /// The latest note's threads (what agents said about its asides), when they change.
    Threads(PathBuf, Vec<ThreadEvent>),
}

struct Recording {
    recorder: audio::Recorder,
    started_at: Instant,
    started: DateTime<Local>,
    dir: PathBuf,
    new_note: Option<Option<String>>,
    target_label: String,
    /// The session it's spoken to (written beside the note), and that session's agent.
    route: Option<Route>,
    to_agent: Option<String>,
    images: Vec<CapturedImage>,
    /// Measure & Annotate annotations made while recording, as (seconds, Markdown).
    texts: Vec<(f32, String)>,
    environment: Environment,
    /// The shortcut press that started it, for hold-to-talk.
    pressed: Option<(Action, Instant)>,
    clipboard: isize,
}

struct LatestNote {
    title: String,
    entries: u32,
    file: PathBuf,
    /// Entries agents haven't resolved or dismissed, once agents have acted on some.
    open: Option<usize>,
    /// Why the next aside starts a new note instead (`auto_new_note`).
    moved_on: Option<&'static str>,
}

struct App {
    mtm: MainThreadMarker,
    proxy: EventLoopProxy<UserEvent>,
    config: Config,
    icons: Option<Icons>,
    tray: Option<Tray>,
    hotkeys: Option<Hotkeys>,
    models: Arc<ModelManager>,
    jobs: Sender<Job>,
    bound: Option<PathBuf>,
    pending_new: Option<PendingNew>,
    recent_dirs: Vec<PathBuf>,
    latest: Option<LatestNote>,
    recording: Option<Recording>,
    in_flight: usize,
    model_state: Option<ModelState>,
    flash: Option<(Instant, String, String)>,
    error: Option<(Instant, String)>,
    mic: Mic,
    keys_down: HashSet<u32>,
    last_request: Option<String>,
    /// The last Send to Agent in each directory, newest first, published in `state.json` for the
    /// agents listening.
    agent_sends: Vec<AgentSend>,
    /// Send to Agent was chosen while recordings were still being transcribed.
    pending_send: bool,
    listeners: Vec<Listener>,
    /// The agent sessions listening (`sessions::Session`), oldest first.
    sessions: Vec<Session>,
    /// The session the user's asides go to; `None` for Notes Only.
    target: Option<String>,
    /// Sessions already seen, so only new ones become the target.
    known_sessions: HashSet<String>,
    sessions_seen: bool,
    /// The coding agents installed (`None` until the login shell has answered).
    agents: Option<Vec<AgentKind>>,
    /// The shortcut that switches sessions is registered (only while there are sessions).
    next_target_on: bool,
    /// The user declined to put the `asides` command on the PATH (published, so it's asked once).
    command_link_declined: bool,
    /// Set to stop watching Wispr Flow's history.
    wispr: Option<Arc<AtomicBool>>,
    /// Measure & Annotate: when the screens were asked for, the screens, and the windows on them
    /// when the shortcut was pressed (until the capture arrives).
    capturing: Option<(Instant, Vec<ScreenInfo>, Vec<WindowInfo>)>,
    /// Measure & Annotate's overlay, while it's open.
    overlay: Option<Overlay>,
    /// The last session's guides and held measurements, which the next can restore.
    last_measure: Saved,
    /// The overlay showed its keys once already.
    overlay_tips_seen: bool,
    /// The directory whose latest note's threads are watched.
    threads_dir: Arc<Mutex<Option<PathBuf>>>,
    /// The note those threads are of, and how many events of them were seen.
    threads_seen: Option<(PathBuf, usize)>,
    /// What agents said lately, for the menu, newest first.
    replies: Vec<String>,
    quit_when_idle: bool,
    quit: bool,
}

fn main() {
    // An agent's process can start the app (its `asides mcp` opens a link): don't hand that
    // session's markers to every terminal and agent the app starts.
    for marker in launch::inherited_markers() {
        // SAFETY: first thing in main, before any thread exists to read the environment.
        unsafe { std::env::remove_var(marker) };
    }
    let mtm = MainThreadMarker::new().expect("must start on the main thread");
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    event_loop.set_activation_policy(ActivationPolicy::Accessory);
    // tao activates the app at launch by default; a background menu-bar app must not take focus.
    event_loop.set_activate_ignoring_other_apps(false);
    let proxy = event_loop.create_proxy();

    let p = proxy.clone();
    GlobalHotKeyEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(UserEvent::HotKey(e));
    }));
    let p = proxy.clone();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = p.send_event(UserEvent::Menu(e));
    }));
    // Unused, but without a handler every hover over the icon would queue an event forever.
    TrayIconEvent::set_event_handler(Some(|_| {}));

    let mut app = App::new(mtm, proxy);
    event_loop.run(move |event, _, control_flow| {
        if let Event::NewEvents(StartCause::ResumeTimeReached { .. }) = &event {
            app.tick();
        }
        match event {
            Event::NewEvents(StartCause::Init) => app.init(),
            Event::Opened { urls } => urls.iter().for_each(|url| app.handle_link(url)),
            Event::UserEvent(event) => app.handle(event),
            _ => {}
        }
        *control_flow = if app.quit {
            ControlFlow::Exit
        } else {
            app.next_wakeup().map_or(ControlFlow::Wait, ControlFlow::WaitUntil)
        };
    });
}

impl App {
    fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<UserEvent>) -> App {
        let config_path = paths::config_path();
        let config = Config::load(&config_path).unwrap_or_default();
        if !config_path.exists() {
            let _ = config.save(&config_path);
        }
        let p = proxy.clone();
        let models = ModelManager::new(move |id, state| {
            let _ = p.send_event(UserEvent::Model(id, state));
        });
        let p = proxy.clone();
        let jobs = worker::spawn(models.clone(), move |outcome| {
            let _ = p.send_event(UserEvent::Work(outcome));
        });
        // Re-bind to where the previous session left off (the CLI usually re-binds anyway).
        let previous = AppState::read(&paths::state_path()).unwrap_or_default();
        let bound = previous.bound_dir.filter(|d| d.is_dir());
        let pending_new = bound.as_ref().and(previous.pending_new);
        let agent_sends = previous.agent_sends;
        let target = previous.target_session;
        let command_link_declined = previous.command_link_declined;
        let overlay_tips_seen = previous.overlay_tips_seen;
        let mut recent_dirs: Vec<PathBuf> = previous.recent_dirs.into_iter().filter(|d| d.is_dir()).collect();
        if let Some(dir) = &bound {
            app_state::remember_dir(&mut recent_dirs, dir);
        }
        App {
            mtm,
            proxy,
            config,
            icons: None,
            tray: None,
            hotkeys: None,
            models,
            jobs,
            bound,
            pending_new,
            recent_dirs,
            latest: None,
            recording: None,
            in_flight: 0,
            model_state: None,
            flash: None,
            error: None,
            mic: macos::mic_status(),
            keys_down: HashSet::new(),
            last_request: None,
            agent_sends,
            pending_send: false,
            listeners: Vec::new(),
            sessions: Vec::new(),
            target,
            known_sessions: HashSet::new(),
            sessions_seen: false,
            agents: None,
            next_target_on: false,
            command_link_declined,
            wispr: None,
            capturing: None,
            overlay: None,
            last_measure: Saved::default(),
            overlay_tips_seen,
            threads_dir: Arc::new(Mutex::new(None)),
            threads_seen: None,
            replies: Vec::new(),
            quit_when_idle: false,
            quit: false,
        }
    }

    fn init(&mut self) {
        let icons = Icons::load();
        match Tray::new(&self.config, &audio::input_device_names(), &terminal::installed(), &icons.idle) {
            Ok(mut tray) => {
                tray.set_directories(&self.recent_dirs, self.bound.as_deref());
                self.tray = Some(tray);
            }
            Err(err) => eprintln!("could not create the status item: {err:#}"),
        }
        self.icons = Some(icons);
        match Hotkeys::new(&self.config.shortcuts, self.config.agent_delivery == AgentDelivery::Batch, false) {
            Ok(hotkeys) => {
                if let Some(err) = hotkeys.errors.first() {
                    self.error = Some((Instant::now(), err.clone()));
                }
                self.hotkeys = Some(hotkeys);
            }
            Err(err) => self.error = Some((Instant::now(), format!("Global shortcuts unavailable: {err}"))),
        }
        environment::track_browsers();
        // Ask for the microphone up front, while the user is at the terminal, not mid-bug.
        if self.mic == Mic::Undetermined && !cfg!(feature = "dev-hooks") {
            self.request_mic();
        }
        self.models.ensure(&self.config.model);
        self.watch_config();
        let proxy = self.proxy.clone();
        watch::sessions(paths::listeners_dir(), paths::sessions_dir(), move |listeners, sessions| {
            proxy.send_event(UserEvent::Sessions(listeners, sessions)).is_ok()
        });
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::Agents(terminal::installed_agents()));
        });
        self.sync_wispr();
        *self.threads_dir.lock().unwrap() = self.bound.clone();
        let proxy = self.proxy.clone();
        watch::threads(self.threads_dir.clone(), move |note, events| {
            proxy.send_event(UserEvent::Threads(note, events)).is_ok()
        });
        self.link_command();
        self.refresh_latest();
        self.refresh();
        self.publish();
    }

    /// The first time a copy installed by hand runs, puts the `asides` command on the PATH, as the
    /// Homebrew cask does: /usr/local/bin/asides → the CLI in the bundle. Skipped when an `asides`
    /// is already set up, or once the user declined the password prompt.
    fn link_command(&self) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        // …/Asides.app/Contents/MacOS/Asides
        let app = std::env::current_exe().ok().and_then(|exe| exe.ancestors().nth(3).map(Path::to_path_buf));
        let Some(cli) = app.and_then(|app| install::installed_cli(&app, &home)) else { return };
        if self.command_link_declined || install::command_installed(&install::command_dirs(&home)) {
            return;
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::CommandLinked(macos::link_command(&cli)));
        });
    }

    fn handle(&mut self, event: UserEvent) {
        match event {
            UserEvent::HotKey(event) => self.on_hotkey(event),
            UserEvent::Menu(event) => self.on_menu(event),
            UserEvent::Work(outcome) => self.on_outcome(outcome),
            UserEvent::Model(id, state) => {
                if id == ModelSpec::find(&self.config.model).id {
                    self.model_state = Some(state);
                    self.refresh();
                    self.publish();
                }
            }
            UserEvent::Mic(granted) => {
                self.mic = if granted { Mic::Granted } else { Mic::Denied };
                self.refresh();
                self.publish();
            }
            UserEvent::ConfigFileChanged => self.reload_config(),
            UserEvent::Wispr(Ok(dictation)) => self.add_dictation(dictation),
            UserEvent::Wispr(Err(message)) => self.fail(message),
            UserEvent::Sessions(listeners, sessions) => self.on_sessions(listeners, sessions),
            UserEvent::Agents(agents) => {
                self.agents = Some(agents);
                self.refresh();
            }
            UserEvent::Launched(Ok(said)) => {
                let status = said.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("Started").to_string();
                self.flash = Some((Instant::now() + SENT_FLASH, "Started".into(), status));
                self.refresh();
            }
            UserEvent::Launched(Err(message)) => self.fail(message),
            UserEvent::CommandLinked(CommandLink::Linked) => {
                let status = format!("You can now run `asides` in any terminal ({})", install::LINK);
                self.flash = Some((Instant::now() + SENT_FLASH, "asides".into(), status));
                self.refresh();
            }
            UserEvent::CommandLinked(CommandLink::Declined) => {
                self.command_link_declined = true;
                self.publish();
            }
            UserEvent::CommandLinked(CommandLink::Failed(message)) => {
                self.fail(format!("Could not add the asides command to {}: {message}", install::LINK));
            }
            UserEvent::Captured(Ok(captures)) => self.show_overlay(captures),
            UserEvent::Captured(Err(message)) => {
                self.capturing = None;
                self.fail(message);
            }
            UserEvent::Overlay(message) => self.on_overlay(message),
            UserEvent::Threads(note, events) => self.on_threads(note, events),
        }
    }

    fn handle_link(&mut self, url: &url::Url) {
        match Link::parse(url) {
            Some(Link::Bind { dir, new_note, title, request }) => {
                self.last_request = request;
                // A session there gets what's said there; otherwise it's notes only.
                self.target = self.sessions.iter().rev().find(|s| s.dir == dir).map(|s| s.id.clone());
                self.bind(dir, new_note.then_some(PendingNew { title }));
            }
            Some(Link::Session { id, request }) => {
                self.last_request = request;
                match Session::read(&paths::sessions_dir(), &id) {
                    Some(session) => {
                        self.known_sessions.insert(id.clone());
                        if !self.sessions.iter().any(|s| s.id == id) {
                            self.sessions.push(session);
                            self.sync_session_hotkey();
                        }
                        self.set_target(Some(id));
                    }
                    None => self.fail("An agent session asked for your asides, but it's gone.".into()),
                }
                self.publish();
            }
            Some(Link::Quit) => self.request_quit(),
            Some(Link::Send { request }) => {
                self.last_request = request;
                self.send_to_agent();
                self.publish();
            }
            Some(Link::Simulate { wav, new_note, image, image_at, request }) => {
                self.last_request = request;
                #[cfg(feature = "dev-hooks")]
                if let Err(err) = self.simulate(&wav, new_note, image, image_at) {
                    self.fail(format!("simulate: {err:#}"));
                }
                #[cfg(not(feature = "dev-hooks"))]
                {
                    let _ = (wav, new_note, image, image_at);
                    self.fail("This build does not support `asides simulate`.".into());
                }
                self.publish();
            }
            Some(Link::Measure { request }) => {
                self.last_request = request;
                self.measure();
                self.publish();
            }
            Some(Link::SimulateMeasure { image, script, request }) => {
                self.last_request = request;
                #[cfg(feature = "dev-hooks")]
                if let Err(err) = self.simulate_measure(&image, &script) {
                    self.fail(format!("simulate-measure: {err:#}"));
                }
                #[cfg(not(feature = "dev-hooks"))]
                {
                    let _ = (image, script);
                    self.fail("This build does not support `asides simulate-measure`.".into());
                }
                self.publish();
            }
            None => {}
        }
    }

    fn on_hotkey(&mut self, event: GlobalHotKeyEvent) {
        let Some(action) = self.hotkeys.as_ref().and_then(|h| h.action(event.id)) else { return };
        let is_record = matches!(action, Action::Record | Action::RecordNewNote);
        match event.state {
            HotKeyState::Pressed => {
                // Ignore key repeat while a record shortcut is held down.
                if is_record && !self.keys_down.insert(event.id) {
                    return;
                }
                match action {
                    Action::Record | Action::RecordNewNote if self.recording.is_some() => self.stop_recording(),
                    Action::Record => self.start_recording(false, Some((action, Instant::now()))),
                    Action::RecordNewNote => self.start_recording(true, Some((action, Instant::now()))),
                    Action::AttachImage => self.attach_image(),
                    Action::Cancel => self.cancel_recording(),
                    Action::SendToAgent => self.send_to_agent(),
                    Action::Measure => self.measure(),
                    Action::NextTarget => self.next_target(),
                }
            }
            HotKeyState::Released => {
                self.keys_down.remove(&event.id);
                let held = self
                    .recording
                    .as_ref()
                    .and_then(|r| r.pressed)
                    .is_some_and(|(a, at)| a == action && at.elapsed() >= TAP_MAX);
                if self.config.hold_to_talk && held {
                    self.stop_recording();
                }
            }
        }
    }

    fn on_menu(&mut self, event: MenuEvent) {
        let Some(action) = self.tray.as_ref().and_then(|t| t.action(&event.id)) else { return };
        let checked = self.tray.as_ref().is_some_and(|t| t.checked(&action));
        match action {
            MenuAction::Record if self.recording.is_some() => self.stop_recording(),
            MenuAction::Record => self.start_recording(false, None),
            MenuAction::RecordNewNote => self.start_recording(true, None),
            MenuAction::AttachImage => self.attach_image(),
            MenuAction::Measure => self.measure(),
            MenuAction::Cancel => self.cancel_recording(),
            MenuAction::SendToAgent => self.send_to_agent(),
            MenuAction::OpenReplies => self.with_latest(macos::open),
            MenuAction::Target(id) => self.set_target(id),
            MenuAction::NewSession { agent, dir, background } => self.new_session(agent, dir, background),
            MenuAction::ShowSession(id) => self.show_session(&id),
            MenuAction::StopSession(id) => self.stop_session(&id),
            MenuAction::SwitchDirectory(dir) => self.bind_from_menu(&dir),
            MenuAction::ChooseDirectory => {
                if let Some(dir) = macos::choose_directory(self.mtm, self.bound.as_deref()) {
                    self.bind_from_menu(&dir);
                }
            }
            MenuAction::OpenNote => self.with_latest(macos::open),
            MenuAction::RevealNote => self.with_latest(macos::reveal),
            MenuAction::CopyNotePath => self.with_latest(|file| clipboard::copy_text(&file.to_string_lossy())),
            MenuAction::ToggleHoldToTalk => self.update_config(|c| c.hold_to_talk = checked),
            MenuAction::ToggleAutoAttach => self.update_config(|c| c.auto_attach_images = checked),
            MenuAction::ToggleSounds => self.update_config(|c| c.sounds = checked),
            MenuAction::ToggleKeepAudio => self.update_config(|c| c.keep_audio = checked),
            MenuAction::ToggleWisprFlow => {
                self.update_config(|c| c.wispr_flow = checked);
                self.sync_wispr();
            }
            MenuAction::ToggleIdentifyElements => {
                self.update_config(|c| c.measure.identify_elements = checked);
                if checked && !elements::trusted() {
                    elements::request_trust();
                    let status = "Allow Asides in System Settings → Privacy & Security → Accessibility to name the \
                                  elements you mark";
                    self.flash = Some((Instant::now() + SENT_FLASH, "Accessibility".into(), status.into()));
                    self.refresh();
                }
            }
            MenuAction::ToggleMeasurePixels => {
                self.update_config(|c| c.measure.units = if checked { Units::Px } else { Units::Pt });
            }
            MenuAction::AnnotationDetail(detail) => self.update_config(|c| c.annotation_detail = detail),
            MenuAction::ScreenRecordingSettings => {
                if !capture::permitted() && !capture::request() {
                    capture::open_settings();
                }
            }
            MenuAction::AccessibilitySettings => {
                if !elements::request_trust() {
                    elements::open_settings();
                }
            }
            MenuAction::ToggleMeasureRecord => self.update_config(|c| c.measure.record = checked),
            MenuAction::AgentDelivery(mode) => {
                self.update_config(|c| c.agent_delivery = mode);
                self.apply_hotkeys();
                self.refresh();
            }
            MenuAction::Terminal(name) => self.update_config(|c| c.agents.terminal = name.clone()),
            MenuAction::Model(id) => {
                self.update_config(|c| c.model = id.clone());
                self.model_state = None;
                self.models.ensure(&id);
            }
            MenuAction::InputDevice(name) => self.update_config(|c| c.input_device = name.clone()),
            MenuAction::MicrophoneSettings => {
                if self.mic == Mic::Undetermined {
                    self.request_mic();
                } else {
                    macos::open_microphone_settings();
                }
            }
            MenuAction::EditSettings => macos::open_in_text_editor(&paths::config_path()),
            MenuAction::Quit => self.request_quit(),
        }
    }

    fn on_outcome(&mut self, outcome: Outcome) {
        self.in_flight = self.in_flight.saturating_sub(1);
        let now = Instant::now();
        match outcome {
            Outcome::Appended { note, number } => {
                self.error = None;
                self.flash = Some((now + FLASH, format!("#{number}"), format!("Added #{number} to {note}")));
            }
            Outcome::ImageAdded { note } => {
                self.flash = Some((now + FLASH, "📎".into(), format!("Image added to {note}")));
            }
            Outcome::NothingHeard => {
                self.flash = Some((now + FLASH, "–".into(), "Nothing heard; nothing was written".into()));
            }
            Outcome::Failed(message) => self.fail(message),
        }
        self.refresh_latest();
        self.flush_send();
        self.refresh();
        self.publish();
        if self.quit_when_idle && self.in_flight == 0 && self.recording.is_none() {
            self.quit = true;
        }
    }

    /// Points future recordings at `dir` (a recording in progress keeps its own directory).
    fn bind(&mut self, dir: PathBuf, pending_new: Option<PendingNew>) {
        self.recent_dirs.retain(|d| d.is_dir());
        app_state::remember_dir(&mut self.recent_dirs, &dir);
        *self.threads_dir.lock().unwrap() = Some(dir.clone());
        self.bound = Some(dir);
        self.pending_new = pending_new;
        self.error = None;
        if let Some(tray) = &mut self.tray {
            tray.set_directories(&self.recent_dirs, self.bound.as_deref());
        }
        self.refresh_latest();
        self.refresh();
        self.publish();
    }

    /// Binding chosen in the menu: same rules as the CLI (repo root, local git exclude).
    fn bind_from_menu(&mut self, chosen: &Path) {
        let dir = match project::resolve_bind_dir(chosen) {
            Ok(dir) => dir,
            Err(err) => {
                self.recent_dirs.retain(|d| d != chosen);
                if let Some(tray) = &mut self.tray {
                    tray.set_directories(&self.recent_dirs, self.bound.as_deref());
                }
                return self.fail(format!("Cannot use {}: {err}", tilde(chosen)));
            }
        };
        if let Err(err) = project::ensure_git_exclude(&dir) {
            eprintln!("could not update .git/info/exclude: {err}");
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        self.target = self.sessions.iter().rev().find(|s| s.dir == dir).map(|s| s.id.clone());
        self.bind(dir.clone(), None);
        self.flash = Some((Instant::now() + FLASH, name, format!("Now writing to {}", tilde(&dir))));
        self.refresh();
    }

    fn start_recording(&mut self, force_new: bool, pressed: Option<(Action, Instant)>) {
        let Some(dir) = self.bound.clone() else {
            return self.fail("Not bound to a directory. Run `asides` in a repo first.".into());
        };
        if !dir.is_dir() {
            return self.fail(format!("{} no longer exists. Run `asides` again.", dir.display()));
        }
        match macos::mic_status() {
            Mic::Granted => self.mic = Mic::Granted,
            Mic::Undetermined => {
                if let Some(overlay) = &mut self.overlay {
                    let text = "Asides needs the microphone: close this (Esc) to answer macOS's prompt.".into();
                    overlay.send(&OverlayCommand::Notice { text });
                }
                return self.request_mic();
            }
            Mic::Denied => {
                self.mic = Mic::Denied;
                return self.fail("Microphone access is denied. Preferences → Microphone Permission…".into());
            }
        }
        let recorder = match audio::Recorder::start(&self.config.input_device) {
            Ok(recorder) => recorder,
            Err(err) => return self.fail(format!("{err:#}")),
        };
        let pending = self.pending_new.take();
        let new_note = (force_new || pending.is_some()).then(|| pending.and_then(|p| p.title));
        let target_label = match (&new_note, &self.latest) {
            (Some(Some(title)), _) => format!("new note \"{title}\""),
            (Some(None), _) | (None, None) => "new note".into(),
            (None, Some(latest)) => latest.title.clone(),
        };
        let route = self.route(&dir);
        let to_agent = self.target_session().filter(|s| s.dir == dir).map(|s| s.agent.clone());
        self.recording = Some(Recording {
            recorder,
            started_at: Instant::now(),
            started: Local::now(),
            dir,
            new_note,
            target_label,
            route,
            to_agent,
            images: Vec::new(),
            texts: Vec::new(),
            environment: environment::capture(self.mtm),
            pressed,
            clipboard: clipboard::change_count(),
        });
        // While Measure & Annotate is open, Esc belongs to it, and a recording started then (with
        // its talk key, the microphone, or the shortcut) ends with it.
        if let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.set_cancel_active(self.overlay.is_none());
        }
        if let Some(overlay) = &mut self.overlay {
            overlay.started_recording = true;
        }
        self.tell_overlay_recording();
        self.error = None;
        self.flash = None;
        self.sound("Tink");
        self.refresh();
        self.publish();
    }

    fn stop_recording(&mut self) {
        let Some(recording) = self.recording.take() else { return };
        if let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.set_cancel_active(false);
        }
        self.tell_overlay_recording();
        let (samples, sample_rate) = recording.recorder.finish();
        self.sound("Pop");
        self.sync_session_hotkey();
        let seconds = samples.len() as f32 / sample_rate.max(1) as f32;
        if seconds < MIN_RECORDING_SECS && recording.images.is_empty() && recording.texts.is_empty() {
            self.refresh();
            self.publish();
            return;
        }
        self.submit(Job::Recording(Box::new(RecordingJob {
            dir: recording.dir,
            samples,
            sample_rate,
            started: recording.started,
            new_note: recording.new_note,
            images: recording.images,
            texts: recording.texts,
            environment: recording.environment,
            keep_audio: self.config.keep_audio,
            model: self.config.model.clone(),
            languages: self.config.languages.clone(),
            auto_new_note: self.config.auto_new_note,
            route: recording.route,
        })));
    }

    fn cancel_recording(&mut self) {
        let Some(recording) = self.recording.take() else { return };
        if let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.set_cancel_active(false);
        }
        self.tell_overlay_recording();
        drop(recording.recorder.finish());
        // A cancelled "new note" request stays pending for the next recording.
        if let Some(title) = recording.new_note {
            self.pending_new = Some(PendingNew { title });
        }
        self.sound("Funk");
        self.flash = Some((Instant::now() + FLASH, "✕".into(), "Recording cancelled".into()));
        self.refresh();
        self.publish();
    }

    fn attach_image(&mut self) {
        let Some(image) = clipboard::read_image() else {
            return self.fail("The clipboard has no image. Copy a screenshot first (⌃⇧⌘4).".into());
        };
        if let Some(recording) = &mut self.recording {
            recording.images.push(CapturedImage {
                offset: recording.started_at.elapsed().as_secs_f32(),
                at: Local::now(),
                bytes: image.bytes,
                extension: image.extension,
                details: None,
            });
            recording.clipboard = clipboard::change_count();
            self.sound("Tink");
            self.refresh();
            return;
        }
        let Some(dir) = self.bound.clone() else {
            return self.fail("Not bound to a directory. Run `asides` in a repo first.".into());
        };
        let new_note = self.pending_new.take().map(|p| p.title);
        self.submit(Job::Image(Box::new(ImageJob {
            dir,
            image: CapturedImage {
                offset: 0.0,
                at: Local::now(),
                bytes: image.bytes,
                extension: image.extension,
                details: None,
            },
            new_note,
            environment: environment::capture(self.mtm),
            model: self.config.model.clone(),
            auto_new_note: self.config.auto_new_note,
        })));
    }

    /// A Wispr Flow dictation becomes an entry, like a recording.
    fn add_dictation(&mut self, dictation: Dictation) {
        if !wispr::app_matches(dictation.app.as_deref(), &self.config.wispr_flow_apps) {
            return;
        }
        let Some(dir) = self.bound.clone() else {
            return self
                .fail("A Wispr Flow dictation arrived, but no directory is bound. Run `asides` in a repo.".into());
        };
        if !dir.is_dir() {
            return self.fail(format!("{} no longer exists. Run `asides` again.", dir.display()));
        }
        let new_note = self.pending_new.take().map(|p| p.title);
        let route = self.route(&dir);
        self.submit(Job::Dictation(Box::new(DictationJob {
            route,
            dir,
            text: dictation.text,
            at: dictation.at.with_timezone(&Local),
            source: "Wispr Flow",
            new_note,
            environment: environment::capture(self.mtm),
            auto_new_note: self.config.auto_new_note,
        })));
    }

    /// Measure & Annotate: freezes every screen (asking for the Screen Recording permission the
    /// first time). The overlay opens once the screenshots arrive. Pressed while it's open, the
    /// shortcut adds what's there to the aside.
    fn measure(&mut self) {
        if let Some(overlay) = &mut self.overlay {
            return overlay.send(&OverlayCommand::Finish);
        }
        // A capture that never answered doesn't block the next one for long.
        if self.capturing.as_ref().is_some_and(|(at, ..)| at.elapsed() < CAPTURE_TIMEOUT) {
            return;
        }
        if !capture::permitted() {
            capture::request();
            return self.fail(
                "Measure & Annotate needs to see the screen: allow Asides in System Settings → Privacy & Security → \
                 Screen & System Audio Recording, then quit and reopen Asides."
                    .into(),
            );
        }
        let screens = capture::screens(self.mtm);
        let displays = screens
            .iter()
            .map(|s| {
                let px = |pt: f64| (pt * s.scale).round() as usize;
                (s.display, px(s.frame.size.width), px(s.frame.size.height))
            })
            .collect();
        // The windows as they are now, before the overlay covers them.
        self.capturing = Some((Instant::now(), screens, elements::windows()));
        let proxy = Mutex::new(self.proxy.clone());
        capture::capture(displays, move |result| {
            let _ = proxy.lock().unwrap().send_event(UserEvent::Captured(result));
        });
    }

    fn show_overlay(&mut self, captures: Vec<Capture>) {
        let Some((_, screens, windows)) = self.capturing.take() else { return };
        let mut captures: Vec<Option<Capture>> = captures.into_iter().map(Some).collect();
        let mut frames = Vec::new();
        let mut spec_screens = Vec::new();
        for s in screens {
            let Some(capture) =
                captures.iter_mut().find(|c| c.as_ref().is_some_and(|c| c.display == s.display)).and_then(Option::take)
            else {
                continue;
            };
            let (w, h) = (s.frame.size.width, s.frame.size.height);
            spec_screens.push(SpecScreen {
                display: s.display,
                screen: Screen { name: s.name, width: w, height: h, scale: s.scale },
                origin: s.origin,
                frame: PtRect::new(s.frame.origin.x, s.frame.origin.y, w, h),
                pixels: String::new(),
                width: capture.frame.width,
                height: capture.frame.height,
            });
            frames.push(capture.frame);
        }
        if spec_screens.is_empty() {
            return self.fail("Could not capture the screen.".into());
        }
        if self.config.measure.identify_elements && !elements::trusted() {
            elements::request_trust();
        }
        let tips = !self.overlay_tips_seen;
        let spec = Spec { screens: spec_screens, tips, ..self.overlay_spec() };
        if let Err(err) = self.open_overlay(spec, &frames, windows) {
            return self.fail(format!("Could not open Measure & Annotate: {err:#}"));
        }
        if tips {
            self.overlay_tips_seen = true;
            self.publish();
        }
        if self.config.measure.record && self.recording.is_none() {
            self.start_recording(false, None);
        }
    }

    /// The overlay's settings, from the config.
    fn overlay_spec(&self) -> Spec {
        let m = &self.config.measure;
        Spec {
            color: m.color.clone(),
            units: m.units,
            tolerance: m.tolerance.min(100),
            detail: self.config.annotation_detail,
            padding: m.padding.max(0.0),
            identify: m.identify_elements,
            keys: m.keys.clone(),
            record_shortcut: asides_core::config::pretty_shortcut(&self.config.shortcuts.record),
            recording: self.recording.as_ref().map(|r| r.started_at.elapsed().as_secs_f32()),
            batch: self.config.agent_delivery == AgentDelivery::Batch,
            bound: self
                .bound
                .as_ref()
                .filter(|d| d.is_dir())
                .and_then(|d| d.file_name())
                .map(|n| n.to_string_lossy().into_owned()),
            toolbar: m.toolbar,
            saved: self.last_measure.clone(),
            ..Spec::default()
        }
    }

    fn open_overlay(
        &mut self,
        spec: Spec,
        frames: &[asides_core::measure::Frame],
        windows: Vec<WindowInfo>,
    ) -> anyhow::Result<()> {
        let proxy = Mutex::new(self.proxy.clone());
        let notify = move |message| {
            let _ = proxy.lock().unwrap().send_event(UserEvent::Overlay(message));
        };
        let mut overlay = overlay::start(spec, frames, windows, notify)?;
        overlay.previous_app = macos::frontmost_app();
        self.overlay = Some(overlay);
        // Esc belongs to the overlay while it's open.
        if let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.set_cancel_active(false);
        }
        Ok(())
    }

    /// Tells the overlay whether a recording is under way (its toolbar shows it).
    fn tell_overlay_recording(&mut self) {
        let seconds = self.recording.as_ref().map(|r| r.started_at.elapsed().as_secs_f32());
        if let Some(overlay) = &mut self.overlay {
            overlay.send(&OverlayCommand::Recording { seconds });
        }
    }

    fn on_overlay(&mut self, message: OverlayMessage) {
        let Some(overlay) = &mut self.overlay else { return };
        match message {
            OverlayMessage::Event(OverlayEvent::Ready) => overlay.opened = Some(Instant::now()),
            OverlayMessage::Event(OverlayEvent::Identify { id, x, y }) => {
                let deep = self.config.measure.identify_elements;
                let element = elements::identify(&overlay.windows, x, y, deep);
                overlay.send(&OverlayCommand::Element { id, element: element.map(Box::new) });
            }
            OverlayMessage::Event(OverlayEvent::Talk { down }) => self.talk(down),
            OverlayMessage::Event(OverlayEvent::Toolbar { shown }) => self.update_config(|c| c.measure.toolbar = shown),
            OverlayMessage::Event(OverlayEvent::Copied) => {
                // Not a screenshot to attach: it's already part of the session.
                if let Some(recording) = &mut self.recording {
                    recording.clipboard = clipboard::change_count();
                }
            }
            OverlayMessage::Event(OverlayEvent::Finished) => {
                overlay.over = true;
                let started_recording = overlay.started_recording;
                let opened_offset = self.recording.as_ref().map(|r| overlay.offset_from(r.started_at));
                match overlay.result() {
                    Ok((outcome, images)) => {
                        self.last_measure = outcome.saved.clone();
                        let send = outcome.send;
                        self.add_inspection(outcome, images, opened_offset);
                        // A recording started in the overlay ends with it.
                        if started_recording || send {
                            self.stop_recording();
                        }
                        if send {
                            self.send_to_agent();
                        }
                    }
                    Err(err) => self.fail(format!("Measure & Annotate: {err:#}")),
                }
            }
            OverlayMessage::Event(OverlayEvent::Cancelled { saved }) => {
                overlay.over = true;
                if !saved.is_empty() {
                    self.last_measure = saved;
                }
                if overlay.started_recording {
                    self.cancel_recording();
                }
            }
            OverlayMessage::Exited => {
                let crashed = !overlay.over;
                let previous = overlay.previous_app;
                self.overlay = None;
                if let Some(hotkeys) = &mut self.hotkeys {
                    hotkeys.set_cancel_active(self.recording.is_some());
                }
                if let Some(pid) = previous {
                    macos::activate_app(pid);
                }
                if crashed {
                    self.fail("Measure & Annotate closed unexpectedly.".into());
                }
                self.refresh();
            }
        }
    }

    /// The overlay's talk key or microphone button, like the record shortcut: a tap toggles, a
    /// longer press records until it's let go.
    fn talk(&mut self, down: bool) {
        if down {
            if self.recording.is_some() {
                return self.stop_recording();
            }
            return self.start_recording(false, Some((Action::Record, Instant::now())));
        }
        let held = self
            .recording
            .as_ref()
            .and_then(|r| r.pressed)
            .is_some_and(|(a, at)| a == Action::Record && at.elapsed() >= TAP_MAX);
        if self.config.hold_to_talk && held {
            self.stop_recording();
        }
    }

    /// A finished Measure & Annotate session: into the recording in progress, at the moments it
    /// happened (so marks sit among what was said about them), or as an entry of its own.
    /// `opened_offset` is when the overlay opened, in seconds into the recording.
    fn add_inspection(&mut self, outcome: OverlayOutcome, images: Vec<Option<Vec<u8>>>, opened_offset: Option<f32>) {
        let at = Local::now();
        let mut pictures = Vec::new();
        let mut texts = Vec::new();
        for (item, bytes) in outcome.items.into_iter().zip(images) {
            let offset = opened_offset.map_or(item.time, |start| (start + item.time).max(0.0));
            match bytes {
                Some(bytes) => {
                    pictures.push(CapturedImage { offset, at, bytes, extension: "png".into(), details: item.text })
                }
                None => texts.extend(item.text.map(|text| (offset, text))),
            }
        }
        if let Some(recording) = &mut self.recording {
            recording.images.extend(pictures);
            recording.texts.extend(texts);
            self.sound("Tink");
            return self.refresh();
        }
        let Some(dir) = self.bound.clone().filter(|d| d.is_dir()) else {
            return self.fail("Not bound to a directory. Run `asides` in a repo first.".into());
        };
        let new_note = self.pending_new.take().map(|p| p.title);
        let route = self.route(&dir);
        self.submit(Job::Screen(Box::new(ScreenJob {
            route,
            dir,
            at,
            images: pictures,
            texts,
            new_note,
            environment: environment::capture(self.mtm),
            model: self.config.model.clone(),
            auto_new_note: self.config.auto_new_note,
        })));
    }

    /// An agent resolved, dismissed, acknowledged or replied to an aside: say so in the menu bar,
    /// and list it under Agent Replies.
    fn on_threads(&mut self, note: PathBuf, events: Vec<ThreadEvent>) {
        let seen = match &self.threads_seen {
            Some((dir, seen)) if *dir == note => (*seen).min(events.len()),
            // A note we just started watching: what's there already isn't news.
            _ => events.len(),
        };
        if let Some(event) = events[seen..].iter().rev().find(|e| e.by == Role::Agent) {
            let who = event.author.clone().unwrap_or_else(|| "An agent".into());
            let id = event.aside;
            let (title, verb) = match event.status {
                Some(Status::Resolved) => (format!("✓ #{id}"), "resolved"),
                Some(Status::Dismissed) => (format!("– #{id}"), "dismissed"),
                Some(Status::Acknowledged) => (format!("#{id}"), "is on"),
                Some(Status::Pending) => (format!("#{id}"), "reopened"),
                None => (format!("💬 #{id}"), "replied to"),
            };
            let mut status = format!("{who} {verb} #{id}");
            if let Some(message) = &event.message {
                status.push_str(&format!(": {}", short(message, 90)));
            }
            self.flash = Some((Instant::now() + SENT_FLASH, title, status));
            self.sound("Purr");
        }
        self.replies = events
            .iter()
            .rev()
            .filter(|e| e.by == Role::Agent)
            .take(MAX_REPLIES)
            .map(|e| {
                let mark = match e.status {
                    Some(Status::Resolved) => "✓ ",
                    Some(Status::Dismissed) => "– ",
                    None => "💬 ",
                    _ => "",
                };
                let who = e.author.clone().unwrap_or_else(|| "Agent".into());
                let text = match (&e.message, e.status) {
                    (Some(m), _) => short(m, 70),
                    (None, Some(s)) => s.as_str().to_string(),
                    (None, None) => String::new(),
                };
                format!("#{} {mark}{who}: {text}", e.aside)
            })
            .collect();
        self.threads_seen = Some((note, events.len()));
        self.refresh_latest();
        self.refresh();
    }

    /// Hands what's been recorded so far to agents listening in batch mode. A recording in
    /// progress is stopped first, and the batch goes out once every recording is transcribed.
    /// With no agent listening, nothing is sent: the batch waits for the next send.
    fn send_to_agent(&mut self) {
        if self.bound.is_none() {
            return self.fail("Not bound to a directory. Run `asides` in a repo first.".into());
        }
        if self.recording.is_some() {
            self.stop_recording();
        }
        self.pending_send = true;
        self.flush_send();
    }

    fn flush_send(&mut self) {
        if !self.pending_send || self.in_flight > 0 {
            return;
        }
        self.pending_send = false;
        let Some(dir) = self.bound.clone() else { return };
        let Some(session) = self.target_session().filter(|s| s.dir == dir).cloned() else {
            return self.fail(
                "No agent is listening. Start one (New Agent Session), or ask your coding agent to listen to your \
                 asides, then send again."
                    .into(),
            );
        };
        let latest = notes::latest_note(&dir);
        let send = AgentSend {
            at_ms: now_ms(),
            entry: latest.as_ref().map_or(0, |n| n.next_entry_number() - 1),
            note: latest.map(|n| n.dir),
            dir,
            session: Some(session.id.clone()),
        };
        app_state::remember_send(&mut self.agent_sends, send);
        let (title, status) = match session.state(&self.listeners, now_ms()) {
            SessionState::Listening => ("Sent", format!("Sent to {}", session.agent)),
            _ => ("Queued", format!("Queued: {} gets them when it's ready", session.agent)),
        };
        self.flash = Some((Instant::now() + SENT_FLASH, title.into(), status));
        self.sound("Glass");
        self.refresh();
        self.publish();
    }

    /// The session the user's asides go to, if it's still around.
    fn target_session(&self) -> Option<&Session> {
        self.target.as_ref().and_then(|id| self.sessions.iter().find(|s| &s.id == id))
    }

    /// Where an aside spoken now in `dir` goes, to write beside the note: the target session, or
    /// nobody (Notes Only) when sessions are listening there. `None` when there's no session to
    /// tell apart, as before sessions.
    fn route(&self, dir: &Path) -> Option<Route> {
        match self.target_session().filter(|s| s.dir == dir) {
            Some(session) => {
                Some(Route { entry: 0, session: Some(session.id.clone()), agent: Some(session.agent.clone()) })
            }
            None if self.sessions.iter().any(|s| s.dir == dir) => Some(Route::default()),
            None => None,
        }
    }

    /// The agent sessions changed: a new one becomes the target (the user just started it, or
    /// asked its agent to listen), and when the target ends, the newest other session in its repo
    /// takes over, or the notes only.
    fn on_sessions(&mut self, listeners: Vec<Listener>, all: Vec<Session>) {
        self.listeners = listeners;
        let live: Vec<Session> = all.into_iter().filter(|s| s.listening).collect();
        let first = !std::mem::replace(&mut self.sessions_seen, true);
        let newest = live.iter().rev().find(|s| !self.known_sessions.contains(&s.id)).map(|s| s.id.clone());
        let ended = self.target_session().filter(|t| !live.iter().any(|s| s.id == t.id)).cloned();
        self.known_sessions.extend(live.iter().map(|s| s.id.clone()));
        self.sessions = live;
        self.sync_session_hotkey();
        if first {
            // What was there before the app started: keep the target it had, if it's still around.
            if self.target_session().is_none() {
                self.target = None;
            }
            self.publish();
        } else if let Some(id) = newest {
            return self.set_target(Some(id));
        } else if let Some(gone) = ended {
            // Another session in that repo takes over; never one elsewhere, unannounced.
            let next = self.sessions.iter().rev().find(|s| s.dir == gone.dir).map(|s| s.id.clone());
            let status = match &next {
                Some(id) => format!("{} ended: your asides go to {}", gone.agent, self.label_of(id)),
                None => format!("{} ended: your asides are saved to the notes only", gone.agent),
            };
            self.target = next;
            self.flash = Some((Instant::now() + SENT_FLASH, "Ended".into(), status));
            self.publish();
        }
        self.refresh();
    }

    /// Registers the shortcut that switches sessions while there are some, and frees it otherwise;
    /// not mid-recording, where re-registering the shortcuts could lose the record key's release.
    fn sync_session_hotkey(&mut self) {
        if self.next_target_on != !self.sessions.is_empty() && self.recording.is_none() {
            self.apply_hotkeys();
        }
    }

    /// Sends the user's asides to session `id` (binding to its repo), or to the notes only.
    fn set_target(&mut self, id: Option<String>) {
        let session = id.as_ref().and_then(|id| self.sessions.iter().find(|s| &s.id == id)).cloned();
        self.target = session.as_ref().map(|s| s.id.clone());
        let (title, status) = match &session {
            Some(session) => {
                if self.bound.as_ref() != Some(&session.dir) {
                    self.bind(session.dir.clone(), None);
                }
                (format!("→ {}", session.agent), format!("Your asides go to {}", self.label_of(&session.id)))
            }
            None => ("Notes".to_string(), "Your asides are saved to the notes only".to_string()),
        };
        self.flash = Some((Instant::now() + SENT_FLASH, title, status));
        self.refresh();
        self.publish();
    }

    /// Switches to the next session, then Notes Only, then the first session again.
    fn next_target(&mut self) {
        let mut choices: Vec<Option<String>> = self.sessions.iter().map(|s| Some(s.id.clone())).collect();
        choices.push(None);
        let at = choices.iter().position(|c| *c == self.target).unwrap_or(choices.len() - 1);
        self.set_target(choices[(at + 1) % choices.len()].clone());
    }

    /// The menu's name for session `id`, telling apart sessions of one agent in one repo.
    fn label_of(&self, id: &str) -> String {
        let labels = sessions::labels(&self.sessions);
        self.sessions.iter().zip(labels).find(|(s, _)| s.id == id).map(|(_, l)| l).unwrap_or_default()
    }

    /// New Agent Session: `asides <agent>` in a terminal window, or in the background.
    fn new_session(&mut self, agent: AgentKind, dir: Option<PathBuf>, background: bool) {
        let chosen = match dir {
            Some(dir) => dir,
            None => match macos::choose_directory(self.mtm, self.bound.as_deref()) {
                Some(dir) => dir,
                None => return,
            },
        };
        let dir = match project::resolve_bind_dir(&chosen) {
            Ok(dir) => dir,
            Err(err) => return self.fail(format!("Cannot use {}: {err}", tilde(&chosen))),
        };
        let Some(cli) = terminal::cli() else {
            return self.fail("The asides command isn't in the app bundle. Reinstall Asides.".into());
        };
        let mut command = format!(
            "{} -C {} {}",
            terminal::quote(&cli.to_string_lossy()),
            terminal::quote(&dir.to_string_lossy()),
            agent.program()
        );
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if background {
            command.push_str(" --background");
        }
        let command = terminal::from_this_app(&command);
        if background {
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                let result = terminal::run(&command).map_err(|e| format!("Could not start the session: {e:#}"));
                let _ = proxy.send_event(UserEvent::Launched(result));
            });
        } else {
            let title = format!("{} · {name}", agent.name());
            if let Err(err) = terminal::open(&terminal::app(&self.config.agents.terminal), &dir, &command, &title) {
                return self.fail(format!("{err:#}"));
            }
        }
        let status = format!("Starting {} in {name}…", agent.name());
        self.flash = Some((Instant::now() + SENT_FLASH, agent.name().into(), status));
        self.refresh();
    }

    /// Opens a background session in a terminal (`claude attach`).
    fn show_session(&mut self, id: &str) {
        let Some(session) = self.sessions.iter().find(|s| s.id == id).cloned() else { return };
        let command = match (&session.thread, &session.background) {
            (Some(thread), _) => format!("codex resume {}", terminal::quote(thread)),
            (None, Some(name)) => format!("claude attach {}", terminal::quote(name)),
            (None, None) => return,
        };
        // Where it works: Codex asks which directory to use when it's opened elsewhere.
        let dir = match (&session.thread, &session.worktree) {
            (Some(_), Some(tree)) => project::worktree_path(&session.dir, tree),
            _ => session.dir.clone(),
        };
        let terminal = terminal::app(&self.config.agents.terminal);
        if let Err(err) = terminal::open(&terminal, &dir, &command, &self.label_of(id)) {
            self.fail(format!("{err:#}"));
        }
    }

    /// Ends a background session: Claude Code's with `claude stop` (its conversation is kept), a
    /// Codex one by stopping what pushes asides into its thread (which `codex resume` still opens).
    fn stop_session(&mut self, id: &str) {
        let Some(session) = self.sessions.iter().find(|s| s.id == id).cloned() else { return };
        let Some(name) = session.background else { return };
        if session.thread.is_some() {
            if let Ok(pid) = libc::pid_t::try_from(session.pid)
                && pid > 0
            {
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        } else {
            let command = format!("claude stop {}", terminal::quote(&name));
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                if let Err(err) = terminal::run(&command) {
                    let _ = proxy.send_event(UserEvent::Launched(Err(format!("Could not stop the session: {err:#}"))));
                }
            });
        }
        let status = format!("Stopping {}…", self.label_of(id));
        self.flash = Some((Instant::now() + SENT_FLASH, "Stop".into(), status));
        self.refresh();
    }

    /// Starts or stops watching Wispr Flow's history to match the settings.
    fn sync_wispr(&mut self) {
        let source = self.wispr_source();
        if source.is_some() == self.wispr.is_some() {
            return;
        }
        if let Some(stop) = self.wispr.take() {
            stop.store(true, Ordering::Relaxed);
        }
        if let Some(db) = source {
            let proxy = self.proxy.clone();
            self.wispr = Some(watch::wispr_flow(db, move |found| proxy.send_event(UserEvent::Wispr(found)).is_ok()));
        }
    }

    fn wispr_source(&self) -> Option<PathBuf> {
        // The end-to-end test stands in for Wispr Flow with a database of its own.
        #[cfg(feature = "dev-hooks")]
        {
            let test = paths::support_dir().join("test-wispr-flow.sqlite");
            if test.is_file() {
                return Some(test);
            }
        }
        self.config.wispr_flow.then(wispr::default_db)
    }

    fn submit(&mut self, job: Job) {
        self.in_flight += 1;
        if self.jobs.send(job).is_err() {
            self.in_flight -= 1;
            self.fail("The transcription worker stopped. Please restart Asides.".into());
        }
        self.refresh();
        self.publish();
    }

    fn tick(&mut self) {
        let auto_attach = self.config.auto_attach_images;
        if let Some(recording) = &mut self.recording {
            let count = clipboard::change_count();
            if auto_attach && count != recording.clipboard {
                recording.clipboard = count;
                if clipboard::has_image()
                    && let Some(image) = clipboard::read_image()
                {
                    recording.images.push(CapturedImage {
                        offset: recording.started_at.elapsed().as_secs_f32(),
                        at: Local::now(),
                        bytes: image.bytes,
                        extension: image.extension,
                        details: None,
                    });
                    self.sound("Tink");
                }
            }
        }
        if self.flash.as_ref().is_some_and(|(until, ..)| Instant::now() >= *until) {
            self.flash = None;
        }
        self.refresh();
    }

    fn next_wakeup(&self) -> Option<Instant> {
        let now = Instant::now();
        let error_icon = self.error.as_ref().is_some_and(|(at, _)| now < *at + ERROR_ICON);
        (self.recording.is_some() || self.flash.is_some() || error_icon).then_some(now + TICK)
    }

    fn refresh(&mut self) {
        let Some(icons) = &self.icons else { return };
        let now = Instant::now();
        // A directory deleted since it was bound can't be recorded into either.
        let usable = self.bound.as_ref().filter(|d| d.is_dir());
        let dir_name = usable.and_then(|d| d.file_name()).map(|n| n.to_string_lossy().into_owned());
        let downloading = match &self.model_state {
            Some(ModelState::Downloading { progress }) => Some((progress * 100.0).floor() as u32),
            _ => None,
        };
        let model_error = match &self.model_state {
            Some(ModelState::Failed(message)) => Some(message.clone()),
            _ => None,
        };
        let error_text = self.error.as_ref().map(|(_, m)| m.clone()).or(model_error.clone());
        let error_icon = self.error.as_ref().is_some_and(|(at, _)| now < *at + ERROR_ICON) || model_error.is_some();

        let (icon_key, icon, title, status) = if let Some(recording) = &self.recording {
            let secs = recording.started_at.elapsed().as_secs();
            let clock = format!("{}:{:02}", secs / 60, secs % 60);
            let images = match recording.images.len() {
                0 => String::new(),
                n => format!(" · 📎{n}"),
            };
            let to = recording.to_agent.as_ref().map(|agent| format!(" → {agent}")).unwrap_or_default();
            (
                "recording",
                &icons.recording,
                Some(format!("{clock}{images}{to}")),
                format!("● Recording {clock} → {}{}", recording.target_label, to.replacen(" →", " ·", 1)),
            )
        } else if self.in_flight > 0 {
            let status = match downloading {
                Some(p) => format!("Waiting for the speech model download… {p}%"),
                None if self.in_flight > 1 => format!("Transcribing… ({} recordings queued)", self.in_flight),
                None => "Transcribing…".into(),
            };
            ("busy", &icons.busy, None, status)
        } else if let Some((_, title, status)) = &self.flash {
            ("done", &icons.done, Some(title.clone()), status.clone())
        } else if error_icon {
            ("error", &icons.error, None, format!("⚠ {}", error_text.clone().unwrap_or_default()))
        } else if let Some(p) = downloading {
            ("download", &icons.download, Some(format!("{p}%")), format!("Downloading speech model… {p}%"))
        } else if let Some(dir) = &dir_name {
            let target = self.target_session().map(|s| (s.agent.clone(), s.state(&self.listeners, now_ms())));
            let status = match (&error_text, target) {
                (Some(message), _) => format!("⚠ {message}"),
                (None, Some((agent, SessionState::Listening))) => format!("Ready · {dir} → {agent}, listening"),
                (None, Some((agent, SessionState::Starting))) => format!("Ready · {dir} → {agent}, starting"),
                (None, Some((agent, SessionState::Working))) => format!("Ready · {dir} → {agent}, working"),
                (None, None) if !self.sessions.is_empty() => format!("Ready · {dir} · notes only"),
                (None, None) => format!("Ready · {dir}"),
            };
            ("idle", &icons.idle, None, status)
        } else {
            // Said up front, in the menu bar itself, not once a recording fails.
            let status = match &self.bound {
                Some(gone) => format!("{} no longer exists · run `asides` in a repo", tilde(gone)),
                None => "Not bound · start an agent (New Agent Session), choose a Directory, or run `asides`".into(),
            };
            ("unbound", &icons.unbound, Some("No repo".into()), status)
        };

        let note_info = match (&dir_name, &self.pending_new, &self.latest) {
            (None, ..) => "No directory".into(),
            (Some(dir), Some(pending), _) => match &pending.title {
                Some(title) => format!("{dir} › next recording starts \"{title}\""),
                None => format!("{dir} › next recording starts a new note"),
            },
            (Some(dir), None, Some(latest)) if latest.moved_on.is_some() => {
                format!("{dir} › next recording starts a new note ({})", latest.moved_on.unwrap_or_default())
            }
            (Some(dir), None, Some(latest)) => match latest.open {
                Some(open) => format!("{dir} › {} · {} · {open} open", latest.title, entries(latest.entries)),
                None => format!("{dir} › {} · {}", latest.title, entries(latest.entries)),
            },
            (Some(dir), None, None) => format!("{dir} · no notes yet"),
        };
        let view = View {
            icon_key,
            icon,
            title,
            status,
            recording: self.recording.is_some(),
            bound: self.bound.as_deref(),
            note_info,
            has_note: self.latest.is_some(),
            send_label: match self.target_session() {
                Some(session) => format!("Send to {}", session.agent),
                None => "Send to Agent".into(),
            },
            send_enabled: self.config.agent_delivery == AgentDelivery::Batch && self.bound.is_some(),
            replies: &self.replies,
        };
        let labels = sessions::labels(&self.sessions);
        let mut rows: Vec<TargetRow> = self
            .sessions
            .iter()
            .zip(labels)
            .map(|(s, label)| TargetRow {
                id: Some(s.id.clone()),
                label: format!("{label} — {}", s.state(&self.listeners, now_ms()).as_str()),
                checked: self.target.as_ref() == Some(&s.id),
            })
            .collect();
        if !rows.is_empty() {
            rows.push(TargetRow { id: None, label: "Notes Only".into(), checked: self.target_session().is_none() });
        }
        let shows: Vec<(String, String)> = self
            .sessions
            .iter()
            .filter(|s| s.background.is_some())
            .map(|s| (s.id.clone(), self.label_of(&s.id)))
            .collect();
        let here = self.bound.clone().filter(|d| d.is_dir());
        if let Some(tray) = &mut self.tray {
            tray.show(&view);
            tray.set_targets(&rows, &shows);
            tray.set_new_session(self.agents.as_deref(), here.as_deref(), &self.recent_dirs);
        }
    }

    fn refresh_latest(&mut self) {
        let auto = self.config.auto_new_note;
        let bound = self.bound.clone();
        self.latest = bound.as_deref().and_then(notes::latest_note).map(|note| {
            let open = note.dir.join(threads::THREADS_FILE).is_file().then(|| {
                let content = std::fs::read_to_string(note.file()).unwrap_or_default();
                threads::open_count(&note.dir, &notes::parse_entries(&content))
            });
            let moved_on = auto
                .then(|| {
                    let branch = bound.as_deref().and_then(project::current_branch);
                    notes::moved_on(&note, branch.as_deref(), SystemTime::now())
                })
                .flatten();
            LatestNote { entries: note.entry_count(), file: note.file(), title: note.title, open, moved_on }
        });
    }

    /// Publishes the app's state for the CLI (`asides status`, bind confirmation).
    fn publish(&self) {
        let status = if self.recording.is_some() {
            "recording"
        } else if self.in_flight > 0 {
            "transcribing"
        } else if matches!(self.model_state, Some(ModelState::Downloading { .. })) {
            "downloading"
        } else if self.error.is_some() || matches!(self.model_state, Some(ModelState::Failed(_))) {
            "error"
        } else {
            "idle"
        };
        let detail = match (&self.error, &self.model_state) {
            (Some((_, message)), _) => Some(message.clone()),
            (None, Some(ModelState::Failed(message))) => Some(message.clone()),
            (None, Some(ModelState::Downloading { progress })) => {
                Some(format!("Downloading the speech model: {:.0}%", progress * 100.0))
            }
            _ => None,
        };
        let state = AppState {
            pid: if self.quit { 0 } else { std::process::id() },
            bound_dir: self.bound.clone(),
            pending_new: self.pending_new.clone(),
            status: status.into(),
            detail,
            microphone: self.mic.as_str().into(),
            last_request: self.last_request.clone(),
            recent_dirs: self.recent_dirs.clone(),
            agent_sends: self.agent_sends.clone(),
            target_session: self.target_session().map(|s| s.id.clone()),
            command_link_declined: self.command_link_declined,
            overlay_tips_seen: self.overlay_tips_seen,
            updated_ms: now_ms(),
        };
        if let Err(err) = state.write(&paths::state_path()) {
            eprintln!("could not write state.json: {err}");
        }
    }

    fn fail(&mut self, message: String) {
        // The overlay covers the menu bar: say it there too.
        if let Some(overlay) = &mut self.overlay {
            overlay.send(&OverlayCommand::Notice { text: message.clone() });
        }
        self.error = Some((Instant::now(), message));
        self.sound("Basso");
        self.refresh();
        self.publish();
    }

    fn sound(&self, name: &str) {
        if self.config.sounds {
            macos::play(name);
        }
    }

    fn request_mic(&mut self) {
        let proxy = self.proxy.clone();
        macos::request_mic(move |granted| {
            let _ = proxy.send_event(UserEvent::Mic(granted));
        });
    }

    fn request_quit(&mut self) {
        if self.recording.is_some() {
            self.stop_recording();
        }
        if self.in_flight == 0 {
            self.quit = true;
            self.publish();
        } else {
            // Finish the queued transcriptions first.
            self.quit_when_idle = true;
        }
    }

    fn with_latest(&mut self, f: impl FnOnce(&std::path::Path)) {
        self.refresh_latest();
        match &self.latest {
            Some(latest) => f(&latest.file),
            None => self.fail("There is no note yet.".into()),
        }
    }

    fn update_config(&mut self, change: impl FnOnce(&mut Config)) {
        change(&mut self.config);
        if let Err(err) = self.config.save(&paths::config_path()) {
            self.fail(format!("Could not save settings: {err}"));
        }
        if let Some(tray) = &self.tray {
            tray.sync_config(&self.config);
        }
    }

    fn reload_config(&mut self) {
        let config = match Config::load(&paths::config_path()) {
            Ok(config) => config,
            Err(err) => return self.fail(format!("config.toml: {err}")),
        };
        if config == self.config {
            return;
        }
        let hotkeys_changed =
            config.shortcuts != self.config.shortcuts || config.agent_delivery != self.config.agent_delivery;
        if config.model != self.config.model {
            self.model_state = None;
            self.models.ensure(&config.model);
        }
        self.config = config;
        if hotkeys_changed {
            self.apply_hotkeys();
        }
        self.sync_wispr();
        if let Some(tray) = &self.tray {
            tray.sync_config(&self.config);
        }
        self.refresh();
    }

    fn apply_hotkeys(&mut self) {
        let Some(hotkeys) = &mut self.hotkeys else { return };
        self.next_target_on = !self.sessions.is_empty();
        hotkeys.apply(&self.config.shortcuts, self.config.agent_delivery == AgentDelivery::Batch, self.next_target_on);
        if self.recording.is_some() && self.overlay.is_none() {
            hotkeys.set_cancel_active(true);
        }
        if let Some(err) = hotkeys.errors.first() {
            self.error = Some((Instant::now(), err.clone()));
        }
    }

    fn watch_config(&self) {
        let proxy = self.proxy.clone();
        let path = paths::config_path();
        std::thread::spawn(move || {
            let modified = || std::fs::metadata(&path).and_then(|m| m.modified()).ok();
            let mut last = modified();
            loop {
                std::thread::sleep(Duration::from_secs(2));
                let now = modified();
                if now != last {
                    last = now;
                    if proxy.send_event(UserEvent::ConfigFileChanged).is_err() {
                        return;
                    }
                }
            }
        });
    }

    #[cfg(feature = "dev-hooks")]
    fn simulate(
        &mut self,
        wav: &std::path::Path,
        new_note: bool,
        image: Option<PathBuf>,
        image_at: f32,
    ) -> anyhow::Result<()> {
        use anyhow::Context;
        let dir = self.bound.clone().context("not bound")?;
        let mut reader = hound::WavReader::open(wav)?;
        let spec = reader.spec();
        let interleaved: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int => reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 / (1i64 << (spec.bits_per_sample - 1)) as f32))
                .collect::<Result<_, _>>()?,
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
        };
        let channels = spec.channels.max(1) as usize;
        let samples = interleaved.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect();
        let images = match image {
            Some(path) => vec![CapturedImage {
                offset: image_at,
                at: Local::now(),
                bytes: std::fs::read(&path)?,
                extension: path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into()),
                details: None,
            }],
            None => Vec::new(),
        };
        let pending = self.pending_new.take();
        let new_note = (new_note || pending.is_some()).then(|| pending.and_then(|p| p.title));
        let route = self.route(&dir);
        self.submit(Job::Recording(Box::new(RecordingJob {
            route,
            dir,
            samples,
            sample_rate: spec.sample_rate,
            started: Local::now(),
            new_note,
            images,
            texts: Vec::new(),
            environment: environment::capture(self.mtm),
            keep_audio: self.config.keep_audio,
            model: self.config.model.clone(),
            languages: self.config.languages.clone(),
            auto_new_note: self.config.auto_new_note,
        })));
        Ok(())
    }

    /// Runs a scripted Measure & Annotate session on `image` instead of the screen, in the overlay's
    /// process without windows; the result is added like the user's own (the end-to-end test
    /// checks it).
    #[cfg(feature = "dev-hooks")]
    fn simulate_measure(&mut self, image: &Path, script: &Path) -> anyhow::Result<()> {
        use anyhow::Context;
        let script =
            asides_core::overlay::Script::parse(&std::fs::read_to_string(script)?).map_err(anyhow::Error::msg)?;
        let frame = overlay::load_image(image).context("could not read the image")?;
        let (w, h) = (frame.width as f64 / script.scale, frame.height as f64 / script.scale);
        let screen = SpecScreen {
            display: 0,
            screen: Screen { name: Some("Simulated screen".into()), width: w, height: h, scale: script.scale },
            origin: (0.0, 0.0),
            frame: PtRect::new(0.0, 0.0, w, h),
            pixels: String::new(),
            width: frame.width,
            height: frame.height,
        };
        let spec = Spec { screens: vec![screen], script: Some(script), ..self.overlay_spec() };
        self.open_overlay(spec, &[frame], Vec::new())
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// `text` on one line, cut to `max` characters.
fn short(text: &str, max: usize) -> String {
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= max { line } else { format!("{}…", line.chars().take(max - 1).collect::<String>()) }
}

fn entries(n: u32) -> String {
    if n == 1 { "1 entry".into() } else { format!("{n} entries") }
}
