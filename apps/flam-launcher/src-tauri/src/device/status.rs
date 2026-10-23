//! Commandes en lecture seule de l'écran « Ma Flam » (#227) : état de l'appareil et liste des
//! histoires installées.
//!
//! Rien n'est écrit sur la Flam : ni reprise d'index, ni création de dossier. Le seul fichier
//! écrit est la sauvegarde personnelle de `.mdf`, dans le dossier de données du launcher
//! (voir [`super::detect::backup_mdf`]). Le titre renvoyé ici est celui de la bibliothèque du
//! launcher, associé à l'histoire de la Flam par l'UUID de `uuid.bin` de son `.plain.pk` (celui
//! qui nomme `str/<uuid>/`, #276). Le titre déchiffré de `info` et la vignette de la Flam sont
//! lus ensuite, à la demande, par [`super::details`] ; à défaut, l'interface affiche l'UUID
//! court.
//!
//! La logique ne dépend pas de Tauri : l'environnement de détection et le dossier de
//! sauvegarde sont passés en paramètre pour tester sur un faux appareil.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use serde::Serialize;
use tauri::State;

use super::detect::{self, DetectedDevice, DeviceWarning, Environment, MdfError, Space};
use super::guard::{self, FirmwareStatus, WriteContext};
use super::index::{self, IndexKind, IndexState};
use super::lock::DEVICE_LOCK;
use crate::catalog::blocking;
use crate::install::InstallService;
use crate::library::{Library, LibraryEntry};
use crate::settings::SettingsStore;
use crate::storage::ConfigState;
use crate::AppResult;

/// Longueur de l'UUID court affiché quand le titre est inconnu.
const SHORT_ID_LEN: usize = 8;
/// Nombre maximal d'histoires renvoyées : au-delà, la liste est tronquée (et signalée).
pub const MAX_STORIES: usize = 5000;

/// Vrai tant que l'échec de sauvegarde de `.mdf` a déjà été journalisé : un seul message par
/// épisode, pas un toutes les 2 s.
static BACKUP_WARNED: AtomicBool = AtomicBool::new(false);

/// Résumé de l'appareil détecté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceSummary {
    /// Lettre ou chemin du volume (`E:\`).
    pub root: String,
    /// Nom du volume (vide s'il est illisible).
    pub volume_label: String,
    /// Système de fichiers (vide s'il est illisible).
    pub file_system: String,
    pub space: Option<Space>,
    pub warnings: Vec<DeviceWarning>,
    /// Problème de lecture de `.mdf` (tronqué, démesuré). Un format simplement non spécifié
    /// n'est pas un problème.
    pub mdf_problem: Option<MdfError>,
    /// La sauvegarde personnelle de `.mdf` a échoué (dossier de données inaccessible).
    pub backup_failed: bool,
    /// Version du firmware lue dans `.mdf` (`1.15.14`, `2.4.2`…), `None` si illisible. Une
    /// Flam en 1.x n'accepte que le mode carrier (#234).
    pub firmware: Option<String>,
    /// État du firmware et de la garde des écritures (#317) : famille, disposition, état des
    /// clés, raison d'un refus d'écrire. Aucune clé, aucun numéro de série.
    pub firmware_status: FirmwareStatus,
}

/// État de l'appareil (réponse de `device_status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceStatus {
    pub detected: bool,
    /// Message d'aide quand aucune Flam n'est détectée.
    pub hint: Option<String>,
    /// Première Flam détectée.
    pub device: Option<DeviceSummary>,
    /// Nombre de Flam détectées (seule la première est affichée).
    pub device_count: usize,
}

/// Lecture d'un index, sans détail des lignes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexHealth {
    Valid,
    Absent,
    /// Lisible, mais des lignes invalides ou en double ont été ignorées.
    Damaged,
    Unreadable,
}

impl IndexHealth {
    fn of(state: &IndexState) -> Self {
        match state {
            IndexState::Valid => Self::Valid,
            IndexState::Absent => Self::Absent,
            IndexState::Damaged => Self::Damaged,
            IndexState::Unreadable(_) => Self::Unreadable,
        }
    }
}

