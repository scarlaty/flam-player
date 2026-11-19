//! Commandes Tauri de la création d'une histoire à partir d'un dossier (lot 5,
//! `docs/creation-dossier.md` § 8) : [`authoring_read`] (arbre et rapport, rien n'est écrit) et
//! [`authoring_create`] (uuid et version enregistrés, tâche ajoutée à la file d'installation).
//!
//! Le chemin vient du sélecteur de dossier natif (ou de la source d'une histoire de « Mes
//! histoires ») : il est revérifié ici ([`tree::scan`] : racines interdites, liens), comme celui
//! d'une source dossier.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::State;

use super::build::{self, scan_limits};
use super::identity::{self, Conflict, ConflictChoice, Identity, Registry, SavedIn};
use super::settings::Mode;
use super::tree::{self, Node, Problem, Scan, Severity};
use crate::catalog::blocking;
use crate::install::folder::FolderJob;
use crate::install::{InstallService, InstallTask};
use crate::storage::ConfigState;
use crate::{AppError, AppResult};

/// Rapport de lecture d'un dossier (miroir de `FolderReport` dans `src/api/authoring.ts`).
#[derive(Debug, Clone, Serialize)]
pub struct FolderReport {
    /// Dossier lu (forme résolue).
    pub folder: String,
    pub title: String,
    /// Arbre : menus, histoires, chapitres.
    pub root: Node,
    /// Problèmes du scan et de la création (`collection_in_book`).
    pub problems: Vec<Problem>,
    /// Un problème bloquant empêche la création.
    pub blocked: bool,
    pub story_count: usize,
    pub audio_bytes: u64,
    /// Mode effectif de la racine.
    pub mode: Mode,
    /// Mode proposé quand `_histoire.json` n'en fixe pas.
    pub suggested_mode: Option<Mode>,
    /// Uuid connu (actualisation) ; `None` : création.
    pub uuid: Option<String>,
    /// Version installée dans « Mes histoires » pour cet uuid.
    pub installed_version: Option<u32>,
    /// L'uuid est gardé par le launcher (dossier en lecture seule).
    pub remembered: bool,
    /// Conflit à faire trancher (`authoring_create`, paramètre `choice`).
    pub conflict: Option<Conflict>,
}

/// Résultat de [`authoring_create`] (miroir de `FolderCreated`).
#[derive(Debug, Clone, Serialize)]
pub struct FolderCreated {
    pub task: InstallTask,
    pub uuid: String,
    pub version: u32,
    /// Première création (uuid neuf).
    pub created: bool,
    /// Où l'uuid a été enregistré.
    pub saved_in: SavedIn,
}

/// Lecteur de la Flam qui contient `path` : racine du volume marquée par `.mdf` (seul fichier
/// regardé, sans le lire).
fn flam_drive_of(path: &Path) -> Option<PathBuf> {
    let root = path.ancestors().last()?;
    std::fs::symlink_metadata(root.join(".mdf"))
        .is_ok()
        .then(|| root.to_path_buf())
}

/// Dossiers interdits en plus de ceux du système : configuration, programme, lecteur de la Flam
/// (le dossier du cache est ajouté par l'appelant).
fn launcher_forbidden(config_dir: &Path, path: &Path) -> Vec<PathBuf> {
    let mut out = vec![config_dir.to_path_buf()];
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    {
        out.push(exe_dir);
    }
    out.extend(flam_drive_of(path));
    out
}

/// Dossier lu, problèmes de création compris.
struct Read {
    scan: Scan,
    problems: Vec<Problem>,
    identity: Identity,
    forbidden: Vec<PathBuf>,
}

