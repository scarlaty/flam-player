//! Lecture d'un dossier d'histoire : arbre de menus et d'histoires, et rapport de problèmes.
//! Spécification : `docs/creation-dossier.md` § 3, § 5.1, § 9 et § 10.
//!
//! Fonction **pure** au sens du launcher : ni Tauri, ni ffmpeg, ni écriture. [`scan`] lit le
//! dossier (noms, types et tailles des entrées, fichiers de réglages) et rend un [`Scan`] :
//! l'arbre ([`Node`]) et la liste des [`Problem`]. Le format réel des fichiers audio et images
//! n'est pas vérifié ici (seulement l'extension) : `probe` le fera à la création.
//!
//! # Règles de lecture (§ 3)
//!
//! - Un dossier = un menu (recueil) ou une partie (chapitres) ; ses sous-dossiers et ses
//!   fichiers audio (`mp3`, `m4a`, `aac`, `ogg`, `opus`, `wav`, `flac`) sont ses options.
//! - Fichiers spéciaux (noms comparés sans casse) : `_histoire.json`, `_couverture.<image>` à
//!   la racine ; `_dossier.json`, `_image.<image>` dans un sous-dossier ; `_titre.<audio>`
//!   partout ; `<nom>.<image>` et `<nom>.titre.<audio>` à côté de `<nom>.<audio>`. Un
//!   `<nom>.titre.<audio>` sans `<nom>.<audio>` voisin est une histoire ordinaire.
//! - Ignorés sans rien dire : noms commençant par `.` (dont la corbeille `.flam/` et les
//!   temporaires de `storage::write_atomic`), fichiers cachés ou système (attributs Windows ;
//!   sauf `_histoire.json` et `_dossier.json`, lus même cachés), `desktop.ini`, `Thumbs.db`,
//!   `.DS_Store`. Tout autre fichier non reconnu est signalé.
//! - Ordre : `ordre` des réglages, puis les autres entrées en tri naturel ([`natural_cmp`]).
//! - Titre : `titres` des réglages, sinon le nom sans extension ni préfixe de tri
//!   ([`strip_sort_prefix`]), nettoyé (`clean_line`).
//! - Noms comparés par leur clé `settings::name_key` (NFC puis minuscules) : `ordre`,
//!   `titres`, appariement image / titre lu avec le fichier audio.
//! - Mode : celui des réglages du dossier, sinon celui du parent ; à la racine, sinon le mode
//!   proposé ([`Scan::suggested_mode`], § 5.1). `fin` et `annoncer` héritent de même.
//!
//! # Invariants de sécurité (§ 10)
//!
//! - **Liens jamais suivis** : une entrée lien symbolique ou jonction (`FileType::is_symlink`,
//!   qui ne suit pas les liens et couvre sous Windows les points de réanalyse « substituts de
//!   nom », comme `folder.rs`) est ignorée et signalée ; la racine elle-même ne peut pas en
//!   être un. Les autres points de réanalyse (fichiers et dossiers OneDrive « à la demande »)
//!   restent lisibles : seuls leurs noms et tailles sont lus ici. Les réglages, eux, sont
//!   ouverts sans suivre de lien et refusent tout point de réanalyse (`authoring::settings`).
//! - **Parcours borné** ([`Limits`]) : au plus `max_entries` entrées lues en tout (au-delà le
//!   parcours s'arrête, [`Scan::truncated`]) et jamais plus de `max_menu_depth` niveaux ; un
//!   dossier trop profond n'est pas lu, seulement regardé pour savoir s'il contient de l'audio
//!   (budget global `max_peek_entries` ; épuisé → bloquant). Réglages lus dans un budget cumulé de
//!   [`SETTINGS_BUDGET`] octets. Les limites métier (profondeur avec audio, histoires, options
//!   par menu, taille de l'audio) dépassées sont des problèmes **bloquants** du rapport, pas des
//!   erreurs.
//! - **Racines interdites** (`authoring::root::check_root`, chemins résolus) : forme non prise
//!   en charge, racine d'un disque ou d'un partage, profil utilisateur ou l'un de ses parents,
//!   dossier Windows, Program Files, et les dossiers `extra_forbidden` de l'appelant (dossier
//!   du launcher, lecteur de la Flam), avec leurs parents et sous-dossiers → [`ScanError`].
//! - **Identifiants** : l'id d'un élément est son chemin relatif à la racine, composants
//!   séparés par `/` (racine : `""`), noms réels sur le disque. Le front ne transmet que ces
//!   ids ; les opérations (lot 6) les retrouvent dans un nouveau [`Scan`], jamais en
//!   construisant un chemin libre. N'ont pas d'id (ignorés et signalés `invalid_name`) : un nom
//!   qui n'est pas de l'Unicode valide, un nom finissant par un point ou une espace (Win32 le
//!   confondrait avec un autre fichier : `a.mp3.` désigne `a.mp3`).
//! - **Textes nettoyés** : titres sur une ligne, sans caractère de format, bornés.
//! - [`Scan::story`] ne garde pas `extra` (clés inconnues, valeurs brutes) : une opération qui
//!   réécrit les réglages relit le fichier (`settings::load_*`) au moment d'agir.
//!
//! # Invariants à tenir par les opérations (lot 6)
//!
//! La lecture et l'action sont séparées dans le temps : entre ce [`Scan`] et une opération, un
//! autre programme peut remplacer un dossier par un lien ou une jonction, ou un fichier par un
//! autre. Une opération ne se fie donc **jamais** à ce qu'a vu le scan : au moment d'agir, elle
//! revérifie la racine ([`check_root`] à nouveau) puis **chaque composant** du chemin, depuis
//! la racine, sans suivre de lien (`symlink_metadata`, poignées ouvertes avec
//! `FILE_FLAG_OPEN_REPARSE_POINT` comme `install::pack::open_no_follow`), et l'état attendu
//! (taille, date) avant toute écriture (§ 4.5).
//!
//! # Codes de problèmes (stables)
//!
//! Bloquants : `no_story`, `too_deep` (dossier trop profond contenant de l'audio),
//! `too_many_entries`, `too_many_stories`, `too_many_options`, `audio_too_large`,
//! `unreadable_folder`.
//!
//! Avertissements : `too_deep` (dossier trop profond sans audio), `link_ignored`,
//! `invalid_name`, `unsupported_file`, `misplaced_special_file`, `duplicate_special_file`,
//! `orphan_image`, `duplicate_story_name`, `empty_menu`, `title_too_long`, `fin_ignored`, et
//! ceux des réglages (`settings_unreadable`, `settings_invalid_value`,
//! `settings_newer_format`).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use telmi2flam::text::{text as font_text, LABEL_MAX};
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;

