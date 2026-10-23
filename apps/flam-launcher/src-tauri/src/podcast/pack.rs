//! Composition d'un pack de podcasts (#362, spécification `docs/podcasts.md` § 2.2, § 4, N1 à
//! N3, N6, N9) : modèle partagé par l'éditeur (#363), le brouillon et la création (#217).
//!
//! - **Modèle** : métadonnées saisies, couverture choisie, arborescence de menus (catégories et
//!   épisodes). Un épisode est désigné par un **identifiant** ([`EpisodeRef`] : source + id opaque
//!   de [`episode_id`], dérivé du `guid` ou, à défaut, de l'URL d'enclosure). Le front ne
//!   transmet ni URL ni chemin (D11).
//! - **Validation** ([`validate`]) : pure, à partir des flux en cache des sources ; rapport
//!   structuré (un code stable par problème, chemin dans l'arborescence).
//! - **Brouillon unique** (N3 b) : `pack-draft.json` dans le dossier de configuration.
//!
//! # Invariants
//!
//! - Pack valide : 1 à [`MAX_EPISODES`] épisodes (D1), [`MAX_MENU_DEPTH`] niveaux de menus au
//!   plus racine comprise, [`MAX_OPTIONS_PER_MENU`] options par menu (N2 b), aucune catégorie
//!   vide, noms de catégories de 1 à [`MAX_LABEL_CHARS`] caractères (A6), chaque épisode une
//!   seule fois (N9 a) et retrouvé dans le flux en cache de sa source, tailles estimées sous les
//!   plafonds D1 (D5), titre saisi ou flux unique (N1 b), couverture parmi les flux et épisodes
//!   du pack (N6 b).
//! - Aucune URL (flux, enclosure, image) dans les rapports, messages ou traces : seulement des
//!   titres et des identifiants.
//! - Brouillon : écrit sous le verrou de configuration, de façon atomique
//!   ([`storage::write_json`]) ; une sauvegarde dont la révision n'est pas supérieure à celle
//!   enregistrée est ignorée ; l'effacement garde la révision (une sauvegarde en retard ne
//!   ressuscite pas le brouillon) ; un fichier illisible, trop gros ou d'un schéma plus récent
//!   est mis de côté (`*.bak`), jamais perdu.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tauri::State;

use super::rss::{PodcastEpisode, PodcastFeed};
use super::source::{episode_id, PodcastService};
use crate::settings::{Environment, SettingsStore};
use crate::sources::{Source, SourcesStore};
use crate::storage::{self, ConfigState, Loaded};
use crate::{AppError, AppResult};

/// Niveaux de menus au plus, racine comprise (N2 b ; comme `archive::MAX_MENU_DEPTH`).
pub const MAX_MENU_DEPTH: usize = 3;
/// Options (épisodes et catégories) par menu (N2 b).
pub const MAX_OPTIONS_PER_MENU: usize = 50;
/// Épisodes par pack (D1).
pub const MAX_EPISODES: usize = 50;
/// Taille estimée d'un épisode (D1 : 200 Mo).
pub const MAX_EPISODE_BYTES: u64 = 200_000_000;
/// Taille estimée du pack (D1 : 1 Gio).
pub const MAX_PACK_BYTES: u64 = 1_073_741_824;
/// Nom de catégorie, en caractères (A6).
pub const MAX_LABEL_CHARS: usize = 40;
/// Titre et sous-titre, en caractères (comme `archive::MAX_TITLE_CHARS`).
pub const MAX_TITLE_CHARS: usize = 256;
/// Description, en octets UTF-8 (comme `archive::MAX_DESCRIPTION_BYTES`).
pub const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;
/// Débit d'estimation sans taille annoncée : 192 kbps, pire cas du réencodage (D5, D6).
pub const FALLBACK_BYTES_PER_SECOND: u64 = 24_000;
/// Longueur d'un identifiant de source ou d'épisode.
const MAX_ID_LEN: usize = 128;

/// Fichier du brouillon, dans le dossier de configuration.
pub const DRAFT_FILE: &str = "pack-draft.json";
/// Version de schéma du brouillon.
pub const DRAFT_SCHEMA: u32 = 1;
/// Taille du fichier de brouillon au plus (lecture et écriture).
pub const MAX_DRAFT_BYTES: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------------------
// Modèle
// ---------------------------------------------------------------------------------------

