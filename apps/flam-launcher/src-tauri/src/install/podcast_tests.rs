//! Packs de podcasts dans la file (#217) : création complète, injection de pannes à chaque
//! étape (état final, aucun fichier résiduel, `library.json` cohérent), reprise après un arrêt
//! brutal (même uuid), composition invalide refusée.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;

use super::*;
use crate::install::podcast::{
    archive_path, clear_draft_if_unchanged, pack_entry, BoxFuture, PackBackend, PackJob, WORK_DIR,
};
use crate::podcast::artwork::{Artwork, ArtworkKind};
use crate::podcast::fetch::{DropReason, Dropped, Fetched, Item, Progress, Report, Workdir};
use crate::podcast::pack::{
    self as packmod, CachedFeed, CoverChoice, EpisodeRef, Feeds, PackComposition,
    PackMetadataInput, PackNode,
};
use crate::podcast::rss::{PodcastEpisode, PodcastFeed};
use crate::podcast::source::episode_id;

/// Panne injectée.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    /// Le deuxième épisode échoue au téléchargement (P7).
    DropOne,
    /// Un épisode a disparu du flux (reprise après changement du flux).
    FeedChanged,
    DiskFull,
    FetchFatal,
    AllDropped,
    AudioAllFail,
    CancelFetch,
    CancelAudio,
    /// Le convertisseur (faux telmi2flam) échoue.
    ConvertFail,
    /// Le décodeur isolé des pochettes échoue (enfant mort) : images de repli.
    ArtworkChildDies,
    /// `fetch::Limits::batch_timeout` dépassé.
    BatchTimeout,
}

struct FakePacks {
    fault: Fault,
    entered: AtomicBool,
    drafts: Mutex<Vec<u64>>,
    /// Concurrence demandée à chaque lot.
    concurrency: Mutex<Vec<usize>>,
    /// Lots en cours et maximum observé (sérialisation des préparations).
    running: AtomicUsize,
    max_running: AtomicUsize,
}

impl FakePacks {
    fn new(fault: Fault) -> Arc<Self> {
        Arc::new(Self {
            fault,
            entered: AtomicBool::new(false),
            drafts: Mutex::default(),
            concurrency: Mutex::default(),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
        })
    }
}

fn episode(guid: &str, title: &str) -> PodcastEpisode {
    PodcastEpisode {
        guid: guid.into(),
        title: title.into(),
        description: None,
        pub_date: None,
        duration: Some(60),
        author: None,
        image: Some(format!("https://img.example.test/{guid}.png")),
        enclosure_url: format!("https://audio.example.test/{guid}.mp3?token=secret"),
        enclosure_type: "audio/mpeg".into(),
        enclosure_length: Some(1000),
        explicit: None,
    }
}

fn feed(title: &str, episodes: Vec<PodcastEpisode>) -> PodcastFeed {
    PodcastFeed {
        title: title.into(),
        description: None,
        image: Some("https://img.example.test/feed.png".into()),
        link: None,
        copyright: None,
        explicit: None,
        episodes,
        warnings: Vec::new(),
    }
}

fn feeds(fault: Fault) -> Feeds {
    let mut s2 = vec![episode("g2", "Deux"), episode("g3", "Trois")];
    if fault == Fault::FeedChanged {
        s2.pop();
    }
    Feeds::from([
        (
            "s1".to_string(),
            CachedFeed::Ready(feed("Flux 1", vec![episode("g1", "Un")])),
        ),
        ("s2".to_string(), CachedFeed::Ready(feed("Flux 2", s2))),
    ])
}

fn ep(source: &str, guid: &str) -> EpisodeRef {
    EpisodeRef {
        source_id: source.into(),
        episode_id: episode_id(guid),
    }
}

/// Deux flux, un sous-menu.
fn composition() -> PackComposition {
    PackComposition {
        metadata: PackMetadataInput {
            title: "Mon pack".into(),
            ..PackMetadataInput::default()
        },
        cover: CoverChoice::Feed {
            source_id: "s1".into(),
        },
        menu: vec![
            PackNode::Episode(ep("s1", "g1")),
            PackNode::Category {
                name: "Saison".into(),
                entries: vec![
                    PackNode::Episode(ep("s2", "g2")),
                    PackNode::Episode(ep("s2", "g3")),
                ],
            },
        ],
    }
}

fn png() -> Vec<u8> {
    Artwork::fallback(ArtworkKind::Episode, [1, 2, 3])
        .unwrap()
        .png
}

impl PackBackend for FakePacks {
    fn ready(&self) -> AppResult<()> {
        Ok(())
    }

