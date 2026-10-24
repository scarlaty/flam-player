//! Compositions nommées réutilisables des packs de podcasts (#461, N3 c, spécification
//! `docs/podcasts.md` § 14).
//!
//! Après la création d'un pack, sa composition ([`PackComposition`], #362) est conservée sous
//! un nom dans `pack-compositions.json` (dossier de configuration). Le parent peut la rouvrir
//! (copie dans le brouillon unique, N3 b) pour refaire ou dériver un pack, la renommer, la
//! supprimer. Chaque pack reste une nouvelle histoire (P3) : rien ici ne touche à la file
//! d'installation ni à la bibliothèque.
//!
//! # Invariants
//!
//! - Toute modification est un cycle lecture-modification-écriture fait **sous le verrou de
//!   configuration** par l'appelant, terminé par une écriture atomique
//!   ([`storage::write_atomic`]) : un arrêt brutal laisse l'ancien fichier ou le nouveau.
//! - Rien n'est jamais perdu sans copie : un fichier illisible, trop gros ou d'un schéma plus
//!   récent est mis de côté (`*.bak`) ; un fichier lisible dont certaines entrées ne le sont
//!   pas est **copié** en `*.bak` avant d'être réécrit avec les seules entrées lisibles.
//! - [`MAX_COMPOSITIONS`] entrées au plus, aucune suppression automatique ; noms uniques sans
//!   tenir compte de la casse, de 1 à [`MAX_NAME_CHARS`] caractères.
//! - Aucune URL sur le disque ni dans les messages : une composition ne contient que des
//!   identifiants et des textes saisis (D11).
//! - Une composition identique à une entrée existante ne crée pas de doublon (« refaire ») ;
//!   une composition modifiée crée une nouvelle entrée (« dériver »), l'originale reste
//!   intacte.

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::State;

use super::pack::{self, PackComposition, SaveOutcome, MAX_REVISION};
use crate::sources::{Source, SourcesStore};
use crate::storage::{self, ConfigState, Loaded};
use crate::{AppError, AppResult};

/// Fichier des compositions, dans le dossier de configuration.
pub const COMPOSITIONS_FILE: &str = "pack-compositions.json";
/// Version de schéma du fichier.
pub const COMPOSITIONS_SCHEMA: u32 = 1;
/// Nombre de compositions conservées au plus.
pub const MAX_COMPOSITIONS: usize = 50;
/// Longueur d'un nom, en caractères.
pub const MAX_NAME_CHARS: usize = 60;
/// Taille d'une composition sérialisée au plus, en octets.
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// Taille du fichier au plus, en octets (lecture et écriture).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Longueur d'un identifiant de composition au plus.
const MAX_ID_LEN: usize = 64;
/// Nom donné à une composition dont le titre effectif est vide.
const FALLBACK_NAME: &str = "Pack";

/// Composition conservée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedComposition {
    /// Identifiant opaque (uuid v4 tiré par le launcher).
    pub id: String,
    pub name: String,
    /// Secondes Unix.
    #[serde(default)]
    pub created_at: u64,
    /// Date de la dernière création lancée depuis cette composition (secondes Unix).
    #[serde(default)]
    pub last_used_at: u64,
    /// Créations lancées depuis cette composition.
    #[serde(default)]
    pub packs: u32,
    pub composition: PackComposition,
}

/// Contenu écrit de `pack-compositions.json`.
#[derive(Serialize)]
struct StoreFile<'a> {
    schema_version: u32,
    compositions: &'a [SavedComposition],
}

/// Lecture tolérante : chaque entrée est désérialisée à part (une entrée illisible ne fait pas
/// perdre les autres).
#[derive(Deserialize)]
struct RawStore {
    #[serde(default)]
    compositions: Vec<Value>,
}

/// Migration du fichier (aucune : schéma 1 seulement ; version 0 = même forme sans
/// `schema_version`).
fn migrate(_from: u32, _object: &mut Map<String, Value>) {}

