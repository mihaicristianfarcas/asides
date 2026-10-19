# Contributing

Fixes, docs and features are welcome. For anything bigger than a fix, open an issue first. Read
[ARCHITECTURE.md](ARCHITECTURE.md) before changing the app.

## Set up

macOS 14+, recent stable Rust, the Xcode Command Line Tools and CMake (`brew install cmake`).

```bash
git clone https://github.com/mihaicristianfarcas/asides && cd asides
make cert        # once, only without an Apple Development or Developer ID certificate
make install     # build, sign and install asides.app and the asides command
make fixtures    # once: test audio from the macOS voices
```

- `make install` signs with the best identity it finds (`scripts/signing-identity.sh`). A stable
  identity keeps the microphone permission across rebuilds; ad hoc, macOS asks every time.
  `make cert` creates a self-signed one in your login keychain (it asks for your password).
- `make fixtures` needs the voice "Ioana" (System Settings → Accessibility → Spoken Content →
  Manage Voices…). The files are gitignored: macOS voices may not be redistributed.
- `make test-speech` and `make e2e` need large-v3-turbo: `asides download large-v3-turbo`.

## Before a pull request

```bash
cargo fmt --all
cargo clippy --workspace --all-targets --features asides-app/dev-hooks -- -D warnings
make test
make e2e          # if CLI or app behavior changed
```

CI runs formatting, clippy, the unit tests and the bundle. `main` only takes pull requests that
pass it.

## Development loop

| You changed | Check it with |
|---|---|
| `asides-core` | `cargo test -p asides-core` |
| the CLI | `cargo run -p asides-cli -- status` (and the rest) |
| `asides mcp` | JSON-RPC lines into `cargo run -p asides-cli -- mcp` (below), then in an agent |
| finding sessions | `asides sessions`, or `cargo test -p asides-core discovers -- --ignored --nocapture` |
| sending | a dev-hooks build (`make install FEATURES=dev-hooks`), an agent of your own in a scratch repo, `asides simulate <wav> --image <png> --to <pid>`; read the agent's transcript |
| the asides window | `ASIDES_WINDOW_CAPTURE=<dir> target/debug/asides-overlay --window` captures every page, dark and light (a few seconds on screen); `HOME` (settings of your own), `ASIDES_HOME` (a history of your own), `ASIDES_WINDOW_PAGE`, `ASIDES_WINDOW_ASIDE`, `ASIDES_WINDOW_APPEARANCE`, `ASIDES_WINDOW_OPEN` (a select's id, opened), `ASIDES_WINDOW_FOCUS` (a key recording, or a field focused, for posted keys) |
| the overlay | a replay (below) |
| the app | `make install`, then use it (it quits and replaces the running asides) |

```bash
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"me","version":"1"}}}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"get_aside","arguments":{}}}' \
  | cargo run -q -p asides-cli -- mcp
```

- **The app's output** is discarded when launched normally. To see it: `asides quit`, then run
  `~/Applications/asides.app/Contents/MacOS/asides` (macOS may then ask for the microphone for your
  terminal).
- **Real agents** cost a model call: start yours in a scratch repo with a first message that keeps
  it small ("reply in one line, don't run tools"). Claude Code asks to trust a new folder first.
  Start terminals from the Dock: one launched from inside an agent's session hands its markers
  (`CLAUDE_CODE_CHILD_SESSION`…) to every shell, and Claude Code then stops saving transcripts.
- **A release installed too** has the same bundle ID. The CLI talks to the app it ships in (or
  `ASIDES_APP`); opened from Spotlight, either may start. Quit the one you're not testing.
- **Updates** are off in builds that aren't notarized (`make install`'s). To try one on a copy of a
  release: `ASIDES_UPDATE_FEED=file:///…/appcast.json` points it at a feed of your own, and
  `ASIDES_UPDATE_UNNOTARIZED=1` lets a build of yours update (to a notarized release only).

## Tests

| Layer | Command | Covers |
|---|---|---|
| Unit | `make test` | history, sessions, delivery plans, messages, links, config, measuring, the overlay session, its rendering, the MCP server, updates (the feed, `Flow`, the swap script) |
| Speech | `make test-speech` | transcribing the fixtures, a real model download |
| End-to-end | `make e2e` | a dev-hooks build driven through the CLI with a history of its own: recordings, scripted sessions, Wispr Flow, sending to a stand-in agent (Diri, else Terminal), `asides mcp`. Restores your config and clipboard. |
| Overlay | replays (below) | the real windows, driven by AppKit events, captured to PNGs |
| Manual | checklist (below) | what a script can't do |

