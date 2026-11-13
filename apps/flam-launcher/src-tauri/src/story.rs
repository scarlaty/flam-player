//! Type commun décrivant une histoire proposée par une source (catalogue ou dossier local).
//!
//! Partagé par `catalog` (#12) et `folder` (#13) ; miroir : `StoryEntry` dans `src/api/story.ts`.

/// Noms de fichier réservés par Windows (avec ou sans extension).
const WINDOWS_RESERVED: [&str; 4] = ["CON", "PRN", "AUX", "NUL"];

/// Vrai si `id` est un identifiant d'histoire utilisable tel quel comme nom de fichier ou de
/// dossier : motif `^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`, sans `..`, sans point final (Windows le
/// supprime) et hors noms réservés Windows (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`COM9`,
/// `LPT1`-`LPT9`, quelle que soit la casse, extension éventuelle comprise : `con.txt`).
pub fn validate_story_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 || !bytes[0].is_ascii_alphanumeric() {
        return false;
    }
    if !bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return false;
    }
    if id.contains("..") || id.ends_with('.') {
        return false;
    }
    let stem = id.split('.').next().unwrap_or(id).to_ascii_uppercase();
    let numbered = |prefix: &str| {
        stem.strip_prefix(prefix)
            .is_some_and(|n| matches!(n.as_bytes(), [b'1'..=b'9']))
    };
    !(WINDOWS_RESERVED.contains(&stem.as_str()) || numbered("COM") || numbered("LPT"))
}

use serde::{Deserialize, Serialize};

/// Nature de l'archive d'une histoire.
///
/// **Variante persistée** : `kind` est écrit dans `library.json` et `install_queue.json`. Une
/// version antérieure du launcher qui lit une variante qu'elle ne connaît pas échoue à la
/// désérialisation et met **tout** le fichier en quarantaine (`*.bak`), alors qu'elle refuse
/// intact un fichier de version de schéma supérieure (`config_version`, voir
/// `storage::load_versioned`). Règle pour toute nouvelle variante (#578) :
///
/// 1. la déclarer dans [`StoryKind::min_schema`] avec une **nouvelle** version de schéma (la
///    plus haute existante + 1) si une version publiée du launcher ne la connaît pas ;
/// 2. monter `library::SCHEMA_VERSION` et `install::QUEUE_SCHEMA` à cette version (lecture sans
///    migration : aucune donnée ne change) ;
/// 3. les fichiers restent écrits à la plus basse version qui décrit leur contenu (maximum des
///    `min_schema` de leurs entrées, 1 au moins), comme `SourcesStore::save` : sans la nouvelle
///    variante, revenir à une version antérieure reste possible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    /// Archive `.zip` au format Telmi (à convertir avec `telmi2flam`).
    TelmiZip,
    /// Archive `.zip` au format STUdio (`story.json`, `assets/`). Installée en pack Lunii
    /// `.lunii.pk` (`install::studio`) pour une Flam 2.x ; jamais jouable sur le PC (#517, Q1).
    StudioZip,
    /// Dossier `.plain` FLAM (déjà converti).
    FlamPlain,
    /// Archive `.plain.pk` FLAM (zip « stored », déjà converti).
    FlamPk,
}

impl StoryKind {
    /// Première version de schéma de `library.json` et `install_queue.json` qui connaît cette
    /// variante (voir la règle ci-dessus) : 1 pour les variantes de la bêta.3, 2 pour
    /// `studio_zip` (bêta.4, inconnue de la bêta.3).
    pub fn min_schema(self) -> u32 {
        match self {
            Self::TelmiZip | Self::FlamPlain | Self::FlamPk => 1,
            Self::StudioZip => 2,
        }
    }
}

/// Plus basse version de schéma qui décrit des entrées de natures `kinds` (1 au moins).
pub fn schema_for(kinds: impl IntoIterator<Item = StoryKind>) -> u32 {
    kinds
        .into_iter()
        .map(StoryKind::min_schema)
        .max()
        .unwrap_or(1)
        .max(1)
}

/// Refus de jouer sur le PC une histoire STUdio (archive d'une source ou pack Lunii installé) :
/// flam-player ne lit que le format Flam (`docs/studio-lunii.md`, § 5.6, Q1).
pub const STUDIO_NOT_PLAYABLE: &str = "Lecture sur le PC indisponible pour une histoire STUdio : installez-la, puis envoyez-la sur une Flam en firmware 2.x.";

