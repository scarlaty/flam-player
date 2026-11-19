//! Commandes Tauri de la fenêtre de l'éditeur (lot 7, `docs/creation-dossier.md` § 4).
//!
//! # Une fenêtre, un dossier, un [`Editor`]
//!
//! - [`authoring_editor_open`] (fenêtre principale) ouvre le dossier choisi en écriture
//!   ([`Editor::open`] : mêmes refus que la lecture, plus l'identité de fichier), puis crée une
//!   fenêtre `editor-<n>` d'après le modèle `editor` de `tauri.conf.json` (seule fenêtre avec
//!   `dragDropEnabled: true`, § 4.3, Q6). Un dossier déjà ouvert n'ouvre pas de seconde
//!   fenêtre : la sienne revient au premier plan et la réponse le signale.
//! - Les autres commandes ne prennent **aucun chemin de cible** : l'éditeur visé est celui de la
//!   fenêtre qui appelle (son libellé, fourni par Tauri, pas par la page), et les éléments sont
//!   désignés par leurs ids (`ops::parse_id`, puis retrouvés par une nouvelle lecture). Seules
//!   exceptions : les **sources** d'une copie, d'une image ou d'un titre lu, chemins absolus
//!   choisis dans le sélecteur natif, lues et jamais modifiées (§ 10).
//! - Chaque opération rend la vue relue ([`EditorView`] : arbre, rapport, états vus, journal) ;
//!   une erreur porte un code stable ([`EditorError::kind`] : celui de `OpError::code`, ou le
//!   `kind` d'`AppError`, ou [`CLOSED`]).
//! - La fenêtre fermée, son éditeur est oublié ([`EditorState::remove`], appelé par
//!   `lib.rs` sur `WindowEvent::Destroyed`) : le journal ne survit pas à la fenêtre, les fichiers
//!   et la corbeille `.flam/` restent.
//! - Le dossier est surveillé tant que la fenêtre est ouverte ([`super::editor_watch`],
//!   lot 9) : les opérations tiennent un silence ([`EditorState::quiet`]) pour que leurs
//!   propres changements ne soient pas signalés ; la surveillance s'arrête avec la fenêtre.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use serde::{Serialize, Serializer};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use super::build::scan_limits;
use super::commands::{self, FolderCreated, FolderReport};
use super::editor_watch::{self, EditorWatch, QuietGuard};
use super::identity::{path_key, ConflictChoice};
use super::ops::{
    parse_id, Done, Editor, FolderPatch, OpError, Skipped, Stamp, StoryPatch, Target,
};
use super::root::ForbiddenRoots;
use super::settings::{self, Debut, Fin, Mode};
use super::tree::{Node, IMAGE_EXTENSIONS};
use crate::catalog::blocking;
use crate::install::InstallService;
use crate::podcast::artwork::{convert_isolated, ArtworkKind, MAX_INPUT_BYTES};
use crate::storage::ConfigState;
use crate::{thumbnail, AppError};

/// Préfixe des libellés des fenêtres de l'éditeur.
pub const EDITOR_LABEL_PREFIX: &str = "editor-";
/// Libellé du modèle de fenêtre de l'éditeur dans `tauri.conf.json` (`create: false`).
pub const EDITOR_TEMPLATE: &str = "editor";
/// Événement envoyé à la fenêtre principale quand l'éditeur lance une création (tâche à suivre
/// dans le panneau des tâches).
pub const TASK_EVENT: &str = "authoring:task";
/// Code d'erreur : la fenêtre n'a pas (ou plus) d'éditeur.
pub const CLOSED: &str = "editor_closed";
/// Côté maximal d'une vignette (pixels).
const MAX_THUMB_DIM: u32 = 320;
/// Vignettes gardées en mémoire par éditeur.
const MAX_THUMBS: usize = 600;

// ---------------------------------------------------------------------------
// Erreurs
// ---------------------------------------------------------------------------

/// Erreur d'une commande de l'éditeur, sérialisée comme `AppError` : `{ kind, message }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorError {
    /// Code stable (`changed`, `not_undoable`, `invalid_id`, `editor_closed`, `io`…).
    pub kind: String,
    pub message: String,
}

impl EditorError {
    fn closed() -> Self {
        Self {
            kind: CLOSED.into(),
            message: "cet éditeur est fermé : rouvrez le dossier".into(),
        }
    }
}

impl From<OpError> for EditorError {
    fn from(e: OpError) -> Self {
        Self {
            kind: e.code().into(),
            message: e.to_string(),
        }
    }
}

