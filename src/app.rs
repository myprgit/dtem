//! Application: OS windows, tabs, split panes, input, rendering, dropdown & session handling.
use crate::config::{Config, Rgb, Theme};
use crate::font::{self, FontSet};
use crate::keys::{self, Action, Bindings};
use crate::layout::{self, Dir, Node, Rect};
use crate::pane::{Pane, SpawnOpts};
use crate::remote::{self, Remote};
use crate::renderer::{rgba, Gpu, Renderer};
use crate::session::{self, SNode, Session, TabState, WinState};
use crate::UserEvent;
use alacritty_terminal::event::{Event as TermEvent, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{ClearMode, Color, CursorShape, Handler, NamedColor};
use copypasta::ClipboardProvider;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition, PhysicalSize};
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::monitor::MonitorHandle;
use winit::raw_window_handle::{HasDisplayHandle, RawDisplayHandle};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Main,
    Dropdown,
}

impl Role {
    pub fn name(&self) -> &'static str {
        match self {
            Role::Main => "main",
            Role::Dropdown => "dropdown",
        }
    }
}

pub struct Startup {
    pub restore: bool,
    pub session_file: Option<PathBuf>,
    pub show_dropdown: bool,
    pub remote: Option<Vec<String>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Confirm {
    Tab(usize),
    Pane(u64),
    Window,
    /// A program asked to read the clipboard (OSC 52)
    ClipRead(u64),
}

struct Tab {
    root: Node,
    active: u64,
    title: Option<String>,
    zoomed: bool,
}

struct OsWin {
    window: Arc<Window>,
    renderer: Renderer,
    tabs: Vec<Tab>,
    active_tab: usize,
    dropdown: bool,
    visible: bool,
    focused: bool,
    font_size: f32,
    scale: f64,
    rename: Option<String>,
    confirm: Option<Confirm>,
    preedit: Option<String>,
    toast: Option<(String, Instant)>,
    /// (pane, grid line, first col, last col, url) under the mouse while ctrl is held
    hover_url: Option<(u64, i32, usize, usize, String)>,
    pointer_cursor: bool,
    ime_area: Option<(i32, i32)>,
    cursor: (f64, f64),
    left_down: bool,
    selecting: Option<u64>,
    reporting_button: Option<u8>,
    last_click: Option<(Instant, u32, (f64, f64))>,
    monitor: Option<String>,
    pane_rects: Vec<(u64, Rect)>,
    tab_rects: Vec<(usize, Rect)>,
    pos: Option<(i32, i32)>,
    size: (u32, u32),
    last_title: String,
}

struct Clip {
    clipboard: Box<dyn ClipboardProvider>,
    primary: Box<dyn ClipboardProvider>,
}

pub struct App {
    cfg: Config,
    bindings: Bindings,
    theme: Theme,
    role: Role,
    proxy: EventLoopProxy<UserEvent>,
    gpu: Option<Gpu>,
    fonts: FontSet,
    windows: HashMap<WindowId, OsWin>,
    order: Vec<WindowId>,
    panes: HashMap<u64, Pane>,
    pane_win: HashMap<u64, WindowId>,
    next_id: u64,
    modifiers: ModifiersState,
    clip: Option<Clip>,
    startup: Option<Startup>,
    last_save: Instant,
    last_session_json: String,
    hotkey: Option<global_hotkey::GlobalHotKeyManager>,
    focus_try: Option<Instant>,
    /// arboard: images and file lists
    rich_clip: Option<arboard::Clipboard>,
    clip_read_always: bool,
    pending_clip: HashMap<WindowId, Arc<dyn Fn(&str) -> String + Send + Sync>>,
    helpers_done: std::collections::HashSet<String>,
    last_focused: Option<WindowId>,
    url_re: regex::Regex,
    exiting: bool,
}

fn pt_to_px(pt: f32, scale: f64) -> f32 {
    (pt * scale as f32 * 96.0 / 72.0).round()
}

fn to_rgb(c: alacritty_terminal::vte::ansi::Rgb) -> Rgb {
    [c.r, c.g, c.b]
}

fn resolve(c: Color, colors: &Colors, theme: &Theme) -> Rgb {
    match c {
        Color::Spec(rgb) => to_rgb(rgb),
        Color::Indexed(i) => colors[i as usize].map(to_rgb).unwrap_or(theme.table[i as usize]),
        Color::Named(n) => {
            let i = n as usize;
            colors[i].map(to_rgb).unwrap_or(theme.table[i.min(theme.table.len() - 1)])
        }
    }
}

fn dim(c: Rgb) -> Rgb {
    [(c[0] as f32 * 0.66) as u8, (c[1] as f32 * 0.66) as u8, (c[2] as f32 * 0.66) as u8]
}

impl App {
    pub fn new(cfg: Config, role: Role, proxy: EventLoopProxy<UserEvent>, startup: Startup) -> anyhow::Result<App> {
        let fonts = FontSet::new(&cfg.font_family)?;
        Ok(App {
            bindings: Bindings::new(&cfg),
            theme: Theme::from_config(&cfg.colors),
            cfg,
            role,
            proxy,
            gpu: None,
            fonts,
            windows: HashMap::new(),
            order: Vec::new(),
            panes: HashMap::new(),
            pane_win: HashMap::new(),
            next_id: 1,
            modifiers: ModifiersState::empty(),
            clip: None,
            startup: Some(startup),
            last_save: Instant::now(),
            last_session_json: String::new(),
            hotkey: None,
            focus_try: None,
            rich_clip: None,
            clip_read_always: false,
            pending_clip: HashMap::new(),
            helpers_done: Default::default(),
            last_focused: None,
            url_re: regex::Regex::new(r#"(?:https?|ftp|file)://[^\s<>"'`{}|\\^]+|www\.[^\s<>"'`{}|\\^]+"#).unwrap(),
            exiting: false,
        })
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ------------------------------------------------------------------ monitors

    fn monitor_by_name(el: &ActiveEventLoop, name: Option<&str>) -> Option<MonitorHandle> {
        let name = name?;
        el.available_monitors().find(|m| m.name().as_deref() == Some(name))
    }

    fn monitor_at(el: &ActiveEventLoop, x: i32, y: i32) -> Option<MonitorHandle> {
        el.available_monitors().find(|m| {
            let p = m.position();
            let s = m.size();
            x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32
        })
    }

    fn fallback_monitor(el: &ActiveEventLoop) -> Option<MonitorHandle> {
        el.primary_monitor().or_else(|| el.available_monitors().next())
    }

    fn dropdown_geometry(&self, mon: &MonitorHandle) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
        let p = mon.position();
        let mut s = mon.size();
        log::debug!("monitor {:?} pos {:?} size {:?}", mon.name(), p, s);
        if s.width < 100 || s.height < 100 {
            // Broken/unknown monitor info: use a sane default instead of a 0-sized window.
            s = PhysicalSize::new(1280, 800);
        }
        let w = ((s.width as f32) * self.cfg.dropdown.width.clamp(0.1, 1.0)) as u32;
        let h = ((s.height as f32) * self.cfg.dropdown.height.clamp(0.1, 1.0)) as u32;
        (PhysicalPosition::new(p.x + (s.width as i32 - w as i32) / 2, p.y), PhysicalSize::new(w, h))
    }

    // ------------------------------------------------------------------ windows

    fn create_window(&mut self, el: &ActiveEventLoop, ws: Option<&WinState>, dropdown: bool) -> Option<WindowId> {
        let mut attrs = Window::default_attributes().with_title("dtem");
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            let class = if dropdown { "dtem-dropdown" } else { "dtem" };
            attrs = WindowAttributesExtX11::with_name(attrs, class, class);
            attrs = WindowAttributesExtWayland::with_name(attrs, class, class);
        }
        let mut monitor_name = ws.and_then(|w| w.monitor.clone());
        if dropdown {
            let mon = Self::monitor_by_name(el, monitor_name.as_deref()).or_else(|| Self::fallback_monitor(el));
            if let Some(m) = &mon {
                let (pos, size) = self.dropdown_geometry(m);
                attrs = attrs.with_position(pos).with_inner_size(size);
                monitor_name = m.name();
            }
            attrs = attrs
                .with_decorations(false)
                .with_window_level(WindowLevel::AlwaysOnTop)
                .with_visible(false);
        } else {
            match ws.and_then(|w| w.width.zip(w.height)) {
                Some((w, h)) => attrs = attrs.with_inner_size(PhysicalSize::new(w.max(100), h.max(60))),
                None => attrs = attrs.with_inner_size(LogicalSize::new(960.0, 600.0)),
            }
            if let Some((x, y)) = ws.and_then(|w| w.x.zip(w.y)) {
                // Only restore position if it's on a connected monitor
                if Self::monitor_at(el, x + 40, y + 40).is_some() {
                    attrs = attrs.with_position(PhysicalPosition::new(x, y));
                }
            }
            if ws.map(|w| w.maximized).unwrap_or(false) {
                attrs = attrs.with_maximized(true);
            }
        }
        if self.cfg.background_opacity < 1.0 {
            attrs = attrs.with_transparent(true);
        }
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("cannot create window: {e}");
                return None;
            }
        };
        window.set_ime_allowed(self.cfg.ime);
        window.set_cursor(winit::window::CursorIcon::Text);
        if !dropdown && ws.map(|w| w.fullscreen).unwrap_or(false) {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        // GPU + surface
        let surface = if let Some(gpu) = &self.gpu {
            match gpu.instance.create_surface(window.clone()) {
                Ok(s) => s,
                Err(e) => {
                    log::error!("surface: {e}");
                    return None;
                }
            }
        } else {
            match Gpu::new(window.clone()) {
                Ok((g, s)) => {
                    self.gpu = Some(g);
                    s
                }
                Err(e) => {
                    log::error!("GPU init failed: {e}");
                    el.exit();
                    return None;
                }
            }
        };
        if self.clip.is_none() {
            self.clip = Some(make_clipboard(&window));
        }
        let scale = window.scale_factor();
        let font_size = ws.and_then(|w| w.font_size).unwrap_or(self.cfg.font_size);
        let metrics = font::cell_metrics(&self.fonts, pt_to_px(font_size, scale), self.cfg.line_height);
        let size = window.inner_size();
        let renderer = Renderer::new(
            self.gpu.as_ref().unwrap(),
            surface,
            size.width,
            size.height,
            metrics,
            self.cfg.background_opacity,
        );
        let id = window.id();
        let pos = window.outer_position().ok().map(|p| (p.x, p.y));
        self.windows.insert(
            id,
            OsWin {
                window,
                renderer,
                tabs: Vec::new(),
                active_tab: 0,
                dropdown,
                visible: !dropdown,
                focused: false,
                font_size,
                scale,
                rename: None,
                confirm: None,
                preedit: None,
                toast: None,
                hover_url: None,
                pointer_cursor: false,
                ime_area: None,
                cursor: (0.0, 0.0),
                left_down: false,
                selecting: None,
                reporting_button: None,
                last_click: None,
                monitor: monitor_name,
                pane_rects: Vec::new(),
                tab_rects: Vec::new(),
                pos,
                size: (size.width, size.height),
                last_title: String::new(),
            },
        );
        self.order.push(id);
        Some(id)
    }

    fn pad(&self, win: &OsWin) -> f32 {
        (self.cfg.padding * win.scale as f32).round()
    }

    fn tab_bar_height(&self, win: &OsWin) -> f32 {
        if win.tabs.len() >= self.cfg.tab_bar_min_tabs.max(1) {
            (win.renderer.metrics.height + 6.0 * win.scale as f32).round()
        } else {
            0.0
        }
    }

    fn content_rect(&self, win: &OsWin) -> Rect {
        let (w, h) = win.renderer.size();
        let tb = self.tab_bar_height(win);
        Rect { x: 0.0, y: tb, w, h: (h - tb).max(1.0) }
    }

