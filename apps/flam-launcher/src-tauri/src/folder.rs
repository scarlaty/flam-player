//! Source « dossier local » : analyse d'un dossier d'histoires (`.zip` Telmi, dossiers
//! `.plain`, archives `.plain.pk`) et production de `StoryEntry`.
//!
//! Les archives sont non fiables : rien n'est extrait sur disque (seules les vignettes
//! réduites vont dans `<cache>/thumbs/`), les lectures sont bornées, les noms d'entrées
//! dangereux ignorés et les liens symboliques (et jonctions) jamais suivis. Un fichier invalide
//! ne fait pas échouer l'analyse : il est ignoré et signalé dans `warnings`.
//!
//! **Vignettes à la demande** : `folder_scan` ne renvoie jamais d'image (jusqu'à 5000 entrées :
//! des centaines de Mio d'IPC). `thumb_url` est une référence (l'`entry_id`) à résoudre par
//! `folder_thumbnail`, ou `null` ;
//! `folder_thumbnail(source_id, entry_id)` renvoie l'URL `data:` d'une seule histoire, et
//! uniquement pour une entrée du dernier scan de cette source (jamais un chemin libre).

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::State;
use zip::result::ZipError;
use zip::ZipArchive;

use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::ConfigState;
use crate::story::{validate_story_id, StoryEntry, StoryKind};
use crate::thumbnail::{self, RgbaImage, ThumbCache, ThumbError};
use crate::{AppError, AppResult};
use telmi2flam::lif;

/// `metadata.json` / `info.plain` : 1 Mio au plus.
const MAX_TEXT_BYTES: u64 = 1024 * 1024;
/// Vignette (PNG Telmi ou LIF) : 8 Mio au plus.
const MAX_IMAGE_BYTES: u64 = 8 * 1024 * 1024;
const UUID_BIN_LEN: u64 = 16;
/// Entrées maximum d'une archive.
const MAX_ZIP_ENTRIES: usize = 100_000;
/// Profondeur maximale du parcours récursif.
const MAX_DEPTH: usize = 8;
/// Histoires maximum par analyse.
const MAX_STORIES: usize = 5_000;
const MAX_TITLE_CHARS: usize = 200;
const MAX_FIELD_CHARS: usize = 2_000;
/// Côté maximal des vignettes servies.
const THUMB_DIM: u32 = 160;

/// Fichier ou archive ignoré lors de l'analyse (miroir de `ScanWarning` dans `src/api/folder.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScanWarning {
    /// Chemin du fichier ou dossier concerné.
    pub path: String,
    /// Explication lisible.
    pub message: String,
}

/// Résultat de `folder_scan` (miroir de `FolderScan` dans `src/api/folder.ts`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FolderScan {
    pub entries: Vec<StoryEntry>,
    pub warnings: Vec<ScanWarning>,
}

// ---------------------------------------------------------------------------
// Identifiants
// ---------------------------------------------------------------------------

impl From<ThumbError> for AppError {
    fn from(e: ThumbError) -> Self {
        AppError::Validation(format!("vignette illisible : {e}"))
    }
}

/// Identifiant d'entrée `"<source_id>:<hash>"` : hash FNV-1a 64 bits du chemin relatif au
/// dossier de la source (séparateurs normalisés en `/`), stable d'un scan à l'autre et unique
/// dans la source. Un `.plain` et un `.plain.pk` de même `uuid` ont donc des identifiants
/// différents.
fn entry_id_of(source_id: &str, root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/");
    format!(
        "{source_id}:{:016x}",
        thumbnail::fnv1a(&[relative.as_bytes()])
    )
}

// ---------------------------------------------------------------------------
// Lecture bornée de fichiers (dossier ou archive)
// ---------------------------------------------------------------------------

/// Accès en lecture aux fichiers d'une histoire FLAM (dossier `.plain` ou `.plain.pk`).
trait FlamFiles {
    /// Lit un fichier (au plus `limit` octets) ; `Ok(None)` s'il n'existe pas.
    fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String>;
    /// Taille et date de modification (ns) d'un fichier, `None` s'il n'existe pas.
    fn stat(&mut self, name: &str) -> Option<(u64, u128)>;
}

fn read_limited(reader: impl Read, limit: u64, what: &str) -> Result<Vec<u8>, String> {
    let mut buf = Vec::new();
    reader
        .take(limit + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("lecture de {what} impossible : {e}"))?;
    if buf.len() as u64 > limit {
        return Err(format!("{what} trop volumineux"));
    }
    Ok(buf)
}

fn nanos(time: Option<SystemTime>) -> u128 {
    time.and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos())
}

struct DirFiles<'a>(&'a Path);

impl FlamFiles for DirFiles<'_> {
    fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        let path = self.0.join(name);
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(format!("{name} illisible : {e}")),
        };
        if !meta.is_file() {
            return Ok(None);
        }
        if meta.len() > limit {
            return Err(format!("{name} trop volumineux"));
        }
        let file = File::open(&path).map_err(|e| format!("{name} illisible : {e}"))?;
        read_limited(file, limit, name).map(Some)
    }

    fn stat(&mut self, name: &str) -> Option<(u64, u128)> {
        let meta = std::fs::symlink_metadata(self.0.join(name)).ok()?;
        meta.is_file()
            .then(|| (meta.len(), nanos(meta.modified().ok())))
    }
}

/// Archive zip avec table des noms normalisés (`\` -> `/`, comme `telmi2flam.py`) vers
/// les noms d'origine.
struct Zip {
    archive: ZipArchive<File>,
    names: HashMap<String, String>,
}

impl Zip {
    fn open(path: &Path) -> Result<Self, String> {
        let file = File::open(path).map_err(|e| format!("ouverture impossible : {e}"))?;
        let archive = ZipArchive::new(file).map_err(|e| format!("archive zip invalide : {e}"))?;
        if archive.len() > MAX_ZIP_ENTRIES {
            return Err("archive zip avec trop d'entrées".into());
        }
        let names = archive
            .file_names()
            .map(|n| (n.replace('\\', "/"), n.to_string()))
            .collect();
        Ok(Self { archive, names })
    }

    fn contains(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    fn normalized_names(&self) -> Vec<String> {
        self.names.keys().cloned().collect()
    }
}

impl FlamFiles for Zip {
    fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        let Some(original) = self.names.get(name) else {
            return Ok(None);
        };
        let entry = match self.archive.by_name(original) {
            Ok(entry) => entry,
            Err(ZipError::FileNotFound) => return Ok(None),
            Err(e) => return Err(format!("{name} illisible dans l'archive : {e}")),
        };
        if entry.is_dir() {
            return Ok(None);
        }
        if entry.size() > limit {
            return Err(format!("{name} trop volumineux"));
        }
        // Borné même si la taille déclarée est mensongère.
        read_limited(entry, limit, name).map(Some)
    }

