//! Éditeur d'histoires sur dossier (`docs/creation-dossier.md`).
//!
//! L'histoire **est** le dossier (§ 3) : rien n'est gardé ailleurs que dans les fichiers de
//! l'utilisateur et ses réglages `_histoire.json` / `_dossier.json` (sauf l'uuid d'un dossier en
//! lecture seule, gardé par le launcher, § 8).
//!
//! - [`settings`] : lecture tolérante et écriture atomique des réglages (§ 6) ;
//! - [`root`] : racines refusées (disques, système, profil, launcher, Flam), sur chemins
//!   résolus (§ 10) ;
//! - [`tree`] : lecture pure du dossier en arbre de menus et d'histoires, avec rapport de
//!   problèmes (§ 3, § 5.1, § 9, § 10) ;
//! - [`build`] : l'arbre devient une histoire (modes, compléments, bornes propres, fabrication
//!   par l'étape commune des packs) ;
//! - [`identity`] : uuid et version (§ 8), dossiers en lecture seule et dossiers copiés ;
//! - [`commands`] : commandes Tauri « lire un dossier » et « créer ou actualiser l'histoire » ;
//! - [`editor_watch`] : surveillance du dossier ouvert dans l'éditeur (§ 4.5) ;
//! - [`ops_commands`] : commandes de la fenêtre de l'éditeur (une fenêtre et un éditeur par
//!   dossier, éléments désignés par ids, vue relue après chaque opération).
//!
//! Lot 5 : un dossier fait à la main devient une histoire, sans éditeur. Opérations sur le
//! disque et éditeur visuel viennent dans les lots suivants (§ 11.2).

pub mod build;
pub mod commands;
pub mod editor_watch;
pub mod identity;
pub mod images;
pub mod ops;
pub mod ops_commands;
pub mod root;
pub mod settings;
pub mod title_card;
pub mod tree;
