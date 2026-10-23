//! Conversion audio des épisodes de podcast par le ffmpeg embarqué (#214, épopée #207).
//!
//! Règle **D6** (`docs/podcasts.md`) :
//!
//! - **copie** si le fichier est un MP3 à 44 100 Hz, mono ou stéréo, entre 64 et 192 kbps :
//!   les trames audio sont recopiées sans réencodage (`-c:a copy`, identiques à la source) ;
//! - sinon **réencodage** en MP3 44 100 Hz **stéréo**, au débit source arrondi au multiple de
//!   16 kbps supérieur, borné à [64, 192] kbps (192 si le débit est inconnu). Le débit annoncé
//!   par ffprobe (bits/s, estimé) est d'abord arrondi **au plus proche** en kbps, puis au
//!   multiple de 16 supérieur : 128,4 kbps donne 128 puis 128, 129 kbps donne 144 ;
//! - métadonnées (ID3), chapitres et flux vidéo (pochettes intégrées) retirés dans les deux cas.
//!
//! Précautions de #366 : les chemins passés à ffmpeg et ffprobe sont préfixés `file:` (jamais
//! interprétés comme option, `-`, `pipe:`, `concat:`…) avec `-protocol_whitelist file` ; la
//! sortie est écrite dans un temporaire voisin, renommé seulement en cas de succès, et le
//! temporaire est supprimé sur échec, annulation ou délai. Si l'appel est abandonné en cours de
//! route (futur détruit), le temporaire peut subsister sous Windows (ffmpeg le tient encore
//! ouvert) : chaque conversion supprime d'abord les `.part` laissés à côté de sa sortie depuis
//! plus de [`STALE_PART_AGE`] (un `.part` récent peut appartenir à une conversion en cours).
//! Règle : **une conversion à la fois par fichier de sortie** ; [`convert_all`] refuse les
//! sorties en double (`duplicate_output`). Après conversion, la sortie est sondée : une durée
//! nulle ou inconnue (MP3 réduit à son en-tête) est refusée (`ffmpeg_failed`).
//!
//! Erreurs par épisode : `AppError::Conversion` avec un code stable (`unreadable_audio`,
//! `no_audio`, `ffmpeg_failed`, `duplicate_output`), `AppError::Cancelled`, ou `AppError::Io`
//! de genre `TimedOut` ou `NotFound` (dossier de sortie absent).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tokio::sync::{watch, Semaphore};

use crate::ffmpeg::{Ffmpeg, Limits, Tool};
use crate::{AppError, AppResult};

/// Échantillonnage attendu par le player.
pub const TARGET_SAMPLE_RATE: u32 = 44_100;
/// Bornes de débit (kbps) de la copie et du réencodage.
pub const MIN_KBPS: u32 = 64;
pub const MAX_KBPS: u32 = 192;
/// Nombre maximal de conversions simultanées dans [`convert_all`].
pub const MAX_PARALLEL: usize = 2;

const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// Délai d'une conversion dont la durée est inconnue.
const UNKNOWN_DURATION_TIMEOUT: Duration = Duration::from_secs(600);
const MIN_CONVERT_TIMEOUT: Duration = Duration::from_secs(120);
/// Plafond du délai de conversion, quelle que soit la durée annoncée par la source (non fiable).
const MAX_CONVERT_TIMEOUT: Duration = Duration::from_secs(6 * 3600);
/// Âge au-delà duquel un `.part` voisin de la sortie est considéré comme abandonné.
pub const STALE_PART_AGE: Duration = Duration::from_secs(10 * 60);

/// Ce que ffprobe dit du premier flux audio d'un fichier.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SourceInfo {
    /// Nom du codec (`mp3`, `aac`, `opus`…).
    pub codec: String,
    /// Le conteneur est un fichier MP3 (et non du MP3 dans un MP4, par exemple).
    pub container_is_mp3: bool,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    /// Débit en bits par seconde (flux, sinon conteneur) ; `None` si inconnu.
    pub bit_rate: Option<u64>,
    /// Le fichier contient un flux vidéo (pochette intégrée).
    pub has_video: bool,
    pub duration_secs: Option<f64>,
}

/// Décision de la règle D6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// MP3 conforme : trames audio recopiées sans réencodage, étiquettes, chapitres et pochette
    /// retirés.
    Copy,
    /// Réencodage en MP3 44 100 Hz stéréo à ce débit (kbps).
    Transcode { kbps: u32 },
}

/// Débit en kbps, arrondi au plus proche (ffprobe donne des bits par seconde estimés ; un
/// débit variable ou mal annoncé peut être légèrement au-dessus ou au-dessous).
fn kbps_of(bit_rate: Option<u64>) -> Option<u32> {
    bit_rate
        .filter(|&b| b > 0)
        .map(|b| u32::try_from(b.saturating_add(500) / 1000).unwrap_or(u32::MAX))
}

/// Débit de réencodage : débit source arrondi au plus proche (kbps, voir `kbps_of`) puis au
/// multiple de 16 supérieur, borné à [64, 192] ; 192 si inconnu.
pub fn transcode_kbps(bit_rate: Option<u64>) -> u32 {
    match kbps_of(bit_rate) {
        None => MAX_KBPS,
        Some(k) => (k.div_ceil(16).saturating_mul(16)).clamp(MIN_KBPS, MAX_KBPS),
    }
}

