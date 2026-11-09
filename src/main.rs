mod app;
mod config;
mod font;
mod keys;
mod layout;
mod pane;
mod remote;
mod renderer;
mod session;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use winit::event_loop::EventLoop;

pub enum UserEvent {
    Term(u64, alacritty_terminal::event::Event),
    Command(String),
    Signal,
    /// Text to type into a pane (e.g. uploaded file paths)
    InsertText(u64, String),
    Toast(u64, String),
    /// File fetched from a remote host via `dtem-copy`
    RemoteFile(u64, String, Vec<u8>),
    Redraw(winit::window::WindowId),
}

const HELP: &str = "dtem — GPU accelerated terminal

USAGE:
  dtem                          open a normal window (restores the last session)
  dtem --dropdown               start the dropdown terminal (shown)
  dtem --dropdown --hidden      start the dropdown hidden (for autostart)
  dtem --toggle                 show/hide the dropdown (starts it if needed)
  dtem --session FILE.json      open a layout/session file
  dtem --remote [ssh args] HOST open an ssh session in a new tab (clipboard/file sync)
  dtem --no-restore             don't restore the previous session
  dtem --install-gnome-shortcut register the dropdown hotkey in GNOME (X11 + Wayland)
  dtem --quit [--dropdown]      quit the running instance (session is saved)

Config:  ~/.config/dtem/dtem.toml
Session: ~/.local/state/dtem/";

struct Args {
    dropdown: bool,
    toggle: bool,
    hidden: bool,
    no_restore: bool,
    session: Option<PathBuf>,
    install_shortcut: bool,
    quit: bool,
    remote: Option<Vec<String>>,
}

fn parse_args() -> Args {
    let mut a = Args {
        dropdown: false,
        toggle: false,
        hidden: false,
        no_restore: false,
        session: None,
        install_shortcut: false,
        quit: false,
        remote: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dropdown" | "-d" => a.dropdown = true,
            "--toggle" => a.toggle = true,
            "--hidden" => a.hidden = true,
            "--no-restore" => a.no_restore = true,
            "--session" => {
                a.session = it.next().map(|p| std::fs::canonicalize(&p).unwrap_or_else(|_| PathBuf::from(p)))
            }
            "--install-gnome-shortcut" => a.install_shortcut = true,
            "--quit" => a.quit = true,
            "--remote" | "-r" => {
                let rest: Vec<String> = it.by_ref().collect();
                if rest.is_empty() {
                    eprintln!("--remote needs an ssh destination, e.g. dtem --remote user@host");
                    std::process::exit(2);
                }
                a.remote = Some(rest);
            }
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }
    a
}

fn socket_path(role: &str) -> PathBuf {
    let dir = dirs::runtime_dir().unwrap_or_else(|| {
        let d = PathBuf::from(format!("/tmp/dtem-{}", unsafe { libc::getuid() }));
        let _ = std::fs::create_dir_all(&d);
        d
    });
    dir.join(format!("dtem-{role}.sock"))
}

fn send_command(role: &str, cmd: &str) -> bool {
    match UnixStream::connect(socket_path(role)) {
        Ok(mut s) => s.write_all(format!("{cmd}\n").as_bytes()).is_ok(),
        Err(_) => false,
    }
}

/// Convert "ctrl+shift+F12" style to a global_hotkey HotKey.
pub fn hotkey_from_config(s: &str) -> Option<global_hotkey::hotkey::HotKey> {
    let parts: Vec<String> = s.split('+').map(|p| p.trim().to_lowercase()).collect();
    let mut mods = Vec::new();
    for p in &parts[..parts.len() - 1] {
        mods.push(match p.as_str() {
            "ctrl" | "control" => "control",
            "shift" => "shift",
            "alt" => "alt",
            "super" | "meta" | "cmd" => "super",
            _ => return None,
        });
    }
    let key = parts.last()?;
    let key = match key.as_str() {
        "grave" | "`" => "Backquote".to_string(),
        "space" => "Space".to_string(),
        "enter" | "return" => "Enter".to_string(),
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphabetic()) => format!("Key{}", k.to_uppercase()),
        k if k.len() == 1 && k.chars().all(|c| c.is_ascii_digit()) => format!("Digit{k}"),
        k if k.starts_with('f') && k[1..].parse::<u8>().is_ok() => k.to_uppercase(),
        k => k.to_string(),
    };
    let mut spec = mods.join("+");
    if !spec.is_empty() {
        spec.push('+');
    }
    spec.push_str(&key);
    spec.parse().ok()
}

/// GNOME accelerator syntax for gsettings, e.g. "<Control><Shift>F12".
fn gnome_accel(s: &str) -> String {
    let parts: Vec<String> = s.split('+').map(|p| p.trim().to_string()).collect();
    let mut out = String::new();
    for p in &parts[..parts.len() - 1] {
        out.push_str(match p.to_lowercase().as_str() {
            "ctrl" | "control" => "<Control>",
            "shift" => "<Shift>",
            "alt" => "<Alt>",
            "super" | "meta" | "cmd" => "<Super>",
            _ => "",
        });
    }
    let key = parts.last().cloned().unwrap_or_default();
    let lk = key.to_lowercase();
    let k = if lk.starts_with('f') && lk[1..].parse::<u8>().is_ok() {
        lk.to_uppercase()
    } else if lk == "`" {
        "grave".into()
    } else {
        lk
    };
    out.push_str(&k);
    out
}

