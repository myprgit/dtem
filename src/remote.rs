//! SSH remote support: `dtem --remote host`, clipboard / file transfer over the
//! multiplexed SSH connection (ControlMaster), remote helper installation.
use anyhow::{bail, Context};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// ssh options that take an argument (see `man ssh`).
const ARG_OPTS: &str = "BbcDEeFIiJLlmOoPpQRSWw";

#[derive(Clone, Debug, PartialEq)]
pub struct Remote {
    /// ssh options + destination (no remote command)
    pub args: Vec<String>,
    /// Uses our ControlMaster socket (created by `dtem --remote`)
    pub multiplexed: bool,
}

pub fn control_path() -> String {
    let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
    format!("{}/dtem-ssh-%C", dir.display())
}

pub fn shell_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@+,%".contains(c)) {
        s.to_string()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

/// Split ssh argv (without the leading "ssh") into options+destination and the remote command.
pub fn split_ssh_args(args: &[String]) -> Option<(Vec<String>, Vec<String>)> {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            i += 1;
            break;
        }
        if let Some(flags) = a.strip_prefix('-') {
            if flags.is_empty() {
                return None;
            }
            // e.g. -p22 (attached) or -p 22 (separate) or -tt (flags only)
            let mut chars = flags.chars();
            while let Some(c) = chars.next() {
                if ARG_OPTS.contains(c) {
                    if chars.as_str().is_empty() {
                        i += 1; // value is the next argument
                    }
                    break;
                }
            }
            i += 1;
            continue;
        }
        break;
    }
    if i >= args.len() {
        return None;
    }
    // args[i] is the destination
    Some((args[..=i].to_vec(), args[i + 1..].to_vec()))
}

impl Remote {
    pub fn new_multiplexed(args: Vec<String>) -> Remote {
        Remote { args, multiplexed: true }
    }

    /// The destination as shown to the user (user@host).
    pub fn label(&self) -> String {
        split_ssh_args(&self.args).and_then(|(o, _)| o.last().cloned()).unwrap_or_else(|| self.args.join(" "))
    }

    /// argv for the interactive session.
    pub fn session_argv(&self) -> Vec<String> {
        let mut v = vec![
            "-o".into(),
            "ControlMaster=auto".into(),
            "-o".into(),
            format!("ControlPath={}", control_path()),
            "-o".into(),
            "ControlPersist=60".into(),
        ];
        v.extend(self.args.iter().cloned());
        v
    }

    /// Shell script run in the pane: connects and offers to reconnect on disconnect.
    /// `ask_first` = show the reconnect prompt before the first connection (restored sessions).
    pub fn pane_script(&self, ask_first: bool) -> String {
        let argv = self.session_argv().iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
        let label = shell_quote(&self.label());
        format!(
            r#"ask={ask}
while :; do
  if [ "$ask" = 1 ]; then
    printf '\r\n\033[2m[dtem] %s — Enter: reconnect, Ctrl+D: close\033[0m ' {label}
    read -r _ || exit 0
  fi
  ask=1
  ssh {argv}
  rc=$?
  [ $rc -eq 0 ] && exit 0
done"#,
            ask = if ask_first { 1 } else { 0 },
        )
    }

    fn command(&self) -> Command {
        let mut c = Command::new("ssh");
        if self.multiplexed {
            c.args(["-o", &format!("ControlPath={}", control_path())]);
        }
        c.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"]);
        c.args(&self.args);
        c
    }

    fn run(&self, remote_cmd: &str, input: Option<&[u8]>) -> anyhow::Result<Vec<u8>> {
        let mut cmd = self.command();
        cmd.arg(remote_cmd).stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().context("cannot run ssh")?;
        if let Some(data) = input {
            let mut stdin = child.stdin.take().unwrap();
            stdin.write_all(data)?;
        }
        let out = child.wait_with_output()?;
        if !out.status.success() {
            bail!("ssh: {}", String::from_utf8_lossy(&out.stderr).trim());
        }
        Ok(out.stdout)
    }

