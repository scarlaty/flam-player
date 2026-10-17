//! Détection d'une Flam en « mode connexion USB » (#225).
//!
//! Dans ce mode, la Flam apparaît comme un volume FAT dont la racine contient le fichier
//! `.mdf` (126 octets sur l'appareil inventorié, voir `flam-device-inventory.md`). Ce fichier
//! est le marqueur de l'appareil ; `str/` (histoires) et `etc/library/` (index) sont attendus
//! mais leur absence n'est qu'un avertissement.
//!
//! Le format interne de `.mdf` n'est **pas encore spécifié** (#224) : [`parse_mdf`] renvoie
//! [`MdfError::FormatUnspecified`] et ne lit aucun champ. Seuls la taille et l'empreinte
//! SHA-256 du contenu sont exposées.
//!
//! La logique est sans Tauri et sans accès direct au système : les racines candidates, le nom
//! de volume et l'espace disque passent par [`Environment`] (même modèle que
//! `settings::Environment`), ce qui permet de tester sur des dossiers temporaires. Tout est en
//! lecture seule sur le volume ; le seul octet de `.mdf` écrit ailleurs est la sauvegarde
//! personnelle de [`backup_mdf`], dans le dossier de données de l'utilisateur.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::catalog::blocking;
use crate::AppResult;

/// Nom du fichier marqueur, à la racine du volume.
pub const MDF_FILE: &str = ".mdf";
/// Taille observée de `.mdf` (inventaire du 2026-06-14). En dessous : fichier tronqué.
pub const MDF_DOCUMENTED_SIZE: u64 = 126;
/// Taille maximale lue : au-delà, le fichier n'est ni lu ni sauvegardé.
pub const MDF_MAX_SIZE: u64 = 64 * 1024;
/// Message affiché quand aucun volume n'est trouvé.
pub const NOT_DETECTED_HINT: &str = "Activez le mode connexion USB sur la Flam";
/// Sous-dossier des sauvegardes dans le dossier de données de l'application.
pub const BACKUP_SUBDIR: &str = "device-backups";

/// Espace disque d'un volume, en octets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Space {
    pub free: u64,
    pub total: u64,
}

/// Nom et système de fichiers d'un volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub label: String,
    pub fs: String,
}

/// Environnement de détection, injectable.
#[derive(Clone)]
pub struct Environment {
    /// Racines candidates (lecteurs amovibles ou FAT), déjà filtrées.
    pub roots: Vec<PathBuf>,
    /// Nom et système de fichiers d'une racine (`None` : volume illisible ou retiré).
    pub volume_info: fn(&Path) -> Option<VolumeInfo>,
    /// Espace libre et total d'une racine (`None` : indisponible).
    pub space: fn(&Path) -> Option<Space>,
}

impl Environment {
    /// Environnement réel : lecteurs logiques Windows amovibles ou FAT (vide hors Windows).
    pub fn current() -> Self {
        Self {
            roots: sys::candidate_roots(),
            volume_info: sys::volume_info,
            space: sys::disk_space,
        }
    }
}

/// Avertissement non bloquant sur un appareil détecté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceWarning {
    /// Pas de dossier `str/` à la racine.
    MissingStrDir,
    /// Pas de dossier `etc/library/` à la racine.
    MissingLibraryDir,
    /// `.mdf` plus grand que la taille observée.
    UnexpectedMdfSize { size: u64 },
}

/// Erreur de lecture de `.mdf` (typée, jamais une panique).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MdfError {
    /// Fichier plus court que la taille observée (vide ou coupé).
    Truncated { size: u64, expected_min: u64 },
    /// Fichier démesuré : ni lu ni sauvegardé.
    TooLarge { size: u64, max: u64 },
    /// Le contenu est lisible mais son format n'est pas encore spécifié (#224).
    FormatUnspecified,
}

impl std::fmt::Display for MdfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Truncated { size, expected_min } => write!(
                f,
                "fichier .mdf tronqué ({size} octets, {expected_min} attendus au moins)"
            ),
            Self::TooLarge { size, max } => {
                write!(f, "fichier .mdf trop grand ({size} octets, maximum {max})")
            }
            Self::FormatUnspecified => write!(f, "format de .mdf non spécifié"),
        }
    }
}

/// Champs de `.mdf` connus. Vide tant que le format n'est pas spécifié (#224) : le numéro de
/// série, les versions de firmware et VID/PID seront ajoutés ici.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MdfInfo {}

