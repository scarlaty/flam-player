//! Titres et images des histoires officielles depuis le catalogue en ligne (#290), pour les
//! histoires de « Ma Flam » dont le titre n'a pas pu être déchiffré (`info` chiffré avec la clé
//! d'un autre compte).
//!
//! # Quand le réseau est utilisé
//!
//! Le front ne demande ce catalogue que pour les histoires **sans titre déchiffré**. Le service
//! n'appelle l'API que si, parmi les UUID demandés, au moins un est absent du cache **et** que
//! le cache n'existe pas ou date de plus d'un jour ([`CACHE_MAX_AGE`]) :
//!
//! - aucun UUID demandé : ni cache lu, ni réseau ;
//! - tous présents dans le cache, même ancien : réponse du cache, sans réseau ;
//! - un absent mais cache de moins d'un jour : réponse du cache (le catalogue ne le contient
//!   pas), sans réseau ;
//! - un absent et cache périmé ou inexistant : un rafraîchissement, au plus un par jour en cas
//!   de succès, et **au plus un par session en cas d'échec** (serveur injoignable, format
//!   inattendu, réponse trop grosse : on se replie sur le cache, sans erreur).
//!
//! # Ce qui est envoyé
//!
//! Deux requêtes `GET` en HTTPS, sans corps ni paramètre : un jeton invité, puis le catalogue
//! complet. **Rien de la Flam n'est envoyé** (ni numéro de série, ni clé, ni UUID d'histoire) :
//! le filtrage par UUID se fait localement. Les images, téléchargées à la demande, sont
//! demandées par leur chemin dans le catalogue, jamais par une valeur venue du front.
//!
//! # Cache
//!
//! `<cache>/official/catalog.json` : date du dernier téléchargement et, par UUID, titre et chemin
//! de l'image (écriture atomique). Les images réduites vont dans `<cache>/thumbs/` comme celles
//! de la Flam. L'API n'est pas documentée : le format est lu avec tolérance, et un catalogue
//! sans aucune entrée exploitable ne remplace jamais un cache existant.
//!
//! Le jeton et les URL ne sont jamais journalisés.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;
use tokio::sync::Semaphore;
use url::Url;

use super::index::StoryId;
use crate::catalog::{
    blocking, read_body, sniff_image, CONNECT_TIMEOUT, MAX_CATALOG_BYTES, MAX_THUMB_BYTES,
    REQUEST_TIMEOUT, USER_AGENT,
};
use crate::settings::{Environment, SettingsStore};
use crate::storage::{write_atomic, ConfigState};
use crate::thumbnail::{self, ThumbCache, ThumbError};
use crate::AppResult;

/// Jeton invité.
const TOKEN_URL: &str = "https://server-auth-prod.lunii.com/guest/create";
/// Catalogue complet.
const PACKS_URL: &str = "https://server-data-prod.lunii.com/v2/packs";
/// Préfixe des images : s'ajoute au chemin `/public/images/…` du catalogue.
const IMAGE_BASE: &str = "https://storage.googleapis.com/lunii-data-prod";
/// En-tête exigé par le catalogue.
const SENDER: (&str, &str) = ("application-sender", "luniistore_desktop");
const AUTH_HEADER: &str = "x-auth-token";

/// Âge au-delà duquel le cache est rafraîchi (si une histoire demandée y manque).
pub const CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// Taille maximale de la réponse du jeton.
const MAX_TOKEN_BYTES: usize = 64 * 1024;
/// Longueur maximale d'un jeton accepté.
const MAX_TOKEN_LEN: usize = 4096;
/// Taille maximale lue du fichier de cache.
const MAX_CACHE_BYTES: u64 = 16 * 1024 * 1024;
/// Entrées conservées au plus (au-delà : ignorées).
const MAX_ENTRIES: usize = 20_000;
/// UUID pris en compte par demande.
pub const MAX_QUERY: usize = 1000;
/// Longueur maximale d'un titre conservé (en caractères).
const MAX_TITLE_CHARS: usize = 200;
/// Côté maximal de l'image réduite (comme les vignettes de la Flam).
const THUMB_DIM: u32 = 96;
/// Images téléchargées en même temps.
const IMAGE_CONCURRENCY: usize = 3;
const CACHE_VERSION: u32 = 1;
const CACHE_DIR: &str = "official";
const CACHE_FILE: &str = "catalog.json";

/// Entrée du catalogue : titre et chemin de l'image (`/public/images/…`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    #[serde(rename = "t")]
    title: String,
    #[serde(rename = "i", default, skip_serializing_if = "Option::is_none")]
    image: Option<String>,
}

