//! Création d'un pack de podcasts dans la file d'installation (#217, spécification
//! `docs/podcasts.md` § 5, A7, N7 a, N8 a, N10 a, P7, D1, D5, D11).
//!
//! Une tâche de pack suit les étapes `queued` → `preparing` → `prepared` → `converting` →
//! `installing` → `done` (ou `failed`, `cancelled`). La **préparation** produit l'archive Telmi
//! synthétique dans le dossier de travail de la tâche, puis le pipeline existant la convertit
//! (`telmi2flam`, un seul convertisseur à la fois) et la publie (`library.json`).
//!
//! # Invariants
//!
//! - **Point d'entrée** ([`pack_create`], N7 a) : la composition (identifiants seulement) est
//!   **revalidée entièrement** à partir des flux en cache ; invalide, aucune tâche n'est créée.
//!   L'uuid du pack est tiré **une seule fois**, à la création de la tâche, et enregistré avec la
//!   composition dans la file persistée ([`PackJob`]).
//! - **Reprise** (N10 a) : une tâche relue au démarrage garde son uuid et sa composition ; la
//!   préparation repart d'un dossier de travail vidé et **revalide** les épisodes (un épisode
//!   disparu donne un échec propre, rien n'est publié).
//! - **Dossier de travail** `<cache>/podcast-work/<tâche>/` : supprimé hors du runtime
//!   asynchrone à la fin de la tâche, quelle qu'en soit l'issue (N8 a) ; au démarrage, les
//!   dossiers qui n'appartiennent à aucune tâche connue sont supprimés ([`sweep_workdirs`]).
//! - **URL jamais journalisées** (D11) : les adresses d'enclosure et d'image sont relues dans le
//!   cache et passées au téléchargeur ; les messages ne citent que des titres, rangs et uuid de
//!   source.
//! - **Plafonds sur les tailles réelles** : total téléchargé (D1, 1 Gio), MP3 produits (bornes de
//!   l'archive) ; un épisode trop gros est retiré (P7), un total dépassé fait échouer la tâche.
//! - **Espace disque** (D5) vérifié avant tout téléchargement.
//! - **Cache des MP3 convertis** (#460, N8 b, [`mp3cache`]) : un épisode déjà converti est
//!   repris du cache (ni téléchargé ni reconverti), un épisode converti y est mis avant la
//!   suppression du dossier de travail. Le cache est au mieux : ses erreurs ne font jamais
//!   échouer le pack.
//! - **Titres lus** (#462, § 12, [`pack_speech`](super::pack_speech)) : si la composition le
//!   demande, la voix est contrôlée à la création de la tâche (aucune voix française : refus),
//!   puis les titres sont synthétisés **avant les téléchargements** ; titre isolé en échec :
//!   muet et avertissement (`PackStatus::warnings`) ; premier titre ou plus de la moitié en
//!   échec : arrêt `tts_failed`.
//! - `library.json` n'est jamais touché ici : seule la publication existante l'écrit.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::watch;

use super::pack_speech::{self, SpokenTitles, TitleKey};
use super::{child_of, converter_engine, is_task_id, lock, InstallService, TaskState, Toolchain};
use crate::catalog::blocking;
use crate::ffmpeg::Verified;
use crate::podcast::archive::{
    self, MenuEntry, Pack, PackEpisode, PackMetadata, MAX_AUDIO_BYTES, MAX_TOTAL_AUDIO_BYTES,
};
use crate::podcast::artwork::{self, Artwork, ArtworkError, ArtworkKind};
use crate::podcast::fetch::{self, Item, Progress, Report, Workdir};
use crate::podcast::mp3cache::{self, CacheAccess, EntryKey, Hit, Wanted};
use crate::podcast::pack::{
    self, CachedFeed, CoverChoice, EpisodeRef, Feeds, PackComposition, PackNode, ResolvedMetadata,
    ValidationReport, MAX_PACK_BYTES,
};
use crate::podcast::rss::PodcastEpisode;
use crate::podcast::source::{episode_id, PodcastService};
use crate::podcast::speech::{self, Synthesizer, Wav};
use crate::storage::ConfigState;
use crate::story::{StoryEntry, StoryKind};
use crate::{AppError, AppResult};

/// Sous-dossier du cache des dossiers de travail (listé dans `cache::APP_CACHE_SUBDIRS`).
pub const WORK_DIR: &str = "podcast-work";
/// `source_id` de l'entrée d'un pack : aucune source réelle (P3, P4).
pub const PACK_SOURCE_ID: &str = "podcast-pack";
/// Version fixe d'un pack (P3).
pub const PACK_VERSION: u32 = 1;
/// Archive Telmi, dans le dossier de travail.
const ARCHIVE_NAME: &str = "pack.zip";
/// D5 : facteur d'espace disque (originaux, MP3, archive, `.plain.pk`, § 10).
const DISK_FACTOR: u64 = 4;
/// D5 : taille comptée pour un épisode sans taille ni durée annoncées.
const UNKNOWN_EPISODE_BYTES: u64 = 50_000_000;
/// D5 : marge fixe.
const DISK_MARGIN: u64 = 200_000_000;
/// Délai du téléchargement de la couverture.
const COVER_TIMEOUT: Duration = Duration::from_secs(120);

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Composition et uuid d'un pack, enregistrés avec la tâche (N10 a).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackJob {
    /// Uuid du pack, forme canonique (tiré une seule fois, à la création de la tâche).
    pub uuid: String,
    pub composition: PackComposition,
    /// Révision du brouillon (#362) à effacer après la publication.
    #[serde(default)]
    pub draft_revision: Option<u64>,
}

/// État d'un pack exposé au front (miroir de `PackStatus` dans `src/api/install.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct PackStatus {
    /// Épisodes de la composition.
    pub episodes: usize,
    /// Titres des épisodes retirés du pack (P7), connus après la préparation.
    pub dropped: Vec<String>,
    /// Avertissements des titres lus (#462) : voix remplacée par la voix par défaut, titres
    /// restés muets. Vide sans titres lus.
    pub warnings: Vec<String>,
}

