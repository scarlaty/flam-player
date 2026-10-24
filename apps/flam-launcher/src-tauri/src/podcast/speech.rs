//! Synthèse vocale des titres d'un pack (#462, épopée podcasts #207, note § 12).
//!
//! PR 1 : le module seul ; la PR 2 l'appelle depuis la création des packs
//! (`install::pack_speech`). Il fournit :
//!
//! - un trait [`Synthesizer`] (liste des voix, synthèse d'un texte en WAV) : implémentation
//!   WinRT `Windows.Media.SpeechSynthesis` sous Windows (`winrt`), « aucune voix » ailleurs
//!   ([`NoSpeech`]), voix simulée déterministe pour les tests (`testing`) ;
//! - le tri des voix : françaises seulement (`fr`, `fr-FR`, `fr-CA`…), voix en ligne exclues,
//!   ordre stable par nom, défaut = voix par défaut du système si elle est française, sinon la
//!   première, un identifiant n'apparaissant qu'une fois ([`french_voices`], [`resolve_voice`]) ;
//! - la préparation du texte lu ([`prepare_text`]) : première ligne, 120 caractères au plus, les
//!   titres sans lettre restent muets ;
//! - la synthèse bornée dans le temps ([`synthesize_wav`]) et le contrôle du WAV obtenu
//!   ([`Wav::parse`]) : 16 bits, mono ou stéréo, non vide, 15 s au plus ;
//! - la conversion en MP3 mono 64 kbit/s, 44 100 Hz par le ffmpeg embarqué ([`wav_to_mp3`]), avec
//!   les mêmes précautions que `podcast::audio` (arguments fixes, empreinte vérifiée, délai,
//!   annulation, aucun temporaire qui reste) ;
//! - les commandes Tauri `tts_voices`, `tts_preview` et `tts_preview_stop`.
//!
//! Choix et limites :
//!
//! - **Aperçu joué côté Rust** (`PlaySoundW`, WAV en mémoire, fonction déjà présente dans
//!   `windows-sys`, aucune dépendance de plus) : la politique CSP du launcher reste inchangée
//!   (pas de `media-src blob:`). Un aperçu à la fois : un nouvel aperçu, ou `tts_preview_stop`,
//!   coupe le précédent et annule la synthèse en cours (son résultat est jeté) ;
//! - **voix en ligne** : l'API ne dit pas si une voix est locale ; seules celles dont le nom ou
//!   l'identifiant contient « online » ou « en ligne » (voix « naturelles » de Windows 11) sont
//!   écartées. Heuristique, à confirmer sur machine sans réseau (note § 12.8) ;
//! - **vie privée** : ni texte ni nom de voix dans les messages d'erreur ni dans les journaux ;
//! - **délai** : une synthèse qui ne rend pas la main est abandonnée (le thread reste, borné par
//!   la durée de l'appel système) ; le résultat tardif est jeté.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::sync::watch;

use crate::ffmpeg::{Ffmpeg, Limits, Tool, Verified};
use crate::podcast::audio::file_arg;
use crate::{AppError, AppResult};

/// Longueur maximale du texte lu, en caractères.
pub const MAX_TEXT_CHARS: usize = 120;
/// Durée maximale d'un titre synthétisé ; au-delà, le titre reste muet (pas de coupe).
pub const MAX_AUDIO_SECS: f64 = 15.0;
/// Délai d'une synthèse.
pub const SYNTH_TIMEOUT: Duration = Duration::from_secs(10);
/// Délai de la liste des voix et de la voix par défaut (WinRT), chacune.
#[cfg(windows)]
const VOICES_TIMEOUT: Duration = Duration::from_secs(10);
/// Délai global de l'énumération des voix françaises (liste **et** voix par défaut, qui ont
/// chacune leur délai de 10 s) : un service de voix bloqué ne retient jamais l'appelant plus
/// longtemps (#480).
pub const VOICES_TOTAL_TIMEOUT: Duration = Duration::from_secs(12);
/// Délai de la conversion d'un titre en MP3.
pub const MP3_TIMEOUT: Duration = Duration::from_secs(30);
/// Taille maximale d'un WAV accepté (15 s en 48 kHz stéréo 16 bits font moins de 3 Mio).
const MAX_WAV_BYTES: usize = 8 * 1024 * 1024;
/// Débit du MP3 de titre (mono).
const MP3_KBPS: u32 = 64;