/// Écriture du fichier (production : [`storage::write_atomic`] ; tests : injection de pannes).
pub type Writer = fn(&Path, &[u8]) -> io::Result<()>;

/// Entrées lues, avec le message à afficher une fois (fichier mis de côté, entrées écartées).
#[derive(Debug, Default)]
pub struct StoreRead {
    pub entries: Vec<SavedComposition>,
    pub notice: Option<String>,
}

/// Accès au fichier des compositions de `dir`. L'appelant tient le verrou de configuration.
pub struct Store<'a> {
    dir: &'a Path,
    write: Writer,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Nom saisi, nettoyé : espaces de bord retirés ; `None` s'il est vide, trop long ou contient
/// un caractère de contrôle.
fn checked_name(name: &str) -> Option<&str> {
    let name = name.trim();
    let len = name.chars().count();
    (len > 0 && len <= MAX_NAME_CHARS && !name.chars().any(char::is_control)).then_some(name)
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn name_taken(entries: &[SavedComposition], name: &str, except: Option<&str>) -> bool {
    entries
        .iter()
        .any(|e| Some(e.id.as_str()) != except && same_name(&e.name, name))
}

/// Nom libre dérivé de `title` : titre nettoyé et borné, suffixe « (2) », « (3) »… si pris.
fn unique_name(title: &str, entries: &[SavedComposition]) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let base: String = cleaned.trim().chars().take(MAX_NAME_CHARS).collect();
    let base = match base.trim() {
        "" => FALLBACK_NAME.to_string(),
        b => b.to_string(),
    };
    if !name_taken(entries, &base, None) {
        return base;
    }
    // Au plus `MAX_COMPOSITIONS` noms pris : un suffixe libre existe parmi les suivants.
    for n in 2.. {
        let suffix = format!(" ({n})");
        let keep = MAX_NAME_CHARS - suffix.chars().count();
        let stem: String = base.chars().take(keep).collect();
        let candidate = format!("{}{suffix}", stem.trim_end());
        if !name_taken(entries, &candidate, None) {
            return candidate;
        }
    }
    unreachable!("suffixe de nom libre introuvable")
}

/// Copie `path` en `<nom>.<horodatage>.bak` (suffixe numérique si déjà pris), sans le
/// toucher.
fn backup_copy(path: &Path) -> io::Result<PathBuf> {
    let stamp = storage::utc_stamp(now_secs());
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("chemin sans nom de fichier"))?
        .to_string_lossy()
        .into_owned();
    let mut backup = path.with_file_name(format!("{name}.{stamp}.bak"));
    let mut n = 1;
    while backup.exists() {
        backup = path.with_file_name(format!("{name}.{stamp}-{n}.bak"));
        n += 1;
    }
    fs::copy(path, &backup)?;
    Ok(backup)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

impl<'a> Store<'a> {
    pub fn new(dir: &'a Path) -> Self {
        Self::with_writer(dir, storage::write_atomic)
    }

    pub fn with_writer(dir: &'a Path, write: Writer) -> Self {
        Self { dir, write }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(COMPOSITIONS_FILE)
    }

    fn set_aside(&self, why: &str) -> AppResult<StoreRead> {
        let backup = storage::backup_corrupted(&self.path())?;
        tracing::warn!(
            "compositions de pack : fichier {why}, mis de côté en {}",
            backup.display()
        );
        Ok(StoreRead {
            entries: Vec::new(),
            notice: Some(format!(
                "Le fichier des compositions enregistrées ({why}) a été mis de côté ({}).",
                file_name(&backup)
            )),
        })
    }