/// Longueur maximale d'un titre d'histoire lu au scan (caractères), partagée par les sources et
/// la conversion STUdio (#551).
pub const MAX_TITLE_CHARS: usize = 200;

/// Une histoire d'une source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryEntry {
    /// Identifiant de la source qui la propose.
    pub source_id: String,
    /// Identifiant stable et unique **dans une source**, clé de grille de l'interface et clé de
    /// cache des vignettes côté front. Opaque pour le front. Catalogue : `"<source_id>:<uuid>"`.
    /// Dossier local : défini par la source (par exemple dérivé du chemin relatif).
    pub entry_id: String,
    /// Identifiant de l'histoire (Telmi : champ `uuid` ; FLAM : `uuid.bin` en hexadécimal).
    pub uuid: String,
    pub version: Option<u32>,
    pub title: String,
    pub description: Option<String>,
    pub age: Option<u32>,
    pub category: Option<String>,
    pub author: Option<String>,
    pub voice: Option<String>,
    pub license: Option<String>,
    /// Référence de la vignette, **pas** une URL affichable : la CSP bloque les images distantes.
    /// Le front la résout via la commande de vignette de la source (`getCatalogThumbnail` pour un
    /// catalogue), qui renvoie une URL `data:`. Pour un dossier local, la source peut fournir
    /// directement une URL `data:`.
    pub thumb_url: Option<String>,
    /// Source du contenu : URL pour un catalogue, chemin local absolu pour un dossier.
    pub download_url: String,
    /// Taille en octets, si connue.
    pub size: Option<u64>,
    /// Date de dernière modification (ISO 8601 UTC), si connue.
    pub updated_at: Option<String>,
    /// Nombre de téléchargements publié par le catalogue (`download_count`), si connu. Sert au
    /// tri par popularité ; absent pour un dossier ou la bibliothèque.
    #[serde(default)]
    pub download_count: Option<u64>,
    pub kind: StoryKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiants_valides() {
        for id in [
            "a",
            "0",
            "3f2c9a10-77aa-4b1e-9c11-0123456789ab",
            "Histoire_1.v2",
            "a.b-c_d",
            "COM10",
            "CONSOLE",
            "LPT0",
            "COM0",
            &"x".repeat(64),
        ] {
            assert!(validate_story_id(id), "{id}");
        }
    }

    #[test]
    fn identifiants_refuses() {
        for id in [
            "",
            ".a",
            "-a",
            "_a",
            "a/b",
            r"a\b",
            "a b",
            "a:b",
            "a..b",
            "..",
            "a.",
            "é",
            "a\0b",
            "a\nb",
            &"x".repeat(65),
        ] {
            assert!(!validate_story_id(id), "{id:?}");
        }
    }

    #[test]
    fn noms_reserves_windows_refuses() {
        for id in [
            "CON",
            "con",
            "Prn",
            "AUX",
            "nul",
            "COM1",
            "com9",
            "LPT1",
            "lpt9",
            "con.txt",
            "NUL.tar.gz",
            "Com3.x",
        ] {
            assert!(!validate_story_id(id), "{id}");
        }
    }

    #[test]
    fn le_kind_est_serialise_en_snake_case() {
        let json = |k| serde_json::to_value(k).unwrap();
        assert_eq!(json(StoryKind::TelmiZip), "telmi_zip");
        assert_eq!(json(StoryKind::StudioZip), "studio_zip");
        assert_eq!(json(StoryKind::FlamPlain), "flam_plain");
        assert_eq!(json(StoryKind::FlamPk), "flam_pk");
    }

    /// #578 : seule une variante inconnue de la bêta.3 fait monter la version écrite.
    #[test]
    fn version_de_schema_selon_les_variantes() {
        assert_eq!(schema_for([]), 1);
        assert_eq!(
            schema_for([StoryKind::TelmiZip, StoryKind::FlamPlain, StoryKind::FlamPk]),
            1
        );
        assert_eq!(schema_for([StoryKind::FlamPk, StoryKind::StudioZip]), 2);
    }
}
