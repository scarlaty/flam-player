//! Identifiant et version d'une histoire faite d'un dossier (`docs/creation-dossier.md` § 8).
//!
//! # Règles
//!
//! - **Création** (aucun uuid connu pour ce dossier) : uuid v4 neuf, `version: 1`.
//! - **Actualisation** (uuid connu) : même uuid, version = la plus grande de celle du dossier et
//!   de celle installée dans « Mes histoires », plus 1 : la version publiée augmente toujours.
//! - **Où l'uuid est gardé** : dans `_histoire.json` (seul fichier écrit dans le dossier, et
//!   seulement ses champs `uuid` et `version` : le reste est relu brut juste avant et gardé tel
//!   quel, `settings::write_story_identity`). Si le dossier ne peut pas
//!   l'accueillir (lecture seule, fichier abîmé ou non lu, format plus récent, toute erreur
//!   d'écriture), il est gardé dans la configuration du launcher ([`REGISTRY_FILE`]), indexé
//!   par le chemin résolu du dossier. L'entrée de la configuration l'emporte sur l'uuid du
//!   fichier (elle est toujours plus récente) ; elle est oubliée dès que le fichier a pu être
//!   écrit.
//! - **Conflits** ([`Conflict`]) : l'uuid est déjà celui d'une histoire de « Mes histoires »
//!   venue d'**un autre dossier** qui existe encore et porte toujours cet uuid (dossier copié :
//!   « Remplacer l'autre » ou « Nouvelle histoire ») ; ou celui d'une histoire d'une **autre
//!   source** (catalogue, pack de podcasts : seulement « Nouvelle histoire », l'histoire
//!   installée n'est jamais remplacée). Un dossier déplacé (l'autre chemin n'existe plus ou n'a
//!   plus cet uuid) n'est pas un conflit : l'histoire suit le dossier.
//! - Un uuid nul (`00000000-…`) vaut absent.
//!
//! Les chemins comparés sont les formes résolues (`authoring::root::check_root`), sans casse.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::build::SOURCE_ID;
use super::settings::{self, Loaded, StorySettings};
use crate::library::LibraryEntry;
use crate::storage;
use crate::{AppError, AppResult};

/// Fichier de la configuration du launcher qui garde l'uuid des dossiers en lecture seule.
pub const REGISTRY_FILE: &str = "authoring.json";
/// Version du schéma de [`REGISTRY_FILE`].
const REGISTRY_SCHEMA: u32 = 1;
/// Dossiers retenus au plus (les plus anciens sont oubliés).
const MAX_REMEMBERED: usize = 1_000;

/// Uuid et version d'un dossier gardés par le launcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remembered {
    /// Chemin résolu du dossier.
    pub path: String,
    pub uuid: String,
    pub version: u32,
    /// Uuid lu dans `_histoire.json` quand l'entrée a été posée (`None` : aucun). L'entrée ne
    /// vaut que si le fichier porte toujours cet uuid : sinon c'est un autre dossier au même
    /// chemin (lettre de lecteur réutilisée, dossier recréé) ou un uuid écrit à la main depuis.
    #[serde(default)]
    pub file_uuid: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct RegistryFile {
    schema_version: u32,
    #[serde(default)]
    folders: Vec<Remembered>,
}

/// Clé de comparaison d'un chemin : texte en minuscules (Windows ignore la casse).
pub fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// Registre des uuid des dossiers en lecture seule (`<config>/authoring.json`). L'appelant
/// tient le verrou de la configuration (`ConfigState::guard`) pendant lecture et écriture.
#[derive(Debug, Clone)]
pub struct Registry {
    file: PathBuf,
}

