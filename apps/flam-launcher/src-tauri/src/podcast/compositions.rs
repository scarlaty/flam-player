//! Compositions nommées réutilisables des packs de podcasts (#461, N3 c, spécification
//! `docs/podcasts.md` § 14).
//!
//! Après la création d'un pack, sa composition ([`PackComposition`], #362) est conservée sous
//! un nom dans `pack-compositions.json` (dossier de configuration). Le parent peut la rouvrir
//! (copie dans le brouillon unique, N3 b) pour refaire ou dériver un pack, la renommer, la
//! supprimer. Chaque pack reste une nouvelle histoire (P3) : rien ici ne touche à la file
//! d'installation ni à la bibliothèque.
//!
//! # Invariants
//!
//! - Toute modification est un cycle lecture-modification-écriture fait **sous le verrou de
//!   configuration** par l'appelant, terminé par une écriture atomique
//!   ([`storage::write_atomic`]) : un arrêt brutal laisse l'ancien fichier ou le nouveau.
//! - Rien n'est jamais perdu sans copie : un fichier illisible ou trop gros est mis de côté
//!   (`*.bak`) ; un fichier lisible dont certaines entrées ne le sont pas est **copié** en
//!   `*.bak` avant d'être réécrit avec les seules entrées lisibles (copie retirée si la
//!   réécriture échoue : l'original, intact, la remplace). Seules les [`MAX_BACKUPS`] copies
//!   les plus récentes sont gardées.
//! - Un fichier d'un schéma plus récent n'est ni lu ni modifié (erreur, comme
//!   `settings.json`) ; les champs inconnus d'un fichier lisible (premier niveau du fichier,
//!   d'une entrée ou de sa composition) sont conservés à la réécriture.
//! - [`MAX_COMPOSITIONS`] entrées au plus, aucune suppression automatique ; noms uniques sans
//!   tenir compte de la casse, de 1 à [`MAX_NAME_CHARS`] caractères.
//! - Aucune URL sur le disque ni dans les messages : une composition ne contient que des
//!   identifiants et des textes saisis (D11).
//! - Une composition identique à une entrée existante ne crée pas de doublon (« refaire ») ;
//!   une composition modifiée crée une nouvelle entrée (« dériver »), l'originale reste
//!   intacte.

use std::collections::{BTreeSet, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::State;

use super::pack::{self, PackComposition, MAX_REVISION};
use crate::sources::{Source, SourcesStore};
use crate::storage::{self, ConfigState, Loaded};
use crate::{AppError, AppResult};

/// Fichier des compositions, dans le dossier de configuration.
pub const COMPOSITIONS_FILE: &str = "pack-compositions.json";
/// Version de schéma du fichier.
pub const COMPOSITIONS_SCHEMA: u32 = 1;
/// Nombre de compositions conservées au plus.
pub const MAX_COMPOSITIONS: usize = 50;
/// Longueur d'un nom, en caractères.
pub const MAX_NAME_CHARS: usize = 60;
/// Taille d'une composition sérialisée au plus, en octets (JSON indenté à partir de la racine de la\n/// composition ; dans le fichier, plus profondément indentée, elle est un peu plus grosse).
pub const MAX_ENTRY_BYTES: usize = 256 * 1024;
/// Copies `*.bak` du fichier gardées au plus (les plus récentes).
pub const MAX_BACKUPS: usize = 3;
/// Taille du fichier au plus, en octets (lecture et écriture).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Longueur d'un identifiant de composition au plus.
const MAX_ID_LEN: usize = 64;
/// Nom donné à une composition dont le titre effectif est vide.
const FALLBACK_NAME: &str = "Pack";

/// Composition conservée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedComposition {
    /// Identifiant opaque (uuid v4 tiré par le launcher).
    pub id: String,
    pub name: String,
    /// Secondes Unix.
    #[serde(default)]
    pub created_at: u64,
    /// Date de la dernière création lancée depuis cette composition (secondes Unix).
    #[serde(default)]
    pub last_used_at: u64,
    /// Créations lancées depuis cette composition.
    #[serde(default)]
    pub packs: u32,
    pub composition: PackComposition,
    /// Champs inconnus de l'entrée (écrits par une version plus récente), conservés tels quels
    /// à la réécriture.
    #[serde(flatten)]
    pub extra: Map<String, Value>,
    /// Champs inconnus de `composition` (même raison) ; réinsérés par [`Store::write`].
    #[serde(skip)]
    pub composition_extra: Map<String, Value>,
}

