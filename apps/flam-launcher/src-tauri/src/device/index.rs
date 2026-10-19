//! Index des histoires d'une Flam : `etc/library/list` (histoires affichées, dans l'ordre) et
//! `etc/library/list.hidden` (histoires masquées). Un UUID par ligne.
//!
//! Ce module lit ces fichiers de façon tolérante, les réécrit sans jamais laisser l'appareil
//! sans index valide, et reprend au démarrage une écriture interrompue (Flam débranchée).
//! Aucune dépendance à Tauri : la racine de l'appareil est passée en paramètre.
//!
//! # Format écrit (normalisation)
//!
//! Chaque UUID en minuscules, au format `8-4-4-4-12`, suivi de `\n` (y compris le dernier ;
//! une liste vide donne un fichier vide). Les doublons sont retirés (première occurrence
//! gardée), l'ordre est conservé. À la lecture sont acceptés : BOM UTF-8, fins de ligne LF ou
//! CRLF, espaces autour de l'UUID, lignes vides, majuscules. Les lignes invalides et les
//! doublons sont ignorés et signalés ([`LineIssue`]).
//!
//! # Écriture sûre
//!
//! FAT n'a ni journal ni renommage atomique garanti : le remplacement d'un fichier peut, en
//! cas de coupure, laisser la cible absente alors que la source existe encore. La procédure
//! ne repose donc pas sur l'atomicité du renommage mais sur un ordre d'étapes dont chaque
//! état intermédiaire est reconnaissable par [`recover`] :
//!
//! 1. `list.new` écrit, synchronisé sur le disque, puis relu et comparé ;
//! 2. `list.bak` : copie de `list` (fichier vide si `list` était absent), synchronisée et
//!    relue. Sa présence garantit que `list.new` est complet (étape 1 terminée) ;
//! 3. `list.new` renommé en `list` (remplacement, écriture directe sur le disque) ;
//! 4. `list` relu et comparé ;
//! 5. `usr/0/library.cache` supprimé (le firmware le reconstruit) ;
//! 6. `list.bak` supprimé : la transaction est terminée.
//!
//! `list` n'est jamais supprimé ni tronqué avant l'étape 3, où le remplaçant est complet.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::storage::retry_transient;

/// Dossier des index, relatif à la racine de l'appareil.
pub const LIBRARY_DIR: &str = "etc/library";
/// Cache de bibliothèque du firmware, relatif à la racine. Reconstruit par le firmware au
/// démarrage s'il est absent.
pub const LIBRARY_CACHE: &str = "usr/0/library.cache";
/// Dossier des histoires installées, relatif à la racine.
pub const STORIES_DIR: &str = "str";
/// Taille maximale lue pour un index : au-delà, le fichier est jugé illisible (un index réel
/// fait quelques centaines d'octets, 37 octets par histoire).
pub const MAX_INDEX_BYTES: u64 = 1024 * 1024;
/// Longueur maximale du texte d'une ligne invalide recopié dans un [`LineIssue`].
const ISSUE_TEXT_MAX: usize = 64;

/// Index visé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    /// `list` : histoires affichées, dans l'ordre d'affichage.
    Visible,
    /// `list.hidden` : histoires masquées.
    Hidden,
}

impl IndexKind {
    /// Les deux index, dans l'ordre de reprise.
    pub const ALL: [IndexKind; 2] = [IndexKind::Visible, IndexKind::Hidden];

    fn file_name(self) -> &'static str {
        match self {
            IndexKind::Visible => "list",
            IndexKind::Hidden => "list.hidden",
        }
    }
}

/// Chemins d'un index et de ses fichiers de travail.
struct Paths {
    /// L'index lui-même.
    list: PathBuf,
    /// Remplaçant en cours d'écriture (étape 1).
    new: PathBuf,
    /// Sauvegarde de l'index et marqueur de transaction (étape 2).
    bak: PathBuf,
    /// Fichier de travail d'une restauration depuis `bak` pendant la reprise.
    restore: PathBuf,
    /// Cache du firmware à invalider.
    cache: PathBuf,
}

