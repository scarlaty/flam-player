//! Paramètres de l'application : `settings.json`, valeurs par défaut, validation et
//! détection automatique de `flam-player.exe` et de Python.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::sources::{validate_catalog_url, TELMI_STORE_URL};
use crate::storage::{self, ConfigState, MigrationStep};
use crate::{AppError, AppResult};

/// Version courante du schéma de `settings.json`.
pub const SCHEMA_VERSION: u32 = 1;
const FILE_NAME: &str = "settings.json";
const PLAYER_EXE: &str = "flam-player.exe";

/// Paramètres enregistrés (miroir de `Settings` dans `src/api/settings.ts`).
///
/// Un chemin à `None` signifie « détection automatique » (voir `resolve`).
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            player_path: None,
            python_path: None,
            cache_dir: None,
            default_catalog_url: TELMI_STORE_URL.into(),
        }
    }
}

/// Valeurs effectives après détection automatique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedSettings {
    /// `None` si le player n'a été ni configuré ni trouvé.
    pub player_path: Option<String>,
    /// `None` si Python n'a été ni configuré ni trouvé dans le PATH.
    pub python_path: Option<String>,
    pub cache_dir: String,
}

/// Paramètres enregistrés et valeurs effectives (réponse de `settings_get` / `settings_set`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsView {
    pub settings: Settings,
    pub resolved: ResolvedSettings,
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
        }
    }
}

/// Migrations de schéma de `settings.json` (aucune pour l'instant : v0 -> v1 ajoute
/// seulement `schema_version`).
fn migrate(_from: u32, _object: &mut serde_json::Map<String, serde_json::Value>) {}
const MIGRATE: MigrationStep = migrate;

fn is_nonempty_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file() && meta.len() > 0)
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

/// Cherche Python dans le PATH (sans exécuter de script). Les fichiers vides (alias du
/// Microsoft Store) sont ignorés.
pub fn detect_python(env: &Environment) -> Option<PathBuf> {
    let names: &[&str] = if cfg!(windows) {
        &["python.exe"]
    } else {
        &["python3", "python"]
    };
    let path_var = env.path_var.as_ref()?;
    std::env::split_paths(path_var)
        .flat_map(|dir| names.iter().map(move |name| dir.join(name)))
        .find(|candidate| is_nonempty_file(candidate))
}

/// Calcule les valeurs effectives.
pub fn resolve(settings: &Settings, env: &Environment) -> ResolvedSettings {
    let as_string = |path: PathBuf| path.to_string_lossy().into_owned();
    ResolvedSettings {
        player_path: settings
            .player_path
            .clone()
            .or_else(|| detect_player(env).map(as_string)),
        python_path: settings
            .python_path
            .clone()
            .or_else(|| detect_python(env).map(as_string)),
        cache_dir: settings
            .cache_dir
            .clone()
            .unwrap_or_else(|| as_string(env.default_cache_dir.clone())),
    }
}

fn clean(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn validate_file(label: &str, raw: &str) -> AppResult<()> {
    let path = Path::new(raw);
    if !path.is_absolute() || !path.is_file() {
        return Err(AppError::Validation(format!(
            "{label} : « {raw} » n'est pas un fichier existant (chemin absolu attendu)"
        )));
    }
    Ok(())
}

/// Normalise (espaces, valeurs vides -> automatique) et valide des paramètres.
pub fn validate(settings: Settings) -> AppResult<Settings> {
    let player_path = clean(settings.player_path);
    let python_path = clean(settings.python_path);
    let cache_dir = clean(settings.cache_dir);
    if let Some(path) = &player_path {
        validate_file("chemin du player", path)?;
    }
    if let Some(path) = &python_path {
        validate_file("chemin de Python", path)?;
    }
    if let Some(dir) = &cache_dir {
        let path = Path::new(dir);
        if !path.is_absolute() || path.is_file() {
            return Err(AppError::Validation(format!(
                "dossier de cache : « {dir} » doit être un chemin de dossier absolu"
            )));
        }
    }
    Ok(Settings {
        schema_version: SCHEMA_VERSION,
        player_path,
        python_path,
        cache_dir,
        default_catalog_url: validate_catalog_url(&settings.default_catalog_url)?,
    })
}

/// Accès à `settings.json` dans un dossier de configuration (injecté pour les tests).
pub struct SettingsStore {
    path: PathBuf,
}

impl SettingsStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join(FILE_NAME),
        }
    }

    /// Charge les paramètres. Absent ou corrompu : valeurs par défaut, écrites sur disque.
    pub fn load(&self) -> AppResult<Settings> {
        match storage::load_versioned::<Settings>(&self.path, SCHEMA_VERSION, MIGRATE)? {
            Some(settings) => Ok(settings),
            None => {
                let settings = Settings::default();
                storage::write_json(&self.path, &settings)?;
                Ok(settings)
            }
        }
    }

    /// Valide puis enregistre de nouveaux paramètres ; renvoie les paramètres normalisés.
    pub fn update(&self, settings: Settings) -> AppResult<Settings> {
        let settings = validate(settings)?;
        storage::write_json(&self.path, &settings)?;
        Ok(settings)
    }
}

