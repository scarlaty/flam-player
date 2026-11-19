//! Réglages d'une histoire faite sur dossier : `_histoire.json` (racine) et `_dossier.json`
//! (sous-dossiers). Spécification : `docs/creation-dossier.md` § 6 et § 5.1.
//!
//! Ces fichiers sont écrits par l'éditeur **et** modifiables à la main : leur lecture ne doit
//! jamais faire échouer l'ouverture d'un dossier.
//!
//! # Invariants
//!
//! - **Lecture tolérante** : fichier absent → réglages par défaut, sans avertissement ; fichier
//!   trop gros, illisible, JSON invalide ou qui n'est pas un objet → réglages par défaut et
//!   avertissement, [`Loaded::damaged`] vrai ; valeur invalide ou inconnue d'un champ connu →
//!   valeur par défaut (`None`) et avertissement. Les valeurs des énumérations sont lues sans
//!   casse (`"Chapitres"` est accepté, `"chapitres"` est écrit).
//! - **Lecture bornée** : au plus [`MAX_SETTINGS_BYTES`] octets par fichier, et un budget
//!   cumulé fourni par l'appelant (le parcours d'un dossier : `authoring::tree`) ; fichier
//!   ouvert **sans suivre** de lien (`install::pack::open_no_follow`) : un réglage en lien
//!   symbolique, en jonction ou autre point de réanalyse est refusé (avertissement). Un fichier
//!   non lu faute de budget n'est **pas** abîmé : [`Loaded::skipped`], pas
//!   [`Loaded::damaged`] ; il se relit seul avant toute opération.
//! - **Rien n'est perdu à la réécriture** : `extra` garde les clés inconnues **et** la valeur
//!   brute d'un champ connu invalide (`"mode": "roman"`, `"ordre": "a"`…). L'écriture part de
//!   `extra` et n'y pose que les champs typés renseignés (`Some`, liste non vide) : une valeur
//!   brute invalide survit tant que l'appelant ne pose pas de valeur pour ce champ. Seules les
//!   entrées invalides **à l'intérieur** de `ordre` et `titres` sont abandonnées (avertissement).
//! - **Format** : un fichier au `format` plus récent que [`FORMAT`], ou au `format` invalide
//!   (`"2"`, `0`…), est lu au mieux (avertissement) mais **jamais réécrit** ([`write_folder`]
//!   et [`write_story`] refusent avec `settings_newer_format`).
//! - **Titres** ([`TitleTable`]) : la clé exacte prime, puis la première clé de même
//!   [`name_key`] dans l'ordre stable de `titres`.
//! - **Écriture atomique** (`storage::write_atomic` : temporaire voisin, `sync_all`,
//!   renommage), taille bornée **avant** toute écriture (JSON indenté, sinon compact s'il
//!   tient seul dans la borne). Un fichier abîmé ([`Loaded::damaged`]) n'est jamais sauvegardé
//!   ici : c'est à l'appelant (opérations du lot 6, § 4.5) de le mettre à la corbeille `.flam/`
//!   avant de le réécrire.
//! - **Textes nettoyés** (`install::studio::clean_line`) : une ligne, sans caractère de format,
//!   longueur bornée ; un texte vide après nettoyage vaut absent.
//! - **Noms de `ordre` et `titres`** : simples noms d'entrées du dossier (ni `/`, ni `\`, ni
//!   `.`/`..`, ni vide) ; les autres sont ignorés avec avertissement. Ils ne servent qu'à être
//!   comparés à la liste lue du dossier par leur clé [`name_key`] (NFC puis minuscules), jamais
//!   à construire un chemin. Une entrée qui ne désigne plus rien est ignorée à la lecture du
//!   dossier et retirée par [`FolderSettings::prune`].
//! - **Héritage** (§ 6) : `mode`, `fin` et `annoncer` absents d'un `_dossier.json` héritent du
//!   dossier parent ([`Inherited::child`]) ; à la racine, ces champs (ainsi que `ordre` et
//!   `titres`) sont dans `_histoire.json`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::install::pack::open_no_follow;
use crate::install::studio::clean_line;
use crate::story::MAX_TITLE_CHARS;
use crate::{AppError, AppResult};