/// Contenu du fichier de cache.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CacheFile {
    version: u32,
    /// Secondes Unix du dernier téléchargement réussi.
    fetched_at: u64,
    entries: BTreeMap<String, Entry>,
}

/// Titre d'une histoire trouvée dans le catalogue officiel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OfficialTitle {
    pub uuid: String,
    pub title: String,
    /// Le catalogue a une image pour cette histoire (voir [`device_official_thumbnail`]).
    pub has_image: bool,
}

/// Réponse de `device_official_titles`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
pub struct OfficialTitles {
    /// Histoires demandées que le catalogue connaît (les autres sont omises).
    pub titles: Vec<OfficialTitle>,
}

/// Service du catalogue officiel (état Tauri).
pub struct OfficialCatalog {
    client: reqwest::Client,
    token_url: String,
    packs_url: String,
    image_base: String,
    /// HTTPS exigé (toujours vrai hors tests).
    https_only: bool,
    max_packs_bytes: usize,
    /// Un seul rafraîchissement à la fois.
    refresh: tokio::sync::Mutex<()>,
    /// Un échec de rafraîchissement a eu lieu pendant cette session.
    failed: Mutex<bool>,
    image_slots: Semaphore,
}

impl OfficialCatalog {
    pub fn new() -> reqwest::Result<Self> {
        no_redirect_client(CONNECT_TIMEOUT, REQUEST_TIMEOUT).map(|client| {
            Self::with_endpoints(
                client,
                TOKEN_URL.into(),
                PACKS_URL.into(),
                IMAGE_BASE.into(),
                true,
            )
        })
    }

    fn with_endpoints(
        client: reqwest::Client,
        token_url: String,
        packs_url: String,
        image_base: String,
        https_only: bool,
    ) -> Self {
        Self {
            client,
            token_url,
            packs_url,
            image_base,
            https_only,
            max_packs_bytes: MAX_CATALOG_BYTES,
            refresh: tokio::sync::Mutex::new(()),
            failed: Mutex::new(false),
            image_slots: Semaphore::new(IMAGE_CONCURRENCY),
        }
    }

