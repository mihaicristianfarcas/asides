# Architecture

How asides fits together, for anyone changing it, and why it works this way.

## Overview

```
                      asides.app (menu bar, one process)
 terminal            ┌──────────────────────────────────────────────────────────────┐
┌──────────┐ open -g │ main thread: tao event loop                                  │
│  asides  │ ──────▶ │   App: recording, status item and menu, shortcuts, sessions, │
│  (CLI)   │ asides: │        sending; worker thread: Whisper → ~/.asides/history   │
│          │ ◀────── │                                                              │
└──────────┘ state.  └──────────────────────────────────────────────────────────────┘
      │       json      │ JSON lines (stdin/stdout)      │ ps, lsof            │ dirijor, osascript
      ▼                 ▼                                ▼                     ▼
 ~/.asides/history   asides-overlay (GPUI): the     agent sessions: claude, codex, opencode…
 (asides mcp reads)  frozen screens and the pill;   in Terminal, Ghostty or Diri, each aside
                     --window: the asides window    typed in as the user's message
```

- **The app** is one long-lived, menu-bar-only process. The main thread owns the UI and recording;
  one worker thread transcribes and keeps asides in the history; each send runs on its own thread.
- **Sending**: the app finds agents running in terminals in the process table (`sessions.rs`) and
  types each aside into one, as the user would (`deliver.rs`). Nothing listens, nothing is installed
  in the agents.
- **The history** (`~/.asides/history`) keeps every aside, sent or not, pruned after
  `history_days`. Nothing is written into the user's repositories.
- **The overlay** is a second process (`asides-overlay`, GPUI), because GPUI and tao each own their
  `NSApplication`. The app captures the screens and hands it the pixels; it draws the frozen screens
  and the pill. The next one is started ahead of time (`--standby`). The session's logic
  (`overlay.rs`, `measure.rs`, `inspect.rs`) lives in `asides-core`. `--window` is the asides window.
- **The CLI and the window** are remote controls: `asides://` links in, `state.json` and the history
  out. `asides mcp` only reads the history and measures images.

## Code map

