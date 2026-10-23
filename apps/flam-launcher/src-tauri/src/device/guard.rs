//! Garde commune de toute écriture sur la Flam (#317) : envoi, reprise d'un transfert, retrait,
//! réordonnancement. Point d'entrée unique : [`check_write`], qui rend un [`WritePermit`]
//! exigé par chacune de ces écritures (elles ne compilent pas sans lui).
//!
//! # Conditions communes (refus sinon)
//!
//! 1. La racine est un lecteur détecté (amovible, ou FAT fixe), comme avant (#314).
//! 2. `.mdf` est présent, lisible (64 Kio au plus) et couvre les champs lus (au moins
//!    [`MDF_MIN_LEN`] octets, jusqu'à la fin de l'IV).
//! 3. La version du firmware est lisible, avec ou sans préfixe « main: », et de la famille 1.x
//!    ou 2.x ([`firmware_family`], forme stricte ; 3.x et suivants refusés).
//! 4. En 2.x seulement (les clés de `.mdf` y servent) : numéro de série hexadécimal valide,
//!    clé et IV non triviaux (ni tout à zéro, ni tout à `FF`).
//!
//! Une taille autre que 126 octets ou un en-tête autre que `01 00` (seules valeurs mesurées,
//! sur une Flam 1.15.14) ne refuse rien : l'état signale une « disposition non standard ». Un
//! VID/PID USB inattendu (0x4A, 0x4C) est seulement journalisé.
//!
//! # Firmware 1.x
//!
//! Aucune autre condition : les clés d'un envoi restent celles du mode carrier, vérifiées sur une
//! histoire officielle à chaque envoi (#234).
//!
//! # Firmware 2.x
//!
//! Comportement observé de la 2.x : les histoires **officielles** gardent le fichier `key` de leur
//! compte et sont chiffrées avec la clé de ce compte, que les clés de `.mdf` ne déchiffrent pas.
//! Seules les histoires installées par un outil tiers (fichier `key` déduit du numéro de série)
//! se déchiffrent avec elles. Les histoires officielles ne prouvent donc rien.
//!
//! - **Confirmation** : avant la première écriture sur une Flam 2.x, l'utilisateur confirme
//!   avoir sauvegardé sa Flam ([`confirm`]). La confirmation est mémorisée sur le PC par
//!   empreinte de `.mdf` ([`TrustStore`] : SHA-256 préfixé, jamais la clé elle-même), d'une
//!   session à l'autre.
//! - **Vérification par la Flam** : après un envoi et un redémarrage de la Flam, le firmware
//!   reconstruit `usr/0/library.cache` avec les titres qu'il a déchiffrés. Avant toute écriture
//!   (donc avant que le launcher supprime ce cache), et à l'affichage, le titre d'une histoire
//!   installée par ce launcher y est cherché ([`flam_cache_proof`]) : trouvé, l'état devient
//!   « vérifié par la Flam », mémorisé de même ; la confirmation n'est plus demandée.
//! - **Cohérence** (informative, [`coherence_v2`]) : seules comptent les histoires **au format
//!   Flam** (`main.lsf` présent, `info` non lisible en clair), non installées par ce launcher,
//!   dont le fichier `key` est celui déduit du numéro de série. `info` déchiffré : clés
//!   cohérentes ; illisible : **contradiction**, qui refuse l'envoi (les autres écritures, dont
//!   le retrait, restent possibles). Les histoires au format Lunii (`ni`, `li`, `ri`, `si` sans
//!   `.lsf`) et celles dont `info` est en clair sont écartées ; un `info` lisible avec un autre
//!   fichier `key` confirme la clé et l'IV sans rien conclure sur le fichier `key`.
//!
//! L'état exposé à l'interface ([`FirmwareStatus`]) ne contient ni clé, ni `bt`, ni numéro de
//! série, ni empreinte de `.mdf` : seulement la version, la taille, l'en-tête (2 octets), le
//! VID/PID et des nombres d'histoires.

use std::cell::OnceCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::carrier::{
    info_is_plain_text, read_info, read_small, story_dirs, KEY_FILE, MAX_SMALL_FILE,
};
use super::crypto::{
    firmware_family, FirmwareFamily, KEY_FILE_LEN, MDF_IV_OFFSET, MDF_KEY_OFFSET, MDF_LEN,
};
use super::detect::MDF_FILE;
use super::index::LIBRARY_CACHE;
use super::transfer::{ensure_root_among, firmware_field, keys_from_mdf, read_mdf, TransferError};

