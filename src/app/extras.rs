//! Notifications, tab activity markers, shell-integration navigation, hints and the
//! "shell exited" prompt.
use super::*;
use crate::stream::StreamEvent;

impl App {
    /// The pane is on screen in a focused, visible window's active tab.
    pub(super) fn attended(&self, pane: u64) -> bool {
        let Some(wid) = self.pane_win.get(&pane) else { return false };
        let Some(w) = self.windows.get(wid) else { return false };
        w.visible && w.focused && w.tabs.get(w.active_tab).map(|t| t.root.leaf_list().contains(&pane)).unwrap_or(false)
    }

    /// Clear activity/bell/failure markers of the tab the user is looking at.
    pub(super) fn clear_attended(&mut self, wid: WindowId) {
        let Some(w) = self.windows.get(&wid) else { return };
        if !(w.visible && w.focused) {
            return;
        }
        let Some(t) = w.tabs.get(w.active_tab) else { return };
        for id in t.root.leaf_list() {
            if let Some(p) = self.panes.get_mut(&id) {
                p.activity = false;
                p.bell = false;
                p.failed = false;
            }
        }
    }

    /// Marker shown in front of a tab title.
    pub(super) fn tab_marker(&self, tab: &Tab) -> &'static str {
        if !self.cfg.tab_activity_markers {
            return "";
        }
        let panes: Vec<&Pane> = tab.root.leaf_list().iter().filter_map(|id| self.panes.get(id)).collect();
        if panes.iter().any(|p| p.bell) {
            "🔔 "
        } else if panes.iter().any(|p| p.failed) {
            "✗ "
        } else if panes.iter().any(|p| p.activity) {
            "● "
        } else {
            ""
        }
    }

    fn tab_label(&mut self, pane: u64) -> String {
        let Some(wid) = self.pane_win.get(&pane).copied() else { return "dtem".into() };
        let Some(w) = self.windows.get(&wid) else { return "dtem".into() };
        let Some((i, t)) = w.tabs.iter().enumerate().find(|(_, t)| t.root.leaf_list().contains(&pane)) else {
            return "dtem".into();
        };
        let title = t.title.clone();
        let base = match title {
            Some(s) => s,
            None => self.panes.get_mut(&pane).map(|p| p.display_title()).unwrap_or_default(),
        };
        format!("Tab {}: {base}", i + 1)
    }

    /// Desktop notification; clicking it focuses the pane.
    pub(super) fn notify(&mut self, pane: u64, title: &str, body: &str) {
        let title = if title.is_empty() { self.tab_label(pane) } else { title.to_string() };
        let body = body.to_string();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            // D-Bus (org.freedesktop.Notifications) — no notify-send needed
            let shown = notify_rust::Notification::new()
                .appname("dtem")
                .icon("utilities-terminal")
                .summary(&title)
                .body(&body)
                .action("default", "Show")
                .show();
            match shown {
                Ok(handle) => handle.wait_for_action(|action| {
                    if action == "default" {
                        let _ = proxy.send_event(UserEvent::FocusPane(pane));
                    }
                }),
                Err(e) => log::warn!("notification failed: {e}"),
            }
        });
    }

    pub(super) fn on_stream(&mut self, _el: &ActiveEventLoop, id: u64, ev: StreamEvent) {
        match ev {
            StreamEvent::Reply(b) => {
                if let Some(p) = self.panes.get(&id) {
                    p.write(b);
                }
            }
            StreamEvent::Cwd { host, path } => {
                let local = hostname();
                if let Some(p) = self.panes.get_mut(&id) {
                    // a different host means we're inside ssh: remember the remote directory
                    if !host.is_empty() && host != local && host != "localhost" {
                        p.remote_cwd = Some(path);
                    }
                }
            }
            StreamEvent::Notify { title, body } => {
                if !self.cfg.notify_programs {
                    return;
                }
                if self.attended(id) {
                    let msg = if title.is_empty() { body } else { format!("{title}: {body}") };
                    self.toast_pane(id, msg);
                } else {
                    self.notify(id, &title, &body);
                }
            }
            StreamEvent::CommandStart => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.cmd_started = Some(Instant::now());
                }
            }
            StreamEvent::CommandEnd(code) => {
                let attended = self.attended(id);
                let threshold = self.cfg.notify_command_seconds;
                let Some(p) = self.panes.get_mut(&id) else { return };
                let started = p.cmd_started.take();
                p.exit_code = code;
                let failed = code.map(|c| c != 0).unwrap_or(false);
                if failed && !attended {
                    p.failed = true;
                }
                let secs = started.map(|s| s.elapsed().as_secs()).unwrap_or(0);
                if threshold > 0 && secs >= threshold && !attended {
                    let body = match code {
                        Some(0) | None => format!("✓ finished in {}", fmt_secs(secs)),
                        Some(c) => format!("✗ failed (exit {c}) after {}", fmt_secs(secs)),
                    };
                    self.notify(id, "", &body);
                }
                if let Some(w) = self.pane_win.get(&id).and_then(|w| self.windows.get(w)) {
                    w.window.request_redraw();
                }
            }
        }
    }

    pub(super) fn focus_pane_global(&mut self, el: &ActiveEventLoop, pane: u64) {
        let Some(wid) = self.pane_win.get(&pane).copied() else { return };
        let Some(w) = self.windows.get_mut(&wid) else { return };
        if let Some(i) = w.tabs.iter().position(|t| t.root.leaf_list().contains(&pane)) {
            w.active_tab = i;
        }
        self.focus_pane(wid, pane);
        self.relayout(wid);
        let w = &self.windows[&wid];
        if w.dropdown && !w.visible {
            self.show_dropdown(el, wid);
        } else {
            if let (Some(x), Some(xid)) = (&self.x11, crate::x11::window_id(&w.window)) {
                x.activate(xid);
            }
            w.window.focus_window();
        }
    }

    // ------------------------------------------------------------ shell-integration navigation

    /// Lines (grid coordinates) where prompts start, oldest first.
    fn prompt_lines(&self, pane: u64) -> Vec<i32> {
        let Some(p) = self.panes.get(&pane) else { return vec![] };
        let term = p.term.lock();
        let g = term.grid();
        let mut out = Vec::new();
        let mut last_mark = String::new();
        let top = -(g.history_size() as i32);
        for l in top..g.screen_lines() as i32 {
            let row = &g[Line(l)];
            let mark = (0..g.columns()).find_map(|c| {
                row[Column(c)].hyperlink().map(|h| h.uri().to_string()).filter(|u| u.starts_with("dtem:prompt:"))
            });
            if let Some(m) = mark {
                if m != last_mark {
                    out.push(l);
                    last_mark = m;
                }
            }
        }
        out
    }

    pub(super) fn jump_prompt(&mut self, wid: WindowId, forward: bool) {
        let Some(id) = self.active_pane_id(wid) else { return };
        let lines = self.prompt_lines(id);
        if lines.is_empty() {
            self.toast(wid, "No prompt marks — shell integration is off or not loaded in this shell");
            return;
        }
        let Some(p) = self.panes.get(&id) else { return };
        let mut term = p.term.lock();
        let top = -(term.grid().display_offset() as i32);
        let target = if forward {
            lines.iter().copied().find(|&l| l > top)
        } else {
            lines.iter().copied().rev().find(|&l| l < top)
        };
        match target {
            Some(l) => {
                let want = (-l).max(0);
                let cur = term.grid().display_offset() as i32;
                term.scroll_display(Scroll::Delta(want - cur));
            }
            None if forward => term.scroll_display(Scroll::Bottom),
            None => {}
        }
        drop(term);
        self.windows[&wid].window.request_redraw();
    }

    /// Text of the most recent command output (OSC 133 C..D region).
    pub(super) fn copy_last_output(&mut self, wid: WindowId) {
        let Some(id) = self.active_pane_id(wid) else { return };
        let Some(p) = self.panes.get(&id) else { return };
        let text = {
            let term = p.term.lock();
            let g = term.grid();
            let top = -(g.history_size() as i32);
            let cols = g.columns();
            // find the newest output mark
            let mut newest: Option<String> = None;
            for l in (top..g.screen_lines() as i32).rev() {
                let row = &g[Line(l)];
                if let Some(u) = (0..cols).find_map(|c| row[Column(c)].hyperlink().map(|h| h.uri().to_string()).filter(|u| u.starts_with("dtem:output:"))) {
                    newest = Some(u);
                    break;
                }
            }
            let Some(mark) = newest else {
                drop(term);
                self.toast(wid, "No command output marks — shell integration is off or not loaded in this shell");
                return;
            };
            let mut lines: Vec<String> = Vec::new();
            let mut cont = false;
            for l in top..g.screen_lines() as i32 {
                let row = &g[Line(l)];
                let cells: String = (0..cols)
                    .filter(|&c| row[Column(c)].hyperlink().map(|h| h.uri() == mark).unwrap_or(false))
                    .filter(|&c| !row[Column(c)].flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER))
                    .map(|c| row[Column(c)].c)
                    .collect();
                let has = (0..cols).any(|c| row[Column(c)].hyperlink().map(|h| h.uri() == mark).unwrap_or(false));
                if !has {
                    continue;
                }
                let cells = cells.replace('\0', " ").replace(crate::images::PLACEHOLDER, " ");
                if cont {
                    lines.last_mut().unwrap().push_str(&cells);
                } else {
                    lines.push(cells);
                }
                cont = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
                if !cont {
                    let last = lines.last_mut().unwrap();
                    *last = last.trim_end().to_string();
                }
            }
            while lines.last().map(|l| l.is_empty()).unwrap_or(false) {
                lines.pop();
            }
            lines.join("\n")
        };
        let n = text.lines().count();
        self.clip_set(false, text);
        self.toast(wid, format!("Copied last command output ({n} lines)"));
    }

    // ------------------------------------------------------------ hints

    pub(super) fn start_hints(&mut self, wid: WindowId) {
        let Some(id) = self.active_pane_id(wid) else { return };
        let Some(p) = self.panes.get(&id) else { return };
        let mut found: Vec<HintItem> = Vec::new();
        {
            let term = p.term.lock();
            let g = term.grid();
            let off = g.display_offset() as i32;
            let cols = g.columns();
            for vr in 0..g.screen_lines() as i32 {
                let line = Line(vr - off);
                let row = &g[line];
                let mut text = String::new();
                let mut col_of = Vec::new();
                for c in 0..cols {
                    let cell = &row[Column(c)];
                    if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                        continue;
                    }
                    let ch = if cell.c == '\0' || cell.c == crate::images::PLACEHOLDER { ' ' } else { cell.c };
                    for _ in 0..ch.len_utf8() {
                        col_of.push(c);
                    }
                    text.push(ch);
                }
                let mut taken: Vec<(usize, usize)> = Vec::new();
                for (kind, re) in self.hint_res.iter() {
                    for m in re.find_iter(&text) {
                        let mut s = m.as_str();
                        while let Some(last) = s.chars().last() {
                            if ".,;:!?'\")]}>".contains(last) && !(last == ')' && s.contains('(')) {
                                s = &s[..s.len() - last.len_utf8()];
                            } else {
                                break;
                            }
                        }
                        if s.len() < 3 {
                            continue;
                        }
                        let (a, b) = (m.start(), m.start() + s.len());
                        if taken.iter().any(|&(x, y)| a < y && b > x) {
                            continue;
                        }
                        taken.push((a, b));
                        found.push(HintItem {
                            line,
                            start: col_of[a],
                            end: col_of[b - 1],
                            text: s.to_string(),
                            kind: *kind,
                            label: String::new(),
                        });
                    }
                }
            }
        }
        if found.is_empty() {
            self.toast(wid, "Nothing to select on screen");
            return;
        }
        // labels: home row first; two letters when there are many matches
        let alphabet: Vec<char> = "asdfghjklqwertyuiopzxcvbnm".chars().collect();
        let n = found.len();
        for (i, h) in found.iter_mut().enumerate() {
            h.label = if n <= alphabet.len() {
                alphabet[i].to_string()
            } else {
                format!("{}{}", alphabet[(i / alphabet.len()) % alphabet.len()], alphabet[i % alphabet.len()])
            };
        }
        let w = self.windows.get_mut(&wid).unwrap();
        w.hints = Some(Hints { pane: id, items: found, typed: String::new() });
        w.window.request_redraw();
    }

    /// Keys while hints are shown. Returns true if consumed.
    pub(super) fn hints_key(&mut self, wid: WindowId, ev: &KeyEvent) -> bool {
        let Some(w) = self.windows.get_mut(&wid) else { return false };
        let Some(h) = w.hints.as_mut() else { return false };
        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => w.hints = None,
            Key::Named(NamedKey::Backspace) => {
                h.typed.pop();
            }
            Key::Character(s) => {
                let paste = s.chars().any(|c| c.is_uppercase()) || self.modifiers.shift_key();
                h.typed.push_str(&s.to_lowercase());
                let typed = h.typed.clone();
                let exact = h.items.iter().find(|i| i.label == typed).cloned();
                let any = h.items.iter().any(|i| i.label.starts_with(&typed));
                let pane = h.pane;
                if let Some(item) = exact {
                    w.hints = None;
                    self.run_hint(wid, pane, item, paste);
                } else if !any {
                    h.typed.pop();
                }
            }
            Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super) => {}
            _ => w.hints = None,
        }
        if let Some(w) = self.windows.get(&wid) {
            w.window.request_redraw();
        }
        true
    }

    fn run_hint(&mut self, wid: WindowId, pane: u64, item: HintItem, paste: bool) {
        if paste {
            self.paste_into(pane, &item.text);
            return;
        }
        match item.kind {
            HintKind::Url => {
                let u = if item.text.starts_with("www.") { format!("https://{}", item.text) } else { item.text.clone() };
                self.open_url(&u);
                self.toast(wid, format!("Opening {u}"));
            }
            HintKind::Email => self.open_url(&format!("mailto:{}", item.text)),
            _ => {
                self.clip_set(false, item.text.clone());
                self.clip_set(true, item.text.clone());
                self.toast(wid, format!("Copied: {}", item.text));
            }
        }
    }

    // ------------------------------------------------------------ "shell exited" prompt

    /// The shell of a pane exited by itself. Ask before closing (config: confirm_close.on_exit).
    pub(super) fn pane_exited(&mut self, el: &ActiveEventLoop, id: u64, code: Option<i32>) {
        let ask = self.cfg.confirm_close.on_exit
            && self.panes.get(&id).map(|p| !p.settings).unwrap_or(false)
            && !self.exiting;
        let Some(wid) = self.pane_win.get(&id).copied() else { return };
        let busy = self.windows.get(&wid).map(|w| w.confirm.is_some()).unwrap_or(true);
        if !ask || busy {
            self.close_pane(el, id);
            return;
        }
        if let Some(p) = self.panes.get_mut(&id) {
            p.exit_code = code;
        }
        // show the pane that exited
        if let Some(w) = self.windows.get_mut(&wid) {
            if let Some(i) = w.tabs.iter().position(|t| t.root.leaf_list().contains(&id)) {
                w.active_tab = i;
            }
        }
        self.focus_pane(wid, id);
        self.relayout(wid);
        let w = self.windows.get_mut(&wid).unwrap();
        w.confirm = Some(Confirm::Exited(id));
        w.window.request_redraw();
    }

    /// Replace an exited pane with a fresh shell in the same directory.
    pub(super) fn restart_pane(&mut self, wid: WindowId, id: u64) {
        let cwd = self.panes.get_mut(&id).and_then(|p| p.cwd());
        self.seed_history = self.panes.get(&id).and_then(|p| p.history_file.clone());
        let rect = self.windows[&wid].pane_rects.iter().find(|(i, _)| *i == id).map(|x| x.1).unwrap_or_default();
        let new = self.alloc_id();
        if let Some(w) = self.windows.get_mut(&wid) {
            for t in w.tabs.iter_mut() {
                t.root.swap(id, new);
                if t.active == id {
                    t.active = new;
                }
            }
        }
        self.pane_win.remove(&id);
        if let Some(p) = self.panes.remove(&id) {
            p.shutdown();
        }
        let opts = SpawnOpts { cwd, command: None, restore: None, persist_id: None, title: None, program: None, remote: None, history_seed: None };
        if !self.spawn_pane(wid, new, opts, rect) {
            self.remove_leaf(wid, new);
        }
        self.relayout(wid);
    }
}

