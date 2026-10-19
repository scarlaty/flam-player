//! Transfert sûr d'une histoire préparée ([`super::prepare`]) vers la Flam (#229).
//!
//! # Déroulé
//!
//! 0. Garde commune ([`check_write`], #317) : racine détectée, `.mdf` lisible, firmware 1.x ou
//!    2.x, clés lisibles, confirmation ou vérification d'une 2.x ; sinon rien n'est écrit, pas même une
//!    reprise. Clés ([`send_from_library`]) : lues dans `.mdf` en firmware 2.x ; en 1.x, couple
//!    « carrier » d'une histoire officielle, après consentement et contrôle de cohérence avec la
//!    Flam ([`carrier::resolve`], #234). Rien n'est écrit tant que ce n'est pas passé ; `info`
//!    est relu après l'installation ([`carrier::readback`]).
//! 1. Vérifications, sans rien écrire : appareil (`.mdf`), UUID, verrou de l'appareil
//!    ([`DeviceLock`](super::lock::DeviceLock)), reprise d'un transfert interrompu ([`recover`]) et des index
//!    ([`index::recover`]), index lisibles, histoire ni masquée ni déjà présente (sauf mise à
//!    jour demandée), espace libre suffisant (taille arrondie aux clusters, plus
//!    [`SPACE_MARGIN`]).
//! 2. Copie dans `.flam-transfer/<uuid>.in/` à la racine de l'appareil, fichier par fichier,
//!    chacun synchronisé sur le disque (`sync_all`). Empreinte SHA-256 calculée à la lecture de
//!    la source.
//! 3. Vérification : chaque fichier est relu sur l'appareil (sous Windows sans le cache du
//!    système, `FILE_FLAG_NO_BUFFERING`) ; taille et empreinte comparées.
//! 4. Dernier point d'annulation. Mise à jour : l'ancien `str/<uuid>/` est mis de côté par
//!    renommage en `.flam-transfer/<uuid>.old/` (rien n'est supprimé).
//! 5. Renommage de `<uuid>.in/` en `str/<uuid>/`. En cas d'échec, l'ancienne version est remise
//!    en place.
//! 6. Ajout de l'UUID à la fin de `etc/library/list` par [`index::write_index`] (écriture sûre),
//!    s'il n'y figure pas déjà.
//! 7. Suppression de `usr/0/library.cache` (le firmware le reconstruit), même si l'index n'a pas
//!    changé : le contenu de l'histoire, donc peut-être son titre, a changé.
//! 8. Suppression de l'ancienne version, puis du dossier `.flam-transfer/` s'il est vide.
//!
//! # Emplacement du dossier temporaire (point T3 de `docs/flam-device-format.md`)
//!
//! Le dossier temporaire est **hors de `str/`**, dans `.flam-transfer/` à la racine du volume,
//! comme la corbeille du retrait (#231). Le firmware énumère peut-être `str/` (T3, T4) : un
//! dossier `str/.tmp-<uuid>` incomplet pourrait y être vu comme une histoire. Un dossier inconnu
//! à la racine n'est pas une histoire ; au pire le firmware l'ignore ou le supprime, ce qui ne
//! perd qu'une copie en cours. Le renommage vers `str/<uuid>` reste un déplacement sur le même
//! volume (aucune donnée copiée). `tmp/` est écarté : c'est le dossier de travail du firmware.
//! À confirmer sur appareil (#236).
//!
//! # Invariants
//!
//! - Aucun index ne pointe vers un dossier incomplet : `str/<uuid>/` n'apparaît que par
//!   renommage d'un dossier entièrement copié, synchronisé et relu ; l'index n'est modifié
//!   qu'ensuite.
//! - L'ancienne version d'une histoire mise à jour n'est jamais supprimée avant que la nouvelle
//!   soit en place ; jusque-là elle est soit dans `str/<uuid>/`, soit dans
//!   `.flam-transfer/<uuid>.old/` d'où [`recover`] la remet en place.
//! - Annulation, échec ou espace insuffisant avant l'étape 5 : index et `str/` inchangés, le
//!   dossier temporaire est supprimé. Une vérification d'espace qui échoue n'écrit rien.
//! - Seuls `str/<uuid>/` de cet UUID, `.flam-transfer/`, `list` (via `index`) et
//!   `library.cache` sont modifiés ; jamais `usr/0/` (sauvegardes) ni les réglages.
//!
//! # Coupure ou échec à chaque étape
//!
//! | Coupure après…                 | État laissé                                   | Reprise ([`recover`])                     |
//! |--------------------------------|-----------------------------------------------|-------------------------------------------|
//! | les vérifications              | rien de modifié                               | aucune                                    |
//! | une copie partielle            | `<uuid>.in/` incomplet ; index, `str/` intacts | `<uuid>.in/` supprimé                     |
//! | la vérification                | `<uuid>.in/` complet mais non promu            | `<uuid>.in/` supprimé (copie à refaire)   |
//! | la mise de côté (mise à jour)  | `<uuid>.old/`, `<uuid>.in/`, pas de `str/<uuid>` | `<uuid>.old/` remis en `str/<uuid>/`, `.in` supprimé |
//! | le renommage final             | `str/<uuid>/` complet, peut-être `<uuid>.old/` | `<uuid>.old/` supprimé, cache supprimé   |
//! | (installation) avant l'index   | `str/<uuid>/` complet hors index (orphelin)   | aucune ; un nouvel envoi le remplace      |
//! | une étape de l'index           | ancien ou nouvel index (voir `index`)          | `index::recover`                          |
//! | l'index, avant le cache        | `library.cache` périmé                         | supprimé si `<uuid>.old/` reste ; sinon reconstruit au prochain envoi ou redémarrage |
//!
//! FAT n'a pas de journal : une coupure pendant un renommage de dossier peut laisser deux
//! entrées vers le même dossier (incohérence que `chkdsk` répare). Avant de supprimer un
//! `.old` ou un `.in` dont l'histoire est aussi dans `str/`, la reprise vérifie que ce ne sont
//! pas deux noms du même dossier (fichier sonde) ; si c'est le cas, rien n'est supprimé.
//!
//! Les opérations sur l'appareil sont sérialisées par [`DeviceLock`](super::lock::DeviceLock) (dans le processus) ; un
//! second launcher est empêché par le plugin d'instance unique.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::State;

use super::carrier::{
    self, CarrierChoice, CarrierError, CarrierRequest, CarrierStore, EnteredBt, EnteredPreview,
    Resolved,
};
use super::crypto::{
    parse_serial_field, CryptoError, DeviceKeys, FirmwareFamily, MDF_IV_OFFSET, MDF_KEY_OFFSET,
    MDF_SERIAL_OFFSET, SERIAL_FIELD_LEN,
};
use super::detect::{MDF_FILE, MDF_MAX_SIZE};
use super::guard::{
    check_write, VersionForm, WriteContext, WriteKind, WritePermit, WriteRefused, MDF_MIN_LEN,
};
use super::index::{
    self, read_index, Consent, IndexError, IndexKind, IndexState, Recovery, StoryId, WriteOutcome,
    LIBRARY_CACHE, STORIES_DIR,
};
use super::lock::{Busy, OpGuard, Operation, DEVICE_LOCK};
use super::prepare::{self, Limits, PrepareError, PreparedStory};
use crate::catalog::{blocking, CatalogService};
use crate::library::Library;
use crate::settings::{Environment, SettingsStore};
use crate::storage::{retry_transient, ConfigState};
use crate::{AppError, AppResult};