impl SavedComposition {
    /// Valeur écrite : l'entrée, avec les champs inconnus de sa composition réinsérés.
    fn to_stored(&self) -> AppResult<Value> {
        let mut value = serde_json::to_value(self)?;
        if let Some(Value::Object(composition)) = value.get_mut("composition") {
            for (key, extra) in &self.composition_extra {
                composition
                    .entry(key.clone())
                    .or_insert_with(|| extra.clone());
            }
        }
        Ok(value)
    }

    /// Lit une entrée en gardant les champs inconnus de sa composition (ceux que
    /// [`PackComposition`] ne réécrit pas).
    fn from_stored(value: Value) -> Option<Self> {
        let raw_composition = value.get("composition").cloned();
        let mut entry: Self = serde_json::from_value(value).ok()?;
        if let Some(Value::Object(raw)) = raw_composition {
            let known = match serde_json::to_value(&entry.composition) {
                Ok(Value::Object(known)) => known,
                _ => Map::new(),
            };
            entry.composition_extra = raw
                .into_iter()
                .filter(|(key, _)| !known.contains_key(key))
                .collect();
        }
        Some(entry)
    }
}

/// Contenu écrit de `pack-compositions.json`.
#[derive(Serialize)]
struct StoreFile<'a> {
    schema_version: u32,
    compositions: Vec<Value>,
    /// Champs inconnus du premier niveau, conservés.
    #[serde(flatten)]
    extra: &'a Map<String, Value>,
}

/// Lecture tolérante : chaque entrée est désérialisée à part (une entrée illisible ne fait pas
/// perdre les autres).
#[derive(Deserialize)]
struct RawStore {
    #[serde(default)]
    compositions: Vec<Value>,
    /// Tout le reste, sauf `schema_version` (réécrit) : conservé.
    #[serde(flatten)]
    extra: Map<String, Value>,
}

/// Migration du fichier (aucune : schéma 1 seulement ; version 0 = même forme sans
/// `schema_version`).
fn migrate(_from: u32, _object: &mut Map<String, Value>) {}

/// Écriture du fichier (production : [`storage::write_atomic`] ; tests : injection de pannes).
pub type Writer = fn(&Path, &[u8]) -> io::Result<()>;

/// Entrées lues, avec le message à afficher une fois (fichier mis de côté, entrées écartées).
#[derive(Debug, Default)]
pub struct StoreRead {
    pub entries: Vec<SavedComposition>,
    pub notice: Option<String>,
    /// Champs inconnus du premier niveau du fichier, réécrits tels quels.
    extra: Map<String, Value>,
    /// Entrées écartées dont la copie n'a pas pu être faite ou gardée (réécriture en échec) :
    /// la prochaine écriture copie d'abord le fichier d'origine, sinon elle n'a pas lieu. Porte
    /// le compte et le détail des raisons, pour que le message dise la copie faite ensuite.
    backup_pending: Option<(usize, String)>,
}

/// Accès au fichier des compositions de `dir`. L'appelant tient le verrou de configuration.
pub struct Store<'a> {
    dir: &'a Path,
    write: Writer,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

/// Nom saisi, nettoyé : espaces de bord retirés ; `None` s'il est vide, trop long ou contient
/// un caractère de contrôle.
fn checked_name(name: &str) -> Option<&str> {
    let name = name.trim();
    let len = name.chars().count();
    (len > 0 && len <= MAX_NAME_CHARS && !name.chars().any(char::is_control)).then_some(name)
}

