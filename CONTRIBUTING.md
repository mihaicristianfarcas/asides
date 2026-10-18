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
| finding sessions | `cargo test -p asides-core discovers -- --ignored --nocapture` lists the sessions running, or `asides sessions` |
| sending (`deliver.rs`) | start an agent in a scratch repo in Terminal, Ghostty or Diri, then, with a dev-hooks build installed, `asides simulate <wav> --image <png> --to <pid>` (the pid from `asides sessions`), and read what the agent got in its own transcript |
| the Asides window | `target/debug/asides-overlay --window`; `ASIDES_HOME` (a history of your own), `ASIDES_WINDOW_SECTION` (`sessions`, `settings`), `ASIDES_WINDOW_ASIDE` (an id) and `ASIDES_WINDOW_APPEARANCE` (`light`, `dark`) to capture each part (`screencapture -l`) |
| the app | `make install`, then try it for real |

`make install` quits the running Asides and replaces it.

The MCP server speaks JSON-RPC, one message per line, on stdin and stdout:

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"me","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_aside","arguments":{}}}' \
  | cargo run -q -p asides-cli -- mcp
```

Sending to a real agent takes a model call. Start it with a first message that keeps it small
("reply to each test message in one line, don't run tools"), in a scratch repo: Claude Code asks
whether to trust a folder the first time (a new session waits at that question). Terminal and
Ghostty ask whether Asides may control them, the first time; Diri doesn't (Asides uses its
`dirijor` command), which is why the end-to-end test prefers it. Agents mark the processes they
run as theirs (`CLAUDE_CODE_CHILD_SESSION`…): a terminal app started from inside an agent's session
hands that to every shell it opens, and Claude Code then stops saving transcripts; start it from
the Dock, or strip the markers first.

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
| Unit | `make test` | the history (keeping, listing, pruning), finding sessions (the process table, apps, last input, labels, folders), delivery plans for each app and agent, Markdown and messages, links, state, config, resampling, language choice, icon rendering, measuring (edges, snapping, guides, colors), the Asides session (measuring, holds, guides, annotations, drawings, screenshots, restore, freezing again) and its Markdown, the overlay's keys and protocol (targets, Tab), rendering a session's screenshot offscreen, the MCP server, `asides measure` |
| Speech | `make test-speech` | transcription of the fixtures (needs the model) and a real model download with checksum check (190 MB) |
| End-to-end | `make e2e` | installs a build with the `dev-hooks` feature, then drives it through the `asides` command, with a history of its own (`ASIDES_HOME`): WAV files instead of the microphone (`asides simulate`), kept in the history; a scripted session on a drawn screen (`asides simulate-measure`, played by the bundled `asides-overlay` without windows), and one with recordings arriving while it's open, one of them about an annotation, which make one aside in the order things happened; Wispr Flow dictations (a database of its own) made before Asides opened and while frozen; sending to a stand-in agent (`scripts/e2e-agent.c`, built as `claude`, run in Diri, or in Terminal without Diri): the text, Return, screenshots, the clipboard put back; `asides mcp`. It restores your `config.toml` and clipboard afterwards |
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
directory of your own (or `asides-overlay --standby`, then send `{"type":"open","dir":"<dir>"}` on
its standard input, as the app does): `session.json` (`helper::Spec`) with a screen whose `pixels` file holds
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

Run the relevant parts when you change the app's UI, recording, shortcuts, sending or permissions,
and say in your pull request what you tried.

- [ ] **First launch after `tccutil reset Microphone io.github.mihaicristianfarcas.asides`:**
  the microphone prompt names Asides, not your terminal. Nothing downloads; the menu says there's
  no voice model yet.
- [ ] **The menu bar:** the mark alone (`[a]`; a red dot while listening, dots while transcribing or
  sending, `[n]` for the day's nth aside sent, `!` on an error, with its reason atop the menu). The
  menu: the status, Open Asides, Asides Window…, Quit.
- [ ] **Opening:** with Claude Code running in Terminal in a repo, and a browser in front:
  - `⌃⌥Space` freezes every screen at once (no flash), the pill at the top in the middle, below the
    menu bar and the notch, naming the Claude Code session; the keys sheet the first time
  - with a model on disk, the microphone is red and counts; without one, it's crossed out, and a
    click on it downloads one (the progress shows), after which it listens
  - with Identify on, the first open asks for Accessibility; hovering outlines elements and names
    them
- [ ] **Pinning:** a click pins the element (no card); a drag pins an area; a click on a pin's
  number opens its card; `⏎` saves and closes it, `⏎` again sends. Typing in a card, Space types,
  held it talks about that pin.
- [ ] **The pill:** `Tab` switches the session (with a note of where it now goes); a click on its
  name lists the sessions (one in Warp or VS Code dimmed, with why), New session in… (agent chips,
  recent folders) and History only. Drag it by its body; it stays there next time. `M`, `D`, `S`
  switch tools (the pill shows which; a click on it goes back to Annotate).
- [ ] **The microphone:** a click mutes and unmutes it; muted, hold `Space` to talk; with
  **Listen as soon as Asides opens** off, it opens muted.
- [ ] **Live:** `⌃⌥P` goes live: only the pill, and the browser takes clicks and typing (Esc too);
  talking goes on; `⌃⌥P` freezes again (the pill not in the screenshot); `⌃⌥Space` sends from live
  too. Closed, `⌃⌥P` opens Asides live.
- [ ] **Sending, for each of Claude Code, Codex and OpenCode, in each of Terminal, Ghostty and Diri:**
  - the first time in Terminal or Ghostty, macOS asks whether Asides may control it
  - the aside shows in the chat as your message, each screenshot attached (`[Image #1]`) where it
    was taken, and the agent answers it
  - sent while the agent works, it waits for its turn (Codex shows it queued) and isn't lost
  - your clipboard has what it had before
  - with the session gone, the menu says so, and the aside is in History
- [ ] **New session in…:** in each of Terminal (the default), Ghostty and Diri (Settings), a window
  opens in the folder running the agent with the aside as its first message; in bash, zsh and fish.
- [ ] **Closing:** `Esc` (twice with something done) or × sends nothing; the app you were in has the
  keyboard back, as it was (no gray window buttons for a frame).
- [ ] **The Asides window** (menu, `asides`): History lists every aside with where it went; Send to,
  Copy, Show in Finder and Delete work; Sessions lists what `asides sessions` does; every setting
  changes `config.toml` and takes effect (the model, listening on open, the microphone, naming, the
  detail, units, the terminal and agent for new sessions, history days, audio, sounds, Wispr Flow);
  light and dark mode follow the system.
- [ ] **Wispr Flow:** dictated while frozen, it joins the aside where it was said; dictated anywhere
  else, nothing happens.
- [ ] **Kiosk:** while frozen, the Dock and menu bar stay hidden, and ⌘Tab, Mission Control, App
  Exposé, Show Desktop, spaces (keys, gestures, hot corners) do nothing; Force Quit still works.
- [ ] **Measuring** (`M`): distances, holds (`H`, `V`), guides (`⇧H`, `⇧V`), drags snapping to
  objects (right-drag as drawn, `⌥`-drag each object), the tolerance (`-`, `=`, `T`), `U`, copying
  (`⌘C`, `C`, `⌘⇧C`), `R` restoring the last session's.
- [ ] **Several displays:** each freezes; the pill opens on the one with the pointer; drawing and
  measuring work on each.
- [ ] **Frame by frame** (`screencapture -v` and per-frame diffs): opening, ▶/⏸ and closing show no
  stale frame, no flash, no menu bar a frame early.

## Common changes

- **A setting:**
  1. Add the field to `Config` in `config.rs`, with a default so older `config.toml` files still
     load.
  2. Read it where it's needed in the app.
  3. Add it to the Asides window's Settings (`window.rs`).
  4. Document it in the README's Settings block.
- **A menu item:** keep the menu short (the window has the rest). A `MenuAction` variant and the
  item in `tray.rs`, handled in `App::on_menu`; escape `&` in dynamic text with `menu_text`.
- **A CLI command:**
  - Add a `Command` variant in `asides-cli/src/main.rs`.
  - If it needs the app, add a `Link` variant in `link.rs` (with a round-trip test) and handle it
    in `App::handle_link`.
  - Replies go through `state.json` (add a field to `AppState` with a serde default) or the history.
- **An MCP tool:** add it to `tools()` in `asides-cli/src/mcp.rs` and handle it in `call_tool`. Keep
  tools read-only (and annotated `readOnlyHint`), and stdout for JSON-RPC only. Cover it in the
  end-to-end test, and try it in Claude Code and one other agent.
- **Another agent or terminal app:** an agent is a row in `agents.rs` (its process, its command line
  for a new session, the key that sends). An app is a `Host` in `sessions.rs` (told by its
  environment) and its plan in `deliver.rs`. Measure what the agent records when text, Return and a
  screenshot reach it there (a multi-line message, one sent while it's busy, an image with Ctrl+V
  and as a path) before writing the plan, and add the results to design/redesign.md.
- **A model:** add it to `MODELS` in `models.rs` with the exact size and SHA-256 from the Hugging
  Face file listing, and `english` for `.en` models.
- **The app icon:** edit `scripts/make-icon.swift` and run it (`swift scripts/make-icon.swift`). It
  writes `packaging/Asides.icns`, which `make bundle` copies into the app. The same mark, the a in
  brackets, is drawn in `crates/asides-app/src/icons.rs` (the status item), `docs/favicon.svg` and
  `docs/index.html` (the header and the demo's menu bar): change them together, then run
  `python3 scripts/make-og.py`.
- **The message format:** change `markdown.rs` (and `inspect.rs` for what pins say) along with their
  tests, then the e2e expectations and the README example.
- **A tool or key on the screen:** put its logic in `asides-core/src/overlay.rs` (with a test,
  and an `Action` if a script should drive it). Give a key a default in `OverlayKeys`
  (`config.rs`), then route it in `asides-overlay/src/ui.rs` (`Keys`, `Overlay::command`, or the
  mouse methods) and draw it in `ScreenView::render` (and in `export.rs` if it belongs in the
  image). Add it to the keys sheet, the context menu if it fits, and the README. Check it with a
  replay and look at the captures.
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
