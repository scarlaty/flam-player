//! Statut de compatibilité des histoires.
//!
//! Deux sources, fusionnées par [`build_report`] :
//!
//! - le `status.json` de la campagne de compatibilité (`tools/telmi2flam`, enveloppe
//!   `{schema_version, generated, converter_hash, stories: {uuid: {...}}}`), lu **en lecture
//!   seule** (il appartient à l'outil : jamais renommé ni réécrit ici) ;
//! - le résultat réel de chaque conversion faite par la surcouche, enregistré dans
//!   `<cache>/compat/launcher_results.json` ([`record_conversion`]).
//!
//! Chemin du `status.json` (premier trouvé) : variable d'environnement
//! `FLAM_COMPAT_STATUS_FILE` (chemin absolu), puis en développement
//! `<racine du dépôt>/.cache/telmi/compat/status.json`, sinon `<cache>/compat/status.json`.
//! Le front n'envoie jamais de chemin.
//!
//! Tout ce qui est lu sur disque est non fiable : taille bornée, analyse tolérante (une fiche
//! illisible est ignorée sans invalider les autres), `uuid` revalidés, textes nettoyés et
//! tronqués. Un `schema_version` inconnu n'est pas interprété : le rapport est vide et
//! signale le problème (`issue`).
//!
//! Règles de niveau ([`CompatLevel`]) :
//!
//! | statut de campagne            | niveau         |
//! | ----------------------------- | -------------- |
//! | `OK`                          | `compatible`   |
//! | `OK_AVERTISSEMENTS`           | `warnings`     |
//! | `ECHEC_*` (connu ou non)      | `incompatible` |
//! | autre / absent / illisible    | `untested`     |
//!
//! Fusion avec une conversion locale de même `uuid` (et même version si connue) : un résultat
//! local remplace une fiche de campagne périmée (`stale`) ; sinon le **pire** des deux niveaux
//! l'emporte (égalité : la campagne, plus détaillée).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::catalog::blocking;
use crate::install::convert::Converted;
use crate::settings::{Environment, SettingsStore};
use crate::storage::{self, ConfigState};
use crate::story::{validate_story_id, StoryEntry};
use crate::{AppError, AppResult};

/// Sous-dossier du cache (listé dans `cache::APP_CACHE_SUBDIRS`).
pub const COMPAT_DIR: &str = "compat";
/// Sous-dossier du cache propre à l'application (listé dans `cache::APP_CACHE_SUBDIRS`) :
/// il ne contient que `launcher_results.json`. `compat/` (où peut se trouver un `status.json`
/// posé par l'utilisateur) n'est jamais vidé.
pub const RESULTS_DIR: &str = "compat-local";
/// Fichier de la campagne, produit par `telmi2flam`.
pub const STATUS_FILE: &str = "status.json";
/// Résultats des conversions faites par la surcouche, dans `compat/`.
pub const RESULTS_FILE: &str = "launcher_results.json";
/// Variable d'environnement qui remplace le chemin du `status.json`.
pub const ENV_STATUS_FILE: &str = "FLAM_COMPAT_STATUS_FILE";
/// Version de `status.json` comprise par la surcouche.
pub const SUPPORTED_STATUS_SCHEMA: u64 = 1;
/// Version du schéma de `launcher_results.json`.
pub const RESULTS_SCHEMA: u32 = 1;
/// Taille maximale lue pour un fichier de statut.
pub const MAX_FILE_BYTES: u64 = 8 * 1024 * 1024;
/// Nombre maximal de fiches retenues par fichier.
pub const MAX_STORIES: usize = 20_000;
const MAX_TEXT_CHARS: usize = 300;
const MAX_VERSION_CHARS: usize = 64;
const MAX_WARNINGS: u32 = 1_000_000;

/// Niveau de compatibilité (miroir de `CompatLevel` dans `src/api/compat.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatLevel {
    Untested,
    Compatible,
    Warnings,
    Incompatible,
}

