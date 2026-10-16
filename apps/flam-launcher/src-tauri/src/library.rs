//! Bibliothèque des histoires installées : `<cache>/library/<uuid>-<version>.plain.pk` et son
//! index `<cache>/library/library.json`.
//!
//! **Une installation par `uuid`** : installer une histoire dont l'`uuid` est déjà présent
//! remplace l'installation précédente (autre version, ou autre archive de même `uuid`). C'est
//! la règle des doublons : un `.plain` et un `.plain.pk` de même `uuid` dans une source dossier
//! donnent deux entrées côté source, mais **la dernière installée** est celle de la
//! bibliothèque.
//!
//! `uuid` et `version` proviennent d'archives et de catalogues non fiables : ils sont
//! **revalidés** ici ([`validate_story_id`]) avant de construire un chemin, et le chemin final
//! doit rester directement sous `library/`.
//!
//! **Désinstallation** : [`Library::remove`] ne touche qu'au fichier et à l'index ; la commande
//! `library_remove` purge ensuite la copie de travail `<cache>/play/<empreinte>/` du player, et
//! la progression de l'enfant (`<config>/saves/<empreinte>/`) seulement sur demande
//! (`purge_saves`, faux par défaut). Mettre une histoire à jour (nouvelle version du même
//! `uuid`) ne supprime aucune progression ; voir `player::Workspace` pour ce que le player en
//! relit.
//!
//! **Verrous** : toute écriture et toute réconciliation passent par le seul verrou
//! inter-processus `library/.lock` (aussi exclusif entre threads du même processus), pris par
//! [`Library`] elle-même ; il n'y a pas de verrou d'application devant lui. `list` ne fait que
//! l'essayer (`try_lock`) : une installation ou une autre instance qui le tient ne fige donc
//! jamais l'écran Bibliothèque (lecture de l'index sans réconcilier). La désinstallation, qui
//! tient le créneau du player, n'attend le verrou que brièvement ([`Library::remove_with`]).
//!
//! La logique ([`Library`]) ne dépend pas de Tauri ; les commandes `library_*` passent par le
//! service d'installation, qui passe par le verrou inter-processus décrit ci-dessus.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::catalog::{blocking, CatalogService};
use crate::install::InstallService;
use crate::library_thumb::{self, Prepared};
use crate::player::{purge_library_story, PlayerService, SAVES_ROOT};
use crate::settings::{Environment, SettingsStore};
use crate::storage::{self, ConfigState, Loaded};
use crate::story::{validate_story_id, StoryEntry};
use crate::{AppError, AppResult};

/// Sous-dossier du cache (listé dans `cache::APP_CACHE_SUBDIRS`).
pub const LIBRARY_DIR: &str = "library";
/// Index de la bibliothèque, dans `library/`.
pub const INDEX_FILE: &str = "library.json";
/// Dossier de préparation (sous `library/`, même volume : publication par renommage).
pub const STAGING_DIR: &str = ".staging";
/// Suffixe de la sauvegarde d'un fichier remplacé pendant une installation.
const BACKUP_SUFFIX: &str = ".bak";
/// Suffixe du marqueur d'intention d'une publication en cours.
const PENDING_SUFFIX: &str = ".pending";
/// Suffixe d'un marqueur **neutralisé** (`<fichier>.pending.unjudged`) : voir
/// [`Library::neutralize`].
const UNJUDGED_SUFFIX: &str = ".unjudged";
/// Verrou inter-processus de la bibliothèque, dans `library/`.
const LOCK_FILE: &str = ".lock";
/// Attente maximale du verrou inter-processus pour une installation ou une suppression.
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);
/// Pas d'interrogation du verrou pendant l'attente.
const LOCK_POLL: Duration = Duration::from_millis(50);
/// Essais d'une lecture de l'index sans verrou (erreur d'E/S passagère, voir `load_unlocked`).
const UNLOCKED_READ_ATTEMPTS: u32 = 3;
/// Version du schéma de `library.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Histoire installée (miroir de `LibraryEntry` dans `src/api/library.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    /// Identifiant de l'histoire (celui de la source : `uuid` Telmi pour un zip Telmi).
    pub uuid: String,
    /// Version installée (`None` : inconnue, fichier `<uuid>-0.plain.pk`).
    pub version: Option<u32>,
    pub title: String,
    /// Nom du fichier sous `library/` (`<uuid>-<version>.plain.pk`).
    pub file: String,
    /// Taille du `.plain.pk` en octets.
    pub size: u64,
    /// Date d'installation (secondes Unix).
    pub installed_at: u64,
    /// Version de `telmi2flam` utilisée (champ `version` de l'événement `start`, sinon repli
    /// `sha256:<12 hex>` du script), `None` sans conversion.
    pub converter: Option<String>,
    /// Entrée d'origine telle que proposée par la source (source, lien, métadonnées).
    pub source: StoryEntry,
    /// Vignette conservée sous `library/thumbs/` (nom de fichier, voir `library_thumb`), `None`
    /// sans vignette ou pour une installation antérieure à #80.
    #[serde(default)]
    pub thumb: Option<String>,
}

impl LibraryEntry {
    /// Vrai si `candidate` (même `uuid`) propose une autre version que celle installée.
    /// Une version inconnue d'un côté ou de l'autre n'est jamais une mise à jour.
    pub fn is_outdated_by(&self, candidate: &StoryEntry) -> bool {
        candidate.uuid == self.uuid
            && matches!((self.version, candidate.version), (Some(a), Some(b)) if a != b)
    }
}

/// Vrai la première fois qu'un `flag` est consulté, faux ensuite (une seule alerte par cause).
fn first_time(flag: &AtomicBool) -> bool {
    flag.swap(false, Ordering::Relaxed)
}

/// `tracing::warn!` la première fois depuis ce site d'appel (une fois par processus), `debug!`
/// ensuite : `list` réconcilie à chaque appel, et une cause persistante (verrou indisponible,
/// restauration bloquée) ne doit pas inonder le journal.
macro_rules! warn_once {
    ($($arg:tt)*) => {{
        static FIRST: AtomicBool = AtomicBool::new(true);
        if first_time(&FIRST) {
            tracing::warn!($($arg)*);
        } else {
            tracing::debug!($($arg)*);
        }
    }};
}

/// Marqueur d'intention d'une publication (`.staging/<fichier>.pending`).
#[derive(Debug, Serialize, Deserialize)]
struct PendingMarker {
    /// Date d'installation de la nouvelle entrée (la retrouve dans l'index si validée).
    installed_at: u64,
    /// Un fichier de même nom existait (il est alors en `.bak` dès le premier renommage).
    had_previous: bool,
    /// Taille attendue du fichier publié : avec `installed_at`, elle identifie la publication
    /// (deux publications du même fichier dans la même seconde ne sont distinguées que par là).
    /// Absente d'un marqueur écrit par une version antérieure : seul `installed_at` compte.
    #[serde(default)]
    size: Option<u64>,
    /// Fichiers des anciennes versions du même `uuid`, à supprimer une fois l'index écrit.
    /// Listés ici pour que le nettoyage survive à un arrêt (jamais défaits, jamais perdus).
    ///
    /// **Fuite ponctuelle connue** : un marqueur d'une version antérieure de l'application n'a
    /// pas ce champ (vide par défaut). Si un tel marqueur est terminé après une mise à jour de
    /// l'application, les anciennes versions du même `uuid` ne sont pas nettoyées : leurs
    /// `.plain.pk` restent dans `library/`, non indexés et sans marqueur, donc **jamais
    /// supprimés** (le choix sûr). Un seul cas, à l'occasion de cette mise à jour : l'utilisateur
    /// peut les supprimer à la main (ou les ignorer, ils ne sont pas listés).
    #[serde(default)]
    old_files: Vec<String>,
    /// Entrée d'index de la nouvelle publication. Permet, quand l'index a été perdu (corrompu,
    /// mis de côté), de **réintégrer** une publication terminée au lieu de la défaire : un
    /// `.plain.pk` n'est jamais perdu. Absente d'un marqueur d'une version antérieure.
    #[serde(default)]
    entry: Option<LibraryEntry>,
}

impl PendingMarker {
    /// Vrai si `entry` est l'entrée que cette publication a écrite dans l'index.
    fn matches(&self, entry: &LibraryEntry) -> bool {
        entry.installed_at == self.installed_at && self.size.is_none_or(|s| s == entry.size)
    }
}

/// Résultat de [`Library::adopt_unjudged`].
struct Adoption {
    /// Publications terminées à réintégrer (marqueur, `.bak`, chemin du marqueur), de la plus
    /// ancienne à la plus récente.
    adopted: Vec<(PendingMarker, PathBuf, PathBuf)>,
    /// Marqueurs non jugeables d'autres histoires, à neutraliser avant la réécriture de l'index.
    unjudged: Vec<PathBuf>,
}

/// Contenu de `library.json`.
#[derive(Debug, Serialize, Deserialize)]
struct IndexFile {
    schema_version: u32,
    stories: Vec<LibraryEntry>,
}

/// Nom de fichier `<uuid>-<version>.plain.pk` après revalidation de l'`uuid` (motif
/// `^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`, sans `..`, hors noms réservés Windows).
pub fn file_name_for(uuid: &str, version: Option<u32>) -> AppResult<String> {
    if !validate_story_id(uuid) {
        return Err(AppError::Validation(format!(
            "identifiant d'histoire refusé : {uuid:?}"
        )));
    }
    Ok(format!("{uuid}-{}.plain.pk", version.unwrap_or(0)))
}