impl From<AppError> for EditorError {
    fn from(e: AppError) -> Self {
        Self {
            kind: e.kind().into(),
            message: e.to_string(),
        }
    }
}

impl Serialize for EditorError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        AppErrorShape {
            kind: &self.kind,
            message: &self.message,
        }
        .serialize(serializer)
    }
}

#[derive(Serialize)]
struct AppErrorShape<'a> {
    kind: &'a str,
    message: &'a str,
}

pub type EditorResult<T> = Result<T, EditorError>;

// ---------------------------------------------------------------------------
// État : un éditeur par fenêtre
// ---------------------------------------------------------------------------

/// Éditeur d'une fenêtre et vignettes déjà calculées.
#[derive(Debug)]
pub struct Session {
    editor: Editor,
    /// Vignettes déjà calculées ([`ThumbKey`]) → URL `data:`.
    thumbs: HashMap<ThumbKey, String>,
}

impl Session {
    fn new(editor: Editor) -> Self {
        Self {
            editor,
            thumbs: HashMap::new(),
        }
    }
}

/// Image, état vu (taille, date) et côté d'une vignette.
type ThumbKey = (String, u64, u64, u32, u32);

fn thumb_key(id: &str, stamp: Stamp, dim: u32) -> ThumbKey {
    (
        id.to_owned(),
        stamp.len,
        stamp.modified_ms,
        stamp.modified_ns,
        dim,
    )
}

type Shared = Arc<Mutex<Session>>;

fn lock(session: &Shared) -> MutexGuard<'_, Session> {
    session.lock().unwrap_or_else(|e| e.into_inner())
}

/// Éditeur ouvert : clé du dossier (immuable, comparée sans verrouiller la session) et session.
#[derive(Debug)]
struct Open {
    key: String,
    session: Shared,
    /// Surveillance du dossier (lot 9), posée après la création de la fenêtre.
    watch: Option<EditorWatch>,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, Open>,
    /// Sessions de fenêtres fermées dont une opération peut encore tourner : leur dossier ne se
    /// rouvre pas tant qu'elles vivent (deux éditeurs sur un même `.flam/tmp`).
    closing: Vec<(String, Weak<Mutex<Session>>)>,
    next: u64,
}

/// Éditeurs ouverts, par libellé de fenêtre (`app.manage`).
#[derive(Debug, Default)]
pub struct EditorState {
    inner: Mutex<Inner>,
}

/// Résultat de [`EditorState::register`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Registered {
    /// Nouvel éditeur, pour la fenêtre de ce libellé (à créer).
    New(String),
    /// Le même dossier est déjà ouvert dans la fenêtre de ce libellé.
    Existing(String),
    /// La fenêtre de ce dossier vient d'être fermée pendant une opération qui n'est pas finie.
    Closing,
}

