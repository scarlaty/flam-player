//! Registre des sources d'histoires : `sources.json` (catalogues, dossiers locaux, flux
//! podcast), source par défaut « Telmi Store » et source implicite « Mes histoires ».
//!
//! L'URL d'un flux podcast peut porter un jeton (D11, `docs/podcasts.md`, décidé le
//! 2026-10-23) : elle est affichée en clair dans l'interface, mais n'est jamais journalisée ni
//! recopiée dans un message d'erreur ou le diagnostic. Le front reçoit une [`PublicSource`],
//! qui ajoute un libellé court (`feed_hint`, `https://hôte/…`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::catalog::{purge_catalog_cache, CatalogService};
use crate::podcast::net::redact_url;
use crate::podcast::source::{purge_podcast_cache, PodcastService};
use crate::storage::{self, ConfigState, MigrationStep, Recovery};
use crate::watch::WatchState;
use crate::{AppError, AppResult};

/// Version courante du schéma de `sources.json`.
///
/// - v1 : catalogues et dossiers ;
/// - v2 (#211) : ajout des sources podcast. Aucune donnée ne change, mais la version monte :
///   un launcher v1 refuse ainsi un fichier v2 (`config_version`, fichier intact) au lieu de
///   le mettre entièrement en quarantaine faute de connaître la variante `podcast`. Un fichier
///   sans podcast est toujours écrit en v1 : sans podcast, revenir à un launcher v1 reste
///   possible.
pub const SCHEMA_VERSION: u32 = 2;
const FILE_NAME: &str = "sources.json";

/// URL du catalogue Telmi Store (source par défaut).
pub const TELMI_STORE_URL: &str = "https://gist.githubusercontent.com/DantSu/49ed776755f3a01c995e78e3fd1cb79f/raw/telmi-interactive.json";
/// Identifiant stable de la source Telmi Store créée par défaut.
pub const TELMI_STORE_ID: &str = "telmi-store";
/// Identifiant de la source implicite « Mes histoires » (jamais écrite dans `sources.json`).
pub const LIBRARY_ID: &str = "library";
const LIBRARY_NAME: &str = "Mes histoires";
pub(crate) const MAX_NAME_LEN: usize = 100;
const MAX_URL_LEN: usize = 2048;

/// Une source d'histoires (miroir de `Source` dans `src/api/sources.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Source {
    /// Bibliothèque des histoires installées : implicite, première, non modifiable.
    Library { id: String, name: String },
    /// Catalogue JSON au format Telmi Store.
    Catalog {
        id: String,
        name: String,
        url: String,
    },
    /// Dossier local d'archives (`.zip`, `.plain`, `.plain.pk`).
    Folder {
        id: String,
        name: String,
        path: String,
    },
    /// Flux podcast RSS (#211). `url` est affichée dans l'interface mais jamais journalisée
    /// (D11) : ne pas formater cette valeur avec `{:?}` dans une trace.
    Podcast {
        id: String,
        name: String,
        url: String,
    },
}

impl Source {
    pub fn id(&self) -> &str {
        match self {
            Source::Library { id, .. }
            | Source::Catalog { id, .. }
            | Source::Folder { id, .. }
            | Source::Podcast { id, .. } => id,
        }
    }

    fn library() -> Self {
        Source::Library {
            id: LIBRARY_ID.into(),
            name: LIBRARY_NAME.into(),
        }
    }

    fn telmi_store() -> Self {
        Source::Catalog {
            id: TELMI_STORE_ID.into(),
            name: "Telmi Store".into(),
            url: TELMI_STORE_URL.into(),
        }
    }
}

/// Données d'une source à créer ou à mettre à jour (sans identifiant).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SourceInput {
    Catalog { name: String, url: String },
    Folder { name: String, path: String },
}

/// Source telle que renvoyée au front (miroir de `Source` dans `src/api/sources.ts`) : comme
/// [`Source`], avec en plus, pour un flux podcast, un libellé court `feed_hint` (`https://hôte/…`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PublicSource {
    Library {
        id: String,
        name: String,
    },
    Catalog {
        id: String,
        name: String,
        url: String,
    },
    Folder {
        id: String,
        name: String,
        path: String,
    },
    Podcast {
        id: String,
        name: String,
        /// URL complète, affichée dans l'interface (D11, décidé le 2026-10-23).
        url: String,
        /// Libellé court : schéma et hôte.
        feed_hint: String,
    },
}

impl From<Source> for PublicSource {
    fn from(source: Source) -> Self {
        match source {
            Source::Library { id, name } => PublicSource::Library { id, name },
            Source::Catalog { id, name, url } => PublicSource::Catalog { id, name, url },
            Source::Folder { id, name, path } => PublicSource::Folder { id, name, path },
            Source::Podcast { id, name, url } => PublicSource::Podcast {
                id,
                name,
                feed_hint: redact_url(&url),
                url,
            },
        }
    }
}

fn public(sources: Vec<Source>) -> Vec<PublicSource> {
    sources.into_iter().map(PublicSource::from).collect()
}

