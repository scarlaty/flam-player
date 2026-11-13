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
    assert_eq!(StoryFormat::of_path(&path), Some(StoryFormat::Lunii));
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
    // Sorties préparées laissées par l'arrêt (#556) : supprimées (tâche inconnue ou relancée).
    let staged_orphans = [
        staging.join(format!("{OTHER_TASK}.lunii.pk")),
        staging.join(format!("{OTHER_TASK}.plain.pk")),
        staging.join(format!("{OTHER_TASK}.plain.pk.4242.part")),
        staging.join(format!("{TASK_ID}.lunii.pk")),
    ];
    for f in &staged_orphans {
        std::fs::write(f, b"partiel").unwrap();
    }
    // Conservés : sauvegarde et marqueur de la bibliothèque, autres noms, UUID non canonique.
    let kept_staged = [
        staging.join(format!("{UUID}-3.plain.pk.bak")),
        staging.join(format!("{UUID}-3.lunii.pk.pending")),
        staging.join("t.plain.pk"),
        staging.join(format!("{OTHER_TASK}.plain.pk.part")),
        staging.join(format!("{OTHER_TASK}.plain.pk.x1.part")),
        staging.join(format!("{OTHER_TASK}.lunii.pk.4242.part")),
        staging.join("33333333-AAAA-4333-8444-555555555555.plain.pk"),
    ];
    for f in &kept_staged {
        std::fs::write(f, b"garder").unwrap();
    }
    // Dossier au nom d'une sortie (un fichier est attendu) : conservé.
    let staged_dir = staging.join("44444444-2222-4333-8444-555555555555.plain.pk");
    std::fs::create_dir_all(staged_dir.join("x")).unwrap();
    // Jonctions (Windows, sans privilège) aux noms exacts d'un reste : jamais suivies ni
    // supprimées, leur cible reste intacte.
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("precieux.txt"), b"precieux").unwrap();
    let links = [
        staging.join("55555555-2222-4333-8444-555555555555.plain.pk"),
        staging.join(format!(
            ".55555555-2222-4333-8444-555555555555.lunii.pk.{hex}.tmp"
        )),
    ];
    if cfg!(windows) {
        for link in &links {
            let out = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(outside.path())
                .output()
                .unwrap();
            assert!(out.status.success(), "mklink /J : {out:?}");
        }
    }

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
    for f in &staged_orphans {
        assert!(
            !f.exists(),
            "sortie préparée orpheline supprimée : {}",
            f.display()
        );
    }
    for f in &kept_staged {
        assert!(f.is_file(), "conservé : {}", f.display());
    }
    assert!(
        staged_dir.join("x").is_dir(),
        "dossier au nom d'une sortie conservé"
    );
    if cfg!(windows) {
        for link in &links {
            assert!(
                std::fs::symlink_metadata(link).is_ok(),
                "jonction conservée"
            );
        }
    }
    assert_eq!(
        std::fs::read(outside.path().join("precieux.txt")).unwrap(),
        b"precieux",
        "cible des jonctions intacte"
    );

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

#[test]
fn noms_des_sorties_preparees() {
    assert_eq!(staged_task(&format!("{TASK_ID}.plain.pk")), Some(TASK_ID));
    assert_eq!(staged_task(&format!("{TASK_ID}.lunii.pk")), Some(TASK_ID));
    assert_eq!(
        staged_task(&format!("{TASK_ID}.plain.pk.1.part")),
        Some(TASK_ID)
    );
    assert_eq!(
        staged_task(&format!("{TASK_ID}.plain.pk.4294967295.part")),
        Some(TASK_ID)
    );
    for name in [
        format!("{TASK_ID}.plain.pk.12345678901.part"),
        format!("{TASK_ID}.plain.pk..part"),
        format!("{TASK_ID}.plain.pk.-1.part"),
        format!("{TASK_ID}.lunii.pk.1.part"),
        format!("{TASK_ID}.plain.pk.part"),
        format!("{TASK_ID}.plain.pk.bak"),
        format!("{TASK_ID}.plain.pk.pending"),
        format!("{TASK_ID}.PLAIN.PK"),
        format!("{}.plain.pk", TASK_ID.to_uppercase()),
        format!("{UUID}-3.plain.pk"),
        format!(".{TASK_ID}.plain.pk"),
        format!("../{TASK_ID}.plain.pk"),
        ".plain.pk".to_string(),
        String::new(),
    ] {
        assert_eq!(staged_task(&name), None, "{name}");
    }
    let hex = "0123456789abcdef0123456789abcdef";
    assert_eq!(
        staging_leftover(&format!(".{TASK_ID}.lunii.pk.{hex}.tmp")),
        Some((TASK_ID, true))
    );
    assert_eq!(
        staging_leftover(&format!("{TASK_ID}.lunii.pk")),
        Some((TASK_ID, false))
    );
}

