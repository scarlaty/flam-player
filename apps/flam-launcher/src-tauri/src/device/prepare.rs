//! Préparation hors appareil d'une histoire pour la Flam (firmware 2.x, #228).
//!
//! À partir d'un `library/<uuid>-<version>.plain.pk`, construit dans un dossier de travail du
//! launcher l'arborescence `str/<uuid>/` que la Flam attend ; le transfert sur l'appareil est
//! l'affaire de #229. **Rien n'est écrit sur l'appareil ici.**
//!
//! Transformation de chaque entrée de l'archive :
//!
//! | Entrée du `.plain.pk`       | Dans `str/<uuid>/`        | Traitement          |
//! |-----------------------------|---------------------------|---------------------|
//! | `uuid.bin`                  | (aucune)                  | donne le nom du dossier |
//! | `_metadata.json`, `_thumbnail.png` | (aucune)           | ignorées            |
//! | `info.plain`, `*.plain`     | `info`, nom sans `.plain` | chiffré             |
//! | `main.lua`, `script/*.lua`  | `main.lsf`, `script/*.lsf`| chiffré             |
//! | tout le reste (`version`, `img/…`, `sounds/…`) | même nom | copié tel quel |
//! | (aucune)                    | `key`                     | déduit de l'appareil |
//!
//! Invariants :
//! - le dossier final `<travail>/str/<uuid>/` est soit absent, soit complet : tout est écrit
//!   dans un dossier de chantier `<travail>/.partiel-<uuid>-<aléa>`, renommé à la fin ;
//! - en cas d'erreur ou d'annulation, le chantier est supprimé (au mieux) ; un chantier laissé
//!   par un arrêt brutal est supprimé par [`cleanup_stale`] ;
//! - l'archive source n'est jamais modifiée ; tailles et nombre d'entrées sont bornés
//!   ([`Limits`]) d'après les tailles **réellement lues**, pas seulement déclarées ;
//! - les noms d'entrées sont contrôlés (pas de remontée `..`, pas de chemin absolu, pas de
//!   caractère interdit sous Windows, pas de doublon à la casse près : FAT ignore la casse) ;
//! - aucune clé n'est journalisée ni écrite ailleurs que dans les fichiers chiffrés et `key`.

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use zip::ZipArchive;

use super::crypto::DeviceKeys;

/// Dossier des histoires préparées, sous le dossier de travail (même nom que sur la Flam).
pub const STORIES_DIR: &str = "str";
/// Préfixe des dossiers de chantier, sous le dossier de travail.
pub const STAGING_PREFIX: &str = ".partiel-";
/// Nom du fichier d'autorisation d'une histoire.
pub const KEY_FILE: &str = "key";
/// Entrées obligatoires du `.plain.pk`.
pub const REQUIRED: [&str; 4] = ["uuid.bin", "info.plain", "main.lua", "version"];
/// Entrées propres au `.plain.pk`, jamais copiées sur la Flam.
pub const EXCLUDED: [&str; 3] = ["uuid.bin", "_metadata.json", "_thumbnail.png"];

const UUID_ENTRY: &str = "uuid.bin";
const COPY_CHUNK: usize = 64 * 1024;
/// Longueur maximale d'un nom de fichier ou de dossier sur FAT (unités UTF-16).
const MAX_COMPONENT_UTF16: usize = 255;

/// Bornes de la préparation.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Nombre maximal d'entrées de l'archive.
    pub max_entries: usize,
    /// Taille cumulée maximale des fichiers produits.
    pub max_total_bytes: u64,
    /// Taille maximale d'un fichier à chiffrer (lu entièrement en mémoire).
    pub max_encrypted_bytes: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_total_bytes: 4 * 1024 * 1024 * 1024,
            max_encrypted_bytes: 16 * 1024 * 1024,
        }
    }
}

/// Erreur de préparation. Les messages ne contiennent jamais de clé.
#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    /// Erreur d'entrée-sortie (disque plein compris).
    #[error("préparation de l'histoire : {0}")]
    Io(#[from] io::Error),
    /// Archive illisible, tronquée ou incomplète.
    #[error("archive .plain.pk invalide : {0}")]
    Invalid(String),
    /// Une borne de [`Limits`] est dépassée.
    #[error("archive .plain.pk trop volumineuse : {0}")]
    TooLarge(String),
    /// Annulation demandée.
    #[error("préparation annulée")]
    Cancelled,
}

/// Histoire préparée, prête à copier sur la Flam.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedStory {
    /// UUID de l'histoire, en minuscules avec tirets (nom du dossier sur la Flam).
    pub uuid: String,
    /// Dossier `<travail>/str/<uuid>`.
    pub dir: PathBuf,
    /// Fichiers produits (chemins relatifs en `/`, triés).
    pub files: Vec<String>,
    /// Taille cumulée des fichiers produits.
    pub total_bytes: u64,
}

