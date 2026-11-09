# MyPixels Next — Settings Redesign Specification

**Status:** Implemented (`docs` copy of a proposed spec, adapted to this tree)
**Repository:** [volca02/mypixels-next](https://github.com/volca02/mypixels-next)
**Primary source:** [`src/egui_ui.rs`](../src/egui_ui.rs)
**Scope:** Settings information architecture and navigation. No configuration migration.

## 1. Problem statement

The settings pages were **Appearance**, **Window**, **Tabs**, **Terminal**, and **Keys**.
The `Tabs` page grouped unrelated settings: tab-strip layout and text, tab colors, tab-group
colors, new-tab behavior, the graveyard, and the Alt-held tab switcher. These describe
different user concerns and should not share a page just because tabs are involved.

The settings panel is an egui sidebar anchored to the right, width default 448 px (minimum
412 px, maximum 640 px). Its header, category selector, and pinned Save/Cancel controls
surround a scrollable body. Changes preview live, Save persists, Cancel reverts by reloading
saved configuration. Those behaviors are unchanged.

### Goals

1. Group settings by user intent and scope, not by internal component ownership.
2. Keep *Tabs* focused on the tab bar and new-tab lifecycle.
3. Consolidate colors and fonts in *Appearance*.
4. Give the tab switcher and workspaces a *Navigation* home.
5. Preserve every setting, YAML key, value range, default, and save/cancel behavior.
6. Stay usable in the existing compact right-side panel without extra navigation depth.

### Non-goals

- No new runtime functionality, config keys, or migration.
- No redesign of the terminal tab strip itself.
- No settings search, profiles, or presets.
- No full-screen settings or separate settings window.

## 2. Categories

Six top-level categories, in this order:

| Category | Purpose | Sections |
|---|---|---|
| **Appearance** | Visual customization, whatever it styles | Theme & colors; Fonts; Tab colors |
| **Window** | Window geometry and presentation | Window dimensions; Window behavior |
| **Tabs** | Tab-strip layout and tab lifecycle | Tab bar; Tab behavior |
| **Navigation** | Ways to move between terminals | Tab switcher; Workspaces |
| **Terminal** | Terminal emulation and content | Scrollback |
| **Keys** | What each shortcut is, and rebinding it | Groups from `KEY_GROUPS` |

**Ownership principle:** put each setting where users expect to find the *kind of change* they
want. Tab-strip background is an appearance setting even though its config field references
tabs. The switcher and workspaces are navigation mechanisms even though both show tabs.
How many terminals a closed tab survives as is tab lifecycle, not appearance.

## 3. Control map

Every control below exists in `config_window`. "Moves" is a relocation only: same config
field, same widget, same range.

### Appearance

| Control | Field | Was | Now |
|---|---|---|---|
| Theme | `theme` | Appearance → Colors | Appearance → Theme & colors |
| Background, Foreground, Selection, Cursor, Accent, Background tint | `background_color`, `foreground_color`, `selection_color`, `cursor_color`, `accent_color`, `bg_tint_color` | Appearance → Colors | Appearance → Theme & colors |
| Terminal font, Tab font | `term_font`, `tablist_font` | Appearance → Fonts | Appearance → Fonts |
| Line spacing | `line_spacing` | Appearance → Fonts | Appearance → Fonts |
| Tab strip, Add button | `tablist_background`, `add_button_color` | Appearance → Colors | Appearance → Tab colors |
| Group colors | `col` (each entry) | Tabs → Tab group colors | Appearance → Tab colors → Group colors |
| Workspace colors | `workspace_colors` (each entry) | Appearance → Colors | Appearance → Tab colors |

The theme control stays above manual overrides. `crate::theme::available()` and
`crate::theme::apply_name(...)` behavior is unchanged.

### Window

| Control | Field | Was | Now |
|---|---|---|---|
| Default width, Default height | `default_width`, `default_height` | Window → Window | Window → Window dimensions |
| Start maximized | `default_maximize` | Window → Window | Window → Window behavior |
| Transparency | `term_transparency` | Window → Window | Window → Window behavior |

`term_transparency` is the field the UI uses. It is unchanged.

### Tabs

| Control | Field | Was | Now |
|---|---|---|---|
| Show tab titles | `show_tab_titles` | Tabs → Tab strip | Tabs → Tab bar |
| Strip on top, Hide strip, Hide single tab, Multiline tabs | `tablist_top`, `tablist_hide`, `hide_single_tab`, `multiline_tabs` | Tabs → Tab strip | Tabs → Tab bar |
| Title length, Active title length, Shorten from start | `tab_title_max_length`, `tab_title_short_current`, `tab_title_shorten_start` | Tabs → Tab strip | Tabs → Tab bar |
| Focus new tabs | `focus_new_tabs` | Tabs → Tab strip | Tabs → Tab behavior |
| New tab position | `new_tab_position` (`next`, `end`, `first`) | Tabs → Tab strip | Tabs → Tab behavior |
| Graveyard keeps | `graveyard_limit` | Tabs → Tab strip | Tabs → Tab behavior |

### Navigation

| Control | Field | Was | Now |
|---|---|---|---|
| Tab switcher enabled | `tab_overview_on_alt` | Tabs → Tab switcher | Navigation → Tab switcher |
| Backdrop dim, Delay (ms), Preview scale | `tab_overview_opacity`, `tab_overview_delay_ms`, `tab_overview_scale` | Tabs → Tab switcher | Navigation → Tab switcher |
| Enable workspaces | `workspaces_enabled` | Tabs → Tab strip (labeled "Workspaces (Alt+Q…P)") | Navigation → Workspaces |

### Terminal

| Control | Field | Was | Now |
|---|---|---|---|
| Scrollback lines | `scrollback_lines` | Terminal → Scrollback | Terminal → Scrollback |

The page stays sparse. It is not padded with controls that belong elsewhere.

### Keys

Unchanged in content. Its sections come from `KEY_GROUPS` plus the generated Numbered tabs
and Workspaces groups. Rebinding stays app-owned: the app records the chord from winit input,
because egui does not see every key and takes some for its own clipboard handling.

## 4. Deviations from the original proposal

The proposal was written against an older tree. These differences are deliberate.

1. **Six categories, not five.** A Keys page already exists. The proposal's §10 argued
   against a Keyboard category on the grounds that shortcuts were only "separately
   discoverable in the application menu". That is no longer true: the Keys page lists every
   action, records chords, and resets or clears bindings per action. It is a first-class
   category and is navigated like the rest.
2. **No general tab color controls.** `tab_color_bg`, `tab_color_fg` and their selected
   variants are not exposed. The UI styles tabs through the workspace accent instead.
   `appearance_colors_exposes_workspaces_instead_of_general_tab_colors` locks that decision
   by asserting the four retired labels stay absent. The proposal listed them because the
   older UI had them; reviving them here would undo committed work.
3. **`col` group swatches stay exposed** as *Group colors* under Appearance → Tab colors,
   beside the workspace colors, so both color systems are in one place and one is labelled
   as the one that drives tab chrome.
4. **`workspaces_enabled` is Navigation, not Tabs.** The proposal had no such control. It
   turns a navigation mechanism on and off; the tab bar only shows the result. Its label used to
   embed a chord — "Workspaces (Alt+Q…P)" — which is no longer the default and may not be bound
   at all. The shortcut now lives only on the Keys page, where it is read from real bindings.
   Inside the Navigation page's *Workspaces* section the row reads "Enable workspaces", because a
   row that repeats its own heading says nothing.
5. **`graveyard_limit` is Tab behavior.** It decides how many closed tabs stay raisable.
   That is lifecycle, and the graveyard panel's own keys are on the Keys page.
6. **The selector is a 2 × 3 grid.** The proposal recommended two columns for five labels
   with one orphan. Six labels fill three rows exactly, so no spanning rule is needed.

## 5. Navigation UI

A compact two-column category grid sits under the title and above the scrollable body:

```text
Settings                                          [×]
Make this terminal your own.

[ Appearance   ] [ Window      ]
[ Tabs         ] [ Navigation  ]
[ Terminal     ] [ Keys        ]
──────────────────────────────────────────────────
Appearance

Theme & colors
  Theme                         [preset]
  Background                    [color]
Fonts
  Terminal font                 [chooser]
Tab colors
  Tab strip                     [color]
  Group colors                  [1] [2] ...
──────────────────────────────────────────────────
Changes preview instantly.
[ Cancel ]                                       [ Save ]
```

### Layout invariants

- Right-side anchoring and the existing width bounds are unchanged.
- Header, category selector, and Save/Cancel stay visible while the body scrolls.
- Only the page body scrolls vertically.
- `TermStyle` and the existing primitives only: `section`, `form_grid`, `row`, `checkbox_row`,
  `font_row`, `color_row`, `color_cell`. No new surfaces.
- A selected category is distinguishable by weight and shape, not color alone.
- No category name is truncated at the 412 px minimum.
- No horizontal scrolling.

### Keyboard and focus

- Clicking a category only changes the page. It does not save, revert, or emit PTY input.
- Opening Settings does not steal terminal keyboard focus unless an editor is active.
- Changing page dismisses the font popup and cancels an armed key capture. Color popups are
  keyed to their row's id, so they close with the page that owns them.
- Sidebar close semantics and mutual exclusion with History are unchanged.
- Per-page scroll ids stay derived from `settings_page as u8`, so each page keeps its own
  scroll position.

## 6. State behavior

**Live edit.** Preview is immediate. Switching pages keeps unsaved edits in `cfg`, including
colors a theme preset changed indirectly.

**Save.** Persists every change made in the session across every page, closes the panel, and
preserves unrelated YAML and comments. With no binding edited in the UI, the `bindings:` block
is left byte-for-byte alone; a recorded chord rewrites that block and nothing else.

**Cancel.** Reloads the persisted configuration and closes the panel, from whatever page it
is pressed on.

**Runtime.** Switching category must not create tabs, restart a PTY, reset a terminal, or
rebuild a renderer. Settings and History stay mutually exclusive. The terminal keeps rendering
while the sidebar is open. Theme discovery caching is kept.

## 7. Code shape

In `src/egui_ui.rs`:

```rust
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum SettingsPage {
    #[default]
    Appearance,
    Window,
    Tabs,
    Navigation,
    Terminal,
    Keys,
}

const SETTINGS_PAGES: &[(SettingsPage, &str)] = &[
    (SettingsPage::Appearance, "Appearance"),
    (SettingsPage::Window, "Window"),
    (SettingsPage::Tabs, "Tabs"),
    (SettingsPage::Navigation, "Navigation"),
    (SettingsPage::Terminal, "Terminal"),
    (SettingsPage::Keys, "Keys"),
];
```

`SETTINGS_PAGES` is the single definition used by the selector. `st.settings_page` remains the
only page-selection state. `ConfigEvents` keeps its `apply`, `save`, `revert` shape.

Each page renders through its own function, so a page's sections are readable in one screen
and the match arm is a dispatch:

```text
appearance_page   Theme & colors, Fonts, Tab colors
window_page       Window dimensions, Window behavior
tabs_page         Tab bar, Tab behavior
navigation_page   Tab switcher, Workspaces
terminal_page     Scrollback
keys_page         KEY_GROUPS plus generated groups
```

No generic rendering framework. Plain functions taking `&mut Config`, `&mut UiState`,
`&mut ConfigEvents`, and `&TermStyle`.

## 8. Testing

### Rust

- `settings_pages_keep_save_and_cancel_visible_after_resize` iterates `SETTINGS_PAGES`, so a
  new category is covered by adding it to the list rather than by editing test bodies.
- `every_category_is_selectable_and_owns_its_controls` asserts, per page, that its own control
  labels are present, no other page's labels appear, and all six categories are selectable from
  every page. It locks the grouping: a control that drifts to another page fails here. Control
  labels are used rather than section titles because the Keys page has a "Workspaces" group of
  chords and Navigation has a "Workspaces" section of settings — titles are not unique, labels
  are.
- `category_labels_survive_the_minimum_panel_width` checks every category label at 412, 448 and
  640 px: unclipped and inside the window.
- `appearance_colors_exposes_workspaces_instead_of_general_tab_colors` still asserts the
  retired general tab-color labels stay absent.
- Viewport sizes **1280×850**, **900×540**, **640×420**: labels, inputs, and pinned actions
  stay inside the window and unclipped.

### Headless smoke tests

The category selector's geometry is no longer guessed by the harness. The app logs the center
of each category cell when that geometry changes:

```text
settings nav: Appearance=949,129 Window=1159,129 Tabs=949,171 Navigation=1159,171 Terminal=949,213 Keys=1159,213
```

`tests/settings_smoke.py` and `tests/panel_keyboard_smoke.py` parse that line and click a
category by name. A layout change moves the numbers; the tests do not.

Coverage kept:

1. Visit all six categories, screenshot each.
2. Edit a Window value, Save, assert the persisted YAML and that leading comments survive.
3. Edit on one page, Cancel, assert the file is byte-identical to the save.
4. Open font and color popups, switch category, assert no orphan popup.
5. Settings and History mutual exclusion, and matching sidebar backgrounds.
6. Resize, then confirm pinned actions are reachable and no input leaked into the PTY.

### Commands

```bash
cargo test
cargo build
xvfb-run -a python3 tests/settings_smoke.py
xvfb-run -a python3 tests/panel_keyboard_smoke.py
xvfb-run -a python3 tests/workspace_smoke.py
```

Run headless suites with `WAYLAND_DISPLAY` unset, so they cannot touch a live session.

## 9. Acceptance criteria

- [x] Exactly six top-level categories: Appearance, Window, Tabs, Navigation, Terminal, Keys.
- [x] Every control reachable, with unchanged config mapping, ranges, and defaults.
- [x] All font and color controls, including group and workspace colors, under Appearance.
- [x] Tabs contains only Tab bar and Tab behavior.
- [x] Switcher and workspace settings are under Navigation.
- [x] Window and Terminal stay coherent, and Terminal stays sparse.
- [x] Selector usable at the 412 px minimum, with no truncated label.
- [x] Body scrolls alone; selector and Save/Cancel stay pinned.
- [x] Live preview, cross-page Save, and cross-page Cancel work.
- [x] Changing category never reaches the PTY.
- [x] Changing page closes its transient popups and any armed chord capture.
- [x] Settings and History remain mutually exclusive.
- [x] Rust layout tests and the headless smoke tests pass.
- [x] No migration and no terminal runtime change required.

## 10. Sources

- [`src/egui_ui.rs`](../src/egui_ui.rs) — page enum, selector, page renderers, UI tests.
- [`src/config.rs`](../src/config.rs) — config fields, defaults, binding storage.
- [`src/ui_style.rs`](../src/ui_style.rs) — shared primitives and icons.
- [`tests/settings_smoke.py`](../tests/settings_smoke.py) — Save/Cancel, popups, navigation, resize.
- [`tests/panel_keyboard_smoke.py`](../tests/panel_keyboard_smoke.py) — keyboard routing regressions.
- [`README.md`](../README.md) — features and configuration architecture.