    fn stat(&mut self, name: &str) -> Option<(u64, u128)> {
        let original = self.names.get(name)?;
        let entry = self.archive.by_name(original).ok()?;
        (!entry.is_dir()).then(|| (entry.size(), 0))
    }
}

// ---------------------------------------------------------------------------
// Utilitaires de texte et de date
// ---------------------------------------------------------------------------

/// Supprime le BOM UTF-8 éventuel puis décode en UTF-8 strict.
fn utf8_text(bytes: &[u8], what: &str) -> Result<String, String> {
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    String::from_utf8(bytes.to_vec()).map_err(|_| format!("{what} n'est pas de l'UTF-8 valide"))
}

/// Texte nettoyé : caractères de contrôle remplacés par des espaces, bords rognés, longueur bornée.
fn clean(text: &str, max_chars: usize) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    flat.trim().chars().take(max_chars).collect()
}

/// Champ texte JSON (chaîne ou nombre) non vide.
fn json_text(object: &Value, key: &str, max_chars: usize) -> Option<String> {
    let text = match object.get(key)? {
        Value::String(s) => clean(s, max_chars),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

/// Champ entier JSON (nombre ou chaîne numérique).
fn json_u32(object: &Value, key: &str) -> Option<u32> {
    match object.get(key)? {
        Value::Number(n) => n.as_u64().and_then(|v| u32::try_from(v).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Date UTC ISO 8601 (`2026-10-15T08:30:00Z`) à partir d'un instant.
pub(crate) fn iso8601(time: SystemTime) -> Option<String> {
    let secs = i64::try_from(time.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()?;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Jour civil depuis le nombre de jours écoulés depuis 1970-01-01 (algorithme de H. Hinnant).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    ))
}

// ---------------------------------------------------------------------------
// Histoire Telmi (.zip)
// ---------------------------------------------------------------------------

/// Nom d'entrée (déjà normalisé en `/`) sans élément dangereux : absolu, `..`, lecteur,
/// `__MACOSX`, `._*`.
fn is_safe_entry_name(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains(':')
        && name
            .split('/')
            .all(|part| part != ".." && part != "__MACOSX" && !part.starts_with("._"))
}

/// Dossier racine de l'histoire dans l'archive : celui qui contient `metadata.json` et
/// `nodes.json` (miroir de `find_root` de `telmi2flam.py`). Le moins profond gagne ;
/// plusieurs candidats au même niveau est une erreur. `names` est normalisé en `/`.
fn find_root(names: &[String]) -> Result<String, String> {
    let set: HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut candidates: Vec<&str> = names
        .iter()
        .filter(|n| is_safe_entry_name(n))
        .filter_map(|n| n.strip_suffix("metadata.json"))
        .filter(|root| root.is_empty() || root.ends_with('/'))
        .filter(|root| set.contains(format!("{root}nodes.json").as_str()))
        .collect();
    if candidates.is_empty() {
        return Err("metadata.json et nodes.json introuvables (pas une histoire Telmi ?)".into());
    }
    let depth = candidates
        .iter()
        .map(|r| r.matches('/').count())
        .min()
        .unwrap_or(0);
    candidates.retain(|r| r.matches('/').count() == depth);
    candidates.sort_unstable();
    candidates.dedup();
    if candidates.len() > 1 {
        return Err("plusieurs histoires dans l'archive".into());
    }
    Ok(candidates[0].to_string())
}

/// Nom de la vignette Telmi à lire : `title.png` ou `cover.png` selon `image`, l'autre en
/// secours. Jamais un nom venant de l'archive.
fn telmi_image_names(meta: &Value) -> [&'static str; 2] {
    if json_text(meta, "image", 20).as_deref() == Some("cover.png") {
        ["cover.png", "title.png"]
    } else {
        ["title.png", "cover.png"]
    }
}

fn read_telmi_metadata(zip: &mut Zip, root: &str) -> Result<Value, String> {
    let raw = zip
        .read(&format!("{root}metadata.json"), MAX_TEXT_BYTES)?
        .ok_or("metadata.json introuvable")?;
    let meta: Value = serde_json::from_str(&utf8_text(&raw, "metadata.json")?)
        .map_err(|e| format!("metadata.json invalide : {e}"))?;
    if !meta.is_object() {
        return Err("metadata.json invalide : objet attendu".into());
    }
    Ok(meta)
}

/// Histoire analysée, sans l'identifiant d'entrée (déduit du chemin).
#[derive(Debug, Clone)]
struct Parsed {
    story: StoryEntry,
    has_thumb: bool,
    warnings: Vec<String>,
}

fn telmi_entry(source_id: &str, path: &Path, info: &FileInfo) -> Result<Parsed, String> {
    let mut zip = Zip::open(path)?;
    let root = find_root(&zip.normalized_names())?;
    let meta = read_telmi_metadata(&mut zip, &root)?;
    let title =
        json_text(&meta, "title", MAX_TITLE_CHARS).ok_or("titre absent de metadata.json")?;
    let uuid = json_text(&meta, "uuid", 100).ok_or("uuid absent de metadata.json")?;
    if !validate_story_id(&uuid) {
        return Err("uuid de metadata.json refusé (caractères ou nom non autorisés)".into());
    }
    if let Some(Value::String(version)) = meta.get("version") {
        if !validate_story_id(version.trim()) {
            return Err("version de metadata.json refusée".into());
        }
    }
    let has_thumb = telmi_image_names(&meta)
        .iter()
        .any(|name| zip.contains(&format!("{root}{name}")));

    let story = StoryEntry {
        entry_id: String::new(),
        source_id: source_id.to_string(),
        uuid,
        version: json_u32(&meta, "version"),
        title,
        description: json_text(&meta, "description", MAX_FIELD_CHARS),
        age: json_u32(&meta, "age"),
        category: json_text(&meta, "category", MAX_FIELD_CHARS),
        author: json_text(&meta, "author", MAX_FIELD_CHARS),
        voice: json_text(&meta, "voice", MAX_FIELD_CHARS),
        license: json_text(&meta, "license", MAX_FIELD_CHARS),
        thumb_url: None,
        download_url: path.to_string_lossy().into_owned(),
        size: Some(info.size),
        updated_at: info.updated_at.clone(),
        download_count: None,
        kind: StoryKind::TelmiZip,
    };
    Ok(Parsed {
        story,
        has_thumb,
        warnings: Vec::new(),
    })
}

/// Image source de la vignette d'une archive Telmi (PNG décodé).
fn telmi_thumb_image(path: &Path) -> Result<RgbaImage, String> {
    let mut zip = Zip::open(path)?;
    let root = find_root(&zip.normalized_names())?;
    let meta = read_telmi_metadata(&mut zip, &root)?;
    for name in telmi_image_names(&meta) {
        if let Some(bytes) = zip.read(&format!("{root}{name}"), MAX_IMAGE_BYTES)? {
            return thumbnail::decode_png(&bytes).map_err(|e| format!("vignette {name} : {e}"));
        }
    }
    Err("aucune vignette dans l'archive".into())
}

// ---------------------------------------------------------------------------
// Histoire FLAM (.plain, .plain.pk)
// ---------------------------------------------------------------------------

const LIF_NAME: &str = "img/thumbnail.lif";

fn flam_entry(
    source_id: &str,
    path: &Path,
    kind: StoryKind,
    info: &FileInfo,
    files: &mut dyn FlamFiles,
) -> Result<Parsed, String> {
    let raw = files
        .read("info.plain", MAX_TEXT_BYTES)?
        .ok_or("info.plain introuvable (pas une histoire FLAM ?)")?;
    let text = utf8_text(&raw, "info.plain")?;
    let title = clean(text.lines().next().unwrap_or(""), MAX_TITLE_CHARS);
    if title.is_empty() {
        return Err("titre vide dans info.plain".into());
    }
    let uuid_bin = files
        .read("uuid.bin", UUID_BIN_LEN)?
        .ok_or("uuid.bin introuvable")?;
    if uuid_bin.len() as u64 != UUID_BIN_LEN {
        return Err("uuid.bin doit faire 16 octets".into());
    }
    let uuid: String = uuid_bin.iter().map(|b| format!("{b:02x}")).collect();
    let has_thumb = files.stat(LIF_NAME).is_some();

    let story = StoryEntry {
        entry_id: String::new(),
        source_id: source_id.to_string(),
        uuid,
        version: None,
        title,
        description: None,
        age: None,
        category: None,
        author: None,
        voice: None,
        license: None,
        thumb_url: None,
        download_url: path.to_string_lossy().into_owned(),
        size: (kind == StoryKind::FlamPk).then_some(info.size),
        updated_at: info.updated_at.clone(),
        download_count: None,
        kind,
    };
    Ok(Parsed {
        story,
        has_thumb,
        warnings: Vec::new(),
    })
}

fn lif_thumb_image(files: &mut dyn FlamFiles) -> Result<RgbaImage, String> {
    let bytes = files
        .read(LIF_NAME, MAX_IMAGE_BYTES)?
        .ok_or("aucune vignette img/thumbnail.lif")?;
    lif::decode(&bytes).map_err(|e| format!("vignette {LIF_NAME} : {e}"))
}

// ---------------------------------------------------------------------------
// Informations de fichier
// ---------------------------------------------------------------------------

struct FileInfo {
    size: u64,
    mtime: u128,
    updated_at: Option<String>,
}

fn file_info(meta: &std::fs::Metadata) -> FileInfo {
    let modified = meta.modified().ok();
    FileInfo {
        size: meta.len(),
        mtime: nanos(modified),
        updated_at: modified.and_then(iso8601),
    }
}

// ---------------------------------------------------------------------------
// Parcours du dossier
// ---------------------------------------------------------------------------

/// Classe une entrée du dossier (insensible à la casse). Seuls les dossiers `.plain` et les
/// fichiers ordinaires `.zip` / `.plain.pk` sont retenus (jamais un FIFO ou un périphérique,
/// dont l'ouverture pourrait bloquer).
fn classify(name: &str, file_type: &std::fs::FileType) -> Option<StoryKind> {
    let lower = name.to_lowercase();
    if file_type.is_dir() {
        lower.ends_with(".plain").then_some(StoryKind::FlamPlain)
    } else if !file_type.is_file() {
        None
    } else if lower.ends_with(".plain.pk") {
        Some(StoryKind::FlamPk)
    } else if lower.ends_with(".zip") {
        Some(StoryKind::TelmiZip)
    } else {
        None
    }
}

fn collect(
    dir: &Path,
    depth: usize,
    recursive: bool,
    found: &mut Vec<(PathBuf, StoryKind)>,
    warnings: &mut Vec<ScanWarning>,
) {
    let read = match std::fs::read_dir(dir) {
        Ok(read) => read,
        Err(e) => {
            warnings.push(ScanWarning {
                path: dir.to_string_lossy().into_owned(),
                message: format!("dossier illisible : {e}"),
            });
            return;
        }
    };
    let mut children: Vec<_> = read.filter_map(Result::ok).collect();
    children.sort_by_key(|c| c.file_name().to_string_lossy().to_lowercase());
    for child in children {
        // `file_type` ne suit pas les liens symboliques ni les jonctions : ignorés.
        let Ok(file_type) = child.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let path = child.path();
        let name = child.file_name().to_string_lossy().into_owned();
        if found.len() >= MAX_STORIES {
            warnings.push(ScanWarning {
                path: dir.to_string_lossy().into_owned(),
                message: format!("analyse limitée à {MAX_STORIES} histoires"),
            });
            return;
        }
        if let Some(kind) = classify(&name, &file_type) {
            found.push((path, kind));
        } else if file_type.is_dir() && recursive && depth < MAX_DEPTH {
            collect(&path, depth + 1, recursive, found, warnings);
        }
    }
}

/// Métadonnées déjà lues d'une archive (clé : chemin, taille, date de modification), pour ne
/// pas rouvrir chaque zip à chaque analyse. Les dossiers `.plain` ne sont pas mis en cache
/// (la date d'un dossier ne change pas quand un fichier interne change).
#[derive(Default, Clone)]
pub struct MetaCache(HashMap<PathBuf, (u64, u128, Result<Parsed, String>)>);

/// Où retrouver la vignette d'une entrée du dernier scan.
#[derive(Debug, Clone)]
struct ThumbRef {
    path: PathBuf,
    kind: StoryKind,
}

type ThumbRefs = HashMap<String, ThumbRef>;

fn parse_one(
    source_id: &str,
    path: &Path,
    kind: StoryKind,
    cache: &mut MetaCache,
) -> Result<Parsed, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("illisible : {e}"))?;
    let info = file_info(&meta);
    let cacheable = kind != StoryKind::FlamPlain;
    if cacheable {
        if let Some((size, mtime, parsed)) = cache.0.get(path) {
            if *size == info.size && *mtime == info.mtime {
                return parsed.clone();
            }
        }
    }
    let parsed = match kind {
        StoryKind::TelmiZip => telmi_entry(source_id, path, &info),
        StoryKind::FlamPlain => flam_entry(source_id, path, kind, &info, &mut DirFiles(path)),
        StoryKind::FlamPk => {
            Zip::open(path).and_then(|mut zip| flam_entry(source_id, path, kind, &info, &mut zip))
        }
    };
    if cacheable {
        cache
            .0
            .insert(path.to_path_buf(), (info.size, info.mtime, parsed.clone()));
    }
    parsed
}

/// Analyse `root` (non récursif par défaut) : histoires, avertissements et références de
/// vignettes (entry_id -> archive).
fn scan_folder(
    source_id: &str,
    root: &Path,
    recursive: bool,
    cache: &mut MetaCache,
) -> AppResult<(FolderScan, ThumbRefs)> {
    match std::fs::metadata(root) {
        Ok(meta) if meta.is_dir() => {}
        _ => {
            return Err(AppError::FolderUnavailable(format!(
                "dossier introuvable ou inaccessible : {}",
                root.display()
            )))
        }
    }
    let mut scan = FolderScan::default();
    let mut refs = ThumbRefs::new();
    let mut found = Vec::new();
    collect(root, 0, recursive, &mut found, &mut scan.warnings);

    // Les archives disparues ne restent pas en cache.
    let present: HashSet<&PathBuf> = found.iter().map(|(p, _)| p).collect();
    cache
        .0
        .retain(|p, _| !p.starts_with(root) || present.contains(p));

    for (path, kind) in &found {
        let shown = path.to_string_lossy().into_owned();
        match parse_one(source_id, path, *kind, cache) {
            Ok(mut parsed) => {
                // Le cache de métadonnées est partagé entre sources : identité fixée ici.
                let entry_id = entry_id_of(source_id, root, path);
                parsed.story.source_id = source_id.to_string();
                parsed.story.entry_id = entry_id.clone();
                if parsed.has_thumb {
                    parsed.story.thumb_url = Some(entry_id.clone());
                    refs.insert(
                        entry_id,
                        ThumbRef {
                            path: path.clone(),
                            kind: *kind,
                        },
                    );
                }
                scan.entries.push(parsed.story);
                scan.warnings
                    .extend(parsed.warnings.into_iter().map(|message| ScanWarning {
                        path: shown.clone(),
                        message,
                    }));
            }
            Err(message) => {
                tracing::warn!("{shown} ignoré : {message}");
                scan.warnings.push(ScanWarning {
                    path: shown,
                    message: format!("ignoré : {message}"),
                });
            }
        }
    }
    Ok((scan, refs))
}

/// Vignette d'une histoire : URL `data:`, depuis le cache si possible (clé : chemin, taille,
/// date de l'archive et, pour un `.plain`, taille et date de `img/thumbnail.lif`), sinon
/// décodée puis mise en cache. Le cache est consulté avant d'ouvrir l'archive.
fn make_thumbnail(thumb: &ThumbRef, cache_dir: &Path) -> AppResult<String> {
    let invalid = |m: String| AppError::Validation(format!("vignette illisible : {m}"));
    let meta = std::fs::metadata(&thumb.path).map_err(|e| invalid(e.to_string()))?;
    let info = file_info(&meta);
    let lif_stat = if thumb.kind == StoryKind::FlamPlain {
        DirFiles(&thumb.path)
            .stat(LIF_NAME)
            .ok_or_else(|| invalid("aucune vignette".into()))?
    } else {
        (0, 0)
    };
    let key = thumbnail::fnv1a(&[
        thumb.path.to_string_lossy().as_bytes(),
        &info.size.to_le_bytes(),
        &info.mtime.to_le_bytes(),
        &lif_stat.0.to_le_bytes(),
        &lif_stat.1.to_le_bytes(),
        &THUMB_DIM.to_le_bytes(),
    ]);
    ThumbCache::new(cache_dir).get_or_make::<AppError>(key, THUMB_DIM, || {
        match thumb.kind {
            StoryKind::TelmiZip => telmi_thumb_image(&thumb.path),
            StoryKind::FlamPk => {
                Zip::open(&thumb.path).and_then(|mut zip| lif_thumb_image(&mut zip))
            }
            StoryKind::FlamPlain => lif_thumb_image(&mut DirFiles(&thumb.path)),
        }
        .map_err(invalid)
    })
}

// ---------------------------------------------------------------------------
// État partagé et commandes Tauri
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Inner {
    meta: MetaCache,
    /// Références de vignettes du dernier scan de chaque source.
    thumbs: HashMap<String, ThumbRefs>,
    /// Entrées du dernier scan de chaque source, par `entry_id` (installation, #15).
    entries: HashMap<String, HashMap<String, StoryEntry>>,
}

/// Mémoire du dernier scan de chaque source et cache de métadonnées (en mémoire).
#[derive(Clone, Default)]
pub struct FolderState(Arc<Mutex<Inner>>);

impl FolderState {
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Analyse un dossier et mémorise les entrées pour `thumbnail`.
    pub fn scan(&self, source_id: &str, root: &Path, recursive: bool) -> AppResult<FolderScan> {
        // Copie de travail du cache : l'analyse (longue) ne tient pas le verrou et plusieurs
        // scans simultanés se partagent le cache fusionné à la fin.
        let mut meta = self.lock().meta.clone();
        let result = scan_folder(source_id, root, recursive, &mut meta);
        let mut inner = self.lock();
        inner
            .meta
            .0
            .retain(|p, _| !p.starts_with(root) || meta.0.contains_key(p));
        inner.meta.0.extend(meta.0);
        let (scan, refs) = result?;
        inner.thumbs.insert(source_id.to_string(), refs);
        let entries = scan
            .entries
            .iter()
            .map(|e| (e.entry_id.clone(), e.clone()))
            .collect();
        inner.entries.insert(source_id.to_string(), entries);
        Ok(scan)
    }

    /// Entrée `entry_id` du dernier scan de `source_id` (jamais un chemin libre venu du front).
    pub fn entry(&self, source_id: &str, entry_id: &str) -> AppResult<StoryEntry> {
        self.lock()
            .entries
            .get(source_id)
            .and_then(|entries| entries.get(entry_id))
            .cloned()
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "histoire inconnue dans le dernier scan : {entry_id}"
                ))
            })
    }

    /// URL `data:` de la vignette d'une entrée du dernier scan de `source_id`.
    pub fn thumbnail(
        &self,
        cache_dir: &Path,
        source_id: &str,
        entry_id: &str,
    ) -> AppResult<String> {
        let thumb = self
            .lock()
            .thumbs
            .get(source_id)
            .and_then(|refs| refs.get(entry_id))
            .cloned()
            .ok_or_else(|| {
                AppError::NotFound(format!(
                    "histoire inconnue dans le dernier scan : {entry_id}"
                ))
            })?;
        make_thumbnail(&thumb, cache_dir)
    }
}

