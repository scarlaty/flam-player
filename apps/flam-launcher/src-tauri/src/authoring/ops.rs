//! Opérations de l'éditeur sur le dossier d'une histoire, avec journal annuler/rétablir et
//! corbeille. Spécification : `docs/creation-dossier.md` § 4.2, § 4.4, § 4.5 et § 10.
//!
//! L'éditeur **modifie les fichiers de l'utilisateur** : chaque règle ci-dessous protège contre
//! une perte de données ou une écriture hors du dossier.
//!
//! # Invariants
//!
//! - **Cibles désignées par identifiant** : un id est le chemin relatif d'un élément tel que le
//!   rend `authoring::tree::scan` (composants séparés par `/`, racine `""`). Il est validé
//!   ([`parse_id`] : ni `.`/`..`, ni composant commençant par un point — donc jamais `.flam/` —,
//!   ni caractère interdit par Windows (`:` des flux alternatifs compris), ni nom réservé ou
//!   finissant par un point ou une espace), puis retrouvé dans une **nouvelle lecture** du
//!   dossier au moment d'agir. Seules exceptions : les **sources** d'une copie ou d'une image,
//!   chemins absolus venus de l'Explorateur, lues sans suivre de lien et jamais modifiées.
//! - **Racine revérifiée à chaque opération** : `check_root` (forme, racines interdites) doit
//!   rendre la même forme résolue qu'à l'ouverture, la racine n'est pas un lien et garde la même
//!   identité de fichier (volume + index) ; l'**identité** est aussi comparée à celle des
//!   dossiers interdits et de leurs parents ([`check_identity`]) : un partage administratif
//!   (`\\localhost\C$\Windows\…`) ne contourne pas le refus par chemin.
//! - **Chemins épinglés au moment d'agir** (TOCTOU) : la racine puis chaque dossier du chemin
//!   visé sont ouverts sans suivre de lien et sans partager la suppression (`fsx`) ; un
//!   composant qui est devenu un lien, une jonction ou autre chose qu'un dossier → refus. Tant
//!   que ces poignées sont ouvertes, aucun de ces dossiers ne peut être renommé ni remplacé.
//! - **État attendu vérifié** avant chaque action ([`Stamp`] : type, taille, date de
//!   modification ; présence seule pour un dossier) : sinon rien n'est fait et l'erreur
//!   [`OpError::Changed`] est rendue (§ 4.5).
//! - **Jamais d'écrasement** : renommages sans remplacement (`MoveFileExW` sans
//!   `MOVEFILE_REPLACE_EXISTING`), copies en `create_new` ; nom déjà pris → suffixe « (2) »,
//!   « (3) »… Pour un fichier audio, le nom est libre si aucune entrée du dossier n'a la même
//!   base (`<base>.<ext>`, `<base>.titre.<ext>`) : son image et son titre lu le suivent sous la
//!   même base, sans jamais s'apparier à un fichier existant.
//! - **Aucun effacement définitif** : supprimer, remplacer une image ou défaire une copie
//!   déplace vers `.flam/corbeille/<horodatage> <n>/` (même volume : renommage). Seuls sont
//!   effacés les fichiers de travail de l'éditeur (`.flam/tmp/`) et la capture d'un réglage
//!   identique octet pour octet au contenu gardé dans le journal. « Vider la corbeille »
//!   ([`Editor::empty_trash`]) l'envoie dans la Corbeille Windows, refusé hors disque fixe.
//! - **Réglages** : relus au moment d'agir (`settings::load_*`), modifiés par fusion (clés
//!   inconnues gardées), réécrits par renommage : le fichier en place est d'abord **capturé**
//!   dans la corbeille, comparé au contenu attendu (sinon remis en place, rien n'est fait), puis
//!   le nouveau fichier prend sa place. Un fichier abîmé reste dans la corbeille (§ 4.5) ; un
//!   format plus récent n'est jamais réécrit.
//! - **Journal** ([`Editor::undo`], [`Editor::redo`]) : chaque opération est une suite
//!   d'actions élémentaires réversibles (déplacement ↔ déplacement inverse, création ↔ mise à la
//!   corbeille, contenu JSON ↔ contenu précédent). Une opération ou une annulation qui échoue en
//!   cours de route défait ce qu'elle a déjà fait : le dossier reste dans l'état d'avant.
//!   Annuler une opération dont un fichier a changé depuis ne fait rien, le dit
//!   ([`OpError::NotUndoable`]) et la retire du journal.
//! - **Copies bornées** ([`ImportLimits`]) : profondeur, nombre de fichiers, taille cumulée,
//!   entrées lues ; formats acceptés seulement (§ 3.3) ; une source qui est un lien, qui est dans
//!   le dossier de l'histoire ou qui le contient est refusée. Une limite dépassée refuse toute
//!   la copie avant d'écrire quoi que ce soit.

mod edit;
mod fsx;

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use super::root::{check_root, win32_alters, ForbiddenRoots};
use super::settings::{
    name_key, Debut, Fin, FolderSettings, Mode, StorySettings, MAX_SETTINGS_BYTES,
};
use super::tree::{scan, Limits, Scan, ScanError, AUDIO_EXTENSIONS, IMAGE_EXTENSIONS};
use crate::install::studio::clean_line;

