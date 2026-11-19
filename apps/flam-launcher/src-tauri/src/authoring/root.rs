//! Racines refusées pour un dossier d'histoire (`docs/creation-dossier.md` § 10).
//!
//! L'éditeur écrira dans le dossier choisi (lot 6) : il ne doit jamais s'agir d'un disque
//! entier, d'un dossier système, du profil utilisateur, du dossier du launcher ni du lecteur de
//! la Flam.
//!
//! # Invariants
//!
//! - **Forme verbatim simplifiée d'abord** ([`simplify`]) : `\\?\C:\x` → `C:\x`,
//!   `\\?\UNC\srv\partage\x` → `\\srv\partage\x`, seulement si la forme courte désigne le même
//!   chemin pour Win32 (aucun composant nom de périphérique réservé, aucun composant finissant
//!   par un point ou une espace, moins de 260 caractères) ; sinon la forme `\\?\` est gardée.
//! - **Forme refusée ensuite** (sans accès au disque) : chemin relatif, composant `.` ou `..`,
//!   préfixe Windows autre que lecteur (`C:`), lecteur verbatim (`\\?\C:`) ou partage UNC
//!   (`\\serveur\partage`) — donc `\\?\GLOBALROOT\…`, `\\.\…` refusés —, composant finissant par
//!   un point ou une espace (Win32 les retire : `C:\Users\moi.` désigne `C:\Users\moi`) ou nom
//!   de périphérique réservé (`CON`, `NUL`, `COM1`…), racine d'un disque ou d'un partage.
//! - **Comparaison sur chemins résolus** : la racine et chaque dossier interdit sont résolus
//!   (`canonicalize` : liens, jonctions comme `C:\Documents and Settings`, noms courts 8.3,
//!   lecteurs `subst`, casse), simplifiés de même, puis comparés composant par composant sans
//!   casse. Un dossier interdit introuvable est comparé sous sa forme écrite. La forme résolue
//!   de la racine est rendue : c'est elle que lit `authoring::tree::scan`.
//! - **Deux familles** ([`ForbiddenRoots`]) : `profile` (profil utilisateur) refuse le dossier
//!   lui-même et ses parents, ses sous-dossiers restent permis (`Documents\Histoires`) ;
//!   `trees` (Windows, Program Files, dossier du launcher, lecteur de la Flam) refuse en plus
//!   tous leurs sous-dossiers.
//! - Ce contrôle vaut **au moment de l'ouverture** ; les opérations (lot 6) revérifient chaque
//!   chemin au moment d'agir (voir `authoring::tree`).

use std::path::{Component, Path, PathBuf};

use super::tree::ScanError;

/// Dossiers interdits comme racine (voir la doc du module).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForbiddenRoots {
    /// Refusés avec leurs parents ; sous-dossiers permis.
    pub profile: Vec<PathBuf>,
    /// Refusés avec leurs parents et tous leurs sous-dossiers.
    pub trees: Vec<PathBuf>,
}

impl ForbiddenRoots {
    /// Dossiers du système : profil utilisateur ; `SystemRoot`, `windir`, `ProgramFiles`,
    /// `ProgramFiles(x86)`, `ProgramW6432`.
    pub fn system() -> Self {
        let trees = [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
        ]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .collect();
        Self {
            profile: dirs::home_dir().into_iter().collect(),
            trees,
        }
    }

    /// Ajoute des dossiers refusés avec tous leurs sous-dossiers (dossier du launcher, lecteur
    /// de la Flam…).
    pub fn with_trees(mut self, extra: &[PathBuf]) -> Self {
        self.trees.extend(extra.iter().cloned());
        self
    }
}

fn forbidden(root: &Path, reason: &'static str) -> ScanError {
    ScanError::Forbidden {
        path: root.to_string_lossy().into_owned(),
        reason,
    }
}

/// Contrôle de forme, sans accès au disque.
fn check_shape(root: &Path) -> Result<(), ScanError> {
    if !root.is_absolute() {
        return Err(ScanError::NotAbsolute(root.to_string_lossy().into_owned()));
    }
    for component in root.components() {
        match component {
            Component::Prefix(prefix) => {
                use std::path::Prefix;
                if !matches!(
                    prefix.kind(),
                    Prefix::Disk(_) | Prefix::VerbatimDisk(_) | Prefix::UNC(..)
                ) {
                    return Err(forbidden(root, "forme de chemin non prise en charge"));
                }
            }
            Component::ParentDir | Component::CurDir => {
                return Err(ScanError::NotAbsolute(root.to_string_lossy().into_owned()));
            }
            Component::Normal(name) => {
                if win32_alters(&name.to_string_lossy()) {
                    return Err(forbidden(
                        root,
                        "nom de dossier réservé ou finissant par un point ou une espace",
                    ));
                }
            }
            Component::RootDir => {}
        }
    }
    if root.parent().is_none() {
        return Err(forbidden(root, "racine d'un disque ou d'un partage"));
    }
    Ok(())
}

