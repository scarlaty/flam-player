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
use crate::catalog::{blocking, CatalogService};
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

/// `dir` (`<cache>/podcasts/<source>/thumbs`) est dans un `podcasts/` qui porte le marqueur de
/// l'application (#582, #591) : sinon, rien n'y est balayé ni supprimé automatiquement.
fn in_owned_podcasts(dir: &Path) -> bool {
    dir.parent()
        .and_then(Path::parent)
        .is_some_and(crate::cache::is_owned)
}

/// Supprime les vignettes du même épisode que `name` portant une autre empreinte d'image
/// (image changée dans le flux) : le dossier ne grossit pas à chaque changement. Rien n'est
/// supprimé hors d'un `podcasts/` marqué ([`in_owned_podcasts`]).
fn remove_outdated(dir: &Path, name: &str) {
    if !in_owned_podcasts(dir) {
        return;
    }
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

/// Supprime les restes de téléchargements (`*.dl`, `*.part`) de plus de [`STALE_AFTER`]. Rien
/// n'est lu ni supprimé hors d'un `podcasts/` marqué ([`in_owned_podcasts`]).
fn sweep_stale(dir: &Path) {
    if !in_owned_podcasts(dir) {
        return;
    }
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

/// [`load_gated`] sans portillon du cache (tests).
#[cfg(test)]
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
    load_gated(access, key, None, download, convert).await
}

/// Vignette (URL `data:`) de l'épisode `key` : cache des vignettes, sinon pochette du cache
/// des MP3, sinon `download` (appelé avec le fichier de destination, hors du cache, rend `true`
/// si l'image y a été écrite). La vignette obtenue est mise en cache. `None` : pas d'aperçu.
///
/// `gate` (tenu en lecture à l'appel) couvre les seules lectures et écritures du cache : relâché
/// avant le téléchargement et le décodage, repris pour écrire la vignette (#582). L'écriture ne
/// crée aucun dossier : si un vidage est passé entre-temps, la vignette n'est pas mise en cache.
pub(super) async fn load_gated<D, Fut>(
    access: &CacheAccess,
    key: &EntryKey,
    mut gate: Option<Gate>,
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
    let from_cache = access.cached_image(key, MAX_INPUT_BYTES as u64).await;
    if from_cache.is_none() {
        prepare_dir(access, key).await?;
    }
    if let Some(gate) = gate.as_mut() {
        gate.release();
    }
    let bytes = match from_cache {
        Some(bytes) => bytes,
        None => fetch_bytes(download).await?,
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
    if let Some(gate) = gate.as_mut() {
        gate.reacquire().await;
    }
    let (stored, to_write) = (name, png.clone());
    let written = access
        .in_subdir(THUMBS_DIR, source_id, Mode::Read, move |dir| {
            write_thumb(dir, &stored, &to_write)
        })
        .await;
    drop(gate);
    if let Some(Err(err)) = written {
        tracing::warn!("aperçu de pochette : vignette non mise en cache : {err}");
    }
    Some(thumbnail::data_uri(&png))
}

/// Vrai la première fois de la session que `dir` (dossier des vignettes d'une source) reçoit un
/// téléchargement : le balayage des restes ne se refait pas à chaque téléchargement.
fn first_fetch(dir: &Path) -> bool {
    static SWEPT: std::sync::Mutex<Option<std::collections::HashSet<PathBuf>>> =
        std::sync::Mutex::new(None);
    let mut swept = SWEPT.lock().unwrap_or_else(|e| e.into_inner());
    swept
        .get_or_insert_with(Default::default)
        .insert(dir.to_path_buf())
}

/// Portillon du cache ([`crate::catalog::CatalogService::cache_gate`]) tenu en lecture par
/// l'aperçu, relâchable : « Vider le cache » (écriture) n'attend ainsi que les sections qui lisent
/// ou écrivent le cache, jamais une attente réseau (#582).
pub(super) struct Gate {
    lock: Arc<tokio::sync::RwLock<()>>,
    held: Option<tokio::sync::OwnedRwLockReadGuard<()>>,
}

impl Gate {
    /// `held` : garde de lecture déjà pris sur `lock`.
    pub(super) fn new(
        lock: Arc<tokio::sync::RwLock<()>>,
        held: tokio::sync::OwnedRwLockReadGuard<()>,
    ) -> Self {
        Self {
            lock,
            held: Some(held),
        }
    }

    fn release(&mut self) {
        self.held = None;
    }

    /// Reprend le portillon (en attendant la fin d'un vidage en cours). Rien d'autre ne doit être
    /// tenu pendant l'attente.
    async fn reacquire(&mut self) {
        self.held = Some(self.lock.clone().read_owned().await);
    }
}

/// Fichier de destination d'un téléchargement : **hors du cache** (dossier temporaire du
/// système, nom aléatoire). Le portillon du cache est relâché pendant le téléchargement : un
/// vidage ne peut ni le supprimer sous l'écriture en cours ni le voir recréer un dossier purgé.
fn download_dest() -> PathBuf {
    std::env::temp_dir().join(format!("flam-thumb-{}.dl", uuid::Uuid::new_v4().simple()))
}

/// Prépare le dossier des vignettes de la source, sous le verrou de la source : une source déjà
/// purgée n'obtient rien (`None`), et les restes d'anciens téléchargements abandonnés (arrêt
/// brutal) sont balayés une fois par source et par session (le dossier est lu en entier).
async fn prepare_dir(access: &CacheAccess, key: &EntryKey) -> Option<()> {
    access
        .in_subdir(THUMBS_DIR, &key.source_id, Mode::Write, |dir| {
            if first_fetch(dir) {
                sweep_stale(dir);
            }
        })
        .await
}

/// Télécharge l'image de l'épisode (hors du cache, voir [`download_dest`]), au plus
/// [`DOWNLOADS`] à la fois.
async fn fetch_bytes<D, Fut>(download: D) -> Option<Vec<u8>>
where
    D: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = bool>,
{
    static SLOTS: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(DOWNLOADS);
    let dest = download_dest();
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
    bounded_download(
        &dest,
        DOWNLOAD_TIMEOUT,
        download_image(policy, &url, &dest, cancel),
    )
    .await
}

/// Exécute `download` (qui écrit `dest`) en `limit` au plus. Au-delà, le futur est abandonné sans
/// avoir pu nettoyer : le fichier partiel (`<dest>.part`) est supprimé ici.
async fn bounded_download(
    dest: &Path,
    limit: Duration,
    download: impl Future<Output = AppResult<bool>>,
) -> bool {
    match tokio::time::timeout(limit, download).await {
        Ok(result) => matches!(result, Ok(true)),
        Err(_) => {
            let _ = tokio::fs::remove_file(crate::install::download::part_path(dest)).await;
            false
        }
    }
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
    catalog: State<'_, CatalogService>,
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
    // Portillon du cache en lecture, comme les autres écrivains du cache : « Vider le cache »
    // (écriture) attend la fin des sections qui lisent ou écrivent le cache. Il est relâché par
    // `load_gated` pendant l'attente d'un créneau, le téléchargement (écrit hors du cache) et le
    // décodage, puis repris pour l'écriture : le pire cas n'est plus de l'ordre de 40 s (#582).
    // Rien de ce qui suit ne reprend ce portillon en le tenant déjà (`find_episode`,
    // `CacheAccess` et `load_gated` : une relecture imbriquée pendant qu'un vidage attend
    // s'interbloquerait), et l'appelant n'en tient aucun.
    let gate = Gate::new(
        catalog.cache_gate(),
        catalog.cache_gate().read_owned().await,
    );
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
    Ok(load_gated(
        &access,
        &key,
        Some(gate),
        |dest| download_from_web(image, dest),
        |b, kind| artwork::convert_isolated(b, kind, None),
    )
    .await)
}

#[cfg(test)]
mod tests;