    /// Lit les compositions. Absent : liste vide. Illisible, trop gros ou d'un schéma plus
    /// récent : mis de côté, liste vide et message. Entrées illisibles : fichier copié puis
    /// réécrit avec les entrées lisibles, message. Erreur seulement si le disque est
    /// inaccessible.
    pub fn read(&self) -> AppResult<StoreRead> {
        let path = self.path();
        match fs::metadata(&path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(StoreRead::default()),
            Err(err) => return Err(err.into()),
            Ok(meta) if meta.len() > MAX_FILE_BYTES => return self.set_aside("trop volumineux"),
            Ok(_) => {}
        }
        let raw = match storage::load_versioned::<RawStore>(&path, COMPOSITIONS_SCHEMA, migrate) {
            Ok(Loaded::Missing) => return Ok(StoreRead::default()),
            Ok(Loaded::Found(raw)) => raw,
            Ok(Loaded::Recovered { backup }) => {
                return Ok(StoreRead {
                    entries: Vec::new(),
                    notice: Some(format!(
                        "Le fichier des compositions enregistrées était illisible : il a été mis de côté ({backup})."
                    )),
                })
            }
            Err(AppError::ConfigVersion { .. }) => {
                return self.set_aside("écrit par une version plus récente")
            }
            Err(err) => return Err(err),
        };
        let mut entries: Vec<SavedComposition> = Vec::new();
        let mut ids = HashSet::new();
        let mut rejected = 0usize;
        for value in raw.compositions {
            let ok = serde_json::from_value::<SavedComposition>(value)
                .ok()
                .filter(|e| {
                    valid_id(&e.id)
                        && checked_name(&e.name) == Some(e.name.as_str())
                        && !name_taken(&entries, &e.name, None)
                        && !ids.contains(&e.id)
                        && entries.len() < MAX_COMPOSITIONS
                });
            match ok {
                Some(entry) => {
                    ids.insert(entry.id.clone());
                    entries.push(entry);
                }
                None => rejected += 1,
            }
        }
        if rejected == 0 {
            return Ok(StoreRead {
                entries,
                notice: None,
            });
        }
        // Copie intégrale d'abord : la réécriture ne perd rien.
        let backup = backup_copy(&path)?;
        tracing::warn!(
            "compositions de pack : {rejected} entrée(s) illisible(s) écartée(s), fichier copié en {}",
            backup.display()
        );
        let notice = format!(
            "{rejected} composition(s) enregistrée(s) illisible(s) écartée(s) ; le fichier d'origine a été copié ({}).",
            file_name(&backup)
        );
        if let Err(err) = self.write(&entries) {
            // Les entrées lisibles restent servies ; la réécriture sera retentée.
            tracing::warn!("compositions de pack : réécriture impossible : {err}");
        }
        Ok(StoreRead {
            entries,
            notice: Some(notice),
        })
    }

