# asides

**Talk through bugs while you test. Get them appended, transcribed on your Mac, to a Markdown note
in the repo you're working on**, with screenshots inline, ready for a colleague, or handed straight to
your coding agent.

```
$ cd ~/code/shop && asides
Asides ready in ~/code/shop
  → appending to "Checkout session" (4 entries)
  ⌃⌥Space open Asides (again: add it all to the aside; hold: talk) · ⌃⌥N into a new note
  ⌃⌥P back to the live screen, or freeze it again · ⌃⌥V attach clipboard image
```

Notice a bug → hold `⌃⌥Space` and say what's wrong → let go → tap `⌃⌥Space` → keep testing. A few
seconds later the transcript is in the note, and the menu-bar icon shows `[✓] #5`.

- **Local and free.** Speech is transcribed by [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
  on the GPU (Metal). No account, no API key, and no audio leaves your Mac.
- **Verbatim.** What you said, as Whisper heard it. No rewriting, no summarizing.
- **Multilingual.** Any language Whisper supports, detected per recording. You can restrict
  detection to the languages you speak.
- **Screenshots in place.** Copy a screenshot while you talk (`⌃⇧⌘4`) and it lands next to the
  sentence you were saying.
- **Context for whoever reads it.** Each new note records the git remote, branch, commit and
  uncommitted changes, plus your macOS version, browser and display.
- **Straight to your agent.** `asides claude` (or Codex, OpenCode, Gemini CLI, Copilot CLI, Qwen
  Code, or New Agent Session in the menu) starts an agent that's already listening: each aside you record goes to it as soon as it's
  transcribed, or in a batch when you send it. Run several, in one repo or many, and switch which
  one you're talking to with `⌃⌥Tab`.
- **One toolbar, over anything.** `⌃⌥Space` freezes your screen, in any app, under one toolbar:
  measure it like with PixelSnap (distances between edges, the size of anything, guides), annotate
  elements and areas, draw arrows and circles, take screenshots of parts, and talk while you do.
  Press play (`⌃⌥P`) to keep using your app as you talk, and freeze it again whenever you like. You
  get one aside with what you said and everything you did, in the order it happened.
- **The loop closes.** Agents acknowledge, resolve, dismiss or reply to each aside, or to each
  annotation in it, as with Agentation's MCP server. You see it in the menu.
- **Wispr Flow too.** If you dictate with Wispr Flow, your dictations can land in the note next to
  your recordings.
- **Native and quiet.** A menu-bar icon and global shortcuts. No Dock icon, no window.

## Requirements

- macOS 14 or later. Developed and tested on macOS 26 with Apple silicon.
- Rust (recent stable), the Xcode Command Line Tools (`xcode-select --install`) and CMake
  (`brew install cmake`). The full Xcode app is not needed.
- About 600 MB of disk space for the speech model, plus 1 GB of RAM while it's loaded. The model
  unloads after 15 idle minutes.

## Install

Releases are signed and notarized by Apple, and run on Apple silicon Macs.

### Homebrew

This repository is also a Homebrew tap:

```bash
brew tap mihaicristianfarcas/asides https://github.com/mihaicristianfarcas/asides
brew install --cask mihaicristianfarcas/asides/asides
```

Homebrew 7 and later only load casks from other taps once you trust them, and installing by the
full name does that. After that, the short name `asides` works.

It installs `Asides.app` in `/Applications` and links the `asides` command into Homebrew's
`bin`. `brew upgrade --cask asides` updates it; the upgrade quits the app, so run `asides`
again afterwards. `brew uninstall --cask asides` removes it, and adding `--zap` also deletes
your settings and the speech model.

### Download

Download the [disk image](https://download.asides.mihaicristian.dev/Asides.dmg) (or
`Asides-<version>.zip` from the [Releases page](https://github.com/mihaicristianfarcas/asides/releases)),
drag `Asides.app` to `/Applications`, and open it. The first time, it links the `asides` command
that ships inside it into `/usr/local/bin`, which is on every shell's `PATH`. If that folder
belongs to the system, as it usually does on Apple silicon, macOS asks for your password. Cancel,
and Asides doesn't ask again; you can then link it yourself:

```bash
sudo ln -s /Applications/Asides.app/Contents/Helpers/asides /usr/local/bin/asides
```

### From source

```bash
git clone https://github.com/mihaicristianfarcas/asides && cd asides
make install   # Asides.app → ~/Applications, plus a link to its CLI in ~/.local/bin
```

With a source build, make sure `~/.local/bin` is on your `PATH`. On first launch, Asides asks for microphone access and downloads the speech model (574 MB,
checksum-verified) to `~/Library/Application Support/Asides/models/`.

`make install` signs both binaries with the hardened runtime, using the best identity in your
keychain: a Developer ID Application certificate, then an Apple Development one, then a
self-signed "Asides Dev" one. Any of these keeps the microphone permission across updates.
Without one, the app is signed ad hoc: macOS treats every rebuild as a new app and asks for the
microphone again. If you don't have an Apple certificate, run `make cert` once. It creates the
self-signed identity in your login keychain and asks for your password.

`make uninstall` removes the app and the command; for a download, delete `Asides.app` and
`/usr/local/bin/asides`. Settings and models stay in `~/Library/Application Support/Asides`.

## Use

### The command

| Command | |
|---|---|
| `asides` | Bind the repo you're in (its git root, or the current directory if it isn't a repo) and start the menu-bar app. Recordings append to the latest note there. |
| `asides claude` | Start Claude Code here, already listening: each aside you record goes to it. `asides codex`, `asides opencode`, `asides gemini`, `asides copilot` and `asides qwen` do the same for those. Arguments go to the agent (`asides claude --model opus`). `--background` (Claude Code, Codex, OpenCode) runs it without a terminal; `--no-worktree` keeps a second session in the repo itself. See [Coding agents](#coding-agents). |
| `asides new [TITLE]` | The next recording starts a new note, titled TITLE or "Untitled". |
| `asides path` | Print the latest note's path, e.g. `claude "fix the bugs in $(asides path)"`. |
| `asides open` | Open the latest note in your Markdown editor. |
| `asides status` | Show what the app is doing, where it's bound, and the agent sessions (which one your asides go to). |
| `asides export [--out FILE]` | Zip the latest note (text, screenshots, audio) to share it. |
| `asides send` | Send to Agent: hand what you've recorded to agents listening in batch mode. |
| `asides mcp` | The MCP server for coding agents. Agents start it themselves; see [Coding agents](#coding-agents). |
| `asides listen` | Print your asides as they reach an agent session. Claude Code runs it in the background while it listens. |
| `asides annotate` | Open Asides with the screen frozen, like `⌃⌥Space`. Run again while it's open, adds it all to the aside. |
| `asides measure IMAGE X,Y` | Measure a screenshot (PNG): the distances from a point to the nearest edges. `X,Y,W,H` finds the object inside an area and the space around it. `--scale 2` adds points for Retina screenshots; `--tolerance N` looks through shadows. |
| `asides quit` | Quit the app, after it finishes any queued transcriptions. |

`-C <dir>` runs any command as if it were started in `<dir>`.

### Shortcuts

| Shortcut | |
|---|---|
| `⌃⌥Space` | Open Asides with the screen frozen (see [below](#asides-on-your-screen)). Held, it talks until you let go, from the moment you press it. While it's open, a tap adds it all to the aside. |
| `⌃⌥N` | The same, into a new note. |
| `⌃⌥P` | Play: back to the live screen, to keep using your app. Again, pause: freeze it to measure and annotate it. Closed, opens Asides frozen, like `⌃⌥Space`. |
| `⌃⌥V` | Attach the image on the clipboard: into the aside while Asides is open, otherwise at the end of the note. |
| `⌃⌥Enter` | Send to Agent. Only registered while agents get asides in batches. |
| `⌃⌥Tab` | Switch which agent session your asides go to (then Notes Only). Only registered while a session is listening. |

Images you copy while you talk are attached automatically. You can change any shortcut in
**Preferences → Edit Shortcuts & Settings** (see [Settings](#settings)).

### The menu

The icon shows what's happening: idle, recording (red, with a timer), transcribing, saved
(`✓ #5`) or a problem. Click it to:

- open Asides (in a new note too), freeze the screen, add it all to the aside or close it without
  adding anything, and attach the clipboard image
- send what you've recorded to your coding agent (in batch mode)
- choose which agent session your asides go to, or Notes Only (**Asides Go To**, once one is
  listening), and open a background session (**Show**)
- start a coding agent (Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI or Qwen Code, the ones
  you have) in this repo, a recent one or another, in a terminal or in the background (**New Agent
  Session**)
- read what agents said about your asides (**Agent Replies**: resolved, dismissed, questions)
- switch between recently used repos, or choose another one (**Directory** submenu)
- open, reveal or copy the path of the latest note
- change preferences: starting to talk when Asides opens, hold to talk, auto-attaching images,
  sounds, keeping audio clips, Wispr Flow dictations, measuring and annotating, the speech model,
  the microphone, how agents get asides, and the app agent sessions open in

The status line shows where your asides go, e.g. `Ready · shop → Claude Code, listening`.
When an agent resolves or replies to an aside, the menu bar says so for a moment (`[✓] #3`, or
`[?] #3` when it asks you something; with the screen frozen, Asides says it on screen), and the
note line shows how many asides are still open.

## Asides on your screen

Press `⌃⌥Space` (or choose **Open Asides**). Every screen freezes, in any app, with one toolbar at
the bottom of the screen you're on: measure, annotate, draw and take screenshots, and talk while
you do. While it's frozen, nothing macOS lets an app hold back gets in the way: the Dock and the
menu bar stay hidden, and switching apps (`⌘Tab`) or spaces, Mission Control, App Exposé and Show
Desktop are off, whether by keys, trackpad gestures or hot corners. (Notification Center,
Spotlight and Force Quit still work; a click on the frozen screen takes it back.) To keep using
your app as you talk, press the toolbar's ▶ (or `⌃⌥P`): the screen goes live, with only the
toolbar over it, and your app has the keyboard again. Pick a tool (or ⏸, or `⌃⌥P` again) to
freeze it again, as often as you like. It brings
[PixelSnap 2](https://pixelsnap.com)'s measuring and [Agentation](https://www.agentation.com)'s
annotations to the whole screen, and everything goes into one aside, in the order you did it:
**Done** (✓, `⌃⌥Space` tapped again, or `⏎` on a frozen screen) adds it, and **×** (or `Esc` on a
frozen screen) closes Asides without adding anything. Over the live screen, Asides takes no keys
but its shortcuts: your app gets everything you type, Esc included.

The toolbar is icons only (hover one for its name). Over the live screen: the four tools (each
freezes the screen), the microphone, Done, close and fold. Frozen, also the tolerance, points or
pixels, undo, copy, `?` and ▶ (back to live). Drag it by its handle (⠿) anywhere on the screen,
and fold it (‹) to just the handle and the microphone; Asides remembers both. On a frozen screen,
hover `?` for every key; the first time, the keys show by themselves. `⇧Tab` hides the frozen
screen's toolbar for good (`measure.toolbar`): everything also has a key, and a small timer stays
while you record.

**Talk** whenever you like: hold `⌃⌥Space` while you speak, anywhere, or press the toolbar's
microphone; on a frozen screen, also hold `Space`, or tap it to start and stop. Talk as many times as you like: it's
all kept for **Done**, which adds what you said and what you did as one aside, in the order it
happened. Each measurement, annotation and screenshot lands in the transcript at the moment you
made it, between what you said before and after it, and each time you talked is a paragraph of its
own; each freeze's screenshot comes just before the first thing you did on it. While you talk, new
annotations don't open their card. To say something about one annotation, hold `Space` in its card,
or use its microphone (below). Asides must be bound to a repo to record and to keep what you do: if
it isn't, the menu bar shows "No repo", and a frozen screen says so at the top.

**Measure** (`D`):

| | |
|---|---|
| Move | The distances from the cursor to the nearest edges, across and down. |
| Click | Hold those distances on screen. `H` or `V` holds just the horizontal or vertical one. |
| Drag | The size of what's inside: the area snaps to the object's bounds. Right-drag (or ⌘ held) keeps it as drawn, `⌥`-drag holds each object inside separately, `⇧` makes it square, and `Space` while dragging moves it. |
| `⇧H` / `⇧V` | A horizontal or vertical guide at the cursor, snapped to a held object's edge or middle, or to the nearest edge. Distances stop at guides and at held objects, and the gaps between guides are shown at the screen's edge. Arrow keys nudge the last guide (⇧: 10 pt). |
| `⇧` held | A crosshair across the whole screen, to check alignment. |
| `-` / `=` | Lower or raise the tolerance: how sharply the color must change to count as an edge (soft shadows, gradients and the blur behind glass never do). `T` switches to a high one that looks through borders and hard shadows; the toolbar cycles Low, Med, High. |
| `U` | Points or pixels. |
| `⌘C` / `C` / `⌘⇧C` | Copy the size (`120 × 32`), the screenshot with everything on it, or everything as Markdown. |
| `R` | Put back the last session's guides and held measurements. |

**Annotate** (`A`): click an element to put a numbered marker on it, or drag over an area. Click
inside a section you measured (an area held with Measure, snapped to an object or drawn by hand)
to annotate that section; an element much smaller than it, under the pointer, is annotated instead.
What a click annotates is outlined in blue. A small card asks what should change:

- **Type it.** Long text wraps, and the card grows to show it all.
- **Or say it.** Hold `Space` in the card while you speak (a tap still types a space), or press its
  microphone (tap, or hold while you speak). What you say is about that annotation: it's
  transcribed right away, shown in the card, and goes in the annotation as `**Said:**`, not in the
  transcript. A trash button appears beside the microphone as soon as you speak; it deletes what
  you said (or are saying), so you can say it again. Speak again without it to add more.
- Its four dots say what kind of request it is (fix, change, question, approve; `⌘1`–`⌘4`).

`⏎`, `Esc` or a click elsewhere closes the card and keeps the annotation and what you gave it; a
click elsewhere also does what it does there, so you can go straight to the next annotation while
the last one's words are still being transcribed. Click a number to open its card again and
change it. To point out a typo, select the text before you freeze the screen, then click it: the
annotation quotes the selection. With **Identify UI
Elements** on (it needs the Accessibility permission), hovering outlines the element under the
cursor, and the annotation names it: its role and label (`button "Save"`), the app and window,
and in browsers the page's URL and a CSS-like location (`form.settings > button#save.btn`).
Without it, annotations name the app and window.

**Draw** (`P`): drag to circle or underline something, `⇧`-drag for an arrow ("move this there").
Drawings are numbered like markers: an arrow names what it starts on and what it points to, a
circle what it surrounds. Say what you mean, or click a drawing's number to type it.

**Shot** (`⌘S` or `S`): drag over a part of the screen. It's copied to the clipboard, with what's
drawn on it, and added to the aside; then you're back to the tool you had. Hover a held object and
click its camera for a screenshot of just that object.

Right-click anywhere for guides, holds, a screenshot, copying and **Annotate Here**; on a measured
section, **Annotate This** annotates the section. `⏎` (**Done**) adds it all to the aside (`⌥⏎`
keeps the whole screen in the last screenshot). `Esc` (or **×**) closes Asides without adding
anything, what you said in it included; with something to lose, it asks for a second `Esc`. When agents get asides in batches, `⌘⏎` (**Send**) also sends it to your agent. `⌫` removes the
annotation or measurement under the pointer (anywhere else, the last thing added), `X` clears
everything, `Tab` hides it all to look at the frozen screen. A recording you started in Asides ends
with it.
Every key can be changed in `[measure.keys]` (see [Settings](#settings)).

```markdown
## 4 · 14:52

![14:52:10](images/04-145210.png)

**In:** Safari · "Settings — Shop" · http://localhost:3000/settings
**Measured** in pt (screen: Built-in Retina Display, 1512 × 982 pt, @2x)
**Guides:** x 380 · x 700 (320 apart)

- Distance from (412, 230): 24 across (x 400 to x 424), 16 down (y 222 to y 238)

The gap under the title is bigger than the one above the fields, it should be the same.

- Area 120 × 32 (15:4) at (380, 200), snapped to the object inside

### 1. button "Save"
**Location:** form.settings > div.actions > button#save.btn.btn-primary
**Intent:** fix
**Feedback:** Line it up with the fields above.
**Said:** And make it the same width as the fields.

And then the heading.

### 2. Arrow from button "Save" to heading "Profile"
**From → to:** (440, 216) → (440, 96)
```

The screenshot comes just before the first thing you did on that screen, with where the annotations
are (`**In:** Safari · "Settings — Shop" · http://localhost:3000/settings`, when they're all in the
same app and page; without annotations, the app and window you were in), the screen and units of
the measurements, and the guides. What you say gets the same line where you start talking, and
again whenever you're somewhere else, so an agent knows which app, window and page you meant even
when there's no screenshot (the page's address needs Identify UI Elements; window titles, the
Screen Recording permission Asides already has to freeze the screen). Then each
measurement and annotation is placed when you made it, among what you said; measurements made one
after another stay one list. An entry made without talking is marked `· Screen`, with the same
blocks in the order you made them. Images added to an aside are at most 1568 pixels on their long
side: agents scale larger ones down anyway.

How much each annotation says is **Preferences → Measure & Annotate → Annotation Detail**
(`annotation_detail`). **Compact** is one line each. **Standard** gives the element, location, page
and feedback. **Detailed** adds its position, size and context. **Forensic** adds its colors and
padding, measured from the screenshot (`background #2563EB, content #FFFFFF, padding 6 12 6 12 pt`),
and its accessibility role and identifier. **Preferences → Start Talking When Asides Opens**
(`measure.record`) starts talking for you.

Freezing the screen needs the **Screen Recording** permission; macOS asks the first time you freeze
it (you may have to quit and reopen Asides after allowing it).

## Notes

```
<repo>/.asides/                      ← added to .git/info/exclude (local only)
  2026-10-15-1432 Checkout session/
    notes.md
    images/02-143851.png
    audio/02-143812.m4a                 ← only with "Keep Audio Clips"
```

```markdown
---
title: Checkout session
created: 2026-10-15T14:32:08+03:00
directory: /Users/me/code/shop
git:
  remote: "git@github.com:acme/shop.git"
  branch: feat/checkout
  commit: a1b2c3d
  commit_subject: Fix cart total rounding
  uncommitted_files: 3
environment:
  macos: 26.5.2
  browser: Firefox 143.0
  display: 3024x1964 @2x
transcription: asides 0.1.0 · whisper large-v3-turbo (local)
---

## 1 · 14:32

The padding on the summary card should be bigger, to compensate for the sticky header.

![14:38:51](images/01-143851.png)

And on mobile the button text wraps.
```

- **Metadata** is written once, when the note is created.
- **A new note starts** when you ask for one (`asides new`, `⌃⌥N`), and on its own when you've
  switched git branch or come back after 8 hours (`auto_new_note`): each note stays about one piece
  of work, easy to share and cheap for an agent to read. `asides` and the menu say so beforehand
  ("next recording starts a new note").
- **Entries** are numbered, so people can refer to "#3".
- **Writes** only ever append, so an editor or agent that has the note open simply reloads it.
- **Sharing:** let your coding agent listen (below), pass it the note's path, or give colleagues the
  zip from `asides export`.

## Coding agents

Start the agent from Asides. In your repo:

```bash
asides claude          # or: asides codex, opencode, gemini, copilot, qwen
```

Or, from the menu, **New Agent Session ▸ Claude Code** (in the repo Asides is bound to, a recent one,
or another): it opens Terminal there, or the app you pick under **Preferences → Open Agent Sessions
In**: Ghostty (a new tab in your Ghostty window), iTerm, or [Diri](https://diri.sh) (a Diri session
named after the agent and repo). The first time a session opens in Ghostty, macOS asks whether
Asides may control Ghostty; if you don't allow it, each session gets a Ghostty window of its own
instead. Either way the agent starts already listening, with Asides' MCP server on its command
line, so there's nothing to set up first, and its asides tools allowed. Each aside you record from
then on goes to it as a prompt; you can still type to it as usual. Your own arguments go along
(`asides claude --model opus`), and `[agents]` in the settings can add some to every session.
Agents ask before editing or running things as they normally do. Gemini CLI can't be given a
server on its command line: the first `asides gemini` adds Asides' to its settings for you
(`gemini mcp add --scope user --trust asides asides mcp`), and every session uses it as its own.

### Several sessions

Each aside goes to one session: the one you're talking to, the target. A new session becomes the
target. **Asides Go To** in the menu lists the sessions, what each is doing (starting, listening,
working), and **Notes Only**, which saves your asides without handing them to an agent; `⌃⌥Tab`
switches to the next. While you record, the menu bar shows where it's going (`0:04 → Codex`).

- **In different repos:** start one in each. Switching to a session also switches the repo your
  notes go to.
- **In the same repo:** a second session works in a git worktree of the repo, so the two don't edit
  the same files. It branches from the commit you're on, so it has your branch's work (not what you
  haven't committed). Claude Code makes the worktree, in `.claude/worktrees/`; for Codex and
  OpenCode, Asides makes it in `.asides/worktrees/`, and removes it after the session if nothing was
  done in it. Your notes stay
  in the repo. `--no-worktree` (or `worktrees = false`) keeps the session in the repo itself.
- **In the background:** **Claude Code in the Background**, **Codex in the Background** or
  **OpenCode in the Background** (or `asides claude --background`, and the same for `codex` and
  `opencode`) starts a session without a terminal; **Show** in the menu opens it (`claude attach`,
  `codex resume`, `opencode --session`), and **Stop** ends it.

When the session you're talking to ends, your asides go to the newest other session in that repo,
or to the notes only. Every aside is still written to the note, whichever session gets it.

### An agent you started yourself

`asides mcp` is an [MCP](https://modelcontextprotocol.io) server. Add it to your agent once:

```bash
claude mcp add --scope user asides -- asides mcp      # Claude Code
codex mcp add asides -- asides mcp                    # Codex
opencode mcp add --global asides -- asides mcp        # OpenCode
gemini mcp add --scope user --trust asides asides mcp # Gemini CLI
copilot mcp add asides -- asides mcp                  # Copilot CLI
qwen mcp add --scope user --trust asides asides mcp   # Qwen Code
amp mcp add asides -- asides mcp                      # Amp
```

Editors and other agents take the same server in their MCP settings: Cursor (`~/.cursor/mcp.json`),
Windsurf, VS Code, Zed, Goose, Cline, Kiro, Warp… In the usual JSON:

```json
{ "mcpServers": { "asides": { "command": "asides", "args": ["mcp"] } } }
```

Then, in a session in your repo, ask the agent to **listen to your asides** (in Claude Code,
`/asides:listen` says the same). It acts on each aside as a prompt, until you tell it to stop.
Listening makes it a session like the others, and the target: Asides starts if it isn't running,
and binds to the agent's repo.

How the agent gets your asides depends on what it can do:

- **Claude Code** runs `asides listen` in the background, with its Monitor tool. Each aside shows
  up in the chat as it reaches the agent, and the agent acts on it. In between, the agent is free:
  you can ask it other things.
- **Codex, started by Asides,** gets each aside as a message, like one you typed, with its
  screenshots. Asides starts the session as a thread in Codex's app server (the daemon Codex keeps
  running; Asides starts it if it isn't), hands it each aside, and the window opens that thread
  (`codex resume`). Codex never
  sits waiting for asides, and one that arrives while it works is queued for when it's done. If an
  aside can't reach it, the menu says so, and Asides keeps trying. Without the app server (older
  Codex), it waits like the agents below.
- **OpenCode, started by Asides,** gets each aside as a message too. OpenCode 2 runs its sessions
  in a background service; Asides starts the session there, with Asides' tools for it, adds each
  aside as the session's next message, and the terminal opens it (`opencode --session`). OpenCode 1
  waits like the agents below.
- **Gemini CLI, Copilot CLI and Qwen Code, started by Asides,** wait in a tool call
  (`wait_for_asides`), with their time limit for one raised to an hour, so they wait long between
  asides.
- **Agents you started yourself** wait in a tool call (`wait_for_asides`), act on what
  it returns, then wait again. Asides that arrive while the agent works wait for it. Most agents hide
  tool results, so the agent first lists the asides it got, one line each.

### Each aside, or in batches

How asides reach the agent is a preference (**Preferences → Agents Get Asides**, or
`agent_delivery`):

- **Each aside as it's transcribed** (`each`, the default): every recording becomes a prompt a few
  seconds after you stop talking.
- **In batches** (`batch`): recordings wait until you choose **Send to Agent** (`⌃⌥Enter`, the menu,
  or `asides send`), which hands everything recorded since the last send to the session you're
  talking to, including recordings made before it started listening. A recording in progress is
  stopped and included.

The menu shows where things stand. Its status line says `→ Claude Code, listening` while the agent
waits for asides, and `working` while it acts on them. Send to Agent shows **Sent** when the agent is
waiting, and **Queued** when it's busy: it gets the batch when it's done. With no agent session to
send to, nothing is sent; the menu says so, and the batch waits for your next Send to Agent.

The agent gets the transcript, the note's path and your screenshots (as images through
`wait_for_asides`, as file paths through both). It can also read the whole latest note on request
("work through my asides", or `/asides:asides` in Claude Code); asides you spoke to another session
are marked as such.

### Closing the loop

As with Agentation's MCP server, the agent tells you what it did with each aside. An aside is `#3`
(entry 3) or `#3.2` (annotation 2 of an entry with annotations).

| Tool | |
|---|---|
| `acknowledge_aside` | Seen it, working on it. |
| `resolve_aside` | Done, with a summary of what it did. |
| `dismiss_aside` | Won't act on it, and why. |
| `reply_to_aside` | A question or an update. Answer by recording another aside. |
| `get_pending_asides` | What's still open (pending or acknowledged), with the replies so far. In Claude Code, `/asides:open` works through them. |
| `list_asides_notes` | The notes in the repo, with how many asides are open. |
| `measure_image` | Measure a screenshot like Asides does on a frozen screen: distances between edges, the bounds of an object, padding and colors. Handy for checking its own work. |

These go to `threads.jsonl` beside the note. The note itself is never rewritten. You see them under
**Agent Replies** in the menu, and the agent sees them under the aside the next time it reads the
note:

```markdown
> #3
> **Claude Code** (resolved): Added 16px padding to the summary card in Summary.tsx
```

Codex asks before each call to these tools, since they aren't read-only.

Agents give up on a tool call after a while: Codex and most others after 60 s, Claude Code much
later. So the agent's wait ends after 50 s (25 min in Claude Code), and the agent calls again.
Codex sessions started by `asides codex` wait 50 minutes at a time. To wait longer in a Codex you
started yourself, raise its `tool_timeout_sec` for the server and pass the same wait to asides:
`codex mcp add asides -- asides mcp --timeout 3000`, then set `tool_timeout_sec = 3600` under
`[mcp_servers.asides]` in `~/.codex/config.toml`. Claude Code's monitors last at most 30 minutes,
so `asides listen` stops after 29 and prints the command that carries on where it stopped; the
agent runs it, and nothing recorded in between is lost.

## Wispr Flow

If you dictate with [Wispr Flow](https://wisprflow.ai), turn on **Preferences → Add Wispr Flow
Dictations** (`wispr_flow = true`). Each dictation you make while it's on becomes an entry in the
latest note, marked with its source:

```markdown
## 6 · 14:41 · Wispr Flow

The coupon field accepts expired codes.
```

- Asides reads Wispr Flow's history on your Mac (`~/Library/Application Support/Wispr Flow/flow.sqlite`),
  read-only, with the system's `sqlite3`. Wispr Flow must keep history on this Mac.
- Only dictations made after you turn it on are added. List apps in `wispr_flow_apps` (for example
  `["Safari", "Google Chrome"]`) to add only what you dictate there, so your messages elsewhere stay
  out of the note.
- The text is what Wispr Flow typed, formatting included; Asides doesn't change it. Wispr Flow
  transcribes in its own way, not on your Mac with Whisper.
- To attach a screenshot, press `⌃⌥V` after the dictation.

## Settings

`~/Library/Application Support/Asides/config.toml` (Preferences → Edit Shortcuts & Settings…).
Changes apply as soon as you save.

```toml
hold_to_talk = true          # holding the record shortcut records until release; a tap toggles
auto_attach_images = true    # attach images copied while recording
sounds = true
keep_audio = false           # keep an .m4a of each recording next to the note
model = "large-v3-turbo"     # or "large-v3" (more accurate, slower) or "small" (faster, less accurate)
input_device = ""            # microphone name; empty means the system default
languages = []               # e.g. ["en", "ro"] to choose only between these; [] means any
agent_delivery = "each"      # or "batch": agents get asides when you choose Send to Agent
auto_new_note = true         # a new note after switching git branch, or after 8 hours away
wispr_flow = false           # add Wispr Flow dictations to the note
wispr_flow_apps = []         # e.g. ["Safari"] to add only dictations made there; [] means any app
annotation_detail = "standard"  # "compact", "standard", "detailed" or "forensic"

[agents]                     # sessions started by `asides claude` and New Agent Session
terminal = ""                # the app they open in: "" (Terminal), "Ghostty", "iTerm", "Diri", or another
worktrees = true             # a second session in a repo works in a git worktree of it
claude = []                  # extra arguments for each agent, e.g. ["--model", "opus"]
codex = []
opencode = []
gemini = []
copilot = []
qwen = []

[measure]
color = "#FF2D55"            # lines and labels
units = "pt"                 # or "px"
tolerance = 2                # % the color must change by, pixel to pixel, to count as an edge
identify_elements = false    # name what's under each annotation (needs Accessibility)
padding = 48.0               # points of screen kept around what you measured, in the screenshot
record = false               # start talking when Asides opens
toolbar = true               # show its toolbar (⇧Tab hides and shows it)

[measure.keys]               # the overlay's keys; several separated by commas, "" for none
measure = "d"
annotate = "a"
draw = "p"
shot = "cmd+s, s"
hold_horizontal = "h"
hold_vertical = "v"
guide_horizontal = "shift+h"
guide_vertical = "shift+v"
tolerance_up = "=, shift+="
tolerance_down = "-"
tolerance_high = "t"
units = "u"
copy_dimensions = "cmd+c"
copy_image = "c"
copy_markdown = "cmd+shift+c"
undo = "backspace, cmd+z"
clear = "x"
hide = "tab"
toolbar = "shift+tab"
restore = "r"
talk = "space"               # hold to talk, tap to toggle
help = "shift+/, f1"
done = "enter"
done_whole_screen = "alt+enter"
send = "cmd+enter"
live = ""                    # back to the live screen (⌃⌥P does it from anywhere)
close = "escape"             # close without adding anything (again, with something to lose)

[shortcuts]
asides = "ctrl+alt+Space"          # open Asides; while open, tap: add it all; hold: talk
new_note = "ctrl+alt+KeyN"         # the same, into a new note
attach_image = "ctrl+alt+KeyV"
send_to_agent = "ctrl+alt+Enter"   # only registered with agent_delivery = "batch"
freeze = "ctrl+alt+KeyP"           # pause (freeze the screen) or play (back to live)
next_target = "ctrl+alt+Tab"       # switch agent session; only registered while one is listening
```

An empty shortcut (`""`) turns that shortcut off.

If you speak only a few languages, list them in `languages`. Detection on short clips is then much
more reliable; a single entry skips detection altogether. Whisper writes in the language it
detects, so a misdetected clip comes out in another language: when it isn't sure of a clip's
language (often a short one, or an accent), Asides keeps the language it was last sure of.

## Privacy and permissions

- **Audio** stays on your Mac. It's held in memory until it's transcribed, then discarded, unless
  you turn on "Keep Audio Clips".
- **The network** is used by Asides for one thing only: downloading the model from Hugging Face the
  first time (or when you switch models).
- **Agents** talk to `asides mcp` on your Mac, over standard input and output. It reads your notes
  and passes them to the agent, which may send them on to its model provider, like anything else you
  give it. New Agent Session opens your terminal with a script, as if you had double-clicked it.
  The agents run with their own permission settings.
- **Wispr Flow** transcribes with its own service, under its own terms. Asides only reads the
  history Wispr Flow keeps on your Mac, and only when you turn the integration on.
- **Microphone** access is required.
- **Administrator password:** asked once, if you installed Asides by hand and `/usr/local/bin`
  belongs to the system, to link the `asides` command there. Cancel to skip it.
- **Clipboard:** macOS may ask once whether Asides may paste from other apps, when it reads a
  screenshot from the clipboard. Choose Allow.
- **Screen Recording** is asked for the first time you freeze the screen, which Asides does with a
  screenshot. Screenshots stay on your Mac, in memory, until you add one to an aside.
  Recording alone never needs it.
- **Automation** (controlling Ghostty) is asked for the first time New Agent Session opens a session
  in Ghostty while it's running, to open it as a tab in your window. Asides only asks Ghostty for
  that tab. Say no and sessions open in a window of their own.
- **Accessibility** is only asked for if you turn on **Identify UI Elements**. Asides then asks the
  app under an annotation (or under the cursor, with Annotate) what's there, and nothing else.
- **Input Monitoring** is not needed: the shortcuts use the system hot-key API, and Measure &
  Annotate only sees keys while its own windows are in front. Its overlay (`asides-overlay`, inside
  the app) gets the screenshots from the app, and asks the app what's under an annotation; it needs no
  permission of its own.

## Troubleshooting

- **macOS asks for the microphone after every update.** The app is signed ad hoc. Run `make cert`
  once, or install an Apple Development or Developer ID certificate (see Install).
- **A shortcut does nothing.** Another app may already use it. Pick another in `config.toml`.
  macOS refuses shortcuts whose only modifier is Option, or Option and Shift.
- **"Nothing heard."** Check Preferences → Microphone. Bluetooth headsets take about a second to
  switch to their microphone; start talking after the start sound.
- **The text came out in the wrong language.** Set `languages` in `config.toml`.
- **The agent doesn't get my asides.** Check the status line in the menu: it says which session your
  asides go to, and whether it's listening or working on your last asides. If it names another
  session, switch with `⌃⌥Tab` or Asides Go To; if it says notes only, start one (New Agent Session)
  or ask your agent to listen to your asides. In batch mode, choose Send to Agent; it says whether
  the batch was sent, queued, or had no session to go to.
- **New Agent Session opens the wrong app, or none.** Pick one under Preferences → Open Agent
  Sessions In, or set `terminal` under `[agents]` to an app's name, as `open -a` takes it (it must
  run `.command` scripts). The agent must be installed for your login shell (`which claude` in a
  terminal).
- **Wispr Flow dictations don't show up.** Check that Wispr Flow keeps history on this Mac, and that
  the app you dictate in matches `wispr_flow_apps`.
- **Asides says it can't see the screen when it freezes.** Allow Asides in System Settings → Privacy &
  Security → Screen & System Audio Recording, then quit and reopen Asides.
- **Distances stop at the wrong place, or go through an edge.** Change the tolerance with `-` and
  `=`: lower it to see faint borders (a light card on white), raise it (or press `T`) to look through
  borders and hard shadows. Soft shadows, gradients, what glass blurs behind it and changes too
  faint to see never stop them.
- **Annotations only name the app.** Turn on Identify UI Elements and allow Asides under
  Accessibility. Chrome and Electron apps build their accessibility tree on the first query, so
  the first annotation in them may only name the window.
- **A key does something else in the overlay.** Your keyboard layout may put it elsewhere; change it
  in `[measure.keys]`.
- **The menu bar shows empty brackets.** Asides isn't bound to a directory (or it was deleted): run `asides`
  in your project, or pick one under Directory. Until then it won't record.

## How it works

- **Two parts.** A menu-bar app, `Asides.app`, does the recording and transcription. The
  `asides` command tells it which repo to use, and which agent session your asides go to, through
  `asides://` links opened with `open -g`, and reads the state the app publishes in `state.json`. The app is never started as a
  child of your terminal: it keeps running when the terminal closes, and macOS asks for the
  microphone on behalf of Asides rather than your terminal.
- **Recording.** Audio is captured with CoreAudio (cpal), resampled to 16 kHz, and transcribed in
  one pass after you stop. With large-v3-turbo on an M4 Pro, 18 s of speech takes about 1 s and
  95 s about 4 s, in the background while you keep testing.
- **Whisper over Apple's SpeechAnalyzer.** Apple's on-device recognizer is faster and needs no
  download, but it lacks many languages (Romanian, for one) and it ignored vocabulary hints in our
  tests.
- **No prompt.** Whisper is not given a vocabulary or prompt. A prompt helps it spell jargon, but it
  also changes what you said: in testing it silently dropped a word.
- **One language per recording.** Whisper detects the language once per recording. If you switch
  languages partway through, it can drop the part in the other language.
- **Measuring.** ScreenCaptureKit takes one sRGB screenshot per screen, so colors read like the CSS
  that drew them. Edges are where the color changes sharply, from one pixel to the next (or over two
  anti-aliased pixels) by more than the tolerance, visibly (compared as the eye sees colors, in
  OKLab), and within a few pixels; slow changes aren't edges, however far they go, so measuring works
  on glass and frosted surfaces, over gradients and through soft shadows, and on colors only the
  display can show. An area
  snaps to the bounds of what's drawn inside it: whatever its outline can't reach without crossing
  an edge. The same Rust code measures in the overlay, in `asides measure` and in `measure_image`.
- **Sessions.** Each agent session has a small record in `~/Library/Application Support/Asides/sessions/`.
  The app writes which session each aside went to in `routes.jsonl` beside the note, just before the
  aside itself; each agent's `asides mcp` or `asides listen` hands it only its own.
- **The overlay.** Asides' toolbar and frozen screens are drawn with [GPUI](https://gpui.rs), Zed's
  GPU-accelerated UI framework, in a process of its own (`asides-overlay`), since GPUI runs its own
  event loop. The app freezes the screens, hands it their pixels, answers what's under an
  annotation, records you, and writes what it returns into the note.

## Contributing

Contributions are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) covers setup, the development loop,
tests (including a manual QA checklist) and pull requests. [ARCHITECTURE.md](ARCHITECTURE.md)
explains how the CLI, the app and its threads fit together, and the invariants to keep. Coding
agents follow [AGENTS.md](AGENTS.md); Claude Code loads it through `CLAUDE.md`.

```bash
make test     # unit tests
make e2e      # end-to-end through the CLI (run `make fixtures` once first)
```

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT),
at your option. Unless you explicitly state otherwise, any contribution intentionally submitted for
inclusion in this project by you, as defined in the Apache-2.0 license, shall be dual licensed as
above, without any additional terms or conditions.

The Whisper models are released by OpenAI under the MIT license; the ggml conversions are
downloaded from [ggerganov/whisper.cpp on Hugging Face](https://huggingface.co/ggerganov/whisper.cpp).
