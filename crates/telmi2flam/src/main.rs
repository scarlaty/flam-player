//! Binaire `telmi2flam` : pour l'instant, répond seulement à `--version` et `--info`.
//!
//! Fin de ligne : `\n` sur tous les systèmes. Python, en mode texte, écrit `\r\n` sous
//! Windows ; l'écart est assumé (les consommateurs lisent ligne à ligne et la parité
//! compare après `strip()`). Un tube fermé (EPIPE) n'est pas une erreur de conversion :
//! l'échec d'écriture est ignoré au lieu de faire paniquer `println!`.

use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

/// Écrit une ligne sur la sortie standard ; l'erreur d'écriture (tube fermé) est ignorée.
fn out_line(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Écrit une ligne sur la sortie d'erreur ; l'erreur d'écriture est ignorée.
fn err_line(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

fn main() -> ExitCode {
    // args_os : un chemin non Unicode ne doit pas faire paniquer le binaire.
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [flag] if flag == "--version" => {
            out_line(&format!("telmi2flam {}", telmi2flam::VERSION));
            ExitCode::SUCCESS
        }
        [a, b] if a == "--info" || b == "--info" => {
            let input = if a == "--info" { b } else { a };
            match telmi2flam::info::read_info(Path::new(input)) {
                Ok(info) => {
                    out_line(&info.to_json());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    err_line(&format!("ERREUR : {e}"));
                    ExitCode::from(e.exit_code)
                }
            }
        }
        _ => {
            err_line(
                "telmi2flam : conversion non implémentée (utilisation : --version, <archive> --info)",
            );
            ExitCode::from(2)
        }
    }
}
