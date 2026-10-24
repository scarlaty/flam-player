//! Préparation d'un pack avec le cache des MP3 convertis (#460) : un pack refait ne
//! retélécharge ni ne reconvertit, une entrée tronquée est retéléchargée, un disque plein ou
//! une source supprimée pendant la création ne font pas échouer le pack.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use super::*;
use crate::install::{InstallDone, InstallEvents, InstallFailed, InstallOptions, InstallProgress};
use crate::podcast::fetch::Fetched;
use crate::podcast::mp3cache::tests::{episode, TestLocks};
use crate::podcast::mp3cache::{link_or_copy, Cancel, Placer, CACHE_DIR};
use crate::podcast::pack::{CachedFeed, CoverChoice, PackMetadataInput};
use crate::podcast::rss::PodcastFeed;
use crate::podcast::source::source_dir;

struct NoEvents;

impl InstallEvents for NoEvents {
    fn progress(&self, _: &InstallProgress) {}
    fn done(&self, _: &InstallDone) {}
    fn failed(&self, _: &InstallFailed) {}
}

/// Panne injectée.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    None,
    /// Disque plein à chaque écriture en cache.
    DiskFull,
    /// La source `s1` est supprimée pendant la conversion audio.
    PurgeDuringAudio,
    /// Erreur disque au téléchargement de la pochette de `g2`.
    ImageError,
    /// Annulation de la tâche pendant la première copie depuis le cache.
    CancelInCache,
}

struct Fake {
    cache_root: PathBuf,
    locks: Arc<TestLocks>,
    fault: Mutex<Fault>,
    /// Version des URL de pochettes d'épisode (0 : celles de `episode`).
    image_version: Mutex<u32>,
    /// Couverture de la composition.
    cover: Mutex<CoverChoice>,
    /// Identifiants des épisodes demandés au téléchargeur, par lot.
    fetched: Mutex<Vec<Vec<String>>>,
    converted: AtomicUsize,
    images: AtomicUsize,
    /// Téléchargements d'images simultanés : en cours, maximum observé.
    images_in_flight: AtomicUsize,
    images_max_in_flight: AtomicUsize,
    /// Annulation de la préparation en cours.
    cancel: Mutex<Option<Arc<watch::Sender<bool>>>>,
}

fn png() -> Vec<u8> {
    Artwork::fallback(ArtworkKind::Episode, [1, 2, 3])
        .unwrap()
        .png
}

fn feeds(image_version: u32) -> Feeds {
    let ep = |guid: &str| {
        let mut e = episode(guid);
        if image_version > 0 {
            e.image = Some(format!(
                "https://img.example.test/{guid}-v{image_version}.png"
            ));
        }
        e
    };
    let feed = |title: &str, episodes| PodcastFeed {
        title: title.into(),
        description: None,
        image: Some("https://img.example.test/feed.png".into()),
        link: None,
        copyright: None,
        explicit: None,
        episodes,
        warnings: Vec::new(),
    };
    Feeds::from([
        (
            "s1".to_string(),
            CachedFeed::Ready(feed("Flux 1", vec![ep("g1"), ep("g2")])),
        ),
        (
            "s2".to_string(),
            CachedFeed::Ready(feed("Flux 2", vec![ep("g3")])),
        ),
    ])
}

fn composition(cover: CoverChoice) -> PackComposition {
    let ep = |s: &str, g: &str| {
        PackNode::Episode(EpisodeRef {
            source_id: s.into(),
            episode_id: crate::podcast::source::episode_id(g),
        })
    };
    PackComposition {
        metadata: PackMetadataInput {
            title: "Mon pack".into(),
            ..PackMetadataInput::default()
        },
        cover,
        menu: vec![ep("s1", "g1"), ep("s1", "g2"), ep("s2", "g3")],
        ..PackComposition::default()
    }
}

impl PackBackend for Fake {
    fn ready(&self) -> AppResult<()> {
        Ok(())
    }