/// Place d'une histoire sur l'appareil.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Dans `etc/library/list`.
    Visible,
    /// Dans `etc/library/list.hidden`.
    Hidden,
    /// Dossier `str/<uuid>` absent des deux index.
    Unlisted,
}

/// Une histoire de la Flam.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceStory {
    pub uuid: String,
    /// Huit premiers caractères de l'UUID.
    pub short_id: String,
    pub placement: Placement,
    /// Titre de l'histoire dans la bibliothèque du launcher, s'il y figure.
    pub title: Option<String>,
    /// L'histoire figure dans la bibliothèque du launcher (vignette disponible).
    pub in_library: bool,
    /// Identifiants de bibliothèque des histoires installées sous cet UUID (`uuid.bin` de leur
    /// archive) : l'interface reconnaît ainsi une histoire d'une source dossier ou Telmi, dont
    /// l'identifiant de bibliothèque n'est pas l'UUID de la Flam.
    pub library_uuids: Vec<String>,
    /// Listée dans `list` mais sans dossier `str/<uuid>` (index qui pointe dans le vide).
    pub missing: bool,
}

/// Histoires de l'appareil (réponse de `device_list_stories`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoriesReport {
    /// Visibles dans l'ordre de l'index, puis masquées, puis non listées.
    pub stories: Vec<DeviceStory>,
    pub visible_index: IndexHealth,
    pub hidden_index: IndexHealth,
    /// La liste a été coupée à `limit` histoires.
    pub truncated: bool,
    pub limit: usize,
    /// `str/` illisible (absent ou erreur d'entrée/sortie) : seuls les index sont affichés.
    pub str_unreadable: bool,
}

fn summarize(
    device: &DetectedDevice,
    backup_failed: bool,
    ctx: &WriteContext<'_>,
) -> DeviceSummary {
    let firmware_status = match device.mdf_bytes.as_deref() {
        Some(mdf) => {
            guard::firmware_status(&device.root, mdf, ctx, DEVICE_LOCK.current().is_some())
        }
        None => guard::unreadable_status(
            device.mdf_size,
            device
                .mdf_error
                .as_ref()
                .map_or_else(|| "illisible".to_string(), ToString::to_string),
        ),
    };
    DeviceSummary {
        root: device.root.display().to_string(),
        volume_label: device.volume_label.clone(),
        file_system: device.file_system.clone(),
        space: device.space,
        warnings: device.warnings.clone(),
        mdf_problem: device
            .mdf_error
            .clone()
            .filter(|e| !matches!(e, MdfError::FormatUnspecified)),
        backup_failed,
        firmware: device
            .mdf_bytes
            .as_deref()
            .and_then(super::transfer::firmware_from_mdf),
        firmware_status,
    }
}

/// État de l'appareil : détection, puis sauvegarde personnelle de `.mdf` dans `backup_dir`
/// (idempotente, jamais bloquante), et état du firmware ([`guard::firmware_status`], preuve
/// des clés 2.x en lecture seule). Aucune écriture sur la Flam.
pub fn status(env: &Environment, backup_dir: &Path, ctx: &WriteContext<'_>) -> DeviceStatus {
    let report = detect::detect(env);
    let Some(first) = report.devices.first() else {
        return DeviceStatus {
            detected: false,
            hint: report.hint,
            device: None,
            device_count: 0,
        };
    };
    let backup_failed = detect::backup_mdf(backup_dir, first).is_err();
    if !backup_failed {
        BACKUP_WARNED.store(false, Ordering::Relaxed);
    } else if !BACKUP_WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            "sauvegarde de .mdf impossible dans {}",
            backup_dir.display()
        );
    }
    DeviceStatus {
        detected: true,
        hint: None,
        device: Some(summarize(first, backup_failed, ctx)),
        device_count: report.devices.len(),
    }
}

/// Histoires de la bibliothèque installées sous un même UUID de la Flam.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibraryMatch {
    /// Titre de la bibliothèque.
    pub title: String,
    /// Identifiants de bibliothèque (`LibraryEntry::uuid`), dans l'ordre de la bibliothèque.
    pub library_uuids: Vec<String>,
}