/// Réponse de `sources_list` (miroir de `SourcesView` dans `src/api/sources.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourcesView {
    pub sources: Vec<PublicSource>,
    /// Présent tant que l'utilisateur n'a pas acquitté (`config_ack_recovery`) la
    /// restauration de `sources.json` aux valeurs par défaut.
    pub recovery: Option<Recovery>,
}

/// Contenu de `sources.json`.
#[derive(Debug, Serialize, Deserialize)]
struct SourcesFile {
    schema_version: u32,
    sources: Vec<Source>,
}

/// Forme écrite de `sources.json` (version choisie à l'écriture, voir `SourcesStore::save`).
#[derive(Serialize)]
struct SourcesFileRef<'a> {
    schema_version: u32,
    sources: &'a [Source],
}

impl Default for SourcesFile {
    fn default() -> Self {
        Self {
            // Sans podcast, le fichier reste en v1 (voir `SourcesStore::save`).
            schema_version: 1,
            sources: vec![Source::telmi_store()],
        }
    }
}

/// Migrations de schéma de `sources.json`. Aucune ne transforme de donnée : v0 -> v1 ajoute
/// seulement `schema_version`, v1 -> v2 ajoute le type `podcast` (voir [`SCHEMA_VERSION`]).
fn migrate(_from: u32, _object: &mut serde_json::Map<String, serde_json::Value>) {}
const MIGRATE: MigrationStep = migrate;

/// Valide une URL de catalogue (http/https avec hôte) et la renvoie normalisée.
pub fn validate_catalog_url(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.len() > MAX_URL_LEN {
        return Err(AppError::Validation(format!(
            "l'URL ne peut pas dépasser {MAX_URL_LEN} caractères"
        )));
    }
    let url = url::Url::parse(raw)
        .map_err(|err| AppError::Validation(format!("URL invalide « {raw} » : {err}")))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(AppError::Validation(format!(
            "l'URL « {raw} » doit commencer par http:// ou https://"
        )));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AppError::Validation(
            "l'URL ne doit pas contenir d'identifiants (utilisateur:mot de passe@)".into(),
        ));
    }
    Ok(url.into())
}

pub(crate) fn validate_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err(AppError::Validation("le nom ne peut pas être vide".into()));
    }
    if name.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "le nom ne peut pas contenir de caractères de contrôle".into(),
        ));
    }
    if name.chars().count() > MAX_NAME_LEN {
        return Err(AppError::Validation(format!(
            "le nom ne peut pas dépasser {MAX_NAME_LEN} caractères"
        )));
    }
    Ok(name.to_string())
}

fn validate_folder(raw: &str) -> AppResult<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(AppError::Validation(
            "le chemin du dossier ne peut pas être vide".into(),
        ));
    }
    let path = Path::new(raw);
    if !path.is_absolute() {
        return Err(AppError::Validation(format!(
            "le chemin « {raw} » doit être absolu"
        )));
    }
    if !path.is_dir() {
        return Err(AppError::Validation(format!(
            "« {raw} » n'est pas un dossier existant"
        )));
    }
    Ok(raw.to_string())
}

/// Valide une entrée et construit la source correspondante avec l'identifiant donné.
fn build(id: String, input: SourceInput) -> AppResult<Source> {
    Ok(match input {
        SourceInput::Catalog { name, url } => Source::Catalog {
            id,
            name: validate_name(&name)?,
            url: validate_catalog_url(&url)?,
        },
        SourceInput::Folder { name, path } => Source::Folder {
            id,
            name: validate_name(&name)?,
            path: validate_folder(&path)?,
        },
    })
}

/// Accès à `sources.json` dans un dossier de configuration (injecté pour les tests).
pub struct SourcesStore {
    dir: PathBuf,
}

