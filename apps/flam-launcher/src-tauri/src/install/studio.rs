//! Conversion d'une histoire STUdio (zip : `story.json`, `assets/`, `thumbnail.png`) en pack
//! Lunii **clair** `.lunii.pk`, à l'installation (#541, PR « G1 » de l'épopée #517 ; note de
//! conception `docs/studio-lunii.md`, § 2, 3, 5.2 et 6).
//!
//! Le pack produit est le contrat lu par [`crate::device::prepare_lunii`] (chiffrement à
//! l'envoi) : zip « stored », entrées dans cet ordre, dates fixes (1980-01-01) :
//!
//! | Entrée              | Contenu                                                                   |
//! |---------------------|---------------------------------------------------------------------------|
//! | `uuid.bin`          | UUID du pack (celui du nœud d'entrée), 16 octets                          |
//! | `_metadata.json`    | `title`, `description` (assainis, UTF-8), `version`, `uuid`, `source_format` = `studio` |
//! | `_thumbnail.png`    | `thumbnail.png` d'origine, s'il a servi de vignette et est bien un PNG    |
//! | `ni`, `nm`          | index des nœuds ; `nm` vide si `nightModeAvailable`                       |
//! | `li`, `ri`, `si`    | index en clair (`studio2lunii::index`)                                    |
//! | `info`              | ASCII, LF, 5 lignes : titre, titre, `0`, titre, `No authors.` (§ 3.8, Q4) |
//! | `img/thumbnail.lif` | vignette 104 × 136 (`telmi2flam`), si une source existe                   |
//! | `rf/000/<NOM>`      | images en BMP 4 bits RLE 320 × 240 (`studio2lunii::bitmap`)               |
//! | `sf/000/<NOM>`      | sons MP3 MPEG-1 couche III, 44,1 kHz, mono, sans étiquette                |
//!
//! Sons : un MP3 conforme (`studio2lunii::audio::transcoding_required` faux) est gardé, ses
//! étiquettes retirées (seule la plage audio, `mp3_audio_range`, est copiée : aucune seconde
//! copie du son) ; tout autre son est transcodé par le ffmpeg embarqué ([`crate::ffmpeg`],
//! démuxeurs limités à [`FFMPEG_INPUT_FORMATS`], arguments `FFMPEG_TRANSCODE_ARGS`, empreinte
//! vérifiée une fois par conversion), puis la sortie est recontrôlée (refus si elle n'est pas
//! conforme) et nettoyée. Un son dont le nom n'est pas en `.mp3` est copié du zip vers le
//! dossier temporaire en flux, sans passer par la mémoire.
//!
//! ffmpeg n'est exigé que si un son doit être transcodé. Une pré-analyse bon marché (extension,
//! puis en-tête des [`PROBE_BYTES`] premiers octets de chaque MP3) repère avant toute
//! conversion un son qui devra l'être : ffmpeg absent est alors refusé d'emblée, et son
//! empreinte vérifiée avant les images. Un son que seule la lecture complète déclare non
//! conforme (trames illisibles plus loin) n'est découvert qu'à son tour.
//!
//! Vignette : `thumbnail.png`, sinon l'image du nœud d'entrée (PNG, JPEG ou BMP), sinon aucune
//! (`prepare_lunii` l'accepte). Dimensions lues et bornées avant tout décodage (bornes de
//! `studio2lunii::bitmap`). Une source de vignette illisible, trop lourde ou trop grande est un
//! **avertissement**
//! ([`StudioPack::warnings`]), jamais un refus : la vignette n'est pas une scène de l'histoire.
//!
//! # Invariants
//!
//! 1. Tout est lu et contrôlé avant d'écrire la sortie : noms d'entrées (`device::prepare` :
//!    `..`, absolu, `\`, lecteur, noms réservés, doublons à la casse près, liens symboliques),
//!    entrées chiffrées, racine de l'histoire, `story.json` (`parse_story`), index
//!    (`build_index`), présence de **chaque** asset référencé (#535), vignette.
//! 2. Aucun nom de l'archive ne devient un chemin de sortie : les noms du pack sont des
//!    constantes ou viennent de `build_index` ; les fichiers temporaires ont des noms fixes.
//! 3. Écriture atomique : tout est écrit dans un dossier temporaire voisin de la sortie
//!    (`.<sortie>.<aléa>.tmp/`), le pack est fermé, synchronisé puis renommé sur la sortie
//!    (remplacée si elle existe). Erreur ou annulation : le dossier temporaire est supprimé et la
//!    sortie n'est ni créée ni modifiée.
//! 4. Coût borné et linéaire (note § 6.2) : entrées du zip, tailles déclarées **puis lues**,
//!    `story.json`, images (32 Mio), sons (512 Mio), total lu et total écrit (4 Gio), délai de
//!    ffmpeg (120 s par son, [`FFMPEG_TOTAL_BUDGET`] pour tous les sons d'une conversion),
//!    pré-analyse ([`PROBE_BYTES`] par son). Aucun parcours du graphe. Mémoire : un son au
//!    plus (512 Mio) est tenu en mémoire à la fois, sans copie.
//! 5. Conversion déterministe : mêmes entrées (et même ffmpeg), mêmes octets.
//! 6. Aucune clé : le pack est en clair, indépendant de l'appareil.
//!
//! # Pannes
//!
//! | Situation                                                     | Erreur                                            |
//! |---------------------------------------------------------------|---------------------------------------------------|
//! | Zip illisible, nom hostile, lien, entrée chiffrée, `story.json` hors racine, archive ambiguë (`uuid.bin`) | `conversion` `invalid_archive` |
//! | `story.json` invalide (format, graphe, références, noms)      | `conversion` `invalid_story`                      |
//! | Asset référencé absent de l'archive                           | `conversion` `missing_asset`                      |
//! | Borne dépassée (entrées, tailles déclarées ou lues, total)    | `conversion` `too_large`                          |
//! | Image illisible ou hors bornes                                | `conversion` `unreadable_image`                   |
//! | MP3 aux étiquettes incohérentes                               | `conversion` `unreadable_audio`                   |
//! | ffmpeg en échec, sortie non conforme                          | `conversion` `ffmpeg_failed`                      |
//! | ffmpeg absent, empreinte fausse                               | `converter_unavailable`                           |
//! | Délai de ffmpeg dépassé (par son ou budget global)            | `io` (`TimedOut`)                                 |
//! | Annulation                                                    | `cancelled`                                       |
//! | Écriture impossible (disque plein, sortie inaccessible)       | `io`                                              |
//! | Lecture de l'archive refusée par le système (antivirus, fichier verrouillé, volume retiré : erreur portant un code de l'OS ; `EIO` compris sous Unix) | `io` (archive non mise en cause, #578 ; erreur persistante : « Vider le cache », si `archives/` est marqué, voir `install::system_io`) |
//! | Contenu stocké illisible sous Windows (`ERROR_CRC`, `ERROR_FILE_CORRUPT`, `ERROR_DISK_CORRUPT`), positionnement refusé (`InvalidInput`) | `conversion` `invalid_archive` |
//!
//! Dans tous ces cas, rien ne reste du côté de la sortie, sauf si le futur est abandonné
//! pendant une étape bloquante (fichier encore ouvert sous Windows) : le dossier temporaire
//! `.…tmp/` peut alors subsister dans le dossier de sortie (`library/.staging/` en G2).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use studio2lunii::audio::{
    mp3_audio_range, probe_mp3, transcoding_required, MpegVersion, ACCEPTED_SAMPLE_RATE_HZ,
    FFMPEG_TRANSCODE_ARGS, TARGET_CHANNELS,
};
use studio2lunii::bitmap::{to_lunii_bmp, MAX_INPUT_BYTES as MAX_IMAGE_BYTES};
use studio2lunii::index::{build_index, AssetKind, PackIndex};
use studio2lunii::story::{parse_story, Story, MAX_STORY_JSON_BYTES};
use tokio::sync::{watch, OnceCell};
use unicode_normalization::char::canonical_combining_class;
use unicode_normalization::UnicodeNormalization;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::convert::Step;
use super::podcast::BoxFuture;
use crate::catalog::blocking;
use crate::device::prepare::{for_each_entry, Archive, PrepareError};
use crate::ffmpeg::{self, Ffmpeg, Tool, Verified};
use crate::podcast::audio::file_arg;
use crate::settings::ThumbFit;
use crate::{AppError, AppResult};
// Longueur maximale du titre (caractères) : constante unique, celle du scan des dossiers.
use crate::story::MAX_TITLE_CHARS;

