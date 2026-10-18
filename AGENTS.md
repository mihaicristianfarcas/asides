# AGENTS.md

Instructions for coding agents working in this repository. Read [ARCHITECTURE.md](ARCHITECTURE.md)
before changing the app. [CONTRIBUTING.md](CONTRIBUTING.md) has the full human workflow.

## The project

asides is a macOS menu-bar app plus a CLI.
- `asides` binds a repo.
- A global shortcut (`⌃⌥Space`) opens Asides: one mode, the screen frozen under a toolbar to
  measure and annotate it while talking, or live (▶, `⌃⌥P`), the toolbar alone over it, to talk
  while using the app.
- whisper.cpp transcribes it locally, verbatim.
- The transcript is appended to `<repo>/.asides/<YYYY-MM-DD-HHMM Title>/notes.md`.
- Coding agents can listen through `asides mcp`, an MCP server in the CLI, and get each new
  entry (or a batch, on Send to Agent) as a prompt. They acknowledge, resolve, dismiss or reply to
  asides and their annotations (Agentation's feedback loop), appended to `threads.jsonl` beside the note.
- `asides claude` (or `codex`, `opencode`, `gemini`, `copilot`, `qwen`; New Agent Session in the menu runs it in a terminal)
  starts an agent already listening, with the MCP server on its command line. Codex sessions are
  threads in Codex's app-server daemon, and OpenCode sessions live in OpenCode's background service:
  asides pushes each aside into them. Every listening agent is a session; each aside goes to one, the target (`⌃⌥Tab` switches), recorded in `routes.jsonl`
  beside the note. A second session in a repo works in a git worktree.
- Freezing (a tool or ⏸ on the toolbar, or `⌃⌥P`) freezes every screen, in any app, under the same
  toolbar: PixelSnap-style measuring (distances, snapped areas, holds, guides), numbered
  annotations on elements and areas, drawings (arrows, circles), screenshots of parts, and talking
  (Space) while doing it; ▶ or `⌃⌥P` goes back to live, and a session can freeze many times. Done
  adds it all as one aside: each freeze's screenshot, each measurement and annotation where the
  user was speaking, and what's said from an annotation's card in that annotation. The toolbar
  moves (its handle) and folds. Its overlay is a separate GPUI process, `asides-overlay`. Web-only (Agentation's toolbar, React
  names) and CleanShot-only (PixelSnap's hand-offs) features are out of scope: it works on
  anything on screen.
- With the Wispr Flow integration on, Wispr Flow dictations become entries too.

It's a Rust workspace:
- `crates/asides-core`: platform-independent logic.
- `crates/asides-cli`: the `asides` binary.
- `crates/asides-app`: the menu-bar app (binary `asides-app`, bundled as `Asides.app`).
- `crates/asides-overlay`: the overlay, toolbar and frozen screens (binary `asides-overlay`, GPUI), bundled
  beside the app's binary. Any new UI for it is GPUI.

## Commands

| Task | Command |
|---|---|
| Format | `cargo fmt --all` |
| Lint (must be clean) | `cargo clippy --workspace --all-targets --features asides-app/dev-hooks -- -D warnings` |
| Unit tests | `make test` (= `cargo test --workspace`) |
| End-to-end | `make e2e` (needs fixtures and the model; see below) |
| Overlay image | `ASIDES_OVERLAY_DUMP=<dir> cargo test -p asides-overlay renders_annotations` (writes `session.png`) |
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
- **Don't add permissions** without the maintainer's agreement. Freezing the screen uses Screen
  Recording (asked on first use) and Accessibility (only with `identify_elements`); talking over
  the live screen must keep working without either. New Agent Session uses Automation for Ghostty only (a
  tab per session; declined, a window of its own). Input Monitoring is not used.
- **Keep the docs in sync.** Update the README (user-facing), ARCHITECTURE.md (structure,
  invariants) and CONTRIBUTING.md (workflow) when your change affects them. The website
  (`docs/index.html`) repeats the shortcuts, requirements and install steps.

## What you can and can't verify

**You can:**
- run all the automated tests
- drive the installed app through the CLI with `asides simulate <wav> [--new] [--image FILE --at SECONDS]` (dev-hooks builds only; `make e2e` sets this up)
- run frozen sessions on an image with `asides simulate-measure <png> <script.json>` (`"live"` and
  `"freeze"` steps for several freezes)
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
- run `asides claude`/`codex`/`opencode`/`gemini`/`copilot`/`qwen` with a fake agent first on `PATH` to see the command line it
  builds, and check the real agent accepts it without a model call (CONTRIBUTING.md)
- run `asides listen --timeout N` and read what it prints
- render the website with headless Firefox (`--screenshot`) to check layouts at phone width

**You can't:**
- press global shortcuts
- click the status item or menus (New Agent Session, Asides Go To, Show)
- use the overlay with a real mouse and keyboard, or check that it takes the keyboard without a click
- check that the live toolbar leaves the keyboard and the clicks around it to the user's app, or
  drag it (it follows the real mouse)
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
- There's no global Esc: over the live screen, Esc (and every key) belongs to the user's app; only
  the global shortcuts reach Asides. It closes with its toolbar's × (or the menu), or Esc on a
  frozen screen (twice when there's something to lose).
- The live toolbar is its own GPUI pop-up window (a non-activating panel). `becomesKeyOnlyIfNeeded`
  keeps clicks on it from taking the keyboard from the user's app, and `hidesOnDeactivate` must be
  off: the overlay's app is never active over the live screen. Its window is sized to the toolbar,
  so the toolbar must lay out at its own size (a flex parent): stretched to the window, the two
  never shrink. GPUI doesn't implement `start_window_move` on macOS: the window follows the mouse
  from the clock (`windows::follow_toolbar`, `NSEvent.pressedMouseButtons`), which replays can't
  drive. A freeze's screenshots leave out the overlay's own windows (`capture::capture` `without`).
- On a frozen screen the overlay sets every kiosk presentation option that holds the system back
  (hide the Dock and the menu bar; no app switching, which also turns off Mission Control, App
  Exposé, Show Desktop and switching spaces, with their gestures and hot corners; no hiding the
  app; no Apple menu), but not Force Quit: a way out. An invalid combination throws (hiding the
  menu bar needs the Dock hidden, and so does disabling switching). They apply only while its app is
  active, so they're on while frozen and off live (`native::kiosk`, from `sync`). Notification
  Center, Spotlight and the screenshot keys can't be held back.
- macOS 26 takes the kiosk options from an accessory app (`currentSystemPresentationOptions` says
  so) but still switches apps with ⌘Tab; for a regular one it doesn't (tested). The overlay is a
  regular app while frozen (its Dock icon hidden with the Dock) and an accessory live. Asides'
  global shortcuts still work under them.
- `NSApp.activate()` (macOS 14's cooperative activation) was declined while the user's app was in
  front: the overlay reported itself active, but the system's presentation options stayed off and
  the frontmost app didn't change. `activateIgnoringOtherApps` (deprecated) activates it. The app
  the overlay replaces gets the keyboard back from the overlay (active, it may hand it on:
  `native::give_focus`), not from the menu-bar app, and only when the overlay has it: closing over
  the live screen once sent the user back to the app they had left.
- Handing the keyboard back is asynchronous: hidden at once, the frozen screens showed the user's
  app drawn as one in the background (gray window buttons) for a frame. They stay up until it's in
  front with the Dock and the menu bar back, and drew itself (`windows::handed_back`); closing frozen
  waits the same way. With the Dock always shown that's about half a second (it slides back in, then
  drops the overlay's icon, only once it's visible), so on ▶ the frozen screens show just the
  screenshot with the live toolbar drawn where its window is, at once, in one redraw (ordering a
  window in and drawing another didn't reach the screen in the same frame). Drawn after the overlay
  went to the background, a frozen screen's redraw never showed: it redraws while still active.
  Taking the system the moment they're ordered in, the menu bar hid a frame before they covered
  it: the kiosk options wait until they're on screen (`windows::take_over`). `screencapture -v`
  and per-frame diffs (ffmpeg, PIL) show such one-frame flashes.
- GPUI makes its app regular as it finishes launching; turned back to an accessory and made
  regular again for the kiosk moments later, the overlay got a second Dock tile that the Dock kept
  over the live screen, until it quit (accessibility lists the Dock's items). The overlay keeps
  that launch-time call from taking effect (`native::stay_accessory_at_launch`).
- ScreenCaptureKit's older screenshots (`captureImageWithFilter`) leave windows' shadows out,
  `ignoreShadowsDisplay` or not: a frozen screen was lighter under every window than the live one.
  macOS 26's (`captureScreenshotWithFilter`, `SCScreenshotConfiguration`) has them, in the
  display's color space; `capture::srgb` converts it.
- An opaque window over an app's windows occludes them (`NSWindowOcclusionState` loses
  `visible`), and many apps stop drawing while occluded: closing frozen over Diri showed its
  window blurred for a frame (a stale placeholder) before it drew again, however long the overlay
  waited, since it only draws once uncovered. The frozen screens are non-opaque
  (`WindowBackgroundAppearance::Transparent`), though the screenshot covers every pixel: the
  apps under them stay visible to macOS and keep drawing. A small window logging
  `didChangeOcclusionStateNotification` under a cover shows it.
- GPUI's pop-ups fade in and out (`NSWindowAnimationBehaviorUtilityWindow`), showing what they
  showed last while they fade: the overlay turns that off. A hidden GPUI window isn't drawn (its
  display link stops), so it shows its last frame when it's shown again (the last freeze, the
  pointer where it was): `native::draw_now` draws it first (`displayLayer:`, outside any update).
- A GPUI window opened from `cx.open_window` with `show: true` shows before its first frame. The
  overlay opens its windows hidden. With nothing else holding the overlay's entity (its windows'
  views did, when they opened at launch), the clock holds it.
- GPUI drops what AppKit tells it while it's updating (`try_borrow_mut` fails): a window resized,
  shown or made key from inside `handle.update` never told GPUI its new size or key status. The
  overlay changes its windows outside updates, by their views (`native::show`, `set_frame`).
- A shortcut's ⌃⌥ also reached the frozen screen as Measure's ⌥: its lines changed while the
  shortcut was held. With ⌃ down, Measure takes no modifiers.
- GPUI keeps a `RenderImage` in its atlas until the window drops it (`Window::drop_image`): each
  freeze's screenshots are dropped once the next are drawn.
- Whisper writes in the language it detects: English said with an accent, in a short clip, came out
  in Romanian. Unsure detections keep the last sure language (`stt::choose_language`).
- GPUI can't show, hide or move a window from inside an event it's handling for that window: the
  overlay changes its windows from the clock (`windows::sync`), after the event. The first time it
  went live, opening the toolbar's window marked the toolbar shown, so the frozen screens never hid
  (and covered it) until the next freeze: toggles seemed to do nothing.
- The overlay's panels are non-activating, so the frontmost app keeps its look. Borderless panels
  need `canBecomeKeyWindow` to take the keyboard, and mouse moves come from an `ActiveAlways`
  tracking area.
- An accessibility hit test on the system-wide element finds the overlay itself. Hit-test the app
  that owns the window under the point (from the window list taken before the overlay opened).
- GPUI owns its `NSApplication` and event loop, like tao: the two can't share a process, hence
  `asides-overlay`. GPUI also makes its app a regular one (a Dock icon); `native::accessory` turns
  that back right away (frozen, `native::kiosk` makes it regular again).
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
  `~/Applications`, and Launch Services, left to pick, opened the release for the CLI's links (both
  ended up running, fighting over the shortcuts: the release's frozen-only overlay showed). The CLI
  now sends its links to the app it ships in (`own_app`), or to `ASIDES_APP` (`open -a`; `make e2e`
  sets it). Opened from Spotlight, either copy may still start: quit the one not under test.

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
- `codex queue` refuses images ("does not support image attachments", though `--help` lists
  `--image`): every aside with a screenshot failed, retried every 400 ms. A `turn/start` with
  `localImage` inputs takes them, but on an active thread it joins the turn in progress (steers):
  `thread/read`'s `status.type` says `idle` or `active`. The daemon lets a thread go once no client
  has it open, and `thread/read` then fails ("thread not loaded"): `thread/resume` (with its
  `config`) loads it again. `codex app-server generate-json-schema --out DIR` writes the protocol.
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
- Gemini CLI reads MCP servers only from settings files, and skips a system settings file
  (`GEMINI_CLI_SYSTEM_SETTINGS_PATH`, or the defaults one) unless root owns its folder: a session
  can't have a server of its own. It passes its environment to its servers (plus `GEMINI_CLI=1`),
  so the user's `asides mcp` becomes the session's through `ASIDES_SESSION`. In a folder the user
  hasn't trusted, its MCP servers are disabled (`gemini mcp list` says Disabled;
  `GEMINI_CLI_TRUST_WORKSPACE=true` trusts it for a check).
