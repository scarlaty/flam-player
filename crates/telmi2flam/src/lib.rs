//! Convertisseur d'histoires Telmi vers le format flam.
//!
//! Port Rust de `tools/telmi2flam` (voir `tools/telmi2flam/PORT-RUST.md`), en cours : codecs
//! [`lif`] et [`mp3map`], lecture de l'archive ([`archive`], [`info`]), JSON ([`json`]), texte
//! ([`text`]), sérialisation Lua ([`lua`]) et décodage [`png`] ; les autres modules de conversion
//! arrivent dans les PR suivantes.
//!
//! # Pile d'appels
//!
//! Une entrée hostile ne doit jamais abandonner le processus ; or un débordement de pile
//! l'abandonne (pas de panique rattrapable). L'imbrication JSON est bornée à
//! [`json::MAX_DEPTH`] niveaux, et à cette profondeur [`json::load_json`], [`lua::Lua::from_json`],
//! [`lua::lua_repr`] et la destruction de [`json::Value`] et [`lua::Lua`] sont itératives :
//! elles tiennent dans une pile de 256 Kio même en debug (test
//! `profondeur_maximale_dans_une_pile_de_256_kio`). Les `derive` `Clone`, `PartialEq` et
//! `Debug` de ces types restent récursifs et dépassent 1 Mo en debug à cette profondeur.
//!
//! Le reste de la conversion n'a pas cette garantie. **Un appelant de la bibliothèque exécute
//! donc la conversion dans un fil à pile dédiée de 8 Mo** (`std::thread::Builder::stack_size`),
//! comme le binaire `telmi2flam` : le fil principal n'a qu'1 Mo sous Windows.

pub mod archive;
pub mod error;
pub mod info;
pub mod json;
pub mod lif;
pub mod lua;
pub mod mp3map;
pub mod png;
pub mod text;
mod text_tables;
mod zipread;

/// Version de la crate, affichée par `telmi2flam --version`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::VERSION;

    #[test]
    fn la_version_est_celle_de_la_crate() {
        assert_eq!(VERSION, env!("CARGO_PKG_VERSION"));
        assert!(!VERSION.is_empty());
    }
}
