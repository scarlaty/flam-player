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
//!   avertissement, [`Loaded::damaged`] vrai ; valeur invalide d'un champ connu → valeur par
//!   défaut (`None`) et avertissement ; le reste du fichier est gardé.
//! - **Lecture bornée** : au plus [`MAX_SETTINGS_BYTES`] octets, fichier ouvert **sans suivre**
//!   de lien (`install::pack::open_no_follow`) : un réglage en lien symbolique ou en jonction
//!   est refusé (avertissement).
//! - **Clés inconnues conservées** : toute clé non reconnue (et la valeur brute de `titresLus`)
//!   est gardée telle quelle dans `extra` et réécrite. Une clé connue dont la valeur était
//!   invalide est en revanche **remplacée** à l'écriture par la valeur typée (ou retirée).
//! - **Écriture atomique** (`storage::write_atomic` : temporaire voisin, `sync_all`,
//!   renommage), taille bornée **avant** toute écriture. Un fichier abîmé ([`Loaded::damaged`])
//!   n'est jamais sauvegardé ici : c'est à l'appelant (opérations du lot 6, § 4.5) de le mettre
//!   à la corbeille `.flam/` avant de le réécrire.
//! - **Textes nettoyés** (`install::studio::clean_line`) : une ligne, sans caractère de format,
//!   longueur bornée ; un texte vide après nettoyage vaut absent.
//! - **Noms de `ordre` et `titres`** : simples noms d'entrées du dossier (ni `/`, ni `\`, ni
//!   `.`/`..`, ni vide) ; les autres sont ignorés avec avertissement. Ils ne servent qu'à être
//!   comparés à la liste lue du dossier, jamais à construire un chemin. Une entrée qui ne
//!   désigne plus rien est ignorée à la lecture du dossier et retirée par [`FolderSettings::prune`].
//! - **Héritage** (§ 6) : `mode`, `fin` et `annoncer` absents d'un `_dossier.json` héritent du
//!   dossier parent ([`Inherited::child`]) ; à la racine, ces champs (ainsi que `ordre` et
//!   `titres`) sont dans `_histoire.json`.

use std::collections::{BTreeMap, HashSet};
use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

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

/// Avertissement : fichier de réglages illisible (taille, lien, entrée/sortie, JSON, pas un objet).
pub const CODE_UNREADABLE: &str = "settings_unreadable";
/// Avertissement : valeur d'un champ connu invalide, remplacée par la valeur par défaut.
pub const CODE_INVALID_VALUE: &str = "settings_invalid_value";
/// Avertissement : `format` plus récent que [`FORMAT`] (lu au mieux).
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
    /// Le fichier existe mais n'a pas pu être lu comme un objet JSON : il ne doit pas être
    /// réécrit sans sauvegarde.
    pub damaged: bool,
    pub warnings: Vec<SettingsWarning>,
}

/// Champs communs à `_dossier.json` et à la racine (`_histoire.json`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FolderSettings {
    /// `format` lu (absent : [`FORMAT`]) ; réécrit tel quel s'il est plus récent.
    pub format: u64,
    /// Ordre choisi : noms d'entrées du dossier, sans doublon (comparaison sans casse).
    pub ordre: Vec<String>,
    /// Titres affichés, par nom d'entrée.
    pub titres: BTreeMap<String, String>,
    pub mode: Option<Mode>,
    pub fin: Option<Fin>,
    pub annoncer: Option<bool>,
    /// Clés inconnues, conservées à la réécriture.
    pub extra: Map<String, Value>,
}

/// Contenu de `_histoire.json`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StorySettings {
    /// Champs de dossier de la racine (`ordre`, `titres`, `mode`, `fin`, `annoncer`) et clés
    /// inconnues.
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

impl FolderSettings {
    /// Titre choisi pour l'entrée `name` (comparaison sans casse, comme les noms Windows).
    pub fn title_of(&self, name: &str) -> Option<&str> {
        if let Some(title) = self.titres.get(name) {
            return Some(title);
        }
        let lower = name.to_lowercase();
        self.titres
            .iter()
            .find(|(key, _)| key.to_lowercase() == lower)
            .map(|(_, title)| title.as_str())
    }

