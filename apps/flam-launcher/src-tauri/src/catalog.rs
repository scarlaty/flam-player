//! Source « catalogue » : téléchargement d'un catalogue JSON au format Telmi Store, lecture
//! tolérante, normalisation en `StoryEntry`, cache hors ligne et vignettes.
//!
//! La logique (`CatalogService`, `parse_catalog`) ne dépend pas de Tauri : le client HTTP et le
//! dossier de cache sont injectés, ce qui permet de la tester contre un serveur HTTP local. Les
//! commandes `catalog_*` ne sont que de fines enveloppes.
//!
//! # Cache
//!
//! Sous `<cache>/catalogs/<source_id>/` :
//! - `catalog.json` : dernier catalogue **valide** reçu, tel quel ;
//! - `meta.json` : `{ url, etag, last_modified, sha256, fetched_at }` (écrit après
//!   `catalog.json`) ;
//! - `thumbs/<sha256 de l'URL>.img` : vignettes téléchargées à la demande.
//!
//! Garanties de cohérence :
//! - un cache dont l'URL diffère de celle de la source (URL modifiée) est ignoré ;
//! - `meta.json` contient le SHA-256 de `catalog.json` : s'il ne correspond pas (écriture
//!   interrompue, fichier modifié), le catalogue reste lisible mais l'ETag et la date sont
//!   ignorés (pas de requête conditionnelle : téléchargement complet) ;
//! - un seul rafraîchissement à la fois par source (verrou par source) ;
//! - un catalogue **valide mais sans aucune entrée exploitable** ne remplace pas un cache non
//!   vide : le cache est conservé et `status.error` l'indique.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::redirect::{Attempt, Policy};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use tokio::sync::Semaphore;
use url::Url;

use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::{self, ConfigState};
use crate::story::{validate_story_id, StoryEntry, StoryKind};
use crate::{AppError, AppResult};

/// Délai de connexion et délai total d'une requête.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Nombre maximal de redirections suivies.
const MAX_REDIRECTS: usize = 5;
/// Taille maximale acceptée pour un catalogue et pour une vignette.
const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
const MAX_THUMB_BYTES: usize = 2 * 1024 * 1024;
/// Nombre maximal d'entrées lues dans un catalogue (les suivantes sont ignorées).
const MAX_ENTRIES: usize = 10_000;
/// Nombre maximal d'avertissements détaillés conservés (le reste est résumé).
const MAX_WARNINGS: usize = 50;
/// Téléchargements de vignettes simultanés.
const THUMB_CONCURRENCY: usize = 6;
/// Âge au-delà duquel une vignette en cache est revalidée auprès du serveur.
const THUMB_TTL: Duration = Duration::from_secs(7 * 24 * 3600);
/// Délai avant de retenter la revalidation d'une vignette qui vient d'échouer.
const THUMB_RETRY_SECS: u64 = 600;
const CACHE_FILE: &str = "catalog.json";
const META_FILE: &str = "meta.json";

/// Décide si une redirection est suivie : 5 sauts au plus, jamais de https vers http.
fn redirect_check(previous: &[Url], next: &Url) -> Result<(), &'static str> {
    if previous.len() > MAX_REDIRECTS {
        return Err("trop de redirections");
    }
    if previous.last().is_some_and(|p| p.scheme() == "https") && next.scheme() != "https" {
        return Err("redirection de https vers http refusée");
    }
    Ok(())
}

pub(crate) fn redirect_policy(attempt: Attempt<'_>) -> reqwest::redirect::Action {
    match redirect_check(attempt.previous(), attempt.url()) {
        Ok(()) => attempt.follow(),
        Err(reason) => attempt.error(reason),
    }
}

/// Client `reqwest` (rustls, sans OpenSSL natif) avec délais, redirections bornées et
/// `User-Agent` explicite.
pub fn build_client(connect: Duration, total: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(connect)
        .timeout(total)
        .redirect(Policy::custom(redirect_policy))
        .user_agent(USER_AGENT)
        .build()
}

/// `User-Agent` de toutes les requêtes de l'application.
pub(crate) const USER_AGENT: &str = concat!(
    "flam-launcher/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/scarlaty/flam-player)"
);

/// Exécute un travail bloquant (fichiers, JSON) hors du runtime asynchrone.
pub(crate) async fn blocking<T: Send + 'static>(
    job: impl FnOnce() -> T + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(job)
        .await
        .map_err(|err| AppError::Io(std::io::Error::other(err.to_string())))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// URL du catalogue et `thumb_url` connues pour cette URL.
type KnownThumbs = (String, Arc<HashSet<String>>);

/// Service du catalogue (état Tauri) : client HTTP et état partagé en mémoire.
pub struct CatalogService {
    client: reqwest::Client,
    /// Un verrou par source : un seul rafraîchissement à la fois.
    refresh_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Par source : (URL du catalogue, `thumb_url` connues). Évite de relire le catalogue à
    /// chaque vignette ; remplacé à chaque rafraîchissement.
    known_thumbs: Mutex<HashMap<String, KnownThumbs>>,
    /// Limite les téléchargements de vignettes simultanés.
    thumb_slots: Semaphore,
    /// Âge maximal d'une vignette en cache avant revalidation.
    thumb_ttl: Duration,
    /// Dernier échec de revalidation par vignette (secondes Unix) : évite de retenter à chaque
    /// affichage quand le serveur est injoignable.
    thumb_failures: Mutex<HashMap<PathBuf, u64>>,
    /// Portillon d'écriture dans le cache : les rafraîchissements le prennent en lecture (ils
    /// restent parallèles entre sources), le vidage du cache en écriture. Partagé avec le
    /// pipeline d'installation (`install`), qui le tient en lecture pendant chaque tâche.
    cache_gate: Arc<tokio::sync::RwLock<()>>,
}

impl CatalogService {
    pub fn new() -> reqwest::Result<Self> {
        build_client(CONNECT_TIMEOUT, REQUEST_TIMEOUT).map(Self::with_client)
    }

    pub fn with_client(client: reqwest::Client) -> Self {
        Self::with_limits(client, THUMB_CONCURRENCY)
    }

    pub fn with_limits(client: reqwest::Client, thumb_concurrency: usize) -> Self {
        Self {
            client,
            refresh_locks: Mutex::default(),
            known_thumbs: Mutex::default(),
            thumb_slots: Semaphore::new(thumb_concurrency),
            thumb_ttl: THUMB_TTL,
            thumb_failures: Mutex::default(),
            cache_gate: Arc::default(),
        }
    }

    /// Supprime le cache disque (`<cache>/catalogs/<source_id>/`) et l'état en mémoire d'une
    /// source : suppression de la source ou changement de son URL. Attend un rafraîchissement
    /// en cours de cette source. Ne touche qu'à ce sous-dossier (identifiant validé), sans
    /// suivre de lien ; absent : rien à faire.
    pub async fn purge_source(&self, cache_root: &Path, source_id: &str) -> AppResult<()> {
        let dir = source_dir(cache_root, source_id)?;
        let _gate = self.cache_gate.read().await;
        let source_lock = self.source_lock(source_id);
        let _held = source_lock.lock().await;
        lock(&self.known_thumbs).remove(source_id);
        blocking(move || remove_source_dir(&dir)).await??;
        Ok(())
    }

    /// Attend la fin des rafraîchissements en cours et empêche d'en démarrer (ou d'en écrire
    /// le résultat) tant que le garde renvoyé est conservé. Sert au vidage du cache.
    pub async fn pause_refreshes(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.cache_gate.write().await
    }

    /// Portillon d'écriture dans le cache, à prendre en lecture par tout module qui écrit dans
    /// le cache (le vidage le prend en écriture).
    pub fn cache_gate(&self) -> Arc<tokio::sync::RwLock<()>> {
        self.cache_gate.clone()
    }

    fn source_lock(&self, source_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        lock(&self.refresh_locks)
            .entry(source_id.to_string())
            .or_default()
            .clone()
    }

    fn set_known_thumbs(
        &self,
        source_id: &str,
        url: &str,
        entries: &[StoryEntry],
    ) -> Arc<HashSet<String>> {
        let set: Arc<HashSet<String>> =
            Arc::new(entries.iter().filter_map(|e| e.thumb_url.clone()).collect());
        lock(&self.known_thumbs).insert(source_id.to_string(), (url.to_string(), set.clone()));
        set
    }

    /// `thumb_url` connues de la source : mémoire, sinon lecture (unique) du cache disque.
    async fn known_thumbs(&self, dir: &Path, source_id: &str, url: &str) -> Arc<HashSet<String>> {
        if let Some((cached_url, set)) = lock(&self.known_thumbs).get(source_id) {
            if cached_url == url {
                return set.clone();
            }
        }
        match load_cache_blocking(dir, source_id, url).await {
            Some(c) => self.set_known_thumbs(source_id, url, &c.parsed.entries),
            None => Arc::default(),
        }
    }