/// En-tête de `.mdf` mesuré (Flam 1.15.14), pour la disposition « standard ».
pub const KNOWN_HEADER: [u8; 2] = [0x01, 0x00];
/// Taille minimale de `.mdf` : jusqu'à la fin de l'IV (0x6E), dernier champ lu.
pub const MDF_MIN_LEN: usize = MDF_IV_OFFSET + 16;
/// VID et PID USB attendus à 0x4A et 0x4C (supposés, Flam de première génération).
const EXPECTED_USB: (u16, u16) = (0x303A, 0x819E);
/// Durée pendant laquelle l'écran « Ma Flam » réutilise un contrôle de cohérence avant de le
/// refaire (l'écriture, elle, le refait toujours).
const STATUS_REUSE: Duration = Duration::from_secs(30);
/// Taille maximale lue de `library.cache` (185 Kio observés).
const MAX_CACHE_READ: u64 = 32 * 1024 * 1024;
/// Longueur minimale (caractères) d'un titre cherché dans `library.cache`.
const MIN_PROOF_TITLE: usize = 4;
/// Fichier des confirmations et vérifications, dans le dossier de données du launcher.
pub const TRUST_FILE: &str = "device-trust.json";
/// Taille maximale lue de [`TRUST_FILE`].
const MAX_TRUST_FILE: u64 = 1024 * 1024;
/// Nombre maximal d'empreintes conservées par liste (les plus anciennes sont oubliées).
const MAX_TRUST_ENTRIES: usize = 256;

/// Forme du texte de version de `.mdf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionForm {
    /// « main: x.y.z-… ».
    Main,
    /// Version nue en tête du champ (`1.15.14-…`, mesurée en 1.x).
    Bare,
    /// Aucune version lisible.
    Unreadable,
}

/// Famille du firmware, pour l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    V1,
    V2,
    /// Version lisible mais hors 1.x et 2.x (3.x et suivants, forme non canonique).
    Unsupported,
    /// Version illisible ou `.mdf` illisible.
    Unreadable,
}

/// Disposition de `.mdf` (aucun champ secret).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdfLayout {
    pub size: u64,
    pub header: Option<[u8; 2]>,
    pub version: Option<String>,
    pub form: VersionForm,
    pub family: Family,
    /// 126 octets et en-tête `01 00` (seule disposition mesurée).
    pub standard: bool,
    /// VID et PID USB lus à 0x4A et 0x4C (petit-boutistes).
    pub usb: Option<(u16, u16)>,
}

/// Analyse la disposition de `.mdf` (lecture pure).
pub fn analyze(mdf: &[u8]) -> MdfLayout {
    let size = mdf.len() as u64;
    let header = mdf.get(..2).map(|h| [h[0], h[1]]);
    let (version, form) = match firmware_field(mdf) {
        Some((version, form)) => (Some(version), form),
        None => (None, VersionForm::Unreadable),
    };
    let family = match version.as_deref() {
        None => Family::Unreadable,
        Some(v) => match firmware_family(v) {
            Some(FirmwareFamily::V1) => Family::V1,
            Some(FirmwareFamily::V2) => Family::V2,
            None => Family::Unsupported,
        },
    };
    let u16_at = |at: usize| {
        mdf.get(at..at + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let usb = u16_at(0x4A).zip(u16_at(0x4C));
    MdfLayout {
        size,
        header,
        version,
        form,
        family,
        standard: mdf.len() == MDF_LEN && header == Some(KNOWN_HEADER),
        usb,
    }
}

// --- Cohérence des clés 2.x --------------------------------------------------------------------

/// Bilan du contrôle de cohérence (nombres d'histoires seulement).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct CoherenceCounts {
    /// Histoires au format Flam examinées (non installées par ce launcher, `key` et `info`
    /// chiffré présents).
    pub examined: usize,
    /// Dont le fichier `key` est celui déduit du numéro de série.
    pub key_matches: usize,
    /// Dont le fichier `key` correspond et `info` se déchiffre avec les clés de `.mdf`.
    pub decrypted: usize,
    /// Dont `info` se déchiffre avec les clés de `.mdf` malgré un autre fichier `key`.
    pub decrypted_other_key: usize,
    /// Histoires écartées : format Lunii (`ni`, `li`, `ri`, `si` sans `.lsf`).
    pub lunii_format: usize,
    /// Histoires écartées : `info` lisible en clair.
    pub clear_info: usize,
}

/// Nombre maximal d'UUID d'histoires en cause gardés dans [`Coherence::Contradiction`].
pub const MAX_CONTRADICTING: usize = 20;

/// Résultat du contrôle de cohérence des clés 2.x.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Coherence {
    /// 1.x : sans objet (mode carrier).
    NotApplicable,
    /// Contrôle non fait (refus antérieur, ou Flam occupée).
    NotChecked,
    /// Au moins une histoire au format Flam se déchiffre avec les clés de `.mdf`.
    Coherent { counts: CoherenceCounts },
    /// Aucune ne se déchiffre, et l'une au moins aurait dû (même fichier `key`). `stories` :
    /// UUID de ces histoires (au plus [`MAX_CONTRADICTING`], triés), pour que l'utilisateur
    /// sache lesquelles retirer ; pas un secret, jamais repris dans le diagnostic copiable.
    Contradiction {
        counts: CoherenceCounts,
        stories: Vec<String>,
    },
    /// Aucune histoire exploitable.
    NoEvidence { counts: CoherenceCounts },
}

