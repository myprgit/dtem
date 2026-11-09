mod app;
mod confedit;
mod config;
mod font;
mod keys;
mod layout;
mod pane;
mod remote;
mod renderer;
mod schemes;
mod session;
mod settings;
mod tilda;
mod x11;

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
    /// dtem.toml changed on disk
    ConfigChanged,
}

const HELP: &str = "dtem — GPU accelerated terminal

USAGE:
  dtem                          open a normal window (restores the last session)
  dtem --dropdown               start the dropdown terminal (shown)
  dtem --dropdown --hidden      start the dropdown hidden (for autostart)
  dtem --toggle                 show/hide the dropdown (starts it if needed)
  dtem --session FILE.json      open a layout/session file
  dtem --remote [ssh args] HOST ssh with clipboard/file sync (inside dtem: in the current pane)
  dtem --remote --new-tab HOST  ssh in a new tab of the running dtem
  dtem --no-restore             don't restore the previous session
  dtem --install-gnome-shortcut register the dropdown hotkey in GNOME (X11 + Wayland)
  dtem --quit [--dropdown] [--no-save]
                                quit the running instance (--no-save: forget the session)
  dtem --settings[=keys]        settings page / shortcut editor (also ctrl+, or right click)
  dtem --import-tilda [FILE]    import a tilda config (default ~/.config/tilda/config_0)

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
    no_save: bool,
    settings: Option<Option<String>>,
    import_tilda: Option<Option<PathBuf>>,
    remote: Option<Vec<String>>,
    new_tab: bool,
    install_helpers: Option<Vec<String>>,
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
        no_save: false,
        settings: None,
        import_tilda: None,
        remote: None,
        new_tab: false,
        install_helpers: None,
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
            "--no-save" => a.no_save = true,
            "--settings" => a.settings = Some(None),
            s if s.starts_with("--settings=") => a.settings = Some(Some(s["--settings=".len()..].to_string())),
            "--import-tilda" => {
                let next = it.next();
                if next.as_deref().map(|n| n.starts_with('-')).unwrap_or(false) {
                    eprintln!("--import-tilda takes an optional config file path");
                    std::process::exit(2);
                }
                a.import_tilda = Some(next.map(PathBuf::from));
            }
            "--install-remote-helpers" => {
                a.install_helpers = Some(it.by_ref().collect());
            }
            "--new-tab" => a.new_tab = true,
            "--remote" | "-r" => {
                let mut rest: Vec<String> = it.by_ref().collect();
                if rest.first().map(|s| s == "--new-tab").unwrap_or(false) {
                    rest.remove(0);
                    a.new_tab = true;
                }
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
pub fn gnome_accel(s: &str) -> String {
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

const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const CUSTOM_PREFIX: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/";

fn gsettings(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("gsettings").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn unquote(s: &str) -> String {
    s.trim().trim_matches('\'').to_string()
}

fn custom_paths() -> Vec<String> {
    gsettings(&["get", MEDIA_KEYS, "custom-keybindings"])
        .map(|cur| cur.split('\'').enumerate().filter(|(i, _)| i % 2 == 1).map(|(_, s)| s.to_string()).collect())
        .unwrap_or_default()
}

fn custom_get(path: &str, key: &str) -> String {
    gsettings(&["get", &format!("{MEDIA_KEYS}.custom-keybinding:{path}"), key]).map(|s| unquote(&s)).unwrap_or_default()
}

/// GNOME accelerators bound to `dtem --toggle`.
pub fn gnome_dtem_accels() -> Vec<String> {
    custom_paths()
        .iter()
        .filter(|p| custom_get(p, "command").contains("--toggle") && custom_get(p, "command").contains("dtem"))
        .map(|p| custom_get(p, "binding").to_lowercase())
        .collect()
}

fn accel_norm(a: &str) -> String {
    a.to_lowercase().replace("<primary>", "<control>").replace("<ctrl>", "<control>")
}

/// Register every dropdown hotkey as a GNOME custom shortcut running `dtem --toggle`.
/// Other custom shortcuts using the same key are disabled (reported).
pub fn install_gnome_shortcut(cfg: &config::Config, quiet: bool) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let cmd = format!("{} --toggle", exe.display());
    let keys = cfg.dropdown.hotkey.list();
    let mut paths = custom_paths();
    // drop our previous entries
    paths.retain(|p| !p.contains("/dtem-dropdown"));
    let mut ours = Vec::new();
    for (n, key) in keys.iter().enumerate() {
        let accel = gnome_accel(key);
        // disable conflicting shortcuts (e.g. tilda on Ctrl+Alt+T)
        for p in &paths {
            if accel_norm(&custom_get(p, "binding")) == accel_norm(&accel) {
                let name = custom_get(p, "name");
                gsettings(&["set", &format!("{MEDIA_KEYS}.custom-keybinding:{p}"), "binding", ""]);
                if !quiet {
                    println!("disabled GNOME shortcut \"{name}\" ({accel}) — it now opens dtem");
                }
            }
        }
        let path = if n == 0 { format!("{CUSTOM_PREFIX}dtem-dropdown/") } else { format!("{CUSTOM_PREFIX}dtem-dropdown-{}/", n + 1) };
        let schema = format!("{MEDIA_KEYS}.custom-keybinding:{path}");
        for (k, v) in [("name", "dtem dropdown"), ("command", cmd.as_str()), ("binding", accel.as_str())] {
            anyhow::ensure!(gsettings(&["set", &schema, k, v]).is_some(), "gsettings set {k} failed");
        }
        ours.push(path);
        if !quiet {
            println!("GNOME shortcut: {accel} → {cmd}");
        }
    }
    paths.extend(ours);
    let list = format!("[{}]", paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", "));
    anyhow::ensure!(gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &list]).is_some(), "gsettings failed");
    Ok(())
}

