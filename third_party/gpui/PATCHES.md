# GPUI 0.2.2, patched

The crate as published on crates.io (its examples left out), with what asides needs of its Linux
backends (under `src/platform/linux/` or only reached from there), and a few changes for every
system (All). Upstream's later versions moved on too far to take (asides' macOS and Windows code
depends on 0.2.2's internals).

## X11 (`src/platform/linux/x11/window.rs`)

- `X11Window` gives its window and display handles (they were `unimplemented!()`, a panic): the
  overlay makes GPUI's windows override-redirect, moves, maps and focuses them on a connection of
  its own (`crates/asides-overlay/src/lin/native.rs`).
- `WindowOptions::show` is honored (the window was always mapped as it opened): a frozen screen's
  window stays unmapped until it's ready to show, and a window manager never sees it.
- `process_x11_events` (`x11/client.rs`) returns the error when the connection is lost, which ends
  the event loop (it logged it and polled on, at 100% of a core, for as long as the process lived).
- Keys go to the input method (XIM, IBus's on Ubuntu) only while a text input is focused
  (`wants_input_method`): every key went there and came back only if it wasn't taken, so where
  nothing takes text (a frozen screen's tools), a slow, restarted or silent IBus lost them.

## Wayland (`src/platform/linux/wayland/window.rs`)

- A compositor without xdg-decoration (Mutter) leaves the frame to the window: `request_decorations`
  says so (asked for `Server`, the window went without a frame and `window_decorations` said
  `Server`).

## macOS (`src/platform/mac/`)

- Lines are shaped with one copy of each font at each size (`text_system.rs`): a copy made for each
  line had CoreText work out its advances again (for a variable font, from its variations), 25 µs a
  line. R bringing back hundreds of labels spent more than a frame on it.
- Before a frame is presented with a Core Animation transaction (`displayLayer:`, which the overlay
  calls as it resizes a window and draws it in one), the last frame presented on its own (the
  display link's) is waited for. Still on the GPU, it sometimes showed after the transaction's
  frame, at its old size stretched over the window's new one: the pill smeared over its keys sheet
  for a frame as it opened.

## All

- `Window::draw_now`: draws and presents a frame at once, as the platform's frame callback would.
- `TextSystem::layout_line_in` shapes a line outside any window, and `TextSystem::of_platform`
  is the platform's text system outside any app (CoreText, DirectWrite or cosmic-text): the
  overlay's test of R's cost times shaping its labels (an app is made only on the main thread, a
  test runs on another).
- `DispatchEventResult` is public: `Window::dispatch_event` returns it (0.2.2's private type).
- Truncated text (`truncate()`) is cut, with its ellipsis (`elements/text.rs`): a layout measured
  without a width (a size it would like) was kept and answered every later measure, so the text ran
  on under its clip, cut off mid-letter. Only an untruncated layout answers an untruncated measure
  (upstream's fix, zed `b05f40c`); text that fits as shaped isn't truncated (adding each character's
  width, without kerning, made it wider); and the ellipsis follows no space or punctuation
  (`text_system/line_wrapper.rs`), as upstream.