    /// Écrit les entrées de façon atomique, sérialisées une fois et bornées.
    fn write(&self, entries: &[SavedComposition]) -> AppResult<()> {
        let mut bytes = serde_json::to_vec_pretty(&StoreFile {
            schema_version: COMPOSITIONS_SCHEMA,
            compositions: entries,
        })?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(AppError::Validation(format!(
                "fichier des compositions trop volumineux : {} octets (au plus {MAX_FILE_BYTES})",
                bytes.len()
            )));
        }
        (self.write)(&self.path(), &bytes)?;
        Ok(())
    }

    /// Conserve `composition`, créée sous le titre effectif `title`, à la date `now`. Identique
    /// à une entrée existante : cette entrée est mise à jour. Sinon : nouvelle entrée au nom
    /// libre dérivé du titre. Refusé au-delà de [`MAX_COMPOSITIONS`] ou de
    /// [`MAX_ENTRY_BYTES`].
    pub fn record(
        &self,
        composition: &PackComposition,
        title: &str,
        now: u64,
    ) -> AppResult<Recorded> {
        let StoreRead {
            mut entries,
            notice,
        } = self.read()?;
        let (id, name, new) = if let Some(entry) =
            entries.iter_mut().find(|e| e.composition == *composition)
        {
            entry.last_used_at = now;
            entry.packs = entry.packs.saturating_add(1);
            (entry.id.clone(), entry.name.clone(), false)
        } else {
            if entries.len() >= MAX_COMPOSITIONS {
                return Err(AppError::Validation(format!(
                        "{MAX_COMPOSITIONS} compositions enregistrées au plus : supprimez-en une pour conserver les suivantes"
                    )));
            }
            let size = serde_json::to_vec(composition)?.len();
            if size > MAX_ENTRY_BYTES {
                return Err(AppError::Validation(format!(
                    "composition trop volumineuse : {size} octets (au plus {MAX_ENTRY_BYTES})"
                )));
            }
            let entry = SavedComposition {
                id: uuid::Uuid::new_v4().to_string(),
                name: unique_name(title, &entries),
                created_at: now,
                last_used_at: now,
                packs: 1,
                composition: composition.clone(),
            };
            let out = (entry.id.clone(), entry.name.clone(), true);
            entries.push(entry);
            out
        };
        self.write(&entries)?;
        Ok(Recorded {
            id,
            name,
            new,
            notice,
        })
    }

    fn find<'e>(entries: &'e [SavedComposition], id: &str) -> AppResult<&'e SavedComposition> {
        entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| AppError::NotFound("composition introuvable".into()))
    }

    /// Renomme une composition (nom nettoyé, unique sans tenir compte de la casse).
    pub fn rename(&self, id: &str, name: &str) -> AppResult<()> {
        let Some(name) = checked_name(name) else {
            return Err(AppError::Validation(format!(
                "nom de composition vide ou trop long (1 à {MAX_NAME_CHARS} caractères)"
            )));
        };
        let mut entries = self.read()?.entries;
        Self::find(&entries, id)?;
        if name_taken(&entries, name, Some(id)) {
            return Err(AppError::Validation(
                "une autre composition porte déjà ce nom".into(),
            ));
        }
        if let Some(entry) = entries.iter_mut().find(|e| e.id == id) {
            if entry.name == name {
                return Ok(());
            }
            entry.name = name.to_string();
        }
        self.write(&entries)
    }

    /// Supprime une composition ; absente : rien à faire.
    pub fn delete(&self, id: &str) -> AppResult<()> {
        let mut entries = self.read()?.entries;
        let before = entries.len();
        entries.retain(|e| e.id != id);
        if entries.len() == before {
            return Ok(());
        }
        self.write(&entries)
    }

    /// Copie une composition dans le brouillon unique, à la révision qui suit celle
    /// enregistrée : une sauvegarde automatique en retard ne peut pas l'écraser.
    pub fn open(&self, id: &str) -> AppResult<SaveOutcome> {
        let entries = self.read()?.entries;
        let entry = Self::find(&entries, id)?;
        let stored = pack::read_draft(self.dir)?.revision;
        let revision = stored
            .checked_add(1)
            .filter(|r| *r <= MAX_REVISION)
            .ok_or_else(|| AppError::Validation("révision de brouillon épuisée".into()))?;
        pack::save_draft(self.dir, &entry.composition, revision)
    }
}

/// Résultat de [`Store::record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub id: String,
    pub name: String,
    /// `false` : composition identique à une entrée existante (pack refait).
    pub new: bool,
    /// Message de la lecture (fichier mis de côté, entrées écartées).
    pub notice: Option<String>,
}

/// Conservation après `pack_create`, exposée au front (miroir de `CompositionRecord` dans
/// `src/api/install.ts`). Jamais une erreur : un échec est rapporté dans `error`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CompositionRecord {
    /// Nom sous lequel la composition est conservée ; `None` : non conservée (voir `error`).
    pub name: Option<String>,
    /// `false` : composition déjà conservée (pack refait).
    pub new: bool,
    /// Raison de la non-conservation, en français.
    pub error: Option<String>,
    /// Message de la lecture du fichier (mis de côté, entrées écartées).
    pub notice: Option<String>,
}

