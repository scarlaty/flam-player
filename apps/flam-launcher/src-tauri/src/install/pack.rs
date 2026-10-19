//! Histoires FLAM déjà converties : vérification d'un `.plain.pk` et empaquetage d'un dossier
//! `.plain` en `.plain.pk` (zip « stored »). Fonctions bloquantes (à appeler hors du runtime).

use std::fs::File;
use std::io::{self, BufWriter, Read};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::{AppError, AppResult};

/// Profondeur et nombre de fichiers maximum d'un dossier `.plain` empaqueté.
const MAX_DEPTH: usize = 16;
const MAX_FILES: usize = 100_000;
/// Taille cumulée maximale des fichiers d'un dossier `.plain` empaqueté.
const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
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

fn refused(what: &Path) -> AppError {
    AppError::Validation(format!(
        "lien symbolique, jonction ou fichier OneDrive « à la demande » refusé dans le dossier .plain : {}",
        what.display()
    ))
}

fn too_big() -> AppError {
    AppError::Validation("dossier .plain trop volumineux (taille cumulée)".into())
}

/// Vrai si `meta` décrit un lien symbolique ou un point de réanalyse (jonction…).
///
/// **Refus volontaire des fichiers OneDrive « à la demande »** : sous Windows, un fichier non
/// téléchargé d'un dossier synchronisé (OneDrive, « Files On-Demand ») est un *point de réanalyse*
/// (attribut `FILE_ATTRIBUTE_REPARSE_POINT`, balise « cloud »), indiscernable ici d'une jonction
/// par l'attribut seul. On refuse sans distinguer plutôt que de déclencher un téléchargement
/// silencieux (volumineux, réseau, hors du contrôle de l'application) ou de suivre un lien. Pour
/// installer un tel dossier `.plain` : le copier hors du dossier synchronisé avant de le
/// réanalyser (« Toujours conserver sur cet appareil » n'est pas garanti de retirer le point de
/// réanalyse : non vérifié sur appareil).
pub(crate) fn is_link(meta: &std::fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const REPARSE: u32 = 0x400;
        if meta.file_attributes() & REPARSE != 0 {
            return true;
        }
    }
    false
}

/// Ouvre un fichier **sans suivre** de lien, puis contrôle la poignée ouverte (une substitution
/// entre la collecte et la lecture est ainsi détectée).
pub(crate) fn open_no_follow(path: &Path) -> AppResult<File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    let opened = file.metadata()?;
    if is_link(&opened) || !opened.is_file() {
        return Err(refused(path));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let on_disk = std::fs::symlink_metadata(path)?;
        if is_link(&on_disk) || on_disk.ino() != opened.ino() || on_disk.dev() != opened.dev() {
            return Err(refused(path));
        }
    }
    Ok(file)
}

/// Fichiers du dossier `.plain` `dir` tels que [`pack_plain_dir`] les empaquetterait : noms
/// relatifs en `/` et chemins, mêmes refus (racine ou entrée en lien, profondeur, nombre et
/// taille cumulée). Sert à l'empreinte de la source (#288), sans rien écrire.
pub fn plain_dir_files(dir: &Path) -> AppResult<Vec<(String, PathBuf)>> {
    let root = std::fs::symlink_metadata(dir)?;
    if is_link(&root) || !root.is_dir() {
        return Err(refused(dir));
    }
    let mut files = Vec::new();
    let mut total = 0u64;
    collect(dir, "", 0, &mut files, &mut total, MAX_TOTAL_BYTES)?;
    Ok(files)
}

/// Empaquette le dossier `.plain` `dir` dans `output` (zip « stored », noms relatifs en `/`).
/// Un lien symbolique ou une jonction (racine comprise) est **refusé** ; `REQUIRED` doit être
/// présent ; la taille cumulée est bornée par `MAX_TOTAL_BYTES`.
pub fn pack_plain_dir(dir: &Path, output: &Path) -> AppResult<()> {
    pack_limited(dir, output, MAX_TOTAL_BYTES)
}

