//! Éjection logicielle de la Flam (#232), sans jamais forcer.
//!
//! # Méthode (Windows)
//!
//! 1. **Préparation du volume** (`\\.\E:`) : `FlushFileBuffers` (toutes les écritures en
//!    attente du volume vont sur la clé), `FSCTL_LOCK_VOLUME` (échoue si un autre processus a
//!    un fichier ouvert : on s'arrête là, sans forcer), `FSCTL_DISMOUNT_VOLUME`, puis
//!    `IOCTL_STORAGE_MEDIA_REMOVAL` (autorise le retrait). Le verrou est rendu à la fermeture.
//!    Sans droit d'ouvrir le volume en écriture, l'étape est sautée : la demande suivante
//!    vide les caches et vérifie les fichiers ouverts d'elle-même.
//! 2. **Demande d'éjection** `CM_Request_Device_EjectW` sur le parent USB du disque (le
//!    périphérique que retire « Retirer le périphérique en toute sécurité »). Le disque est
//!    trouvé par son numéro (`IOCTL_STORAGE_GET_DEVICE_NUMBER`) parmi les interfaces
//!    `GUID_DEVINTERFACE_DISK`. Le gestionnaire Plug and Play interroge chaque pilote et
//!    processus : un refus (« veto ») est respecté et donne le repli.
//!
//! Choix : `CM_Request_Device_EjectW` plutôt que `IOCTL_STORAGE_EJECT_MEDIA`, qui sur un
//! stockage USB ne retire que le support (le lecteur reste visible, le périphérique n'est pas
//! arrêté) et ne passe pas par la négociation Plug and Play. Aucune option de retrait forcé
//! n'est utilisée : la Flam peut exécuter des tâches à l'éjection (mise à jour du firmware).
//!
//! # Repli
//!
//! Tout échec (fichier ouvert, refus, appareil introuvable, autre système) donne
//! [`EjectOutcome::Fallback`] avec [`FALLBACK_MESSAGE`] : l'utilisateur éjecte depuis
//! l'Explorateur. Une éjection est **refusée** (erreur) si une autre opération est en cours sur
//! l'appareil ([`DeviceLock`]) : un transfert ou un retrait ne doit jamais être coupé.

use std::fs;
use std::path::Path;

use serde::Serialize;

use super::detect::MDF_FILE;
use super::lock::{Busy, DeviceLock, Operation, DEVICE_LOCK};
use crate::catalog::blocking;
use crate::{AppError, AppResult};

/// Message après une éjection réussie.
pub const EJECTED_MESSAGE: &str = "Vous pouvez débrancher la Flam.";
/// Message de repli quand l'éjection logicielle échoue.
pub const FALLBACK_MESSAGE: &str = "Retirez la Flam en toute sécurité : éjectez-la depuis \
l'Explorateur (clic droit sur le lecteur, puis « Éjecter »), puis débranchez-la.";

/// Résultat d'une éjection (miroir côté interface).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum EjectOutcome {
    /// Volume démonté et périphérique arrêté : la Flam peut être débranchée.
    Ejected { message: String },
    /// Éjection logicielle impossible : l'utilisateur éjecte depuis l'Explorateur.
    Fallback { reason: String, message: String },
}

impl EjectOutcome {
    fn ejected() -> Self {
        EjectOutcome::Ejected {
            message: EJECTED_MESSAGE.into(),
        }
    }

    fn fallback(reason: impl Into<String>) -> Self {
        EjectOutcome::Fallback {
            reason: reason.into(),
            message: FALLBACK_MESSAGE.into(),
        }
    }
}