/// Opérations d'écriture, abstraites pour les tests d'injection de panne.
pub(crate) trait Sink {
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + '_>>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    fn remove_dir_all(&self, path: &Path) -> io::Result<()>;
}

pub(crate) struct RealSink;

impl Sink for RealSink {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::create_dir_all(path)
    }
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + '_>> {
        Ok(Box::new(File::create_new(path)?))
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        // Antivirus ou indexeur sous Windows : quelques essais sur un refus transitoire.
        crate::storage::retry_transient(|| std::fs::rename(from, to), std::thread::sleep)
    }
    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_dir_all(path)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    Copy,
    Encrypt,
}

struct Planned {
    index: usize,
    target: String,
    action: Action,
}

/// Prépare l'histoire `plain_pk` pour l'appareil `keys` dans `work_dir` (créé au besoin).
/// Une préparation précédente de la même histoire est remplacée. Bloquant : à appeler hors du
/// runtime asynchrone. `cancel` est consulté entre chaque bloc écrit.
///
/// Un seul appel à la fois par histoire et par dossier de travail (sérialisé par l'appelant).
pub fn prepare_story(
    plain_pk: &Path,
    keys: &DeviceKeys,
    work_dir: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<PreparedStory, PrepareError> {
    prepare_with(&RealSink, plain_pk, keys, work_dir, limits, cancel)
}

/// Supprime les dossiers de chantier laissés par un arrêt brutal ; renvoie leur nombre.
pub fn cleanup_stale(work_dir: &Path) -> io::Result<usize> {
    let entries = match std::fs::read_dir(work_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(STAGING_PREFIX) && entry.file_type()?.is_dir() {
            std::fs::remove_dir_all(entry.path())?;
            removed += 1;
        }
    }
    Ok(removed)
}

fn invalid(msg: impl Into<String>) -> PrepareError {
    PrepareError::Invalid(msg.into())
}

fn check_cancel(cancel: &AtomicBool) -> Result<(), PrepareError> {
    if cancel.load(Ordering::Relaxed) {
        Err(PrepareError::Cancelled)
    } else {
        Ok(())
    }
}

/// Noms réservés de Windows (insensibles à la casse, extension comprise).
fn is_reserved_windows_name(component: &str) -> bool {
    let stem = component
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit())
}

/// Chemin relatif sûr (composants séparés par `/`) d'une entrée, ou erreur.
/// Majuscules caractère par caractère, comme la table de FAT : une correspondance Unicode à
/// plusieurs caractères (`ß` → `SS`, `ﬁ` → `FI`) laisse le caractère tel quel.
fn fat_upper(name: &str) -> String {
    name.chars()
        .map(|c| {
            let mut upper = c.to_uppercase();
            match (upper.next(), upper.next()) {
                (Some(u), None) => u,
                _ => c,
            }
        })
        .collect()
}

fn safe_relative(raw_name: &str, enclosed: Option<PathBuf>) -> Result<String, PrepareError> {
    let bad = || invalid(format!("nom d'entrée refusé : {raw_name:?}"));
    let path = enclosed.ok_or_else(bad)?;
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return Err(bad());
        };
        let part = part.to_str().ok_or_else(bad)?;
        let forbidden = |c: char| c.is_control() || "<>:\"|?*\\".contains(c);
        if part.is_empty()
            || part.chars().any(forbidden)
            || part.ends_with('.')
            || part.ends_with(' ')
            || is_reserved_windows_name(part)
        {
            return Err(bad());
        }
        parts.push(part.to_string());
    }
    if parts.is_empty() {
        return Err(bad());
    }
    Ok(parts.join("/"))
}

/// Retire un suffixe ASCII sans tenir compte de la casse (`.LUA` comme `.lua`).
fn strip_suffix_ci<'a>(text: &'a str, suffix: &str) -> Option<&'a str> {
    let cut = text.len().checked_sub(suffix.len())?;
    (text.is_char_boundary(cut) && text[cut..].eq_ignore_ascii_case(suffix)).then(|| &text[..cut])
}

/// Nom sur la Flam et traitement d'une entrée (`None` : entrée ignorée).
fn plan_entry(relative: &str) -> Result<Option<(String, Action)>, PrepareError> {
    if EXCLUDED.iter().any(|e| e.eq_ignore_ascii_case(relative)) {
        return Ok(None);
    }
    let (target, action) = if let Some(stem) = strip_suffix_ci(relative, ".plain") {
        (stem.to_string(), Action::Encrypt)
    } else if let Some(stem) = strip_suffix_ci(relative, ".lua") {
        (format!("{stem}.lsf"), Action::Encrypt)
    } else {
        (relative.to_string(), Action::Copy)
    };
    if target.is_empty() || target.ends_with('/') || target.eq_ignore_ascii_case(KEY_FILE) {
        return Err(invalid(format!("nom d'entrée refusé : {relative:?}")));
    }
    Ok(Some((target, action)))
}