/// Histoires de la première Flam détectée (`None` : aucune Flam). `titles` associe un UUID en
/// minuscules aux histoires de la bibliothèque du launcher. Lecture seule : l'index est lu tel
/// quel, sans reprise.
pub fn list_stories(
    env: &Environment,
    titles: &HashMap<String, LibraryMatch>,
) -> Option<StoriesReport> {
    let report = detect::detect(env);
    let device = report.devices.first()?;
    Some(read_stories(&device.root, titles))
}

fn read_stories(root: &Path, titles: &HashMap<String, LibraryMatch>) -> StoriesReport {
    let visible = index::read_index(root, IndexKind::Visible);
    let hidden = index::read_index(root, IndexKind::Hidden);
    let missing: HashSet<_> = index::missing_stories(root, &visible.entries)
        .into_iter()
        .collect();
    let (folders, str_unreadable) = match index::scan_stories(root) {
        Ok(ids) => (ids, false),
        Err(_) => (Vec::new(), true),
    };

    let mut seen = HashSet::new();
    let mut stories = Vec::new();
    let mut push = |id: &index::StoryId, placement: Placement, missing: bool| {
        if !seen.insert(id.clone()) {
            return;
        }
        let uuid = id.as_str().to_string();
        let found = titles.get(&uuid);
        stories.push(DeviceStory {
            short_id: uuid.chars().take(SHORT_ID_LEN).collect(),
            in_library: found.is_some(),
            title: found.map(|m| m.title.clone()),
            library_uuids: found.map(|m| m.library_uuids.clone()).unwrap_or_default(),
            uuid,
            placement,
            missing,
        });
    };
    for id in &visible.entries {
        push(id, Placement::Visible, missing.contains(id));
    }
    for id in &hidden.entries {
        push(id, Placement::Hidden, false);
    }
    for id in &folders {
        push(id, Placement::Unlisted, false);
    }

    let truncated = stories.len() > MAX_STORIES;
    stories.truncate(MAX_STORIES);
    StoriesReport {
        stories,
        truncated,
        limit: MAX_STORIES,
        visible_index: IndexHealth::of(&visible.state),
        hidden_index: IndexHealth::of(&hidden.state),
        str_unreadable,
    }
}

/// Détecte la Flam et sauvegarde son `.mdf` (lecture seule sur l'appareil).
#[tauri::command]
pub async fn device_status(config: State<'_, ConfigState>) -> AppResult<DeviceStatus> {
    let backup_dir = crate::storage::data_dir().join(detect::BACKUP_SUBDIR);
    // Scrutée : dossier de cache résolu à neuf seulement si les réglages ont changé (#326).
    let cache_root = super::transfer::polled_cache_root(&config);
    blocking(move || {
        status(
            &Environment::current(),
            &backup_dir,
            &WriteContext::current(cache_root),
        )
    })
    .await
}

/// UUID (forme à tirets, minuscules) sur la Flam des histoires de la bibliothèque du launcher
/// de `cache_root` : ce sont celles qu'il a pu installer, exclues du contrôle de cohérence
/// ([`guard::coherence_v2`]) et seules cherchées dans `library.cache`
/// ([`guard::flam_cache_proof`]). Vide (journalisé) si la bibliothèque est illisible.
pub(crate) fn launcher_story_ids(cache_root: Option<&Path>) -> HashSet<String> {
    let Some(root) = cache_root else {
        return HashSet::new();
    };
    match Library::new(root).list() {
        Ok(entries) => library_titles(Some(root), entries).into_keys().collect(),
        Err(err) => {
            tracing::warn!(
                "bibliothèque illisible pour la vérification des clés de la Flam : {err}"
            );
            HashSet::new()
        }
    }
}

