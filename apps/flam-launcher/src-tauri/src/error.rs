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
/// | `FolderUnavailable` | `folder_unavailable` | Dossier d'une source locale absent ou inaccessible |
/// | `Network`       | `network`       | Réseau indisponible, délai dépassé ou réponse HTTP en échec |
/// | `CatalogInvalid`| `catalog_invalid`| Catalogue reçu inexploitable (JSON invalide, structure inconnue) |
/// | `FeedInvalid`   | `feed_invalid`  | Flux podcast reçu inexploitable (XML invalide, DTD, trop grand, sans épisode audio) |
/// | `Cancelled`     | `cancelled`     | Tâche d'installation annulée                            |
/// | `DownloadInvalid` | `download_invalid` | Téléchargement incomplet, trop gros ou contenu inattendu |
/// | `ConverterUnavailable` | `converter_unavailable` | `telmi2flam.exe` introuvable |
/// | `Conversion`    | `conversion`    | Échec de `telmi2flam` (code du convertisseur dans le message) |
/// | `PlayerUnavailable` | `player_unavailable` | `flam-player.exe` introuvable ou impossible à lancer |
/// | `PlayerBusy`    | `player_busy`   | Un player est déjà en cours d'exécution                 |
/// | `StoryFileMissing` | `story_file_missing` | Le fichier de l'histoire à lancer a disparu      |
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

    /// Réseau indisponible, délai dépassé ou réponse HTTP en échec (4xx, 5xx).
    #[error("{0}")]
    Network(String),

    /// Catalogue reçu mais inexploitable : JSON invalide ou structure sans liste `data`.
    #[error("{0}")]
    CatalogInvalid(String),

    /// Flux podcast reçu mais inexploitable : XML invalide, DTD refusée, encodage inconnu, flux
    /// trop grand ou sans épisode audio (message distinct pour chaque cas, sans l'URL : D11).
    #[error("{0}")]
    FeedInvalid(String),

    /// Dossier d'une source locale absent ou inaccessible (disque débranché, dossier supprimé).
    #[error("{0}")]
    FolderUnavailable(String),

    /// Tâche d'installation annulée (par l'utilisateur, ou convertisseur interrompu).
    #[error("installation annulée")]
    Cancelled,

    /// Téléchargement inexploitable : taille reçue différente de la taille annoncée, fichier
    /// trop gros, contenu qui n'est pas une archive attendue.
    #[error("{0}")]
    DownloadInvalid(String),

    /// Le convertisseur `telmi2flam.exe` est introuvable.
    #[error("{0}")]
    ConverterUnavailable(String),

    /// Échec de la conversion par `telmi2flam` : `code` est le code stable de l'événement
    /// `error` du convertisseur (`missing_asset`, `invalid_content`…) ou `exit_<n>`.
    #[error("conversion impossible ({code}) : {message}")]
    Conversion { code: String, message: String },

    /// `flam-player.exe` introuvable (ni configuré ni détecté) ou impossible à lancer.
    #[error("{0}")]
    PlayerUnavailable(String),

    /// Un seul player à la fois : un autre est déjà en cours d'exécution.
    #[error("{0}")]
    PlayerBusy(String),

    /// Le fichier (`.plain.pk` ou dossier `.plain`) de l'histoire à lancer n'existe plus.
    #[error("{0}")]
    StoryFileMissing(String),
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
            AppError::Network(_) => "network",
            AppError::CatalogInvalid(_) => "catalog_invalid",
            AppError::FeedInvalid(_) => "feed_invalid",
            AppError::FolderUnavailable(_) => "folder_unavailable",
            AppError::Cancelled => "cancelled",
            AppError::DownloadInvalid(_) => "download_invalid",
            AppError::ConverterUnavailable(_) => "converter_unavailable",
            AppError::Conversion { .. } => "conversion",
            AppError::PlayerUnavailable(_) => "player_unavailable",
            AppError::PlayerBusy(_) => "player_busy",
            AppError::StoryFileMissing(_) => "story_file_missing",
        }
    }

    /// Ajoute `note` au message d'une erreur de lancement, sans changer son `kind`. Seules les
    /// variantes à message libre sont complétées ; les autres sont renvoyées telles quelles.
    pub fn with_note(self, note: &str) -> Self {
        let extend = |message: String| format!("{message} Information sur la progression : {note}");
        match self {
            AppError::Validation(m) => AppError::Validation(extend(m)),
            AppError::NotFound(m) => AppError::NotFound(extend(m)),
            AppError::PlayerUnavailable(m) => AppError::PlayerUnavailable(extend(m)),
            AppError::PlayerBusy(m) => AppError::PlayerBusy(extend(m)),
            AppError::StoryFileMissing(m) => AppError::StoryFileMissing(extend(m)),
            other => other,
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
        assert_eq!(AppError::Network("x".into()).kind(), "network");
        assert_eq!(
            AppError::CatalogInvalid("x".into()).kind(),
            "catalog_invalid"
        );
        assert_eq!(
            AppError::FolderUnavailable("x".into()).kind(),
            "folder_unavailable"
        );
        assert_eq!(AppError::FeedInvalid("x".into()).kind(), "feed_invalid");
        assert_eq!(AppError::Cancelled.kind(), "cancelled");
        assert_eq!(
            AppError::DownloadInvalid("x".into()).kind(),
            "download_invalid"
        );
        assert_eq!(
            AppError::ConverterUnavailable("x".into()).kind(),
            "converter_unavailable"
        );
        assert_eq!(
            AppError::PlayerUnavailable("x".into()).kind(),
            "player_unavailable"
        );
        assert_eq!(AppError::PlayerBusy("x".into()).kind(), "player_busy");
        assert_eq!(
            AppError::StoryFileMissing("x".into()).kind(),
            "story_file_missing"
        );
        let conversion = AppError::Conversion {
            code: "missing_asset".into(),
            message: "m".into(),
        };
        assert_eq!(conversion.kind(), "conversion");
        assert_eq!(
            conversion.to_string(),
            "conversion impossible (missing_asset) : m"
        );
    }

    #[test]
    fn with_note_complete_le_message_sans_changer_le_kind() {
        let err = AppError::StoryFileMissing("fichier absent.".into()).with_note("mises de côté");
        assert_eq!(err.kind(), "story_file_missing");
        assert_eq!(
            err.to_string(),
            "fichier absent. Information sur la progression : mises de côté"
        );
        let cancelled = AppError::Cancelled.with_note("x");
        assert_eq!(cancelled.to_string(), "installation annulée");
    }

    #[test]
    fn le_message_de_validation_est_celui_fourni() {
        let json = serde_json::to_value(AppError::Validation("nom vide".into())).unwrap();
        assert_eq!(json["message"], "nom vide");
    }
}
