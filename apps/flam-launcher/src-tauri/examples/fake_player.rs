//! Faux `flam-player` pour les tests du module `player` (jamais livré avec l'application).
//!
//! Appelé comme le vrai player : `fake_player [--stdin-control] [--parent-hwnd <n>] <chemin>`.
//! Le fichier `<chemin>` contient un scénario, une instruction par ligne : `stderr <texte>`,
//! `sleep <ms>`, `child_sleep <ms>`, `exit <code>`, `args` (écrit les options reçues sur
//! stderr), `wait_quit` (attend la ligne `quit` ou la fin de stdin, puis sort avec le code 0,
//! comme le vrai player avec `--stdin-control`).
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
    // Options du vrai player, avant le chemin de l'histoire.
    let mut options = Vec::new();
    let mut rest = args.as_slice();
    loop {
        match rest {
            [flag, rest_after @ ..] if flag == "--stdin-control" => {
                options.push(flag.clone());
                rest = rest_after;
            }
            [flag, value, rest_after @ ..] if flag == "--parent-hwnd" => {
                options.push(format!("{flag} {value}"));
                rest = rest_after;
            }
            _ => break,
        }
    }
    let [target] = rest else {
        eprintln!("attendu : un seul chemin après les options, reçu {args:?}");
        std::process::exit(4);
    };
    let Ok(script) = std::fs::read_to_string(target) else {
        eprintln!("Erreur de chargement : {target}");
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
            "args" => eprintln!("options : {}", options.join(" | ")),
            "wait_quit" => {
                let mut line = String::new();
                loop {
                    line.clear();
                    match std::io::stdin().read_line(&mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.trim() == "quit" => break,
                        Ok(_) => {}
                    }
                }
                eprintln!("arrêt demandé");
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