pub use super::root::{check_root, ForbiddenRoots};
use super::settings::{
    self, name_key, FolderSettings, Inherited, Loaded, Mode, SettingsWarning, StorySettings,
    FOLDER_FILE, STORY_FILE,
};
use crate::install::studio::clean_line;
use crate::story::MAX_TITLE_CHARS;

/// Budget cumulé de lecture des fichiers de réglages pendant un parcours (octets).
pub const SETTINGS_BUDGET: u64 = 16 * 1024 * 1024;

/// Extensions audio acceptées (§ 3.3).
pub const AUDIO_EXTENSIONS: [&str; 7] = ["mp3", "m4a", "aac", "ogg", "opus", "wav", "flac"];
/// Extensions d'images acceptées (§ 3.3).
pub const IMAGE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "bmp", "webp"];
/// Fichiers ignorés sans rien dire (comparaison sans casse).
const SILENT_FILES: [&str; 3] = ["desktop.ini", "thumbs.db", ".ds_store"];

/// Bornes du parcours et limites métier (§ 9). Valeurs par défaut : celles visées par la note,
/// à confirmer par les essais sur la Flam (§ 13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Limits {
    /// Niveaux de menus, racine comprise.
    pub max_menu_depth: usize,
    /// Histoires (ou chapitres) au total.
    pub max_stories: usize,
    /// Options (histoires et sous-menus) par menu.
    pub max_options_per_menu: usize,
    /// Entrées de dossier lues au plus (toutes, ignorées comprises) : borne du parcours.
    pub max_entries: usize,
    /// Taille cumulée des fichiers audio (octets).
    pub max_audio_bytes: u64,
    /// Entrées regardées au plus, en tout, dans les dossiers trop profonds pour y chercher de
    /// l'audio ; budget épuisé → dossier supposé contenir de l'audio (bloquant).
    pub max_peek_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_menu_depth: 5,
            max_stories: 300,
            max_options_per_menu: 50,
            max_entries: 10_000,
            max_audio_bytes: 4 * 1024 * 1024 * 1024,
            max_peek_entries: 10_000,
        }
    }
}

/// Gravité d'un problème.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Empêche la création de l'histoire.
    Blocking,
    /// Signalé, sans empêcher la création.
    Warning,
}

/// Une ligne du rapport.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    /// Code stable `snake_case` (voir la doc du module).
    pub code: &'static str,
    pub severity: Severity,
    /// Chemin relatif à la racine, en `/` (racine : `""`).
    pub path: String,
    /// Explication lisible.
    pub message: String,
}

