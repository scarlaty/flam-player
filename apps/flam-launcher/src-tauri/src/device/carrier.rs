//! Mode « carrier » pour la Flam en firmware 1.x (#234).
//!
//! Le firmware 1.x n'accepte pas les clés lues dans `.mdf` (voir [`super::crypto`]). Il lit
//! en revanche une histoire chiffrée avec les clés d'une histoire **officielle du même compte**
//! déjà présente sur la Flam : son fichier `key` (32 octets, copié tel quel) et son fichier `bt`
//! (32 octets : clé AES-128 puis IV). Ce module :
//!
//! - liste les histoires officielles de la Flam qui portent ce matériel ([`list_candidates`]) ;
//! - lit le couple depuis une histoire de la Flam ([`read_device_carrier`]), depuis un fichier
//!   choisi par l'utilisateur ([`read_file_carrier`] : `bt` avec `key` à côté, ou archive zip),
//!   ou depuis un `bt` saisi en hexadécimal ([`find_by_bt`] : le `key` est celui de l'histoire
//!   de la Flam dont `info` se déchiffre avec ce `bt`, #271) ;
//! - vérifie que le couple correspond à la Flam **avant toute copie** ([`check_against_device`]) ;
//! - conserve le couple sur le PC, chiffré par DPAPI (portée utilisateur), lié à la Flam
//!   ([`CarrierStore`]) ;
//! - relit `info` après l'écriture ([`readback`]).
//!
//! # Règles
//!
//! - Rien n'est fait sans le consentement enregistré pour la version courante de
//!   l'avertissement ([`WARNING_TEXT`], [`WARNING_VERSION`]) : ni lecture de `bt`, ni copie.
//! - Les clés ne sortent jamais de ce module autrement que vers le chiffrement et le fichier
//!   `key` de l'histoire installée : jamais dans les journaux, les erreurs, l'IPC ni les exports.
//!   `Debug` les masque ; elles sont effacées de la mémoire à la destruction (`zeroize`).
//! - Aucune clé n'est fournie par le launcher ; aucune histoire officielle n'est copiée.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use zeroize::Zeroizing;

use super::crypto::{
    decrypt_blocks, firmware_family, FirmwareFamily, BLOCK_SIZE, KEY_FILE_LEN, MDF_SERIAL_OFFSET,
};
use super::index::{StoryId, STORIES_DIR};
use super::transfer::{firmware_from_mdf, read_mdf, TransferError};
use crate::catalog::blocking;
use crate::settings::SettingsStore;
use crate::storage::ConfigState;
use crate::{AppError, AppResult};

/// Texte de l'avertissement, affiché tel quel avant la première utilisation (validé par le
/// mainteneur, versionné dans `docs/flam-carrier.md`).
pub const WARNING_TEXT: &str = "Fonction expérimentale, non officielle. Pour installer une histoire sur une Flam en firmware 1.x, flam-launcher réutilise les clés d'une histoire officielle déjà présente sur cette Flam et achetée sur votre compte. Aucune clé n'est fournie par flam-launcher, aucune histoire officielle n'est copiée ni exportée. Si cette histoire officielle est retirée de la Flam ou de votre compte, les histoires installées ainsi peuvent devenir illisibles. Usage strictement personnel, à vos risques ; fonction ni approuvée ni prise en charge par Lunii.";
/// Version de [`WARNING_TEXT`] : tout changement du texte l'incrémente et redemande le
/// consentement.
pub const WARNING_VERSION: u32 = 1;

/// Nom du fichier du couple dans une histoire officielle.
pub const BT_FILE: &str = "bt";
/// Nom du fichier d'autorisation d'une histoire.
pub const KEY_FILE: &str = "key";
/// Fichier `info` (titre chiffré) d'une histoire.
pub const INFO_FILE: &str = "info";
/// Dossier des histoires masquées.
pub(crate) const HIDDEN_STORIES_DIR: &str = "str.hidden";
/// Taille du fichier `bt` utile : clé AES (16) puis IV (16).
pub const BT_LEN: usize = 32;
/// Taille maximale lue de `bt` et `key` (au-delà : refusé, ce n'est pas un couple).
pub(crate) const MAX_SMALL_FILE: u64 = 4096;
/// Taille maximale déchiffrée de `info` pour le contrôle.
pub(crate) const MAX_INFO: u64 = 4096;
/// Taille maximale d'une archive choisie comme source.
const MAX_ARCHIVE: u64 = 64 * 1024 * 1024;
/// Nombre maximal de dossiers d'histoires examinés.
const MAX_SCANNED: usize = 5000;
/// Longueur maximale acceptée d'une clé saisie (espaces compris).
const MAX_ENTERED_CHARS: usize = 1024;
/// Longueur maximale d'un titre affiché.
const MAX_TITLE_CHARS: usize = 80;
/// Sous-dossier du dossier de données où le couple est conservé (chiffré par DPAPI).
pub const STORE_SUBDIR: &str = "carrier-keys";
/// Extension des fichiers conservés.
const STORE_EXT: &str = "dpapi";
/// En-tête du clair conservé (avant chiffrement DPAPI).
const STORE_MAGIC: &[u8; 8] = b"FLCARR01";
/// Entropie complémentaire de DPAPI (liée à l'application et à la Flam).
const STORE_ENTROPY: &[u8] = b"flam-launcher/carrier/v1/";

