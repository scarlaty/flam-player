//! Packs de podcasts dans la file (#217) : création complète, injection de pannes à chaque
//! étape (état final, aucun fichier résiduel, `library.json` cohérent), reprise après un arrêt
//! brutal (même uuid), composition invalide refusée.

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
}

struct FakePacks {
    fault: Fault,
    entered: AtomicBool,
    drafts: Mutex<Vec<u64>>,
}

impl FakePacks {
    fn new(fault: Fault) -> Arc<Self> {
        Arc::new(Self {
            fault,
            entered: AtomicBool::new(false),
            drafts: Mutex::default(),
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
        mut cancel: watch::Receiver<bool>,
        _on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> BoxFuture<'a, AppResult<Report>> {
        Box::pin(async move {
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
}

impl Ctx {
    fn packs(&self, fault: Fault) -> Arc<FakePacks> {
        let fake = FakePacks::new(fault);
        self.service.set_pack_backend(fake.clone());
        fake
    }

    fn start_pack(&self) -> InstallTask {
        let c = composition();
        let report = packmod::validate(&c, &feeds(Fault::None));
        assert!(report.valid, "{report:?}");
        self.service.start_pack(c, &report, Some(7)).unwrap()
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
    let task = ctx.start_pack();
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
    // P7 : l'épisode en échec est retiré et signalé.
    assert_eq!(done.pack.unwrap().dropped, ["Deux"]);
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
    ] {
        let ctx = Ctx::new(Some(py.clone()));
        install_directly(&ctx, "deja", b"PK");
        let fake = ctx.packs(fault);
        let task = ctx.start_pack();
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
        draft_revision: None,
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
    let err = ctx.service.start_pack(c, &report, None).unwrap_err();
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
    assert!(ctx.service.start_pack(deep, &report, None).is_err());
    assert!(ctx.service.list().is_empty());
    assert!(ctx.queue().is_empty());
}

#[tokio::test]
async fn sans_python_le_pack_echoue_avant_tout_telechargement() {
    let ctx = Ctx::new(None);
    let fake = ctx.packs(Fault::None);
    let task = ctx.start_pack();
    let end = ctx.finished(&task.id).await;
    assert_eq!(end.error.unwrap().kind, "converter_unavailable");
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
