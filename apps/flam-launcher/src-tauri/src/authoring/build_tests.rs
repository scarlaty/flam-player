//! Dossier lu → commande de fabrication (modes, images, titres lus, bornes), puis fabrication
//! complète de l'archive avec des accès simulés (ni ffmpeg, ni voix réelle, ni réseau).

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;
use tokio::sync::watch;

use super::*;
use crate::authoring::tree::{scan, Limits as ScanLimits};
use crate::ffmpeg::Verified;
use crate::install::podcast::BoxFuture;
use crate::install::{
    InstallDone, InstallEvents, InstallFailed, InstallOptions, InstallProgress, InstallService,
};
use crate::podcast::artwork::{Artwork, ArtworkError, ArtworkKind};
use crate::podcast::speech::testing::Simulated;
use crate::podcast::speech::{Synthesizer, Wav};

fn png() -> Vec<u8> {
    Artwork::fallback(ArtworkKind::Episode, [1, 2, 3])
        .unwrap()
        .png
}

/// Crée `rel` (et ses dossiers) sous `root` avec `bytes`.
fn put(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

/// Fichier audio factice « ID3 <nom> » (accepté par l'archive après la copie simulée).
fn audio(root: &Path, rel: &str) {
    put(root, rel, format!("ID3 {rel}").as_bytes());
}

fn scanned(root: &Path) -> Scan {
    scan(root, &scan_limits(), &[]).unwrap()
}

fn mode_of(node: &MenuNode) -> Option<MenuMode> {
    match node {
        MenuNode::Category { mode, .. } => *mode,
        MenuNode::Item(_) => None,
    }
}

/// Recueil : racine (`a`, `b`), sous-dossier « Animaux » avec image et titre lu, une histoire
/// avec image et titre lu, couverture et titre lu de la racine.
fn collection(root: &Path) {
    put(root, "_histoire.json", br#"{"mode": "recueil"}"#);
    audio(root, "01 Le loup.mp3");
    put(root, "01 Le loup.png", &png());
    audio(root, "01 Le loup.titre.mp3");
    audio(root, "02 Le chat.ogg");
    audio(root, "03 Animaux/01 Ours.mp3");
    put(root, "03 Animaux/_image.jpg", &png());
    audio(root, "03 Animaux/_titre.wav");
    put(root, "_couverture.png", &png());
    audio(root, "_titre.mp3");
}

#[test]
fn un_recueil_devient_une_commande_de_fabrication() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Mes histoires");
    collection(&root);
    let scan = scanned(&root);
    assert!(!scan.is_blocked(), "{:?}", scan.problems);
    let plan = plan(&scan, "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    let spec = &plan.spec;
    let titles: Vec<&str> = spec.items.iter().map(|i| i.title.as_str()).collect();
    assert_eq!(titles, ["Le loup", "Le chat", "Ours"]);
    assert_eq!(spec.metadata.title, "Mes histoires");
    assert_eq!(
        spec.options.mode,
        MenuMode::Collection {
            end: CollectionEnd::Menu
        }
    );
    assert_eq!(spec.options.limits, archive_limits());
    assert_eq!(spec.menu.len(), 3);
    assert_eq!(spec.menu[0], MenuNode::Item(0));
    assert_eq!(spec.menu[1], MenuNode::Item(1));
    let MenuNode::Category {
        name,
        entries,
        picture,
        image,
        ..
    } = &spec.menu[2]
    else {
        panic!("catégorie attendue");
    };
    assert_eq!(name, "Animaux");
    assert_eq!(entries, &[MenuNode::Item(2)]);
    assert_eq!(image, &None);
    assert_eq!(
        picture.as_deref(),
        Some(scan.base.join("03 Animaux").join("_image.jpg").as_path())
    );
    assert_eq!(
        mode_of(&spec.menu[2]),
        Some(MenuMode::Collection {
            end: CollectionEnd::Menu
        })
    );
    // Images : celle du même nom ; sans image dans un recueil, aucune (image unie).
    assert_eq!(
        spec.items[0].image.as_deref(),
        Some(scan.base.join("01 Le loup.png").as_path())
    );
    assert_eq!(spec.items[1].image, None);
    assert_eq!(spec.items[2].image, None);
    assert_eq!(
        spec.items[0].audio,
        ItemAudio::Source(scan.base.join("01 Le loup.mp3"))
    );
    assert_eq!(
        plan.cover.as_deref(),
        Some(scan.base.join("_couverture.png").as_path())
    );
    // Titres lus fournis : racine, histoire 0, catégorie (position dans l'arbre).
    let keys: Vec<&TitleKey> = plan.titles.iter().map(|(k, _)| k).collect();
    assert_eq!(
        keys,
        [
            &TitleKey::Pack,
            &TitleKey::Node(vec![0]),
            &TitleKey::Node(vec![2]),
        ]
    );
    assert!(spec.items.iter().all(|i| i.title_audio.is_none()));
    assert!(book_problems(&scan).is_empty());
}

#[test]
fn sans_couverture_la_premiere_image_d_histoire_sert() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Recueil");
    audio(&root, "a.mp3");
    audio(&root, "b.mp3");
    put(&root, "b.png", &png());
    let scan = scanned(&root);
    let plan = plan(&scan, "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    assert_eq!(
        plan.cover.as_deref(),
        Some(scan.base.join("b.png").as_path())
    );
}

#[test]
fn un_livre_a_chapitres_garde_ses_reglages_et_ses_images_de_repli() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Livre");
    put(
        &root,
        "_histoire.json",
        br#"{"mode": "chapitres", "debut": "menu", "fin": "fin", "annoncer": true,
            "sousTitre": "Livre audio", "age": 6, "titresLus": {"voix": "Voix-1"}}"#,
    );
    put(&root, "_couverture.png", &png());
    audio(&root, "Partie 1/01 Début.mp3");
    put(&root, "Partie 1/_image.png", &png());
    audio(&root, "Partie 1/02 Suite.mp3");
    put(&root, "Partie 1/02 Suite.png", &png());
    audio(&root, "Partie 2/03 Fin.mp3");
    let scan = scanned(&root);
    assert!(!scan.is_blocked(), "{:?}", scan.problems);
    let plan = plan(&scan, "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    let spec = &plan.spec;
    let settings = ChapterSettings {
        start: ChapterStart::Menu,
        end: BookEnd::End,
        announce: true,
    };
    assert_eq!(spec.options.mode, MenuMode::Chapters(settings));
    assert_eq!(mode_of(&spec.menu[0]), Some(MenuMode::Chapters(settings)));
    assert_eq!(spec.metadata.subtitle, "Livre audio");
    assert_eq!(spec.metadata.age, 6);
    assert_eq!(requested_voice(&scan).as_deref(), Some("Voix-1"));
    let images: Vec<Option<PathBuf>> = spec.items.iter().map(|i| i.image.clone()).collect();
    assert_eq!(
        images,
        [
            // Chapitre sans image : celle de sa partie.
            Some(scan.base.join("Partie 1").join("_image.png")),
            Some(scan.base.join("Partie 1").join("02 Suite.png")),
            // Partie sans image : la couverture.
            Some(scan.base.join("_couverture.png")),
        ]
    );
}