/// Entrées maximales de l'archive (note § 6.2).
pub const MAX_ENTRIES: usize = 100_000;
/// Total décompressé (déclaré, puis lu) et total écrit dans le pack (note § 6.2).
pub const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Taille maximale d'un son source, et d'un son transcodé relu (note § 6.2).
pub const MAX_SOUND_BYTES: u64 = 512 * 1024 * 1024;
/// Délai de ffmpeg par son (note § 6.2, comme `telmi2flam`).
pub const FFMPEG_TIMEOUT: Duration = Duration::from_secs(120);
/// Budget de temps de ffmpeg pour l'ensemble des sons d'une conversion (note § 6.2) : au-delà,
/// le transcodage en cours est tué et la conversion échoue (`TimedOut`). Une heure laisse
/// largement passer une histoire réelle (quelques secondes par son) et arrête une archive
/// forgée de milliers de sons lents.
pub const FFMPEG_TOTAL_BUDGET: Duration = Duration::from_secs(60 * 60);
/// Démuxeurs autorisés en entrée de ffmpeg (`-format_whitelist`) : les formats de son acceptés
/// par STUdio (note § 2.3) — MP3, OGG (Vorbis, Opus), WAV, FLAC, AAC (ADTS), M4A (`mov`).
pub const FFMPEG_INPUT_FORMATS: &str = "mp3,ogg,wav,flac,aac,mov";
/// Octets lus en tête de chaque MP3 par la pré-analyse (note § 6.2).
pub const PROBE_BYTES: u64 = 64 * 1024;
/// Phases annoncées par la progression : `images`, `audio`, `zip`.
pub const PHASES: usize = 3;

/// Codes stables des erreurs `conversion` (voir le tableau des pannes).
pub const INVALID_ARCHIVE: &str = "invalid_archive";
pub const INVALID_STORY: &str = "invalid_story";
pub const MISSING_ASSET: &str = "missing_asset";
pub const TOO_LARGE: &str = "too_large";
pub const UNREADABLE_IMAGE: &str = "unreadable_image";
pub const UNREADABLE_AUDIO: &str = "unreadable_audio";
pub const FFMPEG_FAILED: &str = "ffmpeg_failed";

const STORY_JSON: &str = "story.json";
const THUMBNAIL_PNG: &str = "thumbnail.png";
const MAX_DESCRIPTION_CHARS: usize = 2_000;
/// Ligne d'auteur de `info`, comme Lunii.QT (note § 3.8, Q4).
const NO_AUTHORS: &str = "No authors.";
/// Titre de repli (titre absent, ou sans lettre ni chiffre une fois translittéré).
const UNTITLED: &str = "Sans titre";
/// Noms fixes dans le dossier temporaire.
const PACK_PART: &str = "pack.part";
const SOUND_IN: &str = "son-source";
const SOUND_OUT: &str = "son-transcode.mp3";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

/// Une conversion.
pub struct StudioJob<'a> {
    /// Zip STUdio à convertir (jamais modifié).
    pub input: &'a Path,
    /// Pack `.lunii.pk` à produire ; son dossier doit exister. Remplacé s'il existe.
    pub output: &'a Path,
    /// ffmpeg embarqué localisé ; `None` : absent (refus seulement si un son doit être
    /// transcodé).
    pub ffmpeg: Option<&'a Ffmpeg>,
    /// Remplissage de la vignette 104 × 136.
    pub thumb_fit: ThumbFit,
}

/// Pack produit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudioPack {
    /// UUID du pack (nœud d'entrée), en minuscules avec tirets.
    pub uuid: String,
    /// `version` de `story.json`.
    pub version: u16,
    /// Titre assaini (UTF-8), celui de `_metadata.json`.
    pub title: String,
    /// Description assainie (UTF-8, vide si absente).
    pub description: String,
    pub night_mode: bool,
    /// Images (`rf/000/`) et sons (`sf/000/`) du pack.
    pub images: usize,
    pub sounds: usize,
    /// Sons transcodés par ffmpeg.
    pub transcoded: usize,
    /// `img/thumbnail.lif` présent.
    pub thumbnail: bool,
    /// Avertissements (vignette illisible).
    pub warnings: Vec<String>,
    /// Taille du pack écrit.
    pub bytes: u64,
}

/// Transcodage d'un son vers un MP3 (ffmpeg embarqué ; simulé dans les tests).
pub trait Transcoder: Send + Sync {
    /// Transcode `input` en `output` (MP3) ; `AppError::Cancelled` si `cancel` passe à vrai.
    fn transcode<'a>(
        &'a self,
        input: &'a Path,
        output: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<()>>;

    /// Contrôle anticipé, avant toute conversion, quand la pré-analyse a trouvé un son à
    /// transcoder (ffmpeg embarqué : vérification de l'empreinte). Par défaut, rien.
    fn prepare<'a>(&'a self, cancel: &'a watch::Receiver<bool>) -> BoxFuture<'a, AppResult<()>> {
        let _ = cancel;
        Box::pin(async { Ok(()) })
    }
}

