//! Configuration with defaults ported from the Python version's config dict,
//! plus keybinding parsing ("MOD1+SHIFT+Left", "CONTROL+ALT+x", "MOD1+_1" ...).

use winit::keyboard::{Key, ModifiersState, NamedKey};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn to_f32(&self) -> [f32; 3] {
        [self.0 as f32 / 255.0, self.1 as f32 / 255.0, self.2 as f32 / 255.0]
    }
}

impl Rgb {
    /// Legacy GDK configs also use #rrrrggggbbbb (16 bits per channel).
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('#');
        if !s.is_ascii() { return None; }
        if s.len() == 3 {
            let v = u32::from_str_radix(s, 16).ok()?;
            let (r, g, b) = ((v >> 8 & 0xf) as u8, (v >> 4 & 0xf) as u8, (v & 0xf) as u8);
            return Some(Rgb(r * 17, g * 17, b * 17));
        }
        if s.len() == 6 {
            let v = u32::from_str_radix(s, 16).ok()?;
            return Some(Rgb((v >> 16 & 0xff) as u8, (v >> 8 & 0xff) as u8, (v & 0xff) as u8));
        }
        if s.len() == 12 {
            let channel = |part| u16::from_str_radix(part, 16).ok().map(|v| ((v as u32 + 128) / 257) as u8);
            return Some(Rgb(channel(&s[0..4])?, channel(&s[4..8])?, channel(&s[8..12])?));
        }
        None
    }
}

pub fn parse_hex(s: &str) -> Rgb {
    Rgb::parse(s).unwrap_or(Rgb(0, 0, 0))
}

/// "Cascadia Code 9" -> ("Cascadia Code", 9.0). Pango-style: the size is
/// the last whitespace-separated token; everything before is the family.
pub fn parse_font(s: &str) -> (String, f32) {
    let s = s.trim();
    match s.rfind(char::is_whitespace) {
        Some(idx) if idx + 1 < s.len() => {
            let size: f32 = s[idx + 1..].trim().parse().unwrap_or(12.0);
            (s[..idx].trim().to_string(), size)
        },
        _ => (s.to_string(), 12.0),
    }
}

/// Colon-separated hex colors (terminator/wal format). Takes the first 16;
/// more than 16 (256-color palettes) is fine. Requires at least 16 so a
/// truncated palette doesn't silently pad the rest with black.
#[cfg(test)]
fn parse_palette(s: &str) -> Option<[Rgb; 16]> {
    crate::theme::parse_palette(&serde_yaml::Value::String(s.to_string())).ok()
}

#[derive(Clone)]
pub struct Config {
    // tablist
    pub tablist_background: Rgb,
    /// Workspace identity colors, W1 through W10. Tab chrome is derived from these.
    pub workspace_colors: [Rgb; 10],
    pub tab_color_bg: Rgb,
    pub tab_color_bg_selected: Rgb,
    pub tab_color_fg: Rgb,
    pub tab_color_fg_selected: Rgb,
    pub tab_color_entangle: Rgb,
    pub tab_color_idx: Rgb,
    pub add_button_color: Rgb,
    pub tab_title_max_length: usize,
    pub tab_title_short_current: usize,
    pub tab_shorten_str: String,
    // terminal
    pub foreground_color: Rgb,
    pub background_color: Rgb,
    pub selection_color: Rgb,
    pub cursor_color: Rgb,
    pub accent_color: Rgb,
    /// The selected theme (preset name, `wal`, or a YAML path); empty means
    /// none. Kept as written so the picker and the file agree after a save.
    pub theme: String,
    /// External color sources monitored by the theme worker, never by draw().
    pub theme_files: Vec<std::path::PathBuf>,
    /// 16-color palette (None = alacritty defaults). Terminal runtime OSC 4
    /// changes override these per color.
    pub palette: Option<[Rgb; 16]>,
    pub term_font: String,
    /// Extra row height as a fraction of the font's normal line height.
    pub line_spacing: f32,
    pub tablist_font: String,
    /// Scrollback depth. Costs width*24+40 bytes per line (full-width
    /// Vec<Cell>), so raising this is a real RAM decision; the command
    /// output pool (`history_bytes`) is what keeps deep history cheap.
    pub scrollback_lines: usize,
    /// Global budget for the command-output pool (all tabs, LRU-evicted
    /// oldest closed block first). 32 MB typical-use is never approached.
    pub history_bytes: usize,
    /// A single command's captured output is capped here, with an explicit
    /// truncation marker - one `ls -lR /` cannot eat the whole pool.
    pub history_block_bytes: usize,
    // tab color groups
    pub col: [Rgb; 8],
    // behavior
    /// Enables the Alt-held visual tab switcher. Numbered shortcuts work independently.
    pub tab_overview_on_alt: bool,
    pub show_tab_titles: bool,
    /// Ten independent tab sets per window. Off by default.
    pub workspaces_enabled: bool,
    /// Opt-in alternative workspace shortcut: Alt+Q..P. Off by default because
    /// bare Alt+letter collides with shell and editor bindings (word-wise
    /// backward word, readline shortcuts). The standard scheme is
    /// Alt+Shift+digit, which shares no range with those.
    pub workspace_letter_keys: bool,
    pub tab_overview_opacity: f32,
    pub tab_overview_delay_ms: u32,
    /// Switcher preview scale: the tab's frame shrunk by this factor (1.0 =
    /// window size). The preview keeps the terminal's own aspect ratio, so a
    /// single factor is the whole setting.
    pub tab_overview_scale: f32,
    pub focus_new_tabs: bool,
    /// Where a new tab lands: `next` (right of the current tab, the
    /// default), `end` (after the last tab) or `first` (at the start).
    pub new_tab_position: String,
    pub hide_single_tab: bool,
    /// How many closed tabs the graveyard keeps running. Buried tabs stay
    /// alive (pty, child, scrollback) until raised; the oldest dies when
    /// the limit is exceeded.
    pub graveyard_limit: usize,
    /// OSC 133 semantic shell integration: fold the shell's prompt/command
    /// marks into navigable command blocks (scripts/shell_integration.*).
    pub osc133: bool,
    /// kitty-compatible identity in TERMINAL ("mypixels-kitty") for
    /// allowlist-style detectors like broot. Never touches TERM (terminfo
    /// honesty); the claim is backed by passing kitty's own icat
    /// conformance. See docs/graphics notes.
    pub kitty_compatible: bool,
    pub multiline_tabs: bool,
    pub tablist_top: bool,
    pub tablist_hide: bool,
    pub tab_title_shorten_start: bool,
    pub default_tab_title: String,
    pub default_width: u32,
    pub default_height: u32,
    pub default_maximize: bool,
    /// Amount of *transparency*: 0 = opaque, 1 = fully see-through. Named
    /// for what it does; the inverted `term_opacity` spelling it replaces is
    /// still parsed (see term_opacity_legacy).
    pub term_transparency: f32,
    /// Deprecated `term_opacity:` value, kept only so a file predating the
    /// rename keeps working. The new key always wins when both are present.
    term_opacity_legacy: Option<f32>,
    transparency_explicit: bool,
    pub bg_tint_color: Rgb,
    pub tab_format: String,
    pub tab_prepend: Option<String>,
    pub tab_append: Option<String>,
    pub autohide_delay: i32,
    // keybindings
    pub bindings: Vec<(String, Vec<Binding>)>,
}

