//! Retrait d'une histoire de la Flam (#231) : l'index d'abord, le dossier ensuite.
//!
//! # Invariants
//!
//! - Aucun index ne pointe vers un dossier supprimé : le dossier n'est touché qu'après la
//!   réécriture de l'index ([`super::index::write_index`]) **et** une relecture qui confirme
//!   l'absence de l'UUID dans `list` et `list.hidden`.
//! - Aucun chemin n'est construit à partir d'une entrée non validée : l'UUID passe par
//!   [`StoryId::parse`] (36 caractères `[0-9a-f-]`, ni séparateur ni `..`) avant tout accès.
//! - Seuls `str/<uuid>/` et `str.hidden/<uuid>/` de cet UUID sont déplacés ; le dossier d'une
//!   autre histoire, les sauvegardes `usr/0/` et les réglages ne sont jamais touchés.
//! - Sans confirmation de l'utilisateur, rien n'est lu ni écrit.
//!
//! # Suppression reprenable
//!
//! Supprimer récursivement un dossier sur FAT prend du temps et peut être interrompu : il
//! resterait un dossier d'histoire à moitié vide dans `str/`. Le dossier est donc d'abord
//! renommé dans la corbeille de l'appareil ([`TRASH_DIR`], à la racine, hors de `str/` que le
//! firmware énumère), sous un nom unique, puis supprimé. Un renommage dans le même volume est
//! une seule mise à jour d'entrées de répertoire : après une coupure, le dossier est soit
//! encore entier dans `str/` (orphelin signalé par [`find_orphans`], jamais supprimé sans
//! confirmation), soit dans la corbeille, vidée par [`purge_trash`] à la prochaine opération.
//!
//! | Coupure ou échec après…        | État laissé                                    | Reprise                                      |
//! |--------------------------------|------------------------------------------------|----------------------------------------------|
//! | la validation                  | rien de modifié                                | aucune                                       |
//! | une étape de l'index           | voir `index` : ancien ou nouvel index          | `index::recover`                             |
//! | l'index de `list` (UUID aussi dans `list.hidden`) | UUID encore dans `list.hidden`, dossier intact | nouveau retrait                  |
//! | l'index réécrit                | dossier entier dans `str/`, absent de l'index  | orphelin signalé, retrait sur confirmation   |
//! | le renommage en corbeille      | dossier dans `.flam-trash/`                     | [`purge_trash`] au début de chaque opération |
//! | une suppression partielle      | restes dans `.flam-trash/`                      | [`purge_trash`]                              |
//!
//! Les opérations sur l'appareil sont sérialisées par [`DeviceLock`].

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::detect::MDF_FILE;
use super::index::{
    read_index, recover, write_index, Consent, IndexError, IndexKind, IndexRead, IndexState,
    Recovery, StoryId, STORIES_DIR,
};
use super::lock::{Busy, DeviceLock, Operation, DEVICE_LOCK};
use crate::catalog::blocking;
use crate::{AppError, AppResult};

/// Dossier des histoires masquées, relatif à la racine.
pub const HIDDEN_STORIES_DIR: &str = "str.hidden";
/// Corbeille du launcher sur l'appareil, relative à la racine. Hors de `str/` : le firmware
/// énumère `str/` et ne doit pas y voir un dossier en cours de suppression.
pub const TRASH_DIR: &str = ".flam-trash";

/// Résultat d'un retrait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RemoveReport {
    /// UUID retiré (forme canonique).
    pub id: String,
    /// L'UUID figurait dans un index, qui a été réécrit (faux : dossier orphelin supprimé).
    pub index_updated: bool,
    /// Nombre de dossiers d'histoire retirés (`str/` et `str.hidden/`).
    pub folders_removed: usize,
    /// La corbeille n'a pas pu être vidée tout de suite (fichier verrouillé) : l'histoire est
    /// retirée, l'espace sera libéré à la prochaine opération.
    pub cleanup_pending: bool,
}

/// Dossier d'histoire présent sur l'appareil mais absent des deux index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Orphan {
    pub id: String,
    /// Dans `str.hidden/` plutôt que `str/`.
    pub hidden: bool,
}