/// Voix installée sur le poste.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Voice {
    /// Identifiant stable (`VoiceInformation.Id`), mémorisé dans les réglages.
    pub id: String,
    /// Nom affiché (« Microsoft Hortense »).
    pub name: String,
    /// Étiquette de langue BCP 47 (`fr-FR`).
    pub language: String,
}

/// Voix françaises du poste, prêtes pour l'interface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceList {
    /// Triées par nom ; vide si aucune voix française (case désactivée côté interface).
    pub voices: Vec<Voice>,
    /// Voix proposée par défaut, `None` si la liste est vide.
    pub default_id: Option<String>,
}

/// Échec du module, converti en [`AppError`] (codes `tts_*`) à la frontière des commandes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpeechError {
    /// Synthèse impossible sur ce poste (autre système, service de voix indisponible).
    Unavailable,
    /// Aucune voix française installée.
    NoVoice,
    /// La voix demandée n'est plus installée.
    VoiceGone,
    /// Le texte n'a rien à lire (vide, sans lettre).
    NothingToSay,
    /// La synthèse n'a pas rendu la main à temps.
    Timeout,
    /// Audio de plus de [`MAX_AUDIO_SECS`].
    TooLong,
    /// Autre échec (détail technique, sans texte ni nom de voix).
    Failed(String),
}

impl From<SpeechError> for AppError {
    fn from(e: SpeechError) -> Self {
        match e {
            SpeechError::Unavailable => AppError::TtsUnavailable(
                "La synthèse vocale n'est pas disponible sur ce poste.".into(),
            ),
            SpeechError::NoVoice => AppError::TtsNoVoice(
                "Aucune voix française n'est installée dans Windows \
                 (Paramètres > Heure et langue > Voix)."
                    .into(),
            ),
            SpeechError::VoiceGone => {
                AppError::TtsNoVoice("La voix choisie n'est plus installée dans Windows.".into())
            }
            SpeechError::NothingToSay => {
                AppError::Validation("Ce titre n'a rien à lire à voix haute.".into())
            }
            SpeechError::Timeout => AppError::TtsFailed("La voix n'a pas répondu à temps.".into()),
            SpeechError::TooLong => AppError::TtsFailed(format!(
                "Le titre est trop long à lire (plus de {MAX_AUDIO_SECS} s)."
            )),
            SpeechError::Failed(detail) => {
                AppError::TtsFailed(format!("La synthèse vocale a échoué ({detail})."))
            }
        }
    }
}

/// Moteur de synthèse. Les méthodes sont bloquantes : elles tournent hors du runtime async.
pub trait Synthesizer: Send + Sync + 'static {
    /// Toutes les voix installées, sans filtre.
    fn voices(&self) -> Result<Vec<Voice>, SpeechError>;
    /// Identifiant de la voix par défaut du système, si elle existe.
    fn default_voice_id(&self) -> Option<String>;
    /// Synthétise `text` (déjà préparé) avec la voix `voice_id` : WAV PCM complet, en mémoire.
    fn synthesize(&self, text: &str, voice_id: &str) -> Result<Vec<u8>, SpeechError>;
}

/// Moteur des systèmes sans synthèse : « synthèse indisponible ».
pub struct NoSpeech;

impl Synthesizer for NoSpeech {
    fn voices(&self) -> Result<Vec<Voice>, SpeechError> {
        Err(SpeechError::Unavailable)
    }
    fn default_voice_id(&self) -> Option<String> {
        None
    }
    fn synthesize(&self, _text: &str, _voice_id: &str) -> Result<Vec<u8>, SpeechError> {
        Err(SpeechError::Unavailable)
    }
}

/// Moteur du système : WinRT sous Windows, sinon [`NoSpeech`].
pub fn system() -> Arc<dyn Synthesizer> {
    #[cfg(windows)]
    {
        Arc::new(winrt::WinRtSynthesizer)
    }
    #[cfg(not(windows))]
    {
        Arc::new(NoSpeech)
    }
}

// --- voix --------------------------------------------------------------------------------------

fn is_french(voice: &Voice) -> bool {
    let lang = voice.language.to_ascii_lowercase();
    lang == "fr" || lang.starts_with("fr-")
}

/// Voix en ligne : détectée d'après le nom ou l'identifiant (l'API n'expose pas d'indicateur).
fn is_online(voice: &Voice) -> bool {
    let text = format!("{} {}", voice.name, voice.id).to_lowercase();
    text.contains("online") || text.contains("en ligne")
}