    fn feeds<'a>(&'a self, _c: &'a PackComposition) -> BoxFuture<'a, AppResult<Feeds>> {
        Box::pin(async move { Ok(feeds(self.fault)) })
    }

    fn free_space(&self, _dir: &Path) -> Option<u64> {
        (self.fault == Fault::DiskFull).then_some(1000)
    }

    fn fetch<'a>(
        &'a self,
        items: &'a [Item],
        dir: &'a Workdir,
        concurrency: usize,
        mut cancel: watch::Receiver<bool>,
        _on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> BoxFuture<'a, AppResult<Report>> {
        Box::pin(async move {
            lock(&self.concurrency).push(concurrency);
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_running.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(150)).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
            // Aucune URL ni titre dans les étiquettes (D11).
            for item in items {
                assert!(!item.id.contains("://") && !item.feed.contains("://"));
                assert!(item.feed == "s1" || item.feed == "s2");
            }
            let mode = if self.fault == Fault::ConvertFail {
                "fail"
            } else {
                "ok"
            };
            let mut report = Report::default();
            for (n, item) in items.iter().enumerate() {
                let audio = dir.path().join(format!("ep-{n:04}.audio"));
                std::fs::write(&audio, format!("ID3mode={mode};audio {n}")).unwrap();
                let image = dir.path().join(format!("ep-{n:04}.img"));
                std::fs::write(&image, png()).unwrap();
                let dropped = match self.fault {
                    Fault::AllDropped => true,
                    Fault::DropOne => n == 1,
                    _ => false,
                };
                if dropped {
                    report.dropped.push(Dropped {
                        id: item.id.clone(),
                        feed: item.feed.clone(),
                        reason: DropReason::Unreachable,
                    });
                } else {
                    report.fetched.push(Fetched {
                        id: item.id.clone(),
                        feed: item.feed.clone(),
                        audio_bytes: std::fs::metadata(&audio).unwrap().len(),
                        audio,
                        image: Some(image),
                    });
                }
            }
            self.entered.store(true, Ordering::SeqCst);
            match self.fault {
                Fault::CancelFetch => {
                    download::cancelled(&mut cancel).await;
                    Err(AppError::Cancelled)
                }
                Fault::FetchFatal => Err(AppError::Io(std::io::Error::other("disque"))),
                Fault::BatchTimeout => {
                    Err(AppError::DownloadInvalid("délai du lot dépassé".into()))
                }
                Fault::AllDropped => Err(AppError::DownloadInvalid(
                    "aucun épisode récupérable".into(),
                )),
                _ => Ok(report),
            }
        })
    }

    fn fetch_image<'a>(
        &'a self,
        _url: &'a str,
        dest: &'a Path,
        _cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            std::fs::write(dest, png())?;
            Ok(true)
        })
    }

    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>> {
        Box::pin(async move {
            if self.fault == Fault::CancelAudio {
                self.entered.store(true, Ordering::SeqCst);
                let mut cancel = cancel.clone();
                download::cancelled(&mut cancel).await;
                return Ok(items.iter().map(|_| Err(AppError::Cancelled)).collect());
            }
            Ok(items
                .iter()
                .map(|(from, to)| {
                    if self.fault == Fault::AudioAllFail {
                        return Err(AppError::Conversion {
                            code: "ffmpeg_failed".into(),
                            message: "échec".into(),
                        });
                    }
                    std::fs::copy(from, to).map_err(AppError::from)
                })
                .collect())
        })
    }

    fn draft_done(&self, revision: u64) {
        lock(&self.drafts).push(revision);
    }

    fn convert_artwork(
        &self,
        bytes: &[u8],
        kind: ArtworkKind,
    ) -> Result<Artwork, crate::podcast::artwork::ArtworkError> {
        if self.fault == Fault::ArtworkChildDies {
            return Err(crate::podcast::artwork::ArtworkError::Isolated(
                "décodeur arrêté".into(),
            ));
        }
        Artwork::convert(bytes, kind)
    }
}

impl Ctx {
    fn packs(&self, fault: Fault) -> Arc<FakePacks> {
        let fake = FakePacks::new(fault);
        self.service.set_pack_backend(fake.clone());
        fake
    }

    async fn start_pack(&self) -> InstallTask {
        let c = composition();
        let report = packmod::validate(&c, &feeds(Fault::None));
        assert!(report.valid, "{report:?}");
        self.service.start_pack(c, &report, Some(7)).await.unwrap()
    }

    /// Aucun fichier de travail ni de publication en cours ne reste.
    fn assert_clean(&self) {
        let work = self.cache().join(WORK_DIR);
        let left: Vec<_> = std::fs::read_dir(&work)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "dossiers de travail résiduels : {left:?}");
        assert!(self.staging_is_empty());
        assert!(self.queue().is_empty(), "file vidée");
    }
}

