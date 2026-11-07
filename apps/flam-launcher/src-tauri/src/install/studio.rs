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
//! | `_thumbnail.png`    | `thumbnail.png` d'origine, s'il a pu servir de vignette                   |
//! | `ni`, `nm`          | index des nœuds ; `nm` vide si `nightModeAvailable`                       |
//! | `li`, `ri`, `si`    | index en clair (`studio2lunii::index`)                                    |
//! | `info`              | ASCII, LF, 5 lignes : titre, titre, `0`, titre, `No authors.` (§ 3.8, Q4) |
//! | `img/thumbnail.lif` | vignette 104 × 136 (`telmi2flam`), si une source existe                   |
//! | `rf/000/<NOM>`      | images en BMP 4 bits RLE 320 × 240 (`studio2lunii::bitmap`)               |
//! | `sf/000/<NOM>`      | sons MP3 MPEG-1 couche III, 44,1 kHz, mono, sans étiquette                |
//!
//! Sons : un MP3 conforme (`studio2lunii::audio::transcoding_required` faux) est gardé, ses
//! étiquettes retirées ; tout autre son est transcodé par le ffmpeg embarqué
//! ([`crate::ffmpeg`], arguments `FFMPEG_TRANSCODE_ARGS`, empreinte vérifiée une fois par
//! conversion), puis la sortie est recontrôlée (refus si elle n'est pas conforme) et nettoyée.
//! ffmpeg n'est exigé que si un son doit être transcodé.
//!
//! Vignette : `thumbnail.png`, sinon l'image du nœud d'entrée (PNG, JPEG ou BMP), sinon aucune
//! (`prepare_lunii` l'accepte). Une source de vignette illisible est un **avertissement**
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
//!    ffmpeg (120 s par son). Aucun parcours du graphe.
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
//! | Délai de ffmpeg dépassé                                       | `io` (`TimedOut`)                                 |
//! | Annulation                                                    | `cancelled`                                       |
//! | Écriture impossible (disque plein, sortie inaccessible)       | `io`                                              |
//!
//! Dans tous ces cas, rien ne reste du côté de la sortie, sauf si le futur est abandonné
//! pendant une étape bloquante (fichier encore ouvert sous Windows) : le dossier temporaire
//! `.…tmp/` peut alors subsister dans le dossier de sortie (`library/.staging/` en G2).

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use studio2lunii::audio::{
    mp3_tag_cleanup, tags_removal_required, transcoding_required, FFMPEG_TRANSCODE_ARGS,
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
use crate::device::prepare::{for_each_entry, open_archive, Archive, Limits, PrepareError};
use crate::ffmpeg::{self, Ffmpeg, Tool, Verified};
use crate::podcast::audio::file_arg;
use crate::settings::ThumbFit;
use crate::{AppError, AppResult};

/// Entrées maximales de l'archive (note § 6.2).
pub const MAX_ENTRIES: usize = 100_000;
/// Total décompressé (déclaré, puis lu) et total écrit dans le pack (note § 6.2).
pub const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Taille maximale d'un son source, et d'un son transcodé relu (note § 6.2).
pub const MAX_SOUND_BYTES: u64 = 512 * 1024 * 1024;
/// Délai de ffmpeg par son (note § 6.2, comme `telmi2flam`).
pub const FFMPEG_TIMEOUT: Duration = Duration::from_secs(120);
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
/// Longueur maximale du titre (caractères), comme le scan des dossiers.
const MAX_TITLE_CHARS: usize = 200;
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

impl Transcoder for EmbeddedFfmpeg {
    fn transcode<'a>(
        &'a self,
        input: &'a Path,
        output: &'a Path,
        cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            let verified = self
                .verified
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
                .await?;
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
            args.extend(FFMPEG_TRANSCODE_ARGS.iter().map(OsString::from));
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
        cancel,
        on_step,
    )
    .await
}

/// [`convert`] avec un transcodeur injecté (`None` : ffmpeg absent).
pub(crate) async fn convert_with(
    input: &Path,
    output: &Path,
    fit: ThumbFit,
    transcoder: Option<&dyn Transcoder>,
    cancel: &watch::Receiver<bool>,
    on_step: &mut (dyn FnMut(&Step) + Send),
) -> AppResult<StudioPack> {
    check_cancel(cancel)?;
    let tmp = {
        let output = output.to_path_buf();
        blocking(move || TempDir::create(&output)).await??
    };
    let result = run(input, output, &tmp.0, fit, transcoder, cancel, on_step).await;
    // Suppression hors du runtime ; `Drop` la refait (sans effet) si elle a échoué.
    let _ = blocking(move || drop(tmp)).await;
    result
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
        let limits = Limits {
            max_entries: MAX_ENTRIES,
            ..Limits::default()
        };
        let mut zip = open_archive(input, &limits).map_err(archive_error)?;
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
        for index in 0..zip.len() {
            let entry = zip
                .by_index_raw(index)
                .map_err(|e| invalid_archive(format!("entrée {index} : {e}")))?;
            if entry.encrypted() {
                return Err(invalid_archive(
                    "archive protégée par mot de passe : décompressez-la puis recompressez-la \
                     sans mot de passe",
                ));
            }
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
            .map_err(|e| invalid_archive(format!("{what} illisible : {e}")))?;
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
            .map_err(|e| invalid_archive(format!("{what} illisible : {e}")))?;
        if data.len() as u64 > max {
            return Err(too_large(format!("{what} dépasse {max} octets")));
        }
        self.read = self.read.saturating_add(data.len() as u64);
        if self.read > MAX_TOTAL_BYTES {
            return Err(too_large(format!(
                "plus de {MAX_TOTAL_BYTES} octets décompressés"
            )));
        }
        Ok(data)
    }
}

