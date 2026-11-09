//! Configuration: ~/.config/dtem/dtem.toml
use serde::Deserialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// fontconfig family/pattern, e.g. "JetBrains Mono", "monospace"
    pub font_family: String,
    /// Point size
    pub font_size: f32,
    /// Line height multiplier
    pub line_height: f32,
    /// Inner padding of each pane (logical px)
    pub padding: f32,
    pub scrollback_lines: usize,
    /// Persist scrollback across restarts (incl. reboot)
    pub persist_scrollback: bool,
    /// Max lines persisted per pane
    pub persist_scrollback_lines: usize,
    /// Shell to use; default $SHELL
    pub shell: Option<String>,
    /// Autosave interval (seconds) for the session (protects against reboot/crash)
    pub autosave_interval_secs: u64,
    /// Show tab bar when tab count >= this (kitty default: 2)
    pub tab_bar_min_tabs: usize,
    /// "block" | "beam" | "underline"
    pub cursor_shape: String,
    pub copy_on_select: bool,
    /// Under Wayland, run normal windows through XWayland (enables position restore)
    pub force_x11: bool,
    /// Modifier used for `kitty_mod` in key bindings
    pub kitty_mod: String,
    /// Programming ligatures (fonts like Fira Code / JetBrains Mono)
    pub ligatures: bool,
    /// 0.0..1.0 — needs a compositor
    pub background_opacity: f32,
    /// Input method (ibus/fcitx) support
    pub ime: bool,
    /// Allow apps (nvim, ...) to enable the kitty keyboard protocol
    pub kitty_keyboard: bool,
    /// Programs reading the clipboard via OSC 52 (e.g. remote nvim): "ask" | "allow" | "deny"
    pub clipboard_read: String,
    /// Command used to open URLs (ctrl+click)
    pub open_url_with: String,
    /// Install the `dtem-copy` helper on hosts opened with `dtem --remote`
    pub remote_install_helpers: bool,
    pub colors: Colors,
    pub dropdown: Dropdown,
    /// Ask before closing (each can be disabled)
    pub confirm_close: ConfirmClose,
    /// "combo" = "action"; action "none" removes a default binding
    pub keybindings: HashMap<String, String>,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Dropdown {
    /// Global hotkey (X11 session) and the binding used by --install-gnome-shortcut
    pub hotkey: String,
    /// Fraction of monitor height / width
    pub height: f32,
    pub width: f32,
    pub hide_on_focus_loss: bool,
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct ConfirmClose {
    /// Closing a tab
    pub tab: bool,
    /// Closing a split pane (kitty "window")
    pub pane: bool,
    /// Closing an OS window / the whole terminal
    pub window: bool,
}

impl Default for ConfirmClose {
    fn default() -> Self {
        Self { tab: true, pane: true, window: true }
    }
}

#[derive(Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Colors {
    pub foreground: String,
    pub background: String,
    pub cursor: String,
    pub cursor_text: String,
    pub selection_foreground: String,
    pub selection_background: String,
    pub active_border: String,
    pub inactive_border: String,
    pub tab_bar_background: String,
    pub active_tab_foreground: String,
    pub active_tab_background: String,
    pub inactive_tab_foreground: String,
    pub inactive_tab_background: String,
    /// 16 ANSI colors
    pub palette: Vec<String>,
}

