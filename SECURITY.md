# Security

asides sees your screens, hears your microphone and types into your terminals, so a hole in it
matters. This page says how to report one, and what asides does and doesn't protect.

## Reporting a vulnerability

Report it privately, through GitHub's **private vulnerability reporting** (the repository's
**Security** tab, **Report a vulnerability**), not in an issue. Say what an attacker needs (local
access, a crafted file or page, a position on the network), what they gain, and how to reproduce
it. You'll get an answer, and credit in the fix if you want it.

Only the latest release is supported.

## What asides does

- **Your computer keeps what you say and show.** Speech is transcribed on it. The network is used
  for the voice model, from Hugging Face when you ask, and the update feed, from GitHub. Neither
  sends anything about you or your asides. [Privacy](guide/privacy.md) has every permission.
- **It types only into sessions you chose**, as you would: nothing is installed in your agents, and
  nothing listens for them.
- **Updates** come only from this repository's releases, and only newer versions. On a Mac, a
  download is taken only if `codesign` verifies it, its team and bundle identifier are the running
  copy's, and Gatekeeper accepts it (notarized). On Windows, only if the release's minisign
  signature names it (the key asides is built with, `packaging/release-signing.pub`) and Windows
  trusts its Authenticode signature, by the running copy's publisher. Linux copies only say a
  release is out.
- **On Linux**, where the kernel restricts one process reaching into another, typing into most
  terminals needs the helper `asides-keys` to hold `cap_sys_ptrace`, which you grant in setup to
  that file alone. It only writes keystrokes to terminals of your own, and drops the capability
  before reading anything.
- **Your history** (`~/.asides`) and settings are files of your own user's. `asides mcp` lets the
  agents you add it to read your history.

## Out of scope

- Your agent and its model provider: an aside typed in is theirs, like anything you type.
- Anyone who can already run programs as you: they can read your history and settings anyway.