fn same_name(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

fn name_taken(entries: &[SavedComposition], name: &str, except: Option<&str>) -> bool {
    entries
        .iter()
        .any(|e| Some(e.id.as_str()) != except && same_name(&e.name, name))
}

/// Nom libre dérivé de `title` : titre nettoyé et borné, suffixe « (2) », « (3) »… si pris.
fn unique_name(title: &str, entries: &[SavedComposition]) -> String {
    let cleaned: String = title
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let base: String = cleaned.trim().chars().take(MAX_NAME_CHARS).collect();
    let base = match base.trim() {
        "" => FALLBACK_NAME.to_string(),
        b => b.to_string(),
    };
    if !name_taken(entries, &base, None) {
        return base;
    }
    // Au plus `MAX_COMPOSITIONS` noms pris : un suffixe libre existe parmi les suivants.
    for n in 2.. {
        let suffix = format!(" ({n})");
        let keep = MAX_NAME_CHARS - suffix.chars().count();
        let stem: String = base.chars().take(keep).collect();
        let candidate = format!("{}{suffix}", stem.trim_end());
        if !name_taken(entries, &candidate, None) {
            return candidate;
        }
    }
    unreachable!("suffixe de nom libre introuvable")
}

/// Copie `path` en `<nom>.<horodatage>.bak` (suffixe numérique si déjà pris), sans le
/// toucher.
fn backup_copy(path: &Path) -> io::Result<PathBuf> {
    let backup = storage::backup_path(path)?;
    fs::copy(path, &backup)?;
    Ok(backup)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Rang d'une copie `<COMPOSITIONS_FILE>.<AAAAMMJJ-HHMMSS>[-<n>].bak` (horodatage puis
/// suffixe) ; `None` pour tout autre nom, jamais supprimé. Le nom est lu tel quel, sans
/// conversion avec perte : un nom qui n'est pas de l'Unicode valide n'est jamais une copie
/// (#489).
fn backup_rank(name: &OsStr) -> Option<(String, u64)> {
    let middle = name
        .to_str()?
        .strip_prefix(COMPOSITIONS_FILE)?
        .strip_prefix('.')?
        .strip_suffix(".bak")?;
    const STAMP_LEN: usize = "AAAAMMJJ-HHMMSS".len();
    let stamp = middle.get(..STAMP_LEN)?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(stamp.is_ascii() && digits(&stamp[..8]) && &stamp[8..9] == "-" && digits(&stamp[9..])) {
        return None;
    }
    let n = match &middle[STAMP_LEN..] {
        "" => 0,
        rest => {
            let n = rest.strip_prefix('-')?;
            if !digits(n) {
                return None;
            }
            n.parse().ok()?
        }
    };
    Some((stamp.to_string(), n))
}

/// Ne garde que [`MAX_BACKUPS`] copies du fichier dans `dir` : `keep` (nom de la copie qui
/// vient d'être faite, jamais supprimée, même si l'horloge a reculé) et les plus récentes des
/// autres. Noms comparés tels quels (`OsStr`), jamais après une conversion avec perte (#489).
/// Au mieux : un échec est tracé, jamais remonté.
fn prune_backups(dir: &Path, keep: &OsStr) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("compositions de pack : copies non triées : {err}");
            return;
        }
    };
    let mut copies: Vec<((String, u64), PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name();
            if name.as_os_str() == keep {
                return None;
            }
            Some((backup_rank(&name)?, e.path()))
        })
        .collect();
    let others = MAX_BACKUPS.saturating_sub(1);
    if copies.len() <= others {
        return;
    }
    copies.sort();
    let excess = copies.len() - others;
    for (_, path) in copies.into_iter().take(excess) {
        if let Err(err) = fs::remove_file(&path) {
            tracing::warn!(
                "compositions de pack : ancienne copie {} non supprimée : {err}",
                path.display()
            );
        }
    }
}

/// Raisons pour lesquelles des entrées ont été écartées à la lecture.
#[derive(Debug, Default)]
struct Rejected {
    /// Désérialisation impossible, identifiant ou nom invalide.
    unreadable: usize,
    /// Identifiant ou nom (casse ignorée) déjà pris par une entrée précédente.
    duplicate: usize,
    /// Au-delà de [`MAX_COMPOSITIONS`].
    surplus: usize,
}

impl Rejected {
    fn total(&self) -> usize {
        self.unreadable + self.duplicate + self.surplus
    }

    /// Détail des raisons, en français : « 1 illisible, 2 en double ».
    fn detail(&self) -> String {
        let mut parts = Vec::new();
        if self.unreadable > 0 {
            parts.push(format!("{} illisible(s)", self.unreadable));
        }
        if self.duplicate > 0 {
            parts.push(format!(
                "{} en double (identifiant ou nom déjà pris)",
                self.duplicate
            ));
        }
        if self.surplus > 0 {
            parts.push(format!("{} au-delà de {MAX_COMPOSITIONS}", self.surplus));
        }
        parts.join(", ")
    }
}