/// Voix françaises locales, triées par nom (puis identifiant) ; défaut = voix par défaut du
/// système si elle en fait partie, sinon la première. Liste vide : aucune voix française.
pub fn french_voices(synth: &dyn Synthesizer) -> Result<VoiceList, SpeechError> {
    // Un identifiant n'apparaît qu'une fois, même sous deux noms (la première voix rendue gagne).
    let mut seen = std::collections::HashSet::new();
    let mut voices: Vec<Voice> = synth
        .voices()?
        .into_iter()
        .filter(|v| is_french(v) && !is_online(v) && seen.insert(v.id.clone()))
        .collect();
    voices.sort_by_cached_key(|v| (v.name.to_lowercase(), v.id.clone()));
    let default_id = synth
        .default_voice_id()
        .filter(|id| voices.iter().any(|v| &v.id == id))
        .or_else(|| voices.first().map(|v| v.id.clone()));
    Ok(VoiceList { voices, default_id })
}

/// [`french_voices`] borné par un délai global (thread dédié, résultat tardif jeté) ; délai
/// dépassé : [`SpeechError::Unavailable`] (service de voix qui ne répond pas).
pub fn french_voices_within(
    synth: &Arc<dyn Synthesizer>,
    timeout: Duration,
) -> Result<VoiceList, SpeechError> {
    let engine = Arc::clone(synth);
    match run_bounded(timeout, move || french_voices(engine.as_ref())) {
        Ok(result) => result,
        Err(SpeechError::Timeout) => Err(SpeechError::Unavailable),
        Err(e) => Err(e),
    }
}

/// Choisit la voix : `requested` s'il existe encore (`VoiceGone` sinon), le défaut si aucune
/// voix n'est demandée. Le repli sur le défaut d'une voix disparue est laissé à l'appelant.
pub fn resolve_voice(list: &VoiceList, requested: Option<&str>) -> Result<Voice, SpeechError> {
    if list.voices.is_empty() {
        return Err(SpeechError::NoVoice);
    }
    let wanted = match requested {
        Some(id) => Some(id),
        None => list.default_id.as_deref(),
    };
    wanted
        .and_then(|id| list.voices.iter().find(|v| v.id == id))
        .cloned()
        .ok_or(SpeechError::VoiceGone)
}

// --- texte -------------------------------------------------------------------------------------

/// Texte réellement lu : première ligne non vide, caractères de contrôle retirés, espaces
/// réduits, 120 caractères au plus (coupé sur un mot si possible). `None` : le titre reste muet
/// (vide, ou sans aucune lettre : « 🎧 », « #12 »).
pub fn prepare_text(raw: &str) -> Option<String> {
    let line = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let cleaned: String = line
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let text = if cleaned.chars().count() > MAX_TEXT_CHARS {
        let cut: String = cleaned.chars().take(MAX_TEXT_CHARS).collect();
        let at_boundary = cleaned
            .chars()
            .nth(MAX_TEXT_CHARS)
            .is_some_and(char::is_whitespace);
        let kept = match cut.rfind(char::is_whitespace) {
            Some(i) if !at_boundary && cut[..i].chars().count() >= MAX_TEXT_CHARS / 2 => &cut[..i],
            _ => cut.as_str(),
        };
        kept.trim_end().to_owned()
    } else {
        cleaned
    };
    text.chars().any(char::is_alphabetic).then_some(text)
}

// --- WAV ---------------------------------------------------------------------------------------

/// WAV PCM 16 bits contrôlé.
#[derive(Debug, Clone, PartialEq)]
pub struct Wav {
    pub bytes: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_secs: f64,
}

fn failed(detail: &str) -> SpeechError {
    SpeechError::Failed(detail.to_owned())
}

