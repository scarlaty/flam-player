//! Installation des histoires STUdio (#554, PR « G2 » de l'épopée #517) : zip STUdio
//! synthétique → scan du dossier → installation (`install::studio`) → `.lunii.pk` dans la
//! bibliothèque → `Library::playable` → préparation Lunii pour une Flam 2.x. Doublons
//! Telmi/STUdio (Q10), archive modifiée depuis le scan, annulation, reprise au démarrage et
//! balayage des dossiers temporaires de conversion (#553).

use std::io::{Cursor, Write};
use std::sync::atomic::AtomicBool;

use serde_json::json;
use zip::write::SimpleFileOptions;

use super::*;
use crate::device::crypto::tests::test_keys;
use crate::device::prepare::Limits as PrepareLimits;
use crate::device::prepare_lunii;
use crate::device::transfer::LibraryFormat;
use crate::folder::FolderState;
use crate::library::{LibraryItem, StoryFormat};

/// UUID du nœud d'entrée, écrit en majuscules dans `story.json` (l'identifiant retenu est en
/// minuscules, au scan comme dans le pack).
const NODE_UUID: &str = "0A1B2C3D-4E5F-4071-8293-A4B5C6D7E8F9";
const UUID: &str = "0a1b2c3d-4e5f-4071-8293-a4b5c6d7e8f9";
const OTHER_UUID: &str = "1b2c3d4e-5f60-4182-93a4-b5c6d7e8f90a";
const OTHER_TASK: &str = "11111111-2222-4333-8444-555555555555";

/// MP3 conforme des fixtures de `studio2lunii` (gardé tel quel : ffmpeg inutile).
fn mp3() -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../crates/studio2lunii/tests/data/audio/mono_44100_cbr.mp3");
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x * 7) as u8, (y * 5) as u8, 120, 255])
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// Zip STUdio inventé : un nœud d'entrée (image, son) et une action qui y revient. Le champ
/// `uuid` racine diffère volontairement de celui du nœud d'entrée (il est ignoré).
fn studio_zip(path: &Path, node_uuid: &str) {
    let controls =
        json!({"wheel": true, "ok": true, "home": true, "pause": false, "autoplay": false});
    let story = json!({
        "format": "v1",
        "version": 3,
        "title": "  Le conte\tinventé ",
        "uuid": OTHER_UUID,
        "description": "Une histoire inventée pour les tests.",
        "nightModeAvailable": false,
        "stageNodes": [
            {"uuid": node_uuid, "image": "scene.png", "audio": "voix.mp3", "squareOne": true,
             "okTransition": {"actionNode": "menu", "optionIndex": 0},
             "homeTransition": null, "controlSettings": controls}
        ],
        "actionNodes": [{"id": "menu", "options": [node_uuid]}]
    });
    let mut zip = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let options = SimpleFileOptions::default();
    for (name, data) in [
        ("story.json", serde_json::to_vec_pretty(&story).unwrap()),
        ("thumbnail.png", png(300, 300)),
        ("assets/scene.png", png(640, 480)),
        ("assets/voix.mp3", mp3()),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.finish().unwrap();
}

/// Dossier source `src/` avec `conte.zip`, et l'entrée STUdio du scan.
fn source(ctx_dir: &Path) -> (PathBuf, StoryEntry) {
    let src = ctx_dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    studio_zip(&src.join("conte.zip"), NODE_UUID);
    (src.clone(), scanned(&src))
}

fn scanned(src: &Path) -> StoryEntry {
    let scan = FolderState::default().scan("src", src, false).unwrap();
    assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
    let entry = scan.entries.into_iter().next().expect("histoire STUdio");
    assert_eq!(entry.kind, StoryKind::StudioZip);
    entry
}

/// Histoire Flam (Telmi convertie) de même `uuid` déjà dans la bibliothèque.
fn install_flam_twin(cache: &Path) {
    let library = Library::new(cache);
    let staged = library.staging_dir().unwrap().join("jumelle.plain.pk");
    std::fs::write(&staged, b"PK flam").unwrap();
    library
        .install(&staged, &story(UUID, Some(3)), Some("v".into()), 1)
        .unwrap();
}

#[tokio::test]
async fn installation_studio_de_bout_en_bout() {
    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    // Scan aligné sur le pack (#553) : uuid du nœud d'entrée, titre assaini, version.
    assert_eq!(entry.uuid, UUID);
    assert_eq!(entry.title, "Le conte inventé");
    assert_eq!(entry.version, Some(3));

    let t = ctx.service.start(entry.clone()).unwrap();
    let task = ctx.finished(&t.id).await;
    assert_eq!(task.state, TaskState::Done, "{:?}", task.error);
    let states = ctx.events.states(&t.id);
    assert!(states.contains(&TaskState::Converting), "{states:?}");
    let phases: Vec<String> = ctx
        .events
        .all()
        .into_iter()
        .filter_map(|r| match r {
            Recorded::Progress(p) if p.task_id == t.id => p.phase,
            _ => None,
        })
        .collect();
    assert!(phases.iter().any(|p| p == "zip"), "{phases:?}");

    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    let installed = &library[0];
    assert_eq!(installed.file, format!("{UUID}-3.lunii.pk"));
    assert_eq!(
        installed.title, entry.title,
        "titre du scan = titre du pack"
    );
    assert_eq!(installed.source.kind, StoryKind::StudioZip);
    assert_eq!(installed.converter, None);
    assert!(
        installed.source_stamp.is_some(),
        "empreinte de la source dossier"
    );
    let thumb = installed.thumb.as_deref().expect("vignette du pack");
    assert!(thumb.starts_with(&format!("{UUID}-3.lunii.")), "{thumb}");
    assert_eq!(
        LibraryItem::from(installed.clone()).format,
        StoryFormat::Lunii
    );
    assert!(
        ctx.staging_is_empty(),
        "ni pack préparé ni dossier temporaire"
    );

    // De la bibliothèque à l'envoi : fichier choisi par `playable`, préparation Lunii 2.x.
    let (path, title) = Library::new(&ctx.cache()).playable(UUID).unwrap();
    assert_eq!(title, entry.title);
    assert_eq!(LibraryFormat::of(&path), LibraryFormat::Lunii);
    let work = ctx.dir.path().join("travail");
    let prepared = prepare_lunii::prepare(
        &path,
        &test_keys(),
        &work,
        &PrepareLimits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(
        prepared.uuid, UUID,
        "même uuid au scan, en bibliothèque et sur la Flam"
    );
    for name in ["ni", "li", "ri", "si", "info", "key", "img/thumbnail.lif"] {
        assert!(
            prepared.files.iter().any(|f| f == name),
            "{name} : {:?}",
            prepared.files
        );
    }

    // Réinstallation au même format : mise à jour habituelle, sans confirmation.
    let t = ctx.service.start(entry).unwrap();
    assert_eq!(ctx.finished(&t.id).await.state, TaskState::Done);
    assert_eq!(ctx.service.library_list().await.unwrap().len(), 1);
}

/// Q10 : l'histoire Telmi convertie (format Flam) de même `uuid` n'est remplacée par le pack
/// STUdio qu'après confirmation ; sans elle, refus avant toute conversion.
#[tokio::test]
async fn doublon_telmi_studio_remplace_seulement_sur_confirmation() {
    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    install_flam_twin(&ctx.cache());

    let t = ctx.service.start(entry.clone()).unwrap();
    let task = ctx.finished(&t.id).await;
    assert_eq!(task.state, TaskState::Failed);
    let error = task.error.unwrap();
    assert_eq!(error.kind, "validation");
    assert!(error.message.contains("confirmez"), "{}", error.message);
    assert!(
        !ctx.events.states(&t.id).contains(&TaskState::Converting),
        "refus avant la conversion"
    );
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].file, format!("{UUID}-3.plain.pk"));
    assert!(ctx.staging_is_empty());

    let t = ctx.service.start_with(entry, true).unwrap();
    let task = ctx.finished(&t.id).await;
    assert_eq!(task.state, TaskState::Done, "{:?}", task.error);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].file, format!("{UUID}-3.lunii.pk"));
    let root = ctx.cache().join("library");
    assert!(!root.join(format!("{UUID}-3.plain.pk")).exists());
}

