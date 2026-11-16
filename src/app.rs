//! Application: OS windows, tabs, split panes, input, rendering, dropdown & session handling.
mod extras;
mod stash;
mod workspace;
use workspace::Workspace;
use extras::{hint_regexes, HintKind, Hints, HistSearch};
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
    Quit,
    QuitNoSave,
    /// The shell of this pane exited by itself
    Exited(u64),
    /// Stash a tab in which a program is running (it will end)
    Stash(usize),
    /// Close workspace i with all its tabs
    Workspace(usize),
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
    cursor_icon: winit::window::CursorIcon,
    ime_area: Option<(i32, i32)>,
    /// When the window last lost focus (the dropdown hotkey itself steals focus)
    unfocused_at: Option<Instant>,
    search: Option<SearchState>,
    scroll_drag: Option<u64>,
    /// Index of the tab being dragged in the tab bar (left button held)
    tab_drag: Option<usize>,
    /// Last left click on a tab (double-click renames)
    tab_last_click: Option<(Instant, usize)>,
    /// Stash browser overlay
    stash: Option<stash::StashView>,
    /// Stash button at the end of the tab bar
    stash_btn: Option<Rect>,
    /// All workspaces; the current one's tabs are `tabs` (see workspace.rs)
    workspaces: Vec<Workspace>,
    ws_cur: usize,
    /// Workspace button at the start of the tab bar
    ws_btn: Option<Rect>,
    /// The rename prompt edits the workspace name instead of the tab title
    rename_ws: bool,
    bell_at: Option<Instant>,
    menu: Option<Menu>,
    hints: Option<Hints>,
    hist: Option<HistSearch>,
    /// Dropdown: fullscreen state when it was last hidden (restored on show)
    was_fullscreen: bool,
    /// Moves caused by our own show/fullscreen requests are not "the user moved it"
    ignore_moves_until: Option<Instant>,
    /// Re-apply the dropdown geometry after the WM restored it (leaving fullscreen)
    reapply_geometry_at: Option<Instant>,
    /// Go fullscreen again (on the remembered monitor) at this time — used to move a
    /// fullscreen dropdown to another monitor
    refullscreen_at: Option<Instant>,
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
    keycfg: keys::KeyCfg,
    transparency_on: bool,
    x11: Option<crate::x11::X11>,
    last_toggle: Option<Instant>,
    anim: Option<Anim>,
    pending_hide: Option<(WindowId, Instant)>,
    last_input: Instant,
    discard_session: bool,
    installed_hotkeys: Vec<String>,
    /// Dropdown profile ("" = default, otherwise a key of [dropdowns])
    profile: String,
    hint_res: Vec<(HintKind, regex::Regex)>,
    /// global hotkey id → dropdown profile it toggles
    hotkey_targets: Arc<parking_lot::Mutex<HashMap<u32, String>>>,
    /// history file the next spawned pane copies (the pane a new tab/split was opened from)
    seed_history: Option<PathBuf>,
    /// touchpad ctrl+scroll accumulator (pixels)
    zoom_acc: f64,
    last_wheel: Option<(Instant, bool)>,
    /// Number of stashed tabs (shared by all instances; refreshed on focus)
    stash_count: usize,
    stash_last_click: Option<(Instant, usize)>,
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
    pub fn new(cfg: Config, role: Role, profile: String, proxy: EventLoopProxy<UserEvent>, startup: Startup) -> anyhow::Result<App> {
        let fonts = FontSet::new(&cfg.font_family)?;
        Ok(App {
            bindings: Bindings::new(&cfg),
            theme: profile_theme(&cfg, role, &profile),
            keycfg: keys::KeyCfg::from_config(&cfg),
            transparency_on: cfg.transparency,
            url_re: build_url_regex(&cfg),
            x11: None,
            last_toggle: None,
            anim: None,
            pending_hide: None,
            last_input: Instant::now(),
            discard_session: false,
            installed_hotkeys: Vec::new(),
            profile: profile.clone(),
            hint_res: hint_regexes(),
            hotkey_targets: Default::default(),
            seed_history: None,

            zoom_acc: 0.0,
            last_wheel: None,
            stash_count: stash::count(),
            stash_last_click: None,
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
            exiting: false,
        })
    }

    /// Name used for the session file and IPC socket.
    pub(super) fn rname(&self) -> String {
        match (self.role, self.profile.as_str()) {
            (Role::Main, _) => "main".into(),
            (Role::Dropdown, "") => "dropdown".into(),
            (Role::Dropdown, p) => format!("dropdown-{p}"),
        }
    }

    /// This dropdown's settings.
    pub(super) fn dd(&self) -> crate::config::Dropdown {
        self.cfg.dropdown_profile(&self.profile)
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
        let d = self.dd();
        let w = ((s.width as f32) * d.width.clamp(0.1, 1.0)) as u32;
        let h = ((s.height as f32) * d.height.clamp(0.1, 1.0)) as u32;
        let x = if d.centered_horizontally { (s.width as i32 - w as i32) / 2 } else { d.x.min(s.width as i32 - w as i32).max(0) };
        let y = if d.centered_vertically { (s.height as i32 - h as i32) / 2 } else { d.y.min(s.height as i32 - h as i32).max(0) };
        (PhysicalPosition::new(p.x + x, p.y + y), PhysicalSize::new(w, h))
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
        let mut monitor_name = ws
            .and_then(|w| w.monitor.clone())
            .or_else(|| Some(self.dd().monitor.clone()).filter(|m| dropdown && !m.is_empty()));
        if dropdown {
            let mon = Self::monitor_by_name(el, monitor_name.as_deref()).or_else(|| Self::fallback_monitor(el));
            if let Some(m) = &mon {
                let (pos, size) = self.dropdown_geometry(m);
                attrs = attrs.with_position(pos).with_inner_size(size);
                monitor_name = m.name();
            }
            attrs = attrs.with_decorations(false).with_visible(false);
            if self.dd().always_on_top {
                attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
            }
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
        // Always request an alpha-capable window so transparency can be toggled live.
        attrs = attrs.with_transparent(true);
        let window = match el.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("cannot create window: {e}");
                return None;
            }
        };
        window.set_ime_allowed(self.cfg.ime);
        if dropdown {
            if self.x11.is_none() {
                self.x11 = crate::x11::X11::connect();
            }
            if let (Some(x), Some(id)) = (&self.x11, crate::x11::window_id(&window)) {
                let d = self.dd();
                x.preset(id, d.skip_taskbar, d.all_workspaces, d.always_on_top);
            }
        }
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
        let base_font = if dropdown { self.dd().font_size.unwrap_or(self.cfg.font_size) } else { self.cfg.font_size };
        let font_size = ws.and_then(|w| w.font_size).unwrap_or(base_font);
        let metrics = font::cell_metrics(&self.fonts, pt_to_px(font_size, scale), self.cfg.line_height);
        let size = window.inner_size();
        let renderer = Renderer::new(
            self.gpu.as_ref().unwrap(),
            surface,
            size.width,
            size.height,
            metrics,
            self.cfg.effective_opacity(self.transparency_on),
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
                cursor_icon: winit::window::CursorIcon::Text,
                ime_area: None,
                unfocused_at: None,
                search: None,
                scroll_drag: None,
                tab_drag: None,
                tab_last_click: None,
                stash: None,
                stash_btn: None,
                workspaces: vec![Workspace::new(workspace::DEFAULT_NAME)],
                ws_cur: 0,
                ws_btn: None,
                rename_ws: false,
                bell_at: None,
                menu: None,
                hints: None,
                hist: None,
                ignore_moves_until: None,
                reapply_geometry_at: None,
                refullscreen_at: None,
                was_fullscreen: dropdown && (ws.map(|w| w.fullscreen).unwrap_or(false) || self.dd().start_fullscreen),
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
        if self.cfg.tab_bar_position != "hidden" && (win.tabs.len() >= self.cfg.tab_bar_min_tabs.max(1) || win.ws_visible()) {
            (win.renderer.metrics.height + 6.0 * win.scale as f32).round()
        } else {
            0.0
        }
    }

    fn content_rect(&self, win: &OsWin) -> Rect {
        let (w, h) = win.renderer.size();
        let tb = self.tab_bar_height(win);
        let y = if self.cfg.tab_bar_position == "bottom" { 0.0 } else { tb };
        Rect { x: 0.0, y, w, h: (h - tb).max(1.0) }
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

    fn spawn_pane(&mut self, wid: WindowId, id: u64, mut opts: SpawnOpts, rect: Rect) -> bool {
        if opts.history_seed.is_none() {
            opts.history_seed = self.seed_history.take();
        }
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
                SNode::Leaf { cwd, command, scrollback, title, remote, remote_cwd } => {
                    let id = app.alloc_id();
                    let remote = remote.clone().map(|a| Remote { cwd: remote_cwd.clone(), ..Remote::new_multiplexed(a) });
                    let program = remote.as_ref().map(|r| {
                        // restored remote panes ask before reconnecting (programs aren't restarted)
                        ("/bin/sh".to_string(), vec!["-c".to_string(), r.pane_script(restore)])
                    });
                    if let Some(r) = &remote {
                        app.ensure_remote_helpers(r);
                    }
                    let restore_bytes = if restore && app.cfg.persist_scrollback {
                        scrollback.as_ref().and_then(|s| session::read_scrollback(&app.rname(), s))
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
                            history_seed: None,
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
        let after = self.cfg.new_tab_after_current;
        let at = self.windows.get(&wid).map(|w| if w.tabs.is_empty() { 0 } else if after { w.active_tab + 1 } else { w.tabs.len() });
        let cwd = cwd.filter(|_| self.cfg.inherit_working_dir).or_else(|| {
            Some(self.cfg.working_dir.trim()).filter(|d| !d.is_empty()).map(expand_tilde)
        });
        let command = Some(self.cfg.command.trim().to_string()).filter(|c| !c.is_empty());
        let ts = TabState {
            title: None,
            active: 0,
            layout: SNode::Leaf {
                cwd: cwd.map(|p| p.display().to_string()),
                command,
                scrollback: None,
                title: None,
                remote: None,
                remote_cwd: None,
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
            SpawnOpts { cwd, command: None, restore: None, persist_id: None, title: None, program: None, remote: None, history_seed: None };
        if !self.spawn_pane(wid, id, opts, r) {
            self.remove_leaf(wid, id);
        }
        self.relayout(wid);
    }

    /// Remove a pane id from its tab's tree (does not touch the process).
    fn remove_leaf(&mut self, wid: WindowId, id: u64) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let Some(loc) = win.find_pane(id) else { return };
        if loc.ws != win.ws_cur {
            // a pane in a workspace you aren't looking at (e.g. its shell exited)
            let ws = &mut win.workspaces[loc.ws];
            let tab = &mut ws.tabs[loc.tab];
            let root = std::mem::replace(&mut tab.root, Node::Leaf(0));
            match root.remove(id) {
                Some(r) => {
                    tab.root = r;
                    if tab.active == id {
                        tab.active = tab.root.leaf_list()[0];
                        tab.zoomed = false;
                    }
                }
                None => {
                    ws.tabs.remove(loc.tab);
                    if loc.tab < ws.active_tab {
                        ws.active_tab -= 1;
                    }
                    if ws.active_tab >= ws.tabs.len() {
                        ws.active_tab = ws.tabs.len().saturating_sub(1);
                    }
                    if ws.tabs.is_empty() {
                        win.workspaces.remove(loc.ws);
                        if win.ws_cur > loc.ws {
                            win.ws_cur -= 1;
                        }
                    }
                }
            }
            return;
        }
        let ti = loc.tab;
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
        // last tab of a workspace closed while other workspaces exist: show another one
        if self.windows.get(&wid).map(|w| w.tabs.is_empty()).unwrap_or(false) && self.drop_empty_current_workspace(wid, None) {
            self.relayout(wid);
            self.mark_save_soon();
            return;
        }
        let Some(win) = self.windows.get(&wid) else { return };
        if win.tabs.is_empty() {
            if win.dropdown {
                match self.dd().on_last_tab_exit.as_str() {
                    "quit" => {
                        self.save_session(true);
                        self.exiting = true;
                        el.exit();
                        return;
                    }
                    "new_tab" => self.new_tab(wid, None),
                    _ => {
                        self.new_tab(wid, None);
                        self.hide_dropdown(wid);
                    }
                }
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
            for t in win.all_tabs() {
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
            Confirm::Window | Confirm::Quit | Confirm::QuitNoSave => cc.window,
            Confirm::ClipRead(_) | Confirm::Exited(_) | Confirm::Stash(_) | Confirm::Workspace(_) => true,
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
            Confirm::Stash(i) => self.stash_tab(el, wid, i),
            Confirm::Workspace(i) => self.close_workspace(el, wid, i),
            Confirm::Pane(id) => self.close_pane(el, id),
            Confirm::ClipRead(id) => {
                if let Some(f) = self.pending_clip.remove(&wid) {
                    let text = self.clip_get(false);
                    if let Some(p) = self.panes.get(&id) {
                        p.write(f(&text).into_bytes());
                    }
                }
            }
            Confirm::Quit => self.quit(el, false),
            Confirm::QuitNoSave => self.quit(el, true),
            Confirm::Exited(id) => self.close_pane(el, id),
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

    /// Quit the whole instance. `discard`: forget the session (next start is fresh).
    fn quit(&mut self, el: &ActiveEventLoop, discard: bool) {
        if discard {
            let role_name = self.rname();
        let role = role_name.as_str();
            let _ = std::fs::remove_file(session::session_path(role));
            let _ = std::fs::remove_dir_all(session::scrollback_dir(role));
            self.discard_session = true;
        } else {
            self.save_session(true);
        }
        self.exiting = true;
        el.exit();
    }

    /// Confirmations that offer "quit without saving".
    fn confirm_is_quit(&self, c: Confirm) -> bool {
        match c {
            Confirm::Quit => true,
            Confirm::Window => self.role == Role::Main && self.windows.values().filter(|w| !w.dropdown).count() == 1,
            _ => false,
        }
    }

    fn confirm_text(&self, wid: WindowId, c: Confirm) -> String {
        let win = &self.windows[&wid];
        let tabs_all: usize = self.windows.values().map(|w| w.all_tabs().count()).sum();
        match c {
            Confirm::Quit => format!("Quit dtem? ({tabs_all} tabs — restored on next start)"),
            Confirm::QuitNoSave => "Quit and FORGET the session? (next start is empty)".to_string(),
            Confirm::Exited(id) => {
                let code = self.panes.get(&id).and_then(|p| p.exit_code);
                match code {
                    Some(c) if c != 0 => format!("The shell exited (code {c}). Close this pane?"),
                    _ => "The shell exited. Close this pane?".to_string(),
                }
            }
            Confirm::Tab(i) => {
                let n = win.tabs.get(i).map(|t| t.root.leaf_list().len()).unwrap_or(1);
                if n > 1 {
                    format!("Close tab {} ({n} panes)?", i + 1)
                } else {
                    format!("Close tab {}?", i + 1)
                }
            }
            Confirm::Pane(_) => "Close this pane?".to_string(),
            Confirm::Workspace(i) => {
                let name = if i == win.ws_cur { win.ws_name().to_string() } else { win.workspaces.get(i).map(|w| w.name.clone()).unwrap_or_default() };
                let n = win.ws_tabs(i).len();
                format!("Close workspace “{name}” and its {n} tab{}?", if n == 1 { "" } else { "s" })
            }
            Confirm::Stash(i) => {
                let progs: Vec<String> = win
                    .tabs
                    .get(i)
                    .map(|t| t.root.leaf_list())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|id| self.panes.get(id))
                    .filter(|p| p.remote.is_none() && !p.shell_idle())
                    .filter_map(|p| p.foreground_name())
                    .collect();
                let what = if progs.is_empty() { "A program is running".to_string() } else { format!("{} is running", progs.join(", ")) };
                format!("{what} in this tab and will end. Stash it anyway?")
            }
            Confirm::ClipRead(id) => {
                let who = self.panes.get(&id).and_then(|p| p.remote.as_ref().map(|r| r.label())).unwrap_or("A program".into());
                format!("{who} wants to read your clipboard.")
            }
            Confirm::Window => {
                let tabs = win.all_tabs().count();
                let panes: usize = win.all_tabs().map(|t| t.root.leaf_list().len()).sum();
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
        if self.windows.get(&wid).map(|w| w.window.fullscreen().is_some()).unwrap_or(false) {
            return;
        }
        let name = self.windows.get(&wid).and_then(|w| w.monitor.clone());
        let Some(mon) = Self::monitor_by_name(el, name.as_deref()).or_else(|| Self::fallback_monitor(el)) else { return };
        let (pos, size) = self.dropdown_geometry(&mon);
        let win = self.windows.get_mut(&wid).unwrap();
        win.monitor = mon.name();
        win.window.set_outer_position(pos);
        let _ = win.window.request_inner_size(size);
    }

    fn show_dropdown(&mut self, el: &ActiveEventLoop, wid: WindowId) {
        self.pending_hide = None;
        if let Some(w) = self.windows.get_mut(&wid) {
            w.ignore_moves_until = Some(Instant::now() + Duration::from_millis(800));
        }
        self.apply_dropdown_geometry(el, wid);
        let d = self.dd();
        let name = self.windows.get(&wid).and_then(|w| w.monitor.clone());
        let mon = Self::monitor_by_name(el, name.as_deref()).or_else(|| Self::fallback_monitor(el));
        let target = mon.as_ref().map(|m| self.dropdown_geometry(m));
        let win = self.windows.get_mut(&wid).unwrap();
        let fullscreen = win.was_fullscreen;
        if let (true, false, Some((pos, size)), Some(m)) = (d.animation, fullscreen, target, mon.as_ref()) {
            let from = offscreen(&d.animation_from, m, pos, size);
            win.window.set_outer_position(from);
            self.anim = Some(Anim {
                wid,
                start: Instant::now(),
                dur: Duration::from_millis(d.animation_duration_ms.max(1)),
                from: (from.x, from.y),
                to: (pos.x, pos.y),
                hide: false,
            });
        }
        win.window.set_visible(true);
        if d.always_on_top {
            win.window.set_window_level(WindowLevel::AlwaysOnTop);
        }
        win.visible = true;
        win.window.request_redraw();
        if self.anim.is_none() && !fullscreen {
            // Some WMs ignore the position while the window is unmapped: re-apply after mapping.
            self.apply_dropdown_geometry(el, wid);
        }
        let win = &self.windows[&wid];
        if let (Some(x), Some(id)) = (&self.x11, crate::x11::window_id(&win.window)) {
            x.apply(id, d.skip_taskbar, d.all_workspaces, d.always_on_top);
            if d.grab_focus {
                x.activate(id);
            }
        }
        if d.grab_focus {
            win.window.focus_window();
        }
        if fullscreen {
            self.set_fullscreen(wid, true, mon.clone());
        }
    }

    /// Enter/leave fullscreen (the WM's real fullscreen, on the window's monitor).
    /// `mon`: monitor to go fullscreen on (dropdowns pass their remembered monitor — the
    /// window's "current monitor" can be stale right after it was shown).
    fn set_fullscreen(&mut self, wid: WindowId, on: bool, mon: Option<MonitorHandle>) {
        let Some(w) = self.windows.get_mut(&wid) else { return };
        w.ignore_moves_until = Some(Instant::now() + Duration::from_millis(800));
        if on {
            let mon = mon.or_else(|| w.window.current_monitor());
            w.window.set_fullscreen(Some(Fullscreen::Borderless(mon)));
        } else {
            w.window.set_fullscreen(None);
            if w.dropdown {
                // the WM restores the pre-fullscreen geometry asynchronously; put it back after
                w.reapply_geometry_at = Some(Instant::now() + Duration::from_millis(250));
            }
        }
    }

    /// The dropdown's remembered monitor (or the primary one).
    fn dropdown_monitor(&self, el: &ActiveEventLoop, wid: WindowId) -> Option<MonitorHandle> {
        let name = self.windows.get(&wid).and_then(|w| w.monitor.clone());
        Self::monitor_by_name(el, name.as_deref()).or_else(|| Self::fallback_monitor(el))
    }

    fn hide_dropdown(&mut self, wid: WindowId) {
        self.pending_hide = None;
        let d = self.dd();
        let Some(win) = self.windows.get_mut(&wid) else { return };
        if win.visible {
            win.was_fullscreen = win.window.fullscreen().is_some() || win.refullscreen_at.take().is_some();
            if win.was_fullscreen {
                // leave fullscreen before unmapping so the WM restores it cleanly next time
                self.set_fullscreen(wid, false, None);
                let win = self.windows.get_mut(&wid).unwrap();
                win.window.set_visible(false);
                win.visible = false;
                return;
            }
        }
        if d.animation && win.visible && self.anim.is_none() {
            if let (Ok(pos), Some(m)) = (win.window.outer_position(), win.window.current_monitor()) {
                let size = win.window.outer_size();
                let to = offscreen(&d.animation_from, &m, pos, size);
                self.anim = Some(Anim {
                    wid,
                    start: Instant::now(),
                    dur: Duration::from_millis(d.animation_duration_ms.max(1)),
                    from: (pos.x, pos.y),
                    to: (to.x, to.y),
                    hide: true,
                });
                win.visible = false;
                return;
            }
        }
        win.window.set_visible(false);
        win.visible = false;
    }

    /// Advance the slide animation; returns the next wake-up time while running.
    fn step_anim(&mut self, el: &ActiveEventLoop) -> Option<Instant> {
        let a = self.anim.as_ref()?;
        let t = (a.start.elapsed().as_secs_f32() / a.dur.as_secs_f32()).min(1.0);
        let e = 1.0 - (1.0 - t).powi(3); // ease-out cubic
        let x = a.from.0 + ((a.to.0 - a.from.0) as f32 * e).round() as i32;
        let y = a.from.1 + ((a.to.1 - a.from.1) as f32 * e).round() as i32;
        let (wid, hide) = (a.wid, a.hide);
        if let Some(w) = self.windows.get(&wid) {
            w.window.set_outer_position(PhysicalPosition::new(x, y));
        }
        if t >= 1.0 {
            self.anim = None;
            if hide {
                if let Some(w) = self.windows.get(&wid) {
                    w.window.set_visible(false);
                }
            } else {
                self.apply_dropdown_geometry(el, wid);
            }
            return None;
        }
        Some(Instant::now() + Duration::from_millis(8))
    }

    fn toggle_dropdown(&mut self, el: &ActiveEventLoop) {
        // Like tilda: ignore toggles closer than 150 ms (key repeat / double triggers).
        if self.last_toggle.map(|t| t.elapsed() < Duration::from_millis(150)).unwrap_or(false) {
            return;
        }
        self.last_toggle = Some(Instant::now());
        let wid = match self.dropdown_id() {
            Some(w) => w,
            None => {
                let Some(w) = self.create_window(el, None, true) else { return };
                self.new_tab(w, None);
                w
            }
        };
        let win = &self.windows[&wid];
        // Pressing the global hotkey makes the WM grab the keyboard, so the dropdown loses
        // focus right before our toggle arrives. Treat such a recent loss as "focused".
        let lost_to_hotkey = win.unfocused_at.map(|t| t.elapsed() < Duration::from_millis(700)).unwrap_or(false);
        let recently_tried = self.focus_try.map(|t| t.elapsed() < Duration::from_secs(3)).unwrap_or(false);
        let hide_unfocused = self.dd().unfocused_toggle == "hide";
        if !win.visible {
            self.focus_try = None;
            self.show_dropdown(el, wid);
        } else if win.focused || lost_to_hotkey || recently_tried || hide_unfocused {
            self.focus_try = None;
            self.hide_dropdown(wid);
        } else {
            // Visible behind other windows: bring it to the front first.
            if let (Some(x), Some(id)) = (&self.x11, crate::x11::window_id(&win.window)) {
                x.activate(id);
            }
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
        if self.windows[&wid].window.fullscreen().is_some() {
            // leave fullscreen, move to the new monitor (geometry re-apply), then fullscreen there
            self.set_fullscreen(wid, false, None);
            let w = self.windows.get_mut(&wid).unwrap();
            w.reapply_geometry_at = Some(Instant::now() + Duration::from_millis(120));
            w.refullscreen_at = Some(Instant::now() + Duration::from_millis(300));
        } else {
            self.apply_dropdown_geometry(el, wid);
        }
        self.mark_save_soon();
    }

    // ------------------------------------------------------------------ session

    fn snapshot(&mut self) -> Session {
        let mut windows = Vec::new();
        let order = self.order.clone();
        for wid in order {
            let Some(win) = self.windows.get(&wid) else { continue };
            let with_ws = workspace::snapshot_needed(win);
            // (name, active tab, tabs) per workspace
            let ws_info: Vec<(String, usize, Vec<(Node, u64, Option<String>)>)> = (0..win.workspaces.len())
                .map(|i| {
                    let name = if i == win.ws_cur { win.ws_name().to_string() } else { win.workspaces[i].name.clone() };
                    let active = if i == win.ws_cur { win.active_tab } else { win.workspaces[i].active_tab };
                    (name, active, win.ws_tabs(i).iter().map(|t| (t.root.clone(), t.active, t.title.clone())).collect())
                })
                .collect();
            let ws_cur = win.ws_cur;
            let mut workspaces = Vec::new();
            for (name, active_tab, tab_info) in ws_info {
                let mut tabs = Vec::new();
                for (root, active, title) in tab_info {
                    let leaves = root.leaf_list();
                    if leaves.iter().all(|id| self.panes.get(id).map(|p| p.settings).unwrap_or(false)) {
                        continue; // settings page tabs aren't part of the session
                    }
                    let layout = self.snode(&root);
                    tabs.push(TabState { title, active: leaves.iter().position(|x| *x == active).unwrap_or(0), layout });
                }
                workspaces.push(session::WorkspaceState { name, active_tab: active_tab.min(tabs.len().saturating_sub(1)), tabs });
            }
            // flattened list for older versions (and for the single-workspace format)
            let offset: usize = workspaces.iter().take(ws_cur).map(|w| w.tabs.len()).sum();
            let flat_active = offset + workspaces.get(ws_cur).map(|w| w.active_tab).unwrap_or(0);
            let tabs: Vec<TabState> = workspaces.iter().flat_map(|w| w.tabs.clone()).collect();
            let (workspaces, active_workspace) = if with_ws { (workspaces, ws_cur) } else { (Vec::new(), 0) };
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
                fullscreen: if win.dropdown && !win.visible { win.was_fullscreen } else { win.window.fullscreen().is_some() },
                monitor,
                font_size: Some(win.font_size),
                active_tab: flat_active,
                tabs,
                workspaces,
                active_workspace,
            });
        }
        Session { version: 1, windows }
    }

    fn snode(&mut self, n: &Node) -> SNode {
        match n {
            Node::Leaf(id) => {
                let (cwd, sb, remote, remote_cwd) = match self.panes.get_mut(id) {
                    Some(p) => {
                        // a pane running our multiplexed ssh (inline `dtem --remote`) is saved
                        // as a remote pane, so it offers to reconnect after a restart
                        let r = p.remote.clone().or_else(|| p.remote_target().filter(|r| r.multiplexed));
                        let rc = if r.is_some() { p.remote_cwd.clone() } else { None };
                        (p.cwd().map(|c| c.display().to_string()), Some(p.persist_id.clone()), r.map(|r| r.args), rc)
                    }
                    None => (None, None, None, None),
                };
                SNode::Leaf { cwd, command: None, scrollback: sb, title: None, remote, remote_cwd }
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
        let role_name = self.rname();
        let role = role_name.as_str();
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
            self.gc_history();
        }
    }

    /// Remove per-tab history files that no pane of any dtem instance uses anymore.
    fn gc_history(&self) {
        let dir = crate::config::state_dir().join("history");
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        // ids referenced by any saved session (other dropdowns/windows too)
        let mut used: Vec<String> = self.panes.values().map(|p| p.persist_id.clone()).collect();
        // stashed tabs keep their history until they are restored or deleted
        used.extend(stash::referenced_ids());
        if let Ok(sd) = std::fs::read_dir(crate::config::state_dir()) {
            for e in sd.flatten() {
                let n = e.file_name().to_string_lossy().to_string();
                if n.starts_with("session-") && n.ends_with(".json") {
                    if let Ok(t) = std::fs::read_to_string(e.path()) {
                        for part in t.split("\"scrollback\"").skip(1) {
                            if let Some(id) = part.split('"').nth(1) {
                                used.push(id.to_string());
                            }
                        }
                    }
                }
            }
        }
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some(id) = name.strip_suffix(".hist") else { continue };
            let old = e.metadata().and_then(|m| m.modified()).map(|t| t.elapsed().unwrap_or_default()).unwrap_or_default();
            // grace period: a pane may have just been created by another instance
            if !used.iter().any(|u| u == id) && old > Duration::from_secs(3600) {
                let _ = std::fs::remove_file(e.path());
            }
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
            if !ws.has_tabs() {
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
            self.restore_workspaces(wid, ws, restore);
            if self.windows.get(&wid).map(|w| w.tabs.is_empty()).unwrap_or(true) {
                continue;
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
                win.rename_ws = false;
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
                let w = &self.windows[&wid];
                let on = w.window.fullscreen().is_none();
                let mon = if w.dropdown { self.dropdown_monitor(el, wid) } else { None };
                self.set_fullscreen(wid, on, mon);
                if !on && self.windows[&wid].dropdown {
                    self.apply_dropdown_geometry(el, wid);
                }
            }
            Action::SaveSessionFile => self.export_session_file(),
            Action::ReloadConfig => {
                self.cfg = Config::load();
                self.bindings = Bindings::new(&self.cfg);
                self.theme = profile_theme(&self.cfg, self.role, &self.profile);
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
            Action::ToggleTransparency => {
                self.transparency_on = !self.transparency_on;
                let op = self.cfg.effective_opacity(self.transparency_on);
                for w in self.windows.values_mut() {
                    w.renderer.set_opacity(op);
                    w.window.request_redraw();
                }
            }
            Action::Search => {
                if let Some(id) = active {
                    let win = self.windows.get_mut(&wid).unwrap();
                    win.search = Some(SearchState { pane: id, query: String::new(), regex: None, current: None });
                    win.window.request_redraw();
                }
            }
            Action::Quit => self.request_close(el, wid, Confirm::Quit),
            Action::QuitWithoutSaving => self.request_close(el, wid, Confirm::QuitNoSave),
            Action::OpenSettings => self.open_settings_tab(el, wid, None),
            Action::Hints => self.start_hints(wid),
            Action::HistorySearch => self.open_history_search(wid),
            Action::StashTab => {
                if let Some(i) = self.windows.get(&wid).map(|w| w.active_tab) {
                    // running programs end when stashed (like on restart): ask first
                    let busy = self.windows[&wid].tabs[i]
                        .root
                        .leaf_list()
                        .iter()
                        .filter_map(|id| self.panes.get(id))
                        .any(|p| p.remote.is_none() && !p.settings && !p.shell_idle());
                    if busy {
                        self.request_close(el, wid, Confirm::Stash(i));
                    } else {
                        self.stash_tab(el, wid, i);
                    }
                }
            }
            Action::StashedTabs => self.open_stash_view(wid),
            Action::NextWorkspace => self.cycle_workspace(wid, true),
            Action::PreviousWorkspace => self.cycle_workspace(wid, false),
            Action::NewWorkspace => self.new_workspace(wid),
            Action::RenameWorkspace => self.rename_workspace(wid),
            Action::Workspaces => {
                let (x, y) = match self.windows.get(&wid).and_then(|w| w.ws_btn) {
                    Some(r) => self.ws_menu_anchor(r),
                    None => (0.0, self.windows.get(&wid).map(|w| self.tab_bar_height(w)).unwrap_or(0.0)),
                };
                self.open_workspace_menu(wid, x, y);
            }
            Action::GotoWorkspace(i) => {
                if i >= 1 {
                    self.switch_workspace(wid, i - 1);
                }
            }
            Action::PromptPrev => self.jump_prompt(wid, false),
            Action::PromptNext => self.jump_prompt(wid, true),
            Action::CopyLastOutput => self.copy_last_output(wid),
            Action::None => {}
        }
    }

    // ------------------------------------------------------------------ search

    fn search_update(&mut self, wid: WindowId, from_bottom: bool, direction: alacritty_terminal::index::Direction) {
        use alacritty_terminal::index::Direction;
        use alacritty_terminal::term::search::RegexSearch;
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let Some(st) = win.search.as_mut() else { return };
        let Some(p) = self.panes.get(&st.pane) else { return };
        if st.query.is_empty() {
            st.regex = None;
            st.current = None;
            win.window.request_redraw();
            return;
        }
        // smart case: case-insensitive unless the query has capitals
        let pat = if st.query.chars().any(|c| c.is_uppercase()) {
            regex::escape(&st.query)
        } else {
            format!("(?i){}", regex::escape(&st.query))
        };
        if st.regex.is_none() || from_bottom {
            st.regex = RegexSearch::new(&pat).ok();
        }
        let Some(re) = st.regex.as_mut() else { return };
        let mut term = p.term.lock();
        let origin = if from_bottom || st.current.is_none() {
            let g = term.grid();
            Point::new(Line(g.screen_lines() as i32 - 1), Column(g.columns() - 1))
        } else {
            let m = st.current.clone().unwrap();
            match direction {
                Direction::Left => *m.start(),
                Direction::Right => *m.end(),
            }
        };
        let side = if direction == Direction::Left { Side::Left } else { Side::Right };
        let found = term.search_next(re, origin, direction, side, None);
        let found = match (found, &st.current) {
            // stepping from a match returns the same match first; step once more
            (Some(m), Some(cur)) if !from_bottom && m == *cur => {
                let o = if direction == Direction::Left { *m.start() } else { *m.end() };
                let o = if direction == Direction::Left {
                    if o.column.0 > 0 { Point::new(o.line, o.column - 1) } else { Point::new(o.line - 1, Column(term.grid().columns() - 1)) }
                } else {
                    o
                };
                term.search_next(re, o, direction, side, None)
            }
            (f, _) => f,
        };
        if let Some(m) = &found {
            term.scroll_to_point(*m.start());
        }
        st.current = found;
        drop(term);
        win.window.request_redraw();
    }

    /// Keys while the search bar is open. Returns true if consumed.
    fn search_key(&mut self, wid: WindowId, ev: &KeyEvent) -> bool {
        use alacritty_terminal::index::Direction;
        let Some(win) = self.windows.get_mut(&wid) else { return false };
        let Some(st) = win.search.as_mut() else { return false };
        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => {
                win.search = None;
                win.window.request_redraw();
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::ArrowUp) if !self.modifiers.shift_key() => {
                self.search_update(wid, false, Direction::Left)
            }
            Key::Named(NamedKey::Enter) | Key::Named(NamedKey::ArrowDown) => self.search_update(wid, false, Direction::Right),
            Key::Named(NamedKey::Backspace) => {
                st.query.pop();
                self.search_update(wid, true, Direction::Left);
            }
            _ => {
                if self.modifiers.control_key() || self.modifiers.alt_key() {
                    return false; // let bindings (e.g. paste) work
                }
                let Some(t) = &ev.text else { return true };
                st.query.extend(t.chars().filter(|c| !c.is_control()));
                self.search_update(wid, true, Direction::Left);
            }
        }
        true
    }

    // ------------------------------------------------------------------ input

    fn on_key(&mut self, el: &ActiveEventLoop, wid: WindowId, ev: KeyEvent) {
        if ev.state != ElementState::Pressed {
            // Key releases only matter for apps using the kitty keyboard protocol.
            if let Some(p) = self.active_pane_id(wid).and_then(|id| self.panes.get(&id)) {
                let mode = *p.term.lock().mode();
                if mode.contains(TermMode::REPORT_EVENT_TYPES) {
                    if let Some(b) = keys::encode_kitty(&ev, self.modifiers, mode, 3, &self.keycfg) {
                        p.write(b);
                    }
                }
            }
            return;
        }
        if self.windows.get(&wid).map(|w| w.preedit.is_some()).unwrap_or(false) {
            return; // input method is composing
        }
        if self.windows.get(&wid).map(|w| w.menu.is_some()).unwrap_or(false) && self.menu_key(el, wid, &ev) {
            return;
        }
        if self.windows.get(&wid).map(|w| w.hints.is_some()).unwrap_or(false) && self.hints_key(wid, &ev) {
            return;
        }
        if self.windows.get(&wid).map(|w| w.hist.is_some()).unwrap_or(false) && self.history_key(wid, &ev) {
            return;
        }
        if self.windows.get(&wid).map(|w| w.stash.is_some()).unwrap_or(false) && self.stash_key(wid, &ev) {
            return;
        }
        // Close confirmation prompt
        if let Some(c) = self.windows.get(&wid).and_then(|w| w.confirm) {
            if let Confirm::Exited(id) = c {
                match &ev.logical_key {
                    Key::Named(NamedKey::Shift | NamedKey::Control | NamedKey::Alt | NamedKey::Super) => return,
                    Key::Character(s) if s.eq_ignore_ascii_case("r") => {
                        self.windows.get_mut(&wid).unwrap().confirm = None;
                        self.restart_pane(wid, id);
                    }
                    Key::Named(NamedKey::Enter) => {
                        self.windows.get_mut(&wid).unwrap().confirm = None;
                        self.close_pane(el, id);
                    }
                    Key::Character(s) if s.eq_ignore_ascii_case("y") => {
                        self.windows.get_mut(&wid).unwrap().confirm = None;
                        self.close_pane(el, id);
                    }
                    _ => self.windows.get_mut(&wid).unwrap().confirm = None,
                }
                if let Some(w) = self.windows.get(&wid) {
                    w.window.request_redraw();
                }
                return;
            }
            if matches!(&ev.logical_key, Key::Character(s) if s.eq_ignore_ascii_case("d")) && self.confirm_is_quit(c) {
                self.windows.get_mut(&wid).unwrap().confirm = None;
                self.quit(el, true);
                return;
            }
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
                    if std::mem::take(&mut win.rename_ws) {
                        if !title.is_empty() {
                            let c = win.ws_cur;
                            win.workspaces[c].name = title;
                        }
                    } else if let Some(t) = win.tabs.get_mut(win.active_tab) {
                        t.title = if title.is_empty() { None } else { Some(title) };
                    }
                    self.relayout(wid);
                    self.mark_save_soon();
                }
                Key::Named(NamedKey::Escape) => {
                    let w = self.windows.get_mut(&wid).unwrap();
                    w.rename = None;
                    w.rename_ws = false;
                }
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
        if self.windows.get(&wid).map(|w| w.search.is_some()).unwrap_or(false) && self.search_key(wid, &ev) {
            return;
        }
        self.last_input = Instant::now();
        // The settings page captures shortcuts with "report all keys": don't eat them.
        let capturing = self
            .active_pane_id(wid)
            .and_then(|id| self.panes.get(&id))
            .map(|p| p.settings && p.term.lock().mode().contains(TermMode::REPORT_ALL_KEYS_AS_ESC))
            .unwrap_or(false);
        if !capturing {
            if let Some(a) = self.bindings.lookup(&ev, self.modifiers) {
                // Ctrl+R is ours only at a local shell prompt; vim/less/ssh… get the key
                let pass = (a == Action::HistorySearch && !self.active_pane_id(wid).map(|id| self.shell_idle(id)).unwrap_or(false))
                    // workspace cycling keys (ctrl+page_up/down) belong to the program until there are workspaces
                    || (matches!(a, Action::NextWorkspace | Action::PreviousWorkspace)
                        && self.windows.get(&wid).map(|w| w.workspaces.len() < 2).unwrap_or(true));
                if !pass {
                    self.do_action(el, wid, a);
                    return;
                }
            }
        }
        let Some(id) = self.active_pane_id(wid) else { return };
        let Some(p) = self.panes.get(&id) else { return };
        let mode = *p.term.lock().mode();
        let bytes = if mode.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
            keys::encode_kitty(&ev, self.modifiers, mode, if ev.repeat { 2 } else { 1 }, &self.keycfg)
        } else {
            keys::encode(&ev, self.modifiers, mode, &self.keycfg)
        };
        if let Some(bytes) = bytes {
            {
                let mut t = p.term.lock();
                if self.cfg.scroll_on_key {
                    t.scroll_display(Scroll::Bottom);
                }
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
        if self.windows[&wid].menu.is_some() {
            self.menu_mouse(el, wid, pressed, button);
            return;
        }
        if self.windows[&wid].stash.is_some() {
            if pressed && button == MouseButton::Left {
                self.stash_click(wid, x as f32, y as f32);
            }
            return;
        }
        // Stash button at the end of the tab bar, workspace button at its start
        if pressed && matches!(button, MouseButton::Left | MouseButton::Right) {
            if self.windows[&wid].stash_btn.map(|r| r.contains(x as f32, y as f32)).unwrap_or(false) {
                self.open_stash_view(wid);
                return;
            }
            if let Some(r) = self.windows[&wid].ws_btn.filter(|r| r.contains(x as f32, y as f32)) {
                let (mx, my) = self.ws_menu_anchor(r);
                self.open_workspace_menu(wid, mx, my);
                return;
            }
        }
        // Right click: context menu (shift+right click always, even if the app uses the mouse)
        if pressed && button == MouseButton::Right {
            if let Some((id, rect)) = self.pane_at(wid, x, y) {
                let mouse_app = self.panes.get(&id).map(|p| p.term.lock().mode().intersects(TermMode::MOUSE_MODE)).unwrap_or(false);
                if !mouse_app || self.modifiers.shift_key() {
                    self.focus_pane(wid, id);
                    let (point, _, _, _) = self.cell_at(wid, id, rect, x, y);
                    let url = self.url_at(id, point).map(|u| u.2);
                    self.open_menu(wid, x as f32, y as f32, url);
                    return;
                }
            } else if let Some(i) = self.windows[&wid].tab_rects.iter().find(|(_, r)| r.contains(x as f32, y as f32)).map(|t| t.0) {
                // the menu's tab items act on the tab that was right-clicked
                self.windows.get_mut(&wid).unwrap().active_tab = i;
                self.relayout(wid);
                self.open_menu(wid, x as f32, y as f32, None);
                return;
            }
        }
        // Tab bar
        if !pressed && button == MouseButton::Left && self.windows.get_mut(&wid).unwrap().tab_drag.take().is_some() {
            return;
        }
        if pressed {
            let hit = self.windows[&wid].tab_rects.iter().find(|(_, r)| r.contains(x as f32, y as f32)).map(|x| x.0);
            if let Some(i) = hit {
                match button {
                    MouseButton::Left => {
                        let now = Instant::now();
                        let win = self.windows.get_mut(&wid).unwrap();
                        let double = matches!(win.tab_last_click, Some((t, j)) if j == i && now - t < Duration::from_millis(400));
                        win.tab_last_click = if double { None } else { Some((now, i)) };
                        win.active_tab = i;
                        win.tab_drag = (!double).then_some(i);
                        self.relayout(wid);
                        if double {
                            self.do_action(el, wid, Action::SetTabTitle);
                        }
                    }
                    MouseButton::Middle => self.request_close(el, wid, Confirm::Tab(i)),
                    _ => {}
                }
                return;
            }
        }
        // Scrollbar drag
        if button == MouseButton::Left {
            if !pressed && self.windows.get_mut(&wid).unwrap().scroll_drag.take().is_some() {
                return;
            }
            if pressed {
                if let Some((id, rect)) = self.scrollbar_hit(wid, x, y) {
                    self.windows.get_mut(&wid).unwrap().scroll_drag = Some(id);
                    self.scroll_to_y(wid, id, rect, y);
                    return;
                }
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
        if pressed && button == MouseButton::Left && (self.modifiers.control_key() || !self.cfg.url_requires_ctrl) {
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

    fn scrollbar_hit(&self, wid: WindowId, x: f64, y: f64) -> Option<(u64, Rect)> {
        if self.cfg.scrollbar == "none" {
            return None;
        }
        let win = &self.windows[&wid];
        let zone = (8.0 * win.scale as f32).round();
        let (id, r) = self.pane_at(wid, x, y)?;
        let x = x as f32;
        let inside = if self.cfg.scrollbar == "left" { x < r.x + zone } else { x > r.x + r.w - zone };
        let has_hist = self.panes.get(&id).map(|p| p.term.lock().grid().history_size() > 0).unwrap_or(false);
        (inside && has_hist).then_some((id, r))
    }

    fn scroll_to_y(&self, wid: WindowId, id: u64, r: Rect, y: f64) {
        let Some(p) = self.panes.get(&id) else { return };
        let mut t = p.term.lock();
        let hist = t.grid().history_size();
        let frac = ((y as f32 - r.y) / r.h).clamp(0.0, 1.0);
        let target = ((1.0 - frac) * hist as f32).round() as i32;
        let cur = t.grid().display_offset() as i32;
        t.scroll_display(Scroll::Delta(target - cur));
        self.windows[&wid].window.request_redraw();
    }

    fn on_cursor_moved(&mut self, wid: WindowId, x: f64, y: f64) {
        self.windows.get_mut(&wid).unwrap().cursor = (x, y);
        if self.windows[&wid].menu.is_some() {
            self.update_hover(wid);
        }
        if let Some(menu) = self.windows.get_mut(&wid).unwrap().menu.as_mut() {
            let h = menu.item_at(x as f32, y as f32);
            if h != menu.hover {
                menu.hover = h;
                self.windows[&wid].window.request_redraw();
            }
            return;
        }
        if let Some(from) = self.windows[&wid].tab_drag {
            self.drag_tab(wid, from, x as f32);
            return;
        }
        if let Some(id) = self.windows[&wid].scroll_drag {
            if let Some(r) = self.windows[&wid].pane_rects.iter().find(|(i, _)| *i == id).map(|x| x.1) {
                self.scroll_to_y(wid, id, r, y);
            }
            return;
        }
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

    /// Move the dragged tab to the slot under the cursor's x (y is ignored, so the
    /// drag keeps working when the pointer slips off the tab bar).
    fn drag_tab(&mut self, wid: WindowId, from: usize, x: f32) {
        let win = self.windows.get_mut(&wid).unwrap();
        let (Some(first), Some(last)) = (win.tab_rects.first(), win.tab_rects.last()) else { return };
        let to = if x < first.1.x {
            first.0
        } else if x >= last.1.x + last.1.w {
            last.0
        } else {
            match win.tab_rects.iter().find(|(_, r)| x >= r.x && x < r.x + r.w) {
                Some((i, _)) => *i,
                None => return,
            }
        };
        if to == from || from >= win.tabs.len() || to >= win.tabs.len() {
            return;
        }
        let tab = win.tabs.remove(from);
        win.tabs.insert(to, tab);
        win.active_tab = to;
        win.tab_drag = Some(to);
        self.relayout(wid);
        self.mark_save_soon();
    }

    fn on_wheel(&mut self, wid: WindowId, delta: MouseScrollDelta) {
        if self.windows.get(&wid).map(|w| w.stash.is_some()).unwrap_or(false) {
            // wheel moves the selection in the stash browser
            let up = match delta {
                MouseScrollDelta::LineDelta(_, y) => y > 0.0,
                MouseScrollDelta::PixelDelta(p) => p.y > 0.0,
            };
            self.stash_scroll(wid, up);
            return;
        }
        // Some X setups deliver a wheel click twice (XI2 valuator + legacy button): drop
        // a same-direction line event arriving within 15 ms.
        if let MouseScrollDelta::LineDelta(_, y) = delta {
            let now = Instant::now();
            if let Some((t, up)) = self.last_wheel {
                if now - t < Duration::from_millis(15) && up == (y > 0.0) {
                    return;
                }
            }
            self.last_wheel = Some((now, y > 0.0));
        }
        // Ctrl+scroll: font size of this window
        if self.modifiers.control_key() {
            let steps = match delta {
                MouseScrollDelta::LineDelta(_, y) => {
                    self.zoom_acc = 0.0;
                    if y > 0.0 { 1 } else if y < 0.0 { -1 } else { 0 }
                }
                MouseScrollDelta::PixelDelta(p) => {
                    self.zoom_acc += p.y;
                    let n = (self.zoom_acc / 40.0).trunc();
                    self.zoom_acc -= n * 40.0;
                    n as i32
                }
            };
            if steps != 0 {
                let size = (self.windows[&wid].font_size + steps as f32).clamp(4.0, 72.0);
                self.set_font_size(wid, size);
                self.toast(wid, format!("Font size {size}pt  (Ctrl+Shift+Backspace: reset)"));
            }
            return;
        }
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
        self.clear_attended(wid);
        let markers: Vec<&'static str> =
            self.windows.get(&wid).map(|w| w.tabs.iter().map(|t| self.tab_marker(t)).collect()).unwrap_or_default();
        let last_input = self.last_input;
        let tab_bar_h = self.windows.get(&wid).map(|w| self.tab_bar_height(w)).unwrap_or(0.0);
        let tb_h_pre = tab_bar_h;
        let stash_count = self.stash_count;
        // workspace button: "Work ▾" (+ marker of activity in the other workspaces)
        let ws_chip: Option<String> = self.windows.get(&wid).filter(|w| w.ws_visible()).map(|w| {
            let mark = self.ws_others_marker(w);
            match (&w.rename, w.rename_ws) {
                (Some(buf), true) => format!("✎ {buf}▏"),
                _ => format!("{} ▾{}", w.ws_name(), if mark.is_empty() { String::new() } else { format!(" {}", mark.trim_end()) }),
            }
        });
        let ws_title = self.windows.get(&wid).filter(|w| w.ws_visible()).map(|w| w.ws_name().to_string());
        let confirm_text = self.windows.get(&wid).and_then(|w| w.confirm).map(|c| {
            let hint = if matches!(c, Confirm::ClipRead(_)) {
                "[y] allow once   [a] always   [n] deny"
            } else if matches!(c, Confirm::Exited(_)) {
                "[Enter] close   [r] restart shell   [Esc] keep it open"
            } else if self.confirm_is_quit(c) {
                "[y/Enter] quit   [d] quit without saving   [n/Esc] cancel"
            } else {
                "[y/Enter] yes   [n/Esc] no"
            };
            (self.confirm_text(wid, c), hint.to_string())
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
            let blink_off = cfg.cursor_blink && win.focused && (last_input.elapsed().as_millis() / 530) % 2 == 1;
            let term = pane.term.lock();
            let content = term.renderable_content();
            let colors = content.colors;
            let offset = content.display_offset as i32;
            let selection = content.selection;
            let cursor = content.cursor;
            // visible search matches
            let mut matches: Vec<alacritty_terminal::term::search::Match> = Vec::new();
            let mut current_match = None;
            if let Some(st) = win.search.as_mut().filter(|s| s.pane == *id) {
                current_match = st.current.clone();
                if let Some(re) = st.regex.as_mut() {
                    let g = term.grid();
                    let start = Point::new(Line(-offset), Column(0));
                    let end = Point::new(Line(g.screen_lines() as i32 - 1 - offset), Column(g.columns() - 1));
                    matches = alacritty_terminal::term::search::RegexIter::new(
                        start,
                        end,
                        alacritty_terminal::index::Direction::Right,
                        &term,
                        re,
                    )
                    .take(500)
                    .collect();
                }
            }
            let ox = r.x + pad;
            let oy = r.y + pad;
            let mut row_buf: Vec<RCell> = Vec::with_capacity(pane.cols);
            let mut cur_row = i32::MIN;
            let imgs = pane.images.lock();
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
                // kitty graphics placeholder cell: draw its slice of the image
                if cell.c == crate::images::PLACEHOLDER {
                    if let (Color::Spec(f), Color::Spec(b)) = (cell.fg, cell.bg) {
                        let (key, irow, icol) = crate::images::decode_cell([f.r, f.g, f.b], [b.r, b.g, b.b]);
                        if let Some(pl) = imgs.placements.get(&key) {
                            if let Some(img) = imgs.images.get(&pl.image) {
                                // fit the image into its cell box, keeping the aspect ratio
                                let (bw, bh) = (pl.cols as f32 * m.width, pl.rows as f32 * m.height);
                                let sc = (bw / img.w as f32).min(bh / img.h as f32);
                                let (dw, dh) = (img.w as f32 * sc, img.h as f32 * sc);
                                let (x0, y0) = (icol as f32 * m.width, irow as f32 * m.height);
                                let (x1, y1) = ((x0 + m.width).min(dw), (y0 + m.height).min(dh));
                                if x1 > x0 && y1 > y0 {
                                    let uv = [x0 / dw, y0 / dh, (x1 - x0) / dw, (y1 - y0) / dh];
                                    win.renderer.image(gpu, (*id, pl.image), img, [x, y, x1 - x0, y1 - y0], uv);
                                }
                            }
                        }
                    }
                    continue;
                }
                let fg_color = match cell.fg {
                    Color::Named(n) if cfg.bold_is_bright && cell.flags.contains(Flags::BOLD) && (n as usize) < 8 => {
                        Color::Indexed(n as u8 + 8)
                    }
                    Color::Indexed(i) if cfg.bold_is_bright && cell.flags.contains(Flags::BOLD) && i < 8 => Color::Indexed(i + 8),
                    c => c,
                };
                let mut fg = resolve(fg_color, colors, theme);
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
                } else if !matches.is_empty() && matches.iter().any(|m| m.contains(&ind.point)) {
                    let cur = current_match.as_ref().map(|m| m.contains(&ind.point)).unwrap_or(false);
                    bg = if cur { theme.sel_bg } else { theme.search_match };
                    fg = theme.table[257];
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
            drop(imgs);
            draw_row(&mut win.renderer, gpu, fonts, &row_buf, ox, oy + cur_row as f32 * m.height, cfg.ligatures);
            // Cursor
            let crow = cursor.point.line.0 + offset;
            if cursor.shape != CursorShape::Hidden && !blink_off && crow >= 0 && (crow as usize) < pane.rows {
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

        // Keyboard hints
        if let Some(h) = &win.hints {
            if let Some((_, r)) = rects.iter().find(|(id, _)| *id == h.pane) {
                if let Some(pane) = panes.get(&h.pane) {
                    let off = pane.term.lock().grid().display_offset() as i32;
                    let (ox, oy) = (r.x + pad, r.y + pad);
                    for it in h.items.iter().filter(|i| i.label.starts_with(&h.typed)) {
                        let row = it.line.0 + off;
                        if row < 0 || row as usize >= pane.rows {
                            continue;
                        }
                        let y = oy + row as f32 * m.height;
                        let x = ox + it.start as f32 * m.width;
                        let w = (it.end + 1 - it.start) as f32 * m.width;
                        win.renderer.rect_top(x, y + m.height - m.thickness, w, m.thickness, rgba(theme.search_match, 1.0));
                        let label = it.label.to_uppercase();
                        let lw = label.chars().count() as f32 * m.width;
                        win.renderer.rect_top(x, y, lw, m.height, rgba(theme.search_match, 1.0));
                        win.renderer.text(gpu, fonts, &label, x, y, lw + 1.0, rgba(theme.table[257], 1.0));
                    }
                }
            }
            let (_, wh) = win.renderer.size();
            let bh = m.height + 6.0;
            let by = if cfg.tab_bar_position == "bottom" { wh - tb_h_pre - bh } else { wh - bh };
            win.renderer.rect_top(0.0, by, ww, bh, rgba(theme.tab_bar_bg, 1.0));
            let txt = format!("Hints {}▏  letter: open link / copy   Shift+letter: paste into terminal   Esc: cancel", h.typed.to_uppercase());
            win.renderer.text(gpu, fonts, &txt, m.width, by + 3.0, ww - m.width, rgba(theme.table[256], 1.0));
        }

        // Ctrl+R history search
        if let Some(hs) = &win.hist {
            let (_, wh) = win.renderer.size();
            let rows = ((wh * 0.45 / m.height) as usize).clamp(5, 20);
            let bh = (rows + 2) as f32 * m.height + 8.0;
            let by = wh - bh;
            let r = &mut win.renderer;
            r.rect_top(0.0, by - 1.0, ww, 1.0, rgba(theme.active_border, 1.0));
            r.rect_top(0.0, by, ww, bh, rgba(theme.tab_bar_bg, 1.0));
            let scope = if hs.all { "all tabs" } else { "this tab" };
            let head = format!(" History ({scope}) {}/{}   Enter: insert   Ctrl+R: {}   Esc: cancel",
                hs.matches.len(), hs.items.len(), if hs.all { "this tab only" } else { "all tabs" });
            r.text(gpu, fonts, &head, 0.0, by + 4.0, ww, rgba(theme.inactive_border, 1.0));
            // list: best match at the bottom (closest to the prompt), like fzf
            let list_bottom = wh - m.height - 4.0;
            let start = hs.sel.saturating_sub(rows - 1);
            for (k, (idx, pos)) in hs.matches.iter().enumerate().skip(start).take(rows) {
                let y = list_bottom - ((k - start) + 1) as f32 * m.height;
                let selected = k == hs.sel;
                if selected {
                    r.rect_top(0.0, y, ww, m.height, rgba(theme.sel_bg, 1.0));
                }
                let fg = if selected { theme.sel_fg } else { theme.table[256] };
                let line: String = hs.items[*idx].replace('\n', " ⏎ ");
                let x0 = 2.0 * m.width;
                // highlight matched characters
                for &p in pos {
                    let x = x0 + p as f32 * m.width;
                    if x + m.width < ww {
                        r.rect_top(x, y + m.height - m.thickness * 2.0, m.width, m.thickness * 2.0, rgba(theme.search_match, 1.0));
                    }
                }
                r.text(gpu, fonts, if selected { "▶" } else { " " }, 0.0, y, m.width * 2.0, rgba(fg, 1.0));
                r.text(gpu, fonts, &line, x0, y, ww - x0, rgba(fg, 1.0));
            }
            let q = format!("> {}▏", hs.query);
            r.text(gpu, fonts, &q, m.width, list_bottom, ww, rgba(theme.table[256], 1.0));
        }

        // Stash browser
        if let Some(v) = &win.stash {
            let size = win.renderer.size();
            let (_, wh) = size;
            let g = stash::stash_geometry(size, m, tab_bar_h, cfg);
            let bottom = if cfg.tab_bar_position == "bottom" { wh - tab_bar_h } else { wh };
            let r = &mut win.renderer;
            let dim = rgba(theme.inactive_border, 1.0);
            let fg = rgba(theme.table[256], 1.0);
            r.rect_top(0.0, g.top, ww, bottom - g.top, rgba(theme.tab_bar_bg, 1.0));
            let head = format!(
                " Stashed tabs ({})   Enter / double-click: restore   d: delete   Esc: close",
                v.entries.len()
            );
            r.text(gpu, fonts, &head, 0.0, g.top + 4.0, ww, dim);
            r.rect_top(0.0, g.list_y - 5.0, ww, 1.0, rgba(theme.active_border, 1.0));
            if v.entries.is_empty() {
                let key = keys::effective_combos(cfg, "stash_tab").first().map(|c| pretty_combo(c)).unwrap_or_default();
                let msg = if key.is_empty() {
                    "No stashed tabs. Right-click a tab → Stash Tab puts it away.".to_string()
                } else {
                    format!("No stashed tabs. {key} (or right-click → Stash Tab) puts the current tab away.")
                };
                r.text(gpu, fonts, &msg, m.width * 2.0, g.list_y + m.height, ww, fg);
            } else {
                let start = g.first_row(v.sel);
                for (k, e) in v.entries.iter().enumerate().skip(start).take(g.rows) {
                    let y = g.list_y + (k - start) as f32 * g.row_h;
                    let selected = k == v.sel;
                    if selected {
                        r.rect_top(0.0, y, g.list_w, g.row_h - 2.0, rgba(theme.sel_bg, 1.0));
                    }
                    let (c1, c2) = if selected { (rgba(theme.sel_fg, 1.0), rgba(theme.sel_fg, 0.75)) } else { (fg, dim) };
                    let title = if e.title.is_empty() { "(untitled)" } else { e.title.as_str() };
                    r.text(gpu, fonts, &format!("{} {title}", if selected { "▶" } else { " " }), m.width * 0.5, y + 2.0, g.list_w - m.width, c1);
                    let mut info = Vec::new();
                    if let Some(c) = &e.cwd {
                        info.push(stash::tilde(c));
                    }
                    if e.panes > 1 {
                        info.push(format!("{} panes", e.panes));
                    }
                    info.push(stash::ago(e.stashed_at));
                    r.text(gpu, fonts, &info.join(" · "), m.width * 2.5, y + 2.0 + m.height, g.list_w - m.width * 3.0, c2);
                }
                // preview of the selected tab's focused pane (newest lines at the bottom)
                r.rect_top(g.list_w, g.list_y - 4.0, 1.0, bottom - g.list_y + 4.0, rgba(theme.active_border, 1.0));
                let x0 = g.list_w + m.width;
                let rows = ((bottom - g.list_y - 4.0) / m.height).max(1.0) as usize;
                // panes share the height; each shows its newest lines under a header
                let n = v.preview.len().max(1);
                let per = (rows / n).max(2);
                let mut y = g.list_y;
                for (i, (header, lines)) in v.preview.iter().enumerate() {
                    let mut avail = if i + 1 == n { rows.saturating_sub(per * i) } else { per };
                    if !header.is_empty() {
                        if i > 0 {
                            r.rect_top(g.list_w, y - 2.0, ww - g.list_w, 1.0, rgba(theme.inactive_border, 1.0));
                        }
                        r.text(gpu, fonts, header, x0, y, ww - x0, rgba(theme.active_border, 1.0));
                        y += m.height;
                        avail = avail.saturating_sub(1);
                    }
                    let first = lines.len().saturating_sub(avail);
                    for l in &lines[first..] {
                        r.text(gpu, fonts, l, x0, y, ww - x0, fg);
                        y += m.height;
                    }
                    if lines.is_empty() {
                        r.text(gpu, fonts, "(no saved output)", x0, y, ww - x0, dim);
                    }
                    y = g.list_y + (per * (i + 1)) as f32 * m.height;
                }
            }
            if v.confirm_delete {
                if let Some(e) = v.entries.get(v.sel) {
                    let msg = format!(" Press d again to delete “{}” for good ", e.title);
                    let w = (msg.chars().count() as f32 * m.width).min(ww);
                    let y = bottom - m.height - 6.0;
                    r.rect_top(0.0, y - 3.0, w, m.height + 6.0, rgba(theme.search_match, 1.0));
                    r.text(gpu, fonts, &msg, 0.0, y, w, rgba(theme.table[257], 1.0));
                }
            }
        }

        // Scrollbars
        if cfg.scrollbar != "none" {
            let sw = (4.0 * win.scale as f32).round().max(3.0);
            for (id, r) in &rects {
                let Some(pane) = panes.get(id) else { continue };
                let t = pane.term.lock();
                let hist = t.grid().history_size();
                if hist == 0 {
                    continue;
                }
                let off = t.grid().display_offset();
                let total = (hist + pane.rows) as f32;
                let th = (r.h * pane.rows as f32 / total).max(16.0).min(r.h);
                let ty = r.y + (r.h - th) * (1.0 - off as f32 / hist as f32);
                let x = if cfg.scrollbar == "left" { r.x } else { r.x + r.w - sw };
                win.renderer.rect_top(x, r.y, sw, r.h, rgba(theme.inactive_border, 0.35));
                win.renderer.rect_top(x, ty, sw, th, rgba(theme.active_border, 0.8));
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
        let tb_h = tab_bar_h;
        let (_, wh_all) = win.renderer.size();
        let bar_y = if cfg.tab_bar_position == "bottom" { wh_all - tb_h } else { 0.0 };
        win.tab_rects.clear();
        let mut titles: Vec<String> = Vec::new();
        for (i, t) in win.tabs.iter().enumerate() {
            let base = match &t.title {
                Some(s) => s.clone(),
                None => panes
                    .get_mut(&t.active)
                    .map(|p| {
                        let fallback = p.fallback_title();
                        format_title(cfg, &p.title, &fallback)
                    })
                    .unwrap_or_default(),
            };
            let base = if win.rename.is_some() && !win.rename_ws && i == active_tab {
                format!("✎ {}▏", win.rename.as_ref().unwrap())
            } else {
                base
            };
            let z = if t.zoomed { " [Z]" } else { "" };
            let n = t.root.leaf_list().len();
            let count = if n > 1 { format!(" ({n})") } else { String::new() };
            let mark = markers.get(i).copied().unwrap_or("");
            titles.push(format!("{}: {mark}{}{}{}", i + 1, base, count, z));
        }
        win.stash_btn = None;
        win.ws_btn = None;
        let mut tabs_x = 0.0;
        if tb_h > 0.0 {
            win.renderer.rect(0.0, bar_y, ww, tb_h, rgba(theme.tab_bar_bg, 1.0));
            if let Some(label) = &ws_chip {
                let bw = ((label.chars().count() + 2) as f32 * m.width).min(ww / 3.0).floor();
                let ty = bar_y + ((tb_h - m.height) / 2.0).round();
                win.renderer.rect(0.0, bar_y, bw, tb_h, rgba(theme.active_border, 1.0));
                win.renderer.text(gpu, fonts, label, m.width, ty, bw - m.width, rgba(theme.tab_bar_bg, 1.0));
                win.ws_btn = Some(Rect { x: 0.0, y: bar_y, w: bw, h: tb_h });
                tabs_x = bw + 1.0;
            }
            // "≡ 3 stashed" button at the right end (opens the stash browser)
            let mut tabs_w = ww;
            if stash_count > 0 {
                let label = format!("≡ {stash_count} stashed");
                let bw = ((label.chars().count() + 2) as f32 * m.width).min(ww / 3.0).floor();
                let bx = ww - bw;
                let ty = bar_y + ((tb_h - m.height) / 2.0).round();
                win.renderer.rect(bx, bar_y, bw, tb_h, rgba(theme.tab_inactive_bg, 1.0));
                win.renderer.text(gpu, fonts, &label, bx + m.width, ty, bw - m.width, rgba(theme.tab_inactive_fg, 1.0));
                win.stash_btn = Some(Rect { x: bx, y: bar_y, w: bw, h: tb_h });
                tabs_w = bx - 1.0;
            }
            let ww = tabs_w - tabs_x;
            let n = win.tabs.len().max(1) as f32;
            let tw = if cfg.tab_bar_expand { (ww / n).floor() } else { (ww / n).min(m.width * 32.0).floor() };
            for (i, title) in titles.iter().enumerate() {
                let x = tabs_x + i as f32 * tw;
                let (fg, bg) = if i == active_tab {
                    (theme.tab_active_fg, theme.tab_active_bg)
                } else {
                    (theme.tab_inactive_fg, theme.tab_inactive_bg)
                };
                win.renderer.rect(x, bar_y, tw - 1.0, tb_h, rgba(bg, 1.0));
                let ty = bar_y + ((tb_h - m.height) / 2.0).round();
                win.renderer.text(gpu, fonts, title, x + m.width, ty, tw - 2.0 * m.width, rgba(fg, 1.0));
                win.tab_rects.push((i, Rect { x, y: bar_y, w: tw, h: tb_h }));
            }
        } else if win.rename.is_some() {
            // Rename prompt without a tab bar: draw an overlay line on top
            win.renderer.rect(0.0, 0.0, ww, m.height + 4.0, rgba(theme.tab_active_bg, 1.0));
            let line = if win.rename_ws {
                format!("Workspace name: {}▏", win.rename.as_deref().unwrap_or(""))
            } else {
                format!("Tab title: {}", titles.get(active_tab).cloned().unwrap_or_default())
            };
            win.renderer.text(gpu, fonts, &line, m.width, 2.0, ww, rgba(theme.tab_active_fg, 1.0));
        }
        // Window title
        if let Some(t) = titles.get(active_tab) {
            let t = t.split_once(": ").map(|x| x.1).unwrap_or(t).to_string();
            let title = match &ws_title { Some(w) => format!("{t} — {w} — dtem"), None => format!("{t} — dtem") };
            if title != win.last_title {
                win.window.set_title(&title);
                win.last_title = title;
            }
        }
        // Close confirmation overlay
        if let Some((line, hint)) = confirm_text {
            let (_, wh) = win.renderer.size();
            let tw = (line.chars().count().max(hint.chars().count()) as f32 + 4.0) * m.width;
            let bw = tw.min(ww);
            let bh = m.height * 4.0;
            let bx = ((ww - bw) / 2.0).max(0.0);
            let by = ((wh - bh) / 2.0).max(0.0);
            win.renderer.rect_top(bx - 2.0, by - 2.0, bw + 4.0, bh + 4.0, rgba(theme.active_border, 1.0));
            win.renderer.rect_top(bx, by, bw, bh, rgba(theme.tab_bar_bg, 1.0));
            win.renderer.text(gpu, fonts, &line, bx + 2.0 * m.width, by + m.height, bw - 2.0 * m.width, rgba(theme.table[256], 1.0));
            win.renderer.text(gpu, fonts, &hint, bx + 2.0 * m.width, by + m.height * 2.0, bw - 2.0 * m.width, rgba(theme.active_border, 1.0));
        }
        // Search bar
        if let Some(st) = &win.search {
            let (_, wh) = win.renderer.size();
            let bh = m.height + 6.0;
            let by = if cfg.tab_bar_position == "bottom" { wh - tb_h - bh } else { wh - bh };
            win.renderer.rect_top(0.0, by, ww, bh, rgba(theme.tab_bar_bg, 1.0));
            win.renderer.rect_top(0.0, by, ww, 1.0, rgba(theme.active_border, 1.0));
            let status = if st.query.is_empty() {
                ""
            } else if st.current.is_none() {
                "   (no match)"
            } else {
                ""
            };
            let text = format!("Search: {}▏{status}", st.query);
            win.renderer.text(gpu, fonts, &text, m.width, by + 3.0, ww * 0.6, rgba(theme.table[256], 1.0));
            let hint = "Enter/↑ older  Shift+Enter/↓ newer  Esc close";
            let hw = hint.chars().count() as f32 * m.width;
            if hw < ww * 0.4 {
                win.renderer.text(gpu, fonts, hint, ww - hw - m.width, by + 3.0, hw + m.width, rgba(theme.inactive_border, 1.0));
            }
        }
        // Visual bell
        if let Some(t) = win.bell_at {
            if t.elapsed() < Duration::from_millis(150) {
                let (_, wh) = win.renderer.size();
                win.renderer.rect_top(0.0, 0.0, ww, wh, rgba(theme.table[256], 0.12));
            } else {
                win.bell_at = None;
            }
        }
        // Context menu
        if let Some(menu) = &win.menu {
            let h = menu.height();
            let r = &mut win.renderer;
            r.rect_top(menu.x + 3.0, menu.y + 3.0, menu.w, h, [0.0, 0.0, 0.0, 0.35]);
            r.rect_top(menu.x - 1.0, menu.y - 1.0, menu.w + 2.0, h + 2.0, rgba(theme.active_border, 1.0));
            r.rect_top(menu.x, menu.y, menu.w, h, rgba(theme.tab_bar_bg, 1.0));
            let mut y = menu.y + 4.0;
            for (i, it) in menu.items.iter().enumerate() {
                if it.cmd.is_none() {
                    r.rect_top(menu.x + 6.0, y + menu.sep_h / 2.0, menu.w - 12.0, 1.0, rgba(theme.inactive_border, 1.0));
                    y += menu.sep_h;
                    continue;
                }
                let hover = menu.hover == Some(i);
                if hover {
                    r.rect_top(menu.x + 2.0, y, menu.w - 4.0, menu.row_h, rgba(theme.sel_bg, 1.0));
                }
                let fg = if hover { theme.sel_fg } else { theme.table[256] };
                let ty = y + ((menu.row_h - m.height) / 2.0).round();
                r.text(gpu, fonts, &it.label, menu.x + m.width, ty, menu.w - 2.0 * m.width, rgba(fg, 1.0));
                if !it.hint.is_empty() {
                    let hw = it.hint.chars().count() as f32 * m.width;
                    let hc = if hover { theme.sel_fg } else { theme.inactive_border };
                    r.text(gpu, fonts, &it.hint, menu.x + menu.w - hw - m.width, ty, hw + m.width, rgba(hc, 1.0));
                }
                y += menu.row_h;
            }
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
                let attended = self.attended(id);
                if let Some(p) = self.panes.get_mut(&id) {
                    p.dirty = true;
                    if !attended && !p.activity {
                        p.activity = true;
                        if let Some(w) = wid.and_then(|w| self.windows.get(&w)) {
                            w.window.request_redraw();
                        }
                    }
                    if self.cfg.scroll_on_output {
                        let mut t = p.term.lock();
                        if t.grid().display_offset() > 0 {
                            t.scroll_display(Scroll::Bottom);
                        }
                    }
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
                if !self.attended(id) {
                    if let Some(p) = self.panes.get_mut(&id) {
                        p.bell = true;
                    }
                    if self.cfg.notify_on_bell {
                        self.notify(id, "", "🔔 Bell — waiting for you?");
                    }
                    if let Some(w) = wid.and_then(|w| self.windows.get(&w)) {
                        w.window.request_redraw();
                    }
                }
                if !self.cfg.bell {
                    return;
                }
                if self.cfg.visual_bell {
                    if let Some(w) = wid.and_then(|w| self.windows.get_mut(&w)) {
                        w.bell_at = Some(Instant::now());
                        w.window.request_redraw();
                    }
                    if let Some(w) = wid {
                        let proxy = self.proxy.clone();
                        std::thread::spawn(move || {
                            std::thread::sleep(Duration::from_millis(160));
                            let _ = proxy.send_event(UserEvent::Redraw(w));
                        });
                    }
                }
                if let Some(w) = wid.and_then(|w| self.windows.get(&w)) {
                    if !w.focused {
                        w.window.request_user_attention(Some(winit::window::UserAttentionType::Informational));
                    }
                }
            }
            TermEvent::ChildExit(_) | TermEvent::Exit => {
                let code = match &ev {
                    TermEvent::ChildExit(st) => st.code(),
                    _ => None,
                };
                if let Some(p) = self.panes.get_mut(&id) {
                    if !p.exited {
                        p.exited = true;
                        self.pane_exited(el, id, code);
                    }
                }
            }
            _ => {}
        }
    }

    fn startup(&mut self, el: &ActiveEventLoop) {
        let Some(st) = self.startup.take() else { return };
        if st.restore {
            if let Some(s) = session::load(&session::session_path(&self.rname())) {
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
        if self.hotkey.is_none() {
            match global_hotkey::GlobalHotKeyManager::new() {
                Ok(m) => {
                    let proxy = self.proxy.clone();
                    let targets = self.hotkey_targets.clone();
                    global_hotkey::GlobalHotKeyEvent::set_event_handler(Some(move |e: global_hotkey::GlobalHotKeyEvent| {
                        if e.state == global_hotkey::HotKeyState::Pressed {
                            let name = targets.lock().get(&e.id).cloned().unwrap_or_default();
                            let _ = proxy.send_event(UserEvent::Command(format!("toggle:{name}")));
                        }
                    }));
                    self.hotkey = Some(m);
                }
                Err(e) => {
                    log::warn!("global hotkey unavailable: {e}");
                    return;
                }
            }
        }
        let m = self.hotkey.as_ref().unwrap();
        for k in self.installed_hotkeys.drain(..) {
            if let Some(h) = crate::hotkey_from_config(&k) {
                let _ = m.unregister(h);
            }
        }
        // Keys already handled by a GNOME shortcut must not be grabbed twice.
        let gnome: Vec<String> = crate::gnome_dtem_accels();
        // The default dropdown also grabs the keys of the other dropdowns and forwards them.
        let mut wanted: Vec<(String, String)> = self.dd().hotkey.list().into_iter().map(|k| (k, self.profile.clone())).collect();
        if self.profile.is_empty() {
            for (name, d) in &self.cfg.dropdowns {
                wanted.extend(d.hotkey.list().into_iter().map(|k| (k, name.clone())));
            }
        }
        let mut targets = self.hotkey_targets.lock();
        targets.clear();
        for (k, prof) in wanted {
            if gnome.iter().any(|g| g.eq_ignore_ascii_case(&crate::gnome_accel(&k))) {
                continue;
            }
            match crate::hotkey_from_config(&k) {
                Some(h) => match m.register(h) {
                    Ok(_) => {
                        targets.insert(h.id(), prof);
                        self.installed_hotkeys.push(k);
                    }
                    Err(e) => log::warn!("cannot grab hotkey {k}: {e}"),
                },
                None => log::error!("invalid dropdown hotkey {k:?}"),
            }
        }
    }

    /// Apply a changed dtem.toml to the running instance.
    fn apply_config(&mut self, el: &ActiveEventLoop) {
        let new = match Config::try_load() {
            Ok(c) => c,
            Err(e) => {
                for w in self.order.clone() {
                    self.toast(w, e.clone());
                }
                return;
            }
        };
        let old = std::mem::replace(&mut self.cfg, new);
        self.bindings = Bindings::new(&self.cfg);
        self.theme = profile_theme(&self.cfg, self.role, &self.profile);
        self.keycfg = keys::KeyCfg::from_config(&self.cfg);
        self.url_re = build_url_regex(&self.cfg);
        if old.transparency != self.cfg.transparency {
            self.transparency_on = self.cfg.transparency;
        }
        if old.font_family != self.cfg.font_family {
            match FontSet::new(&self.cfg.font_family) {
                Ok(f) => self.fonts = f,
                Err(e) => log::error!("font: {e}"),
            }
        }
        let font_changed = old.font_family != self.cfg.font_family
            || old.font_size != self.cfg.font_size
            || old.line_height != self.cfg.line_height;
        let op = self.cfg.effective_opacity(self.transparency_on);
        for wid in self.order.clone() {
            let Some(w) = self.windows.get_mut(&wid) else { continue };
            w.renderer.set_opacity(op);
            if font_changed {
                if old.font_size != self.cfg.font_size {
                    w.font_size = self.cfg.font_size;
                }
                let fs = w.font_size;
                self.set_font_size(wid, fs);
            }
            self.relayout(wid);
            if self.windows[&wid].dropdown && self.windows[&wid].visible && self.anim.is_none() {
                self.apply_dropdown_geometry(el, wid);
                let d = self.dd();
                let w = &self.windows[&wid];
                w.window.set_window_level(if d.always_on_top { WindowLevel::AlwaysOnTop } else { WindowLevel::Normal });
                if let (Some(x), Some(id)) = (&self.x11, crate::x11::window_id(&w.window)) {
                    x.apply(id, d.skip_taskbar, d.all_workspaces, d.always_on_top);
                }
            }
        }
        let keys_of = |c: &Config| {
            let mut v = vec![c.dropdown.hotkey.list()];
            v.extend(c.dropdowns.values().map(|d| d.hotkey.list()));
            v
        };
        if self.role == Role::Dropdown && keys_of(&old) != keys_of(&self.cfg) {
            if !crate::gnome_dtem_accels().is_empty() {
                if let Err(e) = crate::install_gnome_shortcut(&self.cfg, true) {
                    log::warn!("GNOME shortcut update failed: {e}");
                }
            }
            self.register_hotkey();
        }
        for w in self.windows.values() {
            w.window.request_redraw();
        }
    }
}

#[derive(Clone)]
enum MenuCmd {
    Action(&'static str),
    CopyLink(String),
    OpenLink(String),
    Shortcuts,
    /// switch to workspace i
    Workspace(usize),
    CloseWorkspace(usize),
    /// move the active tab to workspace i (None = a new workspace)
    MoveTab(Option<usize>),
}

/// One row of the right-click menu; `cmd == None` is a separator.
struct MenuItem {
    label: String,
    hint: String,
    cmd: Option<MenuCmd>,
}

struct Menu {
    x: f32,
    y: f32,
    w: f32,
    row_h: f32,
    sep_h: f32,
    items: Vec<MenuItem>,
    hover: Option<usize>,
}

impl Menu {
    fn item_at(&self, px: f32, py: f32) -> Option<usize> {
        if px < self.x || px > self.x + self.w {
            return None;
        }
        let mut y = self.y + 4.0;
        for (i, it) in self.items.iter().enumerate() {
            let h = if it.cmd.is_some() { self.row_h } else { self.sep_h };
            if py >= y && py < y + h {
                return it.cmd.as_ref().map(|_| i);
            }
            y += h;
        }
        None
    }

    fn height(&self) -> f32 {
        8.0 + self.items.iter().map(|i| if i.cmd.is_some() { self.row_h } else { self.sep_h }).sum::<f32>()
    }
}

/// "ctrl+shift+t" → "Ctrl+Shift+T"
fn pretty_combo(c: &str) -> String {
    c.split('+')
        .map(|p| match p {
            "ctrl" => "Ctrl".to_string(),
            "shift" => "Shift".to_string(),
            "alt" => "Alt".to_string(),
            "super" => "Super".to_string(),
            "page_up" => "PgUp".to_string(),
            "page_down" => "PgDn".to_string(),
            "comma" => ",".to_string(),
            "period" => ".".to_string(),
            "equal" => "=".to_string(),
            "minus" => "-".to_string(),
            "bracketleft" => "[".to_string(),
            "bracketright" => "]".to_string(),
            k if k.len() == 1 => k.to_uppercase(),
            k if k.starts_with('f') && k[1..].parse::<u8>().is_ok() => k.to_uppercase(),
            k => {
                let mut c = k.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Theme of this instance (a dropdown profile may override the color scheme).
fn profile_theme(cfg: &Config, role: Role, profile: &str) -> Theme {
    if role == Role::Dropdown {
        if let Some(cs) = cfg.dropdown_profile(profile).color_scheme.filter(|c| !c.is_empty()) {
            let mut c = cfg.clone();
            c.color_scheme = cs;
            c.colors = Default::default();
            return Theme::from_config(&c);
        }
    }
    Theme::from_config(cfg)
}

struct Anim {
    wid: WindowId,
    start: Instant,
    dur: Duration,
    from: (i32, i32),
    to: (i32, i32),
    hide: bool,
}

/// Position just outside the monitor edge the dropdown slides from.
fn offscreen(from: &str, m: &MonitorHandle, pos: PhysicalPosition<i32>, size: PhysicalSize<u32>) -> PhysicalPosition<i32> {
    let (mp, ms) = (m.position(), m.size());
    match from {
        "bottom" => PhysicalPosition::new(pos.x, mp.y + ms.height as i32),
        "left" => PhysicalPosition::new(mp.x - size.width as i32, pos.y),
        "right" => PhysicalPosition::new(mp.x + ms.width as i32, pos.y),
        _ => PhysicalPosition::new(pos.x, mp.y - size.height as i32),
    }
}

fn build_url_regex(cfg: &Config) -> regex::Regex {
    let mut parts = Vec::new();
    if cfg.match_web_uris {
        parts.push(r#"(?:https?|ftp)://[^\s<>"'`{}|\\^]+"#.to_string());
        parts.push(r#"www\.[^\s<>"'`{}|\\^]+"#.to_string());
    }
    if cfg.match_file_uris {
        parts.push(r#"file://[^\s<>"'`{}|\\^]+"#.to_string());
    }
    if cfg.match_email {
        parts.push(r#"(?:mailto:)?[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)+"#.to_string());
    }
    if parts.is_empty() {
        parts.push("$^".into());
    }
    regex::Regex::new(&parts.join("|")).unwrap()
}

struct SearchState {
    pane: u64,
    query: String,
    regex: Option<alacritty_terminal::term::search::RegexSearch>,
    current: Option<alacritty_terminal::term::search::Match>,
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

/// Tab title from the program-set title and the cwd fallback (tilda's d_set_title & title_behaviour).
fn format_title(cfg: &Config, program: &str, fallback: &str) -> String {
    let base = if cfg.title.is_empty() { fallback } else { cfg.title.as_str() };
    let t = match (cfg.dynamic_title.as_str(), program.is_empty()) {
        ("none", _) => base.to_string(),
        ("before", false) => format!("{program} - {base}"),
        ("after", false) => format!("{base} - {program}"),
        (_, false) => program.to_string(),
        (_, true) => fallback.to_string(),
    };
    let n = cfg.title_max_length;
    let len = t.chars().count();
    if n == 0 || len <= n {
        return t;
    }
    match cfg.title_truncate.as_str() {
        "start" => format!("…{}", t.chars().skip(len - n).collect::<String>()),
        "none" => t,
        _ => format!("{}…", t.chars().take(n).collect::<String>()),
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
            UserEvent::ConfigChanged => self.apply_config(el),
            UserEvent::Stream(id, ev) => self.on_stream(el, id, ev),
            UserEvent::FocusPane(id) => self.focus_pane_global(el, id),
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
                    t if t.starts_with("toggle:") => {
                        let name = &t["toggle:".len()..];
                        if name == self.profile {
                            self.toggle_dropdown(el);
                        } else {
                            // another dropdown: forward to its process, or start it
                            let sock = if name.is_empty() { "dropdown".to_string() } else { format!("dropdown-{name}") };
                            if !crate::send_command(&sock, "toggle") {
                                if let Ok(exe) = std::env::current_exe() {
                                    let arg = if name.is_empty() { "--toggle".to_string() } else { format!("--toggle={name}") };
                                    let _ = std::process::Command::new(exe).arg(arg).spawn();
                                }
                            }
                        }
                    }
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
                    "quit" => self.quit(el, false),
                    "quit-nosave" => self.quit(el, true),
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
                let own_move = win.ignore_moves_until.map(|t| Instant::now() < t).unwrap_or(false);
                if win.dropdown && win.visible && !own_move && win.reapply_geometry_at.is_none() {
                    // Track the monitor the user moved the dropdown to (e.g. Super+Shift+Arrow)
                    let size = win.window.outer_size();
                    let cx = pos.x + size.width as i32 / 2;
                    let cy = pos.y + size.height.min(20) as i32 / 2;
                    if let Some(m) = Self::monitor_at(el, cx, cy) {
                        if m.name() != win.monitor {
                            win.monitor = m.name();
                            // a fullscreen window stays fullscreen on the new monitor
                            if win.window.fullscreen().is_none() {
                                self.apply_dropdown_geometry(el, wid);
                            }
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
                    // another instance may have stashed/restored tabs meanwhile
                    self.stash_count = stash::count();
                }
                let win = self.windows.get_mut(&wid).unwrap();
                win.focused = f;
                win.unfocused_at = if f { None } else { Some(Instant::now()) };
                win.window.request_redraw();
                let dropdown = win.dropdown;
                if let Some(id) = self.active_pane_id(wid) {
                    if let Some(p) = self.panes.get(&id) {
                        if p.term.lock().mode().contains(TermMode::FOCUS_IN_OUT) {
                            p.write(if f { b"\x1b[I".to_vec() } else { b"\x1b[O".to_vec() });
                        }
                    }
                }
                if dropdown {
                    if f {
                        self.pending_hide = None;
                    } else if self.dd().hide_on_focus_loss && self.windows[&wid].visible {
                        self.pending_hide =
                            Some((wid, Instant::now() + Duration::from_millis(self.dd().auto_hide_delay_ms)));
                    }
                }
            }
            WindowEvent::CursorLeft { .. } => {
                let w = &self.windows[&wid];
                if w.dropdown && w.visible && self.dd().hide_on_mouse_leave {
                    self.pending_hide =
                        Some((wid, Instant::now() + Duration::from_millis(self.dd().auto_hide_delay_ms)));
                }
            }
            WindowEvent::CursorEntered { .. } => {
                if self.pending_hide.map(|(w, _)| w == wid).unwrap_or(false) && !self.dd().hide_on_focus_loss {
                    self.pending_hide = None;
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
        let mut next = self.last_save + interval;
        if let Some(t) = self.step_anim(el) {
            next = next.min(t);
        }
        let now = Instant::now();
        let due: Vec<WindowId> =
            self.windows.iter().filter(|(_, w)| w.reapply_geometry_at.map(|t| now >= t).unwrap_or(false)).map(|(id, _)| *id).collect();
        for wid in due {
            if let Some(w) = self.windows.get_mut(&wid) {
                w.reapply_geometry_at = None;
                w.ignore_moves_until = Some(now + Duration::from_millis(500));
            }
            if self.windows.get(&wid).map(|w| w.visible && w.window.fullscreen().is_none()).unwrap_or(false) {
                self.apply_dropdown_geometry(el, wid);
            }
        }
        let due: Vec<WindowId> =
            self.windows.iter().filter(|(_, w)| w.refullscreen_at.map(|t| now >= t).unwrap_or(false)).map(|(id, _)| *id).collect();
        for wid in due {
            self.windows.get_mut(&wid).unwrap().refullscreen_at = None;
            if self.windows[&wid].visible {
                let mon = self.dropdown_monitor(el, wid);
                self.set_fullscreen(wid, true, mon);
            }
        }
        if let Some(t) = self.windows.values().filter_map(|w| w.reapply_geometry_at.into_iter().chain(w.refullscreen_at).min()).min() {
            next = next.min(t);
        }

        if let Some((w, at)) = self.pending_hide {
            if Instant::now() >= at {
                self.pending_hide = None;
                let still_unfocused = self.windows.get(&w).map(|x| !x.focused || self.dd().hide_on_mouse_leave).unwrap_or(false);
                if still_unfocused {
                    self.hide_dropdown(w);
                }
            } else {
                next = next.min(at);
            }
        }
        // Cursor blink: redraw focused windows on the blink period
        if self.cfg.cursor_blink {
            const PERIOD: u128 = 530;
            let ms = self.last_input.elapsed().as_millis();
            let wait = PERIOD - ms % PERIOD;
            next = next.min(Instant::now() + Duration::from_millis(wait as u64));
            for w in self.windows.values().filter(|w| w.focused && w.visible) {
                w.window.request_redraw();
            }
        }
        el.set_control_flow(ControlFlow::WaitUntil(next));
    }

    fn exiting(&mut self, _el: &ActiveEventLoop) {
        if !self.exiting && !self.discard_session {
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
            layout: SNode::Leaf { cwd: None, command: None, scrollback: None, title: None, remote: Some(args), remote_cwd: None },
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
        if let Some(h) = row[point.column].hyperlink().filter(|h| !h.uri().starts_with("dtem:")) {
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
                let url = if url.starts_with("www.") {
                    format!("https://{url}")
                } else if url.contains('@') && !url.contains("://") && !url.starts_with("mailto:") {
                    format!("mailto:{url}")
                } else {
                    url.to_string()
                };
                return Some((a, b, url));
            }
        }
        None
    }

    fn update_hover(&mut self, wid: WindowId) {
        let Some(win) = self.windows.get(&wid) else { return };
        let (x, y) = win.cursor;
        let mut hover = None;
        if self.modifiers.control_key() || !self.cfg.url_requires_ctrl {
            if let Some((id, rect)) = self.pane_at(wid, x, y) {
                let (point, _, _, _) = self.cell_at(wid, id, rect, x, y);
                hover = self.url_at(id, point).map(|(a, b, u)| (id, point.line.0, a, b, u));
            }
        }
        // Mouse cursor: hand on links and tabs, arrow on menu/scrollbar/borders, I-beam on text
        use winit::window::CursorIcon;
        let w = &self.windows[&wid];
        let icon = if w.menu.is_some() {
            CursorIcon::Default
        } else if hover.is_some() || w.tab_rects.iter().any(|(_, r)| r.contains(x as f32, y as f32)) {
            CursorIcon::Pointer
        } else if self.scrollbar_hit(wid, x, y).is_some() {
            CursorIcon::Default
        } else if self.pane_at(wid, x, y).is_some() {
            CursorIcon::Text
        } else {
            CursorIcon::Default
        };
        let win = self.windows.get_mut(&wid).unwrap();
        let changed = win.hover_url.as_ref().map(|h| (h.0, h.1, h.2)) != hover.as_ref().map(|h| (h.0, h.1, h.2));
        win.hover_url = hover;
        if icon != win.cursor_icon {
            win.cursor_icon = icon;
            win.window.set_cursor(icon);
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

// ---------------------------------------------------------------------- context menu
impl App {
    fn open_menu(&mut self, wid: WindowId, x: f32, y: f32, url: Option<String>) {
        let hint = |a: &str| keys::effective_combos(&self.cfg, a).first().map(|c| pretty_combo(c)).unwrap_or_default();
        let item = |label: &str, a: &'static str| MenuItem { label: label.into(), hint: hint(a), cmd: Some(MenuCmd::Action(a)) };
        let sep = || MenuItem { label: String::new(), hint: String::new(), cmd: None };
        let mut items = vec![item("New Tab", "new_tab"), item("Rename Tab…", "set_tab_title"), item("Close Tab", "close_tab"), sep()];
        items.push(item("Stash Tab", "stash_tab"));
        let n = self.stash_count;
        items.push(item(&if n > 0 { format!("Stashed Tabs ({n})…") } else { "Stashed Tabs…".to_string() }, "stashed_tabs"));
        if let Some(win) = self.windows.get(&wid) {
            for i in (0..win.workspaces.len()).filter(|i| *i != win.ws_cur) {
                items.push(MenuItem {
                    label: format!("Move Tab to “{}”", win.workspaces[i].name),
                    hint: String::new(),
                    cmd: Some(MenuCmd::MoveTab(Some(i))),
                });
            }
        }
        items.push(MenuItem { label: "Move Tab to New Workspace".into(), hint: String::new(), cmd: Some(MenuCmd::MoveTab(None)) });
        items.push(sep());
        items.push(item("Copy", "copy_to_clipboard"));
        items.push(item("Paste", "paste_from_clipboard"));
        if let Some(u) = url {
            let short: String = if u.chars().count() > 40 { format!("{}…", u.chars().take(40).collect::<String>()) } else { u.clone() };
            items.push(MenuItem { label: format!("Open Link  {short}"), hint: String::new(), cmd: Some(MenuCmd::OpenLink(u.clone())) });
            items.push(MenuItem { label: "Copy Link".into(), hint: String::new(), cmd: Some(MenuCmd::CopyLink(u)) });
        }
        items.push(sep());
        items.push(item("Split Horizontally", "hsplit"));
        items.push(item("Split Vertically", "vsplit"));
        items.push(item("Close Pane", "close_window"));
        items.push(sep());
        items.push(item("Toggle Fullscreen", "toggle_fullscreen"));
        items.push(item("Toggle Transparency", "toggle_transparency"));
        items.push(item("Toggle Searchbar", "search"));
        items.push(sep());
        items.push(item("Preferences…", "open_settings"));
        items.push(MenuItem { label: "Keyboard Shortcuts…".into(), hint: String::new(), cmd: Some(MenuCmd::Shortcuts) });
        items.push(sep());
        items.push(item("Quit", "quit"));
        self.show_menu(wid, x, y, items);
    }

    /// Open a popup menu at (x, y), kept inside the window.
    fn show_menu(&mut self, wid: WindowId, x: f32, y: f32, items: Vec<MenuItem>) {
        let Some(win) = self.windows.get_mut(&wid) else { return };
        let m = win.renderer.metrics;
        let (ww, wh) = win.renderer.size();
        let lw = items.iter().map(|i| i.label.chars().count()).max().unwrap_or(10);
        let hw = items.iter().map(|i| i.hint.chars().count()).max().unwrap_or(0);
        let w = ((lw + hw + 6) as f32 * m.width).min(ww);
        let mut menu = Menu { x, y, w, row_h: (m.height + 6.0 * win.scale as f32).round(), sep_h: 7.0, items, hover: None };
        let h = menu.height();
        if menu.x + w > ww {
            menu.x = (ww - w).max(0.0);
        }
        if menu.y + h > wh {
            menu.y = (y - h).max(0.0);
        }
        win.menu = Some(menu);
        win.window.request_redraw();
    }

    fn run_menu(&mut self, el: &ActiveEventLoop, wid: WindowId, cmd: MenuCmd) {
        match cmd {
            MenuCmd::Action(a) => {
                if let Some(a) = Action::parse(a) {
                    self.do_action(el, wid, a);
                }
            }
            MenuCmd::CopyLink(u) => self.clip_set(false, u),
            MenuCmd::OpenLink(u) => self.open_url(&u),
            MenuCmd::Shortcuts => self.open_settings_tab(el, wid, Some("keys")),
            MenuCmd::Workspace(i) => self.switch_workspace(wid, i),
            MenuCmd::CloseWorkspace(i) => {
                let n = self.windows.get(&wid).map(|w| w.ws_tabs(i).len()).unwrap_or(0);
                if n == 0 {
                    self.close_workspace(el, wid, i);
                } else {
                    self.request_close(el, wid, Confirm::Workspace(i));
                }
            }
            MenuCmd::MoveTab(to) => {
                if let Some(i) = self.windows.get(&wid).map(|w| w.active_tab) {
                    self.move_tab_to_workspace(wid, i, to);
                }
            }
        }
    }

    /// Mouse/keyboard while the menu is open. Returns true if consumed.
    fn menu_mouse(&mut self, el: &ActiveEventLoop, wid: WindowId, pressed: bool, button: MouseButton) -> bool {
        let Some(win) = self.windows.get_mut(&wid) else { return false };
        let Some(menu) = win.menu.as_ref() else { return false };
        let (x, y) = (win.cursor.0 as f32, win.cursor.1 as f32);
        if !pressed {
            return true;
        }
        let hit = menu.item_at(x, y);
        let inside = x >= menu.x && x <= menu.x + menu.w && y >= menu.y && y <= menu.y + menu.height();
        if inside && hit.is_none() {
            return true; // separator / padding
        }
        let cmd = hit.and_then(|i| menu.items[i].cmd.clone()).filter(|_| button == MouseButton::Left);
        win.menu = None;
        win.window.request_redraw();
        if let Some(c) = cmd {
            self.run_menu(el, wid, c);
        }
        true
    }

    fn menu_key(&mut self, el: &ActiveEventLoop, wid: WindowId, ev: &KeyEvent) -> bool {
        let Some(win) = self.windows.get_mut(&wid) else { return false };
        let Some(menu) = win.menu.as_mut() else { return false };
        let selectable: Vec<usize> = menu.items.iter().enumerate().filter(|(_, i)| i.cmd.is_some()).map(|(i, _)| i).collect();
        let pos = menu.hover.and_then(|h| selectable.iter().position(|&i| i == h));
        match &ev.logical_key {
            Key::Named(NamedKey::ArrowDown) => {
                menu.hover = Some(selectable[pos.map(|p| (p + 1) % selectable.len()).unwrap_or(0)]);
            }
            Key::Named(NamedKey::ArrowUp) => {
                menu.hover = Some(selectable[pos.map(|p| (p + selectable.len() - 1) % selectable.len()).unwrap_or(selectable.len() - 1)]);
            }
            Key::Named(NamedKey::Enter) => {
                let cmd = menu.hover.and_then(|h| menu.items[h].cmd.clone());
                win.menu = None;
                if let Some(c) = cmd {
                    self.run_menu(el, wid, c);
                }
            }
            _ => win.menu = None,
        }
        if let Some(w) = self.windows.get(&wid) {
            w.window.request_redraw();
        }
        true
    }

    fn open_settings_tab(&mut self, el: &ActiveEventLoop, wid: WindowId, section: Option<&str>) {
        let Ok(exe) = std::env::current_exe() else { return };
        let arg = match section {
            Some(s) => format!("--settings={s}"),
            None => "--settings".into(),
        };
        let at = self.windows.get(&wid).map(|w| w.active_tab + 1);
        let id = self.alloc_id();
        let title = if section == Some("keys") { "⌨ Shortcuts" } else { "⚙ Settings" };
        let tab = Tab { root: Node::Leaf(id), active: id, title: Some(title.into()), zoomed: false };
        let win = self.windows.get_mut(&wid).unwrap();
        let idx = at.unwrap_or(0).min(win.tabs.len());
        win.tabs.insert(idx, tab);
        win.active_tab = idx;
        self.relayout(wid);
        let r = self.windows[&wid].pane_rects.first().map(|x| x.1).unwrap_or_default();
        let opts = SpawnOpts {
            cwd: None,
            command: None,
            restore: None,
            persist_id: None,
            title: Some(title.into()),
            program: Some((exe.display().to_string(), vec![arg])),
            remote: None,
            history_seed: None,
        };
        if !self.spawn_pane(wid, id, opts, r) {
            self.remove_leaf(wid, id);
        }
        self.after_structure_change(el, wid);
    }
}