impl Paths {
    fn new(root: &Path, kind: IndexKind) -> Self {
        let dir = root.join(LIBRARY_DIR);
        let name = kind.file_name();
        Self {
            list: dir.join(name),
            new: dir.join(format!("{name}.new")),
            bak: dir.join(format!("{name}.bak")),
            restore: dir.join(format!("{name}.restore")),
            cache: root.join(LIBRARY_CACHE),
        }
    }
}

/// Identifiant d'histoire : UUID canonique en minuscules (`8-4-4-4-12`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoryId(String);

impl StoryId {
    /// Reconnaît un UUID `8-4-4-4-12` (chiffres hexadécimaux, casse indifférente), sans
    /// espaces ni accolades. Rend la forme en minuscules.
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 36 {
            return None;
        }
        let ok = bytes.iter().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => *b == b'-',
            _ => b.is_ascii_hexdigit(),
        });
        ok.then(|| StoryId(text.to_ascii_lowercase()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for StoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Anomalie d'une ligne, ignorée à la lecture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineIssue {
    /// Ligne non vide qui n'est pas un UUID (texte tronqué à 64 caractères).
    Invalid { line: usize, text: String },
    /// UUID déjà présent plus haut.
    Duplicate { line: usize, id: StoryId },
}

/// État d'un index lu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexState {
    /// Fichier absent.
    Absent,
    /// Toutes les lignes non vides sont des UUID distincts.
    Valid,
    /// Lisible, mais des lignes ont été ignorées (voir `issues`).
    Damaged,
    /// Illisible : erreur de lecture, fichier trop gros, octet nul, ou contenu non vide sans
    /// aucun UUID. Rien n'est réécrit sans l'accord explicite de l'utilisateur.
    Unreadable(String),
}

/// Résultat d'une lecture tolérante.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRead {
    pub state: IndexState,
    /// Histoires reconnues, dans l'ordre du fichier, sans doublon.
    pub entries: Vec<StoryId>,
    /// Lignes ignorées.
    pub issues: Vec<LineIssue>,
}

impl IndexRead {
    fn unreadable(reason: String) -> Self {
        Self {
            state: IndexState::Unreadable(reason),
            entries: Vec::new(),
            issues: Vec::new(),
        }
    }

    /// Index exploitable : présent et lisible (éventuellement avec des lignes ignorées).
    fn usable(&self) -> bool {
        matches!(self.state, IndexState::Valid | IndexState::Damaged)
    }
}

/// Accord de l'utilisateur pour réécrire un index absent ou abîmé.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consent {
    /// Réécrit seulement un index valide (ou `list.hidden` absent).
    IfValid,
    /// L'utilisateur a confirmé : réécrit même un index absent, abîmé ou illisible (par
    /// exemple une reconstruction depuis [`scan_stories`]).
    Overwrite,
}

/// Résultat d'une écriture réussie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteOutcome {
    /// Le contenu normalisé était déjà celui de l'index : rien n'a été touché.
    Unchanged,
    /// Index remplacé et cache du firmware supprimé.
    Written,
}

/// Résultat d'une reprise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Aucune écriture interrompue.
    Clean,
    /// Écriture interrompue avant le remplacement : l'index d'origine est gardé, les fichiers
    /// de travail sont supprimés.
    RolledBack,
    /// Écriture interrompue après que le remplaçant a été complet : la transaction est
    /// terminée (remplacement si besoin, cache supprimé).
    RolledForward,
    /// Index perdu pendant le remplacement : l'ancien contenu est restauré depuis `list.bak`.
    RestoredFromBackup,
    /// Aucun candidat exploitable : rien n'est touché, décision de l'utilisateur requise
    /// (reconstruction depuis `str/` avec [`Consent::Overwrite`]).
    NeedsUser,
}