/// Conserve la composition d'un pack dont la tâche vient d'être créée. L'appelant tient le
/// verrou de configuration. Un échec ne remonte pas : la création du pack continue.
pub fn record_after_create(
    dir: &Path,
    composition: &PackComposition,
    title: &str,
) -> CompositionRecord {
    match Store::new(dir).record(composition, title, now_secs()) {
        Ok(r) => CompositionRecord {
            name: Some(r.name),
            new: r.new,
            error: None,
            notice: r.notice,
        },
        Err(err) => {
            tracing::warn!("composition de pack non conservée : {err}");
            CompositionRecord {
                error: Some(err.to_string()),
                ..CompositionRecord::default()
            }
        }
    }
}

/// Résumé d'une composition pour la liste (miroir de `CompositionSummary` dans
/// `src/api/pack.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompositionSummary {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub last_used_at: u64,
    pub packs: u32,
    /// Épisodes de l'arborescence.
    pub episodes: usize,
    /// Flux cités (épisodes et couverture).
    pub feeds: usize,
    /// Flux cités dont la source a été supprimée (composition gardée, § 14.5).
    pub missing_feeds: usize,
}

/// Liste des compositions (miroir de `CompositionsView` dans `src/api/pack.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompositionsView {
    /// Dernière utilisée d'abord.
    pub compositions: Vec<CompositionSummary>,
    pub notice: Option<String>,
    pub max: usize,
}

/// Vue de la liste ; `podcasts` : identifiants des sources podcast existantes.
pub fn view(read: StoreRead, podcasts: &HashSet<String>) -> CompositionsView {
    let mut compositions: Vec<CompositionSummary> = read
        .entries
        .into_iter()
        .map(|e| {
            let sources: BTreeSet<String> = e.composition.source_ids();
            CompositionSummary {
                episodes: e.composition.episodes().len(),
                feeds: sources.len(),
                missing_feeds: sources.iter().filter(|s| !podcasts.contains(*s)).count(),
                id: e.id,
                name: e.name,
                created_at: e.created_at,
                last_used_at: e.last_used_at,
                packs: e.packs,
            }
        })
        .collect();
    compositions.sort_by(|a, b| {
        b.last_used_at
            .cmp(&a.last_used_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    CompositionsView {
        compositions,
        notice: read.notice,
        max: MAX_COMPOSITIONS,
    }
}

// ---------------------------------------------------------------------------------------
// Commandes Tauri
// ---------------------------------------------------------------------------------------

/// Liste les compositions conservées, avec les flux supprimés de chacune.
#[tauri::command]
pub async fn pack_compositions_list(config: State<'_, ConfigState>) -> AppResult<CompositionsView> {
    let _guard = config.guard();
    let read = Store::new(&config.dir).read()?;
    let podcasts: HashSet<String> = SourcesStore::new(&config.dir)
        .list()?
        .into_iter()
        .filter_map(|s| match s {
            Source::Podcast { id, .. } => Some(id),
            _ => None,
        })
        .collect();
    Ok(view(read, &podcasts))
}

/// Renomme une composition.
#[tauri::command]
pub async fn pack_composition_rename(
    config: State<'_, ConfigState>,
    id: String,
    name: String,
) -> AppResult<()> {
    let _guard = config.guard();
    Store::new(&config.dir).rename(&id, &name)
}

/// Supprime une composition (les histoires créées ne sont pas touchées).
#[tauri::command]
pub async fn pack_composition_delete(config: State<'_, ConfigState>, id: String) -> AppResult<()> {
    let _guard = config.guard();
    Store::new(&config.dir).delete(&id)
}

/// Copie une composition dans le brouillon (le brouillon en cours est remplacé) ; le front
/// ouvre ensuite l'éditeur, qui relit le brouillon.
#[tauri::command]
pub async fn pack_composition_open(
    config: State<'_, ConfigState>,
    id: String,
) -> AppResult<SaveOutcome> {
    let _guard = config.guard();
    Store::new(&config.dir).open(&id)
}

#[cfg(test)]
mod tests;
