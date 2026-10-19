//! Réordonnancement des histoires de la Flam (#233) : réécriture de l'index `list` dans l'ordre
//! choisi par l'utilisateur. L'ordre d'affichage sur la Flam est celui de `list`.
//!
//! # Invariants
//!
//! - L'ordre demandé est **l'ordre complet** : exactement les UUID de `list`, chacun une fois.
//!   Toute différence (UUID invalide, en double, inconnu, masqué, absent de la demande, liste
//!   vide) est refusée **avant toute écriture**. Un index modifié entre l'affichage et la demande
//!   (envoi, retrait) est donc refusé, jamais fusionné.
//! - Seul `etc/library/list` est réécrit, par [`write_index`] (fichier neuf, sauvegarde,
//!   remplacement, relecture), ainsi que ses fichiers de travail éphémères ; `usr/0/library.cache`
//!   est supprimé par cette même écriture pour que le firmware reconstruise sa bibliothèque.
//!   `list.hidden`, `str/`, `str.hidden/`, `usr/0/` (sauvegardes) et les réglages ne sont jamais
//!   touchés ; aucun dossier n'est déplacé.
//! - Un index absent, abîmé ou illisible (`list`), ou un `list.hidden` abîmé ou illisible, n'est
//!   jamais réécrit : l'appartenance des histoires n'y est pas sûre.
//! - Le verrou d'appareil ([`Operation::Reorder`]) est tenu de la reprise à la fin de l'écriture ;
//!   une autre opération en cours fait refuser la demande sans rien lire ni écrire.
//!
//! # Coupure ou échec à chaque étape
//!
//! | Coupure ou échec après…              | État laissé                               | Reprise                    |
//! |--------------------------------------|-------------------------------------------|----------------------------|
//! | la validation                        | rien de modifié                           | aucune                     |
//! | `list.new` (complet ou partiel)      | ancien `list`, `list.new` sans `list.bak` | `list.new` supprimé : ancien ordre |
//! | `list.bak` (complet ou partiel)      | ancien `list`, `list.new`, `list.bak`     | ancien ordre gardé |
//! | le renommage, cible perdue           | `list` absent, `list.new`, `list.bak`     | `list.new` promu : nouvel ordre |
//! | le remplacement, la relecture, le cache | nouveau `list`, `list.bak`             | nouvel ordre, cache supprimé |
//!
//! L'index est donc toujours l'ancien ou le nouvel ordre complet, jamais vide ni tronqué. La
//! reprise des index ([`recover`]) est lancée au début de chaque opération et au branchement
//! (`device_recover_transfers`).

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::Path;

use serde::Serialize;

use super::detect::MDF_FILE;
use super::guard::{check_write, WriteContext, WriteKind, WriteRefused};
use super::index::{
    read_index, recover, write_index, Consent, IndexError, IndexKind, IndexState, Recovery,
    StoryId, WriteOutcome,
};
use super::lock::{Busy, DeviceLock, Operation, DEVICE_LOCK};
use crate::catalog::blocking;
use crate::{AppError, AppResult};

/// Résultat d'un réordonnancement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReorderReport {
    /// L'index a été réécrit (faux : l'ordre demandé était déjà celui de la Flam).
    pub changed: bool,
    /// `library.cache` est supprimé (faux si rien n'a changé, ou suppression à refaire : la
    /// reprise suivante s'en charge).
    pub cache_removed: bool,
    /// Nombre d'histoires de l'index.
    pub count: usize,
}