/// Dossier de travail du launcher sur l'appareil, relatif à la racine (hors de `str/`).
pub const TRANSFER_DIR: &str = ".flam-transfer";
/// Dossier des histoires masquées, relatif à la racine.
pub const HIDDEN_STORIES_DIR: &str = "str.hidden";
/// Suffixe du dossier de copie d'une histoire (`<uuid>.in`).
pub const INCOMING_SUFFIX: &str = ".in";
/// Suffixe de l'ancienne version mise de côté pendant une mise à jour (`<uuid>.old`).
pub const PREVIOUS_SUFFIX: &str = ".old";
/// Taille de cluster supposée pour estimer la place occupée (FAT32 de 8 à 32 Go : 16 ou
/// 32 Kio). Surestimer ne coûte qu'un refus prudent.
pub const ASSUMED_CLUSTER: u64 = 32 * 1024;
/// Marge d'espace libre laissée sur l'appareil après le transfert.
pub const SPACE_MARGIN: u64 = 32 * 1024 * 1024;
/// Sous-dossier du cache du launcher où les histoires sont préparées avant transfert.
pub const WORK_SUBDIR: &str = "flam-transfer";
/// Événement Tauri de progression.
pub const EVENT_PROGRESS: &str = "device://transfer-progress";
/// Taille des blocs copiés et relus (un point d'annulation et un événement par bloc).
const CHUNK: usize = 1024 * 1024;
/// Préfixe du fichier sonde de [`DeviceFs::same_directory`].
const PROBE_PREFIX: &str = ".flam-sonde-";
/// Vrai une fois un échec de [`DeviceFs::sync_dir`] journalisé en avertissement.
static SYNC_DIR_WARNED: AtomicBool = AtomicBool::new(false);

/// Étape en cours.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// Copie vers l'appareil.
    Copy,
    /// Relecture et comparaison.
    Verify,
}

/// Avancement : `done` croît de 0 à `total` (= deux fois la taille de l'histoire : copie puis
/// relecture).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Progress {
    pub phase: Phase,
    pub done: u64,
    pub total: u64,
}

/// Résultat d'un transfert réussi.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TransferReport {
    /// UUID de l'histoire (forme canonique).
    pub id: String,
    /// Octets copiés.
    pub bytes: u64,
    /// Nombre de fichiers copiés.
    pub files: usize,
    /// Une version précédente (`str/<uuid>/`) a été remplacée.
    pub replaced: bool,
    /// L'UUID a été ajouté à `list`.
    pub index_updated: bool,
    /// `library.cache` est supprimé (faux : à supprimer plus tard, voir le journal).
    pub cache_removed: bool,
    /// Mode carrier (firmware 1.x) : `info` relu sur la Flam se déchiffre avec le couple.
    /// `None` hors mode carrier.
    pub carrier_readback: Option<bool>,
    /// Mode carrier : le couple est conservé sur ce PC pour cette Flam (les envois suivants
    /// peuvent le relire). `None` hors mode carrier.
    pub carrier_stored: Option<bool>,
    /// Entrées de `.flam-transfer/` que la reprise faite avant l'envoi n'a pas pu traiter
    /// (« entrée : erreur »), par exemple un dossier verrouillé : l'envoi a réussi, mais une
    /// autre histoire peut rester incomplète jusqu'à la prochaine reprise. Vide d'ordinaire.
    pub recovery_failures: Vec<String>,
}

/// Bilan d'une reprise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TransferRecovery {
    /// Copies interrompues supprimées (`<uuid>.in`).
    pub incomplete_removed: usize,
    /// Anciennes versions remises en place (`<uuid>.old` sans `str/<uuid>`).
    pub previous_restored: usize,
    /// Mises à jour terminées (`<uuid>.old` supprimé, nouvelle version en place).
    pub updates_finished: usize,
    /// Entrées laissées en place : inconnues, ou deux noms du même dossier (`chkdsk`).
    pub left_alone: usize,
}