#[test]
fn modes_fin_et_debut_traduits_pour_l_archive() {
    assert_eq!(
        menu_mode(Mode::Recueil, Fin::Suivante, false, Debut::Chapitre),
        MenuMode::PODCAST
    );
    assert_eq!(
        menu_mode(Mode::Recueil, Fin::Menu, true, Debut::Menu),
        MenuMode::Collection {
            end: CollectionEnd::Menu
        }
    );
    assert_eq!(
        menu_mode(Mode::Chapitres, Fin::Menu, false, Debut::Chapitre),
        MenuMode::Chapters(ChapterSettings::default())
    );
}

#[test]
fn un_recueil_dans_un_livre_est_bloquant() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Livre");
    put(&root, "_histoire.json", br#"{"mode": "chapitres"}"#);
    audio(&root, "01 Un.mp3");
    audio(&root, "Bonus/a.mp3");
    put(&root, "Bonus/_dossier.json", br#"{"mode": "recueil"}"#);
    let scan = scanned(&root);
    let problems = book_problems(&scan);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert_eq!(problems[0].code, "collection_in_book");
    assert_eq!(problems[0].severity, Severity::Blocking);
    assert_eq!(problems[0].path, "Bonus");
}

#[test]
fn bornes_du_dossier() {
    let limits = scan_limits();
    assert_eq!(limits.max_menu_depth, 5);
    assert_eq!(limits.max_stories, 300);
    assert_eq!(limits.max_options_per_menu, 50);
    assert_eq!(limits.max_audio_bytes, 3 * 1024 * 1024 * 1024);
    let archive = archive_limits();
    assert_eq!(archive.max_menu_depth, limits.max_menu_depth);
    assert_eq!(archive.max_episodes, limits.max_stories);
    assert_eq!(archive.max_options_per_menu, limits.max_options_per_menu);
    assert_eq!(archive.max_total_audio_bytes, limits.max_audio_bytes);
    // zip32 : nettement sous 4 Gio.
    const { assert!(MAX_AUDIO_BYTES < 4 * 1024 * 1024 * 1024 - 512 * 1024 * 1024) };
    assert_eq!(MAX_STAGES, 1 + 300 + 300 + 300 + 300 * 4);
}

#[test]
fn trop_d_histoires_d_options_ou_de_niveaux_est_bloquant() {
    let dir = tempfile::tempdir().unwrap();
    // 301 histoires réparties en 7 dossiers de 43 (moins de 50 options chacun).
    let many = dir.path().join("Beaucoup");
    for n in 0..301 {
        put(&many, &format!("d{}/h{n:03}.mp3", n / 43), b"ID3");
    }
    let scan = scanned(&many);
    assert!(scan
        .problems
        .iter()
        .any(|p| p.code == "too_many_stories" && p.severity == Severity::Blocking));
    // 51 options dans un menu.
    let wide = dir.path().join("Large");
    for n in 0..51 {
        put(&wide, &format!("h{n:02}.mp3"), b"ID3");
    }
    let scan = scanned(&wide);
    assert!(scan
        .problems
        .iter()
        .any(|p| p.code == "too_many_options" && p.severity == Severity::Blocking));
    // Six niveaux de menus avec de l'audio au fond.
    let deep = dir.path().join("Profond");
    put(&deep, "1/2/3/4/5/a.mp3", b"ID3");
    let scan = scanned(&deep);
    assert!(scan
        .problems
        .iter()
        .any(|p| p.code == "too_deep" && p.severity == Severity::Blocking));
    // Cinq niveaux : accepté.
    let ok = dir.path().join("Cinq");
    put(&ok, "1/2/3/4/a.mp3", b"ID3");
    assert!(!scanned(&ok).is_blocked());
}

#[test]
fn audio_au_dela_de_la_borne_est_bloquant() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Gros");
    audio(&root, "a.mp3");
    audio(&root, "b.mp3");
    let limits = ScanLimits {
        max_audio_bytes: 10,
        ..scan_limits()
    };
    let scan = scan(&root, &limits, &[]).unwrap();
    assert!(scan
        .problems
        .iter()
        .any(|p| p.code == "audio_too_large" && p.severity == Severity::Blocking));
}