/// Élément de l'arbre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Node {
    /// Dossier : menu (recueil) ou partie (chapitres). La racine est un `Menu` d'id `""`.
    Menu {
        id: String,
        /// Nom du dossier (racine : nom du dossier choisi).
        dir: String,
        title: String,
        /// Mode effectif (réglé ou hérité).
        mode: Mode,
        /// Fin effective (réglée ou héritée, ramenée à `menu` si elle n'a pas de sens dans le
        /// mode).
        fin: settings::Fin,
        /// Annonce du titre avant un chapitre (effective).
        annoncer: bool,
        /// Id de `_image.*` (racine : `_couverture.*`).
        image: Option<String>,
        /// Id de `_titre.*`.
        title_audio: Option<String>,
        children: Vec<Node>,
    },
    /// Fichier audio : histoire (recueil) ou chapitre.
    Story {
        id: String,
        /// Nom du fichier.
        file: String,
        title: String,
        /// Id de `<nom>.<image>`.
        image: Option<String>,
        /// Id de `<nom>.titre.<audio>`.
        title_audio: Option<String>,
        /// Taille du fichier (octets).
        bytes: u64,
    },
}

impl Node {
    /// Id de l'élément.
    pub fn id(&self) -> &str {
        match self {
            Node::Menu { id, .. } | Node::Story { id, .. } => id,
        }
    }

    /// Titre affiché.
    pub fn title(&self) -> &str {
        match self {
            Node::Menu { title, .. } | Node::Story { title, .. } => title,
        }
    }

    /// Options d'un menu (vide pour une histoire).
    pub fn children(&self) -> &[Node] {
        match self {
            Node::Menu { children, .. } => children,
            Node::Story { .. } => &[],
        }
    }

    /// Cherche l'élément d'id `id` dans ce sous-arbre.
    pub fn find(&self, id: &str) -> Option<&Node> {
        if self.id() == id {
            return Some(self);
        }
        self.children().iter().find_map(|child| child.find(id))
    }
}

/// Résultat de [`scan`].
#[derive(Debug, Clone, PartialEq)]
pub struct Scan {
    /// Dossier lu : forme résolue et vérifiée de la racine ([`check_root`]). Base des ids pour
    /// les opérations (lot 6), qui la revérifient au moment d'agir.
    pub base: PathBuf,
    /// Racine : `Node::Menu` d'id `""`.
    pub root: Node,
    /// `_histoire.json` lu.
    pub story: Loaded<StorySettings>,
    /// Mode proposé (§ 5.1) quand `_histoire.json` ne fixe pas `mode` ; `None` sinon. Le mode
    /// effectif de la racine est alors ce mode proposé.
    pub suggested_mode: Option<Mode>,
    pub problems: Vec<Problem>,
    /// Histoires trouvées.
    pub story_count: usize,
    /// Taille cumulée des fichiers audio.
    pub audio_bytes: u64,
    /// Entrées de dossier lues.
    pub entries_seen: usize,
    /// Parcours arrêté par `max_entries` : l'arbre est incomplet.
    pub truncated: bool,
}

impl Scan {
    /// Vrai si un problème bloquant empêche la création.
    pub fn is_blocked(&self) -> bool {
        self.problems
            .iter()
            .any(|p| p.severity == Severity::Blocking)
    }
}

/// Dossier refusé ou illisible.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("« {0} » n'est pas un chemin de dossier absolu")]
    NotAbsolute(String),
    #[error("dossier introuvable : « {0} »")]
    NotFound(String),
    #[error("« {0} » n'est pas un dossier")]
    NotADirectory(String),
    #[error("« {0} » est un lien ou une jonction : refusé")]
    Link(String),
    #[error("« {path} » ne peut pas être le dossier d'une histoire : {reason}")]
    Forbidden { path: String, reason: &'static str },
    #[error("dossier illisible : {0}")]
    Io(#[from] std::io::Error),
}

impl From<ScanError> for crate::AppError {
    fn from(e: ScanError) -> Self {
        match e {
            ScanError::Io(io) => crate::AppError::Io(io),
            other => crate::AppError::Validation(other.to_string()),
        }
    }
}

// ---------------------------------------------------------------------------
// Tri et titres
// ---------------------------------------------------------------------------

/// Texte replié pour le tri : décomposé, sans diacritiques, en minuscules.
fn fold(text: &str) -> Vec<char> {
    text.nfd()
        .filter(|&c| canonical_combining_class(c) == 0)
        .flat_map(char::to_lowercase)
        .collect()
}

/// Tri naturel : suites de chiffres comparées comme des nombres (`2` avant `10`), le reste sans
/// casse ni accents ; à égalité, comparaison exacte (ordre total et stable).
pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let (fa, fb) = (fold(a), fold(b));
    let (mut i, mut j) = (0, 0);
    while i < fa.len() && j < fb.len() {
        if fa[i].is_ascii_digit() && fb[j].is_ascii_digit() {
            let run = |s: &[char], mut k: usize| {
                let start = k;
                while k < s.len() && s[k].is_ascii_digit() {
                    k += 1;
                }
                (start, k)
            };
            let ((sa, ea), (sb, eb)) = (run(&fa, i), run(&fb, j));
            let trim = |s: &[char], start: usize, end: usize| {
                let mut k = start;
                while k + 1 < end && s[k] == '0' {
                    k += 1;
                }
                s[k..end].to_vec()
            };
            let (na, nb) = (trim(&fa, sa, ea), trim(&fb, sb, eb));
            let order = na.len().cmp(&nb.len()).then_with(|| na.cmp(&nb));
            if order != Ordering::Equal {
                return order;
            }
            i = ea;
            j = eb;
        } else {
            let order = fa[i].cmp(&fb[j]);
            if order != Ordering::Equal {
                return order;
            }
            i += 1;
            j += 1;
        }
    }
    (fa.len() - i).cmp(&(fb.len() - j)).then_with(|| a.cmp(b))
}

