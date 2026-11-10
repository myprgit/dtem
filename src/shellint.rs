//! Shell integration (OSC 133 prompt/command marks + OSC 7 cwd) for bash, zsh and fish.
//! The user's own startup files are untouched: dtem loads them first, then adds hooks.
use std::path::PathBuf;

pub const BASH: &str = include_str!("shell/dtem.bash");
pub const ZSHENV: &str = include_str!("shell/zsh/.zshenv");
pub const FISH: &str = include_str!("shell/dtem.fish");
pub const LAUNCHER: &str = include_str!("shell/dtem-shell");
pub const ICAT: &str = include_str!("shell/dtem-icat");

/// ble.sh (BSD-3-Clause, https://github.com/akinomyoga/ble.sh) bundled with dtem.
static BLESH_TGZ: &[u8] = include_bytes!("../vendor/blesh.tar.gz");

/// ble.sh for suggestions while typing: a user-installed one (~/.local/share/blesh) wins,
/// otherwise the bundled copy is unpacked to ~/.local/share/dtem/blesh.
pub fn blesh() -> Option<PathBuf> {
    let own = dirs::data_dir()?.join("blesh/ble.sh");
    if own.is_file() {
        return Some(own);
    }
    static ONCE: std::sync::Once = std::sync::Once::new();
    let dir = dirs::data_dir()?.join("dtem/blesh");
    ONCE.call_once(|| {
        let stamp = format!("{}", BLESH_TGZ.len());
        let stamp_file = dir.join(".dtem-stamp");
        if std::fs::read_to_string(&stamp_file).map(|s| s == stamp).unwrap_or(false) && dir.join("ble.sh").is_file() {
            return;
        }
        let tmp = dir.with_extension("tmp");
        let _ = std::fs::remove_dir_all(&tmp);
        let ok = (|| -> std::io::Result<()> {
            std::fs::create_dir_all(&tmp)?;
            let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(BLESH_TGZ));
            for entry in ar.entries()? {
                let mut e = entry?;
                let path = e.path()?.into_owned();
                // strip the top-level "ble-nightly/" directory; refuse anything odd
                let rel: PathBuf = path.components().skip(1).collect();
                if rel.as_os_str().is_empty() || rel.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
                    continue;
                }
                let out = tmp.join(&rel);
                if let Some(p) = out.parent() {
                    std::fs::create_dir_all(p)?;
                }
                e.unpack(&out)?;
            }
            std::fs::write(tmp.join(".dtem-stamp"), &stamp)?;
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::rename(&tmp, &dir)?;
            Ok(())
        })();
        if let Err(e) = ok {
            log::warn!("cannot unpack bundled ble.sh: {e}");
        }
    });
    let p = dir.join("ble.sh");
    p.is_file().then_some(p)
}

pub fn dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("dtem/shell")
}

fn write_if_changed(path: &std::path::Path, data: &str) {
    if std::fs::read_to_string(path).map(|s| s == data).unwrap_or(false) {
        return;
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, data);
}

/// Write the integration scripts locally (once per process).
pub fn install_local() -> PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let d = dir();
    ONCE.call_once(|| {
        write_if_changed(&d.join("dtem.bash"), BASH);
        write_if_changed(&d.join("zsh/.zshenv"), ZSHENV);
        write_if_changed(&d.join("dtem.fish"), FISH);
    });
    d
}

/// Program arguments and environment to start `shell` with integration.
/// Returns None for shells we don't integrate with.
pub fn launch(shell: &str, login: bool) -> Option<(Vec<String>, Vec<(String, String)>)> {
    let name = std::path::Path::new(shell).file_name()?.to_str()?.to_string();
    let d = install_local();
    match name.as_str() {
        "bash" => {
            let mut env = Vec::new();
            if login {
                env.push(("DTEM_BASH_LOGIN".into(), "1".into()));
            }
            Some((vec!["--rcfile".into(), d.join("dtem.bash").display().to_string(), "-i".into()], env))
        }
        "zsh" => {
            let orig = std::env::var("ZDOTDIR").unwrap_or_default();
            let env = vec![
                ("DTEM_ORIG_ZDOTDIR".into(), orig),
                ("ZDOTDIR".into(), d.join("zsh").display().to_string()),
            ];
            Some((if login { vec!["-l".into()] } else { vec![] }, env))
        }
        "fish" => {
            let mut args = Vec::new();
            if login {
                args.push("-l".into());
            }
            args.push("--init-command".into());
            args.push(format!("source '{}'", d.join("dtem.fish").display()));
            Some((args, vec![]))
        }
        _ => None,
    }
}

/// Shell script (for `sh -s`) that installs dtem's helpers on a remote host.
pub fn remote_installer(copy_helper: &str) -> String {
    let files: [(&str, &str, bool); 6] = [
        (".local/bin/dtem-copy", copy_helper, true),
        (".local/bin/dtem-icat", ICAT, true),
        (".local/bin/dtem-shell", LAUNCHER, true),
        (".local/share/dtem/shell/dtem.bash", BASH, false),
        (".local/share/dtem/shell/zsh/.zshenv", ZSHENV, false),
        (".local/share/dtem/shell/dtem.fish", FISH, false),
    ];
    let mut s = String::from("set -e\ncd \"$HOME\"\nmkdir -p .local/bin .local/share/dtem/shell/zsh\n");
    for (i, (path, content, exec)) in files.iter().enumerate() {
        s.push_str(&format!("cat > \"{path}\" <<'DTEM_EOF_{i}'\n{content}\nDTEM_EOF_{i}\n"));
        if *exec {
            s.push_str(&format!("chmod +x \"{path}\"\n"));
        }
    }
    s
}