/// Restes d'installation dans `staging` : un dossier temporaire STUdio et une sortie préparée
/// de `OTHER_TASK`.
fn staging_leftovers(staging: &Path) -> [PathBuf; 2] {
    let hex = "0123456789abcdef0123456789abcdef";
    let tmp = staging.join(format!(".{OTHER_TASK}.lunii.pk.{hex}.tmp"));
    std::fs::create_dir_all(&tmp).unwrap();
    std::fs::write(tmp.join("pack.part"), b"partiel").unwrap();
    let staged = staging.join(format!("{OTHER_TASK}.plain.pk"));
    std::fs::write(&staged, b"partiel").unwrap();
    [tmp, staged]
}

/// #591 : `library/` sans marqueur de l'application (homonyme étranger) : `.staging/` n'est ni
/// lu ni balayé.
#[test]
fn le_balayage_de_staging_ne_touche_pas_une_bibliotheque_sans_marqueur() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("library").join(crate::library::STAGING_DIR);
    std::fs::create_dir_all(&staging).unwrap();
    let leftovers = staging_leftovers(&staging);
    let read = std::cell::Cell::new(false);
    sweep_staging_in(&staging, &|names| {
        read.set(true);
        names
    });
    assert!(!read.get(), "contenu jamais relevé");
    for path in &leftovers {
        assert!(path.exists(), "{} supprimé", path.display());
    }
}

/// #591 : `library/` marqué : les restes orphelins sont balayés comme avant.
#[test]
fn le_balayage_de_staging_supprime_les_orphelins_d_une_bibliotheque_marquee() {
    let dir = tempfile::tempdir().unwrap();
    let library = crate::cache::ensure_subdir(dir.path(), "library").unwrap();
    let staging = library.join(crate::library::STAGING_DIR);
    std::fs::create_dir_all(&staging).unwrap();
    let leftovers = staging_leftovers(&staging);
    sweep_staging_in(&staging, &|names| names);
    for path in &leftovers {
        assert!(!path.exists(), "{} gardé", path.display());
    }
    assert!(crate::cache::is_owned(&library), "marqueur gardé");
}

/// Message exact du refus d'un pack qui ne correspond pas à l'entrée analysée (#556).
#[test]
fn refus_d_un_pack_different_de_l_analyse() {
    let mut entry = story(UUID, Some(3));
    assert_eq!(studio_mismatch(&entry, UUID, 3), None);
    let other = studio_mismatch(&entry, OTHER_UUID, 3).unwrap();
    assert!(
        other.contains("a changé") && other.contains("identifiant"),
        "{other}"
    );
    let version = studio_mismatch(&entry, UUID, 4).unwrap();
    assert!(
        version.contains("a changé") && version.contains("version 4 au lieu de 3"),
        "{version}"
    );
    entry.version = None;
    let unknown = studio_mismatch(&entry, UUID, 3).unwrap();
    assert!(!unknown.contains("a changé"), "{unknown}");
    assert!(unknown.contains("pas été reconnue"), "{unknown}");
}

/// Remplacement confirmé de l'histoire Flam par sa version STUdio : la copie de travail du
/// player (`<cache>/play/…`) de l'ancienne version est purgée, la progression conservée (#556).
#[tokio::test]
async fn changement_de_format_purge_la_copie_de_travail() {
    use crate::player::{purge_library_story, Workspace, PLAY_DIR};
    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    install_flam_twin(&ctx.cache());
    let saves = ctx.dir.path().join("config").join("saves");
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    {
        let (saves, calls) = (saves.clone(), calls.clone());
        ctx.service
            .set_play_purger(Arc::new(move |cache: &Path, uuid: &str| {
                lock(&calls).push(uuid.to_string());
                purge_library_story(cache, &saves, uuid, false)
            }));
    }
    // Copie de travail de l'histoire Flam, avec une partie en cours dans `saves/`.
    let (flam, _) = Library::new(&ctx.cache()).playable(UUID).unwrap();
    let ws = Workspace::new(
        &ctx.cache().join(PLAY_DIR),
        &saves,
        &format!("library:{UUID}"),
    );
    let copy = ws.stage(&flam).unwrap();
    let work = copy.parent().unwrap().to_path_buf();
    std::fs::create_dir_all(work.join("saves")).unwrap();
    std::fs::write(work.join("saves").join("etat"), b"niveau 4").unwrap();

    let t = ctx.service.start_with(entry, true).unwrap();
    let task = ctx.finished(&t.id).await;
    assert_eq!(task.state, TaskState::Done, "{:?}", task.error);
    assert_eq!(*lock(&calls), vec![UUID.to_string()]);
    assert!(!work.exists(), "copie de travail purgée");
    let kept = std::fs::read_dir(&saves)
        .unwrap()
        .flatten()
        .any(|d| d.path().join("etat").is_file());
    assert!(kept, "progression rapatriée et conservée");
}

