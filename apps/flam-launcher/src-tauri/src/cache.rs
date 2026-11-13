//! Dossier de cache : sous-dossiers de l'application, taille occupée et vidage.
//!
//! Le dossier de cache peut être choisi par l'utilisateur et contenir autre chose que les
//! données de l'application. Le vidage ne supprime donc **jamais** le contenu du dossier
//! configuré : seuls les sous-dossiers listés dans [`APP_CACHE_SUBDIRS`] **et** portant le
//! marqueur [`MARKER`] (preuve que l'application les a créés, #570) sont comptés et vidés. Les
//! liens symboliques et jonctions ne sont jamais suivis. Tout nouveau sous-dossier créé par un
//! module de l'application doit être ajouté à cette liste ; il est créé, marqué, par
//! [`ensure_subdirs`] (appelée par `SettingsStore::resolved_cache_dir`).

use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use serde::Serialize;
use tauri::State;

use crate::catalog::{blocking, CatalogService};
use crate::player::PlayerService;
use crate::settings::{resolve, Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::AppResult;

/// Sous-dossiers du cache créés par l'application (et eux seuls) : archives téléchargées, catalogues mis en cache,
/// histoires converties, catalogue officiel en ligne (`official`), copies de travail des
/// histoires lancées (`play`), flux podcast mis en cache (`podcasts`, #211), vignettes.
pub const APP_CACHE_SUBDIRS: &[&str] = &[
    "archives",
    "catalogs",
    "compat-local",
    "library",
    "official",
    "play",
    "podcast-work",
    "podcasts",
    "thumbs",
];

/// Réponse de `cache_usage` (miroir de `CacheUsage` dans `src/api/cache.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheUsage {
    /// Dossier de cache effectif.
    pub path: String,
    /// Octets occupés par les sous-dossiers de l'application.
    pub bytes: u64,
    /// Sous-dossiers que l'application compte et vide : ceux de `APP_CACHE_SUBDIRS` présents et
    /// marqués (un homonyme non marqué, épargné par le vidage, n'est pas annoncé).
    pub subdirs: Vec<String>,
}

/// Réponse de `cache_clear` (miroir de `CacheCleared` dans `src/api/cache.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CacheCleared {
    /// Octets libérés.
    pub freed_bytes: u64,
}

/// Marqueur d'appartenance : fichier vide déposé à la racine de chaque sous-dossier du cache que
/// l'application crée. Un homonyme sans marqueur n'est ni compté, ni vidé, ni balayé.
pub const MARKER: &str = ".flam-launcher";

/// Préfixe des dossiers temporaires de [`ensure_subdir`] (marqueur écrit avant le renommage).
const TMP_PREFIX: &str = ".flam-launcher-new-";

/// Témoin de la migration des marqueurs, dans le dossier de configuration (pas dans le cache).
pub const MIGRATION_WITNESS: &str = "cache-markers.done";

/// Point d'injection de panne : appelé avant chaque étape qui écrit ou supprime (tests).
type Fault<'a> = &'a dyn Fn(&'static str) -> io::Result<()>;

fn no_fault(_: &'static str) -> io::Result<()> {
    Ok(())
}

/// Dossier réel (ni lien, ni jonction, ni fichier).
fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir() && !meta.is_symlink())
}

/// `dir` est un dossier réel qui porte le marqueur de l'application.
pub fn is_owned(dir: &Path) -> bool {
    is_real_dir(dir) && fs::symlink_metadata(dir.join(MARKER)).is_ok_and(|meta| meta.is_file())
}

/// Crée `root/name` avec son marqueur s'il n'existe pas, et renvoie son chemin. Un élément déjà
/// présent sous ce nom (dossier marqué ou non, fichier, lien) est laissé tel quel : seul un
/// dossier que l'application vient de créer reçoit le marqueur. La création est atomique : le
/// marqueur est écrit dans un dossier temporaire renommé ensuite vers `name`.
pub fn ensure_subdir(root: &Path, name: &str) -> io::Result<PathBuf> {
    ensure_subdir_with(root, name, &no_fault)
}

fn ensure_subdir_with(root: &Path, name: &str, fault: Fault) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = root.join(name);
    match fs::symlink_metadata(&path) {
        Ok(_) => return Ok(path),
        Err(err) if err.kind() == ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    fs::create_dir_all(root)?;
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let tmp = root.join(format!("{TMP_PREFIX}{name}-{}-{n}", std::process::id()));
    fault("create_tmp")?;
    fs::create_dir(&tmp)?;
    let done = fault("marker")
        .and_then(|()| fs::write(tmp.join(MARKER), b""))
        .and_then(|()| fault("rename"))
        .and_then(|()| fs::rename(&tmp, &path));
    match done {
        Ok(()) => Ok(path),
        Err(err) => {
            let _ = fs::remove_file(tmp.join(MARKER));
            let _ = fs::remove_dir(&tmp);
            // Un autre traitement a créé le nom entre-temps : il est gardé tel quel.
            if fs::symlink_metadata(&path).is_ok() {
                Ok(path)
            } else {
                Err(err)
            }
        }
    }
}

/// Crée, marqués, les sous-dossiers de [`APP_CACHE_SUBDIRS`] absents de `root`. Un échec n'est
/// que journalisé : le module concerné échouera lui-même s'il ne peut pas écrire.
pub fn ensure_subdirs(root: &Path) {
    for name in APP_CACHE_SUBDIRS {
        if let Err(err) = ensure_subdir(root, name) {
            tracing::warn!("cache : sous-dossier « {name} » non créé : {err}");
        }
    }
}

/// Sous-dossiers de l'application existant réellement **et** marqués (nom et chemin, dans
/// l'ordre de [`APP_CACHE_SUBDIRS`]). Un homonyme sans marqueur est ignoré (et journalisé, par
/// son nom seulement).
fn app_dirs(root: &Path) -> Vec<(&'static str, PathBuf)> {
    APP_CACHE_SUBDIRS
        .iter()
        .filter_map(|name| {
            let path = root.join(name);
            if is_owned(&path) {
                Some((*name, path))
            } else {
                if fs::symlink_metadata(&path).is_ok() {
                    tracing::warn!(
                        "cache : « {name} » n'a pas le marqueur de l'application, ignoré"
                    );
                }
                None
            }
        })
        .collect()
}

/// Noms de [`APP_CACHE_SUBDIRS`] présents dans `root` sans être des sous-dossiers marqués
/// (dossier sans marqueur, fichier, lien) : `root` ne peut pas devenir le dossier de cache.
pub fn foreign_entries(root: &Path) -> Vec<&'static str> {
    APP_CACHE_SUBDIRS
        .iter()
        .copied()
        .filter(|name| {
            let path = root.join(name);
            fs::symlink_metadata(&path).is_ok() && !is_owned(&path)
        })
        .collect()
}

