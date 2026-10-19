//! Empreinte de la source locale d'une histoire installée (#288).
//!
//! Une histoire d'une source dossier est **copiée** dans la bibliothèque
//! (`<cache>/library/<uuid>-<version>.plain.pk`). Reconstruite avec le même `uuid` et la même
//! version, sa source change sans que le nom de la copie change : la copie de la bibliothèque
//! (et donc ce qui part sur la Flam) resterait l'ancienne. On mémorise donc à l'installation
//! une empreinte de ce qui a été copié ([`SourceStamp`], champ `LibraryEntry::source_stamp`), et
//! [`check`] compare la source actuelle à cette empreinte.
//!
//! - `.plain.pk` et `.zip` : SHA-256 des octets du fichier. Pour un `.plain.pk`, la copie de la
//!   bibliothèque est identique octet pour octet : l'empreinte se calcule aussi sur la copie.
//! - Dossier `.plain` : SHA-256 d'un **manifeste** (nom relatif, taille et SHA-256 du contenu de
//!   chaque fichier, triés par nom), calculable à l'identique sur le dossier et sur le
//!   `.plain.pk` qui en a été empaqueté ([`pk_manifest`]).
//!
//! **Pré-filtre** (`SourceStamp::quick`) : taille et date de modification d'un fichier ; pour un
//! dossier, empreinte de la liste (nom, taille, date) de ses fichiers, sans les lire. Égal à
//! celui de l'installation : source inchangée, rien n'est relu. Différent : l'empreinte du
//! contenu est recalculée (mémorisée en mémoire par chemin et pré-filtre, pour ne pas relire la
//! source à chaque vérification), puis comparée.
//!
//! **Migration** (histoire installée avant #288, sans empreinte : jamais de faux positif
//! bloquant, l'état ne fait que proposer « Mettre à jour ») : un `.plain.pk` de taille
//! différente de la copie est « modifié » ; une source dont aucun fichier n'est plus récent que
//! l'installation est « inchangée » ; sinon les empreintes de la source et de la copie sont
//! calculées et comparées (`.plain.pk`, `.plain`). Un `.zip` converti n'est pas comparable à sa
//! copie : modifié après l'installation, il est « modifié ».
//!
//! Lecture seule : rien n'est écrit ici. La mise à jour est une réinstallation depuis la source
//! par le chemin d'installation habituel (`install_start`), qui ne touche pas aux sauvegardes
//! (clé `library:<uuid>`, voir `player::Workspace`).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;
use zip::ZipArchive;

use crate::catalog::blocking;
use crate::install::pack;
use crate::library::{Library, LibraryEntry};
use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::ConfigState;
use crate::story::StoryKind;
use crate::{AppError, AppResult};

/// Préfixe des empreintes de contenu.
const DIGEST_PREFIX: &str = "sha256:";
/// Empreintes gardées en mémoire au plus (vidées au-delà).
const MAX_MEMO: usize = 512;
/// Entrées maximum d'un `.plain.pk` lu pour son manifeste (comme l'empaquetage).
const MAX_PK_ENTRIES: usize = 100_000;
/// Taille des blocs lus pour le hachage.
const CHUNK: usize = 1024 * 1024;

/// Empreinte de la source locale au moment de l'installation (miroir de `SourceStamp` dans
/// `src/api/library.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceStamp {
    /// Pré-filtre : `file:<taille>:<date en ns>`, ou `dir:<sha256 de la liste>` pour un `.plain`.
    pub quick: String,
    /// `sha256:<hex>` du contenu installé (octets du fichier, ou manifeste d'un `.plain`).
    pub sha256: String,
}

/// État de la source d'une histoire installée (miroir de `SourceState`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceState {
    /// Source identique à la copie de la bibliothèque.
    Unchanged,
    /// Source modifiée depuis l'installation : « Mettre à jour » est proposé.
    Changed,
    /// Source disparue : la copie de la bibliothèque est conservée telle quelle.
    Missing,
    /// Source non vérifiable (source dossier retirée, chemin hors du dossier, lien refusé,
    /// lecture impossible) : rien n'est proposé.
    Unknown,
}

/// Résultat de `library_source_status` pour une histoire (miroir de `SourceCheck`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceCheck {
    pub uuid: String,
    pub state: SourceState,
}

