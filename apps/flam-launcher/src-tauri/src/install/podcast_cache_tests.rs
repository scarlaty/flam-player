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
use crate::podcast::mp3cache::{Placer, CACHE_DIR};
use crate::podcast::pack::{CachedFeed, CoverChoice, PackMetadataInput};
use crate::podcast::rss::PodcastFeed;
use crate::podcast::source::source_dir;
use crate::settings::ConverterEngine;

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
}

struct Fake {
    cache_root: PathBuf,
    locks: Arc<TestLocks>,
    fault: Mutex<Fault>,
    /// Version des URL de pochettes d'épisode (0 : celles de `episode`).
    image_version: Mutex<u32>,
    /// Identifiants des épisodes demandés au téléchargeur, par lot.
    fetched: Mutex<Vec<Vec<String>>>,
    converted: AtomicUsize,
    images: AtomicUsize,
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

fn composition() -> PackComposition {
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
        cover: CoverChoice::Feed {
            source_id: "s1".into(),
        },
        menu: vec![ep("s1", "g1"), ep("s1", "g2"), ep("s2", "g3")],
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
        _url: &'a str,
        dest: &'a Path,
        _cancel: watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<bool>> {
        Box::pin(async move {
            self.images.fetch_add(1, Ordering::SeqCst);
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
                let full: Arc<Placer> = Arc::new(|_: &Path, dest: &Path| {
                    std::fs::write(dest, b"ID3 tron")?;
                    Err(std::io::Error::new(
                        std::io::ErrorKind::StorageFull,
                        "disque plein",
                    ))
                });
                access.with_placer(full)
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
}

impl Ctx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&cache_root).unwrap();
        let tc = Toolchain {
            cache_root: cache_root.clone(),
            engine: ConverterEngine::Rust,
            python: None,
            converter: None,
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
            fetched: Mutex::default(),
            converted: AtomicUsize::new(0),
            images: AtomicUsize::new(0),
        });
        service.set_pack_backend(fake.clone());
        Self {
            _dir: dir,
            service,
            tc,
            fake,
        }
    }

    /// Prépare un pack (tâche neuve) jusqu'à l'archive, puis supprime le dossier de travail.
    async fn prepare(&self, fault: Fault) -> Vec<String> {
        *self.fake.fault.lock().unwrap() = fault;
        self.fake.fetched.lock().unwrap().clear();
        let job = PackJob {
            uuid: archive::new_pack_uuid().hyphenated().to_string(),
            composition: composition(),
            draft_revision: None,
        };
        let id = uuid::Uuid::new_v4().to_string();
        let (_tx, mut cancel) = watch::channel(false);
        let prepared = self
            .service
            .prepare_pack(&id, &job, &self.tc, &mut cancel)
            .await
            .expect("pack préparé");
        assert!(prepared.archive.is_file());
        finish_workdir(prepared.workdir).await;
        self.fake.fetched.lock().unwrap().concat()
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
    assert!(ctx.prepare(Fault::None).await.is_empty(), "remis en cache");
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
