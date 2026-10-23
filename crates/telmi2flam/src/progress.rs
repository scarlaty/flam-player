//! Événements de progression (`--progress-json`) : `start`, `step`, `warning`, `done` et
//! `error`, identiques à ceux de `tools/telmi2flam/telmi2flam.py` (`_emit`, `_Progress`,
//! `_print_event`, README « Sortie machine »).
//!
//! Format : un objet JSON par ligne, compact (`separators=(",", ":")`), ASCII seulement
//! (`ensure_ascii=True` : `\uXXXX` minuscule par unité UTF-16), clés dans l'ordre de Python.
//! Les chemins (`input`, `output`) sont encodés comme Python les voit : unités UTF-16 du chemin
//! sous Windows (surrogates isolés compris), octets non UTF-8 en `\udcXX` ailleurs
//! (`surrogateescape`).
//!
//! Étapes : chaque phase émet `done:0` au début et `done:total` à la fin (même si `total` vaut
//! 0) ; les événements intermédiaires sont espacés d'au moins [`PROGRESS_INTERVAL`].
//!
//! La bibliothèque n'écrit rien : les événements vont au rappel du [`Reporter`]. Le binaire
//! écrit la ligne `  ! …` de chaque avertissement sur stderr (contrat de `compat.py`) et, avec
//! `--progress-json`, l'événement sur stdout.

use std::ffi::OsStr;
use std::time::{Duration, Instant};

/// Espacement minimal de deux événements `step` intermédiaires d'une même phase.
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Phases annoncées par `start`, dans l'ordre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Images,
    Audio,
    Scripts,
    Zip,
}

impl Phase {
    pub const ALL: [Phase; 4] = [Phase::Images, Phase::Audio, Phase::Scripts, Phase::Zip];

    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Images => "images",
            Phase::Audio => "audio",
            Phase::Scripts => "scripts",
            Phase::Zip => "zip",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Événement de progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event<'a> {
    Start {
        input: &'a OsStr,
        version: &'a str,
    },
    Step {
        phase: Phase,
        done: u64,
        total: u64,
    },
    Warning {
        code: &'a str,
        message: &'a str,
    },
    /// Information humaine (`  i …` de stderr) : jamais émise en JSON.
    Info {
        message: &'a str,
    },
    Done {
        output: &'a OsStr,
        size: u64,
        warnings: u64,
    },
    Error {
        code: &'a str,
        message: &'a str,
        exit: u8,
    },
}

impl Event<'_> {
    /// Ligne JSON de l'événement (sans fin de ligne), octet pour octet celle de Python.
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"event\":");
        match *self {
            Event::Start { input, version } => {
                out.push_str("\"start\",\"total\":");
                out.push_str(&Phase::ALL.len().to_string());
                out.push_str(",\"phases\":[");
                for (i, p) in Phase::ALL.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    json_str(p.as_str(), &mut out);
                }
                out.push_str("],\"input\":");
                json_os_str(input, &mut out);
                out.push_str(",\"version\":");
                json_str(version, &mut out);
            }
            Event::Step { phase, done, total } => {
                out.push_str("\"step\",\"phase\":");
                json_str(phase.as_str(), &mut out);
                out.push_str(&format!(",\"done\":{done},\"total\":{total}"));
            }
            Event::Warning { code, message } => {
                out.push_str("\"warning\",\"code\":");
                json_str(code, &mut out);
                out.push_str(",\"message\":");
                json_str(message, &mut out);
            }
            Event::Info { message } => {
                out.push_str("\"info\",\"message\":");
                json_str(message, &mut out);
            }
            Event::Done {
                output,
                size,
                warnings,
            } => {
                out.push_str("\"done\",\"output\":");
                json_os_str(output, &mut out);
                out.push_str(&format!(",\"size\":{size},\"warnings\":{warnings}"));
            }
            Event::Error {
                code,
                message,
                exit,
            } => {
                out.push_str("\"error\",\"code\":");
                json_str(code, &mut out);
                out.push_str(",\"message\":");
                json_str(message, &mut out);
                out.push_str(&format!(",\"exit\":{exit}"));
            }
        }
        out.push('}');
        out
    }
}

/// Chaîne JSON avec `ensure_ascii=True` à partir d'unités UTF-16 : tout ce qui n'est pas
/// entre `' '` et `'~'` est échappé en `\uXXXX` minuscule (un surrogate isolé aussi, comme
/// Python).
pub(crate) fn json_units(units: impl IntoIterator<Item = u16>, out: &mut String) {
    out.push('"');
    for unit in units {
        match unit {
            0x22 => out.push_str("\\\""),
            0x5C => out.push_str("\\\\"),
            0x0A => out.push_str("\\n"),
            0x0D => out.push_str("\\r"),
            0x09 => out.push_str("\\t"),
            0x08 => out.push_str("\\b"),
            0x0C => out.push_str("\\f"),
            0x20..=0x7E => out.push(char::from(unit as u8)),
            _ => out.push_str(&format!("\\u{unit:04x}")),
        }
    }
    out.push('"');
}

