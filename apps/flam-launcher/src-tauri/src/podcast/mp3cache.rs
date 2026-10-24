//! Cache des MP3 convertis par source de podcast (#460, N8 b, `docs/podcasts.md` § 12).
//!
//! Refaire un pack ne retélécharge ni ne reconvertit un épisode déjà converti : son MP3 (et la
//! pochette d'origine) est gardé sous `<cache>/podcasts/<source_id>/mp3/`, donc purgé avec la
//! source (D15) et par « Vider le cache ».
//!
//! # Entrée
//!
//! `<id>.mp3`, `<id>.img` (facultatif) et `<id>.json` ([`EntryMeta`]), `<id>` étant
//! l'identifiant opaque de l'épisode ([`episode_id`]). `<id>.json` est écrit **en dernier** et
//! vaut validation : sans lui, rien n'est servi. Il ne contient que des empreintes, jamais
//! d'URL (D11).
//!
//! # Invariants
//!
//! - Une entrée n'est servie que si son empreinte ([`EntryKey::fingerprint`] : profil de
//!   conversion, guid, URL, taille et type de l'enclosure) correspond et si la taille **et** le
//!   SHA-256 du MP3 correspondent ; sinon elle est supprimée (retéléchargement).
//! - Écriture : `<id>.json` supprimé, fichiers en `.part` puis renommés, `<id>.json` écrit en
//!   dernier (temporaire puis renommage). Une erreur (disque plein…) supprime toute l'entrée.
//! - Toute opération sur une source se fait sous son verrou ([`SourceLocks`]) et seulement si
//!   sa génération n'a pas changé : une source supprimée n'est jamais recréée.
//! - Le cache est **au mieux** : ses erreurs sont journalisées (uuid de source et nombre
//!   d'entrées seulement) et ne font jamais échouer un pack.
//! - Taille bornée ([`MAX_CACHE_BYTES`]) : éviction des entrées les moins récemment utilisées,
//!   jamais de celles protégées (pack en cours).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::OwnedMutexGuard;

use super::rss::PodcastEpisode;
use super::source::{episode_id, source_dir, CACHE_SUBDIR};
use crate::catalog::blocking;
use crate::storage;

/// Sous-dossier des MP3 dans le dossier de cache d'une source.
pub const CACHE_DIR: &str = "mp3";
/// Taille maximale du cache, toutes sources confondues (deux packs au plafond D1).
pub const MAX_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Profil de conversion : **à incrémenter** à chaque changement de la règle D6 ou de
/// `podcast/audio.rs` (toutes les entrées deviennent invalides).
pub const AUDIO_PROFILE: &str = "d6-1";
/// Version du format des entrées.
const FORMAT: u32 = 1;
const MP3_EXT: &str = "mp3";
const IMG_EXT: &str = "img";
const META_EXT: &str = "json";
const PART_EXT: &str = "part";

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Verrous et générations des sources (production : `PodcastService`, ceux des
/// rafraîchissements et de la purge).
pub trait SourceLocks: Send + Sync {
    /// Génération courante de la source (incrémentée par sa purge).
    fn generation(&self, source_id: &str) -> u64;
    /// Verrou de la source ; `None` si elle a été purgée depuis `generation`.
    fn lock<'a>(
        &'a self,
        source_id: &'a str,
        generation: u64,
    ) -> BoxFuture<'a, Option<OwnedMutexGuard<()>>>;
}

/// Verrous réels : ceux du `PodcastService` de l'application.
pub struct LiveLocks(pub tauri::AppHandle);

impl SourceLocks for LiveLocks {
    fn generation(&self, source_id: &str) -> u64 {
        use tauri::Manager;
        self.0
            .state::<super::source::PodcastService>()
            .generation(source_id)
    }

