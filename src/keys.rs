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
    ("kitty_mod+alt+t", "set_tab_title"),
    ("alt+1", "goto_tab_1"),
    ("alt+2", "goto_tab_2"),
    ("alt+3", "goto_tab_3"),
    ("alt+4", "goto_tab_4"),
    ("alt+5", "goto_tab_5"),
    ("alt+6", "goto_tab_6"),
    ("alt+7", "goto_tab_7"),
    ("alt+8", "goto_tab_8"),
    ("alt+9", "goto_tab_9"),
    // windows (panes)
    ("kitty_mod+enter", "new_window"),
    ("f5", "hsplit"),
    ("f6", "vsplit"),
    ("f7", "rotate_split"),
    ("kitty_mod+w", "close_window"),
    ("kitty_mod+bracketright", "next_window"),
    ("kitty_mod+bracketleft", "previous_window"),
    ("kitty_mod+f", "move_window_forward"),
    ("kitty_mod+b", "move_window_backward"),
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
];

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

fn mod_param(m: ModifiersState) -> u8 {
    1 + (m.shift_key() as u8) + 2 * (m.alt_key() as u8) + 4 * (m.control_key() as u8) + 8 * (m.super_key() as u8)
}

/// Encode a key press into bytes for the PTY.
pub fn encode(ev: &KeyEvent, mods: ModifiersState, mode: TermMode) -> Option<Vec<u8>> {
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
            NamedKey::Backspace => Some(alt_prefix(if mods.control_key() { vec![0x08] } else { vec![0x7f] })),
            NamedKey::Tab => Some(if mods.shift_key() { b"\x1b[Z".to_vec() } else { alt_prefix(vec![b'\t']) }),
            NamedKey::Escape => Some(alt_prefix(vec![0x1b])),
            NamedKey::ArrowUp => Some(ss3_or_csi("A")),
            NamedKey::ArrowDown => Some(ss3_or_csi("B")),
            NamedKey::ArrowRight => Some(ss3_or_csi("C")),
            NamedKey::ArrowLeft => Some(ss3_or_csi("D")),
            NamedKey::Home => Some(ss3_or_csi("H")),
            NamedKey::End => Some(ss3_or_csi("F")),
            NamedKey::Insert => Some(tilde(2)),
            NamedKey::Delete => Some(tilde(3)),
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