/// Résultat de [`migrate_markers`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Migration {
    /// Témoin déjà présent : rien n'a été fait.
    AlreadyDone,
    /// Marqueurs posés (`marked` nouveaux) puis témoin écrit.
    Done { marked: usize },
    /// Un dossier de cache est absent : rejouée au prochain démarrage.
    Deferred,
}

/// Migration unique (#570) : les sous-dossiers de [`APP_CACHE_SUBDIRS`] déjà présents dans les
/// dossiers de cache `roots` (celui déjà configuré, et le dossier par défaut) servaient déjà à
/// l'application ; ils reçoivent le marqueur, avec une preuve d'usage pour ceux dont la forme du
/// contenu est connue (voir [`proven_by_content`]). Le témoin `witness` est écrit en dernier :
/// interrompue, la migration est rejouée (elle est idempotente). Une racine est un dossier choisi
/// par l'utilisateur : elle peut être une jonction ou un lien (#591), seul le premier niveau doit
/// être un dossier réel. Limite : un homonyme de l'utilisateur sans preuve d'usage exigée
/// (`archives`, `catalogs`…) déjà présent dans le dossier configuré est marqué lui aussi.
pub fn migrate_markers(witness: &Path, roots: &[PathBuf]) -> io::Result<Migration> {
    migrate_markers_with(witness, roots, &no_fault)
}

fn migrate_markers_with(witness: &Path, roots: &[PathBuf], fault: Fault) -> io::Result<Migration> {
    if fs::symlink_metadata(witness).is_ok() {
        return Ok(Migration::AlreadyDone);
    }
    if roots.iter().any(|root| !root.is_dir()) {
        return Ok(Migration::Deferred);
    }
    let mut marked = 0;
    for root in roots {
        marked += mark_existing(root, fault)?;
    }
    fault("witness")?;
    if let Some(parent) = witness.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(witness, b"")?;
    Ok(Migration::Done { marked })
}

/// Marque les sous-dossiers de [`APP_CACHE_SUBDIRS`] présents dans `root` (dossiers réels sans
/// marqueur, dont le contenu prouve l'usage par l'application) et renvoie leur nombre.
/// Idempotente. Un marqueur anormal (lien, dossier) est laissé tel quel, sans marquer ce
/// sous-dossier.
fn mark_existing(root: &Path, fault: Fault) -> io::Result<usize> {
    let mut marked = 0;
    for name in APP_CACHE_SUBDIRS {
        let dir = root.join(name);
        if !is_real_dir(&dir) || is_owned(&dir) {
            continue;
        }
        if fs::symlink_metadata(dir.join(MARKER)).is_ok() {
            tracing::warn!("cache : marqueur de « {name} » inattendu, non migré");
            continue;
        }
        if !proven_by_content(name, &dir) {
            tracing::warn!(
                "cache : contenu de « {name} » non reconnu, non migré (ni compté ni vidé)"
            );
            continue;
        }
        fault("marker")?;
        // `create_new` : n'ouvre jamais un élément existant (ni lien, ni fichier).
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(MARKER))?;
        marked += 1;
    }
    Ok(marked)
}

/// Preuve d'usage exigée par la migration (#591) pour les sous-dossiers dont l'application
/// connaît la forme du contenu ; en cas de doute, rien n'est marqué (fuite, jamais de perte) :
/// - `library` : index reconnu ([`crate::library::is_legacy_library`]) ou dossier vide ;
/// - `podcasts` : chaque enfant est un dossier réel nommé comme un identifiant de source
///   (règle de `podcast::source::source_dir`) ;
/// - `podcast-work` : chaque enfant est un dossier réel nommé par un identifiant de tâche
///   (`install::is_task_id`) ;
/// - `play` : chaque enfant est un dossier réel nommé par l'empreinte de `player::Workspace`
///   (16 chiffres hexadécimaux minuscules).
///
/// Les autres sous-dossiers n'exigent pas de preuve.
fn proven_by_content(name: &str, dir: &Path) -> bool {
    let child_shape: fn(&str) -> bool = match name {
        crate::library::LIBRARY_DIR => {
            return crate::library::is_legacy_library(dir) || library_children_known(dir);
        }
        crate::podcast::source::CACHE_SUBDIR => |child| {
            !child.is_empty()
                && child.len() <= 64
                && child
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        },
        crate::install::podcast::WORK_DIR => crate::install::is_task_id,
        crate::player::PLAY_DIR => |child| {
            child.len() == 16
                && child
                    .chars()
                    .all(|c| c.is_ascii_digit() || matches!(c, 'a'..='f'))
        },
        _ => return true,
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        entry.is_ok_and(|entry| {
            entry
                .file_type()
                .is_ok_and(|kind| kind.is_dir() && !kind.is_symlink())
                && entry.file_name().to_str().is_some_and(child_shape)
        })
    })
}

/// Tous les éléments de `library/` ont une forme que l'application écrit à sa racine (vide
/// compris) : verrou `.lock` (fichier), `.staging` et `thumbs` (dossiers), index `library.json`,
/// index mis de côté `library.json.<horodatage>[-n].bak`, temporaire d'écriture atomique de
/// l'index `.library.json.<pid>-<n>.tmp`, histoires `*.plain.pk` et `*.lunii.pk` (fichiers). Un
/// seul autre nom, un lien ou un type inattendu : refus. Une bibliothèque ouverte sans rien
/// installer (`.lock` seul) est ainsi reconnue (#591).
fn library_children_known(dir: &Path) -> bool {
    let index = crate::library::INDEX_FILE;
    let tmp_prefix = format!(".{index}.");
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        let Ok(entry) = entry else {
            return false;
        };
        let (Ok(kind), Some(name)) = (
            entry.file_type(),
            entry.file_name().to_str().map(str::to_owned),
        ) else {
            return false;
        };
        if kind.is_symlink() {
            return false;
        }
        if kind.is_dir() {
            return name == crate::library::STAGING_DIR || name == crate::library_thumb::THUMBS_DIR;
        }
        kind.is_file()
            && (name == ".lock"
                || name == index
                || name.ends_with(crate::library::FLAM_SUFFIX)
                || name.ends_with(".lunii.pk")
                || name
                    .strip_prefix(index)
                    .is_some_and(|rest| rest.starts_with('.') && rest.ends_with(".bak"))
                || name
                    .strip_prefix(tmp_prefix.as_str())
                    .is_some_and(|rest| rest.ends_with(".tmp")))
    })
}

/// Migration au démarrage pour le dossier de configuration `config_dir` : dossier de cache
/// effectif, plus le dossier par défaut s'il existe. Témoin présent : rien n'est lu (ni réglages
/// ni cache), le démarrage n'attend aucun accès au cache (#591). Quand le dossier résolu n'est pas
/// celui des réglages (valeur configurée refusée, repli sur le défaut), la migration reste
/// reportée (`Deferred`, sans témoin) pour le dossier configuré, mais le dossier par défaut, alors
/// en usage, est migré quand même (#591).
pub fn migrate_configured(config_dir: &Path, env: &Environment) -> AppResult<Migration> {
    migrate_configured_with(config_dir, env, &no_fault)
}

