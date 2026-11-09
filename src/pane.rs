//! A terminal pane: PTY + alacritty_terminal state + persistence helpers.
use crate::config::Config;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config as TermConfig, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{Color, NamedColor, Processor};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use winit::event_loop::EventLoopProxy;

use crate::UserEvent;

#[derive(Clone)]
pub struct Listener {
    pub pane: u64,
    pub proxy: EventLoopProxy<UserEvent>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let _ = self.proxy.send_event(UserEvent::Term(self.pane, event));
    }
}

pub struct Pane {
    #[allow(dead_code)]
    pub id: u64,
    /// Stable id used for the scrollback file name
    pub persist_id: String,
    pub term: Arc<FairMutex<Term<Listener>>>,
    sender: EventLoopSender,
    pub pid: u32,
    /// PTY master fd (owned by the event loop thread; valid while the pane lives)
    pty_fd: i32,
    pub title: String,
    pub cols: usize,
    pub rows: usize,
    /// Content changed since last save
    pub dirty: bool,
    /// Last known cwd (kept if the process can't be inspected anymore)
    pub last_cwd: Option<PathBuf>,
    pub exited: bool,
    /// Pane opened with `dtem --remote`
    pub remote: Option<crate::remote::Remote>,
}

pub struct SpawnOpts {
    pub cwd: Option<PathBuf>,
    pub command: Option<String>,
    pub restore: Option<Vec<u8>>,
    pub persist_id: Option<String>,
    pub title: Option<String>,
    /// Run this program directly instead of the shell (used for remote panes)
    pub program: Option<(String, Vec<String>)>,
    pub remote: Option<crate::remote::Remote>,
}

pub fn new_persist_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    format!("{:x}-{:x}-{:x}", t.as_secs(), t.subsec_nanos(), C.fetch_add(1, Ordering::Relaxed) ^ std::process::id() as u64)
}

