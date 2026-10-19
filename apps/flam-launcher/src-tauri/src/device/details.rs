//! Titres et vignettes des histoires de la Flam pour l'écran « Ma Flam » (#276), en lecture
//! seule stricte : rien n'est écrit sur la Flam, aucune clé ne sort de ce module.
//!
//! - **Titre** : première ligne non vide du fichier `info` déchiffré (lecture bornée à
//!   [`MAX_INFO`]). Firmware 2.x : clés de `.mdf`. Firmware 1.x : couple carrier **déjà
//!   conservé** pour cette Flam, seulement si le consentement carrier est à jour ; aucune clé
//!   n'est demandée ni lue ailleurs (pas de `bt`). Un `info` illisible (mauvaise clé, fichier
//!   abîmé) donne `None` : l'interface se replie sur le titre de la bibliothèque, puis sur l'UUID
//!   court. Contrôle de lisibilité : celui du mode carrier ([`super::carrier::info_text`]).
//! - **Vignette** : `img/thumbnail.lif` de `str/<uuid>/` (ou `str.hidden/<uuid>/`), non
//!   chiffrée, au plus [`MAX_LIF_BYTES`], décodée par le décodeur LIF du launcher, réduite et
//!   réencodée en PNG, puis mise en cache sur le PC (`<cache>/thumbs/`, clé : empreinte de
//!   l'UUID, de la taille et de la date du fichier ; aucune donnée sensible). Absente ou
//!   invalide : `None`, l'interface affiche la vignette par défaut.
//!
//! # Performances et cohabitation
//!
//! Les titres sont demandés par lots d'au plus [`MAX_BATCH`] histoires, les vignettes une à
//! une : la liste s'affiche d'abord, les détails arrivent ensuite. Ces lectures ne prennent
//! **pas** le verrou d'appareil (elles ne doivent jamais faire refuser un transfert) : elles
//! s'arrêtent dès qu'une opération est en cours ([`super::lock::DEVICE_LOCK`]) et l'interface
//! réessaie plus tard. [`device_story_details_cancel`] interrompt le lot en cours (changement
//! de Flam, écran quitté).

use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::State;

use super::carrier::{
    consent_is_current, stored_consent, CarrierConsent, CarrierStore, DeviceId, HIDDEN_STORIES_DIR,
    INFO_FILE, MAX_INFO,
};
use super::crypto::{firmware_family, DeviceKeys, FirmwareFamily};
use super::detect::MDF_FILE;
use super::index::{StoryId, STORIES_DIR};
use super::lock::DEVICE_LOCK;
use super::transfer::{firmware_from_mdf, keys_from_mdf, read_mdf};
use crate::catalog::blocking;
use crate::settings::{Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::thumbnail::{self, ThumbCache, ThumbError};
use crate::AppResult;

/// Nombre maximal d'histoires par demande de titres.
pub const MAX_BATCH: usize = 100;
/// Taille maximale lue d'une vignette `thumbnail.lif` (mesuré sur une Flam 1.x : 0,7 à 22 Kio).
pub const MAX_LIF_BYTES: u64 = 1024 * 1024;
/// Côté maximal de la vignette produite (ligne de 3 rem, écrans à haute densité).
const THUMB_DIM: u32 = 96;
/// Vignette d'une histoire, relative à son dossier.
const LIF_PATH: [&str; 2] = ["img", "thumbnail.lif"];

/// Génération des demandes : [`device_story_details_cancel`] l'incrémente, ce qui interrompt
/// le lot de titres en cours.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Titre d'une histoire de la Flam (`None` : clé absente ou `info` illisible).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryTitle {
    pub uuid: String,
    pub title: Option<String>,
}

/// Réponse de `device_story_titles`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryTitles {
    /// Titres des histoires traitées, dans l'ordre de la demande (UUID invalides omis).
    pub titles: Vec<StoryTitle>,
    /// Lot interrompu (opération en cours sur la Flam, ou annulation) : les histoires absentes
    /// de `titles` sont à redemander plus tard.
    pub interrupted: bool,
}