fn migrate_configured_with(
    config_dir: &Path,
    env: &Environment,
    fault: Fault,
) -> AppResult<Migration> {
    let witness = config_dir.join(MIGRATION_WITNESS);
    if fs::symlink_metadata(&witness).is_ok() {
        return Ok(Migration::AlreadyDone);
    }
    let settings = SettingsStore::new(config_dir).load()?;
    let (resolved, _) = resolve(&settings, env);
    let effective = PathBuf::from(resolved.cache_dir);
    if let Some(configured) = &settings.cache_dir {
        if !crate::settings::same_path(Path::new(configured), &effective) {
            if env.default_cache_dir.is_dir() {
                mark_existing(&env.default_cache_dir, fault)?;
            }
            return Ok(Migration::Deferred);
        }
    }
    let mut roots = vec![effective.clone()];
    // Le dossier par défaut (données de l'application) sert aussi s'il est repris plus tard.
    if env.default_cache_dir != effective && env.default_cache_dir.is_dir() {
        roots.push(env.default_cache_dir.clone());
    }
    Ok(migrate_markers_with(&witness, &roots, fault)?)
}

/// Nettoyage des dossiers temporaires d'une création interrompue dans le dossier de cache
/// effectif et dans le dossier par défaut, à chaque démarrage, **après** l'ouverture de la porte
/// de démarrage (il ne dépend pas du marqueur ; les temporaires récents sont gardés). Les réglages
/// sont lus sous le verrou de configuration ; le nettoyage se fait hors verrou.
pub fn remove_stale_tmp_configured(state: &ConfigState, env: &Environment) {
    let effective = {
        let _guard = state.guard();
        SettingsStore::new(&state.dir)
            .load()
            .map(|settings| PathBuf::from(resolve(&settings, env).0.cache_dir))
    };
    if let Ok(effective) = &effective {
        remove_stale_tmp(effective);
    }
    if !effective.is_ok_and(|dir| crate::settings::same_path(&dir, &env.default_cache_dir)) {
        remove_stale_tmp(&env.default_cache_dir);
    }
}

/// Ancien cache du launcher (version sans marqueur) : `library/` est un dossier réel qui contient
/// l'index de la bibliothèque, lisible ([`crate::library::is_legacy_library`]). Il est alors
/// adopté : **seul** `library/` reçoit le marqueur (s'il ne l'a pas déjà) et `true` est renvoyé.
/// Les autres homonymes sans marqueur (dossiers, fichiers, liens) ne sont ni marqués ni bloquants :
/// l'application les utilise comme avant, sans jamais les compter ni les vider. Idempotente.
/// Sinon rien n'est écrit, `false`.
pub fn adopt_legacy_cache(root: &Path) -> io::Result<bool> {
    adopt_legacy_cache_with(root, &no_fault)
}

fn adopt_legacy_cache_with(root: &Path, fault: Fault) -> io::Result<bool> {
    let library = root.join(crate::library::LIBRARY_DIR);
    if !is_real_dir(&library) || !crate::library::is_legacy_library(&library) {
        return Ok(false);
    }
    if is_owned(&library) {
        return Ok(true);
    }
    if fs::symlink_metadata(library.join(MARKER)).is_ok() {
        // Lien ou dossier à la place du marqueur : rien n'est prouvé, rien n'est écrit.
        return Ok(false);
    }
    fault("marker")?;
    // `create_new` : n'ouvre jamais un élément existant (ni lien, ni fichier).
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(library.join(MARKER))?;
    Ok(true)
}

/// Âge minimal d'un dossier temporaire de création avant sa suppression : en deçà, une création
/// peut être en cours (autre commande, seconde instance), #591.
const STALE_TMP_AGE: Duration = Duration::from_secs(60);

/// Supprime les dossiers temporaires laissés par une [`ensure_subdir`] interrompue : seulement
/// s'ils sont vides ou ne contiennent que le marqueur (`remove_dir` échoue sinon), et modifiés il
/// y a au moins [`STALE_TMP_AGE`] (date illisible ou dans le futur : gardé).
pub fn remove_stale_tmp(root: &Path) {
    let now = SystemTime::now();
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let path = entry.path();
        if !name.to_string_lossy().starts_with(TMP_PREFIX) || !is_real_dir(&path) {
            continue;
        }
        let old_enough = fs::symlink_metadata(&path)
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| {
                now.duration_since(modified)
                    .is_ok_and(|age| age >= STALE_TMP_AGE)
            });
        if !old_enough {
            continue;
        }
        let only_marker = fs::read_dir(&path).is_ok_and(|inner| {
            inner
                .map(|e| e.map(|e| e.file_name() == MARKER))
                .all(|is_marker| is_marker.unwrap_or(false))
        });
        if !only_marker {
            continue;
        }
        let _ = fs::remove_file(path.join(MARKER));
        if let Err(err) = fs::remove_dir(&path) {
            tracing::warn!("cache : dossier temporaire non supprimé : {err}");
        }
    }
}

/// Profondeur maximale explorée sous un sous-dossier du cache (au-delà, rien n'est compté).
const MAX_DEPTH: usize = 32;

/// Taille cumulée des fichiers sous `dir`, sans suivre les liens symboliques, sur au plus
/// [`MAX_DEPTH`] niveaux. Ce qui disparaît pendant le calcul (`NotFound`) compte pour zéro.
pub(crate) fn dir_size(dir: &Path) -> std::io::Result<u64> {
    dir_size_at(dir, 0)
}

fn dir_size_at(dir: &Path, depth: usize) -> std::io::Result<u64> {
    if depth >= MAX_DEPTH {
        return Ok(0);
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err),
    };
    let mut total = 0;
    for entry in entries {
        let size = entry.and_then(|entry| {
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                Ok(0)
            } else if kind.is_dir() {
                dir_size_at(&entry.path(), depth + 1)
            } else {
                entry.metadata().map(|meta| meta.len())
            }
        });
        match size {
            Ok(size) => total += size,
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    Ok(total)
}

/// Octets occupés par les sous-dossiers de l'application dans `root`.
pub fn usage_of(root: &Path) -> AppResult<u64> {
    Ok(usage_report(root)?.0)
}

/// Octets occupés et noms des sous-dossiers comptés (ceux que le vidage videra : marqués).
fn usage_report(root: &Path) -> AppResult<(u64, Vec<String>)> {
    let mut total = 0;
    let mut names = Vec::new();
    for (name, dir) in app_dirs(root) {
        total += dir_size(&dir)?;
        names.push(name.to_string());
    }
    Ok((total, names))
}

