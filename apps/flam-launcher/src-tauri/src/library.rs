//! Bibliothèque des histoires installées : `<cache>/library/<uuid>-<version>.plain.pk` et son
//! index `<cache>/library/library.json`.
//!
//! **Une installation par `uuid`** : installer une histoire dont l'`uuid` est déjà présent
//! remplace l'installation précédente (autre version, ou autre archive de même `uuid`). C'est
//! la règle des doublons : un `.plain` et un `.plain.pk` de même `uuid` dans une source dossier
//! donnent deux entrées côté source, mais **la dernière installée** est celle de la
//! bibliothèque.
//!
//! `uuid` et `version` proviennent d'archives et de catalogues non fiables : ils sont
//! **revalidés** ici ([`validate_story_id`]) avant de construire un chemin, et le chemin final
//! doit rester directement sous `library/`.
//!
//! La logique ([`Library`]) ne dépend pas de Tauri ; les commandes `library_*` passent par le
//! service d'installation, qui sérialise les écritures avec le pipeline.

use std::fs::File;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::install::InstallService;
use crate::storage::{self, Loaded};
use crate::story::{validate_story_id, StoryEntry};
use crate::{AppError, AppResult};

/// Sous-dossier du cache (listé dans `cache::APP_CACHE_SUBDIRS`).
pub const LIBRARY_DIR: &str = "library";
/// Index de la bibliothèque, dans `library/`.
pub const INDEX_FILE: &str = "library.json";
/// Dossier de préparation (sous `library/`, même volume : publication par renommage).
pub const STAGING_DIR: &str = ".staging";
/// Suffixe de la sauvegarde d'un fichier remplacé pendant une installation.
const BACKUP_SUFFIX: &str = ".bak";
/// Suffixe du marqueur d'intention d'une publication en cours.
const PENDING_SUFFIX: &str = ".pending";
/// Verrou inter-processus de la bibliothèque, dans `library/`.
const LOCK_FILE: &str = ".lock";
/// Version du schéma de `library.json`.
pub const SCHEMA_VERSION: u32 = 1;

/// Histoire installée (miroir de `LibraryEntry` dans `src/api/library.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryEntry {
    /// Identifiant de l'histoire (celui de la source : `uuid` Telmi pour un zip Telmi).
    pub uuid: String,
    /// Version installée (`None` : inconnue, fichier `<uuid>-0.plain.pk`).
    pub version: Option<u32>,
    pub title: String,
    /// Nom du fichier sous `library/` (`<uuid>-<version>.plain.pk`).
    pub file: String,
    /// Taille du `.plain.pk` en octets.
    pub size: u64,
    /// Date d'installation (secondes Unix).
    pub installed_at: u64,
    /// Version de `telmi2flam` utilisée (champ `version` de l'événement `start`, sinon repli
    /// `sha256:<12 hex>` du script), `None` sans conversion.
    pub converter: Option<String>,
    /// Entrée d'origine telle que proposée par la source (source, lien, métadonnées).
    pub source: StoryEntry,
}

impl LibraryEntry {
    /// Vrai si `candidate` (même `uuid`) propose une autre version que celle installée.
    /// Une version inconnue d'un côté ou de l'autre n'est jamais une mise à jour.
    pub fn is_outdated_by(&self, candidate: &StoryEntry) -> bool {
        candidate.uuid == self.uuid
            && matches!((self.version, candidate.version), (Some(a), Some(b)) if a != b)
    }
}

/// Marqueur d'intention d'une publication (`.staging/<fichier>.pending`).
#[derive(Debug, Serialize, Deserialize)]
struct PendingMarker {
    /// Date d'installation de la nouvelle entrée (la retrouve dans l'index si validée).
    installed_at: u64,
    /// Un fichier de même nom existait (il est alors en `.bak` dès le premier renommage).
    had_previous: bool,
}

/// Contenu de `library.json`.
#[derive(Debug, Serialize, Deserialize)]
struct IndexFile {
    schema_version: u32,
    stories: Vec<LibraryEntry>,
}

