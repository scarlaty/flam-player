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
//!   dernier (temporaire puis renommage). Une erreur (disque plein…) supprime toute l'entrée,
//!   sauf pour une pochette seule remplacée (voir « Annulation » ci-dessous).
//! - Toute opération sur une source se fait sous son verrou ([`SourceLocks`]) et seulement si
//!   sa génération n'a pas changé : une source supprimée n'est jamais recréée.
//! - Le cache est **au mieux** : ses erreurs sont journalisées (uuid de source et nombre
//!   d'entrées seulement) et ne font jamais échouer un pack.
//! - Taille bornée ([`MAX_CACHE_BYTES`]) : éviction des entrées les moins récemment utilisées,
//!   jamais de celles protégées (pack en cours).
//! - Aucun lien suivi (#466) : `podcasts/`, `podcasts/<id>/` et `podcasts/<id>/mp3/` doivent
//!   être de vrais dossiers (ni lien symbolique ni jonction), sinon la source est ignorée ; les
//!   fichiers d'une entrée doivent être de vrais fichiers.
//! - Annulation (#466) : le hachage et la copie s'arrêtent au bloc suivant ; une lecture
//!   annulée ne supprime jamais une entrée, une écriture annulée ne laisse rien. La remise en
//!   cache d'un épisode servi dont seule la pochette a changé ([`Store::image_only`], #489) ne
//!   touche qu'à la pochette : annulée ou en échec, le MP3 reste en cache et servi.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::future::Future;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{watch, OwnedMutexGuard};

use super::archive::MAX_AUDIO_BYTES;
use super::rss::PodcastEpisode;
use super::source::{episode_id, source_dir, PodcastService, CACHE_SUBDIR};
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
/// Taille maximale d'un `<id>.json` lu.
const MAX_META_BYTES: u64 = 64 * 1024;
/// Taille des blocs lus (hachage) ou copiés : l'annulation est vue entre deux blocs.
const CHUNK: usize = 256 * 1024;

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

/// Verrous et générations du `PodcastService` : ceux des rafraîchissements et de la purge.
impl SourceLocks for PodcastService {
    fn generation(&self, source_id: &str) -> u64 {
        PodcastService::generation(self, source_id)
    }

    fn lock<'a>(
        &'a self,
        source_id: &'a str,
        generation: u64,
    ) -> BoxFuture<'a, Option<OwnedMutexGuard<()>>> {
        Box::pin(self.lock_source(source_id, generation))
    }
}

/// Verrous réels : ceux du `PodcastService` de l'application (état Tauri).
pub struct LiveLocks(pub tauri::AppHandle);

impl SourceLocks for LiveLocks {
    fn generation(&self, source_id: &str) -> u64 {
        use tauri::Manager;
        SourceLocks::generation(self.0.state::<PodcastService>().inner(), source_id)
    }

    fn lock<'a>(
        &'a self,
        source_id: &'a str,
        generation: u64,
    ) -> BoxFuture<'a, Option<OwnedMutexGuard<()>>> {
        use tauri::Manager;
        SourceLocks::lock(
            self.0.state::<PodcastService>().inner(),
            source_id,
            generation,
        )
    }
}

/// Drapeau d'annulation de la tâche, lu entre deux blocs par les opérations bloquantes
/// (hachage, copie) ; par défaut, jamais levé.
#[derive(Clone, Default)]
pub struct Cancel(Option<watch::Receiver<bool>>);

impl Cancel {
    pub fn new(flag: watch::Receiver<bool>) -> Self {
        Self(Some(flag))
    }

    pub fn is_set(&self) -> bool {
        self.0.as_ref().is_some_and(|flag| *flag.borrow())
    }

