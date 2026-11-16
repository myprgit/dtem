//! Key bindings (kitty-like defaults) and key → terminal byte encoding.
use crate::config::Config;
use alacritty_terminal::term::TermMode;
use std::collections::HashMap;
use winit::event::KeyEvent;
use winit::keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey};
use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    MoveTabForward,
    MoveTabBackward,
    GotoTab(usize),
    SetTabTitle,
    NewWindow,
    HSplit,
    VSplit,
    CloseWindow,
    NextWindow,
    PreviousWindow,
    MoveWindowForward,
    MoveWindowBackward,
    NthWindow(usize),
    Neighbor(crate::layout::Dir),
    Resize(crate::layout::Dir),
    ToggleZoom,
    RotateSplit,
    Copy,
    Paste,
    PasteSelection,
    ScrollLineUp,
    ScrollLineDown,
    ScrollPageUp,
    ScrollPageDown,
    ScrollHome,
    ScrollEnd,
    ClearScrollback,
    IncreaseFontSize,
    DecreaseFontSize,
    ResetFontSize,
    NewOsWindow,
    ToggleFullscreen,
    SaveSessionFile,
    ReloadConfig,
    ToggleDropdown,
    DropdownNextMonitor,
    DropdownPrevMonitor,
    ToggleTransparency,
    Search,
    Quit,
    QuitWithoutSaving,
    OpenSettings,
    Hints,
    PromptPrev,
    PromptNext,
    CopyLastOutput,
    HistorySearch,
    StashTab,
    StashedTabs,
    None,
}

impl Action {
    pub fn parse(s: &str) -> Option<Action> {
        use crate::layout::Dir::*;
        let s = s.trim();
        let num = |p: &str| s.strip_prefix(p).and_then(|n| n.parse::<usize>().ok());
        if let Some(n) = num("goto_tab_") {
            return Some(Action::GotoTab(n));
        }
        if let Some(n) = num("nth_window_") {
            return Some(Action::NthWindow(n));
        }
        Some(match s {
            "new_tab" => Action::NewTab,
            "close_tab" => Action::CloseTab,
            "next_tab" => Action::NextTab,
            "previous_tab" => Action::PreviousTab,
            "move_tab_forward" => Action::MoveTabForward,
            "move_tab_backward" => Action::MoveTabBackward,
            "set_tab_title" => Action::SetTabTitle,
            "new_window" => Action::NewWindow,
            "hsplit" => Action::HSplit,
            "vsplit" => Action::VSplit,
            "close_window" => Action::CloseWindow,
            "next_window" => Action::NextWindow,
            "previous_window" => Action::PreviousWindow,
            "move_window_forward" => Action::MoveWindowForward,
            "move_window_backward" => Action::MoveWindowBackward,
            "neighboring_window_left" => Action::Neighbor(Left),
            "neighboring_window_right" => Action::Neighbor(Right),
            "neighboring_window_up" => Action::Neighbor(Up),
            "neighboring_window_down" => Action::Neighbor(Down),
            "resize_left" => Action::Resize(Left),
            "resize_right" => Action::Resize(Right),
            "resize_up" => Action::Resize(Up),
            "resize_down" => Action::Resize(Down),
            "toggle_zoom" | "toggle_layout_stack" => Action::ToggleZoom,
            "rotate_split" | "layout_action_rotate" => Action::RotateSplit,
            "copy_to_clipboard" | "copy" => Action::Copy,
            "paste_from_clipboard" | "paste" => Action::Paste,
            "paste_from_selection" => Action::PasteSelection,
            "scroll_line_up" => Action::ScrollLineUp,
            "scroll_line_down" => Action::ScrollLineDown,
            "scroll_page_up" => Action::ScrollPageUp,
            "scroll_page_down" => Action::ScrollPageDown,
            "scroll_home" => Action::ScrollHome,
            "scroll_end" => Action::ScrollEnd,
            "clear_scrollback" => Action::ClearScrollback,
            "increase_font_size" => Action::IncreaseFontSize,
            "decrease_font_size" => Action::DecreaseFontSize,
            "reset_font_size" => Action::ResetFontSize,
            "new_os_window" => Action::NewOsWindow,
            "toggle_fullscreen" => Action::ToggleFullscreen,
            "save_session" => Action::SaveSessionFile,
            "reload_config" => Action::ReloadConfig,
            "toggle_dropdown" => Action::ToggleDropdown,
            "dropdown_next_monitor" => Action::DropdownNextMonitor,
            "dropdown_prev_monitor" => Action::DropdownPrevMonitor,
            "toggle_transparency" => Action::ToggleTransparency,
            "search" | "toggle_searchbar" => Action::Search,
            "quit" => Action::Quit,
            "quit_without_saving" => Action::QuitWithoutSaving,
            "open_settings" | "edit_config_file" => Action::OpenSettings,
            "hints" | "open_url_hints" => Action::Hints,
            "scroll_to_prompt_up" | "previous_prompt" => Action::PromptPrev,
            "scroll_to_prompt_down" | "next_prompt" => Action::PromptNext,
            "copy_last_command_output" | "copy_last_output" => Action::CopyLastOutput,
            "history_search" => Action::HistorySearch,
            "stash_tab" => Action::StashTab,
            "stashed_tabs" | "stash_browser" => Action::StashedTabs,
            "none" | "no_op" => Action::None,
            _ => return None,
        })
    }
}