/// Accès externes de la préparation (production : [`LivePackBackend`] ; tests : injection de
/// pannes).
pub trait PackBackend: Send + Sync {
    /// Prérequis (ffmpeg) : vérifiés avant tout téléchargement.
    fn ready(&self) -> AppResult<()>;
    /// Flux en cache des sources citées par la composition (sans réseau).
    fn feeds<'a>(&'a self, composition: &'a PackComposition) -> BoxFuture<'a, AppResult<Feeds>>;
    /// Espace libre du volume de `dir` (`None` : inconnu, non bloquant).
    fn free_space(&self, dir: &Path) -> Option<u64>;
    /// Téléchargement du lot (#213), `concurrency` fichiers à la fois.
    fn fetch<'a>(
        &'a self,
        items: &'a [Item],
        dir: &'a Workdir,
        concurrency: usize,
        cancel: watch::Receiver<bool>,
        on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> BoxFuture<'a, AppResult<Report>>;
    /// Téléchargement d'une image isolée (couverture) ; `false` si elle est inaccessible.
    fn fetch_image<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<bool>>;
    /// Conversion audio (#214) : taille de chaque MP3 produit, ou son erreur (P7).
    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>>;
    /// Pack publié : efface le brouillon s'il n'a pas changé depuis `revision`.
    fn draft_done(&self, revision: u64);
    /// Cache des MP3 convertis (#460) des `sources`, sous `cache_root` ; `None` : sans cache.
    /// Appelé **avant** le chargement des flux (générations des sources, voir
    /// [`CacheAccess::new`]).
    fn mp3_cache(&self, _cache_root: PathBuf, _sources: BTreeSet<String>) -> Option<CacheAccess> {
        None
    }
    /// Conversion d'une pochette (#215), bloquante. Par défaut **dans un processus enfant**
    /// (#263, [`artwork::convert_isolated`]) : un décodeur qui panique sur une image hostile
    /// n'arrête pas le launcher ; l'échec donne l'image de repli (D13).
    fn convert_artwork(&self, bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
        artwork::convert_isolated(bytes, kind, None)
    }
    /// Moteur de synthèse des titres lus (#462) : voix Windows par défaut.
    fn speech(&self) -> Arc<dyn Synthesizer> {
        speech::system()
    }
    /// Voix mémorisée du poste (réglage `tts_voice`), bloquant ; `None` : voix par défaut.
    fn preferred_voice(&self) -> Option<String> {
        None
    }
    /// Délai de synthèse d'un titre (injectable en test).
    fn synth_timeout(&self) -> Duration {
        speech::SYNTH_TIMEOUT
    }
    /// Convertisseur des titres lus pour toute l'étape (#487) : ffmpeg embarqué localisé et
    /// empreinte vérifiée **une seule fois**, au premier titre synthétisé.
    fn title_encoder<'a>(
        &'a self,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Verified>> {
        Box::pin(async move {
            let ff = crate::ffmpeg::Ffmpeg::locate(&crate::settings::Environment::current())?;
            let mut cancel = cancel.clone();
            ff.verify(crate::ffmpeg::Tool::Ffmpeg, &mut cancel).await
        })
    }
    /// Conversion d'un titre synthétisé en MP3 `dest` par `encoder` ; taille du MP3.
    fn title_mp3<'a>(
        &'a self,
        encoder: &'a Verified,
        wav: &'a Wav,
        dest: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<u64>> {
        Box::pin(async move {
            let mut cancel = cancel.clone();
            speech::wav_to_mp3_verified(encoder, wav, dest, &mut cancel).await
        })
    }
}

/// Accès réels : état Tauri (configuration, flux en cache), réseau gardé (D10), ffmpeg.
pub struct LivePackBackend {
    pub app: tauri::AppHandle,
}

impl PackBackend for LivePackBackend {
    fn ready(&self) -> AppResult<()> {
        crate::ffmpeg::Ffmpeg::locate(&crate::settings::Environment::current()).map(|_| ())
    }

    fn feeds<'a>(&'a self, composition: &'a PackComposition) -> BoxFuture<'a, AppResult<Feeds>> {
        Box::pin(async move {
            use tauri::Manager;
            let config = self.app.state::<ConfigState>();
            let podcasts = self.app.state::<PodcastService>();
            pack::load_feeds(&config, &podcasts, composition).await
        })
    }

    fn free_space(&self, dir: &Path) -> Option<u64> {
        crate::device::detect::free_space(dir)
    }

    fn fetch<'a>(
        &'a self,
        items: &'a [Item],
        dir: &'a Workdir,
        concurrency: usize,
        cancel: watch::Receiver<bool>,
        on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> BoxFuture<'a, AppResult<Report>> {
        Box::pin(async move {
            let limits = fetch::Limits {
                concurrency,
                ..fetch::Limits::default()
            };
            let fetcher = fetch::Fetcher::new(crate::podcast::net::NetPolicy::strict())?;
            fetcher
                .fetch_all(items, dir, &limits, cancel, on_progress)
                .await
        })
    }

    fn fetch_image<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(download_image(
            crate::podcast::net::NetPolicy::strict(),
            url,
            dest,
            cancel,
        ))
    }

    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>> {
        Box::pin(async move {
            let ff = crate::ffmpeg::Ffmpeg::locate(&crate::settings::Environment::current())?;
            Ok(crate::podcast::audio::convert_all(&ff, items, cancel)
                .await
                .into_iter()
                .map(|r| r.map(|c| c.output_bytes))
                .collect())
        })
    }

    fn draft_done(&self, revision: u64) {
        use tauri::Manager;
        let config = self.app.state::<ConfigState>();
        let _guard = config.guard();
        if let Err(err) = clear_draft_if_unchanged(&config.dir, revision) {
            tracing::warn!("brouillon de pack non effacé : {err}");
        }
    }

    fn mp3_cache(&self, cache_root: PathBuf, sources: BTreeSet<String>) -> Option<CacheAccess> {
        Some(CacheAccess::new(
            Arc::new(mp3cache::LiveLocks(self.app.clone())),
            cache_root,
            sources,
        ))
    }

    fn preferred_voice(&self) -> Option<String> {
        use tauri::Manager;
        let config = self.app.state::<ConfigState>();
        let _guard = config.guard();
        crate::settings::SettingsStore::new(&config.dir)
            .load()
            .ok()?
            .tts_voice
    }
}

/// Efface le brouillon de `dir` si sa révision n'a pas avancé depuis la création du pack : un
/// brouillon commencé ensuite n'est jamais effacé.
pub fn clear_draft_if_unchanged(dir: &Path, revision: u64) -> AppResult<()> {
    let draft = pack::read_draft(dir)?;
    if draft.composition.is_some() && draft.revision <= revision {
        pack::clear_draft(dir, revision)?;
    }
    Ok(())
}

