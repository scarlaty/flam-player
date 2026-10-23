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
//! Annulation et panique (#132) :
//! - `--cancel-on-stdin-eof` (option propre au port, passée par le launcher) : la fin de stdin
//!   annule ; sans l'option, stdin n'est jamais lu. `Ctrl+C`, `CTRL_BREAK` (Windows),
//!   `SIGINT`, `SIGTERM` (ailleurs) annulent aussi ([`telmi2flam::cancel`]). Annulation :
//!   `.part` supprimés, événement `error` `cancelled`, code 130 ;
//! - crochet de panique ([`install_panic_hook`]) : temporaires du registre supprimés
//!   ([`telmi2flam::output::discard_pending`]), événement `error` `internal_error`, code 6.
//!   Il s'exécute avant l'abandon du profil release (`panic = "abort"`) et sort lui-même.
//!
//! Options (#133) : analyse façon `argparse` dans [`telmi2flam::cli`] (formes `--opt=valeur`,
//! préfixes, `--`, `-h`, messages et codes de Python). Résumé humain `OK -> …` comme
//! `telmi2flam.py` : sur stdout, ou sur stderr avec `--progress-json` ; les lignes `  i …`
//! (rééchantillonnage réussi) et `  ! …` (avertissements) vont toujours sur stderr, au fil de
//! la conversion.
//!
//! Fin de ligne : `\n` sur tous les systèmes. Python, en mode texte, écrit `\r\n` sous
//! Windows ; l'écart est assumé (les consommateurs lisent ligne à ligne et la parité
//! compare après `strip()`).

use std::ffi::OsString;
use std::io::Write;
use std::path::Path;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, Ordering};

use telmi2flam::cancel;
use telmi2flam::cli::{self, Parsed};
use telmi2flam::convert::{convert, Report, Request};
use telmi2flam::package::Options;
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

/// `--progress-json` actif : lu par le crochet de panique.
static PROGRESS_JSON: AtomicBool = AtomicBool::new(false);

/// Code de sortie d'une panique (`internal_error`, comme une exception non prévue en Python).
const EXIT_PANIC: u8 = 6;

/// Crochet de panique : supprime les temporaires en cours, émet `internal_error` (avec
/// `--progress-json`) et le message humain, puis sort avec le code 6. Aucun `Drop` ne
/// s'exécute ensuite : ni en `panic = "abort"` (release), ni ici, puisque `exit` précède le
/// déroulement.
fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        // ffmpeg d'abord : ses fichiers ouverts empêcheraient de supprimer `t2f_ff_*`.
        telmi2flam::ffmpeg::kill_children();
        telmi2flam::output::discard_pending();
        let payload = info.payload();
        let what = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
            .unwrap_or("panique");
        let place = info
            .location()
            .map(|l| format!(" ({}:{})", l.file(), l.line()))
            .unwrap_or_default();
        let message = format!("panic: {what}{place}");
        if PROGRESS_JSON.load(Ordering::SeqCst) {
            let line = Event::Error {
                code: "internal_error",
                message: &message,
                exit: EXIT_PANIC,
            }
            .to_json();
            let mut out = std::io::stdout().lock();
            let _ = writeln!(out, "{line}").and_then(|()| out.flush());
        }
        err_line(&format!("ERREUR inattendue (panic) : {what}{place}"));
        std::process::exit(i32::from(EXIT_PANIC));
    }));
}

