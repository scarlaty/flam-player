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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    /// Archive `.zip` au format Telmi (à convertir avec `telmi2flam`).
    TelmiZip,
    /// Dossier `.plain` FLAM (déjà converti).
    FlamPlain,
    /// Archive `.plain.pk` FLAM (zip « stored », déjà converti).
    FlamPk,
}

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
        assert_eq!(json(StoryKind::FlamPlain), "flam_plain");
        assert_eq!(json(StoryKind::FlamPk), "flam_pk");
    }
}