#[test]
fn entree_de_mes_histoires_retient_le_dossier() {
    let meta = ResolvedMetadata {
        title: "Le Petit Prince".into(),
        subtitle: "Livre audio".into(),
        age: 6,
        description: None,
    };
    let folder = Path::new(r"C:\Histoires\Prince");
    let entry = story_entry("5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d", 3, &meta, folder);
    assert_eq!(entry.source_id, SOURCE_ID);
    assert_eq!(
        entry.entry_id,
        "folder:5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d"
    );
    assert_eq!(entry.version, Some(3));
    assert_eq!(entry.download_url, folder.to_string_lossy());
    assert_eq!(entry.category.as_deref(), Some("Livre audio"));
    assert_eq!(entry.kind, StoryKind::TelmiZip);
}

#[test]
fn chemin_d_un_id() {
    let base = Path::new("/base");
    assert_eq!(path_of(base, ""), base);
    assert_eq!(path_of(base, "a/b c.mp3"), base.join("a").join("b c.mp3"));
}

// ---------------------------------------------------------------------------------------
// Fabrication complète (accès simulés)
// ---------------------------------------------------------------------------------------

#[derive(Default)]
struct Events {
    phases: Mutex<Vec<String>>,
}

impl InstallEvents for Events {
    fn progress(&self, p: &InstallProgress) {
        if let Some(phase) = &p.phase {
            self.phases.lock().unwrap().push(phase.clone());
        }
    }
    fn done(&self, _: &InstallDone) {}
    fn failed(&self, _: &InstallFailed) {}
}