/// Nom de fichier `<uuid>-<version>.plain.pk` après revalidation de l'`uuid` (motif
/// `^[0-9A-Za-z][0-9A-Za-z._-]{0,63}$`, sans `..`, hors noms réservés Windows).
pub fn file_name_for(uuid: &str, version: Option<u32>) -> AppResult<String> {
    if !validate_story_id(uuid) {
        return Err(AppError::Validation(format!(
            "identifiant d'histoire refusé : {uuid:?}"
        )));
    }
    Ok(format!("{uuid}-{}.plain.pk", version.unwrap_or(0)))
}

/// Accès à la bibliothèque d'un dossier de cache. L'appelant sérialise les écritures (verrou
/// du service d'installation).
pub struct Library {
    root: PathBuf,
}

impl Library {
    /// Bibliothèque de `<cache_root>/library`.
    pub fn new(cache_root: &Path) -> Self {
        Self {
            root: cache_root.join(LIBRARY_DIR),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Dossier de préparation, créé si besoin.
    pub fn staging_dir(&self) -> AppResult<PathBuf> {
        let dir = self.root.join(STAGING_DIR);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Chemin final d'une histoire, vérifié : directement sous `library/`.
    fn final_path(&self, uuid: &str, version: Option<u32>) -> AppResult<PathBuf> {
        let name = file_name_for(uuid, version)?;
        self.checked(&name)
    }

    /// `name` doit être un simple nom de fichier `.plain.pk` sous `library/`.
    fn checked(&self, name: &str) -> AppResult<PathBuf> {
        let path = self.root.join(name);
        let direct = path.parent() == Some(self.root.as_path())
            && path.file_name().and_then(|n| n.to_str()) == Some(name)
            && name.ends_with(".plain.pk");
        if !direct {
            return Err(AppError::Validation(format!(
                "chemin hors de la bibliothèque : {name:?}"
            )));
        }
        Ok(path)
    }

    /// Lit l'index. Absent : vide. Corrompu : sauvegardé en `*.bak`, vide (avertissement).
    /// Version future : erreur, fichier intact.
    fn load(&self) -> AppResult<Vec<LibraryEntry>> {
        let path = self.root.join(INDEX_FILE);
        let loaded = storage::load_versioned::<IndexFile>(&path, SCHEMA_VERSION, |_, _| {})?;
        Ok(match loaded {
            Loaded::Found(index) => index.stories,
            Loaded::Missing | Loaded::Recovered { .. } => Vec::new(),
        })
    }

    fn save(&self, stories: Vec<LibraryEntry>) -> AppResult<()> {
        storage::write_json(
            &self.root.join(INDEX_FILE),
            &IndexFile {
                schema_version: SCHEMA_VERSION,
                stories,
            },
        )
    }

    /// Verrou entre processus (`library/.lock`), pris pendant publication et réconciliation.
    /// `wait` : attendre le verrou ; sinon `Ok(None)` s'il est tenu par une autre instance.
    fn lock_file(&self, wait: bool) -> AppResult<Option<File>> {
        std::fs::create_dir_all(&self.root)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(self.root.join(LOCK_FILE))?;
        if wait {
            file.lock()?;
            return Ok(Some(file));
        }
        match file.try_lock() {
            Ok(()) => Ok(Some(file)),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(err)) => Err(err.into()),
        }
    }

    /// Lit l'index et, si le verrou inter-processus est obtenu, défait les publications
    /// interrompues. Si une autre instance tient le verrou, rien n'est réconcilié.
    fn load_reconciled(&self) -> AppResult<Vec<LibraryEntry>> {
        match self.lock_file(false) {
            Ok(Some(_guard)) => self.reconcile(),
            Ok(None) => self.load(),
            Err(err) => {
                tracing::warn!("verrou de la bibliothèque indisponible, lecture seule : {err}");
                self.load()
            }
        }
    }

    /// Réconciliation (verrou tenu par l'appelant). **Seules** les publications qui ont un
    /// marqueur d'intention `.staging/<fichier>.pending` sont défaites : un `.plain.pk` non
    /// indexé sans marqueur n'est jamais supprimé. Pour un marqueur dont l'index ne contient pas
    /// la nouvelle entrée (même `installed_at`) : le fichier publié est retiré et la sauvegarde
    /// `.bak` restaurée ; sinon la publication était terminée et seuls les résidus partent.
    fn reconcile(&self) -> AppResult<Vec<LibraryEntry>> {
        let stories = self.load()?;
        let Ok(read) = std::fs::read_dir(self.root.join(STAGING_DIR)) else {
            return Ok(stories);
        };
        for item in read.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(file) = name.strip_suffix(PENDING_SUFFIX) else {
                continue;
            };
            let Ok(target) = self.checked(file) else {
                continue;
            };
            let Some(marker) = std::fs::read(item.path())
                .ok()
                .and_then(|bytes| serde_json::from_slice::<PendingMarker>(&bytes).ok())
            else {
                let _ = remove_if_exists(&item.path());
                continue;
            };
            let backup = item.path().with_file_name(format!("{file}{BACKUP_SUFFIX}"));
            let committed = stories
                .iter()
                .any(|e| e.file == file && e.installed_at == marker.installed_at);
            if committed {
                let _ = remove_if_exists(&backup);
                let _ = remove_if_exists(&item.path());
            } else if restore_publication(&target, &backup, &item.path(), marker.had_previous) {
                let _ = remove_if_exists(&item.path());
            }
        }
        Ok(stories)
    }

    /// Histoires installées dont le fichier existe encore (une entrée dont le fichier a
    /// disparu est ignorée), triées par titre.
    pub fn list(&self) -> AppResult<Vec<LibraryEntry>> {
        let mut stories: Vec<LibraryEntry> = self
            .load_reconciled()?
            .into_iter()
            .filter(|e| self.checked(&e.file).is_ok_and(|p| p.is_file()))
            .collect();
        stories.sort_by_cached_key(|e| (e.title.to_lowercase(), e.uuid.clone()));
        Ok(stories)
    }

    /// Publie `staged` (un `.plain.pk` complet) comme installation de `source`.
    ///
    /// Ordre sûr en cas d'arrêt ou d'échec : l'éventuel fichier de même nom est d'abord mis de
    /// côté (`.staging/<fichier>.bak`), le nouveau est renommé en place, l'index est écrit, puis
    /// la sauvegarde et les fichiers des anciennes versions du même `uuid` sont supprimés. Un
    /// échec avant la fin remet l'état précédent ; un arrêt brutal est réconcilié au chargement
    /// suivant ([`Library::load_reconciled`]).
    pub fn install(
        &self,
        staged: &Path,
        source: &StoryEntry,
        converter: Option<String>,
        now: u64,
    ) -> AppResult<LibraryEntry> {
        self.install_with(staged, source, converter, now, &|stories| {
            self.save(stories)
        })
    }

    fn install_with(
        &self,
        staged: &Path,
        source: &StoryEntry,
        converter: Option<String>,
        now: u64,
        save: &dyn Fn(Vec<LibraryEntry>) -> AppResult<()>,
    ) -> AppResult<LibraryEntry> {
        let target = self.final_path(&source.uuid, source.version)?;
        let file = file_name_for(&source.uuid, source.version)?;
        let _lock = self.lock_file(true)?;
        let mut stories = self.reconcile()?;
        let staging = self.staging_dir()?;
        let backup = staging.join(format!("{file}{BACKUP_SUFFIX}"));
        let pending = staging.join(format!("{file}{PENDING_SUFFIX}"));
        // Une restauration restée en échec (marqueur ou `.bak` survivant à la réconciliation)
        // ne doit pas être écrasée : on refuse sans rien toucher.
        if pending.exists() || backup.exists() {
            return Err(AppError::Validation(format!(
                "restauration précédente non terminée pour {file}, réessayer plus tard"
            )));
        }
        let had_previous = target.is_file();
        // Journal d'intention : écrit avant tout renommage, effacé une fois l'index écrit.
        storage::write_json(
            &pending,
            &PendingMarker {
                installed_at: now,
                had_previous,
            },
        )?;
        if had_previous {
            if let Err(err) = std::fs::rename(&target, &backup) {
                let _ = remove_if_exists(&pending);
                return Err(err.into());
            }
        }
        let restore = || {
            restore_publication(&target, &backup, &pending, had_previous);
        };
        let size = match std::fs::rename(staged, &target).and_then(|()| std::fs::metadata(&target))
        {
            Ok(meta) => meta.len(),
            Err(err) => {
                restore();
                return Err(err.into());
            }
        };
        let entry = LibraryEntry {
            uuid: source.uuid.clone(),
            version: source.version,
            title: source.title.clone(),
            file: file.clone(),
            size,
            installed_at: now,
            converter,
            source: source.clone(),
        };
        let old_files: Vec<String> = stories
            .iter()
            .filter(|e| e.uuid == entry.uuid && e.file != file)
            .map(|e| e.file.clone())
            .collect();
        stories.retain(|e| e.uuid != entry.uuid && e.file != file);
        stories.push(entry.clone());
        if let Err(err) = save(stories) {
            restore();
            return Err(err);
        }
        // L'index est à jour : le reste est du nettoyage.
        let _ = remove_if_exists(&pending);
        let _ = remove_if_exists(&backup);
        for old in old_files {
            if let Ok(path) = self.checked(&old) {
                let _ = remove_if_exists(&path);
            }
        }
        Ok(entry)
    }

    /// Désinstalle l'histoire `uuid` (fichier et entrée d'index). Inconnue : `not_found`.
    pub fn remove(&self, uuid: &str) -> AppResult<()> {
        let _lock = self.lock_file(true)?;
        let mut stories = self.reconcile()?;
        let Some(pos) = stories.iter().position(|e| e.uuid == uuid) else {
            return Err(AppError::NotFound(format!(
                "histoire non installée : {uuid}"
            )));
        };
        let entry = stories.remove(pos);
        if let Ok(path) = self.checked(&entry.file) {
            remove_if_exists(&path)?;
        }
        self.save(stories)
    }
}

/// Défait une publication : remet la sauvegarde `.bak` en place (le `rename` remplace la cible,
/// qui n'est donc jamais supprimée avant), ou retire le fichier publié s'il n'y avait pas
/// d'ancienne version. Le marqueur `pending` n'est effacé **que si la restauration a réussi** ;
/// sinon marqueur et `.bak` restent pour la réconciliation suivante. Renvoie `true` si réussie.
fn restore_publication(target: &Path, backup: &Path, pending: &Path, had_previous: bool) -> bool {
    let restored = if backup.is_file() {
        std::fs::rename(backup, target)
    } else if had_previous {
        // Sauvegarde absente : rien à restaurer (déjà remise en place).
        Ok(())
    } else {
        remove_if_exists(target)
    };
    if let Err(err) = restored {
        tracing::warn!(
            "restauration de {} impossible, marqueur et sauvegarde conservés : {err}",
            target.display()
        );
        return false;
    }
    if let Err(err) = remove_if_exists(pending) {
        tracing::warn!("marqueur {} non effacé : {err}", pending.display());
    }
    true
}

/// Supprime un fichier ; absent n'est pas une erreur.
pub fn remove_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    }
}