    /// Entrées du dernier catalogue en cache, sans accès réseau (vide si jamais téléchargé).
    pub async fn entries(
        &self,
        cache_root: &Path,
        source_id: &str,
        url: &str,
    ) -> AppResult<CatalogView> {
        let dir = source_dir(cache_root, source_id)?;
        let cached = load_cache_blocking(&dir, source_id, url).await;
        if let Some(c) = &cached {
            self.set_known_thumbs(source_id, url, &c.parsed.entries);
        }
        Ok(view_from(source_id, cached, false, None))
    }

    /// Télécharge le catalogue (requête conditionnelle ETag / If-Modified-Since si un cache de
    /// confiance existe) et met le cache à jour. Un seul rafraîchissement à la fois par source.
    ///
    /// - 200 valide : nouveau cache ;
    /// - 304 : cache conservé, date de vérification mise à jour ;
    /// - échec réseau, délai, redirection refusée, statut HTTP d'erreur : cache servi avec
    ///   `offline = true` ;
    /// - 200 mais catalogue inexploitable, ou valide mais **vide** alors que le cache ne l'est
    ///   pas : cache servi avec `error.kind = "catalog_invalid"` ;
    /// - dans ces cas **sans cache** : `Err` (`network` ou `catalog_invalid`).
    pub async fn refresh(
        &self,
        cache_root: &Path,
        source_id: &str,
        url: &str,
        now: u64,
    ) -> AppResult<CatalogView> {
        let dir = source_dir(cache_root, source_id)?;
        let _gate = self.cache_gate.read().await;
        let source_lock = self.source_lock(source_id);
        let _held = source_lock.lock().await;
        let view = self.refresh_locked(&dir, source_id, url, now).await?;
        let known = self.set_known_thumbs(source_id, url, &view.entries);
        // Le catalogue vient d'être confirmé : les vignettes qu'il n'utilise plus sont inutiles.
        if !view.status.offline && view.status.error.is_none() {
            let _ = blocking(move || {
                if let Err(err) = prune_thumbs(&dir, &known) {
                    tracing::warn!("nettoyage des vignettes impossible : {err}");
                }
            })
            .await;
        }
        Ok(view)
    }

    async fn refresh_locked(
        &self,
        dir: &Path,
        source_id: &str,
        url: &str,
        now: u64,
    ) -> AppResult<CatalogView> {
        let cached = load_cache_blocking(dir, source_id, url).await;

        let mut request = self.client.get(url);
        if let Some(c) = cached.as_ref().filter(|c| c.trusted) {
            if let Some(etag) = &c.meta.etag {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(date) = &c.meta.last_modified {
                request = request.header(IF_MODIFIED_SINCE, date);
            }
        }

        // Échec : cache s'il existe, erreur sinon.
        let fail = |cached: Option<Cached>, error: AppError, offline: bool| match cached {
            Some(c) => {
                tracing::warn!("rafraîchissement du catalogue {source_id} en échec : {error}");
                Ok(view_from(source_id, Some(c), offline, Some(&error)))
            }
            None => Err(error),
        };

        let response = match request.send().await {
            Ok(response) => response,
            Err(err) => return fail(cached, network_error(&err), true),
        };
        let status = response.status();

        if status == StatusCode::NOT_MODIFIED {
            if let Some(mut c) = cached {
                c.meta.fetched_at = now;
                let (meta_path, meta) = (dir.join(META_FILE), c.meta.clone());
                let written = blocking(move || storage::write_json(&meta_path, &meta)).await?;
                if let Err(err) = written {
                    tracing::warn!("mise à jour du cache impossible ({source_id}) : {err}");
                }
                return Ok(view_from(source_id, Some(c), false, None));
            }
            return Err(AppError::Network(
                "le serveur répond « inchangé » sans cache local".into(),
            ));
        }
        if !status.is_success() {
            let error =
                AppError::Network(format!("catalogue inaccessible : réponse HTTP {status}"));
            return fail(cached, error, true);
        }

        let etag = header_text(&response, ETAG);
        let last_modified = header_text(&response, LAST_MODIFIED);
        let body = match read_body(response, MAX_CATALOG_BYTES).await {
            Ok(body) => body,
            Err(err) => return fail(cached, err, true),
        };
        let (sid, curl) = (source_id.to_string(), url.to_string());
        let parsed = blocking(move || {
            let sha256 = hex(&Sha256::digest(&body));
            parse_catalog(&sid, &curl, &body).map(|parsed| (body, sha256, parsed))
        })
        .await?;
        let (body, sha256, parsed) = match parsed {
            Ok(ok) => ok,
            Err(err) => return fail(cached, err, false),
        };
        if parsed.entries.is_empty()
            && cached
                .as_ref()
                .is_some_and(|c| !c.parsed.entries.is_empty())
        {
            let error = AppError::CatalogInvalid(
                "le serveur a renvoyé un catalogue sans histoire exploitable : \
                 le cache précédent est conservé"
                    .into(),
            );
            return fail(cached, error, false);
        }
        let meta = CacheMeta {
            url: url.to_string(),
            etag,
            last_modified,
            sha256,
            fetched_at: now,
        };
        // Un cache non écrit ne doit pas masquer un catalogue valide fraîchement reçu.
        let (dir_owned, meta_owned) = (dir.to_path_buf(), meta.clone());
        let stored = blocking(move || store_cache(&dir_owned, &body, &meta_owned)).await?;
        if let Err(err) = stored {
            tracing::warn!("écriture du cache impossible ({source_id}) : {err}");
        }
        Ok(view_from(
            source_id,
            Some(Cached {
                meta,
                parsed,
                trusted: true,
            }),
            false,
            None,
        ))
    }

    /// Renvoie la vignette `thumb_url` d'une histoire du catalogue sous forme d'URL `data:`
    /// (compatible avec la CSP, sans élargir le protocole d'asset). Téléchargée à la première
    /// demande (6 téléchargements simultanés au plus) puis servie depuis
    /// `catalogs/<source_id>/thumbs/`.
    ///
    /// `thumb_url` doit être celle d'une entrée du catalogue en cache (ensemble gardé en
    /// mémoire, remplacé à chaque rafraîchissement) : le front ne peut pas faire télécharger une
    /// URL arbitraire. Sans réseau et sans copie locale : `Err(network)`.
    pub async fn thumbnail(
        &self,
        cache_root: &Path,
        source_id: &str,
        catalog_url: &str,
        thumb_url: &str,
    ) -> AppResult<String> {
        let dir = source_dir(cache_root, source_id)?;
        if !self
            .known_thumbs(&dir, source_id, catalog_url)
            .await
            .contains(thumb_url)
        {
            return Err(AppError::NotFound(
                "vignette inconnue pour ce catalogue".into(),
            ));
        }

        let path = thumb_path(&dir, thumb_url);
        let meta_path = thumb_meta_path(&dir, thumb_url);
        let ttl = self.thumb_ttl.as_secs();
        let load = || async {
            let image = read_thumb(&path).await;
            let meta = read_thumb_meta(&meta_path).await;
            let fresh = image.is_some()
                && meta.as_ref().is_some_and(|m| {
                    let now = now_secs();
                    m.fetched_at <= now && now - m.fetched_at < ttl
                });
            (image, meta, fresh)
        };
        let (image, _, fresh) = load().await;
        if let (Some(url), true) = (&image, fresh) {
            return Ok(url.clone());
        }

        // Copie périmée et revalidation récemment en échec : on la sert sans retenter.
        if let Some(url) = &image {
            let failed = lock(&self.thumb_failures).get(&path).copied();
            if failed.is_some_and(|at| now_secs().saturating_sub(at) < THUMB_RETRY_SECS) {
                return Ok(url.clone());
            }
        }

        let _slot = self
            .thumb_slots
            .acquire()
            .await
            .map_err(|_| AppError::Network("téléchargements interrompus".into()))?;
        // Une autre demande a pu la télécharger pendant l'attente.
        let (image, meta, fresh) = load().await;
        if let (Some(url), true) = (&image, fresh) {
            return Ok(url.clone());
        }
        // Absente, périmée ou sans suivi : requête (conditionnelle si on a une copie) ; en cas
        // d'échec, la copie locale reste servie plutôt que rien.
        let stale = image;
        let mut request = self.client.get(thumb_url);
        if stale.is_some() {
            if let Some(etag) = meta.as_ref().and_then(|m| m.etag.as_ref()) {
                request = request.header(IF_NONE_MATCH, etag);
            }
            if let Some(date) = meta.as_ref().and_then(|m| m.last_modified.as_ref()) {
                request = request.header(IF_MODIFIED_SINCE, date);
            }
        }
        let fetched = async {
            let response = request.send().await.map_err(|err| network_error(&err))?;
            let status = response.status();
            if status == StatusCode::NOT_MODIFIED && stale.is_some() {
                return Ok((None, meta.clone().unwrap_or_default()));
            }
            if !status.is_success() {
                return Err(AppError::Network(format!(
                    "vignette inaccessible : réponse HTTP {status}"
                )));
            }
            let new_meta = ThumbMeta {
                etag: header_text(&response, ETAG),
                last_modified: header_text(&response, LAST_MODIFIED),
                fetched_at: 0,
            };
            let body = read_body(response, MAX_THUMB_BYTES).await?;
            let url = data_url(&body).ok_or_else(|| {
                AppError::CatalogInvalid(
                    "la vignette n'est pas une image PNG, JPEG, GIF ou WebP".into(),
                )
            })?;
            Ok((Some((body, url)), new_meta))
        }
        .await;
        match fetched {
            Ok((downloaded, mut new_meta)) => {
                new_meta.fetched_at = now_secs();
                let served = match &downloaded {
                    Some((_, url)) => url.clone(),
                    None => stale.unwrap_or_default(),
                };
                let body = downloaded.map(|(body, _)| body);
                lock(&self.thumb_failures).remove(&path);
                self.store_thumb(path, meta_path, body, new_meta).await;
                Ok(served)
            }
            Err(err) => match stale {
                Some(url) => {
                    tracing::warn!("revalidation de la vignette impossible : {err}");
                    lock(&self.thumb_failures).insert(path, now_secs());
                    Ok(url)
                }
                None => Err(err),
            },
        }
    }

    /// Écrit une vignette (si `body`) et son suivi, sous le portillon d'écriture du cache : un
    /// vidage du cache en cours attend la fin de l'écriture.
    async fn store_thumb(
        &self,
        path: PathBuf,
        meta_path: PathBuf,
        body: Option<Vec<u8>>,
        meta: ThumbMeta,
    ) {
        let _gate = self.cache_gate.read().await;
        let written = blocking(move || {
            if let Some(body) = body {
                storage::write_atomic(&path, &body)?;
            }
            storage::write_json(&meta_path, &meta)
        })
        .await;
        match written {
            Ok(Ok(())) => {}
            Ok(Err(err)) => tracing::warn!("écriture de la vignette impossible : {err}"),
            Err(err) => tracing::warn!("écriture de la vignette impossible : {err}"),
        }
    }
}

/// Suivi d'une vignette en cache (`<hash>.meta.json`) pour sa revalidation.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct ThumbMeta {
    etag: Option<String>,
    last_modified: Option<String>,
    /// Dernier téléchargement ou dernière revalidation (secondes Unix).
    fetched_at: u64,
}

