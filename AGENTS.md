# AGENTS.md

Instructions for coding agents working in this repository. Read [ARCHITECTURE.md](ARCHITECTURE.md)
before changing the app; [CONTRIBUTING.md](CONTRIBUTING.md) has the workflow, and
[guide/using.md](guide/using.md) what users see.

## The project

asides is a macOS menu-bar app plus a CLI. `⌃⌥Space` freezes every screen under a pill; the user
pins elements, measures, draws and talks; `⌃⌥Space` again types it all, transcribed locally and
verbatim by whisper.cpp, into a coding agent's session in Terminal, Ghostty or Diri, as the user's
own message with screenshots. `⌃⌥P` goes live and freezes again (one aside, many freezes). Every
aside is kept in `~/.asides/history`. It works on anything on screen: web-only and CleanShot-only
features are out of scope. Why it works this way: ARCHITECTURE.md's design decisions.

A Rust workspace:
- `crates/asides-core`: platform-independent logic.
- `crates/asides-cli`: the `asides` binary.
- `crates/asides-app`: the menu-bar app (binary `asides-app`, bundled as `asides.app`).
- `crates/asides-overlay`: the overlay, the pill, the frozen screens and the asides window (binary
  `asides-overlay`, GPUI), bundled beside the app's binary. Any new UI is GPUI.

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
| README picture | `python3 scripts/make-screenshot.py` (the real overlay on the demo, `guide/overlay.png`; shows a window for a few seconds) |
| App icon | `swift scripts/make-icon.swift` (writes `packaging/asides.icns`) |
| Notarized release | `make release` (only when the maintainer asks; see Rules) |
| Deploy the website | pushes to `main` deploy it; `make publish-site` by hand (only when the maintainer asks) |

Run format, lint and unit tests after every change. Run `make e2e` when CLI or app behavior
changes.

## Rules

- **Never poll.** Wait on events (input, AppKit and NSWorkspace notifications, FSEvents through
  `asides_core::watch`, kqueue, a child ending, a channel) or sleep until the exact deadline. A
  timer only where nothing says, with a comment saying why.
- **Follow the invariants in ARCHITECTURE.md.** The ones most often at risk:
  - `asides-core` has no macOS dependencies
  - AppKit, tray and menu code runs on the main thread only
  - only the worker thread creates asides in the history
  - an aside is typed in as the user would: nothing installed in the agents, nothing listening
  - never into a session the user didn't choose (tests send `--to` a session of their own)
  - transcripts are verbatim: no prompts, no post-processing
- **Never commit** `.asides/` (old notes and voice recordings), `tests/fixtures/`
  (generated with macOS voices, which may not be redistributed), `target/` or `dist/`. They're
  gitignored; keep them that way.
- **Never edit the user's history** in `~/.asides` (or old notes in a repo's `.asides/`). Tests use
  a history of their own (`ASIDES_HOME`; the app gets it with `launchctl setenv`, unset after).
- **Never send to the user's own agent sessions** while testing: start sessions of your own in a
  scratch repo and send `--to` them.
- **Run `make release` only when the maintainer asks.** It uploads the build to Apple's notary
  service and needs their stored notarytool credentials.
- **Don't run `make cert`** unless asked. It changes the login keychain and prompts for a password.
- **Keep the stable signing.** `make bundle` picks the identity with `scripts/signing-identity.sh`
  and signs with the hardened runtime. Don't fall back to ad hoc (the microphone permission would
  reset every build), and add an entitlement for any new capability.
- **Check `asides status` before `make install` or `make e2e`.** If it says open, recording,
  transcribing or sending, the user is in the middle of something: don't reinstall.
- **Tell the user what `make install` and `make e2e` did.** Both quit the running asides and
  replace the installed app, and `make e2e` installs a test build with dev hooks. Afterwards run
  `make install` to restore a normal build, and tell the user to run `asides` again.
- **Know what `make e2e` uses.** It sets the model to large-v3-turbo (the fixtures are English and
  Romanian), which must be in `~/Library/Application Support/asides/models/` (`asides download
  large-v3-turbo`). It keeps its asides in a temporary history, sends only to a stand-in agent of
  its own (in Diri, or Terminal without Diri), and puts back the user's settings and clipboard.