    fn lock<'a>(
        &'a self,
        source_id: &'a str,
        generation: u64,
    ) -> BoxFuture<'a, Option<OwnedMutexGuard<()>>> {
        Box::pin(async move {
            use tauri::Manager;
            self.0
                .state::<super::source::PodcastService>()
                .lock_source(source_id, generation)
                .await
        })
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Clé d'un épisode dans le cache. Sans URL (D11) : `Debug` peut être journalisé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryKey {
    pub source_id: String,
    /// Identifiant opaque ([`episode_id`]) : nom des fichiers de l'entrée.
    pub episode_id: String,
    /// Empreinte de l'audio (profil de conversion, guid, enclosure).
    pub fingerprint: String,
    /// Empreinte de l'URL de la pochette, si l'épisode en a une.
    pub image_fingerprint: Option<String>,
}

impl EntryKey {
    pub fn of(source_id: &str, episode: &PodcastEpisode) -> Self {
        let mut audio = Sha256::new();
        for part in [
            "flam-mp3-cache",
            &FORMAT.to_string(),
            AUDIO_PROFILE,
            &episode.guid,
            &episode.enclosure_url,
            &episode
                .enclosure_length
                .map(|l| l.to_string())
                .unwrap_or_default(),
            &episode.enclosure_type,
        ] {
            // Longueur en tête : deux suites de champs différentes ne se confondent jamais.
            audio.update((part.len() as u64).to_le_bytes());
            audio.update(part.as_bytes());
        }
        Self {
            source_id: source_id.to_string(),
            episode_id: episode_id(&episode.guid),
            fingerprint: hex(&audio.finalize()),
            image_fingerprint: episode
                .image
                .as_deref()
                .map(|url| hex(&Sha256::digest(url.as_bytes()))),
        }
    }
}

/// Taille et empreinte d'un fichier de l'entrée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FileMeta {
    bytes: u64,
    sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ImageMeta {
    fingerprint: String,
    #[serde(flatten)]
    file: FileMeta,
}

/// `<id>.json` : métadonnées d'une entrée (aucune URL).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EntryMeta {
    format: u32,
    fingerprint: String,
    mp3: FileMeta,
    image: Option<ImageMeta>,
    stored_at: u64,
    used_at: u64,
}

/// Épisode cherché dans le cache, avec les chemins où le placer s'il y est.
#[derive(Debug, Clone)]
pub struct Wanted {
    /// Rang de l'épisode dans le pack (rendu dans [`Hit`]).
    pub rank: usize,
    pub key: EntryKey,
    pub mp3_dest: PathBuf,
    pub image_dest: PathBuf,
}

/// Épisode servi par le cache, placé dans le dossier de travail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub rank: usize,
    pub mp3: PathBuf,
    pub mp3_bytes: u64,
    /// `None` : pas de pochette en cache (ou périmée) ; à télécharger.
    pub image: Option<PathBuf>,
}

/// Épisode converti à mettre en cache.
#[derive(Debug, Clone)]
pub struct Store {
    pub key: EntryKey,
    pub mp3: PathBuf,
    /// Pochette d'origine téléchargée (gardée seulement si la clé a une empreinte d'image).
    pub image: Option<PathBuf>,
}

/// Place `src` en `dest` : lien physique, sinon copie (système de fichiers sans liens). Une
/// copie interrompue est supprimée.
pub type Placer = dyn Fn(&Path, &Path) -> io::Result<()> + Send + Sync;

/// [`Placer`] par défaut.
pub fn link_or_copy(src: &Path, dest: &Path) -> io::Result<()> {
    if std::fs::hard_link(src, dest).is_ok() {
        return Ok(());
    }
    let copied = std::fs::copy(src, dest).and_then(|_| {
        // Copie complète et durable avant d'être déclarée valide.
        std::fs::OpenOptions::new()
            .write(true)
            .open(dest)?
            .sync_all()
    });
    if copied.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    copied.map(|_| ())
}

