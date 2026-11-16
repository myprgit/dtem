//! Workspaces: named groups of tabs inside one window ("Work", "Personal", …).
//!
//! The tabs of the *current* workspace live in `OsWin::tabs` / `OsWin::active_tab`, so
//! all tab code keeps working unchanged. The other workspaces keep their tabs in
//! `OsWin::workspaces[i]`; switching swaps the vectors. `workspaces[ws_cur].tabs` is
//! always empty (its tabs are the ones in `OsWin::tabs`).
use super::*;

pub(super) const DEFAULT_NAME: &str = "Default";

pub(super) struct Workspace {
    pub name: String,
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
}

impl Workspace {
    pub fn new(name: impl Into<String>) -> Self {
        Workspace { name: name.into(), tabs: Vec::new(), active_tab: 0 }
    }
}

/// Where a pane lives inside a window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct PaneLoc {
    /// workspace index
    pub ws: usize,
    /// tab index inside that workspace
    pub tab: usize,
}

fn strongest(marks: impl Iterator<Item = &'static str>) -> &'static str {
    let mut best = "";
    for m in marks {
        let rank = |m: &str| match m {
            "🔔 " => 3,
            "✗ " => 2,
            "● " => 1,
            _ => 0,
        };
        if rank(m) > rank(best) {
            best = m;
        }
    }
    best
}

impl OsWin {
    /// Tabs of workspace `i` (the live list for the current one).
    pub(super) fn ws_tabs(&self, i: usize) -> &[Tab] {
        if i == self.ws_cur { &self.tabs } else { self.workspaces.get(i).map(|w| w.tabs.as_slice()).unwrap_or(&[]) }
    }

    /// Every tab of every workspace.
    pub(super) fn all_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().chain(self.workspaces.iter().flat_map(|w| w.tabs.iter()))
    }

    pub(super) fn all_tabs_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut().chain(self.workspaces.iter_mut().flat_map(|w| w.tabs.iter_mut()))
    }

    pub(super) fn find_pane(&self, pane: u64) -> Option<PaneLoc> {
        (0..self.workspaces.len()).find_map(|ws| {
            self.ws_tabs(ws).iter().position(|t| t.root.leaf_list().contains(&pane)).map(|tab| PaneLoc { ws, tab })
        })
    }

    pub(super) fn ws_name(&self) -> &str {
        self.workspaces.get(self.ws_cur).map(|w| w.name.as_str()).unwrap_or(DEFAULT_NAME)
    }

    /// Show the workspace chip in the tab bar?
    pub(super) fn ws_visible(&self) -> bool {
        self.workspaces.len() > 1 || self.rename_ws
    }

    /// Make workspace `i` current (no relayout).
    pub(super) fn ws_swap_to(&mut self, i: usize) {
        if i == self.ws_cur || i >= self.workspaces.len() {
            return;
        }
        let cur = self.ws_cur;
        self.workspaces[cur].tabs = std::mem::take(&mut self.tabs);
        self.workspaces[cur].active_tab = self.active_tab;
        self.tabs = std::mem::take(&mut self.workspaces[i].tabs);
        self.active_tab = self.workspaces[i].active_tab.min(self.tabs.len().saturating_sub(1));
        self.ws_cur = i;
        // overlays and index-based confirmations belong to the old workspace
        if matches!(self.confirm, Some(Confirm::Tab(_) | Confirm::Stash(_) | Confirm::Workspace(_))) {
            self.confirm = None;
        }
        self.rename = None;
        self.rename_ws = false;
        self.search = None;
        self.hints = None;
        self.hist = None;
        self.tab_drag = None;
    }
}

