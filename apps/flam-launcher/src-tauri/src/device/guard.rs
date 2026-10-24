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
//! Aucune autre condition (#326) : les clés de `.mdf`, le fichier `key` déduit du numéro de
//! série, le chiffrement, les chemins et l'index sont ceux de Lunii.QT (`device_flam.py`, révision
//! `c8afe43`), qui écrit sur une 2.x sans confirmation. Il n'y a donc ni confirmation « j'ai
//! sauvegardé ma Flam », ni preuve par `usr/0/library.cache`.
//!
//! Comportement observé de la 2.x : les histoires **officielles** gardent le fichier `key` de leur
//! compte et sont chiffrées avec la clé de ce compte, que les clés de `.mdf` ne déchiffrent pas.
//! Seules les histoires installées par un outil tiers (fichier `key` déduit du numéro de série)
//! se déchiffrent avec elles.
//!
//! **Cohérence** (informative, [`coherence_v2`], à l'affichage seulement) : seules comptent les
//! histoires **au format Flam** (`main.lsf` présent, `info` non lisible en clair), non installées
//! par ce launcher, dont le fichier `key` est celui déduit du numéro de série. `info` déchiffré :
//! clés cohérentes ; illisible : **contradiction**, signalée comme un avertissement dans « Ma
//! Flam » sans rien refuser (faux positifs possibles : `info` en Latin-1, ou de moins de 4
//! caractères). Les histoires au format Lunii (`ni`, `li`, `ri`, `si` sans `.lsf`) et celles dont
//! `info` est en clair sont écartées ; un `info` lisible avec un autre fichier `key` confirme la
//! clé et l'IV sans rien conclure sur le fichier `key`.
//!
//! L'état exposé à l'interface ([`FirmwareStatus`]) ne contient ni clé, ni `bt`, ni numéro de
//! série, ni empreinte de `.mdf` : seulement la version, la taille, l'en-tête (2 octets), le
//! VID/PID, des nombres d'histoires et les UUID des histoires en contradiction.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::carrier::{
    info_is_plain_text, read_info, read_small, story_dirs, KEY_FILE, MAX_SMALL_FILE,
};
use super::crypto::{
    firmware_family, FirmwareFamily, KEY_FILE_LEN, MDF_IV_OFFSET, MDF_KEY_OFFSET, MDF_LEN,
};
use super::detect::MDF_FILE;
use super::transfer::{ensure_root_among, firmware_field, keys_from_mdf, read_mdf, TransferError};

/// En-tête de `.mdf` mesuré (Flam 1.15.14), pour la disposition « standard ».
pub const KNOWN_HEADER: [u8; 2] = [0x01, 0x00];
/// Taille minimale de `.mdf` : jusqu'à la fin de l'IV (0x6E), dernier champ lu.
pub const MDF_MIN_LEN: usize = MDF_IV_OFFSET + 16;
/// VID et PID USB attendus à 0x4A et 0x4C (supposés, Flam de première génération).
const EXPECTED_USB: (u16, u16) = (0x303A, 0x819E);
/// Durée pendant laquelle l'écran « Ma Flam » réutilise un contrôle de cohérence avant de le
/// refaire.
const STATUS_REUSE: Duration = Duration::from_secs(30);

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
    /// Aucune ne se déchiffre, et l'une au moins aurait dû (même fichier `key`) : avertissement,
    /// sans refus (faux positifs possibles, voir le module). `stories` :
    /// UUID de ces histoires (au plus [`MAX_CONTRADICTING`], triés), pour que l'utilisateur
    /// sache lesquelles retirer ; pas un secret, jamais repris dans le diagnostic copiable.
    Contradiction {
        counts: CoherenceCounts,
        stories: Vec<String>,
        /// Nombre d'UUID distincts en cause, avant troncature (un même UUID dans `str/` et
        /// `str.hidden/` ne compte qu'une fois).
        total: usize,
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
        // Jugé sur la structure du clair (UTF-8, premier bloc strict), pas sur la présence d'un
        // titre : un `info` déchiffré sans titre n'est pas une contradiction (#326).
        let readable = keys.decrypts_info(&info);
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
        let total = contradicting.len();
        contradicting.truncate(MAX_CONTRADICTING);
        Coherence::Contradiction {
            counts,
            stories: contradicting,
            total,
        }
    } else {
        Coherence::NoEvidence { counts }
    })
}

/// Empreinte de `.mdf` pour la mémoire de session : SHA-256 préfixé, en hexadécimal. Jamais
/// exposée à l'interface, ni journalisée, ni écrite sur disque.
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

// --- Contexte et mémoire de session ---------------------------------------------------------

/// Résultats de session (jamais écrits sur disque) : dernier contrôle de cohérence, par empreinte
/// de `.mdf` et racine.
#[derive(Debug, Default)]
pub struct KeySession {
    entries: Mutex<HashMap<(String, String), SessionEntry>>,
}

#[derive(Debug, Clone, Default)]
struct SessionEntry {
    coherence: Option<(Coherence, Instant)>,
    /// Nombre d'écritures `Other` contrôlées : une scrutation commencée avant l'une d'elles ne
    /// garde pas son résultat, peut-être antérieur à un retrait (#345).
    generation: u64,
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

/// Contexte d'une écriture : mémoire de session, racines détectées, histoires installées par ce
/// launcher (les deux dernières lues à la demande, une seule fois).
pub struct WriteContext<'a> {
    session: &'a KeySession,
    /// Racines détectées ; `None` : pas de contrôle (hors Windows, aucune détection). Énumérées
    /// au premier contrôle de racine seulement : l'affichage (`device_status`, scruté) n'en a
    /// pas besoin (#326).
    detected_roots: OnceCell<Option<Vec<PathBuf>>>,
    cache_root: Option<PathBuf>,
    launcher: OnceCell<HashSet<String>>,
}