/// Provenance d'une fiche.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompatOrigin {
    /// `status.json` de la campagne.
    Campaign,
    /// Conversion faite par la surcouche.
    Conversion,
}

/// Statut d'une histoire (miroir de `CompatEntry` dans `src/api/compat.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatEntry {
    pub uuid: String,
    /// Version évaluée (`"3"`, `"local"`…), `None` si inconnue.
    pub version: Option<String>,
    pub level: CompatLevel,
    /// Phrase d'une ligne, nettoyée et tronquée (`summary` de la campagne, message local).
    pub reason: Option<String>,
    /// Statut brut de la campagne (`ECHEC_VALIDATION`…) ou code du convertisseur
    /// (`missing_asset`…) : le front en tire une raison par défaut si `reason` est absente.
    pub code: Option<String>,
    pub origin: CompatOrigin,
    /// Fiche de campagne évaluée avec un autre convertisseur que le courant.
    pub stale: bool,
}

/// Problème de lecture du `status.json` (le rapport est alors vide).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CompatIssue {
    /// Fichier illisible, JSON invalide ou de forme inattendue.
    Unreadable,
    /// Fichier plus gros que [`MAX_FILE_BYTES`].
    TooLarge,
    /// `schema_version` absent ou différent de [`SUPPORTED_STATUS_SCHEMA`].
    UnsupportedVersion { found: Option<u64>, supported: u64 },
}

/// Réponse de `compat_statuses` (miroir de `CompatReport` dans `src/api/compat.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CompatReport {
    pub entries: Vec<CompatEntry>,
    pub issue: Option<CompatIssue>,
}

// ---------------------------------------------------------------------------------------
// Lecture de status.json
// ---------------------------------------------------------------------------------------

/// Supprime les caractères de contrôle, replie les espaces et tronque.
fn clean_text(raw: &str, max_chars: usize) -> Option<String> {
    let flat: String = raw
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let text = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.is_empty() {
        return None;
    }
    if text.chars().count() > max_chars {
        let cut: String = text.chars().take(max_chars - 1).collect();
        return Some(format!("{cut}…"));
    }
    Some(text)
}

fn level_of(status: &str) -> CompatLevel {
    match status {
        "OK" => CompatLevel::Compatible,
        "OK_AVERTISSEMENTS" => CompatLevel::Warnings,
        "ECHEC_TELECHARGEMENT" | "ECHEC_CONVERSION" | "ECHEC_VALIDATION" | "ECHEC_EXECUTION" => {
            CompatLevel::Incompatible
        }
        _ => CompatLevel::Untested,
    }
}

/// Fiche de campagne ; `None` si la valeur n'est pas un objet.
fn parse_story(uuid: &str, value: &Value) -> Option<CompatEntry> {
    let object = value.as_object()?;
    let status = object.get("status").and_then(Value::as_str).unwrap_or("");
    let version = match object.get("version") {
        Some(Value::String(s)) => clean_text(s, MAX_VERSION_CHARS),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    Some(CompatEntry {
        uuid: uuid.to_string(),
        version,
        level: level_of(status),
        reason: object
            .get("summary")
            .and_then(Value::as_str)
            .and_then(|s| clean_text(s, MAX_TEXT_CHARS)),
        code: clean_text(status, 64),
        origin: CompatOrigin::Campaign,
        stale: object
            .get("stale")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    })
}

/// Analyse le contenu d'un `status.json`.
pub fn parse_status(bytes: &[u8]) -> Result<Vec<CompatEntry>, CompatIssue> {
    let Ok(Value::Object(root)) = serde_json::from_slice::<Value>(bytes) else {
        return Err(CompatIssue::Unreadable);
    };
    let found = root.get("schema_version").and_then(Value::as_u64);
    if found != Some(SUPPORTED_STATUS_SCHEMA) {
        return Err(CompatIssue::UnsupportedVersion {
            found,
            supported: SUPPORTED_STATUS_SCHEMA,
        });
    }
    let Some(Value::Object(stories)) = root.get("stories") else {
        return Err(CompatIssue::Unreadable);
    };
    Ok(stories
        .iter()
        .filter(|(uuid, _)| validate_story_id(uuid))
        .filter_map(|(uuid, value)| parse_story(uuid, value))
        .take(MAX_STORIES)
        .collect())
}

/// Lit au plus [`MAX_FILE_BYTES`] octets. `Ok(None)` : fichier absent.
fn read_bounded(path: &Path) -> Result<Option<Vec<u8>>, CompatIssue> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(CompatIssue::Unreadable),
    };
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CompatIssue::Unreadable)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(CompatIssue::TooLarge);
    }
    Ok(Some(bytes))
}

