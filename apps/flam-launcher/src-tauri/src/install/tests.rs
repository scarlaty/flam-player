//! Tests du pipeline : serveur HTTP local, faux convertisseur (`convert::fake`) émettant des
//! événements JSON, annulations, reprise après redémarrage simulé, erreurs typées.

use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::library::tests::story;

/// Faux `telmi2flam.exe` (lanceur du faux convertisseur, voir `convert::fake`), qui exige Python
/// dans le `PATH`. Avec `FLAM_REQUIRE_PATH_PYTHON=1` (CI), son absence fait échouer le test au
/// lieu de le sauter en silence (#426).
fn fake_converter() -> Option<PathBuf> {
    let found = convert::fake::exe();
    if found.is_none() {
        assert!(
            std::env::var_os("FLAM_REQUIRE_PATH_PYTHON").is_none_or(|v| v != "1"),
            "FLAM_REQUIRE_PATH_PYTHON=1 : Python introuvable dans le PATH"
        );
        eprintln!("Python introuvable : test de conversion ignoré");
    }
    found
}

#[derive(Debug, Clone, PartialEq)]
enum Recorded {
    Progress(InstallProgress),
    Done(Box<InstallDone>),
    Failed(InstallFailed),
}

#[derive(Default)]
struct Recorder(Mutex<Vec<Recorded>>);

impl InstallEvents for Recorder {
    fn progress(&self, e: &InstallProgress) {
        lock(&self.0).push(Recorded::Progress(e.clone()));
    }
    fn done(&self, e: &InstallDone) {
        lock(&self.0).push(Recorded::Done(Box::new(e.clone())));
    }
    fn failed(&self, e: &InstallFailed) {
        lock(&self.0).push(Recorded::Failed(e.clone()));
    }
}

impl Recorder {
    fn all(&self) -> Vec<Recorded> {
        lock(&self.0).clone()
    }
    fn states(&self, id: &str) -> Vec<TaskState> {
        let mut states: Vec<TaskState> = Vec::new();
        for r in self.all() {
            if let Recorded::Progress(p) = r {
                if p.task_id == id && states.last() != Some(&p.state) {
                    states.push(p.state);
                }
            }
        }
        states
    }
}

struct Ctx {
    dir: tempfile::TempDir,
    events: Arc<Recorder>,
    service: InstallService,
    gate: Arc<RwLock<()>>,
}

impl Ctx {
    /// `exe` : faux `telmi2flam.exe` (l'interpréteur Python, voir `convert::fake`) ; `None` :
    /// binaire absent.
    fn new(exe: Option<PathBuf>) -> Self {
        Self::with_dir(tempfile::tempdir().unwrap(), exe, Duration::from_secs(5))
    }

    fn with_dir(dir: tempfile::TempDir, exe: Option<PathBuf>, grace: Duration) -> Self {
        let cache_root = dir.path().join("cache");
        let resolver: Resolver = Arc::new(move || {
            Ok(Toolchain {
                cache_root: cache_root.clone(),
                rust_converter: exe.clone(),
                ffmpeg: None,
                max_downloads: 2,
                thumb_fit: crate::settings::ThumbFit::default(),
            })
        });
        Self::with_resolver(dir, resolver, grace)
    }

    fn with_resolver(dir: tempfile::TempDir, resolver: Resolver, grace: Duration) -> Self {
        let events = Arc::new(Recorder::default());
        let gate = Arc::new(RwLock::new(()));
        let service = InstallService::new(InstallOptions {
            client: download::build_client().unwrap(),
            resolver,
            events: events.clone(),
            queue_path: dir.path().join("config").join(QUEUE_FILE),
            gate: gate.clone(),
            grace,
        });
        Self {
            dir,
            events,
            service,
            gate,
        }
    }

    fn cache(&self) -> PathBuf {
        self.dir.path().join("cache")
    }

    fn task(&self, id: &str) -> InstallTask {
        self.service
            .list()
            .into_iter()
            .find(|t| t.id == id)
            .expect("tâche connue")
    }

    async fn wait(&self, id: &str, what: impl Fn(&InstallTask) -> bool) -> InstallTask {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let task = self.task(id);
            if what(&task) {
                return task;
            }
            assert!(Instant::now() < deadline, "attente expirée : {task:?}");
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn finished(&self, id: &str) -> InstallTask {
        let task = self.wait(id, |t| !t.state.is_active()).await;
        self.service.flush_queue().await;
        // L'événement final suit l'écriture de la file : on l'attend aussi.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline
            && !self.events.all().iter().any(|e| match e {
                Recorded::Done(d) => d.task_id == id,
                Recorded::Failed(f) => f.task_id == id,
                Recorded::Progress(_) => false,
            })
        {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        task
    }

    fn queue(&self) -> Vec<QueuedTask> {
        load_queue(&self.dir.path().join("config").join(QUEUE_FILE))
    }

    fn staging_is_empty(&self) -> bool {
        let staging = self.cache().join("library").join(".staging");
        std::fs::read_dir(staging).map_or(true, |mut d| d.next().is_none())
    }
}

/// Archive zip « stored » réelle dont le faux convertisseur lit le mode (`mode=<x>;`).
fn zip_bytes(mode: &str) -> Vec<u8> {
    zip_padded(mode, 0)
}

fn zip_padded(mode: &str, pad: usize) -> Vec<u8> {
    convert::fake::archive(mode, pad)
}

const TASK_ID: &str = "0b8f3c1e-4a5d-4e6f-9a7b-8c9d0e1f2a3b";

fn remote(url: &str, uuid: &str) -> StoryEntry {
    let mut entry = story(uuid, Some(2));
    entry.download_url = url.to_string();
    entry
}

async fn serve(body: Vec<u8>) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/a.zip"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(body)
                .insert_header("ETag", "\"v1\""),
        )
        .mount(&server)
        .await;
    server
}

#[tokio::test]
async fn telechargement_conversion_et_installation() {
    let Some(py) = fake_converter() else { return };
    let ctx = Ctx::new(Some(py));
    let server = serve(zip_bytes("noise")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    assert_eq!(task.state, TaskState::Queued);
    // Une seconde demande pour le même uuid renvoie la même tâche.
    let again = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    assert_eq!(again.id, task.id);

    let done = ctx.finished(&task.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    assert_eq!(done.percent, Some(100.0));
    assert_eq!(
        ctx.events.states(&task.id),
        [
            TaskState::Queued,
            TaskState::Downloading,
            TaskState::Converting,
            TaskState::Installing
        ]
    );
    let events = ctx.events.all();
    let Some(Recorded::Done(event)) = events.last() else {
        panic!("dernier événement : {events:?}")
    };
    assert_eq!(event.warnings, 1);
    assert_eq!(event.entry.file, "abc-2.plain.pk");
    // Sans `start.version` : empreinte du binaire.
    assert!(event
        .entry
        .converter
        .as_deref()
        .unwrap()
        .starts_with("sha256:"));
    let progress_phases: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Recorded::Progress(p) => p.phase.clone(),
            _ => None,
        })
        .collect();
    assert!(progress_phases.contains(&"zip".to_string()));

    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].source.download_url, task.entry.download_url);
    assert!(ctx.cache().join("library/abc-2.plain.pk").is_file());
    assert!(!ctx.cache().join("archives/abc-2.zip").exists());
    assert!(ctx.staging_is_empty());
    assert!(ctx.queue().is_empty(), "file vidée après succès");
    let recorded =
        std::fs::read_to_string(ctx.cache().join("compat-local/launcher_results.json")).unwrap();
    assert!(
        recorded.contains("\"abc\"") && recorded.contains("\"ok\": true"),
        "{recorded}"
    );
    assert_eq!(recorded_result(&ctx)["engine"], "rust");

    ctx.service.library_remove("abc".into()).await.unwrap();
    assert!(ctx.service.library_list().await.unwrap().is_empty());
    assert_eq!(
        ctx.service
            .library_remove("abc".into())
            .await
            .unwrap_err()
            .kind(),
        "not_found"
    );
}