fn le16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Wav {
    /// Contrôle un WAV rendu par un moteur : en-tête RIFF/WAVE, `fmt ` PCM 16 bits mono ou
    /// stéréo, données non vides, durée au plus [`MAX_AUDIO_SECS`]. Un champ de taille de données
    /// trop grand (flux) est ramené à ce qui est présent.
    pub fn parse(bytes: Vec<u8>) -> Result<Wav, SpeechError> {
        if bytes.len() > MAX_WAV_BYTES {
            return Err(SpeechError::TooLong);
        }
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(failed("audio sans en-tête WAV"));
        }
        let mut fmt: Option<(u16, u32)> = None;
        let mut data_len = None;
        let mut pos = 12usize;
        while pos + 8 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let size = le32(&bytes, pos + 4) as usize;
            let body = pos + 8;
            if id == b"fmt " {
                if size < 16 || body + 16 > bytes.len() {
                    return Err(failed("bloc fmt du WAV tronqué"));
                }
                let tag = le16(&bytes, body);
                let channels = le16(&bytes, body + 2);
                let rate = le32(&bytes, body + 4);
                let bits = le16(&bytes, body + 14);
                // 0xFFFE : WAVE_FORMAT_EXTENSIBLE, dont le sous-format (GUID à `body + 24`, ses
                // 4 premiers octets) doit être 1 (PCM).
                let pcm = match tag {
                    1 => true,
                    0xFFFE => {
                        size >= 40 && body + 28 <= bytes.len() && le32(&bytes, body + 24) == 1
                    }
                    _ => false,
                };
                if !pcm || bits != 16 {
                    return Err(failed("format audio inattendu (PCM 16 bits requis)"));
                }
                if !(1..=2).contains(&channels) || !(8_000..=96_000).contains(&rate) {
                    return Err(failed("fréquence ou nombre de canaux inattendu"));
                }
                fmt = Some((channels, rate));
            } else if id == b"data" {
                data_len = Some(size.min(bytes.len() - body));
                break;
            }
            pos = match body.checked_add(size).and_then(|p| p.checked_add(size & 1)) {
                Some(p) => p,
                None => break,
            };
        }
        let (Some((channels, rate)), Some(data_len)) = (fmt, data_len) else {
            return Err(failed("WAV sans format ou sans données"));
        };
        let frames = data_len / (usize::from(channels) * 2);
        if frames == 0 {
            return Err(failed("audio vide"));
        }
        let duration_secs = frames as f64 / f64::from(rate);
        if duration_secs > MAX_AUDIO_SECS {
            return Err(SpeechError::TooLong);
        }
        Ok(Wav {
            bytes,
            sample_rate: rate,
            channels,
            duration_secs,
        })
    }
}

/// Synthèse d'un titre : texte préparé, moteur appelé dans un thread dédié ([`run_bounded`]), WAV
/// contrôlé.
pub fn synthesize_wav(
    synth: &Arc<dyn Synthesizer>,
    text: &str,
    voice_id: &str,
    timeout: Duration,
) -> Result<Wav, SpeechError> {
    let text = prepare_text(text).ok_or(SpeechError::NothingToSay)?;
    let engine = Arc::clone(synth);
    let voice = voice_id.to_owned();
    let bytes = run_bounded(timeout, move || engine.synthesize(&text, &voice))??;
    Wav::parse(bytes)
}

/// Exécute `job` dans un thread dédié ; `timeout` dépassé : [`SpeechError::Timeout`] (le thread est
/// abandonné, son résultat jeté). Thread interrompu (panique) : échec.
fn run_bounded<T: Send + 'static>(
    timeout: Duration,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T, SpeechError> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("tts".into())
        .spawn(move || {
            // Le récepteur peut avoir abandonné (délai) : l'envoi est alors sans effet.
            let _ = tx.send(job());
        })
        .map_err(|_| failed("thread de synthèse non lancé"))?;
    match rx.recv_timeout(timeout) {
        Ok(result) => Ok(result),
        Err(RecvTimeoutError::Timeout) => Err(SpeechError::Timeout),
        Err(RecvTimeoutError::Disconnected) => Err(failed("synthèse interrompue")),
    }
}

// --- MP3 ---------------------------------------------------------------------------------------

/// Arguments de ffmpeg : WAV en entrée, MP3 mono 44 100 Hz 64 kbit/s en sortie, sans
/// métadonnées ; chemins préfixés `file:` (jamais pris pour une option ni un protocole).
fn mp3_args(wav: &Path, mp3: &Path) -> Vec<OsString> {
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
    args.push(file_arg(wav));
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
            "-c:a",
            "libmp3lame",
            "-ar",
            "44100",
            "-ac",
            "1",
            "-b:a",
        ]
        .map(OsString::from),
    );
    args.push(format!("{MP3_KBPS}k").into());
    args.extend(["-f", "mp3"].map(OsString::from));
    args.push(file_arg(mp3));
    args
}

/// Temporaires d'une conversion, supprimés à la destruction (succès, échec, annulation).
struct Scratch {
    wav: PathBuf,
    mp3: PathBuf,
}

impl Scratch {
    fn beside(output: &Path) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = output.file_name().unwrap_or_default().to_string_lossy();
        let tag = format!("{name}.{}-{n}", std::process::id());
        Self {
            wav: output.with_file_name(format!("{tag}.wav.part")),
            mp3: output.with_file_name(format!("{tag}.mp3.part")),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.wav);
        let _ = std::fs::remove_file(&self.mp3);
    }
}