/// Source telle qu'observée sur disque, sans lire son contenu.
struct Observed {
    quick: String,
    /// Taille du fichier (cumulée pour un dossier).
    size: u64,
    /// Date de modification la plus récente (secondes Unix).
    newest: u64,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn modified(meta: &std::fs::Metadata) -> (u64, u128) {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or((0, 0), |d| (d.as_secs(), d.as_nanos()))
}

/// Pré-filtre, taille et date d'une source. Un dossier `.plain` est parcouru comme pour
/// l'empaquetage (mêmes fichiers, liens refusés) ; un fichier doit être un fichier ordinaire.
fn observe(path: &Path, kind: StoryKind) -> AppResult<Observed> {
    match kind {
        StoryKind::FlamPlain => {
            let mut files = pack::plain_dir_files(path)?;
            files.sort_by(|a, b| a.0.cmp(&b.0));
            let mut hasher = Sha256::new();
            let (mut size, mut newest) = (0u64, 0u64);
            for (name, file) in &files {
                let meta = std::fs::symlink_metadata(file)?;
                let (secs, nanos) = modified(&meta);
                hasher.update((name.len() as u64).to_le_bytes());
                hasher.update(name.as_bytes());
                hasher.update(meta.len().to_le_bytes());
                hasher.update(nanos.to_le_bytes());
                size = size.saturating_add(meta.len());
                newest = newest.max(secs);
            }
            Ok(Observed {
                quick: format!("dir:{}", hex(&hasher.finalize())),
                size,
                newest,
            })
        }
        StoryKind::FlamPk | StoryKind::TelmiZip => {
            let meta = std::fs::symlink_metadata(path)?;
            if !meta.is_file() {
                return Err(AppError::Validation(format!(
                    "source qui n'est pas un fichier ordinaire : {}",
                    path.display()
                )));
            }
            let (secs, nanos) = modified(&meta);
            Ok(Observed {
                quick: format!("file:{}:{nanos}", meta.len()),
                size: meta.len(),
                newest: secs,
            })
        }
    }
}

/// Taille lue et SHA-256 d'un flux.
fn hash_stream(reader: &mut dyn Read) -> io::Result<(u64, [u8; 32])> {
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        hasher.update(&buf[..n]);
    }
    Ok((total, hasher.finalize().into()))
}

/// `sha256:<hex>` des octets d'un fichier.
fn file_digest(path: &Path) -> AppResult<String> {
    let (_, digest) = hash_stream(&mut File::open(path)?)?;
    Ok(format!("{DIGEST_PREFIX}{}", hex(&digest)))
}

/// Empreinte d'un manifeste `(nom, taille, sha256)`, indépendante de l'ordre de lecture.
fn manifest(mut items: Vec<(String, u64, [u8; 32])>) -> String {
    items.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (name, size, digest) in &items {
        hasher.update((name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(size.to_le_bytes());
        hasher.update(digest);
    }
    format!("{DIGEST_PREFIX}{}", hex(&hasher.finalize()))
}

/// Manifeste d'un dossier `.plain` (fichiers ouverts sans suivre de lien).
fn dir_manifest(dir: &Path) -> AppResult<String> {
    let mut items = Vec::new();
    for (name, path) in pack::plain_dir_files(dir)? {
        let (size, digest) = hash_stream(&mut pack::open_no_follow(&path)?)?;
        items.push((name, size, digest));
    }
    Ok(manifest(items))
}

/// Manifeste d'un `.plain.pk` (fichiers du zip, dossiers ignorés) : égal à celui du dossier
/// `.plain` dont il a été empaqueté.
pub fn pk_manifest(pk: &Path) -> AppResult<String> {
    let invalid = |e: zip::result::ZipError| {
        AppError::Validation(format!("archive {} illisible : {e}", pk.display()))
    };
    let mut zip = ZipArchive::new(BufReader::new(File::open(pk)?)).map_err(invalid)?;
    if zip.len() > MAX_PK_ENTRIES {
        return Err(AppError::Validation(format!(
            "archive {} : trop d'entrées",
            pk.display()
        )));
    }
    let mut items = Vec::with_capacity(zip.len());
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).map_err(invalid)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let (size, digest) = hash_stream(&mut entry)?;
        items.push((name, size, digest));
    }
    Ok(manifest(items))
}

