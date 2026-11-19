//! Étape commune de fabrication d'un pack (lot 4) à partir de fichiers **locaux**, sans flux ni
//! épisode : accès simulés ([`BuildBackend`] seul), voix simulée, aucun réseau ni ffmpeg.

use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Mutex;

use serde_json::Value;

use super::*;
use crate::install::podcast::{pack_entry, PackStatus};
use crate::install::{
    InstallDone, InstallEvents, InstallFailed, InstallOptions, InstallProgress, InstallTask, Slot,
    QUEUE_FILE,
};
use crate::podcast::speech::testing::Simulated;

/// Phase et pourcentage de chaque avancement.
#[derive(Default)]
struct Events {
    phases: Mutex<Vec<(String, f64)>>,
}

impl InstallEvents for Events {
    fn progress(&self, p: &InstallProgress) {
        if let Some(phase) = &p.phase {
            lock(&self.phases).push((phase.clone(), p.percent.unwrap_or(-1.0)));
        }
    }
    fn done(&self, _: &InstallDone) {}
    fn failed(&self, _: &InstallFailed) {}
}

/// Trame MP3 silencieuse répétée (titre lu factice).
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

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Fault {
    #[default]
    None,
    /// ffprobe absent : arrêt du pack.
    ConverterGone,
}

/// Accès de fabrication seuls (aucune notion de flux) : la conversion audio copie la source,
/// sauf les sources dont le nom est dans `failing`.
#[derive(Default)]
struct Local {
    fault: Fault,
    failing: Vec<String>,
    /// Sources converties, dans l'ordre des demandes.
    converted: Mutex<Vec<PathBuf>>,
}

impl BuildBackend for Local {
    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>> {
        Box::pin(async move {
            lock(&self.converted).extend(items.iter().map(|(from, _)| from.clone()));
            Ok(items
                .iter()
                .map(|(from, to)| {
                    if self.fault == Fault::ConverterGone {
                        return Err(AppError::ConverterUnavailable("ffprobe.exe absent".into()));
                    }
                    let name = from.file_name().unwrap().to_string_lossy().into_owned();
                    if self.failing.contains(&name) {
                        return Err(AppError::Conversion {
                            code: "ffmpeg_failed".into(),
                            message: "échec simulé".into(),
                        });
                    }
                    std::fs::copy(from, to).map_err(AppError::from)
                })
                .collect())
        })
    }

    fn convert_artwork(&self, bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
        Artwork::convert(bytes, kind)
    }

    fn speech(&self) -> Arc<dyn Synthesizer> {
        Simulated::french().shared()
    }

    fn title_encoder<'a>(
        &'a self,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Verified>> {
        Box::pin(async { Ok(Verified::unchecked("ffmpeg-factice")) })
    }

    fn title_mp3<'a>(
        &'a self,
        _encoder: &'a Verified,
        _wav: &'a Wav,
        dest: &'a Path,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<u64>> {
        Box::pin(async move {
            let bytes = mp3_bytes();
            std::fs::write(dest, &bytes)?;
            Ok(bytes.len() as u64)
        })
    }
}

struct Ctx {
    dir: tempfile::TempDir,
    service: InstallService,
    events: Arc<Events>,
    /// Dossier des fichiers source (hors du dossier de travail).
    sources: PathBuf,
    workdir: PathBuf,
    id: String,
}