impl App {
    /// Marker of a whole workspace (strongest marker of its tabs).
    pub(super) fn ws_marker(&self, win: &OsWin, i: usize) -> &'static str {
        strongest(win.ws_tabs(i).iter().map(|t| self.tab_marker(t)))
    }

    /// Strongest marker among the workspaces you aren't looking at.
    pub(super) fn ws_others_marker(&self, win: &OsWin) -> &'static str {
        strongest((0..win.workspaces.len()).filter(|i| *i != win.ws_cur).map(|i| self.ws_marker(win, i)))
    }

    pub(super) fn switch_workspace(&mut self, wid: WindowId, i: usize) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        if i == win.ws_cur || i >= win.workspaces.len() {
            return;
        }
        win.ws_swap_to(i);
        if win.tabs.is_empty() {
            self.new_tab(wid, None);
        }
        self.relayout(wid);
        let name = self.windows[&wid].ws_name().to_string();
        self.toast(wid, format!("Workspace: {name}"));
        self.mark_save_soon();
    }

    pub(super) fn cycle_workspace(&mut self, wid: WindowId, forward: bool) {
        let Some(win) = self.windows.get(&wid) else { return };
        let n = win.workspaces.len();
        if n < 2 {
            let key = keys::effective_combos(&self.cfg, "workspaces").first().map(|c| pretty_combo(c)).unwrap_or_default();
            self.toast(wid, format!("Only one workspace. {key} (or click the tab bar's workspace button) creates more."));
            return;
        }
        let i = if forward { (win.ws_cur + 1) % n } else { (win.ws_cur + n - 1) % n };
        self.switch_workspace(wid, i);
    }

    /// New workspace with one fresh tab; asks for its name right away.
    pub(super) fn new_workspace(&mut self, wid: WindowId) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let mut n = win.workspaces.len() + 1;
        while win.workspaces.iter().any(|w| w.name == format!("Workspace {n}")) {
            n += 1;
        }
        let name = format!("Workspace {n}");
        win.workspaces.push(Workspace::new(name.clone()));
        let i = win.workspaces.len() - 1;
        win.ws_swap_to(i);
        self.new_tab(wid, None);
        self.relayout(wid);
        let win = self.windows.get_mut(&wid).unwrap();
        win.rename = Some(name);
        win.rename_ws = true;
        win.window.request_redraw();
        self.mark_save_soon();
    }

    pub(super) fn rename_workspace(&mut self, wid: WindowId) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        win.rename = Some(win.ws_name().to_string());
        win.rename_ws = true;
        win.window.request_redraw();
    }

    /// Move tab `idx` of the current workspace to workspace `to` (None = a new one).
    pub(super) fn move_tab_to_workspace(&mut self, wid: WindowId, idx: usize, to: Option<usize>) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        if idx >= win.tabs.len() || to == Some(win.ws_cur) {
            return;
        }
        let to = match to {
            Some(t) if t < win.workspaces.len() => t,
            Some(_) => return,
            None => {
                let mut n = win.workspaces.len() + 1;
                while win.workspaces.iter().any(|w| w.name == format!("Workspace {n}")) {
                    n += 1;
                }
                win.workspaces.push(Workspace::new(format!("Workspace {n}")));
                win.workspaces.len() - 1
            }
        };
        let tab = win.tabs.remove(idx);
        if win.active_tab >= win.tabs.len() && !win.tabs.is_empty() {
            win.active_tab = win.tabs.len() - 1;
        } else if idx < win.active_tab {
            win.active_tab -= 1;
        }
        let dest = &mut win.workspaces[to];
        dest.tabs.push(tab);
        dest.active_tab = dest.tabs.len() - 1;
        let name = dest.name.clone();
        if win.tabs.is_empty() {
            // the tab was the last one here: follow it
            self.drop_empty_current_workspace(wid, Some(to));
        }
        self.relayout(wid);
        self.toast(wid, format!("Moved the tab to “{name}”"));
        self.mark_save_soon();
    }

    /// The current workspace has no tabs left: remove it and show `prefer` (or a
    /// neighbour). Returns false if it was the only workspace.
    pub(super) fn drop_empty_current_workspace(&mut self, wid: WindowId, prefer: Option<usize>) -> bool {
        let Some(win) = self.windows.get_mut(&wid) else { return false };
        if win.workspaces.len() < 2 || !win.tabs.is_empty() {
            return false;
        }
        let cur = win.ws_cur;
        let target = prefer.filter(|p| *p != cur && *p < win.workspaces.len()).unwrap_or(if cur > 0 { cur - 1 } else { 1 });
        win.ws_swap_to(target);
        win.workspaces.remove(cur);
        if win.ws_cur > cur {
            win.ws_cur -= 1;
        }
        true
    }

    /// Close workspace `i` and every tab in it.
    pub(super) fn close_workspace(&mut self, el: &ActiveEventLoop, wid: WindowId, i: usize) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        if i >= win.workspaces.len() {
            return;
        }
        if i == win.ws_cur && win.workspaces.len() > 1 {
            // show another workspace first, then close this one from the background
            let other = if i > 0 { i - 1 } else { 1 };
            win.ws_swap_to(other);
        }
        let (tabs, only) = if i == win.ws_cur {
            (std::mem::take(&mut win.tabs), true)
        } else {
            (std::mem::take(&mut win.workspaces[i].tabs), false)
        };
        if !only {
            win.workspaces.remove(i);
            if win.ws_cur > i {
                win.ws_cur -= 1;
            }
        } else {
            win.workspaces[i].name = DEFAULT_NAME.to_string();
        }
        for t in tabs {
            for id in t.root.leaf_list() {
                self.pane_win.remove(&id);
                if let Some(p) = self.panes.remove(&id) {
                    p.shutdown();
                }
            }
        }
        self.after_structure_change(el, wid);
    }

    /// Show the workspace and tab that contain `pane` (notifications, exited shells).
    pub(super) fn reveal_pane(&mut self, wid: WindowId, pane: u64) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let Some(loc) = win.find_pane(pane) else { return };
        if loc.ws != win.ws_cur {
            win.ws_swap_to(loc.ws);
        }
        win.active_tab = loc.tab;
    }

    /// Where the workspace menu opens: under the button, or above it for a bottom tab bar
    /// (the menu flips upwards from there instead of covering the bar).
    pub(super) fn ws_menu_anchor(&self, r: Rect) -> (f32, f32) {
        if self.cfg.tab_bar_position == "bottom" { (r.x, r.y - 1.0) } else { (r.x, r.y + r.h) }
    }

    /// Workspace switcher popup (also used by the tab bar's workspace button).
    pub(super) fn open_workspace_menu(&mut self, wid: WindowId, x: f32, y: f32) {
        let Some(win) = self.windows.get(&wid) else { return };
        let hint = |a: &str| keys::effective_combos(&self.cfg, a).first().map(|c| pretty_combo(c)).unwrap_or_default();
        let mut items = Vec::new();
        for i in 0..win.workspaces.len() {
            let name = if i == win.ws_cur { win.ws_name().to_string() } else { win.workspaces[i].name.clone() };
            let n = win.ws_tabs(i).len();
            let cur = if i == win.ws_cur { "▸ " } else { "  " };
            let mark = self.ws_marker(win, i);
            items.push(MenuItem {
                label: format!("{cur}{mark}{name}"),
                hint: format!("{n} tab{}", if n == 1 { "" } else { "s" }),
                cmd: Some(MenuCmd::Workspace(i)),
            });
        }
        let sep = || MenuItem { label: String::new(), hint: String::new(), cmd: None };
        items.push(sep());
        items.push(MenuItem { label: "New Workspace".into(), hint: hint("new_workspace"), cmd: Some(MenuCmd::Action("new_workspace")) });
        items.push(MenuItem { label: "Rename Workspace…".into(), hint: hint("rename_workspace"), cmd: Some(MenuCmd::Action("rename_workspace")) });
        if win.workspaces.len() > 1 {
            items.push(MenuItem { label: "Close Workspace…".into(), hint: String::new(), cmd: Some(MenuCmd::CloseWorkspace(win.ws_cur)) });
        }
        self.show_menu(wid, x, y, items);
        // preselect the current workspace for keyboard use
        let cur = win_cur_of(&self.windows, wid);
        if let Some(m) = self.windows.get_mut(&wid).and_then(|w| w.menu.as_mut()) {
            m.hover = m.items.iter().position(|it| matches!(it.cmd, Some(MenuCmd::Workspace(i)) if i == cur));
        }
    }
}