impl Default for Colors {
    fn default() -> Self {
        Self {
            foreground: "#dddddd".into(),
            background: "#1a1b26".into(),
            cursor: "#c0caf5".into(),
            cursor_text: "#1a1b26".into(),
            selection_foreground: "#1a1b26".into(),
            selection_background: "#7aa2f7".into(),
            active_border: "#7aa2f7".into(),
            inactive_border: "#3b4261".into(),
            tab_bar_background: "#16161e".into(),
            active_tab_foreground: "#1a1b26".into(),
            active_tab_background: "#7aa2f7".into(),
            inactive_tab_foreground: "#a9b1d6".into(),
            inactive_tab_background: "#24283b".into(),
            palette: [
                "#15161e", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#a9b1d6",
                "#414868", "#f7768e", "#9ece6a", "#e0af68", "#7aa2f7", "#bb9af7", "#7dcfff", "#c0caf5",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        }
    }
}

impl Default for Dropdown {
    fn default() -> Self {
        Self { hotkey: "F12".into(), height: 0.45, width: 1.0, hide_on_focus_loss: false }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            font_family: "monospace".into(),
            font_size: 11.0,
            line_height: 1.0,
            padding: 4.0,
            scrollback_lines: 10_000,
            persist_scrollback: true,
            persist_scrollback_lines: 10_000,
            shell: None,
            autosave_interval_secs: 5,
            tab_bar_min_tabs: 2,
            cursor_shape: "block".into(),
            copy_on_select: false,
            force_x11: false,
            kitty_mod: "ctrl+shift".into(),
            ligatures: true,
            background_opacity: 1.0,
            ime: true,
            kitty_keyboard: true,
            clipboard_read: "ask".into(),
            open_url_with: "xdg-open".into(),
            remote_install_helpers: true,
            colors: Colors::default(),
            dropdown: Dropdown::default(),
            confirm_close: ConfirmClose::default(),
            keybindings: HashMap::new(),
        }
    }
}

pub fn config_dir() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("dtem")
}

pub fn state_dir() -> PathBuf {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("dtem")
}

impl Config {
    pub fn load() -> Config {
        let path = config_dir().join("dtem.toml");
        match std::fs::read_to_string(&path) {
            Ok(s) => match toml::from_str::<Config>(&s) {
                Ok(c) => c,
                Err(e) => {
                    log::error!("config parse error in {}: {e}", path.display());
                    Config::default()
                }
            },
            Err(_) => Config::default(),
        }
    }

    pub fn shell(&self) -> String {
        self.shell
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/bash".into())
    }
}

pub type Rgb = [u8; 3];

pub fn parse_color(s: &str) -> Rgb {
    let s = s.trim().trim_start_matches('#');
    if s.len() == 6 {
        if let Ok(v) = u32::from_str_radix(s, 16) {
            return [(v >> 16) as u8, (v >> 8) as u8, v as u8];
        }
    }
    [255, 0, 255]
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
}

fn dim(c: Rgb) -> Rgb {
    [(c[0] as f32 * 0.66) as u8, (c[1] as f32 * 0.66) as u8, (c[2] as f32 * 0.66) as u8]
}

impl Theme {
    pub fn from_config(c: &Colors) -> Theme {
        let mut t = vec![[0u8; 3]; 269];
        for i in 0..16 {
            t[i] = c.palette.get(i).map(|s| parse_color(s)).unwrap_or([128, 128, 128]);
        }
        let steps = [0u8, 95, 135, 175, 215, 255];
        for i in 0..216 {
            t[16 + i] = [steps[i / 36], steps[(i / 6) % 6], steps[i % 6]];
        }
        for i in 0..24 {
            let v = (8 + i * 10) as u8;
            t[232 + i] = [v, v, v];
        }
        let fg = parse_color(&c.foreground);
        t[256] = fg;
        t[257] = parse_color(&c.background);
        t[258] = parse_color(&c.cursor);
        for i in 0..8 {
            t[259 + i] = dim(t[i]);
        }
        t[267] = fg;
        t[268] = dim(fg);
        Theme {
            table: t,
            cursor_text: parse_color(&c.cursor_text),
            sel_fg: parse_color(&c.selection_foreground),
            sel_bg: parse_color(&c.selection_background),
            active_border: parse_color(&c.active_border),
            inactive_border: parse_color(&c.inactive_border),
            tab_bar_bg: parse_color(&c.tab_bar_background),
            tab_active_fg: parse_color(&c.active_tab_foreground),
            tab_active_bg: parse_color(&c.active_tab_background),
            tab_inactive_fg: parse_color(&c.inactive_tab_foreground),
            tab_inactive_bg: parse_color(&c.inactive_tab_background),
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
        assert_eq!(c.dropdown.hotkey, "F12");
        let c: super::Config = toml::from_str("[confirm_close]\ntab = false").unwrap();
        assert!(!c.confirm_close.tab && c.confirm_close.window);
    }
}
