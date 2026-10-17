//! Paramètres de l'application : `settings.json`, valeurs par défaut, validation et
//! détection automatique de `flam-player.exe` et de Python.
//!
//! Les exécutables configurés (`player_path`, `python_path`) seront lancés par l'application :
//! ils doivent être des fichiers `.exe` locaux (ni UNC, ni lecteur réseau). `resolved.*` n'est
//! **pas** une garantie : le fichier peut disparaître après le calcul ; l'appelant (lancement
//! du player, #16) doit revérifier juste avant d'exécuter.

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::sources::{validate_catalog_url, TELMI_STORE_URL};
use crate::storage::{self, ConfigState, MigrationStep, Recovery};
use crate::{AppError, AppResult};

/// Version courante du schéma de `settings.json`.
pub const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "settings.json";
const PLAYER_EXE: &str = "flam-player.exe";
/// Valeur de retour de `GetDriveTypeW` pour un lecteur réseau (`DRIVE_REMOTE`).
#[cfg(windows)]
const DRIVE_REMOTE: u32 = 4;

/// `kind` d'un [`SettingsIssue`] : chemin refusé par la validation (extension, UNC, réseau…).
pub const ISSUE_INVALID_PATH: &str = "invalid_path";
/// `kind` d'un [`SettingsIssue`] : fichier configuré disparu.
pub const ISSUE_NOT_FOUND: &str = "not_found";
/// `kind` d'un [`SettingsIssue`] : seul un alias Microsoft Store a été trouvé pour Python.
pub const ISSUE_STORE_ALIAS: &str = "store_alias";

/// Thème de l'interface (miroir de `ThemePreference` dans `src/theme/theme.ts`).
///
/// Une valeur inconnue (fichier édité à la main ou écrit par une autre version) retombe sur
/// `System` au lieu de faire rejeter tout le fichier.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    /// Suit la préférence du système (doit rester la dernière variante pour `serde(other)`).
    #[default]
    #[serde(other)]
    System,
}

/// Paramètres enregistrés (miroir de `Settings` dans `src/api/settings.ts`).
///
/// Un chemin à `None` signifie « détection automatique » (voir [`resolve`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema_version: u32,
    /// Chemin de `flam-player.exe` ; `None` : détection automatique.
    pub player_path: Option<String>,
    /// Chemin de l'interpréteur Python ; `None` : `python` du PATH.
    pub python_path: Option<String>,
    /// Dossier de cache (téléchargements, histoires converties) ; `None` : dossier par défaut.
    pub cache_dir: Option<String>,
    /// URL du catalogue proposé par défaut.
    pub default_catalog_url: String,
    /// Thème de l'interface. Absent des fichiers écrits avant son ajout : `System`
    /// (`#[serde(default)]`, aucune migration de schéma nécessaire).
    pub theme: Theme,
    /// Téléchargements simultanés du pipeline d'installation (1 à
    /// [`MAX_DOWNLOADS_LIMIT`]). Absent des fichiers écrits avant son ajout :
    /// [`DEFAULT_MAX_DOWNLOADS`] (`#[serde(default)]`, sans migration de schéma).
    pub max_downloads: u32,
}

/// Téléchargements simultanés par défaut.
pub const DEFAULT_MAX_DOWNLOADS: u32 = 2;
/// Téléchargements simultanés au plus.
pub const MAX_DOWNLOADS_LIMIT: u32 = 4;

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            player_path: None,
            python_path: None,
            cache_dir: None,
            default_catalog_url: TELMI_STORE_URL.into(),
            theme: Theme::default(),
            max_downloads: DEFAULT_MAX_DOWNLOADS,
        }
    }
}

/// Valeurs effectives après détection automatique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedSettings {
    /// `None` si le player n'a été ni configuré (valide) ni trouvé.
    pub player_path: Option<String>,
    /// Python embarqué s'il est présent ([`detect_bundled_python`]), sinon Python configuré
    /// (valide), sinon trouvé dans le PATH ; `None` si aucun.
    pub python_path: Option<String>,
    pub cache_dir: String,
}

/// Problème détecté en relisant les paramètres (miroir de `SettingsIssue` côté TypeScript).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsIssue {
    /// Champ concerné : `player_path`, `python_path` ou `cache_dir`.
    pub field: String,
    /// `invalid_path`, `not_found` ou `store_alias`.
    pub kind: String,
    pub message: String,
}

/// Réponse de `settings_get` / `settings_set`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsView {
    pub settings: Settings,
    pub resolved: ResolvedSettings,
    /// Valeurs configurées devenues invalides (remplacées par la détection automatique)
    /// ou détection incomplète.
    pub issues: Vec<SettingsIssue>,
    /// Présent tant que l'utilisateur n'a pas acquitté (`config_ack_recovery`) la
    /// restauration de `settings.json` aux valeurs par défaut.
    pub recovery: Option<Recovery>,
}

/// Contexte de détection, injectable pour les tests.
#[derive(Debug, Clone)]
pub struct Environment {
    /// Dossier de l'exécutable de l'application.
    pub exe_dir: Option<PathBuf>,
    /// Valeur de la variable `PATH`.
    pub path_var: Option<OsString>,
    /// Dossier de cache utilisé quand aucun n'est configuré.
    pub default_cache_dir: PathBuf,
    /// Cherche aussi `<racine du dépôt>/build/flam-player.exe` (développement).
    pub search_repo_build: bool,
    /// Dossiers refusés comme dossier de cache (dossier utilisateur, dossier Windows…).
    pub protected_dirs: Vec<PathBuf>,
}