impl Default for Config {
    fn default() -> Self {
        let mut bindings: Vec<(String, Vec<Binding>)> = Vec::new();
        let mut bind = |k: &str, v: &[&str]| {
            let list: Vec<Binding> = v.iter().filter_map(|s| Binding::parse(s)).collect();
            bindings.push((k.to_string(), list));
        };

        bind("move_prev_tab", &["MOD1+SHIFT+Left"]);
        bind("move_next_tab", &["MOD1+SHIFT+Right"]);
        bind("move_first_tab", &["MOD1+SHIFT+Home"]);
        bind("move_last_tab", &["MOD1+SHIFT+End"]);
        bind("prev_tab", &["MOD1+Left", "MOD1+SHIFT+f"]);
        bind("next_tab", &["MOD1+Right", "MOD1+SHIFT+b"]);
        bind("prev_tab_row", &["MOD1+Up"]);
        bind("next_tab_row", &["MOD1+Down"]);
        bind("first_tab", &["MOD1+Home"]);
        bind("last_tab", &["MOD1+End"]);
        bind("close_tab", &["CONTROL+ALT+x", "CONTROL+ALT+q"]);
        bind("prev_command", &["CONTROL+ALT+Up"]);
        bind("next_command", &["CONTROL+ALT+Down"]);
        bind("reopen_tab", &["CONTROL+SHIFT+z"]);
        bind("graveyard", &["CONTROL+SHIFT+g"]);
        bind("new_tab", &["CONTROL+ALT+z", "CONTROL+SHIFT+t"]);
        bind("new_window", &["CONTROL+SHIFT+n"]);
        bind("send_tab_to_window", &["MOD1+SHIFT+w"]);
        // Copy/paste follow the VTE and GNOME Terminal bindings the original gets
        // from GTK, so the fingers that already know them keep working. Selecting
        // text feeds the *primary* buffer separately and needs no key.
        bind("copy", &["CONTROL+SHIFT+c"]);
        bind("paste", &["CONTROL+SHIFT+v"]);
        // Copy the last command's captured output (the OSC 133 pool) without
        // leaving the terminal - the in-app twin of `mypixels last --copy`.
        bind("copy_last_output", &["CONTROL+SHIFT+l"]);
        // The command-history panel (list, filters, search, actions).
        bind("show_history", &["MOD1+SHIFT+h"]);
        bind("select_all", &["CONTROL+SHIFT+a"]);
        bind("paste_primary", &["SHIFT+Insert"]);
        bind("scroll_page_up", &["SHIFT+Page_Up"]);
        bind("scroll_page_down", &["SHIFT+Page_Down"]);
        bind("scroll_bottom", &["CONTROL+SHIFT+End"]);
        bind("shrink_font", &["MOD1+SHIFT+KP_Subtract"]);
        bind("enlarge_font", &["MOD1+SHIFT+KP_Add"]);
        bind("toggle_hide_tabs", &["CONTROL+ALT+v"]);
        bind("prev_color", &["MOD1+SHIFT+braceleft"]);
        bind("next_color", &["MOD1+SHIFT+braceright"]);
        bind("toggle_entangle", &["MOD1+SHIFT+KP_Multiply"]);
        bind("rename_tab", &["MOD1+SHIFT+n"]);
        bind("show_search", &["MOD1+SHIFT+s"]);
        bind("show_search_all", &["MOD1+SHIFT+g"]);
        bind("search_prev", &["MOD1+SHIFT+a"]);
        bind("search_next", &["MOD1+SHIFT+d"]);
        bind("config_reload", &["CONTROL+SHIFT+r"]);
        bind("show_config", &["MOD1+SHIFT+c"]);
        for (i, d) in (0..10).zip(["_1", "_2", "_3", "_4", "_5", "_6", "_7", "_8", "_9", "_0"]) {
            bind(&format!("goto_tab_{i}"), &[&format!("MOD1+{d}")]);
        }

        for (i, digit) in ["_1", "_2", "_3", "_4", "_5", "_6", "_7", "_8", "_9", "_0"].iter().enumerate() {
            // Alt+Shift+digit: the standard scheme. Bare Alt+letter is opt-in
            // (`workspace_letter_keys`) because it masks shell key combinations.
            bind(&format!("goto_workspace_{i}"), &[&format!("MOD1+SHIFT+{digit}")]);
        }

        Self {
            tablist_background: parse_hex("#303030"),
            workspace_colors: [
                "#5e9eff", "#f07b72", "#71c58c", "#e7b85a", "#b792ed",
                "#5fc7d0", "#e998be", "#ec9756", "#bbc96a", "#95a8c4",
            ].map(parse_hex),
            tab_color_bg: parse_hex("#303030"),
            tab_color_bg_selected: parse_hex("#505020"),
            tab_color_fg: parse_hex("#888888"),
            tab_color_fg_selected: parse_hex("#ffffff"),
            tab_color_entangle: parse_hex("#ff0000"),
            tab_color_idx: parse_hex("#808080"),
            add_button_color: parse_hex("#ffa500"),
            tab_title_max_length: 16,
            tab_title_short_current: 32,
            tab_shorten_str: "\u{2026}".to_string(),
            foreground_color: parse_hex("#bbbbbb"),
            background_color: parse_hex("#000000"),
            selection_color: parse_hex("#3584e4"),
            cursor_color: Rgb(140, 140, 153),
            accent_color: Rgb(92, 191, 191),
            theme: String::new(),
            theme_files: Vec::new(),
            palette: None,
            term_font: "Terminus 8".to_string(),
            line_spacing: 0.0,
            tablist_font: "Liberation Mono 7".to_string(),
            scrollback_lines: 5000,
            history_bytes: 32 * 1024 * 1024,
            history_block_bytes: 8 * 1024 * 1024,
            col: [
                parse_hex("#b58900"),
                parse_hex("#cb4b16"),
                parse_hex("#dc322f"),
                parse_hex("#d33682"),
                parse_hex("#6c71c4"),
                parse_hex("#268bd2"),
                parse_hex("#2aa198"),
                parse_hex("#859900"),
            ],
            tab_overview_on_alt: true,
            show_tab_titles: true,
            workspaces_enabled: false,
            workspace_letter_keys: false,
            tab_overview_opacity: 0.3,
            tab_overview_delay_ms: 500,
            tab_overview_scale: 0.25,
            focus_new_tabs: true,
            new_tab_position: "next".to_string(),
            hide_single_tab: false,
            graveyard_limit: 5,
            osc133: true,
            kitty_compatible: true,
            multiline_tabs: true,
            tablist_top: true,
            tablist_hide: false,
            tab_title_shorten_start: false,
            default_tab_title: String::new(),
            default_width: 640,
            default_height: 480,
            default_maximize: false,
            term_transparency: 0.1,
            term_opacity_legacy: None,
            transparency_explicit: false,
            bg_tint_color: parse_hex("#140008"),
            tab_format: "<span %(color)s>%(tcorner)s%(entangled)s%(tname)s]</span>".to_string(),
            tab_prepend: Some("clock".to_string()),
            tab_append: None,
            autohide_delay: -1,
            bindings,
        }
    }
}

impl Config {
    /// Startup can fall back; explicit/live reload uses try_load to keep the
    /// last valid configuration when a file is incomplete or invalid.
    pub fn load() -> Config {
        Self::try_load().unwrap_or_else(|e| { log::warn!("{e}; using defaults"); Self::default() })
    }

    /// The file the configuration is read from (and written to on save):
    /// $MYPIXELS_CONFIG (or the deprecated $SPIKE_CONFIG), else
    /// $XDG_CONFIG_HOME/mypixels/config.yml
    /// (~/.config/mypixels/config.yml by default). A legacy ~/.mypixels that
    /// predates the move keeps working while the XDG file does not exist --
    /// nobody's configuration gets orphaned by the relocation, and saving
    /// writes back to the very file that was loaded.
    pub fn config_path() -> Option<std::path::PathBuf> {
        Self::config_path_in(
            Self::env_override("MYPIXELS_CONFIG"),
            dirs::config_dir().as_deref(),
            dirs::home_dir().as_deref(),
        )
    }

    fn config_path_in(
        override_: Option<std::path::PathBuf>,
        xdg: Option<&std::path::Path>,
        home: Option<&std::path::Path>,
    ) -> Option<std::path::PathBuf> {
        if let Some(p) = override_.as_deref() {
            return Some(p.to_path_buf());
        }
        let modern = xdg?.join("mypixels").join("config.yml");
        if modern.is_file() {
            return Some(modern);
        }
        let legacy = home?.join(".mypixels");
        if legacy.is_file() {
            return Some(legacy);
        }
        Some(modern)
    }

    /// A path environment override, current name first. The SPIKE_ spellings
    /// predate the rename from prototype binary to product and still work --
    /// muscle memory is a real interface.
    pub(crate) fn env_override(name: &str) -> Option<std::path::PathBuf> {
        let legacy = format!("SPIKE_{}", name.strip_prefix("MYPIXELS_")?);
        std::env::var_os(name)
            .or_else(|| std::env::var_os(legacy))
            .map(std::path::PathBuf::from)
    }

