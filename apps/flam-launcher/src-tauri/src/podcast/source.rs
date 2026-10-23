//! Source « Podcast » (#211) : un flux RSS ajouté par son URL (P1), téléchargé, lu par
//! [`super::rss`] et gardé en cache hors ligne.
//!
//! # URL du flux : un secret (D11)
//!
//! Une URL de flux peut porter un jeton d'abonnement. Elle n'est donc :
//! - **jamais journalisée** ni recopiée dans un message d'erreur (les erreurs `reqwest`, dont
//!   le texte contient l'URL, sont réduites à leur cause) ;
//! - **jamais renvoyée au front** : la liste des sources en donne un indice ([`feed_hint`]),
//!   les épisodes sont désignés par un identifiant opaque (empreinte du `guid`, qui peut être
//!   l'URL de l'enclosure), sans URL d'enclosure ni d'image ;
//! - **jamais écrite dans le cache** : `meta.json` garde son empreinte SHA-256.
//!
//! # Cache
//!
//! Sous `<cache>/podcasts/<source_id>/` :
//! - `feed.xml` : dernier flux **valide** reçu, tel quel ;
//! - `meta.json` : `{ url_sha256, etag, last_modified, sha256, fetched_at }`, écrit après
//!   `feed.xml`.
//!
//! Tout fichier propre à une source podcast (épisodes téléchargés compris) doit vivre sous ce
//! dossier : la suppression de la source le purge entièrement (D15), sans toucher aux
//! histoires installées (P4).
//!
//! Garanties (mêmes règles que le cache des catalogues, `catalog.rs`) :
//! - un cache dont l'empreinte d'URL diffère de celle de la source est ignoré ;
//! - `feed.xml` dont l'empreinte ne correspond pas à `meta.json` (écriture interrompue) reste
//!   servi s'il est lisible, mais sans requête conditionnelle ;
//! - seul un flux lisible remplace le cache : un flux invalide ou sans épisode audio le laisse
//!   intact et l'erreur est signalée avec le cache ;
//! - un seul rafraîchissement à la fois par source ; le portillon d'écriture est celui du
//!   catalogue (vidage du cache) ;
//! - la purge supprime `meta.json` en premier : un dossier à moitié supprimé n'est jamais
//!   relu comme un cache ; un rafraîchissement en attente pendant la purge est abandonné
//!   (compteur de génération).
//!
//! # Adresses privées (D10)
//!
//! Le contrôle des adresses privées et locales appartient au client de téléchargement de #213,
//! absent de `main` à l'écriture de ce module. Tout accès réseau d'une source podcast passe par
//! le client construit par [`feed_client`] : c'est le point où brancher ce contrôle.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use url::Url;

use super::rss::{parse_feed, PodcastFeed, RssError, MAX_FEED_BYTES};
use crate::catalog::{blocking, build_client, CatalogProblem, CatalogStatus};
use crate::settings::{Environment, SettingsStore};
use crate::sources::{validate_name, PublicSource, Source, SourcesStore};
use crate::storage::{self, ConfigState};
use crate::{AppError, AppResult};

/// Longueur maximale de l'URL d'un flux, en octets.
pub const MAX_FEED_URL_LEN: usize = 2048;
/// Délais de connexion et de requête (un flux peut atteindre 16 Mio).
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Sous-dossier du cache (déclaré dans `cache::APP_CACHE_SUBDIRS`).
pub const CACHE_SUBDIR: &str = "podcasts";
const FEED_FILE: &str = "feed.xml";
const META_FILE: &str = "meta.json";
/// Nom d'une source dont le flux n'a pas de titre exploitable.
const DEFAULT_NAME: &str = "Podcast";
const MAX_NAME_CHARS: usize = 100;