fn new_staging_name(uuid: &str) -> String {
    format!("{STAGING_PREFIX}{uuid}-{}", uuid::Uuid::new_v4().simple())
}

pub(crate) fn prepare_with(
    sink: &dyn Sink,
    plain_pk: &Path,
    keys: &DeviceKeys,
    work_dir: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<PreparedStory, PrepareError> {
    check_cancel(cancel)?;
    let file = File::open(plain_pk)?;
    let mut zip =
        ZipArchive::new(BufReader::new(file)).map_err(|e| invalid(format!("zip : {e}")))?;
    if zip.len() > limits.max_entries {
        return Err(PrepareError::TooLarge(format!(
            "{} entrées (au plus {})",
            zip.len(),
            limits.max_entries
        )));
    }

    // 1. Plan : noms, doublons, entrées obligatoires, UUID. Rien n'est écrit.
    let mut planned = Vec::new();
    // Noms comparés en minuscules et en majuscules : la table de majuscules de FAT ne suit pas
    // exactement `to_lowercase` (`ſ` et `s`, `ı` et `i` ne se confondent qu'en majuscules).
    let mut seen_sources = HashSet::new();
    let mut seen_sources_upper = HashSet::new();
    let mut exact_names = HashSet::new();
    let mut seen_targets = HashSet::new();
    let mut seen_targets_upper = HashSet::new();
    let mut uuid_index = None;
    let mut declared = 0u64;
    for index in 0..zip.len() {
        let entry = zip
            .by_index_raw(index)
            .map_err(|e| invalid(format!("entrée {index} : {e}")))?;
        if entry.is_dir() {
            continue;
        }
        if entry.is_symlink() {
            return Err(invalid(format!(
                "lien symbolique refusé : {:?}",
                entry.name()
            )));
        }
        let relative = safe_relative(entry.name(), entry.enclosed_name())?;
        let new_lower = seen_sources.insert(relative.to_lowercase());
        let new_upper = seen_sources_upper.insert(fat_upper(&relative));
        if !(new_lower && new_upper) {
            return Err(invalid(format!("entrée en double : {relative:?}")));
        }
        exact_names.insert(relative.clone());
        if relative == UUID_ENTRY {
            uuid_index = Some(index);
        }
        if let Some((target, action)) = plan_entry(&relative)? {
            let new_lower = seen_targets.insert(target.to_lowercase());
            let new_upper = seen_targets_upper.insert(fat_upper(&target));
            if !(new_lower && new_upper) {
                return Err(invalid(format!("deux entrées donnent {target:?}")));
            }
            if target
                .split('/')
                .any(|c| c.encode_utf16().count() > MAX_COMPONENT_UTF16)
            {
                return Err(invalid(format!("nom trop long pour FAT : {target:?}")));
            }
            declared = declared.saturating_add(entry.size());
            planned.push(Planned {
                index,
                target,
                action,
            });
        }
    }
    // Un fichier et un dossier de même nom (`a` et `a/b`) : refusé dès le plan, avant toute
    // écriture, plutôt que par une erreur d'E/S en cours de chantier.
    for (targets, target) in [&seen_targets, &seen_targets_upper]
        .into_iter()
        .flat_map(|set| set.iter().map(move |t| (set, t)))
    {
        let mut at = 0;
        while let Some(i) = target[at..].find('/') {
            at += i;
            if targets.contains(&target[..at]) {
                return Err(invalid(format!(
                    "{:?} est à la fois un fichier et un dossier",
                    &target[..at]
                )));
            }
            at += 1;
        }
    }
    // Refus précoce d'après les tailles déclarées par l'annuaire de l'archive (avant
    // chiffrement). Les tailles réellement lues restent contrôlées pendant l'écriture.
    if declared > limits.max_total_bytes {
        return Err(PrepareError::TooLarge(format!(
            "plus de {} octets déclarés",
            limits.max_total_bytes
        )));
    }
    for required in REQUIRED {
        if !exact_names.contains(required) {
            return Err(invalid(format!("{required} absent")));
        }
    }
    let uuid = read_uuid(&mut zip, uuid_index.expect("uuid.bin présent"))?;

    // 2. Chantier, puis 3. renommage en dossier final.
    let staging = work_dir.join(new_staging_name(&uuid));
    let result = (|| {
        sink.create_dir_all(&staging)?;
        let total = write_entries(sink, &mut zip, &planned, keys, &staging, limits, cancel)?;
        check_cancel(cancel)?;
        let key = keys.key_file();
        write_all_to(sink, &staging.join(KEY_FILE), &key[..])?;
        let total = total + key.len() as u64;
        check_cancel(cancel)?;
        let stories = work_dir.join(STORIES_DIR);
        sink.create_dir_all(&stories)?;
        let dir = stories.join(&uuid);
        // Une préparation précédente est écartée par renommage (jamais supprimée en place) :
        // le dossier final reste complet, et elle est remise en place si le renommage échoue.
        let previous = if dir.exists() {
            let old = work_dir.join(new_staging_name(&format!("ancien-{uuid}")));
            sink.rename(&dir, &old)?;
            Some(old)
        } else {
            None
        };
        if let Err(e) = sink.rename(&staging, &dir) {
            if let Some(old) = &previous {
                if let Err(back) = sink.rename(old, &dir) {
                    tracing::warn!(ancien = %old.display(), "restauration impossible : {back}");
                }
            }
            return Err(e.into());
        }
        if let Some(old) = previous {
            if let Err(e) = sink.remove_dir_all(&old) {
                // Laissé à `cleanup_stale` (préfixe de chantier).
                tracing::warn!(ancien = %old.display(), "suppression différée : {e}");
            }
        }
        Ok::<_, PrepareError>((dir, total))
    })();
    let (dir, total_bytes) = match result {
        Ok(ok) => ok,
        Err(e) => {
            if staging.exists() {
                if let Err(clean) = std::fs::remove_dir_all(&staging) {
                    tracing::warn!(chantier = %staging.display(), "nettoyage impossible : {clean}");
                }
            }
            return Err(e);
        }
    };
    let mut files: Vec<String> = planned.iter().map(|p| p.target.clone()).collect();
    files.push(KEY_FILE.to_string());
    files.sort();
    tracing::info!(%uuid, fichiers = files.len(), octets = total_bytes, "histoire préparée pour la Flam");
    Ok(PreparedStory {
        uuid,
        dir,
        files,
        total_bytes,
    })
}

fn read_uuid<R: Read + io::Seek>(
    zip: &mut ZipArchive<R>,
    index: usize,
) -> Result<String, PrepareError> {
    let mut entry = zip
        .by_index(index)
        .map_err(|e| invalid(format!("uuid.bin : {e}")))?;
    let mut bytes = Vec::with_capacity(16);
    (&mut entry)
        .take(17)
        .read_to_end(&mut bytes)
        .map_err(|e| invalid(format!("uuid.bin : {e}")))?;
    let uuid = uuid::Uuid::from_slice(&bytes)
        .map_err(|_| invalid(format!("uuid.bin : {} octets au lieu de 16", bytes.len())))?;
    Ok(uuid.hyphenated().to_string())
}

fn write_all_to(sink: &dyn Sink, path: &Path, data: &[u8]) -> io::Result<()> {
    let mut out = sink.create(path)?;
    out.write_all(data)?;
    out.flush()
}

fn write_entries<R: Read + io::Seek>(
    sink: &dyn Sink,
    zip: &mut ZipArchive<R>,
    planned: &[Planned],
    keys: &DeviceKeys,
    staging: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<u64, PrepareError> {
    let mut total = 0u64;
    let too_large = |what: String| PrepareError::TooLarge(what);
    for p in planned {
        check_cancel(cancel)?;
        let target = staging.join(&p.target);
        if let Some(parent) = target.parent() {
            sink.create_dir_all(parent)?;
        }
        let mut entry = zip
            .by_index(p.index)
            .map_err(|e| invalid(format!("{} : {e}", p.target)))?;
        let read_err = |e: io::Error| invalid(format!("{} : {e}", p.target));
        match p.action {
            Action::Encrypt => {
                let max = limits.max_encrypted_bytes;
                let mut plain = Vec::new();
                (&mut entry)
                    .take(max.saturating_add(1))
                    .read_to_end(&mut plain)
                    .map_err(read_err)?;
                if plain.len() as u64 > max {
                    return Err(too_large(format!("{} dépasse {max} octets", p.target)));
                }
                let data = keys.encrypt(&plain);
                total += data.len() as u64;
                if total > limits.max_total_bytes {
                    return Err(too_large(format!(
                        "plus de {} octets",
                        limits.max_total_bytes
                    )));
                }
                write_all_to(sink, &target, &data)?;
            }
            Action::Copy => {
                let mut out = sink.create(&target)?;
                let mut buf = vec![0u8; COPY_CHUNK];
                loop {
                    check_cancel(cancel)?;
                    let n = entry.read(&mut buf).map_err(read_err)?;
                    if n == 0 {
                        break;
                    }
                    total += n as u64;
                    if total > limits.max_total_bytes {
                        return Err(too_large(format!(
                            "plus de {} octets",
                            limits.max_total_bytes
                        )));
                    }
                    out.write_all(&buf[..n])?;
                }
                out.flush()?;
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::crypto::decrypt_blocks;
    use crate::device::crypto::tests::{test_keys, TEST_IV, TEST_KEY, TEST_SERIAL};
    use std::cell::Cell;
    use std::sync::Arc;
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    const UUID_BYTES: [u8; 16] = [
        0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7, 0xe8,
        0xf9,
    ];
    const UUID_STR: &str = "0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9";
    const INFO: &[u8] = b"Titre\nSous-titre\n000000\nTitre\n";
    const MAIN: &[u8] = b"function setup() end -- 33 octets";

    fn write_pk(path: &Path, entries: &[(&str, &[u8])]) {
        let mut zip = ZipWriter::new(File::create(path).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries {
            if name.ends_with('/') {
                zip.add_directory(*name, options).unwrap();
            } else {
                zip.start_file(*name, options).unwrap();
                zip.write_all(data).unwrap();
            }
        }
        zip.finish().unwrap();
    }

    fn base_entries() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("uuid.bin", UUID_BYTES.to_vec()),
            ("info.plain", INFO.to_vec()),
            ("version", b"1".to_vec()),
            ("main.lua", MAIN.to_vec()),
            ("_metadata.json", b"{}".to_vec()),
            ("_thumbnail.png", b"png".to_vec()),
            ("script/", Vec::new()),
            ("script/nodes.lua", b"return {}".to_vec()),
            ("img/thumbnail.lif", vec![7u8; 100]),
            ("img/script/a.lif", vec![8u8; 3]),
            ("sounds/a.mp3", vec![9u8; 200_000]),
            ("sounds/a.mp3map", vec![1u8; 40]),
        ]
    }

    fn pk_with(dir: &Path, entries: &[(&str, Vec<u8>)]) -> PathBuf {
        let path = dir.join(format!("{UUID_STR}-1.plain.pk"));
        let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        write_pk(&path, &refs);
        path
    }

    fn run(pk: &Path, work: &Path) -> Result<PreparedStory, PrepareError> {
        prepare_story(
            pk,
            &test_keys(),
            work,
            &Limits::default(),
            &AtomicBool::new(false),
        )
    }

    fn list_files(root: &Path) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for e in std::fs::read_dir(dir).unwrap() {
                let p = e.unwrap().path();
                if p.is_dir() {
                    walk(root, &p, out);
                } else {
                    let rel = p.strip_prefix(root).unwrap();
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        let mut out = Vec::new();
        if root.exists() {
            walk(root, root, &mut out);
        }
        out.sort();
        out
    }

    fn decrypt(data: &[u8]) -> Vec<u8> {
        let mut d = data.to_vec();
        decrypt_blocks(&TEST_KEY, &TEST_IV, &mut d).unwrap();
        d
    }

    /// Aucun dossier de chantier ni dossier final dans `work`.
    fn assert_nothing_left(work: &Path) {
        assert!(list_files(work).is_empty(), "{:?}", list_files(work));
        if let Ok(entries) = std::fs::read_dir(work) {
            for e in entries {
                let name = e.unwrap().file_name();
                assert!(!name.to_string_lossy().starts_with(STAGING_PREFIX));
            }
        }
    }

    #[test]
    fn le_dossier_prepare_contient_exactement_les_fichiers_attendus() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let prepared = run(&pk, &work).unwrap();
        let expected = [
            "img/script/a.lif",
            "img/thumbnail.lif",
            "info",
            "key",
            "main.lsf",
            "script/nodes.lsf",
            "sounds/a.mp3",
            "sounds/a.mp3map",
            "version",
        ];
        assert_eq!(prepared.uuid, UUID_STR);
        assert_eq!(prepared.dir, work.join("str").join(UUID_STR));
        assert_eq!(prepared.files, expected);
        assert_eq!(list_files(&prepared.dir), expected);
        assert_eq!(
            list_files(&work).len(),
            expected.len(),
            "rien hors str/<uuid>"
        );

        let read = |name: &str| std::fs::read(prepared.dir.join(name)).unwrap();
        let info = read("info");
        assert_eq!(info.len(), 32, "30 octets complétés à 32");
        assert_eq!(&decrypt(&info)[..INFO.len()], INFO);
        let main = read("main.lsf");
        assert_eq!(main.len(), 48, "33 octets complétés à 48");
        assert_eq!(&decrypt(&main)[..MAIN.len()], MAIN);
        assert_eq!(&decrypt(&read("script/nodes.lsf"))[..9], b"return {}");
        assert_eq!(read("version"), b"1");
        assert_eq!(read("sounds/a.mp3"), vec![9u8; 200_000]);
        assert_eq!(read("img/script/a.lif"), vec![8u8; 3]);
        assert_eq!(read("key"), test_keys().key_file().to_vec());
        let total: u64 = expected.iter().map(|f| read(f).len() as u64).sum();
        assert_eq!(prepared.total_bytes, total);
    }

    #[test]
    /// Les journaux sont vérifiés à part, dans `tests/device_prepare_logs.rs` (processus
    /// séparé : un abonné local manque des événements quand d'autres tests tournent en
    /// parallèle).
    fn aucune_cle_dans_les_erreurs_ni_dans_les_fichiers_produits() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let mut bad = base_entries();
        bad.push(("../evil", b"x".to_vec()));
        let bad_pk = tmp.path().join("bad.plain.pk");
        let refs: Vec<(&str, &[u8])> = bad.iter().map(|(n, d)| (*n, d.as_slice())).collect();
        write_pk(&bad_pk, &refs);
        let prepared = run(&pk, &work).unwrap();
        let err = run(&bad_pk, &work).unwrap_err();
        let texts = [err.to_string(), format!("{err:?}"), format!("{prepared:?}")];
        let secrets: [&[u8]; 3] = [&TEST_KEY, &TEST_IV, &TEST_SERIAL];
        for secret in secrets {
            let as_hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
            for text in &texts {
                assert!(!text.to_lowercase().contains(&as_hex), "{text}");
            }
            // Ni la clé ni l'IV en clair dans un fichier produit.
            for file in &prepared.files {
                let data = std::fs::read(prepared.dir.join(file)).unwrap();
                if secret.len() == 16 {
                    assert!(!data.windows(16).any(|w| w == secret), "{file}");
                }
            }
        }
    }

    #[test]
    fn une_preparation_precedente_est_remplacee() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let stale = work.join("str").join(UUID_STR);
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("ancien"), b"x").unwrap();
        let prepared = run(&pk, &work).unwrap();
        assert!(!prepared.dir.join("ancien").exists());
        assert_eq!(list_files(&prepared.dir), prepared.files);
    }

    fn expect_invalid(entries: &[(&str, Vec<u8>)], needle: &str) {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), entries);
        let work = tmp.path().join("travail");
        match run(&pk, &work) {
            Err(PrepareError::Invalid(m)) => assert!(m.contains(needle), "{m}"),
            other => panic!("attendu Invalid({needle}), obtenu {other:?}"),
        }
        assert_nothing_left(&work);
    }

    #[test]
    fn entrees_obligatoires_absentes() {
        for required in REQUIRED {
            let entries: Vec<_> = base_entries()
                .into_iter()
                .filter(|(n, _)| *n != required)
                .collect();
            expect_invalid(&entries, required);
        }
    }

    #[test]
    fn uuid_de_mauvaise_taille() {
        for size in [0usize, 15, 17] {
            let mut entries = base_entries();
            entries[0].1 = vec![1u8; size];
            expect_invalid(&entries, "uuid.bin");
        }
    }

    #[test]
    fn noms_dangereux_ou_en_collision_refuses() {
        for bad in [
            "../evil",
            "/abs",
            "img/../../x",
            "img/a:b.lif",
            "img/CON.lif",
            "img/fin.",
            ".plain",
            "key",
            "Key",
            "KEY.plain",
        ] {
            let mut entries = base_entries();
            entries.push((bad, b"x".to_vec()));
            expect_invalid(&entries, "refusé");
        }
        let mut entries = base_entries();
        entries.push(("MAIN.LSF", b"x".to_vec()));
        expect_invalid(&entries, "donnent");
        let mut entries = base_entries();
        entries.push(("Version", b"x".to_vec()));
        expect_invalid(&entries, "double");
    }

    #[test]
    fn collisions_de_casse_hors_ascii_et_arbre_incoherent_refuses_au_plan() {
        let mut entries = base_entries();
        entries.push(("img/\u{e9}.png", b"x".to_vec()));
        entries.push(("img/\u{c9}.png", b"y".to_vec()));
        expect_invalid(&entries, "double");
        // Confondus seulement en majuscules (`ſ` et `s` donnent `S`, `ı` et `i` donnent `I`).
        for (a, b) in [
            ("img/\u{17f}.png", "img/s.png"),
            ("img/\u{131}.png", "img/i.png"),
        ] {
            let mut entries = base_entries();
            entries.push((a, b"x".to_vec()));
            entries.push((b, b"y".to_vec()));
            expect_invalid(&entries, "double");
        }
        // Correspondance à plusieurs caractères (`ß` → `SS`) : FAT garde `ß`, deux fichiers.
        let mut entries = base_entries();
        entries.push(("img/\u{df}.png", b"x".to_vec()));
        entries.push(("img/ss.png", b"y".to_vec()));
        let tmp = tempfile::tempdir().unwrap();
        run(&pk_with(tmp.path(), &entries), &tmp.path().join("travail")).unwrap();
        // `sounds/a.mp3` est un fichier : `sounds/a.mp3/b` ne peut pas être un dossier.
        let mut entries = base_entries();
        entries.push(("sounds/a.mp3/b", b"x".to_vec()));
        expect_invalid(&entries, "fichier et un dossier");
        let long = format!("img/{}.png", "n".repeat(300));
        let mut entries = base_entries();
        entries.push((long.as_str(), b"x".to_vec()));
        expect_invalid(&entries, "trop long");
    }

    #[test]
    fn extension_lua_en_majuscules_chiffree_et_fichiers_propres_exclus_a_la_casse_pres() {
        let tmp = tempfile::tempdir().unwrap();
        let mut entries = base_entries();
        entries.push(("script/AUTRE.LUA", b"return 1".to_vec()));
        entries.retain(|(name, _)| *name != "_thumbnail.png");
        entries.push(("_THUMBNAIL.PNG", b"png".to_vec()));
        let pk = pk_with(tmp.path(), &entries);
        let work = tmp.path().join("travail");
        let story = run(&pk, &work).unwrap();
        assert!(story.files.contains(&"script/AUTRE.lsf".to_string()));
        assert!(!story
            .files
            .iter()
            .any(|f| f.to_lowercase().contains("thumbnail.png")));
        assert!(!story.files.iter().any(|f| f.ends_with(".LUA")));
    }

    #[test]
    fn tailles_declarees_refusees_avant_toute_ecriture() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let limits = Limits {
            max_total_bytes: 1_000,
            ..Limits::default()
        };
        let err =
            prepare_story(&pk, &test_keys(), &work, &limits, &AtomicBool::new(false)).unwrap_err();
        assert!(
            matches!(&err, PrepareError::TooLarge(m) if m.contains("déclarés")),
            "{err:?}"
        );
        assert!(!work.exists(), "aucun chantier créé");
    }

    #[test]
    fn archive_corrompue_ou_tronquee() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("travail");
        let garbage = tmp.path().join("garbage.plain.pk");
        std::fs::write(&garbage, b"pas un zip du tout").unwrap();
        assert!(matches!(
            run(&garbage, &work),
            Err(PrepareError::Invalid(_))
        ));

        let pk = pk_with(tmp.path(), &base_entries());
        let bytes = std::fs::read(&pk).unwrap();
        for cut in [bytes.len() / 2, bytes.len() - 10, 30] {
            let truncated = tmp.path().join("tronque.plain.pk");
            std::fs::write(&truncated, &bytes[..cut]).unwrap();
            assert!(matches!(
                run(&truncated, &work),
                Err(PrepareError::Invalid(_))
            ));
            assert_nothing_left(&work);
        }

        // Contenu altéré (CRC faux) dans un fichier copié tel quel.
        let mut corrupt = bytes.clone();
        let pos = corrupt.windows(8).position(|w| w == [9u8; 8]).unwrap();
        corrupt[pos + 1000] ^= 0xff;
        let altered = tmp.path().join("altere.plain.pk");
        std::fs::write(&altered, &corrupt).unwrap();
        assert!(matches!(
            run(&altered, &work),
            Err(PrepareError::Invalid(_))
        ));
        assert_nothing_left(&work);
    }

    #[test]
    fn bornes_respectees() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let cancel = AtomicBool::new(false);
        let cases = [
            Limits {
                max_entries: 5,
                ..Limits::default()
            },
            Limits {
                max_total_bytes: 100_000,
                ..Limits::default()
            },
            Limits {
                max_encrypted_bytes: 20,
                ..Limits::default()
            },
        ];
        for limits in cases {
            let err = prepare_story(&pk, &test_keys(), &work, &limits, &cancel).unwrap_err();
            assert!(matches!(err, PrepareError::TooLarge(_)), "{err:?}");
            assert_nothing_left(&work);
        }
    }

    /// Simule un arrêt (erreur ou annulation) à la n-ième opération d'écriture.
    struct FaultySink {
        ops: Cell<usize>,
        fail_at: usize,
        mode: Fault,
        cancel: Arc<AtomicBool>,
        budget: Cell<Option<u64>>,
    }

    #[derive(Clone, Copy, PartialEq)]
    enum Fault {
        Error,
        Cancel,
        None,
    }

    impl FaultySink {
        fn new(fail_at: usize, mode: Fault, cancel: Arc<AtomicBool>) -> Self {
            Self {
                ops: Cell::new(0),
                fail_at,
                mode,
                cancel,
                budget: Cell::new(None),
            }
        }
        fn tick(&self) -> io::Result<()> {
            let n = self.ops.get();
            self.ops.set(n + 1);
            if n == self.fail_at {
                match self.mode {
                    Fault::Error => return Err(io::Error::other("panne simulée")),
                    Fault::Cancel => self.cancel.store(true, Ordering::Relaxed),
                    Fault::None => {}
                }
            }
            Ok(())
        }
    }

    struct FaultyWriter<'a> {
        sink: &'a FaultySink,
        inner: Box<dyn Write + 'a>,
    }

    impl Write for FaultyWriter<'_> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.sink.tick()?;
            if let Some(left) = self.sink.budget.get() {
                if (buf.len() as u64) > left {
                    return Err(io::Error::new(io::ErrorKind::StorageFull, "disque plein"));
                }
                self.sink.budget.set(Some(left - buf.len() as u64));
            }
            self.inner.write(buf)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.sink.tick()?;
            self.inner.flush()
        }
    }

    impl Sink for FaultySink {
        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            self.tick()?;
            RealSink.create_dir_all(path)
        }
        fn create(&self, path: &Path) -> io::Result<Box<dyn Write + '_>> {
            self.tick()?;
            Ok(Box::new(FaultyWriter {
                sink: self,
                inner: Box::new(File::create_new(path)?),
            }))
        }
        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.tick()?;
            RealSink.rename(from, to)
        }
        fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
            self.tick()?;
            RealSink.remove_dir_all(path)
        }
    }

    #[test]
    fn injection_de_panne_et_d_annulation_a_chaque_etape() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");

        // Nombre total d'opérations d'une préparation réussie (sur une préparation existante,
        // pour couvrir aussi la suppression de l'ancienne).
        run(&pk, &work).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let counter = FaultySink::new(usize::MAX, Fault::None, cancel.clone());
        prepare_with(
            &counter,
            &pk,
            &test_keys(),
            &work,
            &Limits::default(),
            &cancel,
        )
        .unwrap();
        let total_ops = counter.ops.get();
        assert!(total_ops > 10);

        for mode in [Fault::Error, Fault::Cancel] {
            for fail_at in 0..total_ops {
                std::fs::remove_dir_all(&work).unwrap();
                run(&pk, &work).unwrap();
                let previous = list_files(&work.join("str").join(UUID_STR));
                let cancel = Arc::new(AtomicBool::new(false));
                let sink = FaultySink::new(fail_at, mode, cancel.clone());
                let result =
                    prepare_with(&sink, &pk, &test_keys(), &work, &Limits::default(), &cancel);
                let final_dir = work.join("str").join(UUID_STR);
                // Les 4 dernières opérations (dossier `str`, mise à l'écart de l'ancienne
                // préparation, renommage final, suppression de l'ancienne) suivent le dernier
                // contrôle d'annulation ; seule la dernière peut échouer sans faire échouer
                // la préparation.
                let tail = fail_at + 4 >= total_ops;
                let last = fail_at + 1 == total_ops;
                match result {
                    Err(PrepareError::Io(_)) if mode == Fault::Error && !last => {}
                    Ok(_) if mode == Fault::Error && last => {
                        // Ancienne préparation laissée sous un nom de chantier.
                        assert_eq!(cleanup_stale(&work).unwrap(), 1, "étape {fail_at}");
                    }
                    Err(PrepareError::Cancelled) if mode == Fault::Cancel && !tail => {}
                    Ok(_) if mode == Fault::Cancel && tail => {}
                    other => panic!("{fail_at} : résultat inattendu {other:?}"),
                }
                // Invariant : pas de chantier.
                for e in std::fs::read_dir(&work).unwrap() {
                    let name = e.unwrap().file_name();
                    assert!(
                        !name.to_string_lossy().starts_with(STAGING_PREFIX),
                        "chantier laissé (étape {fail_at})"
                    );
                }
                // Une préparation (l'ancienne ou la nouvelle, de même contenu) reste en place.
                assert_eq!(list_files(&final_dir), previous, "étape {fail_at}");
            }
        }
    }

    #[test]
    fn disque_plein_simule() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let cancel = Arc::new(AtomicBool::new(false));
        let sink = FaultySink::new(usize::MAX, Fault::None, cancel.clone());
        sink.budget.set(Some(150_000));
        let err =
            prepare_with(&sink, &pk, &test_keys(), &work, &Limits::default(), &cancel).unwrap_err();
        match err {
            PrepareError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::StorageFull),
            other => panic!("{other:?}"),
        }
        assert_nothing_left(&work);
    }

    #[test]
    fn annulation_avant_le_debut() {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        let work = tmp.path().join("travail");
        let cancel = AtomicBool::new(true);
        let err = prepare_story(&pk, &test_keys(), &work, &Limits::default(), &cancel).unwrap_err();
        assert!(matches!(err, PrepareError::Cancelled));
        assert!(!work.exists());
    }

    #[test]
    fn chantiers_orphelins_nettoyes() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path();
        assert_eq!(cleanup_stale(&work.join("absent")).unwrap(), 0);
        std::fs::create_dir_all(work.join(".partiel-abc").join("img")).unwrap();
        std::fs::create_dir_all(work.join("str").join(UUID_STR)).unwrap();
        std::fs::write(work.join(".partiel-fichier"), b"x").unwrap();
        assert_eq!(cleanup_stale(work).unwrap(), 1);
        assert!(!work.join(".partiel-abc").exists());
        assert!(work.join("str").join(UUID_STR).exists());
        assert!(work.join(".partiel-fichier").exists());
    }
}