    fn session_failed(&self) -> bool {
        *self.failed.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn set_session_failed(&self) {
        *self.failed.lock().unwrap_or_else(|e| e.into_inner()) = true;
    }

    /// Titres du catalogue pour `uuids` (voir la documentation du module pour les appels
    /// réseau). Ne renvoie jamais d'erreur : sans cache ni réseau, la réponse est vide.
    pub async fn titles(
        &self,
        cache_dir: &Path,
        uuids: &[String],
        now: SystemTime,
    ) -> OfficialTitles {
        let ids = normalize(uuids);
        if ids.is_empty() {
            return OfficialTitles::default();
        }
        let path = cache_path(cache_dir);
        let mut cache = read_cache_async(&path).await;
        if self.must_refresh(cache.as_ref(), &ids, now) {
            let _turn = self.refresh.lock().await;
            // Un autre appel a pu rafraîchir pendant l'attente.
            cache = read_cache_async(&path).await;
            if self.must_refresh(cache.as_ref(), &ids, now) {
                match self.download().await {
                    Ok(entries) => {
                        let fresh = CacheFile {
                            version: CACHE_VERSION,
                            fetched_at: unix_secs(now),
                            entries,
                        };
                        let (stored, written) = store_cache(&path, fresh).await;
                        cache = stored;
                        if !written {
                            // Sans cache durable, chaque appel retélécharge : on s'arrête là.
                            self.set_session_failed();
                        }
                    }
                    Err(reason) => {
                        tracing::warn!("catalogue officiel indisponible : {reason}");
                        self.set_session_failed();
                    }
                }
            }
        }
        answer(cache.as_ref(), &ids)
    }

    /// Faut-il appeler l'API ? Seulement si un UUID demandé manque au cache et que celui-ci est
    /// absent ou périmé, et qu'aucun échec n'a eu lieu pendant cette session.
    fn must_refresh(&self, cache: Option<&CacheFile>, ids: &[StoryId], now: SystemTime) -> bool {
        let missing = |c: &CacheFile| ids.iter().any(|id| !c.entries.contains_key(id.as_str()));
        let wanted = match cache {
            Some(c) => missing(c) && is_stale(c, now),
            None => true,
        };
        wanted && !self.session_failed()
    }

    /// Jeton invité puis catalogue. Le message d'erreur est destiné au journal : il ne contient
    /// ni jeton ni URL.
    async fn download(&self) -> Result<BTreeMap<String, Entry>, String> {
        let token = self.guest_token().await?;
        let url = self.checked_url(&self.packs_url)?;
        let mut headers = HeaderMap::new();
        headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
        headers.insert(
            HeaderName::from_static(SENDER.0),
            HeaderValue::from_static(SENDER.1),
        );
        let mut auth =
            HeaderValue::from_str(&token).map_err(|_| "jeton inutilisable".to_string())?;
        auth.set_sensitive(true);
        headers.insert(HeaderName::from_static(AUTH_HEADER), auth);
        let response = self
            .client
            .get(url)
            .headers(headers)
            .send()
            .await
            .map_err(|e| request_reason(&e))?;
        if !response.status().is_success() {
            return Err(format!(
                "catalogue : état HTTP {}",
                response.status().as_u16()
            ));
        }
        let body = read_body(response, self.max_packs_bytes)
            .await
            .map_err(|e| e.to_string())?;
        let parsed = blocking(move || parse_packs(&body))
            .await
            .map_err(|e| e.to_string())?;
        parsed.map_err(str::to_string)
    }

    async fn guest_token(&self) -> Result<String, String> {
        let url = self.checked_url(&self.token_url)?;
        let response = self
            .client
            .get(url)
            .header(ACCEPT, "application/json")
            .send()
            .await
            .map_err(|e| request_reason(&e))?;
        if !response.status().is_success() {
            return Err(format!("jeton : état HTTP {}", response.status().as_u16()));
        }
        let body = read_body(response, MAX_TOKEN_BYTES)
            .await
            .map_err(|e| e.to_string())?;
        parse_token(&body).ok_or_else(|| "jeton : format inattendu".to_string())
    }

    fn checked_url(&self, text: &str) -> Result<Url, String> {
        let url = Url::parse(text).map_err(|_| "URL invalide".to_string())?;
        if self.https_only && url.scheme() != "https" {
            return Err("HTTPS exigé".into());
        }
        Ok(url)
    }

    /// Image réduite de l'histoire `uuid` (URL `data:` d'un PNG), depuis le cache si possible,
    /// sinon téléchargée (au plus [`MAX_THUMB_BYTES`]). Seules les histoires du cache du
    /// catalogue sont servies. `None` en cas d'échec, sans erreur.
    pub async fn thumbnail(&self, cache_dir: &Path, uuid: &str) -> Option<String> {
        let id = StoryId::parse(uuid)?;
        let cache = read_cache_async(&cache_path(cache_dir)).await?;
        let image = cache.entries.get(id.as_str())?.image.clone()?;
        let key = image_key(&id, &image);
        let dir = cache_dir.to_path_buf();
        let cached = {
            let dir = dir.clone();
            blocking(move || {
                ThumbCache::new(&dir)
                    .get_or_make::<ThumbError>(key, THUMB_DIM, || {
                        Err(ThumbError::Decode("absente du cache".into()))
                    })
                    .ok()
            })
            .await
            .ok()?
        };
        if cached.is_some() {
            return cached;
        }
        let _slot = self.image_slots.acquire().await.ok()?;
        let url = self
            .checked_url(&format!("{}{image}", self.image_base))
            .map_err(|reason| tracing::debug!("image du catalogue : {reason}"))
            .ok()?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| tracing::debug!("image du catalogue : {}", request_reason(&e)))
            .ok()?;
        if !response.status().is_success() {
            tracing::debug!(
                "image du catalogue : état HTTP {}",
                response.status().as_u16()
            );
            return None;
        }
        let bytes = read_body(response, MAX_THUMB_BYTES)
            .await
            .map_err(|e| tracing::debug!("image du catalogue : {e}"))
            .ok()?;
        if sniff_image(&bytes) != Some("image/png") {
            tracing::debug!("image du catalogue : format non pris en charge");
            return None;
        }
        blocking(move || {
            ThumbCache::new(&dir)
                .get_or_make::<ThumbError>(key, THUMB_DIM, || thumbnail::decode_png(&bytes))
                .map_err(|e| tracing::debug!("image du catalogue illisible : {e}"))
                .ok()
        })
        .await
        .ok()?
    }
}

/// Client sans redirection : les hôtes sont connus d'avance, et une redirection enverrait le
/// jeton (`X-AUTH-TOKEN`, que `reqwest` ne retire pas) ou une requête vers un autre hôte.
fn no_redirect_client(connect: Duration, total: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(connect)
        .timeout(total)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(USER_AGENT)
        .build()
}