/// ffmpeg embarqué : empreinte vérifiée au premier son transcodé, une fois par conversion.
struct EmbeddedFfmpeg {
    ffmpeg: Ffmpeg,
    verified: OnceCell<Verified>,
}

fn ffmpeg_unavailable(why: impl std::fmt::Display) -> AppError {
    AppError::ConverterUnavailable(format!(
        "ffmpeg embarqué indisponible, nécessaire pour convertir les sons de cette histoire \
         STUdio (réinstallez Flam Launcher) : {why}"
    ))
}

/// Dernière ligne non vide de stderr, bornée.
fn last_line(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    line.chars().take(200).collect()
}

impl EmbeddedFfmpeg {
    /// Exécutable vérifié (empreinte calculée au premier appel seulement).
    async fn verified(&self, cancel: &watch::Receiver<bool>) -> AppResult<&Verified> {
        self.verified
            .get_or_try_init(|| async {
                let mut cancel = cancel.clone();
                self.ffmpeg
                    .verify(Tool::Ffmpeg, &mut cancel)
                    .await
                    .map_err(|e| match e {
                        AppError::ConverterUnavailable(m) => ffmpeg_unavailable(m),
                        other => other,
                    })
            })
            .await
    }
}

/// Arguments d'entrée de ffmpeg, avant le fichier source : aucun protocole hors `file`, et
/// seulement les démuxeurs des formats de son attendus (le format reste détecté au contenu ;
/// un autre est refusé, « Format not on whitelist »).
const FFMPEG_INPUT_ARGS: &[&str] = &[
    "-hide_banner",
    "-nostdin",
    "-v",
    "error",
    "-y",
    "-protocol_whitelist",
    "file",
    "-format_whitelist",
    FFMPEG_INPUT_FORMATS,
    "-i",
];

impl Transcoder for EmbeddedFfmpeg {
    fn prepare<'a>(&'a self, cancel: &'a watch::Receiver<bool>) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move { self.verified(cancel).await.map(|_| ()) })
    }

    fn transcode<'a>(
        &'a self,
        input: &'a Path,
        output: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let verified = self.verified(cancel).await?;
            let mut args: Vec<OsString> = FFMPEG_INPUT_ARGS.iter().map(OsString::from).collect();
            args.push(file_arg(input));
            args.extend(FFMPEG_TRANSCODE_ARGS.iter().map(OsString::from));
            // Sortie bornée sur le disque (relue ensuite avec la même borne).
            args.push("-fs".into());
            args.push(MAX_SOUND_BYTES.to_string().into());
            args.push(file_arg(output));
            let mut cancel = cancel.clone();
            let out = verified
                .run(&args, ffmpeg::Limits::new(FFMPEG_TIMEOUT), &mut cancel)
                .await?;
            if !out.success() {
                return Err(conversion(
                    FFMPEG_FAILED,
                    format!(
                        "ffmpeg n'a pas pu convertir un son (code {:?}) : {}",
                        out.code,
                        last_line(&out.stderr)
                    ),
                ));
            }
            Ok(())
        })
    }
}

/// Convertit le zip STUdio `job.input` en pack clair `job.output`. `on_step` reçoit
/// l'avancement (phases `images`, `audio`, `zip`). Voir l'en-tête du module.
pub async fn convert(
    job: &StudioJob<'_>,
    cancel: &watch::Receiver<bool>,
    on_step: &mut (dyn FnMut(&Step) + Send),
) -> AppResult<StudioPack> {
    let transcoder = job.ffmpeg.map(|ff| EmbeddedFfmpeg {
        ffmpeg: ff.clone(),
        verified: OnceCell::new(),
    });
    convert_with(
        job.input,
        job.output,
        job.thumb_fit,
        transcoder.as_ref().map(|t| t as &dyn Transcoder),
        FFMPEG_TOTAL_BUDGET,
        cancel,
        on_step,
    )
    .await
}

/// [`convert`] avec un transcodeur injecté (`None` : ffmpeg absent) et le budget de temps
/// global des transcodages (`budget`, [`FFMPEG_TOTAL_BUDGET`] en production).
pub(crate) async fn convert_with(
    input: &Path,
    output: &Path,
    fit: ThumbFit,
    transcoder: Option<&dyn Transcoder>,
    budget: Duration,
    cancel: &watch::Receiver<bool>,
    on_step: &mut (dyn FnMut(&Step) + Send),
) -> AppResult<StudioPack> {
    check_cancel(cancel)?;
    let tmp = {
        let (input, output) = (input.to_path_buf(), output.to_path_buf());
        blocking(move || {
            refuse_same_file(&input, &output)?;
            TempDir::create(&output)
        })
        .await??
    };
    let options = RunOptions {
        fit,
        transcoder,
        budget,
    };
    let result = run(input, output, &tmp.0, &options, cancel, on_step).await;
    // Suppression hors du runtime ; `Drop` la refait (sans effet) si elle a échoué.
    let _ = blocking(move || drop(tmp)).await;
    result
}

/// La sortie ne doit jamais remplacer l'archive source (invariant : source jamais modifiée).
fn refuse_same_file(input: &Path, output: &Path) -> AppResult<()> {
    if let (Ok(a), Ok(b)) = (std::fs::canonicalize(input), std::fs::canonicalize(output)) {
        if a == b {
            return Err(AppError::Validation(
                "la sortie de la conversion STUdio ne peut pas être l'archive source".into(),
            ));
        }
    }
    Ok(())
}

fn check_cancel(cancel: &watch::Receiver<bool>) -> AppResult<()> {
    if *cancel.borrow() {
        Err(AppError::Cancelled)
    } else {
        Ok(())
    }
}

fn conversion(code: &str, message: impl Into<String>) -> AppError {
    AppError::Conversion {
        code: code.to_string(),
        message: message.into(),
    }
}

fn invalid_archive(message: impl std::fmt::Display) -> AppError {
    conversion(
        INVALID_ARCHIVE,
        format!("archive STUdio refusée : {message}"),
    )
}

fn too_large(message: impl std::fmt::Display) -> AppError {
    conversion(
        TOO_LARGE,
        format!("archive STUdio trop volumineuse : {message}"),
    )
}

/// Erreurs des contrôles partagés avec la préparation des histoires Flam.
fn archive_error(err: PrepareError) -> AppError {
    match err {
        PrepareError::Io(e) => AppError::Io(e),
        PrepareError::Cancelled => AppError::Cancelled,
        PrepareError::TooLarge(m) | PrepareError::LuniiTooLarge(m) => too_large(m),
        PrepareError::Invalid(m) | PrepareError::LuniiInvalid(m) => invalid_archive(m),
        other @ PrepareError::LuniiNeedsV2 => invalid_archive(other),
    }
}

