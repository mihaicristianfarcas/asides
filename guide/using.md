# Using asides

The first time asides opens, setup shows how it works, then asks where new sessions open
(Terminal, Ghostty or Diri, and the agent), what macOS must allow, the voice model and the
shortcuts. Skip it for the defaults: asides asks for each permission and the model when it first
needs them. General → **Set Up Again** goes through it again. **Help**, at the bottom of the asides
window's sidebar, shows how to use asides, with clips of it.

## Freeze, pin, talk, send

| | |
|---|---|
| `⌃⌥Space` | Freeze every screen. The pill at the top shows the keys (?), the microphone, where the aside goes, send (✓) and close (×). |
| Click | Pin the element under the pointer. Drag to pin an area. |
| Drag a pin's number | Move what it's on: a copy goes where you drop it, with an arrow, and the pin says where to. |
| Talk | Each pin's number lands after the word you said as you placed it (`this [1] a different color`). |
| `⌃⌥Space`, `⏎` or ✓ | Send. A moment later it's in your agent's chat: what you said, with a screenshot around what you marked (none if you marked nothing). |
| `Esc` or × | Close without sending (press twice if there's something to lose). |
| `⌃⌥P` | Back to the live screen: only the pill stays, your app gets every key and click. Press again to freeze again. One aside holds all its freezes. With asides closed, it opens live. |

While frozen, the Dock, the menu bar, `⌘Tab` and Mission Control are held back (Force Quit still
works). Drag the pill anywhere; it stays there. Point at its `?` (or press `?`) for every key.

## Talking

The microphone starts as asides opens (a setting). Click it to mute; while muted, hold `Space` to
talk, and each new pin opens its card, to type in.

To talk about one pin, click its number to open its card and hold `Space` there (or press its
microphone): that goes into the pin. The card also takes typed feedback and a kind (fix, change,
question, approve: `⌘1`–`⌘4`; with no card open, they set the last pin's), shown in the pin's
color. Its pencil and ruler pick Draw and Measure (their keys type there). `⏎` or `Esc` closes it.

## Where it goes

The pill names the agent session you typed into last. Press `Tab` for the next one (held, it goes
through them slowly enough to read), or point at the name for the list (click it to keep it open):

- **Sessions**: every agent running in a terminal, by the title its agent gave it. Those in apps
  asides can't type into (Warp, iTerm, VS Code…) are dimmed.
- **New session in…**: an agent and a folder (a recent project, or **Choose Folder…**). Sending
  starts the agent there, in Terminal, Ghostty or Diri (Settings), with the aside as its first
  message.
- **History only**: keep it, send it later.

## Tools

| Key | Tool |
|---|---|
| `A` | **Annotate** (the default): click pins an element, drag pins an area. To quote a typo, select the text before freezing, then click it. |
| `M` | **Measure**, like PixelSnap (below). |
| `D` | **Draw**: drag to circle or underline, `⇧`-drag for an arrow. |
| `S`, `⌘S` | **Shot**: drag over part of the screen. It's copied and added to the aside. |

A tool's key again, or a click on its name on the pill, puts it down: back to Annotate (from Shot,
to the tool before it).

Measure:

| | |
|---|---|
| Move | The distances to the nearest edges. Click to hold them (`H` or `V`: only one). |
| Drag | The size of what's inside, snapped to it. Right-drag keeps it as drawn, `⌥`-drag measures each object, `⇧` makes it square. |
| `⇧H`, `⇧V` | A guide at the cursor. Arrow keys nudge it. |
| `⇧` held | A crosshair across the screen. |
| `-`, `=`, `T` | Lower or raise how sharp a change counts as an edge. `T` looks through borders and shadows. |
| `U` | Points or pixels. |
| `⌘C`, `C`, `⌘⇧C` | Copy the size, the screenshot, or everything as Markdown. |
| `R` | Bring back the last session's guides and held measurements. |

Anywhere: right-click for a menu, `⌫` removes what's under the pointer (or the last thing), `X`
clears everything, `⇧Tab` hides it all, `⌥⏎` sends with the whole screen (even with nothing
marked).

## What the agent gets

```markdown
**In:** Safari ("Settings — Shop", http://localhost:3000/settings)

![Screenshot 1](…/images/1.png)

The gap under the title is bigger than the one above the fields, and this [1] should go there [2].

### 1. button "Save"
**Location:** form.settings > div.actions > button#save.btn.btn-primary
**Said:** And make it the same width as the fields.

### 2. Area 320 × 48 at (380, 200)
**Move to:** (380, 120), 80 pt up, over form "Settings"
**Intent:** change
```

Each screenshot goes where its freeze began, around what was marked on it; a pin's details follow
the paragraph it was said in.

How much each pin says is a setting: **Compact**, **Standard**, **Detailed** (position, size) or
**Forensic** (colors, padding, accessibility role).

## The asides window

Open it from the menu bar or with `asides`. While it's open it's in the Dock and `⌘Tab`; `⌘W`
closes it, `⌘Q` quits asides.

- **History**: every aside, and whether it was sent (to which session) or only saved. What was
  said with its pins' numbers, the screenshots and each pin. Send it again, copy it, show it in
  Finder, delete it, or **Edit** its Markdown (`⌘S` saves it once nothing in it is wrong: each
  problem is listed by line). asides are kept in `~/.asides/history` for 30 days by default.
- **Sessions**: the agent sessions asides can send to, and where new ones open.
- **General**, **Keys** (the shortcuts, and the frozen screen's keys), **Voice**, **Pins**: every
  [setting](settings.md). Click a shortcut or a key, then press the new one.
- **Help** (`⌘?`): how to use asides, step by step, with clips of it.

The menu bar menu opens asides (frozen or live), copies the last aside, turns listening, sounds and
naming pins on or off, and opens History or Settings.

## Updates

asides looks for a new version every 6 hours, downloads it, and installs it when it quits (General
→ **Update automatically**). To install it now, click **Restart to update** at the bottom of the
asides window's sidebar (or **Update to …** in the menu bar menu): asides reopens on the new
version, then the sidebar offers **What's new**. **Check for Updates…** is in the asides menu.
Copies built from source don't update themselves.

## The command

| Command | |
|---|---|
| `asides` | Open the asides window. |
| `asides open` | Open asides on the screen, like `⌃⌥Space`. |
| `asides sessions` | The sessions asides can send to (`--json`). |
| `asides history` | Past asides, newest first (`-n 50`, `--json`). |
| `asides show [ID]` | An aside as Markdown (the newest by default). |
| `asides send [ID] [--to SESSION]` | Send an aside again, to a session from `asides sessions` or `--to history`. |
| `asides download [MODEL]` | Download a voice model. |
| `asides measure IMAGE X,Y` | Measure a PNG from a point, or `X,Y,W,H` for the object in an area. |
| `asides status` | What asides is doing, and where asides go. |
| `asides mcp` | The MCP server (below). |
| `asides quit` | Quit the app. |

## For your agents (optional)

asides needs nothing from your agents. If you want them to read past asides or measure
screenshots, add its MCP server (`get_aside`, `list_asides`, `measure_image`, all read-only):

```bash
claude mcp add --scope user asides -- asides mcp
codex mcp add asides -- asides mcp
```

## Wispr Flow

Turn on Settings → **Wispr Flow dictations**: what you dictate with Wispr Flow while the screen is
frozen joins the aside, where you said it.