/// Télécharge une image (couverture) par le client gardé (D10). `Ok(false)` si elle est
/// inaccessible ; seule une erreur disque ou l'annulation remontent.
pub(crate) async fn download_image(
    policy: crate::podcast::net::NetPolicy,
    url: &str,
    dest: &Path,
    mut cancel: watch::Receiver<bool>,
) -> AppResult<bool> {
    use crate::install::download::{self, Download, Validator};
    let Ok(parsed) = url::Url::parse(url) else {
        return Ok(false);
    };
    if crate::podcast::net::check_url(&parsed, policy).is_err() {
        return Ok(false);
    }
    let client = crate::podcast::net::guarded_client(
        policy,
        Duration::from_secs(15),
        Duration::from_secs(60),
    )
    .map_err(|_| AppError::Network("client réseau indisponible".into()))?;
    let request = Download {
        client: &client,
        url: parsed.as_str(),
        dest,
        max_bytes: fetch::MAX_IMAGE_BYTES,
        ignores_proxy: true,
    };
    let mut validator = Validator::default();
    let result = tokio::time::timeout(
        COVER_TIMEOUT,
        download::download(
            &request,
            &mut validator,
            &mut cancel,
            &mut |_| {},
            &mut |_, _| {},
        ),
    )
    .await;
    let _ = tokio::fs::remove_file(download::part_path(dest)).await;
    match result {
        Ok(Ok(())) => Ok(true),
        Ok(Err(AppError::Cancelled)) => Err(AppError::Cancelled),
        Ok(Err(AppError::Io(err))) => Err(AppError::Io(err)),
        _ => {
            tracing::warn!("couverture du pack inaccessible : image de repli");
            Ok(false)
        }
    }
}

/// Entrée de bibliothèque d'un pack (P3 : uuid du pack, version fixe, aucune source réelle).
pub fn pack_entry(uuid: &str, meta: &ResolvedMetadata, archive: &Path) -> StoryEntry {
    StoryEntry {
        source_id: PACK_SOURCE_ID.into(),
        entry_id: format!("pack:{uuid}"),
        uuid: uuid.to_string(),
        version: Some(PACK_VERSION),
        title: meta.title.clone(),
        description: None,
        age: Some(u32::from(meta.age)),
        category: Some(meta.subtitle.clone()).filter(|s| !s.is_empty()),
        author: None,
        voice: None,
        license: None,
        thumb_url: None,
        // Archive du dossier de travail : la conversion la lit, puis elle est supprimée (N8 a).
        // Le lien recopié dans `library.json` (`LibraryEntry::source`) est donc **mort** après
        // la publication : sans usage pour un pack (`PACK_SOURCE_ID` n'est pas une source).
        download_url: archive.display().to_string(),
        size: None,
        updated_at: None,
        download_count: None,
        kind: StoryKind::TelmiZip,
    }
}

/// Dossier de travail de la tâche `id`.
pub fn workdir_path(cache_root: &Path, id: &str) -> AppResult<PathBuf> {
    child_of(&cache_root.join(WORK_DIR), id)
}

/// Archive Telmi du pack de la tâche `id`.
pub fn archive_path(cache_root: &Path, id: &str) -> AppResult<PathBuf> {
    Ok(workdir_path(cache_root, id)?.join(ARCHIVE_NAME))
}

/// Supprime les dossiers de travail orphelins (arrêt brutal : le `Drop` de [`Workdir`] ne
/// s'exécute pas à l'arrêt du runtime). Bloquant.
///
/// Ordre sûr : le dossier est **lu d'abord**, puis `orphans` (appelé une fois, sous le verrou
/// des tâches) garde les noms qui n'appartiennent à aucune tâche ; la suppression se fait hors
/// verrou. Le dossier d'une tâche n'est créé qu'après son insertion dans la liste : un nom lu
/// avant le relevé appartient soit à une tâche déjà listée (gardé), soit à aucune (une tâche
/// créée ensuite a un identifiant neuf).
pub fn sweep_workdirs(cache_root: &Path, orphans: &dyn Fn(Vec<String>) -> Vec<String>) {
    let work = cache_root.join(WORK_DIR);
    let Ok(entries) = std::fs::read_dir(&work) else {
        return;
    };
    let names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    for name in orphans(names) {
        let Ok(path) = child_of(&work, &name) else {
            continue;
        };
        let result = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if let Err(err) = result {
            tracing::warn!("dossier de travail orphelin {name} non supprimé : {err}");
        }
    }
}

/// Résultat de la préparation : archive écrite, entrée à publier, épisodes retirés.
pub(super) struct Prepared {
    pub workdir: Workdir,
    pub archive: PathBuf,
    pub entry: StoryEntry,
}

/// Épisode retenu de la composition, avec son flux.
struct Chosen<'a> {
    reference: &'a EpisodeRef,
    episode: &'a PodcastEpisode,
}

/// Message d'un rapport invalide (titres et nombres seulement).
fn invalid_message(report: &ValidationReport) -> String {
    let details: Vec<&str> = report
        .problems
        .iter()
        .take(3)
        .map(|p| p.message.as_str())
        .collect();
    format!("composition du pack invalide : {}", details.join(" ; "))
}

/// Épisodes servis par le cache des MP3 (#460), par rang, placés dans le dossier de travail
/// sous des noms distincts de ceux du téléchargement (`cache-<rang>.mp3`, `cache-<rang>.img`).
async fn lookup_cache(
    cache: &CacheAccess,
    keys: &[EntryKey],
    dir: &Path,
    now: u64,
) -> HashMap<usize, Hit> {
    let wanted = keys
        .iter()
        .enumerate()
        .map(|(n, key)| Wanted {
            rank: n,
            key: key.clone(),
            mp3_dest: dir.join(format!("cache-{n:04}.mp3")),
            image_dest: dir.join(format!("cache-{n:04}.img")),
        })
        .collect();
    cache
        .lookup(wanted, now)
        .await
        .into_iter()
        .map(|h| (h.rank, h))
        .collect()
}