/// Règle D6 : copie ou réencodage.
pub fn decide(info: &SourceInfo) -> Plan {
    let conforming = info.codec == "mp3"
        && info.container_is_mp3
        && info.sample_rate == Some(TARGET_SAMPLE_RATE)
        && matches!(info.channels, Some(1 | 2))
        && kbps_of(info.bit_rate).is_some_and(|k| (MIN_KBPS..=MAX_KBPS).contains(&k));
    if conforming {
        Plan::Copy
    } else {
        Plan::Transcode {
            kbps: transcode_kbps(info.bit_rate),
        }
    }
}

/// Argument de chemin pour ffmpeg/ffprobe : préfixe `file:`, pour qu'un chemin ne soit jamais
/// pris pour une option ni pour un protocole (`-`, `pipe:`, `concat:`, `subfile:`…).
pub fn file_arg(path: &Path) -> OsString {
    let mut arg = OsString::from("file:");
    arg.push(path.as_os_str());
    arg
}

/// Mesures d'une conversion réussie.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    pub plan: Plan,
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub duration_secs: Option<f64>,
    /// Temps passé (sonde comprise).
    pub elapsed: Duration,
}

// --- ffprobe ---------------------------------------------------------------------------------

#[derive(Deserialize, Default)]
struct ProbeJson {
    #[serde(default)]
    streams: Vec<ProbeStream>,
    format: Option<ProbeFormat>,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    bit_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    format_name: Option<String>,
    bit_rate: Option<String>,
    duration: Option<String>,
}

fn unreadable(why: impl Into<String>) -> AppError {
    AppError::Conversion {
        code: "unreadable_audio".into(),
        message: why.into(),
    }
}

/// Analyse la sortie JSON de ffprobe.
fn parse_probe(json: &[u8]) -> AppResult<SourceInfo> {
    let parsed: ProbeJson =
        serde_json::from_slice(json).map_err(|_| unreadable("réponse de ffprobe illisible"))?;
    let audio = parsed
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .ok_or_else(|| AppError::Conversion {
            code: "no_audio".into(),
            message: "le fichier ne contient aucun flux audio".into(),
        })?;
    let num = |s: &Option<String>| s.as_deref().and_then(|v| v.trim().parse::<u64>().ok());
    let secs = |s: &Option<String>| {
        s.as_deref()
            .and_then(|v| v.trim().parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d >= 0.0)
    };
    let format = parsed.format.as_ref();
    Ok(SourceInfo {
        codec: audio.codec_name.clone().unwrap_or_default(),
        container_is_mp3: format
            .and_then(|f| f.format_name.as_deref())
            .is_some_and(|n| n.split(',').any(|p| p == "mp3")),
        sample_rate: num(&audio.sample_rate).and_then(|v| u32::try_from(v).ok()),
        channels: audio.channels,
        bit_rate: num(&audio.bit_rate)
            .filter(|&b| b > 0)
            .or_else(|| format.and_then(|f| num(&f.bit_rate)).filter(|&b| b > 0)),
        has_video: parsed
            .streams
            .iter()
            .any(|s| s.codec_type.as_deref() == Some("video")),
        duration_secs: secs(&audio.duration).or_else(|| format.and_then(|f| secs(&f.duration))),
    })
}

/// Sonde `input` avec ffprobe.
pub async fn probe(
    ff: &Ffmpeg,
    input: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<SourceInfo> {
    let mut args: Vec<OsString> = [
        "-v",
        "error",
        "-protocol_whitelist",
        "file",
        "-show_entries",
        "stream=codec_type,codec_name,sample_rate,channels,bit_rate,duration:format=format_name,bit_rate,duration",
        "-of",
        "json",
        "-i",
    ]
    .map(OsString::from)
    .to_vec();
    args.push(file_arg(input));
    let out = ff
        .run(Tool::Ffprobe, &args, Limits::new(PROBE_TIMEOUT), cancel)
        .await?;
    if !out.success() {
        return Err(unreadable(format!(
            "fichier audio illisible : {}",
            last_line(&out.stderr)
        )));
    }
    parse_probe(&out.stdout)
}

fn last_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("aucun détail");
    line.chars().take(300).collect()
}

// --- conversion ------------------------------------------------------------------------------

/// Temporaire de sortie : supprimé à la destruction sauf s'il a été renommé.
struct TempOutput {
    path: PathBuf,
    armed: bool,
}

impl TempOutput {
    async fn beside(output: &Path) -> Self {
        let mut name = output.file_name().unwrap_or_default().to_os_string();
        remove_stale_parts(output, STALE_PART_AGE).await;
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        name.push(format!(".{}-{n}.part", std::process::id()));
        Self {
            path: output.with_file_name(name),
            armed: true,
        }
    }

    async fn commit(&mut self, output: &Path) -> std::io::Result<()> {
        tokio::fs::rename(&self.path, output).await?;
        self.armed = false;
        Ok(())
    }
}

/// Supprime les `<sortie>.<…>.part` modifiés il y a plus de `max_age` (conversion abandonnée vers
/// la même sortie). Les `.part` récents, ou d'une autre sortie, sont conservés.
async fn remove_stale_parts(output: &Path, max_age: Duration) {
    let (Some(dir), Some(name)) = (output.parent(), output.file_name()) else {
        return;
    };
    let prefix = format!("{}.", name.to_string_lossy());
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let n = entry.file_name().to_string_lossy().into_owned();
        if !(n.starts_with(&prefix) && n.ends_with(".part")) {
            continue;
        }
        let age = entry
            .metadata()
            .await
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a >= max_age) {
            let _ = tokio::fs::remove_file(entry.path()).await;
        }
    }
}

