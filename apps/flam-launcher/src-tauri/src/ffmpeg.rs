//! Lancement du ffmpeg LGPL embarqué (#209, épopée podcasts #207).
//!
//! ffmpeg et ffprobe sont des programmes externes statiques (`scripts/build-ffmpeg.ps1`) : jamais
//! liés au launcher. Ce module :
//!
//! - **localise** le dossier embarqué (`settings::ffmpeg_path`, jamais le `PATH`, dont la licence
//!   est inconnue) ;
//! - **vérifie l'empreinte SHA-256** de l'exécutable avant chaque lancement, d'après le fichier
//!   `ffmpeg.sha256` écrit à côté par le script de build (format de `sha256sum`) : un binaire
//!   absent du fichier, modifié ou tronqué est refusé. Le fichier est livré avec le binaire : il
//!   garde contre une copie corrompue ou un remplacement partiel, pas contre un attaquant qui
//!   peut écrire dans le dossier d'installation (qui peut aussi remplacer le launcher) ;
//! - **lance sans shell**, avec des arguments fixes fournis par l'appelant (un seul argument par
//!   élément, jamais une ligne à découper), sans fenêtre de console, stdin fermé, dans un
//!   environnement réduit à l'essentiel ;
//! - **lit la sortie** : stdout borné (le début), stderr borné (la fin) ;
//! - gère l'**annulation** (`watch`, comme les téléchargements et la conversion) et un **délai
//!   maximal** : dans les deux cas le processus est tué et attendu.
//!
//! Limites assumées (#366) :
//!
//! - l'arrêt tue le processus lancé, **pas ses éventuels petits-enfants** (sous Windows, `kill`
//!   ne vise que le processus ; ffmpeg et ffprobe statiques n'en créent pas). Si un autre
//!   programme passe un jour par ce module, il faudra un Job Object ;
//! - la vérification d'empreinte puis le lancement ne sont pas atomiques (TOCTOU) : le fichier
//!   peut être remplacé entre les deux par quiconque écrit dans le dossier d'installation, ce
//!   qui est hors du modèle de menace décrit plus haut ;
//! - le hachage s'interrompt à l'annulation (entre deux morceaux de 64 Kio) ; après la fin du
//!   processus, la lecture de stdout/stderr n'attend qu'un court délai ([`READER_GRACE`]) : un
//!   petit-enfant qui garderait les tubes ouverts ne bloque pas l'appelant (sortie alors vide).

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::install::download::cancelled;
use crate::settings::{self, Environment};
use crate::{AppError, AppResult};

/// Fichier d'empreintes à côté des exécutables : une ligne `<sha256 hex>  <nom>` par fichier.
pub const MANIFEST_FILE: &str = "ffmpeg.sha256";

/// Taille maximale du fichier d'empreintes lu.
const MAX_MANIFEST: u64 = 16 * 1024;
/// Début de stdout conservé par défaut.
pub const DEFAULT_STDOUT_CAP: usize = 1024 * 1024;
/// Fin de stderr conservée.
const STDERR_TAIL: usize = 8 * 1024;
/// Attente maximale des lecteurs de stdout/stderr après la fin du processus.
const READER_GRACE: Duration = Duration::from_secs(5);

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Programme du dossier ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Ffmpeg,
    Ffprobe,
}

impl Tool {
    /// Nom du fichier, tel qu'il figure dans le fichier d'empreintes.
    pub fn file_name(self) -> &'static str {
        match (self, cfg!(windows)) {
            (Tool::Ffmpeg, true) => "ffmpeg.exe",
            (Tool::Ffmpeg, false) => "ffmpeg",
            (Tool::Ffprobe, true) => "ffprobe.exe",
            (Tool::Ffprobe, false) => "ffprobe",
        }
    }
}

/// Limites d'un lancement.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Durée maximale ; au-delà le processus est tué et l'appel échoue.
    pub timeout: Duration,
    /// Octets de stdout conservés (le début ; le reste est lu puis jeté).
    pub stdout_cap: usize,
}

impl Limits {
    pub fn new(timeout: Duration) -> Self {
        Self {
            timeout,
            stdout_cap: DEFAULT_STDOUT_CAP,
        }
    }
}

/// Résultat d'un processus terminé de lui-même.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Code de sortie (`None` : terminé par un signal).
    pub code: Option<i32>,
    /// Début de stdout (au plus `Limits::stdout_cap` octets).
    pub stdout: Vec<u8>,
    /// Fin de stderr, décodée en tolérant les erreurs.
    pub stderr: String,
}

impl Output {
    pub fn success(&self) -> bool {
        self.code == Some(0)
    }
}