/// Accès simulés : la conversion audio copie la source (sauf les noms de `failing`), la voix
/// est simulée (ou absente), les titres synthétisés sont des MP3 factices.
#[derive(Default)]
struct Fake {
    failing: Vec<String>,
    no_voice: bool,
    /// Textes envoyés à la voix.
    spoken: Mutex<Vec<String>>,
    /// Fichiers source convertis (nom), dans l'ordre.
    converted: Mutex<Vec<String>>,
}

impl BuildBackend for Fake {
    fn convert_audio<'a>(
        &'a self,
        items: Vec<(PathBuf, PathBuf)>,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Vec<AppResult<u64>>>> {
        Box::pin(async move {
            Ok(items
                .iter()
                .map(|(from, to)| {
                    let name = from.file_name().unwrap().to_string_lossy().into_owned();
                    self.converted.lock().unwrap().push(name.clone());
                    if self.failing.contains(&name) {
                        return Err(AppError::Conversion {
                            code: "ffmpeg_failed".into(),
                            message: "échec simulé".into(),
                        });
                    }
                    fs::copy(from, to).map_err(AppError::from)
                })
                .collect())
        })
    }

    fn convert_artwork(&self, bytes: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
        Artwork::convert(bytes, kind)
    }

    fn image_decoder(&self) -> crate::authoring::images::Convert {
        |bytes, kind| Artwork::convert(bytes, kind)
    }

    fn cover_extractor<'a>(
        &'a self,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<Verified>> {
        Box::pin(async {
            Err(AppError::ConverterUnavailable(
                "ffmpeg absent (test)".into(),
            ))
        })
    }

    fn speech(&self) -> Arc<dyn Synthesizer> {
        let mut voice = Simulated::french();
        if self.no_voice {
            voice.voices.clear();
            voice.default_id = None;
        }
        voice.shared()
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
            self.spoken
                .lock()
                .unwrap()
                .push(dest.file_name().unwrap().to_string_lossy().into_owned());
            let bytes = b"ID3 synthese".to_vec();
            fs::write(dest, &bytes)?;
            Ok(bytes.len() as u64)
        })
    }
}

fn service(dir: &Path) -> InstallService {
    let tc = crate::install::Toolchain {
        cache_root: dir.join("cache"),
        rust_converter: None,
        ffmpeg: None,
        max_downloads: 2,
        thumb_fit: crate::settings::ThumbFit::default(),
    };
    InstallService::new(InstallOptions {
        client: crate::install::download::build_client().unwrap(),
        resolver: Arc::new(move || Ok(tc.clone())),
        events: Arc::new(Events::default()),
        queue_path: dir.join("config").join(crate::install::QUEUE_FILE),
        gate: Arc::default(),
        grace: Duration::from_secs(5),
    })
}

fn read_zip(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        out.insert(f.name().to_string(), bytes);
    }
    out
}

/// Fabrique l'archive du dossier `root` avec `backend` ; rend ses fichiers.
async fn fabricate(
    root: &Path,
    backend: Arc<Fake>,
) -> AppResult<(BTreeMap<String, Vec<u8>>, PathBuf)> {
    fabricate_with(root, backend, None).await
}