/// Convertit un WAV synthétisé en MP3 mono dans `output` (dossier existant). Le WAV est écrit
/// dans un temporaire à côté de la sortie ; `output` n'existe qu'en cas de succès, aucun
/// temporaire ne reste en cas d'échec, d'annulation ou de délai. Rend la taille du MP3.
pub async fn wav_to_mp3(
    ff: &Ffmpeg,
    wav: &Wav,
    output: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<u64> {
    let verified = ff.verify(Tool::Ffmpeg, cancel).await?;
    wav_to_mp3_verified(&verified, wav, output, cancel).await
}

/// Comme [`wav_to_mp3`], avec un ffmpeg dont l'empreinte a déjà été vérifiée pour l'étape
/// ([`Ffmpeg::verify`]) : une série de titres ne recalcule pas le SHA-256 à chaque titre (#487).
pub async fn wav_to_mp3_verified(
    ffmpeg: &Verified,
    wav: &Wav,
    output: &Path,
    cancel: &mut watch::Receiver<bool>,
) -> AppResult<u64> {
    let scratch = Scratch::beside(output);
    tokio::fs::write(&scratch.wav, &wav.bytes).await?;
    let args = mp3_args(&scratch.wav, &scratch.mp3);
    let out = ffmpeg.run(&args, Limits::new(MP3_TIMEOUT), cancel).await?;
    finish_mp3(&out, &scratch, output, cancel).await
}

/// Après ffmpeg : sortie contrôlée, MP3 non vide, annulation vérifiée une dernière fois, puis
/// renommage vers `output`. Rend la taille du MP3.
async fn finish_mp3(
    out: &crate::ffmpeg::Output,
    scratch: &Scratch,
    output: &Path,
    cancel: &watch::Receiver<bool>,
) -> AppResult<u64> {
    if !out.success() {
        return Err(SpeechError::Failed("conversion MP3 en échec".into()).into());
    }
    let size = tokio::fs::metadata(&scratch.mp3).await?.len();
    if size == 0 {
        return Err(SpeechError::Failed("MP3 vide".into()).into());
    }
    if *cancel.borrow() {
        return Err(AppError::Cancelled);
    }
    tokio::fs::rename(&scratch.mp3, output).await?;
    Ok(size)
}

// --- lecture de l'aperçu -----------------------------------------------------------------------

/// Numéro du dernier aperçu demandé : une synthèse dépassée par un nouvel aperçu est jetée.
static PREVIEW_GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
mod playback {
    use std::ptr::{null, null_mut};
    use std::sync::{Mutex, PoisonError};

    use windows_sys::Win32::Media::Audio::{
        PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_PURGE,
    };

    use super::SpeechError;

    /// WAV en cours de lecture : `PlaySoundW` asynchrone relit ce tampon jusqu'à la fin, il doit
    /// donc vivre jusqu'à l'arrêt de la lecture.
    static BUFFER: Mutex<Vec<u8>> = Mutex::new(Vec::new());

    /// Joue `wav` si `is_current()` est encore vrai. Le test est fait sous le verrou de `BUFFER` :
    /// un `stop` concurrent, qui prend le même verrou, passe donc avant (le test échoue) ou après
    /// (il coupe ce son). Rend `false` si rien n'a été joué.
    pub fn play(wav: Vec<u8>, is_current: &dyn Fn() -> bool) -> Result<bool, SpeechError> {
        let mut buffer = BUFFER.lock().unwrap_or_else(PoisonError::into_inner);
        if !is_current() {
            return Ok(false);
        }
        // SAFETY : appel documenté ; SND_PURGE arrête la lecture en cours.
        unsafe { PlaySoundW(null(), null_mut(), SND_PURGE) };
        *buffer = wav;
        // SAFETY : avec SND_MEMORY le premier argument est l'adresse d'un WAV complet ;
        // le tampon reste valide (verrou, `BUFFER` statique) jusqu'au prochain arrêt.
        let ok = unsafe {
            PlaySoundW(
                buffer.as_ptr().cast(),
                null_mut(),
                SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
            )
        };
        if ok == 0 {
            buffer.clear();
            return Err(SpeechError::Failed("lecture audio impossible".into()));
        }
        Ok(true)
    }

    pub fn stop() {
        let mut buffer = BUFFER.lock().unwrap_or_else(PoisonError::into_inner);
        // SAFETY : voir `play`.
        unsafe { PlaySoundW(null(), null_mut(), SND_PURGE) };
        buffer.clear();
    }
}

#[cfg(not(windows))]
mod playback {
    use super::SpeechError;

    pub fn play(_wav: Vec<u8>, _is_current: &dyn Fn() -> bool) -> Result<bool, SpeechError> {
        Err(SpeechError::Unavailable)
    }

    pub fn stop() {}
}

// --- commandes ---------------------------------------------------------------------------------

/// Résultat d'un aperçu.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PreviewInfo {
    /// `false` : un aperçu plus récent (ou un arrêt) a rendu celui-ci inutile, rien n'est joué.
    pub played: bool,
    /// Durée du son joué, en millisecondes (0 si rien n'est joué).
    pub duration_ms: u64,
}

