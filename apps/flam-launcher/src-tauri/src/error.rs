use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

/// Erreur commune des commandes Tauri.
///
/// Sérialisée en objet stable `{ "kind": "<code>", "message": "<texte fr>" }`
/// (miroir : `src/api/error.ts`). Chaque variante a un `kind` stable : le front
/// s'appuie dessus, jamais sur le message.
///
/// | Variante        | `kind`          | Sens                                                    |
/// | --------------- | --------------- | ------------------------------------------------------- |
/// | `Io`            | `io`            | Erreur d'entrée/sortie                                  |
/// | `Validation`    | `validation`    | Valeur refusée (URL, chemin, nom) ou opération interdite |
/// | `NotFound`      | `not_found`     | Élément introuvable (source inconnue…)                  |
/// | `ConfigVersion` | `config_version`| Fichier de config d'une version plus récente            |
/// | `ConfigDir`     | `config_dir`    | Dossier de configuration introuvable                    |
/// | `Serialization` | `serialization` | Échec de sérialisation JSON                             |
///
/// Un fichier de configuration corrompu n'est pas une erreur : il est sauvegardé en
/// `*.bak` et remplacé par les valeurs par défaut (avertissement tracé).
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Erreur d'entrée/sortie (fichier, dossier de données…).
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] std::io::Error),

    /// Valeur refusée par la validation (URL, chemin, nom, opération interdite…).
    #[error("{0}")]
    Validation(String),

    /// Élément introuvable (source inconnue…).
    #[error("{0}")]
    NotFound(String),

    /// Fichier de configuration écrit par une version plus récente de l'application.
    /// Il n'est ni modifié ni remplacé.
    #[error(
        "le fichier {file} utilise la version de schéma {found}, \
         plus récente que celle prise en charge ({supported})"
    )]
    ConfigVersion {
        file: String,
        found: u64,
        supported: u32,
    },

    /// Dossier de configuration de l'application introuvable.
    #[error("dossier de configuration introuvable")]
    ConfigDir,

    /// Échec de sérialisation JSON d'une configuration.
    #[error("erreur de sérialisation JSON : {0}")]
    Serialization(#[from] serde_json::Error),
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    /// Code stable de l'erreur, utilisé côté front.
    pub fn kind(&self) -> &'static str {
        match self {
            AppError::Io(_) => "io",
            AppError::Validation(_) => "validation",
            AppError::NotFound(_) => "not_found",
            AppError::ConfigVersion { .. } => "config_version",
            AppError::ConfigDir => "config_dir",
            AppError::Serialization(_) => "serialization",
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

    #[test]
    fn les_kinds_sont_stables() {
        assert_eq!(AppError::Validation("x".into()).kind(), "validation");
        assert_eq!(AppError::NotFound("x".into()).kind(), "not_found");
        let version = AppError::ConfigVersion {
            file: "f".into(),
            found: 9,
            supported: 1,
        };
        assert_eq!(version.kind(), "config_version");
        assert_eq!(AppError::ConfigDir.kind(), "config_dir");
        let json_err = serde_json::from_str::<u8>("x").unwrap_err();
        assert_eq!(AppError::from(json_err).kind(), "serialization");
    }

    #[test]
    fn le_message_de_validation_est_celui_fourni() {
        let json = serde_json::to_value(AppError::Validation("nom vide".into())).unwrap();
        assert_eq!(json["message"], "nom vide");
    }
}