impl EditorState {
    fn inner(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Garde `editor` pour une nouvelle fenêtre, sauf si son dossier (forme résolue, sans
    /// casse) est déjà ouvert : une seule fenêtre par dossier (§ 4.3).
    /// Aucune session n'est verrouillée ici : une opération longue d'un autre éditeur ne bloque
    /// ni l'ouverture ni le registre.
    pub fn register(&self, editor: Editor) -> Registered {
        let key = path_key(editor.base());
        let mut inner = self.inner();
        let existing = inner
            .sessions
            .iter()
            .find(|(_, open)| open.key == key)
            .map(|(label, _)| label.clone());
        if let Some(label) = existing {
            return Registered::Existing(label);
        }
        inner.closing.retain(|(_, weak)| weak.strong_count() > 0);
        if inner.closing.iter().any(|(k, _)| *k == key) {
            return Registered::Closing;
        }
        inner.next += 1;
        let label = format!("{EDITOR_LABEL_PREFIX}{}", inner.next);
        let session = Arc::new(Mutex::new(Session::new(editor)));
        inner.sessions.insert(
            label.clone(),
            Open {
                key,
                session,
                watch: None,
            },
        );
        Registered::New(label)
    }

    /// Éditeur de la fenêtre `label`.
    pub fn get(&self, label: &str) -> EditorResult<Shared> {
        self.inner()
            .sessions
            .get(label)
            .map(|open| open.session.clone())
            .ok_or_else(EditorError::closed)
    }

    /// Oublie l'éditeur de la fenêtre `label` (fenêtre fermée). Une opération encore en cours
    /// garde sa session vivante : le dossier ne se rouvre qu'à sa fin ([`Registered::Closing`]).
    pub fn remove(&self, label: &str) {
        let watch = {
            let mut inner = self.inner();
            let Some(open) = inner.sessions.remove(label) else {
                return;
            };
            let weak = Arc::downgrade(&open.session);
            drop(open.session);
            if weak.strong_count() > 0 {
                inner.closing.push((open.key, weak));
            }
            open.watch
        };
        // Surveillance arrêtée hors du verrou du registre (attente de son thread).
        drop(watch);
    }

    /// Pose la surveillance du dossier de la fenêtre `label` (sans effet si elle est fermée).
    pub fn set_watch(&self, label: &str, watch: EditorWatch) {
        let replaced = {
            let mut inner = self.inner();
            match inner.sessions.get_mut(label) {
                Some(open) => open.watch.replace(watch),
                None => Some(watch),
            }
        };
        drop(replaced);
    }

    /// Silence de la surveillance pendant une opération de la fenêtre `label` : les
    /// changements que fait l'éditeur ne sont pas signalés comme faits ailleurs.
    pub fn quiet(&self, label: &str) -> Option<QuietGuard> {
        self.inner()
            .sessions
            .get(label)
            .and_then(|open| open.watch.as_ref())
            .map(EditorWatch::quiet)
    }

    /// Libellés des fenêtres ouvertes.
    pub fn labels(&self) -> Vec<String> {
        self.inner().sessions.keys().cloned().collect()
    }
}

// ---------------------------------------------------------------------------
// Vue
// ---------------------------------------------------------------------------

/// Annuler / Rétablir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct JournalView {
    pub can_undo: bool,
    pub can_redo: bool,
    pub undo_label: Option<String>,
    pub redo_label: Option<String>,
}

/// Réglages de `_histoire.json` montrés dans l'en-tête (valeurs **réglées**, `None` : absentes).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StoryFields {
    pub titre: Option<String>,
    pub sous_titre: Option<String>,
    pub description: Option<String>,
    pub age: Option<u8>,
    pub debut: Option<Debut>,
    pub mode: Option<Mode>,
    pub fin: Option<Fin>,
    pub annoncer: Option<bool>,
}

/// Réglages réglés d'un dossier (`_dossier.json`), pour proposer « comme le dossier parent ».
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FolderFields {
    pub mode: Option<Mode>,
    pub fin: Option<Fin>,
    pub annoncer: Option<bool>,
}

/// Ce que montre la fenêtre de l'éditeur.
#[derive(Debug, Clone, Serialize)]
pub struct EditorView {
    /// Arbre et rapport (comme « Créer une histoire depuis un dossier »).
    pub report: FolderReport,
    /// Nom du dossier ouvert.
    pub folder_name: String,
    /// État vu de chaque élément (id → état) : à rendre avec la cible d'une opération.
    pub stamps: HashMap<String, Stamp>,
    pub story: StoryFields,
    /// Réglages réglés des sous-dossiers (id → réglages).
    pub folders: HashMap<String, FolderFields>,
    pub journal: JournalView,
}

/// Résultat d'une opération : ce qui a été fait, la vue relue.
#[derive(Debug, Clone, Serialize)]
pub struct EditorUpdate {
    /// Libellé de l'opération faite (ou défaite).
    pub label: String,
    /// Ids des éléments créés ou touchés (sélection gardée).
    pub ids: Vec<String>,
    /// Éléments d'une copie non copiés.
    pub skipped: Vec<Skipped>,
    /// « Vider la corbeille » : éléments envoyés dans la Corbeille Windows.
    pub trashed: Option<usize>,
    pub view: EditorView,
}

/// Ce qu'a fait une opération.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub done: Done,
    pub skipped: Vec<Skipped>,
    pub trashed: Option<usize>,
}

impl From<Done> for Outcome {
    fn from(done: Done) -> Self {
        Self {
            done,
            skipped: Vec::new(),
            trashed: None,
        }
    }
}

/// Résultat de [`authoring_editor_open`].
#[derive(Debug, Clone, Serialize)]
pub struct EditorOpened {
    /// Le dossier était déjà ouvert : sa fenêtre est revenue au premier plan.
    pub already_open: bool,
}

/// Résultat de [`authoring_editor_create`].
#[derive(Debug, Clone, Serialize)]
pub struct EditorCreated {
    pub created: FolderCreated,
    pub view: EditorView,
}