impl Drop for TempOutput {
    fn drop(&mut self) {
        if self.armed {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Arguments communs : pas de bannière ni de stdin, un seul protocole, entrée `file:`.
fn base_args(input: &Path) -> Vec<OsString> {
    let mut args: Vec<OsString> = [
        "-hide_banner",
        "-nostdin",
        "-v",
        "error",
        "-y",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .map(OsString::from)
    .to_vec();
    args.push(file_arg(input));
    args
}

/// Arguments de ffmpeg pour un plan avec ffmpeg (`Copy` ou `Transcode`).
fn ffmpeg_args(plan: Plan, input: &Path, temp: &Path) -> Vec<OsString> {
    let mut args = base_args(input);
    // Premier flux audio seul ; ni vidéo, ni sous-titres, ni données, ni métadonnées, ni
    // chapitres ; `bitexact` évite l'étiquette d'encodeur.
    args.extend(
        [
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-map_metadata",
            "-1",
            "-map_chapters",
            "-1",
            "-fflags",
            "+bitexact",
        ]
        .map(OsString::from),
    );
    match plan {
        Plan::Transcode { kbps } => {
            args.extend(
                ["-c:a", "libmp3lame", "-ar", "44100", "-ac", "2", "-b:a"].map(OsString::from),
            );
            args.push(format!("{kbps}k").into());
        }
        _ => args.extend(["-c:a", "copy"].map(OsString::from)),
    }
    args.extend(["-f", "mp3"].map(OsString::from));
    args.push(file_arg(temp));
    args
}

fn convert_timeout(duration_secs: Option<f64>) -> Duration {
    match duration_secs {
        // Un quart de la durée annoncée, plus 60 s : la conversion doit aller au moins 4 fois plus
        // vite que le temps réel, alors que libmp3lame fait bien plus de 50 fois le temps réel.
        // La durée vient du fichier (non fiable) : plafonnée, et bornée avant la conversion en
        // `Duration` (qui paniquerait sur une valeur énorme).
        Some(d) => Duration::from_secs_f64((60.0 + d / 4.0).min(MAX_CONVERT_TIMEOUT.as_secs_f64()))
            .clamp(MIN_CONVERT_TIMEOUT, MAX_CONVERT_TIMEOUT),
        None => UNKNOWN_DURATION_TIMEOUT,
    }
}

fn failed(why: &str) -> AppError {
    AppError::Conversion {
        code: "ffmpeg_failed".into(),
        message: format!("conversion audio échouée : {why}"),
    }
}

/// Convertit `input` en MP3 conforme dans `output` (règle D6). `output` n'existe qu'en cas de
/// succès ; en cas d'échec, d'annulation ou de délai, aucun fichier partiel ne reste.
pub async fn convert(
    ff: &Ffmpeg,
    input: &Path,
    output: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<Converted> {
    convert_with(ff, input, output, cancel, None).await
}

async fn convert_with(
    ff: &Ffmpeg,
    input: &Path,
    output: &Path,
    cancel: &mut watch::Receiver<bool>,
    timeout: Option<Duration>,
) -> AppResult<Converted> {
    let started = Instant::now();
    if let Some(dir) = output.parent().filter(|d| !d.as_os_str().is_empty()) {
        if !tokio::fs::metadata(dir).await.is_ok_and(|m| m.is_dir()) {
            return Err(AppError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("dossier de sortie absent : {}", dir.display()),
            )));
        }
    }
    let input_bytes = tokio::fs::metadata(input).await?.len();
    let info = probe(ff, input, cancel).await?;
    let plan = decide(&info);
    let mut temp = TempOutput::beside(output).await;
    let args = ffmpeg_args(plan, input, &temp.path);
    let limits = Limits::new(timeout.unwrap_or_else(|| convert_timeout(info.duration_secs)));
    let out = ff.run(Tool::Ffmpeg, &args, limits, cancel).await?;
    if !out.success() {
        return Err(failed(&last_line(&out.stderr)));
    }
    let output_bytes = tokio::fs::metadata(&temp.path).await?.len();
    if output_bytes == 0 {
        return Err(failed("fichier de sortie vide"));
    }
    // Un MP3 réduit à son en-tête (Xing) a une taille non nulle mais aucune trame audio.
    let written = match probe(ff, &temp.path, cancel).await {
        Err(AppError::Conversion { message, .. }) => {
            return Err(failed(&format!("sortie inexploitable ({message})")))
        }
        other => other?,
    };
    if !written.duration_secs.is_some_and(|d| d > 0.0) {
        return Err(failed("sortie sans audio (durée nulle ou inconnue)"));
    }
    if *cancel.borrow() {
        return Err(AppError::Cancelled);
    }
    // Une sortie existante est remplacée.
    temp.commit(output).await?;
    Ok(Converted {
        plan,
        input_bytes,
        output_bytes,
        duration_secs: info.duration_secs,
        elapsed: started.elapsed(),
    })
}

/// Clé de comparaison de deux chemins de sortie (dossier résolu si possible, nom en minuscules
/// sous Windows). Limite : ni liens symboliques ou physiques du fichier lui-même, ni systèmes de
/// fichiers insensibles à la casse hors Windows (macOS) ne sont reconnus.
async fn output_key(output: &Path) -> PathBuf {
    let dir = output
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let dir = tokio::fs::canonicalize(dir)
        .await
        .unwrap_or_else(|_| dir.to_path_buf());
    let name = output.file_name().unwrap_or_default();
    if cfg!(windows) {
        dir.join(name.to_string_lossy().to_lowercase())
    } else {
        dir.join(name)
    }
}

/// Convertit plusieurs fichiers (`(entrée, sortie)`), au plus [`MAX_PARALLEL`] à la fois. Le
/// résultat de chaque fichier est rendu dans l'ordre de `items` : un échec n'arrête pas les
/// autres (le fichier est signalé, P7) ; l'annulation les arrête tous. Une sortie en double dans
/// `items` est refusée (`duplicate_output`) : une conversion à la fois par sortie.
pub async fn convert_all(
    ff: &Ffmpeg,
    items: Vec<(PathBuf, PathBuf)>,
    cancel: &watch::Receiver<bool>,
) -> Vec<AppResult<Converted>> {
    let gate = Arc::new(Semaphore::new(MAX_PARALLEL));
    let mut set = tokio::task::JoinSet::new();
    let count = items.len();
    let mut results: Vec<Option<AppResult<Converted>>> = (0..count).map(|_| None).collect();
    let mut seen = std::collections::HashSet::new();
    for (index, (input, output)) in items.into_iter().enumerate() {
        // Une conversion à la fois par sortie : la seconde écraserait la première.
        if !seen.insert(output_key(&output).await) {
            results[index] = Some(Err(AppError::Conversion {
                code: "duplicate_output".into(),
                message: format!("sortie en double : {}", output.display()),
            }));
            continue;
        }
        let (ff, gate, mut cancel) = (ff.clone(), gate.clone(), cancel.clone());
        set.spawn(async move {
            let result = match gate.acquire().await {
                Ok(_permit) => convert(&ff, &input, &output, &mut cancel).await,
                Err(_) => Err(AppError::Cancelled),
            };
            (index, result)
        });
    }
    while let Some(joined) = set.join_next().await {
        if let Ok((index, result)) = joined {
            results[index] = Some(result);
        }
    }
    results
        .into_iter()
        .map(|r| {
            r.unwrap_or_else(
                || Err(std::io::Error::other("tâche de conversion interrompue").into()),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ffmpeg::MANIFEST_FILE;
    use std::fs;

    fn mp3(rate: u32, channels: u32, bit_rate: Option<u64>) -> SourceInfo {
        SourceInfo {
            codec: "mp3".into(),
            container_is_mp3: true,
            sample_rate: Some(rate),
            channels: Some(channels),
            bit_rate,
            ..SourceInfo::default()
        }
    }

    #[test]
    fn mp3_conforme_copie() {
        assert_eq!(decide(&mp3(44_100, 1, Some(128_000))), Plan::Copy);
        assert_eq!(decide(&mp3(44_100, 2, Some(64_000))), Plan::Copy);
        assert_eq!(decide(&mp3(44_100, 2, Some(192_000))), Plan::Copy);
        // Débit estimé légèrement au-delà : arrondi au plus proche.
        assert_eq!(decide(&mp3(44_100, 2, Some(192_400))), Plan::Copy);
    }

    #[test]
    fn mp3_conforme_avec_pochette_est_copie() {
        let info = SourceInfo {
            has_video: true,
            ..mp3(44_100, 1, Some(128_000))
        };
        assert_eq!(decide(&info), Plan::Copy);
    }

    #[test]
    fn mp3_trop_fort_reencode_a_192() {
        assert_eq!(
            decide(&mp3(44_100, 2, Some(256_000))),
            Plan::Transcode { kbps: 192 }
        );
    }

    #[test]
    fn mp3_trop_faible_reencode_a_64() {
        assert_eq!(
            decide(&mp3(44_100, 2, Some(48_000))),
            Plan::Transcode { kbps: 64 }
        );
    }

    #[test]
    fn mp3_22050_reencode() {
        assert_eq!(
            decide(&mp3(22_050, 1, Some(64_000))),
            Plan::Transcode { kbps: 64 }
        );
    }

    #[test]
    fn mp3_debit_inconnu_reencode_a_192() {
        assert_eq!(decide(&mp3(44_100, 2, None)), Plan::Transcode { kbps: 192 });
        assert_eq!(
            decide(&mp3(44_100, 2, Some(0))),
            Plan::Transcode { kbps: 192 }
        );
    }

    #[test]
    fn mp3_six_canaux_ou_dans_un_mp4_reencode() {
        assert!(matches!(
            decide(&mp3(44_100, 6, Some(128_000))),
            Plan::Transcode { .. }
        ));
        let mut info = mp3(44_100, 2, Some(128_000));
        info.container_is_mp3 = false;
        assert_eq!(decide(&info), Plan::Transcode { kbps: 128 });
    }

    #[test]
    fn debit_arrondi_au_multiple_de_16_superieur() {
        let aac = |bps| SourceInfo {
            codec: "aac".into(),
            ..mp3(44_100, 2, Some(bps))
        };
        assert_eq!(decide(&aac(96_000)), Plan::Transcode { kbps: 96 });
        assert_eq!(decide(&aac(100_000)), Plan::Transcode { kbps: 112 });
        assert_eq!(decide(&aac(97_000)), Plan::Transcode { kbps: 112 });
        assert_eq!(decide(&aac(32_000)), Plan::Transcode { kbps: 64 });
        assert_eq!(decide(&aac(190_000)), Plan::Transcode { kbps: 192 });
        assert_eq!(decide(&aac(320_000)), Plan::Transcode { kbps: 192 });
        assert_eq!(decide(&aac(u64::MAX)), Plan::Transcode { kbps: 192 });
        assert_eq!(transcode_kbps(None), 192);
    }

    #[test]
    fn autres_codecs_reencodes() {
        for codec in ["aac", "opus", "vorbis", "flac", "pcm_s16le"] {
            let info = SourceInfo {
                codec: codec.into(),
                container_is_mp3: false,
                ..mp3(44_100, 2, Some(128_000))
            };
            assert!(matches!(decide(&info), Plan::Transcode { .. }), "{codec}");
        }
    }

    #[test]
    fn chemin_prefixe_file() {
        for p in [
            "-",
            "-i",
            "pipe:1",
            "concat:a|b",
            "subfile,,start,0,end,0,,:x",
            "C:\\a b\\é.mp3",
        ] {
            let arg = file_arg(Path::new(p));
            assert_eq!(arg.to_string_lossy(), format!("file:{p}"));
        }
    }

    #[test]
    fn arguments_ffmpeg_fixes() {
        let args: Vec<String> = ffmpeg_args(
            Plan::Transcode { kbps: 112 },
            Path::new("-evil.m4a"),
            Path::new("out.mp3.part"),
        )
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
        let at = |needle: &str| args.iter().position(|a| a == needle).unwrap();
        assert_eq!(args[at("-i") + 1], "file:-evil.m4a");
        assert_eq!(args.last().unwrap(), "file:out.mp3.part");
        assert!(at("-protocol_whitelist") < at("-i"));
        assert_eq!(args[at("-protocol_whitelist") + 1], "file");
        assert_eq!(args[at("-b:a") + 1], "112k");
        assert_eq!(args[at("-ac") + 1], "2");
        assert_eq!(args[at("-ar") + 1], "44100");
        for flag in ["-nostdin", "-vn", "-sn", "-dn"] {
            assert!(args.iter().any(|a| a == flag), "{flag}");
        }
        let stripped = ffmpeg_args(Plan::Copy, Path::new("a"), Path::new("b"));
        assert!(stripped.iter().any(|a| a == "copy"));
        assert!(!stripped.iter().any(|a| a == "libmp3lame"));
    }

    #[test]
    fn sonde_json() {
        let json = br#"{"streams":[{"codec_type":"video","codec_name":"mjpeg"},
            {"codec_type":"audio","codec_name":"mp3","sample_rate":"44100","channels":2,
             "bit_rate":"128000","duration":"61.5"}],
            "format":{"format_name":"mp3","bit_rate":"131000","duration":"61.6"}}"#;
        let info = parse_probe(json).unwrap();
        assert_eq!(info.codec, "mp3");
        assert!(info.container_is_mp3 && info.has_video);
        assert_eq!(info.sample_rate, Some(44_100));
        assert_eq!(info.bit_rate, Some(128_000));
        assert_eq!(info.duration_secs, Some(61.5));
        assert_eq!(decide(&info), Plan::Copy);

        // Débit du flux absent : celui du conteneur ; format MP4 : pas un MP3.
        let json = br#"{"streams":[{"codec_type":"audio","codec_name":"aac","sample_rate":"48000","channels":2}],
            "format":{"format_name":"mov,mp4,m4a,3gp,3g2,mj2","bit_rate":"99000"}}"#;
        let info = parse_probe(json).unwrap();
        assert!(!info.container_is_mp3);
        assert_eq!(info.bit_rate, Some(99_000));
    }

    #[test]
    fn sonde_sans_audio_ou_illisible() {
        let err = parse_probe(br#"{"streams":[{"codec_type":"video"}],"format":{}}"#).unwrap_err();
        assert!(
            matches!(&err, AppError::Conversion { code, .. } if code == "no_audio"),
            "{err:?}"
        );
        let err = parse_probe(b"pas du json").unwrap_err();
        assert!(
            matches!(&err, AppError::Conversion { code, .. } if code == "unreadable_audio"),
            "{err:?}"
        );
    }

    #[test]
    fn delai_proportionnel_a_la_duree() {
        assert_eq!(convert_timeout(Some(60.0)), MIN_CONVERT_TIMEOUT);
        assert!(convert_timeout(Some(36_000.0)) > Duration::from_secs(5000));
        assert_eq!(convert_timeout(None), UNKNOWN_DURATION_TIMEOUT);
        // Durée annoncée aberrante : plafonnée, sans panique.
        assert_eq!(convert_timeout(Some(1e300)), MAX_CONVERT_TIMEOUT);
        assert_eq!(convert_timeout(Some(f64::MAX)), MAX_CONVERT_TIMEOUT);
    }

    #[tokio::test]
    async fn temporaire_supprime_sauf_renomme() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("ep.mp3");
        let tmp_path = {
            let temp = TempOutput::beside(&out).await;
            fs::write(&temp.path, b"partiel").unwrap();
            temp.path.clone()
        };
        assert!(!tmp_path.exists(), "supprimé à la destruction");
        let mut temp = TempOutput::beside(&out).await;
        fs::write(&temp.path, b"complet").unwrap();
        temp.commit(&out).await.unwrap();
        drop(temp);
        assert_eq!(fs::read(&out).unwrap(), b"complet");
    }

    #[tokio::test]
    async fn part_perime_supprime_autre_sortie_ou_recent_garde() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("ep.mp3");
        let old = std::time::SystemTime::now() - Duration::from_secs(3600);
        let write = |name: &str, mtime: std::time::SystemTime| {
            let path = dir.path().join(name);
            fs::write(&path, b"x").unwrap();
            fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(mtime)
                .unwrap();
            path
        };
        let stale = write("ep.mp3.9-0.part", old);
        let other = write("autre.mp3.9-0.part", old);
        let live = write("ep.mp3.9-1.part", std::time::SystemTime::now());
        remove_stale_parts(&out, STALE_PART_AGE).await;
        assert!(!stale.exists(), "périmé supprimé");
        assert!(other.exists(), "autre sortie conservée");
        assert!(live.exists(), "récent (conversion en cours) conservé");
        // `beside` applique le même nettoyage.
        let _temp = TempOutput::beside(&out).await;
        assert!(live.exists());
    }

    #[tokio::test]
    async fn sorties_en_double_meme_cle() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("ep.mp3");
        assert_eq!(
            output_key(&a).await,
            output_key(&dir.path().join(".").join("ep.mp3")).await
        );
        assert_ne!(
            output_key(&a).await,
            output_key(&dir.path().join("autre.mp3")).await
        );
    }

    fn is_duplicate(r: &AppResult<Converted>) -> bool {
        matches!(r, Err(AppError::Conversion { code, .. }) if code == "duplicate_output")
    }

    #[tokio::test]
    async fn convert_all_refuse_les_sorties_en_double() {
        let dir = tempfile::tempdir().unwrap();
        let ff = Ffmpeg::in_dir(dir.path());
        let out = dir.path().join("ep.mp3");
        let items = vec![
            (dir.path().join("a.mp3"), out.clone()),
            (dir.path().join("b.mp3"), out),
        ];
        let (_tx, rx) = watch::channel(false);
        let results = convert_all(&ff, items, &rx).await;
        assert_eq!(results.len(), 2);
        assert!(is_duplicate(&results[1]), "{:?}", results[1]);
        // La première n'est pas refusée comme doublon (ici : entrée absente).
        assert!(!is_duplicate(&results[0]));
    }

    #[tokio::test]
    async fn dossier_de_sortie_absent_n_est_pas_ffmpeg_failed() {
        let dir = tempfile::tempdir().unwrap();
        let ff = Ffmpeg::in_dir(dir.path());
        let (_tx, mut rx) = watch::channel(false);
        let err = convert(
            &ff,
            &dir.path().join("a.mp3"),
            &dir.path().join("absent").join("b.mp3"),
            &mut rx,
        )
        .await
        .unwrap_err();
        match err {
            AppError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::NotFound),
            other => panic!("{other:?}"),
        }
    }

