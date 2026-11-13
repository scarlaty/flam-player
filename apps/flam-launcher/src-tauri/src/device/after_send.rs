//! Retrait de « Mes histoires » après un envoi réussi sur la Flam (#609).
//!
//! L'histoire envoyée reste sur la Flam ; seule sa copie dans la bibliothèque du launcher (dossier
//! de cache) est retirée, par le chemin du bouton « Supprimer »
//! ([`crate::library::remove_story`] : créneau du player, verrou de la bibliothèque, purge de la
//! copie de travail ; la progression de l'enfant est conservée). Pour la renvoyer plus tard, il
//! faut la réinstaller.
//!
//! # Succès d'un envoi
//!
//! [`super::transfer::send_from_library`] ne rend `Ok` qu'après la copie, la relecture de chaque
//! fichier sur la Flam (taille et SHA-256), le renommage en `str/<uuid>/` et l'index à jour ; tout
//! échec, annulation, appareil disparu ou « déjà sur la Flam » est une erreur. En mode carrier
//! (Flam 1.x), un `info` relu illisible (`carrier_readback == Some(false)`) n'est pas un succès :
//! l'interface l'affiche en échec, et rien n'est retiré.
//!
//! # Même génération
//!
//! L'empreinte de l'entrée envoyée ([`SentStamp`] : fichier, taille et date d'installation de
//! l'entrée, taille et date du fichier) est prise **avant** la préparation, revérifiée après
//! l'envoi, puis une dernière fois sous le verrou de la bibliothèque juste avant le retrait. Une
//! histoire réinstallée ou mise à jour entre-temps n'est jamais retirée.
//!
//! # Retrait manuel
//!
//! Un envoi réussi enregistre l'empreinte dans [`SENT`] (mémoire du processus) : « Retirer du
//! PC » ([`library_remove_sent`]) ne retire que ce qui y figure, et seulement si l'entrée n'a pas
//! changé depuis. Il prend le verrou de la Flam : refusé pendant un envoi.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use serde::Serialize;
use tauri::State;

use super::lock::{Operation, DEVICE_LOCK};
use super::transfer::{TransferError, TransferReport};
use crate::catalog::{blocking, CatalogService};
use crate::install::InstallService;
use crate::library::{Library, LibraryEntry};
use crate::player::{PlayerService, SAVES_ROOT};
use crate::settings::{Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::{AppError, AppResult};

/// L'entrée a changé depuis l'envoi : elle n'est pas retirée.
pub const CHANGED_MESSAGE: &str = "l'histoire a changé dans Mes histoires depuis son envoi \
     (réinstallée ou mise à jour) : elle n'a pas été retirée du PC";
/// Installation de la même histoire en cours : pas de retrait.
pub const INSTALLING_MESSAGE: &str = "une installation de cette histoire est en cours : elle \
     n'a pas été retirée du PC";
/// Aucun envoi réussi de cette histoire n'est enregistré.
pub const NOT_SENT_MESSAGE: &str = "aucun envoi réussi de cette histoire n'est connu : pour la \
     retirer du PC, utilisez « Supprimer » dans Mes histoires";

/// Empreinte de l'entrée de bibliothèque envoyée.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SentStamp {
    path: PathBuf,
    file: String,
    size: u64,
    installed_at: u64,
    len: u64,
    modified: Option<SystemTime>,
}

impl SentStamp {
    /// Empreinte de l'histoire `uuid` installée sous `cache_root` ; `None` si elle n'est pas
    /// installée ou si son fichier est illisible (aucun retrait possible).
    pub(crate) fn capture(cache_root: &Path, uuid: &str) -> Option<Self> {
        let (entry, path) = Library::new(cache_root)
            .installed_entry(uuid)
            .ok()
            .flatten()?;
        Self::of(&entry, &path)
    }