fn view(settings: Settings) -> SettingsView {
    let resolved = resolve(&settings, &Environment::current());
    SettingsView { settings, resolved }
}

/// Renvoie les paramètres enregistrés et les valeurs effectives.
#[tauri::command]
pub async fn settings_get(state: State<'_, ConfigState>) -> AppResult<SettingsView> {
    let _guard = state.guard();
    SettingsStore::new(&state.dir).load().map(view)
}

/// Valide et enregistre les paramètres.
#[tauri::command]
pub async fn settings_set(
    state: State<'_, ConfigState>,
    settings: Settings,
) -> AppResult<SettingsView> {
    let _guard = state.guard();
    SettingsStore::new(&state.dir).update(settings).map(view)
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
        }
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, "contenu").unwrap();
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
    fn fichier_corrompu_sauvegarde_puis_defaut() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), "\u{0}\u{0} corrompu").unwrap();
        let settings = SettingsStore::new(dir.path()).load().unwrap();
        assert_eq!(settings, Settings::default());
        let baks = fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".bak")
            })
            .count();
        assert_eq!(baks, 1);
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
    fn version_future_refusee() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("settings.json"), r#"{"schema_version":42}"#).unwrap();
        let err = SettingsStore::new(dir.path()).load().unwrap_err();
        assert_eq!(err.kind(), "config_version");
    }

    #[test]
    fn mise_a_jour_valide_est_persistee_et_normalisee() {
        let dir = tempfile::tempdir().unwrap();
        let player = dir.path().join("p/flam-player.exe");
        touch(&player);
        let store = SettingsStore::new(dir.path());
        let saved = store
            .update(Settings {
                player_path: Some(format!("  {} ", player.display())),
                python_path: Some("   ".into()),
                cache_dir: Some(dir.path().join("cache").display().to_string()),
                default_catalog_url: " https://exemple.org/c.json ".into(),
                schema_version: 0,
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
        assert!(!dir.path().join("settings.json").exists());
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
        env.exe_dir = Some(dir.path().join("apps/flam-launcher/src-tauri/target/debug"));
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

    #[test]
    fn detection_de_python_dans_le_path() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("vide");
        let good = dir.path().join("bon");
        fs::create_dir_all(&empty).unwrap();
        touch(&good.join(python_name()));
        let mut env = env_in(dir.path());
        env.path_var = Some(std::env::join_paths([&empty, &good]).unwrap());
        assert_eq!(detect_python(&env), Some(good.join(python_name())));
        env.path_var = Some(std::env::join_paths([&empty]).unwrap());
        assert_eq!(detect_python(&env), None);
        env.path_var = None;
        assert_eq!(detect_python(&env), None);
    }

    #[test]
    fn un_alias_vide_n_est_pas_python() {
        let dir = tempfile::tempdir().unwrap();
        let alias = dir.path().join("alias");
        fs::create_dir_all(&alias).unwrap();
        fs::write(alias.join(python_name()), "").unwrap();
        let mut env = env_in(dir.path());
        env.path_var = Some(std::env::join_paths([&alias]).unwrap());
        assert_eq!(detect_python(&env), None);
    }

    #[test]
    fn resolution_prefere_la_configuration_puis_la_detection() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let detected = dir.path().join("app").join(PLAYER_EXE);
        touch(&detected);

        let auto = resolve(&Settings::default(), &env);
        assert_eq!(auto.player_path, Some(detected.display().to_string()));
        assert_eq!(auto.python_path, None);
        assert_eq!(
            auto.cache_dir,
            dir.path().join("cache-defaut").display().to_string()
        );

        let manual = resolve(
            &Settings {
                player_path: Some("C:\\x\\p.exe".into()),
                cache_dir: Some("D:\\cache".into()),
                ..Settings::default()
            },
            &env,
        );
        assert_eq!(manual.player_path.as_deref(), Some("C:\\x\\p.exe"));
        assert_eq!(manual.cache_dir, "D:\\cache");
    }
}
