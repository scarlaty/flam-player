//! Erreurs de conversion attendues, avec le même `code` et le même code de sortie que
//! `ConvError` de l'ancien convertisseur Python (README, section « Codes de sortie »).

use std::fmt;

/// Erreur de conversion : identifiant stable de l'événement JSON `error` et code de sortie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvError {
    pub code: &'static str,
    pub exit_code: u8,
    pub message: String,
}

impl ConvError {
    fn new(code: &'static str, exit_code: u8, message: impl Into<String>) -> Self {
        ConvError {
            code,
            exit_code,
            message: message.into(),
        }
    }

    /// Archive absente, illisible ou corrompue (Python : `ArchiveError`, code 3).
    pub fn archive_unreadable(message: impl Into<String>) -> Self {
        Self::new("archive_unreadable", 3, message)
    }

    /// Erreur d'entrée/sortie du système en lisant l'archive d'entrée (antivirus, fichier
    /// verrouillé, volume retiré) : l'archive n'est pas en cause, à la différence
    /// d'`archive_unreadable` (#580). Même code de sortie (3).
    pub fn archive_io(message: impl Into<String>) -> Self {
        Self::new("archive_io", 3, message)
    }

    /// L'archive n'est pas une histoire Telmi (Python : `ArchiveError(code="not_telmi")`).
    pub fn not_telmi(message: impl Into<String>) -> Self {
        Self::new("not_telmi", 3, message)
    }

    /// Contenu invalide (Python : `ConvError` par défaut, code 4).
    pub fn invalid_content(message: impl Into<String>) -> Self {
        Self::new("invalid_content", 4, message)
    }

    /// Cas que le Rust ne traite pas encore et délègue à Python (décision D4, code 7).
    pub fn unsupported_format(message: impl Into<String>) -> Self {
        Self::new("unsupported_format", 7, message)
    }

    /// Image ou audio référencé mais absent de l'archive, sans `--allow-missing` (Python :
    /// `MissingAssetError`, code 5).
    pub fn missing_asset(message: impl Into<String>) -> Self {
        Self::new("missing_asset", 5, message)
    }

    /// Conversion annulée (Python : `KeyboardInterrupt`, événement `cancelled`, code 130).
    pub fn cancelled() -> Self {
        Self::new("cancelled", 130, "conversion annulee")
    }

    /// Écriture de la sortie impossible (Python : `OutputError`, code 6).
    pub fn output_error(message: impl Into<String>) -> Self {
        Self::new("output_error", 6, message)
    }

    /// État impossible atteint (Python : exception non rattrapée, `internal_error`, code 6).
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::new("internal_error", 6, message)
    }
}

/// L'erreur vient-elle du système (elle porte un code de l'OS : accès refusé par un antivirus,
/// fichier verrouillé, volume retiré) plutôt que du contenu de l'archive ? Règle unique, aussi
/// utilisée par le launcher pour décider de garder l'archive téléchargée. Exclus, l'archive
/// reste en cause : le positionnement refusé (`InvalidInput` : décalage absurde lu dans
/// l'archive, zip64 forgé) et, sous Windows, le contenu stocké illisible (`ERROR_CRC` 23,
/// `ERROR_FILE_CORRUPT` 1392, `ERROR_DISK_CORRUPT` 1393 : une copie du fichier ailleurs sur le
/// disque le réparerait). Les erreurs construites par le lecteur zip (CRC, en-tête,
/// troncature) n'ont pas de code de l'OS. Limite : sous Unix, `EIO` reste une erreur du
/// système (secteur défectueux ou volume retiré, indiscernables).
pub fn system_io(e: &std::io::Error) -> bool {
    /// `ERROR_CRC`, `ERROR_FILE_CORRUPT`, `ERROR_DISK_CORRUPT` (Windows).
    const STORED_CONTENT_UNREADABLE: [i32; 3] = [23, 1392, 1393];
    e.raw_os_error()
        .is_some_and(|code| !(cfg!(windows) && STORED_CONTENT_UNREADABLE.contains(&code)))
        && e.kind() != std::io::ErrorKind::InvalidInput
}

impl fmt::Display for ConvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConvError {}

impl From<crate::json::ContentError> for ConvError {
    fn from(e: crate::json::ContentError) -> Self {
        ConvError::invalid_content(e.message)
    }
}

#[cfg(test)]
mod tests {
    use super::system_io;
    use std::io::{Error, ErrorKind};

    /// Règle unique du convertisseur et du launcher (#580).
    #[test]
    fn erreur_du_systeme_ou_du_contenu() {
        // Erreurs construites sans code de l'OS : contenu en cause.
        assert!(!system_io(&Error::new(ErrorKind::UnexpectedEof, "tronque")));
        assert!(!system_io(&Error::from(ErrorKind::InvalidInput)));
        // Code de l'OS : le système est en cause (13 : `EACCES`, 5 : accès refusé sous Windows).
        assert!(system_io(&Error::from_raw_os_error(if cfg!(windows) {
            5
        } else {
            13
        })));
        // `EINVAL` / `ERROR_INVALID_PARAMETER` : positionnement refusé, `InvalidInput`.
        assert!(!system_io(&Error::from_raw_os_error(if cfg!(windows) {
            87
        } else {
            22
        })));
        // Windows : contenu stocké illisible, l'archive reste en cause.
        #[cfg(windows)]
        for code in [23, 1392, 1393] {
            assert!(!system_io(&Error::from_raw_os_error(code)), "{code}");
        }
        // Les mêmes nombres ne sont pas ces erreurs ailleurs.
        #[cfg(not(windows))]
        for code in [23, 1392, 1393] {
            assert!(system_io(&Error::from_raw_os_error(code)), "{code}");
        }
    }
}