- Copilot CLI's `--allow-tool` takes every value after it unless given with `=`, and its `mcp list`
  doesn't take `--additional-mcp-config`. It validates that config before asking for a login, so
  `-p` in place of `--interactive` checks a command line without a model call. Qwen Code (from
  Gemini CLI) takes servers with `--mcp-config` and says "MCP server(s) failed to start" under `-p`
  when one doesn't.
- The agent CLIs write into your home on their first run, even for `--version` (Copilot unpacks
  170 MB into `~/Library/Caches/copilot`; Gemini makes `~/.gemini`): install them into a scratch
  prefix (`npm install --prefix`) and run them with a scratch `HOME`.
- A freeze is in sRGB, but the display's colors are Display P3: a saturated blue (a sky wallpaper)
  is outside sRGB, its red clipped near 0, where one level of the screen's is 3 to 6 of sRGB's. Those
  jumps were edges (phantom stops on an empty sky, objects in a blur behind glass) until edges had to
  be visible (OKLab). `screencapture` saves the display's colors, so it doesn't show this: test
  measuring on what a freeze captures (`dumps_a_freeze` in `capture.rs`), or convert with `sips
  --matchTo '/System/Library/ColorSync/Profiles/sRGB Profile.icc'`.
- Whisper's segment times run through silence: two takes joined with a 2 s pause came back as
  segments with no gap between them, or as one. Keep each take's segments in it (`fit_segments`).
- On a notched display, the notch sits in the menu bar (the screen's safe area, 38 pt; the menu bar
  is 39). A banner centered at the top of a frozen screen was half under it.
- `NSRunningApplication.hide()` from a command-line tool returns false: it can't hide other apps.
- `dirijor new` ignores `--help` and creates a Diri session. Read its source (or `dirijor session
  run --help`, which does print help) before trying its commands.

## Commits

Imperative subjects ("Add …", "Fix …"), a body that explains why, and each commit building and
passing tests on its own. Commit only when the user asks.