/// Retire le préfixe de tri `^\d+\s*[-._)]?\s*` (chiffres ASCII) : « 01 - Le loup » → « Le
/// loup ». Un nom qui ne commence pas par un chiffre (« -5 degrés ») est rendu tel quel.
pub fn strip_sort_prefix(name: &str) -> &str {
    let rest = name.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() == name.len() {
        return name;
    }
    let rest = rest.trim_start();
    let rest = rest.strip_prefix(['-', '.', '_', ')']).unwrap_or(rest);
    rest.trim_start()
}

/// Nom sans la dernière extension (`.mp3`) ; un nom sans point (ou `.x`) est rendu entier.
fn stem_of(name: &str) -> &str {
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    }
}

/// Extension en minuscules (sans le point), ou `""`.
fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_lowercase(),
        _ => String::new(),
    }
}

/// Titre déduit d'un nom (fichier : sans extension ; dossier : entier), sans préfixe de tri.
pub fn title_from_name(name: &str, is_file: bool) -> String {
    let base = if is_file { stem_of(name) } else { name };
    let title = clean_line(strip_sort_prefix(base), MAX_TITLE_CHARS);
    if !title.is_empty() {
        return title;
    }
    let title = clean_line(base, MAX_TITLE_CHARS);
    if title.is_empty() {
        clean_line(name, MAX_TITLE_CHARS)
    } else {
        title
    }
}

/// Le nom commence par un chiffre (fichier « numéroté », § 5.1).
fn is_numbered(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_digit())
}

// ---------------------------------------------------------------------------
// Parcours
// ---------------------------------------------------------------------------

fn child_id(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// Dossier lu, avant l'application des modes.
struct RawDir {
    id: String,
    name: String,
    title: String,
    settings: FolderSettings,
    /// Id du fichier de réglages lu (nom réel sur le disque).
    settings_id: Option<String>,
    image: Option<String>,
    title_audio: Option<String>,
    items: Vec<RawItem>,
}

enum RawItem {
    Dir(RawDir),
    Story(RawStory),
}

struct RawStory {
    id: String,
    file: String,
    title: String,
    image: Option<String>,
    title_audio: Option<String>,
    bytes: u64,
}

impl RawItem {
    fn name(&self) -> &str {
        match self {
            RawItem::Dir(d) => &d.name,
            RawItem::Story(s) => &s.file,
        }
    }
}

impl RawDir {
    fn story_count(&self) -> usize {
        self.items
            .iter()
            .map(|item| match item {
                RawItem::Dir(d) => d.story_count(),
                RawItem::Story(_) => 1,
            })
            .sum()
    }
}

struct Walker<'a> {
    limits: &'a Limits,
    problems: Vec<Problem>,
    entries: usize,
    stories: usize,
    audio_bytes: u64,
    truncated: bool,
    /// Octets de réglages encore lisibles.
    settings_budget: u64,
    /// Entrées encore regardables dans les dossiers trop profonds.
    peek_left: usize,
    /// `_histoire.json` (lu pendant la lecture de la racine).
    story: Loaded<StorySettings>,
}

/// Nom de fichier de réglages (lu même caché).
fn is_settings_name(lower: &str) -> bool {
    lower == STORY_FILE || lower == FOLDER_FILE
}

/// Entrée de dossier retenue après les filtres.
enum Entry {
    Dir(String),
    File(String, u64),
}

