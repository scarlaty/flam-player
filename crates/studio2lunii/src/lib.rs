//! Conversion d'une histoire STUdio (`story.json` + `assets/`) en pack Lunii, le format que la
//! Flam lit à partir du firmware 2.x (épopée #517, note de conception `docs/studio-lunii.md`).
//!
//! Logique pure, sans accès à l'appareil ni à Tauri : le launcher fournit les octets lus dans
//! l'archive et reçoit les fichiers du pack à transférer.
//!
//! Modules développés en parallèle, chacun dans son fichier :
//!
//! - [`story`] : lecture et validation de `story.json` ;
//! - [`index`] : production des index binaires `ni`, `li`, `ri`, `si` ;
//! - [`bitmap`] : images au format attendu par l'appareil (BMP 4 bits RLE) ;
//! - [`audio`] : contrôle des MP3 et retrait des étiquettes.

pub mod audio;
pub mod bitmap;
pub mod index;
pub mod story;