/// Réglages de l'histoire, à la racine du dossier.
pub const STORY_FILE: &str = "_histoire.json";
/// Réglages d'un sous-dossier (menu ou partie).
pub const FOLDER_FILE: &str = "_dossier.json";
/// Taille maximale d'un fichier de réglages, en lecture comme en écriture.
pub const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
/// Version du format des fichiers de réglages écrits par cette version.
pub const FORMAT: u64 = 1;
/// Longueur maximale (caractères) de `sousTitre` et `description` après nettoyage.
pub const MAX_TEXT_CHARS: usize = 2_000;
/// Âge maximal accepté pour `age`.
pub const MAX_AGE: u64 = 99;

/// Avertissement : fichier de réglages illisible (taille, budget, lien, entrée/sortie, JSON,
/// pas un objet).
pub const CODE_UNREADABLE: &str = "settings_unreadable";
/// Avertissement : valeur d'un champ connu invalide, lue comme la valeur par défaut.
pub const CODE_INVALID_VALUE: &str = "settings_invalid_value";
/// Avertissement (lecture) ou refus (écriture) : `format` plus récent que [`FORMAT`].
pub const CODE_NEWER_FORMAT: &str = "settings_newer_format";

/// Organisation d'un dossier (§ 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Menu de petites histoires (§ 5.2).
    Recueil,
    /// Livre à chapitres qui s'enchaînent (§ 5.3).
    Chapitres,
}

/// Fin d'une histoire (recueil : `menu`, `suivante`) ou du livre (chapitres : `menu`, `fin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fin {
    /// Retour au menu (défaut des deux modes).
    Menu,
    /// Recueil : l'histoire suivante du même menu démarre.
    Suivante,
    /// Chapitres : fin d'histoire, retour à la bibliothèque de la Flam.
    Fin,
}

impl Fin {
    /// Vrai si cette fin a un sens dans `mode` (§ 5.2, § 5.3).
    pub fn fits(self, mode: Mode) -> bool {
        match self {
            Fin::Menu => true,
            Fin::Suivante => mode == Mode::Recueil,
            Fin::Fin => mode == Mode::Chapitres,
        }
    }
}

/// Démarrage d'un livre à chapitres (§ 5.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Debut {
    /// Premier chapitre (défaut).
    Chapitre,
    /// Menu des chapitres.
    Menu,
}

/// Avertissement de lecture d'un fichier de réglages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsWarning {
    /// Code stable : [`CODE_UNREADABLE`], [`CODE_INVALID_VALUE`] ou [`CODE_NEWER_FORMAT`].
    pub code: &'static str,
    /// Clé concernée (`None` : le fichier entier).
    pub key: Option<String>,
    /// Explication lisible.
    pub message: String,
}

/// Résultat d'une lecture tolérante.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Loaded<T> {
    /// Réglages lus (valeurs par défaut pour ce qui manque ou est invalide).
    pub settings: T,
    /// Le fichier existe (même abîmé).
    pub exists: bool,
    /// Le fichier a été lu mais n'est pas un objet JSON exploitable (trop gros, illisible, JSON
    /// invalide, lien refusé) : il est **abîmé**, il ne doit pas être réécrit sans sauvegarde
    /// (corbeille `.flam/`, lot 6).
    pub damaged: bool,
    /// Le fichier n'a **pas été lu** : budget cumulé du parcours épuisé (`authoring::tree`). Il
    /// n'est pas abîmé et ne doit ni être mis à la corbeille ni être réécrit d'après ce
    /// résultat : le relire seul ([`load_folder`], [`load_story`]) avant toute opération.
    pub skipped: bool,
    pub warnings: Vec<SettingsWarning>,
}