/// Échec de lecture de l'archive (`context` : ce qui était lu). Erreur d'entrée/sortie du
/// système ([`super::system_io`] : antivirus, fichier verrouillé, volume retiré) : `io`, l'archive
/// n'est pas mise en cause et reste réutilisable ; sinon (CRC, en-tête, troncature, flux
/// compressé invalide) : `invalid_archive` (#578).
pub(super) fn read_error(context: &str, err: std::io::Error) -> AppError {
    if super::system_io(&err) {
        // Erreur d'origine gardée telle quelle (code de l'OS compris) ; contexte journalisé.
        tracing::warn!("lecture de l'archive STUdio refusée par le système ({context}) : {err}");
        AppError::Io(err)
    } else {
        invalid_archive(format!("{context} : {err}"))
    }
}

/// [`read_error`] pour une erreur du lecteur zip.
pub(super) fn zip_read_error(context: &str, err: zip::result::ZipError) -> AppError {
    match err {
        zip::result::ZipError::Io(e) => read_error(context, e),
        other => invalid_archive(format!("{context} : {other}")),
    }
}

fn zip_error(err: zip::result::ZipError) -> AppError {
    match err {
        zip::result::ZipError::Io(e) => AppError::Io(e),
        other => AppError::Io(std::io::Error::other(other.to_string())),
    }
}

// ---------------------------------------------------------------------------------------
// Dossier temporaire
// ---------------------------------------------------------------------------------------

/// Dossier temporaire voisin de la sortie, supprimé à la destruction.
struct TempDir(PathBuf);

impl TempDir {
    fn create(output: &Path) -> AppResult<Self> {
        let parent = output
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .ok_or_else(|| AppError::Validation("sortie sans dossier parent".into()))?;
        let name = output
            .file_name()
            .ok_or_else(|| AppError::Validation("sortie sans nom de fichier".into()))?
            .to_string_lossy();
        let dir = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.0) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => tracing::warn!(
                dossier = %self.0.display(),
                "dossier temporaire de conversion STUdio non supprimé : {e}"
            ),
        }
    }
}

// ---------------------------------------------------------------------------------------
// Lecture de l'archive
// ---------------------------------------------------------------------------------------

/// Archive STUdio ouverte, entrées contrôlées.
struct Source {
    zip: Archive,
    /// Fichiers sous la racine de l'histoire : chemin relatif à la racine → indice.
    files: HashMap<String, usize>,
    /// Mêmes chemins en minuscules (doublons à la casse près déjà refusés).
    lower: HashMap<String, usize>,
    /// Octets décompressés lus jusqu'ici.
    read: u64,
}

/// Entrée sans rôle dans l'histoire : `__MACOSX/…` ou `._*` (ressources macOS).
fn is_ignored(relative: &str) -> bool {
    let first = relative.split('/').next().unwrap_or(relative);
    let last = relative.rsplit('/').next().unwrap_or(relative);
    first.eq_ignore_ascii_case("__MACOSX") || last.starts_with("._")
}

/// Racine de l'histoire (note § 2.1, Q8) : `""` si `story.json` est à la racine, `"<dossier>/"`
/// si **toutes** les entrées sont sous ce dossier unique et qu'il contient `story.json`.
fn story_root(names: &[&str]) -> AppResult<String> {
    let has_story = |root: &str| {
        names.iter().any(|n| {
            n.strip_prefix(root)
                .is_some_and(|rest| rest.eq_ignore_ascii_case(STORY_JSON))
        })
    };
    if has_story("") {
        return Ok(String::new());
    }
    if let Some((dir, _)) = names.first().and_then(|n| n.split_once('/')) {
        let root = format!("{dir}/");
        if names.iter().all(|n| n.starts_with(&root)) && has_story(&root) {
            return Ok(root);
        }
    }
    Err(invalid_archive(
        "story.json introuvable à la racine de l'archive (ni dans un dossier unique)",
    ))
}

impl Source {
    fn open(input: &Path) -> AppResult<Self> {
        // Ouverture faite ici plutôt que par `device::prepare::open_archive`, qui classe toute
        // erreur du répertoire central en archive invalide : une erreur d'entrée/sortie du
        // système n'y met pas l'archive en cause (#578, [`read_error`]).
        let mut zip: Archive = zip::ZipArchive::new(BufReader::new(File::open(input)?))
            .map_err(|e| zip_read_error("zip", e))?;
        if zip.len() > MAX_ENTRIES {
            return Err(too_large(format!(
                "{} entrées (au plus {MAX_ENTRIES})",
                zip.len()
            )));
        }
        // En-têtes locaux lus (et chiffrement refusé) **avant** `for_each_entry`, qui classe
        // toute erreur de lecture en archive invalide : la position d'un en-tête lu est mémorisée
        // par le lecteur zip, `for_each_entry` ne fait donc plus qu'un `seek`, sans lecture.
        for index in 0..zip.len() {
            let entry = zip
                .by_index_raw(index)
                .map_err(|e| zip_read_error(&format!("entrée {index}"), e))?;
            if entry.encrypted() {
                return Err(invalid_archive(
                    "archive protégée par mot de passe : décompressez-la puis recompressez-la \
                     sans mot de passe",
                ));
            }
        }
        let mut entries = Vec::new();
        let mut declared = 0u64;
        for_each_entry(&mut zip, |index, relative, size| {
            declared = declared.saturating_add(size);
            entries.push((relative, index));
            Ok(())
        })
        .map_err(archive_error)?;
        if declared > MAX_TOTAL_BYTES {
            return Err(too_large(format!(
                "{declared} octets annoncés (au plus {MAX_TOTAL_BYTES})"
            )));
        }
        entries.retain(|(name, _)| !is_ignored(name));
        let names: Vec<&str> = entries.iter().map(|(n, _)| n.as_str()).collect();
        let root = story_root(&names)?;
        let mut files = HashMap::with_capacity(entries.len());
        let mut lower = HashMap::with_capacity(entries.len());
        for (name, index) in entries {
            if let Some(relative) = name.strip_prefix(&root) {
                lower.insert(relative.to_lowercase(), index);
                files.insert(relative.to_string(), index);
            }
        }
        if lower.contains_key("uuid.bin") {
            return Err(invalid_archive(
                "archive ambiguë : elle contient à la fois story.json (STUdio) et uuid.bin \
                 (histoire Flam .plain.pk)",
            ));
        }
        Ok(Self {
            zip,
            files,
            lower,
            read: 0,
        })
    }

    /// Indice d'un fichier sous la racine : nom exact, sinon sans la casse.
    fn find(&self, relative: &str) -> Option<usize> {
        self.files
            .get(relative)
            .or_else(|| self.lower.get(&relative.to_lowercase()))
            .copied()
    }