- **Keep binary names unique ignoring case** (case-insensitive APFS): the CLI is `asides`, the
  app's `asides-app`. `pkill -x asides` would also stop the agents' `asides mcp` servers.
- **Don't add permissions** without the maintainer's agreement. Freezing the screen uses Screen
  Recording (asked on first use) and Accessibility (with `identify_elements`, on by default; pins
  work without it); talking over the live screen must keep working without either. Sending uses
  Automation for Terminal and Ghostty (asked the first time asides types into or opens a session
  there); Diri needs none. Input Monitoring is not used.
- **Keep the docs in sync, and short.** User-facing behavior goes in `guide/` (the README is only a
  pitch, install and links), structure and invariants in ARCHITECTURE.md, workflow in
  CONTRIBUTING.md. The website (`docs/index.html`) repeats the shortcuts, requirements and install
  steps. When the overlay's look changes, regenerate `guide/overlay.png`.

## What you can and can't verify

**You can:**
- run all the automated tests
- drive the installed app through the CLI with `asides simulate <wav> [--image FILE --at SECONDS] [--to SESSION]` (dev-hooks builds only; `make e2e` sets this up)
- run frozen sessions on an image with `asides simulate-measure <png> <script.json> [--to SESSION]`
  (`"live"` and `"freeze"` steps for several freezes) (dev-hooks builds), and look at the image it
  adds with `ASIDES_OVERLAY_DUMP`
- list the sessions (`asides sessions`), start agents of your own in a scratch repo (Diri's
  `dirijor session spawn`, or Terminal and Ghostty through AppleScript from your terminal) and send
  them asides (`--to`), reading what each agent recorded (Claude Code's transcript in
  `~/.claude/projects`, Codex's rollout in `~/.codex/sessions`, OpenCode's database, read-only)
- run the real overlay windows on a screenshot in a small window, play mouse and key steps in them
  (`replay`: AppKit events into GPUI's view), and capture them to PNGs to look at
  (`screencapture -l`, when your terminal has Screen Recording). Keep the window small and the run
  short: it's on the user's screen
- open the asides window on a history of your own and capture each part (CONTRIBUTING.md)
- measure screenshots with `asides measure <png> X,Y` (or `X,Y,W,H`)
- render the menu-bar icons to inspect them: `ASIDES_ICON_DUMP=<dir> cargo test -p asides-app icons` writes RGBA dumps you can convert to PNG
- read `~/Library/Application Support/asides/state.json`
- drive `asides mcp` with JSON-RPC on stdin (see CONTRIBUTING.md)
- render the website with headless Firefox (`--screenshot`) to check layouts at phone width

**You can't:**
- press global shortcuts
- click the status item or menus
- use the overlay with a real mouse and keyboard, or check that it takes the keyboard without a click
- check that the live pill leaves the keyboard and the clicks around it to the user's app, or
  drag it (it follows the real mouse)
- speak into the microphone
- grant permissions (Microphone, Screen Recording, Accessibility, Automation): an Automation prompt
  holds an AppleScript for up to 2 minutes, then it fails

When a change touches those, finish by listing the exact manual checks for the user to run. The
manual QA checklist in CONTRIBUTING.md has the full list.

## Working from asides

The user's asides arrive in your chat as their messages. To read one again (with its screenshots'
paths), run `asides show` (the newest) or `asides history`, or use `asides mcp`'s `get_aside`.

- What was said is raw Whisper output, in English or Romanian. Expect misheard words, especially
  project names (for example "Local Tatus" for "local-tts").
- Each screenshot is where the user took it (`[Image #1]`, or a path into `~/.asides/history`).
  Look at them.
- `### N.` blocks are pins: the element (role and label), the app and window, the page and a
  CSS-like location, its size, what the user typed or said about it.
- Also report defects the aside itself reveals, such as a misplaced screenshot or a wrong element.

## Pitfalls already hit

### Tools and build

- `target/release/asides` and `Asides` are the same file on case-insensitive APFS (the bundle once
  shipped the CLI as the app).
- `afconvert`: AAC above 32 kbps fails for 16 kHz mono (`-b 32000`). `say -o x.aiff
  --data-format=LEI16@16000` fails: use `--file-format=WAVE`.
- `git rev-parse --abbrev-ref HEAD` fails before the first commit: `git branch --show-current`.
- `ditto -c -k` garbles diacritics in zip names: export uses the `zip` crate.
- The updater refuses a release zip holding symlinks (`check_zip`): a framework in the bundle
  (`Versions/Current`) would need that changed first.
