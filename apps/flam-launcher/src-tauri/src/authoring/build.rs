//! Création d'une histoire à partir d'un dossier lu par [`tree::scan`]
//! (`docs/creation-dossier.md` § 5, § 7, § 8, § 9, lot 5).
//!
//! - [`plan`] : l'arbre lu devient la commande de l'étape commune de fabrication
//!   (`install::pack_build`, lot 4) : éléments par rang, arbre de menus avec le **mode** de
//!   chaque dossier (recueil ou chapitres, lot 3), images, titres lus fournis, couverture,
//!   bornes propres aux dossiers ([`archive_limits`]).
//! - [`prepare`] : titres lus (fichiers fournis convertis, sinon voix Windows, sinon muets),
//!   MP3 conformes, images, archive Telmi, dans le dossier de travail d'une tâche de la file
//!   d'installation (progression et annulation de l'étape commune).
//! - La tâche de la file (`install::folder`) convertit ensuite l'archive (`telmi2flam`) et la
//!   publie dans « Mes histoires » ([`story_entry`] : la source retient le dossier).
//!
//! # Compléments (§ 7)
//!
//! - Titre lu : `_titre.*` d'un dossier (`_titre.*` de la racine : titre de l'histoire),
//!   `<nom>.titre.*` d'une histoire, convertis comme l'audio ; sinon voix Windows (voix de
//!   `titresLus.voix`, sinon celle du poste) ; sinon muet, avec un avertissement (aucune voix,
//!   synthèse en échec, fichier illisible).
//! - Image d'une histoire : image du même nom ; dans un livre à chapitres, sinon celle de la
//!   partie (ou d'un dossier parent), sinon la couverture ; sinon [`complement_image`] (point
//!   d'extension : pochette extraite et image avec titre, lot 10), sinon image unie.
//! - Image d'un menu : `_image.*` (image propre du sous-menu), sinon celle de sa première
//!   histoire (règle de l'archive, N5 a).
//! - Couverture : `_couverture.*`, sinon la première image d'histoire, sinon image unie.
//!
//! # Invariants
//!
//! - **Rien n'est écrit dans le dossier source** ici : seul `_histoire.json` l'est, par
//!   `authoring::identity::commit`, avant la mise en file. Les fichiers produits sont nommés par
//!   rang dans le dossier de travail (`ep-NNNN.*`, `title-NNNN.mp3`, `titlesrc-NNNN.mp3`,
//!   `menu-NNNN.png`) ; aucun nom de l'utilisateur n'y est réutilisé.
//! - **Chemins** : seulement des ids du scan (noms lus sur le disque sous la racine résolue),
//!   jamais un chemin venu du front.
//! - **Bornes** : celles du scan ([`scan_limits`], bloquantes dans le rapport) et celles de
//!   l'archive ([`archive_limits`]) vont ensemble ; la borne de scènes s'en déduit et ne peut
//!   pas être atteinte seule.
//! - Un recueil dans un livre à chapitres n'est pas pris en charge par l'archive : problème
//!   bloquant `collection_in_book` du rapport ([`book_problems`]).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::settings::{Debut, Fin, Mode};
use super::tree::{self, Node, Problem, Scan, Severity};
use crate::install::pack_build::{
    converter_unavailable_for, is_fatal_conversion, BuildBackend, BuildSource, ItemAudio,
    LocalItem, MenuNode, PackBuild, PackSpec,
};
use crate::install::pack_speech::{SpokenTitles, TitleKey};
use crate::podcast::archive::{
    self, ArchiveOptions, BookEnd, ChapterSettings, ChapterStart, CollectionEnd, Limits, MenuMode,
};
use crate::podcast::pack::ResolvedMetadata;
use crate::story::{StoryEntry, StoryKind};
use crate::{AppError, AppResult};

/// `source_id` d'une histoire faite d'un dossier dans « Mes histoires ».
pub const SOURCE_ID: &str = "authoring-folder";
/// Niveaux de menus au plus, racine comprise (§ 9, à confirmer sur la Flam, § 13).
pub const MAX_MENU_DEPTH: usize = 5;
/// Histoires ou chapitres au plus (§ 9).
pub const MAX_STORIES: usize = 300;
/// Options par menu (§ 9).
pub const MAX_OPTIONS_PER_MENU: usize = 50;
/// Audio au plus : 3 Gio, nettement sous la limite de l'archive zip32 (4 Gio) avec les images.
/// Vaut pour les fichiers source (rapport) et pour les MP3 produits, titres lus compris.
pub const MAX_AUDIO_BYTES: u64 = 3 * 1024 * 1024 * 1024;
/// Scènes au plus, déduites des autres bornes : scène d'entrée, une scène par histoire, une
/// annonce par chapitre, une option par histoire, une option par sous-menu (chaque sous-menu
/// gardé contient au moins une histoire, au plus `MAX_MENU_DEPTH - 1` sous-menus par histoire).
pub const MAX_STAGES: usize = 1 + 3 * MAX_STORIES + MAX_STORIES * (MAX_MENU_DEPTH - 1);