/// Erreur du mode carrier. Les messages ne contiennent jamais de clé.
#[derive(Debug, thiserror::Error)]
pub enum CarrierError {
    #[error("acceptez d'abord l'avertissement du mode carrier (Flam 1.x) : rien n'a été écrit")]
    ConsentRequired,
    #[error("Flam 1.x : choisissez l'histoire officielle dont les clés serviront (mode carrier) ; rien n'a été écrit")]
    CarrierRequired,
    #[error("aucune clé conservée pour cette Flam : choisissez une histoire officielle ou un fichier ; rien n'a été écrit")]
    NoStoredKeys,
    #[error("source des clés invalide : {0} ; rien n'a été écrit")]
    InvalidSource(String),
    #[error("ces clés ne correspondent à aucune histoire officielle présente sur cette Flam (couple invalide ou d'un autre compte) : rien n'a été écrit")]
    Mismatch,
    #[error(
        "cette clé ne correspond à aucune histoire officielle de cette Flam : rien n'a été écrit"
    )]
    EnteredMismatch,
    #[error("confirmez le titre de l'histoire reconnue par cette clé avant d'envoyer : rien n'a été écrit")]
    EnteredUnconfirmed,
    #[error("transfert annulé : la Flam est inchangée")]
    Cancelled,
    #[error("clés conservées : {0}")]
    Store(String),
    #[error("{0}")]
    Io(#[from] io::Error),
}

/// Couple d'une histoire officielle : fichier `key` et clé et IV de `bt`.
pub struct CarrierKeys {
    key_file: Zeroizing<[u8; KEY_FILE_LEN]>,
    story_key: Zeroizing<[u8; 16]>,
    story_iv: Zeroizing<[u8; 16]>,
}

impl CarrierKeys {
    /// Couple à partir du contenu de `bt` et de `key` (au moins 32 octets chacun : seuls les
    /// 32 premiers servent).
    pub fn from_parts(bt: &[u8], key_file: &[u8]) -> Result<Self, CarrierError> {
        if bt.len() < BT_LEN {
            return Err(CarrierError::InvalidSource(format!(
                "fichier {BT_FILE} de {} octets ({BT_LEN} attendus)",
                bt.len()
            )));
        }
        if key_file.len() < KEY_FILE_LEN {
            return Err(CarrierError::InvalidSource(format!(
                "fichier {KEY_FILE} de {} octets ({KEY_FILE_LEN} attendus)",
                key_file.len()
            )));
        }
        let mut keys = Self {
            key_file: Zeroizing::new([0; KEY_FILE_LEN]),
            story_key: Zeroizing::new([0; 16]),
            story_iv: Zeroizing::new([0; 16]),
        };
        keys.key_file.copy_from_slice(&key_file[..KEY_FILE_LEN]);
        keys.story_key.copy_from_slice(&bt[..16]);
        keys.story_iv.copy_from_slice(&bt[16..BT_LEN]);
        Ok(keys)
    }

    pub(crate) fn key_file(&self) -> &[u8; KEY_FILE_LEN] {
        &self.key_file
    }

    pub(crate) fn story_key(&self) -> &[u8; 16] {
        &self.story_key
    }

    pub(crate) fn story_iv(&self) -> &[u8; 16] {
        &self.story_iv
    }

    /// Clair de `info` (titre…) déchiffré avec ce couple, s'il est lisible.
    fn decrypt_info(&self, data: &[u8]) -> Option<String> {
        info_text(&self.story_key, &self.story_iv, data)
    }

    /// Même `bt`, avec le fichier `key` d'une autre histoire de la Flam.
    fn with_key_file(&self, key_file: &[u8]) -> Result<Self, CarrierError> {
        let mut bt = Zeroizing::new([0u8; BT_LEN]);
        bt[..16].copy_from_slice(&self.story_key[..]);
        bt[16..].copy_from_slice(&self.story_iv[..]);
        Self::from_parts(&bt[..], key_file)
    }
}

/// Clair de `info` déchiffré avec la clé et l'IV d'un `bt`, s'il est lisible.
pub(crate) fn info_text(key: &[u8; 16], iv: &[u8; 16], data: &[u8]) -> Option<String> {
    let len = data.len().min(MAX_INFO as usize) / BLOCK_SIZE * BLOCK_SIZE;
    if len == 0 {
        return None;
    }
    let mut plain = Zeroizing::new(data[..len].to_vec());
    decrypt_blocks(key, iv, &mut plain).ok()?;
    readable_text(&plain).filter(|text| first_block_clean(text))
}

/// Vrai si le début de `info` est déjà du texte lisible, sans déchiffrement (`info` en clair,
/// histoire au format d'un autre outil).
pub(crate) fn info_is_plain_text(info: &[u8]) -> bool {
    readable_text(&info[..info.len().min(MAX_INFO as usize)]).is_some()
}

/// Contrôle strict du premier bloc du clair (16 octets). En AES-CBC, l'IV n'agit que sur ce
/// bloc : un IV faux (faute de frappe dans la seconde moitié d'un `bt` saisi) y altère des
/// octets sans toucher au reste. Le premier bloc doit donc être de l'UTF-8 strict sans
/// caractère de remplacement ni caractère de contrôle autre que `\n` (ou `\r` d'un `\r\n`).
/// Ce contrôle réduit les IV faux acceptés sans les exclure : voir `docs/flam-carrier.md`.
fn first_block_clean(text: &str) -> bool {
    text.char_indices()
        .take_while(|(at, _)| *at < BLOCK_SIZE)
        .all(|(at, c)| match c {
            '\n' => true,
            '\r' => text[at + 1..].starts_with('\n'),
            '\u{FFFD}' => false,
            c => !c.is_control(),
        })
}

/// `bt` saisi par l'utilisateur : 64 caractères hexadécimaux, espaces et majuscules tolérés.
/// Le texte est effacé de la mémoire à la destruction et masqué par `Debug` ; il n'apparaît
/// dans aucun message d'erreur.
#[derive(Clone, PartialEq, Eq)]
pub struct EnteredBt(Zeroizing<String>);