// ---------------------------------------------------------------------------------------
// Opérations bloquantes sur le dossier d'une source (appelant : verrou de la source tenu)
// ---------------------------------------------------------------------------------------

fn file(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{ext}"))
}

fn part(dir: &Path, id: &str, ext: &str) -> PathBuf {
    dir.join(format!("{id}.{ext}.{PART_EXT}"))
}

/// Identifiant d'épisode sûr comme nom de fichier (forme de [`episode_id`]).
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn remove_if_exists(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Supprime une entrée : `<id>.json` d'abord (elle n'est plus jamais servie), puis le reste.
fn remove_entry(dir: &Path, id: &str) -> io::Result<()> {
    remove_if_exists(&file(dir, id, META_EXT))?;
    let mut result = Ok(());
    for path in [
        file(dir, id, MP3_EXT),
        file(dir, id, IMG_EXT),
        part(dir, id, MP3_EXT),
        part(dir, id, IMG_EXT),
    ] {
        if let Err(err) = remove_if_exists(&path) {
            result = Err(err);
        }
    }
    result
}

fn read_meta(dir: &Path, id: &str) -> Option<EntryMeta> {
    let bytes = std::fs::read(file(dir, id, META_EXT)).ok()?;
    let meta: EntryMeta = serde_json::from_slice(&bytes).ok()?;
    (meta.format == FORMAT).then_some(meta)
}

/// Taille et SHA-256 de `path`, lu par blocs.
fn measure(path: &Path) -> io::Result<FileMeta> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut bytes = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        bytes += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok(FileMeta {
        bytes,
        sha256: hex(&hasher.finalize()),
    })
}

/// Le fichier a exactement la taille et l'empreinte attendues (taille vérifiée d'abord :
/// un fichier tronqué n'est pas relu).
fn intact(path: &Path, expected: &FileMeta) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() == expected.bytes)
        && measure(path).is_ok_and(|m| m == *expected)
}

/// Cherche un épisode ; s'il est servi, le place en `w.mp3_dest` (et sa pochette en
/// `w.image_dest`) et renouvelle sa date d'usage. Toute incohérence supprime l'entrée.
fn lookup_one(dir: &Path, w: &Wanted, now: u64, place: &Placer) -> Option<Hit> {
    let id = w.key.episode_id.as_str();
    if !valid_id(id) {
        return None;
    }
    let mut meta = read_meta(dir, id)?;
    if meta.fingerprint != w.key.fingerprint || !intact(&file(dir, id, MP3_EXT), &meta.mp3) {
        let _ = remove_entry(dir, id);
        return None;
    }
    let _ = remove_if_exists(&w.mp3_dest);
    if place(&file(dir, id, MP3_EXT), &w.mp3_dest).is_err() {
        let _ = remove_if_exists(&w.mp3_dest);
        return None;
    }
    let mut image = None;
    if let Some(stored) = &meta.image {
        let img = file(dir, id, IMG_EXT);
        if w.key.image_fingerprint.as_deref() != Some(stored.fingerprint.as_str())
            || !intact(&img, &stored.file)
        {
            // Pochette périmée ou altérée : retirée, le MP3 reste servi.
            meta.image = None;
            let _ = remove_if_exists(&img);
        } else {
            let _ = remove_if_exists(&w.image_dest);
            if place(&img, &w.image_dest).is_ok() {
                image = Some(w.image_dest.clone());
            } else {
                let _ = remove_if_exists(&w.image_dest);
            }
        }
    }
    meta.used_at = now;
    if storage::write_json(&file(dir, id, META_EXT), &meta).is_err() {
        // Date d'usage non renouvelée : sans gravité (l'ancienne entrée reste valide).
        tracing::debug!("cache des MP3 : date d'usage non renouvelée");
    }
    Some(Hit {
        rank: w.rank,
        mp3: w.mp3_dest.clone(),
        mp3_bytes: meta.mp3.bytes,
        image,
    })
}