/// Ids de l'arbre (éléments, images et titres lus).
fn tree_ids(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Menu {
            id,
            image,
            title_audio,
            children,
            ..
        } => {
            out.push(id.clone());
            out.extend(image.iter().cloned());
            out.extend(title_audio.iter().cloned());
            for child in children {
                tree_ids(child, out);
            }
        }
        Node::Story {
            id,
            image,
            title_audio,
            ..
        } => {
            out.push(id.clone());
            out.extend(image.iter().cloned());
            out.extend(title_audio.iter().cloned());
        }
    }
}

/// États actuels des éléments de `root` (un élément disparu entre-temps est omis : l'opération
/// qui le vise sera refusée comme « changé »).
fn stamps_of(editor: &Editor, root: &Node) -> HashMap<String, Stamp> {
    let mut ids = Vec::new();
    tree_ids(root, &mut ids);
    ids.into_iter()
        .filter_map(|id| editor.stamp(&id).ok().map(|stamp| (id, stamp)))
        .collect()
}

fn journal_of(editor: &Editor) -> JournalView {
    JournalView {
        can_undo: editor.can_undo(),
        can_redo: editor.can_redo(),
        undo_label: editor.undo_label().map(str::to_owned),
        redo_label: editor.redo_label().map(str::to_owned),
    }
}

/// Réglages réglés des sous-dossiers de `root` (relus, sans suivre de lien).
fn folders_of(base: &Path, root: &Node) -> HashMap<String, FolderFields> {
    fn walk(base: &Path, node: &Node, out: &mut HashMap<String, FolderFields>) {
        if let Node::Menu { id, children, .. } = node {
            if !id.is_empty() {
                let loaded = settings::load_folder(&super::build::path_of(base, id)).settings;
                out.insert(
                    id.clone(),
                    FolderFields {
                        mode: loaded.mode,
                        fin: loaded.fin,
                        annoncer: loaded.annoncer,
                    },
                );
            }
            for child in children {
                walk(base, child, out);
            }
        }
    }
    let mut out = HashMap::new();
    walk(base, root, &mut out);
    out
}

/// Dossier ouvert par la session et bornes d'une lecture.
fn base_of(session: &Shared) -> PathBuf {
    lock(session).editor.base().to_path_buf()
}

/// Vue relue du dossier de `session`.
async fn view_of(
    config: &ConfigState,
    install: &InstallService,
    session: &Shared,
) -> EditorResult<EditorView> {
    let base = base_of(session);
    let read = commands::read(config, install, &base.to_string_lossy()).await?;
    let report = commands::report(&read);
    let story = &read.scan.story.settings;
    let fields = StoryFields {
        titre: story.titre.clone(),
        sous_titre: story.sous_titre.clone(),
        description: story.description.clone(),
        age: story.age,
        debut: story.debut,
        mode: story.folder.mode,
        fin: story.folder.fin,
        annoncer: story.folder.annoncer,
    };
    let root = read.scan.root.clone();
    let shared = session.clone();
    let (stamps, folders, journal) = blocking(move || {
        let session = lock(&shared);
        let stamps = stamps_of(&session.editor, &root);
        let folders = folders_of(session.editor.base(), &root);
        (stamps, folders, journal_of(&session.editor))
    })
    .await?;
    let folder_name = base
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| base.to_string_lossy().into_owned());
    Ok(EditorView {
        report,
        folder_name,
        stamps,
        story: fields,
        folders,
        journal,
    })
}

// ---------------------------------------------------------------------------
// Opérations
// ---------------------------------------------------------------------------

/// Opération demandée par la page (ids et sources seulement).
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Import {
        menu: String,
        sources: Vec<PathBuf>,
    },
    SetImage {
        target: Target,
        source: PathBuf,
    },
    SetTitleAudio {
        target: Target,
        source: PathBuf,
    },
    Keep {
        target: Target,
    },
    Reorder {
        menu: String,
        order: Vec<String>,
    },
    Move {
        target: Target,
        dest: String,
        before: Option<String>,
    },
    Rename {
        target: Target,
        title: String,
        rename_file: bool,
    },
    NewMenu {
        parent: String,
        name: String,
    },
    Delete {
        target: Target,
    },
    Story(StoryPatch),
    Folder {
        menu: String,
        patch: FolderPatch,
    },
    Undo,
    Redo,
    EmptyTrash,
}

/// Sources choisies dans le sélecteur natif : chemins absolus seulement (le reste est vérifié
/// par l'opération, qui ne les modifie jamais).
fn sources(paths: Vec<String>) -> EditorResult<Vec<PathBuf>> {
    if paths.is_empty() || paths.len() > 1_000 {
        return Err(OpError::Refused("entre 1 et 1 000 fichiers à la fois".into()).into());
    }
    paths.into_iter().map(source).collect()
}

