//! Parité de l'analyse des arguments avec `argparse` (#133). Les vecteurs
//! `tests/data/cli_cases.json` sont produits par `tests/data/gen_cli_cases.py` : `main()` de
//! `telmi2flam.py` (Python 3.14.8) joué dans le processus, `convert` et `read_info` remplacés
//! par des enregistreurs. Pour chaque suite d'arguments : code de sortie, stdout, stderr (usage
//! et aide, `<usage>` et `<help>` dans le fichier, `<version>` pour la version) et, si une
//! conversion ou `--info` serait lancé, les arguments reçus.

use std::ffi::OsString;

use serde_json::Value;
use telmi2flam::cli::{self, Parsed};
use telmi2flam::progress::Event;

const CASES: &str = include_str!("data/cli_cases.json");

fn expand(text: &str, usage: &str, help: &str) -> String {
    text.replace("<usage><help>", &format!("{usage}{help}"))
        .replace("<usage>", usage)
        .replace("<version>", telmi2flam::VERSION)
}

#[test]
fn memes_resultats_que_argparse() {
    let doc: Value = serde_json::from_str(CASES).unwrap();
    let usage = doc["usage"].as_str().unwrap();
    let help = doc["help"].as_str().unwrap();
    assert_eq!(cli::USAGE, usage);
    assert_eq!(cli::HELP, help);
    let cases = doc["cases"].as_array().unwrap();
    assert!(cases.len() >= 900);
    let mut failures = Vec::new();
    for case in cases {
        let argv: Vec<OsString> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| OsString::from(a.as_str().unwrap()))
            .collect();
        let shown = format!("{:?}", case["argv"]);
        let expected_exit = case["exit"].as_u64().unwrap();
        let expected_out = expand(case["stdout"].as_str().unwrap(), usage, help);
        let expected_err = expand(case["stderr"].as_str().unwrap(), usage, help);
        match cli::parse(&argv) {
            Parsed::Run(args) => {
                let run = &case["run"];
                // `--info` : Python affiche le `{}` de l'enregistreur de `read_info`.
                let out_ok =
                    expected_out.is_empty() || (run["info"] == true && expected_out == "{}\n");
                if expected_exit != 0 || !out_ok || !expected_err.is_empty() {
                    failures.push(format!("{shown} : Python sort ({expected_exit})"));
                    continue;
                }
                let got = serde_json::json!({
                    "input": args.input.to_string_lossy(),
                    "info": args.info,
                });
                if run["info"] == true {
                    if got["input"] != run["input"] || !args.info {
                        failures.push(format!("{shown} : --info {got} / {run}"));
                    }
                    continue;
                }
                let full = serde_json::json!({
                    "input": args.input.to_string_lossy(),
                    "output": args.output.as_ref().map(|o| o.to_string_lossy().into_owned()),
                    "keep_size": args.keep_size,
                    "plain": args.plain,
                    "selector": args.selector.as_str(),
                    "allow_missing": args.allow_missing,
                    "progress_json": args.progress_json,
                    "info": false,
                    "thumb_fit": args.thumb_fit.as_str(),
                });
                if args.info || &full != run {
                    failures.push(format!("{shown} : {full} / {run}"));
                }
            }
            parsed => {
                let (code, mut out, err) = parsed.render().unwrap();
                // Erreur d'usage : le binaire émet d'abord l'événement `bad_arguments` si
                // `--progress-json` figure (tel quel) dans les arguments.
                if let Parsed::Error(message) = &parsed {
                    if argv.iter().any(|a| a == "--progress-json") {
                        out = format!(
                            "{}\n",
                            Event::Error {
                                code: "bad_arguments",
                                message,
                                exit: 2
                            }
                            .to_json()
                        );
                    }
                }
                if !case["run"].is_null()
                    || u64::from(code) != expected_exit
                    || out != expected_out
                    || err != expected_err
                {
                    failures.push(format!(
                        "{shown} : code {code} / {expected_exit}\n  stdout {out:?} / {expected_out:?}\n  stderr {err:?} / {expected_err:?}"
                    ));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} écart(s) :\n{}",
        failures.len(),
        failures
            .iter()
            .take(15)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