/// Point d'extension : décode le contenu de `.mdf`. Tant que #224 n'a pas spécifié le format,
/// aucun champ n'est lu (pas d'offsets inventés).
pub fn parse_mdf(_bytes: &[u8]) -> Result<MdfInfo, MdfError> {
    Err(MdfError::FormatUnspecified)
}

/// Appareil détecté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectedDevice {
    /// Racine du volume (`E:\`).
    pub root: PathBuf,
    /// Nom du volume (vide s'il est illisible).
    pub volume_label: String,
    /// Système de fichiers (`FAT32`…), vide s'il est illisible.
    pub file_system: String,
    pub space: Option<Space>,
    /// Taille de `.mdf` en octets.
    pub mdf_size: u64,
    /// SHA-256 (hexadécimal) du contenu de `.mdf`, absent si le fichier n'a pas été lu.
    pub mdf_sha256: Option<String>,
    /// Résultat du décodage de `.mdf` : `None` si décodé, sinon l'erreur typée.
    pub mdf_error: Option<MdfError>,
    pub warnings: Vec<DeviceWarning>,
    /// Contenu de `.mdf` (jamais sérialisé), pour [`backup_mdf`].
    #[serde(skip)]
    pub mdf_bytes: Option<Vec<u8>>,
}

/// Résultat d'un balayage (miroir côté interface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DetectionReport {
    pub devices: Vec<DetectedDevice>,
    /// Message d'aide quand aucun appareil n'est détecté.
    pub hint: Option<String>,
}

/// Empreinte SHA-256 en hexadécimal.
fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Lit `.mdf` : `(octets lus, erreur typée)`. `None` si le volume a disparu en cours de route.
fn read_mdf(mdf_path: &Path, size: u64) -> Option<(Option<Vec<u8>>, Option<MdfError>)> {
    let too_large = |size| MdfError::TooLarge {
        size,
        max: MDF_MAX_SIZE,
    };
    if size > MDF_MAX_SIZE {
        return Some((None, Some(too_large(size))));
    }
    // Lecture bornée : le fichier peut grandir entre `metadata` et `read`.
    let mut buf = Vec::new();
    fs::File::open(mdf_path)
        .ok()?
        .take(MDF_MAX_SIZE + 1)
        .read_to_end(&mut buf)
        .ok()?;
    let len = buf.len() as u64;
    if len > MDF_MAX_SIZE {
        return Some((None, Some(too_large(len))));
    }
    let error = if len < MDF_DOCUMENTED_SIZE {
        Some(MdfError::Truncated {
            size: len,
            expected_min: MDF_DOCUMENTED_SIZE,
        })
    } else {
        parse_mdf(&buf).err()
    };
    Some((Some(buf), error))
}

