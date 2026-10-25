//! Aperçu des pochettes d'épisodes (#504, `docs/podcasts.md` § 15) : petite vignette PNG, en
//! URL `data:`, pour le choix de l'image d'une catégorie (N5 c).
//!
//! # Invariants
//!
//! - **Ordre des sources** (#508) : vignette déjà en cache (`podcasts/<source>/thumbs/`, purgée
//!   avec la source), sinon pochette d'origine du cache des MP3 ([`CacheAccess::cached_image`]),
//!   sinon **téléchargement à la demande** de l'image de l'épisode. Sans image, hors ligne,
//!   image refusée ou altérée, le résultat est « pas d'aperçu » (`None`), jamais une erreur.
//! - **Téléchargement** : [`download_image`], la chaîne des couvertures (client gardé D10 :
//!   adresses locales refusées, redirections contrôlées ; taille bornée à [`MAX_INPUT_BYTES`]) ;
//!   délai de [`DOWNLOAD_TIMEOUT`] ; au plus [`DOWNLOADS`] téléchargements simultanés. Le
//!   format est contrôlé par la signature à la conversion (PNG, JPEG, WebP ; SVG, GIF et le
//!   reste sont refusés).
//! - **Décodage isolé** (#263) : les octets viennent d'Internet ; le décodage passe par
//!   [`artwork::convert_isolated`] (processus enfant), jamais dans le processus du launcher.
//! - **Taille bornée** : côté de [`THUMB_DIM`] pixels au plus, résultat de [`MAX_THUMB_BYTES`]
//!   octets au plus, sinon pas d'aperçu.
//! - **Purge** : la génération de la source est lue avant la résolution de son adresse ; une
//!   source supprimée entre-temps n'obtient ni lecture ni écriture de cache.
//! - Aucune URL ni chemin dans les résultats, messages ou traces (D11) : seulement des
//!   identifiants.

use std::future::Future;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, State};
use tokio::sync::watch;

use super::artwork::{self, Artwork, ArtworkError, ArtworkKind, MAX_INPUT_BYTES};
use super::mp3cache::{CacheAccess, EntryKey, LiveLocks, Mode};
use super::pack::EpisodeRef;
use super::rss::PodcastEpisode;
use super::source::{episode_id, PodcastService};
use crate::catalog::blocking;
use crate::install::podcast::download_image;
use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::ConfigState;
use crate::thumbnail;
use crate::AppResult;

/// Côté maximal d'une vignette, en pixels.
pub const THUMB_DIM: u32 = 96;
/// Taille maximale du PNG d'une vignette, en octets (avant l'encodage en base 64).
pub const MAX_THUMB_BYTES: usize = 64 * 1024;
/// Décodages d'aperçus simultanés au plus.
const DECODERS: usize = 2;
/// Téléchargements d'images simultanés au plus (une catégorie de 40 épisodes ne lance pas 40
/// connexions).
const DOWNLOADS: usize = 3;
/// Délai d'un téléchargement d'aperçu (plus court que celui d'une couverture : l'aperçu est
/// facultatif).
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(20);
/// Sous-dossier des vignettes dans le dossier de cache d'une source.
const THUMBS_DIR: &str = "thumbs";
/// Extension des téléchargements en cours et des écritures temporaires.
const TEMP_EXTS: [&str; 2] = ["dl", "part"];
/// Âge au-delà duquel un reste de téléchargement (arrêt brutal) est supprimé.
const STALE_AFTER: Duration = Duration::from_secs(3600);
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Convertisseur de pochette (production : processus enfant isolé).
pub(super) type Convert = fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError>;

