//! Opérations publiques de l'éditeur (§ 4.2) : chacune lit le dossier à nouveau, vérifie
//! l'état attendu, puis s'exprime en actions élémentaires réversibles (`super::Action`).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use super::{
    ancestor_ids, base_key, is_audio, is_image, is_link_meta, join, name_of, parent_of, parse_id,
    read_bounded, sanitize_name, stem_of, valid_component, Action, Doc, Done, Editor, FolderPatch,
    ImportReport, OpError, OpResult, Skipped, Stamp, StoryPatch, Taken, Target, Tx,
    MAX_IMAGE_BYTES,
};
use crate::authoring::settings::{
    self, name_key, FolderSettings, FOLDER_FILE, MAX_AGE, MAX_SETTINGS_BYTES, MAX_TEXT_CHARS,
    STORY_FILE,
};
use crate::authoring::tree::{strip_sort_prefix, title_from_name, Node, Scan};
use crate::install::studio::clean_line;
use crate::story::MAX_TITLE_CHARS;

/// Fichiers ignorés sans rien dire dans une source (comparaison sans casse).
const SILENT_FILES: [&str; 3] = ["desktop.ini", "thumbs.db", ".ds_store"];

fn find<'s>(scan: &'s Scan, id: &str) -> OpResult<&'s Node> {
    scan.root
        .find(id)
        .ok_or_else(|| OpError::NotFound(id.to_owned()))
}

fn require_menu<'s>(scan: &'s Scan, id: &str) -> OpResult<&'s Node> {
    match find(scan, id)? {
        node @ Node::Menu { .. } => Ok(node),
        Node::Story { .. } => Err(OpError::Refused(format!("« {id} » n'est pas un menu"))),
    }
}

/// Nom sur le disque d'une option de menu.
fn entry_name(node: &Node) -> &str {
    match node {
        Node::Menu { dir, .. } => dir,
        Node::Story { file, .. } => file,
    }
}

/// Noms des options du menu `node`, dans l'ordre affiché.
fn child_names(node: &Node) -> Vec<String> {
    node.children()
        .iter()
        .map(|c| entry_name(c).to_owned())
        .collect()
}

/// Base et reste d'un nom de fichier : `Loup.titre.mp3` → (`Loup`, `.titre.mp3`).
fn split_base(name: &str) -> (&str, &str) {
    let stem = stem_of(name);
    let base = if stem.to_lowercase().ends_with(".titre") && stem.len() > ".titre".len() {
        &stem[..stem.len() - ".titre".len()]
    } else {
        stem
    };
    (base, &name[base.len()..])
}

fn settings_error(e: crate::AppError) -> OpError {
    match e {
        crate::AppError::Io(io) => OpError::Io(io),
        other => OpError::Settings(other.to_string()),
    }
}

fn source_error(path: &Path, reason: &'static str) -> OpError {
    OpError::Source {
        path: path.to_string_lossy().into_owned(),
        reason,
    }
}

fn is_hidden_meta(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & (0x2 | 0x4) != 0
    }
    #[cfg(not(windows))]
    {
        let _ = meta;
        false
    }
}

/// Copie `src` (lu sans suivre de lien, `expected` octets) vers `dst` (créé, jamais écrasé),
/// avec sa date de modification.
fn copy_file(src: &Path, dst: &Path, expected: u64) -> OpResult<()> {
    let file = crate::install::pack::open_no_follow(src).map_err(|e| match e {
        crate::AppError::Io(io) if io.kind() != io::ErrorKind::NotFound => OpError::Io(io),
        crate::AppError::Io(_) => source_error(src, "introuvable"),
        _ => source_error(src, "lien, jonction ou fichier « à la demande »"),
    })?;
    let meta = file.metadata()?;
    if meta.len() != expected {
        return Err(source_error(src, "modifié pendant la copie"));
    }
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    let copied = io::copy(&mut file.take(expected + 1), &mut out)?;
    if copied != expected {
        return Err(source_error(src, "modifié pendant la copie"));
    }
    if let Ok(modified) = meta.modified() {
        let _ = out.set_modified(modified);
    }
    out.sync_all()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Préparation d'une copie
// ---------------------------------------------------------------------------

struct SourceFile {
    src: PathBuf,
    name: String,
    len: u64,
}

/// Dossier déposé : arborescence à recréer (chemins relatifs au dossier déposé).
struct SourceDir {
    name: String,
    dirs: Vec<String>,
    files: Vec<(String, PathBuf, u64)>,
}

struct Plan<'l> {
    limits: &'l super::ImportLimits,
    loose: Vec<SourceFile>,
    folders: Vec<SourceDir>,
    skipped: Vec<Skipped>,
    files: usize,
    bytes: u64,
    entries: usize,
}

