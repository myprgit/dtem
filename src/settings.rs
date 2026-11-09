//! Settings page (`dtem --settings`, opened in a tab with kitty_mod+F2 / ctrl+,).
//! Every change is written to dtem.toml immediately (comments preserved) and the running
//! dtem instances pick it up live.
use crate::config::{hex, parse_color, Config, Theme};
use crate::{confedit, keys, schemes};
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};

#[derive(Clone)]
enum Kind {
    Bool,
    Int(i64, i64, i64),
    Float(f64, f64, f64),
    /// Fraction stored, shown/edited as a percentage (min, max, step in fraction units)
    Percent(f64, f64, f64),
    Text,
    Choice(&'static [&'static str]),
    Color,
    Palette(usize),
    Hotkeys,
    Binding(&'static str),
    /// GNOME's system-wide `enable-animations` (gsettings, not dtem.toml)
    GnomeAnimations,
}

#[derive(Clone)]
struct Item {
    path: String,
    label: String,
    kind: Kind,
    help: &'static str,
}

struct Section {
    name: &'static str,
    items: Vec<Item>,
}

fn it(path: &str, label: &str, kind: Kind, help: &'static str) -> Item {
    Item { path: path.into(), label: label.into(), kind, help }
}

fn sections() -> Vec<Section> {
    use Kind::*;
    let mut v = vec![
        Section {
            name: "Appearance",
            items: vec![
                it("font_family", "Font", Text, "fontconfig name (e.g. JetBrains Mono) or a font file path"),
                it("font_size", "Font size", Float(5.0, 48.0, 0.5), "points"),
                it("line_height", "Line height", Float(0.8, 2.0, 0.05), "multiplier"),
                it("padding", "Padding", Float(0.0, 40.0, 1.0), "space around the text (px)"),
                it("ligatures", "Ligatures", Bool, "-> => != … with fonts that support them"),
                it("color_scheme", "Color scheme", Choice(schemes::NAMES), "choosing a scheme clears custom colors"),
                it("transparency", "Transparency", Bool, "transparent background (toggle key: toggle_transparency)"),
                it("background_opacity", "Opacity", Float(0.05, 1.0, 0.05), "used while transparency is on"),
                it("cursor_shape", "Cursor shape", Choice(&["block", "beam", "underline"]), ""),
                it("cursor_blink", "Cursor blink", Bool, ""),
                it("bold_is_bright", "Bold is bright", Bool, "draw bold text with bright colors"),
                it("scrollbar", "Scrollbar", Choice(&["none", "right", "left"]), ""),
                it("tab_bar_position", "Tab bar", Choice(&["top", "bottom", "hidden"]), ""),
                it("tab_bar_min_tabs", "Show tab bar from", Int(1, 10, 1), "1 = always show the tab bar"),
                it("tab_bar_expand", "Expand tabs", Bool, "tabs share the full width"),
            ],
        },
        Section {
            name: "Colors",
            items: {
                let mut v = vec![
                    it("colors.foreground", "Foreground", Color, "#rrggbb; empty = from scheme"),
                    it("colors.background", "Background", Color, ""),
                    it("colors.cursor", "Cursor", Color, ""),
                    it("colors.cursor_text", "Cursor text", Color, ""),
                    it("colors.selection_background", "Selection", Color, ""),
                    it("colors.selection_foreground", "Selection text", Color, ""),
                    it("colors.active_border", "Active border", Color, ""),
                    it("colors.active_tab_background", "Active tab", Color, ""),
                    it("colors.tab_bar_background", "Tab bar", Color, ""),
                    it("colors.search_match", "Search match", Color, ""),
                ];
                let names = [
                    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "bright black", "bright red",
                    "bright green", "bright yellow", "bright blue", "bright magenta", "bright cyan", "bright white",
                ];
                for (i, n) in names.iter().enumerate() {
                    v.push(it("colors.palette", &format!("Palette {i:>2} {n}"), Palette(i), "ANSI palette"));
                }
                v
            },
        },
        Section { name: "Dropdown", items: dropdown_items("dropdown") },
        Section { name: "Dropdown 2", items: dropdown_items("dropdowns.2") },
        Section {
            name: "Notifications",
            items: vec![
                it("tab_activity_markers", "Tab markers ● 🔔 ✗", Bool, "mark tabs with unseen output, bells, failed commands"),
                it("notify_command_seconds", "Notify long commands", Int(0, 3600, 5), "seconds; 0 = off (needs shell integration)"),
                it("notify_on_bell", "Notify on bell", Bool, "bell in a tab you aren't looking at"),
                it("notify_programs", "Program notifications", Bool, "OSC 9 / OSC 777 from programs"),
                it("bell", "Bell", Bool, ""),
                it("visual_bell", "Visual bell", Bool, ""),
            ],
        },
        Section {
            name: "Behavior",
            items: vec![
                it("scrollback_lines", "Scrollback lines", Int(100, 1_000_000, 1000), ""),
                it("scrollback_infinite", "Infinite scrollback", Bool, ""),
                it("scroll_on_output", "Scroll on output", Bool, ""),
                it("scroll_on_key", "Scroll on keystroke", Bool, ""),
                it("word_chars", "Word characters", Text, "double-click selection, e.g. -A-Za-z0-9,./?%&#:_"),
                it("copy_on_select", "Copy on select", Bool, ""),
                it("url_requires_ctrl", "Ctrl+click opens links", Bool, "off = plain click opens links"),
                it("match_web_uris", "Match web links", Bool, ""),
                it("match_file_uris", "Match file:// links", Bool, ""),
                it("match_email", "Match e-mail addresses", Bool, ""),
                it("open_url_with", "Open links with", Text, ""),
                it("backspace_sends", "Backspace sends", Choice(&["auto", "del", "bs", "escape"]), ""),
                it("delete_sends", "Delete sends", Choice(&["escape", "del", "bs", "auto"]), ""),
                it("ime", "Input method (IME)", Bool, ""),
                it("kitty_keyboard", "Kitty keyboard protocol", Bool, ""),
                it("clipboard_read", "Programs reading clipboard", Choice(&["ask", "allow", "deny"]), "OSC 52"),
            ],
        },
        Section {
            name: "Shell & titles",
            items: vec![
                it("shell", "Shell", Text, "empty = $SHELL"),
                it("shell_integration", "Shell integration", Bool, "prompt jumps, last output copy, command notifications, remote cwd (new tabs)"),
                it("login_shell", "Login shell", Bool, ""),
                it("command", "Command for new tabs", Text, "empty = just the shell"),
                it("command_exit", "When command exits", Choice(&["shell", "restart", "close"]), ""),
                it("working_dir", "Working directory", Text, "used when not inheriting; empty = home"),
                it("inherit_working_dir", "Inherit directory", Bool, "new tabs start in the current tab's directory"),
                it("new_tab_after_current", "New tab after current", Bool, ""),
                it("title", "Title", Text, ""),
                it("dynamic_title", "Program titles", Choice(&["replace", "before", "after", "none"]), "how titles set by programs are shown"),
                it("title_max_length", "Max title length", Int(0, 200, 1), "0 = unlimited"),
                it("title_truncate", "Truncate", Choice(&["end", "start", "none"]), "end = keep the beginning"),
            ],
        },
        Section {
            name: "Session",
            items: vec![
                it("persist_scrollback", "Save scrollback", Bool, ""),
                it("persist_scrollback_lines", "Saved lines", Int(100, 1_000_000, 1000), ""),
                it("autosave_interval_secs", "Autosave interval", Int(1, 600, 1), "seconds"),
                it("confirm_close.tab", "Confirm closing tab", Bool, ""),
                it("confirm_close.pane", "Confirm closing pane", Bool, ""),
                it("confirm_close.window", "Confirm quit / window", Bool, ""),
                it("confirm_close.on_exit", "Ask when the shell exits", Bool, "after `exit` / Ctrl+D: close, restart or keep"),
                it("remote_install_helpers", "Install dtem-copy on ssh", Bool, ""),
                it("force_x11", "Force X11 (Wayland)", Bool, "restart needed"),
            ],
        },
    ];
    v.push(Section {
        name: "Keys",
        items: keys::ACTIONS.iter().map(|(a, d)| it("keybindings", d, Binding(a), "comma separated combos, e.g. ctrl+shift+t, f5")).collect(),
    });
    v
}

