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