impl Pane {
    #[allow(clippy::too_many_arguments)]
    pub fn spawn(
        id: u64,
        cfg: &Config,
        opts: SpawnOpts,
        cols: usize,
        rows: usize,
        cell_w: f32,
        cell_h: f32,
        proxy: EventLoopProxy<UserEvent>,
    ) -> anyhow::Result<Pane> {
        let cols = cols.max(2);
        let rows = rows.max(1);
        let listener = Listener { pane: id, proxy };
        let tcfg = TermConfig {
            scrolling_history: cfg.scrollback(),
            kitty_keyboard: cfg.kitty_keyboard,
            semantic_escape_chars: cfg.semantic_escape_chars(),
            // Reads are gated by `clipboard_read` in the app (ask/allow/deny)
            osc52: alacritty_terminal::term::Osc52::CopyPaste,
            ..Default::default()
        };
        let mut term = Term::new(tcfg, &TermSize::new(cols, rows), listener.clone());

        if let Some(bytes) = opts.restore.as_ref().filter(|b| !b.is_empty()) {
            let mut p: Processor = Processor::new();
            p.advance(&mut term, bytes);
            p.advance(&mut term, b"\x1b[0m\r\n");
        }

        let cwd = opts.cwd.filter(|p| p.is_dir()).or_else(dirs::home_dir);
        let shell = cfg.shell();
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-256color".to_string());
        env.insert("COLORTERM".to_string(), "truecolor".to_string());
        env.insert("TERM_PROGRAM".to_string(), "dtem".to_string());
        let shell_opt = match &opts.command {
            _ if opts.program.is_some() => {
                let (prog, args) = opts.program.clone().unwrap();
                Some(tty::Shell::new(prog, args))
            }
            Some(cmd) if !cmd.trim().is_empty() => {
                // The configured new-tab command follows `command_exit`; session-file commands
                // always drop to a shell afterwards.
                let exit_mode = if cmd.trim() == cfg.command.trim() { cfg.command_exit.as_str() } else { "shell" };
                let login = if cfg.login_shell { " -l" } else { "" };
                let script = match exit_mode {
                    "close" => cmd.clone(),
                    "restart" => format!("while :; do {cmd}; printf '\\r\\n\\r\\n'; done"),
                    _ => format!("{cmd}; exec {shell}{login}"),
                };
                let mut args = Vec::new();
                if cfg.login_shell {
                    args.push("-l".to_string());
                }
                args.push("-c".into());
                args.push(script);
                Some(tty::Shell::new(shell.clone(), args))
            }
            _ => Some(tty::Shell::new(shell.clone(), if cfg.login_shell { vec!["-l".into()] } else { vec![] })),
        };
        let topts = tty::Options { shell: shell_opt, working_directory: cwd.clone(), drain_on_exit: false, env };
        let ws = WindowSize {
            num_lines: rows as u16,
            num_cols: cols as u16,
            cell_width: cell_w as u16,
            cell_height: cell_h as u16,
        };
        let pty = tty::new(&topts, ws, id)?;
        let pid = pty.child().id();
        let pty_fd = {
            use std::os::fd::AsRawFd;
            pty.file().as_raw_fd()
        };
        let term = Arc::new(FairMutex::new(term));
        let el = EventLoop::new(term.clone(), listener, pty, false, false)?;
        let sender = el.channel();
        el.spawn();
        Ok(Pane {
            id,
            persist_id: opts.persist_id.unwrap_or_else(new_persist_id),
            term,
            sender,
            pid,
            pty_fd,
            title: opts.title.unwrap_or_default(),
            cols,
            rows,
            dirty: true,
            last_cwd: cwd,
            exited: false,
            remote: opts.remote,
        })
    }

    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let _ = self.sender.send(Msg::Input(bytes.into()));
    }

    pub fn resize(&mut self, cols: usize, rows: usize, cell_w: f32, cell_h: f32) {
        let cols = cols.max(2);
        let rows = rows.max(1);
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        self.term.lock().resize(TermSize::new(cols, rows));
        let _ = self.sender.send(Msg::Resize(WindowSize {
            num_lines: rows as u16,
            num_cols: cols as u16,
            cell_width: cell_w as u16,
            cell_height: cell_h as u16,
        }));
    }

    pub fn shutdown(&self) {
        let _ = self.sender.send(Msg::Shutdown);
    }

    /// Current working directory of the shell (via /proc).
    pub fn cwd(&mut self) -> Option<PathBuf> {
        if let Ok(p) = std::fs::read_link(format!("/proc/{}/cwd", self.pid)) {
            self.last_cwd = Some(p);
        }
        self.last_cwd.clone()
    }

    /// SSH target for clipboard/file transfer: the pane's own remote, or a foreground `ssh`.
    pub fn remote_target(&self) -> Option<crate::remote::Remote> {
        if let Some(r) = &self.remote {
            return Some(r.clone());
        }
        crate::remote::detect_foreground_ssh(self.pty_fd)
    }

    /// True if the shell itself is in the foreground (no program running).
    pub fn shell_idle(&self) -> bool {
        let pg = unsafe { libc::tcgetpgrp(self.pty_fd) };
        pg > 0 && pg as u32 == self.pid
    }

    /// Title used when the program didn't set one: ⇄ host for remote panes, else the cwd.
    pub fn fallback_title(&mut self) -> String {
        let saved = std::mem::take(&mut self.title);
        let t = self.display_title();
        self.title = saved;
        t
    }

    /// Name of the foreground process (e.g. "nvim"), if any other than the shell.
    pub fn display_title(&mut self) -> String {
        if !self.title.is_empty() {
            return self.title.clone();
        }
        if let Some(r) = &self.remote {
            return format!("⇄ {}", r.label());
        }
        self.cwd()
            .map(|p| {
                let s = p.display().to_string();
                if let Some(home) = dirs::home_dir() {
                    let h = home.display().to_string();
                    if s == h {
                        return "~".to_string();
                    }
                    if let Some(rest) = s.strip_prefix(&(h + "/")) {
                        return format!("~/{rest}");
                    }
                }
                s
            })
            .unwrap_or_else(|| "shell".into())
    }

    /// Serialize primary screen + scrollback to ANSI bytes. Returns cached dump while on alt screen.
    pub fn dump_scrollback(&mut self, max_lines: usize) -> Option<Vec<u8>> {
        // When the shell is idle the cursor line is just its prompt; the restored shell prints
        // a fresh one, so skip it to avoid stacking prompts on every restart.
        let skip_prompt = self.shell_idle();
        let mut term = self.term.lock();
        let data = if term.mode().contains(TermMode::ALT_SCREEN) {
            // An app (vim, htop...) is on the alternate screen. Temporarily swap the primary
            // screen in to read it, then put the alternate screen back exactly as it was.
            let alt = term.grid().clone();
            term.swap_alt();
            let data = serialize_grid(&term, max_lines, skip_prompt);
            term.swap_alt(); // this resets the alt grid ...
            *term.grid_mut() = alt; // ... so restore its contents and cursor
            data
        } else {
            serialize_grid(&term, max_lines, skip_prompt)
        };
        drop(term);
        Some(data)
    }
}