#[tokio::test]
async fn echec_de_conversion_type() {
    let Some(py) = fake_converter() else { return };
    let logs = crate::testlog::Logs::capture();
    let ctx = Ctx::new(Some(py));
    let server = serve(zip_bytes("fail")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    let failed = ctx.finished(&task.id).await;
    assert_eq!(failed.state, TaskState::Failed);
    let error = failed.error.unwrap();
    assert_eq!(error.kind, "conversion");
    assert_eq!(error.code.as_deref(), Some("missing_asset"));
    // Message fixe par code : le nom de fichier du convertisseur n'est ni montré ni journalisé (#475).
    assert!(!error.message.contains("s1.png"), "{}", error.message);
    assert!(
        error.message.contains("absent de l'archive"),
        "{}",
        error.message
    );
    let text = logs.text();
    assert!(!text.contains("s1.png"), "{text}");
    assert!(text.contains("conversion (missing_asset)"), "{text}");
    assert!(matches!(
        ctx.events.all().last(),
        Some(Recorded::Failed(InstallFailed {
            state: TaskState::Failed,
            ..
        }))
    ));
    assert!(ctx.staging_is_empty());
    assert!(ctx.queue().is_empty());
    let recorded =
        std::fs::read_to_string(ctx.cache().join("compat-local/launcher_results.json")).unwrap();
    assert!(
        recorded.contains("\"ok\": false") && recorded.contains("missing_asset"),
        "{recorded}"
    );
}

/// Résultat de compatibilité enregistré pour `abc` (`launcher_results.json`).
fn recorded_result(ctx: &Ctx) -> serde_json::Value {
    let text =
        std::fs::read_to_string(ctx.cache().join("compat-local/launcher_results.json")).unwrap();
    serde_json::from_str::<serde_json::Value>(&text).unwrap()["stories"]["abc"].clone()
}

/// `unsupported_format` (code 7, D4) : erreur finale (#138), sans repli. Message fixe et clair
/// montré à l'utilisateur, jamais celui du convertisseur (nom de fichier de l'histoire) ni dans
/// les journaux ; aucune histoire installée ; résultat de compatibilité en échec, moteur `rust`.
#[tokio::test]
async fn format_non_pris_en_charge_erreur_finale_sans_repli() {
    let Some(py) = fake_converter() else { return };
    let logs = crate::testlog::Logs::capture();
    let ctx = Ctx::new(Some(py));
    let server = serve(zip_bytes("err:unsupported_format:7")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    let failed = ctx.finished(&task.id).await;
    assert_eq!(failed.state, TaskState::Failed, "{failed:?}");
    let error = failed.error.unwrap();
    assert_eq!(error.kind, "conversion");
    assert_eq!(error.code.as_deref(), Some("unsupported_format"));
    assert!(
        error.message.contains(convert::UNSUPPORTED_FORMAT),
        "{}",
        error.message
    );
    assert!(!error.message.contains("msg-"), "{}", error.message);
    assert!(ctx.staging_is_empty());
    assert!(!ctx.cache().join("library/abc-2.plain.pk").exists());
    assert!(ctx.service.library_list().await.unwrap().is_empty());
    let result = recorded_result(&ctx);
    assert_eq!(result["ok"], false);
    assert_eq!(result["code"], "unsupported_format");
    assert_eq!(result["engine"], "rust");
    assert_eq!(result["fallback"], serde_json::Value::Null);
    assert!(
        result["message"]
            .as_str()
            .is_some_and(|m| m == convert::UNSUPPORTED_FORMAT),
        "{result}"
    );
    let text = logs.text();
    assert!(!text.contains("msg-unsupported_format"), "{text}");
}

/// Disque plein, erreur de contenu, erreur interne : échec rendu tel quel, trace du moteur `rust`.
#[tokio::test]
async fn echecs_du_convertisseur_rendus_tels_quels() {
    let Some(py) = fake_converter() else { return };
    for (mode, code) in [
        ("err:output_error:6", "output_error"),
        ("err:archive_unreadable:3", "archive_unreadable"),
        ("err:missing_asset:5", "missing_asset"),
        ("err:internal_error:6", "internal_error"),
    ] {
        let ctx = Ctx::new(Some(py.clone()));
        let server = serve(zip_bytes(mode)).await;
        let task = ctx
            .service
            .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
            .unwrap();
        let failed = ctx.finished(&task.id).await;
        assert_eq!(failed.state, TaskState::Failed, "{mode}");
        assert_eq!(failed.error.unwrap().code.as_deref(), Some(code), "{mode}");
        assert!(ctx.staging_is_empty(), "{mode}");
        assert!(
            !ctx.cache().join("library/abc-2.plain.pk").exists(),
            "{mode}"
        );
        let result = recorded_result(&ctx);
        assert_eq!(result["ok"], false, "{mode}");
        assert_eq!(result["engine"], "rust", "{mode}");
        assert_eq!(result["fallback"], serde_json::Value::Null, "{mode}");
    }
}

/// Un ancien `settings.json` qui porte `converter_engine: "python"` (réglage caché supprimé,
/// #138) reste lisible : le champ est ignoré et la conversion se fait avec le binaire Rust.
#[tokio::test]
async fn ancien_reglage_python_utilise_le_moteur_rust() {
    let Some(py) = fake_converter() else { return };
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir_all(&config).unwrap();
    let cache = dir.path().join("cache");
    std::fs::write(
        config.join("settings.json"),
        serde_json::json!({
            "schema_version": 1,
            "converter_engine": "python",
            "cache_dir": cache.display().to_string(),
        })
        .to_string(),
    )
    .unwrap();
    // Résolution réelle des réglages ; seul le binaire (introuvable dans un `cargo test`) est
    // remplacé par le faux convertisseur.
    let real = settings_resolver(config);
    let resolver: Resolver = Arc::new(move || {
        let mut tc = real()?;
        tc.rust_converter = Some(py.clone());
        Ok(tc)
    });
    let ctx = Ctx::with_resolver(dir, resolver, Duration::from_secs(5));
    let server = serve(zip_bytes("versioned")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    assert_eq!(ctx.finished(&task.id).await.state, TaskState::Done);
    // Le faux convertisseur rend les options reçues : c'est bien la ligne de commande du Rust.
    let seen = std::fs::read_to_string(ctx.cache().join("args.txt")).unwrap();
    assert!(seen.contains("--cancel-on-stdin-eof"), "{seen}");
    assert_eq!(recorded_result(&ctx)["engine"], "rust");
}

#[tokio::test]
async fn plantage_sans_evenement_error_ne_reprend_pas_stderr() {
    let Some(py) = fake_converter() else { return };
    let logs = crate::testlog::Logs::capture();
    let ctx = Ctx::new(Some(py));
    let server = serve(zip_bytes("crash")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    let error = ctx.finished(&task.id).await.error.unwrap();
    assert_eq!(error.kind, "conversion");
    assert_eq!(error.code.as_deref(), Some("exit_6"));
    // La dernière ligne de stderr (ici un nom de fichier) n'est ni affichée ni journalisée.
    assert!(
        !error.message.contains("histoire-secrete"),
        "{}",
        error.message
    );
    let text = logs.text();
    assert!(!text.contains("histoire-secrete"), "{text}");
}

async fn annulation_pendant_la_conversion(mode: &str, grace: Duration) {
    let Some(py) = fake_converter() else { return };
    let ctx = Ctx::with_dir(tempfile::tempdir().unwrap(), Some(py), grace);
    let server = serve(zip_bytes(mode)).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    ctx.wait(&task.id, |t| t.state == TaskState::Converting)
        .await;
    // Le faux convertisseur signale qu'il est prêt (`.part` écrit, annulation armée) : on
    // attend cet événement plutôt qu'une durée.
    let cache = ctx.cache();
    let ready = || {
        std::fs::read_dir(&cache).is_ok_and(|d| {
            d.flatten()
                .any(|e| e.file_name().to_string_lossy().starts_with("ready-"))
        })
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    while !ready() {
        assert!(Instant::now() < deadline, "convertisseur jamais prêt");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let staging = cache.join("library/.staging");
    assert!(
        std::fs::read_dir(&staging).unwrap().count() > 0,
        "le .part existe avant le signal de disponibilité"
    );
    let started = Instant::now();
    ctx.service.cancel(&task.id).unwrap();
    let cancelled = ctx.finished(&task.id).await;
    assert_eq!(cancelled.state, TaskState::Cancelled);
    assert_eq!(cancelled.error.unwrap().kind, "cancelled");
    assert!(started.elapsed() < Duration::from_secs(10));
    assert!(ctx.staging_is_empty(), "temporaires nettoyés");
    assert!(!ctx.cache().join("archives/abc-2.zip").exists());
    assert!(ctx.queue().is_empty());
    assert!(ctx.service.library_list().await.unwrap().is_empty());
}

/// La fermeture de stdin suffit (le faux binaire ignore tout signal), bien avant le délai de
/// grâce de 20 s. Aucun repli : la tâche est annulée, rien n'est installé.
#[tokio::test]
async fn annulation_par_fermeture_de_stdin_pendant_la_conversion() {
    annulation_pendant_la_conversion("slow", Duration::from_secs(20)).await;
}

#[tokio::test]
async fn arret_force_apres_le_delai_de_grace() {
    annulation_pendant_la_conversion("stubborn", Duration::from_millis(500)).await;
}

#[tokio::test]
async fn une_seule_conversion_a_la_fois() {
    let Some(py) = fake_converter() else { return };
    let ctx = Ctx::new(Some(py));
    let server = serve(zip_bytes("slow")).await;
    let a = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "aaa"))
        .unwrap();
    let b = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "bbb"))
        .unwrap();
    let first = ctx
        .wait(&a.id, |_| {
            ctx.service
                .list()
                .iter()
                .any(|t| t.state == TaskState::Converting)
        })
        .await;
    let _ = first;
    // Laisse à l'autre tâche le temps de finir son téléchargement.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let converting = ctx
        .service
        .list()
        .iter()
        .filter(|t| t.state == TaskState::Converting)
        .count();
    assert_eq!(converting, 1);
    // La tâche en attente du créneau de conversion : annulée, son archive est supprimée.
    let (waiting, running) = if ctx.task(&a.id).state == TaskState::Converting {
        (&b, &a)
    } else {
        (&a, &b)
    };
    let archive = ctx
        .cache()
        .join("archives")
        .join(format!("{}-2.zip", waiting.entry.uuid));
    assert!(archive.is_file(), "archive téléchargée en attente");
    ctx.service.cancel(&waiting.id).unwrap();
    assert_eq!(ctx.finished(&waiting.id).await.state, TaskState::Cancelled);
    assert!(!archive.exists(), "archive supprimée à l'annulation");
    ctx.service.cancel(&running.id).unwrap();
    ctx.finished(&running.id).await;
}

/// Panne injectée : binaire Rust non livré. Erreur `converter_unavailable` qui nomme le binaire.
#[tokio::test]
async fn binaire_rust_absent_donne_converter_unavailable() {
    let ctx = Ctx::new(None);
    let server = serve(zip_bytes("ok")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    let error = ctx.finished(&task.id).await.error.unwrap();
    assert_eq!(error.kind, "converter_unavailable");
    assert!(
        error.message.contains(RUST_CONVERTER_EXE),
        "{}",
        error.message
    );
    assert!(!error.message.contains("Python"), "{}", error.message);
    assert!(ctx.staging_is_empty());
}

/// Panne injectée : binaire Rust détecté puis disparu avant le lancement (supprimé, mis en
/// quarantaine par un antivirus). L'échec du lancement l'emporte sur l'empreinte de repli,
/// elle aussi impossible.
#[tokio::test]
async fn binaire_rust_disparu_avant_le_lancement() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("telmi2flam").join(RUST_CONVERTER_EXE);
    let ctx = Ctx::with_dir(dir, Some(gone), Duration::from_secs(5));
    let server = serve(zip_bytes("ok")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    let error = ctx.finished(&task.id).await.error.unwrap();
    assert_eq!(error.kind, "converter_unavailable", "{error:?}");
    assert!(
        error.message.contains(RUST_CONVERTER_EXE),
        "{}",
        error.message
    );
    assert!(ctx.staging_is_empty());
    assert!(ctx.queue().is_empty());
}

#[tokio::test]
async fn erreurs_http_et_contenu_non_zip() {
    let ctx = Ctx::new(None);
    let server = MockServer::start().await;
    Mock::given(path("/absent.zip"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(path("/page.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<html>"))
        .mount(&server)
        .await;
    let a = ctx
        .service
        .start(remote(&format!("{}/absent.zip", server.uri()), "a404"))
        .unwrap();
    let b = ctx
        .service
        .start(remote(&format!("{}/page.zip", server.uri()), "html"))
        .unwrap();
    assert_eq!(ctx.finished(&a.id).await.error.unwrap().kind, "network");
    assert_eq!(
        ctx.finished(&b.id).await.error.unwrap().kind,
        "download_invalid"
    );
    assert!(!ctx.cache().join("archives/html-2.zip").exists());
}

#[tokio::test]
async fn entrees_refusees() {
    let ctx = Ctx::new(None);
    for (uuid, url, kind) in [
        ("../x", "https://h.example.test/a.zip", StoryKind::TelmiZip),
        ("CON", "https://h.example.test/a.zip", StoryKind::TelmiZip),
        ("abc", "relatif/a.zip", StoryKind::TelmiZip),
        ("abc", "https://h.example.test/a", StoryKind::FlamPlain),
    ] {
        let mut entry = remote(url, uuid);
        entry.kind = kind;
        assert_eq!(
            ctx.service.start(entry).unwrap_err().kind(),
            "validation",
            "{uuid} {url}"
        );
    }
    assert!(ctx.service.list().is_empty());
    assert_eq!(
        ctx.service.cancel("inconnue").unwrap_err().kind(),
        "not_found"
    );
}

// --- Serveur HTTP minimal : coupures, blocages, reprise ------------------------------------

/// Réponse du serveur minimal à une requête (en-tête `Range` éventuel en argument).
type Handler = Arc<dyn Fn(Option<String>, Option<String>) -> RawReply + Send + Sync>;

struct RawReply {
    head: String,
    body: Vec<u8>,
    /// Après l'envoi du corps : bloquer (sans fermer) au lieu de fermer.
    stall: bool,
}

/// Démarre un serveur HTTP/1.1 rudimentaire ; renvoie son URL et le nombre de requêtes.
async fn raw_server(handler: Handler) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let count = Arc::new(AtomicUsize::new(0));
    let counter = count.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            counter.fetch_add(1, Ordering::SeqCst);
            let handler = handler.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buf = [0u8; 1024];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buf[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request).to_string();
                let header = |name: &str| {
                    text.lines().find_map(|l| {
                        let (k, v) = l.split_once(':')?;
                        k.trim()
                            .eq_ignore_ascii_case(name)
                            .then(|| v.trim().to_string())
                    })
                };
                let reply = handler(header("range"), header("if-range"));
                let _ = socket.write_all(reply.head.as_bytes()).await;
                let _ = socket.write_all(&reply.body).await;
                let _ = socket.flush().await;
                if reply.stall {
                    tokio::time::sleep(Duration::from_secs(120)).await;
                }
            });
        }
    });
    (format!("http://{addr}/a.zip"), count)
}