/// Marqueurs d'une histoire au format Lunii (sans scripts `.lsf`).
const LUNII_FILES: [&str; 4] = ["ni", "li", "ri", "si"];

/// Format d'un dossier d'histoire.
fn is_flam_format(dir: &Path) -> bool {
    dir.join("main.lsf").is_file()
}

fn is_lunii_format(dir: &Path) -> bool {
    !is_flam_format(dir) && LUNII_FILES.iter().any(|name| dir.join(name).is_file())
}

/// Contrôle de cohérence des clés 2.x de `mdf` avec les histoires de la Flam de `root`, hors
/// celles de `launcher_stories` (voir le module). Lecture seule ; aucune clé ne sort de cette
/// fonction. Erreur : clés illisibles dans `.mdf`.
pub fn coherence_v2(
    root: &Path,
    mdf: &[u8],
    launcher_stories: &HashSet<String>,
) -> Result<Coherence, TransferError> {
    let keys = keys_from_mdf(mdf)?;
    let expected_key = keys.key_file();
    let mut counts = CoherenceCounts::default();
    let mut contradicting = Vec::new();
    for (id, dir, _) in story_dirs(root) {
        if launcher_stories.contains(id.as_str()) {
            continue;
        }
        if is_lunii_format(&dir) {
            counts.lunii_format += 1;
            continue;
        }
        if !is_flam_format(&dir) {
            continue;
        }
        let Ok(Some(key)) = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE) else {
            continue;
        };
        let Ok(Some(info)) = read_info(&dir) else {
            continue;
        };
        if info_is_plain_text(&info) {
            counts.clear_info += 1;
            continue;
        }
        counts.examined += 1;
        let same_key = key.len() == KEY_FILE_LEN && key[..] == expected_key[..];
        let readable = keys.title_of_info(&info).is_some();
        match (same_key, readable) {
            (true, true) => {
                counts.key_matches += 1;
                counts.decrypted += 1;
            }
            (true, false) => {
                counts.key_matches += 1;
                contradicting.push(id.as_str().to_string());
            }
            (false, true) => counts.decrypted_other_key += 1,
            (false, false) => {}
        }
    }
    Ok(if counts.decrypted + counts.decrypted_other_key > 0 {
        Coherence::Coherent { counts }
    } else if counts.key_matches > 0 {
        contradicting.sort();
        contradicting.dedup();
        contradicting.truncate(MAX_CONTRADICTING);
        Coherence::Contradiction {
            counts,
            stories: contradicting,
        }
    } else {
        Coherence::NoEvidence { counts }
    })
}

// --- Vérification par la Flam (library.cache) -------------------------------------------------

/// Vrai si `usr/0/library.cache` de la Flam de `root` contient le titre (UTF-8 ou UTF-16LE)
/// d'une histoire installée par ce launcher (UUID de `launcher_stories`, fichier `key` déduit du
/// numéro de série, `info` déchiffré avec les clés de `mdf`) : le firmware a donc déchiffré cette
/// histoire. Lecture seule (le cache n'est jamais modifié ici).
pub fn flam_cache_proof(root: &Path, mdf: &[u8], launcher_stories: &HashSet<String>) -> bool {
    if launcher_stories.is_empty() {
        return false;
    }
    let Ok(keys) = keys_from_mdf(mdf) else {
        return false;
    };
    let expected_key = keys.key_file();
    let mut titles = Vec::new();
    for (id, dir, _) in story_dirs(root) {
        if !launcher_stories.contains(id.as_str()) {
            continue;
        }
        let Ok(Some(key)) = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE) else {
            continue;
        };
        if key.len() != KEY_FILE_LEN || key[..] != expected_key[..] {
            continue;
        }
        let Ok(Some(info)) = read_info(&dir) else {
            continue;
        };
        if let Some(title) = keys.title_of_info(&info) {
            if title.chars().count() >= MIN_PROOF_TITLE {
                titles.push(title);
            }
        }
    }
    if titles.is_empty() {
        return false;
    }
    let Some(cache) = read_cache(root) else {
        return false;
    };
    titles.iter().any(|title| {
        let units: Vec<u16> = title.encode_utf16().collect();
        contains_field(&cache, title.as_bytes()) || contains_field_utf16(&cache, &units)
    })
}

fn read_cache(root: &Path) -> Option<Vec<u8>> {
    let path = root.join(LIBRARY_CACHE);
    if !fs::symlink_metadata(&path).is_ok_and(|m| m.is_file()) {
        return None;
    }
    let mut data = Vec::new();
    fs::File::open(&path)
        .ok()?
        .take(MAX_CACHE_READ)
        .read_to_end(&mut data)
        .ok()?;
    Some(data)
}