#[tokio::test]
async fn creation_complete_d_un_pack_multi_flux_avec_sous_menu() {
    let Some(py) = python() else { return };
    let ctx = Ctx::new(Some(py));
    let fake = ctx.packs(Fault::DropOne);
    let task = ctx.start_pack().await;
    assert_eq!(
        task.entry.source_id,
        crate::install::podcast::PACK_SOURCE_ID
    );
    assert_eq!(task.pack.as_ref().unwrap().episodes, 3);
    // Composition et uuid enregistrés avec la tâche (N10 a).
    ctx.service.flush_queue().await;
    let queued = ctx.queue();
    let job = queued[0].pack.clone().expect("pack persisté");
    assert_eq!(job.uuid, task.entry.uuid);
    assert!(!serde_json::to_string(&queued[0])
        .unwrap()
        .contains("token=secret"));

    let done = ctx.finished(&task.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    assert_eq!(
        ctx.events.states(&task.id),
        [
            TaskState::Queued,
            TaskState::Preparing,
            TaskState::Prepared,
            TaskState::Converting,
            TaskState::Installing
        ]
    );
    // P7 : l'épisode en échec est retiré et signalé, jusque dans `install://done`.
    assert_eq!(done.pack.unwrap().dropped, ["Deux"]);
    let event = ctx
        .events
        .all()
        .into_iter()
        .find_map(|e| match e {
            Recorded::Done(d) if d.task_id == task.id => Some(d),
            _ => None,
        })
        .expect("événement de fin");
    assert_eq!(event.pack.unwrap().dropped, ["Deux"]);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].uuid, job.uuid);
    assert_eq!(library[0].version, Some(1));
    assert_eq!(library[0].title, "Mon pack");
    ctx.assert_clean();
    assert_eq!(
        *lock(&fake.drafts),
        [7],
        "brouillon effacé après publication"
    );
    // `max_downloads` (2 dans `Ctx`) borne les téléchargements du lot.
    assert_eq!(*lock(&fake.concurrency), [2]);
}

/// Injection de pannes à chaque étape : état final, aucun fichier résiduel, bibliothèque
/// inchangée (l'histoire déjà installée reste seule et lisible).
#[tokio::test]
async fn pannes_a_chaque_etape_sans_residu_et_bibliotheque_coherente() {
    let Some(py) = python() else { return };
    for (fault, expected, kind) in [
        (Fault::FeedChanged, TaskState::Failed, "validation"),
        (Fault::DiskFull, TaskState::Failed, "io"),
        (Fault::FetchFatal, TaskState::Failed, "io"),
        (Fault::AllDropped, TaskState::Failed, "download_invalid"),
        (Fault::AudioAllFail, TaskState::Failed, "download_invalid"),
        (Fault::CancelFetch, TaskState::Cancelled, "cancelled"),
        (Fault::CancelAudio, TaskState::Cancelled, "cancelled"),
        (Fault::ConvertFail, TaskState::Failed, "conversion"),
        (Fault::BatchTimeout, TaskState::Failed, "download_invalid"),
    ] {
        let ctx = Ctx::new(Some(py.clone()));
        install_directly(&ctx, "deja", b"PK");
        let fake = ctx.packs(fault);
        let task = ctx.start_pack().await;
        if matches!(fault, Fault::CancelFetch | Fault::CancelAudio) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !fake.entered.load(Ordering::SeqCst) {
                assert!(
                    Instant::now() < deadline,
                    "{fault:?} : étape jamais atteinte"
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            ctx.service.cancel(&task.id).unwrap();
        }
        let end = ctx.finished(&task.id).await;
        assert_eq!(end.state, expected, "{fault:?} : {end:?}");
        if fault == Fault::BatchTimeout {
            let message = &end.error.as_ref().unwrap().message;
            assert!(
                message.contains("téléchargement du pack interrompu")
                    && message.contains("délai du lot dépassé")
                    && message.contains("2 h"),
                "{message}"
            );
        }
        assert_eq!(
            end.error.as_ref().unwrap().kind,
            kind,
            "{fault:?} : {end:?}"
        );
        assert!(
            !end.error.unwrap().message.contains("example.test"),
            "{fault:?} : aucune URL dans l'erreur (D11)"
        );
        let library = ctx.service.library_list().await.unwrap();
        assert_eq!(library.len(), 1, "{fault:?}");
        assert_eq!(library[0].uuid, "deja");
        ctx.assert_clean();
        assert!(lock(&fake.drafts).is_empty(), "{fault:?} : brouillon gardé");
    }
}

fn queued_pack(ctx_dir: &Path, fault_title: &str) -> (PackJob, String) {
    let job = PackJob {
        uuid: "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d".into(),
        composition: composition(),
        draft_revision: Some(9),
    };
    let cache = ctx_dir.join("cache");
    let mut entry = pack_entry(
        &job.uuid,
        &packmod::ResolvedMetadata {
            title: fault_title.into(),
            subtitle: String::new(),
            age: 0,
            description: None,
        },
        &archive_path(&cache, TASK_ID).unwrap(),
    );
    // Un lien persisté n'est jamais repris tel quel.
    entry.download_url = "https://evil.example.test/x.zip".into();
    storage::write_json(
        &ctx_dir.join("config").join(QUEUE_FILE),
        &QueueFile {
            schema_version: QUEUE_SCHEMA,
            tasks: vec![QueuedTask {
                id: TASK_ID.into(),
                entry,
                created_at: 1,
                validator: Validator::default(),
                pack: Some(job.clone()),
            }],
        },
    )
    .unwrap();
    // Arrêt brutal : dossier de travail de la tâche et dossier orphelin laissés.
    let work = cache.join(WORK_DIR);
    std::fs::create_dir_all(work.join(TASK_ID)).unwrap();
    std::fs::write(work.join(TASK_ID).join("ep-0000.audio.part"), b"partiel").unwrap();
    let orphan = work.join("11111111-2222-4333-8444-555555555555");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("pack.zip"), b"vieux").unwrap();
    (job, orphan.display().to_string())
}