```
crates/
  asides-core/     platform-independent, unit-tested
    agents.rs         the agents: processes, new-session command lines, submit key, session markers
    sessions.rs       running sessions from ps/lsof: host app, last input, labels, folders
    deliver.rs        per-app plans for typing an aside in; new-session launch scripts
    history.rs        ~/.asides/history: records, messages, images; listing, pruning, edits
    markdown.rs       what was said and made → the message, placed by time (pins by the word); checks
    link.rs           asides:// links (CLI and window → app)
    state.rs          state.json (app → CLI and window)
    measure.rs        PixelSnap-style measuring: edges, distances, snapping, guides, colors
    inspect.rs        a session's measurements, pins, drawings, elements, and their Markdown
    overlay.rs        the session state machine (tools, holds, guides, pins, shots, restore), scripts
    takes.rs          a session's recordings, joined into one, with their moments
    helper.rs         app ↔ overlay protocol: Spec, Event, Command, Outcome
    keys.rs           the overlay's key chords
    install.rs        linking the asides command for a hand-installed app
    wispr.rs          Wispr Flow's dictation history (read-only sqlite3)
    watch.rs          files and folders watched (FSEvents): what changes, as it changes
    log.rs            ~/Library/Logs/asides/asides.log: lines, problems, panics, standard error; crash reports
    watchdog.rs       main threads that stop or spin: slow, stuck, given up
    config.rs         config.toml schema and defaults
    models.rs         Whisper model catalog (sizes, SHA-256)
    resample.rs       resampling to 16 kHz
    update.rs         updating asides: the feed, versions, the status shown, release notes
    paths.rs          bundle ID, URL scheme, paths
  asides-cli/      the asides binary (asides.app/Contents/Helpers/asides)
    main.rs           subcommands, links, waiting for the app
    mcp.rs            asides mcp: get_aside, list_asides, measure_image
    measure.rs        asides measure and measure_image
  asides-overlay/  GPUI overlay (asides.app/Contents/MacOS/asides-overlay)
    main.rs           session, standby, window and script modes
    ui.rs             Overlay, ScreenView, the pill and its list, cards, menus, keys sheet
    window.rs         the asides window: History, Sessions, General, Keys, Voice, Pins, Help; setup
    help.rs           Help's clips: frames of the overlay, the pointer between them, drawn when due
    controls.rs       its look (after pingpong's): colors, type, and the controls its pages use
    windows.rs        opening, showing, hiding and moving the windows (the clock, as things happen)
    input.rs          the card's text field, and the area History edits Markdown in
    export.rs         images that leave it, drawn with AppKit
    finish.rs         result.json
    style.rs          shared colors and shapes
    glass.rs          glass on a frozen screen: a blurred copy of the freeze
    native.rs         AppKit glue: borderless panels, levels, kiosk, focus, drawing
    io.rs             stdin/stdout protocol, spec and pixels
    replay.rs         development: replays AppKit events and captures windows
    assets.rs         SVG icons, and help/ (embedded by build.rs)
    help/             the clips' frames and clips.json, made by scripts/make-help.py
  asides-app/      the menu-bar app (asides-app, renamed asides in the bundle)
    main.rs           App: event loop, recording, overlay sessions, sending
    tray.rs, icons.rs the status item, its menu and its icons
    hotkeys.rs        Carbon hot keys
    audio.rs          cpal capture
    worker.rs         transcription, keeping asides
    watch.rs          Wispr Flow (as its database changes), sessions (when asked, and as one ends)
    terminal.rs       running plans (dirijor, osascript, clipboard), starting sessions
    clipboard.rs      pasting screenshots, restoring the pasteboard
    capture.rs        ScreenCaptureKit screenshots in sRGB; Screen Recording
    overlay.rs        driving asides-overlay; the standby process
    elements.rs       what's under a pin: window list and accessibility tree
    stt.rs            model downloads, whisper-rs, language choice
    macos.rs          microphone permission, sounds, frontmost app, the command link, opening at login
    update.rs         the updater (check, download, verify), the swap script, Flow (its states)
    sounds/           its sounds (embedded by macos.rs), made by scripts/make-sounds.py
packaging/            Info.plist, icon, entitlements, Cloudflare Workers (download/, site/)
scripts/              e2e, fixtures, signing, release, dmg, website images, icon, sounds
docs/                 the website
guide/                user documentation
Casks/asides.rb       the Homebrew cask (the repository is a tap)
```

## Threads

