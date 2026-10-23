//! Conversion complète d'une archive (`convert` de `tools/telmi2flam/telmi2flam.py`) :
//! événement `start`, assemblage du paquet ([`crate::package`]), écriture ([`crate::output`]),
//! événement `done`. Les erreurs sont rendues à l'appelant, qui émet l'événement `error`
//! (le binaire, avec le code de sortie de [`ConvError::exit_code`]).
//!
//! À lancer dans un fil à pile de 8 Mo (voir la documentation de la crate).

use std::path::{Path, PathBuf};

use crate::archive::Archive;
use crate::error::ConvError;
use crate::output::{write_package_progress, Target};
use crate::package::{build_package_with, Options, Stats};
use crate::progress::Reporter;

/// Conversion demandée.
#[derive(Debug, Clone)]
pub struct Request<'a> {
    /// Archive Telmi (`input`).
    pub input: &'a Path,
    /// `-o` : `None` pour le nom par défaut dans le dossier courant.
    pub output: Option<&'a Path>,
    /// `--plain` : écrire aussi le dossier `.plain`.
    pub emit_plain: bool,
    pub options: Options,
}

/// Conversion réussie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Sortie (`-o` ou nom par défaut), champ `output` de `done`.
    pub output: PathBuf,
    /// Chemins publiés, dans l'ordre.
    pub published: Vec<PathBuf>,
    /// Taille du `.plain.pk`, ou somme des fichiers pour un dossier `.plain` seul.
    pub size: u64,
    /// Avertissements émis.
    pub warnings: u64,
    /// Titre de l'histoire (résumé humain).
    pub title: String,
    /// Compteurs du résumé humain.
    pub stats: Stats,
    /// Seul le dossier `.plain` a été écrit (`-o` finissant par `.plain`).
    pub dir_only: bool,
}

/// Convertit `req.input`. Émet `start` avant toute lecture, puis les étapes et
/// avertissements, et `done` en cas de succès ; une erreur est rendue sans événement.
pub fn convert(
    req: &Request<'_>,
    reporter: &mut Reporter<'_>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Report, ConvError> {
    reporter.start(req.input.as_os_str());
    let mut archive = Archive::open(req.input)?;
    let package = build_package_with(&mut archive, &req.options, cancelled, reporter)?;
    drop(archive);
    let target = Target::new(req.output, req.emit_plain, &package);
    let published = write_package_progress(&package, &target, cancelled, reporter)?;
    let size = if target.dir_only {
        package.entries.iter().map(|(_, d)| d.len() as u64).sum()
    } else {
        std::fs::metadata(&target.out_path)
            .map_err(|e| {
                ConvError::internal_error(format!("OSError: {} : {e}", target.out_path.display()))
            })?
            .len()
    };
    reporter.done(target.out_path.as_os_str(), size);
    Ok(Report {
        title: package.title,
        stats: package.stats,
        dir_only: target.dir_only,
        output: target.out_path,
        published,
        size,
        warnings: reporter.warnings(),
    })
}