fn thumb_stem(thumb_url: &str) -> String {
    hex(&Sha256::digest(thumb_url.as_bytes()))[..32].to_string()
}

/// Fichier de la vignette `thumb_url` dans le dossier de cache `dir` d'une source.
fn thumb_path(dir: &Path, thumb_url: &str) -> PathBuf {
    dir.join("thumbs")
        .join(format!("{}.img", thumb_stem(thumb_url)))
}

fn thumb_meta_path(dir: &Path, thumb_url: &str) -> PathBuf {
    dir.join("thumbs")
        .join(format!("{}.meta.json", thumb_stem(thumb_url)))
}

async fn read_thumb_meta(path: &Path) -> Option<ThumbMeta> {
    let path = path.to_path_buf();
    blocking(move || serde_json::from_slice(&std::fs::read(path).ok()?).ok())
        .await
        .ok()
        .flatten()
}

/// Supprime de `thumbs/` les vignettes (et leur suivi) qui ne correspondent à aucune
/// `thumb_url` de `known`. Seuls les fichiers `*.img` et `*.meta.json` sont concernés.
fn prune_thumbs(dir: &Path, known: &HashSet<String>) -> std::io::Result<()> {
    let thumbs = dir.join("thumbs");
    match std::fs::symlink_metadata(&thumbs) {
        Ok(meta) if meta.is_dir() => {}
        _ => return Ok(()),
    }
    let keep: HashSet<String> = known.iter().map(|u| thumb_stem(u)).collect();
    for entry in std::fs::read_dir(&thumbs)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.ends_with(".img") || name.ends_with(".meta.json")) {
            continue;
        }
        let stem = name.split('.').next().unwrap_or_default();
        if !keep.contains(stem) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Supprime le dossier de cache d'une source (`dir` = `<cache>/catalogs/<id>`) sans suivre de
/// lien. Refusé si `catalogs/` est lui-même un lien.
fn remove_source_dir(dir: &Path) -> std::io::Result<()> {
    if let Some(parent) = dir.parent() {
        if std::fs::symlink_metadata(parent).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(std::io::Error::other(
                "« catalogs » est un lien : suppression refusée",
            ));
        }
    }
    crate::player::remove_tree(dir)
}

/// Vignette `thumb_url` de la source `source_id` déjà téléchargée dans le cache, sans réseau
/// (bibliothèque, #80). `None` si l'identifiant est invalide.
pub(crate) fn cached_thumb_path(
    cache_root: &Path,
    source_id: &str,
    thumb_url: &str,
) -> Option<PathBuf> {
    Some(thumb_path(
        &source_dir(cache_root, source_id).ok()?,
        thumb_url,
    ))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn read_thumb(path: &Path) -> Option<String> {
    let path = path.to_path_buf();
    blocking(move || std::fs::read(path).ok().and_then(|b| data_url(&b)))
        .await
        .ok()
        .flatten()
}

// ---------------------------------------------------------------------------------------
// Lecture tolérante du format Telmi Store
// ---------------------------------------------------------------------------------------

/// Résultat de la lecture d'un catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedCatalog {
    pub entries: Vec<StoryEntry>,
    /// Entrées ignorées et anomalies, en français, lisibles telles quelles.
    pub warnings: Vec<String>,
}

fn text(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(s)) => s.trim().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

/// Texte optionnel : absent, de mauvais type ou vide = `None`.
fn opt_text(value: &Value, key: &str) -> Option<String> {
    Some(text(value, key)).filter(|s| !s.is_empty())
}