impl EnteredBt {
    pub fn new(text: impl Into<String>) -> Self {
        Self(Zeroizing::new(text.into()))
    }

    /// Octets du `bt` saisi. Refus (sans reprendre le texte saisi) : caractère non
    /// hexadécimal, ou nombre de chiffres différent de 64.
    pub fn parse(&self) -> Result<Zeroizing<[u8; BT_LEN]>, CarrierError> {
        let bad = |m: String| CarrierError::InvalidSource(format!("clé saisie : {m}"));
        if self.0.len() > MAX_ENTERED_CHARS {
            return Err(bad("texte trop long".into()));
        }
        let mut bt = Zeroizing::new([0u8; BT_LEN]);
        let mut digits = 0usize;
        for c in self.0.chars().filter(|c| !c.is_whitespace()) {
            let Some(value) = c.to_digit(16) else {
                return Err(bad(
                    "seuls les chiffres 0 à 9 et les lettres a à f sont acceptés".into(),
                ));
            };
            if let Some(byte) = bt.get_mut(digits / 2) {
                // `value` < 16 : la conversion est exacte.
                *byte = (*byte << 4) | value as u8;
            }
            digits += 1;
        }
        if digits != BT_LEN * 2 {
            return Err(bad(format!(
                "{digits} caractères hexadécimaux ({} attendus)",
                BT_LEN * 2
            )));
        }
        Ok(bt)
    }
}

impl std::fmt::Debug for EnteredBt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EnteredBt { <masqué> }")
    }
}

impl<'de> Deserialize<'de> for EnteredBt {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d).map(Self::new)
    }
}

impl std::fmt::Debug for CarrierKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CarrierKeys { <masqué> }")
    }
}

/// Texte lisible d'un clair : UTF-8 strict (octets nuls finaux retirés ; un caractère coupé
/// par la limite de lecture est toléré à la fin), au moins 4 caractères, dont au moins 90 %
/// imprimables ou fins de ligne. Une mauvaise clé donne du bruit, refusé.
fn readable_text(plain: &[u8]) -> Option<String> {
    let end = plain.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let bytes = &plain[..end];
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(e) if e.error_len().is_none() => std::str::from_utf8(&bytes[..e.valid_up_to()]).ok()?,
        Err(_) => return None,
    };
    let total = text.chars().count();
    if total < 4 {
        return None;
    }
    let printable = text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .count();
    (printable * 10 >= total * 9).then(|| text.to_string())
}

/// Première ligne non vide, sans caractère de contrôle, tronquée.
pub(crate) fn title_of(text: &str) -> String {
    text.lines()
        .map(|l| l.chars().filter(|c| !c.is_control()).collect::<String>())
        .map(|l| l.trim().to_string())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// Lit au plus `max` octets de `path` ; plus long : `None` (ce n'est pas le fichier attendu).
pub(crate) fn read_small(path: &Path, max: u64) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    let file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut data = Zeroizing::new(Vec::new());
    file.take(max + 1).read_to_end(&mut data)?;
    Ok((data.len() as u64 <= max).then_some(data))
}

/// Début de `info`, lu pour le contrôle (borné).
pub(crate) fn read_info(dir: &Path) -> io::Result<Option<Vec<u8>>> {
    let file = match File::open(dir.join(INFO_FILE)) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    let mut data = Vec::new();
    file.take(MAX_INFO).read_to_end(&mut data)?;
    Ok(Some(data))
}

/// Dossiers d'histoires de `str/` puis `str.hidden/` (nom au format UUID), bornés.
pub(crate) fn story_dirs(root: &Path) -> Vec<(StoryId, PathBuf, bool)> {
    let mut out = Vec::new();
    for (sub, hidden) in [(STORIES_DIR, false), (HIDDEN_STORIES_DIR, true)] {
        let Ok(entries) = fs::read_dir(root.join(sub)) else {
            continue;
        };
        let mut found: Vec<_> = entries
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|e| {
                let id = e.file_name().to_str().and_then(StoryId::parse)?;
                Some((id, e.path(), hidden))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend(found);
    }
    out.truncate(MAX_SCANNED);
    out
}

/// Couple d'un dossier d'histoire (`bt` et `key`), s'il y est et a la bonne taille.
fn carrier_in_dir(dir: &Path) -> io::Result<Option<CarrierKeys>> {
    let Some(bt) = read_small(&dir.join(BT_FILE), MAX_SMALL_FILE)? else {
        return Ok(None);
    };
    let Some(key) = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE)? else {
        return Ok(None);
    };
    Ok(CarrierKeys::from_parts(&bt, &key).ok())
}

/// Histoire officielle de la Flam utilisable comme carrier (sans aucune clé).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CarrierCandidate {
    pub uuid: String,
    /// Titre déchiffré de `info` (première ligne).
    pub title: String,
    /// Histoire masquée (`str.hidden/`).
    pub hidden: bool,
    /// Histoire sans `bt` : son `info` se déchiffre avec le `bt` conservé pour cette Flam.
    pub with_stored: bool,
}

/// Couple d'un dossier d'histoire sans `bt` : le `bt` conservé pour cette Flam, s'il déchiffre
/// son `info`, avec le `key` du dossier. Rend aussi le clair de `info`.
fn stored_carrier_for_dir(dir: &Path, stored: &CarrierKeys) -> Option<(CarrierKeys, String)> {
    let info = read_info(dir).ok()??;
    let text = stored.decrypt_info(&info)?;
    let key = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE).ok()??;
    Some((stored.with_key_file(&key).ok()?, text))
}

