//! Ouverture d'une adresse dans le navigateur par défaut (#323).
//!
//! Le front n'a **aucune** permission du plugin `opener` (voir `capabilities/default.json`) : il
//! n'atteint le navigateur que par la commande `open_external`, qui n'accepte que les pages du
//! dépôt du projet. L'adresse est analysée puis réécrite sous sa forme normalisée avant
//! l'ouverture : les segments `..` ou `%2e%2e` sont résolus et ne permettent pas de sortir du
//! dépôt. Aucun chemin local, aucun autre schéma.

use tauri_plugin_opener::OpenerExt;
use url::Url;

use crate::{AppError, AppResult};

/// Hôte autorisé.
const ALLOWED_HOST: &str = "github.com";
/// Préfixe de chemin autorisé (le dépôt du projet, avec la barre finale).
const ALLOWED_PATH_PREFIX: &str = "/scarlaty/flam-player/";
/// Longueur maximale d'une adresse acceptée.
const MAX_URL_LEN: usize = 2048;

/// Valide une adresse à ouvrir : `https`, hôte `github.com`, port par défaut, sans identifiants,
/// chemin (normalisé) sous `/scarlaty/flam-player/`. Renvoie l'adresse normalisée.
pub fn validate_external_url(raw: &str) -> AppResult<Url> {
    let refused = || AppError::Validation("adresse refusée : lien externe non autorisé".into());
    if raw.len() > MAX_URL_LEN {
        return Err(refused());
    }
    let url = Url::parse(raw).map_err(|_| refused())?;
    let allowed = url.scheme() == "https"
        && url.host_str() == Some(ALLOWED_HOST)
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with(ALLOWED_PATH_PREFIX);
    if allowed {
        Ok(url)
    } else {
        Err(refused())
    }
}

/// Ouvre une page du dépôt du projet dans le navigateur par défaut.
#[tauri::command]
pub async fn open_external(app: tauri::AppHandle, url: String) -> AppResult<()> {
    let url = validate_external_url(&url)?;
    app.opener()
        .open_url(url.as_str(), None::<&str>)
        .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepte_les_pages_du_depot() {
        for raw in [
            "https://github.com/scarlaty/flam-player/blob/main/apps/flam-launcher/docs/flam-transfert.md#risques",
            "https://github.com/scarlaty/flam-player/releases",
            "https://github.com/scarlaty/flam-player/issues/new?template=bug.md",
            // Modèle d'issue des testeurs (#318), ouvert depuis « Ma Flam ».
            "https://github.com/scarlaty/flam-player/issues/new?template=retour-testeur-flam.yml",
            "https://GITHUB.com/scarlaty/flam-player/issues",
            "https://github.com:443/scarlaty/flam-player/issues",
        ] {
            assert!(validate_external_url(raw).is_ok(), "{raw}");
        }
    }

    #[test]
    fn refuse_hors_domaine_schema_et_depot() {
        for raw in [
            "https://example.com/scarlaty/flam-player/",
            "https://github.com.evil.example/scarlaty/flam-player/x",
            "https://evil.example/https://github.com/scarlaty/flam-player/x",
            "https://gist.github.com/scarlaty/flam-player/x",
            "http://github.com/scarlaty/flam-player/issues",
            "javascript:alert(1)",
            "file:///C:/Windows/System32/calc.exe",
            "file://github.com/scarlaty/flam-player/x",
            r"C:\Windows\System32\calc.exe",
            r"\\serveur\partage\x.exe",
            "mailto:a@b.fr",
            "ms-msdt:/id",
            "https://github.com/autre/depot/issues",
            "https://github.com/scarlaty/flam-player-evil/issues",
            "https://github.com/scarlaty/flam-player",
            "https://github.com/scarlaty/",
            "https://github.com/",
            "",
            "   ",
        ] {
            assert!(validate_external_url(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn refuse_les_identifiants_et_les_ports() {
        for raw in [
            "https://user@github.com/scarlaty/flam-player/issues",
            "https://user:mdp@github.com/scarlaty/flam-player/issues",
            "https://github.com@evil.example/scarlaty/flam-player/issues",
            "https://github.com:8443/scarlaty/flam-player/issues",
        ] {
            assert!(validate_external_url(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn refuse_la_sortie_du_depot_par_segments_relatifs() {
        for raw in [
            "https://github.com/scarlaty/flam-player/../autre/depot",
            "https://github.com/scarlaty/flam-player/%2e%2e/autre/depot",
            "https://github.com/scarlaty/flam-player/%2E%2E/%2e%2e/evil",
            r"https://github.com/scarlaty/flam-player/..\..\evil",
            "https://github.com/scarlaty/flam-player/./../x",
        ] {
            assert!(validate_external_url(raw).is_err(), "{raw}");
        }
    }

    #[test]
    fn refuse_une_adresse_demesuree() {
        let raw = format!(
            "https://github.com/scarlaty/flam-player/{}",
            "a".repeat(MAX_URL_LEN)
        );
        assert!(validate_external_url(&raw).is_err());
    }

    #[test]
    fn renvoie_l_adresse_normalisee() {
        let url = validate_external_url("https://GITHUB.com/scarlaty/flam-player/a/./b").unwrap();
        assert_eq!(url.as_str(), "https://github.com/scarlaty/flam-player/a/b");
    }

    #[test]
    fn le_front_n_a_aucune_permission_du_plugin() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/capabilities");
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let content = std::fs::read_to_string(&path).unwrap();
            assert!(!content.contains("opener"), "{}", path.display());
            count += 1;
        }
        assert!(count >= 1);
        let conf = include_str!("../tauri.conf.json");
        assert!(!conf.contains("opener"));
    }
}