impl SourcesStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            dir: config_dir.to_path_buf(),
        }
    }

    /// Charge le fichier. Absent ou corrompu : valeurs par défaut (Telmi Store), écrites sur
    /// disque (corruption : sauvegarde `*.bak` et restauration mémorisée). Les entrées
    /// « Mes histoires » et les identifiants en double (fichier édité à la main) sont écartés.
    fn load(&self) -> AppResult<SourcesFile> {
        let mut file: SourcesFile =
            storage::load_or_default(&self.dir, FILE_NAME, SCHEMA_VERSION, MIGRATE)?;
        let mut seen = std::collections::HashSet::new();
        file.sources.retain(|source| {
            if matches!(source, Source::Library { .. }) {
                return false;
            }
            let fresh = seen.insert(source.id().to_string());
            if !fresh {
                tracing::warn!(
                    "sources.json : identifiant en double ignoré : {}",
                    source.id()
                );
            }
            fresh
        });
        Ok(file)
    }

    /// Écrit le fichier. La version écrite est la plus basse qui décrit son contenu : v1 tant
    /// qu'aucune source podcast n'existe, pour qu'un retour à un launcher v1 reste possible ;
    /// v2 dès qu'il y en a une (un launcher v1 refuse alors le fichier sans le modifier).
    fn save(&self, file: &SourcesFile) -> AppResult<()> {
        let has_podcast = file
            .sources
            .iter()
            .any(|s| matches!(s, Source::Podcast { .. }));
        let written = SourcesFileRef {
            schema_version: if has_podcast { SCHEMA_VERSION } else { 1 },
            sources: &file.sources,
        };
        storage::write_json(&self.dir.join(FILE_NAME), &written)
    }

    /// Liste et restauration éventuelle à signaler à l'utilisateur.
    pub fn view(&self) -> AppResult<SourcesView> {
        let sources = public(self.list()?);
        let recovery = storage::pending_recovery(&self.dir, FILE_NAME);
        Ok(SourcesView { sources, recovery })
    }

    /// Renvoie une source par identifiant (« Mes histoires » comprise).
    pub fn get(&self, id: &str) -> AppResult<Source> {
        self.list()?
            .into_iter()
            .find(|source| source.id() == id)
            .ok_or_else(|| not_found(id))
    }

    /// Liste ordonnée : « Mes histoires » d'abord, puis les sources enregistrées.
    pub fn list(&self) -> AppResult<Vec<Source>> {
        let mut sources = vec![Source::library()];
        sources.extend(self.load()?.sources);
        Ok(sources)
    }

    /// Ajoute une source en fin de liste.
    pub fn add(&self, input: SourceInput) -> AppResult<Source> {
        let mut file = self.load()?;
        let source = build(uuid::Uuid::new_v4().to_string(), input)?;
        file.sources.push(source.clone());
        self.save(&file)?;
        Ok(source)
    }

    /// Enregistre une source podcast déjà validée (`url` normalisée par
    /// [`crate::podcast::source::validate_feed_url`], `name` par [`validate_name`]) sous
    /// l'identifiant `id`. Refusé si le même flux est déjà une source : le message ne cite pas
    /// l'URL (D11).
    pub fn add_podcast(&self, id: String, name: String, url: String) -> AppResult<Source> {
        let mut file = self.load()?;
        if file
            .sources
            .iter()
            .any(|s| matches!(s, Source::Podcast { url: u, .. } if *u == url))
        {
            return Err(AppError::Validation(
                "ce flux podcast est déjà dans vos sources".into(),
            ));
        }
        if file.sources.iter().any(|s| s.id() == id) {
            return Err(AppError::Validation(format!(
                "identifiant de source déjà utilisé : {id}"
            )));
        }
        let source = Source::Podcast { id, name, url };
        file.sources.push(source.clone());
        self.save(&file)?;
        Ok(source)
    }

    /// Met à jour une source existante (le type ne peut pas changer).
    pub fn update(&self, id: &str, input: SourceInput) -> AppResult<Source> {
        reject_library(id, "modifiée")?;
        let mut file = self.load()?;
        let slot = file
            .sources
            .iter_mut()
            .find(|source| source.id() == id)
            .ok_or_else(|| not_found(id))?;
        let updated = build(id.to_string(), input)?;
        if std::mem::discriminant(slot) != std::mem::discriminant(&updated) {
            return Err(AppError::Validation(
                "le type d'une source ne peut pas être modifié".into(),
            ));
        }
        *slot = updated.clone();
        self.save(&file)?;
        Ok(updated)
    }

    /// Supprime une source (sauf « Mes histoires »).
    pub fn remove(&self, id: &str) -> AppResult<()> {
        reject_library(id, "supprimée")?;
        let mut file = self.load()?;
        let before = file.sources.len();
        file.sources.retain(|source| source.id() != id);
        if file.sources.len() == before {
            return Err(not_found(id));
        }
        self.save(&file)
    }

    /// Réordonne les sources enregistrées : `ids` doit être une permutation exacte de leurs
    /// identifiants (sans « Mes histoires », toujours en tête).
    pub fn reorder(&self, ids: &[String]) -> AppResult<Vec<Source>> {
        if ids.iter().any(|id| id == LIBRARY_ID) {
            return Err(AppError::Validation(
                "« Mes histoires » est toujours en première position".into(),
            ));
        }
        let mut file = self.load()?;
        let mut remaining = std::mem::take(&mut file.sources);
        let mut ordered = Vec::with_capacity(remaining.len());
        for id in ids {
            let index = remaining
                .iter()
                .position(|source| source.id() == id)
                .ok_or_else(|| {
                    AppError::Validation(format!("identifiant inconnu ou en double : {id}"))
                })?;
            ordered.push(remaining.remove(index));
        }
        if !remaining.is_empty() {
            return Err(AppError::Validation(
                "la liste doit contenir toutes les sources".into(),
            ));
        }
        file.sources = ordered;
        self.save(&file)?;
        self.list()
    }
}

fn reject_library(id: &str, action: &str) -> AppResult<()> {
    if id == LIBRARY_ID {
        return Err(AppError::Validation(format!(
            "« {LIBRARY_NAME} » ne peut pas être {action}"
        )));
    }
    Ok(())
}

fn not_found(id: &str) -> AppError {
    AppError::NotFound(format!("source introuvable : {id}"))
}

