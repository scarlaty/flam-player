//! Paramètres de l'application : `settings.json`, valeurs par défaut, validation et
//! détection automatique de `flam-player.exe`.
//!
//! L'exécutable configuré (`player_path`) sera lancé par l'application :
//! il doit être un fichier `.exe` local (ni UNC, ni lecteur réseau). `resolved.*` n'est
//! **pas** une garantie : le fichier peut disparaître après le calcul ; l'appelant (lancement
//! du player, #16) doit revérifier juste avant d'exécuter.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::device::carrier::CarrierConsent;
use crate::sources::{validate_catalog_url, TELMI_STORE_URL};
use crate::storage::{self, ConfigState, MigrationStep, Recovery};
use crate::{AppError, AppResult};

/// Version courante du schéma de `settings.json`.
pub const SCHEMA_VERSION: u32 = 1;
pub(crate) const FILE_NAME: &str = "settings.json";
const PLAYER_EXE: &str = "flam-player.exe";
/// Valeur de retour de `GetDriveTypeW` pour un lecteur réseau (`DRIVE_REMOTE`).
#[cfg(windows)]
const DRIVE_REMOTE: u32 = 4;

/// `kind` d'un [`SettingsIssue`] : chemin refusé par la validation (extension, UNC, réseau…).
pub const ISSUE_INVALID_PATH: &str = "invalid_path";
/// `kind` d'un [`SettingsIssue`] : fichier configuré disparu.
pub const ISSUE_NOT_FOUND: &str = "not_found";

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

/// Remplissage des vignettes de bibliothèque (104 × 136, portrait) par le convertisseur
/// (#279) : option `--thumb-fit` du convertisseur.
///
/// Ensemble fermé : l'argument transmis au convertisseur ne vient jamais d'une chaîne libre
/// ([`ThumbFit::as_arg`]). Une valeur inconnue (fichier édité à la main ou écrit par une autre
/// version) retombe sur `Flou` au lieu de faire rejeter tout le fichier.
///
/// Pendant de `telmi2flam::image::ThumbFit`, gardé ici parce que la crate n'a pas de serde et
/// que ce type porte le repli `serde(other)`. Les noms viennent de la crate ([`ThumbFit::as_arg`]
/// passe par `From`) ; le test `memes_modes_que_la_crate_telmi2flam` garde les variantes alignées.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThumbFit {
    /// Image agrandie et rognée au centre pour couvrir la vignette.
    Rogner,
    /// Image entière centrée, bandes transparentes.
    Bandes,
    /// Image entière centrée, fond = la même image agrandie et floutée (doit rester la dernière
    /// variante pour `serde(other)`).
    #[default]
    #[serde(other)]
    Flou,
}

impl From<ThumbFit> for telmi2flam::image::ThumbFit {
    fn from(fit: ThumbFit) -> Self {
        match fit {
            ThumbFit::Flou => Self::Flou,
            ThumbFit::Rogner => Self::Rogner,
            ThumbFit::Bandes => Self::Bandes,
        }
    }
}

impl ThumbFit {
    /// Valeur de l'option `--thumb-fit` du convertisseur (nom défini par la crate, commun aux
    /// deux moteurs).
    pub fn as_arg(self) -> &'static str {
        telmi2flam::image::ThumbFit::from(self).as_str()
    }
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
    /// Consentement à l'avertissement du mode carrier (Flam 1.x, #234), avec sa date. Écrit
    /// seulement par `device_carrier_accept` : la valeur reçue par `settings_set` est ignorée
    /// (l'enregistrée est conservée). Absent des anciens fichiers : `None`.
    pub carrier_consent: Option<CarrierConsent>,
    /// Remplissage des vignettes des histoires converties (#279). Absent des anciens fichiers :
    /// `Flou` (`#[serde(default)]`, sans migration de schéma).
    pub thumb_fit: ThumbFit,
    /// Voix des titres lus des packs (#462, `VoiceInformation.Id`), mémorisée par poste ;
    /// `None` : voix française par défaut. Absent des anciens fichiers : `None`. Une valeur
    /// invalide (mauvais type, trop longue, caractères de contrôle) est **ignorée** à la lecture
    /// comme à l'enregistrement (`None`, avertissement sans la valeur) : elle ne fait jamais
    /// échouer `load` ni `settings_set`.
    #[serde(deserialize_with = "lenient_voice")]
    pub tts_voice: Option<String>,
    /// Retirer l'histoire de « Mes histoires » après son envoi réussi sur la Flam (#609).
    /// Désactivé par défaut. Absent des anciens fichiers : `false`, sans migration de schéma (la
    /// bêta.4 ignore ce champ à la lecture et l'efface en réécrivant le fichier : retour à `false`).
    /// Une valeur qui n'est pas un booléen est lue `false` (jamais d'échec de `load`).
    #[serde(deserialize_with = "lenient_flag")]
    pub remove_after_send: bool,
}

/// Lecture tolérante d'un réglage booléen : seul `true` l'active.
fn lenient_flag<'de, D: serde::Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    Ok(matches!(
        serde_json::Value::deserialize(d)?,
        serde_json::Value::Bool(true)
    ))
}

