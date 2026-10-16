//! Dossier de cache : taille occupée et vidage.
//!
//! Le dossier de cache peut être choisi par l'utilisateur et contenir autre chose que les
//! données de l'application. Le vidage ne supprime donc **jamais** le contenu du dossier
//! configuré : seuls les sous-dossiers listés dans [`APP_CACHE_SUBDIRS`] (créés par
//! l'application) sont comptés et supprimés. Les liens symboliques et jonctions ne sont jamais
//! suivis ni supprimés. Tout nouveau sous-dossier créé par un module de l'application doit être
//! ajouté à cette liste.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::catalog::{blocking, CatalogService};
use crate::settings::{resolve, Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::AppResult;

/// Sous-dossiers du cache créés par l'application (et eux seuls) : archives téléchargées, catalogues mis en cache,
/// histoires converties, vignettes.
pub const APP_CACHE_SUBDIRS: &[&str] = &["archives", "catalogs", "library", "thumbs"];

/// Réponse de `cache_usage` (miroir de `CacheUsage` dans `src/api/cache.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheUsage {
    /// Dossier de cache effectif.
    pub path: String,
    /// Octets occupés par les sous-dossiers de l'application.
    pub bytes: u64,
    /// Sous-dossiers que l'application compte et supprime (`APP_CACHE_SUBDIRS`).
    pub subdirs: Vec<String>,
}

/// Réponse de `cache_clear` (miroir de `CacheCleared` dans `src/api/cache.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleared {
    /// Octets libérés.
    pub freed_bytes: u64,
}

/// Sous-dossiers de l'application existant réellement (ni absents, ni liens, ni fichiers).
fn app_dirs(root: &Path) -> Vec<PathBuf> {
    APP_CACHE_SUBDIRS
        .iter()
        .map(|name| root.join(name))
        .filter(|path| {
            fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !meta.is_symlink())
        })
        .collect()
}

/// Profondeur maximale explorée sous un sous-dossier du cache (au-delà, rien n'est compté).
const MAX_DEPTH: usize = 32;

/// Taille cumulée des fichiers sous `dir`, sans suivre les liens symboliques, sur au plus
/// [`MAX_DEPTH`] niveaux. Ce qui disparaît pendant le calcul (`NotFound`) compte pour zéro.
fn dir_size(dir: &Path) -> std::io::Result<u64> {
    dir_size_at(dir, 0)
}