/// Liste les sources (« Mes histoires » en tête) et signale une éventuelle restauration.
#[tauri::command]
pub async fn sources_list(state: State<'_, ConfigState>) -> AppResult<SourcesView> {
    let _guard = state.guard();
    SourcesStore::new(&state.dir).view()
}

/// Ajoute une source de type catalogue ou dossier.
#[tauri::command]
pub async fn sources_add(
    state: State<'_, ConfigState>,
    input: SourceInput,
) -> AppResult<PublicSource> {
    let _guard = state.guard();
    SourcesStore::new(&state.dir).add(input).map(Into::into)
}

/// Met à jour le nom et l'URL/le chemin d'une source.
#[tauri::command]
pub async fn sources_update(
    state: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    watch: State<'_, WatchState>,
    id: String,
    input: SourceInput,
) -> AppResult<PublicSource> {
    let (previous, updated) = {
        let _guard = state.guard();
        let store = SourcesStore::new(&state.dir);
        let previous = store.get(&id).ok();
        (previous, store.update(&id, input)?)
    };
    // Le dossier surveillé n'est plus celui de la source : le front relancera la surveillance
    // sur le nouveau chemin quand il rouvrira la source.
    if previous
        .as_ref()
        .is_some_and(|old| folder_path_changed(old, &updated))
    {
        watch.stop(&id);
    }
    if previous.is_some_and(|old| catalog_url_changed(&old, &updated)) {
        purge_catalog_cache(&state, &catalog, &id).await;
    }
    Ok(updated.into())
}

/// Supprime une source. Le cache d'un catalogue ou d'un podcast est purgé ; les histoires
/// installées ne sont jamais touchées (P4, D15).
#[tauri::command]
pub async fn sources_remove(
    state: State<'_, ConfigState>,
    catalog: State<'_, CatalogService>,
    podcasts: State<'_, PodcastService>,
    watch: State<'_, WatchState>,
    id: String,
) -> AppResult<()> {
    let removed = {
        let _guard = state.guard();
        let store = SourcesStore::new(&state.dir);
        let previous = store.get(&id).ok();
        store.remove(&id)?;
        previous
    };
    // Sans effet si la source n'était pas un dossier surveillé.
    watch.stop(&id);
    match removed {
        Some(Source::Catalog { .. }) => purge_catalog_cache(&state, &catalog, &id).await,
        Some(Source::Podcast { .. }) => purge_podcast_cache(&state, &podcasts, &id).await,
        _ => {}
    }
    Ok(())
}

/// Le chemin d'une source dossier a changé : sa surveillance ne porte plus sur le bon dossier.
fn folder_path_changed(old: &Source, new: &Source) -> bool {
    matches!((old, new), (Source::Folder { path: a, .. }, Source::Folder { path: b, .. }) if a != b)
}

/// L'URL d'une source catalogue a changé : son cache n'est plus utilisable.
fn catalog_url_changed(old: &Source, new: &Source) -> bool {
    matches!((old, new), (Source::Catalog { url: a, .. }, Source::Catalog { url: b, .. }) if a != b)
}

