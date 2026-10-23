//! Rééchantillonnage à 44 100 Hz des MP3 par un ffmpeg **externe** (`_find_ffmpeg` et
//! `_resample_mp3` de `tools/telmi2flam/telmi2flam.py`, #46, #354). Le port n'embarque ni
//! décodeur ni encodeur MP3 : il lance le ffmpeg désigné par `FLAM_FFMPEG` (le launcher y met
//! le sien), sinon celui du `PATH`, avec les mêmes arguments, le même délai et les mêmes
//! raisons d'échec que Python. La sortie de ffmpeg est opaque : c'est l'appelant
//! ([`crate::assets`]) qui la valide avec [`crate::mp3map::build`].
//!
//! Invariants :
//! - aucun shell, entrée standard et sortie standard nulles, protocole `file` seul, dossier de
//!   travail temporaire `t2f_ff_*` propre à l'appel (droits `0700` sous Unix) ;
//! - le processus enfant ne survit jamais à l'appel : annulation (jeton consulté pendant
//!   l'attente) ou délai dépassé → `kill` puis `wait` ; panique du binaire → le crochet le tue
//!   ([`kill_children`], registre des PID) avant de supprimer les temporaires (#132) ;
//! - le dossier temporaire est inscrit au registre des temporaires de [`crate::output`] (le
//!   crochet de panique le supprime) et supprimé dans tous les cas (garde [`Drop`]) ; la sortie est lue
//!   avant, et un échec de suppression (antivirus, handle Windows) est ignoré au lieu de perdre
//!   une sortie valide (constat de #274, corrigé ici seulement) ;
//! - la sortie d'erreur est lue par un fil (pas d'interblocage de tube), mémoire bornée à sa
//!   fin ([`STDERR_TAIL`]) ;
//! - aucune panique.
//!
//! Écarts assumés avec Python : le texte des erreurs du système (`ffmpeg inutilisable (…)`)
//! est celui de Rust ; un ffmpeg trouvé dans le `PATH` est lancé par son chemin absolu (Python
//! garde un chemin relatif si l'entrée du `PATH` l'est) ; si un descendant de ffmpeg garde la
//! sortie d'erreur ouverte, son fil de lecture survit à l'appel (Python attendrait la fin du
//! tube jusqu'au délai, puis échoue de la même façon) ; une dernière ligne d'erreur de plus de
//! [`STDERR_TAIL`] octets (ou plus de `STDERR_TAIL` octets de blancs finaux) donne un autre
//! message (improbable avec `-loglevel error`) ; sous Unix, « exécutable » est un bit `x`
//! quelconque (Python : `os.access(X_OK)`, selon l'utilisateur) ; un dossier courant supprimé
//! donne « introuvable » (Python : exception, avertissement `mp3map_failed`).

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Mutex, MutexGuard, PoisonError, TryLockError};
use std::time::{Duration, Instant};

use crate::error::ConvError;
use crate::mp3map;
use crate::text::py_strip;

/// Fréquence attendue par le firmware (`SAMPLERATE`).
pub const SAMPLERATE: u32 = 44_100;
/// Délai par fichier (`FFMPEG_TIMEOUT`).
pub const FFMPEG_TIMEOUT: Duration = Duration::from_secs(120);
/// Débits de sortie possibles en kbit/s (`_MP3_BITRATES`, plage TELMI valide en MPEG-1).
pub const MP3_BITRATES: [u32; 7] = [64, 80, 96, 112, 128, 160, 192];
/// Octets de fin de la sortie d'erreur conservés (Python garde tout, puis n'en lit que la
/// dernière ligne).
pub const STDERR_TAIL: usize = 64 * 1024;
/// Pause maximale entre deux consultations du processus et du jeton d'annulation.
const POLL_MAX: Duration = Duration::from_millis(20);
/// `CREATE_NO_WINDOW` : pas de console ouverte pour ffmpeg sous Windows.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Résultat d'un rééchantillonnage : octets produits, ou raison de l'échec (texte de Python).
pub type Outcome = Result<Vec<u8>, String>;

/// Raison quand aucun ffmpeg n'est trouvé.
pub const NOT_FOUND: &str = "ffmpeg introuvable (variable FLAM_FFMPEG ou PATH)";