/// Vrai si `needle` figure dans `haystack` comme **champ entier** : précédé du début, d'un octet
/// de contrôle (< 0x20) ou de sa longueur, et suivi de la fin ou d'un octet de contrôle. Un titre
/// inclus dans un autre (« Capi » dans « Le Noël de Capi ») ne compte pas.
fn contains_field(haystack: &[u8], needle: &[u8]) -> bool {
    let n = needle.len();
    if n == 0 || haystack.len() < n {
        return false;
    }
    let len_byte = (n & 0xFF) as u8;
    memchr_all(haystack, needle).any(|at| {
        let before = at.checked_sub(1).map(|i| haystack[i]);
        let after = haystack.get(at + n).copied();
        before.is_none_or(|b| b < 0x20 || b == len_byte) && after.is_none_or(|b| b < 0x20)
    })
}

/// Même règle que [`contains_field`] pour un titre en UTF-16 petit-boutiste (unités de 16 bits
/// alignées sur leur début dans le texte trouvé).
fn contains_field_utf16(haystack: &[u8], units: &[u16]) -> bool {
    let needle: Vec<u8> = units.iter().flat_map(|u| u.to_le_bytes()).collect();
    let n = needle.len();
    if units.is_empty() || haystack.len() < n {
        return false;
    }
    let unit_at = |i: usize| {
        haystack
            .get(i..i + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let count = units.len() as u16;
    let found = memchr_all(haystack, &needle).any(|at| {
        let before = at.checked_sub(2).and_then(unit_at);
        let after = unit_at(at + n);
        at % 2 == 0
            && (at == 0 || before.is_some_and(|u| u < 0x20 || u == count))
            && (at + n + 2 > haystack.len() || after.is_some_and(|u| u < 0x20))
    });
    found
}

/// Positions de `needle` dans `haystack` (recherche guidée par le premier octet).
fn memchr_all<'h>(haystack: &'h [u8], needle: &'h [u8]) -> impl Iterator<Item = usize> + 'h {
    // Motif vide ou plus long que le texte : aucune position (jamais de panique).
    let usable = !needle.is_empty() && haystack.len() >= needle.len();
    let first = needle.first().copied().unwrap_or(0);
    let last_start = if usable {
        haystack.len() - needle.len() + 1
    } else {
        0
    };
    haystack[..last_start]
        .iter()
        .enumerate()
        .filter(move |(i, &b)| b == first && &haystack[*i..*i + needle.len()] == needle)
        .map(|(i, _)| i)
}

// --- Confirmations et vérifications mémorisées ----------------------------------------------

/// Empreinte de `.mdf` pour la mémoire du PC : SHA-256 préfixé, en hexadécimal. Jamais exposée
/// à l'interface ni journalisée.
fn fingerprint(mdf: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"flam-launcher/mdf-fingerprint/v1/");
    hasher.update(mdf);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct TrustData {
    #[serde(default)]
    confirmed: BTreeSet<String>,
    #[serde(default)]
    flam_verified: BTreeSet<String>,
}

/// Confiance accordée à une Flam 2.x, pour l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Trust {
    /// 1.x : sans objet.
    NotApplicable,
    /// Le titre d'une histoire envoyée figure dans `library.cache` reconstruit par la Flam.
    FlamVerified,
    /// L'utilisateur a confirmé avoir sauvegardé cette Flam.
    Confirmed,
    /// Confirmation à demander avant toute écriture.
    Unconfirmed,
}

/// Mémoire des confirmations et vérifications, par empreinte de `.mdf` (fichier JSON du dossier
/// de données ; en mémoire seulement si `path` est `None`).
#[derive(Debug, Default)]
pub struct TrustStore {
    path: Option<PathBuf>,
    data: Mutex<Option<TrustData>>,
}

static APP_TRUST: LazyLock<TrustStore> =
    LazyLock::new(|| TrustStore::at(crate::storage::data_dir().join(TRUST_FILE)));

impl TrustStore {
    /// Mémoire enregistrée dans le fichier `path`.
    pub fn at(path: PathBuf) -> Self {
        Self {
            path: Some(path),
            data: Mutex::new(None),
        }
    }

    /// Mémoire de l'application.
    pub fn app() -> &'static TrustStore {
        &APP_TRUST
    }

    fn with<T>(&self, f: impl FnOnce(&mut TrustData) -> T) -> T {
        let mut guard = self.data.lock().unwrap_or_else(PoisonError::into_inner);
        let data = guard.get_or_insert_with(|| self.load());
        f(data)
    }

    fn load(&self) -> TrustData {
        let Some(path) = &self.path else {
            return TrustData::default();
        };
        let mut text = String::new();
        let read =
            fs::File::open(path).and_then(|f| f.take(MAX_TRUST_FILE).read_to_string(&mut text));
        match read {
            Ok(_) => serde_json::from_str(&text).unwrap_or_else(|err| {
                tracing::warn!("confirmations des Flam illisibles, ignorées : {err}");
                TrustData::default()
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => TrustData::default(),
            Err(err) => {
                tracing::warn!("confirmations des Flam illisibles, ignorées : {err}");
                TrustData::default()
            }
        }
    }

    fn save(&self, data: &TrustData) {
        let Some(path) = &self.path else {
            return;
        };
        let result = serde_json::to_vec_pretty(data)
            .map_err(std::io::Error::other)
            .and_then(|bytes| crate::storage::write_atomic(path, &bytes));
        if let Err(err) = result {
            tracing::warn!("confirmation de la Flam non enregistrée (session seulement) : {err}");
        }
    }

    fn trust(&self, mdf: &[u8]) -> Trust {
        let fp = fingerprint(mdf);
        self.with(|data| {
            if data.flam_verified.contains(&fp) {
                Trust::FlamVerified
            } else if data.confirmed.contains(&fp) {
                Trust::Confirmed
            } else {
                Trust::Unconfirmed
            }
        })
    }

    fn record(&self, mdf: &[u8], verified: bool) {
        let fp = fingerprint(mdf);
        let snapshot = self.with(|data| {
            let set = if verified {
                &mut data.flam_verified
            } else {
                &mut data.confirmed
            };
            if !set.insert(fp) {
                return None;
            }
            while set.len() > MAX_TRUST_ENTRIES {
                let first = set.iter().next().cloned();
                if let Some(first) = first {
                    set.remove(&first);
                }
            }
            Some(data.clone())
        });
        if let Some(data) = snapshot {
            self.save(&data);
        }
    }
}

// --- Contexte et mémoire de session ---------------------------------------------------------

/// Résultats de session (jamais écrits sur disque) : dernier contrôle de cohérence et dernier
/// `library.cache` examiné sans succès, par empreinte de `.mdf` et racine.
#[derive(Debug, Default)]
pub struct KeySession {
    entries: Mutex<HashMap<(String, String), SessionEntry>>,
}

#[derive(Debug, Clone, Default)]
struct SessionEntry {
    coherence: Option<(Coherence, Instant)>,
    /// Taille et date du `library.cache` déjà examiné sans succès.
    cache_seen: Option<(u64, Option<std::time::SystemTime>)>,
    usb_logged: bool,
}

static SESSION: LazyLock<KeySession> = LazyLock::new(KeySession::default);

impl KeySession {
    /// Mémoire de l'application.
    pub fn global() -> &'static KeySession {
        &SESSION
    }

    fn with<T>(&self, key: (String, String), f: impl FnOnce(&mut SessionEntry) -> T) -> T {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        f(entries.entry(key).or_default())
    }
}

fn session_key(root: &Path, mdf: &[u8]) -> (String, String) {
    let root = root
        .to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase();
    (root, fingerprint(mdf))
}

/// Contexte d'une écriture : mémoires, racines détectées, histoires installées par ce launcher
/// (lues à la demande, une seule fois).
pub struct WriteContext<'a> {
    session: &'a KeySession,
    trust: &'a TrustStore,
    /// Racines détectées ; `None` : pas de contrôle (hors Windows, aucune détection).
    detected_roots: Option<Vec<PathBuf>>,
    cache_root: Option<PathBuf>,
    launcher: OnceCell<HashSet<String>>,
}