/// Bornes du scan d'un dossier d'histoire (§ 9) : dépassées, problèmes bloquants du rapport.
pub fn scan_limits() -> tree::Limits {
    tree::Limits {
        max_menu_depth: MAX_MENU_DEPTH,
        max_stories: MAX_STORIES,
        max_options_per_menu: MAX_OPTIONS_PER_MENU,
        max_audio_bytes: MAX_AUDIO_BYTES,
        ..tree::Limits::default()
    }
}

/// Bornes de l'archive d'un dossier d'histoire (mêmes valeurs que [`scan_limits`]) ; taille
/// d'un MP3 : celle des podcasts.
pub fn archive_limits() -> Limits {
    Limits {
        max_menu_depth: MAX_MENU_DEPTH,
        max_options_per_menu: MAX_OPTIONS_PER_MENU,
        max_episodes: MAX_STORIES,
        max_stages: MAX_STAGES,
        max_audio_bytes: archive::MAX_AUDIO_BYTES,
        max_total_audio_bytes: MAX_AUDIO_BYTES,
    }
}

/// Mode d'archive d'un dossier : mode, fin et annonce effectifs, `debut` de l'histoire.
pub fn menu_mode(mode: Mode, fin: Fin, annoncer: bool, debut: Debut) -> MenuMode {
    match mode {
        Mode::Recueil => MenuMode::Collection {
            end: if fin == Fin::Suivante {
                CollectionEnd::Next
            } else {
                CollectionEnd::Menu
            },
        },
        Mode::Chapitres => MenuMode::Chapters(ChapterSettings {
            start: if debut == Debut::Menu {
                ChapterStart::Menu
            } else {
                ChapterStart::Chapter
            },
            end: if fin == Fin::Fin {
                BookEnd::End
            } else {
                BookEnd::Menu
            },
            announce: annoncer,
        }),
    }
}

/// Chemin absolu de l'élément d'id `id` (chemin relatif en `/`, noms lus sur le disque) sous
/// la racine résolue `base`.
pub fn path_of(base: &Path, id: &str) -> PathBuf {
    id.split('/')
        .filter(|part| !part.is_empty())
        .fold(base.to_path_buf(), |path, part| path.join(part))
}

/// Image d'une histoire qui n'en a pas (point d'extension du lot 10 : pochette du fichier
/// audio, image générée avec le titre). Aujourd'hui : aucune, l'archive met l'image unie.
pub fn complement_image(_audio: &Path, _title: &str) -> Option<PathBuf> {
    None
}

/// Problèmes bloquants propres à la création, en plus de ceux du scan : recueil placé dans un
/// livre à chapitres (`collection_in_book`).
pub fn book_problems(scan: &Scan) -> Vec<Problem> {
    fn walk(node: &Node, in_book: bool, out: &mut Vec<Problem>) {
        let Node::Menu {
            id,
            dir,
            mode,
            children,
            ..
        } = node
        else {
            return;
        };
        if in_book && *mode == Mode::Recueil {
            out.push(Problem {
                code: "collection_in_book",
                severity: Severity::Blocking,
                path: id.clone(),
                message: format!(
                    "« {dir} » est un recueil placé dans un livre à chapitres : non pris en \
                     charge (mettez ce dossier en mode chapitres ou sortez-le du livre)"
                ),
            });
        }
        for child in children {
            walk(child, in_book || *mode == Mode::Chapitres, out);
        }
    }
    let mut out = Vec::new();
    walk(&scan.root, false, &mut out);
    out
}

/// Métadonnées de l'histoire : titre (réglage `titre`, sinon nom du dossier), sous-titre, âge,
/// description de `_histoire.json`, bornés pour l'archive.
pub fn metadata(scan: &Scan) -> ResolvedMetadata {
    let story = &scan.story.settings;
    let bound = |s: &str| s.chars().take(archive::MAX_TITLE_CHARS).collect::<String>();
    let title = bound(scan.root.title().trim());
    ResolvedMetadata {
        title: if title.is_empty() {
            "Histoire".into()
        } else {
            title
        },
        subtitle: bound(story.sous_titre.as_deref().unwrap_or_default()),
        age: story.age.unwrap_or(0),
        description: story.description.clone(),
    }
}

/// Commande de fabrication d'une histoire.
#[derive(Debug, Clone)]
pub struct Plan {
    pub spec: PackSpec,
    /// Couverture source (`_couverture.*`, sinon première image d'histoire).
    pub cover: Option<PathBuf>,
    /// Titres lus fournis par le dossier (fichiers audio à convertir), par emplacement.
    pub titles: Vec<(TitleKey, PathBuf)>,
}