/// [`fabricate`] avec le cache des MP3 `cache`.
async fn fabricate_with(
    root: &Path,
    backend: Arc<Fake>,
    cache: Option<&CacheAccess>,
) -> AppResult<(BTreeMap<String, Vec<u8>>, PathBuf)> {
    let work = tempfile::tempdir().unwrap();
    let service = service(work.path());
    let workdir = work.path().join("travail");
    fs::create_dir_all(&workdir).unwrap();
    let scan = scanned(root);
    let plan = plan(&scan, "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    let (_tx, cancel) = watch::channel(false);
    let stage = PackBuild {
        service: &service,
        id: "c0a80101-0000-4000-8000-000000000001",
        workdir: &workdir,
        backend: &backend,
        cancel: &cancel,
        source: BuildSource::Folder,
    };
    let archive = prepare(&stage, plan, None, cache).await?;
    // Rien de ffmpeg ni du cache ne reste à côté de l'archive, hors fichiers produits.
    for entry in fs::read_dir(&workdir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(
            !name.starts_with("cover-"),
            "sortie de ffmpeg restée : {name}"
        );
    }
    let files = read_zip(&archive);
    // Le dossier de travail temporaire disparaît avec `work` : on garde son chemin pour info.
    Ok((files, archive))
}

/// Fichiers du dossier source, contenus compris (pour vérifier qu'ils ne changent pas).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                out.insert(path.clone(), fs::read(&path).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

#[tokio::test]
async fn un_recueil_devient_une_archive_sans_toucher_au_dossier() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Mes histoires");
    collection(&root);
    let before = snapshot(&root);
    let backend = Arc::new(Fake::default());
    let (files, _) = fabricate(&root, backend.clone()).await.unwrap();
    assert_eq!(snapshot(&root), before, "dossier source modifié");
    // Image propre du sous-menu, titres lus fournis (convertis) et synthétisés.
    assert!(files.contains_key("images/m001.png"), "{:?}", files.keys());
    assert_eq!(files["title.mp3"], b"ID3 _titre.mp3");
    let nodes: Value = serde_json::from_slice(&files["nodes.json"]).unwrap();
    let options: BTreeMap<String, Value> = nodes["stages"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(id, _)| id.starts_with('o'))
        .map(|(id, st)| (id.clone(), st.clone()))
        .collect();
    assert_eq!(options.len(), 4, "{options:?}");
    // Le titre de « Le loup » est le fichier fourni ; « Le chat » et « Ours » sont synthétisés ;
    // « Animaux » a son `_titre.wav`.
    let audios: Vec<&[u8]> = options
        .values()
        .filter_map(|st| st["audio"].as_str())
        .map(|name| files[&format!("audios/{name}")].as_slice())
        .collect();
    assert!(audios.contains(&b"ID3 01 Le loup.titre.mp3".as_slice()));
    assert!(audios.contains(&b"ID3 03 Animaux/_titre.wav".as_slice()));
    assert_eq!(
        audios
            .iter()
            .filter(|a| **a == b"ID3 synthese".as_slice())
            .count(),
        2
    );
    // Seuls les titres sans fichier sont proposés à la voix : « Le chat » et « Ours ».
    assert_eq!(backend.spoken.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn sans_voix_les_titres_restent_muets_et_l_histoire_est_creee() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Muet");
    audio(&root, "a.mp3");
    audio(&root, "b.mp3");
    let backend = Arc::new(Fake {
        no_voice: true,
        ..Fake::default()
    });
    let (files, _) = fabricate(&root, backend).await.unwrap();
    assert!(!files.contains_key("title.mp3"));
    let nodes: Value = serde_json::from_slice(&files["nodes.json"]).unwrap();
    for (id, st) in nodes["stages"].as_object().unwrap() {
        if id.starts_with('o') {
            assert!(st["audio"].is_null(), "{id} : {st}");
        }
    }
}

#[tokio::test]
async fn un_titre_fourni_illisible_est_remplace_par_la_voix() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Titres");
    audio(&root, "a.mp3");
    audio(&root, "a.titre.mp3");
    let backend = Arc::new(Fake {
        failing: vec!["a.titre.mp3".into()],
        ..Fake::default()
    });
    let (files, _) = fabricate(&root, backend.clone()).await.unwrap();
    // Titre de l'histoire et de l'option synthétisés.
    assert_eq!(backend.spoken.lock().unwrap().len(), 2);
    assert_eq!(files["title.mp3"], b"ID3 synthese");
}

