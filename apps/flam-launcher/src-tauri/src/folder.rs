//! Source « dossier local » : analyse d'un dossier d'histoires (`.zip` Telmi, dossiers
//! `.plain`, archives `.plain.pk`) et production de `StoryEntry`.
//!
//! Les archives sont non fiables : rien n'est extrait sur disque (seules les vignettes
//! réduites vont dans `<cache>/thumbs/`), les lectures sont bornées, les noms d'entrées
//! dangereux ignorés et les liens symboliques jamais suivis. Un fichier invalide ne fait pas
//! échouer l'analyse : il est ignoré et signalé dans `warnings`.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::Value;
use tauri::State;
use zip::result::ZipError;
use zip::ZipArchive;

use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::ConfigState;
use crate::story::{StoryEntry, StoryKind};
use crate::thumbnail::{self, ThumbCache, ThumbError};
use crate::{lif, AppError, AppResult};

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
// Lecture bornée de fichiers (dossier ou archive)
// ---------------------------------------------------------------------------

/// Accès en lecture aux fichiers d'une histoire FLAM (dossier `.plain` ou `.plain.pk`).
trait FlamFiles {
    /// Lit un fichier (au plus `limit` octets) ; `Ok(None)` s'il n'existe pas.
    fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String>;
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
}

/// Lecture d'une entrée d'archive par son nom exact.
fn read_zip_entry(
    zip: &mut ZipArchive<File>,
    name: &str,
    limit: u64,
) -> Result<Option<Vec<u8>>, String> {
    let entry = match zip.by_name(name) {
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
    read_limited(entry, limit, name).map(Some)
}

struct ZipFiles(ZipArchive<File>);

impl FlamFiles for ZipFiles {
    fn read(&mut self, name: &str, limit: u64) -> Result<Option<Vec<u8>>, String> {
        read_zip_entry(&mut self.0, name, limit)
    }
}

fn open_zip(path: &Path) -> Result<ZipArchive<File>, String> {
    let file = File::open(path).map_err(|e| format!("ouverture impossible : {e}"))?;
    let zip = ZipArchive::new(file).map_err(|e| format!("archive zip invalide : {e}"))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err("archive zip avec trop d'entrées".into());
    }
    Ok(zip)
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
fn iso8601(time: SystemTime) -> Option<String> {
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

/// Nom d'entrée sans élément dangereux (absolu, `..`, lettre de lecteur, `__MACOSX`, `._*`).
fn is_safe_entry_name(name: &str) -> bool {
    !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains(':')
        && name
            .split('/')
            .all(|part| part != ".." && part != "__MACOSX" && !part.starts_with("._"))
}

/// Dossier racine de l'histoire dans l'archive : celui qui contient `metadata.json` et
/// `nodes.json` (miroir de `find_root` de `telmi2flam.py`). Le moins profond gagne ;
/// plusieurs candidats au même niveau est une erreur.
fn find_root(names: &[String]) -> Result<String, String> {
    let set: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
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

struct FileInfo {
    size: u64,
    updated_at: Option<String>,
    /// Clé de cache des vignettes : change quand le fichier change.
    cache_key: u64,
}

fn file_info(path: &Path, meta: &std::fs::Metadata) -> FileInfo {
    let modified = meta.modified().ok();
    let secs = modified
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos());
    FileInfo {
        size: meta.len(),
        updated_at: modified.and_then(iso8601),
        cache_key: thumbnail::fnv1a(&[
            path.to_string_lossy().as_bytes(),
            &meta.len().to_le_bytes(),
            &secs.to_le_bytes(),
        ]),
    }
}

/// Résultat d'analyse d'une archive : l'histoire et les avertissements non bloquants.
type Parsed = (StoryEntry, Vec<String>);

fn telmi_entry(
    source_id: &str,
    path: &Path,
    info: &FileInfo,
    cache: &ThumbCache,
) -> Result<Parsed, String> {
    let mut zip = open_zip(path)?;
    let names: Vec<String> = zip.file_names().map(str::to_string).collect();
    let root = find_root(&names)?;

    let raw = read_zip_entry(&mut zip, &format!("{root}metadata.json"), MAX_TEXT_BYTES)?
        .ok_or("metadata.json introuvable")?;
    let meta: Value = serde_json::from_str(&utf8_text(&raw, "metadata.json")?)
        .map_err(|e| format!("metadata.json invalide : {e}"))?;
    if !meta.is_object() {
        return Err("metadata.json invalide : objet attendu".into());
    }
    let title =
        json_text(&meta, "title", MAX_TITLE_CHARS).ok_or("titre absent de metadata.json")?;
    let uuid = json_text(&meta, "uuid", 100).ok_or("uuid absent de metadata.json")?;

    // Seuls `title.png` et `cover.png` sont lus, jamais un nom venant de l'archive.
    let preferred = if json_text(&meta, "image", 20).as_deref() == Some("cover.png") {
        ["cover.png", "title.png"]
    } else {
        ["title.png", "cover.png"]
    };
    let mut warnings = Vec::new();
    let mut thumb_url = None;
    for name in preferred {
        match read_zip_entry(&mut zip, &format!("{root}{name}"), MAX_IMAGE_BYTES) {
            Ok(Some(bytes)) => {
                let key = thumbnail::fnv1a(&[&info.cache_key.to_le_bytes(), name.as_bytes()]);
                match cache.get_or_make::<ThumbError>(key, || thumbnail::decode_png(&bytes)) {
                    Ok(uri) => thumb_url = Some(uri),
                    Err(e) => warnings.push(format!("vignette {name} ignorée : {e}")),
                }
                break;
            }
            Ok(None) => {}
            Err(e) => {
                warnings.push(format!("vignette ignorée : {e}"));
                break;
            }
        }
    }

    let entry = StoryEntry {
        source_id: source_id.to_string(),
        uuid,
        version: json_u32(&meta, "version"),
        title,
        description: json_text(&meta, "description", MAX_FIELD_CHARS).unwrap_or_default(),
        age: json_u32(&meta, "age"),
        category: json_text(&meta, "category", MAX_FIELD_CHARS).unwrap_or_default(),
        author: json_text(&meta, "author", MAX_FIELD_CHARS).unwrap_or_default(),
        voice: json_text(&meta, "voice", MAX_FIELD_CHARS).unwrap_or_default(),
        license: json_text(&meta, "license", MAX_FIELD_CHARS).unwrap_or_default(),
        thumb_url,
        download_url: path.to_string_lossy().into_owned(),
        size: Some(info.size),
        updated_at: info.updated_at.clone().unwrap_or_default(),
        kind: StoryKind::TelmiZip,
    };
    Ok((entry, warnings))
}

// ---------------------------------------------------------------------------
// Histoire FLAM (.plain, .plain.pk)
// ---------------------------------------------------------------------------

fn flam_entry(
    source_id: &str,
    path: &Path,
    kind: StoryKind,
    info: &FileInfo,
    files: &mut dyn FlamFiles,
    cache: &ThumbCache,
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

    let mut warnings = Vec::new();
    let mut thumb_url = None;
    match files.read("img/thumbnail.lif", MAX_IMAGE_BYTES) {
        Ok(Some(bytes)) => {
            let key = thumbnail::fnv1a(&[&info.cache_key.to_le_bytes(), b"thumbnail.lif"]);
            match cache.get_or_make::<ThumbError>(key, || Ok(lif::decode(&bytes)?)) {
                Ok(uri) => thumb_url = Some(uri),
                Err(e) => warnings.push(format!("vignette img/thumbnail.lif ignorée : {e}")),
            }
        }
        Ok(None) => {}
        Err(e) => warnings.push(format!("vignette ignorée : {e}")),
    }

    let entry = StoryEntry {
        source_id: source_id.to_string(),
        uuid,
        version: None,
        title,
        description: String::new(),
        age: None,
        category: String::new(),
        author: String::new(),
        voice: String::new(),
        license: String::new(),
        thumb_url,
        download_url: path.to_string_lossy().into_owned(),
        size: (kind == StoryKind::FlamPk).then_some(info.size),
        updated_at: info.updated_at.clone().unwrap_or_default(),
        kind,
    };
    Ok((entry, warnings))
}

// ---------------------------------------------------------------------------
// Parcours du dossier
// ---------------------------------------------------------------------------

/// Classe un nom de fichier ou de dossier (insensible à la casse).
fn classify(name: &str, is_dir: bool) -> Option<StoryKind> {
    let lower = name.to_lowercase();
    if is_dir {
        lower.ends_with(".plain").then_some(StoryKind::FlamPlain)
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
        // `file_type` ne suit pas les liens symboliques : ils sont ignorés.
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
        if let Some(kind) = classify(&name, file_type.is_dir()) {
            found.push((path, kind));
        } else if file_type.is_dir() && recursive && depth < MAX_DEPTH {
            collect(&path, depth + 1, recursive, found, warnings);
        }
    }
}

/// Analyse `root` (non récursif par défaut) et renvoie les histoires et les avertissements.
pub fn scan_folder(
    source_id: &str,
    root: &Path,
    recursive: bool,
    cache_dir: &Path,
) -> AppResult<FolderScan> {
    match std::fs::metadata(root) {
        Ok(meta) if meta.is_dir() => {}
        _ => {
            return Err(AppError::FolderUnavailable(format!(
                "dossier introuvable ou inaccessible : {}",
                root.display()
            )))
        }
    }
    let cache = ThumbCache::new(cache_dir);
    let mut scan = FolderScan::default();
    let mut found = Vec::new();
    collect(root, 0, recursive, &mut found, &mut scan.warnings);

    for (path, kind) in found {
        let shown = path.to_string_lossy().into_owned();
        let parsed = std::fs::metadata(&path)
            .map_err(|e| format!("illisible : {e}"))
            .and_then(|meta| {
                let info = file_info(&path, &meta);
                match kind {
                    StoryKind::TelmiZip => telmi_entry(source_id, &path, &info, &cache),
                    StoryKind::FlamPlain => {
                        flam_entry(source_id, &path, kind, &info, &mut DirFiles(&path), &cache)
                    }
                    StoryKind::FlamPk => {
                        let mut files = ZipFiles(open_zip(&path)?);
                        flam_entry(source_id, &path, kind, &info, &mut files, &cache)
                    }
                }
            });
        match parsed {
            Ok((entry, notes)) => {
                scan.entries.push(entry);
                scan.warnings
                    .extend(notes.into_iter().map(|message| ScanWarning {
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
    Ok(scan)
}

// ---------------------------------------------------------------------------
// Commande Tauri
// ---------------------------------------------------------------------------

/// Chemin du dossier de la source et dossier de cache effectif.
fn resolve_folder(
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
#[tauri::command]
pub async fn folder_scan(
    state: State<'_, ConfigState>,
    source_id: String,
    recursive: Option<bool>,
) -> AppResult<FolderScan> {
    let (path, cache_dir) = {
        let _guard = state.guard();
        resolve_folder(&state.dir, &Environment::current(), &source_id)?
    };
    // Verrou relâché : l'analyse peut être longue et ne touche pas à la configuration.
    tauri::async_runtime::spawn_blocking(move || {
        scan_folder(&source_id, &path, recursive.unwrap_or(false), &cache_dir)
    })
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

    fn scan(root: &Path, recursive: bool) -> (FolderScan, tempfile::TempDir) {
        let cache = tempfile::tempdir().unwrap();
        let scan = scan_folder("src", root, recursive, cache.path()).unwrap();
        (scan, cache)
    }

    #[test]
    fn zip_telmi_avec_dossier_racine_et_bom() {
        let dir = tempfile::tempdir().unwrap();
        let mut meta = vec![0xEF, 0xBB, 0xBF];
        meta.extend_from_slice(META.as_bytes());
        write_telmi(&dir.path().join("histoire.zip"), "Mon histoire/", &meta);

        let (scan, cache) = scan(dir.path(), false);
        assert!(scan.warnings.is_empty(), "{:?}", scan.warnings);
        let e = &scan.entries[0];
        assert_eq!(e.source_id, "src");
        assert_eq!(e.uuid, "abc-123");
        assert_eq!(e.title, "Le Petit Chaperon rouge");
        assert_eq!(e.version, Some(3));
        assert_eq!(e.age, Some(5));
        assert_eq!(e.category, "Conte");
        assert_eq!(e.description, "Une histoire");
        assert_eq!(e.kind, StoryKind::TelmiZip);
        assert!(e.download_url.ends_with("histoire.zip"));
        assert!(e.size.unwrap() > 0);
        assert!(e.updated_at.ends_with('Z'));
        let uri = e.thumb_url.as_deref().unwrap();
        assert!(uri.starts_with("data:image/png;base64,"));
        assert_eq!(
            std::fs::read_dir(cache.path().join("thumbs"))
                .unwrap()
                .count(),
            1
        );
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
        let (scan, _cache) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].thumb_url.is_none());
        assert_eq!(scan.warnings.len(), 1);
    }

    #[test]
    fn dossier_plain_et_plain_pk() {
        let dir = tempfile::tempdir().unwrap();
        write_plain_dir(
            &dir.path().join("Été.1A2B3C4D.plain"),
            "Ça va être « génial »",
        );
        write_plain_pk(&dir.path().join("histoire.1A2B3C4D.plain.pk"), "Titre pk");
        let (scan, _cache) = scan(dir.path(), false);
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
            let uri = e.thumb_url.as_deref().unwrap();
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
        let (scan, _cache) = scan(dir.path(), false);
        assert_eq!(scan.entries.len(), 1);
        assert!(scan.entries[0].thumb_url.is_none());
        assert_eq!(scan.warnings.len(), 1);
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
        let cache = tempfile::tempdir().unwrap();
        let err = scan_folder("s", &dir.path().join("absent"), false, cache.path()).unwrap_err();
        assert_eq!(err.kind(), "folder_unavailable");
        let file = dir.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let err = scan_folder("s", &file, false, cache.path()).unwrap_err();
        assert_eq!(err.kind(), "folder_unavailable");
    }

    #[test]
    fn le_cache_de_vignettes_est_reutilise() {
        let dir = tempfile::tempdir().unwrap();
        write_telmi(&dir.path().join("a.zip"), "", META.as_bytes());
        let cache = tempfile::tempdir().unwrap();
        let first = scan_folder("s", dir.path(), false, cache.path()).unwrap();
        let second = scan_folder("s", dir.path(), false, cache.path()).unwrap();
        assert_eq!(first.entries, second.entries);
        assert_eq!(
            std::fs::read_dir(cache.path().join("thumbs"))
                .unwrap()
                .count(),
            1
        );
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
}
