# Architecture

How asides fits together, for anyone about to change it. User-facing behavior is in the
[README](README.md); how to set up, test and submit changes is in [CONTRIBUTING.md](CONTRIBUTING.md).

## Bird's-eye view

```
 terminal                        Asides.app (menu bar, one process)
┌───────────┐  open -g -b …   ┌─────────────────────────────────────────────────────────────┐
│  asides   │ ──────────────▶ │ main thread: tao event loop                                 │
│   (CLI)   │   asides://…    │   App: bindings, recording, status icon and menu, shortcuts │
│           │                 │        │ Job                          ▲ Outcome             │
│           │ ◀────────────── │        ▼                              │                     │
└───────────┘   state.json    │ worker thread: resample → Whisper → write note files        │
      │                       └─────────────────────────────────────────────────────────────┘
      │ path / open / export                │ appends            ▲ JSON lines (stdin/stdout)
      ▼                                      ▼                    ▼
                                                     asides-overlay (GPUI, while Asides is open)
            <repo>/.asides/<YYYY-MM-DD-HHMM Title>/notes.md, images/, audio/
      ▲
      │ reads entries; state.json (batch sends), config.toml (delivery mode)
┌─────────────┐  stdio JSON-RPC   ┌────────────────────────────┐   asides claude|codex|gemini|…
│ asides mcp  │ ◀───────────────▶ │ coding agent (Claude Code, │ ◀── (in a terminal the menu's New
│   (CLI)     │                   │ Codex, OpenCode, …)        │     Agent Session opens, or --bg)
└─────────────┘                   └────────────────────────────┘
   writes sessions/<id>.json and listeners/<pid>.json, which the app shows in its menu;
   reads routes.jsonl (which session each entry is for), written by the app's worker
```

- **The CLI is a remote control.** It works out which directory to bind, then hands the app a link
  through Launch Services, which launches the app if needed. It reads the app's published state to
  confirm and report. Commands that only read notes (`path`, `open`, `export`) don't involve the
  app at all.
- **The app is one long-lived process.** It is menu-bar only (`LSUIElement`, accessory activation
  policy). The main thread owns the UI and recording; a single worker thread does everything slow
  or file-related.
- **Agents read, the app writes.** `asides mcp` is started by a coding agent, one per agent
  session. It reads notes and the app's published state, and hands new entries to the agent.
  Claude Code also runs `asides listen` in the background, which prints the same deliveries. The
  app never talks to either directly: it publishes batch sends and the target in `state.json`, and
  shows the session and listener records they write. The one thing agents write is what they did
  with an aside (acknowledged, resolved, dismissed, a reply), appended to `threads.jsonl` beside the
  note; the app watches it and shows it.
- **Each aside goes to one session.** Every agent listening is a session (`sessions.rs`); the app
  sends the user's asides to one of them, the target, and binds to its repo. Its worker writes each
  entry's session in `routes.jsonl` before the entry, and each `asides mcp`/`asides listen` hands
  its agent only its own. `asides claude` (and the menu's New Agent Session, which runs it in a
  terminal) starts an agent already listening, with the MCP server on the agent's command line.
- **Asides is one mode, frozen or live, in its own process.** The shortcut captures every display
  (ScreenCaptureKit) and starts `asides-overlay`, which shows the screenshots in borderless GPUI
  windows above everything; going live leaves only its toolbar, over the live screen, until the
  screen freezes again. GPUI runs its own event loop, so it can't share the
  app's (tao's) process. The session's logic (`asides-core/overlay.rs`, `measure.rs`,
  `inspect.rs`) is platform-independent; the overlay feeds it events and draws it. Permissions stay
  in the app: it hands the overlay the pixels, and names what's under an annotation when asked. The result
  reaches the worker like a screenshot does, inside the recording if one is under way.

## Code map