/// Écrit `src` dans le fichier `ext` de l'entrée via un `.part` ; rend sa mesure.
fn write_file(dir: &Path, id: &str, ext: &str, src: &Path, place: &Placer) -> io::Result<FileMeta> {
    let measured = measure(src)?;
    let tmp = part(dir, id, ext);
    remove_if_exists(&tmp)?;
    place(src, &tmp)?;
    // Taille seulement : une copie en échec est signalée par `place`, un lien physique est le
    // fichier mesuré lui-même ; l'empreinte complète est vérifiée à chaque lecture.
    if !std::fs::metadata(&tmp).is_ok_and(|m| m.is_file() && m.len() == measured.bytes) {
        let _ = remove_if_exists(&tmp);
        return Err(io::Error::other("copie incomplète"));
    }
    storage::rename_with_retry(&tmp, &file(dir, id, ext))?;
    Ok(measured)
}

fn store_inner(dir: &Path, s: &Store, now: u64, place: &Placer) -> io::Result<u64> {
    let id = s.key.episode_id.as_str();
    std::fs::create_dir_all(dir)?;
    // L'ancienne entrée n'est plus servie dès maintenant.
    remove_entry(dir, id)?;
    let mp3 = write_file(dir, id, MP3_EXT, &s.mp3, place)?;
    let image = match (&s.key.image_fingerprint, &s.image) {
        (Some(fingerprint), Some(src)) => match write_file(dir, id, IMG_EXT, src, place) {
            Ok(file) => Some(ImageMeta {
                fingerprint: fingerprint.clone(),
                file,
            }),
            Err(err) if err.kind() == io::ErrorKind::NotFound => None,
            Err(err) => return Err(err),
        },
        _ => None,
    };
    let bytes = mp3.bytes + image.as_ref().map_or(0, |i| i.file.bytes);
    let meta = EntryMeta {
        format: FORMAT,
        fingerprint: s.key.fingerprint.clone(),
        mp3,
        image,
        stored_at: now,
        used_at: now,
    };
    storage::write_json(&file(dir, id, META_EXT), &meta)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(bytes)
}

/// Met un épisode en cache ; en cas d'erreur, rien de l'entrée ne reste.
fn store_one(dir: &Path, s: &Store, now: u64, place: &Placer) -> io::Result<u64> {
    if !valid_id(&s.key.episode_id) {
        return Err(io::Error::other("identifiant d'épisode invalide"));
    }
    let result = store_inner(dir, s, now, place);
    if result.is_err() {
        let _ = remove_entry(dir, &s.key.episode_id);
    }
    result
}

/// Entrée relevée par [`scan`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Scanned {
    source_id: String,
    episode_id: String,
    bytes: u64,
    used_at: u64,
}

/// Relevé du cache : entrées valides et restes (fichiers sans entrée valide), par source.
#[derive(Debug, Default)]
struct Survey {
    entries: Vec<Scanned>,
    junk: BTreeMap<String, Vec<String>>,
}

/// Parcourt `<cache>/podcasts/*/mp3/` sans suivre de lien. Ce qui disparaît pendant le
/// parcours est ignoré.
fn scan(cache_root: &Path) -> Survey {
    let mut survey = Survey::default();
    let Ok(sources) = std::fs::read_dir(cache_root.join(CACHE_SUBDIR)) else {
        return survey;
    };
    for source in sources.flatten() {
        let source_id = source.file_name().to_string_lossy().into_owned();
        let Ok(dir) = source_dir(cache_root, &source_id).map(|d| d.join(CACHE_DIR)) else {
            continue;
        };
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir() && !m.is_symlink()) {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut ids: HashSet<String> = HashSet::new();
        let mut junk: Vec<String> = Vec::new();
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            let stem = name.split('.').next().unwrap_or_default();
            if valid_id(stem) && !name.ends_with(PART_EXT) {
                ids.insert(stem.to_string());
            } else {
                junk.push(name);
            }
        }
        for id in ids {
            match read_meta(&dir, &id) {
                Some(meta) => {
                    let size = |ext| std::fs::metadata(file(&dir, &id, ext)).map_or(0, |m| m.len());
                    let bytes = size(MP3_EXT) + size(IMG_EXT);
                    survey.entries.push(Scanned {
                        source_id: source_id.clone(),
                        episode_id: id,
                        bytes,
                        used_at: meta.used_at,
                    });
                }
                None => junk.push(id),
            }
        }
        if !junk.is_empty() {
            survey.junk.insert(source_id, junk);
        }
    }
    survey
}