    fn feeds<'a>(&'a self, _c: &'a PackComposition) -> BoxFuture<'a, AppResult<Feeds>> {
        Box::pin(async move { Ok(feeds(*self.image_version.lock().unwrap())) })
    }

    fn free_space(&self, _dir: &Path) -> Option<u64> {
        None
    }

    fn fetch<'a>(
        &'a self,
        items: &'a [Item],
        dir: &'a Workdir,
        _concurrency: usize,
        _cancel: watch::Receiver<bool>,
        _on_progress: Arc<dyn Fn(Progress) + Send + Sync>,
    ) -> BoxFuture<'a, AppResult<Report>> {
        Box::pin(async move {
            self.fetched
                .lock()
                .unwrap()
                .push(items.iter().map(|i| i.id.clone()).collect());
            let mut report = Report::default();
            for (n, item) in items.iter().enumerate() {
                let audio = dir.path().join(format!("ep-{n:04}.audio"));
                std::fs::write(&audio, format!("ID3 audio {}", item.id)).unwrap();
                let image = dir.path().join(format!("ep-{n:04}.img"));
                std::fs::write(&image, png()).unwrap();
                report.fetched.push(Fetched {
                    id: item.id.clone(),
                    feed: item.feed.clone(),
                    audio_bytes: std::fs::metadata(&audio).unwrap().len(),
                    audio,
                    image: Some(image),
                });
            }
            Ok(report)
        })
    }

    fn fetch_image<'a>(
        &'a self,
        url: &'a str,
        dest: &'a Path,
        _cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            let fault = *self.fault.lock().unwrap();
            let running = self.images_in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            self.images_max_in_flight
                .fetch_max(running, Ordering::SeqCst);
            // Panne `ImageError` : les autres pochettes sont lentes (doivent être arrêtées).
            let slow = fault == Fault::ImageError && !url.contains("/g2");
            tokio::time::sleep(Duration::from_millis(if slow { 2000 } else { 30 })).await;
            self.images_in_flight.fetch_sub(1, Ordering::SeqCst);
            self.images.fetch_add(1, Ordering::SeqCst);
            if fault == Fault::ImageError && url.contains("/g2") {
                return Err(AppError::Io(std::io::Error::new(
                    std::io::ErrorKind::StorageFull,
                    "disque plein",
                )));
            }
            std::fs::write(dest, png())?;
            Ok(true)
        })
    }

    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>> {
        Box::pin(async move {
            if *self.fault.lock().unwrap() == Fault::PurgeDuringAudio {
                self.locks.purge(&self.cache_root, "s1");
            }
            self.converted.fetch_add(items.len(), Ordering::SeqCst);
            Ok(items
                .iter()
                .map(|(from, to)| std::fs::copy(from, to).map_err(AppError::from))
                .collect())
        })
    }

    fn draft_done(&self, _revision: u64) {}

    fn mp3_cache(&self, cache_root: PathBuf, sources: BTreeSet<String>) -> Option<CacheAccess> {
        let access = CacheAccess::new(self.locks.clone(), cache_root, sources);
        Some(match *self.fault.lock().unwrap() {
            Fault::DiskFull => {
                let full: Arc<Placer> = Arc::new(|_: &Path, dest: &Path, _: &Cancel| {
                    std::fs::write(dest, b"ID3 tron")?;
                    Err(std::io::Error::new(
                        std::io::ErrorKind::StorageFull,
                        "disque plein",
                    ))
                });
                access.with_placer(full)
            }
            Fault::CancelInCache => {
                let tx = self.cancel.lock().unwrap().clone().unwrap();
                let cancelling: Arc<Placer> =
                    Arc::new(move |src: &Path, dest: &Path, cancel: &Cancel| {
                        tx.send_replace(true);
                        link_or_copy(src, dest, cancel)
                    });
                access.with_placer(cancelling)
            }
            _ => access,
        })
    }

    fn convert_artwork(
        &self,
        bytes: &[u8],
        kind: ArtworkKind,
    ) -> Result<Artwork, crate::podcast::artwork::ArtworkError> {
        Artwork::convert(bytes, kind)
    }
}