/// Erreur d'un transfert. Les messages ne contiennent jamais de clé.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("Flam introuvable à cet emplacement (fichier .mdf absent) : rien n'a été écrit")]
    DeviceNotFound,
    #[error("identifiant d'histoire invalide : rien n'a été écrit")]
    InvalidId,
    #[error(
        "emplacement refusé : ce n'est pas un lecteur amovible ou FAT détecté ; rien n'a été écrit"
    )]
    InvalidRoot,
    #[error("espace insuffisant sur la Flam : {needed} octets nécessaires (marge comprise), {free} disponibles ; rien n'a été installé")]
    NotEnoughSpace { needed: u64, free: u64 },
    #[error("l'histoire {id} est déjà sur la Flam{}", hidden_note(.hidden))]
    AlreadyInstalled { id: StoryId, hidden: bool },
    /// « Remplacer » visait une autre histoire de la Flam que celle préparée (`uuid.bin`).
    #[error(
        "l'histoire préparée ({id}) n'est pas celle à remplacer sur la Flam : rien n'a été écrit"
    )]
    ReplaceMismatch { id: StoryId },
    #[error("firmware de la Flam non pris en charge : « {0} » (2.x, ou 1.x avec les clés d'une histoire officielle ; 3.x et suivants non vérifiés)")]
    UnsupportedFirmware(String),
    #[error("clés de la Flam illisibles : {0}")]
    KeyUnavailable(String),
    #[error(transparent)]
    Busy(#[from] Busy),
    /// Refus de la garde commune des écritures ([`super::guard`]).
    #[error(transparent)]
    Refused(#[from] WriteRefused),
    #[error("transfert annulé : la Flam est inchangée")]
    Cancelled,
    #[error("vérification échouée ({0}) : la copie relue sur la Flam diffère de l'original ; rien n'a été installé")]
    Verify(String),
    #[error(
        "index de la Flam absent, abîmé ou illisible : réparez-le avant d'ajouter une histoire"
    )]
    IndexNeedsUser,
    #[error("index de la Flam : {0}")]
    Index(#[from] IndexError),
    #[error(transparent)]
    Prepare(PrepareError),
    /// Mode carrier (firmware 1.x) : consentement, source ou cohérence du couple.
    #[error(transparent)]
    Carrier(#[from] CarrierError),
    #[error("{0}")]
    Io(#[from] io::Error),
}

fn hidden_note(hidden: &bool) -> &'static str {
    if *hidden {
        " (masquée)"
    } else {
        ""
    }
}

impl From<TransferError> for AppError {
    fn from(err: TransferError) -> Self {
        match err {
            TransferError::Cancelled
            | TransferError::Prepare(PrepareError::Cancelled)
            | TransferError::Carrier(CarrierError::Cancelled) => AppError::Cancelled,
            TransferError::DeviceNotFound
            | TransferError::InvalidId
            | TransferError::InvalidRoot
            | TransferError::NotEnoughSpace { .. }
            | TransferError::AlreadyInstalled { .. }
            | TransferError::ReplaceMismatch { .. }
            | TransferError::UnsupportedFirmware(_)
            | TransferError::KeyUnavailable(_)
            | TransferError::Busy(_)
            | TransferError::Refused(_)
            | TransferError::IndexNeedsUser
            | TransferError::Carrier(
                CarrierError::ConsentRequired
                | CarrierError::CarrierRequired
                | CarrierError::NoStoredKeys
                | CarrierError::InvalidSource(_)
                | CarrierError::Mismatch
                | CarrierError::EnteredMismatch
                | CarrierError::EnteredUnconfirmed
                | CarrierError::Store(_),
            ) => AppError::Validation(err.to_string()),
            TransferError::Carrier(CarrierError::Io(_))
            | TransferError::Verify(_)
            | TransferError::Index(_)
            | TransferError::Prepare(_)
            | TransferError::Io(_) => AppError::Io(io::Error::other(err.to_string())),
        }
    }
}

/// Fichier écrit sur l'appareil.
pub(crate) trait SyncWrite: Write {
    /// Force l'écriture des données sur le disque.
    fn sync(&mut self) -> io::Result<()>;
}

impl SyncWrite for File {
    fn sync(&mut self) -> io::Result<()> {
        self.sync_all()
    }
}

/// Opérations sur l'appareil, abstraites pour les tests d'injection de panne. La source (la
/// préparation, hors appareil) est lue directement.
pub(crate) trait DeviceFs {
    /// Espace libre du volume.
    fn free_space(&self, root: &Path) -> io::Result<u64>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Crée un fichier qui ne doit pas exister.
    fn create_file(&self, path: &Path) -> io::Result<Box<dyn SyncWrite + '_>>;
    /// Ouvre un fichier pour la vérification, sans cache du système si possible.
    fn open_verify(&self, path: &Path) -> io::Result<Box<dyn Read + '_>>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Supprime un dossier et son contenu ; absent = succès.
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Supprime un dossier s'il est vide (erreur sinon).
    fn remove_empty_dir(&self, path: &Path) -> io::Result<()>;
    /// Supprime un fichier ; absent = succès.
    fn remove_file(&self, path: &Path) -> io::Result<()>;
    /// Vrai si `a` et `b` désignent le même dossier (deux entrées FAT vers les mêmes
    /// clusters après une coupure pendant un renommage).
    fn same_directory(&self, a: &Path, b: &Path) -> io::Result<bool>;
    /// Remplace `list` par `entries` ([`index::write_index`], sans accord : index valide requis).
    fn write_index(&self, root: &Path, entries: &[StoryId]) -> Result<WriteOutcome, IndexError>;
    /// Force sur le disque les entrées d'un dossier (renommages). Sans effet par défaut.
    fn sync_dir(&self, _path: &Path) -> io::Result<()> {
        Ok(())
    }
}

/// Opérations réelles.
pub(crate) struct RealFs {
    /// Espace libre d'un volume (injectable : `None` hors Windows).
    pub free_space: fn(&Path) -> Option<u64>,
}

impl Default for RealFs {
    fn default() -> Self {
        Self {
            free_space: super::detect::free_space,
        }
    }
}

fn not_found_ok(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

impl DeviceFs for RealFs {
    fn free_space(&self, root: &Path) -> io::Result<u64> {
        (self.free_space)(root)
            .ok_or_else(|| io::Error::other("espace libre de la Flam impossible à lire"))
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }
    fn create_file(&self, path: &Path) -> io::Result<Box<dyn SyncWrite + '_>> {
        Ok(Box::new(File::create_new(path)?))
    }
    fn open_verify(&self, path: &Path) -> io::Result<Box<dyn Read + '_>> {
        #[cfg(windows)]
        match unbuffered::open(path) {
            Ok(reader) => return Ok(Box::new(reader)),
            Err(err) => {
                tracing::warn!(
                    "relecture sans cache impossible ({err}) : relecture avec le cache du système"
                );
            }
        }
        Ok(Box::new(File::open(path)?))
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        retry_transient(|| fs::rename(from, to), std::thread::sleep)
    }
    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        not_found_ok(retry_transient(
            || fs::remove_dir_all(path),
            std::thread::sleep,
        ))
    }
    fn remove_empty_dir(&self, path: &Path) -> io::Result<()> {
        fs::remove_dir(path)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        not_found_ok(retry_transient(
            || fs::remove_file(path),
            std::thread::sleep,
        ))
    }
    fn same_directory(&self, a: &Path, b: &Path) -> io::Result<bool> {
        let name = format!("{PROBE_PREFIX}{}", uuid::Uuid::new_v4().simple());
        let probe = a.join(&name);
        File::create_new(&probe)?.sync_all()?;
        let seen = b.join(&name).try_exists();
        let removed = fs::remove_file(&probe);
        let seen = seen?;
        removed?;
        Ok(seen)
    }
    fn write_index(&self, root: &Path, entries: &[StoryId]) -> Result<WriteOutcome, IndexError> {
        index::write_index(root, IndexKind::Visible, entries, Consent::IfValid)
    }
    #[cfg(windows)]
    fn sync_dir(&self, path: &Path) -> io::Result<()> {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
        // Un dossier ne s'ouvre qu'avec FILE_FLAG_BACKUP_SEMANTICS ; FlushFileBuffers exige
        // l'accès en écriture.
        fs::OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)?
            .sync_all()
    }
}

/// Lecture sans le cache de fichiers de Windows : la vérification relit le support, pas une
/// copie en mémoire de ce qui vient d'être écrit.
#[cfg(windows)]
mod unbuffered {
    use std::fs::{File, OpenOptions};
    use std::io::{self, Read};
    use std::os::windows::fs::OpenOptionsExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_NO_BUFFERING;

    /// Alignement de l'adresse et de la taille des lectures (multiple des tailles de secteur
    /// courantes, 512 et 4096 octets).
    const ALIGN: usize = 4096;
    const BLOCK: usize = 1024 * 1024;

    pub struct Reader {
        file: File,
        buf: Vec<u8>,
        start: usize,
        pos: usize,
        end: usize,
        eof: bool,
    }

    pub fn open(path: &Path) -> io::Result<Reader> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_NO_BUFFERING)
            .open(path)?;
        let buf = vec![0u8; BLOCK + ALIGN];
        let start = buf.as_ptr().align_offset(ALIGN);
        Ok(Reader {
            file,
            buf,
            start,
            pos: 0,
            end: 0,
            eof: false,
        })
    }

    impl Read for Reader {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.pos == self.end {
                if self.eof || out.is_empty() {
                    return Ok(0);
                }
                let n = self
                    .file
                    .read(&mut self.buf[self.start..self.start + BLOCK])?;
                // Une lecture courte ne se produit qu'en fin de fichier ; la suivante serait à
                // une position non alignée, refusée par le système.
                self.eof = n < BLOCK;
                self.pos = 0;
                self.end = n;
                if n == 0 {
                    return Ok(0);
                }
            }
            let k = (self.end - self.pos).min(out.len());
            let from = self.start + self.pos;
            out[..k].copy_from_slice(&self.buf[from..from + k]);
            self.pos += k;
            Ok(k)
        }
    }
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), TransferError> {
    if cancel.load(Ordering::Relaxed) {
        Err(TransferError::Cancelled)
    } else {
        Ok(())
    }
}