    /// Upload bytes to the remote; returns the absolute remote path.
    /// Clipboard images go to a private temp dir (0700, files 0600, removed after 24h);
    /// other files go to ~/.cache/dtem/uploads.
    pub fn upload(&self, data: &[u8], name: &str, clipboard_image: bool) -> anyhow::Result<String> {
        let name: String =
            name.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
        let dir = if clipboard_image {
            "d=\"${TMPDIR:-/tmp}/dtem-clipboard-$(id -u)\"; umask 077; mkdir -p \"$d\" && chmod 700 \"$d\" && \
             find \"$d\" -type f -mmin +1440 -delete 2>/dev/null;"
        } else {
            "d=\"$HOME/.cache/dtem/uploads\"; mkdir -p \"$d\" &&"
        };
        let script = format!("{dir} cat > \"$d\"/{n} && printf '%s' \"$d\"/{n}", n = shell_quote(&name));
        let out = self.run(&script, Some(data))?;
        let path = String::from_utf8_lossy(&out).trim().to_string();
        if path.is_empty() {
            bail!("upload failed");
        }
        Ok(path)
    }

    pub fn fetch(&self, path: &str) -> anyhow::Result<Vec<u8>> {
        self.run(&format!("cat -- {}", shell_quote(path)), None)
    }

    pub fn is_connected(&self) -> bool {
        if !self.multiplexed {
            return true;
        }
        Command::new("ssh")
            .args(["-o", &format!("ControlPath={}", control_path()), "-O", "check"])
            .args(&self.args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Install `dtem-copy` on the remote (in ~/.local/bin).
    pub fn install_helpers(&self) -> anyhow::Result<()> {
        let script = "mkdir -p \"$HOME/.local/bin\" && cat > \"$HOME/.local/bin/dtem-copy\" && chmod +x \"$HOME/.local/bin/dtem-copy\"";
        self.run(script, Some(HELPER.as_bytes()))?;
        Ok(())
    }
}

/// Remote helper: copy text (stdin) or a file/image to the *local* clipboard through dtem.
pub const HELPER: &str = r#"#!/bin/sh
# dtem-copy — copy to your local clipboard through the dtem terminal.
#   echo hello | dtem-copy     copy text
#   dtem-copy screenshot.png   copy an image/file (dtem fetches it over ssh)
osc() {
  if [ -n "$TMUX" ]; then printf '\033Ptmux;\033\033]52;c;%s\a\033\\' "$1"
  else printf '\033]52;c;%s\a' "$1"; fi > /dev/tty
}
b64() { base64 | tr -d '\n'; }
if [ $# -gt 0 ]; then
  for f in "$@"; do
    p=$(readlink -f -- "$f") || { echo "dtem-copy: $f: not found" >&2; exit 1; }
    [ -f "$p" ] || { echo "dtem-copy: $f: not a file" >&2; exit 1; }
    osc "$(printf 'dtem-file:%s' "$p" | b64)"
  done
else
  osc "$(b64)"
fi
"#;

/// If the pane's foreground process is `ssh`, derive a Remote from its command line.
pub fn detect_foreground_ssh(pty_fd: i32) -> Option<Remote> {
    let pg = unsafe { libc::tcgetpgrp(pty_fd) };
    if pg <= 0 {
        return None;
    }
    let raw = std::fs::read(format!("/proc/{pg}/cmdline")).ok()?;
    let argv: Vec<String> =
        raw.split(|b| *b == 0).filter(|s| !s.is_empty()).map(|s| String::from_utf8_lossy(s).to_string()).collect();
    let prog = PathBuf::from(argv.first()?);
    if prog.file_name()?.to_str()? != "ssh" {
        return None;
    }
    let (opts, _cmd) = split_ssh_args(&argv[1..])?;
    let multiplexed = opts.iter().any(|a| a.contains("ControlPath=") && a.contains("dtem-ssh-"));
    let args = if multiplexed {
        // strip our own -o options; they are re-added by Remote::command
        let mut v = Vec::new();
        let mut i = 0;
        while i < opts.len() {
            if opts[i] == "-o"
                && opts.get(i + 1).map(|o| o.starts_with("Control")).unwrap_or(false)
            {
                i += 2;
                continue;
            }
            v.push(opts[i].clone());
            i += 1;
        }
        v
    } else {
        opts
    };
    Some(Remote { args, multiplexed })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }
    #[test]
    fn split() {
        let (o, c) = split_ssh_args(&v(&["-p", "2222", "-A", "user@host", "ls", "-l"])).unwrap();
        assert_eq!(o, v(&["-p", "2222", "-A", "user@host"]));
        assert_eq!(c, v(&["ls", "-l"]));
        let (o, c) = split_ssh_args(&v(&["-p2222", "-tt", "host"])).unwrap();
        assert_eq!(o, v(&["-p2222", "-tt", "host"]));
        assert!(c.is_empty());
        assert!(split_ssh_args(&v(&["-p", "22"])).is_none());
        assert_eq!(Remote::new_multiplexed(v(&["-i", "k", "me@srv"])).label(), "me@srv");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }
}
