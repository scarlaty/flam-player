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
#[cfg(test)]
mod testlog;
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

/// Tâche de démarrage du cache, hors du fil de la fenêtre (#591), dans cet ordre : migration des
/// marqueurs (#570), ouverture de la porte de démarrage (`opener` détruit : les commandes en
/// attente reprennent), lancement de la reprise de la file d'installation (`resume`, aussitôt :
/// une installation lancée ensuite ne réécrit pas la file avant sa relecture), puis nettoyage des
/// dossiers temporaires d'une création interrompue et purge des vignettes périmées (#37). Une
/// panique de la migration n'est rattrapée qu'en test et en développement : la version publiée
/// compile en `panic = "abort"`, le processus s'arrête alors. Le garde `Drop` de `opener` ouvre
/// aussi la porte si cette tâche n'est jamais exécutée.
fn startup_cache_tasks(
    config_dir: &std::path::Path,
    opener: storage::GateOpener,
    app: &tauri::AppHandle,
    resume: impl FnOnce(),
) {
    let env = settings::Environment::current();
    if std::panic::catch_unwind(|| migrate_cache_markers(config_dir, &env)).is_err() {
        tracing::warn!("migration des marqueurs du cache interrompue (panique)");
    }
    drop(opener);
    resume();
    cache::remove_stale_tmp_configured(&app.state::<storage::ConfigState>(), &env);
    purge_stale_thumbs(app);
}

/// Migration des marqueurs du cache (#570), une seule fois, puis à chaque démarrage rattrapage
/// des sous-dossiers qu'elle a refusés faute de preuve (#591). Un échec n'est que journalisé :
/// la migration est rejouée au démarrage suivant.
fn migrate_cache_markers(config_dir: &std::path::Path, env: &settings::Environment) {
    match cache::migrate_configured(config_dir, env) {
        Ok(cache::Migration::Done { marked }) => {
            tracing::info!("cache : {marked} sous-dossier(s) existant(s) marqué(s)")
        }
        Ok(cache::Migration::CaughtUp { marked }) => {
            tracing::info!("cache : {marked} sous-dossier(s) en attente prouvé(s) et marqué(s)")
        }
        Ok(cache::Migration::Deferred) => tracing::info!(
            "cache : dossier configuré absent ou non retenu, marquage reporté au prochain démarrage"
        ),
        Ok(cache::Migration::AlreadyDone) => {}
        Err(err) => tracing::warn!("migration des marqueurs du cache interrompue : {err}"),
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
        // Fermeture de la fenêtre retenue pendant une écriture sur la Flam (#577), attente bornée.
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    device::close::on_close_requested(window, api);
                }
            }
        })
        .setup(|app| {
            let config_dir = storage::config_dir()?;
            // Porte de démarrage (#591) : toute commande (verrou de configuration) et toute tâche
            // d'installation attendent la fin de la migration des marqueurs du cache (#570),
            // faite hors du fil de la fenêtre (voir `startup_cache_tasks`).
            let (gate, opener) = storage::StartupGate::closed();
            let saves_root = config_dir.join(player::SAVES_ROOT);
            let catalog = catalog::CatalogService::new()?;
            let install = install::InstallService::new(install::InstallOptions {
                client: install::download::build_client()?,
                resolver: install::settings_resolver(config_dir.clone(), gate.clone()),
                events: Arc::new(install::TauriEvents(app.handle().clone())),
                queue_path: config_dir.join(install::QUEUE_FILE),
                gate: catalog.cache_gate(),
                grace: install::GRACE,
            });
            let podcasts = podcast::source::PodcastService::new(
                podcast::net::NetPolicy::strict(),
                catalog.cache_gate(),
            )?;
            app.manage(storage::ConfigState::with_gate(config_dir.clone(), gate));
            app.manage(folder::FolderState::default());
            app.manage(watch::WatchState::default());
            app.manage(catalog);
            app.manage(podcasts);
            app.manage(device::official::OfficialCatalog::new()?);
            install.set_pack_backend(Arc::new(install::podcast::LivePackBackend {
                app: app.handle().clone(),
            }));
            app.manage(install.clone());
            let player = player::PlayerService::new(Arc::new(player::TauriPlayerEvents(
                app.handle().clone(),
            )));
            // Sous Windows, le player est incrusté dans la fenêtre du launcher (#201).
            #[cfg(windows)]
            let player = player.with_embed(Arc::new(embed::TauriEmbed::new(app.handle().clone())));
            // Changement de format d'une histoire installée : copie de travail purgée (#556).
            install.set_play_purger(install::play_purger(player.clone(), saves_root));
            app.manage(player);
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let app = handle.clone();
                // Reprise de la file persistée : chaque entrée est ré-résolue depuis sa source.
                let resume = move || {
                    tauri::async_runtime::spawn(async move {
                        install
                            .restore(|source_id, entry_id| {
                                let handle = handle.clone();
                                async move {
                                    install::resolve_for_restore(&handle, &source_id, &entry_id)
                                        .await
                                }
                            })
                            .await;
                    });
                };
                startup_cache_tasks(&config_dir, opener, &app, resume);
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
            device::close::device_write_in_progress,
            device::close::device_close_verdict,
            device::close::device_cancel_close,
            device::transfer::device_carrier_preview,
            device::diagnostic::device_diagnostic,
            device::remove::device_remove_story,
            device::reorder::device_reorder_stories,
            device::eject::device_eject,
            device::status::device_status,
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
            podcast::compositions::pack_composition_delete,
            podcast::compositions::pack_composition_open,
            podcast::compositions::pack_composition_rename,
            podcast::compositions::pack_compositions_list,
            podcast::pack::pack_draft_clear,
            podcast::pack::pack_draft_read,
            podcast::pack::pack_draft_save,
            podcast::pack::pack_validate,
            podcast::thumb::pack_episode_thumbnail,
            install::podcast::pack_create,
            podcast::mp3size::podcast_mp3_cache_size,
            podcast::source::podcast_add,
            podcast::source::podcast_feed,
            podcast::source::podcast_refresh,
            podcast::speech::tts_preview,
            podcast::speech::tts_preview_stop,
            podcast::speech::tts_voices,
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
            device::after_send::library_remove_sent,
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
            ("device::close", include_str!("device/close.rs")),
            ("device::diagnostic", include_str!("device/diagnostic.rs")),
            ("device::eject", include_str!("device/eject.rs")),
            ("device::official", include_str!("device/official.rs")),
            ("device::remove", include_str!("device/remove.rs")),
            ("device::status", include_str!("device/status.rs")),
            ("device::transfer", include_str!("device/transfer.rs")),
            ("external", include_str!("external.rs")),
            ("folder", include_str!("folder.rs")),
            ("install", include_str!("install/mod.rs")),
            ("install::podcast", include_str!("install/podcast.rs")),
            ("library", include_str!("library.rs")),
            ("player", include_str!("player.rs")),
            ("podcast::pack", include_str!("podcast/pack.rs")),
            ("podcast::source", include_str!("podcast/source.rs")),
            ("podcast::speech", include_str!("podcast/speech.rs")),
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