/// Épisode désigné par sa source et son identifiant opaque ([`episode_id`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EpisodeRef {
    pub source_id: String,
    pub episode_id: String,
}

/// Option d'un menu : `{"kind":"episode",…}` ou `{"kind":"category",…}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackNode {
    Episode(EpisodeRef),
    Category {
        name: String,
        #[serde(default)]
        entries: Vec<PackNode>,
    },
}

/// Couverture choisie (N6 b).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CoverChoice {
    /// Image du premier flux utilisé qui en a une (ordre de l'arborescence) ; aucun : image
    /// unie de repli.
    #[default]
    Default,
    /// Image d'un flux du pack.
    Feed { source_id: String },
    /// Image d'un épisode du pack.
    Episode(EpisodeRef),
}

/// Métadonnées saisies. Vides ou absentes : défauts D16 et N1 b.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackMetadataInput {
    /// Vide : titre du flux s'il n'y en a qu'un, sinon obligatoire (N1 b).
    #[serde(default)]
    pub title: String,
    /// `null` : copyright du flux s'il n'y en a qu'un, sinon vide (N1 b) ; `""` : vide voulu.
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub age: u8,
    /// `null` : liste des épisodes, calculée à la création (D16).
    #[serde(default)]
    pub description: Option<String>,
}

/// Composition d'un pack, telle que l'éditeur la manipule.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackComposition {
    #[serde(default)]
    pub metadata: PackMetadataInput,
    #[serde(default)]
    pub cover: CoverChoice,
    /// Menu racine.
    #[serde(default)]
    pub menu: Vec<PackNode>,
}

impl PackComposition {
    /// Épisodes dans l'ordre de l'arborescence (doublons compris).
    pub fn episodes(&self) -> Vec<&EpisodeRef> {
        fn walk<'a>(menu: &'a [PackNode], out: &mut Vec<&'a EpisodeRef>) {
            for node in menu {
                match node {
                    PackNode::Episode(e) => out.push(e),
                    PackNode::Category { entries, .. } => walk(entries, out),
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.menu, &mut out);
        out
    }

    /// Sources citées (épisodes et couverture), sans doublon.
    pub fn source_ids(&self) -> BTreeSet<String> {
        let mut ids: BTreeSet<String> = self
            .episodes()
            .iter()
            .map(|e| e.source_id.clone())
            .collect();
        match &self.cover {
            CoverChoice::Default => {}
            CoverChoice::Feed { source_id } => {
                ids.insert(source_id.clone());
            }
            CoverChoice::Episode(e) => {
                ids.insert(e.source_id.clone());
            }
        }
        ids
    }
}

// ---------------------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------------------

/// Code stable d'un problème (le front s'appuie dessus, jamais sur le message).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemCode {
    NoEpisode,
    TooManyEpisodes,
    TooDeep,
    TooManyOptions,
    EmptyCategory,
    InvalidCategoryName,
    DuplicateEpisode,
    InvalidId,
    TitleRequired,
    TextTooLong,
    EpisodeUnavailable,
    EpisodeTooLarge,
    PackTooLarge,
    CoverUnavailable,
}

/// Problème bloquant la création.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Problem {
    pub code: ProblemCode,
    /// Message en français (titres et nombres, jamais d'URL).
    pub message: String,
    /// Position dans l'arborescence (indices depuis le menu racine ; `[]` : menu racine).
    pub path: Option<Vec<usize>>,
    pub episode: Option<EpisodeRef>,
    /// Champ des métadonnées concerné : `title`, `subtitle`, `description` ou `cover`.
    pub field: Option<&'static str>,
}

/// Métadonnées effectives (défauts appliqués).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedMetadata {
    pub title: String,
    pub subtitle: String,
    pub age: u8,
    /// `None` : description par défaut, calculée à la création.
    pub description: Option<String>,
}

