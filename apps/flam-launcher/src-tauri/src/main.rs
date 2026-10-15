// Évite la fenêtre console supplémentaire sous Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    flam_launcher_lib::run();
}