/// Erreur d'un retrait.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("retrait non confirmé : aucune suppression")]
    NotConfirmed,
    #[error("identifiant d'histoire invalide : aucune suppression")]
    InvalidId,
    #[error("ce dossier n'est pas une Flam (fichier .mdf absent) : aucune suppression")]
    NotADevice,
    #[error("histoire {0} introuvable sur la Flam")]
    NotFound(StoryId),
    #[error(transparent)]
    Busy(#[from] Busy),
    #[error("index de la Flam abîmé ou illisible : réparez-le avant de retirer une histoire")]
    IndexNeedsUser,
    #[error("l'histoire {0} figure encore dans l'index : son dossier n'est pas supprimé")]
    StillIndexed(StoryId),
    #[error("index de la Flam : {0}")]
    Index(#[from] IndexError),
    #[error("histoire retirée de l'index, mais son dossier n'a pas pu être supprimé ({0}) : il apparaîtra comme dossier orphelin")]
    FolderLeft(#[source] io::Error),
    #[error("{0}")]
    Io(#[from] io::Error),
}

impl From<RemoveError> for AppError {
    fn from(err: RemoveError) -> Self {
        match err {
            RemoveError::NotFound(_) => AppError::NotFound(err.to_string()),
            RemoveError::NotConfirmed
            | RemoveError::InvalidId
            | RemoveError::NotADevice
            | RemoveError::Busy(_)
            | RemoveError::IndexNeedsUser
            | RemoveError::StillIndexed(_) => AppError::Validation(err.to_string()),
            RemoveError::Index(_) | RemoveError::FolderLeft(_) | RemoveError::Io(_) => {
                AppError::Io(io::Error::other(err.to_string()))
            }
        }
    }
}

/// Étapes après lesquelles un test peut simuler une coupure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    IndexWritten(IndexKind),
    MovedToTrash,
    Purged,
}

/// Racine d'un appareil : `.mdf` présent (fichier, pas un lien). Garde-fou contre une racine
/// arbitraire transmise par l'interface.
fn check_device(root: &Path) -> Result<(), RemoveError> {
    match fs::symlink_metadata(root.join(MDF_FILE)) {
        Ok(meta) if meta.is_file() => Ok(()),
        _ => Err(RemoveError::NotADevice),
    }
}

/// Vrai dossier (ni fichier ni lien ou jonction).
fn is_real_dir(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.is_dir() && !meta.file_type().is_symlink()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err),
    }
}

/// Dossiers possibles d'une histoire.
fn story_dirs(root: &Path, id: &StoryId) -> [(PathBuf, bool); 2] {
    [
        (root.join(STORIES_DIR).join(id.as_str()), false),
        (root.join(HIDDEN_STORIES_DIR).join(id.as_str()), true),
    ]
}

/// Lit les deux index après reprise. Refuse si une reprise demande l'utilisateur ou si un
/// index est abîmé ou illisible : l'appartenance d'un UUID n'y est pas sûre.
fn read_indexes(root: &Path) -> Result<[IndexRead; 2], RemoveError> {
    for kind in IndexKind::ALL {
        if recover(root, kind)? == Recovery::NeedsUser {
            return Err(RemoveError::IndexNeedsUser);
        }
    }
    let reads = IndexKind::ALL.map(|kind| read_index(root, kind));
    for read in &reads {
        if !matches!(read.state, IndexState::Valid | IndexState::Absent) {
            return Err(RemoveError::IndexNeedsUser);
        }
    }
    Ok(reads)
}

/// Vide la corbeille de l'appareil (restes d'un retrait interrompu). Rend le nombre d'entrées
/// supprimées ; continue après une entrée verrouillée et rend la première erreur à la fin.
/// Ne suit aucun lien : une corbeille qui n'est pas un vrai dossier est laissée telle quelle.
pub fn purge_trash(root: &Path) -> io::Result<usize> {
    let trash = root.join(TRASH_DIR);
    if !is_real_dir(&trash)? {
        return Ok(0);
    }
    let mut removed = 0;
    let mut first_error = None;
    for entry in fs::read_dir(&trash)? {
        let result = entry.and_then(|entry| {
            let path = entry.path();
            if is_real_dir(&path)? {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            }
        });
        match result {
            Ok(()) => removed += 1,
            Err(err) => {
                tracing::warn!("corbeille de la Flam : suppression impossible : {err}");
                first_error.get_or_insert(err);
            }
        }
    }
    if let Some(err) = first_error {
        return Err(err);
    }
    // Corbeille vide : le dossier lui-même est retiré (sans erreur s'il reste occupé).
    let _ = fs::remove_dir(&trash);
    Ok(removed)
}

