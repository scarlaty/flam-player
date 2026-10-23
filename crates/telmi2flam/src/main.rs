//! Binaire `telmi2flam` : conversion (`<archive> [-o sortie]`), `--info`, `--version`, et
//! protocole de `telmi2flam.py` (#131) :
//! - avec `--progress-json`, **stdout ne contient que les événements JSON**, une ligne chacun,
//!   vidangée aussitôt ; stdout fermé par le parent : l'émission cesse sans erreur et la
//!   conversion continue (comme `_print_event`) ;
//! - chaque avertissement écrit aussi `  ! <message>` sur stderr, avec ou sans
//!   `--progress-json` (contrat de `compat.py`) ; les erreurs écrivent `ERREUR : <message>` ;
//! - codes de sortie : 0 succès, 2 arguments invalides (`bad_arguments`), 3 archive, 4 contenu,
//!   5 asset manquant, 6 sortie ou erreur interne, 7 délégation à Python
//!   (`unsupported_format`, décision D4), 130 annulation.
//!
//! Reste à faire ailleurs : analyse des options façon `argparse` (formes `--opt=valeur`,
//! préfixes, `-h`), résumé humain `OK -> …` (#133) ; annulation par signal ou par fermeture
//! de stdin et crochet de panique (#132) : une panique sort aujourd'hui avec 101, sans
//! événement `error`.
//!
//! Fin de ligne : `\n` sur tous les systèmes. Python, en mode texte, écrit `\r\n` sous
//! Windows ; l'écart est assumé (les consommateurs lisent ligne à ligne et la parité
//! compare après `strip()`).

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;

use telmi2flam::convert::{convert, Request};
use telmi2flam::image::ThumbFit;
use telmi2flam::package::{Options, Selector};
use telmi2flam::progress::{Event, Reporter};

/// Écrit une ligne sur la sortie standard ; l'erreur d'écriture (tube fermé) est ignorée.
fn out_line(line: &str) {
    let _ = writeln!(std::io::stdout().lock(), "{line}");
}

/// Écrit une ligne sur la sortie d'erreur ; l'erreur d'écriture est ignorée.
fn err_line(line: &str) {
    let _ = writeln!(std::io::stderr().lock(), "{line}");
}

/// Pile du fil de travail : filet de sécurité de la documentation de la crate (#189). Le fil
/// principal n'a qu'1 Mo sous Windows ; un débordement de pile aborte le processus sans
/// message, contrairement à une erreur `invalid_content`.
const WORKER_STACK: usize = 8 << 20;

const USAGE: &str = "usage: telmi2flam [-h] [--version] [-o OUTPUT] [--plain] [--keep-size] \
[--allow-missing] [--selector {carousel,image}] [--thumb-fit {flou,rogner,bandes}] \
[--progress-json] [--info] input";

fn main() -> ExitCode {
    let worker = std::thread::Builder::new()
        .name("telmi2flam".to_owned())
        .stack_size(WORKER_STACK)
        .spawn(run);
    match worker {
        // Une panique du fil est relancée ici : même sortie (code 101) que sans fil.
        Ok(handle) => handle
            .join()
            .unwrap_or_else(|panic| std::panic::resume_unwind(panic)),
        // Fil impossible à créer (mémoire) : le travail tourne quand même, sur la pile
        // principale, suffisante pour les histoires réelles.
        Err(_) => run(),
    }
}

/// Événements JSON sur stdout (`_print_event`).
struct Events {
    enabled: bool,
    /// stdout fermé ou cassé : plus aucune émission.
    dead: bool,
}

impl Events {
    fn new(enabled: bool) -> Self {
        Events {
            enabled,
            dead: false,
        }
    }

    fn emit(&mut self, event: &Event<'_>) {
        if !self.enabled || self.dead {
            return;
        }
        let mut out = std::io::stdout().lock();
        let line = event.to_json();
        if writeln!(out, "{line}").and_then(|()| out.flush()).is_err() {
            self.dead = true;
        }
    }

    /// Échec : événement `error`, message humain sur stderr, code de sortie.
    fn fail(&mut self, code: &str, message: &str, exit: u8, human: Option<&str>) -> ExitCode {
        self.emit(&Event::Error {
            code,
            message,
            exit,
        });
        match human {
            Some(h) => err_line(h),
            None => err_line(&format!("ERREUR : {message}")),
        }
        ExitCode::from(exit)
    }
}

/// Arguments reconnus.
#[derive(Debug, Default)]
struct Args {
    input: Option<OsString>,
    output: Option<OsString>,
    plain: bool,
    keep_size: bool,
    allow_missing: bool,
    selector: Selector,
    thumb_fit: ThumbFit,
    progress_json: bool,
    info: bool,
    version: bool,
}

