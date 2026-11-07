Mypixels
========

A tabbed terminal emulator written in Rust: `alacritty_terminal`
drives the emulation and pty, `glyphon` renders on `wgpu` in a
`winit` window, and an `egui` layer draws the tab strip, config
panel, font dialog and tab switcher.

This is a from-scratch rewrite of the original Python/GTK/VTE
terminal of the same name, kept unchanged at
[`legacy/mypixels`](legacy/mypixels) (a single script,
GPL-3.0-or-later) as the behavioural reference this rewrite
reproduces, down to the Alt-held tab switcher.

Design principles, carried over from the original:

* screen space effectivity - no menus or status lines that can't be
  hidden or take up too much space
* all keyboard operation

Building
========

    cargo build --release        # target/release/mypixels
    cargo test                   # unit tests, no display needed

Headless UI smoke tests:

    MYPIXELS_BIN=$PWD/target/release/mypixels xvfb-run -a python3 tests/preview_smoke.py

Usage
=====

    usage: mypixels [options] [command args...]
      -n, --new-window   open a new window instead of joining the session
      -s, --standalone   run a private instance: never join or host a session

Without flags, a mypixels window already running on this display is
found over D-Bus and asked to open a tab for the command, in the
current directory; typed inside a window, the tab opens in that same
window. With no session to answer - or with `-n` - this process
becomes a window of the session.

`mypixels last [n] [--copy]` prints (or clipboard-copies) the output
of the last n commands from the running session's memory pool - a
query, not a launch.

Features
========

* Tab switcher with live previews - hold Alt: each tab is a scaled,
  live view of its own terminal; arrows move the selection.
* Entanglement - one command, many tabs: toggle a tab into the group
  (Alt+Shift+*) and keystrokes land in all of them.
* Tab colorization - mark a tab with a colour so it can be easily
  found (Alt+Shift+{ / }).
* All keyboard - Alt+Left/Right/Up/Down/Home/End move between tabs,
  Ctrl+Alt+Z new tab, Ctrl+Alt+X close tab, Ctrl+Shift+R reloads the
  config, Ctrl+Shift+C/V copy and paste (the GTK bindings),
  Shift+Insert pastes the primary selection.
* Kitty graphics protocol - PNG images (`icat`) render as textured
  quads in the grid and scrollback.
* Kitty keyboard protocol - full disambiguation, with real key
  releases.
* Command observability - with the shell integration scripts (OSC
  133 marks) every command is captured with its output, exit status
  and location: Alt+Shift+H opens the history panel, and
  `mypixels last` / Ctrl+Shift+L read the last output from the shell.
* MCP endpoint - a read-only Model Context Protocol server on a
  per-display unix socket: LLM tooling lists the session's windows
  and tabs and reads captured command output without a shell.
* Config panel and font dialog (Alt+Shift+C); the file reloads live.

Configuration
=============

Config is `$XDG_CONFIG_HOME/mypixels/config.yml` (the legacy
`~/.mypixels` is read until an XDG file exists). Every key is
editable in the config panel; key bindings are rebindable under
`bindings:`.

Themes are YAML colour sets, discovered in this repo's `themes/`,
`~/.config/mypixels/themes`, `~/.local/share/mypixels/themes` and
the XDG data dirs; see [themes/README.md](themes/README.md).

For command capture, source the matching
`scripts/shell_integration.{bash,zsh,fish}` from your shell rc.

Layout
======

* `src/` - the terminal (Rust)
* `scripts/` - shell integration, test harnesses
* `tests/` - unit and headless UI smoke tests
* `themes/` - bundled themes
* `legacy/mypixels` - the original Python/GTK/VTE implementation