/// Lecture tolérante de `tts_voice` : toute valeur JSON est acceptée, seule une chaîne valide
/// est gardée.
fn lenient_voice<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    Ok(match raw {
        serde_json::Value::Null => None,
        serde_json::Value::String(s) => lenient_voice_id(Some(s)),
        _ => {
            tracing::warn!("réglage tts_voice ignoré : type inattendu");
            None
        }
    })
}

/// [`clean_voice_id`] sans erreur : un identifiant invalide devient `None` (avertissement
/// journalisé sans la valeur).
pub fn lenient_voice_id(value: Option<String>) -> Option<String> {
    clean_voice_id(value).unwrap_or_else(|_| {
        tracing::warn!("réglage tts_voice ignoré : identifiant de voix invalide");
        None
    })
}

/// Longueur maximale d'un identifiant de voix (`tts_voice`, `titles_voice` d'un pack).
pub const MAX_VOICE_ID_CHARS: usize = 512;

/// Identifiant de voix normalisé : espaces retirés, vide -> `None` ; trop long ou avec des
/// caractères de contrôle : refusé.
pub fn clean_voice_id(value: Option<String>) -> AppResult<Option<String>> {
    let value = clean(value);
    if let Some(id) = &value {
        if id.chars().count() > MAX_VOICE_ID_CHARS || id.chars().any(char::is_control) {
            return Err(AppError::Validation(
                "identifiant de voix invalide (trop long ou caractères de contrôle)".into(),
            ));
        }
    }
    Ok(value)
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
            cache_dir: None,
            default_catalog_url: TELMI_STORE_URL.into(),
            theme: Theme::default(),
            max_downloads: DEFAULT_MAX_DOWNLOADS,
            carrier_consent: None,
            thumb_fit: ThumbFit::default(),
            tts_voice: None,
            remove_after_send: false,
        }
    }
}

/// Valeurs effectives après détection automatique.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedSettings {
    /// `None` si le player n'a été ni configuré (valide) ni trouvé.
    pub player_path: Option<String>,
    pub cache_dir: String,
}

/// Problème détecté en relisant les paramètres (miroir de `SettingsIssue` côté TypeScript).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SettingsIssue {
    /// Champ concerné : `player_path` ou `cache_dir`.
    pub field: String,
    /// `invalid_path` ou `not_found`.
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