/// Fichier à copier : chemin relatif (`/`) et taille de la source.
struct Planned {
    relative: String,
    size: u64,
}

/// Relit la préparation : chemins relatifs sûrs, fichiers présents, tailles réelles.
fn plan(prepared: &PreparedStory) -> Result<(Vec<Planned>, u64), TransferError> {
    let mut files = Vec::with_capacity(prepared.files.len());
    let mut total = 0u64;
    for relative in &prepared.files {
        let safe = !relative.is_empty()
            && Path::new(relative)
                .components()
                .all(|c| matches!(c, Component::Normal(_)))
            && !relative.contains('\\');
        if !safe {
            return Err(TransferError::Io(io::Error::other(format!(
                "chemin préparé refusé : {relative:?}"
            ))));
        }
        let meta = fs::symlink_metadata(prepared.dir.join(relative))?;
        if !meta.is_file() {
            return Err(TransferError::Io(io::Error::other(format!(
                "{relative} n'est pas un fichier"
            ))));
        }
        total += meta.len();
        files.push(Planned {
            relative: relative.clone(),
            size: meta.len(),
        });
    }
    Ok((files, total))
}

/// Place occupée estimée : chaque fichier arrondi au cluster, un cluster par dossier.
fn needed_space(files: &[Planned]) -> u64 {
    let mut dirs = std::collections::HashSet::new();
    dirs.insert(String::new());
    let mut needed = 0u64;
    for f in files {
        needed += f.size.div_ceil(ASSUMED_CLUSTER) * ASSUMED_CLUSTER;
        let mut parent = f.relative.as_str();
        while let Some((dir, _)) = parent.rsplit_once('/') {
            dirs.insert(dir.to_string());
            parent = dir;
        }
    }
    needed + dirs.len() as u64 * ASSUMED_CLUSTER + SPACE_MARGIN
}

fn incoming_dir(root: &Path, id: &StoryId) -> PathBuf {
    root.join(TRANSFER_DIR)
        .join(format!("{id}{INCOMING_SUFFIX}"))
}

fn previous_dir(root: &Path, id: &StoryId) -> PathBuf {
    root.join(TRANSFER_DIR)
        .join(format!("{id}{PREVIOUS_SUFFIX}"))
}

