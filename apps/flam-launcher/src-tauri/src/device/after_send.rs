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
//! # Retrait automatique : écriture confirmée sur le support
//!
//! La relecture a lieu dans `.flam-transfer/<uuid>.in/`, avant le renommage en `str/<uuid>/`, et
//! le renommage n'est forcé sur le support qu'au mieux. Le retrait automatique n'a donc lieu que
//! si, en plus, le renommage a été forcé (`dir_synced`) et la relecture a évité le cache du
//! système (`verified_uncached`), et seulement sous Windows ([`durable`]). Sinon la copie est
//! gardée (`kept`, `unconfirmed`) : « Retirer du PC » reste proposé.
//!
//! Le retrait automatique se fait verrou de la Flam tenu, après l'envoi : il peut attendre le
//! créneau du player (lancement en préparation), et l'annulation de l'envoi est alors sans effet.
//!
//! # Même génération
//!
//! L'empreinte de l'entrée envoyée ([`SentStamp`] : fichier, taille et date d'installation de
//! l'entrée, taille et date du fichier) est prise **avant** la préparation, revérifiée après
//! l'envoi, puis une dernière fois sous le verrou de la bibliothèque juste avant le retrait. Une
//! histoire réinstallée ou mise à jour entre-temps n'est jamais retirée.
//!
//! # Toujours sur la Flam
//!
//! Un envoi réussi enregistre dans [`SENT`] (mémoire du processus) l'empreinte et l'histoire
//! telle qu'installée ([`OnDevice`] : racine, empreinte du `.mdf`, identifiant sur la Flam,
//! fichiers et tailles). Avant tout retrait, verrou de la Flam tenu, [`OnDevice::verify`] relit
//! la Flam : même racine et même `.mdf`, histoire listée dans l'index visible, chaque fichier
//! envoyé présent dans `str/<id>/` avec sa taille. Sinon refus, et l'entrée est oubliée.
//! L'entrée est aussi oubliée au début de tout nouvel envoi de l'histoire, et quand l'histoire
//! est retirée de la Flam (`device_remove_story`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::SystemTime;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use super::index::{read_index, IndexKind, IndexState, StoryId, STORIES_DIR};
use super::lock::{Operation, DEVICE_LOCK};
use super::transfer::{read_mdf, TransferError, TransferReport};
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
/// La Flam de l'envoi n'est pas branchée (débranchée, ou une autre Flam).
pub const OTHER_DEVICE_MESSAGE: &str = "la Flam de l'envoi n'est pas branchée : l'histoire n'a \
     pas été retirée du PC";
/// L'histoire n'est plus sur la Flam telle qu'elle a été envoyée.
pub const NOT_ON_DEVICE_MESSAGE: &str = "l'histoire n'a pas été retrouvée sur la Flam : elle n'a \
     pas été retirée du PC";

/// Racine d'une Flam, sans séparateur final ni casse (comme la garde des écritures).
fn root_key(root: &Path) -> String {
    root.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

/// Deux écritures du même UUID (formes à tirets ou non, casse indifférente).
fn same_story(a: &str, b: &str) -> bool {
    matches!(
        (uuid::Uuid::try_parse(a), uuid::Uuid::try_parse(b)),
        (Ok(x), Ok(y)) if x == y
    )
}

/// Histoire envoyée, telle qu'installée sur la Flam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnDevice {
    root: String,
    mdf_digest: [u8; 32],
    story: String,
    files: Vec<(String, u64)>,
}

impl OnDevice {
    /// Relevé juste après un envoi réussi : `.mdf` relu ; `None` s'il est illisible ou si le
    /// rapport ne cite aucun fichier (aucun retrait possible).
    fn capture(root: &Path, report: &TransferReport) -> Option<Self> {
        if report.installed_files.is_empty() {
            return None;
        }
        let mdf = read_mdf(root).ok()?;
        Some(Self {
            root: root_key(root),
            mdf_digest: Sha256::digest(&*mdf).into(),
            story: report.id.clone(),
            files: report.installed_files.clone(),
        })
    }

