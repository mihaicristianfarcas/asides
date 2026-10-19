//! asides: a menu-bar app. `⌃⌥Space` freezes the screen under a small pill: click what's wrong (a
//! numbered pin), drag an area, measure, draw, and say why; `⌃⌥Space` again sends it all, as one
//! aside, transcribed on this Mac, into the coding agent's session the user last typed into, typed
//! there as their own message (`asides_core::deliver`). Every aside is kept in the history
//! (`~/.asides/history`). The overlay is a separate process (`asides-overlay`, GPUI).

mod audio;
mod capture;
mod clipboard;
mod elements;
mod hotkeys;
mod icons;
mod macos;
mod overlay;
mod stt;
mod terminal;
mod tray;
mod update;
mod watch;
mod worker;

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use asides_core::agents::{self, AgentKind};
use asides_core::config::Config;
use asides_core::deliver;
use asides_core::helper::{
    Command as OverlayCommand, Event as OverlayEvent, Folder, ModelInfo, Outcome as OverlayOutcome, Spec, SpecScreen,
    Target, TargetSession, Targets,
};
use asides_core::history::{self, Aside, Delivery};
use asides_core::inspect::{self, PtRect, Screen};
use asides_core::install;
use asides_core::link::Link;
use asides_core::models::ModelSpec;
use asides_core::overlay::Saved;
use asides_core::paths;
use asides_core::sessions::{self, AgentSession};
use asides_core::state::{self as app_state, AppState, Permission, Permissions};
use asides_core::takes;
use asides_core::update::{Phase, Status as UpdateStatus, Step, Version};
use asides_core::wispr::Dictation;
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
use crate::hotkeys::{Action, Hotkeys};
use crate::icons::{Icons, Mark};
use crate::macos::{CommandLink, Mic};
use crate::overlay::{Message as OverlayMessage, Overlay, Standby};
use crate::stt::{ModelManager, ModelState};
use crate::tray::{MenuAction, Tray, View};
use crate::update::{Flow, Next, Reply as UpdateReply, Request as UpdateRequest, Updater};
use crate::worker::{AsideJob, CapturedImage, Job, Outcome, TranscribeJob};

/// A press shorter than this on an annotation's microphone toggles it; longer, it talks until let go.
const TAP_MAX: Duration = Duration::from_millis(350);
const TICK: Duration = Duration::from_millis(250);
/// How often the pill's microphone hears how loud it is, while asides is open and listening.
const LEVEL_TICK: Duration = Duration::from_millis(50);
const FLASH: Duration = Duration::from_secs(2);
const ERROR_ICON: Duration = Duration::from_secs(6);
/// Recordings shorter than this (without images) are treated as accidental and dropped.
const MIN_RECORDING_SECS: f32 = 0.3;
/// How long the outcome of a send stays in the menu bar.
const SENT_FLASH: Duration = Duration::from_secs(4);
/// How long asides waits for the screenshots before it may ask again.
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(10);
/// How many folders New session in… offers.
const FOLDERS: usize = 8;
/// How long the folders found are good for.
const FOLDERS_FRESH: Duration = Duration::from_secs(60);
/// How often the permissions are looked at while the asides window is open (setup shows them).
const PERMISSIONS_TICK: Duration = Duration::from_millis(1500);