/// Transfère `prepared` sur la Flam de racine `root`. `guard` prouve que le verrou de
/// l'appareil est tenu. `replace` : UUID de l'histoire de la Flam que l'utilisateur a choisi de
/// remplacer (forme simple ou à tirets) ; une histoire déjà listée n'est mise à jour que s'il
/// désigne `prepared.uuid` (sinon [`TransferError::ReplaceMismatch`] ; sans `replace`,
/// [`TransferError::AlreadyInstalled`]). Un dossier `str/<uuid>/` absent de l'index (orphelin,
/// par exemple après une coupure avant l'étape 6) est remplacé sans condition, avec la même
/// mise de côté qu'une mise à jour. `cancel` est consulté entre chaque bloc, jusqu'au
/// renommage final. Bloquant.
pub fn send_story(
    _guard: &OpGuard<'_>,
    permit: &WritePermit,
    root: &Path,
    prepared: &PreparedStory,
    replace: Option<&str>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<TransferReport, TransferError> {
    send_with(
        &RealFs::default(),
        permit,
        root,
        prepared,
        replace,
        cancel,
        progress,
    )
}

pub(crate) fn send_with(
    fs: &dyn DeviceFs,
    permit: &WritePermit,
    root: &Path,
    prepared: &PreparedStory,
    replace: Option<&str>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<TransferReport, TransferError> {
    if !root.join(MDF_FILE).is_file() {
        return Err(TransferError::DeviceNotFound);
    }
    let id = StoryId::parse(&prepared.uuid)
        .filter(|id| id.as_str() == prepared.uuid)
        .ok_or(TransferError::InvalidId)?;
    // « Remplacer » est lié à l'histoire choisie : une archive dont `uuid.bin` désigne une autre
    // histoire de la Flam ne l'écrase jamais. Contrôlé avant toute écriture, reprise comprise.
    let replace = match replace {
        None => false,
        Some(target) if same_uuid(target, &id) => true,
        Some(_) => return Err(TransferError::ReplaceMismatch { id }),
    };
    check_cancel(cancel)?;

    // 1. Reprises et vérifications : rien n'est écrit tant qu'elles ne sont pas passées
    // (hormis la fin d'opérations interrompues).
    // Une entrée de reprise qui échoue (dossier verrouillé) ne bloque pas l'envoi d'une autre
    // histoire ; elle est signalée dans le rapport, et celle de cet UUID est refusée plus bas.
    let (_, failures) = recover_collect(fs, permit, root)?;
    // Un résidu laissé par la reprise (deux noms du même dossier, ou entrée verrouillée) ne
    // doit jamais être réutilisé ni supprimé par le nettoyage d'un échec : il partage peut-être
    // les fichiers de l'histoire.
    for leftover in [incoming_dir(root, &id), previous_dir(root, &id)] {
        if leftover.try_exists()? {
            let name = leftover
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            let failed = failures
                .iter()
                .find(|(entry, _)| Some(entry) == name.as_ref());
            let message = match failed {
                // Reprise en échec (fichier ouvert, erreur d'entrée/sortie) : pas une
                // incohérence du système de fichiers.
                Some((_, err)) => format!(
                    "{} n'a pas pu être nettoyé ({err}) : fermez ce qui utilise la Flam puis réessayez ; rien n'a été écrit",
                    leftover.display()
                ),
                None => format!(
                    "{} n'a pas pu être nettoyé (vérifiez la Flam avec chkdsk) : rien n'a été écrit",
                    leftover.display()
                ),
            };
            return Err(TransferError::Io(io::Error::other(message)));
        }
    }
    let recovery_failures: Vec<String> = failures
        .iter()
        .map(|(entry, err)| format!("{entry} : {err}"))
        .collect();
    for kind in IndexKind::ALL {
        if index::recover(root, kind)? == Recovery::NeedsUser {
            return Err(TransferError::IndexNeedsUser);
        }
    }
    let list = read_index(root, IndexKind::Visible);
    if list.state != IndexState::Valid {
        return Err(TransferError::IndexNeedsUser);
    }
    let hidden = read_index(root, IndexKind::Hidden);
    if !matches!(hidden.state, IndexState::Valid | IndexState::Absent) {
        return Err(TransferError::IndexNeedsUser);
    }
    if hidden.entries.contains(&id) || root.join(HIDDEN_STORIES_DIR).join(id.as_str()).exists() {
        return Err(TransferError::AlreadyInstalled { id, hidden: true });
    }
    let target = root.join(STORIES_DIR).join(id.as_str());
    let listed = list.entries.contains(&id);
    let present = target.try_exists()?;
    if listed && present && !replace {
        return Err(TransferError::AlreadyInstalled { id, hidden: false });
    }
    let (files, total) = plan(prepared)?;
    let needed = needed_space(&files);
    let free = fs.free_space(root)?;
    if free < needed {
        return Err(TransferError::NotEnoughSpace { needed, free });
    }
    check_cancel(cancel)?;

    // 2 et 3. Copie et vérification dans le dossier temporaire.
    let incoming = incoming_dir(root, &id);
    let copied = (|| {
        fs.create_dir_all(&incoming)?;
        copy_and_verify(fs, prepared, &files, &incoming, total, cancel, progress)?;
        check_cancel(cancel)
    })();
    if let Err(err) = copied {
        discard(fs, root, &incoming);
        return Err(match err {
            TransferError::Io(e) if e.kind() == io::ErrorKind::StorageFull => {
                TransferError::NotEnoughSpace {
                    needed,
                    free: fs.free_space(root).unwrap_or(0),
                }
            }
            other => other,
        });
    }

    // 4 et 5. Promotion : plus d'annulation possible à partir d'ici.
    let previous = previous_dir(root, &id);
    let promoted = (|| {
        fs.create_dir_all(&root.join(STORIES_DIR))?;
        if present {
            fs.rename(&target, &previous)?;
        }
        if let Err(err) = fs.rename(&incoming, &target) {
            if present {
                if let Err(back) = fs.rename(&previous, &target) {
                    tracing::warn!(%id, "ancienne version laissée de côté, remise en place à la prochaine reprise : {back}");
                }
            }
            return Err(err);
        }
        Ok(())
    })();
    if let Err(err) = promoted {
        // Un renommage interrompu peut laisser deux noms du même dossier : dans le doute, le
        // dossier temporaire est laissé à la reprise plutôt que supprimé.
        let aliased = target.try_exists().unwrap_or(true)
            && fs.same_directory(&incoming, &target).unwrap_or(true);
        if !aliased {
            discard(fs, root, &incoming);
        }
        return Err(err.into());
    }

    // Les renommages de dossier sont forcés sur le disque avant d'aller plus loin (au mieux) :
    // sans écriture d'index (mise à jour d'une histoire déjà listée), rien d'autre ne le ferait.
    for dir in [
        target.as_path(),
        &root.join(STORIES_DIR),
        &root.join(TRANSFER_DIR),
    ] {
        if let Err(err) = fs.sync_dir(dir) {
            // FAT peut refuser à chaque envoi : un seul avertissement, puis du débogage.
            if SYNC_DIR_WARNED.swap(true, Ordering::Relaxed) {
                tracing::debug!(%id, dossier = %dir.display(), "synchronisation du dossier impossible : {err}");
            } else {
                tracing::warn!(%id, dossier = %dir.display(), "synchronisation du dossier impossible : {err}");
            }
        }
    }

    // 6. Index.
    let index_updated = if listed {
        false
    } else {
        let mut entries = list.entries.clone();
        entries.push(id.clone());
        match fs.write_index(root, &entries) {
            Ok(WriteOutcome::Written) => true,
            Ok(WriteOutcome::Unchanged) => false,
            // Index écrit ; le cache est retenté juste après.
            Err(IndexError::CacheNotRemoved(_)) => true,
            Err(err) => return Err(err.into()),
        }
    };

    // 7. Cache du firmware.
    let cache_removed = match fs.remove_file(&root.join(LIBRARY_CACHE)) {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(%id, "library.cache non supprimé : {err}");
            false
        }
    };

    // 8. Ancienne version et dossier de travail.
    if present {
        if let Err(err) = fs.remove_dir_all(&previous) {
            tracing::warn!(%id, "ancienne version à supprimer à la prochaine reprise : {err}");
        }
    }
    let _ = fs.remove_empty_dir(&root.join(TRANSFER_DIR));
    tracing::info!(%id, fichiers = files.len(), octets = total, mise_a_jour = present, "histoire transférée sur la Flam");
    Ok(TransferReport {
        id: id.to_string(),
        bytes: total,
        files: files.len(),
        replaced: present,
        index_updated,
        cache_removed,
        carrier_readback: None,
        carrier_stored: None,
        recovery_failures,
    })
}

/// Vrai si `target` (forme simple de 32 caractères ou forme à tirets, casse indifférente)
/// désigne l'UUID `id`.
fn same_uuid(target: &str, id: &StoryId) -> bool {
    matches!(target.len(), 32 | 36)
        && matches!(
            (uuid::Uuid::try_parse(target), uuid::Uuid::try_parse(id.as_str())),
            (Ok(a), Ok(b)) if a == b
        )
}

/// Supprime le dossier temporaire (au mieux : la reprise s'en charge sinon).
fn discard(fs: &dyn DeviceFs, root: &Path, incoming: &Path) {
    if let Err(err) = fs.remove_dir_all(incoming) {
        tracing::warn!(dossier = %incoming.display(), "copie interrompue à supprimer à la prochaine reprise : {err}");
    }
    let _ = fs.remove_empty_dir(&root.join(TRANSFER_DIR));
}

fn copy_and_verify(
    fs: &dyn DeviceFs,
    prepared: &PreparedStory,
    files: &[Planned],
    incoming: &Path,
    total: u64,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<(), TransferError> {
    let grand_total = 2 * total;
    let mut done = 0u64;
    let mut buf = vec![0u8; CHUNK];
    progress(Progress {
        phase: Phase::Copy,
        done,
        total: grand_total,
    });

    let mut digests = Vec::with_capacity(files.len());
    for f in files {
        check_cancel(cancel)?;
        let target = incoming.join(&f.relative);
        if let Some(parent) = target.parent() {
            fs.create_dir_all(parent)?;
        }
        let mut input = File::open(prepared.dir.join(&f.relative))?;
        let mut out = fs.create_file(&target)?;
        let mut hasher = Sha256::new();
        let mut len = 0u64;
        loop {
            check_cancel(cancel)?;
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            len += n as u64;
            if len > f.size {
                break;
            }
            out.write_all(&buf[..n])?;
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(Progress {
                phase: Phase::Copy,
                done,
                total: grand_total,
            });
        }
        if len != f.size {
            return Err(TransferError::Io(io::Error::other(format!(
                "{} a changé pendant la copie",
                f.relative
            ))));
        }
        out.flush()?;
        out.sync()?;
        drop(out);
        digests.push(hasher.finalize());
    }

    for (f, expected) in files.iter().zip(digests) {
        check_cancel(cancel)?;
        let mut input = fs.open_verify(&incoming.join(&f.relative))?;
        let mut hasher = Sha256::new();
        let mut len = 0u64;
        loop {
            check_cancel(cancel)?;
            let n = input.read(&mut buf)?;
            if n == 0 {
                break;
            }
            len += n as u64;
            if len > f.size {
                return Err(TransferError::Verify(f.relative.clone()));
            }
            hasher.update(&buf[..n]);
            done += n as u64;
            progress(Progress {
                phase: Phase::Verify,
                done,
                total: grand_total,
            });
        }
        if len != f.size || hasher.finalize() != expected {
            return Err(TransferError::Verify(f.relative.clone()));
        }
    }
    progress(Progress {
        phase: Phase::Verify,
        done: grand_total,
        total: grand_total,
    });
    Ok(())
}

/// Reprend les transferts interrompus sur l'appareil `root` (voir le tableau du module). À
/// appeler sous le verrou de l'appareil, à la détection et avant chaque opération ; le
/// transfert l'appelle lui-même. Une erreur d'entrée/sortie interrompt la reprise sans toucher
/// à la suite.
pub fn recover(
    _guard: &OpGuard<'_>,
    permit: &WritePermit,
    root: &Path,
) -> Result<TransferRecovery, TransferError> {
    recover_with(&RealFs::default(), permit, root)
}

pub(crate) fn recover_with(
    fs: &dyn DeviceFs,
    permit: &WritePermit,
    root: &Path,
) -> Result<TransferRecovery, TransferError> {
    let (report, mut failures) = recover_collect(fs, permit, root)?;
    if failures.is_empty() {
        Ok(report)
    } else {
        Err(failures.swap_remove(0).1)
    }
}

/// Reprise qui continue après l'échec d'une entrée : rend le bilan et les erreurs par entrée.
/// Une erreur de lecture du dossier de travail lui-même interrompt tout.
pub(crate) fn recover_collect(
    fs: &dyn DeviceFs,
    permit: &WritePermit,
    root: &Path,
) -> Result<(TransferRecovery, Vec<(String, TransferError)>), TransferError> {
    if !permit.covers(root) {
        return Err(TransferError::InvalidRoot);
    }
    let dir = root.join(TRANSFER_DIR);
    let mut names = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            for entry in entries {
                names.push(entry?.file_name().to_string_lossy().into_owned());
            }
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Ok((TransferRecovery::default(), Vec::new()))
        }
        Err(err) => return Err(err.into()),
    }
    names.sort();
    let mut report = TransferRecovery::default();
    let mut failures = Vec::new();
    let parse = |name: &str, suffix: &str| {
        name.strip_suffix(suffix)
            .and_then(StoryId::parse)
            .filter(|id| name.starts_with(id.as_str()))
    };
    // Anciennes versions d'abord : elles décident du sort de `str/<uuid>`.
    for name in &names {
        let Some(id) = parse(name, PREVIOUS_SUFFIX) else {
            continue;
        };
        let step = (|| -> Result<(), TransferError> {
            let old = dir.join(name);
            let target = root.join(STORIES_DIR).join(id.as_str());
            if target.try_exists()? {
                if fs.same_directory(&old, &target)? {
                    tracing::warn!(%id, "deux entrées vers le même dossier d'histoire : rien n'est supprimé (chkdsk recommandé)");
                    report.left_alone += 1;
                    return Ok(());
                }
                // Nouvelle version en place (renommée seulement après vérification).
                if let Err(err) = fs.remove_file(&root.join(LIBRARY_CACHE)) {
                    tracing::warn!("library.cache non supprimé : {err}");
                }
                fs.remove_dir_all(&old)?;
                report.updates_finished += 1;
            } else {
                fs.create_dir_all(&root.join(STORIES_DIR))?;
                fs.rename(&old, &target)?;
                report.previous_restored += 1;
            }
            Ok(())
        })();
        if let Err(err) = step {
            tracing::warn!(entree = %name, "reprise impossible : {err}");
            failures.push((name.clone(), err));
        }
    }
    for name in &names {
        if parse(name, PREVIOUS_SUFFIX).is_some() {
            continue;
        }
        let Some(id) = parse(name, INCOMING_SUFFIX) else {
            tracing::warn!(entree = %name, "entrée inconnue laissée dans {TRANSFER_DIR}");
            report.left_alone += 1;
            continue;
        };
        let step = (|| -> Result<(), TransferError> {
            let incoming = dir.join(name);
            let target = root.join(STORIES_DIR).join(id.as_str());
            if target.try_exists()? && fs.same_directory(&incoming, &target)? {
                tracing::warn!(%id, "deux entrées vers le même dossier d'histoire : rien n'est supprimé (chkdsk recommandé)");
                report.left_alone += 1;
                return Ok(());
            }
            fs.remove_dir_all(&incoming)?;
            report.incomplete_removed += 1;
            Ok(())
        })();
        if let Err(err) = step {
            tracing::warn!(entree = %name, "reprise impossible : {err}");
            failures.push((name.clone(), err));
        }
    }
    let _ = fs.remove_empty_dir(&dir);
    if report != TransferRecovery::default() {
        tracing::info!(?report, "reprise des transferts de la Flam");
    }
    Ok((report, failures))
}