const DEFAULTS: &[(&str, &str)] = &[
    // tabs
    ("kitty_mod+t", "new_tab"),
    ("kitty_mod+q", "close_tab"),
    ("kitty_mod+right", "next_tab"),
    ("ctrl+tab", "next_tab"),
    ("kitty_mod+left", "previous_tab"),
    ("ctrl+shift+tab", "previous_tab"),
    ("kitty_mod+period", "move_tab_forward"),
    ("kitty_mod+comma", "move_tab_backward"),
    ("kitty_mod+r", "set_tab_title"),
    ("kitty_mod+alt+t", "set_tab_title"),
    ("kitty_mod+d", "stash_tab"),
    ("kitty_mod+o", "stashed_tabs"),
    ("alt+1", "goto_tab_1"),
    ("alt+2", "goto_tab_2"),
    ("alt+3", "goto_tab_3"),
    ("alt+4", "goto_tab_4"),
    ("alt+5", "goto_tab_5"),
    ("alt+6", "goto_tab_6"),
    ("alt+7", "goto_tab_7"),
    ("alt+8", "goto_tab_8"),
    ("alt+9", "goto_tab_9"),
    ("alt+0", "goto_tab_10"),
    // windows (panes)
    ("kitty_mod+enter", "new_window"),
    ("f5", "hsplit"),
    ("f6", "vsplit"),
    ("f7", "rotate_split"),
    ("kitty_mod+w", "close_window"),
    ("kitty_mod+bracketright", "next_window"),
    ("kitty_mod+bracketleft", "previous_window"),
    ("kitty_mod+alt+f", "move_window_forward"),
    ("kitty_mod+alt+b", "move_window_backward"),
    ("kitty_mod+f", "search"),
    ("kitty_mod+1", "nth_window_1"),
    ("kitty_mod+2", "nth_window_2"),
    ("kitty_mod+3", "nth_window_3"),
    ("kitty_mod+4", "nth_window_4"),
    ("kitty_mod+5", "nth_window_5"),
    ("kitty_mod+6", "nth_window_6"),
    ("kitty_mod+7", "nth_window_7"),
    ("kitty_mod+8", "nth_window_8"),
    ("kitty_mod+9", "nth_window_9"),
    ("kitty_mod+alt+h", "neighboring_window_left"),
    ("kitty_mod+alt+l", "neighboring_window_right"),
    ("kitty_mod+alt+k", "neighboring_window_up"),
    ("kitty_mod+alt+j", "neighboring_window_down"),
    ("kitty_mod+alt+left", "resize_left"),
    ("kitty_mod+alt+right", "resize_right"),
    ("kitty_mod+alt+up", "resize_up"),
    ("kitty_mod+alt+down", "resize_down"),
    ("kitty_mod+l", "toggle_zoom"),
    // clipboard
    ("kitty_mod+c", "copy_to_clipboard"),
    ("kitty_mod+v", "paste_from_clipboard"),
    ("shift+insert", "paste_from_selection"),
    ("kitty_mod+s", "paste_from_selection"),
    // scrolling
    ("kitty_mod+up", "scroll_line_up"),
    ("kitty_mod+k", "scroll_line_up"),
    ("kitty_mod+down", "scroll_line_down"),
    ("kitty_mod+j", "scroll_line_down"),
    ("kitty_mod+page_up", "scroll_page_up"),
    ("shift+page_up", "scroll_page_up"),
    ("kitty_mod+page_down", "scroll_page_down"),
    ("shift+page_down", "scroll_page_down"),
    ("kitty_mod+home", "scroll_home"),
    ("kitty_mod+end", "scroll_end"),
    ("kitty_mod+delete", "clear_scrollback"),
    // misc
    ("kitty_mod+equal", "increase_font_size"),
    ("kitty_mod+plus", "increase_font_size"),
    ("kitty_mod+minus", "decrease_font_size"),
    ("kitty_mod+backspace", "reset_font_size"),
    ("kitty_mod+n", "new_os_window"),
    ("kitty_mod+f11", "toggle_fullscreen"),
    ("kitty_mod+alt+s", "save_session"),
    ("kitty_mod+f5", "reload_config"),
    ("kitty_mod+alt+m", "dropdown_next_monitor"),
    ("kitty_mod+f2", "open_settings"),
    ("ctrl+comma", "open_settings"),
    ("kitty_mod+alt+q", "quit"),
    ("kitty_mod+e", "hints"),
    ("kitty_mod+z", "previous_prompt"),
    ("kitty_mod+x", "next_prompt"),
    ("kitty_mod+g", "copy_last_command_output"),
    ("ctrl+r", "history_search"),
];