    fn of(entry: &LibraryEntry, path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
        Some(Self {
            path: path.to_path_buf(),
            file: entry.file.clone(),
            size: entry.size,
            installed_at: entry.installed_at,
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }

    fn matches(&self, entry: &LibraryEntry, path: Option<&Path>) -> bool {
        path.and_then(|p| Self::of(entry, p))
            .is_some_and(|now| now == *self)
    }
}

/// Envois réussis de la session : histoire de la bibliothèque -> empreinte envoyée.
#[derive(Debug, Default)]
pub(crate) struct SentRegistry(Mutex<BTreeMap<String, SentStamp>>);

impl SentRegistry {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }

    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, SentStamp>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, uuid: &str, stamp: SentStamp) {
        self.map().insert(uuid.to_string(), stamp);
    }

    fn get(&self, uuid: &str) -> Option<SentStamp> {
        self.map().get(uuid).cloned()
    }

    /// Oublie `uuid` s'il est toujours associé à `stamp` (un envoi plus récent est gardé).
    fn forget(&self, uuid: &str, stamp: &SentStamp) {
        let mut map = self.map();
        if map.get(uuid) == Some(stamp) {
            map.remove(uuid);
        }
    }

    /// Un envoi réussi de `uuid` est enregistré.
    #[cfg(test)]
    pub(crate) fn knows(&self, uuid: &str) -> bool {
        self.map().contains_key(uuid)
    }
}

/// Registre du processus.
pub(crate) static SENT: SentRegistry = SentRegistry::new();

/// Copie de l'histoire dans « Mes histoires » après un envoi réussi (champ `library_copy` du
/// rapport d'envoi).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum LibraryCopy {
    /// Gardée ; « Retirer du PC » est possible.
    Kept,
    /// Retirée automatiquement (réglage `remove_after_send`).
    Removed,
    /// Retrait automatique impossible : l'envoi reste réussi, l'histoire reste sur le PC.
    RemoveFailed { message: String },
}

/// Vérification faite sous le verrou de la bibliothèque, avant toute purge : entrée installée
/// et chemin de son fichier.
pub(crate) type Check<'a> = &'a dyn Fn(&LibraryEntry, Option<&Path>) -> AppResult<()>;

