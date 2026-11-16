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
        /// ssh arguments for remote panes (`dtem --remote ...`)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote: Option<Vec<String>>,
        /// directory on the remote host (reported by shell integration)
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote_cwd: Option<String>,
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
    /// Tabs of the window. With workspaces this is the flattened list of all of them
    /// (so older dtem versions still restore every tab); `workspaces` wins when present.
    pub tabs: Vec<TabState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<WorkspaceState>,
    #[serde(default)]
    pub active_workspace: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct WorkspaceState {
    pub name: String,
    #[serde(default)]
    pub active_tab: usize,
    pub tabs: Vec<TabState>,
}

impl WinState {
    /// (name, active tab, tabs) per workspace; one unnamed workspace for old sessions.
    pub fn workspace_list(&self) -> Vec<(Option<String>, usize, Vec<TabState>)> {
        if self.workspaces.is_empty() {
            vec![(None, self.active_tab, self.tabs.clone())]
        } else {
            self.workspaces.iter().map(|w| (Some(w.name.clone()), w.active_tab, w.tabs.clone())).collect()
        }
    }

    pub fn has_tabs(&self) -> bool {
        !self.tabs.is_empty() || self.workspaces.iter().any(|w| !w.tabs.is_empty())
    }
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
            SNode::Leaf { cwd, title, command, remote, remote_cwd, .. } => SNode::Leaf {
                cwd: cwd.clone(),
                command: command.clone(),
                scrollback: None,
                title: title.clone(),
                remote: remote.clone(),
                remote_cwd: remote_cwd.clone(),
            },
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
        for t in w.tabs.iter_mut().chain(w.workspaces.iter_mut().flat_map(|ws| ws.tabs.iter_mut())) {
            t.layout = strip(&t.layout);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_session_is_one_unnamed_workspace() {
        let old = r#"{"version":1,"windows":[{"active_tab":1,"tabs":[
            {"layout":{"Leaf":{"cwd":"/tmp"}}},{"title":"b","layout":{"Leaf":{"cwd":"/"}}}]}]}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        let w = &s.windows[0];
        assert!(w.has_tabs());
        let l = w.workspace_list();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].0, None);
        assert_eq!(l[0].1, 1);
        assert_eq!(l[0].2.len(), 2);
    }

    #[test]
    fn workspaces_round_trip() {
        let tab = |t: &str| TabState {
            title: Some(t.into()),
            active: 0,
            layout: SNode::Leaf { cwd: None, command: None, scrollback: None, title: None, remote: None, remote_cwd: None },
        };
        let w = WinState {
            tabs: vec![tab("a"), tab("b")],
            workspaces: vec![
                WorkspaceState { name: "Work".into(), active_tab: 0, tabs: vec![tab("a")] },
                WorkspaceState { name: "Home".into(), active_tab: 0, tabs: vec![tab("b")] },
            ],
            active_workspace: 1,
            ..Default::default()
        };
        let json = serde_json::to_string(&Session { version: 1, windows: vec![w] }).unwrap();
        let s: Session = serde_json::from_str(&json).unwrap();
        let l = s.windows[0].workspace_list();
        assert_eq!(l.iter().map(|x| x.0.clone().unwrap()).collect::<Vec<_>>(), vec!["Work", "Home"]);
        assert_eq!(s.windows[0].active_workspace, 1);
        // older versions read the flattened list
        assert_eq!(s.windows[0].tabs.len(), 2);
    }
}

