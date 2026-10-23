//! Téléchargement par lot des épisodes et des pochettes d'un pack (#213).
//!
//! S'appuie sur le téléchargeur de l'installation ([`crate::install::download`] : fichier
//! `.part`, reprise `Range`/`If-Range`, taille bornée, renommage final) avec le client gardé de
//! [`super::net`] (D10 : adresses privées refusées après DNS et à chaque redirection).
//!
//! Comportement :
//! - un pack peut mêler des épisodes de **plusieurs flux** (`Item::feed` n'est qu'une étiquette) ;
//! - concurrence bornée, progression globale, annulation (tous les fichiers du lot sont alors
//!   supprimés) ;
//! - épisode inaccessible (HTTP, délai, adresse refusée, trop gros) : **retiré** et signalé
//!   (P7) ; échec global seulement si aucun épisode n'est récupérable ;
//! - pochette inaccessible : l'épisode est gardé sans pochette, signalé dans le rapport ;
//! - plafonds D1 vérifiés pendant le téléchargement : par épisode (l'épisode est retiré), au
//!   total du lot (arrêt propre de tout le lot, fichiers supprimés) ;
//! - coupure en cours de fichier : reprise automatique (jusqu'à 3 essais) avec `Range` ;
//! - débit minimal : un fichier qui reçoit moins de 1 Kio/s pendant 2 minutes est retiré
//!   (`TimedOut`), en plus du délai par fichier ; après une reprise, seuls les octets reçus
//!   par la nouvelle requête comptent ;
//! - durée du lot bornée ([`Limits::batch_timeout`], 2 h) : au-delà, tout le lot est arrêté et
//!   ses fichiers supprimés ;
//! - aucune URL ni identifiant d'épisode dans les journaux (D11) : seul l'index dans le lot ;
//!   `Item::id` n'est jamais une URL (lot refusé), un `Item::feed` qui en a la forme est
//!   remplacé par [`NEUTRAL_FEED_LABEL`].
//!
//! Les fichiers sont nommés `ep-<n>.audio` et `ep-<n>.img` dans le dossier de travail ; ce
//! dernier est supprimé par l'appelant après la publication du pack (N8 a, [`Workdir`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Semaphore};
use tokio::task::JoinSet;
use url::Url;

use super::net::{check_url, guarded_client, redact_url, NetPolicy};
use crate::install::download::{self, Download, Validator, BLOCKED_MESSAGE};
use crate::{AppError, AppResult};

/// D1 : épisodes par pack.
pub const MAX_EPISODES: usize = 50;
/// D1 : octets par épisode (200 Mio).
pub const MAX_EPISODE_BYTES: u64 = 200 * 1024 * 1024;
/// D1 : octets au total (1 Gio), audio et pochettes confondus.
pub const MAX_TOTAL_BYTES: u64 = 1024 * 1024 * 1024;
/// Débit minimal toléré (octets par seconde), mesuré sur [`MIN_RATE_WINDOW`].
pub const MIN_RATE_BYTES_PER_SEC: u64 = 1024;
/// Fenêtre de mesure du débit minimal.
pub const MIN_RATE_WINDOW: Duration = Duration::from_secs(120);
/// Octets par pochette (comme l'entrée de la conversion).
pub const MAX_IMAGE_BYTES: u64 = super::artwork::MAX_INPUT_BYTES as u64;
/// Durée maximale d'un lot complet : le débit minimal seul laisserait 50 épisodes et leurs
/// pochettes à ~1 Kio/s occuper le lot une douzaine d'heures.
pub const BATCH_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
/// Libellé qui remplace un `Item::feed` ayant la forme d'une URL (D11).
pub const NEUTRAL_FEED_LABEL: &str = "flux";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const ATTEMPTS: usize = 3;

