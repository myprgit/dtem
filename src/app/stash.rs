//! Tab stash: put a tab away (layout, cwd, scrollback) and bring it back later.
//! Stashed tabs live in `<state>/stash/<id>/` and are shared by all dtem instances
//! (normal windows and every dropdown), so a tab stashed in the dropdown can be
//! restored in a window. The browser shows each tab's scrollback without restoring it.
use super::*;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub(super) struct StashEntry {
    #[serde(skip)]
    pub id: String,
    /// Title shown in the tab bar when it was stashed
    pub title: String,
    pub cwd: Option<String>,
    pub panes: usize,
    /// Unix seconds
    pub stashed_at: u64,
    pub tab: TabState,
}

pub(super) struct StashView {
    pub entries: Vec<StashEntry>,
    pub sel: usize,
    /// Plain-text scrollback of the selected entry, one section per pane
    pub preview: Vec<(String, Vec<String>)>,
    /// `d` was pressed once: the next `d` deletes the selected entry
    pub confirm_delete: bool,
}

pub(super) fn stash_dir() -> PathBuf {
    crate::config::state_dir().join("stash")
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// All stashed tabs, newest first.
pub(super) fn list() -> Vec<StashEntry> {
    let mut v: Vec<StashEntry> = std::fs::read_dir(stash_dir())
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let id = e.file_name().to_string_lossy().to_string();
                    let s = std::fs::read_to_string(e.path().join("tab.json")).ok()?;
                    let mut entry: StashEntry = serde_json::from_str(&s).ok()?;
                    entry.id = id;
                    Some(entry)
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.stashed_at.cmp(&a.stashed_at).then(b.id.cmp(&a.id)));
    v
}

pub(super) fn count() -> usize {
    std::fs::read_dir(stash_dir())
        .map(|rd| rd.flatten().filter(|e| e.path().join("tab.json").is_file()).count())
        .unwrap_or(0)
}

/// Scrollback ids referenced by stashed tabs (their per-tab history files must be kept).
pub(super) fn referenced_ids() -> Vec<String> {
    let mut out = Vec::new();
    fn walk(n: &SNode, out: &mut Vec<String>) {
        match n {
            SNode::Leaf { scrollback: Some(s), .. } => out.push(s.clone()),
            SNode::Leaf { .. } => {}
            SNode::Split { a, b, .. } => {
                walk(a, out);
                walk(b, out);
            }
        }
    }
    for e in list() {
        walk(&e.tab.layout, &mut out);
    }
    out
}

fn leaves(n: &SNode, out: &mut Vec<(Option<String>, Option<String>)>) {
    match n {
        SNode::Leaf { scrollback, cwd, remote, remote_cwd, .. } => {
            let place = match remote {
                Some(args) => Some(format!("⇄ {}{}", args.last().cloned().unwrap_or_default(), remote_cwd.as_ref().map(|c| format!(":{c}")).unwrap_or_default())),
                None => cwd.clone().map(|c| tilde(&c)),
            };
            out.push((scrollback.clone(), place))
        }
        SNode::Split { a, b, .. } => {
            leaves(a, out);
            leaves(b, out);
        }
    }
}

/// `/home/me/x` → `~/x`
pub(super) fn tilde(p: &str) -> String {
    match dirs::home_dir().map(|h| h.display().to_string()) {
        Some(h) if !h.is_empty() && (p == h || p.starts_with(&format!("{h}/"))) => format!("~{}", &p[h.len()..]),
        _ => p.to_string(),
    }
}

/// One section per pane: (header, plain-text scrollback without ANSI sequences).
/// Single-pane tabs get an empty header.
pub(super) fn preview(e: &StashEntry) -> Vec<(String, Vec<String>)> {
    let mut panes = Vec::new();
    leaves(&e.tab.layout, &mut panes);
    let n = panes.len();
    panes
        .into_iter()
        .enumerate()
        .map(|(i, (id, place))| {
            let data = id.and_then(|id| std::fs::read(stash_dir().join(&e.id).join(format!("{id}.ansi"))).ok()).unwrap_or_default();
            let mut lines: Vec<String> =
                strip_ansi(&String::from_utf8_lossy(&data)).lines().map(|l| l.trim_end().replace('\t', "    ")).collect();
            while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
                lines.pop();
            }
            let header = if n > 1 {
                let focus = if i == e.tab.active { " ●" } else { "" };
                format!("pane {}/{n}{focus}  {}", i + 1, place.unwrap_or_default())
            } else {
                String::new()
            };
            (header, lines)
        })
        .collect()
}

