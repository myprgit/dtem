//! Self-installation (`dtem --install` / `--uninstall`) and config transfer
//! (`dtem --export-config` / `--import-config`). A single dtem binary is all you need.
use crate::config::{config_dir, config_path};
use std::path::{Path, PathBuf};

const EXAMPLE_CONFIG: &str = include_str!("../dtem.example.toml");

fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

fn data_home() -> PathBuf {
    dirs::data_dir().unwrap_or_else(|| home().join(".local/share"))
}

fn autostart_file() -> PathBuf {
    dirs::config_dir().unwrap_or_else(|| home().join(".config")).join("autostart/dtem-dropdown.desktop")
}

fn desktop_file() -> PathBuf {
    data_home().join("applications/dtem.desktop")
}

fn user_bin() -> PathBuf {
    home().join(".local/bin/dtem")
}

fn write(path: &Path, data: &str) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(path, data)
}

/// Install for the current user. Options: --no-autostart, --no-shortcut.
pub fn install(args: &[String]) -> anyhow::Result<()> {
    let exe = std::env::current_exe()?;
    let system = exe.starts_with("/usr/") || exe.starts_with("/opt/");
    // 1. binary: copy to ~/.local/bin (unless installed system-wide, e.g. from the .deb)
    let bin = if system {
        println!("✓ using system binary {}", exe.display());
        exe.clone()
    } else {
        let target = user_bin();
        if exe != target {
            std::fs::create_dir_all(target.parent().unwrap())?;
            let tmp = target.with_extension("new");
            std::fs::copy(&exe, &tmp)?;
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
            std::fs::rename(&tmp, &target)?; // atomic: running instances keep the old file
        }
        println!("✓ binary: {}", target.display());
        target
    };
    let b = bin.display();

    // 2. menu entry
    write(
        &desktop_file(),
        &format!(
            "[Desktop Entry]\nType=Application\nName=dtem\nComment=GPU accelerated terminal\nExec={b}\nIcon=utilities-terminal\nTerminal=false\nCategories=System;TerminalEmulator;\nStartupWMClass=dtem\nActions=dropdown;settings;\n\n[Desktop Action dropdown]\nName=Toggle dropdown\nExec={b} --toggle\n\n[Desktop Action settings]\nName=Settings\nExec={b} --settings\n"
        ),
    )?;
    println!("✓ menu entry: {}", desktop_file().display());

    // 3. dropdown at login (hidden; restores its tabs)
    if args.iter().any(|a| a == "--no-autostart") {
        let _ = std::fs::remove_file(autostart_file());
    } else {
        write(
            &autostart_file(),
            &format!("[Desktop Entry]\nType=Application\nName=dtem dropdown\nExec={b} --dropdown --hidden\nX-GNOME-Autostart-enabled=true\nNoDisplay=true\n"),
        )?;
        println!("✓ autostart: {}", autostart_file().display());
    }

    // 4. config
    if !config_path().exists() {
        write(&config_path(), EXAMPLE_CONFIG)?;
        println!("✓ config: {}", config_path().display());
    } else {
        println!("✓ config kept: {}", config_path().display());
    }

    // 5. shell integration + bundled ble.sh
    let d = crate::shellint::install_local();
    println!("✓ shell integration: {}", d.display());
    if let Some(bl) = crate::shellint::blesh() {
        println!("✓ ble.sh: {}", bl.display());
    }

    // 6. dropdown hotkey(s) as GNOME shortcuts
    let gnome = std::env::var("XDG_CURRENT_DESKTOP").map(|d| d.to_uppercase().contains("GNOME")).unwrap_or(false);
    if gnome && !args.iter().any(|a| a == "--no-shortcut") {
        let cfg = crate::config::Config::load();
        // the shortcut must call the installed binary
        match crate::install_gnome_shortcut_for(&cfg, &bin, false) {
            Ok(_) => {}
            Err(e) => println!("! GNOME shortcut: {e}"),
        }
    }

    // PATH hint
    if !system {
        let in_path = std::env::var_os("PATH")
            .map(|p| std::env::split_paths(&p).any(|d| d == user_bin().parent().unwrap()))
            .unwrap_or(false);
        if !in_path {
            println!("! ~/.local/bin is not in your PATH (log out and in, or add it to ~/.profile)");
        }
    }
    println!("\nDone. Start it with `dtem`, or the dropdown with `dtem --dropdown`.");
    Ok(())
}