/// Sonde une racine : `None` si ce n'est pas une Flam ou si le volume a disparu en cours de
/// lecture (état « non détecté », jamais une panique).
fn probe(env: &Environment, root: &Path) -> Option<DetectedDevice> {
    let mdf_path = root.join(MDF_FILE);
    // `symlink_metadata` : un lien nommé `.mdf` n'est pas suivi.
    let meta = fs::symlink_metadata(&mdf_path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let size = meta.len();
    let (bytes, mdf_error) = read_mdf(&mdf_path, size)?;

    let mut warnings = Vec::new();
    if !root.join("str").is_dir() {
        warnings.push(DeviceWarning::MissingStrDir);
    }
    if !root.join("etc").join("library").is_dir() {
        warnings.push(DeviceWarning::MissingLibraryDir);
    }
    if size > MDF_DOCUMENTED_SIZE && size <= MDF_MAX_SIZE {
        warnings.push(DeviceWarning::UnexpectedMdfSize { size });
    }
    let volume = (env.volume_info)(root).unwrap_or(VolumeInfo {
        label: String::new(),
        fs: String::new(),
    });
    Some(DetectedDevice {
        root: root.to_path_buf(),
        volume_label: volume.label,
        file_system: volume.fs,
        space: (env.space)(root),
        mdf_size: size,
        mdf_sha256: bytes.as_deref().map(sha256_hex),
        mdf_error,
        warnings,
        mdf_bytes: bytes,
    })
}

/// Balaye les racines candidates et rend les appareils trouvés (lecture seule).
pub fn detect(env: &Environment) -> DetectionReport {
    let _quiet = sys::QuietErrors::new();
    let devices: Vec<DetectedDevice> = env
        .roots
        .iter()
        .filter_map(|root| probe(env, root))
        .collect();
    let hint = devices.is_empty().then(|| NOT_DETECTED_HINT.to_string());
    DetectionReport { devices, hint }
}

/// Nom de fichier sûr tiré du nom de volume : lettres, chiffres, `-` et `_`, 32 caractères au
/// plus ; `flam` si rien ne reste.
fn sanitize_label(label: &str) -> String {
    let cleaned: String = label
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(32)
        .collect();
    if cleaned.is_empty() {
        "flam".to_string()
    } else {
        cleaned
    }
}

/// Sauvegarde personnelle de `.mdf` : `<dir>/<volume>-<16 premiers hex du SHA-256>.mdf`
/// (le numéro de série n'est pas encore lisible, #224). Idempotent : un contenu déjà
/// sauvegardé n'est pas réécrit. Rend `Ok(None)` si le contenu n'a pas été lu (trop grand) ou
/// est tronqué : un fichier douteux ne doit pas être conservé comme référence.
pub fn backup_mdf(dir: &Path, device: &DetectedDevice) -> std::io::Result<Option<PathBuf>> {
    let (Some(bytes), Some(hash)) = (&device.mdf_bytes, &device.mdf_sha256) else {
        return Ok(None);
    };
    if matches!(device.mdf_error, Some(MdfError::Truncated { .. })) {
        return Ok(None);
    }
    let name = format!(
        "{}-{}.mdf",
        sanitize_label(&device.volume_label),
        &hash[..16]
    );
    let target = dir.join(name);
    if fs::read(&target).is_ok_and(|existing| existing == *bytes) {
        return Ok(Some(target));
    }
    crate::storage::write_atomic(&target, bytes)?;
    Ok(Some(target))
}

/// Espace libre du volume de `root`, en octets (`None` : indisponible, ou hors Windows).
pub(crate) fn free_space(root: &Path) -> Option<u64> {
    sys::disk_space(root).map(|space| space.free)
}

/// Détecte la Flam branchée (lecture seule : aucune sauvegarde, aucune écriture).
#[tauri::command]
pub async fn device_detect() -> AppResult<DetectionReport> {
    blocking(|| detect(&Environment::current())).await
}

#[cfg(windows)]
mod sys {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};

    use windows_sys::Win32::Storage::FileSystem::{
        GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDrives, GetVolumeInformationW,
    };
    use windows_sys::Win32::System::Diagnostics::Debug::{
        SetThreadErrorMode, SEM_FAILCRITICALERRORS,
    };

    use super::{Space, VolumeInfo};

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_FIXED: u32 = 3;

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    fn text(buf: &[u16]) -> String {
        let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..end])
    }

    /// Désactive la boîte « Insérez un disque » du thread courant tant qu'elle vit.
    pub struct QuietErrors(u32);

    impl QuietErrors {
        pub fn new() -> Self {
            let mut old = 0u32;
            // SAFETY: pointeur de sortie valide ; ne concerne que le thread courant.
            unsafe { SetThreadErrorMode(SEM_FAILCRITICALERRORS, &mut old) };
            Self(old)
        }
    }

    impl Drop for QuietErrors {
        fn drop(&mut self) {
            // SAFETY: restaure la valeur lue par `new`.
            unsafe { SetThreadErrorMode(self.0, std::ptr::null_mut()) };
        }
    }

    pub fn volume_info(root: &Path) -> Option<VolumeInfo> {
        let root = wide(root);
        let mut label = [0u16; 261];
        let mut fs = [0u16; 261];
        // SAFETY: tampons valides de 261 éléments, chemin terminé par un zéro ; les autres
        // pointeurs de sortie sont facultatifs (nuls).
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        (ok != 0).then(|| VolumeInfo {
            label: text(&label),
            fs: text(&fs),
        })
    }

    pub fn disk_space(root: &Path) -> Option<Space> {
        let root = wide(root);
        let (mut free, mut total) = (0u64, 0u64);
        // SAFETY: pointeurs de sortie valides, chemin terminé par un zéro.
        let ok = unsafe {
            GetDiskFreeSpaceExW(root.as_ptr(), &mut free, &mut total, std::ptr::null_mut())
        };
        (ok != 0).then_some(Space { free, total })
    }

    /// Lecteurs amovibles, ou fixes en FAT (un disque USB peut se déclarer fixe). Le disque
    /// système (NTFS, fixe) n'est jamais sondé.
    pub fn candidate_roots() -> Vec<PathBuf> {
        let _quiet = QuietErrors::new();
        // SAFETY: simple lecture du masque des lecteurs logiques.
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|bit| mask & (1 << bit) != 0)
            .map(|bit| PathBuf::from(format!("{}:\\", char::from(b'A' + bit))))
            .filter(|root| {
                let wide_root = wide(root);
                // SAFETY: chemin terminé par un zéro.
                let kind = unsafe { GetDriveTypeW(wide_root.as_ptr()) };
                match kind {
                    // Lecteur sans média (lecteur de cartes vide) : ignoré.
                    DRIVE_REMOVABLE => volume_info(root).is_some(),
                    DRIVE_FIXED => {
                        volume_info(root).is_some_and(|v| v.fs.to_ascii_uppercase().contains("FAT"))
                    }
                    _ => false,
                }
            })
            .collect()
    }
}