/// Entrées à évincer : les moins récemment utilisées, hors `protect`, jusqu'à repasser sous
/// `max_bytes`.
fn victims(
    entries: &[Scanned],
    max_bytes: u64,
    protect: &HashSet<(String, String)>,
) -> Vec<Scanned> {
    let mut total: u64 = entries.iter().map(|e| e.bytes).sum();
    let mut sorted: Vec<&Scanned> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        a.used_at
            .cmp(&b.used_at)
            .then_with(|| a.source_id.cmp(&b.source_id))
            .then_with(|| a.episode_id.cmp(&b.episode_id))
    });
    let mut out = Vec::new();
    for e in sorted {
        if total <= max_bytes {
            break;
        }
        if protect.contains(&(e.source_id.clone(), e.episode_id.clone())) {
            continue;
        }
        total = total.saturating_sub(e.bytes);
        out.push(e.clone());
    }
    out
}

/// Supprime, sous le verrou de la source, les restes et les entrées évincées (une entrée
/// réutilisée depuis le relevé est gardée).
fn remove_in(dir: &Path, junk: &[String], evicted: &[(String, u64)]) {
    for name in junk {
        if valid_id(name) {
            // Identifiant sans métadonnées valides au relevé : toujours sans ?
            if read_meta(dir, name).is_none() {
                let _ = remove_entry(dir, name);
            }
        } else if !name.contains(['/', '\\']) {
            let _ = remove_if_exists(&dir.join(name));
        }
    }
    for (id, used_at) in evicted {
        if read_meta(dir, id).is_none_or(|m| m.used_at == *used_at) {
            let _ = remove_entry(dir, id);
        }
    }
}

// ---------------------------------------------------------------------------------------
// Accès asynchrone pour une préparation de pack
// ---------------------------------------------------------------------------------------

/// Cache vu par une préparation : générations des sources lues **avant** le chargement des
/// flux, pour qu'une source supprimée ensuite ne soit jamais recréée.
pub struct CacheAccess {
    locks: Arc<dyn SourceLocks>,
    cache_root: PathBuf,
    generations: HashMap<String, u64>,
    max_bytes: u64,
    place: Arc<Placer>,
}

impl CacheAccess {
    pub fn new(
        locks: Arc<dyn SourceLocks>,
        cache_root: PathBuf,
        sources: impl IntoIterator<Item = String>,
    ) -> Self {
        let generations = sources
            .into_iter()
            .map(|s| {
                let g = locks.generation(&s);
                (s, g)
            })
            .collect();
        Self {
            locks,
            cache_root,
            generations,
            max_bytes: MAX_CACHE_BYTES,
            place: Arc::new(link_or_copy),
        }
    }

    /// Plafond de taille (tests).
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Fonction de placement (tests : injection de pannes).
    pub fn with_placer(mut self, place: Arc<Placer>) -> Self {
        self.place = place;
        self
    }

    fn dir(&self, source_id: &str) -> Option<PathBuf> {
        source_dir(&self.cache_root, source_id)
            .ok()
            .map(|d| d.join(CACHE_DIR))
    }