    /// Retire de `ordre` et `titres` les noms absents de `existing` (noms en minuscules).
    pub fn prune(&mut self, existing: &HashSet<String>) {
        self.ordre
            .retain(|name| existing.contains(&name.to_lowercase()));
        self.titres
            .retain(|name, _| existing.contains(&name.to_lowercase()));
    }
}

// ---------------------------------------------------------------------------
// Lecture
// ---------------------------------------------------------------------------

/// Lit `<dir>/_dossier.json`.
pub fn load_folder(dir: &Path) -> Loaded<FolderSettings> {
    let (raw, mut loaded) = read_object(&dir.join(FOLDER_FILE));
    let mut warnings = Vec::new();
    let settings = raw.map_or_else(FolderSettings::default_new, |mut map| {
        let settings = parse_folder(&mut map, &mut warnings);
        FolderSettings {
            extra: map,
            ..settings
        }
    });
    loaded.warnings.extend(warnings);
    Loaded {
        settings,
        exists: loaded.exists,
        damaged: loaded.damaged,
        warnings: loaded.warnings,
    }
}

/// Lit `<root>/_histoire.json`.
pub fn load_story(root: &Path) -> Loaded<StorySettings> {
    let (raw, loaded) = read_object(&root.join(STORY_FILE));
    let mut warnings = loaded.warnings;
    let settings = match raw {
        None => StorySettings {
            folder: FolderSettings::default_new(),
            ..StorySettings::default()
        },
        Some(mut map) => {
            let folder = parse_folder(&mut map, &mut warnings);
            let story = parse_story(&mut map, &mut warnings);
            StorySettings {
                folder: FolderSettings {
                    extra: map,
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
        warnings,
    }
}

impl FolderSettings {
    /// Réglages vides, au format courant.
    fn default_new() -> Self {
        Self {
            format: FORMAT,
            ..Self::default()
        }
    }
}

fn unreadable(message: String) -> SettingsWarning {
    SettingsWarning {
        code: CODE_UNREADABLE,
        key: None,
        message,
    }
}

fn invalid(key: &str, message: impl Into<String>) -> SettingsWarning {
    SettingsWarning {
        code: CODE_INVALID_VALUE,
        key: Some(key.to_owned()),
        message: message.into(),
    }
}

/// Lit `path` comme un objet JSON borné. `None` si absent ou abîmé (voir `Loaded`).
fn read_object(path: &Path) -> (Option<Map<String, Value>>, Loaded<()>) {
    let mut loaded = Loaded::<()>::default();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (None, loaded),
        _ => loaded.exists = true,
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
    let mut bytes = Vec::new();
    if let Err(e) = file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes) {
        damaged(format!("{name} illisible, réglages par défaut : {e}"));
        return (None, loaded);
    }
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
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

/// Retire et valide les champs de dossier de `map` (les clés inconnues y restent).
fn parse_folder(
    map: &mut Map<String, Value>,
    warnings: &mut Vec<SettingsWarning>,
) -> FolderSettings {
    let format = match map.remove("format") {
        None => FORMAT,
        Some(value) => match value.as_u64() {
            Some(n) if n >= 1 => {
                if n > FORMAT {
                    warnings.push(SettingsWarning {
                        code: CODE_NEWER_FORMAT,
                        key: Some("format".into()),
                        message: format!(
                            "réglages au format {n}, plus récent que celui pris en charge \
                             ({FORMAT}) : lus au mieux"
                        ),
                    });
                }
                n
            }
            _ => {
                warnings.push(invalid("format", "« format » invalide, ignoré"));
                FORMAT
            }
        },
    };
    FolderSettings {
        format,
        ordre: parse_order(map.remove("ordre"), warnings),
        titres: parse_titles(map.remove("titres"), warnings),
        mode: parse_enum(map, "mode", warnings),
        fin: parse_enum(map, "fin", warnings),
        annoncer: match map.remove("annoncer") {
            None | Some(Value::Null) => None,
            Some(Value::Bool(b)) => Some(b),
            Some(_) => {
                warnings.push(invalid(
                    "annoncer",
                    "« annoncer » doit valoir true ou false",
                ));
                None
            }
        },
        extra: Map::new(),
    }
}

/// Retire et valide les champs propres à `_histoire.json`.
fn parse_story(map: &mut Map<String, Value>, warnings: &mut Vec<SettingsWarning>) -> StorySettings {
    let uuid = match map.remove("uuid") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => match uuid::Uuid::parse_str(s.trim()) {
            Ok(id) => Some(id.hyphenated().to_string()),
            Err(_) => {
                warnings.push(invalid("uuid", "« uuid » invalide, ignoré"));
                None
            }
        },
        Some(_) => {
            warnings.push(invalid("uuid", "« uuid » invalide, ignoré"));
            None
        }
    };
    let version = match map.remove("version") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64().and_then(|n| u32::try_from(n).ok()) {
            Some(n) if n >= 1 => Some(n),
            _ => {
                warnings.push(invalid("version", "« version » invalide, ignorée"));
                None
            }
        },
    };
    let age = match map.remove("age") {
        None | Some(Value::Null) => None,
        Some(value) => match value.as_u64() {
            Some(n) if n <= MAX_AGE => u8::try_from(n).ok(),
            _ => {
                warnings.push(invalid("age", "« age » invalide, ignoré"));
                None
            }
        },
    };
    let titres_lus = match map.remove("titresLus") {
        None | Some(Value::Null) => None,
        Some(Value::Object(obj)) => Some(obj),
        Some(_) => {
            warnings.push(invalid(
                "titresLus",
                "« titresLus » doit être un objet, ignoré",
            ));
            None
        }
    };
    StorySettings {
        folder: FolderSettings::default(),
        uuid,
        version,
        titre: parse_text(map, "titre", MAX_TITLE_CHARS, warnings),
        sous_titre: parse_text(map, "sousTitre", MAX_TEXT_CHARS, warnings),
        description: parse_text(map, "description", MAX_TEXT_CHARS, warnings),
        age,
        debut: parse_enum(map, "debut", warnings),
        titres_lus,
    }
}

fn parse_text(
    map: &mut Map<String, Value>,
    key: &str,
    max: usize,
    warnings: &mut Vec<SettingsWarning>,
) -> Option<String> {
    match map.remove(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(clean_line(&s, max)).filter(|s| !s.is_empty()),
        Some(_) => {
            warnings.push(invalid(
                key,
                format!("« {key} » doit être un texte, ignoré"),
            ));
            None
        }
    }
}

fn parse_enum<T: serde::de::DeserializeOwned>(
    map: &mut Map<String, Value>,
    key: &str,
    warnings: &mut Vec<SettingsWarning>,
) -> Option<T> {
    match map.remove(key) {
        None | Some(Value::Null) => None,
        Some(value) => match serde_json::from_value::<T>(value.clone()) {
            Ok(v) => Some(v),
            Err(_) => {
                warnings.push(invalid(
                    key,
                    format!("« {key} » : valeur {value} inconnue, valeur par défaut"),
                ));
                None
            }
        },
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

fn parse_order(value: Option<Value>, warnings: &mut Vec<SettingsWarning>) -> Vec<String> {
    let items = match value {
        None | Some(Value::Null) => return Vec::new(),
        Some(Value::Array(items)) => items,
        Some(_) => {
            warnings.push(invalid("ordre", "« ordre » doit être une liste, ignoré"));
            return Vec::new();
        }
    };
    let mut seen = HashSet::new();
    let mut ordre = Vec::new();
    let mut rejected = 0usize;
    for item in items {
        match item {
            Value::String(name) if is_plain_name(&name) => {
                if seen.insert(name.to_lowercase()) {
                    ordre.push(name);
                }
            }
            _ => rejected += 1,
        }
    }
    if rejected > 0 {
        warnings.push(invalid(
            "ordre",
            format!("« ordre » : {rejected} entrée(s) invalide(s) ignorée(s)"),
        ));
    }
    ordre
}

fn parse_titles(
    value: Option<Value>,
    warnings: &mut Vec<SettingsWarning>,
) -> BTreeMap<String, String> {
    let obj = match value {
        None | Some(Value::Null) => return BTreeMap::new(),
        Some(Value::Object(obj)) => obj,
        Some(_) => {
            warnings.push(invalid("titres", "« titres » doit être un objet, ignoré"));
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
        warnings.push(invalid(
            "titres",
            format!("« titres » : {rejected} entrée(s) invalide(s) ignorée(s)"),
        ));
    }
    titres
}

// ---------------------------------------------------------------------------
// Écriture
// ---------------------------------------------------------------------------

fn put<T: Serialize>(map: &mut Map<String, Value>, key: &str, value: Option<T>) -> AppResult<()> {
    match value {
        Some(v) => {
            map.insert(key.to_owned(), serde_json::to_value(v)?);
        }
        None => {
            map.remove(key);
        }
    }
    Ok(())
}

/// Objet JSON des champs de dossier (clés inconnues comprises).
fn folder_object(settings: &FolderSettings) -> AppResult<Map<String, Value>> {
    let mut map = settings.extra.clone();
    map.insert("format".into(), Value::from(settings.format.max(FORMAT)));
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

/// Sérialise `map` (JSON indenté), vérifie la taille, puis écrit atomiquement.
fn write_object(path: &Path, map: Map<String, Value>) -> AppResult<()> {
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(map))?;
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

/// Écrit `<dir>/_dossier.json` (atomique, clés inconnues conservées).
pub fn write_folder(dir: &Path, settings: &FolderSettings) -> AppResult<()> {
    write_object(&dir.join(FOLDER_FILE), folder_object(settings)?)
}

/// Écrit `<root>/_histoire.json` (atomique, clés inconnues conservées).
pub fn write_story(root: &Path, settings: &StorySettings) -> AppResult<()> {
    write_object(&root.join(STORY_FILE), story_object(settings)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, text: &str) {
        fs::write(dir.join(name), text).unwrap();
    }

    #[test]
    fn absent_donne_les_valeurs_par_defaut_sans_avertissement() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_folder(dir.path());
        assert!(!loaded.exists && !loaded.damaged);
        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.settings.format, FORMAT);
        let story = load_story(dir.path());
        assert!(!story.exists);
        assert_eq!(story.settings.folder.format, FORMAT);
    }

    #[test]
    fn lit_les_champs_de_la_note() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            STORY_FILE,
            r#"{
              "format": 1, "uuid": "8C1E0000-0000-4000-8000-000000000001", "version": 3,
              "titre": "Le Petit Prince", "sousTitre": "Livre audio", "description": "Lu par…",
              "age": 6, "mode": "chapitres", "debut": "chapitre", "fin": "menu",
              "annoncer": false, "titresLus": { "voix": "Hortense" }
            }"#,
        );
        let loaded = load_story(dir.path());
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        let s = loaded.settings;
        assert_eq!(
            s.uuid.as_deref(),
            Some("8c1e0000-0000-4000-8000-000000000001")
        );
        assert_eq!(s.version, Some(3));
        assert_eq!(s.titre.as_deref(), Some("Le Petit Prince"));
        assert_eq!(s.sous_titre.as_deref(), Some("Livre audio"));
        assert_eq!(s.age, Some(6));
        assert_eq!(s.debut, Some(Debut::Chapitre));
        assert_eq!(s.folder.mode, Some(Mode::Chapitres));
        assert_eq!(s.folder.fin, Some(Fin::Menu));
        assert_eq!(s.folder.annoncer, Some(false));
        assert_eq!(s.titres_lus.unwrap()["voix"], "Hortense");
        assert!(s.folder.extra.is_empty());

        write(
            dir.path(),
            FOLDER_FILE,
            r#"{ "format": 1, "ordre": ["02 Le chat.m4a", "01 Le loup.mp3", "Bonus"],
                 "titres": { "01 Le loup.mp3": "Le grand méchant loup" },
                 "mode": "recueil", "fin": "suivante" }"#,
        );
        let folder = load_folder(dir.path()).settings;
        assert_eq!(folder.ordre, ["02 Le chat.m4a", "01 Le loup.mp3", "Bonus"]);
        assert_eq!(
            folder.title_of("01 LE LOUP.MP3"),
            Some("Le grand méchant loup")
        );
        assert_eq!(folder.mode, Some(Mode::Recueil));
        assert_eq!(folder.fin, Some(Fin::Suivante));
    }

    #[test]
    fn json_abime_donne_les_valeurs_par_defaut_et_un_avertissement() {
        for text in ["{ pas du json", "[1, 2]", "\"texte\"", ""] {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), FOLDER_FILE, text);
            let loaded = load_folder(dir.path());
            assert!(loaded.exists && loaded.damaged, "{text}");
            assert_eq!(loaded.warnings.len(), 1, "{text}");
            assert_eq!(loaded.warnings[0].code, CODE_UNREADABLE);
            assert_eq!(loaded.settings, FolderSettings::default_new());
        }
    }

    #[test]
    fn bom_utf8_tolere() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join(FOLDER_FILE),
            b"\xEF\xBB\xBF{\"mode\":\"chapitres\"}",
        )
        .unwrap();
        let loaded = load_folder(dir.path());
        assert!(loaded.warnings.is_empty());
        assert_eq!(loaded.settings.mode, Some(Mode::Chapitres));
    }

    #[test]
    fn valeurs_invalides_remplacees_par_defaut_avec_avertissement() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            STORY_FILE,
            r#"{ "format": "un", "uuid": "pas-un-uuid", "version": 0, "age": 300,
                 "titre": 12, "mode": "roman", "fin": true, "debut": "milieu",
                 "annoncer": "oui", "titresLus": [], "ordre": "a",
                 "titres": { "a.mp3": 3, "../x": "X", "b.mp3": "  B  " },
                 "inconnu": { "garde": true } }"#,
        );
        let loaded = load_story(dir.path());
        assert!(!loaded.damaged);
        let keys: Vec<_> = loaded
            .warnings
            .iter()
            .map(|w| (w.code, w.key.clone().unwrap()))
            .collect();
        for key in [
            "format",
            "uuid",
            "version",
            "age",
            "titre",
            "mode",
            "fin",
            "debut",
            "annoncer",
            "titresLus",
            "ordre",
            "titres",
        ] {
            assert!(
                keys.contains(&(CODE_INVALID_VALUE, key.to_owned())),
                "{key} : {keys:?}"
            );
        }
        let s = loaded.settings;
        assert_eq!(s.uuid, None);
        assert_eq!(s.version, None);
        assert_eq!(s.age, None);
        assert_eq!(s.titre, None);
        assert_eq!(s.folder.mode, None);
        assert_eq!(s.folder.fin, None);
        assert_eq!(s.folder.annoncer, None);
        assert!(s.folder.ordre.is_empty());
        assert_eq!(s.folder.titres.len(), 1);
        assert_eq!(s.folder.titres["b.mp3"], "B");
        assert_eq!(s.folder.extra["inconnu"]["garde"], true);
    }

    #[test]
    fn ordre_sans_doublon_ni_chemin() {
        let mut warnings = Vec::new();
        let ordre = parse_order(
            Some(serde_json::json!([
                "a.mp3", "A.MP3", "x/y", "..", "", 3, "b"
            ])),
            &mut warnings,
        );
        assert_eq!(ordre, ["a.mp3", "b"]);
        assert_eq!(warnings.len(), 1);
    }

    #[test]
    fn format_plus_recent_lu_au_mieux_et_garde() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            FOLDER_FILE,
            r#"{ "format": 7, "mode": "recueil" }"#,
        );
        let loaded = load_folder(dir.path());
        assert_eq!(loaded.warnings[0].code, CODE_NEWER_FORMAT);
        assert_eq!(loaded.settings.mode, Some(Mode::Recueil));
        write_folder(dir.path(), &loaded.settings).unwrap();
        assert_eq!(load_folder(dir.path()).settings.format, 7);
    }

    #[test]
    fn cles_inconnues_conservees_a_la_reecriture() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            STORY_FILE,
            r#"{ "uuid": "8c1e0000-0000-4000-8000-000000000001", "version": 1,
                 "titresLus": { "voix": "A", "vitesse": 2 },
                 "futur": [1, 2, 3], "mode": "inconnu" }"#,
        );
        let mut story = load_story(dir.path()).settings;
        story.version = Some(2);
        story.folder.mode = Some(Mode::Recueil);
        write_story(dir.path(), &story).unwrap();

        let raw: Value =
            serde_json::from_slice(&fs::read(dir.path().join(STORY_FILE)).unwrap()).unwrap();
        assert_eq!(raw["futur"], serde_json::json!([1, 2, 3]));
        assert_eq!(raw["titresLus"]["vitesse"], 2);
        assert_eq!(raw["version"], 2);
        assert_eq!(raw["mode"], "recueil");
        assert_eq!(raw["format"], 1);
        // Champs absents non écrits.
        assert!(raw.get("titre").is_none() && raw.get("ordre").is_none());
        let again = load_story(dir.path());
        assert!(again.warnings.is_empty(), "{:?}", again.warnings);
        assert_eq!(again.settings, story);
        // Aucun temporaire laissé.
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, [STORY_FILE]);
    }

    #[test]
    fn ecriture_bornee() {
        let dir = tempfile::tempdir().unwrap();
        let mut settings = FolderSettings::default_new();
        settings
            .extra
            .insert("gros".into(), Value::String("x".repeat(2 * 1024 * 1024)));
        assert!(matches!(
            write_folder(dir.path(), &settings),
            Err(AppError::Validation(_))
        ));
        assert!(!dir.path().join(FOLDER_FILE).exists());
    }

    #[test]
    fn lecture_bornee() {
        let dir = tempfile::tempdir().unwrap();
        let big = format!("{{\"x\":\"{}\"}}", "a".repeat(MAX_SETTINGS_BYTES as usize));
        write(dir.path(), FOLDER_FILE, &big);
        let loaded = load_folder(dir.path());
        assert!(loaded.damaged);
        assert_eq!(loaded.warnings[0].code, CODE_UNREADABLE);
    }

    #[test]
    fn heritage_mode_fin_annoncer() {
        let root = FolderSettings {
            mode: Some(Mode::Recueil),
            fin: Some(Fin::Suivante),
            ..FolderSettings::default()
        };
        let top = Inherited::root(&root, Mode::Chapitres);
        assert_eq!(top.mode, Mode::Recueil);
        assert_eq!(top.effective_fin(), Fin::Suivante);
        assert!(!top.annoncer);

        let livre = FolderSettings {
            mode: Some(Mode::Chapitres),
            annoncer: Some(true),
            ..FolderSettings::default()
        };
        let child = top.child(&livre);
        assert_eq!(child.mode, Mode::Chapitres);
        assert!(child.annoncer);
        // `suivante` héritée n'a pas de sens en chapitres : fin effective `menu`.
        assert_eq!(child.fin, Some(Fin::Suivante));
        assert_eq!(child.effective_fin(), Fin::Menu);
        let partie = child.child(&FolderSettings::default());
        assert_eq!(partie, child);

        let sans_reglage = Inherited::root(&FolderSettings::default(), Mode::Chapitres);
        assert_eq!(sans_reglage.mode, Mode::Chapitres);
        assert_eq!(sans_reglage.effective_fin(), Fin::Menu);
    }

    #[test]
    fn prune_retire_les_noms_disparus() {
        let mut settings = FolderSettings {
            ordre: vec!["a.mp3".into(), "B.mp3".into(), "c".into()],
            titres: [
                ("b.MP3".to_owned(), "B".to_owned()),
                ("z".into(), "Z".into()),
            ]
            .into_iter()
            .collect(),
            ..FolderSettings::default()
        };
        let existing: HashSet<String> = ["b.mp3".to_owned(), "c".into()].into_iter().collect();
        settings.prune(&existing);
        assert_eq!(settings.ordre, ["B.mp3", "c"]);
        assert_eq!(settings.titres.keys().collect::<Vec<_>>(), ["b.MP3"]);
    }

    #[test]
    fn reglage_en_lien_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        write(other.path(), FOLDER_FILE, r#"{ "mode": "chapitres" }"#);
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(
            other.path().join(FOLDER_FILE),
            dir.path().join(FOLDER_FILE),
        );
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(
            other.path().join(FOLDER_FILE),
            dir.path().join(FOLDER_FILE),
        );
        if linked.is_err() {
            return; // création de liens non autorisée sur cette machine
        }
        let loaded = load_folder(dir.path());
        assert!(loaded.damaged);
        assert_eq!(loaded.settings.mode, None);
    }
}
