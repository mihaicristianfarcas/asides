# AGENTS.md

Instructions for coding agents working in this repository. Read [ARCHITECTURE.md](ARCHITECTURE.md)
before changing the app. [CONTRIBUTING.md](CONTRIBUTING.md) has the full human workflow.

## The project

asides is a macOS menu-bar app plus a CLI.
- `asides` binds a repo.
- A global shortcut records speech.
- whisper.cpp transcribes it locally, verbatim.
- The transcript is appended to `<repo>/.asides/<YYYY-MM-DD-HHMM Title>/notes.md`.
- Coding agents can listen through `asides mcp`, an MCP server in the CLI, and get each new
  entry (or a batch, on Send to Agent) as a prompt. They acknowledge, resolve, dismiss or reply to
  asides and their marks (Agentation's feedback loop), appended to `threads.jsonl` beside the note.
- `asides claude` (or `codex`, `opencode`; New Agent Session in the menu runs it in a terminal)
  starts an agent already listening, with the MCP server on its command line. Codex sessions are
  threads in Codex's app-server daemon, and OpenCode sessions live in OpenCode's background service:
  asides pushes each aside into them. Every listening agent is a session; each aside goes to one, the target (`⌃⌥Tab` switches), recorded in `routes.jsonl`
  beside the note. A second session in a repo works in a git worktree.
- Measure & Annotate (`⌃⌥M`) freezes every screen, in any app, under one toolbar: PixelSnap-style
  measuring (distances, snapped areas, holds, guides), numbered marks on elements and areas,
  drawings (arrows, circles), screenshots of parts, and talking (Space) while doing it. The
  annotated screenshot and Markdown land in the aside, each mark where the user was speaking. Its
  overlay is a separate GPUI process, `asides-overlay`. Web-only (Agentation's toolbar, React
  names) and CleanShot-only (PixelSnap's hand-offs) features are out of scope: it works on
  anything on screen.
- With the Wispr Flow integration on, Wispr Flow dictations become entries too.

It's a Rust workspace:
- `crates/asides-core`: platform-independent logic.
- `crates/asides-cli`: the `asides` binary.
- `crates/asides-app`: the menu-bar app (binary `asides-app`, bundled as `Asides.app`).
- `crates/asides-overlay`: the Measure & Annotate overlay (binary `asides-overlay`, GPUI), bundled
  beside the app's binary. Any new UI for it is GPUI.

## Commands

| Task | Command |
|---|---|
| Format | `cargo fmt --all` |
| Lint (must be clean) | `cargo clippy --workspace --all-targets --features asides-app/dev-hooks -- -D warnings` |
| Unit tests | `make test` (= `cargo test --workspace`) |
| End-to-end | `make e2e` (needs fixtures and the model; see below) |
| Overlay image | `ASIDES_OVERLAY_DUMP=<dir> cargo test -p asides-overlay renders_marks` (writes `session.png`) |
| Overlay windows | `target/debug/asides-overlay <session dir>` with `windowed` and `replay` (see CONTRIBUTING.md) |
| Speech tests | `make test-speech` (needs the model; also downloads 190 MB) |
| Install | `make install` |
| Test fixtures | `make fixtures` (needs the macOS voice "Ioana") |
| Share image | `python3 scripts/make-og.py` (renders `docs/og.png` from the demo; needs Firefox) |
| App icon | `swift scripts/make-icon.swift` (writes `packaging/Asides.icns`) |
| Notarized release | `make release` (only when the maintainer asks; see Rules) |
| Deploy the website | pushes to `main` deploy it; `make publish-site` by hand (only when the maintainer asks) |

Run format, lint and unit tests after every change. Run `make e2e` when CLI or app behavior
changes.

## Rules

- **Follow the invariants in ARCHITECTURE.md.** The ones most often at risk:
  - `asides-core` has no macOS dependencies
  - AppKit, tray and menu code runs on the main thread only
  - only the worker thread writes notes
  - notes are append-only
  - transcripts are verbatim: no prompts, no post-processing
  - new `config.toml` and `state.json` fields get serde defaults
- **Never commit** `.asides/` (the user's notes and voice recordings), `tests/fixtures/`
  (generated with macOS voices, which may not be redistributed), `target/` or `dist/`. They're
  gitignored; keep them that way.
- **Never edit the user's notes** in `.asides/`.
- **Run `make release` only when the maintainer asks.** It uploads the build to Apple's notary
  service and needs their stored notarytool credentials.
- **Don't run `make cert`** unless asked. It changes the login keychain and prompts for a password.
- **Keep the stable signing.** `make bundle` picks the identity with `scripts/signing-identity.sh`
  and signs with the hardened runtime. Don't fall back to ad hoc (the microphone permission would
  reset every build), and add an entitlement for any new capability.
- **Check `asides status` before `make install` or `make e2e`.** If it says recording or
  transcribing, the user is in the middle of something: don't reinstall.
- **Tell the user what `make install` and `make e2e` did.** Both quit the running Asides and
  replace the installed app, and `make e2e` installs a test build with dev hooks. Afterwards run
  `make install` to restore a normal build, and tell the user to run `asides` again.
- **Know what `make e2e` uses.** It relies on the model in
  `~/Library/Application Support/Asides/models/`; if it's missing, the app downloads 574 MB.
  It also rebinds the app to a temporary repo.
- **Keep binary names unique ignoring case** (case-insensitive APFS). Don't name a binary
  `Asides`.
- **Don't add permissions** without the maintainer's agreement. Measure & Annotate uses Screen
  Recording (asked on first use) and Accessibility (only with `identify_elements`); recording
  itself must keep working without either. New Agent Session uses Automation for Ghostty only (a
  tab per session; declined, a window of its own). Input Monitoring is not used.
- **Keep the docs in sync.** Update the README (user-facing), ARCHITECTURE.md (structure,
  invariants) and CONTRIBUTING.md (workflow) when your change affects them. The website
  (`docs/index.html`) repeats the shortcuts, requirements and install steps.

## What you can and can't verify

**You can:**
- run all the automated tests
- drive the installed app through the CLI with `asides simulate <wav> [--new] [--image FILE --at SECONDS]` (dev-hooks builds only; `make e2e` sets this up)
- run Measure & Annotate sessions on an image with `asides simulate-measure <png> <script.json>`
  (dev-hooks builds), and look at the image it adds with `ASIDES_OVERLAY_DUMP`
- run the real overlay windows on a screenshot in a small window, play mouse and key steps in them
  (`replay`: AppKit events into GPUI's view), and capture them to PNGs to look at
  (`screencapture -l`, when your terminal has Screen Recording). Keep the window small and the run
  short: it's on the user's screen
- measure screenshots with `asides measure <png> X,Y` (or `X,Y,W,H`)
- render the menu-bar icons to inspect them: `ASIDES_ICON_DUMP=<dir> cargo test -p asides-app icons` writes RGBA dumps you can convert to PNG
- read `~/Library/Application Support/Asides/state.json`
- drive `asides mcp` with JSON-RPC on stdin (see CONTRIBUTING.md), and run Claude Code, Codex or
  OpenCode against it with a one-off MCP config, using `asides simulate` for speech
- run `asides claude`/`codex`/`opencode` with a fake agent first on `PATH` to see the command line it
  builds, and check the real agent accepts it without a model call (CONTRIBUTING.md)
- run `asides listen --timeout N` and read what it prints
- render the website with headless Firefox (`--screenshot`) to check layouts at phone width

**You can't:**
- press global shortcuts
- click the status item or menus (New Agent Session, Asides Go To, Show)
- use the overlay with a real mouse and keyboard, or check that it takes the keyboard without a click
- speak into the microphone
- grant permissions (Microphone, Screen Recording, Accessibility)

When a change touches those, finish by listing the exact manual checks for the user to run. The
manual QA checklist in CONTRIBUTING.md has the full list.

## Working from asides notes

The user may record a request and ask you to "implement the note". Find it with
`asides path` (or `asides -C <repo> path`): it's the latest `notes.md` under `.asides/`.

- Entries (`## N · HH:MM`) are raw Whisper output, in English or Romanian. Expect misheard words,
  especially project names (for example "Local Tatus" for "local-tts").
- Early entries may be microphone tests. Act on the entries that ask for something, and say which
  ones you implemented.
- Images referenced in an entry (`images/…`) are screenshots the user attached at that point.
  Look at them.
- The front matter records the branch and commit the user was testing.
- Also report defects the note itself reveals, such as wrong metadata or a failed audio conversion.

## Pitfalls already hit

- `target/release/asides` and `target/release/Asides` are the same file on case-insensitive
  APFS. The bundle once shipped the CLI as the app.
- `afconvert` rejects AAC above 32 kbps for 16 kHz mono (`-b 32000` works).
- `git rev-parse --abbrev-ref HEAD` fails before the first commit; use `git branch --show-current`.
- `ditto -c -k` zips don't flag UTF-8 names, so diacritics get garbled. Export uses the `zip` crate.
- muda drops a single `&` from menu titles (mnemonics); escape dynamic text with `menu_text`.
- tray-icon scales icons to at most 22 pt tall; `icons.rs` draws on a 22 pt canvas at 2×.
- `if let` keeps its scrutinee's temporaries (including a `MutexGuard`) alive for the whole block,
  so locking the same mutex inside it deadlocks. Bind the value first.
- An `Esc` key equivalent on a menu item would cancel the recording when Esc closes the menu, so
  the shortcut is shown in the item's label instead.
- `say -o x.aiff --data-format=LEI16@16000` fails; use `--file-format=WAVE`.
- tao activates the app at launch unless `set_activate_ignoring_other_apps(false)`; a menu-bar app
  must not take focus.
- Changing the status item's icon or title, or a menu item's text, while the menu is open does
  **not** close it (tested). Look elsewhere if the menu closes unexpectedly: focus or activation
  changes, or system prompts.
- Clicking a standard menu item always closes the menu. Switch rows (view-based items) stay open,
  but they were tried for the preferences and reverted: they don't line up with native items.
- Without a `TrayIconEvent` handler, tray-icon queues an event for every mouse move over the icon,
  forever.
- Apple's SpeechAnalyzer and FoundationModels are Swift-only, so Rust can't call them without a
  Swift shim.
- Codex (and clients built on the MCP SDKs' defaults) abandon a tool call after 60 s, so
  `wait_for_asides` returns within 50 s unless the client is Claude Code. Claude Code allows hours,
  but cuts off a stdio call that sends nothing for 30 min; progress notifications keep it alive.
- Codex asks for approval before each call to an MCP tool not marked `readOnlyHint` (and fails
  it under `approval: never`), so a listening loop needs read-only tools. That's why binding is a
  separate tool, `bind_asides`.
- Claude Code's Monitor tool kills a command after at most 30 min, so the listen prompt runs
  `asides listen --timeout 1740`, which prints the command to carry on (with an `--after` token)
  before that happens. Monitor events aren't replies from the user; the prompt says asides are.
- Claude Code probes `server/discover` (a newer MCP revision) before `initialize`. Answering
  "method not found" makes it fall back to `initialize`.
- Wispr Flow's `flow.sqlite` schema changes often between its versions. Discover columns, don't
  assume them, and don't open it any way but read-only: Wispr Flow writes to it in WAL mode.
- On the website, a `padding` shorthand on `.wrap` inside a media query reset the sections' top
  padding on phones. Set only `padding-inline` there.
- Headless Firefox takes `--screenshot` at load, mid-transition. Turn transitions off to capture a
  frame (as `scripts/make-og.py` does).
- Drawing a label's text with `NSString::drawAtPoint_withAttributes` changes the current stroke
  color: set it again before the next line.
- In `define_class!`, a protocol method declared `-> bool` is turned into `Bool`: an early `return
  false` doesn't compile. Move the body into a plain function.
- `CGBitmapContextCreate` isn't in objc2-core-graphics; read a `CGImage`'s pixels from its data
  provider (`capture::to_frame`, which handles every 8-bit byte order).
- The global Esc (cancel recording) would steal Esc from the overlay: it's released while the
  overlay is open and restored after.
- The overlay's panels are non-activating, so the frontmost app keeps its look. Borderless panels
  need `canBecomeKeyWindow` to take the keyboard, and mouse moves come from an `ActiveAlways`
  tracking area.
- An accessibility hit test on the system-wide element finds the overlay itself. Hit-test the app
  that owns the window under the point (from the window list taken before the overlay opened).
- GPUI owns its `NSApplication` and event loop, like tao: the two can't share a process, hence
  `asides-overlay`. GPUI also makes its app a regular one (a Dock icon); `native::accessory` turns
  that back right away.
- GPUI handles an input event by updating its window, so an event sent while that window is being
  updated (from inside `handle.update`, or a listener) is dropped with a logged error. The replay
  sends its AppKit events from its task, between updates. Likewise, focus the card's field with the
  `window` in hand, not through the window handle.
- AppKit events made with `NSEvent::mouseEventWithType…` have no button number, so GPUI reads every
  one as the left button: the replay sends a right-click as a control-click.
- GPUI reads keys through the current keyboard layout from the key code; synthetic key events need
  the US key codes (`native::synthetic`), and shifted symbols arrive as the symbol (`?`, not `⇧/`).
- `Window::dispatch_event` returns a private type, so it can't be called from outside GPUI.
- Changing an `NSWindow`'s style mask (`native::make_overlay` makes GPUI's panel borderless) rebuilds
  its frame and resets its first responder to the window. Keys still reach GPUI, but typed text goes
  through the first responder's input context, so the card took no text. Make GPUI's view the first
  responder again after the change. Replays sent events straight to the view and didn't show it.
- Posting key events to check this: a `CGEvent` made without a private source inherits the Fn flag
  from an earlier function key, and Fn+A is macOS's "focus the Dock". Use
  `CGEventSource(stateID: .privateState)` and set the flags. With `asides-overlay-debug` in the
  temporary directory, the overlay logs the keys AppKit and GPUI get, and focus changes, to
  `asides-overlay-debug.log` there (`native::debug`).
- The overlay covers the menu bar, so an error shown there while it's open goes unseen: `App::fail`
  also sends it to the overlay (`notice`).
- A light gray card on white (#F3F4F6) differs by 12 of 255: the default tolerance (2%) must stay
  below that.
- A release in `/Applications/Asides.app` has the same bundle ID as the build `make install` puts in
  `~/Applications`, and Launch Services may open it for the CLI's links instead (both may end up
  running). Set `ASIDES_APP=~/Applications/Asides.app` to make the CLI send its links to that copy
  (`open -a`); `make e2e` does.

- `claude`'s `--mcp-config` and `--allowedTools` take every value after them, so the prompt goes
  first; after them it was read as a config file. A server given with `--mcp-config` replaces a
  user-scope one of the same name (`"source":"dynamic"` in the init message).
- OpenCode 2 runs agents and their MCP servers in a shared background service (`opencode serve
  --service`), which doesn't see a TUI's environment: `OPENCODE_CONFIG_CONTENT` needs
  `--standalone`, and `opencode debug config` and `mcp list` ask the shared service, not the
  process you run. 2.0 keeps servers under `mcp.servers.<name>`; 1.x under `mcp.<name>` and has no
  `--standalone`. That service also starts `asides mcp` for every directory it's used in: those
  records aren't listening, and the menu ignores them.
- The app is launched with Launch Services' bare `PATH`, so agents are found and run through the
  user's login shell (`$SHELL -ilc`); `.command` scripts opened in a terminal get its login shell.
- Ghostty can't be started from its CLI on macOS (`+new-window` is unsupported), and asks "Allow
  Ghostty to execute …?" before running a script it's asked to open (`open -a Ghostty x.command`).
  New Agent Session starts it with `open -na Ghostty --args --initial-command=<script>` instead: a
  process of its own, no prompt. Pass the script as an option, not a bare path (`-e <path>`):
  AppKit also opens a bare path argument as a document, which brings the prompt back.
- Claude Code asks whether to trust a folder the first time it runs there, launched by asides or
  not. A scratch repo for a live test stalls on it; your own repos are already trusted.
- macOS has no `timeout` command; scripts wait with a loop and `kill`.
- Codex's app-server daemon speaks JSON-RPC over WebSocket; `codex app-server proxy` connects stdio
  to its socket, so a client sends the HTTP upgrade first and masked frames after. Threads started
  by `codex exec` run in that process, not the daemon: `codex queue` messages for them wait.
- A Codex window given `-c` options runs a server of its own, which a thread the daemon holds is
  locked against ("This conversation is open in another app"). `codex resume <thread>` for a
  thread asides started passes no `-c`; the thread already has its configuration.
- OpenCode's free models refuse `opencode run` ("can only be used from within OpenCode"), but not
  prompts sent to its background service, which is what the TUI does: asides starts and pushes
  into sessions there. The service registers its address and password in
  `~/.local/state/opencode/service.json` (HTTP basic auth, user `opencode`); the API is under
  `/api/`, takes the directory as `location[directory]=<dir>`, and `GET /api/mcp` only lists the
  servers of directories already loaded. `delivery: "queue"` waits for the turn in progress;
  `"steer"` would join it.
- Claude Code's `--worktree` branches from origin's default branch (`worktree.baseRef = "fresh"`),
  not from the branch the user is on; `--settings '{"worktree":{"baseRef":"head"}}'` changes that
  for one session.
- `claude stop` takes a background session's id (`claude agents --json` gives it for the name);
  `claude attach` takes either.
- Agents mark the processes they run as part of their session (`CLAUDECODE`,
  `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`, `CLAUDE_EFFORT`, `OPENCODE`, `AGENT`…), and
  `open` passes its caller's environment to the app it starts. An app started by an agent's `asides
  mcp` handed them to every terminal and agent it started: Claude Code warned "Transcript saving is
  off — inherited CLAUDE_CODE_CHILD_SESSION marker" and ran at the first session's effort. The app
  and `asides <agent>` drop them (`launch::session_markers`). `ps eww` shows the environment a
  process was started with, not what it removed since: check a child's.
- Terminal's AppleScript can't make a tab (only ⌘T through System Events, which needs
  Accessibility), and "Prefer tabs when opening documents" doesn't apply to scripts it's asked to
  open (tested). Ghostty 1.3's dictionary has `new tab … with configuration`; `osacompile` checks a
  script against it without sending Ghostty anything.
- The Automation prompt holds the AppleScript until the user answers, and the script fails after
  the Apple event timeout (2 minutes) if they don't: run it off the main thread. Under the hardened
  runtime it needs the `automation.apple-events` entitlement, and the prompt shows
  `NSAppleEventsUsageDescription`.
- With the Accessibility permission (your terminal may have it: `AXIsProcessTrusted`), the real
  status menu can be driven: a synthetic click at the frame of the app's `AXExtrasMenuBar` child
  opens it (`AXPress` doesn't open tray-icon's menu), then its items read and press through
  accessibility. Shortcuts need the whole chord posted (modifier key events, then the key, through
  a private `CGEventSource`); a key event carrying only modifier flags doesn't fire a Carbon hot
  key. It's the user's screen and mouse: keep it short.
- `dirijor new` ignores `--help` and creates a Diri session. Read its source (or `dirijor session
  run --help`, which does print help) before trying its commands.

## Commits

Imperative subjects ("Add …", "Fix …"), a body that explains why, and each commit building and
passing tests on its own. Commit only when the user asks.