impl Environment {
    /// Environnement réel du processus.
    pub fn current() -> Self {
        Self {
            exe_dir: std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(Path::to_path_buf)),
            path_var: std::env::var_os("PATH"),
            default_cache_dir: storage::default_cache_dir(),
            search_repo_build: cfg!(debug_assertions),
            protected_dirs: protected_dirs(),
        }
    }
}

/// Dossiers qui ne peuvent pas servir de cache : l'application y créerait des sous-dossiers
/// (et en supprimerait) au milieu de données de l'utilisateur ou du système.
pub fn protected_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = dirs::home_dir().into_iter().collect();
    for var in ["SystemRoot", "windir"] {
        if let Some(value) = std::env::var_os(var) {
            dirs.push(PathBuf::from(value));
        }
    }
    dirs
}

/// Migrations de schéma de `settings.json` (aucune pour l'instant : v0 -> v1 ajoute
/// seulement `schema_version`).
fn migrate(_from: u32, _object: &mut serde_json::Map<String, serde_json::Value>) {}
const MIGRATE: MigrationStep = migrate;

fn is_nonempty_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
}

/// Chemin UNC (`\\serveur\partage`, `\\?\UNC\…`) ou lecteur réseau (Windows).
pub fn is_network_path(path: &Path) -> bool {
    let text = path.to_string_lossy();
    if text.starts_with("\\\\") || text.starts_with("//") {
        return true;
    }
    #[cfg(windows)]
    if let Some(Component::Prefix(prefix)) = path.components().next() {
        use std::os::windows::ffi::OsStrExt;
        use std::path::Prefix;
        if let Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) = prefix.kind() {
            let root: Vec<u16> = std::ffi::OsString::from(format!("{}:\\", letter as char))
                .encode_wide()
                .chain(Some(0))
                .collect();
            // SAFETY : `root` est une chaîne UTF-16 terminée par un zéro, valide pendant l'appel.
            let drive_type =
                unsafe { windows_sys::Win32::Storage::FileSystem::GetDriveTypeW(root.as_ptr()) };
            return drive_type == DRIVE_REMOTE;
        }
    }
    false
}

/// Vérifie un exécutable configuré : chemin absolu local, extension `.exe`, fichier existant.
/// L'erreur porte le `kind` du [`SettingsIssue`] correspondant et un message lisible.
fn check_exe(label: &str, raw: &str) -> Result<(), (&'static str, String)> {
    let path = Path::new(raw);
    let invalid = |reason: &str| Err((ISSUE_INVALID_PATH, format!("{label} : « {raw} » {reason}")));
    if !path.is_absolute() {
        return invalid("doit être un chemin absolu");
    }
    if is_network_path(path) {
        return invalid("ne doit pas être un chemin réseau (UNC ou lecteur réseau)");
    }
    let is_exe = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    if !is_exe {
        return invalid("doit être un fichier .exe");
    }
    if !path.is_file() {
        return Err((
            ISSUE_NOT_FOUND,
            format!("{label} : « {raw} » n'existe pas ou n'est pas un fichier"),
        ));
    }
    Ok(())
}

/// Vérifie un dossier de cache : absolu, pas un fichier, ni racine de disque ni dossier protégé.
/// L'application ne supprime jamais que les sous-dossiers qu'elle a elle-même créés dans ce
/// dossier ; ces garde-fous évitent de la pointer sur un dossier contenant autre chose.
fn check_cache_dir(raw: &str, protected: &[PathBuf]) -> Result<(), (&'static str, String)> {
    let path = Path::new(raw);
    let invalid = |reason: &str| {
        Err((
            ISSUE_INVALID_PATH,
            format!("dossier de cache : « {raw} » {reason}"),
        ))
    };
    if !path.is_absolute() || path.is_file() {
        return invalid("doit être un chemin de dossier absolu");
    }
    if path.parent().is_none() {
        return invalid("ne peut pas être la racine d'un disque");
    }
    if protected.iter().any(|dir| same_path(dir, path)) {
        return invalid("ne peut pas être le dossier utilisateur ou le dossier Windows");
    }
    Ok(())
}

/// Compare deux chemins composant par composant, sans tenir compte de la casse ni des `/` ou
/// `\` finaux.
fn same_path(a: &Path, b: &Path) -> bool {
    let parts = |path: &Path| -> Vec<String> {
        path.components()
            .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
            .collect()
    };
    parts(a) == parts(b)
}

/// Cherche `flam-player.exe` à côté de l'exécutable de l'application, sinon (développement)
/// dans `build/` d'un dossier ancêtre (racine du dépôt).
pub fn detect_player(env: &Environment) -> Option<PathBuf> {
    let exe_dir = env.exe_dir.as_deref()?;
    let beside = exe_dir.join(PLAYER_EXE);
    if beside.is_file() {
        return Some(beside);
    }
    if env.search_repo_build {
        return exe_dir
            .ancestors()
            .map(|dir| dir.join("build").join(PLAYER_EXE))
            .find(|candidate| candidate.is_file());
    }
    None
}

/// Dossier du convertisseur embarqué, à côté de l'exécutable (ressource Tauri `telmi2flam/`
/// produite par `scripts/build-converter.ps1`, #71).
pub const BUNDLED_CONVERTER_DIR: &str = "telmi2flam";

/// Python embarqué avec le convertisseur : `<exe>/telmi2flam/python/python.exe`. Retenu
/// seulement si le script embarqué `<exe>/telmi2flam/telmi2flam.py` l'accompagne : la
/// détection du convertisseur (`install::detect_converter`) prend alors ce même script, et
/// l'interpréteur embarqué (chemins d'import fixés par son `._pth`) n'est jamais associé à
/// un autre script.
pub fn detect_bundled_python(env: &Environment) -> Option<PathBuf> {
    let dir = env.exe_dir.as_deref()?.join(BUNDLED_CONVERTER_DIR);
    let python = dir.join("python").join("python.exe");
    (is_nonempty_file(&python) && dir.join("telmi2flam.py").is_file()).then_some(python)
}

