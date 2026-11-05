# GPUI 0.2.2, patched

The crate as published on crates.io (its examples left out), with what asides needs of its Linux
backends. Everything here is under `src/platform/linux/` or only reached from there: macOS and
Windows build the same code as the published crate. Upstream's later versions moved on too far to
take (asides' macOS and Windows code depends on 0.2.2's internals).

## X11 (`src/platform/linux/x11/window.rs`)

- `X11Window` gives its window and display handles (they were `unimplemented!()`, a panic): the
  overlay makes GPUI's windows override-redirect, moves, maps and focuses them on a connection of
  its own (`crates/asides-overlay/src/lin/native.rs`).
- `WindowOptions::show` is honored (the window was always mapped as it opened): a frozen screen's
  window stays unmapped until it's ready to show, and a window manager never sees it.
- `process_x11_events` (`x11/client.rs`) returns the error when the connection is lost, which ends
  the event loop (it logged it and polled on, at 100% of a core, for as long as the process lived).

## Wayland (`src/platform/linux/wayland/window.rs`)

- A compositor without xdg-decoration (Mutter) leaves the frame to the window: `request_decorations`
  says so (asked for `Server`, the window went without a frame and `window_decorations` said
  `Server`).

## All

- `Window::draw_now`: draws and presents a frame at once, as the platform's frame callback would.
- `DispatchEventResult` is public: `Window::dispatch_event` returns it (0.2.2's private type).