impl Plan<'_> {
    fn skip(&mut self, path: &Path, reason: &'static str) {
        self.skipped.push(Skipped {
            path: path.to_string_lossy().into_owned(),
            reason,
        });
    }

    fn count_file(&mut self, len: u64) -> OpResult<()> {
        self.files += 1;
        self.bytes = self.bytes.saturating_add(len);
        if self.files > self.limits.max_files {
            return Err(OpError::TooLarge(format!(
                "plus de {} fichiers",
                self.limits.max_files
            )));
        }
        if self.bytes > self.limits.max_bytes {
            return Err(OpError::TooLarge(format!(
                "plus de {} Mio",
                self.limits.max_bytes / (1024 * 1024)
            )));
        }
        Ok(())
    }

    fn count_entry(&mut self) -> OpResult<()> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(OpError::TooLarge(format!(
                "plus de {} fichiers et dossiers à parcourir",
                self.limits.max_entries
            )));
        }
        Ok(())
    }

    /// Fichier retenu ? (`None` : ignoré sans rien dire ; `Some(Err)` : ignoré et signalé).
    fn accept_file(name: &str, meta: &fs::Metadata) -> Option<Result<(), &'static str>> {
        let lower = name.to_lowercase();
        if name.starts_with('.') || SILENT_FILES.contains(&lower.as_str()) || is_hidden_meta(meta) {
            return None;
        }
        if !valid_component(name) {
            return Some(Err("nom de fichier refusé"));
        }
        if !(is_audio(name) || is_image(name)) {
            return Some(Err("format non pris en charge"));
        }
        Some(Ok(()))
    }

    /// Ajoute la source `src` (chemin absolu venu de l'Explorateur).
    fn add_source(&mut self, ed: &Editor, src: &Path) -> OpResult<()> {
        if !src.is_absolute() {
            return Err(source_error(src, "chemin relatif"));
        }
        let meta = match fs::symlink_metadata(src) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Err(source_error(src, "introuvable"))
            }
            Err(e) => return Err(e.into()),
        };
        if is_link_meta(&meta) {
            return Err(source_error(
                src,
                "lien, jonction ou fichier « à la demande »",
            ));
        }
        // Par identité : ni dans le dossier de l'histoire (copie sur soi-même), ni parent de
        // celui-ci (copie sans fin).
        let src_ids = ancestor_ids(src);
        if src_ids.contains(&ed.root_id) {
            return Err(source_error(src, "déjà dans le dossier de l'histoire"));
        }
        if let Some(own) = src_ids.first() {
            if ancestor_ids(&ed.base).contains(own) {
                return Err(source_error(src, "contient le dossier de l'histoire"));
            }
        }
        let name = src
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_owned)
            .ok_or_else(|| source_error(src, "nom illisible"))?;
        if meta.is_file() {
            match Self::accept_file(&name, &meta) {
                None => self.skip(src, "fichier caché ou système"),
                Some(Err(reason)) => self.skip(src, reason),
                Some(Ok(())) => {
                    self.count_file(meta.len())?;
                    self.loose.push(SourceFile {
                        src: src.to_path_buf(),
                        name,
                        len: meta.len(),
                    });
                }
            }
            return Ok(());
        }
        if !meta.is_dir() {
            self.skip(src, "ni fichier ni dossier");
            return Ok(());
        }
        let Some(name) = sanitize_name(&name) else {
            self.skip(src, "nom de dossier refusé");
            return Ok(());
        };
        let mut folder = SourceDir {
            name,
            dirs: Vec::new(),
            files: Vec::new(),
        };
        // Parcours en largeur, sans suivre de lien, borné.
        let mut stack = vec![(src.to_path_buf(), String::new(), 1usize)];
        while let Some((dir, rel, depth)) = stack.pop() {
            let mut entries = Vec::new();
            for entry in fs::read_dir(&dir)? {
                self.count_entry()?;
                entries.push(entry?);
            }
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let path = entry.path();
                let Ok(name) = entry.file_name().into_string() else {
                    self.skip(&path, "nom illisible");
                    continue;
                };
                let meta = fs::symlink_metadata(&path)?;
                if is_link_meta(&meta) {
                    self.skip(&path, "lien, jonction ou fichier « à la demande »");
                    continue;
                }
                let child = join(&rel, &name);
                if meta.is_dir() {
                    if name.starts_with('.') || is_hidden_meta(&meta) {
                        continue;
                    }
                    if !valid_component(&name) {
                        self.skip(&path, "nom de dossier refusé");
                        continue;
                    }
                    if depth + 1 > self.limits.max_depth {
                        return Err(OpError::TooLarge(format!(
                            "plus de {} niveaux de dossiers",
                            self.limits.max_depth
                        )));
                    }
                    stack.push((path, child, depth + 1));
                } else if meta.is_file() {
                    match Self::accept_file(&name, &meta) {
                        None => {}
                        Some(Err(reason)) => self.skip(&path, reason),
                        Some(Ok(())) => {
                            self.count_file(meta.len())?;
                            folder.files.push((child, path, meta.len()));
                        }
                    }
                }
            }
        }
        if folder.files.is_empty() {
            self.skip(src, "aucun fichier audio ou image");
            return Ok(());
        }
        // Seuls les dossiers qui mènent à un fichier copié sont recréés.
        let mut dirs: HashSet<String> = HashSet::new();
        for (rel, _, _) in &folder.files {
            let mut parent = parent_of(rel);
            while !parent.is_empty() && dirs.insert(parent.to_owned()) {
                parent = parent_of(parent);
            }
        }
        let mut dirs: Vec<String> = dirs.into_iter().collect();
        dirs.sort_by_key(|d| d.matches('/').count());
        folder.dirs = dirs;
        self.folders.push(folder);
        Ok(())
    }
}