// ---------------------------------------------------------------------------------------
// Textes : titre, description, `info`
// ---------------------------------------------------------------------------------------

/// Texte sur une ligne : contrôles et blancs consécutifs réduits à une espace, bords rognés,
/// longueur bornée (#535).
pub(crate) fn clean_line(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
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
        if canonical_combining_class(c) != 0 {
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
    let mut warnings = Vec::new();
    let thumbnail = thumbnail(source, &story, &index, &sources, fit, cancel, &mut warnings)?;
    Ok(Plan {
        title: display_title(&story),
        description: clean_line(story.description(), MAX_DESCRIPTION_CHARS),
        story,
        index,
        sources,
        thumbnail,
        warnings,
    })
}

/// Vignette : `thumbnail.png`, sinon l'image du nœud d'entrée, sinon aucune.
fn thumbnail(
    source: &mut Source,
    story: &Story,
    index: &PackIndex,
    sources: &[usize],
    fit: ThumbFit,
    cancel: &watch::Receiver<bool>,
    warnings: &mut Vec<String>,
) -> AppResult<Option<Thumbnail>> {
    let cancelled = || *cancel.borrow();
    if let Some(at) = source.find(THUMBNAIL_PNG) {
        let png = source.read(at, MAX_IMAGE_BYTES as u64, THUMBNAIL_PNG)?;
        match telmi2flam::image::png_to_thumb_lif(&png, fit.into(), &cancelled) {
            Ok(lif) => {
                return Ok(Some(Thumbnail {
                    lif,
                    png: Some(png),
                }))
            }
            Err(e) => warnings.push(format!("thumbnail.png illisible, ignorée : {e}")),
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
    let bytes = source.read(sources[position], MAX_IMAGE_BYTES as u64, name)?;
    let lif = as_png(&bytes).and_then(|png| {
        telmi2flam::image::png_to_thumb_lif(&png, fit.into(), &cancelled).map_err(|e| e.to_string())
    });
    match lif {
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

/// PNG tel quel, ou JPEG/BMP décodé (bornes de `studio2lunii::bitmap`) puis réencodé en PNG.
fn as_png(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.starts_with(PNG_SIGNATURE) {
        return Ok(bytes.to_vec());
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    if !matches!(
        reader.format(),
        Some(image::ImageFormat::Jpeg | image::ImageFormat::Bmp)
    ) {
        return Err("format d'image non pris en charge".into());
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(studio2lunii::bitmap::MAX_SIDE);
    limits.max_image_height = Some(studio2lunii::bitmap::MAX_SIDE);
    limits.max_alloc = Some(256 * 1024 * 1024);
    reader.limits(limits.clone());
    let (w, h) = reader.into_dimensions().map_err(|e| e.to_string())?;
    if u64::from(w) * u64::from(h) > studio2lunii::bitmap::MAX_PIXELS {
        return Err(format!("image trop grande : {w} x {h}"));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?;
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

/// MP3 gardé : étiquettes retirées.
fn clean_mp3(bytes: Vec<u8>, name: &str) -> AppResult<Vec<u8>> {
    if !tags_removal_required(&bytes) {
        return Ok(bytes);
    }
    mp3_tag_cleanup(&bytes).map_err(|e| conversion(UNREADABLE_AUDIO, format!("son {name:?} : {e}")))
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

#[allow(clippy::too_many_arguments)]
async fn run(
    input: &Path,
    output: &Path,
    tmp: &Path,
    fit: ThumbFit,
    transcoder: Option<&dyn Transcoder>,
    cancel: &watch::Receiver<bool>,
    on_step: &mut (dyn FnMut(&Step) + Send),
) -> AppResult<StudioPack> {
    let (input, part) = (input.to_path_buf(), tmp.join(PACK_PART));
    let rx = cancel.clone();
    let (mut work, plan) = blocking(move || -> AppResult<(Work, Plan)> {
        let mut source = Source::open(&input)?;
        let plan = plan(&mut source, fit, &rx)?;
        check_cancel(&rx)?;
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
            let bytes = work.source.read(at, MAX_SOUND_BYTES, &name)?;
            if transcoding_required(&name, &bytes) {
                std::fs::write(&staged, &bytes)?;
                return Ok((work, Sound::Transcode));
            }
            let mp3 = clean_mp3(bytes, &name)?;
            work.pack.add(&target, &mp3)?;
            Ok((work, Sound::Kept))
        })
        .await??;
        work = back;
        if let Sound::Transcode = sound {
            let transcoder = transcoder.ok_or_else(|| {
                ffmpeg_unavailable(format!(
                    "le son {:?} n'est pas un MP3 44,1 kHz mono",
                    asset.name
                ))
            })?;
            transcoder.transcode(&sound_in, &sound_out, cancel).await?;
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
                let mp3 = clean_mp3(mp3, &name)?;
                work.pack.add(&target, &mp3)?;
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
