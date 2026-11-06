# asides

**Freeze your screen, click what's wrong, say why.** It lands in your coding agent's chat as your
own message, with the screenshot, the elements you clicked and what you measured. Transcribed
locally, on your Mac or PC, on Windows or Linux.

![Report bugs out loud while you test: the asides pill over the heading, a pin on its words, a measurement under it, the download button circled and a card reading Make it a bit bigger](docs/og.png)

```
⌃⌥Space   freeze the screen (and start listening)      Windows, Linux: Ctrl+Alt+Space
click     pin what's under the pointer; drag to pin an area; talk
⌃⌥Space   send it to the agent session you typed into last
```

- **Into the session you're already in:** Claude Code, Codex, OpenCode, Gemini CLI, Copilot CLI or
  Qwen Code, in Terminal, Ghostty or [Diri](https://diri.sh) (on Windows and Linux, any terminal).
  Nothing to install in the agent.
- **For anything on screen:** pins name the element (`button "Save"`, and in browsers the page and
  a CSS-like location). Measure like PixelSnap, draw arrows, take partial screenshots.
- **Local and free:** [whisper.cpp](https://github.com/ggml-org/whisper.cpp) on your Mac's GPU (on
  Windows and Linux, the CPU). No account, no API key, no audio leaves your computer.

## Install

[asides.mihaicristian.dev](https://asides.mihaicristian.dev/#install) picks the download for your
system. Every file is signed, and also in the
[releases](https://github.com/mihaicristianfarcas/asides/releases).

### macOS

macOS 14 or later, Apple silicon.

```bash
brew tap mihaicristianfarcas/asides https://github.com/mihaicristianfarcas/asides
brew install --cask mihaicristianfarcas/asides/asides
```

Or download the [disk image](https://download.asides.mihaicristian.dev/asides.dmg) and drag
asides to Applications. On first launch it offers to link the `asides` command into
`/usr/local/bin` (macOS may ask for your password).

### Windows

Windows 10 (2004) or 11: run the installer for
[x64](https://download.asides.mihaicristian.dev/asides-windows-x64-setup.exe) or
[Arm64](https://download.asides.mihaicristian.dev/asides-windows-arm64-setup.exe) (no administrator needed). On first
launch asides puts the `asides` command on your PATH. [What differs](guide/using.md#on-windows).

### Linux

GNOME or KDE on Wayland, or any X11 desktop, with Vulkan. [What differs](guide/using.md#on-linux).

| | x86_64 | Arm64 | Then |
|---|---|---|---|
| Debian, Ubuntu, Mint | [.deb](https://download.asides.mihaicristian.dev/asides-linux-x86_64.deb) | [.deb](https://download.asides.mihaicristian.dev/asides-linux-aarch64.deb) | `sudo apt install ./asides-linux-*.deb` |
| Fedora, openSUSE | [.rpm](https://download.asides.mihaicristian.dev/asides-linux-x86_64.rpm) | [.rpm](https://download.asides.mihaicristian.dev/asides-linux-aarch64.rpm) | `sudo dnf install ./asides-linux-*.rpm` (`zypper install` on openSUSE) |
| Arch | | | `makepkg -si`, beside [PKGBUILD](packaging/linux/PKGBUILD) |
| Any other | [.tar.gz](https://download.asides.mihaicristian.dev/asides-linux-x86_64.tar.gz) | [.tar.gz](https://download.asides.mihaicristian.dev/asides-linux-aarch64.tar.gz) | `./install.sh`, from the archive, for you alone |

### From source

Needs Rust, the Xcode Command Line Tools and CMake (`brew install cmake`).

```bash
git clone https://github.com/mihaicristianfarcas/asides && cd asides
make install   # ~/Applications/asides.app, and the command in ~/.local/bin
```

On [Windows](CONTRIBUTING.md#windows), `scripts\package-windows.ps1`; on
[Linux](CONTRIBUTING.md#linux), `scripts/package-linux.sh`.

## Get started

1. Open asides. Setup walks you through the permissions, the voice model (190 MB) and where new
   sessions open, or skip it. asides lives in the menu bar (or the notification area) as `[a]`.
2. Start your agent (`claude`, `codex`…) in Terminal, Ghostty or Diri (on Windows and Linux, any
   terminal), as usual.
3. In the app you're testing, press `⌃⌥Space`, click, talk, and press `⌃⌥Space` again.

Pins, measuring and screenshots work without a voice model; skipped, it's downloaded the first time
you click the pill's microphone.

## Documentation

- [Using asides](guide/using.md): the tools, keys, where asides go, the window and the command
- [Settings](guide/settings.md): `config.toml`
- [Privacy and permissions](guide/privacy.md)
- [Troubleshooting](guide/troubleshooting.md); questions in
  [Discussions](https://github.com/mihaicristianfarcas/asides/discussions/categories/q-a)
- [Security](SECURITY.md): what asides protects, and reporting a vulnerability
- [Contributing](CONTRIBUTING.md) and [Architecture](ARCHITECTURE.md)

## License

[MIT](LICENSE-MIT) or [Apache 2.0](LICENSE-APACHE), at your option. Contributions are dual licensed
the same way. The Whisper models are OpenAI's, under MIT, downloaded from
[Hugging Face](https://huggingface.co/ggerganov/whisper.cpp).