/// Champs communs à `_dossier.json` et à la racine (`_histoire.json`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderSettings {
    /// `format` lu (absent ou invalide : [`FORMAT`]). Plus récent : écriture refusée.
    pub format: u64,
    /// Ordre choisi : noms d'entrées du dossier, sans doublon (comparaison par [`name_key`]).
    pub ordre: Vec<String>,
    /// Titres affichés, par nom d'entrée.
    pub titres: BTreeMap<String, String>,
    pub mode: Option<Mode>,
    pub fin: Option<Fin>,
    pub annoncer: Option<bool>,
    /// Clés inconnues et valeurs brutes invalides des champs connus, conservées à la réécriture.
    pub extra: Map<String, Value>,
}

/// Contenu de `_histoire.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StorySettings {
    /// Champs de dossier de la racine (`ordre`, `titres`, `mode`, `fin`, `annoncer`), clés
    /// inconnues et valeurs brutes invalides.
    pub folder: FolderSettings,
    /// Identifiant de l'histoire (uuid valide, en minuscules).
    pub uuid: Option<String>,
    pub version: Option<u32>,
    pub titre: Option<String>,
    pub sous_titre: Option<String>,
    pub description: Option<String>,
    pub age: Option<u8>,
    pub debut: Option<Debut>,
    /// Réglages des titres lus (`titresLus`), objet gardé tel quel.
    pub titres_lus: Option<Map<String, Value>>,
}

/// Valeurs héritées d'un dossier par ses sous-dossiers (§ 5.1, § 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inherited {
    pub mode: Mode,
    pub fin: Option<Fin>,
    pub annoncer: bool,
}

impl Inherited {
    /// Valeurs effectives de la racine : `mode` réglé, sinon `fallback` (mode proposé).
    pub fn root(settings: &FolderSettings, fallback: Mode) -> Self {
        Self {
            mode: settings.mode.unwrap_or(fallback),
            fin: settings.fin,
            annoncer: settings.annoncer.unwrap_or(false),
        }
    }

    /// Valeurs effectives d'un sous-dossier : ses réglages, sinon ceux de `self` (parent).
    pub fn child(&self, settings: &FolderSettings) -> Self {
        Self {
            mode: settings.mode.unwrap_or(self.mode),
            fin: settings.fin.or(self.fin),
            annoncer: settings.annoncer.unwrap_or(self.annoncer),
        }
    }

    /// Fin effective : la fin héritée ou réglée si elle a un sens dans le mode, sinon `menu`.
    pub fn effective_fin(&self) -> Fin {
        self.fin
            .filter(|fin| fin.fits(self.mode))
            .unwrap_or(Fin::Menu)
    }
}

/// Clé de comparaison d'un nom d'entrée : forme NFC puis minuscules. Deux noms de même clé
/// désignent la même entrée (Windows ignore la casse ; macOS et certains outils écrivent en
/// NFD).
pub fn name_key(name: &str) -> String {
    name.nfc().collect::<String>().to_lowercase()
}

/// Titres d'un dossier prêts à la recherche ([`FolderSettings::title_table`]) : la clé exacte
/// prime ; sinon, parmi les clés de même [`name_key`], la première dans l'ordre stable de
/// `titres` (ordre des chaînes).
#[derive(Debug, Clone, Default)]
pub struct TitleTable<'a> {
    exact: HashMap<&'a str, &'a str>,
    folded: HashMap<String, &'a str>,
}

impl<'a> TitleTable<'a> {
    /// Titre de l'entrée `name`.
    pub fn get(&self, name: &str) -> Option<&'a str> {
        self.exact
            .get(name)
            .or_else(|| self.folded.get(&name_key(name)))
            .copied()
    }
}