#[tokio::test]
async fn un_livre_devient_une_archive_a_chapitres() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Livre");
    put(
        &root,
        "_histoire.json",
        br#"{"mode": "chapitres", "fin": "fin"}"#,
    );
    audio(&root, "01 Un.mp3");
    audio(&root, "02 Deux.mp3");
    audio(&root, "03 Trois.mp3");
    let (files, _) = fabricate(&root, Arc::new(Fake::default())).await.unwrap();
    let nodes: Value = serde_json::from_slice(&files["nodes.json"]).unwrap();
    // Démarrage direct sur le premier chapitre ; le dernier termine l'histoire.
    let start = &nodes["stages"]["start"]["ok"]["action"];
    assert_ne!(start, "m0");
    assert!(nodes["stages"]["e3"]["ok"].is_null());
    assert!(!nodes["stages"]["e1"]["ok"].is_null());
}

#[tokio::test]
async fn toutes_les_conversions_en_echec_donnent_un_message_d_histoire() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Echec");
    audio(&root, "a.mp3");
    let backend = Arc::new(Fake {
        failing: vec!["a.mp3".into()],
        ..Fake::default()
    });
    let err = fabricate(&root, backend).await.unwrap_err();
    assert!(
        err.to_string().contains("aucune histoire utilisable"),
        "{err}"
    );
}

// ---------------------------------------------------------------------------------------
// Compléments d'image (§ 7) et cache des MP3 (§ 8)
// ---------------------------------------------------------------------------------------

fn solid(kind: ArtworkKind) -> Vec<u8> {
    Artwork::fallback(kind, crate::podcast::artwork::FALLBACK_COLOR)
        .unwrap()
        .png
}

#[tokio::test]
async fn sans_image_les_histoires_et_la_couverture_recoivent_leur_titre() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Sans images");
    audio(&root, "01 Le loup.mp3");
    audio(&root, "02 Le chat.mp3");
    // Image fournie illisible : complément aussi, pas l'image unie.
    put(&root, "02 Le chat.png", b"pas une image");
    audio(&root, "03 Animaux/01 Ours.mp3");
    let before = snapshot(&root);
    let (files, _) = fabricate(&root, Arc::new(Fake::default())).await.unwrap();
    assert_eq!(snapshot(&root), before, "dossier source modifié");
    let unie = solid(ArtworkKind::Episode);
    let episodes: Vec<&Vec<u8>> = ["images/e001.png", "images/e002.png", "images/e003.png"]
        .iter()
        .map(|name| {
            files
                .get(*name)
                .unwrap_or_else(|| panic!("{name} : {:?}", files.keys()))
        })
        .collect();
    for png in &episodes {
        assert_ne!(**png, unie, "image unie au lieu de l'image du titre");
    }
    // Une image par titre : toutes différentes.
    assert_ne!(episodes[0], episodes[1]);
    assert_ne!(episodes[1], episodes[2]);
    // Couverture : image générée avec le titre de l'histoire.
    assert_ne!(files["cover.png"], solid(ArtworkKind::Cover));
}

#[tokio::test]
async fn une_image_fournie_reste_prioritaire_et_sert_de_couverture() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Recueil");
    audio(&root, "a.mp3");
    audio(&root, "b.mp3");
    put(&root, "b.png", &png());
    let (files, _) = fabricate(&root, Arc::new(Fake::default())).await.unwrap();
    let fournie = Artwork::convert(&png(), ArtworkKind::Episode).unwrap().png;
    assert_eq!(files["images/e002.png"], fournie);
    assert_ne!(files["images/e001.png"], fournie);
    assert_ne!(files["images/e001.png"], solid(ArtworkKind::Episode));
    let couverture = Artwork::convert(&png(), ArtworkKind::Cover).unwrap().png;
    assert_eq!(files["cover.png"], couverture);
}