/// Vrai si Win32 (hors forme `\\?\`) ne désigne pas ce composant tel quel : nom finissant par un
/// point ou une espace, ou nom de périphérique réservé (`CON`, `PRN`, `AUX`, `NUL`, `COM1`-`9`,
/// `LPT1`-`9`, extension comprise : `nul.txt`).
pub(crate) fn win32_alters(name: &str) -> bool {
    if name.ends_with(['.', ' ']) {
        return true;
    }
    let base = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_end()
        .to_ascii_uppercase();
    matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((base.starts_with("COM") || base.starts_with("LPT"))
            && base.len() == 4
            && matches!(base.as_bytes()[3], b'1'..=b'9'))
}

/// Longueur (exclue) au-delà de laquelle la forme `\\?\` est gardée (`MAX_PATH`).
const MAX_SHORT_PATH: usize = 260;

/// Retire le préfixe verbatim `\\?\` (`\\?\C:\x` → `C:\x`, `\\?\UNC\srv\x` → `\\srv\x`)
/// seulement si la forme courte désigne le même chemin pour Win32 ; sinon rend `path` tel quel.
pub(crate) fn simplify(path: PathBuf) -> PathBuf {
    if !cfg!(windows) {
        return path;
    }
    let text = path.to_string_lossy();
    let short = if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            rest.to_owned()
        } else {
            return path;
        }
    } else {
        return path;
    };
    let safe = short.encode_utf16().count() < MAX_SHORT_PATH
        && !short.contains('/')
        && short
            .split('\\')
            .skip(if short.starts_with(r"\\") { 4 } else { 1 })
            .all(|part| part.is_empty() || !win32_alters(part));
    if safe {
        PathBuf::from(short)
    } else {
        path
    }
}

/// Chemin résolu (liens, 8.3, `subst`, casse), simplifié par [`simplify`].
fn resolve(path: &Path) -> std::io::Result<PathBuf> {
    std::fs::canonicalize(path).map(simplify)
}

/// Composants en minuscules ; préfixes verbatim et ordinaires ramenés à la même forme
/// (`\\?\C:` = `C:`, `\\?\UNC\srv\p` = `\\srv\p`).
fn lower_parts(path: &Path) -> Vec<String> {
    use std::path::Prefix;
    path.components()
        .filter(|c| !matches!(c, Component::RootDir))
        .map(|c| match c {
            Component::Prefix(prefix) => match prefix.kind() {
                Prefix::Disk(d) | Prefix::VerbatimDisk(d) => {
                    format!("{}:", (d as char).to_ascii_lowercase())
                }
                Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => format!(
                    r"\\{}\{}",
                    server.to_string_lossy(),
                    share.to_string_lossy()
                )
                .to_lowercase(),
                _ => c.as_os_str().to_string_lossy().to_lowercase(),
            },
            _ => c.as_os_str().to_string_lossy().to_lowercase(),
        })
        .collect()
}

/// `a` est égal à `b` ou l'un de ses parents.
fn covers(a: &[String], b: &[String]) -> bool {
    a.len() <= b.len() && b[..a.len()] == *a
}

/// Vérifie que `root` peut être le dossier d'une histoire et rend sa forme résolue.
pub fn check_root(root: &Path, forbidden_roots: &ForbiddenRoots) -> Result<PathBuf, ScanError> {
    // `\\?\UNC\srv\p\x` est contrôlé comme `\\srv\p\x` (forme sûre seulement).
    let simplified = simplify(root.to_path_buf());
    check_shape(&simplified)?;
    let resolved = match resolve(&simplified) {
        Ok(path) => path,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(ScanError::NotFound(root.to_string_lossy().into_owned()));
        }
        Err(e) => return Err(ScanError::Io(e)),
    };
    if resolved.parent().is_none() {
        return Err(forbidden(root, "racine d'un disque ou d'un partage"));
    }
    let parts = lower_parts(&resolved);
    let resolved_parts =
        |dir: &PathBuf| lower_parts(&resolve(dir).unwrap_or_else(|_| simplify(dir.clone())));
    for dir in &forbidden_roots.profile {
        if covers(&parts, &resolved_parts(dir)) {
            return Err(forbidden(
                root,
                "profil utilisateur ou dossier qui le contient",
            ));
        }
    }
    for dir in &forbidden_roots.trees {
        let other = resolved_parts(dir);
        if covers(&parts, &other) || covers(&other, &parts) {
            return Err(forbidden(
                root,
                "dossier système, du launcher ou de la Flam (ou dossier qui le contient)",
            ));
        }
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests;