/// Valide l'URL d'un flux et la renvoie normalisée (fragment retiré) : http ou https, hôte
/// présent, 2 048 octets au plus, sans identifiants (`utilisateur:mot de passe@`). Les
/// messages d'erreur ne recopient jamais l'URL (D11).
pub fn validate_feed_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Validation(
            "l'adresse du flux ne peut pas être vide".into(),
        ));
    }
    if raw.len() > MAX_FEED_URL_LEN {
        return Err(AppError::Validation(format!(
            "l'adresse du flux ne peut pas dépasser {MAX_FEED_URL_LEN} caractères"
        )));
    }
    let mut url = Url::parse(raw)
        .map_err(|err| AppError::Validation(format!("adresse de flux invalide : {err}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Validation(
            "l'adresse du flux doit commencer par http:// ou https://".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "l'adresse du flux ne doit pas contenir d'identifiants (utilisateur:mot de passe@)"
                .into(),
        ));
    }
    url.set_fragment(None);
    let normalized = String::from(url);
    if normalized.len() > MAX_FEED_URL_LEN {
        return Err(AppError::Validation(format!(
            "l'adresse du flux ne peut pas dépasser {MAX_FEED_URL_LEN} caractères"
        )));
    }
    Ok(normalized)
}

/// Indice affichable d'une URL de flux : schéma et hôte seulement (`https://hôte/…`), sans
/// chemin, port ni paramètres. `…` si l'URL est illisible.
pub fn feed_hint(url: &str) -> String {
    match Url::parse(url) {
        Ok(url) => match url.host_str() {
            Some(host) => format!("{}://{host}/…", url.scheme()),
            None => "…".into(),
        },
        Err(_) => "…".into(),
    }
}

/// Client HTTP des flux podcast : rustls (certificats vérifiés), 5 redirections au plus,
/// jamais de https vers http, aucune redirection par page HTML.
///
/// **Point d'accroche D10 (#213)** : le refus des adresses privées et locales (après
/// résolution DNS et à chaque redirection) s'ajoute ici, en remplaçant ce client par celui de
/// #213. C'est le seul client utilisé par ce module.
pub fn feed_client() -> reqwest::Result<reqwest::Client> {
    build_client(CONNECT_TIMEOUT, REQUEST_TIMEOUT)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Empreinte (SHA-256, hexadécimal) : seule trace de l'URL écrite dans le cache.
fn digest(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Identifiant opaque d'un épisode : 32 premiers chiffres hexadécimaux du SHA-256 du `guid`.
pub fn episode_id(guid: &str) -> String {
    digest(guid)[..32].to_string()
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

// ---------------------------------------------------------------------------------------
// Vues renvoyées au front
// ---------------------------------------------------------------------------------------

/// Épisode d'un flux (miroir à créer côté front avec #212). Aucune URL (D11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EpisodeView {
    /// Identifiant opaque et stable ([`episode_id`]).
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    /// ISO 8601 UTC, `None` si inconnue.
    pub pub_date: Option<String>,
    /// Durée en secondes, si connue.
    pub duration: Option<u32>,
    pub author: Option<String>,
    /// Type MIME de l'enclosure (`audio/…`).
    pub enclosure_type: String,
    /// Taille annoncée de l'enclosure, en octets.
    pub enclosure_length: Option<u64>,
    /// L'épisode (ou le canal) a une image.
    pub has_image: bool,
}

/// Contenu d'un flux, épisodes triés par date décroissante (P5 ; sans date : à la fin, dans
/// l'ordre du flux).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeedView {
    pub title: String,
    pub description: Option<String>,
    pub copyright: Option<String>,
    pub has_image: bool,
    pub episodes: Vec<EpisodeView>,
    /// Épisodes ou éléments ignorés à la lecture.
    pub warnings: Vec<String>,
}

impl From<&PodcastFeed> for FeedView {
    fn from(feed: &PodcastFeed) -> Self {
        let mut episodes: Vec<EpisodeView> = feed
            .episodes
            .iter()
            .map(|e| EpisodeView {
                id: episode_id(&e.guid),
                title: e.title.clone(),
                description: e.description.clone(),
                pub_date: e.pub_date.clone(),
                duration: e.duration,
                author: e.author.clone(),
                enclosure_type: e.enclosure_type.clone(),
                enclosure_length: e.enclosure_length,
                has_image: e.image.is_some(),
            })
            .collect();
        // Tri stable : dates ISO 8601 UTC de même forme, comparables comme du texte.
        episodes.sort_by(|a, b| match (&a.pub_date, &b.pub_date) {
            (Some(x), Some(y)) => y.cmp(x),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });
        FeedView {
            title: feed.title.clone(),
            description: feed.description.clone(),
            copyright: feed.copyright.clone(),
            has_image: feed.image.is_some(),
            episodes,
            warnings: feed.warnings.clone(),
        }
    }
}

/// Flux d'une source et état de fraîcheur (`status` : comme pour un catalogue).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PodcastView {
    pub source_id: String,
    /// `None` : jamais obtenu (aucun cache).
    pub feed: Option<FeedView>,
    pub status: CatalogStatus,
}

