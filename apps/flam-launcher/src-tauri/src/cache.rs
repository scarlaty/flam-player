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

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::catalog::{blocking, CatalogService};
use crate::player::PlayerService;
use crate::settings::{resolve, Environment, SettingsStore};
use crate::storage::ConfigState;
use crate::AppResult;

/// Sous-dossiers du cache créés par l'application (et eux seuls) : archives téléchargées, catalogues mis en cache,
/// histoires converties, catalogue officiel en ligne (`official`), copies de travail des
/// histoires lancées (`play`), flux podcast mis en cache (`podcasts`, #211), vignettes, travail
/// de préparation d'un envoi vers la Flam (`flam-transfer`, #591).
pub const APP_CACHE_SUBDIRS: &[&str] = &[
    "archives",
    "catalogs",
    "compat-local",
    "flam-transfer",
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

/// Sous-dossiers ajoutés à [`APP_CACHE_SUBDIRS`] après la première migration : un cache déjà
/// migré (témoin [`MIGRATION_WITNESS`] présent) ne les a jamais évalués.
const LATE_SUBDIRS: &[&str] = &[crate::device::transfer::WORK_SUBDIR];

/// Témoin de la migration tardive de [`LATE_SUBDIRS`] (`migrate_late`), dans le dossier de
/// configuration : écrit une fois ces sous-dossiers évalués, ou par la migration complète.
const LATE_WITNESS: &str = "cache-markers-transfer.done";

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
    /// Témoin déjà présent, aucun sous-dossier en attente marqué par le rattrapage.
    AlreadyDone,
    /// Témoin déjà présent : le rattrapage ([`catch_up`]) a marqué `marked` sous-dossiers en
    /// attente, devenus prouvés.
    CaughtUp { marked: usize },
    /// Marqueurs posés (`marked` nouveaux) puis témoin écrit.
    Done { marked: usize },
    /// Un dossier de cache est absent : rejouée au prochain démarrage.
    Deferred,
}

/// Sous-dossier `name` de la racine `root` vu et refusé faute de preuve par la migration, en
/// attente du rattrapage ([`catch_up`]). Persisté dans le témoin de migration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Pending {
    root: String,
    name: String,
}

/// Contenu du témoin de migration : vide (aucun sous-dossier en attente, forme écrite aussi par
/// `SettingsStore::update`) ou JSON `{"pending":[{"root":…,"name":…}]}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct WitnessFile {
    #[serde(default)]
    pending: Vec<Pending>,
}

/// Taille maximale du témoin lu (au-delà : rien en attente).
const MAX_WITNESS_BYTES: u64 = 1024 * 1024;

/// Sous-dossiers en attente d'après le témoin. Témoin vide, illisible, trop gros ou de forme
/// inattendue : rien en attente (aucun marquage possible par le rattrapage). Une entrée dont le
/// nom n'est pas dans [`APP_CACHE_SUBDIRS`] ou dont la racine n'est pas absolue est ignorée.
fn read_pending(witness: &Path) -> Vec<Pending> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let read = fs::File::open(witness).and_then(|file| {
        file.take(MAX_WITNESS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map(drop)
    });
    if read.is_err() || bytes.len() as u64 > MAX_WITNESS_BYTES || bytes.is_empty() {
        return Vec::new();
    }
    match serde_json::from_slice::<WitnessFile>(&bytes) {
        Ok(file) => file
            .pending
            .into_iter()
            .filter(|p| {
                APP_CACHE_SUBDIRS.contains(&p.name.as_str()) && Path::new(&p.root).is_absolute()
            })
            .collect(),
        Err(err) => {
            tracing::warn!("cache : témoin de migration illisible, rien en attente : {err}");
            Vec::new()
        }
    }
}

/// Écrit le témoin (écriture atomique) : vide sans sous-dossier en attente, JSON sinon.
fn write_witness(witness: &Path, pending: &[Pending]) -> io::Result<()> {
    let bytes = if pending.is_empty() {
        Vec::new()
    } else {
        let file = WitnessFile {
            pending: pending.to_vec(),
        };
        serde_json::to_vec_pretty(&file).map_err(io::Error::other)?
    };
    if let Some(parent) = witness.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::storage::write_atomic(witness, &bytes)
}

/// Racines du cache au démarrage, pour le rattrapage ([`catch_up`]).
#[derive(Debug, Default)]
struct CurrentRoots {
    /// Dossier de cache effectif et dossier par défaut : leurs entrées sont réévaluées.
    eval: Vec<PathBuf>,
    /// Dossier configuré mais non retenu par la résolution des réglages (repli sur le défaut) :
    /// ses entrées sont gardées sans être évaluées (il peut redevenir le cache effectif).
    keep: Vec<PathBuf>,
}

/// Racines courantes d'après les réglages de `config_dir`, lus **sans effet de bord**
/// ([`SettingsStore::peek_readable`]). Réglages illisibles, d'une version future, ou erreur de
/// lecture : `None` (rien n'est évalué ni retiré).
fn current_roots(config_dir: &Path, env: &Environment) -> Option<CurrentRoots> {
    let settings = match SettingsStore::new(config_dir).peek_readable() {
        Ok(Some(settings)) => settings,
        Ok(None) => return None,
        Err(err) => {
            tracing::warn!("cache : réglages illisibles pour le rattrapage : {err}");
            return None;
        }
    };
    let effective = PathBuf::from(resolve(&settings, env).0.cache_dir);
    let mut roots = CurrentRoots {
        eval: vec![effective.clone(), env.default_cache_dir.clone()],
        keep: Vec::new(),
    };
    if let Some(configured) = settings.cache_dir {
        if !crate::settings::same_path(Path::new(&configured), &effective) {
            roots.keep.push(PathBuf::from(configured));
        }
    }
    Some(roots)
}

/// Rattrapage à chaque démarrage où le témoin existe (#591), et seulement au démarrage (porte de
/// démarrage fermée) : chaque sous-dossier refusé par la migration faute de preuve (liste du
/// témoin, [`read_pending`]) est réévalué avec la même preuve ([`proven_by_content`]) et le même
/// chemin de marquage que la migration, **si sa racine est le dossier de cache effectif ou le
/// dossier par défaut** (`current`, comparés par `same_path`). Un sous-dossier redevenu prouvé
/// (par exemple `archives/` vidé par une installation reprise après un `.part` sans mémoire) est
/// marqué et sort de la liste.
///
/// Sortent aussi de la liste : un sous-dossier déjà marqué ou absent (il sera créé marqué par
/// [`ensure_subdir`]) d'une racine existante ; toute entrée d'une racine qui n'est plus ni le
/// dossier effectif, ni le dossier par défaut, ni le dossier configuré non retenu (dossier de
/// cache quitté : un retour ultérieur sur cette racine passe par l'adoption, [`adopt_legacy_cache`],
/// et elle seule). Restent en attente, sans rien écrire : racine absente ou qui n'est pas un
/// dossier (disque débranché, partage hors ligne), contenu non prouvé, élément qui n'est pas un
/// dossier réel, marqueur anormal, erreur d'écriture du marqueur, dossier configuré non retenu.
///
/// La liste vide, rien n'est lu (ni réglages ni cache). `current` est appelé seulement si la
/// liste n'est pas vide ; `None` (réglages illisibles) : rien n'est évalué ni retiré. Un dossier
/// de cache choisi après la mise à jour n'est jamais réévalué ici. Ne renvoie jamais d'erreur :
/// un échec de réécriture du témoin est journalisé, la liste précédente est relue au démarrage
/// suivant (les entrées marquées entre-temps en sortent alors ; rien n'est marqué sans preuve).
fn catch_up(witness: &Path, current: &dyn Fn() -> Option<CurrentRoots>, fault: Fault) -> Migration {
    let pending = read_pending(witness);
    if pending.is_empty() {
        return Migration::AlreadyDone;
    }
    let Some(current) = current() else {
        tracing::warn!("cache : dossier de cache effectif inconnu, rattrapage reporté");
        return Migration::AlreadyDone;
    };
    let among =
        |roots: &[PathBuf], root: &Path| roots.iter().any(|r| crate::settings::same_path(r, root));
    let mut marked = 0;
    let mut kept = Vec::new();
    for entry in &pending {
        let root = Path::new(&entry.root);
        let Some(name) = APP_CACHE_SUBDIRS.iter().find(|n| **n == entry.name) else {
            continue;
        };
        if !among(&current.eval, root) {
            if among(&current.keep, root) {
                kept.push(entry.clone());
            } else {
                tracing::info!("cache : « {name} » d'un ancien dossier de cache, plus en attente");
            }
            continue;
        }
        if !root.is_dir() {
            kept.push(entry.clone());
            continue;
        }
        let dir = root.join(name);
        match fs::symlink_metadata(&dir) {
            Err(err) if err.kind() == ErrorKind::NotFound => continue,
            _ if is_owned(&dir) => continue,
            _ => {}
        }
        match mark_one(root, name, Marking::Migration, fault) {
            Ok(MarkOutcome::Marked) => marked += 1,
            Ok(_) => kept.push(entry.clone()),
            Err(err) => {
                tracing::warn!("cache : marqueur de « {name} » non écrit au rattrapage : {err}");
                kept.push(entry.clone());
            }
        }
    }
    if kept.len() != pending.len() {
        if let Err(err) = fault("pending").and_then(|()| write_witness(witness, &kept)) {
            tracing::warn!("cache : liste des sous-dossiers en attente non mise à jour : {err}");
        }
    }
    if marked > 0 {
        Migration::CaughtUp { marked }
    } else {
        Migration::AlreadyDone
    }
}

/// Migration unique (#570) : les sous-dossiers de [`APP_CACHE_SUBDIRS`] déjà présents dans les
/// dossiers de cache `roots` (celui déjà configuré, et le dossier par défaut) servaient déjà à
/// l'application ; ils reçoivent le marqueur seulement si leur contenu prouve cet usage (voir
/// [`proven_by_content`], #591). Un sous-dossier non prouvé (homonyme de l'utilisateur, ou reste
/// d'un arrêt brutal de forme ambiguë) n'est pas marqué, journalisé, et ne fait pas échouer la
/// migration : il n'est pas compté ni vidé tant qu'il n'est pas prouvé (fuite de place, jamais
/// de perte). Il est inscrit dans le témoin `witness` et réévalué à chaque démarrage suivant
/// ([`catch_up`]) jusqu'à être prouvé. Le témoin est écrit en dernier, de façon atomique, avec
/// cette liste : interrompue (ou en échec d'écriture du témoin), la migration est rejouée au
/// démarrage suivant (elle est idempotente) ; témoin présent, seul le rattrapage est fait. Une
/// racine est un dossier choisi par l'utilisateur : elle peut être une jonction ou un lien
/// (#591), seul le premier niveau doit être un dossier réel. Un sous-dossier absent n'est pas
/// inscrit : `ensure_subdir` le créera marqué.
pub fn migrate_markers(witness: &Path, roots: &[PathBuf]) -> io::Result<Migration> {
    migrate_markers_with(witness, roots, &no_fault)
}

fn migrate_markers_with(witness: &Path, roots: &[PathBuf], fault: Fault) -> io::Result<Migration> {
    if fs::symlink_metadata(witness).is_ok() {
        // Racines courantes : celles passées par l'appelant.
        let current = || {
            Some(CurrentRoots {
                eval: roots.to_vec(),
                keep: Vec::new(),
            })
        };
        return Ok(catch_up(witness, &current, fault));
    }
    if roots.iter().any(|root| !root.is_dir()) {
        return Ok(Migration::Deferred);
    }
    let mut marked = 0;
    let mut pending = Vec::new();
    for root in roots {
        let (count, unproven) = mark_existing(root, Marking::Migration, fault)?;
        marked += count;
        match root.to_str() {
            Some(text) => pending.extend(unproven.into_iter().map(|name| Pending {
                root: text.to_string(),
                name: name.to_string(),
            })),
            None if !unproven.is_empty() => tracing::warn!(
                "cache : racine au nom non UTF-8, sous-dossiers non prouvés jamais réévalués"
            ),
            None => {}
        }
    }
    fault("witness")?;
    write_witness(witness, &pending)?;
    Ok(Migration::Done { marked })
}

/// Origine d'un marquage de sous-dossiers existants : [`mark_existing`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marking {
    /// Migration de démarrage ([`migrate_markers`]) : preuve [`proven_by_content`], une erreur
    /// d'écriture interrompt la migration.
    Migration,
    /// Adoption d'un ancien cache ([`adopt_legacy_cache`]) : preuve [`proven_at_adoption`], une
    /// erreur d'écriture est journalisée et le sous-dossier reste non marqué.
    Adoption,
}

/// Résultat de [`mark_one`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MarkOutcome {
    /// Marqueur écrit.
    Marked,
    /// Rien à faire ou refus sans rapport avec la preuve : absent, pas un dossier réel (fichier,
    /// lien, jonction), déjà marqué, marqueur anormal (laissé tel quel).
    Skipped,
    /// Dossier réel sans marqueur dont le contenu ne prouve pas l'usage par l'application.
    Unproven,
}

/// Marque les sous-dossiers de [`APP_CACHE_SUBDIRS`] présents dans `root` (dossiers réels sans
/// marqueur, dont le contenu prouve l'usage par l'application, selon `marking`). Renvoie leur
/// nombre et les noms refusés faute de preuve ([`MarkOutcome::Unproven`]). Idempotente. Un
/// marqueur anormal (lien, dossier) est laissé tel quel, sans marquer ce sous-dossier.
fn mark_existing(
    root: &Path,
    marking: Marking,
    fault: Fault,
) -> io::Result<(usize, Vec<&'static str>)> {
    let mut marked = 0;
    let mut unproven = Vec::new();
    // Les sous-dossiers de [`LATE_SUBDIRS`] en dernier : leur marquage exige qu'un autre
    // sous-dossier de l'application soit déjà marqué ([`has_app_trace`]).
    let ordered = APP_CACHE_SUBDIRS
        .iter()
        .filter(|name| !LATE_SUBDIRS.contains(*name))
        .chain(LATE_SUBDIRS);
    for name in ordered {
        match mark_one(root, name, marking, fault) {
            Ok(MarkOutcome::Marked) => marked += 1,
            Ok(MarkOutcome::Unproven) => unproven.push(*name),
            Ok(MarkOutcome::Skipped) => {}
            // Adoption : un sous-dossier non marqué ne fait que fuir, le changement de dossier
            // de cache n'échoue pas pour lui (#591).
            Err(err) if marking == Marking::Adoption => {
                tracing::warn!("cache : marqueur de « {name} » non écrit à l'adoption : {err}");
            }
            Err(err) => return Err(err),
        }
    }
    Ok((marked, unproven))
}

