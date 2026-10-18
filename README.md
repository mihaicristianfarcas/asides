# asides

**Freeze your screen, click what's wrong, say why. It's in your coding agent's chat**, typed there as
your own message, with the screenshot, the element you clicked and what you measured. Transcribed
on your Mac.

```
⌃⌥Space   the screen freezes; a small pill shows which agent session it goes to
click     a numbered pin on the element (a drag: an area); talk while you do
⌃⌥Space   it's sent, as one message, to the session you last typed into
```

That's faster than a screenshot, paste and six typed words: you don't type, and the agent gets the
element's name, its location in the page and its size, not just pixels.

- **Into the session you're already in.** Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI or
  Qwen Code, running in Terminal, Ghostty or [Diri](https://diri.sh): Asides finds them, sends to
  the one you typed into last, and types the aside in as you would. It shows in the chat as your
  message. Nothing to install in the agents, nothing that has to listen. Pick another session on
  the pill, or start a new one in any folder with the aside as its first message.
- **For anything on screen.** Pins name what's under them (`button "Save"`, the app, the window,
  and in browsers the page and a CSS-like location). Measure like with PixelSnap (distances
  between edges, the size of anything, guides), draw arrows and circles, take screenshots of parts.
  Press `⌃⌥P` to use your app again and freeze it again whenever you like: one aside holds every
  freeze, in the order you did things.
- **Local and free.** Speech is transcribed by [whisper.cpp](https://github.com/ggml-org/whisper.cpp)
  on the GPU (Metal). No account, no API key, and no audio leaves your Mac. **Verbatim**: what you
  said, as Whisper heard it.
- **A history, not files in your repo.** Every aside is kept in `~/.asides/history` (30 days by
  default): send it again, copy it, or let your agent read it (`asides mcp`).
- **Native and quiet.** A menu-bar mark, two global shortcuts, and a window for history and
  settings when you want it.

## Requirements

- macOS 14 or later. Developed and tested on macOS 26 with Apple silicon.
- To talk: a voice model, downloaded when you first click the microphone (190 MB for English,
  574 MB for any language). Asides works without one: pins, measurements and screenshots.
- To build it: Rust (recent stable), the Xcode Command Line Tools (`xcode-select --install`) and
  CMake (`brew install cmake`).

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
`bin`. `brew upgrade --cask asides` updates it (the upgrade quits the app: open it again
afterwards). `brew uninstall --cask asides` removes it, and adding `--zap` also deletes your
settings, the voice model and the history.

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

With a source build, make sure `~/.local/bin` is on your `PATH`.

`make install` signs both binaries with the hardened runtime, using the best identity in your
keychain: a Developer ID Application certificate, then an Apple Development one, then a
self-signed "Asides Dev" one. Any of these keeps the microphone permission across updates.
Without one, the app is signed ad hoc: macOS treats every rebuild as a new app and asks for the
microphone again. If you don't have an Apple certificate, run `make cert` once. It creates the
self-signed identity in your login keychain and asks for your password.

`make uninstall` removes the app and the command; for a download, delete `Asides.app` and
`/usr/local/bin/asides`. Settings and models stay in `~/Library/Application Support/Asides`, the
history in `~/.asides`.

## Use

Open Asides (from Applications, or `asides`). It lives in the menu bar: its mark, `[a]`, says
what's going on (a dot while it listens, dots while it transcribes or sends, `[3]` for the third
aside sent today, `!` when something needs you), and its menu has **Open Asides**, the **Asides
window** and **Quit**.

Start your agent as usual (`claude`, `codex`, `opencode`…) in Terminal, Ghostty or Diri, in your
project. Then, in the app you're testing:

1. **Press `⌃⌥Space`.** Every screen freezes. At the top, the pill: the microphone (listening, red,
   with the time; or muted), where the aside goes, send (✓) and close (×).
2. **Click** what's wrong: a numbered pin on the element under the pointer (outlined in blue before
   you click). **Drag** over an area to pin the area. **Talk**: what you say goes between the pins,
   in the order it happened.
3. **Press `⌃⌥Space` again** (or `⏎`, or ✓). Asides closes at once; a moment later the aside is in
   your agent's chat, as your message, with each freeze's screenshot.

`Esc` (or ×) closes without sending anything (with something to lose, press it twice).

### Where it goes