    /// Relit la Flam de `root` : même racine et même `.mdf` (sinon [`OTHER_DEVICE_MESSAGE`]),
    /// histoire listée dans l'index visible, dossier `str/<id>/` avec chaque fichier envoyé à sa
    /// taille (sinon [`NOT_ON_DEVICE_MESSAGE`]). Lecture seule.
    pub(crate) fn verify(&self, root: &Path) -> AppResult<()> {
        let other = || AppError::NotRemovable(OTHER_DEVICE_MESSAGE.into());
        let missing = || AppError::NotRemovable(NOT_ON_DEVICE_MESSAGE.into());
        if root_key(root) != self.root {
            return Err(other());
        }
        let mdf = read_mdf(root).map_err(|_| other())?;
        if <[u8; 32]>::from(Sha256::digest(&*mdf)) != self.mdf_digest {
            return Err(other());
        }
        let id = StoryId::parse(&self.story).ok_or_else(missing)?;
        let list = read_index(root, IndexKind::Visible);
        if list.state != IndexState::Valid || !list.entries.contains(&id) {
            return Err(missing());
        }
        let dir = root.join(STORIES_DIR).join(id.as_str());
        let intact = !self.files.is_empty()
            && self.files.iter().all(|(relative, size)| {
                std::fs::symlink_metadata(dir.join(relative))
                    .is_ok_and(|m| m.is_file() && m.len() == *size)
            });
        if intact {
            Ok(())
        } else {
            Err(missing())
        }
    }
}

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

/// Envoi réussi enregistré : entrée de bibliothèque et histoire sur la Flam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Sent {
    stamp: SentStamp,
    device: OnDevice,
}

/// Envois réussis de la session : histoire de la bibliothèque -> envoi enregistré.
#[derive(Debug, Default)]
pub(crate) struct SentRegistry(Mutex<BTreeMap<String, Sent>>);

impl SentRegistry {
    pub(crate) const fn new() -> Self {
        Self(Mutex::new(BTreeMap::new()))
    }

    fn map(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Sent>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, uuid: &str, sent: Sent) {
        self.map().insert(uuid.to_string(), sent);
    }

    fn get(&self, uuid: &str) -> Option<Sent> {
        self.map().get(uuid).cloned()
    }

    /// Oublie `uuid` s'il est toujours associé à `sent` (un envoi plus récent est gardé).
    fn forget(&self, uuid: &str, sent: &Sent) {
        let mut map = self.map();
        if map.get(uuid) == Some(sent) {
            map.remove(uuid);
        }
    }

    /// Oublie l'envoi de l'histoire `uuid` de la bibliothèque (nouvel envoi qui commence).
    pub(crate) fn forget_uuid(&self, uuid: &str) {
        self.map().remove(uuid);
    }