fn color_sgr(c: Color, fg: bool, out: &mut String) {
    let base = if fg { 30 } else { 40 };
    match c {
        Color::Named(n) => {
            let i = n as usize;
            match n {
                NamedColor::Foreground | NamedColor::BrightForeground | NamedColor::DimForeground if fg => {}
                NamedColor::Background if !fg => {}
                _ if i < 8 => {
                    let _ = write!(out, ";{}", base + i);
                }
                _ if i < 16 => {
                    let _ = write!(out, ";{}", base + 60 + i - 8);
                }
                _ if (259..=266).contains(&i) => {
                    let _ = write!(out, ";{}", base + i - 259);
                }
                _ => {}
            }
        }
        Color::Indexed(i) => {
            let _ = write!(out, ";{};5;{}", base + 8, i);
        }
        Color::Spec(rgb) => {
            let _ = write!(out, ";{};2;{};{};{}", base + 8, rgb.r, rgb.g, rgb.b);
        }
    }
}

fn cell_sgr(cell: &Cell) -> String {
    let mut s = String::from("0");
    let f = cell.flags;
    if f.contains(Flags::BOLD) {
        s.push_str(";1");
    }
    if f.contains(Flags::DIM) {
        s.push_str(";2");
    }
    if f.contains(Flags::ITALIC) {
        s.push_str(";3");
    }
    if f.intersects(Flags::ALL_UNDERLINES) {
        s.push_str(";4");
    }
    if f.contains(Flags::INVERSE) {
        s.push_str(";7");
    }
    if f.contains(Flags::HIDDEN) {
        s.push_str(";8");
    }
    if f.contains(Flags::STRIKEOUT) {
        s.push_str(";9");
    }
    color_sgr(cell.fg, true, &mut s);
    color_sgr(cell.bg, false, &mut s);
    s
}

fn is_blank(cell: &Cell) -> bool {
    (cell.c == ' ' || cell.c == '\0')
        && matches!(cell.bg, Color::Named(NamedColor::Background))
        && !cell.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES)
}

pub fn serialize_grid<T: EventListener>(term: &Term<T>, max_lines: usize, skip_prompt: bool) -> Vec<u8> {
    let grid = term.grid();
    let history = grid.history_size() as i32;
    let cols = grid.columns();
    let cursor_line = grid.cursor.point.line.0;
    // Last line that has content (don't persist trailing empty screen lines)
    let mut end = cursor_line;
    for l in (cursor_line..grid.screen_lines() as i32).rev() {
        if (0..cols).any(|c| !is_blank(&grid[Line(l)][Column(c)])) {
            end = l;
            break;
        }
    }
    if skip_prompt && end == cursor_line {
        end -= 1;
        // a wrapped prompt spans several rows: drop them all
        while end >= -history && grid[Line(end)][Column(cols - 1)].flags.contains(Flags::WRAPLINE) {
            end -= 1;
        }
    }
    if end < -history {
        return Vec::new();
    }
    let start = (-history).max(end - max_lines as i32 + 1);
    let mut out = String::new();
    let mut cur_sgr = String::from("0");
    for l in start..=end {
        let row = &grid[Line(l)];
        let wrapped = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
        let last = if wrapped {
            cols
        } else {
            (0..cols).rev().find(|&c| !is_blank(&row[Column(c)])).map(|c| c + 1).unwrap_or(0)
        };
        for c in 0..last {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let sgr = cell_sgr(cell);
            if sgr != cur_sgr {
                let _ = write!(out, "\x1b[{sgr}m");
                cur_sgr = sgr;
            }
            out.push(if cell.c == '\0' { ' ' } else { cell.c });
            if let Some(zw) = cell.zerowidth() {
                out.extend(zw.iter());
            }
        }
        if !wrapped && l != end {
            if cur_sgr != "0" {
                out.push_str("\x1b[0m");
                cur_sgr = "0".into();
            }
            out.push_str("\r\n");
        }
    }
    out.push_str("\x1b[0m");
    out.into_bytes()
}