fn watch_config(proxy: winit::event_loop::EventLoopProxy<UserEvent>) {
    std::thread::spawn(move || {
        let path = config::config_path();
        let mtime = |p: &std::path::Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        let mut last = mtime(&path);
        loop {
            std::thread::sleep(std::time::Duration::from_millis(400));
            let now = mtime(&path);
            if now != last {
                last = now;
                if proxy.send_event(UserEvent::ConfigChanged).is_err() {
                    return;
                }
            }
        }
    });
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let args = parse_args();
    let cfg = config::Config::load();

    if args.install_shortcut {
        if let Err(e) = install_gnome_shortcut(&cfg, false) {
            eprintln!("failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(section) = args.settings.clone() {
        if let Err(e) = settings::run(section) {
            eprintln!("settings: {e}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(p) = &args.import_tilda {
        let path = p.clone().unwrap_or_else(|| dirs::config_dir().unwrap_or_default().join("tilda/config_0"));
        match tilda::import(&path) {
            Ok(r) => {
                println!("Imported {} settings from {} into {}", r.applied, path.display(), config::config_path().display());
                if let Some(b) = r.backup {
                    println!("Previous config backed up to {}", b.display());
                }
                if !r.ignored.is_empty() {
                    println!("Not applicable: {}", r.ignored.join(", "));
                }
            }
            Err(e) => {
                eprintln!("import failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // Background helper installer started by an inline `dtem --remote`
    if let Some(rargs) = args.install_helpers.clone() {
        let r = remote::Remote::new_multiplexed(rargs);
        for _ in 0..300 {
            if r.is_connected() {
                let _ = r.install_helpers();
                return;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
        return;
    }
    // `dtem --remote host` typed inside a dtem pane: run ssh right here (like `kitten ssh`).
    // The multiplexed ssh is detected as the pane's foreground process, so image/file paste
    // and dtem-copy work without a separate tab.
    if let Some(rargs) = args.remote.clone() {
        let inside_dtem = std::env::var("TERM_PROGRAM").map(|v| v == "dtem").unwrap_or(false);
        if inside_dtem && !args.new_tab {
            if remote::split_ssh_args(&rargs).is_none() {
                eprintln!("--remote: no ssh destination given");
                std::process::exit(2);
            }
            let r = remote::Remote::new_multiplexed(rargs.clone());
            if cfg.remote_install_helpers {
                if let Ok(exe) = std::env::current_exe() {
                    use std::process::Stdio;
                    let _ = std::process::Command::new(exe)
                        .arg("--install-remote-helpers")
                        .args(&rargs)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .spawn();
                }
            }
            use std::os::unix::process::CommandExt;
            let err = std::process::Command::new("ssh").args(r.session_argv()).exec();
            eprintln!("cannot run ssh: {err}");
            std::process::exit(1);
        }
    }

    let role = if args.dropdown || args.toggle { app::Role::Dropdown } else { app::Role::Main };
    let rname = role.name();

    // Already running? Forward the request and exit.
    let cmd = if args.quit {
        if args.no_save { "quit-nosave".to_string() } else { "quit".to_string() }
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
    watch_config(proxy.clone());

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
        // `dtem --toggle` always shows; `dtem --dropdown` honours dropdown.start_hidden
        show_dropdown: !args.hidden && (args.toggle || !cfg.dropdown.start_hidden),
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
