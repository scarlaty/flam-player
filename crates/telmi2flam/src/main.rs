//! Binaire `telmi2flam` : pour l'instant, répond seulement à `--version` et `--info`.

use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    // args_os : un chemin non Unicode ne doit pas faire paniquer le binaire.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => {
            println!("telmi2flam {}", telmi2flam::VERSION);
            ExitCode::SUCCESS
        }
        [a, b] if a == "--info" || b == "--info" => {
            let input = if a == "--info" { b } else { a };
            match telmi2flam::info::read_info(Path::new(input)) {
                Ok(info) => {
                    println!("{}", info.to_json());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("ERREUR : {e}");
                    ExitCode::from(e.exit_code)
                }
            }
        }
        _ => {
            eprintln!(
                "telmi2flam : conversion non implémentée (utilisation : --version, <archive> --info)"
            );
            ExitCode::from(2)
        }
    }
}
