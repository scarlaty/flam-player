//! Titres lus à voix haute dans un pack de podcasts (#462, PR 2, note `docs/podcasts.md` § 12).
//!
//! Étape de la préparation (`install::podcast`), quand la composition le demande
//! (`titles_audio`) : chaque titre d'épisode, chaque nom de catégorie et le titre du pack sont
//! synthétisés par la voix Windows choisie ([`crate::podcast::speech`]) puis convertis en MP3
//! dans le dossier de travail ; l'archive (`podcast::archive`) les place dans l'audio des scènes
//! d'option et dans `title.mp3`.
//!
//! # Invariants
//!
//! - **Voix** ([`choose_voice`]) : voix demandée si elle existe encore, sinon voix française par
//!   défaut avec un avertissement ; `tts_no_voice` seulement s'il n'y a plus aucune voix
//!   française ; synthèse indisponible : `tts_unavailable`. L'énumération est bornée par un délai
//!   global ([`speech::VOICES_TOTAL_TIMEOUT`]).
//! - **Avant tout téléchargement** : l'étape tourne avant les téléchargements, pour qu'une voix
//!   inutilisable arrête la création sans rien télécharger ni publier.
//! - **Échecs** ([`speak`]) : un titre isolé en échec (synthèse, WAV, conversion MP3) reste muet,
//!   avec un avertissement ; la création s'arrête (`tts_failed`) si le **premier** titre à lire
//!   échoue ou si **plus de la moitié** échouent. Un titre sans rien à lire (vide, sans lettre)
//!   reste muet sans avertissement et ne compte pas. Deux délais (ou indisponibilités) de
//!   synthèse consécutifs arrêtent aussi la création (service de voix figé).
//! - **Annulation** : vérifiée avant chaque titre et pendant la synthèse (le thread de synthèse,
//!   borné par son délai, est abandonné ; son résultat est jeté) et pendant la conversion.
//! - **Fichiers** : MP3 aux noms générés (`title-NNNN.mp3`) dans le dossier de travail, supprimé
//!   avec lui ; les temporaires de conversion ne restent jamais (`speech::wav_to_mp3`).
//! - **Vie privée** : ni texte ni voix dans les journaux (rang seulement) ; les titres non lus
//!   ne sont cités que dans les avertissements montrés à l'utilisateur.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::watch;

use super::podcast::PackBackend;
use crate::podcast::archive;
use crate::podcast::pack::{EpisodeRef, PackComposition, PackNode};
use crate::podcast::speech::{self, SpeechError, Synthesizer, Voice};
use crate::{AppError, AppResult};

/// Emplacement d'un titre lu : titre du pack, ou option désignée par sa position dans
/// l'arborescence de la composition (indices depuis le menu racine).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TitleKey {
    Pack,
    Node(Vec<usize>),
}

/// Titre à lire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleText {
    pub key: TitleKey,
    /// Texte d'origine (pas le libellé translittéré : la voix prononce mieux les accents).
    pub text: String,
}

/// Résultat de la synthèse des titres.
#[derive(Debug, Default)]
pub struct SpokenTitles {
    /// MP3 produits, par emplacement.
    pub audio: HashMap<TitleKey, PathBuf>,
    /// Titres en échec (restés muets), dans l'ordre : emplacement et texte.
    pub failed: Vec<(TitleKey, String)>,
}

/// Titres de la composition, dans l'ordre de lecture : titre du pack, puis les options en
/// profondeur (une catégorie, puis ses options), comme le parcours de l'archive.
/// `episode_title` rend le titre d'origine d'un épisode (`None` : épisode inconnu, sauté).
pub fn titles(
    composition: &PackComposition,
    pack_title: &str,
    episode_title: &dyn Fn(&EpisodeRef) -> Option<String>,
) -> Vec<TitleText> {
    fn walk(
        menu: &[PackNode],
        path: &mut Vec<usize>,
        episode_title: &dyn Fn(&EpisodeRef) -> Option<String>,
        out: &mut Vec<TitleText>,
    ) {
        for (i, node) in menu.iter().enumerate() {
            path.push(i);
            match node {
                PackNode::Episode(e) => {
                    if let Some(text) = episode_title(e) {
                        out.push(TitleText {
                            key: TitleKey::Node(path.clone()),
                            text,
                        });
                    }
                }
                PackNode::Category { name, entries } => {
                    out.push(TitleText {
                        key: TitleKey::Node(path.clone()),
                        text: name.trim().to_string(),
                    });
                    walk(entries, path, episode_title, out);
                }
            }
            path.pop();
        }
    }
    let mut out = vec![TitleText {
        key: TitleKey::Pack,
        text: pack_title.to_string(),
    }];
    walk(&composition.menu, &mut Vec::new(), episode_title, &mut out);
    out
}