/// Erreur d'un réordonnancement. Aucune écriture n'a eu lieu, sauf pour [`ReorderError::Index`]
/// (l'index est alors l'ancien ou le nouvel ordre, voir le module).
#[derive(Debug, thiserror::Error)]
pub enum ReorderError {
    #[error("ce dossier n'est pas une Flam (fichier .mdf absent) : rien n'a été écrit")]
    NotADevice,
    #[error("ordre demandé vide : rien n'a été écrit")]
    Empty,
    #[error("identifiant d'histoire invalide en position {0} : rien n'a été écrit")]
    InvalidId(usize),
    #[error("l'histoire {} figure deux fois dans l'ordre demandé : rien n'a été écrit", short(.0))]
    Duplicate(StoryId),
    #[error("l'histoire {} n'est pas dans l'index de la Flam : actualisez la liste ; rien n'a été écrit", short(.0))]
    Unknown(StoryId),
    #[error("l'histoire {} est masquée : son ordre ne se change pas ; rien n'a été écrit", short(.0))]
    Hidden(StoryId),
    #[error(
        "l'histoire {} manque dans l'ordre demandé : actualisez la liste ; rien n'a été écrit",
        short(.0)
    )]
    Missing(StoryId),
    #[error(transparent)]
    Busy(#[from] Busy),
    /// Refus de la garde commune des écritures ([`super::guard`]) : rien n'a été écrit.
    #[error(transparent)]
    Refused(#[from] WriteRefused),
    #[error("index de la Flam absent, abîmé ou illisible : réparez-le avant de changer l'ordre ; rien n'a été écrit")]
    IndexNeedsUser,
    #[error(
        "index de la Flam : {0} ; l'ordre affiché est celui relu sur la Flam, actualisez la liste"
    )]
    Index(#[from] IndexError),
}

/// Identifiant court d'une histoire pour les messages (8 premiers caractères de l'UUID).
fn short(id: &StoryId) -> &str {
    &id.as_str()[..8]
}

impl From<ReorderError> for AppError {
    fn from(err: ReorderError) -> Self {
        match err {
            ReorderError::Index(_) => AppError::Io(io::Error::other(err.to_string())),
            _ => AppError::Validation(err.to_string()),
        }
    }
}

/// Écriture de `list`, injectable pour les tests de coupure.
type WriteList<'a> = dyn FnMut(&Path, &[StoryId]) -> Result<WriteOutcome, IndexError> + 'a;

/// Racine d'un appareil : chemin absolu, `.mdf` présent (fichier, pas un lien).
fn check_device(root: &Path) -> Result<(), ReorderError> {
    if !root.is_absolute() {
        return Err(ReorderError::NotADevice);
    }
    match fs::symlink_metadata(root.join(MDF_FILE)) {
        Ok(meta) if meta.is_file() => Ok(()),
        _ => Err(ReorderError::NotADevice),
    }
}

/// Analyse l'ordre demandé : non vide, UUID valides (forme canonique), sans doublon. Ne lit
/// rien sur l'appareil.
fn parse_order(order: &[String]) -> Result<Vec<StoryId>, ReorderError> {
    if order.is_empty() {
        return Err(ReorderError::Empty);
    }
    let mut seen = HashSet::with_capacity(order.len());
    let mut ids = Vec::with_capacity(order.len());
    for (position, text) in order.iter().enumerate() {
        let id = StoryId::parse(text).ok_or(ReorderError::InvalidId(position + 1))?;
        if !seen.insert(id.clone()) {
            return Err(ReorderError::Duplicate(id));
        }
        ids.push(id);
    }
    Ok(ids)
}

/// Compare l'ordre demandé (sans doublon) à l'index actuel : mêmes UUID, ni plus ni moins.
fn check_same_set(
    order: &[StoryId],
    current: &[StoryId],
    hidden: &[StoryId],
) -> Result<(), ReorderError> {
    let known: HashSet<&StoryId> = current.iter().collect();
    if let Some(extra) = order.iter().find(|id| !known.contains(id)) {
        return Err(if hidden.contains(extra) {
            ReorderError::Hidden(extra.clone())
        } else {
            ReorderError::Unknown(extra.clone())
        });
    }
    let asked: HashSet<&StoryId> = order.iter().collect();
    if let Some(lost) = current.iter().find(|id| !asked.contains(id)) {
        return Err(ReorderError::Missing(lost.clone()));
    }
    Ok(())
}

