# Architecture

How asides fits together, for anyone about to change it. User-facing behavior is in the
[README](README.md); how to set up, test and submit changes is in [CONTRIBUTING.md](CONTRIBUTING.md);
why it works this way (and what was measured) in [design/redesign.md](design/redesign.md).

## Bird's-eye view

```
                      Asides.app (menu bar, one process)
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
                     --window: the Asides window    typed in as the user's message
```

- **The app is one long-lived process.** It is menu-bar only (`LSUIElement`, accessory activation
  policy). The main thread owns the UI and recording; a single worker thread transcribes and keeps
  each aside in the history; sending runs on a thread per aside.
- **An aside goes to an agent session, as the user's message.** The app finds the coding agents
  running in a terminal in the process table (`sessions.rs`), and types each aside into one of
  them the way the user would (`deliver.rs`): the agent sees a normal message, and the user sees it
  in the chat. Nothing has to listen, and nothing is installed in the agents. The default is the
  session the user typed into last; the overlay's pill picks another, or a new session.
- **Every aside is kept** in `~/.asides/history` (`history.rs`), whether it was sent or not, and
  pruned after `history_days`. Nothing is written into the user's repositories.
- **Asides is one mode, frozen or live, in its own process.** The shortcut captures every display
  (ScreenCaptureKit) and hands the screenshots to `asides-overlay`, which shows them in borderless
  GPUI windows above everything, under the pill; going live leaves only the pill, over the live
  screen, until the screen freezes again. GPUI runs its own event loop, so it can't share the app's
  (tao's) process. The app starts the next overlay ahead of time, so the shortcut doesn't wait for a
  process, GPUI and its windows to start. The session's logic (`asides-core/overlay.rs`,
  `measure.rs`, `inspect.rs`) is platform-independent; the overlay feeds it events and draws it.
  Permissions stay in the app: it hands the overlay the pixels, and names what's under a pin when
  asked. The same binary, with `--window`, is the Asides window (History, Sessions, Settings).
- **The CLI and the window are remote controls.** They hand the app `asides://` links through Launch
  Services (which launches the app if needed), and read what it publishes (`state.json`), the
  history and the settings. `asides mcp` only reads the history and measures images.

## Code map