    /// Oublie les envois dont l'histoire sur la Flam est `story` (retirée de la Flam, ou
    /// remplacée par un autre envoi).
    pub(crate) fn forget_story(&self, story: &str) {
        self.map()
            .retain(|_, sent| !same_story(&sent.device.story, story));
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
    /// Gardée ; « Retirer du PC » est possible. `unconfirmed` : l'écriture sur la Flam n'a pas
    /// pu être confirmée sur le support ([`durable`]) ; l'interface rappelle d'éjecter la Flam
    /// avant de retirer la copie (et, si le retrait automatique était demandé, dit pourquoi il
    /// n'a pas eu lieu).
    /// `auto` : le retrait automatique était demandé (réglage), seul cas notifié.
    Kept { unconfirmed: bool, auto: bool },
    /// Retirée automatiquement (réglage `remove_after_send`).
    Removed,
    /// Retrait automatique impossible : l'envoi reste réussi, l'histoire reste sur le PC.
    RemoveFailed { message: String },
}

/// Vérification faite sous le verrou de la bibliothèque, avant toute purge : entrée installée
/// et chemin de son fichier.
pub(crate) type Check<'a> = &'a dyn Fn(&LibraryEntry, Option<&Path>) -> AppResult<()>;

/// Retire l'histoire `uuid` envoyée (enregistrée dans `registry`) par `remove`, à appeler verrou
/// de la Flam tenu. D'abord, l'histoire doit être retrouvée telle qu'envoyée sur la Flam de
/// `root` ([`OnDevice::verify`]) ; puis `remove` reçoit la vérification à faire sous le verrou de
/// la bibliothèque : pas d'installation en cours (`installing`), entrée identique à celle
/// envoyée. L'envoi enregistré est oublié une fois l'histoire retirée, ou s'il ne peut plus
/// servir (histoire absente de la Flam ou de la bibliothèque, entrée changée).
pub(crate) fn remove_sent(
    registry: &SentRegistry,
    root: &Path,
    uuid: &str,
    installing: &dyn Fn() -> bool,
    remove: impl FnOnce(Check<'_>) -> AppResult<()>,
) -> AppResult<()> {
    let sent = registry
        .get(uuid)
        .ok_or_else(|| AppError::NotFound(NOT_SENT_MESSAGE.into()))?;
    if let Err(err) = sent.device.verify(root) {
        registry.forget(uuid, &sent);
        return Err(err);
    }
    let changed = std::cell::Cell::new(false);
    let check = |entry: &LibraryEntry, path: Option<&Path>| -> AppResult<()> {
        if installing() {
            return Err(AppError::Validation(INSTALLING_MESSAGE.into()));
        }
        if !sent.stamp.matches(entry, path) {
            changed.set(true);
            return Err(AppError::NotRemovable(CHANGED_MESSAGE.into()));
        }
        Ok(())
    };
    let result = remove(&check);
    if result.is_ok() || changed.get() || matches!(result, Err(AppError::NotFound(_))) {
        registry.forget(uuid, &sent);
    }
    result
}

/// Écriture confirmée sur le support, condition du retrait automatique : renommage forcé et
/// relecture sans cache, et seulement sous Windows (`windows`).
fn durable_on(report: &TransferReport, windows: bool) -> bool {
    windows && report.dir_synced && report.verified_uncached
}

/// [`durable_on`] pour la plate-forme courante.
fn durable(report: &TransferReport) -> bool {
    durable_on(report, cfg!(windows))
}

/// Envoi de l'histoire `uuid` de la bibliothèque de `cache_root` sur la Flam de `root` par
/// `send` (qui reçoit le chemin de son fichier), puis suite donnée à sa copie dans « Mes
/// histoires » si l'envoi a réussi ([`settle`]). L'envoi enregistré précédent de cette histoire
/// est oublié avant l'envoi (un échec ne le laisse pas). Une erreur de `send` est rendue telle
/// quelle, sans rien retirer ni enregistrer. À appeler sous le verrou de la Flam.
#[allow(clippy::too_many_arguments)]
pub(crate) fn send_then_settle(
    registry: &SentRegistry,
    cache_root: &Path,
    root: &Path,
    uuid: &str,
    auto: bool,
    installing: &dyn Fn() -> bool,
    send: impl FnOnce(&Path) -> AppResult<TransferReport>,
    remove: impl FnOnce(Check<'_>) -> AppResult<()>,
) -> AppResult<TransferReport> {
    // Oublié avant l'envoi, même si la garde le refuse ensuite sans rien écrire (autre Flam que
    // celle du clic, #609) : ce refus a le même `kind` que les changements de Flam vus pendant
    // l'envoi, après lesquels l'ancien relevé ne vaut plus à coup sûr ; les distinguer ici pour
    // rétablir l'entrée ferait courir un risque pour un gain nul. Oublier ne retire qu'une
    // commodité (« Retirer du PC », que l'interface retire de toute façon dès que la Flam
    // change) et ne permet jamais un retrait de plus.
    registry.forget_uuid(uuid);
    // Empreinte prise avant de lire le fichier envoyé.
    let before = SentStamp::capture(cache_root, uuid);
    let (plain_pk, _) = Library::new(cache_root).playable(uuid)?;
    let mut report = send(&plain_pk)?;
    if report.device_changed {
        // Flam changée juste après l'envoi (#577) : la vérification se ferait peut-être sur une
        // autre Flam. Ni retrait automatique ni « Retirer du PC » ; un envoi plus ancien de la
        // même histoire ne vaut plus.
        registry.forget_story(&report.id);
        report.library_copy = None;
        return Ok(report);
    }
    report.library_copy = settle(
        registry,
        cache_root,
        root,
        uuid,
        before,
        &report,
        auto,
        durable(&report),
        installing,
        remove,
    );
    Ok(report)
}

/// Suite d'un envoi réussi de l'histoire `uuid` de la bibliothèque de `cache_root` sur la Flam
/// de `root` : `before` est son empreinte prise avant la préparation, `durable` vrai si
/// l'écriture est confirmée sur le support. Rend `None` si rien n'est proposé (relecture carrier
/// illisible, empreinte absente, `.mdf` illisible, ou entrée changée sans retrait automatique
/// demandé). L'envoi est réussi quoi qu'il arrive : un retrait en échec est rapporté, jamais
/// propagé.
#[allow(clippy::too_many_arguments)]
fn settle(
    registry: &SentRegistry,
    cache_root: &Path,
    root: &Path,
    uuid: &str,
    before: Option<SentStamp>,
    report: &TransferReport,
    auto: bool,
    durable: bool,
    installing: &dyn Fn() -> bool,
    remove: impl FnOnce(Check<'_>) -> AppResult<()>,
) -> Option<LibraryCopy> {
    // L'histoire de la Flam vient d'être (ré)écrite, même illisible ou sans suite ici : un envoi
    // plus ancien qui la désignait ne vaut plus.
    registry.forget_story(&report.id);
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
    let Some(device) = OnDevice::capture(root, report) else {
        tracing::warn!(%uuid, "histoire envoyée, mais .mdf illisible : « Retirer du PC » indisponible");
        return None;
    };
    registry.record(
        uuid,
        Sent {
            stamp: before,
            device,
        },
    );
    if !auto {
        // Pas de retrait demandé : l'interface rappelle seulement d'éjecter avant de retirer.
        return Some(LibraryCopy::Kept {
            unconfirmed: !durable,
            auto: false,
        });
    }
    if !durable {
        tracing::info!(%uuid, "écriture sur la Flam non confirmée : pas de retrait automatique");
        return Some(LibraryCopy::Kept {
            unconfirmed: true,
            auto: true,
        });
    }
    Some(
        match remove_sent(registry, root, uuid, installing, remove) {
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
        },
    )
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
/// progression. Verrou de la Flam tenu, l'histoire doit d'abord être retrouvée telle qu'envoyée
/// sur la Flam de `root` ([`OnDevice::verify`], lecture seule).
///
/// Refus passagers (l'envoi reste enregistré, un nouvel essai peut réussir) : Flam occupée
/// (`validation`, message « la Flam est occupée »), lecture en cours (`player_busy`),
/// installation en cours (`validation`), bibliothèque tenue par une autre instance (`io`).
/// Refus définitifs (envoi oublié) : `not_removable` (autre Flam ou Flam débranchée, histoire
/// plus sur la Flam, entrée changée), `not_found` (aucun envoi réussi connu).
#[tauri::command]
pub async fn library_remove_sent(
    service: State<'_, InstallService>,
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    player: State<'_, PlayerService>,
    root: String,
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
        remove_sent(
            &SENT,
            Path::new(&root),
            &uuid,
            &|| installing(&installs, &uuid),
            |check| {
                crate::library::remove_story(
                    &installs,
                    &players,
                    &cache_root,
                    &saves_root,
                    &uuid,
                    false,
                    check,
                )
            },
        )
    })
    .await?
}

#[cfg(test)]
mod tests;