- macOS has no `timeout` (wait with a loop and `kill`), and its `grep` has no `\|` (`grep -E`).
- `pgrep -f "<text>"` also matches the shell whose wait loop contains the text.
- `launchctl setenv` reaches every app launched afterwards: `launchctl unsetenv` when done.
- `dirijor new` ignores `--help` and creates a session: read its source (or `dirijor session run
  --help`) before trying commands.
- The agent CLIs write into your home on first run (Copilot unpacks 170 MB): install them with
  `npm install --prefix` and run them with a scratch `HOME`.
- Headless Firefox screenshots mid-transition: turn transitions off. On the website, a `padding`
  shorthand on `.wrap` in a media query reset the sections' top padding: set `padding-inline`.

### Rust, AppKit, menus

- `if let` keeps its scrutinee's temporaries (a `MutexGuard`) alive for the whole block: bind the
  value first, or locking again deadlocks.
- `define_class!` turns a protocol method's `-> bool` into `Bool`, so an early `return false`
  doesn't compile: move the body into a plain function.
- `CGBitmapContextCreate` isn't in objc2-core-graphics: read a `CGImage` from its data provider
  (`capture::to_frame`).
- `NSString::drawAtPoint_withAttributes` changes the stroke color: set it again after.
- SpeechAnalyzer and FoundationModels are Swift-only. `NSRunningApplication.hide()` from a CLI
  returns false.
- muda drops a single `&` from menu titles: escape dynamic text with `menu_text`.
- tray-icon caps icons at 22 pt (`icons.rs` draws on 22 pt at 2×), and without a `TrayIconEvent`
  handler it queues an event per mouse move, forever.
- tao activates the app at launch unless `set_activate_ignoring_other_apps(false)`.
- Changing the status item or a menu item's text doesn't close an open menu; if it closes, look at
  focus, activation or system prompts. Standard items always close it; view-based switch rows
  don't, but didn't line up (reverted).
- Launch Services gives the app a bare `PATH`: agents are found and run through `$SHELL -ilc`.
- A release in `/Applications` has the same bundle ID as `make install`'s build. The CLI opens its
  own app (`own_app`) or `ASIDES_APP`; from Spotlight either may start: quit the one not under test.
- Agents mark their processes (`CLAUDECODE`, `CLAUDE_CODE_SESSION_ID`, `CLAUDE_CODE_CHILD_SESSION`,
  `CLAUDE_EFFORT`, `OPENCODE`, `AGENT`…) and `open` passes them to the app it starts, which then
  passed them to every terminal (Claude Code: "Transcript saving is off"). The app drops them
  (`agents::session_markers`). `ps eww` shows a process's starting environment: check a child's.

### GPUI

- GPUI owns `NSApplication` and the event loop, like tao: hence a second process. It makes its app
  regular (a Dock icon): `native::accessory` undoes it, and `native::stay_accessory_at_launch` stops
  its launch-time call (else a second Dock tile stayed until quit). The asides window's process
  stays regular (Dock, ⌘Tab): it quits with its window.
- GPUI drops what AppKit tells it during an update: a window resized, shown or made key inside
  `handle.update` never told GPUI. Change windows outside updates, by their views (`native::show`,
  `set_frame`).
- GPUI can't show, hide or move a window from inside that window's event: the overlay does it from
  the clock (`windows::sync`).
- An input event sent while its window updates is dropped: replays send between updates; focus the
  card's field with the `window` in hand.
- `cx.open_window` with `show: true` shows before the first frame: open hidden. When nothing else
  holds the overlay's entity, the clock does.
- A hidden window isn't drawn and shows its last frame when shown: `native::draw_now` draws first.
  Pop-ups fade showing old content: the overlay turns the animation off.
- `RenderImage`s stay in the atlas until `Window::drop_image`: drop a freeze's once the next is drawn.
- No `start_window_move` on macOS: the pill follows the mouse from the clock
  (`windows::follow_toolbar`), which replays can't drive. `Window::dispatch_event` returns a private
  type.
