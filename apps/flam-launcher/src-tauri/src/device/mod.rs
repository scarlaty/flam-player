//! Appareil Flam branché en USB : détection (#225), index `list` (#226), écran « Ma Flam » (#227),
//! chiffrement et préparation des histoires (#228).
//!
//! La logique ne dépend pas de Tauri, sauf les commandes d'un module dédié.

pub mod crypto;
pub mod detect;
pub mod prepare;
