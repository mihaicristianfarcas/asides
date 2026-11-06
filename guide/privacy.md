# Privacy and permissions

## Your data

- **Audio** stays on your computer, in memory until it's transcribed, then discarded (unless you turn on
  keeping it).
- **asides** are kept in `~/.asides/history` (what was sent, its screenshots, where it went) and
  deleted after `history_days`.
- **The network** is used for two things: downloading the voice model from Hugging Face, when you
  ask, and looking for updates on GitHub every 6 hours (turn off **Update automatically** to look
  only when you ask). Neither sends anything about you or your asides.
- **Your agent** gets each aside like anything you type, and may send it to its model provider.
- **Wispr Flow**, when you turn the integration on: asides reads the history Wispr Flow keeps on your
  Mac. Wispr Flow itself transcribes under its own terms.

## Permissions

Setup asks for each as you go (General → **Set Up Again** shows them, allowed or not). Skipped:

| Permission | Asked | For |
|---|---|---|
| Microphone | the first time you talk | talking |
| Screen Recording | the first time you freeze the screen | the screenshots |
| Accessibility | the first time asides opens | naming what's under a pin. Without it, pins name the app and window. |
| Automation | the first time asides types into Terminal or Ghostty | typing the aside in (Diri needs none) |
| Administrator password | once, for a downloaded copy | linking `asides` into `/usr/local/bin`. Cancel to skip. |

Input Monitoring isn't needed: the shortcuts use the system's hot keys.

On Windows only the microphone is asked for (Settings → Privacy & security → Microphone → **Let
desktop apps access your microphone**): freezing the screen, naming what's under a pin and typing
into consoles need no permission.

On Linux nothing asks for the microphone. GNOME on Wayland asks once before the first screenshot,
and only for the app in front: setup's **Screenshots** row asks with asides' window open (after a
"Deny", **Allow…** asks again). KDE allows asides by its launcher entry. Where the kernel restricts
one process reaching into another (`ptrace_scope`, on Ubuntu and Arch), typing into most terminals
needs the helper `asides-keys` to hold `cap_sys_ptrace`: setup's **Allow…** asks for your password
and grants it to that file alone. The helper only writes keystrokes to terminals of your own and
drops it before reading anything. tmux, Konsole and WezTerm need none of this, and an update
replaces the file, so setup asks again.

## Uninstall

`brew uninstall --cask asides` (add `--zap` to delete settings, models and history too), or
`make uninstall` for a source build. For a downloaded copy, delete `asides.app` and
`/usr/local/bin/asides`. Settings and models are in `~/Library/Application Support/asides`, the
history in `~/.asides`.

On Windows, uninstall asides from Settings → Apps: it quits asides and takes its folder off the
PATH. Settings, models and the log stay in `%LOCALAPPDATA%\asides`, the history in
`%USERPROFILE%\.asides`.

On Linux, remove the package (`apt remove asides`, `dnf remove asides`, `pacman -R asides-bin`), or
run `install.sh --uninstall` from the archive. Settings stay in `~/.config/asides`, models in
`~/.local/share/asides`, the log in `~/.local/state/asides`, the history in `~/.asides`. GNOME keeps
the shortcuts asides added until you delete them in Settings → Keyboard.