fn unsigned(value: &Value, key: &str) -> Option<u64> {
    match value.get(key)? {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Résout `raw` par rapport à l'URL du catalogue ; seules les URL http(s) sont conservées.
fn resolve_url(base: &Url, raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let url = base.join(raw).ok()?;
    matches!(url.scheme(), "http" | "https").then(|| url.to_string())
}

/// Extrait de texte sûr pour un message (caractères de contrôle remplacés, longueur bornée).
fn printable(raw: &str) -> String {
    let clean: String = raw
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .take(40)
        .collect();
    if raw.chars().count() > 40 {
        format!("{clean}…")
    } else {
        clean
    }
}

fn parse_entry(source_id: &str, base: &Url, item: &Value) -> Result<StoryEntry, String> {
    if !item.is_object() {
        return Err("ce n'est pas un objet".into());
    }
    let uuid = text(item, "uuid");
    if uuid.is_empty() {
        return Err("identifiant (uuid) manquant".into());
    }
    if !validate_story_id(&uuid) {
        return Err(format!(
            "identifiant (uuid) invalide : {}",
            printable(&uuid)
        ));
    }
    let download_url = resolve_url(base, &text(item, "download"))
        .ok_or_else(|| format!("{uuid} : lien de téléchargement absent ou invalide"))?;
    // La grille affiche de petites vignettes : « small » d'abord.
    let thumb_url = item.get("thumbs").and_then(|thumbs| {
        ["small", "medium"]
            .iter()
            .find_map(|size| resolve_url(base, &text(thumbs, size)))
    });
    Ok(StoryEntry {
        source_id: source_id.to_string(),
        entry_id: format!("{source_id}:{uuid}"),
        uuid,
        kind: StoryKind::TelmiZip,
        version: unsigned(item, "version").and_then(|v| u32::try_from(v).ok()),
        title: text(item, "title"),
        description: opt_text(item, "description"),
        age: unsigned(item, "age").and_then(|v| u32::try_from(v).ok()),
        category: opt_text(item, "category"),
        author: opt_text(item, "author"),
        voice: opt_text(item, "voice"),
        license: opt_text(item, "license"),
        thumb_url,
        download_url,
        size: unsigned(item, "size"),
        updated_at: opt_text(item, "updated_at"),
    })
}

/// Lit un catalogue au format Telmi Store (`{ banner, data: [...] }`).
///
/// Tolérant : champs inconnus ignorés, champs absents ou de mauvais type remplacés par une
/// valeur par défaut, entrée sans `uuid` valide (voir `validate_story_id`) ni lien de
/// téléchargement valide ignorée avec un avertissement, doublon d'`uuid` ignoré (première
/// occurrence conservée). Au-delà de 10 000 entrées, les suivantes sont ignorées (avertissement).
/// Une racine qui est directement une liste est acceptée. `Err(CatalogInvalid)` si le JSON est
/// invalide ou ne contient aucune liste d'histoires.
pub fn parse_catalog(source_id: &str, catalog_url: &str, bytes: &[u8]) -> AppResult<ParsedCatalog> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let root: Value = serde_json::from_slice(bytes)
        .map_err(|err| AppError::CatalogInvalid(format!("catalogue illisible : {err}")))?;
    let base = Url::parse(catalog_url)
        .map_err(|err| AppError::Validation(format!("URL de catalogue invalide : {err}")))?;
    let items = match &root {
        Value::Array(items) => items,
        Value::Object(map) => match map.get("data") {
            Some(Value::Array(items)) => items,
            _ => {
                return Err(AppError::CatalogInvalid(
                    "catalogue sans liste d'histoires (champ « data »)".into(),
                ))
            }
        },
        _ => {
            return Err(AppError::CatalogInvalid(
                "le catalogue n'est ni un objet ni une liste".into(),
            ))
        }
    };

    let mut entries: Vec<StoryEntry> = Vec::with_capacity(items.len().min(MAX_ENTRIES));
    let mut seen: HashSet<String> = HashSet::new();
    let mut warnings = Vec::new();
    let mut skipped = 0usize;
    let mut warn = |message: String| {
        if warnings.len() < MAX_WARNINGS {
            warnings.push(message);
        } else {
            skipped += 1;
        }
    };
    for (index, item) in items.iter().take(MAX_ENTRIES).enumerate() {
        match parse_entry(source_id, &base, item) {
            Ok(entry) if !seen.insert(entry.uuid.clone()) => {
                warn(format!(
                    "entrée {} ignorée : identifiant en double ({})",
                    index + 1,
                    entry.uuid
                ));
            }
            Ok(entry) => entries.push(entry),
            Err(reason) => warn(format!("entrée {} ignorée : {reason}", index + 1)),
        }
    }
    if items.len() > MAX_ENTRIES {
        warn(format!(
            "catalogue tronqué : {} entrée(s) au-delà de {MAX_ENTRIES} ignorée(s)",
            items.len() - MAX_ENTRIES
        ));
    }
    if skipped > 0 {
        warnings.push(format!("… et {skipped} autre(s) avertissement(s)"));
    }
    Ok(ParsedCatalog { entries, warnings })
}

// ---------------------------------------------------------------------------------------
// Cache
// ---------------------------------------------------------------------------------------

/// Métadonnées du cache d'un catalogue (`meta.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheMeta {
    /// URL dont provient le catalogue en cache.
    url: String,
    etag: Option<String>,
    last_modified: Option<String>,
    /// SHA-256 (hexadécimal) de `catalog.json` tel qu'écrit.
    #[serde(default)]
    sha256: String,
    /// Dernière vérification réussie (secondes Unix), 304 compris.
    fetched_at: u64,
}

struct Cached {
    meta: CacheMeta,
    parsed: ParsedCatalog,
    /// `catalog.json` correspond au hash de `meta.json` : ETag et date utilisables.
    trusted: bool,
}

/// Dossier de cache d'une source. L'identifiant doit être un nom de dossier sûr.
fn source_dir(cache_root: &Path, source_id: &str) -> AppResult<PathBuf> {
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
    Ok(cache_root.join("catalogs").join(source_id))
}

/// Charge le cache d'une source s'il est lisible et correspond à `url`.
fn load_cache(dir: &Path, source_id: &str, url: &str) -> Option<Cached> {
    let meta: CacheMeta = serde_json::from_slice(&std::fs::read(dir.join(META_FILE)).ok()?)
        .map_err(|err| tracing::warn!("meta de catalogue illisible ({source_id}) : {err}"))
        .ok()?;
    if meta.url != url {
        return None;
    }
    let bytes = std::fs::read(dir.join(CACHE_FILE)).ok()?;
    let trusted = hex(&Sha256::digest(&bytes)) == meta.sha256;
    if !trusted {
        tracing::warn!("catalogue en cache modifié ou incomplet ({source_id}) : ETag ignoré");
    }
    let parsed = parse_catalog(source_id, url, &bytes)
        .map_err(|err| tracing::warn!("catalogue en cache illisible ({source_id}) : {err}"))
        .ok()?;
    Some(Cached {
        meta,
        parsed,
        trusted,
    })
}

async fn load_cache_blocking(dir: &Path, source_id: &str, url: &str) -> Option<Cached> {
    let (dir, sid, url) = (dir.to_path_buf(), source_id.to_string(), url.to_string());
    blocking(move || load_cache(&dir, &sid, &url))
        .await
        .ok()
        .flatten()
}

fn store_cache(dir: &Path, bytes: &[u8], meta: &CacheMeta) -> AppResult<()> {
    storage::write_atomic(&dir.join(CACHE_FILE), bytes)?;
    storage::write_json(&dir.join(META_FILE), meta)
}

// ---------------------------------------------------------------------------------------
// Vues
// ---------------------------------------------------------------------------------------

/// Problème rencontré lors du dernier rafraîchissement (miroir de l'`AppError` correspondante).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogProblem {
    /// `network` ou `catalog_invalid`.
    pub kind: String,
    pub message: String,
}

/// État de fraîcheur des données renvoyées.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogStatus {
    /// `true` si le rafraîchissement a échoué faute de réseau/serveur et que les données
    /// viennent du cache.
    pub offline: bool,
    /// Dernière vérification réussie du catalogue (secondes Unix) ; `None` si jamais obtenu.
    pub updated_at: Option<u64>,
    /// Pourquoi le rafraîchissement a échoué (ou a été écarté, catalogue vide), quand des
    /// données du cache sont servies.
    pub error: Option<CatalogProblem>,
}

/// Entrées d'un catalogue et état (miroir de `CatalogView` dans `src/api/catalog.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CatalogView {
    pub source_id: String,
    pub entries: Vec<StoryEntry>,
    /// Entrées ignorées à la lecture du catalogue.
    pub warnings: Vec<String>,
    pub status: CatalogStatus,
}