/// Message de la voix disparue (repli sur la voix par défaut). Sans nom de voix (vie privée).
pub const VOICE_FALLBACK_WARNING: &str = "La voix choisie pour lire les titres n'est plus \
     installée dans Windows : la voix française par défaut a été utilisée.";

/// Voix des titres (bloquant, borné par [`speech::VOICES_TOTAL_TIMEOUT`]) : `requested` s'il
/// existe encore, sinon la voix française par défaut avec un avertissement (rendu en second).
pub fn choose_voice(
    synth: &Arc<dyn Synthesizer>,
    requested: Option<&str>,
) -> AppResult<(Voice, Option<String>)> {
    let list = speech::french_voices_within(synth, speech::VOICES_TOTAL_TIMEOUT)?;
    match speech::resolve_voice(&list, requested) {
        Ok(voice) => Ok((voice, None)),
        Err(SpeechError::VoiceGone) => {
            let voice = speech::resolve_voice(&list, None)?;
            tracing::warn!("titres lus : voix choisie disparue, repli sur la voix par défaut");
            Ok((voice, Some(VOICE_FALLBACK_WARNING.to_string())))
        }
        Err(e) => Err(e.into()),
    }
}

/// Avertissement d'un titre resté muet (montré à l'utilisateur, jamais journalisé). Le titre du
/// pack est lu en premier : son échec arrête la création ; sa branche est défensive.
pub fn failure_warning(key: &TitleKey, text: &str) -> String {
    match key {
        TitleKey::Pack => "Titre du pack non lu à voix haute (synthèse en échec).".to_string(),
        TitleKey::Node(_) => format!(
            "« {} » : titre non lu à voix haute (synthèse en échec), option muette.",
            archive::label(text, "?")
        ),
    }
}

/// Délais (ou indisponibilités) de synthèse consécutifs qui arrêtent la création.
const MAX_STALLED: usize = 2;

fn stopped(message: String) -> AppError {
    AppError::TtsFailed(message)
}

/// Synthèse d'un titre préparé, dans un thread bloquant, interrompue par l'annulation.
async fn synthesize(
    synth: &Arc<dyn Synthesizer>,
    text: &str,
    voice_id: &str,
    cancel: &watch::Receiver<bool>,
) -> AppResult<Result<speech::Wav, SpeechError>> {
    let (engine, text, voice) = (Arc::clone(synth), text.to_owned(), voice_id.to_owned());
    let job = tokio::task::spawn_blocking(move || {
        speech::synthesize_wav(&engine, &text, &voice, speech::SYNTH_TIMEOUT)
    });
    let mut cancel = cancel.clone();
    tokio::select! {
        joined = job => Ok(joined.unwrap_or_else(|_| {
            Err(SpeechError::Failed("synthèse interrompue".into()))
        })),
        // Le thread de synthèse, borné par son délai, finit seul ; son résultat est jeté.
        _ = super::download::cancelled(&mut cancel) => Err(AppError::Cancelled),
    }
}

