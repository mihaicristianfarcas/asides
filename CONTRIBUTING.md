# Contributing to asides

Thanks for helping! Bug reports, fixes, docs and features are all welcome. For anything bigger
than a fix, please open an issue first, so we can agree on the approach before you spend time on
it. Read [ARCHITECTURE.md](ARCHITECTURE.md) before changing the app: it explains how the pieces
fit together and lists the invariants the code relies on.

## Set up

You need macOS 14 or later (Apple silicon recommended), recent stable Rust, the Xcode Command
Line Tools and CMake (`brew install cmake`). Full Xcode isn't required.

```bash
git clone https://github.com/mihaicristianfarcas/asides && cd asides
make cert        # once, only if you have no Apple Development or Developer ID certificate
make install     # builds, signs and installs Asides.app and the asides command
make fixtures    # once: test audio generated with the macOS voices
```

- `make install` signs with the best identity it finds (`scripts/signing-identity.sh`): Developer
  ID Application, then Apple Development, then "Asides Dev". A stable identity is what lets
  macOS keep the microphone permission across rebuilds; with an ad hoc signature it asks after
  every `make install`. `make cert` creates the self-signed "Asides Dev" identity and asks for
  your password.
- `make fixtures` needs the Romanian voice "Ioana": System Settings → Accessibility → Spoken
  Content → System Voice → Manage Voices… The generated files are gitignored, because the macOS
  license doesn't allow redistributing system-voice audio.
- The first launch downloads the speech model (574 MB) to
  `~/Library/Application Support/Asides/models/`.

## Development loop

| You changed | Check it with |
|---|---|
| `asides-core` | `cargo test -p asides-core` |
| the CLI | `cargo run -p asides-cli -- status` (and the other subcommands) |
| `asides mcp` | pipe JSON-RPC lines into `cargo run -p asides-cli -- mcp` (example below), then try it in an agent |
| `asides claude` and the like | put a fake `claude` (a script that prints its arguments) first on `PATH`, as `scripts/e2e-mcp.py` does; then start the real one |
| opening a terminal | `ASIDES_TERMINAL=Diri ASIDES_TERMINAL_DIR=<repo> ASIDES_TERMINAL_COMMAND='asides claude' cargo test -p asides-app opens_a_terminal -- --ignored` runs New Agent Session's `terminal::open` for real |
| the app | `make install`, then `asides` in a scratch repo and try it for real |

`make install` quits the running Asides and replaces it.

The MCP server speaks JSON-RPC, one message per line, on stdin and stdout:

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"me","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_asides","arguments":{}}}' \
  | cargo run -q -p asides-cli -- mcp
```

To try it in an agent without changing its settings, run `asides claude` (or `asides codex`,
`opencode`, `copilot`, `qwen`) in a scratch repo: it passes the server on the agent's command line
(`asides gemini` adds it to Gemini CLI's settings once). To check the command line itself without a
model call, add `-p --output-format stream-json --verbose` for Claude Code and read the `init`
message's `mcp_servers`, run `codex -c … mcp get asides` with the same `-c` options, give Copilot
CLI `-p` in place of `--interactive` (it checks the server config before asking for a login), or
give Qwen Code `-p` (it warns when a server fails to start). With a dev-hooks build installed, `asides simulate <wav>` stands in for speaking.

The app's output (`eprintln!`) is discarded when it's launched normally. To see it, quit the app
(`asides quit`) and run the binary from a terminal:
`~/Applications/Asides.app/Contents/MacOS/Asides`. In that mode macOS attributes the
microphone permission to your terminal app, so it may ask again.

Before you open a pull request:

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --features asides-app/dev-hooks -- -D warnings
make test
make e2e          # if you changed behavior of the CLI or the app
```

CI runs formatting, clippy, the unit tests and the app bundle on every pull request. `main` is
protected: changes reach it through pull requests that pass CI, and it can't be force-pushed or
deleted.

## Tests