/// De la bibliothèque à la Flam (#556) : le pack STUdio installé est envoyé par
/// `send_from_library_with` sur un faux appareil en firmware 2.x ; une Flam 1.x le refuse avant
/// toute écriture.
#[tokio::test]
async fn envoi_complet_bibliotheque_vers_un_faux_appareil() {
    use crate::device::detect::MDF_FILE;
    use crate::device::guard::tests::{mdf_v1_real, test_ctx};
    use crate::device::transfer::tests::{device, listed, mdf, snapshot, write, OTHER};
    use crate::device::transfer::{send_from_library_with, RealFs, TransferError, WORK_SUBDIR};

    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    let t = ctx.service.start(entry).unwrap();
    assert_eq!(ctx.finished(&t.id).await.state, TaskState::Done);
    let (pk, _) = Library::new(&ctx.cache()).playable(UUID).unwrap();
    let work = ctx.cache().join(WORK_SUBDIR);
    let send = |root: &Path| {
        send_from_library_with(
            &RealFs::default(),
            root,
            &pk,
            &work,
            None,
            None,
            &test_ctx(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
    };

    // Flam 1.x : refus, rien d'écrit ni de préparé.
    // `device` écrit un `.mdf` de Flam 1.x réelle (firmware 1.x).
    let root1 = device(&ctx.dir.path().join("flam1"), false);
    assert_eq!(std::fs::read(root1.join(MDF_FILE)).unwrap(), mdf_v1_real());
    let before = snapshot(&root1);
    let err = send(&root1).unwrap_err();
    assert!(matches!(err, TransferError::LuniiNeedsV2), "{err:?}");
    assert_eq!(snapshot(&root1), before, "Flam 1.x inchangée");
    assert!(!work.exists(), "préparation jamais lancée");

    // Flam 2.x : envoi complet, l'histoire est listée sous l'uuid du scan.
    let root2 = device(&ctx.dir.path().join("flam2"), false);
    write(&root2.join(MDF_FILE), &mdf("2.4.2"));
    let report = send(&root2).unwrap();
    assert_eq!(report.id, UUID);
    assert!(report.index_updated && !report.replaced);
    let story_dir = root2.join("str").join(UUID);
    for name in ["ni", "li", "ri", "si", "info", "key", "img/thumbnail.lif"] {
        assert!(story_dir.join(name).is_file(), "{name}");
    }
    let ids: Vec<String> = listed(&root2).iter().map(|i| i.to_string()).collect();
    assert_eq!(ids, vec![OTHER.to_string(), UUID.to_string()]);
    assert!(
        !work.join("str").join(UUID).exists(),
        "préparation supprimée"
    );
}

/// Purge refusée (`player_busy` : le player lit l'ancienne version) : rien n'est supprimé et
/// l'installation reste réussie (#556).
#[tokio::test]
async fn purge_refusee_n_empeche_pas_l_installation() {
    let ctx = Ctx::new(None);
    let (_src, entry) = source(ctx.dir.path());
    install_flam_twin(&ctx.cache());
    let calls = Arc::new(AtomicUsize::new(0));
    {
        let calls = calls.clone();
        ctx.service
            .set_play_purger(Arc::new(move |_: &Path, _: &str| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(AppError::PlayerBusy("en cours de lecture".into()))
            }));
    }
    let t = ctx.service.start_with(entry, true).unwrap();
    let task = ctx.finished(&t.id).await;
    assert_eq!(task.state, TaskState::Done, "{:?}", task.error);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library[0].file, format!("{UUID}-3.lunii.pk"));
}

