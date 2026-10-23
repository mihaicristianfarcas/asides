# Settings

All of them are in the asides window, and in
`~/Library/Application Support/asides/config.toml`, applied as soon as you save. Open at login
(General) is macOS's, not in the file: System Settings → General → Login Items changes it too. The
defaults:

```toml
mic_on_open = true              # listen as soon as asides opens
sounds = true
keep_audio = false              # keep an .m4a of what you said with each aside
model = "small.en"              # or "medium.en", "large-v3-turbo", "large-v3", "small" (see below)
input_device = ""               # microphone name; "" is the system default
languages = []                  # multilingual models: e.g. ["en", "ro"]
history_days = 30               # 0 keeps asides forever
auto_update = true              # look for updates every 6 hours, install them as asides quits
wispr_flow = false              # Wispr Flow dictations made while frozen join the aside
annotation_detail = "standard"  # "compact", "standard", "detailed" or "forensic"

[shortcuts]                     # "" turns one off
asides = "ctrl+alt+Space"       # open; while open, send
freeze = "ctrl+alt+KeyP"        # frozen ↔ live

[agents]                        # New session in…
terminal = ""                   # "" (Terminal), "Ghostty" or "Diri"
default = ""                    # "" (Claude Code), "codex", "opencode", "gemini", "copilot", "qwen"
claude = []                     # extra arguments per agent, e.g. ["--model", "opus"]; also codex, opencode…

[measure]
color = "#FF2D55"
units = "pt"                    # or "px"
tolerance = 2                   # % a color must change by to count as an edge
identify_elements = true        # name what's under each pin (needs Accessibility)
padding = 48.0                  # points kept around what you pinned, in the screenshot

[measure.keys]                  # several separated by commas, "" for none
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
talk = "space"
mic = "cmd+shift+m"
target = "tab"
sessions = "w"
help = "shift+/, f1"
done = "enter"
done_whole_screen = "alt+enter"
restart = "cmd+backspace"
live = ""
close = "escape"
```

## Voice models

| Model | Size | |
|---|---|---|
| `small.en` | 190 MB | English, the default |
| `medium.en` | 539 MB | English, more accurate |
| `large-v3-turbo` | 574 MB | Any language |
| `large-v3` | 1.1 GB | Any language, most accurate, slower |
| `small` | 190 MB | Any language, fastest |

With a multilingual model, list the languages you speak in `languages`: detection on short clips is
much more reliable, and a single entry skips it.