```
crates/
  asides-core/     platform-independent, no macOS or UI dependencies, unit-tested
    notes.rs          note folders, latest-note discovery, titles, append, entries, zip export
    frontmatter.rs    YAML metadata (written once per note)
    markdown.rs       entry composition: segments → paragraphs, image placement by timestamp
    project.rs        bind rules (git root or the current directory), git facts, .git/info/exclude
    link.rs           asides:// link format (the CLI → app protocol)
    state.rs          state.json (the app → CLI protocol), recent directories, batch sends, the target
    agents.rs         the coding agents asides starts, in one place (Claude Code, Codex, OpenCode, Gemini CLI,
                      Copilot CLI, Qwen Code): names, how each listens, runs in the background, its MCP
                      client's name and tool-call time limit, the variables marking its commands; client labels
    listening.rs      what a listening agent has seen and what's new (Cursor, Listening), listener records
    sessions.rs       agent sessions (sessions/<id>.json): records, labels, what each is doing
    routes.rs         routes.jsonl: which session each entry was spoken to
    launch.rs         starting an agent: its command line with the MCP server; the listen prompt
    threads.rs        threads.jsonl: what agents (and the user) did with each aside; aside ids (3, 3.2)
    measure.rs        PixelSnap-style measuring on a frame: distances to edges, snapping areas and guides, objects, colors
    inspect.rs        a Measure & Annotate result (measurements, guides, annotations, drawings, elements, what was
                      said about them) and its Markdown
    overlay.rs        the Measure & Annotate session state machine (Measure, Annotate, Draw, Shot; drags, holds,
                      guides, markers, annotated sections, screenshots, restore; when each was made), scripts
    takes.rs          what's said during a Measure & Annotate session, joined into one recording; its moments on it
    helper.rs         the app ↔ overlay protocol: session.json (Spec), Events, Commands, result.json (Outcome)
    keys.rs           the overlay's key chords (`[measure.keys]`): parsing, matching, labels
    install.rs        where a hand-installed app links the `asides` command, and whether one is set up
    wispr.rs          Wispr Flow's dictation history (flow.sqlite through the system's sqlite3)
    config.rs         config.toml schema and defaults, shortcut pretty-printing
    models.rs         Whisper model catalog (file names, sizes, SHA-256)
    resample.rs       windowed-sinc resampler to 16 kHz
    paths.rs          bundle ID, URL scheme, Application Support paths, tilde()
  asides-cli/      shipped inside the bundle as Asides.app/Contents/Helpers/asides
    main.rs           clap subcommands, `open -g -b <bundle id> <link>`, waits for the app to acknowledge
    start.rs          `asides claude|codex|opencode|gemini|copilot|qwen`: a session record, a worktree when another session
                      works in the repo, then `exec` of the agent (or `claude --bg`, or a Codex
                      thread or OpenCode session it pushes into)
    codex.rs          Codex sessions: a thread in its app-server daemon (WebSocket JSON-RPC through
                      `codex app-server proxy`); each aside a turn there (or `codex queue` while busy)
    opencode.rs       OpenCode sessions: a session in its background service (HTTP on localhost), its
                      MCP server for that directory, and a prompt per aside
    session.rs        the session `asides mcp`/`asides listen` deliver to: its record, listening, the target
    mcp.rs            `asides mcp`: the MCP server (waiting, reading, the feedback loop, measure_image; prompts)
    listen.rs         `asides listen`: deliveries on stdout, for agents watching it in the background
    measure.rs        `asides measure` and measure_image: PNG decoding and the measurement as text
  asides-overlay/  binary `asides-overlay`, shipped as Asides.app/Contents/MacOS/asides-overlay (GPUI)
    main.rs           reads the session, opens the live toolbar or a window per screen (or plays a script)
    ui.rs             Overlay (the session, live and frozen, and around it), ScreenView (drawing, mouse, keys)
                      and ToolbarView: the toolbar, the annotation card, the context menu, the keys sheet,
                      toasts, talking, finishing
    windows.rs        the windows: opened, shown and hidden by mode, the live toolbar fitted and moved
    input.rs          the card's text field (GPUI's input example: IME, selection, clipboard; wraps and grows)
    export.rs         the images that leave it (aside, Shot, clipboard), drawn with AppKit into a bitmap
    finish.rs         result.json: the screen images, each measurement's and annotation's Markdown, the screenshots, by time
    style.rs          colors and shapes shared by the windows and the exported images
    native.rs         what GPUI's windows need to be an overlay (borderless, screen-saver level, all spaces),
                      and the live toolbar's panel (never key)
    io.rs             Events on stdout, Commands from stdin
    replay.rs         development: plays mouse and key steps as AppKit events and captures the windows
    assets.rs         the toolbar's icons (SVG)
  asides-app/      binary `asides-app`, renamed Asides inside the bundle
    main.rs           App: event loop, UserEvent dispatch, recording state machine, bind(), refresh(), publish()
    tray.rs           status item and menu (MenuAction, View, Directory submenu)
    icons.rs          the status item's icons: the asides mark per status, drawn to RGBA for tray-icon
    hotkeys.rs        global shortcuts (Carbon hot keys): Asides, a new note, freeze, attach, send, switch
    audio.rs          cpal microphone capture → mono f32 at the device rate
    worker.rs         Job queue: transcription, note creation, images, audio, Wispr Flow dictations, routes, append;
                      transcribing what was said about an annotation, for its card
    watch.rs          polling threads: Wispr Flow's history, agent sessions and listeners, the latest note's threads
    terminal.rs       New Agent Session: a script for Terminal, Ghostty (a tab), iTerm or Diri; the login shell (PATH, agents)
    capture.rs        ScreenCaptureKit screenshots (sRGB) → frames; the Screen Recording permission; displays
    overlay.rs        starts asides-overlay with a session directory, relays its messages, reads its result
    elements.rs       what's under an annotation: the window list, and the accessibility tree (AXUIElement)
    stt.rs            ModelManager (download, verify, wait), Transcriber (whisper-rs), language choice
    clipboard.rs      NSPasteboard: images (PNG, TIFF, file URLs), change counter, copy text
    environment.rs    macOS version, frontmost or last-active browser, display (note metadata)
    macos.rs          microphone permission, folder picker, sounds, open/reveal, the command link
packaging/Info.plist  bundle ID, LSUIElement, NSMicrophoneUsageDescription, URL scheme, icon (Screen Recording
                      and Accessibility need no usage string: macOS shows its own prompts)
packaging/Asides.icns the app icon, drawn by scripts/make-icon.swift
packaging/Asides.entitlements   microphone entitlement (required by the hardened runtime)
packaging/download/   the Cloudflare Worker that serves the website's disk image (static files only)
packaging/site/       the Cloudflare Worker that serves the website, docs/ (static files only)
scripts/              e2e.sh (+ e2e-mcp.py, a scripted agent), make-fixtures.sh, signing-identity.sh,
                      make-signing-cert.sh, release.sh, make-dmg.sh, publish-dmg.sh, make-og.py,
                      make-icon.swift
docs/                 the website (asides.mihaicristian.dev): index.html, wallpaper.webp, favicon.svg, og.png
Casks/asides.rb       the Homebrew cask; the repository doubles as a tap
```

## Threads and how they talk

