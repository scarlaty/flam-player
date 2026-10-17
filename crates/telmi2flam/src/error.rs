//! Erreurs de conversion attendues, avec le même `code` et le même code de sortie que
//! `ConvError` de `telmi2flam.py` (README, section « Codes de sortie »).

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
}

impl fmt::Display for ConvError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ConvError {}
