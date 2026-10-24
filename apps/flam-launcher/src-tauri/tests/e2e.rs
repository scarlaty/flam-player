//! Test de bout en bout sur le backend réel, sans WebView (issue #21).
//!
//! Enchaîne les services réels de l'application, comme le font les commandes Tauri :
//!
//! 1. un catalogue local factice (serveur HTTP, format Telmi Store) propose une histoire ;
//! 2. `CatalogService` le télécharge ; `InstallService` télécharge l'archive et la convertit
//!    avec le **vrai** `telmi2flam.exe` (aucun Python, #138) ;
//! 3. la bibliothèque la liste ; `PlayerService` lance le **vrai** `flam-player.exe` en pilotes
//!    SDL `dummy` et rendu logiciel (ni fenêtre ni carte son ni GPU) sur une copie de travail ;
//! 4. le player est arrêté par l'ordre `quit` (sortie volontaire, code 0) et l'événement `exited` est reçu (retour à la surcouche).
//!
//! L'archive est synthétique (aucun contenu sous licence : images unies, MP3 muets).
//!
//! Outils cherchés, dans l'ordre :
//! - `FLAM_E2E_INSTALL_DIR` : dossier d'une application installée (ou `target/release`, à la racine du workspace Cargo, après
//!   `tauri build` avec `tauri.bundle.conf.json`) : `flam-player.exe` et `telmi2flam/telmi2flam.exe`
//!   sont détectés comme le fait l'application (`settings::detect_player`,
//!   `install::detect_rust_converter`) ;
//! - `FLAM_E2E_PLAYER` (chemin de `flam-player.exe`) et `FLAM_E2E_RUST_CONVERTER` (chemin de
//!   `telmi2flam.exe`, `cargo build -p telmi2flam`).
//!
//! Sans `flam-player.exe` ni convertisseur, le test se saute avec un message ;
//! `FLAM_E2E_REQUIRE=1` (CI) le transforme en échec.

#![cfg(windows)]

use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use flam_launcher_lib::catalog::CatalogService;
use flam_launcher_lib::install::{
    self, InstallDone, InstallEvents, InstallFailed, InstallOptions, InstallProgress,
    InstallService, Resolver, Toolchain,
};
use flam_launcher_lib::library::Library;
use flam_launcher_lib::player::{
    PlayerEvents, PlayerExited, PlayerService, PlayerSession, Workspace, PLAY_DIR,
};
use flam_launcher_lib::settings::{self, Environment};
use flam_launcher_lib::story::StoryKind;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const STORY_UUID: &str = "e2e-0001";

struct Tools {
    player: PathBuf,
    /// `telmi2flam.exe` : détecté dans l'installation, ou `FLAM_E2E_RUST_CONVERTER`.
    converter: PathBuf,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn skip_or_fail(reason: &str) -> Option<Tools> {
    assert!(
        std::env::var_os("FLAM_E2E_REQUIRE").is_none(),
        "prérequis du test e2e absents : {reason}"
    );
    eprintln!("e2e ignoré : {reason} (FLAM_E2E_REQUIRE=1 pour en faire un échec)");
    None
}

fn find_tools() -> Option<Tools> {
    if let Some(dir) = env_path("FLAM_E2E_INSTALL_DIR") {
        let env = Environment {
            exe_dir: Some(dir.clone()),
            default_cache_dir: dir.join("cache-inutilise"),
            search_repo_build: false,
            protected_dirs: Vec::new(),
        };
        let Some(player) = settings::detect_player(&env) else {
            return skip_or_fail(&format!("flam-player.exe absent de {}", dir.display()));
        };
        let Some(converter) = install::detect_rust_converter(&env) else {
            return skip_or_fail(&format!(
                "telmi2flam/telmi2flam.exe absent de {}",
                dir.display()
            ));
        };
        return Some(Tools { player, converter });
    }
    let Some(player) = env_path("FLAM_E2E_PLAYER").filter(|p| p.is_file()) else {
        return skip_or_fail("FLAM_E2E_PLAYER (flam-player.exe) non défini ou absent");
    };
    let Some(converter) = env_path("FLAM_E2E_RUST_CONVERTER").filter(|p| p.is_file()) else {
        return skip_or_fail("FLAM_E2E_RUST_CONVERTER (telmi2flam.exe) non défini ou absent");
    };
    Some(Tools { player, converter })
}

// ---------------------------------------------------------------------------------------
// Histoire synthétique (équivalent Rust de tests/e2e/e2e_telmi.py : aucun contenu sous licence)
// ---------------------------------------------------------------------------------------

fn png_bytes(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        let pixels: Vec<u8> = (0..w * h).flat_map(|_| rgba).collect();
        writer.write_image_data(&pixels).unwrap();
    }
    out
}