impl WriteContext<'static> {
    /// Contexte de l'application : mémoires globales, lecteurs détectés (Windows), bibliothèque
    /// du dossier de cache `cache_root`.
    pub fn current(cache_root: Option<PathBuf>) -> Self {
        Self {
            session: KeySession::global(),
            trust: TrustStore::app(),
            detected_roots: if cfg!(windows) {
                Some(super::detect::Environment::current().roots)
            } else {
                None
            },
            cache_root,
            launcher: OnceCell::new(),
        }
    }
}

impl<'a> WriteContext<'a> {
    /// Contexte explicite (tests).
    pub fn new(
        session: &'a KeySession,
        trust: &'a TrustStore,
        detected_roots: Option<Vec<PathBuf>>,
        launcher_stories: HashSet<String>,
    ) -> Self {
        let launcher = OnceCell::new();
        let _ = launcher.set(launcher_stories);
        Self {
            session,
            trust,
            detected_roots,
            cache_root: None,
            launcher,
        }
    }

    fn launcher_stories(&self) -> &HashSet<String> {
        self.launcher
            .get_or_init(|| super::status::launcher_story_ids(self.cache_root.as_deref()))
    }
}

// --- Refus, autorisation, état ----------------------------------------------------------------

/// Nature de l'écriture demandée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteKind {
    /// Envoi d'une histoire (contenu chiffré avec les clés de la Flam).
    Send,
    /// Reprise, retrait, réordonnancement : rien n'est chiffré.
    Other,
}