/// Histoires de la Flam utilisables comme carrier : celles qui portent un couple (`bt` et
/// `key`) dont `bt` déchiffre leur propre `info` en texte lisible, et, avec `stored` (couple
/// conservé pour cette Flam), celles sans `bt` dont `info` se déchiffre avec le `bt` conservé.
/// Lecture seule. Les clés ne quittent pas cette fonction.
pub fn list_candidates(root: &Path, stored: Option<&CarrierKeys>) -> Vec<CarrierCandidate> {
    let mut out = Vec::new();
    for (id, dir, hidden) in story_dirs(root) {
        let found = match carrier_in_dir(&dir) {
            Ok(Some(keys)) => read_info(&dir)
                .ok()
                .flatten()
                .and_then(|info| keys.decrypt_info(&info))
                .map(|text| (text, false)),
            Ok(None) => stored
                .and_then(|stored| stored_carrier_for_dir(&dir, stored))
                .map(|(_, text)| (text, true)),
            Err(_) => None,
        };
        if let Some((text, with_stored)) = found {
            out.push(CarrierCandidate {
                uuid: id.to_string(),
                title: title_of(&text),
                hidden,
                with_stored,
            });
        }
    }
    out
}

/// Couple de l'histoire `uuid` de la Flam (`str/` ou `str.hidden/`) : ses fichiers `bt` et
/// `key`, ou, sans `bt`, le `bt` conservé (`stored`, appelé seulement dans ce cas) s'il
/// déchiffre son `info`, avec son `key`.
pub fn read_device_carrier(
    root: &Path,
    uuid: &str,
    stored: impl FnOnce() -> Option<CarrierKeys>,
) -> Result<CarrierKeys, CarrierError> {
    let id = StoryId::parse(uuid)
        .ok_or_else(|| CarrierError::InvalidSource("identifiant d'histoire invalide".into()))?;
    for sub in [STORIES_DIR, HIDDEN_STORIES_DIR] {
        let dir = root.join(sub).join(id.as_str());
        if dir.is_dir() {
            if let Some(keys) = carrier_in_dir(&dir)? {
                return Ok(keys);
            }
            return stored()
                .and_then(|stored| stored_carrier_for_dir(&dir, &stored))
                .map(|(keys, _)| keys)
                .ok_or_else(|| {
                    CarrierError::InvalidSource(format!(
                        "l'histoire {id} n'a pas de fichiers {BT_FILE} et {KEY_FILE} utilisables"
                    ))
                });
        }
    }
    Err(CarrierError::InvalidSource(format!(
        "l'histoire {id} n'est pas sur la Flam"
    )))
}

/// Couple lu dans un fichier choisi par l'utilisateur : une archive zip contenant un seul
/// `bt` et le `key` du même dossier, ou un fichier `bt` avec `key` dans le même dossier
/// (copie d'un dossier d'histoire officielle).
pub fn read_file_carrier(path: &Path) -> Result<CarrierKeys, CarrierError> {
    // Ni périphérique, ni tube, ni dossier : seul un fichier ordinaire est ouvert.
    // (Une erreur de lecture des métadonnées est laissée aux ouvertures qui suivent.)
    if fs::metadata(path).is_ok_and(|meta| !meta.is_file()) {
        return Err(CarrierError::InvalidSource(
            "le chemin choisi n'est pas un fichier".into(),
        ));
    }
    let mut head = [0u8; 4];
    let is_zip = File::open(path)
        .and_then(|mut f| f.read_exact(&mut head))
        .is_ok()
        && head == *b"PK\x03\x04";
    if is_zip {
        return read_zip_carrier(path);
    }
    let bt = read_small(path, MAX_SMALL_FILE)?.ok_or_else(|| {
        CarrierError::InvalidSource("fichier introuvable ou trop grand pour être un bt".into())
    })?;
    let dir = path
        .parent()
        .ok_or_else(|| CarrierError::InvalidSource("fichier sans dossier".into()))?;
    let key = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE)?.ok_or_else(|| {
        CarrierError::InvalidSource(format!(
            "fichier {KEY_FILE} absent du dossier du fichier choisi"
        ))
    })?;
    CarrierKeys::from_parts(&bt, &key)
}

fn read_zip_carrier(path: &Path) -> Result<CarrierKeys, CarrierError> {
    let bad = |m: &str| CarrierError::InvalidSource(m.to_string());
    if fs::metadata(path)?.len() > MAX_ARCHIVE {
        return Err(bad("archive trop volumineuse"));
    }
    let mut zip = zip::ZipArchive::new(io::BufReader::new(File::open(path)?))
        .map_err(|_| bad("archive zip illisible"))?;
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_string();
    let bts: Vec<&String> = names.iter().filter(|n| base(n) == BT_FILE).collect();
    let [bt_name] = bts.as_slice() else {
        return Err(bad("l'archive doit contenir exactement un fichier bt"));
    };
    let folder = &bt_name[..bt_name.len() - BT_FILE.len()];
    let key_name = format!("{folder}{KEY_FILE}");
    let mut read = |name: &str| -> Result<Zeroizing<Vec<u8>>, CarrierError> {
        let entry = zip
            .by_name(name)
            .map_err(|_| bad("fichier key absent à côté de bt dans l'archive"))?;
        let mut data = Zeroizing::new(Vec::new());
        entry
            .take(MAX_SMALL_FILE + 1)
            .read_to_end(&mut data)
            .map_err(|_| bad("archive zip illisible"))?;
        if data.len() as u64 > MAX_SMALL_FILE {
            return Err(bad("fichier trop grand dans l'archive"));
        }
        Ok(data)
    };
    let bt = read(bt_name)?;
    let key = read(&key_name)?;
    CarrierKeys::from_parts(&bt, &key)
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), CarrierError> {
    if cancel.load(Ordering::Relaxed) {
        Err(CarrierError::Cancelled)
    } else {
        Ok(())
    }
}