#[tokio::test]
async fn reprise_apres_arret_brutal_meme_uuid() {
    let Some(py) = python() else { return };
    let dir = tempfile::tempdir().unwrap();
    let (job, orphan) = queued_pack(dir.path(), "Mon pack");
    let ctx = Ctx::with_dir(dir, Some(py), GRACE);
    ctx.packs(Fault::None);
    ctx.service
        .restore(|_, _| async { Err(AppError::NotFound("aucune source".into())) })
        .await;
    assert!(!Path::new(&orphan).exists(), "dossier orphelin supprimé");
    let list = ctx.service.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].entry.uuid, job.uuid);
    assert!(!list[0].entry.download_url.contains("evil"));
    let done = ctx.finished(TASK_ID).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].uuid, job.uuid, "même pack reconstruit");
    ctx.assert_clean();
}

#[tokio::test]
async fn reprise_avec_episode_disparu_echoue_proprement() {
    let Some(py) = python() else { return };
    let dir = tempfile::tempdir().unwrap();
    queued_pack(dir.path(), "Mon pack");
    let ctx = Ctx::with_dir(dir, Some(py), GRACE);
    ctx.packs(Fault::FeedChanged);
    ctx.service
        .restore(|_, _| async { Err(AppError::NotFound("aucune source".into())) })
        .await;
    let end = ctx.finished(TASK_ID).await;
    assert_eq!(end.state, TaskState::Failed);
    assert_eq!(end.error.unwrap().kind, "validation");
    assert!(ctx.service.library_list().await.unwrap().is_empty());
    ctx.assert_clean();
}

#[tokio::test]
async fn composition_invalide_refusee_par_le_backend() {
    let ctx = Ctx::new(None);
    ctx.packs(Fault::None);
    let mut c = composition();
    // Doublon.
    c.menu.push(PackNode::Episode(ep("s1", "g1")));
    let report = packmod::validate(&c, &feeds(Fault::None));
    assert!(!report.valid);
    let err = ctx.service.start_pack(c, &report, None).await.unwrap_err();
    assert_eq!(err.kind(), "validation");
    // Profondeur : 4 niveaux de menus.
    let mut deep = composition();
    deep.menu = vec![PackNode::Category {
        name: "a".into(),
        entries: vec![PackNode::Category {
            name: "b".into(),
            entries: vec![PackNode::Category {
                name: "c".into(),
                entries: vec![PackNode::Episode(ep("s1", "g1"))],
            }],
        }],
    }];
    let report = packmod::validate(&deep, &feeds(Fault::None));
    assert!(ctx.service.start_pack(deep, &report, None).await.is_err());
    assert!(ctx.service.list().is_empty());
    assert!(ctx.queue().is_empty());
}

