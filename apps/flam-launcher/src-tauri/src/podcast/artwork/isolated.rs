//! Décodage des pochettes dans un processus enfant (#263).
//!
//! Le profil release du workspace a `panic = "abort"` : un panic d'un décodeur d'image
//! (`zune-jpeg`, `image-webp`, `png`) sur une pochette hostile arrêterait tout le launcher, et
//! `catch_unwind` est alors inopérant. Le profil `panic` ne se règle que pour tout le workspace
//! (le launcher et `telmi2flam` partagent le profil racine), d'où le choix d'un **processus
//! enfant** : le launcher se relance lui-même avec [`DECODE_ARG`] (voir `main.rs`), lui envoie
//! les octets sur son entrée standard et lit le PNG sur sa sortie. Si l'enfant meurt (panic,
//! abandon, mémoire) ou dépasse [`CHILD_TIMEOUT`], l'appelant reçoit
//! [`ArtworkError::Isolated`] et le launcher continue (pochette de repli, voir
//! [`Artwork::fallback`]).
//!
//! # Protocole
//!
//! Entrée : 1 octet de version ([`PROTOCOL_VERSION`]), 1 octet d'usage (0 épisode, 1
//! couverture), 3 octets de fond, puis l'image jusqu'à la fin du flux. Sortie : l'octet de
//! version, un octet de statut (0 = succès, sinon [`ArtworkError`] voir `Tag`) suivi de la
//! charge : largeur et hauteur (u32 petit-boutiste) puis le PNG, ou la
//! taille (u64) / les dimensions (2 × u32) / le message UTF-8 de l'erreur.
//!
//! Les garde-fous de taille, de dimensions et de mémoire de [`Artwork::convert_on`] s'appliquent
//! dans l'enfant ; la taille d'entrée est aussi refusée côté parent, avant tout lancement.
//!
//! # Confinement
//!
//! Sous Windows, l'enfant est rattaché à un Job Object (plafond mémoire de 512 Mio, arrêt à la
//! fermeture du job, donc aussi de ses éventuels descendants) et ne déclenche pas la fenêtre de
//! rapport d'erreurs de Windows s'il s'arrête.