/// Remove what `--install` created. `--purge` also removes config, sessions and history.
pub fn uninstall(args: &[String]) -> anyhow::Result<()> {
    for f in [desktop_file(), autostart_file()] {
        if std::fs::remove_file(&f).is_ok() {
            println!("✓ removed {}", f.display());
        }
    }
    crate::remove_gnome_shortcuts();
    println!("✓ GNOME shortcuts removed");
    let _ = std::fs::remove_dir_all(data_home().join("dtem"));
    if args.iter().any(|a| a == "--purge") {
        let _ = std::fs::remove_dir_all(config_dir());
        let _ = std::fs::remove_dir_all(crate::config::state_dir());
        println!("✓ config, sessions and history removed");
    }
    if std::fs::remove_file(user_bin()).is_ok() {
        println!("✓ removed {}", user_bin().display());
    }
    println!("Done. (running dtem instances keep working until they quit)");
    Ok(())
}

/// Pack config (dtem.toml, saved layouts, ~/.blerc) into a .tar.gz.
pub fn export_config(path: Option<&str>) -> anyhow::Result<()> {
    let out = PathBuf::from(path.unwrap_or("dtem-config.tar.gz"));
    let f = std::fs::File::create(&out)?;
    let gz = flate2::write::GzEncoder::new(f, flate2::Compression::default());
    let mut tar = tar::Builder::new(gz);
    let mut n = 0;
    if config_path().exists() {
        tar.append_path_with_name(config_path(), "dtem/dtem.toml")?;
        n += 1;
    }
    let sessions = config_dir().join("sessions");
    if sessions.is_dir() {
        tar.append_dir_all("dtem/sessions", &sessions)?;
        n += 1;
    }
    let blerc = home().join(".blerc");
    if blerc.is_file() {
        tar.append_path_with_name(&blerc, "blerc")?;
        n += 1;
    }
    tar.into_inner()?.finish()?;
    anyhow::ensure!(n > 0, "nothing to export (no config yet)");
    println!("✓ exported to {}", out.display());
    println!("  on the other PC: dtem --import-config {}", out.file_name().unwrap().to_string_lossy());
    Ok(())
}

/// Unpack an exported config; the current config is backed up first.
pub fn import_config(path: &str) -> anyhow::Result<()> {
    let f = std::fs::File::open(path)?;
    let mut ar = tar::Archive::new(flate2::read::GzDecoder::new(f));
    if config_path().exists() {
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let bak = config_path().with_extension(format!("toml.bak-{ts}"));
        std::fs::copy(config_path(), &bak)?;
        println!("✓ previous config backed up to {}", bak.display());
    }
    let cfg_root = config_dir().parent().map(|p| p.to_path_buf()).unwrap_or_else(|| home().join(".config"));
    for entry in ar.entries()? {
        let mut e = entry?;
        let p = e.path()?.into_owned();
        if p.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
            continue; // no absolute paths / ..
        }
        let target = if p == Path::new("blerc") {
            home().join(".blerc")
        } else if p.starts_with("dtem") {
            cfg_root.join(&p)
        } else {
            continue;
        };
        if let Some(d) = target.parent() {
            std::fs::create_dir_all(d)?;
        }
        e.unpack(&target)?;
        println!("✓ {}", target.display());
    }
    println!("Done. Running dtem instances apply the new config automatically.");
    Ok(())
}