/// Rapport de validation d'une composition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub problems: Vec<Problem>,
    /// Épisodes introuvables (source supprimée, flux changé ou jamais obtenu).
    pub unavailable: Vec<EpisodeRef>,
    /// Épisodes de l'arborescence (doublons compris).
    pub episodes: usize,
    /// Sources distinctes des épisodes.
    pub feeds: usize,
    /// Taille estimée des épisodes disponibles (D5), en octets.
    pub estimated_bytes: u64,
    /// Épisodes disponibles sans taille ni durée annoncées.
    pub unknown_size: usize,
    /// `None` : titre obligatoire manquant.
    pub metadata: Option<ResolvedMetadata>,
    /// Couverture effective ; `None` : image unie de repli.
    pub cover: Option<CoverChoice>,
}

/// Flux connus, par source : clé absente = source supprimée (ou qui n'est pas un podcast) ;
/// `None` = flux jamais obtenu.
pub type Feeds = HashMap<String, Option<PodcastFeed>>;

fn problem(code: ProblemCode, message: String) -> Problem {
    Problem {
        code,
        message,
        path: None,
        episode: None,
        field: None,
    }
}

fn on(mut p: Problem, field: &'static str) -> Problem {
    p.field = Some(field);
    p
}

fn at(mut p: Problem, path: &[usize]) -> Problem {
    p.path = Some(path.to_vec());
    p
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && !id.chars().any(char::is_control)
}

/// Taille estimée (D5) : taille annoncée, sinon durée × 192 kbps ; `None` si inconnue.
pub fn estimate_bytes(episode: &PodcastEpisode) -> Option<u64> {
    episode.enclosure_length.filter(|&n| n > 0).or_else(|| {
        episode
            .duration
            .filter(|&d| d > 0)
            .map(|d| u64::from(d) * FALLBACK_BYTES_PER_SECOND)
    })
}

fn megabytes(bytes: u64) -> String {
    format!("{} Mo", bytes.div_ceil(1_000_000))
}

fn truncate_chars(s: &str, max: usize) -> String {
    s.chars().take(max).collect()
}

struct Walker<'a> {
    problems: Vec<Problem>,
    /// Épisodes distincts et position de leur première occurrence, dans l'ordre.
    seen: Vec<(&'a EpisodeRef, Vec<usize>)>,
    seen_set: HashSet<&'a EpisodeRef>,
    count: usize,
}

impl<'a> Walker<'a> {
    /// `level` : niveau du menu `menu` (1 = racine).
    fn menu(&mut self, menu: &'a [PackNode], level: usize, path: &mut Vec<usize>) {
        if menu.len() > MAX_OPTIONS_PER_MENU {
            self.problems.push(at(
                problem(
                    ProblemCode::TooManyOptions,
                    format!(
                        "menu de {} options (au plus {MAX_OPTIONS_PER_MENU})",
                        menu.len()
                    ),
                ),
                path,
            ));
        }
        for (i, node) in menu.iter().enumerate() {
            path.push(i);
            match node {
                PackNode::Episode(e) => self.episode(e, path),
                PackNode::Category { name, entries } => {
                    let name = name.trim();
                    let len = name.chars().count();
                    if len == 0 || len > MAX_LABEL_CHARS || name.chars().any(char::is_control) {
                        self.problems.push(at(
                            problem(
                                ProblemCode::InvalidCategoryName,
                                format!(
                                    "nom de catégorie vide ou trop long (1 à {MAX_LABEL_CHARS} caractères)"
                                ),
                            ),
                            path,
                        ));
                    }
                    if entries.is_empty() {
                        self.problems.push(at(
                            problem(
                                ProblemCode::EmptyCategory,
                                format!("catégorie vide : « {} »", truncate_chars(name, 80)),
                            ),
                            path,
                        ));
                    }
                    // Le sous-menu est au niveau `level + 1`.
                    if level == MAX_MENU_DEPTH {
                        self.problems.push(at(
                            problem(
                                ProblemCode::TooDeep,
                                format!(
                                    "arborescence trop profonde (au plus {MAX_MENU_DEPTH} niveaux de menus)"
                                ),
                            ),
                            path,
                        ));
                    }
                    self.menu(entries, level + 1, path);
                }
            }
            path.pop();
        }
    }

