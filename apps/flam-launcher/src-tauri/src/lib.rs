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
        ])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}
