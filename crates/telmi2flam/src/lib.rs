//! Convertisseur d'histoires Telmi vers le format flam.
//!
//! Port Rust de `tools/telmi2flam` (voir `tools/telmi2flam/PORT-RUST.md`), en cours : codecs
//! [`lif`] et [`mp3map`], lecture de l'archive ([`archive`], [`info`]), JSON ([`json`]), texte
//! ([`text`]), sérialisation Lua ([`lua`]), décodage [`png`], redimensionnement [`resample`]
//! (port de Pillow), conversions d'images en LIF ([`image`]) et graphe `nodes.json` vers Lua
//! ([`graph`]), assets et assemblage du paquet en mémoire ([`assets`], [`package`], runtime
//! embarqué [`runtime`]), écriture atomique de la sortie ([`output`]), conversion complète
//! ([`convert`]), événements de progression ([`progress`]) et jeton d'annulation du binaire
//! ([`cancel`]).
//!
//! # Pile d'appels
//!
//! Une entrée hostile ne doit jamais abandonner le processus ; or un débordement de pile
//! l'abandonne (pas de panique rattrapable). L'imbrication JSON est bornée à
//! [`json::MAX_DEPTH`] niveaux, et à cette profondeur [`json::load_json`], [`lua::Lua::from_json`],
//! [`lua::lua_repr`] et la destruction de [`json::Value`] et [`lua::Lua`] sont itératives :
//! elles tiennent dans une pile de 256 Kio même en debug (test
//! `profondeur_maximale_dans_une_pile_de_256_kio`) et s'appellent sur n'importe quel fil.
//! Les `derive` `Clone`, `PartialEq` et `Debug` de ces types restent récursifs : mesuré en
//! debug à 512 niveaux d'objets, plus de 1 Mo de pile (moins de 8 Mo). Ne pas les appliquer à
//! une valeur issue d'une entrée hors d'un fil de 8 Mo.
//!
//! Le reste de la conversion (graphe, ressources) n'a pas cette garantie. **Un appelant qui
//! lance une conversion complète l'exécute donc dans un fil à pile dédiée de 8 Mo**
//! (`std::thread::Builder::stack_size`), comme le binaire `telmi2flam` : le fil principal n'a
//! qu'1 Mo sous Windows. Les codecs seuls (par exemple [`lif`]) n'en ont pas besoin.

pub mod archive;
pub mod assets;
mod blur;
pub mod cancel;
pub mod cli;
pub mod convert;
pub mod error;
pub mod ffmpeg;
pub mod graph;
mod identify;
pub mod image;
pub mod info;
pub mod json;
pub mod lif;
pub mod lua;
mod md5;
pub mod mp3map;
pub mod output;
pub mod package;
pub mod png;
pub mod progress;
pub mod resample;
pub mod runtime;
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
