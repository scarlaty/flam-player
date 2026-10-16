//! Faux `flam-player` pour les tests du module `player` (jamais livré avec l'application).
//!
//! Appelé comme le vrai player : `fake_player <chemin>`. Le fichier `<chemin>` contient un
//! scénario, une instruction par ligne : `stderr <texte>`, `sleep <ms>`, `child_sleep <ms>`, `exit <code>`.
//! Fichier illisible : message sur stderr et code 3 (le vrai player échoue aussi au chargement).

use std::io::Write;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let [flag, ms] = args.as_slice() {
        if flag == "--sleep" {
            std::thread::sleep(Duration::from_millis(ms.parse().unwrap_or(0)));
            return;
        }
    }
    if args.len() != 1 {
        eprintln!("attendu : un seul argument, reçu {args:?}");
        std::process::exit(4);
    }
    let Ok(script) = std::fs::read_to_string(&args[0]) else {
        eprintln!("Erreur de chargement : {}", args[0]);
        std::process::exit(3);
    };
    for line in script.lines() {
        let (op, rest) = line.split_once(' ').unwrap_or((line, ""));
        match op {
            "stderr" => {
                eprintln!("{rest}");
                let _ = std::io::stderr().flush();
            }
            "sleep" => std::thread::sleep(Duration::from_millis(rest.parse().unwrap_or(0))),
            // Processus fils qui hérite de stderr et survit au « player ».
            "child_sleep" => {
                let _ = std::process::Command::new(std::env::current_exe().unwrap())
                    .args(["--sleep", rest])
                    .spawn();
            }
            "exit" => std::process::exit(rest.parse().unwrap_or(0)),
            _ => {}
        }
    }
}
