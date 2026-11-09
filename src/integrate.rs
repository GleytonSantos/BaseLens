//! Desktop integration for copies that aren't installed by a package
//! (AppImage, the .tar.gz).
//!
//! On Wayland a program can't hand its icon to the window: the desktop
//! (KDE, GNOME...) looks up a `.desktop` file whose name or StartupWMClass
//! matches the window's app id (`baselens`) and takes the icon from there.
//! Without one it shows a generic icon (KDE's "W") or nothing. So when BaseLens
//! runs from an AppImage it writes its icons and a `baselens.desktop` into the
//! person's own folders (~/.local/share), pointing at the AppImage file.
//! `--remove-desktop-entry` takes them away again.

use std::path::{Path, PathBuf};

/// Marks the files this module wrote, so it only ever updates or removes its own.
const MARK: &str = "X-BaseLens-Integrated=true";
const ICON_SIZES: [u32; 7] = [16, 32, 48, 64, 128, 256, 512];

fn data_home() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))
}

fn desktop_path(data: &Path) -> PathBuf {
    data.join("applications/baselens.desktop")
}

fn icon_path(data: &Path, n: u32) -> PathBuf {
    data.join(format!("icons/hicolor/{n}x{n}/apps/baselens.png"))
}

fn svg_path(data: &Path) -> PathBuf {
    data.join("icons/hicolor/scalable/apps/baselens.svg")
}

/// Quotes a path for the Exec line of a .desktop file.
fn exec_quote(p: &str) -> String {
    let mut s = String::from("\"");
    for c in p.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            s.push('\\');
        }
        s.push(c);
    }
    s.push('"');
    s
}

pub fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nGenericName=Database Editor\nComment={} Free, private database editor.\nExec={} %F\nTryExec={}\nIcon=baselens\nCategories=Development;Database;\nKeywords=sql;database;postgres;mysql;sqlite;json;\nStartupWMClass=baselens\nTerminal=false\n{MARK}\n",
        crate::brand::NAME,
        crate::brand::TAGLINE,
        exec_quote(exec),
        exec.replace(' ', "\\s"),
    )
}

/// What `ensure` did.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Icons and the menu entry were written or refreshed.
    Written,
    /// Everything was already in place.
    UpToDate,
    /// Another baselens.desktop (from a package or the person) is there: left alone.
    OtherEntry,
}

/// Writes the icons and a menu entry that starts `exec`, into `data`
/// (normally ~/.local/share). Never touches a baselens.desktop it didn't write.
pub fn ensure_in(data: &Path, exec: &str) -> std::io::Result<Outcome> {
    let desktop = desktop_path(data);
    let wanted = desktop_entry(exec);
    let existing = std::fs::read_to_string(&desktop).ok();
    if let Some(e) = &existing {
        if !e.contains(MARK) {
            return Ok(Outcome::OtherEntry);
        }
    }
    let icons_ok = ICON_SIZES.iter().all(|n| icon_path(data, *n).is_file()) && svg_path(data).is_file();
    if existing.as_deref() == Some(wanted.as_str()) && icons_ok {
        return Ok(Outcome::UpToDate);
    }
    for n in ICON_SIZES {
        let p = icon_path(data, n);
        std::fs::create_dir_all(p.parent().unwrap())?;
        image::save_buffer(&p, &crate::brand::icon_rgba(n), n, n, image::ExtendedColorType::Rgba8).map_err(std::io::Error::other)?;
    }
    let svg = svg_path(data);
    std::fs::create_dir_all(svg.parent().unwrap())?;
    std::fs::write(&svg, crate::brand::svg(256))?;
    std::fs::create_dir_all(desktop.parent().unwrap())?;
    let tmp = desktop.with_extension("desktop.tmp");
    std::fs::write(&tmp, wanted)?;
    std::fs::rename(&tmp, &desktop)?;
    Ok(Outcome::Written)
}

/// Removes the files `ensure_in` wrote (only if they are its own).
pub fn remove_in(data: &Path) -> std::io::Result<bool> {
    let desktop = desktop_path(data);
    match std::fs::read_to_string(&desktop) {
        Ok(e) if e.contains(MARK) => {}
        _ => return Ok(false),
    }
    std::fs::remove_file(&desktop)?;
    for n in ICON_SIZES {
        let _ = std::fs::remove_file(icon_path(data, n));
    }
    let _ = std::fs::remove_file(svg_path(data));
    Ok(true)
}