fn dropdown_items(p: &str) -> Vec<Item> {
    let mut v = dropdown_items_inner(p);
    if p == "dropdown" {
        let at = v.iter().position(|i| i.path.ends_with(".animation")).unwrap_or(v.len());
        v.insert(
            at,
            it("gnome.enable-animations", "GNOME animations", Kind::GnomeAnimations, "system-wide GNOME setting (all windows, not only dtem)"),
        );
    }
    v
}

fn gnome_animations() -> Option<bool> {
    let out = std::process::Command::new("gsettings").args(["get", "org.gnome.desktop.interface", "enable-animations"]).output().ok()?;
    match String::from_utf8_lossy(&out.stdout).trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn set_gnome_animations(on: bool) -> bool {
    std::process::Command::new("gsettings")
        .args(["set", "org.gnome.desktop.interface", "enable-animations", if on { "true" } else { "false" }])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn dropdown_items_inner(p: &str) -> Vec<Item> {
    use Kind::*;
    let f = |k: &str| format!("{p}.{k}");
    let first = if p == "dropdown" {
        it(&f("hotkey"), "Hotkey(s)", Hotkeys, "comma separated, e.g. F1, ctrl+alt+t")
    } else {
        it(&f("hotkey"), "Hotkey(s)", Hotkeys, "e.g. shift+F1 — setting a key creates this second dropdown")
    };
    vec![
        first,
        it(&f("centered_horizontally"), "Center horizontally", Bool, ""),
        it(&f("centered_vertically"), "Center vertically", Bool, ""),
        it(&f("width"), "Width", Percent(0.1, 1.0, 0.05), "% of the monitor width (←→ = 5%, or type e.g. 70)"),
        it(&f("height"), "Height", Percent(0.1, 1.0, 0.05), "% of the monitor height (←→ = 5%, or type e.g. 50)"),
        it(&f("x"), "X offset", Int(0, 10000, 10), "px inside the monitor (when not centered)"),
        it(&f("y"), "Y offset", Int(0, 10000, 10), "px inside the monitor (when not centered)"),
        it(&f("monitor"), "Initial monitor", Text, "e.g. DP-1; afterwards the last used monitor is remembered"),
        it(&f("color_scheme"), "Color scheme", Text, "empty = global scheme (Tab completes)"),
        it(&f("font_size"), "Font size", Float(5.0, 48.0, 0.5), "empty = global font size"),
        it(&f("animation"), "Slide animation", Bool, "dtem's own slide-in"),
        it(&f("animation_duration_ms"), "Slide time", Int(30, 2000, 10), "ms"),
        it(&f("animation_from"), "Slide from", Choice(&["top", "bottom", "left", "right"]), ""),
        it(&f("hide_on_focus_loss"), "Hide on focus loss", Bool, ""),
        it(&f("hide_on_mouse_leave"), "Hide on mouse leave", Bool, ""),
        it(&f("auto_hide_delay_ms"), "Auto-hide delay", Int(0, 10000, 100), "ms"),
        it(&f("unfocused_toggle"), "Hotkey when unfocused", Choice(&["focus", "hide"]), "visible but not focused: focus it or hide it"),
        it(&f("grab_focus"), "Grab focus on show", Bool, ""),
        it(&f("always_on_top"), "Always on top", Bool, ""),
        it(&f("skip_taskbar"), "Hide from taskbar", Bool, ""),
        it(&f("all_workspaces"), "All workspaces", Bool, ""),
        it(&f("start_hidden"), "Start hidden", Bool, ""),
        it(&f("start_fullscreen"), "Start fullscreen", Bool, ""),
        it(&f("on_last_tab_exit"), "Last tab exits", Choice(&["new_tab_hide", "new_tab", "quit"]), ""),
    ]
}

fn lookup<'a>(v: &'a toml::Value, path: &str) -> Option<&'a toml::Value> {
    let mut cur = v;
    for p in path.split('.') {
        cur = cur.get(p)?;
    }
    Some(cur)
}