/// Refus d'écrire (sérialisé pour l'interface : aucun champ secret). Le message dit toujours
/// que rien n'a été écrit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, thiserror::Error)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteRefused {
    #[error("Flam introuvable à cet emplacement (fichier .mdf absent) : rien n'a été écrit")]
    DeviceNotFound,
    #[error(
        "emplacement refusé : ce n'est pas un lecteur amovible ou FAT détecté ; rien n'a été écrit"
    )]
    InvalidRoot,
    #[error("fichier .mdf de la Flam illisible ({detail}) : rien n'a été écrit")]
    MdfUnreadable { detail: String },
    #[error("fichier .mdf de la Flam tronqué ({size} octets, {MDF_MIN_LEN} au moins) : rien n'a été écrit")]
    Truncated { size: u64 },
    #[error("version du firmware illisible dans .mdf : rien n'a été écrit ; voir l'état du firmware dans « Ma Flam »")]
    VersionUnreadable,
    #[error("firmware de la Flam non pris en charge : « {version} » (1.x ou 2.x seulement) ; rien n'a été écrit")]
    UnsupportedFirmware { version: String },
    #[error("clés de la Flam illisibles dans .mdf ({detail}) : rien n'a été écrit")]
    KeysUnreadable { detail: String },
    #[error("clés de la Flam invalides dans .mdf (firmware {version} : clé ou IV vide) : rien n'a été écrit ; voir l'état du firmware dans « Ma Flam »")]
    KeysTrivial { version: String },
    #[error("les clés lues dans .mdf ne déchiffrent pas une histoire de cette Flam qui devrait l'être (firmware {version}) : rien n'a été envoyé ; voir l'état du firmware dans « Ma Flam »")]
    KeysContradiction { version: String },
    #[error("Flam en firmware {version} (en test) : confirmez dans « Ma Flam » avoir sauvegardé la Flam ; rien n'a été écrit")]
    ConfirmationRequired { version: String, standard: bool },
}

/// Preuve que [`check_write`] a autorisé l'écriture. Seul ce module la construit.
#[derive(Debug)]
pub struct WritePermit {
    family: FirmwareFamily,
    mdf_digest: [u8; 32],
    /// Racine contrôlée, normalisée (`None` : permis de test, valable partout).
    root: Option<String>,
}

impl WritePermit {
    /// Famille du firmware autorisée.
    pub fn family(&self) -> FirmwareFamily {
        self.family
    }

    /// Vrai si `mdf` est le `.mdf` contrôlé par la garde (relu ensuite par l'écriture).
    pub fn same_mdf(&self, mdf: &[u8]) -> bool {
        <[u8; 32]>::from(Sha256::digest(mdf)) == self.mdf_digest
    }

    /// Vrai si le permis a été donné pour la Flam de `root`.
    pub fn covers(&self, root: &Path) -> bool {
        self.root
            .as_ref()
            .is_none_or(|checked| *checked == session_key(root, &[]).0)
    }

    /// Autorisation sans contrôle, pour les tests des étapes d'écriture elles-mêmes.
    #[cfg(test)]
    pub(crate) fn for_tests(family: FirmwareFamily, mdf: &[u8]) -> Self {
        Self {
            family,
            mdf_digest: Sha256::digest(mdf).into(),
            root: None,
        }
    }
}

/// État du firmware de la Flam (réponse sérialisée : aucune clé, aucun `bt`, aucun numéro de
/// série, aucune empreinte).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirmwareStatus {
    pub version: Option<String>,
    pub family: Family,
    pub version_form: VersionForm,
    pub mdf_size: u64,
    /// Deux premiers octets de `.mdf` en hexadécimal (`01 00`).
    pub header: Option<String>,
    /// 126 octets et en-tête `01 00` (seule disposition mesurée) ; faux : « non standard ».
    pub layout_standard: bool,
    /// VID:PID USB lus dans `.mdf` (`303a:819e`).
    pub usb_ids: Option<String>,
    pub coherence: Coherence,
    pub trust: Trust,
    /// Raison du refus d'écrire ; `None` : écriture autorisée. `confirmation_required` :
    /// autorisée après confirmation.
    pub write_block: Option<WriteRefused>,
}

fn header_hex([a, b]: [u8; 2]) -> String {
    format!("{a:02x} {b:02x}")
}

/// Comment obtenir les contrôles 2.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Écriture : contrôles refaits.
    Write(WriteKind),
    /// Affichage : contrôle réutilisé s'il est récent ; aucun si la Flam est occupée.
    Status { busy: bool },
}

struct Assessment {
    layout: MdfLayout,
    coherence: Coherence,
    trust: Trust,
    refused: Option<WriteRefused>,
}

