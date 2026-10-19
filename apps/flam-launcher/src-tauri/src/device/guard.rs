//! Garde commune de toute écriture sur la Flam (#317) : envoi, reprise d'un transfert, retrait,
//! réordonnancement. Point d'entrée unique : [`check_write`], qui rend un [`WritePermit`]
//! exigé par chacune de ces écritures (elles ne compilent pas sans lui).
//!
//! # Conditions communes
//!
//! 1. La racine est un lecteur détecté (amovible, ou FAT fixe), comme avant (#314).
//! 2. `.mdf` est présent et lisible (64 Kio au plus), d'au moins 126 octets.
//! 3. La version du firmware est lisible et de la famille 1.x ou 2.x
//!    ([`firmware_family`], forme stricte).
//!
//! # Disposition reconnue
//!
//! Seule disposition mesurée : celle d'une Flam 1.15.14 (126 octets, en-tête `01 00`, version
//! nue). Est « reconnue » :
//!
//! - en 1.x : 126 octets, en-tête `01 00`, version nue (mesuré) ;
//! - en 2.x : 126 octets, en-tête `01 00`, « main: » présent (supposé : jamais mesuré).
//!
//! # Firmware 1.x
//!
//! Écriture autorisée si la disposition est reconnue ; sinon refus. Les clés d'un envoi restent
//! celles du mode carrier, vérifiées sur une histoire officielle (#234) : rien ne change.
//!
//! # Firmware 2.x : vérification par preuve
//!
//! Avant la première écriture d'une session sur une Flam (racine et contenu de `.mdf`), les
//! clés lues dans `.mdf` (clé et IV à 0x4E et 0x5E, fichier `key` déduit du numéro de série)
//! sont confrontées aux histoires présentes **qui n'ont pas été installées par ce launcher**
//! (en pratique, les histoires officielles) : [`prove_v2_keys`]. Une histoire installée par le
//! launcher ne prouve rien, puisqu'elle a été chiffrée avec ces mêmes clés.
//!
//! | Résultat de la preuve | Disposition reconnue                     | Disposition inconnue |
//! |-----------------------|------------------------------------------|----------------------|
//! | vérifiées             | écriture autorisée                       | écriture autorisée   |
//! | contradiction         | refus                                    | refus                |
//! | non vérifiables       | autorisée après confirmation explicite   | refus                |
//!
//! - **Vérifiées** : une histoire a un fichier `key` identique à celui déduit du numéro de
//!   série **et** son `info` se déchiffre avec la clé et l'IV de `.mdf` (contrôle strict du mode
//!   carrier, [`super::carrier::info_text`]). Mémorisé pour la session.
//! - **Contradiction** : aucune histoire n'est vérifiée, mais l'une d'elles aurait dû l'être :
//!   même fichier `key` et `info` illisible (clé ou IV faux), ou `info` lisible avec un autre
//!   fichier `key` (fichier `key` déduit faux). Rien n'est écrit.
//! - **Non vérifiables** : aucune histoire exploitable. L'utilisateur peut confirmer
//!   (« j'ai sauvegardé ma Flam ») dans l'interface, seulement si la disposition est reconnue ;
//!   la confirmation est mémorisée pour la session seulement ([`KeySession`]).
//!
//! La preuve est refaite avant chaque écriture tant qu'elle n'est pas « vérifiée » : une
//! contradiction apparue après une confirmation l'emporte.
//!
//! L'état exposé à l'interface ([`FirmwareStatus`]) ne contient ni clé, ni `bt`, ni numéro de
//! série, ni empreinte de `.mdf` : seulement la version, la taille, l'en-tête (2 octets) et des
//! nombres d'histoires.

use std::cell::OnceCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::carrier::{read_info, read_small, story_dirs, KEY_FILE, MAX_SMALL_FILE};
use super::crypto::{firmware_family, FirmwareFamily, KEY_FILE_LEN, MDF_LEN};
use super::detect::MDF_FILE;
use super::transfer::{ensure_root_among, firmware_field, keys_from_mdf, read_mdf, TransferError};