fn display(v: Option<&toml::Value>) -> String {
    match v {
        None => String::new(),
        Some(toml::Value::String(s)) => s.clone(),
        Some(toml::Value::Float(f)) => {
            let s = format!("{f:.2}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
        Some(toml::Value::Array(a)) => a.iter().map(|x| display(Some(x))).collect::<Vec<_>>().join(", "),
        Some(other) => other.to_string(),
    }
}

/// Waiting for a key combination (Keys section). `true` = add instead of replace.
type Capture = Option<bool>;

/// crossterm key event → dtem combo ("ctrl+shift+t"); None for lone modifiers.
fn combo_of(k: &KeyEvent) -> Option<String> {
    let mut shift = k.modifiers.contains(KeyModifiers::SHIFT);
    let key = match k.code {
        KeyCode::Char(c) => match c.to_ascii_lowercase() {
            ' ' => "space".to_string(),
            '=' => "equal".into(),
            '+' => "plus".into(),
            '-' => "minus".into(),
            ',' => "comma".into(),
            '.' => "period".into(),
            '[' => "bracketleft".into(),
            ']' => "bracketright".into(),
            '/' => "slash".into(),
            '\\' => "backslash".into(),
            '`' => "grave".into(),
            ';' => "semicolon".into(),
            '\'' => "apostrophe".into(),
            c => c.to_string(),
        },
        KeyCode::F(n) => format!("f{n}"),
        KeyCode::Enter => "enter".into(),
        KeyCode::Tab => "tab".into(),
        KeyCode::BackTab => {
            shift = true;
            "tab".into()
        }
        KeyCode::Backspace => "backspace".into(),
        KeyCode::Delete => "delete".into(),
        KeyCode::Insert => "insert".into(),
        KeyCode::Home => "home".into(),
        KeyCode::End => "end".into(),
        KeyCode::PageUp => "page_up".into(),
        KeyCode::PageDown => "page_down".into(),
        KeyCode::Left => "left".into(),
        KeyCode::Right => "right".into(),
        KeyCode::Up => "up".into(),
        KeyCode::Down => "down".into(),
        KeyCode::Esc => "escape".into(),
        _ => return None,
    };
    let mut v = Vec::new();
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        v.push("ctrl".to_string());
    }
    if shift {
        v.push("shift".to_string());
    }
    if k.modifiers.contains(KeyModifiers::ALT) {
        v.push("alt".to_string());
    }
    if k.modifiers.contains(KeyModifiers::SUPER) {
        v.push("super".to_string());
    }
    v.push(key);
    Some(v.join("+"))
}

fn set_capture(on: bool) {
    let mut out = std::io::stdout();
    if on {
        let _ = ratatui::crossterm::execute!(
            out,
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
    } else {
        let _ = ratatui::crossterm::execute!(out, PopKeyboardEnhancementFlags);
    }
}

struct App {
    capture: Capture,
    /// completion candidates for the field being edited
    cands: Vec<String>,
    cand_sel: Option<usize>,
    cache: std::collections::HashMap<&'static str, Vec<String>>,
    gnome_anim: Option<bool>,
    sections: Vec<Section>,
    tab: usize,
    sel: Vec<ListState>,
    editing: Option<String>,
    status: String,
    cfg: Config,
    eff: toml::Value,
    theme: Theme,
}

impl App {
    fn reload(&mut self) {
        match Config::try_load() {
            Ok(c) => {
                self.cfg = c;
            }
            Err(e) => self.status = e,
        }
        self.eff = toml::Value::try_from(&self.cfg).unwrap_or(toml::Value::Table(Default::default()));
        self.theme = Theme::from_config(&self.cfg);
    }

    /// Effective value: config value, or the default for a not-yet-created dropdown profile.
    fn eff_value(&self, path: &str) -> Option<toml::Value> {
        if let Some(v) = lookup(&self.eff, path) {
            return Some(v.clone());
        }
        let rest = path.strip_prefix("dropdowns.")?;
        let (_, key) = rest.split_once('.')?;
        let d = toml::Value::try_from(crate::config::Dropdown::default()).ok()?;
        let v = lookup(&d, key)?.clone();
        // a profile without its own hotkey has none (not the default F12)
        if key == "hotkey" {
            return Some(toml::Value::String(String::new()));
        }
        Some(v)
    }

    fn item(&self) -> Option<&Item> {
        let i = self.sel[self.tab].selected()?;
        self.sections[self.tab].items.get(i)
    }

    fn value_of(&self, item: &Item) -> String {
        match &item.kind {
            Kind::Binding(a) => keys::effective_combos(&self.cfg, a).join(", "),
            Kind::GnomeAnimations => match self.gnome_anim {
                Some(b) => b.to_string(),
                None => "unavailable".into(),
            },
            Kind::Palette(i) => hex(self.theme.table[*i]),
            Kind::Percent(..) => {
                let f = self.eff_value(&item.path).and_then(|v| v.as_float()).unwrap_or(0.0);
                format!("{}%", (f * 100.0).round())
            }
            Kind::Color => {
                let v = display(lookup(&self.eff, &item.path));
                if v.is_empty() {
                    let eff = match item.path.as_str() {
                        "colors.foreground" => Some(self.theme.table[256]),
                        "colors.background" => Some(self.theme.table[257]),
                        "colors.cursor" => Some(self.theme.table[258]),
                        "colors.cursor_text" => Some(self.theme.cursor_text),
                        "colors.selection_background" => Some(self.theme.sel_bg),
                        "colors.selection_foreground" => Some(self.theme.sel_fg),
                        "colors.active_border" => Some(self.theme.active_border),
                        "colors.active_tab_background" => Some(self.theme.tab_active_bg),
                        "colors.tab_bar_background" => Some(self.theme.tab_bar_bg),
                        "colors.search_match" => Some(self.theme.search_match),
                        _ => None,
                    };
                    eff.map(|c| format!("{} (scheme)", hex(c))).unwrap_or_default()
                } else {
                    v
                }
            }
            _ => display(self.eff_value(&item.path).as_ref()),
        }
    }

    fn write(&mut self, f: impl FnOnce(&mut toml_edit::DocumentMut)) {
        let mut doc = confedit::load_doc();
        f(&mut doc);
        // validate before saving
        match toml::from_str::<Config>(&doc.to_string()) {
            Ok(_) => match confedit::save_doc(&doc) {
                Ok(_) => self.status = "saved — applied live".into(),
                Err(e) => self.status = format!("cannot save: {e}"),
            },
            Err(e) => self.status = format!("invalid value: {e}"),
        }
        self.reload();
    }

    fn set_value(&mut self, item: &Item, input: &str) {
        let input = input.trim().to_string();
        let path = item.path.clone();
        match item.kind.clone() {
            Kind::GnomeAnimations => {
                let on = matches!(input.as_str(), "true" | "yes" | "1" | "on");
                self.set_gnome(on);
            }
            Kind::Bool => {
                let b = matches!(input.as_str(), "true" | "yes" | "1" | "on");
                self.write(|d| confedit::set(d, &path, b));
            }
            Kind::Int(min, max, _) => match input.parse::<i64>() {
                Ok(v) => self.write(|d| confedit::set(d, &path, v.clamp(min, max))),
                Err(_) => self.status = "not a number".into(),
            },
            Kind::Float(min, max, _) => match input.replace(',', ".").parse::<f64>() {
                Ok(v) => self.write(|d| confedit::set(d, &path, (v.clamp(min, max) * 100.0).round() / 100.0)),
                Err(_) => self.status = "not a number".into(),
            },
            Kind::Percent(min, max, _) => {
                let t = input.trim_end_matches('%').trim().replace(',', ".");
                match t.parse::<f64>() {
                    // "70" / "70%" are percentages; "0.7" is a fraction
                    Ok(v) => {
                        let f = if v > 1.0 || input.contains('%') { v / 100.0 } else { v };
                        self.write(|d| confedit::set(d, &path, (f.clamp(min, max) * 100.0).round() / 100.0))
                    }
                    Err(_) => self.status = "not a number".into(),
                }
            }
            Kind::Text | Kind::Choice(_) => {
                if input.is_empty() && matches!(item.kind, Kind::Text) {
                    self.write(|d| confedit::remove(d, &path));
                } else {
                    self.write(|d| confedit::set(d, &path, input.clone()));
                }
            }
            Kind::Color => {
                if input.is_empty() {
                    self.write(|d| confedit::remove(d, &path));
                } else if parse_color(&input) == [255, 0, 255] && !input.eq_ignore_ascii_case("#ff00ff") {
                    self.status = "use #rrggbb".into();
                } else {
                    let h = hex(parse_color(&input));
                    self.write(|d| confedit::set(d, &path, h));
                }
            }
            Kind::Palette(i) => {
                let c = parse_color(&input);
                if c == [255, 0, 255] && !input.eq_ignore_ascii_case("#ff00ff") {
                    self.status = "use #rrggbb".into();
                    return;
                }
                let mut pal: Vec<String> = (0..16).map(|k| hex(self.theme.table[k])).collect();
                pal[i] = hex(c);
                self.write(|d| confedit::set(d, "colors.palette", confedit::string_array(&pal)));
            }
            Kind::Hotkeys => {
                let list: Vec<String> = input.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                if let Some(bad) = list.iter().find(|k| crate::hotkey_from_config(k).is_none()) {
                    self.status = format!("unknown key: {bad}");
                    return;
                }
                self.write(|d| {
                    if list.len() == 1 {
                        confedit::set(d, &path, list[0].clone())
                    } else {
                        confedit::set(d, &path, confedit::string_array(&list))
                    }
                });
                self.status.push_str(" (GNOME shortcut updated by the running dropdown)");
            }
            Kind::Binding(action) => {
                let kmod = self.cfg.kitty_mod.clone();
                let mut new = Vec::new();
                for c in input.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
                    match keys::normalize_combo(c, &kmod) {
                        Some(n) => new.push(n),
                        None => {
                            self.status = format!("cannot parse: {c}");
                            return;
                        }
                    }
                }
                self.rebind(action, new);
            }
        }
    }

    /// Bind a captured combo to `action` (replace or add), reporting what it took over.
    fn bind_captured(&mut self, action: &str, combo: String, add: bool) {
        let taken: Vec<&str> = keys::ACTIONS
            .iter()
            .filter(|(a, _)| *a != action && keys::effective_combos(&self.cfg, a).contains(&combo))
            .map(|(_, d)| *d)
            .collect();
        let mut list = if add { keys::effective_combos(&self.cfg, action) } else { Vec::new() };
        if !list.contains(&combo) {
            list.push(combo.clone());
        }
        self.rebind(action, list);
        if !taken.is_empty() {
            self.status = format!("{combo} assigned (removed from: {})", taken.join(", "));
        } else {
            self.status = format!("{combo} assigned — applied live");
        }
    }

    /// Make `action` bound to exactly `combos` (defaults unbound with "none" when removed).
    fn rebind(&mut self, action: &str, combos: Vec<String>) {
        let kmod = self.cfg.kitty_mod.clone();
        let defaults: Vec<String> = keys::default_combos(action).iter().filter_map(|c| keys::normalize_combo(c, &kmod)).collect();
        let existing = self.cfg.keybindings.clone();
        self.write(|d| {
            // drop user entries for this action and entries for the combos we assign
            for (combo, act) in &existing {
                let n = keys::normalize_combo(combo, &kmod).unwrap_or_default();
                if act.trim() == action || combos.contains(&n) {
                    if let Some(t) = d.get_mut("keybindings").and_then(|t| t.as_table_like_mut()) {
                        t.remove(combo);
                    }
                }
            }
            for dc in &defaults {
                if !combos.contains(dc) {
                    confedit::set(d, &format!("keybindings.{}", dc.replace('.', "period")), "none");
                }
            }
            for c in &combos {
                if !defaults.contains(c) {
                    confedit::set(d, &format!("keybindings.{}", c.replace('.', "period")), action);
                }
            }
        });
    }

    fn set_gnome(&mut self, on: bool) {
        if self.gnome_anim.is_none() {
            self.status = "GNOME settings not available (gsettings)".into();
            return;
        }
        if set_gnome_animations(on) {
            self.gnome_anim = gnome_animations();
            self.status = format!("GNOME animations {} (system-wide)", if on { "on" } else { "off" });
        } else {
            self.status = "gsettings failed".into();
        }
    }

    fn reset(&mut self, item: &Item) {
        if matches!(item.kind, Kind::GnomeAnimations) {
            self.set_gnome(true); // GNOME's default
            return;
        }
        let path = item.path.clone();
        match item.kind {
            Kind::Binding(a) => {
                let defaults: Vec<String> = keys::default_combos(a)
                    .iter()
                    .filter_map(|c| keys::normalize_combo(c, &self.cfg.kitty_mod))
                    .collect();
                self.rebind(a, defaults);
            }
            Kind::Palette(_) => self.write(|d| confedit::remove(d, "colors.palette")),
            _ => self.write(|d| confedit::remove(d, &path)),
        }
        self.status = format!("{} reset to default", item.label.trim());
    }

    /// Left/right on adjustable items.
    fn adjust(&mut self, item: &Item, dir: i64) {
        let cur = self.value_of(item);
        match &item.kind {
            Kind::Bool => self.set_value(item, if cur == "true" { "false" } else { "true" }),
            Kind::GnomeAnimations => self.set_gnome(cur != "true"),
            Kind::Choice(opts) => {
                let i = opts.iter().position(|o| *o == cur).unwrap_or(0) as i64;
                let n = (i + dir).rem_euclid(opts.len() as i64) as usize;
                let v = opts[n];
                self.set_value(item, v);
                if item.path == "color_scheme" {
                    self.write(|d| {
                        for k in ["colors.foreground", "colors.background", "colors.cursor", "colors.palette"] {
                            confedit::remove(d, k);
                        }
                    });
                }
            }
            Kind::Int(_, _, step) => {
                let v = cur.parse::<i64>().unwrap_or(0) + dir * step;
                self.set_value(item, &v.to_string());
            }
            Kind::Float(_, _, step) => {
                let v = cur.parse::<f64>().unwrap_or(0.0) + dir as f64 * step;
                self.set_value(item, &format!("{v:.3}"));
            }
            Kind::Percent(_, _, step) => {
                let f = self.eff_value(&item.path).and_then(|v| v.as_float()).unwrap_or(0.5);
                let v = ((f + dir as f64 * step) * 100.0).round();
                self.set_value(item, &format!("{v}%"));
            }
            _ => {}
        }
    }
}

fn cmd_lines(cmd: &str, args: &[&str]) -> Vec<String> {
    std::process::Command::new(cmd)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().map(|l| l.to_string()).collect())
        .unwrap_or_default()
}

