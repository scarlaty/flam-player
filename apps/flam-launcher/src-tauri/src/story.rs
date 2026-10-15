//! Type commun d'une histoire proposée par une source (catalogue ou dossier local).
//!
//! Module partagé : `catalog` (catalogues Telmi Store) et `folder` (dossiers locaux) produisent
//! tous deux des `StoryEntry`, consommées ensuite par l'interface et par le pipeline
//! d'installation. Aucune dépendance à Tauri.

use serde::{Deserialize, Serialize};

/// Nature de l'archive d'une histoire, donc chaîne de traitement à appliquer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoryKind {
    /// Archive `.zip` au format Telmi (à convertir avec `telmi2flam`).
    TelmiZip,
    /// Histoire Flam non compressée (`.plain`).
    FlamPlain,
    /// Histoire Flam empaquetée (`.plain.pk`).
    FlamPk,
}

/// Une histoire proposée par une source (miroir de `StoryEntry` dans `src/api/catalog.ts`).
///
/// Les textes absents valent la chaîne vide ; `version`, `age`, `thumb_url` et `size` sont
/// optionnels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoryEntry {
    /// Identifiant de la source d'origine.
    pub source_id: String,
    /// Identifiant de l'histoire (UUID Telmi), stable d'une version à l'autre.
    pub uuid: String,
    /// Nature de l'archive.
    pub kind: StoryKind,
    /// Numéro de version, quand la source en fournit un.
    pub version: Option<u32>,
    pub title: String,
    pub description: String,
    /// Âge minimal recommandé, quand il est connu.
    pub age: Option<u32>,
    pub category: String,
    pub author: String,
    pub voice: String,
    pub license: String,
    /// URL (absolue) de la vignette, à télécharger paresseusement.
    pub thumb_url: Option<String>,
    /// Où récupérer l'archive : URL http(s) absolue pour un catalogue, chemin de fichier
    /// pour un dossier local.
    pub download_url: String,
    /// Taille de l'archive en octets, si elle est connue.
    pub size: Option<u64>,
    /// Date de dernière mise à jour telle que fournie par la source (ISO 8601 en général).
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_kinds_sont_serialises_en_snake_case() {
        let json = |kind| serde_json::to_value(kind).unwrap();
        assert_eq!(json(StoryKind::TelmiZip), "telmi_zip");
        assert_eq!(json(StoryKind::FlamPlain), "flam_plain");
        assert_eq!(json(StoryKind::FlamPk), "flam_pk");
    }
}