impl FolderSettings {
    /// Table de recherche des titres, à construire une fois par dossier.
    pub fn title_table(&self) -> TitleTable<'_> {
        let mut table = TitleTable::default();
        for (name, title) in &self.titres {
            table.exact.insert(name, title);
            table.folded.entry(name_key(name)).or_insert(title);
        }
        table
    }

    /// Titre choisi pour l'entrée `name` (voir [`TitleTable`]). Pour plusieurs recherches,
    /// préférer [`FolderSettings::title_table`].
    pub fn title_of(&self, name: &str) -> Option<&str> {
        if let Some(title) = self.titres.get(name) {
            return Some(title);
        }
        let key = name_key(name);
        self.titres
            .iter()
            .find(|(other, _)| name_key(other) == key)
            .map(|(_, title)| title.as_str())
    }

    /// Retire de `ordre` et `titres` les noms dont la clé [`name_key`] n'est pas dans `existing`.
    pub fn prune(&mut self, existing: &HashSet<String>) {
        self.ordre.retain(|name| existing.contains(&name_key(name)));
        self.titres
            .retain(|name, _| existing.contains(&name_key(name)));
    }

    /// Réglages vides, au format courant.
    pub fn default_new() -> Self {
        Self {
            format: FORMAT,
            ..Self::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Lecture
// ---------------------------------------------------------------------------

/// Lit `<dir>/_dossier.json` (budget : un fichier).
pub fn load_folder(dir: &Path) -> Loaded<FolderSettings> {
    let mut budget = MAX_SETTINGS_BYTES;
    load_folder_file(&dir.join(FOLDER_FILE), &mut budget)
}

/// Lit `<root>/_histoire.json` (budget : un fichier).
pub fn load_story(root: &Path) -> Loaded<StorySettings> {
    let mut budget = MAX_SETTINGS_BYTES;
    load_story_file(&root.join(STORY_FILE), &mut budget)
}

/// Lit le fichier `_dossier.json` `path` en décomptant les octets lus de `budget`.
pub fn load_folder_file(path: &Path, budget: &mut u64) -> Loaded<FolderSettings> {
    let (raw, mut loaded) = read_object(path, budget);
    let settings = match raw {
        None => FolderSettings::default_new(),
        Some(map) => {
            let mut fields = Fields::new(map, &mut loaded.warnings);
            let settings = fields.folder();
            FolderSettings {
                extra: fields.finish(),
                ..settings
            }
        }
    };
    Loaded {
        settings,
        exists: loaded.exists,
        damaged: loaded.damaged,
        skipped: loaded.skipped,
        warnings: loaded.warnings,
    }
}

/// Lit le fichier `_histoire.json` `path` en décomptant les octets lus de `budget`.
pub fn load_story_file(path: &Path, budget: &mut u64) -> Loaded<StorySettings> {
    let (raw, mut loaded) = read_object(path, budget);
    let settings = match raw {
        None => StorySettings {
            folder: FolderSettings::default_new(),
            ..StorySettings::default()
        },
        Some(map) => {
            let mut fields = Fields::new(map, &mut loaded.warnings);
            let folder = fields.folder();
            let story = fields.story();
            StorySettings {
                folder: FolderSettings {
                    extra: fields.finish(),
                    ..folder
                },
                ..story
            }
        }
    };
    Loaded {
        settings,
        exists: loaded.exists,
        damaged: loaded.damaged,
        skipped: loaded.skipped,
        warnings: loaded.warnings,
    }
}

fn unreadable(message: String) -> SettingsWarning {
    SettingsWarning {
        code: CODE_UNREADABLE,
        key: None,
        message,
    }
}

/// Lit `path` comme un objet JSON borné (par fichier et par `budget`). `None` si absent ou
/// abîmé (voir `Loaded`).
fn read_object(path: &Path, budget: &mut u64) -> (Option<Map<String, Value>>, Loaded<()>) {
    let mut loaded = Loaded::<()>::default();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let size = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, loaded),
        Err(_) => 0,
        Ok(meta) => meta.len(),
    };
    loaded.exists = true;
    // Trop gros pour un réglage : abîmé (plus bas). Simplement plus de budget : non lu.
    if size <= MAX_SETTINGS_BYTES && size > *budget {
        loaded.skipped = true;
        loaded.warnings.push(unreadable(format!(
            "{name} non lu : trop de réglages à lire dans ce dossier, réglages par défaut"
        )));
        return (None, loaded);
    }
    let mut damaged = |message: String| {
        loaded.damaged = true;
        loaded.warnings.push(unreadable(message));
    };
    let file = match open_no_follow(path) {
        Ok(file) => file,
        Err(e) => {
            damaged(format!("{name} illisible, réglages par défaut : {e}"));
            return (None, loaded);
        }
    };
    let limit = MAX_SETTINGS_BYTES.min(*budget);
    let mut bytes = Vec::new();
    let read = file.take(limit + 1).read_to_end(&mut bytes);
    *budget = budget.saturating_sub(bytes.len() as u64);
    if let Err(e) = read {
        damaged(format!("{name} illisible, réglages par défaut : {e}"));
        return (None, loaded);
    }
    if bytes.len() as u64 > limit {
        damaged(format!(
            "{name} trop volumineux (plus de {} Kio), réglages par défaut",
            MAX_SETTINGS_BYTES / 1024
        ));
        return (None, loaded);
    }
    // BOM UTF-8 toléré (fichier enregistré par le Bloc-notes).
    let text = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
    match serde_json::from_slice::<Value>(text) {
        Ok(Value::Object(map)) => (Some(map), loaded),
        Ok(_) => {
            damaged(format!(
                "{name} n'est pas un objet JSON, réglages par défaut"
            ));
            (None, loaded)
        }
        Err(e) => {
            damaged(format!("{name} : JSON invalide ({e}), réglages par défaut"));
            (None, loaded)
        }
    }
}

