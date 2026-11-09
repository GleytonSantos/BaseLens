//! BaseLens: a free, secure desktop database editor.

// no console window behind the app on Windows (release builds)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod ai;
mod argon2tool;
mod app;
mod assist;
mod brand;
mod db;
mod debug;
mod filter;
mod finder;
mod integrate;
mod store;
mod theme;
mod widgets;
#[cfg(test)]
mod tests;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    debug::init(&args);
    // menu entry and icon for copies not installed by a package (AppImage, tar.gz)
    for (flag, f) in [("--install-desktop-entry", integrate::install_cli as fn() -> Result<String, String>), ("--remove-desktop-entry", integrate::remove_cli)] {
        if args.iter().any(|a| a == flag) {
            match f() {
                Ok(m) => println!("{m}"),
                Err(e) => {
                    eprintln!("{flag} failed: {e}");
                    std::process::exit(1);
                }
            }
            return Ok(());
        }
    }
    // `--app-id` prints the window id the desktop will use to find the icon
    if args.iter().any(|a| a == "--app-id") {
        println!("{}", integrate::app_id());
        return Ok(());
    }
    // `--bench "<sql>" [--conn <name>]` times a query 10 times and exits
    if let Some(sql) = arg("--bench") {
        match debug::bench(&sql, arg("--conn").as_deref()) {
            Ok(report) => println!("{report}"),
            Err(e) => {
                eprintln!("bench failed: {e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    // `--export-brand <folder>` writes the logo (SVG, PNG) and a .desktop file
    if let Some(dir) = arg("--export-brand") {
        match brand::export(std::path::Path::new(&dir)) {
            Ok(_) => println!("brand files written to {dir}"),
            Err(e) => eprintln!("could not write brand files: {e}"),
        }
        return Ok(());
    }
    // test robot: `--self-test` or `--monkey <frames>` (isolated, see app/robot.rs)
    let monkey = arg("--monkey");
    if args.iter().any(|a| a == "--self-test") || monkey.is_some() {
        let dir = std::env::temp_dir().join(format!("baselens-robot-{}", std::process::id()));
        store::set_robot_mode(&dir);
        std::env::set_var("BASELENS_ROBOT", match &monkey {
            Some(n) => format!("monkey:{n}"),
            None => "selftest".into(),
        });
        let report = dir.join("robot-report.txt");
        std::panic::set_hook(Box::new(move |info| {
            let msg = format!("CRASH: {info}\n");
            eprintln!("{msg}");
            let _ = std::fs::write(&report, msg);
            std::process::exit(2);
        }));
    }
    let screenshot = arg("--screenshot").map(std::path::PathBuf::from);
    if screenshot.is_none() {
        // AppImage: Wayland desktops take the window icon from a menu entry
        integrate::on_startup();
    }
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(brand::NAME)
            .with_app_id(integrate::app_id())
            .with_icon(brand::window_icon())
            .with_inner_size(window_size())
            .with_min_inner_size([760.0, 480.0]),
        ..Default::default()
    };
    eframe::run_native(brand::NAME, options, Box::new(move |cc| Ok(Box::new(app::App::new(cc, screenshot)))))
}

/// Window size; `BASELENS_SIZE=WIDTHxHEIGHT` overrides it (used to test small screens).
fn window_size() -> [f32; 2] {
    std::env::var("BASELENS_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some([w.trim().parse().ok()?, h.trim().parse().ok()?])
        })
        .unwrap_or([1280.0, 800.0])
}