/// Dossier caché de l'éditeur, à la racine de l'histoire.
pub const EDITOR_DIR: &str = ".flam";
/// Corbeille de l'éditeur (§ 4.4), relative à la racine.
pub const TRASH_DIR: &str = ".flam/corbeille";
/// Fichiers de travail de l'éditeur, relatifs à la racine.
const WORK_DIR: &str = ".flam/tmp";
/// Taille maximale d'une image déposée.
pub const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Longueur maximale (caractères) d'un nom de fichier ou de dossier choisi dans l'éditeur.
const MAX_NAME_CHARS: usize = 120;
/// Opérations gardées dans le journal (les plus anciennes sont oubliées).
const MAX_JOURNAL_ENTRIES: usize = 200;
/// Octets de réglages gardés dans le journal (contenus avant/après).
const MAX_JOURNAL_BYTES: usize = 32 * 1024 * 1024;
/// Essais de suffixe « (n) » avant d'abandonner.
const MAX_SUFFIX: usize = 999;

// ---------------------------------------------------------------------------
// Types publics
// ---------------------------------------------------------------------------

/// État attendu d'un fichier ou d'un dossier (§ 4.5). Un dossier n'est comparé que par sa
/// présence (sa date change avec son contenu). Date en millisecondes (sûres en JavaScript) et
/// reste en nanosecondes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stamp {
    pub dir: bool,
    pub len: u64,
    pub modified_ms: u64,
    pub modified_ns: u32,
}

impl Stamp {
    fn of(meta: &fs::Metadata) -> Self {
        if meta.is_dir() {
            return Self {
                dir: true,
                len: 0,
                modified_ms: 0,
                modified_ns: 0,
            };
        }
        let since = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .unwrap_or_default();
        Self {
            dir: false,
            len: meta.len(),
            modified_ms: u64::try_from(since.as_millis()).unwrap_or(u64::MAX),
            modified_ns: since.subsec_nanos() % 1_000_000,
        }
    }

    fn matches(&self, other: &Stamp) -> bool {
        if self.dir || other.dir {
            self.dir == other.dir
        } else {
            self == other
        }
    }
}

/// Élément visé par une opération : son id et l'état vu par l'éditeur ([`Editor::stamp`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub id: String,
    pub stamp: Stamp,
}

/// Résultat d'une opération, d'une annulation ou d'un rétablissement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Done {
    /// Libellé de l'opération (« Suppression de « Le loup » »).
    pub label: String,
    /// Ids créés ou nouveaux ids des éléments touchés (copie, déplacement, renommage, nouveau
    /// menu), pour garder la sélection.
    pub ids: Vec<String>,
}

/// Bornes d'une copie depuis l'Explorateur (§ 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportLimits {
    /// Niveaux de dossiers sous un dossier déposé (le dossier déposé est le niveau 1).
    pub max_depth: usize,
    /// Fichiers copiés au plus.
    pub max_files: usize,
    /// Taille cumulée copiée au plus (octets).
    pub max_bytes: u64,
    /// Entrées de dossier lues au plus pendant la préparation.
    pub max_entries: usize,
}

impl Default for ImportLimits {
    fn default() -> Self {
        Self {
            max_depth: 5,
            max_files: 1_000,
            max_bytes: 4 * 1024 * 1024 * 1024,
            max_entries: 20_000,
        }
    }
}

/// Élément d'une source non copié.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Skipped {
    pub path: String,
    pub reason: &'static str,
}

/// Résultat d'une copie.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImportReport {
    pub done: Done,
    pub skipped: Vec<Skipped>,
}

/// Désérialise un champ « absent / effacé / valeur » : absent → `None`, `null` → `Some(None)`.
fn double<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// Modification des réglages d'un dossier : `None` garde la valeur, `Some(None)` l'efface (le
/// dossier hérite alors du parent), `Some(Some(v))` la fixe.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPatch {
    #[serde(default, deserialize_with = "double")]
    pub mode: Option<Option<Mode>>,
    #[serde(default, deserialize_with = "double")]
    pub fin: Option<Option<Fin>>,
    #[serde(default, deserialize_with = "double")]
    pub annoncer: Option<Option<bool>>,
}

/// Modification de `_histoire.json` (mêmes conventions que [`FolderPatch`]). `uuid` et
/// `version` ne se modifient pas ici (création de l'histoire, § 8).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoryPatch {
    #[serde(flatten)]
    pub folder: FolderPatch,
    #[serde(default, deserialize_with = "double")]
    pub titre: Option<Option<String>>,
    #[serde(default, deserialize_with = "double")]
    pub sous_titre: Option<Option<String>>,
    #[serde(default, deserialize_with = "double")]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "double")]
    pub age: Option<Option<u8>>,
    #[serde(default, deserialize_with = "double")]
    pub debut: Option<Option<Debut>>,
    #[serde(default, deserialize_with = "double")]
    pub titres_lus: Option<Option<Map<String, Value>>>,
}