fn win_cur_of(windows: &HashMap<WindowId, OsWin>, wid: WindowId) -> usize {
    windows.get(&wid).map(|w| w.ws_cur).unwrap_or(0)
}

// ------------------------------------------------------------ session

/// Session description of a window's workspaces; empty when there is only the
/// default workspace (the plain `tabs` list then says it all).
pub(super) fn snapshot_needed(win: &OsWin) -> bool {
    win.workspaces.len() > 1 || win.ws_name() != DEFAULT_NAME
}

impl App {
    /// Rebuild a window's workspaces from the session (old sessions: one workspace).
    /// Tabs are appended to existing ones, so opening a layout file merges into the window.
    pub(super) fn restore_workspaces(&mut self, wid: WindowId, ws: &WinState, restore: bool) {
        let list = ws.workspace_list();
        let want_cur = ws.active_workspace.min(list.len().saturating_sub(1));
        let mut cur_index = None;
        for (k, (name, active_tab, tabs)) in list.into_iter().enumerate() {
            if tabs.is_empty() {
                continue;
            }
            // target workspace: same name if it exists, else the current one for the
            // first unnamed list, else a new workspace
            let Some(win) = self.windows.get_mut(&wid) else { return };
            let target = match &name {
                Some(n) => match (0..win.workspaces.len()).find(|i| {
                    let wn = if *i == win.ws_cur { win.ws_name().to_string() } else { win.workspaces[*i].name.clone() };
                    &wn == n
                }) {
                    Some(i) => i,
                    None if win.workspaces.len() == 1 && win.tabs.is_empty() => {
                        // a fresh window: its default workspace takes the first name
                        let c = win.ws_cur;
                        win.workspaces[c].name = n.clone();
                        c
                    }
                    None => {
                        win.workspaces.push(Workspace::new(n.clone()));
                        win.workspaces.len() - 1
                    }
                },
                None => win.ws_cur,
            };
            win.ws_swap_to(target);
            let before = self.windows[&wid].tabs.len();
            for ts in &tabs {
                self.add_tab_from_state(wid, ts, restore, None);
            }
            if let Some(win) = self.windows.get_mut(&wid) {
                if !win.tabs.is_empty() {
                    win.active_tab = (before + active_tab).min(win.tabs.len() - 1);
                }
                if k == want_cur {
                    cur_index = Some(win.ws_cur);
                }
            }
        }
        if let Some(i) = cur_index {
            if let Some(win) = self.windows.get_mut(&wid) {
                win.ws_swap_to(i);
            }
        }
    }
}