fn fmt_secs(s: u64) -> String {
    if s >= 3600 {
        format!("{}h {}m", s / 3600, (s % 3600) / 60)
    } else if s >= 60 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{s}s")
    }
}

pub(super) fn hostname() -> String {
    std::fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_default()
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum HintKind {
    Url,
    Path,
    Hash,
    Ip,
    Email,
}

#[derive(Clone, Debug)]
pub(super) struct HintItem {
    pub line: Line,
    pub start: usize,
    pub end: usize,
    pub text: String,
    pub kind: HintKind,
    pub label: String,
}

pub(super) struct Hints {
    pub pane: u64,
    pub items: Vec<HintItem>,
    pub typed: String,
}

pub(super) fn hint_regexes() -> Vec<(HintKind, regex::Regex)> {
    let r = |s: &str| regex::Regex::new(s).unwrap();
    vec![
        (HintKind::Url, r(r#"(?:https?|ftp|file)://[^\s<>"'`{}|\\^]+|www\.[^\s<>"'`{}|\\^]+"#)),
        (HintKind::Email, r(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+")),
        (HintKind::Ip, r(r"\b(?:\d{1,3}\.){3}\d{1,3}(?::\d+)?\b|\b(?:[0-9a-fA-F]{1,4}:){7}[0-9a-fA-F]{1,4}\b")),
        (HintKind::Path, r(r"(?:~|\.{1,2})?/[\w.\-@~+%/]+|\b[\w.\-]+/[\w.\-/]+")),
        (HintKind::Hash, r(r"\b[0-9a-f]{7,40}\b")),
    ]
}

// ------------------------------------------------------------ Ctrl+R: history search

pub(super) struct HistSearch {
    pub pane: u64,
    /// all tabs (+ ~/.bash_history) or only this tab
    pub all: bool,
    pub query: String,
    pub items: Vec<String>,
    /// (item index, matched char positions)
    pub matches: Vec<(usize, Vec<usize>)>,
    pub sel: usize,
}

fn read_history_file(path: &std::path::Path, out: &mut Vec<String>) {
    let Ok(data) = std::fs::read(path) else { return };
    let text = String::from_utf8_lossy(&data);
    let fish = path.to_string_lossy().contains("fish");
    let mut v: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if fish {
            if let Some(c) = line.strip_prefix("- cmd: ") {
                v.push(c.replace("\\n", "\n").replace("\\\\", "\\"));
            }
            continue;
        }
        // bash timestamps (#1699999999) and zsh extended history (: 1699999999:0;cmd)
        if line.starts_with('#') && line[1..].chars().all(|c| c.is_ascii_digit()) && line.len() > 1 {
            continue;
        }
        let cmd = if line.starts_with(": ") && line.contains(';') { line.splitn(2, ';').nth(1).unwrap_or("") } else { line };
        if !cmd.trim().is_empty() {
            v.push(cmd.to_string());
        }
    }
    // newest last in the file → newest first
    v.reverse();
    out.extend(v);
}

/// Subsequence fuzzy match: score (higher is better) and matched char positions.
pub(super) fn fuzzy(query: &str, cand: &str) -> Option<(i64, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, vec![]));
    }
    let q: Vec<char> = query.to_lowercase().chars().collect();
    let c: Vec<char> = cand.chars().collect();
    let cl: Vec<char> = cand.to_lowercase().chars().collect();
    if cl.len() != c.len() {
        return None;
    }
    // prefer a contiguous substring match
    let joined: String = q.iter().collect();
    let lower: String = cl.iter().collect();
    if let Some(byte) = lower.find(&joined) {
        let start = lower[..byte].chars().count();
        let word_start = start == 0 || !cl[start - 1].is_alphanumeric();
        let score = 1000 + if word_start { 200 } else { 0 } - start as i64 - c.len() as i64 / 4;
        return Some((score, (start..start + q.len()).collect()));
    }
    let mut pos = Vec::with_capacity(q.len());
    let mut qi = 0;
    let mut score = 0i64;
    let mut last: Option<usize> = None;
    for (i, ch) in cl.iter().enumerate() {
        if qi < q.len() && *ch == q[qi] {
            score += match last {
                Some(l) if l + 1 == i => 15,
                _ => 1,
            };
            if i == 0 || !cl[i - 1].is_alphanumeric() {
                score += 10;
            }
            if let Some(l) = last {
                score -= (i - l - 1).min(10) as i64;
            }
            pos.push(i);
            last = Some(i);
            qi += 1;
        }
    }
    (qi == q.len()).then(|| (score - c.len() as i64 / 8, pos))
}

impl App {
    fn history_items(&self, pane: u64, all: bool) -> Vec<String> {
        let mut raw = Vec::new();
        let own = self.panes.get(&pane).and_then(|p| p.history_file.clone());
        if let Some(f) = &own {
            read_history_file(f, &mut raw);
        }
        if all || own.is_none() {
            let dir = crate::config::state_dir().join("history");
            let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(&dir)
                .map(|rd| {
                    rd.flatten()
                        .filter(|e| Some(e.path()) != own)
                        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                        .collect()
                })
                .unwrap_or_default();
            files.sort_by(|a, b| b.0.cmp(&a.0));
            for (_, f) in files {
                read_history_file(&f, &mut raw);
            }
            if let Some(h) = dirs::home_dir() {
                for f in [".bash_history", ".zsh_history", ".local/share/fish/fish_history"] {
                    read_history_file(&h.join(f), &mut raw);
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        raw.into_iter().filter(|c| seen.insert(c.clone())).take(50_000).collect()
    }

    /// Is the pane's shell waiting at its prompt (no program in the foreground)?
    pub(super) fn shell_idle(&self, pane: u64) -> bool {
        self.panes.get(&pane).map(|p| p.shell_idle() && p.remote.is_none()).unwrap_or(false)
    }

    pub(super) fn open_history_search(&mut self, wid: WindowId) {
        let Some(id) = self.active_pane_id(wid) else { return };
        let items = self.history_items(id, true);
        let mut hs = HistSearch { pane: id, all: true, query: String::new(), items, matches: vec![], sel: 0 };
        refilter(&mut hs);
        let w = self.windows.get_mut(&wid).unwrap();
        w.hist = Some(hs);
        w.window.request_redraw();
    }

    pub(super) fn history_key(&mut self, wid: WindowId, ev: &KeyEvent) -> bool {
        let ctrl = self.modifiers.control_key();
        let Some(w) = self.windows.get_mut(&wid) else { return false };
        let Some(hs) = w.hist.as_mut() else { return false };
        let n = hs.matches.len();
        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => w.hist = None,
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("g") => w.hist = None,
            Key::Named(NamedKey::ArrowUp) => hs.sel = if n == 0 { 0 } else { (hs.sel + 1).min(n - 1) },
            Key::Named(NamedKey::ArrowDown) => hs.sel = hs.sel.saturating_sub(1),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("p") => hs.sel = if n == 0 { 0 } else { (hs.sel + 1).min(n - 1) },
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("n") => hs.sel = hs.sel.saturating_sub(1),
            Key::Named(NamedKey::PageUp) => hs.sel = if n == 0 { 0 } else { (hs.sel + 10).min(n - 1) },
            Key::Named(NamedKey::PageDown) => hs.sel = hs.sel.saturating_sub(10),
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("r") => {
                // toggle: all tabs <-> this tab
                let (pane, all) = (hs.pane, !hs.all);
                let items = self.history_items(pane, all);
                let w = self.windows.get_mut(&wid).unwrap();
                let hs = w.hist.as_mut().unwrap();
                hs.all = all;
                hs.items = items;
                refilter(hs);
            }
            Key::Character(c) if ctrl && c.eq_ignore_ascii_case("u") => {
                hs.query.clear();
                refilter(hs);
            }
            Key::Named(NamedKey::Backspace) => {
                hs.query.pop();
                refilter(hs);
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::Tab) => {
                let cmd = hs.matches.get(hs.sel).map(|(i, _)| hs.items[*i].clone());
                let pane = hs.pane;
                w.hist = None;
                if let Some(cmd) = cmd {
                    if let Some(p) = self.panes.get(&pane) {
                        // replace the current command line: end-of-line, kill to start, insert
                        p.write(b"\x05\x15".to_vec());
                    }
                    self.paste_into(pane, &cmd);
                }
            }
            Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super) => {}
            _ => {
                if let Some(t) = &ev.text {
                    if !ctrl {
                        hs.query.extend(t.chars().filter(|c| !c.is_control()));
                        refilter(hs);
                    }
                }
            }
        }
        if let Some(w) = self.windows.get(&wid) {
            w.window.request_redraw();
        }
        true
    }
}

fn refilter(hs: &mut HistSearch) {
    let mut m: Vec<(i64, usize, Vec<usize>)> = hs
        .items
        .iter()
        .enumerate()
        .filter_map(|(i, c)| fuzzy(&hs.query, c).map(|(s, p)| (s, i, p)))
        .collect();
    if !hs.query.is_empty() {
        // best score first; recency (lower index) breaks ties
        m.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }
    m.truncate(2000);
    hs.matches = m.into_iter().map(|(_, i, p)| (i, p)).collect();
    hs.sel = 0;
}

#[cfg(test)]
mod tests {
    #[test]
    fn fuzzy_match() {
        assert!(super::fuzzy("dckps", "docker ps -a").is_some());
        assert!(super::fuzzy("xyz", "docker ps").is_none());
        let a = super::fuzzy("git", "git status").unwrap().0;
        let b = super::fuzzy("git", "go install thing").unwrap_or((0, vec![])).0;
        assert!(a > b);
    }
}