/// Retire l'histoire `uuid` envoyée (empreinte de `registry`) par `remove`, qui reçoit la
/// vérification à faire sous le verrou : pas d'installation en cours (`installing`), entrée
/// identique à celle envoyée. L'empreinte est oubliée une fois l'histoire retirée, ou si elle ne
/// peut plus l'être (absente, changée).
pub(crate) fn remove_sent(
    registry: &SentRegistry,
    uuid: &str,
    installing: &dyn Fn() -> bool,
    remove: impl FnOnce(Check<'_>) -> AppResult<()>,
) -> AppResult<()> {
    let stamp = registry
        .get(uuid)
        .ok_or_else(|| AppError::NotFound(NOT_SENT_MESSAGE.into()))?;
    let changed = std::cell::Cell::new(false);
    let check = |entry: &LibraryEntry, path: Option<&Path>| -> AppResult<()> {
        if installing() {
            return Err(AppError::Validation(INSTALLING_MESSAGE.into()));
        }
        if !stamp.matches(entry, path) {
            changed.set(true);
            return Err(AppError::Validation(CHANGED_MESSAGE.into()));
        }
        Ok(())
    };
    let result = remove(&check);
    if result.is_ok() || changed.get() || matches!(result, Err(AppError::NotFound(_))) {
        registry.forget(uuid, &stamp);
    }
    result
}

/// Envoi de l'histoire `uuid` de la bibliothèque de `cache_root` par `send` (qui reçoit le
/// chemin de son fichier), puis suite donnée à sa copie dans « Mes histoires » si l'envoi a
/// réussi ([`settle`]) : retrait si `auto`, sinon « Retirer du PC » possible. Une erreur de
/// `send` est rendue telle quelle, sans rien retirer ni enregistrer. À appeler sous le verrou de
/// la Flam (aucun autre envoi ne doit s'intercaler).
pub(crate) fn send_then_settle(
    registry: &SentRegistry,
    cache_root: &Path,
    uuid: &str,
    auto: bool,
    installing: &dyn Fn() -> bool,
    send: impl FnOnce(&Path) -> AppResult<TransferReport>,
    remove: impl FnOnce(Check<'_>) -> AppResult<()>,
) -> AppResult<TransferReport> {
    // Empreinte prise avant de lire le fichier envoyé.
    let before = SentStamp::capture(cache_root, uuid);
    let (plain_pk, _) = Library::new(cache_root).playable(uuid)?;
    let mut report = send(&plain_pk)?;
    report.library_copy = settle(
        registry, cache_root, uuid, before, &report, auto, installing, remove,
    );
    Ok(report)
}

/// Suite d'un envoi réussi de l'histoire `uuid` de la bibliothèque de `cache_root` : `before`
/// est son empreinte prise avant la préparation. Rend `None` si rien n'est proposé (relecture
/// carrier illisible, empreinte absente, ou entrée changée sans retrait automatique demandé).
/// L'envoi est réussi quoi qu'il arrive : un retrait en échec est rapporté, jamais propagé.
#[allow(clippy::too_many_arguments)]
fn settle(
    registry: &SentRegistry,
    cache_root: &Path,
    uuid: &str,
    before: Option<SentStamp>,
    report: &TransferReport,
    auto: bool,
    installing: &dyn Fn() -> bool,
    remove: impl FnOnce(Check<'_>) -> AppResult<()>,
) -> Option<LibraryCopy> {
    // `info` illisible sur la Flam avec les clés carrier : affiché en échec, rien n'est proposé.
    if report.carrier_readback == Some(false) {
        return None;
    }
    let before = before?;
    if SentStamp::capture(cache_root, uuid).as_ref() != Some(&before) {
        tracing::info!(%uuid, "histoire changée dans la bibliothèque pendant l'envoi : gardée");
        return auto.then(|| LibraryCopy::RemoveFailed {
            message: CHANGED_MESSAGE.into(),
        });
    }
    registry.record(uuid, before);
    if !auto {
        return Some(LibraryCopy::Kept);
    }
    Some(match remove_sent(registry, uuid, installing, remove) {
        Ok(()) => {
            tracing::info!(%uuid, "histoire retirée de la bibliothèque après l'envoi");
            LibraryCopy::Removed
        }
        Err(err) => {
            tracing::warn!(%uuid, "retrait après l'envoi impossible : {err}");
            LibraryCopy::RemoveFailed {
                message: err.to_string(),
            }
        }
    })
}

/// Une installation de `uuid` est en attente ou en cours.
pub(crate) fn installing(installs: &InstallService, uuid: &str) -> bool {
    installs
        .list()
        .iter()
        .any(|task| task.state.is_active() && task.entry.uuid == uuid)
}

/// « Retirer du PC » après un envoi réussi de cette session : retire l'histoire `uuid` de « Mes
/// histoires » (elle reste sur la Flam), par le chemin du bouton « Supprimer », sans toucher à la
/// progression. Refusé (`busy`) pendant une opération sur la Flam, `player_busy` pendant sa
/// lecture, `validation` pendant son installation ou si elle a changé depuis l'envoi,
/// `not_found` si aucun envoi réussi n'est connu.
#[tauri::command]
pub async fn library_remove_sent(
    service: State<'_, InstallService>,
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    player: State<'_, PlayerService>,
    uuid: String,
) -> AppResult<()> {
    let cache_root = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    let saves_root = config.dir.join(SAVES_ROOT);
    // Même portillon que « Supprimer ».
    let _gate = catalog.cache_gate().read_owned().await;
    let (installs, players) = (service.inner().clone(), player.inner().clone());
    blocking(move || {
        // Pas pendant un envoi (de cette histoire ou d'une autre) : le verrou de la Flam est tenu.
        let _device = DEVICE_LOCK
            .try_begin(Operation::Transfer)
            .map_err(TransferError::from)?;
        remove_sent(&SENT, &uuid, &|| installing(&installs, &uuid), |check| {
            crate::library::remove_story(
                &installs,
                &players,
                &cache_root,
                &saves_root,
                &uuid,
                false,
                check,
            )
        })
    })
    .await?
}

#[cfg(test)]
mod tests;