fn dir_size_at(dir: &Path, depth: usize) -> std::io::Result<u64> {
    if depth >= MAX_DEPTH {
        return Ok(0);
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut total = 0;
    for entry in entries {
        let size = entry.and_then(|entry| {
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                Ok(0)
            } else if kind.is_dir() {
                dir_size_at(&entry.path(), depth + 1)
            } else {
                entry.metadata().map(|meta| meta.len())
            }
        });
        match size {
            Ok(size) => total += size,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(total)
}

/// Octets occupés par les sous-dossiers de l'application dans `root`.
pub fn usage_of(root: &Path) -> AppResult<u64> {
    let mut total = 0;
    for dir in app_dirs(root) {
        total += dir_size(&dir)?;
    }
    Ok(total)
}

/// Supprime les sous-dossiers de l'application dans `root` et renvoie les octets libérés.
/// Tout le reste du dossier est laissé intact. Un élément déjà disparu est ignoré.
pub fn clear_of(root: &Path) -> AppResult<u64> {
    let mut freed = 0;
    for dir in app_dirs(root) {
        let size = dir_size(&dir)?;
        match fs::remove_dir_all(&dir) {
            Ok(()) => freed += size,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
    }
    Ok(freed)
}

fn effective_dir(state: &ConfigState) -> AppResult<PathBuf> {
    let settings = SettingsStore::new(&state.dir).load()?;
    let (resolved, _) = resolve(&settings, &Environment::current());
    Ok(PathBuf::from(resolved.cache_dir))
}

/// Dossier de cache effectif et espace occupé par les données de l'application.
#[tauri::command]
pub async fn cache_usage(state: State<'_, ConfigState>) -> AppResult<CacheUsage> {
    let root = {
        let _guard = state.guard();
        effective_dir(&state)?
    };
    let scanned = root.clone();
    let bytes = blocking(move || usage_of(&scanned)).await??;
    Ok(CacheUsage {
        bytes,
        path: root.to_string_lossy().into_owned(),
        subdirs: APP_CACHE_SUBDIRS.iter().map(|s| (*s).to_string()).collect(),
    })
}

/// Vide le cache : supprime uniquement les sous-dossiers créés par l'application. Attend
/// les rafraîchissements de catalogue en cours et bloque les nouveaux pendant le vidage.
#[tauri::command]
pub async fn cache_clear(
    state: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
) -> AppResult<CacheCleared> {
    let _pause = catalog.pause_refreshes().await;
    let root = {
        let _guard = state.guard();
        effective_dir(&state)?
    };
    let freed_bytes = blocking(move || clear_of(&root)).await??;
    Ok(CacheCleared { freed_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn un_cache_absent_ou_vide_occupe_zero_octet() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(usage_of(&dir.path().join("absent")).unwrap(), 0);
        assert_eq!(usage_of(dir.path()).unwrap(), 0);
        assert_eq!(clear_of(dir.path()).unwrap(), 0);
    }

    #[test]
    fn la_taille_ne_compte_que_les_sous_dossiers_de_l_application() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("archives/a.zip"), 100);
        write(&dir.path().join("library/histoire/x/y.bin"), 50);
        write(&dir.path().join("thumbs/t.png"), 7);
        write(&dir.path().join("catalogs/src/catalog.json"), 3);
        write(&dir.path().join("documents/important.txt"), 1000);
        write(&dir.path().join("racine.txt"), 500);
        assert_eq!(usage_of(dir.path()).unwrap(), 160);
    }

    #[test]
    fn le_vidage_ne_supprime_que_les_sous_dossiers_de_l_application() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("archives/a.zip"), 100);
        write(&dir.path().join("library/h/y.bin"), 50);
        write(&dir.path().join("documents/important.txt"), 1000);
        write(&dir.path().join("racine.txt"), 500);
        // Un fichier qui porte le nom d'un sous-dossier n'est pas supprimé.
        write(&dir.path().join("thumbs"), 3);

        assert_eq!(clear_of(dir.path()).unwrap(), 150);

        assert!(!dir.path().join("archives").exists());
        assert!(!dir.path().join("library").exists());
        assert!(dir.path().join("thumbs").is_file());
        assert!(dir.path().join("documents/important.txt").is_file());
        assert!(dir.path().join("racine.txt").is_file());
        assert!(dir.path().exists(), "le dossier configuré reste en place");
        assert_eq!(usage_of(dir.path()).unwrap(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn un_lien_ou_une_jonction_n_est_ni_suivi_ni_supprime() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("precieux.txt"), 10);
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        // Jonction (sans privilège) créée via `mklink /J`.
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(cache.join("library"))
            .arg(outside.path())
            .output()
            .unwrap();
        assert!(status.status.success(), "mklink /J a échoué");

        assert_eq!(usage_of(&cache).unwrap(), 0);
        assert_eq!(clear_of(&cache).unwrap(), 0);
        assert!(outside.path().join("precieux.txt").is_file());
    }

    #[test]
    fn la_profondeur_est_bornee() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = dir.path().join("library");
        for i in 0..(MAX_DEPTH + 5) {
            deep = deep.join(format!("d{i}"));
        }
        write(&deep.join("profond.bin"), 40);
        write(&dir.path().join("library/proche.bin"), 2);
        // Le fichier trop profond n'est pas compté, mais le calcul se termine.
        assert_eq!(usage_of(dir.path()).unwrap(), 2);
        // Le vidage supprime tout, y compris le profond.
        assert_eq!(clear_of(dir.path()).unwrap(), 2);
        assert!(!dir.path().join("library").exists());
    }

    #[test]
    fn un_dossier_disparu_pendant_le_calcul_compte_pour_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dir_size(&dir.path().join("disparu")).unwrap(), 0);
    }

    #[test]
    fn les_sous_dossiers_exposes_sont_ceux_de_l_application() {
        assert_eq!(
            APP_CACHE_SUBDIRS,
            ["archives", "catalogs", "library", "thumbs"]
        );
    }

    #[tokio::test]
    async fn le_vidage_attend_les_rafraichissements_en_cours() {
        let service = CatalogService::with_client(reqwest::Client::new());
        let pause = service.pause_refreshes().await;
        // Un rafraîchissement ne peut pas démarrer pendant le vidage.
        let dir = tempfile::tempdir().unwrap();
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            service.refresh(dir.path(), "src", "http://127.0.0.1:1/", 0),
        )
        .await;
        assert!(blocked.is_err(), "le rafraîchissement doit attendre");
        drop(pause);
    }
}