#[tokio::test]
async fn sans_python_le_pack_echoue_avant_tout_telechargement() {
    let ctx = Ctx::new(None);
    let fake = ctx.packs(Fault::None);
    let task = ctx.start_pack().await;
    let end = ctx.finished(&task.id).await;
    assert_eq!(end.error.unwrap().kind, "converter_unavailable");
    assert!(!fake.entered.load(Ordering::SeqCst), "rien téléchargé");
    // #445 : la tâche échouée ne garde pas le chemin du dossier de travail supprimé.
    assert!(end.entry.download_url.is_empty(), "{:?}", end.entry);
    let list = ctx.service.list();
    assert_eq!(list.len(), 1);
    assert!(list[0].entry.download_url.is_empty());
    ctx.assert_clean();
}

/// #445 : moteur Rust, `telmi2flam.exe` présent, aucun Python : le pré-contrôle passe, le pack
/// n'est pas refusé comme « convertisseur indisponible » et la préparation démarre.
#[tokio::test]
async fn moteur_rust_avec_binaire_sans_python_passe_le_precontrole() {
    let Some(py) = python() else { return };
    let ctx = Ctx::with_engine(Some(py), ConverterEngine::Rust);
    let fake = ctx.packs(Fault::None);
    let task = ctx.start_pack().await;
    let end = ctx.finished(&task.id).await;
    // Une éventuelle erreur vient de la suite (le faux exe est un python.exe), jamais du
    // pré-contrôle ni de la validation.
    let kind = end.error.as_ref().map(|e| e.kind.as_str());
    assert!(
        !matches!(kind, Some("converter_unavailable" | "validation")),
        "{end:?}"
    );
    assert!(fake.entered.load(Ordering::SeqCst), "préparation lancée");
    ctx.assert_clean();
}

/// #440 : le pré-contrôle suit le moteur choisi. Moteur Rust sans `telmi2flam.exe` : refus avant
/// tout téléchargement, avec un message propre au moteur Rust (et non « Python introuvable »).
#[tokio::test]
async fn moteur_rust_sans_binaire_le_pack_echoue_avant_tout_telechargement() {
    let ctx = Ctx::with_engine(None, ConverterEngine::Rust);
    let fake = ctx.packs(Fault::None);
    let task = ctx.start_pack().await;
    let end = ctx.finished(&task.id).await;
    let error = end.error.unwrap();
    assert_eq!(error.kind, "converter_unavailable");
    assert!(error.message.contains("telmi2flam.exe"), "{error:?}");
    assert!(!fake.entered.load(Ordering::SeqCst), "rien téléchargé");
    ctx.assert_clean();
}

#[test]
fn le_brouillon_n_est_efface_que_s_il_n_a_pas_change() {
    let dir = tempfile::tempdir().unwrap();
    packmod::save_draft(dir.path(), &composition(), 5).unwrap();
    // Brouillon modifié depuis la création (révision 5 > 3) : gardé.
    clear_draft_if_unchanged(dir.path(), 3).unwrap();
    assert!(packmod::read_draft(dir.path())
        .unwrap()
        .composition
        .is_some());
    clear_draft_if_unchanged(dir.path(), 5).unwrap();
    assert!(packmod::read_draft(dir.path())
        .unwrap()
        .composition
        .is_none());
}

#[tokio::test]
async fn annulation_en_attente_du_convertisseur_sans_residu() {
    let Some(py) = python() else { return };
    let ctx = Ctx::new(Some(py));
    ctx.packs(Fault::None);
    // Convertisseur occupé par une autre tâche : le pack préparé attend son tour.
    let busy = ctx.service.0.conversions.acquire().await.unwrap();
    let task = ctx.start_pack().await;
    ctx.wait(&task.id, |t| t.state == TaskState::Prepared).await;
    ctx.service.cancel(&task.id).unwrap();
    let end = ctx.finished(&task.id).await;
    drop(busy);
    assert_eq!(end.state, TaskState::Cancelled, "{end:?}");
    assert!(ctx.service.library_list().await.unwrap().is_empty());
    ctx.assert_clean();
}

#[tokio::test]
async fn reprise_d_un_pack_deja_publie_ne_relance_rien() {
    let dir = tempfile::tempdir().unwrap();
    let (job, orphan) = queued_pack(dir.path(), "Mon pack");
    let ctx = Ctx::with_dir(dir, None, GRACE);
    // Arrêt brutal entre la publication et la réécriture de la file.
    install_directly(&ctx, &job.uuid, b"PK");
    let fake = ctx.packs(Fault::None);
    ctx.service
        .restore(|_, _| async { Err(AppError::NotFound("aucune source".into())) })
        .await;
    assert!(ctx.service.list().is_empty(), "aucune tâche relancée");
    assert!(!fake.entered.load(Ordering::SeqCst));
    assert_eq!(*lock(&fake.drafts), [9], "brouillon effacé");
    assert!(!Path::new(&orphan).exists());
    ctx.assert_clean();
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].uuid, job.uuid);
}

