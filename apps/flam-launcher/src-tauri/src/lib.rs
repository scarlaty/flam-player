//! Backend Rust de flam-launcher. Voir `apps/flam-launcher/README.md` pour l'architecture.

mod app;
pub mod cache;
pub mod catalog;
pub mod compat;
pub mod device;
pub mod embed;
mod error;
pub mod external;
pub mod ffmpeg;
pub mod folder;
pub mod install;
pub mod library;
pub mod library_source;
pub mod library_thumb;
pub mod player;
pub mod podcast;
pub mod settings;
pub mod sources;
pub mod storage;
pub mod story;
pub mod thumbnail;
pub mod watch;

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

/// Purge `<cache>/thumbs/` selon la politique par défaut. Un échec n'est jamais bloquant : il est
/// seulement journalisé.
fn purge_stale_thumbs(app: &tauri::AppHandle) {
    let state = app.state::<storage::ConfigState>();
    let cache = {
        let _guard = state.guard();
        settings::SettingsStore::new(&state.dir).cache_dir(&settings::Environment::current())
    };
    let result = cache.map_err(|e| e.to_string()).and_then(|cache| {
        thumbnail::purge_thumbs(
            &cache,
            &thumbnail::PurgePolicy::default(),
            std::time::SystemTime::now(),
        )
        .map_err(|e| e.to_string())
    });
    match result {
        Ok(report) if report.removed > 0 || report.failed > 0 => tracing::info!(
            "purge des vignettes : {} supprimée(s), {} octets libérés, {} échec(s)",
            report.removed,
            report.freed_bytes,
            report.failed
        ),
        Ok(_) => {}
        Err(err) => tracing::warn!("purge des vignettes impossible : {err}"),
    }
}

/// Filtre des traces : `spec` (équivalent de `RUST_LOG`) ou, à défaut, `info`. En `debug`,
/// `reqwest` journalise les URL (redirections, réponses) : celle d'un flux podcast est un
/// secret (D11, #211), d'où la directive `reqwest=info`. Elle ne couvre pas une directive plus
/// précise de l'utilisateur (`RUST_LOG=reqwest::x=debug`), qui la contourne.
pub(crate) fn log_filter(spec: Option<&str>) -> tracing_subscriber::EnvFilter {
    let base = spec
        .and_then(|s| tracing_subscriber::EnvFilter::try_new(s).ok())
        .unwrap_or_else(|| tracing_subscriber::EnvFilter::new("info"));
    base.add_directive("reqwest=info".parse().expect("directive de trace valide"))
}

/// Point d'entrée : initialise les traces puis lance l'application Tauri.
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(log_filter(std::env::var("RUST_LOG").ok().as_deref()))
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
        // Ouverture d'adresses : aucune permission du plugin n'est donnée au front, seule la
        // commande `external::open_external` (liste blanche du dépôt) l'utilise (#323).
        // Le script de clic sur les liens du plugin est désactivé : tout passe par la commande.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
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
            let podcasts = podcast::source::PodcastService::new(
                podcast::net::NetPolicy::strict(),
                catalog.cache_gate(),
            )?;
            app.manage(storage::ConfigState::new(config_dir));
            app.manage(folder::FolderState::default());
            app.manage(watch::WatchState::default());
            app.manage(catalog);
            app.manage(podcasts);
            app.manage(device::official::OfficialCatalog::new()?);
            app.manage(install.clone());
            let player = player::PlayerService::new(Arc::new(player::TauriPlayerEvents(
                app.handle().clone(),
            )));
            // Sous Windows, le player est incrusté dans la fenêtre du launcher (#201).
            #[cfg(windows)]
            let player = player.with_embed(Arc::new(embed::TauriEmbed::new(app.handle().clone())));
            app.manage(player);
            // Purge des vignettes périmées de `<cache>/thumbs/` (#37), hors du démarrage.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || purge_stale_thumbs(&handle));
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
            external::open_external,
            device::carrier::device_carrier_state,
            device::carrier::device_carrier_accept,
            device::carrier::device_carrier_forget,
            device::transfer::device_carrier_preview,
            device::detect::device_detect,
            device::diagnostic::device_diagnostic,
            device::remove::device_remove_story,
            device::reorder::device_reorder_stories,
            device::eject::device_eject,
            device::status::device_status,
            device::status::device_confirm_firmware,
            device::status::device_list_stories,
            device::details::device_story_titles,
            device::details::device_story_thumbnail,
            device::details::device_story_details_cancel,
            device::official::device_official_titles,
            device::official::device_official_thumbnail,
            device::transfer::device_send_story,
            device::transfer::device_cancel_transfer,
            device::transfer::device_recover_transfers,
            compat::compat_statuses,
            podcast::source::podcast_add,
            podcast::source::podcast_feed,
            podcast::source::podcast_refresh,
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
            watch::folder_watch,
            install::install_start,
            install::install_cancel,
            install::install_list,
            library::library_list,
            library::library_remove,
            library_source::library_source_status,
            library_thumb::library_thumbnail,
            player::player_launch,
            player::player_launch_folder,
            player::player_status,
            player::player_stop,
            player::player_embed_layout,
            player::player_embed_focus,
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
            ("device::carrier", include_str!("device/carrier.rs")),
            ("device::diagnostic", include_str!("device/diagnostic.rs")),
            ("device::eject", include_str!("device/eject.rs")),
            ("device::official", include_str!("device/official.rs")),
            ("device::remove", include_str!("device/remove.rs")),
            ("device::status", include_str!("device/status.rs")),
            ("device::transfer", include_str!("device/transfer.rs")),
            ("external", include_str!("external.rs")),
            ("folder", include_str!("folder.rs")),
            ("install", include_str!("install/mod.rs")),
            ("library", include_str!("library.rs")),
            ("player", include_str!("player.rs")),
            ("podcast::source", include_str!("podcast/source.rs")),
            ("settings", include_str!("settings.rs")),
            ("sources", include_str!("sources.rs")),
            ("storage", include_str!("storage.rs")),
            ("watch", include_str!("watch.rs")),
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