/// Empreinte du contenu de la source.
fn source_digest(path: &Path, kind: StoryKind) -> AppResult<String> {
    match kind {
        StoryKind::FlamPlain => dir_manifest(path),
        StoryKind::FlamPk | StoryKind::TelmiZip => file_digest(path),
    }
}

/// Empreintes déjà calculées : (chemin, pré-filtre) -> empreinte.
static MEMO: Mutex<Option<HashMap<(PathBuf, String), String>>> = Mutex::new(None);

/// Empreinte de `path` dans l'état `quick`, calculée par `compute` si elle n'est pas connue.
fn memoized(
    path: &Path,
    quick: &str,
    compute: impl FnOnce() -> AppResult<String>,
) -> AppResult<String> {
    let key = (path.to_path_buf(), quick.to_string());
    if let Some(found) = MEMO
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .and_then(|memo| memo.get(&key))
    {
        return Ok(found.clone());
    }
    let digest = compute()?;
    let mut memo = MEMO.lock().unwrap_or_else(PoisonError::into_inner);
    let memo = memo.get_or_insert_with(HashMap::new);
    if memo.len() >= MAX_MEMO {
        memo.clear();
    }
    memo.insert(key, digest.clone());
    Ok(digest)
}

/// Pré-filtre d'une source locale, relevé **avant** sa copie : si elle change pendant la copie,
/// la vérification suivante voit un pré-filtre différent et recalcule l'empreinte.
pub fn quick_of(path: &Path, kind: StoryKind) -> Option<String> {
    observe(path, kind).ok().map(|o| o.quick)
}

/// Empreinte de ce qui a été installé depuis une source `.plain.pk` ou `.plain` : calculée sur
/// le `.plain.pk` préparé `staged` (la copie elle-même), avec le pré-filtre `quick` relevé avant
/// la copie. `None` (avertissement) si elle ne peut pas être calculée : l'installation n'échoue
/// jamais pour une empreinte, l'entrée est alors traitée comme une entrée antérieure à #288.
pub fn stamp_copy(quick: Option<String>, kind: StoryKind, staged: &Path) -> Option<SourceStamp> {
    let quick = quick?;
    let sha256 = match kind {
        StoryKind::FlamPk => file_digest(staged),
        StoryKind::FlamPlain => pk_manifest(staged),
        StoryKind::TelmiZip => return None,
    };
    match sha256 {
        Ok(sha256) => Some(SourceStamp { quick, sha256 }),
        Err(err) => {
            tracing::warn!("empreinte de {} non calculée : {err}", staged.display());
            None
        }
    }
}

/// Empreinte d'une archive `.zip` locale, calculée sur la source **avant** sa conversion.
pub fn stamp_source(path: &Path, kind: StoryKind) -> Option<SourceStamp> {
    let stamp = observe(path, kind).and_then(|observed| {
        Ok(SourceStamp {
            sha256: source_digest(path, kind)?,
            quick: observed.quick,
        })
    });
    match stamp {
        Ok(stamp) => Some(stamp),
        Err(err) => {
            tracing::warn!("empreinte de {} non calculée : {err}", path.display());
            None
        }
    }
}

/// Vrai si `path` est un chemin absolu, sans `..`, situé sous `root`.
fn is_under(path: &Path, root: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|c| matches!(c, Component::ParentDir))
        && path.starts_with(root)
        && path != root
}

/// État de la source de `entry` : `folders` associe l'identifiant de chaque source dossier à
/// son chemin. Une histoire dont la source n'est pas (ou plus) un dossier connu, ou dont le
/// chemin enregistré sort de ce dossier, est `Unknown` : rien n'est lu.
pub fn check(
    entry: &LibraryEntry,
    library: &Library,
    folders: &HashMap<String, PathBuf>,
) -> SourceState {
    let source = &entry.source;
    let Some(root) = folders.get(&source.source_id) else {
        return SourceState::Unknown;
    };
    let path = PathBuf::from(&source.download_url);
    if !is_under(&path, root) {
        return SourceState::Unknown;
    }
    let observed = match observe(&path, source.kind) {
        Ok(observed) => observed,
        Err(AppError::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
            return SourceState::Missing
        }
        Err(err) => {
            tracing::debug!("source de {} non vérifiable : {err}", entry.uuid);
            return SourceState::Unknown;
        }
    };
    let state = match &entry.source_stamp {
        Some(stamp) => compare(&path, source.kind, &observed, stamp),
        None => migrate(entry, library, &path, &observed),
    };
    state.unwrap_or_else(|err| {
        tracing::debug!("source de {} non vérifiable : {err}", entry.uuid);
        SourceState::Unknown
    })
}

