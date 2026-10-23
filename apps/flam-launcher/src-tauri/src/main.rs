// Évite la fenêtre console supplémentaire sous Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Processus enfant de décodage des pochettes (#263) : un panic d'un décodeur d'image n'arrête
    // que cet enfant. Avant toute initialisation (fenêtre, instance unique, journal).
    if std::env::args_os()
        .nth(1)
        .is_some_and(|a| a == flam_launcher_lib::podcast::artwork::DECODE_ARG)
    {
        std::process::exit(flam_launcher_lib::podcast::artwork::run_child());
    }
    flam_launcher_lib::run();
}