/// Message des entrées écartées, une fois le fichier d'origine copié en `backup`.
fn copied_notice(total: usize, detail: &str, backup: &Path) -> String {
    format!(
        "{total} composition(s) enregistrée(s) écartée(s) ({detail}) ; le fichier d'origine a été copié ({}).",
        file_name(backup)
    )
}

/// Ajoute `notice` au message de `err` : une action en échec ne perd pas le message dit une
/// fois (fichier mis de côté, entrées écartées). Le genre de l'erreur est inchangé.
fn with_notice(err: AppError, notice: Option<String>) -> AppError {
    let Some(notice) = notice else { return err };
    match err {
        AppError::NotFound(m) => AppError::NotFound(format!("{m}. {notice}")),
        AppError::Validation(m) => AppError::Validation(format!("{m}. {notice}")),
        AppError::Io(e) => AppError::Io(io::Error::new(e.kind(), format!("{e}. {notice}"))),
        other => {
            tracing::warn!("compositions de pack : message non transmis : {notice}");
            other
        }
    }
}

impl<'a> Store<'a> {
    pub fn new(dir: &'a Path) -> Self {
        Self::with_writer(dir, storage::write_atomic)
    }

    pub fn with_writer(dir: &'a Path, write: Writer) -> Self {
        Self { dir, write }
    }

    fn path(&self) -> PathBuf {
        self.dir.join(COMPOSITIONS_FILE)
    }

    fn set_aside(&self, why: &str) -> AppResult<StoreRead> {
        let backup = storage::backup_corrupted(&self.path())?;
        tracing::warn!(
            "compositions de pack : fichier {why}, mis de côté en {}",
            backup.display()
        );
        prune_backups(self.dir, backup.file_name().unwrap_or_default());
        Ok(StoreRead {
            notice: Some(format!(
                "Le fichier des compositions enregistrées ({why}) a été mis de côté ({}).",
                file_name(&backup)
            )),
            ..StoreRead::default()
        })
    }

    /// Lit les compositions. Absent : liste vide. Illisible ou trop gros : mis de côté, liste
    /// vide et message. D'un schéma plus récent : erreur, fichier intact (ni lu ni réécrit).
    /// Entrées écartées : fichier copié puis réécrit avec les entrées lisibles, message.
    pub fn read(&self) -> AppResult<StoreRead> {
        let path = self.path();
        match fs::metadata(&path) {
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(StoreRead::default()),
            Err(err) => return Err(err.into()),
            Ok(meta) if meta.len() > MAX_FILE_BYTES => return self.set_aside("trop volumineux"),
            Ok(_) => {}
        }
        let raw = match storage::load_versioned::<RawStore>(&path, COMPOSITIONS_SCHEMA, migrate) {
            Ok(Loaded::Missing) => return Ok(StoreRead::default()),
            Ok(Loaded::Found(raw)) => raw,
            Ok(Loaded::Recovered { backup }) => {
                // `backup` : nom rendu en `String` par `load_versioned`, exact ici (nom du
                // fichier et horodatage en ASCII, voir `storage::backup_path`).
                prune_backups(self.dir, OsStr::new(&backup));
                return Ok(StoreRead {
                    notice: Some(format!(
                        "Le fichier des compositions enregistrées était illisible : il a été mis de côté ({backup})."
                    )),
                    ..StoreRead::default()
                });
            }
            // Comme `settings.json` : un fichier d'une version plus récente n'est jamais
            // modifié ni remplacé (aucun chemin dans le message).
            Err(AppError::ConfigVersion {
                found, supported, ..
            }) => {
                return Err(AppError::ConfigVersion {
                    file: COMPOSITIONS_FILE.into(),
                    found,
                    supported,
                })
            }
            Err(err) => return Err(err),
        };
        let mut extra = raw.extra;
        extra.remove("schema_version");
        let mut entries: Vec<SavedComposition> = Vec::new();
        let mut ids = HashSet::new();
        let mut rejected = Rejected::default();
        for value in raw.compositions {
            let Some(entry) = SavedComposition::from_stored(value)
                .filter(|e| valid_id(&e.id) && checked_name(&e.name) == Some(e.name.as_str()))
            else {
                rejected.unreadable += 1;
                continue;
            };
            if ids.contains(&entry.id) || name_taken(&entries, &entry.name, None) {
                rejected.duplicate += 1;
            } else if entries.len() >= MAX_COMPOSITIONS {
                rejected.surplus += 1;
            } else {
                ids.insert(entry.id.clone());
                entries.push(entry);
            }
        }
        if rejected.total() == 0 {
            return Ok(StoreRead {
                entries,
                notice: None,
                extra,
                backup_pending: None,
            });
        }
        let (total, detail) = (rejected.total(), rejected.detail());
        // Copie intégrale d'abord (faite par l'écriture) : la réécriture ne perd rien.
        let mut read = StoreRead {
            entries,
            notice: None,
            extra,
            backup_pending: Some((total, detail.clone())),
        };
        let notice = match self.write_with_copy(&read) {
            Ok(backup) => {
                read.backup_pending = None;
                let backup = backup.unwrap_or_default();
                tracing::warn!(
                    "compositions de pack : {total} entrée(s) écartée(s) ({detail}), fichier copié en {}",
                    file_name(&backup)
                );
                copied_notice(total, &detail, &backup)
            }
            Err(err) => {
                // Copie retirée par l'écriture (l'original, intact, en tient lieu) ; elle sera
                // refaite avant toute écriture suivante (`backup_pending`).
                tracing::warn!("compositions de pack : réécriture impossible : {err}");
                format!(
                    "{total} composition(s) enregistrée(s) écartée(s) ({detail}) ; le fichier, laissé intact, n'a pas pu être réécrit."
                )
            }
        };
        Ok(StoreRead {
            notice: Some(notice),
            ..read
        })
    }