/// Erreur d'écriture ou de reprise.
#[derive(Debug, thiserror::Error)]
pub enum IndexError {
    #[error(
        "index absent, abîmé ou illisible : l'accord de l'utilisateur est requis pour le réécrire"
    )]
    NeedsConsent,
    #[error("fichier de travail de l'index impossible à résoudre : {0}")]
    Leftover(String),
    #[error("vérification de {0} échouée : le contenu relu diffère du contenu écrit")]
    Verify(String),
    #[error("index écrit, mais le cache du firmware n'a pas pu être supprimé ({0}) ; nouvel essai à la prochaine reprise")]
    CacheNotRemoved(#[source] io::Error),
    #[error("{file} : {source}")]
    Io {
        file: String,
        #[source]
        source: io::Error,
    },
}

fn io_err(path: &Path) -> impl FnOnce(io::Error) -> IndexError + '_ {
    move |source| IndexError::Io {
        file: display_name(path),
        source,
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Étapes d'écriture, après lesquelles un test peut simuler une coupure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    NewWritten,
    BackupWritten,
    Replaced,
    Verified,
    CacheRemoved,
}

/// Lit les octets d'un fichier, `None` s'il est absent, en réessayant si le fichier est
/// brièvement verrouillé. Lit au plus `MAX_INDEX_BYTES + 1` octets : un fichier plus gros est
/// jugé illisible par [`parse`].
fn read_raw(path: &Path) -> io::Result<Option<Vec<u8>>> {
    let read = || -> io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_INDEX_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    match retry_transient(read, std::thread::sleep) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Analyse tolérante du contenu d'un index.
pub fn parse(bytes: &[u8]) -> IndexRead {
    if bytes.len() as u64 > MAX_INDEX_BYTES {
        return IndexRead::unreadable(format!("plus de {MAX_INDEX_BYTES} octets"));
    }
    if bytes.contains(&0) {
        return IndexRead::unreadable("octet nul dans le fichier".into());
    }
    let text = String::from_utf8_lossy(bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut entries: Vec<StoryId> = Vec::new();
    let mut seen = HashSet::new();
    let mut issues = Vec::new();
    let mut non_blank = false;
    for (i, raw) in text.split('\n').enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        non_blank = true;
        match StoryId::parse(line) {
            Some(id) if seen.contains(&id) => {
                issues.push(LineIssue::Duplicate { line: i + 1, id });
            }
            Some(id) => {
                seen.insert(id.clone());
                entries.push(id);
            }
            None => issues.push(LineIssue::Invalid {
                line: i + 1,
                text: line.chars().take(ISSUE_TEXT_MAX).collect(),
            }),
        }
    }
    if non_blank && entries.is_empty() {
        return IndexRead::unreadable("aucun UUID reconnu".into());
    }
    let state = if issues.is_empty() {
        IndexState::Valid
    } else {
        IndexState::Damaged
    };
    IndexRead {
        state,
        entries,
        issues,
    }
}

/// Lit un fichier d'index : `(octets bruts, analyse)`. Une erreur d'entrée/sortie (fichier
/// verrouillé, clé défaillante) est rendue telle quelle : elle ne dit rien du contenu et ne
/// doit jamais faire remplacer le fichier.
fn load(path: &Path) -> Result<(Option<Vec<u8>>, IndexRead), IndexError> {
    Ok(match read_raw(path).map_err(io_err(path))? {
        None => (
            None,
            IndexRead {
                state: IndexState::Absent,
                entries: Vec::new(),
                issues: Vec::new(),
            },
        ),
        Some(bytes) => {
            let read = parse(&bytes);
            (Some(bytes), read)
        }
    })
}

/// Lecture tolérante d'un index de l'appareil. Une erreur de lecture donne
/// [`IndexState::Unreadable`]. Ne lance pas de reprise : appeler [`recover`] d'abord pour voir
/// l'état après une écriture interrompue.
pub fn read_index(root: &Path, kind: IndexKind) -> IndexRead {
    load(&Paths::new(root, kind).list)
        .map(|(_, read)| read)
        .unwrap_or_else(|err| IndexRead::unreadable(err.to_string()))
}

/// Contenu normalisé : UUID distincts, dans l'ordre, chacun suivi de `\n`.
pub fn serialize(entries: &[StoryId]) -> Vec<u8> {
    let mut seen = HashSet::with_capacity(entries.len());
    let mut out = Vec::with_capacity(entries.len() * 37);
    for id in entries {
        if seen.insert(id) {
            out.extend_from_slice(id.as_str().as_bytes());
            out.push(b'\n');
        }
    }
    out
}

/// Histoires de l'index sans dossier `str/<uuid>` sur l'appareil.
pub fn missing_stories(root: &Path, entries: &[StoryId]) -> Vec<StoryId> {
    let dir = root.join(STORIES_DIR);
    entries
        .iter()
        .filter(|id| !dir.join(id.as_str()).is_dir())
        .cloned()
        .collect()
}

/// Dossiers d'histoires présents dans `str/` (nom au format UUID), triés. Base d'une
/// reconstruction de l'index proposée à l'utilisateur quand l'index est illisible.
pub fn scan_stories(root: &Path) -> io::Result<Vec<StoryId>> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(root.join(STORIES_DIR))? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if let Some(id) = entry.file_name().to_str().and_then(StoryId::parse) {
            ids.push(id);
        }
    }
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// Crée (ou écrase) `path` avec `bytes`, synchronise sur le disque, puis relit et compare.
fn write_verified(path: &Path, bytes: &[u8]) -> Result<(), IndexError> {
    let mut file = fs::File::create(path).map_err(io_err(path))?;
    file.write_all(bytes).map_err(io_err(path))?;
    file.sync_all().map_err(io_err(path))?;
    drop(file);
    verify(path, bytes)
}