#[tokio::test]
async fn annulation_pendant_le_telechargement() {
    let ctx = Ctx::new(None);
    let (url, _) = raw_server(Arc::new(|_, _| RawReply {
        head: "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nETag: \"e\"\r\n\r\n".into(),
        body: vec![b'x'; 400],
        stall: true,
    }))
    .await;
    let task = ctx.service.start(remote(&url, "abc")).unwrap();
    let part = ctx.cache().join("archives/abc-2.zip.part");
    ctx.wait(&task.id, |_| {
        std::fs::metadata(&part).is_ok_and(|m| m.len() >= 400)
    })
    .await;
    // La file persistée garde le validateur pour une reprise.
    ctx.service.flush_queue().await;
    assert_eq!(ctx.queue()[0].validator.etag.as_deref(), Some("\"e\""));
    ctx.service.cancel(&task.id).unwrap();
    let cancelled = ctx.finished(&task.id).await;
    assert_eq!(cancelled.state, TaskState::Cancelled);
    assert!(!part.exists(), "le .part est supprimé à l'annulation");
    assert!(ctx.queue().is_empty());
}

#[tokio::test]
async fn connexion_coupee_taille_incomplete() {
    let ctx = Ctx::new(None);
    let (url, _) = raw_server(Arc::new(|_, _| RawReply {
        head: "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nETag: \"e\"\r\n\r\n".into(),
        body: zip_bytes("ok"),
        stall: false,
    }))
    .await;
    let task = ctx.service.start(remote(&url, "abc")).unwrap();
    let failed = ctx.finished(&task.id).await;
    let kind = failed.error.unwrap().kind;
    assert!(kind == "download_invalid" || kind == "network", "{kind}");
}