    /// `Err(Interrupted)` si l'annulation est demandée.
    fn check(&self) -> io::Result<()> {
        if self.is_set() {
            Err(io::Error::new(io::ErrorKind::Interrupted, "annulé"))
        } else {
            Ok(())
        }
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
    /// Épisode servi par le cache dont seule la pochette a changé (#489) : si l'entrée est
    /// toujours celle de `key` (empreinte et taille du MP3), seule sa pochette est remplacée ;
    /// une panne ou une annulation n'efface alors jamais le MP3. Sinon, l'entrée est réécrite
    /// en entier depuis `mp3`.
    pub image_only: bool,
}

/// Place `src` en `dest` : lien physique, sinon copie (système de fichiers sans liens). Une
/// copie interrompue (erreur ou annulation) est supprimée.
pub type Placer = dyn Fn(&Path, &Path, &Cancel) -> io::Result<()> + Send + Sync;

/// [`Placer`] par défaut.
pub fn link_or_copy(src: &Path, dest: &Path, cancel: &Cancel) -> io::Result<()> {
    if std::fs::hard_link(src, dest).is_ok() {
        return Ok(());
    }
    copy_file(src, dest, cancel)
}

/// Copie par blocs, annulable entre deux blocs, durable avant d'être déclarée réussie ; en
/// cas d'échec, `dest` est supprimé.
fn copy_file(src: &Path, dest: &Path, cancel: &Cancel) -> io::Result<()> {
    let copied = (|| {
        let mut from = std::fs::File::open(src)?;
        let mut to = std::fs::File::create(dest)?;
        let mut buf = vec![0u8; CHUNK];
        loop {
            cancel.check()?;
            let n = from.read(&mut buf)?;
            if n == 0 {
                break;
            }
            to.write_all(&buf[..n])?;
        }
        to.sync_all()
    })();
    if copied.is_err() {
        let _ = std::fs::remove_file(dest);
    }
    copied
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

/// Vrai dossier : ni lien symbolique ni jonction (`symlink_metadata` ne les suit pas, et une
/// jonction y est vue comme un lien).
fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
}

/// `podcasts/<source_id>/mp3/`, si `podcasts/`, `podcasts/<source_id>/` et `mp3/` sont de
/// vrais dossiers : un lien ou une jonction sur ce chemin pourrait faire lire, écrire ou
/// supprimer hors du cache (#466). `Ok(None)` : un dossier manque (et `create` est faux).
/// Avec `create`, les dossiers manquants sont créés un par un puis recontrôlés.
fn real_mp3_dir(cache_root: &Path, source_id: &str, create: bool) -> io::Result<Option<PathBuf>> {
    let source =
        source_dir(cache_root, source_id).map_err(|err| io::Error::other(err.to_string()))?;
    let mp3 = source.join(CACHE_DIR);
    if create {
        std::fs::create_dir_all(cache_root)?;
    }
    for dir in [&cache_root.join(CACHE_SUBDIR), &source, &mp3] {
        match std::fs::symlink_metadata(dir) {
            Ok(_) if is_real_dir(dir) => continue,
            Ok(_) => {}
            Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
            Err(_) if !create => return Ok(None),
            Err(_) => match std::fs::create_dir(dir) {
                Err(err) if err.kind() != io::ErrorKind::AlreadyExists => return Err(err),
                _ if is_real_dir(dir) => continue,
                _ => {}
            },
        }
        return Err(io::Error::other(
            "chemin du cache des MP3 refusé : pas un vrai dossier (lien, jonction ou fichier)",
        ));
    }
    Ok(Some(mp3))
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
    let path = file(dir, id, META_EXT);
    // Vrai fichier de taille raisonnable seulement (jamais un lien suivi).
    real_file_len(&path).filter(|&len| len <= MAX_META_BYTES)?;
    let bytes = std::fs::read(path).ok()?;
    let meta: EntryMeta = serde_json::from_slice(&bytes).ok()?;
    (meta.format == FORMAT).then_some(meta)
}

/// Taille et SHA-256 de `path`, lu par blocs ; `Err(Interrupted)` si l'annulation est
/// demandée en cours de lecture.
fn measure(path: &Path, cancel: &Cancel) -> io::Result<FileMeta> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut bytes = 0u64;
    loop {
        cancel.check()?;
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

/// Taille d'un vrai fichier (jamais un lien suivi).
fn real_file_len(path: &Path) -> Option<u64> {
    std::fs::symlink_metadata(path)
        .ok()
        .filter(|m| m.is_file())
        .map(|m| m.len())
}

/// Le fichier (vrai fichier, pas un lien) a exactement la taille et l'empreinte attendues
/// (taille vérifiée d'abord : un fichier tronqué n'est pas relu).
fn intact(path: &Path, expected: &FileMeta, cancel: &Cancel) -> bool {
    real_file_len(path) == Some(expected.bytes)
        && measure(path, cancel).is_ok_and(|m| m == *expected)
}

/// Paramètres communs des opérations bloquantes d'une préparation.
struct Op<'a> {
    now: u64,
    place: &'a Placer,
    cancel: &'a Cancel,
    /// Taille au-delà de laquelle un MP3 en cache n'est plus servi (borne du pack).
    max_entry_bytes: u64,
}

/// Cherche un épisode ; s'il est servi, le place en `w.mp3_dest` (et sa pochette en
/// `w.image_dest`) et renouvelle sa date d'usage. Toute incohérence supprime l'entrée ; une
/// annulation ne supprime rien.
fn lookup_one(dir: &Path, w: &Wanted, op: &Op<'_>) -> Option<Hit> {
    let id = w.key.episode_id.as_str();
    if !valid_id(id) {
        return None;
    }
    let mut meta = read_meta(dir, id)?;
    if meta.fingerprint == w.key.fingerprint && meta.mp3.bytes > op.max_entry_bytes {
        // Le pack le refuserait : l'entrée est invalidée (borne abaissée depuis l'écriture).
        tracing::warn!(
            "cache des MP3 de la source {} : entrée trop volumineuse supprimée",
            w.key.source_id
        );
        let _ = remove_entry(dir, id);
        return None;
    }
    if meta.fingerprint != w.key.fingerprint
        || !intact(&file(dir, id, MP3_EXT), &meta.mp3, op.cancel)
    {
        // Hachage interrompu : l'entrée n'est pas jugée.
        if !op.cancel.is_set() {
            let _ = remove_entry(dir, id);
        }
        return None;
    }
    let _ = remove_if_exists(&w.mp3_dest);
    if (op.place)(&file(dir, id, MP3_EXT), &w.mp3_dest, op.cancel).is_err() {
        let _ = remove_if_exists(&w.mp3_dest);
        return None;
    }
    let mut image = None;
    if let Some(stored) = &meta.image {
        let img = file(dir, id, IMG_EXT);
        if w.key.image_fingerprint.as_deref() != Some(stored.fingerprint.as_str())
            || !intact(&img, &stored.file, op.cancel)
        {
            if op.cancel.is_set() {
                // Rien n'est jugé ni renouvelé : l'épisode n'est pas servi.
                let _ = remove_if_exists(&w.mp3_dest);
                return None;
            }
            // Pochette périmée ou altérée : retirée, le MP3 reste servi.
            meta.image = None;
            let _ = remove_if_exists(&img);
        } else {
            let _ = remove_if_exists(&w.image_dest);
            if (op.place)(&img, &w.image_dest, op.cancel).is_ok() {
                image = Some(w.image_dest.clone());
            } else {
                let _ = remove_if_exists(&w.image_dest);
                if op.cancel.is_set() {
                    // Copie interrompue : rien n'est renouvelé, l'épisode n'est pas servi.
                    let _ = remove_if_exists(&w.mp3_dest);
                    return None;
                }
            }
        }
    }
    meta.used_at = op.now;
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
fn write_file(dir: &Path, id: &str, ext: &str, src: &Path, op: &Op<'_>) -> io::Result<FileMeta> {
    let measured = measure(src, op.cancel)?;
    let tmp = part(dir, id, ext);
    remove_if_exists(&tmp)?;
    (op.place)(src, &tmp, op.cancel)?;
    // Taille seulement : une copie en échec est signalée par `place`, un lien physique est le
    // fichier mesuré lui-même ; l'empreinte complète est vérifiée à chaque lecture.
    if real_file_len(&tmp) != Some(measured.bytes) {
        let _ = remove_if_exists(&tmp);
        return Err(io::Error::other("copie incomplète"));
    }
    op.cancel.check()?;
    storage::rename_with_retry(&tmp, &file(dir, id, ext))?;
    Ok(measured)
}

fn store_inner(dir: &Path, s: &Store, op: &Op<'_>) -> io::Result<u64> {
    let id = s.key.episode_id.as_str();
    // L'ancienne entrée n'est plus servie dès maintenant.
    remove_entry(dir, id)?;
    let mp3 = write_file(dir, id, MP3_EXT, &s.mp3, op)?;
    let image = match (&s.key.image_fingerprint, &s.image) {
        (Some(fingerprint), Some(src)) => match write_file(dir, id, IMG_EXT, src, op) {
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
        stored_at: op.now,
        used_at: op.now,
    };
    storage::write_json(&file(dir, id, META_EXT), &meta)
        .map_err(|err| io::Error::other(err.to_string()))?;
    Ok(bytes)
}

/// Entrée de `s` toujours en place : métadonnées valides, même empreinte, MP3 de la taille
/// enregistrée. Son SHA-256 a été vérifié par la lecture de cette préparation, verrou relâché
/// depuis : une altération à taille égale entre-temps n'est pas vue ici, mais la lecture
/// suivante revérifie le SHA-256 et ne sert jamais l'entrée altérée.
fn current_entry(dir: &Path, s: &Store) -> Option<EntryMeta> {
    let id = s.key.episode_id.as_str();
    read_meta(dir, id).filter(|meta| {
        meta.fingerprint == s.key.fingerprint
            && real_file_len(&file(dir, id, MP3_EXT)) == Some(meta.mp3.bytes)
    })
}

/// Remplace la pochette d'une entrée en place sans toucher à son MP3 (#489). Les métadonnées
/// cessent d'abord de citer l'ancienne pochette ; la nouvelle passe par un `.part`, puis les
/// métadonnées sont réécrites. En cas d'erreur ou d'annulation, seule la pochette est
/// supprimée : l'entrée reste servie sans pochette.
fn refresh_image(dir: &Path, s: &Store, mut meta: EntryMeta, op: &Op<'_>) -> io::Result<u64> {
    let id = s.key.episode_id.as_str();
    let meta_path = file(dir, id, META_EXT);
    let img = file(dir, id, IMG_EXT);
    let write_meta = |meta: &EntryMeta| {
        storage::write_json(&meta_path, meta).map_err(|err| io::Error::other(err.to_string()))
    };
    if meta.image.take().is_some() {
        write_meta(&meta)?;
    }
    let written = (|| {
        remove_if_exists(&img)?;
        let new = match (&s.key.image_fingerprint, &s.image) {
            (Some(fingerprint), Some(src)) => match write_file(dir, id, IMG_EXT, src, op) {
                Ok(file) => Some(ImageMeta {
                    fingerprint: fingerprint.clone(),
                    file,
                }),
                Err(err) if err.kind() == io::ErrorKind::NotFound => None,
                Err(err) => return Err(err),
            },
            _ => None,
        };
        meta.image = new;
        meta.used_at = op.now;
        write_meta(&meta)
    })();
    if let Err(err) = written {
        let _ = remove_if_exists(&img);
        let _ = remove_if_exists(&part(dir, id, IMG_EXT));
        return Err(err);
    }
    Ok(meta.mp3.bytes + meta.image.as_ref().map_or(0, |i| i.file.bytes))
}

/// Met un épisode en cache ; en cas d'erreur ou d'annulation, rien de l'entrée ne reste
/// (annulation demandée avant le début : l'ancienne entrée est gardée), sauf pour une
/// pochette seule remplacée ([`Store::image_only`]) : le MP3 reste.
fn store_one(dir: &Path, s: &Store, op: &Op<'_>) -> io::Result<u64> {
    if !valid_id(&s.key.episode_id) {
        return Err(io::Error::other("identifiant d'épisode invalide"));
    }
    op.cancel.check()?;
    if s.image_only {
        if let Some(meta) = current_entry(dir, s) {
            return refresh_image(dir, s, meta, op);
        }
    }
    let result = store_inner(dir, s, op);
    if result.is_err() {
        let _ = remove_entry(dir, &s.key.episode_id);
    }
    result
}

/// Octets de la pochette d'origine en cache pour `key`, lecture seule (aucune date d'usage
/// renouvelée, aucune entrée supprimée) : `None` si l'entrée, la pochette ou son empreinte
/// manque, si elle dépasse `limit` octets ou si elle est altérée (taille et SHA-256).
fn read_cached_image(dir: &Path, key: &EntryKey, limit: u64) -> Option<Vec<u8>> {
    let id = key.episode_id.as_str();
    if !valid_id(id) {
        return None;
    }
    let stored = read_meta(dir, id)?.image?;
    if key.image_fingerprint.as_deref() != Some(stored.fingerprint.as_str())
        || stored.file.bytes > limit
    {
        return None;
    }
    let path = file(dir, id, IMG_EXT);
    if !intact(&path, &stored.file, &Cancel::default()) {
        return None;
    }
    std::fs::read(path).ok()
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

/// Fichier d'une entrée : `<id>.mp3`, `<id>.img` ou `<id>.json` ; rend `<id>`.
fn entry_stem(name: &str) -> Option<&str> {
    let (stem, ext) = name.split_once('.')?;
    (valid_id(stem) && [MP3_EXT, IMG_EXT, META_EXT].contains(&ext)).then_some(stem)
}

/// Parcourt `<cache>/podcasts/*/mp3/` sans suivre de lien ni de jonction (voir
/// [`real_mp3_dir`]). Ce qui disparaît pendant le parcours est ignoré.
fn scan(cache_root: &Path) -> Survey {
    let mut survey = Survey::default();
    let podcasts = cache_root.join(CACHE_SUBDIR);
    if !is_real_dir(&podcasts) {
        return survey;
    }
    let Ok(sources) = std::fs::read_dir(&podcasts) else {
        return survey;
    };
    for source in sources.flatten() {
        let source_id = source.file_name().to_string_lossy().into_owned();
        let Ok(Some(dir)) = real_mp3_dir(cache_root, &source_id, false) else {
            continue;
        };
        let Ok(files) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut ids: HashSet<String> = HashSet::new();
        let mut junk: Vec<String> = Vec::new();
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().into_owned();
            match entry_stem(&name) {
                Some(stem) => {
                    ids.insert(stem.to_string());
                }
                // `.part`, temporaires, extension inconnue : restes.
                None => junk.push(name),
            }
        }
        for id in ids {
            match read_meta(&dir, &id) {
                Some(meta) => {
                    let size = |ext| real_file_len(&file(&dir, &id, ext)).unwrap_or(0);
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
    max_entry_bytes: u64,
    place: Arc<Placer>,
    cancel: Cancel,
}

/// Rangement d'un dossier de source : lecture ou nettoyage (rien n'est créé), écriture (les
/// dossiers manquants sont créés).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Read,
    Write,
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
            max_entry_bytes: MAX_AUDIO_BYTES,
            place: Arc::new(link_or_copy),
            cancel: Cancel::default(),
        }
    }

    /// Plafond de taille (tests).
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Taille au-delà de laquelle une entrée n'est plus servie (tests ; par défaut, la borne
    /// d'un épisode dans un pack).
    pub fn with_max_entry_bytes(mut self, max_entry_bytes: u64) -> Self {
        self.max_entry_bytes = max_entry_bytes;
        self
    }

    /// Fonction de placement (tests : injection de pannes).
    pub fn with_placer(mut self, place: Arc<Placer>) -> Self {
        self.place = place;
        self
    }

    /// Drapeau d'annulation de la tâche : hachage et copie s'arrêtent au bloc suivant.
    pub fn with_cancel(mut self, cancel: watch::Receiver<bool>) -> Self {
        self.cancel = Cancel::new(cancel);
        self
    }

    /// Exécute `job` sur le dossier de `source_id`, sous son verrou, si la source n'a pas été
    /// purgée depuis la lecture de sa génération et si son chemin ne passe par aucun lien
    /// ([`real_mp3_dir`]). `None` sinon (en lecture : aussi si le dossier n'existe pas).
    async fn under_lock<T: Send + 'static>(
        &self,
        source_id: &str,
        generation: u64,
        mode: Mode,
        job: impl FnOnce(&Path) -> T + Send + 'static,
    ) -> Option<T> {
        let guard = self.locks.lock(source_id, generation).await;
        let Some(guard) = guard else {
            tracing::info!("cache des MP3 : source {source_id} supprimée, cache ignoré");
            return None;
        };
        let (root, sid) = (self.cache_root.clone(), source_id.to_string());
        blocking(move || {
            let _guard = guard;
            match real_mp3_dir(&root, &sid, mode == Mode::Write) {
                Ok(Some(dir)) => Some(job(&dir)),
                Ok(None) => None,
                Err(err) => {
                    tracing::warn!("cache des MP3 de la source {sid} ignoré : {err}");
                    None
                }
            }
        })
        .await
        .ok()
        .flatten()
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
            let (place, cancel) = (self.place.clone(), self.cancel.clone());
            let max_entry_bytes = self.max_entry_bytes;
            let found = self
                .under_lock(&source_id, generation, Mode::Read, move |dir| {
                    let op = Op {
                        now,
                        place: &*place,
                        cancel: &cancel,
                        max_entry_bytes,
                    };
                    wanted
                        .iter()
                        .take_while(|_| !cancel.is_set())
                        .filter_map(|w| lookup_one(dir, w, &op))
                        .collect::<Vec<_>>()
                })
                .await
                .unwrap_or_default();
            hits.extend(found);
        }
        hits
    }

    /// Pochette d'origine en cache pour `key` (aperçu, #504), sans la placer ni renouveler
    /// l'entrée ; `None` si elle n'y est pas, est altérée ou dépasse `limit` octets.
    pub async fn cached_image(&self, key: &EntryKey, limit: u64) -> Option<Vec<u8>> {
        let generation = *self.generations.get(&key.source_id)?;
        let key = key.clone();
        self.under_lock(&key.source_id.clone(), generation, Mode::Read, move |dir| {
            read_cached_image(dir, &key, limit)
        })
        .await
        .flatten()
    }

    /// Met en cache les épisodes convertis ; rend le nombre d'entrées écrites.
    pub async fn store(&self, items: Vec<Store>, now: u64) -> usize {
        let mut stored = 0;
        for (source_id, items) in Self::by_source(items, |s| &s.key.source_id) {
            let Some(&generation) = self.generations.get(&source_id) else {
                continue;
            };
            let (place, cancel) = (self.place.clone(), self.cancel.clone());
            let max_entry_bytes = self.max_entry_bytes;
            let sid = source_id.clone();
            let (ok, failed) = self
                .under_lock(&source_id, generation, Mode::Write, move |dir| {
                    let op = Op {
                        now,
                        place: &*place,
                        cancel: &cancel,
                        max_entry_bytes,
                    };
                    let mut ok = 0usize;
                    let mut failed = 0usize;
                    for s in &items {
                        if cancel.is_set() {
                            // Tâche annulée : rien de plus n'est écrit (ni journalisé).
                            break;
                        }
                        match store_one(dir, s, &op) {
                            Ok(_) => ok += 1,
                            Err(_) if cancel.is_set() => break,
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
            self.under_lock(&source_id, generation, Mode::Read, move |dir| {
                remove_in(dir, &junk, &evicted);
            })
            .await;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests;
