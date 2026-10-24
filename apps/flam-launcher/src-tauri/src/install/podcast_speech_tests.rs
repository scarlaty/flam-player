//! Préparation d'un pack avec titres lus (#462, PR 2, note § 12) : contenu de l'archive (un
//! audio par option, `title.mp3`, `"audio": null` pour un titre muet) et injection de pannes
//! (voix en échec sur le premier titre ou sur plus de la moitié, ffmpeg absent, annulation
//! pendant la synthèse, voix disparue, aucune voix française). Voix simulée, aucun réseau.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use serde_json::Value;

use super::*;
use crate::install::{InstallDone, InstallEvents, InstallFailed, InstallOptions, InstallProgress};
use crate::podcast::fetch::Fetched;
use crate::podcast::pack::{CachedFeed, CoverChoice, PackMetadataInput};
use crate::podcast::rss::PodcastFeed;
use crate::podcast::speech::testing::{voice, Simulated};

struct NoEvents;

impl InstallEvents for NoEvents {
    fn progress(&self, _: &InstallProgress) {}
    fn done(&self, _: &InstallDone) {}
    fn failed(&self, _: &InstallFailed) {}
}

/// Trame MP3 silencieuse (MPEG1 Layer III, 128 kbit/s, 44 100 Hz) : MP3 de titre factice.
fn mp3_bytes() -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    frame.repeat(4)
}

fn png() -> Vec<u8> {
    Artwork::fallback(ArtworkKind::Episode, [1, 2, 3])
        .unwrap()
        .png
}

struct Fake {
    synth: Mutex<Arc<dyn Synthesizer>>,
    preferred: Mutex<Option<String>>,
    /// ffmpeg absent : la conversion des titres passe par le vrai `wav_to_mp3` sans exécutable.
    ffmpeg_missing: bool,
    fetches: AtomicUsize,
    titles_converted: AtomicUsize,
    missing_dir: PathBuf,
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
        enclosure_url: format!("https://audio.example.test/{guid}.mp3"),
        enclosure_type: "audio/mpeg".into(),
        enclosure_length: Some(1000),
        explicit: None,
    }
}

fn feeds() -> Feeds {
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
            CachedFeed::Ready(feed("Flux 1", vec![episode("g1", "Un")])),
        ),
        (
            "s2".to_string(),
            CachedFeed::Ready(feed(
                "Flux 2",
                vec![episode("g2", "Deux"), episode("g3", "Trois")],
            )),
        ),
    ])
}

/// Titres lus : « Mon pack », puis Un, Saison, Deux, Trois (5 titres à lire).
fn composition(voice: Option<&str>) -> PackComposition {
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
        menu: vec![
            ep("s1", "g1"),
            PackNode::Category {
                name: "Saison".into(),
                entries: vec![ep("s2", "g2"), ep("s2", "g3")],
            },
        ],
        titles_audio: true,
        titles_voice: voice.map(str::to_string),
    }
}

impl PackBackend for Fake {
    fn ready(&self) -> AppResult<()> {
        Ok(())
    }

    fn feeds<'a>(&'a self, _c: &'a PackComposition) -> BoxFuture<'a, AppResult<Feeds>> {
        Box::pin(async move { Ok(feeds()) })
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
            self.fetches.fetch_add(1, Ordering::SeqCst);
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
            Ok(items
                .iter()
                .map(|(from, to)| std::fs::copy(from, to).map_err(AppError::from))
                .collect())
        })
    }

    fn draft_done(&self, _revision: u64) {}

    fn convert_artwork(
        &self,
        bytes: &[u8],
        kind: ArtworkKind,
    ) -> Result<Artwork, crate::podcast::artwork::ArtworkError> {
        Artwork::convert(bytes, kind)
    }

    fn speech(&self) -> Arc<dyn Synthesizer> {
        self.synth.lock().unwrap().clone()
    }

    fn preferred_voice(&self) -> Option<String> {
        self.preferred.lock().unwrap().clone()
    }

    fn title_mp3<'a>(
        &'a self,
        wav: &'a Wav,
        dest: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<u64>> {
        Box::pin(async move {
            self.titles_converted.fetch_add(1, Ordering::SeqCst);
            if self.ffmpeg_missing {
                let ff = crate::ffmpeg::Ffmpeg::in_dir(&self.missing_dir);
                let mut cancel = cancel.clone();
                return speech::wav_to_mp3(&ff, wav, dest, &mut cancel).await;
            }
            let bytes = mp3_bytes();
            std::fs::write(dest, &bytes)?;
            Ok(bytes.len() as u64)
        })
    }
}