/// Refus d'éjecter.
#[derive(Debug, thiserror::Error)]
pub enum EjectError {
    #[error(transparent)]
    Busy(#[from] Busy),
    #[error("ce dossier n'est pas une Flam (fichier .mdf absent)")]
    NotADevice,
}

impl From<EjectError> for AppError {
    fn from(err: EjectError) -> Self {
        AppError::Validation(err.to_string())
    }
}

/// Échec de la préparation du volume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrepareError {
    /// Pas le droit d'ouvrir le volume en écriture : étape sautée, la demande d'éjection
    /// vérifie elle-même les fichiers ouverts.
    NoAccess,
    /// Volume occupé (fichier ouvert par un autre processus) : aucune éjection.
    InUse(String),
    /// Autre échec : aucune éjection.
    Failed(String),
}

/// Appels système de l'éjection, injectables pour les tests.
pub trait EjectSystem {
    /// Vide les caches, verrouille et démonte le volume de `root`.
    fn prepare(&self, root: &Path) -> Result<(), PrepareError>;
    /// Demande l'arrêt du périphérique de `root`. `Err` : raison du refus ou de l'échec.
    fn request_eject(&self, root: &Path) -> Result<(), String>;
}

/// Éjecte la Flam montée en `root`. Refuse si `root` n'est pas une Flam ou si une autre
/// opération est en cours ; sinon rend toujours un résultat (éjectée ou repli).
pub fn eject(
    root: &Path,
    lock: &DeviceLock,
    system: &dyn EjectSystem,
) -> Result<EjectOutcome, EjectError> {
    match fs::symlink_metadata(root.join(MDF_FILE)) {
        Ok(meta) if meta.is_file() => {}
        _ => return Err(EjectError::NotADevice),
    }
    // Verrou gardé jusqu'à la fin : aucun transfert ne peut démarrer pendant l'éjection.
    let _guard = lock.try_begin(Operation::Eject)?;
    match system.prepare(root) {
        Ok(()) | Err(PrepareError::NoAccess) => {}
        Err(PrepareError::InUse(reason)) => {
            tracing::info!("éjection : volume occupé ({reason})");
            return Ok(EjectOutcome::fallback(format!(
                "un fichier de la Flam est ouvert par un autre programme ({reason})"
            )));
        }
        Err(PrepareError::Failed(reason)) => {
            tracing::warn!("éjection : préparation impossible ({reason})");
            return Ok(EjectOutcome::fallback(reason));
        }
    }
    Ok(match system.request_eject(root) {
        Ok(()) => EjectOutcome::ejected(),
        Err(reason) => {
            tracing::warn!("éjection refusée : {reason}");
            EjectOutcome::fallback(reason)
        }
    })
}

#[cfg(not(windows))]
pub use other::Unsupported as CurrentSystem;
/// Système réel.
#[cfg(windows)]
pub use sys::WindowsEject as CurrentSystem;

/// Éjecte la Flam montée en `root` (`E:\`). Rend l'éjection réussie ou le repli à afficher ;
/// erreur si une opération est en cours sur l'appareil.
#[tauri::command]
pub async fn device_eject(root: String) -> AppResult<EjectOutcome> {
    blocking(move || eject(Path::new(&root), &DEVICE_LOCK, &CurrentSystem))
        .await?
        .map_err(AppError::from)
}

/// Lettre de lecteur d'une racine de volume (`E:\`, `E:/` ou `E:`), en majuscule.
pub fn drive_letter(root: &Path) -> Option<char> {
    let text = root.to_str()?;
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let mut chars = text.chars();
    let letter = chars.next().filter(char::is_ascii_alphabetic)?;
    if chars.next() != Some(':') {
        return None;
    }
    match chars.as_str() {
        "" | "\\" | "/" => Some(letter.to_ascii_uppercase()),
        _ => None,
    }
}