/// Tells the desktop its menu changed (in the background; best effort).
fn refresh_menus(data: &Path) {
    let apps = data.join("applications");
    let icons = data.join("icons/hicolor");
    std::thread::spawn(move || {
        let run = |cmd: &str, args: &[&std::ffi::OsStr]| {
            let _ = std::process::Command::new(cmd)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        };
        run("update-desktop-database", &[apps.as_os_str()]);
        // only refresh a GTK icon cache that is already there (never create one)
        if icons.join("icon-theme.cache").is_file() {
            run("gtk-update-icon-cache", &["-q".as_ref(), "-t".as_ref(), icons.as_os_str()]);
        }
        // KDE Plasma reads menus through its own cache
        run("kbuildsycoca6", &[]);
        run("kbuildsycoca5", &[]);
    });
}

/// Called at start-up: integrates an AppImage copy (the AppImage runtime sets
/// $APPIMAGE to the file's path). Does nothing for packaged installs.
pub fn on_startup() {
    if !cfg!(target_os = "linux") || crate::store::robot_mode() || std::env::var_os("BASELENS_NO_INTEGRATION").is_some() {
        return;
    }
    let Some(appimage) = std::env::var("APPIMAGE").ok().filter(|p| Path::new(p).is_file()) else {
        return;
    };
    let Some(data) = data_home() else { return };
    match ensure_in(&data, &appimage) {
        Ok(Outcome::Written) => {
            crate::debug::log(&format!("desktop entry and icons written for {appimage}"));
            refresh_menus(&data);
        }
        Ok(_) => {}
        Err(e) => eprintln!("BaseLens: could not add the menu entry and icon: {e}"),
    }
}

/// `--install-desktop-entry`: same as the AppImage does, for any copy (tar.gz).
pub fn install_cli() -> Result<String, String> {
    let exe = std::env::var("APPIMAGE").ok().map(PathBuf::from).or_else(|| std::env::current_exe().ok()).ok_or("can't find this program's path")?;
    let data = data_home().ok_or("can't find ~/.local/share")?;
    let exe = exe.to_string_lossy().to_string();
    match ensure_in(&data, &exe).map_err(|e| e.to_string())? {
        Outcome::OtherEntry => Ok(format!("{} already exists (from a package or made by you); left as it is.", desktop_path(&data).display())),
        o => {
            refresh_menus(&data);
            std::thread::sleep(std::time::Duration::from_millis(300));
            Ok(format!("{}: menu entry and icons for {exe}", if o == Outcome::Written { "Added" } else { "Already there" }))
        }
    }
}

/// `--remove-desktop-entry`.
pub fn remove_cli() -> Result<String, String> {
    let data = data_home().ok_or("can't find ~/.local/share")?;
    if remove_in(&data).map_err(|e| e.to_string())? {
        refresh_menus(&data);
        std::thread::sleep(std::time::Duration::from_millis(300));
        Ok("Removed the BaseLens menu entry and icons.".into())
    } else {
        Ok("No menu entry made by BaseLens was found.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_updates_and_removes_only_its_own_files() {
        let data = std::env::temp_dir().join(format!("baselens-integrate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let exe = "/home/me/Apps/Base Lens.AppImage";
        assert_eq!(ensure_in(&data, exe).unwrap(), Outcome::Written);
        let entry = std::fs::read_to_string(desktop_path(&data)).unwrap();
        assert!(entry.contains("Exec=\"/home/me/Apps/Base Lens.AppImage\" %F"));
        assert!(entry.contains("Icon=baselens") && entry.contains("StartupWMClass=baselens"));
        assert!(icon_path(&data, 256).is_file() && svg_path(&data).is_file());
        assert_eq!(ensure_in(&data, exe).unwrap(), Outcome::UpToDate);
        // the AppImage moved: the entry follows it
        assert_eq!(ensure_in(&data, "/opt/BaseLens.AppImage").unwrap(), Outcome::Written);
        assert!(remove_in(&data).unwrap());
        assert!(!desktop_path(&data).exists() && !icon_path(&data, 256).exists());
        // someone else's baselens.desktop is never touched
        std::fs::create_dir_all(desktop_path(&data).parent().unwrap()).unwrap();
        std::fs::write(desktop_path(&data), "[Desktop Entry]\nName=Mine\n").unwrap();
        assert_eq!(ensure_in(&data, exe).unwrap(), Outcome::OtherEntry);
        assert!(!remove_in(&data).unwrap());
        assert!(std::fs::read_to_string(desktop_path(&data)).unwrap().contains("Mine"));
        let _ = std::fs::remove_dir_all(&data);
    }
}