struct Ctx {
    _dir: tempfile::TempDir,
    service: InstallService,
    tc: Toolchain,
    fake: Arc<Fake>,
    /// MP3 de la dernière archive préparée, dans l'ordre des épisodes.
    audios: Mutex<Vec<String>>,
}

/// Contenu des `audios/*.mp3` d'une archive Telmi, dans l'ordre des noms (`e001`, `e002`…).
fn archive_audios(path: &Path) -> Vec<String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut names: Vec<String> = zip
        .file_names()
        .filter(|n| n.starts_with("audios/"))
        .map(str::to_string)
        .collect();
    names.sort();
    names
        .iter()
        .map(|n| {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut zip.by_name(n).unwrap(), &mut text).unwrap();
            text
        })
        .collect()
}

impl Ctx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&cache_root).unwrap();
        let tc = Toolchain {
            cache_root: cache_root.clone(),
            // Jamais lancé : la préparation s'arrête avant la conversion.
            rust_converter: Some(dir.path().join("telmi2flam.exe")),
            ffmpeg: None,
            max_downloads: 2,
            thumb_fit: crate::settings::ThumbFit::default(),
        };
        let resolved = tc.clone();
        let service = InstallService::new(InstallOptions {
            client: super::super::download::build_client().unwrap(),
            resolver: Arc::new(move || Ok(resolved.clone())),
            events: Arc::new(NoEvents),
            queue_path: dir.path().join("config").join(super::super::QUEUE_FILE),
            gate: Arc::default(),
            grace: Duration::from_secs(5),
        });
        let fake = Arc::new(Fake {
            cache_root,
            locks: Arc::new(TestLocks::default()),
            fault: Mutex::new(Fault::None),
            image_version: Mutex::new(0),
            cover: Mutex::new(CoverChoice::Feed {
                source_id: "s1".into(),
            }),
            fetched: Mutex::default(),
            converted: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
            images_in_flight: AtomicUsize::new(0),
            images_max_in_flight: AtomicUsize::new(0),
            cancel: Mutex::default(),
        });
        service.set_pack_backend(fake.clone());
        Self {
            _dir: dir,
            service,
            tc,
            fake,
            audios: Mutex::default(),
        }
    }

    /// Prépare un pack (tâche neuve) jusqu'à l'archive, puis supprime le dossier de travail.
    async fn prepare(&self, fault: Fault) -> Vec<String> {
        self.try_prepare(fault).await.expect("pack préparé")
    }

    /// Comme [`Ctx::prepare`] ; rend les épisodes téléchargés, ou l'erreur de la préparation.
    /// Les MP3 de l'archive sont relevés dans `audios`.
    async fn try_prepare(&self, fault: Fault) -> AppResult<Vec<String>> {
        *self.fake.fault.lock().unwrap() = fault;
        self.fake.fetched.lock().unwrap().clear();
        let job = PackJob {
            uuid: archive::new_pack_uuid().hyphenated().to_string(),
            composition: composition(self.fake.cover.lock().unwrap().clone()),
            draft_revision: None,
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, mut cancel) = watch::channel(false);
        *self.fake.cancel.lock().unwrap() = Some(Arc::new(tx));
        let prepared = self
            .service
            .prepare_pack(&id, &job, &self.tc, &mut cancel)
            .await?;
        assert!(prepared.archive.is_file());
        *self.audios.lock().unwrap() = archive_audios(&prepared.archive);
        finish_workdir(prepared.workdir).await;
        Ok(self.fake.fetched.lock().unwrap().concat())
    }

    fn mp3_dir(&self, source: &str) -> PathBuf {
        source_dir(&self.tc.cache_root, source)
            .unwrap()
            .join(CACHE_DIR)
    }

    fn entries(&self, source: &str) -> usize {
        std::fs::read_dir(self.mp3_dir(source))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".json"))
                    .count()
            })
            .unwrap_or(0)
    }

    fn leftovers(&self) -> Vec<String> {
        let mut out = Vec::new();
        for s in ["s1", "s2"] {
            if let Ok(d) = std::fs::read_dir(self.mp3_dir(s)) {
                for e in d.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.ends_with(".part") || name.ends_with(".tmp") {
                        out.push(name);
                    }
                }
            }
        }
        out
    }
}