    fn episode(&mut self, e: &'a EpisodeRef, path: &[usize]) {
        self.count += 1;
        if !valid_id(&e.source_id) || !valid_id(&e.episode_id) {
            self.problems.push(at(
                problem(
                    ProblemCode::InvalidId,
                    "identifiant d'épisode invalide".into(),
                ),
                path,
            ));
            return;
        }
        if !self.seen_set.insert(e) {
            let mut p = at(
                problem(
                    ProblemCode::DuplicateEpisode,
                    "épisode présent deux fois dans le pack".into(),
                ),
                path,
            );
            p.episode = Some(e.clone());
            self.problems.push(p);
            return;
        }
        self.seen.push((e, path.to_vec()));
    }
}

/// Valide une composition à partir des flux en cache (`feeds`, voir [`Feeds`]). Pure.
pub fn validate(composition: &PackComposition, feeds: &Feeds) -> ValidationReport {
    let mut w = Walker {
        problems: Vec::new(),
        seen: Vec::new(),
        seen_set: HashSet::new(),
        count: 0,
    };
    w.menu(&composition.menu, 1, &mut Vec::new());
    let Walker {
        mut problems,
        seen,
        count,
        ..
    } = w;

    if count == 0 {
        problems.push(problem(
            ProblemCode::NoEpisode,
            "aucun épisode dans le pack".into(),
        ));
    } else if count > MAX_EPISODES {
        problems.push(problem(
            ProblemCode::TooManyEpisodes,
            format!("{count} épisodes (au plus {MAX_EPISODES})"),
        ));
    }

    // Index des épisodes de chaque flux utilisé, par identifiant opaque.
    let mut index: HashMap<&str, HashMap<String, &PodcastEpisode>> = HashMap::new();
    for (e, _) in &seen {
        if let Some(Some(feed)) = feeds.get(&e.source_id) {
            index.entry(e.source_id.as_str()).or_insert_with(|| {
                feed.episodes
                    .iter()
                    .map(|ep| (episode_id(&ep.guid), ep))
                    .collect()
            });
        }
    }
    let find = |e: &EpisodeRef| -> Option<&PodcastEpisode> {
        index
            .get(e.source_id.as_str())
            .and_then(|m| m.get(&e.episode_id).copied())
    };

    let mut unavailable = Vec::new();
    let mut estimated: u64 = 0;
    let mut unknown_size = 0;
    let mut sources: Vec<&str> = Vec::new();
    for (e, path) in &seen {
        if !sources.contains(&e.source_id.as_str()) {
            sources.push(&e.source_id);
        }
        let Some(ep) = find(e) else {
            let reason = match feeds.get(&e.source_id) {
                None => "source supprimée",
                Some(None) => "flux jamais obtenu",
                Some(Some(_)) => "épisode retiré du flux",
            };
            let mut p = at(
                problem(
                    ProblemCode::EpisodeUnavailable,
                    format!("épisode indisponible ({reason})"),
                ),
                path,
            );
            p.episode = Some((*e).clone());
            problems.push(p);
            unavailable.push((*e).clone());
            continue;
        };
        match estimate_bytes(ep) {
            Some(bytes) => {
                estimated = estimated.saturating_add(bytes);
                if bytes > MAX_EPISODE_BYTES {
                    let mut p = at(
                        problem(
                            ProblemCode::EpisodeTooLarge,
                            format!(
                                "« {} » : environ {} (au plus {} par épisode)",
                                truncate_chars(&ep.title, 80),
                                megabytes(bytes),
                                megabytes(MAX_EPISODE_BYTES)
                            ),
                        ),
                        path,
                    );
                    p.episode = Some((*e).clone());
                    problems.push(p);
                }
            }
            None => unknown_size += 1,
        }
    }
    if estimated > MAX_PACK_BYTES {
        problems.push(problem(
            ProblemCode::PackTooLarge,
            format!(
                "pack d'environ {} (au plus 1 Gio, soit {})",
                megabytes(estimated),
                megabytes(MAX_PACK_BYTES)
            ),
        ));
    }

    // Métadonnées (D16, N1 b).
    let single_feed = match sources.as_slice() {
        [only] => feeds.get(*only).and_then(Option::as_ref),
        _ => None,
    };
    let input = &composition.metadata;
    let given = input.title.trim();
    let title = if given.is_empty() {
        single_feed
            .map(|f| truncate_chars(f.title.trim(), MAX_TITLE_CHARS))
            .filter(|t| !t.is_empty())
    } else {
        Some(given.to_string())
    };
    if title.is_none() {
        problems.push(on(
            problem(
                ProblemCode::TitleRequired,
                "titre du pack obligatoire (plusieurs flux ou flux indisponible)".into(),
            ),
            "title",
        ));
    }
    let subtitle = match &input.subtitle {
        Some(s) => s.trim().to_string(),
        None => single_feed
            .and_then(|f| f.copyright.as_deref())
            .map(|c| truncate_chars(c.trim(), MAX_TITLE_CHARS))
            .unwrap_or_default(),
    };
    for (field, what, text) in [
        ("title", "titre", given),
        ("subtitle", "sous-titre", subtitle.as_str()),
    ] {
        let len = text.chars().count();
        if len > MAX_TITLE_CHARS {
            problems.push(on(
                problem(
                    ProblemCode::TextTooLong,
                    format!("{what} trop long : {len} caractères (au plus {MAX_TITLE_CHARS})"),
                ),
                field,
            ));
        }
    }
    if let Some(d) = &input.description {
        if d.len() > MAX_DESCRIPTION_BYTES {
            problems.push(on(
                problem(
                    ProblemCode::TextTooLong,
                    format!(
                        "description trop longue : {} octets (au plus {MAX_DESCRIPTION_BYTES})",
                        d.len()
                    ),
                ),
                "description",
            ));
        }
    }
    let metadata = title.map(|title| ResolvedMetadata {
        title,
        subtitle,
        age: input.age,
        description: input.description.clone(),
    });

    // Couverture (N6 b).
    let feed_has_image = |id: &str| {
        sources.contains(&id) && matches!(feeds.get(id), Some(Some(f)) if f.image.is_some())
    };
    let cover = match &composition.cover {
        CoverChoice::Default => {
            sources
                .iter()
                .find(|id| feed_has_image(id))
                .map(|id| CoverChoice::Feed {
                    source_id: (*id).to_string(),
                })
        }
        chosen => {
            let ok = match chosen {
                CoverChoice::Feed { source_id } => feed_has_image(source_id),
                CoverChoice::Episode(e) => {
                    seen.iter().any(|(s, _)| *s == e)
                        && find(e).is_some_and(|ep| ep.image.is_some())
                }
                CoverChoice::Default => true,
            };
            if !ok {
                problems.push(on(
                    problem(
                        ProblemCode::CoverUnavailable,
                        "couverture choisie indisponible (image absente ou hors du pack)".into(),
                    ),
                    "cover",
                ));
            }
            ok.then(|| chosen.clone())
        }
    };

    ValidationReport {
        valid: problems.is_empty(),
        problems,
        unavailable,
        episodes: count,
        feeds: sources.len(),
        estimated_bytes: estimated,
        unknown_size,
        metadata,
        cover,
    }
}

