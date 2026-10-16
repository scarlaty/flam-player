//! Backend Rust de flam-launcher. Voir `apps/flam-launcher/README.md` pour l'architecture.

mod app;
pub mod catalog;
mod error;
pub mod folder;
pub mod lif;
pub mod settings;
pub mod sources;
pub mod storage;
pub mod story;
pub mod thumbnail;

use tauri::Manager;

pub use error::{AppError, AppResult};

/// Point d'entrée : initialise les traces puis lance l'application Tauri.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    tracing::info!("démarrage de flam-launcher {}", env!("CARGO_PKG_VERSION"));

    tauri::Builder::default()
        .setup(|app| {
            app.manage(storage::ConfigState::new(storage::config_dir()?));
            app.manage(folder::FolderState::default());
            app.manage(catalog::CatalogService::new()?);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::app_info,
            catalog::catalog_refresh,
            catalog::catalog_entries,
            catalog::catalog_thumbnail,
            storage::config_ack_recovery,
            settings::settings_get,
            settings::settings_set,
            sources::sources_list,
            sources::sources_add,
            sources::sources_update,
            sources::sources_remove,
            sources::sources_reorder,
            folder::folder_scan,
            folder::folder_thumbnail,
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}

#[cfg(test)]
mod tests {
    /// Toute commande `#[tauri::command]` d'un module doit être enregistrée dans
    /// `generate_handler!` : sinon l'appel du front échoue à l'exécution (« command not found »).
    #[test]
    fn toutes_les_commandes_sont_enregistrees() {
        let lib = include_str!("lib.rs");
        let modules = [
            ("app", include_str!("app.rs")),
            ("catalog", include_str!("catalog.rs")),
            ("folder", include_str!("folder.rs")),
            ("settings", include_str!("settings.rs")),
            ("sources", include_str!("sources.rs")),
            ("storage", include_str!("storage.rs")),
        ];
        let mut count = 0;
        for (module, source) in modules {
            let mut lines = source.lines();
            while let Some(line) = lines.next() {
                if line.trim() != "#[tauri::command]" {
                    continue;
                }
                let signature = lines.next().unwrap_or_default();
                let name = signature
                    .split("fn ")
                    .nth(1)
                    .and_then(|rest| rest.split('(').next())
                    .unwrap_or_default();
                assert!(
                    lib.contains(&format!("{module}::{name},")),
                    "commande {module}::{name} non enregistrée dans generate_handler!"
                );
                count += 1;
            }
        }
        assert!(count >= 10, "commandes détectées : {count}");
    }
}