/// Confirmation « Firmware 2.x en test : j'ai sauvegardé ma Flam » ([`guard::confirm`]),
/// mémorisée sur le PC par empreinte de `.mdf`. `understood` : case cochée dans l'interface
/// (sinon refus). Rend l'état du firmware à jour. Aucune écriture sur la Flam.
#[tauri::command]
pub async fn device_confirm_firmware(
    config: State<'_, ConfigState>,
    root: String,
    understood: bool,
) -> AppResult<FirmwareStatus> {
    if !understood {
        return Err(crate::AppError::Validation(
            "cochez « J'ai sauvegardé ma Flam » pour confirmer".into(),
        ));
    }
    let cache_root = super::transfer::launcher_cache_root(&config);
    blocking(move || {
        guard::confirm(Path::new(&root), &WriteContext::current(cache_root))
            .map_err(|err| crate::AppError::Validation(err.to_string()))
    })
    .await?
}

/// UUID d'une histoire installée par le launcher, tel que lu dans `uuid.bin` de `archive`
/// (borné à 16 octets) : c'est lui qui nomme `str/<uuid>/` sur la Flam. `None` si l'archive
/// ou `uuid.bin` est illisible.
fn uuid_bin_of(archive: &Path) -> Option<String> {
    let file = std::fs::File::open(archive).ok()?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).ok()?;
    let entry = zip.by_name("uuid.bin").ok()?;
    let mut bytes = Vec::with_capacity(16);
    entry.take(17).read_to_end(&mut bytes).ok()?;
    let uuid = uuid::Uuid::from_slice(&bytes).ok()?;
    Some(uuid.hyphenated().to_string())
}

/// Clé de [`DEVICE_UUIDS`] : fichier, taille et date d'installation (une réinstallation change
/// la clé).
type ArchiveKey = (String, u64, u64);

/// UUID sur la Flam des archives déjà ouvertes (la liste est relue à chaque rafraîchissement).
static DEVICE_UUIDS: Mutex<Option<HashMap<ArchiveKey, Option<String>>>> = Mutex::new(None);

/// UUID sur la Flam de l'histoire de la bibliothèque `entry` (archive sous `cache_root`) :
/// celui de `uuid.bin` ; à défaut, `entry.uuid` s'il s'écrit comme un UUID (forme simple de
/// 32 caractères d'une source dossier, ou avec tirets). Une histoire Telmi a un identifiant de
/// source sans rapport avec `uuid.bin` : seul ce dernier permet de la retrouver sur la Flam.
fn device_uuid_of(cache_root: Option<&Path>, entry: &LibraryEntry) -> Option<String> {
    let from_archive = cache_root.and_then(|root| archive_uuid(root, entry));
    from_archive.or_else(|| {
        uuid::Uuid::try_parse(&entry.uuid)
            .ok()
            .map(|u| u.hyphenated().to_string())
    })
}

/// UUID de `uuid.bin` de l'archive de `entry`, mémorisé.
fn archive_uuid(cache_root: &Path, entry: &LibraryEntry) -> Option<String> {
    let key = (entry.file.clone(), entry.size, entry.installed_at);
    let cached = DEVICE_UUIDS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(&key).cloned());
    match cached {
        Some(found) => found,
        None => {
            let found = Library::new(cache_root)
                .checked(&entry.file)
                .ok()
                .and_then(|path| uuid_bin_of(&path));
            // Un échec peut être passager (archive ouverte ailleurs) : seul un succès est
            // mémorisé.
            if found.is_some() {
                DEVICE_UUIDS
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get_or_insert_with(HashMap::new)
                    .insert(key, found.clone());
            }
            found
        }
    }
}

/// Histoires de la bibliothèque, par UUID de l'histoire sur la Flam ([`device_uuid_of`]). Si
/// plusieurs entrées donnent le même UUID, le titre est celui de la dernière (comme avant) et
/// tous les identifiants sont gardés.
fn library_titles(
    cache_root: Option<&Path>,
    entries: Vec<LibraryEntry>,
) -> HashMap<String, LibraryMatch> {
    let mut found: HashMap<String, LibraryMatch> = HashMap::new();
    for entry in entries {
        let Some(device_uuid) = device_uuid_of(cache_root, &entry) else {
            continue;
        };
        let slot = found.entry(device_uuid).or_default();
        slot.title = entry.title;
        if !slot.library_uuids.contains(&entry.uuid) {
            slot.library_uuids.push(entry.uuid);
        }
    }
    found
}