    fn tab_layout(&self, win: &OsWin, tab: &Tab) -> Vec<(u64, Rect)> {
        let area = self.content_rect(win);
        let mut v = Vec::new();
        tab.root.rects(area, layout::BORDER, &mut v);
        if tab.zoomed {
            for (id, r) in v.iter_mut() {
                if *id == tab.active {
                    *r = area;
                }
            }
        }
        v
    }

    fn grid_size(&self, win: &OsWin, r: &Rect) -> (usize, usize) {
        let m = win.renderer.metrics;
        let pad = self.pad(win);
        let cols = ((r.w - 2.0 * pad) / m.width).floor().max(2.0) as usize;
        let rows = ((r.h - 2.0 * pad) / m.height).floor().max(1.0) as usize;
        (cols, rows)
    }

    /// Recompute pane geometry for all tabs of a window and resize PTYs.
    fn relayout(&mut self, wid: WindowId) {
        let Some(win) = self.windows.get(&wid) else { return };
        let mut resizes = Vec::new();
        let mut active_rects = Vec::new();
        for (i, tab) in win.tabs.iter().enumerate() {
            let rects = self.tab_layout(win, tab);
            for (id, r) in &rects {
                resizes.push((*id, self.grid_size(win, r)));
            }
            if i == win.active_tab {
                active_rects = if tab.zoomed { rects.into_iter().filter(|(id, _)| *id == tab.active).collect() } else { rects };
            }
        }
        let m = win.renderer.metrics;
        for (id, (c, r)) in resizes {
            if let Some(p) = self.panes.get_mut(&id) {
                p.resize(c, r, m.width, m.height);
            }
        }
        let win = self.windows.get_mut(&wid).unwrap();
        win.pane_rects = active_rects;
        win.window.request_redraw();
    }

    fn spawn_pane(&mut self, wid: WindowId, id: u64, opts: SpawnOpts, rect: Rect) -> bool {
        let Some(win) = self.windows.get(&wid) else { return false };
        let (cols, rows) = self.grid_size(win, &rect);
        let m = win.renderer.metrics;
        match Pane::spawn(id, &self.cfg, opts, cols, rows, m.width, m.height, self.proxy.clone()) {
            Ok(p) => {
                self.panes.insert(id, p);
                self.pane_win.insert(id, wid);
                true
            }
            Err(e) => {
                log::error!("cannot spawn shell: {e}");
                false
            }
        }
    }

    /// Build a tab from a session description and spawn its panes.
    fn add_tab_from_state(&mut self, wid: WindowId, ts: &TabState, restore_scrollback: bool, at: Option<usize>) {
        let mut specs: Vec<(u64, SpawnOpts)> = Vec::new();
        fn build(app: &mut App, n: &SNode, specs: &mut Vec<(u64, SpawnOpts)>, restore: bool) -> Node {
            match n {
                SNode::Leaf { cwd, command, scrollback, title, remote } => {
                    let id = app.alloc_id();
                    let remote = remote.clone().map(Remote::new_multiplexed);
                    let program = remote.as_ref().map(|r| {
                        // restored remote panes ask before reconnecting (programs aren't restarted)
                        ("/bin/sh".to_string(), vec!["-c".to_string(), r.pane_script(restore)])
                    });
                    if let Some(r) = &remote {
                        app.ensure_remote_helpers(r);
                    }
                    let restore_bytes = if restore && app.cfg.persist_scrollback {
                        scrollback.as_ref().and_then(|s| session::read_scrollback(app.role.name(), s))
                    } else {
                        None
                    };
                    specs.push((
                        id,
                        SpawnOpts {
                            cwd: cwd.as_ref().map(|c| expand_tilde(c)),
                            command: command.clone(),
                            restore: restore_bytes,
                            persist_id: if restore { scrollback.clone() } else { None },
                            title: title.clone(),
                            program,
                            remote,
                        },
                    ));
                    Node::Leaf(id)
                }
                SNode::Split { vertical, ratio, a, b } => Node::Split {
                    vertical: *vertical,
                    ratio: ratio.clamp(0.05, 0.95),
                    a: Box::new(build(app, a, specs, restore)),
                    b: Box::new(build(app, b, specs, restore)),
                },
            }
        }
        let root = build(self, &ts.layout, &mut specs, restore_scrollback);
        let leaves = root.leaf_list();
        let active = leaves.get(ts.active).copied().unwrap_or(leaves[0]);
        let tab = Tab { root, active, title: ts.title.clone(), zoomed: false };
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let idx = at.unwrap_or(win.tabs.len()).min(win.tabs.len());
        win.tabs.insert(idx, tab);
        win.active_tab = idx;
        let win = &self.windows[&wid];
        let rects = self.tab_layout(win, &win.tabs[idx]);
        for (id, opts) in specs {
            let r = rects.iter().find(|(i, _)| *i == id).map(|x| x.1).unwrap_or_default();
            if !self.spawn_pane(wid, id, opts, r) {
                self.remove_leaf(wid, id);
            }
        }
        self.relayout(wid);
    }

    fn new_tab(&mut self, wid: WindowId, cwd: Option<PathBuf>) {
        let at = self.windows.get(&wid).map(|w| if w.tabs.is_empty() { 0 } else { w.active_tab + 1 });
        let ts = TabState {
            title: None,
            active: 0,
            layout: SNode::Leaf {
                cwd: cwd.map(|p| p.display().to_string()),
                command: None,
                scrollback: None,
                title: None,
                remote: None,
            },
        };
        self.add_tab_from_state(wid, &ts, false, at);
    }

    fn active_pane_id(&self, wid: WindowId) -> Option<u64> {
        let w = self.windows.get(&wid)?;
        w.tabs.get(w.active_tab).map(|t| t.active)
    }

    fn active_cwd(&mut self, wid: WindowId) -> Option<PathBuf> {
        let id = self.active_pane_id(wid)?;
        self.panes.get_mut(&id)?.cwd()
    }

    fn split(&mut self, wid: WindowId, vertical: Option<bool>) {
        let Some(active) = self.active_pane_id(wid) else { return };
        let cwd = self.active_cwd(wid);
        let vertical = vertical.unwrap_or_else(|| {
            let w = &self.windows[&wid];
            w.pane_rects.iter().find(|(id, _)| *id == active).map(|(_, r)| r.w > r.h).unwrap_or(true)
        });
        let id = self.alloc_id();
        {
            let win = self.windows.get_mut(&wid).unwrap();
            let tab = &mut win.tabs[win.active_tab];
            tab.zoomed = false;
            tab.root.split(active, id, vertical);
            tab.active = id;
        }
        self.relayout(wid);
        let r = self.windows[&wid].pane_rects.iter().find(|(i, _)| *i == id).map(|x| x.1).unwrap_or_default();
        let opts =
            SpawnOpts { cwd, command: None, restore: None, persist_id: None, title: None, program: None, remote: None };
        if !self.spawn_pane(wid, id, opts, r) {
            self.remove_leaf(wid, id);
        }
        self.relayout(wid);
    }