/// Chaîne JSON (`ensure_ascii=True`) d'un texte.
pub(crate) fn json_str(s: &str, out: &mut String) {
    json_units(s.encode_utf16(), out);
}

/// Chaîne JSON (`ensure_ascii=True`) d'un chemin, tel que Python le reçoit dans `sys.argv`.
pub(crate) fn json_os_str(s: &OsStr, out: &mut String) {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        json_units(s.encode_wide(), out);
    }
    #[cfg(not(windows))]
    {
        // `surrogateescape` : chaque octet non UTF-8 devient U+DC80..U+DCFF.
        let mut units = Vec::new();
        for chunk in s.as_encoded_bytes().utf8_chunks() {
            units.extend(chunk.valid().encode_utf16());
            units.extend(chunk.invalid().iter().map(|&b| 0xDC00 | u16::from(b)));
        }
        json_units(units, out);
    }
}

/// Récepteur de la progression d'une conversion : phases et avertissements.
pub trait Progress {
    /// Début d'une phase : événement `done:0`.
    fn begin(&mut self, phase: Phase, total: u64);
    /// Un élément de la phase est traité.
    fn tick(&mut self, phase: Phase);
    /// Avertissement non bloquant (`code` stable, `message` lisible).
    fn warn(&mut self, code: &'static str, message: &str);
    /// Information non bloquante (rééchantillonnage réussi) : ligne humaine seulement.
    fn info(&mut self, _message: &str) {}
}

/// Progression ignorée (appel de la bibliothèque sans suivi).
impl Progress for () {
    fn begin(&mut self, _phase: Phase, _total: u64) {}
    fn tick(&mut self, _phase: Phase) {}
    fn warn(&mut self, _code: &'static str, _message: &str) {}
}

#[derive(Debug, Clone, Copy)]
struct PhaseState {
    done: u64,
    total: u64,
    last: Instant,
}

/// Rappel qui reçoit chaque événement.
pub type Sink<'s> = &'s mut dyn FnMut(&Event<'_>);

/// Émetteur des événements (`_Progress` et `_warn` de Python) : espacement des `step`,
/// comptage des avertissements pour `done`.
pub struct Reporter<'s> {
    sink: Sink<'s>,
    interval: Duration,
    phases: [Option<PhaseState>; 4],
    warnings: u64,
}

impl<'s> Reporter<'s> {
    pub fn new(sink: Sink<'s>) -> Self {
        Self::with_interval(sink, PROGRESS_INTERVAL)
    }

    /// `interval` : espacement des `step` intermédiaires (0 : tous, comme les tests Python
    /// avec `PROGRESS_INTERVAL = 0`).
    pub fn with_interval(sink: Sink<'s>, interval: Duration) -> Self {
        Reporter {
            sink,
            interval,
            phases: [None; 4],
            warnings: 0,
        }
    }

    /// Premier événement, avant toute lecture de l'archive.
    pub fn start(&mut self, input: &OsStr) {
        (self.sink)(&Event::Start {
            input,
            version: crate::VERSION,
        });
    }

    /// Dernier événement d'une conversion réussie.
    pub fn done(&mut self, output: &OsStr, size: u64) {
        let warnings = self.warnings;
        (self.sink)(&Event::Done {
            output,
            size,
            warnings,
        });
    }

    /// Avertissements émis depuis la création.
    pub fn warnings(&self) -> u64 {
        self.warnings
    }

    fn emit_step(&mut self, phase: Phase) {
        if let Some(st) = &mut self.phases[phase.index()] {
            st.last = Instant::now();
            let (done, total) = (st.done, st.total);
            (self.sink)(&Event::Step { phase, done, total });
        }
    }
}

impl Progress for Reporter<'_> {
    fn begin(&mut self, phase: Phase, total: u64) {
        self.phases[phase.index()] = Some(PhaseState {
            done: 0,
            total,
            last: Instant::now(),
        });
        self.emit_step(phase);
    }

    fn tick(&mut self, phase: Phase) {
        // Phase non commencée : Python lèverait `KeyError` ; état impossible, ignoré.
        let Some(st) = &mut self.phases[phase.index()] else {
            return;
        };
        st.done += 1;
        if st.done >= st.total || st.last.elapsed() >= self.interval {
            self.emit_step(phase);
        }
    }

    fn warn(&mut self, code: &'static str, message: &str) {
        self.warnings += 1;
        (self.sink)(&Event::Warning { code, message });
    }