The pill names the session the aside goes to: the agent session you typed into last (Asides sees
when each one's terminal last got input). Press `Tab` for the next one, or click the name for the
list:

- **Sessions**: every agent running in a terminal, the one you typed into last first. Those in an
  app Asides can't type into (Warp, iTerm, VS Code's terminal…) are listed, dimmed.
- **New session in…**: pick an agent (Claude Code, Codex, OpenCode) and a folder: your recent
  projects, from your sessions, your asides and the projects your agents know, or any other with
  **Choose Folder…**. Sending starts the agent there, in a new window of Terminal (or Ghostty, or
  Diri: Settings), with the aside as its first message.
- **History only**: keep it, send it later.

How the aside gets in, by app: in **Diri**, through its command (`dirijor`); in **Ghostty**, with
AppleScript (`input text`); in **Terminal**, with AppleScript (`do script`). Screenshots go in the
way each agent takes them: pasted with Ctrl+V in Diri, as their file's path in Ghostty and
Terminal (the agent attaches it). Your clipboard is put back afterwards. Codex gets Tab rather
than Return, so a message sent while it's working waits for its turn instead of joining it.

### Talking

The microphone starts when Asides opens (Settings: **Listen as soon as Asides opens**), and keeps
listening while you use your app live. Click the pill's microphone to mute it; muted, hold `Space`
to talk. Without a voice model, the microphone downloads one first.

Each pin, measurement and screenshot lands in the transcript at the moment you made it, between
what you said before and after it. To say something about one pin only, click its number to open
its card, and hold `Space` there (a tap types a space) or press its microphone: it's transcribed
right away and goes in that pin as `**Said:**`.

### Shortcuts

| Shortcut | |
|---|---|
| `⌃⌥Space` | Open Asides with the screen frozen. While it's open (frozen or live): send. |
| `⌃⌥P` | Switch between the frozen screen and the live one (to use your app, then freeze it again). With Asides closed, open it over the live screen. |

Both can be changed in `config.toml` (Settings → Edit shortcuts and more…).

## Asides on your screen