/// Accès à la bibliothèque d'un dossier de cache.
///
/// Les écritures (`install`, `remove`) et la réconciliation sont sérialisées **par le verrou
/// inter-processus `library/.lock`**, que `Library` prend elle-même : elles le sont donc entre
/// threads comme entre instances de l'application, sans verrou de l'appelant. Les lectures
/// (`list`, `playable`) ne l'exigent pas (voir `list`).
///
/// **Unicité de la sauvegarde** : `.staging/<fichier>.bak` est unique par fichier cible. Une
/// installation est refusée tant qu'un `.bak` ou un marqueur du même fichier subsiste, si bien
/// qu'un `.bak` n'est jamais écrasé.
pub struct Library {
    root: PathBuf,
    /// Attente maximale du verrou (réglable par les tests).
    lock_timeout: Duration,
}

impl Library {
    /// Bibliothèque de `<cache_root>/library`.
    pub fn new(cache_root: &Path) -> Self {
        Self {
            root: cache_root.join(LIBRARY_DIR),
            lock_timeout: LOCK_TIMEOUT,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Dossier de préparation, créé si besoin.
    pub fn staging_dir(&self) -> AppResult<PathBuf> {
        let dir = self.root.join(STAGING_DIR);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Chemin final d'une histoire, vérifié : directement sous `library/`.
    fn final_path(&self, uuid: &str, version: Option<u32>) -> AppResult<PathBuf> {
        let name = file_name_for(uuid, version)?;
        self.checked(&name)
    }

    /// `name` doit être un simple nom de fichier `.plain.pk` sous `library/`.
    fn checked(&self, name: &str) -> AppResult<PathBuf> {
        let path = self.root.join(name);
        let direct = path.parent() == Some(self.root.as_path())
            && path.file_name().and_then(|n| n.to_str()) == Some(name)
            && name.ends_with(".plain.pk");
        if !direct {
            return Err(AppError::Validation(format!(
                "chemin hors de la bibliothèque : {name:?}"
            )));
        }
        Ok(path)
    }

    /// Lit l'index. Absent : vide. Corrompu : sauvegardé en `*.bak`, vide (avertissement).
    /// Version future : erreur, fichier intact.
    fn load(&self) -> AppResult<Vec<LibraryEntry>> {
        Ok(self.load_state()?.0)
    }

    /// Comme [`Library::load`], avec un booléen vrai si l'index ne fait pas foi : corrompu (mis de
    /// côté à cet appel) ou absent après avoir été mis de côté ; la liste vide ne dit alors rien
    /// de ce qui est publié.
    fn load_state(&self) -> AppResult<(Vec<LibraryEntry>, bool)> {
        let path = self.root.join(INDEX_FILE);
        let loaded = storage::load_versioned::<IndexFile>(&path, SCHEMA_VERSION, |_, _| {})?;
        Ok(match loaded {
            Loaded::Found(index) => (index.stories, false),
            // Index absent mais un index corrompu a été mis de côté : l'absence vient de cette
            // mise de côté, elle ne prouve pas que rien n'est publié.
            Loaded::Missing => (Vec::new(), self.index_quarantined()),
            Loaded::Recovered { .. } => (Vec::new(), true),
        })
    }

    /// Vrai si un `library.json.*.bak` (index corrompu mis de côté) existe dans `library/`.
    fn index_quarantined(&self) -> bool {
        std::fs::read_dir(&self.root).is_ok_and(|d| {
            d.flatten().any(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.starts_with("library.json.") && name.ends_with(".bak")
            })
        })
    }

    fn save(&self, stories: Vec<LibraryEntry>) -> AppResult<()> {
        storage::write_json(
            &self.root.join(INDEX_FILE),
            &IndexFile {
                schema_version: SCHEMA_VERSION,
                stories,
            },
        )
    }

    /// Verrou entre processus (`library/.lock`), pris pendant publication et réconciliation.
    /// `wait` : attendre le verrou (`try_lock` en boucle, au plus `lock_timeout`, puis erreur
    /// explicite) ; sinon `Ok(None)` s'il est tenu par une autre instance.
    pub(crate) fn lock_file(&self, wait: bool) -> AppResult<Option<File>> {
        self.lock_core(wait.then_some(self.lock_timeout))
    }

    /// Comme `lock_file(true)`, mais avec une attente maximale `wait` : sert aux appelants qui
    /// tiennent déjà le créneau du player et ne doivent pas le bloquer longtemps.
    fn lock_within(&self, wait: Duration) -> AppResult<File> {
        // Attente bornée : `lock_core` renvoie le verrou ou l'erreur d'expiration, jamais `None`.
        Ok(self
            .lock_core(Some(wait))?
            .expect("attente bornée : verrou ou erreur, jamais None"))
    }

    fn lock_core(&self, wait: Option<Duration>) -> AppResult<Option<File>> {
        std::fs::create_dir_all(&self.root)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(LOCK_FILE))?;
        let deadline = wait.map(|w| Instant::now() + w);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(file)),
                Err(std::fs::TryLockError::WouldBlock) => {
                    let Some((deadline, wait)) = deadline.zip(wait) else {
                        return Ok(None);
                    };
                    if Instant::now() >= deadline {
                        let shown = if wait.as_secs() >= 1 {
                            format!("{} s", wait.as_secs())
                        } else {
                            format!("{} ms", wait.as_millis())
                        };
                        return Err(AppError::Io(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!(
                                "bibliothèque utilisée par une autre instance de l'application \
                                 (verrou {LOCK_FILE} non obtenu en {shown}), réessayer"
                            ),
                        )));
                    }
                    std::thread::sleep(LOCK_POLL);
                }
                Err(std::fs::TryLockError::Error(err)) => return Err(err.into()),
            }
        }
    }

    /// Lit l'index et, si le verrou inter-processus est obtenu, défait les publications
    /// interrompues. Si une autre instance tient le verrou, rien n'est réconcilié.
    fn load_reconciled(&self) -> AppResult<Vec<LibraryEntry>> {
        match self.lock_file(false) {
            Ok(Some(_guard)) => self.reconcile(),
            Ok(None) => self.load_unlocked(),
            Err(err) => {
                warn_once!("verrou de la bibliothèque indisponible, lecture seule : {err}");
                self.load_unlocked()
            }
        }
    }

    /// Lecture de l'index sans le verrou : l'instance qui le tient peut être en train de mettre
    /// de côté un index corrompu (renommage concurrent de `library.json`), d'où une erreur
    /// d'E/S passagère (fichier disparu entre lecture et renommage, accès refusé pendant le
    /// renommage). Elle est réessayée au plus [`UNLOCKED_READ_ATTEMPTS`] fois, à
    /// [`LOCK_POLL`] d'intervalle ; une erreur persistante est renvoyée telle quelle.
    fn load_unlocked(&self) -> AppResult<Vec<LibraryEntry>> {
        self.load_unlocked_with(|_| {})
    }

    /// [`Library::load_unlocked`] avec un crochet appelé après chaque essai en échec (tests).
    fn load_unlocked_with(&self, mut failed: impl FnMut(u32)) -> AppResult<Vec<LibraryEntry>> {
        let mut attempt = 1;
        loop {
            match self.load() {
                Err(AppError::Io(err)) if attempt < UNLOCKED_READ_ATTEMPTS => {
                    tracing::debug!("lecture de l'index sans verrou, nouvel essai : {err}");
                    failed(attempt);
                    attempt += 1;
                    std::thread::sleep(LOCK_POLL);
                }
                other => return other,
            }
        }
    }

    /// Réconciliation (verrou tenu par l'appelant). **Seules** les publications qui ont un
    /// marqueur d'intention `.staging/<fichier>.pending` sont défaites : un `.plain.pk` non
    /// indexé sans marqueur n'est jamais supprimé. Pour un marqueur dont l'index ne contient pas
    /// la nouvelle entrée (même `installed_at`) : le fichier publié est retiré et la sauvegarde
    /// `.bak` restaurée ; sinon la publication était terminée et seuls les résidus partent.
    ///
    /// Index **corrompu** (mis de côté, vu vide), ou absent alors qu'un `library.json.*.bak`
    /// existe : on ne sait rien de ce qui est publié, donc rien n'est défait ni nettoyé ;
    /// marqueurs et `.bak` restent (supprimer le `library.json.*.bak` après réparation les
    /// libère).
    fn reconcile(&self) -> AppResult<Vec<LibraryEntry>> {
        Ok(self.reconcile_state()?.0)
    }

    /// [`Library::reconcile`], avec un booléen vrai si l'index ne faisait pas foi (rien n'a alors
    /// été défait) : `install_with` réintègre ou refuse avant de réécrire l'index.
    fn reconcile_state(&self) -> AppResult<(Vec<LibraryEntry>, bool)> {
        let (stories, recovered) = self.load_state()?;
        if recovered {
            warn_once!("index de la bibliothèque corrompu : aucune publication défaite");
            return Ok((stories, true));
        }
        let Ok(read) = std::fs::read_dir(self.root.join(STAGING_DIR)) else {
            return Ok((stories, false));
        };
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(file) = name.strip_suffix(PENDING_SUFFIX) else {
                continue;
            };
            let Ok(target) = self.checked(file) else {
                continue;
            };
            let backup = item.path().with_file_name(format!("{file}{BACKUP_SUFFIX}"));
            let Some(marker) = std::fs::read(item.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PendingMarker>(&bytes).ok())
            else {
                if backup.exists() {
                    // Le `.bak` est peut-être la seule copie de l'ancienne version.
                    warn_once!(
                        "marqueur {} illisible : marqueur et sauvegarde conservés",
                        item.path().display()
                    );
                } else {
                    let _ = remove_if_exists(&item.path());
                }
                continue;
            };
            let committed = stories.iter().any(|e| e.file == file && marker.matches(e));
            if committed {
                self.finish_publication(&stories, &marker, &backup, &item.path());
            } else if restore_publication(&target, &backup, &item.path(), marker.had_previous) {
                let _ = remove_if_exists(&item.path());
            }
        }
        Ok((stories, false))
    }

    /// Index perdu (voir [`Library::reconcile_state`]) : les publications à marqueur ne peuvent
    /// pas être jugées. Celles dont le marqueur porte l'entrée et dont le fichier publié est
    /// présent à la taille attendue sont **réintégrées** (terminées), triées par `installed_at`
    /// (la plus récente d'un même `uuid` l'emporte). Un marqueur non jugeable (illisible, ancien
    /// format, fichier absent ou de taille différente) qui concerne `file` ou `uuid` fait **refuser**
    /// l'installation (rien n'est touché) ; ceux des autres histoires sont laissés intacts
    /// (avertissement) et n'empêchent pas l'installation. Renvoie les publications à réintégrer
    /// (marqueur, `.bak`, chemin du marqueur) et les marqueurs non jugeables à **neutraliser**
    /// ([`Library::neutralize`]) avant que l'index ne soit réécrit.
    fn adopt_unjudged(&self, file: &str, uuid: &str) -> AppResult<Adoption> {
        let mut adopted = Vec::new();
        let mut unjudged = Vec::new();
        let Ok(read) = std::fs::read_dir(self.root.join(STAGING_DIR)) else {
            return Ok(Adoption { adopted, unjudged });
        };
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(marked) = name.strip_suffix(PENDING_SUFFIX) else {
                continue;
            };
            let marker = std::fs::read(item.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PendingMarker>(&bytes).ok());
            let marker = match marker {
                Some(m) if self.published_intact(marked, &m) => m,
                other => {
                    // `<uuid>-<version>.plain.pk` : l'uuid se déduit du nom si le marqueur est muet.
                    let named_uuid = marked
                        .strip_suffix(".plain.pk")
                        .and_then(|stem| stem.rsplit_once('-'))
                        .map(|(u, _)| u);
                    let entry_uuid = other
                        .as_ref()
                        .and_then(|m| m.entry.as_ref())
                        .map(|e| e.uuid.as_str());
                    let concerned =
                        marked == file || entry_uuid == Some(uuid) || named_uuid == Some(uuid);
                    if concerned {
                        return Err(AppError::Validation(format!(
                            "installation de {file} refusée : le marqueur .staging/{marked}{PENDING_SUFFIX} \
                             ne peut pas être jugé (index de la bibliothèque perdu, sauvegardé en \
                             library.json.*.bak). Vérifier library/{marked}, puis supprimer ce marqueur \
                             (et .staging/{marked}{BACKUP_SUFFIX} s'il existe) ou restaurer l'index depuis la \
                             sauvegarde, et réessayer"
                        )));
                    }
                    unjudged.push(item.path());
                    continue;
                }
            };
            let backup = item
                .path()
                .with_file_name(format!("{marked}{BACKUP_SUFFIX}"));
            adopted.push((marker, backup, item.path()));
        }
        adopted.sort_by_key(|(m, _, _)| m.installed_at);
        Ok(Adoption { adopted, unjudged })
    }

    /// Vrai si le marqueur porte l'entrée de sa publication et que le fichier publié est présent
    /// à la taille attendue : la publication est terminée et peut être réintégrée.
    fn published_intact(&self, marked: &str, marker: &PendingMarker) -> bool {
        let (Ok(target), Some(entry)) = (self.checked(marked), marker.entry.as_ref()) else {
            return false;
        };
        entry.file == marked
            && std::fs::metadata(target).is_ok_and(|t| t.is_file() && t.len() == entry.size)
    }

    /// Neutralise des marqueurs non jugeables d'autres histoires au moment où l'index récupéré
    /// est réécrit : `.staging/<fichier>.pending` devient `.staging/<fichier>.pending.unjudged`.
    ///
    /// Pourquoi : une fois l'index réécrit, il fait de nouveau foi, et la réconciliation jugerait
    /// ce marqueur **inachevé** (la nouvelle entrée n'est pas dans l'index) : elle supprimerait le
    /// fichier publié (sans version précédente) ou restaurerait son `.bak`, c'est-à-dire une
    /// perte de données possible. La réconciliation ne lit que les `*.pending` : un marqueur
    /// neutralisé est ignoré, et **ni son fichier ni son `.bak` ne sont jamais touchés**. Chaque
    /// marqueur est signalé par un avertissement ; l'utilisateur le traite à la main (vérifier
    /// le fichier de `library/`, puis supprimer le `.pending.unjudged`). Un nom déjà pris reçoit
    /// un numéro (`.pending.unjudged.2`…), aucun marqueur neutralisé n'en écrase un autre.
    fn neutralize(&self, unjudged: &[PathBuf]) -> AppResult<()> {
        for path in unjudged {
            let mut target = PathBuf::from(format!("{}{UNJUDGED_SUFFIX}", path.display()));
            let mut n = 2;
            while target.exists() {
                target = PathBuf::from(format!("{}{UNJUDGED_SUFFIX}.{n}", path.display()));
                n += 1;
            }
            std::fs::rename(path, &target).map_err(|err| {
                AppError::Io(std::io::Error::new(
                    err.kind(),
                    format!(
                        "marqueur {} non neutralisé, installation refusée (index non réécrit) : {err}",
                        path.display()
                    ),
                ))
            })?;
            tracing::warn!("{}", neutralized_warning(path, &target));
        }
        Ok(())
    }

    /// Précontrôle **avant le téléchargement** : si l'index est perdu et qu'un marqueur non
    /// jugeable concerne `uuid` ou son fichier, l'installation serait refusée en fin de course
    /// ([`Library::install`]) ; on le dit tout de suite, avec le même message, sans rien
    /// neutraliser ni installer (au plus, comme `list`, la mise de côté d'un index corrompu).
    /// Sans effet si une autre instance tient le verrou (le contrôle de l'installation reste la
    /// référence).
    pub fn precheck_install(&self, uuid: &str, version: Option<u32>) -> AppResult<()> {
        let file = file_name_for(uuid, version)?;
        let Some(_lock) = self.lock_file(false)? else {
            return Ok(());
        };
        if self.load_state()?.1 {
            self.adopt_unjudged(&file, uuid)?;
        }
        Ok(())
    }

    /// Histoires installées dont le fichier existe encore (une entrée dont le fichier a
    /// disparu est ignorée), triées par titre.
    pub fn list(&self) -> AppResult<Vec<LibraryEntry>> {
        let mut stories: Vec<LibraryEntry> = self
            .load_reconciled()?
            .into_iter()
            .filter(|e| self.checked(&e.file).is_ok_and(|p| p.is_file()))
            .collect();
        stories.sort_by_cached_key(|e| (e.title.to_lowercase(), e.uuid.clone()));
        Ok(stories)
    }

    /// Chemin et titre de l'histoire installée `uuid`, relus de l'index et revérifiés (fichier
    /// directement sous `library/`, existant). `not_found` si elle n'est pas installée,
    /// `story_file_missing` si son fichier a disparu.
    pub fn playable(&self, uuid: &str) -> AppResult<(PathBuf, String)> {
        let entry = self
            .load()?
            .into_iter()
            .find(|e| e.uuid == uuid)
            .ok_or_else(|| AppError::NotFound(format!("histoire non installée : {uuid}")))?;
        let path = self.checked(&entry.file)?;
        if !path.is_file() {
            return Err(AppError::StoryFileMissing(format!(
                "le fichier de « {} » a disparu de la bibliothèque",
                entry.title
            )));
        }
        Ok((path, entry.title))
    }

    /// Vérifie que `uuid` est dans l'index (`not_found` sinon), sans toucher au fichier.
    pub fn playable_entry(&self, uuid: &str) -> AppResult<()> {
        self.load()?
            .iter()
            .any(|e| e.uuid == uuid)
            .then_some(())
            .ok_or_else(|| AppError::NotFound(format!("histoire non installée : {uuid}")))
    }

    /// Publie `staged` (un `.plain.pk` complet) comme installation de `source`.
    ///
    /// Ordre sûr en cas d'arrêt ou d'échec : l'éventuel fichier de même nom est d'abord mis de
    /// côté (`.staging/<fichier>.bak`), le nouveau est renommé en place, l'index est écrit, puis
    /// la sauvegarde et les fichiers des anciennes versions du même `uuid` sont supprimés. Un
    /// échec avant la fin remet l'état précédent ; un arrêt brutal est réconcilié au chargement
    /// suivant ([`Library::load_reconciled`]).
    pub fn install(
        &self,
        staged: &Path,
        source: &StoryEntry,
        converter: Option<String>,
        now: u64,
    ) -> AppResult<LibraryEntry> {
        self.install_thumbed(staged, source, converter, now, None)
    }

    /// [`Library::install`] avec la vignette `thumb` (préparée par `library_thumb::prepare`),
    /// écrite sous le verrou avant l'index ; son échec n'empêche jamais l'installation.
    pub fn install_thumbed(
        &self,
        staged: &Path,
        source: &StoryEntry,
        converter: Option<String>,
        now: u64,
        thumb: Option<&Prepared>,
    ) -> AppResult<LibraryEntry> {
        self.install_with(staged, source, converter, now, thumb, &|stories| {
            self.save(stories)
        })
    }

    fn install_with(
        &self,
        staged: &Path,
        source: &StoryEntry,
        converter: Option<String>,
        now: u64,
        thumb: Option<&Prepared>,
        save: &dyn Fn(Vec<LibraryEntry>) -> AppResult<()>,
    ) -> AppResult<LibraryEntry> {
        let target = self.final_path(&source.uuid, source.version)?;
        let file = file_name_for(&source.uuid, source.version)?;
        let _lock = self.lock_file(true)?;
        let (mut stories, untrusted) = self.reconcile_state()?;
        let staging = self.staging_dir()?;
        let backup = staging.join(format!("{file}{BACKUP_SUFFIX}"));
        let pending = staging.join(format!("{file}{PENDING_SUFFIX}"));
        // Une restauration restée en échec (marqueur ou `.bak` survivant à la réconciliation)
        // ne doit pas être écrasée : on refuse sans rien toucher.
        if pending.exists() || backup.exists() {
            // `.bak` d'une publication dont le marqueur a été neutralisé : la réconciliation ne
            // le traitera jamais, il faut le dire (et nommer les deux fichiers à traiter).
            // Nom réellement trouvé : `neutralize` peut avoir ajouté un suffixe numéroté.
            let prefix = format!("{file}{PENDING_SUFFIX}{UNJUDGED_SUFFIX}");
            let neutralized = std::fs::read_dir(&staging).ok().and_then(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|name| name.starts_with(&prefix))
                    .min()
            });
            if let (false, Some(unjudged)) = (pending.exists(), neutralized) {
                return Err(AppError::Validation(format!(
                    "installation de {file} refusée : la sauvegarde .staging/{file}{BACKUP_SUFFIX} \
                     appartient à une publication non jugeable (marqueur neutralisé \
                     .staging/{unjudged}). Vérifier library/{file}, puis supprimer \
                     .staging/{file}{BACKUP_SUFFIX} et .staging/{unjudged}, et réessayer"
                )));
            }
            return Err(AppError::Validation(format!(
                "restauration ou nettoyage précédent non terminé pour {file} (marqueur ou sauvegarde \
                 restés dans .staging), réessayer plus tard"
            )));
        }
        // Index perdu : réintègre les publications terminées (ou refuse) avant de le réécrire.
        let Adoption { adopted, unjudged } = if untrusted {
            self.adopt_unjudged(&file, &source.uuid)?
        } else {
            Adoption {
                adopted: Vec::new(),
                unjudged: Vec::new(),
            }
        };
        // Réintégration (de la plus ancienne à la plus récente) ; les fichiers évincés d'un même
        // `uuid` rejoignent `old_files`.
        let mut evicted: Vec<String> = Vec::new();
        for (previous, _, _) in &adopted {
            if let Some(kept) = &previous.entry {
                evicted.extend(
                    stories
                        .iter()
                        .filter(|e| e.uuid == kept.uuid && e.file != kept.file)
                        .map(|e| e.file.clone()),
                );
                stories.retain(|e| e.uuid != kept.uuid && e.file != kept.file);
                stories.push(kept.clone());
            }
        }
        let had_previous = target.is_file();
        let staged_size = std::fs::metadata(staged)?.len();
        // Vignette (#80) : après les refus, avant l'index. Un échec donne `None` ; une publication
        // défaite plus bas la laisse orpheline, supprimée par `library_thumb::sweep`.
        let thumb = thumb.and_then(|t| library_thumb::store(&self.root, &file, now, t));
        let entry = LibraryEntry {
            uuid: source.uuid.clone(),
            version: source.version,
            title: source.title.clone(),
            file: file.clone(),
            size: staged_size,
            installed_at: now,
            converter,
            source: source.clone(),
            thumb,
        };
        let mut old_files: Vec<String> = stories
            .iter()
            .filter(|e| e.uuid == source.uuid && e.file != file)
            .map(|e| e.file.clone())
            .collect();
        for gone in evicted {
            if gone != file && !old_files.contains(&gone) {
                old_files.push(gone);
            }
        }
        let marker = PendingMarker {
            installed_at: now,
            had_previous,
            size: Some(staged_size),
            old_files,
            entry: Some(entry.clone()),
        };
        // Les marqueurs non jugeables des autres histoires sont neutralisés **avant** toute
        // écriture (marqueur, renommages, index) : l'index réécrit ne doit pas leur faire perdre
        // un fichier à la réconciliation suivante.
        self.neutralize(&unjudged)?;
        // Journal d'intention : écrit avant tout renommage, effacé une fois l'index écrit.
        storage::write_json(&pending, &marker)?;
        if had_previous {
            if let Err(err) = std::fs::rename(&target, &backup) {
                let _ = remove_if_exists(&pending);
                return Err(err.into());
            }
        }
        let restore = || {
            restore_publication(&target, &backup, &pending, had_previous);
        };
        if let Err(err) = std::fs::rename(staged, &target) {
            restore();
            return Err(err.into());
        }
        stories.retain(|e| e.uuid != entry.uuid && e.file != file);
        stories.push(entry.clone());
        if let Err(err) = save(stories.clone()) {
            restore();
            return Err(err);
        }
        // L'index est à jour : le reste est du nettoyage.
        self.finish_publication(&stories, &marker, &backup, &pending);
        // Index perdu avant cette installation, ou mis de côté et restaurable à la main (#97) :
        // ses vignettes doivent survivre, on ne réconcilie pas.
        if !untrusted && !self.index_quarantined() {
            library_thumb::sweep(&self.root, &stories);
        }
        for (previous, previous_backup, previous_pending) in &adopted {
            self.finish_publication(&stories, previous, previous_backup, previous_pending);
        }
        Ok(entry)
    }

    /// Nettoyage d'une publication terminée (verrou tenu), dans cet ordre : anciennes versions du
    /// même `uuid` listées dans le marqueur (sauf si l'index les référence encore), puis `.bak`,
    /// puis marqueur. Le marqueur n'est effacé que si tout est parti : sinon il reste (avertissement)
    /// et la réconciliation suivante retente (un `.bak` sans marqueur ne serait jamais revu et
    /// bloquerait le fichier ; une ancienne version sans marqueur fuirait pour toujours).
    fn finish_publication(
        &self,
        stories: &[LibraryEntry],
        marker: &PendingMarker,
        backup: &Path,
        pending: &Path,
    ) {
        let mut clean = true;
        for old in &marker.old_files {
            let Ok(path) = self.checked(old) else {
                continue;
            };
            if stories.iter().any(|e| e.file == *old) {
                continue;
            }
            if let Err(err) = remove_if_exists(&path) {
                warn_once!(
                    "ancienne version {} non supprimée, marqueur conservé : {err}",
                    path.display()
                );
                clean = false;
            }
        }
        if let Err(err) = remove_if_exists(backup) {
            warn_once!(
                "sauvegarde {} non supprimée, marqueur conservé : {err}",
                backup.display()
            );
            clean = false;
        }
        if !clean {
            return;
        }
        if let Err(err) = remove_if_exists(pending) {
            warn_once!("marqueur {} non effacé : {err}", pending.display());
        }
    }

    /// Désinstalle l'histoire `uuid` (fichier et entrée d'index). Inconnue : `not_found`.
    pub fn remove(&self, uuid: &str) -> AppResult<()> {
        self.remove_with(uuid, self.lock_timeout, || Ok(()))
    }

    /// Désinstalle `uuid` en une seule section critique : verrou inter-processus obtenu en au
    /// plus `wait` (sinon erreur « bibliothèque utilisée par une autre instance », sans rien
    /// toucher), vérification de l'installation (`not_found` avant toute action), `before`
    /// (purge de la copie de travail et de la progression : un échec laisse l'histoire
    /// installée), puis retrait du fichier et de l'entrée d'index.
    ///
    /// Un appelant qui tient le créneau du player passe un `wait` court : l'attente d'une autre
    /// instance ne doit pas retarder `player_launch` ni la sortie du player.
    pub fn remove_with(
        &self,
        uuid: &str,
        wait: Duration,
        before: impl FnOnce() -> AppResult<()>,
    ) -> AppResult<()> {
        let _lock = self.lock_within(wait)?;
        let mut stories = self.reconcile()?;
        let Some(pos) = stories.iter().position(|e| e.uuid == uuid) else {
            return Err(AppError::NotFound(format!(
                "histoire non installée : {uuid}"
            )));
        };
        before()?;
        let entry = stories.remove(pos);
        if let Ok(path) = self.checked(&entry.file) {
            remove_if_exists(&path)?;
        }
        self.save(stories.clone())?;
        if !self.index_quarantined() {
            library_thumb::sweep(&self.root, &stories);
        }
        Ok(())
    }

    /// URL `data:` de la vignette conservée de l'histoire installée `uuid` (`not_found` si elle
    /// n'est pas installée ou n'a pas de vignette).
    pub fn thumbnail(&self, uuid: &str) -> AppResult<String> {
        let entry = self
            .load()?
            .into_iter()
            .find(|e| e.uuid == uuid)
            .ok_or_else(|| AppError::NotFound(format!("histoire non installée : {uuid}")))?;
        library_thumb::data_url(&self.root, &entry)
    }
}

