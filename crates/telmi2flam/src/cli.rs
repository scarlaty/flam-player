//! Analyse des arguments, à la façon d'`argparse` de Python 3.14 (#133) : mêmes formes
//! acceptées (`--opt=valeur`, `-ovaleur`, `-o=valeur`, préfixes d'options, `--`, valeurs
//! qui ressemblent à un nombre négatif), mêmes messages et même ordre des erreurs que
//! `telmi2flam.py`. Les vecteurs de `tests/data/cli_cases.json` (générés par l'oracle Python,
//! `gen_cli_cases.py`) sont rejoués par `tests/cli_parity.rs`.
//!
//! Écarts assumés :
//! - le nom du programme est `telmi2flam` (et non `telmi2flam.py`) ;
//! - `--cancel-on-stdin-eof` (option du port, voir `cancel`) est acceptée mais absente de
//!   l'usage et de l'aide ; elle participe aux préfixes (`--c` la désigne) ;
//! - l'aide est mise en forme pour 80 colonnes (`COLUMNS` est ignoré) ;
//! - les valeurs non Unicode sont comparées sur leur forme avec remplacement ;
//! - `repr` des valeurs citées dans les messages : seuls les caractères de contrôle ASCII sont
//!   échappés (Python échappe aussi les non imprimables Unicode : `\xa0`, `\u200b`…).

use std::ffi::OsString;

use crate::image::ThumbFit;
use crate::package::Selector;

/// Usage (stderr en cas d'erreur, début de l'aide).
pub const USAGE: &str = "usage: telmi2flam [-h] [--version] [-o OUTPUT] [--plain] [--keep-size]
                  [--allow-missing] [--selector {carousel,image}]
                  [--thumb-fit {flou,rogner,bandes}] [--progress-json]
                  [--info]
                  input
";

/// Corps de l'aide de `-h`, après l'usage.
pub const HELP: &str = "
Convertit une histoire TELMI (.zip) en FLAM (.plain.pk)

positional arguments:
  input                 archive TELMI .zip

options:
  -h, --help            show this help message and exit
  --version             afficher la version du convertisseur et quitter
  -o, --output OUTPUT   sortie : fichier .plain.pk (defaut) ou dossier .plain
  --plain               ecrire AUSSI le dossier .plain extrait a cote du
                        .plain.pk (pour le simulateur)
  --keep-size           garder la resolution native des images (pas
                        d'ajustement 320x212, bornee a 2047 px)
  --allow-missing       convertir malgre des images/audios manquants (audio ->
                        silent.mp3)
  --selector {carousel,image}
                        affichage des choix multiples : carousel (defaut, 3
                        vignettes + label) ou image (image-choice plein ecran
                        d'origine)
  --thumb-fit {flou,rogner,bandes}
                        remplissage de la vignette 104x136 : flou (defaut :
                        image entiere centree, fond = image agrandie et
                        floutee), rogner (cover centre) ou bandes (image
                        entiere, bandes transparentes)
  --progress-json       evenements de progression en JSON (une ligne par
                        evenement) sur stdout ; les messages humains restent
                        sur stderr
  --info                afficher les metadonnees de l'histoire en JSON, sans
                        convertir
";

/// Arguments reconnus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub input: OsString,
    pub output: Option<OsString>,
    pub plain: bool,
    pub keep_size: bool,
    pub allow_missing: bool,
    pub selector: Selector,
    pub thumb_fit: ThumbFit,
    pub progress_json: bool,
    pub info: bool,
    pub cancel_on_stdin_eof: bool,
}

/// Résultat de l'analyse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parsed {
    Run(Args),
    /// `-h` : usage et aide sur stdout, code 0.
    Help,
    /// `--version` : `telmi2flam <version>` sur stdout, code 0.
    Version,
    /// Erreur d'usage : message d'argparse, code 2.
    Error(String),
}

impl Parsed {
    /// Sorties des cas qui n'exécutent rien : `(code, stdout, stderr)`.
    pub fn render(&self) -> Option<(u8, String, String)> {
        match self {
            Parsed::Run(_) => None,
            Parsed::Help => Some((0, format!("{USAGE}{HELP}"), String::new())),
            Parsed::Version => Some((0, format!("telmi2flam {}\n", crate::VERSION), String::new())),
            Parsed::Error(m) => Some((2, String::new(), usage_error(m))),
        }
    }
}