/// En-tête de `.mdf` mesuré (Flam 1.15.14) : seul admis pour une disposition reconnue.
pub const KNOWN_HEADER: [u8; 2] = [0x01, 0x00];
/// Durée pendant laquelle l'écran « Ma Flam » réutilise une preuve non vérifiée avant de la
/// refaire (l'écriture, elle, la refait toujours).
const STATUS_REUSE: Duration = Duration::from_secs(30);

/// Forme du texte de version de `.mdf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionForm {
    /// « main: x.y.z-… » (forme supposée des firmwares 2.x).
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
    /// Disposition reconnue (voir le module).
    pub recognized: bool,
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
    let expected_form = match family {
        Family::V1 => Some(VersionForm::Bare),
        Family::V2 => Some(VersionForm::Main),
        _ => None,
    };
    let recognized = mdf.len() == MDF_LEN
        && header == Some(KNOWN_HEADER)
        && expected_form.is_some_and(|f| f == form);
    MdfLayout {
        size,
        header,
        version,
        form,
        family,
        recognized,
    }
}

/// Bilan d'une preuve (nombres d'histoires seulement).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ProofCounts {
    /// Histoires examinées (non installées par ce launcher, avec `key` et `info`).
    pub examined: usize,
    /// Dont le fichier `key` est celui déduit du numéro de série.
    pub key_matches: usize,
    /// Dont le fichier `key` correspond et `info` se déchiffre avec les clés de `.mdf`.
    pub decrypted: usize,
    /// Dont `info` se déchiffre avec les clés de `.mdf` malgré un autre fichier `key`.
    pub decrypted_other_key: usize,
}

/// Résultat d'une preuve des clés 2.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyProof {
    Verified(ProofCounts),
    Contradiction(ProofCounts),
    Unverifiable(ProofCounts),
}

impl KeyProof {
    fn from_counts(counts: ProofCounts) -> Self {
        if counts.decrypted > 0 {
            Self::Verified(counts)
        } else if counts.key_matches > 0 || counts.decrypted_other_key > 0 {
            Self::Contradiction(counts)
        } else {
            Self::Unverifiable(counts)
        }
    }
}

/// Confronte les clés 2.x de `mdf` aux histoires de la Flam de `root`, sauf celles dont l'UUID
/// figure dans `launcher_stories` (installées par ce launcher). Lecture seule ; aucune clé ne
/// sort de cette fonction. Erreur : clés illisibles dans `.mdf` (numéro de série, taille).
pub fn prove_v2_keys(
    root: &Path,
    mdf: &[u8],
    launcher_stories: &HashSet<String>,
) -> Result<KeyProof, TransferError> {
    let keys = keys_from_mdf(mdf)?;
    let expected_key = keys.key_file();
    let mut counts = ProofCounts::default();
    for (id, dir, _) in story_dirs(root) {
        if launcher_stories.contains(id.as_str()) {
            continue;
        }
        let Ok(Some(key)) = read_small(&dir.join(KEY_FILE), MAX_SMALL_FILE) else {
            continue;
        };
        let Ok(Some(info)) = read_info(&dir) else {
            continue;
        };
        counts.examined += 1;
        let same_key = key.len() == KEY_FILE_LEN && key[..] == expected_key[..];
        let readable = keys.title_of_info(&info).is_some();
        match (same_key, readable) {
            (true, true) => {
                counts.key_matches += 1;
                counts.decrypted += 1;
            }
            (true, false) => counts.key_matches += 1,
            (false, true) => counts.decrypted_other_key += 1,
            (false, false) => {}
        }
    }
    Ok(KeyProof::from_counts(counts))
}

/// Identité d'une Flam pour la session : racine normalisée et empreinte de `.mdf` (jamais
/// exposée).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SessionKey {
    root: String,
    mdf: [u8; 32],
}

impl SessionKey {
    fn new(root: &Path, mdf: &[u8]) -> Self {
        Self {
            root: normalize_root(root),
            mdf: Sha256::digest(mdf).into(),
        }
    }
}