impl WriteContext<'static> {
    /// Contexte de l'application : mémoire de session globale, lecteurs détectés (Windows,
    /// énumérés à la demande), bibliothèque du dossier de cache `cache_root`.
    pub fn current(cache_root: Option<PathBuf>) -> Self {
        Self {
            session: KeySession::global(),
            detected_roots: OnceCell::new(),
            cache_root,
            launcher: OnceCell::new(),
        }
    }
}

impl<'a> WriteContext<'a> {
    /// Contexte explicite (tests).
    pub fn new(
        session: &'a KeySession,
        detected_roots: Option<Vec<PathBuf>>,
        launcher_stories: HashSet<String>,
    ) -> Self {
        let launcher = OnceCell::new();
        let _ = launcher.set(launcher_stories);
        Self {
            session,
            detected_roots: OnceCell::from(detected_roots),
            cache_root: None,
            launcher,
        }
    }

    /// Racines détectées (énumérées au premier appel pour le contexte de l'application).
    fn detected_roots(&self) -> Option<&[PathBuf]> {
        self.detected_roots
            .get_or_init(|| cfg!(windows).then(|| super::detect::Environment::current().roots))
            .as_deref()
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
    /// Cohérence des clés 2.x (informative : une contradiction est un avertissement).
    pub coherence: Coherence,
    /// Raison du refus d'écrire ; `None` : écriture autorisée.
    pub write_block: Option<WriteRefused>,
}

fn header_hex([a, b]: [u8; 2]) -> String {
    format!("{a:02x} {b:02x}")
}

/// Comment obtenir les contrôles 2.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Écriture : conditions refaites ; cohérence non contrôlée (informative, affichage seul).
    Write(WriteKind),
    /// Affichage : contrôle réutilisé s'il est récent ; aucun si la Flam est occupée.
    Status { busy: bool },
}

struct Assessment {
    layout: MdfLayout,
    coherence: Coherence,
    refused: Option<WriteRefused>,
}

fn trivial(field: &[u8]) -> bool {
    field.iter().all(|&b| b == 0) || field.iter().all(|&b| b == 0xFF)
}

/// Évaluation commune à l'écriture et à l'affichage.
fn assess(root: &Path, mdf: &[u8], ctx: &WriteContext<'_>, mode: Mode) -> Assessment {
    let layout = analyze(mdf);
    let version = layout.version.clone().unwrap_or_default();
    let done = |layout: MdfLayout, coherence, refused| Assessment {
        layout,
        coherence,
        refused,
    };
    if mdf.len() < MDF_MIN_LEN {
        let refused = WriteRefused::Truncated { size: layout.size };
        return done(layout, Coherence::NotChecked, Some(refused));
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
                Some(WriteRefused::VersionUnreadable),
            );
        }
        Family::Unsupported => {
            let refused = WriteRefused::UnsupportedFirmware { version };
            return done(layout, Coherence::NotChecked, Some(refused));
        }
        Family::V1 => return done(layout, Coherence::NotApplicable, None),
        Family::V2 => {}
    }

    // 2.x : clés de `.mdf` lisibles et non triviales.
    if let Err(err) = keys_from_mdf(mdf) {
        let refused = WriteRefused::KeysUnreadable {
            detail: err.to_string(),
        };
        return done(layout, Coherence::NotChecked, Some(refused));
    }
    if trivial(&mdf[MDF_KEY_OFFSET..MDF_KEY_OFFSET + 16])
        || trivial(&mdf[MDF_IV_OFFSET..MDF_IV_OFFSET + 16])
    {
        let refused = WriteRefused::KeysTrivial { version };
        return done(layout, Coherence::NotChecked, Some(refused));
    }

    // Cohérence : informative, contrôlée pour l'affichage seulement (une contradiction ne refuse
    // rien, #326). Une écriture `Other` (retrait, réordonnancement, reprise) peut retirer
    // l'histoire en cause : elle oublie le contrôle gardé et fait avancer la génération, pour
    // qu'une scrutation commencée avant elle ne garde pas son résultat et que « Ma Flam » ne
    // montre pas un UUID retiré pendant [`STATUS_REUSE`] (#345). L'envoi n'écrit que des
    // histoires du launcher, exclues du contrôle : il ne touche à rien.
    let busy = match mode {
        Mode::Write(kind) => {
            if kind == WriteKind::Other {
                ctx.session.with(key, |e| {
                    e.generation += 1;
                    e.coherence = None;
                });
            }
            return done(layout, Coherence::NotChecked, None);
        }
        Mode::Status { busy } => busy,
    };
    let (cached, generation) = ctx
        .session
        .with(key.clone(), |e| (e.coherence.clone(), e.generation));
    let coherence = match cached {
        Some((c, at)) if busy || at.elapsed() < STATUS_REUSE => c,
        None if busy => Coherence::NotChecked,
        _ => match coherence_v2(root, mdf, ctx.launcher_stories()) {
            Ok(c) => {
                ctx.session.with(key, |e| {
                    if e.generation == generation {
                        e.coherence = Some((c.clone(), Instant::now()));
                    }
                });
                c
            }
            Err(err) => {
                let refused = WriteRefused::KeysUnreadable {
                    detail: err.to_string(),
                };
                return done(layout, Coherence::NotChecked, Some(refused));
            }
        },
    };
    done(layout, coherence, None)
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
    if let Some(roots) = ctx.detected_roots() {
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
        write_block: Some(WriteRefused::MdfUnreadable { detail }),
    }
}

#[cfg(test)]
pub(crate) mod tests;
