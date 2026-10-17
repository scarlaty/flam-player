//! Podcasts : un flux RSS devient une source d'histoires (épopée #207).
//!
//! - `rss` (#210) : lecture d'un flux RSS 2.0 avec extensions iTunes, en logique pure (sans
//!   réseau ni Tauri) ; le contenu est déjà téléchargé.

pub mod rss;