/// Erreur typée d'une opération. Sauf [`OpError::Inconsistent`], le dossier est laissé tel
/// qu'il était avant l'appel.
#[derive(Debug, thiserror::Error)]
pub enum OpError {
    #[error(transparent)]
    Root(#[from] ScanError),
    #[error("le dossier de l'histoire a été déplacé ou remplacé : rouvrez-le")]
    RootChanged,
    #[error("identifiant d'élément refusé : « {0} »")]
    InvalidId(String),
    #[error("élément introuvable : « {0} »")]
    NotFound(String),
    #[error("« {0} » a changé depuis la dernière lecture du dossier : rien n'a été fait")]
    Changed(String),
    #[error("« {0} » est un lien, une jonction ou un fichier « à la demande » : refusé")]
    Link(String),
    #[error("{0}")]
    Refused(String),
    #[error("source « {path} » refusée : {reason}")]
    Source { path: String, reason: &'static str },
    #[error("copie refusée : {0}")]
    TooLarge(String),
    #[error("réglages non modifiés : {0}")]
    Settings(String),
    #[error("rien à annuler")]
    NothingToUndo,
    #[error("rien à rétablir")]
    NothingToRedo,
    #[error("impossible de défaire « {label} » : {reason} ; rien n'a été fait")]
    NotUndoable { label: String, reason: String },
    #[error("corbeille non vidée : {0}")]
    NoRecycleBin(&'static str),
    #[error("opération interrompue ({cause}) et défaite seulement en partie : {details}")]
    Inconsistent { cause: String, details: String },
    #[error("erreur d'entrée/sortie : {0}")]
    Io(#[from] io::Error),
}

impl OpError {
    /// Code stable pour le front.
    pub fn code(&self) -> &'static str {
        match self {
            OpError::Root(_) => "root_refused",
            OpError::RootChanged => "root_changed",
            OpError::InvalidId(_) => "invalid_id",
            OpError::NotFound(_) => "not_found",
            OpError::Changed(_) => "changed",
            OpError::Link(_) => "link",
            OpError::Refused(_) => "refused",
            OpError::Source { .. } => "source_refused",
            OpError::TooLarge(_) => "too_large",
            OpError::Settings(_) => "settings",
            OpError::NothingToUndo => "nothing_to_undo",
            OpError::NothingToRedo => "nothing_to_redo",
            OpError::NotUndoable { .. } => "not_undoable",
            OpError::NoRecycleBin(_) => "no_recycle_bin",
            OpError::Inconsistent { .. } => "inconsistent",
            OpError::Io(_) => "io",
        }
    }
}

impl From<OpError> for crate::AppError {
    fn from(e: OpError) -> Self {
        match e {
            OpError::Io(io) => crate::AppError::Io(io),
            other => crate::AppError::Validation(other.to_string()),
        }
    }
}

type OpResult<T> = Result<T, OpError>;

// ---------------------------------------------------------------------------
// Noms et identifiants
// ---------------------------------------------------------------------------

/// Caractères interdits dans un nom Windows (`:` : flux alternatifs).
fn forbidden_char(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c.is_control()
}

/// Composant d'id acceptable : simple nom, sans point initial, sans caractère interdit, ni
/// réservé ni finissant par un point ou une espace.
fn valid_component(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= 255
        && !name.starts_with('.')
        && !name.chars().any(forbidden_char)
        && !win32_alters(name)
}

/// Valide un id venu de l'extérieur (voir la doc du module). `""` désigne la racine.
pub fn parse_id(id: &str) -> OpResult<String> {
    if id.is_empty() {
        return Ok(String::new());
    }
    if id.split('/').all(valid_component) {
        Ok(id.to_owned())
    } else {
        Err(OpError::InvalidId(id.to_owned()))
    }
}

fn parent_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn name_of(rel: &str) -> &str {
    rel.rsplit_once('/').map_or(rel, |(_, name)| name)
}

fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// Nom sans la dernière extension ; un nom sans point (ou `.x`) est rendu entier.
fn stem_of(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// Dernière extension avec son point (`.mp3`), ou `""`.
fn ext_with_dot(name: &str) -> &str {
    &name[stem_of(name).len()..]
}

fn ext_key(name: &str) -> String {
    ext_with_dot(name).trim_start_matches('.').to_lowercase()
}

fn is_audio(name: &str) -> bool {
    AUDIO_EXTENSIONS.contains(&ext_key(name).as_str())
}

fn is_image(name: &str) -> bool {
    IMAGE_EXTENSIONS.contains(&ext_key(name).as_str())
}

/// Base d'appariement d'un nom (clé) : sans extension, sans `.titre` final.
fn base_key(name: &str) -> String {
    let key = name_key(name);
    let stem = stem_of(&key);
    stem.strip_suffix(".titre").unwrap_or(stem).to_owned()
}

/// Nom choisi par l'utilisateur rendu sûr pour Windows ; `None` s'il ne reste rien.
fn sanitize_name(text: &str) -> Option<String> {
    let line = clean_line(text, MAX_NAME_CHARS);
    let mapped: String = line
        .chars()
        .map(|c| if forbidden_char(c) { ' ' } else { c })
        .collect();
    let mut name = mapped.split_whitespace().collect::<Vec<_>>().join(" ");
    name = name
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .trim()
        .to_owned();
    if name.is_empty() {
        return None;
    }
    if win32_alters(&name) {
        name.push('_');
    }
    valid_component(&name).then_some(name)
}

/// Candidats successifs : `base`, `base (2)`, `base (3)`…
fn candidates(base: &str) -> impl Iterator<Item = String> + '_ {
    std::iter::once(base.to_owned()).chain((2..=MAX_SUFFIX).map(move |n| format!("{base} ({n})")))
}

/// Noms présents dans un dossier, vus par clé exacte et par base d'appariement.
struct Taken {
    keys: HashSet<String>,
    bases: HashSet<String>,
}

impl Taken {
    fn new<'a>(names: impl IntoIterator<Item = &'a String>) -> Self {
        let mut taken = Taken {
            keys: HashSet::new(),
            bases: HashSet::new(),
        };
        for name in names {
            taken.add(name);
        }
        taken
    }

    fn add(&mut self, name: &str) {
        self.keys.insert(name_key(name));
        self.bases.insert(base_key(name));
    }

    /// Base libre pour un groupe de fichiers (audio + image + titre lu) : aucune entrée de même
    /// base.
    fn free_base(&self, wanted: &str) -> OpResult<String> {
        candidates(wanted)
            .find(|c| !self.bases.contains(&name_key(c)) && !self.keys.contains(&name_key(c)))
            .ok_or_else(|| OpError::Refused(format!("aucun nom libre pour « {wanted} »")))
    }

    /// Nom libre pour un dossier.
    fn free_name(&self, wanted: &str) -> OpResult<String> {
        candidates(wanted)
            .find(|c| !self.keys.contains(&name_key(c)))
            .ok_or_else(|| OpError::Refused(format!("aucun nom libre pour « {wanted} »")))
    }
}

// ---------------------------------------------------------------------------
// Journal
// ---------------------------------------------------------------------------

/// Action élémentaire, réversible. Chemins relatifs à la racine, en `/`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Action {
    /// Renommage sans écrasement de `from` (dans l'état `stamp`) en `to` (absent).
    Move {
        from: String,
        to: String,
        stamp: Stamp,
    },
    /// `path` a été créé dans l'état `stamp` (copie, nouveau menu, réglage nouveau). Ne
    /// s'exécute pas : son inverse le met à la corbeille.
    Created { path: String, stamp: Stamp },
    /// Le contenu de `path` passe de `before` à `after` (réglages).
    Json {
        path: String,
        before: Vec<u8>,
        after: Vec<u8>,
    },
}

impl Action {
    fn bytes(&self) -> usize {
        match self {
            Action::Json { before, after, .. } => before.len() + after.len(),
            _ => 0,
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    label: String,
    ids: Vec<String>,
    actions: Vec<Action>,
}

impl Entry {
    fn bytes(&self) -> usize {
        self.actions.iter().map(Action::bytes).sum()
    }
}

#[derive(Debug, Default)]
struct Journal {
    undo: Vec<Entry>,
    redo: Vec<Entry>,
}

impl Journal {
    fn push_undo(&mut self, entry: Entry) {
        self.undo.push(entry);
        let mut total: usize = self.undo.iter().chain(&self.redo).map(Entry::bytes).sum();
        while self.undo.len() > MAX_JOURNAL_ENTRIES
            || (total > MAX_JOURNAL_BYTES && self.undo.len() > 1)
        {
            total -= self.undo.remove(0).bytes();
        }
    }
}

// ---------------------------------------------------------------------------
// Éditeur
// ---------------------------------------------------------------------------

/// Dossier d'histoire ouvert en écriture, avec son journal de session.
#[derive(Debug)]
pub struct Editor {
    /// Forme résolue et vérifiée de la racine.
    base: PathBuf,
    root_id: fsx::FileId,
    forbidden: ForbiddenRoots,
    /// Dossiers interdits ajoutés par l'appelant (pour `tree::scan`).
    extra: Vec<PathBuf>,
    limits: Limits,
    import_limits: ImportLimits,
    journal: Journal,
    /// `.flam/tmp/` déjà nettoyé pendant cette session.
    work_cleaned: bool,
}

/// Compteur des noms de travail et des places de corbeille (unicité dans le processus).
static COUNTER: AtomicU64 = AtomicU64::new(0);

fn next_counter() -> u64 {
    COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Identités de `path` et de ses parents (ceux qu'on peut ouvrir), `path` d'abord.
fn ancestor_ids(path: &Path) -> Vec<fsx::FileId> {
    path.ancestors()
        .filter_map(|p| fsx::open_entry(p, false).ok().map(|(_, info)| info.id))
        .collect()
}

fn forbidden(base: &Path, reason: &'static str) -> OpError {
    OpError::Root(ScanError::Forbidden {
        path: base.to_string_lossy().into_owned(),
        reason,
    })
}

/// Racines interdites comparées par **identité de fichier** (volume + index) et non plus par
/// chemin : `\\localhost\C$\Windows\x`, un nom court ou un autre alias du même dossier est
/// refusé comme `C:\Windows\x`. Mêmes familles que `check_root`.
fn check_identity(base: &Path, forbidden_roots: &ForbiddenRoots) -> OpResult<()> {
    let own = ancestor_ids(base);
    let Some(&base_id) = own.first() else {
        return Err(OpError::RootChanged);
    };
    for dir in &forbidden_roots.profile {
        if ancestor_ids(dir).contains(&base_id) {
            return Err(forbidden(
                base,
                "profil utilisateur ou dossier qui le contient",
            ));
        }
    }
    for dir in &forbidden_roots.trees {
        let theirs = ancestor_ids(dir);
        let inside = theirs.first().is_some_and(|id| own.contains(id));
        if inside || theirs.contains(&base_id) {
            return Err(forbidden(
                base,
                "dossier système, du launcher ou de la Flam (ou dossier qui le contient)",
            ));
        }
    }
    Ok(())
}

/// Lit `path` (au plus `max` octets) sans suivre de lien ; `None` s'il est absent.
fn read_bounded(path: &Path, max: u64, rel: &str) -> OpResult<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let file = crate::install::pack::open_no_follow(path).map_err(|e| match e {
        crate::AppError::Io(io) => OpError::Io(io),
        _ => OpError::Link(rel.to_owned()),
    })?;
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Réglages d'un dossier en cours de modification (racine : `_histoire.json`).
#[derive(Debug, Clone, PartialEq)]
enum Doc {
    Story(StorySettings),
    Folder(FolderSettings),
}

impl Doc {
    fn folder(&mut self) -> &mut FolderSettings {
        match self {
            Doc::Story(s) => &mut s.folder,
            Doc::Folder(f) => f,
        }
    }
}

/// Opération en cours : la racine reste épinglée, les actions faites sont gardées pour les
/// défaire en cas d'échec.
struct Tx<'e> {
    ed: &'e Editor,
    _root: fs::File,
    done: Vec<Action>,
}

impl Editor {
    /// Ouvre le dossier `root` en écriture : mêmes refus que `tree::scan` (racine interdite,
    /// lien, pas un dossier), plus la comparaison par identité de fichier ([`check_identity`]).
    /// N'écrit rien : `.flam/` n'est créé qu'à la première opération qui en a besoin (§ 8).
    pub fn open(
        root: &Path,
        forbidden_roots: ForbiddenRoots,
        extra: Vec<PathBuf>,
        limits: Limits,
    ) -> OpResult<Self> {
        let base = check_root(root, &forbidden_roots)?;
        let shown = || root.to_string_lossy().into_owned();
        let meta = fs::symlink_metadata(root).map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => OpError::Root(ScanError::NotFound(shown())),
            _ => OpError::Io(e),
        })?;
        if meta.file_type().is_symlink() {
            return Err(OpError::Root(ScanError::Link(shown())));
        }
        let (_, info) = fsx::open_entry(&base, false)?;
        if info.reparse {
            return Err(OpError::Root(ScanError::Link(shown())));
        }
        if !info.dir {
            return Err(OpError::Root(ScanError::NotADirectory(shown())));
        }
        check_identity(&base, &forbidden_roots)?;
        Ok(Self {
            base,
            root_id: info.id,
            forbidden: forbidden_roots,
            extra,
            limits,
            import_limits: ImportLimits::default(),
            journal: Journal::default(),
            work_cleaned: false,
        })
    }

    /// [`Editor::open`] avec les racines interdites du système et `extra` (dossier du
    /// launcher, lecteur de la Flam), comme `tree::scan`.
    pub fn open_default(root: &Path, extra: &[PathBuf]) -> OpResult<Self> {
        let forbidden_roots = ForbiddenRoots::system().with_trees(extra);
        Self::open(root, forbidden_roots, extra.to_vec(), Limits::default())
    }

    /// Change les bornes des copies.
    pub fn with_import_limits(mut self, limits: ImportLimits) -> Self {
        self.import_limits = limits;
        self
    }

    /// Forme résolue de la racine.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Lecture du dossier (`tree::scan`).
    pub fn scan(&self) -> OpResult<Scan> {
        Ok(scan(&self.base, &self.limits, &self.extra)?)
    }

    /// État actuel de l'élément `id`, à rendre avec la cible d'une opération.
    pub fn stamp(&self, id: &str) -> OpResult<Stamp> {
        let id = parse_id(id)?;
        let meta = self
            .lstat(&id)?
            .ok_or_else(|| OpError::NotFound(id.clone()))?;
        if is_link_meta(&meta) {
            return Err(OpError::Link(id));
        }
        Ok(Stamp::of(&meta))
    }

    pub fn can_undo(&self) -> bool {
        !self.journal.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.journal.redo.is_empty()
    }

    /// Libellé de l'opération que [`Editor::undo`] déferait.
    pub fn undo_label(&self) -> Option<&str> {
        self.journal.undo.last().map(|e| e.label.as_str())
    }

    /// Libellé de l'opération que [`Editor::redo`] referait.
    pub fn redo_label(&self) -> Option<&str> {
        self.journal.redo.last().map(|e| e.label.as_str())
    }

    // -- chemins ------------------------------------------------------------

    fn abs(&self, rel: &str) -> PathBuf {
        let mut path = self.base.clone();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            path.push(part);
        }
        path
    }