    /// Lit l'entrée `index` en entier, au plus `max` octets (taille déclarée puis lue).
    fn read(&mut self, index: usize, max: u64, what: &str) -> AppResult<Vec<u8>> {
        let mut entry = self
            .zip
            .by_index(index)
            .map_err(|e| zip_read_error(&format!("{what} illisible"), e))?;
        if entry.size() > max {
            return Err(too_large(format!(
                "{what} : {} octets annoncés (au plus {max})",
                entry.size()
            )));
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(max.saturating_add(1))
            .read_to_end(&mut data)
            .map_err(|e| read_error(&format!("{what} illisible"), e))?;
        if data.len() as u64 > max {
            return Err(too_large(format!("{what} dépasse {max} octets")));
        }
        drop(entry);
        self.count(data.len() as u64)?;
        Ok(data)
    }

    /// Ajoute `n` octets lus au total décompressé, borné.
    fn count(&mut self, n: u64) -> AppResult<()> {
        self.read = self.read.saturating_add(n);
        if self.read > MAX_TOTAL_BYTES {
            return Err(too_large(format!(
                "plus de {MAX_TOTAL_BYTES} octets décompressés"
            )));
        }
        Ok(())
    }

    /// Copie l'entrée `index` dans le fichier `path` (créé) en flux, au plus `max` octets
    /// (taille déclarée puis lue) : rien n'est tenu en mémoire au-delà d'un tampon.
    fn copy_to(&mut self, index: usize, max: u64, what: &str, path: &Path) -> AppResult<()> {
        let mut entry = self
            .zip
            .by_index(index)
            .map_err(|e| zip_read_error(&format!("{what} illisible"), e))?;
        if entry.size() > max {
            return Err(too_large(format!(
                "{what} : {} octets annoncés (au plus {max})",
                entry.size()
            )));
        }
        let mut out = BufWriter::new(File::create_new(path)?);
        let mut buf = vec![0u8; 64 * 1024];
        let mut copied = 0u64;
        loop {
            let n = entry
                .read(&mut buf)
                .map_err(|e| read_error(&format!("{what} illisible"), e))?;
            if n == 0 {
                break;
            }
            copied += n as u64;
            if copied > max {
                return Err(too_large(format!("{what} dépasse {max} octets")));
            }
            out.write_all(&buf[..n])?;
        }
        out.into_inner().map_err(|e| e.into_error())?;
        drop(entry);
        self.count(copied)
    }

    /// Les `max` premiers octets de l'entrée `index`, pour la pré-analyse (hors du total lu :
    /// [`PROBE_BYTES`] par son au plus). Une entrée illisible rend ce qui a pu être lu : la
    /// lecture complète la refusera à son tour.
    fn prefix(&mut self, index: usize, max: u64) -> Vec<u8> {
        let mut data = Vec::new();
        if let Ok(entry) = self.zip.by_index(index) {
            let _ = entry.take(max).read_to_end(&mut data);
        }
        data
    }
}

// ---------------------------------------------------------------------------------------
// Textes : titre, description, `info`
// ---------------------------------------------------------------------------------------

/// Caractère Unicode de format invisible (catégorie `Cf` et assimilés) : contrôles
/// bidirectionnels (U+061C, U+200E/F, U+202A–U+202E, U+2066–U+2069), espace et liants sans
/// chasse (U+200B, U+2060–U+2064, U+FEFF), trait d'union conditionnel, séparateur vocalique
/// mongol, formats déconseillés (U+206A–U+206F), annotations interlinéaires (U+FFF9–U+FFFB),
/// étiquettes (U+E0001, U+E0020–U+E007F). Ils retourneraient ou masqueraient le texte affiché
/// (« Trojan Source »). ZWNJ et ZWJ (U+200C, U+200D) sont gardés dans le texte UTF-8 : ils
/// servent à l'écriture (persan, langues indiennes) et aux émojis composés ; la
/// translittération ASCII les retire ([`ascii_text`]).
fn is_format_char(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200E}'
            | '\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Texte sur une ligne : caractères de format retirés, contrôles et blancs consécutifs réduits
/// à une espace, bords rognés, longueur bornée (#535, #553).
pub(crate) fn clean_line(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .filter(|&c| !is_format_char(c))
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let joined = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.chars().take(max_chars).collect()
}

/// Translittération ASCII imprimable (à la manière de `unidecode`, utilisé par Lunii.QT) :
/// décomposition NFKD, diacritiques retirés, ligatures et ponctuation typographique
/// remplacées, tout autre caractère devient `?`.
pub(crate) fn ascii_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.nfkd() {
        if c.is_ascii() {
            out.push(if c.is_ascii_control() { ' ' } else { c });
            continue;
        }
        if canonical_combining_class(c) != 0
            || is_format_char(c)
            || matches!(c, '\u{200C}' | '\u{200D}')
        {
            continue;
        }
        let replacement = match c {
            'Œ' => "OE",
            'œ' => "oe",
            'Æ' => "AE",
            'æ' => "ae",
            'ß' => "ss",
            'Ø' => "O",
            'ø' => "o",
            'Đ' | 'Ð' => "D",
            'đ' | 'ð' => "d",
            'Ł' => "L",
            'ł' => "l",
            'Þ' => "TH",
            'þ' => "th",
            'ı' => "i",
            '‘' | '’' | '‚' | '‛' | '′' | '`' => "'",
            '“' | '”' | '„' | '‟' | '″' | '«' | '»' => "\"",
            '‐' | '‑' | '‒' | '–' | '—' | '―' | '−' => "-",
            '•' | '·' => "-",
            '€' => "EUR",
            c if c.is_whitespace() => " ",
            _ => "?",
        };
        out.push_str(replacement);
    }
    clean_line(&out, usize::MAX)
}

/// Titre assaini (UTF-8), avec repli.
fn display_title(story: &Story) -> String {
    let title = clean_line(story.title(), MAX_TITLE_CHARS);
    if title.is_empty() {
        UNTITLED.to_string()
    } else {
        title
    }
}

/// Contenu de `info` (note § 3.8, Q4) : titre ASCII, LF, sans fin de ligne finale.
pub(crate) fn info_bytes(title: &str) -> Vec<u8> {
    let ascii = ascii_text(title);
    let ascii = if ascii.chars().any(|c| c.is_ascii_alphanumeric()) {
        ascii
    } else {
        UNTITLED.to_string()
    };
    format!("{ascii}\n{ascii}\n0\n{ascii}\n{NO_AUTHORS}").into_bytes()
}

// ---------------------------------------------------------------------------------------
// Plan : tout ce qui se contrôle avant d'écrire
// ---------------------------------------------------------------------------------------

struct Thumbnail {
    lif: Vec<u8>,
    /// `thumbnail.png` d'origine (absent si la vignette vient de l'image du nœud d'entrée).
    png: Option<Vec<u8>>,
}