/// Version principale du firmware (`2.4.2`) lue dans `.mdf` (`None` : illisible).
pub fn firmware_from_mdf(mdf: &[u8]) -> Option<String> {
    firmware_field(mdf).map(|(version, _)| version)
}

/// Version principale du firmware lue dans `.mdf` et forme du texte qui la porte (`None` :
/// illisible).
pub(crate) fn firmware_field(mdf: &[u8]) -> Option<(String, VersionForm)> {
    firmware_main(mdf.get(2..MDF_SERIAL_OFFSET)?)
}

/// Version principale du firmware (`2.4.2`) lue dans le texte de `.mdf` : « main: x.y.z-… »
/// (cherché n'importe où dans le champ, même après « comm: … »), ou la version nue en tête du
/// champ (`1.15.14-…`, observée sur une Flam 1.15.14, #269).
fn firmware_main(text: &[u8]) -> Option<(String, VersionForm)> {
    let text = String::from_utf8_lossy(text);
    let (rest, form) = match text.find("main:") {
        Some(at) => (&text[at + "main:".len()..], VersionForm::Main),
        None => (&text[..], VersionForm::Bare),
    };
    let version: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    version
        .starts_with(|c: char| c.is_ascii_digit())
        .then_some((version, form))
}

/// Clés de l'appareil d'après le contenu de `.mdf` (disposition dans [`super::crypto`]).
pub fn keys_from_mdf(mdf: &[u8]) -> Result<DeviceKeys, TransferError> {
    if mdf.len() < MDF_MIN_LEN {
        return Err(TransferError::KeyUnavailable(format!(
            "fichier .mdf de {} octets (au moins {MDF_MIN_LEN} attendus)",
            mdf.len()
        )));
    }
    let (firmware, _) = firmware_main(&mdf[2..MDF_SERIAL_OFFSET]).ok_or_else(|| {
        TransferError::KeyUnavailable("version du firmware illisible dans .mdf".into())
    })?;
    let serial = parse_serial_field(&mdf[MDF_SERIAL_OFFSET..MDF_SERIAL_OFFSET + SERIAL_FIELD_LEN])
        .map_err(|e| TransferError::KeyUnavailable(e.to_string()))?;
    // Références dans le tampon `.mdf` (effacé par l'appelant) : ce code ne copie pas les clés
    // (le compilateur peut toutefois en laisser des copies temporaires en registre ou sur la pile).
    let field = |offset: usize| -> &[u8; 16] {
        (&mdf[offset..offset + 16])
            .try_into()
            .expect("16 octets dans un .mdf de taille contrôlée")
    };
    DeviceKeys::new(
        &firmware,
        &serial,
        field(MDF_KEY_OFFSET),
        field(MDF_IV_OFFSET),
    )
    .map_err(|e| match e {
        CryptoError::UnsupportedFirmware(v) => TransferError::UnsupportedFirmware(v),
        other => TransferError::KeyUnavailable(other.to_string()),
    })
}

