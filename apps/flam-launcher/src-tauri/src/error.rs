use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Erreur commune des commandes Tauri.
///
/// Sérialisée en objet stable `{ "kind": "<code>", "message": "<texte fr>" }`
/// (miroir : `src/api/error.ts`). Chaque variante a un `kind` stable : le front
/// s'appuie dessus, jamais sur le message.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Erreur d'entrée/sortie (fichier, dossier de données…).
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    /// Code stable de l'erreur, utilisé côté front.
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Io(_) => "io",
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("kind", self.kind())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_erreur_est_serialisee_en_objet_kind_message() {
        let err = AppError::from(std::io::Error::other("boum"));
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "kind": "io",
                "message": "erreur d'entrée/sortie : boum"
            })
        );
    }

    #[test]
    fn le_kind_io_est_stable() {
        assert_eq!(AppError::from(std::io::Error::other("x")).kind(), "io");
    }
}