/// MP3 muet : `n` trames MPEG-1 couche III, 128 kb/s, 44,1 kHz (417 octets chacune, ~1 s pour 40).
fn mp3_silence(n: usize) -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    frame.repeat(n)
}

/// Paquet Telmi : démarrage, scène audio, choix à deux options, fin.
fn story_zip() -> Vec<u8> {
    let control = json!({"ok": true, "home": false, "autoplay": true});
    let option = json!({"ok": true, "home": false, "autoplay": false});
    let nodes = json!({
        "startAction": {"action": "a0", "index": 0},
        "stages": {
            "p0": {"image": null, "audio": null, "ok": {"action": "a1", "index": 0}, "home": null, "control": control},
            "s0": {"image": "s0.png", "audio": "s0.mp3", "ok": {"action": "a2", "index": 0}, "home": null, "control": control},
            "s1": {"image": "s1.png", "audio": "s1.mp3", "ok": null, "home": null, "control": option},
            "s2": {"image": "s2.png", "audio": "s2.mp3", "ok": null, "home": null, "control": option},
        },
        "actions": {
            "a0": [{"stage": "p0"}],
            "a1": [{"stage": "s0"}],
            "a2": [{"stage": "s1"}, {"stage": "s2"}],
        },
    });
    let notes = json!({"s1": {"title": "Option un"}, "s2": {"title": "Option deux"}});
    let mp3 = mp3_silence(40);
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "metadata.json",
            json!({"title": "Histoire E2E", "uuid": STORY_UUID})
                .to_string()
                .into_bytes(),
        ),
        ("nodes.json", nodes.to_string().into_bytes()),
        ("notes.json", notes.to_string().into_bytes()),
        ("title.png", png_bytes(64, 48, [200, 10, 10, 255])),
        ("images/s0.png", png_bytes(320, 240, [10, 200, 10, 255])),
        ("images/s1.png", png_bytes(320, 240, [10, 10, 200, 255])),
        ("images/s2.png", png_bytes(320, 240, [200, 200, 10, 255])),
        ("audios/s0.mp3", mp3.clone()),
        ("audios/s1.mp3", mp3.clone()),
        ("audios/s2.mp3", mp3),
    ];
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in files {
        zip.start_file(name, options).unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.finish().unwrap().into_inner()
}

// ---------------------------------------------------------------------------------------
// Enregistreurs d'événements
// ---------------------------------------------------------------------------------------

enum Finished {
    Done(Box<InstallDone>),
    Failed(Box<InstallFailed>),
}

struct InstallRecorder(Mutex<Sender<Finished>>);

impl InstallEvents for InstallRecorder {
    fn progress(&self, _: &InstallProgress) {}
    fn done(&self, event: &InstallDone) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .send(Finished::Done(Box::new(event.clone())));
    }
    fn failed(&self, event: &InstallFailed) {
        let _ = self
            .0
            .lock()
            .unwrap()
            .send(Finished::Failed(Box::new(event.clone())));
    }
}

struct PlayerRecorder {
    started: Mutex<Sender<PlayerSession>>,
    exited: Mutex<Sender<PlayerExited>>,
}

impl PlayerEvents for PlayerRecorder {
    fn started(&self, session: &PlayerSession) {
        let _ = self.started.lock().unwrap().send(session.clone());
    }
    fn exited(&self, event: &PlayerExited) {
        let _ = self.exited.lock().unwrap().send(event.clone());
    }
}