fn verify(path: &Path, bytes: &[u8]) -> Result<(), IndexError> {
    match read_raw(path).map_err(io_err(path))? {
        Some(read) if read == bytes => Ok(()),
        _ => Err(IndexError::Verify(display_name(path))),
    }
}

/// Supprime un fichier ; absent = succès.
fn remove_if_exists(path: &Path) -> io::Result<()> {
    match retry_transient(|| fs::remove_file(path), std::thread::sleep) {
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

fn remove(path: &Path) -> Result<(), IndexError> {
    remove_if_exists(path).map_err(io_err(path))
}

fn remove_cache(path: &Path) -> Result<(), IndexError> {
    remove_if_exists(path).map_err(IndexError::CacheNotRemoved)
}

/// Renomme `from` en `to` en remplaçant `to`, puis synchronise `to` (FlushFileBuffers : son
/// entrée de répertoire est écrite). `MOVEFILE_WRITE_THROUGH` ne garantit rien de plus pour
/// un renommage sur le même volume. Une coupure avant la synchronisation peut perdre le
/// renommage : la reprise reconnaît chaque état laissé (voir [`recover`]).
fn replace_file(from: &Path, to: &Path) -> Result<(), IndexError> {
    retry_transient(|| replace_once(from, to), std::thread::sleep).map_err(io_err(to))?;
    let sync = || fs::OpenOptions::new().write(true).open(to)?.sync_all();
    retry_transient(sync, std::thread::sleep).map_err(io_err(to))
}

#[cfg(windows)]
fn replace_once(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };
    let wide = |p: &Path| -> Vec<u16> { p.as_os_str().encode_wide().chain(Some(0)).collect() };
    let (from, to) = (wide(from), wide(to));
    // SAFETY : deux chaînes UTF-16 terminées par un nul, vivantes pendant tout l'appel.
    let ok = unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn replace_once(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

/// Reprend une écriture interrompue de l'index `kind`. À appeler à chaque détection de
/// l'appareil, avant toute lecture ou écriture.
///
/// | `new` | `bak` | `list`                 | décision                                    |
/// |-------|-------|------------------------|---------------------------------------------|
/// | non   | non   | quelconque             | `Clean`                                     |
/// | oui   | non   | quelconque             | `new` supprimé : `RolledBack`               |
/// | oui   | oui   | exploitable, = `new`   | `new`, cache, `bak` supprimés : `RolledForward` |
/// | oui   | oui   | exploitable, commence par `bak` | `bak` puis `new` supprimés : `RolledBack` |
/// | oui   | oui   | autre (absent, illisible, vide…) | `new` promu si valide, sinon `bak` restauré |
/// | non   | oui   | valide                 | cache puis `bak` supprimés : `RolledForward` |
/// | non   | oui   | absent, abîmé, illisible | `bak` restauré : `RestoredFromBackup`      |
///
/// « Commence par `bak` » : `list` est l'index d'origine et `bak` en est une copie, peut-être
/// partielle (coupure pendant l'étape 2). Sans `new`, un `list` abîmé n'est pas celui que nous
/// avons écrit (toujours valide) : `bak` est restauré. Sans candidat exploitable : `NeedsUser`,
/// rien n'est touché. Une erreur d'entrée/sortie à la lecture interrompt la reprise sans rien
/// toucher. Un `list.restore` résiduel (restauration interrompue) n'est jamais une source : il
/// est supprimé d'abord.
pub fn recover(root: &Path, kind: IndexKind) -> Result<Recovery, IndexError> {
    let p = Paths::new(root, kind);
    let new_present = exists(&p.new)?;
    let bak_present = exists(&p.bak)?;
    if exists(&p.restore)? {
        remove(&p.restore)?;
    }
    if !new_present && !bak_present {
        return Ok(Recovery::Clean);
    }
    if !bak_present {
        // Étape 2 jamais atteinte : `list` est intact (ou absent comme avant l'écriture).
        remove(&p.new)?;
        return Ok(Recovery::RolledBack);
    }
    let (list_raw, list_read) = load(&p.list)?;
    if new_present {
        let (new_raw, new_read) = load(&p.new)?;
        let (bak_raw, _) = load(&p.bak)?;
        if list_read.usable() {
            if list_raw.is_some() && list_raw == new_raw {
                // Remplacement fait, mais l'entrée `new` est restée.
                remove(&p.new)?;
                remove_cache(&p.cache)?;
                remove(&p.bak)?;
                return Ok(Recovery::RolledForward);
            }
            let list = list_raw.as_deref().unwrap_or_default();
            if bak_raw.as_deref().is_some_and(|bak| list.starts_with(bak)) {
                // Remplacement jamais fait : `list` est l'index d'origine, intact.
                remove(&p.bak)?;
                remove(&p.new)?;
                return Ok(Recovery::RolledBack);
            }
        }
        if new_read.state == IndexState::Valid {
            // `bak` présent : l'étape 1 était terminée, `new` est complet (un fichier vide
            // est une liste cible vide).
            replace_file(&p.new, &p.list)?;
            remove_cache(&p.cache)?;
            remove(&p.bak)?;
            return Ok(Recovery::RolledForward);
        }
        return restore_from_backup(&p, true);
    }
    if list_read.state == IndexState::Valid {
        // Étapes 3 et 4 faites : finir la transaction.
        remove_cache(&p.cache)?;
        remove(&p.bak)?;
        return Ok(Recovery::RolledForward);
    }
    restore_from_backup(&p, false)
}

/// Restaure `list` depuis `bak` via `list.restore`, puis termine la transaction.
fn restore_from_backup(p: &Paths, new_present: bool) -> Result<Recovery, IndexError> {
    let (bak_raw, bak_read) = load(&p.bak)?;
    // Un `bak` vide (index absent ou vide avant l'écriture) est valide.
    let Some(bak_raw) = bak_raw.filter(|_| bak_read.usable()) else {
        return Ok(Recovery::NeedsUser);
    };
    write_verified(&p.restore, &bak_raw)?;
    replace_file(&p.restore, &p.list)?;
    if new_present {
        remove(&p.new)?;
    }
    remove_cache(&p.cache)?;
    remove(&p.bak)?;
    Ok(Recovery::RestoredFromBackup)
}

fn exists(path: &Path) -> Result<bool, IndexError> {
    path.try_exists().map_err(io_err(path))
}

/// Remplace l'index `kind` par `entries` (normalisé, voir le module). Reprend d'abord une
/// écriture interrompue. Sans [`Consent::Overwrite`], refuse de réécrire un index absent
/// (sauf `list.hidden`), abîmé ou illisible. En cas d'échec, une reprise est tentée aussitôt :
/// l'index reste l'ancien ou devient le nouveau, jamais vide ni tronqué.
///
/// Les appelants sérialisent leurs appels à `write_index` et [`recover`] pour un même
/// appareil : il n'y a de verrou ni entre processus ni entre fils d'exécution.
pub fn write_index(
    root: &Path,
    kind: IndexKind,
    entries: &[StoryId],
    consent: Consent,
) -> Result<WriteOutcome, IndexError> {
    let result = write_with(root, kind, entries, consent, &mut |_| Ok(()));
    if let Err(err) = &result {
        if !matches!(err, IndexError::NeedsConsent | IndexError::Leftover(_)) {
            if let Err(again) = recover(root, kind) {
                tracing::warn!("reprise de l'index après échec impossible : {again}");
            }
        }
    }
    result
}

/// Corps de [`write_index`]. `hook` est appelé après chaque étape ; une erreur rendue par
/// `hook` interrompt l'écriture sans aucun nettoyage, comme une coupure de courant.
pub(crate) fn write_with(
    root: &Path,
    kind: IndexKind,
    entries: &[StoryId],
    consent: Consent,
    hook: &mut dyn FnMut(Step) -> io::Result<()>,
) -> Result<WriteOutcome, IndexError> {
    let p = Paths::new(root, kind);
    let cut = |source| IndexError::Io {
        file: "coupure simulée".into(),
        source,
    };

    if recover(root, kind)? == Recovery::NeedsUser {
        if consent != Consent::Overwrite {
            return Err(IndexError::NeedsConsent);
        }
        // Accord explicite : les fichiers de travail inexploitables sont abandonnés.
        remove(&p.new)?;
        remove(&p.bak)?;
    }
    for leftover in [&p.new, &p.bak, &p.restore] {
        if exists(leftover)? {
            return Err(IndexError::Leftover(display_name(leftover)));
        }
    }

    let (list_raw, list_read) = load(&p.list)?;
    let allowed = match list_read.state {
        IndexState::Valid => true,
        IndexState::Absent => kind == IndexKind::Hidden,
        IndexState::Damaged | IndexState::Unreadable(_) => false,
    };
    if !allowed && consent != Consent::Overwrite {
        return Err(IndexError::NeedsConsent);
    }
    let bytes = serialize(entries);
    if list_read.state == IndexState::Valid && list_raw.as_deref() == Some(&bytes[..]) {
        return Ok(WriteOutcome::Unchanged);
    }

    let dir = p.list.parent().expect("chemin d'index sous etc/library");
    fs::create_dir_all(dir).map_err(io_err(dir))?;

    // 1. Remplaçant complet et vérifié.
    write_verified(&p.new, &bytes)?;
    hook(Step::NewWritten).map_err(cut)?;
    // 2. Sauvegarde de l'existant : marque aussi que `new` est complet.
    write_verified(&p.bak, list_raw.as_deref().unwrap_or_default())?;
    hook(Step::BackupWritten).map_err(cut)?;
    // 3. Remplacement.
    replace_file(&p.new, &p.list)?;
    hook(Step::Replaced).map_err(cut)?;
    // 4. Vérification. Contenu relu différent : l'ancien index est restauré depuis `bak`
    // (vérifié à l'étape 2) ; la reprise générique garderait un `list` valide mais faux.
    if let Err(err) = verify(&p.list, &bytes) {
        if matches!(err, IndexError::Verify(_)) {
            restore_from_backup(&p, false)?;
        }
        return Err(err);
    }
    hook(Step::Verified).map_err(cut)?;
    // 5. Cache du firmware, seulement après une écriture réussie.
    remove_cache(&p.cache)?;
    hook(Step::CacheRemoved).map_err(cut)?;
    // 6. Fin de transaction.
    remove(&p.bak)?;
    Ok(WriteOutcome::Written)
}

#[cfg(test)]
mod tests;