/// Fiches de la campagne. Fichier absent : vide, sans problème.
pub fn read_status_file(path: &Path) -> Result<Vec<CompatEntry>, CompatIssue> {
    match read_bounded(path)? {
        None => Ok(Vec::new()),
        Some(bytes) => parse_status(&bytes),
    }
}

// ---------------------------------------------------------------------------------------
// Résultats des conversions de la surcouche
// ---------------------------------------------------------------------------------------

/// Résultat réel d'une conversion (une entrée par `uuid`, la plus récente).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalResult {
    pub version: Option<u32>,
    pub ok: bool,
    /// Code du convertisseur en cas d'échec (`missing_asset`, `exit_1`…).
    pub code: Option<String>,
    /// Message d'échec, nettoyé et tronqué.
    pub message: Option<String>,
    pub warnings: u32,
    /// Empreinte du convertisseur utilisé.
    pub converter: Option<String>,
    /// Date de la conversion (secondes Unix).
    pub at: u64,
}

impl LocalResult {
    /// Le fichier est non fiable : textes nettoyés et tronqués, compteur borné.
    fn sanitized(self) -> Self {
        Self {
            code: self.code.and_then(|c| clean_text(&c, 64)),
            message: self.message.and_then(|m| clean_text(&m, MAX_TEXT_CHARS)),
            converter: self.converter.and_then(|c| clean_text(&c, 64)),
            warnings: self.warnings.min(MAX_WARNINGS),
            ..self
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct ResultsFile {
    schema_version: u32,
    #[serde(default)]
    stories: BTreeMap<String, LocalResult>,
}

fn results_path(cache_root: &Path) -> PathBuf {
    cache_root.join(RESULTS_DIR).join(RESULTS_FILE)
}

/// Résultats enregistrés ; absents, illisibles, trop gros ou d'une version inconnue : vide.
/// Le booléen indique qu'un fichier d'une version future existe (à ne pas écraser).
fn load_results(cache_root: &Path) -> (BTreeMap<String, LocalResult>, bool) {
    let path = results_path(cache_root);
    let bytes = match read_bounded(&path) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return (BTreeMap::new(), false),
        Err(issue) => {
            tracing::warn!(
                "résultats de conversion ignorés ({issue:?}) : {}",
                path.display()
            );
            return (BTreeMap::new(), false);
        }
    };
    match serde_json::from_slice::<ResultsFile>(&bytes) {
        Ok(file) if file.schema_version == RESULTS_SCHEMA => {
            let stories = file
                .stories
                .into_iter()
                .filter(|(uuid, _)| validate_story_id(uuid))
                .take(MAX_STORIES)
                .map(|(uuid, result)| (uuid, result.sanitized()))
                .collect();
            (stories, false)
        }
        Ok(_) => (BTreeMap::new(), true),
        Err(err) => {
            tracing::warn!(
                "résultats de conversion illisibles ({err}) : {}",
                path.display()
            );
            (BTreeMap::new(), false)
        }
    }
}

/// Résultat à enregistrer pour une conversion terminée. `None` pour ce qui n'est pas un
/// résultat du convertisseur (annulation, erreur d'entrée/sortie, convertisseur introuvable).
pub fn outcome_of(
    entry: &StoryEntry,
    converter: &str,
    result: &AppResult<Converted>,
    now: u64,
) -> Option<LocalResult> {
    let base = LocalResult {
        version: entry.version,
        ok: true,
        code: None,
        message: None,
        warnings: 0,
        converter: Some(converter.to_string()),
        at: now,
    };
    match result {
        Ok(converted) => Some(LocalResult {
            warnings: converted.warnings,
            ..base
        }),
        Err(AppError::Conversion { code, message }) => Some(LocalResult {
            ok: false,
            code: clean_text(code, 64),
            message: clean_text(message, MAX_TEXT_CHARS),
            ..base
        }),
        Err(_) => None,
    }
}

/// Enregistre le résultat réel d'une conversion de `entry` (succès ou échec du convertisseur).
/// Le plus récent par `uuid` est conservé. L'appelant sérialise les appels (créneau de
/// conversion unique). Les erreurs ne bloquent jamais l'installation : elles sont tracées.
pub fn record_conversion(
    cache_root: &Path,
    entry: &StoryEntry,
    converter: &str,
    result: &AppResult<Converted>,
    now: u64,
) {
    if let Some(outcome) = outcome_of(entry, converter, result, now) {
        store_outcome(cache_root, &entry.uuid, outcome);
    }
}

/// Écrit `outcome` dans `launcher_results.json` (bloquant : fichiers).
pub fn store_outcome(cache_root: &Path, uuid: &str, outcome: LocalResult) {
    if !validate_story_id(uuid) {
        return;
    }
    let (mut stories, future) = load_results(cache_root);
    if future {
        tracing::warn!("résultats de conversion d'une version plus récente : non modifiés");
        return;
    }
    stories.insert(uuid.to_string(), outcome);
    while stories.len() > MAX_STORIES {
        let oldest = stories
            .iter()
            .min_by_key(|(_, r)| r.at)
            .map(|(uuid, _)| uuid.clone());
        match oldest {
            Some(uuid) => stories.remove(&uuid),
            None => break,
        };
    }
    let file = ResultsFile {
        schema_version: RESULTS_SCHEMA,
        stories,
    };
    if let Err(err) = storage::write_json(&results_path(cache_root), &file) {
        tracing::warn!("enregistrement du résultat de conversion impossible : {err}");
    }
}

fn local_entry(uuid: &str, result: &LocalResult) -> CompatEntry {
    let (level, reason) = match (result.ok, result.warnings) {
        (true, 0) => (CompatLevel::Compatible, None),
        (true, n) => (
            CompatLevel::Warnings,
            Some(format!(
                "Convertie sur cet ordinateur avec {n} avertissement(s)."
            )),
        ),
        (false, _) => (
            CompatLevel::Incompatible,
            Some(match &result.message {
                Some(message) => format!("Échec de la conversion sur cet ordinateur : {message}"),
                None => "Échec de la conversion sur cet ordinateur.".to_string(),
            }),
        ),
    };
    CompatEntry {
        uuid: uuid.to_string(),
        version: result.version.map(|v| v.to_string()),
        level,
        reason,
        code: result.code.clone(),
        origin: CompatOrigin::Conversion,
        stale: false,
    }
}

// ---------------------------------------------------------------------------------------
// Fusion
// ---------------------------------------------------------------------------------------

fn rank(level: CompatLevel) -> u8 {
    match level {
        CompatLevel::Untested => 0,
        CompatLevel::Compatible => 1,
        CompatLevel::Warnings => 2,
        CompatLevel::Incompatible => 3,
    }
}

/// Même histoire : même `uuid`, et même version quand les deux la connaissent.
fn same_story(campaign: &CompatEntry, local: &CompatEntry) -> bool {
    campaign.uuid == local.uuid
        && match (&campaign.version, &local.version) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
}

/// Fusionne la campagne et les conversions locales (règles : documentation du module).
pub fn build_report(
    campaign: Vec<CompatEntry>,
    local: &BTreeMap<String, LocalResult>,
) -> Vec<CompatEntry> {
    let locals: Vec<CompatEntry> = local.iter().map(|(u, r)| local_entry(u, r)).collect();
    let mut used = vec![false; locals.len()];
    let mut out = Vec::with_capacity(campaign.len() + locals.len());
    for entry in campaign {
        let matching = locals
            .iter()
            .enumerate()
            .find(|(_, l)| same_story(&entry, l));
        match matching {
            None => out.push(entry),
            Some((index, l)) => {
                used[index] = true;
                if entry.stale || rank(l.level) > rank(entry.level) {
                    out.push(l.clone());
                } else {
                    out.push(entry);
                }
            }
        }
    }
    for (index, l) in locals.into_iter().enumerate() {
        if !used[index] {
            out.push(l);
        }
    }
    out
}

// ---------------------------------------------------------------------------------------
// Chemins et commande
// ---------------------------------------------------------------------------------------

/// Chemin du `status.json` (voir la documentation du module).
pub fn status_path(
    env: &Environment,
    cache_root: &Path,
    override_path: Option<PathBuf>,
) -> PathBuf {
    if let Some(path) = override_path.filter(|p| p.is_absolute()) {
        return path;
    }
    if env.search_repo_build {
        let dev = env.exe_dir.as_deref().and_then(|exe| {
            exe.ancestors()
                .map(|dir| {
                    dir.join(".cache")
                        .join("telmi")
                        .join(COMPAT_DIR)
                        .join(STATUS_FILE)
                })
                .find(|candidate| candidate.is_file())
        });
        if let Some(path) = dev {
            return path;
        }
    }
    cache_root.join(COMPAT_DIR).join(STATUS_FILE)
}

/// Rapport complet pour un dossier de cache et un chemin de statut donnés.
pub fn report_for(status: &Path, cache_root: &Path) -> CompatReport {
    let (campaign, issue) = match read_status_file(status) {
        Ok(entries) => (entries, None),
        Err(issue) => (Vec::new(), Some(issue)),
    };
    let (local, _) = load_results(cache_root);
    CompatReport {
        entries: build_report(campaign, &local),
        issue,
    }
}

/// Statut de compatibilité de toutes les histoires connues (campagne + conversions locales).
/// Le front associe une histoire à sa fiche par `uuid` (et version).
#[tauri::command]
pub async fn compat_statuses(state: State<'_, ConfigState>) -> AppResult<CompatReport> {
    let env = Environment::current();
    let cache_root = {
        let _guard = state.guard();
        let settings = SettingsStore::new(&state.dir).load()?;
        PathBuf::from(crate::settings::resolve(&settings, &env).0.cache_dir)
    };
    let status = status_path(
        &env,
        &cache_root,
        std::env::var_os(ENV_STATUS_FILE).map(PathBuf::from),
    );
    blocking(move || report_for(&status, &cache_root)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write(dir: &Path, name: &str, contents: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::create_dir_all(dir).unwrap();
        fs::write(&path, contents).unwrap();
        path
    }

    fn envelope(stories: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "generated": "2026-01-01T00:00:00",
            "converter_hash": "0123",
            "stories": stories,
        }))
        .unwrap()
    }

