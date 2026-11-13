//! Runtime au gel des références de parité (#140), partagé par les tests qui comparent un
//! paquet entier aux sorties figées de l'ancien convertisseur Python (`package_parity.rs`,
//! `mp3_stream.rs`, `ffmpeg.rs`, `progress_parity.rs`).
//!
//! Le runtime (`tools/telmi2flam/runtime`) est copié tel quel dans chaque paquet : les
//! références figées portent donc son contenu au gel. Quand un fichier du runtime évolue
//! volontairement, les références ne sont pas régénérées (PORT-RUST.md §4.5) : le fichier
//! figure dans [`UPDATED`] avec son contenu au gel, et avant comparaison l'entrée produite,
//! vérifiée identique au runtime embarqué, reprend ce contenu. Tout le reste du paquet (noms,
//! ordre, autres entrées, conteneur ZIP) reste comparé à l'octet près. Le contenu au gel est
//! lui-même contrôlé par les SHA-256 par entrée des vecteurs.
#![allow(dead_code)]

use telmi2flam::package::Data;
use telmi2flam::runtime::RUNTIME;

/// Fichiers du runtime modifiés depuis le gel : nom dans le paquet et contenu au gel (copie
/// sous `tests/data/runtime_frozen`). Liste fermée, une ligne par évolution, avec son issue.
pub const UPDATED: &[(&str, &[u8])] = &[
    // #620 : le carrousel ne retient plus que les sources des vignettes affichées.
    (
        "script/carousel_1_0_0.lua",
        include_bytes!("../data/runtime_frozen/script/carousel_1_0_0.lua"),
    ),
];

/// Contenu embarqué dans le binaire (copie de travail de `runtime/`).
fn shipped(name: &str) -> &'static [u8] {
    RUNTIME
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("{name} absent du runtime embarqué"))
        .data
}

/// Contenu au gel de l'entrée `name` si c'est un fichier du runtime modifié depuis le gel,
/// après avoir vérifié que `data` est bien le runtime embarqué.
pub fn frozen(name: &str, data: &[u8]) -> Option<&'static [u8]> {
    let (_, old) = UPDATED.iter().find(|(n, _)| *n == name)?;
    let now = shipped(name);
    assert_ne!(
        now, *old,
        "{name} : runtime identique au gel, retirer sa ligne de UPDATED"
    );
    assert!(
        data == now,
        "{name} : entrée différente du runtime embarqué"
    );
    Some(old)
}

/// [`frozen`] appliqué aux entrées d'un paquet en mémoire.
pub fn freeze_bytes(entries: &mut [(String, Vec<u8>)]) {
    for (name, data) in entries.iter_mut() {
        if let Some(old) = frozen(name, data) {
            *data = old.to_vec();
        }
    }
}

/// [`frozen`] appliqué aux entrées d'un paquet en flux (les fichiers du runtime sont des
/// octets produits, jamais des MP3 relus dans l'archive).
pub fn freeze_data(entries: &mut [(String, Data)]) {
    for (name, data) in entries.iter_mut() {
        if let Data::Bytes(bytes) = data {
            if let Some(old) = frozen(name, bytes) {
                *bytes = old.to_vec();
            }
        }
    }
}

/// Écart de taille d'une sortie complète (entrées *Stored* du `.plain.pk`, ou fichiers du
/// dossier `.plain`) : runtime embarqué moins runtime au gel.
pub fn size_delta() -> i64 {
    UPDATED
        .iter()
        .map(|(name, old)| shipped(name).len() as i64 - old.len() as i64)
        .sum()
}