// ---------------------------------------------------------------------------------------
// Brouillon (N3 b)
// ---------------------------------------------------------------------------------------

/// Contenu de `pack-draft.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackDraft {
    #[serde(default)]
    pub schema_version: u32,
    /// Révision fournie par le front (croissante, à partir de 1).
    #[serde(default)]
    pub revision: u64,
    /// `None` : brouillon effacé (la révision est conservée).
    #[serde(default)]
    pub composition: Option<PackComposition>,
}

/// Brouillon lu.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DraftRead {
    pub composition: Option<PackComposition>,
    /// Dernière révision enregistrée ; la prochaine sauvegarde doit être supérieure.
    pub revision: u64,
    /// Brouillon mis de côté (illisible, trop gros, schéma plus récent) : message à afficher.
    pub notice: Option<String>,
}

/// Migration du brouillon. Version 0 : fichier sans `schema_version`, même forme que la 1.
fn migrate(_from: u32, _object: &mut Map<String, Value>) {}

fn set_aside(path: &Path, why: &str) -> AppResult<DraftRead> {
    let backup = storage::backup_corrupted(path)?;
    tracing::warn!(
        "brouillon de pack {why} : mis de côté en {}",
        backup.display()
    );
    let name = backup
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(DraftRead {
        notice: Some(format!(
            "Le brouillon de pack précédent ({why}) a été mis de côté ({name})."
        )),
        ..DraftRead::default()
    })
}