```
crates/
  asides-core/     platform-independent, no macOS or UI dependencies, unit-tested
    agents.rs         the coding agents: names, how their processes look, how a new session starts
                      (its command line with the first message), the key that sends (Tab for Codex),
                      the variables marking an agent's session
    sessions.rs       the agent sessions running: from the process table (`ps`, `lsof`), their apps
                      (Terminal, Ghostty, Diri, or one asides can't type into), last input, labels,
                      the folders a new session can start in
    deliver.rs        how an aside is typed into a session in each app (a plan of steps: dirijor,
                      AppleScript, the clipboard), and the launch script for a new session
    history.rs        ~/.asides/history: an aside's record, message and images; listing, pruning
    markdown.rs       what was said and what was made → the message: segments → paragraphs, images
                      and blocks placed by time; splitting a message into text and images
    link.rs           asides:// links (CLI and window → app)
    state.rs          state.json (app → CLI and window): status, sessions, recent folders, the pill
    measure.rs        PixelSnap-style measuring on a frame: distances to edges, snapping areas and guides, objects, colors
    inspect.rs        an Asides session's result (measurements, guides, annotations, drawings, elements, what was
                      said about them) and its Markdown
    overlay.rs        the session state machine (Measure, Annotate, Draw, Shot; drags, holds, guides,
                      markers, annotated sections, screenshots, restore; when each was made), scripts
    takes.rs          what's said during a session, joined into one recording; its moments on it
    helper.rs         the app ↔ overlay protocol: session.json (Spec, with the targets for the pill),
                      Events, Commands, result.json (Outcome, with where it goes); STANDBY, WINDOW
    keys.rs           the overlay's key chords (`[measure.keys]`): parsing, matching, labels
    install.rs        where a hand-installed app links the `asides` command, and whether one is set up
    wispr.rs          Wispr Flow's dictation history (flow.sqlite through the system's sqlite3)
    config.rs         config.toml schema and defaults, shortcut pretty-printing
    models.rs         Whisper model catalog (English-only and multilingual; file names, sizes, SHA-256)
    resample.rs       windowed-sinc resampler to 16 kHz
    paths.rs          bundle ID, URL scheme, Application Support paths, tilde()
  asides-cli/      shipped inside the bundle as Asides.app/Contents/Helpers/asides
    main.rs           clap subcommands: the window, open, sessions, history, show, send, download,
                      status, quit; `open -g` links; waiting for the app to acknowledge
    mcp.rs            `asides mcp`: get_aside, list_asides, measure_image (read-only)
    measure.rs        `asides measure` and measure_image: PNG decoding and the measurement as text
  asides-overlay/  binary `asides-overlay`, shipped as Asides.app/Contents/MacOS/asides-overlay (GPUI)
    main.rs           reads the session, opens the pill or a window per screen (or plays a script);
                      `--standby`: starts GPUI and the screens' windows, then waits; `--window`: the window
    ui.rs             Overlay (the session, live and frozen, and around it), ScreenView (drawing, mouse, keys),
                      the pill (`toolbar`) and its session list, the annotation card, the context menu,
                      the keys sheet, toasts, talking, finishing
    window.rs         the Asides window: History, Sessions, Settings
    windows.rs        the overlay's windows: opened (or taken from those opened while standing by), shown and
                      hidden by mode, the live pill fitted and moved
    input.rs          the card's text field (GPUI's input example: IME, selection, clipboard; wraps and grows)
    export.rs         the images that leave it (aside, Shot, clipboard), drawn with AppKit into a bitmap
    finish.rs         result.json: the screen images, each measurement's and annotation's Markdown, the screenshots, by time
    style.rs          colors and shapes shared by the windows and the exported images
    native.rs         what GPUI's windows need to be an overlay (borderless, screen-saver level, all spaces),
                      and the live pill's panel (never key)
    io.rs             Events on stdout, Commands from stdin; the session's spec and pixels
    replay.rs         development: plays mouse and key steps as AppKit events and captures the windows
    assets.rs         the pill's and menus' icons (SVG)
  asides-app/      binary `asides-app`, renamed Asides inside the bundle
    main.rs           App: event loop, UserEvent dispatch, recording, the overlay's sessions, sending,
                      refresh(), publish()
    tray.rs           status item and its short menu
    icons.rs          the status item's icons: the asides mark per status, drawn to RGBA for tray-icon
    hotkeys.rs        global shortcuts (Carbon hot keys): Asides (open, send) and freeze
    audio.rs          cpal microphone capture → mono f32 at the device rate
    worker.rs         Job queue: transcription, keeping asides in the history; what was said about an annotation
    watch.rs          polling threads: Wispr Flow's history, the agent sessions
    terminal.rs       running a plan's steps (dirijor, osascript, the clipboard), starting new sessions
                      (Terminal, Ghostty, Diri), the agents installed (the login shell)
    clipboard.rs      the pasteboard while screenshots are pasted: put a PNG, keep and restore what was there
    capture.rs        ScreenCaptureKit screenshots (sRGB) → frames; the Screen Recording permission; displays
    overlay.rs        hands asides-overlay a session directory (the one standing by, or a new one), relays its
                      messages, reads its result; starts the next one ahead of time (`Standby`)
    elements.rs       what's under a pin: the window list, and the accessibility tree (AXUIElement)
    stt.rs            ModelManager (download when asked, verify, wait), Transcriber (whisper-rs), language choice
    macos.rs          microphone permission, sounds, apps in front, the command link
packaging/Info.plist  bundle ID, LSUIElement, usage strings (microphone, Apple events), URL scheme, icon
packaging/Asides.icns the app icon, drawn by scripts/make-icon.swift
packaging/Asides.entitlements   microphone and Apple events entitlements (required by the hardened runtime)
packaging/download/   the Cloudflare Worker that serves the website's disk image (static files only)
packaging/site/       the Cloudflare Worker that serves the website, docs/ (static files only)
scripts/              e2e.sh (+ e2e-agent.c, a stand-in agent), make-fixtures.sh, signing-identity.sh,
                      make-signing-cert.sh, release.sh, make-dmg.sh, publish-dmg.sh, make-og.py,
                      make-icon.swift
design/               redesign.md: the 2026-10 redesign, its decisions and measurements
docs/                 the website (asides.mihaicristian.dev): index.html, wallpaper.webp, favicon.svg, og.png
Casks/asides.rb       the Homebrew cask; the repository doubles as a tap
```

## Threads and how they talk

