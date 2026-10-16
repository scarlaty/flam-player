//! Histoires FLAM déjà converties : vérification d'un `.plain.pk` et empaquetage d'un dossier
//! `.plain` en `.plain.pk` (zip « stored »). Fonctions bloquantes (à appeler hors du runtime).

use std::fs::File;
use std::io::{self, BufWriter};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{AppError, AppResult};

/// Profondeur et nombre de fichiers maximum d'un dossier `.plain` empaqueté.
const MAX_DEPTH: usize = 16;
const MAX_FILES: usize = 100_000;
/// Fichiers indispensables d'une histoire FLAM.
const REQUIRED: [&str; 3] = ["info.plain", "uuid.bin", "main.lua"];

/// Vérifie qu'un fichier est un `.plain.pk` exploitable : zip lisible contenant `info.plain`,
/// `uuid.bin` et `main.lua` à la racine.
pub fn check_plain_pk(path: &Path) -> AppResult<()> {
    let invalid = |m: String| AppError::DownloadInvalid(format!("archive FLAM invalide : {m}"));
    let file = File::open(path)?;
    let zip = ZipArchive::new(io::BufReader::new(file)).map_err(|e| invalid(e.to_string()))?;
    let names: Vec<&str> = zip.file_names().collect();
    for required in REQUIRED {
        if !names.contains(&required) {
            return Err(invalid(format!("{required} absent")));
        }
    }
    Ok(())
}

/// Empaquette le dossier `.plain` `dir` dans `output` (zip « stored », noms relatifs en `/`).
/// Les liens symboliques et jonctions sont ignorés ; `REQUIRED` doit être présent.
pub fn pack_plain_dir(dir: &Path, output: &Path) -> AppResult<()> {
    for required in REQUIRED {
        if !dir.join(required).is_file() {
            return Err(AppError::Validation(format!(
                "dossier .plain incomplet : {required} absent"
            )));
        }
    }
    let mut files = Vec::new();
    collect(dir, "", 0, &mut files)?;
    let result = (|| -> AppResult<()> {
        let mut zip = ZipWriter::new(BufWriter::new(File::create(output)?));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true);
        for (name, path) in &files {
            zip.start_file(name.as_str(), options)
                .map_err(|e| AppError::Io(io::Error::other(e.to_string())))?;
            io::copy(&mut File::open(path)?, &mut zip)?;
        }
        zip.finish()
            .map_err(|e| AppError::Io(io::Error::other(e.to_string())))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(output);
    }
    result
}

fn collect(
    dir: &Path,
    prefix: &str,
    depth: usize,
    out: &mut Vec<(String, std::path::PathBuf)>,
) -> AppResult<()> {
    if depth > MAX_DEPTH {
        return Ok(());
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let kind = entry.file_type()?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if kind.is_symlink() || name.contains(['/', '\\']) {
            continue;
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            // Jonctions et autres points de réanalyse : jamais suivis.
            const REPARSE: u32 = 0x400;
            if entry.metadata()?.file_attributes() & REPARSE != 0 {
                continue;
            }
        }
        let relative = format!("{prefix}{name}");
        if kind.is_dir() {
            collect(&entry.path(), &format!("{relative}/"), depth + 1, out)?;
        } else if kind.is_file() {
            if out.len() >= MAX_FILES {
                return Err(AppError::Validation(
                    "dossier .plain trop volumineux (nombre de fichiers)".into(),
                ));
            }
            out.push((relative, entry.path()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_dir(root: &Path) -> std::path::PathBuf {
        let dir = root.join("h.plain");
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("info.plain"), "Titre\n").unwrap();
        std::fs::write(dir.join("uuid.bin"), [7u8; 16]).unwrap();
        std::fs::write(dir.join("main.lua"), "-- x").unwrap();
        std::fs::write(dir.join("img/thumbnail.lif"), b"liff").unwrap();
        dir
    }

    #[test]
    fn empaquetage_stored_verifiable() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = plain_dir(tmp.path());
        let out = tmp.path().join("h.plain.pk");
        pack_plain_dir(&dir, &out).unwrap();
        check_plain_pk(&out).unwrap();
        let mut zip = ZipArchive::new(File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = zip.file_names().map(str::to_string).collect();
        names.sort();
        assert_eq!(
            names,
            ["img/thumbnail.lif", "info.plain", "main.lua", "uuid.bin"]
        );
        for i in 0..zip.len() {
            assert_eq!(
                zip.by_index(i).unwrap().compression(),
                CompressionMethod::Stored
            );
        }
    }

    #[test]
    fn dossier_incomplet_ou_archive_invalide_refuses() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = plain_dir(tmp.path());
        std::fs::remove_file(dir.join("main.lua")).unwrap();
        let out = tmp.path().join("h.plain.pk");
        assert_eq!(pack_plain_dir(&dir, &out).unwrap_err().kind(), "validation");
        std::fs::write(&out, b"pas un zip").unwrap();
        assert_eq!(check_plain_pk(&out).unwrap_err().kind(), "download_invalid");
    }
}