/// Couple d'un `bt` saisi : voir [`find_story_by_bt`].
pub fn find_by_bt(
    root: &Path,
    bt: &[u8; BT_LEN],
    cancel: &AtomicBool,
) -> Result<CarrierKeys, CarrierError> {
    find_story_by_bt(root, bt, cancel).map(|found| found.keys)
}

/// Histoire de la Flam reconnue par un `bt` saisi.
pub struct FoundStory {
    pub keys: CarrierKeys,
    pub id: StoryId,
    pub hidden: bool,
    /// Titre déchiffré (première ligne de `info`).
    pub title: String,
}

/// Parcourt les histoires de la Flam (`str/` puis `str.hidden/`, bornées) et retient la
/// première dont `info` se déchiffre avec ce `bt` en texte lisible ; son fichier `key`
/// complète le couple. Aucune : [`CarrierError::EnteredMismatch`]. Lecture seule ;
/// l'annulation est vérifiée à chaque histoire.
pub fn find_story_by_bt(
    root: &Path,
    bt: &[u8; BT_LEN],
    cancel: &AtomicBool,
) -> Result<FoundStory, CarrierError> {
    let mut story_key = Zeroizing::new([0u8; 16]);
    let mut story_iv = Zeroizing::new([0u8; 16]);
    story_key.copy_from_slice(&bt[..16]);
    story_iv.copy_from_slice(&bt[16..]);
    for (id, dir, hidden) in story_dirs(root) {
        check_cancel(cancel)?;
        let Ok(Some(info)) = read_info(&dir) else {
            continue;
        };
        let Some(text) = info_text(&story_key, &story_iv, &info) else {
            continue;
        };
        let Ok(Some(key)) = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE) else {
            continue;
        };
        if let Ok(keys) = CarrierKeys::from_parts(&bt[..], &key) {
            return Ok(FoundStory {
                keys,
                id,
                hidden,
                title: title_of(&text),
            });
        }
    }
    check_cancel(cancel)?;
    Err(CarrierError::EnteredMismatch)
}

/// Vérifie, **avant toute copie**, que le couple correspond à la Flam : une histoire de la Flam
/// a le même fichier `key` et son `info` se déchiffre avec ce couple en texte lisible. Rend
/// l'histoire trouvée ; sinon [`CarrierError::Mismatch`]. Lecture seule ; l'annulation est
/// vérifiée à chaque histoire.
pub fn check_against_device(
    root: &Path,
    keys: &CarrierKeys,
    cancel: &AtomicBool,
) -> Result<StoryId, CarrierError> {
    for (id, dir, _) in story_dirs(root) {
        check_cancel(cancel)?;
        let Ok(Some(key)) = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE) else {
            continue;
        };
        if key.len() < KEY_FILE_LEN || key[..KEY_FILE_LEN] != keys.key_file[..] {
            continue;
        }
        let Ok(Some(info)) = read_info(&dir) else {
            continue;
        };
        if keys.decrypt_info(&info).is_some() {
            return Ok(id);
        }
    }
    Err(CarrierError::Mismatch)
}

/// Relecture après écriture : `info` de l'histoire installée se déchiffre avec le couple.
pub fn readback(root: &Path, id: &StoryId, keys: &CarrierKeys) -> bool {
    let dir = root.join(STORIES_DIR).join(id.as_str());
    let key_ok = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE)
        .ok()
        .flatten()
        .is_some_and(|k| k.len() == KEY_FILE_LEN && k[..] == keys.key_file[..]);
    let info_ok = read_info(&dir)
        .ok()
        .flatten()
        .is_some_and(|info| keys.decrypt_info(&info).is_some());
    key_ok && info_ok
}

// --- Consentement -----------------------------------------------------------------------------

/// Consentement à l'avertissement, enregistré dans les réglages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CarrierConsent {
    /// Version de l'avertissement acceptée ([`WARNING_VERSION`]).
    pub text_version: u32,
    /// Date UTC ISO 8601 de l'acceptation.
    pub accepted_at: String,
}

/// Le consentement couvre-t-il l'avertissement courant ?
pub fn consent_is_current(consent: Option<&CarrierConsent>) -> bool {
    consent.is_some_and(|c| c.text_version == WARNING_VERSION && !c.accepted_at.is_empty())
}

// --- Conservation chiffrée -----------------------------------------------------------------

/// Identifiant d'une Flam pour la conservation : empreinte du champ du numéro de série de
/// `.mdf` (jamais le numéro lui-même). `None` si `.mdf` est trop court.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceId([u8; 16]);

impl DeviceId {
    pub fn from_mdf(mdf: &[u8]) -> Option<Self> {
        let field = mdf.get(MDF_SERIAL_OFFSET..MDF_SERIAL_OFFSET + 24)?;
        let mut hasher = Sha256::new();
        hasher.update(b"flam-launcher/device-id/v1/");
        hasher.update(field);
        let digest = hasher.finalize();
        let mut id = [0u8; 16];
        id.copy_from_slice(&digest[..16]);
        Some(Self(id))
    }

    fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

impl std::fmt::Debug for DeviceId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceId { <masqué> }")
    }
}

/// Chiffrement des données conservées (DPAPI en production, injecté dans les tests).
pub trait Protector: Send + Sync {
    fn protect(&self, plain: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>>;
    fn unprotect(&self, blob: &[u8], entropy: &[u8]) -> io::Result<Zeroizing<Vec<u8>>>;
}

/// DPAPI de Windows, portée utilisateur (`CryptProtectData` sans
/// `CRYPTPROTECT_LOCAL_MACHINE`) : seul ce compte Windows peut déchiffrer.
pub struct Dpapi;

#[cfg(windows)]
impl Protector for Dpapi {
    fn protect(&self, plain: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
        dpapi::protect(plain, entropy)
    }
    fn unprotect(&self, blob: &[u8], entropy: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        dpapi::unprotect(blob, entropy)
    }
}

#[cfg(not(windows))]
impl Protector for Dpapi {
    fn protect(&self, _: &[u8], _: &[u8]) -> io::Result<Vec<u8>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "conservation chiffrée disponible sous Windows seulement",
        ))
    }
    fn unprotect(&self, _: &[u8], _: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "conservation chiffrée disponible sous Windows seulement",
        ))
    }
}