| Thread | Owns | Reports with |
|---|---|---|
| Main (tao) | `App`, status item, menu, shortcuts, the cpal stream | n/a |
| cpal callback | appends samples to a shared buffer | the buffer, read on stop; `UserEvent::Heard` (the pill's level) |
| Worker | the `Transcriber`; **creating asides** | `UserEvent::Work` |
| Model download | the `.part` file | `UserEvent::Model`, a `Condvar` for the worker |
| Sending (per aside) | a plan's steps, the pasteboard | `UserEvent::Delivered` |
| Sessions | `sessions::discover` when asked (asides opening, the window coming forward, an aside sent, the user leaving Terminal, Ghostty or Diri), as one starts (its agent's file, FSEvents) and as one ends (kqueue) | `UserEvent::Sessions` |
| Config watcher | `config.toml`, as it changes (FSEvents) | `UserEvent::ConfigFileChanged` |
| Wispr Flow watcher | `flow.sqlite`, when it changes | `UserEvent::Wispr` |
| Agent finder (launch) | the login shell's installed agents | `UserEvent::Agents` |
| ScreenCaptureKit | screenshots → frames | `UserEvent::Captured` |
| Overlay reader | the overlay's stdout | `UserEvent::Overlay` |
| Updater | the feed, downloads into `~/Library/Caches/asides/updates` | `UserEvent::Update` |
| Log (each process) | its standard error, into the log | the log |
| Watchdog (each GUI process) | the main thread's spell of work, from a run loop observer | the log; the overlay ends itself after 10 s stuck |

Background threads never touch AppKit (the pasteboard excepted); they send `UserEvent`s that
`App::handle` runs on the main thread. The app writes overlay commands from the main thread.

## Lifecycles

### Finding sessions

`sessions::discover` lists processes (`ps`), keeps interactive agents with a terminal (one per
terminal), and reads their environment (`ps -E`) and folder (`lsof -d cwd`). `TERM_PROGRAM` or
`DIRIJOR_SESSION_ID` gives the host app. The terminal device's access time is the last input: the
latest reachable session is the default target (`sessions::latest`).

Looking takes tens of milliseconds, so it's done before asides opens: as a session starts (Claude
Code and Codex write a file, `sessions::start_folders`), when the user leaves a terminal app it can
type into, and as one ends. Opening only reads the terminals' access times again (microseconds), so
the pill names the session typed into last from its first frame.

### Opening, freezing, sending

1. **Open** (`App::open_asides`): refresh sessions; capture each display (`capture_screens`,
   ScreenCaptureKit, converted to sRGB in strips); write the pixels and `session.json`
   (`helper::Spec`) to a temporary directory; hand it to the standby overlay (`Command::Open`).
   With `mic_on_open`, start recording.
2. **The overlay** takes its prepared windows (`windows::Spare`), activates (kiosk options on while
   frozen, `native::kiosk`) and says `ready`. Views turn input into `overlay::Session` calls, each
   stamped with its time (`Session::clock`). A new pin asks the app `identify`
   (`elements::identify`).
3. **Live and back** (`⌃⌥P`): the overlay hands the keyboard back and shows only the pill, a
   non-activating panel (the same one frozen, above the screens). Freezing again captures again, leaving out the overlay's windows
   (`Session::refrozen`).
4. **Recording**: cpal while open; each stretch is a take (`App::takes`). One started from a pin's
   card is transcribed at once and goes in that pin (`Annotation::said`).
5. **Send** (`finish`): the overlay renders each freeze's screenshot, around what's on it
   (`export.rs`; none with nothing on it, unless `⌥⏎` asks for the whole screen), and Markdown
   (`inspect`) into `result.json`. The app joins the takes (`takes::join`), places every item and
   Wispr Flow dictation at its moment, and queues one `Job::Aside`. `cancelled` sends nothing.
6. **Worker** (`keep`): resample, transcribe (no prompt; each word timed), compose
   (`markdown::compose_message`: a pin's number after the word said as it was placed, its details
   after that paragraph), keep (`history::create`, with the pins' colors), and return
   `Outcome::Kept` with the target.

### Delivering

`App::deliver` sends to:
- **A session**: `deliver::plan` for its app and agent (Diri's `dirijor`, Ghostty's `input text`,
  Terminal's `do script` with bracketed paste), screenshots by Ctrl+V or by path as each agent
  takes them, Return to send (Tab for Codex). `terminal::run_steps` runs it and restores the
  pasteboard.
- **A new session**: `terminal::start_session` writes a launch script (login shell's `PATH`, `cd`,
  the agent with the aside) and opens it in Terminal, Ghostty or Diri.
- **History only**: nothing more.

`UserEvent::Delivered` records where it went (`Aside::record_delivery`) and flashes `[n]` or the
error.

### Setting up

Until `onboarded`, the app opens the window on setup at launch (`--window`, page `setup`). It shows
how asides works first (the basics' clip, as Help does). Its steps change `config.toml` as the
settings do, and ask the app through links: `asides://permission`
(macOS's prompt; Automation with `AEDeterminePermissionToAutomateTarget`, the app started first),
`asides://setup` (the step reached, to go on after a restart; `done` ends it), `asides://login`
(opening at login, as General's switch does). While the window is open, the app looks at the
permissions and opening at login again, off the main thread, into `state.permissions` and
`state.open_at_login`, when an app comes to the front (back from System Settings, or a prompt
answered) or Accessibility changes for an app: macOS says nothing more.

### Updating

The updater (`asides-app/src/update.rs`) reads `appcast.json` from the latest GitHub release
(written by `release.sh`), 20 s after launch and every 6 hours with `auto_update`, or when asked
(`asides://update`). A newer release is downloaded, checked (size and SHA-256, `codesign`, the same
team and bundle ID, Gatekeeper, the version) and unpacked. `Flow` turns replies and clicks into
`state.update`, which the window's sidebar and the menu show. When asides quits (or restarts for
it), `update::install` starts a script in its own process group: it copies the update beside the
app, waits for asides to exit, swaps them by renaming, and reopens asides. The new copy shows the
notes it was handed (`Status::handed_over`) until they're seen (`whats_new_seen`).

## Invariants

Break these only on purpose, and update this list.

1. **`asides-core` is platform-independent**: no AppKit, objc2 or audio crates.
2. **Nothing polls.** Everything waits on what happens (input, notifications, FSEvents through
   `watch.rs`, kqueue, a process ending, a channel) or sleeps until the exact time something is
   due. A timer only where nothing says, and the code says why (the update check, the watchdog's
   deadline: a stuck thread can't say so).
3. **AppKit, tray and menu objects are touched on the main thread only** (the pasteboard excepted).
4. **Only the worker creates asides**, in order. Records change after only to note delivery (a new
   file renamed over the old), an edit in History (only a message `markdown::check_message`
   passes), deletion, or pruning.
5. **An aside is typed in as the user would**: nothing installed in or changed for the agents.
6. **Never into a session the user didn't choose.** Dev hooks send to the history unless `--to`.
7. **Transcripts are verbatim**: no prompt, vocabulary or post-processing (a prompt silently dropped
   words in testing).
8. **The CLI ↔ app protocol is `link.rs` plus `state.rs`.** Change both sides together.
9. **Launch Services launches the app**, never a terminal (or the terminal gets the microphone
   permission, and the app dies with it).
10. **Binary names differ by more than case** (case-insensitive APFS).
11. **Stable signing identity, hardened runtime**: permissions follow the identity; every capability
    needs an entitlement.
12. **No new permissions without discussion.** Today: Microphone, Screen Recording, Accessibility
    (optional), Automation for Terminal and Ghostty. Not Input Monitoring.
13. **`asides mcp` writes only JSON-RPC to stdout**, and its tools are read-only (and marked so).
14. **The overlay needs no permission**: the app captures and identifies. Its stdout carries only
    protocol lines.
15. **Measuring logic stays in `asides-core`**, shared by the overlay, `asides measure` and
    `measure_image`.
16. **On-screen UI is GPUI**, in `asides-overlay`. Exported images use AppKit (GPUI can't render
    offscreen), following `style.rs`.
17. **Each aside goes to one place**: a session, a new session, or the history.
18. **Only a notarized asides from the same team, of the version promised, replaces asides**, and
    only once it has quit. Copies that aren't notarized never update themselves.
19. **What goes wrong is in the log**: errors as problems (`log::problem`, `App::fail`), panics,
    standard error, how an overlay ended, slow and stuck main threads, the overlay's clock spinning.
    A frozen overlay stuck 10 s ends itself (`abort`: macOS keeps a report of where it was).

## Persistent state

| Path | Written by | Notes |
|---|---|---|
| `~/.asides/history/<id>/` | worker; main thread for deliveries | `aside.json`, `message.md`, `images/`, `audio/`; `ASIDES_HOME` moves it |
| `~/Library/Application Support/asides/config.toml` | app, window, user | reloaded on change |
| `…/asides/state.json` | app | status, sessions, folders, the pill's place, the last aside, the update, What's New seen, permissions, opening at login, setup |
| `…/asides/models/` | download thread | `.part` until the SHA-256 checks |
| `/usr/local/bin/asides` | app | a link, for a copy installed by hand |
| `$TMPDIR/asides-sessions/*.sh` | app | new-session launch scripts (delete themselves) |
| `~/Library/Caches/asides/updates/<version>/` | updater | `staged/asides.app`, `install.sh`, `install.log`; removed once that version runs |
| `$TMPDIR/asides-overlay-<pid>-<n>/` | app, overlay | a session's pixels, spec, result; removed on exit |
| `~/Library/Logs/asides/asides.log` | app, overlay, window | a line each, `!` for a problem; `asides.log.1` past 4 MB |
| `~/Library/Application Support/Wispr Flow/flow.sqlite` | Wispr Flow | read-only, with `wispr_flow` on |

## Design decisions

- **Typed into the terminal, not pushed through MCP.** MCP can't post into a chat; Claude Code's
  channels need a launch flag; hooks add hidden context. Typing works for every agent, shows in the
  chat, and queues while the agent is busy. How each agent takes text, Return and images in each app
  was measured, and is documented in `deliver.rs`.
- **The session typed into last**: the terminal device's access time (what `w` uses), for any
  agent in any app, with nothing installed.
- **A history, not notes in the repo**: asides aren't tied to a repo, and nothing needs ignoring.
- **The pill, not a toolbar**: the default path must beat a screenshot. Annotate is the default,
  pins open no card (unless the microphone is off: then the user types), everything else is a key
  away.
- **Press, not hold**: holding `⌃⌥` while clicking makes right-clicks and changes measuring.
- **English model by default, downloaded on request**: smaller, never mistakes the language, and
  asides works without one. On 54 synthetic clips (`compares_models`), `small.en` had a 0.8% word
  error rate at 0.34 s a clip, against 0.4% at 1.30 s for `large-v3-turbo`. With a strong accent,
  `large-v3-turbo` with `languages = ["en"]` was the most accurate.
- **Measure a frozen frame**: what's measured is exactly what's in the screenshot, nothing moves.
  Captured with ScreenCaptureKit in sRGB, so colors match the CSS that drew them.
- **Edges are sharp, visible changes along a contour**, so measuring works over glass, gradients,
  soft shadows and grainy wallpapers. The rules are in `measure.rs`'s module docs and AGENTS.md's
  pitfalls. A frozen screen's edges are found once (`measure::Edges`), then every snap reads them.
- **A standby overlay**: GPUI, its windows and Metal shaders are ready before the shortcut. Opening
  went from 262 ms to 109 ms.
- **Talk in order**: every pin, measurement and shot carries its time, so it lands where the user
  was speaking.
- **Elements through accessibility**: browsers expose DOM ids, classes and the URL, and it covers
  native apps too. Without the permission, pins name the app and window.
- **Anything on screen, not just the web**: no DOM toolbar, no design-app hand-offs.
- **Session markers dropped**: agents mark their processes (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`…)
  and `open` passes them on, so the app strips them (`agents::session_markers`) before starting
  terminals.
- **whisper.cpp via whisper-rs, with Metal**: every language, fast, built by cargo. One pass after
  stop; detection limited to `languages`. Apple's SpeechAnalyzer has no Romanian.
- **Links plus a state file, not a socket**: Launch Services gives a single instance and launches on
  demand.
- **The status item is the app's mark**: brackets around a state, drawn as template images on a
  22 pt canvas (tray-icon's limit).
- **Carbon hot keys**: no Accessibility needed.
- **`/usr/local/bin` for the command**: on every shell's `PATH`; Homebrew links into its own `bin`.
- **Updates trust Apple, as Diri's do**: Developer ID and notarization, checked on the download,
  instead of Sparkle and a signing key of our own to keep. The feed is a release asset, so
  publishing a release publishes the update.
- **Opening at login is macOS's**, not a setting: `SMAppService`'s main app, which System Settings
  → General → Login Items changes too. The app asks macOS and publishes what it says; a copy in
  `config.toml` would drift from it. Turned off there, only there can it be turned on again.
- **Wispr Flow through its database**, read with `/usr/bin/sqlite3`, columns discovered at run
  time.