/// Défait une publication : remet la sauvegarde `.bak` en place (le `rename` remplace la cible,
/// qui n'est donc jamais supprimée avant), ou retire le fichier publié s'il n'y avait pas
/// d'ancienne version. Le marqueur `pending` n'est effacé **que si la restauration a réussi** ;
/// sinon marqueur et `.bak` restent pour la réconciliation suivante. Renvoie `true` si réussie.
fn restore_publication(target: &Path, backup: &Path, pending: &Path, had_previous: bool) -> bool {
    let restored = if backup.is_file() {
        std::fs::rename(backup, target)
    } else if had_previous {
        // Sauvegarde absente : rien à restaurer (déjà remise en place).
        Ok(())
    } else {
        remove_if_exists(target)
    };
    if let Err(err) = restored {
        warn_once!(
            "restauration de {} impossible, marqueur et sauvegarde conservés : {err}",
            target.display()
        );
        return false;
    }
    if let Err(err) = remove_if_exists(pending) {
        warn_once!("marqueur {} non effacé : {err}", pending.display());
    }
    true
}

/// Avertissement d'un marqueur neutralisé ([`Library::neutralize`]).
fn neutralized_warning(marker: &Path, target: &Path) -> String {
    format!(
        "marqueur {} non jugeable (index perdu) : neutralisé en {} ; son fichier n'est \
         pas touché, à vérifier puis à supprimer à la main",
        marker.display(),
        target.display()
    )
}

