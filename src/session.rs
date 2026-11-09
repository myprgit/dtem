//! Session persistence (autosave, reboot-safe) and user session files.
use crate::config::state_dir;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum SNode {
    Leaf {
        #[serde(default)]
        cwd: Option<String>,
        /// Only used in hand-written session files: command to run (then drop to shell)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        /// Scrollback file id (autosave only)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scrollback: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    Split {
        vertical: bool,
        #[serde(default = "half")]
        ratio: f32,
        a: Box<SNode>,
        b: Box<SNode>,
    },
}

fn half() -> f32 {
    0.5
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct TabState {
    #[serde(default)]
    pub title: Option<String>,
    /// Index of the focused pane among leaves (in order)
    #[serde(default)]
    pub active: usize,
    pub layout: SNode,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct WinState {
    #[serde(default)]
    pub x: Option<i32>,
    #[serde(default)]
    pub y: Option<i32>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub maximized: bool,
    #[serde(default)]
    pub fullscreen: bool,
    /// Monitor name the window was on (used by dropdown; also helps normal windows)
    #[serde(default)]
    pub monitor: Option<String>,
    #[serde(default)]
    pub font_size: Option<f32>,
    #[serde(default)]
    pub active_tab: usize,
    pub tabs: Vec<TabState>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Session {
    #[serde(default)]
    pub version: u32,
    pub windows: Vec<WinState>,
}

pub fn session_path(role: &str) -> PathBuf {
    state_dir().join(format!("session-{role}.json"))
}

pub fn scrollback_dir(role: &str) -> PathBuf {
    state_dir().join(format!("scrollback-{role}"))
}

pub fn load(path: &Path) -> Option<Session> {
    let s = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&s) {
        Ok(s) => Some(s),
        Err(e) => {
            log::error!("cannot parse session {}: {e}", path.display());
            None
        }
    }
}

/// Atomic write (tmp + fsync + rename) so a reboot mid-write never corrupts the session.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, path)
}

pub fn save(path: &Path, s: &Session) -> std::io::Result<()> {
    let data = serde_json::to_vec_pretty(s).map_err(std::io::Error::other)?;
    write_atomic(path, &data)
}

pub fn read_scrollback(role: &str, id: &str) -> Option<Vec<u8>> {
    std::fs::read(scrollback_dir(role).join(format!("{id}.ansi"))).ok()
}

pub fn write_scrollback(role: &str, id: &str, data: &[u8]) -> std::io::Result<()> {
    write_atomic(&scrollback_dir(role).join(format!("{id}.ansi")), data)
}

/// Remove scrollback files not referenced by the session.
pub fn gc_scrollback(role: &str, keep: &[String]) {
    if let Ok(rd) = std::fs::read_dir(scrollback_dir(role)) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(id) = name.strip_suffix(".ansi") {
                if !keep.iter().any(|k| k == id) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
    }
}

/// Strip runtime-only data so the session can be shared as a layout file.
pub fn to_layout_file(s: &Session) -> Session {
    fn strip(n: &SNode) -> SNode {
        match n {
            SNode::Leaf { cwd, title, command, .. } => {
                SNode::Leaf { cwd: cwd.clone(), command: command.clone(), scrollback: None, title: title.clone() }
            }
            SNode::Split { vertical, ratio, a, b } => SNode::Split {
                vertical: *vertical,
                ratio: *ratio,
                a: Box::new(strip(a)),
                b: Box::new(strip(b)),
            },
        }
    }
    let mut s = s.clone();
    for w in &mut s.windows {
        for t in &mut w.tabs {
            t.layout = strip(&t.layout);
        }
    }
    s
}
