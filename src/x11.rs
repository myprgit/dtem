//! Small EWMH helpers for the dropdown on X11 (activation that survives focus-stealing
//! prevention, skip-taskbar, sticky, keep-above).
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, PropMode};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_WM_STATE,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_STATE_SKIP_PAGER,
        _NET_WM_STATE_STICKY,
        _NET_WM_STATE_ABOVE,
        _NET_WM_DESKTOP,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _NET_WM_WINDOW_TYPE_UTILITY,
    }
}

pub struct X11 {
    conn: RustConnection,
    root: u32,
    atoms: Atoms,
}

/// The X11 window id of a winit window (None on Wayland).
pub fn window_id(w: &winit::window::Window) -> Option<u32> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match w.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => Some(h.window as u32),
        RawWindowHandle::Xcb(h) => Some(h.window.get()),
        _ => None,
    }
}

impl X11 {
    pub fn connect() -> Option<X11> {
        let (conn, screen) = x11rb::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        Some(X11 { conn, root, atoms })
    }

    fn client_message(&self, win: u32, typ: u32, data: [u32; 5]) {
        let ev = ClientMessageEvent::new(32, win, typ, data);
        let _ = self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
            ev,
        );
        let _ = self.conn.flush();
    }

    /// Raise + focus. Source indication 2 ("pager") is honoured by mutter/kwin even
    /// when the request doesn't come with a fresh user timestamp.
    pub fn activate(&self, win: u32) {
        self.client_message(win, self.atoms._NET_ACTIVE_WINDOW, [2, 0, 0, 0, 0]);
    }

    fn state(&self, win: u32, add: bool, a: u32, b: u32) {
        self.client_message(win, self.atoms._NET_WM_STATE, [add as u32, a, b, 2, 0]);
    }

    /// Apply dropdown window states on a mapped window.
    pub fn apply(&self, win: u32, skip_taskbar: bool, sticky: bool, above: bool) {
        let a = &self.atoms;
        self.state(win, skip_taskbar, a._NET_WM_STATE_SKIP_TASKBAR, a._NET_WM_STATE_SKIP_PAGER);
        self.state(win, sticky, a._NET_WM_STATE_STICKY, 0);
        self.state(win, above, a._NET_WM_STATE_ABOVE, 0);
        if sticky {
            self.client_message(win, a._NET_WM_DESKTOP, [0xFFFF_FFFF, 2, 0, 0, 0]);
        }
    }

    /// Window type: utility (no GNOME open/close animation) or normal.
    /// mutter only lets *normal* windows go fullscreen, so fullscreen switches to normal.
    pub fn set_normal_type(&self, win: u32, normal: bool) {
        let a = &self.atoms;
        let t = if normal { a._NET_WM_WINDOW_TYPE_NORMAL } else { a._NET_WM_WINDOW_TYPE_UTILITY };
        let _ = self.conn.change_property32(PropMode::REPLACE, win, a._NET_WM_WINDOW_TYPE, AtomEnum::ATOM, &[t]);
        let _ = self.conn.flush();
    }

    /// Initial states for a not-yet-mapped window (read by the WM at map time).
    pub fn preset(&self, win: u32, skip_taskbar: bool, sticky: bool, above: bool) {
        let a = &self.atoms;
        let mut v = Vec::new();
        if skip_taskbar {
            v.push(a._NET_WM_STATE_SKIP_TASKBAR);
            v.push(a._NET_WM_STATE_SKIP_PAGER);
        }
        if sticky {
            v.push(a._NET_WM_STATE_STICKY);
        }
        if above {
            v.push(a._NET_WM_STATE_ABOVE);
        }
        let _ = self.conn.change_property32(PropMode::REPLACE, win, a._NET_WM_STATE, AtomEnum::ATOM, &v);
        if sticky {
            let _ = self.conn.change_property32(PropMode::REPLACE, win, a._NET_WM_DESKTOP, AtomEnum::CARDINAL, &[0xFFFF_FFFF]);
        }
        let _ = self.conn.flush();
    }
}
