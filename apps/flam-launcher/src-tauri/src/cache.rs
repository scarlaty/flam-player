//! Dossier de cache : taille occupée et vidage.
//!
//! Le dossier de cache peut être choisi par l'utilisateur et contenir autre chose que les
//! données de l'application. Le vidage ne supprime donc **jamais** le contenu du dossier
//! configuré : seuls les sous-dossiers listés dans [`APP_CACHE_SUBDIRS`] (créés par
//! l'application) sont comptés et supprimés. Les liens symboliques et jonctions ne sont jamais
//! suivis ni supprimés. Tout nouveau sous-dossier créé par un module de l'application doit être
//! ajouté à cette liste.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use crate::settings::{resolve, Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::AppResult;

/// Sous-dossiers du cache créés par l'application (et eux seuls) : archives téléchargées,
/// histoires converties, vignettes.
pub const APP_CACHE_SUBDIRS: &[&str] = &["archives", "library", "thumbs"];

/// Réponse de `cache_usage` (miroir de `CacheUsage` dans `src/api/cache.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheUsage {
    /// Dossier de cache effectif.
    pub path: String,
    /// Octets occupés par les sous-dossiers de l'application.
    pub bytes: u64,
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

/// Taille cumulée des fichiers sous `dir`, sans suivre les liens symboliques.
fn dir_size(dir: &Path) -> std::io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        total += if kind.is_dir() {
            dir_size(&entry.path())?
        } else {
            entry.metadata()?.len()
        };
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
/// Tout le reste du dossier est laissé intact.
pub fn clear_of(root: &Path) -> AppResult<u64> {
    let mut freed = 0;
    for dir in app_dirs(root) {
        let size = dir_size(&dir)?;
        fs::remove_dir_all(&dir)?;
        freed += size;
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
    let _guard = state.guard();
    let root = effective_dir(&state)?;
    Ok(CacheUsage {
        bytes: usage_of(&root)?,
        path: root.to_string_lossy().into_owned(),
    })
}

/// Vide le cache : supprime uniquement les sous-dossiers créés par l'application.
#[tauri::command]
pub async fn cache_clear(state: State<'_, ConfigState>) -> AppResult<CacheCleared> {
    let _guard = state.guard();
    let root = effective_dir(&state)?;
    Ok(CacheCleared {
        freed_bytes: clear_of(&root)?,
    })
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
        write(&dir.path().join("documents/important.txt"), 1000);
        write(&dir.path().join("racine.txt"), 500);
        assert_eq!(usage_of(dir.path()).unwrap(), 157);
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
}