/// Réponse de `device_story_thumbnail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryThumbnail {
    /// URL `data:image/png` de la vignette, `None` si absente ou invalide.
    pub thumb: Option<String>,
    /// Une opération est en cours sur la Flam : rien n'a été lu, à redemander plus tard.
    pub busy: bool,
}

/// Clés qui déchiffrent les titres de la Flam de `root`, sans rien demander à l'utilisateur :
/// 2.x, clés de `.mdf` ; 1.x, couple carrier conservé pour cette Flam si `consent` est à jour.
/// `None` dans tous les autres cas (aucune erreur : le titre se replie).
pub fn title_keys(
    root: &Path,
    consent: Option<&CarrierConsent>,
    store: &CarrierStore,
) -> Option<DeviceKeys> {
    let mdf = read_mdf(root).ok()?;
    let firmware = firmware_from_mdf(&mdf)?;
    match firmware_family(&firmware)? {
        FirmwareFamily::V2 => keys_from_mdf(&mdf).ok(),
        FirmwareFamily::V1 => {
            if !consent_is_current(consent) {
                return None;
            }
            let id = DeviceId::from_mdf(&mdf)?;
            if !store.has(&id) {
                return None;
            }
            let carrier = match store.load(&id) {
                Ok(carrier) => carrier?,
                Err(err) => {
                    // Le message ne contient aucune clé (voir `CarrierError`).
                    tracing::debug!("titres de la Flam : clés carrier illisibles : {err}");
                    return None;
                }
            };
            DeviceKeys::with_carrier(&firmware, &carrier).ok()
        }
    }
}

/// Dossier de l'histoire `id` : `str/<uuid>/`, sinon `str.hidden/<uuid>/` (vrais dossiers).
fn story_dir(root: &Path, id: &StoryId) -> Option<PathBuf> {
    [STORIES_DIR, HIDDEN_STORIES_DIR]
        .iter()
        .map(|sub| root.join(sub).join(id.as_str()))
        .find(|dir| fs::symlink_metadata(dir).is_ok_and(|m| m.is_dir()))
}

/// Ouvre `path` s'il s'agit d'un vrai fichier (ni lien, ni dossier) ; rend aussi ses
/// métadonnées.
fn open_regular(path: &Path) -> Option<(File, fs::Metadata)> {
    let meta = fs::symlink_metadata(path).ok()?;
    meta.is_file().then_some(())?;
    Some((File::open(path).ok()?, meta))
}

/// Lit au plus `max` octets de `file` ; plus long : `None`.
fn read_bounded(file: File, max: u64) -> Option<Vec<u8>> {
    let mut data = Vec::new();
    file.take(max + 1).read_to_end(&mut data).ok()?;
    (data.len() as u64 <= max).then_some(data)
}

/// Titre de l'histoire `id` avec `keys` : début de `info` (au plus [`MAX_INFO`] octets).
fn story_title(root: &Path, id: &StoryId, keys: &DeviceKeys) -> Option<String> {
    let (file, _) = open_regular(&story_dir(root, id)?.join(INFO_FILE))?;
    let mut info = zeroize::Zeroizing::new(Vec::new());
    file.take(MAX_INFO).read_to_end(&mut info).ok()?;
    keys.title_of_info(&info)
}

/// Titres des histoires `uuids` (au plus [`MAX_BATCH`] premiers) de la Flam de `root`.
/// `stop` est consulté avant chaque histoire : vrai, le lot s'arrête (`interrupted`). Sans clé,
/// aucun `info` n'est lu. Lecture seule.
pub fn read_titles(
    root: &Path,
    uuids: &[String],
    keys: Option<&DeviceKeys>,
    stop: &dyn Fn() -> bool,
) -> StoryTitles {
    let mut titles = Vec::new();
    for uuid in uuids.iter().take(MAX_BATCH) {
        if stop() {
            return StoryTitles {
                titles,
                interrupted: true,
            };
        }
        let Some(id) = StoryId::parse(uuid) else {
            continue;
        };
        let title = keys.and_then(|keys| story_title(root, &id, keys));
        titles.push(StoryTitle {
            uuid: id.to_string(),
            title,
        });
    }
    StoryTitles {
        titles,
        interrupted: false,
    }
}