struct Ctx {
    _dir: tempfile::TempDir,
    service: InstallService,
    tc: Toolchain,
    fake: Arc<Fake>,
}

impl Ctx {
    fn new(synth: Simulated) -> Self {
        Self::with(synth, false)
    }

    fn with(synth: Simulated, ffmpeg_missing: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let cache_root = dir.path().join("cache");
        std::fs::create_dir_all(&cache_root).unwrap();
        let tc = Toolchain {
            cache_root,
            // Jamais lancé : la préparation s'arrête à l'archive.
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
            synth: Mutex::new(synth.shared()),
            preferred: Mutex::new(None),
            ffmpeg_missing,
            fetches: AtomicUsize::new(0),
            titles_converted: AtomicUsize::new(0),
            missing_dir: dir.path().join("ffmpeg-absent"),
        });
        service.set_pack_backend(fake.clone());
        Self {
            _dir: dir,
            service,
            tc,
            fake,
        }
    }

    fn job(composition: PackComposition) -> PackJob {
        PackJob {
            uuid: archive::new_pack_uuid().hyphenated().to_string(),
            composition,
            draft_revision: None,
        }
    }

    /// Prépare le pack ; en cas de succès, rend les entrées de l'archive et les avertissements,
    /// puis supprime le dossier de travail.
    async fn prepare(
        &self,
        composition: PackComposition,
    ) -> AppResult<(BTreeMap<String, Vec<u8>>, Vec<String>)> {
        let job = Self::job(composition);
        let id = uuid::Uuid::new_v4().to_string();
        // Tâche connue du service : les avertissements y sont rangés.
        self.register(&id);
        let (_tx, mut cancel) = watch::channel(false);
        let prepared = self
            .service
            .prepare_pack(&id, &job, &self.tc, &mut cancel)
            .await?;
        let files = read_zip(&prepared.archive);
        finish_workdir(prepared.workdir).await;
        Ok((files, self.warnings(&id)))
    }

    fn register(&self, id: &str) {
        let entry = pack_entry(
            "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
            &ResolvedMetadata {
                title: "Mon pack".into(),
                subtitle: String::new(),
                age: 0,
                description: None,
            },
            Path::new("pack.zip"),
        );
        let mut tasks = lock(&self.service.0.tasks);
        tasks.push(super::super::Slot {
            task: super::super::InstallTask {
                id: id.to_string(),
                entry,
                state: TaskState::Preparing,
                percent: None,
                error: None,
                created_at: 0,
                pack: Some(PackStatus::default()),
            },
            cancel: watch::channel(false).0,
            validator: Default::default(),
            pack: None,
        });
    }

    fn warnings(&self, id: &str) -> Vec<String> {
        lock(&self.service.0.tasks)
            .iter()
            .find(|s| s.task.id == id)
            .and_then(|s| s.task.pack.clone())
            .map(|p| p.warnings)
            .unwrap_or_default()
    }

    /// Aucun dossier de travail ne reste.
    fn assert_clean(&self) {
        let work = self.tc.cache_root.join(WORK_DIR);
        let left: Vec<_> = std::fs::read_dir(&work)
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        assert!(left.is_empty(), "dossiers de travail résiduels : {left:?}");
    }
}

fn read_zip(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        out.insert(f.name().to_string(), bytes);
    }
    out
}

/// Audio de chaque scène d'option, par libellé (`notes.json`).
fn option_audios(files: &BTreeMap<String, Vec<u8>>) -> BTreeMap<String, Value> {
    let nodes: Value = serde_json::from_slice(&files["nodes.json"]).unwrap();
    let notes: Value = serde_json::from_slice(&files["notes.json"]).unwrap();
    nodes["stages"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(id, _)| id.starts_with('o'))
        .map(|(id, st)| {
            (
                notes[id]["title"].as_str().unwrap().to_string(),
                st["audio"].clone(),
            )
        })
        .collect()
}

