//! Accès disque bas niveau des opérations (`authoring::ops`) : poignées qui ne suivent pas les
//! liens, identité de fichier, épinglage des dossiers, renommage sans écrasement, Corbeille
//! Windows.
//!
//! # Épinglage (TOCTOU)
//!
//! [`open_entry`] avec `pin` ouvre un dossier **sans suivre** de lien
//! (`FILE_FLAG_OPEN_REPARSE_POINT`) avec un accès en lecture de données (`FILE_LIST_DIRECTORY`)
//! et un partage **sans** `FILE_SHARE_DELETE` : tant que la poignée est ouverte, personne ne
//! peut renommer, supprimer ni remplacer ce dossier par une jonction. Un accès limité aux
//! attributs ne suffirait pas : Windows ne compte pas ces poignées dans le contrôle de partage.
//! Les opérations épinglent la racine puis chaque dossier du chemin, depuis la racine, en
//! contrôlant chaque poignée ouverte (dossier, pas de point de réanalyse) avant d'agir dedans.

use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

/// Identité d'un fichier ou d'un dossier : numéro de série du volume et index du fichier.
/// Deux chemins différents (`C:\x`, `\\localhost\C$\x`, nom court 8.3…) désignent le même
/// objet si et seulement si leurs identités sont égales (index sur 64 bits : ReFS, qui a des
/// identifiants de 128 bits, peut en théorie produire des collisions ; NTFS non).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct FileId {
    volume: u64,
    index: u64,
}

/// Ce que dit une poignée ouverte sans suivre de lien.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Info {
    pub id: FileId,
    pub dir: bool,
    /// Lien symbolique, jonction ou autre point de réanalyse (OneDrive « à la demande »…).
    pub reparse: bool,
}

/// Ouvre `path` sans suivre de lien et lit son identité. Avec `pin`, la poignée interdit aux
/// autres de renommer ou supprimer l'entrée tant qu'elle est ouverte (voir la doc du module).
#[cfg(windows)]
pub(crate) fn open_entry(path: &Path, pin: bool) -> io::Result<(File, Info)> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };

    const FILE_LIST_DIRECTORY: u32 = 0x0001;
    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const SYNCHRONIZE: u32 = 0x0010_0000;
    const SHARE_READ_WRITE: u32 = 0x1 | 0x2;
    const SHARE_ALL: u32 = 0x1 | 0x2 | 0x4;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;

    let (access, share) = if pin {
        (
            FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            SHARE_READ_WRITE,
        )
    } else {
        (FILE_READ_ATTRIBUTES, SHARE_ALL)
    };
    let file = fs::OpenOptions::new()
        .access_mode(access)
        .share_mode(share)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    // SAFETY : poignée valide (fichier ouvert ci-dessus) et structure de sortie locale.
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let attrs = info.dwFileAttributes;
    Ok((
        file,
        Info {
            id: FileId {
                volume: u64::from(info.dwVolumeSerialNumber),
                index: (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
            },
            dir: attrs & FILE_ATTRIBUTE_DIRECTORY != 0,
            reparse: attrs & FILE_ATTRIBUTE_REPARSE_POINT != 0,
        },
    ))
}

/// Hors Windows (non livré) : pas d'épinglage, identité `dev`/`ino`.
#[cfg(not(windows))]
pub(crate) fn open_entry(path: &Path, _pin: bool) -> io::Result<(File, Info)> {
    use std::os::unix::fs::MetadataExt;
    let meta = fs::symlink_metadata(path)?;
    let file = File::open(path)?;
    Ok((
        file,
        Info {
            id: FileId {
                volume: meta.dev(),
                index: meta.ino(),
            },
            dir: meta.is_dir(),
            reparse: meta.file_type().is_symlink(),
        },
    ))
}

/// Chemin en UTF-16 terminé par un zéro, en forme `\\?\` au-delà de la longueur classique
/// (la bibliothèque standard fait de même pour ses propres appels).
#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    let raw: Vec<u16> = path.as_os_str().encode_wide().collect();
    let backslash = u16::from(b'\\');
    let verbatim: Vec<u16> = r"\\?\".encode_utf16().collect();
    let mut out = if raw.len() >= 248 && !raw.starts_with(&verbatim) {
        if raw.starts_with(&[backslash, backslash]) {
            let mut v: Vec<u16> = r"\\?\UNC\".encode_utf16().collect();
            v.extend_from_slice(&raw[2..]);
            v
        } else {
            let mut v = verbatim;
            v.extend_from_slice(&raw);
            v
        }
    } else {
        raw
    };
    out.push(0);
    out
}

