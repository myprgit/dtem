//! Shell integration (OSC 133 prompt/command marks + OSC 7 cwd) for bash, zsh and fish.
//! The user's own startup files are untouched: dtem loads them first, then adds hooks.
use std::path::PathBuf;

pub const BASH: &str = include_str!("shell/dtem.bash");
pub const ZSHENV: &str = include_str!("shell/zsh/.zshenv");
pub const FISH: &str = include_str!("shell/dtem.fish");
pub const LAUNCHER: &str = include_str!("shell/dtem-shell");
pub const ICAT: &str = include_str!("shell/dtem-icat");

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