    fn info(&mut self, message: &str) {
        (self.sink)(&Event::Info { message });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(interval: Duration, f: impl FnOnce(&mut Reporter<'_>)) -> Vec<String> {
        let mut lines = Vec::new();
        let mut sink = |e: &Event<'_>| lines.push(e.to_json());
        let mut r = Reporter::with_interval(&mut sink, interval);
        f(&mut r);
        lines
    }

    /// Lignes de `json.dumps(event, ensure_ascii=True, separators=(",", ":"))`.
    #[test]
    fn evenements_comme_python() {
        let lines = collect(Duration::ZERO, |r| {
            r.start(OsStr::new("C:\\h\\é \"x\".zip"));
            r.begin(Phase::Images, 1);
            r.warn("image_missing", "image manquante: s\u{1F600}.png\n");
            r.tick(Phase::Images);
            r.done(OsStr::new("Titre.AB12CD34.plain.pk"), 1234);
        });
        let version = crate::VERSION;
        assert_eq!(
            lines,
            [
                format!(
                    "{{\"event\":\"start\",\"total\":4,\"phases\":[\"images\",\"audio\",\
                     \"scripts\",\"zip\"],\"input\":\"C:\\\\h\\\\\\u00e9 \\\"x\\\".zip\",\
                     \"version\":\"{version}\"}}"
                ),
                "{\"event\":\"step\",\"phase\":\"images\",\"done\":0,\"total\":1}".to_owned(),
                "{\"event\":\"warning\",\"code\":\"image_missing\",\"message\":\
                 \"image manquante: s\\ud83d\\ude00.png\\n\"}"
                    .to_owned(),
                "{\"event\":\"step\",\"phase\":\"images\",\"done\":1,\"total\":1}".to_owned(),
                "{\"event\":\"done\",\"output\":\"Titre.AB12CD34.plain.pk\",\"size\":1234,\
                 \"warnings\":1}"
                    .to_owned(),
            ]
        );
        let err = Event::Error {
            code: "cancelled",
            message: "conversion annulee\u{7f}\t",
            exit: 130,
        };
        assert_eq!(
            err.to_json(),
            "{\"event\":\"error\",\"code\":\"cancelled\",\"message\":\
             \"conversion annulee\\u007f\\t\",\"exit\":130}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn chemin_avec_surrogate_isole() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        let path = OsString::from_wide(&[0x61, 0xD800, 0x2E]);
        let mut out = String::new();
        json_os_str(&path, &mut out);
        assert_eq!(out, "\"a\\ud800.\"");
    }

    #[cfg(not(windows))]
    #[test]
    fn chemin_non_utf8_en_surrogateescape() {
        use std::os::unix::ffi::OsStrExt;
        let mut out = String::new();
        json_os_str(OsStr::from_bytes(b"a\xff\xc3\xa9"), &mut out);
        assert_eq!(out, "\"a\\udcff\\u00e9\"");
    }

    /// Phase vide : un seul `done:0,total:0` ; sans intermédiaires, le premier et le dernier.
    #[test]
    fn espacement_garde_le_premier_et_le_dernier() {
        let lines = collect(Duration::from_secs(3600), |r| {
            r.begin(Phase::Audio, 0);
            r.begin(Phase::Zip, 5);
            for _ in 0..5 {
                r.tick(Phase::Zip);
            }
        });
        assert_eq!(
            lines,
            [
                "{\"event\":\"step\",\"phase\":\"audio\",\"done\":0,\"total\":0}",
                "{\"event\":\"step\",\"phase\":\"zip\",\"done\":0,\"total\":5}",
                "{\"event\":\"step\",\"phase\":\"zip\",\"done\":5,\"total\":5}",
            ]
        );
        // Intervalle nul : un événement par élément, `done` croissant.
        let lines = collect(Duration::ZERO, |r| {
            r.begin(Phase::Scripts, 3);
            for _ in 0..3 {
                r.tick(Phase::Scripts);
            }
        });
        assert_eq!(lines.len(), 4);
        assert!(lines[2].contains("\"done\":2,"));
    }

    #[test]
    fn intermediaires_espaces_de_l_intervalle() {
        let lines = collect(Duration::from_millis(30), |r| {
            r.begin(Phase::Images, 1000);
            for _ in 0..999 {
                r.tick(Phase::Images);
            }
            std::thread::sleep(Duration::from_millis(40));
            r.tick(Phase::Images);
        });
        // Début, fin, et au plus quelques intermédiaires (machine lente).
        assert!(lines.len() >= 2 && lines.len() < 50, "{}", lines.len());
        assert!(lines.last().unwrap().contains("\"done\":1000,"));
    }

    #[test]
    fn tick_d_une_phase_non_commencee_ignore() {
        let lines = collect(Duration::ZERO, |r| r.tick(Phase::Zip));
        assert!(lines.is_empty());
    }
}
