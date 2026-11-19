//! Tâche « dossier » de la file d'installation : une histoire faite d'un dossier
//! (`docs/creation-dossier.md` § 8, lot 5).
//!
//! Mêmes étapes et mêmes garanties qu'un pack de podcasts (`install::podcast`) :
//! `queued` → `preparing` (titres lus, audio, images, archive : `authoring::build::prepare`) →
//! `prepared` → `converting` (`telmi2flam`) → `installing` → `done`, annulable à tout moment,
//! une préparation à la fois, dossier de travail `<cache>/podcast-work/<tâche>/` supprimé à la
//! fin quelle qu'en soit l'issue et balayé au démarrage s'il est orphelin.
//!
//! # Invariants
//!
//! - **Le dossier est relu** au début de la préparation (il a pu changer depuis le rapport) :
//!   un problème bloquant apparu entre-temps fait échouer la tâche, rien n'est publié.
//! - **Uuid et version** ([`FolderJob`]) décidés et enregistrés **avant** la mise en file
//!   (`authoring::identity`) ; la tâche les garde (reprise au démarrage comprise) et ne réécrit
//!   rien dans le dossier.
//! - **Une tâche par uuid** : une création déjà en file pour cet uuid refuse la seconde.
//! - L'histoire publiée remplace la précédente de même uuid (`Library::install_replacing`) et
//!   sa source retient le dossier (`StoryEntry::download_url`, chemin résolu).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::watch;

use super::pack_build::{BuildSource, PackBuild, Prepared};
use super::podcast::{finish_workdir, workdir_path, PackBackend};
use super::{converter_engine, is_task_id, lock, InstallService, InstallTask, Produced, Toolchain};
use crate::authoring::build::{self as folder_build, scan_limits};
use crate::authoring::tree;
use crate::catalog::blocking;
use crate::library::Library;
use crate::podcast::fetch::Workdir;
use crate::story::StoryEntry;
use crate::{AppError, AppResult};

/// Création d'une histoire à partir d'un dossier, enregistrée avec la tâche.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderJob {
    /// Uuid de l'histoire (forme canonique), décidé avant la mise en file.
    pub uuid: String,
    /// Version publiée.
    pub version: u32,
    /// Dossier de l'histoire (forme résolue, revérifiée à la préparation).
    pub folder: String,
    /// Voix des titres lus (`titresLus.voix`, sinon celle du poste) ; `None` : voix par défaut.
    #[serde(default)]
    pub voice: Option<String>,
    /// Dossiers interdits en plus de ceux du système (lecteur de la Flam…), relus à la
    /// préparation avec le dossier du launcher.
    #[serde(default)]
    pub forbidden: Vec<String>,
}

impl FolderJob {
    /// Champs relus de la file : uuid canonique, version, chemin absolu.
    pub fn is_valid(&self) -> bool {
        is_task_id(&self.uuid) && self.version >= 1 && Path::new(&self.folder).is_absolute()
    }
}

/// Message d'un dossier devenu bloqué depuis le rapport.
fn blocked_message(problems: &[tree::Problem]) -> String {
    let first: Vec<String> = problems
        .iter()
        .filter(|p| p.severity == tree::Severity::Blocking)
        .take(3)
        .map(|p| p.message.clone())
        .collect();
    format!(
        "le dossier ne peut plus devenir une histoire : {}",
        first.join(" ; ")
    )
}

impl InstallService {
    /// Ajoute la tâche de création de l'histoire du dossier `job` ; `entry` : entrée à publier
    /// ([`folder_build::story_entry`]).
    pub fn start_folder(&self, job: FolderJob, entry: StoryEntry) -> AppResult<InstallTask> {
        if !job.is_valid() || job.uuid != entry.uuid {
            return Err(AppError::Validation("création d'histoire invalide".into()));
        }
        self.enqueue(
            uuid::Uuid::new_v4().to_string(),
            entry,
            None,
            Some(job),
            super::now_secs(),
            false,
        )
    }

    /// Vrai si une tâche active concerne déjà l'histoire `uuid`.
    pub fn has_active(&self, uuid: &str) -> bool {
        lock(&self.0.tasks)
            .iter()
            .any(|s| s.task.state.is_active() && s.task.entry.uuid == uuid)
    }

    /// Dossier du cache effectif (refusé comme dossier d'histoire).
    pub async fn cache_root(&self) -> AppResult<PathBuf> {
        Ok(self.toolchain().await?.cache_root)
    }

    /// Vrai si l'histoire `uuid` est déjà installée à la version `version`.
    pub(super) async fn folder_installed(&self, uuid: &str, version: u32) -> bool {
        let Ok(tc) = self.toolchain().await else {
            return false;
        };
        let uuid = uuid.to_string();
        blocking(move || {
            Library::new(&tc.cache_root).list().is_ok_and(|list| {
                list.iter()
                    .any(|e| e.uuid == uuid && e.version == Some(version))
            })
        })
        .await
        .unwrap_or(false)
    }

