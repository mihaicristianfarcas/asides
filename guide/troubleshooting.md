# Troubleshooting

**The aside didn't reach my agent.** The menu (and History in the asides window) says why. asides
can type into Terminal, Ghostty and Diri (on Windows, any console: Windows Terminal, VS Code's
terminal…); the first time, allow it to control Terminal or Ghostty (System Settings → Privacy &
Security → Automation). Every aside stays in History: send it again from there, or with `asides
send`.

**It went to the wrong session.** It goes to the session you typed into last. Press `Tab` on the
frozen screen, or click the session's name on the pill, to pick another before sending.

**Two agents in one folder in Ghostty.** Ghostty only tells asides each terminal's folder, so it
types into the first. Diri and Terminal tell them apart.

**A new session waits at "Do you trust this folder?"** That's your agent asking, the first time
it runs in a folder. Answer it and the aside follows.

**asides can't see the screen.** Allow it in System Settings → Privacy & Security → Screen &
System Audio Recording, then quit and reopen asides.

**Pins only name the app.** Turn on Settings → **Name what's under each pin** and allow asides
under Accessibility. Chrome and Electron apps build their accessibility tree on the first query,
so the first pin there may only name the window.

**"Nothing heard."** Check Settings → Microphone. Bluetooth headsets take about a second to switch
to their microphone.

**The text came out in the wrong language.** Use an English model, or set `languages` with a
multilingual one ([Settings](settings.md)).

**Distances stop at the wrong place.** Lower the tolerance (`-`) to see faint borders, raise it
(`=`, or `T`) to look through borders and shadows.

**A shortcut does nothing.** Another app may use it: pick another in Settings → Keys. macOS refuses
shortcuts whose only modifiers are Option, or Option and Shift.

**"Updates are off."** The reason follows: a copy built from source (not notarized), one that
isn't in an app, or on Windows one that isn't signed or didn't come from the installer, doesn't
update itself. "asides can't write to …" means its folder isn't yours: download the new version from
the website.

**An update didn't install.** What happened is in `install.log`, in
`~/Library/Caches/asides/updates/<version>/` (Windows: `%LOCALAPPDATA%\asides\updates\<version>\`,
beside the installer's `setup.log`); if anything failed, the old copy stays. The update installs only
once asides has quit.

**macOS asks for the microphone after every update.** Your build is signed ad hoc. Run `make cert`
once, or use an Apple Development or Developer ID certificate (see
[CONTRIBUTING.md](../CONTRIBUTING.md)).

**Windows says it protected your PC.** asides is signed (**More info** names the publisher, Open
Source Developer Farcaș Mihai-Cristian), but SmartScreen also goes by how many people have run a
file, and a new release starts with few: click **More info**, then **Run anyway**.

## On Linux

**"can't type" beside a session.** Your kernel restricts the helper that types into terminals. Allow
it in Setup (General → **Set Up Again**), which asks for your password, or run `sudo setcap
cap_sys_ptrace+ep /usr/lib/asides/asides-keys`. An update replaces the file: allow it again. Or use
tmux, Konsole or WezTerm, which need none of this.

**A shortcut does nothing on Wayland.** Look in your desktop's keyboard settings: GNOME lists them
under Custom Shortcuts, KDE under Shortcuts, and another key may have them. On another desktop, bind
keys to `asides open` and `asides live`.

**The screen doesn't freeze on Wayland.** The log says why (`asides log`). GNOME asks once, from
asides' window: press **Allow…** by Screenshots in Setup or General (again after a "Deny"). asides'
windows need Xwayland: install it if the error names it (`xwayland`, or `xorg-x11-server-Xwayland`).
KDE allows the program asides' launcher entry names, which asides writes as it starts (a package
brings its own): moved elsewhere, start it again from there.

**The screen freezes but nothing is drawn.** The overlay needs Vulkan: install your GPU's driver
(`mesa-vulkan-drivers`, `vulkan-radeon`, `vulkan-intel`…). Without a GPU, Mesa's software driver draws
it, slowly; if the GPU's driver crashes asides (some VMs' virtual GPUs), asides switches to it until
it quits, and the log says so.

**No `[a]` on GNOME.** GNOME has no status icons without the AppIndicator extension (Ubuntu's comes
with it). Everything else still works.

**Choose Folder… says there's no folder picker.** It's your desktop's file chooser portal: install
`xdg-desktop-portal-gtk`, or on GNOME its Files app (`nautilus`).

**Pins only name the app, or only a point.** Turn on your desktop's accessibility (screen reader or
"assistive technologies"), which Qt apps and browsers wait for, or start Chrome with
`--force-renderer-accessibility`.

Still stuck? [Open an issue](https://github.com/mihaicristianfarcas/asides/issues/new?template=bug_report.md)
with your macOS, Windows or Linux version (and desktop) and the output of `asides status`.