| Layer | Command | Covers |
|---|---|---|
| Unit | `make test` | notes, metadata, Markdown, git info, links, state, config, resampling, language choice, icon rendering, measuring (edges, snapping, guides, colors), the Asides session (measuring, holds, guides, annotations, drawings, screenshots, restore, freezing again) and its Markdown, the overlay's keys and protocol, rendering a session's screenshot offscreen, threads, the MCP feedback-loop tools, `asides measure` |
| Speech | `make test-speech` | transcription of the fixtures (needs the model) and a real model download with checksum check (190 MB) |
| End-to-end | `make e2e` | installs a build with the `dev-hooks` feature, then drives it through the `asides` command in a throwaway git repo, feeding WAV files instead of the microphone (`asides simulate`), a scripted Measure & Annotate session on a drawn screen instead of the real one (`asides simulate-measure`, played by the bundled `asides-overlay` without windows; and one with recordings arriving while it's open, one of them about an annotation, which make one entry in the order things happened), a database of its own instead of Wispr Flow's history, and `scripts/e2e-mcp.py` as a coding agent (listening making its session the target, each and batch delivery, two sessions in one repo each getting only their own asides, Send to Agent, cancelling, `asides listen`, sending with no agent listening, resolving, dismissing and replying to annotations, `measure_image`), plus fake agents for `asides claude`, `codex` and `opencode` (their command lines, worktrees for a second session, pruning them; a fake Codex daemon and `codex queue` for pushed Codex sessions, in a window and in the background, and the fallback without the daemon). It restores your `config.toml` afterwards |
| Overlay replay | below | the real overlay windows, driven by AppKit mouse and key events, captured to PNGs you look at |
| Manual | checklist below | everything a script can't do: the microphone, shortcuts, menus, your own hand on the overlay, and permissions |

To look at the menu-bar icons without installing, render them to files:

```bash
ASIDES_ICON_DUMP=/tmp/icons cargo test -p asides-app icons
```

This writes raw RGBA data (`<status>-44x44.rgba`) for each icon.

If a release is also installed (say `/Applications/Asides.app`), it has the same bundle ID. The
CLI sends its links to the app it ships in (`asides` from `~/.local/bin` reaches your build), or to
`ASIDES_APP`; `make e2e` sets it. Opened from Spotlight or Finder, either copy may start, and two
copies fight over the shortcuts: quit the one you're not testing.

To look at the image an aside gets from Measure & Annotate (a scripted session on a drawn screen,
rendered offscreen by the overlay's exporter):

```bash
ASIDES_OVERLAY_DUMP=/tmp/overlay cargo test -p asides-overlay renders_annotations
```

It writes `session.png`. Measure works on a freeze's pixels, in sRGB, which a screenshot in the
display's colors (`screencapture`'s) isn't: to measure what a freeze sees, capture one with
`ASIDES_CAPTURE_DUMP=/tmp/freeze cargo test -p asides-app dumps_a_freeze -- --ignored` (it writes
RGBA; needs Screen Recording). To try a session of your own on any screenshot, with an installed
dev-hooks build (`make install FEATURES=dev-hooks`):

```bash
asides simulate-measure shot.png script.json   # {"scale": 2, "actions": [{"drag": {"from": [10, 10], "to": [200, 90]}}]}
```

The actions are `click`, `drag`, `objects`, `hold`, `guide`, `annotate`, `draw`, `shot`, `tool`,
`tolerance`, `units` and `wait` (seconds with the session open: `asides simulate` meanwhile is said
in it, and `asides simulate --annotation N` about annotation N, as the end-to-end test does); see
`overlay::Action`.

To see and drive the overlay itself, run `target/debug/asides-overlay <dir>` on a session
directory of your own: `session.json` (`helper::Spec`) with a screen whose `pixels` file holds
raw RGBA, plus `"windowed": {"x": …, "y": …, "width": …, "height": …}` for a small window instead
of the whole screen, and `"replay": [...]` with steps like `{"move": [200, 140]}`,
`{"down": [50, 160]}`, `{"drag": [175, 205]}`, `{"up": [175, 205]}`, `{"key": "shift+h"}`,
`{"type": "Too tight"}`, `{"answer": {element}}` (as the app would answer a lookup),
`{"recording": 14}` (as if recording) and `{"capture": "card.png"}` (see `replay.rs`).
Replays send events to GPUI's view directly; to check what the real input path delivers (focus,
the first responder), run the overlay through the app (`launchctl setenv ASIDES_OVERLAY <binary>`,
then restart Asides) and create `asides-overlay-debug` in the temporary directory
(`getconf DARWIN_USER_TEMP_DIR`): the overlay logs the keys it gets and focus changes to
`asides-overlay-debug.log` there. The steps reach GPUI as AppKit events, so they take the
path real ones do, and the captures (`screencapture -l`, which needs Screen Recording for your
terminal) show what the user would see. Look at them.

### Manual QA checklist

