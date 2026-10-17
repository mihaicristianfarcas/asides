//! The status item and its menu.

use std::path::{Path, PathBuf};
use std::str::FromStr;

use asides_core::config::{AgentDelivery, Config, pretty_shortcut};
use asides_core::inspect::{Detail, Units};
use asides_core::launch::AgentKind;
use asides_core::models::MODELS;
use asides_core::paths::tilde;
use tray_icon::menu::accelerator::Accelerator;
use tray_icon::menu::{CheckMenuItem, IsMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::icons::StatusIcon;

#[derive(Debug, Clone, PartialEq)]
pub enum MenuAction {
    Record,
    RecordNewNote,
    AttachImage,
    Measure,
    Cancel,
    SendToAgent,
    /// An agent's reply in the Agent Replies menu: opens the note.
    OpenReplies,
    /// Send asides to this session, or (`None`) to the notes only.
    Target(Option<String>),
    /// Start an agent session in `dir` (`None`: choose a directory first).
    NewSession {
        agent: AgentKind,
        dir: Option<PathBuf>,
        background: bool,
    },
    /// Open a background session in a terminal.
    ShowSession(String),
    SwitchDirectory(PathBuf),
    ChooseDirectory,
    OpenNote,
    RevealNote,
    CopyNotePath,
    ToggleHoldToTalk,
    ToggleAutoAttach,
    ToggleSounds,
    ToggleKeepAudio,
    ToggleWisprFlow,
    ToggleIdentifyElements,
    ToggleMeasurePixels,
    ToggleMeasureRecord,
    AnnotationDetail(Detail),
    ScreenRecordingSettings,
    AccessibilitySettings,
    AgentDelivery(AgentDelivery),
    /// The app agent sessions open in.
    Terminal(String),
    Model(String),
    InputDevice(String),
    MicrophoneSettings,
    EditSettings,
    Quit,
}

/// A choice under Asides Go To: an agent session, or Notes Only (`id: None`).
#[derive(Debug, Clone, PartialEq)]
pub struct TargetRow {
    pub id: Option<String>,
    pub label: String,
    pub checked: bool,
}

/// What the session rows show: each choice (id, label), and each Show item (id, label).
type TargetsShape = (Vec<(Option<String>, String)>, Vec<(String, String)>);
/// What New Agent Session offers: the agents found, the bound directory, the recent ones.
type NewSessionShape = (Option<Vec<AgentKind>>, Option<PathBuf>, Vec<PathBuf>);

/// Where the session rows go: after the first separator.
const SESSIONS_AT: usize = 9;

pub struct Tray {
    icon: TrayIcon,
    menu: Menu,
    /// Asides Go To: its title, the choices, and Show items for background sessions.
    target_rows: Vec<(Option<String>, CheckMenuItem)>,
    target_extras: Vec<Box<dyn IsMenuItem>>,
    shows: Vec<(String, MenuItem)>,
    shown_targets: TargetsShape,
    new_session: Submenu,
    new_session_items: Vec<Box<dyn IsMenuItem>>,
    new_session_actions: Vec<(MenuId, MenuAction)>,
    shown_new_session: Option<NewSessionShape>,
    shown: Option<(&'static str, Option<String>)>,
    status: MenuItem,
    record: MenuItem,
    record_new: MenuItem,
    attach: MenuItem,
    measure: MenuItem,
    cancel: MenuItem,
    send: MenuItem,
    replies_menu: Submenu,
    replies: Vec<MenuItem>,
    shown_replies: Vec<String>,
    directory_menu: Submenu,
    /// Recent directories at the top of the Directory submenu, then a separator.
    directories: Vec<(PathBuf, CheckMenuItem)>,
    directories_separator: Option<PredefinedMenuItem>,
    choose_directory: MenuItem,
    note_info: MenuItem,
    open: MenuItem,
    reveal: MenuItem,
    copy_path: MenuItem,
    hold: CheckMenuItem,
    auto_attach: CheckMenuItem,
    sounds: CheckMenuItem,
    keep_audio: CheckMenuItem,
    wispr_flow: CheckMenuItem,
    identify: CheckMenuItem,
    pixels: CheckMenuItem,
    measure_record: CheckMenuItem,
    details: Vec<(Detail, CheckMenuItem)>,
    screen_recording: MenuItem,
    accessibility: MenuItem,
    deliveries: Vec<(AgentDelivery, CheckMenuItem)>,
    terminals: Vec<(String, CheckMenuItem)>,
    models: Vec<(String, CheckMenuItem)>,
    devices: Vec<(String, CheckMenuItem)>,
    microphone_settings: MenuItem,
    edit_settings: MenuItem,
    quit: MenuItem,
}

/// What the menu should show; computed by the app on every change.
pub struct View<'a> {
    pub icon_key: &'static str,
    pub icon: &'a StatusIcon,
    pub title: Option<String>,
    pub status: String,
    pub recording: bool,
    pub bound: Option<&'a Path>,
    pub note_info: String,
    pub has_note: bool,
    /// "Send to Claude Code", or "Send to Agent" when no single agent is listening.
    pub send_label: String,
    pub send_enabled: bool,
    /// What agents said about the latest note's asides, newest first.
    pub replies: &'a [String],
}

impl Tray {
    pub fn new(config: &Config, devices: &[String], terminals: &[String], icon: &StatusIcon) -> anyhow::Result<Tray> {
        let status = MenuItem::new("", false, None);
        let record = MenuItem::new("Start Recording", false, accelerator(&config.shortcuts.record));
        let record_new = MenuItem::new("Start in New Note", false, accelerator(&config.shortcuts.record_new_note));
        let attach = MenuItem::new("Attach Clipboard Image", false, accelerator(&config.shortcuts.attach_image));
        let measure =
            MenuItem::new(menu_text("Measure & Annotate Screen"), true, accelerator(&config.shortcuts.measure));
        let replies_menu = Submenu::new("Agent Replies", false);
        let cancel = MenuItem::new(cancel_label(config), false, None);
        let send = MenuItem::new("Send to Agent", false, accelerator(&config.shortcuts.send_to_agent));
        let new_session = Submenu::new("New Agent Session", true);
        let choose_directory = MenuItem::new("Choose Directory…", true, None);
        let directory_menu = Submenu::new("Directory", true);
        directory_menu.append(&choose_directory)?;
        let note_info = MenuItem::new("", false, None);
        let open = MenuItem::new("Open Note", false, None);
        let reveal = MenuItem::new("Reveal in Finder", false, None);
        let copy_path = MenuItem::new("Copy Note Path", false, None);

        let hold = CheckMenuItem::new("Hold to Talk (tap toggles)", true, config.hold_to_talk, None);
        let auto_attach =
            CheckMenuItem::new("Auto-attach Images Copied While Recording", true, config.auto_attach_images, None);
        let sounds = CheckMenuItem::new("Sounds", true, config.sounds, None);
        let keep_audio = CheckMenuItem::new("Keep Audio Clips", true, config.keep_audio, None);
        let wispr_flow = CheckMenuItem::new("Add Wispr Flow Dictations", true, config.wispr_flow, None);
        let identify = CheckMenuItem::new(
            "Identify UI Elements (needs Accessibility)",
            true,
            config.measure.identify_elements,
            None,
        );
        let pixels = CheckMenuItem::new("Measure in Pixels, Not Points", true, config.measure.units == Units::Px, None);
        let measure_record = CheckMenuItem::new("Start Recording When It Opens", true, config.measure.record, None);
        let details: Vec<(Detail, CheckMenuItem)> = [
            (Detail::Compact, "Compact"),
            (Detail::Standard, "Standard"),
            (Detail::Detailed, "Detailed"),
            (Detail::Forensic, "Forensic (Colors, Padding, Accessibility)"),
        ]
        .into_iter()
        .map(|(detail, label)| (detail, CheckMenuItem::new(label, true, detail == config.annotation_detail, None)))
        .collect();
        let detail_menu = Submenu::new("Annotation Detail", true);
        for (_, item) in &details {
            detail_menu.append(item)?;
        }
        let screen_recording = MenuItem::new("Screen Recording Permission…", true, None);
        let accessibility = MenuItem::new("Accessibility Permission…", true, None);
        let measure_menu = Submenu::new(menu_text("Measure & Annotate"), true);
        measure_menu.append_items(&[
            &measure_record,
            &identify,
            &pixels,
            &detail_menu,
            &PredefinedMenuItem::separator(),
            &screen_recording,
            &accessibility,
        ])?;
        let deliveries: Vec<(AgentDelivery, CheckMenuItem)> = [
            (AgentDelivery::Each, "Each Aside as It's Transcribed"),
            (AgentDelivery::Batch, "In Batches, with Send to Agent"),
        ]
        .into_iter()
        .map(|(mode, label)| (mode, CheckMenuItem::new(label, true, mode == config.agent_delivery, None)))
        .collect();
        let delivery_menu = Submenu::new("Agents Get Asides", true);
        for (_, item) in &deliveries {
            delivery_menu.append(item)?;
        }
        // The installed ones, and one named in the settings that isn't among them.
        let chosen = terminal_name(config);
        let mut names = terminals.to_vec();
        if !names.iter().any(|n| n.eq_ignore_ascii_case(&chosen)) {
            names.push(chosen.clone());
        }
        let terminals: Vec<(String, CheckMenuItem)> = names
            .into_iter()
            .map(|name| {
                let checked = name.eq_ignore_ascii_case(&chosen);
                let item = CheckMenuItem::new(menu_text(&name), true, checked, None);
                (name, item)
            })
            .collect();
        let terminal_menu = Submenu::new("Open Agent Sessions In", true);
        for (_, item) in &terminals {
            terminal_menu.append(item)?;
        }
        let models: Vec<(String, CheckMenuItem)> = MODELS
            .iter()
            .map(|m| (m.id.to_string(), CheckMenuItem::new(m.label, true, m.id == config.model, None)))
            .collect();
        let model_menu = Submenu::new("Speech Model", true);
        for (_, item) in &models {
            model_menu.append(item)?;
        }
        let mut devices_items =
            vec![(String::new(), CheckMenuItem::new("System Default", true, config.input_device.is_empty(), None))];
        for name in devices {
            devices_items.push((name.clone(), CheckMenuItem::new(name, true, *name == config.input_device, None)));
        }
        let device_menu = Submenu::new("Microphone", true);
        for (_, item) in &devices_items {
            device_menu.append(item)?;
        }
        let microphone_settings = MenuItem::new("Microphone Permission…", true, None);
        let edit_settings = MenuItem::new(menu_text("Edit Shortcuts & Settings…"), true, None);
        let preferences = Submenu::new("Preferences", true);
        preferences.append_items(&[
            &hold,
            &auto_attach,
            &sounds,
            &keep_audio,
            &wispr_flow,
            &measure_menu,
            &PredefinedMenuItem::separator(),
            &model_menu,
            &device_menu,
            &delivery_menu,
            &terminal_menu,
            &PredefinedMenuItem::separator(),
            &microphone_settings,
            &edit_settings,
        ])?;
        let quit = MenuItem::new("Quit Asides", true, Accelerator::from_str("cmd+KeyQ").ok());

        let menu = Menu::new();
        menu.append_items(&[
            &status,
            &record,
            &record_new,
            &attach,
            &measure,
            &cancel,
            &send,
            &replies_menu,
            &PredefinedMenuItem::separator(),
            &new_session,
            &PredefinedMenuItem::separator(),
            &directory_menu,
            &note_info,
            &open,
            &reveal,
            &copy_path,
            &PredefinedMenuItem::separator(),
            &preferences,
            &PredefinedMenuItem::separator(),
            &quit,
        ])?;

        let builder = TrayIconBuilder::new()
            .with_menu(Box::new(menu.clone()))
            .with_menu_on_left_click(true)
            .with_tooltip("Asides");
        let builder = if icon.template {
            builder.with_icon_templated(icon.icon.clone())
        } else {
            builder.with_icon(icon.icon.clone())
        };
        let icon = builder.build()?;

        Ok(Tray {
            icon,
            menu,
            target_rows: Vec::new(),
            target_extras: Vec::new(),
            shows: Vec::new(),
            shown_targets: (Vec::new(), Vec::new()),
            new_session,
            new_session_items: Vec::new(),
            new_session_actions: Vec::new(),
            shown_new_session: None,
            shown: None,
            status,
            record,
            record_new,
            attach,
            measure,
            cancel,
            send,
            replies_menu,
            replies: Vec::new(),
            shown_replies: Vec::new(),
            directory_menu,
            directories: Vec::new(),
            directories_separator: None,
            choose_directory,
            note_info,
            open,
            reveal,
            copy_path,
            hold,
            auto_attach,
            sounds,
            keep_audio,
            wispr_flow,
            identify,
            pixels,
            measure_record,
            details,
            screen_recording,
            accessibility,
            deliveries,
            terminals,
            models,
            devices: devices_items,
            microphone_settings,
            edit_settings,
            quit,
        })
    }

    pub fn action(&self, id: &MenuId) -> Option<MenuAction> {
        let simple = [
            (&self.record, MenuAction::Record),
            (&self.record_new, MenuAction::RecordNewNote),
            (&self.attach, MenuAction::AttachImage),
            (&self.measure, MenuAction::Measure),
            (&self.screen_recording, MenuAction::ScreenRecordingSettings),
            (&self.accessibility, MenuAction::AccessibilitySettings),
            (&self.cancel, MenuAction::Cancel),
            (&self.send, MenuAction::SendToAgent),
            (&self.choose_directory, MenuAction::ChooseDirectory),
            (&self.open, MenuAction::OpenNote),
            (&self.reveal, MenuAction::RevealNote),
            (&self.copy_path, MenuAction::CopyNotePath),
            (&self.microphone_settings, MenuAction::MicrophoneSettings),
            (&self.edit_settings, MenuAction::EditSettings),
            (&self.quit, MenuAction::Quit),
        ];
        if let Some((_, action)) = simple.into_iter().find(|(item, _)| item.id() == id) {
            return Some(action);
        }
        let checks = [
            (&self.hold, MenuAction::ToggleHoldToTalk),
            (&self.auto_attach, MenuAction::ToggleAutoAttach),
            (&self.sounds, MenuAction::ToggleSounds),
            (&self.keep_audio, MenuAction::ToggleKeepAudio),
            (&self.wispr_flow, MenuAction::ToggleWisprFlow),
            (&self.identify, MenuAction::ToggleIdentifyElements),
            (&self.pixels, MenuAction::ToggleMeasurePixels),
            (&self.measure_record, MenuAction::ToggleMeasureRecord),
        ];
        if let Some((_, action)) = checks.into_iter().find(|(item, _)| item.id() == id) {
            return Some(action);
        }
        if self.replies.iter().any(|item| item.id() == id) {
            return Some(MenuAction::OpenReplies);
        }
        if let Some((target, _)) = self.target_rows.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::Target(target.clone()));
        }
        if let Some((session, _)) = self.shows.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::ShowSession(session.clone()));
        }
        if let Some((_, action)) = self.new_session_actions.iter().find(|(item, _)| item == id) {
            return Some(action.clone());
        }
        if let Some((detail, _)) = self.details.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::AnnotationDetail(*detail));
        }
        if let Some((mode, _)) = self.deliveries.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::AgentDelivery(*mode));
        }
        if let Some((name, _)) = self.terminals.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::Terminal(name.clone()));
        }
        if let Some((dir, _)) = self.directories.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::SwitchDirectory(dir.clone()));
        }
        if let Some((model, _)) = self.models.iter().find(|(_, item)| item.id() == id) {
            return Some(MenuAction::Model(model.clone()));
        }
        self.devices.iter().find(|(_, item)| item.id() == id).map(|(name, _)| MenuAction::InputDevice(name.clone()))
    }

    pub fn show(&mut self, view: &View) {
        let shown = (view.icon_key, view.title.clone());
        if self.shown.as_ref().map(|s| s.0) != Some(view.icon_key) {
            let icon = Some(view.icon.icon.clone());
            let _ = if view.icon.template { self.icon.set_icon_templated(icon) } else { self.icon.set_icon(icon) };
        }
        if self.shown.as_ref().map(|s| &s.1) != Some(&view.title) {
            self.icon.set_title(view.title.as_deref());
        }
        self.shown = Some(shown);

        self.status.set_text(menu_text(&view.status));
        self.record.set_text(if view.recording { "Stop Recording" } else { "Start Recording" });
        self.record.set_enabled(view.bound.is_some());
        self.record_new.set_enabled(view.bound.is_some() && !view.recording);
        self.attach.set_enabled(view.bound.is_some());
        self.cancel.set_enabled(view.recording);
        self.send.set_text(menu_text(&view.send_label));
        self.send.set_enabled(view.send_enabled);
        let directory = view.bound.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned());
        self.directory_menu.set_text(menu_text(&format!("Directory: {}", directory.as_deref().unwrap_or("none"))));
        for (dir, item) in &self.directories {
            item.set_checked(Some(dir.as_path()) == view.bound);
        }
        self.note_info.set_text(menu_text(&view.note_info));
        for item in [&self.open, &self.reveal, &self.copy_path] {
            item.set_enabled(view.has_note);
        }
        self.set_replies(view.replies);
    }

    /// Rebuilds the Agent Replies submenu when what agents said changes.
    fn set_replies(&mut self, replies: &[String]) {
        if self.shown_replies == replies {
            return;
        }
        for item in self.replies.drain(..) {
            let _ = self.replies_menu.remove(&item);
        }
        for text in replies {
            let item = MenuItem::new(menu_text(text), true, None);
            let _ = self.replies_menu.append(&item);
            self.replies.push(item);
        }
        self.replies_menu.set_enabled(!replies.is_empty());
        self.replies_menu.set_text(match replies.len() {
            0 => "Agent Replies".to_string(),
            n => format!("Agent Replies ({n})"),
        });
        self.shown_replies = replies.to_vec();
    }

    /// Shows where asides go (with sessions around): the sessions and Notes Only, the target
    /// checked, and a Show item for each background session (`(id, label)`).
    pub fn set_targets(&mut self, rows: &[TargetRow], shows: &[(String, String)]) {
        let shape = (rows.iter().map(|r| (r.id.clone(), r.label.clone())).collect::<Vec<_>>(), shows.to_vec());
        if self.shown_targets != shape {
            for (_, item) in self.target_rows.drain(..) {
                let _ = self.menu.remove(&item);
            }
            for (_, item) in self.shows.drain(..) {
                let _ = self.menu.remove(&item);
            }
            for item in self.target_extras.drain(..) {
                let _ = self.menu.remove(item.as_ref());
            }
            let mut at = SESSIONS_AT;
            let mut insert = |menu: &Menu, item: &dyn IsMenuItem| {
                let _ = menu.insert(item, at);
                at += 1;
            };
            if !rows.is_empty() {
                let title = MenuItem::new("Asides Go To", false, None);
                insert(&self.menu, &title);
                self.target_extras.push(Box::new(title));
            }
            for row in rows {
                let item = CheckMenuItem::new(menu_text(&row.label), true, row.checked, None);
                insert(&self.menu, &item);
                self.target_rows.push((row.id.clone(), item));
            }
            for (id, label) in shows {
                let item = MenuItem::new(menu_text(&format!("Show {label}")), true, None);
                insert(&self.menu, &item);
                self.shows.push((id.clone(), item));
            }
            self.shown_targets = shape;
        }
        for ((_, item), row) in self.target_rows.iter().zip(rows) {
            item.set_checked(row.checked);
        }
    }

    /// The New Agent Session submenu: each agent (`None` while still looking for them) in the bound
    /// directory, in recent ones, or in one to choose.
    pub fn set_new_session(&mut self, agents: Option<&[AgentKind]>, here: Option<&Path>, recent: &[PathBuf]) {
        let shape: NewSessionShape = (agents.map(<[AgentKind]>::to_vec), here.map(Path::to_path_buf), recent.to_vec());
        if self.shown_new_session.as_ref() == Some(&shape) {
            return;
        }
        for item in self.new_session_items.drain(..) {
            let _ = self.new_session.remove(item.as_ref());
        }
        self.new_session_actions.clear();
        let mut items: Vec<Box<dyn IsMenuItem>> = Vec::new();
        let mut actions = Vec::new();
        let agent_items = |dir: Option<&Path>, actions: &mut Vec<(MenuId, MenuAction)>| {
            let mut items: Vec<MenuItem> = Vec::new();
            for &agent in agents.unwrap_or_default() {
                let mut add = |label: String, background: bool| {
                    let item = MenuItem::new(menu_text(&label), true, None);
                    let dir = dir.map(Path::to_path_buf);
                    actions.push((item.id().clone(), MenuAction::NewSession { agent, dir, background }));
                    items.push(item);
                };
                add(agent.name().into(), false);
                if agent.runs_in_background() {
                    add(format!("{} in the Background", agent.name()), true);
                }
            }
            items
        };
        match agents {
            None => items.push(Box::new(MenuItem::new("Looking for coding agents…", false, None))),
            Some([]) => items.push(Box::new(MenuItem::new(
                "No coding agents found (Claude Code, Codex or OpenCode)",
                false,
                None,
            ))),
            Some(_) => {
                if let Some(here) = here {
                    let name = here.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    items.push(Box::new(MenuItem::new(menu_text(&format!("In {name}")), false, None)));
                    items.extend(
                        agent_items(Some(here), &mut actions).into_iter().map(|i| Box::new(i) as Box<dyn IsMenuItem>),
                    );
                    items.push(Box::new(PredefinedMenuItem::separator()));
                }
                for dir in recent.iter().filter(|d| Some(d.as_path()) != here) {
                    let menu = Submenu::new(menu_text(&format!("In {}", tilde(dir))), true);
                    for item in agent_items(Some(dir), &mut actions) {
                        let _ = menu.append(&item);
                    }
                    items.push(Box::new(menu));
                }
                let other = Submenu::new("In Another Directory…", true);
                for item in agent_items(None, &mut actions) {
                    let _ = other.append(&item);
                }
                items.push(Box::new(other));
            }
        }
        for item in &items {
            let _ = self.new_session.append(item.as_ref());
        }
        self.new_session_items = items;
        self.new_session_actions = actions;
        self.shown_new_session = Some(shape);
    }

    /// Reflects a (re)loaded config in check marks and shortcut hints.
    pub fn sync_config(&self, config: &Config) {
        self.hold.set_checked(config.hold_to_talk);
        self.auto_attach.set_checked(config.auto_attach_images);
        self.sounds.set_checked(config.sounds);
        self.keep_audio.set_checked(config.keep_audio);
        self.wispr_flow.set_checked(config.wispr_flow);
        self.identify.set_checked(config.measure.identify_elements);
        self.pixels.set_checked(config.measure.units == Units::Px);
        self.measure_record.set_checked(config.measure.record);
        for (detail, item) in &self.details {
            item.set_checked(*detail == config.annotation_detail);
        }
        for (mode, item) in &self.deliveries {
            item.set_checked(*mode == config.agent_delivery);
        }
        let chosen = terminal_name(config);
        for (name, item) in &self.terminals {
            item.set_checked(name.eq_ignore_ascii_case(&chosen));
        }
        for (id, item) in &self.models {
            item.set_checked(*id == config.model);
        }
        for (name, item) in &self.devices {
            item.set_checked(*name == config.input_device);
        }
        let _ = self.record.set_accelerator(accelerator(&config.shortcuts.record));
        let _ = self.record_new.set_accelerator(accelerator(&config.shortcuts.record_new_note));
        let _ = self.attach.set_accelerator(accelerator(&config.shortcuts.attach_image));
        let _ = self.send.set_accelerator(accelerator(&config.shortcuts.send_to_agent));
        let _ = self.measure.set_accelerator(accelerator(&config.shortcuts.measure));
        self.cancel.set_text(cancel_label(config));
    }

    /// Rebuilds the recent-directories part of the Directory submenu.
    pub fn set_directories(&mut self, recent: &[PathBuf], current: Option<&Path>) {
        let unchanged =
            recent.len() == self.directories.len() && recent.iter().zip(&self.directories).all(|(a, (b, _))| a == b);
        if !unchanged {
            for (_, item) in self.directories.drain(..) {
                let _ = self.directory_menu.remove(&item);
            }
            if let Some(separator) = self.directories_separator.take() {
                let _ = self.directory_menu.remove(&separator);
            }
            for (i, dir) in recent.iter().enumerate() {
                let item = CheckMenuItem::new(menu_text(&tilde(dir)), true, false, None);
                let _ = self.directory_menu.insert(&item, i);
                self.directories.push((dir.clone(), item));
            }
            if !recent.is_empty() {
                let separator = PredefinedMenuItem::separator();
                let _ = self.directory_menu.insert(&separator, recent.len());
                self.directories_separator = Some(separator);
            }
        }
        for (dir, item) in &self.directories {
            item.set_checked(Some(dir.as_path()) == current);
        }
    }

    pub fn checked(&self, action: &MenuAction) -> bool {
        match action {
            MenuAction::ToggleHoldToTalk => self.hold.is_checked(),
            MenuAction::ToggleAutoAttach => self.auto_attach.is_checked(),
            MenuAction::ToggleSounds => self.sounds.is_checked(),
            MenuAction::ToggleKeepAudio => self.keep_audio.is_checked(),
            MenuAction::ToggleWisprFlow => self.wispr_flow.is_checked(),
            MenuAction::ToggleIdentifyElements => self.identify.is_checked(),
            MenuAction::ToggleMeasurePixels => self.pixels.is_checked(),
            MenuAction::ToggleMeasureRecord => self.measure_record.is_checked(),
            _ => false,
        }
    }
}

/// The app agent sessions open in, as the settings say (Terminal when they don't).
fn terminal_name(config: &Config) -> String {
    match config.agents.terminal.trim() {
        "" => "Terminal".into(),
        name => name.into(),
    }
}

/// Menu titles treat a single `&` as a mnemonic marker; double it to show it literally.
fn menu_text(text: &str) -> String {
    text.replace('&', "&&")
}

fn accelerator(shortcut: &str) -> Option<Accelerator> {
    Accelerator::from_str(shortcut).ok()
}

/// Esc is shown in the label rather than as a key equivalent, so Esc can still close the menu.
fn cancel_label(config: &Config) -> String {
    format!("Cancel Recording ({})", pretty_shortcut(&config.shortcuts.cancel))
}