/// Vignette PNG de la pochette `bytes` (JPEG, PNG ou WebP), décodée par `convert`. `None` si
/// l'image est illisible ou si la vignette dépasse [`MAX_THUMB_BYTES`]. Bloquant.
pub fn thumbnail_png(
    bytes: &[u8],
    convert: impl Fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError>,
) -> Option<Vec<u8>> {
    let art = convert(bytes, ArtworkKind::Episode)
        .inspect_err(|err| tracing::info!("aperçu de pochette : image illisible ({err})"))
        .ok()?;
    let image = thumbnail::decode_png(&art.png).ok()?;
    let png = thumbnail::encode_png(&thumbnail::fit(image, THUMB_DIM)).ok()?;
    (png.len() <= MAX_THUMB_BYTES).then_some(png)
}

/// Comme [`thumbnail_png`], en URL `data:image/png`.
pub fn thumbnail_data_url(
    bytes: &[u8],
    convert: impl Fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError>,
) -> Option<String> {
    thumbnail_png(bytes, convert).map(|png| thumbnail::data_uri(&png))
}

/// Nom du fichier de vignette : identifiant de l'épisode et empreinte de son adresse d'image
/// (une image changée dans le flux invalide la vignette). `None` si l'un des deux est inattendu
/// (jamais un nom venu d'ailleurs que de [`EntryKey`]).
fn thumb_name(key: &EntryKey) -> Option<String> {
    let safe = |s: &str| {
        !s.is_empty()
            && s.len() <= 64
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    };
    let fingerprint = key.image_fingerprint.as_deref()?.get(..16)?;
    (safe(&key.episode_id) && safe(fingerprint))
        .then(|| format!("{}-{fingerprint}.png", key.episode_id))
}

/// Vignette en cache : vrai fichier PNG de [`MAX_THUMB_BYTES`] octets au plus, jamais un lien.
fn read_thumb(dir: &Path, name: &str) -> Option<Vec<u8>> {
    let path = dir.join(name);
    let meta = std::fs::symlink_metadata(&path).ok()?;
    if !meta.is_file() || meta.len() > MAX_THUMB_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    bytes.starts_with(PNG_SIGNATURE).then_some(bytes)
}

/// Écrit la vignette (fichier temporaire puis renommage) et supprime les restes anciens de
/// téléchargements interrompus.
fn write_thumb(dir: &Path, name: &str, png: &[u8]) -> std::io::Result<()> {
    sweep_stale(dir);
    remove_outdated(dir, name);
    let tmp = dir.join(format!("{name}.part"));
    std::fs::write(&tmp, png)?;
    std::fs::rename(&tmp, dir.join(name)).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Supprime les vignettes du même épisode que `name` portant une autre empreinte d'image
/// (image changée dans le flux) : le dossier ne grossit pas à chaque changement.
fn remove_outdated(dir: &Path, name: &str) {
    let Some((episode, _)) = name.rsplit_once('-') else {
        return;
    };
    let prefix = format!("{episode}-");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if file != name && file.starts_with(&prefix) && file.ends_with(".png") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn sweep_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let temp = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| TEMP_EXTS.contains(&e));
        let old = entry
            .metadata()
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > STALE_AFTER);
        if temp && old {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Octets de l'image téléchargée dans `path` (au plus [`MAX_INPUT_BYTES`]), puis fichier
/// supprimé. Bloquant.
fn take_download(path: &Path) -> Option<Vec<u8>> {
    let bytes = (|| {
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_INPUT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() <= MAX_INPUT_BYTES).then_some(bytes)
    })();
    let _ = std::fs::remove_file(path);
    bytes
}

