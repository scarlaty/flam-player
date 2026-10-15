//! Source « catalogue » : téléchargement d'un catalogue JSON au format Telmi Store, lecture
//! tolérante, normalisation en `StoryEntry`, cache hors ligne et vignettes.
//!
//! La logique (`parse_catalog`, `refresh`, `cached_entries`, `thumbnail`) ne dépend pas de
//! Tauri : le client HTTP et le dossier de cache sont injectés, ce qui permet de la tester
//! contre un serveur HTTP local. Les commandes `catalog_*` ne sont que de fines enveloppes.
//!
//! # Cache
//!
//! Sous `<cache>/catalogs/<source_id>/` :
//! - `catalog.json` : dernier catalogue **valide** reçu, tel quel ;
//! - `meta.json` : `{ url, etag, last_modified, fetched_at }` (écrit après `catalog.json`) ;
//! - `thumbs/<sha256 de l'URL>.img` : vignettes téléchargées à la demande.
//!
//! Un cache dont l'URL diffère de celle de la source (URL modifiée) est ignoré.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use url::Url;

use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::{self, ConfigState};
use crate::story::{StoryEntry, StoryKind};
use crate::{AppError, AppResult};

/// Délai de connexion et délai total d'une requête.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Taille maximale acceptée pour un catalogue et pour une vignette.
const MAX_CATALOG_BYTES: usize = 16 * 1024 * 1024;
const MAX_THUMB_BYTES: usize = 2 * 1024 * 1024;
/// Nombre maximal d'avertissements détaillés conservés (le reste est résumé).
const MAX_WARNINGS: usize = 50;
const CACHE_FILE: &str = "catalog.json";
const META_FILE: &str = "meta.json";

/// Client HTTP partagé (état Tauri).
pub struct CatalogHttp(pub reqwest::Client);

impl CatalogHttp {
    pub fn new() -> reqwest::Result<Self> {
        build_client(CONNECT_TIMEOUT, REQUEST_TIMEOUT).map(Self)
    }
}

/// Client `reqwest` (rustls, sans OpenSSL natif) avec délais et `User-Agent` explicite.
pub fn build_client(connect: Duration, total: Duration) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(connect)
        .timeout(total)
        .user_agent(concat!(
            "flam-launcher/",
            env!("CARGO_PKG_VERSION"),
            " (+https://github.com/scarlaty/flam-player)"
        ))
        .build()
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

