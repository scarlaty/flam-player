//! Appareil Flam branché en USB (stockage de masse FAT) : détection (#225), index `list`
//! (#226), écran « Ma Flam » (#227).
//!
//! La logique ne dépend pas de Tauri, sauf les commandes d'un module dédié : la racine de
//! l'appareil est toujours passée en paramètre, ce qui permet de tester sur un faux appareil
//! (dossier temporaire).

pub mod detect;
pub mod index;