/// Réponse de `podcast_add`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PodcastAdded {
    pub source: PublicSource,
    pub view: PodcastView,
}

// ---------------------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------------------

/// Métadonnées du cache d'un flux (`meta.json`). Aucune URL en clair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheMeta {
    /// SHA-256 de l'URL dont provient le flux en cache.
    url_sha256: String,
    etag: Option<String>,
    last_modified: Option<String>,
    /// SHA-256 de `feed.xml` tel qu'écrit.
    sha256: String,
    /// Dernière vérification réussie (secondes Unix), 304 compris.
    fetched_at: u64,
}

struct Cached {
    meta: CacheMeta,
    feed: PodcastFeed,
    /// `feed.xml` correspond à `meta.sha256` : ETag et date utilisables.
    trusted: bool,
}

/// Dossier de cache d'une source (identifiant restreint à un nom de dossier sûr).
pub fn source_dir(cache_root: &Path, source_id: &str) -> AppResult<PathBuf> {
    let safe = !source_id.is_empty()
        && source_id.len() <= 64
        && source_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !safe {
        return Err(AppError::Validation(format!(
            "identifiant de source invalide : {source_id}"
        )));
    }
    Ok(cache_root.join(CACHE_SUBDIR).join(source_id))
}

/// Charge le cache s'il est lisible et provient de l'URL d'empreinte `url_sha256`. Ne reçoit
/// jamais l'URL elle-même : rien de ce qui est journalisé ici ne peut la contenir.
fn load_cache(dir: &Path, source_id: &str, url_sha256: &str) -> Option<Cached> {
    let meta: CacheMeta = match std::fs::read(dir.join(META_FILE)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|err| tracing::warn!("cache du podcast {source_id} : meta illisible ({err})"))
            .ok()?,
        Err(_) => return None,
    };
    if meta.url_sha256 != url_sha256 {
        return None;
    }
    let bytes = std::fs::read(dir.join(FEED_FILE)).ok()?;
    let trusted = hex(&Sha256::digest(&bytes)) == meta.sha256;
    if !trusted {
        tracing::warn!("flux en cache du podcast {source_id} modifié ou incomplet : ETag ignoré");
    }
    let feed = parse_feed(&bytes)
        .map_err(|err| tracing::warn!("flux en cache du podcast {source_id} illisible : {err}"))
        .ok()?;
    Some(Cached {
        meta,
        feed,
        trusted,
    })
}

async fn load_cache_blocking(dir: &Path, source_id: &str, url_sha256: &str) -> Option<Cached> {
    let (dir, sid, hash) = (
        dir.to_path_buf(),
        source_id.to_string(),
        url_sha256.to_string(),
    );
    blocking(move || load_cache(&dir, &sid, &hash))
        .await
        .ok()
        .flatten()
}

/// Écrit `feed.xml` puis `meta.json`, chacun de façon atomique.
fn store_cache(dir: &Path, body: &[u8], meta: &CacheMeta) -> AppResult<()> {
    storage::write_atomic(&dir.join(FEED_FILE), body)?;
    storage::write_json(&dir.join(META_FILE), meta)
}

