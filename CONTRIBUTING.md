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
`asides opencode`) in a scratch repo: it passes the server on the agent's command line. To check
the command line itself without a model call, add `-p --output-format stream-json --verbose` for
Claude Code and read the `init` message's `mcp_servers`, or run `codex -c … mcp get asides` with the
same `-c` options. With a dev-hooks build installed, `asides simulate <wav>` stands in for speaking.

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
| Unit | `make test` | notes, metadata, Markdown, git info, links, state, config, resampling, language choice, icon rendering, measuring (edges, snapping, guides, colors), the Measure & Annotate session (measuring, holds, guides, marks, drawings, screenshots, restore) and its Markdown, the overlay's keys and protocol, rendering a session's screenshot offscreen, threads, the MCP feedback-loop tools, `asides measure` |
| Speech | `make test-speech` | transcription of the fixtures (needs the model) and a real model download with checksum check (190 MB) |
| End-to-end | `make e2e` | installs a build with the `dev-hooks` feature, then drives it through the `asides` command in a throwaway git repo, feeding WAV files instead of the microphone (`asides simulate`), a scripted Measure & Annotate session on a drawn screen instead of the real one (`asides simulate-measure`, played by the bundled `asides-overlay` without windows), a database of its own instead of Wispr Flow's history, and `scripts/e2e-mcp.py` as a coding agent (listening making its session the target, each and batch delivery, two sessions in one repo each getting only their own asides, Send to Agent, cancelling, `asides listen`, sending with no agent listening, resolving, dismissing and replying to marks, `measure_image`), plus fake agents for `asides claude`, `codex` and `opencode` (their command lines, worktrees for a second session, pruning them; a fake Codex daemon and `codex queue` for pushed Codex sessions, in a window and in the background, and the fallback without the daemon). It restores your `config.toml` afterwards |
| Overlay replay | below | the real overlay windows, driven by AppKit mouse and key events, captured to PNGs you look at |
| Manual | checklist below | everything a script can't do: the microphone, shortcuts, menus, your own hand on the overlay, and permissions |

To look at the menu-bar icons without installing, render them to files:

```bash
ASIDES_ICON_DUMP=/tmp/icons cargo test -p asides-app icons
```

This writes raw RGBA data (`<symbol>-<width>x<height>.rgba`) for each icon.

If a release is also installed (say `/Applications/Asides.app`), it has the same bundle ID, and
Launch Services may open it instead of your build when the CLI sends a link. Set
`ASIDES_APP=~/Applications/Asides.app` to send the CLI's links to your build; `make e2e` does.

To look at the image an aside gets from Measure & Annotate (a scripted session on a drawn screen,
rendered offscreen by the overlay's exporter):

```bash
ASIDES_OVERLAY_DUMP=/tmp/overlay cargo test -p asides-overlay renders_marks
```

It writes `session.png`. To try a session of your own on any screenshot, with an installed
dev-hooks build (`make install FEATURES=dev-hooks`):

```bash
asides simulate-measure shot.png script.json   # {"scale": 2, "actions": [{"drag": {"from": [10, 10], "to": [200, 90]}}]}
```

The actions are `click`, `drag`, `objects`, `hold`, `guide`, `annotate`, `draw`, `shot`, `tool`,
`tolerance` and `units`; see `overlay::Action`.

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
  - `⌃⌥Space` tap starts, a second tap stops
  - holding `⌃⌥Space` records until release
  - `⌃⌥N` records into a new note
  - `Esc` cancels and writes nothing
  - Esc still works normally in other apps afterwards
- [ ] **Screenshots:** `⌃⇧⌘4` while recording is placed at the right sentence; `⌃⌥V` while idle
  appends to the note.
- [ ] **Menu:**
  - the icon states: idle, red with a timer, ellipsis, `✓ #N`, warning
  - Directory switching: recent repos and Choose Directory… (focus returns to your previous app)
  - every Preferences item is saved to `config.toml`
- [ ] **Robustness:**
  - a recording started while the previous one is still transcribing
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
  under Asides Go To; Codex and OpenCode the same. With Ghostty picked and running, the first
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
- [ ] **Measure & Annotate, first use after `tccutil reset ScreenCapture io.github.mihaicristianfarcas.asides`:**
  `⌃⌥M` asks for Screen Recording (naming Asides), and says what to do; after allowing it (and
  reopening Asides), `⌃⌥M` freezes every screen.
- [ ] **Measure & Annotate, opening** (with a browser in front, on a laptop and a second display,
  and over a full-screen app): every screen freezes within a second, the toolbar is on the screen
  with the mouse, no Dock icon appears, keys work without clicking first, and closing gives the
  focus back to the browser. The keys sheet shows the first time only.
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
- [ ] **Measure & Annotate, marking and drawing:**
  - `A`, then click a button in Safari or Chrome: marker 1, the card takes typing right away (with
    diacritics, an input method, and ⌘V), Return saves it, the dots and `⌘1`–`⌘4` pick a kind, its
    microphone starts and stops recording, `⌫` over a marker removes it
  - with Identify UI Elements on (and Accessibility allowed), hovering outlines elements and the
    entry names the button, the page's URL and a selector with the DOM's classes
  - `P`: a circle around a heading names it; a ⇧-drag arrow names both ends
  - `⌘S`: a dragged screenshot is on the clipboard, and in the aside after Done
- [ ] **Measure & Annotate, toolbar and keys:** icons only, names on hover; hovering `?` shows the
  keys; `⇧Tab` hides the toolbar (a timer stays while recording) and it stays hidden next time.
- [ ] **No repo:** with Asides unbound (or bound to a deleted folder), the menu bar shows "No repo";
  the overlay says so at the top, and Space, the microphone and Done say why they don't act.
- [ ] **Measure & Annotate, talking:**
  - hold Space and speak, mark two things, let go: the toolbar showed the time; Done writes one
    entry with the transcript, the screenshot, and each mark where you were speaking about it
  - tap the microphone to start, mark, tap to stop, keep marking, Done
  - `⌘⏎` (Send) in batch mode: the entry is written and sent to the listening agent
  - Esc closes without adding, cancels a recording started in it, and Esc works in other apps after
  - `⌃⌥M` while it's open adds what's there, like Done
- [ ] **Agent replies:** an agent's `resolve_aside` flashes `✓ #N` in the menu bar and shows under
  Agent Replies; the note line counts the open asides.

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
  writes `packaging/Asides.icns`, which `make bundle` copies into the app.
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
