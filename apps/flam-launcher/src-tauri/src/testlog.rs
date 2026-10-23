//! Journal des tests, **global** et partagé (#412, #430).
//!
//! Un abonné local au thread (`with_default`) est piégé par le cache d'intérêt des callsites
//! de `tracing` : si un test parallèle sans abonné atteint d'abord un callsite, celui-ci est
//! marqué « jamais » pour tout le processus et le journal capturé reste vide, d'où un test qui
//! passe à vide ou échoue selon l'ordonnancement. Un abonné global, posé une seule fois, évite
//! ce piège ; `try_init` pose aussi `LogTracer`, si bien que les traces `log` (reqwest, hyper)
//! et celles des tâches `blocking` y passent aussi.
//!
//! Le journal est partagé entre tous les tests du processus : on n'y vérifie que l'**absence**
//! d'un secret, jamais la présence d'une trace précise propre à un test.

use std::sync::{Arc, Mutex, OnceLock};

/// Journal en mémoire.
#[derive(Clone, Default)]
pub(crate) struct Logs(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Logs {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logs {
    type Writer = Logs;
    fn make_writer(&'a self) -> Logs {
        self.clone()
    }
}

impl Logs {
    /// Journal **global** (une seule fois par processus). Le filtre est celui de production
    /// (`crate::log_filter`) réglé sur `trace` : `reqwest` reste à `info`.
    pub(crate) fn capture() -> Logs {
        static LOGS: OnceLock<Logs> = OnceLock::new();
        LOGS.get_or_init(|| {
            let logs = Logs::default();
            tracing_subscriber::fmt()
                .with_writer(logs.clone())
                .with_ansi(false)
                .with_env_filter(crate::log_filter(Some("trace")))
                .try_init()
                .expect("abonné global déjà posé : le journal serait vide");
            logs
        })
        .clone()
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Témoin : le journal global capture bien une trace connue. Sans lui, les assertions
    /// d'absence de secret (`assert_no_leak`) passeraient à vide si le journal restait vide.
    #[test]
    fn le_journal_global_capture_une_trace_connue() {
        let logs = Logs::capture();
        tracing::info!("temoin-testlog-7f3a");
        assert!(
            logs.text().contains("temoin-testlog-7f3a"),
            "journal global vide : les assertions d'absence de secret passeraient à vide"
        );
    }
}