enum UserEvent {
    HotKey(GlobalHotKeyEvent),
    Menu(MenuEvent),
    Work(Outcome),
    Model(&'static str, ModelState),
    Mic(bool),
    ConfigFileChanged,
    Wispr(Result<Dictation, String>),
    /// The agent sessions running.
    Sessions(Vec<AgentSession>),
    /// The coding agents installed, as the user's login shell finds them.
    Agents(Vec<AgentKind>),
    /// An aside was typed into its session (where, for the history), or what went wrong.
    Delivered {
        id: String,
        delivery: Delivery,
    },
    CommandLinked(CommandLink),
    /// The screens, frozen.
    Captured(Result<Vec<Capture>, String>),
    /// What the overlay (its process id) said.
    Overlay(u32, OverlayMessage),
    /// What the updater found, downloaded or couldn't do.
    Update(UpdateReply),
    /// What macOS lets asides do now (and the microphone).
    Permissions(Permissions, Mic),
}

struct Recording {
    recorder: audio::Recorder,
    started_at: Instant,
    started: DateTime<Local>,
    /// Measure & Annotate annotations made while recording, as (seconds, Markdown).
    texts: Vec<(f32, String)>,
    /// The press that started it (an annotation's microphone, Space): held, it stops on release.
    pressed: Option<Instant>,
    /// Space held while muted: it stops when Space is let go.
    push_to_talk: bool,
    /// Started from an annotation's card (its key): what's said is about it.
    annotation: Option<u32>,
    /// Where the user was when they started talking (`**In:** …`), once it's been looked up.
    place: Place,
}

/// A recording that ended while asides was open. What's said in a session waits for it to end, to
/// make one aside with what was annotated (`asides_core::takes`).
struct Take {
    started_at: Instant,
    started: DateTime<Local>,
    images: Vec<CapturedImage>,
    texts: Vec<(f32, String)>,
    samples: Vec<f32>,
    sample_rate: u32,
    annotation: Option<u32>,
}

impl Take {
    /// Too short to be meant, with nothing attached.
    fn accidental(&self) -> bool {
        let seconds = self.samples.len() as f32 / self.sample_rate.max(1) as f32;
        seconds < MIN_RECORDING_SECS && self.images.is_empty() && self.texts.is_empty()
    }
}

/// A finished session waiting for what was said about its annotations to be transcribed, to keep
/// it all at once.
struct Finishing {
    outcome: OverlayOutcome,
    images: Vec<Option<Vec<u8>>>,
    takes: Vec<Take>,
    origin: Instant,
    /// Its recordings about annotations are those up to this one (a next session's come after).
    last_take: u64,
    /// Wispr Flow dictations made in it while frozen, at the seconds since it opened.
    dictations: Vec<(f32, String)>,
}

/// A moment in the menu bar: a mark between the brackets, and the menu's status line.
struct Flash {
    until: Instant,
    mark: Mark,
    status: String,
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
    recording: Option<Recording>,
    in_flight: usize,
    /// asides being typed into their sessions.
    sending: usize,
    model_state: Option<ModelState>,
    flash: Option<Flash>,
    error: Option<(Instant, String)>,
    mic: Mic,
    keys_down: HashSet<u32>,
    /// Where the user put the pill (published in the state for next time).
    pill_at: Option<(f64, f64)>,
    last_request: Option<String>,
    /// The agent sessions running, as last found, and how to ask for a fresh look.
    sessions: Vec<AgentSession>,
    find_sessions: Option<Sender<()>>,
    /// asides opened and asked for a fresh look, not answered yet.
    looking: bool,
    /// The coding agents installed (`None` until the login shell has answered).
    agents: Option<Vec<AgentKind>>,
    recent_dirs: Vec<PathBuf>,
    /// Folders New session in… offers, and when they were found.
    folders: Option<(Instant, Vec<PathBuf>)>,
    /// Where the open session's aside goes.
    target: Option<Target>,
    last_aside: Option<String>,
    /// The user declined to put the `asides` command on the PATH (published, so it's asked once).
    command_link_declined: bool,
    /// Set to stop watching Wispr Flow's history.
    wispr: Option<Arc<AtomicBool>>,
    /// A freeze under way: when it began, the screens and windows (until the capture arrives).
    capturing: Option<Capturing>,
    /// Screen Recording is allowed. Asking takes 11 ms each time, so it's asked until it is, and
    /// again after a capture fails.
    screen_permitted: bool,
    /// The overlay, while asides is open...
    overlay: Option<Overlay>,
    /// ...and the next one, started ahead of time.
    standby: Option<Standby>,
    /// asides opens over the live screen (`⌃⌥P` with it closed), once the screens are listed.
    open_live: bool,
    /// What was said in the open session, until it ends.
    takes: Vec<Take>,
    /// What was said about its annotations: (annotation key, take, transcript once it's back).
    said: Vec<(u32, u64, Option<String>)>,
    next_take: u64,
    finishing: Option<Finishing>,
    /// Wispr Flow dictations made while frozen, at the seconds since asides opened.
    dictations: Vec<(f32, String)>,
    /// The last session's guides and held measurements, which the next can restore.
    last_measure: Saved,
    /// The overlay showed its keys once already.
    overlay_tips_seen: bool,
    /// The asides window's process.
    window: Option<Child>,
    /// The microphones, for the window's settings.
    input_devices: Vec<String>,
    /// asides' own updates: the thread that checks and downloads, and where the update is.
    updater: Option<Updater>,
    updates: Flow,
    /// asides restarts into the update as it quits (and opens the window again, if it was open).
    restart: Option<bool>,
    /// The newest version whose What's New the user saw.
    whats_new_seen: String,
    /// What macOS lets asides do, as last looked at, when, and whether a look is under way.
    permissions: Permissions,
    permissions_at: Option<Instant>,
    checking_permissions: bool,
    /// The user went through setup or skipped it, and how far it got.
    onboarded: bool,
    setup_step: Option<String>,
    quit_when_idle: bool,
    quit: bool,
}

fn main() {
    // An agent's process can start the app (`asides` run by an agent): don't hand that session's
    // markers to every terminal and agent the app starts.
    for marker in agents::inherited_markers() {
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
    // Without a handler, tray-icon queues an event for every mouse move over the icon, forever.
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
            Event::LoopDestroyed => app.exit(),
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
        let previous = AppState::read(&paths::state_path()).unwrap_or_default();
        let model_state = models.state(&config.model);
        // A fresh copy starts with its own What's New seen; one just updated is told what's new by
        // the copy before it, until it checks.
        let whats_new_seen = previous.whats_new_seen.clone().unwrap_or_else(|| Version::current().to_string());
        let updates = Flow::new(UpdateStatus { notes: previous.update.handed_over(), ..UpdateStatus::default() });
        App {
            mtm,
            proxy,
            config,
            icons: None,
            tray: None,
            hotkeys: None,
            models,
            jobs,
            recording: None,
            in_flight: 0,
            sending: 0,
            model_state,
            flash: None,
            error: None,
            mic: macos::mic_status(),
            keys_down: HashSet::new(),
            pill_at: previous.pill_at,
            last_request: None,
            sessions: Vec::new(),
            find_sessions: None,
            looking: false,
            agents: None,
            recent_dirs: previous.recent_dirs.into_iter().filter(|d| d.is_dir()).collect(),
            folders: None,
            target: None,
            last_aside: previous.last_aside,
            command_link_declined: previous.command_link_declined,
            wispr: None,
            capturing: None,
            screen_permitted: false,
            overlay: None,
            standby: None,
            open_live: false,
            takes: Vec::new(),
            said: Vec::new(),
            next_take: 1,
            finishing: None,
            dictations: Vec::new(),
            last_measure: Saved::default(),
            overlay_tips_seen: previous.overlay_tips_seen,
            window: None,
            input_devices: audio::input_device_names(),
            updater: None,
            updates,
            restart: None,
            whats_new_seen,
            permissions: previous.permissions,
            permissions_at: None,
            checking_permissions: false,
            onboarded: previous.onboarded,
            setup_step: previous.setup_step,
            quit_when_idle: false,
            quit: false,
        }
    }

    fn init(&mut self) {
        let icons = Icons::load();
        match Tray::new(&self.config, &icons.get(Mark::Letter)) {
            Ok(tray) => self.tray = Some(tray),
            Err(err) => eprintln!("could not create the status item: {err:#}"),
        }
        self.icons = Some(icons);
        match Hotkeys::new(&self.config.shortcuts) {
            Ok(hotkeys) => {
                if let Some(err) = hotkeys.errors.first() {
                    self.error = Some((Instant::now(), err.clone()));
                }
                self.hotkeys = Some(hotkeys);
            }
            Err(err) => self.error = Some((Instant::now(), format!("Global shortcuts unavailable: {err}"))),
        }
        self.watch_config();
        let (asked, requests) = std::sync::mpsc::channel();
        self.find_sessions = Some(asked);
        let proxy = self.proxy.clone();
        watch::sessions(requests, move |found| proxy.send_event(UserEvent::Sessions(found)).is_ok());
        self.look_for_agents();
        self.sync_wispr();
        self.prune_history();
        self.link_command();
        self.stand_by();
        self.start_updater();
        self.check_permissions();
        // The first time: setup (permissions, the voice model, where new sessions open). Skipped,
        // each permission and the model are asked for when first needed. Test builds skip it.
        let setup = !cfg!(feature = "dev-hooks") || std::env::var_os("ASIDES_SETUP").is_some();
        if !self.onboarded && setup {
            self.open_window_at(Some("setup"));
        }
        self.refresh();
        self.publish();
    }

    /// The first time a copy installed by hand runs, puts the `asides` command on the PATH, as the
    /// Homebrew cask does: /usr/local/bin/asides → the CLI in the bundle. Skipped when an `asides`
    /// is already set up, or once the user declined the password prompt.
    fn link_command(&self) {
        let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
        // …/asides.app/Contents/MacOS/asides-app
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
                    self.tell_overlay_model();
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
            UserEvent::Sessions(found) => {
                self.sessions = found;
                self.looking = false;
                self.tell_overlay_targets();
                self.publish();
            }
            UserEvent::Agents(agents) => {
                self.agents = Some(agents);
                self.tell_overlay_targets();
                self.publish();
            }
            UserEvent::Delivered { id, delivery } => self.delivered(&id, delivery, true),
            UserEvent::CommandLinked(CommandLink::Linked) => {
                let status = format!("You can now run `asides` in any terminal ({})", install::LINK);
                self.flash(SENT_FLASH, Mark::Check, status);
                self.refresh();
            }
            UserEvent::CommandLinked(CommandLink::Declined) => {
                self.command_link_declined = true;
                self.publish();
            }
            UserEvent::CommandLinked(CommandLink::Failed(message)) => {
                self.fail(format!("Could not add the asides command to {}: {message}", install::LINK));
            }
            // asides frozen as it opens, or, open, frozen again.
            UserEvent::Captured(Ok(captures)) if self.overlay.is_some() => self.freeze_overlay(captures),
            UserEvent::Captured(Ok(captures)) => self.show_overlay(captures),
            UserEvent::Captured(Err(message)) => {
                self.capturing = None;
                self.screen_permitted = false;
                self.fail(message);
            }
            UserEvent::Overlay(pid, message) => self.on_overlay(pid, message),
            UserEvent::Update(reply) => self.on_update(reply),
            UserEvent::Permissions(mut permissions, mic) => {
                // macOS answers only for an app that runs: one that quit keeps its last answer.
                for (app, status) in &mut permissions.automation {
                    if let Some(known) = self.permissions.automation(app).filter(|s| *s == "granted" || *s == "denied")
                        && status == "not running"
                    {
                        *status = known.to_string();
                    }
                }
                self.checking_permissions = false;
                self.permissions_at = Some(Instant::now());
                self.screen_permitted |= permissions.screen;
                let changed = mic != self.mic
                    || Permissions { checked_ms: 0, ..permissions.clone() }
                        != Permissions { checked_ms: 0, ..self.permissions.clone() };
                (self.permissions, self.mic) = (permissions, mic);
                if changed {
                    self.refresh();
                    self.publish();
                }
            }
        }
    }

    fn handle_link(&mut self, url: &url::Url) {
        let Some(link) = Link::parse(url) else { return };
        self.last_request = link.request().cloned();
        match link {
            Link::Open { .. } => self.open_or_send(),
            Link::Window { .. } => self.open_window(),
            Link::Send { id, to, .. } => self.send_again(&id, to),
            Link::NewSession { id, agent, dir, .. } => self.send_again_to(&id, Target::New { agent, dir }),
            Link::Download { model, .. } => {
                if ModelSpec::find(&model).id != self.config.model {
                    self.update_config(|c| c.model = model.clone());
                }
                self.model_state = self.models.state(&self.config.model);
                self.models.download(&self.config.model);
            }
            Link::Update { step, .. } => self.update_step(step),
            Link::Permission { name, .. } => self.ask_permission(name),
            Link::Setup { step, .. } => match step.as_str() {
                "done" => (self.onboarded, self.setup_step) = (true, None),
                _ => self.setup_step = Some(step),
            },
            Link::Quit => self.request_quit(),
            Link::Simulate { wav, image, image_at, annotation, to, .. } => {
                #[cfg(feature = "dev-hooks")]
                if let Err(err) = self.simulate(&wav, image, image_at, annotation, to) {
                    self.fail(format!("simulate: {err:#}"));
                }
                #[cfg(not(feature = "dev-hooks"))]
                {
                    let _ = (wav, image, image_at, annotation, to);
                    self.fail("This build does not support `asides simulate`.".into());
                }
            }
            Link::SimulateMeasure { image, script, to, .. } => {
                #[cfg(feature = "dev-hooks")]
                if let Err(err) = self.simulate_measure(&image, &script, to) {
                    self.fail(format!("simulate-measure: {err:#}"));
                }
                #[cfg(not(feature = "dev-hooks"))]
                {
                    let _ = (image, script, to);
                    self.fail("This build does not support `asides simulate-measure`.".into());
                }
            }
        }
        self.publish();
    }