/// Dossier de ffmpeg embarqué, à côté de l'exécutable (ressource Tauri `ffmpeg/` produite par
/// `scripts/build-ffmpeg.ps1`, #209) : `ffmpeg.exe`, ses DLL, `LICENSE` et `BUILD.txt`.
pub const BUNDLED_FFMPEG_DIR: &str = "ffmpeg";

/// Nom de l'exécutable ffmpeg.
pub const FFMPEG_EXE: &str = if cfg!(windows) {
    "ffmpeg.exe"
} else {
    "ffmpeg"
};

/// Cherche le ffmpeg LGPL embarqué : `<exe>/ffmpeg/ffmpeg.exe` sinon (développement)
/// `build/ffmpeg/ffmpeg.exe` d'un dossier ancêtre. Aucun ffmpeg du `PATH` n'est jamais retenu
/// (build GPL possible) et aucun processus n'est lancé. Pas de réglage de surcharge pour
/// l'instant : la conversion audio (#214) en décidera.
pub fn ffmpeg_path(env: &Environment) -> Option<PathBuf> {
    let exe_dir = env.exe_dir.as_deref()?;
    let beside = exe_dir.join(BUNDLED_FFMPEG_DIR).join(FFMPEG_EXE);
    if is_nonempty_file(&beside) {
        return Some(beside);
    }
    if env.search_repo_build {
        return exe_dir
            .ancestors()
            .map(|dir| dir.join("build").join(BUNDLED_FFMPEG_DIR).join(FFMPEG_EXE))
            .find(|candidate| is_nonempty_file(candidate));
    }
    None
}

/// Résultat de la recherche de Python.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PythonDetection {
    pub path: Option<PathBuf>,
    /// Un alias du Microsoft Store (fichier vide dans `WindowsApps`) a été rencontré et ignoré.
    pub store_alias_ignored: bool,
}

fn in_windows_apps(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps"))
}

/// Cherche Python dans le PATH (sans exécuter de script ni de processus) : `python.exe`,
/// `python3.exe`, puis le lanceur Windows `py.exe` (`python3` / `python` hors Windows).
///
/// Les entrées relatives du PATH sont ignorées. Les alias du Microsoft Store (fichiers vides
/// dans `WindowsApps`) sont **ignorés** : savoir s'ils désignent un Python réellement installé
/// exigerait de les lancer, ce que la détection s'interdit. L'information est remontée
/// (`store_alias_ignored`) pour que l'interface invite à choisir le chemin de Python.
pub fn detect_python(env: &Environment) -> PythonDetection {
    let names: &[&str] = if cfg!(windows) {
        &["python.exe", "python3.exe", "py.exe"]
    } else {
        &["python3", "python"]
    };
    let mut detection = PythonDetection {
        path: None,
        store_alias_ignored: false,
    };
    let Some(path_var) = env.path_var.as_ref() else {
        return detection;
    };
    for dir in std::env::split_paths(path_var).filter(|dir| dir.is_absolute()) {
        for name in names {
            let candidate = dir.join(name);
            if is_nonempty_file(&candidate) {
                detection.path = Some(candidate);
                return detection;
            }
            if in_windows_apps(&candidate) && std::fs::symlink_metadata(&candidate).is_ok() {
                detection.store_alias_ignored = true;
            }
        }
    }
    detection
}