/// Archive remplacée par une autre histoire entre le scan et l'installation : le pack ne
/// porte plus l'`uuid` de l'entrée, rien n'est publié.
#[tokio::test]
async fn archive_modifiee_depuis_l_analyse_refusee() {
    let ctx = Ctx::new(None);
    let (src, entry) = source(ctx.dir.path());
    studio_zip(&src.join("conte.zip"), OTHER_UUID);
    let t = ctx.service.start(entry).unwrap();
    let task = ctx.finished(&t.id).await;
    assert_eq!(task.state, TaskState::Failed);
    let error = task.error.unwrap();
    assert_eq!(error.kind, "validation");
    assert!(error.message.contains("a changé"), "{}", error.message);
    assert!(ctx.service.library_list().await.unwrap().is_empty());
    assert!(ctx.staging_is_empty());
}

/// Annulation pendant l'attente du créneau de conversion (partagé avec Telmi).
#[tokio::test]
async fn annulation_d_une_installation_studio() {
    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    let slot = ctx.service.0.conversions.acquire().await.unwrap();
    let t = ctx.service.start(entry).unwrap();
    // Annulation à tout moment avant le créneau (portillon, précontrôle, empreinte, créneau).
    ctx.service.cancel(&t.id).unwrap();
    let task = ctx.finished(&t.id).await;
    drop(slot);
    assert_eq!(task.state, TaskState::Cancelled);
    assert!(ctx.service.library_list().await.unwrap().is_empty());
    assert!(ctx.staging_is_empty());
    assert!(ctx.queue().is_empty());
}