/// `_find_ffmpeg` : `FLAM_FFMPEG` s'il est défini (vide = désactivé), sinon le `PATH`, jamais un
/// ffmpeg trouvé dans le dossier courant. Variables lues à chaque appel, comme Python.
pub fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(env) = std::env::var_os("FLAM_FFMPEG") {
        return from_env(&env);
    }
    let found = which("ffmpeg")?;
    let found = abspath(&found)?;
    let cwd = abspath(&std::env::current_dir().ok()?)?;
    if found.parent() == Some(cwd.as_path()) {
        return None;
    }
    Some(found)
}

/// Valeur de `FLAM_FFMPEG` : `strip()`, vide = désactivé, sinon chemin absolu d'un fichier.
pub fn from_env(value: &OsStr) -> Option<PathBuf> {
    let value = match value.to_str() {
        Some(s) => OsString::from(py_strip(s)),
        None => value.to_owned(),
    };
    if value.is_empty() {
        return None;
    }
    let path = PathBuf::from(value);
    if path.is_file() {
        abspath(&path)
    } else {
        None
    }
}

/// `os.path.abspath` : chemin absolu normalisé (sous Windows, `GetFullPathNameW` comme
/// `ntpath` ; ailleurs, normalisation lexicale de `posixpath.normpath`).
fn abspath(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    #[cfg(windows)]
    {
        Some(abs)
    }
    #[cfg(not(windows))]
    {
        use std::path::Component;
        let mut out = PathBuf::new();
        for c in abs.components() {
            match c {
                Component::CurDir => {}
                Component::ParentDir => {
                    if out.parent().is_some() {
                        out.pop();
                    }
                }
                other => out.push(other),
            }
        }
        Some(out)
    }
}

/// Découpe une variable de liste (`PATH`, `PATHEXT`) sur le séparateur du système, sans
/// retirer de guillemets (`str.split(os.pathsep)`, contrairement à `env::split_paths`).
fn split_list(value: &OsStr) -> Vec<OsString> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        let wide: Vec<u16> = value.encode_wide().collect();
        wide.split(|&c| c == u16::from(b';'))
            .map(OsString::from_wide)
            .collect()
    }
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        value
            .as_bytes()
            .split(|&c| c == b':')
            .map(|s| OsString::from_vec(s.to_vec()))
            .collect()
    }
}

/// `_access_check(fn, F_OK | X_OK)` : existe, exécutable, pas un dossier.
fn access_check(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.is_dir() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `shutil.which(cmd)` pour un nom sans dossier (Python 3.12+), avec l'environnement du
/// processus.
fn which(cmd: &str) -> Option<PathBuf> {
    which_in(
        cmd,
        std::env::var_os("PATH"),
        std::env::var_os("NoDefaultCurrentDirectoryInExePath").is_some(),
        std::env::var_os("PATHEXT"),
    )
}

/// [`which`] sur un environnement donné : `path` (`PATH`), `no_default_cwd` (la variable
/// `NoDefaultCurrentDirectoryInExePath` existe), `pathext` (`PATHEXT`, Windows).
fn which_in(
    cmd: &str,
    path: Option<OsString>,
    no_default_cwd: bool,
    pathext: Option<OsString>,
) -> Option<PathBuf> {
    let path = match path {
        Some(p) => p,
        None if cfg!(windows) => OsString::from(".;C:\\bin"),
        None => OsString::from("/bin:/usr/bin"),
    };
    if path.is_empty() {
        return None;
    }
    let mut dirs = split_list(&path);
    let mut files: Vec<OsString> = vec![OsString::from(cmd)];
    if cfg!(windows) {
        // NeedCurrentDirectoryForExePath : le dossier courant d'abord, sauf si la variable
        // NoDefaultCurrentDirectoryInExePath existe.
        if !no_default_cwd {
            dirs.insert(0, OsString::from("."));
        }
        let pathext = pathext
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| OsString::from(".COM;.EXE;.BAT;.CMD;.VBS;.JS;.WS;.MSC"));
        let exts: Vec<String> = split_list(&pathext)
            .into_iter()
            .filter(|e| !e.is_empty())
            .map(|e| e.to_string_lossy().trim_end_matches('.').to_owned())
            .collect();
        let upper = cmd.to_uppercase();
        let direct = exts.iter().any(|e| upper.ends_with(&e.to_uppercase()));
        files = exts
            .iter()
            .map(|e| OsString::from(format!("{cmd}{e}")))
            .collect();
        if direct {
            files.insert(0, OsString::from(cmd));
        }
    }
    let mut seen = std::collections::HashSet::new();
    for dir in dirs {
        let norm = if cfg!(windows) {
            dir.to_string_lossy().to_lowercase().replace('/', "\\")
        } else {
            dir.to_string_lossy().into_owned()
        };
        if !seen.insert(norm) {
            continue;
        }
        for file in &files {
            let name = Path::new(&dir).join(file);
            if access_check(&name) {
                return Some(name);
            }
        }
    }
    None
}