fn font_names() -> Vec<String> {
    let mut v: Vec<String> = cmd_lines("fc-list", &[":spacing=mono", "family"])
        .into_iter()
        .flat_map(|l| l.split(',').next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect();
    v.push("monospace".into());
    v.sort_by_key(|s| s.to_lowercase());
    v.dedup();
    v
}

fn monitor_names() -> Vec<String> {
    cmd_lines("xrandr", &["--listmonitors"]).iter().skip(1).filter_map(|l| l.split_whitespace().last().map(|s| s.to_string())).collect()
}

fn shells() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_to_string("/etc/shells")
        .unwrap_or_default()
        .lines()
        .filter(|l| l.starts_with('/'))
        .map(|l| l.to_string())
        .collect();
    v.dedup();
    v
}

fn in_path(prog: &str) -> bool {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|d| d.join(prog).is_file()))
        .unwrap_or(false)
}

fn browsers() -> Vec<String> {
    ["xdg-open", "firefox", "google-chrome", "chromium", "chromium-browser", "brave-browser", "zen", "zen-browser", "microsoft-edge", "opera", "vivaldi"]
        .iter()
        .filter(|b| in_path(b))
        .map(|s| s.to_string())
        .collect()
}

const KEY_NAMES: &[&str] = &[
    "ctrl", "shift", "alt", "super", "kitty_mod", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11",
    "f12", "enter", "tab", "space", "backspace", "escape", "delete", "insert", "home", "end", "page_up", "page_down",
    "left", "right", "up", "down", "comma", "period", "minus", "equal", "plus", "bracketleft", "bracketright", "slash",
    "backslash", "grave", "semicolon", "apostrophe", "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n",
    "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z", "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
];