#[cfg(not(windows))]
mod sys {
    use std::path::{Path, PathBuf};

    use super::{Space, VolumeInfo};

    pub fn volume_info(_root: &Path) -> Option<VolumeInfo> {
        None
    }

    pub fn disk_space(_root: &Path) -> Option<Space> {
        None
    }

    pub fn candidate_roots() -> Vec<PathBuf> {
        Vec::new()
    }

    pub struct QuietErrors;

    impl QuietErrors {
        pub fn new() -> Self {
            Self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Faux appareil : `.mdf` synthétique, `str/`, `etc/library/`.
    fn fake_device(content: &[u8]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MDF_FILE), content).unwrap();
        fs::create_dir(dir.path().join("str")).unwrap();
        fs::create_dir_all(dir.path().join("etc").join("library")).unwrap();
        dir
    }

    fn synthetic_mdf() -> Vec<u8> {
        (0..MDF_DOCUMENTED_SIZE as u8).collect()
    }

    fn first_device(content: &[u8]) -> (tempfile::TempDir, DetectedDevice) {
        let dev = fake_device(content);
        let d = detect(&env(vec![dev.path().to_path_buf()]))
            .devices
            .remove(0);
        (dev, d)
    }

    #[test]
    fn un_faux_appareil_est_detecte() {
        let dev = fake_device(&synthetic_mdf());
        let report = detect(&env(vec![dev.path().to_path_buf()]));
        assert_eq!(report.devices.len(), 1);
        assert!(report.hint.is_none());
        let d = &report.devices[0];
        assert_eq!(d.volume_label, "FLAM");
        assert_eq!(d.file_system, "FAT32");
        assert_eq!(
            d.space,
            Some(Space {
                free: 100,
                total: 1000
            })
        );
        assert_eq!(d.mdf_size, MDF_DOCUMENTED_SIZE);
        assert_eq!(d.mdf_sha256.as_deref().map(str::len), Some(64));
        assert!(d.warnings.is_empty());
        // Format non spécifié : erreur typée, pas de champ inventé.
        assert_eq!(d.mdf_error, Some(MdfError::FormatUnspecified));
    }