- Changing a window's style mask (`native::make_overlay`) resets the first responder: keys arrive
  but typed text doesn't. Make GPUI's view first responder again (replays didn't show it).
- Synthetic events: `mouseEventWithType…` has no button number (every click is left; send a
  right-click as control-click); keys need US key codes (`native::synthetic`), and shifted symbols
  arrive as the symbol (`?`).
- Posted `CGEvent`s inherit Fn from an earlier function key (Fn+A focuses the Dock): use a private
  `CGEventSource` and set the flags. `asides-overlay-debug` in the temporary directory makes the
  overlay log keys and focus to `asides-overlay-debug.log` (`native::debug`).

### Overlay windows, focus, kiosk

- Panels are non-activating, so the user's app keeps its look. Borderless panels need
  `canBecomeKeyWindow`; mouse moves need an `ActiveAlways` tracking area.
- The pill is one panel, frozen (above the screens) and live: `becomesKeyOnlyIfNeeded` (clicks
  don't take the keyboard), `hidesOnDeactivate` off, laid out at its own size (a flex parent). Freezes leave out the overlay's
  windows (`capture::capture` `without`).
- There's no global Esc: live, every key belongs to the user's app; only the global shortcuts reach
  asides.
- Frozen, the kiosk options hide the Dock and menu bar and stop app switching (Mission Control,
  spaces, gestures, hot corners), hiding and the Apple menu, but not Force Quit. Hiding the menu bar
  or disabling switching needs the Dock hidden, or it throws. They apply only while the app is
  active (`native::kiosk`). Notification Center, Spotlight and screenshot keys can't be held back.
- macOS 26 takes kiosk options from an accessory app but still switches apps on ⌘Tab: the overlay
  is regular while frozen, an accessory live.
- `NSApp.activate()` is declined while the user's app is in front: use `activateIgnoringOtherApps`.
  The overlay hands the keyboard back itself (`native::give_focus`), only when it has it.
- Handing the keyboard back is asynchronous: the frozen screens stay up until the app is in front
  and redrawn, with the Dock and menu bar back (`windows::handed_back`, about 0.5 s). Redraw while
  still active; later redraws never showed. Take the kiosk options once the windows are on screen
  (`windows::take_over`), or the menu bar hides a frame early.
- An opaque window occludes the apps under it, and they stop drawing: frozen screens are
  non-opaque (`WindowBackgroundAppearance::Transparent`).
- One-frame flashes show in `screencapture -v` with per-frame diffs (ffmpeg, PIL).
- An accessibility hit test on the system-wide element finds the overlay: hit-test the app owning
  the window under the point (from the window list taken before opening).
- Errors in the menu bar are hidden under the overlay: `App::fail` also sends a `notice`.
- The notch is in the menu bar (safe area 38 pt, menu bar 39): keep drawing below it.
- A shortcut's ⌃⌥ reached Measure as ⌥: with ⌃ down, Measure takes no modifiers.
- `NSOpenPanel` runs in its own service: posted keys don't reach it, press its buttons through
  accessibility. `native::choose_folder` puts it above the frozen screens.
- GPUI updates hover only on pointer moves inside a window: leaving the pill's window, `on_hover`
  never hears it. What hovering opens there closes from the clock too (`Overlay::pointer_on_pill`).
- GPUI can't blur behind an element. The pill's window (and the asides window) is `Blurred`: GPUI
  puts an `NSVisualEffectView` beside its view in the window's content view, blurring the whole
  window; its `maskImage` keeps it to the pill's shapes (`native::set_glass`, again when the window
  resizes). No GPUI shadows on glass: they fill under the element too, and turned it near black.
  On a frozen screen, surfaces draw a blurred copy of the freeze (`glass.rs`). `screencapture -l`
  of a blurred window shows gray behind it: capture its rect (`-R`).

### Capture and measuring

- `captureImageWithFilter` drops window shadows; macOS 26's `captureScreenshotWithFilter` keeps
  them, in the display's color space (`capture::srgb`).
- Freezes are sRGB, the display P3: saturated colors clip, and one screen level is 3 to 6 sRGB
  levels, which made phantom edges until edges had to be visible (OKLab). `screencapture` keeps the
  display's colors: test on a freeze (`dumps_a_freeze`) or `sips --matchTo '/System/Library/ColorSync/Profiles/sRGB Profile.icc'`.
  A bare desktop picture is `screencapture -l` of the Dock's `Wallpaper-` window.
- Grain makes single-pixel changes as visible as a faint card's border (OKLab 0.01 to 0.03): faint
  changes must go on along a contour, compared along the change's own channel. Faint rounded corners
  and hairline ends count where they touch an edge the other way. In heavy grain, a faint change
  that doesn't stand out from it must go on 15 rows. Test on many images at once (wallpapers,
  photos, light and dark UI).