/// Débit de sortie (kbit/s) : débit de l'audio seul de la source, sans tag ID3v2 ni ID3v1 de
/// fin (une pochette intégrée gonflerait l'estimation), le plus proche de [`MP3_BITRATES`]
/// (le premier en cas d'égalité) ; 128 si la durée n'est pas positive.
pub fn estimate_kbps(data: &[u8], duration: f64) -> u32 {
    // `if duration and duration > 0` : NaN et négatifs compris.
    if duration.is_nan() || duration <= 0.0 {
        return 128;
    }
    let n = data.len();
    let id3v1 = n >= 128 && &data[n - 128..n - 125] == b"TAG";
    let end = if id3v1 { n - 128 } else { n };
    let audio = end.saturating_sub(mp3map::skip_id3v2(data));
    let kbps = (audio as u64 * 8) as f64 / duration / 1000.0;
    let mut best = MP3_BITRATES[0];
    let mut best_diff = (f64::from(best) - kbps).abs();
    for &b in &MP3_BITRATES[1..] {
        let diff = (f64::from(b) - kbps).abs();
        if diff < best_diff {
            best = b;
            best_diff = diff;
        }
    }
    best
}

/// `"file:" + chemin`.
fn file_url(path: &Path) -> OsString {
    let mut s = OsString::from("file:");
    s.push(path.as_os_str());
    s
}

/// Arguments de ffmpeg (sans l'exécutable), identiques à Python.
pub fn ffmpeg_args(src: &Path, dst: &Path, kbps: u32) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .iter()
    .map(OsString::from)
    .collect();
    args.push(file_url(src));
    let ar = SAMPLERATE.to_string();
    let ba = format!("{kbps}k");
    for a in [
        "-map",
        "0:a:0",
        "-vn",
        "-sn",
        "-dn",
        "-map_metadata",
        "-1",
        "-ar",
        &ar,
        "-c:a",
        "libmp3lame",
        "-b:a",
        &ba,
        "-write_xing",
        "0",
        "-id3v2_version",
        "0",
        "-f",
        "mp3",
    ] {
        args.push(OsString::from(a));
    }
    args.push(file_url(dst));
    args
}

/// Dossier de travail temporaire, supprimé à la destruction (erreurs ignorées).
struct WorkDir {
    path: PathBuf,
}

impl WorkDir {
    fn create(root: &Path) -> io::Result<WorkDir> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        // `abspath(gettempdir())` de Python : un dossier temporaire relatif (`TMPDIR=tmp`) serait
        // résolu une seconde fois par ffmpeg, qui tourne avec ce dossier pour dossier courant.
        let root = abspath(root).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "dossier temporaire inutilisable",
            )
        })?;
        for _ in 0..100 {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0);
            let path = root.join(format!("t2f_ff_{}_{n}_{nanos:08x}", std::process::id()));
            #[cfg_attr(not(unix), allow(unused_mut))]
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            // Inscrit avant la création : le crochet de panique du binaire le supprime (#132).
            // Un échec de création le retire aussitôt. Dans l'intervalle, une panique d'un autre
            // fil pourrait supprimer un dossier de ce nom déjà présent (`AlreadyExists`) : le
            // nom porte le pid, l'horloge et un compteur, ce ne peut être que le reste
            // abandonné d'un ancien processus de même pid, jamais un dossier vivant.
            crate::output::track(&path);
            match builder.create(&path) {
                Ok(()) => return Ok(WorkDir { path }),
                Err(e) => {
                    crate::output::untrack(std::slice::from_ref(&path));
                    if e.kind() == io::ErrorKind::AlreadyExists {
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "aucun nom de dossier temporaire libre",
        ))
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        crate::output::untrack(std::slice::from_ref(&self.path));
    }
}

/// PID des ffmpeg en cours, pour [`kill_children`] (crochet de panique du binaire, #132).
static CHILDREN: Mutex<Vec<u32>> = Mutex::new(Vec::new());