/// Motif d'échec d'une requête, sans URL (celle de `reqwest::Error` en contient une).
fn request_reason(err: &reqwest::Error) -> String {
    let reason = if err.is_timeout() {
        "délai dépassé"
    } else if err.is_redirect() {
        "redirection refusée"
    } else if err.is_connect() {
        "connexion impossible"
    } else {
        "échec de la requête"
    };
    reason.to_string()
}

fn unix_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Le cache est périmé s'il a plus d'un jour, ou s'il date d'un futur lointain (horloge
/// déréglée : sans cela il ne serait jamais rafraîchi).
fn is_stale(cache: &CacheFile, now: SystemTime) -> bool {
    let now = unix_secs(now);
    let max = CACHE_MAX_AGE.as_secs();
    now.saturating_sub(cache.fetched_at) >= max || cache.fetched_at > now + max
}

/// UUID valides, en minuscules, sans doublon, au plus [`MAX_QUERY`].
fn normalize(uuids: &[String]) -> Vec<StoryId> {
    let mut seen = HashSet::new();
    uuids
        .iter()
        .filter_map(|u| StoryId::parse(u))
        .filter(|id| seen.insert(id.clone()))
        .take(MAX_QUERY)
        .collect()
}

fn answer(cache: Option<&CacheFile>, ids: &[StoryId]) -> OfficialTitles {
    let Some(cache) = cache else {
        return OfficialTitles::default();
    };
    let titles = ids
        .iter()
        .filter_map(|id| {
            let entry = cache.entries.get(id.as_str())?;
            Some(OfficialTitle {
                uuid: id.to_string(),
                title: entry.title.clone(),
                has_image: entry.image.is_some(),
            })
        })
        .collect();
    OfficialTitles { titles }
}

// ---------------------------------------------------------------------------------------
// Lecture tolérante des réponses
// ---------------------------------------------------------------------------------------

/// Jeton invité : `response.token.server` (ASCII visible, longueur bornée).
fn parse_token(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    let token = value
        .get("response")?
        .get("token")?
        .get("server")?
        .as_str()?;
    let valid = !token.is_empty()
        && token.len() <= MAX_TOKEN_LEN
        && token.bytes().all(|b| b.is_ascii_graphic());
    valid.then(|| token.to_string())
}

/// Catalogue : `response` est un objet dont chaque valeur décrit une histoire (`uuid`, `title`,
/// `localized_infos.<langue>.image.image_url`). Les entrées inexploitables sont ignorées ;
/// aucune entrée exploitable : échec.
fn parse_packs(body: &[u8]) -> Result<BTreeMap<String, Entry>, &'static str> {
    let root: Value = serde_json::from_slice(body).map_err(|_| "catalogue : JSON invalide")?;
    let packs = root
        .get("response")
        .and_then(Value::as_object)
        .ok_or("catalogue : format inattendu")?;
    let mut entries = BTreeMap::new();
    for pack in packs.values().take(MAX_ENTRIES) {
        let Some(id) = pack
            .get("uuid")
            .and_then(Value::as_str)
            .and_then(StoryId::parse)
        else {
            continue;
        };
        let Some(title) = pack_title(pack) else {
            continue;
        };
        entries.insert(
            id.to_string(),
            Entry {
                title,
                image: pack_image(pack),
            },
        );
    }
    if entries.is_empty() {
        return Err("catalogue : aucune entrée exploitable");
    }
    Ok(entries)
}

/// Titre : champ `title`, sinon celui de la première langue qui en a un.
fn pack_title(pack: &Value) -> Option<String> {
    let direct = pack.get("title").and_then(Value::as_str);
    let localized = || {
        pack.get("localized_infos")?
            .as_object()?
            .values()
            .find_map(|info| info.get("title")?.as_str())
    };
    clean_title(direct.or_else(localized)?)
}

/// Titre nettoyé : espaces de bord retirés, caractères de contrôle écartés, longueur bornée.
fn clean_title(raw: &str) -> Option<String> {
    let text: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_TITLE_CHARS)
        .collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Chemin de l'image : langue `fr_FR` d'abord, sinon la première langue qui en a une.
fn pack_image(pack: &Value) -> Option<String> {
    let infos = pack.get("localized_infos")?.as_object()?;
    let path_of = |info: &Value| {
        let path = info.get("image")?.get("image_url")?.as_str()?;
        valid_image_path(path).then(|| path.to_string())
    };
    infos
        .get("fr_FR")
        .and_then(path_of)
        .or_else(|| infos.values().find_map(path_of))
}