/// Télécharge les images `(rang, URL, destination)`, `concurrency` à la fois ; rend celles
/// obtenues, par rang. La première erreur (disque, annulation) arrête les autres.
async fn fetch_images(
    backend: &Arc<dyn PackBackend>,
    jobs: Vec<(usize, String, PathBuf)>,
    concurrency: usize,
    cancel: &watch::Receiver<bool>,
) -> AppResult<Vec<(usize, PathBuf)>> {
    let mut jobs = jobs.into_iter();
    let mut running = tokio::task::JoinSet::new();
    let mut fetched = Vec::new();
    loop {
        while running.len() < concurrency.max(1) {
            let Some((n, url, dest)) = jobs.next() else {
                break;
            };
            let (backend, cancel) = (backend.clone(), cancel.clone());
            running.spawn(async move {
                let ok = backend.fetch_image(&url, &dest, cancel).await?;
                AppResult::Ok(ok.then_some((n, dest)))
            });
        }
        // `running` abandonné sur erreur : les téléchargements restants sont interrompus.
        let Some(done) = running.join_next().await else {
            break;
        };
        let done = match done {
            Ok(done) => done,
            // Panique d'un téléchargement : propagée comme sans parallélisme.
            Err(err) if err.is_panic() => std::panic::resume_unwind(err.into_panic()),
            Err(_) => {
                return Err(AppError::Io(std::io::Error::other(
                    "téléchargement interrompu",
                )))
            }
        };
        fetched.extend(done?);
    }
    fetched.sort_unstable();
    Ok(fetched)
}

/// Supprime le dossier de travail (hors du runtime asynchrone : `Workdir::finish`).
pub(super) async fn finish_workdir(workdir: Workdir) {
    if let Err(err) = workdir.finish().await {
        tracing::warn!("dossier de travail non supprimé : {err}");
    }
}

/// Message d'un lot arrêté par `fetch::Limits::batch_timeout` : dit clairement à l'utilisateur
/// ce qui s'est passé et quoi faire. Les autres erreurs passent telles quelles.
pub(super) fn explain_fetch_error(err: AppError) -> AppError {
    match err {
        AppError::DownloadInvalid(m) if m.contains("délai du lot") => {
            AppError::DownloadInvalid(format!(
                "téléchargement du pack interrompu : {m} ({} h au plus ; connexion trop lente ou                  serveur qui ne répond plus). Réessayez plus tard ou avec moins d'épisodes.",
                fetch::BATCH_TIMEOUT.as_secs() / 3600
            ))
        }
        other => other,
    }
}

impl InstallService {
    /// Ajoute une tâche de pack pour une composition **déjà validée** (`report.valid`).
    pub async fn start_pack(
        &self,
        composition: PackComposition,
        report: &ValidationReport,
        draft_revision: Option<u64>,
    ) -> AppResult<super::InstallTask> {
        let metadata = match (&report.metadata, report.valid) {
            (Some(m), true) => m,
            _ => return Err(AppError::Validation(invalid_message(report))),
        };
        let mut composition = composition;
        if composition.titles_audio {
            self.check_speech(&mut composition).await?;
        } else {
            // Sans titres lus, la voix ne sert à rien : jamais enregistrée avec la tâche.
            composition.titles_voice = None;
        }
        let tc = self.toolchain().await?;
        let id = uuid::Uuid::new_v4().to_string();
        let uuid = archive::new_pack_uuid().hyphenated().to_string();
        let archive = archive_path(&tc.cache_root, &id)?;
        let entry = pack_entry(&uuid, metadata, &archive);
        let job = PackJob {
            uuid,
            composition,
            draft_revision,
        };
        self.enqueue(id, entry, Some(job), super::now_secs())
    }

    /// Titres lus demandés (#462) : voix normalisée, voix mémorisée du poste reprise si la
    /// composition n'en choisit pas (enregistrée avec la tâche : la reprise utilise la même), et
    /// au moins une voix française présente. Sinon refus, aucune tâche créée.
    async fn check_speech(&self, composition: &mut PackComposition) -> AppResult<()> {
        let backend = lock(&self.0.packs).clone().ok_or_else(|| {
            AppError::ConverterUnavailable("création de pack indisponible".into())
        })?;
        let requested = crate::settings::clean_voice_id(composition.titles_voice.take())?;
        let voice = blocking(move || {
            let requested = requested.or_else(|| {
                backend
                    .preferred_voice()
                    .and_then(|v| crate::settings::clean_voice_id(Some(v)).ok().flatten())
            });
            pack_speech::choose_voice(&backend.speech(), requested.as_deref())?;
            Ok::<_, AppError>(requested)
        })
        .await??;
        composition.titles_voice = voice;
        Ok(())
    }