/// Réordonne les sources enregistrées et renvoie la liste complète.
#[tauri::command]
pub async fn sources_reorder(
    state: State<'_, ConfigState>,
    ids: Vec<String>,
) -> AppResult<Vec<PublicSource>> {
    let _guard = state.guard();
    SourcesStore::new(&state.dir).reorder(&ids).map(public)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn store() -> (tempfile::TempDir, SourcesStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = SourcesStore::new(dir.path());
        (dir, store)
    }

    fn catalog(name: &str, url: &str) -> SourceInput {
        SourceInput::Catalog {
            name: name.into(),
            url: url.into(),
        }
    }

    #[test]
    fn changement_de_chemin_detecte_pour_un_dossier_seulement() {
        let folder = |path: &str| Source::Folder {
            id: "f".into(),
            name: "d".into(),
            path: path.into(),
        };
        assert!(!folder_path_changed(&folder("/a"), &folder("/a")));
        assert!(folder_path_changed(&folder("/a"), &folder("/b")));
        let catalog = Source::Catalog {
            id: "c".into(),
            name: "c".into(),
            url: "https://a.example.test/c.json".into(),
        };
        assert!(!folder_path_changed(&catalog, &folder("/b")));
        assert!(!folder_path_changed(&folder("/a"), &catalog));
    }

    #[test]
    fn changement_d_url_detecte_pour_un_catalogue_seulement() {
        let (_dir, store) = store();
        let old = store
            .add(catalog("Un", "https://a.example.test/c.json"))
            .unwrap();
        let same = store
            .update(
                old.id(),
                catalog("Renommé", "https://a.example.test/c.json"),
            )
            .unwrap();
        assert!(!catalog_url_changed(&old, &same));
        let moved = store
            .update(
                old.id(),
                catalog("Renommé", "https://b.example.test/c.json"),
            )
            .unwrap();
        assert!(catalog_url_changed(&same, &moved));
        let folder = Source::Folder {
            id: "f".into(),
            name: "d".into(),
            path: "x".into(),
        };
        assert!(!catalog_url_changed(&folder, &moved));
    }

    fn ids(sources: &[Source]) -> Vec<String> {
        sources.iter().map(|s| s.id().to_string()).collect()
    }

    #[test]
    fn creation_par_defaut_avec_library_et_telmi_store() {
        let (dir, store) = store();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0], Source::library());
        assert_eq!(list[1], Source::telmi_store());
        let json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(dir.path().join("sources.json")).unwrap())
                .unwrap();
        assert_eq!(json["schema_version"], 1);
        assert_eq!(json["sources"].as_array().unwrap().len(), 1);
        assert_eq!(json["sources"][0]["type"], "catalog");
        assert_eq!(json["sources"][0]["url"], TELMI_STORE_URL);
    }

    #[test]
    fn fichier_corrompu_sauvegarde_puis_valeurs_par_defaut() {
        let (dir, store) = store();
        fs::write(dir.path().join("sources.json"), "}{ pas du json").unwrap();
        let list = store.list().unwrap();
        assert_eq!(ids(&list), [LIBRARY_ID, TELMI_STORE_ID]);
        let baks: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("sources.json.") && n.ends_with(".bak"))
            .collect();
        assert_eq!(baks.len(), 1);
        // Le fichier par défaut a été réécrit.
        assert!(dir.path().join("sources.json").exists());
    }

    #[test]
    fn la_restauration_est_signalee_jusqu_a_l_acquittement() {
        let (dir, store) = store();
        assert_eq!(store.view().unwrap().recovery, None);
        fs::write(dir.path().join("sources.json"), "corrompu").unwrap();
        let recovery = store.view().unwrap().recovery.unwrap();
        assert_eq!(recovery.file, "sources.json");
        assert!(dir.path().join(&recovery.backup).exists());
        assert_eq!(store.view().unwrap().recovery, Some(recovery));
        storage::ack_recovery(dir.path(), Some("sources.json")).unwrap();
        assert_eq!(store.view().unwrap().recovery, None);
    }

    #[test]
    fn identifiants_en_double_dedoublonnes() {
        let (dir, store) = store();
        fs::write(
            dir.path().join("sources.json"),
            r#"{"schema_version":1,"sources":[
                {"type":"catalog","id":"a","name":"A1","url":"https://exemple.org/1.json"},
                {"type":"catalog","id":"a","name":"A2","url":"https://exemple.org/2.json"},
                {"type":"catalog","id":"b","name":"B","url":"https://exemple.org/3.json"}]}"#,
        )
        .unwrap();
        let list = store.list().unwrap();
        assert_eq!(ids(&list), [LIBRARY_ID, "a", "b"]);
        assert!(matches!(&list[1], Source::Catalog { name, .. } if name == "A1"));
    }

    #[test]
    fn get_renvoie_une_source_ou_not_found() {
        let (_dir, store) = store();
        assert_eq!(store.get(LIBRARY_ID).unwrap(), Source::library());
        assert_eq!(store.get(TELMI_STORE_ID).unwrap(), Source::telmi_store());
        assert_eq!(store.get("inconnu").unwrap_err().kind(), "not_found");
    }

    #[test]
    fn url_trop_longue_ou_avec_identifiants_refusee() {
        let (_dir, store) = store();
        let long = format!("https://exemple.org/{}", "a".repeat(2048));
        for bad in [
            long.as_str(),
            "https://user:pass@exemple.org/c.json",
            "https://user@exemple.org/c.json",
        ] {
            let err = store.add(catalog("X", bad)).unwrap_err();
            assert_eq!(err.kind(), "validation", "{bad}");
        }
    }

    #[test]
    fn nom_avec_caracteres_de_controle_refuse() {
        let (_dir, store) = store();
        for bad in ["a\nb", "a\u{0}b", "a\tb", "\u{7f}x"] {
            let err = store
                .add(catalog(bad, "https://exemple.org/c.json"))
                .unwrap_err();
            assert_eq!(err.kind(), "validation", "{bad:?}");
        }
    }

    #[test]
    fn telmi_store_supprime_ne_revient_pas() {
        let (_dir, store) = store();
        store.remove(TELMI_STORE_ID).unwrap();
        assert_eq!(ids(&store.list().unwrap()), [LIBRARY_ID]);
    }

    #[test]
    fn la_migration_depuis_un_fichier_sans_version_conserve_les_sources() {
        let (dir, store) = store();
        fs::write(
            dir.path().join("sources.json"),
            r#"{"sources":[{"type":"catalog","id":"a","name":"A","url":"https://exemple.org/c.json"}]}"#,
        )
        .unwrap();
        assert_eq!(ids(&store.list().unwrap()), [LIBRARY_ID, "a"]);
    }

    #[test]
    fn library_stockee_dans_le_fichier_est_ignoree() {
        let (dir, store) = store();
        fs::write(
            dir.path().join("sources.json"),
            r#"{"schema_version":1,"sources":[{"type":"library","id":"x","name":"X"}]}"#,
        )
        .unwrap();
        assert_eq!(ids(&store.list().unwrap()), [LIBRARY_ID]);
    }

    #[test]
    fn version_de_schema_future_est_refusee() {
        let (dir, store) = store();
        fs::write(
            dir.path().join("sources.json"),
            r#"{"schema_version":99,"sources":[]}"#,
        )
        .unwrap();
        assert_eq!(store.list().unwrap_err().kind(), "config_version");
    }

    #[test]
    fn ajout_catalogue_normalise_et_persiste() {
        let (_dir, store) = store();
        let added = store
            .add(catalog("  Mon catalogue ", " https://exemple.org/c.json "))
            .unwrap();
        match &added {
            Source::Catalog { name, url, .. } => {
                assert_eq!(name, "Mon catalogue");
                assert_eq!(url, "https://exemple.org/c.json");
            }
            other => panic!("type inattendu : {other:?}"),
        }
        let list = store.list().unwrap();
        assert_eq!(list.last().unwrap(), &added);
    }

    #[test]
    fn ajout_dossier_existant() {
        let (_dir, store) = store();
        let target = tempfile::tempdir().unwrap();
        let path = target.path().to_str().unwrap().to_string();
        let added = store
            .add(SourceInput::Folder {
                name: "Dossier".into(),
                path: path.clone(),
            })
            .unwrap();
        assert!(matches!(added, Source::Folder { path: p, .. } if p == path));
    }

    #[test]
    fn validation_des_urls() {
        let (_dir, store) = store();
        for bad in [
            "",
            "pas une url",
            "ftp://exemple.org/c.json",
            "file:///C:/c.json",
            "https://",
        ] {
            let err = store.add(catalog("X", bad)).unwrap_err();
            assert_eq!(err.kind(), "validation", "{bad}");
        }
    }

    #[test]
    fn validation_des_noms() {
        let (_dir, store) = store();
        let url = "https://exemple.org/c.json";
        assert_eq!(
            store.add(catalog("   ", url)).unwrap_err().kind(),
            "validation"
        );
        let long = "x".repeat(101);
        assert_eq!(
            store.add(catalog(&long, url)).unwrap_err().kind(),
            "validation"
        );
        assert!(store.add(catalog(&"é".repeat(100), url)).is_ok());
    }

    #[test]
    fn validation_des_chemins() {
        let (dir, store) = store();
        let file = dir.path().join("fichier.txt");
        fs::write(&file, "x").unwrap();
        let missing = dir.path().join("absent");
        for bad in [
            "".to_string(),
            "relatif/dossier".to_string(),
            file.to_str().unwrap().to_string(),
            missing.to_str().unwrap().to_string(),
        ] {
            let err = store
                .add(SourceInput::Folder {
                    name: "D".into(),
                    path: bad.clone(),
                })
                .unwrap_err();
            assert_eq!(err.kind(), "validation", "{bad}");
        }
        // Rien n'a été ajouté.
        assert_eq!(store.list().unwrap().len(), 2);
    }

    #[test]
    fn mise_a_jour() {
        let (_dir, store) = store();
        let updated = store
            .update(
                TELMI_STORE_ID,
                catalog("Renommé", "https://exemple.org/autre.json"),
            )
            .unwrap();
        assert_eq!(
            updated,
            Source::Catalog {
                id: TELMI_STORE_ID.into(),
                name: "Renommé".into(),
                url: "https://exemple.org/autre.json".into()
            }
        );
        assert_eq!(store.list().unwrap()[1], updated);
    }

    #[test]
    fn mise_a_jour_erreurs() {
        let (dir, store) = store();
        let url = "https://exemple.org/c.json";
        assert_eq!(
            store
                .update("inconnu", catalog("X", url))
                .unwrap_err()
                .kind(),
            "not_found"
        );
        assert_eq!(
            store
                .update(LIBRARY_ID, catalog("X", url))
                .unwrap_err()
                .kind(),
            "validation"
        );
        // Changement de type interdit.
        let err = store
            .update(
                TELMI_STORE_ID,
                SourceInput::Folder {
                    name: "D".into(),
                    path: dir.path().to_str().unwrap().into(),
                },
            )
            .unwrap_err();
        assert_eq!(err.kind(), "validation");
        // Entrée invalide : source inchangée.
        assert!(store.update(TELMI_STORE_ID, catalog("", url)).is_err());
        assert_eq!(store.list().unwrap()[1], Source::telmi_store());
    }

    #[test]
    fn suppression() {
        let (_dir, store) = store();
        let added = store
            .add(catalog("A", "https://exemple.org/a.json"))
            .unwrap();
        store.remove(added.id()).unwrap();
        assert_eq!(ids(&store.list().unwrap()), [LIBRARY_ID, TELMI_STORE_ID]);
        assert_eq!(store.remove(added.id()).unwrap_err().kind(), "not_found");
    }

    #[test]
    fn mes_histoires_n_est_pas_supprimable() {
        let (_dir, store) = store();
        assert_eq!(store.remove(LIBRARY_ID).unwrap_err().kind(), "validation");
        assert_eq!(store.list().unwrap()[0], Source::library());
    }

    #[test]
    fn reordonnancement() {
        let (_dir, store) = store();
        let a = store
            .add(catalog("A", "https://exemple.org/a.json"))
            .unwrap();
        let b = store
            .add(catalog("B", "https://exemple.org/b.json"))
            .unwrap();
        let reordered = store
            .reorder(&[b.id().into(), TELMI_STORE_ID.into(), a.id().into()])
            .unwrap();
        let expected = [LIBRARY_ID, b.id(), TELMI_STORE_ID, a.id()];
        assert_eq!(ids(&reordered), expected);
        // L'ordre est persisté.
        assert_eq!(ids(&store.list().unwrap()), expected);
    }

    #[test]
    fn reordonnancement_invalide() {
        let (_dir, store) = store();
        let a = store
            .add(catalog("A", "https://exemple.org/a.json"))
            .unwrap();
        let before = store.list().unwrap();
        let cases: Vec<Vec<String>> = vec![
            vec![],
            vec![TELMI_STORE_ID.into()],
            vec![TELMI_STORE_ID.into(), TELMI_STORE_ID.into()],
            vec![TELMI_STORE_ID.into(), "inconnu".into()],
            vec![LIBRARY_ID.into(), TELMI_STORE_ID.into(), a.id().into()],
        ];
        for ids in cases {
            let err = store.reorder(&ids).unwrap_err();
            assert_eq!(err.kind(), "validation", "{ids:?}");
        }
        assert_eq!(store.list().unwrap(), before);
    }

    #[test]
    fn les_identifiants_sont_uniques() {
        let (_dir, store) = store();
        let url = "https://exemple.org/a.json";
        let a = store.add(catalog("A", url)).unwrap();
        let b = store.add(catalog("A", url)).unwrap();
        assert_ne!(a.id(), b.id());
    }

    const FEED: &str = "https://podcasts.example.test/prive/flux.xml?jeton=SECRET42";

    fn podcast(store: &SourcesStore, id: &str, url: &str) -> AppResult<Source> {
        store.add_podcast(id.into(), "Podcast".into(), url.into())
    }

    fn sources_json(dir: &Path) -> serde_json::Value {
        serde_json::from_str(&fs::read_to_string(dir.join("sources.json")).unwrap()).unwrap()
    }

    /// Fichier v1 réel (catalogue modifié, dossier, Telmi Store supprimé puis ajouté à la main) :
    /// lu sans perte, réécrit en v2 seulement à la première écriture, avec toutes ses entrées.
    #[test]
    fn un_fichier_v1_est_lu_sans_perte_puis_reecrit_en_v2() {
        let (dir, store) = store();
        let folder = tempfile::tempdir().unwrap();
        let folder_path = folder.path().to_str().unwrap().replace('\\', "\\\\");
        let v1 = format!(
            r#"{{"schema_version":1,"sources":[
                {{"type":"folder","id":"f","name":"Clé USB","path":"{folder_path}"}},
                {{"type":"catalog","id":"c","name":"Catalogue","url":"https://exemple.org/c.json","inconnu":1}},
                {{"type":"catalog","id":"telmi-store","name":"Telmi","url":"{TELMI_STORE_URL}"}}]}}"#
        );
        fs::write(dir.path().join("sources.json"), &v1).unwrap();
        let before = store.list().unwrap();
        assert_eq!(ids(&before), [LIBRARY_ID, "f", "c", TELMI_STORE_ID]);
        // Lecture seule : le fichier n'est pas réécrit.
        assert_eq!(
            fs::read_to_string(dir.path().join("sources.json")).unwrap(),
            v1
        );
        podcast(&store, "p", FEED).unwrap();
        let json = sources_json(dir.path());
        assert_eq!(json["schema_version"], 2);
        let after = store.list().unwrap();
        assert_eq!(&after[..4], &before[..]);
        assert!(matches!(&after[4], Source::Podcast { id, url, .. } if id == "p" && url == FEED));
        assert_eq!(storage::pending_recovery(dir.path(), FILE_NAME), None);
    }

    /// Retour possible à un launcher v1 : la version écrite ne monte qu'avec un podcast, et
    /// redescend quand le dernier podcast est supprimé.
    #[test]
    fn la_version_ecrite_ne_monte_qu_avec_un_podcast() {
        let (dir, store) = store();
        store
            .add(catalog("A", "https://exemple.org/a.json"))
            .unwrap();
        assert_eq!(sources_json(dir.path())["schema_version"], 1);
        podcast(&store, "p", FEED).unwrap();
        assert_eq!(sources_json(dir.path())["schema_version"], 2);
        store.remove("p").unwrap();
        assert_eq!(sources_json(dir.path())["schema_version"], 1);
    }

    #[test]
    fn un_fichier_v2_avec_podcast_est_relu() {
        let (dir, store) = store();
        fs::write(
            dir.path().join("sources.json"),
            format!(
                r#"{{"schema_version":2,"sources":[{{"type":"podcast","id":"p","name":"P","url":"{FEED}"}}]}}"#
            ),
        )
        .unwrap();
        assert_eq!(
            store.get("p").unwrap(),
            Source::Podcast {
                id: "p".into(),
                name: "P".into(),
                url: FEED.into()
            }
        );
    }

    /// Un fichier v2 lu par un launcher v1 serait refusé (version future), pas mis en
    /// quarantaine : même garde, vue depuis la version courante avec une version v3.
    #[test]
    fn un_fichier_d_une_version_future_reste_intact() {
        let (dir, store) = store();
        let v3 = r#"{"schema_version":3,"sources":[{"type":"inconnu","id":"x"}]}"#;
        fs::write(dir.path().join("sources.json"), v3).unwrap();
        assert_eq!(store.list().unwrap_err().kind(), "config_version");
        assert_eq!(
            fs::read_to_string(dir.path().join("sources.json")).unwrap(),
            v3
        );
    }

    /// Panne : `sources.json` tronqué au milieu d'une entrée podcast. Les octets d'origine
    /// sont gardés dans le `.bak`, la restauration est signalée.
    #[test]
    fn un_fichier_tronque_est_mis_de_cote_sans_perte_d_octets() {
        let (dir, store) = store();
        podcast(&store, "p", FEED).unwrap();
        let full = fs::read(dir.path().join("sources.json")).unwrap();
        let truncated = &full[..full.len() - 20];
        fs::write(dir.path().join("sources.json"), truncated).unwrap();
        let view = store.view().unwrap();
        assert_eq!(ids_public(&view.sources), [LIBRARY_ID, TELMI_STORE_ID]);
        let recovery = view.recovery.unwrap();
        assert_eq!(
            fs::read(dir.path().join(&recovery.backup)).unwrap(),
            truncated
        );
    }

    /// Panne : écriture interrompue avant le renommage. Le fichier temporaire est ignoré, le
    /// fichier précédent (complet) est lu.
    #[test]
    fn une_ecriture_interrompue_laisse_le_fichier_precedent() {
        let (dir, store) = store();
        podcast(&store, "p", FEED).unwrap();
        fs::write(
            dir.path().join(".sources.json.999-0.tmp"),
            r#"{"schema_version":2,"sour"#,
        )
        .unwrap();
        assert_eq!(
            ids(&store.list().unwrap()),
            [LIBRARY_ID, TELMI_STORE_ID, "p"]
        );
        assert_eq!(storage::pending_recovery(dir.path(), FILE_NAME), None);
    }

    #[test]
    fn un_flux_deja_present_ou_un_identifiant_pris_est_refuse() {
        let (dir, store) = store();
        podcast(&store, "p", FEED).unwrap();
        let err = podcast(&store, "q", FEED).unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert!(!err.to_string().contains("SECRET42"), "{err}");
        let err = podcast(&store, "p", "https://autre.example.test/f.xml").unwrap_err();
        assert_eq!(err.kind(), "validation");
        assert_eq!(
            sources_json(dir.path())["sources"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn un_podcast_ne_change_pas_de_type_et_se_supprime() {
        let (_dir, store) = store();
        podcast(&store, "p", FEED).unwrap();
        let err = store
            .update("p", catalog("X", "https://exemple.org/c.json"))
            .unwrap_err();
        assert_eq!(err.kind(), "validation");
        store.remove("p").unwrap();
        assert_eq!(ids(&store.list().unwrap()), [LIBRARY_ID, TELMI_STORE_ID]);
    }

    /// D11 (décidé le 2026-10-23) : la vue renvoyée au front contient l'URL complète et un
    /// libellé court.
    #[test]
    fn la_vue_publique_donne_l_url_et_un_libelle_court() {
        let (_dir, store) = store();
        podcast(&store, "p", FEED).unwrap();
        let view = store.view().unwrap();
        assert_eq!(
            view.sources[2],
            PublicSource::Podcast {
                id: "p".into(),
                name: "Podcast".into(),
                url: FEED.into(),
                feed_hint: "https://podcasts.example.test/…".into()
            }
        );
        // Les autres types sont inchangés.
        assert_eq!(
            serde_json::to_value(&view.sources[1]).unwrap(),
            serde_json::to_value(Source::telmi_store()).unwrap()
        );
    }

    fn ids_public(sources: &[PublicSource]) -> Vec<String> {
        sources
            .iter()
            .map(|s| match s {
                PublicSource::Library { id, .. }
                | PublicSource::Catalog { id, .. }
                | PublicSource::Folder { id, .. }
                | PublicSource::Podcast { id, .. } => id.clone(),
            })
            .collect()
    }

    #[test]
    fn serialisation_json_des_sources() {
        let json = serde_json::to_value(Source::library()).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"type":"library","id":"library","name":"Mes histoires"})
        );
        let input: SourceInput =
            serde_json::from_str(r#"{"type":"folder","name":"D","path":"C:\\x"}"#).unwrap();
        assert_eq!(
            input,
            SourceInput::Folder {
                name: "D".into(),
                path: "C:\\x".into()
            }
        );
    }
}
