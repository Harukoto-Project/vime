#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

#[cfg(all(windows, target_arch = "x86_64"))]
mod dashboard;
#[cfg(all(windows, target_arch = "x86_64"))]
mod desktop;
#[cfg(all(windows, target_arch = "x86_64"))]
mod hotkeys;
#[cfg(all(windows, target_arch = "x86_64"))]
mod keyboard;
#[cfg(all(windows, target_arch = "x86_64"))]
mod overlay_surface;
#[cfg(all(windows, target_arch = "x86_64"))]
mod power;
#[cfg(all(windows, target_arch = "x86_64"))]
mod single_instance;
#[cfg(all(windows, target_arch = "x86_64"))]
mod tray;

#[cfg(all(windows, target_arch = "x86_64"))]
fn main() -> eframe::Result {
    if std::env::args_os().any(|arg| arg == "--refresh-steamvr-manifest") {
        let result = vime::settings::Settings::directory()
            .and_then(|directory| vime::steamvr::refresh_startup_manifest(&directory));
        std::process::exit(i32::from(result.is_err()));
    }
    let launch_from_steamvr = std::env::args_os().any(|arg| arg == vime::steamvr::LAUNCH_ARGUMENT);
    let Some(_instance) = single_instance::start(launch_from_steamvr)
        .map_err(|error| eframe::Error::AppCreation(Box::new(error)))?
    else {
        return Ok(());
    };
    if vime::update::launch_migration() {
        return Ok(());
    }
    desktop::run(launch_from_steamvr)
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn main() {
    eprintln!("ViME requires Windows x64.");
}