/// Réécrit `list` de la Flam montée en `root` dans l'ordre `order` (liste complète des UUID de
/// `list`, dans l'ordre voulu). Refuse sans rien écrire si l'ordre ne correspond pas exactement
/// à l'index, si un index n'est pas sûr, si une autre opération est en cours ou si la garde du
/// firmware refuse ([`check_write`]).
pub fn reorder_stories(
    root: &Path,
    lock: &DeviceLock,
    order: &[String],
    ctx: &WriteContext<'_>,
) -> Result<ReorderReport, ReorderError> {
    reorder_with(root, lock, order, ctx, &mut |root, entries| {
        write_index(root, IndexKind::Visible, entries, Consent::IfValid)
    })
}

/// Corps de [`reorder_stories`] ; `write` remplace `list` (injectable pour les tests).
fn reorder_with(
    root: &Path,
    lock: &DeviceLock,
    order: &[String],
    ctx: &WriteContext<'_>,
    write: &mut WriteList<'_>,
) -> Result<ReorderReport, ReorderError> {
    let order = parse_order(order)?;
    check_device(root)?;
    let _guard = lock.try_begin(Operation::Reorder)?;
    // Garde commune, sous le verrou et avant toute écriture (reprise d'index comprise).
    check_write(root, ctx, WriteKind::Other)?;

    // Reprise d'une écriture d'index interrompue, puis lecture des deux index.
    for kind in IndexKind::ALL {
        if recover(root, kind)? == Recovery::NeedsUser {
            return Err(ReorderError::IndexNeedsUser);
        }
    }
    let list = read_index(root, IndexKind::Visible);
    if list.state != IndexState::Valid {
        return Err(ReorderError::IndexNeedsUser);
    }
    let hidden = read_index(root, IndexKind::Hidden);
    if !matches!(hidden.state, IndexState::Valid | IndexState::Absent) {
        return Err(ReorderError::IndexNeedsUser);
    }
    check_same_set(&order, &list.entries, &hidden.entries)?;

    let count = order.len();
    if order == list.entries {
        return Ok(ReorderReport {
            changed: false,
            cache_removed: false,
            count,
        });
    }
    let (changed, cache_removed) = match write(root, &order) {
        Ok(WriteOutcome::Written) => (true, true),
        Ok(WriteOutcome::Unchanged) => (false, false),
        // Index réécrit et vérifié ; le cache et la fin de transaction sont repris plus tard.
        Err(IndexError::CacheNotRemoved(err)) => {
            tracing::warn!("ordre écrit, library.cache non supprimé : {err}");
            (true, false)
        }
        Err(IndexError::NeedsConsent) => return Err(ReorderError::IndexNeedsUser),
        Err(err) => return Err(err.into()),
    };
    tracing::info!(
        histoires = count,
        change = changed,
        "ordre des histoires de la Flam"
    );
    Ok(ReorderReport {
        changed,
        cache_removed,
        count,
    })
}

/// Réordonne les histoires de la Flam montée en `root`. `order` : tous les UUID de l'index
/// `list`, dans l'ordre voulu (voir [`reorder_stories`]).
#[tauri::command]
pub async fn device_reorder_stories(
    config: tauri::State<'_, crate::storage::ConfigState>,
    root: String,
    order: Vec<String>,
) -> AppResult<ReorderReport> {
    let cache_root = super::transfer::launcher_cache_root(&config);
    blocking(move || -> AppResult<ReorderReport> {
        // Racine reçue de l'interface : contrôlée par la garde (lecteur détecté seulement).
        let ctx = WriteContext::current(cache_root);
        Ok(reorder_stories(
            Path::new(&root),
            &DEVICE_LOCK,
            &order,
            &ctx,
        )?)
    })
    .await?
}

#[cfg(test)]
mod tests;