struct Planner<'a> {
    base: &'a Path,
    debut: Debut,
    items: Vec<LocalItem>,
    titles: Vec<(TitleKey, PathBuf)>,
}

impl Planner<'_> {
    /// Options du dossier `children` (mode `mode`) ; `fallback` : image des chapitres sans
    /// image (partie, dossier parent ou couverture).
    fn menu(
        &mut self,
        children: &[Node],
        mode: Mode,
        fallback: Option<&PathBuf>,
        path: &mut Vec<usize>,
    ) -> Vec<MenuNode> {
        let mut out = Vec::with_capacity(children.len());
        for (i, child) in children.iter().enumerate() {
            path.push(i);
            match child {
                Node::Story {
                    id,
                    title,
                    image,
                    title_audio,
                    ..
                } => {
                    let audio = path_of(self.base, id);
                    let image = match image {
                        Some(image) => Some(path_of(self.base, image)),
                        None if mode == Mode::Chapitres => fallback.cloned(),
                        None => complement_image(&audio, title),
                    };
                    if let Some(t) = title_audio {
                        self.titles
                            .push((TitleKey::Node(path.clone()), path_of(self.base, t)));
                    }
                    out.push(MenuNode::Item(self.items.len()));
                    self.items.push(LocalItem {
                        title: title.clone(),
                        audio: ItemAudio::Source(audio),
                        image,
                        title_audio: None,
                        cache_key: None,
                    });
                }
                Node::Menu {
                    title,
                    mode: sub_mode,
                    fin,
                    annoncer,
                    image,
                    title_audio,
                    children,
                    ..
                } => {
                    let picture = image.as_ref().map(|id| path_of(self.base, id));
                    if let Some(t) = title_audio {
                        self.titles
                            .push((TitleKey::Node(path.clone()), path_of(self.base, t)));
                    }
                    let inner = picture.as_ref().or(fallback);
                    let entries = self.menu(children, *sub_mode, inner, path);
                    out.push(MenuNode::Category {
                        name: title.clone(),
                        entries,
                        image: None,
                        mode: Some(menu_mode(*sub_mode, *fin, *annoncer, self.debut)),
                        picture,
                    });
                }
            }
            path.pop();
        }
        out
    }
}

/// Commande de fabrication du dossier lu `scan`, sous l'uuid `uuid`. Le scan doit être sans
/// problème bloquant (ni [`book_problems`]) : sinon l'archive refusera la commande.
pub fn plan(scan: &Scan, uuid: &str) -> Plan {
    let base = scan.base.as_path();
    let debut = scan.story.settings.debut.unwrap_or(Debut::Chapitre);
    let (root_mode, cover_src, root_title, children) = match &scan.root {
        Node::Menu {
            mode,
            fin,
            annoncer,
            image,
            title_audio,
            children,
            ..
        } => (
            (*mode, menu_mode(*mode, *fin, *annoncer, debut)),
            image.as_ref().map(|id| path_of(base, id)),
            title_audio.as_ref().map(|id| path_of(base, id)),
            children.as_slice(),
        ),
        Node::Story { .. } => ((Mode::Recueil, MenuMode::default()), None, None, &[][..]),
    };
    let mut planner = Planner {
        base,
        debut,
        items: Vec::new(),
        titles: Vec::new(),
    };
    if let Some(t) = root_title {
        planner.titles.push((TitleKey::Pack, t));
    }
    let menu = planner.menu(children, root_mode.0, cover_src.as_ref(), &mut Vec::new());
    let cover = cover_src.or_else(|| planner.items.iter().find_map(|i| i.image.clone()));
    Plan {
        spec: PackSpec {
            uuid: uuid.to_string(),
            metadata: metadata(scan),
            items: planner.items,
            menu,
            options: ArchiveOptions {
                mode: root_mode.1,
                limits: archive_limits(),
            },
        },
        cover,
        titles: planner.titles,
    }
}

/// Entrée de « Mes histoires » d'une histoire faite du dossier `folder` : la source retient le
/// dossier (`download_url`, chemin résolu) ; `entry_id` stable par uuid.
pub fn story_entry(uuid: &str, version: u32, meta: &ResolvedMetadata, folder: &Path) -> StoryEntry {
    StoryEntry {
        source_id: SOURCE_ID.into(),
        entry_id: format!("folder:{uuid}"),
        uuid: uuid.to_string(),
        version: Some(version),
        title: meta.title.clone(),
        description: meta.description.clone(),
        age: Some(u32::from(meta.age)),
        category: Some(meta.subtitle.clone()).filter(|s| !s.is_empty()),
        author: None,
        voice: None,
        license: None,
        thumb_url: None,
        download_url: folder.to_string_lossy().into_owned(),
        size: None,
        updated_at: None,
        download_count: None,
        kind: StoryKind::TelmiZip,
    }
}