/// Complete a filesystem path: (directory part kept, entries)
fn path_cands(token: &str) -> Vec<String> {
    let home = dirs::home_dir().unwrap_or_default().display().to_string();
    let expanded = if let Some(r) = token.strip_prefix('~') { format!("{home}{r}") } else { token.to_string() };
    let (dir, _) = match expanded.rfind('/') {
        Some(i) => (&expanded[..=i], &expanded[i + 1..]),
        None => ("./", expanded.as_str()),
    };
    let shown_dir = match token.rfind('/') {
        Some(i) => &token[..=i],
        None => "",
    };
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                    Some(format!("{shown_dir}{name}{}", if is_dir { "/" } else { "" }))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// (fixed prefix of the buffer, token being completed)
fn split_token(item: &Item, buf: &str) -> (String, String) {
    match item.kind {
        Kind::Hotkeys | Kind::Binding(_) => {
            let i = buf.rfind(|c| c == '+' || c == ',').map(|i| i + 1).unwrap_or(0);
            let tok = buf[i..].trim_start();
            (buf[..buf.len() - tok.len()].to_string(), tok.to_string())
        }
        _ => (String::new(), buf.to_string()),
    }
}

fn filter_cands(all: &[String], token: &str) -> Vec<String> {
    let t = token.to_lowercase();
    let mut v: Vec<String> = all.iter().filter(|c| c.to_lowercase().starts_with(&t)).cloned().collect();
    if v.is_empty() && !t.is_empty() {
        v = all.iter().filter(|c| c.to_lowercase().contains(&t)).cloned().collect();
    }
    v
}

fn common_prefix(v: &[String]) -> String {
    let Some(first) = v.first() else { return String::new() };
    let mut n = first.len();
    for s in v.iter().skip(1) {
        n = first.char_indices().zip(s.chars()).take_while(|((_, a), b)| a.eq_ignore_ascii_case(b)).count().min(n);
    }
    first.chars().take(n).collect()
}

impl App {
    fn source(&mut self, item: &Item, token: &str) -> Vec<String> {
        let path_like = token.starts_with('/') || token.starts_with('~') || token.starts_with('.');
        let key: &'static str = match (item.path.as_str(), &item.kind) {
            (_, Kind::Hotkeys) | (_, Kind::Binding(_)) => return KEY_NAMES.iter().map(|s| s.to_string()).collect(),
            ("font_family", _) if path_like => return path_cands(token),
            ("font_family", _) => "fonts",
            (p, _) if p.ends_with(".monitor") => "monitors",
            (p, _) if p.ends_with("color_scheme") => return schemes::NAMES.iter().map(|s| s.to_string()).collect(),
            ("shell", _) => "shells",
            ("open_url_with", _) => "browsers",
            ("working_dir", _) | ("command", _) if path_like => return path_cands(token),
            ("working_dir", _) => return path_cands(&format!("~/{token}")),
            (_, Kind::Choice(opts)) => return opts.iter().map(|s| s.to_string()).collect(),
            _ => return Vec::new(),
        };
        self.cache
            .entry(key)
            .or_insert_with(|| match key {
                "fonts" => font_names(),
                "monitors" => monitor_names(),
                "shells" => shells(),
                _ => browsers(),
            })
            .clone()
    }

    /// Recompute candidates for the current edit buffer.
    fn refresh_cands(&mut self) {
        self.cand_sel = None;
        self.cands.clear();
        let (Some(buf), Some(item)) = (self.editing.clone(), self.item().cloned()) else { return };
        let (_, token) = split_token(&item, &buf);
        let all = self.source(&item, &token);
        self.cands = filter_cands(&all, &token);
        if self.cands.len() == 1 && self.cands[0] == token {
            self.cands.clear();
        }
    }

    /// Tab / Shift+Tab while editing.
    fn complete(&mut self, back: bool) {
        let (Some(buf), Some(item)) = (self.editing.clone(), self.item().cloned()) else { return };
        if self.cands.is_empty() {
            self.refresh_cands();
            if self.cands.is_empty() {
                self.status = "no completions".into();
                return;
            }
        }
        let (keep, token) = split_token(&item, &buf);
        if self.cand_sel.is_none() {
            let cp = common_prefix(&self.cands);
            if self.cands.len() == 1 || cp.chars().count() > token.chars().count() {
                let full = if self.cands.len() == 1 { self.cands[0].clone() } else { cp };
                self.editing = Some(format!("{keep}{full}"));
                let was_dir = full.ends_with('/');
                self.refresh_cands();
                if was_dir || self.cands.len() > 1 {
                    return;
                }
                self.cands.clear();
                return;
            }
        }
        let n = self.cands.len();
        let i = match self.cand_sel {
            None => if back { n - 1 } else { 0 },
            Some(i) => if back { (i + n - 1) % n } else { (i + 1) % n },
        };
        self.cand_sel = Some(i);
        // keep = everything before the token of the *original* text
        let (keep, _) = split_token(&item, &buf);
        self.editing = Some(format!("{keep}{}", self.cands[i]));
    }

    /// Remainder of the best candidate (shown dimmed after the cursor).
    fn ghost(&self) -> String {
        let (Some(buf), Some(item)) = (self.editing.as_ref(), self.item()) else { return String::new() };
        if self.cand_sel.is_some() {
            return String::new();
        }
        let (_, token) = split_token(item, buf);
        match self.cands.first() {
            Some(c) if !token.is_empty() && c.to_lowercase().starts_with(&token.to_lowercase()) => {
                c.chars().skip(token.chars().count()).collect()
            }
            _ => String::new(),
        }
    }
}