/// Drop CSI / OSC / other escape sequences and carriage returns.
pub(super) fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.next() {
                Some('[') => {
                    // parameters/intermediates until a final byte 0x40..=0x7e
                    for c in it.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') | Some('P') | Some('_') | Some('^') => {
                    // string until BEL or ST (ESC \)
                    while let Some(c) = it.next() {
                        if c == '\x07' {
                            break;
                        }
                        if c == '\x1b' && it.peek() == Some(&'\\') {
                            it.next();
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            c if c == '\n' || c == '\t' || !c.is_control() => out.push(c),
            _ => {}
        }
    }
    out
}

/// "5m ago", "3h ago", "2d ago"
pub(super) fn ago(secs: u64) -> String {
    let d = now_secs().saturating_sub(secs);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", d / 60),
        3600..=86_399 => format!("{}h ago", d / 3600),
        _ => format!("{}d ago", d / 86_400),
    }
}

impl App {
    /// Title of a tab as shown in the tab bar (without number and markers).
    pub(super) fn tab_display_title(&mut self, wid: WindowId, idx: usize) -> String {
        let Some(t) = self.windows.get(&wid).and_then(|w| w.tabs.get(idx)) else { return String::new() };
        if let Some(s) = &t.title {
            return s.clone();
        }
        let active = t.active;
        let cfg = &self.cfg;
        self.panes
            .get_mut(&active)
            .map(|p| {
                let fallback = p.fallback_title();
                format_title(cfg, &p.title, &fallback)
            })
            .unwrap_or_default()
    }

    /// Save tab `idx` to the stash and close it. Running programs end (like on restart);
    /// layout, working directories, scrollback and per-tab history are kept.
    pub(super) fn stash_tab(&mut self, el: &ActiveEventLoop, wid: WindowId, idx: usize) {
        let Some(win) = self.windows.get(&wid) else { return };
        let Some(tab) = win.tabs.get(idx) else { return };
        let (root, active, title) = (tab.root.clone(), tab.active, tab.title.clone());
        let ids = root.leaf_list();
        if ids.iter().all(|id| self.panes.get(id).map(|p| p.settings).unwrap_or(true)) {
            self.toast(wid, "The settings tab can't be stashed");
            return;
        }
        let display = self.tab_display_title(wid, idx);
        let layout = self.snode(&root);
        let ts = TabState { title, active: ids.iter().position(|x| *x == active).unwrap_or(0), layout };
        let cwd = self.panes.get_mut(&active).and_then(|p| p.cwd()).map(|c| c.display().to_string());

        let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0);
        let id = format!("{stamp}-{}", std::process::id());
        let dir = stash_dir().join(&id);
        let max = self.cfg.persist_scrollback_lines.max(1000);
        for pid in &ids {
            if let Some(p) = self.panes.get_mut(pid) {
                if let Some(data) = p.dump_scrollback(max) {
                    if let Err(e) = session::write_atomic(&dir.join(format!("{}.ansi", p.persist_id)), &data) {
                        log::error!("stash scrollback: {e}");
                    }
                }
            }
        }
        let entry = StashEntry { id: id.clone(), title: display.clone(), cwd, panes: ids.len(), stashed_at: now_secs(), tab: ts };
        let json = serde_json::to_vec_pretty(&entry).unwrap_or_default();
        if let Err(e) = session::write_atomic(&dir.join("tab.json"), &json) {
            log::error!("stash: {e}");
            let _ = std::fs::remove_dir_all(&dir);
            self.toast(wid, format!("Could not stash the tab: {e}"));
            return;
        }