/// Voix françaises pour la liste de l'éditeur. Synthèse indisponible : liste vide (même
/// comportement que « aucune voix », la case est désactivée).
fn voices_for_front(synth: &Arc<dyn Synthesizer>) -> AppResult<VoiceList> {
    match french_voices_within(synth, VOICES_TOTAL_TIMEOUT) {
        Ok(list) => Ok(list),
        Err(SpeechError::Unavailable) => Ok(VoiceList {
            voices: Vec::new(),
            default_id: None,
        }),
        Err(e) => Err(e.into()),
    }
}

/// Prépare un aperçu : voix résolue (`voice` absent : voix par défaut), texte synthétisé.
fn prepare_preview(
    synth: &Arc<dyn Synthesizer>,
    text: &str,
    voice: Option<&str>,
) -> Result<Wav, SpeechError> {
    let list = french_voices_within(synth, VOICES_TOTAL_TIMEOUT)?;
    let voice = resolve_voice(&list, voice)?;
    synthesize_wav(synth, text, &voice.id, SYNTH_TIMEOUT)
}

/// Aperçu : numéro de génération pris, lecture en cours coupée (`stop`), synthèse, puis lecture
/// (`play`) si aucun aperçu plus récent n'a été demandé entre-temps (revérifié par `play` sous le
/// verrou du tampon audio).
fn preview_with(
    generation: &AtomicU64,
    synth: &Arc<dyn Synthesizer>,
    text: &str,
    voice: Option<&str>,
    stop: impl FnOnce(),
    play: impl FnOnce(Vec<u8>, &dyn Fn() -> bool) -> Result<bool, SpeechError>,
) -> Result<PreviewInfo, SpeechError> {
    let mine = generation.fetch_add(1, Ordering::SeqCst) + 1;
    stop();
    let result = prepare_preview(synth, text, voice);
    // Aperçu abandonné entre-temps : ni lecture, ni erreur pour une demande caduque.
    if generation.load(Ordering::SeqCst) != mine {
        return Ok(PreviewInfo {
            played: false,
            duration_ms: 0,
        });
    }
    let wav = result?;
    let duration_ms = (wav.duration_secs * 1000.0).round() as u64;
    // Le test de génération est refait par `play` sous son verrou (atomique avec le démarrage du
    // son) : un `stop` arrivé depuis le test ci-dessus l'emporte.
    let played = play(wav.bytes, &|| generation.load(Ordering::SeqCst) == mine)?;
    Ok(PreviewInfo {
        played,
        duration_ms: if played { duration_ms } else { 0 },
    })
}

fn join_error(_: tokio::task::JoinError) -> AppError {
    // Message fixe : celui d'une panique pourrait citer le texte du titre.
    AppError::TtsFailed("La synthèse vocale a été interrompue.".into())
}

/// Voix françaises installées (liste vide si aucune ou si la synthèse est indisponible).
#[tauri::command]
pub async fn tts_voices() -> AppResult<VoiceList> {
    let synth = system();
    tokio::task::spawn_blocking(move || voices_for_front(&synth))
        .await
        .map_err(join_error)?
}

/// Joue `text` avec la voix `voice` (défaut si absente) sur la sortie audio du poste. Un seul
/// aperçu à la fois : celui-ci coupe le précédent. Le son n'est pas renvoyé à l'interface.
#[tauri::command]
pub async fn tts_preview(text: String, voice: Option<String>) -> AppResult<PreviewInfo> {
    let synth = system();
    tokio::task::spawn_blocking(move || {
        preview_with(
            &PREVIEW_GENERATION,
            &synth,
            &text,
            voice.as_deref(),
            playback::stop,
            playback::play,
        )
    })
    .await
    .map_err(join_error)?
    .map_err(AppError::from)
}