/// Refuse un **nouveau** dossier de cache qui contient déjà, sous un nom de
/// `cache::APP_CACHE_SUBDIRS`, un élément qui n'a pas le marqueur de l'application (#570) : le
/// vidage l'ignorerait, mais l'application y écrirait au milieu de données qui ne sont pas à elle.
///
/// Exception : un ancien cache du launcher (`library/` avec l'index lisible d'une version sans
/// marqueur) est accepté ; `library/` est marqué, puis `podcasts/`, `podcast-work/`, `play/`,
/// `flam-transfer/` et `thumbs/` sur preuve par leur contenu (`cache::adopt_legacy_cache`),
/// **avant** l'enregistrement des réglages. Sans bibliothèque reconnue, un `flam-transfer/` non
/// vide dont le contenu est prouvé, dans un dossier qui porte une autre trace de l'application
/// (un autre sous-dossier de la liste déjà marqué), n'est pas un homonyme : il est marqué
/// (`cache::late_adoptable`, `cache::adopt_late_subdirs`) seulement si le dossier est accepté.
/// Une copie des histoires de la Flam rangée sous ce nom dans un dossier sans autre trace reste
/// refusée. Les autres homonymes (dont `archives/`, `catalogs/`, `compat-local/`,
/// `official/`, jamais marqués ici) ne sont ni marqués ni bloquants (jamais comptés ni vidés). Le
/// marquage est idempotent : si l'enregistrement échoue ensuite, les marqueurs posés restent, et
/// un nouvel essai réussit.
fn check_cache_dir_free(raw: &str) -> AppResult<()> {
    let root = Path::new(raw);
    // `flam-transfer` prouvé, dans un dossier qui porte une autre trace de l'application : pas un
    // homonyme. Marqué seulement si le dossier est accepté (aucun marqueur dans un dossier refusé).
    let late = crate::cache::late_adoptable(root);
    let foreign: Vec<&str> = crate::cache::foreign_entries(root)
        .into_iter()
        .filter(|name| !late.contains(name))
        .collect();
    if foreign.is_empty() {
        crate::cache::adopt_late_subdirs(root, &late);
        return Ok(());
    }
    if crate::cache::adopt_legacy_cache(root)? {
        return Ok(());
    }
    let names = foreign
        .iter()
        .map(|name| format!("« {name} »"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(AppError::CacheDirInUse(format!(
        "dossier de cache : « {raw} » contient déjà {names} sans le marqueur de Flam Launcher. \
         Choisissez un dossier vide ou réservé au launcher."
    )))
}

/// Compare deux chemins composant par composant, sans tenir compte de la casse ni des `/` ou
/// `\` finaux.
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
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

/// Dossier de ffmpeg embarqué, à côté de l'exécutable (ressource Tauri `ffmpeg/` produite par
/// `scripts/build-ffmpeg.ps1`, #209) : `ffmpeg.exe`, `ffprobe.exe` (statiques), les licences et `BUILD.txt`.
pub const BUNDLED_FFMPEG_DIR: &str = "ffmpeg";

/// Nom de l'exécutable ffmpeg.
pub const FFMPEG_EXE: &str = if cfg!(windows) {
    "ffmpeg.exe"
} else {
    "ffmpeg"
};

/// Cherche le ffmpeg LGPL embarqué : `<exe>/ffmpeg/ffmpeg.exe` sinon (développement)
/// `build/ffmpeg/ffmpeg.exe` d'un dossier ancêtre. Aucun ffmpeg du `PATH` n'est jamais retenu
/// (build GPL possible) et aucun processus n'est lancé. Le convertisseur la reçoit par
/// `FLAM_FFMPEG` (rééchantillonnage audio, #46).
pub fn ffmpeg_path(env: &Environment) -> Option<PathBuf> {
    let exe_dir = env.exe_dir.as_deref()?;
    let beside = exe_dir.join(BUNDLED_FFMPEG_DIR).join(FFMPEG_EXE);
    if is_nonempty_file(&beside) {
        return Some(beside);
    }
    if env.search_repo_build {
        // `build-ffmpeg.ps1` écrit dans `apps/flam-launcher/build/ffmpeg` ; `build/ffmpeg` à la
        // racine d'un ancêtre est aussi accepté.
        return exe_dir.ancestors().find_map(|dir| {
            [
                dir.join("apps").join("flam-launcher").join("build"),
                dir.join("build"),
            ]
            .into_iter()
            .map(|base| base.join(BUNDLED_FFMPEG_DIR).join(FFMPEG_EXE))
            .find(|candidate| is_nonempty_file(candidate))
        });
    }
    None
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
    let configured_cache = settings.cache_dir.as_deref().and_then(|raw| {
        match check_cache_dir(raw, &env.protected_dirs) {
            Ok(()) => Some(raw.to_string()),
            Err(err) => {
                issue("cache_dir", err);
                None
            }
        }
    });

    let resolved = ResolvedSettings {
        player_path: configured_player.or_else(|| detect_player(env).map(as_string)),
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
    let cache_dir = clean(settings.cache_dir);
    if let Some(path) = &player_path {
        check_exe("chemin du player", path).map_err(to_validation)?;
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
        cache_dir,
        default_catalog_url: validate_catalog_url(&settings.default_catalog_url)?,
        theme: settings.theme,
        max_downloads: settings.max_downloads,
        carrier_consent: settings.carrier_consent,
        thumb_fit: settings.thumb_fit,
        tts_voice: lenient_voice_id(settings.tts_voice),
        remove_after_send: settings.remove_after_send,
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

    /// Lecture **pure** (#578), pour un lecteur qui ne tient pas le verrou `ConfigState`
    /// (`install::settings_resolver`) : rien n'est écrit ni mis de côté. Absent ou corrompu :
    /// valeurs par défaut en mémoire (la quarantaine et la restauration restent à
    /// `settings_get` / `settings_set`, sous le verrou). Version future : erreur, comme [`Self::load`].
    pub fn peek(&self) -> AppResult<Settings> {
        Ok(
            storage::peek_versioned(&self.dir.join(FILE_NAME), SCHEMA_VERSION, MIGRATE)?
                .unwrap_or_default(),
        )
    }

    /// Comme [`Self::peek`] (rien n'est écrit ni mis de côté), mais un fichier présent et
    /// illisible (corrompu) donne `None` au lieu des valeurs par défaut ; absent : valeurs par
    /// défaut. Pour un lecteur qui ne doit rien décider sur des réglages inconnus
    /// (`cache::catch_up`, #591). Version future : erreur.
    pub(crate) fn peek_readable(&self) -> AppResult<Option<Settings>> {
        let path = self.dir.join(FILE_NAME);
        match storage::peek_versioned(&path, SCHEMA_VERSION, MIGRATE)? {
            Some(settings) => Ok(Some(settings)),
            None if std::fs::symlink_metadata(&path).is_ok() => Ok(None),
            None => Ok(Some(Settings::default())),
        }
    }

    /// Remplace **l'objet complet** après validation (pas de fusion : un champ à `None` repasse
    /// en automatique). Le fichier existant est d'abord relu par le même chemin que `load` :
    /// version future -> refus sans le toucher ; fichier corrompu -> sauvegardé en `*.bak`.
    /// Renvoie les paramètres normalisés.
    ///
    /// Le consentement du mode carrier n'est pas modifiable ici : la valeur enregistrée est
    /// conservée quelle que soit celle reçue (voir [`Self::record_carrier_consent`]).
    pub fn update(&self, settings: Settings) -> AppResult<Settings> {
        self.update_with(settings, &|path, settings| {
            storage::write_json(path, settings)
        })
    }

    /// Comme [`Self::update`], avec l'écriture de `settings.json` injectable (tests de panne).
    fn update_with(
        &self,
        settings: Settings,
        write: &dyn Fn(&Path, &Settings) -> AppResult<()>,
    ) -> AppResult<Settings> {
        let stored = self.load()?;
        let mut settings = validate(settings)?;
        let mut new_witness = None;
        if let Some(dir) = &settings.cache_dir {
            let unchanged = stored
                .cache_dir
                .as_deref()
                .is_some_and(|old| same_path(Path::new(old), Path::new(dir)));
            if !unchanged {
                check_cache_dir_free(dir)?;
                // Migration des marqueurs encore en attente (#591) : elle ne vaut que pour le
                // dossier configuré au moment de la mise à jour. Le nouveau dossier, contrôlé
                // ci-dessus, ne doit jamais être migré à l'aveugle : témoin écrit avant le réglage
                // (arrêt entre les deux : l'ancien dossier reste non marqué, fuite sans perte).
                let witness = self.dir.join(crate::cache::MIGRATION_WITNESS);
                if std::fs::symlink_metadata(&witness).is_err() {
                    std::fs::create_dir_all(&self.dir)?;
                    std::fs::write(&witness, b"")?;
                    new_witness = Some(witness);
                }
            }
        }
        settings.carrier_consent = stored.carrier_consent;
        if let Err(err) = write(&self.dir.join(FILE_NAME), &settings) {
            // Réglage inchangé : la migration de l'ancien dossier reste à faire.
            if let Some(witness) = new_witness {
                let _ = std::fs::remove_file(witness);
            }
            return Err(err);
        }
        Ok(settings)
    }

    /// Enregistre (ou retire, `None`) le consentement du mode carrier, sans toucher au reste.
    pub fn record_carrier_consent(&self, consent: Option<CarrierConsent>) -> AppResult<()> {
        let mut settings = self.load()?;
        settings.carrier_consent = consent;
        storage::write_json(&self.dir.join(FILE_NAME), &settings)
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

    /// Dossier de cache effectif, créé s'il n'existe pas, avec les sous-dossiers de
    /// `cache::APP_CACHE_SUBDIRS` absents (créés marqués, #570) : tout module qui passe par ici
    /// écrit ensuite dans un sous-dossier que le vidage reconnaît comme le sien.
    pub fn resolved_cache_dir(&self, env: &Environment) -> AppResult<PathBuf> {
        let dir = self.cache_dir(env)?;
        std::fs::create_dir_all(&dir)?;
        crate::cache::ensure_subdirs(&dir);
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

    /// #578 : lecture pure de `settings_resolver` : fichier corrompu laissé tel quel (ni `.bak`,
    /// ni restauration mémorisée), absent non créé, valeurs par défaut en mémoire ; version
    /// future refusée comme par `load`. La quarantaine reste à `load`, sous le verrou.
    #[test]
    fn peek_ne_modifie_rien() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let path = dir.path().join("settings.json");

        assert_eq!(store.peek().unwrap(), Settings::default());
        assert!(!path.exists(), "fichier absent non créé");

        fs::write(&path, "{ corrompu").unwrap();
        assert_eq!(store.peek().unwrap(), Settings::default());
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ corrompu");
        assert_eq!(baks(dir.path()), 0);
        assert!(!pending_recovery_exists(dir.path()));

        fs::write(&path, r#"{"schema_version":1,"max_downloads":3}"#).unwrap();
        assert_eq!(store.peek().unwrap().max_downloads, 3);

        fs::write(&path, r#"{"schema_version":42}"#).unwrap();
        assert_eq!(store.peek().unwrap_err().kind(), "config_version");
        assert_eq!(baks(dir.path()), 0);
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
    fn memes_modes_que_la_crate_telmi2flam() {
        use telmi2flam::image::ThumbFit as Crate;
        for fit in [ThumbFit::Flou, ThumbFit::Rogner, ThumbFit::Bandes] {
            let theirs = Crate::from_name(fit.as_arg()).expect("mode inconnu de la crate");
            assert_eq!(theirs.as_str(), fit.as_arg());
        }
        assert_eq!(Crate::default().as_str(), ThumbFit::default().as_arg());
        // Et dans l'autre sens : les noms que la crate accepte sont tous connus ici.
        for name in ["flou", "rogner", "bandes"] {
            assert_eq!(Crate::from_name(name).unwrap().as_str(), name);
        }
    }

    /// Le réglage caché `converter_engine` (#136) n'existe plus (#138) : un ancien
    /// `settings.json` qui le porte (`python`, `rust`, ou une valeur quelconque) reste lisible,
    /// le champ est ignoré (Rust est le seul convertisseur) et disparaît à la prochaine écriture.
    #[test]
    fn ancien_reglage_converter_engine_ignore() {
        for value in [
            r#""python""#,
            r#""rust""#,
            r#""go""#,
            "1",
            "null",
            r#"{"engine":"python"}"#,
        ] {
            let s: Settings = serde_json::from_str(&format!(
                r#"{{"schema_version":1,"converter_engine":{value},"max_downloads":3}}"#
            ))
            .unwrap_or_else(|e| panic!("{value} : {e}"));
            assert_eq!(s.max_downloads, 3, "{value}");
        }
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("settings.json"),
            r#"{"schema_version":1,"converter_engine":"python","max_downloads":3}"#,
        )
        .unwrap();
        let store = SettingsStore::new(dir.path());
        assert_eq!(store.load().unwrap().max_downloads, 3);
        store.update(store.load().unwrap()).unwrap();
        let written = fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(!written.contains("converter_engine"), "{written}");
        assert_eq!(store.load().unwrap().max_downloads, 3);
    }

    /// Le réglage `python_path` n'existe plus (#139) : un `settings.json` écrit par une version
    /// qui l'avait (valeur quelconque, même invalide) reste lisible, les autres réglages sont
    /// conservés, le champ est ignoré et disparaît à la prochaine écriture.
    #[test]
    fn ancien_reglage_python_path_ignore() {
        let dir = tempfile::tempdir().unwrap();
        for value in [
            r#""C:\\Python312\\python.exe""#,
            r#""\\\\serveur\\partage\\python.exe""#,
            r#""python""#,
            "null",
            "42",
        ] {
            fs::write(
                dir.path().join("settings.json"),
                format!(
                    r#"{{"schema_version":1,"python_path":{value},"max_downloads":3,"theme":"dark"}}"#
                ),
            )
            .unwrap();
            let store = SettingsStore::new(dir.path());
            let loaded = store.load().unwrap_or_else(|e| panic!("{value} : {e}"));
            assert_eq!(loaded.max_downloads, 3, "{value}");
            assert_eq!(loaded.theme, Theme::Dark, "{value}");
            store.update(loaded).unwrap();
            let written = fs::read_to_string(dir.path().join("settings.json")).unwrap();
            assert!(!written.contains("python_path"), "{value} : {written}");
            assert_eq!(store.load().unwrap().max_downloads, 3, "{value}");
        }
    }

    #[test]
    fn remplissage_des_vignettes_ferme_et_par_defaut() {
        // absent des anciens fichiers : flou
        let old: Settings = serde_json::from_str(r#"{"schema_version":1}"#).unwrap();
        assert_eq!(old.thumb_fit, ThumbFit::Flou);
        assert_eq!(Settings::default().thumb_fit.as_arg(), "flou");
        for (text, fit, arg) in [
            ("flou", ThumbFit::Flou, "flou"),
            ("rogner", ThumbFit::Rogner, "rogner"),
            ("bandes", ThumbFit::Bandes, "bandes"),
        ] {
            let s: Settings =
                serde_json::from_str(&format!(r#"{{"thumb_fit":"{text}"}}"#)).unwrap();
            assert_eq!(s.thumb_fit, fit);
            assert_eq!(fit.as_arg(), arg);
            assert_eq!(
                serde_json::to_value(fit).unwrap(),
                serde_json::Value::String(text.into())
            );
        }
        // valeur inconnue (ou tentative d'injection) : retombe sur flou, jamais transmise telle quelle
        for bad in ["etirer", "flou --plain", "../x", ""] {
            let s: Settings =
                serde_json::from_value(serde_json::json!({ "thumb_fit": bad })).unwrap();
            assert_eq!(s.thumb_fit, ThumbFit::Flou, "{bad:?}");
            assert_eq!(s.thumb_fit.as_arg(), "flou");
        }
        // la valeur choisie survit à la validation
        let saved = validate_with(
            Settings {
                thumb_fit: ThumbFit::Bandes,
                ..Settings::default()
            },
            &[],
        )
        .unwrap();
        assert_eq!(saved.thumb_fit, ThumbFit::Bandes);
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
                cache_dir: Some(dir.path().join("cache").display().to_string()),
                default_catalog_url: " https://exemple.org/c.json ".into(),
                schema_version: 0,
                theme: Theme::Dark,
                max_downloads: 3,
                carrier_consent: None,
                thumb_fit: ThumbFit::Rogner,
                tts_voice: Some("  fr-hortense ".into()),
                remove_after_send: true,
            })
            .unwrap();
        assert_eq!(saved.schema_version, SCHEMA_VERSION);
        assert_eq!(saved.player_path, Some(player.display().to_string()));
        assert_eq!(saved.thumb_fit, ThumbFit::Rogner);
        assert_eq!(saved.tts_voice.as_deref(), Some("fr-hortense"));
        assert!(saved.remove_after_send);
        assert_eq!(store.load().unwrap(), saved);
    }

    #[test]
    fn un_nouveau_cache_avec_un_homonyme_sans_marqueur_est_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let media = dir.path().join("media");
        touch(&media.join("podcasts/emission.mp3"));
        let with_cache = |path: &Path| Settings {
            cache_dir: Some(path.display().to_string()),
            ..Settings::default()
        };
        let before = store.load().unwrap();

        let err = store.update(with_cache(&media)).unwrap_err();
        assert_eq!(err.kind(), "cache_dir_in_use");
        assert!(err.to_string().contains("« podcasts »"), "{err}");
        assert_eq!(store.load().unwrap(), before, "rien d'enregistré");
        assert!(media.join("podcasts/emission.mp3").is_file());
        assert!(!media.join("library").exists(), "aucun dossier créé");

        // `podcasts/` personnel seul : toujours refusé, même avec un `library/` vide à côté.
        std::fs::create_dir_all(media.join("library")).unwrap();
        assert_eq!(
            store.update(with_cache(&media)).unwrap_err().kind(),
            "cache_dir_in_use"
        );
        assert!(!crate::cache::is_owned(&media.join("library")));

        // Un dossier dont les sous-dossiers sont ceux de l'application est accepté…
        let ours = dir.path().join("ours");
        crate::cache::ensure_subdirs(&ours);
        store.update(with_cache(&ours)).unwrap();
        // … et le réenregistrer reste possible même si un homonyme y est apparu depuis.
        std::fs::remove_dir_all(ours.join("thumbs")).unwrap();
        touch(&ours.join("thumbs/perso.png"));
        store.update(with_cache(&ours)).unwrap();
    }

    /// Un ancien cache dont `library.json` n'est pas reconnu (pas d'adoption) mais qui porte une
    /// trace de l'application (un autre sous-dossier marqué) et dont `flam-transfer` a la forme
    /// écrite par l'application est accepté, `flam-transfer` marqué. Sans trace, un vide, un
    /// contenu personnel, ou un autre homonyme refusé : refus, aucun marqueur laissé. Une copie
    /// des histoires de la Flam rangée sous ce nom dans un dossier sans autre trace est refusée.
    #[test]
    fn un_flam_transfer_prouve_est_marque_seulement_dans_un_cache_de_l_application() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let with_cache = |path: &Path| Settings {
            cache_dir: Some(path.display().to_string()),
            ..Settings::default()
        };
        let story = "0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e";
        let device_copy = |root: &Path| {
            for file in ["key", "info", "main.lsf", "img/a.png", "sounds/a.mp3"] {
                touch(&root.join(format!("flam-transfer/str/{story}/{file}")));
            }
        };

        // Trace d'usage : accepté et marqué.
        let old = dir.path().join("ancien");
        crate::cache::ensure_subdir(&old, "archives").unwrap();
        device_copy(&old);
        store.update(with_cache(&old)).unwrap();
        assert!(crate::cache::is_owned(&old.join("flam-transfer")));

        // Copie façon appareil sans autre trace : refusée, rien de marqué ni supprimé.
        let backup = dir.path().join("sauvegarde");
        device_copy(&backup);
        let err = store.update(with_cache(&backup)).unwrap_err();
        assert_eq!(err.kind(), "cache_dir_in_use");
        assert!(err.to_string().contains("« flam-transfer »"), "{err}");
        assert!(!crate::cache::is_owned(&backup.join("flam-transfer")));
        assert!(backup
            .join(format!("flam-transfer/str/{story}/key"))
            .is_file());

        // Même copie avec un `podcasts/` personnel à côté : refusée, aucun marqueur laissé.
        let mixed = dir.path().join("mixte");
        device_copy(&mixed);
        touch(&mixed.join("podcasts/emission.mp3"));
        assert_eq!(
            store.update(with_cache(&mixed)).unwrap_err().kind(),
            "cache_dir_in_use"
        );
        assert!(!crate::cache::is_owned(&mixed.join("flam-transfer")));
        assert!(!mixed.join("podcasts").join(crate::cache::MARKER).exists());

        // Avec une trace mais un autre homonyme personnel : refus, `flam-transfer` non marqué
        // (le marquage attend l'acceptation du dossier).
        let traced = dir.path().join("trace-et-homonyme");
        crate::cache::ensure_subdir(&traced, "archives").unwrap();
        device_copy(&traced);
        touch(&traced.join("podcasts/emission.mp3"));
        assert_eq!(
            store.update(with_cache(&traced)).unwrap_err().kind(),
            "cache_dir_in_use"
        );
        assert!(!crate::cache::is_owned(&traced.join("flam-transfer")));

        // Vide, fichiers de l'explorateur seuls ou contenu personnel : refusés comme tout homonyme.
        for (name, file) in [
            ("perso", "flam-transfer/notes.txt"),
            ("vide", "flam-transfer/desktop.ini"),
        ] {
            let other = dir.path().join(name);
            crate::cache::ensure_subdir(&other, "archives").unwrap();
            touch(&other.join(file));
            let err = store.update(with_cache(&other)).unwrap_err();
            assert_eq!(err.kind(), "cache_dir_in_use", "{name}");
            assert!(
                !crate::cache::is_owned(&other.join("flam-transfer")),
                "{name}"
            );
        }
    }

    #[test]
    fn un_ancien_cache_du_launcher_est_accepte_puis_marque() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let old = dir.path().join("ancien-cache");
        touch(&old.join("library/abc-1.plain.pk"));
        fs::write(
            old.join("library/library.json"),
            r#"{"schema_version":1,"stories":[]}"#,
        )
        .unwrap();
        touch(&old.join("podcasts/src/feed.xml"));
        touch(&old.join("thumbs/0000000000000001.png"));
        // `play/` personnel : refusé par la preuve d'usage.
        touch(&old.join("play/film.mkv"));
        // `archives/` : aucune preuve par le contenu, jamais marqué à l'adoption.
        touch(&old.join("archives/perso.zip"));

        let saved = store
            .update(Settings {
                cache_dir: Some(old.display().to_string()),
                ..Settings::default()
            })
            .unwrap();
        assert_eq!(saved.cache_dir, Some(old.display().to_string()));
        // Sous-dossiers marqués sur preuve par le contenu (#591).
        assert!(crate::cache::is_owned(&old.join("library")));
        assert!(crate::cache::is_owned(&old.join("podcasts")));
        assert!(crate::cache::is_owned(&old.join("thumbs")));
        assert!(!crate::cache::is_owned(&old.join("play")));
        assert!(!crate::cache::is_owned(&old.join("archives")));
        assert!(old.join("podcasts/src/feed.xml").is_file());
        assert!(old.join("play/film.mkv").is_file());
        assert!(
            old.join("library/abc-1.plain.pk").is_file(),
            "rien supprimé"
        );
    }

    /// Migration encore en attente (#591) : elle ne vaut que pour le dossier configuré au moment
    /// de la mise à jour, jamais pour un dossier choisi ensuite.
    #[test]
    fn un_cache_choisi_apres_la_mise_a_jour_n_est_jamais_migre() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let env = env_in(dir.path());
        let witness = dir.path().join(crate::cache::MIGRATION_WITNESS);
        // Ancien cache choisi alors que la migration est reportée (témoin absent) ; son
        // `archives/` n'est pas prouvé « à l'application » par son contenu.
        let old = dir.path().join("ancien-cache");
        touch(&old.join("library/abc-1.plain.pk"));
        fs::write(
            old.join("library/library.json"),
            r#"{"schema_version":1,"stories":[]}"#,
        )
        .unwrap();
        touch(&old.join("archives/perso.zip"));
        assert!(!witness.exists());
        store
            .update(Settings {
                cache_dir: Some(old.display().to_string()),
                ..Settings::default()
            })
            .unwrap();
        assert!(witness.is_file(), "témoin écrit avec le nouveau réglage");
        assert_eq!(
            crate::cache::migrate_configured(dir.path(), &env).unwrap(),
            crate::cache::Migration::AlreadyDone
        );
        assert!(!crate::cache::is_owned(&old.join("archives")));
        assert!(old.join("archives/perso.zip").is_file());
    }

    #[test]
    fn echec_d_ecriture_des_reglages_retire_le_temoin_cree_par_l_appel() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let witness = dir.path().join(crate::cache::MIGRATION_WITNESS);
        let cache = dir.path().join("nouveau");
        let with_cache = Settings {
            cache_dir: Some(cache.display().to_string()),
            ..Settings::default()
        };
        let before = store.load().unwrap();
        let fail = |_: &Path, _: &Settings| -> AppResult<()> {
            Err(AppError::Io(std::io::Error::other(
                "panne simulée : settings.json",
            )))
        };
        assert!(store.update_with(with_cache.clone(), &fail).is_err());
        assert!(
            !witness.exists(),
            "témoin retiré : migration toujours à faire"
        );
        assert_eq!(store.load().unwrap(), before);
        // Témoin déjà présent avant l'appel : jamais retiré.
        fs::write(&witness, b"").unwrap();
        assert!(store.update_with(with_cache.clone(), &fail).is_err());
        assert!(witness.is_file());
        // Reprise : enregistré.
        store.update(with_cache).unwrap();
    }

    #[test]
    fn reenregistrer_le_meme_cache_ne_clot_pas_la_migration() {
        let dir = tempfile::tempdir().unwrap();
        let store = SettingsStore::new(dir.path());
        let cache = dir.path().join("cache");
        crate::cache::ensure_subdirs(&cache);
        let with_cache = Settings {
            cache_dir: Some(cache.display().to_string()),
            ..Settings::default()
        };
        store.update(with_cache.clone()).unwrap();
        fs::remove_file(dir.path().join(crate::cache::MIGRATION_WITNESS)).unwrap();
        // Même dossier : rien de neuf, la migration de ce dossier reste à faire.
        store.update(with_cache).unwrap();
        assert!(!dir.path().join(crate::cache::MIGRATION_WITNESS).exists());
    }

    #[test]
    fn resolved_cache_dir_cree_les_sous_dossiers_marques() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let cache = SettingsStore::new(dir.path())
            .resolved_cache_dir(&env)
            .unwrap();
        for name in crate::cache::APP_CACHE_SUBDIRS {
            assert!(crate::cache::is_owned(&cache.join(name)), "{name}");
        }
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
                player_path: Some(not_exe.display().to_string()),
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
        let ok = dir.path().join("Flam-Player.ExE");
        let ko = dir.path().join("flam-player.cmd");
        touch(&ok);
        touch(&ko);
        assert!(check_exe("x", &ok.display().to_string()).is_ok());
        let (kind, _) = check_exe("x", &ko.display().to_string()).unwrap_err();
        assert_eq!(kind, ISSUE_INVALID_PATH);
    }

    #[test]
    fn chemins_unc_et_reseau_refuses() {
        for bad in [
            "\\\\serveur\\partage\\flam-player.exe",
            "\\\\?\\UNC\\serveur\\partage\\flam-player.exe",
            "//serveur/partage/flam-player.exe",
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
    fn ffmpeg_de_developpement_disposition_du_depot() {
        let dir = tempfile::tempdir().unwrap();
        let mut env = env_in(dir.path());
        env.exe_dir = Some(dir.path().join("target").join("debug"));
        env.search_repo_build = true;
        let dev = dir
            .path()
            .join("apps")
            .join("flam-launcher")
            .join("build")
            .join(BUNDLED_FFMPEG_DIR)
            .join(FFMPEG_EXE);
        touch(&dev);
        assert_eq!(ffmpeg_path(&env), Some(dev));
    }

    #[test]
    fn resolution_prefere_la_configuration_puis_la_detection() {
        let dir = tempfile::tempdir().unwrap();
        let env = env_in(dir.path());
        let detected = dir.path().join("app").join(PLAYER_EXE);
        touch(&detected);

        let (auto, issues) = resolve(&Settings::default(), &env);
        assert_eq!(auto.player_path, Some(detected.display().to_string()));
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
        let env = env_in(dir.path());
        let detected = dir.path().join("app").join(PLAYER_EXE);
        touch(&detected);
        let gone = dir.path().join("disparu").join("lecteur.exe");
        let settings = Settings {
            player_path: Some(gone.display().to_string()),
            cache_dir: Some("relatif".into()),
            ..Settings::default()
        };
        let (resolved, issues) = resolve(&settings, &env);
        assert_eq!(resolved.player_path, Some(detected.display().to_string()));
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

    /// Un `tts_voice` corrompu sur le disque n'empêche ni la lecture ni l'enregistrement : il
    /// est ignoré, les autres réglages sont gardés et le fichier est réécrit propre.
    #[test]
    fn voix_des_titres_corrompue_sur_disque_ignoree() {
        let long = format!("\"{}\"", "v".repeat(MAX_VOICE_ID_CHARS + 1));
        for bad in [
            "\"a\\u0007b\"",
            long.as_str(),
            "42",
            "[\"x\"]",
            "{\"id\": 1}",
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join(FILE_NAME);
            std::fs::write(
                &path,
                format!(
                    "{{\"schema_version\": {SCHEMA_VERSION}, \"max_downloads\": 3, \
                     \"tts_voice\": {bad}}}"
                ),
            )
            .unwrap();
            let store = SettingsStore::new(dir.path());
            let loaded = store.load().unwrap();
            assert_eq!(loaded.tts_voice, None, "{bad}");
            assert_eq!(loaded.max_downloads, 3, "{bad} : autres réglages gardés");
            let saved = store
                .update(Settings {
                    theme: Theme::Dark,
                    ..loaded
                })
                .unwrap();
            assert_eq!(saved.tts_voice, None);
            let written = std::fs::read_to_string(&path).unwrap();
            let json: serde_json::Value = serde_json::from_str(&written).unwrap();
            assert!(json["tts_voice"].is_null(), "{bad} : {written}");
            assert_eq!(json["max_downloads"], 3);
        }
        // `settings_set` avec une voix invalide : ignorée, jamais une erreur.
        let dir = tempfile::tempdir().unwrap();
        let saved = SettingsStore::new(dir.path())
            .update(Settings {
                tts_voice: Some("a\u{0}b".into()),
                ..Settings::default()
            })
            .unwrap();
        assert_eq!(saved.tts_voice, None);
    }

    #[test]
    fn voix_des_titres_absente_des_anciens_fichiers_et_bornee() {
        // Fichier écrit avant #462 : pas de `tts_voice`.
        let old: Settings =
            serde_json::from_str(r#"{"schema_version": 1, "max_downloads": 2}"#).unwrap();
        assert_eq!(old.tts_voice, None);
        assert_eq!(lenient_voice_id(Some("a\u{1}b".into())), None);
        assert_eq!(clean_voice_id(Some("   ".into())).unwrap(), None);
        for bad in ["a\u{0}b".to_string(), "v".repeat(MAX_VOICE_ID_CHARS + 1)] {
            assert_eq!(clean_voice_id(Some(bad)).unwrap_err().kind(), "validation");
        }
    }

    /// #609 : « Retirer de Mes histoires après l'envoi » est désactivé par défaut, absent des
    /// anciens fichiers (bêta.4 comprise) sans migration de schéma, et une valeur qui n'est pas
    /// un booléen ne l'active jamais ni ne fait échouer la lecture.
    #[test]
    fn retrait_apres_envoi_desactive_par_defaut_et_tolerant() {
        assert!(!Settings::default().remove_after_send);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let store = SettingsStore::new(dir.path());
        // Fichier écrit par la bêta.4 : pas de champ.
        fs::write(
            &path,
            r#"{"schema_version":1,"max_downloads":3,"theme":"dark","thumb_fit":"rogner"}"#,
        )
        .unwrap();
        let old = store.load().unwrap();
        assert!(!old.remove_after_send);
        assert_eq!(old.max_downloads, 3);
        for bad in [r#""true""#, "1", "null", "{}"] {
            fs::write(
                &path,
                format!(r#"{{"schema_version":1,"max_downloads":3,"remove_after_send":{bad}}}"#),
            )
            .unwrap();
            let loaded = store.load().unwrap_or_else(|e| panic!("{bad} : {e}"));
            assert!(!loaded.remove_after_send, "{bad}");
            assert_eq!(loaded.max_downloads, 3, "{bad} : fichier non restauré");
        }
        // Activé puis relu ; le schéma ne change pas.
        let saved = store
            .update(Settings {
                remove_after_send: true,
                ..store.load().unwrap()
            })
            .unwrap();
        assert!(saved.remove_after_send);
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(json["remove_after_send"], true);
        assert_eq!(json["schema_version"], 1);
        assert!(store.load().unwrap().remove_after_send);
    }
}