    /// Remove a pane id from its tab's tree (does not touch the process).
    fn remove_leaf(&mut self, wid: WindowId, id: u64) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let Some(ti) = win.tabs.iter().position(|t| t.root.leaf_list().contains(&id)) else { return };
        let tab = &mut win.tabs[ti];
        let leaves = tab.root.leaf_list();
        let root = std::mem::replace(&mut tab.root, Node::Leaf(0));
        match root.remove(id) {
            Some(r) => {
                tab.root = r;
                if tab.active == id {
                    let pos = leaves.iter().position(|x| *x == id).unwrap_or(0);
                    let remaining = tab.root.leaf_list();
                    tab.active = remaining[pos.saturating_sub(1).min(remaining.len() - 1)];
                    tab.zoomed = false;
                }
            }
            None => {
                win.tabs.remove(ti);
                if win.active_tab >= win.tabs.len() && !win.tabs.is_empty() {
                    win.active_tab = win.tabs.len() - 1;
                } else if ti < win.active_tab {
                    win.active_tab -= 1;
                }
            }
        }
    }

    fn close_pane(&mut self, el: &ActiveEventLoop, id: u64) {
        let Some(wid) = self.pane_win.remove(&id) else { return };
        if let Some(p) = self.panes.remove(&id) {
            p.shutdown();
        }
        self.remove_leaf(wid, id);
        self.after_structure_change(el, wid);
    }

    fn after_structure_change(&mut self, el: &ActiveEventLoop, wid: WindowId) {
        let Some(win) = self.windows.get(&wid) else { return };
        if win.tabs.is_empty() {
            if win.dropdown {
                // Dropdown never disappears: give it a fresh shell and hide it.
                self.new_tab(wid, None);
                self.hide_dropdown(wid);
            } else {
                self.destroy_window(el, wid);
                return;
            }
        }
        self.relayout(wid);
        self.mark_save_soon();
    }

    fn close_tab(&mut self, el: &ActiveEventLoop, wid: WindowId, idx: usize) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        if idx >= win.tabs.len() {
            return;
        }
        let tab = win.tabs.remove(idx);
        if win.active_tab >= win.tabs.len() && !win.tabs.is_empty() {
            win.active_tab = win.tabs.len() - 1;
        }
        for id in tab.root.leaf_list() {
            self.pane_win.remove(&id);
            if let Some(p) = self.panes.remove(&id) {
                p.shutdown();
            }
        }
        self.after_structure_change(el, wid);
    }

    fn destroy_window(&mut self, el: &ActiveEventLoop, wid: WindowId) {
        if let Some(win) = self.windows.remove(&wid) {
            for t in &win.tabs {
                for id in t.root.leaf_list() {
                    self.pane_win.remove(&id);
                    if let Some(p) = self.panes.remove(&id) {
                        p.shutdown();
                    }
                }
            }
        }
        self.order.retain(|w| *w != wid);
        if self.windows.is_empty() {
            self.save_session(true);
            self.exiting = true;
            el.exit();
        } else {
            self.mark_save_soon();
        }
    }

    fn mark_save_soon(&mut self) {
        // Structural changes are saved within a second
        let soon = Instant::now() - Duration::from_secs(self.cfg.autosave_interval_secs.max(1)) + Duration::from_millis(500);
        if soon < self.last_save {
            self.last_save = soon;
        }
    }

    /// Ask for confirmation (if enabled in config) before closing.
    fn request_close(&mut self, el: &ActiveEventLoop, wid: WindowId, c: Confirm) {
        let cc = &self.cfg.confirm_close;
        let ask = match c {
            Confirm::Tab(_) => cc.tab,
            Confirm::Pane(_) => cc.pane,
            Confirm::Window => cc.window,
            Confirm::ClipRead(_) => true,
        };
        if !ask {
            self.execute_close(el, wid, c);
            return;
        }
        if let Some(w) = self.windows.get_mut(&wid) {
            w.confirm = Some(c);
            w.window.focus_window();
            w.window.request_redraw();
        }
    }

    fn execute_close(&mut self, el: &ActiveEventLoop, wid: WindowId, c: Confirm) {
        match c {
            Confirm::Tab(i) => self.close_tab(el, wid, i),
            Confirm::Pane(id) => self.close_pane(el, id),
            Confirm::ClipRead(id) => {
                if let Some(f) = self.pending_clip.remove(&wid) {
                    let text = self.clip_get(false);
                    if let Some(p) = self.panes.get(&id) {
                        p.write(f(&text).into_bytes());
                    }
                }
            }
            Confirm::Window => {
                let normal_windows = self.windows.values().filter(|w| !w.dropdown).count();
                if normal_windows == 1 && self.role == Role::Main {
                    // Closing the last window = quitting: everything is kept for the next start.
                    self.save_session(true);
                    self.exiting = true;
                    el.exit();
                } else {
                    self.destroy_window(el, wid);
                }
            }
        }
    }

    fn confirm_text(&self, wid: WindowId, c: Confirm) -> String {
        let win = &self.windows[&wid];
        match c {
            Confirm::Tab(i) => {
                let n = win.tabs.get(i).map(|t| t.root.leaf_list().len()).unwrap_or(1);
                if n > 1 {
                    format!("Close tab {} ({n} panes)?", i + 1)
                } else {
                    format!("Close tab {}?", i + 1)
                }
            }
            Confirm::Pane(_) => "Close this pane?".to_string(),
            Confirm::ClipRead(id) => {
                let who = self.panes.get(&id).and_then(|p| p.remote.as_ref().map(|r| r.label())).unwrap_or("A program".into());
                format!("{who} wants to read your clipboard.")
            }
            Confirm::Window => {
                let tabs = win.tabs.len();
                let panes: usize = win.tabs.iter().map(|t| t.root.leaf_list().len()).sum();
                let last = self.windows.values().filter(|w| !w.dropdown).count() == 1;
                if last {
                    format!("Quit dtem? ({tabs} tabs, {panes} panes — restored on next start)")
                } else {
                    format!("Close this window? ({tabs} tabs, {panes} panes)")
                }
            }
        }
    }

    // ------------------------------------------------------------------ dropdown

    fn dropdown_id(&self) -> Option<WindowId> {
        self.windows.iter().find(|(_, w)| w.dropdown).map(|(id, _)| *id)
    }

    fn apply_dropdown_geometry(&mut self, el: &ActiveEventLoop, wid: WindowId) {
        let name = self.windows.get(&wid).and_then(|w| w.monitor.clone());
        let Some(mon) = Self::monitor_by_name(el, name.as_deref()).or_else(|| Self::fallback_monitor(el)) else { return };
        let (pos, size) = self.dropdown_geometry(&mon);
        let win = self.windows.get_mut(&wid).unwrap();
        win.monitor = mon.name();
        win.window.set_outer_position(pos);
        let _ = win.window.request_inner_size(size);
    }

    fn show_dropdown(&mut self, el: &ActiveEventLoop, wid: WindowId) {
        self.apply_dropdown_geometry(el, wid);
        let win = self.windows.get_mut(&wid).unwrap();
        win.window.set_visible(true);
        win.window.set_window_level(WindowLevel::AlwaysOnTop);
        win.window.focus_window();
        win.visible = true;
        win.window.request_redraw();
        // Some WMs ignore the position while the window is unmapped: re-apply after mapping.
        self.apply_dropdown_geometry(el, wid);
    }

    fn hide_dropdown(&mut self, wid: WindowId) {
        if let Some(win) = self.windows.get_mut(&wid) {
            win.window.set_visible(false);
            win.visible = false;
        }
    }

    fn toggle_dropdown(&mut self, el: &ActiveEventLoop) {
        let wid = match self.dropdown_id() {
            Some(w) => w,
            None => {
                let Some(w) = self.create_window(el, None, true) else { return };
                self.new_tab(w, None);
                w
            }
        };
        let win = &self.windows[&wid];
        let recently_tried = self.focus_try.map(|t| t.elapsed() < Duration::from_secs(3)).unwrap_or(false);
        if !win.visible {
            self.focus_try = None;
            self.show_dropdown(el, wid);
        } else if win.focused || recently_tried {
            // Focused → hide. Also hide if a focus attempt just failed (WM focus-stealing
            // prevention), so the hotkey never gets "stuck".
            self.focus_try = None;
            self.hide_dropdown(wid);
        } else {
            // Visible behind other windows: bring it to front first (guake behaviour).
            win.window.focus_window();
            self.focus_try = Some(Instant::now());
        }
    }

    fn dropdown_cycle_monitor(&mut self, el: &ActiveEventLoop, wid: WindowId, step: i32) {
        let mut mons: Vec<MonitorHandle> = el.available_monitors().collect();
        if mons.len() < 2 {
            return;
        }
        mons.sort_by_key(|m| (m.position().x, m.position().y));
        let cur = self.windows[&wid].monitor.clone();
        let idx = mons.iter().position(|m| m.name() == cur).unwrap_or(0) as i32;
        let next = (idx + step).rem_euclid(mons.len() as i32) as usize;
        self.windows.get_mut(&wid).unwrap().monitor = mons[next].name();
        self.apply_dropdown_geometry(el, wid);
        self.mark_save_soon();
    }

    // ------------------------------------------------------------------ session

    fn snapshot(&mut self) -> Session {
        let mut windows = Vec::new();
        let order = self.order.clone();
        for wid in order {
            let Some(win) = self.windows.get(&wid) else { continue };
            let mut tabs = Vec::new();
            let tab_info: Vec<(Node, u64, Option<String>)> =
                win.tabs.iter().map(|t| (t.root.clone(), t.active, t.title.clone())).collect();
            for (root, active, title) in tab_info {
                let leaves = root.leaf_list();
                let layout = self.snode(&root);
                tabs.push(TabState { title, active: leaves.iter().position(|x| *x == active).unwrap_or(0), layout });
            }
            let win = &self.windows[&wid];
            let pos = win.window.outer_position().ok().map(|p| (p.x, p.y)).or(win.pos);
            let size = win.window.inner_size();
            let maximized = win.window.is_maximized();
            let monitor = if win.dropdown {
                win.monitor.clone()
            } else {
                win.window.current_monitor().and_then(|m| m.name())
            };
            windows.push(WinState {
                x: pos.map(|p| p.0),
                y: pos.map(|p| p.1),
                width: Some(if maximized { win.size.0 } else { size.width }),
                height: Some(if maximized { win.size.1 } else { size.height }),
                maximized,
                fullscreen: win.window.fullscreen().is_some(),
                monitor,
                font_size: Some(win.font_size),
                active_tab: win.active_tab,
                tabs,
            });
        }
        Session { version: 1, windows }
    }

    fn snode(&mut self, n: &Node) -> SNode {
        match n {
            Node::Leaf(id) => {
                let (cwd, sb, remote) = match self.panes.get_mut(id) {
                    Some(p) => (
                        p.cwd().map(|c| c.display().to_string()),
                        Some(p.persist_id.clone()),
                        p.remote.as_ref().map(|r| r.args.clone()),
                    ),
                    None => (None, None, None),
                };
                SNode::Leaf { cwd, command: None, scrollback: sb, title: None, remote }
            }
            Node::Split { vertical, ratio, a, b } => SNode::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(self.snode(a)),
                b: Box::new(self.snode(b)),
            },
        }
    }

    pub fn save_session(&mut self, force: bool) {
        self.last_save = Instant::now();
        let role = self.role.name();
        let mut keep = Vec::new();
        if self.cfg.persist_scrollback {
            let max = self.cfg.persist_scrollback_lines;
            for p in self.panes.values_mut() {
                if p.dirty || force {
                    if let Some(data) = p.dump_scrollback(max) {
                        if let Err(e) = session::write_scrollback(role, &p.persist_id, &data) {
                            log::error!("scrollback save: {e}");
                        }
                    }
                    p.dirty = false;
                }
                keep.push(p.persist_id.clone());
            }
        }
        let snap = self.snapshot();
        let json = serde_json::to_string(&snap).unwrap_or_default();
        if json != self.last_session_json || force {
            if let Err(e) = session::save(&session::session_path(role), &snap) {
                log::error!("session save failed: {e}");
            } else {
                self.last_session_json = json;
            }
            session::gc_scrollback(role, &keep);
        }
    }

    fn export_session_file(&mut self) {
        let snap = session::to_layout_file(&self.snapshot());
        let dir = crate::config::config_dir().join("sessions");
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let path = dir.join(format!("session-{ts}.json"));
        match session::save(&path, &snap) {
            Ok(_) => log::info!("session written to {}", path.display()),
            Err(e) => log::error!("cannot write {}: {e}", path.display()),
        }
    }

    fn open_session(&mut self, el: &ActiveEventLoop, s: &Session, restore: bool) {
        for ws in &s.windows {
            if ws.tabs.is_empty() {
                continue;
            }
            let wid = if self.role == Role::Dropdown {
                match self.dropdown_id() {
                    Some(w) => {
                        if ws.monitor.is_some() {
                            self.windows.get_mut(&w).unwrap().monitor = ws.monitor.clone();
                        }
                        w
                    }
                    None => match self.create_window(el, Some(ws), true) {
                        Some(w) => w,
                        None => continue,
                    },
                }
            } else {
                match self.create_window(el, Some(ws), false) {
                    Some(w) => w,
                    None => continue,
                }
            };
            for ts in &ws.tabs {
                self.add_tab_from_state(wid, ts, restore, None);
            }
            if let Some(win) = self.windows.get_mut(&wid) {
                if win.tabs.is_empty() {
                    continue;
                }
                win.active_tab = ws.active_tab.min(win.tabs.len() - 1);
            }
            self.relayout(wid);
        }
    }

    fn open_session_file(&mut self, el: &ActiveEventLoop, path: &std::path::Path) {
        match session::load(path) {
            Some(s) => self.open_session(el, &s, false),
            None => log::error!("cannot load session file {}", path.display()),
        }
    }

    // ------------------------------------------------------------------ clipboard

    fn clip_set(&mut self, primary: bool, s: String) {
        if let Some(c) = &mut self.clip {
            let r = if primary { c.primary.set_contents(s) } else { c.clipboard.set_contents(s) };
            if let Err(e) = r {
                log::warn!("clipboard: {e}");
            }
        }
    }

    fn clip_get(&mut self, primary: bool) -> String {
        self.clip
            .as_mut()
            .and_then(|c| if primary { c.primary.get_contents().ok() } else { c.clipboard.get_contents().ok() })
            .unwrap_or_default()
    }

    fn paste(&mut self, wid: WindowId, primary: bool) {
        let Some(id) = self.active_pane_id(wid) else { return };
        // Files / images on the clipboard: upload to ssh panes, or insert local paths.
        if !primary && self.paste_rich(wid, id) {
            return;
        }
        let text = self.clip_get(primary);
        self.paste_into(id, &text);
    }

    // ------------------------------------------------------------------ actions

    fn focus_pane(&mut self, wid: WindowId, id: u64) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let Some(tab) = win.tabs.get_mut(win.active_tab) else { return };
        if tab.active == id {
            return;
        }
        let old = tab.active;
        tab.active = id;
        for (pid, inb) in [(old, false), (id, true)] {
            if let Some(p) = self.panes.get(&pid) {
                if p.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                    p.write(if inb { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                }
            }
        }
        self.relayout(wid);
    }

    fn set_font_size(&mut self, wid: WindowId, size: f32) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        win.font_size = size.clamp(4.0, 72.0);
        let m = font::cell_metrics(&self.fonts, pt_to_px(win.font_size, win.scale), self.cfg.line_height);
        win.renderer.set_metrics(self.gpu.as_ref().unwrap(), m);
        self.relayout(wid);
    }

    fn do_action(&mut self, el: &ActiveEventLoop, wid: WindowId, a: Action) {
        let active = self.active_pane_id(wid);
        match a {
            Action::NewTab => {
                let cwd = self.active_cwd(wid);
                self.new_tab(wid, cwd);
                self.mark_save_soon();
            }
            Action::CloseTab => {
                let idx = self.windows[&wid].active_tab;
                self.request_close(el, wid, Confirm::Tab(idx));
            }
            Action::NextTab | Action::PreviousTab => {
                let win = self.windows.get_mut(&wid).unwrap();
                let n = win.tabs.len();
                if n > 0 {
                    win.active_tab = if a == Action::NextTab { (win.active_tab + 1) % n } else { (win.active_tab + n - 1) % n };
                }
                self.relayout(wid);
            }
            Action::GotoTab(i) => {
                let win = self.windows.get_mut(&wid).unwrap();
                if i >= 1 && i <= win.tabs.len() {
                    win.active_tab = i - 1;
                }
                self.relayout(wid);
            }
            Action::MoveTabForward | Action::MoveTabBackward => {
                let win = self.windows.get_mut(&wid).unwrap();
                let n = win.tabs.len();
                let i = win.active_tab;
                let j = if a == Action::MoveTabForward { (i + 1) % n } else { (i + n - 1) % n };
                win.tabs.swap(i, j);
                win.active_tab = j;
                self.relayout(wid);
                self.mark_save_soon();
            }
            Action::SetTabTitle => {
                let win = self.windows.get_mut(&wid).unwrap();
                let cur = win.tabs.get(win.active_tab).and_then(|t| t.title.clone()).unwrap_or_default();
                win.rename = Some(cur);
                win.window.request_redraw();
            }
            Action::NewWindow => self.split(wid, None),
            Action::HSplit => self.split(wid, Some(false)),
            Action::VSplit => self.split(wid, Some(true)),
            Action::CloseWindow => {
                if let Some(id) = active {
                    self.request_close(el, wid, Confirm::Pane(id));
                }
            }
            Action::NextWindow | Action::PreviousWindow | Action::NthWindow(_) => {
                let win = &self.windows[&wid];
                let Some(tab) = win.tabs.get(win.active_tab) else { return };
                let leaves = tab.root.leaf_list();
                let i = leaves.iter().position(|x| *x == tab.active).unwrap_or(0);
                let n = leaves.len();
                let j = match a {
                    Action::NextWindow => (i + 1) % n,
                    Action::PreviousWindow => (i + n - 1) % n,
                    Action::NthWindow(k) => {
                        if k == 0 || k > n {
                            return;
                        }
                        k - 1
                    }
                    _ => unreachable!(),
                };
                self.focus_pane(wid, leaves[j]);
            }
            Action::MoveWindowForward | Action::MoveWindowBackward => {
                let win = self.windows.get_mut(&wid).unwrap();
                let tab = &mut win.tabs[win.active_tab];
                let leaves = tab.root.leaf_list();
                let n = leaves.len();
                if n > 1 {
                    let i = leaves.iter().position(|x| *x == tab.active).unwrap_or(0);
                    let j = if a == Action::MoveWindowForward { (i + 1) % n } else { (i + n - 1) % n };
                    tab.root.swap(leaves[i], leaves[j]);
                }
                self.relayout(wid);
            }
            Action::Neighbor(d) => {
                if let Some(id) = active {
                    let rects = self.windows[&wid].pane_rects.clone();
                    if let Some(n) = layout::neighbor(&rects, id, d) {
                        self.focus_pane(wid, n);
                    }
                }
            }
            Action::Resize(d) => {
                if let Some(id) = active {
                    let win = self.windows.get_mut(&wid).unwrap();
                    let tab = &mut win.tabs[win.active_tab];
                    let (vert, delta) = match d {
                        Dir::Left => (true, -0.05),
                        Dir::Right => (true, 0.05),
                        Dir::Up => (false, -0.05),
                        Dir::Down => (false, 0.05),
                    };
                    tab.root.resize(id, vert, delta);
                    self.relayout(wid);
                }
            }
            Action::ToggleZoom => {
                let win = self.windows.get_mut(&wid).unwrap();
                if let Some(tab) = win.tabs.get_mut(win.active_tab) {
                    tab.zoomed = !tab.zoomed;
                }
                self.relayout(wid);
            }
            Action::RotateSplit => {
                if let Some(id) = active {
                    let win = self.windows.get_mut(&wid).unwrap();
                    win.tabs[win.active_tab].root.rotate(id);
                    self.relayout(wid);
                }
            }
            Action::Copy => {
                if let Some(s) = active.and_then(|id| self.panes.get(&id)).and_then(|p| p.term.lock().selection_to_string()) {
                    self.clip_set(false, s);
                }
            }
            Action::Paste => self.paste(wid, false),
            Action::PasteSelection => self.paste(wid, true),
            Action::ScrollLineUp
            | Action::ScrollLineDown
            | Action::ScrollPageUp
            | Action::ScrollPageDown
            | Action::ScrollHome
            | Action::ScrollEnd => {
                if let Some(p) = active.and_then(|id| self.panes.get(&id)) {
                    let s = match a {
                        Action::ScrollLineUp => Scroll::Delta(1),
                        Action::ScrollLineDown => Scroll::Delta(-1),
                        Action::ScrollPageUp => Scroll::PageUp,
                        Action::ScrollPageDown => Scroll::PageDown,
                        Action::ScrollHome => Scroll::Top,
                        _ => Scroll::Bottom,
                    };
                    p.term.lock().scroll_display(s);
                    self.windows[&wid].window.request_redraw();
                }
            }
            Action::ClearScrollback => {
                if let Some(p) = active.and_then(|id| self.panes.get_mut(&id)) {
                    p.term.lock().clear_screen(ClearMode::Saved);
                    p.dirty = true;
                    self.windows[&wid].window.request_redraw();
                }
            }
            Action::IncreaseFontSize => {
                let s = self.windows[&wid].font_size + 1.0;
                self.set_font_size(wid, s);
            }
            Action::DecreaseFontSize => {
                let s = self.windows[&wid].font_size - 1.0;
                self.set_font_size(wid, s);
            }
            Action::ResetFontSize => {
                let s = self.cfg.font_size;
                self.set_font_size(wid, s);
            }
            Action::NewOsWindow => {
                if self.role == Role::Main {
                    let cwd = self.active_cwd(wid);
                    if let Some(w) = self.create_window(el, None, false) {
                        self.new_tab(w, cwd);
                    }
                } else if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).spawn();
                }
            }
            Action::ToggleFullscreen => {
                let w = &self.windows[&wid].window;
                w.set_fullscreen(if w.fullscreen().is_some() { None } else { Some(Fullscreen::Borderless(None)) });
            }
            Action::SaveSessionFile => self.export_session_file(),
            Action::ReloadConfig => {
                self.cfg = Config::load();
                self.bindings = Bindings::new(&self.cfg);
                self.theme = Theme::from_config(&self.cfg.colors);
                for w in self.windows.values() {
                    w.window.request_redraw();
                }
            }
            Action::ToggleDropdown => self.toggle_dropdown(el),
            Action::DropdownNextMonitor | Action::DropdownPrevMonitor => {
                if self.windows[&wid].dropdown {
                    self.dropdown_cycle_monitor(el, wid, if a == Action::DropdownNextMonitor { 1 } else { -1 });
                }
            }
            Action::None => {}
        }
    }

    // ------------------------------------------------------------------ input

    fn on_key(&mut self, el: &ActiveEventLoop, wid: WindowId, ev: KeyEvent) {
        if ev.state != ElementState::Pressed {
            // Key releases only matter for apps using the kitty keyboard protocol.
            if let Some(p) = self.active_pane_id(wid).and_then(|id| self.panes.get(&id)) {
                let mode = *p.term.lock().mode();
                if mode.contains(TermMode::REPORT_EVENT_TYPES) {
                    if let Some(b) = keys::encode_kitty(&ev, self.modifiers, mode, 3) {
                        p.write(b);
                    }
                }
            }
            return;
        }
        if self.windows.get(&wid).map(|w| w.preedit.is_some()).unwrap_or(false) {
            return; // input method is composing
        }
        // Close confirmation prompt
        if let Some(c) = self.windows.get(&wid).and_then(|w| w.confirm) {
            let always = matches!(&ev.logical_key, Key::Character(s) if s.eq_ignore_ascii_case("a"))
                && matches!(c, Confirm::ClipRead(_));
            let yes = always
                || match &ev.logical_key {
                    Key::Named(NamedKey::Enter) => true,
                    Key::Character(s) => matches!(s.as_str(), "y" | "Y" | "e" | "E"),
                    _ => false,
                };
            if always {
                self.clip_read_always = true;
            }
            let modifier_only = matches!(
                &ev.logical_key,
                Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super)
            );
            if modifier_only {
                return;
            }
            self.windows.get_mut(&wid).unwrap().confirm = None;
            if yes {
                self.execute_close(el, wid, c);
            } else {
                self.pending_clip.remove(&wid);
            }
            if let Some(w) = self.windows.get(&wid) {
                w.window.request_redraw();
            }
            return;
        }
        // Tab rename prompt
        if let Some(buf) = self.windows.get_mut(&wid).and_then(|w| w.rename.as_mut()) {
            match &ev.logical_key {
                Key::Named(NamedKey::Enter) => {
                    let title = buf.trim().to_string();
                    let win = self.windows.get_mut(&wid).unwrap();
                    win.rename = None;
                    if let Some(t) = win.tabs.get_mut(win.active_tab) {
                        t.title = if title.is_empty() { None } else { Some(title) };
                    }
                    self.mark_save_soon();
                }
                Key::Named(NamedKey::Escape) => self.windows.get_mut(&wid).unwrap().rename = None,
                Key::Named(NamedKey::Backspace) => {
                    buf.pop();
                }
                _ => {
                    if let Some(t) = &ev.text {
                        buf.extend(t.chars().filter(|c| !c.is_control()));
                    }
                }
            }
            self.windows[&wid].window.request_redraw();
            return;
        }
        if let Some(a) = self.bindings.lookup(&ev, self.modifiers) {
            self.do_action(el, wid, a);
            return;
        }
        let Some(id) = self.active_pane_id(wid) else { return };
        let Some(p) = self.panes.get(&id) else { return };
        let mode = *p.term.lock().mode();
        let bytes = if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
            keys::encode_kitty(&ev, self.modifiers, mode, if ev.repeat { 2 } else { 1 })
        } else {
            keys::encode(&ev, self.modifiers, mode)
        };
        if let Some(bytes) = bytes {
            {
                let mut t = p.term.lock();
                t.scroll_display(Scroll::Bottom);
                t.selection = None;
            }
            p.write(bytes);
        }
    }

    fn pane_at(&self, wid: WindowId, x: f64, y: f64) -> Option<(u64, Rect)> {
        self.windows[&wid].pane_rects.iter().find(|(_, r)| r.contains(x as f32, y as f32)).copied()
    }

    fn cell_at(&self, wid: WindowId, id: u64, rect: Rect, x: f64, y: f64) -> (Point, Side, usize, usize) {
        let win = &self.windows[&wid];
        let m = win.renderer.metrics;
        let pad = self.pad(win);
        let p = &self.panes[&id];
        let fx = (x as f32 - rect.x - pad) / m.width;
        let fy = (y as f32 - rect.y - pad) / m.height;
        let col = (fx.floor().max(0.0) as usize).min(p.cols - 1);
        let row = (fy.floor().max(0.0) as usize).min(p.rows - 1);
        let side = if fx - fx.floor() > 0.5 { Side::Right } else { Side::Left };
        let off = p.term.lock().grid().display_offset() as i32;
        (Point::new(Line(row as i32 - off), Column(col)), side, col, row)
    }

    fn mouse_report(&self, id: u64, button: u8, pressed: bool, motion: bool, col: usize, row: usize) -> bool {
        let Some(p) = self.panes.get(&id) else { return false };
        let mode = *p.term.lock().mode();
        if !mode.intersects(TermMode::MOUSE_MODE) || self.modifiers.shift_key() {
            return false;
        }
        if motion && !(mode.contains(TermMode::MOUSE_MOTION) || (mode.contains(TermMode::MOUSE_DRAG) && button < 3)) {
            return true;
        }
        let mut b = button;
        if self.modifiers.alt_key() {
            b += 8;
        }
        if self.modifiers.control_key() {
            b += 16;
        }
        if motion {
            b += 32;
        }
        if mode.contains(TermMode::SGR_MOUSE) {
            p.write(format!("\x1b[<{};{};{}{}", b, col + 1, row + 1, if pressed { 'M' } else { 'm' }).into_bytes());
        } else {
            let b = if pressed || motion { b } else { 3 + (b & !3) };
            let enc = |v: usize| (32 + v + 1).min(255) as u8;
            p.write(vec![0x1b, b'[', b'M', 32 + b, enc(col), enc(row)]);
        }
        true
    }

    fn on_mouse_button(&mut self, el: &ActiveEventLoop, wid: WindowId, state: ElementState, button: MouseButton) {
        let (x, y) = self.windows[&wid].cursor;
        let pressed = state == ElementState::Pressed;
        // Tab bar
        if pressed {
            let hit = self.windows[&wid].tab_rects.iter().find(|(_, r)| r.contains(x as f32, y as f32)).map(|x| x.0);
            if let Some(i) = hit {
                match button {
                    MouseButton::Left => {
                        self.windows.get_mut(&wid).unwrap().active_tab = i;
                        self.relayout(wid);
                    }
                    MouseButton::Middle => self.request_close(el, wid, Confirm::Tab(i)),
                    _ => {}
                }
                return;
            }
        }
        let btn = match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
            _ => return,
        };
        let target = if !pressed { self.windows[&wid].selecting.or(self.active_pane_id(wid)) } else { None };
        let hit = self.pane_at(wid, x, y);
        let (id, rect) = match (pressed, hit) {
            (true, Some(h)) => h,
            (false, _) => match target.and_then(|t| self.windows[&wid].pane_rects.iter().find(|(i, _)| *i == t).copied()) {
                Some(h) => h,
                None => return,
            },
            _ => return,
        };
        if pressed {
            self.focus_pane(wid, id);
        }
        let (point, side, col, row) = self.cell_at(wid, id, rect, x, y);

        // Ctrl+click opens URLs / OSC 8 hyperlinks
        if pressed && button == MouseButton::Left && self.modifiers.control_key() {
            if let Some((_, _, url)) = self.url_at(id, point) {
                self.open_url(&url);
                return;
            }
        }

        // Mouse reporting to applications (vim, htop, ...)
        let reporting = self.windows[&wid].reporting_button;
        if (pressed || reporting == Some(btn)) && self.mouse_report(id, btn, pressed, false, col, row) {
            self.windows.get_mut(&wid).unwrap().reporting_button = if pressed { Some(btn) } else { None };
            return;
        }

        match (button, pressed) {
            (MouseButton::Left, true) => {
                let now = Instant::now();
                let win = self.windows.get_mut(&wid).unwrap();
                let count = match win.last_click {
                    Some((t, c, (px, py)))
                        if now - t < Duration::from_millis(400) && (px - x).abs() < 4.0 && (py - y).abs() < 4.0 =>
                    {
                        c % 3 + 1
                    }
                    _ => 1,
                };
                win.last_click = Some((now, count, (x, y)));
                win.left_down = true;
                win.selecting = Some(id);
                let ty = match count {
                    2 => SelectionType::Semantic,
                    3 => SelectionType::Lines,
                    _ => SelectionType::Simple,
                };
                let p = &self.panes[&id];
                let mut t = p.term.lock();
                t.selection = if count == 1 && !self.modifiers.shift_key() {
                    Some(Selection::new(ty, point, side))
                } else if count == 1 {
                    t.selection.take().map(|mut s| {
                        s.update(point, side);
                        s
                    })
                } else {
                    Some(Selection::new(ty, point, side))
                };
                drop(t);
                self.windows[&wid].window.request_redraw();
            }
            (MouseButton::Left, false) => {
                let win = self.windows.get_mut(&wid).unwrap();
                win.left_down = false;
                if let Some(sid) = win.selecting.take() {
                    let text = self.panes.get(&sid).and_then(|p| p.term.lock().selection_to_string());
                    if let Some(s) = text.filter(|s| !s.is_empty()) {
                        self.clip_set(true, s.clone());
                        if self.cfg.copy_on_select {
                            self.clip_set(false, s);
                        }
                    }
                }
            }
            (MouseButton::Middle, true) => self.paste(wid, true),
            _ => {}
        }
    }

    fn on_cursor_moved(&mut self, wid: WindowId, x: f64, y: f64) {
        self.windows.get_mut(&wid).unwrap().cursor = (x, y);
        self.update_hover(wid);
        let win = self.windows.get_mut(&wid).unwrap();
        let reporting = win.reporting_button;
        let selecting = if win.left_down { win.selecting } else { None };
        if let Some(b) = reporting {
            if let Some((id, rect)) = self.pane_at(wid, x, y) {
                let (_, _, col, row) = self.cell_at(wid, id, rect, x, y);
                self.mouse_report(id, b, true, true, col, row);
            }
            return;
        }
        if let Some(id) = selecting {
            let Some(rect) = self.windows[&wid].pane_rects.iter().find(|(i, _)| *i == id).map(|x| x.1) else { return };
            let (point, side, _, _) = self.cell_at(wid, id, rect, x, y);
            if let Some(p) = self.panes.get(&id) {
                if let Some(s) = p.term.lock().selection.as_mut() {
                    s.update(point, side);
                }
            }
            self.windows[&wid].window.request_redraw();
        } else if let Some((id, rect)) = self.pane_at(wid, x, y) {
            // plain motion reporting (MOUSE_MOTION mode)
            let wants = self.panes.get(&id).map(|p| p.term.lock().mode().contains(TermMode::MOUSE_MOTION)).unwrap_or(false);
            if wants {
                let (_, _, col, row) = self.cell_at(wid, id, rect, x, y);
                self.mouse_report(id, 3, true, true, col, row);
            }
        }
    }

    fn on_wheel(&mut self, wid: WindowId, delta: MouseScrollDelta) {
        let win = &self.windows[&wid];
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => (y * 3.0) as i32,
            MouseScrollDelta::PixelDelta(p) => (p.y / win.renderer.metrics.height as f64) as i32,
        };
        if lines == 0 {
            return;
        }
        let (x, y) = win.cursor;
        let Some((id, rect)) = self.pane_at(wid, x, y) else { return };
        let (_, _, col, row) = self.cell_at(wid, id, rect, x, y);
        let btn = if lines > 0 { 64 } else { 65 };
        let p = &self.panes[&id];
        let mode = *p.term.lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) && !self.modifiers.shift_key() {
            for _ in 0..lines.abs() {
                self.mouse_report(id, btn, true, false, col, row);
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            p.write(seq.repeat(lines.unsigned_abs() as usize));
        } else {
            p.term.lock().scroll_display(Scroll::Delta(lines));
            win.window.request_redraw();
        }
    }

    // ------------------------------------------------------------------ rendering

    fn render(&mut self, wid: WindowId) {
        let confirm_text = self.windows.get(&wid).and_then(|w| w.confirm).map(|c| {
            let hint = if matches!(c, Confirm::ClipRead(_)) {
                "[y] allow once   [a] always   [n] deny"
            } else {
                "[y/Enter] yes   [n/Esc] no"
            };
            format!("{}   {hint}", self.confirm_text(wid, c))
        });
        let App { windows, panes, fonts, gpu, theme, cfg, .. } = self;
        let Some(gpu) = gpu.as_ref() else { return };
        let Some(win) = windows.get_mut(&wid) else { return };
        if !win.visible {
            return;
        }
        let m = win.renderer.metrics;
        let pad = (cfg.padding * win.scale as f32).round();
        let (ww, _wh) = win.renderer.size();
        let bg_default = theme.table[257];
        let rects = win.pane_rects.clone();
        let active_tab = win.active_tab;
        let active_pane = win.tabs.get(active_tab).map(|t| t.active);
        let multi = rects.len() > 1;

        // Borders
        if multi {
            for (id, r) in &rects {
                let c = if Some(*id) == active_pane { theme.active_border } else { theme.inactive_border };
                if Some(*id) != active_pane {
                    win.renderer.rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, rgba(c, 1.0));
                }
            }
            if let Some((_, r)) = rects.iter().find(|(id, _)| Some(*id) == active_pane) {
                win.renderer.rect(r.x - 1.0, r.y - 1.0, r.w + 2.0, r.h + 2.0, rgba(theme.active_border, 1.0));
            }
        }

        let mut cursor_px: Option<(f32, f32)> = None;
        for (id, r) in &rects {
            let Some(pane) = panes.get(id) else { continue };
            let is_active = Some(*id) == active_pane;
            if win.renderer.opacity >= 1.0 {
                win.renderer.rect(r.x, r.y, r.w, r.h, rgba(bg_default, 1.0));
            }
            let hover = win.hover_url.as_ref().filter(|h| h.0 == *id).map(|h| (h.1, h.2, h.3));
            let term = pane.term.lock();
            let content = term.renderable_content();
            let colors = content.colors;
            let offset = content.display_offset as i32;
            let selection = content.selection;
            let cursor = content.cursor;
            let ox = r.x + pad;
            let oy = r.y + pad;
            let mut row_buf: Vec<RCell> = Vec::with_capacity(pane.cols);
            let mut cur_row = i32::MIN;
            for ind in content.display_iter {
                let cell = ind.cell;
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                let row = ind.point.line.0 + offset;
                let col = ind.point.column.0;
                if row != cur_row {
                    draw_row(&mut win.renderer, gpu, fonts, &row_buf, ox, oy + cur_row as f32 * m.height, cfg.ligatures);
                    row_buf.clear();
                    cur_row = row;
                }
                let x = ox + col as f32 * m.width;
                let y = oy + row as f32 * m.height;
                let mut fg = resolve(cell.fg, colors, theme);
                let mut bg = resolve(cell.bg, colors, theme);
                let mut bg_set = !matches!(cell.bg, Color::Named(NamedColor::Background));
                if cell.flags.contains(Flags::DIM) {
                    fg = dim(fg);
                }
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                    bg_set = true;
                }
                if selection.map(|s| s.contains(ind.point)).unwrap_or(false) {
                    fg = theme.sel_fg;
                    bg = theme.sel_bg;
                    bg_set = true;
                }
                if cell.flags.contains(Flags::HIDDEN) {
                    fg = bg;
                }
                let cw = if cell.flags.contains(Flags::WIDE_CHAR) { m.width * 2.0 } else { m.width };
                if bg_set {
                    win.renderer.rect(x, y, cw, m.height, rgba(bg, 1.0));
                }
                let mut style = font::REGULAR;
                if cell.flags.contains(Flags::BOLD) {
                    style |= font::BOLD;
                }
                if cell.flags.contains(Flags::ITALIC) {
                    style |= font::ITALIC;
                }
                let fgc = rgba(fg, 1.0);
                let wide = cell.flags.contains(Flags::WIDE_CHAR);
                row_buf.push(RCell {
                    col,
                    c: cell.c,
                    fg: fgc,
                    style,
                    wide,
                    zw: cell.zerowidth().map(|z| z.to_vec()).unwrap_or_default(),
                });
                let hovered = hover
                    .map(|(l, a, b)| l == ind.point.line.0 && (a..=b).contains(&col))
                    .unwrap_or(false);
                if cell.flags.intersects(Flags::ALL_UNDERLINES) || hovered {
                    win.renderer.rect_top(x, y + m.underline_pos, cw, m.thickness, fgc);
                }
                if cell.flags.contains(Flags::STRIKEOUT) {
                    win.renderer.rect_top(x, y + (m.height / 2.0).round(), cw, m.thickness, fgc);
                }
            }
            draw_row(&mut win.renderer, gpu, fonts, &row_buf, ox, oy + cur_row as f32 * m.height, cfg.ligatures);
            // Cursor
            let crow = cursor.point.line.0 + offset;
            if cursor.shape != CursorShape::Hidden && crow >= 0 && (crow as usize) < pane.rows {
                let x = ox + cursor.point.column.0 as f32 * m.width;
                let y = oy + crow as f32 * m.height;
                let cell = &term.grid()[cursor.point];
                let cw = if cell.flags.contains(Flags::WIDE_CHAR) { m.width * 2.0 } else { m.width };
                let cc = rgba(colors[NamedColor::Cursor].map(to_rgb).unwrap_or(theme.table[258]), 1.0);
                if is_active {
                    cursor_px = Some((x, y));
                }
                let focused = win.focused && is_active;
                let shape = if !focused {
                    CursorShape::HollowBlock
                } else if cursor.shape == CursorShape::Block {
                    match cfg.cursor_shape.as_str() {
                        "beam" => CursorShape::Beam,
                        "underline" => CursorShape::Underline,
                        _ => CursorShape::Block,
                    }
                } else {
                    cursor.shape
                };
                let t = m.thickness.max(1.0);
                match shape {
                    CursorShape::Block => {
                        win.renderer.rect_top(x, y, cw, m.height, cc);
                        let mut style = font::REGULAR;
                        if cell.flags.contains(Flags::BOLD) {
                            style |= font::BOLD;
                        }
                        let cells = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
                        win.renderer.glyph(gpu, fonts, cell.c, style, cells, x, y, rgba(theme.cursor_text, 1.0));
                    }
                    CursorShape::Beam => win.renderer.rect_top(x, y, (t * 2.0).max(2.0), m.height, cc),
                    CursorShape::Underline => win.renderer.rect_top(x, y + m.height - t * 2.0, cw, t * 2.0, cc),
                    _ => {
                        win.renderer.rect_top(x, y, cw, t, cc);
                        win.renderer.rect_top(x, y + m.height - t, cw, t, cc);
                        win.renderer.rect_top(x, y, t, m.height, cc);
                        win.renderer.rect_top(x + cw - t, y, t, m.height, cc);
                    }
                }
            }
        }

        // IME: position the candidate window and draw the preedit text at the cursor
        if let Some((cx, cy)) = cursor_px {
            let area = (cx as i32, cy as i32);
            if win.ime_area != Some(area) {
                win.ime_area = Some(area);
                win.window.set_ime_cursor_area(
                    PhysicalPosition::new(cx as f64, cy as f64),
                    PhysicalSize::new(m.width as f64, m.height as f64),
                );
            }
            if let Some(pre) = win.preedit.clone() {
                let w = pre.chars().map(|c| if c.len_utf8() > 2 { 2.0 } else { 1.0 }).sum::<f32>() * m.width;
                win.renderer.rect_top(cx, cy, w, m.height, rgba(theme.table[257], 1.0));
                win.renderer.text(gpu, fonts, &pre, cx, cy, w + m.width, rgba(theme.table[256], 1.0));
                win.renderer.rect_top(cx, cy + m.height - m.thickness, w, m.thickness, rgba(theme.table[256], 1.0));
            }
        }

        // Tab bar
        let tb_h = if win.tabs.len() >= cfg.tab_bar_min_tabs.max(1) { (m.height + 6.0 * win.scale as f32).round() } else { 0.0 };
        win.tab_rects.clear();
        let mut titles: Vec<String> = Vec::new();
        for (i, t) in win.tabs.iter().enumerate() {
            let base = match &t.title {
                Some(s) => s.clone(),
                None => panes.get_mut(&t.active).map(|p| p.display_title()).unwrap_or_default(),
            };
            let base = if win.rename.is_some() && i == active_tab {
                format!("✎ {}▏", win.rename.as_ref().unwrap())
            } else {
                base
            };
            let z = if t.zoomed { " [Z]" } else { "" };
            let n = t.root.leaf_list().len();
            let count = if n > 1 { format!(" ({n})") } else { String::new() };
            titles.push(format!("{}: {}{}{}", i + 1, base, count, z));
        }
        if tb_h > 0.0 {
            win.renderer.rect(0.0, 0.0, ww, tb_h, rgba(theme.tab_bar_bg, 1.0));
            let n = win.tabs.len().max(1) as f32;
            let tw = (ww / n).min(m.width * 32.0).floor();
            for (i, title) in titles.iter().enumerate() {
                let x = i as f32 * tw;
                let (fg, bg) = if i == active_tab {
                    (theme.tab_active_fg, theme.tab_active_bg)
                } else {
                    (theme.tab_inactive_fg, theme.tab_inactive_bg)
                };
                win.renderer.rect(x, 0.0, tw - 1.0, tb_h, rgba(bg, 1.0));
                let ty = ((tb_h - m.height) / 2.0).round();
                win.renderer.text(gpu, fonts, title, x + m.width, ty, tw - 2.0 * m.width, rgba(fg, 1.0));
                win.tab_rects.push((i, Rect { x, y: 0.0, w: tw, h: tb_h }));
            }
        } else if win.rename.is_some() {
            // Rename prompt without a tab bar: draw an overlay line on top
            win.renderer.rect(0.0, 0.0, ww, m.height + 4.0, rgba(theme.tab_active_bg, 1.0));
            if let Some(t) = titles.get(active_tab) {
                win.renderer.text(gpu, fonts, &format!("Tab title: {t}"), m.width, 2.0, ww, rgba(theme.tab_active_fg, 1.0));
            }
        }
        // Window title
        if let Some(t) = titles.get(active_tab) {
            let t = t.split_once(": ").map(|x| x.1).unwrap_or(t).to_string();
            let title = format!("{t} — dtem");
            if title != win.last_title {
                win.window.set_title(&title);
                win.last_title = title;
            }
        }
        // Close confirmation overlay
        if let Some(text) = confirm_text {
            let (_, wh) = win.renderer.size();
            let line = text;
            let tw = (line.chars().count() as f32 + 4.0) * m.width;
            let bw = tw.min(ww);
            let bh = m.height * 3.0;
            let bx = ((ww - bw) / 2.0).max(0.0);
            let by = ((wh - bh) / 2.0).max(0.0);
            win.renderer.rect_top(bx - 2.0, by - 2.0, bw + 4.0, bh + 4.0, rgba(theme.active_border, 1.0));
            win.renderer.rect_top(bx, by, bw, bh, rgba(theme.tab_bar_bg, 1.0));
            win.renderer.text(gpu, fonts, &line, bx + 2.0 * m.width, by + m.height, bw - 2.0 * m.width, rgba(theme.table[256], 1.0));
        }
        // Toast (bottom right)
        if let Some((msg, t)) = win.toast.clone() {
            if t.elapsed() < Duration::from_secs(4) {
                let (_, wh) = win.renderer.size();
                let tw = ((msg.chars().count() as f32 + 2.0) * m.width).min(ww - 8.0);
                let (bx, by) = (ww - tw - 8.0, wh - m.height * 2.0 - 8.0);
                win.renderer.rect_top(bx - 1.0, by - 1.0, tw + 2.0, m.height * 2.0 + 2.0, rgba(theme.active_border, 1.0));
                win.renderer.rect_top(bx, by, tw, m.height * 2.0, rgba(theme.tab_bar_bg, 1.0));
                win.renderer.text(gpu, fonts, &msg, bx + m.width, by + m.height / 2.0, tw - m.width, rgba(theme.table[256], 1.0));
            } else {
                win.toast = None;
            }
        }
        win.renderer.present(gpu, bg_default);
    }

    fn handle_term_event(&mut self, el: &ActiveEventLoop, id: u64, ev: TermEvent) {
        let wid = self.pane_win.get(&id).copied();
        match ev {
            TermEvent::Wakeup => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.dirty = true;
                }
                if let Some(w) = wid.and_then(|w| self.windows.get(&w)) {
                    if w.pane_rects.iter().any(|(i, _)| *i == id) {
                        w.window.request_redraw();
                    }
                }
            }
            TermEvent::Title(t) => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.title = t;
                }
                if let Some(w) = wid.and_then(|w| self.windows.get(&w)) {
                    w.window.request_redraw();
                }
            }
            TermEvent::ResetTitle => {
                if let Some(p) = self.panes.get_mut(&id) {
                    p.title.clear();
                }
            }
            TermEvent::PtyWrite(s) => {
                if let Some(p) = self.panes.get(&id) {
                    p.write(s.into_bytes());
                }
            }
            TermEvent::ClipboardStore(ty, s) => {
                if let Some(path) = s.strip_prefix("dtem-file:") {
                    // `dtem-copy FILE` on a remote host
                    self.fetch_remote_file(id, path.trim().to_string());
                    return;
                }
                let primary = matches!(ty, alacritty_terminal::term::ClipboardType::Selection);
                let n = s.chars().count();
                self.clip_set(primary, s);
                if !primary && self.panes.get(&id).map(|p| p.remote.is_some()).unwrap_or(false) {
                    self.toast_pane(id, format!("Copied {n} chars from remote"));
                }
            }
            TermEvent::ClipboardLoad(ty, f) => {
                let primary = matches!(ty, alacritty_terminal::term::ClipboardType::Selection);
                let policy = self.cfg.clipboard_read.clone();
                if policy == "allow" || self.clip_read_always {
                    let text = self.clip_get(primary);
                    if let Some(p) = self.panes.get(&id) {
                        p.write(f(&text).into_bytes());
                    }
                } else if policy == "ask" {
                    if let Some(w) = wid {
                        self.pending_clip.insert(w, f);
                        if let Some(win) = self.windows.get_mut(&w) {
                            win.confirm = Some(Confirm::ClipRead(id));
                            win.window.request_redraw();
                        }
                    }
                }
            }
            TermEvent::ColorRequest(idx, f) => {
                if let Some(p) = self.panes.get(&id) {
                    let c = {
                        let t = p.term.lock();
                        t.colors()[idx].map(to_rgb).unwrap_or(self.theme.table[idx.min(268)])
                    };
                    p.write(f(alacritty_terminal::vte::ansi::Rgb { r: c[0], g: c[1], b: c[2] }).into_bytes());
                }
            }
            TermEvent::TextAreaSizeRequest(f) => {
                if let (Some(p), Some(w)) = (self.panes.get(&id), wid.and_then(|w| self.windows.get(&w))) {
                    let m = w.renderer.metrics;
                    let ws = WindowSize {
                        num_lines: p.rows as u16,
                        num_cols: p.cols as u16,
                        cell_width: m.width as u16,
                        cell_height: m.height as u16,
                    };
                    p.write(f(ws).into_bytes());
                }
            }
            TermEvent::Bell => {
                if let Some(w) = wid.and_then(|w| self.windows.get(&w)) {
                    if !w.focused {
                        w.window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
                    }
                }
            }
            TermEvent::ChildExit(_) | TermEvent::Exit => {
                if let Some(p) = self.panes.get_mut(&id) {
                    if !p.exited {
                        p.exited = true;
                        self.close_pane(el, id);
                    }
                }
            }
            _ => {}
        }
    }

    fn startup(&mut self, el: &ActiveEventLoop) {
        let Some(st) = self.startup.take() else { return };
        if st.restore {
            if let Some(s) = session::load(&session::session_path(self.role.name())) {
                self.open_session(el, &s, true);
            }
        }
        if let Some(f) = &st.session_file {
            self.open_session_file(el, f);
        }
        if let Some(args) = st.remote.clone() {
            self.open_remote(el, args);
        }
        match self.role {
            Role::Main => {
                if self.windows.is_empty() {
                    if let Some(w) = self.create_window(el, None, false) {
                        self.new_tab(w, None);
                    }
                }
            }
            Role::Dropdown => {
                let wid = match self.dropdown_id() {
                    Some(w) => w,
                    None => match self.create_window(el, None, true) {
                        Some(w) => {
                            self.new_tab(w, None);
                            w
                        }
                        None => return,
                    },
                };
                if st.show_dropdown {
                    self.show_dropdown(el, wid);
                }
                self.register_hotkey();
            }
        }
        if self.windows.is_empty() {
            el.exit();
            return;
        }
        self.save_session(true);
    }

    fn register_hotkey(&mut self) {
        // Real X11 session only; Wayland/XWayland uses `dtem --toggle` via a desktop shortcut.
        if std::env::var("XDG_SESSION_TYPE").map(|s| s != "x11").unwrap_or(false) || std::env::var_os("WAYLAND_DISPLAY").is_some() {
            log::info!("Wayland session: use `dtem --install-gnome-shortcut` (or bind `dtem --toggle`) for the hotkey");
            return;
        }
        let hk: global_hotkey::hotkey::HotKey = match crate::hotkey_from_config(&self.cfg.dropdown.hotkey) {
            Some(h) => h,
            None => {
                log::error!("invalid dropdown hotkey {:?}", self.cfg.dropdown.hotkey);
                return;
            }
        };
        match global_hotkey::GlobalHotKeyManager::new() {
            Ok(m) => {
                if let Err(e) = m.register(hk) {
                    log::warn!("cannot grab hotkey {} (already used by the desktop shortcut?): {e}", self.cfg.dropdown.hotkey);
                }
                let proxy = self.proxy.clone();
                let id = hk.id();
                global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |e: global_hotkey::GlobalHotKeyEvent| {
                    if e.id == id && e.state == global_hotkey::HotKeyState::Pressed {
                        let _ = proxy.send_event(UserEvent::Command("toggle".into()));
                    }
                }));
                self.hotkey = Some(m);
            }
            Err(e) => log::warn!("global hotkey unavailable: {e}"),
        }
    }
}