    /// Préparation (#217) : revalidation, D5, téléchargements, audio, tailles réelles,
    /// pochettes, archive. Le dossier de travail rendu est à supprimer par l'appelant ; en cas
    /// d'erreur, il est supprimé ici.
    pub(super) async fn prepare_pack(
        &self,
        id: &str,
        job: &PackJob,
        tc: &Toolchain,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<Prepared> {
        // Une préparation à la fois (attente annulable) : D5 n'est pas compté deux fois, le
        // réseau et ffmpeg restent bornés.
        let _slot = tokio::select! {
            s = self.0.preparations.acquire() => s.map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?,
            _ = super::download::cancelled(cancel) => return Err(AppError::Cancelled),
        };
        // Dossier de travail d'abord (vidé s'il reste d'un arrêt brutal) : toute erreur qui
        // suit passe par sa suppression.
        let dir = workdir_path(&tc.cache_root, id)?;
        let workdir = blocking(move || Workdir::at(dir)).await??;
        match self.prepare_checked(id, job, tc, &workdir, cancel).await {
            Ok((archive, entry)) => Ok(Prepared {
                workdir,
                archive,
                entry,
            }),
            Err(err) => {
                finish_workdir(workdir).await;
                Err(err)
            }
        }
    }

    async fn prepare_checked(
        &self,
        id: &str,
        job: &PackJob,
        tc: &Toolchain,
        workdir: &Workdir,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<(PathBuf, StoryEntry)> {
        let backend = lock(&self.0.packs).clone().ok_or_else(|| {
            AppError::ConverterUnavailable("création de pack indisponible".into())
        })?;
        // Prérequis avant tout téléchargement : le pack ne pourrait pas être converti par le
        // convertisseur (`telmi2flam.exe` absent) (#440).
        converter_engine(tc)?;
        {
            let backend = backend.clone();
            blocking(move || backend.ready()).await??;
        }
        self.emit_phase(id, 0.0, "validation");

        // Cache des MP3 (#460) : générations des sources lues avant le chargement des flux
        // (une source supprimée ensuite n'est jamais recréée sur le disque).
        let cache = backend
            .mp3_cache(tc.cache_root.clone(), job.composition.source_ids())
            .map(|c| c.with_cancel(cancel.clone()));
        // Revalidation complète (reprise : flux changé entre-temps).
        let feeds = backend.feeds(&job.composition).await?;
        let report = pack::validate(&job.composition, &feeds);
        let metadata = match (&report.metadata, report.valid) {
            (Some(m), true) => m.clone(),
            _ => return Err(AppError::Validation(invalid_message(&report))),
        };
        // D5 : espace disque avant de commencer.
        let estimate = report
            .estimated_bytes
            .saturating_add((report.unknown_size as u64).saturating_mul(UNKNOWN_EPISODE_BYTES));
        // Les packs préparés en attente du convertisseur produiront aussi leur `.plain.pk`.
        let reserved = self
            .0
            .reserved_bytes
            .load(std::sync::atomic::Ordering::SeqCst);
        let needed = estimate
            .saturating_mul(DISK_FACTOR)
            .saturating_add(DISK_MARGIN)
            .saturating_add(reserved);
        let free = {
            let (backend, root) = (backend.clone(), tc.cache_root.clone());
            blocking(move || backend.free_space(&root)).await?
        };
        if let Some(free) = free {
            if free < needed {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    format!(
                        "espace disque insuffisant pour créer le pack : {} Mo nécessaires, {} Mo libres",
                        needed.div_ceil(1_000_000),
                        free / 1_000_000
                    ),
                )));
            }
        }
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        let concurrency = tc.max_downloads.clamp(1, 4);
        self.prepare_in(
            id,
            job,
            &feeds,
            &report,
            metadata,
            workdir,
            &backend,
            cache.as_ref(),
            concurrency,
            cancel,
        )
        .await
    }

    fn emit_phase(&self, id: &str, percent: f64, phase: &str) {
        self.emit(super::InstallProgress {
            task_id: id.to_string(),
            state: TaskState::Preparing,
            percent: Some(percent),
            received: None,
            total: None,
            phase: Some(phase.to_string()),
        });
    }

    #[allow(clippy::too_many_arguments)]
    async fn prepare_in(
        &self,
        id: &str,
        job: &PackJob,
        feeds: &Feeds,
        report: &ValidationReport,
        metadata: ResolvedMetadata,
        workdir: &Workdir,
        backend: &Arc<dyn PackBackend>,
        cache: Option<&CacheAccess>,
        concurrency: usize,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<(PathBuf, StoryEntry)> {
        // Épisodes retrouvés dans les flux en cache (validation réussie : tous présents).
        let index: HashMap<(&str, String), &PodcastEpisode> = feeds
            .iter()
            .filter_map(|(sid, f)| f.ready().map(|f| (sid, f)))
            .flat_map(|(sid, f)| {
                f.episodes
                    .iter()
                    .map(move |ep| ((sid.as_str(), episode_id(&ep.guid)), ep))
            })
            .collect();
        let mut chosen: Vec<Chosen<'_>> = Vec::new();
        for reference in job.composition.episodes() {
            let episode = index
                .get(&(reference.source_id.as_str(), reference.episode_id.clone()))
                .copied()
                .ok_or_else(|| AppError::Validation("épisode introuvable dans le flux".into()))?;
            chosen.push(Chosen { reference, episode });
        }
        self.set_pack_status(id, chosen.len(), Vec::new());

        // Titres lus (#462) : avant tout téléchargement, pour qu'une voix inutilisable arrête
        // la création sans rien télécharger.
        let (spoken, mut warnings) = if job.composition.titles_audio {
            self.speak_titles(id, job, &chosen, &metadata, workdir, backend, cancel)
                .await?
        } else {
            (SpokenTitles::default(), Vec::new())
        };

        // 0. Cache des MP3 convertis (#460) : un épisode servi n'est ni téléchargé ni converti.
        let now = super::now_secs();
        let keys: Vec<EntryKey> = chosen
            .iter()
            .map(|c| EntryKey::of(&c.reference.source_id, c.episode))
            .collect();
        let hits = match cache {
            Some(cache) => lookup_cache(cache, &keys, workdir.path(), now).await,
            None => HashMap::new(),
        };
        if !hits.is_empty() {
            tracing::info!(
                hits = hits.len(),
                "pack : épisodes servis par le cache des MP3"
            );
        }
        // `id` : rang ; `feed` : uuid de la source. Jamais d'URL ni de titre (D11).
        let items: Vec<Item> = chosen
            .iter()
            .enumerate()
            .filter(|(n, _)| !hits.contains_key(n))
            .map(|(n, c)| Item {
                id: format!("e{n}"),
                feed: c.reference.source_id.clone(),
                audio_url: c.episode.enclosure_url.clone(),
                image_url: c.episode.image.clone(),
            })
            .collect();

        // 1. Téléchargements (0 à 60 %).
        self.emit_phase(id, 0.0, "download");
        let progress: Arc<dyn Fn(Progress) + Send + Sync> = {
            let service = self.clone();
            let id = id.to_string();
            let last = Arc::new(std::sync::Mutex::new(None::<std::time::Instant>));
            Arc::new(move |p: Progress| {
                let mut last = lock(&last);
                let end = p.items_done == p.items_total;
                if end || last.is_none_or(|t| t.elapsed() >= super::PROGRESS_INTERVAL) {
                    *last = Some(std::time::Instant::now());
                    let part = p.items_done as f64 / p.items_total.max(1) as f64;
                    service.emit(super::InstallProgress {
                        task_id: id.clone(),
                        state: TaskState::Preparing,
                        percent: Some(part * 60.0),
                        received: Some(p.bytes),
                        total: None,
                        phase: Some("download".into()),
                    });
                }
            })
        };
        let fetched = if items.is_empty() {
            // Tout vient du cache.
            Report::default()
        } else {
            backend
                .fetch(&items, workdir, concurrency, cancel.clone(), progress)
                .await
                .map_err(explain_fetch_error)?
        };
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        let mut dropped: Vec<usize> = Vec::new();
        let rank = |item_id: &str| -> Option<usize> { item_id.strip_prefix('e')?.parse().ok() };
        for d in &fetched.dropped {
            if let Some(n) = rank(&d.id) {
                dropped.push(n);
            }
        }
        // Plafond D1 sur les tailles réelles (le front compte 0 les tailles inconnues).
        let total: u64 = fetched.fetched.iter().map(|f| f.audio_bytes).sum();
        if total > MAX_PACK_BYTES {
            return Err(AppError::DownloadInvalid(format!(
                "le pack dépasse 1 Gio une fois téléchargé ({} Mo)",
                total.div_ceil(1_000_000)
            )));
        }

        // Pochettes des épisodes servis par le cache qui n'y ont pas la leur (absente ou
        // périmée) : téléchargées `concurrency` à la fois (D13 : un échec donne l'image de
        // repli).
        let mut hit_images: BTreeMap<usize, PathBuf> = BTreeMap::new();
        let mut to_cache: Vec<mp3cache::Store> = Vec::new();
        let mut hit_ranks: Vec<usize> = hits.keys().copied().collect();
        hit_ranks.sort_unstable();
        let mut missing: Vec<(usize, String, PathBuf)> = Vec::new();
        for &n in &hit_ranks {
            match (&hits[&n].image, &chosen[n].episode.image) {
                (Some(path), _) => {
                    hit_images.insert(n, path.clone());
                }
                (None, Some(url)) => {
                    let dest = workdir.path().join(format!("cache-{n:04}.img"));
                    missing.push((n, url.clone(), dest));
                }
                (None, None) => {}
            }
        }
        for (n, dest) in fetch_images(backend, missing, concurrency, cancel).await? {
            // Remise en cache avec la nouvelle pochette (le MP3 est relié).
            to_cache.push(mp3cache::Store {
                key: keys[n].clone(),
                mp3: hits[&n].mp3.clone(),
                image: Some(dest.clone()),
            });
            hit_images.insert(n, dest);
        }
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }

        // 2. Audio (60 à 85 %).
        self.emit_phase(id, 60.0, "audio");
        let mut ready: Vec<(usize, PathBuf, Option<PathBuf>)> = Vec::new();
        let mut conversions = Vec::new();
        for f in &fetched.fetched {
            let Some(n) = rank(&f.id) else { continue };
            let mp3 = workdir.path().join(format!("ep-{n:04}.mp3"));
            conversions.push((f.audio.clone(), mp3.clone()));
            ready.push((n, mp3, f.image.clone()));
        }
        let results = if conversions.is_empty() {
            Vec::new()
        } else {
            backend.convert_audio(conversions, cancel).await?
        };
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        let mut kept: Vec<(usize, PathBuf, Option<PathBuf>)> = Vec::new();
        let mut audio_total = 0u64;
        for &n in &hit_ranks {
            let hit = &hits[&n];
            if hit.mp3_bytes <= MAX_AUDIO_BYTES {
                audio_total += hit.mp3_bytes;
                kept.push((n, hit.mp3.clone(), hit_images.get(&n).cloned()));
            } else {
                tracing::warn!(
                    episode = n,
                    "épisode retiré du pack : MP3 en cache trop volumineux"
                );
                dropped.push(n);
            }
        }
        for ((n, mp3, image), result) in ready.into_iter().zip(results) {
            match result {
                Ok(bytes) if bytes <= MAX_AUDIO_BYTES => {
                    audio_total += bytes;
                    to_cache.push(mp3cache::Store {
                        key: keys[n].clone(),
                        mp3: mp3.clone(),
                        image: image.clone(),
                    });
                    kept.push((n, mp3, image));
                }
                Ok(_) => {
                    tracing::warn!(episode = n, "épisode retiré du pack : MP3 trop volumineux");
                    dropped.push(n);
                }
                Err(AppError::Cancelled) => return Err(AppError::Cancelled),
                Err(err) => {
                    tracing::warn!(episode = n, "épisode retiré du pack : {}", err.kind());
                    dropped.push(n);
                }
            }
        }
        // Mise en cache des MP3 convertis, avant que les pochettes d'origine ne soient
        // supprimées du dossier de travail ; puis éviction (jamais les épisodes de ce pack).
        if let Some(cache) = cache {
            if !to_cache.is_empty() {
                cache.store(to_cache, now).await;
            }
            let protect: HashSet<(String, String)> = keys
                .iter()
                .map(|k| (k.source_id.clone(), k.episode_id.clone()))
                .collect();
            cache.evict(protect).await;
        }
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        if kept.is_empty() {
            let titles: Vec<String> = dropped
                .iter()
                .take(3)
                .filter_map(|&n| chosen.get(n).map(|c| archive::label(&c.episode.title, "?")))
                .collect();
            let more = if dropped.len() > 3 { ", …" } else { "" };
            return Err(AppError::DownloadInvalid(format!(
                "aucun épisode utilisable (téléchargement ou conversion en échec) : {}{more}",
                titles.join(", ")
            )));
        }
        if audio_total > MAX_TOTAL_AUDIO_BYTES {
            return Err(AppError::DownloadInvalid(format!(
                "audio converti trop volumineux : {} Mo",
                audio_total.div_ceil(1_000_000)
            )));
        }
        dropped.sort_unstable();
        dropped.dedup();
        let dropped_titles: Vec<String> = dropped
            .iter()
            .filter_map(|&n| chosen.get(n).map(|c| c.episode.title.clone()))
            .collect();
        self.set_pack_status(id, chosen.len(), dropped_titles.clone());

        // 3. Pochettes (85 à 90 %) : une image illisible ne retire pas l'épisode (D13).
        self.emit_phase(id, 85.0, "images");
        let cover_url: Option<String> = match &report.cover {
            Some(CoverChoice::Feed { source_id }) => feeds
                .get(source_id)
                .and_then(CachedFeed::ready)
                .and_then(|f| f.image.clone()),
            Some(CoverChoice::Episode(e)) => chosen
                .iter()
                .find(|c| c.reference == e)
                .and_then(|c| c.episode.image.clone()),
            _ => None,
        };
        // Couverture : une image d'épisode déjà téléchargée à la même adresse, sinon un
        // téléchargement isolé.
        let cover_source: Option<PathBuf> = match &cover_url {
            None => None,
            Some(url) => {
                let known = fetched
                    .fetched
                    .iter()
                    .filter_map(|f| Some((rank(&f.id)?, f.image.clone()?)))
                    .chain(hit_images.iter().map(|(n, p)| (*n, p.clone())))
                    .find_map(|(n, path)| {
                        (chosen.get(n)?.episode.image.as_deref() == Some(url.as_str()))
                            .then_some(path)
                    });
                match known {
                    Some(path) => Some(path),
                    None => {
                        let dest = workdir.path().join("cover.img");
                        backend
                            .fetch_image(url, &dest, cancel.clone())
                            .await?
                            .then_some(dest)
                    }
                }
            }
        };
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        let dir = workdir.path().to_path_buf();
        let jobs: Vec<(usize, Option<PathBuf>)> =
            kept.iter().map(|(n, _, img)| (*n, img.clone())).collect();
        let converter = backend.clone();
        let (pngs, cover) = blocking(move || {
            convert_artwork(&dir, &jobs, cover_source, &|bytes, kind| {
                converter.convert_artwork(bytes, kind)
            })
        })
        .await??;

        // 4. Archive Telmi (90 à 100 %).
        self.emit_phase(id, 90.0, "archive");
        let mut episodes: HashMap<usize, PackEpisode> = HashMap::new();
        for (n, mp3, _) in kept {
            episodes.insert(
                n,
                PackEpisode {
                    title: chosen[n].episode.title.clone(),
                    audio: mp3,
                    image: pngs.get(&n).cloned(),
                    title_audio: None,
                },
            );
        }
        let ranks: HashMap<&EpisodeRef, usize> = chosen
            .iter()
            .enumerate()
            .map(|(n, c)| (c.reference, n))
            .collect();
        let mut used: HashSet<TitleKey> = HashSet::new();
        let mut menu = build_menu(
            &job.composition.menu,
            &ranks,
            &mut episodes,
            &spoken,
            &mut Vec::new(),
            &mut used,
        );
        mute_shifted_categories(&job.composition.menu, &used, &spoken, &mut menu);
        if menu.is_empty() {
            return Err(AppError::DownloadInvalid(
                "aucun épisode utilisable dans le pack".into(),
            ));
        }
        // Avertissements des seuls titres muets qui restent dans le pack (un épisode retiré
        // n'en a plus besoin).
        let title_audio = spoken.audio.get(&TitleKey::Pack).cloned();
        warnings.extend(
            spoken
                .failed
                .iter()
                .filter(|(key, _)| *key == TitleKey::Pack || used.contains(key))
                .map(|(key, text)| pack_speech::failure_warning(key, text)),
        );
        if !warnings.is_empty() {
            self.set_pack_warnings(id, warnings);
        }
        let description = metadata
            .description
            .clone()
            .unwrap_or_else(|| archive::default_description(&menu));
        let pack = Pack {
            uuid: uuid::Uuid::parse_str(&job.uuid)
                .map_err(|_| AppError::Validation("uuid de pack invalide".into()))?,
            metadata: PackMetadata {
                title: metadata.title.clone(),
                subtitle: metadata.subtitle.clone(),
                age: metadata.age,
                description: archive::with_failures(&description, &dropped_titles),
            },
            cover,
            menu,
            title_audio,
        };
        let archive_path = workdir.path().join(ARCHIVE_NAME);
        let dest = archive_path.clone();
        let flag = cancel.clone();
        blocking(move || archive::write_archive(&pack, &dest, &|| *flag.borrow()))
            .await?
            .map_err(|err| match err {
                archive::ArchiveError::Cancelled => AppError::Cancelled,
                archive::ArchiveError::Io(e) => AppError::Io(e),
                other => AppError::Conversion {
                    code: "pack_archive".into(),
                    message: other.to_string(),
                },
            })?;
        let entry = pack_entry(&job.uuid, &metadata, &archive_path);
        Ok((archive_path, entry))
    }

    fn set_pack_status(&self, id: &str, episodes: usize, dropped: Vec<String>) {
        let mut tasks = lock(&self.0.tasks);
        if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == id) {
            let warnings = slot
                .task
                .pack
                .take()
                .map(|p| p.warnings)
                .unwrap_or_default();
            slot.task.pack = Some(PackStatus {
                episodes,
                dropped,
                warnings,
            });
        }
    }

    fn set_pack_warnings(&self, id: &str, warnings: Vec<String>) {
        let mut tasks = lock(&self.0.tasks);
        if let Some(slot) = tasks.iter_mut().find(|s| s.task.id == id) {
            slot.task
                .pack
                .get_or_insert_with(PackStatus::default)
                .warnings = warnings;
        }
    }

    /// Titres lus (#462) : voix choisie (repli sur la voix par défaut avec un avertissement),
    /// puis synthèse et conversion de chaque titre dans le dossier de travail. Rend les MP3
    /// produits et les avertissements de voix ; voir [`pack_speech::speak`] pour les arrêts.
    #[allow(clippy::too_many_arguments)]
    async fn speak_titles(
        &self,
        id: &str,
        job: &PackJob,
        chosen: &[Chosen<'_>],
        metadata: &ResolvedMetadata,
        workdir: &Workdir,
        backend: &Arc<dyn PackBackend>,
        cancel: &watch::Receiver<bool>,
    ) -> AppResult<(SpokenTitles, Vec<String>)> {
        self.emit_phase(id, 0.0, "speech");
        let synth = backend.speech();
        let (voice, fallback) = {
            let (synth, requested) = (synth.clone(), job.composition.titles_voice.clone());
            blocking(move || pack_speech::choose_voice(&synth, requested.as_deref())).await??
        };
        let titles_by_ref: HashMap<&EpisodeRef, &str> = chosen
            .iter()
            .map(|c| (c.reference, c.episode.title.as_str()))
            .collect();
        let titles = pack_speech::titles(&job.composition, &metadata.title, &|r| {
            titles_by_ref.get(r).map(|t| (*t).to_string())
        });
        // Avancement par titre (0 à 100 % de l'étape, #487) ; durée réelle journalisée (rang et
        // nombre seulement, jamais de texte ni de voix).
        let started = std::time::Instant::now();
        let progress = |done: usize, total: usize| {
            if let Some(percent) = (done * 100).checked_div(total) {
                self.emit_phase(id, percent as f64, "speech");
            }
        };
        let spoken = pack_speech::speak(
            backend,
            &synth,
            &voice,
            &titles,
            workdir.path(),
            cancel,
            &progress,
        )
        .await?;
        tracing::info!(
            titles = spoken.audio.len(),
            failed = spoken.failed.len(),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "titres lus : étape terminée"
        );
        Ok((spoken, fallback.into_iter().collect()))
    }
}

