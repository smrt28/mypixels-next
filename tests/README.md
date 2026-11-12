# Regression checks

Run `cargo test --offline` from the repo root for font layout, configuration, clipboard
transfer bounds, and PTY paste encoding checks.

`dbus-run-session -- xvfb-run -a python3 tests/daemon_smoke.py` checks the
windowless daemon: no startup windows or children, duplicate-owner rejection,
remote launch, last-window closure, restoration with the original child PIDs,
tab order and workspace selections, protocol replies without windows, child
exit cleanup, whole-window graveyard eviction, stable routing from restored
shells, and the graveyard's Restore/Kill window buttons. It uses a private session
bus, display, MCP socket, configuration, and terminal children; it never starts
or stops the desktop's systemd service.

`dbus-run-session -- xvfb-run -a -s "-screen 0 1600x1000x24" python3 tests/close_dialog_smoke.py`
checks the default close dialog and all three visibility policies, Cancel
focus, disabled destructive clicks, the five-second delay, blocked terminal
input, recovery-host disappearance, and permanent closure with a surviving
window. It saves screenshots of the recovery and danger states in `/tmp`.

For keyboard/mouse behavior, build the debug binary, then run from the repository
root:

```sh
xvfb-run -a python3 tests/ui_smoke.py
```

Requires Xvfb, xdotool, xsel, and ImageMagick's `import`. The test creates an
isolated HOME and terminal child, checks the actual bytes pasted into the PTY,
and leaves its log and screenshot in the printed `/tmp/mypixels-ui-*` directory.
Clipboard reads are deliberately delayed to check that changing tabs during a
paste does not change its destination. Never point this test at your desktop.

To exercise native Wayland ownership, start a disposable compositor, then run
with its **isolated** runtime directory and display:

```sh
MYPIXELS_ISOLATED_CLIPBOARD_TEST=1 \
XDG_RUNTIME_DIR=/path/to/isolated/runtime WAYLAND_DISPLAY=wayland-1 \
cargo test --offline \
  wayland_interoperability -- --ignored --nocapture
```

Requires data-control support and wl-copy/wl-paste. It checks against independent
clipboard clients, including Unicode, trailing newlines, primary selection,
ownership changes, and repeated replacement of native sources. The explicit
test flag prevents accidentally running it against a normal desktop.

For an unpaced stream of changing paths (without filesystem traversal), launch:

```sh
MYPIXELS_TRACE=1 target/release/mypixels \
  python3 tests/output_flood.py 10 /tmp/mypixels-flood.json
```

Use the same window size, compositor and GPU for comparisons. `MYPIXELS_TRACE`
reports cumulative frame-build and text-preparation times plus total frame time;
subtract build from text time to isolate layout. The JSON records lines produced
and elapsed time. A software renderer's CPU use is not comparable to a desktop
GPU's, and unpaced input tests throughput as well as rendering.

For preview rendering without xdotool, run from the repo root:

```sh
xvfb-run -a python3 tests/preview_smoke.py
```

Requires ImageMagick `import` and Pillow. It compares a switcher thumbnail
against a downscaled live frame, checking all four edges, aspect ratio, glyph
placement, and colors. Screenshots are left in `/tmp/mypixels-preview-*`.

`xvfb-run -a python3 tests/damage_smoke.py` pixel-compares partial redraws
against `MYPIXELS_FULL_REDRAW=1`, using the same binary and deterministic PTY
output. It covers sparse edits, wide characters, erasure, colors, scrolling,
alternate screens, selection, background previews, tab switches, and resizing.
Requires xdotool, ImageMagick `import`, and Pillow. Screenshots and difference
images are left in `/tmp/mypixels-damage-*`. `MYPIXELS_NOCACHE=1` also selects
the full-redraw diagnostic path.

For desktop notifications, use a private bus and display:

```sh
dbus-run-session -- xvfb-run -a python3 tests/notifications_smoke.py
```

Requires PyGObject (Gio), xdotool, and gdbus. It runs a fake notification daemon
and checks OSC 9/99/777 delivery, chunked messages, replacement, click-to-tab,
protocol replies, and focus filtering without sending real desktop toasts.