    fn lstat(&self, rel: &str) -> OpResult<Option<fs::Metadata>> {
        match fs::symlink_metadata(self.abs(rel)) {
            Ok(meta) => Ok(Some(meta)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Racine revérifiée (forme, racines interdites, identité) et épinglée.
    fn pin_root(&self) -> OpResult<fs::File> {
        let resolved = check_root(&self.base, &self.forbidden)?;
        if resolved != self.base {
            return Err(OpError::RootChanged);
        }
        let (file, info) = match fsx::open_entry(&self.base, true) {
            Ok(opened) => opened,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Err(OpError::RootChanged),
            Err(e) => return Err(e.into()),
        };
        if info.reparse || !info.dir || info.id != self.root_id {
            return Err(OpError::RootChanged);
        }
        check_identity(&self.base, &self.forbidden)?;
        Ok(file)
    }

    /// Épingle chaque dossier de `rel` (racine exclue, épinglée par l'opération), depuis la
    /// racine : chaque poignée est contrôlée (dossier, pas de lien) avant d'ouvrir la suivante.
    fn pin_dir(&self, rel: &str) -> OpResult<Vec<fs::File>> {
        let mut pins = Vec::new();
        let mut current = String::new();
        for part in rel.split('/').filter(|p| !p.is_empty()) {
            current = join(&current, part);
            let (file, info) = match fsx::open_entry(&self.abs(&current), true) {
                Ok(opened) => opened,
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    return Err(OpError::Changed(current))
                }
                Err(e) => return Err(e.into()),
            };
            if info.reparse {
                return Err(OpError::Link(current));
            }
            if !info.dir {
                return Err(OpError::Changed(current));
            }
            pins.push(file);
        }
        Ok(pins)
    }

    /// Crée (si besoin) le dossier interne `rel` (`.flam/…`) et le contrôle : un lien planté à
    /// sa place est refusé.
    fn ensure_internal_dir(&self, rel: &str) -> OpResult<()> {
        let mut current = String::new();
        for part in rel.split('/') {
            current = join(&current, part);
            match fs::create_dir(self.abs(&current)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(e.into()),
            }
            let meta = self
                .lstat(&current)?
                .ok_or_else(|| OpError::Changed(current.clone()))?;
            if is_link_meta(&meta) {
                return Err(OpError::Link(current));
            }
            if !meta.is_dir() {
                return Err(OpError::Changed(current));
            }
        }
        Ok(())
    }

    /// Nouvelle place vide dans la corbeille : `.flam/corbeille/<horodatage> <n>`.
    fn trash_slot(&self) -> OpResult<String> {
        self.ensure_internal_dir(TRASH_DIR)?;
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        let stamp = crate::storage::utc_stamp(secs);
        for _ in 0..MAX_SUFFIX {
            let slot = join(
                TRASH_DIR,
                &format!("{stamp} {}-{}", std::process::id(), next_counter()),
            );
            match fs::create_dir(self.abs(&slot)) {
                Ok(()) => return Ok(slot),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(OpError::Refused("corbeille saturée".into()))
    }

    /// Retire une fois par session les restes de `.flam/tmp/` (session interrompue) : fichiers
    /// de travail de l'éditeur seulement (les captures de réglages vont dans la corbeille,
    /// jamais ici). Appelé racine épinglée ; `.flam` épinglé et contrôlé (pas un lien).
    fn clean_work_once(&mut self) -> OpResult<()> {
        if self.work_cleaned {
            return Ok(());
        }
        if self.lstat(EDITOR_DIR)?.is_some() {
            let _pins = self.pin_dir(EDITOR_DIR)?;
            if let Some(meta) = self.lstat(WORK_DIR)? {
                if is_link_meta(&meta) {
                    return Err(OpError::Link(WORK_DIR.into()));
                }
                if meta.is_dir() {
                    let _ = fs::remove_dir_all(self.abs(WORK_DIR));
                }
            }
        }
        self.work_cleaned = true;
        Ok(())
    }

    /// Nouveau dossier de travail vide `.flam/tmp/<n>` (même volume que l'histoire).
    fn work_slot(&self) -> OpResult<String> {
        self.ensure_internal_dir(WORK_DIR)?;
        for _ in 0..MAX_SUFFIX {
            let slot = join(
                WORK_DIR,
                &format!("{}-{}", std::process::id(), next_counter()),
            );
            match fs::create_dir(self.abs(&slot)) {
                Ok(()) => return Ok(slot),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(OpError::Refused("dossier de travail saturé".into()))
    }

    fn drop_work_slot(&self, slot: &str) {
        let _ = fs::remove_dir_all(self.abs(slot));
    }

    /// Noms des entrées du dossier `dir` (toutes, cachées comprises).
    fn names(&self, dir: &str) -> OpResult<Vec<String>> {
        let mut names = Vec::new();
        for entry in fs::read_dir(self.abs(dir))? {
            names.push(entry?.file_name().to_string_lossy().into_owned());
        }
        Ok(names)
    }

    // -- exécution des actions ---------------------------------------------

    /// Exécute `action` après avoir vérifié l'état attendu.
    fn execute(&self, action: &Action) -> OpResult<()> {
        match action {
            Action::Move { from, to, stamp } => self.exec_move(from, to, stamp),
            Action::Json {
                path,
                before,
                after,
            } => self.exec_json(path, before, after),
            Action::Created { path, .. } => Err(OpError::Refused(format!(
                "action interne non rejouable : création de « {path} »"
            ))),
        }
    }

    fn exec_move(&self, from: &str, to: &str, stamp: &Stamp) -> OpResult<()> {
        // Place de corbeille retirée quand elle a été vidée (annuler) : recréée pour rétablir.
        let to_parent = parent_of(to);
        if parent_of(to_parent) == TRASH_DIR {
            self.ensure_internal_dir(to_parent)?;
        }
        let _from_pins = self.pin_dir(parent_of(from))?;
        let _to_pins = self.pin_dir(parent_of(to))?;
        let meta = self
            .lstat(from)?
            .ok_or_else(|| OpError::Changed(from.to_owned()))?;
        if is_link_meta(&meta) {
            return Err(OpError::Link(from.to_owned()));
        }
        if !Stamp::of(&meta).matches(stamp) {
            return Err(OpError::Changed(from.to_owned()));
        }
        if self.lstat(to)?.is_some() {
            return Err(OpError::Changed(to.to_owned()));
        }
        match fsx::rename_noreplace(&self.abs(from), &self.abs(to)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(OpError::Changed(to.to_owned()))
            }
            Err(e) => return Err(e.into()),
        }
        drop(_from_pins);
        // Place de corbeille vidée par ce retour : retirée (seulement si vide).
        let from_parent = parent_of(from);
        if parent_of(from_parent) == TRASH_DIR {
            let _ = fs::remove_dir(self.abs(from_parent));
        }
        Ok(())
    }

    /// Remplace le contenu `before` de `path` par `after`, sans jamais écraser un contenu
    /// inattendu : capture du fichier en place dans la corbeille, comparaison, puis mise en
    /// place du nouveau fichier.
    fn exec_json(&self, path: &str, before: &[u8], after: &[u8]) -> OpResult<()> {
        let _pins = self.pin_dir(parent_of(path))?;
        let current = read_bounded(&self.abs(path), MAX_SETTINGS_BYTES, path)?;
        if current.as_deref() != Some(before) {
            return Err(OpError::Changed(path.to_owned()));
        }
        // Fichier neuf préparé à côté (même dossier : renommage sur le même volume). Nom
        // commençant par un point : ignoré par la lecture du dossier.
        let staged = join(
            parent_of(path),
            &format!(
                ".{}.{}-{}.flam-tmp",
                name_of(path),
                std::process::id(),
                next_counter()
            ),
        );
        let staged_abs = self.abs(&staged);
        let write = (|| -> io::Result<()> {
            use std::io::Write;
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&staged_abs)?;
            file.write_all(after)?;
            file.sync_all()
        })();
        if let Err(e) = write {
            let _ = fs::remove_file(&staged_abs);
            return Err(e.into());
        }
        let result = self.swap_in(path, &staged, before);
        if result.is_err() {
            let _ = fs::remove_file(&staged_abs);
        }
        result
    }

    /// Capture `path` dans la corbeille, vérifie qu'il valait `before`, met `staged` à sa place
    /// et efface la capture (identique au contenu gardé dans le journal).
    fn swap_in(&self, path: &str, staged: &str, before: &[u8]) -> OpResult<()> {
        let slot = self.trash_slot()?;
        let captured = join(&slot, name_of(path));
        let undo_slot = || {
            let _ = fs::remove_dir(self.abs(&slot));
        };
        match fsx::rename_noreplace(&self.abs(path), &self.abs(&captured)) {
            Ok(()) => {}
            Err(e) => {
                undo_slot();
                return Err(if e.kind() == io::ErrorKind::NotFound {
                    OpError::Changed(path.to_owned())
                } else {
                    e.into()
                });
            }
        }
        let restore = |cause: OpError| -> OpError {
            match fsx::rename_noreplace(&self.abs(&captured), &self.abs(path)) {
                Ok(()) => {
                    undo_slot();
                    cause
                }
                Err(e) => OpError::Inconsistent {
                    cause: cause.to_string(),
                    details: format!(
                        "« {path} » est resté dans la corbeille sous « {captured} » ({e})"
                    ),
                },
            }
        };
        let captured_bytes = match read_bounded(&self.abs(&captured), MAX_SETTINGS_BYTES, path) {
            Ok(bytes) => bytes,
            Err(e) => return Err(restore(e)),
        };
        if captured_bytes.as_deref() != Some(before) {
            return Err(restore(OpError::Changed(path.to_owned())));
        }
        if let Err(e) = fsx::rename_noreplace(&self.abs(staged), &self.abs(path)) {
            let cause = if e.kind() == io::ErrorKind::AlreadyExists {
                OpError::Changed(path.to_owned())
            } else {
                e.into()
            };
            return Err(restore(cause));
        }
        let _ = fs::remove_file(self.abs(&captured));
        undo_slot();
        Ok(())
    }

    /// Action inverse de `action` (une création s'inverse en mise à la corbeille).
    fn inverse(&self, action: &Action) -> OpResult<Action> {
        Ok(match action {
            Action::Move { from, to, stamp } => Action::Move {
                from: to.clone(),
                to: from.clone(),
                stamp: *stamp,
            },
            Action::Json {
                path,
                before,
                after,
            } => Action::Json {
                path: path.clone(),
                before: after.clone(),
                after: before.clone(),
            },
            Action::Created { path, stamp } => {
                let slot = self.trash_slot()?;
                Action::Move {
                    from: path.clone(),
                    to: join(&slot, name_of(path)),
                    stamp: *stamp,
                }
            }
        })
    }

    /// Retire une place de corbeille restée vide (inverse non exécuté).
    fn discard_inverse(&self, action: &Action) {
        if let Action::Move { to, .. } = action {
            let parent = parent_of(to);
            if parent_of(parent) == TRASH_DIR {
                let _ = fs::remove_dir(self.abs(parent));
            }
        }
    }

    // -- opérations : cadre --------------------------------------------------

    /// Exécute `body` dans une opération : racine épinglée, actions défaites en cas d'échec,
    /// entrée du journal ajoutée (et « rétablir » vidé) si quelque chose a été fait.
    fn run<T>(
        &mut self,
        label: String,
        body: impl FnOnce(&mut Tx<'_>, &mut Vec<String>) -> OpResult<T>,
    ) -> OpResult<(T, Done)> {
        let mut ids = Vec::new();
        let (value, actions) = {
            let root = self.pin_root()?;
            self.clean_work_once()?;
            let mut tx = Tx {
                ed: self,
                _root: root,
                done: Vec::new(),
            };
            match body(&mut tx, &mut ids) {
                Ok(value) => (value, tx.done),
                Err(e) => return Err(tx.abort(e)),
            }
        };
        let done = Done {
            label: label.clone(),
            ids: ids.clone(),
        };
        if !actions.is_empty() {
            self.journal.redo.clear();
            self.journal.push_undo(Entry {
                label,
                ids,
                actions,
            });
        }
        Ok((value, done))
    }

    /// Défait la dernière opération du journal.
    pub fn undo(&mut self) -> OpResult<Done> {
        let entry = self.journal.undo.pop().ok_or(OpError::NothingToUndo)?;
        match self.replay_inverse(&entry) {
            Ok(redo) => {
                let done = Done {
                    label: entry.label.clone(),
                    ids: Vec::new(),
                };
                self.journal.redo.push(redo);
                Ok(done)
            }
            Err(e) => Err(self.not_undoable(entry, e, true)),
        }
    }

    /// Refait la dernière opération défaite.
    pub fn redo(&mut self) -> OpResult<Done> {
        let entry = self.journal.redo.pop().ok_or(OpError::NothingToRedo)?;
        match self.replay_inverse(&entry) {
            Ok(undo) => {
                let done = Done {
                    label: entry.label.clone(),
                    ids: undo.ids.clone(),
                };
                self.journal.push_undo(undo);
                Ok(done)
            }
            Err(e) => Err(self.not_undoable(entry, e, false)),
        }
    }

    /// Erreur d'annulation : l'entrée est retirée si l'état a changé (elle ne pourra plus être
    /// défaite), gardée si l'échec est passager (fichier verrouillé…).
    fn not_undoable(&mut self, entry: Entry, error: OpError, undo: bool) -> OpError {
        let keep = matches!(error, OpError::Io(_) | OpError::RootChanged);
        let label = entry.label.clone();
        if keep {
            if undo {
                self.journal.undo.push(entry);
            } else {
                self.journal.redo.push(entry);
            }
        }
        match error {
            OpError::Inconsistent { .. } | OpError::RootChanged | OpError::Root(_) => error,
            other => OpError::NotUndoable {
                label,
                reason: other.to_string(),
            },
        }
    }

    /// Exécute les inverses des actions de `entry`, de la dernière à la première ; rend
    /// l'entrée qui défait ce qui vient d'être fait.
    fn replay_inverse(&self, entry: &Entry) -> OpResult<Entry> {
        let root = self.pin_root()?;
        let mut tx = Tx {
            ed: self,
            _root: root,
            done: Vec::new(),
        };
        for action in entry.actions.iter().rev() {
            let inverse = match self.inverse(action) {
                Ok(inverse) => inverse,
                Err(e) => return Err(tx.abort(e)),
            };
            if let Err(e) = tx.apply(inverse) {
                return Err(tx.abort(e));
            }
        }
        Ok(Entry {
            label: entry.label.clone(),
            ids: entry.ids.clone(),
            actions: tx.done,
        })
    }

    // -- vider la corbeille ---------------------------------------------------

    /// Envoie le contenu de `.flam/corbeille/` dans la Corbeille Windows (§ 4.4, Q4) ; rend le
    /// nombre d'éléments envoyés. Refusé hors disque fixe (la suppression y serait définitive).
    /// Les opérations du journal qui en dépendent ne pourront plus être défaites (elles le
    /// diront).
    pub fn empty_trash(&mut self) -> OpResult<usize> {
        let root = self.pin_root()?;
        if let Some(reason) = fsx::recycle_bin_refusal(&self.base) {
            return Err(OpError::NoRecycleBin(reason));
        }
        let Some(meta) = self.lstat(TRASH_DIR)? else {
            return Ok(0);
        };
        if is_link_meta(&self.lstat(EDITOR_DIR)?.ok_or(OpError::RootChanged)?)
            || is_link_meta(&meta)
            || !meta.is_dir()
        {
            return Err(OpError::Link(TRASH_DIR.into()));
        }
        let mut paths = Vec::new();
        for name in self.names(TRASH_DIR)? {
            let rel = join(TRASH_DIR, &name);
            if self.lstat(&rel)?.is_some_and(|m| is_link_meta(&m)) {
                return Err(OpError::Link(rel));
            }
            paths.push(self.abs(&rel));
        }
        // La Corbeille Windows renomme les éléments : la racine ne doit pas rester épinglée.
        drop(root);
        fsx::recycle(&paths)?;
        Ok(paths.len())
    }
}

fn is_link_meta(meta: &fs::Metadata) -> bool {
    crate::install::pack::is_link(meta)
}

impl Tx<'_> {
    fn apply(&mut self, action: Action) -> OpResult<()> {
        if let Err(e) = self.ed.execute(&action) {
            self.ed.discard_inverse(&action);
            return Err(e);
        }
        self.done.push(action);
        Ok(())
    }

    fn record(&mut self, action: Action) {
        self.done.push(action);
    }

    /// Défait les actions faites, de la dernière à la première ; rend `cause`, ou
    /// [`OpError::Inconsistent`] si quelque chose n'a pas pu être défait.
    fn abort(&mut self, cause: OpError) -> OpError {
        let mut failures = Vec::new();
        while let Some(action) = self.done.pop() {
            let result =
                self.ed
                    .inverse(&action)
                    .and_then(|inverse| match self.ed.execute(&inverse) {
                        Ok(()) => Ok(()),
                        Err(e) => {
                            self.ed.discard_inverse(&inverse);
                            Err(e)
                        }
                    });
            if let Err(e) = result {
                failures.push(format!("{action:?} : {e}"));
            }
        }
        if failures.is_empty() {
            cause
        } else {
            OpError::Inconsistent {
                cause: cause.to_string(),
                details: failures.join(" ; "),
            }
        }
    }
}

#[cfg(test)]
mod tests;
