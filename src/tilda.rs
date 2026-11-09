//! Import a tilda config (~/.config/tilda/config_N) into dtem.toml.
use crate::confedit;
use std::collections::HashMap;
use std::path::Path;

fn parse(text: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            let v = v.strip_prefix('"').and_then(|x| x.strip_suffix('"')).unwrap_or(v);
            m.insert(k.trim().to_string(), v.to_string());
        }
    }
    m
}

/// GTK accelerator ("<Shift><Control>t") → dtem combo ("ctrl+shift+t").
pub fn gtk_accel(s: &str) -> Option<String> {
    let mut mods = Vec::new();
    let mut rest = s.trim();
    while let Some(r) = rest.strip_prefix('<') {
        let (m, after) = r.split_once('>')?;
        let m = match m.to_lowercase().as_str() {
            "control" | "ctrl" | "primary" => "ctrl",
            "shift" => "shift",
            "alt" | "mod1" => "alt",
            "super" | "meta" | "hyper" | "mod4" => "super",
            _ => return None,
        };
        if !mods.contains(&m) {
            mods.push(m);
        }
        rest = after;
    }
    if rest.is_empty() {
        return None;
    }
    let key = match rest {
        "Page_Up" | "Prior" => "page_up".to_string(),
        "Page_Down" | "Next" => "page_down".to_string(),
        "Return" | "KP_Enter" => "enter".to_string(),
        "BackSpace" => "backspace".to_string(),
        "grave" => "grave".to_string(),
        k => k.to_lowercase(),
    };
    let order = ["ctrl", "shift", "alt", "super"];
    let mut v: Vec<String> = order.iter().filter(|o| mods.contains(o)).map(|s| s.to_string()).collect();
    v.push(key);
    Some(v.join("+"))
}

fn hex16(r: u32, g: u32, b: u32) -> String {
    format!("#{:02x}{:02x}{:02x}", r >> 8, g >> 8, b >> 8)
}

/// Monitor geometry by name via xrandr (best effort): (x, y)
fn monitor_origin(name: &str) -> Option<(i32, i32)> {
    let out = std::process::Command::new("xrandr").arg("--listmonitors").output().ok()?;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if line.trim_end().ends_with(name) {
            // " 0: +*DP-1 2560/597x1440/336+2944+0  DP-1"
            let geo = line.split_whitespace().nth(2)?;
            let mut it = geo.split('+').skip(1);
            return Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?));
        }
    }
    None
}

pub struct ImportReport {
    pub applied: usize,
    pub ignored: Vec<String>,
    pub backup: Option<std::path::PathBuf>,
}