/// Vide les sous-dossiers marqués de l'application dans `root` et renvoie les octets libérés.
/// Chaque sous-dossier et son marqueur restent en place (il reste prouvé « à l'application ») ;
/// tout le reste de `root` est laissé intact. Un élément déjà disparu est ignoré ; un lien ou
/// une jonction rencontré est supprimé en tant que lien, jamais suivi.
pub fn clear_of(root: &Path) -> AppResult<u64> {
    clear_of_with(root, &no_fault)
}

fn clear_of_with(root: &Path, fault: Fault) -> AppResult<u64> {
    let mut freed = 0;
    for (_, dir) in app_dirs(root) {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            Err(err) => return Err(err.into()),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            if entry.file_name() == MARKER {
                continue;
            }
            let path = entry.path();
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(err) if err.kind() == ErrorKind::NotFound => continue,
                Err(err) => return Err(err.into()),
            };
            let size = if kind.is_dir() && !kind.is_symlink() {
                dir_size(&path)?
            } else if kind.is_symlink() {
                0
            } else {
                entry.metadata().map_or(0, |meta| meta.len())
            };
            fault("remove")?;
            match remove_entry(&path, kind) {
                Ok(()) => freed += size,
                Err(err) if err.kind() == ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
        }
    }
    Ok(freed)
}