/// Coupe l'aperçu en cours et annule celui qui se prépare.
#[tauri::command]
pub async fn tts_preview_stop() -> AppResult<()> {
    PREVIEW_GENERATION.fetch_add(1, Ordering::SeqCst);
    tokio::task::spawn_blocking(playback::stop)
        .await
        .map_err(join_error)
}

// --- WinRT -------------------------------------------------------------------------------------

#[cfg(windows)]
mod winrt {
    use windows::core::HSTRING;
    use windows::Media::SpeechSynthesis::{SpeechSynthesizer, VoiceInformation};
    use windows::Storage::Streams::DataReader;
    use windows::Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED};

    use super::{run_bounded, SpeechError, Synthesizer, Voice, MAX_WAV_BYTES, VOICES_TIMEOUT};

    /// Voix « OneCore » de Windows (Paramètres > Heure et langue > Voix).
    pub struct WinRtSynthesizer;

    /// Apartment multi-thread du thread courant, libéré à la destruction : `RoUninitialize`
    /// apparie chaque `RoInitialize` réussi (y compris `S_FALSE`, déjà initialisé). Un échec (mode
    /// déjà fixé autrement) n'est pas apparié. À déclarer avant les objets WinRT du thread, pour
    /// qu'ils soient détruits avant lui.
    struct Apartment(bool);

    impl Apartment {
        fn enter() -> Self {
            // SAFETY : appel documenté, sans pointeur.
            Self(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok())
        }
    }

    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY : apparié au `RoInitialize` réussi de `enter`, sur le même thread.
                unsafe { RoUninitialize() };
            }
        }
    }

    fn describe(voice: &VoiceInformation) -> windows::core::Result<Voice> {
        Ok(Voice {
            id: voice.Id()?.to_string(),
            name: voice.DisplayName()?.to_string(),
            language: voice.Language()?.to_string(),
        })
    }

    fn failure(e: &windows::core::Error) -> SpeechError {
        // Code seul : le message système pourrait citer des éléments du texte.
        SpeechError::Failed(format!("code Windows {:#010x}", e.code().0))
    }

    impl Synthesizer for WinRtSynthesizer {
        fn voices(&self) -> Result<Vec<Voice>, SpeechError> {
            // Thread dédié, borné dans le temps : l'apartment n'est pas laissé au pool bloquant.
            let listed = run_bounded(VOICES_TIMEOUT, || {
                let _apartment = Apartment::enter();
                let all = SpeechSynthesizer::AllVoices().map_err(|_| SpeechError::Unavailable)?;
                let mut voices = Vec::new();
                for voice in &all {
                    voices.push(describe(&voice).map_err(|_| SpeechError::Unavailable)?);
                }
                Ok(voices)
            });
            match listed {
                Ok(result) => result,
                // Service de voix qui ne répond pas : même traitement que « indisponible ».
                Err(SpeechError::Timeout) => Err(SpeechError::Unavailable),
                Err(e) => Err(e),
            }
        }

        fn default_voice_id(&self) -> Option<String> {
            run_bounded(VOICES_TIMEOUT, || {
                let _apartment = Apartment::enter();
                let voice = SpeechSynthesizer::DefaultVoice().ok()?;
                Some(voice.Id().ok()?.to_string())
            })
            .ok()
            .flatten()
        }

        fn synthesize(&self, text: &str, voice_id: &str) -> Result<Vec<u8>, SpeechError> {
            // Appelée depuis le thread dédié de `synthesize_wav`.
            let _apartment = Apartment::enter();
            let all = SpeechSynthesizer::AllVoices().map_err(|_| SpeechError::Unavailable)?;
            let voice = all
                .into_iter()
                .find(|v| v.Id().is_ok_and(|id| id == voice_id))
                .ok_or(SpeechError::VoiceGone)?;
            let synth = SpeechSynthesizer::new().map_err(|_| SpeechError::Unavailable)?;
            synth.SetVoice(&voice).map_err(|e| failure(&e))?;
            let stream = synth
                .SynthesizeTextToStreamAsync(&HSTRING::from(text))
                .and_then(|op| op.join())
                .map_err(|e| failure(&e))?;
            let size = stream.Size().map_err(|e| failure(&e))?;
            if size == 0 {
                return Err(SpeechError::Failed("audio vide".into()));
            }
            if size > MAX_WAV_BYTES as u64 {
                return Err(SpeechError::TooLong);
            }
            let size = size as u32;
            let reader = stream
                .GetInputStreamAt(0)
                .and_then(|input| DataReader::CreateDataReader(&input))
                .map_err(|e| failure(&e))?;
            let loaded = reader
                .LoadAsync(size)
                .and_then(|op| op.join())
                .map_err(|e| failure(&e))?;
            let mut bytes = vec![0u8; loaded as usize];
            reader.ReadBytes(&mut bytes).map_err(|e| failure(&e))?;
            Ok(bytes)
        }
    }
}