impl Walker<'_> {
    fn problem(&mut self, code: &'static str, severity: Severity, path: &str, message: String) {
        self.problems.push(Problem {
            code,
            severity,
            path: path.to_owned(),
            message,
        });
    }

    fn warn(&mut self, code: &'static str, path: &str, message: String) {
        self.problem(code, Severity::Warning, path, message);
    }

    fn settings_problems(&mut self, path: &str, warnings: &[SettingsWarning]) {
        for w in warnings {
            self.warn(w.code, path, w.message.clone());
        }
    }

    /// Entrées de `dir` retenues (liens, cachés et noms invalides écartés), bornées.
    fn list(&mut self, dir: &Path, id: &str) -> Vec<Entry> {
        let read = match std::fs::read_dir(dir) {
            Ok(read) => read,
            Err(e) => {
                self.problem(
                    "unreadable_folder",
                    Severity::Blocking,
                    id,
                    format!("dossier illisible : {e}"),
                );
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for entry in read {
            if self.truncated {
                break;
            }
            self.entries += 1;
            if self.entries > self.limits.max_entries {
                self.truncated = true;
                self.problem(
                    "too_many_entries",
                    Severity::Blocking,
                    id,
                    format!(
                        "lecture arrêtée après {} fichiers et dossiers : dossier trop chargé",
                        self.limits.max_entries
                    ),
                );
                break;
            }
            let Ok(entry) = entry else { continue };
            let name = match entry.file_name().into_string() {
                Ok(name) => name,
                Err(raw) => {
                    let shown = raw.to_string_lossy().into_owned();
                    self.warn(
                        "invalid_name",
                        &child_id(id, &shown),
                        format!("« {shown} » : nom illisible, ignoré"),
                    );
                    continue;
                }
            };
            let lower = name.to_lowercase();
            if name.starts_with('.') || SILENT_FILES.contains(&lower.as_str()) {
                continue;
            }
            if name.ends_with(['.', ' ']) {
                self.warn(
                    "invalid_name",
                    &child_id(id, &name),
                    format!(
                        "« {name} » ignoré : un nom finissant par un point ou une espace n'est \
                         pas accessible de façon sûre sous Windows"
                    ),
                );
                continue;
            }
            // `file_type` et `DirEntry::metadata` ne suivent pas les liens.
            let (Ok(file_type), Ok(meta)) = (entry.file_type(), entry.metadata()) else {
                continue;
            };
            if is_hidden(&meta) && !is_settings_name(&lower) {
                continue;
            }
            let entry_id = child_id(id, &name);
            // Sous Windows, `is_symlink` couvre les points de réanalyse « substituts de nom »
            // (liens symboliques et jonctions), comme `folder.rs` ; un dossier OneDrive
            // (balise « cloud ») reste lisible.
            if file_type.is_symlink() {
                self.warn(
                    "link_ignored",
                    &entry_id,
                    format!("« {name} » est un lien ou une jonction : ignoré"),
                );
                continue;
            }
            if file_type.is_dir() {
                out.push(Entry::Dir(name));
            } else if file_type.is_file() {
                out.push(Entry::File(name, meta.len()));
            }
        }
        out
    }

    /// Lit le dossier `path` (id `id`, niveau `depth`, racine = 1).
    fn read_dir(&mut self, path: &Path, id: &str, name: &str, depth: usize) -> RawDir {
        let is_root = depth == 1;
        let entries = self.list(path, id);

        let mut stories: Vec<(String, u64)> = Vec::new();
        let mut titled: Vec<(String, u64)> = Vec::new(); // `<nom>.titre.<audio>`
        let mut images: Vec<String> = Vec::new();
        let mut special_image: Vec<String> = Vec::new();
        let mut special_title: Vec<String> = Vec::new();
        let mut settings_file: Option<String> = None;
        let mut subdirs: Vec<String> = Vec::new();

        for entry in entries {
            let (file, bytes) = match entry {
                Entry::Dir(dir) => {
                    subdirs.push(dir);
                    continue;
                }
                Entry::File(file, bytes) => (file, bytes),
            };
            let key = name_key(&file);
            let ext = ext_of(&key);
            let stem = stem_of(&key);
            let is_audio = AUDIO_EXTENSIONS.contains(&ext.as_str());
            let is_image = IMAGE_EXTENSIONS.contains(&ext.as_str());
            let entry_id = child_id(id, &file);
            let misplaced = |w: &mut Self, hint: &str| {
                w.warn(
                    "misplaced_special_file",
                    &entry_id,
                    format!("« {file} » ignoré : {hint}"),
                );
            };
            if key == STORY_FILE {
                if is_root {
                    settings_file = Some(file);
                } else {
                    misplaced(self, "seulement à la racine de l'histoire");
                }
            } else if key == FOLDER_FILE {
                if is_root {
                    misplaced(self, "à la racine, ces réglages vont dans _histoire.json");
                } else {
                    settings_file = Some(file);
                }
            } else if stem == "_couverture" && is_image {
                if is_root {
                    special_image.push(file);
                } else {
                    misplaced(self, "la couverture va à la racine de l'histoire");
                }
            } else if stem == "_image" && is_image {
                if is_root {
                    misplaced(self, "à la racine, l'image est _couverture");
                } else {
                    special_image.push(file);
                }
            } else if stem == "_titre" && is_audio {
                special_title.push(file);
            } else if is_audio {
                if stem.ends_with(".titre") {
                    titled.push((file, bytes));
                } else {
                    stories.push((file, bytes));
                }
            } else if is_image {
                images.push(file);
            } else {
                self.warn(
                    "unsupported_file",
                    &entry_id,
                    format!("« {file} » ignoré : format non pris en charge"),
                );
            }
        }

        // `<nom>.titre.<audio>` sans `<nom>.<audio>` voisin : une histoire ordinaire.
        let story_stems: HashSet<String> = stories
            .iter()
            .map(|(file, _)| stem_of(&name_key(file)).to_owned())
            .collect();
        let mut titles_by_stem: HashMap<String, Vec<String>> = HashMap::new();
        for (file, bytes) in titled {
            let key = name_key(&file);
            let base = stem_of(&key)
                .strip_suffix(".titre")
                .unwrap_or_default()
                .to_owned();
            if story_stems.contains(&base) {
                titles_by_stem.entry(base).or_default().push(file);
            } else {
                stories.push((file, bytes));
            }
        }
        let mut images_by_stem: HashMap<String, Vec<String>> = HashMap::new();
        for file in images {
            let stem = stem_of(&name_key(&file)).to_owned();
            images_by_stem.entry(stem).or_default().push(file);
        }

        let image = self.pick(id, special_image, true);
        let title_audio = self.pick(id, special_title, true);

        // Réglages du dossier (fichier lu sous son nom réel).
        let settings_id = settings_file.as_deref().map(|file| child_id(id, file));
        let settings = match (&settings_file, &settings_id) {
            (Some(file), Some(file_id)) if is_root => {
                let loaded = settings::load_story_file(&path.join(file), &mut self.settings_budget);
                self.settings_problems(file_id, &loaded.warnings);
                self.story = loaded;
                self.story.settings.folder.clone()
            }
            (Some(file), Some(file_id)) => {
                let loaded =
                    settings::load_folder_file(&path.join(file), &mut self.settings_budget);
                self.settings_problems(file_id, &loaded.warnings);
                loaded.settings
            }
            _ => FolderSettings::default_new(),
        };
        let titles = settings.title_table();

        // Histoires et appariements.
        let mut items = Vec::new();
        let mut stem_seen: HashMap<String, usize> = HashMap::new();
        for (file, _) in &stories {
            *stem_seen
                .entry(stem_of(&name_key(file)).to_owned())
                .or_default() += 1;
        }
        let mut used_images = HashSet::new();
        let mut used_titles = HashSet::new();
        for (file, bytes) in stories {
            let key = name_key(&file);
            let stem = stem_of(&key).to_owned();
            let story_id = child_id(id, &file);
            if stem_seen.get(&stem).copied().unwrap_or(0) > 1 {
                self.warn(
                    "duplicate_story_name",
                    &story_id,
                    format!(
                        "« {file} » : un autre fichier audio porte le même nom, image et titre lu \
                         partagés"
                    ),
                );
            }
            let image = images_by_stem.get(&stem).cloned().and_then(|names| {
                let first_use = used_images.insert(stem.clone());
                self.pick(id, names, first_use)
            });
            let title_audio = titles_by_stem.get(&stem).cloned().and_then(|names| {
                let first_use = used_titles.insert(stem.clone());
                self.pick(id, names, first_use)
            });
            let title = titles
                .get(&file)
                .map(str::to_owned)
                .unwrap_or_else(|| title_from_name(&file, true));
            self.stories += 1;
            self.audio_bytes = self.audio_bytes.saturating_add(bytes);
            items.push(RawItem::Story(RawStory {
                id: story_id,
                file,
                title,
                image,
                title_audio,
                bytes,
            }));
        }
        // Chaque `<nom>.titre.<audio>` retenu a son histoire (voir plus haut) : seules des images
        // peuvent rester orphelines.
        let mut orphans: Vec<String> = images_by_stem
            .iter()
            .filter(|(stem, _)| !used_images.contains(*stem))
            .flat_map(|(_, files)| files.iter().cloned())
            .collect();
        orphans.sort_by(|a, b| natural_cmp(a, b));
        for file in orphans {
            self.warn(
                "orphan_image",
                &child_id(id, &file),
                format!("« {file} » ignoré : aucun fichier audio du même nom"),
            );
        }

        // Sous-dossiers.
        subdirs.sort_by(|a, b| natural_cmp(a, b));
        for dir in subdirs {
            let dir_id = child_id(id, &dir);
            if self.truncated {
                break;
            }
            if depth + 1 > self.limits.max_menu_depth {
                let max = self.limits.max_menu_depth;
                if may_contain_audio(&path.join(&dir), &mut self.peek_left) {
                    self.problem(
                        "too_deep",
                        Severity::Blocking,
                        &dir_id,
                        format!("« {dir} » dépasse {max} niveaux de menus : non lu"),
                    );
                } else {
                    self.warn(
                        "too_deep",
                        &dir_id,
                        format!("« {dir} » dépasse {max} niveaux de menus, sans audio : ignoré"),
                    );
                }
                continue;
            }
            let mut sub = self.read_dir(&path.join(&dir), &dir_id, &dir, depth + 1);
            if let Some(title) = titles.get(&dir) {
                sub.title = title.to_owned();
            }
            items.push(RawItem::Dir(sub));
        }

        sort_items(&mut items, &settings.ordre);

        if items.len() > self.limits.max_options_per_menu {
            self.problem(
                "too_many_options",
                Severity::Blocking,
                id,
                format!(
                    "{} options dans ce menu : {} au plus",
                    items.len(),
                    self.limits.max_options_per_menu
                ),
            );
        }

        RawDir {
            id: id.to_owned(),
            name: name.to_owned(),
            title: title_from_name(name, false),
            settings,
            settings_id,
            image,
            title_audio,
            items,
        }
    }

    /// Id du premier nom dans l'ordre naturel ; les autres sont signalés si `report`.
    fn pick(&mut self, dir_id: &str, mut names: Vec<String>, report: bool) -> Option<String> {
        names.sort_by(|a, b| natural_cmp(a, b));
        let mut iter = names.into_iter();
        let first = iter.next()?;
        for other in iter.filter(|_| report) {
            self.warn(
                "duplicate_special_file",
                &child_id(dir_id, &other),
                format!("« {other} » ignoré : « {first} » est utilisé"),
            );
        }
        Some(child_id(dir_id, &first))
    }
}

/// Fichier caché ou système (Windows).
fn is_hidden(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const HIDDEN: u32 = 0x2;
        const SYSTEM: u32 = 0x4;
        meta.file_attributes() & (HIDDEN | SYSTEM) != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Coup d'œil sur un dossier non lu, dans le budget global `budget` (entrées,
/// [`Limits::max_peek_entries`]) : vrai s'il contient, à toute profondeur et sans suivre de
/// lien, un fichier audio — ou si le budget est épuisé avant d'avoir tout vu (prudence).
fn may_contain_audio(dir: &Path, budget: &mut usize) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        if *budget == 0 {
            return true;
        }
        let Ok(read) = std::fs::read_dir(&current) else {
            continue;
        };
        for entry in read.flatten() {
            if *budget == 0 {
                return true;
            }
            *budget -= 1;
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = name_key(&entry.file_name().to_string_lossy());
            if file_type.is_symlink() || name.starts_with('.') {
                continue;
            }
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() && AUDIO_EXTENSIONS.contains(&ext_of(&name).as_str()) {
                return true;
            }
        }
    }
    false
}