/// Fichiers isolés regroupés par base (audio, image et titre lu ensemble) ; deux fichiers de
/// même nom (venus de dossiers différents) vont dans deux groupes.
fn group_loose(files: Vec<SourceFile>) -> Vec<Vec<SourceFile>> {
    let mut groups: Vec<Vec<SourceFile>> = Vec::new();
    for file in files {
        let base = base_key(&file.name);
        let key = name_key(&file.name);
        let slot = groups
            .iter_mut()
            .find(|g| base_key(&g[0].name) == base && !g.iter().any(|f| name_key(&f.name) == key));
        match slot {
            Some(group) => group.push(file),
            None => groups.push(vec![file]),
        }
    }
    groups
}

// ---------------------------------------------------------------------------
// Aides des opérations
// ---------------------------------------------------------------------------

impl Editor {
    /// Vérifie que `target` existe dans l'état vu par l'éditeur ; rend son id validé.
    fn check_target(&self, target: &Target) -> OpResult<String> {
        let id = parse_id(&target.id)?;
        let meta = self
            .lstat(&id)?
            .ok_or_else(|| OpError::Changed(id.clone()))?;
        if is_link_meta(&meta) {
            return Err(OpError::Link(id));
        }
        if !Stamp::of(&meta).matches(&target.stamp) {
            return Err(OpError::Changed(id));
        }
        Ok(id)
    }

    /// État actuel de `rel` (fichier ou dossier ordinaire).
    fn current_stamp(&self, rel: &str) -> OpResult<Stamp> {
        let meta = self
            .lstat(rel)?
            .ok_or_else(|| OpError::Changed(rel.to_owned()))?;
        if is_link_meta(&meta) {
            return Err(OpError::Link(rel.to_owned()));
        }
        Ok(Stamp::of(&meta))
    }

    /// Image et titre lu du fichier audio `file` du dossier `dir` (noms), sauf si un autre
    /// fichier audio partage sa base : ils restent alors en place.
    fn companions(&self, dir: &str, file: &str) -> OpResult<Vec<String>> {
        let stem = name_key(stem_of(file));
        let titled = format!("{stem}.titre");
        let file_key = name_key(file);
        let mut out = Vec::new();
        for name in self.names(dir)? {
            let key = name_key(&name);
            if key == file_key {
                continue;
            }
            let name_stem = name_key(stem_of(&name));
            if is_audio(&name) && name_stem == stem {
                return Ok(Vec::new());
            }
            let companion =
                (is_image(&name) && name_stem == stem) || (is_audio(&name) && name_stem == titled);
            if companion {
                let meta = self.lstat(&join(dir, &name))?;
                if meta.is_some_and(|m| m.is_file() && !is_link_meta(&m)) {
                    out.push(name);
                }
            }
        }
        out.sort();
        Ok(out)
    }

    /// Titre effectif de l'entrée `name` du dossier `dir` : celui des réglages, sinon `None`.
    fn explicit_title(&self, dir: &str, name: &str) -> Option<String> {
        let abs = self.abs(dir);
        let folder = if dir.is_empty() {
            settings::load_story(&abs).settings.folder
        } else {
            settings::load_folder(&abs).settings
        };
        folder.title_of(name).map(str::to_owned)
    }
}