While it's frozen, nothing macOS lets an app hold back gets in the way: the Dock and the menu bar
stay hidden, and switching apps (`⌘Tab`) or spaces, Mission Control, App Exposé and Show Desktop
are off, whether by keys, trackpad gestures or hot corners. (Notification Center, Spotlight and
Force Quit still work; a click on the frozen screen takes it back.) Over the live screen, only the
pill shows, and your app gets every key and click. Drag the pill anywhere (Asides remembers where).
It brings [PixelSnap 2](https://pixelsnap.com)'s measuring and [Agentation](https://www.agentation.com)'s
annotations to the whole screen. Press `?` for every key; the first time, they show by themselves.

**Annotate** (`A`, the tool Asides opens with): click an element for a numbered pin, or drag over an
area. Click inside a section you measured to pin that section. A pin's card (click its number)
takes typed feedback, what kind of request it is (fix, change, question, approve; `⌘1`–`⌘4`) and what
you say about it. `⏎` or `Esc` closes the card; `⏎` with no card open sends. To point out a typo,
select the text before you freeze the screen, then click it: the pin quotes the selection. With
**Name what's under each pin** on (Settings; it needs the Accessibility permission), hovering
outlines the element under the cursor, and the pin names it: its role and label (`button "Save"`),
the app and window, and in browsers the page's URL and a CSS-like location
(`form.settings > button#save.btn`). Without it, pins name the app and window.

**Measure** (`M`; the pill shows it, click it to go back to Annotate):

| | |
|---|---|
| Move | The distances from the cursor to the nearest edges, across and down. |
| Click | Hold those distances on screen. `H` or `V` holds just the horizontal or vertical one. |
| Drag | The size of what's inside: the area snaps to the object's bounds. Right-drag (or ⌘ held) keeps it as drawn, `⌥`-drag holds each object inside separately, `⇧` makes it square, and `Space` while dragging moves it. |
| `⇧H` / `⇧V` | A horizontal or vertical guide at the cursor, snapped to a held object's edge or middle, or to the nearest edge. Distances stop at guides and at held objects, and the gaps between guides are shown at the screen's edge. Arrow keys nudge the last guide (⇧: 10 pt). |
| `⇧` held | A crosshair across the whole screen, to check alignment. |
| `-` / `=` | Lower or raise the tolerance: how sharply the color must change to count as an edge (soft shadows, gradients and the blur behind glass never do). `T` switches to a high one that looks through borders and hard shadows. |
| `U` | Points or pixels. |
| `⌘C` / `C` / `⌘⇧C` | Copy the size (`120 × 32`), the screenshot with everything on it, or everything as Markdown. |
| `R` | Put back the last session's guides and held measurements. |

**Draw** (`D`): drag to circle or underline something, `⇧`-drag for an arrow ("move this there").
Drawings are numbered like pins: an arrow names what it starts on and what it points to.

**Shot** (`⌘S` or `S`): drag over a part of the screen. It's copied to the clipboard and added to
the aside. Hover a held object and click its camera for a screenshot of just that object.

Right-click anywhere for guides, holds, a screenshot, copying and **Annotate Here**. `⌥⏎` sends with
the whole screen in the last screenshot. `⌫` removes the pin or measurement under the pointer
(anywhere else, the last thing added), `X` clears everything, `⇧Tab` hides it all to look at the
frozen screen. Every key can be changed in `[measure.keys]`.

The aside, as the agent gets it:

```markdown
![Screenshot 1](…/images/1.png)

**In:** Safari · "Settings — Shop" · http://localhost:3000/settings
**Measured** in pt (screen: Built-in Retina Display, 1512 × 982 pt, @2x)

- Distance from (412, 230): 24 across (x 400 to x 424), 16 down (y 222 to y 238)

The gap under the title is bigger than the one above the fields, it should be the same.

### 1. button "Save"
**Location:** form.settings > div.actions > button#save.btn.btn-primary
**Said:** And make it the same width as the fields.
```

The screenshot is attached where it was taken. How much each pin says is Settings → **How much
each pin says**: **Compact** is one line each; **Standard** gives the element, location, page and
feedback; **Detailed** adds its position, size and context; **Forensic** adds its colors and
padding, measured from the screenshot, and its accessibility role and identifier. Images are at
most 1568 pixels on their long side: agents scale larger ones down anyway.

Freezing the screen needs the **Screen Recording** permission; macOS asks the first time.

## The Asides window

Open it from the menu (Asides Window…, `⌘,`) or with `asides`:

- **History**: every aside, newest first, with where it went (or why it didn't). Send it to a
  session again, copy it (with its screenshots' paths), show it in Finder, or delete it.
- **Sessions**: the agent sessions Asides can send to, and the one it sends to now.
- **Settings**: the voice model (download one: English, 190 MB, or more accurate; any language,
  574 MB), listening as Asides opens, the microphone, naming what's under each pin, how much each pin
  says, points or pixels, where new sessions open (Terminal, Ghostty, Diri) and with which agent, how
  long the history keeps asides (7, 30, 90 days, or always), keeping what you said as audio, sounds,
  Wispr Flow, and the permissions.

It follows macOS's light and dark mode.

## The command

| Command | |
|---|---|
| `asides` | Open the Asides window. |
| `asides open` | Open Asides on the screen, as `⌃⌥Space` does (open, it sends). |
| `asides sessions` | The agent sessions Asides can send to (`●` the one it sends to now); `--json` for scripts. |
| `asides history` | The asides in the history, newest first (`-n 50`, `--json`). |
| `asides show [ID]` | An aside (the newest by default) as Markdown, with its screenshots' paths. |
| `asides send [ID] [--to SESSION]` | Send an aside again: to the session typed into last, to session `SESSION` (its id, from `asides sessions`), or `--to history`. |
| `asides download [MODEL]` | Download the voice model (`small.en`, `medium.en`, `large-v3-turbo`, `large-v3`, `small`). |
| `asides status` | Whether Asides runs, what it's doing, and where asides go. |
| `asides measure IMAGE X,Y` | Measure a screenshot (PNG): the distances from a point to the nearest edges. `X,Y,W,H` finds the object inside an area and the space around it. `--scale 2` adds points for Retina screenshots; `--tolerance N` looks through shadows. |
| `asides mcp` | The MCP server, for agents (below). |
| `asides quit` | Quit the app. |

## Coding agents

Nothing needs setting up: Asides finds the agents running in a terminal (by their processes),
and types each aside into one, as you would. Your agent answers it in its own chat.

Optionally, give your agents the MCP server too: `get_aside` reads an aside again (the newest by
default) with its screenshots inline, `list_asides` lists the recent ones, and `measure_image`
measures a screenshot like PixelSnap. Every tool is read-only.

```bash
claude mcp add --scope user asides -- asides mcp
codex mcp add asides -- asides mcp
```

## Wispr Flow

If you dictate with Wispr Flow, turn on Settings → **Wispr Flow dictations**: what you dictate
while the screen is frozen joins the aside, at the moment you said it, as Wispr Flow wrote it.
Dictations made anywhere else aren't touched.

## Settings

The Asides window has most of them. All of them are in
`~/Library/Application Support/Asides/config.toml`; changes apply as soon as you save.

```toml
mic_on_open = true           # listen as soon as Asides opens (the pill's microphone and Space still work)
sounds = true
keep_audio = false           # keep an .m4a of what you said with each aside in the history
model = "small.en"           # "medium.en" (English, more accurate), "large-v3-turbo" (any language),
                             # "large-v3" (any language, most accurate, slower), "small" (any, fastest)
input_device = ""            # microphone name; empty means the system default
languages = []               # with a multilingual model: e.g. ["en", "ro"] to choose only between these
history_days = 30            # how long asides stay in ~/.asides/history; 0 keeps them
wispr_flow = false           # Wispr Flow dictations made while frozen join the aside
annotation_detail = "standard"  # "compact", "standard", "detailed" or "forensic"

[agents]                     # new sessions (New session in…)
terminal = ""                # the app they open in: "" (Terminal), "Ghostty" or "Diri"
default = ""                 # the agent: "" (Claude Code), "codex", "opencode", "gemini", "copilot", "qwen"
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
identify_elements = true     # name what's under each pin (needs Accessibility)
padding = 48.0               # points of screen kept around what you pinned, in the screenshot

[measure.keys]               # the overlay's keys; several separated by commas, "" for none
annotate = "a"
measure = "m"
draw = "d"
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
hide = "shift+tab"
restore = "r"
talk = "space"               # hold to talk while the microphone is muted
target = "tab"               # the next session the aside goes to
help = "shift+/, f1"
done = "enter"               # send (with a card open: save and close it)
done_whole_screen = "alt+enter"
live = ""                    # back to the live screen (⌃⌥P does it from anywhere)
close = "escape"             # close without sending (again, with something to lose)

[shortcuts]
asides = "ctrl+alt+Space"    # open Asides; while it's open, send
freeze = "ctrl+alt+KeyP"     # frozen ↔ live; closed, open it live
```

An empty shortcut (`""`) turns it off.

With a multilingual model, list the languages you speak in `languages`: detection on short clips is
then much more reliable; a single entry skips detection. English models know only English.

## Privacy and permissions

- **Audio** stays on your Mac. It's held in memory until it's transcribed, then discarded, unless
  you turn on keeping it.
- **The history** (`~/.asides/history`) holds each aside: what was sent, its screenshots, where it
  went. Asides deletes asides older than the history keeps them.
- **The network** is used by Asides for one thing only: downloading the voice model from Hugging
  Face when you ask for it.
- **Agents** get each aside as a message in their session, like anything you type there, and may
  send it on to their model provider. New sessions run your agent in your own shell.
- **Automation**: to type into a session in Terminal or Ghostty, and to open new sessions there,
  Asides uses AppleScript; macOS asks once per app whether Asides may control it. Diri needs no
  permission (Asides uses its command).
- **Wispr Flow** transcribes with its own service, under its own terms. Asides only reads the
  history Wispr Flow keeps on your Mac, and only when you turn the integration on.
- **Microphone** access is asked when Asides first starts.
- **Screen Recording** is asked for the first time you freeze the screen. Screenshots stay on your
  Mac.
- **Accessibility** is asked the first time Asides opens, to name what's under each pin. Say no and
  pins name the app and window only.
- **Administrator password:** asked once, if you installed Asides by hand and `/usr/local/bin`
  belongs to the system, to link the `asides` command there. Cancel to skip it.
- **Input Monitoring** is not needed: the shortcuts use the system hot-key API, and Asides only sees
  keys while its frozen screens are in front.

## Troubleshooting

- **The aside didn't reach my agent.** The menu (and History, in the Asides window) says why. Asides
  can type into sessions in Terminal, Ghostty and Diri; one in another app is listed, dimmed. The
  first time, allow Asides to control Terminal or Ghostty (System Settings → Privacy & Security →
  Automation). Every aside stays in History: send it again from there, or `asides send`.
- **It went to the wrong session.** It goes to the session you typed into last; `Tab` on the frozen
  screen (or the pill's list) picks another before you send.
- **Two sessions in one folder in Ghostty.** Ghostty only tells Asides each terminal's folder: with
  two agents in the same folder, it types into the first. Diri and Terminal tell them apart.
- **A new session waits at "Do you trust this folder?".** That's your agent asking, the first time
  it works in a folder. Answer it; the aside follows.
- **macOS asks for the microphone after every update.** The app is signed ad hoc. Run `make cert`
  once, or install an Apple Development or Developer ID certificate (see Install).
- **A shortcut does nothing.** Another app may already use it. Pick another in `config.toml`.
  macOS refuses shortcuts whose only modifier is Option, or Option and Shift.
- **"Nothing heard."** Check Settings → Microphone. Bluetooth headsets take about a second to
  switch to their microphone.
- **The text came out in the wrong language.** With a multilingual model, set `languages`; or use an
  English model.
- **Asides says it can't see the screen when it freezes.** Allow Asides in System Settings → Privacy &
  Security → Screen & System Audio Recording, then quit and reopen Asides.
- **Distances stop at the wrong place, or go through an edge.** Change the tolerance with `-` and
  `=`: lower it to see faint borders (a light card on white), raise it (or press `T`) to look through
  borders and hard shadows.
- **Pins only name the app.** Turn on naming what's under each pin and allow Asides under
  Accessibility. Chrome and Electron apps build their accessibility tree on the first query, so
  the first pin in them may only name the window.

## How it works

- **A menu-bar app, its overlay, and a command.** `Asides.app` records and transcribes; its overlay
  (`asides-overlay`) draws the frozen screens and the pill with [GPUI](https://gpui.rs), Zed's
  GPU-accelerated UI framework, in a process of its own, started ahead of time so the shortcut
  freezes the screen at once; the same binary is the Asides window. The `asides` command talks to
  the app through `asides://` links opened with `open -g`, and reads the state it publishes.
- **Sessions.** The app lists the processes of the agents it knows that have a terminal, with
  their folders (`lsof`) and terminal devices. The environment an agent was started with says which
  app hosts it (`TERM_PROGRAM`, Diri's `DIRIJOR_SESSION_ID`); the device's access time says when it
  last got input, which is the session you typed into last.
- **Sending.** Each aside is kept in the history, then typed into its session: a plan of steps for
  its app (bracketed pastes, keys, the clipboard), measured with each agent in each app.
- **Recording.** Audio is captured with CoreAudio (cpal), resampled to 16 kHz, and transcribed in
  one pass after you send. With large-v3-turbo on an M4 Pro, 18 s of speech takes about 1 s, while
  the next thing goes on.
- **No prompt.** Whisper is not given a vocabulary or prompt. A prompt helps it spell jargon, but it
  also changes what you said: in testing it silently dropped a word.
- **Measuring.** ScreenCaptureKit takes one sRGB screenshot per screen, so colors read like the CSS
  that drew them. Edges are where the color changes sharply, from one pixel to the next (or over two
  anti-aliased pixels), visibly (in OKLab), and within a few pixels; slow changes aren't edges, so
  measuring works on glass and frosted surfaces, over gradients and through soft shadows. A faint
  change is an edge only where it goes on along an outline, so grain in a photo or a wallpaper
  doesn't stop it, while a faint card's rounded corners do. An area snaps to the bounds of what's
  drawn inside it. The same Rust code measures in the overlay, in `asides measure` and in
  `measure_image`.

## Contributing

Contributions are welcome. [CONTRIBUTING.md](CONTRIBUTING.md) covers setup, the development loop,
tests (including a manual QA checklist) and pull requests. [ARCHITECTURE.md](ARCHITECTURE.md)
explains how the parts fit together, and the invariants to keep. Coding agents follow
[AGENTS.md](AGENTS.md); Claude Code loads it through `CLAUDE.md`. Why Asides works this way:
[design/redesign.md](design/redesign.md).

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