/// Marque `root/name` s'il est un dossier réel sans marqueur dont le contenu prouve l'usage par
/// l'application, selon `marking` (voir [`MarkOutcome`]). Une erreur d'écriture du marqueur est
/// renvoyée.
fn mark_one(
    root: &Path,
    name: &'static str,
    marking: Marking,
    fault: Fault,
) -> io::Result<MarkOutcome> {
    let dir = root.join(name);
    if !is_real_dir(&dir) || is_owned(&dir) {
        return Ok(MarkOutcome::Skipped);
    }
    if fs::symlink_metadata(dir.join(MARKER)).is_ok() {
        tracing::warn!("cache : marqueur de « {name} » inattendu, non marqué");
        return Ok(MarkOutcome::Skipped);
    }
    if LATE_SUBDIRS.contains(&name) && !has_app_trace(root) {
        tracing::warn!("cache : « {name} » sans autre sous-dossier de l'application, non marqué");
        return Ok(MarkOutcome::Unproven);
    }
    let proven = match marking {
        Marking::Migration => proven_by_content(name, &dir),
        Marking::Adoption => proven_at_adoption(name, &dir),
    };
    if !proven {
        tracing::warn!("cache : contenu de « {name} » non reconnu, non marqué (ni compté ni vidé)");
        return Ok(MarkOutcome::Unproven);
    }
    // Course preuve/marquage : migration et rattrapage s'exécutent porte de démarrage fermée
    // (aucune commande ni installation de l'application n'écrit pendant ce temps) ; seule une
    // écriture extérieure simultanée (autre programme, utilisateur) entre la preuve et le
    // marqueur pourrait faire marquer un contenu non prouvé : risque accepté. `archives/` relit
    // en outre son dossier deux fois (noms, puis parcours) : un changement entre les deux
    // relèverait du même cas.
    // `create_new` : n'ouvre jamais un élément existant (ni lien, ni fichier).
    fault("marker")?;
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dir.join(MARKER))?;
    Ok(MarkOutcome::Marked)
}

/// `root` porte une trace d'usage par l'application : au moins un sous-dossier de
/// [`APP_CACHE_SUBDIRS`] autre que [`LATE_SUBDIRS`] déjà marqué (vrai dossier avec marqueur).
/// Exigée avant de marquer `flam-transfer` : ses noms (`str/<uuid>/` avec `key`, `info`,
/// `main.lsf`, `img/`, `sounds/`) sont aussi ceux d'une copie des histoires de la Flam (format de
/// l'appareil), qu'un utilisateur peut avoir rangée sous ce nom dans un dossier qui n'a jamais
/// servi de cache : la preuve par les noms ne suffit pas à elle seule.
fn has_app_trace(root: &Path) -> bool {
    APP_CACHE_SUBDIRS
        .iter()
        .filter(|name| !LATE_SUBDIRS.contains(*name))
        .any(|name| is_owned(&root.join(name)))
}

/// Preuve d'usage exigée à l'adoption d'un ancien cache (#591), au moins aussi stricte que celle
/// de la migration : le dossier a été choisi par l'utilisateur et peut contenir ses propres
/// dossiers.
/// - `podcasts`, `podcast-work`, `play`, `flam-transfer` : règle de la migration
///   ([`proven_by_content`]) ;
/// - `thumbs` : chaque élément de la racine est un fichier ordinaire nommé comme ceux de
///   `ThumbCache` ([`crate::thumbnail::is_own_file`]) ou un fichier de l'explorateur
///   ([`is_os_junk`], ignoré sans valoir preuve) ; vide : accepté, comme `podcasts` ou `play` ;
/// - `archives`, `catalogs`, `compat-local`, `official` : jamais (la preuve par le contenu de
///   la migration n'est pas appliquée à l'adoption) ;
/// - `library` : jamais ici (marqué par [`adopt_legacy_cache`] sur l'index reconnu).
fn proven_at_adoption(name: &str, dir: &Path) -> bool {
    match name {
        crate::podcast::source::CACHE_SUBDIR
        | crate::install::podcast::WORK_DIR
        | crate::player::PLAY_DIR
        | crate::device::transfer::WORK_SUBDIR => proven_by_content(name, dir),
        crate::thumbnail::THUMBS_DIR => thumbs_children_known(dir),
        _ => false,
    }
}

/// Tous les éléments de `thumbs/` sont des fichiers ordinaires écrits par `ThumbCache`, ou des
/// fichiers de l'explorateur. Un autre nom, un dossier ou un lien : refus.
fn thumbs_children_known(dir: &Path) -> bool {
    files_known(dir, &crate::thumbnail::is_own_file)
}

/// Chaque élément de `dir` est un fichier de l'explorateur ([`is_os_junk`], fichier ordinaire,
/// ignoré sans valoir preuve), un fichier ordinaire dont le nom est accepté par `file`, ou un
/// dossier réel accepté par `subdir` (nom, chemin). Vide : accepté. Lien ou jonction (type lu
/// sans suivre les liens), autre type, nom non UTF-8, erreur de lecture : refus.
fn entries_known(
    dir: &Path,
    file: &dyn Fn(&str) -> bool,
    subdir: &dyn Fn(&str, &Path) -> bool,
) -> bool {
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
            return subdir(&name, &entry.path());
        }
        kind.is_file() && (is_os_junk(&name) || file(&name))
    })
}

/// [`entries_known`] sans aucun sous-dossier accepté.
fn files_known(dir: &Path, file: &dyn Fn(&str) -> bool) -> bool {
    entries_known(dir, file, &|_, _| false)
}

/// `name` est `file` ou un temporaire d'écriture atomique de `file`
/// ([`crate::storage::atomic_tmp_target`]).
fn is_file_or_its_tmp(name: &str, file: &str) -> bool {
    name == file || crate::storage::atomic_tmp_target(name) == Some(file)
}

/// `archives/` ([`crate::install::ARCHIVES_DIR`]) : chaque élément est un fichier ordinaire (ou
/// un fichier de l'explorateur) parmi :
/// - une mémoire d'archive `<archive>.meta.json`, ou son temporaire d'écriture atomique ;
/// - une archive `<archive>` ou son téléchargement en cours `<archive>.part`, **accompagné** de
///   sa mémoire `<archive>.meta.json` dans le même dossier ;
///
/// où `<archive>` est `<uuid>-<version>.zip` ou `<uuid>-<version>.plain.pk`
/// ([`crate::install::is_archive_name`]). Une archive ou un `.part` sans mémoire (arrêt brutal
/// pendant un premier téléchargement, ou entre la fin du téléchargement et l'écriture de la
/// mémoire) n'est pas prouvé : `vacances-2019.zip` a la même forme.
fn archives_children_known(dir: &Path) -> bool {
    use crate::install::{download::PART_SUFFIX, is_archive_name, ARCHIVE_META_SUFFIX};
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    // Un nom non UTF-8 est omis ici, mais refusé par le parcours ci-dessous.
    let names: std::collections::HashSet<String> = entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    let is_meta = |name: &str| {
        name.strip_suffix(ARCHIVE_META_SUFFIX)
            .is_some_and(is_archive_name)
    };
    files_known(dir, &|name| {
        if is_meta(name) || crate::storage::atomic_tmp_target(name).is_some_and(is_meta) {
            return true;
        }
        let archive = name.strip_suffix(PART_SUFFIX).unwrap_or(name);
        // La mémoire, si elle existe, est elle-même vérifiée (fichier ordinaire) par ce parcours.
        is_archive_name(archive) && names.contains(&format!("{archive}{ARCHIVE_META_SUFFIX}"))
    })
}

/// `catalogs/` ([`crate::catalog::CATALOGS_DIR`]) : chaque élément est un dossier réel nommé
/// comme un identifiant de source ([`crate::catalog::is_source_dir_name`]) dont le contenu a
/// la forme écrite par `catalog` : `catalog.json`, `meta.json`, leurs temporaires d'écriture
/// atomique, et un dossier réel `thumbs/` qui ne contient que des vignettes et leur suivi
/// ([`crate::catalog::is_thumb_file`]) ou leurs temporaires. Fichiers de l'explorateur ignorés
/// à chaque niveau.
fn catalogs_children_known(dir: &Path) -> bool {
    use crate::catalog::{is_thumb_file, CACHE_FILE, META_FILE, THUMBS_DIR};
    let thumb = |name: &str| {
        is_thumb_file(name) || crate::storage::atomic_tmp_target(name).is_some_and(is_thumb_file)
    };
    let source = |path: &Path| {
        entries_known(
            path,
            &|name| is_file_or_its_tmp(name, CACHE_FILE) || is_file_or_its_tmp(name, META_FILE),
            &|name, thumbs| name == THUMBS_DIR && files_known(thumbs, &thumb),
        )
    };
    entries_known(dir, &|_| false, &|name, path| {
        crate::catalog::is_source_dir_name(name) && source(path)
    })
}

/// Preuve d'usage exigée par la migration (#591) : chaque sous-dossier de
/// [`APP_CACHE_SUBDIRS`] doit avoir un contenu de la forme que l'application y écrit, relevée
/// dans le code des versions publiées (bêta.1 à bêta.4) et de la version courante, arrêt brutal
/// compris ; en cas de doute, rien n'est marqué (fuite, jamais de perte) :
/// - `archives` : voir [`archives_children_known`] ;
/// - `catalogs` : voir [`catalogs_children_known`] ;
/// - `compat-local` : `launcher_results.json` ([`crate::compat::RESULTS_FILE`]) et son
///   temporaire d'écriture atomique ;
/// - `official` : `catalog.json` (`device::official`) et son temporaire ;
/// - `thumbs` : règle de l'adoption ([`thumbs_children_known`]) ;
/// - `library` : index reconnu ([`crate::library::is_legacy_library`]) ou dossier vide ;
/// - `podcasts` : chaque enfant est un dossier réel nommé comme un identifiant de source
///   (règle de `podcast::source::source_dir`) dont le contenu a la forme écrite par l'application
///   (voir [`podcast_source_children_known`]) ;
/// - `podcast-work` : chaque enfant est un dossier réel nommé par un identifiant de tâche
///   (`install::is_task_id`) ;
/// - `play` : chaque enfant est un dossier réel nommé par l'empreinte de `player::Workspace`
///   (16 chiffres hexadécimaux minuscules) ;
/// - `flam-transfer` : voir [`transfer_children_known`].
///
/// Partout, un sous-dossier vide est accepté (rien à perdre), les noms sont comparés en
/// respectant la casse, aucun lien ni jonction n'est suivi (refus), et `desktop.ini`,
/// `Thumbs.db` (casse indifférente) et `.DS_Store` sont ignorés s'ils sont des fichiers
/// ordinaires ([`is_os_junk`]) ; ils ne prouvent rien (un sous-dossier qui ne contient qu'eux est
/// jugé comme un sous-dossier vide). Un nom inconnu n'est jamais prouvé.
///
/// Limites connues (#591) : la preuve de `catalogs`, `official` et `compat-local` porte sur les
/// noms seulement (le contenu de `catalog.json`, `meta.json`, des vignettes n'est pas lu) ; un
/// dossier de source vide (`catalogs/<id>/`, `podcasts/<id>/`) est accepté. Un dossier
/// personnel qui imiterait exactement ces noms serait marqué.
fn proven_by_content(name: &str, dir: &Path) -> bool {
    let child_shape: fn(&str) -> bool = match name {
        crate::library::LIBRARY_DIR => {
            return crate::library::is_legacy_library(dir) || library_children_known(dir);
        }
        crate::install::ARCHIVES_DIR => return archives_children_known(dir),
        crate::catalog::CATALOGS_DIR => return catalogs_children_known(dir),
        crate::compat::RESULTS_DIR => {
            return files_known(dir, &|n| is_file_or_its_tmp(n, crate::compat::RESULTS_FILE));
        }
        crate::device::official::CACHE_DIR => {
            return files_known(dir, &|n| {
                is_file_or_its_tmp(n, crate::device::official::CACHE_FILE)
            });
        }
        crate::thumbnail::THUMBS_DIR => return thumbs_children_known(dir),
        crate::device::transfer::WORK_SUBDIR => return transfer_children_known(dir),
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
        _ => return false,
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        entry.is_ok_and(|entry| {
            let Ok(kind) = entry.file_type() else {
                return false;
            };
            // Fichiers déposés par l'explorateur (Windows, macOS) : ignorés, sans valoir preuve.
            if kind.is_file()
                && !kind.is_symlink()
                && entry.file_name().to_str().is_some_and(is_os_junk)
            {
                return true;
            }
            kind.is_dir()
                && !kind.is_symlink()
                && entry.file_name().to_str().is_some_and(child_shape)
                && (name != crate::podcast::source::CACHE_SUBDIR
                    || podcast_source_children_known(&entry.path()))
        })
    })
}

/// Identifiant d'histoire tel que l'écrit `prepare::read_uuid` : UUID en minuscules avec ses
/// tirets (`8-4-4-4-12` chiffres hexadécimaux).
fn is_story_uuid(name: &str) -> bool {
    name.len() == 36
        && name.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}

/// Fichiers de premier niveau d'une préparation (`str/<uuid>/`), relevés dans le code : `prepare`
/// (`info`, `main.lsf`, `version`, la clé `key`) et `prepare_lunii` (`ni`, `nm`, `li`, `ri`,
/// `si`, `info`, `key`).
const PREPARED_FILES: &[&str] = &[
    "key", "info", "main.lsf", "version", "ni", "nm", "li", "ri", "si",
];

/// Dossiers de premier niveau d'une préparation : `img`, `sounds`, `script` (histoire Flam) et
/// `rf`, `sf` (pack Lunii). Le contenu n'est pas examiné.
const PREPARED_DIRS: &[&str] = &["img", "sounds", "script", "rf", "sf"];