fn two_voices() -> Simulated {
    Simulated::new(vec![
        voice("fr-a", "Microsoft A", "fr-FR"),
        voice("fr-b", "Microsoft B", "fr-FR"),
    ])
}

#[tokio::test]
async fn chaque_option_a_son_titre_lu_et_le_pack_son_title_mp3() {
    let ctx = Ctx::new(two_voices());
    let (files, warnings) = ctx.prepare(composition(Some("fr-b"))).await.unwrap();
    assert!(warnings.is_empty(), "{warnings:?}");
    let audios = option_audios(&files);
    assert_eq!(audios.len(), 4, "{audios:?}");
    for (label, audio) in &audios {
        let name = audio
            .as_str()
            .unwrap_or_else(|| panic!("option « {label} » muette"));
        assert!(
            files.contains_key(&format!("audios/{name}")),
            "{label} : {name} absent"
        );
    }
    assert_eq!(audios["Saison"], "c001.mp3");
    assert_eq!(audios["Un"], "t001.mp3");
    assert_eq!(audios["Trois"], "t003.mp3");
    assert_eq!(files["title.mp3"], mp3_bytes());
    assert_eq!(ctx.fake.titles_converted.load(Ordering::SeqCst), 5);
    ctx.assert_clean();
}

#[tokio::test]
async fn sans_l_option_rien_n_est_lu_et_l_archive_reste_muette() {
    // Synthèse indisponible : jamais appelée sans l'option.
    let ctx = Ctx::new(Simulated {
        unavailable: true,
        ..Simulated::french()
    });
    let mut c = composition(None);
    c.titles_audio = false;
    let (files, warnings) = ctx.prepare(c).await.unwrap();
    assert!(warnings.is_empty());
    assert!(option_audios(&files).values().all(Value::is_null));
    assert!(!files.contains_key("title.mp3"));
    assert_eq!(ctx.fake.titles_converted.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn un_titre_isole_en_echec_reste_muet_avec_un_avertissement() {
    let ctx = Ctx::new(Simulated {
        failing: vec!["Deux".into()],
        ..Simulated::french()
    });
    let (files, warnings) = ctx.prepare(composition(None)).await.unwrap();
    let audios = option_audios(&files);
    assert!(audios["Deux"].is_null(), "{audios:?}");
    assert!(!files.contains_key("audios/t002.mp3"));
    assert_eq!(audios["Trois"], "t003.mp3");
    assert!(files.contains_key("title.mp3"));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("« Deux »"), "{warnings:?}");
    ctx.assert_clean();
}

#[tokio::test]
async fn le_premier_titre_en_echec_arrete_avant_tout_telechargement() {
    let ctx = Ctx::new(Simulated {
        failing: vec!["Mon pack".into()],
        ..Simulated::french()
    });
    let err = ctx.prepare(composition(None)).await.unwrap_err();
    assert_eq!(err.kind(), "tts_failed", "{err:?}");
    assert!(err.to_string().contains("premier titre"), "{err}");
    assert_eq!(
        ctx.fake.fetches.load(Ordering::SeqCst),
        0,
        "rien téléchargé"
    );
    ctx.assert_clean();
}

#[tokio::test]
async fn plus_de_la_moitie_des_titres_en_echec_arrete_la_creation() {
    // 3 titres sur 5 en échec (le premier réussit).
    let ctx = Ctx::new(Simulated {
        failing: vec!["Un".into(), "Saison".into(), "Deux".into()],
        ..Simulated::french()
    });
    let err = ctx.prepare(composition(None)).await.unwrap_err();
    assert_eq!(err.kind(), "tts_failed", "{err:?}");
    assert!(err.to_string().contains("3 sur 5"), "{err}");
    assert_eq!(ctx.fake.fetches.load(Ordering::SeqCst), 0);
    ctx.assert_clean();
    // Exactement la moitié ou moins : le pack est créé (2 sur 5).
    let ctx = Ctx::new(Simulated {
        failing: vec!["Un".into(), "Deux".into()],
        ..Simulated::french()
    });
    let (_, warnings) = ctx.prepare(composition(None)).await.unwrap();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
}

#[tokio::test]
async fn ffmpeg_absent_arrete_la_creation_sans_fichier_residuel() {
    let ctx = Ctx::with(Simulated::french(), true);
    let err = ctx.prepare(composition(None)).await.unwrap_err();
    assert_eq!(err.kind(), "tts_failed", "{err:?}");
    assert_eq!(ctx.fake.titles_converted.load(Ordering::SeqCst), 1);
    assert_eq!(ctx.fake.fetches.load(Ordering::SeqCst), 0);
    ctx.assert_clean();
}

#[tokio::test]
async fn annulation_pendant_la_synthese_sans_attendre_la_voix() {
    let ctx = Ctx::new(Simulated {
        delay: Duration::from_secs(3),
        ..Simulated::french()
    });
    let job = Ctx::job(composition(None));
    let id = uuid::Uuid::new_v4().to_string();
    ctx.register(&id);
    let (tx, mut cancel) = watch::channel(false);
    let started = Instant::now();
    let canceller = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        tx.send(true).unwrap();
        tx
    });
    let result = ctx
        .service
        .prepare_pack(&id, &job, &ctx.tc, &mut cancel)
        .await;
    let _tx = canceller.await.unwrap();
    assert!(
        matches!(result, Err(AppError::Cancelled)),
        "{:?}",
        result.err()
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "annulation sans attendre la fin de la synthèse : {:?}",
        started.elapsed()
    );
    assert_eq!(ctx.fake.fetches.load(Ordering::SeqCst), 0);
    ctx.assert_clean();
}

