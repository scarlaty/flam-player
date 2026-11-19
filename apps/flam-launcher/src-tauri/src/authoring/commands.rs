//! Commandes Tauri de la création d'une histoire à partir d'un dossier (lot 5,
//! `docs/creation-dossier.md` § 8) : [`authoring_read`] (arbre et rapport, rien n'est écrit),
//! [`authoring_create`] (uuid et version enregistrés, tâche ajoutée à la file d'installation),
//! [`authoring_retry`] (« Réessayer » une tâche « dossier » en échec : même dossier, même uuid,
//! même réponse au conflit) et [`authoring_relocate`] (« Choisir le dossier » d'une histoire
//! dont le dossier a été déplacé : même uuid exigé).
//!
//! Le chemin vient du sélecteur de dossier natif (ou de la source d'une histoire de « Mes
//! histoires ») : il est revérifié ici ([`tree::scan`] : racines interdites, liens), comme celui
//! d'une source dossier.
//!
//! Les refus propres à la création sont des `AppError::Authoring` qui portent un code stable
//! ([`code`]) : le front choisit son texte d'après lui, jamais d'après le message.

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

/// Codes stables des erreurs `authoring` (`AppError::Authoring`, miroir de
/// `AuthoringErrorCode` dans `src/api/authoring.ts`).
pub mod code {
    /// Dossier introuvable (déplacé, renommé, clé retirée) : `tree::ScanError::code`.
    pub const FOLDER_MISSING: &str = "folder_missing";
    /// Dossier refusé (racine interdite, lien, chemin non absolu) : `tree::ScanError::code`.
    pub const FOLDER_REFUSED: &str = "folder_refused";
    /// Un problème bloquant est apparu depuis la lecture du dossier.
    pub const BLOCKED: &str = "blocked";
    /// Une création est déjà en cours pour cette histoire ou ce dossier.
    pub const BUSY: &str = "busy";
    /// Dossier copié (conflit apparu depuis la lecture, ou réponse manquante).
    pub const CONFLICT_COPIED: &str = "conflict_copied";
    /// Identifiant d'une histoire d'une autre source : seulement « Nouvelle histoire ».
    pub const CONFLICT_FOREIGN: &str = "conflict_foreign";
    /// Version de l'histoire au maximum.
    pub const VERSION_OVERFLOW: &str = "version_overflow";
    /// Tâche à relancer introuvable, encore active ou qui n'est pas une création.
    pub const RETRY_UNAVAILABLE: &str = "retry_unavailable";
    /// Relance : le dossier ne porte plus l'identifiant de la tâche.
    pub const FOLDER_CHANGED: &str = "folder_changed";
    /// « Choisir le dossier » : le dossier choisi porte un autre identifiant, ou aucun.
    pub const UUID_MISMATCH: &str = "uuid_mismatch";
    /// « Choisir le dossier » : l'uuid était gardé par le launcher (dossier en lecture seule),
    /// le dossier déplacé ne peut pas être reconnu.
    pub const READ_ONLY_FOLDER: &str = "read_only_folder";
    /// « Choisir le dossier » : histoire non installée, ou pas faite d'un dossier.
    pub const NOT_FROM_FOLDER: &str = "not_from_folder";
}