- Snapping must find the edges measuring from a point does
  (`snapping_finds_the_edges_measuring_does`); `measure::Contours` does it in two passes per block.
- Edges must not depend on the part of the frame worked on (0.7.2 lost them on each thread band's
  first row). A frozen screen's edges are found once; each area leaves out those reaching past it
  (`for_area`).
- A light gray card on white (#F3F4F6) differs by 12/255: the default tolerance (2%) must stay
  below that.

### Opening speed (262 ms → 109 ms)

- `CGPreflightScreenCaptureAccess` costs 11 ms: don't ask again until a capture fails.
- `getShareableContent` costs 30 to 50 ms: with nothing to leave out, use macOS 26's
  `captureImageInRect` (CoreGraphics coordinates, y down).
- Converting to sRGB is 25 ms on one thread: convert in strips, one per thread.
- A new overlay process took 150 ms: one stands by with its windows open (`windows::Spare`), and a
  spare draws text first. Warming Metal with a throwaway window doesn't last.
- To time it: timestamps from the overlay, `--standby` with a `windowed` session, then send
  `{"type":"open","dir":…}`. `sample` slows every library load.
- On screen (ask first): post `⌃⌥Space` as a whole chord through a private `CGEventSource`, find the
  windows by `proc_name` of `kCGWindowOwnerPID`, close with Esc. Never post it while asides is open:
  it sends. Post one Esc, and only while the frozen screen is up: with nothing done, the first
  closes asides, and a second reaches the app in front (in your agent's terminal, it interrupts you).

### Speech

- Whisper writes in the language it detects; short accented English came out Romanian: unsure
  detections keep the last sure language (`stt::choose_language`).
- Segment times run through silence between takes: keep each take's segments in it
  (`fit_segments`).

### Agents, terminals, MCP

- How each agent takes text, Return and images in each app is measured and documented in
  `deliver.rs`. In short: Terminal's `do script` sends text and CR as one paste (wrap it in
  bracketed-paste markers); Ghostty drops AppleScript Ctrl keys under the kitty keyboard protocol
  (send image paths); Diri keeps pasted paths as text for Claude Code and OpenCode (send Ctrl+V);
  Claude Code attaches a pasted path a moment later; Return while Codex works steers (send Tab).
- Diri's `session send` refuses whitespace-only text, and `session spawn --prompt` typed into
  OpenCode before it was ready: wait for "Ask anything" in `session read`.
- The terminal device's access time (`/dev/ttys003`) moves when the agent reads input: the session
  typed into last.
- Claude Code asks to trust a folder it hasn't seen: a new session in a scratch folder waits there.
- A terminal app started from inside an agent's session hands its markers to every shell: start
  test terminals from the Dock (or strip them), and quit them after.
- The Automation prompt holds an AppleScript until answered, failing after 2 minutes: run it off
  the main thread. `AEDeterminePermissionToAutomateTarget` answers only for an app that runs
  (`procNotFound` otherwise): setup starts it hidden first, and keeps the last answer once it quits. It needs the `automation.apple-events` entitlement and
  `NSAppleEventsUsageDescription`. Permissions follow the signing identity.
- Terminal's AppleScript can't make a tab (only ⌘T through System Events). Ghostty 1.3 has `new
  window … with configuration`; `osacompile` checks a script against its dictionary without sending.
  Ghostty asks before running a script opened with `open -a`.
- With Accessibility, the real status menu can be driven: a synthetic click on the app's
  `AXExtrasMenuBar` child opens it (`AXPress` doesn't), then press items through accessibility.
  Shortcuts need the whole chord posted; modifier flags alone don't fire a hot key.
- Codex asks before every MCP tool not marked `readOnlyHint`. Claude Code probes
  `server/discover` before `initialize`: answer "method not found".
- Wispr Flow's `flow.sqlite` schema changes between versions: discover columns, open it read-only
  (it writes in WAL mode).

## Commits

Imperative subjects ("Add …", "Fix …"), a body that explains why, and each commit building and
passing tests on its own. Commit only when the user asks.