/// Comparaison avec l'empreinte de l'installation.
fn compare(
    path: &Path,
    kind: StoryKind,
    observed: &Observed,
    stamp: &SourceStamp,
) -> AppResult<SourceState> {
    if observed.quick == stamp.quick {
        return Ok(SourceState::Unchanged);
    }
    let digest = memoized(path, &observed.quick, || source_digest(path, kind))?;
    Ok(if digest == stamp.sha256 {
        SourceState::Unchanged
    } else {
        SourceState::Changed
    })
}

/// Entrée sans empreinte (installée avant #288) : voir la documentation du module.
fn migrate(
    entry: &LibraryEntry,
    library: &Library,
    path: &Path,
    observed: &Observed,
) -> AppResult<SourceState> {
    let kind = entry.source.kind;
    // Copie octet pour octet : une autre taille est une autre archive.
    if kind == StoryKind::FlamPk && observed.size != entry.size {
        return Ok(SourceState::Changed);
    }
    if observed.newest < entry.installed_at {
        return Ok(SourceState::Unchanged);
    }
    let copy = library.checked(&entry.file)?;
    let copy_meta = std::fs::metadata(&copy)?;
    let copy_quick = format!("copy:{}:{}", copy_meta.len(), modified(&copy_meta).1);
    let theirs = match kind {
        StoryKind::FlamPk => memoized(&copy, &copy_quick, || file_digest(&copy))?,
        StoryKind::FlamPlain => memoized(&copy, &copy_quick, || pk_manifest(&copy))?,
        // Archive convertie : pas comparable à sa copie ; modifiée après l'installation.
        StoryKind::TelmiZip => return Ok(SourceState::Changed),
    };
    let ours = memoized(path, &observed.quick, || source_digest(path, kind))?;
    Ok(if ours == theirs {
        SourceState::Unchanged
    } else {
        SourceState::Changed
    })
}

/// État de la source de chaque histoire installée depuis une source dossier : `cache_root` est
/// le dossier de cache, `sources` la liste des sources. Bloquant.
pub fn statuses(cache_root: &Path, sources: Vec<Source>) -> AppResult<Vec<SourceCheck>> {
    let folders: HashMap<String, PathBuf> = sources
        .into_iter()
        .filter_map(|source| match source {
            Source::Folder { id, path, .. } => Some((id, PathBuf::from(path))),
            _ => None,
        })
        .collect();
    let library = Library::new(cache_root);
    Ok(library
        .list()?
        .iter()
        .filter(|entry| folders.contains_key(&entry.source.source_id))
        .map(|entry| SourceCheck {
            uuid: entry.uuid.clone(),
            state: check(entry, &library, &folders),
        })
        .collect())
}

