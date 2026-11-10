mod app;
mod confedit;
mod config;
mod font;
mod images;
mod installer;
mod keys;
mod layout;
mod pane;
mod remote;
mod renderer;
mod schemes;
mod session;
mod settings;
mod shellint;
mod stream;
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
    /// Escape sequences picked out of a pane's output
    Stream(u64, stream::StreamEvent),
    /// A desktop notification was clicked: focus this pane
    FocusPane(u64),
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
  dtem --install [--no-autostart] [--no-shortcut]
                                install this binary for the current user (menu entry,
                                dropdown autostart, GNOME hotkeys, shell integration)
  dtem --uninstall [--purge]    remove it again (--purge: also config, sessions, history)
  dtem --export-config [FILE]   pack config + saved layouts (+ ~/.blerc) into a .tar.gz
  dtem --import-config FILE     restore them on another PC
  dtem --install-gnome-shortcut register the dropdown hotkey in GNOME (X11 + Wayland)
  dtem --quit [--dropdown] [--no-save]
                                quit the running instance (--no-save: forget the session)
  dtem --icat FILE...           show images in the terminal (kitty graphics protocol)
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
    /// dropdown profile ("" = default)
    profile: String,
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
        profile: String::new(),
        install_helpers: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--dropdown" | "-d" => a.dropdown = true,
            "--toggle" => a.toggle = true,
            s if s.starts_with("--dropdown=") => {
                a.dropdown = true;
                a.profile = s["--dropdown=".len()..].to_string();
            }
            s if s.starts_with("--toggle=") => {
                a.toggle = true;
                a.profile = s["--toggle=".len()..].to_string();
            }
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
            "--install" | "--uninstall" => {
                let rest: Vec<String> = it.by_ref().collect();
                let r = if arg == "--install" { installer::install(&rest) } else { installer::uninstall(&rest) };
                if let Err(e) = r {
                    eprintln!("dtem: {e}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--export-config" => {
                let file = it.next();
                if let Err(e) = installer::export_config(file.as_deref()) {
                    eprintln!("dtem: {e}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--import-config" => {
                let Some(file) = it.next() else {
                    eprintln!("usage: dtem --import-config FILE.tar.gz");
                    std::process::exit(2);
                };
                if let Err(e) = installer::import_config(&file) {
                    eprintln!("dtem: {e}");
                    std::process::exit(1);
                }
                std::process::exit(0);
            }
            "--version" | "-V" => {
                println!("dtem {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
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
            "--icat" => {
                let files: Vec<String> = it.by_ref().collect();
                std::process::exit(icat(&files));
            }
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

pub fn send_command(role: &str, cmd: &str) -> bool {
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
    install_gnome_shortcut_for(cfg, &exe, quiet)
}

/// Remove dtem's GNOME custom shortcuts.
pub fn remove_gnome_shortcuts() {
    let mut paths = custom_paths();
    let before = paths.len();
    paths.retain(|p| !p.contains("/dtem-dropdown"));
    if paths.len() != before {
        let list = format!("[{}]", paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", "));
        gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &list]);
    }
}

pub fn install_gnome_shortcut_for(cfg: &config::Config, exe: &std::path::Path, quiet: bool) -> anyhow::Result<()> {
    let mut paths = custom_paths();
    // drop our previous entries
    paths.retain(|p| !p.contains("/dtem-dropdown"));
    let mut ours = Vec::new();
    let mut entries: Vec<(String, String, String)> = Vec::new(); // (path, command, key)
    let mut profiles: Vec<String> = vec![String::new()];
    profiles.extend(cfg.dropdowns.keys().cloned());
    for prof in &profiles {
        let d = cfg.dropdown_profile(prof);
        let slug: String = prof.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
        let cmd = if prof.is_empty() { format!("{} --toggle", exe.display()) } else { format!("{} --toggle={prof}", exe.display()) };
        for (n, key) in d.hotkey.list().iter().enumerate() {
            let path = match (prof.is_empty(), n) {
                (true, 0) => format!("{CUSTOM_PREFIX}dtem-dropdown/"),
                (true, n) => format!("{CUSTOM_PREFIX}dtem-dropdown-{}/", n + 1),
                (false, n) => format!("{CUSTOM_PREFIX}dtem-dropdown-p-{slug}-{}/", n + 1),
            };
            entries.push((path, cmd.clone(), key.clone()));
        }
    }
    for (path, cmd, key) in &entries {
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
        let schema = format!("{MEDIA_KEYS}.custom-keybinding:{path}");
        for (k, v) in [("name", "dtem dropdown"), ("command", cmd.as_str()), ("binding", accel.as_str())] {
            anyhow::ensure!(gsettings(&["set", &schema, k, v]).is_some(), "gsettings set {k} failed");
        }
        ours.push(path.clone());
        if !quiet {
            println!("GNOME shortcut: {accel} → {cmd}");
        }
    }
    paths.extend(ours);
    let list = format!("[{}]", paths.iter().map(|p| format!("'{p}'")).collect::<Vec<_>>().join(", "));
    anyhow::ensure!(gsettings(&["set", MEDIA_KEYS, "custom-keybindings", &list]).is_some(), "gsettings failed");
    Ok(())
}

/// Print images with the kitty graphics protocol (PNG payload, 4 KiB chunks).
fn icat(files: &[String]) -> i32 {
    use base64::Engine;
    use std::io::Write;
    let mut rc = 0;
    let mut out = std::io::stdout().lock();
    for f in files {
        let data = match std::fs::read(f) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("dtem --icat: {f}: {e}");
                rc = 1;
                continue;
            }
        };
        let png = if data.starts_with(b"\x89PNG") {
            data
        } else {
            match image::load_from_memory(&data) {
                Ok(img) => {
                    let mut c = std::io::Cursor::new(Vec::new());
                    if img.write_to(&mut c, image::ImageFormat::Png).is_err() {
                        rc = 1;
                        continue;
                    }
                    c.into_inner()
                }
                Err(e) => {
                    eprintln!("dtem --icat: {f}: {e}");
                    rc = 1;
                    continue;
                }
            }
        };
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let chunks: Vec<&[u8]> = b64.as_bytes().chunks(4096).collect();
        for (i, c) in chunks.iter().enumerate() {
            let m = if i + 1 < chunks.len() { 1 } else { 0 };
            let head = if i == 0 { format!("a=T,f=100,q=2,m={m}") } else { format!("m={m}") };
            let _ = write!(out, "\x1b_G{head};");
            let _ = out.write_all(c);
            let _ = write!(out, "\x1b\\");
        }
        let _ = writeln!(out);
    }
    rc
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
    let rname_owned = match (role, args.profile.as_str()) {
        (app::Role::Main, _) => "main".to_string(),
        (app::Role::Dropdown, "") => "dropdown".to_string(),
        (app::Role::Dropdown, p) => format!("dropdown-{p}"),
    };
    let rname = rname_owned.as_str();

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
        show_dropdown: !args.hidden && (args.toggle || !cfg.dropdown_profile(&args.profile).start_hidden),
        remote: args.remote.clone(),
    };
    let mut app = match app::App::new(cfg, role, args.profile.clone(), proxy, startup) {
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