#[tokio::test]
async fn deux_packs_simultanes_sont_prepares_l_un_apres_l_autre() {
    let Some(py) = python() else { return };
    let ctx = Ctx::new(Some(py));
    let fake = ctx.packs(Fault::None);
    let (a, b) = tokio::join!(ctx.start_pack(), ctx.start_pack());
    assert_ne!(a.entry.uuid, b.entry.uuid, "deux histoires distinctes (P3)");
    for id in [&a.id, &b.id] {
        let end = ctx.finished(id).await;
        assert_eq!(end.state, TaskState::Done, "{end:?}");
    }
    assert_eq!(lock(&fake.concurrency).len(), 2);
    assert_eq!(
        fake.max_running.load(Ordering::SeqCst),
        1,
        "une seule préparation à la fois"
    );
    assert_eq!(ctx.service.library_list().await.unwrap().len(), 2);
    ctx.assert_clean();
}

#[test]
fn le_nettoyage_ne_supprime_que_les_orphelins_releves() {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join(WORK_DIR);
    for name in ["garde", "orphelin"] {
        std::fs::create_dir_all(work.join(name)).unwrap();
        std::fs::write(work.join(name).join("f"), b"x").unwrap();
    }
    std::fs::write(work.join("fichier"), b"x").unwrap();
    crate::install::podcast::sweep_workdirs(dir.path(), &|names| {
        names.into_iter().filter(|n| n != "garde").collect()
    });
    assert!(work.join("garde/f").is_file());
    assert!(!work.join("orphelin").exists());
    assert!(!work.join("fichier").exists());
}

#[tokio::test]
async fn pochettes_en_echec_dans_l_enfant_donnent_l_image_de_repli() {
    let Some(py) = python() else { return };
    let ctx = Ctx::new(Some(py));
    ctx.packs(Fault::ArtworkChildDies);
    let task = ctx.start_pack().await;
    let done = ctx.finished(&task.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    assert!(
        done.pack.unwrap().dropped.is_empty(),
        "aucun épisode retiré (D13)"
    );
    ctx.assert_clean();
}

#[test]
fn la_preparation_decode_les_pochettes_par_le_decodeur_fourni() {
    let dir = tempfile::tempdir().unwrap();
    let img = dir.path().join("ep-0000.img");
    std::fs::write(&img, png()).unwrap();
    let cover = dir.path().join("cover.img");
    std::fs::write(&cover, png()).unwrap();
    let calls = Mutex::new(Vec::new());
    let (pngs, cover_png) = crate::install::podcast::convert_artwork(
        dir.path(),
        &[(0, Some(img.clone()))],
        Some(cover),
        &|_, kind| {
            lock(&calls).push(kind);
            Err(crate::podcast::artwork::ArtworkError::Isolated(
                "mort".into(),
            ))
        },
    )
    .unwrap();
    assert_eq!(*lock(&calls), [ArtworkKind::Cover, ArtworkKind::Episode]);
    assert!(pngs.is_empty() && cover_png.is_none(), "repli");
    assert!(!img.exists(), "originaux supprimés");
}

// ---------------------------------------------------------------------------------------------
// Bout en bout (#426) : serveur HTTP local, vrai ffmpeg embarqué, vrai convertisseur Python
// ---------------------------------------------------------------------------------------------

/// Dossier `apps/flam-launcher/build` (sortie de `npm run build:ffmpeg` et `build:converter`).
fn build_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("build")
}

/// ffmpeg embarqué, sinon `None` (test ignoré) ; exigé avec `FLAM_REQUIRE_FFMPEG` (CI).
fn real_ffmpeg_dir() -> Option<PathBuf> {
    let dir = build_dir().join("ffmpeg");
    let present = ["ffmpeg.exe", "ffprobe.exe", "ffmpeg.sha256"]
        .iter()
        .all(|f| dir.join(f).is_file());
    if !present {
        assert!(
            std::env::var_os("FLAM_REQUIRE_FFMPEG").is_none(),
            "ffmpeg embarqué non construit alors que FLAM_REQUIRE_FFMPEG est défini"
        );
        eprintln!("ffmpeg embarqué non construit : test ignoré");
        return None;
    }
    Some(dir)
}