#[tokio::test]
async fn reprise_apres_redemarrage() {
    let Some(py) = fake_converter() else { return };
    let full = zip_padded("ok", 3000);
    let dir = tempfile::tempdir().unwrap();
    // État laissé par une session interrompue : file persistée et `.part` de 1000 octets.
    let archives = dir.path().join("cache/archives");
    std::fs::create_dir_all(&archives).unwrap();
    std::fs::write(archives.join("abc-2.zip.part"), &full[..1000]).unwrap();

    let ranges = Arc::new(Mutex::new(Vec::new()));
    let seen = ranges.clone();
    let body = full.clone();
    let (url, _) = raw_server(Arc::new(move |range, if_range| {
        lock(&seen).push((range.clone(), if_range.clone()));
        match (range, if_range.as_deref()) {
            (Some(r), Some("\"v1\"")) => {
                let start: usize = r
                    .trim_start_matches("bytes=")
                    .trim_end_matches('-')
                    .parse()
                    .unwrap();
                RawReply {
                    head: format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-{}/{}\r\nETag: \"v1\"\r\n\r\n",
                        body.len() - start,
                        body.len() - 1,
                        body.len()
                    ),
                    body: body[start..].to_vec(),
                    stall: false,
                }
            }
            _ => RawReply {
                head: format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"v1\"\r\n\r\n",
                    body.len()
                ),
                body: body.clone(),
                stall: false,
            },
        }
    }))
    .await;
    let queued = QueueFile {
        schema_version: QUEUE_SCHEMA,
        tasks: vec![QueuedTask {
            id: TASK_ID.into(),
            entry: remote(&url, "abc"),
            created_at: 1,
            pack: None,
            validator: Validator {
                etag: Some("\"v1\"".into()),
                last_modified: None,
            },
        }],
    };
    storage::write_json(&dir.path().join("config").join(QUEUE_FILE), &queued).unwrap();

    let ctx = Ctx::with_dir(dir, Some(py), Duration::from_secs(5));
    let fresh = remote(&url, "abc");
    ctx.service
        .restore(|_, _| {
            let fresh = fresh.clone();
            async move { Ok(fresh) }
        })
        .await;
    let done = ctx.finished(TASK_ID).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    assert_eq!(
        lock(&ranges).first().cloned(),
        Some((Some("bytes=1000-".into()), Some("\"v1\"".into())))
    );
    let progress: Vec<u64> = ctx
        .events
        .all()
        .iter()
        .filter_map(|e| match e {
            Recorded::Progress(p) => p.received,
            _ => None,
        })
        .collect();
    assert_eq!(progress.first(), Some(&1000), "reprise à 1000 octets");
    assert_eq!(progress.last(), Some(&(full.len() as u64)));
    assert_eq!(ctx.service.library_list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn reprise_refusee_recommence_depuis_zero() {
    let ctx = Ctx::new(None);
    let dir_archives = ctx.cache().join("archives");
    std::fs::create_dir_all(&dir_archives).unwrap();
    std::fs::write(dir_archives.join("abc-2.plain.pk.part"), b"vieux contenu").unwrap();
    // Paquet FLAM complet servi en 200 malgré le Range (fichier changé côté serveur).
    let pk = {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("h.plain");
        std::fs::create_dir_all(&plain).unwrap();
        std::fs::write(plain.join("info.plain"), "Titre\n").unwrap();
        std::fs::write(plain.join("uuid.bin"), [1u8; 16]).unwrap();
        std::fs::write(plain.join("main.lua"), "-- x").unwrap();
        let out = tmp.path().join("h.plain.pk");
        pack::pack_plain_dir(&plain, &out).unwrap();
        std::fs::read(out).unwrap()
    };
    let (url, _) = raw_server(Arc::new(move |_, _| RawReply {
        head: format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"v2\"\r\n\r\n",
            pk.len()
        ),
        body: pk.clone(),
        stall: false,
    }))
    .await;
    let mut entry = remote(&url, "abc");
    entry.kind = StoryKind::FlamPk;
    // Sans validateur, le `.part` existant ne peut pas être repris : il est réécrit.
    {
        let task = ctx.service.start(entry).unwrap();
        let done = ctx.finished(&task.id).await;
        assert_eq!(done.state, TaskState::Done, "{done:?}");
    }
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library[0].converter, None);
    assert!(!dir_archives.join("abc-2.plain.pk").exists());
}