fn trivial(field: &[u8]) -> bool {
    field.iter().all(|&b| b == 0) || field.iter().all(|&b| b == 0xFF)
}

/// Évaluation commune à l'écriture, à l'affichage et à la confirmation.
fn assess(root: &Path, mdf: &[u8], ctx: &WriteContext<'_>, mode: Mode) -> Assessment {
    let layout = analyze(mdf);
    let version = layout.version.clone().unwrap_or_default();
    let done = |layout: MdfLayout, coherence, trust, refused| Assessment {
        layout,
        coherence,
        trust,
        refused,
    };
    if mdf.len() < MDF_MIN_LEN {
        let refused = WriteRefused::Truncated { size: layout.size };
        return done(
            layout,
            Coherence::NotChecked,
            Trust::NotApplicable,
            Some(refused),
        );
    }
    let key = session_key(root, mdf);
    if let Some(usb) = layout.usb.filter(|usb| *usb != EXPECTED_USB) {
        if !ctx
            .session
            .with(key.clone(), |e| std::mem::replace(&mut e.usb_logged, true))
        {
            tracing::warn!(vid = %format!("{:04x}", usb.0), pid = %format!("{:04x}", usb.1), "VID/PID USB inattendus dans .mdf (sans refus)");
        }
    }
    match layout.family {
        Family::Unreadable => {
            return done(
                layout,
                Coherence::NotChecked,
                Trust::NotApplicable,
                Some(WriteRefused::VersionUnreadable),
            );
        }
        Family::Unsupported => {
            let refused = WriteRefused::UnsupportedFirmware { version };
            return done(
                layout,
                Coherence::NotChecked,
                Trust::NotApplicable,
                Some(refused),
            );
        }
        Family::V1 => return done(layout, Coherence::NotApplicable, Trust::NotApplicable, None),
        Family::V2 => {}
    }

    // 2.x : clés de `.mdf` lisibles et non triviales.
    if let Err(err) = keys_from_mdf(mdf) {
        let refused = WriteRefused::KeysUnreadable {
            detail: err.to_string(),
        };
        return done(
            layout,
            Coherence::NotChecked,
            Trust::Unconfirmed,
            Some(refused),
        );
    }
    if trivial(&mdf[MDF_KEY_OFFSET..MDF_KEY_OFFSET + 16])
        || trivial(&mdf[MDF_IV_OFFSET..MDF_IV_OFFSET + 16])
    {
        let refused = WriteRefused::KeysTrivial { version };
        return done(
            layout,
            Coherence::NotChecked,
            Trust::Unconfirmed,
            Some(refused),
        );
    }
    let busy = mode == Mode::Status { busy: true };

    // Vérification par la Flam, avant toute écriture (le cache n'est pas encore supprimé).
    let mut trust = ctx.trust.trust(mdf);
    if trust != Trust::FlamVerified && !busy {
        let cache_meta = fs::metadata(root.join(LIBRARY_CACHE))
            .ok()
            .map(|m| (m.len(), m.modified().ok()));
        let seen = ctx.session.with(key.clone(), |e| e.cache_seen);
        let launcher = ctx.launcher_stories();
        if cache_meta.is_some() && cache_meta != seen && !launcher.is_empty() {
            if flam_cache_proof(root, mdf, launcher) {
                ctx.trust.record(mdf, true);
                trust = Trust::FlamVerified;
                tracing::info!(
                    "Flam 2.x : clés vérifiées par la Flam (titre trouvé dans library.cache)"
                );
            } else {
                ctx.session.with(key.clone(), |e| e.cache_seen = cache_meta);
            }
        }
    }

    // Cohérence (informative ; une contradiction refuse l'envoi).
    let cached = ctx.session.with(key.clone(), |e| e.coherence.clone());
    let coherence = match (mode, cached) {
        (Mode::Status { .. }, Some((c, at))) if busy || at.elapsed() < STATUS_REUSE => c,
        (Mode::Status { busy: true }, None) => Coherence::NotChecked,
        _ => match coherence_v2(root, mdf, ctx.launcher_stories()) {
            Ok(c) => {
                let stored = c.clone();
                ctx.session.with(key.clone(), |e| {
                    e.coherence = Some((stored, Instant::now()))
                });
                c
            }
            Err(err) => {
                let refused = WriteRefused::KeysUnreadable {
                    detail: err.to_string(),
                };
                return done(layout, Coherence::NotChecked, trust, Some(refused));
            }
        },
    };
    let blocks_send = mode != Mode::Write(WriteKind::Other);
    let refused = if matches!(coherence, Coherence::Contradiction { .. }) && blocks_send {
        Some(WriteRefused::KeysContradiction { version })
    } else if trust == Trust::Unconfirmed {
        Some(WriteRefused::ConfirmationRequired {
            version,
            standard: layout.standard,
        })
    } else {
        None
    };
    done(layout, coherence, trust, refused)
}