/// Filet de sécurité : ferme le player (et ses enfants) quand le test se termine, y compris sur un échec : sinon un
/// `assert!` en cours de route laisserait `flam-player.exe` vivant.
struct KillOnDrop(u32, std::sync::atomic::AtomicBool);

impl KillOnDrop {
    fn kill(&self) -> std::io::Result<std::process::Output> {
        std::process::Command::new("taskkill")
            .args(["/PID", &self.0.to_string(), "/F", "/T"])
            .output()
    }
}

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        // Désarmé après une sortie volontaire : le pid pourrait avoir été réutilisé.
        if !self.1.load(std::sync::atomic::Ordering::SeqCst) {
            let _ = self.kill();
        }
    }
}

fn find_staged_main_lua(play_dir: &Path) -> Option<PathBuf> {
    for work in std::fs::read_dir(play_dir).ok()?.flatten() {
        let Ok(entries) = std::fs::read_dir(work.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let main = entry.path().join("main.lua");
            if main.is_file() {
                return Some(main);
            }
        }
    }
    None
}

/// Les variables SDL sont positionnées avant de créer le runtime multi-thread : aucune autre
/// thread ne lit l'environnement pendant ce temps.
#[test]
fn catalogue_installation_lancement_retour() {
    std::env::set_var("SDL_VIDEODRIVER", "dummy");
    std::env::set_var("SDL_AUDIODRIVER", "dummy");
    // Sans carte graphique, `SDL_CreateRenderer(ACCELERATED)` échoue : rendu logiciel.
    std::env::set_var("SDL_RENDER_DRIVER", "software");
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
        .block_on(scenario());
}