#[tokio::test]
async fn sources_locales_plain_et_plain_pk() {
    let ctx = Ctx::new(None);
    let src = ctx.dir.path().join("src");
    let plain = src.join("h.plain");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("info.plain"), "Titre\n").unwrap();
    std::fs::write(plain.join("uuid.bin"), [1u8; 16]).unwrap();
    std::fs::write(plain.join("main.lua"), "-- x").unwrap();
    let pk = src.join("h.plain.pk");
    pack::pack_plain_dir(&plain, &pk).unwrap();

    let uuid = "01".repeat(16);
    let mut from_dir = story(&uuid, None);
    from_dir.kind = StoryKind::FlamPlain;
    from_dir.download_url = plain.to_string_lossy().into_owned();
    let t = ctx.service.start(from_dir).unwrap();
    assert_eq!(ctx.finished(&t.id).await.state, TaskState::Done);

    // Même uuid en .plain.pk : remplace l'installation précédente (une par uuid).
    let mut from_pk = story(&uuid, None);
    from_pk.kind = StoryKind::FlamPk;
    from_pk.download_url = pk.to_string_lossy().into_owned();
    let t = ctx.service.start(from_pk).unwrap();
    assert_eq!(ctx.finished(&t.id).await.state, TaskState::Done);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library.len(), 1);
    assert_eq!(library[0].source.kind, StoryKind::FlamPk);
    assert_eq!(library[0].file, format!("{uuid}-0.plain.pk"));
    assert!(pk.is_file(), "la source locale n'est pas modifiée");
    pack::check_plain_pk(&ctx.cache().join("library").join(&library[0].file)).unwrap();
}

/// #288 : une source dossier reconstruite à version égale est signalée, et « Mettre à jour »
/// (nouvelle installation depuis la source) remplace la copie sans toucher aux sauvegardes.
#[tokio::test]
async fn source_dossier_modifiee_a_version_egale_puis_mise_a_jour() {
    use crate::library_source::{self, SourceState};
    use crate::sources::Source;

    let ctx = Ctx::new(None);
    let src = ctx.dir.path().join("src");
    let plain = src.join("capi.plain");
    let build = |thumb: &[u8]| {
        std::fs::create_dir_all(plain.join("img")).unwrap();
        std::fs::write(plain.join("info.plain"), "Capi\n").unwrap();
        std::fs::write(plain.join("uuid.bin"), [1u8; 16]).unwrap();
        std::fs::write(plain.join("main.lua"), "-- x").unwrap();
        std::fs::write(plain.join("img/thumbnail.lif"), thumb).unwrap();
        let pk = src.join("capi.plain.pk");
        pack::pack_plain_dir(&plain, &pk).unwrap();
        std::fs::remove_dir_all(&plain).unwrap();
        pk
    };
    let pk = build(b"128x96");

    let uuid = "01".repeat(16);
    let mut entry = story(&uuid, Some(0));
    entry.source_id = "dossier".into();
    entry.kind = StoryKind::FlamPk;
    entry.download_url = pk.to_string_lossy().into_owned();
    let sources = || {
        vec![Source::Folder {
            id: "dossier".into(),
            name: "Jeux".into(),
            path: src.to_string_lossy().into_owned(),
        }]
    };
    let state = || library_source::statuses(&ctx.cache(), sources()).unwrap()[0].state;

    let t = ctx.service.start(entry.clone()).unwrap();
    assert_eq!(ctx.finished(&t.id).await.state, TaskState::Done);
    let installed = ctx.service.library_list().await.unwrap();
    assert!(installed[0].source_stamp.is_some(), "empreinte enregistrée");
    assert_eq!(state(), SourceState::Unchanged);

    // Progression de l'enfant (hors cache) : jamais touchée par une mise à jour.
    let saves = ctx.dir.path().join("config/saves/0123456789abcdef");
    std::fs::create_dir_all(&saves).unwrap();
    std::fs::write(saves.join("save.bin"), b"niveau 3").unwrap();

    // Reconstruction : même uuid, même version, nouvelle vignette.
    build(b"104x136");
    assert_eq!(state(), SourceState::Changed);
    let copy = ctx.cache().join("library").join(&installed[0].file);
    assert_ne!(std::fs::read(&copy).unwrap(), std::fs::read(&pk).unwrap());

    // « Mettre à jour » : réinstallation depuis la source, au même fichier.
    let t = ctx.service.start(entry).unwrap();
    assert_eq!(ctx.finished(&t.id).await.state, TaskState::Done);
    let updated = ctx.service.library_list().await.unwrap();
    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].file, installed[0].file);
    assert_ne!(updated[0].source_stamp, installed[0].source_stamp);
    assert_eq!(std::fs::read(&copy).unwrap(), std::fs::read(&pk).unwrap());
    assert_eq!(state(), SourceState::Unchanged);
    assert_eq!(std::fs::read(saves.join("save.bin")).unwrap(), b"niveau 3");
    assert!(ctx.staging_is_empty());

    // Source supprimée : la copie est conservée.
    std::fs::remove_file(&pk).unwrap();
    assert_eq!(state(), SourceState::Missing);
    assert_eq!(ctx.service.library_list().await.unwrap().len(), 1);
    assert!(copy.is_file());
}

#[test]
fn uuid_attendu_verifie() {
    let entry = story("abc", Some(1));
    assert!(check_expected_uuid(&entry, None).is_ok());
    assert!(check_expected_uuid(&entry, Some("abc")).is_ok());
    assert_eq!(
        check_expected_uuid(&entry, Some("autre"))
            .unwrap_err()
            .kind(),
        "not_found"
    );
}

#[tokio::test]
async fn le_vidage_du_cache_attend_les_taches() {
    let ctx = Ctx::new(None);
    let (url, _) = raw_server(Arc::new(|_, _| RawReply {
        head: "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n".into(),
        body: vec![b'x'; 10],
        stall: true,
    }))
    .await;
    let task = ctx.service.start(remote(&url, "abc")).unwrap();
    ctx.wait(&task.id, |t| t.state == TaskState::Downloading)
        .await;
    let blocked = tokio::time::timeout(Duration::from_millis(200), ctx.gate.write()).await;
    assert!(blocked.is_err(), "le vidage doit attendre la tâche");
    ctx.service.cancel(&task.id).unwrap();
    ctx.finished(&task.id).await;
    let _clear = tokio::time::timeout(Duration::from_secs(5), ctx.gate.write())
        .await
        .expect("portillon libéré");
}

#[test]
fn limite_de_telechargements_modifiable() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let limiter = Arc::new(Limiter::new(1));
        let a = limiter.acquire().await;
        let waiting = tokio::time::timeout(Duration::from_millis(50), limiter.acquire()).await;
        assert!(waiting.is_err());
        limiter.set_limit(2);
        let b = tokio::time::timeout(Duration::from_millis(50), limiter.acquire())
            .await
            .expect("limite relevée");
        drop((a, b));
        assert_eq!(lock(&limiter.state).0, 0);
    });
}

#[test]
fn empreinte_du_convertisseur() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join(RUST_CONVERTER_EXE);
    std::fs::write(&exe, b"MZ").unwrap();
    let fingerprint = converter_fingerprint(&exe).unwrap();
    assert!(fingerprint.starts_with("sha256:"), "{fingerprint}");
    assert_eq!(fingerprint.len(), "sha256:".len() + 12);
    assert_eq!(converter_fingerprint(&exe).unwrap(), fingerprint);
    assert!(converter_fingerprint(&dir.path().join("absent.exe")).is_err());
}