/// Lit le brouillon de `dir`. Absent : vide ; illisible, trop gros ou d'un schéma plus récent :
/// mis de côté avec un message. Erreur seulement si le disque est inaccessible.
pub fn read_draft(dir: &Path) -> AppResult<DraftRead> {
    let path = dir.join(DRAFT_FILE);
    match fs::metadata(&path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(DraftRead::default()),
        Err(err) => return Err(err.into()),
        Ok(meta) if meta.len() > MAX_DRAFT_BYTES => return set_aside(&path, "trop volumineux"),
        Ok(_) => {}
    }
    match storage::load_versioned::<PackDraft>(&path, DRAFT_SCHEMA, migrate) {
        Ok(Loaded::Missing) => Ok(DraftRead::default()),
        Ok(Loaded::Found(draft)) => Ok(DraftRead {
            composition: draft.composition,
            revision: draft.revision,
            notice: None,
        }),
        Ok(Loaded::Recovered { backup }) => Ok(DraftRead {
            notice: Some(format!(
                "Le brouillon de pack précédent était illisible : il a été mis de côté ({backup})."
            )),
            ..DraftRead::default()
        }),
        Err(AppError::ConfigVersion { .. }) => {
            set_aside(&path, "écrit par une version plus récente")
        }
        Err(err) => Err(err),
    }
}

/// Révision enregistrée (0 si absente). Un fichier présent mais illisible, trop gros ou d'un
/// schéma plus récent est d'abord mis de côté (`*.bak`) : il n'est jamais écrasé.
fn stored_revision(path: &Path) -> AppResult<u64> {
    #[derive(Deserialize)]
    struct Head {
        #[serde(default)]
        schema_version: u64,
        #[serde(default)]
        revision: u64,
    }
    let bytes = match fs::metadata(path) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(err) => return Err(err.into()),
        Ok(meta) if meta.len() > MAX_DRAFT_BYTES => None,
        Ok(_) => Some(fs::read(path)?),
    };
    match bytes.and_then(|b| serde_json::from_slice::<Head>(&b).ok()) {
        Some(head) if head.schema_version <= u64::from(DRAFT_SCHEMA) => Ok(head.revision),
        _ => {
            set_aside(path, "illisible ou d'une autre version")?;
            Ok(0)
        }
    }
}

/// Résultat d'une sauvegarde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SaveOutcome {
    /// `false` : révision dépassée, rien n'est écrit.
    pub saved: bool,
    /// Révision enregistrée après l'appel : la prochaine sauvegarde doit la dépasser.
    pub revision: u64,
}

/// Écrit le brouillon de façon atomique, sérialisé une seule fois et borné.
fn write_draft(path: &Path, draft: &PackDraft) -> AppResult<()> {
    let mut bytes = serde_json::to_vec_pretty(draft)?;
    bytes.push(b'\n');
    let size = bytes.len() as u64;
    if size > MAX_DRAFT_BYTES {
        return Err(AppError::Validation(format!(
            "brouillon de pack trop volumineux : {size} octets (au plus {MAX_DRAFT_BYTES})"
        )));
    }
    storage::write_atomic(path, &bytes)?;
    Ok(())
}

/// Enregistre le brouillon si `revision` dépasse la révision enregistrée.
pub fn save_draft(
    dir: &Path,
    composition: &PackComposition,
    revision: u64,
) -> AppResult<SaveOutcome> {
    let path = dir.join(DRAFT_FILE);
    let stored = stored_revision(&path)?;
    if revision <= stored {
        return Ok(SaveOutcome {
            saved: false,
            revision: stored,
        });
    }
    let draft = PackDraft {
        schema_version: DRAFT_SCHEMA,
        revision,
        composition: Some(composition.clone()),
    };
    write_draft(&path, &draft)?;
    Ok(SaveOutcome {
        saved: true,
        revision,
    })
}

/// Efface le brouillon (après la création du pack) en gardant la plus grande révision connue :
/// une sauvegarde de révision inférieure ou égale à `revision` arrivée ensuite est ignorée.
pub fn clear_draft(dir: &Path, revision: u64) -> AppResult<()> {
    let path = dir.join(DRAFT_FILE);
    let draft = PackDraft {
        schema_version: DRAFT_SCHEMA,
        revision: revision.max(stored_revision(&path)?),
        composition: None,
    };
    write_draft(&path, &draft)
}