| Thread | Owns | Talks back through |
|---|---|---|
| Main (tao event loop) | `App`, the status item and menu, shortcut registration, the cpal `Stream` | n/a |
| cpal callback (CoreAudio) | pushes samples into `Arc<Mutex<Vec<f32>>>` | the shared buffer, read on stop |
| Worker (`worker::spawn`) | the loaded `Transcriber`, **the history's new asides** | `UserEvent::Work(Outcome)` |
| Model download (one per model, when asked) | the `.part` file | `UserEvent::Model(id, ModelState)`, plus a `Condvar` for the worker |
| Sending (one per aside) | running a plan's steps: `dirijor`, `osascript`, the pasteboard | `UserEvent::Delivered` |
| Sessions (`watch::sessions`) | `sessions::discover` every 10 s, and at once when asked (Asides opening) | `UserEvent::Sessions` |
| Config watcher | polls `config.toml`'s modification time every 2 s | `UserEvent::ConfigFileChanged` |
| Wispr Flow watcher (while on) | a `wispr::History`; queries only after `flow.sqlite` changes | `UserEvent::Wispr` |
| Agent finder (at launch) | asks the login shell which agents are installed | `UserEvent::Agents` |
| ScreenCaptureKit queues | turn each display's screenshot into a frame | `UserEvent::Captured` |
| Overlay reader (while it's open, or standing by) | reads `asides-overlay`'s standard output | `UserEvent::Overlay` (with its process id) |
| global-hotkey, muda, tray-icon | | `UserEvent::HotKey`, `UserEvent::Menu` |

Background threads never touch AppKit objects (the pasteboard excepted, which is safe from any
thread). They send `UserEvent`s through the `EventLoopProxy`, and `App::handle` runs them on the
main thread. The overlay is another process with its own main thread (GPUI's); the app writes its
commands to the overlay's standard input from the main thread.

## Lifecycles

### Finding the sessions

`sessions::discover` lists the processes (`ps -A -o pid,ppid,tty,comm`), keeps the agents'
(`AgentKind::of_process`: `claude`, `codex`, `opencode` by their executable; Gemini CLI, Copilot
CLI and Qwen Code by their Node script) that have a terminal and run interactively (not `claude -p`,
`codex exec`, `opencode serve`…), one per terminal (not an agent another agent started there), then
reads their arguments and environments (`ps -E`) and their working directories (`lsof -d cwd`). The
environment says which app hosts each one: `TERM_PROGRAM` (`Apple_Terminal`, `ghostty`; others,
like Warp or iTerm, are listed as apps asides can't type into), else Diri's `DIRIJOR_SESSION_ID`.
The access time of its terminal device (`/dev/ttys003`) is when it last got input: the latest
reachable one is the default target (`sessions::latest`). Claude Code's own name for a session comes
from `~/.claude/sessions/<pid>.json`. It takes a few tens of milliseconds; the app looks every 10 s,
when Asides opens, and again before sending to a session it doesn't know yet.

### A recording

1. **Start** (`start_recording`, while Asides is open: as it opens with `mic_on_open`, the pill's
   microphone, `Space` held while muted, an annotation's microphone):
   - checks the microphone permission, and starts cpal
   - looks up where the user is (`elements::place`, on a thread of its own: an app can be slow to
     answer accessibility): the frontmost window of the app a frozen screen stands for, or of the
     one in front, its title, and the page's URL with `identify_elements`. It goes in the aside as
     `**In:** …` where the take starts (`markdown::without_repeated_places` drops repeats)
   - asks the worker to load the model (`Job::Warm`) while the user talks
   - plays the start sound
2. **Stop**: cpal stops. Recordings under 0.3 s are dropped as accidental. In Asides, the rest
   are takes, kept for the end of the session; one about an annotation is transcribed at once for
   its card (`Job::Transcribe`).
3. **Worker** (`keep`): resamples to 16 kHz; transcribes (English models in English; otherwise
   language detection limited to `config.languages`, then one beam-search pass), waiting for a
   download under way (never starting one); composes the message (`markdown::compose_message`);
   keeps the aside in the history (`history::create`), with its screenshots and, with
   `keep_audio`, its audio; hands it back (`Outcome::Kept`) with where it goes.

### Asides: live and frozen

1. **Open** (`App::open_asides`, on `⌃⌥Space`, `⌃⌥P`, the menu or `asides open`): it asks for a fresh
   look at the sessions, then, frozen (`⌃⌥Space`), `capture_screens` checks the Screen Recording
   permission (asking for it the first time; once allowed, not again until a capture fails: asking
   takes 11 ms), lists the displays and the frontmost app, and asks ScreenCaptureKit for one
   screenshot per display: with nothing to leave out, macOS 26 takes the part of the screen each
   display shows (`captureImageInRect`) without listing what's shareable first; freezing again from
   live leaves out the overlay's windows, through a content filter. Each is converted to sRGB in
   strips, a thread each. Meanwhile it snapshots the window list and looks up where the user is (as
   a recording does). On `UserEvent::Captured`, `show_overlay`: `overlay::start` writes each
   display's pixels and `session.json` (`helper::Spec`: the screens, the settings and keys, the last
   session's guides, the pill's place, the voice model, the targets: the sessions, the folders a new
   session can start in, the agents installed, and the default) into a temporary session directory,
   and hands it to the overlay standing by (`Command::Open`), or starts `asides-overlay` with it.
   Opened live (`⌃⌥P`), the screens are listed without pixels, and only the pill shows. With
   `mic_on_open` (and a model on disk, or downloading), recording starts.
   The app starts an overlay ahead of time (`overlay::Standby`, `asides-overlay --standby`) when it
   launches and when a session ends: it starts GPUI, opens a hidden window for each screen (Metal
   compiles GPUI's shaders for each window, 25 ms, and GPUI draws it as it opens) and waits on its
   standard input, without a timer. Given the session, it reads the pixels (a few threads at once),
   and the frozen screens take those windows (`windows::Spare`); screens that changed since get new
   ones. Opening takes about 109 ms. It stays an accessory app with no window shown, using no CPU
   while it waits; if it quits, the next session starts its own.
   While it's open, the Asides shortcut sends (`Command::Finish`).
2. **The pill**: at the top of the screen under the pointer, in the middle, below the menu bar (or
   where the user dragged it: `pill_at`): the microphone (listening and its time, or muted; without a
   model, a download), where the aside goes (`Targets::label`; a click opens the list: the sessions,
   those asides can't type into dimmed, New session in… with an agent chip and the recent folders,
   History only; `Tab` on a frozen screen picks the next), the tool when it isn't Annotate, send and
   close. Frozen, it's drawn in the screen's window, centered by layout; live, it's a small GPUI
   pop-up window (a non-activating panel that never becomes key, `native::make_toolbar`), sized to
   the pill and its list (`follow_toolbar`; it grows down from the top). The user's apps keep the
   mouse and keyboard. The pill moves when it's dragged anywhere but its buttons.
3. **Freeze again** (`⌃⌥P`, `App::capture_screens`, as when it opens, leaving out the overlay's own
   windows): the pixels go into the session directory and to the overlay (`frozen`), which starts
   a new freeze of the session (`Session::refrozen`: numbering and clock go on).
4. **The frozen screens** (`windows::open_screens`, the first time): one borderless window per
   display (GPUI, made a non-activating panel at screen-saver level on every space by
   `native.rs`) showing the screenshot, with Annotate (the tool it opens with) outlining what a click
   pins; the overlay activates itself, a regular app while frozen so the kiosk options hold
   (`native::kiosk`), and says `ready`. `⌃⌥P` goes back to live (`live`): the overlay gives the
   keyboard back to the app that had it, and once that app is in front again
   (`windows::handed_back`), the screens hide and the pill shows; what was done on the freeze is kept
   for the aside (`finish::items`, once the live pill shows). Closing frozen gives it back too;
   closing live leaves it with whatever app the user is in. The windows open hidden and change from
   the clock (`windows::sync`), outside GPUI's updates and not from inside their events: each is
   drawn before it shows (`native::draw_now`), so it never shows its last freeze, and the screens
   and the pill take each other's place at once. The view turns mouse and key events into
   `overlay::Session` calls (in the screen's points) and redraws, stamping what's added with the
   seconds into the session (`Session::clock`). A click pins the element under it (no card opens; a
   click on a pin's number does); a new pin (and the element under a resting cursor) is looked up
   with `identify`: the app answers from `elements::identify` (the app and window from the window
   list, and with `identify_elements` and the Accessibility permission, the element from the app's
   accessibility tree, hitting that app so the overlay isn't hit). The pill's microphone sends `mic`;
   `Space` (held while muted) and an annotation's microphone send `talk` down and up; the app tells
   the overlay when it records (`recording`). A recording that stops while the overlay is open isn't
   kept on its own: it's a take (`App::takes`), kept for the end of the session. One started from an
   annotation's card is about that annotation: the app sends the transcript to the overlay
   (`transcribing`, then `said`), which shows it in the card and keeps it in the annotation
   (`Annotation::said`). Errors the app raises while the overlay covers the menu bar are also sent to
   it (`notice`). Nothing the overlay draws goes above a screen's menu bar, where the notch is
   (`SpecScreen::menu_bar`).
5. **Send** (`⏎` on a frozen screen with no card open, ✓, or the shortcut, which the app forwards as
   `finish`): for each freeze, and each screen with something on it, the overlay renders the
   screenshot cropped around what's there (`export.rs`, sRGB, the screen's resolution) and
   `inspect` writes the Markdown: what goes with the screenshot (where, the screen and units, the
   guides), then each measurement and each annotation (with what was said about it, `**Said:**`).
   `result.json` lists them with the seconds into the session each was made, plus the Shot tool's
   screenshots, and where the aside goes (`Outcome::target`). On `finished`, the app first ends a
   recording about an annotation and waits for what was said about the annotations to be
   transcribed (`App::finishing`, `try_finish`). Then `App::add_session` joins the takes into one
   recording (`takes::join`: in order, with a pause between them) and puts each item, and each Wispr
   Flow dictation made while frozen, at its moment on it, as text attachments: one `Job::Aside`.
   Whisper runs its segments' times through the pauses between takes, so the worker keeps each
   segment within its take (`takes::fit_segments`). With nothing said, the items go in the order
   they were made. `cancelled` (`Esc`, ×) sends nothing, the session's takes included. If the
   overlay dies, or Asides quits, what was said is kept, in the history only. Either way the app
   keeps the guides and held measurements for the next session's R, removes the session directory
   when the process exits, and gives the focus back to the app that had it.
6. **Dev hooks:** `asides simulate-measure IMAGE SCRIPT [--to SESSION]` starts the overlay frozen with
   an `overlay::Script` in the spec: it plays it without windows (`live` and `freeze` steps freeze it
   again) and finishes; the aside goes to `SESSION` or the history only, never the user's own
   sessions by default. A `replay` in the spec plays mouse and key steps in real windows instead, as
   AppKit events, and captures them (`screencapture -l`), to check what the user sees.

### Sending an aside

On `Outcome::Kept`, `App::deliver`:
- **A session** (`Target::Session`, its process id): looked up among the sessions (looked for again
  if it isn't there), then `deliver::plan` for its app and agent: Diri's command (`dirijor session
  send --no-submit`, `session key`), Ghostty's AppleScript (`input text`, `send key`, the terminal
  found by its working directory) or Terminal's (`do script` in the tab whose `tty` is the agent's,
  its text wrapped in bracketed-paste markers). Screenshots go in as each agent takes them in that
  app (`deliver.rs` says how, as measured): with the pasteboard and Ctrl+V, or as their paths. The
  key that sends it is Return, or Tab for Codex (`AgentKind::submit_key`). A thread runs the steps
  (`terminal::run_steps`), keeping what the pasteboard held and putting it back.
- **A new session** (`Target::New`): `terminal::start_session` writes a `/bin/sh` script (the login
  shell's PATH, `cd` into the folder, the agent with the aside as its first message, its
  screenshots as paths, `ASIDES_LAUNCH` to know it later), and types its quoted path into a new
  window of Terminal or Ghostty (the user's shell, whatever it is), or runs it as a Diri session.
- **History only**: nothing more.

`UserEvent::Delivered` notes where it went, or why it didn't, in the aside's record
(`Aside::record_delivery`), flashes `[n]` (the aside's number today) or the error, and asks for a
fresh look at the sessions (the one it went to got input).

### A Wispr Flow dictation

With `wispr_flow` on, the watcher reports each dictation Wispr Flow finished since it started
(`formatted` or `raw_transcript` rows of its `History` table, not archived). One made while Asides
is open and frozen (after it opened: the watcher reports it a moment later) joins the session as
text, at its moment, as Wispr Flow typed it; any other is left alone.

## Invariants

Break these only on purpose, and update this document when you do.

1. **`asides-core` stays platform-independent.** No AppKit, objc2 or audio crates (it runs `ps` and
   `lsof`, which any Unix has). Logic that can live there should, because that's where it can be
   unit-tested.
2. **AppKit, tray and menu objects are only touched on the main thread.** Background work reports
   back through `UserEvent`. (The pasteboard is the exception, while a sending thread pastes.)
3. **Only the worker creates asides** in the history, one job at a time, in the order submitted.
   Their records change after only to note where they went (from the main thread, a whole new file
   renamed over the old one), or when the user deletes them (the window) or the history prunes them.
4. **An aside is typed in as the user would.** Nothing is installed in the agents, nothing in their
   settings or files changes, and nothing has to listen: every agent that runs in a terminal asides
   can reach works as it is.
5. **Never into a session the user didn't choose.** The default is the session typed into last;
   development hooks send to the history only unless told (`--to`).
6. **Transcripts are verbatim.** No prompt, no vocabulary, no post-processing; in testing, a
   prompt silently dropped words. Changes here need a design discussion first. Wispr Flow's text is
   as it typed it.
7. **The CLI and window ↔ app protocol is `link.rs` plus `state.rs`.** Change both sides together.
   New `state.json` and `config.toml` fields need serde defaults, so files written by older versions
   still load (and fields they no longer have are ignored).
8. **The app is launched by Launch Services, never as a child of a terminal.** Otherwise macOS
   attributes the microphone permission to the terminal, and the app dies with the terminal.
9. **Binary names must differ by more than letter case.** On case-insensitive APFS,
   `asides` and `Asides` in `target/release/` are the same file.
10. **Signed with a stable identity and the hardened runtime.** macOS ties the microphone (and
    Automation) permissions to the signing identity, so ad hoc signatures re-prompt after every
    build. Under the hardened runtime every capability needs an entitlement
    (`packaging/Asides.entitlements`).
11. **No new permissions without discussion.** The microphone; Screen Recording (asked the first
    time the screen freezes); Accessibility (with `identify_elements`, on by default; everything
    works without it); Automation for Terminal and Ghostty (asked the first time asides types into or
    opens a session there). Input Monitoring is still not needed. Anything else changes what users
    must trust, so discuss it first.
12. **`asides mcp` writes nothing to stdout but JSON-RPC,** and its tools stay read-only (and
    marked so): agents like Codex ask before every call to a tool that isn't.
13. **The overlay needs no permission of its own.** Screenshots (Screen Recording) and naming
    elements (Accessibility) stay in the app, which hands the overlay pixels and answers `identify`.
    Its standard output carries only protocol lines (`helper::Event`); diagnostics go to standard
    error.
14. **Measuring logic stays in `asides-core`.** The overlay draws and routes events, but distances,
    snapping, guides, marks and their Markdown are computed in `measure.rs`, `overlay.rs` and
    `inspect.rs`, where they're unit-tested, and shared with `asides measure` and `measure_image`.
15. **The overlay's UI is GPUI.** New on-screen UI (and the Asides window) is built with GPUI in
    `asides-overlay`. The exported images are drawn with AppKit only because GPUI can't render
    offscreen; they follow `style.rs`.
16. **Each aside goes to one place:** a session, a new session, or the history only.

## Persistent state

| Path | Written by | Notes |
|---|---|---|
| `~/.asides/history/<id>/` | worker; the main thread notes deliveries | `aside.json`, `message.md`, `images/`, `audio/`; `ASIDES_HOME` moves `~/.asides` (tests) |
| `~/Library/Application Support/Asides/config.toml` | app, the Asides window, user | reloaded on change; unknown fields ignored, missing ones defaulted |
| `~/Library/Application Support/Asides/state.json` | app | status, sessions, recent folders, the pill's place, the last aside, whether the user declined the command link |
| `/usr/local/bin/asides` | app | for a copy installed by hand: a link to the CLI in the bundle, made on first launch |
| `~/Library/Application Support/Asides/models/` | download thread | `.part` while downloading, renamed after the SHA-256 check |
| `$TMPDIR/asides-sessions/*.sh` | app | a new session's launch script; deletes itself when it runs |
| `$TMPDIR/asides-overlay-<pid>-<n>/` | app, `asides-overlay` | an Asides session: the screens' pixels, `session.json`, `result.json`, images; removed when the overlay exits |
| `~/Library/Application Support/Wispr Flow/flow.sqlite` | Wispr Flow | read-only, only with `wispr_flow` on. Dev-hooks builds read `Asides/test-wispr-flow.sqlite` instead when it exists (end-to-end test) |

## Design decisions

- **Typed into the session, not pushed through MCP.** MCP has no way for a server to post a message
  into an agent's chat; Claude Code's channels can, but need a launch flag (and org policy) and can't
  reach a session already open; a UserPromptSubmit hook adds hidden context, not a message. Typing
  into the terminal the session runs in is what the user would do: it works with every agent, the
  message shows in the chat, and a busy agent queues it. The apps that let another program type
  into a terminal are the ones asides supports (Diri's command, Ghostty's and Terminal's
  AppleScript); others are listed, dimmed. Codex's app-server daemon and OpenCode's service could
  take messages without a terminal, but only for sessions living there.
- **Measured per agent and app.** How text, Return and screenshots reach each agent differs, and was
  measured with a harness reading what each agent recorded (its transcript): Terminal sends `do
  script`'s text and Return in one burst, which agents take for a paste (Return a new line), so the
  text is wrapped in bracketed-paste markers; Ghostty 1.3 drops Ctrl keys sent by AppleScript when
  the agent uses the kitty keyboard protocol, so screenshots go in as their paths there (all three
  agents attach a pasted path in Ghostty); in Diri a pasted path stays text in Claude Code and
  OpenCode, so screenshots go in with Ctrl+V; Claude Code attaches a path a moment after it's
  pasted (so in Terminal it goes alone, first), Codex and OpenCode as it's pasted (so with the text,
  or it's sent alone). Return while Codex works joins its turn, and it may answer only the newer
  message: Tab queues it. See design/redesign.md for the table.
- **The session typed into last.** The terminal device's access time changes when the agent reads
  input, which is what `w` uses for idle time: it's the session the user last typed into, in any app
  and for any agent, with nothing installed. A pick on the pill lasts for that aside.
- **A history instead of notes in the repo.** Asides aren't bound to a repo any more, so they don't
  belong to one: `~/.asides/history` keeps them (and their screenshots, which agents read by path or
  through `get_aside`), pruned after `history_days`. Nothing needs ignoring in git.
- **The pill instead of a toolbar.** The default path must be faster than a screenshot: press, click,
  talk, press. Annotate is the default tool, and a pin needs no card; everything else (Measure, Draw,
  Shot, guides) is a key away, in the keys sheet and the right-click menu. The pill only says what
  happens (listening, where it goes) and holds what has no key over the live screen (send, close).
- **Press, not hold.** Holding `⌃⌥Space` while clicking turns clicks into right-clicks (`⌃`), changes
  measuring (`⌥`), keeps a hand on the keyboard, and sends on an accidental release. Press to open,
  press to send is as many actions.
- **English models by default, downloaded when asked.** English-only Whisper models are smaller and
  never mistake a language; multilingual ones stay a choice. Nothing downloads until the user asks
  (the microphone, Settings, `asides download`), so Asides works at once for pins and measuring.
- **Live, then frozen on demand.** The user keeps using their app while talking (the pill is a panel
  that never takes the keyboard), and freezes the screen to measure and annotate it, as
  often as they like, all in one aside. PixelSnap 2 measures what's on screen when it's invoked.
  Measuring a still frame keeps the overlay simple: the frame is what's measured, drawn
  and exported. The annotated screenshot in the aside matches exactly what the numbers describe,
  and nothing moves while you measure. The cost is the Screen Recording permission.
- **ScreenCaptureKit, in sRGB.** `CGDisplayCreateImage` and `CGWindowListCreateImage` are
  obsoleted in macOS 15. Capturing in sRGB means the colors Asides reports are close to the CSS
  colors that produced them, not the display profile's. On macOS 26 the screenshots come from its
  newer API, which keeps windows' shadows (the frozen screen looks exactly like the live one), and
  are converted to sRGB.
- **Edges are sharp, visible changes.** An edge is where a pixel differs by more than the tolerance
  from the one before it, or from the one two before (anti-aliasing spreads an edge over two
  pixels), visibly, and all at once. Slow changes aren't edges, however far they drift: on glass and
  frosted materials (macOS 26's Liquid Glass, vibrancy) the blurred background changes by a level or
  two a pixel and drifts by a hundred across a panel, while what's drawn on them jumps by tens;
  gradients and soft shadows are the same. Comparing with the scan's first pixel instead stopped
  wherever the drift passed the tolerance. Over a bright, high-contrast background (a cloud behind a
  menu), the blur can change faster than the tolerance, but it keeps changing that way for tens of
  pixels, while an edge's steep part (changing at least half as fast as where it's steepest) spans at
  most a dozen (`measure::EDGE_WIDTH`: a soft edge in a painting scaled up to the screen takes seven
  to ten, then a slower tail as long again, which doesn't count; a Gaussian blur is an edge up to
  σ = 5 px). Measured to a quarter as fast, the tail left gaps in a cloud's outline. And a change must be one the eye can see:
  the screen shows colors sRGB can't (Display P3), and converted to sRGB their clipped channels
  jump by 3 to 6 levels from one pixel to the next where the screen changes by one. Two colors
  closer than 0.008 in OKLab look the same (`measure::INVISIBLE`; a 6-level step between light grays
  is 0.018, that jitter 0.003). And a faint change must go on along a contour: wallpapers and photos
  have grain, compression specks and dust, single pixels as visible as a faint UI border (0.01 to
  0.03), and the eye tells them apart by their length. A change of 0.05 or more is an edge on its own
  pixel (`measure::STRONG`: a dot, a flower in a field); a fainter one must have a change like it on
  each of the next rows (or columns), more of them the fainter it is, up to 7 (`lines_needed`): a
  6-level gray step needs 3, so a card's border, a hairline or text are edges and grain isn't. Like
  it (`Change::like`): along its strongest channel, the same way, at least half as much, between
  colors as near its own as they are to each other (a contour parts the same two things all along;
  grain changes from shade to shade at random), ending from where it starts to a pixel after where
  it ends (`near`; three pixels before to two after let dense grain line up by chance). Where grain
  changes as much from every pixel to the next (an iMac wallpaper's, a photo's), a faint change that
  doesn't stand out from the line on each side of all it spreads over (4 times the middling step
  over 6 pixels, `stands_out`; a soft edge's own pixels change alike) must go on across 15 rows
  (`THROUGH_GRAIN`): a line drawn on grain does. And a faint change that doesn't go on still counts
  where it touches one across the line that does (`joins`, hysteresis as in Canny's edge detector):
  where a contour runs nearly along the line it moves too far from a row to the next to follow, but
  crosses the columns steeply, at the top of a faint card's rounded corner, at a hairline's ends.
  Measuring from a point follows each change's contour (`continues`); snapping an area finds every
  change of a block first, along its rows and its columns, then each one's contour length in two
  passes, top-down and bottom-up (`measure::Contours`), then the faint ones that touch an edge the
  other way (`Contours::touch`), which gives the same edges (a test checks it on grain) in a pass over
  the area. The work is shared by threads a band of rows at a time, each taking the next as it
  finishes (`measure::by_rows`), since areas are snapped on every frame of a drag. A frozen screen
  doesn't change, so its edges are found once, for all of it, on a thread of their own as soon as
  the pointer is on it (`measure::Edges`, kept by `overlay::EdgeCache` for the session's tolerance),
  and areas snapped while dragging read them: a drag then only fills. An area sees only the edges
  within it (`measure::for_area` leaves out anti-aliased ones that start or end past it), so it
  snaps the same as with edges found for it alone, which it does until they're ready. Edges never
  depend on the part of the frame being worked on, a thread's band or an area.
  An area's content is whatever the background on its outline can't reach without crossing an
  edge (a scanline fill), so no background color is needed: the outline's background is its longest
  stretch without an edge, those colored like it, and any as long as a quarter of it, so a button
  the area cuts into is still content. The default (2%) is low enough to see a light gray card on
  white, and `T` switches to 30% to look through borders and hard shadows.
- **GPUI, in its own process.** The overlay is the richest UI Asides has (the pill, cards, menus,
  live measuring at display refresh rate), and GPUI draws it on the GPU from Rust. GPUI owns its
  event loop and `NSApplication`, as tao does for the app, so the overlay is a second binary in the
  bundle, one process per session, talking JSON lines. The screenshot is taken before it shows, so
  the app being tested keeps its active look in it; when the overlay exits, that app gets the focus
  back.
- **The next overlay waits, started ahead of time.** Opening Asides took 262 ms from the
  shortcut to the frozen screens on screen (0.7.3, the median of 10). On its own, each part took:
  11 ms asking about the permission, 140 ms for the screenshot (50 listing what's shareable, 25
  converting to sRGB on one thread), and for the overlay a process (20), GPUI (40), its window (30,
  Metal compiling GPUI's shaders), reading and copying the pixels (30) and drawing. Waiting, an
  overlay has already started GPUI and opened the screens' windows, which costs 53 MB and no CPU;
  a session in it still gets a process of its own, so ending one ends all its state, as before. The
  screenshot comes from macOS 26's quickest call (`captureImageInRect`, which has the shadows and
  leaves the pointer out, as the screen shows it) and is converted by several threads: about 55 ms.
  Opening now takes 109 ms (94 to 116), the first time after launching Asides and right after
  closing it too, and freezing again from live 129 ms instead of 186.
- **Talking is the main input, in the order it happened.** Pins don't open a card, and each annotation, measurement and screenshot carries the seconds into the session
  it was made, so it lands in the transcript where the user was speaking about it, never gathered
  at the start or the end. What's said from an annotation's card is about it and goes in it. Typing
  stays available for when talking isn't.
- **Elements from the accessibility tree, opt-in.** Agentation reads the DOM from inside the page.
  A menu-bar app can't, but browsers expose the DOM's ids and classes (`AXDOMIdentifier`,
  `AXDOMClassList`) and the page's URL through accessibility, which also covers native apps. It
  needs the Accessibility permission, asked the first time Asides opens (on by default); without
  it, pins name the app and window from the window list.
- **For anything on screen, not just the web.** Agentation lives in the page (a React toolbar, the
  DOM, component names) and PixelSnap hands off to CleanShot and design apps. Asides keeps what
  works on any app: measuring pixels, annotating elements found through accessibility, drawing,
  screenshots.
- **A session is the user's own, not a child of the agent that started the app.** Agents mark the
  processes they run as part of their session (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`,
  `CLAUDE_EFFORT`, `OPENCODE`, `AGENT`…, and Diri its `DIRIJOR_SESSION_ID`), and `open` passes its
  caller's environment to the app it starts, so the app, started from an agent's session (`asides`
  run by an agent), handed them to every terminal and agent it started: Claude Code took those
  sessions for children of the first one (and stopped saving their transcripts), and a session
  started in Terminal looked like Diri's. The app drops them as it starts
  (`agents::session_markers`). An agent's settings with the same prefixes come back from its
  settings file or the login shell.
- **whisper.cpp large-v3-turbo (q5_0) via whisper-rs with Metal.** It covers all of Whisper's
  ~99 languages (Romanian and English included), is fast on Apple silicon (18 s of speech in about 1 s on an M4 Pro), and
  `cargo build` compiles it with no separate install. Apple's SpeechAnalyzer was evaluated and
  rejected: it has no Romanian model and ignored vocabulary hints.
- **One pass after stop instead of streaming.** Single-pass latency is a few seconds even for long
  recordings, and it runs while the user keeps testing. Chunked transcription would only be needed
  to handle language switches within one recording; whole-recording detection drops the minority
  language.
- **Language detection limited to a configured set.** Short clips are often misdetected as a
  related language (Romanian as Italian, for example); restricting the candidates fixes that.
- **Links plus a state file instead of a socket.** Launch Services already guarantees a single
  instance and launches the app on demand, and the CLI only needs a one-shot acknowledgement.
- **The status item is the app's mark.** Every status is the letter a's brackets with something
  between them (the a when ready, a dot while recording, dots while transcribing, a check when
  sent, a cross when cancelled…), so the menu bar and the app icon are one shape. A moment's mark
  (`icons::Mark`, an app `Flash`) says what happened on its own, and the menu says the rest: beside
  the brackets goes nothing but a recording's time or a download's progress. An aside just sent
  shows its number among the day's between them (`[5]`), in digits drawn in the same strokes; for two digits or more
  the brackets move apart. tray-icon takes RGBA, so `icons.rs` draws them itself, from strokes and
  circles, as template images the system tints for light and dark menu bars (red while
  recording). tray-icon caps icons at 22 pt, so they are drawn on a 22 pt canvas at 2×.
- **Shortcuts through Carbon hot keys** (global-hotkey). They need no Accessibility permission.
- **The command link in /usr/local/bin.** It's on every shell's default `PATH`, unlike
  `~/.local/bin`, so a download works without editing shell profiles. It usually belongs to root,
  hence the one-time administrator prompt (`osascript … with administrator privileges`). The
  Homebrew cask links into Homebrew's `bin` instead, and the app leaves any existing `asides`
  alone, so the two never collide.
- **Wispr Flow through its history, read with `/usr/bin/sqlite3`.** Wispr Flow has no public API
  for its dictations, and its local database is the record of them. The system's `sqlite3`
  (`-readonly -json`) avoids compiling SQLite into the app. Its schema changes between versions, so
  columns are discovered at run time and only `timestamp` and a text column are required.