#[tokio::test]
async fn pack_refait_sans_retelecharger_ni_reconvertir() {
    let ctx = Ctx::new();
    assert_eq!(ctx.prepare(Fault::None).await, ["e0", "e1", "e2"]);
    assert_eq!(ctx.fake.converted.load(Ordering::SeqCst), 3);
    assert_eq!((ctx.entries("s1"), ctx.entries("s2")), (2, 1));
    // Seule la couverture (image du flux) est téléchargée à part.
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 1);

    // Deuxième pack : tout vient du cache (aucun téléchargement, aucune conversion, aucune
    // pochette d'épisode retéléchargée).
    assert!(ctx.prepare(Fault::None).await.is_empty());
    assert_eq!(ctx.fake.converted.load(Ordering::SeqCst), 3);
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 2);
    // Aucun dossier de travail résiduel.
    let work = ctx.tc.cache_root.join(WORK_DIR);
    assert_eq!(std::fs::read_dir(&work).map_or(0, |d| d.count()), 0);
}

/// Panne : MP3 en cache tronqué : seul cet épisode est retéléchargé, puis remis en cache.
#[tokio::test]
async fn entree_tronquee_retelechargee() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::None).await;
    let id = crate::podcast::source::episode_id("g2");
    let mp3 = ctx.mp3_dir("s1").join(format!("{id}.mp3"));
    std::fs::OpenOptions::new()
        .write(true)
        .open(&mp3)
        .unwrap()
        .set_len(3)
        .unwrap();
    assert_eq!(ctx.prepare(Fault::None).await, ["e1"]);
    assert_eq!(ctx.fake.converted.load(Ordering::SeqCst), 4);
    // Cache partiel : chaque rang de l'archive a le bon MP3 (servi ou retéléchargé).
    let expected = ["ID3 audio e0", "ID3 audio e1", "ID3 audio e2"];
    assert_eq!(*ctx.audios.lock().unwrap(), expected);
    assert!(ctx.prepare(Fault::None).await.is_empty(), "remis en cache");
    assert_eq!(*ctx.audios.lock().unwrap(), expected);
}

/// Panne : disque plein à l'écriture en cache : le pack est créé, le cache reste vide et
/// sans reste.
#[tokio::test]
async fn disque_plein_le_pack_est_cree_sans_cache() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::DiskFull).await;
    assert_eq!((ctx.entries("s1"), ctx.entries("s2")), (0, 0));
    assert!(ctx.leftovers().is_empty(), "{:?}", ctx.leftovers());
    // Le pack suivant retélécharge tout.
    assert_eq!(ctx.prepare(Fault::None).await, ["e0", "e1", "e2"]);
}

/// Panne : source supprimée pendant la création : le pack est créé, le dossier de la source
/// n'est pas recréé ; l'autre source est mise en cache.
#[tokio::test]
async fn purge_pendant_la_creation() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::PurgeDuringAudio).await;
    assert!(!source_dir(&ctx.tc.cache_root, "s1").unwrap().exists());
    assert_eq!(ctx.entries("s2"), 1);
}