fn normalize_root(root: &Path) -> String {
    root.to_string_lossy()
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase()
}

#[derive(Debug, Clone, Copy)]
struct SessionEntry {
    proof: Option<(KeyProof, Instant)>,
    confirmed: bool,
}

/// Mémoire de session des preuves et des confirmations (jamais écrite sur disque).
#[derive(Debug, Default)]
pub struct KeySession {
    entries: Mutex<HashMap<SessionKey, SessionEntry>>,
}

static SESSION: LazyLock<KeySession> = LazyLock::new(KeySession::default);

impl KeySession {
    /// Mémoire de l'application.
    pub fn global() -> &'static KeySession {
        &SESSION
    }

    fn get(&self, key: &SessionKey) -> Option<SessionEntry> {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(key)
            .copied()
    }

    fn update(&self, key: SessionKey, f: impl FnOnce(&mut SessionEntry)) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        f(entries.entry(key).or_insert(SessionEntry {
            proof: None,
            confirmed: false,
        }));
    }
}

/// Contexte d'une écriture : mémoire de session, racines détectées, histoires installées par
/// ce launcher (lues à la demande, une seule fois).
pub struct WriteContext<'a> {
    session: &'a KeySession,
    /// Racines détectées ; `None` : pas de contrôle (hors Windows, aucune détection).
    detected_roots: Option<Vec<PathBuf>>,
    cache_root: Option<PathBuf>,
    launcher: OnceCell<HashSet<String>>,
}

impl WriteContext<'static> {
    /// Contexte de l'application : mémoire globale, lecteurs détectés (Windows), bibliothèque
    /// du dossier de cache `cache_root`.
    pub fn current(cache_root: Option<PathBuf>) -> Self {
        Self {
            session: KeySession::global(),
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
    /// Contexte explicite (tests, ou appelant qui connaît déjà les histoires du launcher).
    pub fn new(
        session: &'a KeySession,
        detected_roots: Option<Vec<PathBuf>>,
        launcher_stories: HashSet<String>,
    ) -> Self {
        let launcher = OnceCell::new();
        let _ = launcher.set(launcher_stories);
        Self {
            session,
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
    #[error(
        "fichier .mdf de la Flam tronqué ({size} octets, {MDF_LEN} attendus) : rien n'a été écrit"
    )]
    Truncated { size: u64 },
    #[error("version du firmware illisible dans .mdf : rien n'a été écrit ; voir l'état du firmware dans « Ma Flam »")]
    VersionUnreadable,
    #[error("firmware de la Flam non pris en charge : « {version} » (1.x ou 2.x seulement) ; rien n'a été écrit")]
    UnsupportedFirmware { version: String },
    #[error("disposition du fichier .mdf inconnue (firmware {version}, {size} octets, en-tête {header}) : rien n'a été écrit ; voir l'état du firmware dans « Ma Flam »")]
    UnknownLayout {
        version: String,
        size: u64,
        header: String,
    },
    #[error("clés de la Flam illisibles dans .mdf ({detail}) : rien n'a été écrit")]
    KeysUnreadable { detail: String },
    #[error("les clés lues dans .mdf ne correspondent pas aux histoires officielles de cette Flam (firmware {version}) : rien n'a été écrit ; voir l'état du firmware dans « Ma Flam »")]
    KeysContradiction { version: String },
    #[error("clés de la Flam non vérifiables (firmware {version}, aucune histoire officielle exploitable) : confirmez dans « Ma Flam » avoir sauvegardé la Flam ; rien n'a été écrit")]
    KeysUnverified { version: String },
}

/// Preuve que [`check_write`] a autorisé l'écriture. Seul ce module la construit.
#[derive(Debug)]
pub struct WritePermit {
    family: FirmwareFamily,
    mdf_digest: [u8; 32],
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

    /// Autorisation sans contrôle, pour les tests des étapes d'écriture elles-mêmes.
    #[cfg(test)]
    pub(crate) fn for_tests(family: FirmwareFamily, mdf: &[u8]) -> Self {
        Self {
            family,
            mdf_digest: Sha256::digest(mdf).into(),
        }
    }
}

/// État des clés, pour l'interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeysState {
    /// 1.x : clés du mode carrier, vérifiées à chaque envoi (#234).
    NotApplicable,
    /// Preuve non faite (firmware refusé avant, ou Flam occupée).
    NotChecked,
    Verified {
        counts: ProofCounts,
    },
    Contradiction {
        counts: ProofCounts,
    },
    Unverifiable {
        counts: ProofCounts,
    },
    /// Non vérifiables, confirmé par l'utilisateur pour la session.
    Confirmed {
        counts: ProofCounts,
    },
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
    pub layout_recognized: bool,
    pub keys: KeysState,
    /// Raison du refus d'écrire ; `None` : écriture autorisée. `keys_unverified` : autorisée
    /// après confirmation.
    pub write_block: Option<WriteRefused>,
}

