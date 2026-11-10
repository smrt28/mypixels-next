# Reviewed upstream integration

Source: [smrt28/mypixels-next](https://github.com/smrt28/mypixels-next),
`main` at `839bf3c4575c18398fc4c5b4aa68f1c1dc8e1d16`.
The reviewed changes were authored by **Ondrej Holecek <ondrej@vivaldi.com>**.
They were selectively ported onto the current Rust UI so the newer settings,
workspace navigation, previews, history, and damage handling remain intact.

| Upstream commits | Result |
| --- | --- |
| `a67ed8b`, `a8ffe77` | Workspace, tab-title, and line-spacing behavior already covered by newer implementations here. |
| `32fcd73`, `e71ab7f` | Optional compact tab bar with number padding, existing typography, theme colors, workspace indicator, and close controls. Default spacing remains unchanged. |
| `f013de2` | Terminal color adjustment, named **Terminal brightness** in Appearance settings. Defaults to zero and leaves theme/interface colors intact. |
| `3423914` | Live transparency updates for every window, with an alpha-capable X11 visual and updated Wayland opaque regions. |
| `71873da` | Window actions carry their source window identity; redraw restores the event's window after deferred actions. |
| `b9adc45` | Applicable warning/deprecation cleanup and corrected test registration, preserving newer functionality. |
| `06bb505` | Remove the tab-strip clock and its `tab_prepend` config field, as requested. History timestamps and animation timing remain. |
| `839bf3c` | Kitty image crop/offset handling, placement replacement, quiet responses, image generations, and Yazi tile rendering. |

The compact strip was visually reviewed with Dracula and Catppuccin Latte.
It uses the existing chrome and remains optional. Brightness is also optional;
no upstream palette or global interface redesign was adopted.

Graphics adaptations follow the [Kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/):
source crop coordinates differ from display offsets, ordinary images preserve
aspect ratio within cell extents, and retransmission clears prior placements.
Explicitly cropped one-cell Yazi tiles fill their cell to avoid rounding seams.
Active, parked-workspace, and graveyard previews share the placement geometry.

Regression work also fixed native tab detach losing its source window when the
new window became current, and preserved graveyard damage until frame capture.

## Validation

- `cargo test --offline`: 199 passed, 1 ignored.
- `cargo build --offline` and `git diff --check`.
- Graphical checks: workspace panel, workspace switching, settings, application
  menus/new windows, compact and regular tabs, overflow/reorder/native detach,
  terminal brightness, damage rendering, graveyard, and actual Yazi previews.
- Live transparency checked on isolated native Wayland and X11 displays.