/// Clé de cache d'une vignette : empreinte de l'UUID, de la taille et de la date du fichier
/// (aucune donnée sensible).
fn cache_key(id: &StoryId, meta: &fs::Metadata) -> u64 {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    thumbnail::fnv1a(&[
        b"flam-device-thumb/v1",
        id.as_str().as_bytes(),
        &meta.len().to_le_bytes(),
        &mtime.to_le_bytes(),
        &THUMB_DIM.to_le_bytes(),
    ])
}

/// Vignette de l'histoire `uuid` de la Flam de `root` (URL `data:` d'un PNG), depuis le cache
/// de `cache_dir` si possible. `None` : UUID invalide, vignette absente, vide, trop grosse ou
/// invalide. Lecture seule sur la Flam ; seul le cache du PC est écrit.
pub fn story_thumbnail(root: &Path, uuid: &str, cache_dir: &Path) -> Option<String> {
    let id = StoryId::parse(uuid)?;
    let dir = story_dir(root, &id)?;
    let path = LIF_PATH.iter().fold(dir, |p, part| p.join(part));
    let (file, meta) = open_regular(&path)?;
    if meta.len() == 0 || meta.len() > MAX_LIF_BYTES {
        return None;
    }
    ThumbCache::new(cache_dir)
        .get_or_make::<ThumbError>(cache_key(&id, &meta), THUMB_DIM, || {
            let lif = read_bounded(file, MAX_LIF_BYTES)
                .ok_or_else(|| ThumbError::Decode("vignette trop volumineuse".into()))?;
            Ok(telmi2flam::lif::decode(&lif)?)
        })
        .map_err(|err| tracing::debug!(%id, "vignette de la Flam illisible : {err}"))
        .ok()
}

/// La Flam est en cours d'écriture (transfert, retrait, éjection) : ne pas la lire.
fn device_busy() -> bool {
    DEVICE_LOCK.current().is_some()
}

/// Titres déchiffrés d'un lot d'histoires de la Flam de `root` (voir [`read_titles`]).
#[tauri::command]
pub async fn device_story_titles(
    config: State<'_, ConfigState>,
    root: String,
    uuids: Vec<String>,
) -> AppResult<StoryTitles> {
    let consent = stored_consent(&config)?;
    let generation = GENERATION.load(Ordering::SeqCst);
    blocking(move || {
        let root = Path::new(&root);
        let stop = || device_busy() || GENERATION.load(Ordering::SeqCst) != generation;
        if stop() || !root.join(MDF_FILE).is_file() {
            return StoryTitles {
                titles: Vec::new(),
                interrupted: stop(),
            };
        }
        let keys = title_keys(root, consent.as_ref(), &CarrierStore::app());
        read_titles(root, &uuids, keys.as_ref(), &stop)
    })
    .await
}

/// Vignette d'une histoire de la Flam de `root` (voir [`story_thumbnail`]).
#[tauri::command]
pub async fn device_story_thumbnail(
    config: State<'_, ConfigState>,
    root: String,
    uuid: String,
) -> AppResult<StoryThumbnail> {
    let cache_dir = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    blocking(move || {
        if device_busy() {
            return StoryThumbnail {
                thumb: None,
                busy: true,
            };
        }
        let root = Path::new(&root);
        let thumb = root
            .join(MDF_FILE)
            .is_file()
            .then(|| story_thumbnail(root, &uuid, &cache_dir))
            .flatten();
        StoryThumbnail { thumb, busy: false }
    })
    .await
}

/// Interrompt le lot de titres en cours (changement de Flam, écran quitté).
#[tauri::command]
pub async fn device_story_details_cancel() -> AppResult<()> {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests;