/// Refus `authoring` de code `code`.
fn refused(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::Authoring {
        code,
        message: message.into(),
    }
}

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
pub(super) fn launcher_forbidden(config_dir: &Path, path: &Path) -> Vec<PathBuf> {
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
pub(super) struct Read {
    pub(super) scan: Scan,
    pub(super) problems: Vec<Problem>,
    pub(super) identity: Identity,
    pub(super) forbidden: Vec<PathBuf>,
}

pub(super) async fn read(
    config: &ConfigState,
    install: &InstallService,
    path: &str,
) -> AppResult<Read> {
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

pub(super) fn report(read: &Read) -> FolderReport {
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

/// Corps de [`authoring_create`], partagé avec l'éditeur (`ops_commands`).
pub(super) async fn create_at(
    config: &ConfigState,
    install: &InstallService,
    path: &str,
    choice: Option<ConflictChoice>,
) -> AppResult<FolderCreated> {
    create(config, install, path, choice, None).await
}

/// Crée (ou actualise) l'histoire du dossier `path` ; `retry` : tâche en échec relancée (son
/// uuid est exigé, sa réponse au conflit reprise, voir [`retry_choice`]).
async fn create(
    config: &ConfigState,
    install: &InstallService,
    path: &str,
    choice: Option<ConflictChoice>,
    retry: Option<&FolderJob>,
) -> AppResult<FolderCreated> {
    let read = read(config, install, path).await?;
    let report = report(&read);
    if report.blocked {
        let first: Vec<&str> = report
            .problems
            .iter()
            .filter(|p| p.severity == Severity::Blocking)
            .take(3)
            .map(|p| p.message.as_str())
            .collect();
        return Err(refused(
            code::BLOCKED,
            format!(
                "ce dossier ne peut pas devenir une histoire : {}",
                first.join(" ; ")
            ),
        ));
    }
    let choice = match retry {
        Some(job) => retry_choice(&read.identity, job)?,
        None => choice,
    };
    let decision = identity::decide(&read.identity, choice)?;
    let base = read.scan.base.clone();
    // Contrôle, enregistrement et mise en file sous le verrou de la configuration : deux
    // créations simultanées du même dossier (uuid neuf chacune) ne passent jamais toutes deux.
    let _guard = config.guard();
    if install.has_active(&decision.uuid, &base) {
        return Err(refused(
            code::BUSY,
            "cette histoire est déjà en cours de création : attendez la fin ou annulez-la, puis \
             réessayez",
        ));
    }
    let saved_in = identity::commit(&base, &decision, &Registry::new(&config.dir))?;
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
        choice,
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

/// Réponse au conflit d'une relance de la tâche `job`. Le dossier doit porter **le même uuid**
/// que la tâche (sinon `folder_changed` : il a changé depuis, « Actualiser » le relit) ;
/// « Remplacer l'autre » n'est repris que si la tâche l'avait choisi ; « Nouvelle histoire »
/// ne l'est jamais (l'uuid neuf est déjà enregistré : le redemander en créerait un troisième).
/// Un conflit apparu depuis est laissé à `identity::decide` (`conflict_copied`,
/// `conflict_foreign`).
pub fn retry_choice(identity: &Identity, job: &FolderJob) -> AppResult<Option<ConflictChoice>> {
    if identity.uuid.as_deref() != Some(job.uuid.as_str()) {
        return Err(refused(
            code::FOLDER_CHANGED,
            "le dossier ne porte plus l'identifiant de cette histoire : utilisez « Actualiser »",
        ));
    }
    Ok(match (&identity.conflict, job.choice) {
        (Some(Conflict::Copied { .. }), Some(ConflictChoice::ReplaceOther)) => {
            Some(ConflictChoice::ReplaceOther)
        }
        _ => None,
    })
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
    create(&config, &install, &path, choice, None).await
}

/// « Réessayer » la tâche « dossier » `task_id` en échec ou annulée : nouvelle création depuis
/// le même dossier, sous le même uuid et avec la même réponse au conflit ([`retry_choice`]),
/// à la version suivante. La tâche relancée n'est pas retirée ici (le front la remplace).
#[tauri::command]
pub async fn authoring_retry(
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    task_id: String,
) -> AppResult<FolderCreated> {
    let job = install.retryable_folder(&task_id).ok_or_else(|| {
        refused(
            code::RETRY_UNAVAILABLE,
            "cette création ne peut pas être relancée",
        )
    })?;
    create(&config, &install, &job.folder, None, Some(&job)).await
}

/// « Choisir le dossier » de l'histoire installée `uuid` dont le dossier est introuvable
/// (§ 8). Le dossier `path` est lu comme pour une création (racines interdites, liens) et doit
/// porter le même uuid (`_histoire.json`, ou configuration du launcher pour ce chemin), sinon
/// `uuid_mismatch` ; le chemin retenu par « Mes histoires » devient alors sa forme résolue,
/// rendue. Rien n'est écrit dans le dossier.
#[tauri::command]
pub async fn authoring_relocate(
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    catalog: State<'_, crate::catalog::CatalogService>,
    uuid: String,
    path: String,
) -> AppResult<String> {
    let read = read(&config, &install, &path).await?;
    if let Err(err) = check_relocation(&read.identity, &uuid) {
        // Uuid gardé par le launcher pour l'ancien chemin (dossier en lecture seule) : le
        // dossier déplacé ne le porte pas, il ne peut pas être reconnu (§ 8).
        if read.identity.uuid.is_none() && remembered_elsewhere(&config, &install, &uuid).await {
            return Err(refused(
                code::READ_ONLY_FOLDER,
                "l'identifiant de cette histoire était gardé par le launcher (dossier en lecture \
                 seule) : le dossier déplacé ne peut pas être reconnu",
            ));
        }
        return Err(err);
    }
    let base = read.scan.base.clone();
    // Une création en cours publierait l'ancien chemin en se terminant.
    if install.has_active(&uuid, &base) {
        return Err(refused(
            code::BUSY,
            "cette histoire est en cours de création : attendez la fin ou annulez-la",
        ));
    }
    let location = base.to_string_lossy().into_owned();
    // Index de la bibliothèque sous le cache : pas de déplacement ni de vidage du cache pendant
    // l'écriture (même portillon que `library_remove`).
    let _gate = catalog.cache_gate().read_owned().await;
    install
        .relocate_folder_story(&uuid, &location)
        .await
        .map_err(|err| match err {
            AppError::NotFound(_) | AppError::Validation(_) => refused(
                code::NOT_FROM_FOLDER,
                "cette histoire n'est pas installée depuis un dossier",
            ),
            other => other,
        })?;
    Ok(location)
}

/// Vrai si l'uuid `uuid` est gardé par le launcher pour le dossier que retient « Mes
/// histoires » (dossier en lecture seule, § 8).
async fn remembered_elsewhere(config: &ConfigState, install: &InstallService, uuid: &str) -> bool {
    let Ok(library) = install.library_list().await else {
        return false;
    };
    let Some(old) = library
        .iter()
        .find(|e| e.uuid == uuid && e.source.source_id == build::SOURCE_ID)
        .map(|e| PathBuf::from(&e.source.download_url))
    else {
        return false;
    };
    let _guard = config.guard();
    Registry::new(&config.dir)
        .find(&old)
        .is_some_and(|r| r.uuid == uuid)
}

/// Le dossier lu (identité `identity`) est celui de l'histoire `uuid` : sinon `uuid_mismatch`.
/// Le dossier d'origine existe encore avec cet uuid (`Conflict::Copied`) : le dossier choisi en
/// est une copie, refusée (`conflict_copied`).
pub fn check_relocation(identity: &Identity, uuid: &str) -> AppResult<()> {
    if identity.uuid.as_deref() == Some(uuid) {
        if let Some(Conflict::Copied { .. }) = identity.conflict {
            return Err(refused(
                code::CONFLICT_COPIED,
                "le dossier d'origine de cette histoire existe encore : ce dossier en est une copie",
            ));
        }
        Ok(())
    } else {
        Err(refused(
            code::UUID_MISMATCH,
            "ce dossier n'est pas celui de cette histoire (identifiant différent ou absent)",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UUID: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

    fn job(choice: Option<ConflictChoice>) -> FolderJob {
        FolderJob {
            uuid: UUID.into(),
            version: 2,
            folder: r"C:\Histoires\Loup".into(),
            voice: None,
            forbidden: Vec::new(),
            choice,
        }
    }

    fn identity(uuid: Option<&str>, conflict: Option<Conflict>) -> Identity {
        Identity {
            uuid: uuid.map(str::to_string),
            version: Some(2),
            conflict,
            ..Identity::default()
        }
    }

    fn copied() -> Option<Conflict> {
        Some(Conflict::Copied {
            other_folder: r"C:\Autre".into(),
            title: "Autre".into(),
        })
    }

    fn code_of(err: AppError) -> &'static str {
        match err {
            AppError::Authoring { code, .. } => code,
            other => panic!("erreur authoring attendue : {other:?}"),
        }
    }

    #[test]
    fn une_relance_exige_le_meme_uuid() {
        for uuid in [None, Some("6d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d")] {
            let err = retry_choice(&identity(uuid, None), &job(None)).unwrap_err();
            assert_eq!(code_of(err), code::FOLDER_CHANGED);
        }
        assert_eq!(
            retry_choice(&identity(Some(UUID), None), &job(None)).unwrap(),
            None
        );
    }

    #[test]
    fn une_relance_reprend_remplacer_mais_jamais_nouvelle_histoire() {
        let replace = Some(ConflictChoice::ReplaceOther);
        let fresh = Some(ConflictChoice::NewStory);
        assert_eq!(
            retry_choice(&identity(Some(UUID), copied()), &job(replace)).unwrap(),
            replace
        );
        // « Nouvelle histoire » : l'uuid neuf est déjà celui du dossier, aucun conflit attendu.
        assert_eq!(
            retry_choice(&identity(Some(UUID), None), &job(fresh)).unwrap(),
            None
        );
        // Conflit apparu depuis, sans « Remplacer » choisi : laissé à `decide`, qui refuse.
        for choice in [None, fresh] {
            let answer = retry_choice(&identity(Some(UUID), copied()), &job(choice)).unwrap();
            assert_eq!(answer, None);
            let err = identity::decide(&identity(Some(UUID), copied()), answer).unwrap_err();
            assert_eq!(code_of(err), code::CONFLICT_COPIED);
        }
    }

    #[test]
    fn choisir_le_dossier_exige_le_meme_uuid() {
        check_relocation(&identity(Some(UUID), None), UUID).unwrap();
        let err = check_relocation(&identity(Some(UUID), copied()), UUID).unwrap_err();
        assert_eq!(code_of(err), code::CONFLICT_COPIED);
        for uuid in [None, Some("6d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d")] {
            let err = check_relocation(&identity(uuid, None), UUID).unwrap_err();
            assert_eq!(code_of(err), code::UUID_MISMATCH);
        }
    }

    #[test]
    fn un_dossier_introuvable_ou_refuse_a_son_code() {
        let missing = AppError::from(tree::ScanError::NotFound("x".into()));
        assert_eq!(code_of(missing), code::FOLDER_MISSING);
        let link = AppError::from(tree::ScanError::Link("x".into()));
        assert_eq!(code_of(link), code::FOLDER_REFUSED);
    }
}