/// Dossier ffmpeg embarqué, localisé. Ne garantit pas l'empreinte : elle est revérifiée à chaque
/// lancement par [`Ffmpeg::run`].
#[derive(Debug, Clone)]
pub struct Ffmpeg {
    dir: PathBuf,
}

impl Ffmpeg {
    /// Localise le ffmpeg embarqué (voir `settings::ffmpeg_path`).
    pub fn locate(env: &Environment) -> AppResult<Self> {
        let exe = settings::ffmpeg_path(env).ok_or_else(|| {
            AppError::ConverterUnavailable("ffmpeg embarqué introuvable".to_string())
        })?;
        let dir = exe
            .parent()
            .ok_or_else(|| AppError::ConverterUnavailable("ffmpeg embarqué introuvable".into()))?;
        Ok(Self::in_dir(dir))
    }

    /// Dossier contenant les exécutables et `ffmpeg.sha256`.
    pub fn in_dir(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Chemin de l'exécutable après vérification de son empreinte (calcul bloquant : à appeler
    /// depuis `spawn_blocking` dans un contexte asynchrone).
    pub fn verified_path(&self, tool: Tool) -> AppResult<PathBuf> {
        self.verified_path_until(tool, None)
    }

    /// Comme [`Self::verified_path`] ; le hachage s'arrête (erreur) dès que `stop` passe à vrai.
    fn verified_path_until(&self, tool: Tool, stop: Option<&AtomicBool>) -> AppResult<PathBuf> {
        let name = tool.file_name();
        let path = self.dir.join(name);
        let expected = expected_hash(&self.dir.join(MANIFEST_FILE), name)?;
        let actual = hash_file(&path, stop).map_err(|e| {
            AppError::ConverterUnavailable(format!("{name} illisible ({}) : {e}", path.display()))
        })?;
        if actual != expected {
            return Err(AppError::ConverterUnavailable(format!(
                "{name} refusé : l'empreinte SHA-256 ne correspond pas à {MANIFEST_FILE} \
                 (réinstaller Flam Launcher)"
            )));
        }
        Ok(path)
    }

    /// Vérifie l'empreinte puis lance `tool` avec `args` (sans shell). Annulation par `cancel`
    /// (`AppError::Cancelled`), délai dépassé : erreur `TimedOut`. Le processus est tué dans ces
    /// deux cas. Un fichier de sortie partiel peut rester : l'appelant écrit dans un temporaire et ne le
    /// renomme que si success().
    pub async fn run(
        &self,
        tool: Tool,
        args: &[OsString],
        limits: Limits,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<Output> {
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        let this = self.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_hash = stop.clone();
        let path = tokio::select! {
            () = cancelled(cancel) => {
                // Le hachage en cours (thread bloquant) s'arrête au prochain morceau.
                stop.store(true, Ordering::Relaxed);
                return Err(AppError::Cancelled);
            }
            joined = tokio::task::spawn_blocking(move || {
                this.verified_path_until(tool, Some(&stop_hash))
            }) => {
                joined.map_err(|e| std::io::Error::other(e.to_string()))??
            }
        };
        run_program(&path, args, limits, cancel).await
    }
}

/// Lance `program` sans shell et lit sa sortie. Aucune vérification d'empreinte ici : réservé à
/// [`Ffmpeg::run`] (et aux tests).
async fn run_program(
    program: &Path,
    args: &[OsString],
    limits: Limits,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<Output> {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Environnement réduit : ni PATH ni variables d'un tiers (ffmpeg n'en a pas besoin ; sous
    // Windows, SystemRoot reste nécessaire au chargement des DLL système).
    cmd.env_clear();
    for key in ["SystemRoot", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            cmd.env(key, value);
        }
    }
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let mut child = cmd.spawn().map_err(|e| {
        AppError::ConverterUnavailable(format!(
            "impossible de lancer ffmpeg ({}) : {e}",
            program.display()
        ))
    })?;
    let stdout = child.stdout.take().expect("stdout redirigé");
    let stderr = child.stderr.take().expect("stderr redirigé");
    let out_task = tokio::spawn(read_head(stdout, limits.stdout_cap));
    let err_task = tokio::spawn(read_tail(stderr, STDERR_TAIL));

    let outcome = tokio::select! {
        status = child.wait() => Ok(status),
        () = cancelled(cancel) => Err(AppError::Cancelled),
        () = tokio::time::sleep(limits.timeout) => Err(AppError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!("ffmpeg : délai de {:?} dépassé", limits.timeout),
        ))),
    };
    let status = match outcome {
        Ok(status) => status?,
        Err(stop) => {
            // Tué puis attendu : plus aucun processus ni tube ouvert au retour.
            let _ = child.kill().await;
            out_task.abort();
            err_task.abort();
            return Err(stop);
        }
    };
    let (stdout, stderr) = tokio::join!(
        join_bounded(out_task, READER_GRACE),
        join_bounded(err_task, READER_GRACE)
    );
    Ok(Output {
        code: status.code(),
        stdout,
        stderr,
    })
}