fn source(path: String) -> EditorResult<PathBuf> {
    let path = PathBuf::from(path);
    if !path.is_absolute() {
        return Err(OpError::Source {
            path: path.to_string_lossy().into_owned(),
            reason: "chemin relatif",
        }
        .into());
    }
    Ok(path)
}

/// Première image d'histoire sous `node`, dans l'ordre affiché (règle des compléments, § 7).
fn first_story_image(node: &Node) -> Option<&str> {
    match node {
        Node::Story { image, .. } => image.as_deref(),
        Node::Menu { children, .. } => children.iter().find_map(first_story_image),
    }
}

/// Image que « Garder » écrit pour le menu `id` (racine : la couverture) : le complément
/// automatique, celle de sa première histoire (§ 7) ; `None` si le menu a déjà la sienne ou si
/// aucune histoire n'a d'image.
pub fn keep_source(root: &Node, id: &str) -> Option<String> {
    match root.find(id)? {
        Node::Menu { image: None, .. } => first_story_image(root.find(id)?).map(str::to_owned),
        _ => None,
    }
}

/// Exécute `op` sur l'éditeur (bloquant).
fn apply(editor: &mut Editor, op: Op) -> EditorResult<Outcome> {
    let done = match op {
        Op::Import { menu, sources } => {
            let report = editor.import(&menu, &sources)?;
            return Ok(Outcome {
                done: report.done,
                skipped: report.skipped,
                trashed: None,
            });
        }
        Op::SetImage { target, source } => editor.set_image(&target, &source)?,
        Op::SetTitleAudio { target, source } => editor.set_title_audio(&target, &source)?,
        Op::Keep { target } => {
            let id = parse_id(&target.id)?;
            let scan = editor.scan()?;
            let from = keep_source(&scan.root, &id).ok_or_else(|| {
                OpError::Refused("aucune image automatique à garder pour cet élément".into())
            })?;
            let source = super::build::path_of(editor.base(), &from);
            editor.set_image(&target, &source)?
        }
        Op::Reorder { menu, order } => editor.reorder(&menu, &order)?,
        Op::Move {
            target,
            dest,
            before,
        } => editor.move_item(&target, &dest, before.as_deref())?,
        Op::Rename {
            target,
            title,
            rename_file,
        } => editor.rename(&target, &title, rename_file)?,
        Op::NewMenu { parent, name } => editor.new_menu(&parent, &name)?,
        Op::Delete { target } => editor.delete(&target)?,
        Op::Story(patch) => editor.set_story_settings(&patch)?,
        Op::Folder { menu, patch } => editor.set_folder_settings(&menu, &patch)?,
        Op::Undo => editor.undo()?,
        Op::Redo => editor.redo()?,
        Op::EmptyTrash => {
            let count = editor.empty_trash()?;
            return Ok(Outcome {
                done: Done {
                    label: "Corbeille de l'histoire vidée".into(),
                    ids: Vec::new(),
                },
                skipped: Vec::new(),
                trashed: Some(count),
            });
        }
    };
    Ok(done.into())
}

/// Exécute `op` sur l'éditeur de la fenêtre `label` (une opération à la fois par éditeur).
pub fn run_op(state: &EditorState, label: &str, op: Op) -> EditorResult<Outcome> {
    let session = state.get(label)?;
    let _quiet = state.quiet(label);
    let mut session = lock(&session);
    let result = apply(&mut session.editor, op);
    if result.is_ok() {
        session.thumbs.clear();
    }
    result
}

async fn op_command(
    window: &WebviewWindow,
    config: &ConfigState,
    install: &InstallService,
    editors: &Arc<EditorState>,
    op: Op,
) -> EditorResult<EditorUpdate> {
    let label = window.label().to_owned();
    let session = editors.get(&label)?;
    let shared = editors.clone();
    let outcome = blocking(move || run_op(&shared, &label, op)).await??;
    let view = view_of(config, install, &session).await?;
    Ok(EditorUpdate {
        label: outcome.done.label,
        ids: outcome.done.ids,
        skipped: outcome.skipped,
        trashed: outcome.trashed,
        view,
    })
}

// ---------------------------------------------------------------------------
// Vignettes
// ---------------------------------------------------------------------------