/// Chemin du dossier de la source et dossier de cache effectif.
pub(crate) fn resolve_folder(
    config_dir: &Path,
    env: &Environment,
    source_id: &str,
) -> AppResult<(PathBuf, PathBuf)> {
    match SourcesStore::new(config_dir).get(source_id)? {
        Source::Folder { path, .. } => {
            let path = PathBuf::from(path);
            if !path.is_absolute() {
                return Err(AppError::Validation(
                    "le chemin du dossier doit être absolu".into(),
                ));
            }
            let cache = SettingsStore::new(config_dir).resolved_cache_dir(env)?;
            Ok((path, cache))
        }
        _ => Err(AppError::Validation(
            "cette source n'est pas un dossier local".into(),
        )),
    }
}

/// Analyse la source dossier `source_id` ; `recursive` parcourt aussi les sous-dossiers.
/// Aucune image n'est renvoyée : voir `folder_thumbnail`.
#[tauri::command]
pub async fn folder_scan(
    config: State<'_, ConfigState>,
    folder: State<'_, FolderState>,
    source_id: String,
    recursive: Option<bool>,
) -> AppResult<FolderScan> {
    let (path, _cache_dir) = {
        let _guard = config.guard();
        resolve_folder(&config.dir, &Environment::current(), &source_id)?
    };
    let state = folder.inner().clone();
    // Verrou de configuration relâché : l'analyse peut être longue.
    tauri::async_runtime::spawn_blocking(move || {
        state.scan(&source_id, &path, recursive.unwrap_or(false))
    })
    .await
    .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?
}