/// Every bindable action with a short description (used by the settings page).
pub const ACTIONS: &[(&str, &str)] = &[
    ("new_tab", "New tab"),
    ("close_tab", "Close tab"),
    ("next_tab", "Next tab"),
    ("previous_tab", "Previous tab"),
    ("move_tab_forward", "Move tab right"),
    ("move_tab_backward", "Move tab left"),
    ("set_tab_title", "Rename tab"),
    ("stash_tab", "Stash tab"),
    ("stashed_tabs", "Stashed tabs"),
    ("goto_tab_1", "Go to tab 1"),
    ("goto_tab_2", "Go to tab 2"),
    ("goto_tab_3", "Go to tab 3"),
    ("goto_tab_4", "Go to tab 4"),
    ("goto_tab_5", "Go to tab 5"),
    ("goto_tab_6", "Go to tab 6"),
    ("goto_tab_7", "Go to tab 7"),
    ("goto_tab_8", "Go to tab 8"),
    ("goto_tab_9", "Go to tab 9"),
    ("goto_tab_10", "Go to tab 10"),
    ("new_window", "Split (longer side)"),
    ("hsplit", "Split horizontally"),
    ("vsplit", "Split vertically"),
    ("rotate_split", "Rotate split"),
    ("close_window", "Close pane"),
    ("next_window", "Next pane"),
    ("previous_window", "Previous pane"),
    ("move_window_forward", "Move pane forward"),
    ("move_window_backward", "Move pane backward"),
    ("neighboring_window_left", "Focus pane left"),
    ("neighboring_window_right", "Focus pane right"),
    ("neighboring_window_up", "Focus pane up"),
    ("neighboring_window_down", "Focus pane down"),
    ("resize_left", "Move divider left"),
    ("resize_right", "Move divider right"),
    ("resize_up", "Move divider up"),
    ("resize_down", "Move divider down"),
    ("toggle_zoom", "Zoom pane"),
    ("copy_to_clipboard", "Copy"),
    ("paste_from_clipboard", "Paste"),
    ("paste_from_selection", "Paste primary selection"),
    ("search", "Search in scrollback"),
    ("hints", "Select link/path/hash with keyboard"),
    ("previous_prompt", "Jump to previous prompt"),
    ("next_prompt", "Jump to next prompt"),
    ("copy_last_command_output", "Copy last command output"),
    ("history_search", "History search (at a shell prompt)"),
    ("scroll_line_up", "Scroll line up"),
    ("scroll_line_down", "Scroll line down"),
    ("scroll_page_up", "Scroll page up"),
    ("scroll_page_down", "Scroll page down"),
    ("scroll_home", "Scroll to top"),
    ("scroll_end", "Scroll to bottom"),
    ("clear_scrollback", "Clear scrollback"),
    ("increase_font_size", "Font bigger"),
    ("decrease_font_size", "Font smaller"),
    ("reset_font_size", "Font reset"),
    ("toggle_transparency", "Transparency on/off"),
    ("toggle_fullscreen", "Fullscreen on/off"),
    ("new_os_window", "New OS window"),
    ("save_session", "Export layout file"),
    ("open_settings", "Settings page"),
    ("reload_config", "Reload config"),
    ("dropdown_next_monitor", "Dropdown: next monitor"),
    ("dropdown_prev_monitor", "Dropdown: previous monitor"),
    ("quit", "Quit (session kept)"),
    ("quit_without_saving", "Quit and forget session"),
];