/// Extraction des champs connus d'un objet JSON : chaque champ est retiré de `map` ; une
/// valeur invalide est gardée brute (`kept`) avec un avertissement, puis rendue dans `extra`.
struct Fields<'a> {
    map: Map<String, Value>,
    kept: Map<String, Value>,
    warnings: &'a mut Vec<SettingsWarning>,
}

impl<'a> Fields<'a> {
    fn new(map: Map<String, Value>, warnings: &'a mut Vec<SettingsWarning>) -> Self {
        Self {
            map,
            kept: Map::new(),
            warnings,
        }
    }

    /// Valeur de `key` (`null` vaut absent).
    fn take(&mut self, key: &str) -> Option<Value> {
        self.map.remove(key).filter(|v| !v.is_null())
    }

    /// Garde la valeur brute invalide `value` de `key` et avertit.
    fn keep(&mut self, key: &str, value: Value, message: String) {
        self.kept.insert(key.to_owned(), value);
        self.warn(key, message);
    }

    /// [`Fields::keep`], puis `None` (valeur typée absente).
    fn reject<T>(&mut self, key: &str, value: Value, message: String) -> Option<T> {
        self.keep(key, value, message);
        None
    }

    fn warn(&mut self, key: &str, message: String) {
        self.warnings.push(SettingsWarning {
            code: CODE_INVALID_VALUE,
            key: Some(key.to_owned()),
            message,
        });
    }

    /// Clés inconnues et valeurs brutes invalides.
    fn finish(self) -> Map<String, Value> {
        let mut extra = self.map;
        extra.extend(self.kept);
        extra
    }

    fn folder(&mut self) -> FolderSettings {
        let format = match self.take("format") {
            None => FORMAT,
            Some(value) => match value.as_u64() {
                Some(n) if n >= 1 => {
                    if n > FORMAT {
                        self.warnings.push(SettingsWarning {
                            code: CODE_NEWER_FORMAT,
                            key: Some("format".into()),
                            message: format!(
                                "réglages au format {n}, plus récent que celui pris en charge \
                                 ({FORMAT}) : lus au mieux, jamais réécrits"
                            ),
                        });
                    }
                    n
                }
                _ => {
                    // Gardé brut : l'écriture sera refusée, jamais écrasée.
                    self.keep(
                        "format",
                        value,
                        "« format » invalide : lu au mieux, jamais réécrit".into(),
                    );
                    FORMAT
                }
            },
        };
        FolderSettings {
            format,
            ordre: self.order(),
            titres: self.titles(),
            mode: self.enumeration("mode"),
            fin: self.enumeration("fin"),
            annoncer: match self.take("annoncer") {
                None => None,
                Some(Value::Bool(b)) => Some(b),
                Some(value) => self.reject(
                    "annoncer",
                    value,
                    "« annoncer » doit valoir true ou false".into(),
                ),
            },
            extra: Map::new(),
        }
    }