fn children() -> MutexGuard<'static, Vec<u32>> {
    CHILDREN.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Inscription d'un ffmpeg lancé, retirée à la destruction. Déclarée **après** le [`Child`] :
/// elle est détruite avant lui, donc tant qu'un PID est inscrit, le handle du processus reste
/// ouvert (sous Windows, le PID ne peut pas être réattribué).
struct Running(u32);

impl Running {
    fn new(child: &Child) -> Running {
        children().push(child.id());
        Running(child.id())
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let mut reg = children();
        if let Some(i) = reg.iter().rposition(|&p| p == self.0) {
            reg.swap_remove(i);
        }
    }
}

/// Tue les ffmpeg en cours et attend leur fin (5 s au plus chacun, pour que leurs fichiers
/// soient fermés avant la suppression de `t2f_ff_*`). Appelé par le crochet de panique
/// du binaire, avant [`crate::output::discard_pending`] : après `process::exit`, rien d'autre
/// ne les arrêterait (le job du launcher seulement à sa fermeture). Verrou tenté 1 s au plus.
///
/// Limite sous Unix : le PID est retiré juste après que `try_wait` a récolté le processus ;
/// une panique d'un autre fil dans cet intervalle de quelques instructions pourrait viser un
/// PID réattribué. De même, `waitpid` récolte ici le processus à l'insu du [`Child`] : si le
/// fil de travail l'interroge ensuite avant `process::exit` (panique d'un autre fil, cas
/// théorique avec un seul fil de travail), `try_wait` échoue (`ECHILD`) et `Child::kill` vise
/// un PID libéré. Verrou non obtenu en 1 s (impossible en pratique) : rien n'est tué.
///
/// Hors de portée (ffmpeg finit seul son fichier, `t2f_ff_*` reste sur le disque) : arrêt
/// brutal de telmi2flam (`TerminateProcess`, `kill -9` ; sous le launcher, son job tue aussi
/// ffmpeg) et débordement de pile (abandon sans crochet).
pub fn kill_children() {
    for _ in 0..100 {
        match CHILDREN.try_lock() {
            Ok(reg) => return reg.iter().for_each(|&pid| sys::kill_pid(pid)),
            Err(TryLockError::Poisoned(p)) => {
                return p.into_inner().iter().for_each(|&pid| sys::kill_pid(pid))
            }
            Err(TryLockError::WouldBlock) => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
        fn TerminateProcess(process: *mut c_void, code: u32) -> i32;
        fn WaitForSingleObject(handle: *mut c_void, ms: u32) -> u32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }

    const PROCESS_TERMINATE: u32 = 0x0001;
    const SYNCHRONIZE: u32 = 0x0010_0000;

    pub fn kill_pid(pid: u32) {
        // SAFETY : handle vérifié non nul, fermé une fois.
        unsafe {
            let h = OpenProcess(PROCESS_TERMINATE | SYNCHRONIZE, 0, pid);
            if h.is_null() {
                return;
            }
            TerminateProcess(h, 1);
            WaitForSingleObject(h, 5000);
            CloseHandle(h);
        }
    }
}

#[cfg(unix)]
mod sys {
    use std::time::{Duration, Instant};

    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
        fn waitpid(pid: i32, status: *mut i32, options: i32) -> i32;
    }

    /// `WNOHANG` : même valeur sous Linux, macOS et les BSD. Ailleurs (illumos : 0x40),
    /// `waitpid` refuse l'option (`EINVAL`) et l'attente s'arrête aussitôt, comme avant.
    const WNOHANG: i32 = 1;

    /// `SIGKILL`, puis attente de la fin (5 s au plus, toutes les 10 ms), comme
    /// `WaitForSingleObject` sous Windows : ffmpeg récolté, ses fichiers sont fermés avant la
    /// suppression de `t2f_ff_*`. `waitpid` rend une erreur (`ECHILD`) si le processus a déjà
    /// été récolté : l'attente s'arrête alors aussi.
    pub fn kill_pid(pid: u32) {
        let Ok(pid) = i32::try_from(pid) else {
            return;
        };
        // SAFETY : appel sans pointeur ; SIGKILL (9).
        if unsafe { kill(pid, 9) } != 0 {
            return;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let mut status = 0i32;
            // SAFETY : `status` est un entier local valide pendant l'appel.
            let r = unsafe { waitpid(pid, &mut status, WNOHANG) };
            if r > 0
                || (r < 0
                    && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted)
            {
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod sys {
    pub fn kill_pid(_pid: u32) {}
}

/// Tue le processus et attend sa fin (handles et fichiers libérés avant la suppression du
/// dossier de travail).
fn kill(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// `r.returncode` de Python : non signé sous Windows, `-signal` sous Unix.
fn return_code(status: ExitStatus) -> i64 {
    #[cfg(windows)]
    {
        status.code().map_or(-1, |c| i64::from(c as u32))
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        match (status.code(), status.signal()) {
            (Some(c), _) => i64::from(c),
            (None, Some(s)) => -i64::from(s),
            (None, None) => -1,
        }
    }
}

/// Fin de ligne au sens de `str.splitlines()`.
fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\r'
            | '\x0b'
            | '\x0c'
            | '\x1c'
            | '\x1d'
            | '\x1e'
            | '\u{85}'
            | '\u{2028}'
            | '\u{2029}'
    )
}

/// `stderr.decode("utf-8", "replace").strip().splitlines()[-1][:200]`, ou `None` si vide.
pub fn last_stderr_line(stderr: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(stderr);
    let text = py_strip(&text);
    if text.is_empty() {
        return None;
    }
    let last = text.rsplit(is_line_break).next().unwrap_or(text);
    Some(last.chars().take(200).collect())
}

fn unusable(e: &io::Error) -> String {
    format!("ffmpeg inutilisable ({e})")
}

fn too_long(timeout: Duration) -> String {
    format!("ffmpeg trop long (> {} s)", timeout.as_secs())
}

/// Lit `stream` jusqu'à la fin, en ne gardant que ses `STDERR_TAIL` derniers octets.
fn read_tail(mut stream: impl Read) -> Vec<u8> {
    let mut tail = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > 2 * STDERR_TAIL {
                    tail.drain(..tail.len() - STDERR_TAIL);
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    if tail.len() > STDERR_TAIL {
        tail.drain(..tail.len() - STDERR_TAIL);
    }
    tail
}

/// Faute de test (#132) : avec `TELMI2FLAM_TEST_FAULT=panic:ffmpeg`, panique 300 ms après le
/// lancement de ffmpeg, pendant qu'il tourne. Absente de la release du launcher.
#[cfg(any(debug_assertions, feature = "test-hooks"))]
fn test_panic(started: Instant) {
    if started.elapsed() >= Duration::from_millis(300)
        && std::env::var("TELMI2FLAM_TEST_FAULT").is_ok_and(|v| v == "panic:ffmpeg")
    {
        panic!("faute injectee (ffmpeg en cours)");
    }
}

/// `_resample_mp3(data, duration)` : ffmpeg de [`find_ffmpeg`], dossier temporaire du
/// système, délai [`FFMPEG_TIMEOUT`]. `Err` seulement sur annulation.
pub fn resample_mp3(
    data: &[u8],
    duration: f64,
    cancelled: &dyn Fn() -> bool,
) -> Result<Outcome, ConvError> {
    let Some(exe) = find_ffmpeg() else {
        return Ok(Err(NOT_FOUND.to_owned()));
    };
    run_ffmpeg(
        &exe,
        data,
        estimate_kbps(data, duration),
        &std::env::temp_dir(),
        FFMPEG_TIMEOUT,
        cancelled,
    )
}

/// Lance `exe` sur `data` dans un dossier de travail créé sous `tmp_root`. `Ok(Err(raison))`
/// pour tout échec de ffmpeg (texte de Python) ; `Err` seulement sur annulation, après avoir
/// tué ffmpeg. Le dossier de travail est supprimé avant le retour.
pub fn run_ffmpeg(
    exe: &Path,
    data: &[u8],
    kbps: u32,
    tmp_root: &Path,
    timeout: Duration,
    cancelled: &dyn Fn() -> bool,
) -> Result<Outcome, ConvError> {
    if cancelled() {
        return Err(ConvError::cancelled());
    }
    let work = match WorkDir::create(tmp_root) {
        Ok(w) => w,
        Err(e) => return Ok(Err(unusable(&e))),
    };
    let src = work.path.join("in.mp3");
    let dst = work.path.join("out.mp3");
    if let Err(e) = std::fs::write(&src, data) {
        return Ok(Err(unusable(&e)));
    }
    let mut cmd = Command::new(exe);
    cmd.args(ffmpeg_args(&src, &dst, kbps))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .current_dir(&work.path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let started = Instant::now();
    let deadline = started + timeout;
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Ok(Err(unusable(&e))),
    };
    let running = Running::new(&child);
    let (tx, rx) = mpsc::channel();
    if let Some(stderr) = child.stderr.take() {
        let reader = std::thread::Builder::new()
            .name("ffmpeg-stderr".to_owned())
            .spawn(move || {
                let _ = tx.send(read_tail(stderr));
            });
        if let Err(e) = reader {
            kill(&mut child);
            return Ok(Err(unusable(&e)));
        }
    } else {
        drop(tx);
    }
    let mut pause = Duration::from_millis(1);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(e) => {
                kill(&mut child);
                return Ok(Err(unusable(&e)));
            }
        }
        if cancelled() {
            kill(&mut child);
            return Err(ConvError::cancelled());
        }
        #[cfg(any(debug_assertions, feature = "test-hooks"))]
        test_panic(started);
        if Instant::now() >= deadline {
            kill(&mut child);
            return Ok(Err(too_long(timeout)));
        }
        std::thread::sleep(pause);
        pause = (pause * 2).min(POLL_MAX);
    };
    // Processus récolté : PID retiré du registre avant qu'il puisse être réattribué (Unix).
    drop(running);
    // Sous Unix, un Ctrl+C au terminal envoie aussi SIGINT à ffmpeg (même groupe) : son échec
    // est alors celui de l'annulation, pas un « ffmpeg a echoue » à signaler avant `cancelled`.
    if cancelled() {
        return Err(ConvError::cancelled());
    }
    // Fin du tube d'erreur (un descendant peut le garder ouvert) : même délai, même jeton.
    let stderr = loop {
        match rx.recv_timeout(pause) {
            Ok(tail) => break tail,
            Err(RecvTimeoutError::Disconnected) => break Vec::new(),
            Err(RecvTimeoutError::Timeout) => {
                if cancelled() {
                    return Err(ConvError::cancelled());
                }
                if Instant::now() >= deadline {
                    return Ok(Err(too_long(timeout)));
                }
            }
        }
    };
    if !status.success() {
        let why = last_stderr_line(&stderr).unwrap_or_else(|| return_code(status).to_string());
        return Ok(Err(format!("ffmpeg a echoue ({why})")));
    }
    let out = match std::fs::read(&dst) {
        Ok(out) => out,
        Err(e) => return Ok(Err(unusable(&e))),
    };
    drop(work);
    if out.is_empty() {
        return Ok(Err("ffmpeg n'a rien produit".to_owned()));
    }
    Ok(Ok(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trames MPEG-1 Layer III stéréo à zéro (`mp3_frames` des tests Python).
    fn frames(b2: u8, n: usize) -> Vec<u8> {
        let rates = [44_100u32, 48_000, 32_000];
        let kbps = [
            0u32, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
        ];
        let br = kbps[usize::from(b2 >> 4)];
        let sr = rates[usize::from((b2 >> 2) & 3)];
        let len = (144_000 * br / sr) as usize + usize::from((b2 >> 1) & 1);
        let mut f = vec![0u8; len];
        f[..4].copy_from_slice(&[0xFF, 0xFB, b2, 0]);
        f.repeat(n)
    }

    fn kbps_of(data: &[u8]) -> u32 {
        let map = mp3map::build(data).unwrap();
        estimate_kbps(data, map.duration_s)
    }

    #[test]
    fn debit_estime_comme_python() {
        // 48 kHz : 128 kbit/s → 128 ; 320 → 192 (borne haute) ; 32 → 64 (borne basse).
        assert_eq!(kbps_of(&frames(0x94, 100)), 128);
        assert_eq!(kbps_of(&frames(0xE4, 100)), 192);
        assert_eq!(kbps_of(&frames(0x14, 100)), 64);
        // Tags ID3v2 (10 + 4000 octets) et ID3v1 exclus du calcul.
        let mut id3 = b"ID3\x03\x00\x00\x00\x00\x1f\x20".to_vec();
        id3.resize(10 + 4000, 0);
        let mut tagged = id3.clone();
        tagged.extend(frames(0x94, 100));
        tagged.extend(b"TAG");
        tagged.extend([0u8; 125]);
        assert_eq!(kbps_of(&tagged), 128);
        // Durée nulle ou non finie : 128.
        assert_eq!(estimate_kbps(b"x", 0.0), 128);
        assert_eq!(estimate_kbps(b"x", f64::NAN), 128);
        assert_eq!(estimate_kbps(b"x", -1.0), 128);
        // Égalité : le premier débit de la liste (72 est à 8 de 64 et de 80).
        assert_eq!(estimate_kbps(&[0u8; 9000], 1.0), 64);
        // Données plus courtes que le tag ID3v2 annoncé : 0 kbit/s → 64.
        assert_eq!(estimate_kbps(b"ID3\x03\x00\x00\x7f\x7f\x7f\x7f", 1.0), 64);
        // Moins de 128 octets : pas d'ID3v1 cherché.
        assert_eq!(estimate_kbps(b"TAG", 0.0001), 192);
    }

    #[test]
    fn arguments_identiques_a_python() {
        let args = ffmpeg_args(Path::new("w/in.mp3"), Path::new("w/out.mp3"), 96);
        let got: Vec<String> = args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let src = format!("file:{}", Path::new("w/in.mp3").display());
        let dst = format!("file:{}", Path::new("w/out.mp3").display());
        let want = [
            "-nostdin",
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-protocol_whitelist",
            "file",
            "-i",
            &src,
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-ar",
            "44100",
            "-c:a",
            "libmp3lame",
            "-b:a",
            "96k",
            "-write_xing",
            "0",
            "-id3v2_version",
            "0",
            "-f",
            "mp3",
            &dst,
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn derniere_ligne_d_erreur() {
        assert_eq!(last_stderr_line(b""), None);
        assert_eq!(last_stderr_line(b" \r\n\t"), None);
        assert_eq!(last_stderr_line(b"a\nb\r\n"), Some("b".to_owned()));
        assert_eq!(last_stderr_line(b"a\r\nb  c \x0c"), Some("b  c".to_owned()));
        assert_eq!(
            last_stderr_line(b"x\xc3\xa9\xff"),
            Some("x\u{e9}\u{fffd}".to_owned())
        );
        assert_eq!(
            last_stderr_line("a\u{2028}b".as_bytes()),
            Some("b".to_owned())
        );
        let long = "é".repeat(300);
        assert_eq!(last_stderr_line(long.as_bytes()).unwrap(), "é".repeat(200));
    }

    #[test]
    fn fin_de_sortie_d_erreur_bornee() {
        let mut data = vec![b'x'; 5 * STDERR_TAIL];
        data.extend(b"\nfin");
        let tail = read_tail(&data[..]);
        assert_eq!(tail.len(), STDERR_TAIL);
        assert!(tail.ends_with(b"\nfin"));
        assert_eq!(read_tail(&b"court"[..]), b"court");
    }

    #[test]
    fn variable_flam_ffmpeg() {
        let dir = std::env::temp_dir().join(format!("t2f-ffenv-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("ff.bin");
        std::fs::write(&exe, b"x").unwrap();
        assert_eq!(from_env(OsStr::new("")), None);
        assert_eq!(from_env(OsStr::new("  \t")), None);
        assert_eq!(from_env(dir.as_os_str()), None, "dossier");
        assert_eq!(from_env(dir.join("absent").as_os_str()), None);
        let padded = format!("  {}\n", exe.display());
        assert_eq!(from_env(OsStr::new(&padded)), Some(abspath(&exe).unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Dossier temporaire relatif (`TMPDIR=tmp`) : le dossier de travail est absolu (#369).
    #[test]
    fn dossier_de_travail_absolu() {
        let rel = PathBuf::from(format!("t2f-ffrel-{}", std::process::id()));
        std::fs::create_dir_all(&rel).unwrap();
        let path = {
            let w = WorkDir::create(&rel).unwrap();
            assert!(w.path.is_absolute(), "{:?}", w.path);
            assert!(w.path.is_dir());
            w.path.clone()
        };
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&rel);
    }

    fn scratch_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("t2f-ffwhich-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Fichier exécutable (bit `x` requis par `access_check` sous Unix).
    fn write_exe(path: &Path) {
        std::fs::write(path, b"x").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn list(dirs: &[&Path]) -> Option<OsString> {
        Some(std::env::join_paths(dirs).unwrap())
    }

    /// `which` : premier dossier du `PATH` gagnant, dossier vide ou introuvable.
    #[test]
    fn which_ordre_du_path() {
        let (a, b) = (scratch_dir("ordre-a"), scratch_dir("ordre-b"));
        let name = if cfg!(windows) { "outil.exe" } else { "outil" };
        write_exe(&b.join(name));
        let pathext = Some(OsString::from(".EXE"));
        let both = list(&[&a, &b]);
        assert_eq!(
            which_in("outil", both.clone(), true, pathext.clone()),
            Some(b.join(name.replace(".exe", ".EXE")))
        );
        write_exe(&a.join(name));
        assert_eq!(
            which_in("outil", both, true, pathext.clone()),
            Some(a.join(name.replace(".exe", ".EXE")))
        );
        assert_eq!(
            which_in("outil", Some(OsString::new()), true, pathext),
            None
        );
        assert_eq!(
            which_in("outil", list(&[&b.join("absent")]), true, None),
            None
        );
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }

    /// Windows : les extensions de `PATHEXT` sont essayées dans l'ordre, le nom déjà terminé
    /// par l'une d'elles d'abord.
    #[cfg(windows)]
    #[test]
    fn which_ordre_de_pathext() {
        let d = scratch_dir("pathext");
        std::fs::write(d.join("outil.cmd"), b"x").unwrap();
        std::fs::write(d.join("outil.exe"), b"x").unwrap();
        let dirs = list(&[&d]);
        let found = |ext: &str, cmd: &str| {
            which_in(cmd, dirs.clone(), true, Some(OsString::from(ext)))
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        };
        assert_eq!(found(".CMD;.EXE", "outil").as_deref(), Some("outil.CMD"));
        assert_eq!(found(".EXE;.CMD", "outil").as_deref(), Some("outil.EXE"));
        // Extension vide ignorée ; point final retiré (`.EXE.`).
        assert_eq!(found(";.EXE.;", "outil").as_deref(), Some("outil.EXE"));
        // Nom déjà terminé par une extension de PATHEXT : essayé tel quel en premier.
        assert_eq!(
            found(".CMD;.EXE", "outil.exe").as_deref(),
            Some("outil.exe")
        );
        // PATHEXT vide : valeur par défaut de Python.
        assert_eq!(found("", "outil").as_deref(), Some("outil.EXE"));
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Windows : le dossier courant est cherché en premier, sauf avec
    /// `NoDefaultCurrentDirectoryInExePath` ; une entrée vide du `PATH` désigne le dossier
    /// courant dans tous les cas (le dossier du paquet, `Cargo.toml`, sert de témoin).
    #[test]
    fn which_dossier_courant() {
        let other = scratch_dir("cwd");
        let pathext = Some(OsString::from(".TOML"));
        let in_cwd = which_in("Cargo.toml", list(&[&other]), false, pathext.clone());
        let empty_entry = {
            let mut p = OsString::from(if cfg!(windows) { ";" } else { ":" });
            p.push(other.as_os_str());
            which_in("Cargo.toml", Some(p), true, pathext.clone())
        };
        let skipped = which_in("Cargo.toml", list(&[&other]), true, pathext);
        if cfg!(windows) {
            assert_eq!(in_cwd, Some(PathBuf::from(".").join("Cargo.toml")));
            assert_eq!(skipped, None);
            assert_eq!(empty_entry, Some(PathBuf::from("Cargo.toml")));
        } else {
            // Sans PATHEXT : le dossier courant n'est pas ajouté, l'entrée vide le désigne.
            assert_eq!(in_cwd, None);
            assert_eq!(empty_entry, None, "Cargo.toml n'est pas exécutable");
        }
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn dossier_de_travail_supprime() {
        let root = std::env::temp_dir().join(format!("t2f-ffwork-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let path = {
            let w = WorkDir::create(&root).unwrap();
            std::fs::write(w.path.join("in.mp3"), b"x").unwrap();
            assert!(w
                .path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("t2f_ff_"));
            w.path.clone()
        };
        assert!(!path.exists());
        // Exécutable absent : raison « inutilisable », aucun dossier laissé.
        let r = run_ffmpeg(
            &root.join("absent.exe"),
            b"x",
            128,
            &root,
            Duration::from_secs(5),
            &|| false,
        )
        .unwrap();
        assert!(r.unwrap_err().starts_with("ffmpeg inutilisable ("));
        assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        // Annulation avant le lancement.
        let e = run_ffmpeg(
            &root.join("absent.exe"),
            b"x",
            128,
            &root,
            Duration::from_secs(5),
            &|| true,
        )
        .unwrap_err();
        assert_eq!(e.code, "cancelled");
        let _ = std::fs::remove_dir_all(&root);
    }
}