/// Contenu de `.mdf` (borné, effacé de la mémoire à la destruction).
pub fn read_mdf(root: &Path) -> Result<zeroize::Zeroizing<Vec<u8>>, TransferError> {
    // Capacité fixe : aucune réallocation, donc aucune copie des clés hors du tampon effacé.
    let mut mdf = zeroize::Zeroizing::new(Vec::with_capacity(MDF_MAX_SIZE as usize + 1));
    match File::open(root.join(MDF_FILE)) {
        Ok(file) => file.take(MDF_MAX_SIZE + 1).read_to_end(&mut mdf)?,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return Err(TransferError::DeviceNotFound)
        }
        Err(err) => return Err(err.into()),
    };
    // Même borne que la détection : un `.mdf` trop grand n'est pas celui d'une Flam.
    if mdf.len() as u64 > MDF_MAX_SIZE {
        return Err(TransferError::KeyUnavailable(format!(
            "fichier .mdf de plus de {MDF_MAX_SIZE} octets"
        )));
    }
    Ok(mdf)
}

/// Clés de chiffrement pour la Flam de `root`, et couple carrier vérifié en firmware 1.x
/// ([`carrier::resolve`] : consentement, source, cohérence avec la Flam). Rien n'est écrit sur
/// la Flam.
fn device_keys(
    root: &Path,
    permit: &WritePermit,
    carrier: Option<&CarrierRequest<'_>>,
    cancel: &AtomicBool,
) -> Result<(DeviceKeys, Option<Resolved>), TransferError> {
    let mdf = read_mdf(root)?;
    // Le `.mdf` relu doit être celui que la garde a contrôlé (remplacé entre-temps : refus).
    if !permit.same_mdf(&mdf) {
        return Err(TransferError::KeyUnavailable(
            "le fichier .mdf a changé depuis sa vérification ; rien n'a été écrit".into(),
        ));
    }
    let firmware = firmware_from_mdf(&mdf);
    if permit.family() == FirmwareFamily::V2 {
        // 2.x : clés de `.mdf`, prouvées (ou confirmées) par la garde.
        return Ok((keys_from_mdf(&mdf)?, None));
    }
    let firmware = firmware.unwrap_or_default();
    let request = carrier.ok_or(CarrierError::CarrierRequired)?;
    let resolved = carrier::resolve(root, &mdf, request, cancel).map_err(|e| match e {
        CarrierError::Cancelled => TransferError::Cancelled,
        other => TransferError::Carrier(other),
    })?;
    let keys = DeviceKeys::with_carrier(&firmware, &resolved.keys).map_err(|e| match e {
        CryptoError::UnsupportedFirmware(v) => TransferError::UnsupportedFirmware(v),
        other => TransferError::KeyUnavailable(other.to_string()),
    })?;
    tracing::info!(firmware = %firmware, "Flam 1.x : clés carrier vérifiées");
    Ok((keys, Some(resolved)))
}

/// Prépare `plain_pk` dans `work_dir` pour la Flam de `root`, la transfère, puis supprime la
/// préparation. Le verrou de l'appareil est tenu par l'appelant pendant tout l'appel.
/// `carrier` : demande du mode carrier, obligatoire pour une Flam en firmware 1.x (ignorée
/// sinon).
#[allow(clippy::too_many_arguments)]
pub fn send_from_library(
    _guard: &OpGuard<'_>,
    root: &Path,
    plain_pk: &Path,
    work_dir: &Path,
    replace: Option<&str>,
    carrier: Option<&CarrierRequest<'_>>,
    ctx: &WriteContext<'_>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<TransferReport, TransferError> {
    send_from_library_with(
        &RealFs::default(),
        root,
        plain_pk,
        work_dir,
        replace,
        carrier,
        ctx,
        cancel,
        progress,
    )
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn send_from_library_with(
    fs: &dyn DeviceFs,
    root: &Path,
    plain_pk: &Path,
    work_dir: &Path,
    replace: Option<&str>,
    carrier: Option<&CarrierRequest<'_>>,
    ctx: &WriteContext<'_>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> Result<TransferReport, TransferError> {
    // Garde commune : racine, `.mdf`, firmware, clés et confiance d'une 2.x.
    let permit = check_write(root, ctx, WriteKind::Send)?;
    let (keys, resolved) = device_keys(root, &permit, carrier, cancel)?;
    if let Err(err) = prepare::cleanup_stale(work_dir) {
        tracing::warn!("préparations abandonnées non supprimées : {err}");
    }
    let prepared = prepare::prepare_story(plain_pk, &keys, work_dir, &Limits::default(), cancel)
        .map_err(|e| match e {
            PrepareError::Cancelled => TransferError::Cancelled,
            other => TransferError::Prepare(other),
        })?;
    drop(keys);
    let result = send_with(fs, &permit, root, &prepared, replace, cancel, progress);
    if let Err(err) = std::fs::remove_dir_all(&prepared.dir) {
        tracing::warn!(dossier = %prepared.dir.display(), "préparation non supprimée : {err}");
    }
    let mut report = result?;
    if let Some(resolved) = &resolved {
        // Relecture de `info` sur la Flam : se déchiffre-t-il avec le couple ?
        let ok = StoryId::parse(&report.id)
            .is_some_and(|id| carrier::readback(root, &id, &resolved.keys));
        if !ok {
            tracing::warn!(id = %report.id, "relecture de info après écriture : illisible avec les clés carrier");
        }
        report.carrier_readback = Some(ok);
        report.carrier_stored = Some(resolved.stored);
    }
    Ok(report)
}

/// Refuse une racine qui n'est pas un lecteur détecté (amovible, ou FAT fixe) : le front ne
/// doit pas pouvoir faire écrire dans un dossier quelconque qui contiendrait un `.mdf`.
/// Hors Windows, aucune détection n'existe : pas de contrôle.
pub(crate) fn ensure_detected_root(root: &Path) -> Result<(), TransferError> {
    #[cfg(windows)]
    {
        ensure_root_among(root, &super::detect::Environment::current().roots)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(())
    }
}

/// Contrôle de [`ensure_detected_root`] sur une liste de racines : comparaison sans tenir
/// compte de la casse ni du séparateur final (`d:`, `D:\`, `D:/`).
pub(crate) fn ensure_root_among(root: &Path, roots: &[PathBuf]) -> Result<(), TransferError> {
    let norm = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_ascii_lowercase()
    };
    let wanted = norm(root);
    if !wanted.is_empty() && roots.iter().any(|r| norm(r) == wanted) {
        Ok(())
    } else {
        Err(TransferError::InvalidRoot)
    }
}

/// Drapeau d'annulation du transfert en cours (un seul à la fois, sous le verrou).
static CURRENT_CANCEL: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn current_cancel() -> std::sync::MutexGuard<'static, Option<Arc<AtomicBool>>> {
    CURRENT_CANCEL
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// Charge de [`EVENT_PROGRESS`].
#[derive(Debug, Clone, Serialize)]
struct ProgressEvent<'a> {
    uuid: &'a str,
    #[serde(flatten)]
    progress: Progress,
}