    #[test]
    fn un_dossier_sans_mdf_est_ignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("str")).unwrap();
        let report = detect(&env(vec![dir.path().to_path_buf()]));
        assert!(report.devices.is_empty());
        assert_eq!(report.hint.as_deref(), Some(NOT_DETECTED_HINT));
    }

    #[test]
    fn un_dossier_nomme_mdf_nest_pas_un_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(MDF_FILE)).unwrap();
        assert!(detect(&env(vec![dir.path().to_path_buf()]))
            .devices
            .is_empty());
    }

    #[test]
    fn un_mdf_tronque_ou_vide_donne_une_erreur_typee() {
        for len in [0usize, 1, 125] {
            let (_dev, d) = first_device(&vec![7u8; len]);
            assert_eq!(
                d.mdf_error,
                Some(MdfError::Truncated {
                    size: len as u64,
                    expected_min: MDF_DOCUMENTED_SIZE
                })
            );
        }
    }

    #[test]
    fn un_mdf_demesure_nest_pas_lu() {
        let (_dev, d) = first_device(&vec![0u8; MDF_MAX_SIZE as usize + 1]);
        assert!(matches!(d.mdf_error, Some(MdfError::TooLarge { .. })));
        assert!(d.mdf_bytes.is_none() && d.mdf_sha256.is_none());
    }

    #[test]
    fn un_mdf_plus_grand_que_lobserve_donne_un_avertissement() {
        let (_dev, d) = first_device(&[1u8; 200]);
        assert_eq!(
            d.warnings,
            vec![DeviceWarning::UnexpectedMdfSize { size: 200 }]
        );
    }

    #[test]
    fn les_dossiers_manquants_sont_des_avertissements() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(MDF_FILE), synthetic_mdf()).unwrap();
        let d = &detect(&env(vec![dir.path().to_path_buf()])).devices[0];
        assert_eq!(
            d.warnings,
            vec![
                DeviceWarning::MissingStrDir,
                DeviceWarning::MissingLibraryDir
            ]
        );
    }

    #[test]
    fn un_lecteur_retire_donne_non_detecte() {
        let dev = fake_device(&synthetic_mdf());
        let environment = env(vec![
            dev.path().to_path_buf(),
            PathBuf::from("Z:\\absent\\jamais"),
        ]);
        assert_eq!(detect(&environment).devices.len(), 1);
        drop(dev); // le volume disparaît pendant la scrutation
        let report = detect(&environment);
        assert!(report.devices.is_empty());
        assert_eq!(report.hint.as_deref(), Some(NOT_DETECTED_HINT));
    }

    #[test]
    fn sans_nom_de_volume_ni_espace_pas_de_panique() {
        let dev = fake_device(&synthetic_mdf());
        let environment = Environment {
            roots: vec![dev.path().to_path_buf()],
            volume_info: |_| None,
            space: |_| None,
        };
        let d = &detect(&environment).devices[0];
        assert_eq!(d.volume_label, "");
        assert!(d.space.is_none());
    }

    #[test]
    fn la_sauvegarde_va_dans_le_dossier_de_donnees_et_est_idempotente() {
        let content = synthetic_mdf();
        let (dev, d) = first_device(&content);
        let data = tempfile::tempdir().unwrap();
        let first = backup_mdf(data.path(), &d).unwrap().unwrap();
        assert!(first.starts_with(data.path()));
        assert_eq!(fs::read(&first).unwrap(), content);
        let name = first.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.starts_with("FLAM-") && name.ends_with(".mdf"));
        let second = backup_mdf(data.path(), &d).unwrap().unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 1);
        // Le volume n'a reçu aucune écriture : `.mdf`, `str/`, `etc/`.
        assert_eq!(fs::read_dir(dev.path()).unwrap().count(), 3);
    }

    #[test]
    fn un_contenu_different_donne_une_autre_sauvegarde() {
        let data = tempfile::tempdir().unwrap();
        for byte in [1u8, 2] {
            let (_dev, d) = first_device(&[byte; 126]);
            backup_mdf(data.path(), &d).unwrap();
        }
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 2);
    }

    #[test]
    fn un_mdf_tronque_ou_non_lu_nest_pas_sauvegarde() {
        let data = tempfile::tempdir().unwrap();
        let (_a, d) = first_device(&[1, 2, 3]);
        assert_eq!(backup_mdf(data.path(), &d).unwrap(), None);
        let (_b, d) = first_device(&vec![0u8; MDF_MAX_SIZE as usize + 1]);
        assert_eq!(backup_mdf(data.path(), &d).unwrap(), None);
        assert_eq!(fs::read_dir(data.path()).unwrap().count(), 0);
    }

    #[test]
    fn le_nom_de_volume_est_assaini() {
        assert_eq!(sanitize_label("FLAM"), "FLAM");
        assert_eq!(sanitize_label("..\\../évil name"), "vilname");
        assert_eq!(sanitize_label(""), "flam");
        assert_eq!(sanitize_label("é/é"), "flam");
        assert_eq!(sanitize_label(&"a".repeat(80)).len(), 32);
    }

    #[test]
    fn le_rapport_ne_serialise_pas_le_contenu_de_mdf() {
        let dev = fake_device(&synthetic_mdf());
        let report = detect(&env(vec![dev.path().to_path_buf()]));
        let json = serde_json::to_string(&report).unwrap();
        assert!(!json.contains("mdf_bytes"));
        assert!(json.contains("format_unspecified"));
    }

    /// Lecteurs Windows réels : `cargo test -- --ignored lecteurs_reels --nocapture`. Branchez
    /// la Flam en mode connexion USB (ou non) : le balayage ne doit jamais paniquer et le
    /// résultat est affiché.
    #[test]
    #[ignore = "lecteurs réels : nécessite Windows, résultat selon le matériel branché"]
    fn lecteurs_reels() {
        let environment = Environment::current();
        println!("racines candidates : {:?}", environment.roots);
        let report = detect(&environment);
        println!("{report:#?}");
        if cfg!(windows) {
            // Le disque système n'est jamais candidat.
            let sys = std::env::var("SystemDrive")
                .unwrap_or_else(|_| "C:".into())
                .to_ascii_uppercase();
            assert!(environment
                .roots
                .iter()
                .all(|r| !r.to_string_lossy().to_ascii_uppercase().starts_with(&sys)));
        }
    }
}