/// Texte de `parser.error(message)` : usage puis `telmi2flam: error: <message>`.
pub fn usage_error(message: &str) -> String {
    format!("{USAGE}telmi2flam: error: {message}\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Act {
    Help,
    Version,
    Output,
    Plain,
    KeepSize,
    AllowMissing,
    Selector,
    ThumbFit,
    ProgressJson,
    Info,
    CancelOnStdinEof,
}

/// Chaînes d'option dans l'ordre d'enregistrement d'argparse.
const OPTS: &[(&str, Act)] = &[
    ("-h", Act::Help),
    ("--help", Act::Help),
    ("--version", Act::Version),
    ("-o", Act::Output),
    ("--output", Act::Output),
    ("--plain", Act::Plain),
    ("--keep-size", Act::KeepSize),
    ("--allow-missing", Act::AllowMissing),
    ("--selector", Act::Selector),
    ("--thumb-fit", Act::ThumbFit),
    ("--progress-json", Act::ProgressJson),
    ("--info", Act::Info),
    ("--cancel-on-stdin-eof", Act::CancelOnStdinEof),
];

impl Act {
    fn takes_value(self) -> bool {
        matches!(self, Act::Output | Act::Selector | Act::ThumbFit)
    }

    /// Nom dans « argument X: … » : les chaînes d'option jointes par `/`.
    fn display(self) -> String {
        let names: Vec<&str> = OPTS
            .iter()
            .filter(|(_, a)| *a == self)
            .map(|(s, _)| *s)
            .collect();
        names.join("/")
    }
}

fn lookup(s: &str) -> Option<Act> {
    OPTS.iter().find(|(o, _)| *o == s).map(|(_, a)| *a)
}

/// Chiffres décimaux Unicode 16.0.0 (catégorie Nd, `\d` de `re`) : plages inclusives.
const DECIMAL_DIGITS: &[(u32, u32)] = &[
    (0x30, 0x39),
    (0x660, 0x669),
    (0x6F0, 0x6F9),
    (0x7C0, 0x7C9),
    (0x966, 0x96F),
    (0x9E6, 0x9EF),
    (0xA66, 0xA6F),
    (0xAE6, 0xAEF),
    (0xB66, 0xB6F),
    (0xBE6, 0xBEF),
    (0xC66, 0xC6F),
    (0xCE6, 0xCEF),
    (0xD66, 0xD6F),
    (0xDE6, 0xDEF),
    (0xE50, 0xE59),
    (0xED0, 0xED9),
    (0xF20, 0xF29),
    (0x1040, 0x1049),
    (0x1090, 0x1099),
    (0x17E0, 0x17E9),
    (0x1810, 0x1819),
    (0x1946, 0x194F),
    (0x19D0, 0x19D9),
    (0x1A80, 0x1A89),
    (0x1A90, 0x1A99),
    (0x1B50, 0x1B59),
    (0x1BB0, 0x1BB9),
    (0x1C40, 0x1C49),
    (0x1C50, 0x1C59),
    (0xA620, 0xA629),
    (0xA8D0, 0xA8D9),
    (0xA900, 0xA909),
    (0xA9D0, 0xA9D9),
    (0xA9F0, 0xA9F9),
    (0xAA50, 0xAA59),
    (0xABF0, 0xABF9),
    (0xFF10, 0xFF19),
    (0x104A0, 0x104A9),
    (0x10D30, 0x10D39),
    (0x10D40, 0x10D49),
    (0x11066, 0x1106F),
    (0x110F0, 0x110F9),
    (0x11136, 0x1113F),
    (0x111D0, 0x111D9),
    (0x112F0, 0x112F9),
    (0x11450, 0x11459),
    (0x114D0, 0x114D9),
    (0x11650, 0x11659),
    (0x116C0, 0x116C9),
    (0x116D0, 0x116E3),
    (0x11730, 0x11739),
    (0x118E0, 0x118E9),
    (0x11950, 0x11959),
    (0x11BF0, 0x11BF9),
    (0x11C50, 0x11C59),
    (0x11D50, 0x11D59),
    (0x11DA0, 0x11DA9),
    (0x11F50, 0x11F59),
    (0x16130, 0x16139),
    (0x16A60, 0x16A69),
    (0x16AC0, 0x16AC9),
    (0x16B50, 0x16B59),
    (0x16D70, 0x16D79),
    (0x1CCF0, 0x1CCF9),
    (0x1D7CE, 0x1D7FF),
    (0x1E140, 0x1E149),
    (0x1E2F0, 0x1E2F9),
    (0x1E4F0, 0x1E4F9),
    (0x1E5F1, 0x1E5FA),
    (0x1E950, 0x1E959),
    (0x1FBF0, 0x1FBF9),
];

fn is_decimal_digit(c: char) -> bool {
    let c = c as u32;
    DECIMAL_DIGITS
        .iter()
        .any(|&(lo, hi)| (lo..=hi).contains(&c))
}

/// `_negative_number_matcher` de Python 3.14 : `-`, éventuellement `.`, puis un chiffre
/// (le reste de l'argument est libre : `-1.pk` est une valeur).
fn looks_negative_number(s: &str) -> bool {
    let mut it = s.chars();
    if it.next() != Some('-') {
        return false;
    }
    match it.next() {
        Some('.') => it.next().is_some_and(is_decimal_digit),
        Some(c) => is_decimal_digit(c),
        None => false,
    }
}

/// `repr(str)` de Python pour les valeurs citées dans les messages d'erreur.
fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\x{:02x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// Option reconnue (ou non) à une position : `(action, chaîne d'option, séparateur, argument
/// explicite)`, comme les tuples d'argparse.
#[derive(Debug, Clone)]
struct Opt {
    act: Option<Act>,
    option: String,
    sep: Option<&'static str>,
    explicit: Option<String>,
    /// Abréviation ambiguë : message, levé quand l'option est consommée (pas au classement :
    /// les actions qui la précèdent, comme `--version`, s'exécutent d'abord).
    ambiguous: Option<String>,
}

#[derive(Debug, Clone)]
enum Class {
    /// Argument positionnel (`A`).
    Arg,
    /// Le premier `--` (`-`).
    DashDash,
    /// Option (`O`).
    Opt(Opt),
}

/// `_get_option_tuples` : options dont l'argument est un préfixe (abréviation).
fn option_tuples(arg: &str) -> Vec<Opt> {
    let mut chars = arg.chars();
    let (c0, c1) = (chars.next(), chars.next());
    let mut found = Vec::new();
    if c0 == Some('-') && c1 == Some('-') {
        let (prefix, sep, explicit) = match arg.split_once('=') {
            Some((p, e)) => (p, Some("="), Some(e.to_owned())),
            None => (arg, None, None),
        };
        for (o, a) in OPTS {
            if o.starts_with(prefix) {
                found.push(Opt {
                    act: Some(*a),
                    option: (*o).to_owned(),
                    sep,
                    explicit: explicit.clone(),
                    ambiguous: None,
                });
            }
        }
    } else if c0 == Some('-') && c1.is_some() {
        // Option courte collée à sa valeur (`-ovaleur`) ou préfixe d'une option.
        let short: String = arg.chars().take(2).collect();
        let tail: String = arg.chars().skip(2).collect();
        for (o, a) in OPTS {
            if *o == short {
                found.push(Opt {
                    act: Some(*a),
                    option: (*o).to_owned(),
                    sep: Some(""),
                    explicit: Some(tail.clone()),
                    ambiguous: None,
                });
            } else if o.starts_with(arg) {
                found.push(Opt {
                    act: Some(*a),
                    option: (*o).to_owned(),
                    sep: None,
                    explicit: None,
                    ambiguous: None,
                });
            }
        }
    }
    found
}

/// `_parse_optional` : `Ok(None)` pour un argument positionnel.
fn parse_optional(arg: &str) -> Option<Opt> {
    if !arg.starts_with('-') {
        return None;
    }
    if let Some(a) = lookup(arg) {
        return Some(Opt {
            act: Some(a),
            option: arg.to_owned(),
            sep: None,
            explicit: None,
            ambiguous: None,
        });
    }
    if arg.chars().count() == 1 {
        return None;
    }
    if let Some((o, e)) = arg.split_once('=') {
        if let Some(a) = lookup(o) {
            return Some(Opt {
                act: Some(a),
                option: o.to_owned(),
                sep: Some("="),
                explicit: Some(e.to_owned()),
                ambiguous: None,
            });
        }
    }
    let mut tuples = option_tuples(arg);
    if tuples.len() > 1 {
        let names: Vec<&str> = tuples.iter().map(|t| t.option.as_str()).collect();
        return Some(Opt {
            act: None,
            option: arg.to_owned(),
            sep: None,
            explicit: None,
            ambiguous: Some(format!(
                "ambiguous option: {arg} could match {}",
                names.join(", ")
            )),
        });
    }
    if let Some(t) = tuples.pop() {
        return Some(t);
    }
    if looks_negative_number(arg) || arg.contains(' ') {
        return None;
    }
    Some(Opt {
        act: None,
        option: arg.to_owned(),
        sep: None,
        explicit: None,
        ambiguous: None,
    })
}

/// Fin anticipée de l'analyse.
enum Stop {
    Help,
    Version,
    Error(String),
}

#[derive(Default)]
struct State {
    input: Option<OsString>,
    output: Option<OsString>,
    plain: bool,
    keep_size: bool,
    allow_missing: bool,
    selector: Selector,
    thumb_fit: ThumbFit,
    progress_json: bool,
    info: bool,
    cancel_on_stdin_eof: bool,
}

/// Longueur du motif `-*A-*` en tête de `pattern`, ou `None`.
fn match_one(pattern: &[u8]) -> Option<usize> {
    let lead = pattern.iter().take_while(|&&c| c == b'-').count();
    if pattern.get(lead) != Some(&b'A') {
        return None;
    }
    let trail = pattern[lead + 1..]
        .iter()
        .take_while(|&&c| c == b'-')
        .count();
    Some(lead + 1 + trail)
}

/// Valeur d'un argument à une valeur : le premier `--` retiré, puis le premier élément.
fn take_value(slice: &[OsString]) -> OsString {
    let mut items: Vec<&OsString> = slice.iter().collect();
    if let Some(p) = items.iter().position(|s| s.as_encoded_bytes() == b"--") {
        items.remove(p);
    }
    items.first().map(|s| (*s).clone()).unwrap_or_default()
}

impl State {
    fn take(&mut self, act: Act, value: Option<OsString>) -> Result<(), Stop> {
        let text = value
            .as_ref()
            .map(|v| v.to_string_lossy().into_owned())
            .unwrap_or_default();
        match act {
            Act::Help => return Err(Stop::Help),
            Act::Version => return Err(Stop::Version),
            Act::Output => self.output = value,
            Act::Plain => self.plain = true,
            Act::KeepSize => self.keep_size = true,
            Act::AllowMissing => self.allow_missing = true,
            Act::ProgressJson => self.progress_json = true,
            Act::Info => self.info = true,
            Act::CancelOnStdinEof => self.cancel_on_stdin_eof = true,
            Act::Selector => {
                self.selector = match text.as_str() {
                    "carousel" => Selector::Carousel,
                    "image" => Selector::Image,
                    _ => {
                        return Err(Stop::Error(format!(
                            "argument {}: invalid choice: {} (choose from 'carousel', 'image')",
                            act.display(),
                            py_repr(&text)
                        )))
                    }
                };
            }
            Act::ThumbFit => {
                self.thumb_fit = ThumbFit::from_name(&text).ok_or_else(|| {
                    Stop::Error(format!(
                        "argument {}: invalid choice: {} (choose from 'flou', 'rogner', 'bandes')",
                        act.display(),
                        py_repr(&text)
                    ))
                })?;
            }
        }
        Ok(())
    }
}

fn ignored(act: Act, explicit: &str) -> Stop {
    Stop::Error(format!(
        "argument {}: ignored explicit argument {}",
        act.display(),
        py_repr(explicit)
    ))
}

/// Analyse `argv` (sans le nom du programme), comme `parse_args` de `_Parser`.
pub fn parse(argv: &[OsString]) -> Parsed {
    match parse_inner(argv) {
        Ok(a) => Parsed::Run(a),
        Err(Stop::Help) => Parsed::Help,
        Err(Stop::Version) => Parsed::Version,
        Err(Stop::Error(m)) => Parsed::Error(m),
    }
}

fn parse_inner(argv: &[OsString]) -> Result<Args, Stop> {
    let strings: Vec<String> = argv
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    // Classement de tous les arguments d'abord.
    let mut classes: Vec<Class> = Vec::with_capacity(strings.len());
    let mut after_dashdash = false;
    for s in &strings {
        if after_dashdash {
            classes.push(Class::Arg);
        } else if s == "--" {
            after_dashdash = true;
            classes.push(Class::DashDash);
        } else {
            match parse_optional(s) {
                Some(o) => classes.push(Class::Opt(o)),
                None => classes.push(Class::Arg),
            }
        }
    }
    let pattern: Vec<u8> = classes
        .iter()
        .map(|c| match c {
            Class::Arg => b'A',
            Class::DashDash => b'-',
            Class::Opt(_) => b'O',
        })
        .collect();
    let last_option = classes.iter().rposition(|c| matches!(c, Class::Opt(_)));

    let mut st = State::default();
    let mut extras: Vec<usize> = Vec::new();
    let mut start = 0usize;

    let consume_positionals = |st: &mut State, start: usize| -> usize {
        if st.input.is_some() {
            return start;
        }
        match match_one(&pattern[start..]) {
            Some(n) => {
                st.input = Some(take_value(&argv[start..start + n]));
                start + n
            }
            None => start,
        }
    };

    if let Some(max) = last_option {
        while start <= max {
            let mut next = start;
            while next <= max && !matches!(classes[next], Class::Opt(_)) {
                next += 1;
            }
            if start != next {
                let end = consume_positionals(&mut st, start);
                if end > start {
                    start = end;
                    continue;
                }
            }
            if !matches!(classes[start], Class::Opt(_)) {
                extras.extend(start..next);
                start = next;
            }
            start = consume_optional(&mut st, &classes, &pattern, argv, start, &mut extras)?;
        }
    }
    let stop = consume_positionals(&mut st, start);
    extras.extend(stop..argv.len());

    let Some(input) = st.input.take() else {
        return Err(Stop::Error(
            "the following arguments are required: input".to_owned(),
        ));
    };
    if !extras.is_empty() {
        let list: Vec<&str> = extras.iter().map(|&i| strings[i].as_str()).collect();
        return Err(Stop::Error(format!(
            "unrecognized arguments: {}",
            list.join(" ")
        )));
    }
    Ok(Args {
        input,
        output: st.output,
        plain: st.plain,
        keep_size: st.keep_size,
        allow_missing: st.allow_missing,
        selector: st.selector,
        thumb_fit: st.thumb_fit,
        progress_json: st.progress_json,
        info: st.info,
        cancel_on_stdin_eof: st.cancel_on_stdin_eof,
    })
}

/// `consume_optional` : applique l'option à la position `start`, rend l'index suivant.
fn consume_optional(
    st: &mut State,
    classes: &[Class],
    pattern: &[u8],
    argv: &[OsString],
    start: usize,
    extras: &mut Vec<usize>,
) -> Result<usize, Stop> {
    let Class::Opt(opt) = &classes[start] else {
        return Ok(start + 1);
    };
    if let Some(message) = &opt.ambiguous {
        return Err(Stop::Error(message.clone()));
    }
    let mut act = opt.act;
    let mut option = opt.option.clone();
    let mut sep = opt.sep;
    let mut explicit = opt.explicit.clone();
    // Actions différées (grappe d'options courtes `-hx`) : exécutées après l'analyse de la
    // grappe, comme argparse.
    let mut queue: Vec<(Act, Option<OsString>)> = Vec::new();
    let stop;
    loop {
        let Some(a) = act else {
            extras.push(start);
            return Ok(start + 1);
        };
        if let Some(e) = explicit.clone() {
            let count = usize::from(a.takes_value());
            let single_dash = !option.starts_with("--");
            if count == 0 && single_dash && !e.is_empty() {
                if sep.is_some_and(|s| !s.is_empty()) || e.starts_with('-') {
                    return Err(ignored(a, &e));
                }
                queue.push((a, None));
                let mut it = e.chars();
                let c = it.next().unwrap_or('-');
                option = format!("-{c}");
                match lookup(&option) {
                    Some(next) => {
                        act = Some(next);
                        let rest: String = it.collect();
                        if rest.is_empty() {
                            sep = None;
                            explicit = None;
                        } else if let Some(r) = rest.strip_prefix('=') {
                            sep = Some("=");
                            explicit = Some(r.to_owned());
                        } else {
                            sep = Some("");
                            explicit = Some(rest);
                        }
                    }
                    // Caractère inconnu après `-h` : l'aide est déjà en attente (argparse
                    // l'exécute avant de signaler l'argument ignoré).
                    None => return run_queue(st, queue).map(|()| start + 1),
                }
            } else if count == 1 {
                stop = start + 1;
                queue.push((a, Some(OsString::from(e))));
                break;
            } else {
                return Err(ignored(a, &e));
            }
        } else {
            let from = start + 1;
            let n = if a.takes_value() {
                // Une option ne prend pas de `--` autour de sa valeur (motif `A`).
                match pattern.get(from) {
                    Some(b'A') => 1,
                    _ => {
                        return Err(Stop::Error(format!(
                            "argument {}: expected one argument",
                            a.display()
                        )))
                    }
                }
            } else {
                0
            };
            stop = from + n;
            let value = a.takes_value().then(|| argv[from].clone());
            queue.push((a, value));
            break;
        }
    }
    run_queue(st, queue)?;
    Ok(stop)
}

fn run_queue(st: &mut State, queue: Vec<(Act, Option<OsString>)>) -> Result<(), Stop> {
    for (act, value) in queue {
        st.take(act, value)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(list: &[&str]) -> Parsed {
        parse(&list.iter().map(OsString::from).collect::<Vec<_>>())
    }

    fn run(list: &[&str]) -> Args {
        match p(list) {
            Parsed::Run(a) => a,
            other => panic!("{list:?} : {other:?}"),
        }
    }

    fn err(list: &[&str]) -> String {
        match p(list) {
            Parsed::Error(m) => m,
            other => panic!("{list:?} : {other:?}"),
        }
    }

    #[test]
    fn options_du_launcher() {
        let a = run(&[
            "in.zip",
            "-o",
            "out.plain.pk",
            "--progress-json",
            "--thumb-fit",
            "bandes",
        ]);
        assert_eq!(a.input, "in.zip");
        assert_eq!(
            a.output.as_deref(),
            Some(std::ffi::OsStr::new("out.plain.pk"))
        );
        assert!(a.progress_json && !a.plain && !a.info);
        assert_eq!(a.thumb_fit, ThumbFit::Bandes);
        let a = run(&["--allow-missing", "x.zip", "--plain", "--selector", "image"]);
        assert!(a.allow_missing && a.plain);
        assert_eq!(a.selector, Selector::Image);
        assert_eq!(run(&["x.zip"]).thumb_fit, ThumbFit::Flou);
        assert!(!run(&["x.zip"]).cancel_on_stdin_eof);
        assert!(run(&["x.zip", "--cancel-on-stdin-eof"]).cancel_on_stdin_eof);
    }

    #[test]
    fn formes_d_argparse() {
        for (list, output) in [
            (&["x", "-o", "-1.pk"][..], "-1.pk"),
            (&["x", "--output=a"], "a"),
            (&["x", "--out", "a"], "a"),
            (&["x", "-oa"], "a"),
            (&["x", "-o=a"], "a"),
            (&["x", "-o", "-x y"], "-x y"),
            (&["x", "-o", "-"], "-"),
            (&["-o", "a", "--", "x"], "a"),
        ] {
            let a = run(list);
            assert_eq!(a.output.unwrap().to_string_lossy(), output, "{list:?}");
        }
        assert_eq!(run(&["--", "x"]).input, "x");
    }

    #[test]
    fn erreurs_d_usage() {
        assert_eq!(err(&[]), "the following arguments are required: input");
        assert_eq!(
            err(&["--progress-json"]),
            "the following arguments are required: input"
        );
        assert_eq!(err(&["x", "y"]), "unrecognized arguments: y");
        assert_eq!(err(&["x", "--bogus"]), "unrecognized arguments: --bogus");
        assert_eq!(
            err(&["x", "-o"]),
            "argument -o/--output: expected one argument"
        );
        assert_eq!(
            err(&["x", "-o", "--plain"]),
            "argument -o/--output: expected one argument"
        );
        assert_eq!(
            err(&["x", "--p"]),
            "ambiguous option: --p could match --plain, --progress-json"
        );
        assert_eq!(
            err(&["x", "--selector", "nope"]),
            "argument --selector: invalid choice: 'nope' (choose from 'carousel', 'image')"
        );
        assert_eq!(
            err(&["x", "--plain=1"]),
            "argument --plain: ignored explicit argument '1'"
        );
        assert_eq!(p(&["--version", "x"]), Parsed::Version);
        assert_eq!(p(&["x", "-h"]), Parsed::Help);
    }

    #[test]
    fn chiffres_decimaux() {
        assert!(looks_negative_number("-1.pk"));
        assert!(looks_negative_number("-.5x"));
        assert!(looks_negative_number("-\u{661}"));
        assert!(!looks_negative_number("-\u{b2}"));
        assert!(!looks_negative_number("-.x"));
        assert!(!looks_negative_number("-a"));
    }
}