    // --- tests avec le vrai ffmpeg embarqué --------------------------------------------------

    /// ffmpeg embarqué (`npm run build:ffmpeg`), sinon `None` (test ignoré) ; exigé si
    /// `FLAM_REQUIRE_FFMPEG` est défini.
    fn real_ffmpeg() -> Option<Ffmpeg> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("build")
            .join("ffmpeg");
        if !dir.join(Tool::Ffmpeg.file_name()).is_file()
            || !dir.join(Tool::Ffprobe.file_name()).is_file()
            || !dir.join(MANIFEST_FILE).is_file()
        {
            assert!(
                std::env::var_os("FLAM_REQUIRE_FFMPEG").is_none(),
                "ffmpeg embarqué non construit alors que FLAM_REQUIRE_FFMPEG est défini"
            );
            eprintln!("ffmpeg embarqué non construit : test ignoré");
            return None;
        }
        Some(Ffmpeg::in_dir(dir))
    }

    async fn run_ff(ff: &Ffmpeg, args: Vec<OsString>) {
        let (_tx, mut rx) = watch::channel(false);
        let out = ff
            .run(
                Tool::Ffmpeg,
                &args,
                Limits::new(Duration::from_secs(60)),
                &mut rx,
            )
            .await
            .unwrap();
        assert!(out.success(), "{out:?}");
    }

    fn gen_args(rate: u32) -> Vec<OsString> {
        let mut args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
        ]
        .map(OsString::from)
        .to_vec();
        args.push(format!("sine=frequency=440:duration=1:sample_rate={rate}").into());
        args
    }

    /// Génère un MP3 de 1 s (sinus) : échantillonnage, canaux, débit.
    async fn gen_mp3(ff: &Ffmpeg, path: &Path, rate: u32, channels: u32, kbps: u32) {
        let mut args = gen_args(rate);
        args.extend([
            "-ac".into(),
            channels.to_string().into(),
            "-c:a".into(),
            "libmp3lame".into(),
            "-b:a".into(),
            format!("{kbps}k").into(),
            "-f".into(),
            "mp3".into(),
        ]);
        args.push(file_arg(path));
        run_ff(ff, args).await;
    }

    /// Génère un WAV mono 22 050 Hz (PCM seulement : le build n'a pas d'encodeur d'image).
    async fn gen_wav(ff: &Ffmpeg, path: &Path) {
        let mut args = gen_args(22_050);
        args.extend(["-ac", "1", "-c:a", "pcm_s16le", "-f", "wav"].map(OsString::from));
        args.push(file_arg(path));
        run_ff(ff, args).await;
    }

    /// Préfixe `mp3` d'une étiquette ID3v2.3 portant une pochette (octets JPEG factices : seul
    /// le démultiplexage est exercé, jamais le décodage).
    fn with_cover(mp3: &[u8]) -> Vec<u8> {
        let image = [
            0xff, 0xd8, 0xff, 0xe0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
            0xff, 0xd9,
        ];
        let mut frame_body = vec![0u8];
        frame_body.extend_from_slice(b"image/jpeg\0");
        frame_body.extend_from_slice(&[3, 0]);
        frame_body.extend_from_slice(&image);
        let mut frame = b"APIC".to_vec();
        frame.extend_from_slice(&(frame_body.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&frame_body);
        let size = frame.len() as u32;
        let mut out = b"ID3\x03\x00\x00".to_vec();
        out.extend_from_slice(&[
            ((size >> 21) & 0x7f) as u8,
            ((size >> 14) & 0x7f) as u8,
            ((size >> 7) & 0x7f) as u8,
            (size & 0x7f) as u8,
        ]);
        out.extend_from_slice(&frame);
        out.extend_from_slice(mp3);
        out
    }

    fn no_part_files(dir: &Path) {
        let left: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".part"))
            .collect();
        assert!(left.is_empty(), "temporaires restants : {left:?}");
    }

    /// Trames audio seules (sans étiquettes ni en-têtes) : remux dans un fichier, lu en octets.
    /// N'utilise que le muxer `mp3` et le protocole `file` du ffmpeg embarqué.
    async fn frames_of(ff: &Ffmpeg, path: &Path) -> Vec<u8> {
        let tmp = path.with_extension("frames.mp3");
        let mut args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-protocol_whitelist",
            "file",
            "-i",
        ]
        .map(OsString::from)
        .to_vec();
        args.push(file_arg(path));
        args.extend(
            [
                "-map",
                "0:a:0",
                "-c:a",
                "copy",
                "-map_metadata",
                "-1",
                "-map_chapters",
                "-1",
                "-fflags",
                "+bitexact",
                "-f",
                "mp3",
            ]
            .map(OsString::from),
        );
        args.push(file_arg(&tmp));
        run_ff(ff, args).await;
        fs::read(&tmp).unwrap()
    }

    /// Étiquettes du conteneur, en JSON.
    async fn tags_of(ff: &Ffmpeg, path: &Path) -> String {
        let mut args: Vec<OsString> = [
            "-v",
            "error",
            "-show_entries",
            "format_tags",
            "-of",
            "json",
            "-i",
        ]
        .map(OsString::from)
        .to_vec();
        args.push(file_arg(path));
        let (_tx, mut rx) = watch::channel(false);
        let out = ff
            .run(
                Tool::Ffprobe,
                &args,
                Limits::new(Duration::from_secs(60)),
                &mut rx,
            )
            .await
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    async fn probe_of(ff: &Ffmpeg, path: &Path) -> SourceInfo {
        let (_tx, mut rx) = watch::channel(false);
        probe(ff, path, &mut rx).await.unwrap()
    }

    #[tokio::test]
    async fn reel_mp3_mono_128_trames_identiques_sans_etiquettes() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.mp3"), dir.path().join("b.mp3"));
        let mut args = gen_args(44_100);
        args.extend(
            [
                "-ac",
                "1",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "128k",
                "-metadata",
                "title=Secret",
                "-f",
                "mp3",
            ]
            .map(OsString::from),
        );
        args.push(file_arg(&src));
        run_ff(&ff, args).await;
        assert!(
            tags_of(&ff, &src).await.contains("Secret"),
            "étiquette source"
        );
        let (_tx, mut rx) = watch::channel(false);
        let done = convert(&ff, &src, &dst, &mut rx).await.unwrap();
        assert_eq!(done.plan, Plan::Copy);
        assert_eq!(frames_of(&ff, &src).await, frames_of(&ff, &dst).await);
        assert!(
            !tags_of(&ff, &dst).await.contains("Secret"),
            "étiquette retirée"
        );
        let info = probe_of(&ff, &dst).await;
        assert_eq!((info.sample_rate, info.channels), (Some(44_100), Some(1)));
        assert!(done.duration_secs.is_some_and(|d| d > 0.5));
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_mp3_256_reencode_a_192_stereo() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.mp3"), dir.path().join("b.mp3"));
        gen_mp3(&ff, &src, 44_100, 2, 256).await;
        let (_tx, mut rx) = watch::channel(false);
        let done = convert(&ff, &src, &dst, &mut rx).await.unwrap();
        assert_eq!(done.plan, Plan::Transcode { kbps: 192 });
        let info = probe_of(&ff, &dst).await;
        assert_eq!((info.sample_rate, info.channels), (Some(44_100), Some(2)));
        assert!(info.container_is_mp3 && !info.has_video);
        assert_eq!(decide(&info), Plan::Copy, "la sortie est conforme");
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_mp3_22050_mono_devient_44100_stereo() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.mp3"), dir.path().join("b.mp3"));
        gen_mp3(&ff, &src, 22_050, 1, 64).await;
        let (_tx, mut rx) = watch::channel(false);
        let done = convert(&ff, &src, &dst, &mut rx).await.unwrap();
        assert!(matches!(done.plan, Plan::Transcode { .. }), "{done:?}");
        let info = probe_of(&ff, &dst).await;
        assert_eq!((info.sample_rate, info.channels), (Some(44_100), Some(2)));
    }

    #[tokio::test]
    async fn reel_wav_reencode_a_192() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.wav"), dir.path().join("b.mp3"));
        gen_wav(&ff, &src).await;
        let (_tx, mut rx) = watch::channel(false);
        let done = convert(&ff, &src, &dst, &mut rx).await.unwrap();
        assert_eq!(done.plan, Plan::Transcode { kbps: 192 });
        assert_eq!(probe_of(&ff, &dst).await.codec, "mp3");
    }

    #[tokio::test]
    async fn reel_pochette_integree_retiree() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("plain.mp3");
        gen_mp3(&ff, &plain, 44_100, 2, 128).await;
        let covered = dir.path().join("cover.mp3");
        fs::write(&covered, with_cover(&fs::read(&plain).unwrap())).unwrap();
        assert!(probe_of(&ff, &covered).await.has_video, "pochette détectée");
        let (_tx, mut rx) = watch::channel(false);
        // Conforme : recopié sans l'image.
        let dst = dir.path().join("out1.mp3");
        let done = convert(&ff, &covered, &dst, &mut rx).await.unwrap();
        assert_eq!(done.plan, Plan::Copy);
        assert!(!probe_of(&ff, &dst).await.has_video);
        // Réencodé (22 050 Hz) : sans l'image non plus.
        let low = dir.path().join("low.mp3");
        gen_mp3(&ff, &low, 22_050, 1, 64).await;
        let low_cover = dir.path().join("low_cover.mp3");
        fs::write(&low_cover, with_cover(&fs::read(&low).unwrap())).unwrap();
        let dst = dir.path().join("out2.mp3");
        let done = convert(&ff, &low_cover, &dst, &mut rx).await.unwrap();
        assert!(matches!(done.plan, Plan::Transcode { .. }));
        assert!(!probe_of(&ff, &dst).await.has_video);
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_fichier_corrompu_signale_sans_sortie() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.mp3"), dir.path().join("b.mp3"));
        fs::write(&src, vec![0x5au8; 4096]).unwrap();
        let (_tx, mut rx) = watch::channel(false);
        let err = convert(&ff, &src, &dst, &mut rx).await.unwrap_err();
        assert!(
            matches!(&err, AppError::Conversion { code, .. } if code == "unreadable_audio"),
            "{err:?}"
        );
        assert!(!dst.exists());
        no_part_files(dir.path());
    }

    /// WAV PCM 16 bits mono 22 050 Hz **sans aucun échantillon** : lisible, mais l'encodage MP3
    /// qui en découle ne produit que l'en-tête Xing/Info (aucune trame audio). ffprobe refuse
    /// une telle sortie (« sortie inexploitable ») ; la branche « durée nulle » du contrôle
    /// couvre le cas où il l'accepterait.
    fn empty_wav() -> Vec<u8> {
        let mut wav = b"RIFF".to_vec();
        wav.extend_from_slice(&36u32.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&22_050u32.to_le_bytes());
        wav.extend_from_slice(&44_100u32.to_le_bytes()); // octets par seconde
        wav.extend_from_slice(&2u16.to_le_bytes()); // alignement
        wav.extend_from_slice(&16u16.to_le_bytes()); // bits par échantillon
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&0u32.to_le_bytes());
        wav
    }

    #[tokio::test]
    async fn reel_mp3_reduit_a_l_en_tete_xing_est_ffmpeg_failed() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("vide.wav"), dir.path().join("out.mp3"));
        fs::write(&src, empty_wav()).unwrap();
        let (_tx, mut rx) = watch::channel(false);
        let err = convert(&ff, &src, &dst, &mut rx).await.unwrap_err();
        assert!(
            matches!(&err, AppError::Conversion { code, message }
                if code == "ffmpeg_failed"
                    && (message.contains("sortie inexploitable")
                        || message.contains("sortie sans audio"))),
            "{err:?}"
        );
        assert!(!dst.exists());
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_nom_de_fichier_ressemblant_a_une_option() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        // Noms commençant par « - » ou ressemblant à un protocole : jamais interprétés.
        let src = dir.path().join("-i concat;x.mp3");
        let dst = dir.path().join("-y pipe;1.mp3");
        gen_mp3(&ff, &src, 22_050, 1, 64).await;
        // Chemin relatif : le préfixe `file:` le protège aussi.
        let (_tx, mut rx) = watch::channel(false);
        convert(&ff, &src, &dst, &mut rx).await.unwrap();
        assert!(dst.is_file());
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_annulation_en_cours_ne_laisse_aucun_fichier() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("long.wav"), dir.path().join("b.mp3"));
        // 10 minutes de PCM : la conversion dure bien plus que le délai avant l'annulation.
        let mut args: Vec<OsString> = [
            "-hide_banner",
            "-nostdin",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=600:sample_rate=22050",
            "-ac",
            "1",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
        ]
        .map(OsString::from)
        .to_vec();
        args.push(file_arg(&src));
        run_ff(&ff, args).await;
        let (tx, mut rx) = watch::channel(false);
        let (ff2, src2, dst2) = (ff.clone(), src.clone(), dst.clone());
        let task = tokio::spawn(async move { convert(&ff2, &src2, &dst2, &mut rx).await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.send(true).unwrap();
        let err = task.await.unwrap().unwrap_err();
        assert!(matches!(err, AppError::Cancelled), "{err:?}");
        assert!(!dst.exists());
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_delai_depasse_ne_laisse_aucun_fichier() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let (src, dst) = (dir.path().join("a.mp3"), dir.path().join("b.mp3"));
        gen_mp3(&ff, &src, 22_050, 1, 64).await;
        let (_tx, mut rx) = watch::channel(false);
        let err = convert_with(&ff, &src, &dst, &mut rx, Some(Duration::from_millis(1)))
            .await
            .unwrap_err();
        match err {
            AppError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut),
            other => panic!("{other:?}"),
        }
        assert!(!dst.exists());
        no_part_files(dir.path());
    }

    #[tokio::test]
    async fn reel_lot_en_parallele_un_echec_n_arrete_pas_les_autres() {
        let Some(ff) = real_ffmpeg() else { return };
        let dir = tempfile::tempdir().unwrap();
        let mut items = Vec::new();
        for i in 0..4 {
            let src = dir.path().join(format!("{i}.mp3"));
            if i == 2 {
                fs::write(&src, b"corrompu").unwrap();
            } else {
                gen_mp3(&ff, &src, 22_050, 1, 64).await;
            }
            items.push((src, dir.path().join(format!("out{i}.mp3"))));
        }
        let (_tx, rx) = watch::channel(false);
        let results = convert_all(&ff, items, &rx).await;
        assert_eq!(results.len(), 4);
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r.is_ok(), i != 2, "{i}: {r:?}");
        }
        no_part_files(dir.path());
    }
}
