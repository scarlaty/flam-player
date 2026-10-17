//! Podcasts : un flux RSS devient une source d'histoires (épopée #207).
//!
//! Logique pure (sans réseau ni Tauri), découpée en sous-modules :
//! - `artwork` (#215) : conversion des pochettes de podcast en PNG ;
//! - `rss` (#210) : lecture d'un flux RSS 2.0 avec extensions iTunes ; le contenu est déjà
//!   téléchargé.

pub mod artwork;
pub mod rss;