/// État de la source des histoires installées depuis une source dossier : `changed` propose
/// « Mettre à jour » (réinstallation depuis la source, sans remplacement silencieux). Lecture
/// seule.
#[tauri::command]
pub async fn library_source_status(config: State<'_, ConfigState>) -> AppResult<Vec<SourceCheck>> {
    let (cache_root, sources) = {
        let _guard = config.guard();
        let cache_root =
            SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?;
        (cache_root, SourcesStore::new(&config.dir).list()?)
    };
    blocking(move || statuses(&cache_root, sources)).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tests::story;
    use crate::story::StoryEntry;
    use std::io::Write;
    use std::time::{Duration, SystemTime};

    const UUID: &str = "a701e27e";
    const FOLDER: &str = "dossier";

    /// `.plain.pk` minimal dont la vignette vaut `thumb`.
    fn write_pk(path: &Path, thumb: &[u8]) {
        let mut zip = zip::ZipWriter::new(File::create(path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, data) in [
            ("info.plain", &b"Capi\n"[..]),
            ("uuid.bin", &[7u8; 16][..]),
            ("main.lua", &b"-- x"[..]),
            ("img/thumbnail.lif", thumb),
        ] {
            zip.start_file(name, options).unwrap();
            zip.write_all(data).unwrap();
        }
        zip.finish().unwrap();
    }

    fn write_plain(dir: &Path, thumb: &[u8]) {
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("info.plain"), "Capi\n").unwrap();
        std::fs::write(dir.join("uuid.bin"), [7u8; 16]).unwrap();
        std::fs::write(dir.join("main.lua"), "-- x").unwrap();
        std::fs::write(dir.join("img/thumbnail.lif"), thumb).unwrap();
    }

    fn set_mtime(path: &Path, secs: u64) {
        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(UNIX_EPOCH + Duration::from_secs(secs))
            .unwrap();
    }

    fn mtime(path: &Path) -> u64 {
        std::fs::metadata(path)
            .unwrap()
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    struct Env {
        _tmp: tempfile::TempDir,
        src: PathBuf,
        lib: Library,
        folders: HashMap<String, PathBuf>,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        let lib = Library::new(&tmp.path().join("cache"));
        let folders = HashMap::from([(FOLDER.to_string(), src.clone())]);
        Env {
            _tmp: tmp,
            src,
            lib,
            folders,
        }
    }

    fn entry_for(path: &Path, kind: StoryKind) -> StoryEntry {
        let mut entry = story(UUID, Some(0));
        entry.source_id = FOLDER.into();
        entry.entry_id = format!("{FOLDER}:x");
        entry.download_url = path.to_string_lossy().into_owned();
        entry.kind = kind;
        entry
    }

    /// Installation comme le pipeline : pré-filtre avant la copie, empreinte sur la copie.
    /// `stamped` faux : installation antérieure à #288 (sans empreinte).
    fn install(env: &Env, path: &Path, kind: StoryKind, now: u64, stamped: bool) {
        let staged = env.lib.staging_dir().unwrap().join("t.plain.pk");
        let quick = quick_of(path, kind);
        match kind {
            StoryKind::FlamPlain => pack::pack_plain_dir(path, &staged).unwrap(),
            _ => {
                std::fs::copy(path, &staged).unwrap();
            }
        }
        let stamp = if stamped {
            stamp_copy(quick, kind, &staged)
        } else {
            None
        };
        assert_eq!(stamp.is_some(), stamped);
        env.lib
            .install_thumbed(&staged, &entry_for(path, kind), None, now, None, stamp)
            .unwrap();
    }

    fn state(env: &Env) -> SourceState {
        let entry = env.lib.list().unwrap().into_iter().next().unwrap();
        check(&entry, &env.lib, &env.folders)
    }

    #[test]
    fn pk_modifie_a_version_egale_puis_mise_a_jour() {
        let env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"vignette 128x96");
        install(&env, &pk, StoryKind::FlamPk, 10, true);
        assert_eq!(state(&env), SourceState::Unchanged);
        // Reconstruite : même uuid, même version, nouvelle vignette.
        write_pk(&pk, b"vignette 104x136 !");
        assert_eq!(state(&env), SourceState::Changed);
        assert_eq!(state(&env), SourceState::Changed, "empreinte mémorisée");
        // « Mettre à jour » : réinstallation depuis la source.
        install(&env, &pk, StoryKind::FlamPk, 20, true);
        assert_eq!(state(&env), SourceState::Unchanged);
        let entries = env.lib.list().unwrap();
        assert_eq!(entries.len(), 1);
        let copy = env.lib.checked(&entries[0].file).unwrap();
        assert_eq!(std::fs::read(copy).unwrap(), std::fs::read(&pk).unwrap());
    }

    #[test]
    fn source_identique_rien_a_proposer() {
        let env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"v");
        install(&env, &pk, StoryKind::FlamPk, 10, true);
        // Réécrite à l'identique : le pré-filtre diffère, l'empreinte non.
        let bytes = std::fs::read(&pk).unwrap();
        std::fs::write(&pk, &bytes).unwrap();
        set_mtime(&pk, 1_900_000_000);
        assert_eq!(state(&env), SourceState::Unchanged);
    }

    #[test]
    fn source_supprimee_copie_conservee() {
        let env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"v");
        install(&env, &pk, StoryKind::FlamPk, 10, true);
        std::fs::remove_file(&pk).unwrap();
        assert_eq!(state(&env), SourceState::Missing);
        let entries = env.lib.list().unwrap();
        assert_eq!(entries.len(), 1, "copie conservée");
        assert!(env.lib.checked(&entries[0].file).unwrap().is_file());
        // Dossier `.plain` disparu : de même.
        let plain = env.src.join("capi.plain");
        write_plain(&plain, b"v");
        install(&env, &plain, StoryKind::FlamPlain, 20, true);
        std::fs::remove_dir_all(&plain).unwrap();
        assert_eq!(state(&env), SourceState::Missing);
    }

    #[test]
    fn dossier_plain_manifeste_et_modification_interne() {
        let env = env();
        let plain = env.src.join("capi.plain");
        write_plain(&plain, b"ancienne");
        let pk = env.src.join("paquet.plain.pk");
        pack::pack_plain_dir(&plain, &pk).unwrap();
        assert_eq!(dir_manifest(&plain).unwrap(), pk_manifest(&pk).unwrap());
        std::fs::remove_file(&pk).unwrap();

        install(&env, &plain, StoryKind::FlamPlain, 10, true);
        assert_eq!(state(&env), SourceState::Unchanged);
        // Fichier interne réécrit à l'identique : inchangée.
        std::fs::write(plain.join("img/thumbnail.lif"), b"ancienne").unwrap();
        set_mtime(&plain.join("img/thumbnail.lif"), 1_900_000_000);
        assert_eq!(state(&env), SourceState::Unchanged);
        // Fichier interne modifié (la date du dossier ne change pas forcément) : modifiée.
        std::fs::write(plain.join("img/thumbnail.lif"), b"nouvelle").unwrap();
        assert_eq!(state(&env), SourceState::Changed);
        // Fichier ajouté : modifiée aussi.
        install(&env, &plain, StoryKind::FlamPlain, 20, true);
        assert_eq!(state(&env), SourceState::Unchanged);
        std::fs::write(plain.join("bonus.txt"), b"x").unwrap();
        assert_eq!(state(&env), SourceState::Changed);
    }

    #[test]
    fn migration_entree_sans_empreinte() {
        let env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"vignette A");
        let built = mtime(&pk);
        // Installée après la construction, sans empreinte (version antérieure).
        install(&env, &pk, StoryKind::FlamPk, built + 100, false);
        assert!(env.lib.list().unwrap()[0].source_stamp.is_none());
        assert_eq!(state(&env), SourceState::Unchanged, "pas plus récente");
        // Réécrite à l'identique après l'installation : empreintes comparées, inchangée.
        let bytes = std::fs::read(&pk).unwrap();
        std::fs::write(&pk, &bytes).unwrap();
        set_mtime(&pk, built + 200);
        assert_eq!(state(&env), SourceState::Unchanged);
        // Même taille, contenu différent, plus récente : modifiée.
        write_pk(&pk, b"vignette B");
        set_mtime(&pk, built + 300);
        assert_eq!(state(&env), SourceState::Changed);
        // Autre taille : modifiée sans rien relire, même plus ancienne.
        write_pk(&pk, b"vignette beaucoup plus longue");
        set_mtime(&pk, built);
        assert_eq!(state(&env), SourceState::Changed);
    }

    #[test]
    fn migration_dossier_plain_et_zip() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let first = env();
        let plain = first.src.join("capi.plain");
        write_plain(&plain, b"A");
        install(&first, &plain, StoryKind::FlamPlain, now + 100, false);
        assert_eq!(state(&first), SourceState::Unchanged);
        // Fichier réécrit à l'identique après l'installation : manifestes égaux.
        std::fs::write(plain.join("main.lua"), "-- x").unwrap();
        set_mtime(&plain.join("main.lua"), now + 200);
        assert_eq!(state(&first), SourceState::Unchanged);
        std::fs::write(plain.join("main.lua"), "-- y").unwrap();
        set_mtime(&plain.join("main.lua"), now + 300);
        assert_eq!(state(&first), SourceState::Changed);

        // Archive Telmi convertie : non comparable ; plus récente que l'installation, modifiée.
        let second = env();
        let zip = second.src.join("capi.zip");
        std::fs::write(&zip, b"zip").unwrap();
        install(&second, &zip, StoryKind::TelmiZip, now + 100, false);
        set_mtime(&zip, now);
        assert_eq!(state(&second), SourceState::Unchanged);
        set_mtime(&zip, now + 200);
        assert_eq!(state(&second), SourceState::Changed);
    }

    #[test]
    fn zip_local_empreinte_de_la_source() {
        let env = env();
        let zip = env.src.join("capi.zip");
        std::fs::write(&zip, b"zip v1").unwrap();
        let stamp = stamp_source(&zip, StoryKind::TelmiZip).unwrap();
        let staged = env.lib.staging_dir().unwrap().join("t.plain.pk");
        std::fs::write(&staged, b"converti").unwrap();
        env.lib
            .install_thumbed(
                &staged,
                &entry_for(&zip, StoryKind::TelmiZip),
                None,
                10,
                None,
                Some(stamp),
            )
            .unwrap();
        assert_eq!(state(&env), SourceState::Unchanged);
        std::fs::write(&zip, b"zip v2").unwrap();
        assert_eq!(state(&env), SourceState::Changed);
    }

    #[test]
    fn source_non_verifiable() {
        let mut env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"v");
        install(&env, &pk, StoryKind::FlamPk, 10, true);
        write_pk(&pk, b"modifiee");
        assert_eq!(state(&env), SourceState::Changed);
        let entry = env.lib.list().unwrap().remove(0);
        // Chemin hors du dossier de la source, ou avec `..` : rien n'est lu.
        let mut outside = entry.clone();
        outside.source.download_url = env.src.join("..").join("x.plain.pk").display().to_string();
        assert_eq!(
            check(&outside, &env.lib, &env.folders),
            SourceState::Unknown
        );
        let elsewhere = HashMap::from([(FOLDER.to_string(), env.src.join("autre"))]);
        assert_eq!(check(&entry, &env.lib, &elsewhere), SourceState::Unknown);
        // Source dossier retirée : inconnue.
        env.folders.clear();
        assert_eq!(state(&env), SourceState::Unknown);
        // Fichier remplacé par un dossier : inconnue, pas « modifiée ».
        env.folders = HashMap::from([(FOLDER.to_string(), env.src.clone())]);
        std::fs::remove_file(&pk).unwrap();
        std::fs::create_dir(&pk).unwrap();
        assert_eq!(state(&env), SourceState::Unknown);
    }

    #[test]
    fn statuses_ne_liste_que_les_sources_dossier() {
        let env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"v");
        install(&env, &pk, StoryKind::FlamPk, 10, true);
        // Histoire d'un catalogue : non listée.
        let staged = env.lib.staging_dir().unwrap().join("c.plain.pk");
        std::fs::write(&staged, b"catalogue").unwrap();
        env.lib
            .install(&staged, &story("cat", Some(1)), None, 11)
            .unwrap();
        write_pk(&pk, b"modifiee");
        let cache_root = env.lib.root().parent().unwrap().to_path_buf();
        let sources = vec![
            Source::Folder {
                id: FOLDER.into(),
                name: "Dossier".into(),
                path: env.src.display().to_string(),
            },
            Source::Catalog {
                id: "src".into(),
                name: "Catalogue".into(),
                url: "https://h.example.test/c.json".into(),
            },
        ];
        assert_eq!(
            statuses(&cache_root, sources).unwrap(),
            vec![SourceCheck {
                uuid: UUID.into(),
                state: SourceState::Changed
            }]
        );
    }

    #[test]
    fn index_anterieur_sans_empreinte_relu() {
        let env = env();
        let pk = env.src.join("capi.plain.pk");
        write_pk(&pk, b"v");
        install(&env, &pk, StoryKind::FlamPk, 10, true);
        // Index écrit par une version antérieure : champ `source_stamp` absent.
        let index = env.lib.root().join("library.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        json["stories"][0]
            .as_object_mut()
            .unwrap()
            .remove("source_stamp");
        std::fs::write(&index, serde_json::to_vec(&json).unwrap()).unwrap();
        let entries = env.lib.list().unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].source_stamp.is_none());
    }
}