fn parse_entry(source_id: &str, base: &Url, item: &Value) -> Result<StoryEntry, String> {
    if !item.is_object() {
        return Err("ce n'est pas un objet".into());
    }
    let uuid = text(item, "uuid");
    if uuid.is_empty() {
        return Err("identifiant (uuid) manquant".into());
    }
    let download_url = resolve_url(base, &text(item, "download"))
        .ok_or_else(|| format!("{uuid} : lien de téléchargement absent ou invalide"))?;
    let thumb_url = item.get("thumbs").and_then(|thumbs| {
        ["medium", "small"]
            .iter()
            .find_map(|size| resolve_url(base, &text(thumbs, size)))
    });
    Ok(StoryEntry {
        source_id: source_id.to_string(),
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
/// valeur par défaut, entrée sans `uuid` ni lien de téléchargement valide ignorée avec un
/// avertissement, doublon d'`uuid` ignoré (première occurrence conservée). Une racine qui est
/// directement une liste est acceptée. `Err(CatalogInvalid)` si le JSON est invalide ou ne
/// contient aucune liste d'histoires.
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

    let mut entries: Vec<StoryEntry> = Vec::with_capacity(items.len());
    let mut warnings = Vec::new();
    let mut skipped = 0usize;
    let mut warn = |message: String| {
        if warnings.len() < MAX_WARNINGS {
            warnings.push(message);
        } else {
            skipped += 1;
        }
    };
    for (index, item) in items.iter().enumerate() {
        match parse_entry(source_id, &base, item) {
            Ok(entry) if entries.iter().any(|e| e.uuid == entry.uuid) => {
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
    /// Dernière vérification réussie (secondes Unix), 304 compris.
    fetched_at: u64,
}

struct Cached {
    meta: CacheMeta,
    parsed: ParsedCatalog,
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
    let parsed = parse_catalog(source_id, url, &bytes)
        .map_err(|err| tracing::warn!("catalogue en cache illisible ({source_id}) : {err}"))
        .ok()?;
    Some(Cached { meta, parsed })
}

fn store_cache(dir: &Path, bytes: &[u8], meta: &CacheMeta) -> AppResult<()> {
    storage::write_atomic(&dir.join(CACHE_FILE), bytes)?;
    storage::write_json(&dir.join(META_FILE), meta)
}

// ---------------------------------------------------------------------------------------
// Rafraîchissement
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
    /// Pourquoi le rafraîchissement a échoué, quand des données du cache sont servies.
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

/// Entrées du dernier catalogue en cache, sans accès réseau (vide si jamais téléchargé).
pub fn cached_entries(cache_root: &Path, source_id: &str, url: &str) -> AppResult<CatalogView> {
    let dir = source_dir(cache_root, source_id)?;
    Ok(view_from(
        source_id,
        load_cache(&dir, source_id, url),
        false,
        None,
    ))
}

fn network_error(err: &reqwest::Error) -> AppError {
    let reason = if err.is_timeout() {
        "délai dépassé"
    } else if err.is_connect() {
        "connexion impossible"
    } else {
        "échec de la requête"
    };
    AppError::Network(format!("catalogue inaccessible : {reason}"))
}

/// Lit le corps de la réponse en refusant de dépasser `max` octets.
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

/// Télécharge le catalogue (requête conditionnelle ETag / If-Modified-Since si un cache valide
/// existe) et met le cache à jour.
///
/// - 200 valide : nouveau cache ;
/// - 304 : cache conservé, date de vérification mise à jour ;
/// - échec réseau, délai, statut HTTP d'erreur : cache servi avec `offline = true` ;
/// - 200 mais catalogue inexploitable : cache servi avec `error.kind = "catalog_invalid"` ;
/// - dans les deux derniers cas **sans cache** : `Err` (`network` ou `catalog_invalid`).
pub async fn refresh(
    client: &reqwest::Client,
    cache_root: &Path,
    source_id: &str,
    url: &str,
    now: u64,
) -> AppResult<CatalogView> {
    let dir = source_dir(cache_root, source_id)?;
    let cached = load_cache(&dir, source_id, url);

    let mut request = client.get(url);
    if let Some(c) = &cached {
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
            if let Err(err) = storage::write_json(&dir.join(META_FILE), &c.meta) {
                tracing::warn!("mise à jour du cache impossible ({source_id}) : {err}");
            }
            return Ok(view_from(source_id, Some(c), false, None));
        }
        return Err(AppError::Network(
            "le serveur répond « inchangé » sans cache local".into(),
        ));
    }
    if !status.is_success() {
        let error = AppError::Network(format!("catalogue inaccessible : réponse HTTP {status}"));
        return fail(cached, error, true);
    }

    let etag = header_text(&response, ETAG);
    let last_modified = header_text(&response, LAST_MODIFIED);
    let body = match read_body(response, MAX_CATALOG_BYTES).await {
        Ok(body) => body,
        Err(err) => return fail(cached, err, true),
    };
    let parsed = match parse_catalog(source_id, url, &body) {
        Ok(parsed) => parsed,
        Err(err) => return fail(cached, err, false),
    };
    let meta = CacheMeta {
        url: url.to_string(),
        etag,
        last_modified,
        fetched_at: now,
    };
    // Un cache non écrit ne doit pas masquer un catalogue valide fraîchement reçu.
    if let Err(err) = store_cache(&dir, &body, &meta) {
        tracing::warn!("écriture du cache impossible ({source_id}) : {err}");
    }
    Ok(view_from(
        source_id,
        Some(Cached { meta, parsed }),
        false,
        None,
    ))
}

// ---------------------------------------------------------------------------------------
// Vignettes
// ---------------------------------------------------------------------------------------

/// Type MIME d'une image reconnue à ses premiers octets (PNG, JPEG, GIF, WebP). Le SVG est
/// volontairement refusé (contenu actif).
fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
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

/// Renvoie la vignette `thumb_url` d'une histoire du catalogue sous forme d'URL `data:`
/// (compatible avec la CSP, sans élargir le protocole d'asset). Téléchargée à la première
/// demande puis servie depuis `catalogs/<source_id>/thumbs/`.
///
/// `thumb_url` doit être celle d'une entrée du catalogue en cache : le front ne peut pas faire
/// télécharger une URL arbitraire. Sans réseau et sans copie locale : `Err(network)`.
pub async fn thumbnail(
    client: &reqwest::Client,
    cache_root: &Path,
    source_id: &str,
    catalog_url: &str,
    thumb_url: &str,
) -> AppResult<String> {
    let dir = source_dir(cache_root, source_id)?;
    let known = load_cache(&dir, source_id, catalog_url).is_some_and(|c| {
        c.parsed
            .entries
            .iter()
            .any(|e| e.thumb_url.as_deref() == Some(thumb_url))
    });
    if !known {
        return Err(AppError::NotFound(
            "vignette inconnue pour ce catalogue".into(),
        ));
    }

    let hash = Sha256::digest(thumb_url.as_bytes());
    let name: String = hash.iter().take(16).map(|b| format!("{b:02x}")).collect();
    let path = dir.join("thumbs").join(format!("{name}.img"));
    if let Some(url) = std::fs::read(&path).ok().and_then(|b| data_url(&b)) {
        return Ok(url);
    }

    let response = client
        .get(thumb_url)
        .send()
        .await
        .map_err(|err| network_error(&err))?;
    if !response.status().is_success() {
        return Err(AppError::Network(format!(
            "vignette inaccessible : réponse HTTP {}",
            response.status()
        )));
    }
    let body = read_body(response, MAX_THUMB_BYTES).await?;
    let url = data_url(&body).ok_or_else(|| {
        AppError::CatalogInvalid("la vignette n'est pas une image PNG, JPEG, GIF ou WebP".into())
    })?;
    if let Err(err) = storage::write_atomic(&path, &body) {
        tracing::warn!("écriture de la vignette impossible : {err}");
    }
    Ok(url)
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

/// Télécharge le catalogue de la source (cache et requête conditionnelle comprises). Sans
/// réseau, renvoie le cache avec `status.offline = true`.
#[tauri::command]
pub async fn catalog_refresh(
    config: State<'_, ConfigState>,
    http: State<'_, CatalogHttp>,
    source_id: String,
) -> AppResult<CatalogView> {
    let (url, cache_root) = resolve_from_state(&config, &source_id)?;
    refresh(&http.0, &cache_root, &source_id, &url, now_secs()).await
}

/// Entrées du dernier catalogue en cache, sans accès réseau.
#[tauri::command]
pub async fn catalog_entries(
    config: State<'_, ConfigState>,
    source_id: String,
) -> AppResult<CatalogView> {
    let (url, cache_root) = resolve_from_state(&config, &source_id)?;
    cached_entries(&cache_root, &source_id, &url)
}

/// Vignette d'une histoire du catalogue (URL `data:`), téléchargée paresseusement.
#[tauri::command]
pub async fn catalog_thumbnail(
    config: State<'_, ConfigState>,
    http: State<'_, CatalogHttp>,
    source_id: String,
    thumb_url: String,
) -> AppResult<String> {
    let (url, cache_root) = resolve_from_state(&config, &source_id)?;
    thumbnail(&http.0, &cache_root, &source_id, &url, &thumb_url).await
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
        // URL relatives résolues par rapport au catalogue ; vignette « medium » préférée.
        assert_eq!(a.download_url, "https://cat.example.test/zip/a.zip");
        assert_eq!(
            a.thumb_url.as_deref(),
            Some("https://cat.example.test/img/a-medium.png")
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

        let view = refresh(&client(), cache.path(), "src", &url, 1000)
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

        let cached = cached_entries(cache.path(), "src", &url).unwrap();
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
        refresh(&client(), cache.path(), "src", &url_of(&server), 1)
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
        let view = refresh(&client(), cache.path(), "src", &url_of(&server), 5)
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
        refresh(&client(), cache.path(), "src", &url, 1000)
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
        let view = refresh(&client(), cache.path(), "src", &url, 2000)
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
        let again = cached_entries(cache.path(), "src", &url).unwrap();
        assert_eq!(again.status.updated_at, Some(2000));
    }

    #[tokio::test]
    async fn refresh_404_sert_le_cache_hors_ligne() {
        let server = MockServer::start().await;
        serve(&server, catalog_response(&sample()), Some(1)).await;
        let cache = tempfile::tempdir().unwrap();
        let url = url_of(&server);
        refresh(&client(), cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(&server, ResponseTemplate::new(404), None).await;
        let view = refresh(&client(), cache.path(), "src", &url, 2000)
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
        let err = refresh(&client(), cache.path(), "src", &url_of(&server), 1)
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
        refresh(&client(), cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(
            &server,
            ResponseTemplate::new(200).set_delay(Duration::from_secs(3)),
            None,
        )
        .await;
        let fast = build_client(Duration::from_millis(500), Duration::from_millis(300)).unwrap();
        let view = refresh(&fast, cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert!(view.status.offline);
        assert_eq!(view.entries.len(), 2);

        let other = tempfile::tempdir().unwrap();
        let err = refresh(&fast, other.path(), "src", &url, 2000)
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
        refresh(&client(), cache.path(), "src", &url, 1000)
            .await
            .unwrap();
        drop(server);
        let view = refresh(&client(), cache.path(), "src", &url, 2000)
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
        refresh(&client(), cache.path(), "src", &url, 1000)
            .await
            .unwrap();

        serve(
            &server,
            ResponseTemplate::new(200).set_body_string("<html>erreur</html>"),
            None,
        )
        .await;
        let view = refresh(&client(), cache.path(), "src", &url, 2000)
            .await
            .unwrap();
        assert_eq!(view.entries.len(), 2);
        assert!(!view.status.offline);
        assert_eq!(view.status.error.as_ref().unwrap().kind, "catalog_invalid");

        // Sans cache : l'erreur est remontée.
        let other = tempfile::tempdir().unwrap();
        let err = refresh(&client(), other.path(), "src", &url, 2000)
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
        refresh(&client(), cache.path(), "src", &url_of(&server), 1)
            .await
            .unwrap();
        let view = cached_entries(cache.path(), "src", "http://autre.example.test/c.json").unwrap();
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
        let view = refresh(&client(), cache.path(), "src", &url, 1)
            .await
            .unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();

        let first = thumbnail(&client(), cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert!(first.starts_with("data:image/png;base64,"));
        // Deuxième appel : servie depuis le disque (le mock n'attend qu'une requête).
        let second = thumbnail(&client(), cache.path(), "src", &url, &thumb)
            .await
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(
            std::fs::read_dir(cache.path().join("catalogs/src/thumbs"))
                .unwrap()
                .count(),
            1
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
        let view = refresh(&client(), cache.path(), "src", &url, 1)
            .await
            .unwrap();
        let thumb = view.entries[0].thumb_url.clone().unwrap();

        let err = thumbnail(
            &client(),
            cache.path(),
            "src",
            &url,
            "http://evil.example.test/x.png",
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind(), "not_found");

        let err = thumbnail(&client(), cache.path(), "src", &url, &thumb)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "catalog_invalid");
        assert!(!cache.path().join("catalogs/src/thumbs").exists());

        drop(server);
        let err = thumbnail(&client(), cache.path(), "src", &url, &thumb)
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
}