/// Dossiers d'histoire (nom au format UUID) présents dans `str/` ou `str.hidden/` mais
/// absents des deux index. Signalés à l'utilisateur, jamais supprimés sans confirmation.
pub fn find_orphans(root: &Path) -> Result<Vec<Orphan>, RemoveError> {
    let reads = read_indexes(root)?;
    let indexed = |id: &StoryId| reads.iter().any(|read| read.entries.contains(id));
    let mut orphans = Vec::new();
    for (dir, hidden) in [(STORIES_DIR, false), (HIDDEN_STORIES_DIR, true)] {
        let entries = match fs::read_dir(root.join(dir)) {
            Ok(entries) => entries,
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        for entry in entries {
            let entry = entry?;
            let Some(id) = entry.file_name().to_str().and_then(StoryId::parse) else {
                continue;
            };
            if is_real_dir(&entry.path())? && !indexed(&id) {
                orphans.push(Orphan {
                    id: id.to_string(),
                    hidden,
                });
            }
        }
    }
    orphans.sort_by(|a, b| (&a.id, a.hidden).cmp(&(&b.id, b.hidden)));
    Ok(orphans)
}

/// Retire l'histoire `id` de la Flam montée en `root` : index d'abord, dossier ensuite (voir
/// le module). Une histoire absente des index mais dont le dossier existe (orphelin) est
/// supprimée sans toucher aux index. Refuse sans `confirmed`, si l'UUID est invalide, si
/// l'histoire n'existe pas ou si une autre opération est en cours sur l'appareil.
pub fn remove_story(
    root: &Path,
    lock: &DeviceLock,
    id: &str,
    confirmed: bool,
) -> Result<RemoveReport, RemoveError> {
    remove_with(root, lock, id, confirmed, &mut |_| Ok(()))
}

/// Corps de [`remove_story`]. `hook` est appelé après chaque étape ; une erreur rendue par
/// `hook` interrompt le retrait sans aucun nettoyage, comme une coupure.
fn remove_with(
    root: &Path,
    lock: &DeviceLock,
    id: &str,
    confirmed: bool,
    hook: &mut dyn FnMut(Step) -> io::Result<()>,
) -> Result<RemoveReport, RemoveError> {
    if !confirmed {
        return Err(RemoveError::NotConfirmed);
    }
    let id = StoryId::parse(id).ok_or(RemoveError::InvalidId)?;
    check_device(root)?;
    let _guard = lock.try_begin(Operation::Remove)?;

    if let Err(err) = purge_trash(root) {
        tracing::warn!("restes d'un retrait précédent non supprimés : {err}");
    }

    let reads = read_indexes(root)?;
    let indexed: Vec<IndexKind> = IndexKind::ALL
        .into_iter()
        .zip(&reads)
        .filter(|(_, read)| read.entries.contains(&id))
        .map(|(kind, _)| kind)
        .collect();
    let mut folders = Vec::new();
    for (dir, _) in story_dirs(root, &id) {
        if is_real_dir(&dir)? {
            folders.push(dir);
        }
    }
    if indexed.is_empty() && folders.is_empty() {
        return Err(RemoveError::NotFound(id));
    }

    // 1. Index : l'UUID est retiré de chaque index qui le contient.
    for (kind, read) in IndexKind::ALL.into_iter().zip(&reads) {
        if !indexed.contains(&kind) {
            continue;
        }
        let rest: Vec<StoryId> = read.entries.iter().filter(|e| **e != id).cloned().collect();
        match write_index(root, kind, &rest, Consent::IfValid) {
            Ok(_) => {}
            Err(IndexError::NeedsConsent) => return Err(RemoveError::IndexNeedsUser),
            Err(err) => return Err(err.into()),
        }
        hook(Step::IndexWritten(kind))?;
    }

    // 2. Relecture : aucun dossier n'est touché tant que l'UUID est lisible dans un index.
    for kind in IndexKind::ALL {
        let read = read_index(root, kind);
        if !matches!(read.state, IndexState::Valid | IndexState::Absent)
            || read.entries.contains(&id)
        {
            return Err(RemoveError::StillIndexed(id));
        }
    }

    // 3. Dossiers déplacés dans la corbeille (renommage dans le même volume).
    let trash = root.join(TRASH_DIR);
    let mut moved = Vec::new();
    for folder in &folders {
        fs::create_dir_all(&trash).map_err(RemoveError::FolderLeft)?;
        let target = trash.join(format!("{id}-{}", uuid::Uuid::new_v4().simple()));
        fs::rename(folder, &target).map_err(RemoveError::FolderLeft)?;
        moved.push(target);
        hook(Step::MovedToTrash)?;
    }

    // 4. Suppression. Un échec n'annule rien : l'histoire est retirée, la corbeille sera
    // vidée à la prochaine opération.
    let mut cleanup_pending = false;
    for target in &moved {
        if let Err(err) = fs::remove_dir_all(target) {
            tracing::warn!("dossier retiré non supprimé, nouvel essai plus tard : {err}");
            cleanup_pending = true;
        }
    }
    if !cleanup_pending {
        let _ = fs::remove_dir(&trash);
    }
    hook(Step::Purged)?;

    Ok(RemoveReport {
        id: id.to_string(),
        index_updated: !indexed.is_empty(),
        folders_removed: moved.len(),
        cleanup_pending,
    })
}

/// Retire une histoire de la Flam montée en `root`. `confirmed` : l'utilisateur a confirmé
/// la suppression dans l'interface (sinon refus, rien n'est touché).
#[tauri::command]
pub async fn device_remove_story(
    root: String,
    id: String,
    confirmed: bool,
) -> AppResult<RemoveReport> {
    blocking(move || remove_story(Path::new(&root), &DEVICE_LOCK, &id, confirmed))
        .await?
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests;