/// Python embarqué et `telmi2flam.py` voisin (`build/telmi2flam`), sinon `None` ; exigé avec
/// `FLAM_TEST_REQUIRE_PYTHON=1` (CI, après `npm run build:converter`).
fn embedded_converter() -> Option<(PathBuf, PathBuf)> {
    let root = std::env::var_os("FLAM_TEST_PYTHON")
        .map(PathBuf::from)
        .and_then(|p| p.parent()?.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| build_dir().join("telmi2flam"));
    let python = root.join("python").join("python.exe");
    let script = root.join("telmi2flam.py");
    if !python.is_file() || !script.is_file() {
        assert!(
            std::env::var_os("FLAM_TEST_REQUIRE_PYTHON").is_none_or(|v| v != "1"),
            "FLAM_TEST_REQUIRE_PYTHON=1 : Python embarqué introuvable"
        );
        eprintln!("Python embarqué absent : test ignoré");
        return None;
    }
    Some((python, script))
}

type Served = HashMap<String, (&'static str, Vec<u8>)>;

/// Serveur HTTP local : chemin -> (type, corps). Renvoie le port.
async fn serve_files(files: Served) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let files = Arc::new(files);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let files = Arc::clone(&files);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).into_owned();
                let target = text
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("/");
                let path = target.split('?').next().unwrap_or("/");
                let reply = match files.get(path) {
                    Some((kind, body)) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let mut out = head.into_bytes();
                        out.extend_from_slice(body);
                        out
                    }
                    None => {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    }
                };
                let _ = socket.write_all(&reply).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    port
}

/// MP3 réel (sinus) produit par le ffmpeg embarqué.
fn make_mp3(ffmpeg_dir: &Path, out: &Path, hz: u32, rate: u32, channels: u32) {
    let status = std::process::Command::new(ffmpeg_dir.join("ffmpeg.exe"))
        .args([
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ])
        .arg(format!("sine=frequency={hz}:duration=2:sample_rate={rate}"))
        .args(["-ac", &channels.to_string(), "-c:a", "libmp3lame"])
        .arg(out)
        .status()
        .expect("lancement de ffmpeg");
    assert!(status.success(), "ffmpeg n'a pas produit {}", out.display());
}

/// Accès réseau et audio réels, réseau de test (bouclage autorisé) ; flux en cache fournis.
struct LocalPacks {
    feeds: Feeds,
    ffmpeg: crate::ffmpeg::Ffmpeg,
    drafts: Mutex<Vec<u64>>,
}

impl PackBackend for LocalPacks {
    fn ready(&self) -> AppResult<()> {
        Ok(())
    }

    fn feeds<'a>(&'a self, _c: &'a PackComposition) -> BoxFuture<'a, AppResult<Feeds>> {
        Box::pin(async move { Ok(self.feeds.clone()) })
    }

    fn free_space(&self, _dir: &Path) -> Option<u64> {
        None
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
            let limits = crate::podcast::fetch::Limits {
                concurrency,
                ..Default::default()
            };
            let fetcher = crate::podcast::fetch::Fetcher::new(
                crate::podcast::net::NetPolicy::loopback_for_tests(),
            )?;
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
        Box::pin(crate::install::podcast::download_image(
            crate::podcast::net::NetPolicy::loopback_for_tests(),
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
            Ok(
                crate::podcast::audio::convert_all(&self.ffmpeg, items, cancel)
                    .await
                    .into_iter()
                    .map(|r| r.map(|c| c.output_bytes))
                    .collect(),
            )
        })
    }

    fn draft_done(&self, revision: u64) {
        lock(&self.drafts).push(revision);
    }

    // Décodage des pochettes dans le processus de test : l'enfant isolé est le binaire du
    // launcher, absent d'un `cargo test`.
    fn convert_artwork(
        &self,
        bytes: &[u8],
        kind: ArtworkKind,
    ) -> Result<Artwork, crate::podcast::artwork::ArtworkError> {
        Artwork::convert(bytes, kind)
    }
}

