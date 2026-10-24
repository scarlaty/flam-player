//! Aperçu des pochettes d'épisodes (#504, `docs/podcasts.md` § 15) : petite vignette PNG, en
//! URL `data:`, pour le choix de l'image d'une catégorie (N5 c).
//!
//! # Invariants
//!
//! - **Aucun accès réseau** : la pochette vient du cache des MP3 ([`CacheAccess::cached_image`],
//!   pochette d'origine gardée avec l'épisode converti) ; sans entrée de cache, sans pochette ou
//!   avec une pochette altérée, le résultat est « pas d'aperçu » (`None`), jamais une erreur.
//! - **Décodage isolé** (#263) : les octets viennent d'Internet ; le décodage passe par
//!   [`artwork::convert_isolated`] (processus enfant), jamais dans le processus du launcher.
//! - **Taille bornée** : côté de [`THUMB_DIM`] pixels au plus, résultat de [`MAX_THUMB_BYTES`]
//!   octets au plus, sinon pas d'aperçu.
//! - Aucune URL ni chemin dans les résultats, messages ou traces (D11) : seulement des
//!   identifiants.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::{AppHandle, State};

use super::artwork::{self, Artwork, ArtworkError, ArtworkKind, MAX_INPUT_BYTES};
use super::mp3cache::{CacheAccess, EntryKey, LiveLocks};
use super::pack::EpisodeRef;
use super::rss::PodcastEpisode;
use super::source::{episode_id, PodcastService};
use crate::catalog::blocking;
use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::ConfigState;
use crate::thumbnail;
use crate::AppResult;

/// Côté maximal d'une vignette, en pixels.
pub const THUMB_DIM: u32 = 96;
/// Taille maximale du PNG d'une vignette, en octets (avant l'encodage en base 64).
pub const MAX_THUMB_BYTES: usize = 64 * 1024;

/// Vignette (URL `data:image/png`) de la pochette `bytes` (JPEG, PNG ou WebP), décodée par
/// `convert`. `None` si l'image est illisible ou si la vignette dépasse [`MAX_THUMB_BYTES`].
/// Bloquant.
pub fn thumbnail_data_url(
    bytes: &[u8],
    convert: impl Fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError>,
) -> Option<String> {
    let art = convert(bytes, ArtworkKind::Episode)
        .inspect_err(|err| tracing::info!("aperçu de pochette : image illisible ({err})"))
        .ok()?;
    let image = thumbnail::decode_png(&art.png).ok()?;
    let png = thumbnail::encode_png(&thumbnail::fit(image, THUMB_DIM)).ok()?;
    (png.len() <= MAX_THUMB_BYTES).then(|| thumbnail::data_uri(&png))
}

/// Épisode `episode` du flux en cache de la source, avec la racine du cache. `None` : source
/// inconnue (ou qui n'est pas un podcast), flux jamais obtenu ou illisible, épisode absent.
async fn find_episode(
    config: &ConfigState,
    service: &PodcastService,
    episode: &EpisodeRef,
) -> AppResult<Option<(PathBuf, PodcastEpisode)>> {
    let (url, root) = {
        let _guard = config.guard();
        let url = SourcesStore::new(&config.dir)
            .list()?
            .into_iter()
            .find_map(|s| match s {
                Source::Podcast { id, url, .. } if id == episode.source_id => Some(url),
                _ => None,
            });
        let Some(url) = url else {
            return Ok(None);
        };
        let root = SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?;
        (url, root)
    };
    let feed = match service.cached_feed(&root, &episode.source_id, &url).await {
        Ok(Some(feed)) => feed,
        Ok(None) => return Ok(None),
        Err(err) => {
            tracing::warn!(
                "aperçu de pochette : flux en cache de la source {} illisible : {err}",
                episode.source_id
            );
            return Ok(None);
        }
    };
    Ok(feed
        .episodes
        .into_iter()
        .find(|e| episode_id(&e.guid) == episode.episode_id)
        .map(|e| (root, e)))
}

/// Aperçu de la pochette d'un épisode, depuis le cache des MP3 (sans réseau). `Ok(None)` :
/// pas d'aperçu (le front affiche alors le titre seul).
#[tauri::command]
pub async fn pack_episode_thumbnail(
    app: AppHandle,
    config: State<'_, ConfigState>,
    service: State<'_, PodcastService>,
    source_id: String,
    episode_id: String,
) -> AppResult<Option<String>> {
    let episode = EpisodeRef {
        source_id,
        episode_id,
    };
    if !super::pack::valid_id(&episode.source_id) || !super::pack::valid_id(&episode.episode_id) {
        return Ok(None);
    }
    let Some((root, found)) = find_episode(&config, &service, &episode).await? else {
        return Ok(None);
    };
    if found.image.is_none() {
        return Ok(None);
    }
    let key = EntryKey::of(&episode.source_id, &found);
    let access = CacheAccess::new(Arc::new(LiveLocks(app)), root, [episode.source_id.clone()]);
    let Some(bytes) = access.cached_image(&key, MAX_INPUT_BYTES as u64).await else {
        return Ok(None);
    };
    let url = blocking(move || {
        thumbnail_data_url(&bytes, |b, kind| artwork::convert_isolated(b, kind, None))
    })
    .await
    .ok()
    .flatten();
    Ok(url)
}

#[cfg(test)]
mod tests;