    /// Écrit les entrées de `read` (et ses champs inconnus) de façon atomique, sérialisées une
    /// fois et bornées. Si la copie en attente des entrées écartées (`backup_pending`) vient
    /// d'être faite, le message de `read` est remplacé : il ne dit plus que le fichier n'a pas pu
    /// être réécrit, mais nomme la copie (#489).
    fn write(&self, read: &mut StoreRead) -> AppResult<()> {
        let copy = self.write_with_copy(read)?;
        if let (Some((total, detail)), Some(copy)) = (read.backup_pending.take(), copy) {
            read.notice = Some(copied_notice(total, &detail, &copy));
        }
        Ok(())
    }

    /// [`Store::write`], précédée d'une copie du fichier d'origine si des entrées écartées
    /// n'ont pas encore de copie (`backup_pending`) : rien n'est réécrit sans elles tant que la
    /// copie n'existe pas. La copie est retirée si l'écriture échoue (l'original est intact),
    /// gardée sinon (et les plus anciennes élaguées). Renvoie la copie faite.
    fn write_with_copy(&self, read: &StoreRead) -> AppResult<Option<PathBuf>> {
        let compositions = read
            .entries
            .iter()
            .map(SavedComposition::to_stored)
            .collect::<AppResult<Vec<_>>>()?;
        let mut bytes = serde_json::to_vec_pretty(&StoreFile {
            schema_version: COMPOSITIONS_SCHEMA,
            compositions,
            extra: &read.extra,
        })?;
        bytes.push(b'\n');
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(AppError::Validation(format!(
                "fichier des compositions trop volumineux : {} octets (au plus {MAX_FILE_BYTES})",
                bytes.len()
            )));
        }
        let path = self.path();
        let copy = if read.backup_pending.is_some() {
            Some(backup_copy(&path)?)
        } else {
            None
        };
        match (self.write)(&path, &bytes) {
            Ok(()) => {
                if let Some(copy) = &copy {
                    prune_backups(self.dir, copy.file_name().unwrap_or_default());
                }
                Ok(copy)
            }
            Err(err) => {
                if let Some(copy) = &copy {
                    if let Err(err) = fs::remove_file(copy) {
                        tracing::warn!(
                            "compositions de pack : copie {} non retirée : {err}",
                            copy.display()
                        );
                    }
                }
                Err(err.into())
            }
        }
    }

    /// Conserve `composition`, créée sous le titre effectif `title`, à la date `now`. Identique
    /// à une entrée existante : cette entrée est mise à jour. Sinon : nouvelle entrée au nom
    /// libre dérivé du titre. Refusé au-delà de [`MAX_COMPOSITIONS`] ou de
    /// [`MAX_ENTRY_BYTES`] ; le message de la lecture suit alors l'erreur.
    pub fn record(
        &self,
        composition: &PackComposition,
        title: &str,
        now: u64,
    ) -> AppResult<Recorded> {
        let mut read = self.read()?;
        match self.record_in(&mut read, composition, title, now) {
            Ok((id, name, new)) => Ok(Recorded {
                id,
                name,
                new,
                notice: read.notice,
            }),
            Err(err) => Err(with_notice(err, read.notice)),
        }
    }

    /// [`Store::record`] sur des entrées déjà lues : `(id, nom, nouvelle)`. Le message de
    /// `read` est mis à jour si l'écriture fait la copie en attente.
    fn record_in(
        &self,
        read: &mut StoreRead,
        composition: &PackComposition,
        title: &str,
        now: u64,
    ) -> AppResult<(String, String, bool)> {
        let out = if let Some(entry) = read
            .entries
            .iter_mut()
            // Une entrée aux champs inconnus (version plus récente) n'est jamais « identique » :
            // la refaire lui ferait porter des réglages que ce pack n'avait pas.
            .find(|e| e.composition_extra.is_empty() && e.composition == *composition)
        {
            entry.last_used_at = now;
            entry.packs = entry.packs.saturating_add(1);
            (entry.id.clone(), entry.name.clone(), false)
        } else {
            if read.entries.len() >= MAX_COMPOSITIONS {
                return Err(AppError::Validation(format!(
                    "{MAX_COMPOSITIONS} compositions enregistrées au plus : supprimez-en une pour conserver les suivantes"
                )));
            }
            // Mesurée comme elle sera écrite (JSON indenté).
            let size = serde_json::to_vec_pretty(composition)?.len();
            if size > MAX_ENTRY_BYTES {
                return Err(AppError::Validation(format!(
                    "composition trop volumineuse : {size} octets (au plus {MAX_ENTRY_BYTES})"
                )));
            }
            let entry = SavedComposition {
                id: uuid::Uuid::new_v4().to_string(),
                name: unique_name(title, &read.entries),
                created_at: now,
                last_used_at: now,
                packs: 1,
                composition: composition.clone(),
                extra: Map::new(),
                composition_extra: Map::new(),
            };
            let out = (entry.id.clone(), entry.name.clone(), true);
            read.entries.push(entry);
            out
        };
        self.write(read)?;
        Ok(out)
    }

    fn find<'e>(entries: &'e [SavedComposition], id: &str) -> AppResult<&'e SavedComposition> {
        entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| AppError::NotFound("composition introuvable".into()))
    }

    /// Renomme une composition (nom nettoyé, unique sans tenir compte de la casse). Renvoie le
    /// message de la lecture (fichier mis de côté, entrées écartées), joint à l'erreur en cas
    /// d'échec.
    pub fn rename(&self, id: &str, name: &str) -> AppResult<Option<String>> {
        let Some(name) = checked_name(name) else {
            return Err(AppError::Validation(format!(
                "nom de composition vide ou trop long (1 à {MAX_NAME_CHARS} caractères)"
            )));
        };
        let mut read = self.read()?;
        let result = (|| {
            Self::find(&read.entries, id)?;
            if name_taken(&read.entries, name, Some(id)) {
                return Err(AppError::Validation(
                    "une autre composition porte déjà ce nom".into(),
                ));
            }
            if let Some(entry) = read.entries.iter_mut().find(|e| e.id == id) {
                if entry.name == name {
                    return Ok(());
                }
                entry.name = name.to_string();
            }
            self.write(&mut read)
        })();
        match result {
            Ok(()) => Ok(read.notice),
            Err(err) => Err(with_notice(err, read.notice)),
        }
    }

    /// Supprime une composition ; absente : rien à faire. Renvoie le message de la lecture.
    pub fn delete(&self, id: &str) -> AppResult<Option<String>> {
        let mut read = self.read()?;
        let before = read.entries.len();
        read.entries.retain(|e| e.id != id);
        if read.entries.len() == before {
            return Ok(read.notice);
        }
        match self.write(&mut read) {
            Ok(()) => Ok(read.notice),
            Err(err) => Err(with_notice(err, read.notice)),
        }
    }

    /// Copie une composition dans le brouillon unique, à la révision qui suit celle
    /// enregistrée : une sauvegarde automatique en retard ne peut pas l'écraser. Renvoie aussi
    /// le message de la lecture.
    pub fn open(&self, id: &str) -> AppResult<Opened> {
        let mut read = self.read()?;
        let mut notice = read.notice.take();
        let result = (|| {
            let entry = Self::find(&read.entries, id)?;
            let draft = pack::read_draft(self.dir)?;
            // Brouillon illisible, mis de côté : son message est dit aussi (une fois).
            if let Some(draft_notice) = draft.notice {
                notice = Some(match notice.take() {
                    Some(n) => format!("{n} {draft_notice}"),
                    None => draft_notice,
                });
            }
            let revision = draft
                .revision
                .checked_add(1)
                .filter(|r| *r <= MAX_REVISION)
                .ok_or_else(|| AppError::Validation("révision de brouillon épuisée".into()))?;
            pack::save_draft(self.dir, &entry.composition, revision)
        })();
        match result {
            Ok(outcome) => Ok(Opened {
                saved: outcome.saved,
                revision: outcome.revision,
                notice,
            }),
            Err(err) => Err(with_notice(err, notice)),
        }
    }
}

