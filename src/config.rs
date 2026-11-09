//! Configuration: ~/.config/dtem/dtem.toml (hot-reloaded)
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// A single value or a list (e.g. `hotkey = "F1"` or `hotkey = ["F1", "ctrl+alt+t"]`).
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

impl OneOrMany {
    pub fn list(&self) -> Vec<String> {
        match self {
            OneOrMany::One(s) if s.trim().is_empty() => vec![],
            OneOrMany::One(s) => vec![s.clone()],
            OneOrMany::Many(v) => v.iter().filter(|s| !s.trim().is_empty()).cloned().collect(),
        }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    // ---- font & look
    /// fontconfig family or a font file path
    pub font_family: String,
    pub font_size: f32,
    pub line_height: f32,
    pub padding: f32,
    pub ligatures: bool,
    /// Preset name, see `schemes.rs`
    pub color_scheme: String,
    /// Transparent background on/off (toggle with `toggle_transparency`)
    pub transparency: bool,
    /// Opacity used while transparency is on
    pub background_opacity: f32,
    /// "block" | "beam" | "underline"
    pub cursor_shape: String,
    pub cursor_blink: bool,
    pub bold_is_bright: bool,
    /// "none" | "right" | "left"
    pub scrollbar: String,

    // ---- tabs & titles
    /// Show the tab bar from this many tabs on (1 = always)
    pub tab_bar_min_tabs: usize,
    /// "top" | "bottom" | "hidden"
    pub tab_bar_position: String,
    /// Tabs share the full width
    pub tab_bar_expand: bool,
    pub new_tab_after_current: bool,
    /// Base title (used by `dynamic_title`)
    pub title: String,
    /// How program-set titles are shown: "replace" | "before" | "after" | "none"
    pub dynamic_title: String,
    /// 0 = unlimited
    pub title_max_length: usize,
    /// When longer than max: "end" (keep first chars) | "start" (keep last chars) | "none"
    pub title_truncate: String,

    // ---- scrolling & selection
    pub scrollback_lines: usize,
    pub scrollback_infinite: bool,
    pub scroll_on_output: bool,
    pub scroll_on_key: bool,
    /// Characters that belong to a word on double-click (tilda syntax, e.g. "-A-Za-z0-9,./?%&#:_")
    pub word_chars: String,
    pub copy_on_select: bool,

    // ---- links
    pub open_url_with: String,
    /// Ctrl must be held to open links (otherwise a plain click opens them)
    pub url_requires_ctrl: bool,
    pub match_web_uris: bool,
    pub match_file_uris: bool,
    pub match_email: bool,

    // ---- keyboard
    /// "auto" | "del" | "bs" | "escape"
    pub backspace_sends: String,
    /// "escape" | "del" | "bs" | "auto"
    pub delete_sends: String,
    pub ime: bool,
    pub kitty_keyboard: bool,
    pub kitty_mod: String,
    /// "combo" = "action"; action "none" removes a default binding
    pub keybindings: HashMap<String, String>,

    // ---- shell
    pub shell: Option<String>,
    /// Command run in new tabs (empty = just the shell)
    pub command: String,
    /// When `command` exits: "shell" | "restart" | "close"
    pub command_exit: String,
    pub login_shell: bool,
    /// Start directory when not inheriting (empty = home)
    pub working_dir: String,
    pub inherit_working_dir: bool,

    // ---- bell
    pub bell: bool,
    pub visual_bell: bool,

    // ---- session
    pub persist_scrollback: bool,
    pub persist_scrollback_lines: usize,
    pub autosave_interval_secs: u64,
    pub confirm_close: ConfirmClose,

    // ---- misc
    pub force_x11: bool,
    /// Programs reading the clipboard via OSC 52: "ask" | "allow" | "deny"
    pub clipboard_read: String,
    pub remote_install_helpers: bool,

    pub colors: Colors,
    pub dropdown: Dropdown,
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct ConfirmClose {
    pub tab: bool,
    pub pane: bool,
    /// Closing a window / quitting
    pub window: bool,
}

impl Default for ConfirmClose {
    fn default() -> Self {
        Self { tab: true, pane: true, window: true }
    }
}

#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(default)]
pub struct Dropdown {
    /// One key or a list: "F1" or ["F1", "ctrl+alt+t"]
    pub hotkey: OneOrMany,
    /// Fraction of the monitor
    pub width: f32,
    pub height: f32,
    pub centered_horizontally: bool,
    pub centered_vertically: bool,
    /// Offset inside the monitor (px) when not centered
    pub x: i32,
    pub y: i32,
    /// Monitor to use the first time (e.g. "DP-1"); afterwards the last used one is remembered
    pub monitor: String,
    pub animation: bool,
    pub animation_duration_ms: u64,
    /// "top" | "bottom" | "left" | "right"
    pub animation_from: String,
    pub hide_on_focus_loss: bool,
    pub hide_on_mouse_leave: bool,
    pub auto_hide_delay_ms: u64,
    /// Hotkey while visible but not focused: "focus" | "hide"
    pub unfocused_toggle: String,
    pub grab_focus: bool,
    pub always_on_top: bool,
    pub skip_taskbar: bool,
    /// Visible on all workspaces
    pub all_workspaces: bool,
    pub start_hidden: bool,
    pub start_fullscreen: bool,
    /// When the last tab exits: "new_tab_hide" | "new_tab" | "quit"
    pub on_last_tab_exit: String,
}

impl Default for Dropdown {
    fn default() -> Self {
        Self {
            hotkey: OneOrMany::One("F12".into()),
            width: 1.0,
            height: 0.45,
            centered_horizontally: true,
            centered_vertically: false,
            x: 0,
            y: 0,
            monitor: String::new(),
            animation: false,
            animation_duration_ms: 180,
            animation_from: "top".into(),
            hide_on_focus_loss: false,
            hide_on_mouse_leave: false,
            auto_hide_delay_ms: 0,
            unfocused_toggle: "focus".into(),
            grab_focus: true,
            always_on_top: true,
            skip_taskbar: true,
            all_workspaces: true,
            start_hidden: false,
            start_fullscreen: false,
            on_last_tab_exit: "new_tab_hide".into(),
        }
    }
}

/// Explicit colors override the scheme.
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
#[serde(default)]
pub struct Colors {
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub cursor: Option<String>,
    pub cursor_text: Option<String>,
    pub selection_foreground: Option<String>,
    pub selection_background: Option<String>,
    pub active_border: Option<String>,
    pub inactive_border: Option<String>,
    pub tab_bar_background: Option<String>,
    pub active_tab_foreground: Option<String>,
    pub active_tab_background: Option<String>,
    pub inactive_tab_foreground: Option<String>,
    pub inactive_tab_background: Option<String>,
    pub search_match: Option<String>,
    /// 16 ANSI colors
    pub palette: Option<Vec<String>>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: "monospace".into(),
            font_size: 11.0,
            line_height: 1.0,
            padding: 4.0,
            ligatures: true,
            color_scheme: "tokyo-night".into(),
            transparency: false,
            background_opacity: 0.85,
            cursor_shape: "block".into(),
            cursor_blink: false,
            bold_is_bright: false,
            scrollbar: "none".into(),
            tab_bar_min_tabs: 2,
            tab_bar_position: "top".into(),
            tab_bar_expand: false,
            new_tab_after_current: true,
            title: "dtem".into(),
            dynamic_title: "replace".into(),
            title_max_length: 0,
            title_truncate: "end".into(),
            scrollback_lines: 10_000,
            scrollback_infinite: false,
            scroll_on_output: false,
            scroll_on_key: true,
            word_chars: String::new(),
            copy_on_select: false,
            open_url_with: "xdg-open".into(),
            url_requires_ctrl: true,
            match_web_uris: true,
            match_file_uris: true,
            match_email: true,
            backspace_sends: "auto".into(),
            delete_sends: "escape".into(),
            ime: true,
            kitty_keyboard: true,
            kitty_mod: "ctrl+shift".into(),
            keybindings: HashMap::new(),
            shell: None,
            command: String::new(),
            command_exit: "shell".into(),
            login_shell: false,
            working_dir: String::new(),
            inherit_working_dir: true,
            bell: true,
            visual_bell: false,
            persist_scrollback: true,
            persist_scrollback_lines: 10_000,
            autosave_interval_secs: 5,
            confirm_close: ConfirmClose::default(),
            force_x11: false,
            clipboard_read: "ask".into(),
            remote_install_helpers: true,
            colors: Colors::default(),
            dropdown: Dropdown::default(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("dtem")
}

pub fn config_path() -> PathBuf {
    config_dir().join("dtem.toml")
}

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dtem")
}

impl Config {
    pub fn load() -> Config {
        Self::try_load().unwrap_or_else(|e| {
            log::error!("{e}");
            Config::default()
        })
    }

    pub fn try_load() -> Result<Config, String> {
        let path = config_path();
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str::<Config>(&s).map_err(|e| format!("config error in {}: {e}", path.display())),
            Err(_) => Ok(Config::default()),
        }
    }

    pub fn shell(&self) -> String {
        self.shell
            .clone()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".into())
    }

    pub fn scrollback(&self) -> usize {
        if self.scrollback_infinite {
            1_000_000
        } else {
            self.scrollback_lines
        }
    }

    pub fn effective_opacity(&self, transparency_on: bool) -> f32 {
        if transparency_on {
            self.background_opacity.clamp(0.05, 1.0)
        } else {
            1.0
        }
    }

    /// alacritty's "semantic escape chars" derived from tilda-style `word_chars`.
    pub fn semantic_escape_chars(&self) -> String {
        let base = alacritty_terminal::term::SEMANTIC_ESCAPE_CHARS;
        if self.word_chars.is_empty() {
            return base.to_string();
        }
        let word = expand_char_ranges(&self.word_chars);
        base.chars().filter(|c| !word.contains(c)).collect()
    }
}

/// Expand "-A-Za-z0-9,./" (a leading/trailing '-' is literal) into characters.
pub fn expand_char_ranges(s: &str) -> Vec<char> {
    let chars: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    if chars.first() == Some(&'-') {
        out.push('-');
        i = 1;
    }
    while i < chars.len() {
        if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i] <= chars[i + 2] {
            out.extend(chars[i]..=chars[i + 2]);
            i += 3;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

pub type Rgb = [u8; 3];

pub fn parse_color(s: &str) -> Rgb {
    let s = s.trim();
    let named = match s.to_lowercase().as_str() {
        "black" => Some([0, 0, 0]),
        "white" => Some([255, 255, 255]),
        "red" => Some([255, 0, 0]),
        "green" => Some([0, 255, 0]),
        "blue" => Some([0, 0, 255]),
        "yellow" => Some([255, 255, 0]),
        "gray" | "grey" => Some([128, 128, 128]),
        _ => None,
    };
    if let Some(c) = named {
        return c;
    }
    let s = s.trim_start_matches('#');
    if s.len() == 6 {
        if let Ok(v) = u32::from_str_radix(s, 16) {
            return [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        }
    }
    [255, 0, 255]
}

pub fn hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Resolved colors used by the renderer.
#[derive(Clone)]
pub struct Theme {
    /// 0..=255 xterm palette, 256 fg, 257 bg, 258 cursor, 259..=266 dim, 267 bright fg, 268 dim fg
    pub table: Vec<Rgb>,
    pub cursor_text: Rgb,
    pub sel_fg: Rgb,
    pub sel_bg: Rgb,
    pub active_border: Rgb,
    pub inactive_border: Rgb,
    pub tab_bar_bg: Rgb,
    pub tab_active_fg: Rgb,
    pub tab_active_bg: Rgb,
    pub tab_inactive_fg: Rgb,
    pub tab_inactive_bg: Rgb,
    pub search_match: Rgb,
}

fn scale(c: Rgb, f: f32) -> Rgb {
    [(c[0] as f32 * f).min(255.0) as u8, (c[1] as f32 * f).min(255.0) as u8, (c[2] as f32 * f).min(255.0) as u8]
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    [m(a[0], b[0]), m(a[1], b[1]), m(a[2], b[2])]
}

impl Theme {
    pub fn from_config(cfg: &Config) -> Theme {
        let c = &cfg.colors;
        let scheme = crate::schemes::get(&cfg.color_scheme).unwrap_or_else(crate::schemes::default_scheme);
        let pick = |o: &Option<String>, d: Rgb| o.as_deref().filter(|s| !s.is_empty()).map(parse_color).unwrap_or(d);
        let mut t = vec![[0u8; 3]; 269];
        for (i, slot) in t.iter_mut().enumerate().take(16) {
            *slot = c.palette.as_ref().and_then(|p| p.get(i)).map(|s| parse_color(s)).unwrap_or(scheme.palette[i]);
        }
        let steps = [0u8, 95, 135, 175, 215, 255];
        for i in 0..216 {
            t[16 + i] = [steps[i / 36], steps[(i / 6) % 6], steps[i % 6]];
        }
        for i in 0..24 {
            let v = (8 + i * 10) as u8;
            t[232 + i] = [v, v, v];
        }
        let fg = pick(&c.foreground, scheme.fg);
        let bg = pick(&c.background, scheme.bg);
        t[256] = fg;
        t[257] = bg;
        t[258] = pick(&c.cursor, scheme.cursor.unwrap_or(fg));
        for i in 0..8 {
            t[259 + i] = scale(t[i], 0.66);
        }
        t[267] = fg;
        t[268] = scale(fg, 0.66);
        let light = (bg[0] as u32 + bg[1] as u32 + bg[2] as u32) > 384;
        let accent = t[4];
        Theme {
            cursor_text: pick(&c.cursor_text, bg),
            sel_fg: pick(&c.selection_foreground, bg),
            sel_bg: pick(&c.selection_background, accent),
            active_border: pick(&c.active_border, accent),
            inactive_border: pick(&c.inactive_border, mix(bg, fg, 0.25)),
            tab_bar_bg: pick(&c.tab_bar_background, if light { scale(bg, 0.92) } else { scale(bg, 0.8) }),
            tab_active_fg: pick(&c.active_tab_foreground, bg),
            tab_active_bg: pick(&c.active_tab_background, accent),
            tab_inactive_fg: pick(&c.inactive_tab_foreground, mix(bg, fg, 0.75)),
            tab_inactive_bg: pick(&c.inactive_tab_background, mix(bg, fg, 0.12)),
            search_match: pick(&c.search_match, t[3]),
            table: t,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn example_config_parses() {
        let s = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/dtem.example.toml")).unwrap();
        let c: super::Config = toml::from_str(&s).unwrap();
        assert!(c.confirm_close.tab && c.confirm_close.pane && c.confirm_close.window);
        let c: super::Config =
            toml::from_str("[confirm_close]\ntab = false\n[dropdown]\nhotkey = [\"F1\", \"ctrl+alt+t\"]").unwrap();
        assert!(!c.confirm_close.tab && c.confirm_close.window);
        assert_eq!(c.dropdown.hotkey.list(), vec!["F1", "ctrl+alt+t"]);
    }

    #[test]
    fn word_chars() {
        let v = super::expand_char_ranges("-A-Za-z0-9,./?%&#:_");
        assert!(v.contains(&'-') && v.contains(&'q') && v.contains(&'Q') && v.contains(&'5') && v.contains(&':'));
        let c = super::Config { word_chars: "-A-Za-z0-9,./?%&#:_".into(), ..Default::default() };
        let esc = c.semantic_escape_chars();
        assert!(!esc.contains(',') && !esc.contains(':') && esc.contains(' '));
    }
}