/// Renomme `from` en `to` **sans jamais écraser** : `to` existant → erreur
/// `AlreadyExists`. Même volume seulement (pas de copie). Patiente si le fichier est
/// brièvement verrouillé (antivirus, indexeur).
pub(crate) fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::MoveFileExW;
        let (wfrom, wto) = (wide(from), wide(to));
        crate::storage::retry_transient(
            || {
                // Drapeaux 0 : ni remplacement, ni copie entre volumes.
                // SAFETY : chaînes UTF-16 terminées par un zéro, vivantes pendant l'appel.
                let ok = unsafe { MoveFileExW(wfrom.as_ptr(), wto.as_ptr(), 0) };
                if ok != 0 {
                    return Ok(());
                }
                let err = io::Error::last_os_error();
                // ERROR_FILE_EXISTS (80), ERROR_ALREADY_EXISTS (183).
                if matches!(err.raw_os_error(), Some(80 | 183)) {
                    return Err(io::Error::new(io::ErrorKind::AlreadyExists, err));
                }
                Err(err)
            },
            std::thread::sleep,
        )
    }
    #[cfg(not(windows))]
    {
        // Non livré : contrôle puis renommage (fenêtre de course acceptée hors Windows).
        if fs::symlink_metadata(to).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "nom déjà pris",
            ));
        }
        fs::rename(from, to)
    }
}

/// Raison pour laquelle la Corbeille Windows n'est pas utilisable pour `base`, ou `None`.
/// Seuls les disques locaux fixes ont une Corbeille : ailleurs (clé USB, réseau), la
/// suppression serait **définitive**, ce que l'éditeur ne fait jamais.
pub(crate) fn recycle_bin_refusal(base: &Path) -> Option<&'static str> {
    let text = base.to_string_lossy();
    if text.starts_with(r"\\") {
        return Some("dossier réseau ou chemin long : pas de Corbeille Windows");
    }
    let bytes = text.as_bytes();
    if bytes.len() < 3 || !bytes[0].is_ascii_alphabetic() || bytes[1] != b':' || bytes[2] != b'\\' {
        return Some("forme de chemin inattendue");
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumePathNameW};
        const DRIVE_FIXED: u32 = 3;
        // Volume réel du dossier (une clé USB peut être montée dans un dossier de C:), pas la
        // lettre de lecteur du chemin.
        let path = wide(base);
        let mut volume = vec![0u16; 1024];
        // SAFETY : chaîne d'entrée terminée par un zéro, tampon de sortie de la taille donnée.
        let ok =
            unsafe { GetVolumePathNameW(path.as_ptr(), volume.as_mut_ptr(), volume.len() as u32) };
        if ok == 0 {
            return Some("volume du dossier introuvable");
        }
        // SAFETY : `GetVolumePathNameW` a écrit une chaîne terminée par un zéro.
        if unsafe { GetDriveTypeW(volume.as_ptr()) } != DRIVE_FIXED {
            return Some("lecteur amovible ou réseau : pas de Corbeille Windows");
        }
        None
    }
    #[cfg(not(windows))]
    Some("Corbeille Windows indisponible hors Windows")
}

/// Envoie `paths` (chemins absolus non verbatim, sur un disque fixe : voir
/// [`recycle_bin_refusal`]) dans la Corbeille Windows. `FOF_WANTNUKEWARNING` : si un élément
/// ne peut pas y aller (Corbeille désactivée, trop gros), Windows demande confirmation au lieu
/// de supprimer définitivement en silence.
pub(crate) fn recycle(paths: &[PathBuf]) -> io::Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::UI::Shell::{
            SHFileOperationW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_SILENT, FOF_WANTNUKEWARNING,
            FO_DELETE, SHFILEOPSTRUCTW,
        };
        // Liste de chemins séparés par des zéros, terminée par deux zéros.
        let mut from: Vec<u16> = Vec::new();
        for path in paths {
            from.extend(path.as_os_str().encode_wide());
            from.push(0);
        }
        from.push(0);
        let flags = FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_WANTNUKEWARNING;
        let mut op = SHFILEOPSTRUCTW {
            hwnd: std::ptr::null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: std::ptr::null(),
            fFlags: flags as u16,
            fAnyOperationsAborted: 0,
            hNameMappings: std::ptr::null_mut(),
            lpszProgressTitle: std::ptr::null(),
        };
        // SAFETY : structure initialisée, `from` vivant pendant l'appel (synchrone).
        let code = unsafe { SHFileOperationW(&mut op) };
        if code != 0 {
            return Err(io::Error::other(format!(
                "envoi dans la Corbeille Windows refusé (code {code})"
            )));
        }
        if op.fAnyOperationsAborted != 0 {
            return Err(io::Error::other(
                "envoi dans la Corbeille Windows interrompu",
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(io::Error::other(
        "Corbeille Windows indisponible hors Windows",
    ))
}