fn install_gnome_shortcut(cfg: &config::Config) -> anyhow::Result<()> {
    use std::process::Command;
    let exe = std::env::current_exe()?;
    let base = "org.gnome.settings-daemon.plugins.media-keys";
    let path = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/dtem-dropdown/";
    let out = Command::new("gsettings").args(["get", base, "custom-keybindings"]).output()?;
    let cur = String::from_utf8_lossy(&out.stdout).to_string();
    let mut items: Vec<String> = cur.split('\'').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, s)| s.to_string()).collect();
    if !items.iter().any(|p| p == path) {
        items.push(path.to_string());
    }
    let list = format!("[{}]", items.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", "));
    let schema = format!("{base}.custom-keybinding:{path}");
    let accel = gnome_accel(&cfg.dropdown.hotkey);
    let cmd = format!("{} --toggle", exe.display());
    for (k, v) in [("name", "dtem dropdown"), ("command", cmd.as_str()), ("binding", accel.as_str())] {
        let st = Command::new("gsettings").args(["set", &schema, k, v]).status()?;
        anyhow::ensure!(st.success(), "gsettings set {k} failed");
    }
    let st = Command::new("gsettings").args(["set", base, "custom-keybindings", &list]).status()?;
    anyhow::ensure!(st.success(), "gsettings set custom-keybindings failed");
    println!("GNOME shortcut installed: {accel} → {cmd}");
    Ok(())
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn,dtem=info")).init();
    let args = parse_args();
    let cfg = config::Config::load();

    if args.install_shortcut {
        if let Err(e) = install_gnome_shortcut(&cfg) {
            eprintln!("failed: {e}");
            std::process::exit(1);
        }
        return;
    }

    let role = if args.dropdown || args.toggle { app::Role::Dropdown } else { app::Role::Main };
    let rname = role.name();

    // Already running? Forward the request and exit.
    let cmd = if args.quit {
        "quit".to_string()
    } else {
        match (role, &args.session) {
            (app::Role::Dropdown, _) if args.hidden => String::new(),
            (app::Role::Dropdown, _) => "toggle".into(),
            (app::Role::Main, _) if args.remote.is_some() => {
                format!("remote {}", serde_json::to_string(args.remote.as_ref().unwrap()).unwrap())
            }
            (app::Role::Main, Some(p)) => format!("session {}", p.display()),
            (app::Role::Main, None) => "new-window".into(),
        }
    };
    if UnixStream::connect(socket_path(rname)).is_ok() {
        if !cmd.is_empty() {
            send_command(rname, &cmd);
        }
        return;
    }
    if args.quit {
        return;
    }

    let sock = socket_path(rname);
    let _ = std::fs::remove_file(&sock);
    let listener = match UnixListener::bind(&sock) {
        Ok(l) => Some(l),
        Err(e) => {
            log::warn!("IPC socket unavailable: {e}");
            None
        }
    };

    // Backend selection. The dropdown needs absolute positioning + monitor control, which
    // Wayland (GNOME has no layer-shell) doesn't allow → run it through XWayland.
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let has_x = std::env::var_os("DISPLAY").is_some();
    let use_x11 = wayland && has_x && (role == app::Role::Dropdown || cfg.force_x11);
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    if use_x11 {
        use winit::platform::x11::EventLoopBuilderExtX11;
        builder.with_x11();
        log::info!("using XWayland backend for {rname}");
    }
    let event_loop = match builder.build() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("cannot create event loop: {e}");
            std::process::exit(1);
        }
    };
    let proxy = event_loop.create_proxy();

    if let Some(l) = listener {
        let p = proxy.clone();
        std::thread::spawn(move || {
            for stream in l.incoming().flatten() {
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() && !line.trim().is_empty() {
                    let _ = p.send_event(UserEvent::Command(line.trim().to_string()));
                }
            }
        });
    }
    {
        use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
        let p = proxy.clone();
        if let Ok(mut sigs) = signal_hook::iterator::Signals::new([SIGTERM, SIGHUP, SIGINT]) {
            std::thread::spawn(move || {
                if sigs.forever().next().is_some() {
                    let _ = p.send_event(UserEvent::Signal);
                }
            });
        }
    }

    let startup = app::Startup {
        restore: !args.no_restore,
        session_file: args.session.clone(),
        show_dropdown: !args.hidden,
        remote: args.remote.clone(),
    };
    let mut app = match app::App::new(cfg, role, proxy, startup) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("dtem: {e}");
            let _ = std::fs::remove_file(&sock);
            std::process::exit(1);
        }
    };
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("event loop: {e}");
    }
    let _ = std::fs::remove_file(&sock);
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    #[test]
    fn hotkeys() {
        assert!(super::hotkey_from_config("F12").is_some());
        assert!(super::hotkey_from_config("ctrl+grave").is_some());
        assert!(super::hotkey_from_config("ctrl+shift+t").is_some());
        assert_eq!(super::gnome_accel("ctrl+shift+F12"), "<Control><Shift>F12");
        assert_eq!(super::gnome_accel("F12"), "F12");
    }
}