/// Résultat de [`Store::open`] (miroir de `CompositionOpened` dans `src/api/pack.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Opened {
    /// `false` : révision dépassée, rien n'est écrit (voir [`pack::SaveOutcome`]).
    pub saved: bool,
    pub revision: u64,
    /// Message de la lecture du fichier des compositions (mis de côté, entrées écartées).
    pub notice: Option<String>,
}

/// Résultat d'un renommage ou d'une suppression (miroir de `CompositionAction` dans
/// `src/api/pack.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ActionOutcome {
    /// Message de la lecture du fichier des compositions (mis de côté, entrées écartées).
    pub notice: Option<String>,
}

/// Résultat de [`Store::record`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    pub id: String,
    pub name: String,
    /// `false` : composition identique à une entrée existante (pack refait).
    pub new: bool,
    /// Message de la lecture (fichier mis de côté, entrées écartées).
    pub notice: Option<String>,
}

/// Conservation après `pack_create`, exposée au front (miroir de `CompositionRecord` dans
/// `src/api/install.ts`). Jamais une erreur : un échec est rapporté dans `error`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CompositionRecord {
    /// Nom sous lequel la composition est conservée ; `None` : non conservée (voir `error`).
    pub name: Option<String>,
    /// `false` : composition déjà conservée (pack refait).
    pub new: bool,
    /// Raison de la non-conservation, en français.
    pub error: Option<String>,
    /// Message de la lecture du fichier (mis de côté, entrées écartées).
    pub notice: Option<String>,
}