/// Ordre du § 3.2 : entrées de `ordre` (par [`name_key`]) d'abord, puis le reste en tri
/// naturel.
fn sort_items(items: &mut [RawItem], ordre: &[String]) {
    let rank: HashMap<String, usize> = ordre
        .iter()
        .enumerate()
        .map(|(i, name)| (name_key(name), i))
        .collect();
    items.sort_by(|a, b| {
        let ra = rank.get(&name_key(a.name()));
        let rb = rank.get(&name_key(b.name()));
        match (ra, rb) {
            (Some(x), Some(y)) => x.cmp(y),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => natural_cmp(a.name(), b.name()),
        }
    });
}

fn mode_label(mode: Mode) -> &'static str {
    match mode {
        Mode::Recueil => "recueil",
        Mode::Chapitres => "chapitres",
    }
}

/// Mode proposé (§ 5.1) : chapitres si toutes les histoires sont des fichiers numérotés,
/// rangés à la racine ou dans des sous-dossiers qui ne contiennent que cela ; recueil sinon.
fn suggest_mode(root: &RawDir) -> Mode {
    fn numbered_only(dir: &RawDir) -> bool {
        dir.items.iter().all(|item| match item {
            RawItem::Story(s) => is_numbered(&s.file),
            RawItem::Dir(d) => numbered_only(d),
        })
    }
    if root.story_count() > 0 && numbered_only(root) {
        Mode::Chapitres
    } else {
        Mode::Recueil
    }
}