// ---------------------------------------------------------------------------------------
// Commandes Tauri
// ---------------------------------------------------------------------------------------

/// Flux en cache des sources citées par `composition` (sans réseau).
async fn load_feeds(
    config: &ConfigState,
    service: &PodcastService,
    composition: &PackComposition,
) -> AppResult<Feeds> {
    let wanted = composition.source_ids();
    let (podcasts, root) = {
        let _guard = config.guard();
        let podcasts: Vec<(String, String)> = SourcesStore::new(&config.dir)
            .list()?
            .into_iter()
            .filter_map(|s| match s {
                Source::Podcast { id, url, .. } if wanted.contains(&id) => Some((id, url)),
                _ => None,
            })
            .collect();
        let root = SettingsStore::new(&config.dir).resolved_cache_dir(&Environment::current())?;
        (podcasts, root)
    };
    let mut feeds = Feeds::new();
    for (id, url) in podcasts {
        let feed = match service.cached_feed(&root, &id, &url).await {
            Ok(feed) => feed,
            Err(err) => {
                tracing::warn!("pack : flux en cache de la source {id} illisible : {err}");
                None
            }
        };
        feeds.insert(id, feed);
    }
    Ok(feeds)
}

/// Brouillon lu au démarrage de l'éditeur, avec sa validation (épisodes devenus
/// indisponibles signalés, brouillon conservé tel quel).
#[derive(Debug, Clone, Serialize)]
pub struct DraftView {
    #[serde(flatten)]
    pub draft: DraftRead,
    /// `None` : pas de brouillon, ou flux illisibles (voir `notice`).
    pub report: Option<ValidationReport>,
}

/// Assemble la vue : un échec de lecture des flux ne masque jamais le brouillon (sa
/// composition et sa révision restent transmises au front), il est seulement signalé.
pub fn draft_view(mut draft: DraftRead, feeds: Option<AppResult<Feeds>>) -> DraftView {
    let report = match (&draft.composition, feeds) {
        (Some(c), Some(Ok(feeds))) => Some(validate(c, &feeds)),
        (_, Some(Err(err))) => {
            tracing::warn!("pack : validation du brouillon impossible : {err}");
            let text = "Les épisodes du brouillon n'ont pas pu être vérifiés \
                        (sources ou cache inaccessibles).";
            draft.notice = Some(match draft.notice.take() {
                Some(n) => format!("{n} {text}"),
                None => text.to_string(),
            });
            None
        }
        _ => None,
    };
    DraftView { draft, report }
}

/// Lit le brouillon unique et le valide. Le front reprend ses sauvegardes à `revision + 1`.
#[tauri::command]
pub async fn pack_draft_read(
    config: State<'_, ConfigState>,
    service: State<'_, PodcastService>,
) -> AppResult<DraftView> {
    let draft = {
        let _guard = config.guard();
        read_draft(&config.dir)?
    };
    let feeds = match &draft.composition {
        Some(c) => Some(load_feeds(&config, &service, c).await),
        None => None,
    };
    Ok(draft_view(draft, feeds))
}

/// Enregistre le brouillon (sauvegarde automatique, révision croissante émise par le front).
#[tauri::command]
pub async fn pack_draft_save(
    config: State<'_, ConfigState>,
    composition: PackComposition,
    revision: u64,
) -> AppResult<SaveOutcome> {
    let _guard = config.guard();
    save_draft(&config.dir, &composition, revision)
}

/// Efface le brouillon ; `revision` : dernière révision émise par le front.
#[tauri::command]
pub async fn pack_draft_clear(config: State<'_, ConfigState>, revision: u64) -> AppResult<()> {
    let _guard = config.guard();
    clear_draft(&config.dir, revision)
}

/// Valide une composition à partir des flux en cache. Relit et analyse le flux de chaque
/// source : le front espace les appels (après une modification, pas à chaque frappe).
#[tauri::command]
pub async fn pack_validate(
    config: State<'_, ConfigState>,
    service: State<'_, PodcastService>,
    composition: PackComposition,
) -> AppResult<ValidationReport> {
    let feeds = load_feeds(&config, &service, &composition).await?;
    Ok(validate(&composition, &feeds))
}

#[cfg(test)]
mod tests;