/// Synthétise et convertit `titles` dans `dir` avec `voice`. Voir les règles d'échec et
/// d'annulation dans la documentation du module.
pub async fn speak(
    backend: &Arc<dyn PackBackend>,
    synth: &Arc<dyn Synthesizer>,
    voice: &Voice,
    titles: &[TitleText],
    dir: &Path,
    cancel: &watch::Receiver<bool>,
) -> AppResult<SpokenTitles> {
    let speakable = titles
        .iter()
        .filter(|t| speech::prepare_text(&t.text).is_some())
        .count();
    let mut out = SpokenTitles::default();
    let mut attempted = 0usize;
    let mut stalled = 0usize;
    for (rank, title) in titles.iter().enumerate() {
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        if speech::prepare_text(&title.text).is_none() {
            // Rien à lire : muet, sans avertissement.
            continue;
        }
        attempted += 1;
        let result = match synthesize(synth, &title.text, &voice.id, cancel).await? {
            Ok(wav) => {
                let dest = dir.join(format!("title-{rank:04}.mp3"));
                match backend.title_mp3(&wav, &dest, cancel).await {
                    Ok(_) => Ok(dest),
                    Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                    Err(err) => {
                        let _ = tokio::fs::remove_file(&dest).await;
                        Err(err.kind().to_string())
                    }
                }
            }
            Err(err) => {
                if matches!(err, SpeechError::Timeout | SpeechError::Unavailable) {
                    stalled += 1;
                }
                Err(format!("{err:?}"))
            }
        };
        if result.is_ok() {
            stalled = 0;
        }
        match result {
            Ok(path) => {
                out.audio.insert(title.key.clone(), path);
            }
            Err(reason) => {
                // Rang et cause technique seulement (D11, vie privée).
                tracing::warn!(rank, "titre lu en échec : {reason}");
                // Service de voix figé : deux délais (ou indisponibilités) de suite arrêtent la
                // création au lieu d'attendre 10 s par titre jusqu'à la règle de la moitié.
                if stalled >= MAX_STALLED {
                    return Err(stopped(
                        "La voix de Windows ne répond plus : le pack n'a pas été créé. \
                         Réessayez, ou décochez « Lire les titres à voix haute »."
                            .into(),
                    ));
                }
                if attempted == 1 {
                    return Err(stopped(
                        "Le premier titre n'a pas pu être lu à voix haute : le pack n'a pas été \
                         créé. Vérifiez les voix de Windows (Paramètres > Heure et langue > Voix) \
                         ou décochez « Lire les titres à voix haute »."
                            .into(),
                    ));
                }
                out.failed.push((title.key.clone(), title.text.clone()));
                if out.failed.len() * 2 > speakable {
                    return Err(stopped(format!(
                        "Plus de la moitié des titres n'ont pas pu être lus à voix haute ({} sur \
                         {speakable}) : le pack n'a pas été créé. Vérifiez les voix de Windows ou \
                         décochez « Lire les titres à voix haute ».",
                        out.failed.len()
                    )));
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::podcast::speech::testing::{voice, Simulated};

    fn e(n: usize) -> PackNode {
        PackNode::Episode(EpisodeRef {
            source_id: "s".into(),
            episode_id: format!("e{n}"),
        })
    }

    #[test]
    fn les_titres_suivent_l_ordre_du_parcours() {
        let composition = PackComposition {
            menu: vec![
                e(1),
                PackNode::Category {
                    name: "  Saison  ".into(),
                    entries: vec![e(2), e(3)],
                },
                e(4),
            ],
            ..PackComposition::default()
        };
        let list = titles(&composition, "Mon pack", &|r| {
            (r.episode_id != "e3").then(|| format!("Titre {}", r.episode_id))
        });
        let got: Vec<(TitleKey, &str)> = list.iter().map(|t| (t.key.clone(), &*t.text)).collect();
        assert_eq!(
            got,
            [
                (TitleKey::Pack, "Mon pack"),
                (TitleKey::Node(vec![0]), "Titre e1"),
                (TitleKey::Node(vec![1]), "Saison"),
                (TitleKey::Node(vec![1, 0]), "Titre e2"),
                (TitleKey::Node(vec![2]), "Titre e4"),
            ]
        );
    }

    #[test]
    fn la_voix_disparue_donne_la_voix_par_defaut_avec_un_avertissement() {
        let synth = Simulated::new(vec![
            voice("fr-a", "A", "fr-FR"),
            voice("fr-b", "B", "fr-CA"),
        ])
        .shared();
        let (v, warning) = choose_voice(&synth, Some("fr-b")).unwrap();
        assert_eq!((v.id.as_str(), warning), ("fr-b", None));
        let (v, warning) = choose_voice(&synth, None).unwrap();
        assert_eq!((v.id.as_str(), warning), ("fr-a", None));
        let (v, warning) = choose_voice(&synth, Some("disparue")).unwrap();
        assert_eq!(v.id, "fr-a");
        assert_eq!(warning.as_deref(), Some(VOICE_FALLBACK_WARNING));
    }

    #[test]
    fn sans_voix_francaise_ou_sans_synthese_la_creation_est_refusee() {
        let none = Simulated::new(vec![voice("en", "Zira", "en-US")]).shared();
        let err = choose_voice(&none, Some("en")).unwrap_err();
        assert_eq!(err.kind(), "tts_no_voice");
        let down = Simulated {
            unavailable: true,
            ..Simulated::french()
        }
        .shared();
        assert_eq!(
            choose_voice(&down, None).unwrap_err().kind(),
            "tts_unavailable"
        );
    }

    #[test]
    fn l_avertissement_d_un_titre_cite_son_libelle() {
        let w = failure_warning(&TitleKey::Node(vec![0]), "L’été “chaud”");
        assert!(w.contains("« L'été \"chaud\" »"), "{w}");
        assert!(failure_warning(&TitleKey::Pack, "x").contains("Titre du pack"));
    }
}