/// Bornes d'un lot. `Default` : les plafonds D1.
#[derive(Debug, Clone)]
pub struct Limits {
    pub max_episodes: usize,
    pub max_episode_bytes: u64,
    pub max_total_bytes: u64,
    pub max_image_bytes: u64,
    /// Téléchargements simultanés.
    pub concurrency: usize,
    /// Durée maximale d'un fichier (silence ou lenteur extrême), reprises comprises.
    pub file_timeout: Duration,
    /// Débit minimal (octets par seconde) sur `min_rate_window` ; en dessous, le fichier est
    /// retiré (`TimedOut`). Un flux malveillant très lent ne peut pas occuper le lot des heures.
    pub min_rate_bytes_per_sec: u64,
    /// Fenêtre de mesure du débit minimal (zéro : désactivé).
    pub min_rate_window: Duration,
    /// Durée maximale du lot entier ; au-delà, le lot est arrêté et ses fichiers supprimés.
    pub batch_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_episodes: MAX_EPISODES,
            max_episode_bytes: MAX_EPISODE_BYTES,
            max_total_bytes: MAX_TOTAL_BYTES,
            max_image_bytes: MAX_IMAGE_BYTES,
            concurrency: 4,
            file_timeout: Duration::from_secs(30 * 60),
            min_rate_bytes_per_sec: MIN_RATE_BYTES_PER_SEC,
            min_rate_window: MIN_RATE_WINDOW,
            batch_timeout: BATCH_TIMEOUT,
        }
    }
}

/// Un épisode à récupérer. Les URL sont des secrets (D11) : ne jamais les journaliser.
#[derive(Clone)]
pub struct Item {
    /// Identifiant stable de l'épisode (rapport). Jamais une URL (D11) : un guid RSS qui en
    /// est une doit être remplacé par un identifiant local ; `fetch_all` le refuse sinon.
    pub id: String,
    /// Libellé affichable du flux d'origine (plusieurs flux possibles dans un même lot). #217 y
    /// passe l'identifiant local de la source, pas son titre ni son URL. Une valeur qui a la
    /// forme d'une URL (titre de flux mal choisi, URL passée par erreur) n'est ni journalisée ni
    /// renvoyée : elle est remplacée par [`NEUTRAL_FEED_LABEL`], sans faire échouer le lot.
    pub feed: String,
    pub audio_url: String,
    pub image_url: Option<String>,
}

// Pas de `Debug` dérivé : il afficherait les URL.
impl std::fmt::Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Item")
            .field("id", &self.id)
            .field("feed", &self.feed)
            .finish_non_exhaustive()
    }
}

/// Pourquoi un épisode est retiré du pack (P7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// Erreur HTTP, connexion impossible ou coupée.
    Unreachable,
    /// Délai dépassé.
    TimedOut,
    /// Adresse privée, locale ou non publique (D10).
    Blocked,
    /// Au-delà du plafond par épisode (D1).
    TooLarge,
    /// Réponse inexploitable (taille incohérente…).
    Invalid,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub id: String,
    pub feed: String,
    pub reason: DropReason,
}

/// Épisode récupéré.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub id: String,
    pub feed: String,
    pub audio: PathBuf,
    pub audio_bytes: u64,
    /// `None` : pas d'adresse de pochette, ou pochette inaccessible (voir
    /// [`Report::image_failures`]).
    pub image: Option<PathBuf>,
}

/// Résultat d'un lot : épisodes dans l'ordre d'entrée.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub fetched: Vec<Fetched>,
    pub dropped: Vec<Dropped>,
    /// Identifiants des épisodes dont la pochette n'a pas pu être récupérée.
    pub image_failures: Vec<String>,
}

/// Progression globale du lot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    pub items_done: usize,
    pub items_total: usize,
    /// Octets reçus au total (audio et pochettes).
    pub bytes: u64,
}

/// Dossier de travail d'un lot, supprimé à la fin (N8 a) : [`Workdir::finish`] ou, à défaut,
/// à l'abandon (`Drop`, au mieux).
pub struct Workdir {
    path: PathBuf,
    armed: bool,
}