/// Default combos for an action (raw, with `kitty_mod`).
pub fn default_combos(action: &str) -> Vec<&'static str> {
    DEFAULTS.iter().filter(|(_, a)| *a == action).map(|(c, _)| *c).collect()
}

pub fn normalize_combo(combo: &str, kitty_mod: &str) -> Option<String> {
    let (m, k) = parse_combo(combo, kitty_mod)?;
    let mut v = Vec::new();
    if m.ctrl {
        v.push("ctrl".to_string());
    }
    if m.shift {
        v.push("shift".to_string());
    }
    if m.alt {
        v.push("alt".to_string());
    }
    if m.sup {
        v.push("super".to_string());
    }
    v.push(k);
    Some(v.join("+"))
}

/// Effective combos of an action after config overrides.
pub fn effective_combos(cfg: &Config, action: &str) -> Vec<String> {
    let mut out: Vec<String> =
        default_combos(action).iter().filter_map(|c| normalize_combo(c, &cfg.kitty_mod)).collect();
    for (combo, act) in &cfg.keybindings {
        let Some(n) = normalize_combo(combo, &cfg.kitty_mod) else { continue };
        if act.trim() == action {
            if !out.contains(&n) {
                out.push(n);
            }
        } else {
            out.retain(|c| c != &n);
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Mods {
    ctrl: bool,
    shift: bool,
    alt: bool,
    sup: bool,
}

impl Mods {
    pub fn from_state(m: ModifiersState) -> Mods {
        Mods { ctrl: m.control_key(), shift: m.shift_key(), alt: m.alt_key(), sup: m.super_key() }
    }
}

fn canonical_key(k: &str) -> String {
    let k = k.to_lowercase();
    match k.as_str() {
        "=" => "equal",
        "+" => "plus",
        "-" => "minus",
        "," => "comma",
        "." => "period",
        "[" => "bracketleft",
        "]" => "bracketright",
        "\\" => "backslash",
        "/" => "slash",
        "`" | "grave_accent" => "grave",
        ";" => "semicolon",
        "'" => "apostrophe",
        "return" => "enter",
        "esc" => "escape",
        "pageup" | "pgup" => "page_up",
        "pagedown" | "pgdn" => "page_down",
        "del" => "delete",
        "ins" => "insert",
        " " => "space",
        _ => return k,
    }
    .to_string()
}

fn parse_combo(s: &str, kitty_mod: &str) -> Option<(Mods, String)> {
    let s = s.replace("kitty_mod", kitty_mod);
    let mut m = Mods::default();
    let parts: Vec<&str> = s.split('+').collect();
    // handle "ctrl+shift++"
    let (mods, key) = if s.ends_with("++") {
        (&parts[..parts.len() - 2], "+".to_string())
    } else {
        (&parts[..parts.len() - 1], parts.last()?.to_string())
    };
    for p in mods {
        match p.trim().to_lowercase().as_str() {
            "ctrl" | "control" => m.ctrl = true,
            "shift" => m.shift = true,
            "alt" | "opt" | "option" => m.alt = true,
            "super" | "cmd" | "logo" | "meta" => m.sup = true,
            "" => {}
            _ => return None,
        }
    }
    Some((m, canonical_key(key.trim())))
}

pub struct Bindings {
    map: HashMap<(Mods, String), Action>,
}

impl Bindings {
    pub fn new(cfg: &Config) -> Bindings {
        let mut map = HashMap::new();
        for (combo, act) in DEFAULTS {
            if let (Some(k), Some(a)) = (parse_combo(combo, &cfg.kitty_mod), Action::parse(act)) {
                map.insert(k, a);
            }
        }
        for (combo, act) in &cfg.keybindings {
            match (parse_combo(combo, &cfg.kitty_mod), Action::parse(act)) {
                (Some(k), Some(Action::None)) => {
                    map.remove(&k);
                }
                (Some(k), Some(a)) => {
                    map.insert(k, a);
                }
                _ => log::warn!("invalid keybinding: {combo} = {act}"),
            }
        }
        Bindings { map }
    }

    pub fn lookup(&self, ev: &KeyEvent, mods: ModifiersState) -> Option<Action> {
        let m = Mods::from_state(mods);
        let names = key_names(ev);
        names.into_iter().find_map(|n| self.map.get(&(m, n)).cloned())
    }
}

fn named_name(n: &NamedKey) -> Option<&'static str> {
    Some(match n {
        NamedKey::Enter => "enter",
        NamedKey::Tab => "tab",
        NamedKey::Space => "space",
        NamedKey::Backspace => "backspace",
        NamedKey::Escape => "escape",
        NamedKey::Delete => "delete",
        NamedKey::Insert => "insert",
        NamedKey::Home => "home",
        NamedKey::End => "end",
        NamedKey::PageUp => "page_up",
        NamedKey::PageDown => "page_down",
        NamedKey::ArrowLeft => "left",
        NamedKey::ArrowRight => "right",
        NamedKey::ArrowUp => "up",
        NamedKey::ArrowDown => "down",
        NamedKey::F1 => "f1",
        NamedKey::F2 => "f2",
        NamedKey::F3 => "f3",
        NamedKey::F4 => "f4",
        NamedKey::F5 => "f5",
        NamedKey::F6 => "f6",
        NamedKey::F7 => "f7",
        NamedKey::F8 => "f8",
        NamedKey::F9 => "f9",
        NamedKey::F10 => "f10",
        NamedKey::F11 => "f11",
        NamedKey::F12 => "f12",
        _ => return None,
    })
}

fn physical_name(k: KeyCode) -> Option<String> {
    let s = match k {
        KeyCode::BracketLeft => "bracketleft",
        KeyCode::BracketRight => "bracketright",
        KeyCode::Minus => "minus",
        KeyCode::Equal => "equal",
        KeyCode::Comma => "comma",
        KeyCode::Period => "period",
        KeyCode::Backslash => "backslash",
        KeyCode::Slash => "slash",
        KeyCode::Backquote => "grave",
        KeyCode::Semicolon => "semicolon",
        KeyCode::Quote => "apostrophe",
        _ => {
            let d = format!("{k:?}");
            if let Some(c) = d.strip_prefix("Key") {
                return Some(c.to_lowercase());
            }
            if let Some(c) = d.strip_prefix("Digit") {
                return Some(c.to_string());
            }
            return None;
        }
    };
    Some(s.to_string())
}

/// Candidate names for the pressed key: layout-aware first, then US physical position
/// (so bindings work on non-US layouts such as Turkish Q/F).
fn key_names(ev: &KeyEvent) -> Vec<String> {
    let mut v = Vec::new();
    match ev.key_without_modifiers() {
        Key::Named(n) => {
            if let Some(s) = named_name(&n) {
                v.push(s.to_string());
            }
        }
        Key::Character(s) => v.push(canonical_key(s.as_str())),
        _ => {}
    }
    if let Key::Character(s) = &ev.logical_key {
        let c = canonical_key(s.as_str());
        if !v.contains(&c) {
            v.push(c);
        }
    }
    if let PhysicalKey::Code(code) = ev.physical_key {
        if let Some(p) = physical_name(code) {
            if !v.contains(&p) {
                v.push(p);
            }
        }
    }
    v
}

/// What Backspace/Delete send (tilda's backspace_key/delete_key).
#[derive(Clone, Copy, Debug)]
pub struct KeyCfg {
    pub backspace: Erase,
    pub delete: Erase,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Erase {
    Del,
    Bs,
    Escape,
}

impl KeyCfg {
    pub fn from_config(cfg: &Config) -> KeyCfg {
        let e = |s: &str, auto: Erase| match s {
            "del" => Erase::Del,
            "bs" => Erase::Bs,
            "escape" => Erase::Escape,
            _ => auto,
        };
        KeyCfg { backspace: e(&cfg.backspace_sends, Erase::Del), delete: e(&cfg.delete_sends, Erase::Escape) }
    }
}

impl Default for KeyCfg {
    fn default() -> Self {
        KeyCfg { backspace: Erase::Del, delete: Erase::Escape }
    }
}

fn mod_param(m: ModifiersState) -> u8 {
    1 + (m.shift_key() as u8) + 2 * (m.alt_key() as u8) + 4 * (m.control_key() as u8) + 8 * (m.super_key() as u8)
}

/// Encode a key press into bytes for the PTY.
pub fn encode(ev: &KeyEvent, mods: ModifiersState, mode: TermMode, kc: &KeyCfg) -> Option<Vec<u8>> {
    let mp = mod_param(mods);
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let csi = |final_: &str, num: Option<u8>| -> Vec<u8> {
        if mp > 1 {
            format!("\x1b[{};{}{}", num.unwrap_or(1), mp, final_).into_bytes()
        } else if let Some(n) = num {
            format!("\x1b[{n}{final_}").into_bytes()
        } else {
            format!("\x1b[{final_}").into_bytes()
        }
    };
    let ss3_or_csi = |c: &str| -> Vec<u8> {
        if mp > 1 {
            format!("\x1b[1;{mp}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    let tilde = |n: u8| -> Vec<u8> {
        if mp > 1 {
            format!("\x1b[{n};{mp}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    let alt_prefix = |mut b: Vec<u8>| -> Vec<u8> {
        if mods.alt_key() {
            b.insert(0, 0x1b);
        }
        b
    };

    if let Key::Named(n) = &ev.logical_key {
        let out = match n {
            NamedKey::Enter => Some(alt_prefix(vec![b'\r'])),
            NamedKey::Backspace => Some(alt_prefix(match (kc.backspace, mods.control_key()) {
                (Erase::Bs, false) | (Erase::Del, true) => vec![0x08],
                (Erase::Escape, _) => b"\x1b[3~".to_vec(),
                _ => vec![0x7f],
            })),
            NamedKey::Tab => Some(if mods.shift_key() { b"\x1b[Z".to_vec() } else { alt_prefix(vec![b'\t']) }),
            NamedKey::Escape => Some(alt_prefix(vec![0x1b])),
            NamedKey::ArrowUp => Some(ss3_or_csi("A")),
            NamedKey::ArrowDown => Some(ss3_or_csi("B")),
            NamedKey::ArrowRight => Some(ss3_or_csi("C")),
            NamedKey::ArrowLeft => Some(ss3_or_csi("D")),
            NamedKey::Home => Some(ss3_or_csi("H")),
            NamedKey::End => Some(ss3_or_csi("F")),
            NamedKey::Insert => Some(tilde(2)),
            NamedKey::Delete => Some(match kc.delete {
                Erase::Del if mp == 1 => vec![0x7f],
                Erase::Bs if mp == 1 => vec![0x08],
                _ => tilde(3),
            }),
            NamedKey::PageUp => Some(tilde(5)),
            NamedKey::PageDown => Some(tilde(6)),
            NamedKey::F1 => Some(if mp > 1 { csi("P", None) } else { b"\x1bOP".to_vec() }),
            NamedKey::F2 => Some(if mp > 1 { csi("Q", None) } else { b"\x1bOQ".to_vec() }),
            NamedKey::F3 => Some(if mp > 1 { csi("R", None) } else { b"\x1bOR".to_vec() }),
            NamedKey::F4 => Some(if mp > 1 { csi("S", None) } else { b"\x1bOS".to_vec() }),
            NamedKey::F5 => Some(tilde(15)),
            NamedKey::F6 => Some(tilde(17)),
            NamedKey::F7 => Some(tilde(18)),
            NamedKey::F8 => Some(tilde(19)),
            NamedKey::F9 => Some(tilde(20)),
            NamedKey::F10 => Some(tilde(21)),
            NamedKey::F11 => Some(tilde(23)),
            NamedKey::F12 => Some(tilde(24)),
            NamedKey::Space => Some(alt_prefix(if mods.control_key() { vec![0] } else { vec![b' '] })),
            _ => None,
        };
        if out.is_some() {
            return out;
        }
    }

    if mods.control_key() {
        let base = match ev.key_without_modifiers() {
            Key::Character(s) => s.chars().next(),
            _ => None,
        };
        if let Some(c) = base {
            let c = c.to_ascii_lowercase();
            let b = match c {
                'a'..='z' => Some(c as u8 - b'a' + 1),
                '@' | ' ' | '2' => Some(0),
                '[' | '3' => Some(0x1b),
                '\\' | '4' => Some(0x1c),
                ']' | '5' => Some(0x1d),
                '^' | '6' => Some(0x1e),
                '_' | '-' | '/' | '7' => Some(0x1f),
                '8' | '?' => Some(0x7f),
                _ => None,
            };
            if let Some(b) = b {
                return Some(alt_prefix(vec![b]));
            }
        }
    }

    let text = ev.text.as_ref()?;
    if text.is_empty() {
        return None;
    }
    Some(alt_prefix(text.as_bytes().to_vec()))
}

// ---------------------------------------------------------------- kitty keyboard protocol
// https://sw.kovidgoyal.net/kitty/keyboard-protocol/

enum Fk {
    /// CSI 1;mods X  (arrows, home/end, F1/F2/F4)
    Letter(char),
    /// CSI n;mods ~
    Tilde(u32),
    /// CSI code;mods u
    U(u32),
}

fn functional(ev: &KeyEvent) -> Option<Fk> {
    use winit::keyboard::KeyLocation;
    let right = ev.location == KeyLocation::Right;
    let Key::Named(n) = &ev.logical_key else { return None };
    Some(match n {
        NamedKey::Escape => Fk::U(27),
        NamedKey::Enter => Fk::U(13),
        NamedKey::Tab => Fk::U(9),
        NamedKey::Backspace => Fk::U(127),
        NamedKey::Insert => Fk::Tilde(2),
        NamedKey::Delete => Fk::Tilde(3),
        NamedKey::PageUp => Fk::Tilde(5),
        NamedKey::PageDown => Fk::Tilde(6),
        NamedKey::ArrowUp => Fk::Letter('A'),
        NamedKey::ArrowDown => Fk::Letter('B'),
        NamedKey::ArrowRight => Fk::Letter('C'),
        NamedKey::ArrowLeft => Fk::Letter('D'),
        NamedKey::Home => Fk::Letter('H'),
        NamedKey::End => Fk::Letter('F'),
        NamedKey::F1 => Fk::Letter('P'),
        NamedKey::F2 => Fk::Letter('Q'),
        NamedKey::F3 => Fk::Tilde(13),
        NamedKey::F4 => Fk::Letter('S'),
        NamedKey::F5 => Fk::Tilde(15),
        NamedKey::F6 => Fk::Tilde(17),
        NamedKey::F7 => Fk::Tilde(18),
        NamedKey::F8 => Fk::Tilde(19),
        NamedKey::F9 => Fk::Tilde(20),
        NamedKey::F10 => Fk::Tilde(21),
        NamedKey::F11 => Fk::Tilde(23),
        NamedKey::F12 => Fk::Tilde(24),
        NamedKey::CapsLock => Fk::U(57358),
        NamedKey::ScrollLock => Fk::U(57359),
        NamedKey::NumLock => Fk::U(57360),
        NamedKey::PrintScreen => Fk::U(57361),
        NamedKey::Pause => Fk::U(57362),
        NamedKey::ContextMenu => Fk::U(57363),
        NamedKey::Shift => Fk::U(if right { 57447 } else { 57441 }),
        NamedKey::Control => Fk::U(if right { 57448 } else { 57442 }),
        NamedKey::Alt => Fk::U(if right { 57449 } else { 57443 }),
        NamedKey::Super => Fk::U(if right { 57450 } else { 57444 }),
        _ => return None,
    })
}

fn is_modifier_key(ev: &KeyEvent) -> bool {
    matches!(
        ev.logical_key,
        Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super)
    )
}

/// Encode a key event using the kitty keyboard protocol.
/// `event`: 1 press, 2 repeat, 3 release.
pub fn encode_kitty(ev: &KeyEvent, mods: ModifiersState, mode: TermMode, event: u8, kc: &KeyCfg) -> Option<Vec<u8>> {
    let all = mode.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let types = mode.contains(TermMode::REPORT_EVENT_TYPES);
    let alt_keys = mode.contains(TermMode::REPORT_ALTERNATE_KEYS);
    let assoc = mode.contains(TermMode::REPORT_ASSOCIATED_TEXT);
    let event = if types { event } else if event == 3 { return None } else { 1 };
    let m = mod_param(mods) as u32;
    let ev_suffix = if event > 1 { format!(":{event}") } else { String::new() };
    let mods_part = |force: bool| -> String {
        if m > 1 || event > 1 || force {
            format!(";{m}{ev_suffix}")
        } else {
            String::new()
        }
    };

    if let Some(fk) = functional(ev) {
        if is_modifier_key(ev) && !all {
            return None;
        }
        let plain = m == 1;
        match fk {
            Fk::U(code @ (13 | 9 | 127)) if !all && plain => {
                if event == 3 {
                    return None;
                }
                return Some(match code {
                    13 => b"\r".to_vec(),
                    9 => b"\t".to_vec(),
                    _ => vec![0x7f],
                });
            }
            Fk::U(code) => return Some(format!("\x1b[{code}{}u", mods_part(false)).into_bytes()),
            Fk::Letter(c) => {
                if !all && event == 1 {
                    return encode(ev, mods, mode, kc);
                }
                return Some(format!("\x1b[1{}{c}", mods_part(true)).into_bytes());
            }
            Fk::Tilde(n) => {
                if !all && event == 1 {
                    return encode(ev, mods, mode, kc);
                }
                return Some(format!("\x1b[{n}{}~", mods_part(false)).into_bytes());
            }
        }
    }

    // Text-producing key
    let base = match ev.key_without_modifiers() {
        Key::Character(s) => s.chars().next()?,
        Key::Named(NamedKey::Space) => ' ',
        _ => return None,
    };
    let code = base.to_lowercase().next().unwrap_or(base) as u32;
    let text = ev.text.as_ref().map(|t| t.to_string()).filter(|t| t.chars().all(|c| !c.is_control()));
    let has_cm = mods.control_key() || mods.alt_key() || mods.super_key();
    if !all && !has_cm {
        return match event {
            3 => None,
            _ => text.map(|t| t.into_bytes()),
        };
    }
    let mut key = code.to_string();
    if alt_keys && mods.shift_key() {
        if let Some(shifted) = text.as_ref().and_then(|t| t.chars().next()) {
            if shifted as u32 != code {
                key = format!("{code}:{}", shifted as u32);
            }
        }
    }
    let text_part = match (&text, assoc && all && event != 3) {
        (Some(t), true) => format!(";{}", t.chars().map(|c| (c as u32).to_string()).collect::<Vec<_>>().join(":")),
        _ => String::new(),
    };
    let mp = if text_part.is_empty() { mods_part(false) } else { mods_part(true) };
    Some(format!("\x1b[{key}{mp}{text_part}u").into_bytes())
}