/// Pack complet, de bout en bout : enclosures et images servies par un serveur HTTP local, vrai
/// ffmpeg embarqué (MP3 sources de formats différents, un épisode introuvable), vrai
/// `telmi2flam.py` du Python embarqué, jusqu'à l'histoire publiée dans `library.json`. Ignoré sans
/// ffmpeg ou sans Python embarqué, sauf `FLAM_REQUIRE_FFMPEG` / `FLAM_TEST_REQUIRE_PYTHON=1` (CI).
#[tokio::test]
async fn bout_en_bout_pack_reel_jusqu_a_la_bibliotheque() {
    let Some(ffmpeg_dir) = real_ffmpeg_dir() else {
        return;
    };
    let Some((python, script)) = embedded_converter() else {
        return;
    };
    let work = tempfile::tempdir().unwrap();
    let mut files = Served::new();
    // g1 : mono 22 050 Hz, g2 : stéréo 48 000 Hz ; g3 n'est pas servi (404, P7).
    for (guid, hz, rate, channels) in [("g1", 440, 22_050, 1), ("g2", 660, 48_000, 2)] {
        let mp3 = work.path().join(format!("{guid}.mp3"));
        make_mp3(&ffmpeg_dir, &mp3, hz, rate, channels);
        files.insert(
            format!("/{guid}.mp3"),
            ("audio/mpeg", std::fs::read(&mp3).unwrap()),
        );
        files.insert(format!("/{guid}.png"), ("image/png", png()));
    }
    files.insert("/feed.png".into(), ("image/png", png()));
    let port = serve_files(files).await;

    let mut feeds = feeds(Fault::None);
    for cached in feeds.values_mut() {
        let CachedFeed::Ready(f) = cached else {
            continue;
        };
        f.image = Some(format!("http://127.0.0.1:{port}/feed.png"));
        for e in &mut f.episodes {
            e.image = Some(format!("http://127.0.0.1:{port}/{}.png", e.guid));
            e.enclosure_url = format!("http://127.0.0.1:{port}/{}.mp3?token=secret", e.guid);
            e.enclosure_length = None;
        }
    }
    let backend = Arc::new(LocalPacks {
        feeds: feeds.clone(),
        ffmpeg: crate::ffmpeg::Ffmpeg::in_dir(&ffmpeg_dir),
        drafts: Mutex::default(),
    });

    let ctx = Ctx::with_converter(
        tempfile::tempdir().unwrap(),
        Some(python),
        Duration::from_secs(5),
        Some(script),
    );
    ctx.service.set_pack_backend(backend.clone());
    let c = composition();
    let report = packmod::validate(&c, &feeds);
    assert!(report.valid, "{report:?}");
    let task = ctx.service.start_pack(c, &report, Some(7)).await.unwrap();
    let job_uuid = task.entry.uuid.clone();

    let done = ctx.finished(&task.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    assert_eq!(
        ctx.events.states(&task.id),
        [
            TaskState::Queued,
            TaskState::Preparing,
            TaskState::Prepared,
            TaskState::Converting,
            TaskState::Installing
        ]
    );
    // Épisode introuvable retiré (P7), les deux autres publiés.
    assert_eq!(done.pack.unwrap().dropped, ["Trois"]);

    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1, "{library:?}");
    let entry = &library[0];
    assert_eq!(entry.uuid, job_uuid);
    assert_eq!(entry.title, "Mon pack");
    assert!(entry.size > 0);
    // Fichier publié : un vrai `.plain.pk` (zip), de la taille enregistrée.
    let published = ctx.cache().join("library").join(&entry.file);
    let bytes = std::fs::read(&published).unwrap();
    assert_eq!(bytes.len() as u64, entry.size);
    assert_eq!(&bytes[..2], b"PK", "paquet zip");

    // Aucun chemin de l'archive de travail supprimée (ni nom d'utilisateur) dans `library.json`.
    assert!(entry.source.download_url.is_empty(), "{:?}", entry.source);
    let index = std::fs::read_to_string(ctx.cache().join("library").join("library.json")).unwrap();
    assert!(!index.contains("podcast-work"), "{index}");
    assert!(!index.contains("pack.zip"), "{index}");
    assert!(!index.contains("token=secret"), "{index}");
    assert!(!index.contains(&format!("127.0.0.1:{port}")), "{index}");

    ctx.assert_clean();
    assert_eq!(*lock(&backend.drafts), [7]);
}

/// L'entrée d'un pack ne garde aucun chemin local dans `library.json` : `library_source::check`
/// la juge inconnue (jamais lue) au lieu de suivre un lien mort.
#[tokio::test]
async fn pack_publie_sans_chemin_mort_et_source_inconnue() {
    let Some(py) = python() else { return };
    let ctx = Ctx::new(Some(py));
    ctx.packs(Fault::None);
    let task = ctx.start_pack().await;
    assert!(
        task.entry.download_url.contains("pack.zip"),
        "l'archive de travail sert encore à la conversion"
    );
    let done = ctx.finished(&task.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    // #436 : plus de chemin de l'archive de travail dans la tâche terminée.
    assert!(done.entry.download_url.is_empty(), "{:?}", done.entry);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert!(library[0].source.download_url.is_empty());
    let index = std::fs::read_to_string(ctx.cache().join("library").join("library.json")).unwrap();
    assert!(!index.contains("podcast-work"), "{index}");
    let lib = crate::library::Library::new(&ctx.cache());
    let state = crate::library_source::check(&library[0], &lib, &HashMap::new());
    assert_eq!(state, crate::library_source::SourceState::Unknown);
}