impl Workdir {
    /// Crée un sous-dossier unique de `base`.
    pub fn create(base: &Path) -> AppResult<Self> {
        let path = base.join(format!("podcast-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&path)?;
        Ok(Self { path, armed: true })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Supprime le dossier et son contenu (hors du thread asynchrone appelant) ; l'erreur
    /// éventuelle est renvoyée.
    pub async fn finish(mut self) -> AppResult<()> {
        self.armed = false;
        let path = std::mem::take(&mut self.path);
        let removed = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(path))
            .await
            .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?;
        match removed {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

impl Drop for Workdir {
    fn drop(&mut self) {
        if self.armed {
            let path = std::mem::take(&mut self.path);
            // Suppression hors du thread appelant (souvent un thread asynchrone) si possible.
            // TODO(#217) : à l'arrêt du runtime cette tâche peut ne jamais s'exécuter, et un
            // arrêt brutal ne passe pas ici : le dossier `podcast-*` reste orphelin. Le
            // nettoyage au démarrage de #217 (N10 a) doit supprimer ces dossiers ; à vérifier
            // dans #217.
            match tokio::runtime::Handle::try_current() {
                Ok(handle) => {
                    drop(handle.spawn_blocking(move || {
                        let _ = std::fs::remove_dir_all(path);
                    }));
                }
                Err(_) => {
                    let _ = std::fs::remove_dir_all(path);
                }
            }
        }
    }
}

/// Téléchargeur de lots : client gardé (D10) et politique réseau.
pub struct Fetcher {
    client: reqwest::Client,
    policy: NetPolicy,
}

impl Fetcher {
    pub fn new(policy: NetPolicy) -> AppResult<Self> {
        let client = guarded_client(policy, CONNECT_TIMEOUT, READ_TIMEOUT)
            .map_err(|_| AppError::Network("client réseau indisponible".into()))?;
        Ok(Self { client, policy })
    }

    /// Télécharge tous les `items` dans `dir` (dossier de travail dédié : les fichiers `ep-*`
    /// y sont supprimés en cas d'échec). Voir la documentation du module.
    pub async fn fetch_all(
        &self,
        items: &[Item],
        dir: &Workdir,
        limits: &Limits,
        mut cancel: watch::Receiver<bool>,
        on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> AppResult<Report> {
        if items.is_empty() {
            return Err(AppError::Validation("aucun épisode à télécharger".into()));
        }
        if items.iter().any(|i| looks_like_url(&i.id)) {
            // Jamais de valeur dans le message : ce pourrait être une URL secrète (D11).
            return Err(AppError::Validation(
                "identifiant d'épisode invalide (une URL n'est pas permise)".into(),
            ));
        }
        let dir = dir.path();
        if items.len() > limits.max_episodes {
            return Err(AppError::Validation(format!(
                "trop d'épisodes ({} au plus par pack)",
                limits.max_episodes
            )));
        }
        let (abort, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            client: self.client.clone(),
            policy: self.policy,
            dir: dir.to_path_buf(),
            limits: limits.clone(),
            total: AtomicU64::new(0),
            exceeded: AtomicBool::new(false),
            abort,
            done: AtomicUsize::new(0),
            items_total: items.len(),
            on_progress,
        });
        let semaphore = Arc::new(Semaphore::new(limits.concurrency.max(1)));
        let mut set = JoinSet::new();
        for (index, mut item) in items.iter().cloned().enumerate() {
            if looks_like_url(&item.feed) {
                item.feed = NEUTRAL_FEED_LABEL.to_string();
            }
            let shared = Arc::clone(&shared);
            let semaphore = Arc::clone(&semaphore);
            set.spawn(async move {
                let Ok(_permit) = semaphore.acquire_owned().await else {
                    return (index, Ok(ItemOutcome::Aborted));
                };
                if *shared.abort.borrow() {
                    return (index, Ok(ItemOutcome::Aborted));
                }
                let outcome = run_item(&shared, index, item).await;
                shared.done.fetch_add(1, Ordering::Relaxed);
                shared.report();
                (index, outcome)
            });
        }

        let mut outcomes: Vec<Option<ItemOutcome>> = (0..items.len()).map(|_| None).collect();
        let mut fatal: Option<AppError> = None;
        let mut user_cancelled = false;
        let mut batch_timed_out = false;
        let deadline = tokio::time::sleep(limits.batch_timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                joined = set.join_next() => match joined {
                    None => break,
                    Some(Ok((index, Ok(outcome)))) => outcomes[index] = Some(outcome),
                    Some(Ok((_, Err(err)))) => {
                        fatal.get_or_insert(err);
                        shared.abort.send_replace(true);
                    }
                    Some(Err(err)) => {
                        fatal.get_or_insert(AppError::Io(std::io::Error::other(err.to_string())));
                        shared.abort.send_replace(true);
                    }
                },
                () = download::cancelled(&mut cancel), if !user_cancelled => {
                    user_cancelled = true;
                    shared.abort.send_replace(true);
                }
                // Les tâches s'arrêtent sur `abort` ; la boucle continue jusqu'à leur fin pour
                // qu'aucune n'écrive après la suppression des fichiers.
                () = &mut deadline, if !batch_timed_out => {
                    batch_timed_out = true;
                    shared.abort.send_replace(true);
                }
            }
        }

        if user_cancelled
            || batch_timed_out
            || fatal.is_some()
            || shared.exceeded.load(Ordering::Relaxed)
        {
            remove_batch_files(dir).await;
            if let Some(err) = fatal {
                return Err(err);
            }
            if user_cancelled {
                return Err(AppError::Cancelled);
            }
            if batch_timed_out {
                return Err(AppError::DownloadInvalid("délai du lot dépassé".into()));
            }
            return Err(AppError::DownloadInvalid(format!(
                "le pack dépasse le plafond de {} octets au total",
                limits.max_total_bytes
            )));
        }

        let mut report = Report::default();
        for outcome in outcomes.into_iter().flatten() {
            match outcome {
                ItemOutcome::Fetched(fetched, image_failed) => {
                    if image_failed {
                        report.image_failures.push(fetched.id.clone());
                    }
                    report.fetched.push(fetched);
                }
                ItemOutcome::Dropped(dropped) => report.dropped.push(dropped),
                ItemOutcome::Aborted => {}
            }
        }
        if report.fetched.is_empty() {
            remove_batch_files(dir).await;
            return Err(AppError::DownloadInvalid(format!(
                "aucun épisode récupérable ({} retiré(s))",
                report.dropped.len()
            )));
        }
        Ok(report)
    }
}

struct Shared {
    client: reqwest::Client,
    policy: NetPolicy,
    dir: PathBuf,
    limits: Limits,
    total: AtomicU64,
    exceeded: AtomicBool,
    abort: watch::Sender<bool>,
    done: AtomicUsize,
    items_total: usize,
    on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
}

impl Shared {
    fn report(&self) {
        (self.on_progress)(Progress {
            items_done: self.done.load(Ordering::Relaxed),
            items_total: self.items_total,
            bytes: self.total.load(Ordering::Relaxed),
        });
    }
}

enum ItemOutcome {
    /// Épisode récupéré ; `true` si sa pochette a échoué.
    Fetched(Fetched, bool),
    Dropped(Dropped),
    Aborted,
}

/// Échec d'un fichier, hors erreurs fatales (disque…).
enum FileFailure {
    Aborted,
    Drop(DropReason),
    Fatal(AppError),
}

async fn run_item(shared: &Shared, index: usize, item: Item) -> AppResult<ItemOutcome> {
    let audio = shared.dir.join(format!("ep-{index:04}.audio"));
    let drop_with = |reason| {
        ItemOutcome::Dropped(Dropped {
            id: item.id.clone(),
            feed: item.feed.clone(),
            reason,
        })
    };
    let audio_bytes = match fetch_file(
        shared,
        &item.audio_url,
        &audio,
        shared.limits.max_episode_bytes,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(FileFailure::Aborted) => {
            discard(&audio).await;
            return Ok(ItemOutcome::Aborted);
        }
        Err(FileFailure::Drop(reason)) => {
            discard(&audio).await;
            tracing::warn!(episode = index, ?reason, "épisode retiré du pack");
            return Ok(drop_with(reason));
        }
        Err(FileFailure::Fatal(err)) => {
            discard(&audio).await;
            return Err(err);
        }
    };

    let mut image = None;
    let mut image_failed = false;
    if let Some(url) = &item.image_url {
        let path = shared.dir.join(format!("ep-{index:04}.img"));
        match fetch_file(shared, url, &path, shared.limits.max_image_bytes).await {
            Ok(_) => image = Some(path),
            Err(FileFailure::Aborted) => {
                discard(&audio).await;
                discard(&path).await;
                return Ok(ItemOutcome::Aborted);
            }
            Err(FileFailure::Drop(reason)) => {
                discard(&path).await;
                tracing::warn!(episode = index, ?reason, "pochette ignorée");
                image_failed = true;
            }
            Err(FileFailure::Fatal(err)) => {
                discard(&audio).await;
                discard(&path).await;
                return Err(err);
            }
        }
    }
    Ok(ItemOutcome::Fetched(
        Fetched {
            id: item.id,
            feed: item.feed,
            audio,
            audio_bytes,
            image,
        },
        image_failed,
    ))
}

/// Télécharge une URL vers `dest` : contrôle de l'adresse littérale, reprises, plafond du lot.
async fn fetch_file(
    shared: &Shared,
    url: &str,
    dest: &Path,
    max_bytes: u64,
) -> Result<u64, FileFailure> {
    let mut counted: u64 = 0;
    let result = fetch_file_counted(shared, url, dest, max_bytes, &mut counted).await;
    if result.is_err() {
        // Les octets d'un fichier retiré ne comptent plus dans le plafond du lot.
        shared.total.fetch_sub(counted, Ordering::Relaxed);
    }
    result
}

async fn fetch_file_counted(
    shared: &Shared,
    url: &str,
    dest: &Path,
    max_bytes: u64,
    counted: &mut u64,
) -> Result<u64, FileFailure> {
    let parsed = Url::parse(url).map_err(|_| FileFailure::Drop(DropReason::Invalid))?;
    if check_url(&parsed, shared.policy).is_err() {
        tracing::warn!(url = %redact_url(url), "adresse refusée (D10)");
        return Err(FileFailure::Drop(DropReason::Blocked));
    }
    tracing::debug!(url = %redact_url(url), "téléchargement");

    let started = tokio::time::Instant::now();
    let mut validator = Validator::default();
    let mut last_error = FileFailure::Drop(DropReason::Unreachable);
    for _ in 0..ATTEMPTS {
        let remaining = shared.limits.file_timeout.saturating_sub(started.elapsed());
        let mut cancel = shared.abort.subscribe();
        let request = Download {
            client: &shared.client,
            url: parsed.as_str(),
            dest,
            max_bytes,
            ignores_proxy: true,
        };
        // Octets reçus par cette tentative seulement (débit minimal) : le premier appel donne
        // la taille déjà présente du `.part` repris, qui ne doit pas payer la première fenêtre.
        let fresh = AtomicU64::new(0);
        let mut previous: Option<u64> = None;
        let mut on_progress = |received: u64, _total: Option<u64>| {
            if let Some(prev) = previous {
                // Une reprise refusée (416) repart de zéro : rien de nouveau à compter.
                fresh.fetch_add(received.saturating_sub(prev), Ordering::Relaxed);
            }
            previous = Some(received);
            if received >= *counted {
                shared
                    .total
                    .fetch_add(received - *counted, Ordering::Relaxed);
            } else {
                shared
                    .total
                    .fetch_sub(*counted - received, Ordering::Relaxed);
            }
            *counted = received;
            if shared.total.load(Ordering::Relaxed) > shared.limits.max_total_bytes {
                shared.exceeded.store(true, Ordering::Relaxed);
                shared.abort.send_replace(true);
            }
            shared.report();
        };
        let mut on_validator = |_: &Validator| {};
        let transfer = download::download(
            &request,
            &mut validator,
            &mut cancel,
            &mut on_validator,
            &mut on_progress,
        );
        let slow = too_slow(
            &fresh,
            shared.limits.min_rate_window,
            shared.limits.min_rate_bytes_per_sec,
        );
        let result = tokio::time::timeout(remaining, async {
            tokio::select! {
                r = transfer => Some(r),
                () = slow => None,
            }
        })
        .await;
        let error = match result {
            Ok(Some(Ok(()))) => return Ok(*counted),
            // Délai du fichier dépassé, ou débit sous le minimum.
            Err(_) | Ok(None) => return Err(FileFailure::Drop(DropReason::TimedOut)),
            Ok(Some(Err(error))) => error,
        };
        // Reprise seulement s'il reste des octets reçus (coupure). Si l'épisode est finalement
        // retiré, `fetch_file` retire ses octets du total du lot.
        let had_part = tokio::fs::metadata(download::part_path(dest))
            .await
            .is_ok_and(|m| m.len() > 0);
        let failure = match error {
            AppError::Cancelled => return Err(FileFailure::Aborted),
            AppError::Network(m) if m == BLOCKED_MESSAGE => {
                return Err(FileFailure::Drop(DropReason::Blocked))
            }
            AppError::Io(_) => return Err(FileFailure::Fatal(error)),
            AppError::DownloadInvalid(m) if m.contains("volumineu") => {
                return Err(FileFailure::Drop(DropReason::TooLarge))
            }
            AppError::DownloadInvalid(_) => FileFailure::Drop(DropReason::Invalid),
            _ => FileFailure::Drop(DropReason::Unreachable),
        };
        if !had_part {
            // Pas d'octets à reprendre (404, refus…) : inutile de réessayer.
            return Err(failure);
        }
        last_error = failure;
    }
    Err(last_error)
}

/// URL avec autorité (`schéma://…`) : refusée comme id ou libellé. Les URI sans hôte
/// (`tag:…`, `urn:uuid:…`) restent permises : ce sont des guid RSS courants, sans jeton réseau.
fn looks_like_url(value: &str) -> bool {
    Url::parse(value).is_ok_and(|u| u.has_host())
}

/// Se termine quand moins de `min_rate` octets/s ont été reçus sur une fenêtre `window`
/// (jamais si `window` est nulle). `received` : octets reçus par la tentative en cours, sans
/// ceux du `.part` repris.
async fn too_slow(received: &AtomicU64, window: Duration, min_rate: u64) {
    if window.is_zero() {
        std::future::pending::<()>().await;
    }
    let needed =
        u64::try_from(u128::from(min_rate) * window.as_millis() / 1000).unwrap_or(u64::MAX);
    let mut last = received.load(Ordering::Relaxed);
    loop {
        tokio::time::sleep(window).await;
        let now = received.load(Ordering::Relaxed);
        if now.saturating_sub(last) < needed {
            return;
        }
        last = now;
    }
}

async fn discard(path: &Path) {
    let _ = tokio::fs::remove_file(path).await;
    let _ = tokio::fs::remove_file(download::part_path(path)).await;
}

/// Supprime les fichiers `ep-*` du lot (annulation, plafond, échec global).
async fn remove_batch_files(dir: &Path) {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        if entry.file_name().to_string_lossy().starts_with("ep-") {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

#[cfg(test)]
mod tests;