    fn on_hotkey(&mut self, event: GlobalHotKeyEvent) {
        let Some(action) = self.hotkeys.as_ref().and_then(|h| h.action(event.id)) else { return };
        match event.state {
            HotKeyState::Pressed => {
                // Key repeat while it's held does nothing more.
                if !self.keys_down.insert(event.id) {
                    return;
                }
                match action {
                    Action::Asides => self.open_or_send(),
                    Action::Freeze => self.freeze(),
                }
            }
            HotKeyState::Released => {
                self.keys_down.remove(&event.id);
            }
        }
    }

    /// The asides shortcut (or the menu, or `asides open`): opens asides frozen, or, open, sends.
    fn open_or_send(&mut self) {
        match &mut self.overlay {
            Some(overlay) => overlay.send(&OverlayCommand::Finish),
            None if self.capturing.is_some() => {}
            None => self.open_asides(false),
        }
    }

    /// The freeze shortcut: freezes the screen, or back to live; closed, opens asides live.
    fn freeze(&mut self) {
        match &mut self.overlay {
            Some(overlay) => overlay.send(&OverlayCommand::ToggleFreeze),
            None => self.open_asides(true),
        }
    }

    fn on_menu(&mut self, event: MenuEvent) {
        let Some(action) = self.tray.as_ref().and_then(|t| t.action(&event.id)) else { return };
        match action {
            MenuAction::Open => self.open_or_send(),
            MenuAction::Live => self.freeze(),
            MenuAction::CopyLast => self.copy_last(),
            MenuAction::Listen => self.update_config(|c| c.mic_on_open = !c.mic_on_open),
            MenuAction::Sounds => self.update_config(|c| c.sounds = !c.sounds),
            MenuAction::Identify => self.update_config(|c| c.measure.identify_elements = !c.measure.identify_elements),
            MenuAction::History => self.open_window_at(Some("history")),
            MenuAction::Settings => self.open_window_at(Some("general")),
            MenuAction::Update => self.update_step(Step::Install),
            MenuAction::Quit => self.request_quit(),
        }
    }

    fn on_outcome(&mut self, outcome: Outcome) {
        self.in_flight = self.in_flight.saturating_sub(1);
        match outcome {
            Outcome::Kept { aside, target } => {
                self.error = None;
                self.last_aside = Some(aside.id.clone());
                self.deliver(*aside, target);
            }
            Outcome::NothingHeard => {
                self.flash(FLASH, Mark::Dash, "Nothing heard; nothing was sent");
            }
            Outcome::Failed(message) => self.fail(message),
            Outcome::Transcribed { id, text } => {
                let text = text.unwrap_or_else(|err| {
                    self.fail(format!("Could not transcribe what you said: {err}"));
                    String::new()
                });
                // Unless it was thrown away meanwhile.
                if let Some(said) = self.said.iter_mut().find(|s| s.1 == id) {
                    said.2 = Some(text.clone());
                    let annotation = said.0;
                    if let Some(overlay) = self.overlay.as_mut().filter(|o| !o.over) {
                        overlay.send(&OverlayCommand::Said { annotation, take: id, text });
                    }
                }
                self.try_finish();
            }
        }
        self.refresh();
        self.publish();
        if self.quit_when_idle && self.in_flight == 0 && self.sending == 0 && self.recording.is_none() {
            self.quit = true;
        }
    }

    // --- Sending ---