/// Supprime un fichier ; absent n'est pas une erreur.
pub fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Histoires installées (triées par titre).
#[tauri::command]
pub async fn library_list(service: State<'_, InstallService>) -> AppResult<Vec<LibraryEntry>> {
    service.library_list().await
}

/// Désinstalle l'histoire `uuid` : fichier, entrée d'index et copie de travail
/// `<cache>/play/<empreinte>/`.
///
/// La progression de l'enfant (`<config>/saves/<empreinte>/`) est **conservée par défaut** :
/// elle n'est supprimée que si `purge_saves` est vrai (choix explicite de l'utilisateur ; absent
/// ou faux : conservée, et retrouvée si l'histoire est réinstallée). Refusé avec `player_busy`
/// si le player lit cette histoire.
#[tauri::command]
pub async fn library_remove(
    service: State<'_, InstallService>,
    config: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    player: State<'_, PlayerService>,
    uuid: String,
    purge_saves: Option<bool>,
) -> AppResult<()> {
    let purge_saves = purge_saves.unwrap_or(false);
    let cache_root = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    let saves_root = config.dir.join(SAVES_ROOT);
    // Même portillon que le lancement : pas de purge pendant une copie de travail.
    let _gate = catalog.cache_gate().read_owned().await;
    let players = player.inner().clone();
    // Tout sous le créneau du player (refus `player_busy` si le player lit cette histoire, et
    // aucun lancement ne peut s'intercaler) : vérification de l'installation, purge, puis retrait.
    // La purge précède le retrait : un échec laisse l'histoire installée (à retenter) au lieu
    // d'une progression impossible à purger. Les purges sont idempotentes.
    let installs = service.inner().clone();
    blocking(move || {
        players.while_not_playing(&uuid, || {
            installs.library_remove_with(&uuid, || {
                purge_library_story(&cache_root, &saves_root, &uuid, purge_saves)
            })
        })
    })
    .await???;
    drop(_gate);
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::story::StoryKind;

    pub(crate) fn story(uuid: &str, version: Option<u32>) -> StoryEntry {
        StoryEntry {
            source_id: "src".into(),
            entry_id: format!("src:{uuid}"),
            uuid: uuid.into(),
            version,
            title: format!("Histoire {uuid}"),
            description: None,
            age: None,
            category: None,
            author: None,
            voice: None,
            license: None,
            thumb_url: None,
            download_url: "https://h.example.test/a.zip".into(),
            size: None,
            updated_at: None,
            kind: StoryKind::TelmiZip,
        }
    }

    fn staged(lib: &Library, name: &str, bytes: &[u8]) -> PathBuf {
        let path = lib.staging_dir().unwrap().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn installation_puis_liste_puis_suppression() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        assert!(lib.list().unwrap().is_empty());
        let s = staged(&lib, "t.plain.pk", b"PK1234");
        let entry = lib
            .install(&s, &story("abc", Some(3)), Some("sha256:x".into()), 7)
            .unwrap();
        assert_eq!(entry.file, "abc-3.plain.pk");
        assert_eq!(entry.size, 6);
        assert!(dir.path().join("library/abc-3.plain.pk").is_file());
        assert!(!s.exists());
        assert_eq!(lib.list().unwrap(), vec![entry]);

        lib.remove("abc").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert!(!dir.path().join("library/abc-3.plain.pk").exists());
        assert_eq!(lib.remove("abc").unwrap_err().kind(), "not_found");
    }

    #[test]
    fn playable_donne_chemin_et_titre_de_l_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        assert_eq!(lib.playable("abc").unwrap_err().kind(), "not_found");
        let s = staged(&lib, "t.plain.pk", b"PK1234");
        lib.install(&s, &story("abc", Some(3)), None, 7).unwrap();
        let (path, title) = lib.playable("abc").unwrap();
        assert_eq!(path, dir.path().join("library/abc-3.plain.pk"));
        assert_eq!(title, "Histoire abc");
        // Un uuid piégé n'est jamais traduit en chemin : il n'est pas dans l'index.
        assert_eq!(
            lib.playable("..\\..\\evil").unwrap_err().kind(),
            "not_found"
        );

        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            lib.playable("abc").unwrap_err().kind(),
            "story_file_missing"
        );
    }

    #[test]
    fn une_seule_installation_par_uuid_la_derniere_gagne() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"v1");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let b = staged(&lib, "b.plain.pk", b"v2!");
        lib.install(&b, &story("abc", Some(2)), None, 2).unwrap();
        let list = lib.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, Some(2));
        assert!(!dir.path().join("library/abc-1.plain.pk").exists());
        // Réinstallation de la même version : remplacée sur place.
        let c = staged(&lib, "c.plain.pk", b"v2bis");
        lib.install(&c, &story("abc", Some(2)), None, 3).unwrap();
        let list = lib.list().unwrap();
        assert_eq!((list.len(), list[0].size), (1, 5));
    }

    #[test]
    fn uuid_dangereux_refuse_avant_tout_chemin() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        for uuid in ["../x", "a/b", r"a\b", "CON", "..", "", "a.", "nul.txt"] {
            let s = staged(&lib, "t.plain.pk", b"x");
            let err = lib.install(&s, &story(uuid, Some(1)), None, 0).unwrap_err();
            assert_eq!(err.kind(), "validation", "{uuid:?}");
        }
        assert!(file_name_for("abc", None).unwrap() == "abc-0.plain.pk");
    }

    #[test]
    fn une_entree_d_index_hors_bibliotheque_est_ignoree() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged(&lib, "t.plain.pk", b"x");
        let mut entry = lib.install(&s, &story("abc", Some(1)), None, 0).unwrap();
        std::fs::write(dir.path().join("secret.plain.pk"), b"x").unwrap();
        entry.uuid = "zzz".into();
        entry.file = "../secret.plain.pk".into();
        let mut stories = lib.load().unwrap();
        stories.push(entry);
        lib.save(stories).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        lib.remove("zzz").unwrap();
        assert!(dir.path().join("secret.plain.pk").exists());
    }

    #[test]
    fn index_corrompu_sauvegarde_et_vide() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        std::fs::create_dir_all(lib.root()).unwrap();
        std::fs::write(lib.root().join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let backups = std::fs::read_dir(lib.root())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".bak")
            })
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn echec_d_ecriture_de_l_index_restaure_l_etat_precedent() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let fail = |_: Vec<LibraryEntry>| Err(AppError::Io(std::io::Error::other("disque plein")));
        let b = staged(&lib, "b.plain.pk", b"nouveau!");
        let err = lib
            .install_with(&b, &story("abc", Some(2)), None, 2, None, &fail)
            .unwrap_err();
        assert_eq!(err.kind(), "io");
        let path = dir.path().join("library/abc-2.plain.pk");
        assert_eq!(std::fs::read(&path).unwrap(), b"ancien");
        let list = lib.list().unwrap();
        assert_eq!((list.len(), list[0].size), (1, 6));
        // Nouvelle histoire : rien ne reste publié.
        let c = staged(&lib, "c.plain.pk", b"x");
        lib.install_with(&c, &story("neuf", Some(1)), None, 3, None, &fail)
            .unwrap_err();
        assert!(!dir.path().join("library/neuf-1.plain.pk").exists());
    }

    /// Injection de panne (#80) : l'index échoue après l'écriture de la vignette. L'entrée
    /// précédente garde la sienne ; la nouvelle reste orpheline jusqu'à la réconciliation
    /// suivante. Index perdu : aucune vignette supprimée.
    #[test]
    fn vignette_orpheline_apres_echec_de_l_index_puis_reconciliee() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let thumbs = dir.path().join("library").join(library_thumb::THUMBS_DIR);
        let png = crate::thumbnail::encode_png(&crate::thumbnail::RgbaImage {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
        })
        .unwrap();
        let thumb = Prepared::from_bytes(png).unwrap();
        let a = staged(&lib, "a.plain.pk", b"ancien");
        let first = lib
            .install_thumbed(&a, &story("abc", Some(2)), None, 1, Some(&thumb))
            .unwrap();
        let fail = |_: Vec<LibraryEntry>| Err(AppError::Io(std::io::Error::other("disque plein")));
        let b = staged(&lib, "b.plain.pk", b"nouveau!");
        lib.install_with(&b, &story("abc", Some(2)), None, 2, Some(&thumb), &fail)
            .unwrap_err();
        assert!(thumbs.join("abc-2.2.png").is_file(), "orpheline");
        assert_eq!(lib.list().unwrap()[0].thumb, first.thumb);
        assert!(lib.thumbnail("abc").is_ok());

        // Index perdu : l'installation suivante ne réconcilie pas les vignettes.
        std::fs::write(dir.path().join("library").join(INDEX_FILE), b"{corrompu").unwrap();
        let c = staged(&lib, "c.plain.pk", b"x");
        lib.install(&c, &story("neuf", Some(1)), None, 3).unwrap();
        assert!(thumbs.join("abc-2.1.png").is_file());
        assert!(thumbs.join("abc-2.2.png").is_file());

        // Index de nouveau fiable et index mis de côté supprimé (#97) : la désinstallation
        // emporte orphelines et vignettes retirées.
        for e in std::fs::read_dir(dir.path().join("library"))
            .unwrap()
            .flatten()
        {
            if e.file_name().to_string_lossy().ends_with(".bak") {
                std::fs::remove_file(e.path()).unwrap();
            }
        }
        lib.remove("neuf").unwrap();
        assert_eq!(std::fs::read_dir(&thumbs).unwrap().count(), 0);
    }

    fn marker(root: &Path, file: &str, installed_at: u64, had_previous: bool) {
        storage::write_json(
            &root.join(format!(".staging/{file}.pending")),
            &PendingMarker {
                installed_at,
                had_previous,
                size: None,
                old_files: Vec::new(),
                entry: None,
            },
        )
        .unwrap();
    }

    #[test]
    fn fichier_non_indexe_prexistant_conserve() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"x");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let root = dir.path().join("library");
        std::fs::write(root.join("perso.plain.pk"), b"a moi").unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(root.join("perso.plain.pk")).unwrap(),
            b"a moi"
        );
        // Index valide et vide : rien n'est supprimé non plus.
        lib.remove("abc").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert!(root.join("perso.plain.pk").exists());
    }

    #[test]
    fn publication_interrompue_defaite_grace_au_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let root = dir.path().join("library");
        // Arrêt simulé : ancien en sauvegarde, nouveau publié, index inchangé, marqueur présent.
        marker(&root, "abc-2.plain.pk", 2, true);
        std::fs::rename(
            root.join("abc-2.plain.pk"),
            root.join(".staging/abc-2.plain.pk.bak"),
        )
        .unwrap();
        std::fs::write(root.join("abc-2.plain.pk"), b"nouveau, pas indexe").unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"ancien"
        );
        assert!(!root.join(".staging/abc-2.plain.pk.bak").exists());
        assert!(!root.join(".staging/abc-2.plain.pk.pending").exists());
        // Nouvelle histoire interrompue : le fichier publié est retiré.
        marker(&root, "neuf-1.plain.pk", 3, false);
        std::fs::write(root.join("neuf-1.plain.pk"), b"x").unwrap();
        lib.list().unwrap();
        assert!(!root.join("neuf-1.plain.pk").exists());
        // Publication terminée (index à jour) mais marqueur resté : le fichier est conservé.
        marker(&root, "abc-2.plain.pk", 1, true);
        lib.list().unwrap();
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"ancien"
        );
        assert!(!root.join(".staging/abc-2.plain.pk.pending").exists());
        // Un index illisible ne fait rien supprimer.
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert!(root.join("abc-2.plain.pk").exists());
    }

    #[test]
    fn restauration_en_echec_garde_marqueur_et_sauvegarde() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let root = dir.path().join("library");
        let target = root.join("abc-2.plain.pk");
        let backup = root.join(".staging/abc-2.plain.pk.bak");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        // La cible devient un dossier non vide : le rename de la sauvegarde échoue.
        let blocked = |_: Vec<LibraryEntry>| -> AppResult<()> {
            std::fs::remove_file(&target)?;
            std::fs::create_dir(&target)?;
            std::fs::write(target.join("x"), b"x")?;
            Err(AppError::Io(std::io::Error::other("disque plein")))
        };
        let b = staged(&lib, "b.plain.pk", b"nouveau!");
        lib.install_with(&b, &story("abc", Some(2)), None, 2, None, &blocked)
            .unwrap_err();
        assert!(pending.exists() && backup.is_file(), "journal conservé");
        // Une nouvelle installation du même fichier est refusée, sans rien toucher.
        let c = staged(&lib, "c.plain.pk", b"autre");
        let err = lib
            .install(&c, &story("abc", Some(2)), None, 3)
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{err}");
        assert!(pending.exists() && backup.is_file(), "journal intact");
        assert_eq!(std::fs::read(&backup).unwrap(), b"ancien");
        assert!(c.exists(), "archive préparée non consommée");
        // La réconciliation retente tant que la cible est bloquée (entrée masquée : pas un fichier).
        assert!(lib.list().unwrap().is_empty());
        assert!(pending.exists() && backup.is_file());
        // Cible libérée : la réconciliation suivante restaure l'ancienne version.
        std::fs::remove_dir_all(&target).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(std::fs::read(&target).unwrap(), b"ancien");
        assert!(!pending.exists() && !backup.exists());
    }

    #[test]
    fn nettoyage_bloque_garde_le_marqueur_puis_reconciliation_nettoie() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let root = dir.path().join("library");
        let backup = root.join(".staging/abc-2.plain.pk.bak");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        // Index écrit pour de bon, puis le `.bak` devient un dossier non vide (non supprimable).
        let block = |stories: Vec<LibraryEntry>| -> AppResult<()> {
            lib.save(stories)?;
            std::fs::remove_file(&backup)?;
            std::fs::create_dir(&backup)?;
            std::fs::write(backup.join("x"), b"x")?;
            Ok(())
        };
        let b = staged(&lib, "b.plain.pk", b"nouveau!");
        lib.install_with(&b, &story("abc", Some(2)), None, 2, None, &block)
            .unwrap();
        assert!(pending.exists() && backup.exists(), "marqueur conservé");
        assert_eq!(lib.list().unwrap().len(), 1);
        assert!(pending.exists(), "obstacle toujours là");
        std::fs::remove_dir_all(&backup).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert!(!pending.exists() && !backup.exists());
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"nouveau!"
        );
        let c = staged(&lib, "c.plain.pk", b"encore");
        lib.install(&c, &story("abc", Some(2)), None, 3).unwrap();
    }

    #[test]
    fn marqueur_illisible_avec_sauvegarde_ne_supprime_rien() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        let backup = root.join(".staging/abc-2.plain.pk.bak");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        std::fs::write(&pending, b"{pas du json").unwrap();
        std::fs::write(&backup, b"ancien").unwrap();
        lib.list().unwrap();
        assert!(pending.exists() && backup.is_file());
        // Sans `.bak`, un marqueur illisible n'a plus d'intérêt : il est nettoyé.
        std::fs::remove_file(&backup).unwrap();
        lib.list().unwrap();
        assert!(!pending.exists());
    }

    #[test]
    fn restauration_sans_ancienne_version_en_echec_garde_le_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        let target = root.join("neuf-1.plain.pk");
        marker(&root, "neuf-1.plain.pk", 3, false);
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("x"), b"x").unwrap();
        lib.list().unwrap();
        assert!(root.join(".staging/neuf-1.plain.pk.pending").exists());
        std::fs::remove_dir_all(&target).unwrap();
        std::fs::write(&target, b"x").unwrap();
        lib.list().unwrap();
        assert!(!target.exists());
        assert!(!root.join(".staging/neuf-1.plain.pk.pending").exists());
    }

    #[test]
    fn lecture_sans_verrou_disponible_retombe_sur_l_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"x");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        // `.lock` devient un dossier : l'ouverture du verrou échoue.
        let lock = dir.path().join("library").join(LOCK_FILE);
        std::fs::remove_file(&lock).unwrap();
        std::fs::create_dir(&lock).unwrap();
        assert!(lib.lock_file(false).is_err());
        assert_eq!(lib.list().unwrap().len(), 1);
        assert!(lib.install(&a, &story("abc", Some(1)), None, 2).is_err());
    }

    #[test]
    fn une_autre_instance_ne_reconcilie_pas_pendant_une_publication() {
        let dir = tempfile::tempdir().unwrap();
        let first = Library::new(dir.path());
        let second = Library::new(dir.path());
        let a = staged(&first, "a.plain.pk", b"x");
        first.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let root = dir.path().join("library");
        marker(&root, "neuf-1.plain.pk", 9, false);
        std::fs::write(root.join("neuf-1.plain.pk"), b"en cours").unwrap();
        {
            let _held = first.lock_file(true).unwrap().expect("verrou");
            assert!(second.lock_file(false).unwrap().is_none());
            assert_eq!(second.list().unwrap().len(), 1);
            assert!(root.join("neuf-1.plain.pk").exists(), "non réconcilié");
        }
        second.list().unwrap();
        assert!(!root.join("neuf-1.plain.pk").exists());
    }

    #[test]
    fn identite_de_publication_taille_attendue_du_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"nouveau!");
        lib.install(&a, &story("abc", Some(2)), None, 5).unwrap();
        let root = dir.path().join("library");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        let backup = root.join(".staging/abc-2.plain.pk.bak");
        let write = |size: Option<u64>| {
            storage::write_json(
                &pending,
                &PendingMarker {
                    installed_at: 5,
                    had_previous: true,
                    size,
                    old_files: Vec::new(),
                    entry: None,
                },
            )
            .unwrap();
            std::fs::write(&backup, b"ancien").unwrap();
        };
        // Même seconde, autre taille : ce n'est pas la publication indexée, elle est défaite.
        write(Some(99));
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"ancien"
        );
        assert!(!pending.exists() && !backup.exists());
        // Même seconde, même taille : publication terminée, le fichier publié est gardé.
        std::fs::write(root.join("abc-2.plain.pk"), b"nouveau!").unwrap();
        write(Some(8));
        lib.list().unwrap();
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"nouveau!"
        );
        assert!(!pending.exists() && !backup.exists());
        // Marqueur d'une version antérieure (sans taille) : seul `installed_at` compte.
        std::fs::write(
            &pending,
            br#"{"installed_at":5,"had_previous":false}"#.as_slice(),
        )
        .unwrap();
        lib.list().unwrap();
        assert!(root.join("abc-2.plain.pk").exists() && !pending.exists());
    }

    #[test]
    fn anciennes_versions_nettoyees_malgre_un_blocage_puis_a_la_reconciliation() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        let a = staged(&lib, "a.plain.pk", b"v1");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let old = root.join("abc-1.plain.pk");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        // Index écrit pour de bon, puis l'ancienne version devient un dossier non supprimable.
        let block = |stories: Vec<LibraryEntry>| -> AppResult<()> {
            lib.save(stories)?;
            std::fs::remove_file(&old)?;
            std::fs::create_dir(&old)?;
            std::fs::write(old.join("x"), b"x")?;
            Ok(())
        };
        let b = staged(&lib, "b.plain.pk", b"v2!");
        lib.install_with(&b, &story("abc", Some(2)), None, 2, None, &block)
            .unwrap();
        let marker: PendingMarker =
            serde_json::from_slice(&std::fs::read(&pending).unwrap()).unwrap();
        assert_eq!(marker.old_files, ["abc-1.plain.pk"], "marqueur conservé");
        // Arrêt simulé ici : la réconciliation suivante retente, sans rien défaire.
        assert_eq!(lib.list().unwrap().len(), 1);
        assert!(pending.exists(), "obstacle toujours là");
        std::fs::remove_dir_all(&old).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert!(!old.exists() && !pending.exists());
        assert!(root.join("abc-2.plain.pk").is_file());
    }

    #[test]
    fn marqueur_termine_nettoie_les_anciennes_versions_sauf_si_indexees() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        let a = staged(&lib, "a.plain.pk", b"v2");
        let entry = lib.install(&a, &story("abc", Some(2)), None, 4).unwrap();
        std::fs::write(root.join("abc-1.plain.pk"), b"reste").unwrap();
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        let write = |old_files: &[&str]| {
            storage::write_json(
                &pending,
                &PendingMarker {
                    installed_at: 4,
                    had_previous: false,
                    size: Some(entry.size),
                    old_files: old_files.iter().map(|f| (*f).to_string()).collect(),
                    entry: None,
                },
            )
            .unwrap();
        };
        // Une entrée d'index n'est jamais supprimée, ni un nom hors bibliothèque.
        write(&["abc-2.plain.pk", "../x.plain.pk"]);
        lib.list().unwrap();
        assert!(root.join("abc-2.plain.pk").is_file() && !pending.exists());
        // Arrêt après l'index, avant le nettoyage : l'ancienne version est supprimée.
        write(&["abc-1.plain.pk"]);
        lib.list().unwrap();
        assert!(!root.join("abc-1.plain.pk").exists() && !pending.exists());
    }

    #[test]
    fn index_corrompu_ne_defait_aucune_publication() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let root = dir.path().join("library");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        let backup = root.join(".staging/abc-2.plain.pk.bak");
        // Publication terminée dont l'index est ensuite corrompu : vu vide, il ne prouve rien.
        marker(&root, "abc-2.plain.pk", 2, true);
        std::fs::write(&backup, b"ancien").unwrap();
        std::fs::write(root.join("abc-2.plain.pk"), b"nouveau").unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"nouveau"
        );
        assert!(
            pending.exists() && backup.is_file(),
            "rien défait ni nettoyé"
        );
        // L'index est maintenant absent (mis de côté) : toujours rien de défait.
        assert!(lib.list().unwrap().is_empty());
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"nouveau"
        );
        assert!(pending.exists() && backup.is_file(), "second appel : idem");
    }

    /// Marqueur d'une publication terminée (avec son entrée) dont l'index est ensuite perdu.
    fn terminee_puis_index_perdu(lib: &Library, root: &Path) -> LibraryEntry {
        let a = staged(lib, "a.plain.pk", b"nouveau");
        let entry = lib.install(&a, &story("abc", Some(2)), None, 2).unwrap();
        storage::write_json(
            &root.join(".staging/abc-2.plain.pk.pending"),
            &PendingMarker {
                installed_at: 2,
                had_previous: false,
                size: Some(entry.size),
                old_files: Vec::new(),
                entry: Some(entry.clone()),
            },
        )
        .unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        entry
    }

    #[test]
    fn index_perdu_puis_installation_ne_perd_aucun_fichier() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        let entry = terminee_puis_index_perdu(&lib, &root);
        // Une autre histoire est installée : l'index est réécrit avec la publication terminée.
        let b = staged(&lib, "b.plain.pk", b"autre");
        lib.install(&b, &story("xyz", Some(1)), None, 3).unwrap();
        let list = lib.list().unwrap();
        let mut uuids: Vec<&str> = list.iter().map(|e| e.uuid.as_str()).collect();
        uuids.sort_unstable();
        assert_eq!(uuids, ["abc", "xyz"]);
        assert_eq!(list.iter().find(|e| e.uuid == "abc"), Some(&entry));
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"nouveau"
        );
        assert!(!root.join(".staging/abc-2.plain.pk.pending").exists());
    }

    #[test]
    fn index_perdu_marqueur_d_un_autre_uuid_n_empeche_pas_l_installation() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        // Marqueur non jugeable (sans entrée) d'une autre histoire : laissé intact.
        marker(&root, "autre-1.plain.pk", 2, false);
        std::fs::write(root.join("autre-1.plain.pk"), b"autre").unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let b = staged(&lib, "b.plain.pk", b"nouveau");
        lib.install(&b, &story("xyz", Some(1)), None, 3).unwrap();
        // Le marqueur est neutralisé (jamais jugé par la réconciliation), son fichier intact.
        assert!(!root.join(".staging/autre-1.plain.pk.pending").exists());
        assert!(root
            .join(".staging/autre-1.plain.pk.pending.unjudged")
            .exists());
        assert!(root.join("autre-1.plain.pk").is_file());
        // Même uuid (autre version) : refus, message nommant le fichier et le marqueur (le
        // marqueur est remis à son nom : il n'a plus été neutralisé).
        std::fs::rename(
            root.join(".staging/autre-1.plain.pk.pending.unjudged"),
            root.join(".staging/autre-1.plain.pk.pending"),
        )
        .unwrap();
        let c = staged(&lib, "c.plain.pk", b"v2");
        std::fs::remove_file(root.join(INDEX_FILE)).unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let err = lib
            .install(&c, &story("autre", Some(2)), None, 4)
            .unwrap_err();
        let text = err.to_string();
        assert_eq!(err.kind(), "validation");
        assert_eq!(
            text,
            "installation de autre-2.plain.pk refusée : le marqueur \
             .staging/autre-1.plain.pk.pending ne peut pas être jugé (index de la bibliothèque \
             perdu, sauvegardé en library.json.*.bak). Vérifier library/autre-1.plain.pk, puis \
             supprimer ce marqueur (et .staging/autre-1.plain.pk.bak s'il existe) ou restaurer \
             l'index depuis la sauvegarde, et réessayer"
        );
        assert_single_line(&text);
        assert!(c.exists(), "rien n'est consommé");
    }

    /// Message destiné à l'utilisateur : une seule ligne, sans espaces doublés (#91).
    fn assert_single_line(text: &str) {
        assert!(!text.contains('\n') && !text.contains("  "), "{text:?}");
    }

    #[test]
    fn l_avertissement_de_neutralisation_tient_sur_une_ligne() {
        let text = neutralized_warning(Path::new("m.pending"), Path::new("m.pending.unjudged"));
        assert_eq!(
            text,
            "marqueur m.pending non jugeable (index perdu) : neutralisé en m.pending.unjudged ; \
             son fichier n'est pas touché, à vérifier puis à supprimer à la main"
        );
        assert_single_line(&text);
    }

    /// #91 : un `.bak` resté avec un marqueur neutralisé n'est jamais traité par la
    /// réconciliation ; le refus nomme les deux fichiers, sans parler de restauration en cours.
    #[test]
    fn un_bak_reste_avec_un_marqueur_neutralise_est_nomme_dans_le_refus() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        lib.list().unwrap();
        let staging = lib.staging_dir().unwrap();
        std::fs::write(staging.join("abc-1.plain.pk.pending.unjudged"), b"m").unwrap();
        std::fs::write(staging.join("abc-1.plain.pk.bak"), b"ancien").unwrap();
        let a = staged(&lib, "a.plain.pk", b"x");
        let err = lib
            .install(&a, &story("abc", Some(1)), None, 1)
            .unwrap_err();
        assert_eq!(err.kind(), "validation");
        let text = err.to_string();
        assert_eq!(
            text,
            "installation de abc-1.plain.pk refusée : la sauvegarde .staging/abc-1.plain.pk.bak \
             appartient à une publication non jugeable (marqueur neutralisé \
             .staging/abc-1.plain.pk.pending.unjudged). Vérifier library/abc-1.plain.pk, puis \
             supprimer .staging/abc-1.plain.pk.bak et .staging/abc-1.plain.pk.pending.unjudged, \
             et réessayer"
        );
        assert_single_line(&text);
        assert!(a.exists(), "rien n'est consommé");
        assert_eq!(
            std::fs::read(staging.join("abc-1.plain.pk.bak")).unwrap(),
            b"ancien"
        );
        // #100 : nom numéroté par `neutralize` (`.unjudged` déjà pris) : c'est lui qui est nommé.
        std::fs::rename(
            staging.join("abc-1.plain.pk.pending.unjudged"),
            staging.join("abc-1.plain.pk.pending.unjudged.2"),
        )
        .unwrap();
        let text = lib
            .install(&a, &story("abc", Some(1)), None, 1)
            .unwrap_err()
            .to_string();
        assert!(
            text.contains("neutralisé .staging/abc-1.plain.pk.pending.unjudged.2)")
                && text.contains("et .staging/abc-1.plain.pk.pending.unjudged.2, et réessayer"),
            "{text}"
        );
        // Sans marqueur neutralisé, le message générique reste celui d'une restauration.
        std::fs::remove_file(staging.join("abc-1.plain.pk.pending.unjudged.2")).unwrap();
        let err = lib
            .install(&a, &story("abc", Some(1)), None, 1)
            .unwrap_err();
        assert!(err.to_string().contains("non terminé"), "{err}");
    }

    /// Priorité de #82 : un marqueur non jugeable d'une autre histoire, laissé tel quel pendant la
    /// réécriture de l'index, aurait été défait par la réconciliation suivante (fichier supprimé).
    #[test]
    fn un_marqueur_non_jugeable_d_une_autre_histoire_ne_fait_pas_perdre_son_fichier() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        marker(&root, "autre-1.plain.pk", 2, false);
        std::fs::write(root.join("autre-1.plain.pk"), b"autre").unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let b = staged(&lib, "b.plain.pk", b"nouveau");
        lib.install(&b, &story("xyz", Some(1)), None, 3).unwrap();
        // Deux listes : la seconde réconcilie un index qui fait de nouveau foi.
        for _ in 0..2 {
            let list = lib.list().unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].uuid, "xyz");
            assert_eq!(
                std::fs::read(root.join("autre-1.plain.pk")).unwrap(),
                b"autre",
                "fichier conservé"
            );
        }
        assert!(root
            .join(".staging/autre-1.plain.pk.pending.unjudged")
            .is_file());
        assert!(!root.join(".staging/autre-1.plain.pk.pending").exists());
    }

    #[test]
    fn le_precontrole_refuse_avant_le_telechargement_sans_rien_modifier() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        // Index sain : rien à contrôler.
        lib.precheck_install("abc", Some(1)).unwrap();
        marker(&root, "abc-1.plain.pk", 2, false);
        lib.precheck_install("abc", Some(1)).unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        // Même uuid : refus immédiat, marqueur intact.
        let err = lib.precheck_install("abc", Some(2)).unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(root.join(".staging/abc-1.plain.pk.pending").exists());
        // Autre histoire : accepté, et rien n'est neutralisé par un simple précontrôle.
        lib.precheck_install("xyz", Some(1)).unwrap();
        assert!(root.join(".staging/abc-1.plain.pk.pending").exists());
    }

    #[test]
    fn plusieurs_marqueurs_neutralises_d_un_meme_fichier_ne_s_ecrasent_pas() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        let staging = lib.staging_dir().unwrap();
        std::fs::write(staging.join("autre-1.plain.pk.pending.unjudged"), b"ancien").unwrap();
        marker(&root, "autre-1.plain.pk", 2, false);
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let b = staged(&lib, "b.plain.pk", b"x");
        lib.install(&b, &story("xyz", Some(1)), None, 3).unwrap();
        assert_eq!(
            std::fs::read(staging.join("autre-1.plain.pk.pending.unjudged")).unwrap(),
            b"ancien"
        );
        assert!(staging
            .join("autre-1.plain.pk.pending.unjudged.2")
            .is_file());
    }

    #[test]
    fn index_absent_avec_sauvegarde_reintegre_la_publication_terminee() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        let entry = terminee_puis_index_perdu(&lib, &root);
        // `list` a mis l'index corrompu de côté : il est absent, `library.json.*.bak` existe.
        assert!(!root.join(INDEX_FILE).exists());
        assert!(lib.index_quarantined());
        let b = staged(&lib, "b.plain.pk", b"autre");
        lib.install(&b, &story("xyz", Some(1)), None, 3).unwrap();
        let list = lib.list().unwrap();
        assert!(list.contains(&entry), "publication réintégrée : {list:?}");
        assert!(root.join("abc-2.plain.pk").is_file());
    }

    #[test]
    fn fichier_de_taille_differente_n_est_pas_reintegre_et_n_est_jamais_supprime() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        terminee_puis_index_perdu(&lib, &root);
        // Le fichier publié a changé depuis (taille différente) : la publication ne se juge plus.
        std::fs::write(root.join("abc-2.plain.pk"), b"modifie a la main, plus long").unwrap();
        // Autre histoire : installation acceptée, marqueur neutralisé, fichier conservé.
        let b = staged(&lib, "b.plain.pk", b"autre");
        lib.install(&b, &story("xyz", Some(1)), None, 3).unwrap();
        for _ in 0..2 {
            let list = lib.list().unwrap();
            assert!(list.iter().all(|e| e.uuid != "abc"), "pas réintégrée");
            assert_eq!(
                std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
                b"modifie a la main, plus long"
            );
        }
        assert!(root
            .join(".staging/abc-2.plain.pk.pending.unjudged")
            .is_file());
        // Même uuid : refus (rien n'est touché).
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        lib.list().unwrap();
        let c = staged(&lib, "c.plain.pk", b"v3");
        std::fs::rename(
            root.join(".staging/abc-2.plain.pk.pending.unjudged"),
            root.join(".staging/abc-2.plain.pk.pending"),
        )
        .unwrap();
        let err = lib
            .install(&c, &story("abc", Some(3)), None, 4)
            .unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(c.exists(), "rien n'est consommé");
    }

    #[test]
    fn reintegration_triee_par_date_et_fichiers_evinces_supprimes() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        // Deux publications terminées du même uuid, marqueurs seuls (index perdu).
        let published = |version: u32, at: u64, bytes: &[u8]| {
            let file = format!("abc-{version}.plain.pk");
            std::fs::write(root.join(&file), bytes).unwrap();
            let s = story("abc", Some(version));
            let entry = LibraryEntry {
                uuid: s.uuid.clone(),
                version: s.version,
                title: s.title.clone(),
                file: file.clone(),
                size: bytes.len() as u64,
                installed_at: at,
                converter: None,
                source: s,
                thumb: None,
            };
            storage::write_json(
                &root.join(format!(".staging/{file}.pending")),
                &PendingMarker {
                    installed_at: at,
                    had_previous: false,
                    size: Some(entry.size),
                    old_files: Vec::new(),
                    entry: Some(entry),
                },
            )
            .unwrap();
        };
        published(1, 10, b"v1");
        published(2, 20, b"v2!");
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        // Une autre histoire est installée : la plus récente (v2) gagne, v1 est évincée et supprimée.
        let b = staged(&lib, "b.plain.pk", b"autre");
        lib.install(&b, &story("xyz", Some(1)), None, 30).unwrap();
        let list = lib.list().unwrap();
        let abc: Vec<_> = list.iter().filter(|e| e.uuid == "abc").collect();
        assert_eq!(abc.len(), 1);
        assert_eq!(abc[0].version, Some(2));
        assert!(root.join("abc-2.plain.pk").is_file());
        assert!(!root.join("abc-1.plain.pk").exists(), "évincée supprimée");
        // Réinstaller le même uuid évince à son tour le fichier réintégré.
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        lib.list().unwrap();
        published(2, 20, b"v2!");
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        lib.list().unwrap();
        let c = staged(&lib, "c.plain.pk", b"v3");
        lib.install(&c, &story("abc", Some(3)), None, 40).unwrap();
        assert!(root.join("abc-3.plain.pk").is_file());
        assert!(!root.join("abc-2.plain.pk").exists());
    }

    #[test]
    fn index_perdu_publication_non_resolue_refuse_l_installation() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        // Marqueur sans entrée (ancien format) : on ne sait pas le juger.
        marker(&root, "abc-2.plain.pk", 2, false);
        std::fs::write(root.join("abc-2.plain.pk"), b"nouveau").unwrap();
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let b = staged(&lib, "b.plain.pk", b"autre");
        let err = lib
            .install(&b, &story("abc", Some(1)), None, 3)
            .unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(b.exists(), "rien n'est consommé");
        assert!(!root.join(INDEX_FILE).exists(), "index non réécrit");
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"nouveau"
        );
        assert!(root.join(".staging/abc-2.plain.pk.pending").exists());
        // Sans publication en attente, l'installation reste possible après une corruption.
        std::fs::remove_file(root.join(".staging/abc-2.plain.pk.pending")).unwrap();
        lib.install(&b, &story("abc", Some(1)), None, 3).unwrap();
    }

    /// #91 : `list` sans verrou (une autre instance le tient) réessaie une erreur d'E/S passagère
    /// à la lecture de l'index, et renvoie une erreur persistante après des essais bornés.
    #[test]
    fn list_sans_verrou_reessaie_une_erreur_passagere_puis_abandonne() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"x");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let index = dir.path().join("library").join(INDEX_FILE);
        let saved = std::fs::read(&index).unwrap();
        let other = Library::new(dir.path());
        let _held = other.lock_file(true).unwrap().expect("verrou");
        // Lire un dossier à la place de l'index : erreur d'E/S (ni absent, ni corrompu).
        std::fs::remove_file(&index).unwrap();
        std::fs::create_dir(&index).unwrap();
        let started = Instant::now();
        assert_eq!(lib.list().unwrap_err().kind(), "io");
        assert!(started.elapsed() >= LOCK_POLL * (UNLOCKED_READ_ATTEMPTS - 1));
        // Passagère : l'index revient après le premier essai en échec (crochet, sans course).
        let mut failures = Vec::new();
        let list = lib
            .load_unlocked_with(|attempt| {
                failures.push(attempt);
                std::fs::remove_dir(&index).unwrap();
                std::fs::write(&index, &saved).unwrap();
            })
            .unwrap();
        assert_eq!(failures, [1], "un seul essai en échec, puis réussite");
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn verrou_occupe_erreur_claire_apres_le_delai() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::new(dir.path());
        lib.lock_timeout = Duration::from_millis(150);
        let other = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"x");
        {
            let _held = other.lock_file(true).unwrap().expect("verrou");
            let started = Instant::now();
            let err = lib
                .install(&a, &story("abc", Some(1)), None, 1)
                .unwrap_err();
            assert_eq!(err.kind(), "io");
            assert_eq!(
                err.to_string(),
                "erreur d'entrée/sortie : bibliothèque utilisée par une autre instance de \
                 l'application (verrou .lock non obtenu en 150 ms), réessayer"
            );
            assert_single_line(&err.to_string());
            assert!(started.elapsed() >= Duration::from_millis(100));
            assert_eq!(lib.remove("abc").unwrap_err().kind(), "io");
            assert!(a.exists(), "rien n'a été consommé");
        }
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
    }

    #[test]
    fn verrou_libere_pendant_l_attente_est_obtenu() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::new(dir.path());
        lib.lock_timeout = Duration::from_secs(20);
        let other = Library::new(dir.path());
        let held = other.lock_file(true).unwrap().expect("verrou");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            drop(held);
        });
        assert!(lib.lock_file(true).unwrap().is_some());
        releaser.join().unwrap();
    }

    #[test]
    fn alerte_unique_par_cause() {
        let flag = AtomicBool::new(true);
        assert!(first_time(&flag));
        assert!(!first_time(&flag));
        assert!(!first_time(&flag));
    }

    #[test]
    fn detection_des_mises_a_jour() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged(&lib, "t.plain.pk", b"x");
        let installed = lib.install(&s, &story("abc", Some(2)), None, 0).unwrap();
        assert!(installed.is_outdated_by(&story("abc", Some(3))));
        assert!(!installed.is_outdated_by(&story("abc", Some(2))));
        assert!(!installed.is_outdated_by(&story("abc", None)));
        assert!(!installed.is_outdated_by(&story("autre", Some(3))));
    }
}