impl Tx<'_> {
    /// Met l'élément préparé `staged` (dossier de travail) en place sous `path`, sans écraser.
    fn place(&mut self, staged: &str, path: &str) -> OpResult<()> {
        let ed = self.ed;
        let _from = ed.pin_dir(parent_of(staged))?;
        let _to = ed.pin_dir(parent_of(path))?;
        if ed.lstat(path)?.is_some() {
            return Err(OpError::Changed(path.to_owned()));
        }
        match super::fsx::rename_noreplace(&ed.abs(staged), &ed.abs(path)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(OpError::Changed(path.to_owned()))
            }
            Err(e) => return Err(e.into()),
        }
        let stamp = ed.current_stamp(path)?;
        self.record(Action::Created {
            path: path.to_owned(),
            stamp,
        });
        Ok(())
    }

    /// Crée le dossier vide `path`.
    fn make_dir(&mut self, path: &str) -> OpResult<()> {
        let ed = self.ed;
        let _pins = ed.pin_dir(parent_of(path))?;
        match fs::create_dir(ed.abs(path)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                return Err(OpError::Changed(path.to_owned()))
            }
            Err(e) => return Err(e.into()),
        }
        let stamp = ed.current_stamp(path)?;
        self.record(Action::Created {
            path: path.to_owned(),
            stamp,
        });
        Ok(())
    }

    /// Déplace `rel` (dans l'état `stamp`) dans la place de corbeille `slot`.
    fn trash_into(&mut self, slot: &str, rel: &str, stamp: Stamp) -> OpResult<()> {
        self.apply(Action::Move {
            from: rel.to_owned(),
            to: join(slot, name_of(rel)),
            stamp,
        })
    }

    /// Modifie les réglages du dossier `dir` (racine : `_histoire.json`) par `change`, sur une
    /// lecture faite maintenant. Rien n'est écrit si `change` ne change rien. Un fichier abîmé
    /// est d'abord mis à la corbeille (§ 4.5) ; un format plus récent est refusé.
    fn write_settings(
        &mut self,
        dir: &str,
        change: impl FnOnce(&mut Doc) -> OpResult<()>,
    ) -> OpResult<()> {
        let ed = self.ed;
        let file = if dir.is_empty() {
            STORY_FILE
        } else {
            FOLDER_FILE
        };
        let path = join(dir, file);
        let pins = ed.pin_dir(dir)?;
        let before = read_bounded(&ed.abs(&path), MAX_SETTINGS_BYTES, &path)?;
        let abs_dir = ed.abs(dir);
        let (mut doc, damaged) = if dir.is_empty() {
            let loaded = settings::load_story(&abs_dir);
            (Doc::Story(loaded.settings), loaded.damaged)
        } else {
            let loaded = settings::load_folder(&abs_dir);
            (Doc::Folder(loaded.settings), loaded.damaged)
        };
        let original = doc.clone();
        change(&mut doc)?;
        if doc == original {
            return Ok(());
        }
        let existing: HashSet<String> = ed.names(dir)?.iter().map(|n| name_key(n)).collect();
        doc.folder().prune(&existing);
        drop(pins);

        let work = ed.work_slot()?;
        let result = (|| -> OpResult<()> {
            let work_abs = ed.abs(&work);
            match &doc {
                Doc::Story(story) => settings::write_story(&work_abs, story),
                Doc::Folder(folder) => settings::write_folder(&work_abs, folder),
            }
            .map_err(settings_error)?;
            let staged = join(&work, file);
            let after = read_bounded(&ed.abs(&staged), MAX_SETTINGS_BYTES, &staged)?
                .ok_or_else(|| OpError::Changed(staged.clone()))?;
            match before {
                None => self.place(&staged, &path),
                Some(_) if damaged => {
                    // Gardé dans la corbeille, jamais écrasé.
                    let stamp = ed.current_stamp(&path)?;
                    let slot = ed.trash_slot()?;
                    self.trash_into(&slot, &path, stamp)?;
                    self.place(&staged, &path)
                }
                Some(before) if before == after => Ok(()),
                Some(before) => self.apply(Action::Json {
                    path: path.clone(),
                    before,
                    after,
                }),
            }
        })();
        ed.drop_work_slot(&work);
        result
    }

    /// Met en place, sous des noms libres de `dest`, un groupe de fichiers préparés dans
    /// `work` (noms d'origine) ; rend les nouveaux noms.
    fn place_group(
        &mut self,
        work: &str,
        dest: &str,
        names: &[String],
        taken: &mut Taken,
    ) -> OpResult<Vec<String>> {
        let lead = names
            .iter()
            .find(|n| is_audio(n) && split_base(n).1.len() == super::ext_with_dot(n).len())
            .unwrap_or(&names[0]);
        let base = taken.free_base(split_base(lead).0)?;
        let mut placed = Vec::new();
        for name in names {
            let new_name = format!("{base}{}", split_base(name).1);
            self.place(&join(work, name), &join(dest, &new_name))?;
            placed.push(new_name);
        }
        for name in &placed {
            taken.add(name);
        }
        Ok(placed)
    }
}

// ---------------------------------------------------------------------------
// Opérations
// ---------------------------------------------------------------------------