fn main() -> ExitCode {
    install_panic_hook();
    let worker = std::thread::Builder::new()
        .name("telmi2flam".to_owned())
        .stack_size(WORKER_STACK)
        .spawn(run);
    match worker {
        // Le crochet sort avant ; reste le cas d'un crochet remplacé : même sortie que sans fil.
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
        if !self.enabled || self.dead || matches!(event, Event::Info { .. }) {
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

fn run() -> ExitCode {
    // args_os : un chemin non Unicode ne doit pas faire paniquer le binaire.
    let argv: Vec<OsString> = std::env::args_os().skip(1).collect();
    let args = match cli::parse(&argv) {
        Parsed::Run(args) => args,
        // `-h` et `--version` : texte de Python sur stdout, tube fermé ignoré.
        parsed @ (Parsed::Help | Parsed::Version) => {
            if let Some((code, stdout, _)) = parsed.render() {
                let mut out = std::io::stdout().lock();
                let _ = out.write_all(stdout.as_bytes()).and_then(|()| out.flush());
                return ExitCode::from(code);
            }
            return ExitCode::SUCCESS;
        }
        Parsed::Error(message) => {
            // Comme `_Parser.error` : l'événement d'abord, si `--progress-json` est présent.
            let mut events = Events::new(argv.iter().any(|a| a == "--progress-json"));
            let usage = cli::usage_error(&message);
            return events.fail(
                "bad_arguments",
                &message,
                2,
                Some(usage.trim_end_matches('\n')),
            );
        }
    };
    let mut events = Events::new(args.progress_json);
    PROGRESS_JSON.store(args.progress_json, Ordering::SeqCst);
    // Après l'analyse des arguments, comme `_install_cancel_handlers` de Python. Un échec
    // d'installation laisse le comportement par défaut (arrêt sans nettoyage).
    cancel::install_signal_handlers();
    if args.cancel_on_stdin_eof {
        if let Err(e) = cancel::cancel_on_stdin_eof() {
            // Sans ce fil, le launcher ne pourrait plus annuler proprement.
            return events.fail(
                "internal_error",
                &format!("OSError: lecture de stdin impossible : {e}"),
                6,
                None,
            );
        }
    }
    let input = Path::new(&args.input);
    if args.info {
        return match telmi2flam::info::read_info(input) {
            // `read_info` ne consulte pas le jeton : un Ctrl+C reçu pendant la lecture annule
            // ici, comme le `KeyboardInterrupt` de Python.
            Ok(_) | Err(_) if cancel::is_cancelled() => {
                let e = telmi2flam::error::ConvError::cancelled();
                events.fail(
                    e.code,
                    &e.message,
                    e.exit_code,
                    Some("Annule : aucune sortie partielle conservee."),
                )
            }
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
    let selector = args.selector;
    #[cfg(any(debug_assertions, feature = "test-hooks"))]
    let mut fault = fault::Fault::from_env();
    #[cfg(any(debug_assertions, feature = "test-hooks"))]
    let fault_set = fault.is_some();
    let result = {
        let mut sink = |e: &Event<'_>| {
            // `_warn` : la ligne de stderr d'abord, puis l'événement.
            match e {
                Event::Warning { message, .. } => err_line(&format!("  ! {message}")),
                Event::Info { message } => err_line(&format!("  i {message}")),
                _ => {}
            }
            events.emit(e);
            #[cfg(any(debug_assertions, feature = "test-hooks"))]
            if let (Some(f), Event::Step { phase, done, total }) = (fault.as_mut(), e) {
                f.on_step(phase.as_str(), *done, *total);
            }
        };
        #[cfg(any(debug_assertions, feature = "test-hooks"))]
        let mut reporter = if fault_set {
            Reporter::with_interval(&mut sink, std::time::Duration::ZERO)
        } else {
            Reporter::new(&mut sink)
        };
        #[cfg(not(any(debug_assertions, feature = "test-hooks")))]
        let mut reporter = Reporter::new(&mut sink);
        convert(&req, &mut reporter, &cancel::is_cancelled)
    };
    match result {
        Ok(report) => {
            summary(&report, selector.as_str(), args.progress_json);
            ExitCode::SUCCESS
        }
        Err(e) if e.code == "cancelled" => events.fail(
            e.code,
            &e.message,
            e.exit_code,
            Some("Annule : aucune sortie partielle conservee."),
        ),
        Err(e) => events.fail(e.code, &e.message, e.exit_code, None),
    }
}

/// Résumé humain de `_convert` : sur stdout, ou sur stderr avec `--progress-json` (stdout est
/// alors réservé aux événements). Tube fermé ignoré.
fn summary(report: &Report, selector: &str, progress_json: bool) {
    let say = |line: String| {
        if progress_json {
            err_line(&line);
        } else {
            out_line(&line);
        }
    };
    let sep = std::path::MAIN_SEPARATOR;
    for (i, p) in report.published.iter().enumerate() {
        // Après le `.plain.pk` éventuel, le dossier `.plain` : terminé par le séparateur.
        let dir = report.dir_only || i > 0;
        say(format!(
            "OK -> {}{}",
            p.display(),
            if dir { sep.to_string() } else { String::new() }
        ));
    }
    let st = &report.stats;
    say(format!("  titre   : {}", report.title));
    say(format!(
        "  stages  : {} | actions : {}",
        st.stages, st.actions
    ));
    say(format!(
        "  images  : {} | audio : {} | mp3map : {}",
        st.img, st.audio, st.mp3map
    ));
    if st.resampled > 0 {
        say(format!(
            "  audio   : {} fichier(s) reechantillonne(s) a 44100 Hz",
            st.resampled
        ));
    }
    say(format!(
        "  lib     : {} scripts framework | {} assets UI",
        st.lib, st.ui
    ));
    say(format!("  choix   : selecteur '{selector}'"));
    if st.missing > 0 || st.bad > 0 {
        say(format!(
            "  alertes : {} manquant(s) | {} illisible(s)",
            st.missing, st.bad
        ));
    }
    if !report.dir_only {
        say(format!("  taille  : {:.1} Mo", report.size as f64 / 1e6));
    }
}

/// Injection de fautes pour les tests d'intégration (`tests/cancel_panic.rs`), absente de la
/// release du launcher (ni `debug_assertions`, ni la feature `test-hooks`).
///
/// `TELMI2FLAM_TEST_FAULT=<action>:<phase>:<begin|end|N>` agit sur le premier événement `step`
/// de la phase (`begin` : `done == 0` ; `end` : `done == total` ; `N` : `done == N`), après
/// son émission. Avec une faute, tous les `step` sont émis (pas d'espacement de 0,1 s) :
/// - `cancel` : lève le jeton d'annulation ;
/// - `panic` : panique dans le fil de travail ;
/// - `wait` : attend l'annulation (60 s au plus), pour annuler de l'extérieur (stdin,
///   `CTRL_BREAK`, signal) à un point connu, par exemple avec le `.part` présent (`zip:end`).
#[cfg(any(debug_assertions, feature = "test-hooks"))]
mod fault {
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Action {
        Cancel,
        Panic,
        Wait,
    }

    #[derive(Debug)]
    pub struct Fault {
        action: Action,
        phase: String,
        point: Point,
        fired: bool,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Point {
        Begin,
        End,
        Done(u64),
    }

    impl Fault {
        pub fn from_env() -> Option<Fault> {
            let spec = std::env::var("TELMI2FLAM_TEST_FAULT").ok()?;
            let mut it = spec.splitn(3, ':');
            let action = match it.next()? {
                "cancel" => Action::Cancel,
                "panic" => Action::Panic,
                "wait" => Action::Wait,
                _ => return None,
            };
            let phase = it.next()?.to_owned();
            let point = match it.next()? {
                "begin" => Point::Begin,
                "end" => Point::End,
                n => Point::Done(n.parse().ok()?),
            };
            Some(Fault {
                action,
                phase,
                point,
                fired: false,
            })
        }

        pub fn on_step(&mut self, phase: &str, done: u64, total: u64) {
            let point = match self.point {
                Point::Begin => 0,
                Point::End => total,
                Point::Done(n) => n,
            };
            if self.fired || phase != self.phase || done != point {
                return;
            }
            self.fired = true;
            match self.action {
                Action::Cancel => telmi2flam::cancel::request(),
                Action::Panic => panic!("faute injectee ({phase} {done}/{total})"),
                Action::Wait => {
                    let start = Instant::now();
                    while !telmi2flam::cancel::is_cancelled()
                        && start.elapsed() < Duration::from_secs(60)
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
        }
    }
}