/// Conserve la composition d'un pack dont la tâche vient d'être créée. L'appelant tient le
/// verrou de configuration. Un échec ne remonte pas : la création du pack continue, et le
/// message de la lecture (fichier mis de côté, entrées écartées) est rapporté même alors.
pub fn record_after_create(
    dir: &Path,
    composition: &PackComposition,
    title: &str,
) -> CompositionRecord {
    record_after_create_in(&Store::new(dir), composition, title)
}

/// [`record_after_create`] sur un `Store` donné (écrivain injectable dans les tests).
fn record_after_create_in(
    store: &Store<'_>,
    composition: &PackComposition,
    title: &str,
) -> CompositionRecord {
    let (notice, result) = match store.read() {
        Ok(mut read) => {
            let result = store.record_in(&mut read, composition, title, now_secs());
            (read.notice, result)
        }
        Err(err) => (None, Err(err)),
    };
    match result {
        Ok((_, name, new)) => CompositionRecord {
            name: Some(name),
            new,
            error: None,
            notice,
        },
        Err(err) => {
            tracing::warn!("composition de pack non conservée : {err}");
            CompositionRecord {
                error: Some(err.to_string()),
                notice,
                ..CompositionRecord::default()
            }
        }
    }
}

/// Résumé d'une composition pour la liste (miroir de `CompositionSummary` dans
/// `src/api/pack.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompositionSummary {
    pub id: String,
    pub name: String,
    pub created_at: u64,
    pub last_used_at: u64,
    pub packs: u32,
    /// Épisodes de l'arborescence.
    pub episodes: usize,
    /// Flux cités (épisodes et couverture).
    pub feeds: usize,
    /// Flux cités dont la source a été supprimée (composition gardée, § 14.5) ; 0 si les
    /// sources n'ont pas pu être lues.
    pub missing_feeds: usize,
}