    pub fn try_load() -> Result<Config, String> {
        let path = Self::config_path().ok_or("Cannot determine configuration path")?;
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("Cannot read {}: {e}", path.display())),
        };
        Self::from_text(&text, path.parent().unwrap_or(std::path::Path::new(".")))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn terminal_line_height(&self, normal_height: f32) -> f32 {
        (normal_height * (1.0 + self.line_spacing)).round().max(1.0)
    }

    pub fn from_text(text: &str, base: &std::path::Path) -> Result<Config, String> {
        let text = dedupe_top_level(&text.replace("!!python/tuple", ""));
        let mut value: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|e| format!("Invalid config YAML: {e}"))?;
        if value.is_null() { value = serde_yaml::Value::Mapping(serde_yaml::Mapping::new()); }
        if !value.is_mapping() { return Err("Configuration must be a YAML mapping".into()); }
        let mut cfg = Self::default();
        crate::theme::configure(&mut cfg, &value, base)?;
        // Legacy `term_opacity:` feeds the renamed setting only when the
        // file does not carry the new key - file order cannot matter, and a
        // save (which only ever writes the new key) supersedes the stale
        // old line that replace-in-place cannot remove.
        if let Some(legacy) = cfg.term_opacity_legacy {
            if !cfg.transparency_explicit {
                cfg.term_transparency = legacy;
            }
        }
        cfg.theme_files.sort();
        cfg.theme_files.dedup();
        cfg.apply_workspace_letter_keys();
        Ok(cfg)
    }

    /// Add the opt-in Alt+Q..P form alongside the standard Alt+Shift+digit
    /// scheme. Runs after a file is merged, because the default bindings are
    /// built before any setting is read. A binding the file states outright
    /// is left alone.
    fn apply_workspace_letter_keys(&mut self) {
        if !self.workspace_letter_keys {
            return;
        }
        for (i, letter) in WORKSPACE_LETTERS.chars().enumerate() {
            let action = format!("goto_workspace_{i}");
            let Some(letter_binding) = Binding::parse(&format!("MOD1+{letter}")) else { continue };
            if let Some((_, list)) = self.bindings.iter_mut().find(|(a, _)| *a == action) {
                if !list.iter().any(|b| b.key == letter_binding.key && b.mods == letter_binding.mods) {
                    list.push(letter_binding.clone());
                }
            } else {
                self.bindings.push((action, vec![letter_binding]));
            }
        }
    }

    /// Persist the fields the configuration UI edits, in place: each
    /// top-level key's value line is replaced, anything else (comments,
    /// bindings, palettes, keys the UI does not touch) stays byte for
    /// byte. Keys the file does not have yet are appended.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path().ok_or("Cannot determine configuration path")?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &std::path::Path) -> Result<(), String> {
        // First save into a fresh XDG location has to create it.
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = std::fs::read_to_string(path).unwrap_or_default();
        let out = replace_top_level_values(&text, &Self::ui_keys(self));
        std::fs::write(path, out).map_err(|e| format!("Cannot write {}: {e}", path.display()))
    }

    /// (yaml key, value text) for every field the configuration UI can change.
    fn ui_keys(&self) -> Vec<(String, String)> {
        let rgb = |r: &Rgb| format!("'#{:02x}{:02x}{:02x}'", r.0, r.1, r.2);
        let str_ = |s: &str| format!("'{}'", s.replace('\'', "''"));
        let mut keys = vec![
            ("term_font".into(), str_(&self.term_font)),
            ("line_spacing".into(), self.line_spacing.to_string()),
            ("tablist_font".into(), str_(&self.tablist_font)),
            ("term_transparency".into(), self.term_transparency.to_string()),
            ("scrollback_lines".into(), self.scrollback_lines.to_string()),
            ("history_bytes".into(), self.history_bytes.to_string()),
            ("history_block_bytes".into(), self.history_block_bytes.to_string()),
            ("background_color".into(), rgb(&self.background_color)),
            ("foreground_color".into(), rgb(&self.foreground_color)),
            ("selection_color".into(), rgb(&self.selection_color)),
            ("cursor_color".into(), rgb(&self.cursor_color)),
            ("accent_color".into(), rgb(&self.accent_color)),
            ("bg_tint_color".into(), rgb(&self.bg_tint_color)),
            ("tablist_background".into(), rgb(&self.tablist_background)),
            ("add_button_color".into(), rgb(&self.add_button_color)),
            ("tab_title_max_length".into(), self.tab_title_max_length.to_string()),
            ("tab_title_short_current".into(), self.tab_title_short_current.to_string()),
            ("tab_overview_on_alt".into(), self.tab_overview_on_alt.to_string()),
            ("show_tab_titles".into(), self.show_tab_titles.to_string()),
            ("workspaces_enabled".into(), self.workspaces_enabled.to_string()),
            ("workspace_letter_keys".into(), self.workspace_letter_keys.to_string()),
            ("tab_overview_opacity".into(), self.tab_overview_opacity.to_string()),
            ("tab_overview_delay_ms".into(), self.tab_overview_delay_ms.to_string()),
            ("tab_overview_scale".into(), self.tab_overview_scale.to_string()),
            ("focus_new_tabs".into(), self.focus_new_tabs.to_string()),
            ("new_tab_position".into(), str_(&self.new_tab_position)),
            ("hide_single_tab".into(), self.hide_single_tab.to_string()),
            ("graveyard_limit".into(), self.graveyard_limit.to_string()),
            ("osc133".into(), self.osc133.to_string()),
            ("kitty_compatible".into(), self.kitty_compatible.to_string()),
            ("multiline_tabs".into(), self.multiline_tabs.to_string()),
            ("tablist_top".into(), self.tablist_top.to_string()),
            ("tablist_hide".into(), self.tablist_hide.to_string()),
            ("tab_title_shorten_start".into(), self.tab_title_shorten_start.to_string()),
            ("default_width".into(), self.default_width.to_string()),
            ("default_height".into(), self.default_height.to_string()),
            ("default_maximize".into(), self.default_maximize.to_string()),
        ];
        for (i, color) in self.workspace_colors.iter().enumerate() {
            keys.push((format!("workspace_color_{}", i + 1), rgb(color)));
        }
        for (i, c) in self.col.iter().enumerate() {
            keys.push((format!("col_{i}"), rgb(c)));
        }
        if !self.theme.is_empty() {
            keys.push(("theme".into(), str_(&self.theme)));
        }
        keys
    }
}