impl Registry {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            file: config_dir.join(REGISTRY_FILE),
        }
    }

    /// Entrées lues ; fichier absent, corrompu (mis de côté) ou trop récent : aucune.
    pub fn load(&self) -> Vec<Remembered> {
        match storage::load_versioned::<RegistryFile>(&self.file, REGISTRY_SCHEMA, |_, _| {}) {
            Ok(storage::Loaded::Found(file)) => file
                .folders
                .into_iter()
                .filter(|r| is_usable_uuid(&r.uuid) && r.version >= 1)
                .collect(),
            Ok(_) => Vec::new(),
            Err(err) => {
                tracing::warn!("registre des dossiers d'histoire ignoré : {err}");
                Vec::new()
            }
        }
    }

    fn save(&self, mut folders: Vec<Remembered>) -> AppResult<()> {
        if folders.len() > MAX_REMEMBERED {
            folders.drain(..folders.len() - MAX_REMEMBERED);
        }
        let file = RegistryFile {
            schema_version: REGISTRY_SCHEMA,
            folders,
        };
        let mut bytes = serde_json::to_vec_pretty(&file)?;
        bytes.push(b'\n');
        storage::write_atomic(&self.file, &bytes)?;
        Ok(())
    }

    /// Entrée du dossier `base`.
    pub fn find(&self, base: &Path) -> Option<Remembered> {
        let key = path_key(base);
        self.load()
            .into_iter()
            .rev()
            .find(|r| path_key(Path::new(&r.path)) == key)
    }

    /// Retient `uuid` et `version` pour le dossier `base` (remplace l'entrée précédente).
    pub fn remember(
        &self,
        base: &Path,
        uuid: &str,
        version: u32,
        file_uuid: Option<&str>,
    ) -> AppResult<()> {
        let key = path_key(base);
        let mut folders = self.load();
        folders.retain(|r| path_key(Path::new(&r.path)) != key);
        folders.push(Remembered {
            path: base.to_string_lossy().into_owned(),
            uuid: uuid.to_string(),
            version,
            file_uuid: file_uuid.map(str::to_string),
        });
        self.save(folders)
    }

    /// Oublie le dossier `base` (sans effet s'il n'est pas retenu).
    pub fn forget(&self, base: &Path) -> AppResult<()> {
        let key = path_key(base);
        let mut folders = self.load();
        let before = folders.len();
        folders.retain(|r| path_key(Path::new(&r.path)) != key);
        if folders.len() == before {
            return Ok(());
        }
        self.save(folders)
    }
}

/// Uuid valide, canonique et non nul.
fn is_usable_uuid(uuid: &str) -> bool {
    uuid::Uuid::parse_str(uuid).is_ok_and(|u| !u.is_nil() && u.hyphenated().to_string() == uuid)
}

/// Conflit d'uuid (§ 8), exposé au front qui fait choisir l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Conflict {
    /// Dossier copié : l'histoire installée de même uuid vient de `other_folder`, qui existe
    /// encore avec cet uuid. Choix : « Remplacer l'autre » ou « Nouvelle histoire ».
    Copied { other_folder: String, title: String },
    /// Uuid d'une histoire d'une autre source (catalogue, pack…) : « Nouvelle histoire »
    /// seulement.
    Foreign { title: String },
}

/// Choix de l'utilisateur face à un [`Conflict`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictChoice {
    /// Garder l'uuid : l'histoire installée (venue de l'autre dossier) est remplacée.
    ReplaceOther,
    /// Nouvel uuid : une seconde histoire.
    NewStory,
}

/// Ce que l'on sait de l'identité d'un dossier avant de créer l'histoire.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Identity {
    /// Uuid connu (dossier, sinon configuration du launcher).
    pub uuid: Option<String>,
    /// Version connue avec cet uuid.
    pub version: Option<u32>,
    /// L'uuid vient de la configuration du launcher (dossier en lecture seule).
    pub remembered: bool,
    /// Version installée dans « Mes histoires » pour cet uuid.
    pub installed_version: Option<u32>,
    pub conflict: Option<Conflict>,
}

/// Uuid et version retenus pour une création.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub uuid: String,
    pub version: u32,
    /// Première création (uuid neuf).
    pub created: bool,
}

/// Où l'identité a été enregistrée.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SavedIn {
    /// `_histoire.json` du dossier.
    Folder,
    /// Configuration du launcher (dossier en lecture seule ou réglages non réécrivables).
    Launcher,
}

/// Uuid et version d'un dossier `base`, d'après ses réglages `story` (lus par le scan) puis
/// l'entrée `remembered` de la configuration.
fn known(
    story: &StorySettings,
    remembered: Option<&Remembered>,
) -> (Option<String>, Option<u32>, bool) {
    let file_uuid = story.uuid.as_deref().filter(|u| is_usable_uuid(u));
    // L'entrée de la configuration est plus récente que l'uuid du fichier qu'elle a vu
    // (`commit` ne la pose que s'il n'a pas pu écrire le fichier, et l'oublie dès qu'il y
    // parvient : dossier copié puis « Nouvelle histoire » sur un dossier non inscriptible). Elle
    // ne vaut que si le fichier porte toujours cet uuid ; sinon le fichier l'emporte.
    if let Some(r) = remembered.filter(|r| r.file_uuid.as_deref() == file_uuid) {
        let version = match (file_uuid, story.version) {
            (Some(uuid), Some(v)) if uuid == r.uuid => r.version.max(v),
            _ => r.version,
        };
        return (Some(r.uuid.clone()), Some(version), true);
    }
    match file_uuid {
        Some(uuid) => (Some(uuid.to_string()), story.version, false),
        None => (None, None, false),
    }
}