struct Plan {
    story: Story,
    index: PackIndex,
    /// Indice dans l'archive de chaque asset de `index.assets` (même ordre).
    sources: Vec<usize>,
    thumbnail: Option<Thumbnail>,
    warnings: Vec<String>,
    title: String,
    description: String,
    /// Premier son que la pré-analyse sait déjà devoir transcoder.
    first_to_transcode: Option<String>,
}

/// Un son au nom autre que `.mp3` est toujours transcodé (règle de
/// `studio2lunii::audio::transcoding_required`, qui ne lit alors pas ses octets).
fn transcoded_by_name(name: &str) -> bool {
    !name.to_ascii_lowercase().ends_with(".mp3")
}

/// Pré-analyse bon marché : premier son qui devra **certainement** être transcodé, d'après son
/// extension, sinon d'après la première trame (confirmée) lue dans ses [`PROBE_BYTES`] premiers
/// octets. Un doute (en-tête illisible dans ce préfixe, étiquette plus longue) ne conclut rien :
/// la lecture complète tranchera à son tour.
fn first_to_transcode(
    source: &mut Source,
    index: &PackIndex,
    sources: &[usize],
    cancel: &watch::Receiver<bool>,
) -> AppResult<Option<String>> {
    let sounds = || {
        index
            .assets
            .iter()
            .zip(sources)
            .filter(|(a, _)| a.kind == AssetKind::Audio)
    };
    if let Some((asset, _)) = sounds().find(|(a, _)| transcoded_by_name(&a.name)) {
        return Ok(Some(asset.name.clone()));
    }
    for (asset, &at) in sounds() {
        check_cancel(cancel)?;
        let head = source.prefix(at, PROBE_BYTES);
        if id3v2_overflows(&head) {
            // Étiquette (pochette) plus longue que le préfixe : les octets lus après seraient
            // ceux de l'image, où une fausse synchronisation pourrait passer pour une trame.
            continue;
        }
        let Ok(info) = probe_mp3(&head) else {
            continue;
        };
        let non_conforming = info.version != MpegVersion::V1
            || info.layer != 3
            || info.sample_rate != ACCEPTED_SAMPLE_RATE_HZ
            || info.channels() != TARGET_CHANNELS;
        if !info.sketchy && non_conforming {
            return Ok(Some(asset.name.clone()));
        }
    }
    Ok(None)
}

/// Le préfixe commence-t-il par une étiquette ID3v2 qui le déborde (taille « synchsafe » de
/// l'en-tête, pied de page éventuel compris) ?
fn id3v2_overflows(head: &[u8]) -> bool {
    let Some(header) = head.get(..10).filter(|h| h.starts_with(b"ID3")) else {
        return false;
    };
    let size = header[6..10]
        .iter()
        .fold(0usize, |acc, &b| (acc << 7) | usize::from(b & 0x7f));
    let footer = if header[5] & 0x10 != 0 { 10 } else { 0 };
    10 + size + footer >= head.len()
}

fn plan(source: &mut Source, fit: ThumbFit, cancel: &watch::Receiver<bool>) -> AppResult<Plan> {
    let story_index = source
        .find(STORY_JSON)
        .ok_or_else(|| invalid_archive("story.json introuvable"))?;
    let bytes = source.read(story_index, MAX_STORY_JSON_BYTES as u64, STORY_JSON)?;
    let story = parse_story(&bytes)
        .map_err(|e| conversion(INVALID_STORY, format!("story.json refusé : {e}")))?;
    drop(bytes);
    let index = build_index(&story);
    // #535 : chaque asset référencé doit exister, avant toute écriture.
    let mut sources = Vec::with_capacity(index.assets.len());
    for asset in &index.assets {
        let found = source.find(&asset.archive_path).ok_or_else(|| {
            conversion(
                MISSING_ASSET,
                format!(
                    "asset {:?} introuvable dans l'archive ({})",
                    asset.name, asset.archive_path
                ),
            )
        })?;
        sources.push(found);
    }
    check_cancel(cancel)?;
    let first_to_transcode = first_to_transcode(source, &index, &sources, cancel)?;
    check_cancel(cancel)?;
    let mut warnings = Vec::new();
    let thumbnail = thumbnail(source, &story, &index, &sources, fit, cancel, &mut warnings)?;
    Ok(Plan {
        first_to_transcode,
        title: display_title(&story),
        description: clean_line(story.description(), MAX_DESCRIPTION_CHARS),
        story,
        index,
        sources,
        thumbnail,
        warnings,
    })
}

/// Vignette : `thumbnail.png`, sinon l'image du nœud d'entrée, sinon aucune. Une source
/// illisible, trop lourde ou trop grande est un avertissement (sauf erreur d'E/S, annulation).
fn thumbnail(
    source: &mut Source,
    story: &Story,
    index: &PackIndex,
    sources: &[usize],
    fit: ThumbFit,
    cancel: &watch::Receiver<bool>,
    warnings: &mut Vec<String>,
) -> AppResult<Option<Thumbnail>> {
    if let Some(at) = source.find(THUMBNAIL_PNG) {
        match source.read(at, MAX_IMAGE_BYTES as u64, THUMBNAIL_PNG) {
            Ok(bytes) => match thumb_lif(&bytes, fit, cancel) {
                // `_thumbnail.png` : l'original, seulement si c'est vraiment un PNG.
                Ok(lif) => {
                    let png = bytes.starts_with(PNG_SIGNATURE).then_some(bytes);
                    return Ok(Some(Thumbnail { lif, png }));
                }
                Err(e) => warnings.push(format!("thumbnail.png illisible, ignorée : {e}")),
            },
            Err(e @ (AppError::Io(_) | AppError::Cancelled)) => return Err(e),
            Err(e) => warnings.push(format!("thumbnail.png ignorée : {e}")),
        }
        check_cancel(cancel)?;
    }
    let entry_image = story.stage_nodes().first().and_then(|n| n.image.as_deref());
    let Some(name) = entry_image else {
        return Ok(None);
    };
    let Some(position) = index
        .assets
        .iter()
        .position(|a| a.kind == AssetKind::Image && a.name == name)
    else {
        return Ok(None);
    };
    // Une image de scène illisible ou trop lourde sera refusée par la conversion des images.
    let bytes = source.read(sources[position], MAX_IMAGE_BYTES as u64, name)?;
    match thumb_lif(&bytes, fit, cancel) {
        Ok(lif) => Ok(Some(Thumbnail { lif, png: None })),
        Err(e) => {
            check_cancel(cancel)?;
            warnings.push(format!(
                "vignette tirée de l'image de départ {name:?} impossible : {e}"
            ));
            Ok(None)
        }
    }
}

/// Vignette LIF 104 × 136 d'une image PNG, JPEG ou BMP (dimensions bornées avant décodage).
fn thumb_lif(
    bytes: &[u8],
    fit: ThumbFit,
    cancel: &watch::Receiver<bool>,
) -> Result<Vec<u8>, String> {
    let png = as_png(bytes)?;
    let cancelled = || *cancel.borrow();
    telmi2flam::image::png_to_thumb_lif(&png, fit.into(), &cancelled).map_err(|e| e.to_string())
}