/// Découpe une liste `MULTI_SZ` (chaînes terminées par un nul, liste terminée par un nul).
pub fn split_multi_sz(buf: &[u16]) -> Vec<String> {
    buf.split(|&c| c == 0)
        .take_while(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(not(windows))]
mod other {
    use super::*;

    /// Hors Windows : toujours le repli.
    pub struct Unsupported;

    impl EjectSystem for Unsupported {
        fn prepare(&self, _root: &Path) -> Result<(), PrepareError> {
            Err(PrepareError::Failed(
                "éjection logicielle disponible sous Windows seulement".into(),
            ))
        }

        fn request_eject(&self, _root: &Path) -> Result<(), String> {
            Err("éjection logicielle disponible sous Windows seulement".into())
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;
    use std::time::Duration;

    use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
        CM_Get_Device_Interface_ListW, CM_Get_Device_Interface_List_SizeW,
        CM_Get_Device_Interface_PropertyW, CM_Get_Parent, CM_Locate_DevNodeW,
        CM_Request_Device_EjectW, PNP_VetoTypeUnknown, CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
        CM_LOCATE_DEVNODE_NORMAL, CR_BUFFER_SMALL, CR_SUCCESS, PNP_VETO_TYPE,
    };
    use windows_sys::Win32::Devices::Properties::{DEVPKEY_Device_InstanceId, DEVPROPTYPE};
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, GENERIC_READ, GENERIC_WRITE, HANDLE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        FlushFileBuffers, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };
    use windows_sys::Win32::System::Ioctl::{
        FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, GUID_DEVINTERFACE_DISK,
        IOCTL_STORAGE_GET_DEVICE_NUMBER, IOCTL_STORAGE_MEDIA_REMOVAL, PREVENT_MEDIA_REMOVAL,
        STORAGE_DEVICE_NUMBER,
    };
    use windows_sys::Win32::System::IO::DeviceIoControl;

    use super::{drive_letter, split_multi_sz, EjectSystem, PrepareError};

    /// Essais du verrouillage : un explorateur ou un antivirus peut garder le volume ouvert
    /// un court instant.
    const LOCK_ATTEMPTS: u32 = 10;
    const LOCK_DELAY: Duration = Duration::from_millis(200);
    /// Essais de la demande d'éjection (refus transitoire juste après le démontage).
    const EJECT_ATTEMPTS: u32 = 3;
    const EJECT_DELAY: Duration = Duration::from_millis(500);
    const VETO_NAME_LEN: usize = 260;

    /// Éjection réelle (voir le module).
    pub struct WindowsEject;

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text)
            .encode_wide()
            .chain(Some(0))
            .collect()
    }

    fn open(path: &str, access: u32) -> io::Result<File> {
        OpenOptions::new()
            .access_mode(access)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(path)
    }

    /// `DeviceIoControl` sans tampon d'entrée ni de sortie, ou avec ceux fournis.
    fn ioctl(
        file: &File,
        code: u32,
        input: Option<(&[u8], u32)>,
        output: Option<(*mut core::ffi::c_void, u32)>,
    ) -> io::Result<()> {
        let mut returned = 0u32;
        let (in_ptr, in_len) = input.map_or((std::ptr::null(), 0), |(b, l)| (b.as_ptr(), l));
        let (out_ptr, out_len) = output.unwrap_or((std::ptr::null_mut(), 0));
        // SAFETY : poignée ouverte vivante ; tampons valides pour les longueurs données.
        let ok = unsafe {
            DeviceIoControl(
                file.as_raw_handle() as HANDLE,
                code,
                in_ptr.cast(),
                in_len,
                out_ptr,
                out_len,
                &mut returned,
                std::ptr::null_mut(),
            )
        };
        if ok == 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    fn device_number(file: &File) -> io::Result<(u32, u32)> {
        let mut number = STORAGE_DEVICE_NUMBER {
            DeviceType: 0,
            DeviceNumber: 0,
            PartitionNumber: 0,
        };
        ioctl(
            file,
            IOCTL_STORAGE_GET_DEVICE_NUMBER,
            None,
            Some((
                (&mut number as *mut STORAGE_DEVICE_NUMBER).cast(),
                size_of::<STORAGE_DEVICE_NUMBER>() as u32,
            )),
        )?;
        Ok((number.DeviceType, number.DeviceNumber))
    }

    fn volume_path(root: &Path) -> Result<String, String> {
        drive_letter(root)
            .map(|letter| format!(r"\\.\{letter}:"))
            .ok_or_else(|| format!("{} n'est pas la racine d'un lecteur", root.display()))
    }

    fn is_in_use(err: &io::Error) -> bool {
        matches!(
            err.raw_os_error().map(|c| c as u32),
            Some(ERROR_ACCESS_DENIED | ERROR_SHARING_VIOLATION)
        )
    }

    /// Interfaces disque présentes.
    fn disk_interfaces() -> Result<Vec<String>, String> {
        for _ in 0..3 {
            let mut len = 0u32;
            // SAFETY : pointeur de sortie valide, GUID statique.
            let cr = unsafe {
                CM_Get_Device_Interface_List_SizeW(
                    &mut len,
                    &GUID_DEVINTERFACE_DISK,
                    std::ptr::null(),
                    CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
                )
            };
            if cr != CR_SUCCESS {
                return Err(format!("liste des disques indisponible (code {cr})"));
            }
            let mut buf = vec![0u16; len as usize];
            // SAFETY : tampon de `len` caractères.
            let cr = unsafe {
                CM_Get_Device_Interface_ListW(
                    &GUID_DEVINTERFACE_DISK,
                    std::ptr::null(),
                    buf.as_mut_ptr(),
                    len,
                    CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
                )
            };
            match cr {
                CR_SUCCESS => return Ok(split_multi_sz(&buf)),
                // Un disque est apparu entre les deux appels : recommencer.
                CR_BUFFER_SMALL => continue,
                _ => return Err(format!("liste des disques indisponible (code {cr})")),
            }
        }
        Err("liste des disques instable".into())
    }

    /// Nœud du disque qui porte le volume de `root`.
    fn disk_devinst(root: &Path) -> Result<u32, String> {
        let volume =
            open(&volume_path(root)?, 0).map_err(|err| format!("volume introuvable : {err}"))?;
        let target = device_number(&volume).map_err(|err| format!("numéro du disque : {err}"))?;
        drop(volume);
        for interface in disk_interfaces()? {
            let Ok(disk) = open(&interface, 0) else {
                continue;
            };
            if device_number(&disk).ok() != Some(target) {
                continue;
            }
            drop(disk);
            return devinst_of_interface(&interface);
        }
        Err("disque de la Flam introuvable".into())
    }

    fn devinst_of_interface(interface: &str) -> Result<u32, String> {
        let path = wide(interface);
        let mut buf = vec![0u16; 512];
        let mut size = (buf.len() * 2) as u32;
        let mut kind: DEVPROPTYPE = 0;
        // SAFETY : chaîne terminée par un nul ; tampon de `size` octets.
        let cr = unsafe {
            CM_Get_Device_Interface_PropertyW(
                path.as_ptr(),
                &DEVPKEY_Device_InstanceId,
                &mut kind,
                buf.as_mut_ptr().cast(),
                &mut size,
                0,
            )
        };
        if cr != CR_SUCCESS {
            return Err(format!("identifiant du disque indisponible (code {cr})"));
        }
        let mut devinst = 0u32;
        // SAFETY : identifiant terminé par un nul écrit par l'appel précédent.
        let cr =
            unsafe { CM_Locate_DevNodeW(&mut devinst, buf.as_ptr(), CM_LOCATE_DEVNODE_NORMAL) };
        if cr != CR_SUCCESS {
            return Err(format!("disque introuvable (code {cr})"));
        }
        Ok(devinst)
    }

    impl EjectSystem for WindowsEject {
        fn prepare(&self, root: &Path) -> Result<(), PrepareError> {
            let path = volume_path(root).map_err(PrepareError::Failed)?;
            let volume = match open(&path, GENERIC_READ | GENERIC_WRITE) {
                Ok(file) => file,
                Err(err) if err.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) => {
                    return Err(PrepareError::NoAccess)
                }
                Err(err) => return Err(PrepareError::Failed(format!("volume : {err}"))),
            };
            // SAFETY : poignée ouverte en écriture, vivante.
            if unsafe { FlushFileBuffers(volume.as_raw_handle() as HANDLE) } == 0 {
                let err = io::Error::last_os_error();
                return Err(PrepareError::Failed(format!("vidage des caches : {err}")));
            }
            let mut attempt = 0;
            loop {
                match ioctl(&volume, FSCTL_LOCK_VOLUME, None, None) {
                    Ok(()) => break,
                    Err(err) if is_in_use(&err) && attempt + 1 < LOCK_ATTEMPTS => {
                        attempt += 1;
                        std::thread::sleep(LOCK_DELAY);
                    }
                    Err(err) if is_in_use(&err) => {
                        return Err(PrepareError::InUse(err.to_string()));
                    }
                    Err(err) => return Err(PrepareError::Failed(format!("verrouillage : {err}"))),
                }
            }
            ioctl(&volume, FSCTL_DISMOUNT_VOLUME, None, None)
                .map_err(|err| PrepareError::Failed(format!("démontage : {err}")))?;
            let allow = PREVENT_MEDIA_REMOVAL {
                PreventMediaRemoval: false,
            };
            // SAFETY : structure `repr(C)` d'un octet, lue comme octets.
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    (&allow as *const PREVENT_MEDIA_REMOVAL).cast::<u8>(),
                    size_of::<PREVENT_MEDIA_REMOVAL>(),
                )
            };
            // Échec sans conséquence : la plupart des clés USB n'empêchent pas le retrait.
            let _ = ioctl(
                &volume,
                IOCTL_STORAGE_MEDIA_REMOVAL,
                Some((bytes, bytes.len() as u32)),
                None,
            );
            // Fermeture : le verrou est rendu, le volume reste démonté jusqu'au prochain accès.
            Ok(())
        }

        fn request_eject(&self, root: &Path) -> Result<(), String> {
            let disk = disk_devinst(root)?;
            let mut parent = 0u32;
            // SAFETY : pointeur de sortie valide.
            let cr = unsafe { CM_Get_Parent(&mut parent, disk, 0) };
            if cr != CR_SUCCESS {
                return Err(format!("périphérique USB introuvable (code {cr})"));
            }
            let mut last = String::new();
            for attempt in 0..EJECT_ATTEMPTS {
                if attempt > 0 {
                    std::thread::sleep(EJECT_DELAY);
                }
                let mut veto: PNP_VETO_TYPE = PNP_VetoTypeUnknown;
                let mut name = [0u16; VETO_NAME_LEN];
                // SAFETY : sorties valides ; `name` fait `VETO_NAME_LEN` caractères. Drapeaux
                // nuls : pas d'interface de Windows, pas de retrait forcé.
                let cr = unsafe {
                    CM_Request_Device_EjectW(
                        parent,
                        &mut veto,
                        name.as_mut_ptr(),
                        VETO_NAME_LEN as u32,
                        0,
                    )
                };
                if cr == CR_SUCCESS && veto == PNP_VetoTypeUnknown {
                    return Ok(());
                }
                let end = name.iter().position(|&c| c == 0).unwrap_or(name.len());
                let who = String::from_utf16_lossy(&name[..end]);
                last = if cr != CR_SUCCESS {
                    format!("demande refusée (code {cr})")
                } else if who.is_empty() {
                    format!("éjection refusée par Windows (motif {veto})")
                } else {
                    format!("éjection refusée par Windows (motif {veto} : {who})")
                };
            }
            Err(last)
        }
    }
}

#[cfg(test)]
mod tests;
