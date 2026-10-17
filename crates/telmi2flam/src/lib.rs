//! Convertisseur d'histoires Telmi vers le format flam.
//!
//! Port Rust de `tools/telmi2flam` (voir `tools/telmi2flam/PORT-RUST.md`), en cours : codecs
//! [`lif`] et [`mp3map`], lecture de l'archive ([`archive`], [`info`]), JSON ([`json`]), texte
//! ([`text`]), sérialisation Lua ([`lua`]), décodage [`png`], redimensionnement [`resample`]
//! (port de Pillow) et conversions d'images en LIF ([`image`]) ; les autres modules de
//! conversion arrivent dans les PR suivantes.

pub mod archive;
pub mod error;
pub mod image;
pub mod info;
pub mod json;
pub mod lif;
pub mod lua;
pub mod mp3map;
pub mod png;
pub mod resample;
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
