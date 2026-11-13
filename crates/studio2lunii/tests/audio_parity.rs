//! Parité de `audio` avec Lunii.QT (`convert_audio.py`, qui repose sur `mutagen` 1.48).
//!
//! Méthode de l'oracle : un script Python hors dépôt fabrique des MP3 **synthétiques** (trames
//! MPEG-1/2 couches II et III à zéro, en-têtes Xing / Info / LAME / VBRI, étiquettes ID3v2.3,
//! ID3v2.4, ID3v1, APEv2 ; aucune musique) dans `tests/data/audio/<nom>.mp3`, appelle
//! `transcoding_required` et `tags_removal_required` de Lunii.QT (commit c8afe43) et, pour le
//! nettoyage, `mutagen.id3.delete` (la sémantique voulue de `mp3_tag_cleanup`, dont le fichier
//! temporaire n'est pas vidé avant d'être relu : la fonction elle-même échoue dans notre banc),
//! puis consigne les résultats dans `tests/data/audio/oracle.txt` (et `<nom>.clean.mp3`).
//!
//! Écarts attendus et choisis (note de conception, § 3.6) : 44 100 Hz exactement (Lunii.QT
//! accepte 48 kHz), MPEG-1 couche III seulement, première trame à l'octet 0 une fois les
//! étiquettes écartées (`mutagen` en cherche une sur 1 Mio : les octets avant elle forcent ici
//! le transcodage, car la copie les garderait) et trames lues de bout en bout ; étiquettes
//! APEv2, ID3v2 empilées et pied ID3v2.4 retirés aussi ; une étiquette vide compte comme
//! étiquette.

use std::fs;
use std::path::PathBuf;

use studio2lunii::audio::{mp3_tag_cleanup, tags_removal_required, transcoding_required};

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/data/audio")
}

struct Case {
    name: String,
    filename: String,
    data: Vec<u8>,
    transcode: Option<bool>,
    tags: Option<bool>,
    clean: Option<Vec<u8>>,
}

fn tri(s: &str) -> Option<bool> {
    match s {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn cases() -> Vec<Case> {
    let oracle = fs::read_to_string(dir().join("oracle.txt")).unwrap();
    oracle
        .lines()
        .filter(|l| !l.starts_with('#') && !l.is_empty())
        .map(|l| {
            let f: Vec<&str> = l.split('|').collect();
            Case {
                name: f[0].to_string(),
                filename: f[1].to_string(),
                data: fs::read(dir().join(format!("{}.mp3", f[0]))).unwrap(),
                transcode: tri(f[2]),
                tags: tri(f[3]),
                clean: (f[4] == "ok")
                    .then(|| fs::read(dir().join(format!("{}.clean.mp3", f[0]))).unwrap()),
            }
        })
        .collect()
}

/// Cas où notre décision est volontairement plus stricte que celle de Lunii.QT : fréquence et
/// couche, et octets avant la première trame (#580 : la copie gardée ne doit contenir que des trames).
const STRICTER_TRANSCODE: &[&str] = &[
    "mono_48000_cbr",
    "layer2_mono_44100",
    "junk_then_frames",
    "false_sync_then_frames",
];
/// Étiquettes que `mutagen` ne compte pas (aucune trame / APEv2) mais que nous retirons.
const EXTRA_TAGS: &[&str] = &["tag_v2_empty_no_frames", "tag_v1_all_empty", "tag_ape_tail"];
/// Nettoyages où nous retirons plus que `mutagen`.
const EXTRA_CLEANUP: &[&str] = &[
    "tag_id3v2_stacked",
    "tag_id3v24_footer",
    "tag_ape_tail",
    "tag_ape_and_v1",
];

#[test]
fn fixtures_are_present() {
    assert!(cases().len() >= 35);
}

#[test]
fn transcoding_decision_matches_lunii_qt() {
    let mut failures = Vec::new();
    for c in cases() {
        // Une exception de Lunii.QT (aucune trame) devient « à transcoder ».
        let lunii = c.transcode.unwrap_or(true);
        let expected = lunii || STRICTER_TRANSCODE.contains(&c.name.as_str());
        let got = transcoding_required(&c.filename, &c.data);
        if got != expected {
            failures.push(format!("{} : attendu {expected}, obtenu {got}", c.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn tag_detection_matches_lunii_qt() {
    let mut failures = Vec::new();
    for c in cases() {
        let Some(lunii) = c.tags else { continue };
        let expected = lunii || EXTRA_TAGS.contains(&c.name.as_str());
        let got = tags_removal_required(&c.data);
        if got != expected {
            failures.push(format!("{} : attendu {expected}, obtenu {got}", c.name));
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn tag_cleanup_matches_mutagen() {
    let bare = fs::read(dir().join("mono_44100_cbr.mp3")).unwrap();
    let mut failures = Vec::new();
    for c in cases() {
        let Some(clean) = &c.clean else { continue };
        let got = mp3_tag_cleanup(&c.data).unwrap();
        let expected = if EXTRA_CLEANUP.contains(&c.name.as_str()) {
            // Nous retirons tout : il ne reste que le son nu.
            &bare
        } else {
            clean
        };
        if &got != expected {
            failures.push(format!(
                "{} : {} octets au lieu de {}",
                c.name,
                got.len(),
                expected.len()
            ));
        }
        assert!(
            !tags_removal_required(&got) || c.name == "tag_v1_all_empty",
            "{}",
            c.name
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn cleaning_never_touches_the_audio() {
    // Après nettoyage, un MP3 conforme reste « à garder » : les trames n'ont pas bougé.
    for name in [
        "tag_id3v23",
        "tag_v2_and_v1",
        "tag_with_xing",
        "tag_ape_and_v1",
    ] {
        let data = fs::read(dir().join(format!("{name}.mp3"))).unwrap();
        assert!(!transcoding_required("a.mp3", &data), "{name}");
        let clean = mp3_tag_cleanup(&data).unwrap();
        assert!(!transcoding_required("a.mp3", &clean), "{name}");
        assert_eq!(
            mp3_tag_cleanup(&clean).unwrap(),
            clean,
            "{name} : idempotence"
        );
    }
}