#[cfg(windows)]
mod dpapi {
    use std::io;
    use std::ptr::{null, null_mut};

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    use zeroize::Zeroizing;

    fn blob(data: &[u8]) -> io::Result<CRYPT_INTEGER_BLOB> {
        Ok(CRYPT_INTEGER_BLOB {
            cbData: u32::try_from(data.len()).map_err(io::Error::other)?,
            pbData: data.as_ptr().cast_mut(),
        })
    }

    /// Copie le tampon alloué par DPAPI, l'efface puis le libère.
    ///
    /// # Safety
    /// `out` vient d'un appel DPAPI réussi (tampon `LocalAlloc` de `cbData` octets).
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Zeroizing<Vec<u8>> {
        // SAFETY : tampon valide de `cbData` octets, propriété de l'appelant (contrat ci-dessus).
        let data = unsafe {
            let slice = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
            let copy = Zeroizing::new(slice.to_vec());
            for byte in slice.iter_mut() {
                std::ptr::write_volatile(byte, 0);
            }
            LocalFree(out.pbData.cast());
            copy
        };
        data
    }

    pub fn protect(plain: &[u8], entropy: &[u8]) -> io::Result<Vec<u8>> {
        let input = blob(plain)?;
        let extra = blob(entropy)?;
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };
        // SAFETY : pointeurs valides pendant l'appel ; aucune invite (UI_FORBIDDEN).
        let ok = unsafe {
            CryptProtectData(
                &input,
                null(),
                &extra,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY : appel réussi.
        Ok(unsafe { take(out) }.to_vec())
    }

    pub fn unprotect(blob_in: &[u8], entropy: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        let input = blob(blob_in)?;
        let extra = blob(entropy)?;
        let mut out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: null_mut(),
        };
        // SAFETY : pointeurs valides pendant l'appel ; aucune invite (UI_FORBIDDEN).
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                null_mut(),
                &extra,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY : appel réussi.
        Ok(unsafe { take(out) })
    }
}

/// Couples conservés sur le PC : un fichier par Flam dans `<données>/carrier-keys/`, chiffré
/// par [`Protector`] (DPAPI). Le nom du fichier est l'empreinte [`DeviceId`] ; le clair
/// contient un en-tête, l'identifiant de la Flam, `key` puis `bt`.
pub struct CarrierStore {
    dir: PathBuf,
    protector: Box<dyn Protector>,
}

impl CarrierStore {
    pub fn new(dir: PathBuf, protector: Box<dyn Protector>) -> Self {
        Self { dir, protector }
    }

    /// Conservation de l'application (dossier de données, DPAPI).
    pub fn app() -> Self {
        Self::new(
            crate::storage::data_dir().join(STORE_SUBDIR),
            Box::new(Dpapi),
        )
    }

    fn path(&self, id: &DeviceId) -> PathBuf {
        self.dir.join(format!("{}.{STORE_EXT}", id.hex()))
    }

    fn entropy(id: &DeviceId) -> Vec<u8> {
        [STORE_ENTROPY, &id.0[..]].concat()
    }

    /// Un couple est-il conservé pour cette Flam ?
    pub fn has(&self, id: &DeviceId) -> bool {
        self.path(id).is_file()
    }

    /// Conserve le couple de cette Flam (remplace le précédent, écriture atomique).
    pub fn save(&self, id: &DeviceId, keys: &CarrierKeys) -> Result<(), CarrierError> {
        let mut plain = Zeroizing::new(Vec::with_capacity(8 + 16 + KEY_FILE_LEN + BT_LEN));
        plain.extend_from_slice(STORE_MAGIC);
        plain.extend_from_slice(&id.0);
        plain.extend_from_slice(&keys.key_file[..]);
        plain.extend_from_slice(&keys.story_key[..]);
        plain.extend_from_slice(&keys.story_iv[..]);
        let blob = self
            .protector
            .protect(&plain, &Self::entropy(id))
            .map_err(|e| CarrierError::Store(format!("chiffrement impossible ({e})")))?;
        crate::storage::write_atomic(&self.path(id), &blob)
            .map_err(|e| CarrierError::Store(format!("écriture impossible ({e})")))?;
        tracing::info!("clés carrier conservées (chiffrées) pour cette Flam");
        Ok(())
    }

    /// Couple conservé pour cette Flam (`None` : aucun).
    pub fn load(&self, id: &DeviceId) -> Result<Option<CarrierKeys>, CarrierError> {
        let blob = match fs::read(self.path(id)) {
            Ok(b) => b,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(CarrierError::Store(format!("lecture impossible ({e})"))),
        };
        let unreadable = || {
            CarrierError::Store(
                "fichier illisible (autre compte Windows ou fichier abîmé) : oubliez les clés puis choisissez à nouveau".into(),
            )
        };
        let plain = self
            .protector
            .unprotect(&blob, &Self::entropy(id))
            .map_err(|_| unreadable())?;
        let expected = 8 + 16 + KEY_FILE_LEN + BT_LEN;
        if plain.len() != expected || &plain[..8] != STORE_MAGIC || plain[8..24] != id.0 {
            return Err(unreadable());
        }
        CarrierKeys::from_parts(&plain[24 + KEY_FILE_LEN..], &plain[24..24 + KEY_FILE_LEN])
            .map(Some)
            .map_err(|_| unreadable())
    }

