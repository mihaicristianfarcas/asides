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
                                                     asides-overlay (GPUI, while Measure & Annotate is open)
            <repo>/.asides/<YYYY-MM-DD-HHMM Title>/notes.md, images/, audio/
      ▲
      │ reads entries; state.json (batch sends), config.toml (delivery mode)
┌─────────────┐  stdio JSON-RPC   ┌────────────────────────────┐
│ asides mcp  │ ◀───────────────▶ │ coding agent (Claude Code, │
│   (CLI)     │                   │ Codex, OpenCode, …)        │
└─────────────┘                   └────────────────────────────┘
   writes listeners/<pid>.json, which the app shows in its menu
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
  app never talks to either directly: it publishes batch sends in `state.json`, and shows the
  listener records they write. The one thing agents write is what they did with an aside
  (acknowledged, resolved, dismissed, a reply), appended to `threads.jsonl` beside the note; the
  app watches it and shows it.
- **Measure & Annotate is an overlay on frozen screens, in its own process.** The shortcut captures
  every display (ScreenCaptureKit), and the app starts `asides-overlay`, which shows the screenshots
  in borderless GPUI windows above everything. GPUI runs its own event loop, so it can't share the
  app's (tao's) process. The session's logic (`asides-core/overlay.rs`, `measure.rs`,
  `inspect.rs`) is platform-independent; the overlay feeds it events and draws it. Permissions stay
  in the app: it hands the overlay the pixels, and names what's under a mark when asked. The result
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
    state.rs          state.json (the app → CLI protocol), recent directories, batch sends
    agents.rs         what a listening agent has seen and what's new (Cursor, Listening), listener records
    threads.rs        threads.jsonl: what agents (and the user) did with each aside; aside ids (3, 3.2)
    measure.rs        PixelSnap-style measuring on a frame: distances to edges, snapping areas and guides, objects, colors
    inspect.rs        a Measure & Annotate result (measurements, guides, marks, drawings, elements) and its Markdown
    overlay.rs        the Measure & Annotate session state machine (Measure, Mark, Draw, Shot; drags, holds,
                      guides, markers, screenshots, restore), scripts
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
    mcp.rs            `asides mcp`: the MCP server (waiting, reading, the feedback loop, measure_image; prompts)
    listen.rs         `asides listen`: deliveries on stdout, for agents watching it in the background
    measure.rs        `asides measure` and measure_image: PNG decoding and the measurement as text
  asides-overlay/  binary `asides-overlay`, shipped as Asides.app/Contents/MacOS/asides-overlay (GPUI)
    main.rs           reads the session, opens a window per screen (or plays a script without windows)
    ui.rs             Overlay (the session and around it) and ScreenView (drawing, mouse, keys): the toolbar,
                      the mark card, the context menu, the keys sheet, toasts, talking, finishing
    input.rs          the card's text field (GPUI's input example: IME, selection, clipboard)
    export.rs         the images that leave it (aside, Shot, clipboard), drawn with AppKit into a bitmap
    finish.rs         result.json: the screen images, each mark's Markdown and the screenshots, by time
    style.rs          colors and shapes shared by the windows and the exported images
    native.rs         what GPUI's windows need to be an overlay (borderless, screen-saver level, all spaces)
    io.rs             Events on stdout, Commands from stdin
    replay.rs         development: plays mouse and key steps as AppKit events and captures the windows
    assets.rs         the toolbar's icons (SVG)
  asides-app/      binary `asides-app`, renamed Asides inside the bundle
    main.rs           App: event loop, UserEvent dispatch, recording state machine, bind(), refresh(), publish()
    tray.rs           status item and menu (MenuAction, View, Directory submenu)
    icons.rs          SF Symbols rendered to RGBA for tray-icon
    hotkeys.rs        global shortcuts (Carbon hot keys); Esc registered only while recording
    audio.rs          cpal microphone capture → mono f32 at the device rate
    worker.rs         Job queue: transcription, note creation, images, audio, Wispr Flow dictations, append
    watch.rs          polling threads: Wispr Flow's history, the agents listening, the latest note's threads
    capture.rs        ScreenCaptureKit screenshots (sRGB) → frames; the Screen Recording permission; displays
    overlay.rs        starts asides-overlay with a session directory, relays its messages, reads its result
    elements.rs       what's under a mark: the window list, and the accessibility tree (AXUIElement)
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
| Listeners watcher | reads `listeners/` every 2 s, removes records of dead processes | `UserEvent::Listeners` |
| Threads watcher | checks the bound directory's latest `threads.jsonl` every second | `UserEvent::Threads` |
| ScreenCaptureKit queues | turn each display's screenshot into a frame | `UserEvent::Captured` |
| Overlay reader (while it's open) | reads `asides-overlay`'s standard output | `UserEvent::Overlay` |
| global-hotkey, muda | | `UserEvent::HotKey`, `UserEvent::Menu` |

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

1. **Start** (`start_recording`, on a shortcut press or a menu click):
   - checks the binding and the microphone permission, and starts cpal
   - snapshots the environment (browser, display) and the clipboard's change count
   - consumes any pending "new note" request
   - registers Esc and plays the start sound
2. **While recording** (`tick`, every 250 ms): updates the timer in the menu bar. With
   auto-attach on, a changed clipboard holding an image becomes a `CapturedImage` with its offset
   from the start.
3. **Stop** (`stop_recording`): stops cpal and unregisters Esc. Recordings under 0.3 s with no
   images are dropped as accidental. Anything else becomes a `Job::Recording`.
4. **Worker** (`process_recording`):
   - resamples to 16 kHz and checks for silence
   - waits for and loads the model (cached; unloaded after 15 idle minutes)
   - transcribes: language detection limited to `config.languages`, then one beam-search pass
   - picks the target note: the latest one, or a new one with metadata when asked for, or (with
     `auto_new_note`) when the latest was started on another git branch or not written to for 8
     hours (`notes::moved_on`); the CLI and the menu say so before the next recording
   - saves images and optional audio, composes the entry and appends it
5. **Outcome:** the main thread flashes `✓ #N`, re-reads the latest note and publishes the state.

### Measure & Annotate

1. **Open** (`App::measure`, on the shortcut, the menu or `asides annotate`): checks the Screen
   Recording permission (asking for it the first time), lists the displays and snapshots the
   window list (before the overlay covers it), then asks ScreenCaptureKit for one sRGB screenshot
   per display.
2. **Start the overlay** (`show_overlay`, on `UserEvent::Captured`): `overlay::start` writes each
   display's pixels (raw RGBA) and `session.json` (`helper::Spec`: the screens, the settings and
   keys, the last session's guides, whether a recording is under way) into a temporary session
   directory, and starts `asides-overlay` with it. Esc (the global cancel shortcut) is released
   while it's open, so it reaches the overlay. With `measure.record`, recording starts too.
3. **The overlay** opens one borderless window per display (GPUI, made a non-activating panel at
   screen-saver level on every space by `native.rs`) showing the screenshot, activates itself and
   says `ready`. The view turns mouse and key events into `overlay::Session` calls (in the screen's
   points) and redraws. A new mark (and, with the Mark tool, the element under a resting cursor)
   is looked up with `identify`: the app answers from `elements::identify` (the app and window from
   the window list, and with `identify_elements` and the Accessibility permission, the element from
   the app's accessibility tree, hit-testing that app so the overlay isn't hit). The talk key and the
   microphone send `talk` down and up; the app treats them like the record shortcut and tells the
   overlay when it records (`recording`), for its toolbar. Without a bound directory the spec says
   so (`bound: None`): the overlay shows it from the start and won't record or finish. Errors the
   app raises while the overlay covers the menu bar are also sent to it (`notice`).
4. **Finish** (Return, the toolbar, or the shortcut again, which the app forwards as `finish`): for
   each screen with something on it the overlay renders the screenshot cropped around what's there
   (`export.rs`, sRGB, the screen's resolution) and `inspect` writes the measurements and marks as
   Markdown. `result.json` lists them with the seconds into the session each was made, plus the
   Shot tool's screenshots. On `finished`, `App::add_inspection` reads it: in a recording, the images
   join its images and each mark becomes a text attachment at its moment
   (`markdown::Attachment.text`), so it sits among what was said; otherwise a `Job::Screen` writes
   `## N · HH:MM · Screen`, in the same order. A recording started in the overlay stops with it;
   Send also sends to the agent. `cancelled` (Esc) adds nothing. Either way the app keeps the guides
   and held measurements for the next session's R, removes the session directory when the process
   exits, and gives the focus back to the app that had it.
5. **Dev hooks:** `asides simulate-measure IMAGE SCRIPT` starts the overlay with an
   `overlay::Script` in the spec: it plays it without windows and finishes. The end-to-end test uses
   it. A `replay` in the spec plays mouse and key steps in real windows instead, as AppKit events,
   and captures them (`screencapture -l`), to check what the user sees.

### What happens to an aside (threads)

`acknowledge_aside`, `resolve_aside`, `dismiss_aside` and `reply_to_aside` check that the aside exists
in the note (`3`, or `3.2` for the mark `### 2.` of entry 3), then append one event to the note's
`threads.jsonl`: who (agent or human, and the agent's name), when, a status and/or a message. Each
event is one `write` on a file opened for appending, so concurrent writers don't interleave.
Readers fold the events into each aside's state (`threads::fold`). An annotation without events of
its own takes its entry's status, and an entry with annotations is open while any of them is
(`threads::entry_status`). The app's threads watcher reports the latest note's events. It flashes
the newest agent event in the menu bar, lists recent ones under Agent Replies, and the note line
counts the open asides. `get_asides` and `get_pending_asides` quote each aside's thread under it.

### A Wispr Flow dictation

With `wispr_flow` on, the watcher reports each dictation Wispr Flow finished since the watcher
started (`formatted` or `raw_transcript` rows of its `History` table, not archived). The main thread
checks `wispr_flow_apps` and the binding, consumes a pending "new note" request like a recording
would, and submits a `Job::Dictation`. The worker appends `## N · HH:MM · Wispr Flow` and the text,
as Wispr Flow typed it.

### Agents (`asides mcp`, `asides listen`)

1. The agent starts `asides mcp` in its working directory, which resolves to the repo like
   `asides` does. When the user asks it to listen, the `listen` prompt tells Claude Code to run
   `asides listen` with its Monitor tool, and other agents to call `wait_for_asides` in a loop.
2. Listening starts with the first wait (or with `asides listen`), as an `agents::Listening`:
   - `each`: the cursor starts at the end of the notes; only entries written from then on are new.
   - `batch`: the cursor starts after the last Send to Agent in this directory (or at the start of
     the latest note), so the next batch includes asides recorded before the agent listened.
   - Sends made before listening started are ignored: the user chose them with nobody listening.
3. If the app isn't running or is bound elsewhere, `wait_for_asides` returns at once and says so;
   the agent then calls `bind_asides`, which binds the app like `asides` does. Otherwise the server
   writes `listeners/<pid>.json` (`waiting: true` during a wait). `asides listen` writes its own,
   always waiting, and prints a notice instead.
4. Both poll every 400 ms, reading `config.toml` for `agent_delivery`:
   - `each`: any entry after the cursor, plus text added to the last delivered entry (an image
     attached after it).
   - `batch`: only once the newest of `state.json`'s `agent_sends` is newer than the last one acted
     on and names this directory; then the entries up to its note and entry number. The app
     publishes a send after Send to Agent, once queued recordings are transcribed, and only when an
     agent is listening there; otherwise it says no agent is listening and sends nothing.
5. The server returns the entries (text with absolute links, then the screenshots as images) and
   moves the cursor past them. With nothing new by the deadline (50 s, or 25 min for Claude Code,
   whose tool calls may last longer), it says so and the agent calls again. Progress notifications
   every 20 s keep the call alive in clients that time out idle calls. `asides listen` prints each
   delivery as one block; with `--timeout` it stops and prints the command that carries on, its
   position in an `--after` token.
6. When the agent hangs up (stdin closes), or `asides listen` is stopped, the listener record is
   removed.

## Invariants

Break these only on purpose, and update this document when you do.

1. **`asides-core` stays platform-independent.** No AppKit, objc2 or audio crates. Logic that
   can live there should, because that's where it can be unit-tested.
2. **AppKit, tray and menu objects are only touched on the main thread.** Background work reports
   back through `UserEvent`.
3. **Only the worker writes note files** (`notes.md`, images, audio), one job at a time, in the
   order submitted. That keeps entries ordered and numbered correctly even when recordings queue up,
   and holds for entries from Measure & Annotate too (the overlay writes only to its session
   directory). The exception is `threads.jsonl`, which `asides mcp` appends to directly: one event
   per `write`, append-only, never rewritten.
4. **Notes are append-only.** Entries are added with a single append; existing content is never
   rewritten. Editors and agents can keep the file open, and users can edit notes freely.
5. **Metadata is written once,** when a note is created. Entries carry only `## N · HH:MM`, plus
   an audio link or a source (`· Wispr Flow`) when there is one.
6. **Transcripts are verbatim.** No prompt, no vocabulary, no post-processing; in testing, a
   prompt silently dropped words. Changes here need a design discussion first. Wispr Flow entries
   are Wispr Flow's text as it typed it, marked with their source, and not changed either.
7. **The CLI ↔ app protocol is `link.rs` plus `state.rs`.** Change both sides together. New
   `state.json` and `config.toml` fields need serde defaults, so files written by older versions
   still load. `asides mcp` uses the same files, plus the listener records in `agents.rs`.
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
    with `identify_elements` on). Input Monitoring is still not needed. Anything else changes what
    users must trust, so discuss it first.
12. **`asides mcp` writes nothing to stdout but JSON-RPC.** Anything else breaks the agent's
    connection. `asides listen` writes only deliveries and notices: each burst of output is a
    notification in the agent's chat. Neither writes notes. `bind_asides` is the only tool that
    affects the app, and the feedback-loop tools only append to `threads.jsonl`. Every other tool
    stays read-only (and marked so), because agents like Codex ask before every call to a tool
    that isn't.
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

## Persistent state

| Path | Written by | Notes |
|---|---|---|
| `<dir>/.asides/…` | worker | the user's notes; excluded from git locally |
| `~/Library/Application Support/Asides/config.toml` | app (menu), user | reloaded on change; unknown fields ignored, missing ones defaulted |
| `~/Library/Application Support/Asides/state.json` | app | binding, pending new note, status, recent directories, last acknowledged request, last Send to Agent, whether the user declined the command link |
| `/usr/local/bin/asides` | app | for a copy installed by hand: a link to the CLI in the bundle, made on first launch |
| `~/Library/Application Support/Asides/models/` | download thread | `.part` while downloading, renamed after the SHA-256 check |
| `~/Library/Application Support/Asides/listeners/<pid>.json` | `asides mcp`, `asides listen` | one per agent session listening; stale ones removed by the app |
| `<dir>/.asides/<note>/threads.jsonl` | `asides mcp` | what happened to each aside since; one JSON event per line, append-only |
| `$TMPDIR/asides-overlay-<pid>-<n>/` | app, `asides-overlay` | a Measure & Annotate session: the screens' pixels, `session.json`, `result.json`, images; removed when the overlay exits |
| `~/Library/Application Support/Wispr Flow/flow.sqlite` | Wispr Flow | read-only, only with `wispr_flow` on. Dev-hooks builds read `Asides/test-wispr-flow.sqlite` instead when it exists (end-to-end test) |

## Design decisions

- **Measure & Annotate on a frozen screenshot.** PixelSnap 2 measures what's on screen when it's
  invoked. Measuring a still frame keeps the overlay simple: the frame is what's measured, drawn
  and exported. The annotated screenshot in the note matches exactly what the numbers describe,
  and nothing moves while you measure. The cost is the Screen Recording permission.
- **ScreenCaptureKit, in sRGB.** `CGDisplayCreateImage` and `CGWindowListCreateImage` are
  obsoleted in macOS 15. Capturing in sRGB means the colors Asides reports are close to the CSS
  colors that produced them, not the display profile's.
- **Edges by tolerance from the starting pixel.** Comparing each pixel with the run's first one
  (not its neighbour) stops at soft gradients' ends instead of drifting through them. The default
  (2%) is low enough to see a light gray card on white, and `T` switches to 30% for shadows.
- **GPUI, in its own process.** The overlay is the richest UI Asides has (a toolbar, cards, menus,
  live measuring at display refresh rate), and GPUI draws it on the GPU from Rust. GPUI owns its
  event loop and `NSApplication`, as tao does for the app, so the overlay is a second binary in the
  bundle, started per session and talking JSON lines. The screenshot is taken before it starts, so
  the app being tested keeps its active look in it; when the overlay exits, that app gets the focus
  back.
- **Talking is the main input.** Marks made while recording don't open a card, and each mark,
  measurement and screenshot carries the seconds into the session it was made, so it lands in the
  transcript where the user was speaking about it. Typing stays available for when talking isn't.
- **Elements from the accessibility tree, opt-in.** Agentation reads the DOM from inside the page.
  A menu-bar app can't, but browsers expose the DOM's ids and classes (`AXDOMIdentifier`,
  `AXDOMClassList`) and the page's URL through accessibility, which also covers native apps. It
  needs the Accessibility permission, so it's off until the user turns it on; without it, markers
  name the app and window from the window list.
- **For anything on screen, not just the web.** Agentation lives in the page (a React toolbar, the
  DOM, component names) and PixelSnap hands off to CleanShot and design apps. Asides keeps what
  works on any app: measuring pixels, marking elements found through accessibility, drawing,
  screenshots, and the agent's feedback loop through MCP.
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
- **SF Symbols rendered to bitmaps.** tray-icon takes RGBA; rendering system symbols keeps the
  icons native in light and dark menu bars. tray-icon caps icons at 22 pt, so they are drawn on a
  22 pt canvas at 2×.
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