pub fn import(path: &Path) -> anyhow::Result<ImportReport> {
    let text = std::fs::read_to_string(path)?;
    let t = parse(&text);
    let mut doc = confedit::load_doc();
    let mut applied = 0usize;
    let get = |k: &str| t.get(k).map(|s| s.as_str());
    let b = |k: &str| get(k).map(|v| v == "true");
    let i = |k: &str| get(k).and_then(|v| v.parse::<i64>().ok());
    macro_rules! set {
        ($path:expr, $val:expr) => {{
            confedit::set(&mut doc, $path, $val);
            applied += 1;
        }};
    }

    // font: Pango "Family [Style] Size"
    if let Some(f) = get("font") {
        let mut words: Vec<&str> = f.split_whitespace().collect();
        if let Some(size) = words.last().and_then(|w| w.parse::<f64>().ok()) {
            words.pop();
            set!("font_size", size);
        }
        let styles = ["bold", "italic", "oblique", "regular", "medium", "light", "semi-bold", "book"];
        while words.len() > 1 && styles.contains(&words.last().unwrap().to_lowercase().as_str()) {
            words.pop();
        }
        if !words.is_empty() {
            let fam = words.join(" ");
            set!("font_family", if fam.eq_ignore_ascii_case("monospace") { "monospace".to_string() } else { fam });
        }
    }

    // dropdown hotkey
    if let Some(k) = get("key").and_then(gtk_accel) {
        set!("dropdown.hotkey", k);
    }

    // key bindings
    let mut binds: Vec<(String, String)> = Vec::new();
    let map = [
        ("addtab_key", "new_tab"),
        ("fullscreen_key", "toggle_fullscreen"),
        ("toggle_transparency_key", "toggle_transparency"),
        ("toggle_searchbar_key", "search"),
        ("closetab_key", "close_tab"),
        ("nexttab_key", "next_tab"),
        ("prevtab_key", "previous_tab"),
        ("movetableft_key", "move_tab_backward"),
        ("movetabright_key", "move_tab_forward"),
        ("copy_key", "copy_to_clipboard"),
        ("paste_key", "paste_from_clipboard"),
        ("quit_key", "quit"),
        ("increase_font_size_key", "increase_font_size"),
        ("decrease_font_size_key", "decrease_font_size"),
        ("normalize_font_size_key", "reset_font_size"),
    ];
    for (k, action) in map {
        if let Some(c) = get(k).and_then(gtk_accel) {
            binds.push((c, action.to_string()));
        }
    }
    for n in 1..=10 {
        if let Some(c) = get(&format!("gototab_{n}_key")).and_then(gtk_accel) {
            binds.push((c, format!("goto_tab_{n}")));
        }
    }
    for (combo, action) in binds {
        set!(&format!("keybindings.{}", combo.replace('.', "period")), action);
    }

    // titles
    if let Some(s) = get("title") {
        set!("title", s.to_string());
    }
    if let Some(v) = i("d_set_title") {
        set!("dynamic_title", ["none", "after", "before", "replace"].get(v as usize).copied().unwrap_or("replace"));
    }
    match i("title_behaviour") {
        Some(0) => set!("title_max_length", 0i64),
        Some(v) => {
            set!("title_truncate", if v == 2 { "start" } else { "end" });
            if let Some(n) = i("title_max_length") {
                set!("title_max_length", n);
            }
        }
        None => {}
    }

    // tabs
    if let Some(v) = i("tab_pos") {
        set!("tab_bar_position", match v {
            1 => "bottom",
            4 => "hidden",
            _ => "top",
        });
    }
    if let Some(v) = b("expand_tabs") {
        set!("tab_bar_expand", v);
    }
    if let Some(v) = b("show_single_tab") {
        set!("tab_bar_min_tabs", if v { 1i64 } else { 2 });
    }
    if let Some(v) = b("insert_tab_after_current") {
        set!("new_tab_after_current", v);
    }

    // scrolling / selection
    if let Some(v) = i("lines") {
        set!("scrollback_lines", v);
        set!("persist_scrollback_lines", v);
    }
    if let Some(v) = b("scroll_history_infinite") {
        set!("scrollback_infinite", v);
    }
    if let Some(v) = b("scroll_on_output") {
        set!("scroll_on_output", v);
    }
    if let Some(v) = b("scroll_on_key") {
        set!("scroll_on_key", v);
    }
    if let Some(s) = get("word_chars") {
        set!("word_chars", s.to_string());
    }
    let scrollbar_on = b("scrollbar").unwrap_or(true);
    if let Some(v) = i("scrollbar_pos") {
        set!("scrollbar", match (scrollbar_on, v) {
            (true, 0) => "right",
            (true, 1) => "left",
            _ => "none",
        });
    }

    // keys
    let erase = |v: i64| ["del", "escape", "bs", "auto"].get(v as usize).copied().unwrap_or("auto");
    if let Some(v) = i("backspace_key") {
        set!("backspace_sends", erase(v));
    }
    if let Some(v) = i("delete_key") {
        set!("delete_sends", erase(v));
    }

    // command / shell
    let run = b("run_command").unwrap_or(false);
    set!("command", if run { get("command").unwrap_or("").to_string() } else { String::new() });
    if let Some(v) = i("command_exit") {
        set!("command_exit", ["shell", "restart", "close"].get(v as usize).copied().unwrap_or("shell"));
    }
    if let Some(v) = b("command_login_shell") {
        set!("login_shell", v);
    }
    if let Some(s) = get("working_dir") {
        set!("working_dir", s.to_string());
    }
    if let Some(v) = b("inherit_working_dir") {
        set!("inherit_working_dir", v);
    }

    // look
    if let Some(v) = i("cursor_shape") {
        set!("cursor_shape", ["block", "beam", "underline"].get(v as usize).copied().unwrap_or("block"));
    }
    if let Some(v) = b("blinks") {
        set!("cursor_blink", v);
    }
    if let Some(v) = b("bold_is_bright") {
        set!("bold_is_bright", v);
    }
    if let Some(v) = b("bell") {
        set!("bell", v);
    }
    if let Some(v) = b("enable_transparency") {
        set!("transparency", v);
    }
    if let Some(v) = i("back_alpha") {
        set!("background_opacity", ((v as f64 / 65535.0) * 100.0).round() / 100.0);
    }
    let rgb = |p: &str| -> Option<String> {
        Some(hex16(
            i(&format!("{p}_red"))? as u32,
            i(&format!("{p}_green"))? as u32,
            i(&format!("{p}_blue"))? as u32,
        ))
    };
    if let Some(c) = rgb("back") {
        set!("colors.background", c);
    }
    if let Some(c) = rgb("text") {
        set!("colors.foreground", c);
    }
    if let Some(c) = rgb("cursor") {
        set!("colors.cursor", c);
    }
    if let Some(pal) = get("palette") {
        let nums: Vec<u32> =
            pal.trim_matches(|c| c == '{' || c == '}').split(',').filter_map(|s| s.trim().parse().ok()).collect();
        if nums.len() >= 48 {
            let colors: Vec<String> = nums.chunks(3).take(16).map(|c| hex16(c[0], c[1], c[2])).collect();
            set!("colors.palette", confedit::string_array(&colors));
        }
    }
    let scheme_name = match i("palette_scheme") {
        Some(2) => "linux",
        Some(3) => "xterm",
        Some(4) => "rxvt",
        Some(5) => "zenburn",
        Some(6) => "solarized-light",
        Some(7) => "solarized-dark",
        Some(8) => "snazzy",
        _ => "tango",
    };
    set!("color_scheme", scheme_name);

    // links
    let custom_browser = b("use_custom_web_browser").unwrap_or(false);
    set!("open_url_with", if custom_browser { get("web_browser").unwrap_or("xdg-open").to_string() } else { "xdg-open".into() });
    if let Some(v) = b("control_activates_match") {
        set!("url_requires_ctrl", v);
    }
    if let Some(v) = b("match_web_uris") {
        set!("match_web_uris", v);
    }
    if let Some(v) = b("match_file_uris") {
        set!("match_file_uris", v);
    }
    if let Some(v) = b("match_email_addresses") {
        set!("match_email", v);
    }

    // confirmations
    if let Some(v) = b("confirm_close_tab") {
        set!("confirm_close.tab", v);
    }
    if let Some(v) = b("prompt_on_exit") {
        set!("confirm_close.window", v);
    }

    // dropdown window
    if let Some(v) = i("width_percentage") {
        set!("dropdown.width", ((v as f64 / i32::MAX as f64) * 100.0).round() / 100.0);
    }
    if let Some(v) = i("height_percentage") {
        set!("dropdown.height", ((v as f64 / i32::MAX as f64) * 100.0).round() / 100.0);
    }
    if let Some(v) = b("centered_horizontally") {
        set!("dropdown.centered_horizontally", v);
    }
    if let Some(v) = b("centered_vertically") {
        set!("dropdown.centered_vertically", v);
    }
    let monitor = get("show_on_monitor").unwrap_or("").to_string();
    if !monitor.is_empty() {
        set!("dropdown.monitor", monitor.clone());
    }
    let origin = monitor_origin(&monitor).unwrap_or((0, 0));
    if let Some(x) = i("x_pos") {
        set!("dropdown.x", (x - origin.0 as i64).max(0));
    }
    if let Some(y) = i("y_pos") {
        set!("dropdown.y", (y - origin.1 as i64).max(0));
    }
    if let Some(v) = b("animation") {
        set!("dropdown.animation", v);
    }
    if let Some(v) = i("slide_sleep_usec") {
        set!("dropdown.animation_duration_ms", (v * 32 / 1000).clamp(60, 1000));
    }
    if let Some(v) = i("animation_orientation") {
        set!("dropdown.animation_from", ["top", "bottom", "left", "right"].get(v as usize).copied().unwrap_or("top"));
    }
    if let Some(v) = b("auto_hide_on_focus_lost") {
        set!("dropdown.hide_on_focus_loss", v);
    }
    if let Some(v) = b("auto_hide_on_mouse_leave") {
        set!("dropdown.hide_on_mouse_leave", v);
    }
    if let Some(v) = i("auto_hide_time") {
        set!("dropdown.auto_hide_delay_ms", v);
    }
    if let Some(v) = i("non_focus_pull_up_behaviour") {
        set!("dropdown.unfocused_toggle", if v == 1 { "hide" } else { "focus" });
    }
    if let Some(v) = b("grab_focus") {
        set!("dropdown.grab_focus", v);
    }
    if let Some(v) = b("above") {
        set!("dropdown.always_on_top", v);
    }
    if let Some(v) = b("notaskbar") {
        set!("dropdown.skip_taskbar", v);
    }
    if let Some(v) = b("pinned") {
        set!("dropdown.all_workspaces", v);
    }
    if let Some(v) = b("hidden") {
        set!("dropdown.start_hidden", v);
    }
    if let Some(v) = b("start_fullscreen") {
        set!("dropdown.start_fullscreen", v);
    }
    if let Some(v) = i("on_last_terminal_exit") {
        set!("dropdown.on_last_tab_exit", ["quit", "new_tab", "new_tab_hide"].get(v as usize).copied().unwrap_or("new_tab_hide"));
    }

    let ignored_keys = [
        ("show_title_tooltip", "no tooltips in dtem"),
        ("notebook_border", "dtem has its own tab bar"),
        ("set_as_desktop", "not supported"),
        ("match_numbers", "not supported"),
        ("background_color", "unused by tilda too"),
        ("timer_resolution", "not needed"),
        ("command_timeout_ms", "not needed"),
    ];
    let ignored = ignored_keys
        .iter()
        .filter(|(k, _)| t.contains_key(*k))
        .map(|(k, why)| format!("{k} ({why})"))
        .collect();

    let path_cfg = crate::config::config_path();
    let backup = if path_cfg.exists() {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let bak = path_cfg.with_extension(format!("toml.bak-{ts}"));
        std::fs::copy(&path_cfg, &bak)?;
        Some(bak)
    } else {
        None
    };
    confedit::save_doc(&doc)?;
    Ok(ImportReport { applied, ignored, backup })
}

#[cfg(test)]
mod tests {
    #[test]
    fn accel() {
        assert_eq!(super::gtk_accel("<Shift><Control>t").unwrap(), "ctrl+shift+t");
        assert_eq!(super::gtk_accel("<Primary><Shift>Left").unwrap(), "ctrl+shift+left");
        assert_eq!(super::gtk_accel("<Alt>0").unwrap(), "alt+0");
        assert_eq!(super::gtk_accel("F11").unwrap(), "f11");
        assert_eq!(super::gtk_accel("<Control>Page_Down").unwrap(), "ctrl+page_down");
    }
}