/// Calcule les valeurs effectives et les problèmes rencontrés. Une valeur configurée qui ne
/// passe plus la validation (extension, chemin réseau, fichier disparu…) est écartée au
/// profit de la détection automatique, avec un [`SettingsIssue`].
pub fn resolve(settings: &Settings, env: &Environment) -> (ResolvedSettings, Vec<SettingsIssue>) {
    let as_string = |path: PathBuf| path.to_string_lossy().into_owned();
    let mut issues = Vec::new();
    let mut issue = |field: &str, (kind, message): (&str, String)| {
        issues.push(SettingsIssue {
            field: field.into(),
            kind: kind.into(),
            message,
        });
    };

    let configured_player =
        settings
            .player_path
            .as_deref()
            .and_then(|raw| match check_exe("chemin du player", raw) {
                Ok(()) => Some(raw.to_string()),
                Err(err) => {
                    issue("player_path", err);
                    None
                }
            });
    let configured_python =
        settings
            .python_path
            .as_deref()
            .and_then(|raw| match check_exe("chemin de Python", raw) {
                Ok(()) => Some(raw.to_string()),
                Err(err) => {
                    issue("python_path", err);
                    None
                }
            });
    let configured_cache = settings.cache_dir.as_deref().and_then(|raw| {
        match check_cache_dir(raw, &env.protected_dirs) {
            Ok(()) => Some(raw.to_string()),
            Err(err) => {
                issue("cache_dir", err);
                None
            }
        }
    });

    // Python embarqué (application installée) prioritaire, puis Python configuré, puis PATH.
    let bundled_python = detect_bundled_python(env).map(as_string);
    let python = match bundled_python.or(configured_python) {
        Some(path) => Some(path),
        None => {
            let detection = detect_python(env);
            if detection.path.is_none() && detection.store_alias_ignored {
                issue(
                    "python_path",
                    (
                        ISSUE_STORE_ALIAS,
                        "seul un alias Python du Microsoft Store a été trouvé ; il est ignoré, \
                         indiquez le chemin de python.exe dans les paramètres"
                            .to_string(),
                    ),
                );
            }
            detection.path.map(as_string)
        }
    };

    let resolved = ResolvedSettings {
        player_path: configured_player.or_else(|| detect_player(env).map(as_string)),
        python_path: python,
        cache_dir: configured_cache.unwrap_or_else(|| as_string(env.default_cache_dir.clone())),
    };
    (resolved, issues)
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn to_validation((_, message): (&str, String)) -> AppError {
    AppError::Validation(message)
}

/// Normalise (espaces, valeurs vides -> automatique) et valide des paramètres, avec les
/// dossiers protégés du système courant.
pub fn validate(settings: Settings) -> AppResult<Settings> {
    validate_with(settings, &protected_dirs())
}

/// Comme [`validate`], avec une liste de dossiers protégés explicite.
pub fn validate_with(settings: Settings, protected: &[PathBuf]) -> AppResult<Settings> {
    let player_path = clean(settings.player_path);
    let python_path = clean(settings.python_path);
    let cache_dir = clean(settings.cache_dir);
    if let Some(path) = &player_path {
        check_exe("chemin du player", path).map_err(to_validation)?;
    }
    if let Some(path) = &python_path {
        check_exe("chemin de Python", path).map_err(to_validation)?;
    }
    if let Some(dir) = &cache_dir {
        check_cache_dir(dir, protected).map_err(to_validation)?;
    }
    if !(1..=MAX_DOWNLOADS_LIMIT).contains(&settings.max_downloads) {
        return Err(AppError::Validation(format!(
            "téléchargements simultanés : {} hors de l'intervalle 1 à {MAX_DOWNLOADS_LIMIT}",
            settings.max_downloads
        )));
    }
    Ok(Settings {
        schema_version: SCHEMA_VERSION,
        player_path,
        python_path,
        cache_dir,
        default_catalog_url: validate_catalog_url(&settings.default_catalog_url)?,
        theme: settings.theme,
        max_downloads: settings.max_downloads,
    })
}

/// Accès à `settings.json` dans un dossier de configuration (injecté pour les tests).
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            dir: config_dir.to_path_buf(),
        }
    }

    /// Charge les paramètres. Absent : valeurs par défaut, écrites sur disque. Corrompu :
    /// sauvegarde `*.bak`, valeurs par défaut et restauration mémorisée. Version future : erreur.
    pub fn load(&self) -> AppResult<Settings> {
        storage::load_or_default(&self.dir, FILE_NAME, SCHEMA_VERSION, MIGRATE)
    }

    /// Remplace **l'objet complet** après validation (pas de fusion : un champ à `None` repasse
    /// en automatique). Le fichier existant est d'abord relu par le même chemin que `load` :
    /// version future -> refus sans le toucher ; fichier corrompu -> sauvegardé en `*.bak`.
    /// Renvoie les paramètres normalisés.
    pub fn update(&self, settings: Settings) -> AppResult<Settings> {
        self.load()?;
        let settings = validate(settings)?;
        storage::write_json(&self.dir.join(FILE_NAME), &settings)?;
        Ok(settings)
    }

    /// Paramètres, valeurs effectives, problèmes et restauration à signaler.
    pub fn view(&self, env: &Environment) -> AppResult<SettingsView> {
        let settings = self.load()?;
        let (resolved, issues) = resolve(&settings, env);
        let recovery = storage::pending_recovery(&self.dir, FILE_NAME);
        Ok(SettingsView {
            settings,
            resolved,
            issues,
            recovery,
        })
    }

    /// Dossier de cache effectif, **sans** le créer (lecture seule, pour les traitements qui n'ont
    /// rien à faire si le cache n'existe pas encore, comme la purge des vignettes au démarrage).
    pub fn cache_dir(&self, env: &Environment) -> AppResult<PathBuf> {
        let (resolved, _) = resolve(&self.load()?, env);
        Ok(PathBuf::from(resolved.cache_dir))
    }

    /// Dossier de cache effectif, créé s'il n'existe pas.
    pub fn resolved_cache_dir(&self, env: &Environment) -> AppResult<PathBuf> {
        let dir = self.cache_dir(env)?;
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }
}

/// Renvoie les paramètres enregistrés, les valeurs effectives et les problèmes détectés.
#[tauri::command]
pub async fn settings_get(state: State<'_, ConfigState>) -> AppResult<SettingsView> {
    let _guard = state.guard();
    SettingsStore::new(&state.dir).view(&Environment::current())
}