/// Nom d'un dossier de chantier de `prepare::build_in_staging` :
/// `.partiel-<uuid>-<aléa>` ou `.partiel-ancien-<uuid>-<aléa>` (préparation écartée), l'aléa étant
/// 32 chiffres hexadécimaux minuscules.
fn is_staging_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix(crate::device::prepare::STAGING_PREFIX) else {
        return false;
    };
    let rest = rest.strip_prefix("ancien-").unwrap_or(rest);
    let Some((uuid, random)) = rest.split_at_checked(36) else {
        return false;
    };
    is_story_uuid(uuid)
        && random.strip_prefix('-').is_some_and(|random| {
            random.len() == 32
                && random
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        })
}

/// `flam-transfer/` ([`crate::device::transfer::WORK_SUBDIR`]) : chaque élément est un dossier
/// réel (ou un fichier de l'explorateur, ignoré sans valoir preuve) parmi :
/// - `str/` ([`crate::device::prepare::STORIES_DIR`]), dont chaque enfant est un dossier réel
///   nommé par un UUID d'histoire ([`is_story_uuid`]) qui est vide, contient le fichier `key`
///   (écrit par `prepare` dans la préparation, publiée en `str/<uuid>/` une fois construite),
///   ou est le reste d'une suppression interrompue : tous ses éléments de premier niveau portent
///   un nom de [`PREPARED_FILES`] (fichier) ou de [`PREPARED_DIRS`] (dossier réel), `key` et
///   `info` ayant pu disparaître avant `rf/` et `sf/` ;
/// - un chantier de préparation ([`is_staging_name`]), dont le contenu n'est pas examiné (il est
///   partiel par nature).
///
/// Tout autre nom, un fichier, un lien ou une jonction : refus. Un dossier personnel qui
/// imiterait ces noms exacts serait reconnu (limite connue, comme pour `podcasts/`).
fn transfer_children_known(dir: &Path) -> bool {
    use crate::device::prepare::{KEY_FILE, STORIES_DIR};
    let story = |path: &Path| {
        let Ok(entries) = fs::read_dir(path) else {
            return false;
        };
        let mut names = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else {
                return false;
            };
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                return false;
            };
            if is_os_junk(&name) && entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            names.push((name, entry.file_type()));
        }
        // Reste d'une suppression interrompue : liste fermée, noms exacts, type exact ; un seul
        // nom inconnu (ou un lien) refuse tout.
        let known = |name: &str, kind: &io::Result<fs::FileType>| {
            kind.as_ref().is_ok_and(|k| {
                (k.is_file() && PREPARED_FILES.contains(&name))
                    || (k.is_dir() && PREPARED_DIRS.contains(&name))
            })
        };
        names.is_empty()
            || names
                .iter()
                .any(|(name, kind)| name == KEY_FILE && kind.as_ref().is_ok_and(|k| k.is_file()))
            || names.iter().all(|(name, kind)| known(name, kind))
    };
    let stories = |path: &Path| {
        entries_known(path, &|_| false, &|name, child| {
            is_story_uuid(name) && story(child)
        })
    };
    entries_known(dir, &|_| false, &|name, path| {
        (name == STORIES_DIR && stories(path)) || is_staging_name(name)
    })
}

/// Tous les éléments du dossier d'une source podcast (`podcasts/<source_id>/`) ont une forme que
/// l'application y écrit (vide compris), relevée dans le code des versions publiées bêta.2 à
/// bêta.4 (aucun cache de podcast en bêta.1) et dans la version courante : flux `feed.xml` et
/// métadonnées `meta.json` (`podcast::source`), leurs temporaires d'écriture atomique
/// `.feed.xml.<pid>-<n>.tmp` et `.meta.json.<pid>-<n>.tmp` (`storage::write_atomic`), dossiers
/// `mp3` (`podcast::mp3cache`) et `thumbs` (`podcast::thumb`, depuis la bêta.3). Un autre nom, un
/// lien ou un type inattendu : refus (#591). Le contenu de `mp3/` et `thumbs/` n'est pas examiné.
/// Exception : `desktop.ini`, `Thumbs.db` (casse indifférente) et `.DS_Store`, s'ils sont des
/// fichiers ordinaires, sont ignorés ; ils ne prouvent rien (un dossier de source qui ne contient
/// qu'eux est jugé comme un dossier vide).
fn podcast_source_children_known(dir: &Path) -> bool {
    const FILES: [&str; 2] = ["feed.xml", "meta.json"];
    const DIRS: [&str; 2] = ["mp3", "thumbs"];
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
            return DIRS.contains(&name.as_str());
        }
        // Fichiers déposés par l'explorateur (Windows, macOS) : ignorés, sans valoir preuve.
        if kind.is_file() && is_os_junk(&name) {
            return true;
        }
        kind.is_file()
            && (FILES.contains(&name.as_str())
                || FILES.iter().any(|file| {
                    name.strip_prefix('.')
                        .and_then(|rest| rest.strip_prefix(file))
                        .is_some_and(|rest| rest.starts_with('.') && rest.ends_with(".tmp"))
                }))
    })
}

/// Fichier que l'explorateur de Windows (`desktop.ini`, `Thumbs.db`, casse indifférente) ou le
/// Finder de macOS (`.DS_Store`) dépose dans un dossier ouvert.
fn is_os_junk(name: &str) -> bool {
    name.eq_ignore_ascii_case("desktop.ini")
        || name.eq_ignore_ascii_case("Thumbs.db")
        || name == ".DS_Store"
}

/// Tous les éléments de `library/` ont une forme que l'application écrit à sa racine (vide
/// compris) : verrou `.lock` (fichier), `.staging` et `thumbs` (dossiers), index `library.json`,
/// index mis de côté `library.json.<horodatage>[-n].bak`, temporaire d'écriture atomique de
/// l'index `.library.json.<pid>-<n>.tmp`, histoires `*.plain.pk` et `*.lunii.pk` (fichiers). Un
/// seul autre nom, un lien ou un type inattendu : refus. Une bibliothèque ouverte sans rien
/// installer (`.lock` seul) est ainsi reconnue (#591). Les fichiers de l'explorateur
/// ([`is_os_junk`], fichiers ordinaires seulement) sont ignorés, sans valoir preuve.
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
        // Fichiers déposés par l'explorateur (Windows, macOS) : ignorés, sans valoir preuve.
        if kind.is_file() && is_os_junk(&name) {
            return true;
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
/// effectif, plus le dossier par défaut s'il existe. Témoin présent : les réglages ne sont pas
/// lus, seul le rattrapage ([`catch_up`]) est fait ; il ne lit le cache que pour les
/// sous-dossiers en attente (témoin vide : aucun accès au cache, #591). Quand le dossier résolu n'est pas
/// celui des réglages (valeur configurée refusée, repli sur le défaut), la migration reste
/// reportée (`Deferred`, sans témoin) pour le dossier configuré, mais le dossier par défaut, alors
/// en usage, est migré quand même (#591). Ses refus ne sont pas inscrits (il n'y a pas encore de
/// témoin) : il est réévalué à chaque démarrage tant que la migration est reportée, et ses refus
/// sont inscrits par la migration complète. Limite : si `settings_set` écrit le témoin (vide)
/// pendant ce report, les refus du dossier par défaut ne sont jamais rattrapés.
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
        let outcome = catch_up(&witness, &|| current_roots(config_dir, env), fault);
        let late = migrate_late(config_dir, env, fault);
        return Ok(match (outcome, late) {
            (outcome, 0) => outcome,
            (Migration::CaughtUp { marked }, late) => Migration::CaughtUp {
                marked: marked + late,
            },
            (_, late) => Migration::CaughtUp { marked: late },
        });
    }
    let settings = SettingsStore::new(config_dir).load()?;
    let (resolved, _) = resolve(&settings, env);
    let effective = PathBuf::from(resolved.cache_dir);
    if let Some(configured) = &settings.cache_dir {
        if !crate::settings::same_path(Path::new(configured), &effective) {
            if env.default_cache_dir.is_dir() {
                mark_existing(&env.default_cache_dir, Marking::Migration, fault)?;
            }
            return Ok(Migration::Deferred);
        }
    }
    let mut roots = vec![effective.clone()];
    // Le dossier par défaut (données de l'application) sert aussi s'il est repris plus tard.
    if env.default_cache_dir != effective && env.default_cache_dir.is_dir() {
        roots.push(env.default_cache_dir.clone());
    }
    let outcome = migrate_markers_with(&witness, &roots, fault)?;
    if matches!(outcome, Migration::Done { .. }) {
        // La migration complète a évalué tous les sous-dossiers, [`LATE_SUBDIRS`] compris.
        write_late_witness(config_dir);
    }
    Ok(outcome)
}

/// Écrit le témoin de la migration tardive ; un échec est journalisé, la migration tardive sera
/// rejouée au démarrage suivant (elle est idempotente).
fn write_late_witness(config_dir: &Path) {
    let written = fs::create_dir_all(config_dir)
        .and_then(|()| crate::storage::write_atomic(&config_dir.join(LATE_WITNESS), b""));
    if let Err(err) = written {
        tracing::warn!("cache : témoin de la migration tardive non écrit : {err}");
    }
}

/// Migration tardive (#591), une seule fois pour un cache déjà migré avant l'ajout de
/// [`LATE_SUBDIRS`] à [`APP_CACHE_SUBDIRS`] (la migration ne repasse plus : témoin présent) :
/// ces sous-dossiers, présents sans marqueur dans le dossier de cache effectif ou dans le dossier
/// par défaut, reçoivent le marqueur seulement si leur contenu le prouve ([`proven_by_content`]).
/// Un refus est inscrit dans le témoin de migration et réévalué par [`catch_up`] à chaque
/// démarrage. Les réglages sont lus sans effet de bord ; illisibles, ou dossier de cache effectif
/// absent (disque débranché) : rien n'est fait et le témoin tardif n'est pas écrit (reporté au
/// démarrage suivant). Une erreur d'écriture reporte aussi. Porte de démarrage fermée, comme le
/// rattrapage. Renvoie le nombre de sous-dossiers marqués.
fn migrate_late(config_dir: &Path, env: &Environment, fault: Fault) -> usize {
    let late_witness = config_dir.join(LATE_WITNESS);
    if fs::symlink_metadata(&late_witness).is_ok() {
        return 0;
    }
    let Some(current) = current_roots(config_dir, env) else {
        return 0;
    };
    if !current
        .eval
        .first()
        .is_some_and(|effective| effective.is_dir())
    {
        return 0;
    }
    let mut roots: Vec<&PathBuf> = Vec::new();
    for root in &current.eval {
        if root.is_dir() && !roots.iter().any(|r| crate::settings::same_path(r, root)) {
            roots.push(root);
        }
    }
    let mut marked = 0;
    let mut refused = Vec::new();
    for root in roots {
        for name in LATE_SUBDIRS.iter().copied() {
            match mark_one(root, name, Marking::Migration, fault) {
                Ok(MarkOutcome::Marked) => marked += 1,
                Ok(MarkOutcome::Unproven) => match root.to_str() {
                    Some(text) => refused.push(Pending {
                        root: text.to_string(),
                        name: name.to_string(),
                    }),
                    None => tracing::warn!(
                        "cache : racine au nom non UTF-8, « {name} » non prouvé jamais réévalué"
                    ),
                },
                Ok(MarkOutcome::Skipped) => {}
                Err(err) => {
                    tracing::warn!("cache : marqueur de « {name} » non écrit (tardif) : {err}");
                    return marked;
                }
            }
        }
    }
    // Dossier configuré mais non retenu (il peut redevenir le cache effectif) : inscrit sans être
    // évalué, le rattrapage le garde tant qu'il n'est ni effectif ni quitté.
    for root in &current.keep {
        for name in LATE_SUBDIRS {
            if let Some(text) = root.to_str() {
                refused.push(Pending {
                    root: text.to_string(),
                    name: name.to_string(),
                });
            }
        }
    }
    if !refused.is_empty() {
        let witness = config_dir.join(MIGRATION_WITNESS);
        let mut pending = read_pending(&witness);
        for entry in refused {
            if !pending.contains(&entry) {
                pending.push(entry);
            }
        }
        if let Err(err) = write_witness(&witness, &pending) {
            tracing::warn!("cache : liste des sous-dossiers en attente non écrite : {err}");
            return marked;
        }
    }
    write_late_witness(config_dir);
    marked
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
/// adopté et `true` est renvoyé : `library/` reçoit le marqueur (s'il ne l'a pas déjà), **puis**
/// les autres sous-dossiers de [`APP_CACHE_SUBDIRS`] présents dont le contenu prouve l'usage par
/// l'application reçoivent le leur ([`mark_existing`] avec la preuve [`proven_at_adoption`] :
/// `podcasts`, `podcast-work`, `play`, `flam-transfer` et `thumbs` seulement), sans quoi ils ne seraient plus
/// jamais marqués (#591) : la migration ne repasse plus et [`ensure_subdir`] ne marque pas un
/// dossier existant. `archives`, `catalogs`, `compat-local` et `official` ne sont jamais marqués
/// ici (fuite de place, jamais de perte). Un homonyme refusé
/// (contenu non reconnu, fichier, lien, marqueur anormal) n'est ni marqué ni bloquant :
/// l'application l'utilise comme avant, sans jamais le compter ni le vider. Une erreur
/// d'écriture du marqueur de `library/` est renvoyée ; celle d'un autre sous-dossier est
/// journalisée (nom du sous-dossier) et l'adoption continue. Sinon rien n'est écrit, `false`.
pub fn adopt_legacy_cache(root: &Path) -> io::Result<bool> {
    adopt_legacy_cache_with(root, &no_fault)
}

fn adopt_legacy_cache_with(root: &Path, fault: Fault) -> io::Result<bool> {
    let library = root.join(crate::library::LIBRARY_DIR);
    if !is_real_dir(&library) || !crate::library::is_legacy_library(&library) {
        return Ok(false);
    }
    if !is_owned(&library) {
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
    }
    // Bibliothèque reconnue et marquée : les autres sous-dossiers, sur preuve par le contenu.
    mark_existing(root, Marking::Adoption, fault)?;
    Ok(true)
}

/// Sous-dossiers de [`LATE_SUBDIRS`] de `root` (nouveau dossier de cache choisi, hors adoption
/// d'un ancien cache, [`adopt_legacy_cache`]) qui seraient marqués : non marqués, **non vides**,
/// dont le contenu prouve l'usage par l'application ([`proven_at_adoption`]), **et** `root` porte
/// une autre trace de l'application ([`has_app_trace`], un ancien cache de l'ère des marqueurs a
/// ses autres sous-dossiers marqués). Sans cela, un ancien cache quitté puis repris (dont
/// `library.json` dépasse la taille lisible, donc sans adoption) serait refusé pour ce seul
/// sous-dossier, alors que ses restes sont ceux de l'application. Un dossier vide n'est pas
/// retenu : il peut être un dossier de l'utilisateur, le refus d'un homonyme reste la règle. Ne
/// modifie rien : l'appelant décide du marquage ([`adopt_late_subdirs`]) une fois le dossier
/// accepté.
pub fn late_adoptable(root: &Path) -> Vec<&'static str> {
    if !has_app_trace(root) {
        return Vec::new();
    }
    LATE_SUBDIRS
        .iter()
        .copied()
        .filter(|name| {
            let dir = root.join(name);
            is_real_dir(&dir)
                && !is_owned(&dir)
                && fs::symlink_metadata(dir.join(MARKER)).is_err()
                && fs::read_dir(&dir).is_ok_and(|entries| {
                    entries
                        .flatten()
                        .any(|entry| !entry.file_name().to_str().is_some_and(is_os_junk))
                })
                && proven_at_adoption(name, &dir)
        })
        .collect()
}

