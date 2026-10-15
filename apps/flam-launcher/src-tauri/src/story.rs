//! Type commun décrivant une histoire proposée par une source (catalogue ou dossier local).
//!
//! Partagé par `catalog` (#12) et `folder` (#13) ; miroir : `StoryEntry` dans `src/api/story.ts`.

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
    /// Vignette affichable telle quelle par le front (URL `data:` ou `http(s):`).
    pub thumb_url: Option<String>,
    /// Source du contenu : URL pour un catalogue, chemin local absolu pour un dossier.
    pub download_url: String,
    /// Taille en octets, si connue.
    pub size: Option<u64>,
    /// Date de dernière modification (ISO 8601 UTC), si connue.
    pub updated_at: Option<String>,
    pub kind: StoryKind,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_kind_est_serialise_en_snake_case() {
        let json = |k| serde_json::to_value(k).unwrap();
        assert_eq!(json(StoryKind::TelmiZip), "telmi_zip");
        assert_eq!(json(StoryKind::FlamPlain), "flam_plain");
        assert_eq!(json(StoryKind::FlamPk), "flam_pk");
    }
}