/// Décodeur de pochette ([`PackBackend::convert_artwork`]).
pub(super) type ArtworkDecoder<'a> =
    dyn Fn(&[u8], ArtworkKind) -> Result<Artwork, ArtworkError> + 'a;

/// Convertit les pochettes (640 × 480) et la couverture (512 × 512) en PNG ; supprime les
/// originaux. Une image illisible donne l'image de repli (D13). Bloquant.
pub(super) fn convert_artwork(
    dir: &Path,
    jobs: &[(usize, Option<PathBuf>)],
    cover: Option<PathBuf>,
    decode: &ArtworkDecoder<'_>,
) -> AppResult<(HashMap<usize, PathBuf>, Option<PathBuf>)> {
    let convert = |src: &Path, kind: ArtworkKind, dest: PathBuf| -> AppResult<Option<PathBuf>> {
        let bytes = match read_bounded(src, fetch::MAX_IMAGE_BYTES) {
            Some(b) => b,
            None => return Ok(None),
        };
        match decode(&bytes, kind) {
            Ok(art) => {
                std::fs::write(&dest, &art.png)?;
                Ok(Some(dest))
            }
            Err(err) => {
                tracing::warn!("pochette ignorée : {err}");
                Ok(None)
            }
        }
    };
    let cover_png = match &cover {
        Some(src) => convert(src, ArtworkKind::Cover, dir.join("cover.png"))?,
        None => None,
    };
    let mut pngs = HashMap::new();
    for (n, image) in jobs {
        if let Some(src) = image {
            if let Some(png) = convert(
                src,
                ArtworkKind::Episode,
                dir.join(format!("ep-{n:04}.png")),
            )? {
                pngs.insert(*n, png);
            }
        }
    }
    // Originaux inutiles pour l'archive : place libérée avant son écriture.
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".audio") || name.ends_with(".img") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok((pngs, cover_png))
}

