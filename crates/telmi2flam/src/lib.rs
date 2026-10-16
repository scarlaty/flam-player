//! Convertisseur d'histoires Telmi vers le format flam.
//!
//! Squelette du port Rust de `tools/telmi2flam` (voir `tools/telmi2flam/PORT-RUST.md`) : les
//! modules de conversion arrivent dans les PR suivantes.

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
