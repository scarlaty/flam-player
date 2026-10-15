//! Backend Rust de flam-launcher. Voir `apps/flam-launcher/README.md` pour l'architecture.

mod app;
mod error;

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
        .invoke_handler(tauri::generate_handler![app::app_info])
        .run(tauri::generate_context!())
        .expect("erreur au lancement de l'application Tauri");
}