/// Pochette d'un épisode servi par le cache changée : retéléchargée une fois, puis remise en
/// cache avec le MP3.
#[tokio::test]
async fn pochette_changee_remise_en_cache() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::None).await;
    assert_eq!(
        ctx.fake.images.load(Ordering::SeqCst),
        1,
        "couverture seule"
    );
    *ctx.fake.image_version.lock().unwrap() = 1;
    assert!(ctx.prepare(Fault::None).await.is_empty());
    // Couverture + trois pochettes d'épisode retéléchargées.
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 5);
    assert!(ctx.prepare(Fault::None).await.is_empty());
    // Couverture seule : les pochettes viennent du cache.
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 6);
    assert_eq!(ctx.fake.converted.load(Ordering::SeqCst), 3);
    assert_eq!((ctx.entries("s1"), ctx.entries("s2")), (2, 1));
}

/// Pochettes des épisodes servis par le cache : téléchargées en parallèle, sans dépasser la
/// concurrence des téléchargements (`max_downloads` : 2).
#[tokio::test]
async fn pochettes_du_cache_telechargees_en_parallele_bornees() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::None).await;
    *ctx.fake.image_version.lock().unwrap() = 1;
    ctx.fake.images_max_in_flight.store(0, Ordering::SeqCst);
    assert!(ctx.prepare(Fault::None).await.is_empty());
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 5);
    assert_eq!(ctx.fake.images_max_in_flight.load(Ordering::SeqCst), 2);
}

/// Panne : erreur disque au téléchargement d'une pochette (parmi plusieurs en parallèle) :
/// le pack échoue, les autres téléchargements sont arrêtés, le cache reste intact.
#[tokio::test]
async fn erreur_d_une_pochette_du_cache_arrete_les_autres() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::None).await;
    *ctx.fake.image_version.lock().unwrap() = 1;
    let err = ctx.try_prepare(Fault::ImageError).await.unwrap_err();
    assert!(matches!(err, AppError::Io(_)), "{err:?}");
    // Couverture du premier pack + pochette de g2 : celle de g1, lancée en même temps, est
    // arrêtée et celle de g3 jamais lancée.
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 2);
    // Le pack suivant vient toujours du cache.
    assert!(ctx.prepare(Fault::None).await.is_empty());
    assert_eq!(ctx.fake.converted.load(Ordering::SeqCst), 3);
    let work = ctx.tc.cache_root.join(WORK_DIR);
    assert_eq!(std::fs::read_dir(&work).map_or(0, |d| d.count()), 0);
}

/// Couverture choisie parmi les épisodes : la pochette servie par le cache est reprise, sans
/// téléchargement.
#[tokio::test]
async fn couverture_d_episode_reprise_du_cache() {
    let ctx = Ctx::new();
    *ctx.fake.cover.lock().unwrap() = CoverChoice::Episode(EpisodeRef {
        source_id: "s1".into(),
        episode_id: crate::podcast::source::episode_id("g2"),
    });
    ctx.prepare(Fault::None).await;
    // Couverture prise dans les pochettes téléchargées avec les épisodes.
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 0);
    assert!(ctx.prepare(Fault::None).await.is_empty());
    // Pack refait : prise dans la pochette en cache.
    assert_eq!(ctx.fake.images.load(Ordering::SeqCst), 0);
}

/// Panne : annulation pendant la lecture du cache : la préparation est annulée et aucune
/// entrée n'est supprimée (le pack suivant vient entièrement du cache).
#[tokio::test]
async fn annulation_pendant_la_lecture_du_cache() {
    let ctx = Ctx::new();
    ctx.prepare(Fault::None).await;
    let err = ctx.try_prepare(Fault::CancelInCache).await.unwrap_err();
    assert!(matches!(err, AppError::Cancelled), "{err:?}");
    assert_eq!((ctx.entries("s1"), ctx.entries("s2")), (2, 1));
    assert!(ctx.prepare(Fault::None).await.is_empty());
    assert_eq!(ctx.fake.converted.load(Ordering::SeqCst), 3);
}