/// Lit au plus `max` octets (`None` si absent, illisible ou plus gros).
fn read_bounded(path: &Path, max: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(max + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= max).then_some(bytes)
}

/// Arborescence de l'archive : épisodes retirés sautés, catégories vidées retirées (P7) ;
/// titres lus (#462) repris de `spoken` par position dans la composition (`path`), positions
/// gardées notées dans `used`.
fn build_menu(
    menu: &[PackNode],
    ranks: &HashMap<&EpisodeRef, usize>,
    episodes: &mut HashMap<usize, PackEpisode>,
    spoken: &SpokenTitles,
    path: &mut Vec<usize>,
    used: &mut HashSet<TitleKey>,
) -> Vec<MenuEntry> {
    let mut out = Vec::new();
    for (i, node) in menu.iter().enumerate() {
        path.push(i);
        let key = TitleKey::Node(path.clone());
        match node {
            PackNode::Episode(e) => {
                if let Some(mut ep) = ranks.get(e).and_then(|n| episodes.remove(n)) {
                    ep.title_audio = spoken.audio.get(&key).cloned();
                    out.push(MenuEntry::Episode(ep));
                    used.insert(key);
                }
            }
            PackNode::Category { name, entries } => {
                let entries = build_menu(entries, ranks, episodes, spoken, path, used);
                if !entries.is_empty() {
                    out.push(MenuEntry::Category {
                        name: name.trim().to_string(),
                        entries,
                        audio: spoken.audio.get(&key).cloned(),
                    });
                    used.insert(key);
                }
            }
        }
        path.pop();
    }
    out
}

