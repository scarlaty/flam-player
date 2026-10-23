//! Podcasts : un flux RSS devient une source d'histoires (épopée #207).
//!
//! Sous-modules `archive`, `artwork` et `rss` : logique pure (sans réseau ni Tauri). `source` :
//! réseau, cache et commandes Tauri.
//! - `archive` (#216) : archive Telmi synthétique d'un pack (graphe de menus, médias) ;
//! - `artwork` (#215) : conversion des pochettes de podcast en PNG ;
//! - `audio` (#214) : conversion des épisodes en MP3 44 100 Hz par ffmpeg (règle D6) ;
//! - `rss` (#210) : lecture d'un flux RSS 2.0 avec extensions iTunes ; le contenu est déjà
//!   téléchargé ;
//! - `net` (#213) : contrôle des adresses (D10) et client HTTP gardé, pour tout accès réseau ;
//! - `fetch` (#213) : téléchargement par lot des épisodes et pochettes d'un pack ;
//! - `source` (#211) : source « Podcast » (URL du flux, cache hors ligne, commandes).

pub mod archive;
pub mod artwork;
pub mod audio;
pub mod fetch;
pub mod net;
pub mod rss;
pub mod source;