    /// Oublie le couple de cette Flam, ou tous (`None`). Rend le nombre de fichiers supprimés.
    pub fn forget(&self, id: Option<&DeviceId>) -> Result<usize, CarrierError> {
        let store = |e: io::Error| CarrierError::Store(format!("suppression impossible ({e})"));
        let paths: Vec<PathBuf> = match id {
            Some(id) => vec![self.path(id)],
            None => match fs::read_dir(&self.dir) {
                Ok(entries) => entries
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|x| x == STORE_EXT))
                    .collect(),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(store(e)),
            },
        };
        let mut removed = 0;
        for path in paths {
            match fs::remove_file(&path) {
                Ok(()) => removed += 1,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(store(e)),
            }
        }
        if removed > 0 {
            tracing::info!(fichiers = removed, "clés carrier oubliées");
        }
        Ok(removed)
    }
}

// --- Choix de la source ---------------------------------------------------------------------

/// Source du couple choisie dans l'interface.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CarrierChoice {
    /// Couple conservé pour cette Flam.
    Stored,
    /// Histoire officielle de la Flam.
    Device { uuid: String },
    /// Fichier choisi par l'utilisateur.
    File { path: String },
    /// `bt` saisi en hexadécimal ; le `key` est celui de l'histoire de la Flam qu'il déchiffre.
    /// `confirmed` : l'utilisateur a confirmé le titre de l'histoire reconnue
    /// ([`preview_entered`]) ; sans cela, refus.
    Entered {
        bt: EnteredBt,
        #[serde(default)]
        confirmed: bool,
    },
}

/// Demande d'envoi en mode carrier.
pub struct CarrierRequest<'a> {
    /// Consentement enregistré (réglages).
    pub consent: Option<&'a CarrierConsent>,
    pub choice: CarrierChoice,
    /// Conserver le couple sur le PC (chiffré) après la vérification.
    pub remember: bool,
    pub store: &'a CarrierStore,
}

/// Résout et vérifie le couple d'une demande, sans rien écrire sur la Flam : consentement,
/// lecture de la source, contrôle de cohérence avec la Flam, puis conservation si demandée.
/// `cancel` interrompt le parcours des histoires ([`CarrierError::Cancelled`]).
pub fn resolve(
    root: &Path,
    mdf: &[u8],
    request: &CarrierRequest<'_>,
    cancel: &AtomicBool,
) -> Result<Resolved, CarrierError> {
    if !consent_is_current(request.consent) {
        return Err(CarrierError::ConsentRequired);
    }
    if matches!(
        request.choice,
        CarrierChoice::Entered {
            confirmed: false,
            ..
        }
    ) {
        return Err(CarrierError::EnteredUnconfirmed);
    }
    let id = DeviceId::from_mdf(mdf);
    let keys = match &request.choice {
        CarrierChoice::Stored => match &id {
            Some(id) => request.store.load(id)?.ok_or(CarrierError::NoStoredKeys)?,
            None => return Err(CarrierError::NoStoredKeys),
        },
        CarrierChoice::Device { uuid } => read_device_carrier(root, uuid, || {
            id.as_ref()
                .and_then(|id| request.store.load(id).ok().flatten())
        })?,
        CarrierChoice::File { path } => read_file_carrier(Path::new(path))?,
        // Le couple vient d'une histoire de la Flam dont `info` se déchiffre avec ce `bt`.
        CarrierChoice::Entered { bt, .. } => find_by_bt(root, &*bt.parse()?, cancel)?,
    };
    // Revérifié pour toutes les sources, sauf la saisie : `find_by_bt` vient de retenir une
    // histoire dont le `key` complète le `bt` et dont `info` se déchiffre avec lui, ce que
    // `check_against_device` redemanderait (même parcours, jusqu'à 5000 dossiers).
    if !matches!(request.choice, CarrierChoice::Entered { .. }) {
        check_against_device(root, &keys, cancel)?;
    }
    let stored = match (&request.choice, &id) {
        (CarrierChoice::Stored, _) => true,
        _ if !request.remember => false,
        (_, Some(id)) => match request.store.save(id, &keys) {
            Ok(()) => true,
            Err(err) => {
                tracing::warn!("clés carrier non conservées : {err}");
                false
            }
        },
        (_, None) => {
            tracing::warn!("clés carrier non conservées : Flam non identifiable");
            false
        }
    };
    Ok(Resolved { keys, stored })
}

/// Couple résolu et vérifié ([`resolve`]).
#[derive(Debug)]
pub struct Resolved {
    pub keys: CarrierKeys,
    /// Le couple est conservé sur ce PC pour cette Flam (déjà, ou enregistré à l'instant) :
    /// les envois suivants peuvent utiliser [`CarrierChoice::Stored`].
    pub stored: bool,
}

/// Histoire reconnue par un `bt` saisi, montrée avant tout envoi (aucune clé).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnteredPreview {
    pub uuid: String,
    /// Titre déchiffré : un IV faux altère son début, l'utilisateur le vérifie.
    pub title: String,
    pub hidden: bool,
}

/// Prévisualisation d'un `bt` saisi, en lecture seule : consentement exigé, puis histoire
/// de la Flam dont `info` se déchiffre avec ce `bt` (titre seulement, jamais de clé).
pub fn preview_entered(
    root: &Path,
    consent: Option<&CarrierConsent>,
    bt: &EnteredBt,
    cancel: &AtomicBool,
) -> Result<EnteredPreview, CarrierError> {
    if !consent_is_current(consent) {
        return Err(CarrierError::ConsentRequired);
    }
    let found = find_story_by_bt(root, &*bt.parse()?, cancel)?;
    Ok(EnteredPreview {
        uuid: found.id.to_string(),
        title: found.title,
        hidden: found.hidden,
    })
}

