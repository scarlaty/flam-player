//! Backend Rust de flam-launcher. Voir `apps/flam-launcher/README.md` pour l'architecture.

mod app;
pub mod cache;
pub mod catalog;
pub mod compat;
mod error;
pub mod folder;
pub mod install;
pub mod library;
pub mod library_thumb;
pub mod lif;
pub mod player;
pub mod settings;
pub mod sources;
pub mod storage;
pub mod story;
pub mod thumbnail;

use std::sync::Arc;

use tauri::Manager;

pub use error::{AppError, AppResult};

/// Ramène la fenêtre principale au premier plan (restaurée si réduite).
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

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
        // Instance unique : doit être le premier plugin enregistré. Un second lancement se
        // termine aussitôt et ramène la fenêtre de la première instance au premier plan, de
        // sorte que deux launchers ne se disputent jamais le cache, `<config>/saves/` ni la
        // file d'installation.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            focus_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let config_dir = storage::config_dir()?;
            let catalog = catalog::CatalogService::new()?;
            let install = install::InstallService::new(install::InstallOptions {
                client: install::download::build_client()?,
                resolver: install::settings_resolver(config_dir.clone()),
                events: Arc::new(install::TauriEvents(app.handle().clone())),
                queue_path: config_dir.join(install::QUEUE_FILE),
                gate: catalog.cache_gate(),
                grace: install::GRACE,
            });
            app.manage(storage::ConfigState::new(config_dir));
            app.manage(folder::FolderState::default());
            app.manage(catalog);
            app.manage(install.clone());
            app.manage(player::PlayerService::new(Arc::new(
                player::TauriPlayerEvents(app.handle().clone()),
            )));
            // Reprise de la file persistée : chaque entrée est ré-résolue depuis sa source.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                install
                    .restore(|source_id, entry_id| {
                        let handle = handle.clone();
                        async move {
                            install::resolve_for_restore(&handle, &source_id, &entry_id).await
                        }
                    })
                    .await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::app_info,
            catalog::catalog_refresh,
            catalog::catalog_entries,
            catalog::catalog_thumbnail,
            cache::cache_usage,
            cache::cache_clear,
            compat::compat_statuses,
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
            install::install_start,
            install::install_cancel,
            install::install_list,
            library::library_list,
            library::library_remove,
            library_thumb::library_thumbnail,
            player::player_launch,
            player::player_launch_folder,
            player::player_status,
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
            ("compat", include_str!("compat.rs")),
            ("folder", include_str!("folder.rs")),
            ("install", include_str!("install/mod.rs")),
            ("library", include_str!("library.rs")),
            ("player", include_str!("player.rs")),
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

    /// Instance unique : le plugin doit être enregistré en premier (consigne du plugin), avant
    /// tout autre, sinon un second launcher démarrerait avant la détection.
    #[test]
    fn l_instance_unique_est_le_premier_plugin() {
        let lib = include_str!("lib.rs");
        let builder = lib.split("tauri::Builder::default()").nth(1).unwrap();
        let first = builder.find(".plugin(").unwrap();
        assert!(builder[first..].starts_with(".plugin(tauri_plugin_single_instance::init("));
    }
}