/// Bornes de décodage des vignettes : celles de `studio2lunii::bitmap`.
fn image_limits() -> image::Limits {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(studio2lunii::bitmap::MAX_SIDE);
    limits.max_image_height = Some(studio2lunii::bitmap::MAX_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    limits
}

/// PNG, JPEG ou BMP dont les dimensions (lues dans l'en-tête, avant tout décodage) respectent
/// les bornes de `studio2lunii::bitmap` : un PNG est rendu tel quel, un JPEG ou un BMP est
/// décodé puis réencodé en PNG.
fn as_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let reader = || {
        let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
            .with_guessed_format()
            .map_err(|e| e.to_string())?;
        reader.limits(image_limits());
        Ok::<_, String>(reader)
    };
    let format = reader()?.format();
    if !matches!(
        format,
        Some(image::ImageFormat::Png | image::ImageFormat::Jpeg | image::ImageFormat::Bmp)
    ) {
        return Err("format d'image non pris en charge (PNG, JPEG ou BMP)".into());
    }
    let (w, h) = reader()?.into_dimensions().map_err(|e| e.to_string())?;
    if w == 0
        || h == 0
        || w > studio2lunii::bitmap::MAX_SIDE
        || h > studio2lunii::bitmap::MAX_SIDE
        || u64::from(w) * u64::from(h) > studio2lunii::bitmap::MAX_PIXELS
    {
        return Err(format!("dimensions d'image refusées : {w} x {h}"));
    }
    if format == Some(image::ImageFormat::Png) {
        return Ok(bytes.to_vec());
    }
    let decoded = reader()?.decode().map_err(|e| e.to_string())?;
    let mut png = Vec::new();
    decoded
        .to_rgba8()
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
}

// ---------------------------------------------------------------------------------------
// Écriture du pack
// ---------------------------------------------------------------------------------------

/// Pack en cours d'écriture (zip « stored », dates fixes).
struct Pack {
    zip: ZipWriter<BufWriter<File>>,
    written: u64,
}

impl Pack {
    fn create(path: &Path) -> AppResult<Self> {
        let file = File::create_new(path)?;
        Ok(Self {
            zip: ZipWriter::new(BufWriter::new(file)),
            written: 0,
        })
    }

    fn add(&mut self, name: &str, data: &[u8]) -> AppResult<()> {
        self.written = self.written.saturating_add(data.len() as u64);
        if self.written > MAX_TOTAL_BYTES {
            return Err(too_large(format!(
                "pack de plus de {MAX_TOTAL_BYTES} octets"
            )));
        }
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o644);
        self.zip.start_file(name, options).map_err(zip_error)?;
        self.zip.write_all(data)?;
        Ok(())
    }

    /// Ferme le zip et synchronise le fichier ; rend la taille du pack.
    fn finish(self) -> AppResult<u64> {
        let writer = self.zip.finish().map_err(zip_error)?;
        let file = writer.into_inner().map_err(|e| e.into_error())?;
        file.sync_all()?;
        Ok(file.metadata()?.len())
    }
}

/// Métadonnées de la bibliothèque (`_metadata.json`).
fn metadata(plan: &Plan) -> AppResult<Vec<u8>> {
    let value = serde_json::json!({
        "source_format": "studio",
        "uuid": plan.story.uuid(),
        "version": plan.story.pack_version(),
        "title": plan.title,
        "description": plan.description,
    });
    Ok(serde_json::to_vec_pretty(&value)?)
}

/// Entrées qui précèdent les images et les sons.
fn write_header(pack: &mut Pack, plan: &Plan) -> AppResult<()> {
    let uuid = uuid::Uuid::parse_str(plan.story.uuid())
        .map_err(|_| conversion(INVALID_STORY, "UUID du nœud d'entrée invalide"))?;
    pack.add("uuid.bin", uuid.as_bytes())?;
    pack.add("_metadata.json", &metadata(plan)?)?;
    if let Some(png) = plan.thumbnail.as_ref().and_then(|t| t.png.as_deref()) {
        pack.add("_thumbnail.png", png)?;
    }
    pack.add("ni", &plan.index.ni)?;
    if plan.index.night_mode {
        pack.add("nm", &[])?;
    }
    pack.add("li", &plan.index.li)?;
    pack.add("ri", &plan.index.ri)?;
    pack.add("si", &plan.index.si)?;
    pack.add("info", &info_bytes(&plan.title))?;
    if let Some(thumbnail) = &plan.thumbnail {
        pack.add("img/thumbnail.lif", &thumbnail.lif)?;
    }
    Ok(())
}

/// MP3 gardé : plage des octets audio, étiquettes écartées (le pack reçoit la tranche, sans
/// copie du son).
fn audio_range(bytes: &[u8], name: &str) -> AppResult<Range<usize>> {
    mp3_audio_range(bytes).map_err(|e| conversion(UNREADABLE_AUDIO, format!("son {name:?} : {e}")))
}

/// Son lu : gardé (déjà ajouté au pack), ou écrit dans le dossier temporaire pour ffmpeg.
enum Sound {
    Kept,
    Transcode,
}

/// État des étapes bloquantes, passé d'un fil à l'autre.
struct Work {
    source: Source,
    pack: Pack,
}

fn step(on_step: &mut (dyn FnMut(&Step) + Send), phase: &str, index: usize, done: u64, total: u64) {
    on_step(&Step {
        phase: phase.to_string(),
        index,
        phases: PHASES,
        done,
        total,
    });
}

/// Réglages d'une conversion.
struct RunOptions<'a> {
    fit: ThumbFit,
    transcoder: Option<&'a dyn Transcoder>,
    /// Budget de temps cumulé des transcodages.
    budget: Duration,
}

/// Transcode un son dans le budget restant (`spent` : temps déjà consommé). Au-delà, le futur
/// du transcodeur est abandonné (ffmpeg tué : `kill_on_drop`) et l'erreur est `TimedOut`.
async fn transcode_within(
    transcoder: &dyn Transcoder,
    (input, output): (&Path, &Path),
    budget: Duration,
    spent: &mut Duration,
    cancel: &watch::Receiver<bool>,
) -> AppResult<()> {
    let over = || {
        AppError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "ffmpeg : budget de {} s pour les sons de l'histoire dépassé",
                budget.as_secs()
            ),
        ))
    };
    let remaining = budget.checked_sub(*spent).filter(|r| !r.is_zero());
    let remaining = remaining.ok_or_else(over)?;
    let started = Instant::now();
    let result = tokio::time::timeout(remaining, transcoder.transcode(input, output, cancel)).await;
    *spent = spent.saturating_add(started.elapsed());
    result.map_err(|_| over())?
}