    fn story(&mut self) -> StorySettings {
        let uuid = match self.take("uuid") {
            None => None,
            Some(value) => match value.as_str().map(|s| uuid::Uuid::parse_str(s.trim())) {
                Some(Ok(id)) => Some(id.hyphenated().to_string()),
                _ => self.reject("uuid", value, "« uuid » invalide, ignoré".into()),
            },
        };
        let version = match self.take("version") {
            None => None,
            Some(value) => match value.as_u64().and_then(|n| u32::try_from(n).ok()) {
                Some(n) if n >= 1 => Some(n),
                _ => self.reject("version", value, "« version » invalide, ignorée".into()),
            },
        };
        let age = match self.take("age") {
            None => None,
            Some(value) => match value.as_u64() {
                Some(n) if n <= MAX_AGE => u8::try_from(n).ok(),
                _ => self.reject("age", value, "« age » invalide, ignoré".into()),
            },
        };
        let titres_lus = match self.take("titresLus") {
            None => None,
            Some(Value::Object(obj)) => Some(obj),
            Some(value) => self.reject(
                "titresLus",
                value,
                "« titresLus » doit être un objet, ignoré".into(),
            ),
        };
        StorySettings {
            folder: FolderSettings::default(),
            uuid,
            version,
            titre: self.text("titre", MAX_TITLE_CHARS),
            sous_titre: self.text("sousTitre", MAX_TEXT_CHARS),
            description: self.text("description", MAX_TEXT_CHARS),
            age,
            debut: self.enumeration("debut"),
            titres_lus,
        }
    }

    fn text(&mut self, key: &str, max: usize) -> Option<String> {
        match self.take(key) {
            None => None,
            Some(Value::String(s)) => Some(clean_line(&s, max)).filter(|s| !s.is_empty()),
            Some(value) => self.reject(key, value, format!("« {key} » doit être un texte, ignoré")),
        }
    }

    /// Énumération lue sans casse.
    fn enumeration<T: serde::de::DeserializeOwned>(&mut self, key: &str) -> Option<T> {
        let value = self.take(key)?;
        let parsed = value
            .as_str()
            .and_then(|s| serde_json::from_value::<T>(Value::String(s.trim().to_lowercase())).ok());
        match parsed {
            Some(v) => Some(v),
            None => {
                let message = format!("« {key} » : valeur {value} inconnue, valeur par défaut");
                self.reject(key, value, message)
            }
        }
    }

    fn order(&mut self) -> Vec<String> {
        let items = match self.take("ordre") {
            None => return Vec::new(),
            Some(Value::Array(items)) => items,
            Some(value) => {
                self.keep(
                    "ordre",
                    value,
                    "« ordre » doit être une liste, ignoré".into(),
                );
                return Vec::new();
            }
        };
        let (ordre, rejected) = parse_order_items(items);
        if rejected > 0 {
            self.warn(
                "ordre",
                format!("« ordre » : {rejected} entrée(s) invalide(s) ignorée(s)"),
            );
        }
        ordre
    }

    fn titles(&mut self) -> BTreeMap<String, String> {
        let obj = match self.take("titres") {
            None => return BTreeMap::new(),
            Some(Value::Object(obj)) => obj,
            Some(value) => {
                self.keep(
                    "titres",
                    value,
                    "« titres » doit être un objet, ignoré".into(),
                );
                return BTreeMap::new();
            }
        };
        let mut titres = BTreeMap::new();
        let mut rejected = 0usize;
        for (name, title) in obj {
            let cleaned = match &title {
                Value::String(s) => clean_line(s, MAX_TITLE_CHARS),
                _ => String::new(),
            };
            if is_plain_name(&name) && !cleaned.is_empty() {
                titres.insert(name, cleaned);
            } else {
                rejected += 1;
            }
        }
        if rejected > 0 {
            self.warn(
                "titres",
                format!("« titres » : {rejected} entrée(s) invalide(s) ignorée(s)"),
            );
        }
        titres
    }
}

/// Simple nom d'entrée de dossier : non vide, sans séparateur, ni `.` ni `..`.
pub fn is_plain_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}