/// Histoires installées sur la Flam (`None` : aucune Flam détectée).
#[tauri::command]
pub async fn device_list_stories(
    service: State<'_, InstallService>,
    config: State<'_, ConfigState>,
) -> AppResult<Option<StoriesReport>> {
    // Une bibliothèque illisible ne doit pas empêcher d'afficher l'appareil : les titres
    // manquent simplement.
    let entries = match service.library_list().await {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("bibliothèque illisible pour « Ma Flam » : {err}");
            Vec::new()
        }
    };
    let cache_root = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&crate::settings::Environment::current())
    };
    blocking(move || {
        let cache_root = cache_root
            .map_err(|err| tracing::warn!("dossier de cache illisible pour « Ma Flam » : {err}"))
            .ok();
        let titles = library_titles(cache_root.as_deref(), entries);
        list_stories(&Environment::current(), &titles)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::detect::VolumeInfo;
    use crate::device::guard::tests::test_ctx;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::path::PathBuf;

    const A: &str = "11111111-1111-4111-8111-111111111111";
    const B: &str = "22222222-2222-4222-8222-222222222222";
    const C: &str = "33333333-3333-4333-8333-333333333333";
    const D: &str = "44444444-4444-4444-8444-444444444444";

    fn fake_volume(_: &Path) -> Option<VolumeInfo> {
        Some(VolumeInfo {
            label: "FLAM".into(),
            fs: "FAT32".into(),
        })
    }

    fn fake_space(_: &Path) -> Option<Space> {
        Some(Space {
            free: 100,
            total: 1000,
        })
    }

    fn env(roots: Vec<PathBuf>) -> Environment {
        Environment {
            roots,
            volume_info: fake_volume,
            space: fake_space,
        }
    }

    /// Faux appareil : `.mdf` synthétique de 126 octets, `str/`, `etc/library/`.
    fn fake_device() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let mdf: Vec<u8> = (0..detect::MDF_DOCUMENTED_SIZE as u8).collect();
        fs::write(dir.path().join(detect::MDF_FILE), mdf).unwrap();
        fs::create_dir_all(dir.path().join("etc/library")).unwrap();
        fs::create_dir(dir.path().join("str")).unwrap();
        dir
    }

    fn put(root: &Path, relative: &str, content: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    /// Empreinte de toute l'arborescence : chemins relatifs, types et octets.
    fn fingerprint(root: &Path) -> String {
        fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
            let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                let rel = path.strip_prefix(root).unwrap().display().to_string();
                if path.is_dir() {
                    out.push(format!("D {rel}"));
                    walk(&path, root, out);
                } else {
                    let hash = Sha256::digest(fs::read(&path).unwrap());
                    out.push(format!("F {rel} {hash:x}"));
                }
            }
        }
        let mut lines = Vec::new();
        walk(root, root, &mut lines);
        lines.join("\n")
    }

    fn installed_device() -> tempfile::TempDir {
        let dev = fake_device();
        put(dev.path(), "etc/library/list", &format!("{A}\n{B}\n"));
        put(dev.path(), "etc/library/list.hidden", &format!("{C}\n"));
        put(dev.path(), &format!("str/{A}/info"), "x");
        put(dev.path(), &format!("str/{D}/info"), "x");
        dev
    }

    #[test]
    fn sans_appareil_l_etat_est_non_detecte_avec_une_aide() {
        let backups = tempfile::tempdir().unwrap();
        let empty = tempfile::tempdir().unwrap();
        let status = status(
            &env(vec![empty.path().to_path_buf()]),
            backups.path(),
            &test_ctx(),
        );
        assert!(!status.detected);
        assert_eq!(status.hint.as_deref(), Some(detect::NOT_DETECTED_HINT));
        assert!(status.device.is_none());
        assert_eq!(status.device_count, 0);
        assert!(list_stories(&env(vec![empty.path().to_path_buf()]), &HashMap::new()).is_none());
    }

    #[test]
    fn un_appareil_detecte_est_resume_et_son_mdf_sauvegarde() {
        let dev = fake_device();
        let backups = tempfile::tempdir().unwrap();
        let status = status(
            &env(vec![dev.path().to_path_buf()]),
            backups.path(),
            &test_ctx(),
        );
        assert!(status.detected);
        assert_eq!(status.device_count, 1);
        let device = status.device.unwrap();
        assert_eq!(device.volume_label, "FLAM");
        assert_eq!(device.file_system, "FAT32");
        assert_eq!(
            device.space,
            Some(Space {
                free: 100,
                total: 1000
            })
        );
        assert!(device.warnings.is_empty());
        assert_eq!(
            device.mdf_problem, None,
            "format non spécifié : pas un problème"
        );
        assert!(!device.backup_failed);
        assert_eq!(fs::read_dir(backups.path()).unwrap().count(), 1);
    }

    #[test]
    fn un_mdf_tronque_est_signale_et_non_sauvegarde() {
        let dev = fake_device();
        fs::write(dev.path().join(detect::MDF_FILE), b"court").unwrap();
        let backups = tempfile::tempdir().unwrap();
        let status = status(
            &env(vec![dev.path().to_path_buf()]),
            backups.path(),
            &test_ctx(),
        );
        let device = status.device.unwrap();
        assert!(matches!(
            device.mdf_problem,
            Some(MdfError::Truncated { .. })
        ));
        assert_eq!(fs::read_dir(backups.path()).unwrap().count(), 0);
    }

    #[test]
    fn une_sauvegarde_impossible_n_empeche_pas_l_etat() {
        let dev = fake_device();
        let blocker = tempfile::NamedTempFile::new().unwrap();
        // Un fichier à la place du dossier de sauvegarde : l'écriture échoue.
        let status = status(
            &env(vec![dev.path().to_path_buf()]),
            blocker.path(),
            &test_ctx(),
        );
        assert!(status.detected);
        assert!(status.device.unwrap().backup_failed);
    }

    #[test]
    fn les_avertissements_de_detection_sont_remontes() {
        let dev = fake_device();
        fs::remove_dir(dev.path().join("str")).unwrap();
        let backups = tempfile::tempdir().unwrap();
        let status = status(
            &env(vec![dev.path().to_path_buf()]),
            backups.path(),
            &test_ctx(),
        );
        assert_eq!(
            status.device.unwrap().warnings,
            vec![DeviceWarning::MissingStrDir]
        );
    }

    #[test]
    fn les_histoires_sont_visibles_masquees_ou_non_listees() {
        let dev = installed_device();
        let titles = HashMap::from([(
            A.to_string(),
            LibraryMatch {
                title: "Capi".into(),
                library_uuids: vec!["capi-bib".into()],
            },
        )]);
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &titles).unwrap();
        let summary: Vec<_> = report
            .stories
            .iter()
            .map(|s| (s.uuid.as_str(), s.placement, s.missing, s.in_library))
            .collect();
        assert_eq!(
            summary,
            vec![
                (A, Placement::Visible, false, true),
                (B, Placement::Visible, true, false),
                (C, Placement::Hidden, false, false),
                (D, Placement::Unlisted, false, false),
            ]
        );
        assert_eq!(report.stories[0].title.as_deref(), Some("Capi"));
        assert_eq!(report.stories[0].library_uuids, ["capi-bib"]);
        assert_eq!(report.stories[1].title, None);
        assert!(report.stories[1].library_uuids.is_empty());
        assert_eq!(report.stories[1].short_id, "22222222");
        assert_eq!(report.visible_index, IndexHealth::Valid);
        assert_eq!(report.hidden_index, IndexHealth::Valid);
        assert!(!report.str_unreadable);
    }

    #[test]
    fn une_liste_demesuree_est_tronquee_et_signalee() {
        let dev = fake_device();
        let list: String = (0..MAX_STORIES + 10)
            .map(|i| {
                format!(
                    "00000000-0000-4000-8000-{i:012x}
"
                )
            })
            .collect();
        put(dev.path(), "etc/library/list", &list);
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &HashMap::new()).unwrap();
        assert_eq!(report.stories.len(), MAX_STORIES);
        assert!(report.truncated);
        assert_eq!(report.limit, MAX_STORIES);
        assert_eq!(
            report.stories[0].uuid,
            "00000000-0000-4000-8000-000000000000"
        );
    }

    #[test]
    fn un_uuid_dans_les_deux_index_n_apparait_qu_une_fois() {
        let dev = fake_device();
        put(dev.path(), "etc/library/list", &format!("{A}\n"));
        put(dev.path(), "etc/library/list.hidden", &format!("{A}\n"));
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &HashMap::new()).unwrap();
        assert_eq!(report.stories.len(), 1);
        assert_eq!(report.stories[0].placement, Placement::Visible);
    }

    #[test]
    fn un_index_illisible_est_signale_et_les_dossiers_restent_visibles() {
        let dev = fake_device();
        put(dev.path(), "etc/library/list", "pas un uuid\n");
        put(dev.path(), &format!("str/{A}/info"), "x");
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &HashMap::new()).unwrap();
        assert_eq!(report.visible_index, IndexHealth::Unreadable);
        assert_eq!(report.hidden_index, IndexHealth::Absent);
        assert_eq!(report.stories.len(), 1);
        assert_eq!(report.stories[0].placement, Placement::Unlisted);
    }

    #[test]
    fn un_index_abime_garde_les_lignes_valides() {
        let dev = fake_device();
        put(dev.path(), "etc/library/list", &format!("{A}\nzzz\n{A}\n"));
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &HashMap::new()).unwrap();
        assert_eq!(report.visible_index, IndexHealth::Damaged);
        assert_eq!(report.stories.len(), 1);
    }

    #[test]
    fn sans_dossier_str_l_etat_est_signale() {
        let dev = fake_device();
        fs::remove_dir(dev.path().join("str")).unwrap();
        put(dev.path(), "etc/library/list", &format!("{A}\n"));
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &HashMap::new()).unwrap();
        assert!(report.str_unreadable);
        assert!(report.stories[0].missing);
    }

    #[test]
    fn la_serialisation_n_expose_que_les_champs_prevus() {
        let dev = installed_device();
        let backups = tempfile::tempdir().unwrap();
        let json = serde_json::to_string(&status(
            &env(vec![dev.path().to_path_buf()]),
            backups.path(),
            &test_ctx(),
        ))
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let mut keys: Vec<_> = value["device"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        assert_eq!(
            keys,
            [
                "backup_failed",
                "file_system",
                "firmware",
                "firmware_status",
                "mdf_problem",
                "root",
                "space",
                "volume_label",
                "warnings"
            ]
        );
    }

    fn library_entry(uuid: &str, file: &str, title: &str) -> LibraryEntry {
        LibraryEntry {
            uuid: uuid.into(),
            version: None,
            title: title.into(),
            file: file.into(),
            size: 1,
            installed_at: 1,
            converter: None,
            source: crate::story::StoryEntry {
                source_id: "s".into(),
                entry_id: format!("s:{uuid}"),
                uuid: uuid.into(),
                version: None,
                title: title.into(),
                description: None,
                age: None,
                category: None,
                author: None,
                voice: None,
                license: None,
                thumb_url: None,
                download_url: String::new(),
                size: None,
                updated_at: None,
                download_count: None,
                kind: crate::story::StoryKind::FlamPlain,
            },
            thumb: None,
            source_stamp: None,
        }
    }

    /// `.plain.pk` minimal contenant `uuid.bin`.
    fn archive_with_uuid(path: &Path, uuid_bin: &[u8]) {
        let mut zip = zip::ZipWriter::new(fs::File::create(path).unwrap());
        zip.start_file("uuid.bin", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, uuid_bin).unwrap();
        zip.finish().unwrap();
    }

    /// #276 : une histoire d'une source dossier (UUID de bibliothèque sans tirets) ou Telmi
    /// (identifiant de source sans rapport) est retrouvée sur la Flam par `uuid.bin`.
    #[test]
    fn les_titres_de_la_bibliotheque_suivent_uuid_bin() {
        let cache = tempfile::tempdir().unwrap();
        fs::create_dir(cache.path().join("library")).unwrap();
        let capi: [u8; 16] = [
            0xa7, 0x01, 0xe2, 0x7e, 0x8f, 0x3a, 0x24, 0x8b, 0x72, 0xbd, 0xff, 0x65, 0xfa, 0x5d,
            0x44, 0x0e,
        ];
        archive_with_uuid(&cache.path().join("library/telmi-1.plain.pk"), &[0x11; 16]);
        archive_with_uuid(&cache.path().join("library/bad-1.plain.pk"), &[1, 2, 3]);
        let entries = vec![
            // Source dossier, archive disparue : UUID de la bibliothèque, forme simple.
            library_entry(
                "a701e27e8f3a248b72bdff65fa5d440e",
                "absent-0.plain.pk",
                "Capi",
            ),
            // Telmi : identifiant de source, `uuid.bin` fait foi.
            library_entry("ffffff-199a63526c4", "telmi-1.plain.pk", "Égypte"),
            // `uuid.bin` invalide et identifiant non UUID : ignorée.
            library_entry("zz", "bad-1.plain.pk", "Perdue"),
            // Nom de fichier hors de la bibliothèque : jamais ouvert.
            library_entry("zz2", "../evil.plain.pk", "Hors"),
        ];
        let titles = library_titles(Some(cache.path()), entries.clone());
        let capi_uuid = uuid::Uuid::from_bytes(capi).hyphenated().to_string();
        assert_eq!(capi_uuid, "a701e27e-8f3a-248b-72bd-ff65fa5d440e");
        assert_eq!(titles[&capi_uuid].title, "Capi");
        assert_eq!(
            titles[&capi_uuid].library_uuids,
            ["a701e27e8f3a248b72bdff65fa5d440e"]
        );
        let egypte = &titles["11111111-1111-1111-1111-111111111111"];
        assert_eq!(egypte.title, "Égypte");
        assert_eq!(egypte.library_uuids, ["ffffff-199a63526c4"]);
        assert_eq!(titles.len(), 2);
        // Sans dossier de cache : repli sur l'UUID de la bibliothèque.
        assert_eq!(library_titles(None, entries).len(), 1);
        // Liste de la Flam : le titre de Capi est trouvé.
        let dev = fake_device();
        put(
            dev.path(),
            "etc/library/list",
            &format!(
                "{capi_uuid}
"
            ),
        );
        put(dev.path(), &format!("str/{capi_uuid}/info"), "x");
        let report = list_stories(&env(vec![dev.path().to_path_buf()]), &titles).unwrap();
        assert_eq!(report.stories[0].title.as_deref(), Some("Capi"));
        assert!(report.stories[0].in_library);
        // L'interface retrouve l'histoire de bibliothèque (forme simple) sur la Flam.
        assert_eq!(
            report.stories[0].library_uuids,
            ["a701e27e8f3a248b72bdff65fa5d440e"]
        );
    }

    /// Critère d'acceptation : après état et liste, l'arborescence et les octets de la Flam
    /// sont identiques (aucune reprise d'index, aucune écriture).
    #[test]
    fn l_appareil_est_identique_apres_lecture() {
        let dev = installed_device();
        // Une écriture interrompue : la lecture ne doit pas la « réparer ».
        put(
            dev.path(),
            "etc/library/list.new",
            &format!("{A}\n{B}\n{C}\n"),
        );
        put(dev.path(), "etc/library/list.bak", &format!("{A}\n"));
        put(dev.path(), "usr/0/library.cache", "cache");
        let before = fingerprint(dev.path());
        let backups = tempfile::tempdir().unwrap();
        let environment = env(vec![dev.path().to_path_buf()]);
        for _ in 0..3 {
            let _ = status(&environment, backups.path(), &test_ctx());
            let _ = list_stories(&environment, &HashMap::new());
        }
        assert_eq!(fingerprint(dev.path()), before);
    }
}