To check command history with real zsh, bash, and fish shells (whichever
are installed), run:

```sh
xvfb-run -a python3 tests/history_smoke.py
```

It uses isolated homes and multiline Unicode prompts, checks captured output
and re-running a command, and opens/dismisses the terminal context menu while
the history sidebar remains open. It requires xdotool, xsel, ImageMagick's
`import`, and Pillow, and leaves screenshots in `/tmp/mypixels-history-*`.

For settings navigation, Save/Cancel, font and color pickers, and a small
window preview:

```sh
xvfb-run -a python3 tests/settings_smoke.py
```

It uses an isolated HOME and configuration, checks persistence and cancellation,
and leaves screenshots in `/tmp/mypixels-settings-*`. Requires xdotool,
ImageMagick's `import`, and Pillow.

`xvfb-run -a python3 tests/switcher_smoke.py` exercises eight live terminal
previews, Tab/Shift-Tab and Enter, Escape restoration, card clicks, and navigation
in a small window. It saves screenshots for visual review. The settings smoke
also verifies exclusive sidebar visibility and identical surface backgrounds.

`xvfb-run -a python3 tests/panel_keyboard_smoke.py` checks that history and
settings remain passive for Tab, typing, Enter, Escape, and arrow keys, including
after clicking panel buttons. It also checks explicit editor focus and global
shortcuts. Set `MYPIXELS_BIN=target/release/mypixels` to check the release build.

`xvfb-run -a python3 tests/tab_bar_smoke.py` checks the application menu's
actions, toggle/click-away/Escape dismissal, keyboard ownership, and the egui strip's add,
selection, close and close-cancel behavior, drag reordering, 20-tab overflow,
middle-click close, and native detach into another window. It leaves visual
artifacts in `/tmp/mypixels-tab-bar-*`.

`python3 tests/workspace_smoke.py` uses a dedicated FIFO and isolated terminal
children on the current graphical display. It checks all ten workspace shortcuts,
independent tab sets, remembered selection, background output and protocol replies,
hidden child exits, closing the last tab of a workspace, and independent windows.
It also checks creating missing numbered tabs (including tab 10 and custom
Ctrl+5 bindings), existing tab order, repeated shortcuts, and switcher behavior.
Tab preference checks cover startup with the switcher disabled, enabling the
Alt-hold timer live, disabling an open switcher, and numbered shortcuts in
number-only mode with the switcher disabled.
It sends no desktop keyboard input and does not access the clipboard. Set
`MYPIXELS_WORKSPACE_SCREENSHOT=1` to also save a desktop screenshot with `grim`.

`python3 tests/line_spacing_smoke.py` checks normal, +0.1, and -0.1 row spacing
in live PTYs across two windows and hidden workspaces. It checks terminal pixel
size reports, row counts, restoring the default, and preserving live font zoom.
It uses isolated children and a FIFO, without desktop keyboard or clipboard
input. Set `MYPIXELS_SPACING_SCREENSHOT=1` to save desktop screenshots with grim.

`xvfb-run -a -s "-screen 0 1600x1000x24" python3 tests/workspace_panel_smoke.py`
checks workspace cards, previews of parked terminals, exact-tab preview clicks,
genre search with real editor input, Escape focus release, empty-workspace
creation, the small-window layout, focused tabs beyond the first page, and
left/right overflow buttons. It leaves screenshots in
`/tmp/mypixels-workspace-panel-*`. Requires xdotool, ImageMagick and Pillow.

`xvfb-run -a -s "-screen 0 1600x1200x24" python3 tests/workspace_settings_smoke.py`
checks selecting a workspace, editing its name, choosing a palette color and
icon, changing the button display and position, saving, live button color, and
Cancel. It also checks that settings input stays out of the terminal and keeps
screenshots in `/tmp/mypixels-workspace-settings-*`.

The reviewed fork adds these checks:

- `xvfb-run -a -s "-screen 0 1600x1000x24" python3 tests/menu_window_smoke.py --x11`
  creates windows from the application menu, exercises both old and new windows,
  and checks that window removal does not misroute later actions. The isolated
  X server needs xdotool to focus its own test windows.
- `python3 tests/brightness_smoke.py` checks rendered ANSI, truecolor, runtime
  overrides, and previews. Use a private Wayland compositor with grim.