/// Entrées valides de `ordre`, sans doublon ([`name_key`]), et nombre d'entrées rejetées.
fn parse_order_items(items: Vec<Value>) -> (Vec<String>, usize) {
    let mut seen = HashSet::new();
    let mut ordre = Vec::new();
    let mut rejected = 0usize;
    for item in items {
        match item {
            Value::String(name) if is_plain_name(&name) => {
                if seen.insert(name_key(&name)) {
                    ordre.push(name);
                }
            }
            _ => rejected += 1,
        }
    }
    (ordre, rejected)
}

// ---------------------------------------------------------------------------
// Écriture
// ---------------------------------------------------------------------------

/// Pose `value` sous `key` si elle est renseignée ; sinon laisse `map` tel quel (une valeur
/// brute invalide gardée dans `extra` survit).
fn put<T: Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) -> AppResult<()> {
    if let Some(v) = value {
        map.insert(key.to_owned(), serde_json::to_value(v)?);
    }
    Ok(())
}

/// Objet JSON des champs de dossier (clés inconnues et valeurs brutes comprises).
fn folder_object(settings: &FolderSettings) -> AppResult<Map<String, Value>> {
    // `format` invalide (gardé brut dans `extra`) : traité comme un format plus récent.
    if settings.format > FORMAT || settings.extra.contains_key("format") {
        let found = settings
            .extra
            .get("format")
            .map_or_else(|| settings.format.to_string(), Value::to_string);
        return Err(AppError::Validation(format!(
            "réglages au format {found}, inconnu de cette version ({FORMAT}) : non modifiés \
             ({CODE_NEWER_FORMAT})"
        )));
    }
    let mut map = settings.extra.clone();
    map.insert("format".into(), Value::from(FORMAT));
    let ordre = (!settings.ordre.is_empty()).then_some(&settings.ordre);
    put(&mut map, "ordre", ordre)?;
    let titres = (!settings.titres.is_empty()).then_some(&settings.titres);
    put(&mut map, "titres", titres)?;
    put(&mut map, "mode", settings.mode)?;
    put(&mut map, "fin", settings.fin)?;
    put(&mut map, "annoncer", settings.annoncer)?;
    Ok(map)
}

/// Objet JSON complet de `_histoire.json`.
fn story_object(settings: &StorySettings) -> AppResult<Map<String, Value>> {
    let mut map = folder_object(&settings.folder)?;
    put(&mut map, "uuid", settings.uuid.as_ref())?;
    put(&mut map, "version", settings.version)?;
    put(&mut map, "titre", settings.titre.as_ref())?;
    put(&mut map, "sousTitre", settings.sous_titre.as_ref())?;
    put(&mut map, "description", settings.description.as_ref())?;
    put(&mut map, "age", settings.age)?;
    put(&mut map, "debut", settings.debut)?;
    put(&mut map, "titresLus", settings.titres_lus.as_ref())?;
    Ok(map)
}

/// Sérialise `map` (indenté, sinon compact pour tenir dans la borne), puis écrit atomiquement.
fn write_object(path: &Path, map: Map<String, Value>) -> AppResult<()> {
    let value = Value::Object(map);
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    if bytes.len() as u64 >= MAX_SETTINGS_BYTES {
        bytes = serde_json::to_vec(&value)?;
    }
    bytes.push(b'\n');
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(AppError::Validation(format!(
            "réglages trop volumineux (plus de {} Kio), non enregistrés",
            MAX_SETTINGS_BYTES / 1024
        )));
    }
    crate::storage::write_atomic(path, &bytes)?;
    Ok(())
}

/// Écrit `<dir>/_dossier.json` (atomique, rien de perdu, format plus récent refusé).
pub fn write_folder(dir: &Path, settings: &FolderSettings) -> AppResult<()> {
    write_object(&dir.join(FOLDER_FILE), folder_object(settings)?)
}

/// Écrit `<root>/_histoire.json` (atomique, rien de perdu, format plus récent refusé).
pub fn write_story(root: &Path, settings: &StorySettings) -> AppResult<()> {
    write_object(&root.join(STORY_FILE), story_object(settings)?)
}

#[cfg(test)]
mod tests;