| Thread | Owns | Talks back through |
|---|---|---|
| Main (tao event loop) | `App`, the status item and menu, shortcut registration, the cpal `Stream` | n/a |
| cpal callback (CoreAudio) | pushes samples into `Arc<Mutex<Vec<f32>>>` | the shared buffer, read on stop |
| Worker (`worker::spawn`) | the loaded `Transcriber`, **all note file writes** | `UserEvent::Work(Outcome)` |
| Model download (one per model) | the `.part` file | `UserEvent::Model(id, ModelState)`, plus a `Condvar` for the worker |
| Config watcher | polls `config.toml`'s modification time every 2 s | `UserEvent::ConfigFileChanged` |
| Wispr Flow watcher (while on) | a `wispr::History`; queries only after `flow.sqlite` changes | `UserEvent::Wispr` |
| Sessions watcher | reads `listeners/` and `sessions/` every 2 s, removes records of ended processes | `UserEvent::Sessions` |
| Agent finder (at launch, and when the menu opens, at most once a minute) | asks the login shell which agents are installed | `UserEvent::Agents` |
| Background launch (per launch) | runs `asides claude --background` in the login shell | `UserEvent::Launched` |
| Threads watcher | checks the bound directory's latest `threads.jsonl` every second | `UserEvent::Threads` |
| ScreenCaptureKit queues | turn each display's screenshot into a frame | `UserEvent::Captured` |
| Overlay reader (while it's open) | reads `asides-overlay`'s standard output | `UserEvent::Overlay` |
| global-hotkey, muda, tray-icon | | `UserEvent::HotKey`, `UserEvent::Menu`, `UserEvent::MenuOpened` |

Background threads never touch AppKit objects. They send `UserEvent`s through the
`EventLoopProxy`, and `App::handle` runs them on the main thread. The overlay is another process
with its own main thread (GPUI's); the app writes its commands to the overlay's standard input from
the main thread.

## Lifecycles

### Binding (`asides` in a repo)

1. The CLI calls `project::resolve_bind_dir` (git root, otherwise the current directory) and
   `ensure_git_exclude`.
2. It sends `asides://bind?dir=…&new=1&title=…&req=<id>` with `open -g -b <bundle id>`.
3. The app gets `Event::Opened`, then `handle_link`, then `App::bind`. That sets the bound
   directory and the pending "new note" request, moves the directory to the front of the recent
   list, and publishes `state.json` with `last_request = <id>`.
4. The CLI polls `state.json` until it sees its `<id>`, then prints the "ready" line.

Choosing a directory from the menu goes through `bind_from_menu`, which applies the same rules
(repo root, git exclude) and then calls `bind`.

### A recording

1. **Start** (`start_recording`, while Asides is open: its talk key, a microphone, the Asides
   shortcut held, or `measure.record`):
   - checks the binding and the microphone permission, and starts cpal
   - snapshots the environment (browser, display) and the clipboard's change count
   - looks up where the user is (`elements::place`, on a thread of its own: an app can be slow to
     answer accessibility): the frontmost window of the app a frozen screen stands for, or of the
     one in front, its title, and the page's URL with `identify_elements`. It goes in the aside as
     `**In:** …` where the take starts (`markdown::without_repeated_places` drops repeats)
   - consumes any pending "new note" request
   - plays the start sound
2. **While recording** (`tick`, every 250 ms): updates the timer in the menu bar. With
   auto-attach on, a changed clipboard holding an image becomes a `CapturedImage` with its offset
   from the start.
3. **Stop** (`stop_recording`): stops cpal. Recordings under 0.3 s with no images are dropped as
   accidental. In Asides, the rest are takes (below); otherwise (`asides simulate`) a
   `Job::Recording`.
4. **Worker** (`process_recording`):
   - resamples to 16 kHz and checks for silence
   - waits for and loads the model (cached; unloaded after 15 idle minutes). Loading takes seconds
     (5.8 s for large-v3-turbo on an M4 Pro), so a recording queues `Job::Warm` as it starts: the
     worker loads it while the user talks
   - transcribes: language detection limited to `config.languages`, then one beam-search pass
   - picks the target note: the latest one, or a new one with metadata when asked for, or (with
     `auto_new_note`) when the latest was started on another git branch or not written to for 8
     hours (`notes::moved_on`); the CLI and the menu say so before the next recording
   - saves images and optional audio, composes the entry and appends it
5. **Outcome:** the main thread flashes `✓ #N`, re-reads the latest note and publishes the state.

### Asides: live and frozen

1. **Open** (`App::open_asides`, on `⌃⌥Space`, `⌃⌥N`, `⌃⌥P`, the menu or `asides annotate`): Asides
   opens frozen. `capture_screens` checks the Screen Recording permission (asking for it the first
   time), lists the displays and snapshots the window list and the frontmost app, then asks
   ScreenCaptureKit for one sRGB screenshot per display, and looks up where the user is (as a
   recording does) while it does: it goes with the screen holding that window (`SpecScreen::place`),
   said with its screenshot when no annotation says where it is. On `UserEvent::Captured`, `show_overlay`:
   `overlay::start` writes each display's pixels and `session.json` (`helper::Spec`: the screens, the
   settings and keys, the last session's guides, the toolbar's place, whether a recording is under
   way; `live` opens it over the live screen instead, for development) into a temporary session
   directory, and starts `asides-overlay` with it. With `measure.record`, recording starts too.
   While it's open, the Asides shortcut tapped adds it all (`finish`), and held, talks until it's
   let go (`App::held`, from the clock), from the press that opened it too: that recording is the
   session's (`started_recording`).
2. **The live toolbar** (`windows::open_toolbar`, on ▶ or `⌃⌥P`): a small GPUI pop-up window (a
   non-activating panel that never becomes key, `native::make_toolbar`) on the screen under the
   pointer, where the user left it (`toolbar_at`, a fraction of the screen) or at the bottom,
   sized to the toolbar (`follow_toolbar`). The user's apps keep the mouse and keyboard. Its tools,
   ⏸, or the freeze shortcut (`toggle_freeze`), ask to freeze (`freeze`).
3. **Freeze again** (`App::capture_screens`, as when it opens, leaving out the overlay's own
   windows): the pixels go into the session directory and to the overlay (`frozen`), which starts
   a new freeze of the session (`Session::refrozen`: numbering and clock go on).
4. **The frozen screens** (`windows::open_screens`, the first time): one borderless window per
   display (GPUI, made a non-activating panel at screen-saver level on every space by
   `native.rs`) showing the screenshot, with Measure's lines where the pointer is; the overlay
   activates itself, a regular app while frozen so the kiosk options hold (`native::kiosk`), and
   says `ready`. ▶ or `⌃⌥P` goes back to live (`live`): the overlay gives the keyboard back to the
   app that had it, and once that app is in front again (`windows::handed_back`), the screens hide
   and the toolbar shows; what was done on the freeze is kept for the aside (`finish::items`, once
   the live toolbar shows). Closing frozen gives it back too;
   closing live leaves it with whatever app the user is in. The windows open hidden and change from
   the clock (`windows::sync`), outside GPUI's updates and not from inside their events: each is
   drawn before it shows (`native::draw_now`), so it never shows its last freeze, and the screens
   and the toolbar take each other's place at once. The view turns mouse and key events into `overlay::Session` calls (in the screen's
   points) and redraws, stamping what's added with the seconds into the session (`Session::clock`).
   A new annotation (and, with Annotate, the element under a resting cursor, which the session also
   gets, so a click annotates it rather than a larger measured section around it) is looked up with
   `identify`: the app answers from `elements::identify` (the app and window from
   the window list, and with `identify_elements` and the Accessibility permission, the element from
   the app's accessibility tree, hit-testing that app so the overlay isn't hit). The talk key and the
   microphones send `talk` down and up; the app treats them like the record shortcut and tells the
   overlay when it records (`recording`), for its toolbar. A recording that stops while the overlay
   is open isn't written: it's a take (`App::takes`), kept for the end of the session. One started
   from an annotation's card (or while it's open) is about that annotation (`talk` names its key):
   when it stops, the worker transcribes it straight away (`Job::Transcribe`), and the app sends the
   transcript to the overlay (`transcribing`, then `said`), which shows it in the card and keeps it
   in the annotation (`Annotation::said`). The card's trash sends `unsay`. Closing the card (it
   stops a recording for it) doesn't wait for the transcript. Without a bound directory the spec
   says so (`bound: None`): the overlay shows it from the start and won't record or finish. Errors the
   app raises while the overlay covers the menu bar are also sent to it (`notice`). The toolbar
   moves by its handle and folds; where, and whether, is remembered (`toolbar_placed`, `state.json`).
   Nothing the overlay draws goes above a screen's menu bar, where the notch is: the app measures it
   (`SpecScreen::menu_bar`, from the visible frame and the safe area), and labels, banners, cards,
   menus and the toolbar, live or frozen, stay below it.