/// Supprime le dossier de cache d'une source sans suivre de lien : `meta.json` d'abord (un
/// reste n'est alors jamais relu comme un cache), puis le dossier. Refusé si `podcasts/` est
/// un lien ; absent : rien à faire.
fn remove_source_dir(dir: &Path) -> std::io::Result<()> {
    if let Some(parent) = dir.parent() {
        if std::fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(std::io::Error::other(
                "« podcasts » est un lien : suppression refusée",
            ));
        }
    }
    match std::fs::symlink_metadata(dir) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
        Ok(meta) if meta.is_dir() && !meta.file_type().is_symlink() => {
            match std::fs::remove_file(dir.join(META_FILE)) {
                Err(err) if err.kind() != std::io::ErrorKind::NotFound => return Err(err),
                _ => {}
            }
        }
        Ok(_) => {}
    }
    crate::player::remove_tree(dir)
}

fn view_from(
    source_id: &str,
    cached: Option<&Cached>,
    offline: bool,
    error: Option<&AppError>,
) -> PodcastView {
    PodcastView {
        source_id: source_id.to_string(),
        feed: cached.map(|c| FeedView::from(&c.feed)),
        status: CatalogStatus {
            offline,
            updated_at: cached.map(|c| c.meta.fetched_at),
            error: error.map(|e| CatalogProblem {
                kind: e.kind().to_string(),
                message: e.to_string(),
            }),
        },
    }
}

// ---------------------------------------------------------------------------------------
// Réseau
// ---------------------------------------------------------------------------------------

/// Erreur réseau sans l'URL (le texte d'une `reqwest::Error` la contient).
fn network_error(err: &reqwest::Error) -> AppError {
    let reason = if err.is_timeout() {
        "délai dépassé"
    } else if err.is_redirect() {
        "redirection refusée ou trop longue"
    } else if err.is_connect() {
        "connexion impossible"
    } else {
        "échec de la requête"
    };
    AppError::Network(format!("flux inaccessible : {reason}"))
}

fn too_large(size: usize) -> AppError {
    AppError::FeedInvalid(
        RssError::FeedTooLarge {
            size,
            max: MAX_FEED_BYTES,
        }
        .to_string(),
    )
}