async fn read(config: &ConfigState, install: &InstallService, path: &str) -> AppResult<Read> {
    let root = PathBuf::from(path);
    let mut forbidden = launcher_forbidden(&config.dir, &root);
    let cache_root = install.cache_root().await?;
    let scan_forbidden = {
        let mut all = forbidden.clone();
        all.push(cache_root);
        all
    };
    let scan = blocking(move || tree::scan(&root, &scan_limits(), &scan_forbidden)).await??;
    let mut problems = scan.problems.clone();
    problems.extend(build::book_problems(&scan));
    let library = install.library_list().await?;
    let remembered = {
        let _guard = config.guard();
        Registry::new(&config.dir).find(&scan.base)
    };
    let identity = identity::inspect(&scan.base, &scan.story, remembered.as_ref(), &library);
    // Le cache est relu par la tâche ; les autres dossiers interdits voyagent avec elle.
    forbidden.retain(|p| p.is_absolute());
    Ok(Read {
        scan,
        problems,
        identity,
        forbidden,
    })
}

fn report(read: &Read) -> FolderReport {
    let scan = &read.scan;
    let mode = match &scan.root {
        Node::Menu { mode, .. } => *mode,
        Node::Story { .. } => Mode::Recueil,
    };
    FolderReport {
        folder: scan.base.to_string_lossy().into_owned(),
        title: build::metadata(scan).title,
        root: scan.root.clone(),
        problems: read.problems.clone(),
        blocked: read
            .problems
            .iter()
            .any(|p| p.severity == Severity::Blocking),
        story_count: scan.story_count,
        audio_bytes: scan.audio_bytes,
        mode,
        suggested_mode: scan.suggested_mode,
        uuid: read.identity.uuid.clone(),
        installed_version: read.identity.installed_version,
        remembered: read.identity.remembered,
        conflict: read.identity.conflict.clone(),
    }
}

/// Lit le dossier `path` : arbre, rapport (bloquants et avertissements), identité. N'écrit rien.
#[tauri::command]
pub async fn authoring_read(
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    path: String,
) -> AppResult<FolderReport> {
    let read = read(&config, &install, &path).await?;
    Ok(report(&read))
}

/// Crée (ou actualise) l'histoire du dossier `path` : dossier relu, refus s'il a un problème
/// bloquant, uuid et version décidés (`choice` : réponse à un conflit signalé par
/// [`authoring_read`]) et enregistrés (`_histoire.json`, sinon configuration du launcher), puis
/// tâche ajoutée à la file d'installation.
#[tauri::command]
pub async fn authoring_create(
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    path: String,
    choice: Option<ConflictChoice>,
) -> AppResult<FolderCreated> {
    let read = read(&config, &install, &path).await?;
    let report = report(&read);
    if report.blocked {
        let first: Vec<&str> = report
            .problems
            .iter()
            .filter(|p| p.severity == Severity::Blocking)
            .take(3)
            .map(|p| p.message.as_str())
            .collect();
        return Err(AppError::Validation(format!(
            "ce dossier ne peut pas devenir une histoire : {}",
            first.join(" ; ")
        )));
    }
    let decision = identity::decide(&read.identity, choice)?;
    if install.has_active(&decision.uuid) {
        return Err(AppError::Validation(
            "cette histoire est déjà en cours de création : attendez la fin ou annulez-la, puis \
             réessayez"
                .into(),
        ));
    }
    let base = read.scan.base.clone();
    let saved_in = {
        let _guard = config.guard();
        identity::commit(&base, &decision, &Registry::new(&config.dir))?
    };
    let meta = build::metadata(&read.scan);
    let entry = build::story_entry(&decision.uuid, decision.version, &meta, &base);
    let job = FolderJob {
        uuid: decision.uuid.clone(),
        version: decision.version,
        folder: base.to_string_lossy().into_owned(),
        voice: build::requested_voice(&read.scan),
        forbidden: read
            .forbidden
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
    };
    let task = install.start_folder(job, entry)?;
    Ok(FolderCreated {
        task,
        uuid: decision.uuid,
        version: decision.version,
        created: decision.created,
        saved_in,
    })
}