/// Identité du dossier `base` : uuid connu, version, conflit éventuel avec les histoires
/// installées `library`.
pub fn inspect(
    base: &Path,
    story: &Loaded<StorySettings>,
    remembered: Option<&Remembered>,
    library: &[LibraryEntry],
) -> Identity {
    let (uuid, version, from_registry) = known(&story.settings, remembered);
    let Some(uuid) = uuid else {
        return Identity::default();
    };
    let installed = library.iter().find(|e| e.uuid == uuid);
    let conflict = installed.and_then(|entry| conflict_with(base, &uuid, entry));
    Identity {
        installed_version: installed.and_then(|e| e.version),
        uuid: Some(uuid),
        version,
        remembered: from_registry,
        conflict,
    }
}

/// Conflit entre le dossier `base` (uuid `uuid`) et l'histoire installée `entry` de même uuid.
fn conflict_with(base: &Path, uuid: &str, entry: &LibraryEntry) -> Option<Conflict> {
    let title = entry.title.clone();
    if entry.source.source_id != SOURCE_ID {
        return Some(Conflict::Foreign { title });
    }
    let other = PathBuf::from(&entry.source.download_url);
    if !other.is_absolute() || path_key(&other) == path_key(base) {
        return None;
    }
    // Dossier déplacé : l'autre chemin n'existe plus, ou ne porte plus cet uuid.
    let still_there = std::fs::symlink_metadata(&other).is_ok_and(|m| m.is_dir())
        && settings::load_story(&other).settings.uuid.as_deref() == Some(uuid);
    still_there.then(|| Conflict::Copied {
        other_folder: other.to_string_lossy().into_owned(),
        title,
    })
}

/// Uuid et version de la création, selon `identity` et le choix `choice` de l'utilisateur.
/// Un conflit sans choix, ou « Remplacer » une histoire d'une autre source, est refusé.
pub fn decide(identity: &Identity, choice: Option<ConflictChoice>) -> AppResult<Decision> {
    let fresh = || Decision {
        uuid: uuid::Uuid::new_v4().hyphenated().to_string(),
        version: 1,
        created: true,
    };
    match (&identity.conflict, choice) {
        (_, Some(ConflictChoice::NewStory)) => return Ok(fresh()),
        (Some(Conflict::Foreign { title }), _) => {
            return Err(AppError::Validation(format!(
                "l'identifiant de ce dossier est déjà celui de « {title} », installée depuis une \
                 autre source : choisissez « Nouvelle histoire »"
            )))
        }
        (Some(Conflict::Copied { .. }), None) => {
            return Err(AppError::Validation(
                "ce dossier semble copié d'un autre dossier d'histoire : choisissez « Remplacer \
                 l'autre » ou « Nouvelle histoire »"
                    .into(),
            ))
        }
        _ => {}
    }
    match &identity.uuid {
        None => Ok(fresh()),
        Some(uuid) => {
            let last = identity
                .version
                .unwrap_or(0)
                .max(identity.installed_version.unwrap_or(0));
            let version = last
                .checked_add(1)
                .ok_or_else(|| AppError::Validation("version de l'histoire trop grande".into()))?;
            Ok(Decision {
                uuid: uuid.clone(),
                version,
                created: false,
            })
        }
    }
}

/// Enregistre `decision` : dans `_histoire.json` du dossier `base` si possible (seuls `uuid` et
/// `version` y changent ; abîmé, non lu ou d'un format plus récent : jamais réécrit), sinon dans
/// `registry`. Rend l'endroit choisi.
pub fn commit(base: &Path, decision: &Decision, registry: &Registry) -> AppResult<SavedIn> {
    // Le reste du fichier est gardé tel quel (valeurs que la lecture tolérante nettoierait
    // comprises).
    let written = settings::write_story_identity(base, &decision.uuid, decision.version);
    match written {
        Ok(()) => {
            if let Err(err) = registry.forget(base) {
                tracing::warn!("registre des dossiers d'histoire non mis à jour : {err}");
            }
            Ok(SavedIn::Folder)
        }
        Err(err) => {
            tracing::info!(
                "uuid de l'histoire gardé par le launcher (dossier non inscriptible) : {}",
                err.kind()
            );
            let file_uuid = settings::load_story(base)
                .settings
                .uuid
                .filter(|u| is_usable_uuid(u));
            registry.remember(base, &decision.uuid, decision.version, file_uuid.as_deref())?;
            Ok(SavedIn::Launcher)
        }
    }
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