/// One visible cell of a row, waiting to be drawn.
struct RCell {
    col: usize,
    c: char,
    fg: [f32; 4],
    style: u8,
    wide: bool,
    zw: Vec<char>,
}

/// Draw a row's text. Runs of same-style/same-colour cells are shaped together so the
/// font's ligatures and contextual alternates apply.
#[allow(clippy::too_many_arguments)]
fn draw_row(r: &mut Renderer, gpu: &Gpu, fonts: &mut FontSet, cells: &[RCell], ox: f32, y: f32, ligatures: bool) {
    let m = r.metrics;
    let mut i = 0;
    while i < cells.len() {
        let c = &cells[i];
        let face = fonts.style_face(c.style);
        let shapeable = |x: &RCell| {
            !x.wide && x.zw.is_empty() && x.c != ' ' && x.c != '\0' && x.c.is_ascii_graphic()
        };
        if ligatures && shapeable(c) && fonts.face(face).glyph(c.c) != 0 {
            // extend the run
            let mut j = i + 1;
            while j < cells.len() {
                let n = &cells[j];
                if !shapeable(n) || n.style != c.style || n.fg != c.fg || n.col != cells[j - 1].col + 1 {
                    break;
                }
                j += 1;
            }
            let x0 = ox + c.col as f32 * m.width;
            if j - i == 1 {
                let gid = fonts.face(face).glyph(c.c);
                r.glyph_id(gpu, fonts, face, gid, 1, x0, y, c.fg);
            } else {
                let text: String = cells[i..j].iter().map(|x| x.c).collect();
                let shaped = fonts.shape(face, &text);
                for (gid, cell) in shaped.iter() {
                    r.glyph_id(gpu, fonts, face, *gid, 1, x0 + *cell as f32 * m.width, y, c.fg);
                }
            }
            i = j;
            continue;
        }
        let x = ox + c.col as f32 * m.width;
        let cells_w = if c.wide { 2 } else { 1 };
        r.glyph(gpu, fonts, c.c, c.style, cells_w, x, y, c.fg);
        for z in &c.zw {
            r.glyph(gpu, fonts, *z, c.style, cells_w, x, y, c.fg);
        }
        i += 1;
    }
}