use std::io::{self, Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use super::{Artwork, ArtworkError, ArtworkKind, FLATTEN_BACKGROUND, MAX_INPUT_BYTES};

/// Argument de ligne de commande qui fait du launcher un décodeur de pochette.
pub const DECODE_ARG: &str = "--decode-artwork";
/// Durée maximale d'un décodage : au-delà, l'enfant est tué.
pub const CHILD_TIMEOUT: Duration = Duration::from_secs(30);
/// Version du protocole stdin/stdout, en tête de la requête et de la réponse : un parent et un
/// enfant de versions différentes se refusent proprement au lieu de mal s'interpréter.
const PROTOCOL_VERSION: u8 = 1;
/// Plafond mémoire de l'enfant (Job Object, Windows) : au-dessus du budget de décodage (256 Mio)
/// et de l'entrée (20 Mio).
#[cfg(windows)]
const CHILD_MEMORY_LIMIT: usize = 512 * 1024 * 1024;
/// Attente maximale de la lecture de la réponse une fois l'enfant terminé : un autre processus
/// qui aurait hérité du tube ne doit pas bloquer l'appelant.
const IO_GRACE: Duration = Duration::from_secs(2);
/// Taille maximale de la réponse lue (un PNG 640 × 480 non compressé fait moins de 1 Mio).
const MAX_OUTPUT_BYTES: u64 = 8 * 1024 * 1024;
/// Intervalle de surveillance de l'enfant.
const POLL: Duration = Duration::from_millis(10);

/// Statut de la réponse de l'enfant.
#[derive(Clone, Copy)]
#[repr(u8)]
enum Tag {
    Ok = 0,
    Empty = 1,
    TooLarge = 2,
    Unsupported = 3,
    Dimensions = 4,
    Memory = 5,
    Decode = 6,
    Encode = 7,
}

impl Tag {
    fn from_byte(b: u8) -> Option<Self> {
        Some(match b {
            0 => Self::Ok,
            1 => Self::Empty,
            2 => Self::TooLarge,
            3 => Self::Unsupported,
            4 => Self::Dimensions,
            5 => Self::Memory,
            6 => Self::Decode,
            7 => Self::Encode,
            _ => return None,
        })
    }
}

/// Confinement de l'enfant : Job Object Windows (plafond mémoire, arrêt à la fermeture). Ailleurs,
/// l'arrêt se limite au processus direct. Le `Job` du convertisseur (`install/convert.rs`) est
/// lié à `tokio::process::Child` et sans plafond mémoire ; celui-ci suit le même schéma pour
/// `std::process::Child`.
#[cfg(windows)]
mod job {
    use std::os::windows::io::AsRawHandle;
    use std::process::Child;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
    };

    pub struct Job(HANDLE);

    impl Job {
        /// Crée un job `KILL_ON_JOB_CLOSE` plafonné à `memory` octets par processus et y rattache
        /// `child`. `None` si une étape échoue : l'enfant tourne alors sans confinement (échec
        /// ouvert assumé, le confinement est une défense en profondeur ; le délai reste appliqué).
        pub fn attach(child: &Child, memory: usize) -> Option<Self> {
            // SAFETY : appels Win32 sur des poignées valides ; structure initialisée à zéro.
            unsafe {
                let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
                if handle.is_null() {
                    return None;
                }
                let job = Job(handle);
                let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
                info.BasicLimitInformation.LimitFlags =
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = memory;
                let ok = SetInformationJobObject(
                    handle,
                    JobObjectExtendedLimitInformation,
                    std::ptr::addr_of!(info).cast(),
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
                if ok == 0 || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0 {
                    return None;
                }
                Some(job)
            }
        }

        pub fn terminate(&self) {
            // SAFETY : poignée valide tant que `self` existe.
            unsafe {
                TerminateJobObject(self.0, 1);
            }
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY : poignée possédée, fermée une seule fois (tue le job s'il vit encore).
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Enfant et son confinement.
struct Confined {
    child: Child,
    #[cfg(windows)]
    job: Option<job::Job>,
}

impl Confined {
    fn new(child: Child) -> Self {
        #[cfg(windows)]
        let job = job::Job::attach(&child, CHILD_MEMORY_LIMIT);
        Self {
            child,
            #[cfg(windows)]
            job,
        }
    }

    /// Tue l'enfant (et son job) et le récupère.
    fn kill(&mut self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn kind_byte(kind: ArtworkKind) -> u8 {
    match kind {
        ArtworkKind::Episode => 0,
        ArtworkKind::Cover => 1,
    }
}

fn kind_from_byte(b: u8) -> Option<ArtworkKind> {
    match b {
        0 => Some(ArtworkKind::Episode),
        1 => Some(ArtworkKind::Cover),
        _ => None,
    }
}

/// Convertit une pochette dans un processus enfant : le launcher lui-même lancé avec
/// [`DECODE_ARG`]. Bloquant (à appeler depuis `spawn_blocking`). Voir
/// [`convert_isolated_with`].
pub fn convert_isolated(
    bytes: &[u8],
    kind: ArtworkKind,
    background: Option<[u8; 3]>,
) -> Result<Artwork, ArtworkError> {
    let exe = std::env::current_exe().map_err(|e| ArtworkError::Isolated(e.to_string()))?;
    let mut cmd = Command::new(exe);
    cmd.arg(DECODE_ARG);
    convert_isolated_with(cmd, bytes, kind, background, CHILD_TIMEOUT)
}

/// Comme [`convert_isolated`], avec la commande de l'enfant et le délai au choix (tests).
/// `background` absent : [`FLATTEN_BACKGROUND`].
pub fn convert_isolated_with(
    mut cmd: Command,
    bytes: &[u8],
    kind: ArtworkKind,
    background: Option<[u8; 3]>,
    timeout: Duration,
) -> Result<Artwork, ArtworkError> {
    if bytes.is_empty() {
        return Err(ArtworkError::Empty);
    }
    if bytes.len() > MAX_INPUT_BYTES {
        return Err(ArtworkError::TooLarge(bytes.len()));
    }
    let isolated =
        |what: &str, e: &dyn std::fmt::Display| ArtworkError::Isolated(format!("{what} : {e}"));
    let bg = background.unwrap_or(FLATTEN_BACKGROUND);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| isolated("lancement du décodeur", &e))?;
    let mut stdin = child.stdin.take().expect("entrée en tube");
    let stdout = child.stdout.take().expect("sortie en tube");
    // Rattaché avant d'écrire l'entrée : l'enfant n'a encore rien à décoder.
    let mut child = Confined::new(child);

    let mut input = Vec::with_capacity(bytes.len() + 5);
    input.push(PROTOCOL_VERSION);
    input.push(kind_byte(kind));
    input.extend_from_slice(&bg);
    input.extend_from_slice(bytes);
    // Écriture et lecture dans des fils : l'enfant peut remplir son tube de sortie avant d'avoir
    // fini de lire son entrée, et un enfant mort ferme les deux tubes (le fil se termine).
    // Aucun des deux n'est joint : si un autre processus a hérité d'un tube, le fil reste
    // bloqué mais l'appelant n'attend que `IO_GRACE` (le fil se termine avec le tube).
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
        // `stdin` est fermé ici : fin de flux pour l'enfant.
    });
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut out = Vec::new();
        let r = stdout
            .take(MAX_OUTPUT_BYTES + 1)
            .read_to_end(&mut out)
            .map(|_| out);
        let _ = tx.send(r);
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() >= deadline => {
                child.kill();
                break Err(ArtworkError::Isolated(format!(
                    "décodeur arrêté après {} s",
                    timeout.as_secs()
                )));
            }
            Ok(None) => thread::sleep(POLL),
            Err(e) => {
                child.kill();
                break Err(isolated("attente du décodeur", &e));
            }
        }
    };
    let status = status?;
    if !status.success() {
        return Err(ArtworkError::Isolated(format!(
            "décodeur interrompu ({status})"
        )));
    }
    // Un descendant qui garde le tube ouvert : on le tue avec le job, puis on attend encore un
    // court instant la fin de la lecture.
    let output = match rx.recv_timeout(IO_GRACE) {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(isolated("lecture de la réponse", &e)),
        Err(_) => {
            child.kill();
            rx.recv_timeout(IO_GRACE)
                .ok()
                .and_then(Result::ok)
                .ok_or_else(|| isolated("lecture de la réponse", &"délai dépassé"))?
        }
    };
    parse_response(&output, kind)
}

/// Lit la réponse de l'enfant.
fn parse_response(out: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
    let bad = |what: &str| ArtworkError::Isolated(format!("réponse du décodeur invalide : {what}"));
    if out.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(bad("trop longue"));
    }
    let (&version, out) = out.split_first().ok_or_else(|| bad("vide"))?;
    if version != PROTOCOL_VERSION {
        return Err(bad("version de protocole inconnue"));
    }
    let (&status, rest) = out.split_first().ok_or_else(|| bad("tronquée"))?;
    let tag = Tag::from_byte(status).ok_or_else(|| bad("statut inconnu"))?;
    let u32_at = |i: usize| -> Result<u32, ArtworkError> {
        rest.get(i..i + 4)
            .map(|b| u32::from_le_bytes(b.try_into().expect("4 octets")))
            .ok_or_else(|| bad("tronquée"))
    };
    let u64_at = || -> Result<u64, ArtworkError> {
        rest.get(..8)
            .map(|b| u64::from_le_bytes(b.try_into().expect("8 octets")))
            .ok_or_else(|| bad("tronquée"))
    };
    let message = || String::from_utf8_lossy(rest).into_owned();
    match tag {
        Tag::Ok => {
            let (width, height) = (u32_at(0)?, u32_at(4)?);
            let png = rest
                .get(8..)
                .filter(|p| !p.is_empty())
                .ok_or_else(|| bad("PNG absent"))?;
            // L'enfant traite l'entrée hostile : sa réponse est revérifiée (taille de l'usage, PNG).
            let (tw, th) = kind.size();
            if (width, height) != (tw, th) || !png.starts_with(&[0x89, b'P', b'N', b'G']) {
                return Err(bad("image inattendue"));
            }
            Ok(Artwork {
                width,
                height,
                png: png.to_vec(),
            })
        }
        Tag::Empty => Err(ArtworkError::Empty),
        Tag::TooLarge => Err(ArtworkError::TooLarge(
            usize::try_from(u64_at()?).unwrap_or(usize::MAX),
        )),
        Tag::Unsupported => Err(ArtworkError::UnsupportedFormat),
        Tag::Dimensions => Err(ArtworkError::Dimensions(u32_at(0)?, u32_at(4)?)),
        Tag::Memory => Err(ArtworkError::Memory(u64_at()?)),
        Tag::Decode => Err(ArtworkError::Decode(message())),
        Tag::Encode => Err(ArtworkError::Encode(message())),
    }
}

/// Écrit la réponse pour `result`.
fn write_response(out: &mut impl Write, result: Result<Artwork, ArtworkError>) -> io::Result<()> {
    let mut buf = vec![PROTOCOL_VERSION];
    match result {
        Ok(a) => {
            buf.push(Tag::Ok as u8);
            buf.extend_from_slice(&a.width.to_le_bytes());
            buf.extend_from_slice(&a.height.to_le_bytes());
            buf.extend_from_slice(&a.png);
        }
        Err(e) => match e {
            ArtworkError::Empty => buf.push(Tag::Empty as u8),
            ArtworkError::TooLarge(n) => {
                buf.push(Tag::TooLarge as u8);
                buf.extend_from_slice(&(n as u64).to_le_bytes());
            }
            ArtworkError::UnsupportedFormat => buf.push(Tag::Unsupported as u8),
            ArtworkError::Dimensions(w, h) => {
                buf.push(Tag::Dimensions as u8);
                buf.extend_from_slice(&w.to_le_bytes());
                buf.extend_from_slice(&h.to_le_bytes());
            }
            ArtworkError::Memory(n) => {
                buf.push(Tag::Memory as u8);
                buf.extend_from_slice(&n.to_le_bytes());
            }
            ArtworkError::Decode(m) => {
                buf.push(Tag::Decode as u8);
                buf.extend_from_slice(m.as_bytes());
            }
            // Une erreur d'isolation n'est produite que par le parent ; si elle arrivait ici,
            // elle serait un échec de décodage.
            ArtworkError::Encode(m) | ArtworkError::Isolated(m) => {
                buf.push(Tag::Encode as u8);
                buf.extend_from_slice(m.as_bytes());
            }
        },
    }
    out.write_all(&buf)?;
    out.flush()
}

/// Corps de l'enfant, sur des flux quelconques : lit la requête, décode avec `decode`, écrit la
/// réponse. `decode` est paramétré pour qu'un test simule un panic de décodeur.
fn serve(
    input: &mut impl Read,
    output: &mut impl Write,
    decode: impl Fn(&[u8], ArtworkKind, [u8; 3]) -> Result<Artwork, ArtworkError>,
) -> io::Result<()> {
    // Une lecture de plus que la borne : une entrée trop grande est refusée sans tout lire.
    let mut req = Vec::new();
    input
        .take(MAX_INPUT_BYTES as u64 + 6)
        .read_to_end(&mut req)?;
    let result = match req.as_slice() {
        [v, ..] if *v != PROTOCOL_VERSION => {
            Err(ArtworkError::Decode("version de protocole inconnue".into()))
        }
        [_, k, r, g, b, image @ ..] => match kind_from_byte(*k) {
            Some(kind) => decode(image, kind, [*r, *g, *b]),
            None => Err(ArtworkError::Decode("requête invalide".into())),
        },
        _ => Err(ArtworkError::Decode("requête incomplète".into())),
    };
    write_response(output, result)
}

/// Pas de fenêtre de rapport d'erreurs Windows si l'enfant s'arrête (abandon du profil release,
/// mémoire refusée) : elle resterait affichée jusqu'au délai.
fn quiet_crashes() {
    #[cfg(windows)]
    // SAFETY : réglage du mode d'erreur du processus courant, sans pointeur.
    unsafe {
        use windows_sys::Win32::System::Diagnostics::Debug::{
            SetErrorMode, SEM_FAILCRITICALERRORS, SEM_NOGPFAULTERRORBOX,
        };
        SetErrorMode(SEM_NOGPFAULTERRORBOX | SEM_FAILCRITICALERRORS);
    }
}

/// Point d'entrée de l'enfant (`--decode-artwork`) : entrée et sortie standard. Rend le code
/// de sortie du processus (0 : réponse écrite, y compris pour une image refusée).
pub fn run_child() -> i32 {
    quiet_crashes();
    // Garde-fou : si le parent disparaît, l'enfant ne tourne pas indéfiniment.
    thread::spawn(|| {
        thread::sleep(CHILD_TIMEOUT + Duration::from_secs(5));
        std::process::exit(3);
    });
    let stdin = io::stdin();
    let stdout = io::stdout();
    match serve(&mut stdin.lock(), &mut stdout.lock(), Artwork::convert_on) {
        Ok(()) => 0,
        Err(_) => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    const JPEG: &[u8] = include_bytes!("../../../tests/fixtures/progressif.jpg");

    /// Variable d'environnement qui transforme le test `enfant` en faux décodeur.
    const CHILD_ENV: &str = "FLAM_ARTWORK_TEST_CHILD";

    /// Corps du faux enfant, lancé par [`child_command`] : ne fait rien hors de ce mode.
    #[test]
    fn enfant() {
        let Ok(mode) = std::env::var(CHILD_ENV) else {
            return;
        };
        // Comme `run_child` : un abandon ne doit pas ouvrir la fenêtre de rapport d'erreurs.
        quiet_crashes();
        match mode.as_str() {
            // panic d'un décodeur : en profil test le panic est rattrapé par le harnais (sortie 101),
            // l'abandon du profil release est couvert par le mode `abort`.
            "panic" => {
                let _ = serve(
                    &mut io::stdin().lock(),
                    &mut io::stdout().lock(),
                    |_, _, _| panic!("panic simulé du décodeur"),
                );
            }
            "abort" => std::process::abort(),
            "sleep" => thread::sleep(Duration::from_secs(60)),
            "sleep_court" => thread::sleep(Duration::from_secs(8)),
            "alloc" => {
                // Dépasse le plafond du job : l'allocation échoue, donc abandon.
                let mut v = vec![1u8; 900 * 1024 * 1024];
                v[12345] = 2;
                std::hint::black_box(&v);
            }
            "heritier" => {
                // petit-enfant dormant qui hérite des tubes, puis l'enfant sort sans réponse
                let _ = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", "podcast::artwork::isolated::tests::enfant"])
                    .env(CHILD_ENV, "sleep_court")
                    .spawn();
            }
            "garbage" => {
                let _ = io::stdout().write_all(b"pas une reponse");
            }
            _ => {}
        }
        std::process::exit(0);
    }

    /// Relance ce binaire de test sur le seul test `enfant`, dans le mode `mode`.
    fn child_command(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args([
            "--exact",
            "podcast::artwork::isolated::tests::enfant",
            "--nocapture",
        ])
        .env(CHILD_ENV, mode);
        cmd
    }

    fn run(mode: &str, timeout: Duration) -> Result<Artwork, ArtworkError> {
        convert_isolated_with(
            child_command(mode),
            JPEG,
            ArtworkKind::Episode,
            None,
            timeout,
        )
    }

    #[test]
    fn un_panic_de_decodeur_n_arrete_pas_l_appelant() {
        match run("panic", Duration::from_secs(60)) {
            Err(ArtworkError::Isolated(m)) => assert!(m.contains("interrompu"), "{m}"),
            other => panic!("attendu Isolated : {other:?}"),
        }
    }

    #[test]
    fn un_abandon_du_decodeur_est_une_erreur() {
        assert!(matches!(
            run("abort", Duration::from_secs(60)),
            Err(ArtworkError::Isolated(_))
        ));
    }

    #[test]
    fn un_decodeur_trop_lent_est_tue() {
        let start = Instant::now();
        match run("sleep", Duration::from_millis(300)) {
            Err(ArtworkError::Isolated(m)) => assert!(m.contains("arrêté"), "{m}"),
            other => panic!("attendu Isolated : {other:?}"),
        }
        assert!(start.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn une_reponse_illisible_est_refusee() {
        assert!(matches!(
            run("garbage", Duration::from_secs(60)),
            Err(ArtworkError::Isolated(m)) if m.contains("invalide")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn le_plafond_memoire_du_job_arrete_l_enfant() {
        assert!(matches!(
            run("alloc", Duration::from_secs(60)),
            Err(ArtworkError::Isolated(m)) if m.contains("interrompu")
        ));
    }

    #[test]
    fn un_descendant_qui_garde_le_tube_ne_bloque_pas() {
        let start = Instant::now();
        let r = run("heritier", Duration::from_secs(60));
        assert!(matches!(r, Err(ArtworkError::Isolated(_))), "{r:?}");
        assert!(start.elapsed() < Duration::from_secs(20));
    }

    #[test]
    fn un_binaire_absent_est_une_erreur() {
        let cmd = Command::new("flam-decodeur-inexistant-xyz");
        let r = convert_isolated_with(cmd, JPEG, ArtworkKind::Cover, None, CHILD_TIMEOUT);
        assert!(matches!(r, Err(ArtworkError::Isolated(_))));
    }

    #[test]
    fn entrees_refusees_sans_lancer_d_enfant() {
        let cmd = || Command::new("flam-decodeur-inexistant-xyz");
        assert!(matches!(
            convert_isolated_with(cmd(), &[], ArtworkKind::Cover, None, CHILD_TIMEOUT),
            Err(ArtworkError::Empty)
        ));
        let big = vec![0u8; MAX_INPUT_BYTES + 1];
        assert!(matches!(
            convert_isolated_with(cmd(), &big, ArtworkKind::Cover, None, CHILD_TIMEOUT),
            Err(ArtworkError::TooLarge(n)) if n == MAX_INPUT_BYTES + 1
        ));
    }

    /// Aller-retour du protocole, sans processus : image valide, puis chaque erreur.
    #[test]
    fn protocole_aller_retour() {
        let mut req = vec![PROTOCOL_VERSION, 1u8, 0xff, 0xff, 0xff];
        req.extend_from_slice(JPEG);
        let mut out = Vec::new();
        serve(&mut Cursor::new(req), &mut out, Artwork::convert_on).unwrap();
        let art = parse_response(&out, ArtworkKind::Cover).unwrap();
        let direct = Artwork::convert_on(JPEG, ArtworkKind::Cover, [0xff; 3]).unwrap();
        assert_eq!(art, direct);
        assert_eq!((art.width, art.height), (512, 512));

        let errors = [
            ArtworkError::Empty,
            ArtworkError::TooLarge(42),
            ArtworkError::UnsupportedFormat,
            ArtworkError::Dimensions(9000, 3),
            ArtworkError::Memory(1 << 40),
            ArtworkError::Decode("é cassé".into()),
            ArtworkError::Encode("plein".into()),
        ];
        for e in errors {
            let shown = e.to_string();
            let mut out = Vec::new();
            write_response(&mut out, Err(e)).unwrap();
            let back = parse_response(&out, ArtworkKind::Cover).unwrap_err();
            assert_eq!(back.to_string(), shown);
        }
    }

    #[test]
    fn requetes_et_reponses_malformees() {
        for req in [
            &[][..],
            &[1, 0, 1, 2][..],
            &[1, 9, 0, 0, 0, 1, 2][..],
            &[2, 0, 0, 0, 0, 1, 2][..],
        ] {
            let mut out = Vec::new();
            serve(
                &mut Cursor::new(req.to_vec()),
                &mut out,
                Artwork::convert_on,
            )
            .unwrap();
            assert!(matches!(
                parse_response(&out, ArtworkKind::Cover),
                Err(ArtworkError::Decode(_))
            ));
        }
        // image refusée par le décodeur : erreur transmise telle quelle
        let mut out = Vec::new();
        serve(
            &mut Cursor::new(vec![1, 0, 0, 0, 0, b'x', b'y']),
            &mut out,
            Artwork::convert_on,
        )
        .unwrap();
        assert!(matches!(
            parse_response(&out, ArtworkKind::Cover),
            Err(ArtworkError::UnsupportedFormat)
        ));
        for out in [
            &[][..],
            &[1][..],
            &[1, 99][..],
            &[1, 4, 1][..],
            &[1, 0, 1, 0, 0, 0][..],
            &[2, 1][..],
            &[0, 1][..],
        ] {
            assert!(matches!(
                parse_response(out, ArtworkKind::Cover),
                Err(ArtworkError::Isolated(_))
            ));
        }
    }
}