    /// Exécute `job` sur le dossier de `source_id`, sous son verrou, si la source n'a pas été
    /// purgée depuis la lecture de sa génération. `None` sinon.
    async fn under_lock<T: Send + 'static>(
        &self,
        source_id: &str,
        generation: u64,
        job: impl FnOnce(&Path) -> T + Send + 'static,
    ) -> Option<T> {
        let dir = self.dir(source_id)?;
        let guard = self.locks.lock(source_id, generation).await;
        let Some(_guard) = guard else {
            tracing::info!("cache des MP3 : source {source_id} supprimée, cache ignoré");
            return None;
        };
        blocking(move || job(&dir)).await.ok()
    }

    fn by_source<T>(items: Vec<T>, source: impl Fn(&T) -> &str) -> BTreeMap<String, Vec<T>> {
        let mut out: BTreeMap<String, Vec<T>> = BTreeMap::new();
        for item in items {
            out.entry(source(&item).to_string()).or_default().push(item);
        }
        out
    }

    /// Épisodes servis par le cache, placés dans le dossier de travail.
    pub async fn lookup(&self, wanted: Vec<Wanted>, now: u64) -> Vec<Hit> {
        let mut hits = Vec::new();
        for (source_id, wanted) in Self::by_source(wanted, |w| &w.key.source_id) {
            let Some(&generation) = self.generations.get(&source_id) else {
                continue;
            };
            let place = self.place.clone();
            let found = self
                .under_lock(&source_id, generation, move |dir| {
                    wanted
                        .iter()
                        .filter_map(|w| lookup_one(dir, w, now, &*place))
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
            hits.extend(found);
        }
        hits
    }

    /// Met en cache les épisodes convertis ; rend le nombre d'entrées écrites.
    pub async fn store(&self, items: Vec<Store>, now: u64) -> usize {
        let mut stored = 0;
        for (source_id, items) in Self::by_source(items, |s| &s.key.source_id) {
            let Some(&generation) = self.generations.get(&source_id) else {
                continue;
            };
            let place = self.place.clone();
            let sid = source_id.clone();
            let (ok, failed) = self
                .under_lock(&source_id, generation, move |dir| {
                    let mut ok = 0usize;
                    let mut failed = 0usize;
                    for s in &items {
                        match store_one(dir, s, now, &*place) {
                            Ok(_) => ok += 1,
                            Err(err) => {
                                failed += 1;
                                tracing::warn!(
                                    "cache des MP3 de la source {sid} : écriture impossible ({})",
                                    err.kind()
                                );
                            }
                        }
                    }
                    (ok, failed)
                })
                .await
                .unwrap_or_default();
            if failed > 0 {
                tracing::warn!(
                    "cache des MP3 de la source {source_id} : {failed} épisode(s) non mis en cache"
                );
            }
            stored += ok;
        }
        stored
    }

    /// Évince les entrées les moins récemment utilisées au-delà du plafond (jamais celles de
    /// `protect` : `(source, épisode)`), et supprime les restes.
    pub async fn evict(&self, protect: HashSet<(String, String)>) {
        let root = self.cache_root.clone();
        let Ok(survey) = blocking(move || scan(&root)).await else {
            return;
        };
        let victims = victims(&survey.entries, self.max_bytes, &protect);
        // Par source : restes (noms) et entrées évincées (identifiant, date d'usage relevée).
        type Cleanup = (Vec<String>, Vec<(String, u64)>);
        let mut work: BTreeMap<String, Cleanup> = survey
            .junk
            .into_iter()
            .map(|(sid, junk)| (sid, (junk, Vec::new())))
            .collect();
        for v in victims {
            work.entry(v.source_id)
                .or_default()
                .1
                .push((v.episode_id, v.used_at));
        }
        for (source_id, (junk, evicted)) in work {
            // Source quelconque (pas forcément du pack) : génération courante.
            let generation = self.locks.generation(&source_id);
            self.under_lock(&source_id, generation, move |dir| {
                remove_in(dir, &junk, &evicted);
            })
            .await;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