fn expand_tilde(s: &str) -> PathBuf {
    if let Some(rest) = s.strip_prefix("~") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest.trim_start_matches('/'));
        }
    }
    PathBuf::from(s)
}

fn make_clipboard(window: &Window) -> Clip {
    if let Ok(h) = window.display_handle() {
        if let RawDisplayHandle::Wayland(w) = h.as_raw() {
            let (primary, clipboard) =
                unsafe { copypasta::wayland_clipboard::create_clipboards_from_external(w.display.as_ptr()) };
            return Clip { clipboard: Box::new(clipboard), primary: Box::new(primary) };
        }
    }
    use copypasta::x11_clipboard::{Clipboard as XC, Primary as XP, X11ClipboardContext};
    match (X11ClipboardContext::<XC>::new(), X11ClipboardContext::<XP>::new()) {
        (Ok(c), Ok(p)) => Clip { clipboard: Box::new(c), primary: Box::new(p) },
        _ => Clip {
            clipboard: Box::new(copypasta::nop_clipboard::NopClipboardContext::new().unwrap()),
            primary: Box::new(copypasta::nop_clipboard::NopClipboardContext::new().unwrap()),
        },
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        self.startup(el);
    }

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        if self.exiting {
            return;
        }
        match ev {
            UserEvent::Term(id, e) => self.handle_term_event(el, id, e),
            UserEvent::InsertText(pane, text) => self.paste_into(pane, &text),
            UserEvent::Toast(pane, msg) => self.toast_pane(pane, msg),
            UserEvent::RemoteFile(pane, name, data) => self.on_remote_file(pane, name, data),
            UserEvent::Redraw(w) => {
                if let Some(w) = self.windows.get(&w) {
                    w.window.request_redraw();
                }
            }
            UserEvent::Signal => {
                self.save_session(true);
                self.exiting = true;
                el.exit();
            }
            UserEvent::Command(c) => {
                let c = c.trim();
                match c {
                    "toggle" => self.toggle_dropdown(el),
                    "show" => {
                        if let Some(w) = self.dropdown_id() {
                            self.show_dropdown(el, w);
                        }
                    }
                    "new-window" => {
                        if let Some(w) = self.create_window(el, None, false) {
                            self.new_tab(w, None);
                            self.windows[&w].window.focus_window();
                        }
                    }
                    "quit" => {
                        self.save_session(true);
                        self.exiting = true;
                        el.exit();
                    }
                    _ => {
                        if let Some(p) = c.strip_prefix("session ") {
                            self.open_session_file(el, std::path::Path::new(p));
                        } else if let Some(j) = c.strip_prefix("remote ") {
                            if let Ok(args) = serde_json::from_str::<Vec<String>>(j) {
                                self.open_remote(el, args);
                            }
                        }
                    }
                }
            }
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, wid: WindowId, event: WindowEvent) {
        if self.exiting || !self.windows.contains_key(&wid) {
            return;
        }
        match event {
            WindowEvent::CloseRequested => {
                if self.windows[&wid].dropdown {
                    self.hide_dropdown(wid);
                } else {
                    self.request_close(el, wid, Confirm::Window);
                }
            }
            WindowEvent::Resized(size) => {
                let gpu = self.gpu.as_ref().unwrap();
                let win = self.windows.get_mut(&wid).unwrap();
                win.renderer.resize(gpu, size.width, size.height);
                if !win.window.is_maximized() && win.window.fullscreen().is_none() {
                    win.size = (size.width, size.height);
                }
                self.relayout(wid);
                self.mark_save_soon();
            }
            WindowEvent::Moved(pos) => {
                let win = self.windows.get_mut(&wid).unwrap();
                win.pos = Some((pos.x, pos.y));
                if win.dropdown && win.visible {
                    // Track the monitor the dropdown was moved to (e.g. Super+Shift+Arrow)
                    let size = win.window.outer_size();
                    let cx = pos.x + size.width as i32 / 2;
                    let cy = pos.y + 10;
                    if let Some(m) = Self::monitor_at(el, cx, cy) {
                        if m.name() != win.monitor {
                            win.monitor = m.name();
                            self.apply_dropdown_geometry(el, wid);
                        }
                    }
                }
                self.mark_save_soon();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let win = self.windows.get_mut(&wid).unwrap();
                win.scale = scale_factor;
                let fs = win.font_size;
                self.set_font_size(wid, fs);
            }
            WindowEvent::Focused(f) => {
                if f {
                    self.last_focused = Some(wid);
                }
                let win = self.windows.get_mut(&wid).unwrap();
                win.focused = f;
                win.window.request_redraw();
                let dropdown = win.dropdown;
                if let Some(id) = self.active_pane_id(wid) {
                    if let Some(p) = self.panes.get(&id) {
                        if p.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                            p.write(if f { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                        }
                    }
                }
                if dropdown && !f && self.cfg.dropdown.hide_on_focus_loss {
                    self.hide_dropdown(wid);
                }
            }
            WindowEvent::ModifiersChanged(m) => {
                self.modifiers = m.state();
                self.update_hover(wid);
            }
            WindowEvent::Ime(ime) => {
                use winit::event::Ime;
                let win = self.windows.get_mut(&wid).unwrap();
                match ime {
                    Ime::Preedit(text, _) => {
                        win.preedit = if text.is_empty() { None } else { Some(text) };
                        win.window.request_redraw();
                    }
                    Ime::Commit(text) => {
                        win.preedit = None;
                        win.window.request_redraw();
                        if let Some(p) = self.active_pane_id(wid).and_then(|id| self.panes.get(&id)) {
                            p.term.lock().scroll_display(Scroll::Bottom);
                            p.write(text.into_bytes());
                        }
                    }
                    Ime::Enabled | Ime::Disabled => {
                        win.preedit = None;
                    }
                }
            }
            WindowEvent::DroppedFile(path) => {
                let (x, y) = self.windows[&wid].cursor;
                let target = self.pane_at(wid, x, y).map(|(id, _)| id).or(self.active_pane_id(wid));
                if let Some(id) = target {
                    self.send_paths(wid, id, vec![path]);
                }
            }
            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => self.on_key(el, wid, event),
            WindowEvent::CursorMoved { position, .. } => self.on_cursor_moved(wid, position.x, position.y),
            WindowEvent::MouseInput { state, button, .. } => self.on_mouse_button(el, wid, state, button),
            WindowEvent::MouseWheel { delta, .. } => self.on_wheel(wid, delta),
            WindowEvent::RedrawRequested => self.render(wid),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.exiting {
            return;
        }
        let interval = Duration::from_secs(self.cfg.autosave_interval_secs.max(1));
        if self.last_save.elapsed() >= interval {
            self.save_session(false);
        }
        el.set_control_flow(ControlFlow::WaitUntil(self.last_save + interval));
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if !self.exiting {
            self.save_session(true);
        }
    }
}

// ---------------------------------------------------------------------- remote, clipboard, URLs
impl App {
    fn toast(&mut self, wid: WindowId, msg: impl Into<String>) {
        if let Some(w) = self.windows.get_mut(&wid) {
            w.toast = Some((msg.into(), Instant::now()));
            w.window.request_redraw();
            let proxy = self.proxy.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(4100));
                let _ = proxy.send_event(UserEvent::Redraw(wid));
            });
        }
    }

    fn toast_pane(&mut self, pane: u64, msg: impl Into<String>) {
        if let Some(w) = self.pane_win.get(&pane).copied() {
            self.toast(w, msg);
        }
    }

    /// Install `dtem-copy` on the remote once the multiplexed connection is up (once per host).
    fn ensure_remote_helpers(&mut self, r: &Remote) {
        if !self.cfg.remote_install_helpers || !self.helpers_done.insert(r.label()) {
            return;
        }
        let r = r.clone();
        std::thread::spawn(move || {
            for _ in 0..600 {
                if r.is_connected() {
                    match r.install_helpers() {
                        Ok(_) => log::info!("installed dtem-copy on {}", r.label()),
                        Err(e) => log::warn!("cannot install dtem-copy on {}: {e}", r.label()),
                    }
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
        });
    }

    /// Open `ssh <args>` in a new tab of the most recently focused window.
    fn open_remote(&mut self, el: &ActiveEventLoop, args: Vec<String>) {
        if remote::split_ssh_args(&args).is_none() {
            log::error!("--remote: no destination given");
            return;
        }
        let wid = self
            .last_focused
            .filter(|w| self.windows.get(w).map(|w| !w.dropdown || self.role == Role::Dropdown).unwrap_or(false))
            .or_else(|| self.order.iter().copied().find(|w| !self.windows[w].dropdown || self.role == Role::Dropdown));
        let wid = match wid {
            Some(w) => w,
            None => match self.create_window(el, None, self.role == Role::Dropdown) {
                Some(w) => w,
                None => return,
            },
        };
        let at = self.windows.get(&wid).map(|w| if w.tabs.is_empty() { 0 } else { w.active_tab + 1 });
        let ts = TabState {
            title: None,
            active: 0,
            layout: SNode::Leaf { cwd: None, command: None, scrollback: None, title: None, remote: Some(args) },
        };
        self.add_tab_from_state(wid, &ts, false, at);
        let w = &self.windows[&wid];
        if w.dropdown && !w.visible {
            self.show_dropdown(el, wid);
        } else {
            w.window.focus_window();
        }
        self.mark_save_soon();
    }

    fn rich_clip(&mut self) -> Option<&mut arboard::Clipboard> {
        if self.rich_clip.is_none() {
            self.rich_clip = arboard::Clipboard::new().map_err(|e| log::warn!("clipboard: {e}")).ok();
        }
        self.rich_clip.as_mut()
    }

    /// Paste text into a pane (bracketed paste aware).
    fn paste_into(&self, pane: u64, text: &str) {
        let Some(p) = self.panes.get(&pane) else { return };
        if text.is_empty() {
            return;
        }
        let bracketed = p.term.lock().mode().contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            let clean = text.replace('\x1b', "");
            p.write(format!("\x1b[200~{clean}\x1b[201~").into_bytes());
        } else {
            p.write(text.replace("\r\n", "\r").replace('\n', "\r").into_bytes());
        }
        let mut t = p.term.lock();
        t.scroll_display(Scroll::Bottom);
        t.selection = None;
    }

    /// Clipboard contains files or an image (no text)? Send them to the pane:
    /// uploaded to the remote for ssh panes, or as local paths.
    fn paste_rich(&mut self, wid: WindowId, pane: u64) -> bool {
        let files: Vec<PathBuf> = self
            .rich_clip()
            .and_then(|c| c.get().file_list().ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|p| p.is_file())
            .collect();
        if !files.is_empty() {
            self.send_paths(wid, pane, files);
            return true;
        }
        if !self.clip_get(false).is_empty() {
            return false;
        }
        let img = self.rich_clip().and_then(|c| c.get_image().ok());
        let Some(img) = img else { return false };
        let png = match encode_png(img.width as u32, img.height as u32, &img.bytes) {
            Ok(p) => p,
            Err(e) => {
                self.toast(wid, format!("Image encode failed: {e}"));
                return true;
            }
        };
        let name = format!("clipboard-{}.png", timestamp());
        let remote = self.panes.get(&pane).and_then(|p| p.remote_target());
        match remote {
            Some(r) => self.upload(wid, pane, r, vec![(name, png)], true),
            None => match stage_local_image(&name, &png) {
                // Same as a remote paste: the image path, nothing else (agents attach it as an image)
                Ok(path) => self.paste_into(pane, &path.display().to_string()),
                Err(e) => self.toast(wid, format!("Cannot save image: {e}")),
            },
        }
        true
    }

    fn send_paths(&mut self, wid: WindowId, pane: u64, paths: Vec<PathBuf>) {
        let remote = self.panes.get(&pane).and_then(|p| p.remote_target());
        match remote {
            Some(r) => {
                let mut items = Vec::new();
                for p in paths {
                    match std::fs::read(&p) {
                        Ok(d) => items.push((p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or("file".into()), d)),
                        Err(e) => self.toast(wid, format!("{}: {e}", p.display())),
                    }
                }
                if !items.is_empty() {
                    self.upload(wid, pane, r, items, false);
                }
            }
            None => {
                let text = paths.iter().map(|p| remote::shell_quote(&p.display().to_string())).collect::<Vec<_>>().join(" ");
                self.paste_into(pane, &format!("{text} "));
            }
        }
    }

    /// Upload in the background, then paste the remote path(s) into the pane.
    /// Clipboard images are pasted as the bare path (like a local paste) and stay quiet.
    fn upload(&mut self, wid: WindowId, pane: u64, r: Remote, items: Vec<(String, Vec<u8>)>, clipboard_image: bool) {
        let total: usize = items.iter().map(|(_, d)| d.len()).sum();
        if !clipboard_image || total > 2 << 20 {
            self.toast(wid, format!("Uploading {} file(s), {} to {}…", items.len(), human_size(total), r.label()));
        }
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let mut paths = Vec::new();
            for (name, data) in items {
                match r.upload(&data, &name, clipboard_image) {
                    Ok(p) => paths.push(remote::shell_quote(&p)),
                    Err(e) => {
                        let _ = proxy.send_event(UserEvent::Toast(pane, format!("Upload failed: {e}")));
                        return;
                    }
                }
            }
            let n = paths.len();
            if clipboard_image {
                let _ = proxy.send_event(UserEvent::InsertText(pane, paths.join(" ")));
            } else {
                let _ = proxy.send_event(UserEvent::InsertText(pane, format!("{} ", paths.join(" "))));
                let _ = proxy.send_event(UserEvent::Toast(pane, format!("Uploaded {n} file(s) to {}", r.label())));
            }
        });
    }

    /// `dtem-copy FILE` on the remote: fetch it and put it on the local clipboard.
    fn fetch_remote_file(&mut self, pane: u64, path: String) {
        let Some(r) = self.panes.get(&pane).and_then(|p| p.remote_target()) else {
            self.toast_pane(pane, "dtem-copy: pane is not an ssh session");
            return;
        };
        let proxy = self.proxy.clone();
        std::thread::spawn(move || match r.fetch(&path) {
            Ok(data) => {
                let name = std::path::Path::new(&path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or("file".into());
                let _ = proxy.send_event(UserEvent::RemoteFile(pane, name, data));
            }
            Err(e) => {
                let _ = proxy.send_event(UserEvent::Toast(pane, format!("Fetching {path} failed: {e}")));
            }
        });
    }

    fn on_remote_file(&mut self, pane: u64, name: String, data: Vec<u8>) {
        if let Ok(img) = image::load_from_memory(&data) {
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let res = self.rich_clip().map(|c| {
                c.set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: rgba.into_raw().into() })
            });
            match res {
                Some(Ok(_)) => self.toast_pane(pane, format!("Copied image {name} ({w}×{h}) to clipboard")),
                _ => self.toast_pane(pane, "Cannot set clipboard image"),
            }
            return;
        }
        // Other files: save under ~/Downloads/dtem and copy as a file (paste in the file manager)
        let dir = dirs::download_dir().unwrap_or_else(|| dirs::home_dir().unwrap_or_default()).join("dtem");
        let mut path = dir.join(&name);
        let mut i = 1;
        while path.exists() {
            path = dir.join(format!("{i}-{name}"));
            i += 1;
        }
        match std::fs::create_dir_all(&dir).and_then(|_| std::fs::write(&path, &data)) {
            Ok(_) => {
                let _ = self.rich_clip().map(|c| c.set().file_list(&[&path]));
                self.toast_pane(pane, format!("Saved {} ({}) and copied as file", path.display(), human_size(data.len())));
            }
            Err(e) => self.toast_pane(pane, format!("Cannot save {name}: {e}")),
        }
    }

    // ---- URLs

    /// URL (or OSC 8 hyperlink) at a grid point: (first col, last col, url)
    fn url_at(&self, pane: u64, point: Point) -> Option<(usize, usize, String)> {
        let p = self.panes.get(&pane)?;
        let term = p.term.lock();
        let grid = term.grid();
        let row = &grid[point.line];
        let cols = grid.columns();
        if let Some(h) = row[point.column].hyperlink() {
            let uri = h.uri().to_string();
            let same = |c: usize| row[Column(c)].hyperlink().map(|x| x.uri() == uri).unwrap_or(false);
            let mut a = point.column.0;
            while a > 0 && same(a - 1) {
                a -= 1;
            }
            let mut b = point.column.0;
            while b + 1 < cols && same(b + 1) {
                b += 1;
            }
            return Some((a, b, uri));
        }
        // plain-text URL detection on the row
        let mut text = String::new();
        let mut col_of = Vec::new(); // byte index -> col
        for c in 0..cols {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let ch = if cell.c == '\0' { ' ' } else { cell.c };
            for _ in 0..ch.len_utf8() {
                col_of.push(c);
            }
            text.push(ch);
        }
        for m in self.url_re.find_iter(&text) {
            let mut url = m.as_str();
            // trim trailing punctuation, keep ')' when the URL has a matching '('
            while let Some(last) = url.chars().last() {
                let unbalanced_paren = last == ')' && url.matches('(').count() < url.matches(')').count();
                if ".,;:!?'\"]".contains(last) || unbalanced_paren {
                    url = &url[..url.len() - last.len_utf8()];
                } else {
                    break;
                }
            }
            if url.is_empty() {
                continue;
            }
            let (a, b) = (col_of[m.start()], col_of[m.start() + url.len() - 1]);
            if (a..=b).contains(&point.column.0) {
                let url = if url.starts_with("www.") { format!("https://{url}") } else { url.to_string() };
                return Some((a, b, url));
            }
        }
        None
    }

    fn update_hover(&mut self, wid: WindowId) {
        let Some(win) = self.windows.get(&wid) else { return };
        let (x, y) = win.cursor;
        let mut hover = None;
        if self.modifiers.control_key() {
            if let Some((id, rect)) = self.pane_at(wid, x, y) {
                let (point, _, _, _) = self.cell_at(wid, id, rect, x, y);
                hover = self.url_at(id, point).map(|(a, b, u)| (id, point.line.0, a, b, u));
            }
        }
        let win = self.windows.get_mut(&wid).unwrap();
        let changed = win.hover_url.as_ref().map(|h| (h.0, h.1, h.2)) != hover.as_ref().map(|h| (h.0, h.1, h.2));
        let pointer = hover.is_some();
        win.hover_url = hover;
        if pointer != win.pointer_cursor {
            win.pointer_cursor = pointer;
            win.window.set_cursor(if pointer { winit::window::CursorIcon::Pointer } else { winit::window::CursorIcon::Text });
        }
        if changed {
            win.window.request_redraw();
        }
    }

    fn open_url(&self, url: &str) {
        let mut parts = self.cfg.open_url_with.split_whitespace();
        let Some(prog) = parts.next() else { return };
        match std::process::Command::new(prog).args(parts).arg(url).spawn() {
            Ok(mut child) => {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            Err(e) => log::error!("cannot open {url}: {e}"),
        }
    }
}

/// Write a pasted image to a private temp dir (0700/0600), dropping files older than 24h.
fn stage_local_image(name: &str, data: &[u8]) -> std::io::Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let dir = std::env::temp_dir().join(format!("dtem-clipboard-{}", unsafe { libc::geteuid() }));
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let old = e.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().unwrap_or_default());
            if old.map(|d| d > Duration::from_secs(24 * 3600)).unwrap_or(false) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let path = dir.join(name);
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path)?;
    f.write_all(data)?;
    Ok(path)
}

fn timestamp() -> String {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{}{:03}", d.as_secs(), d.subsec_millis())
}

fn human_size(n: usize) -> String {
    match n {
        n if n >= 1 << 20 => format!("{:.1} MB", n as f64 / (1 << 20) as f64),
        n if n >= 1 << 10 => format!("{:.0} KB", n as f64 / 1024.0),
        n => format!("{n} B"),
    }
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> anyhow::Result<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec()).ok_or_else(|| anyhow::anyhow!("bad image data"))?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}