/// Lit le corps sans dépasser [`MAX_FEED_BYTES`] (longueur annoncée ou flux en morceaux).
async fn read_body(mut response: reqwest::Response) -> AppResult<Vec<u8>> {
    if let Some(len) = response.content_length() {
        if len > MAX_FEED_BYTES as u64 {
            return Err(too_large(usize::try_from(len).unwrap_or(usize::MAX)));
        }
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network_error(&e))? {
        if body.len() + chunk.len() > MAX_FEED_BYTES {
            return Err(too_large(body.len() + chunk.len()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn header_text(response: &reqwest::Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

/// Flux téléchargé et lu.
pub struct Fetched {
    body: Vec<u8>,
    etag: Option<String>,
    last_modified: Option<String>,
    feed: PodcastFeed,
}

async fn parse_blocking(body: Vec<u8>) -> AppResult<(Vec<u8>, PodcastFeed)> {
    blocking(move || match parse_feed(&body) {
        Ok(feed) => Ok((body, feed)),
        Err(err) => Err(AppError::FeedInvalid(err.to_string())),
    })
    .await?
}

// ---------------------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------------------

/// Service des sources podcast (état Tauri) : client HTTP, verrous et générations par source.
pub struct PodcastService {
    client: reqwest::Client,
    /// Portillon d'écriture du cache, partagé avec le catalogue et le vidage du cache.
    cache_gate: Arc<tokio::sync::RwLock<()>>,
    /// Un verrou par source : un seul rafraîchissement (ou une purge) à la fois.
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Génération de chaque source, incrémentée par la purge.
    generations: Mutex<HashMap<String, u64>>,
}

impl PodcastService {
    pub fn new(client: reqwest::Client, cache_gate: Arc<tokio::sync::RwLock<()>>) -> Self {
        Self {
            client,
            cache_gate,
            locks: Mutex::default(),
            generations: Mutex::default(),
        }
    }

    fn source_lock(&self, source_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.locks)
            .entry(source_id.to_string())
            .or_default()
            .clone()
    }

    /// Génération courante de la source, à lire **avant** de résoudre son URL.
    pub fn generation(&self, source_id: &str) -> u64 {
        lock(&self.generations)
            .get(source_id)
            .copied()
            .unwrap_or_default()
    }

    /// Supprime le cache d'une source (suppression de la source). Attend un rafraîchissement
    /// en cours ; ceux qui attendaient sont abandonnés. Ne touche qu'à `podcasts/<id>/`.
    pub async fn purge_source(&self, cache_root: &Path, source_id: &str) -> AppResult<()> {
        let dir = source_dir(cache_root, source_id)?;
        let _gate = self.cache_gate.read().await;
        let source_lock = self.source_lock(source_id);
        let _held = source_lock.lock().await;
        *lock(&self.generations)
            .entry(source_id.to_string())
            .or_default() += 1;
        blocking(move || remove_source_dir(&dir)).await??;
        Ok(())
    }

    /// Flux en cache, sans réseau (vue vide si jamais obtenu).
    pub async fn cached(
        &self,
        cache_root: &Path,
        source_id: &str,
        url: &str,
    ) -> AppResult<PodcastView> {
        let dir = source_dir(cache_root, source_id)?;
        let cached = load_cache_blocking(&dir, source_id, &digest(url)).await;
        Ok(view_from(source_id, cached.as_ref(), false, None))
    }

    /// Flux en cache lu, sans réseau (`None` si jamais obtenu). Pour la création d'un pack
    /// (#217), qui retrouve les épisodes choisis dans le cache.
    pub async fn cached_feed(
        &self,
        cache_root: &Path,
        source_id: &str,
        url: &str,
    ) -> AppResult<Option<PodcastFeed>> {
        let dir = source_dir(cache_root, source_id)?;
        Ok(load_cache_blocking(&dir, source_id, &digest(url))
            .await
            .map(|c| c.feed))
    }

    async fn send(
        &self,
        url: &str,
        conditional: Option<&CacheMeta>,
    ) -> AppResult<reqwest::Response> {
        let mut request = self.client.get(url);
        if let Some(meta) = conditional {
            if let Some(etag) = &meta.etag {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(date) = &meta.last_modified {
                request = request.header(IF_MODIFIED_SINCE, date);
            }
        }
        request.send().await.map_err(|e| network_error(&e))
    }

    /// Télécharge et lit un flux, sans cache (ajout d'une source). Erreurs : `network`
    /// (réseau, statut HTTP) ou `feed_invalid` (illisible, trop grand, sans épisode audio).
    pub async fn fetch(&self, url: &str) -> AppResult<Fetched> {
        let response = self.send(url, None).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(AppError::Network(format!(
                "flux inaccessible : réponse HTTP {}",
                status.as_u16()
            )));
        }
        let etag = header_text(&response, ETAG);
        let last_modified = header_text(&response, LAST_MODIFIED);
        let (body, feed) = parse_blocking(read_body(response).await?).await?;
        Ok(Fetched {
            body,
            etag,
            last_modified,
            feed,
        })
    }

    /// Écrit le flux `fetched` dans le cache d'une source qui vient d'être enregistrée, sauf si
    /// elle a été purgée depuis la lecture de `generation`. Renvoie la vue du flux.
    pub async fn store_fetched(
        &self,
        generation: u64,
        cache_root: &Path,
        source_id: &str,
        url: &str,
        fetched: Fetched,
        now: u64,
    ) -> AppResult<PodcastView> {
        let dir = source_dir(cache_root, source_id)?;
        let _gate = self.cache_gate.read().await;
        let source_lock = self.source_lock(source_id);
        let _held = source_lock.lock().await;
        let cached = Cached {
            meta: CacheMeta {
                url_sha256: digest(url),
                etag: fetched.etag,
                last_modified: fetched.last_modified,
                sha256: hex(&Sha256::digest(&fetched.body)),
                fetched_at: now,
            },
            feed: fetched.feed,
            trusted: true,
        };
        if self.generation(source_id) == generation {
            let (body, meta) = (fetched.body, cached.meta.clone());
            let stored = blocking(move || store_cache(&dir, &body, &meta)).await?;
            if let Err(err) = stored {
                tracing::warn!("écriture du cache du podcast {source_id} impossible : {err}");
            }
        }
        Ok(view_from(source_id, Some(&cached), false, None))
    }

    /// Télécharge le flux (requête conditionnelle si le cache est fiable) et met le cache à
    /// jour. `generation` est lue par l'appelant avant de résoudre `url` : une source supprimée
    /// entre-temps fait abandonner le rafraîchissement.
    ///
    /// - 200 et flux lisible : nouveau cache ;
    /// - 304 : cache conservé, date de vérification renouvelée ;
    /// - réseau, délai, redirection refusée, statut d'erreur : cache servi, `offline = true` ;
    /// - flux illisible, trop grand ou sans épisode audio : cache servi, `status.error` de
    ///   genre `feed_invalid` ;
    /// - dans ces cas **sans cache** : `Err` (`network` ou `feed_invalid`).
    pub async fn refresh(
        &self,
        generation: u64,
        cache_root: &Path,
        source_id: &str,
        url: &str,
        now: u64,
    ) -> AppResult<PodcastView> {
        let dir = source_dir(cache_root, source_id)?;
        let _gate = self.cache_gate.read().await;
        let source_lock = self.source_lock(source_id);
        let _held = source_lock.lock().await;
        if self.generation(source_id) != generation {
            return Err(AppError::NotFound(
                "source supprimée pendant le rafraîchissement".into(),
            ));
        }
        let url_sha256 = digest(url);
        let cached = load_cache_blocking(&dir, source_id, &url_sha256).await;
        let fail = |cached: Option<Cached>, error: AppError, offline: bool| match cached {
            Some(c) => {
                tracing::warn!("rafraîchissement du podcast {source_id} en échec : {error}");
                Ok(view_from(source_id, Some(&c), offline, Some(&error)))
            }
            None => Err(error),
        };

        let trusted = cached.as_ref().filter(|c| c.trusted).map(|c| &c.meta);
        let mut response = match self.send(url, trusted).await {
            Ok(response) => response,
            Err(err) => return fail(cached, err, true),
        };
        // 304 sans copie de confiance (aucun en-tête conditionnel envoyé) : nouvelle requête.
        if response.status() == StatusCode::NOT_MODIFIED && trusted.is_none() {
            response = match self.send(url, None).await {
                Ok(response) => response,
                Err(err) => return fail(cached, err, true),
            };
        }
        let status = response.status();
        if status == StatusCode::NOT_MODIFIED {
            let Some(mut c) = cached else {
                return Err(AppError::Network(
                    "le serveur répond « inchangé » sans cache local".into(),
                ));
            };
            c.meta.fetched_at = now;
            let (meta_path, meta) = (dir.join(META_FILE), c.meta.clone());
            let written = blocking(move || storage::write_json(&meta_path, &meta)).await?;
            if let Err(err) = written {
                tracing::warn!("mise à jour du cache du podcast {source_id} impossible : {err}");
            }
            return Ok(view_from(source_id, Some(&c), false, None));
        }
        if !status.is_success() {
            let error = AppError::Network(format!(
                "flux inaccessible : réponse HTTP {}",
                status.as_u16()
            ));
            return fail(cached, error, true);
        }
        let etag = header_text(&response, ETAG);
        let last_modified = header_text(&response, LAST_MODIFIED);
        let body = match read_body(response).await {
            Ok(body) => body,
            Err(err) => {
                let offline = matches!(err, AppError::Network(_));
                return fail(cached, err, offline);
            }
        };
        let (body, feed) = match parse_blocking(body).await {
            Ok(parsed) => parsed,
            Err(err) => return fail(cached, err, false),
        };
        let fresh = Cached {
            meta: CacheMeta {
                url_sha256,
                etag,
                last_modified,
                sha256: hex(&Sha256::digest(&body)),
                fetched_at: now,
            },
            feed,
            trusted: true,
        };
        let (dir_owned, meta) = (dir.clone(), fresh.meta.clone());
        let stored = blocking(move || store_cache(&dir_owned, &body, &meta)).await?;
        if let Err(err) = stored {
            tracing::warn!("écriture du cache du podcast {source_id} impossible : {err}");
        }
        Ok(view_from(source_id, Some(&fresh), false, None))
    }
}

// ---------------------------------------------------------------------------------------
// Commandes Tauri
// ---------------------------------------------------------------------------------------

/// Nom par défaut d'une source : titre du flux, sur une ligne, 100 caractères au plus.
fn default_name(feed: &PodcastFeed) -> String {
    let one_line: String = feed
        .title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let name: String = one_line.chars().take(MAX_NAME_CHARS).collect();
    validate_name(&name).unwrap_or_else(|_| DEFAULT_NAME.to_string())
}

fn cache_root(config: &ConfigState) -> AppResult<PathBuf> {
    let _guard = config.guard();
    SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())
}

/// URL de la source podcast `source_id` et dossier de cache effectif.
fn resolve_podcast(config: &ConfigState, source_id: &str) -> AppResult<(String, PathBuf)> {
    let _guard = config.guard();
    let url = match SourcesStore::new(&config.dir).get(source_id)? {
        Source::Podcast { url, .. } => url,
        _ => {
            return Err(AppError::Validation(format!(
                "la source {source_id} n'est pas un podcast"
            )))
        }
    };
    let root = SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?;
    Ok((url, root))
}

/// Purge le cache d'une source podcast supprimée. La source est déjà retirée de
/// `sources.json` : un échec est journalisé (identifiant seulement), le reste n'est jamais relu
/// et part avec « Vider le cache ».
pub(crate) async fn purge_podcast_cache(
    config: &ConfigState,
    service: &PodcastService,
    source_id: &str,
) {
    let purged = match cache_root(config) {
        Ok(root) => service.purge_source(&root, source_id).await,
        Err(err) => Err(err),
    };
    if let Err(err) = purged {
        tracing::warn!("purge du cache du podcast {source_id} impossible : {err}");
    }
}

/// Ajoute une source podcast : l'URL est validée, le flux téléchargé et lu **avant**
/// l'enregistrement (un flux invalide, sans épisode audio, trop grand ou injoignable est
/// refusé). Nom par défaut : titre du flux.
#[tauri::command]
pub async fn podcast_add(
    config: State<'_, ConfigState>,
    service: State<'_, PodcastService>,
    url: String,
    name: Option<String>,
) -> AppResult<PodcastAdded> {
    let url = validate_feed_url(&url)?;
    let name = name
        .filter(|n| !n.trim().is_empty())
        .map(|n| validate_name(&n))
        .transpose()?;
    let root = {
        let _guard = config.guard();
        let already = SourcesStore::new(&config.dir)
            .list()?
            .iter()
            .any(|s| matches!(s, Source::Podcast { url: u, .. } if *u == url));
        if already {
            return Err(AppError::Validation(
                "ce flux podcast est déjà dans vos sources".into(),
            ));
        }
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    let fetched = service.fetch(&url).await?;
    let id = uuid::Uuid::new_v4().to_string();
    let generation = service.generation(&id);
    let name = name.unwrap_or_else(|| default_name(&fetched.feed));
    let source = {
        let _guard = config.guard();
        SourcesStore::new(&config.dir).add_podcast(id.clone(), name, url.clone())?
    };
    let view = service
        .store_fetched(generation, &root, &id, &url, fetched, now_secs())
        .await?;
    Ok(PodcastAdded {
        source: source.into(),
        view,
    })
}

/// Flux en cache d'une source podcast, sans réseau.
#[tauri::command]
pub async fn podcast_feed(
    config: State<'_, ConfigState>,
    service: State<'_, PodcastService>,
    source_id: String,
) -> AppResult<PodcastView> {
    let (url, root) = resolve_podcast(&config, &source_id)?;
    service.cached(&root, &source_id, &url).await
}

/// Rafraîchit le flux d'une source podcast. Sans réseau, renvoie le cache avec
/// `status.offline = true`.
#[tauri::command]
pub async fn podcast_refresh(
    config: State<'_, ConfigState>,
    service: State<'_, PodcastService>,
    source_id: String,
) -> AppResult<PodcastView> {
    // Génération lue avant la résolution de l'URL : voir `PodcastService::refresh`.
    let generation = service.generation(&source_id);
    let (url, root) = resolve_podcast(&config, &source_id)?;
    service
        .refresh(generation, &root, &source_id, &url, now_secs())
        .await
}

#[cfg(test)]
mod tests;
