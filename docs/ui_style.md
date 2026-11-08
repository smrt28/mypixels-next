# Terminal UI styling

`src/ui_style.rs` owns the reusable visual language: the theme palette,
typography, vector icons, outlined surfaces, number badges, action rows, form
sections, toggles, and picker density. Change these components here so history,
settings, the switcher, and the tab strip stay consistent. Screen modules own
content, state, and layout rather than copies of these treatments.

Menus use `TermStyle::menu_frame` for the theme-derived background, thin border,
nine-point corners and padding. Wrap their content in `menu_contents` for shared
typography and density; use `menu_item` and `menu_separator` for rows and groups.
Application, terminal context and history overflow menus all use these components.
Long menus scroll within the window. `action_row` is the underlying reusable
control, also used by history details: optional vector icon, truncated label,
right-aligned shortcut, muted disabled state, and accent outline on hover/focus.
Add or change this treatment in the library rather than drawing local copies.

Popup anchors are logical points. Convert a native physical pointer with
`egui_ui::popup_position` and the window's current `pixels_per_point` once at the
event boundary. Strip-relative anchors use the strip height divided by that same
scale. Popup contents own keyboard input until dismissal.

`TermStyle` resolves the effective terminal theme. `TabStyle` adds the strip's
state colors and preserves explicit tab color overrides; legacy default colors
adopt the shared chrome. Font previews and tab labels use the same installed
font provider, so the configured tab font remains effective.

`src/tab_bar.rs` lays out and draws the compact strip through egui. It provides
clipped geometry in logical points to the window controller. The controller
converts this to physical pixels and retains native drag/reorder/detach input;
no tab labels or backgrounds pass through the terminal's glyphon/wgpu renderer.
The strip is at least 28 logical pixels high and grows for larger configured
fonts. Horizontal scrolling keeps the active tab and close control reachable.

Sidebars and the tab strip remain passive keyboard surfaces. Explicitly focused
text and number editors own editing input; panel shortcuts stay global.

## Shared motion and previews

Components declare target states through `transition`, `animated_rect`, and
`animated_rect_in`. egui's animation manager owns interpolation, interrupted
transitions and repaint requests. The shared policy is 160 ms, with cubic easing
for state changes and linear geometry movement. `animated_rect_in` keeps a
selection inside its viewport. Stable tab IDs identify movement; grid-local
coordinates keep scrolling separate from tab reordering. New surfaces snap to
their initial layout; position and size changes share the same interpolation.
The tab bar and switcher both use moving selection outlines and fading state
fills. Add future motion through these helpers instead
of per-widget timers or frame loops.

The window controller preserves both immediate and delayed egui repaint
deadlines, including tooltip delays. It wakes the event loop when needed rather
than relying on unrelated terminal output or cursor blinking.

Switcher and hover previews share `preview_image` and persistent GPU thumbnails
keyed by tab ID. Changed frames render in batches of at most two; idle candidate
preparation is limited to five times a second, with active/hovered tabs first.
The GPU box-filter downsamples at native display density, and egui samples those
textures directly. Closing the switcher keeps the cache; closing/moving tabs
releases their renderer registrations. CPU readback is reserved for explicit
copy-as-image exports.