impl Walker<'_> {
    /// Arbre final : modes hérités, titres trop longs, menus vides (signalés une seule fois,
    /// au plus haut d'une chaîne de dossiers vides : `parent_empty`).
    fn finish(
        &mut self,
        dir: RawDir,
        inherited: Inherited,
        is_root: bool,
        parent_empty: bool,
    ) -> Node {
        let empty = !is_root && dir.story_count() == 0;
        if empty && !parent_empty {
            self.warn(
                "empty_menu",
                &dir.id,
                format!(
                    "« {} » ne contient aucune histoire : ignoré à la création",
                    dir.name
                ),
            );
        }
        if let Some(fin) = dir.settings.fin {
            if !fin.fits(inherited.mode) {
                let fallback = if is_root { STORY_FILE } else { FOLDER_FILE };
                let path = dir
                    .settings_id
                    .clone()
                    .unwrap_or_else(|| child_id(&dir.id, fallback));
                self.warn(
                    "fin_ignored",
                    &path,
                    format!(
                        "« fin » ne convient pas au mode {} : retour au menu",
                        mode_label(inherited.mode)
                    ),
                );
            }
        }
        if !is_root {
            self.check_title(&dir.id, &dir.title);
        }
        let mut children = Vec::with_capacity(dir.items.len());
        for item in dir.items {
            match item {
                RawItem::Dir(sub) => {
                    let child = inherited.child(&sub.settings);
                    children.push(self.finish(sub, child, false, empty));
                }
                RawItem::Story(s) => {
                    self.check_title(&s.id, &s.title);
                    children.push(Node::Story {
                        id: s.id,
                        file: s.file,
                        title: s.title,
                        image: s.image,
                        title_audio: s.title_audio,
                        bytes: s.bytes,
                    });
                }
            }
        }
        Node::Menu {
            id: dir.id,
            dir: dir.name,
            title: dir.title,
            mode: inherited.mode,
            fin: inherited.effective_fin(),
            annoncer: inherited.annoncer,
            image: dir.image,
            title_audio: dir.title_audio,
            children,
        }
    }

    fn check_title(&mut self, id: &str, title: &str) {
        let shown = font_text(title, None);
        if shown.chars().count() > LABEL_MAX {
            self.warn(
                "title_too_long",
                id,
                format!("titre « {title} » coupé à {LABEL_MAX} caractères sur la Flam"),
            );
        }
    }
}