    fn entry(uuid: &str, version: Option<u32>) -> StoryEntry {
        StoryEntry {
            source_id: "s".into(),
            entry_id: format!("s:{uuid}"),
            uuid: uuid.into(),
            version,
            title: "Histoire fictive".into(),
            description: None,
            age: None,
            category: None,
            author: None,
            voice: None,
            license: None,
            thumb_url: None,
            download_url: "https://exemple.org/x.zip".into(),
            size: None,
            updated_at: None,
            kind: crate::story::StoryKind::TelmiZip,
        }
    }

    #[test]
    fn fichier_absent_donne_un_rapport_vide_sans_probleme() {
        let dir = tempfile::tempdir().unwrap();
        let report = report_for(&dir.path().join("status.json"), dir.path());
        assert!(report.entries.is_empty());
        assert_eq!(report.issue, None);
    }

    #[test]
    fn fichier_corrompu_est_signale_sans_etre_modifie() {
        let dir = tempfile::tempdir().unwrap();
        for bad in [&b"{pas du json"[..], b"[1,2]", b"\xff\xfe\x00", b""] {
            let path = write(dir.path(), "status.json", bad);
            let report = report_for(&path, dir.path());
            assert_eq!(report.issue, Some(CompatIssue::Unreadable), "{bad:?}");
            assert!(report.entries.is_empty());
            assert_eq!(fs::read(&path).unwrap(), bad);
        }
        let only: Vec<_> = fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(only.len(), 1, "aucune sauvegarde .bak créée");
    }