#[test]
fn detection_du_binaire_rust() {
    let dir = tempfile::tempdir().unwrap();
    let exe_dir = dir.path().join("target/debug");
    std::fs::create_dir_all(&exe_dir).unwrap();
    let env = |search| Environment {
        exe_dir: Some(exe_dir.clone()),
        default_cache_dir: dir.path().join("c"),
        search_repo_build: search,
        protected_dirs: Vec::new(),
    };
    assert_eq!(detect_rust_converter(&env(true)), None);
    // Développement : binaire du même `target/` que le launcher, seulement en recherche dépôt.
    let built = exe_dir.join(RUST_CONVERTER_EXE);
    std::fs::write(&built, b"MZ").unwrap();
    assert_eq!(detect_rust_converter(&env(true)), Some(built.clone()));
    assert_eq!(detect_rust_converter(&env(false)), None);
    // Un dossier du même nom n'est pas un binaire.
    std::fs::remove_file(&built).unwrap();
    std::fs::create_dir(&built).unwrap();
    assert_eq!(detect_rust_converter(&env(true)), None);
    // Application installée : `telmi2flam/telmi2flam.exe`, prioritaire.
    let bundled = exe_dir.join("telmi2flam").join(RUST_CONVERTER_EXE);
    std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
    std::fs::write(&bundled, b"MZ").unwrap();
    assert_eq!(detect_rust_converter(&env(false)), Some(bundled.clone()));
    assert_eq!(detect_rust_converter(&env(true)), Some(bundled));
    let none = Environment {
        exe_dir: None,
        ..env(true)
    };
    assert_eq!(detect_rust_converter(&none), None);
}

#[test]
fn file_corrompue_ignoree() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(QUEUE_FILE);
    std::fs::write(&path, b"{oops").unwrap();
    assert!(load_queue(&path).is_empty());
}

#[test]
fn erreur_de_tache_serialisee() {
    let err = AppError::Conversion {
        code: "missing_asset".into(),
        message: "m".into(),
    };
    assert_eq!(
        serde_json::to_value(TaskError::from(&err)).unwrap(),
        serde_json::json!({
            "kind": "conversion",
            "message": "conversion impossible (missing_asset) : m",
            "code": "missing_asset"
        })
    );
    assert_eq!(
        serde_json::to_value(TaskState::Converting).unwrap(),
        "converting"
    );
}