Run the relevant parts when you change the app's UI, recording, shortcuts or permissions, and say
in your pull request what you tried.

- [ ] **First launch after `tccutil reset Microphone io.github.mihaicristianfarcas.asides`:**
  the microphone prompt names Asides, not your terminal.
- [ ] **Shortcuts with a browser in front, including full screen:**
  - `⌃⌥Space` opens Asides frozen, its toolbar at the bottom of the screen with the mouse
  - holding `⌃⌥Space` (open or not) talks until release, the first words included while the screen
    freezes; then a tap adds it all to the note
  - `⌃⌥N` does the same into a new note
  - `⌃⌥P` goes live: only the toolbar, and the browser takes clicks and typing (Esc too); again,
    frozen
- [ ] **Screenshots:** `⌃⇧⌘4` while talking is placed at the right sentence; `⌃⌥V` with Asides open
  goes in its aside; closed, it appends to the note.
- [ ] **Menu:**
  - the icon states: idle, red with the time beside it, ellipsis, the entry's number (`[5]`, wider for
    `[12]`), warning; nothing else beside the brackets
  - Directory switching: recent repos and Choose Directory… (focus returns to your previous app)
  - every Preferences item is saved to `config.toml`
- [ ] **Robustness:**
  - Asides opened, and talked in, while the previous aside is still transcribing
  - AirPods as the microphone
  - sleep while recording
  - the bound directory deleted while the app runs
- [ ] **CLI:** closing the terminal after `asides` leaves the app running; `asides quit`
  finishes queued transcriptions first.
- [ ] **Agents:** with an agent listening, the status line names it (`→ Claude Code, listening`, or
  `working` while it acts); a recording reaches it a few seconds after you stop. In Claude Code,
  `/asides:listen` starts a monitor and each aside shows in the chat. In batch mode, `⌃⌥Enter` and
  Send to Agent in the menu deliver everything recorded since the last send, including a recording
  still in progress, and the menu bar shows Sent or Queued; with no agent listening, it says so and
  sends nothing. In each mode the item is disabled and `⌃⌥Enter` stays free for other apps.
