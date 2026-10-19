# Troubleshooting

**The aside didn't reach my agent.** The menu (and History in the asides window) says why. asides
can type into Terminal, Ghostty and Diri; the first time, allow it to control Terminal or Ghostty
(System Settings → Privacy & Security → Automation). Every aside stays in History: send it again
from there, or with `asides send`.

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

**A shortcut does nothing.** Another app may use it: pick another in Settings → General. macOS refuses
shortcuts whose only modifiers are Option, or Option and Shift.

**"Updates are off."** The reason follows: a copy built from source (not notarized), or one that
isn't in an app, doesn't update itself. "asides can't write to …" means its folder isn't yours:
download the new version from the website.

**An update didn't install.** What happened is in
`~/Library/Caches/asides/updates/<version>/install.log`; if anything failed, the old app stays.
The update installs only once asides has quit.

**macOS asks for the microphone after every update.** Your build is signed ad hoc. Run `make cert`
once, or use an Apple Development or Developer ID certificate (see
[CONTRIBUTING.md](../CONTRIBUTING.md)).

Still stuck? [Open an issue](https://github.com/mihaicristianfarcas/asides/issues/new?template=bug_report.md)
with your macOS version and the output of `asides status`.
