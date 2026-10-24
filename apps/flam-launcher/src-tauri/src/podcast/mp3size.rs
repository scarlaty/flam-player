//! Taille du cache des MP3 de podcasts, affichée à part dans les paramètres (#489,
//! `docs/podcasts.md` § 13.6).
//!
//! Les MP3 convertis vivent sous `<cache>/podcasts/<source_id>/mp3/` (voir
//! [`super::mp3cache`]). Leur taille est déjà comprise dans celle de `cache_usage` (le
//! sous-dossier `podcasts` fait partie de `APP_CACHE_SUBDIRS`) : cette commande n'en donne que la
//! part, pour que l'écran des paramètres puisse la montrer séparément. Aucun lien symbolique ni
//! aucune jonction n'est suivi, comme pour le reste du cache.

use std::fs;
use std::path::Path;

use serde::Serialize;
use tauri::State;

use crate::cache::{dir_size, effective_dir};
use crate::catalog::blocking;
use crate::storage::ConfigState;
use crate::AppResult;

use super::mp3cache::CACHE_DIR;
use super::source::CACHE_SUBDIR;

/// Réponse de `podcast_mp3_cache_size` (miroir de `Mp3CacheUsage` dans `src/api/cache.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Mp3CacheUsage {
    /// Octets occupés par les dossiers `podcasts/<source>/mp3/` de toutes les sources.
    pub bytes: u64,
}

/// Vrai dossier : ni lien symbolique ni jonction.
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !meta.is_symlink())
}

/// Octets occupés par `<cache_root>/podcasts/*/mp3/`. Un dossier absent, un lien ou un fichier
/// à la place d'un dossier compte pour zéro ; ce qui disparaît pendant le calcul aussi.
pub fn mp3_cache_bytes(cache_root: &Path) -> AppResult<u64> {
    let podcasts = cache_root.join(CACHE_SUBDIR);
    if !is_real_dir(&podcasts) {
        return Ok(0);
    }
    let sources = match fs::read_dir(&podcasts) {
        Ok(sources) => sources,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
    };
    let mut total = 0;
    for source in sources {
        let source = match source {
            Ok(source) => source.path(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        let mp3 = source.join(CACHE_DIR);
        if is_real_dir(&source) && is_real_dir(&mp3) {
            total += dir_size(&mp3)?;
        }
    }
    Ok(total)
}

/// Octets occupés par le cache des MP3 de podcasts (déjà compris dans `cache_usage`).
#[tauri::command]
pub async fn podcast_mp3_cache_size(state: State<'_, ConfigState>) -> AppResult<Mp3CacheUsage> {
    let root = {
        let _guard = state.guard();
        effective_dir(&state)?
    };
    let bytes = blocking(move || mp3_cache_bytes(&root)).await??;
    Ok(Mp3CacheUsage { bytes })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    #[test]
    fn un_cache_absent_ou_sans_mp3_occupe_zero_octet() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(mp3_cache_bytes(&dir.path().join("absent")).unwrap(), 0);
        assert_eq!(mp3_cache_bytes(dir.path()).unwrap(), 0);
        write(&dir.path().join("podcasts/source-a/feed.xml"), 40);
        assert_eq!(mp3_cache_bytes(dir.path()).unwrap(), 0);
    }

    #[test]
    fn compte_les_mp3_de_toutes_les_sources_et_rien_d_autre() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("podcasts/source-a/mp3/0a.mp3"), 100);
        write(&dir.path().join("podcasts/source-a/mp3/0a.img"), 20);
        write(&dir.path().join("podcasts/source-a/mp3/0a.json"), 3);
        write(&dir.path().join("podcasts/source-a/mp3/0b.mp3.part"), 7);
        write(&dir.path().join("podcasts/source-b/mp3/1c.mp3"), 50);
        // Hors `mp3/` : flux, métadonnées, autres sous-dossiers du cache.
        write(&dir.path().join("podcasts/source-a/feed.xml"), 1000);
        write(&dir.path().join("podcasts/source-a/meta.json"), 500);
        write(&dir.path().join("podcasts/source-c/feed.xml"), 300);
        write(&dir.path().join("thumbs/t.png"), 700);
        write(&dir.path().join("mp3/intrus.mp3"), 900);
        assert_eq!(mp3_cache_bytes(dir.path()).unwrap(), 180);
    }

    #[test]
    fn la_part_des_mp3_est_comprise_dans_l_usage_du_cache() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("podcasts/source-a/mp3/0a.mp3"), 100);
        write(&dir.path().join("podcasts/source-a/feed.xml"), 10);
        write(&dir.path().join("archives/a.zip"), 5);
        let mp3 = mp3_cache_bytes(dir.path()).unwrap();
        assert_eq!(mp3, 100);
        assert_eq!(crate::cache::usage_of(dir.path()).unwrap(), 115);
    }

    #[test]
    fn un_fichier_a_la_place_d_un_dossier_compte_pour_zero() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("podcasts/source-a/mp3"), 100);
        write(&dir.path().join("podcasts/source-b"), 100);
        assert_eq!(mp3_cache_bytes(dir.path()).unwrap(), 0);
        let other = tempfile::tempdir().unwrap();
        write(&other.path().join("podcasts"), 100);
        assert_eq!(mp3_cache_bytes(other.path()).unwrap(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn un_lien_vers_un_dossier_n_est_pas_suivi() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("gros.mp3"), 1000);
        write(&dir.path().join("podcasts/source-a/mp3/0a.mp3"), 10);
        // La création d'un lien symbolique peut être refusée sans privilège : test sans effet.
        if std::os::windows::fs::symlink_dir(outside.path(), dir.path().join("podcasts/source-b"))
            .is_ok()
        {
            write(&outside.path().join("mp3/0a.mp3"), 1000);
            assert_eq!(mp3_cache_bytes(dir.path()).unwrap(), 10);
        }
    }
}