fn view_from(
    source_id: &str,
    cached: Option<Cached>,
    offline: bool,
    error: Option<&AppError>,
) -> CatalogView {
    let (entries, warnings, updated_at) = match cached {
        Some(c) => (c.parsed.entries, c.parsed.warnings, Some(c.meta.fetched_at)),
        None => (Vec::new(), Vec::new(), None),
    };
    CatalogView {
        source_id: source_id.to_string(),
        entries,
        warnings,
        status: CatalogStatus {
            offline,
            updated_at,
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
    AppError::Network(format!("catalogue inaccessible : {reason}"))
}

/// Lit le corps de la réponse en refusant de dépasser `max` octets, que la longueur soit
/// annoncée (`Content-Length`) ou non (flux en morceaux).
async fn read_body(mut response: reqwest::Response, max: usize) -> AppResult<Vec<u8>> {
    if response
        .content_length()
        .is_some_and(|len| len > max as u64)
    {
        return Err(AppError::Network("réponse trop volumineuse".into()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| network_error(&e))? {
        if body.len() + chunk.len() > max {
            return Err(AppError::Network("réponse trop volumineuse".into()));
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

// ---------------------------------------------------------------------------------------
// Vignettes : reconnaissance des images
// ---------------------------------------------------------------------------------------

/// Type MIME d'une image reconnue à ses premiers octets (PNG, JPEG, GIF, WebP). Le SVG est
/// volontairement refusé (contenu actif).
pub(crate) fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xFF\xD8\xFF") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

fn data_url(bytes: &[u8]) -> Option<String> {
    let mime = sniff_image(bytes)?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

// ---------------------------------------------------------------------------------------
// Commandes Tauri
// ---------------------------------------------------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// URL de la source catalogue `source_id` et dossier de cache effectif.
fn resolve_catalog(
    config_dir: &Path,
    env: &Environment,
    source_id: &str,
) -> AppResult<(String, PathBuf)> {
    let url = match SourcesStore::new(config_dir).get(source_id)? {
        Source::Catalog { url, .. } => url,
        _ => {
            return Err(AppError::Validation(format!(
                "la source {source_id} n'est pas un catalogue"
            )))
        }
    };
    let cache_root = SettingsStore::new(config_dir).resolved_cache_dir(env)?;
    Ok((url, cache_root))
}

fn resolve_from_state(state: &ConfigState, source_id: &str) -> AppResult<(String, PathBuf)> {
    let _guard = state.guard();
    resolve_catalog(&state.dir, &Environment::current(), source_id)
}

/// Purge le cache d'une source catalogue supprimée ou dont l'URL a changé. La source est déjà
/// modifiée : un échec est journalisé (le cache orphelin reste, il n'est jamais relu).
pub(crate) async fn purge_catalog_cache(
    config: &ConfigState,
    service: &CatalogService,
    source_id: &str,
) {
    let root = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())
    };
    let purged = match root {
        Ok(root) => service.purge_source(&root, source_id).await,
        Err(err) => Err(err),
    };
    if let Err(err) = purged {
        tracing::warn!("purge du cache du catalogue {source_id} impossible : {err}");
    }
}

/// Télécharge le catalogue de la source (cache et requête conditionnelle comprises). Sans
/// réseau, renvoie le cache avec `status.offline = true`.
#[tauri::command]
pub async fn catalog_refresh(
    config: State<'_, ConfigState>,
    service: State<'_, CatalogService>,
    source_id: String,
) -> AppResult<CatalogView> {
    let (url, cache_root) = resolve_from_state(&config, &source_id)?;
    service
        .refresh(&cache_root, &source_id, &url, now_secs())
        .await
}

/// Entrées du dernier catalogue en cache, sans accès réseau.
#[tauri::command]
pub async fn catalog_entries(
    config: State<'_, ConfigState>,
    service: State<'_, CatalogService>,
    source_id: String,
) -> AppResult<CatalogView> {
    let (url, cache_root) = resolve_from_state(&config, &source_id)?;
    service.entries(&cache_root, &source_id, &url).await
}

/// Vignette d'une histoire du catalogue (URL `data:`), téléchargée paresseusement.
#[tauri::command]
pub async fn catalog_thumbnail(
    config: State<'_, ConfigState>,
    service: State<'_, CatalogService>,
    source_id: String,
    thumb_url: String,
) -> AppResult<String> {
    let (url, cache_root) = resolve_from_state(&config, &source_id)?;
    service
        .thumbnail(&cache_root, &source_id, &url, &thumb_url)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n0000";

    fn sample() -> Value {
        json!({
            "banner": { "title": "Bandeau inventé", "unknown": [1, 2] },
            "data": [
                {
                    "uuid": "aaaa-1",
                    "version": 3,
                    "title": "Le Pré enchanté",
                    "description": "Une histoire inventée, avec des accents et un Œil.",
                    "age": 5,
                    "category": "Conte",
                    "author": "Autrice Fictive",
                    "voice": "Voix Fictive",
                    "designer": "Dessinateur",
                    "publisher": "Éditeur",
                    "license": "CC BY",
                    "thumbs": { "small": "/img/a-small.png", "medium": "/img/a-medium.png" },
                    "download": "/zip/a.zip",
                    "download_count": 12,
                    "awards": [],
                    "created_at": "2025-01-01T10:00:00Z",
                    "updated_at": "2025-02-02T10:00:00Z"
                },
                {
                    "uuid": "bbbb-2",
                    "title": "Sans version",
                    "thumbs": { "small": "https://img.example.test/b.png" },
                    "download": "https://dl.example.test/b.zip",
                    "extra_inconnu": true
                }
            ]
        })
    }

    fn parse(value: &Value) -> AppResult<ParsedCatalog> {
        parse_catalog(
            "src",
            "https://cat.example.test/store/list.json",
            value.to_string().as_bytes(),
        )
    }

    #[test]
    fn lecture_nominale_normalise_les_entrees() {
        let parsed = parse(&sample()).unwrap();
        assert!(parsed.warnings.is_empty());
        assert_eq!(parsed.entries.len(), 2);
        let a = &parsed.entries[0];
        assert_eq!(a.source_id, "src");
        assert_eq!(a.kind, StoryKind::TelmiZip);
        assert_eq!(a.version, Some(3));
        assert_eq!(a.title, "Le Pré enchanté");
        assert!(a.description.as_deref().unwrap().contains("Œil"));
        assert_eq!(a.age, Some(5));
        assert_eq!(a.license.as_deref(), Some("CC BY"));
        assert_eq!(a.size, None);
        assert_eq!(a.updated_at.as_deref(), Some("2025-02-02T10:00:00Z"));
        // URL relatives résolues par rapport au catalogue ; vignette « small » préférée.
        assert_eq!(a.download_url, "https://cat.example.test/zip/a.zip");
        assert_eq!(
            a.thumb_url.as_deref(),
            Some("https://cat.example.test/img/a-small.png")
        );
        let b = &parsed.entries[1];
        assert_eq!(b.version, None);
        assert_eq!(b.description, None);
        assert_eq!(
            b.thumb_url.as_deref(),
            Some("https://img.example.test/b.png")
        );
    }

    #[test]
    fn champs_de_mauvais_type_prennent_des_valeurs_par_defaut() {
        let doc = json!({ "data": [{
            "uuid": 42, "title": ["x"], "age": "7", "version": "oops", "size": "1024",
            "thumbs": "pas un objet", "download": "https://d.example.test/x.zip"
        }]});
        let parsed = parse(&doc).unwrap();
        let e = &parsed.entries[0];
        assert_eq!(e.uuid, "42");
        assert_eq!(e.title, "");
        assert_eq!(e.age, Some(7));
        assert_eq!(e.version, None);
        assert_eq!(e.size, Some(1024));
        assert_eq!(e.thumb_url, None);
    }

    #[test]
    fn entrees_invalides_ignorees_avec_avertissement() {
        let doc = json!({ "data": [
            "texte",
            { "title": "sans uuid", "download": "https://d.example.test/1.zip" },
            { "uuid": "u2", "title": "sans lien" },
            { "uuid": "u3", "download": "ftp://d.example.test/3.zip" },
            { "uuid": "u4", "download": "https://d.example.test/4.zip" },
            { "uuid": "u4", "download": "https://d.example.test/4bis.zip" }
        ]});
        let parsed = parse(&doc).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(
            parsed.entries[0].download_url,
            "https://d.example.test/4.zip"
        );
        assert_eq!(parsed.warnings.len(), 5);
        assert!(parsed.warnings[0].contains("entrée 1"));
        assert!(parsed.warnings[4].contains("double"));
    }

    #[test]
    fn racine_liste_et_bom_acceptes() {
        let mut bytes = b"\xEF\xBB\xBF".to_vec();
        bytes.extend(
            json!([{ "uuid": "x", "download": "/x.zip" }])
                .to_string()
                .as_bytes(),
        );
        let parsed = parse_catalog("s", "https://h.example.test/c.json", &bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
    }

    #[test]
    fn structure_inexploitable_est_refusee() {
        for bytes in [
            &b"pas du json"[..],
            b"{\"banner\": {}}",
            b"42",
            b"{\"data\": {}}",
        ] {
            let err = parse_catalog("s", "https://h.example.test/c.json", bytes).unwrap_err();
            assert_eq!(
                err.kind(),
                "catalog_invalid",
                "{:?}",
                String::from_utf8_lossy(bytes)
            );
        }
    }

    #[test]
    fn avertissements_plafonnes() {
        let items: Vec<Value> = (0..80).map(|_| json!({})).collect();
        let parsed = parse(&json!({ "data": items })).unwrap();
        assert_eq!(parsed.warnings.len(), MAX_WARNINGS + 1);
        assert!(parsed.warnings.last().unwrap().contains("30 autre"));
    }

    #[test]
    fn identifiant_de_source_dangereux_refuse() {
        let root = Path::new("cache");
        for id in ["", "..", "a/b", "a\\b", "x:y"] {
            assert_eq!(
                source_dir(root, id).unwrap_err().kind(),
                "validation",
                "{id}"
            );
        }
        assert!(source_dir(root, "telmi-store").is_ok());
    }

    #[test]
    fn detection_des_images() {
        assert_eq!(sniff_image(PNG), Some("image/png"));
        assert_eq!(sniff_image(b"\xFF\xD8\xFF\xE0xx"), Some("image/jpeg"));
        assert_eq!(sniff_image(b"GIF89a.."), Some("image/gif"));
        assert_eq!(sniff_image(b"RIFF1234WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"<svg xmlns='x'/>"), None);
    }

    // --- Réseau : serveur HTTP local uniquement -----------------------------------------

    fn client() -> reqwest::Client {
        build_client(Duration::from_secs(2), Duration::from_secs(5)).unwrap()
    }

    fn svc() -> CatalogService {
        CatalogService::with_client(client())
    }

    fn svc_with(client: reqwest::Client) -> CatalogService {
        CatalogService::with_client(client)
    }

    fn catalog_response(body: &Value) -> ResponseTemplate {
        ResponseTemplate::new(200)
            .insert_header("ETag", "\"v1\"")
            .insert_header("Last-Modified", "Wed, 01 Jan 2025 10:00:00 GMT")
            .set_body_raw(body.to_string(), "application/json; charset=utf-8")
    }

    async fn serve(server: &MockServer, response: ResponseTemplate, times: Option<u64>) {
        let mock = Mock::given(method("GET"))
            .and(path("/catalog.json"))
            .respond_with(response);
        match times {
            Some(n) => mock.up_to_n_times(n).mount(server).await,
            None => mock.mount(server).await,
        }
    }

    fn url_of(server: &MockServer) -> String {
        format!("{}/catalog.json", server.uri())
    }

    #[tokio::test]
    async fn refresh_nominal_ecrit_le_cache() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), None).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);

        let view = svc()
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 2);
        assert_eq!(
            view.entries[0].download_url,
            format!("{}/zip/a.zip", server.uri())
        );
        assert!(!view.status.offline);
        assert_eq!(view.status.updated_at, Some(1000));
        assert!(view.status.error.is_none());

        let dir = cache.path().join("catalogs").join("src");
        assert!(dir.join("catalog.json").is_file());
        let meta = std::fs::read_to_string(dir.join("meta.json")).unwrap();
        assert!(meta.contains(r#"\"v1\""#));

        let cached = svc().entries(cache.path(), "src", &url).await.unwrap();
        assert_eq!(cached.entries, view.entries);
        assert_eq!(cached.status.updated_at, Some(1000));
    }

    #[tokio::test]
    async fn refresh_envoie_le_user_agent() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(header(
                "user-agent",
                concat!(
                    "flam-launcher/",
                    env!("CARGO_PKG_VERSION"),
                    " (+https://github.com/scarlaty/flam-player)"
                ),
            ))
            .respond_with(catalog_response(&sample()))
            .expect(1)
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        svc()
            .refresh(cache.path(), "src", &url_of(&server), 1)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn refresh_json_partiellement_invalide_garde_les_bonnes_entrees() {
        let doc = json!({ "data": [
            { "uuid": "ok", "title": "Valide", "download": "/ok.zip" },
            { "title": "cassée" },
            12
        ]});
        let server = MockServer::start().await;
        serve(&server, catalog_response(&doc), None).await;
        let cache = tempfile::tempdir().unwrap();
        let view = svc()
            .refresh(cache.path(), "src", &url_of(&server), 5)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 1);
        assert_eq!(view.warnings.len(), 2);
        assert!(!view.status.offline);
    }

    #[tokio::test]
    async fn refresh_304_conserve_le_cache_et_met_a_jour_la_date() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        svc()
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        Mock::given(method("GET"))
            .and(path("/catalog.json"))
            .and(header_exists("if-none-match"))
            .and(header_exists("if-modified-since"))
            .respond_with(ResponseTemplate::new(304))
            .expect(1)
            .mount(&server)
            .await;
        let view = svc()
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 2);
        assert!(!view.status.offline);
        assert_eq!(view.status.updated_at, Some(2000));
        let requests = server.received_requests().await.unwrap();
        let conditional = requests.last().unwrap();
        assert_eq!(conditional.headers["if-none-match"], "\"v1\"");
        assert_eq!(
            conditional.headers["if-modified-since"],
            "Wed, 01 Jan 2025 10:00:00 GMT"
        );
        let again = svc().entries(cache.path(), "src", &url).await.unwrap();
        assert_eq!(again.status.updated_at, Some(2000));
    }

    #[tokio::test]
    async fn refresh_404_sert_le_cache_hors_ligne() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        svc()
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(&server, ResponseTemplate::new(404), None).await;
        let view = svc()
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 2);
        assert!(view.status.offline);
        assert_eq!(view.status.updated_at, Some(1000));
        assert_eq!(view.status.error.as_ref().unwrap().kind, "network");
    }

    #[tokio::test]
    async fn refresh_404_sans_cache_est_une_erreur_network() {
        let server = MockServer::start().await;
        serve(&server, ResponseTemplate::new(404), None).await;
        let cache = tempfile::tempdir().unwrap();
        let err = svc()
            .refresh(cache.path(), "src", &url_of(&server), 1)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "network");
    }

    #[tokio::test]
    async fn refresh_delai_depasse_sert_le_cache() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        svc()
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(
            &server,
            ResponseTemplate::new(200).set_delay(Duration::from_secs(3)),
            None,
        )
        .await;
        let fast = build_client(Duration::from_millis(500), Duration::from_millis(300)).unwrap();
        let view = svc_with(fast.clone())
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert!(view.status.offline);
        assert_eq!(view.entries.len(), 2);

        let other = tempfile::tempdir().unwrap();
        let err = svc_with(fast.clone())
            .refresh(other.path(), "src", &url, 2000)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "network");
    }

    #[tokio::test]
    async fn refresh_serveur_injoignable_sert_le_cache() {
        let server = MockServer::builder().start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        svc()
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();
        drop(server);
        let view = svc()
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert!(view.status.offline);
        assert_eq!(view.entries.len(), 2);
    }

    #[tokio::test]
    async fn refresh_catalogue_invalide_garde_l_ancien_cache() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        svc()
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(
            &server,
            ResponseTemplate::new(200).set_body_string("<html>erreur</html>"),
            None,
        )
        .await;
        let view = svc()
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 2);
        assert!(!view.status.offline);
        assert_eq!(view.status.error.as_ref().unwrap().kind, "catalog_invalid");

        // Sans cache : l'erreur est remontée.
        let other = tempfile::tempdir().unwrap();
        let err = svc()
            .refresh(other.path(), "src", &url, 2000)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "catalog_invalid");
        assert!(!other
            .path()
            .join("catalogs")
            .join("src")
            .join("catalog.json")
            .exists());
    }

    #[tokio::test]
    async fn cache_d_une_autre_url_est_ignore() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), None).await;
        let cache = tempfile::tempdir().unwrap();
        svc()
            .refresh(cache.path(), "src", &url_of(&server), 1)
            .await
            .unwrap();
        let view = svc()
            .entries(cache.path(), "src", "http://autre.example.test/c.json")
            .await
            .unwrap();
        assert!(view.entries.is_empty());
        assert_eq!(view.status.updated_at, None);
    }

    #[tokio::test]
    async fn vignette_telechargee_une_seule_fois() {
        let server = MockServer::start().await;
        let doc = json!({ "data": [{
            "uuid": "u", "download": "/u.zip", "thumbs": { "medium": "/t/u.png" }
        }]});
        serve(&server, catalog_response(&doc), None).await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(PNG, "image/png"))
            .expect(1)
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let view = svc().refresh(cache.path(), "src", &url, 1).await.unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();

        let first = svc()
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert!(first.starts_with("data:image/png;base64,"));
        // Deuxième appel : servie depuis le disque (le mock n'attend qu'une requête).
        let second = svc()
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert_eq!(first, second);
        // Une image et son suivi de revalidation.
        assert_eq!(
            std::fs::read_dir(cache.path().join("catalogs/src/thumbs"))
                .unwrap()
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn vignette_refus_url_inconnue_non_image_et_hors_ligne() {
        let server = MockServer::builder().start().await;
        let doc = json!({ "data": [{
            "uuid": "u", "download": "/u.zip", "thumbs": { "medium": "/t/u.png" }
        }]});
        serve(&server, catalog_response(&doc), None).await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<svg onload=alert(1)/>"))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let view = svc().refresh(cache.path(), "src", &url, 1).await.unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();

        let err = svc()
            .thumbnail(cache.path(), "src", &url, "http://evil.example.test/x.png")
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "not_found");

        let err = svc()
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "catalog_invalid");
        assert!(!cache.path().join("catalogs/src/thumbs").exists());

        drop(server);
        let err = svc()
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "network");
    }

    #[test]
    fn resolve_catalog_refuse_les_sources_non_catalogue() {
        let dir = tempfile::tempdir().unwrap();
        let env = Environment {
            exe_dir: None,
            path_var: None,
            default_cache_dir: dir.path().join("cache"),
            search_repo_build: false,
            protected_dirs: vec![],
        };
        let (url, cache) = resolve_catalog(dir.path(), &env, "telmi-store").unwrap();
        assert!(url.starts_with("https://"));
        assert!(cache.is_dir());
        assert_eq!(
            resolve_catalog(dir.path(), &env, "library")
                .unwrap_err()
                .kind(),
            "validation"
        );
        assert_eq!(
            resolve_catalog(dir.path(), &env, "nope")
                .unwrap_err()
                .kind(),
            "not_found"
        );
    }

    // --- Revue : identifiants, plafonds, redirections, cohérence du cache -------------------

    #[test]
    fn entry_id_combine_source_et_uuid() {
        let parsed = parse(&sample()).unwrap();
        assert_eq!(parsed.entries[0].entry_id, "src:aaaa-1");
        assert_eq!(parsed.entries[1].entry_id, "src:bbbb-2");
    }

    #[test]
    fn uuid_malveillants_ignores_avec_avertissement() {
        let bad = [
            "../evil",
            "a/b",
            "a\\b",
            "CON",
            "nul.txt",
            "x\u{0}y",
            ".cache",
            "a..b",
            "trop-long-0123456789012345678901234567890123456789012345678901234567890",
        ];
        let mut items: Vec<Value> = bad
            .iter()
            .map(|id| json!({ "uuid": id, "download": "https://d.example.test/x.zip" }))
            .collect();
        items.push(json!({ "uuid": "bon-1", "download": "https://d.example.test/x.zip" }));
        let parsed = parse(&json!({ "data": items })).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].uuid, "bon-1");
        assert_eq!(parsed.warnings.len(), bad.len());
        assert!(parsed.warnings.iter().all(|w| w.contains("invalide")));
        // Le caractère de contrôle n'est pas recopié dans le message.
        assert!(parsed.warnings.iter().all(|w| !w.contains('\u{0}')));
    }

    #[test]
    fn nombre_d_entrees_plafonne() {
        let items: Vec<Value> = (0..MAX_ENTRIES + 5)
            .map(|i| json!({ "uuid": format!("u{i}"), "download": "https://d.example.test/x.zip" }))
            .collect();
        let parsed = parse(&json!({ "data": items })).unwrap();
        assert_eq!(parsed.entries.len(), MAX_ENTRIES);
        assert_eq!(parsed.warnings.len(), 1);
        assert!(parsed.warnings[0].contains("tronqué"));
        assert!(parsed.warnings[0].contains("5 entrée"));
    }

    #[test]
    fn politique_de_redirection() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert!(redirect_check(&[], &u("https://a.example.test/")).is_ok());
        assert!(redirect_check(
            &[u("http://a.example.test/")],
            &u("https://b.example.test/")
        )
        .is_ok());
        assert!(
            redirect_check(&[u("http://a.example.test/")], &u("http://b.example.test/")).is_ok()
        );
        assert!(redirect_check(
            &[u("https://a.example.test/")],
            &u("https://b.example.test/")
        )
        .is_ok());
        assert!(redirect_check(
            &[u("https://a.example.test/")],
            &u("http://b.example.test/")
        )
        .is_err());
        let chain: Vec<Url> = (0..=MAX_REDIRECTS)
            .map(|i| u(&format!("http://h.example.test/{i}")))
            .collect();
        assert!(redirect_check(&chain[..MAX_REDIRECTS], &u("http://h.example.test/x")).is_ok());
        assert!(redirect_check(&chain, &u("http://h.example.test/x")).is_err());
    }

    #[tokio::test]
    async fn redirection_suivie_puis_catalogue_lu() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/ancien.json"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/catalog.json"))
            .mount(&server)
            .await;
        serve(&server, catalog_response(&sample()), None).await;
        let cache = tempfile::tempdir().unwrap();
        let url = format!("{}/ancien.json", server.uri());
        let view = svc().refresh(cache.path(), "src", &url, 1).await.unwrap();
        assert_eq!(view.entries.len(), 2);
    }

    #[tokio::test]
    async fn trop_de_redirections_est_une_erreur_network() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/boucle"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/boucle"))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = format!("{}/boucle", server.uri());
        let err = svc()
            .refresh(cache.path(), "src", &url, 1)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "network");
        assert!(err.to_string().contains("redirection"), "{err}");
        // 1 requête initiale + 5 sauts suivis, la sixième redirection est refusée.
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            MAX_REDIRECTS + 1
        );
    }

    /// Serveur TCP minimal : répond avec les morceaux donnés (pause entre chaque) puis ferme.
    async fn raw_server(parts: Vec<Vec<u8>>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 2048];
                let _ = stream.read(&mut buf).await;
                for part in parts {
                    if stream.write_all(&part).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
                let _ = stream.shutdown().await;
            }
        });
        format!("http://{addr}/catalog.json")
    }

    const HUGE_LENGTH: &[u8] =
        b"HTTP/1.1 200 OK\r\nContent-Length: 99999999999\r\nConnection: close\r\n\r\n";

    #[tokio::test]
    async fn content_length_enorme_refuse_sans_lire_le_corps() {
        let url = raw_server(vec![HUGE_LENGTH.to_vec()]).await;
        let response = client().get(&url).send().await.unwrap();
        let err = read_body(response, 1000).await.unwrap_err();
        assert_eq!(err.kind(), "network");
        assert!(err.to_string().contains("volumineuse"));

        // De bout en bout : sans cache, erreur ; le cache n'est pas créé.
        let url = raw_server(vec![HUGE_LENGTH.to_vec()]).await;
        let cache = tempfile::tempdir().unwrap();
        let err = svc()
            .refresh(cache.path(), "src", &url, 1)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("volumineuse"), "{err}");
        assert!(!cache.path().join("catalogs/src/catalog.json").exists());
    }

    #[tokio::test]
    async fn flux_en_morceaux_sans_longueur_plafonne() {
        let mut parts = vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec(),
        ];
        for _ in 0..10 {
            let mut chunk = format!("{:x}\r\n", 1000).into_bytes();
            chunk.extend(vec![b'x'; 1000]);
            chunk.extend(b"\r\n");
            parts.push(chunk);
        }
        parts.push(b"0\r\n\r\n".to_vec());
        let url = raw_server(parts).await;
        let response = client().get(&url).send().await.unwrap();
        assert_eq!(response.content_length(), None);
        let err = read_body(response, 3500).await.unwrap_err();
        assert_eq!(err.kind(), "network");
        assert!(err.to_string().contains("volumineuse"));
    }

    #[tokio::test]
    async fn catalogue_valide_mais_vide_ne_remplace_pas_un_cache_non_vide() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = svc();
        service
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(&server, catalog_response(&json!({ "data": [] })), None).await;
        let view = service
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.status.updated_at, Some(1000));
        assert!(!view.status.offline);
        let error = view.status.error.unwrap();
        assert_eq!(error.kind, "catalog_invalid");
        assert!(error.message.contains("cache précédent"));
        let on_disk = service.entries(cache.path(), "src", &url).await.unwrap();
        assert_eq!(on_disk.entries.len(), 2);

        // Sans cache, un catalogue vide est accepté tel quel.
        let other = tempfile::tempdir().unwrap();
        let view = svc().refresh(other.path(), "src", &url, 3).await.unwrap();
        assert!(view.entries.is_empty());
        assert!(view.status.error.is_none());
    }

    #[tokio::test]
    async fn hash_incoherent_ignore_l_etag() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), None).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = svc();
        service
            .refresh(cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        // Cache intact : requête conditionnelle.
        service
            .refresh(cache.path(), "src", &url, 1500)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        assert!(requests[1].headers.contains_key("if-none-match"));

        // catalog.json modifié après coup : refresh complet, sans ETag.
        let file = cache.path().join("catalogs/src/catalog.json");
        let mut text = std::fs::read_to_string(&file).unwrap();
        text.push(' ');
        std::fs::write(&file, text).unwrap();
        let view = service
            .refresh(cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        let requests = server.received_requests().await.unwrap();
        let last = requests.last().unwrap();
        assert!(!last.headers.contains_key("if-none-match"));
        assert!(!last.headers.contains_key("if-modified-since"));
        assert_eq!(view.entries.len(), 2);
        assert_eq!(view.status.updated_at, Some(2000));
    }

    #[tokio::test]
    async fn refresh_concurrents_sont_serialises() {
        let server = MockServer::start().await;
        // Première réponse lente et sans condition ; ensuite seule une requête conditionnelle
        // (donc faite après l'écriture du cache) obtient un 304.
        serve(
            &server,
            catalog_response(&sample()).set_delay(Duration::from_millis(300)),
            Some(1),
        )
        .await;
        Mock::given(method("GET"))
            .and(path("/catalog.json"))
            .and(header_exists("if-none-match"))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = svc();
        let (a, b) = tokio::join!(
            service.refresh(cache.path(), "src", &url, 1000),
            service.refresh(cache.path(), "src", &url, 2000),
        );
        assert_eq!(a.unwrap().entries.len(), 2);
        assert_eq!(b.unwrap().entries.len(), 2);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].headers.contains_key("if-none-match"));
    }

    async fn thumb_catalog(server: &MockServer, names: &[&str], delay: Duration) -> Value {
        let items: Vec<Value> = names
            .iter()
            .map(|n| {
                json!({
                    "uuid": format!("id-{n}"),
                    "download": "/u.zip",
                    "thumbs": { "small": format!("/t/{n}.png"), "medium": "/m.png" }
                })
            })
            .collect();
        for n in names {
            Mock::given(method("GET"))
                .and(path(format!("/t/{n}.png")))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_delay(delay)
                        .set_body_raw(PNG, "image/png"),
                )
                .mount(server)
                .await;
        }
        json!({ "data": items })
    }

    #[tokio::test]
    async fn vignettes_connues_gardees_en_memoire_et_invalidees_au_refresh() {
        let server = MockServer::start().await;
        let doc = thumb_catalog(&server, &["a", "b"], Duration::ZERO).await;
        serve(&server, catalog_response(&doc), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = svc();
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        let thumb_a = view.entries[0].thumb_url.clone().unwrap();
        let thumb_b = view.entries[1].thumb_url.clone().unwrap();

        // Le catalogue n'est plus relu : le disque peut être vidé.
        std::fs::remove_file(cache.path().join("catalogs/src/catalog.json")).unwrap();
        std::fs::remove_file(cache.path().join("catalogs/src/meta.json")).unwrap();
        assert!(service
            .thumbnail(cache.path(), "src", &url, &thumb_a)
            .await
            .is_ok());

        // Nouveau catalogue sans « b » : l'ensemble est remplacé.
        let doc2 = json!({ "data": [doc["data"][0].clone()] });
        serve(&server, catalog_response(&doc2), None).await;
        service.refresh(cache.path(), "src", &url, 2).await.unwrap();
        assert!(service
            .thumbnail(cache.path(), "src", &url, &thumb_a)
            .await
            .is_ok());
        let err = service
            .thumbnail(cache.path(), "src", &url, &thumb_b)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "not_found");
    }

    #[tokio::test]
    async fn telechargements_de_vignettes_limites_par_semaphore() {
        let server = MockServer::start().await;
        let delay = Duration::from_millis(200);
        let doc = thumb_catalog(&server, &["a", "b", "c"], delay).await;
        serve(&server, catalog_response(&doc), None).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = CatalogService::with_limits(client(), 1);
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        let urls: Vec<String> = view
            .entries
            .iter()
            .map(|e| e.thumb_url.clone().unwrap())
            .collect();

        let start = std::time::Instant::now();
        let (a, b, c) = tokio::join!(
            service.thumbnail(cache.path(), "src", &url, &urls[0]),
            service.thumbnail(cache.path(), "src", &url, &urls[1]),
            service.thumbnail(cache.path(), "src", &url, &urls[2]),
        );
        assert!(a.is_ok() && b.is_ok() && c.is_ok());
        // Un seul créneau : les trois délais de 200 ms s'additionnent.
        assert!(
            start.elapsed() >= Duration::from_millis(580),
            "{:?}",
            start.elapsed()
        );
    }

    // --- Nettoyage du cache (#38) -------------------------------------------------------

    #[tokio::test]
    async fn purge_ne_supprime_que_le_dossier_de_la_source() {
        let server = MockServer::start().await;
        let doc = thumb_catalog(&server, &["a"], Duration::ZERO).await;
        serve(&server, catalog_response(&doc), None).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = svc();
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        service
            .refresh(cache.path(), "autre", &url, 1)
            .await
            .unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();
        service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        std::fs::create_dir_all(cache.path().join("documents")).unwrap();
        std::fs::write(cache.path().join("documents/important.txt"), "x").unwrap();
        std::fs::write(cache.path().join("catalogs/notes.txt"), "x").unwrap();

        service.purge_source(cache.path(), "src").await.unwrap();

        assert!(!cache.path().join("catalogs/src").exists());
        assert!(cache.path().join("catalogs/autre/catalog.json").is_file());
        assert!(cache.path().join("catalogs/notes.txt").is_file());
        assert!(cache.path().join("documents/important.txt").is_file());
        // État en mémoire purgé : plus de cache, donc plus de vignette connue.
        let entries = service.entries(cache.path(), "src", &url).await.unwrap();
        assert!(entries.entries.is_empty());
        let err = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "not_found");
        // Source déjà absente : sans effet.
        service.purge_source(cache.path(), "src").await.unwrap();
    }

    #[tokio::test]
    async fn purge_refuse_un_identifiant_non_sur_et_ne_supprime_rien() {
        let cache = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(cache.path().join("catalogs/src")).unwrap();
        for bad in ["", "..", ".", "a/b", "a\\b", "../catalogs", "src/.."] {
            let err = svc().purge_source(cache.path(), bad).await.unwrap_err();
            assert_eq!(err.kind(), "validation", "{bad}");
        }
        assert!(cache.path().join("catalogs/src").is_dir());
    }

    #[tokio::test]
    async fn une_vignette_obsolete_est_supprimee_au_rafraichissement() {
        let server = MockServer::start().await;
        let doc = thumb_catalog(&server, &["a", "b"], Duration::ZERO).await;
        serve(&server, catalog_response(&doc), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let service = svc();
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        for entry in &view.entries {
            service
                .thumbnail(cache.path(), "src", &url, entry.thumb_url.as_ref().unwrap())
                .await
                .unwrap();
        }
        let thumbs = cache.path().join("catalogs/src/thumbs");
        std::fs::write(thumbs.join("notes.txt"), "x").unwrap();
        let count = |ext: &str| {
            std::fs::read_dir(&thumbs)
                .unwrap()
                .filter(|e| {
                    e.as_ref()
                        .unwrap()
                        .file_name()
                        .to_string_lossy()
                        .ends_with(ext)
                })
                .count()
        };
        assert_eq!((count(".img"), count(".meta.json")), (2, 2));

        let doc2 = json!({ "data": [doc["data"][0].clone()] });
        serve(&server, catalog_response(&doc2), None).await;
        service.refresh(cache.path(), "src", &url, 2).await.unwrap();

        assert_eq!((count(".img"), count(".meta.json")), (1, 1));
        let kept = view.entries[0].thumb_url.as_ref().unwrap();
        assert!(thumb_path(&cache.path().join("catalogs/src"), kept).is_file());
        assert!(thumbs.join("notes.txt").is_file());
    }

    #[tokio::test]
    async fn une_vignette_perimee_est_revalidee_par_etag() {
        let server = MockServer::start().await;
        let doc = json!({ "data": [{
            "uuid": "u", "download": "/u.zip", "thumbs": { "small": "/t/u.png" }
        }]});
        serve(&server, catalog_response(&doc), None).await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .and(header("if-none-match", "\"t1\""))
            .respond_with(ResponseTemplate::new(304))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("ETag", "\"t1\"")
                    .set_body_raw(PNG, "image/png"),
            )
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let mut service = svc();
        service.thumb_ttl = Duration::ZERO;
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();

        let first = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        let second = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert_eq!(first, second);
        let requests = server.received_requests().await.unwrap();
        let thumb_requests: Vec<_> = requests
            .iter()
            .filter(|r| r.url.path() == "/t/u.png")
            .collect();
        assert_eq!(thumb_requests.len(), 2);
        assert!(!thumb_requests[0].headers.contains_key("if-none-match"));
        assert!(thumb_requests[1].headers.contains_key("if-none-match"));
    }

    #[tokio::test]
    async fn une_vignette_modifiee_est_remplacee_et_la_copie_perimee_sert_hors_ligne() {
        let server = MockServer::start().await;
        let doc = json!({ "data": [{
            "uuid": "u", "download": "/u.zip", "thumbs": { "small": "/t/u.png" }
        }]});
        serve(&server, catalog_response(&doc), None).await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(PNG, "image/png"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let mut service = svc();
        service.thumb_ttl = Duration::ZERO;
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();
        let first = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();

        let new_png: &[u8] = b"\x89PNG\r\n\x1a\nNEW!";
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(new_png, "image/png"))
            .mount(&server)
            .await;
        let second = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert_ne!(first, second);

        // Serveur injoignable : la copie locale (périmée) reste servie.
        drop(server);
        let third = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert_eq!(second, third);
    }

    #[tokio::test]
    async fn un_echec_de_revalidation_n_est_pas_retente_a_chaque_affichage() {
        let server = MockServer::start().await;
        let doc = json!({ "data": [{
            "uuid": "u", "download": "/u.zip", "thumbs": { "small": "/t/u.png" }
        }]});
        serve(&server, catalog_response(&doc), None).await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(PNG, "image/png"))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/t/u.png"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        let mut service = svc();
        service.thumb_ttl = Duration::ZERO;
        let view = service.refresh(cache.path(), "src", &url, 1).await.unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();
        let first = service
            .thumbnail(cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        for _ in 0..3 {
            let again = service
                .thumbnail(cache.path(), "src", &url, &thumb)
                .await
                .unwrap();
            assert_eq!(first, again);
        }
        let requests = server.received_requests().await.unwrap();
        // Téléchargement initial + une seule tentative de revalidation (en échec).
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.url.path() == "/t/u.png")
                .count(),
            2
        );
    }
}
