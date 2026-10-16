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
    /// Version de `telmi2flam` utilisée (`sha256:<12 hex>` du script), `None` sans conversion.
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

    /// Histoires installées dont le fichier existe encore (une entrée dont le fichier a
    /// disparu est ignorée), triées par titre.
    pub fn list(&self) -> AppResult<Vec<LibraryEntry>> {
        let mut stories: Vec<LibraryEntry> = self
            .load()?
            .into_iter()
            .filter(|e| self.checked(&e.file).is_ok_and(|p| p.is_file()))
            .collect();
        stories.sort_by_cached_key(|e| (e.title.to_lowercase(), e.uuid.clone()));
        Ok(stories)
    }

    /// Chemin et titre de l'histoire installée `uuid`, relus de l'index et revérifiés (fichier
    /// directement sous `library/`, existant). `not_found` si elle n'est pas installée,
    /// `story_file_missing` si son fichier a disparu.
    pub fn playable(&self, uuid: &str) -> AppResult<(PathBuf, String)> {
        let entry = self
            .load()?
            .into_iter()
            .find(|e| e.uuid == uuid)
            .ok_or_else(|| AppError::NotFound(format!("histoire non installée : {uuid}")))?;
        let path = self.checked(&entry.file)?;
        if !path.is_file() {
            return Err(AppError::StoryFileMissing(format!(
                "le fichier de « {} » a disparu de la bibliothèque",
                entry.title
            )));
        }
        Ok((path, entry.title))
    }

    /// Publie `staged` (un `.plain.pk` complet) comme installation de `source` : renommage
    /// atomique vers `<uuid>-<version>.plain.pk`, mise à jour de l'index, suppression de
    /// l'ancienne installation du même `uuid` si son fichier diffère.
    pub fn install(
        &self,
        staged: &Path,
        source: &StoryEntry,
        converter: Option<String>,
        now: u64,
    ) -> AppResult<LibraryEntry> {
        let target = self.final_path(&source.uuid, source.version)?;
        let file = file_name_for(&source.uuid, source.version)?;
        let mut stories = self.load()?;
        std::fs::rename(staged, &target)?;
        let size = std::fs::metadata(&target)?.len();
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
        for old in stories.iter().filter(|e| e.uuid == entry.uuid) {
            if old.file != file {
                if let Ok(path) = self.checked(&old.file) {
                    remove_if_exists(&path)?;
                }
            }
        }
        stories.retain(|e| e.uuid != entry.uuid && e.file != file);
        stories.push(entry.clone());
        self.save(stories)?;
        Ok(entry)
    }

    /// Désinstalle l'histoire `uuid` (fichier et entrée d'index). Inconnue : `not_found`.
    pub fn remove(&self, uuid: &str) -> AppResult<()> {
        let mut stories = self.load()?;
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
    fn playable_donne_chemin_et_titre_de_l_index() {
        let dir = tempfile::tempdir().unwrap();
        let lib = Library::new(dir.path());
        assert_eq!(lib.playable("abc").unwrap_err().kind(), "not_found");
        let s = staged(&lib, "t.plain.pk", b"PK1234");
        lib.install(&s, &story("abc", Some(3)), None, 7).unwrap();
        let (path, title) = lib.playable("abc").unwrap();
        assert_eq!(path, dir.path().join("library/abc-3.plain.pk"));
        assert_eq!(title, "Histoire abc");
        // Un uuid piégé n'est jamais traduit en chemin : il n'est pas dans l'index.
        assert_eq!(
            lib.playable("..\\..\\evil").unwrap_err().kind(),
            "not_found"
        );

        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            lib.playable("abc").unwrap_err().kind(),
            "story_file_missing"
        );
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
