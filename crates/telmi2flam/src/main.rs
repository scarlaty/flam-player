//! Binaire `telmi2flam` : pour l'instant, répond seulement à `--version`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => {
            println!("telmi2flam {}", telmi2flam::VERSION);
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("telmi2flam : conversion non implémentée (utilisation : --version)");
            ExitCode::from(2)
        }
    }
}