impl Editor {
    /// Copie des fichiers audio, des images ou des dossiers venus de l'Explorateur dans le menu
    /// `menu` (§ 4.2). Les originaux ne sont jamais modifiés ni déplacés ; seuls les formats
    /// acceptés sont copiés ; un nom déjà pris reçoit un suffixe « (2) ». Bornes :
    /// [`super::ImportLimits`] (dépassées → rien n'est copié).
    pub fn import(&mut self, menu: &str, sources: &[PathBuf]) -> OpResult<ImportReport> {
        let menu = parse_id(menu)?;
        let limits = self.import_limits.clone();
        let label = match sources {
            [one] => format!(
                "Copie de « {} »",
                one.file_name().unwrap_or_default().to_string_lossy()
            ),
            _ => format!("Copie de {} éléments", sources.len()),
        };
        let (skipped, done) = self.run(label, |tx, ids| {
            let ed = tx.ed;
            let scan = ed.scan()?;
            require_menu(&scan, &menu)?;
            let mut plan = Plan {
                limits: &limits,
                loose: Vec::new(),
                folders: Vec::new(),
                skipped: Vec::new(),
                files: 0,
                bytes: 0,
                entries: 0,
            };
            for src in sources {
                plan.add_source(ed, src)?;
            }
            let mut taken = Taken::new(&ed.names(&menu)?);
            let work = ed.work_slot()?;
            let result = (|| -> OpResult<()> {
                for group in group_loose(std::mem::take(&mut plan.loose)) {
                    let sub = ed.work_slot()?;
                    let mut names = Vec::new();
                    for file in &group {
                        copy_file(&file.src, &ed.abs(&join(&sub, &file.name)), file.len)?;
                        names.push(file.name.clone());
                    }
                    let placed = tx.place_group(&sub, &menu, &names, &mut taken)?;
                    ed.drop_work_slot(&sub);
                    ids.extend(placed.iter().map(|n| join(&menu, n)));
                }
                for folder in &plan.folders {
                    let staged = join(&work, &folder.name);
                    fs::create_dir(ed.abs(&staged))?;
                    for dir in &folder.dirs {
                        fs::create_dir(ed.abs(&join(&staged, dir)))?;
                    }
                    for (rel, src, len) in &folder.files {
                        copy_file(src, &ed.abs(&join(&staged, rel)), *len)?;
                    }
                    let name = taken.free_name(&folder.name)?;
                    tx.place(&staged, &join(&menu, &name))?;
                    taken.add(&name);
                    ids.push(join(&menu, &name));
                }
                Ok(())
            })();
            ed.drop_work_slot(&work);
            result?;
            Ok(std::mem::take(&mut plan.skipped))
        })?;
        Ok(ImportReport { done, skipped })
    }

    /// Remplace l'image de l'élément `target` (histoire : `<nom>.<ext>` ; menu : `_image.<ext>` ;
    /// racine : `_couverture.<ext>`) par une copie de `source` ; l'ancienne va dans la corbeille.
    pub fn set_image(&mut self, target: &Target, source: &Path) -> OpResult<Done> {
        let target = target.clone();
        let source = source.to_path_buf();
        let label = format!("Image de « {} »", name_of(&target.id));
        let ((), done) = self.run(label, |tx, ids| {
            let ed = tx.ed;
            let id = ed.check_target(&target)?;
            let scan = ed.scan()?;
            let (dir, base) = match find(&scan, &id)? {
                Node::Menu { .. } if id.is_empty() => (String::new(), "_couverture".to_owned()),
                Node::Menu { .. } => (id.clone(), "_image".to_owned()),
                Node::Story { file, .. } => (parent_of(&id).to_owned(), stem_of(file).to_owned()),
            };
            if !source.is_absolute() {
                return Err(source_error(&source, "chemin relatif"));
            }
            let meta = fs::symlink_metadata(&source).map_err(|e| match e.kind() {
                io::ErrorKind::NotFound => source_error(&source, "introuvable"),
                _ => OpError::Io(e),
            })?;
            if is_link_meta(&meta) {
                return Err(source_error(
                    &source,
                    "lien, jonction ou fichier « à la demande »",
                ));
            }
            let src_name = source
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !meta.is_file() || !is_image(src_name) {
                return Err(source_error(&source, "pas une image acceptée"));
            }
            if meta.len() > MAX_IMAGE_BYTES {
                return Err(source_error(&source, "image trop volumineuse"));
            }
            let ext = super::ext_key(src_name);
            let work = ed.work_slot()?;
            let result = (|| -> OpResult<String> {
                let staged = join(&work, &format!("image.{ext}"));
                copy_file(&source, &ed.abs(&staged), meta.len())?;
                let base_key = name_key(&base);
                let old: Vec<String> = ed
                    .names(&dir)?
                    .into_iter()
                    .filter(|n| is_image(n) && name_key(stem_of(n)) == base_key)
                    .collect();
                if !old.is_empty() {
                    let slot = ed.trash_slot()?;
                    for name in old {
                        let rel = join(&dir, &name);
                        let stamp = ed.current_stamp(&rel)?;
                        tx.trash_into(&slot, &rel, stamp)?;
                    }
                }
                let path = join(&dir, &format!("{base}.{ext}"));
                tx.place(&staged, &path)?;
                Ok(path)
            })();
            ed.drop_work_slot(&work);
            ids.push(result?);
            Ok(())
        })?;
        Ok(done)
    }