#[cfg(test)]
pub mod testing {
    //! Voix simulée déterministe : listes de voix configurables, WAV sinusoïdal dont la durée
    //! dépend de la longueur du texte. Sert aux tests de ce module et à ceux de la création de
    //! pack (PR 2) et de l'interface (PR 3).

    use super::*;

    /// Durée simulée par caractère du texte.
    pub const SECS_PER_CHAR: f64 = 0.05;

    pub struct Simulated {
        pub voices: Vec<Voice>,
        pub default_id: Option<String>,
        /// Textes (déjà préparés) dont la synthèse échoue.
        pub failing: Vec<String>,
        /// Attente avant de répondre (voix qui ne rend pas la main).
        pub delay: Duration,
        /// WAV rendu à la place du sinus (WAV corrompu, vide, trop long…).
        pub wav: Option<Vec<u8>>,
        /// Service de voix indisponible.
        pub unavailable: bool,
    }

    pub fn voice(id: &str, name: &str, language: &str) -> Voice {
        Voice {
            id: id.into(),
            name: name.into(),
            language: language.into(),
        }
    }

    impl Simulated {
        pub fn new(voices: Vec<Voice>) -> Self {
            let default_id = voices.first().map(|v| v.id.clone());
            Self {
                voices,
                default_id,
                failing: Vec::new(),
                delay: Duration::ZERO,
                wav: None,
                unavailable: false,
            }
        }

        /// Une voix française locale.
        pub fn french() -> Self {
            Self::new(vec![voice("fr-hortense", "Microsoft Hortense", "fr-FR")])
        }

        /// Aucune voix installée.
        pub fn none() -> Self {
            Self::new(Vec::new())
        }

        pub fn shared(self) -> Arc<dyn Synthesizer> {
            Arc::new(self)
        }
    }

    impl Synthesizer for Simulated {
        fn voices(&self) -> Result<Vec<Voice>, SpeechError> {
            if self.unavailable {
                return Err(SpeechError::Unavailable);
            }
            Ok(self.voices.clone())
        }

        fn default_voice_id(&self) -> Option<String> {
            self.default_id.clone()
        }

        fn synthesize(&self, text: &str, voice_id: &str) -> Result<Vec<u8>, SpeechError> {
            if self.unavailable {
                return Err(SpeechError::Unavailable);
            }
            if !self.voices.iter().any(|v| v.id == voice_id) {
                return Err(SpeechError::VoiceGone);
            }
            std::thread::sleep(self.delay);
            if self.failing.iter().any(|t| t == text) {
                return Err(SpeechError::Failed("voix simulée en échec".into()));
            }
            Ok(match &self.wav {
                Some(bytes) => bytes.clone(),
                None => sine_wav(text.chars().count() as f64 * SECS_PER_CHAR),
            })
        }
    }

    /// WAV PCM 16 bits mono 22 050 Hz : sinus de 440 Hz.
    pub fn sine_wav(secs: f64) -> Vec<u8> {
        const RATE: u32 = 22_050;
        let frames = (secs * f64::from(RATE)) as usize;
        let mut data = Vec::with_capacity(frames * 2);
        for i in 0..frames {
            let t = i as f64 / f64::from(RATE);
            let sample = (t * 440.0 * std::f64::consts::TAU).sin() * 8_000.0;
            data.extend_from_slice(&(sample as i16).to_le_bytes());
        }
        wav_bytes(RATE, 1, 16, &data)
    }

    /// En-tête WAV canonique (44 octets) suivi de `data`.
    pub fn wav_bytes(rate: u32, channels: u16, bits: u16, data: &[u8]) -> Vec<u8> {
        let block = channels * bits / 8;
        let mut out = Vec::with_capacity(44 + data.len());
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(block)).to_le_bytes());
        out.extend_from_slice(&block.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(data);
        out
    }
}

#[cfg(test)]
mod tests;
