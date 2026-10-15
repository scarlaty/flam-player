use serde::{Serialize, Serializer};

/// Erreur commune des commandes Tauri, sérialisée en texte vers le front.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Erreur d'entrée/sortie (fichier, dossier de données…).
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_erreur_est_serialisee_en_texte() {
        let err = AppError::from(std::io::Error::other("boum"));
        let json = serde_json::to_string(&err).unwrap();
        assert_eq!(json, "\"erreur d'entrée/sortie : boum\"");
    }
}