    /// Entrée d'une tâche « dossier » relue de la file : uuid et version de la tâche, textes
    /// de l'entrée enregistrée (bornés), dossier de la tâche.
    pub(super) fn restored_folder_entry(
        &self,
        job: &FolderJob,
        saved: &StoryEntry,
    ) -> AppResult<StoryEntry> {
        if !job.is_valid() {
            return Err(AppError::Validation("création d'histoire invalide".into()));
        }
        let max = crate::podcast::archive::MAX_TITLE_CHARS;
        let meta = crate::podcast::pack::ResolvedMetadata {
            title: saved.title.chars().take(max).collect(),
            subtitle: saved
                .category
                .as_deref()
                .unwrap_or_default()
                .chars()
                .take(max)
                .collect(),
            age: saved.age.and_then(|a| u8::try_from(a).ok()).unwrap_or(0),
            description: None,
        };
        Ok(folder_build::story_entry(
            &job.uuid,
            job.version,
            &meta,
            Path::new(&job.folder),
        ))
    }

    /// Préparation puis conversion de l'histoire du dossier vers `staged` ; dossier de travail
    /// supprimé quelle que soit l'issue. Rend l'entrée à publier (métadonnées relues).
    pub(super) async fn produce_folder(
        &self,
        id: &str,
        job: &FolderJob,
        tc: &Toolchain,
        staged: &Path,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<(StoryEntry, Produced)> {
        let prepared = match self.prepare_folder(id, job, tc, cancel).await {
            Ok(prepared) => prepared,
            Err(err) => {
                self.clear_pack_path(id);
                return Err(err);
            }
        };
        self.produce_prepared(id, prepared, tc, staged, cancel)
            .await
    }

    async fn prepare_folder(
        &self,
        id: &str,
        job: &FolderJob,
        tc: &Toolchain,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<Prepared> {
        // Une préparation à la fois, comme les packs (annulation prioritaire, #562).
        let _slot = tokio::select! {
            biased;
            _ = super::download::cancelled(cancel) => return Err(AppError::Cancelled),
            s = self.0.preparations.acquire() => s.map_err(|e| AppError::Io(std::io::Error::other(e.to_string())))?,
        };
        let dir = workdir_path(&tc.cache_root, id)?;
        let workdir = blocking(move || Workdir::at(dir)).await??;
        match self.prepare_folder_in(id, job, tc, &workdir, cancel).await {
            Ok((archive, entry)) => Ok(Prepared {
                workdir,
                archive,
                entry,
            }),
            Err(err) => {
                finish_workdir(workdir).await;
                Err(err)
            }
        }
    }

    async fn prepare_folder_in(
        &self,
        id: &str,
        job: &FolderJob,
        tc: &Toolchain,
        workdir: &Workdir,
        cancel: &mut watch::Receiver<bool>,
    ) -> AppResult<(PathBuf, StoryEntry)> {
        let backend: Arc<dyn PackBackend> = lock(&self.0.packs).clone().ok_or_else(|| {
            AppError::ConverterUnavailable("création d'histoire indisponible".into())
        })?;
        converter_engine(tc)?;
        backend.ready(cancel).await?;
        self.emit_phase(id, 0.0, "validation");
        // Dossier relu : il a pu changer depuis le rapport.
        let folder = PathBuf::from(&job.folder);
        let mut forbidden: Vec<PathBuf> = job.forbidden.iter().map(PathBuf::from).collect();
        forbidden.push(tc.cache_root.clone());
        let scan = blocking(move || tree::scan(&folder, &scan_limits(), &forbidden)).await??;
        let mut problems = scan.problems.clone();
        problems.extend(folder_build::book_problems(&scan));
        if problems
            .iter()
            .any(|p| p.severity == tree::Severity::Blocking)
        {
            return Err(AppError::Validation(blocked_message(&problems)));
        }
        if *cancel.borrow() {
            return Err(AppError::Cancelled);
        }
        let plan = folder_build::plan(&scan, &job.uuid);
        let meta = plan.spec.metadata.clone();
        let voice = match job.voice.clone() {
            Some(voice) => Some(voice),
            None => {
                let backend = backend.clone();
                blocking(move || {
                    backend
                        .preferred_voice()
                        .and_then(|v| crate::settings::clean_voice_id(Some(v)).ok().flatten())
                })
                .await?
            }
        };
        let stage = PackBuild {
            service: self,
            id,
            workdir: workdir.path(),
            backend: &backend,
            cancel,
            source: BuildSource::Folder,
        };
        let archive = folder_build::prepare(&stage, plan, voice.as_deref()).await?;
        let entry = folder_build::story_entry(&job.uuid, job.version, &meta, &scan.base);
        Ok((archive, entry))
    }
}
