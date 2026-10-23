//! Podcasts : un flux RSS devient une source d'histoires (épopée #207).
//!
//! Sous-modules `artwork` et `rss` : logique pure (sans réseau ni Tauri). `source` : réseau,
//! cache et commandes Tauri.
//! - `artwork` (#215) : conversion des pochettes de podcast en PNG ;
//! - `rss` (#210) : lecture d'un flux RSS 2.0 avec extensions iTunes ; le contenu est déjà
//!   téléchargé ;
//! - `source` (#211) : source « Podcast » (URL du flux, cache hors ligne, commandes).

pub mod artwork;
pub mod rss;
pub mod source;