/// Envoie l'histoire `uuid` de la bibliothèque sur la Flam de racine `root` (préparation puis
/// transfert sûr). Événements [`EVENT_PROGRESS`] pendant la copie et la vérification.
/// `replace` : UUID de l'histoire de la Flam à remplacer (refusé s'il ne désigne pas celle de
/// l'archive ; absent : pas de remplacement). `carrier` : source des clés pour une Flam en
/// firmware 1.x (mode carrier, consentement préalable obligatoire) ; `remember` : conserver le couple sur ce PC, chiffré (faux par défaut).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn device_send_story(
    app: tauri::AppHandle,
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    root: String,
    uuid: String,
    replace: Option<String>,
    carrier: Option<CarrierChoice>,
    remember: Option<bool>,
) -> AppResult<TransferReport> {
    let (cache_root, consent) = {
        let _guard = config.guard();
        let store = SettingsStore::new(&config.dir);
        (
            store.resolved_cache_dir(&Environment::current())?,
            store.load()?.carrier_consent,
        )
    };
    // Pas de vidage du cache pendant la préparation.
    let _gate = catalog.cache_gate().read_owned().await;
    let report = blocking(move || -> AppResult<TransferReport> {
        let guard = DEVICE_LOCK
            .try_begin(Operation::Transfer)
            .map_err(TransferError::from)?;
        // Drapeau posé dès la prise du verrou : une annulation demandée pendant la recherche de
        // l'histoire n'est pas perdue. Retiré à la sortie, y compris sur panique.
        let cancel = Arc::new(AtomicBool::new(false));
        *current_cancel() = Some(cancel.clone());
        struct ClearCancel;
        impl Drop for ClearCancel {
            fn drop(&mut self) {
                *current_cancel() = None;
            }
        }
        let _clear = ClearCancel;
        let ctx = WriteContext::current(Some(cache_root.clone()));
        let (plain_pk, _) = Library::new(&cache_root).playable(&uuid)?;
        let mut emit = |progress: Progress| {
            use tauri::Emitter;
            let _ = app.emit(
                EVENT_PROGRESS,
                ProgressEvent {
                    uuid: &uuid,
                    progress,
                },
            );
        };
        let carrier_store = CarrierStore::app();
        let request = carrier.map(|choice| CarrierRequest {
            consent: consent.as_ref(),
            choice,
            remember: remember.unwrap_or(false),
            store: &carrier_store,
        });
        let result = send_from_library(
            &guard,
            Path::new(&root),
            &plain_pk,
            &cache_root.join(WORK_SUBDIR),
            replace.as_deref(),
            request.as_ref(),
            &ctx,
            &cancel,
            &mut emit,
        );
        Ok(result?)
    })
    .await?;
    // Diagnostic copiable (#318) : erreurs de la session, sans les annulations.
    match &report {
        Ok(report) => {
            for failure in &report.recovery_failures {
                super::diagnostic::record_error("reprise avant envoi", failure);
            }
        }
        Err(AppError::Cancelled) => {}
        Err(err) => super::diagnostic::record_error("envoi", &err.to_string()),
    }
    report
}

/// Mode carrier : histoire de la Flam de `root` reconnue par le `bt` saisi, montrée avant tout
/// envoi pour que l'utilisateur vérifie son titre (un IV faux en altère le début). Lecture
/// seule : consentement exigé, verrou de l'appareil tenu, annulable par
/// [`device_cancel_transfer`]. La réponse ne contient aucune clé.
#[tauri::command]
pub async fn device_carrier_preview(
    config: State<'_, ConfigState>,
    root: String,
    bt: EnteredBt,
) -> AppResult<EnteredPreview> {
    let consent = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).load()?.carrier_consent
    };
    blocking(move || -> AppResult<EnteredPreview> {
        let _guard = DEVICE_LOCK
            .try_begin(Operation::Transfer)
            .map_err(TransferError::from)?;
        let root = Path::new(&root);
        ensure_detected_root(root)?;
        if !root.join(MDF_FILE).is_file() {
            return Err(TransferError::DeviceNotFound.into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *current_cancel() = Some(cancel.clone());
        struct ClearCancel;
        impl Drop for ClearCancel {
            fn drop(&mut self) {
                *current_cancel() = None;
            }
        }
        let _clear = ClearCancel;
        carrier::preview_entered(root, consent.as_ref(), &bt, &cancel).map_err(|e| {
            AppError::from(match e {
                CarrierError::Cancelled => TransferError::Cancelled,
                other => TransferError::Carrier(other),
            })
        })
    })
    .await?
}

/// Reprend sur la Flam de racine `root` les transferts puis les écritures d'index
/// interrompus (voir le tableau du module). À appeler à chaque détection de l'appareil, avant
/// de lire son contenu ; refusé (`Busy`) pendant une autre opération.
pub fn recover_device(
    guard: &OpGuard<'_>,
    root: &Path,
    ctx: &WriteContext<'_>,
) -> Result<TransferRecovery, TransferError> {
    let permit = check_write(root, ctx, WriteKind::Other)?;
    let report = recover(guard, &permit, root)?;
    for kind in IndexKind::ALL {
        index::recover(root, kind)?;
    }
    Ok(report)
}

/// Reprise au branchement : transferts et index interrompus ([`recover_device`]). Refusée,
/// comme toute écriture, si la garde du firmware refuse ([`check_write`]).
#[tauri::command]
pub async fn device_recover_transfers(
    config: State<'_, ConfigState>,
    root: String,
) -> AppResult<TransferRecovery> {
    let cache_root = launcher_cache_root(&config);
    let result = blocking(move || -> AppResult<TransferRecovery> {
        let guard = DEVICE_LOCK
            .try_begin(Operation::Transfer)
            .map_err(TransferError::from)?;
        let ctx = WriteContext::current(cache_root);
        Ok(recover_device(&guard, Path::new(&root), &ctx)?)
    })
    .await?;
    if let Err(err) = &result {
        super::diagnostic::record_error("reprise", &err.to_string());
    }
    result
}

/// Dossier de cache du launcher (bibliothèque), pour reconnaître ses propres histoires dans la
/// garde des clés 2.x ; `None` (journalisé) s'il est illisible.
pub(crate) fn launcher_cache_root(config: &ConfigState) -> Option<PathBuf> {
    let _guard = config.guard();
    SettingsStore::new(&config.dir)
        .resolved_cache_dir(&Environment::current())
        .map_err(|err| {
            tracing::warn!("dossier de cache illisible pour la garde de la Flam : {err}")
        })
        .ok()
}

/// Demande l'annulation du transfert en cours. Rend faux s'il n'y en a pas. Sans effet une fois
/// la nouvelle version vérifiée (renommage et index vont alors jusqu'au bout).
#[tauri::command]
pub async fn device_cancel_transfer() -> AppResult<bool> {
    Ok(match current_cancel().as_ref() {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    })
}

#[cfg(test)]
mod tests;