async fn scenario() {
    let Some(tools) = find_tools() else {
        return;
    };
    eprintln!(
        "e2e : player {}, convertisseur {}",
        tools.player.display(),
        tools.converter.display()
    );
    let dir = tempfile::tempdir().unwrap();
    let cache_root = dir.path().join("cache");
    let config_dir = dir.path().join("config");
    std::fs::create_dir_all(&cache_root).unwrap();

    // 1. Catalogue local factice (format Telmi Store) et archive synthétique.
    let archive = story_zip();
    let server = MockServer::start().await;
    let catalog = json!({"data": [{
        "uuid": STORY_UUID,
        "title": "Histoire E2E",
        "version": 1,
        "download": "/e2e.zip",
        "size": archive.len(),
    }]});
    Mock::given(method("GET"))
        .and(path("/catalog.json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(catalog))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/e2e.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(archive))
        .mount(&server)
        .await;
    let catalog_url = format!("{}/catalog.json", server.uri());

    // 2. Rafraîchissement du catalogue puis installation (téléchargement + conversion réelle).
    let catalogs = CatalogService::with_client(install::download::build_client().unwrap());
    let view = catalogs
        .refresh(&cache_root, "e2e", &catalog_url, 0)
        .await
        .expect("catalogue lisible");
    assert_eq!(
        view.entries.len(),
        1,
        "avertissements : {:?}",
        view.warnings
    );
    let entry = view.entries.into_iter().next().unwrap();
    assert_eq!(entry.kind, StoryKind::TelmiZip);
    assert_eq!(entry.uuid, STORY_UUID);

    let (install_tx, install_rx) = channel();
    let toolchain = Toolchain {
        cache_root: cache_root.clone(),
        ffmpeg: None,
        rust_converter: Some(tools.converter.clone()),
        max_downloads: 2,
        thumb_fit: Default::default(),
    };
    let resolver: Resolver = Arc::new(move || Ok(toolchain.clone()));
    let installer = InstallService::new(InstallOptions {
        client: install::download::build_client().unwrap(),
        resolver,
        events: Arc::new(InstallRecorder(Mutex::new(install_tx))),
        queue_path: config_dir.join(install::QUEUE_FILE),
        gate: catalogs.cache_gate(),
        grace: install::GRACE,
    });
    let task = installer
        .start(entry.clone())
        .expect("installation acceptée");
    let finished =
        tokio::task::spawn_blocking(move || install_rx.recv_timeout(Duration::from_secs(180)))
            .await
            .unwrap()
            .expect("fin de l'installation avant le délai");
    let done = match finished {
        Finished::Done(done) => done,
        Finished::Failed(failed) => panic!("installation en échec : {:?}", failed.error),
    };
    assert_eq!(done.task_id, task.id);
    assert_eq!(done.entry.uuid, STORY_UUID);

    // 3. Bibliothèque : l'histoire convertie est là, c'est un vrai `.plain.pk` (zip).
    let installed = installer.library_list().await.unwrap();
    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].title, "Histoire E2E");
    let library = Library::new(&cache_root);
    let (story_file, title) = library.playable(STORY_UUID).expect("histoire jouable");
    assert_eq!(title, "Histoire E2E");
    assert!(story_file.to_string_lossy().ends_with(".plain.pk"));
    assert_eq!(&std::fs::read(&story_file).unwrap()[..2], b"PK");

    // 4. Lancement du vrai player en pilotes SDL « dummy » (rendu logiciel), depuis une copie de travail.
    let (started_tx, started_rx) = channel();
    let (exited_tx, exited_rx) = channel();
    let player = PlayerService::new(Arc::new(PlayerRecorder {
        started: Mutex::new(started_tx),
        exited: Mutex::new(exited_tx),
    }));
    let saves_root = config_dir.join(flam_launcher_lib::player::SAVES_ROOT);
    let workspace = Workspace::new(
        &cache_root.join(PLAY_DIR),
        &saves_root,
        &format!("library:{STORY_UUID}"),
    );
    let (staging, saving) = (workspace.clone(), workspace);
    let session = player
        .launch(
            &tools.player,
            || staging.stage(&story_file),
            move || saving.persist_saves_logged(),
            "library",
            STORY_UUID,
            &title,
        )
        .expect("lancement du player");
    let _killer = KillOnDrop(session.pid, std::sync::atomic::AtomicBool::new(false));
    let started = started_rx
        .recv_timeout(Duration::from_secs(30))
        .expect("événement started");
    assert_eq!(started.pid, session.pid);

    // Le player charge l'histoire (extraction du .plain.pk) puis reste actif : s'il plantait
    // ou refusait l'histoire, il se terminerait seul.
    let deadline = Instant::now() + Duration::from_secs(60);
    let main_lua = loop {
        if let Ok(early) = exited_rx.try_recv() {
            panic!("flam-player s'est arrêté seul au chargement : {early:?}");
        }
        if let Some(main) = find_staged_main_lua(&cache_root.join(PLAY_DIR)) {
            break main;
        }
        assert!(
            Instant::now() < deadline,
            "histoire jamais extraite par le player"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    tokio::time::sleep(Duration::from_secs(4)).await;
    if let Ok(early) = exited_rx.try_recv() {
        panic!(
            "flam-player s'est arrêté seul ({}) : {early:?}",
            main_lua.display()
        );
    }
    assert!(
        player.status().is_some(),
        "créneau du player libéré trop tôt"
    );

    // 5. Retour : le player est fermé, la surcouche reçoit `exited` et libère le créneau.
    // Sortie volontaire (#119) : `PlayerService::stop` envoie `quit` sur stdin, le player
    // sauvegarde et sort avec le code 0. `_killer` ne sert plus que de filet si le test échoue.
    assert!(player.stop(), "aucun player à arrêter");
    let exited =
        tokio::task::spawn_blocking(move || exited_rx.recv_timeout(Duration::from_secs(10)))
            .await
            .unwrap()
            .expect("événement exited");
    assert_eq!(exited.session.uuid, STORY_UUID);
    assert_eq!(exited.session.pid, session.pid);
    assert!(
        exited.success && exited.code == Some(0),
        "sortie non volontaire : code {:?}, stderr {}",
        exited.code,
        exited.stderr
    );
    for bad in ["Erreur", "ASSERT", "attempt to", "CRASH", "unprotected"] {
        assert!(
            !exited.stderr.contains(bad),
            "stderr du player : {}",
            exited.stderr
        );
    }
    _killer.1.store(true, std::sync::atomic::Ordering::SeqCst);
    let release = Instant::now() + Duration::from_secs(5);
    while player.status().is_some() {
        assert!(Instant::now() < release, "créneau du player jamais libéré");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
