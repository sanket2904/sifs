// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    sifs_lib::extract::serve_helper_if_asked();
    #[cfg(feature = "app")]
    sifs_lib::run();
}
