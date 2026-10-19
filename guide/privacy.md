# Privacy and permissions

## Your data

- **Audio** stays on your Mac, in memory until it's transcribed, then discarded (unless you turn on
  keeping it).
- **asides** are kept in `~/.asides/history` (what was sent, its screenshots, where it went) and
  deleted after `history_days`.
- **The network** is used for one thing: downloading the voice model from Hugging Face, when you
  ask.
- **Your agent** gets each aside like anything you type, and may send it to its model provider.
- **Wispr Flow**, when you turn the integration on: asides reads the history Wispr Flow keeps on your
  Mac. Wispr Flow itself transcribes under its own terms.

## Permissions

| Permission | Asked | For |
|---|---|---|
| Microphone | at first launch | talking |
| Screen Recording | the first time you freeze the screen | the screenshots |
| Accessibility | the first time asides opens | naming what's under a pin. Without it, pins name the app and window. |
| Automation | the first time asides types into Terminal or Ghostty | typing the aside in (Diri needs none) |
| Administrator password | once, for a downloaded copy | linking `asides` into `/usr/local/bin`. Cancel to skip. |

Input Monitoring isn't needed: the shortcuts use the system's hot keys.

## Uninstall

`brew uninstall --cask asides` (add `--zap` to delete settings, models and history too), or
`make uninstall` for a source build. For a downloaded copy, delete `asides.app` and
`/usr/local/bin/asides`. Settings and models are in `~/Library/Application Support/asides`, the
history in `~/.asides`.