        // Keep the window open when its last tab goes into the stash.
        if self.windows[&wid].tabs.len() == 1 {
            self.new_tab(wid, None);
        }
        let Some(idx) = self.windows[&wid].tabs.iter().position(|t| t.active == active) else { return };
        self.close_tab(el, wid, idx);
        self.stash_count = count();
        let hint = keys::effective_combos(&self.cfg, "stashed_tabs").first().map(|c| pretty_combo(c)).unwrap_or_default();
        let msg = if hint.is_empty() { format!("Stashed “{display}”") } else { format!("Stashed “{display}”  ({hint}: stashed tabs)") };
        self.toast(wid, msg);
    }

    pub(super) fn open_stash_view(&mut self, wid: WindowId) {
        let entries = list();
        self.stash_count = entries.len();
        let preview = entries.first().map(preview).unwrap_or_default();
        let Some(w) = self.windows.get_mut(&wid) else { return };
        w.stash = Some(StashView { entries, sel: 0, preview, confirm_delete: false });
        w.window.request_redraw();
    }

    fn stash_select(&mut self, wid: WindowId, sel: usize) {
        let Some(v) = self.windows.get_mut(&wid).and_then(|w| w.stash.as_mut()) else { return };
        if v.entries.is_empty() {
            return;
        }
        let sel = sel.min(v.entries.len() - 1);
        if sel != v.sel || v.preview.is_empty() {
            v.sel = sel;
            v.preview = preview(&v.entries[sel]);
        }
        v.confirm_delete = false;
    }

    /// Bring a stashed tab back into window `wid`, right after the active tab.
    pub(super) fn unstash(&mut self, wid: WindowId, entry: &StashEntry) {
        let dir = stash_dir().join(&entry.id);
        // claim it first, so two instances can't restore the same tab
        let claimed = dir.with_extension(format!("restoring-{}", std::process::id()));
        if std::fs::rename(&dir, &claimed).is_err() {
            self.toast(wid, "That tab was already restored elsewhere");
            self.stash_count = count();
            return;
        }
        let sb = session::scrollback_dir(&self.rname());
        let _ = std::fs::create_dir_all(&sb);
        if let Ok(rd) = std::fs::read_dir(&claimed) {
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if name.ends_with(".ansi") {
                    let _ = std::fs::copy(e.path(), sb.join(&name));
                }
            }
        }
        let at = self.windows.get(&wid).map(|w| w.active_tab + 1);
        // restore the stashed output even when session scrollback persistence is off
        let persist = std::mem::replace(&mut self.cfg.persist_scrollback, true);
        self.add_tab_from_state(wid, &entry.tab, true, at);
        self.cfg.persist_scrollback = persist;
        let _ = std::fs::remove_dir_all(&claimed);
        self.stash_count = count();
        self.mark_save_soon();
    }

    /// Keyboard while the stash browser is open. Always consumes the key.
    pub(super) fn stash_key(&mut self, wid: WindowId, ev: &KeyEvent) -> bool {
        let Some(v) = self.windows.get(&wid).and_then(|w| w.stash.as_ref()) else { return false };
        let (sel, n, confirm) = (v.sel, v.entries.len(), v.confirm_delete);
        let ch = |s: &str| matches!(&ev.logical_key, Key::Character(c) if c.eq_ignore_ascii_case(s));
        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => self.windows.get_mut(&wid).unwrap().stash = None,
            _ if ch("q") => self.windows.get_mut(&wid).unwrap().stash = None,
            Key::Named(NamedKey::ArrowUp) => self.stash_select(wid, sel.saturating_sub(1)),
            _ if ch("k") => self.stash_select(wid, sel.saturating_sub(1)),
            Key::Named(NamedKey::ArrowDown) => self.stash_select(wid, sel + 1),
            _ if ch("j") => self.stash_select(wid, sel + 1),
            Key::Named(NamedKey::Home) => self.stash_select(wid, 0),
            Key::Named(NamedKey::End) => self.stash_select(wid, n.saturating_sub(1)),
            Key::Named(NamedKey::Enter) if n > 0 => {
                let w = self.windows.get_mut(&wid).unwrap();
                let entry = w.stash.take().unwrap().entries.swap_remove(sel);
                self.unstash(wid, &entry);
            }
            Key::Named(NamedKey::Delete) if n > 0 => self.stash_delete(wid, confirm),
            _ if ch("d") && n > 0 => self.stash_delete(wid, confirm),
            Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super) => {}
            _ => {
                if let Some(v) = self.windows.get_mut(&wid).and_then(|w| w.stash.as_mut()) {
                    v.confirm_delete = false;
                }
            }
        }
        if let Some(w) = self.windows.get(&wid) {
            w.window.request_redraw();
        }
        true
    }

    pub(super) fn stash_scroll(&mut self, wid: WindowId, up: bool) {
        let Some(sel) = self.windows.get(&wid).and_then(|w| w.stash.as_ref()).map(|v| v.sel) else { return };
        self.stash_select(wid, if up { sel.saturating_sub(1) } else { sel + 1 });
        if let Some(w) = self.windows.get(&wid) {
            w.window.request_redraw();
        }
    }

    /// `really`: delete now; otherwise arm the confirmation ("press d again").
    fn stash_delete(&mut self, wid: WindowId, really: bool) {
        let Some(v) = self.windows.get_mut(&wid).and_then(|w| w.stash.as_mut()) else { return };
        if !really {
            v.confirm_delete = true;
            return;
        }
        let e = v.entries.remove(v.sel);
        let _ = std::fs::remove_dir_all(stash_dir().join(&e.id));
        v.sel = v.sel.min(v.entries.len().saturating_sub(1));
        v.preview.clear();
        v.confirm_delete = false;
        let sel = v.sel;
        self.stash_select(wid, sel);
        self.stash_count = count();
    }

    /// Mouse in the stash browser: click selects, double-click restores, click outside closes.
    pub(super) fn stash_click(&mut self, wid: WindowId, x: f32, y: f32) {
        let Some(win) = self.windows.get(&wid) else { return };
        let Some(v) = win.stash.as_ref() else { return };
        let g = stash_geometry(win.renderer.size(), win.renderer.metrics, self.tab_bar_height(win), &self.cfg);
        if x > g.list_w || y < g.list_y {
            return;
        }
        let row = ((y - g.list_y) / g.row_h) as usize;
        let start = g.first_row(v.sel);
        let i = start + row;
        if i >= v.entries.len() {
            return;
        }
        let now = Instant::now();
        let double = matches!(self.stash_last_click, Some((t, j)) if j == i && now - t < Duration::from_millis(400));
        self.stash_last_click = Some((now, i));
        if double {
            let w = self.windows.get_mut(&wid).unwrap();
            let entry = w.stash.take().unwrap().entries.swap_remove(i);
            self.unstash(wid, &entry);
        } else {
            self.stash_select(wid, i);
        }
        if let Some(w) = self.windows.get(&wid) {
            w.window.request_redraw();
        }
    }
}