async fn run(
    input: &Path,
    output: &Path,
    tmp: &Path,
    options: &RunOptions<'_>,
    cancel: &watch::Receiver<bool>,
    on_step: &mut (dyn FnMut(&Step) + Send),
) -> AppResult<StudioPack> {
    let (input, fit, rx) = (input.to_path_buf(), options.fit, cancel.clone());
    let (source, plan) = blocking(move || -> AppResult<(Source, Plan)> {
        let mut source = Source::open(&input)?;
        let plan = plan(&mut source, fit, &rx)?;
        Ok((source, plan))
    })
    .await??;
    // ffmpeg exigé : contrôlé avant toute conversion (#553).
    if let Some(name) = &plan.first_to_transcode {
        match options.transcoder {
            None => {
                return Err(ffmpeg_unavailable(format!(
                    "le son {name:?} n'est pas un MP3 44,1 kHz mono"
                )))
            }
            Some(t) => t.prepare(cancel).await?,
        }
    }
    check_cancel(cancel)?;
    let part = tmp.join(PACK_PART);
    let (mut work, plan) = blocking(move || -> AppResult<(Work, Plan)> {
        let mut pack = Pack::create(&part)?;
        write_header(&mut pack, &plan)?;
        Ok((Work { source, pack }, plan))
    })
    .await??;

    let targets: Vec<(usize, &studio2lunii::index::AssetTarget)> = plan
        .sources
        .iter()
        .copied()
        .zip(plan.index.assets.iter())
        .collect();
    let images = targets
        .iter()
        .filter(|(_, a)| a.kind == AssetKind::Image)
        .count();
    let sounds = targets.len() - images;

    step(on_step, "images", 0, 0, images as u64);
    for (done, (at, asset)) in targets
        .iter()
        .filter(|(_, a)| a.kind == AssetKind::Image)
        .enumerate()
    {
        check_cancel(cancel)?;
        let (at, name, target) = (*at, asset.name.clone(), asset.pack_path.clone());
        work = blocking(move || -> AppResult<Work> {
            let bytes = work.source.read(at, MAX_IMAGE_BYTES as u64, &name)?;
            let bmp = to_lunii_bmp(&bytes)
                .map_err(|e| conversion(UNREADABLE_IMAGE, format!("image {name:?} : {e}")))?;
            work.pack.add(&target, &bmp)?;
            Ok(work)
        })
        .await??;
        step(on_step, "images", 0, done as u64 + 1, images as u64);
    }

    step(on_step, "audio", 1, 0, sounds as u64);
    let mut transcoded = 0;
    let mut spent = Duration::ZERO;
    let (sound_in, sound_out) = (tmp.join(SOUND_IN), tmp.join(SOUND_OUT));
    for (done, (at, asset)) in targets
        .iter()
        .filter(|(_, a)| a.kind == AssetKind::Audio)
        .enumerate()
    {
        check_cancel(cancel)?;
        let (at, name, target) = (*at, asset.name.clone(), asset.pack_path.clone());
        let staged = sound_in.clone();
        let (back, sound) = blocking(move || -> AppResult<(Work, Sound)> {
            if transcoded_by_name(&name) {
                // Ni lu en mémoire ni analysé : copié en flux pour ffmpeg.
                work.source.copy_to(at, MAX_SOUND_BYTES, &name, &staged)?;
                return Ok((work, Sound::Transcode));
            }
            let bytes = work.source.read(at, MAX_SOUND_BYTES, &name)?;
            if transcoding_required(&name, &bytes) {
                std::fs::write(&staged, &bytes)?;
                return Ok((work, Sound::Transcode));
            }
            let range = audio_range(&bytes, &name)?;
            work.pack.add(&target, &bytes[range])?;
            Ok((work, Sound::Kept))
        })
        .await??;
        work = back;
        if let Sound::Transcode = sound {
            let transcoder = options.transcoder.ok_or_else(|| {
                ffmpeg_unavailable(format!(
                    "le son {:?} n'est pas un MP3 44,1 kHz mono",
                    asset.name
                ))
            })?;
            transcode_within(
                transcoder,
                (&sound_in, &sound_out),
                options.budget,
                &mut spent,
                cancel,
            )
            .await?;
            check_cancel(cancel)?;
            transcoded += 1;
            let (name, target) = (asset.name.clone(), asset.pack_path.clone());
            let (staged, produced) = (sound_in.clone(), sound_out.clone());
            work = blocking(move || -> AppResult<Work> {
                let mp3 = read_produced(&produced)?;
                if transcoding_required("transcode.mp3", &mp3) {
                    return Err(conversion(
                        FFMPEG_FAILED,
                        format!("son {name:?} : le MP3 produit par ffmpeg n'est pas conforme"),
                    ));
                }
                let range = audio_range(&mp3, &name)?;
                work.pack.add(&target, &mp3[range])?;
                std::fs::remove_file(&staged)?;
                std::fs::remove_file(&produced)?;
                Ok(work)
            })
            .await??;
        }
        step(on_step, "audio", 1, done as u64 + 1, sounds as u64);
    }

    check_cancel(cancel)?;
    step(on_step, "zip", 2, 0, 1);
    let (part, output_path) = (tmp.join(PACK_PART), output.to_path_buf());
    let bytes = blocking(move || -> AppResult<u64> {
        let Work { source, pack } = work;
        drop(source);
        let bytes = pack.finish()?;
        crate::storage::retry_transient(
            || std::fs::rename(&part, &output_path),
            std::thread::sleep,
        )?;
        Ok(bytes)
    })
    .await??;
    step(on_step, "zip", 2, 1, 1);
    tracing::info!(
        uuid = %plan.story.uuid(),
        images,
        sons = sounds,
        transcodes = transcoded,
        octets = bytes,
        "histoire STUdio convertie en pack Lunii"
    );
    Ok(StudioPack {
        uuid: plan.story.uuid().to_string(),
        version: plan.story.pack_version(),
        title: plan.title,
        description: plan.description,
        night_mode: plan.index.night_mode,
        images,
        sounds,
        transcoded,
        thumbnail: plan.thumbnail.is_some(),
        warnings: plan.warnings,
        bytes,
    })
}

/// Relit le MP3 produit par ffmpeg (borné).
fn read_produced(path: &Path) -> AppResult<Vec<u8>> {
    let mut data = Vec::new();
    File::open(path)
        .map_err(|e| conversion(FFMPEG_FAILED, format!("sortie de ffmpeg absente : {e}")))?
        .take(MAX_SOUND_BYTES + 1)
        .read_to_end(&mut data)?;
    if data.len() as u64 > MAX_SOUND_BYTES {
        return Err(too_large(format!(
            "son transcodé de plus de {MAX_SOUND_BYTES} octets"
        )));
    }
    Ok(data)
}

#[cfg(test)]
mod tests;
