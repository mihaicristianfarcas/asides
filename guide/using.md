# Using Asides

## Freeze, pin, talk, send

| | |
|---|---|
| `⌃⌥Space` | Freeze every screen. The pill at the top shows the microphone, where the aside goes, send (✓) and close (×). |
| Click | Pin the element under the pointer (outlined before you click). Drag to pin an area. |
| Talk | What you say lands between the pins, in the order it happened. |
| `⌃⌥Space`, `⏎` or ✓ | Send. A moment later it's in your agent's chat, with each freeze's screenshot. |
| `Esc` or × | Close without sending (press twice if there's something to lose). |
| `⌃⌥P` | Back to the live screen: only the pill stays, your app gets every key and click. Press again to freeze again. One aside holds all its freezes. With Asides closed, it opens live. |

While frozen, the Dock, the menu bar, `⌘Tab` and Mission Control are held back (Force Quit still
works). Drag the pill anywhere; it stays there. Press `?` for every key.

## Talking

The microphone starts as Asides opens (a setting). Click it to mute; while muted, hold `Space` to
talk.

To talk about one pin, click its number to open its card and hold `Space` there (or press its
microphone): that goes into the pin. The card also takes typed feedback and a kind (fix, change,
question, approve: `⌘1`–`⌘4`). `⏎` or `Esc` closes it.

## Where it goes

The pill names the agent session you typed into last. Press `Tab` for the next one, or click the
name for the list:

- **Sessions**: every agent running in a terminal. Those in apps Asides can't type into (Warp,
  iTerm, VS Code…) are dimmed.
- **New session in…**: an agent and a folder (a recent project, or **Choose Folder…**). Sending
  starts the agent there, in Terminal, Ghostty or Diri (Settings), with the aside as its first
  message.
- **History only**: keep it, send it later.

## Tools

| Key | Tool |
|---|---|
| `A` | **Annotate** (the default): click pins an element, drag pins an area. To quote a typo, select the text before freezing, then click it. |
| `M` | **Measure**, like PixelSnap (below). Click the tool's name on the pill to go back to Annotate. |
| `D` | **Draw**: drag to circle or underline, `⇧`-drag for an arrow. |
| `S`, `⌘S` | **Shot**: drag over part of the screen. It's copied and added to the aside. |

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
clears everything, `⇧Tab` hides it all, `⌥⏎` sends with the whole screen.

## What the agent gets

```markdown
![Screenshot 1](…/images/1.png)

**In:** Safari · "Settings — Shop" · http://localhost:3000/settings

The gap under the title is bigger than the one above the fields.

### 1. button "Save"
**Location:** form.settings > div.actions > button#save.btn.btn-primary
**Said:** And make it the same width as the fields.
```

How much each pin says is a setting: **Compact**, **Standard**, **Detailed** (position, size) or
**Forensic** (colors, padding, accessibility role).

## The Asides window

Open it from the menu bar or with `asides`:

- **History**: every aside and where it went. Send it again, copy it, show it in Finder, delete it.
  Asides are kept in `~/.asides/history` for 30 days by default.
- **Sessions**: the agent sessions Asides can send to.
- **Settings**: the voice model, the microphone, sounds, where new sessions open, and the rest of
  [the settings](settings.md).

## The command

| Command | |
|---|---|
| `asides` | Open the Asides window. |
| `asides open` | Open Asides on the screen, like `⌃⌥Space`. |
| `asides sessions` | The sessions Asides can send to (`--json`). |
| `asides history` | Past asides, newest first (`-n 50`, `--json`). |
| `asides show [ID]` | An aside as Markdown (the newest by default). |
| `asides send [ID] [--to SESSION]` | Send an aside again, to a session from `asides sessions` or `--to history`. |
| `asides download [MODEL]` | Download a voice model. |
| `asides measure IMAGE X,Y` | Measure a PNG from a point, or `X,Y,W,H` for the object in an area. |
| `asides status` | What Asides is doing, and where asides go. |
| `asides mcp` | The MCP server (below). |
| `asides quit` | Quit the app. |

## For your agents (optional)

Asides needs nothing from your agents. If you want them to read past asides or measure
screenshots, add its MCP server (`get_aside`, `list_asides`, `measure_image`, all read-only):

```bash
claude mcp add --scope user asides -- asides mcp
codex mcp add asides -- asides mcp
```

## Wispr Flow

Turn on Settings → **Wispr Flow dictations**: what you dictate with Wispr Flow while the screen is
frozen joins the aside, where you said it.