/// Lit `.mdf` de `root` pour la garde.
fn read_device_mdf(root: &Path) -> Result<zeroize::Zeroizing<Vec<u8>>, WriteRefused> {
    match read_mdf(root) {
        Ok(mdf) => Ok(mdf),
        Err(TransferError::DeviceNotFound) => Err(WriteRefused::DeviceNotFound),
        Err(err) => Err(WriteRefused::MdfUnreadable {
            detail: err.to_string(),
        }),
    }
}

fn check_root(root: &Path, ctx: &WriteContext<'_>) -> Result<(), WriteRefused> {
    if let Some(roots) = &ctx.detected_roots {
        ensure_root_among(root, roots).map_err(|_| WriteRefused::InvalidRoot)?;
    }
    if !fs::symlink_metadata(root.join(MDF_FILE)).is_ok_and(|m| m.is_file()) {
        return Err(WriteRefused::DeviceNotFound);
    }
    Ok(())
}

/// **Point d'entrée unique** : autorise ou refuse une écriture `kind` sur la Flam de `root`
/// (voir le module). N'écrit rien sur la Flam. À appeler sous le verrou de l'appareil, avant
/// toute écriture, y compris une reprise.
pub fn check_write(
    root: &Path,
    ctx: &WriteContext<'_>,
    kind: WriteKind,
) -> Result<WritePermit, WriteRefused> {
    check_root(root, ctx)?;
    let mdf = read_device_mdf(root)?;
    let assessment = assess(root, &mdf, ctx, Mode::Write(kind));
    if let Some(reason) = assessment.refused {
        tracing::warn!(raison = %reason, "écriture sur la Flam refusée par la garde du firmware");
        return Err(reason);
    }
    let family = match assessment.layout.family {
        Family::V1 => FirmwareFamily::V1,
        _ => FirmwareFamily::V2,
    };
    if !assessment.layout.standard {
        tracing::warn!(
            taille = assessment.layout.size,
            "écriture sur une Flam à la disposition de .mdf non standard"
        );
    }
    Ok(WritePermit {
        family,
        mdf_digest: Sha256::digest(&mdf[..]).into(),
        root: Some(session_key(root, &[]).0),
    })
}

fn status_of(a: Assessment) -> FirmwareStatus {
    FirmwareStatus {
        version: a.layout.version,
        family: a.layout.family,
        version_form: a.layout.form,
        mdf_size: a.layout.size,
        header: a.layout.header.map(header_hex),
        layout_standard: a.layout.standard,
        usb_ids: a.layout.usb.map(|(v, p)| format!("{v:04x}:{p:04x}")),
        coherence: a.coherence,
        trust: a.trust,
        write_block: a.refused,
    }
}

/// État du firmware pour l'écran « Ma Flam » (rien n'est écrit sur la Flam ; `busy` : une
/// écriture est en cours, les contrôles ne sont pas refaits).
pub fn firmware_status(
    root: &Path,
    mdf: &[u8],
    ctx: &WriteContext<'_>,
    busy: bool,
) -> FirmwareStatus {
    status_of(assess(root, mdf, ctx, Mode::Status { busy }))
}

/// État d'un `.mdf` illisible (trop grand, volume retiré) : écriture refusée.
pub fn unreadable_status(size: u64, detail: String) -> FirmwareStatus {
    FirmwareStatus {
        version: None,
        family: Family::Unreadable,
        version_form: VersionForm::Unreadable,
        mdf_size: size,
        header: None,
        layout_standard: false,
        usb_ids: None,
        coherence: Coherence::NotChecked,
        trust: Trust::NotApplicable,
        write_block: Some(WriteRefused::MdfUnreadable { detail }),
    }
}

/// Enregistre la confirmation « Firmware 2.x en test : j'ai sauvegardé ma Flam », mémorisée par
/// empreinte de `.mdf`. Acceptée seulement si la confirmation est la seule chose qui manque ;
/// sinon rend la raison du refus. Rien n'est écrit sur la Flam.
pub fn confirm(root: &Path, ctx: &WriteContext<'_>) -> Result<FirmwareStatus, WriteRefused> {
    check_root(root, ctx)?;
    let mdf = read_device_mdf(root)?;
    // Une contradiction (qui ne refuse que l'envoi) n'empêche pas de confirmer : le retrait d'une
    // histoire doit rester possible.
    let assessment = assess(root, &mdf, ctx, Mode::Write(WriteKind::Other));
    match assessment.refused {
        Some(WriteRefused::ConfirmationRequired { .. }) => {
            ctx.trust.record(&mdf, false);
            tracing::info!("Flam 2.x : sauvegarde confirmée par l'utilisateur");
        }
        Some(other) => return Err(other),
        None => {}
    }
    Ok(firmware_status(root, &mdf, ctx, false))
}

#[cfg(test)]
pub(crate) mod tests;