    /// Sends an aside where the user said: types it into a session, starts a new one with it, or
    /// keeps it in the history only.
    fn deliver(&mut self, aside: Aside, target: Option<Target>) {
        let id = aside.id.clone();
        match target {
            None | Some(Target::History) => {
                self.flash(SENT_FLASH, Mark::Check, "Saved in History (asides window)");
                self.sound("Glass");
            }
            Some(Target::Session { id: session }) => {
                // Started since the last look (a few seconds ago, at most): look again.
                if !self.sessions.iter().any(|s| s.id() == session) {
                    self.sessions = sessions::discover();
                }
                let found = self.sessions.iter().find(|s| s.id() == session).cloned();
                let Some(session) = found else {
                    let delivery =
                        Delivery { at_ms: history::now_ms(), to: format!("session {session}"), ..Delivery::default() };
                    let error = "that session has ended".to_string();
                    return self.delivered(&id, Delivery { error: Some(error), ..delivery }, false);
                };
                let label = session.label();
                let delivery = Delivery {
                    at_ms: history::now_ms(),
                    to: label.clone(),
                    agent: Some(session.agent),
                    dir: Some(session.dir.clone()),
                    app: session.host.name().to_string(),
                    error: None,
                };
                let plan = match deliver::plan(&session, &aside.parts(), &terminal::dirijor()) {
                    Ok(plan) => plan,
                    Err(message) => return self.delivered(&id, Delivery { error: Some(message), ..delivery }, false),
                };
                self.remember_dir(&session.dir);
                self.sending += 1;
                self.flash(SENT_FLASH, Mark::Ellipsis, format!("Sending to {label}…"));
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let error = terminal::run_steps(&plan).err().map(|e| format!("{e:#}"));
                    let _ = proxy.send_event(UserEvent::Delivered { id, delivery: Delivery { error, ..delivery } });
                });
            }
            Some(Target::New { agent, dir }) => {
                self.remember_dir(&dir);
                let terminal_app = terminal::app(&self.config.agents.terminal);
                let name = sessions::repo_name(&dir);
                let delivery = Delivery {
                    at_ms: history::now_ms(),
                    to: format!("New {} ({name})", agent.name()),
                    agent: Some(agent),
                    dir: Some(dir.clone()),
                    app: terminal_app.clone(),
                    error: None,
                };
                let message = deliver::first_message(&aside.parts());
                let extra = self.config.agents.args(agent).to_vec();
                let readable = vec![history::dir()];
                self.sending += 1;
                self.flash(SENT_FLASH, Mark::Ellipsis, format!("Starting {} in {name}…", agent.name()));
                let proxy = self.proxy.clone();
                std::thread::spawn(move || {
                    let result = terminal::start_session(&terminal_app, agent, &dir, &message, &extra, &readable);
                    let error = result.err().map(|e| format!("{e:#}"));
                    let _ = proxy.send_event(UserEvent::Delivered { id, delivery: Delivery { error, ..delivery } });
                });
            }
        }
        self.refresh();
    }

    /// An aside reached its session (or didn't): say so, and note it in the history.
    /// `started`: it was typed (or tried), counted in `sending`; otherwise it failed before.
    fn delivered(&mut self, id: &str, delivery: Delivery, started: bool) {
        if started {
            self.sending = self.sending.saturating_sub(1);
        }
        match &delivery.error {
            None => {
                let number = asides_today(id);
                self.flash(SENT_FLASH, Mark::Number(number), format!("Sent #{number} to {}", delivery.to));
                self.sound("Glass");
            }
            Some(message) => {
                self.fail(format!("Could not send it to {}: {message}. It's in History.", delivery.to));
            }
        }
        if let Some(mut aside) = history::find(&history::dir(), id)
            && let Err(err) = aside.record_delivery(delivery)
        {
            eprintln!("could not note the delivery of {id}: {err}");
        }
        // It got input: it's the session typed into last now.
        self.ask_for_sessions();
        if self.quit_when_idle && self.in_flight == 0 && self.sending == 0 {
            self.quit = true;
        }
        self.refresh();
        self.publish();
    }

    /// An aside from the history, sent again: to session `to`, or the default one.
    fn send_again(&mut self, id: &str, to: Option<String>) {
        let target = match to.as_deref() {
            Some("history") => Target::History,
            Some(session) => Target::Session { id: session.to_string() },
            None => match sessions::latest(&self.sessions) {
                Some(session) => Target::Session { id: session.id() },
                None => return self.fail("No agent session is running that asides can type into.".into()),
            },
        };
        self.send_again_to(id, target);
    }

    fn send_again_to(&mut self, id: &str, target: Target) {
        match history::find(&history::dir(), id) {
            Some(aside) => self.deliver(aside, Some(target)),
            None => self.fail(format!("There's no aside {id} in the history.")),
        }
    }

    fn remember_dir(&mut self, dir: &Path) {
        let root = dir.ancestors().find(|d| d.join(".git").exists()).unwrap_or(dir);
        app_state::remember_dir(&mut self.recent_dirs, root);
        self.folders = None;
    }

    /// Asks for a fresh look at the sessions running.
    fn ask_for_sessions(&mut self) {
        if let Some(asked) = &self.find_sessions {
            let _ = asked.send(());
        }
    }

    /// Where an aside can go, for the pill.
    fn targets(&mut self) -> Targets {
        let mut running: Vec<AgentSession> = self.sessions.clone();
        // Typed into last first.
        running.sort_by_key(|s| std::cmp::Reverse(s.last_input));
        let labels = sessions::labels(&running);
        let sessions = running
            .iter()
            .zip(labels)
            .map(|(s, label)| TargetSession {
                id: s.id(),
                label,
                app: s.host.name().to_string(),
                unreachable: (!s.host.reachable()).then(|| format!("In {}: asides can't type there", s.host.name())),
            })
            .collect();
        let folders =
            self.folders().into_iter().map(|path| Folder { name: sessions::repo_name(&path), path }).collect();
        let first = self.config.agents.default_agent();
        let mut agents =
            self.agents.clone().unwrap_or_else(|| vec![AgentKind::Claude, AgentKind::Codex, AgentKind::OpenCode]);
        agents.sort_by_key(|&a| a != first);
        let chosen = match sessions::latest(&self.sessions) {
            Some(session) => Target::Session { id: session.id() },
            None => Target::History,
        };
        Targets { sessions, folders, agents, chosen: Some(chosen), looking: self.looking }
    }

    fn folders(&mut self) -> Vec<PathBuf> {
        if let Some((at, folders)) = &self.folders
            && at.elapsed() < FOLDERS_FRESH
        {
            return folders.clone();
        }
        let home = std::env::home_dir().unwrap_or_default();
        let folders = sessions::known_folders(&home, &self.sessions, &self.recent_dirs, FOLDERS);
        self.folders = Some((Instant::now(), folders.clone()));
        folders
    }

    fn tell_overlay_targets(&mut self) {
        if self.overlay.as_ref().is_some_and(|o| !o.over) {
            let targets = self.targets();
            if let Some(overlay) = &mut self.overlay {
                overlay.send(&OverlayCommand::Targets { targets });
            }
        }
    }

    fn model_info(&self) -> ModelInfo {
        let spec = ModelSpec::find(&self.config.model);
        ModelInfo {
            ready: spec.downloaded(),
            label: format!("{}, {}", spec.label, spec.size_label()),
            downloading: match &self.model_state {
                Some(ModelState::Downloading { progress }) => Some(*progress),
                _ => None,
            },
        }
    }

    fn tell_overlay_model(&mut self) {
        let model = self.model_info();
        if let Some(overlay) = self.overlay.as_mut().filter(|o| !o.over) {
            overlay.send(&OverlayCommand::Model { model });
        }
    }

    // --- Recording ---

    /// Starts recording: `annotation`, what's said is about that annotation (from its card);
    /// `push_to_talk`, until Space is let go.
    fn start_recording(&mut self, pressed: Option<Instant>, annotation: Option<u32>, push_to_talk: bool) {
        match macos::mic_status() {
            Mic::Granted => self.mic = Mic::Granted,
            Mic::Undetermined => {
                if let Some(overlay) = &mut self.overlay {
                    let text = "asides needs the microphone: close this (Esc) to answer macOS's prompt.".into();
                    overlay.send(&OverlayCommand::Notice { text });
                }
                return self.request_mic();
            }
            Mic::Denied => {
                self.mic = Mic::Denied;
                return self.fail(
                    "Microphone access is denied: allow asides in System Settings → Privacy & Security → Microphone."
                        .into(),
                );
            }
        }
        if self.models.state(&self.config.model).is_none() {
            self.models.download(&self.config.model);
        }
        let recorder = match audio::Recorder::start(&self.config.input_device) {
            Ok(recorder) => recorder,
            Err(err) => return self.fail(format!("{err:#}")),
        };
        // Where the user is, for the aside: in the app a frozen screen stands for, or the one in
        // front (what's said from an annotation's card is about that annotation).
        let frozen = self.overlay.as_ref().filter(|o| o.frozen);
        let pid = frozen.map_or_else(macos::frontmost_app, |o| o.previous_app).filter(|_| annotation.is_none());
        let place = look_up_place(pid, self.config.measure.identify_elements);
        // The speech model, loaded while the user talks (it's let go after a while unused).
        let _ = self.jobs.send(Job::Warm(self.config.model.clone()));
        self.recording = Some(Recording {
            recorder,
            started_at: Instant::now(),
            started: Local::now(),
            texts: Vec::new(),
            pressed,
            push_to_talk,
            annotation,
            place,
        });
        self.tell_overlay_recording();
        self.error = None;
        self.flash = None;
        self.sound("Tink");
        self.refresh();
        self.publish();
    }

    fn stop_recording(&mut self) {
        if let Some(take) = self.end_recording() {
            self.keep_take(take);
        }
    }

    fn keep_take(&mut self, take: Take) {
        if take.accidental() {
            self.refresh();
            self.publish();
            return;
        }
        // Said about an annotation: transcribed now, for its card and the aside.
        if let Some(annotation) = take.annotation {
            let id = self.next_take;
            self.next_take += 1;
            self.said.push((annotation, id, None));
            if let Some(overlay) = self.overlay.as_mut().filter(|o| !o.over) {
                overlay.send(&OverlayCommand::Transcribing { annotation, take: id });
            }
            let (model, languages) = (self.config.model.clone(), self.config.languages.clone());
            let job = TranscribeJob { id, samples: take.samples, sample_rate: take.sample_rate, model, languages };
            return self.submit(Job::Transcribe(Box::new(job)));
        }
        // In asides, what's said waits for it to be sent, to make one aside with what's annotated.
        if self.overlay.as_ref().is_some_and(|o| !o.over) {
            self.takes.push(take);
            self.refresh();
            self.publish();
            return;
        }
        // Outside asides (`asides simulate`): an aside of its own, kept in the history.
        let origin = take.started_at;
        self.add_session(vec![take], OverlayOutcome::default(), Vec::new(), Vec::new(), origin, Some(Target::History));
    }

    /// Ends the recording under way: what it captured.
    fn end_recording(&mut self) -> Option<Take> {
        let recording = self.recording.take()?;
        self.tell_overlay_recording();
        let (samples, sample_rate) = recording.recorder.finish();
        self.sound("Pop");
        let mut texts = recording.texts;
        if let Some((place, _)) = recording.place.lock().unwrap().take() {
            texts.insert(0, (0.0, place));
        }
        Some(Take {
            started_at: recording.started_at,
            started: recording.started,
            images: Vec::new(),
            texts,
            samples,
            sample_rate,
            annotation: recording.annotation,
        })
    }

    /// A Wispr Flow dictation made while the screen is frozen joins the aside, when it was made.
    /// (The watcher reports it a moment later: one made before asides opened isn't its.)
    fn add_dictation(&mut self, dictation: Dictation) {
        let Some(overlay) = self.overlay.as_ref().filter(|o| o.frozen && !o.over) else { return };
        let at = dictation.at.with_timezone(&Local);
        if at < wall_clock(overlay.origin()) {
            return;
        }
        let ago = (Local::now() - at).to_std().unwrap_or_default();
        let when = Instant::now().checked_sub(ago).unwrap_or_else(Instant::now);
        let offset = overlay::seconds_between(overlay.origin(), when);
        self.dictations.push((offset, dictation.text.trim().to_string()));
        self.sound("Tink");
    }

    // --- asides on screen ---

    /// Opens asides with every screen frozen (or, `live`, over the live screen).
    fn open_asides(&mut self, live: bool) {
        self.open_live = live;
        self.looking = true;
        self.ask_for_sessions();
        self.capture_screens();
    }

    /// Freezes every screen (asking for the Screen Recording permission the first time): asides
    /// opens once the screenshots arrive, or, open, freezes (`freeze_overlay`).
    fn capture_screens(&mut self) {
        // A capture that never answered doesn't block the next one for long.
        if self.capturing.as_ref().is_some_and(|c| c.at.elapsed() < CAPTURE_TIMEOUT) {
            return;
        }
        if !self.screen_permitted && !capture::permitted() {
            capture::request();
            return self.fail(
                "asides needs to see the screen to freeze it: allow it in System Settings → Privacy & Security → \
                 Screen & System Audio Recording, then quit and reopen asides."
                    .into(),
            );
        }
        self.screen_permitted = true;
        let screens = capture::screens(self.mtm);
        let displays = screens
            .iter()
            .map(|s| {
                let px = |pt: f64| (pt * s.scale).round() as usize;
                (s.display, px(s.frame.size.width), px(s.frame.size.height))
            })
            .collect();
        // The app the user is in, to give the keyboard back to when the screen goes live again.
        let overlay = self.overlay.as_mut();
        let without = overlay.as_ref().map(|o| o.pid());
        let front = macos::frontmost_app().filter(|&pid| pid != std::process::id() as i32);
        if let Some(overlay) = overlay {
            overlay.previous_app = front;
        }
        let place = look_up_place(front, self.config.measure.identify_elements);
        let windows = elements::windows(without);
        // Over the live screen, nothing to capture: the overlay opens with the screens listed.
        if self.overlay.is_none() && self.open_live {
            self.capturing = Some(Capturing { at: Instant::now(), screens, windows, place });
            return self.show_overlay(Vec::new());
        }
        let proxy = Mutex::new(self.proxy.clone());
        capture::capture(displays, without, move |result| {
            let _ = proxy.lock().unwrap().send_event(UserEvent::Captured(result));
        });
        // While the screens are taken: where the user is, for the screenshot's aside, and the
        // windows as they are now, before the overlay covers them.
        self.capturing = Some(Capturing { at: Instant::now(), screens, windows, place });
    }

    /// asides, open over the live screen, frozen: the screenshots go to its overlay.
    fn freeze_overlay(&mut self, captures: Vec<Capture>) {
        let Some(Capturing { screens, windows, place, .. }) = self.capturing.take() else { return };
        let (spec_screens, frames) = spec_screens(screens, captures, place.lock().unwrap().take());
        let Some(overlay) = &mut self.overlay else { return };
        overlay.windows = windows;
        if spec_screens.is_empty() {
            return self.fail("Could not capture the screen.".into());
        }
        if let Err(err) = overlay.freeze(spec_screens, &frames) {
            self.fail(format!("Could not freeze the screen: {err:#}"));
        }
        self.refresh();
    }

    fn show_overlay(&mut self, captures: Vec<Capture>) {
        let Some(Capturing { screens, windows, place, .. }) = self.capturing.take() else { return };
        let live = std::mem::take(&mut self.open_live);
        let (spec_screens, frames) = match live {
            // Listed, without pixels: the overlay freezes them when asked.
            true => (spec_screens_live(screens), Vec::new()),
            false => spec_screens(screens, captures, place.lock().unwrap().take()),
        };
        if spec_screens.is_empty() {
            return self.fail("Could not capture the screen.".into());
        }
        if self.config.measure.identify_elements && !elements::trusted() {
            elements::request_trust();
        }
        let tips = !self.overlay_tips_seen;
        let spec = Spec { screens: spec_screens, tips, live, ..self.overlay_spec() };
        if let Err(err) = self.open_overlay(spec, &frames, windows) {
            self.stand_by();
            return self.fail(format!("Could not open asides: {err:#}"));
        }
        if tips {
            self.overlay_tips_seen = true;
            self.publish();
        }
        let can_listen = self.models.state(&self.config.model).is_some_and(|s| !matches!(s, ModelState::Failed(_)));
        if self.config.mic_on_open && can_listen && self.recording.is_none() && self.mic == Mic::Granted {
            self.start_recording(None, None, false);
        }
        self.refresh();
        self.publish();
    }

    /// The overlay's settings, from the config.
    fn overlay_spec(&mut self) -> Spec {
        let targets = self.targets();
        self.target = targets.chosen.clone();
        let m = &self.config.measure;
        Spec {
            color: m.color.clone(),
            units: m.units,
            tolerance: m.tolerance.min(100),
            detail: self.config.annotation_detail,
            padding: m.padding.max(0.0),
            identify: m.identify_elements,
            keys: m.keys.clone(),
            record_shortcut: asides_core::config::pretty_shortcut(&self.config.shortcuts.asides),
            freeze_shortcut: asides_core::config::pretty_shortcut(&self.config.shortcuts.freeze),
            recording: self.recording.as_ref().map(|r| r.started_at.elapsed().as_secs_f32()),
            listen: self.config.mic_on_open,
            model: self.model_info(),
            targets,
            saved: self.last_measure.clone(),
            pill_at: self.pill_at,
            ..Spec::default()
        }
    }

    fn open_overlay(
        &mut self,
        spec: Spec,
        frames: &[asides_core::measure::Frame],
        windows: Vec<WindowInfo>,
    ) -> anyhow::Result<()> {
        // A script plays without windows, in a process of its own: the one standing by stays.
        let standby = if spec.script.is_none() { self.standby.take() } else { None };
        let live = spec.live;
        let mut overlay = overlay::start(spec, frames, windows, standby, self.overlay_notify())?;
        overlay.previous_app = macos::frontmost_app();
        overlay.frozen = !live;
        self.dictations.clear();
        self.overlay = Some(overlay);
        Ok(())
    }

    /// Tells the overlay whether a recording is under way (its pill shows it).
    fn tell_overlay_recording(&mut self) {
        let seconds = self.recording.as_ref().map(|r| r.started_at.elapsed().as_secs_f32());
        let annotation = self.recording.as_ref().and_then(|r| r.annotation);
        if let Some(overlay) = &mut self.overlay {
            overlay.send(&OverlayCommand::Recording { seconds, annotation });
        }
    }

    /// Starts the next overlay ahead of time (`overlay::Standby`), unless one is open or waiting.
    fn stand_by(&mut self) {
        if self.overlay.is_some() || self.standby.is_some() {
            return;
        }
        match overlay::standby(self.overlay_notify()) {
            Ok(standby) => self.standby = Some(standby),
            Err(err) => eprintln!("asides: no overlay standing by: {err:#}"),
        }
    }

    fn overlay_notify(&self) -> impl overlay::Notify {
        let proxy = Mutex::new(self.proxy.clone());
        move |pid, message| {
            let _ = proxy.lock().unwrap().send_event(UserEvent::Overlay(pid, message));
        }
    }

    fn on_overlay(&mut self, pid: u32, message: OverlayMessage) {
        // The one standing by quit: asides opens in a process of its own, until the next session ends.
        if self.standby.as_ref().is_some_and(|s| s.pid() == pid) {
            if matches!(message, OverlayMessage::Exited) {
                self.standby = None;
            }
            return;
        }
        let Some(overlay) = self.overlay.as_mut().filter(|o| o.pid() == pid as i32) else { return };
        match message {
            OverlayMessage::Event(OverlayEvent::Ready) => overlay.opened = Some(Instant::now()),
            OverlayMessage::Event(OverlayEvent::Identify { id, x, y }) => {
                let deep = self.config.measure.identify_elements;
                let element = elements::identify(&overlay.windows, x, y, deep);
                overlay.send(&OverlayCommand::Element { id, element: element.map(Box::new) });
            }
            OverlayMessage::Event(OverlayEvent::Talk { down, annotation }) => self.talk(down, annotation),
            OverlayMessage::Event(OverlayEvent::Mic { on }) => match (on, self.recording.is_some()) {
                (true, false) => self.start_recording(None, None, false),
                (false, true) => self.stop_recording(),
                _ => {}
            },
            OverlayMessage::Event(OverlayEvent::DownloadModel) => {
                self.models.download(&self.config.model);
                self.model_state = self.models.state(&self.config.model);
                self.tell_overlay_model();
                self.refresh();
            }
            OverlayMessage::Event(OverlayEvent::Stop) => self.stop_recording(),
            OverlayMessage::Event(OverlayEvent::Unsay { annotation }) => {
                self.said.retain(|s| s.0 != annotation);
                // Still being said: it goes too, without a word.
                if self.recording.as_ref().is_some_and(|r| r.annotation == Some(annotation))
                    && let Some(recording) = self.recording.take()
                {
                    drop(recording.recorder.finish());
                    self.tell_overlay_recording();
                    self.refresh();
                }
            }
            OverlayMessage::Event(OverlayEvent::Freeze) => {
                overlay.frozen = true;
                self.capture_screens();
            }
            OverlayMessage::Event(OverlayEvent::Live) => {
                // The overlay gives the keyboard back to the app the user was in.
                overlay.frozen = false;
                self.refresh();
            }
            OverlayMessage::Event(OverlayEvent::Target { target }) => self.target = Some(target),
            OverlayMessage::Event(OverlayEvent::PillPlaced { at }) => {
                self.pill_at = at;
                self.publish();
            }
            OverlayMessage::Event(OverlayEvent::Copied) => {}
            OverlayMessage::Event(OverlayEvent::Finished) => {
                overlay.over = true;
                let origin = overlay.origin();
                match overlay.result() {
                    Ok((outcome, images)) => {
                        self.last_measure = outcome.saved.clone();
                        // A recording about an annotation ends: transcribed for it.
                        if self.recording.as_ref().is_some_and(|r| r.annotation.is_some()) {
                            self.stop_recording();
                        }
                        // What was said in it and what was annotated, as one aside.
                        let mut takes = std::mem::take(&mut self.takes);
                        takes.extend(self.end_recording());
                        let last_take = self.next_take - 1;
                        let dictations = std::mem::take(&mut self.dictations);
                        self.finishing = Some(Finishing { outcome, images, takes, origin, last_take, dictations });
                        self.try_finish();
                    }
                    Err(err) => self.fail(format!("asides: {err:#}")),
                }
            }
            OverlayMessage::Event(OverlayEvent::Cancelled { saved }) => {
                overlay.over = true;
                if !saved.is_empty() {
                    self.last_measure = saved;
                }
                // Esc sends nothing, what was said in it included. (A session before it may still
                // be finishing.)
                let before = self.finishing.as_ref().map_or(0, |f| f.last_take);
                self.said.retain(|s| s.1 <= before);
                let talked = !self.takes.is_empty() || self.recording.is_some();
                self.takes.clear();
                self.dictations.clear();
                if let Some(recording) = self.recording.take() {
                    drop(recording.recorder.finish());
                    self.tell_overlay_recording();
                }
                if talked {
                    self.sound("Funk");
                }
                self.flash(FLASH, Mark::Cross, "Closed: nothing was sent");
                self.refresh();
                // The CLI and the window stop saying it's listening.
                self.publish();
            }
            OverlayMessage::Exited => {
                let crashed = !overlay.over;
                // Closing, the overlay gives the keyboard back itself (frozen; live, it's the user's,
                // in whatever app they went on to). Crashed while frozen, it couldn't.
                let previous = overlay.previous_app.filter(|_| crashed && overlay.frozen);
                // Ended without Send or Esc: what was said is kept, in the history.
                if crashed {
                    self.keep_takes();
                }
                if self.finishing.is_none() {
                    self.said.clear();
                }
                self.overlay = None;
                if let Some(pid) = previous {
                    macos::activate_app(pid);
                }
                if crashed {
                    self.fail("asides' overlay closed unexpectedly. What you said is in History.".into());
                }
                self.stand_by();
                self.refresh();
            }
        }
    }

    /// The overlay's talk key (Space held while muted) or an annotation's microphone (a tap toggles,
    /// a longer press records until it's let go).
    fn talk(&mut self, down: bool, annotation: Option<u32>) {
        match (down, annotation) {
            (true, Some(_)) if self.recording.is_some() => self.stop_recording(),
            (true, Some(_)) => self.start_recording(Some(Instant::now()), annotation, false),
            (false, Some(_)) => {
                let held = self.recording.as_ref().and_then(|r| r.pressed).is_some_and(|at| at.elapsed() >= TAP_MAX);
                if held {
                    self.stop_recording();
                }
            }
            (true, None) if self.recording.is_none() => self.start_recording(Some(Instant::now()), None, true),
            (true, None) => {}
            (false, None) => {
                if self.recording.as_ref().is_some_and(|r| r.push_to_talk) {
                    self.stop_recording();
                }
            }
        }
    }

    /// Keeps a finished session once what was said about its annotations is transcribed: into
    /// their blocks (what the overlay didn't have yet), then into the aside.
    fn try_finish(&mut self) {
        let Some(finishing) = &self.finishing else { return };
        let keys: Vec<u32> = finishing.outcome.items.iter().filter_map(|i| i.annotation).collect();
        let last = finishing.last_take;
        if self.said.iter().any(|&(key, take, ref text)| take <= last && keys.contains(&key) && text.is_none()) {
            return;
        }
        let Some(mut f) = self.finishing.take() else { return };
        let detail = self.config.annotation_detail;
        for item in &mut f.outcome.items {
            let (Some(key), Some(block)) = (item.annotation, item.text.as_mut()) else { continue };
            let had = std::mem::take(&mut item.said);
            for (_, _, text) in self.said.iter().filter(|s| s.1 <= last && s.0 == key && !had.contains(&s.1)) {
                *block = inspect::with_said(block, text.as_deref().unwrap_or_default(), detail);
            }
        }
        self.said.retain(|s| s.1 > last);
        let target = f.outcome.target.clone().or_else(|| self.target.clone());
        self.add_session(f.takes, f.outcome, f.images, f.dictations, f.origin, target);
    }

    /// A finished session, as one aside: its takes joined into one recording (`asides_core::takes`),
    /// with each screenshot, annotation and dictation at the moment it was made. `origin` is when
    /// the overlay opened; the items' times are seconds since.
    fn add_session(
        &mut self,
        takes: Vec<Take>,
        outcome: OverlayOutcome,
        images: Vec<Option<Vec<u8>>>,
        dictations: Vec<(f32, String)>,
        origin: Instant,
        target: Option<Target>,
    ) {
        let mut takes: Vec<Take> = takes.into_iter().filter(|t| !t.accidental()).collect();
        let opened = wall_clock(origin);
        if takes.is_empty() {
            // Nothing said: what was made, in order.
            if outcome.items.is_empty() && dictations.is_empty() {
                return;
            }
            let (pictures, mut texts, pins) = inspection_parts(outcome, images, |time| time);
            texts.extend(dictations);
            return self.submit(Job::Aside(Box::new(AsideJob {
                samples: Vec::new(),
                sample_rate: 16_000,
                started: opened,
                images: pictures,
                texts,
                keep_audio: false,
                model: self.config.model.clone(),
                languages: self.config.languages.clone(),
                takes: Vec::new(),
                target,
                pins,
            })));
        }
        takes.sort_by_key(|t| t.started_at);
        let starts: Vec<f32> = takes.iter().map(|t| overlay::seconds_between(origin, t.started_at)).collect();
        let parts: Vec<takes::Take> = takes
            .iter()
            .zip(&starts)
            .map(|(t, &start)| takes::Take { start, samples: &t.samples, sample_rate: t.sample_rate })
            .collect();
        let joined = takes::join(&parts);
        drop(parts);
        let (mut pictures, mut texts, pins) = inspection_parts(outcome, images, |time| joined.offset(time));
        texts.extend(dictations.into_iter().map(|(time, text)| (joined.offset(time), text)));
        let started = takes.first().map_or(opened, |t| t.started.min(opened));
        for (take, start) in takes.into_iter().zip(starts) {
            pictures.extend(
                take.images.into_iter().map(|i| CapturedImage { offset: joined.offset(start + i.offset), ..i }),
            );
            texts.extend(take.texts.into_iter().map(|(offset, text)| (joined.offset(start + offset), text)));
        }
        let parts = joined.takes();
        self.submit(Job::Aside(Box::new(AsideJob {
            samples: joined.samples,
            sample_rate: joined.sample_rate,
            started,
            images: pictures,
            texts,
            keep_audio: self.config.keep_audio,
            model: self.config.model.clone(),
            languages: self.config.languages.clone(),
            takes: parts,
            target,
            pins,
        })));
    }

    /// What was said in a session that ends without Send or Esc (it crashed, or asides quits): kept
    /// in the history.
    fn keep_takes(&mut self) {
        let mut takes = std::mem::take(&mut self.takes);
        takes.extend(self.end_recording());
        if !takes.is_empty() {
            let origin = self.overlay.as_ref().map_or_else(Instant::now, |o| o.origin());
            let dictations = std::mem::take(&mut self.dictations);
            self.add_session(takes, OverlayOutcome::default(), Vec::new(), dictations, origin, Some(Target::History));
        }
    }

    fn open_window(&mut self) {
        self.open_window_at(None);
    }

    /// The asides window (History, Sessions, Settings): `asides-overlay --window`; brought forward
    /// when it's open, at `page` (`history`, `general`…) when there's one.
    fn open_window_at(&mut self, page: Option<&str>) {
        if let Some(child) = &mut self.window
            && child.try_wait().ok().flatten().is_none()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                use std::io::Write;
                let _ = writeln!(stdin, "show {}", page.unwrap_or_default());
            }
            return;
        }
        let mut command = std::process::Command::new(overlay::helper());
        command.arg(asides_core::helper::WINDOW).stdin(std::process::Stdio::piped());
        if let Some(page) = page {
            command.env("ASIDES_WINDOW_PAGE", page);
        }
        let spawned = command.spawn();
        match spawned {
            Ok(child) => self.window = Some(child),
            Err(err) => self.fail(format!("Could not open the asides window: {err}")),
        }
    }

    /// Looks for the coding agents installed (in the login shell, which can take a moment).
    fn look_for_agents(&mut self) {
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let _ = proxy.send_event(UserEvent::Agents(terminal::installed_agents()));
        });
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
        self.config.wispr_flow.then(asides_core::wispr::default_db)
    }

    /// Removes asides older than the history keeps.
    fn prune_history(&self) {
        let days = self.config.history_days;
        std::thread::spawn(move || {
            let removed = history::prune(&history::dir(), days, std::time::SystemTime::now());
            if removed > 0 {
                eprintln!("asides: removed {removed} asides older than {days} days from the history");
            }
        });
    }

    fn submit(&mut self, job: Job) {
        self.in_flight += 1;
        if self.jobs.send(job).is_err() {
            self.in_flight -= 1;
            self.fail("The transcription worker stopped. Please restart asides.".into());
        }
        self.refresh();
        self.publish();
    }

    fn tick(&mut self) {
        if self.flash.as_ref().is_some_and(|f| Instant::now() >= f.until) {
            self.flash = None;
        }
        // While the window is open, what macOS allows shows there (setup's checks).
        if self.window_open() && self.permissions_at.is_none_or(|at| at.elapsed() >= PERMISSIONS_TICK) {
            self.check_permissions();
        }
        // The pill's microphone fills with what it hears.
        if let (Some(recording), Some(overlay)) = (&self.recording, &mut self.overlay) {
            overlay.send(&OverlayCommand::Level { level: recording.recorder.level() });
        }
        self.refresh();
    }

    /// Shows `mark` between the brackets for a moment, and `status` atop the menu.
    fn flash(&mut self, how_long: Duration, mark: Mark, status: impl Into<String>) {
        self.flash = Some(Flash { until: Instant::now() + how_long, mark, status: status.into() });
    }

    fn next_wakeup(&self) -> Option<Instant> {
        let now = Instant::now();
        let error_icon = self.error.as_ref().is_some_and(|(at, _)| now < *at + ERROR_ICON);
        if self.recording.is_some() && self.overlay.is_some() {
            return Some(now + LEVEL_TICK);
        }
        if self.recording.is_some() || self.flash.is_some() || error_icon {
            return Some(now + TICK);
        }
        self.window.is_some().then_some(now + PERMISSIONS_TICK)
    }

    fn refresh(&mut self) {
        let Some(icons) = &self.icons else { return };
        let now = Instant::now();
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
        let shortcut = asides_core::config::pretty_shortcut(&self.config.shortcuts.asides);

        let (mark, title, status) = if let Some(recording) = &self.recording {
            let secs = recording.started_at.elapsed().as_secs();
            let clock = format!("{}:{:02}", secs / 60, secs % 60);
            (Mark::Dot, Some(clock.clone()), format!("● Listening {clock}"))
        } else if self.in_flight > 0 {
            let status = match downloading {
                Some(p) => format!("Waiting for the voice model download… {p}%"),
                None => "Transcribing…".into(),
            };
            (Mark::Ellipsis, None, status)
        } else if let Some(flash) = &self.flash {
            (flash.mark, None, flash.status.clone())
        } else if error_icon {
            (Mark::Exclamation, None, format!("⚠ {}", error_text.clone().unwrap_or_default()))
        } else if let Some(p) = downloading {
            (Mark::Arrow, Some(format!("{p}%")), format!("Downloading the voice model… {p}%"))
        } else if self.overlay.is_some() {
            (Mark::Letter, None, format!("asides is open ({shortcut} sends)"))
        } else {
            let voice = if ModelSpec::find(&self.config.model).downloaded() { "" } else { ", no voice model yet" };
            let status = match &error_text {
                Some(message) => format!("⚠ {message}"),
                None => format!("Ready ({shortcut} opens asides{voice})"),
            };
            (Mark::Letter, None, status)
        };
        let has_last = self.last_aside.is_some();
        let updating = &self.updates.status;
        let version = updating.release.as_ref().map_or("", |r| r.version.as_str());
        let update = match updating.phase {
            Phase::Available => Some((format!("Update to {version}"), true)),
            Phase::Downloading if self.updates.restarting_after_download() => {
                Some((format!("Downloading {version}… {:.0}%", updating.progress * 100.0), false))
            }
            Phase::Ready => Some((format!("Restart to Update to {version}"), true)),
            Phase::Installing => Some(("Restarting…".into(), false)),
            _ => None,
        };
        let open = self.overlay.is_some();
        let view = View { mark, icon: icons.get(mark), title, status, open, has_last, update };
        if let Some(tray) = &mut self.tray {
            tray.show(&view);
        }
    }

    /// Publishes the app's state for the CLI and the asides window.
    fn publish(&self) {
        let status = if self.recording.is_some() {
            "recording"
        } else if self.in_flight > 0 {
            "transcribing"
        } else if self.sending > 0 {
            "sending"
        } else if self.overlay.is_some() {
            "open"
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
                Some(format!("Downloading the voice model: {:.0}%", progress * 100.0))
            }
            _ => None,
        };
        let state = AppState {
            pid: if self.quit { 0 } else { std::process::id() },
            status: status.into(),
            detail,
            microphone: self.mic.as_str().into(),
            last_request: self.last_request.clone(),
            sessions: self.sessions.clone(),
            target: match &self.target {
                Some(Target::Session { id }) => Some(id.clone()),
                _ => None,
            },
            recent_dirs: self.recent_dirs.clone(),
            agents: self.agents.clone().unwrap_or_default(),
            last_aside: self.last_aside.clone(),
            model_ready: ModelSpec::find(&self.config.model).downloaded(),
            input_devices: self.input_devices.clone(),
            command_link_declined: self.command_link_declined,
            overlay_tips_seen: self.overlay_tips_seen,
            pill_at: self.pill_at,
            update: self.updates.status.clone(),
            whats_new_seen: Some(self.whats_new_seen.clone()),
            permissions: self.permissions.clone(),
            onboarded: self.onboarded,
            setup_step: self.setup_step.clone(),
            updated_ms: history::now_ms(),
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

    /// Looks at what macOS lets asides do (off the main thread: Screen Recording takes 11 ms, and
    /// Apple events wait on the app asked about).
    fn check_permissions(&mut self) {
        if std::mem::replace(&mut self.checking_permissions, true) {
            return;
        }
        let proxy = self.proxy.clone();
        let terminals = deliver::installed_terminals();
        std::thread::spawn(move || {
            let automation = terminals
                .into_iter()
                .filter_map(|app| deliver::automated_bundle(app).map(|bundle| (app, bundle)))
                .map(|(app, bundle)| (app.to_string(), macos::automation(bundle, false).as_str().to_string()))
                .collect();
            let permissions = Permissions {
                screen: capture::permitted(),
                accessibility: elements::trusted(),
                automation,
                checked_ms: history::now_ms(),
            };
            let _ = proxy.send_event(UserEvent::Permissions(permissions, macos::mic_status()));
        });
    }

    /// Asks macOS for a permission (setup's buttons): its prompt the first time, then its settings.
    fn ask_permission(&mut self, permission: Permission) {
        match permission {
            Permission::Microphone => self.request_mic(),
            Permission::Screen => {
                capture::request();
            }
            Permission::Accessibility => {
                elements::request_trust();
            }
            Permission::Automation(app) => {
                let Some(bundle) = deliver::automated_bundle(&app) else { return };
                // macOS waits for the answer; the next look at the permissions finds it.
                std::thread::spawn(move || macos::ask_automation(bundle));
            }
        }
        self.permissions_at = None;
    }

    /// The asides window is open (a process that's gone is forgotten).
    fn window_open(&mut self) -> bool {
        if self.window.as_mut().is_some_and(|w| w.try_wait().ok().flatten().is_some()) {
            self.window = None;
        }
        self.window.is_some()
    }

    fn request_quit(&mut self) {
        if let Some(mut window) = self.window.take() {
            let _ = window.kill();
        }
        self.keep_takes();
        if self.in_flight == 0 && self.sending == 0 {
            self.quit = true;
            self.publish();
        } else {
            // Finish the queued transcriptions and sends first.
            self.quit_when_idle = true;
        }
    }

    // --- Updating asides ---

    /// Starts the updater: its first check comes sooner when this copy is new here (for its notes).
    fn start_updater(&mut self) {
        let new_here = Version::parse(&self.whats_new_seen).is_some_and(|seen| seen < Version::current());
        let first = if new_here { Duration::from_secs(3) } else { asides_core::update::FIRST_CHECK };
        let proxy = self.proxy.clone();
        self.updater = Some(update::spawn(self.config.auto_update, first, move |reply| {
            proxy.send_event(UserEvent::Update(reply)).is_ok()
        }));
    }

    fn send_update(&self, request: UpdateRequest) {
        if let Some(updater) = &self.updater {
            updater.send(request);
        }
    }

    fn on_update(&mut self, reply: UpdateReply) {
        let next = self.updates.reply(reply, self.config.auto_update);
        self.follow_update(next);
        self.refresh();
        self.publish();
    }

    /// What the window (or the menu) asked of the updater.
    fn update_step(&mut self, step: Step) {
        match step {
            Step::Seen => self.whats_new_seen = Version::current().to_string(),
            step => {
                let next = self.updates.step(step);
                self.follow_update(next);
            }
        }
        self.refresh();
    }

    fn follow_update(&mut self, next: Vec<Next>) {
        for next in next {
            match next {
                Next::Ask(request) => self.send_update(request),
                Next::Restart => self.restart_into_update(),
            }
        }
    }

    /// Quits to swap the update in, and opens asides again (its window too, if it was open).
    fn restart_into_update(&mut self) {
        let window = self.window.as_mut().is_some_and(|w| w.try_wait().ok().flatten().is_none());
        self.restart = Some(window);
        self.updates.status.phase = Phase::Installing;
        self.publish();
        self.request_quit();
    }

    /// asides is quitting: the update downloaded goes in now. Asked for, asides opens again;
    /// installed as asides quits (automatic updates), it stays closed.
    fn exit(&mut self) {
        let Some(staged) = self.updates.staged.take() else { return };
        let reopen = match self.restart {
            Some(true) => Some("asides://window"),
            Some(false) => Some(""),
            None if self.config.auto_update => None,
            None => return,
        };
        if let Err(err) = update::install(&staged, reopen) {
            eprintln!("asides: could not install {}: {err}", staged.release.version);
        }
    }

    /// The last aside, as Markdown with its screenshots' paths, on the clipboard.
    fn copy_last(&mut self) {
        match self.last_aside.as_ref().and_then(|id| history::find(&history::dir(), id)) {
            Some(aside) => {
                clipboard::put_text(&aside.markdown());
                self.flash(FLASH, Mark::Check, "Copied the last aside");
            }
            None => self.fail("The last aside isn't in History any more.".into()),
        }
        self.refresh();
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
        let shortcuts_changed = config.shortcuts != self.config.shortcuts;
        let model_changed = config.model != self.config.model;
        let history_changed = config.history_days != self.config.history_days;
        let auto_update_changed = config.auto_update != self.config.auto_update;
        self.config = config;
        if auto_update_changed {
            let next = self.updates.automatic(self.config.auto_update);
            self.follow_update(next);
        }
        if model_changed {
            self.model_state = self.models.state(&self.config.model);
            self.tell_overlay_model();
        }
        if shortcuts_changed && let Some(hotkeys) = &mut self.hotkeys {
            hotkeys.apply(&self.config.shortcuts);
            if let Some(err) = hotkeys.errors.first() {
                self.error = Some((Instant::now(), err.clone()));
            }
        }
        if history_changed {
            self.prune_history();
        }
        self.sync_wispr();
        if let Some(tray) = &self.tray {
            tray.sync_config(&self.config);
        }
        self.refresh();
        self.publish();
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
        wav: &Path,
        image: Option<PathBuf>,
        image_at: f32,
        annotation: Option<u32>,
        to: Option<String>,
    ) -> anyhow::Result<()> {
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
                bytes: std::fs::read(&path)?,
                extension: path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_else(|| "png".into()),
                details: None,
            }],
            None => Vec::new(),
        };
        let take = Take {
            started_at: Instant::now(),
            started: Local::now(),
            images,
            texts: Vec::new(),
            samples,
            sample_rate: spec.sample_rate,
            annotation,
        };
        // As if just said: while asides is open, part of its aside; outside it, an aside of its own.
        if self.overlay.as_ref().is_some_and(|o| !o.over) || take.annotation.is_some() {
            self.keep_take(take);
            return Ok(());
        }
        let target = simulated_target(to);
        let origin = take.started_at;
        self.add_session(vec![take], OverlayOutcome::default(), Vec::new(), Vec::new(), origin, Some(target));
        Ok(())
    }

    /// Runs a scripted session on `image` instead of the screen, in the overlay's process without
    /// windows; the result goes to session `to` (or the history only), like the user's own.
    #[cfg(feature = "dev-hooks")]
    fn simulate_measure(&mut self, image: &Path, script: &Path, to: Option<String>) -> anyhow::Result<()> {
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
            place: None,
            menu_bar: 0.0,
        };
        let mut spec = Spec { screens: vec![screen], script: Some(script), ..self.overlay_spec() };
        // Never into the user's own sessions unless asked.
        spec.targets.chosen = Some(simulated_target(to));
        self.target = spec.targets.chosen.clone();
        self.open_overlay(spec, &[frame], Vec::new())
    }
}