fn header_hex(header: Option<[u8; 2]>) -> String {
    header.map_or_else(|| "absent".to_string(), |[a, b]| format!("{a:02x} {b:02x}"))
}

/// Comment obtenir la preuve 2.x.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProofMode {
    /// Écriture : refaite sauf si déjà vérifiée.
    Write,
    /// Affichage : réutilisée si récente ; jamais faite si la Flam est occupée.
    Status { busy: bool },
}

/// Évaluation commune à l'écriture et à l'affichage.
fn assess(
    root: &Path,
    mdf: &[u8],
    ctx: &WriteContext<'_>,
    mode: ProofMode,
) -> (MdfLayout, KeysState, Option<WriteRefused>) {
    let layout = analyze(mdf);
    let version = layout.version.clone().unwrap_or_default();
    let block = |reason| (layout.clone(), KeysState::NotChecked, Some(reason));
    if mdf.len() < MDF_LEN {
        return block(WriteRefused::Truncated { size: layout.size });
    }
    let family = match layout.family {
        Family::Unreadable => return block(WriteRefused::VersionUnreadable),
        Family::Unsupported => return block(WriteRefused::UnsupportedFirmware { version }),
        Family::V1 => FirmwareFamily::V1,
        Family::V2 => FirmwareFamily::V2,
    };
    let unknown_layout = || WriteRefused::UnknownLayout {
        version: version.clone(),
        size: layout.size,
        header: header_hex(layout.header),
    };
    if family == FirmwareFamily::V1 {
        let refused = (!layout.recognized).then(unknown_layout);
        return (layout.clone(), KeysState::NotApplicable, refused);
    }

    let key = SessionKey::new(root, mdf);
    let entry = ctx.session.get(&key);
    let cached = entry.and_then(|e| e.proof);
    let confirmed = entry.is_some_and(|e| e.confirmed);
    let reuse = match (mode, cached) {
        (_, Some((proof @ KeyProof::Verified(_), _))) => Some(proof),
        (ProofMode::Status { .. }, Some((proof, at))) if at.elapsed() < STATUS_REUSE => Some(proof),
        (ProofMode::Status { busy: true }, Some((proof, _))) => Some(proof),
        _ => None,
    };
    let proof = match reuse {
        Some(proof) => proof,
        None if mode == (ProofMode::Status { busy: true }) => {
            return (layout.clone(), KeysState::NotChecked, None);
        }
        None => match prove_v2_keys(root, mdf, ctx.launcher_stories()) {
            Ok(proof) => {
                ctx.session
                    .update(key, |e| e.proof = Some((proof, Instant::now())));
                proof
            }
            Err(err) => {
                return block(WriteRefused::KeysUnreadable {
                    detail: err.to_string(),
                })
            }
        },
    };
    match proof {
        KeyProof::Verified(counts) => (layout.clone(), KeysState::Verified { counts }, None),
        KeyProof::Contradiction(counts) => (
            layout.clone(),
            KeysState::Contradiction { counts },
            Some(WriteRefused::KeysContradiction { version }),
        ),
        KeyProof::Unverifiable(counts) if !layout.recognized => (
            layout.clone(),
            KeysState::Unverifiable { counts },
            Some(unknown_layout()),
        ),
        KeyProof::Unverifiable(counts) if confirmed => {
            (layout.clone(), KeysState::Confirmed { counts }, None)
        }
        KeyProof::Unverifiable(counts) => (
            layout.clone(),
            KeysState::Unverifiable { counts },
            Some(WriteRefused::KeysUnverified { version }),
        ),
    }
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

/// **Point d'entrée unique** : autorise ou refuse une écriture sur la Flam de `root` (voir le
/// module). Lecture seule. À appeler sous le verrou de l'appareil, avant toute écriture, y
/// compris une reprise.
pub fn check_write(root: &Path, ctx: &WriteContext<'_>) -> Result<WritePermit, WriteRefused> {
    if let Some(roots) = &ctx.detected_roots {
        ensure_root_among(root, roots).map_err(|_| WriteRefused::InvalidRoot)?;
    }
    if !std::fs::symlink_metadata(root.join(MDF_FILE)).is_ok_and(|m| m.is_file()) {
        return Err(WriteRefused::DeviceNotFound);
    }
    let mdf = read_device_mdf(root)?;
    let (layout, _, refused) = assess(root, &mdf, ctx, ProofMode::Write);
    if let Some(reason) = refused {
        tracing::warn!(raison = %reason, "écriture sur la Flam refusée par la garde du firmware");
        return Err(reason);
    }
    let family = match layout.family {
        Family::V1 => FirmwareFamily::V1,
        _ => FirmwareFamily::V2,
    };
    Ok(WritePermit {
        family,
        mdf_digest: Sha256::digest(&mdf[..]).into(),
    })
}

/// État du firmware pour l'écran « Ma Flam » (lecture seule ; `busy` : une écriture est en
/// cours, la preuve n'est pas refaite).
pub fn firmware_status(
    root: &Path,
    mdf: &[u8],
    ctx: &WriteContext<'_>,
    busy: bool,
) -> FirmwareStatus {
    let (layout, keys, write_block) = assess(root, mdf, ctx, ProofMode::Status { busy });
    FirmwareStatus {
        version: layout.version,
        family: layout.family,
        version_form: layout.form,
        mdf_size: layout.size,
        header: layout.header.map(|h| header_hex(Some(h))),
        layout_recognized: layout.recognized,
        keys,
        write_block,
    }
}

/// État d'un `.mdf` illisible (trop grand, volume retiré) : écriture refusée.
pub fn unreadable_status(size: u64, detail: String) -> FirmwareStatus {
    FirmwareStatus {
        version: None,
        family: Family::Unreadable,
        version_form: VersionForm::Unreadable,
        mdf_size: size,
        header: None,
        layout_recognized: false,
        keys: KeysState::NotChecked,
        write_block: Some(WriteRefused::MdfUnreadable { detail }),
    }
}

/// Enregistre la confirmation « clés non vérifiables, Flam sauvegardée » pour la session.
/// Acceptée seulement si l'état est exactement « non vérifiables » sur une disposition
/// reconnue (preuve refaite) ; sinon rend la raison du refus actuel.
pub fn confirm_unverified(
    root: &Path,
    ctx: &WriteContext<'_>,
) -> Result<FirmwareStatus, WriteRefused> {
    if let Some(roots) = &ctx.detected_roots {
        ensure_root_among(root, roots).map_err(|_| WriteRefused::InvalidRoot)?;
    }
    let mdf = read_device_mdf(root)?;
    let (_, _, refused) = assess(root, &mdf, ctx, ProofMode::Write);
    match refused {
        Some(WriteRefused::KeysUnverified { .. }) => {
            ctx.session
                .update(SessionKey::new(root, &mdf), |e| e.confirmed = true);
            tracing::info!(
                "clés de la Flam non vérifiables : sauvegarde confirmée pour la session"
            );
        }
        Some(other) => return Err(other),
        None => {}
    }
    Ok(firmware_status(root, &mdf, ctx, false))
}

#[cfg(test)]
pub(crate) mod tests;