/// Attend un lecteur au plus `grace` ; au-delà (tube gardé ouvert par un petit-enfant), il est
/// abandonné et la valeur par défaut est rendue.
async fn join_bounded<T: Default>(mut handle: JoinHandle<T>, grace: Duration) -> T {
    match tokio::time::timeout(grace, &mut handle).await {
        Ok(joined) => joined.unwrap_or_default(),
        Err(_) => {
            handle.abort();
            T::default()
        }
    }
}

/// Lit jusqu'à la fin ; ne garde que les `cap` premiers octets.
async fn read_head<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let room = cap.saturating_sub(kept.len());
                kept.extend_from_slice(&chunk[..n.min(room)]);
            }
        }
    }
    kept
}

/// Lit jusqu'à la fin ; ne garde que les `cap` derniers octets.
async fn read_tail<R: AsyncRead + Unpin>(mut reader: R, cap: usize) -> String {
    let mut tail: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                tail.extend_from_slice(&chunk[..n]);
                if tail.len() > cap * 2 {
                    tail.drain(..tail.len() - cap);
                }
            }
        }
    }
    let start = tail.len().saturating_sub(cap);
    String::from_utf8_lossy(&tail[start..]).into_owned()
}

/// SHA-256 (hexadécimal minuscule) d'un fichier, lu par morceaux ; `stop` l'interrompt.
fn hash_file(path: &Path, stop: Option<&AtomicBool>) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        if stop.is_some_and(|s| s.load(Ordering::Relaxed)) {
            return Err(std::io::ErrorKind::Interrupted.into());
        }
        let n = file.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        hasher.update(&chunk[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Empreinte attendue de `name` d'après le fichier d'empreintes. Refuse un fichier absent, trop
/// gros, une ligne mal formée, un nom en double ou un nom absent.
fn expected_hash(manifest: &Path, name: &str) -> AppResult<String> {
    let unavailable = |why: String| AppError::ConverterUnavailable(why);
    let file = std::fs::File::open(manifest)
        .map_err(|e| unavailable(format!("{MANIFEST_FILE} illisible : {e}")))?;
    let mut text = String::new();
    file.take(MAX_MANIFEST + 1)
        .read_to_string(&mut text)
        .map_err(|e| unavailable(format!("{MANIFEST_FILE} illisible : {e}")))?;
    if text.len() as u64 > MAX_MANIFEST {
        return Err(unavailable(format!("{MANIFEST_FILE} trop gros")));
    }
    let mut found: Option<String> = None;
    for line in text.trim_start_matches('\u{feff}').lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // `sha256sum` : « <hex>  <nom> » (ou « <hex> *<nom> » en mode binaire).
        let (hash, file) = line
            .split_once("  ")
            .or_else(|| line.split_once(" *"))
            .ok_or_else(|| unavailable(format!("{MANIFEST_FILE} : ligne mal formée")))?;
        let valid = hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit());
        if !valid {
            return Err(unavailable(format!(
                "{MANIFEST_FILE} : empreinte mal formée"
            )));
        }
        if file == name && found.replace(hash.to_ascii_lowercase()).is_some() {
            return Err(unavailable(format!("{MANIFEST_FILE} : {name} en double")));
        }
    }
    found.ok_or_else(|| unavailable(format!("{name} absent de {MANIFEST_FILE}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    fn install(dir: &Path, content: &[u8], manifest_hash: &str) -> Ffmpeg {
        fs::write(dir.join(Tool::Ffmpeg.file_name()), content).unwrap();
        fs::write(
            dir.join(MANIFEST_FILE),
            format!("{manifest_hash}  {}\n", Tool::Ffmpeg.file_name()),
        )
        .unwrap();
        Ffmpeg::in_dir(dir)
    }

    fn no_cancel() -> (watch::Sender<bool>, watch::Receiver<bool>) {
        watch::channel(false)
    }

    #[test]
    fn empreinte_conforme_acceptee() {
        let dir = tempfile::tempdir().unwrap();
        let ff = install(dir.path(), b"binaire", &sha(b"binaire"));
        assert_eq!(
            ff.verified_path(Tool::Ffmpeg).unwrap(),
            dir.path().join(Tool::Ffmpeg.file_name())
        );
    }

    #[test]
    fn empreinte_differente_refusee() {
        let dir = tempfile::tempdir().unwrap();
        let ff = install(dir.path(), b"binaire modifie", &sha(b"binaire"));
        let err = ff.verified_path(Tool::Ffmpeg).unwrap_err();
        assert!(err.to_string().contains("empreinte"), "{err}");
    }

    #[test]
    fn manifeste_absent_ou_incomplet_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let ff = Ffmpeg::in_dir(dir.path());
        fs::write(dir.path().join(Tool::Ffmpeg.file_name()), b"x").unwrap();
        assert!(ff.verified_path(Tool::Ffmpeg).is_err(), "manifeste absent");
        // ffprobe absent du manifeste.
        let ff = install(dir.path(), b"x", &sha(b"x"));
        fs::write(dir.path().join(Tool::Ffprobe.file_name()), b"x").unwrap();
        assert!(ff.verified_path(Tool::Ffmpeg).is_ok());
        assert!(ff.verified_path(Tool::Ffprobe).is_err());
    }

    #[test]
    fn manifeste_mal_forme_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let name = Tool::Ffmpeg.file_name();
        let good = sha(b"x");
        let ff = install(dir.path(), b"x", &good);
        for bad in [
            format!("{} {name}\n", &good[..10]),
            format!("{good}  {name}\n{good}  {name}\n"),
            format!("zz{}  {name}\n", &good[2..]),
            "n'importe quoi\n".to_string(),
        ] {
            fs::write(dir.path().join(MANIFEST_FILE), bad).unwrap();
            assert!(ff.verified_path(Tool::Ffmpeg).is_err());
        }
        // BOM, majuscules, mode binaire et lignes vides tolérés.
        fs::write(
            dir.path().join(MANIFEST_FILE),
            format!("\u{feff}\n{} *{name}\n", good.to_uppercase()),
        )
        .unwrap();
        assert!(ff.verified_path(Tool::Ffmpeg).is_ok());
    }

    #[tokio::test]
    async fn binaire_modifie_jamais_lance() {
        let dir = tempfile::tempdir().unwrap();
        let ff = install(dir.path(), b"pas un exe", &sha(b"autre chose"));
        let (_tx, mut rx) = no_cancel();
        let err = ff
            .run(
                Tool::Ffmpeg,
                &[],
                Limits::new(Duration::from_secs(5)),
                &mut rx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::ConverterUnavailable(_)), "{err:?}");
        assert!(err.to_string().contains("empreinte"), "{err}");
    }

    #[tokio::test]
    async fn annulation_avant_lancement() {
        let dir = tempfile::tempdir().unwrap();
        let ff = install(dir.path(), b"x", &sha(b"x"));
        let (tx, mut rx) = no_cancel();
        tx.send(true).unwrap();
        let err = ff
            .run(
                Tool::Ffmpeg,
                &[],
                Limits::new(Duration::from_secs(5)),
                &mut rx,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Cancelled), "{err:?}");
    }

    /// Programme système long et bavard, pour tester sortie, délai et annulation sans ffmpeg.
    fn sleeper(seconds: u32) -> (PathBuf, Vec<OsString>) {
        if cfg!(windows) {
            let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            (
                PathBuf::from(root).join("System32").join("ping.exe"),
                vec![
                    "-n".into(),
                    (seconds + 1).to_string().into(),
                    "127.0.0.1".into(),
                ],
            )
        } else {
            (
                PathBuf::from("/bin/sleep"),
                vec![seconds.to_string().into()],
            )
        }
    }

    #[tokio::test]
    async fn delai_depasse_tue_le_processus() {
        let (program, args) = sleeper(30);
        let (_tx, mut rx) = no_cancel();
        let started = std::time::Instant::now();
        let err = run_program(
            &program,
            &args,
            Limits::new(Duration::from_millis(300)),
            &mut rx,
        )
        .await
        .unwrap_err();
        match err {
            AppError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
            other => panic!("{other:?}"),
        }
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[tokio::test]
    async fn annulation_en_cours_tue_le_processus() {
        let (program, args) = sleeper(30);
        let (tx, mut rx) = no_cancel();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            tx.send(true).unwrap();
            tx
        });
        let started = std::time::Instant::now();
        let err = run_program(
            &program,
            &args,
            Limits::new(Duration::from_secs(60)),
            &mut rx,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::Cancelled), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(10));
        drop(canceller.await);
    }

    /// Programme système qui écrit au moins 100 octets sur stdout puis se termine.
    fn talker() -> (PathBuf, Vec<OsString>) {
        if cfg!(windows) {
            sleeper(0)
        } else {
            (
                PathBuf::from("/bin/sh"),
                vec!["-c".into(), "printf '%0100d' 0".into()],
            )
        }
    }

    #[tokio::test]
    async fn sortie_lue_et_bornee() {
        let (program, args) = talker();
        let (_tx, mut rx) = no_cancel();
        let out = run_program(
            &program,
            &args,
            Limits {
                timeout: Duration::from_secs(20),
                stdout_cap: 16,
            },
            &mut rx,
        )
        .await
        .unwrap();
        assert!(out.success(), "{out:?}");
        assert_eq!(out.stdout.len(), 16, "stdout tronqué au début");
    }

    #[tokio::test]
    async fn lecteur_abandonne_apres_le_delai() {
        let never = tokio::spawn(async {
            std::future::pending::<()>().await;
            vec![1u8]
        });
        let started = std::time::Instant::now();
        let got = join_bounded(never, Duration::from_millis(100)).await;
        assert!(got.is_empty());
        assert!(started.elapsed() < Duration::from_secs(5));
        let done = tokio::spawn(async { vec![7u8] });
        assert_eq!(join_bounded(done, Duration::from_secs(5)).await, vec![7]);
    }

    #[test]
    fn hachage_interrompu() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        fs::write(&path, b"x").unwrap();
        let stop = AtomicBool::new(true);
        let err = hash_file(&path, Some(&stop)).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::Interrupted);
        assert_eq!(hash_file(&path, None).unwrap(), sha(b"x"));
    }

    #[tokio::test]
    async fn programme_introuvable() {
        let (_tx, mut rx) = no_cancel();
        let err = run_program(
            Path::new("Z:/absent/ffmpeg.exe"),
            &[],
            Limits::new(Duration::from_secs(5)),
            &mut rx,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::ConverterUnavailable(_)), "{err:?}");
    }

    /// ffmpeg réel (build/ffmpeg produit par `npm run build:ffmpeg`), sinon test ignoré :
    /// MP3 mono 22 050 Hz converti en MP3 44 100 Hz (critère d'acceptation de #209).
    #[tokio::test]
    async fn mp3_mono_22050_vers_44100() {
        let here = Path::new(env!("CARGO_MANIFEST_DIR"));
        let dir = here.join("..").join("build").join("ffmpeg");
        if !dir.join(Tool::Ffmpeg.file_name()).is_file() || !dir.join(MANIFEST_FILE).is_file() {
            assert!(
                std::env::var_os("FLAM_REQUIRE_FFMPEG").is_none(),
                "ffmpeg embarqué non construit alors que FLAM_REQUIRE_FFMPEG est défini"
            );
            eprintln!("ffmpeg embarqué non construit : test ignoré");
            return;
        }
        let ff = Ffmpeg::in_dir(&dir);
        let work = tempfile::tempdir().unwrap();
        let src = work.path().join("mono.mp3");
        let dst = work.path().join("stereo44.mp3");
        let limits = Limits::new(Duration::from_secs(60));
        let (_tx, mut rx) = no_cancel();
        let common: Vec<OsString> = ["-hide_banner", "-nostdin", "-v", "error", "-y"]
            .map(OsString::from)
            .to_vec();
        let mut gen = common.clone();
        gen.extend(
            [
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:duration=1:sample_rate=22050",
                "-ac",
                "1",
                "-c:a",
                "libmp3lame",
            ]
            .map(OsString::from),
        );
        gen.push(src.clone().into());
        assert!(ff
            .run(Tool::Ffmpeg, &gen, limits, &mut rx)
            .await
            .unwrap()
            .success());
        let mut conv = common;
        conv.push("-i".into());
        conv.push(src.into());
        conv.extend(
            ["-vn", "-ar", "44100", "-c:a", "libmp3lame", "-b:a", "64k"].map(OsString::from),
        );
        conv.push(dst.clone().into());
        let out = ff.run(Tool::Ffmpeg, &conv, limits, &mut rx).await.unwrap();
        assert!(out.success(), "{out:?}");
        let mut probe: Vec<OsString> = [
            "-v",
            "error",
            "-select_streams",
            "a:0",
            "-show_entries",
            "stream=codec_name,sample_rate",
            "-of",
            "default=noprint_wrappers=1",
        ]
        .map(OsString::from)
        .to_vec();
        probe.push(dst.into());
        let out = ff
            .run(Tool::Ffprobe, &probe, limits, &mut rx)
            .await
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("codec_name=mp3") && text.contains("sample_rate=44100"),
            "{text}"
        );
    }
}