/// Where a simulated aside goes: session `to`, or the history only.
#[cfg(feature = "dev-hooks")]
fn simulated_target(to: Option<String>) -> Target {
    match to.as_deref() {
        None | Some("history") => Target::History,
        Some(id) => Target::Session { id: id.to_string() },
    }
}

/// Where the user is (`**In:** …`, and the frame of the window they're in), once it's been looked up.
type Place = Arc<Mutex<Option<(String, PtRect)>>>;

/// Looks up where the user is in app `pid` (`elements::place`) on a thread of its own: an app can
/// take a moment to answer accessibility, and nothing waits for it.
fn look_up_place(pid: Option<i32>, deep: bool) -> Place {
    let place = Place::default();
    if let Some(pid) = pid.filter(|&pid| pid != std::process::id() as i32) {
        let found = place.clone();
        std::thread::spawn(move || *found.lock().unwrap() = elements::place(pid, deep));
    }
    place
}

/// A freeze under way: when it began, the screens and windows as they were, and where the user is.
struct Capturing {
    at: Instant,
    screens: Vec<ScreenInfo>,
    windows: Vec<WindowInfo>,
    place: Place,
}

/// The captured screens, as the overlay gets them, and their pixels; `place` goes with the screen
/// its window is on.
fn spec_screens(
    screens: Vec<ScreenInfo>,
    captures: Vec<Capture>,
    place: Option<(String, PtRect)>,
) -> (Vec<SpecScreen>, Vec<asides_core::measure::Frame>) {
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
            place: place
                .as_ref()
                .filter(|(_, b)| {
                    let (cx, cy) = (b.x + b.width / 2.0, b.y + b.height / 2.0);
                    cx >= s.origin.0 && cx < s.origin.0 + w && cy >= s.origin.1 && cy < s.origin.1 + h
                })
                .map(|(place, _)| place.clone()),
            menu_bar: s.menu_bar,
        });
        frames.push(capture.frame);
    }
    (spec_screens, frames)
}