**Voice models.** Recordings (`<name>.wav`, 16 kHz mono) beside what was said (`<name>.txt`):

```bash
ASIDES_BENCH=~/bench ASIDES_BENCH_MODELS="small.en=$M/ggml-small.en-q5_1.bin,large-v3-turbo=$M/ggml-large-v3-turbo-q5_0.bin" \
  cargo test --release -p asides-app compares_models -- --ignored --nocapture
```

**Images.** Each writes files to look at:

| Command | Writes |
|---|---|
| `ASIDES_OVERLAY_DUMP=/tmp/o cargo test -p asides-overlay renders_annotations` | `session.png`, the image an aside gets |
| `ASIDES_CAPTURE_DUMP=/tmp/f cargo test -p asides-app dumps_a_freeze -- --ignored` | a freeze's pixels in sRGB (RGBA; needs Screen Recording) |
| `ASIDES_ICON_DUMP=/tmp/i cargo test -p asides-app icons` | the menu-bar icons (RGBA) |
| `asides simulate-measure shot.png script.json` | a scripted session on a screenshot (dev-hooks build; actions in `overlay::Action`) |
| `python3 scripts/make-screenshot.py` | `guide/overlay.png`, the README's picture (Firefox, Pillow, Screen Recording) |

**Replays.** `target/debug/asides-overlay <dir>` runs the real overlay on a session directory:
`session.json` (`helper::Spec`) with a screen whose `pixels` file is raw RGBA, `windowed` for a
small window, and `replay` with mouse and key steps and `capture`s (see `replay.rs`;
`scripts/make-screenshot.py` is an example; `"toolbar": true` for the pill's window). Replays send events straight to GPUI's view; to check
the real input path, run the overlay through the app (`launchctl setenv ASIDES_OVERLAY <binary>`)
and create `asides-overlay-debug` in `getconf DARWIN_USER_TEMP_DIR`: keys and focus changes are
logged to `asides-overlay-debug.log` there.

## Manual QA checklist

Run what your change touches, and say in the pull request what you tried.

- [ ] **First launch** after `tccutil reset Microphone io.github.mihaicristianfarcas.asides`: the
  prompt names asides; nothing downloads.
- [ ] **Menu bar**: the mark alone (dot recording, dots busy, `[n]` sent, `!` error); the menu has
  the status, Open asides, Open Live, Copy the Last Aside, three switches, History…, Settings…, Quit.
- [ ] **Opening**: `⌃⌥Space` freezes every screen at once, the pill below the menu bar and notch,
  naming the session; the mic red with a model, crossed out (click downloads) without.
- [ ] **Pins**: click pins, drag pins an area, a pin's number opens its card, `⏎` closes it then
  sends; in a card Space types, held it talks.
- [ ] **Pill**: `Tab` switches the session, the toast under the pill without a jump; pointing at the
  name opens its list (it closes as the pointer leaves), showing sessions (unreachable dimmed), New
  session in… (Choose Folder… above everything, frozen and live) and History only; dragging moves
  it and it stays; `M`, `D`, `S` switch tools.
- [ ] **Mic**: click mutes and unmutes; muted, Space held talks; the setting off opens muted.
- [ ] **Live**: `⌃⌥P` leaves the pill only, unmoved, and the app takes keys and clicks; again
  freezes (no pill in the screenshot); `⌃⌥Space` sends from live; closed, `⌃⌥P` opens live.
- [ ] **Glass**: every surface shows what's behind it blurred, over light and dark.
- [ ] **Sending**, each of Claude Code, Codex, OpenCode in each of Terminal, Ghostty, Diri: the
  Automation prompt the first time; the message with screenshots attached where taken; queued
  while the agent works; the clipboard restored; a gone session reported, the aside in History.
- [ ] **New session in…** in Terminal, Ghostty and Diri, in bash, zsh and fish.
- [ ] **Closing**: `Esc` (twice with something done) or × sends nothing; the previous app gets the
  keyboard back with no gray frame.
- [ ] **asides window**: History actions; Sessions matches `asides sessions`; every setting writes
  `config.toml` and takes effect; light and dark.
- [ ] **Wispr Flow**: a dictation while frozen joins the aside; elsewhere, nothing.
- [ ] **Kiosk**: while frozen, no Dock, menu bar, `⌘Tab`, Mission Control or spaces; Force Quit
  works.
- [ ] **Measuring**: distances, holds, guides, snapping, tolerance, units, copying, `R`.
- [ ] **Several displays**: each freezes; the pill opens on the one with the pointer.
- [ ] **Frame by frame** (`screencapture -v`): no stale frame or flash opening, toggling, closing.
- [ ] **Updates** (a release installed, a newer one published): the sidebar offers it; **Restart
  to update** reopens asides on it, with its window if it was open, and **What's new**.

## Common changes

- **A setting**: a field in `Config` (`config.rs`); read it in the app; add it
  to its page in the window (`window.rs`) and [guide/settings.md](guide/settings.md).
- **A menu item**: keep the menu short. A `MenuAction` in `tray.rs`, handled in `App::on_menu`;
  escape `&` in dynamic text with `menu_text`.
- **A CLI command**: a `Command` in `asides-cli/src/main.rs`; if it needs the app, a `Link` in
  `link.rs` (with a round-trip test) handled in `App::handle_link`; replies through `state.json`
  or the history.
- **An MCP tool**: `tools()` and `call_tool` in `mcp.rs`. Read-only (`readOnlyHint`), stdout for
  JSON-RPC only. Cover it in the e2e test.
- **An agent or terminal app**: an agent is a row in `agents.rs`; an app is a `Host` in
  `sessions.rs` and a plan in `deliver.rs`. Measure what the agent records (multi-line, while busy,
  images by Ctrl+V and by path) before writing the plan, and note the results in `deliver.rs`'s
  module docs.
- **A voice model**: `MODELS` in `models.rs`, with the exact size and SHA-256 from Hugging Face.
- **The icon**: `swift scripts/make-icon.swift` writes `packaging/asides.icns`. The same mark is in
  `icons.rs`, `docs/favicon.svg` and `docs/index.html`: change them together, then
  `python3 scripts/make-og.py`.
- **The message format**: `markdown.rs` and `inspect.rs` with their tests, the e2e expectations and
  the example in [guide/using.md](guide/using.md).
- **A tool or key on the screen**: logic in `asides-core/src/overlay.rs` (with a test, and an
  `Action` for scripts); a default in `OverlayKeys`; routed in `asides-overlay/src/ui.rs`, drawn in
  `ScreenView::render` (and `export.rs` if it's in the image). Add it to the keys sheet and
  [guide/using.md](guide/using.md). Check it with a replay.
- **Something the overlay asks the app**: a `helper::Event` (and a `Command` for the answer),
  handled in `App::on_overlay`. The overlay never needs a permission itself.

## Style

- `rustfmt` (120 columns) and clippy with warnings as errors.
- Logic that doesn't need macOS goes in `asides-core`, where it's unit-tested.
- Comments explain why. Match the surrounding code.
- Errors reach users: say what happened and what to do. No `unwrap()` on user data, files or the
  system.
- Add dependencies sparingly.

## Commits and pull requests

Focused pull requests; each commit builds and passes tests. Imperative subjects ("Add …", "Fix …")
under about 72 characters, and a body that explains why. Fill in the pull request template.

## Releasing (maintainers)

Needs a Developer ID Application certificate.

1. Once: create an app-specific password at [account.apple.com](https://account.apple.com), then
   `xcrun notarytool store-credentials asides-notary --apple-id <Apple ID> --team-id <Team ID>`
   (another name: `NOTARY_PROFILE`).
2. Set the version in `Cargo.toml` (`[workspace.package]`), and write the release notes in
   `dist/notes-<version>.md` (Markdown, what changes for people: the app's What's New shows them).
3. `make release`: builds, signs, notarizes and staples; writes `dist/asides-<version>.zip`,
   `.dmg` and `appcast.json` (the update feed), and the version and checksum into `Casks/asides.rb`.
4. `make publish-dmg`: uploads the disk image to the download link (Wrangler, logged in to
   Cloudflare).
5. `git tag v<version> && git push --tags`, then `gh release create v<version>
   dist/asides-<version>.zip dist/asides-<version>.dmg dist/appcast.json --notes-file
   dist/notes-<version>.md` (`make release` prints it). Installed copies update from the latest
   release's `appcast.json`, so it goes up with the zip it names.
6. Commit and push `Casks/asides.rb` (only after the release exists: the cask downloads from it).

## The website

`docs/` is https://asides.mihaicristian.dev: one static `index.html`, no build step
(`python3 -m http.server -d docs` to preview). Pushes to `main` that change `docs/` deploy it;
`make publish-site` deploys by hand. After changing the demo, run `python3 scripts/make-og.py`
(the share image, needs Firefox) and `python3 scripts/make-screenshot.py`. The page repeats the
shortcuts, requirements and install steps from the README: update both.

## Reporting bugs

Use the bug report template, with your macOS version, `asides --version`, `asides status` and the
steps. asides can contain private things: share only what's relevant.

By contributing, you agree your contributions are dual licensed under MIT and Apache-2.0.