/// Chemin de l'image `id` sous `base`, vérifié : id valide, extension d'image, aucun composant
/// lien ou jonction.
fn image_path(base: &Path, id: &str) -> EditorResult<PathBuf> {
    let id = parse_id(id)?;
    let ext = id.rsplit_once('.').map(|(_, e)| e.to_lowercase());
    if id.is_empty() || !ext.is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.as_str())) {
        return Err(OpError::InvalidId(id).into());
    }
    let mut path = base.to_path_buf();
    for part in id.split('/') {
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(OpError::Io)?;
        if meta.file_type().is_symlink() {
            return Err(OpError::Link(id.clone()).into());
        }
    }
    Ok(path)
}

/// Vignette PNG (URL `data:`) de l'image `bytes`, au plus `dim` pixels de côté. Décodage dans
/// un processus enfant (`convert_isolated`, #263).
fn make_thumb(bytes: &[u8], dim: u32, cover: bool) -> EditorResult<String> {
    let kind = if cover {
        ArtworkKind::Cover
    } else {
        ArtworkKind::Episode
    };
    let failed = |e: &dyn std::fmt::Display| EditorError {
        kind: "thumbnail".into(),
        message: format!("vignette impossible : {e}"),
    };
    let art = convert_isolated(bytes, kind, None).map_err(|e| failed(&e))?;
    let img = thumbnail::decode_png(&art.png).map_err(|e| failed(&e))?;
    let png = thumbnail::encode_png(&thumbnail::fit(img, dim)).map_err(|e| failed(&e))?;
    Ok(thumbnail::data_uri(&png))
}

fn thumbnail_of(state: &EditorState, label: &str, id: &str, dim: u32) -> EditorResult<String> {
    use std::io::Read;
    let dim = dim.clamp(16, MAX_THUMB_DIM);
    let session = state.get(label)?;
    let (base, stamp) = {
        let s = lock(&session);
        let stamp = s.editor.stamp(id)?;
        if let Some(found) = s.thumbs.get(&thumb_key(id, stamp, dim)) {
            return Ok(found.clone());
        }
        (s.editor.base().to_path_buf(), stamp)
    };
    let path = image_path(&base, id)?;
    let file = crate::install::pack::open_no_follow(&path)?;
    let mut bytes = Vec::new();
    file.take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(OpError::Io)?;
    let cover = !id.contains('/') && id.to_lowercase().starts_with("_couverture.");
    let uri = make_thumb(&bytes, dim, cover)?;
    let mut s = lock(&session);
    if s.thumbs.len() >= MAX_THUMBS {
        s.thumbs.clear();
    }
    s.thumbs.insert(thumb_key(id, stamp, dim), uri.clone());
    Ok(uri)
}

// ---------------------------------------------------------------------------
// Fenêtre
// ---------------------------------------------------------------------------

/// Modèle de la fenêtre de l'éditeur (`tauri.conf.json`, libellé [`EDITOR_TEMPLATE`]).
fn editor_window_config(app: &AppHandle) -> Option<tauri::utils::config::WindowConfig> {
    app.config()
        .app
        .windows
        .iter()
        .find(|w| w.label == EDITOR_TEMPLATE)
        .cloned()
}

fn open_window(app: &AppHandle, label: &str, folder_name: &str) -> tauri::Result<()> {
    let mut config = editor_window_config(app).ok_or_else(|| tauri::Error::WindowNotFound)?;
    config.label = label.to_owned();
    config.title = format!("{} – Éditeur d'histoire", folder_name);
    config.create = true;
    tauri::WebviewWindowBuilder::from_config(app, &config)?.build()?;
    Ok(())
}