/// Marque `names` (résultat de [`late_adoptable`]), seulement une fois le dossier accepté. Une
/// erreur d'écriture est journalisée, le sous-dossier reste non marqué (utilisé comme avant).
pub fn adopt_late_subdirs(root: &Path, names: &[&'static str]) {
    for name in names.iter().copied() {
        if let Err(err) = mark_one(root, name, Marking::Adoption, &no_fault) {
            tracing::warn!("cache : marqueur de « {name} » non écrit à l'adoption : {err}");
        }
    }
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

    /// Identifiant d'histoire et nom de chantier de la forme écrite par `prepare`.
    const STORY: &str = "0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e";
    const STAGING: &str =
        ".partiel-0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e-0123456789abcdef0123456789abcdef";

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
        write(&cache.path().join("catalogs/src/catalog.json"), 1);
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
            Migration::Done { marked: 1 }
        );
        assert!(is_owned(&cache.path().join("library")));
        // `archives/a.zip` : nom que le launcher n'écrit jamais, non prouvé.
        assert!(!is_owned(&cache.path().join("archives")));
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
        let files = [
            ("archives", "abc-1.zip.meta.json"),
            ("catalogs", "src/catalog.json"),
            ("official", "catalog.json"),
        ];
        for (name, file) in files {
            write(&cache.path().join(name).join(file), 1);
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
        for (name, file) in files {
            assert!(is_owned(&cache.path().join(name)));
            assert!(
                cache.path().join(name).join(file).is_file(),
                "rien supprimé"
            );
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
    fn un_ancien_cache_est_adopte_sans_marquer_les_noms_sans_preuve() {
        let dir = tempfile::tempdir().unwrap();
        // `archives/a.zip` : nom que le launcher n'écrit jamais, ici dossier personnel.
        legacy_cache(dir.path());
        // Adoption interrompue avant le marqueur de `library/` : erreur, rien d'écrit.
        assert!(adopt_legacy_cache_with(dir.path(), &fail_at("marker", 1)).is_err());
        assert!(!is_owned(&dir.path().join("library")));
        assert!(adopt_legacy_cache(dir.path()).unwrap());
        assert!(adopt_legacy_cache(dir.path()).unwrap(), "idempotente");
        assert!(is_owned(&dir.path().join("library")));
        assert!(
            !is_owned(&dir.path().join("archives")),
            "aucune preuve par le contenu : jamais marqué à l'adoption"
        );
        assert_eq!(foreign_entries(dir.path()), ["archives"]);
        clear_of(dir.path()).unwrap();
        assert!(dir.path().join("archives/a.zip").is_file(), "jamais vidé");
        assert_eq!(names_in(&dir.path().join("library")), [MARKER]);
    }

    /// Ancien cache complet : bibliothèque reconnue (avec un `archives/` personnel, voir
    /// [`legacy_cache`]), `play/`, `podcast-work/`, `podcasts/` et `thumbs/` au contenu de la forme
    /// écrite par l'application, et `catalogs/`, `compat-local/`, `official/` au contenu
    /// quelconque (ces noms ne sont jamais marqués à l'adoption).
    fn complete_legacy_cache(root: &Path) {
        legacy_cache(root);
        write(&root.join("catalogs/perso.txt"), 4);
        write(&root.join("compat-local/perso.txt"), 2);
        write(&root.join("official/perso.txt"), 2);
        write(&root.join("play/0123456789abcdef/story/x.bin"), 3);
        fs::create_dir_all(root.join("podcast-work/0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e")).unwrap();
        write(&root.join("podcasts/source-a/feed.xml"), 6);
        write(&root.join("podcasts/source-a/mp3/0a.mp3"), 7);
        write(&root.join("thumbs/0000000000000001.png"), 8);
        write(&root.join("thumbs/0000000000000002.tmp-42"), 1);
    }

    /// Noms des sous-dossiers marqués dans `root`, dans l'ordre de [`APP_CACHE_SUBDIRS`].
    fn owned_names(root: &Path) -> Vec<&'static str> {
        APP_CACHE_SUBDIRS
            .iter()
            .copied()
            .filter(|name| is_owned(&root.join(name)))
            .collect()
    }

    const NEVER_ADOPTED: [&str; 4] = ["archives", "catalogs", "compat-local", "official"];

    #[test]
    fn l_adoption_d_un_ancien_cache_complet_ne_marque_que_les_contenus_prouves() {
        let dir = tempfile::tempdir().unwrap();
        complete_legacy_cache(dir.path());
        assert!(adopt_legacy_cache(dir.path()).unwrap());
        let adopted = ["library", "play", "podcast-work", "podcasts", "thumbs"];
        assert_eq!(owned_names(dir.path()), adopted);
        assert_eq!(foreign_entries(dir.path()), NEVER_ADOPTED);
        // Comptés et vidés : les seuls sous-dossiers marqués.
        assert_eq!(usage_report(dir.path()).unwrap().1, adopted);
        assert_eq!(
            crate::podcast::mp3size::mp3_cache_bytes(dir.path()).unwrap(),
            7
        );
        clear_of(dir.path()).unwrap();
        for name in adopted {
            assert_eq!(names_in(&dir.path().join(name)), [MARKER], "{name}");
        }
        for path in [
            "archives/a.zip",
            "catalogs/perso.txt",
            "compat-local/perso.txt",
            "official/perso.txt",
        ] {
            assert!(dir.path().join(path).is_file(), "{path} supprimé");
        }

        // La migration de démarrage exige aussi la preuve par le contenu : les contenus
        // personnels de `archives`, `catalogs`, `compat-local` et `official` restent non marqués.
        let migrated = tempfile::tempdir().unwrap();
        let config = tempfile::tempdir().unwrap();
        complete_legacy_cache(migrated.path());
        migrate_markers(
            &config.path().join(MIGRATION_WITNESS),
            &[migrated.path().to_path_buf()],
        )
        .unwrap();
        assert_eq!(owned_names(migrated.path()), adopted);
    }

    /// Cache de la forme réelle écrite par les versions publiées et la version courante, pour
    /// chacun des sous-dossiers de [`APP_CACHE_SUBDIRS`], restes d'écriture interrompue compris.
    fn real_shape_cache(root: &Path) {
        // `library` : index et histoire installée.
        fs::create_dir_all(root.join("library")).unwrap();
        fs::write(
            root.join("library/library.json"),
            r#"{"schema_version":1,"stories":[]}"#,
        )
        .unwrap();
        write(&root.join("library/abc-1.plain.pk"), 10);
        // `archives` : archive complète et sa mémoire, téléchargement en cours et sa mémoire,
        // mémoire seule (suppression interrompue) et temporaire d'écriture d'une mémoire.
        let a = root.join("archives");
        write(&a.join("4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c-3.zip"), 5);
        write(
            &a.join("4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c-3.zip.meta.json"),
            2,
        );
        write(&a.join("abc-0.plain.pk.part"), 5);
        write(&a.join("abc-0.plain.pk.meta.json"), 2);
        write(&a.join("def-4294967295.zip.meta.json"), 2);
        write(&a.join(".ghi-2.zip.meta.json.1234-0.tmp"), 2);
        // `catalogs` : un dossier par source, catalogue, mémoire, temporaires, vignettes.
        let s = root.join("catalogs/0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e");
        write(&s.join("catalog.json"), 3);
        write(&s.join("meta.json"), 3);
        write(&s.join(".catalog.json.42-0.tmp"), 3);
        write(&s.join(".meta.json.42-1.tmp"), 3);
        write(&s.join("thumbs/0123456789abcdef0123456789abcdef.img"), 3);
        write(
            &s.join("thumbs/0123456789abcdef0123456789abcdef.meta.json"),
            3,
        );
        write(
            &s.join("thumbs/.fedcba9876543210fedcba9876543210.img.42-2.tmp"),
            3,
        );
        write(
            &s.join("thumbs/.fedcba9876543210fedcba9876543210.meta.json.42-3.tmp"),
            3,
        );
        fs::create_dir_all(root.join("catalogs/Source_2")).unwrap();
        // `compat-local`, `official` : fichier unique et son temporaire.
        write(&root.join("compat-local/launcher_results.json"), 2);
        write(&root.join("compat-local/.launcher_results.json.7-0.tmp"), 2);
        write(&root.join("official/catalog.json"), 2);
        write(&root.join("official/.catalog.json.7-1.tmp"), 2);
        // `play`, `podcast-work`, `podcasts`, `thumbs`.
        write(&root.join("play/0123456789abcdef/story/x.bin"), 3);
        fs::create_dir_all(root.join("podcast-work/0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e")).unwrap();
        write(&root.join("podcasts/source-a/feed.xml"), 6);
        write(&root.join("podcasts/source-a/mp3/0a.mp3"), 7);
        write(&root.join("thumbs/0000000000000001.png"), 8);
        write(&root.join("thumbs/0000000000000002.tmp-42"), 1);
        // `flam-transfer` : préparation terminée (avec `key`) et chantier interrompu.
        write(&root.join(format!("flam-transfer/str/{STORY}/key")), 16);
        write(&root.join(format!("flam-transfer/str/{STORY}/info")), 5);
        write(&root.join(format!("flam-transfer/{STAGING}/info")), 5);
    }

    /// Migre `root` (seule racine) avec un nouveau témoin et renvoie le résultat.
    fn migrate_once(root: &Path) -> Migration {
        let config = tempfile::tempdir().unwrap();
        migrate_markers(
            &config.path().join(MIGRATION_WITNESS),
            &[root.to_path_buf()],
        )
        .unwrap()
    }

    #[test]
    fn la_migration_marque_tous_les_sous_dossiers_de_forme_reelle() {
        let dir = tempfile::tempdir().unwrap();
        real_shape_cache(dir.path());
        assert_eq!(
            migrate_once(dir.path()),
            Migration::Done {
                marked: APP_CACHE_SUBDIRS.len()
            }
        );
        assert_eq!(owned_names(dir.path()), APP_CACHE_SUBDIRS);
        // Idempotente : une seconde migration (témoin absent) ne change rien.
        assert_eq!(migrate_once(dir.path()), Migration::Done { marked: 0 });
        assert_eq!(owned_names(dir.path()), APP_CACHE_SUBDIRS);
        clear_of(dir.path()).unwrap();
        for name in APP_CACHE_SUBDIRS {
            assert_eq!(names_in(&dir.path().join(name)), [MARKER], "{name}");
        }
    }

    /// Contenu personnel dans un sous-dossier de forme réelle : ce sous-dossier n'est pas marqué
    /// (ni compté ni vidé), la migration réussit et marque tous les autres (#591).
    #[test]
    fn un_contenu_personnel_empeche_le_marquage_de_son_seul_sous_dossier() {
        for (name, personal) in [
            ("archives", "perso.zip"),
            ("archives", "vacances-2019.zip"),
            ("archives", "vacances-2019.zip.part"),
            ("archives", "notes.txt"),
            ("catalogs", "notes.txt"),
            ("catalogs", "0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e/notes.txt"),
            (
                "catalogs",
                "0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4e/thumbs/photo.png",
            ),
            ("catalogs", "Mes catalogues/catalog.json"),
            ("compat-local", "x.json"),
            ("official", "photo.jpg"),
            ("thumbs", "photo.png"),
        ] {
            let dir = tempfile::tempdir().unwrap();
            real_shape_cache(dir.path());
            write(&dir.path().join(name).join(personal), 4);
            let expected: Vec<&str> = APP_CACHE_SUBDIRS
                .iter()
                .copied()
                .filter(|n| *n != name)
                .collect();
            assert_eq!(
                migrate_once(dir.path()),
                Migration::Done {
                    marked: expected.len()
                },
                "{name}/{personal}"
            );
            assert_eq!(owned_names(dir.path()), expected, "{name}/{personal}");
            // Idempotente.
            assert_eq!(migrate_once(dir.path()), Migration::Done { marked: 0 });
            assert_eq!(owned_names(dir.path()), expected, "{name}/{personal}");
            assert!(!usage_report(dir.path())
                .unwrap()
                .1
                .contains(&name.to_string()));
            clear_of(dir.path()).unwrap();
            assert!(
                dir.path().join(name).join(personal).is_file(),
                "{name}/{personal} supprimé"
            );
        }
    }

    /// Migre un cache où seul `subdir` existe, rempli par `setup`, et dit s'il est marqué.
    fn marked_after_migration(subdir: &str, setup: &dyn Fn(&Path)) -> bool {
        let cache = tempfile::tempdir().unwrap();
        let dir = cache.path().join(subdir);
        fs::create_dir_all(&dir).unwrap();
        setup(&dir);
        migrate_once(cache.path());
        is_owned(&dir)
    }

    #[test]
    fn archives_n_est_marque_que_si_chaque_archive_a_sa_memoire() {
        let marked = |files: &[&str]| {
            marked_after_migration("archives", &|d| {
                for file in files {
                    write(&d.join(file), 2);
                }
            })
        };
        assert!(marked(&[]), "vide");
        assert!(marked(&["abc-1.zip", "abc-1.zip.meta.json"]));
        assert!(marked(&["abc-1.zip.part", "abc-1.zip.meta.json"]));
        assert!(marked(&["abc-1.plain.pk", "abc-1.plain.pk.meta.json"]));
        assert!(marked(&[
            "abc-1.zip",
            "abc-1.zip.part",
            "abc-1.zip.meta.json"
        ]));
        assert!(marked(&["abc-0.zip.meta.json"]), "mémoire seule");
        assert!(marked(&[".abc-1.zip.meta.json.42-0.tmp"]));
        assert!(marked(&["a.b_c-1.zip", "a.b_c-1.zip.meta.json"]));
        for refused in [
            &["abc-1.zip"][..],
            &["abc-1.zip.part"],
            &["vacances-2019.zip"],
            &["abc-1.zip", "abc-2.zip.meta.json"],
            &["abc-1.zip.part", "abc-1.plain.pk.meta.json"],
            // Casse exacte.
            &["abc-1.ZIP", "abc-1.ZIP.meta.json"],
            &["abc-1.zip", "abc-1.zip.META.JSON"],
            &["abc-1.Plain.pk", "abc-1.Plain.pk.meta.json"],
            &["abc-1.zip.PART", "abc-1.zip.meta.json"],
            // Version non canonique, absente, trop grande ; identifiant refusé.
            &["abc-01.zip", "abc-01.zip.meta.json"],
            &["abc.zip", "abc.zip.meta.json"],
            &["abc-.zip.meta.json"],
            &["abc-4294967296.zip.meta.json"],
            &["-1.zip.meta.json"],
            &["con-1.zip.meta.json"],
            &["abc-1.rar", "abc-1.rar.meta.json"],
            &["abc-1.zip.meta.json.bak"],
            &[".abc-1.zip.42-0.tmp"],
        ] {
            assert!(!marked(refused), "{refused:?}");
        }
        // Dossier nommé comme un fichier attendu : refusé, même accompagné.
        assert!(!marked_after_migration("archives", &|d| {
            fs::create_dir_all(d.join("abc-1.zip")).unwrap();
            write(&d.join("abc-1.zip.meta.json"), 2);
        }));
        assert!(!marked_after_migration("archives", &|d| {
            write(&d.join("abc-1.zip"), 2);
            fs::create_dir_all(d.join("abc-1.zip.meta.json")).unwrap();
        }));
    }

    #[test]
    fn catalogs_n_est_marque_que_s_il_a_la_forme_d_un_cache_de_catalogue() {
        let marked = |files: &[&str]| {
            marked_after_migration("catalogs", &|d| {
                for file in files {
                    write(&d.join(file), 2);
                }
            })
        };
        assert!(marked(&[]), "vide");
        assert!(marked(&["src/catalog.json", "src/meta.json"]));
        assert!(marked(&["src/.meta.json.1-2.tmp"]));
        assert!(marked(&["src/thumbs/0123456789abcdef0123456789abcdef.img"]));
        assert!(marked(&[
            "src/thumbs/0123456789abcdef0123456789abcdef.meta.json"
        ]));
        assert!(marked(&[
            "src/thumbs/.0123456789abcdef0123456789abcdef.img.1-2.tmp"
        ]));
        assert!(marked(&["src/desktop.ini", "src/thumbs/Thumbs.db"]));
        assert!(marked_after_migration("catalogs", &|d| {
            fs::create_dir_all(d.join("src/thumbs")).unwrap();
        }));
        for refused in [
            &["catalog.json"][..],
            &["src/notes.txt"],
            &["src/Catalog.json"],
            &["src/catalog.JSON"],
            &["src/catalog.json.bak"],
            &["src/.catalog.json.tmp"],
            &["src/sous/catalog.json"],
            &["src.x/catalog.json"],
            &["src/thumbs/photo.png"],
            &["src/thumbs/0123456789abcdef0123456789abcdef.IMG"],
            &["src/thumbs/0123456789ABCDEF0123456789ABCDEF.img"],
            &["src/thumbs/0123456789abcdef0123456789abcde.img"],
            &["src/thumbs/sous/0123456789abcdef0123456789abcdef.img"],
            &["src/Thumbs/0123456789abcdef0123456789abcdef.img"],
        ] {
            assert!(!marked(refused), "{refused:?}");
        }
        assert!(!marked_after_migration("catalogs", &|d| {
            fs::create_dir_all(d.join("src/catalog.json")).unwrap();
        }));
        assert!(!marked_after_migration("catalogs", &|d| {
            write(&d.join("src/thumbs"), 2);
        }));
    }

    #[test]
    fn compat_local_et_official_n_acceptent_que_leur_fichier() {
        for (subdir, file) in [
            ("compat-local", "launcher_results.json"),
            ("official", "catalog.json"),
        ] {
            let marked = |name: &str| marked_after_migration(subdir, &|d| write(&d.join(name), 2));
            assert!(marked_after_migration(subdir, &|_| {}), "{subdir} vide");
            assert!(marked(file), "{subdir}/{file}");
            assert!(marked(&format!(".{file}.12-0.tmp")), "{subdir} temporaire");
            for refused in [
                "x.json",
                "photo.jpg",
                file.to_uppercase().as_str(),
                format!("{file}.bak").as_str(),
                format!(".{file}.tmp").as_str(),
                "meta.json",
            ] {
                assert!(!marked(refused), "{subdir}/{refused}");
            }
            assert!(
                !marked_after_migration(subdir, &|d| fs::create_dir_all(d.join(file)).unwrap()),
                "{subdir}/{file} dossier"
            );
        }
    }

    /// Configuration sans dossier de cache choisi : le cache effectif est `default`.
    fn config_with_default(default: &Path) -> (tempfile::TempDir, Environment) {
        let config = tempfile::tempdir().unwrap();
        fs::write(
            config.path().join("settings.json"),
            r#"{"schema_version":1}"#,
        )
        .unwrap();
        (config, env_with(default, vec![]))
    }

    /// Scénario M1 (#591) : arrêt pendant un premier téléchargement (`.part` sans mémoire),
    /// mise à jour, migration (refus, témoin écrit) ; l'installation reprise vide `archives/`,
    /// le démarrage suivant le marque, « Vider le cache » le vide ensuite.
    #[test]
    fn le_rattrapage_marque_archives_redevenu_prouve() {
        let cache = tempfile::tempdir().unwrap();
        legacy_cache(cache.path());
        fs::remove_file(cache.path().join("archives/a.zip")).unwrap();
        let part = cache.path().join("archives/abc-3.zip.part");
        write(&part, 7);
        let (config, env) = config_with_default(cache.path());
        let witness = config.path().join(MIGRATION_WITNESS);

        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Done { marked: 1 }
        );
        assert!(!is_owned(&cache.path().join("archives")));
        assert_eq!(
            read_pending(&witness),
            [Pending {
                root: cache.path().to_str().unwrap().to_string(),
                name: "archives".into()
            }]
        );
        // Redémarrage sans changement : toujours en attente, rien d'écrit.
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&cache.path().join("archives")));

        // L'installation reprise réussit : archive publiée, `archives/` redevient vide.
        fs::remove_file(&part).unwrap();
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&cache.path().join("archives")));
        assert!(read_pending(&witness).is_empty(), "sorti de la liste");
        assert_eq!(fs::read(&witness).unwrap(), b"", "témoin vide");
        // Idempotent : plus rien en attente.
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        // Désormais compté et vidé.
        write(&cache.path().join("archives/abc-4.zip"), 5);
        write(&cache.path().join("archives/abc-4.zip.meta.json"), 2);
        assert!(usage_report(cache.path())
            .unwrap()
            .1
            .contains(&"archives".to_string()));
        clear_of(cache.path()).unwrap();
        assert_eq!(names_in(&cache.path().join("archives")), [MARKER]);
    }

    #[test]
    fn un_contenu_personnel_n_est_jamais_marque_par_le_rattrapage() {
        let cache = tempfile::tempdir().unwrap();
        write(&cache.path().join("archives/perso.zip"), 4);
        write(&cache.path().join("official/photo.jpg"), 4);
        let (config, env) = config_with_default(cache.path());
        let witness = config.path().join(MIGRATION_WITNESS);
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Done { marked: 0 }
        );
        for _ in 0..5 {
            assert_eq!(
                migrate_configured(config.path(), &env).unwrap(),
                Migration::AlreadyDone
            );
            assert_eq!(owned_names(cache.path()), Vec::<&str>::new());
            assert_eq!(read_pending(&witness).len(), 2, "toujours en attente");
        }
        clear_of(cache.path()).unwrap();
        assert!(cache.path().join("archives/perso.zip").is_file());
        assert!(cache.path().join("official/photo.jpg").is_file());
    }

    /// Un dossier de cache choisi après la mise à jour n'est jamais réévalué par le rattrapage,
    /// même si son `archives/` est vide ou de forme réelle : seule la racine refusée par la
    /// migration l'est.
    #[test]
    fn un_cache_choisi_apres_coup_n_est_jamais_rattrape() {
        let old = tempfile::tempdir().unwrap();
        write(&old.path().join("archives/perso.zip"), 4);
        let (config, env) = config_with_default(old.path());
        let witness = config.path().join(MIGRATION_WITNESS);
        migrate_configured(config.path(), &env).unwrap();
        assert_eq!(read_pending(&witness).len(), 1);

        let shapes: [&dyn Fn(&Path); 2] = [&|_| {}, &|d| {
            write(&d.join("abc-1.zip"), 3);
            write(&d.join("abc-1.zip.meta.json"), 3);
        }];
        for setup in shapes {
            let chosen = tempfile::tempdir().unwrap();
            fs::create_dir_all(chosen.path().join("archives")).unwrap();
            setup(&chosen.path().join("archives"));
            fs::write(
                config.path().join("settings.json"),
                serde_json::json!({ "schema_version": 1, "cache_dir": chosen.path() }).to_string(),
            )
            .unwrap();
            for _ in 0..2 {
                assert_eq!(
                    migrate_configured(config.path(), &env).unwrap(),
                    Migration::AlreadyDone
                );
            }
            assert!(!is_owned(&chosen.path().join("archives")));
            // Témoin vide (écrit par `SettingsStore::update`) : rien en attente non plus.
            let empty = tempfile::tempdir().unwrap();
            fs::write(empty.path().join(MIGRATION_WITNESS), b"").unwrap();
            assert_eq!(
                migrate_markers(
                    &empty.path().join(MIGRATION_WITNESS),
                    &[chosen.path().to_path_buf()]
                )
                .unwrap(),
                Migration::AlreadyDone
            );
            assert!(!is_owned(&chosen.path().join("archives")));
        }
        assert!(!is_owned(&old.path().join("archives")));
    }

    /// Témoin illisible, trop gros ou aux entrées invalides : rien en attente, rien marqué
    /// (même un `archives/` vide, prouvable).
    #[test]
    fn un_temoin_illisible_ne_met_rien_en_attente() {
        let cache = tempfile::tempdir().unwrap();
        fs::create_dir_all(cache.path().join("archives")).unwrap();
        let root = cache.path().to_str().unwrap();
        let entry = |root: &str, name: &str| {
            serde_json::json!({ "pending": [{ "root": root, "name": name }] }).to_string()
        };
        let mut big = entry(root, "archives").into_bytes();
        big.resize(MAX_WITNESS_BYTES as usize + 1, b' ');
        for contents in [
            b"pas du json".to_vec(),
            b"{\"pending\":42}".to_vec(),
            big,
            entry(root, "documents").into_bytes(),
            entry(root, "ARCHIVES").into_bytes(),
            entry("relatif", "archives").into_bytes(),
        ] {
            let config = tempfile::tempdir().unwrap();
            let witness = config.path().join(MIGRATION_WITNESS);
            fs::write(&witness, &contents).unwrap();
            assert_eq!(
                migrate_markers(&witness, &[cache.path().to_path_buf()]).unwrap(),
                Migration::AlreadyDone
            );
            assert!(!is_owned(&cache.path().join("archives")));
            assert_eq!(fs::read(&witness).unwrap(), contents, "témoin non réécrit");
        }
        // Témoin valide : marqué (contrôle du jeu de test).
        let config = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        fs::write(&witness, entry(root, "archives")).unwrap();
        assert_eq!(
            migrate_markers(&witness, &[cache.path().to_path_buf()]).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
    }

    #[test]
    fn pannes_d_ecriture_du_temoin_et_de_la_liste() {
        let cache = tempfile::tempdir().unwrap();
        write(&cache.path().join("archives/abc-1.zip.part"), 3);
        let roots = [cache.path().to_path_buf()];
        let config = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        // Panne à l'écriture du témoin (et de sa liste) : migration non close, rien de marqué
        // sans preuve ; rejouée ensuite.
        assert!(migrate_markers_with(&witness, &roots, &fail_at("witness", 1)).is_err());
        assert!(!witness.exists());
        assert!(!is_owned(&cache.path().join("archives")));
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::Done { marked: 0 }
        );
        assert_eq!(read_pending(&witness).len(), 1);

        fs::remove_file(cache.path().join("archives/abc-1.zip.part")).unwrap();
        // Panne au marqueur pendant le rattrapage : reste en attente, démarrage non bloqué.
        assert_eq!(
            migrate_markers_with(&witness, &roots, &fail_at("marker", 1)).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&cache.path().join("archives")));
        assert_eq!(read_pending(&witness).len(), 1);
        // Panne à la réécriture de la liste : marqué (prouvé), liste inchangée…
        assert_eq!(
            migrate_markers_with(&witness, &roots, &fail_at("pending", 1)).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&cache.path().join("archives")));
        assert_eq!(read_pending(&witness).len(), 1);
        // …puis retirée au démarrage suivant (déjà marqué), sans rien marquer d'autre.
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::AlreadyDone
        );
        assert!(read_pending(&witness).is_empty());
    }

    #[test]
    fn le_rattrapage_garde_les_racines_absentes_et_retire_les_sous_dossiers_disparus() {
        let config = tempfile::tempdir().unwrap();
        let witness = config.path().join(MIGRATION_WITNESS);
        let gone = tempfile::tempdir().unwrap();
        let without = tempfile::tempdir().unwrap();
        let odd = tempfile::tempdir().unwrap();
        for root in [gone.path(), without.path(), odd.path()] {
            write(&root.join("archives/perso.zip"), 1);
        }
        let roots = [
            gone.path().to_path_buf(),
            without.path().to_path_buf(),
            odd.path().to_path_buf(),
        ];
        migrate_markers(&witness, &roots).unwrap();
        assert_eq!(read_pending(&witness).len(), 3);
        // Racine absente (disque débranché) : reste en attente ; sous-dossier supprimé d'une
        // racine présente (recréé marqué par `ensure_subdirs`) : retiré ; contenu prouvé mais
        // marqueur anormal (dossier) : reste en attente, rien d'écrit.
        let gone_path = gone.path().to_path_buf();
        drop(gone);
        fs::remove_dir_all(without.path().join("archives")).unwrap();
        fs::remove_file(odd.path().join("archives/perso.zip")).unwrap();
        fs::create_dir_all(odd.path().join("archives").join(MARKER)).unwrap();
        assert_eq!(
            migrate_markers(&witness, &roots).unwrap(),
            Migration::AlreadyDone
        );
        let left: Vec<String> = read_pending(&witness).into_iter().map(|p| p.root).collect();
        assert_eq!(
            left,
            [gone_path.to_str().unwrap(), odd.path().to_str().unwrap()]
        );
        assert!(!gone_path.exists());
        assert!(!without.path().join("archives").exists());
        assert_eq!(names_in(&odd.path().join("archives")), [MARKER]);
        assert!(odd.path().join("archives").join(MARKER).is_dir());
        ensure_subdirs(without.path());
        assert!(is_owned(&without.path().join("archives")));
    }

    /// Écrit `settings.json` avec `cache_dir` (ou sans, `None`).
    fn set_cache_dir(config: &Path, cache_dir: Option<&Path>) {
        let value = match cache_dir {
            Some(dir) => serde_json::json!({ "schema_version": 1, "cache_dir": dir }),
            None => serde_json::json!({ "schema_version": 1 }),
        };
        fs::write(config.join("settings.json"), value.to_string()).unwrap();
    }

    /// N-m1 (#591) : une entrée d'un dossier de cache quitté n'est jamais réévaluée, elle est
    /// retirée au démarrage suivant ; un retour sur ce dossier passe par l'adoption seule.
    #[test]
    fn le_rattrapage_abandonne_un_dossier_de_cache_quitte() {
        let base = tempfile::tempdir().unwrap();
        let config = base.path().join("config");
        let default = base.path().join("defaut");
        let old = base.path().join("ancien");
        let other = base.path().join("autre");
        for dir in [&config, &default, &other] {
            fs::create_dir_all(dir).unwrap();
        }
        legacy_cache(&old);
        fs::remove_file(old.join("archives/a.zip")).unwrap();
        write(&old.join("archives/perso.zip"), 4);
        let env = env_with(&default, vec![]);
        set_cache_dir(&config, Some(&old));
        let witness = config.join(MIGRATION_WITNESS);
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::Done { marked: 1 }
        );
        assert_eq!(read_pending(&witness).len(), 1);

        // Changement de dossier (pendant la session : aucun rattrapage avant le démarrage
        // suivant), puis `ancien/archives` vidé par l'utilisateur.
        set_cache_dir(&config, Some(&other));
        fs::remove_file(old.join("archives/perso.zip")).unwrap();
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&old.join("archives")), "jamais réévalué");
        assert!(read_pending(&witness).is_empty(), "entrée retirée");

        // Fichiers personnels remis, puis retour sur l'ancien dossier : adoption seule.
        write(&old.join("archives/vacances.zip"), 4);
        let store = SettingsStore::new(&config);
        store
            .update(crate::settings::Settings {
                cache_dir: Some(old.display().to_string()),
                ..crate::settings::Settings::default()
            })
            .unwrap();
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&old.join("archives")));
        clear_of(&old).unwrap();
        assert!(old.join("archives/vacances.zip").is_file());
    }

    /// N-m2 (#591) : racine effective absente pendant un démarrage (disque débranché) : l'entrée
    /// reste, et le sous-dossier est marqué au démarrage de son retour, une fois prouvé.
    #[test]
    fn une_racine_absente_un_demarrage_est_rattrapee_a_son_retour() {
        let base = tempfile::tempdir().unwrap();
        let config = base.path().join("config");
        let default = base.path().join("defaut");
        let disk = base.path().join("disque");
        let away = base.path().join("debranche");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&default).unwrap();
        write(&disk.join("archives/abc-1.zip.part"), 3);
        let env = env_with(&default, vec![]);
        set_cache_dir(&config, Some(&disk));
        let witness = config.join(MIGRATION_WITNESS);
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::Done { marked: 0 }
        );
        fs::rename(&disk, &away).unwrap();
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::AlreadyDone
        );
        assert_eq!(read_pending(&witness).len(), 1, "gardée");
        fs::rename(&away, &disk).unwrap();
        fs::remove_file(disk.join("archives/abc-1.zip.part")).unwrap();
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&disk.join("archives")));
        assert!(read_pending(&witness).is_empty());
    }

    /// Réglages illisibles avec une liste non vide : rien n'est évalué ni retiré, ni le témoin
    /// ni les réglages ne sont réécrits.
    #[test]
    fn des_reglages_illisibles_suspendent_le_rattrapage() {
        let cache = tempfile::tempdir().unwrap();
        write(&cache.path().join("archives/perso.zip"), 4);
        let (config, env) = config_with_default(cache.path());
        let witness = config.path().join(MIGRATION_WITNESS);
        migrate_configured(config.path(), &env).unwrap();
        fs::remove_file(cache.path().join("archives/perso.zip")).unwrap();
        let before = fs::read(&witness).unwrap();
        for corrupt in [&b"{corrompu"[..], br#"{"schema_version":99}"#] {
            fs::write(config.path().join("settings.json"), corrupt).unwrap();
            assert_eq!(
                migrate_configured(config.path(), &env).unwrap(),
                Migration::AlreadyDone
            );
            assert!(!is_owned(&cache.path().join("archives")));
            assert_eq!(fs::read(&witness).unwrap(), before, "liste intacte");
            assert_eq!(
                names_in(config.path()),
                [LATE_WITNESS, MIGRATION_WITNESS, "settings.json"],
                "rien mis de côté"
            );
        }
        // Réglages de nouveau lisibles : rattrapé.
        set_cache_dir(config.path(), None);
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
    }

    /// Une entrée du dossier par défaut est rattrapée même si un autre dossier est configuré
    /// (la migration traite les deux).
    #[test]
    fn le_dossier_par_defaut_est_rattrape_meme_si_un_autre_est_configure() {
        let base = tempfile::tempdir().unwrap();
        let config = base.path().join("config");
        let default = base.path().join("defaut");
        let chosen = base.path().join("choisi");
        fs::create_dir_all(&config).unwrap();
        fs::create_dir_all(&chosen).unwrap();
        write(&default.join("archives/abc-1.zip.part"), 3);
        let env = env_with(&default, vec![]);
        set_cache_dir(&config, Some(&chosen));
        let witness = config.join(MIGRATION_WITNESS);
        migrate_configured(&config, &env).unwrap();
        assert_eq!(
            read_pending(&witness)
                .into_iter()
                .map(|p| p.root)
                .collect::<Vec<_>>(),
            [default.to_str().unwrap()]
        );
        fs::remove_file(default.join("archives/abc-1.zip.part")).unwrap();
        assert_eq!(
            migrate_configured(&config, &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&default.join("archives")));
    }

    /// Garde des constantes (#591) : chaque nom de [`APP_CACHE_SUBDIRS`] a une règle de preuve
    /// (vide : prouvé ; un nom sans règle ne l'est jamais), et les constantes des modules qui
    /// écrivent le cache sont bien des noms de la liste.
    #[test]
    fn chaque_sous_dossier_a_une_regle_de_preuve() {
        let dir = tempfile::tempdir().unwrap();
        for name in APP_CACHE_SUBDIRS {
            assert!(proven_by_content(name, dir.path()), "{name}");
        }
        assert!(!proven_by_content("documents", dir.path()));
        for name in [
            crate::install::ARCHIVES_DIR,
            crate::catalog::CATALOGS_DIR,
            crate::compat::RESULTS_DIR,
            crate::device::official::CACHE_DIR,
            crate::thumbnail::THUMBS_DIR,
            crate::library::LIBRARY_DIR,
            crate::player::PLAY_DIR,
            crate::install::podcast::WORK_DIR,
            crate::podcast::source::CACHE_SUBDIR,
        ] {
            assert!(APP_CACHE_SUBDIRS.contains(&name), "{name}");
        }
    }

    #[test]
    fn thumbs_n_est_marque_a_la_migration_que_s_il_ne_contient_que_des_vignettes() {
        let marked = |name: &str| marked_after_migration("thumbs", &|d| write(&d.join(name), 2));
        assert!(marked_after_migration("thumbs", &|_| {}), "vide");
        assert!(marked("0123456789abcdef.png"));
        assert!(marked("0123456789abcdef.tmp-7"));
        for refused in [
            "photo.png",
            "0123456789abcdef.PNG",
            "0123456789ABCDEF.png",
            "0123456789abcdef.jpg",
        ] {
            assert!(!marked(refused), "{refused}");
        }
        assert!(!marked_after_migration("thumbs", &|d| {
            fs::create_dir_all(d.join("0123456789abcdef.png")).unwrap();
        }));
    }

    #[test]
    fn un_podcasts_non_conforme_n_est_pas_marque_le_reste_est_adopte() {
        let dir = tempfile::tempdir().unwrap();
        complete_legacy_cache(dir.path());
        // MP3 personnel à la racine de `podcasts/` : refusé par la preuve d'usage.
        write(&dir.path().join("podcasts/mon-emission.mp3"), 300);
        // `podcast-work/` personnel : refusé aussi.
        write(&dir.path().join("podcast-work/notes.txt"), 3);
        assert!(adopt_legacy_cache(dir.path()).unwrap());
        assert_eq!(owned_names(dir.path()), ["library", "play", "thumbs"]);
        assert_eq!(
            crate::podcast::mp3size::mp3_cache_bytes(dir.path()).unwrap(),
            0
        );
        clear_of(dir.path()).unwrap();
        assert!(dir.path().join("podcasts/mon-emission.mp3").is_file());
        assert!(dir.path().join("podcasts/source-a/mp3/0a.mp3").is_file());
        assert!(dir.path().join("podcast-work/notes.txt").is_file());
    }

    #[test]
    fn thumbs_n_est_marque_a_l_adoption_que_s_il_ne_contient_que_des_vignettes() {
        let adopted = |setup: &dyn Fn(&Path)| {
            let dir = tempfile::tempdir().unwrap();
            legacy_cache(dir.path());
            fs::create_dir_all(dir.path().join("thumbs")).unwrap();
            setup(&dir.path().join("thumbs"));
            assert!(adopt_legacy_cache(dir.path()).unwrap());
            (is_owned(&dir.path().join("thumbs")), dir)
        };
        assert!(adopted(&|_| {}).0, "vide : accepté, comme podcasts ou play");
        assert!(adopted(&|t| write(&t.join("0123456789abcdef.png"), 3)).0);
        assert!(adopted(&|t| write(&t.join("0123456789abcdef.tmp-7"), 3)).0);
        assert!(
            adopted(&|t| {
                write(&t.join("0123456789abcdef.png"), 3);
                write(&t.join("desktop.ini"), 1);
                write(&t.join("Thumbs.db"), 1);
            })
            .0,
            "fichiers de l'explorateur ignorés"
        );
        assert!(
            adopted(&|t| {
                write(&t.join("desktop.ini"), 1);
                write(&t.join("Thumbs.db"), 1);
            })
            .0,
            "fichiers de l'explorateur seuls : comme un dossier vide"
        );
        for refused in [
            "photo.png",
            "FEDCBA9876543210.png",
            // Radical valide, extension en majuscules : la comparaison est sensible à la casse.
            "fedcba9876543210.PNG",
            "0123456789abcdef.jpg",
            "0123456789abcdef.tmp-",
        ] {
            let (owned, dir) = adopted(&|t| {
                write(&t.join("0123456789abcdef.png"), 3);
                write(&t.join(refused), 5);
            });
            assert!(!owned, "{refused}");
            clear_of(dir.path()).unwrap();
            assert!(
                dir.path().join("thumbs").join(refused).is_file(),
                "{refused}"
            );
        }
        let (owned, _dir) =
            adopted(&|t| fs::create_dir_all(t.join("0123456789abcdef.png")).unwrap());
        assert!(!owned, "dossier nommé comme une vignette");
    }

    #[test]
    fn un_marqueur_impossible_a_ecrire_hors_library_n_empeche_pas_l_adoption() {
        let dir = tempfile::tempdir().unwrap();
        complete_legacy_cache(dir.path());
        // Marqueur n° 1 : `library` ; n° 2 : `play`, premier sous-dossier prouvé.
        assert!(adopt_legacy_cache_with(dir.path(), &fail_at("marker", 2)).unwrap());
        assert_eq!(
            owned_names(dir.path()),
            ["library", "podcast-work", "podcasts", "thumbs"]
        );
        // Un nouvel essai reprend.
        assert!(adopt_legacy_cache(dir.path()).unwrap());
        assert!(is_owned(&dir.path().join("play")));
    }

    #[test]
    fn sans_bibliotheque_reconnue_aucun_sous_dossier_n_est_marque() {
        let dir = tempfile::tempdir().unwrap();
        complete_legacy_cache(dir.path());
        // Index d'un autre schéma : la bibliothèque n'est pas reconnue.
        fs::write(
            dir.path().join("library/library.json"),
            r#"{"schema_version":99,"stories":[]}"#,
        )
        .unwrap();
        assert!(!adopt_legacy_cache(dir.path()).unwrap());
        for name in APP_CACHE_SUBDIRS {
            assert!(!is_owned(&dir.path().join(name)), "{name}");
            assert!(!dir.path().join(name).join(MARKER).exists(), "{name}");
        }
    }

    #[test]
    fn un_sous_dossier_en_lien_ou_jonction_n_est_pas_marque_a_l_adoption() {
        let root = tempfile::tempdir().unwrap();
        let cache = root.path().join("cache");
        complete_legacy_cache(&cache);
        let mut linked = Vec::new();
        for name in ["podcasts", "thumbs"] {
            let target = root.path().join(format!("cible-{name}"));
            fs::rename(cache.join(name), &target).unwrap();
            let link = cache.join(name);
            #[cfg(windows)]
            {
                // Jonction (sans privilège) ; séparateurs Windows pour `mklink`.
                let status = std::process::Command::new("cmd")
                    .args(["/C", "mklink", "/J"])
                    .arg(link.to_string_lossy().replace('/', "\\"))
                    .arg(target.to_string_lossy().replace('/', "\\"))
                    .output()
                    .unwrap();
                assert!(status.status.success(), "mklink /J a échoué");
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, &link).unwrap();
            linked.push((link, target));
        }
        assert!(adopt_legacy_cache(&cache).unwrap());
        assert_eq!(owned_names(&cache), ["library", "play", "podcast-work"]);
        for (link, target) in &linked {
            assert!(!is_owned(link), "{}", link.display());
            assert!(!target.join(MARKER).exists(), "cible jamais marquée");
        }
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
        // `archives/a.zip` : non prouvé, non marqué, sans faire échouer la migration.
        assert!(!is_owned(&default.join("archives")));
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
            Migration::Done { marked: 6 }
        );
        for name in ["library", "podcasts", "podcast-work", "play"] {
            assert!(is_owned(&root.join(name)), "{name}");
        }
        // `archives/a.zip` (nom que le launcher n'écrit jamais) : non prouvé.
        assert!(!is_owned(&root.join("archives")));
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

    /// `podcasts/` : chaque dossier de source doit avoir la forme écrite par les versions publiées
    /// (flux, métadonnées, leurs temporaires, `mp3/`, `thumbs/`) ; un `podcasts/` personnel aux
    /// sous-dossiers bien nommés n'est plus marqué (#591).
    #[test]
    fn un_dossier_de_source_podcast_est_reconnu_a_sa_forme() {
        let marked = |setup: &dyn Fn(&Path)| {
            let config = tempfile::tempdir().unwrap();
            let cache = tempfile::tempdir().unwrap();
            let source = cache
                .path()
                .join("podcasts/4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c");
            fs::create_dir_all(&source).unwrap();
            setup(&source);
            let roots = [cache.path().to_path_buf()];
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap();
            is_owned(&cache.path().join("podcasts"))
        };
        assert!(marked(&|s| {
            write(&s.join("feed.xml"), 3);
            write(&s.join("meta.json"), 3);
            write(&s.join(".feed.xml.42-0.tmp"), 3);
            write(&s.join(".meta.json.42-1.tmp"), 3);
            write(&s.join("mp3/0123456789abcdef0123456789abcdef.mp3"), 3);
            write(&s.join("thumbs/vignette.png"), 3);
        }));
        assert!(
            marked(&|s| fs::create_dir_all(s.join("mp3")).unwrap()),
            "mp3 seul"
        );
        assert!(marked(&|_| {}), "dossier de source vide");
        // Un `podcasts/` personnel aux sous-dossiers bien nommés : refus.
        assert!(!marked(&|s| write(&s.join("episode-12.mp3"), 3)));
        assert!(!marked(&|s| {
            write(&s.join("feed.xml"), 3);
            fs::create_dir_all(s.join("2024")).unwrap();
        }));
        assert!(
            !marked(&|s| fs::create_dir_all(s.join("feed.xml")).unwrap()),
            "flux dossier"
        );
        assert!(!marked(&|s| write(&s.join("mp3"), 3)), "mp3 fichier");
        assert!(!marked(&|s| write(&s.join(".feed.xmlx.tmp"), 3)));
        // Fichiers de l'explorateur : ignorés à côté d'une forme valide, ou seuls (dossier vide).
        for junk in [
            "desktop.ini",
            "DESKTOP.INI",
            "Thumbs.db",
            "thumbs.db",
            ".DS_Store",
        ] {
            assert!(
                marked(&|s| {
                    write(&s.join("feed.xml"), 3);
                    write(&s.join(junk), 3);
                }),
                "{junk}"
            );
            assert!(marked(&|s| write(&s.join(junk), 3)), "{junk} seul");
        }
        // Seulement en fichier ordinaire, et seulement ces noms-là.
        assert!(
            !marked(&|s| {
                write(&s.join("feed.xml"), 3);
                fs::create_dir_all(s.join("desktop.ini")).unwrap();
            }),
            "desktop.ini dossier"
        );
        assert!(!marked(&|s| {
            write(&s.join("feed.xml"), 3);
            write(&s.join("notes.txt"), 3);
        }));
        assert!(!marked(&|s| write(&s.join(".ds_store"), 3)), "casse exacte");
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

    /// `podcasts/` : un lien ou une jonction dans un dossier de source (`mp3`, `thumbs`, ou un nom
    /// de fichier de l'explorateur) empêche le marquage (#591).
    #[cfg(windows)]
    #[test]
    fn une_jonction_dans_un_dossier_de_source_podcast_empeche_le_marquage() {
        let outside = tempfile::tempdir().unwrap();
        for name in ["mp3", "thumbs", "desktop.ini"] {
            let config = tempfile::tempdir().unwrap();
            let cache = tempfile::tempdir().unwrap();
            // Séparateurs Windows : `mklink` lirait `/src` comme une option.
            let source = cache.path().join("podcasts").join("src");
            write(&source.join("feed.xml"), 3);
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(source.join(name))
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(status.status.success(), "mklink /J a échoué");
            let roots = [cache.path().to_path_buf()];
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap();
            assert!(
                !is_owned(&cache.path().join("podcasts")),
                "{name} en jonction"
            );
        }
    }

    /// Fichiers de l'explorateur à la racine d'un sous-dossier soumis à une preuve d'usage
    /// (#591) : ignorés à côté d'un contenu valide, jugés comme un dossier vide quand ils sont
    /// seuls ; refusés en dossier ou en jonction ; un autre nom reste refusé.
    fn root_os_junk_is_ignored(subdir: &str, valid: &dyn Fn(&Path)) {
        let marked = |setup: &dyn Fn(&Path)| {
            let config = tempfile::tempdir().unwrap();
            let cache = tempfile::tempdir().unwrap();
            let dir = cache.path().join(subdir);
            fs::create_dir_all(&dir).unwrap();
            setup(&dir);
            let roots = [cache.path().to_path_buf()];
            migrate_markers(&config.path().join(MIGRATION_WITNESS), &roots).unwrap();
            is_owned(&dir)
        };
        assert!(marked(valid), "{subdir} : contenu valide");
        let empty = marked(&|_| {});
        for junk in [
            "desktop.ini",
            "DESKTOP.INI",
            "Thumbs.db",
            "thumbs.db",
            ".DS_Store",
        ] {
            assert!(
                marked(&|d| {
                    valid(d);
                    write(&d.join(junk), 3);
                }),
                "{subdir}/{junk} à côté d'un contenu valide"
            );
            assert_eq!(
                marked(&|d| write(&d.join(junk), 3)),
                empty,
                "{subdir}/{junk} seul : jugé comme vide"
            );
            assert!(
                !marked(&|d| {
                    valid(d);
                    fs::create_dir_all(d.join(junk)).unwrap();
                }),
                "{subdir}/{junk} dossier"
            );
        }
        assert!(
            !marked(&|d| {
                valid(d);
                write(&d.join("notes.txt"), 3);
            }),
            "{subdir}/notes.txt"
        );
        assert!(
            !marked(&|d| {
                valid(d);
                write(&d.join(".ds_store"), 3);
            }),
            "{subdir}/.ds_store : casse exacte"
        );
        #[cfg(windows)]
        {
            let outside = tempfile::tempdir().unwrap();
            for junk in ["desktop.ini", "Thumbs.db", ".DS_Store"] {
                assert!(
                    !marked(&|d| {
                        valid(d);
                        // Séparateurs Windows : `mklink` lirait `/x` comme une option.
                        let status = std::process::Command::new("cmd")
                            .args(["/C", "mklink", "/J"])
                            .arg(d.join(junk))
                            .arg(outside.path())
                            .output()
                            .unwrap();
                        assert!(status.status.success(), "mklink /J a échoué");
                    }),
                    "{subdir}/{junk} en jonction"
                );
            }
        }
    }

    #[test]
    fn fichiers_de_l_explorateur_a_la_racine_de_library() {
        root_os_junk_is_ignored("library", &|d| write(&d.join(".lock"), 0));
    }

    #[test]
    fn fichiers_de_l_explorateur_a_la_racine_de_podcasts() {
        root_os_junk_is_ignored("podcasts", &|d| write(&d.join("src/feed.xml"), 3));
    }

    #[test]
    fn fichiers_de_l_explorateur_a_la_racine_de_podcast_work() {
        root_os_junk_is_ignored("podcast-work", &|d| {
            write(&d.join("4f1c2a9e-0b7d-4c3e-9a51-2d6e8f0a1b3c/pack.zip"), 3)
        });
    }

    #[test]
    fn fichiers_de_l_explorateur_a_la_racine_de_play() {
        root_os_junk_is_ignored("play", &|d| {
            write(&d.join("0123456789abcdef/story/x.bin"), 3)
        });
    }

    #[test]
    fn fichiers_de_l_explorateur_a_la_racine_des_autres_sous_dossiers() {
        root_os_junk_is_ignored("archives", &|d| {
            write(&d.join("abc-1.zip"), 3);
            write(&d.join("abc-1.zip.meta.json"), 3);
        });
        root_os_junk_is_ignored("catalogs", &|d| write(&d.join("src/catalog.json"), 3));
        root_os_junk_is_ignored("compat-local", &|d| {
            write(&d.join("launcher_results.json"), 3)
        });
        root_os_junk_is_ignored("official", &|d| write(&d.join("catalog.json"), 3));
        root_os_junk_is_ignored("thumbs", &|d| write(&d.join("0123456789abcdef.png"), 3));
    }

    /// `catalogs/` : une jonction à la place d'un dossier de source ou de son `thumbs/`
    /// empêche le marquage, et sa cible n'est jamais marquée (#591).
    #[cfg(windows)]
    #[test]
    fn une_jonction_dans_catalogs_empeche_le_marquage() {
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("catalog.json"), 3);
        for link in ["src", "autre/thumbs"] {
            let cache = tempfile::tempdir().unwrap();
            let catalogs = cache.path().join("catalogs");
            write(&catalogs.join("autre/catalog.json"), 3);
            // Séparateurs Windows : `mklink` lirait `/x` comme une option.
            let path = catalogs.join(link.replace('/', "\\"));
            let status = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(&path)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(status.status.success(), "mklink /J a échoué");
            migrate_once(cache.path());
            assert!(!is_owned(&catalogs), "{link} en jonction");
            assert!(!outside.path().join(MARKER).exists());
        }
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
            Migration::Done { marked: 1 }
        );
        assert!(witness.is_file(), "plus reportée à chaque démarrage");
        assert!(is_owned(&link.join("library")));
        // `library/` seule (index et histoire) : `archives/a.zip` n'est pas prouvé.
        assert_eq!(usage_of(&link).unwrap(), 10 + 33);
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
                "flam-transfer",
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

    // -----------------------------------------------------------------------------------------
    // `flam-transfer` (#591) : travail de préparation d'un envoi vers la Flam
    // -----------------------------------------------------------------------------------------

    /// `root/flam-transfer` de la forme écrite par `prepare` (préparation terminée et chantier
    /// interrompu), sans marqueur : un cache d'avant l'ajout du sous-dossier à la liste.
    fn unmarked_transfer(root: &Path) {
        // Trace d'usage par l'application exigée pour marquer `flam-transfer` ([`has_app_trace`]).
        ensure_subdir(root, "archives").unwrap();
        write(&root.join(format!("flam-transfer/str/{STORY}/key")), 16);
        write(&root.join(format!("flam-transfer/str/{STORY}/info")), 5);
        write(&root.join(format!("flam-transfer/{STAGING}/info")), 5);
    }

    #[test]
    fn flam_transfer_est_cree_marque_compte_et_vide_comme_les_autres() {
        let dir = tempfile::tempdir().unwrap();
        ensure_subdirs(dir.path());
        let work = dir.path().join("flam-transfer");
        assert!(is_owned(&work), "créé marqué");
        write(&work.join(format!("str/{STORY}/key")), 16);
        write(&work.join(format!("str/{STORY}/info")), 4);
        write(&work.join(format!("{STAGING}/info")), 5);
        let (bytes, names) = usage_report(dir.path()).unwrap();
        assert_eq!(bytes, 25);
        assert!(names.contains(&"flam-transfer".to_string()));
        // Un arrêt brutal a laissé ces restes : « Vider le cache » les supprime.
        assert_eq!(clear_of(dir.path()).unwrap(), 25);
        assert_eq!(names_in(&work), [MARKER]);
        assert!(is_owned(&work));
    }

    /// `proven_by_content("flam-transfer", …)` pour un dossier rempli par `fill`.
    fn transfer_proven(fill: impl Fn(&Path)) -> bool {
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("flam-transfer");
        fs::create_dir_all(&work).unwrap();
        fill(&work);
        proven_by_content("flam-transfer", &work)
    }

    #[test]
    fn la_preuve_de_flam_transfer_est_celle_des_noms_ecrits_par_prepare() {
        // Vide, préparation terminée, répertoire de préparation vide (suppression
        // interrompue), chantiers (dont l'ancienne préparation écartée), fichiers de l'explorateur.
        assert!(transfer_proven(|_| {}));
        assert!(transfer_proven(|w| unmarked_transfer(w.parent().unwrap())));
        assert!(transfer_proven(|w| {
            fs::create_dir_all(w.join(format!("str/{STORY}"))).unwrap();
        }));
        assert!(transfer_proven(|w| {
            fs::create_dir_all(w.join(format!(
                ".partiel-ancien-{STORY}-fedcba9876543210fedcba9876543210"
            )))
            .unwrap();
            fs::create_dir_all(w.join("str")).unwrap();
            write(&w.join("desktop.ini"), 1);
            write(&w.join(format!("str/{STORY}/Thumbs.db")), 1);
        }));
        // Doute : refus.
        type Fill<'a> = &'a dyn Fn(&Path);
        let refused: [(&str, Fill); 12] = [
            ("fichier perso", &|w| write(&w.join("notes.txt"), 1)),
            ("dossier perso", &|w| {
                write(&w.join("Mes documents/a.txt"), 1)
            }),
            ("fichier dans str", &|w| write(&w.join("str/notes.txt"), 1)),
            ("nom perso dans str", &|w| {
                write(&w.join("str/vacances/key"), 1)
            }),
            ("histoire sans key", &|w| {
                write(&w.join(format!("str/{STORY}/photo.jpg")), 1);
            }),
            ("uuid en majuscules", &|w| {
                let upper = STORY.to_uppercase();
                write(&w.join(format!("str/{upper}/key")), 1);
            }),
            ("uuid sans tirets", &|w| {
                write(&w.join(format!("str/{}/key", STORY.replace('-', ""))), 1);
            }),
            ("chantier sans aléa", &|w| {
                write(&w.join(format!(".partiel-{STORY}/info")), 1);
            }),
            ("chantier au nom libre", &|w| {
                write(&w.join(".partiel-perso/a"), 1)
            }),
            ("chantier fichier", &|w| {
                write(&w.join(format!("{STAGING}.txt")), 1);
            }),
            ("str est un fichier", &|w| write(&w.join("str"), 1)),
            ("chantier est un fichier", &|w| write(&w.join(STAGING), 1)),
        ];
        for (what, fill) in refused {
            assert!(!transfer_proven(fill), "{what}");
        }
        assert!(is_story_uuid(STORY));
        assert!(!is_story_uuid("0b9f6c1e-2d3a-4b5c-8d7e-9f0a1b2c3d4"));
        assert!(!is_story_uuid("0b9f6c1e_2d3a_4b5c_8d7e_9f0a1b2c3d4e"));
        assert!(is_staging_name(STAGING));
        assert!(!is_staging_name(&format!("{STAGING}0")));
    }

    /// Lien de dossier sans privilège : jonction (`mklink /J`) sous Windows, lien symbolique
    /// ailleurs.
    fn link_dir(link: &Path, target: &Path) {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // Chemins entre guillemets : un dossier temporaire peut contenir des espaces.
            let quoted = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('/', "\\"));
            let out = std::process::Command::new("cmd")
                .arg("/C")
                .raw_arg(format!("\"mklink /J {} {}\"", quoted(link), quoted(target)))
                .output()
                .unwrap();
            assert!(out.status.success(), "mklink /J : {out:?}");
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, link).unwrap();
    }

    #[test]
    fn un_lien_dans_flam_transfer_ne_prouve_rien() {
        let outside = tempfile::tempdir().unwrap();
        write(&outside.path().join("a.txt"), 1);
        // Lien à la place de `str/`, d'une préparation, d'un chantier, d'un dossier de
        // préparation (`img`) : chaque cas est refusé, sans suivre le lien.
        type Place = fn(&Path) -> PathBuf;
        let places: [(&str, Place); 4] = [
            ("str", |w| w.join("str")),
            ("préparation", |w| w.join(format!("str/{STORY}"))),
            ("chantier", |w| w.join(STAGING)),
            ("img", |w| w.join(format!("str/{STORY}/img"))),
        ];
        for (what, place) in places {
            let dir = tempfile::tempdir().unwrap();
            let work = dir.path().join("flam-transfer");
            fs::create_dir_all(work.join("str")).unwrap();
            if what == "img" {
                // Reste partiel (sans `key`) dont `img` est un lien : l'élément n'est pas un
                // dossier réel.
                fs::create_dir_all(work.join(format!("str/{STORY}"))).unwrap();
            } else if what == "str" {
                fs::remove_dir(work.join("str")).unwrap();
            }
            link_dir(&place(&work), outside.path());
            assert!(!proven_by_content("flam-transfer", &work), "{what}");
        }
        assert!(outside.path().join("a.txt").is_file());
    }

    #[test]
    fn un_reste_de_suppression_interrompue_est_prouve_a_noms_exacts() {
        let leftover = |names: &[(&str, bool)]| {
            transfer_proven(|w| {
                for (name, is_dir) in names {
                    let path = w.join(format!("str/{STORY}")).join(name);
                    if *is_dir {
                        fs::create_dir_all(path.join("x")).unwrap();
                    } else {
                        write(&path, 1);
                    }
                }
                fs::create_dir_all(w.join(format!("str/{STORY}"))).unwrap();
            })
        };
        // `key` et `info` ont disparu avant `rf/` et `sf/` : reste d'un pack Lunii ou Flam.
        assert!(leftover(&[
            ("ri", false),
            ("si", false),
            ("rf", true),
            ("sf", true)
        ]));
        assert!(leftover(&[
            ("main.lsf", false),
            ("version", false),
            ("img", true)
        ]));
        assert!(leftover(&[("script", true), ("sounds", true)]));
        // Un seul nom inconnu, un mauvais type, une autre casse : refus.
        assert!(!leftover(&[
            ("ri", false),
            ("photo.jpg", false),
            ("rf", true)
        ]));
        assert!(!leftover(&[("ri", true)]));
        assert!(!leftover(&[("rf", false)]));
        assert!(!leftover(&[("RI", false)]));
        assert!(!leftover(&[("img", true), ("Mes documents", true)]));
        // Fichier étranger au milieu d'un reste connu.
        assert!(!leftover(&[
            ("si", false),
            ("notes.txt", false),
            ("sf", true)
        ]));
    }

    #[test]
    fn un_flam_transfer_personnel_n_est_jamais_marque_ni_vide() {
        for personal in [
            "notes.txt",
            "str/notes.txt",
            "Mes documents/a.txt",
            "str/vacances/key",
        ] {
            // Migration de démarrage.
            let dir = tempfile::tempdir().unwrap();
            write(&dir.path().join("flam-transfer").join(personal), 9);
            unmarked_transfer(dir.path());
            assert_eq!(
                migrate_once(dir.path()),
                Migration::Done { marked: 0 },
                "{personal}"
            );
            assert!(!is_owned(&dir.path().join("flam-transfer")), "{personal}");
            assert_eq!(usage_of(dir.path()).unwrap(), 0, "{personal}");
            assert_eq!(clear_of(dir.path()).unwrap(), 0, "{personal}");
            assert!(dir.path().join("flam-transfer").join(personal).is_file());
            assert_eq!(foreign_entries(dir.path()), ["flam-transfer"]);

            // Adoption d'un ancien cache.
            let legacy = tempfile::tempdir().unwrap();
            complete_legacy_cache(legacy.path());
            write(&legacy.path().join("flam-transfer").join(personal), 9);
            assert!(adopt_legacy_cache(legacy.path()).unwrap());
            assert!(
                !is_owned(&legacy.path().join("flam-transfer")),
                "{personal}"
            );
            clear_of(legacy.path()).unwrap();
            assert!(legacy.path().join("flam-transfer").join(personal).is_file());
        }
    }

    #[test]
    fn flam_transfer_de_forme_reelle_est_adopte_avec_un_ancien_cache() {
        let legacy = tempfile::tempdir().unwrap();
        complete_legacy_cache(legacy.path());
        unmarked_transfer(legacy.path());
        assert!(adopt_legacy_cache(legacy.path()).unwrap());
        assert!(is_owned(&legacy.path().join("flam-transfer")));
        assert!(usage_report(legacy.path())
            .unwrap()
            .1
            .contains(&"flam-transfer".to_string()));
    }

    #[test]
    fn la_migration_tardive_marque_flam_transfer_d_un_cache_deja_migre() {
        let cache = tempfile::tempdir().unwrap();
        unmarked_transfer(cache.path());
        let (config, env) = config_with_default(cache.path());
        // Migration faite par une version d'avant l'ajout de `flam-transfer` : témoin vide.
        fs::write(config.path().join(MIGRATION_WITNESS), b"").unwrap();
        let late = config.path().join(LATE_WITNESS);
        assert!(!late.exists());

        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&cache.path().join("flam-transfer")));
        assert!(late.is_file(), "migration tardive faite une seule fois");
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        // Marqué : compté et vidé.
        assert_eq!(
            usage_report(cache.path()).unwrap().1,
            ["archives", "flam-transfer"]
        );
        clear_of(cache.path()).unwrap();
        assert_eq!(names_in(&cache.path().join("flam-transfer")), [MARKER]);
    }

    #[test]
    fn la_migration_tardive_ne_marque_pas_un_dossier_personnel_et_le_met_en_attente() {
        let cache = tempfile::tempdir().unwrap();
        ensure_subdir(cache.path(), "archives").unwrap();
        write(&cache.path().join("flam-transfer/Mes documents/a.txt"), 3);
        let (config, env) = config_with_default(cache.path());
        let witness = config.path().join(MIGRATION_WITNESS);
        fs::write(&witness, b"").unwrap();

        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&cache.path().join("flam-transfer")));
        assert_eq!(
            read_pending(&witness),
            [Pending {
                root: cache.path().to_str().unwrap().to_string(),
                name: "flam-transfer".into()
            }]
        );
        assert!(config.path().join(LATE_WITNESS).is_file());
        // Réévalué à chaque démarrage par le rattrapage, jamais marqué tant que le contenu
        // personnel y est.
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&cache.path().join("flam-transfer")));
        assert!(cache
            .path()
            .join("flam-transfer/Mes documents/a.txt")
            .is_file());
        // Le contenu personnel retiré par l'utilisateur : prouvé (vide), marqué.
        fs::remove_dir_all(cache.path().join("flam-transfer/Mes documents")).unwrap();
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&cache.path().join("flam-transfer")));
        assert!(read_pending(&witness).is_empty());
    }

    /// Dossier configuré mais non retenu (valeur refusée, repli sur le défaut) : son
    /// `flam-transfer` est inscrit en attente sans être évalué ; quand ce dossier redevient le
    /// cache effectif, le rattrapage le marque s'il est prouvé.
    #[test]
    fn la_migration_tardive_garde_le_dossier_configure_non_retenu() {
        let base = tempfile::tempdir().unwrap();
        let default = base.path().join("defaut");
        let configured = base.path().join("configure");
        fs::create_dir_all(&default).unwrap();
        unmarked_transfer(&configured);
        let config = base.path().join("config");
        fs::create_dir_all(&config).unwrap();
        set_cache_dir(&config, Some(&configured));
        let witness = config.join(MIGRATION_WITNESS);
        fs::write(&witness, b"").unwrap();

        // `configure` est protégé : refusé, repli sur `defaut`.
        let refusing = env_with(&default, vec![configured.clone()]);
        assert_eq!(
            migrate_configured(&config, &refusing).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!is_owned(&configured.join("flam-transfer")), "pas évalué");
        assert!(config.join(LATE_WITNESS).is_file());
        assert_eq!(
            read_pending(&witness),
            [Pending {
                root: configured.to_str().unwrap().to_string(),
                name: "flam-transfer".into()
            }]
        );
        // Encore non retenu au démarrage suivant : toujours en attente.
        assert_eq!(
            migrate_configured(&config, &refusing).unwrap(),
            Migration::AlreadyDone
        );
        assert_eq!(read_pending(&witness).len(), 1);
        // Il redevient le cache effectif : prouvé, marqué.
        let accepting = env_with(&default, vec![]);
        assert_eq!(
            migrate_configured(&config, &accepting).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&configured.join("flam-transfer")));
        assert!(read_pending(&witness).is_empty());
    }

    #[test]
    fn la_migration_tardive_attend_des_reglages_lisibles_et_un_cache_present() {
        let base = tempfile::tempdir().unwrap();
        let cache = base.path().join("cache");
        let (config, env) = config_with_default(&cache);
        fs::write(config.path().join(MIGRATION_WITNESS), b"").unwrap();
        let late = config.path().join(LATE_WITNESS);

        // Dossier de cache absent (disque débranché) : reporté, rien d'écrit.
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!late.exists());
        assert!(!cache.exists());

        // Réglages illisibles : rien n'est évalué, rien n'est marqué.
        unmarked_transfer(&cache);
        let good = fs::read(config.path().join("settings.json")).unwrap();
        fs::write(config.path().join("settings.json"), b"{corrompu").unwrap();
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::AlreadyDone
        );
        assert!(!late.exists());
        assert!(!is_owned(&cache.join("flam-transfer")));

        // Cache présent et réglages lisibles : migré.
        fs::write(config.path().join("settings.json"), good).unwrap();
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(is_owned(&cache.join("flam-transfer")));
        assert!(late.is_file());
    }

    /// Copie des histoires de la Flam (`str/<uuid>/{key,info,main.lsf,img/,sounds/}`), rangée
    /// par l'utilisateur sous `flam-transfer` : mêmes noms que la préparation de l'application.
    fn device_copy(root: &Path) {
        let story = root.join(format!("flam-transfer/str/{STORY}"));
        for file in ["key", "info", "main.lsf"] {
            write(&story.join(file), 8);
        }
        write(&story.join("img/a.png"), 8);
        write(&story.join("sounds/a.mp3"), 8);
    }

    /// Sans autre sous-dossier marqué (le dossier n'a jamais servi de cache), une copie façon
    /// appareil n'est marquée par aucun chemin : migration tardive, rattrapage, migration
    /// complète ; elle reste en attente, jamais comptée ni vidée.
    #[test]
    fn une_copie_de_la_flam_sans_autre_trace_n_est_jamais_marquee() {
        // Migration tardive (cache déjà migré), puis rattrapage à chaque démarrage.
        let cache = tempfile::tempdir().unwrap();
        device_copy(cache.path());
        let (config, env) = config_with_default(cache.path());
        let witness = config.path().join(MIGRATION_WITNESS);
        fs::write(&witness, b"").unwrap();
        for _ in 0..2 {
            assert_eq!(
                migrate_configured(config.path(), &env).unwrap(),
                Migration::AlreadyDone
            );
            assert!(!is_owned(&cache.path().join("flam-transfer")));
        }
        assert_eq!(read_pending(&witness).len(), 1, "en attente");
        assert_eq!(usage_of(cache.path()).unwrap(), 0);
        assert_eq!(clear_of(cache.path()).unwrap(), 0);

        // Migration complète.
        let fresh = tempfile::tempdir().unwrap();
        device_copy(fresh.path());
        assert_eq!(migrate_once(fresh.path()), Migration::Done { marked: 0 });
        assert!(!is_owned(&fresh.path().join("flam-transfer")));

        // Un autre sous-dossier marqué : trace d'usage, la même copie devient prouvée au
        // rattrapage suivant (limite documentée : indiscernable de la préparation de
        // l'application dans un dossier qui sert réellement de cache).
        ensure_subdir(cache.path(), "archives").unwrap();
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::CaughtUp { marked: 1 }
        );
        assert!(cache
            .path()
            .join(format!("flam-transfer/str/{STORY}/key"))
            .is_file());
    }

    #[test]
    fn la_migration_complete_ecrit_aussi_le_temoin_tardif() {
        let cache = tempfile::tempdir().unwrap();
        unmarked_transfer(cache.path());
        let (config, env) = config_with_default(cache.path());
        assert_eq!(
            migrate_configured(config.path(), &env).unwrap(),
            Migration::Done { marked: 1 }
        );
        assert!(is_owned(&cache.path().join("flam-transfer")));
        assert!(config.path().join(LATE_WITNESS).is_file());
    }
}