    /// Range les options du menu `menu` dans l'ordre `order` (noms des entrées) ; les options
    /// non citées suivent dans l'ordre actuel. Écrit `ordre` (§ 3.2, Q3) : aucun fichier n'est
    /// renommé.
    pub fn reorder(&mut self, menu: &str, order: &[String]) -> OpResult<Done> {
        let menu = parse_id(menu)?;
        let order = order.to_vec();
        let label = format!("Ordre de « {} »", name_of(&menu));
        let ((), done) = self.run(label, |tx, _| {
            let scan = tx.ed.scan()?;
            let current = child_names(require_menu(&scan, &menu)?);
            let by_key: HashMap<String, &String> =
                current.iter().map(|n| (name_key(n), n)).collect();
            let mut seen = HashSet::new();
            let mut full = Vec::new();
            for name in &order {
                let key = name_key(name);
                let Some(actual) = by_key.get(&key) else {
                    return Err(OpError::Refused(format!(
                        "« {name} » n'est pas une option de ce menu"
                    )));
                };
                if !seen.insert(key) {
                    return Err(OpError::Refused(format!("« {name} » cité deux fois")));
                }
                full.push((*actual).clone());
            }
            full.extend(
                current
                    .iter()
                    .filter(|n| !seen.contains(&name_key(n)))
                    .cloned(),
            );
            tx.write_settings(&menu, |doc| {
                doc.folder().ordre = full;
                Ok(())
            })
        })?;
        Ok(done)
    }

    /// Déplace l'élément `target` dans le menu `dest`, avant l'option `before` (nom) ou à la
    /// fin. Une histoire emmène son image et son titre lu ; son titre choisi la suit. Les deux
    /// `_dossier.json` sont mis à jour.
    pub fn move_item(
        &mut self,
        target: &Target,
        dest: &str,
        before: Option<&str>,
    ) -> OpResult<Done> {
        let target = target.clone();
        let dest = parse_id(dest)?;
        let before = before.map(str::to_owned);
        let label = format!("Déplacement de « {} »", name_of(&target.id));
        let ((), done) = self.run(label, |tx, ids| {
            let ed = tx.ed;
            let id = ed.check_target(&target)?;
            if id.is_empty() {
                return Err(OpError::Refused("la racine ne se déplace pas".into()));
            }
            let parent = parent_of(&id).to_owned();
            if dest == parent {
                return Err(OpError::Refused(
                    "déjà dans ce menu : changer l'ordre à la place".into(),
                ));
            }
            if dest == id || dest.starts_with(&format!("{id}/")) {
                return Err(OpError::Refused(
                    "un menu ne peut pas aller dans lui-même".into(),
                ));
            }
            let scan = ed.scan()?;
            let node = find(&scan, &id)?;
            let mut dest_order = child_names(require_menu(&scan, &dest)?);
            let position = match &before {
                None => dest_order.len(),
                Some(name) => dest_order
                    .iter()
                    .position(|n| name_key(n) == name_key(name))
                    .ok_or_else(|| {
                        OpError::Refused(format!("« {name} » n'est pas une option de ce menu"))
                    })?,
            };
            let old_name = entry_name(node).to_owned();
            let is_file = matches!(node, Node::Story { .. });
            let mut taken = Taken::new(&ed.names(&dest)?);
            let new_name = if is_file {
                let mut members = vec![old_name.clone()];
                members.extend(ed.companions(&parent, &old_name)?);
                let base = taken.free_base(split_base(&old_name).0)?;
                for member in &members {
                    let from = join(&parent, member);
                    let stamp = ed.current_stamp(&from)?;
                    let to = join(&dest, &format!("{base}{}", split_base(member).1));
                    tx.apply(Action::Move { from, to, stamp })?;
                }
                format!("{base}{}", split_base(&old_name).1)
            } else {
                let name = taken.free_name(&old_name)?;
                let stamp = ed.current_stamp(&id)?;
                tx.apply(Action::Move {
                    from: id.clone(),
                    to: join(&dest, &name),
                    stamp,
                })?;
                name
            };
            taken.add(&new_name);
            let title = ed
                .explicit_title(&parent, &old_name)
                .or_else(|| (new_name != old_name).then(|| title_from_name(&old_name, is_file)));
            dest_order.insert(position, new_name.clone());
            tx.write_settings(&dest, |doc| {
                let folder = doc.folder();
                folder.ordre = dest_order;
                if let Some(title) = title {
                    folder
                        .titres
                        .retain(|k, _| name_key(k) != name_key(&new_name));
                    folder.titres.insert(new_name.clone(), title);
                }
                Ok(())
            })?;
            let old_key = name_key(&old_name);
            tx.write_settings(&parent, |doc| {
                let folder = doc.folder();
                folder.ordre.retain(|n| name_key(n) != old_key);
                folder.titres.retain(|k, _| name_key(k) != old_key);
                Ok(())
            })?;
            ids.push(join(&dest, &new_name));
            Ok(())
        })?;
        Ok(done)
    }