/// Analyse minimale des arguments (formes séparées seulement : `-o SORTIE`) ; `Err` : message
/// d'erreur d'usage (code 2).
fn parse(argv: &[OsString]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = argv.iter();
    let mut extra: Vec<String> = Vec::new();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .filter(|v| !v.as_encoded_bytes().starts_with(b"-"))
                .cloned()
                .ok_or_else(|| format!("argument {name}: expected one argument"))
        };
        match arg.to_str() {
            Some("--version") => args.version = true,
            Some("-o" | "--output") => args.output = Some(value("-o/--output")?),
            Some("--plain") => args.plain = true,
            Some("--keep-size") => args.keep_size = true,
            Some("--allow-missing") => args.allow_missing = true,
            Some("--progress-json") => args.progress_json = true,
            Some("--info") => args.info = true,
            Some("--selector") => {
                let v = value("--selector")?;
                args.selector = match v.to_str() {
                    Some("carousel") => Selector::Carousel,
                    Some("image") => Selector::Image,
                    _ => {
                        return Err(format!(
                            "argument --selector: invalid choice: {:?} \
                             (choose from carousel, image)",
                            v.to_string_lossy()
                        ))
                    }
                };
            }
            Some("--thumb-fit") => {
                let v = value("--thumb-fit")?;
                args.thumb_fit = v.to_str().and_then(ThumbFit::from_name).ok_or_else(|| {
                    format!(
                        "argument --thumb-fit: invalid choice: {:?} \
                         (choose from flou, rogner, bandes)",
                        v.to_string_lossy()
                    )
                })?;
            }
            _ if arg.as_encoded_bytes().starts_with(b"-") && arg.len() > 1 => {
                extra.push(arg.to_string_lossy().into_owned());
            }
            _ if args.input.is_none() => args.input = Some(arg.clone()),
            _ => extra.push(arg.to_string_lossy().into_owned()),
        }
    }
    if args.version {
        return Ok(args);
    }
    if args.input.is_none() {
        return Err("the following arguments are required: input".to_owned());
    }
    if !extra.is_empty() {
        return Err(format!("unrecognized arguments: {}", extra.join(" ")));
    }
    Ok(args)
}

fn run() -> ExitCode {
    // args_os : un chemin non Unicode ne doit pas faire paniquer le binaire.
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    let args = match parse(&argv) {
        Ok(args) => args,
        Err(message) => {
            // Comme `_Parser.error` : l'événement d'abord, si `--progress-json` est présent.
            let mut events = Events::new(argv.iter().any(|a| a == "--progress-json"));
            return events.fail(
                "bad_arguments",
                &message,
                2,
                Some(&format!("{USAGE}\ntelmi2flam: error: {message}")),
            );
        }
    };
    if args.version {
        out_line(&format!("telmi2flam {}", telmi2flam::VERSION));
        return ExitCode::SUCCESS;
    }
    let mut events = Events::new(args.progress_json);
    let input = Path::new(args.input.as_deref().unwrap_or(OsStr::new("")));
    if args.info {
        return match telmi2flam::info::read_info(input) {
            Ok(info) => {
                out_line(&info.to_json());
                ExitCode::SUCCESS
            }
            Err(e) => events.fail(e.code, &e.message, e.exit_code, None),
        };
    }
    let req = Request {
        input,
        output: args.output.as_deref().map(Path::new),
        emit_plain: args.plain,
        options: Options {
            keep_size: args.keep_size,
            allow_missing: args.allow_missing,
            selector: args.selector,
            thumb_fit: args.thumb_fit,
        },
    };
    let result = {
        let mut sink = |e: &Event<'_>| {
            // `_warn` : la ligne de stderr d'abord, puis l'événement.
            if let Event::Warning { message, .. } = e {
                err_line(&format!("  ! {message}"));
            }
            events.emit(e);
        };
        let mut reporter = Reporter::new(&mut sink);
        convert(&req, &mut reporter, &|| false)
    };
    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) if e.code == "cancelled" => events.fail(
            e.code,
            &e.message,
            e.exit_code,
            Some("Annule : aucune sortie partielle conservee."),
        ),
        Err(e) => events.fail(e.code, &e.message, e.exit_code, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Args, String> {
        parse(&list.iter().map(OsString::from).collect::<Vec<_>>())
    }

    #[test]
    fn options_du_launcher() {
        let a = args(&[
            "in.zip",
            "-o",
            "out.plain.pk",
            "--progress-json",
            "--thumb-fit",
            "bandes",
        ])
        .unwrap();
        assert_eq!(a.input.as_deref(), Some(OsStr::new("in.zip")));
        assert_eq!(a.output.as_deref(), Some(OsStr::new("out.plain.pk")));
        assert!(a.progress_json && !a.plain && !a.info);
        assert_eq!(a.thumb_fit, ThumbFit::Bandes);
        let a = args(&["--allow-missing", "x.zip", "--plain", "--selector", "image"]).unwrap();
        assert!(a.allow_missing && a.plain);
        assert_eq!(a.selector, Selector::Image);
        assert_eq!(args(&["x.zip"]).unwrap().thumb_fit, ThumbFit::Flou);
    }

    #[test]
    fn arguments_invalides() {
        for bad in [
            &[][..],
            &["--progress-json"],
            &["x.zip", "y.zip"],
            &["x.zip", "--bogus"],
            &["x.zip", "-o"],
            &["x.zip", "-o", "--plain"],
            &["x.zip", "--selector", "nope"],
            &["x.zip", "--thumb-fit", "x"],
        ] {
            assert!(args(bad).is_err(), "{bad:?}");
        }
        // `--version` l'emporte, comme l'action `version` d'argparse.
        assert!(args(&["--version"]).unwrap().version);
    }
}