/// Voix demandée par `_histoire.json` (`titresLus.voix`), nettoyée ; invalide : aucune.
pub fn requested_voice(scan: &Scan) -> Option<String> {
    let voice = scan
        .story
        .settings
        .titres_lus
        .as_ref()?
        .get("voix")?
        .as_str()?
        .to_string();
    crate::settings::clean_voice_id(Some(voice)).ok().flatten()
}

/// Avertissement d'un titre lu fourni mais illisible.
fn unreadable_title(src: &Path) -> String {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    format!("Titre lu « {name} » illisible : remplacé par la voix de Windows ou muet.")
}

/// Titres lus fournis par le dossier, convertis en MP3 conformes (`titlesrc-NNNN.mp3`). Un
/// fichier illisible est signalé et laissé à la synthèse.
async fn convert_titles<B: BuildBackend + ?Sized + 'static>(
    stage: &PackBuild<'_, B>,
    titles: &[(TitleKey, PathBuf)],
    limits: &Limits,
    warnings: &mut Vec<String>,
) -> AppResult<SpokenTitles> {
    let mut provided = SpokenTitles::default();
    if titles.is_empty() {
        return Ok(provided);
    }
    let jobs: Vec<(PathBuf, PathBuf)> = titles
        .iter()
        .enumerate()
        .map(|(n, (_, src))| {
            (
                src.clone(),
                stage.workdir.join(format!("titlesrc-{n:04}.mp3")),
            )
        })
        .collect();
    let results = stage
        .backend
        .convert_audio(jobs.clone(), stage.cancel)
        .await?;
    for (((key, src), (_, mp3)), result) in titles.iter().zip(jobs).zip(results) {
        match result {
            Ok(bytes) if bytes <= limits.max_audio_bytes => {
                provided.audio.insert(key.clone(), mp3);
            }
            Ok(_) => warnings.push(unreadable_title(src)),
            Err(AppError::Cancelled) => return Err(AppError::Cancelled),
            Err(err) if is_fatal_conversion(&err) => {
                return Err(converter_unavailable_for(BuildSource::Folder, err))
            }
            Err(err) => {
                tracing::warn!("titre lu fourni ignoré : {}", err.kind());
                warnings.push(unreadable_title(src));
            }
        }
    }
    Ok(provided)
}

/// Fabrique l'archive Telmi de `plan` dans le dossier de travail de `stage` : titres lus (0 à
/// 10 %), audio (60 %), images (85 %), archive (90 %). `voice` : voix des titres synthétisés
/// (`None` : voix française par défaut). Rend le chemin de l'archive.
pub(crate) async fn prepare<B: BuildBackend + ?Sized + 'static>(
    stage: &PackBuild<'_, B>,
    plan: Plan,
    voice: Option<&str>,
) -> AppResult<PathBuf> {
    let Plan {
        mut spec,
        cover,
        titles,
    } = plan;
    let limits = spec.options.limits;
    stage
        .service
        .set_pack_status(stage.id, spec.items.len(), Vec::new());
    stage.service.emit_phase(stage.id, 0.0, "speech");
    let mut warnings = Vec::new();
    let provided = convert_titles(stage, &titles, &limits, &mut warnings).await?;
    let skip: HashSet<TitleKey> = provided.audio.keys().cloned().collect();
    // Titres sans fichier : voix de Windows ; aucune voix ou synthèse en échec : muets, avec un
    // avertissement (l'histoire est créée quand même).
    let mut spoken = match stage
        .speak_titles(&spec.menu, &spec.items, &spec.metadata.title, voice, &skip)
        .await
    {
        Ok((spoken, fallback)) => {
            warnings.extend(fallback);
            spoken
        }
        Err(AppError::Cancelled) => return Err(AppError::Cancelled),
        Err(
            err @ (AppError::TtsNoVoice(_) | AppError::TtsUnavailable(_) | AppError::TtsFailed(_)),
        ) => {
            tracing::warn!(
                "titres lus de l'histoire : synthèse abandonnée ({})",
                err.kind()
            );
            warnings.push(format!(
                "Titres non lus à voix haute : {err} Les options sans titre lu fourni restent \
                 muettes."
            ));
            SpokenTitles::default()
        }
        Err(err) => return Err(err),
    };
    spoken.audio.extend(provided.audio);
    if *stage.cancel.borrow() {
        return Err(AppError::Cancelled);
    }
    let converted = stage
        .convert_audio(&spec.items, &limits, None, Vec::new(), now_secs())
        .await?;
    stage.begin_images();
    stage.pictures(&mut spec.menu).await?;
    let images = stage.images(&converted, cover).await?;
    stage
        .write_archive(&spec, converted, images, &spoken, warnings)
        .await
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;