/// Vignette (URL `data:`) de l'épisode `key` : cache des vignettes, sinon pochette du cache
/// des MP3, sinon `download` (appelé avec le fichier de destination, rend `true` si l'image y a
/// été écrite). La vignette obtenue est mise en cache. `None` : pas d'aperçu.
pub(super) async fn load<D, Fut>(
    access: &CacheAccess,
    key: &EntryKey,
    download: D,
    convert: Convert,
) -> Option<String>
where
    D: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = bool>,
{
    let name = thumb_name(key)?;
    let source_id = key.source_id.as_str();
    let wanted = name.clone();
    let cached = access
        .in_subdir(THUMBS_DIR, source_id, Mode::Read, move |dir| {
            read_thumb(dir, &wanted)
        })
        .await
        .flatten();
    if let Some(png) = cached {
        return Some(thumbnail::data_uri(&png));
    }
    let bytes = match access.cached_image(key, MAX_INPUT_BYTES as u64).await {
        Some(bytes) => bytes,
        None => fetch_bytes(access, key, download).await?,
    };
    // Au plus DECODERS décodages à la fois (chaque aperçu lance un processus enfant) : une
    // catégorie de 40 épisodes ne déclenche pas 40 décodeurs simultanés.
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(DECODERS);
    let _permit = SLOTS.acquire().await.ok()?;
    let png = match blocking(move || thumbnail_png(&bytes, convert)).await {
        Ok(png) => png?,
        Err(err) => {
            tracing::warn!("aperçu de pochette : tâche de décodage interrompue : {err}");
            return None;
        }
    };
    drop(_permit);
    let (stored, to_write) = (name, png.clone());
    let written = access
        .in_subdir(THUMBS_DIR, source_id, Mode::Write, move |dir| {
            write_thumb(dir, &stored, &to_write)
        })
        .await;
    if let Some(Err(err)) = written {
        tracing::warn!("aperçu de pochette : vignette non mise en cache : {err}");
    }
    Some(thumbnail::data_uri(&png))
}

/// Télécharge l'image de l'épisode dans le dossier des vignettes de sa source (créé sous le
/// verrou de la source : une source purgée n'en récupère rien), au plus [`DOWNLOADS`] à la fois.
async fn fetch_bytes<D, Fut>(access: &CacheAccess, key: &EntryKey, download: D) -> Option<Vec<u8>>
where
    D: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = bool>,
{
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(DOWNLOADS);
    let dir = access
        .in_subdir(THUMBS_DIR, &key.source_id, Mode::Write, |dir| {
            dir.to_path_buf()
        })
        .await?;
    // Nom unique : deux demandes simultanées pour le même épisode ne s'écrasent pas.
    let dest = dir.join(format!(
        "{}.{}.dl",
        key.episode_id,
        uuid::Uuid::new_v4().simple()
    ));
    let _permit = SLOTS.acquire().await.ok()?;
    let ok = download(dest.clone()).await;
    drop(_permit);
    let read = blocking(move || take_download(&dest)).await.ok().flatten();
    if ok {
        read
    } else {
        None
    }
}

/// Téléchargement réel : chaîne des couvertures ([`download_image`]), délai de
/// [`DOWNLOAD_TIMEOUT`]. Rien n'est journalisé de l'adresse (D11).
async fn download_from_web(url: String, dest: PathBuf) -> bool {
    let (_keep, cancel) = watch::channel(false);
    let policy = super::net::NetPolicy::strict();
    matches!(
        tokio::time::timeout(
            DOWNLOAD_TIMEOUT,
            download_image(policy, &url, &dest, cancel)
        )
        .await,
        Ok(Ok(true))
    )
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

/// Aperçu de la pochette d'un épisode : cache des vignettes, cache des MP3, sinon image
/// téléchargée à la demande (#508). `Ok(None)` : pas d'aperçu (le front affiche alors le titre
/// seul).
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
    // Génération lue **avant** de résoudre l'adresse du flux : une purge ensuite rend le cache
    // inaccessible (voir `CacheAccess::new`).
    let generation = service.generation(&episode.source_id);
    let Some((root, found)) = find_episode(&config, &service, &episode).await? else {
        return Ok(None);
    };
    let Some(image) = found.image.clone() else {
        return Ok(None);
    };
    let key = EntryKey::of(&episode.source_id, &found);
    let access = CacheAccess::new(Arc::new(LiveLocks(app)), root, [episode.source_id.clone()])
        .at_generation(&episode.source_id, generation);
    Ok(load(
        &access,
        &key,
        |dest| download_from_web(image, dest),
        |b, kind| artwork::convert_isolated(b, kind, None),
    )
    .await)
}

#[cfg(test)]
mod tests;