/// Archive STUdio **téléchargée** (#578) : mise en cause par la conversion (`invalid_archive` :
/// zip lisible sans `story.json`), elle est supprimée avec sa mémoire, comme pour `telmi2flam` ;
/// panne étrangère à l'archive (pack d'un autre `uuid` que l'entrée : `validation`), elle est
/// gardée et sera réutilisée sans réseau.
#[tokio::test]
async fn archive_studio_telechargee_mise_en_cause_supprimee() {
    let mut without_story = Vec::new();
    {
        let mut zip = zip::ZipWriter::new(Cursor::new(&mut without_story));
        zip.start_file("lisez-moi.txt", SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"pas une histoire").unwrap();
        zip.finish().unwrap();
    }
    let valid = {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conte.zip");
        studio_zip(&path, NODE_UUID);
        std::fs::read(&path).unwrap()
    };
    for (what, bytes, kind, removed) in [
        ("invalid_archive", without_story, "conversion", true),
        ("autre uuid", valid, "validation", false),
    ] {
        let ctx = Ctx::new(None);
        let server = serve(bytes).await;
        let mut entry = remote(&format!("{}/a.zip", server.uri()), OTHER_UUID);
        entry.kind = StoryKind::StudioZip;
        let t = ctx.service.start(entry).unwrap();
        let task = ctx.finished(&t.id).await;
        assert_eq!(task.state, TaskState::Failed, "{what}");
        let error = task.error.unwrap();
        assert_eq!(error.kind, kind, "{what} : {}", error.message);
        if removed {
            assert_eq!(error.code.as_deref(), Some(what), "{what}");
        }
        let archive = ctx
            .cache()
            .join("archives")
            .join(format!("{OTHER_UUID}-2.zip"));
        assert_eq!(!archive.exists(), removed, "{what} : archive");
        assert_eq!(!meta_path(&archive).exists(), removed, "{what} : mémoire");
        assert!(!download::part_path(&archive).exists(), "{what}");
        assert!(
            ctx.service.library_list().await.unwrap().is_empty(),
            "{what}"
        );
        assert!(ctx.staging_is_empty(), "{what}");
    }
}

/// #578 : une erreur d'entrée/sortie **du système** (code de l'OS : antivirus, volume retiré) en
/// lisant l'archive STUdio rend `io` et ne met pas l'archive en cause ; une erreur construite par
/// le lecteur zip (CRC faux, troncature, en-tête invalide) rend `invalid_archive` et la met en
/// cause.
#[test]
fn erreur_systeme_a_la_lecture_ne_met_pas_l_archive_en_cause() {
    use crate::install::studio::{read_error, zip_read_error};
    use std::io::{Error, ErrorKind};
    use zip::result::ZipError;

    // 32 : `ERROR_SHARING_VIOLATION` sous Windows (fichier verrouillé par un autre processus),
    // `EPIPE` sous Unix. 5 : `ERROR_ACCESS_DENIED` sous Windows (antivirus), `EIO` sous Unix
    // (limite connue : gardée).
    let system = [
        read_error("son illisible", Error::from_raw_os_error(32)),
        zip_read_error("zip", ZipError::Io(Error::from_raw_os_error(32))),
        zip_read_error("entrée 0", ZipError::Io(Error::from_raw_os_error(5))),
    ];
    for err in system {
        assert_eq!(err.kind(), "io", "{err}");
        assert!(!crate::install::archive_at_fault(&err), "{err}");
    }
    let mut corrupt = vec![
        read_error(
            "son illisible",
            Error::new(ErrorKind::InvalidData, "Invalid checksum"),
        ),
        // Vraie erreur de l'OS d'un positionnement absurde : `ERROR_INVALID_PARAMETER` (87,
        // rendu par Windows pour un zip64 forgé), `EINVAL` (22) sous Unix.
        zip_read_error(
            "entrée 0",
            ZipError::Io(Error::from_raw_os_error(if cfg!(windows) {
                87
            } else {
                22
            })),
        ),
        read_error("image illisible", Error::from(ErrorKind::UnexpectedEof)),
        zip_read_error(
            "zip",
            ZipError::InvalidArchive("pas de répertoire central".into()),
        ),
        zip_read_error(
            "entrée 0",
            ZipError::Io(Error::from(ErrorKind::UnexpectedEof)),
        ),
        // Positionnement absurde (décalage forgé d'un zip64) refusé par le système.
        zip_read_error(
            "entrée 0",
            ZipError::Io(Error::new(ErrorKind::InvalidInput, "seek")),
        ),
    ];
    // Windows : contenu stocké illisible (`ERROR_CRC`, `ERROR_FILE_CORRUPT`,
    // `ERROR_DISK_CORRUPT`) : retéléchargé.
    if cfg!(windows) {
        for code in [23, 1392, 1393] {
            corrupt.push(read_error("son illisible", Error::from_raw_os_error(code)));
        }
    }
    for err in corrupt {
        assert_eq!(err.kind(), "conversion", "{err}");
        assert!(crate::install::archive_at_fault(&err), "{err}");
        assert!(
            matches!(&err, AppError::Conversion { code, .. } if code == "invalid_archive"),
            "{err}"
        );
    }
}
