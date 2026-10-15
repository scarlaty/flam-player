//! Commandes générales de l'application (informations, version).

use serde::Serialize;

use crate::AppResult;

/// Informations sur l'application (miroir de `AppInfo` dans `src/api/appInfo.ts`).
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

/// Renvoie le nom et la version de l'application.
#[tauri::command]
pub async fn app_info() -> AppResult<AppInfo> {
    Ok(AppInfo {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn app_info_renvoie_nom_et_version_du_paquet() {
        let info = app_info().await.unwrap();
        assert_eq!(info.name, "flam-launcher");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn app_info_est_serialisee_en_json() {
        let info = AppInfo {
            name: "a".into(),
            version: "1".into(),
        };
        assert_eq!(
            serde_json::to_string(&info).unwrap(),
            r#"{"name":"a","version":"1"}"#
        );
    }
}