/// Histoires installées (triées par titre).
#[tauri::command]
pub async fn library_list(service: State<'_, InstallService>) -> AppResult<Vec<LibraryEntry>> {
    service.library_list().await
}

/// Désinstalle l'histoire `uuid` (fichier et entrée d'index).
#[tauri::command]
pub async fn library_remove(service: State<'_, InstallService>, uuid: String) -> AppResult<()> {
    service.library_remove(uuid).await
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::story::StoryKind;

    pub(crate) fn story(uuid: &str, version: Option<u32>) -> StoryEntry {
        StoryEntry {
            source_id: "src".into(),
            entry_id: format!("src:{uuid}"),
            uuid: uuid.into(),
            version,
            title: format!("Histoire {uuid}"),
            description: None,
            age: None,
            category: None,
            author: None,
            voice: None,
            license: None,
            thumb_url: None,
            download_url: "https://h.example.test/a.zip".into(),
            size: None,
            updated_at: None,
            kind: StoryKind::TelmiZip,
        }
    }

    fn staged(lib: &Library, name: &str, bytes: &[u8]) -> PathBuf {
        let path = lib.staging_dir().unwrap().join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn installation_puis_liste_puis_suppression() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        assert!(lib.list().unwrap().is_empty());
        let s = staged(&lib, "t.plain.pk", b"PK1234");
        let entry = lib
            .install(&s, &story("abc", Some(3)), Some("sha256:x".into()), 7)
            .unwrap();
        assert_eq!(entry.file, "abc-3.plain.pk");
        assert_eq!(entry.size, 6);
        assert!(dir.path().join("library/abc-3.plain.pk").is_file());
        assert!(!s.exists());
        assert_eq!(lib.list().unwrap(), vec![entry]);

        lib.remove("abc").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert!(!dir.path().join("library/abc-3.plain.pk").exists());
        assert_eq!(lib.remove("abc").unwrap_err().kind(), "not_found");
    }

    #[test]
    fn une_seule_installation_par_uuid_la_derniere_gagne() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"v1");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let b = staged(&lib, "b.plain.pk", b"v2!");
        lib.install(&b, &story("abc", Some(2)), None, 2).unwrap();
        let list = lib.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].version, Some(2));
        assert!(!dir.path().join("library/abc-1.plain.pk").exists());
        // Réinstallation de la même version : remplacée sur place.
        let c = staged(&lib, "c.plain.pk", b"v2bis");
        lib.install(&c, &story("abc", Some(2)), None, 3).unwrap();
        let list = lib.list().unwrap();
        assert_eq!((list.len(), list[0].size), (1, 5));
    }

    #[test]
    fn uuid_dangereux_refuse_avant_tout_chemin() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        for uuid in ["../x", "a/b", r"a\b", "CON", "..", "", "a.", "nul.txt"] {
            let s = staged(&lib, "t.plain.pk", b"x");
            let err = lib.install(&s, &story(uuid, Some(1)), None, 0).unwrap_err();
            assert_eq!(err.kind(), "validation", "{uuid:?}");
        }
        assert!(file_name_for("abc", None).unwrap() == "abc-0.plain.pk");
    }

    #[test]
    fn une_entree_d_index_hors_bibliotheque_est_ignoree() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged(&lib, "t.plain.pk", b"x");
        let mut entry = lib.install(&s, &story("abc", Some(1)), None, 0).unwrap();
        std::fs::write(dir.path().join("secret.plain.pk"), b"x").unwrap();
        entry.uuid = "zzz".into();
        entry.file = "../secret.plain.pk".into();
        let mut stories = lib.load().unwrap();
        stories.push(entry);
        lib.save(stories).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        lib.remove("zzz").unwrap();
        assert!(dir.path().join("secret.plain.pk").exists());
    }

    #[test]
    fn index_corrompu_sauvegarde_et_vide() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        std::fs::create_dir_all(lib.root()).unwrap();
        std::fs::write(lib.root().join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        let backups = std::fs::read_dir(lib.root())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".bak")
            })
            .count();
        assert_eq!(backups, 1);
    }

    #[test]
    fn echec_d_ecriture_de_l_index_restaure_l_etat_precedent() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let fail = |_: Vec<LibraryEntry>| Err(AppError::Io(std::io::Error::other("disque plein")));
        let b = staged(&lib, "b.plain.pk", b"nouveau!");
        let err = lib
            .install_with(&b, &story("abc", Some(2)), None, 2, &fail)
            .unwrap_err();
        assert_eq!(err.kind(), "io");
        let path = dir.path().join("library/abc-2.plain.pk");
        assert_eq!(std::fs::read(&path).unwrap(), b"ancien");
        let list = lib.list().unwrap();
        assert_eq!((list.len(), list[0].size), (1, 6));
        // Nouvelle histoire : rien ne reste publié.
        let c = staged(&lib, "c.plain.pk", b"x");
        lib.install_with(&c, &story("neuf", Some(1)), None, 3, &fail)
            .unwrap_err();
        assert!(!dir.path().join("library/neuf-1.plain.pk").exists());
    }

    fn marker(root: &Path, file: &str, installed_at: u64, had_previous: bool) {
        storage::write_json(
            &root.join(format!(".staging/{file}.pending")),
            &PendingMarker {
                installed_at,
                had_previous,
            },
        )
        .unwrap();
    }

    #[test]
    fn fichier_non_indexe_prexistant_conserve() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"x");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let root = dir.path().join("library");
        std::fs::write(root.join("perso.plain.pk"), b"a moi").unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(root.join("perso.plain.pk")).unwrap(),
            b"a moi"
        );
        // Index valide et vide : rien n'est supprimé non plus.
        lib.remove("abc").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert!(root.join("perso.plain.pk").exists());
    }

    #[test]
    fn publication_interrompue_defaite_grace_au_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let root = dir.path().join("library");
        // Arrêt simulé : ancien en sauvegarde, nouveau publié, index inchangé, marqueur présent.
        marker(&root, "abc-2.plain.pk", 2, true);
        std::fs::rename(
            root.join("abc-2.plain.pk"),
            root.join(".staging/abc-2.plain.pk.bak"),
        )
        .unwrap();
        std::fs::write(root.join("abc-2.plain.pk"), b"nouveau, pas indexe").unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"ancien"
        );
        assert!(!root.join(".staging/abc-2.plain.pk.bak").exists());
        assert!(!root.join(".staging/abc-2.plain.pk.pending").exists());
        // Nouvelle histoire interrompue : le fichier publié est retiré.
        marker(&root, "neuf-1.plain.pk", 3, false);
        std::fs::write(root.join("neuf-1.plain.pk"), b"x").unwrap();
        lib.list().unwrap();
        assert!(!root.join("neuf-1.plain.pk").exists());
        // Publication terminée (index à jour) mais marqueur resté : le fichier est conservé.
        marker(&root, "abc-2.plain.pk", 1, true);
        lib.list().unwrap();
        assert_eq!(
            std::fs::read(root.join("abc-2.plain.pk")).unwrap(),
            b"ancien"
        );
        assert!(!root.join(".staging/abc-2.plain.pk.pending").exists());
        // Un index illisible ne fait rien supprimer.
        std::fs::write(root.join(INDEX_FILE), b"{pas du json").unwrap();
        assert!(lib.list().unwrap().is_empty());
        assert!(root.join("abc-2.plain.pk").exists());
    }

    #[test]
    fn restauration_en_echec_garde_marqueur_et_sauvegarde() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"ancien");
        lib.install(&a, &story("abc", Some(2)), None, 1).unwrap();
        let root = dir.path().join("library");
        let target = root.join("abc-2.plain.pk");
        let backup = root.join(".staging/abc-2.plain.pk.bak");
        let pending = root.join(".staging/abc-2.plain.pk.pending");
        // La cible devient un dossier non vide : le rename de la sauvegarde échoue.
        let blocked = |_: Vec<LibraryEntry>| -> AppResult<()> {
            std::fs::remove_file(&target)?;
            std::fs::create_dir(&target)?;
            std::fs::write(target.join("x"), b"x")?;
            Err(AppError::Io(std::io::Error::other("disque plein")))
        };
        let b = staged(&lib, "b.plain.pk", b"nouveau!");
        lib.install_with(&b, &story("abc", Some(2)), None, 2, &blocked)
            .unwrap_err();
        assert!(pending.exists() && backup.is_file(), "journal conservé");
        // Une nouvelle installation du même fichier est refusée, sans rien toucher.
        let c = staged(&lib, "c.plain.pk", b"autre");
        let err = lib
            .install(&c, &story("abc", Some(2)), None, 3)
            .unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{err}");
        assert!(pending.exists() && backup.is_file(), "journal intact");
        assert_eq!(std::fs::read(&backup).unwrap(), b"ancien");
        assert!(c.exists(), "archive préparée non consommée");
        // La réconciliation retente tant que la cible est bloquée (entrée masquée : pas un fichier).
        assert!(lib.list().unwrap().is_empty());
        assert!(pending.exists() && backup.is_file());
        // Cible libérée : la réconciliation suivante restaure l'ancienne version.
        std::fs::remove_dir_all(&target).unwrap();
        assert_eq!(lib.list().unwrap().len(), 1);
        assert_eq!(std::fs::read(&target).unwrap(), b"ancien");
        assert!(!pending.exists() && !backup.exists());
    }

    #[test]
    fn restauration_sans_ancienne_version_en_echec_garde_le_marqueur() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let root = dir.path().join("library");
        lib.list().unwrap();
        lib.staging_dir().unwrap();
        let target = root.join("neuf-1.plain.pk");
        marker(&root, "neuf-1.plain.pk", 3, false);
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("x"), b"x").unwrap();
        lib.list().unwrap();
        assert!(root.join(".staging/neuf-1.plain.pk.pending").exists());
        std::fs::remove_dir_all(&target).unwrap();
        std::fs::write(&target, b"x").unwrap();
        lib.list().unwrap();
        assert!(!target.exists());
        assert!(!root.join(".staging/neuf-1.plain.pk.pending").exists());
    }

    #[test]
    fn lecture_sans_verrou_disponible_retombe_sur_l_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let a = staged(&lib, "a.plain.pk", b"x");
        lib.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        // `.lock` devient un dossier : l'ouverture du verrou échoue.
        let lock = dir.path().join("library").join(LOCK_FILE);
        std::fs::remove_file(&lock).unwrap();
        std::fs::create_dir(&lock).unwrap();
        assert!(lib.lock_file(false).is_err());
        assert_eq!(lib.list().unwrap().len(), 1);
        assert!(lib.install(&a, &story("abc", Some(1)), None, 2).is_err());
    }

    #[test]
    fn une_autre_instance_ne_reconcilie_pas_pendant_une_publication() {
        let dir = tempfile::tempdir().unwrap();
        let first = Library::new(dir.path());
        let second = Library::new(dir.path());
        let a = staged(&first, "a.plain.pk", b"x");
        first.install(&a, &story("abc", Some(1)), None, 1).unwrap();
        let root = dir.path().join("library");
        marker(&root, "neuf-1.plain.pk", 9, false);
        std::fs::write(root.join("neuf-1.plain.pk"), b"en cours").unwrap();
        {
            let _held = first.lock_file(true).unwrap().expect("verrou");
            assert!(second.lock_file(false).unwrap().is_none());
            assert_eq!(second.list().unwrap().len(), 1);
            assert!(root.join("neuf-1.plain.pk").exists(), "non réconcilié");
        }
        second.list().unwrap();
        assert!(!root.join("neuf-1.plain.pk").exists());
    }

    #[test]
    fn detection_des_mises_a_jour() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        let s = staged(&lib, "t.plain.pk", b"x");
        let installed = lib.install(&s, &story("abc", Some(2)), None, 0).unwrap();
        assert!(installed.is_outdated_by(&story("abc", Some(3))));
        assert!(!installed.is_outdated_by(&story("abc", Some(2))));
        assert!(!installed.is_outdated_by(&story("abc", None)));
        assert!(!installed.is_outdated_by(&story("autre", Some(3))));
    }
}