fn pack_limited(dir: &Path, output: &Path, max_total: u64) -> AppResult<()> {
    let root = std::fs::symlink_metadata(dir)?;
    if is_link(&root) || !root.is_dir() {
        return Err(refused(dir));
    }
    for required in REQUIRED {
        match std::fs::symlink_metadata(dir.join(required)) {
            Ok(m) if is_link(&m) => return Err(refused(&dir.join(required))),
            Ok(m) if m.is_file() => {}
            _ => {
                return Err(AppError::Validation(format!(
                    "dossier .plain incomplet : {required} absent"
                )))
            }
        }
    }
    let mut files = Vec::new();
    let mut total = 0u64;
    collect(dir, "", 0, &mut files, &mut total, max_total)?;
    let result = (|| -> AppResult<()> {
        let mut zip = ZipWriter::new(BufWriter::new(File::create(output)?));
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true);
        let mut remaining = max_total;
        for (name, path) in &files {
            zip.start_file(name.as_str(), options)
                .map_err(|e| AppError::Io(io::Error::other(e.to_string())))?;
            // `take(remaining + 1)` : un fichier qui a grossi depuis la collecte est détecté.
            let copied = io::copy(&mut open_no_follow(path)?.take(remaining + 1), &mut zip)?;
            if copied > remaining {
                return Err(too_big());
            }
            remaining -= copied;
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
    out: &mut Vec<(String, PathBuf)>,
    total: &mut u64,
    max_total: u64,
) -> AppResult<()> {
    if depth > MAX_DEPTH {
        return Err(AppError::Validation("dossier .plain trop profond".into()));
    }
    let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<Result<_, _>>()?;
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        // `DirEntry::metadata` ne suit pas les liens.
        let meta = entry.metadata()?;
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if is_link(&meta) {
            return Err(refused(&entry.path()));
        }
        if name.contains(['/', '\\']) {
            continue;
        }
        let relative = format!("{prefix}{name}");
        if meta.is_dir() {
            collect(
                &entry.path(),
                &format!("{relative}/"),
                depth + 1,
                out,
                total,
                max_total,
            )?;
        } else if meta.is_file() {
            if out.len() >= MAX_FILES {
                return Err(AppError::Validation(
                    "dossier .plain trop volumineux (nombre de fichiers)".into(),
                ));
            }
            *total = total.saturating_add(meta.len());
            if *total > max_total {
                return Err(too_big());
            }
            out.push((relative, entry.path()));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain_dir(root: &Path) -> PathBuf {
        let dir = root.join("h.plain");
        std::fs::create_dir_all(dir.join("img")).unwrap();
        std::fs::write(dir.join("info.plain"), "Titre\n").unwrap();
        std::fs::write(dir.join("uuid.bin"), [7u8; 16]).unwrap();
        std::fs::write(dir.join("main.lua"), "-- x").unwrap();
        std::fs::write(dir.join("img/thumbnail.lif"), b"liff").unwrap();
        dir
    }

    /// Crée un lien de dossier `link` -> `target` (jonction sous Windows, sans privilège).
    fn dir_link(link: &Path, target: &Path) -> bool {
        #[cfg(windows)]
        {
            std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J"])
                .arg(link)
                .arg(target)
                .output()
                .is_ok_and(|o| o.status.success())
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, link).is_ok()
        }
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

    #[test]
    fn racine_en_lien_refusee() {
        let tmp = tempfile::tempdir().unwrap();
        let real = plain_dir(tmp.path());
        let link = tmp.path().join("lien.plain");
        if !dir_link(&link, &real) {
            eprintln!("création de lien impossible : test ignoré");
            return;
        }
        let out = tmp.path().join("o.plain.pk");
        let err = pack_plain_dir(&link, &out).unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(err.to_string().contains("lien"), "{err}");
        assert!(!out.exists());
    }

    #[test]
    fn lien_interne_refuse() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = plain_dir(tmp.path());
        let outside = tmp.path().join("dehors");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "x").unwrap();
        if !dir_link(&dir.join("img2"), &outside) {
            eprintln!("création de lien impossible : test ignoré");
            return;
        }
        let out = tmp.path().join("o.plain.pk");
        assert_eq!(pack_plain_dir(&dir, &out).unwrap_err().kind(), "validation");
        assert!(!out.exists());
    }

    #[test]
    fn taille_cumulee_bornee() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = plain_dir(tmp.path());
        std::fs::write(dir.join("gros.bin"), vec![0u8; 5000]).unwrap();
        let out = tmp.path().join("o.plain.pk");
        let err = pack_limited(&dir, &out, 1000).unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(err.to_string().contains("taille"), "{err}");
        assert!(!out.exists());
        pack_limited(&dir, &out, 1 << 20).unwrap();
    }
}