impl Ctx {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let sources = dir.path().join("histoire");
        let workdir = dir.path().join("travail");
        std::fs::create_dir_all(&sources).unwrap();
        std::fs::create_dir_all(&workdir).unwrap();
        let events = Arc::new(Events::default());
        let tc = super::super::Toolchain {
            cache_root: dir.path().join("cache"),
            rust_converter: None,
            ffmpeg: None,
            max_downloads: 2,
            thumb_fit: crate::settings::ThumbFit::default(),
        };
        let service = InstallService::new(InstallOptions {
            client: super::super::download::build_client().unwrap(),
            resolver: Arc::new(move || Ok(tc.clone())),
            events: events.clone(),
            queue_path: dir.path().join("config").join(QUEUE_FILE),
            gate: Arc::default(),
            grace: Duration::from_secs(5),
        });
        let ctx = Self {
            dir,
            service,
            events,
            sources,
            workdir,
            id: uuid::Uuid::new_v4().to_string(),
        };
        ctx.register();
        ctx
    }

    /// Tâche connue du service : l'état du pack y est rangé.
    fn register(&self) {
        let entry = pack_entry(
            "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
            &metadata(),
            Path::new("pack.zip"),
        );
        lock(&self.service.0.tasks).push(Slot {
            task: InstallTask {
                id: self.id.clone(),
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
            replace_format: false,
            started: true,
        });
    }

    fn status(&self) -> PackStatus {
        lock(&self.service.0.tasks)
            .iter()
            .find(|s| s.task.id == self.id)
            .and_then(|s| s.task.pack.clone())
            .unwrap_or_default()
    }

    fn phases(&self) -> Vec<(String, f64)> {
        lock(&self.events.phases).clone()
    }

    /// Fichier source `name` (contenu `text`).
    fn source(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.sources.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// Élément dont l'audio est le fichier local `name` (contenu « ID3 <titre> ») et l'image une
    /// pochette valide.
    fn item(&self, name: &str, title: &str) -> LocalItem {
        LocalItem {
            title: title.into(),
            audio: ItemAudio::Source(self.source(name, format!("ID3 {title}").as_bytes())),
            image: Some(self.source(&format!("{name}.png"), &png())),
            title_audio: None,
            cache_key: None,
        }
    }

    async fn build(
        &self,
        backend: Arc<Local>,
        spec: &PackSpec,
        cover: Option<PathBuf>,
        titles: Option<&TitlesAudio>,
        cancelled: bool,
    ) -> AppResult<PathBuf> {
        let (_tx, cancel) = watch::channel(cancelled);
        PackBuild {
            service: &self.service,
            id: &self.id,
            workdir: &self.workdir,
            backend: &backend,
            cancel: &cancel,
        }
        .build(spec, cover, titles, None)
        .await
    }
}

fn metadata() -> ResolvedMetadata {
    ResolvedMetadata {
        title: "Mon histoire".into(),
        subtitle: String::new(),
        age: 0,
        description: None,
    }
}

fn spec(items: Vec<LocalItem>, menu: Vec<MenuNode>) -> PackSpec {
    PackSpec {
        uuid: "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d".into(),
        metadata: metadata(),
        items,
        menu,
    }
}

/// Élément 0, puis la catégorie « Chapitres » (éléments 1 et 2, image choisie : élément 2).
fn menu() -> Vec<MenuNode> {
    vec![
        MenuNode::Item(0),
        MenuNode::Category {
            name: "Chapitres".into(),
            entries: vec![MenuNode::Item(1), MenuNode::Item(2)],
            image: Some(2),
        },
    ]
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

/// Contenu des `audios/*.mp3` des histoires (hors titres lus), dans l'ordre des noms.
fn story_audios(files: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
    files
        .iter()
        .filter(|(name, bytes)| name.starts_with("audios/") && bytes.starts_with(b"ID3"))
        .map(|(_, bytes)| String::from_utf8_lossy(bytes).into_owned())
        .collect()
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

#[tokio::test]
async fn des_fichiers_locaux_donnent_une_archive_sans_toucher_aux_sources() {
    let ctx = Ctx::new();
    let items = vec![
        ctx.item("a.ogg", "Un"),
        ctx.item("b.m4a", "Deux"),
        ctx.item("c.mp3", "Trois"),
    ];
    let cover = ctx.source("couverture.jpg", &png());
    let backend = Arc::new(Local::default());
    let archive = ctx
        .build(
            backend.clone(),
            &spec(items.clone(), menu()),
            Some(cover.clone()),
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(archive, ctx.workdir.join(ARCHIVE_NAME));
    let files = read_zip(&archive);
    assert_eq!(story_audios(&files), ["ID3 Un", "ID3 Deux", "ID3 Trois"]);
    let notes = String::from_utf8_lossy(&files["notes.json"]).into_owned();
    for title in ["Un", "Deux", "Trois", "Chapitres"] {
        assert!(notes.contains(title), "{title} absent : {notes}");
    }
    // Sans titres lus : options muettes.
    assert!(option_audios(&files).values().all(Value::is_null));
    // Conversions dans l'ordre des rangs, depuis les fichiers source.
    let sources: Vec<PathBuf> = items
        .iter()
        .map(|i| match &i.audio {
            ItemAudio::Source(p) => p.clone(),
            _ => unreachable!(),
        })
        .collect();
    assert_eq!(*lock(&backend.converted), sources);
    // Sources intactes ; fichiers produits nommés par rang.
    for item in &items {
        if let ItemAudio::Source(p) = &item.audio {
            assert!(std::fs::read(p).unwrap().starts_with(b"ID3"));
        }
        assert_eq!(std::fs::read(item.image.as_ref().unwrap()).unwrap(), png());
    }
    assert_eq!(std::fs::read(&cover).unwrap(), png());
    let mut produced: Vec<String> = std::fs::read_dir(&ctx.workdir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    produced.sort();
    assert_eq!(
        produced,
        [
            "cover.png",
            "ep-0000.mp3",
            "ep-0000.png",
            "ep-0001.mp3",
            "ep-0001.png",
            "ep-0002.mp3",
            "ep-0002.png",
            "pack.zip"
        ]
    );
    // Avancement : mêmes phases et pourcentages que l'étape des podcasts.
    assert_eq!(
        ctx.phases(),
        [
            ("audio".to_string(), 60.0),
            ("images".to_string(), 85.0),
            ("archive".to_string(), 90.0)
        ]
    );
    let status = ctx.status();
    assert_eq!((status.episodes, status.dropped.len()), (3, 0));
    assert!(status.warnings.is_empty(), "{:?}", status.warnings);
    assert!(ctx.dir.path().join("histoire").exists());
}

#[tokio::test]
async fn elements_en_echec_ou_absents_retires_et_cites() {
    let ctx = Ctx::new();
    let mut items = vec![
        ctx.item("a.ogg", "Un"),
        ctx.item("b.m4a", "Deux"),
        ctx.item("c.mp3", "Trois"),
    ];
    // « Trois » : audio indisponible ; « Deux » : conversion en échec.
    items[2].audio = ItemAudio::Missing;
    let backend = Arc::new(Local {
        failing: vec!["b.m4a".into()],
        ..Local::default()
    });
    let archive = ctx
        .build(backend, &spec(items, menu()), None, None, false)
        .await
        .unwrap();
    let files = read_zip(&archive);
    assert_eq!(story_audios(&files), ["ID3 Un"]);
    // Catégorie vidée : retirée, sans avertissement d'image.
    let notes = String::from_utf8_lossy(&files["notes.json"]).into_owned();
    assert!(!notes.contains("Chapitres"), "{notes}");
    let status = ctx.status();
    assert_eq!(status.dropped, ["Deux", "Trois"]);
    assert!(status.warnings.is_empty(), "{:?}", status.warnings);
}

#[tokio::test]
async fn image_choisie_illisible_repli_et_avertissement() {
    let ctx = Ctx::new();
    let mut items = vec![
        ctx.item("a.ogg", "Un"),
        ctx.item("b.m4a", "Deux"),
        ctx.item("c.mp3", "Trois"),
    ];
    // Image de l'élément choisi pour la catégorie illisible : image de repli, l'élément reste.
    items[2].image = Some(ctx.source("c.jpg", b"pas une image"));
    let archive = ctx
        .build(
            Arc::new(Local::default()),
            &spec(items, menu()),
            None,
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(story_audios(&read_zip(&archive)).len(), 3);
    let status = ctx.status();
    assert!(status.dropped.is_empty());
    assert_eq!(status.warnings.len(), 1, "{:?}", status.warnings);
    assert!(status.warnings[0].contains("Chapitres"));
}

#[tokio::test]
async fn titres_lus_synthetises_ou_fournis() {
    let ctx = Ctx::new();
    let mut items = vec![
        ctx.item("a.ogg", "Un"),
        ctx.item("b.m4a", "Deux"),
        ctx.item("c.mp3", "Trois"),
    ];
    // Titre lu fourni par la source (MP3 conforme).
    items[0].title_audio = Some(ctx.source("titre-un.mp3", &mp3_bytes()));
    let spec = spec(items, menu());

    // Sans synthèse : seul le titre fourni est lu, pas de `title.mp3`.
    let archive = ctx
        .build(Arc::new(Local::default()), &spec, None, None, false)
        .await
        .unwrap();
    let files = read_zip(&archive);
    let audios = option_audios(&files);
    assert!(!audios["Un"].is_null(), "{audios:?}");
    assert!(audios["Deux"].is_null() && audios["Chapitres"].is_null());
    assert!(!files.contains_key("title.mp3"));

    // Avec synthèse : chaque option et le pack ont leur titre lu, de 0 à 10 %.
    let ctx = Ctx::new();
    let archive = ctx
        .build(
            Arc::new(Local::default()),
            &spec,
            None,
            Some(&TitlesAudio::default()),
            false,
        )
        .await
        .unwrap();
    let files = read_zip(&archive);
    assert!(files.contains_key("title.mp3"));
    assert!(option_audios(&files).values().all(|a| !a.is_null()));
    let phases = ctx.phases();
    let speech: Vec<f64> = phases
        .iter()
        .filter(|(p, _)| p == "speech")
        .map(|(_, v)| *v)
        .collect();
    // Titre du pack, Un, Chapitres, Deux, Trois.
    assert_eq!(speech, [0.0, 2.0, 4.0, 6.0, 8.0, 10.0]);
    assert!(phases.windows(2).all(|w| w[0].1 <= w[1].1), "{phases:?}");
}

#[tokio::test]
async fn aucun_element_utilisable_echec_qui_cite_les_titres() {
    let ctx = Ctx::new();
    let mut items = vec![ctx.item("a.ogg", "Un"), ctx.item("b.m4a", "Deux")];
    items[0].audio = ItemAudio::Missing;
    let backend = Arc::new(Local {
        failing: vec!["b.m4a".into()],
        ..Local::default()
    });
    let err = ctx
        .build(
            backend,
            &spec(items, vec![MenuNode::Item(0), MenuNode::Item(1)]),
            None,
            None,
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "download_invalid");
    let message = err.to_string();
    assert!(message.contains("Un, Deux"), "{message}");
    assert!(!ctx.workdir.join(ARCHIVE_NAME).exists());
}

#[tokio::test]
async fn convertisseur_indisponible_ou_annulation_arretent_le_pack() {
    let ctx = Ctx::new();
    let items = vec![ctx.item("a.ogg", "Un")];
    let backend = Arc::new(Local {
        fault: Fault::ConverterGone,
        ..Local::default()
    });
    let err = ctx
        .build(
            backend,
            &spec(items.clone(), vec![MenuNode::Item(0)]),
            None,
            None,
            false,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "converter_unavailable");
    assert!(err.to_string().contains("ffprobe.exe absent"), "{err}");

    let backend = Arc::new(Local::default());
    let err = ctx
        .build(
            backend.clone(),
            &spec(items, vec![MenuNode::Item(0)]),
            None,
            None,
            true,
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "cancelled");
    assert!(
        lock(&backend.converted).is_empty(),
        "aucune conversion lancée"
    );
    assert!(!ctx.workdir.join(ARCHIVE_NAME).exists());
}

#[tokio::test]
async fn mp3_deja_conforme_repris_sans_conversion_et_trop_gros_retire() {
    let ctx = Ctx::new();
    let ready = ctx.source("deja.mp3", b"ID3 Deja");
    let big = ctx.source("gros.mp3", b"ID3 Gros");
    let mut items = vec![
        ctx.item("a.ogg", "Un"),
        ctx.item("b.m4a", "Deux"),
        ctx.item("c.mp3", "Trois"),
    ];
    items[1].audio = ItemAudio::Ready {
        mp3: ready.clone(),
        bytes: 8,
    };
    items[2].audio = ItemAudio::Ready {
        mp3: big,
        bytes: MAX_AUDIO_BYTES + 1,
    };
    let backend = Arc::new(Local::default());
    let archive = ctx
        .build(
            backend.clone(),
            &spec(items.clone(), menu()),
            None,
            None,
            false,
        )
        .await
        .unwrap();
    assert_eq!(story_audios(&read_zip(&archive)), ["ID3 Un", "ID3 Deja"]);
    // Seule la source à convertir passe par le convertisseur.
    assert_eq!(
        *lock(&backend.converted),
        [match &items[0].audio {
            ItemAudio::Source(p) => p.clone(),
            _ => unreachable!(),
        }]
    );
    assert_eq!(std::fs::read(&ready).unwrap(), b"ID3 Deja");
    let status = ctx.status();
    assert_eq!(status.dropped, ["Trois"]);
    // Image choisie (élément 2) retirée avec lui : repli et avertissement.
    assert_eq!(status.warnings.len(), 1, "{:?}", status.warnings);
}

#[test]
fn contains_item_cherche_a_toute_profondeur() {
    let tree = menu();
    assert!(MenuNode::contains_item(&tree, 0));
    assert!(MenuNode::contains_item(&tree, 2));
    assert!(!MenuNode::contains_item(&tree, 3));
}