/// Supprime une entrée sans suivre les liens : dossier réel récursivement, lien (fichier,
/// dossier ou jonction) en tant que lien, fichier simplement.
fn remove_entry(path: &Path, kind: fs::FileType) -> io::Result<()> {
    if kind.is_symlink() {
        // Lien vers un fichier : `remove_file` ; lien vers un dossier ou jonction : `remove_dir`.
        fs::remove_file(path).or_else(|_| fs::remove_dir(path))
    } else if kind.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub(crate) fn effective_dir(state: &ConfigState) -> AppResult<PathBuf> {
    let settings = SettingsStore::new(&state.dir).load()?;
    let (resolved, _) = resolve(&settings, &Environment::current());
    Ok(PathBuf::from(resolved.cache_dir))
}

/// Dossier de cache effectif et espace occupé par les données de l'application.
#[tauri::command]
pub async fn cache_usage(state: State<'_, ConfigState>) -> AppResult<CacheUsage> {
    let root = {
        let _guard = state.guard();
        effective_dir(&state)?
    };
    let scanned = root.clone();
    let (bytes, subdirs) = blocking(move || usage_report(&scanned)).await??;
    Ok(CacheUsage {
        bytes,
        path: root.to_string_lossy().into_owned(),
        subdirs,
    })
}

/// Vide le cache : vide uniquement les sous-dossiers marqués de l'application. Attend
/// les rafraîchissements de catalogue en cours et bloque les nouveaux pendant le vidage.
/// Refusé (`player_busy`) tant qu'un player tourne : sa copie de travail est dans `play/`. Le
/// créneau du player est tenu pendant le vidage, aucun lancement ne peut démarrer entre-temps.
#[tauri::command]
pub async fn cache_clear(
    state: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    player: State<'_, PlayerService>,
) -> AppResult<CacheCleared> {
    let _pause = catalog.pause_refreshes().await;
    let root = {
        let _guard = state.guard();
        effective_dir(&state)?
    };
    let player = player.inner().clone();
    let freed_bytes = blocking(move || player.while_idle(|| clear_of(&root))).await???;
    Ok(CacheCleared { freed_bytes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn write(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![b'x'; bytes]).unwrap();
    }

    /// Crée `root/name` comme le ferait l'application (marqué).
    fn owned(root: &Path, name: &str) -> PathBuf {
        ensure_subdir(root, name).unwrap()
    }

    /// Panne à la `n`-ième étape `step` (comptée à partir de 1).
    fn fail_at(step: &'static str, n: usize) -> impl Fn(&'static str) -> io::Result<()> {
        let seen = Cell::new(0);
        move |s| {
            if s == step {
                seen.set(seen.get() + 1);
                if seen.get() == n {
                    return Err(io::Error::other(format!("panne simulée : {step} #{n}")));
                }
            }
            Ok(())
        }
    }

    fn names_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn un_cache_absent_ou_vide_occupe_zero_octet() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(usage_of(&dir.path().join("absent")).unwrap(), 0);
        assert_eq!(usage_of(dir.path()).unwrap(), 0);
        assert_eq!(clear_of(dir.path()).unwrap(), 0);
    }

    #[test]
    fn la_taille_ne_compte_que_les_sous_dossiers_marques_de_l_application() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["archives", "library", "thumbs", "catalogs"] {
            owned(dir.path(), name);
        }
        write(&dir.path().join("archives/a.zip"), 100);
        write(&dir.path().join("library/histoire/x/y.bin"), 50);
        write(&dir.path().join("thumbs/t.png"), 7);
        write(&dir.path().join("catalogs/src/catalog.json"), 3);
        write(&dir.path().join("documents/important.txt"), 1000);
        write(&dir.path().join("racine.txt"), 500);
        // Homonyme sans marqueur : pas compté.
        write(&dir.path().join("podcasts/emission.mp3"), 4000);
        assert_eq!(usage_of(dir.path()).unwrap(), 160);
    }

    #[test]
    fn le_vidage_ne_vide_que_les_sous_dossiers_marques_et_garde_le_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        owned(dir.path(), "archives");
        owned(dir.path(), "library");
        write(&dir.path().join("archives/a.zip"), 100);
        write(&dir.path().join("library/h/y.bin"), 50);
        write(&dir.path().join("documents/important.txt"), 1000);
        write(&dir.path().join("racine.txt"), 500);
        // Un fichier qui porte le nom d'un sous-dossier n'est pas supprimé.
        write(&dir.path().join("thumbs"), 3);

        assert_eq!(clear_of(dir.path()).unwrap(), 150);

        for name in ["archives", "library"] {
            assert_eq!(names_in(&dir.path().join(name)), [MARKER], "{name} vidé");
            assert!(is_owned(&dir.path().join(name)), "{name} marqué");
        }
        assert!(dir.path().join("thumbs").is_file());
        assert!(dir.path().join("documents/important.txt").is_file());
        assert!(dir.path().join("racine.txt").is_file());
        assert!(dir.path().exists(), "le dossier configuré reste en place");
        assert_eq!(usage_of(dir.path()).unwrap(), 0);
    }

    #[test]
    fn le_vidage_epargne_un_homonyme_sans_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        owned(dir.path(), "archives");
        write(&dir.path().join("archives/a.zip"), 10);
        // Dossiers de l'utilisateur qui portent des noms de l'application.
        write(&dir.path().join("podcasts/mon-emission.mp3"), 300);
        write(&dir.path().join("library/livres/roman.epub"), 200);

        assert_eq!(clear_of(dir.path()).unwrap(), 10);

        assert!(dir.path().join("podcasts/mon-emission.mp3").is_file());
        assert!(dir.path().join("library/livres/roman.epub").is_file());
        assert!(!dir.path().join("podcasts").join(MARKER).exists());
        assert!(!dir.path().join("library").join(MARKER).exists());
    }

    #[test]
    fn un_marqueur_qui_est_un_dossier_ne_prouve_rien() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("podcasts").join(MARKER).join("x"), 1);
        write(&dir.path().join("podcasts/a.mp3"), 5);
        assert!(!is_owned(&dir.path().join("podcasts")));
        assert_eq!(clear_of(dir.path()).unwrap(), 0);
        assert!(dir.path().join("podcasts/a.mp3").is_file());
    }

    #[test]
    fn un_sous_dossier_vide_puis_recree_reprend_son_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        owned(dir.path(), "play");
        write(&dir.path().join("play/copie/x.bin"), 4);
        clear_of(dir.path()).unwrap();
        assert!(is_owned(&dir.path().join("play")));
        // Supprimé hors de l'application, puis recréé par elle : marqué de nouveau.
        fs::remove_dir_all(dir.path().join("play")).unwrap();
        ensure_subdirs(dir.path());
        assert!(is_owned(&dir.path().join("play")));
    }

    #[test]
    fn ensure_subdirs_cree_les_sous_dossiers_marques_sans_toucher_aux_homonymes() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("podcasts/a.mp3"), 5);
        write(&dir.path().join("thumbs"), 1);
        ensure_subdirs(dir.path());
        for name in APP_CACHE_SUBDIRS {
            let path = dir.path().join(name);
            match *name {
                "podcasts" => assert!(!is_owned(&path), "homonyme jamais marqué"),
                "thumbs" => assert!(path.is_file(), "fichier laissé tel quel"),
                _ => assert!(is_owned(&path), "{name} créé marqué"),
            }
        }
        assert_eq!(foreign_entries(dir.path()), ["podcasts", "thumbs"]);
        assert!(names_in(dir.path())
            .iter()
            .all(|n| !n.starts_with(TMP_PREFIX)));
    }

    #[test]
    fn creation_interrompue_a_chaque_etape_puis_reprise() {
        for step in ["create_tmp", "marker", "rename"] {
            let dir = tempfile::tempdir().unwrap();
            let fault = fail_at(step, 1);
            assert!(ensure_subdir_with(dir.path(), "library", &fault).is_err());
            // Invariant : le nom final n'existe jamais sans marqueur.
            assert!(!dir.path().join("library").exists(), "{step}");
            // Temporaire nettoyé sur échec ; reprise : création marquée.
            assert!(names_in(dir.path()).is_empty(), "{step}");
            assert!(is_owned(&ensure_subdir(dir.path(), "library").unwrap()));
        }
    }

    #[test]
    fn creation_concurrente_garde_le_dossier_d_un_tiers_sans_le_marquer() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // Un tiers crée `library/` avec des données entre la vérification et le renommage.
        let race = |step: &'static str| {
            if step == "rename" {
                write(&root.join("library/perso.txt"), 3);
            }
            Ok(())
        };
        assert_eq!(
            ensure_subdir_with(&root, "library", &race).unwrap(),
            root.join("library")
        );
        assert!(root.join("library/perso.txt").is_file());
        assert!(!is_owned(&root.join("library")));
        assert_eq!(names_in(&root), ["library"], "aucun temporaire ne reste");
    }

    #[test]
    fn le_balayage_des_dossiers_de_travail_garde_le_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        ensure_subdirs(dir.path());
        write(&dir.path().join("podcast-work/orpheline/x"), 1);
        // Tous les noms lus sont déclarés orphelins : seul le marqueur doit survivre.
        crate::install::podcast::sweep_workdirs(dir.path(), &|names| names);
        assert!(is_owned(&dir.path().join("podcast-work")));
        assert_eq!(names_in(&dir.path().join("podcast-work")), [MARKER]);
    }

    #[test]
    fn la_migration_saute_un_marqueur_anormal_sans_bloquer() {
        let config = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        // `.flam-launcher` est un dossier dans `archives/` : ni écrit, ni bloquant.
        write(&cache.path().join("archives").join(MARKER).join("x"), 1);
        write(&cache.path().join("catalogs/h"), 1);
        let roots = [cache.path().to_path_buf()];
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::Done { marked: 1 }
        );
        assert!(!is_owned(&cache.path().join("archives")));
        assert!(is_owned(&cache.path().join("catalogs")));
    }

    #[test]
    fn un_temporaire_laisse_par_un_arret_est_nettoye_sans_toucher_au_reste() {
        let dir = tempfile::tempdir().unwrap();
        // Arrêt après le marqueur, avant le renommage.
        write(
            &dir.path()
                .join(format!("{TMP_PREFIX}library-1-0"))
                .join(MARKER),
            0,
        );
        // Arrêt après la création du temporaire.
        fs::create_dir(dir.path().join(format!("{TMP_PREFIX}thumbs-1-1"))).unwrap();
        // Un dossier au même préfixe avec d'autres données n'est pas touché.
        let other = dir.path().join(format!("{TMP_PREFIX}autre"));
        write(&other.join("garder.txt"), 3);
        write(&other.join(MARKER), 0);
        for name in names_in(dir.path()) {
            backdate(&dir.path().join(name));
        }
        remove_stale_tmp(dir.path());
        assert_eq!(names_in(dir.path()), [format!("{TMP_PREFIX}autre")]);
        assert_eq!(names_in(&other), [MARKER, "garder.txt"]);
    }

    /// Recule d'au moins [`STALE_TMP_AGE`] la date de modification de `path` (fichier ou dossier).
    fn backdate(path: &Path) {
        let mut options = fs::OpenOptions::new();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_WRITE_ATTRIBUTES suffit pour changer les dates ; FILE_FLAG_BACKUP_SEMANTICS
            // permet d'ouvrir un dossier.
            options.access_mode(0x100).custom_flags(0x0200_0000);
        }
        #[cfg(not(windows))]
        options.read(true);
        options
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - 2 * STALE_TMP_AGE)
            .unwrap();
    }

    #[test]
    fn un_temporaire_recent_d_une_autre_creation_n_est_pas_supprime() {
        let dir = tempfile::tempdir().unwrap();
        // Création en cours ailleurs (autre commande, seconde instance) : marqueur écrit, pas
        // encore renommé.
        let busy = dir.path().join(format!("{TMP_PREFIX}library-999-0"));
        write(&busy.join(MARKER), 0);
        remove_stale_tmp(dir.path());
        assert!(busy.join(MARKER).is_file(), "temporaire récent gardé");
        // Une fois vieux d'une minute, c'est un orphelin : supprimé.
        backdate(&busy);
        remove_stale_tmp(dir.path());
        assert!(names_in(dir.path()).is_empty());
    }

    #[test]
    fn les_temporaires_du_dossier_par_defaut_sont_nettoyes_meme_si_la_migration_est_reportee() {
        let config = tempfile::tempdir().unwrap();
        let default = config.path().join("cache-defaut");
        let configured = config.path().join("protege");
        let env = Environment {
            exe_dir: None,
            default_cache_dir: default.clone(),
            search_repo_build: false,
            protected_dirs: vec![configured.clone()],
        };
        fs::write(
            config.path().join("settings.json"),
            serde_json::json!({ "schema_version": 1, "cache_dir": configured }).to_string(),
        )
        .unwrap();
        let stale = default.join(format!("{TMP_PREFIX}play-1-0"));
        fs::create_dir_all(&stale).unwrap();
        backdate(&stale);
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Deferred
        );
        remove_stale_tmp_configured(&ConfigState::new(config.path().to_path_buf()), &env);
        assert!(!stale.exists());
    }

    #[test]
    fn le_vidage_interrompu_garde_les_marqueurs_et_reprend() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["archives", "library", "podcasts"] {
            owned(dir.path(), name);
            write(&dir.path().join(name).join("a.bin"), 10);
            write(&dir.path().join(name).join("b/c.bin"), 10);
        }
        write(&dir.path().join("documents/x.txt"), 7);
        for n in 1..=6 {
            let fault = fail_at("remove", n);
            let _ = clear_of_with(dir.path(), &fault);
            for name in ["archives", "library", "podcasts"] {
                assert!(is_owned(&dir.path().join(name)), "panne {n} : {name}");
            }
            assert!(dir.path().join("documents/x.txt").is_file());
        }
        clear_of(dir.path()).unwrap();
        assert_eq!(usage_of(dir.path()).unwrap(), 0);
        assert!(dir.path().join("documents/x.txt").is_file());
    }

    #[test]
    fn la_migration_marque_les_sous_dossiers_existants_une_seule_fois() {
        let config = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        legacy_cache(cache.path());
        write(&cache.path().join("thumbs"), 1);
        let roots = [cache.path().to_path_buf()];

        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::Done { marked: 2 }
        );
        assert!(is_owned(&cache.path().join("library")));
        assert!(is_owned(&cache.path().join("archives")));
        assert!(cache.path().join("thumbs").is_file());
        assert!(witness.is_file());

        // Une seule fois : un homonyme apparu ensuite n'est plus marqué.
        write(&cache.path().join("podcasts/a.mp3"), 3);
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&cache.path().join("podcasts")));
    }

    #[test]
    fn la_migration_interrompue_est_rejouee_et_idempotente() {
        let config = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        let names = ["archives", "catalogs", "official"];
        for name in names {
            write(&cache.path().join(name).join("x"), 1);
        }
        let roots = [cache.path().to_path_buf()];
        // Panne au deuxième marqueur, puis à l'écriture du témoin.
        assert!(migrate_markers_with(&witness, &roots, &fail_at("marker", 2)).is_err());
        assert!(!witness.exists());
        assert!(is_owned(&cache.path().join("archives")));
        assert!(!is_owned(&cache.path().join("catalogs")));
        assert!(migrate_markers_with(&witness, &roots, &fail_at("witness", 1)).is_err());
        assert!(!witness.exists());
        // Reprise : termine, sans rien marquer deux fois.
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::Done { marked: 0 }
        );
        for name in names {
            assert!(is_owned(&cache.path().join(name)));
            assert!(cache.path().join(name).join("x").is_file(), "rien supprimé");
        }
    }

    /// Cache rempli par une version antérieure : bibliothèque avec son index, sans marqueur.
    fn legacy_cache(root: &Path) {
        fs::create_dir_all(root.join("library")).unwrap();
        fs::write(
            root.join("library/library.json"),
            r#"{"schema_version":1,"stories":[]}"#,
        )
        .unwrap();
        write(&root.join("library/abc-1.plain.pk"), 10);
        write(&root.join("archives/a.zip"), 5);
    }

    #[test]
    fn un_ancien_cache_est_adopte_et_seule_la_bibliotheque_est_marquee() {
        let dir = tempfile::tempdir().unwrap();
        legacy_cache(dir.path());
        // Adoption interrompue avant le marqueur : rien d'écrit ni de supprimé, reprise complète.
        assert!(adopt_legacy_cache_with(dir.path(), &fail_at("marker", 1)).is_err());
        assert!(!is_owned(&dir.path().join("library")));
        assert!(adopt_legacy_cache(dir.path()).unwrap());
        assert!(adopt_legacy_cache(dir.path()).unwrap(), "idempotente");
        assert!(is_owned(&dir.path().join("library")));
        assert!(
            !is_owned(&dir.path().join("archives")),
            "seul library est marqué"
        );
        assert!(dir.path().join("library/abc-1.plain.pk").is_file());
        assert_eq!(foreign_entries(dir.path()), ["archives"]);
    }

    #[test]
    fn un_verrou_sans_index_ne_suffit_pas() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("library/.lock"), 0);
        write(&dir.path().join("library/roman.epub"), 3);
        assert!(!adopt_legacy_cache(dir.path()).unwrap());
        assert!(!is_owned(&dir.path().join("library")));
    }

    #[test]
    fn un_index_au_dela_de_la_borne_n_est_pas_reconnu() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = br#"{"schema_version":1,"stories":[]}"#.to_vec();
        index.resize(4 * 1024 * 1024 + 1, b' ');
        write(&dir.path().join("library/x"), 1);
        fs::write(dir.path().join("library/library.json"), index).unwrap();
        assert!(!adopt_legacy_cache(dir.path()).unwrap());
        assert!(!is_owned(&dir.path().join("library")));
    }

    #[test]
    fn un_ancien_cache_avec_des_homonymes_personnels_est_accepte_sans_les_toucher() {
        let dir = tempfile::tempdir().unwrap();
        legacy_cache(dir.path());
        write(&dir.path().join("podcasts/mon-emission.mp3"), 300);
        write(&dir.path().join("thumbs"), 1);
        assert!(adopt_legacy_cache(dir.path()).unwrap());
        assert!(is_owned(&dir.path().join("library")));
        assert!(!is_owned(&dir.path().join("podcasts")));
        assert!(dir.path().join("thumbs").is_file());
        // Le vidage vide la bibliothèque marquée, jamais `podcasts/` personnel.
        clear_of(dir.path()).unwrap();
        assert!(dir.path().join("podcasts/mon-emission.mp3").is_file());
        assert!(!dir.path().join("podcasts").join(MARKER).exists());
        assert_eq!(names_in(&dir.path().join("library")), [MARKER]);
    }

    #[test]
    fn sans_bibliotheque_reconnue_rien_n_est_adopte_ni_marque() {
        // `podcasts/` personnel seul.
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("podcasts/a.mp3"), 3);
        assert!(!adopt_legacy_cache(dir.path()).unwrap());
        assert!(!is_owned(&dir.path().join("podcasts")));
        // `library/` personnel : index illisible ou d'un autre schéma.
        for index in [
            "pas du json",
            r#"{"schema_version":99,"stories":[]}"#,
            r#"{"livres":[]}"#,
        ] {
            let dir = tempfile::tempdir().unwrap();
            write(&dir.path().join("library/x.epub"), 3);
            fs::write(dir.path().join("library/library.json"), index).unwrap();
            assert!(!adopt_legacy_cache(dir.path()).unwrap(), "{index}");
            assert!(!is_owned(&dir.path().join("library")));
        }
    }

    #[test]
    fn pas_de_temoin_quand_le_dossier_resolu_n_est_pas_celui_des_reglages() {
        let config = tempfile::tempdir().unwrap();
        let default = config.path().join("cache-defaut");
        fs::create_dir_all(&default).unwrap();
        let env = Environment {
            exe_dir: None,
            default_cache_dir: default.clone(),
            search_repo_build: false,
            protected_dirs: vec![config.path().join("protege")],
        };
        // Dossier configuré refusé à la relecture (dossier protégé) : repli sur le défaut.
        let configured = config.path().join("protege");
        fs::write(
            config.path().join("settings.json"),
            serde_json::json!({ "schema_version": 1, "cache_dir": configured }).to_string(),
        )
        .unwrap();
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Deferred
        );
        assert!(!config.path().join(MIGRATION_WITNESS).exists());

        // Sans dossier configuré : migration faite sur le défaut, témoin écrit.
        fs::write(
            config.path().join("settings.json"),
            r#"{"schema_version":1}"#,
        )
        .unwrap();
        assert!(matches!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Done { .. }
        ));
        assert!(config.path().join(MIGRATION_WITNESS).is_file());
    }

    #[test]
    fn la_migration_est_reportee_si_le_cache_est_absent() {
        let config = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        let roots = [config.path().join("disque-debranche")];
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::Deferred
        );
        assert!(!witness.exists());
    }

    fn env_with(default: &Path, protected: Vec<PathBuf>) -> Environment {
        Environment {
            exe_dir: None,
            default_cache_dir: default.to_path_buf(),
            search_repo_build: false,
            protected_dirs: protected,
        }
    }

    #[test]
    fn valeur_configuree_refusee_le_dossier_par_defaut_est_migre_sans_temoin() {
        let config = tempfile::tempdir().unwrap();
        let default = config.path().join("cache-defaut");
        let configured = config.path().join("protege");
        let env = env_with(&default, vec![configured.clone()]);
        fs::write(
            config.path().join("settings.json"),
            serde_json::json!({ "schema_version": 1, "cache_dir": configured }).to_string(),
        )
        .unwrap();
        // Le dossier par défaut, en usage pendant le repli, contient un ancien cache.
        legacy_cache(&default);
        // Panne au premier marqueur : rien d'écrit d'autre, pas de témoin, reprise complète.
        assert!(migrate_configured_with(config.path(), &env, &fail_at("marker", 1)).is_err());
        assert!(!config.path().join(MIGRATION_WITNESS).exists());
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Deferred
        );
        assert!(is_owned(&default.join("library")));
        assert!(is_owned(&default.join("archives")));
        assert!(
            default.join("library/abc-1.plain.pk").is_file(),
            "rien supprimé"
        );
        assert!(
            !config.path().join(MIGRATION_WITNESS).exists(),
            "dossier configuré toujours à migrer"
        );
    }

    #[test]
    fn migration_faite_le_demarrage_ne_relit_ni_reglages_ni_cache() {
        let config = tempfile::tempdir().unwrap();
        fs::write(config.path().join(MIGRATION_WITNESS), b"").unwrap();
        // Réglages illisibles : un `load` les mettrait de côté en `*.bak`.
        fs::write(config.path().join("settings.json"), b"{corrompu").unwrap();
        let env = env_with(&config.path().join("absent"), vec![]);
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        assert_eq!(
            names_in(config.path()),
            [MIGRATION_WITNESS, "settings.json"]
        );
    }

    #[test]
    fn la_migration_exige_une_preuve_d_usage() {
        let config = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path();
        // Homonymes de l'utilisateur : contenu d'une autre forme que celle de l'application.
        write(&root.join("library/livres/roman.epub"), 3);
        write(&root.join("library/.lock"), 0);
        write(&root.join("podcasts/mon-emission.mp3"), 3);
        write(&root.join("podcast-work/projet/notes.txt"), 3);
        write(&root.join("play/Partie 1/sauvegarde.bin"), 3);
        let roots = [root.to_path_buf()];
        assert_eq!(
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap(),
            Migration::Done { marked: 0 }
        );
        for name in ["library", "podcasts", "podcast-work", "play"] {
            assert!(!is_owned(&root.join(name)), "{name} non prouvé");
        }
        // Le vidage les épargne tous, et l'interface ne les annonce pas.
        assert_eq!(clear_of(root).unwrap(), 0);
        assert!(root.join("podcasts/mon-emission.mp3").is_file());
        assert!(root.join("library/livres/roman.epub").is_file());
        assert_eq!(usage_report(root).unwrap(), (0, vec![]));
    }

    #[test]
    fn la_migration_marque_les_contenus_de_la_forme_de_l_application() {
        let config = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path();
        legacy_cache(root);
        write(
            &root.join("podcasts/4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c/feed.xml"),
            3,
        );
        write(&root.join("podcasts/src_2/meta.json"), 3);
        write(
            &root.join("podcast-work/4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c/pack.zip"),
            3,
        );
        write(&root.join("play/0123456789abcdef/story/x.bin"), 3);
        // Vides : rien à perdre, marqués.
        let empty = tempfile::tempdir().unwrap();
        fs::create_dir_all(empty.path().join("library")).unwrap();
        fs::create_dir_all(empty.path().join("podcasts")).unwrap();
        let roots = [root.to_path_buf(), empty.path().to_path_buf()];
        assert_eq!(
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap(),
            Migration::Done { marked: 7 }
        );
        for name in ["library", "archives", "podcasts", "podcast-work", "play"] {
            assert!(is_owned(&root.join(name)), "{name}");
        }
        assert!(is_owned(&empty.path().join("library")));
        assert!(is_owned(&empty.path().join("podcasts")));
    }

    /// `library/` d'une bêta antérieure sans index (application ouverte sans rien installer,
    /// installation échouée, index mis de côté) : reconnue à la forme de ses éléments (#591).
    #[test]
    fn une_bibliotheque_sans_index_est_reconnue_a_sa_forme() {
        let marked = |setup: &dyn Fn(&Path)| {
            let config = tempfile::tempdir().unwrap();
            let cache = tempfile::tempdir().unwrap();
            let library = cache.path().join("library");
            fs::create_dir_all(&library).unwrap();
            setup(&library);
            let roots = [cache.path().to_path_buf()];
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap();
            is_owned(&library)
        };
        assert!(marked(&|l| write(&l.join(".lock"), 0)), "{{.lock}}");
        assert!(marked(&|l| {
            write(&l.join(".lock"), 0);
            fs::create_dir_all(l.join(".staging")).unwrap();
            fs::create_dir_all(l.join("thumbs")).unwrap();
        }));
        assert!(marked(&|l| {
            write(&l.join(".lock"), 0);
            write(&l.join("library.json.20260101T000000.bak"), 3);
            write(&l.join(".library.json.42-0.tmp"), 3);
            write(&l.join("abc-1.plain.pk"), 3);
            write(&l.join("def-2.lunii.pk"), 3);
        }));
        assert!(marked(&|_| {}), "vide");
        // Un seul élément d'une autre forme : refus.
        assert!(!marked(&|l| {
            write(&l.join(".lock"), 0);
            write(&l.join("roman.epub"), 3);
        }));
        assert!(!marked(&|l| {
            write(&l.join(".lock"), 0);
            fs::create_dir_all(l.join("livres")).unwrap();
        }));
        assert!(!marked(&|l| write(&l.join("thumbs"), 3)), "thumbs fichier");
        assert!(!marked(&|l| fs::create_dir_all(l.join(".lock")).unwrap()));
    }

    #[test]
    fn un_seul_enfant_inconnu_empeche_le_marquage() {
        for (name, good, bad) in [
            ("podcasts", "src", "Mes Podcasts"),
            (
                "podcast-work",
                "4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c",
                "4f1c2a9e0b7d4c3e9a512d6e8f0a1b3c",
            ),
            ("play", "0123456789abcdef", "0123456789abcdeg"),
        ] {
            let config = tempfile::tempdir().unwrap();
            let cache = tempfile::tempdir().unwrap();
            fs::create_dir_all(cache.path().join(name).join(good)).unwrap();
            fs::create_dir_all(cache.path().join(name).join(bad)).unwrap();
            let roots = [cache.path().to_path_buf()];
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap();
            assert!(!is_owned(&cache.path().join(name)), "{name}/{bad}");
            // Un fichier à la place d'un dossier ne prouve rien non plus.
            let cache = tempfile::tempdir().unwrap();
            write(&cache.path().join(name).join(good), 1);
            let config = tempfile::tempdir().unwrap();
            let roots = [cache.path().to_path_buf()];
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap();
            assert!(!is_owned(&cache.path().join(name)), "{name}/{good} fichier");
        }
    }

    #[test]
    fn l_usage_n_annonce_que_les_sous_dossiers_marques() {
        let dir = tempfile::tempdir().unwrap();
        owned(dir.path(), "library");
        owned(dir.path(), "archives");
        write(&dir.path().join("archives/a.zip"), 4);
        // Homonyme non marqué : ni compté, ni vidé, ni annoncé.
        write(&dir.path().join("podcasts/a.mp3"), 9);
        assert_eq!(
            usage_report(dir.path()).unwrap(),
            (4, vec!["archives".to_string(), "library".to_string()])
        );
    }

    #[cfg(windows)]
    #[test]
    fn une_racine_en_jonction_est_migree() {
        let config = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        legacy_cache(target.path());
        let link = config.path().join("cache-lien");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(target.path())
            .output()
            .unwrap();
        assert!(status.status.success(), "mklink /J a échoué");
        let witness = config.path().join(MIGRATION_WITNESS);
        assert_eq!(
            migrate_markers(&witness, std::slice::from_ref(&link)).unwrap(),
            Migration::Done { marked: 2 }
        );
        assert!(witness.is_file(), "plus reportée à chaque démarrage");
        assert!(is_owned(&link.join("library")));
        assert_eq!(usage_of(&link).unwrap(), 15 + 33);
    }

    #[cfg(windows)]
    #[test]
    fn un_lien_ou_une_jonction_n_est_ni_suivi_ni_supprime() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("precieux.txt"), 10);
        write(&outside.path().join(MARKER), 0);
        let cache = dir.path().join("cache");
        fs::create_dir_all(&cache).unwrap();
        let junction = |link: &Path| {
            // Jonction (sans privilège) créée via `mklink /J`.
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(status.status.success(), "mklink /J a échoué");
        };
        // Au premier niveau : jamais retenue, même si la cible porte un marqueur.
        junction(&cache.join("library"));
        // À l'intérieur d'un sous-dossier marqué : supprimée en tant que lien.
        owned(&cache, "play");
        junction(&cache.join("play").join("lien"));

        assert_eq!(usage_of(&cache).unwrap(), 0);
        assert_eq!(clear_of(&cache).unwrap(), 0);
        assert!(outside.path().join("precieux.txt").is_file());
        assert!(fs::symlink_metadata(cache.join("library")).is_ok());
        assert!(fs::symlink_metadata(cache.join("play").join("lien")).is_err());
        assert_eq!(foreign_entries(&cache), ["library"]);
    }

    #[test]
    fn la_profondeur_est_bornee() {
        let dir = tempfile::tempdir().unwrap();
        let mut deep = owned(dir.path(), "library");
        for i in 0..(MAX_DEPTH + 5) {
            deep = deep.join(format!("d{i}"));
        }
        write(&deep.join("profond.bin"), 40);
        write(&dir.path().join("library/proche.bin"), 2);
        // Le fichier trop profond n'est pas compté, mais le calcul se termine.
        assert_eq!(usage_of(dir.path()).unwrap(), 2);
        // Le vidage supprime tout, y compris le profond.
        assert_eq!(clear_of(dir.path()).unwrap(), 2);
        assert_eq!(names_in(&dir.path().join("library")), [MARKER]);
    }

    #[test]
    fn un_dossier_disparu_pendant_le_calcul_compte_pour_zero() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(dir_size(&dir.path().join("disparu")).unwrap(), 0);
    }

    #[test]
    fn les_sous_dossiers_exposes_sont_ceux_de_l_application() {
        assert_eq!(
            APP_CACHE_SUBDIRS,
            [
                "archives",
                "catalogs",
                "compat-local",
                "library",
                "official",
                "play",
                "podcast-work",
                "podcasts",
                "thumbs"
            ]
        );
    }

    #[tokio::test]
    async fn le_vidage_attend_les_rafraichissements_en_cours() {
        let service = CatalogService::with_client(reqwest::Client::new());
        let pause = service.pause_refreshes().await;
        // Un rafraîchissement ne peut pas démarrer pendant le vidage.
        let dir = tempfile::tempdir().unwrap();
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            service.refresh(dir.path(), "src", "http://127.0.0.1:1/", 0),
        )
        .await;
        assert!(blocked.is_err(), "le rafraîchissement doit attendre");
        drop(pause);
    }
}
