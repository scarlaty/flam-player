//! Convertisseur d'histoires Telmi vers le format flam.
//!
//! Port Rust de `tools/telmi2flam` (voir `tools/telmi2flam/PORT-RUST.md`), en cours : codecs
//! [`lif`] et [`mp3map`] ; les autres modules de conversion arrivent dans les PR suivantes.

pub mod lif;
pub mod mp3map;

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