/// Chemin d'image accepté : sous `/public/images/`, caractères sûrs seulement, sans `..`.
fn valid_image_path(path: &str) -> bool {
    path.starts_with("/public/images/")
        && path.len() <= 256
        && !path.contains("..")
        && !path.contains("//")
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'))
}

fn image_key(id: &StoryId, image: &str) -> u64 {
    thumbnail::fnv1a(&[
        b"flam-official-thumb/v1",
        id.as_str().as_bytes(),
        image.as_bytes(),
        &THUMB_DIM.to_le_bytes(),
    ])
}

// ---------------------------------------------------------------------------------------
// Cache disque
// ---------------------------------------------------------------------------------------

fn cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join(CACHE_DIR).join(CACHE_FILE)
}

/// Lit le cache : absent, trop gros, illisible ou d'une autre version, il est ignoré.
fn read_cache(path: &Path) -> Option<CacheFile> {
    use std::io::Read;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mut data = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut data)
        .ok()?;
    if data.len() as u64 > MAX_CACHE_BYTES {
        return None;
    }
    let mut cache: CacheFile = serde_json::from_slice(&data)
        .map_err(|_| tracing::debug!("cache du catalogue officiel illisible"))
        .ok()?;
    if cache.version != CACHE_VERSION {
        return None;
    }
    // Clés et chemins relus avec la même rigueur qu'à la réception.
    cache.entries.retain(|uuid, entry| {
        if StoryId::parse(uuid).is_none_or(|id| id.as_str() != uuid) {
            return false;
        }
        if entry.image.as_deref().is_some_and(|i| !valid_image_path(i)) {
            entry.image = None;
        }
        !entry.title.is_empty()
    });
    Some(cache)
}

async fn read_cache_async(path: &Path) -> Option<CacheFile> {
    let path = path.to_path_buf();
    blocking(move || read_cache(&path)).await.ok().flatten()
}

/// Écrit le cache (atomique). Un échec d'écriture n'est pas une erreur : le catalogue reçu sert
/// quand même pour cet appel (le booléen dit si l'écriture a réussi).
async fn store_cache(path: &Path, cache: CacheFile) -> (Option<CacheFile>, bool) {
    let path = path.to_path_buf();
    let result = blocking(move || {
        let written = match serde_json::to_vec(&cache) {
            Ok(bytes) if bytes.len() as u64 <= MAX_CACHE_BYTES => write_atomic(&path, &bytes)
                .map_err(|err| tracing::warn!("cache du catalogue officiel non écrit : {err}"))
                .is_ok(),
            Ok(_) => {
                tracing::warn!("cache du catalogue officiel trop gros : non écrit");
                false
            }
            Err(err) => {
                tracing::warn!("cache du catalogue officiel non sérialisé : {err}");
                false
            }
        };
        (cache, written)
    })
    .await;
    match result {
        Ok((cache, written)) => (Some(cache), written),
        Err(_) => (None, false),
    }
}

// ---------------------------------------------------------------------------------------
// Commandes
// ---------------------------------------------------------------------------------------

fn resolve_cache_dir(config: &ConfigState) -> AppResult<PathBuf> {
    let _guard = config.guard();
    SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())
}

/// Titres du catalogue officiel pour des histoires de la Flam sans titre déchiffré. N'appelle
/// l'API que si nécessaire (voir la documentation du module) ; aucune donnée de la Flam n'est
/// envoyée.
#[tauri::command]
pub async fn device_official_titles(
    config: State<'_, ConfigState>,
    service: State<'_, OfficialCatalog>,
    uuids: Vec<String>,
) -> AppResult<OfficialTitles> {
    if normalize(&uuids).is_empty() {
        return Ok(OfficialTitles::default());
    }
    let cache_dir = resolve_cache_dir(&config)?;
    Ok(service.titles(&cache_dir, &uuids, SystemTime::now()).await)
}

/// Image du catalogue officiel pour une histoire déjà trouvée par `device_official_titles`
/// (URL `data:` d'un PNG, `None` si absente ou indisponible).
#[tauri::command]
pub async fn device_official_thumbnail(
    config: State<'_, ConfigState>,
    service: State<'_, OfficialCatalog>,
    uuid: String,
) -> AppResult<Option<String>> {
    let cache_dir = resolve_cache_dir(&config)?;
    Ok(service.thumbnail(&cache_dir, &uuid).await)
}

#[cfg(test)]
mod tests;