    /// Change le titre affiché de `target` (`titres` du dossier parent ; racine : `titre` de
    /// `_histoire.json`). Titre vide : retour au titre déduit du nom. Avec `rename_file`, le
    /// fichier (et son image, son titre lu) ou le dossier est aussi renommé d'après le titre,
    /// en gardant son préfixe de tri (« 01 »).
    pub fn rename(&mut self, target: &Target, title: &str, rename_file: bool) -> OpResult<Done> {
        let target = target.clone();
        let cleaned = clean_line(title, MAX_TITLE_CHARS);
        let label = format!("Renommage de « {} »", name_of(&target.id));
        let ((), done) = self.run(label, |tx, ids| {
            let ed = tx.ed;
            let id = ed.check_target(&target)?;
            if id.is_empty() {
                if rename_file {
                    return Err(OpError::Refused(
                        "le dossier de l'histoire ne se renomme pas ici".into(),
                    ));
                }
                return tx.write_settings("", |doc| {
                    if let Doc::Story(story) = doc {
                        story.titre = (!cleaned.is_empty()).then(|| cleaned.clone());
                        story.folder.extra.remove("titre");
                    }
                    Ok(())
                });
            }
            let scan = ed.scan()?;
            let node = find(&scan, &id)?;
            let is_file = matches!(node, Node::Story { .. });
            let parent = parent_of(&id).to_owned();
            let old_name = entry_name(node).to_owned();
            let mut new_name = old_name.clone();
            if rename_file {
                if cleaned.is_empty() {
                    return Err(OpError::Refused("titre vide : rien à renommer".into()));
                }
                let old_base = if is_file {
                    split_base(&old_name).0
                } else {
                    old_name.as_str()
                };
                let prefix = &old_base[..old_base.len() - strip_sort_prefix(old_base).len()];
                let wanted = sanitize_name(&format!("{prefix}{cleaned}"))
                    .ok_or_else(|| OpError::Refused("titre inutilisable comme nom".into()))?;
                if name_key(&wanted) != name_key(old_base) {
                    let members = if is_file {
                        let mut members = vec![old_name.clone()];
                        members.extend(ed.companions(&parent, &old_name)?);
                        members
                    } else {
                        vec![old_name.clone()]
                    };
                    let own: HashSet<String> = members.iter().map(|m| name_key(m)).collect();
                    let others: Vec<String> = ed
                        .names(&parent)?
                        .into_iter()
                        .filter(|n| !own.contains(&name_key(n)))
                        .collect();
                    let taken = Taken::new(&others);
                    if is_file {
                        let base = taken.free_base(&wanted)?;
                        for member in &members {
                            let from = join(&parent, member);
                            let stamp = ed.current_stamp(&from)?;
                            let to = join(&parent, &format!("{base}{}", split_base(member).1));
                            tx.apply(Action::Move { from, to, stamp })?;
                        }
                        new_name = format!("{base}{}", split_base(&old_name).1);
                    } else {
                        new_name = taken.free_name(&wanted)?;
                        let stamp = ed.current_stamp(&id)?;
                        tx.apply(Action::Move {
                            from: id.clone(),
                            to: join(&parent, &new_name),
                            stamp,
                        })?;
                    }
                }
            }
            let (old_key, new_key) = (name_key(&old_name), name_key(&new_name));
            let derived = title_from_name(&new_name, is_file);
            tx.write_settings(&parent, |doc| {
                let folder = doc.folder();
                folder
                    .titres
                    .retain(|k, _| name_key(k) != old_key && name_key(k) != new_key);
                for entry in &mut folder.ordre {
                    if name_key(entry) == old_key {
                        entry.clone_from(&new_name);
                    }
                }
                if !cleaned.is_empty() && cleaned != derived {
                    folder.titres.insert(new_name.clone(), cleaned.clone());
                }
                Ok(())
            })?;
            ids.push(join(&parent, &new_name));
            Ok(())
        })?;
        Ok(done)
    }

    /// Crée un sous-menu (dossier) nommé d'après `name` dans le menu `parent`.
    pub fn new_menu(&mut self, parent: &str, name: &str) -> OpResult<Done> {
        let parent = parse_id(parent)?;
        let wanted =
            sanitize_name(name).ok_or_else(|| OpError::Refused("nom de menu vide".into()))?;
        let label = format!("Nouveau menu « {wanted} »");
        let ((), done) = self.run(label, |tx, ids| {
            let ed = tx.ed;
            let scan = ed.scan()?;
            require_menu(&scan, &parent)?;
            let taken = Taken::new(&ed.names(&parent)?);
            let name = taken.free_name(&wanted)?;
            let path = join(&parent, &name);
            tx.make_dir(&path)?;
            ids.push(path);
            Ok(())
        })?;
        Ok(done)
    }