#[tokio::test]
async fn annulation_pendant_les_complements() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Annule");
    audio(&root, "a.mp3");
    let work = tempfile::tempdir().unwrap();
    let service = service(work.path());
    let workdir = work.path().join("travail");
    fs::create_dir_all(&workdir).unwrap();
    let backend = Arc::new(Fake::default());
    let (tx, cancel) = watch::channel(false);
    let stage = PackBuild {
        service: &service,
        id: "c0a80101-0000-4000-8000-000000000001",
        workdir: &workdir,
        backend: &backend,
        cancel: &cancel,
        source: BuildSource::Folder,
    };
    let converted = Converted {
        kept: vec![crate::install::pack_build::Kept {
            rank: 0,
            mp3: workdir.join("ep-0000.mp3"),
            image: None,
        }],
        dropped_titles: Vec::new(),
    };
    let scan = scanned(&root);
    let plan = plan(&scan, "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    tx.send(true).unwrap();
    let mut images = Images::default();
    let err = complete_images(
        &stage,
        &plan.spec.items,
        &[Some(root.join("a.mp3"))],
        "Annule",
        &converted,
        &mut images,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Cancelled), "{err:?}");
    assert!(images.items.is_empty() && images.cover.is_none());
}

#[test]
fn cle_du_cache_suit_chemin_taille_et_date() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.mp3");
    fs::write(&a, b"ID3 un").unwrap();
    let source = cache_source_id("5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    assert_eq!(source, "authoring-5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d");
    let k1 = cache_key(&source, &a).unwrap();
    assert_eq!(cache_key(&source, &a).unwrap(), k1, "clé stable");
    assert_eq!(k1.image_fingerprint, None);
    // Taille changée : autre empreinte, même entrée.
    fs::write(&a, b"ID3 deux !").unwrap();
    let k2 = cache_key(&source, &a).unwrap();
    assert_eq!(k2.episode_id, k1.episode_id);
    assert_ne!(k2.fingerprint, k1.fingerprint);
    // Même taille, date changée : autre empreinte.
    let file = fs::File::options().write(true).open(&a).unwrap();
    file.set_modified(std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000))
        .unwrap();
    drop(file);
    let k3 = cache_key(&source, &a).unwrap();
    assert_ne!(k3.fingerprint, k2.fingerprint);
    // Autre chemin : autre entrée ; dossier : aucune clé.
    let b = dir.path().join("b.mp3");
    fs::copy(&a, &b).unwrap();
    assert_ne!(cache_key(&source, &b).unwrap().episode_id, k3.episode_id);
    assert_eq!(cache_key(&source, dir.path()), None);
}

#[tokio::test]
async fn actualiser_ne_reconvertit_que_les_fichiers_modifies() {
    use crate::podcast::mp3cache::tests::TestLocks;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("Histoire");
    audio(&root, "a.mp3");
    audio(&root, "b.ogg");
    let cache_root = dir.path().join("cache-du-launcher");
    crate::cache::ensure_subdir(&cache_root, crate::podcast::source::CACHE_SUBDIR).unwrap();
    let locks = Arc::new(TestLocks::default());
    let access = || {
        CacheAccess::new(
            locks.clone(),
            cache_root.clone(),
            [cache_source_id("5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d")],
        )
    };

    let first = Arc::new(Fake::default());
    fabricate_with(&root, first.clone(), Some(&access()))
        .await
        .unwrap();
    assert_eq!(*first.converted.lock().unwrap(), ["a.mp3", "b.ogg"]);

    // Rien n'a changé : aucune conversion, archive identique pour l'audio.
    let second = Arc::new(Fake::default());
    let (files, _) = fabricate_with(&root, second.clone(), Some(&access()))
        .await
        .unwrap();
    assert!(second.converted.lock().unwrap().is_empty());
    assert_eq!(files["audios/e001.mp3"], b"ID3 a.mp3");
    assert_eq!(files["audios/e002.mp3"], b"ID3 b.ogg");

    // Un fichier modifié : lui seul est reconverti, le nouveau contenu est dans l'archive.
    put(&root, "b.ogg", b"ID3 b modifie");
    let third = Arc::new(Fake::default());
    let (files, _) = fabricate_with(&root, third.clone(), Some(&access()))
        .await
        .unwrap();
    assert_eq!(*third.converted.lock().unwrap(), ["b.ogg"]);
    assert_eq!(files["audios/e001.mp3"], b"ID3 a.mp3");
    assert_eq!(files["audios/e002.mp3"], b"ID3 b modifie");
}