/// Replace the value of each top-level `key:` line in `text`; absent keys
/// are appended. A top-level key line starts at column 0 with `key:`, so
/// replacing `col_0` cannot touch `col_1`, and indented keys (bindings,
/// palette lists) never match. Everything after the colon on the line is
/// the value -- true for every scalar in this file format.
pub fn replace_top_level_values(text: &str, keys: &[(String, String)]) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    for (key, value) in keys {
        let prefix = format!("{key}:");
        match lines.iter().position(|l| l == key || l.starts_with(&prefix)) {
            Some(i) => lines[i] = format!("{prefix} {value}"),
            None => lines.push(format!("{prefix} {value}")),
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn top_level_key(line: &str) -> Option<String> {
    let mut chars = line.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() && first != '_' {
        return None;
    }
    for c in chars {
        if c == ':' {
            return line.split(':').next().map(String::from);
        }
        if !c.is_ascii_alphanumeric() && c != '_' {
            return None;
        }
    }
    None
}

/// Keep only the last occurrence of each top-level key (Python YAML
/// semantics); drop the earlier blocks.
fn dedupe_top_level(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut blocks: Vec<(String, usize)> = Vec::new();
    for (i, l) in lines.iter().enumerate() {
        if let Some(k) = top_level_key(l) {
            blocks.push((k, i));
        }
    }
    let mut remove = vec![false; blocks.len()];
    for (bi, (key, _)) in blocks.iter().enumerate() {
        if let Some(last) = blocks.iter().rposition(|(k, _)| k == key) {
            if last != bi {
                remove[bi] = true;
            }
        }
    }
    if !remove.iter().any(|&r| r) {
        return text.to_string();
    }
    let mut out = String::new();
    for (i, l) in lines.iter().enumerate() {
        let bi = blocks.iter().rposition(|(_, s)| *s <= i);
        let keep = match bi {
            Some(b) => !remove[b],
            None => true, // header comments before the first key
        };
        if keep {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// The US-layout symbol that Shift puts on a digit's key. Used so a binding
/// written as Alt+Shift+2 also fires when the terminal reports Shift+2 as
/// "@". Layouts that shift digits differently keep working if the binding
/// spells the symbol their keyboard actually produces.
pub(crate) fn shifted_digit_of(c: &str) -> Option<&'static str> {
    Some(match c {
        "1" => "!", "2" => "@", "3" => "#", "4" => "$", "5" => "%",
        "6" => "^", "7" => "&", "8" => "*", "9" => "(", "0" => ")",
        _ => return None,
    })
}

/// Workspaces per window. Sized to the digit row, not to the letter scheme.
pub const WORKSPACES: usize = 10;
/// The opt-in letter shortcut scheme (`workspace_letter_keys`), one key per
/// workspace. Kept separate from the standard Alt+Shift+digit scheme.
pub const WORKSPACE_LETTERS: &str = "qwertyuiop";

/// The human label of a workspace's shortcut, read from the real binding so
/// the UI never advertises a key the user did not choose.
pub fn workspace_shortcut(cfg: &Config, workspace: usize) -> String {
    cfg.bindings
        .iter()
        .find(|(action, _)| *action == format!("goto_workspace_{workspace}"))
        .and_then(|(_, list)| list.first())
        .map(|b| b.label())
        .unwrap_or_else(|| format!("Alt+Shift+{}", (workspace + 1) % 10))
}

pub(crate) fn workspace_color_index(key: &str) -> Option<usize> {
    let index = key.strip_prefix("workspace_color_")?.parse::<usize>().ok()?.checked_sub(1)?;
    (index < WORKSPACES).then_some(index)
}

pub(crate) fn merge(cfg: &mut Config, v: &serde_yaml::Value) {
        let Some(map) = v.as_mapping() else {
            return;
        };
        for (k, val) in map {
            let Some(key) = k.as_str() else {
                continue;
            };
            match key {
                key if workspace_color_index(key).is_some() => {
                    set_rgb(&mut cfg.workspace_colors[workspace_color_index(key).unwrap()], val);
                }
                "bindings" => merge_bindings(&mut cfg.bindings, val),
                "tablist_background" => set_rgb(&mut cfg.tablist_background, val),
                "tab_color_bg" => set_rgb(&mut cfg.tab_color_bg, val),
                "tab_color_bg_selected" => set_rgb(&mut cfg.tab_color_bg_selected, val),
                "tab_color_fg" => set_rgb(&mut cfg.tab_color_fg, val),
                "tab_color_fg_selected" => set_rgb(&mut cfg.tab_color_fg_selected, val),
                "tab_color_entangle" => set_rgb(&mut cfg.tab_color_entangle, val),
                "tab_color_idx" => set_rgb(&mut cfg.tab_color_idx, val),
                "add_button_color" => set_rgb(&mut cfg.add_button_color, val),
                "foreground_color" => set_rgb(&mut cfg.foreground_color, val),
                "background_color" => set_rgb(&mut cfg.background_color, val),
                "selection_color" => set_rgb(&mut cfg.selection_color, val),
                "cursor_color" => set_rgb(&mut cfg.cursor_color, val),
                "accent_color" => set_rgb(&mut cfg.accent_color, val),
                "bg_tint_color" => set_rgb(&mut cfg.bg_tint_color, val),
                key if key.starts_with("col_") && key.len() == 5 && matches!(key.as_bytes()[4], b'0'..=b'7') => {
                    let i = (key.as_bytes()[4] as u8 - b'0') as usize;
                    if i < 8 {
                        set_rgb(&mut cfg.col[i], val);
                    }
                },
                "palette" => {
                    if val.as_str().is_some_and(|s| s.trim().is_empty()) {
                        cfg.palette = None;
                    } else {
                        match crate::theme::parse_palette(val) {
                            Ok(palette) => cfg.palette = Some(palette),
                            Err(e) => log::warn!("{e}"),
                        }
                    }
                },
                "theme" => {
                    if let Some(s) = val.as_str() {
                        cfg.theme = s.to_string();
                    }
                },
                "line_spacing" => {
                    if let Some(value) = val.as_f64().filter(|v| v.is_finite() && (-0.9..=3.0).contains(v)) {
                        cfg.line_spacing = value as f32;
                    } else {
                        log::warn!("line_spacing must be a finite number between -0.9 and 3.0");
                    }
                },
                "term_font" => {
                    if let Some(s) = val.as_str() {
                        cfg.term_font = s.to_string();
                    }
                },
                "tablist_font" => {
                    if let Some(s) = val.as_str() {
                        cfg.tablist_font = s.to_string();
                    }
                },
                "tab_format" => {
                    if let Some(s) = val.as_str() {
                        cfg.tab_format = s.to_string();
                    }
                },
                "tab_shorten_str" => {
                    if let Some(s) = val.as_str() {
                        cfg.tab_shorten_str = s.to_string();
                    }
                },
                "default_tab_title" => {
                    if let Some(s) = val.as_str() {
                        cfg.default_tab_title = s.to_string();
                    }
                },
                "tab_prepend" => {
                    cfg.tab_prepend = val.as_str().map(String::from);
                },
                "tab_append" => {
                    cfg.tab_append = val.as_str().map(String::from);
                },
                "scrollback_lines" => {
                    if let Some(n) = val.as_u64() {
                        cfg.scrollback_lines = n.max(16) as usize;
                    }
                },
                "history_bytes" => {
                    if let Some(n) = val.as_u64() {
                        cfg.history_bytes = (n as usize).max(64 * 1024);
                    }
                },
                "history_block_bytes" => {
                    if let Some(n) = val.as_u64() {
                        cfg.history_block_bytes = (n as usize).max(4096);
                    }
                },
                "default_width" => {
                    if let Some(n) = val.as_u64() {
                        cfg.default_width = n as u32;
                    }
                },
                "default_height" => {
                    if let Some(n) = val.as_u64() {
                        cfg.default_height = n as u32;
                    }
                },
                "tab_title_max_length" => {
                    if let Some(n) = val.as_i64().filter(|n| *n >= 0) {
                        cfg.tab_title_max_length = n as usize;
                    }
                },
                "tab_title_short_current" => {
                    if let Some(n) = val.as_i64().filter(|n| *n >= 0) {
                        cfg.tab_title_short_current = n as usize;
                    }
                },
                "tab_overview_delay_ms" => {
                    if let Some(n) = val.as_u64() {
                        cfg.tab_overview_delay_ms = n as u32;
                    }
                },
                "term_opacity" => {
                    if let Some(f) = val.as_f64() {
                        cfg.term_opacity_legacy = Some(f.clamp(0.0, 1.0) as f32);
                    }
                },
                "term_transparency" => {
                    if let Some(f) = val.as_f64() {
                        cfg.term_transparency = f.clamp(0.0, 1.0) as f32;
                        cfg.transparency_explicit = true;
                    }
                },
                "tab_overview_opacity" => {
                    if let Some(f) = val.as_f64() {
                        cfg.tab_overview_opacity = f.clamp(0.0, 1.0) as f32;
                    }
                },
                "tab_overview_scale" => {
                    if let Some(f) = val.as_f64() {
                        cfg.tab_overview_scale = f.clamp(0.05, 1.0) as f32;
                    }
                },
                "autohide_delay" => {
                    if let Some(n) = val.as_i64() {
                        cfg.autohide_delay = n as i32;
                    }
                },
                "default_maximize" => {
                    if let Some(b) = val.as_bool() {
                        cfg.default_maximize = b;
                    }
                },
                "show_tab_titles" => {
                    if let Some(b) = val.as_bool() {
                        cfg.show_tab_titles = b;
                    }
                },
                "workspaces_enabled" => {
                    if let Some(b) = val.as_bool() {
                        cfg.workspaces_enabled = b;
                    }
                },
                "workspace_letter_keys" => {
                    if let Some(b) = val.as_bool() {
                        cfg.workspace_letter_keys = b;
                    }
                },
                "tab_overview_on_alt" => {
                    if let Some(b) = val.as_bool() {
                        cfg.tab_overview_on_alt = b;
                    }
                },
                "focus_new_tabs" => {
                    if let Some(b) = val.as_bool() {
                        cfg.focus_new_tabs = b;
                    }
                },
                "new_tab_position" => {
                    if let Some(s) = val.as_str() {
                        if matches!(s, "next" | "end" | "first") {
                            cfg.new_tab_position = s.to_string();
                        } else {
                            eprintln!("bad new_tab_position value: {s:?} (expected next, end or first)");
                        }
                    }
                },
                "hide_single_tab" => {
                    if let Some(b) = val.as_bool() {
                        cfg.hide_single_tab = b;
                    }
                },
                "graveyard_limit" => {
                    if let Some(n) = val.as_u64() {
                        cfg.graveyard_limit = n.clamp(1, 32) as usize;
                    }
                },
                "osc133" => {
                    if let Some(b) = val.as_bool() {
                        cfg.osc133 = b;
                    }
                },
                "kitty_compatible" => {
                    if let Some(b) = val.as_bool() {
                        cfg.kitty_compatible = b;
                    }
                },
                "multiline_tabs" => {
                    if let Some(b) = val.as_bool() {
                        cfg.multiline_tabs = b;
                    }
                },
                "tablist_top" => {
                    if let Some(b) = val.as_bool() {
                        cfg.tablist_top = b;
                    }
                },
                "tablist_hide" => {
                    if let Some(b) = val.as_bool() {
                        cfg.tablist_hide = b;
                    }
                },
                "tab_title_shorten_start" => {
                    if let Some(b) = val.as_bool() {
                        cfg.tab_title_shorten_start = b;
                    }
                },
                _ => {}, // unknown keys ignored, like in Python
            }
        }
    }

fn set_rgb(dst: &mut Rgb, v: &serde_yaml::Value) {
    if let Some(s) = v.as_str() {
        if let Some(c) = Rgb::parse(s) {
            *dst = c;
        } else {
            eprintln!("bad color value: {s:?}");
        }
    }
}

fn merge_bindings(bindings: &mut Vec<(String, Vec<Binding>)>, val: &serde_yaml::Value) {
    let Some(map) = val.as_mapping() else {
        return;
    };
    for (k, v) in map {
        let Some(action) = k.as_str() else {
            continue;
        };
        // Standalone overview was retired; old bindings must not swallow keys.
        if action == "show_tab_overview" || action.starts_with("overview_") { continue; }
        let specs: Vec<String> = match v {
            serde_yaml::Value::String(s) => vec![s.clone()],
            serde_yaml::Value::Sequence(seq) => {
                seq.iter().filter_map(|s| s.as_str().map(String::from)).collect()
            },
            _ => continue,
        };
        let parsed: Vec<Binding> = specs.iter().filter_map(|s| Binding::parse(s)).collect();
        if parsed.is_empty() {
            eprintln!("bad binding for {action:?}: {specs:?}");
            continue;
        }
        match bindings.iter_mut().find(|(a, _)| a == action) {
            Some(entry) => entry.1 = parsed,
            None => bindings.push((action.to_string(), parsed)),
        }
    }
}

// ---------------------------------------------------------------------------
// Keybindings
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum BoundKey {
    Named(NamedKey),
    Char(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Binding {
    pub mods: ModifiersState,
    pub key: BoundKey,
}

impl Binding {
    /// Human-readable form of the effective binding, shared by menus and help.
    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.mods.control_key() { parts.push("Ctrl".to_string()); }
        if self.mods.alt_key() { parts.push("Alt".to_string()); }
        if self.mods.shift_key() { parts.push("Shift".to_string()); }
        parts.push(match &self.key {
            BoundKey::Char(c) => match c.as_str() {
                "\r" => "Enter".into(),
                _ => c.to_uppercase(),
            },
            BoundKey::Named(key) => match key {
                NamedKey::ArrowLeft => "Left".into(),
                NamedKey::ArrowRight => "Right".into(),
                NamedKey::ArrowUp => "Up".into(),
                NamedKey::ArrowDown => "Down".into(),
                _ => format!("{key:?}"),
            },
        });
        parts.join("+")
    }

    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split('+').collect();
        let (mod_strs, key_str) = parts.split_at(parts.len().saturating_sub(1));
        let key_str = key_str[0].trim();
        if key_str.is_empty() {
            return None;
        }

        let mut mods = ModifiersState::empty();
        for m in mod_strs.iter().map(|s| s.trim()) {
            match m {
                "MOD1" | "ALT" | "Alt_L" | "Alt_R" => mods |= ModifiersState::ALT,
                "CONTROL" | "CTRL" => mods |= ModifiersState::CONTROL,
                "SHIFT" => mods |= ModifiersState::SHIFT,
                _ => return None,
            }
        }

        // "_1" style: underscore prefix means the literal character key
        if let Some(rest) = key_str.strip_prefix('_') {
            if rest.len() == 1 {
                return Some(Self { mods, key: BoundKey::Char(rest.to_string()) });
            }
        }

        let key = match key_str {
            "Left" => BoundKey::Named(NamedKey::ArrowLeft),
            "Right" => BoundKey::Named(NamedKey::ArrowRight),
            "Up" => BoundKey::Named(NamedKey::ArrowUp),
            "Down" => BoundKey::Named(NamedKey::ArrowDown),
            "Home" => BoundKey::Named(NamedKey::Home),
            "End" => BoundKey::Named(NamedKey::End),
            "Return" => BoundKey::Named(NamedKey::Enter),
            "Escape" => BoundKey::Named(NamedKey::Escape),
            // Numpad keys arrive as characters (NumLock on).
            "KP_Add" => BoundKey::Char("+".to_string()),
            "KP_Subtract" => BoundKey::Char("-".to_string()),
            "KP_Multiply" => BoundKey::Char("*".to_string()),
            "KP_Divide" => BoundKey::Char("/".to_string()),
            "KP_Enter" => BoundKey::Char("\r".to_string()),
            "Space" => BoundKey::Named(NamedKey::Space),
            "Page_Up" => BoundKey::Named(NamedKey::PageUp),
            "Page_Down" => BoundKey::Named(NamedKey::PageDown),
            "Insert" => BoundKey::Named(NamedKey::Insert),
            "Delete" => BoundKey::Named(NamedKey::Delete),
            "Tab" => BoundKey::Named(NamedKey::Tab),
            "Backspace" => BoundKey::Named(NamedKey::Backspace),
            "F1" => BoundKey::Named(NamedKey::F1),
            "F2" => BoundKey::Named(NamedKey::F2),
            "F3" => BoundKey::Named(NamedKey::F3),
            "F4" => BoundKey::Named(NamedKey::F4),
            "F5" => BoundKey::Named(NamedKey::F5),
            "F6" => BoundKey::Named(NamedKey::F6),
            "F7" => BoundKey::Named(NamedKey::F7),
            "F8" => BoundKey::Named(NamedKey::F8),
            "F9" => BoundKey::Named(NamedKey::F9),
            "F10" => BoundKey::Named(NamedKey::F10),
            "F11" => BoundKey::Named(NamedKey::F11),
            "F12" => BoundKey::Named(NamedKey::F12),
            "braceleft" => BoundKey::Char("{".to_string()),
            "braceright" => BoundKey::Char("}".to_string()),
            _ => {
                if key_str.len() == 1 {
                    BoundKey::Char(key_str.to_string())
                } else {
                    return None;
                }
            },
        };
        Some(Self { mods, key })
    }

    /// Match a winit key + modifier state against this binding.
    pub fn matches(&self, key: &Key, mods: ModifiersState) -> bool {
        // Compare the three relevant modifier bits exactly.
        if mods.control_key() != self.mods.control_key()
            || mods.shift_key() != self.mods.shift_key()
            || mods.alt_key() != self.mods.alt_key()
        {
            return false;
        }
        match &self.key {
            BoundKey::Named(n) => matches!(key, Key::Named(k) if k == n),
            // winit reports Shift+C as "C", while legacy GTK binding files
            // spell CONTROL+SHIFT+c. Caps Lock must not break shortcuts either.
            BoundKey::Char(c) => matches!(key, Key::Character(k) if k == c ||
                (self.mods.intersects(ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT)
                    && k.eq_ignore_ascii_case(c))
                // Shift turns a digit into a symbol before the key is ever
                // reported: Alt+Shift+2 arrives as "@", never "2", so a
                // digit written with Shift must also answer to its symbol.
                // Same US-shifted-symbol convention as `braceleft` -> "{`.
                || (self.mods.shift_key() && shifted_digit_of(c).is_some_and(|s| k == s))),
        }
    }

    /// Match against raw character input (for keys that arrive as text).
    pub fn matches_char(&self, c: &str, _mods: ModifiersState) -> bool {
        if self.mods != ModifiersState::empty() {
            return false;
        }
        matches!(&self.key, BoundKey::Char(k) if k == c)
    }
}

/// Find the first action whose any binding matches.
/// Overview bindings only apply while the overview is open; otherwise
/// they would swallow plain Return/Escape/arrows meant for the terminal.
pub fn find_action(bindings: &[(String, Vec<Binding>)], key: &Key, mods: ModifiersState, overview_active: bool) -> Option<String> {
    for (action, list) in bindings {
        if action.starts_with("overview_") && !overview_active {
            continue;
        }
        if list.iter().any(|b| b.matches(key, mods)) {
            return Some(action.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    #[test]
    fn line_spacing_scales_the_normal_height_and_round_trips() {
        let base = std::path::Path::new(".");
        assert_eq!(Config::default().line_spacing, 0.0);
        for (spacing, height) in [(0.0, 20.0), (0.1, 22.0), (-0.1, 18.0), (0.5, 30.0), (-0.9, 2.0)] {
            let cfg = Config::from_text(&format!("wal_palette: ''\nline_spacing: {spacing}\n"), base).unwrap();
            assert_eq!(cfg.terminal_line_height(20.0), height);
            assert!(cfg.terminal_line_height(1.0) >= 1.0);
            let saved = replace_top_level_values("", &cfg.ui_keys());
            assert_eq!(Config::from_text(&saved, base).unwrap().line_spacing, spacing);
        }
        for invalid in [".nan", ".inf", "-.inf", "-1", "4", "'wide'", "null"] {
            let cfg = Config::from_text(&format!("wal_palette: ''\nline_spacing: {invalid}\n"), base).unwrap();
            assert_eq!(cfg.line_spacing, 0.0, "invalid spacing {invalid}");
        }
    }

    #[test]
    fn workspaces_default_off_and_round_trip() {
        assert!(!Config::default().workspaces_enabled);
        let cfg = Config::from_text("wal_palette: ''\nworkspaces_enabled: true\n", std::path::Path::new(".")).unwrap();
        assert!(cfg.workspaces_enabled);
        let saved = replace_top_level_values("", &cfg.ui_keys());
        assert!(Config::from_text(&saved, std::path::Path::new(".")).unwrap().workspaces_enabled);
    }

    #[test]
    fn tab_presentation_preferences_default_on_and_round_trip() {
        let defaults = Config::default();
        assert!(defaults.tab_overview_on_alt);
        assert!(defaults.show_tab_titles);
        let cfg = Config::from_text("wal_palette: ''\ntab_overview_on_alt: false\nshow_tab_titles: false\n", std::path::Path::new(".")).unwrap();
        assert!(!cfg.tab_overview_on_alt);
        assert!(!cfg.show_tab_titles);
        let saved = replace_top_level_values("", &cfg.ui_keys());
        let restored = Config::from_text(&saved, std::path::Path::new(".")).unwrap();
        assert!(!restored.tab_overview_on_alt);
        assert!(!restored.show_tab_titles);
    }

    #[test]
    fn workspace_colors_load_save_and_override_themes_independently() {
        let base = std::path::Path::new(".");
        let mut cfg = Config::from_text("theme: dracula\nworkspace_color_1: '#123456'\nworkspace_color_9: '#fedcba'\nworkspace_color_10: '#abc'\n", base).unwrap();
        assert_eq!(cfg.workspace_colors[0], Rgb(0x12, 0x34, 0x56));
        assert_eq!(cfg.workspace_colors[8], Rgb(0xfe, 0xdc, 0xba));
        assert_eq!(cfg.workspace_colors[9], Rgb(0xaa, 0xbb, 0xcc));
        let defaults = Config::default();
        assert_eq!(cfg.workspace_colors[1..8], defaults.workspace_colors[1..8]);
        cfg.workspace_colors[4] = Rgb(5, 127, 250);
        let saved = replace_top_level_values("# keep comment\nbindings: {new_tab: 'Control+t'}\n", &cfg.ui_keys());
        let restored = Config::from_text(&saved, base).unwrap();
        assert_eq!(restored.workspace_colors, cfg.workspace_colors);
        assert!(saved.contains("# keep comment"));
        assert!(!saved.contains("tab_color_bg:"));
        assert!(saved.contains("workspace_color_10: '#aabbcc'"));
        // Global theme changes preserve workspace identities.
        crate::theme::apply_name(&mut cfg, "catppuccin-latte").unwrap();
        assert_eq!(cfg.workspace_colors, restored.workspace_colors);
    }

    #[test]
    fn invalid_workspace_color_keys_and_values_keep_defaults() {
        let cfg = Config::from_text("wal_palette: ''\nworkspace_color_0: '#fff'\nworkspace_color_11: '#fff'\nworkspace_color_bad: '#fff'\nworkspace_color_1: nope\nworkspace_color_2: 123\n", std::path::Path::new(".")).unwrap();
        assert_eq!(cfg.workspace_colors, Config::default().workspace_colors);
        let colors = Config::default().workspace_colors;
        for (i, color) in colors[..9].iter().enumerate() {
            assert!(!colors[..i].contains(color), "W{} duplicates a default workspace color", i + 1);
        }
    }

    #[test]
    #[test]
    fn workspace_shortcuts_use_alt_shift_digits_and_free_alt_letters() {
        let cfg = Config::default();
        // The standard scheme: Alt+Shift+digit, in physical digit order.
        for (i, digit) in "1234567890".chars().enumerate() {
            let key = Key::Character(digit.to_string().into());
            assert_eq!(
                find_action(&cfg.bindings, &key, ModifiersState::ALT | ModifiersState::SHIFT, false),
                Some(format!("goto_workspace_{i}")),
                "Alt+Shift+{digit}");
        }
        // Bare Alt+letter belongs to the shell (readline, word motions) and
        // must stay unclaimed unless the user opts into the letter scheme.
        for letter in WORKSPACE_LETTERS.chars() {
            let key = Key::Character(letter.to_string().into());
            assert_eq!(find_action(&cfg.bindings, &key, ModifiersState::ALT, false), None,
                "Alt+{letter} must reach the shell by default");
        }
    }

    #[test]
    fn shift_digit_bindings_also_match_the_shifted_symbol() {
        // The key report carries Shift's symbol ("@"), never the digit, so a
        // binding written as MOD1+SHIFT+_2 has to answer to both spellings.
        let cfg = Config::default();
        // "*" is left out: it is also Shift+KP_Multiply (toggle_entangle), and
        // the key report cannot tell the two keys apart.
        for (symbol, i) in [("!", 0), ("@", 1), ("#", 2), ("$", 3), ("%", 4), ("^", 5), ("&", 6), ("(", 8), (")", 9)] {
            let key = Key::Character(symbol.into());
            assert_eq!(
                find_action(&cfg.bindings, &key, ModifiersState::ALT | ModifiersState::SHIFT, false),
                Some(format!("goto_workspace_{i}")),
                "Alt+Shift should match {symbol}");
        }
        // A digit bound without Shift keeps the literal digit only.
        assert_eq!(find_action(&cfg.bindings, &Key::Character("2".into()), ModifiersState::ALT, false),
            Some("goto_tab_1".into()));
        assert_eq!(find_action(&cfg.bindings, &Key::Character("@".into()), ModifiersState::ALT, false), None,
            "the symbol of Shift+2 is not Alt+2");
    }

    #[test]
    fn workspace_letter_keys_adds_alt_letters_without_losing_the_digits() {
        let cfg = Config::from_text("wal_palette: ''\nworkspace_letter_keys: true\n", std::path::Path::new(".")).unwrap();
        for (i, letter) in WORKSPACE_LETTERS.chars().enumerate() {
            let key = Key::Character(letter.to_string().into());
            assert_eq!(find_action(&cfg.bindings, &key, ModifiersState::ALT, false),
                Some(format!("goto_workspace_{i}")), "Alt+{letter}");
            assert_eq!(find_action(&cfg.bindings, &key, ModifiersState::empty(), false), None);
        }
        // Both schemes coexist.
        assert_eq!(find_action(&cfg.bindings, &Key::Character("@".into()),
            ModifiersState::ALT | ModifiersState::SHIFT, false), Some("goto_workspace_1".into()));
        // Saved and reloaded, the scheme does not accumulate duplicates.
        let saved = replace_top_level_values("", &cfg.ui_keys());
        let again = Config::from_text(&saved, std::path::Path::new(".")).unwrap();
        for i in 0..WORKSPACES {
            let list = again.bindings.iter().find(|(a, _)| *a == format!("goto_workspace_{i}")).unwrap();
            assert_eq!(list.1.len(), 2, "workspace {i} bindings");
        }
    }

    #[test]
    fn binding_labels_use_readable_modifiers_and_keys() {
        for (spec, expected) in [
            ("CONTROL+SHIFT+n", "Ctrl+Shift+N"),
            ("MOD1+SHIFT+c", "Alt+Shift+C"),
            ("MOD1+_1", "Alt+1"),
            ("CONTROL+ALT+Left", "Ctrl+Alt+Left"),
            ("SHIFT+KP_Enter", "Shift+Enter"),
        ] {
            assert_eq!(super::Binding::parse(spec).unwrap().label(), expected);
        }
    }

    #[test]
    fn user_config_chords_probe() {
        let cfg = Config::load();
        use winit::keyboard::NamedKey;
        for (label, key, mods) in [
            ("ctrl+w", Key::Character("w".into()), M::CONTROL),
            ("ctrl+left", Key::Named(NamedKey::ArrowLeft), M::CONTROL),
            ("ctrl+right", Key::Named(NamedKey::ArrowRight), M::CONTROL),
            ("ctrl+backspace", Key::Named(NamedKey::Backspace), M::CONTROL),
        ] {
            let hit = find_action(&cfg.bindings, &key, mods, false);
            println!("{label} -> {hit:?}");
        }
        for (action, list) in &cfg.bindings {
            for b in list {
                println!("BIND {action} = {:?} {:?}", b.key, b.mods);
            }
        }
    }
    use super::*;
    use winit::keyboard::ModifiersState as M;

    #[test]
    fn window_actions_are_bound_by_default() {
        let cfg = Config::default();
        assert_eq!(
            find_action(&cfg.bindings, &Key::Character("N".into()), M::CONTROL | M::SHIFT, false)
                .as_deref(),
            Some("new_window"),
            "Ctrl+Shift+n (what winit reports for the shifted key) opens a window"
        );
        assert_eq!(
            find_action(&cfg.bindings, &Key::Character("W".into()), M::ALT | M::SHIFT, false)
                .as_deref(),
            Some("send_tab_to_window"),
            "Alt+Shift+w sends the tab to another window"
        );
    }

    #[test]
    fn clipboard_shortcuts_accept_the_shifted_keys_winit_reports() {
        let cfg = Config::default();
        assert_eq!(find_action(&cfg.bindings, &Key::Character("C".into()), M::CONTROL | M::SHIFT, false).as_deref(), Some("copy"));
        assert_eq!(find_action(&cfg.bindings, &Key::Character("V".into()), M::CONTROL | M::SHIFT, false).as_deref(), Some("paste"));
        assert_eq!(find_action(&cfg.bindings, &Key::Character("L".into()), M::CONTROL | M::SHIFT, false).as_deref(), Some("copy_last_output"));
        assert_eq!(find_action(&cfg.bindings, &Key::Named(NamedKey::Insert), M::SHIFT, false).as_deref(), Some("paste_primary"));
    }

    #[test]
    fn retired_standalone_overview_bindings_do_not_swallow_keys() {
        let mut cfg = Config::default();
        let legacy: serde_yaml::Value = serde_yaml::from_str("show_tab_overview: MOD1+o\noverview_exit: Escape\noverview_nav_left: Left\n").unwrap();
        merge_bindings(&mut cfg.bindings, &legacy);
        // Alt+o is no longer claimed: bare Alt+letters are left to the shell
        // unless `workspace_letter_keys` opts in.
        assert_eq!(find_action(&cfg.bindings, &Key::Character("o".into()), M::ALT, false), None);
        assert_eq!(find_action(&cfg.bindings, &Key::Named(NamedKey::Escape), M::empty(), false), None);
        assert!(cfg.tab_overview_on_alt, "the Alt-held visual switcher remains enabled");
    }

    #[test]
    fn rgb_parse_forms() {
        assert_eq!(Rgb::parse("#ff2c4b"), Some(Rgb(255, 44, 75)));
        assert_eq!(Rgb::parse("#141413"), Some(Rgb(20, 20, 19)));
        // #rgb shorthand expands each nibble to x17.
        assert_eq!(Rgb::parse("#f00"), Some(Rgb(255, 0, 0)));
        assert_eq!(Rgb::parse("#fff"), Some(Rgb(255, 255, 255)));
        assert_eq!(Rgb::parse("#ffff2c2c4b4b"), Some(Rgb(255, 44, 75)));
        assert_eq!(Rgb::parse("#ffff00000000"), Some(Rgb(255, 0, 0)));
        assert_eq!(Rgb::parse("#éééééé"), None);
        // invalid: wrong length / non-hex.
        assert_eq!(Rgb::parse("#12345"), None);
        assert_eq!(Rgb::parse("#zzzzzz"), None);
    }

    #[test]
    fn font_parse_family_and_size() {
        assert_eq!(parse_font("Cascadia Code 9"), ("Cascadia Code".to_string(), 9.0));
        // family may contain parentheses and spaces.
        assert_eq!(
            parse_font("Terminus (TTF) 12"),
            ("Terminus (TTF)".to_string(), 12.0)
        );
        // no size -> default 12.
        assert_eq!(parse_font("Monospace"), ("Monospace".to_string(), 12.0));
    }

    #[test]
    fn palette_badwolf_first_sixteen() {
        let full = "#141413:#ff0208:#aeee00:#fade3e:#0a9dff:#ff9eb8:#8cffba:#d9cec3:#998f84:#ff0208:#aeee00:#ffa724:#0a9dff:#ff9eb8:#8cffba:#ffffff";
        let p = parse_palette(&full).expect("16 colors");
        assert_eq!(p[0], Rgb(20, 20, 19));
        assert_eq!(p[1], Rgb(255, 2, 8));
        assert_eq!(p[2], Rgb(174, 238, 0));
        assert_eq!(p[15], Rgb(255, 255, 255));
        assert_eq!(parse_palette(""), None);
        // Too few is no longer an error -- short lists are completed, not
        // rejected: slots take the nearest given color in Lab space.
        let two = parse_palette("#000:#f00").expect("derived from two");
        assert_eq!(two[0], Rgb(0, 0, 0));
        assert_eq!(two[9], Rgb(255, 0, 0));
    }

    #[test]
    fn config_path_prefers_xdg_then_legacy() {
        let root = std::env::temp_dir().join(format!("mypixels-path-{}", std::process::id()));
        let xdg = root.join("config");
        let home = root.join("home");
        std::fs::create_dir_all(xdg.join("mypixels")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let modern = xdg.join("mypixels").join("config.yml");
        // Neither file exists yet: the modern path wins -- that is where a
        // first save belongs, legacy or not.
        assert_eq!(Config::config_path_in(None, Some(&xdg), Some(&home)).unwrap(), modern);
        // Only the legacy file exists: it is respected, not silently skipped.
        std::fs::write(home.join(".mypixels"), "").unwrap();
        assert_eq!(Config::config_path_in(None, Some(&xdg), Some(&home)).unwrap(),
            home.join(".mypixels"));
        // Once the XDG file exists it wins: the move happens on save, not on
        // read, and never duplicates state.
        std::fs::write(&modern, "").unwrap();
        assert_eq!(Config::config_path_in(None, Some(&xdg), Some(&home)).unwrap(), modern);
        // The test override still outranks everything.
        assert_eq!(
            Config::config_path_in(Some(root.join("env")), Some(&xdg), Some(&home)).unwrap(),
            root.join("env")
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn theme_name_round_trips_through_save() {
        let cfg = Config::from_text("theme: 'gruvbox-dark'\n", std::path::Path::new(".")).unwrap();
        assert_eq!(cfg.theme, "gruvbox-dark");
        let keys = cfg.ui_keys();
        assert!(keys.iter().any(|(k, v)| k == "theme" && v == "'gruvbox-dark'"));
        // Colors the theme set are saved alongside it: the file stays
        // readable by tools that do not know themes at all.
        assert!(keys.iter().any(|(k, _)| k == "background_color"));
        // No theme: no empty `theme: ''` noise appended to the file.
        let cfg = Config::default();
        assert!(!cfg.ui_keys().iter().any(|(k, _)| k == "theme"));
    }

    #[test]
    fn dedupe_keeps_last_top_level_key() {
        let text = "a: 1\nb: 2\na: 3\n  - x\nc: 4\n";
        let out = dedupe_top_level(text);
        // first `a: 1` block dropped; second `a: 3` (with its list item) kept.
        assert!(!out.contains("a: 1"));
        assert!(out.contains("a: 3"));
        assert!(out.contains("- x"));
        assert!(out.contains("b: 2"));
        assert!(out.contains("c: 4"));
    }

    #[test]
    fn dedupe_preserves_header_and_no_duplicates() {
        let text = "# comment\na: 1\nb: 2\n";
        assert_eq!(dedupe_top_level(text), text);
    }

    #[test]
    fn binding_parse_mods_and_key() {
        let b = Binding::parse("MOD1+SHIFT+Left").unwrap();
        assert_eq!(b.mods, M::ALT | M::SHIFT);
        assert!(matches!(b.key, BoundKey::Named(NamedKey::ArrowLeft)));

        let b = Binding::parse("CONTROL+ALT+x").unwrap();
        assert_eq!(b.mods, M::CONTROL | M::ALT);
        assert!(matches!(b.key, BoundKey::Char(ref c) if c == "x"));

        // underscore prefix -> literal char even for a digit.
        let b = Binding::parse("MOD1+_1").unwrap();
        assert_eq!(b.mods, M::ALT);
        assert!(matches!(b.key, BoundKey::Char(ref c) if c == "1"));

        // numpad add arrives as the "+" character.
        let b = Binding::parse("SHIFT+KP_Add").unwrap();
        assert_eq!(b.mods, M::SHIFT);
        assert!(matches!(b.key, BoundKey::Char(ref c) if c == "+"));

        assert!(Binding::parse("").is_none());
        assert!(Binding::parse("FOO+x").is_none()); // unknown modifier
    }

    #[test]
    fn replace_keeps_comments_and_replaces_in_place() {
        let text = "# upraveno\nautohide_delay: -1\nterm_opacity: 0.1\n# a comment\ncol_0: '#111111'\n";
        let out = replace_top_level_values(text, &[
            ("term_opacity".into(), "0.7".into()),
            ("col_0".into(), "'#ff0000'".into()),
        ]);
        assert_eq!(
            out,
            "# upraveno\nautohide_delay: -1\nterm_opacity: 0.7\n# a comment\ncol_0: '#ff0000'\n"
        );
    }

    #[test]
    fn replace_appends_missing_keys_at_end() {
        let text = "term_opacity: 0.1\n";
        let out = replace_top_level_values(text, &[("tablist_top".into(), "false".into())]);
        assert_eq!(out, "term_opacity: 0.1\ntablist_top: false\n");
    }

    #[test]
    fn replace_does_not_touch_indented_or_prefixed_keys() {
        let text = "  col_0: nested\ncol_0: '#111111'\ncol_1: '#222222'\n";
        let out = replace_top_level_values(text, &[("col_0".into(), "'#333333'".into())]);
        assert_eq!(out, "  col_0: nested\ncol_0: '#333333'\ncol_1: '#222222'\n");
    }

    #[test]
    fn save_roundtrip_preserves_bindings_and_comments() {
        let cfg = Config::default();
        let text = "# c\nbindings:\n  new_tab: CONTROL+SHIFT+t\ncol_0: '#000000'\nterm_opacity: 0.1\n";
        let out = replace_top_level_values(text, &cfg.ui_keys());
        // Bindings block and its indented entries survive byte for byte.
        assert!(out.contains("bindings:\n  new_tab: CONTROL+SHIFT+t\n"));
        assert!(out.starts_with("# c\n"));
        // Every managed key is present exactly once.
        assert_eq!(out.lines().filter(|l| l.starts_with("term_opacity:")).count(), 1);
        assert!(out.contains("term_opacity: 0.1")); // default is 0.1, unchanged
    }
}

/// The terminal background the ORIGINAL actually paints: with no 16-color
/// palette set, VTE's background is `bg_tint_color` (the pywal loader keeps
/// `bg_tint_color` equal to the palette background when one is active, and
/// `background_color` is not consulted on that path). Reading only
/// `background_color` made `bg_tint_color` a dead key in the rewrite.
pub fn terminal_bg(c: &Config) -> Rgb {
    if c.palette.is_some() { c.background_color } else { c.bg_tint_color }
}
#[cfg(test)]
mod theme_palette_tests {
    use super::*;
    #[test]
    fn legacy_term_opacity_feeds_term_transparency() {
        // Files predating the rename keep working...
        let cfg = Config::from_text("term_opacity: 0.4\n", std::path::Path::new(".")).unwrap();
        assert!((cfg.term_transparency - 0.4).abs() < 1e-6);
        // ...but the new key always wins, whatever the file order.
        let cfg = Config::from_text(
            "term_opacity: 0.4\nterm_transparency: 0.8\n",
            std::path::Path::new("."),
        )
        .unwrap();
        assert!((cfg.term_transparency - 0.8).abs() < 1e-6);
        let cfg = Config::from_text(
            "term_transparency: 0.8\nterm_opacity: 0.4\n",
            std::path::Path::new("."),
        )
        .unwrap();
        assert!((cfg.term_transparency - 0.8).abs() < 1e-6);
        // Saves write only the new spelling.
        let keys = Config::default().ui_keys();
        assert!(keys.iter().any(|(k, _)| k == "term_transparency"));
        assert!(!keys.iter().any(|(k, _)| k == "term_opacity"));
    }

    #[test]
    fn theme_supersedes_stale_palette_copy_lines() {
        // The user's exact trap: theme line added below a years-old custom
        // palette + col_* copies from an older theme save. The theme must
        // own the 16 slots; fg/bg stay file-overridable.
        let text = "col_0: '#141413'\ncol_4: '#0a9dff'\n\
            palette: \"#141413:#ff2c4b:#aeee00:#fade3e:#0a9dff:#ff9eb8:#8cffba:#d9cec3:\
            #000000:#ff2c4b:#aeee00:#ffa724:#0a9dff:#ff9eb8:#8cffba:#ffffff\"\n\
            foreground_color: '#f8f8f2'\nbackground_color: '#282a36'\ntheme: 'dracula'\n";
        let cfg = Config::from_text(text, std::path::Path::new(".")).unwrap();
        let p = cfg.palette.expect("theme supplies the palette");
        assert_eq!(p[0], Rgb(0x21, 0x22, 0x2c), "dracula black, not stale #141413");
        assert_eq!(p[4], Rgb(0xbd, 0x93, 0xf9), "dracula blue, not stale #0a9dff");
        assert_eq!(cfg.foreground_color, Rgb(0xf8, 0xf8, 0xf2), "explicit fg still wins");
        // No theme: the palette lines are the config, untouched.
        // No theme, and wal explicitly disabled: the palette line is the
        // config, untouched. (Without `wal_palette: ''` a live
        // ~/.cache/wal/colors.yml would win by design - implicit wal is
        // the default when no theme is named.)
        let cfg = Config::from_text(
            "wal_palette: ''\npalette: \"#141413:#ff2c4b:#aeee00:#fade3e:#0a9dff:#ff9eb8:#8cffba:#d9cec3:#000000:#ff2c4b:#aeee00:#ffa724:#0a9dff:#ff9eb8:#8cffba:#ffffff\"\n",
            std::path::Path::new("."),
        )
        .unwrap();
        assert_eq!(cfg.palette.unwrap()[2], Rgb(0xae, 0xee, 0x00));
        assert_eq!(cfg.palette.unwrap()[2], Rgb(0xae, 0xee, 0x00));
    }
}