/// Layout of the stash browser (shared by drawing and mouse hit-testing).
pub(super) struct StashGeometry {
    pub top: f32,
    pub list_y: f32,
    pub list_w: f32,
    pub row_h: f32,
    pub rows: usize,
}

impl StashGeometry {
    pub fn first_row(&self, sel: usize) -> usize {
        sel.saturating_sub(self.rows.saturating_sub(1))
    }
}

pub(super) fn stash_geometry(size: (f32, f32), m: font::CellMetrics, tab_bar_h: f32, cfg: &Config) -> StashGeometry {
    let (ww, wh) = size;
    let top = if cfg.tab_bar_position == "bottom" { 0.0 } else { tab_bar_h };
    let bottom = if cfg.tab_bar_position == "bottom" { wh - tab_bar_h } else { wh };
    let list_y = top + m.height * 2.0 + 8.0;
    let row_h = (m.height * 2.0 + 6.0).round();
    let rows = (((bottom - list_y - m.height - 8.0) / row_h) as usize).max(1);
    let list_w = (ww * 0.38).clamp((m.width * 30.0).min(ww), (m.width * 60.0).max(ww * 0.25));
    StashGeometry { top, list_y, list_w, row_h, rows }
}

#[cfg(test)]
mod tests {
    #[test]
    fn strip() {
        let s = "\x1b[1;32mdev\x1b[0m:~$ ls\r\n\x1b]133;A\x07a\tb\r\n";
        assert_eq!(super::strip_ansi(s), "dev:~$ ls\na\tb\n");
    }
}