- `MYPIXELS_ISOLATED_GRAPHICS_TEST=1 python3 tests/transparency_smoke.py` checks
  opaque startup, live alpha changes, independent brightness, and both windows'
  Wayland opaque-region hints. Use a private compositor with a black background
  and grim; Gamescope with gamescopectl also works. `--x11` checks a private
  XWayland display under the same compositor.
- `python3 tests/yazi_image_smoke.py` checks real Yazi image previews, reuse of
  image ids, navigation, large tiled previews, and resizing. Use a private
  Wayland compositor with grim and Yazi installed; `--x11` uses its XWayland
  display. Checks select the most recent upload when Yazi reuses image ids.

Named-session launcher IPC checks (no display required):

```sh
cargo build --offline
python3 tests/persistent_ipc.py
```

This checks argument validation, concurrent clients attaching during session
startup, preserving the existing session rather than respawning commands, and
error reporting when the owner stops responding. It creates private Unix sockets.

For the display lifecycle, run `target/debug/mypixels -S test`, open several
tabs, rename/reorder them, change directories, and start a long-running command.
Scroll up in another tab, close the window, then reattach with the same command.
Check the command's PID and progress, tab order/titles/cwd, screen and scrollback,
and repeat the detach/attach cycle on both X11 and Wayland. A different name
should open an independent session. Without `-S`, closing the last window should
terminate its processes as before.

Automatic selection is also covered by `cargo test --offline persistent::tests`:
numbered sessions created explicitly and automatically share one namespace,
concurrent launches reserve a detached session only once and allocate distinct
numbers, interrupted attachments release their reservation, and stale session
records do not occupy numbers. These tests use isolated temporary registries
and real file locks/Unix sockets, with no display or desktop sessions involved.
For a UI check, close all windows of `mypixels -S '#2'`, keep `#1` and `#4`
attached, then launch `mypixels -A` three times: it should restore `#2`, then
create `#3` and `#5`. Verify that closing just one of multiple windows keeps
the session attached, and that `-A -S name` fails before opening a window.

Persistent launches (`-S` and `-A`) should return to the invoking shell after
the window appears, while the app remains alive. The session unit tests check
the background readiness handshake, separate Unix session/process group, null
standard I/O, inherited working directory, startup error propagation, and waiting
for restored windows without holding the registry lock. The headless CLI test
also checks real winit initialization failure reaches the launcher and releases
the session lock. For a desktop check, launch from a terminal, close that invoking
terminal, and verify the session window and its running commands remain usable.

The merge regression `tests/persistent_gui_smoke.py` runs on an isolated Wayland
compositor (or its XWayland server with `--x11`) and requires
`MYPIXELS_ISOLATED_SESSION_TEST=1`. It verifies that named background sessions
retain more detached windows than `graveyard_limit`, preserve tab/window order
and child PIDs, continue answering terminal queries while detached, and restore
interactive input. It also exercises the default windowless daemon and its
`--restore-window` command. For example, with a private runtime directory:

```sh
env -u DISPLAY -u WAYLAND_DISPLAY XDG_RUNTIME_DIR=/path/to/private/runtime \
  MYPIXELS_ISOLATED_SESSION_TEST=1 dbus-run-session -- \
  gamescope --backend headless --expose-wayland -W 1600 -H 1000 -- \
  python3 tests/persistent_gui_smoke.py
```

The test creates unique named sessions and a private D-Bus/MCP daemon; it never
uses `-A` against the desktop's shared named-session registry. Screenshots use
Gamescope's capture command or `grim` on a compositor with screencopy support.

Bash shell integration has a display-independent regression check:

```sh
python3 tests/bash_shell_integration.py
```

It starts isolated interactive Bash processes on PTYs and checks OSC 133 marker
order with dynamic multiline prompts, scalar/compound/array `PROMPT_COMMAND`
hooks, repeated sourcing through `.sh` and `.bash`, preserved exit statuses,
failure bells, existing `DEBUG` traps and `PS0` content. The Bash case in
`history_smoke.py` also uses a dynamically rebuilt prompt to verify that history
shows the command and can re-run it.