- [ ] **Agent sessions:** New Agent Session ▸ Claude Code opens Terminal in the repo with Claude
  Code already listening (no approval asked for the monitor or the asides tools; no prompt from the
  terminal in Ghostty or Diri either, picked under Open Agent Sessions In), and becomes the target
  under Asides Go To; Codex and OpenCode the same, and Gemini CLI, Copilot CLI and Qwen Code when
  installed (no Background item for them; Gemini CLI asks you to trust the folder first, and the
  first session says it added Asides' server to its settings). With Ghostty picked and running, the first
  session asks whether Asides may control Ghostty: allowed, each session is a new tab in its front
  window; declined (or Ghostty not running), a Ghostty window of its own. A second session in the
  repo works in a worktree. `⌃⌥Tab` cycles the sessions and Notes Only, the menu bar saying where
  asides go, and a recording shows `→ <agent>` while it runs and reaches only that session. A Codex
  or OpenCode session gets each aside as a message, in its window, without waiting in a tool call.
  Claude Code, Codex and OpenCode in the Background start without a window; Show opens them (Codex
  and OpenCode in their worktree, without asking which directory) and Stop ends them (OpenCode's own
  `asides` server is back in that directory). Quitting the target agent hands asides to another
  session in the repo, or Notes Only. From Spotlight, with Asides not bound: New Agent Session ▸ In
  Another Directory… starts one after choosing a folder. With Asides started by an agent (`open` it
  from Claude Code's shell), sessions still save their transcripts (no "inherited
  CLAUDE_CODE_CHILD_SESSION" warning).
- [ ] **Installed by hand:** with no `asides` command anywhere, opening `/Applications/Asides.app`
  links `/usr/local/bin/asides` (asking for a password if root owns the folder); after Cancel it
  isn't asked again. The app icon shows in Spotlight and Finder.
- [ ] **Wispr Flow:** with Add Wispr Flow Dictations on, a dictation lands in the note marked
  `· Wispr Flow`; with it off, nothing does.
- [ ] **Freezing, first use after `tccutil reset ScreenCapture io.github.mihaicristianfarcas.asides`:**
  `⌃⌥Space` asks for Screen Recording (naming Asides), and says what to do; after allowing it (and
  reopening Asides), it freezes every screen.
- [ ] **Live and frozen** (with a browser in front, on a laptop and a second display, and over a
  full-screen app):
  - opening freezes every screen within a second; keys work without clicking first; no Dock icon
    appears; the keys sheet shows the first time only
  - frozen, the Dock doesn't slide in (pointer at the screen's edge); ⌘Tab, the three- and
    four-finger swipes (Mission Control, App Exposé, switching spaces), the Show Desktop pinch and
    hot corners do nothing; Force Quit still works; live, everything works again
  - ▶ or `⌃⌥P` goes live with the keyboard in the browser; the live toolbar stays above the
    browser, and clicking it doesn't take the keyboard from it
  - a tool (or ⏸) freezes again with that tool picked, without the toolbar in the screenshot;
    Done: one entry, a screenshot for each freeze, in order
  - drag the toolbar by its handle, live and frozen; fold and unfold it; both are remembered next
    time
  - × closes without adding anything; so does Esc on a frozen screen, at once with nothing done,
    otherwise on a second press (the first says so)
  - live, typing in the browser (Esc, Space, letters) never reaches Asides
- [ ] **Measure & Annotate, measuring:**
  - moving shows the distances across and down in one pill, matching what the page's CSS says
  - click holds them; `H`/`V` hold one; a drag around a button snaps to it; right-drag and ⌘-drag
    don't snap; ⌥-drag holds each object; Space moves the area while dragging; Shift shows the
    crosshair and squares the area
  - `⇧H`/`⇧V` add guides that snap to held objects and edges; arrows nudge them; gaps are labelled
  - the toolbar's tolerance cycles Low/Medium/High; `-`/`=`/`T`; `U` switches pt and px
  - `⌘C` puts `W × H` on the clipboard; `C` the image (and it isn't attached twice while recording)
  - right-click: the menu; on a held object, its screenshot; its camera on hover
  - `⌫` undoes, `X` clears, `Tab` hides everything, `R` restores the last session's guides
  - over a glass menu on a bright wallpaper (a sky with clouds), distances go through what the glass
    blurs to the menu's edges and separators; on the empty sky they reach the clouds, and an area
    there finds nothing
  - on a photo or a wallpaper (a painted one, a grainy iMac one), distances go past its grain and
    specks (and a faint haze) to what's drawn: outlines, dots, a faint card's border, a cloud's soft
    edge
  - on a light UI, a faint selected item (#F3F4F6 on #F9FAFB) stops a distance across it near its
    rounded corners too, and an area around a faint hairline snaps to all of it
  - on a notched display, nothing the overlay draws (the "Not bound" banner, labels, cards, menus,
    the toolbar dragged up, live or frozen) goes over the menu bar or under the notch
- [ ] **Measure & Annotate, annotating and drawing:**
  - `A`, then click a button in Safari or Chrome: marker 1, the card takes typing right away (with
    diacritics, an input method, and ⌘V), long text wraps and the card grows, the dots and
    `⌘1`–`⌘4` pick a kind, `⌫` over a marker removes it
  - Return, Esc or a click elsewhere closes the card and keeps the annotation and its text; a click
    elsewhere also places the next annotation; clicking the number opens the card again
  - in the card, hold Space and speak: it talks about the annotation; a tap types a space, also
    while typing fast
  - starting and stopping speaking in a card doesn't flash or move its text field (also near the
    bottom of the screen, where the card opens above the annotation)
  - the card's microphone: tap, speak, tap: "Transcribing…", then what you said in the card, and a
    trash button beside the microphone; the trash deletes it; speaking again adds to it. Close the
    card while it's transcribing and annotate something else: the first card has its words later
  - measure an area (`D`, drag), then `A`: hovering inside it outlines it in blue, and a click
    annotates the area (a second click edits that annotation); with Identify UI Elements on, a
    button inside a large section is outlined and annotated instead
  - right-click a measured area: Annotate This opens the card on the area; elsewhere, Annotate
    Here annotates what's under the pointer
  - with Identify UI Elements on (and Accessibility allowed), hovering outlines elements and the
    entry names the button, the page's URL and a selector with the DOM's classes
  - `P`: a circle around a heading names it; a ⇧-drag arrow names both ends
  - `⌘S`: a dragged screenshot is on the clipboard, and in the aside after Done
- [ ] **Toolbar and keys:** icons only, names on hover; hovering `?` shows the keys; on a frozen
  screen `⇧Tab` hides the toolbar (a timer stays while recording) and it stays hidden next time.
- [ ] **No repo:** with Asides unbound (or bound to a deleted folder), the menu bar shows `[ ]` alone;
  a frozen screen says so at the top, and Space, the microphone and Done say why they don't act.
- [ ] **Talking:**
  - live (▶), hold `⌃⌥Space`, speak while clicking through the browser, let go, then freeze,
    annotate, Done: one entry, what you said between the two screenshots
  - frozen, hold Space and speak, measure something and annotate two things, let go: the toolbar
    showed the time; Done writes one entry with the transcript, the screenshot, and each
    measurement and annotation where you were speaking about it
  - tap the toolbar's microphone to start, annotate, tap to stop, keep annotating, Done
  - with hold to talk: hold Space, speak, let go ("Kept for the aside"), measure, hold Space again,
    speak, let go, Done: one entry, each time you spoke a paragraph, the measurement between them,
    and no entry before Done
  - an annotation's card microphone, then Done before it's transcribed: one entry, the annotation
    with `**Said:**` and what you said
  - `⌘⏎` (Send) in batch mode: the entry is written and sent to the listening agent
  - × closes without adding, cancels a recording started in it and drops what was said in it; the
    menu bar shows `[×]` for a moment, nothing beside it
  - a tap on `⌃⌥Space` while it's open adds it all, like Done
  - the entry says where you were (`**In:** Safari · "…" · http://…`) where you started talking,
    and again, before what you said there, after you switched apps live and talked there; a
    screenshot with only measurements says it too
- [ ] **Agent replies:** an agent's `resolve_aside` flashes `[✓]` in the menu bar (`dismiss_aside`
  `[–]`, `reply_to_aside` `[?]`), the menu's first line says which aside, and it shows under Agent
  Replies; the note line counts the open asides.

## Common changes

- **A setting:**
  1. Add the field to `Config` in `config.rs`, with a default so older `config.toml` files still
     load.
  2. Read it where it's needed in the app.
  3. If it's a toggle, add it to the menu (below).
  4. Document it in the README's Settings block.
- **A menu item:**
  1. Add a `MenuAction` variant and the item in `tray.rs` (`Tray::new`, `Tray::action`, and
     `show` or `sync_config` if its state changes).
  2. Handle the action in `App::on_menu`.
  3. Escape `&` in dynamic text with `menu_text`.
- **A CLI command:**
  - Add a `Command` variant in `asides-cli/src/main.rs`.
  - If it needs the app, add a `Link` variant in `link.rs` (with a round-trip test) and handle it
    in `App::handle_link`.
  - Replies go through `state.json`: add a field to `AppState` with a serde default.
- **An MCP tool or prompt:** add it to `tools()` or `prompts()` in `asides-cli/src/mcp.rs` and handle
  it in `call_tool` or `prompt`. Keep tools read-only (and annotated `readOnlyHint`) unless they
  must change something, like the feedback-loop tools (which only append to `threads.jsonl`), and
  stdout for JSON-RPC only. Cover it in
  `scripts/e2e-mcp.py`, and try it in Claude Code and one other agent: clients differ in timeouts
  and in what content they accept.
- **A model:** add it to `MODELS` in `models.rs` with the exact size and SHA-256 from the Hugging
  Face file listing. All models must be multilingual.
- **The app icon:** edit `scripts/make-icon.swift` and run it (`swift scripts/make-icon.swift`). It
  writes `packaging/Asides.icns`, which `make bundle` copies into the app. The same mark, the a in
  brackets, is drawn in `crates/asides-app/src/icons.rs` (the status item), `docs/favicon.svg` and
  `docs/index.html` (the header and the demo's menu bar): change them together, then run
  `python3 scripts/make-og.py`.
- **The note format:** change `markdown.rs` or `frontmatter.rs` along with their tests, then the
  e2e expectations and the README example. Keep the format append-only (see the invariants in
  ARCHITECTURE.md).
- **A Measure & Annotate tool or key:** put its logic in `asides-core/src/overlay.rs` (with a test,
  and an `Action` if a script should drive it). Give a key a default in `OverlayKeys`
  (`config.rs`), then route it in `asides-overlay/src/ui.rs` (`Keys`, `Overlay::command`, or the
  mouse methods) and draw it in `ScreenView::render` (and in `export.rs` if it belongs in the
  image). Add it to the toolbar's hints, the keys sheet, the context menu if it fits, and the
  README. Check it with a replay and look at the captures.
- **What a mark records:** `inspect.rs` (`Element`, `Annotation`, `Shape`, and the Markdown at each
  detail level).
- **Something the overlay asks the app:** a variant of `helper::Event` (and `Command` for the
  answer), handled in `App::on_overlay`. The overlay never needs a permission itself.

## Style

- `rustfmt` (120 columns, see `rustfmt.toml`) and clippy with warnings treated as errors.
- Put logic in `asides-core` when it doesn't need macOS. That's where it can be unit-tested.
- Comments explain why, not what. Match the surrounding code's naming and comment density.
- Error messages are shown to users in the menu and the CLI, so make them actionable: say what
  happened and what to do. Avoid `unwrap()` on anything that depends on user data, files or the
  system.
- Don't add dependencies lightly; each one adds build time to every contributor and to CI.

## Commits and pull requests

- Keep each pull request focused, and each commit building and passing tests on its own.
- Commit subjects use the imperative mood ("Add …", "Fix …") and stay under about 72 characters.
  The body explains why.
- Fill in the pull request template: what changed, why, and how you tested it, including manual QA.

## Releasing (maintainers)

Releases are built locally and notarized by Apple. You need a "Developer ID Application"
certificate in your keychain.

1. **Store notary credentials once.** Create an app-specific password at
   [account.apple.com](https://account.apple.com) (Sign-In and Security → App-Specific
   Passwords), then run `xcrun notarytool store-credentials asides-notary --apple-id <Apple ID>
   --team-id <Team ID>`. To use another profile name, set `NOTARY_PROFILE`.
2. **Set the version** in `Cargo.toml` (`[workspace.package] version`). The app's `Info.plist` picks
   it up.
3. **Run `make release`.** It:
   - builds the app with the CLI inside (`Contents/Helpers/asides`)
   - signs both with the hardened runtime and a secure timestamp
   - submits the build to Apple and waits; if Apple rejects it, the script prints the notary log
   - staples the ticket and checks the result with Gatekeeper
   - writes `dist/Asides-<version>.zip` and prints its SHA-256
   - writes the version and checksum into the Homebrew cask, `Casks/asides.rb`
   - packs the stapled app into `dist/Asides-<version>.dmg`, with a link to Applications
4. **Publish the disk image:** `make publish-dmg`. It deploys the image to a Cloudflare Worker
   that only serves static files (`packaging/download/`), as
   `https://download.asides.mihaicristian.dev/Asides.dmg`, the website's download link. It
   needs [Wrangler](https://developers.cloudflare.com/workers/wrangler/), logged in to the
   Cloudflare account. The name stays the same across versions, so the link never changes.
5. **Tag and publish:** `git tag v<version> && git push --tags`, then
   `gh release create v<version> dist/Asides-<version>.zip`.
6. **Update Homebrew:** commit `Casks/asides.rb` and push it to `main`. The repository doubles
   as a Homebrew tap, so `brew upgrade` sees the release from then on. Push it only after the
   release is published: the cask downloads the zip from it.

## The website

`docs/` is the landing page, https://asides.mihaicristian.dev, a Cloudflare Worker that only serves
those files (`packaging/site/`). Pushes to `main` that change `docs/` deploy it (the Website
workflow, with the repository's `CLOUDFLARE_API_TOKEN` secret); `make publish-site` deploys it from
your machine, with Wrangler logged in to the Cloudflare account. It's one static `index.html`
with inline CSS and JavaScript and no build step: open it in a browser to preview it (`python3 -m
http.server -d docs` serves it at http://localhost:8000). The demo is a Mac desktop drawn on a 1024
× 640 grid that scales with the page, phones included, like a picture. What the tester says shows as
subtitles inside the picture. It plays in a loop while it's on screen (once, with reduced motion).
`wallpaper.webp` is its background. `og.png` is the 1200×630 image shown when the link is shared:
the demo frozen mid-recording. After changing the demo, regenerate it with `python3
scripts/make-og.py` (it needs Firefox). Pushes to `main` that only change `docs/` skip CI; pull
requests always run it.

The page repeats the shortcuts, requirements and install steps from the README, so update both
together.

## Reporting bugs

Use the bug report template. Include:
- your macOS version
- `asides --version`
- the output of `asides status`
- the steps to reproduce

Notes and audio clips can contain private information, so share only the relevant excerpt.

## License

By contributing, you agree that your contributions are dual licensed under the MIT and Apache-2.0
licenses, as described in the [README](README.md#license).
