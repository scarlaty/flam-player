//! Éditeur d'histoires sur dossier (`docs/creation-dossier.md`).
//!
//! L'histoire **est** le dossier (§ 3) : rien n'est gardé ailleurs que dans les fichiers de
//! l'utilisateur et ses réglages `_histoire.json` / `_dossier.json`.
//!
//! - [`settings`] : lecture tolérante et écriture atomique des réglages (§ 6) ;
//! - [`root`] : racines refusées (disques, système, profil, launcher, Flam), sur chemins
//!   résolus (§ 10) ;
//! - [`tree`] : lecture pure du dossier en arbre de menus et d'histoires, avec rapport de
//!   problèmes (§ 3, § 5.1, § 9, § 10).
//!
//! Lot 2 de la note : aucune commande Tauri, aucune écriture hors `settings::write_*` (qui ne
//! sont pas encore appelées). Conversion, graphe, opérations et front viennent dans les lots
//! suivants (§ 11.2).

pub mod images;
pub mod root;
pub mod settings;
pub mod title_card;
pub mod tree;