/// Lit le dossier d'histoire `root` (voir la doc du module). Refuse les racines interdites
/// ([`check_root`] avec [`ForbiddenRoots::system`] et `extra_forbidden`, refusés avec leurs
/// parents et sous-dossiers : dossier du launcher, lecteur de la Flam), un lien et ce qui n'est
/// pas un dossier. Le parcours lit la forme **résolue** rendue par [`check_root`]
/// ([`Scan::base`]).
pub fn scan(root: &Path, limits: &Limits, extra_forbidden: &[PathBuf]) -> Result<Scan, ScanError> {
    let base = check_root(root, &ForbiddenRoots::system().with_trees(extra_forbidden))?;
    let shown = || root.to_string_lossy().into_owned();
    // Le dossier choisi lui-même ne doit pas être un lien (sa forme résolue n'en est plus un).
    let meta = match std::fs::symlink_metadata(root) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ScanError::NotFound(shown()))
        }
        Err(e) => return Err(ScanError::Io(e)),
    };
    if meta.file_type().is_symlink() {
        return Err(ScanError::Link(shown()));
    }
    if !std::fs::symlink_metadata(&base)?.is_dir() {
        return Err(ScanError::NotADirectory(shown()));
    }
    // Racine illisible : une erreur, pas un problème du rapport.
    drop(std::fs::read_dir(&base)?);

    let mut walker = Walker {
        limits,
        problems: Vec::new(),
        entries: 0,
        stories: 0,
        audio_bytes: 0,
        truncated: false,
        settings_budget: SETTINGS_BUDGET,
        peek_left: limits.max_peek_entries,
        story: Loaded {
            settings: StorySettings {
                folder: FolderSettings::default_new(),
                ..StorySettings::default()
            },
            ..Loaded::default()
        },
    };
    let name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut raw = walker.read_dir(&base, "", &name, 1);
    let mut story = std::mem::take(&mut walker.story);
    if let Some(titre) = &story.settings.titre {
        raw.title = titre.clone();
    }

    let suggested_mode = story
        .settings
        .folder
        .mode
        .is_none()
        .then(|| suggest_mode(&raw));
    let inherited = Inherited::root(
        &story.settings.folder,
        suggested_mode.unwrap_or(Mode::Recueil),
    );
    let root_node = walker.finish(raw, inherited, true, false);

    if walker.stories == 0 {
        walker.problem(
            "no_story",
            Severity::Blocking,
            "",
            "aucun fichier audio dans ce dossier".into(),
        );
    }
    if walker.stories > limits.max_stories {
        walker.problem(
            "too_many_stories",
            Severity::Blocking,
            "",
            format!(
                "{} histoires ou chapitres : {} au plus",
                walker.stories, limits.max_stories
            ),
        );
    }
    if walker.audio_bytes > limits.max_audio_bytes {
        walker.problem(
            "audio_too_large",
            Severity::Blocking,
            "",
            format!(
                "{} Mio d'audio : {} Mio au plus",
                walker.audio_bytes / (1024 * 1024),
                limits.max_audio_bytes / (1024 * 1024)
            ),
        );
    }

    // Les opérations relisent le fichier : ni clés inconnues ni valeurs brutes dans le résultat.
    story.settings.folder.extra.clear();
    Ok(Scan {
        base,
        root: root_node,
        story,
        suggested_mode,
        problems: walker.problems,
        story_count: walker.stories,
        audio_bytes: walker.audio_bytes,
        entries_seen: walker.entries.min(limits.max_entries),
        truncated: walker.truncated,
    })
}

#[cfg(test)]
mod tests;