5. **Finish** (Done, Return on a frozen screen, or the shortcut tapped, which the app forwards as
   `finish`): for each freeze, and each screen with something on it, the overlay renders the
   screenshot cropped around what's there
   (`export.rs`, sRGB, the screen's resolution) and `inspect` writes the Markdown: what goes with the
   screenshot (where, the screen and units, the guides), then each measurement (a list item) and
   each annotation (with what was said about it, `**Said:**`). `result.json` lists them with the
   seconds into the session each was made, the screenshot at the first of its screen's, plus the
   Shot tool's screenshots; an annotation's item names its key and the recordings it already has.
   On `finished`, the app first ends a recording about an annotation and waits for what was said
   about the annotations to be transcribed (`App::finishing`, `try_finish`), adding what the overlay
   didn't have yet to their blocks (`inspect::with_said`). Then, when something was said in the
   session (takes, or a recording started in it, which stops with it), `App::add_session` joins the
   takes into one recording (`takes::join`: in order, with a pause between them) and puts each item
   at its moment on it (`Joined::offset`), as text attachments (`markdown::Attachment.text`), so they
   sit among what was said: one `Job::Recording`, one entry. Measurements made one after another,
   with nothing said between them, stay one list (`markdown::Blocks`). Otherwise `App::add_inspection` adds
   them to the recording that was under way when the overlay opened (which goes on), or a
   `Job::Screen` writes `## N · HH:MM · Screen`, in the same order. Send also sends to the agent.
   `cancelled` (the toolbar's ×) adds nothing, the session's takes included; a recording that was under way when
   it opened is kept. If the overlay dies, or Asides quits, the takes are kept as a recording
   (`keep_takes`). Either way the app keeps the guides
   and held measurements for the next session's R, removes the session directory when the process
   exits, and gives the focus back to the app that had it.
6. **Dev hooks:** `asides simulate-measure IMAGE SCRIPT` starts the overlay frozen with an
   `overlay::Script` in the spec: it plays it without windows (`live` and `freeze` steps freeze it
   again) and finishes. The end-to-end test uses
   it. A `replay` in the spec plays mouse and key steps in real windows instead, as AppKit events,
   and captures them (`screencapture -l`), to check what the user sees.

### What happens to an aside (threads)

`acknowledge_aside`, `resolve_aside`, `dismiss_aside` and `reply_to_aside` check that the aside exists
in the note (`3`, or `3.2` for the annotation `### 2.` of entry 3), then append one event to the note's
`threads.jsonl`: who (agent or human, and the agent's name), when, a status and/or a message. Each
event is one `write` on a file opened for appending, so concurrent writers don't interleave.
Readers fold the events into each aside's state (`threads::fold`). An annotation without events of
its own takes its entry's status, and an entry with annotations is open while any of them is
(`threads::entry_status`). The app's threads watcher reports the latest note's events. It flashes
the newest agent event in the menu bar (and, with the screen frozen over the menu bar, sends it to
the overlay as a `notice`), lists recent ones under Agent Replies, and the note line
counts the open asides. `get_asides` and `get_pending_asides` quote each aside's thread under it.

### A Wispr Flow dictation

With `wispr_flow` on, the watcher reports each dictation Wispr Flow finished since the watcher
started (`formatted` or `raw_transcript` rows of its `History` table, not archived). The main thread
checks `wispr_flow_apps` and the binding, consumes a pending "new note" request like a recording
would, and submits a `Job::Dictation`. The worker appends `## N · HH:MM · Wispr Flow` and the text,
as Wispr Flow typed it.

### Agent sessions (`asides claude`, `asides mcp`, `asides listen`)

1. **A session starts** one of two ways:
   - `asides claude` (or `codex`, `opencode`, `gemini`, `copilot`, `qwen`; the menu's New Agent Session runs it in a terminal
     through a `.command` script, or in the login shell with `--background`) resolves the repo,
     writes `sessions/<id>.json` (listening, with where its asides start: a `Listening` token),
     sends `asides://session?id=…` so the app makes it the target, and `exec`s the agent: from then
     on the record's process is the agent. The agent's command line (`launch.rs`) carries the MCP
     server (`asides -C <repo> mcp --session <id>`; Claude Code's `--mcp-config`, Codex's `-c
     mcp_servers.asides.*`, OpenCode's `OPENCODE_CONFIG_CONTENT` with `--standalone`, Copilot CLI's
     `--additional-mcp-config`, Qwen Code's `--mcp-config`), the listen prompt as its first message,
     and its asides tools pre-approved (Claude Code's `--allowedTools`, Copilot CLI's `--allow-tool`,
     a trusted server for Qwen Code). Those that wait in a tool call get an hour for one (Codex's
     `tool_timeout_sec`, the others' `timeout`) and wait 50 minutes. Gemini CLI takes servers only
     from settings files (a system one only from a folder root owns), so the first `asides gemini`
     adds the user's (`gemini mcp add --scope user --trust`), and it's the session's server through
     `ASIDES_SESSION` and `ASIDES_REPO`, which Gemini passes to its servers. When another
     session is alive in the repo, the new one works in a git worktree: Claude Code makes it
     (`--worktree`); for Codex and OpenCode, `start.rs` makes it in `.asides/worktrees/` and removes
     it at a later start if nothing was done in it.
   - `asides codex` starts the session as a thread in Codex's app-server daemon instead
     (`codex.rs`: `thread/start` with the MCP server in its config, `thread/name/set`, the first
     prompt with `turn/start`), and a process of `asides listen --codex-thread` that pushes each
     delivery into it (the session is `pushed`: `wait_for_asides` returns at once): a `turn/start`
     with the screenshots attached when the thread is idle (loaded again with its MCP server, with
     `thread/resume`, when the daemon has let it go), `codex queue` when it's busy (which takes no
     images: the text names them). A push that fails is retried, less and less often, and the
     pusher's listener record says why (`problem`), which the menu shows (`not getting asides`).
     In a terminal, the agent `exec`d is `codex resume <thread>`, and the pusher stops with
     it; in the background, the pusher is the session's process, and Stop ends it. A daemon that
     isn't running is started (`codex app-server daemon start`); without one (an older Codex),
     Codex waits in `wait_for_asides` like other agents.
   - `asides opencode` (OpenCode 2) does the same in OpenCode's background service (`opencode.rs`,
     found through the address and password it registers in `~/.local/state/opencode/service.json`,
     started with `opencode serve --service` if it isn't running): the session's MCP server for its
     directory (`PUT /api/experimental/mcp/asides`), the session (`POST /api/session`), and the first
     prompt; `asides listen --opencode-session` pushes each delivery as a prompt with `delivery:
     "queue"`, and when it stops, puts back the user's own `asides` server for that directory. The
     agent `exec`d is `opencode --session <id>`. OpenCode 1, or a service that won't start, waits in
     `wait_for_asides` with the server in `OPENCODE_CONFIG_CONTENT`.
   - An agent the user started connects to `asides mcp`, which writes a record that isn't
     listening yet (and removes it when the agent hangs up). When the user asks it to listen (the
     `listen` prompt, its first `wait_for_asides`, or `asides listen --session <id>`, the command
     its instructions give), it's marked listening and sends the same link.
2. **The target.** The app reads both directories (`watch::sessions`). A session it hasn't seen
   before becomes the target, which binds the app to its repo; the menu lists the sessions under
   Asides Go To with Notes Only, and `⌃⌥Tab` (registered while there are sessions) cycles through
   them. When the target ends, the newest other session in its repo takes over, or Notes Only.
   `asides` (bind) and the Directory menu pick the newest session in the directory they bind.
3. **Routes.** A recording (or dictation, or Screen entry) takes the target when it starts. The
   worker appends `{entry, session, agent}` to `routes.jsonl`, then the entry: an agent that sees the
   entry sees its route too. With no session in the repo nothing is written (entries without a
   route reach every listener there); Notes Only writes a route with no session.
4. **Listening** (`agents::Listening`, as its session): it starts where the launch recorded, or at
   the first wait:
   - `each`: the cursor starts at the end of the notes; only entries written from then on are new.
   - `batch`: the cursor starts after the last Send to Agent in this directory (or at the start of
     the latest note), so the next batch includes asides recorded before the agent listened.
   - Sends made before listening started are ignored: the user chose them with nobody listening.
5. Both poll every 400 ms, reading `config.toml` for `agent_delivery`:
   - `each`: any entry after the cursor routed to this session (or to nobody in particular), plus
     text added to it later (an image attached after it). The cursor moves past other sessions'
     entries.
   - `batch`: only once the newest of `state.json`'s `agent_sends` is newer than the last one acted
     on and names this directory; then the entries up to its note and entry number, when the send
     names this session (the target when the user sent it). Otherwise the cursor moves past them.
     The app publishes a send after Send to Agent, once queued recordings are transcribed, and only
     to a target session; otherwise it says no agent is listening and sends nothing.
6. The server returns the entries (text with absolute links, then the screenshots as images) and
   moves the cursor past them. With nothing new by the deadline (50 s, or 25 min for Claude Code,
   whose tool calls may last longer; launched Codex sessions wait 50 min, with the tool timeout
   raised to match), it says so, and why asides may not be coming (another session is the target,
   Notes Only), and the agent calls again. Progress notifications every 20 s keep the call alive in
   clients that time out idle calls. `asides listen` prints each delivery as one block; with
   `--timeout` it stops and prints the command that carries on, its position in an `--after` token.
7. When the agent hangs up (stdin closes), or `asides listen` is stopped, the listener record is
   removed, and the session record when that process made it. Records of processes that died are
   removed by the app.

## Invariants

Break these only on purpose, and update this document when you do.

1. **`asides-core` stays platform-independent.** No AppKit, objc2 or audio crates. Logic that
   can live there should, because that's where it can be unit-tested.
2. **AppKit, tray and menu objects are only touched on the main thread.** Background work reports
   back through `UserEvent`.
3. **Only the worker writes note files** (`notes.md`, images, audio), one job at a time, in the
   order submitted. That keeps entries ordered and numbered correctly even when recordings queue up,
   and holds for entries from Measure & Annotate too (the overlay writes only to its session
   directory). `routes.jsonl` is the worker's too, written just before each entry. The exception is
   `threads.jsonl`, which `asides mcp` appends to directly: one event per `write`, append-only,
   never rewritten.
4. **Notes are append-only.** Entries are added with a single append; existing content is never
   rewritten. Editors and agents can keep the file open, and users can edit notes freely.
5. **Metadata is written once,** when a note is created. Entries carry only `## N · HH:MM`, plus
   an audio link or a source (`· Wispr Flow`) when there is one.
6. **Transcripts are verbatim.** No prompt, no vocabulary, no post-processing; in testing, a
   prompt silently dropped words. Changes here need a design discussion first. Wispr Flow entries
   are Wispr Flow's text as it typed it, marked with their source, and not changed either.
7. **The CLI ↔ app protocol is `link.rs` plus `state.rs`.** Change both sides together. New
   `state.json` and `config.toml` fields need serde defaults, so files written by older versions
   still load. `asides mcp` uses the same files, plus the listener records in `agents.rs` and the
   session records in `sessions.rs` (also with serde defaults).
8. **The app is launched by Launch Services, never as a child of a terminal.** Otherwise macOS
   attributes the microphone permission to the terminal, and the app dies with the terminal.
9. **Binary names must differ by more than letter case.** On case-insensitive APFS,
   `asides` and `Asides` in `target/release/` are the same file.
10. **Signed with a stable identity and the hardened runtime.** macOS ties the microphone
    permission to the signing identity, so ad hoc signatures re-prompt after every build. Under the
    hardened runtime every capability needs an entitlement (`packaging/Asides.entitlements`).
11. **No new permissions without discussion.** Recording needs only the microphone (plus the
    optional clipboard prompt). Measure & Annotate added, at the maintainer's request, Screen
    Recording (asked for the first time it's used, never for recording) and Accessibility (only
    with `identify_elements` on). New Agent Session added, also at the maintainer's request,
    Automation for Ghostty only (asked the first time a session opens in a running Ghostty, for a
    tab; declined, sessions keep a window of their own). Input Monitoring is still not needed.
    Anything else changes what users must trust, so discuss it first.
12. **`asides mcp` writes nothing to stdout but JSON-RPC.** Anything else breaks the agent's
    connection. `asides listen` writes only deliveries and notices: each burst of output is a
    notification in the agent's chat. Neither writes notes. Listening (the first wait) makes the
    agent's session the target, since the user asked it to listen; nothing else an agent does
    changes the app. The feedback-loop tools only append to `threads.jsonl`. Every other tool stays
    read-only (and marked so), because agents like Codex ask before every call to a tool that isn't.
13. **The overlay needs no permission of its own.** Screenshots (Screen Recording) and naming
    elements (Accessibility) stay in the app, which hands the overlay pixels and answers `identify`.
    Its standard output carries only protocol lines (`helper::Event`); diagnostics go to standard
    error.
14. **Measuring logic stays in `asides-core`.** The overlay draws and routes events, but distances,
    snapping, guides, marks and their Markdown are computed in `measure.rs`, `overlay.rs` and
    `inspect.rs`, where they're unit-tested, and shared with `asides measure` and `measure_image`.
15. **The overlay's UI is GPUI.** New on-screen UI for Measure & Annotate is built with GPUI in
    `asides-overlay`. The exported images are drawn with AppKit only because GPUI can't render
    offscreen; they follow `style.rs`.
16. **Each aside goes to one session.** Routes are written before their entry, and a listener moves
    past what isn't its own, so two agents never act on the same aside. Notes are written whatever
    the target; a session only decides who gets the prompt.

## Persistent state

| Path | Written by | Notes |
|---|---|---|
| `<dir>/.asides/…` | worker | the user's notes; excluded from git locally |
| `~/Library/Application Support/Asides/config.toml` | app (menu), user | reloaded on change; unknown fields ignored, missing ones defaulted |
| `~/Library/Application Support/Asides/state.json` | app | binding, pending new note, status, recent directories, last acknowledged request, last Send to Agent, whether the user declined the command link |
| `/usr/local/bin/asides` | app | for a copy installed by hand: a link to the CLI in the bundle, made on first launch |
| `~/Library/Application Support/Asides/models/` | download thread | `.part` while downloading, renamed after the SHA-256 check |
| `~/Library/Application Support/Asides/listeners/<pid>.json` | `asides mcp`, `asides listen` | one per process listening, naming its session; stale ones removed by the app |
| `~/Library/Application Support/Asides/sessions/<id>.json` | `asides <agent>`, `asides mcp`, `asides listen` | one per agent session (listening or not yet); removed when it ends, or by the app once its process is gone |
| `<dir>/.asides/<note>/routes.jsonl` | worker | which session each entry was spoken to; one JSON line per entry, append-only |
| `<dir>/.asides/worktrees/<name>/` | `asides opencode` | a git worktree for a second OpenCode session in the repo; removed at a later start if nothing was done in it |
| `$TMPDIR/asides-sessions/*.command` | app | New Agent Session's (and Show's) script for the terminal; deletes itself when it runs |
| `$TMPDIR/asides-sessions/push-<id>.log` | `asides listen --codex-thread` | what went wrong pushing a Codex session's asides |
| `<dir>/.asides/<note>/threads.jsonl` | `asides mcp` | what happened to each aside since; one JSON event per line, append-only |
| `$TMPDIR/asides-overlay-<pid>-<n>/` | app, `asides-overlay` | a Measure & Annotate session: the screens' pixels, `session.json`, `result.json`, images; removed when the overlay exits |
| `~/Library/Application Support/Wispr Flow/flow.sqlite` | Wispr Flow | read-only, only with `wispr_flow` on. Dev-hooks builds read `Asides/test-wispr-flow.sqlite` instead when it exists (end-to-end test) |

## Design decisions

- **Live, then frozen on demand.** The user keeps using their app while talking (the toolbar is a
  panel that never takes the keyboard), and freezes the screen to measure and annotate it, as
  often as they like, all in one aside. PixelSnap 2 measures what's on screen when it's invoked.
  Measuring a still frame keeps the overlay simple: the frame is what's measured, drawn
  and exported. The annotated screenshot in the note matches exactly what the numbers describe,
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
  pixels: an edge's change spreads over at most a dozen (`measure::EDGE_WIDTH`; a soft edge in a
  painting scaled up to the screen takes about seven). And a change must be one the eye can see:
  the screen shows colors sRGB can't (Display P3), and converted to sRGB their clipped channels
  jump by 3 to 6 levels from one pixel to the next where the screen changes by one. Two colors
  closer than 0.008 in OKLab look the same (`measure::INVISIBLE`; a 6-level step between light grays
  is 0.015, that jitter 0.003). The edge masks are worked out a band of rows per thread
  (`measure::by_rows`), since areas are snapped on every frame of a drag.
  An area's content is whatever the background on its outline can't reach without crossing an
  edge (a scanline fill), so no background color is needed: the outline's background is its longest
  stretch without an edge, those colored like it, and any as long as a quarter of it, so a button
  the area cuts into is still content. The default (2%) is low enough to see a light gray card on
  white, and `T` switches to 30% to look through borders and hard shadows.
- **GPUI, in its own process.** The overlay is the richest UI Asides has (a toolbar, cards, menus,
  live measuring at display refresh rate), and GPUI draws it on the GPU from Rust. GPUI owns its
  event loop and `NSApplication`, as tao does for the app, so the overlay is a second binary in the
  bundle, started per session and talking JSON lines. The screenshot is taken before it starts, so
  the app being tested keeps its active look in it; when the overlay exits, that app gets the focus
  back.
- **Talking is the main input, in the order it happened.** Annotations made while recording don't
  open a card, and each annotation, measurement and screenshot carries the seconds into the session
  it was made, so it lands in the transcript where the user was speaking about it, never gathered
  at the start or the end. What's said from an annotation's card is about it and goes in it. Typing
  stays available for when talking isn't.
- **Elements from the accessibility tree, opt-in.** Agentation reads the DOM from inside the page.
  A menu-bar app can't, but browsers expose the DOM's ids and classes (`AXDOMIdentifier`,
  `AXDOMClassList`) and the page's URL through accessibility, which also covers native apps. It
  needs the Accessibility permission, so it's off until the user turns it on; without it, markers
  name the app and window from the window list.
- **For anything on screen, not just the web.** Agentation lives in the page (a React toolbar, the
  DOM, component names) and PixelSnap hands off to CleanShot and design apps. Asides keeps what
  works on any app: measuring pixels, annotating elements found through accessibility, drawing,
  screenshots, and the agent's feedback loop through MCP.
- **Sessions and one target.** The user talks to one agent at a time, so each aside goes to one
  session, picked in the menu or with `⌃⌥Tab`, and a new session (just started, or just asked to
  listen) becomes it: no binding step, no bind tool. Broadcasting to every agent in a repo had two
  agents in one repo act on the same request.
- **Routes in a sidecar, written before the entry.** The note stays a plain record of what was said
  (invariant 5), and an agent polling it can't see an entry before its route.
- **Agents started with their own command lines.** Most agents can take an MCP server on their
  command line (Claude Code's `--mcp-config` overrides a user-scope server of the same name; Codex's
  `-c`; OpenCode's inline config, read by a `--standalone` server since OpenCode 2's shared service
  wouldn't see it; Copilot CLI's and Qwen Code's JSON), so `asides claude` needs no `mcp add` and
  ties the server to its session. Gemini CLI can't (a settings file of the session's own must be in
  a folder owned by root), so it gets the user's server, registered once, and the session through
  its environment. `exec` keeps the agent in charge of the terminal (Ctrl-Z, signals, exit code).
- **One module for the agents.** What differs between agents (names, how each takes a server and a
  first prompt, how it listens, its MCP client's name and time limit for a tool call, the variables
  it marks its commands with) is declared in `agents.rs` and built into command lines in
  `launch.rs`; the rest of asides asks them. Agents asides doesn't start work through `asides mcp`
  set up by hand, labeled by their client's name (`agents::client_label`).
- **Terminals through scripts; Ghostty tabs through AppleScript.** Sessions open in Terminal unless
  the user picks another app. Terminal and iTerm run a `.command` script they're asked to open
  (`open -a`), in a new window. A tab is better (the user's window, not one per session), but it
  takes AppleScript, and so the Automation permission: a running Ghostty gets `new tab in front
  window` with the script as the surface's command. macOS asks once; the AppleScript waits for the
  answer (up to the Apple event timeout, 2 minutes), so terminals open off the main thread. When
  Ghostty isn't running (starting it would open a window of its own too) or the user said no,
  Ghostty is started as a process of its own (`open -na`) with the script as `--initial-command`,
  since it asks before running a script it's asked to open; that process quits when its window
  closes. Terminal's AppleScript can't make tabs (only ⌘T through System Events, which needs
  Accessibility). iTerm's can (`create tab`), but isn't wired up: it wasn't available to test. Diri
  runs the script as a session in its window (`dirijor session run`). The script runs the command
  in the user's login shell (their PATH).
- **A session is the user's own, not a child of the agent that started the app.** Agents mark the
  processes they run as part of their session (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`,
  `CLAUDE_EFFORT`, `OPENCODE`, `AGENT`…), and `open` passes its caller's environment to the app it
  starts, so an `asides mcp` starting the app handed them to every terminal and agent the app
  started: Claude Code took those sessions for children of the first one (and stopped saving their
  transcripts). The app drops them as it starts, and so does `asides <agent>`
  (`launch::session_markers`). An agent's settings with the same prefixes come back from its
  settings file or the login shell.
- **A worktree for a second session in a repo.** Two agents editing one working tree collide.
  Claude Code manages its own worktrees (and offers to clean them up). Codex's `--worktree` doesn't
  apply to a thread started through its daemon, and OpenCode has none, so for them asides makes one
  inside `.asides/`, which git already ignores locally. Either way the worktree branches from the
  user's HEAD, the branch they're testing: Claude Code's default is origin's default branch, so
  asides passes it `worktree.baseRef = "head"` with `--settings`, for that session only.
- **Codex pushed, not polled.** In `wait_for_asides`, Codex is busy in a tool call that times out
  every 50 minutes, and you can't type to it. Its daemon runs threads whether or not a window has
  them open, and a client can start a turn in one (a window that has it open shows it), or queue a
  message for after the turn in progress (`codex queue`), so asides starts the thread there and
  pushes each aside: Codex is idle between asides, in a window or in the background. OpenCode 2 has the
  same shape: its TUI is a client of a background service that runs the sessions, so asides talks
  to that service as the TUI does. (`opencode run` would be simpler, but OpenCode's free models
  refuse it; they answer the service.) The service runs MCP servers per directory, not per session,
  so a session's own server replaces the user's `asides` server in its directory while it runs:
  two would offer every tool twice, and the user's would make the agent listen as another
  session.
- **Threads in a sidecar, append-only.** Agentation keeps annotation status and replies in its
  server's store. Asides keeps notes append-only and readable by anyone, so status and replies
  go in `threads.jsonl` next to `notes.md`. Several agent sessions write to it with single appends,
  without coordinating, and readers fold the events.

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
  added, a cross when cancelled…), so the menu bar and the app icon are one shape. A moment's mark
  (`icons::Mark`, an app `Flash`) says what happened on its own; beside the brackets goes only a
  word or a number (`#3`, `→ Codex`), never a second symbol. tray-icon takes RGBA, so `icons.rs` draws them itself, from
  strokes and circles, as template images the system tints for light and dark menu bars (red while
  recording). tray-icon caps icons at 22 pt, so they are drawn on a 22 pt canvas at 2×.
- **Shortcuts through Carbon hot keys** (global-hotkey). They need no Accessibility permission,
  and they report key-up, which makes hold-to-talk possible.
- **Agents wait in a tool call, or watch a command.** MCP has no portable way for a server to start
  a turn in the agent, so `wait_for_asides` blocks until there's something to deliver, and its
  description and result tell the agent to call it again. It works the same in every client, but
  the agent is busy while it waits, and tool results are hidden. Claude Code can watch a command's
  output in the background (its Monitor tool), with each burst a notification in the chat, so there
  `asides listen` delivers instead: the user sees each aside, and the agent is free in between.
  Claude Code's channels could push into a session too, but they need a development flag in every
  session.
- **The command link in /usr/local/bin.** It's on every shell's default `PATH`, unlike
  `~/.local/bin`, so a download works without editing shell profiles. It usually belongs to root,
  hence the one-time administrator prompt (`osascript … with administrator privileges`). The
  Homebrew cask links into Homebrew's `bin` instead, and the app leaves any existing `asides`
  alone, so the two never collide.
- **Wispr Flow through its history, read with `/usr/bin/sqlite3`.** Wispr Flow has no public API
  for its dictations, and its local database is the record of them. The system's `sqlite3`
  (`-readonly -json`) avoids compiling SQLite into the app. Its schema changes between versions, so
  columns are discovered at run time and only `timestamp` and a text column are required.
