//! Settings page (`dtem --settings`, opened in a tab with kitty_mod+F2 / ctrl+,).
//! Every change is written to dtem.toml immediately (comments preserved) and the running
//! dtem instances pick it up live.
use crate::config::{hex, parse_color, Config, Theme};
use crate::{confedit, keys, schemes};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Tabs};

#[derive(Clone)]
enum Kind {
    Bool,
    Int(i64, i64, i64),
    Float(f64, f64, f64),
    Text,
    Choice(&'static [&'static str]),
    Color,
    Palette(usize),
    Hotkeys,
    Binding(&'static str),
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
        Section {
            name: "Dropdown",
            items: vec![
                it("dropdown.hotkey", "Hotkey(s)", Hotkeys, "comma separated, e.g. F1, ctrl+alt+t"),
                it("dropdown.width", "Width", Float(0.1, 1.0, 0.05), "fraction of the monitor"),
                it("dropdown.height", "Height", Float(0.1, 1.0, 0.05), "fraction of the monitor"),
                it("dropdown.centered_horizontally", "Center horizontally", Bool, ""),
                it("dropdown.centered_vertically", "Center vertically", Bool, ""),
                it("dropdown.x", "X offset", Int(0, 10000, 10), "px inside the monitor (when not centered)"),
                it("dropdown.y", "Y offset", Int(0, 10000, 10), "px inside the monitor (when not centered)"),
                it("dropdown.monitor", "Initial monitor", Text, "e.g. DP-1; afterwards the last used monitor is remembered"),
                it("dropdown.animation", "Slide animation", Bool, ""),
                it("dropdown.animation_duration_ms", "Animation time", Int(30, 2000, 10), "ms"),
                it("dropdown.animation_from", "Slide from", Choice(&["top", "bottom", "left", "right"]), ""),
                it("dropdown.hide_on_focus_loss", "Hide on focus loss", Bool, ""),
                it("dropdown.hide_on_mouse_leave", "Hide on mouse leave", Bool, ""),
                it("dropdown.auto_hide_delay_ms", "Auto-hide delay", Int(0, 10000, 100), "ms"),
                it("dropdown.unfocused_toggle", "Hotkey when unfocused", Choice(&["focus", "hide"]), "visible but not focused: focus it or hide it"),
                it("dropdown.grab_focus", "Grab focus on show", Bool, ""),
                it("dropdown.always_on_top", "Always on top", Bool, ""),
                it("dropdown.skip_taskbar", "Hide from taskbar", Bool, ""),
                it("dropdown.all_workspaces", "All workspaces", Bool, ""),
                it("dropdown.start_hidden", "Start hidden", Bool, ""),
                it("dropdown.start_fullscreen", "Start fullscreen", Bool, ""),
                it("dropdown.on_last_tab_exit", "Last tab exits", Choice(&["new_tab_hide", "new_tab", "quit"]), ""),
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
                it("bell", "Bell", Bool, ""),
                it("visual_bell", "Visual bell", Bool, ""),
                it("ime", "Input method (IME)", Bool, ""),
                it("kitty_keyboard", "Kitty keyboard protocol", Bool, ""),
                it("clipboard_read", "Programs reading clipboard", Choice(&["ask", "allow", "deny"]), "OSC 52"),
            ],
        },
        Section {
            name: "Shell & titles",
            items: vec![
                it("shell", "Shell", Text, "empty = $SHELL"),
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

struct App {
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

    fn item(&self) -> Option<&Item> {
        let i = self.sel[self.tab].selected()?;
        self.sections[self.tab].items.get(i)
    }

    fn value_of(&self, item: &Item) -> String {
        match &item.kind {
            Kind::Binding(a) => keys::effective_combos(&self.cfg, a).join(", "),
            Kind::Palette(i) => hex(self.theme.table[*i]),
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
            _ => display(lookup(&self.eff, &item.path)),
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

    fn reset(&mut self, item: &Item) {
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
            _ => {}
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

pub fn run() -> anyhow::Result<()> {
    let mut app = App {
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
                            Kind::Bool => if v == "true" { "[x] on".into() } else { "[ ] off".into() },
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
                    (Some(buf), Some(item)) => format!(" {}: {buf}▏   Enter: save   Esc: cancel", item.label.trim()),
                    (None, Some(item)) => format!(
                        " {}\n Tab/Shift+Tab: section  ↑↓: select  ←→/Space: change  Enter: edit  r: reset  q: quit",
                        item.help
                    ),
                    _ => String::new(),
                };
                f.render_widget(Paragraph::new(help_text), help);
                f.render_widget(Paragraph::new(Span::styled(format!(" {}", app.status), Style::default().fg(Color::Green))), status);
            })?;

            let Event::Key(k) = event::read()? else { continue };
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if let Some(buf) = app.editing.as_mut() {
                match k.code {
                    KeyCode::Enter => {
                        let input = app.editing.take().unwrap_or_default();
                        if let Some(item) = app.item().cloned() {
                            app.set_value(&item, &input);
                        }
                    }
                    KeyCode::Esc => app.editing = None,
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Char('u') if k.modifiers.contains(KeyModifiers::CONTROL) => buf.clear(),
                    KeyCode::Char(c) => buf.push(c),
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
                KeyCode::Enter => {
                    if let Some(item) = app.item().cloned() {
                        match item.kind {
                            Kind::Bool | Kind::Choice(_) => app.adjust(&item, 1),
                            _ => {
                                let v = app.value_of(&item);
                                let v = v.trim_end_matches(" (scheme)").to_string();
                                app.editing = Some(v);
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