fn swatch(s: &str) -> Option<Color> {
    let h = s.split_whitespace().next()?;
    if !h.starts_with('#') || h.len() != 7 {
        return None;
    }
    let c = parse_color(h);
    Some(Color::Rgb(c[0], c[1], c[2]))
}

pub fn run(section: Option<String>) -> anyhow::Result<()> {
    let mut app = App {
        capture: None,
        cands: Vec::new(),
        cand_sel: None,
        cache: Default::default(),
        gnome_anim: gnome_animations(),
        sections: sections(),
        tab: 0,
        sel: Vec::new(),
        editing: None,
        status: format!("{}", crate::config::config_path().display()),
        cfg: Config::default(),
        eff: toml::Value::Table(Default::default()),
        theme: Theme::from_config(&Config::default()),
    };
    app.sel = app.sections.iter().map(|_| {
        let mut s = ListState::default();
        s.select(Some(0));
        s
    }).collect();
    app.reload();
    if let Some(sec) = section {
        if let Some(i) = app.sections.iter().position(|s| s.name.to_lowercase().starts_with(&sec.to_lowercase())) {
            app.tab = i;
        }
    }

    let mut term = ratatui::init();
    let res = (|| -> anyhow::Result<()> {
        loop {
            term.draw(|f| {
                let area = f.area();
                let [top, mid, help, status] =
                    Layout::vertical([Constraint::Length(3), Constraint::Min(5), Constraint::Length(2), Constraint::Length(1)])
                        .areas(area);
                let titles: Vec<&str> = app.sections.iter().map(|s| s.name).collect();
                f.render_widget(
                    Tabs::new(titles)
                        .select(app.tab)
                        .block(Block::default().borders(Borders::ALL).title(" dtem settings "))
                        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)),
                    top,
                );
                let label_w = 30usize;
                let items: Vec<ListItem> = app.sections[app.tab]
                    .items
                    .iter()
                    .map(|item| {
                        let v = app.value_of(item);
                        let mut spans = vec![Span::raw(format!(" {:<label_w$} ", item.label))];
                        if let Some(c) = swatch(&v) {
                            spans.push(Span::styled("    ", Style::default().bg(c)));
                            spans.push(Span::raw(" "));
                        }
                        let shown = match item.kind {
                            Kind::Bool | Kind::GnomeAnimations if v == "true" => "[x] on".into(),
                            Kind::Bool | Kind::GnomeAnimations if v == "false" => "[ ] off".into(),
                            Kind::Choice(_) => format!("‹ {v} ›"),
                            _ => v,
                        };
                        spans.push(Span::styled(shown, Style::default().fg(Color::Yellow)));
                        ListItem::new(Line::from(spans))
                    })
                    .collect();
                let list = List::new(items)
                    .block(Block::default().borders(Borders::ALL))
                    .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
                    .highlight_symbol("▶");
                f.render_stateful_widget(list, mid, &mut app.sel[app.tab]);

                let help_text = match (&app.editing, app.item()) {
                    _ if app.capture.is_some() => format!(
                        " {}: PRESS THE NEW KEY COMBINATION…   (Esc: cancel)",
                        app.item().map(|i| i.label.trim().to_string()).unwrap_or_default()
                    ),
                    (Some(_), Some(_)) => String::new(),
                    (None, Some(item)) if matches!(item.kind, Kind::Binding(_)) => {
                        " Enter: press new key   a: add another key   e: type keys   x: unbind   r: default\n Tab/Shift+Tab: section  ↑↓: select  q: quit".to_string()
                    }
                    (None, Some(item)) => format!(
                        " {}\n Tab/Shift+Tab: section  ↑↓: select  ←→/Space: change  Enter: edit  r: reset  q: quit",
                        item.help
                    ),
                    _ => String::new(),
                };
                if let (Some(buf), Some(item)) = (&app.editing, app.item()) {
                    let ghost = app.ghost();
                    let line1 = Line::from(vec![
                        Span::raw(format!(" {}: ", item.label.trim())),
                        Span::styled(buf.clone(), Style::default().fg(Color::Yellow)),
                        Span::styled("▏", Style::default().fg(Color::White)),
                        Span::styled(ghost, Style::default().fg(Color::DarkGray)),
                    ]);
                    let width = help.width as usize;
                    let mut spans = vec![Span::raw(" ")];
                    let mut used = 1usize;
                    let shown = app.cands.len();
                    for (i, c) in app.cands.iter().enumerate() {
                        let w = c.chars().count() + 2;
                        if used + w + 12 > width {
                            spans.push(Span::styled(format!("… +{} more", shown - i), Style::default().fg(Color::DarkGray)));
                            break;
                        }
                        let st = if app.cand_sel == Some(i) {
                            Style::default().fg(Color::Black).bg(Color::Cyan)
                        } else {
                            Style::default().fg(Color::Cyan)
                        };
                        spans.push(Span::styled(c.clone(), st));
                        spans.push(Span::raw("  "));
                        used += w;
                    }
                    let line2 = if app.cands.is_empty() {
                        Line::from(Span::styled(" Tab: complete   →: accept suggestion   Enter: save   Esc: cancel", Style::default().fg(Color::DarkGray)))
                    } else {
                        Line::from(spans)
                    };
                    f.render_widget(Paragraph::new(vec![line1, line2]), help);
                } else {
                    f.render_widget(Paragraph::new(help_text), help);
                }
                f.render_widget(Paragraph::new(Span::styled(format!(" {}", app.status), Style::default().fg(Color::Green))), status);
            })?;

            let Event::Key(k) = event::read()? else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if let Some(add) = app.capture {
                if matches!(k.code, KeyCode::Modifier(_)) {
                    continue;
                }
                app.capture = None;
                set_capture(false);
                if k.code == KeyCode::Esc && k.modifiers.is_empty() {
                    app.status = "cancelled".into();
                    continue;
                }
                if let (Some(combo), Some(Kind::Binding(action))) = (combo_of(&k), app.item().map(|i| i.kind.clone())) {
                    app.bind_captured(action, combo, add);
                }
                continue;
            }
            if let Some(buf) = app.editing.as_mut() {
                match k.code {
                    KeyCode::Enter => {
                        app.cands.clear();
                        let input = app.editing.take().unwrap_or_default();
                        if let Some(item) = app.item().cloned() {
                            app.set_value(&item, &input);
                        }
                    }
                    KeyCode::Esc => {
                        app.editing = None;
                        app.cands.clear();
                    }
                    KeyCode::Tab => app.complete(false),
                    KeyCode::BackTab => app.complete(true),
                    KeyCode::Right | KeyCode::End => {
                        let g = app.ghost();
                        if !g.is_empty() {
                            if let Some(b) = app.editing.as_mut() {
                                b.push_str(&g);
                            }
                            app.refresh_cands();
                        }
                    }
                    KeyCode::Backspace => {
                        buf.pop();
                        app.refresh_cands();
                    }
                    KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                        buf.clear();
                        app.refresh_cands();
                    }
                    KeyCode::Char(c) => {
                        buf.push(c);
                        app.refresh_cands();
                    }
                    _ => {}
                }
                continue;
            }
            let n = app.sections[app.tab].items.len();
            let sel = app.sel[app.tab].selected().unwrap_or(0);
            match k.code {
                KeyCode::Char('q') | KeyCode::Esc => break,
                KeyCode::Tab => app.tab = (app.tab + 1) % app.sections.len(),
                KeyCode::BackTab => app.tab = (app.tab + app.sections.len() - 1) % app.sections.len(),
                KeyCode::Down | KeyCode::Char('j') => app.sel[app.tab].select(Some((sel + 1) % n)),
                KeyCode::Up | KeyCode::Char('k') => app.sel[app.tab].select(Some((sel + n - 1) % n)),
                KeyCode::PageDown => app.sel[app.tab].select(Some((sel + 10).min(n - 1))),
                KeyCode::PageUp => app.sel[app.tab].select(Some(sel.saturating_sub(10))),
                KeyCode::Left | KeyCode::Char('h') | KeyCode::Right | KeyCode::Char('l') | KeyCode::Char(' ') => {
                    if let Some(item) = app.item().cloned() {
                        let dir = if matches!(k.code, KeyCode::Left | KeyCode::Char('h')) { -1 } else { 1 };
                        app.adjust(&item, dir);
                    }
                }
                KeyCode::Enter | KeyCode::Char('a') | KeyCode::Char('e') | KeyCode::Char('x')
                    if matches!(app.item().map(|i| &i.kind), Some(Kind::Binding(_))) =>
                {
                    let Some(Kind::Binding(action)) = app.item().map(|i| i.kind.clone()) else { continue };
                    match k.code {
                        KeyCode::Char('e') => {
                            app.editing = Some(keys::effective_combos(&app.cfg, action).join(", "));
                            app.refresh_cands();
                        }
                        KeyCode::Char('x') => {
                            app.rebind(action, Vec::new());
                            app.status = "unbound".into();
                        }
                        _ => {
                            app.capture = Some(k.code == KeyCode::Char('a'));
                            set_capture(true);
                        }
                    }
                }
                KeyCode::Enter => {
                    if let Some(item) = app.item().cloned() {
                        match item.kind {
                            Kind::Bool | Kind::Choice(_) | Kind::GnomeAnimations => app.adjust(&item, 1),
                            _ => {
                                let v = app.value_of(&item);
                                let v = v.trim_end_matches(" (scheme)").trim_end_matches('%').to_string();
                                app.editing = Some(v);
                                app.refresh_cands();
                            }
                        }
                    }
                }
                KeyCode::Char('r') | KeyCode::Delete => {
                    if let Some(item) = app.item().cloned() {
                        app.reset(&item);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();
    ratatui::restore();
    res
}