/// Liste des compositions (miroir de `CompositionsView` dans `src/api/pack.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompositionsView {
    /// Dernière utilisée d'abord.
    pub compositions: Vec<CompositionSummary>,
    pub notice: Option<String>,
    pub max: usize,
}

/// Vue de la liste ; `podcasts` : identifiants des sources podcast existantes, `None` si les
/// sources n'ont pas pu être lues (aucun flux marqué supprimé).
pub fn view(read: StoreRead, podcasts: Option<&HashSet<String>>) -> CompositionsView {
    let mut compositions: Vec<CompositionSummary> = read
        .entries
        .into_iter()
        .map(|e| {
            let sources: BTreeSet<String> = e.composition.source_ids();
            CompositionSummary {
                episodes: e.composition.episodes().len(),
                feeds: sources.len(),
                missing_feeds: podcasts
                    .map_or(0, |p| sources.iter().filter(|s| !p.contains(*s)).count()),
                id: e.id,
                name: e.name,
                created_at: e.created_at,
                last_used_at: e.last_used_at,
                packs: e.packs,
            }
        })
        .collect();
    compositions.sort_by(|a, b| {
        b.last_used_at
            .cmp(&a.last_used_at)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    CompositionsView {
        compositions,
        notice: read.notice,
        max: MAX_COMPOSITIONS,
    }
}

/// Liste des compositions de `dir`. L'appelant tient le verrou de configuration. Les sources
/// sont lues au mieux : un échec ne fait pas perdre la liste ni son message (dit une fois).
pub fn list_view(dir: &Path) -> AppResult<CompositionsView> {
    let read = Store::new(dir).read()?;
    let podcasts = match SourcesStore::new(dir).list() {
        Ok(sources) => Some(
            sources
                .into_iter()
                .filter_map(|s| match s {
                    Source::Podcast { id, .. } => Some(id),
                    _ => None,
                })
                .collect::<HashSet<String>>(),
        ),
        Err(err) => {
            tracing::warn!(
                "compositions de pack : sources illisibles, flux supprimés non comptés : {err}"
            );
            None
        }
    };
    Ok(view(read, podcasts.as_ref()))
}

// ---------------------------------------------------------------------------------------
// Commandes Tauri
// ---------------------------------------------------------------------------------------

/// Liste les compositions conservées, avec les flux supprimés de chacune.
#[tauri::command]
pub async fn pack_compositions_list(config: State<'_, ConfigState>) -> AppResult<CompositionsView> {
    let _guard = config.guard();
    list_view(&config.dir)
}

/// Renomme une composition.
#[tauri::command]
pub async fn pack_composition_rename(
    config: State<'_, ConfigState>,
    id: String,
    name: String,
) -> AppResult<ActionOutcome> {
    let _guard = config.guard();
    let notice = Store::new(&config.dir).rename(&id, &name)?;
    Ok(ActionOutcome { notice })
}

/// Supprime une composition (les histoires créées ne sont pas touchées).
#[tauri::command]
pub async fn pack_composition_delete(
    config: State<'_, ConfigState>,
    id: String,
) -> AppResult<ActionOutcome> {
    let _guard = config.guard();
    let notice = Store::new(&config.dir).delete(&id)?;
    Ok(ActionOutcome { notice })
}

/// Copie une composition dans le brouillon (le brouillon en cours est remplacé) ; le front
/// ouvre ensuite l'éditeur, qui relit le brouillon.
#[tauri::command]
pub async fn pack_composition_open(
    config: State<'_, ConfigState>,
    id: String,
) -> AppResult<Opened> {
    let _guard = config.guard();
    Store::new(&config.dir).open(&id)
}

#[cfg(test)]
mod tests;