/// Vignette (URL `data:image/png;base64,…`) d'une histoire du dernier scan de la source.
#[tauri::command]
pub async fn folder_thumbnail(
    config: State<'_, ConfigState>,
    folder: State<'_, FolderState>,
    source_id: String,
    entry_id: String,
) -> AppResult<String> {
    let cache_dir = {
        let _guard = config.guard();
        SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?
    };
    let state = folder.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.thumbnail(&cache_dir, &source_id, &entry_id))
        .await
        .map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::CompressionMethod;

    fn png_bytes(rgba: [u8; 4]) -> Vec<u8> {
        thumbnail::encode_png(&thumbnail::RgbaImage {
            width: 2,
            height: 2,
            rgba: rgba.repeat(4),
        })
        .unwrap()
    }

    fn write_zip(path: &Path, method: CompressionMethod, files: &[(&str, &[u8])]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default().compression_method(method);
        for (name, data) in files {
            zip.start_file(*name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    const META: &str = r#"{"title":"Le Petit Chaperon rouge","uuid":"abc-123","version":3,
        "image":"cover.png","category":"Conte","description":"Une histoire","age":"5"}"#;

    fn telmi_files<'a>(prefix: &str, meta: &'a [u8], png: &'a [u8]) -> Vec<(String, &'a [u8])> {
        vec![
            (format!("{prefix}metadata.json"), meta),
            (format!("{prefix}nodes.json"), b"{}"),
            (format!("{prefix}title.png"), png),
            (format!("{prefix}cover.png"), png),
        ]
    }

    fn write_telmi(path: &Path, prefix: &str, meta: &[u8]) {
        let png = png_bytes([200, 10, 10, 255]);
        let files = telmi_files(prefix, meta, &png);
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (n.as_str(), *d)).collect();
        write_zip(path, CompressionMethod::Deflated, &refs);
    }

    /// 16 octets d'uuid, `info.plain` et une vignette LIF 1x1 valide.
    fn flam_files(title: &str) -> Vec<(&'static str, Vec<u8>)> {
        let lif = {
            let mut v = b"liff".to_vec();
            v.extend_from_slice(&1u32.to_be_bytes());
            v.extend_from_slice(&1u32.to_be_bytes());
            v.push(0xA2);
            v.extend_from_slice(&[0xFF, 0x1F, 0xF8, 255]);
            v.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
            v
        };
        vec![
            (
                "info.plain",
                format!("{title}\nsous-titre\n000000\n{title}").into_bytes(),
            ),
            ("version", b"1".to_vec()),
            ("uuid.bin", (0u8..16).collect()),
            ("img/thumbnail.lif", lif),
        ]
    }

    fn write_plain_dir(dir: &Path, title: &str) {
        for (name, data) in flam_files(title) {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, data).unwrap();
        }
    }

    fn write_plain_pk(path: &Path, title: &str) {
        let files = flam_files(title);
        let refs: Vec<(&str, &[u8])> = files.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        write_zip(path, CompressionMethod::Stored, &refs);
    }

    struct Ctx {
        state: FolderState,
        cache: tempfile::TempDir,
    }

    impl Ctx {
        fn thumb(&self, entry: &StoryEntry) -> AppResult<String> {
            self.state
                .thumbnail(self.cache.path(), "src", &entry.entry_id)
        }
    }

    fn scan(root: &Path, recursive: bool) -> (FolderScan, Ctx) {
        let ctx = Ctx {
            state: FolderState::default(),
            cache: tempfile::tempdir().unwrap(),
        };
        let scan = ctx.state.scan("src", root, recursive).unwrap();
        (scan, ctx)
    }

    fn thumb_files(ctx: &Ctx) -> usize {
        std::fs::read_dir(ctx.cache.path().join("thumbs")).map_or(0, |d| d.count())
    }

    #[test]
    fn zip_telmi_avec_dossier_racine_et_bom() {
        let dir = tempfile::tempdir().unwrap();
        let mut meta = vec![0xEF, 0xBB, 0xBF];
        meta.extend_from_slice(META.as_bytes());
        write_telmi(&dir.path().join("histoire.zip"), "Mon histoire/", &meta);

        let (scan, ctx) = scan(dir.path(), false);
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
        let e = &scan.entries[0];
        assert_eq!(e.source_id, "src");
        assert_eq!(e.uuid, "abc-123");
        assert_eq!(e.title, "Le Petit Chaperon rouge");
        assert_eq!(e.version, Some(3));
        assert_eq!(e.age, Some(5));
        assert_eq!(e.category.as_deref(), Some("Conte"));
        assert_eq!(e.description.as_deref(), Some("Une histoire"));
        assert_eq!(e.kind, StoryKind::TelmiZip);
        assert!(e.download_url.ends_with("histoire.zip"));
        assert!(e.size.unwrap() > 0);
        assert!(e.updated_at.as_deref().unwrap().ends_with('Z'));
        // `thumb_url` est une référence opaque : aucune image dans le scan.
        assert_eq!(e.thumb_url.as_deref(), Some(e.entry_id.as_str()));
        assert!(e.entry_id.starts_with("src:"));
        assert_eq!(thumb_files(&ctx), 0);
        assert!(ctx.thumb(e).unwrap().starts_with("data:image/png;base64,"));
        assert_eq!(thumb_files(&ctx), 1);
    }

    #[test]
    fn zip_telmi_sans_dossier_racine() {
        let dir = tempfile::tempdir().unwrap();
        write_telmi(&dir.path().join("a.ZIP"), "", META.as_bytes());
        let (scan, _cache) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1);
        assert_eq!(scan.entries[0].title, "Le Petit Chaperon rouge");
    }

    #[test]
    fn zip_telmi_macosx_et_racine_la_moins_profonde() {
        let dir = tempfile::tempdir().unwrap();
        let png = png_bytes([1, 2, 3, 255]);
        write_zip(
            &dir.path().join("a.zip"),
            CompressionMethod::Deflated,
            &[
                ("__MACOSX/Hist/metadata.json", b"pas du json"),
                ("__MACOSX/Hist/nodes.json", b"{}"),
                ("Hist/metadata.json", META.as_bytes()),
                ("Hist/nodes.json", b"{}"),
                ("Hist/title.png", &png),
            ],
        );
        let (scan, _cache) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1, "{:?}", scan.warnings);
        assert!(scan.entries[0].thumb_url.is_some());
    }

    #[test]
    fn zip_telmi_plusieurs_histoires_ou_sans_nodes_est_ignore() {
        let dir = tempfile::tempdir().unwrap();
        write_zip(
            &dir.path().join("deux.zip"),
            CompressionMethod::Deflated,
            &[
                ("A/metadata.json", META.as_bytes()),
                ("A/nodes.json", b"{}"),
                ("B/metadata.json", META.as_bytes()),
                ("B/nodes.json", b"{}"),
            ],
        );
        write_zip(
            &dir.path().join("sans_nodes.zip"),
            CompressionMethod::Deflated,
            &[("metadata.json", META.as_bytes())],
        );
        let (scan, _cache) = scan(dir.path(), false);
        assert!(scan.entries.is_empty());
        assert_eq!(scan.warnings.len(), 2);
    }

    #[test]
    fn noms_non_ascii_et_vignette_par_defaut_title_png() {
        let dir = tempfile::tempdir().unwrap();
        let meta = r#"{"title":"Été à Noël ✨","uuid":"u","image":"inconnu.png"}"#;
        write_telmi(
            &dir.path().join("Histoire d'été éàü.zip"),
            "Dossier é/",
            meta.as_bytes(),
        );
        let (scan, _cache) = scan(dir.path(), false);
        let e = &scan.entries[0];
        assert_eq!(e.title, "Été à Noël ✨");
        assert!(e.download_url.contains("Histoire d'été éàü.zip"));
        assert!(e.thumb_url.is_some());
        assert_eq!(e.version, None);
        assert_eq!(e.age, None);
    }

    #[test]
    fn metadata_invalides_ignorees_avec_avertissement() {
        let dir = tempfile::tempdir().unwrap();
        for (name, meta) in [
            ("json.zip", &b"{pas du json"[..]),
            ("sans_titre.zip", br#"{"uuid":"x"}"#),
            ("sans_uuid.zip", br#"{"title":"x"}"#),
            ("tableau.zip", b"[1]"),
            ("latin1.zip", b"{\"title\":\"\xe9\",\"uuid\":\"x\"}"),
        ] {
            write_zip(
                &dir.path().join(name),
                CompressionMethod::Deflated,
                &[("metadata.json", meta), ("nodes.json", b"{}")],
            );
        }
        let (scan, _cache) = scan(dir.path(), false);
        assert!(scan.entries.is_empty());
        assert_eq!(scan.warnings.len(), 5);
    }

    #[test]
    fn metadata_trop_volumineux_refuse() {
        let dir = tempfile::tempdir().unwrap();
        let big = vec![b' '; MAX_TEXT_BYTES as usize + 1];
        write_zip(
            &dir.path().join("gros.zip"),
            CompressionMethod::Deflated,
            &[("metadata.json", &big), ("nodes.json", b"{}")],
        );
        let (scan, _cache) = scan(dir.path(), false);
        assert!(scan.entries.is_empty());
        assert!(scan.warnings[0].message.contains("trop volumineux"));
    }

    #[test]
    fn vignette_invalide_garde_l_histoire_avec_avertissement() {
        let dir = tempfile::tempdir().unwrap();
        write_zip(
            &dir.path().join("a.zip"),
            CompressionMethod::Deflated,
            &[
                ("metadata.json", META.as_bytes()),
                ("nodes.json", b"{}"),
                ("cover.png", b"pas un png"),
            ],
        );
        let (scan, ctx) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.warnings.is_empty());
        assert_eq!(
            ctx.thumb(&scan.entries[0]).unwrap_err().kind(),
            "validation"
        );
    }

    #[test]
    fn dossier_plain_et_plain_pk() {
        let dir = tempfile::tempdir().unwrap();
        write_plain_dir(
            &dir.path().join("Été.1A2B3C4D.plain"),
            "Ça va être « génial »",
        );
        write_plain_pk(&dir.path().join("histoire.1A2B3C4D.plain.pk"), "Titre pk");
        let (scan, ctx) = scan(dir.path(), false);
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
        assert_eq!(scan.entries.len(), 2);
        let plain = scan
            .entries
            .iter()
            .find(|e| e.kind == StoryKind::FlamPlain)
            .unwrap();
        assert_eq!(plain.title, "Ça va être « génial »");
        assert_eq!(plain.uuid, "000102030405060708090a0b0c0d0e0f");
        assert_eq!(plain.size, None);
        let pk = scan
            .entries
            .iter()
            .find(|e| e.kind == StoryKind::FlamPk)
            .unwrap();
        assert_eq!(pk.title, "Titre pk");
        assert!(pk.size.unwrap() > 0);
        for e in [plain, pk] {
            let uri = ctx.thumb(e).unwrap();
            let b64 = uri.strip_prefix("data:image/png;base64,").unwrap();
            let png =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).unwrap();
            let img = thumbnail::decode_png(&png).unwrap();
            assert_eq!(img.rgba, vec![248, 0, 248, 255]);
        }
    }

    #[test]
    fn flam_invalides_ignores() {
        let dir = tempfile::tempdir().unwrap();
        // .plain sans uuid.bin
        let no_uuid = dir.path().join("a.plain");
        std::fs::create_dir(&no_uuid).unwrap();
        std::fs::write(no_uuid.join("info.plain"), "Titre").unwrap();
        // .plain avec uuid.bin de 15 octets
        let short = dir.path().join("b.plain");
        write_plain_dir(&short, "Titre");
        std::fs::write(short.join("uuid.bin"), [0u8; 15]).unwrap();
        // .plain avec titre vide
        let empty = dir.path().join("c.plain");
        write_plain_dir(&empty, "");
        // .plain.pk qui n'est pas un zip
        std::fs::write(dir.path().join("d.plain.pk"), b"pas un zip").unwrap();
        // fichiers sans rapport
        std::fs::write(dir.path().join("notes.txt"), "x").unwrap();
        std::fs::create_dir(dir.path().join("dossier")).unwrap();
        let (scan, _cache) = scan(dir.path(), false);
        assert!(scan.entries.is_empty());
        assert_eq!(scan.warnings.len(), 4, "{:?}", scan.warnings);
    }

    #[test]
    fn thumbnail_lif_invalide_garde_l_histoire() {
        let dir = tempfile::tempdir().unwrap();
        let story = dir.path().join("a.plain");
        write_plain_dir(&story, "Titre");
        std::fs::write(
            story.join("img/thumbnail.lif"),
            b"pas un lif du tout, vraiment",
        )
        .unwrap();
        let (scan, ctx) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.warnings.is_empty());
        assert_eq!(
            ctx.thumb(&scan.entries[0]).unwrap_err().kind(),
            "validation"
        );
    }

    #[test]
    fn parcours_recursif_optionnel() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sous").join("dossier");
        std::fs::create_dir_all(&sub).unwrap();
        write_telmi(&sub.join("profond.zip"), "", META.as_bytes());
        write_telmi(&dir.path().join("racine.zip"), "", META.as_bytes());
        // Le contenu d'un dossier .plain n'est pas parcouru comme un dossier ordinaire.
        write_plain_dir(&dir.path().join("x.plain"), "X");
        assert_eq!(scan(dir.path(), false).0.entries.len(), 2);
        assert_eq!(scan(dir.path(), true).0.entries.len(), 3);
    }

    #[test]
    fn dossier_absent_ou_fichier_donne_folder_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let state = FolderState::default();
        let err = state
            .scan("s", &dir.path().join("absent"), false)
            .unwrap_err();
        assert_eq!(err.kind(), "folder_unavailable");
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let err = state.scan("s", &file, false).unwrap_err();
        assert_eq!(err.kind(), "folder_unavailable");
    }

    #[test]
    fn le_cache_de_vignettes_est_reutilise() {
        let dir = tempfile::tempdir().unwrap();
        write_telmi(&dir.path().join("a.zip"), "", META.as_bytes());
        let (first, ctx) = scan(dir.path(), false);
        let uri = ctx.thumb(&first.entries[0]).unwrap();
        assert_eq!(ctx.thumb(&first.entries[0]).unwrap(), uri);
        assert_eq!(thumb_files(&ctx), 1);
        // Deuxième analyse (métadonnées en cache) : mêmes entrées.
        let second = ctx.state.scan("src", dir.path(), false).unwrap();
        assert_eq!(first.entries, second.entries);
    }

    #[test]
    fn find_root_ecarte_les_noms_dangereux() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(find_root(&names(&["../metadata.json", "../nodes.json"])).is_err());
        assert!(find_root(&names(&["/abs/metadata.json", "/abs/nodes.json"])).is_err());
        assert!(find_root(&names(&["C:/x/metadata.json", "C:/x/nodes.json"])).is_err());
        assert!(find_root(&names(&["._a/metadata.json", "._a/nodes.json"])).is_err());
        // « xmetadata.json » n'est pas « metadata.json ».
        assert!(find_root(&names(&["xmetadata.json", "xnodes.json"])).is_err());
        assert_eq!(
            find_root(&names(&["d/metadata.json", "d/nodes.json"])).unwrap(),
            "d/"
        );
    }

    #[test]
    fn dates_iso8601() {
        let at = |s| iso8601(UNIX_EPOCH + std::time::Duration::from_secs(s)).unwrap();
        assert_eq!(at(0), "1970-01-01T00:00:00Z");
        assert_eq!(at(1_784_000_000), "2026-07-14T03:33:20Z");
        assert_eq!(at(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn resolution_de_la_source_dossier() {
        let config = tempfile::tempdir().unwrap();
        let folder = tempfile::tempdir().unwrap();
        let env = Environment {
            exe_dir: None,
            path_var: None,
            default_cache_dir: config.path().join("cache"),
            search_repo_build: false,
            protected_dirs: vec![],
        };
        let store = SourcesStore::new(config.path());
        let source = store
            .add(crate::sources::SourceInput::Folder {
                name: "Mon dossier".into(),
                path: folder.path().to_string_lossy().into_owned(),
            })
            .unwrap();
        let (path, cache) = resolve_folder(config.path(), &env, source.id()).unwrap();
        assert_eq!(path, folder.path());
        assert!(cache.is_dir());
        assert_eq!(
            resolve_folder(config.path(), &env, "library")
                .unwrap_err()
                .kind(),
            "validation"
        );
        assert_eq!(
            resolve_folder(config.path(), &env, "telmi-store")
                .unwrap_err()
                .kind(),
            "validation"
        );
        assert_eq!(
            resolve_folder(config.path(), &env, "inconnu")
                .unwrap_err()
                .kind(),
            "not_found"
        );
    }

    #[test]
    fn classify_n_accepte_que_les_fichiers_ordinaires_et_les_dossiers_plain() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.zip");
        std::fs::write(&file, "x").unwrap();
        let sub = dir.path().join("y.plain");
        std::fs::create_dir(&sub).unwrap();
        let file_type = |p: &Path| std::fs::metadata(p).unwrap().file_type();
        assert_eq!(
            classify("x.ZIP", &file_type(&file)),
            Some(StoryKind::TelmiZip)
        );
        assert_eq!(
            classify("y.plain", &file_type(&sub)),
            Some(StoryKind::FlamPlain)
        );
        // Un dossier nommé x.zip ou un fichier nommé y.plain ne sont pas des histoires.
        assert_eq!(classify("x.zip", &file_type(&sub)), None);
        assert_eq!(classify("y.plain", &file_type(&file)), None);
    }

    #[test]
    fn uuid_et_version_dangereux_refuses() {
        let dir = tempfile::tempdir().unwrap();
        let long = "a".repeat(65);
        let bad = [
            "../x", "a\\b", "CON", "com1.txt", "NUL", "-a", "a b", "a..b", "", &long,
        ];
        for (i, uuid) in bad.iter().enumerate() {
            let meta = serde_json::json!({"title": "T", "uuid": uuid}).to_string();
            write_telmi(&dir.path().join(format!("u{i}.zip")), "", meta.as_bytes());
        }
        let meta = r#"{"title":"T","uuid":"ok","version":"../v"}"#;
        write_telmi(&dir.path().join("v.zip"), "", meta.as_bytes());
        let (scan, _ctx) = scan(dir.path(), false);
        assert!(scan.entries.is_empty(), "{:?}", scan.entries);
        assert_eq!(scan.warnings.len(), bad.len() + 1);
        assert!(validate_story_id(&"a".repeat(64)));
        assert!(validate_story_id("ffffff-199f066d199"));
    }

    #[test]
    fn version_chaine_numerique_acceptee() {
        let dir = tempfile::tempdir().unwrap();
        let meta = r#"{"title":"T","uuid":"ok","version":"2"}"#;
        write_telmi(&dir.path().join("a.zip"), "", meta.as_bytes());
        let (scan, _ctx) = scan(dir.path(), false);
        assert_eq!(scan.entries[0].version, Some(2));
    }

    #[test]
    fn noms_de_zip_avec_antislashs_normalises() {
        let dir = tempfile::tempdir().unwrap();
        let png = png_bytes([1, 2, 3, 255]);
        write_zip(
            &dir.path().join("a.zip"),
            CompressionMethod::Deflated,
            &[
                ("Hist\\metadata.json", META.as_bytes()),
                ("Hist\\nodes.json", b"{}"),
                ("Hist\\cover.png", &png),
            ],
        );
        let (scan, ctx) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1, "{:?}", scan.warnings);
        assert!(ctx.thumb(&scan.entries[0]).is_ok());
    }

    #[test]
    fn plain_et_pk_de_meme_uuid_ont_deux_entry_id_stables() {
        let dir = tempfile::tempdir().unwrap();
        write_plain_dir(&dir.path().join("x.plain"), "Titre");
        write_plain_pk(&dir.path().join("x.plain.pk"), "Titre");
        let (first, ctx) = scan(dir.path(), false);
        assert_eq!(first.entries.len(), 2);
        assert_eq!(first.entries[0].uuid, first.entries[1].uuid);
        assert_ne!(first.entries[0].entry_id, first.entries[1].entry_id);
        let second = ctx.state.scan("src", dir.path(), false).unwrap();
        assert_eq!(first.entries, second.entries);
    }

    #[test]
    fn folder_thumbnail_refuse_les_entrees_inconnues() {
        let dir = tempfile::tempdir().unwrap();
        write_telmi(&dir.path().join("a.zip"), "", META.as_bytes());
        let (scan, ctx) = scan(dir.path(), false);
        let id = scan.entries[0].entry_id.clone();
        let path = scan.entries[0].download_url.clone();
        for (source, entry) in [
            ("src", "deadbeef"),
            ("autre", id.as_str()),
            ("src", path.as_str()),
        ] {
            let err = ctx
                .state
                .thumbnail(ctx.cache.path(), source, entry)
                .unwrap_err();
            assert_eq!(err.kind(), "not_found");
        }
    }

    #[test]
    fn entry_ne_renvoie_que_les_entrees_du_dernier_scan() {
        let dir = tempfile::tempdir().unwrap();
        write_telmi(&dir.path().join("a.zip"), "", META.as_bytes());
        let (scan, ctx) = scan(dir.path(), false);
        let id = scan.entries[0].entry_id.clone();
        assert_eq!(ctx.state.entry("src", &id).unwrap(), scan.entries[0]);
        let path = scan.entries[0].download_url.clone();
        for (source, entry) in [("src", "x"), ("autre", id.as_str()), ("src", &path)] {
            assert_eq!(
                ctx.state.entry(source, entry).unwrap_err().kind(),
                "not_found"
            );
        }
    }

    #[test]
    fn rescan_detecte_un_fichier_modifie() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.zip");
        write_telmi(&path, "", br#"{"title":"Avant","uuid":"u"}"#);
        let (first, ctx) = scan(dir.path(), false);
        assert_eq!(first.entries[0].title, "Avant");
        write_telmi(
            &path,
            "",
            r#"{"title":"Après modification","uuid":"u"}"#.as_bytes(),
        );
        let second = ctx.state.scan("src", dir.path(), false).unwrap();
        assert_eq!(second.entries[0].title, "Après modification");
        std::fs::remove_file(&path).unwrap();
        let third = ctx.state.scan("src", dir.path(), false).unwrap();
        assert!(third.entries.is_empty());
    }

    #[test]
    fn la_cle_de_cache_d_un_plain_suit_thumbnail_lif() {
        let dir = tempfile::tempdir().unwrap();
        let story = dir.path().join("a.plain");
        write_plain_dir(&story, "Titre");
        let (scan, ctx) = scan(dir.path(), false);
        let before = ctx.thumb(&scan.entries[0]).unwrap();
        let mut lif = b"liff".to_vec();
        lif.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1, 0xA2, 0xFF, 0x00, 0x07, 255, 0]);
        lif.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
        std::fs::write(story.join("img/thumbnail.lif"), lif).unwrap();
        let after = ctx.thumb(&scan.entries[0]).unwrap();
        assert_ne!(before, after);
        assert_eq!(thumb_files(&ctx), 2);
    }

    #[test]
    fn png_surdimensionne_refuse_a_la_demande() {
        let dir = tempfile::tempdir().unwrap();
        let mut png = Vec::new();
        let mut enc = png::Encoder::new(&mut png, 9000, 1);
        enc.set_color(png::ColorType::Grayscale);
        enc.set_depth(png::BitDepth::One);
        enc.write_header()
            .unwrap()
            .write_image_data(&vec![0u8; 9000usize.div_ceil(8)])
            .unwrap();
        write_zip(
            &dir.path().join("a.zip"),
            CompressionMethod::Deflated,
            &[
                ("metadata.json", META.as_bytes()),
                ("nodes.json", b"{}"),
                ("cover.png", &png),
            ],
        );
        let (scan, ctx) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1);
        assert!(ctx.thumb(&scan.entries[0]).is_err());
    }

    /// Remplace la taille décompressée déclarée (répertoire central) de `name`.
    fn lie_about_size(path: &Path, name: &str, fake: u32) {
        let mut data = std::fs::read(path).unwrap();
        let mut at = 0;
        while let Some(pos) = data[at..].windows(4).position(|w| w == [0x50, 0x4b, 1, 2]) {
            let start = at + pos;
            let name_len = u16::from_le_bytes([data[start + 28], data[start + 29]]) as usize;
            if &data[start + 46..start + 46 + name_len] == name.as_bytes() {
                data[start + 24..start + 28].copy_from_slice(&fake.to_le_bytes());
            }
            at = start + 4;
        }
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn taille_declaree_mensongere_ne_contourne_pas_la_limite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bombe.zip");
        let big = vec![b' '; 2 * 1024 * 1024];
        write_zip(
            &path,
            CompressionMethod::Deflated,
            &[("metadata.json", &big), ("nodes.json", b"{}")],
        );
        lie_about_size(&path, "metadata.json", 100);
        let (scan, _ctx) = scan(dir.path(), false);
        assert!(scan.entries.is_empty());
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn liens_symboliques_ignores() {
        let dir = tempfile::tempdir().unwrap();
        let real = tempfile::tempdir().unwrap();
        write_telmi(&real.path().join("cible.zip"), "", META.as_bytes());
        write_plain_dir(&real.path().join("cible.plain"), "Titre");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(
            real.path().join("cible.zip"),
            dir.path().join("lien.zip"),
        )
        .and(std::os::windows::fs::symlink_dir(
            real.path().join("cible.plain"),
            dir.path().join("lien.plain"),
        ))
        .and(std::os::windows::fs::symlink_dir(
            real.path(),
            dir.path().join("sous-dossier"),
        ));
        #[cfg(unix)]
        let linked =
            std::os::unix::fs::symlink(real.path().join("cible.zip"), dir.path().join("lien.zip"))
                .and(std::os::unix::fs::symlink(
                    real.path().join("cible.plain"),
                    dir.path().join("lien.plain"),
                ))
                .and(std::os::unix::fs::symlink(
                    real.path(),
                    dir.path().join("sous-dossier"),
                ));
        if linked.is_err() {
            return; // création de liens non autorisée sur cette machine
        }
        let (scan, _ctx) = scan(dir.path(), true);
        assert!(scan.entries.is_empty(), "{:?}", scan.entries);
    }

    #[cfg(windows)]
    #[test]
    fn jonctions_ignorees() {
        let dir = tempfile::tempdir().unwrap();
        let real = tempfile::tempdir().unwrap();
        write_telmi(&real.path().join("a.zip"), "", META.as_bytes());
        let junction = dir.path().join("jonction");
        let made = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(real.path())
            .output()
            .is_ok_and(|o| o.status.success());
        if !made {
            return;
        }
        let (scan, _ctx) = scan(dir.path(), true);
        assert!(scan.entries.is_empty(), "{:?}", scan.entries);
    }
}