/// Installation réelle (ignorée par défaut) : archive Telmi locale servie en HTTP, conversion
/// par le vrai binaire `telmi2flam.exe`, puis une seconde installation annulée en pleine
/// conversion. `FLAM_TELMI_ZIP=<archive.zip> FLAM_TEST_RUST_CONVERTER=<telmi2flam.exe>
/// cargo test installation_reelle -- --ignored`
#[tokio::test]
#[ignore = "nécessite FLAM_TELMI_ZIP (archive Telmi hors dépôt) et FLAM_TEST_RUST_CONVERTER"]
async fn installation_reelle() {
    let zip = PathBuf::from(std::env::var("FLAM_TELMI_ZIP").expect("FLAM_TELMI_ZIP"));
    let exe = PathBuf::from(
        std::env::var_os("FLAM_TEST_RUST_CONVERTER").expect("FLAM_TEST_RUST_CONVERTER"),
    );
    let ctx = Ctx::with_dir(tempfile::tempdir().unwrap(), Some(exe), GRACE);
    let server = serve(std::fs::read(&zip).unwrap()).await;
    let url = format!("{}/a.zip", server.uri());

    let started = Instant::now();
    let task = ctx.service.start(remote(&url, "reel")).unwrap();
    let done = ctx.finished(&task.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    let entry = &ctx.service.library_list().await.unwrap()[0];
    let installed = ctx.cache().join("library").join(&entry.file);
    pack::check_plain_pk(&installed).unwrap();
    eprintln!(
        "installé : {} ({} octets, {}) en {:?}, états {:?}",
        entry.file,
        entry.size,
        entry.converter.as_deref().unwrap_or("-"),
        started.elapsed(),
        ctx.events.states(&task.id)
    );

    let task = ctx.service.start(remote(&url, "annule")).unwrap();
    ctx.wait(&task.id, |t| t.state == TaskState::Converting)
        .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let asked = Instant::now();
    ctx.service.cancel(&task.id).unwrap();
    let cancelled = ctx.finished(&task.id).await;
    eprintln!("annulation en {:?}", asked.elapsed());
    assert_eq!(cancelled.state, TaskState::Cancelled);
    assert!(
        asked.elapsed() < GRACE,
        "arrêt par fermeture de stdin, pas forcé"
    );
    assert!(ctx.staging_is_empty());
    assert_eq!(ctx.service.library_list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn reprise_ignore_les_identifiants_malveillants_et_re_resout_les_entrees() {
    let dir = tempfile::tempdir().unwrap();
    let outside = dir.path().join("dehors.plain.pk");
    std::fs::write(&outside, b"precieux").unwrap();
    let evil = remote("https://evil.example.test/x.zip", "abc");
    let mut gone = remote("https://h.example.test/g.zip", "gone");
    gone.entry_id = "src:gone".into();
    let tasks = [
        "../../dehors",
        r"C:\Windows\x",
        "/etc/x",
        "T1",
        "0B8F3C1E-4A5D-4E6F-9A7B-8C9D0E1F2A3B",
    ]
    .iter()
    .map(|id| QueuedTask {
        id: (*id).into(),
        entry: evil.clone(),
        created_at: 1,
        pack: None,
        validator: Validator::default(),
    })
    .chain([
        QueuedTask {
            id: "11111111-2222-4333-8444-555555555555".into(),
            entry: gone,
            created_at: 1,
            pack: None,
            validator: Validator::default(),
        },
        QueuedTask {
            id: TASK_ID.into(),
            entry: evil.clone(),
            created_at: 1,
            pack: None,
            validator: Validator {
                etag: Some("\"e\"".into()),
                last_modified: None,
            },
        },
    ])
    .collect();
    storage::write_json(
        &dir.path().join("config").join(QUEUE_FILE),
        &QueueFile {
            schema_version: QUEUE_SCHEMA,
            tasks,
        },
    )
    .unwrap();
    let server = serve(zip_bytes("ok")).await;
    let fresh = remote(&format!("{}/a.zip", server.uri()), "abc");
    let ctx = Ctx::with_dir(dir, None, GRACE);
    let looked_up = Arc::new(Mutex::new(Vec::new()));
    let seen = looked_up.clone();
    ctx.service
        .restore(|source_id, entry_id| {
            lock(&seen).push(entry_id.clone());
            let fresh = fresh.clone();
            async move {
                match entry_id.as_str() {
                    "src:abc" => Ok(fresh),
                    _ => Err(AppError::NotFound(format!("{source_id}:{entry_id}"))),
                }
            }
        })
        .await;
    // Seuls les identifiants canoniques sont considérés ; l'entrée disparue est abandonnée.
    assert_eq!(*lock(&looked_up), ["src:gone", "src:abc"]);
    let list = ctx.service.list();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, TASK_ID);
    // L'entrée vient de la source, pas du fichier : le lien persisté n'est jamais utilisé.
    assert_eq!(list[0].entry.download_url, fresh.download_url);
    let task = ctx.finished(TASK_ID).await;
    assert_eq!(task.error.unwrap().kind, "converter_unavailable");
    assert!(outside.is_file());
    assert!(ctx.queue().is_empty());
}

#[test]
fn identifiants_et_chemins_derives() {
    assert!(is_task_id(TASK_ID));
    assert!(is_task_id(&uuid::Uuid::new_v4().to_string()));
    for bad in [
        "",
        "t1",
        "../x",
        "0B8F3C1E-4A5D-4E6F-9A7B-8C9D0E1F2A3B",
        "0b8f3c1e4a5d4e6f9a7b8c9d0e1f2a3b",
    ] {
        assert!(!is_task_id(bad), "{bad}");
    }
    let dir = Path::new("/c/library/.staging");
    assert!(child_of(dir, "a.plain.pk").is_ok());
    for bad in ["../a.plain.pk", "x/a", r"x\a", "/abs", "C:a", "..", ""] {
        assert_eq!(
            child_of(dir, bad).unwrap_err().kind(),
            "validation",
            "{bad}"
        );
    }
}

#[tokio::test]
async fn sans_content_length_une_archive_tronquee_est_refusee() {
    let ctx = Ctx::new(None);
    let mut truncated = zip_padded("ok", 2000);
    truncated.truncate(1500);
    let (url, _) = raw_server(Arc::new(move |_, _| RawReply {
        head: "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".into(),
        body: truncated.clone(),
        stall: false,
    }))
    .await;
    let task = ctx.service.start(remote(&url, "abc")).unwrap();
    let error = ctx.finished(&task.id).await.error.unwrap();
    assert_eq!(error.kind, "download_invalid", "{error:?}");
    assert!(!ctx.cache().join("archives/abc-2.zip").exists());
}

// --- Suivis de revue (#62) ---------------------------------------------------------------------

/// `.plain.pk` valide dont le titre (donc le contenu) dépend de `title`.
fn plain_pk(title: &str) -> Vec<u8> {
    let tmp = tempfile::tempdir().unwrap();
    let plain = tmp.path().join("h.plain");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("info.plain"), format!("{title}\n")).unwrap();
    std::fs::write(plain.join("uuid.bin"), [1u8; 16]).unwrap();
    std::fs::write(plain.join("main.lua"), "-- x").unwrap();
    let out = tmp.path().join("h.plain.pk");
    pack::pack_plain_dir(&plain, &out).unwrap();
    std::fs::read(out).unwrap()
}

fn pk_entry(url: &str) -> StoryEntry {
    let mut entry = remote(url, "abc");
    entry.kind = StoryKind::FlamPk;
    entry
}

#[tokio::test]
async fn version_du_convertisseur_lue_dans_l_evenement_start() {
    let Some(py) = fake_converter() else { return };
    let ctx = Ctx::new(Some(py));
    let server = serve(zip_bytes("versioned")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    assert_eq!(ctx.finished(&task.id).await.state, TaskState::Done);
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(library[0].converter.as_deref(), Some("9.9.9-test"));
}

#[cfg(windows)]
#[tokio::test]
async fn arret_force_termine_aussi_les_sous_processus() {
    let Some(py) = fake_converter() else { return };
    let ctx = Ctx::with_dir(
        tempfile::tempdir().unwrap(),
        Some(py),
        Duration::from_millis(500),
    );
    let server = serve(zip_bytes("tree")).await;
    let task = ctx
        .service
        .start(remote(&format!("{}/a.zip", server.uri()), "abc"))
        .unwrap();
    ctx.wait(&task.id, |t| t.state == TaskState::Converting)
        .await;
    let gpid_file = ctx
        .cache()
        .join("library/.staging")
        .join(format!("{}.plain.pk.gpid", task.id));
    // Le sous-processus n'existe qu'après le démarrage de deux interpréteurs Python (le
    // convertisseur, puis son enfant) : sous forte charge (CI Windows, antivirus) cela dépasse
    // largement 10 s. On attend donc le PID, valide et vivant, avec la même borne que `wait`
    // (30 s), et on reporte l'état de la tâche pour diagnostiquer un éventuel échec.
    let deadline = Instant::now() + Duration::from_secs(30);
    let pid = loop {
        let read = std::fs::read_to_string(&gpid_file).unwrap_or_default();
        if let Ok(pid) = read.trim().parse::<u32>() {
            break pid.to_string();
        }
        assert!(
            Instant::now() < deadline,
            "pas de sous-processus : {:?}",
            ctx.task(&task.id)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let alive = |pid: &str| {
        // CSV : le PID est la 2e colonne ; une simple recherche de sous-chaîne confondrait
        // `123` avec `12 345 Ko` (colonne mémoire).
        let out = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH", "/FO", "CSV"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).lines().any(|l| {
            l.split(',')
                .nth(1)
                .is_some_and(|c| c.trim_matches('"') == pid)
        })
    };
    // Le PID peut avoir été lu avant que `tasklist` ne voie le processus : attente bornée.
    let up = Instant::now() + Duration::from_secs(10);
    while !alive(&pid) {
        assert!(
            Instant::now() < up,
            "le sous-processus {pid} tourne avant l'annulation"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    ctx.service.cancel(&task.id).unwrap();
    assert_eq!(ctx.finished(&task.id).await.state, TaskState::Cancelled);
    let deadline = Instant::now() + Duration::from_secs(10);
    while alive(&pid) {
        assert!(Instant::now() < deadline, "sous-processus {pid} vivant");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn relance_apres_coupure_reprend_le_part() {
    let ctx = Ctx::new(None);
    let pk = plain_pk("Titre");
    let half = pk.len() / 2;
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let seen = ranges.clone();
    let body = pk.clone();
    let (url, _) = raw_server(Arc::new(move |range, if_range| {
        lock(&seen).push((range.clone(), if_range));
        match range {
            None => RawReply {
                head: format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nETag: \"e\"\r\nConnection: close\r\n\r\n",
                    body.len()
                ),
                body: body[..half].to_vec(),
                stall: false,
            },
            Some(_) => RawReply {
                head: format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {half}-{}/{}\r\nETag: \"e\"\r\n\r\n",
                    body.len() - half,
                    body.len() - 1,
                    body.len()
                ),
                body: body[half..].to_vec(),
                stall: false,
            },
        }
    }))
    .await;
    let first = ctx.service.start(pk_entry(&url)).unwrap();
    let failed = ctx.finished(&first.id).await;
    assert_eq!(failed.state, TaskState::Failed, "{failed:?}");
    assert!(ctx.cache().join("archives/abc-2.plain.pk.part").is_file());
    // Une nouvelle demande (nouvelle tâche, sans validateur) reprend le `.part`.
    let second = ctx.service.start(pk_entry(&url)).unwrap();
    assert_ne!(second.id, first.id);
    let done = ctx.finished(&second.id).await;
    assert_eq!(done.state, TaskState::Done, "{done:?}");
    assert_eq!(
        lock(&ranges).last().cloned(),
        Some((Some(format!("bytes={half}-")), Some("\"e\"".into())))
    );
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(
        std::fs::read(ctx.cache().join("library").join(&library[0].file)).unwrap(),
        pk
    );
}

/// Dépose `bytes` comme archive déjà téléchargée, mémorisée pour le lien `url`.
fn preplace_archive(ctx: &Ctx, bytes: &[u8], url: &str) {
    let dir = ctx.cache().join("archives");
    std::fs::create_dir_all(&dir).unwrap();
    let dest = dir.join("abc-2.plain.pk");
    std::fs::write(&dest, bytes).unwrap();
    storage::write_json(
        &meta_path(&dest),
        &ArchiveMeta {
            url: url.to_string(),
            validator: Validator::default(),
            size: Some(bytes.len() as u64),
        },
    )
    .unwrap();
}

#[tokio::test]
async fn archive_d_un_autre_lien_non_reutilisee() {
    let ctx = Ctx::new(None);
    let fresh = plain_pk("Nouveau");
    let body = fresh.clone();
    let (url, count) = raw_server(Arc::new(move |_, _| RawReply {
        head: format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()),
        body: body.clone(),
        stall: false,
    }))
    .await;
    preplace_archive(&ctx, &plain_pk("Ancien"), "https://autre.example.test/x.pk");
    let task = ctx.service.start(pk_entry(&url)).unwrap();
    assert_eq!(ctx.finished(&task.id).await.state, TaskState::Done);
    assert_eq!(count.load(Ordering::SeqCst), 1, "archive retéléchargée");
    let library = ctx.service.library_list().await.unwrap();
    let installed = std::fs::read(ctx.cache().join("library").join(&library[0].file)).unwrap();
    assert_eq!(installed, fresh);
}

#[tokio::test]
async fn archive_du_meme_lien_reutilisee_sinon_refusee() {
    let ctx = Ctx::new(None);
    let (url, count) = raw_server(Arc::new(|_, _| RawReply {
        head: "HTTP/1.1 500 Oops\r\nContent-Length: 0\r\n\r\n".into(),
        body: Vec::new(),
        stall: false,
    }))
    .await;
    let kept = plain_pk("Conservee");
    preplace_archive(&ctx, &kept, &url);
    let task = ctx.service.start(pk_entry(&url)).unwrap();
    assert_eq!(ctx.finished(&task.id).await.state, TaskState::Done);
    assert_eq!(count.load(Ordering::SeqCst), 0, "aucune requête");
    let library = ctx.service.library_list().await.unwrap();
    assert_eq!(
        std::fs::read(ctx.cache().join("library").join(&library[0].file)).unwrap(),
        kept
    );
    // Sans mémoire du lien, ou avec une taille différente : jamais réutilisée (donc
    // retéléchargée, ce que le serveur de test refuse).
    for forget_meta in [true, false] {
        let dest = ctx.cache().join("archives/abc-2.plain.pk");
        preplace_archive(&ctx, &kept, &url);
        if forget_meta {
            std::fs::remove_file(meta_path(&dest)).unwrap();
        } else {
            std::fs::write(&dest, [&kept[..], b"+"].concat()).unwrap();
        }
        let task = ctx.service.start(pk_entry(&url)).unwrap();
        let failed = ctx.finished(&task.id).await;
        assert_eq!(failed.state, TaskState::Failed, "{failed:?}");
        assert_eq!(failed.error.unwrap().kind, "network");
    }
}

#[tokio::test]
async fn ecriture_de_file_ignore_une_capture_perimee() {
    let ctx = Ctx::new(None);
    let newer = QueueSnapshot {
        seq: 2,
        bytes: b"deux".to_vec(),
    };
    let older = QueueSnapshot {
        seq: 1,
        bytes: b"un".to_vec(),
    };
    InstallService::write_queue_blocking(&ctx.service.0, &newer);
    InstallService::write_queue_blocking(&ctx.service.0, &older);
    assert_eq!(
        std::fs::read(ctx.dir.path().join("config").join(QUEUE_FILE)).unwrap(),
        b"deux"
    );
}

#[tokio::test]
async fn capture_recente_en_echec_n_autorise_pas_une_plus_ancienne() {
    let ctx = Ctx::new(None);
    let path = ctx.dir.path().join("config").join(QUEUE_FILE);
    std::fs::create_dir_all(&path).unwrap();
    let newer = QueueSnapshot {
        seq: 2,
        bytes: b"deux".to_vec(),
    };
    InstallService::write_queue_blocking(&ctx.service.0, &newer);
    std::fs::remove_dir_all(&path).unwrap();
    let older = QueueSnapshot {
        seq: 1,
        bytes: b"un".to_vec(),
    };
    InstallService::write_queue_blocking(&ctx.service.0, &older);
    assert!(!path.exists(), "la capture ancienne ne doit pas s'écrire");
}

#[test]
fn memoire_d_archive_bornee_et_revalidee() {
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("abc-2.plain.pk");
    let meta = meta_path(&dest);
    let write = |etag: &str| {
        let json = serde_json::json!({
            "url": "https://h.example.test/a.zip",
            "validator": { "etag": etag, "last_modified": null },
            "size": 3
        });
        std::fs::write(&meta, json.to_string()).unwrap();
    };
    write("\"ok\"");
    let read = read_archive_meta(&meta).unwrap();
    assert_eq!(read.validator.etag.as_deref(), Some("\"ok\""));
    // ETag invalide : validateur vide, lien conservé.
    for bad in ["a\r\nb", "é", &"x".repeat(300)] {
        write(bad);
        let read = read_archive_meta(&meta).unwrap();
        assert_eq!(read.validator, Validator::default(), "{bad:?}");
        assert_eq!(read.url, "https://h.example.test/a.zip");
    }
    // Fichier trop gros : ignoré.
    std::fs::write(&meta, vec![b' '; 5000]).unwrap();
    assert!(read_archive_meta(&meta).is_none());
}

/// Installe `uuid` directement dans la bibliothèque du contexte.
fn install_directly(ctx: &Ctx, uuid: &str, bytes: &[u8]) {
    let library = Library::new(&ctx.cache());
    let staged = library
        .staging_dir()
        .unwrap()
        .join(format!("{uuid}.plain.pk"));
    std::fs::write(&staged, bytes).unwrap();
    library
        .install(&staged, &story(uuid, Some(1)), None, 1)
        .unwrap();
}

#[test]
fn library_remove_with_purge_puis_retire_et_inconnu_est_not_found_avant_purge() {
    let ctx = Ctx::new(None);
    install_directly(&ctx, "abc", b"PK");
    let purged = std::sync::atomic::AtomicBool::new(false);
    // Inconnu : `not_found` avant toute purge.
    let err = ctx
        .service
        .library_remove_with("inconnu", || {
            purged.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(err.kind(), "not_found");
    assert!(
        !purged.load(Ordering::SeqCst),
        "aucune purge pour un inconnu"
    );
    // Purge en échec : l'histoire reste installée (à retenter).
    let err = ctx
        .service
        .library_remove_with("abc", || {
            Err(AppError::Validation("purge impossible".into()))
        })
        .unwrap_err();
    assert_eq!(err.kind(), "validation");
    assert!(ctx.cache().join("library/abc-1.plain.pk").is_file());
    // Réussite : la purge a lieu, puis le fichier et l'entrée disparaissent.
    ctx.service
        .library_remove_with("abc", || {
            purged.store(true, Ordering::SeqCst);
            assert!(ctx.cache().join("library/abc-1.plain.pk").is_file());
            Ok(())
        })
        .unwrap();
    assert!(purged.load(Ordering::SeqCst));
    assert!(!ctx.cache().join("library/abc-1.plain.pk").exists());
    assert!(Library::new(&ctx.cache()).list().unwrap().is_empty());
}

#[test]
fn library_remove_with_renvoie_vite_une_erreur_si_une_autre_instance_tient_le_verrou() {
    let ctx = Ctx::new(None);
    install_directly(&ctx, "abc", b"PK");
    let other = Library::new(&ctx.cache());
    let _held = other.lock_file(true).unwrap().expect("verrou");
    let purged = std::sync::atomic::AtomicBool::new(false);
    let started = Instant::now();
    let err = ctx
        .service
        .library_remove_within("abc", Duration::from_millis(100), || {
            purged.store(true, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(err.kind(), "io");
    assert!(
        err.to_string().contains("utilisée par une autre instance"),
        "{err}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert!(!purged.load(Ordering::SeqCst), "rien n'est purgé");
    assert!(ctx.cache().join("library/abc-1.plain.pk").is_file());
}

#[tokio::test]
async fn library_list_ne_se_fige_pas_quand_une_autre_instance_tient_le_verrou() {
    let ctx = Ctx::new(None);
    install_directly(&ctx, "abc", b"PK");
    let other = Library::new(&ctx.cache());
    let _held = other.lock_file(true).unwrap().expect("verrou");
    let started = Instant::now();
    let list = ctx.service.library_list().await.unwrap();
    assert_eq!(list.len(), 1);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "lecture sans attendre le verrou"
    );
}

#[path = "podcast_tests.rs"]
mod pack_tests;