fn focus(app: &AppHandle, label: &str) {
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

// ---------------------------------------------------------------------------
// Commandes
// ---------------------------------------------------------------------------

/// Ouvre le dossier `path` (sélecteur natif, ou dossier source d'une histoire de « Mes
/// histoires ») dans une fenêtre d'éditeur. Déjà ouvert : sa fenêtre revient au premier plan.
#[tauri::command]
pub async fn authoring_editor_open(
    app: AppHandle,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    path: String,
) -> EditorResult<EditorOpened> {
    let root = PathBuf::from(&path);
    let mut extra = commands::launcher_forbidden(&config.dir, &root);
    extra.push(install.cache_root().await?);
    let editor = blocking(move || {
        let forbidden = ForbiddenRoots::system().with_trees(&extra);
        Editor::open(&root, forbidden, extra, scan_limits())
    })
    .await??;
    let base = editor.base().to_path_buf();
    let folder_name = editor
        .base()
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    match editors.register(editor) {
        Registered::Existing(label) => {
            focus(&app, &label);
            Ok(EditorOpened { already_open: true })
        }
        Registered::Closing => Err(EditorError {
            kind: "editor_closing".into(),
            message: "l'éditeur de ce dossier vient d'être fermé et termine une opération : \
                      réessayez dans un instant"
                .into(),
        }),
        Registered::New(label) => {
            if let Err(e) = open_window(&app, &label, &folder_name) {
                editors.remove(&label);
                return Err(EditorError {
                    kind: "window".into(),
                    message: format!("fenêtre de l'éditeur impossible à ouvrir : {e}"),
                });
            }
            // La surveillance est un confort : sans elle, l'éditeur marche (vue relue après
            // chaque opération, état vérifié avant d'agir).
            let started = blocking({
                let (app, label) = (app.clone(), label.clone());
                move || editor_watch::start_for_window(&app, &label, &base)
            })
            .await;
            match started {
                Ok(Ok(watch)) => editors.set_watch(&label, watch),
                Ok(Err(e)) => tracing::warn!("surveillance du dossier impossible : {e}"),
                Err(e) => tracing::warn!("surveillance du dossier impossible : {e}"),
            }
            Ok(EditorOpened {
                already_open: false,
            })
        }
    }
}

/// Vue du dossier de la fenêtre qui appelle.
#[tauri::command]
pub async fn authoring_editor_view(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
) -> EditorResult<EditorView> {
    let session = editors.get(window.label())?;
    view_of(&config, &install, &session).await
}

/// Copie des fichiers ou dossiers choisis (sources) dans le menu `menu`.
#[tauri::command]
pub async fn authoring_editor_import(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    menu: String,
    sources: Vec<String>,
) -> EditorResult<EditorUpdate> {
    let sources = self::sources(sources)?;
    op_command(
        &window,
        &config,
        &install,
        &editors,
        Op::Import { menu, sources },
    )
    .await
}

/// Image de `target` remplacée par une copie de `source`.
#[tauri::command]
pub async fn authoring_editor_set_image(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    target: Target,
    source: String,
) -> EditorResult<EditorUpdate> {
    let source = self::source(source)?;
    op_command(
        &window,
        &config,
        &install,
        &editors,
        Op::SetImage { target, source },
    )
    .await
}

/// Titre lu de `target` remplacé par une copie du fichier audio `source`.
#[tauri::command]
pub async fn authoring_editor_set_title_audio(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    target: Target,
    source: String,
) -> EditorResult<EditorUpdate> {
    let source = self::source(source)?;
    let op = Op::SetTitleAudio { target, source };
    op_command(&window, &config, &install, &editors, op).await
}

/// « Garder » : l'image automatique du menu `target` (celle de sa première histoire) est
/// écrite dans le dossier.
#[tauri::command]
pub async fn authoring_editor_keep(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    target: Target,
) -> EditorResult<EditorUpdate> {
    op_command(&window, &config, &install, &editors, Op::Keep { target }).await
}

/// Options du menu `menu` rangées dans l'ordre `order` (noms).
#[tauri::command]
pub async fn authoring_editor_reorder(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    menu: String,
    order: Vec<String>,
) -> EditorResult<EditorUpdate> {
    op_command(
        &window,
        &config,
        &install,
        &editors,
        Op::Reorder { menu, order },
    )
    .await
}

/// `target` déplacé dans le menu `dest`, avant l'option `before` (nom) ou à la fin.
#[tauri::command]
pub async fn authoring_editor_move(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    target: Target,
    dest: String,
    before: Option<String>,
) -> EditorResult<EditorUpdate> {
    let op = Op::Move {
        target,
        dest,
        before,
    };
    op_command(&window, &config, &install, &editors, op).await
}

/// Titre affiché de `target` changé (avec `rename_file` : le fichier aussi).
#[tauri::command]
pub async fn authoring_editor_rename(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    target: Target,
    title: String,
    rename_file: bool,
) -> EditorResult<EditorUpdate> {
    let op = Op::Rename {
        target,
        title,
        rename_file,
    };
    op_command(&window, &config, &install, &editors, op).await
}

/// Nouveau sous-menu `name` dans le menu `parent`.
#[tauri::command]
pub async fn authoring_editor_new_menu(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    parent: String,
    name: String,
) -> EditorResult<EditorUpdate> {
    op_command(
        &window,
        &config,
        &install,
        &editors,
        Op::NewMenu { parent, name },
    )
    .await
}

/// `target` mis dans la corbeille de l'histoire.
#[tauri::command]
pub async fn authoring_editor_delete(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    target: Target,
) -> EditorResult<EditorUpdate> {
    op_command(&window, &config, &install, &editors, Op::Delete { target }).await
}

/// Réglages de l'histoire (`_histoire.json`).
#[tauri::command]
pub async fn authoring_editor_story_settings(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    patch: StoryPatch,
) -> EditorResult<EditorUpdate> {
    op_command(&window, &config, &install, &editors, Op::Story(patch)).await
}

/// Réglages du menu `menu` (`_dossier.json`).
#[tauri::command]
pub async fn authoring_editor_folder_settings(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    menu: String,
    patch: FolderPatch,
) -> EditorResult<EditorUpdate> {
    op_command(
        &window,
        &config,
        &install,
        &editors,
        Op::Folder { menu, patch },
    )
    .await
}

/// Annuler.
#[tauri::command]
pub async fn authoring_editor_undo(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
) -> EditorResult<EditorUpdate> {
    op_command(&window, &config, &install, &editors, Op::Undo).await
}

/// Rétablir.
#[tauri::command]
pub async fn authoring_editor_redo(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
) -> EditorResult<EditorUpdate> {
    op_command(&window, &config, &install, &editors, Op::Redo).await
}

/// « Vider la corbeille de l'histoire » : vers la Corbeille Windows.
#[tauri::command]
pub async fn authoring_editor_empty_trash(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
) -> EditorResult<EditorUpdate> {
    op_command(&window, &config, &install, &editors, Op::EmptyTrash).await
}

/// Vignette (URL `data:`) de l'image `id`, au plus `size` pixels de côté.
#[tauri::command]
pub async fn authoring_editor_thumbnail(
    window: WebviewWindow,
    editors: State<'_, Arc<EditorState>>,
    id: String,
    size: u32,
) -> EditorResult<String> {
    let label = window.label().to_owned();
    let state = editors.inner().clone();
    blocking(move || thumbnail_of(&state, &label, &id, size)).await?
}

/// « Ouvrir dans l'Explorateur » : le dossier de l'éditeur.
#[tauri::command]
pub async fn authoring_editor_reveal(
    window: WebviewWindow,
    editors: State<'_, Arc<EditorState>>,
) -> EditorResult<()> {
    use tauri_plugin_opener::OpenerExt;
    let base = base_of(&editors.get(window.label())?);
    window
        .app_handle()
        .opener()
        .open_path(base.to_string_lossy(), None::<&str>)
        .map_err(|e| EditorError::from(AppError::Io(std::io::Error::other(e.to_string()))))
}

/// « Créer l'histoire » / « Mettre à jour l'histoire » depuis l'éditeur (même parcours que
/// `authoring_create`) ; la tâche est signalée à la fenêtre principale ([`TASK_EVENT`]).
#[tauri::command]
pub async fn authoring_editor_create(
    window: WebviewWindow,
    config: State<'_, ConfigState>,
    install: State<'_, InstallService>,
    editors: State<'_, Arc<EditorState>>,
    choice: Option<ConflictChoice>,
) -> EditorResult<EditorCreated> {
    let session = editors.get(window.label())?;
    let base = base_of(&session);
    // La création écrit l'uuid et la version dans `_histoire.json` : pas un changement d'ailleurs.
    let quiet = editors.quiet(window.label());
    let created = commands::create_at(&config, &install, &base.to_string_lossy(), choice).await?;
    drop(quiet);
    let _ = window
        .app_handle()
        .emit_to("main", TASK_EVENT, &created.task);
    let view = view_of(&config, &install, &session).await?;
    Ok(EditorCreated { created, view })
}

/// Fenêtre détruite : l'éditeur d'une fenêtre d'éditeur est oublié ; la fenêtre principale
/// fermée, les fenêtres d'éditeur se ferment aussi (l'application ne reste pas ouverte sans
/// elle).
pub fn on_window_destroyed(window: &tauri::Window) {
    let label = window.label();
    if label.starts_with(EDITOR_LABEL_PREFIX) {
        if let Some(state) = window.try_state::<Arc<EditorState>>() {
            state.remove(label);
        }
    } else if label == "main" {
        for (other, webview) in window.app_handle().webview_windows() {
            if other.starts_with(EDITOR_LABEL_PREFIX) {
                let _ = webview.destroy();
            }
        }
    }
}

#[cfg(test)]
#[path = "ops_commands_tests.rs"]
mod tests;
