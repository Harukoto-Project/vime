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
mod tray;

#[cfg(all(windows, target_arch = "x86_64"))]
fn main() -> eframe::Result {
    desktop::run()
}

#[cfg(not(all(windows, target_arch = "x86_64")))]
fn main() {
    eprintln!("ViME requires Windows x64.");
}