/// Une installation active de la même histoire dans l'autre format n'est pas renvoyée comme si
/// c'était celle demandée : refus explicite.
#[tokio::test]
async fn installation_de_l_autre_format_en_cours_refusee() {
    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    let gate = ctx.gate.clone().write_owned().await;
    let mut telmi = story(UUID, Some(3));
    telmi.download_url = ctx
        .dir
        .path()
        .join("src")
        .join("telmi.zip")
        .to_string_lossy()
        .into();
    let first = ctx.service.start(telmi).unwrap();
    let err = ctx.service.start_with(entry, true).unwrap_err();
    assert_eq!(err.kind(), "validation");
    assert!(err.to_string().contains("autre format"), "{err}");
    // Annulée avant que le portillon se libère : la tâche (sans convertisseur) finit « annulée »
    // quel que soit l'instant où elle est ordonnancée (annulation prioritaire, #560).
    ctx.service.cancel(&first.id).unwrap();
    drop(gate);
    let task = ctx.finished(&first.id).await;
    assert_eq!(task.state, TaskState::Cancelled, "{:?}", task.error);
}

#[test]
fn noms_des_dossiers_temporaires_studio() {
    let hex = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        studio_tmp_task(&format!(".{TASK_ID}.lunii.pk.{hex}.tmp")),
        Some(TASK_ID)
    );
    for name in [
        format!("{TASK_ID}.lunii.pk.{hex}.tmp"),
        format!(".{TASK_ID}.plain.pk.{hex}.tmp"),
        format!(".{TASK_ID}.lunii.pk.{}.tmp", hex.to_uppercase()),
        format!(".{TASK_ID}.lunii.pk.{}.tmp", &hex[1..]),
        format!(".{TASK_ID}.lunii.pk.{hex}.tmp.x"),
        format!(".{}.lunii.pk.{hex}.tmp", TASK_ID.to_uppercase()),
        format!(".../.lunii.pk.{hex}.tmp"),
        ".lunii.pk.tmp".to_string(),
        String::new(),
    ] {
        assert_eq!(studio_tmp_task(&name), None, "{name}");
    }
}

/// Reprise au démarrage (#529) : la tâche STUdio relue de la file est relancée et garde la
/// confirmation de changement de format ; les dossiers temporaires de conversion laissés par
/// l'arrêt sont supprimés (#553), et rien d'autre.
#[tokio::test]
async fn reprise_au_demarrage_et_balayage_des_dossiers_temporaires() {
    let dir = tempfile::tempdir().unwrap();
    let (_src, entry) = source(dir.path());
    let cache = dir.path().join("cache");
    install_flam_twin(&cache);
    let staging = Library::new(&cache).staging_dir().unwrap();
    let hex = "0123456789abcdef0123456789abcdef";
    let orphan = staging.join(format!(".{OTHER_TASK}.lunii.pk.{hex}.tmp"));
    let restored = staging.join(format!(".{TASK_ID}.lunii.pk.{hex}.tmp"));
    for d in [&orphan, &restored] {
        std::fs::create_dir_all(d).unwrap();
        std::fs::write(d.join("pack.part"), b"partiel").unwrap();
    }
    let kept_dir = staging.join(".autre.tmp");
    std::fs::create_dir_all(&kept_dir).unwrap();
    // Majuscules (nom distinct de `orphan` même sur un système insensible à la casse).
    let upper = staging.join(format!(
        ".{OTHER_TASK}.lunii.pk.{}.tmp",
        "FEDCBA9876543210FEDCBA9876543210"
    ));
    std::fs::create_dir_all(&upper).unwrap();
    let kept_file = staging.join(format!(
        ".22222222-2222-4333-8444-555555555555.lunii.pk.{hex}.tmp"
    ));
    std::fs::write(&kept_file, b"fichier, pas dossier").unwrap();

    storage::write_json(
        &dir.path().join("config").join(QUEUE_FILE),
        &QueueFile {
            schema_version: QUEUE_SCHEMA,
            tasks: vec![QueuedTask {
                id: TASK_ID.into(),
                entry: entry.clone(),
                created_at: 1,
                validator: Validator::default(),
                pack: None,
                replace_format: true,
            }],
        },
    )
    .unwrap();
    let ctx = Ctx::with_dir(dir, None, GRACE);
    ctx.service
        .restore(|_, _| {
            let entry = entry.clone();
            async move { Ok(entry) }
        })
        .await;
    assert!(!orphan.exists(), "dossier temporaire orphelin supprimé");
    assert!(
        !restored.exists(),
        "dossier de la tâche relancée (repart de zéro) supprimé"
    );
    assert!(kept_dir.is_dir() && upper.is_dir() && kept_file.is_file());

    let task = ctx.finished(TASK_ID).await;
    assert_eq!(task.state, TaskState::Done, "{:?}", task.error);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(
        library[0].file,
        format!("{UUID}-3.lunii.pk"),
        "confirmation reprise"
    );
    assert!(ctx.queue().is_empty());
}