/// Valide et enregistre les paramètres. Remplace l'objet complet (pas de fusion partielle).
#[tauri::command]
pub async fn settings_set(
    state: State<'_, ConfigState>,
    settings: Settings,
) -> AppResult<SettingsView> {
    let _guard = state.guard();
    let store = SettingsStore::new(&state.dir);
    store.update(settings)?;
    store.view(&Environment::current())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn env_in(dir: &Path) -> Environment {
        Environment {
            exe_dir: Some(dir.join("app")),
            path_var: None,
            default_cache_dir: dir.join("cache-defaut"),
            search_repo_build: false,
            protected_dirs: vec![],
        }
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "contenu").unwrap();
    }

    fn baks(dir: &Path) -> usize {
        fs::read_dir(dir)
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".bak")
            })
            .count()
    }

    #[test]
    fn creation_par_defaut() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let settings = store.load().unwrap();
        assert_eq!(settings, Settings::default());
        assert_eq!(settings.default_catalog_url, TELMI_STORE_URL);
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(json["schema_version"], 1);
        assert!(json["player_path"].is_null());
    }

    #[test]
    fn fichier_corrompu_sauvegarde_puis_defaut_et_signale() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "\u{0}\u{0} corrompu").unwrap();
        let store = SettingsStore::new(dir.path());
        let view = store.view(&env_in(dir.path())).unwrap();
        assert_eq!(view.settings, Settings::default());
        assert_eq!(baks(dir.path()), 1);
        let recovery = view.recovery.unwrap();
        assert_eq!(recovery.file, "settings.json");
        assert!(dir.path().join(&recovery.backup).exists());
        // Toujours signalée à la lecture suivante, jusqu'à l'acquittement.
        let again = store.view(&env_in(dir.path())).unwrap();
        assert_eq!(again.recovery, Some(recovery));
        storage::ack_recovery(dir.path(), None).unwrap();
        assert_eq!(store.view(&env_in(dir.path())).unwrap().recovery, None);
    }

    #[test]
    fn champs_manquants_prennent_leur_valeur_par_defaut() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schema_version":1,"cache_dir":"C:\\cache"}"#,
        )
        .unwrap();
        let settings = SettingsStore::new(dir.path()).load().unwrap();
        assert_eq!(settings.cache_dir.as_deref(), Some("C:\\cache"));
        assert_eq!(settings.default_catalog_url, TELMI_STORE_URL);
    }

    #[test]
    fn fichier_sans_theme_prend_le_theme_systeme() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schema_version":1,"player_path":null}"#,
        )
        .unwrap();
        let settings = SettingsStore::new(dir.path()).load().unwrap();
        assert_eq!(settings.theme, Theme::System);
    }

    #[test]
    fn theme_est_persiste_et_une_valeur_inconnue_retombe_sur_systeme() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        for theme in [Theme::Light, Theme::Dark, Theme::System] {
            let saved = store
                .update(Settings {
                    theme,
                    ..Settings::default()
                })
                .unwrap();
            assert_eq!(saved.theme, theme);
            assert_eq!(store.load().unwrap().theme, theme);
        }
        let json = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(json.contains(r#""theme": "system""#), "{json}");

        fs::write(
            dir.path().join("settings.json"),
            r#"{"schema_version":1,"theme":"violet"}"#,
        )
        .unwrap();
        let settings = store.load().unwrap();
        assert_eq!(settings.theme, Theme::System);
        assert_eq!(baks(dir.path()), 0, "le fichier n'est pas jugé corrompu");
    }

    #[test]
    fn fichier_sans_version_est_migre() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"default_catalog_url":"https://exemple.org/c.json"}"#,
        )
        .unwrap();
        let settings = SettingsStore::new(dir.path()).load().unwrap();
        assert_eq!(settings.schema_version, SCHEMA_VERSION);
        assert_eq!(settings.default_catalog_url, "https://exemple.org/c.json");
    }

    #[test]
    fn version_future_refusee_a_la_lecture() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), r#"{"schema_version":42}"#).unwrap();
        let err = SettingsStore::new(dir.path()).load().unwrap_err();
        assert_eq!(err.kind(), "config_version");
    }

    #[test]
    fn update_refuse_une_version_future_sans_toucher_au_fichier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, r#"{"schema_version":42}"#).unwrap();
        let err = SettingsStore::new(dir.path())
            .update(Settings::default())
            .unwrap_err();
        assert_eq!(err.kind(), "config_version");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            r#"{"schema_version":42}"#
        );
        assert_eq!(baks(dir.path()), 0);
    }

    #[test]
    fn update_sauvegarde_un_fichier_corrompu_avant_d_ecrire() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        fs::write(&path, "{ corrompu").unwrap();
        let store = SettingsStore::new(dir.path());
        let new = Settings {
            default_catalog_url: "https://exemple.org/c.json".into(),
            ..Settings::default()
        };
        store.update(new.clone()).unwrap();
        assert_eq!(baks(dir.path()), 1);
        assert_eq!(store.load().unwrap(), new);
        assert!(pending_recovery_exists(dir.path()));
    }

    fn pending_recovery_exists(dir: &Path) -> bool {
        storage::pending_recovery(dir, "settings.json").is_some()
    }

    #[test]
    fn update_remplace_l_objet_complet() {
        let dir = tempfile::tempdir().unwrap();
        let player = dir.path().join("p/flam-player.exe");
        touch(&player);
        let store = SettingsStore::new(dir.path());
        store
            .update(Settings {
                player_path: Some(player.display().to_string()),
                ..Settings::default()
            })
            .unwrap();
        // Un second enregistrement sans player_path le repasse en automatique.
        let saved = store.update(Settings::default()).unwrap();
        assert_eq!(saved.player_path, None);
        assert_eq!(store.load().unwrap().player_path, None);
    }

    #[test]
    fn telechargements_simultanes_bornes_et_par_defaut() {
        let old: Settings = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(old.max_downloads, DEFAULT_MAX_DOWNLOADS);
        for (n, ok) in [
            (0, false),
            (1, true),
            (MAX_DOWNLOADS_LIMIT, true),
            (9, false),
        ] {
            let result = validate_with(
                Settings {
                    max_downloads: n,
                    ..Settings::default()
                },
                &[],
            );
            assert_eq!(result.is_ok(), ok, "{n}");
        }
    }

    #[test]
    fn mise_a_jour_valide_est_persistee_et_normalisee() {
        let dir = tempfile::tempdir().unwrap();
        let player = dir.path().join("p/FLAM-Player.EXE");
        touch(&player);
        let store = SettingsStore::new(dir.path());
        let saved = store
            .update(Settings {
                player_path: Some(format!("  {} ", player.display())),
                python_path: Some("   ".into()),
                cache_dir: Some(dir.path().join("cache").display().to_string()),
                default_catalog_url: " https://exemple.org/c.json ".into(),
                schema_version: 0,
                theme: Theme::Dark,
                max_downloads: 3,
            })
            .unwrap();
        assert_eq!(saved.schema_version, SCHEMA_VERSION);
        assert_eq!(saved.player_path, Some(player.display().to_string()));
        assert_eq!(saved.python_path, None);
        assert_eq!(store.load().unwrap(), saved);
    }

    #[test]
    fn mise_a_jour_invalide_est_refusee_sans_ecriture() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let missing = dir.path().join("absent.exe").display().to_string();
        let a_file = dir.path().join("f.txt");
        touch(&a_file);
        let not_exe = dir.path().join("script.bat");
        touch(&not_exe);
        let cases = [
            Settings {
                player_path: Some(missing.clone()),
                ..Settings::default()
            },
            Settings {
                python_path: Some("python".into()),
                ..Settings::default()
            },
            Settings {
                python_path: Some(not_exe.display().to_string()),
                ..Settings::default()
            },
            Settings {
                cache_dir: Some("relatif".into()),
                ..Settings::default()
            },
            Settings {
                cache_dir: Some(a_file.display().to_string()),
                ..Settings::default()
            },
            Settings {
                default_catalog_url: "n'importe quoi".into(),
                ..Settings::default()
            },
        ];
        for settings in cases {
            let err = store.update(settings.clone()).unwrap_err();
            assert_eq!(err.kind(), "validation", "{settings:?}");
        }
        // Le fichier par défaut est créé par la relecture, mais rien n'a été modifié.
        assert_eq!(store.load().unwrap(), Settings::default());
    }

    #[test]
    fn extension_exe_exigee_sans_tenir_compte_de_la_casse() {
        let dir = tempfile::tempdir().unwrap();
        let ok = dir.path().join("Python.ExE");
        let ko = dir.path().join("python.cmd");
        touch(&ok);
        touch(&ko);
        assert!(check_exe("x", &ok.display().to_string()).is_ok());
        let (kind, _) = check_exe("x", &ko.display().to_string()).unwrap_err();
        assert_eq!(kind, ISSUE_INVALID_PATH);
    }

    #[test]
    fn chemins_unc_et_reseau_refuses() {
        for bad in [
            "\\\\serveur\\partage\\python.exe",
            "\\\\?\\UNC\\serveur\\partage\\python.exe",
            "//serveur/partage/python.exe",
        ] {
            assert!(is_network_path(Path::new(bad)), "{bad}");
            let (kind, _) = check_exe("x", bad).unwrap_err();
            assert_eq!(kind, ISSUE_INVALID_PATH, "{bad}");
        }
        assert!(!is_network_path(Path::new("C:\\Windows\\notepad.exe")));
    }

    #[test]
    fn cache_refuse_racine_dossier_utilisateur_et_windows() {
        let dir = tempfile::tempdir().unwrap();
        let protected = vec![dir.path().join("Users").join("Moi")];
        let home = &protected[0];
        fs::create_dir_all(home).unwrap();
        let root = dir.path().ancestors().last().unwrap().display().to_string();
        let upper = home.display().to_string().to_uppercase();
        for bad in [
            root,
            home.display().to_string(),
            format!("{}\\", home.display()),
            upper,
        ] {
            let err = validate_with(
                Settings {
                    cache_dir: Some(bad.clone()),
                    ..Settings::default()
                },
                &protected,
            )
            .unwrap_err();
            assert_eq!(err.kind(), "validation", "{bad}");
        }
        // Un sous-dossier est accepté.
        let ok = validate_with(
            Settings {
                cache_dir: Some(home.join("cache").display().to_string()),
                ..Settings::default()
            },
            &protected,
        );
        assert!(ok.is_ok());
    }

    #[test]
    fn le_vrai_dossier_utilisateur_est_refuse() {
        let Some(home) = dirs::home_dir() else { return };
        let err = validate(Settings {
            cache_dir: Some(home.display().to_string()),
            ..Settings::default()
        })
        .unwrap_err();
        assert_eq!(err.kind(), "validation");
    }

    #[test]
    fn detection_du_player_a_cote_de_l_appli() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        assert_eq!(detect_player(&env), None);
        let player = dir.path().join("app").join(PLAYER_EXE);
        touch(&player);
        assert_eq!(detect_player(&env), Some(player));
    }

    #[test]
    fn detection_du_player_dans_build_du_depot_en_dev() {
        let dir = tempfile::tempdir().unwrap();
        let player = dir.path().join("build").join(PLAYER_EXE);
        touch(&player);
        let mut env = env_in(dir.path());
        env.exe_dir = Some(dir.path().join("target/debug"));
        assert_eq!(detect_player(&env), None, "désactivé hors développement");
        env.search_repo_build = true;
        assert_eq!(detect_player(&env), Some(player));
    }

    #[test]
    fn le_player_a_cote_prime_sur_build() {
        let dir = tempfile::tempdir().unwrap();
        touch(&dir.path().join("build").join(PLAYER_EXE));
        let beside = dir.path().join("app").join(PLAYER_EXE);
        touch(&beside);
        let mut env = env_in(dir.path());
        env.search_repo_build = true;
        assert_eq!(detect_player(&env), Some(beside));
    }

    fn python_name() -> &'static str {
        if cfg!(windows) {
            "python.exe"
        } else {
            "python3"
        }
    }

    fn path_of(dirs: &[&Path]) -> Option<OsString> {
        Some(std::env::join_paths(dirs).unwrap())
    }

    #[test]
    fn detection_de_python_dans_le_path() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("vide");
        let good = dir.path().join("bon");
        fs::create_dir_all(&empty).unwrap();
        touch(&good.join(python_name()));
        let mut env = env_in(dir.path());
        env.path_var = path_of(&[&empty, &good]);
        assert_eq!(detect_python(&env).path, Some(good.join(python_name())));
        env.path_var = path_of(&[&empty]);
        assert_eq!(detect_python(&env).path, None);
        env.path_var = None;
        assert_eq!(detect_python(&env).path, None);
    }

    #[cfg(windows)]
    #[test]
    fn detection_de_python3_et_du_lanceur_py() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        touch(&a.join("py.exe"));
        touch(&b.join("python3.exe"));
        let mut env = env_in(dir.path());
        env.path_var = path_of(&[&a]);
        assert_eq!(detect_python(&env).path, Some(a.join("py.exe")));
        env.path_var = path_of(&[&b]);
        assert_eq!(detect_python(&env).path, Some(b.join("python3.exe")));
        // Dans un même dossier, python.exe prime sur py.exe.
        touch(&a.join("python.exe"));
        env.path_var = path_of(&[&a]);
        assert_eq!(detect_python(&env).path, Some(a.join("python.exe")));
    }

    #[test]
    fn les_entrees_relatives_du_path_sont_ignorees() {
        let dir = tempfile::tempdir().unwrap();
        let rel = Path::new("relatif");
        let mut env = env_in(dir.path());
        env.path_var = Some(std::env::join_paths([rel]).unwrap());
        // Même si le dossier relatif existe (dans le dossier courant), il est ignoré.
        let cwd_python = std::env::current_dir().unwrap().join(rel);
        fs::create_dir_all(&cwd_python).unwrap();
        touch(&cwd_python.join(python_name()));
        let detection = detect_python(&env);
        fs::remove_dir_all(&cwd_python).unwrap();
        assert_eq!(detection.path, None);
    }

    #[test]
    fn un_fichier_vide_n_est_pas_python() {
        let dir = tempfile::tempdir().unwrap();
        let alias = dir.path().join("alias");
        fs::create_dir_all(&alias).unwrap();
        fs::write(alias.join(python_name()), "").unwrap();
        let mut env = env_in(dir.path());
        env.path_var = path_of(&[&alias]);
        assert_eq!(
            detect_python(&env),
            PythonDetection {
                path: None,
                store_alias_ignored: false
            }
        );
    }

    #[test]
    fn un_alias_du_store_est_ignore_et_signale() {
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("Microsoft").join("WindowsApps");
        fs::create_dir_all(&apps).unwrap();
        fs::write(apps.join(python_name()), "").unwrap();
        let mut env = env_in(dir.path());
        env.path_var = path_of(&[&apps]);
        let detection = detect_python(&env);
        assert_eq!(detection.path, None);
        assert!(detection.store_alias_ignored);
        let (resolved, issues) = resolve(&Settings::default(), &env);
        assert_eq!(resolved.python_path, None);
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].field, "python_path");
        assert_eq!(issues[0].kind, ISSUE_STORE_ALIAS);
        // Un vrai Python plus loin dans le PATH prime et supprime l'avertissement.
        let real = dir.path().join("real");
        touch(&real.join(python_name()));
        env.path_var = path_of(&[&apps, &real]);
        let (resolved, issues) = resolve(&Settings::default(), &env);
        assert_eq!(
            resolved.python_path,
            Some(real.join(python_name()).display().to_string())
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn python_embarque_exige_le_script_embarque() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let bundled = dir.path().join("app").join(BUNDLED_CONVERTER_DIR);
        let python = bundled.join("python").join("python.exe");
        assert_eq!(detect_bundled_python(&env), None);
        // Interpréteur seul : ignoré (il ne serait associé à aucun script embarqué).
        touch(&python);
        assert_eq!(detect_bundled_python(&env), None);
        touch(&bundled.join("telmi2flam.py"));
        assert_eq!(detect_bundled_python(&env), Some(python.clone()));
        // Fichier vide : pas un interpréteur.
        fs::write(&python, "").unwrap();
        assert_eq!(detect_bundled_python(&env), None);
        let mut env = env;
        env.exe_dir = None;
        assert_eq!(detect_bundled_python(&env), None);
    }

    #[test]
    fn ffmpeg_embarque_a_cote_de_l_executable() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = env_in(dir.path());
        assert_eq!(ffmpeg_path(&env), None);
        let ffmpeg = dir
            .path()
            .join("app")
            .join(BUNDLED_FFMPEG_DIR)
            .join(FFMPEG_EXE);
        touch(&ffmpeg);
        assert_eq!(ffmpeg_path(&env), Some(ffmpeg.clone()));
        // Fichier vide : pas un exécutable.
        fs::write(&ffmpeg, "").unwrap();
        assert_eq!(ffmpeg_path(&env), None);
        // Jamais le PATH, même avec un ffmpeg dedans.
        let in_path = dir.path().join("bin");
        touch(&in_path.join(FFMPEG_EXE));
        env.path_var = path_of(&[&in_path]);
        assert_eq!(ffmpeg_path(&env), None);
        env.exe_dir = None;
        assert_eq!(ffmpeg_path(&env), None);
    }

    #[test]
    fn ffmpeg_de_developpement_dans_build_d_un_ancetre() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = env_in(dir.path());
        let dev = dir
            .path()
            .join("build")
            .join(BUNDLED_FFMPEG_DIR)
            .join(FFMPEG_EXE);
        touch(&dev);
        assert_eq!(ffmpeg_path(&env), None);
        env.search_repo_build = true;
        assert_eq!(ffmpeg_path(&env), Some(dev));
        let beside = dir
            .path()
            .join("app")
            .join(BUNDLED_FFMPEG_DIR)
            .join(FFMPEG_EXE);
        touch(&beside);
        assert_eq!(ffmpeg_path(&env), Some(beside));
    }

    #[test]
    fn python_embarque_prioritaire_sans_avertissement() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = env_in(dir.path());
        // Seul un alias du Store dans le PATH, et un Python configuré valide.
        let apps = dir.path().join("Microsoft").join("WindowsApps");
        fs::create_dir_all(&apps).unwrap();
        fs::write(apps.join(python_name()), "").unwrap();
        env.path_var = path_of(&[&apps]);
        let configured = dir.path().join("autre").join("python.exe");
        touch(&configured);
        let settings = Settings {
            python_path: Some(configured.display().to_string()),
            ..Settings::default()
        };
        let (resolved, _) = resolve(&settings, &env);
        assert_eq!(resolved.python_path, Some(configured.display().to_string()));

        let bundled = dir.path().join("app").join(BUNDLED_CONVERTER_DIR);
        touch(&bundled.join("python").join("python.exe"));
        touch(&bundled.join("telmi2flam.py"));
        let expected = Some(
            bundled
                .join("python")
                .join("python.exe")
                .display()
                .to_string(),
        );
        let (resolved, issues) = resolve(&settings, &env);
        assert_eq!(resolved.python_path, expected);
        assert!(issues.is_empty());
        // Sans Python configuré : pas d'avertissement « alias du Store » non plus.
        let (resolved, issues) = resolve(&Settings::default(), &env);
        assert_eq!(resolved.python_path, expected);
        assert!(issues.is_empty());
    }

    #[test]
    fn resolution_prefere_la_configuration_puis_la_detection() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let detected = dir.path().join("app").join(PLAYER_EXE);
        touch(&detected);

        let (auto, issues) = resolve(&Settings::default(), &env);
        assert_eq!(auto.player_path, Some(detected.display().to_string()));
        assert_eq!(auto.python_path, None);
        assert_eq!(
            auto.cache_dir,
            dir.path().join("cache-defaut").display().to_string()
        );
        assert!(issues.is_empty());

        let mine = dir.path().join("mon/lecteur.exe");
        touch(&mine);
        let (manual, issues) = resolve(
            &Settings {
                player_path: Some(mine.display().to_string()),
                cache_dir: Some(dir.path().join("cache").display().to_string()),
                ..Settings::default()
            },
            &env,
        );
        assert_eq!(manual.player_path, Some(mine.display().to_string()));
        assert_eq!(
            manual.cache_dir,
            dir.path().join("cache").display().to_string()
        );
        assert!(issues.is_empty());
    }

    #[test]
    fn valeurs_configurees_invalides_ou_disparues_retombent_sur_la_detection() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = env_in(dir.path());
        let detected = dir.path().join("app").join(PLAYER_EXE);
        touch(&detected);
        let py_dir = dir.path().join("py");
        touch(&py_dir.join(python_name()));
        env.path_var = path_of(&[&py_dir]);
        let gone = dir.path().join("disparu").join("lecteur.exe");
        let settings = Settings {
            player_path: Some(gone.display().to_string()),
            python_path: Some("\\\\serveur\\partage\\python.exe".into()),
            cache_dir: Some("relatif".into()),
            ..Settings::default()
        };
        let (resolved, issues) = resolve(&settings, &env);
        assert_eq!(resolved.player_path, Some(detected.display().to_string()));
        assert_eq!(
            resolved.python_path,
            Some(py_dir.join(python_name()).display().to_string())
        );
        assert_eq!(
            resolved.cache_dir,
            dir.path().join("cache-defaut").display().to_string()
        );
        let summary: Vec<_> = issues
            .iter()
            .map(|i| (i.field.as_str(), i.kind.as_str()))
            .collect();
        assert_eq!(
            summary,
            [
                ("player_path", ISSUE_NOT_FOUND),
                ("python_path", ISSUE_INVALID_PATH),
                ("cache_dir", ISSUE_INVALID_PATH)
            ]
        );
    }

    #[test]
    fn view_signale_un_player_configure_devenu_invalide() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let player = dir.path().join("lecteur.exe");
        touch(&player);
        let store = SettingsStore::new(dir.path());
        store
            .update(Settings {
                player_path: Some(player.display().to_string()),
                ..Settings::default()
            })
            .unwrap();
        assert!(store.view(&env).unwrap().issues.is_empty());
        fs::remove_file(&player).unwrap();
        let view = store.view(&env).unwrap();
        assert_eq!(view.resolved.player_path, None);
        assert_eq!(view.issues.len(), 1);
        assert_eq!(view.issues[0].kind, ISSUE_NOT_FOUND);
        // La valeur enregistrée est conservée telle quelle.
        assert_eq!(
            view.settings.player_path,
            Some(player.display().to_string())
        );
    }

    #[test]
    fn cache_dir_ne_cree_pas_le_dossier() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let store = SettingsStore::new(dir.path());
        let cache = store.cache_dir(&env).unwrap();
        assert_eq!(cache, dir.path().join("cache-defaut"));
        assert!(!cache.exists());
        assert_eq!(store.resolved_cache_dir(&env).unwrap(), cache);
        assert!(cache.is_dir());
    }

    #[test]
    fn dossier_de_cache_resolu_est_cree() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let store = SettingsStore::new(dir.path());
        let cache = store.resolved_cache_dir(&env).unwrap();
        assert_eq!(cache, dir.path().join("cache-defaut"));
        assert!(cache.is_dir());
    }
}