/// Titres lus « Catégorie k » (nom sans rien d'affichable, #487) : `k` est compté sur la
/// composition, avant les retraits. Si une catégorie vidée plus haut (P7) décale la numérotation
/// de l'archive, le titre lu ne dit plus le libellé affiché : l'option reste alors muette.
fn mute_shifted_categories(
    composition: &[PackNode],
    used: &HashSet<TitleKey>,
    spoken: &SpokenTitles,
    menu: &mut [MenuEntry],
) {
    if spoken.numbered.is_empty() {
        return;
    }
    // Catégories gardées, dans l'ordre de l'archive (une catégorie avant ses options).
    fn kept(
        menu: &[PackNode],
        used: &HashSet<TitleKey>,
        path: &mut Vec<usize>,
        out: &mut Vec<TitleKey>,
    ) {
        for (i, node) in menu.iter().enumerate() {
            path.push(i);
            if let PackNode::Category { entries, .. } = node {
                let key = TitleKey::Node(path.clone());
                if used.contains(&key) {
                    out.push(key);
                }
                kept(entries, used, path, out);
            }
            path.pop();
        }
    }
    fn audios<'a>(menu: &'a mut [MenuEntry], out: &mut Vec<&'a mut Option<PathBuf>>) {
        for entry in menu {
            if let MenuEntry::Category { entries, audio, .. } = entry {
                out.push(audio);
                audios(entries, out);
            }
        }
    }
    let mut keys = Vec::new();
    kept(composition, used, &mut Vec::new(), &mut keys);
    let mut slots = Vec::new();
    audios(menu, &mut slots);
    debug_assert_eq!(keys.len(), slots.len());
    for (rank, (key, audio)) in keys.iter().zip(slots).enumerate() {
        if spoken.numbered.get(key).is_some_and(|&k| k != rank + 1) {
            *audio = None;
        }
    }
}

/// Résultat de [`pack_create`] (miroir de `PackCreated` dans `src/api/install.ts`).
#[derive(Debug, Clone, Serialize)]
pub struct PackCreated {
    /// Tâche créée ; `None` si la composition est invalide (voir `report`).
    pub task: Option<super::InstallTask>,
    pub report: ValidationReport,
    /// Composition conservée après la création de la tâche (#461) ; `None` sans tâche.
    pub composition: Option<crate::podcast::compositions::CompositionRecord>,
}

/// Crée un pack (N7 a) : revalide la composition à partir des flux en cache et, si elle est
/// valide, ajoute une tâche à la file d'installation. `draft_revision` : révision du brouillon,
/// effacé après la publication s'il n'a pas changé entre-temps.
#[tauri::command]
pub async fn pack_create(
    config: State<'_, ConfigState>,
    podcasts: State<'_, PodcastService>,
    service: State<'_, InstallService>,
    composition: PackComposition,
    draft_revision: Option<u64>,
) -> AppResult<PackCreated> {
    if draft_revision.is_some_and(|r| r > pack::MAX_REVISION) {
        return Err(AppError::Validation(
            "révision de brouillon invalide".into(),
        ));
    }
    let feeds = pack::load_feeds(&config, &podcasts, &composition).await?;
    let report = pack::validate(&composition, &feeds);
    if !report.valid {
        return Ok(PackCreated {
            task: None,
            report,
            composition: None,
        });
    }
    let task = service
        .start_pack(composition.clone(), &report, draft_revision)
        .await?;
    // Composition conservée sous un nom (#461) ; un échec ne remet pas la tâche en cause.
    let record = {
        let title = report.metadata.as_ref().map_or("", |m| m.title.as_str());
        let _guard = config.guard();
        crate::podcast::compositions::record_after_create(&config.dir, &composition, title)
    };
    Ok(PackCreated {
        task: Some(task),
        report,
        composition: Some(record),
    })
}

/// Vrai si `uuid` est un uuid de pack canonique (relecture de la file).
pub fn is_pack_uuid(uuid: &str) -> bool {
    is_task_id(uuid)
}

#[cfg(test)]
#[path = "podcast_cache_tests.rs"]
mod cache_tests;

#[cfg(test)]
#[path = "podcast_speech_tests.rs"]
mod speech_tests;