// --- Commandes Tauri --------------------------------------------------------------------------

/// État du mode carrier pour la Flam de `root` (réponse de `device_carrier_state`). Aucune clé.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CarrierState {
    /// Avertissement à afficher tel quel.
    pub warning_text: &'static str,
    pub warning_version: u32,
    /// Consentement enregistré pour l'avertissement courant (`None` : à demander).
    pub consent: Option<CarrierConsent>,
    /// Version du firmware lue dans `.mdf` (`None` : illisible).
    pub firmware: Option<String>,
    /// Le firmware exige le mode carrier (1.x).
    pub carrier_needed: bool,
    /// Un couple est conservé sur ce PC pour cette Flam.
    pub stored_keys: bool,
    /// Histoires officielles utilisables (vide sans consentement ou hors 1.x).
    pub candidates: Vec<CarrierCandidate>,
}

/// Consentement enregistré dans les réglages.
pub(crate) fn stored_consent(config: &ConfigState) -> AppResult<Option<CarrierConsent>> {
    let _guard = config.guard();
    Ok(SettingsStore::new(&config.dir).load()?.carrier_consent)
}

/// État pour la Flam de `root` : avertissement, consentement, firmware, couple conservé et
/// histoires officielles utilisables (lues seulement après consentement). Lecture seule.
pub fn carrier_state(
    root: &Path,
    consent: Option<CarrierConsent>,
    store: &CarrierStore,
) -> Result<CarrierState, TransferError> {
    let consent = consent.filter(|c| consent_is_current(Some(c)));
    let mdf = read_mdf(root)?;
    let firmware = firmware_from_mdf(&mdf);
    let carrier_needed = firmware.as_deref().and_then(firmware_family) == Some(FirmwareFamily::V1);
    let device_id = DeviceId::from_mdf(&mdf);
    let stored_keys = device_id.as_ref().is_some_and(|id| store.has(id));
    let candidates = if carrier_needed && consent.is_some() {
        // Couple conservé : propose aussi les histoires sans `bt` que son `bt` déchiffre.
        let stored = match device_id.as_ref().filter(|_| stored_keys) {
            Some(id) => store.load(id).unwrap_or_else(|err| {
                tracing::warn!("clés carrier conservées illisibles : {err}");
                None
            }),
            None => None,
        };
        list_candidates(root, stored.as_ref())
    } else {
        Vec::new()
    };
    Ok(CarrierState {
        warning_text: WARNING_TEXT,
        warning_version: WARNING_VERSION,
        consent,
        firmware,
        carrier_needed,
        stored_keys,
        candidates,
    })
}

/// État du mode carrier pour la Flam de `root` (voir [`carrier_state`]).
#[tauri::command]
pub async fn device_carrier_state(
    config: State<'_, ConfigState>,
    root: String,
) -> AppResult<CarrierState> {
    let consent = stored_consent(&config)?;
    blocking(move || -> AppResult<CarrierState> {
        Ok(carrier_state(
            Path::new(&root),
            consent,
            &CarrierStore::app(),
        )?)
    })
    .await?
}

/// Enregistre le consentement à l'avertissement `text_version` (case « J'ai compris » cochée :
/// `understood`). Refusé si la case n'est pas cochée ou si l'avertissement a changé.
pub fn accept(
    settings: &SettingsStore,
    text_version: u32,
    understood: bool,
    now: std::time::SystemTime,
) -> AppResult<CarrierConsent> {
    if !understood {
        return Err(AppError::Validation(
            "cochez « J'ai compris » pour utiliser le mode carrier".into(),
        ));
    }
    if text_version != WARNING_VERSION {
        return Err(AppError::Validation(
            "l'avertissement a changé : relisez-le avant de l'accepter".into(),
        ));
    }
    // Une horloge avant 1970 ne donne pas de date : pas de consentement sans date.
    let accepted_at = crate::folder::iso8601(now).ok_or_else(|| {
        AppError::Validation(
            "l'horloge de l'ordinateur est invalide : corrigez la date puis réessayez".into(),
        )
    })?;
    let consent = CarrierConsent {
        text_version,
        accepted_at,
    };
    settings.record_carrier_consent(Some(consent.clone()))?;
    tracing::info!(
        version = text_version,
        "avertissement du mode carrier accepté"
    );
    Ok(consent)
}

/// Enregistre le consentement (voir [`accept`]).
#[tauri::command]
pub async fn device_carrier_accept(
    config: State<'_, ConfigState>,
    text_version: u32,
    understood: bool,
) -> AppResult<CarrierConsent> {
    let _guard = config.guard();
    accept(
        &SettingsStore::new(&config.dir),
        text_version,
        understood,
        std::time::SystemTime::now(),
    )
}

/// Oublie le couple conservé pour la Flam de `root`, ou tous sans `root`. Rend le nombre de
/// fichiers supprimés.
#[tauri::command]
pub async fn device_carrier_forget(root: Option<String>) -> AppResult<usize> {
    blocking(move || -> AppResult<usize> {
        let store = CarrierStore::app();
        let removed = match root {
            Some(root) => {
                // Racine reçue de l'interface : un lecteur détecté seulement, comme pour l'envoi.
                super::transfer::ensure_detected_root(Path::new(&root))?;
                let mdf = read_mdf(Path::new(&root))?;
                let id = DeviceId::from_mdf(&mdf).ok_or_else(|| {
                    AppError::Validation("Flam non identifiable (.mdf trop court)".into())
                })?;
                store.forget(Some(&id))
            }
            None => store.forget(None),
        }
        .map_err(TransferError::from)?;
        Ok(removed)
    })
    .await?
}

#[cfg(test)]
pub(crate) mod tests;