    /// Met l'élément `target` dans la corbeille de l'histoire : une histoire avec son image et
    /// son titre lu, un menu avec tout son contenu, ou seulement une image ou un titre lu. Les
    /// réglages ne sont pas modifiés (titre et rang reviennent avec « Annuler »).
    pub fn delete(&mut self, target: &Target) -> OpResult<Done> {
        let target = target.clone();
        let label = format!("Suppression de « {} »", name_of(&target.id));
        let ((), done) = self.run(label, |tx, _| {
            let ed = tx.ed;
            let id = ed.check_target(&target)?;
            if id.is_empty() {
                return Err(OpError::Refused(
                    "le dossier de l'histoire ne se supprime pas ici".into(),
                ));
            }
            let scan = ed.scan()?;
            let parent = parent_of(&id).to_owned();
            let members = match scan.root.find(&id) {
                Some(Node::Story { file, .. }) => {
                    let mut members = vec![file.clone()];
                    members.extend(ed.companions(&parent, file)?);
                    members
                }
                Some(Node::Menu { dir, .. }) => vec![dir.clone()],
                None if is_special_of(&scan.root, &id) => vec![name_of(&id).to_owned()],
                None => return Err(OpError::NotFound(id)),
            };
            let slot = ed.trash_slot()?;
            for member in members {
                let rel = join(&parent, &member);
                let stamp = ed.current_stamp(&rel)?;
                tx.trash_into(&slot, &rel, stamp)?;
            }
            Ok(())
        })?;
        Ok(done)
    }

    /// Modifie `_histoire.json` (titre, description, âge, mode, début, fin…).
    pub fn set_story_settings(&mut self, patch: &StoryPatch) -> OpResult<Done> {
        let patch = patch.clone();
        if let Some(Some(age)) = patch.age {
            if u64::from(age) > MAX_AGE {
                return Err(OpError::Refused(format!("âge invalide : {age}")));
            }
        }
        let ((), done) = self.run("Réglages de l'histoire".into(), |tx, _| {
            tx.write_settings("", |doc| {
                let Doc::Story(story) = doc else {
                    return Ok(());
                };
                apply_folder_patch(&mut story.folder, &patch.folder);
                let extra = &mut story.folder.extra;
                set_text(
                    &mut story.titre,
                    extra,
                    "titre",
                    &patch.titre,
                    MAX_TITLE_CHARS,
                );
                set_text(
                    &mut story.sous_titre,
                    extra,
                    "sousTitre",
                    &patch.sous_titre,
                    MAX_TEXT_CHARS,
                );
                set_text(
                    &mut story.description,
                    extra,
                    "description",
                    &patch.description,
                    MAX_TEXT_CHARS,
                );
                set_value(&mut story.age, extra, "age", &patch.age);
                set_value(&mut story.debut, extra, "debut", &patch.debut);
                set_value(&mut story.titres_lus, extra, "titresLus", &patch.titres_lus);
                Ok(())
            })
        })?;
        Ok(done)
    }

    /// Modifie les réglages du menu `menu` (`_dossier.json` ; racine : `_histoire.json`).
    pub fn set_folder_settings(&mut self, menu: &str, patch: &FolderPatch) -> OpResult<Done> {
        let menu = parse_id(menu)?;
        let patch = patch.clone();
        let label = format!("Réglages de « {} »", name_of(&menu));
        let ((), done) = self.run(label, |tx, _| {
            let scan = tx.ed.scan()?;
            require_menu(&scan, &menu)?;
            tx.write_settings(&menu, |doc| {
                apply_folder_patch(doc.folder(), &patch);
                Ok(())
            })
        })?;
        Ok(done)
    }
}

/// `id` est l'image ou le titre lu d'un élément de l'arbre.
fn is_special_of(node: &Node, id: &str) -> bool {
    let own = match node {
        Node::Menu {
            image, title_audio, ..
        }
        | Node::Story {
            image, title_audio, ..
        } => image.as_deref() == Some(id) || title_audio.as_deref() == Some(id),
    };
    own || node.children().iter().any(|c| is_special_of(c, id))
}

/// Applique un champ « absent / effacé / valeur » ; effacer retire aussi la valeur brute
/// invalide gardée dans `extra`.
fn set_value<T: Clone>(
    field: &mut Option<T>,
    extra: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    change: &Option<Option<T>>,
) {
    if let Some(value) = change {
        field.clone_from(value);
        if value.is_none() {
            extra.remove(key);
        }
    }
}

fn set_text(
    field: &mut Option<String>,
    extra: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    change: &Option<Option<String>>,
    max: usize,
) {
    let cleaned = change.as_ref().map(|value| {
        value
            .as_deref()
            .map(|text| clean_line(text, max))
            .filter(|text| !text.is_empty())
    });
    set_value(field, extra, key, &cleaned);
}

fn apply_folder_patch(folder: &mut FolderSettings, patch: &FolderPatch) {
    let extra = &mut folder.extra;
    set_value(&mut folder.mode, extra, "mode", &patch.mode);
    set_value(&mut folder.fin, extra, "fin", &patch.fin);
    set_value(&mut folder.annoncer, extra, "annoncer", &patch.annoncer);
}