#[tokio::test]
async fn voix_disparue_repli_sur_la_voix_par_defaut_avec_un_avertissement() {
    let ctx = Ctx::new(two_voices());
    let (files, warnings) = ctx.prepare(composition(Some("disparue"))).await.unwrap();
    assert_eq!(warnings, [pack_speech::VOICE_FALLBACK_WARNING]);
    assert!(option_audios(&files).values().all(Value::is_string));
}

#[tokio::test]
async fn plus_aucune_voix_francaise_a_la_reprise_arrete_avant_toute_ecriture() {
    let ctx = Ctx::new(Simulated::new(vec![voice("en", "Zira", "en-US")]));
    let err = ctx.prepare(composition(Some("fr-a"))).await.unwrap_err();
    assert_eq!(err.kind(), "tts_no_voice", "{err:?}");
    assert_eq!(ctx.fake.fetches.load(Ordering::SeqCst), 0);
    ctx.assert_clean();
}

#[tokio::test]
async fn sans_voix_francaise_la_creation_de_la_tache_est_refusee() {
    let ctx = Ctx::new(Simulated::none());
    let c = composition(None);
    let report = crate::podcast::pack::validate(&c, &feeds());
    assert!(report.valid, "{report:?}");
    let err = ctx.service.start_pack(c, &report, None).await.unwrap_err();
    assert_eq!(err.kind(), "tts_no_voice");
    assert!(ctx.service.list().is_empty(), "aucune tâche créée");
    // Synthèse indisponible (autre système, service arrêté) : refus aussi.
    *ctx.fake.synth.lock().unwrap() = Simulated {
        unavailable: true,
        ..Simulated::french()
    }
    .shared();
    let c = composition(None);
    let err = ctx.service.start_pack(c, &report, None).await.unwrap_err();
    assert_eq!(err.kind(), "tts_unavailable");
    assert!(ctx.service.list().is_empty());
}

#[tokio::test]
async fn la_voix_memorisee_du_poste_est_enregistree_avec_la_tache() {
    let ctx = Ctx::new(two_voices());
    *ctx.fake.preferred.lock().unwrap() = Some(" fr-b ".into());
    let mut c = composition(None);
    ctx.service.check_speech(&mut c).await.unwrap();
    assert_eq!(c.titles_voice.as_deref(), Some("fr-b"));
    // Voix choisie dans la composition : prioritaire sur celle du poste.
    let mut c = composition(Some("fr-a"));
    ctx.service.check_speech(&mut c).await.unwrap();
    assert_eq!(c.titles_voice.as_deref(), Some("fr-a"));
    // Identifiant invalide : refusé.
    let mut c = composition(Some("a\u{7}b"));
    assert_eq!(
        ctx.service.check_speech(&mut c).await.unwrap_err().kind(),
        "validation"
    );
}