    #[test]
    fn version_de_schema_differente_est_refusee() {
        let dir = tempfile::tempdir().unwrap();
        for doc in [
            json!({"schema_version": 2, "stories": {"a": {"status": "OK"}}}),
            json!({"schema_version": 0, "stories": {}}),
            json!({"stories": {"a": {"status": "OK"}}}),
            json!({"schema_version": "1", "stories": {}}),
        ] {
            let path = write(dir.path(), "status.json", doc.to_string().as_bytes());
            let report = report_for(&path, dir.path());
            assert!(matches!(
                report.issue,
                Some(CompatIssue::UnsupportedVersion { supported: 1, .. })
            ));
            assert!(report.entries.is_empty());
        }
        let path = write(
            dir.path(),
            "status.json",
            json!({"schema_version": 2}).to_string().as_bytes(),
        );
        assert_eq!(
            report_for(&path, dir.path()).issue,
            Some(CompatIssue::UnsupportedVersion {
                found: Some(2),
                supported: 1
            })
        );
    }

    #[test]
    fn fichier_trop_gros_est_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "status.json",
            &vec![b' '; MAX_FILE_BYTES as usize + 1],
        );
        assert_eq!(
            report_for(&path, dir.path()).issue,
            Some(CompatIssue::TooLarge)
        );
    }

    #[test]
    fn statuts_connus_inconnus_et_fiches_invalides() {
        let doc = envelope(json!({
            "ok": {"version": "3", "status": "OK", "summary": "Jouable", "stale": false},
            "avert": {"version": 2, "status": "OK_AVERTISSEMENTS", "summary": "Partielle"},
            "echec": {"status": "ECHEC_VALIDATION", "summary": "Image\nmanquante\u{7}"},
            "echec-futur": {"status": "ECHEC_NOUVEAU"},
            "inconnu": {"status": "BIZARRE"},
            "sans-statut": {},
            "perime": {"status": "OK", "stale": true},
            "../evil": {"status": "OK"},
            "mauvais": 12,
        }));
        let entries = parse_status(&doc).unwrap();
        let by = |u: &str| entries.iter().find(|e| e.uuid == u).unwrap();
        assert_eq!(entries.len(), 7, "uuid invalide et fiche non objet ignorés");
        assert_eq!(by("ok").level, CompatLevel::Compatible);
        assert_eq!(by("ok").version.as_deref(), Some("3"));
        assert_eq!(by("avert").level, CompatLevel::Warnings);
        assert_eq!(by("avert").version.as_deref(), Some("2"));
        assert_eq!(by("echec").level, CompatLevel::Incompatible);
        assert_eq!(by("echec").reason.as_deref(), Some("Image manquante"));
        assert_eq!(by("echec").code.as_deref(), Some("ECHEC_VALIDATION"));
        assert_eq!(by("echec-futur").level, CompatLevel::Untested);
        assert_eq!(by("inconnu").level, CompatLevel::Untested);
        assert_eq!(by("sans-statut").level, CompatLevel::Untested);
        assert!(by("perime").stale);
    }

    #[test]
    fn les_textes_sont_tronques() {
        let long = "é".repeat(1000);
        let doc = envelope(json!({"a": {"status": "OK", "summary": long, "version": long}}));
        let e = &parse_status(&doc).unwrap()[0];
        assert_eq!(e.reason.as_ref().unwrap().chars().count(), MAX_TEXT_CHARS);
        assert_eq!(
            e.version.as_ref().unwrap().chars().count(),
            MAX_VERSION_CHARS
        );
    }

    fn converted(warnings: u32) -> AppResult<Converted> {
        Ok(Converted { warnings })
    }

    fn failed(code: &str) -> AppResult<Converted> {
        Err(AppError::Conversion {
            code: code.into(),
            message: "asset manquant".into(),
        })
    }

    #[test]
    fn enregistre_le_resultat_reel_des_conversions() {
        let dir = tempfile::tempdir().unwrap();
        record_conversion(
            dir.path(),
            &entry("a", Some(1)),
            "sha256:aa",
            &converted(0),
            10,
        );
        record_conversion(
            dir.path(),
            &entry("b", None),
            "sha256:aa",
            &converted(2),
            11,
        );
        record_conversion(
            dir.path(),
            &entry("c", Some(3)),
            "sha256:aa",
            &failed("missing_asset"),
            12,
        );
        let (stories, _) = load_results(dir.path());
        assert_eq!(stories.len(), 3);
        assert!(stories["a"].ok && stories["a"].warnings == 0);
        assert_eq!(stories["b"].warnings, 2);
        assert_eq!(stories["c"].code.as_deref(), Some("missing_asset"));
        // Le plus récent remplace.
        record_conversion(
            dir.path(),
            &entry("c", Some(3)),
            "sha256:bb",
            &converted(0),
            20,
        );
        let (stories, _) = load_results(dir.path());
        assert!(stories["c"].ok);
        assert_eq!(stories["c"].converter.as_deref(), Some("sha256:bb"));
    }

    #[test]
    fn annulation_et_erreurs_d_entree_sortie_ne_sont_pas_enregistrees() {
        let dir = tempfile::tempdir().unwrap();
        record_conversion(
            dir.path(),
            &entry("a", None),
            "x",
            &Err(AppError::Cancelled),
            1,
        );
        record_conversion(
            dir.path(),
            &entry("a", None),
            "x",
            &Err(AppError::Io(std::io::Error::other("disque plein"))),
            1,
        );
        assert!(!results_path(dir.path()).exists());
    }

    #[test]
    fn uuid_invalide_non_enregistre() {
        let dir = tempfile::tempdir().unwrap();
        record_conversion(dir.path(), &entry("../x", None), "x", &converted(0), 1);
        assert!(!results_path(dir.path()).exists());
    }

    #[test]
    fn resultats_corrompus_ou_futurs() {
        let dir = tempfile::tempdir().unwrap();
        let compat = dir.path().join(RESULTS_DIR);
        let path = write(&compat, RESULTS_FILE, b"{nope");
        assert!(load_results(dir.path()).0.is_empty());
        // Un enregistrement repart d'un fichier propre.
        record_conversion(dir.path(), &entry("a", None), "x", &converted(0), 1);
        assert_eq!(load_results(dir.path()).0.len(), 1);
        // Version future : intacte.
        let future = br#"{"schema_version": 9, "stories": {}}"#;
        fs::write(&path, future).unwrap();
        record_conversion(dir.path(), &entry("a", None), "x", &converted(0), 2);
        assert_eq!(fs::read(&path).unwrap(), future);
    }

    #[test]
    fn les_resultats_relus_sont_nettoyes_et_bornes() {
        let dir = tempfile::tempdir().unwrap();
        let huge = format!("a\u{7}\nb{}", "é".repeat(5000));
        let doc = json!({"schema_version": 1, "stories": {"a": {
            "version": 1, "ok": false, "code": huge, "message": huge,
            "warnings": 4000000000u64, "converter": huge, "at": 1}}});
        write(
            &dir.path().join(RESULTS_DIR),
            RESULTS_FILE,
            doc.to_string().as_bytes(),
        );
        let (stories, _) = load_results(dir.path());
        let r = &stories["a"];
        assert!(r.message.as_ref().unwrap().starts_with("a b"));
        assert_eq!(r.message.as_ref().unwrap().chars().count(), MAX_TEXT_CHARS);
        assert_eq!(r.code.as_ref().unwrap().chars().count(), 64);
        assert_eq!(r.converter.as_ref().unwrap().chars().count(), 64);
        assert_eq!(r.warnings, MAX_WARNINGS);
    }

    #[test]
    fn echec_local_l_emporte_et_la_campagne_perimee_est_remplacee() {
        let dir = tempfile::tempdir().unwrap();
        let status = write(
            dir.path(),
            "status.json",
            &envelope(json!({
                "a": {"version": "1", "status": "OK", "summary": "Jouable"},
                "b": {"version": "1", "status": "OK_AVERTISSEMENTS", "summary": "Partielle"},
                "c": {"version": "1", "status": "ECHEC_EXECUTION", "stale": true},
                "d": {"version": "1", "status": "OK_AVERTISSEMENTS"},
                "e": {"version": "1", "status": "OK"},
            })),
        );
        record_conversion(
            dir.path(),
            &entry("a", Some(1)),
            "x",
            &failed("missing_asset"),
            1,
        );
        record_conversion(dir.path(), &entry("b", Some(1)), "x", &converted(0), 1);
        record_conversion(dir.path(), &entry("c", Some(1)), "x", &converted(0), 1);
        record_conversion(dir.path(), &entry("d", Some(2)), "x", &failed("exit_1"), 1);
        record_conversion(dir.path(), &entry("z", None), "x", &converted(1), 1);
        let report = report_for(&status, dir.path());
        let all = |u: &str| -> Vec<&CompatEntry> {
            report.entries.iter().filter(|e| e.uuid == u).collect()
        };
        // a : échec local pire que OK.
        assert_eq!(all("a")[0].level, CompatLevel::Incompatible);
        assert_eq!(all("a")[0].origin, CompatOrigin::Conversion);
        assert!(all("a")[0]
            .reason
            .as_ref()
            .unwrap()
            .contains("asset manquant"));
        // b : conversion propre mais campagne avec avertissements -> campagne.
        assert_eq!(all("b")[0].level, CompatLevel::Warnings);
        assert_eq!(all("b")[0].origin, CompatOrigin::Campaign);
        // c : campagne périmée remplacée par le résultat local.
        assert_eq!(all("c")[0].level, CompatLevel::Compatible);
        assert_eq!(all("c")[0].origin, CompatOrigin::Conversion);
        // d : autre version -> deux fiches distinctes.
        assert_eq!(all("d").len(), 2);
        // e : sans résultat local, campagne seule ; z : local seul.
        assert_eq!(all("e")[0].level, CompatLevel::Compatible);
        assert_eq!(all("z")[0].level, CompatLevel::Warnings);
    }

    #[test]
    fn chemin_du_statut() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        let exe = repo.join("apps").join("target").join("debug");
        fs::create_dir_all(&exe).unwrap();
        let env = Environment {
            exe_dir: Some(exe),
            path_var: None,
            default_cache_dir: dir.path().join("c"),
            search_repo_build: true,
            protected_dirs: vec![],
        };
        let cache = dir.path().join("cache");
        let default = cache.join(COMPAT_DIR).join(STATUS_FILE);
        assert_eq!(status_path(&env, &cache, None), default);
        // Dépôt de développement.
        let dev = write(
            &repo.join(".cache").join("telmi").join(COMPAT_DIR),
            STATUS_FILE,
            b"{}",
        );
        assert_eq!(status_path(&env, &cache, None), dev);
        // Variable d'environnement : chemin absolu seulement.
        let custom = dir.path().join("autre.json");
        assert_eq!(status_path(&env, &cache, Some(custom.clone())), custom);
        assert_eq!(
            status_path(&env, &cache, Some(PathBuf::from("relatif.json"))),
            dev
        );
    }
}