/// The screens, listed for asides opening over the live screen (no pixels until they freeze).
fn spec_screens_live(screens: Vec<ScreenInfo>) -> Vec<SpecScreen> {
    screens
        .into_iter()
        .map(|s| {
            let (w, h) = (s.frame.size.width, s.frame.size.height);
            SpecScreen {
                display: s.display,
                screen: Screen { name: s.name, width: w, height: h, scale: s.scale },
                origin: s.origin,
                frame: PtRect::new(s.frame.origin.x, s.frame.origin.y, w, h),
                pixels: String::new(),
                width: 0,
                height: 0,
                place: None,
                menu_bar: s.menu_bar,
            }
        })
        .collect()
}

/// A session's images and Markdown, at `offset(seconds into the session)`, and its pins' colors.
fn inspection_parts(
    outcome: OverlayOutcome,
    images: Vec<Option<Vec<u8>>>,
    offset: impl Fn(f32) -> f32,
) -> (Vec<CapturedImage>, Vec<(f32, String)>, Vec<history::Pin>) {
    let mut pictures = Vec::new();
    let mut texts = Vec::new();
    let mut pins = Vec::new();
    for (item, bytes) in outcome.items.into_iter().zip(images) {
        let offset = offset(item.time);
        let number = item.text.as_deref().and_then(asides_core::markdown::pin_number);
        if let (Some(number), Some(color)) = (number, item.color) {
            pins.push(history::Pin { number, color });
        }
        match bytes {
            Some(bytes) => pictures.push(CapturedImage { offset, bytes, extension: "png".into(), details: item.text }),
            None => texts.extend(item.text.map(|text| (offset, text))),
        }
    }
    (pictures, texts, pins)
}

/// Aside `id`'s number among today's (it's the newest): the menu bar shows it (`[3]`).
fn asides_today(id: &str) -> u32 {
    let today = Local::now().date_naive();
    let list = history::list(&history::dir());
    let count = list.iter().filter(|a| a.created().date_naive() == today).count() as u32;
    let position = list.iter().position(|a| a.id == id).unwrap_or(0) as u32;
    count.saturating_sub(position).max(1)
}

/// The time of day at `instant`.
fn wall_clock(instant: Instant) -> DateTime<Local> {
    Local::now() - chrono::TimeDelta::from_std(instant.elapsed()).unwrap_or_default()
}
