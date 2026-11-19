//! Archive Telmi synthétique d'un pack de podcasts (#216, spécification
//! `docs/podcasts.md` § 6), et d'une histoire faite d'un dossier (`docs/creation-dossier.md`
//! § 5 : modes recueil et chapitres).
//!
//! Un pack est une **nouvelle histoire** (P3) : uuid aléatoire tiré une fois par l'appelant
//! ([`new_pack_uuid`]), version fixe 1. L'arborescence de menus (N2 b) devient un graphe Telmi
//! que `telmi2flam` convertit sans modification.
//!
//! Chaque menu a un **mode** ([`MenuMode`]) : celui du menu racine est donné par
//! [`ArchiveOptions`], une [`MenuEntry::Category`] hérite du mode de son menu, une
//! [`MenuEntry::Submenu`] porte le sien (Q1). [`write_archive`] (podcasts) = recueil
//! `fin = suivante` à la racine et bornes [`Limits::PODCAST`] : graphe identique, octet pour
//! octet, à celui d'avant les modes (test `golden`).
//!
//! # Recueil (podcasts : toujours ce mode)
//!
//! - `startAction` → scène d'entrée (passage sans audio) → menu racine ;
//! - un menu est une action à plusieurs entrées (carrousel) ; chaque option est une scène
//!   image (audio : titre lu ou rien, voir plus bas) dont `ok` mène à l'action « jouer
//!   l'épisode » (action à une entrée) ou à l'action du sous-menu ;
//! - un épisode est une scène `autoplay`. À sa fin, selon [`CollectionEnd`] : `suivante`
//!   (podcasts) → l'épisode **suivant du même menu** (les sous-menus sont sautés), la fin du
//!   dernier retourne à **ce menu**, focalisé sur l'épisode joué (P8, N4 a) ; `menu` → ce menu,
//!   focalisé sur l'épisode suivant (sur lui-même pour le dernier) ;
//! - retour (`home`) : d'un épisode vers son menu, d'une option de sous-menu vers le menu
//!   parent (focalisé sur la catégorie) ; le menu racine garde le retour par défaut du moteur ;
//! - libellés (`notes.json`) : titres translittérés pour les polices du player
//!   (`telmi2flam::text`), 40 caractères au plus (A6) ;
//! - images : celle de l'épisode, sinon l'image unie de repli (D13) ; image d'une catégorie =
//!   image de son premier épisode (N5 a) ; couverture choisie par l'appelant (N6 b), sinon
//!   repli ;
//! - titres lus (synthèse vocale, § 12, #462) : l'audio d'une scène d'option est le titre
//!   prononcé de l'épisode (`audios/tNNN.mp3`, NNN = rang de l'épisode) ou de la catégorie
//!   (`audios/cNNN.mp3`, NNN = rang de la catégorie), joué quand l'option prend le focus ;
//!   sans titre lu, `"audio": null` (`telmi2flam` met alors `silent.mp3`). Le titre lu du pack
//!   est `title.mp3`, à la racine de l'archive. Pas d'annonce avant l'épisode.
//!
//! # Livre à chapitres (§ 5.3, § 5.4)
//!
//! Les chapitres d'un livre sont ses épisodes, **dans l'ordre du parcours en profondeur** ; ses
//! sous-menus sont des **parties** qui ne servent qu'au menu des chapitres (un niveau par
//! partie). Mêmes noms de fichiers que le recueil (`eNNN`, `tNNN`, `cNNN`).
//!
//! - chapitre = scène `autoplay` `eN` dont `ok` mène au chapitre suivant, parties
//!   traversées (action `pN` du suivant) ; `home` → son option dans le menu de sa partie,
//!   focalisée ; valider une option → action `pN` du chapitre, puis la suite ;
//! - fin du dernier chapitre ([`BookEnd`]) : `menu` → premier niveau du menu des chapitres,
//!   sur sa première option ; `fin` → `ok: null`, fin d'histoire ;
//! - démarrage ([`ChapterStart`]) : `chapitre` → action `pN` du premier chapitre, `menu` →
//!   menu des chapitres ; c'est la cible de la scène d'entrée (livre racine) ou de l'option
//!   du recueil qui contient le livre ;
//! - annonce (`annoncer`, réglage du livre ou de la partie) : si le chapitre a un titre lu,
//!   `pN` → scène `nN` (titre lu `tNNN.mp3` sur l'image du chapitre, `autoplay`, même `home`)
//!   → action `qN` → `eN`. Sans titre lu, pas d'annonce ;
//! - retour depuis une option d'une partie → niveau du dessus, focalisé sur la partie ;
//!   depuis le premier niveau : livre placé dans un recueil → le recueil, focalisé sur le
//!   livre ; **livre racine → retour par défaut du moteur** (menu Start, sortie de
//!   l'histoire), comme depuis le menu racine d'un recueil, y compris avec `debut = menu` ;
//! - un recueil dans un livre est refusé ([`ArchiveError::CollectionInBook`]) ; un
//!   [`MenuEntry::Submenu`] en mode chapitres dans un livre est une partie (seul son
//!   `annoncer` compte).
//!
//! Module pur (ni réseau, ni Tauri) : les MP3 (#214) et PNG (#215) sont déjà sur le disque.
//!
//! # Invariants
//!
//! - **Écriture atomique** : temporaire voisin unique, `sync_all`, renommage ; tout échec ou
//!   annulation supprime le temporaire, la destination n'est jamais partielle.
//! - **Noms d'entrées sûrs** : seulement des noms générés ici, jamais tirés d'un titre ni d'un
//!   chemin reçu.
//! - **Bornes** vérifiées avant toute écriture : celles de [`ArchiveOptions::limits`]
//!   ([`Limits::PODCAST`] = constantes `MAX_*` pour [`write_archive`]).
//! - **Graphe** : chaque transition mène à une action existante ; toute scène de lecture
//!   (épisode, chapitre, annonce) a un `home` vers son option ; seules les options du menu
//!   racine (recueil ou livre racine) gardent le retour par défaut du moteur ; un chapitre n'a
//!   `ok: null` que s'il est le dernier d'un livre `fin = fin` ; pas d'inventaire.
//! - L'audio est copié par blocs, jamais chargé entier en mémoire.

use std::collections::HashSet;
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Map, Value};
use telmi2flam::text::{text as font_text, LABEL_MAX};
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::artwork::{Artwork, ArtworkError, ArtworkKind, FALLBACK_COLOR};

/// Niveaux de menus au plus, racine comprise (N2 b).
pub const MAX_MENU_DEPTH: usize = 3;
/// Options (épisodes et catégories) par menu (N2 b).
pub const MAX_OPTIONS_PER_MENU: usize = 50;
/// Épisodes par pack (D1).
pub const MAX_EPISODES: usize = 50;
/// Scènes du graphe au plus. Avec les bornes ci-dessus : 1 scène d'entrée, au plus
/// 50 épisodes et 150 options (chaque catégorie contient au moins un épisode, donc au plus
/// 2 catégories par épisode sur 3 niveaux), soit 201 ; un livre à chapitres annoncés ajoute
/// au plus une scène d'annonce par chapitre, soit 251. Garde-fou pour le moteur Lua.
pub const MAX_STAGES: usize = 256;
/// Taille d'un MP3 : D1 (200 Mo par épisode) × 1,25, marge du réencodage D6 (débit arrondi au
/// multiple de 16 supérieur : au plus + 23 % pour une source d'au moins 64 kbps). Une source
/// plus faible (32 à 48 kbps, remontée à 64) peut grossir davantage : garde-fou technique
/// seulement, la taille des MP3 convertis reste à contrôler par l'appelant (#217).
pub const MAX_AUDIO_BYTES: u64 = 250_000_000;
/// Taille cumulée des MP3 : D1 (1 Gio) × 1,25. Reste loin de la limite zip32 (4 Gio).
pub const MAX_TOTAL_AUDIO_BYTES: u64 = 1_342_177_280;
/// Taille d'un PNG (comme les entrées de `artwork`).
pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// Titre et sous-titre, en caractères.
pub const MAX_TITLE_CHARS: usize = 256;
/// Description, en octets UTF-8. La note de conception annonçait 16 Kio ; 64 Kio sont retenus :
/// la description par défaut (au plus 100 lignes de [`DESCRIPTION_LINE_CHARS`] caractères, avec
/// les échecs) peut dépasser 16 Kio avec des titres longs ou non latins, et l'écrêter
/// perdrait des épisodes de la liste. Le plafond ne protège que d'une saisie démesurée.
pub const MAX_DESCRIPTION_BYTES: usize = 64 * 1024;

/// Taille des blocs de copie de l'audio (annulation vérifiée entre deux blocs).
const COPY_CHUNK: usize = 1024 * 1024;
/// Signature PNG.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// Couverture dans l'archive (référencée par `metadata.image`).
const COVER_NAME: &str = "cover.png";
/// Action et scène d'entrée, menu racine.
const START_ACTION: &str = "begin";
const START_STAGE: &str = "start";
const ROOT_MENU: &str = "m0";

/// Métadonnées saisies (D16, N1 b) : valeurs déjà choisies par l'appelant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackMetadata {
    /// Titre, obligatoire (non vide).
    pub title: String,
    /// Sous-titre, écrit dans `category` (affiché par `telmi2flam`), peut être vide.
    pub subtitle: String,
    pub age: u8,
    /// Voir [`default_description`] et [`with_failures`].
    pub description: String,
}

/// Épisode prêt : MP3 conforme (#214) et PNG 640 × 480 (#215) sur le disque.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackEpisode {
    pub title: String,
    pub audio: PathBuf,
    /// Absente : image unie de repli (D13).
    pub image: Option<PathBuf>,
    /// MP3 du titre lu (§ 12) ; absent : option muette (`"audio": null`).
    pub title_audio: Option<PathBuf>,
}

/// Option d'un menu : un épisode ou une catégorie (sous-menu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Episode(PackEpisode),
    /// Sous-menu qui **hérite** du mode du menu qui le contient (podcasts : toujours ce cas).
    Category {
        name: String,
        entries: Vec<MenuEntry>,
        /// MP3 du nom lu (§ 12) ; absent : option muette (`"audio": null`).
        audio: Option<PathBuf>,
        /// Image choisie (N5 c) : le PNG (même chemin que `PackEpisode::image`) d'un épisode de
        /// la catégorie. Absente, ou qui ne désigne aucun épisode de la catégorie : image du
        /// premier épisode (N5 a).
        image: Option<PathBuf>,
    },
    /// Sous-menu avec **son propre mode** (`creation-dossier.md` § 5.1, Q1) : mêmes champs que
    /// [`MenuEntry::Category`], plus le mode et ses réglages, hérités par ses propres
    /// catégories. Dans un recueil : un recueil (réglage `fin` différent) ou un livre à
    /// chapitres. Dans un livre : une partie (seul `annoncer` compte, `debut` et `fin` sont ceux
    /// du livre) ; un recueil dans un livre est refusé ([`ArchiveError::CollectionInBook`]).
    Submenu {
        name: String,
        entries: Vec<MenuEntry>,
        audio: Option<PathBuf>,
        image: Option<PathBuf>,
        mode: MenuMode,
    },
}

/// Fin d'une histoire d'un recueil (§ 5.2, réglage `fin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionEnd {
    /// `menu` : retour au menu, focalisé sur l'histoire suivante du même menu (les sous-menus
    /// sont sautés) ; la dernière y revient focalisée sur elle-même.
    Menu,
    /// `suivante` : l'histoire suivante du même menu démarre (les sous-menus sont sautés) ; la
    /// dernière ramène au menu, focalisé sur elle. Comportement des podcasts (P8, N4 a).
    Next,
}

/// Démarrage d'un livre à chapitres (§ 5.3, réglage `debut`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChapterStart {
    /// `chapitre` : le premier chapitre démarre directement.
    #[default]
    Chapter,
    /// `menu` : le menu des chapitres s'ouvre, sur le premier.
    Menu,
}

/// Fin d'un livre à chapitres (§ 5.3, réglage `fin`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BookEnd {
    /// `menu` : menu des chapitres du livre, focalisé sur sa première option.
    #[default]
    Menu,
    /// `fin` : fin d'histoire (`ok: null`), retour au menu Start du moteur puis à la
    /// bibliothèque. Vaut aussi pour un livre placé dans un recueil : toute l'histoire se
    /// termine.
    End,
}

/// Réglages d'un livre à chapitres (§ 5.3, § 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ChapterSettings {
    pub start: ChapterStart,
    pub end: BookEnd,
    /// `annoncer` : le titre lu du chapitre (`PackEpisode::title_audio`) est joué avant lui,
    /// sur son image. Sans titre lu, pas d'annonce.
    pub announce: bool,
}

/// Mode d'un menu (§ 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuMode {
    /// Recueil : une option = une histoire ou un sous-menu (§ 5.2).
    Collection { end: CollectionEnd },
    /// Livre à chapitres : lecture enchaînée en profondeur, menu des chapitres (§ 5.3).
    Chapters(ChapterSettings),
}

impl MenuMode {
    /// Mode des packs de podcasts : recueil, `fin = suivante`.
    pub const PODCAST: MenuMode = MenuMode::Collection {
        end: CollectionEnd::Next,
    };
}

impl Default for MenuMode {
    /// [`MenuMode::PODCAST`] : le comportement historique des packs de podcasts.
    fn default() -> Self {
        Self::PODCAST
    }
}

/// Bornes vérifiées par [`write_archive_with`] avant toute écriture. Les podcasts utilisent
/// [`Limits::PODCAST`] (constantes `MAX_*`) ; une autre source fixe les siennes
/// (`creation-dossier.md` § 9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Niveaux de menus au plus, racine comprise.
    pub max_menu_depth: usize,
    /// Options (histoires, chapitres et sous-menus) par menu.
    pub max_options_per_menu: usize,
    /// Histoires (épisodes, chapitres) de toute l'archive.
    pub max_episodes: usize,
    /// Scènes du graphe (garde-fou pour le moteur Lua).
    pub max_stages: usize,
    /// Taille d'un MP3.
    pub max_audio_bytes: u64,
    /// Taille cumulée des MP3, titres lus compris. L'archive est un zip32 (`large_file(false)`) :
    /// la borne doit rester **nettement sous 4 Gio**, images et JSON s'ajoutant à l'audio (ou
    /// passer au zip64 après avoir vérifié que `telmi2flam` le lit).
    pub max_total_audio_bytes: u64,
}

impl Limits {
    /// Bornes des packs de podcasts.
    pub const PODCAST: Limits = Limits {
        max_menu_depth: MAX_MENU_DEPTH,
        max_options_per_menu: MAX_OPTIONS_PER_MENU,
        max_episodes: MAX_EPISODES,
        max_stages: MAX_STAGES,
        max_audio_bytes: MAX_AUDIO_BYTES,
        max_total_audio_bytes: MAX_TOTAL_AUDIO_BYTES,
    };

    // Erreurs de dépassement : avec les bornes des podcasts, les variantes historiques
    // (messages inchangés) ; avec d'autres bornes, `OverLimit` et la borne reçue.

    fn podcast(&self) -> bool {
        *self == Self::PODCAST
    }

    fn too_deep(&self, depth: usize) -> ArchiveError {
        if self.podcast() {
            ArchiveError::TooDeep
        } else {
            over("trop de niveaux de menus", depth, self.max_menu_depth)
        }
    }

    fn too_many_options(&self, n: usize) -> ArchiveError {
        if self.podcast() {
            ArchiveError::TooManyOptions(n)
        } else {
            over("trop d'options dans un menu", n, self.max_options_per_menu)
        }
    }

    fn too_many_episodes(&self, n: usize) -> ArchiveError {
        if self.podcast() {
            ArchiveError::TooManyEpisodes(n)
        } else {
            over("trop d'histoires ou de chapitres", n, self.max_episodes)
        }
    }

    fn too_many_stages(&self, n: usize) -> ArchiveError {
        if self.podcast() {
            ArchiveError::TooManyStages(n)
        } else {
            over("trop de scènes", n, self.max_stages)
        }
    }

    fn too_large(&self, total: u64) -> ArchiveError {
        if self.podcast() {
            ArchiveError::TooLarge(total)
        } else {
            ArchiveError::OverLimit {
                what: "audio trop volumineux au total (octets)",
                count: total,
                max: self.max_total_audio_bytes,
            }
        }
    }
}
impl Default for Limits {
    fn default() -> Self {
        Self::PODCAST
    }
}

fn over(what: &'static str, count: usize, max: usize) -> ArchiveError {
    ArchiveError::OverLimit {
        what,
        count: count as u64,
        max: max as u64,
    }
}

/// Réglages de construction de l'archive qui ne viennent pas du [`Pack`] : mode du menu racine
/// et bornes. Défaut : comportement et bornes des podcasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchiveOptions {
    /// Mode du menu racine, hérité par ses [`MenuEntry::Category`].
    pub mode: MenuMode,
    pub limits: Limits,
}

/// Pack complet à écrire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// Tiré une seule fois à la création de la tâche (N10 a), voir [`new_pack_uuid`].
    pub uuid: Uuid,
    pub metadata: PackMetadata,
    /// PNG 512 × 512 (#215) ; absente : image unie de repli.
    pub cover: Option<PathBuf>,
    /// Menu racine.
    pub menu: Vec<MenuEntry>,
    /// MP3 du titre lu du pack (`title.mp3`, § 12) ; absent : pas de `title.mp3`.
    pub title_audio: Option<PathBuf>,
}

/// Résumé de l'archive écrite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveSummary {
    pub stages: usize,
    pub actions: usize,
    pub episodes: usize,
    pub menus: usize,
    /// Taille de l'archive, en octets.
    pub bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("titre du pack vide")]
    EmptyTitle,
    #[error("{what} trop long : {len} (au plus {max})")]
    TextTooLong {
        what: &'static str,
        len: usize,
        max: usize,
    },
    #[error("aucun épisode dans le pack")]
    NoEpisode,
    #[error("trop d'épisodes : {0} (au plus {MAX_EPISODES})")]
    TooManyEpisodes(usize),
    #[error("menu de {0} options (au plus {MAX_OPTIONS_PER_MENU})")]
    TooManyOptions(usize),
    #[error("catégorie vide : « {0} »")]
    EmptyCategory(String),
    #[error("arborescence trop profonde (au plus {MAX_MENU_DEPTH} niveaux de menus)")]
    TooDeep,
    #[error("épisode présent deux fois : {0}")]
    DuplicateEpisode(String),
    #[error("trop de scènes : {0} (au plus {MAX_STAGES})")]
    TooManyStages(usize),
    #[error("audio {path} : {reason}")]
    InvalidAudio { path: String, reason: String },
    #[error("image {path} : {reason}")]
    InvalidImage { path: String, reason: String },
    #[error("audio trop volumineux au total : {0} octets (au plus {MAX_TOTAL_AUDIO_BYTES})")]
    TooLarge(u64),
    #[error("image de repli : {0}")]
    Artwork(#[from] ArtworkError),
    #[error("écriture de l'archive : {0}")]
    Io(#[from] io::Error),
    #[error("écriture de l'archive : {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("création de l'archive annulée")]
    Cancelled,
    /// Borne d'une autre source que les podcasts dépassée (voir [`Limits`]) ; `what` est le
    /// début du message (« trop de scènes »).
    #[error("{what} : {count} (au plus {max})")]
    OverLimit {
        what: &'static str,
        count: u64,
        max: u64,
    },
    #[error("recueil « {0} » dans un livre à chapitres : non pris en charge")]
    CollectionInBook(String),
}

/// Uuid d'un nouveau pack (P3) : UUID v4. Dans `metadata.json`, il est écrit en minuscules avec
/// tirets ([`Uuid::hyphenated`]) : `telmi2flam` hache la chaîne telle quelle (§ 3.4).
pub fn new_pack_uuid() -> Uuid {
    Uuid::new_v4()
}

/// Description par défaut (D16) : liste des épisodes, dans l'ordre de l'arborescence.
pub fn default_description(menu: &[MenuEntry]) -> String {
    let mut lines = Vec::new();
    collect_titles(menu, &mut lines);
    lines
        .iter()
        .map(|t| format!("- {}", one_line(t)))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Nombre d'épisodes de l'arborescence, catégories comprises.
fn count_episodes(menu: &[MenuEntry]) -> usize {
    menu.iter()
        .map(|e| match e {
            MenuEntry::Episode(_) => 1,
            MenuEntry::Category { entries, .. } | MenuEntry::Submenu { entries, .. } => {
                count_episodes(entries)
            }
        })
        .sum()
}

fn collect_titles<'a>(menu: &'a [MenuEntry], out: &mut Vec<&'a str>) {
    for entry in menu {
        match entry {
            MenuEntry::Episode(e) => out.push(&e.title),
            MenuEntry::Category { entries, .. } | MenuEntry::Submenu { entries, .. } => {
                collect_titles(entries, out)
            }
        }
    }
}

/// Ajoute à la description la liste des épisodes en échec (P7). Sans échec : inchangée.
pub fn with_failures(description: &str, failed: &[String]) -> String {
    if failed.is_empty() {
        return description.to_string();
    }
    let list = failed
        .iter()
        .map(|t| format!("- {}", one_line(t)))
        .collect::<Vec<_>>()
        .join("\n");
    let head = "Épisodes non inclus (échec du téléchargement ou de la conversion) :";
    if description.trim().is_empty() {
        format!("{head}\n{list}")
    } else {
        format!("{}\n\n{head}\n{list}", description.trim_end())
    }
}

/// Longueur d'une ligne de description (titre RSS non borné) : 50 épisodes et 50 échecs
/// tiennent alors largement dans [`MAX_DESCRIPTION_BYTES`].
const DESCRIPTION_LINE_CHARS: usize = 200;

/// Texte sur une ligne, [`DESCRIPTION_LINE_CHARS`] caractères au plus (`…` en fin).
fn one_line(s: &str) -> String {
    let s = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if s.chars().count() <= DESCRIPTION_LINE_CHARS {
        return s;
    }
    let cut: String = s.chars().take(DESCRIPTION_LINE_CHARS - 1).collect();
    format!("{}…", cut.trim_end())
}

/// Lettres latines sans décomposition Unicode, que `telmi2flam` remplacerait par `?`.
const LATIN_EXTRA: [(char, &str); 12] = [
    ('Ł', "L"),
    ('ł', "l"),
    ('Đ', "D"),
    ('đ', "d"),
    ('Ħ', "H"),
    ('ħ', "h"),
    ('ı', "i"),
    ('Ŧ', "T"),
    ('ŧ', "t"),
    ('ƒ', "f"),
    ('Ŋ', "N"),
    ('ŋ', "n"),
];

/// Texte sur une ligne dont les caractères hors police sont translittérés (voir [`label`]),
/// sans troncature : titre et sous-titre du pack (`metadata.json`), que `telmi2flam` affiche
/// tels quels (un `’` y déclencherait l'avertissement `font_replaced`).
fn translit(s: &str) -> String {
    let latin: String = s
        .chars()
        .map(|c| match LATIN_EXTRA.iter().find(|(k, _)| *k == c) {
            Some((_, r)) => (*r).to_string(),
            None => c.to_string(),
        })
        .collect();
    font_text(&latin, None)
}

/// Libellé d'option (A6) : caractères hors police translittérés comme `telmi2flam`
/// (accents hors Latin-1 retirés, typographie remplacée, emoji supprimés, lettres sans
/// équivalent remplacées par `?`, par exemple le grec ou le cyrillique), plus quelques lettres
/// latines ([`LATIN_EXTRA`]) ; une ligne, [`LABEL_MAX`] caractères au plus (`...` en fin) ;
/// `fallback` si vide.
pub fn label(title: &str, fallback: &str) -> String {
    let mut s = translit(title);
    if s.is_empty() {
        s = fallback.to_string();
    }
    if s.chars().count() > LABEL_MAX {
        let cut: String = s.chars().take(LABEL_MAX - 3).collect();
        s = format!("{}...", cut.trim_end());
    }
    s
}

/// Nom de fichier d'un épisode (`n` à partir de 1) : stable, sûr, unique.
fn episode_file(n: usize, ext: &str) -> String {
    format!("e{n:03}.{ext}")
}

/// Titre lu de l'épisode `n` (à partir de 1), dans `audios/`.
fn episode_title_file(n: usize) -> String {
    format!("t{n:03}.mp3")
}

/// Nom lu de la catégorie `n` (à partir de 1), dans `audios/`.
fn category_title_file(n: usize) -> String {
    format!("c{n:03}.mp3")
}

/// Titre lu du pack, à la racine de l'archive (lu par `telmi2flam`).
const TITLE_AUDIO_NAME: &str = "title.mp3";

/// Graphe Telmi construit à partir de l'arborescence (aucune entrée/sortie).
#[derive(Debug, Default)]
struct Graph<'a> {
    stages: Map<String, Value>,
    actions: Map<String, Value>,
    notes: Map<String, Value>,
    /// Épisodes et chapitres dans l'ordre de l'arborescence (parcours en profondeur) : le
    /// n-ième (à partir de 1) a pour fichiers `eNNN.mp3` et `eNNN.png`.
    episodes: Vec<&'a PackEpisode>,
    /// Titres lus des options (§ 12) : nom d'entrée dans `audios/` et MP3 source, dans l'ordre
    /// du parcours.
    title_audios: Vec<(String, &'a Path)>,
    /// Épisodes de tout le pack (calculé avant le parcours), pour le message de dépassement.
    total_episodes: usize,
    menus: usize,
    options: usize,
    categories: usize,
    limits: Limits,
}

/// Transition Telmi `{action, index}`.
fn trans(action: &str, index: usize) -> Value {
    json!({ "action": action, "index": index })
}

/// Contrôles d'une scène d'épisode, de chapitre ou d'annonce.
fn playing_control() -> Value {
    json!({ "ok": true, "home": true, "autoplay": true })
}

/// Chapitre d'un livre en construction.
struct Chapter {
    /// Scène du chapitre (`eN`), dont `ok` mène au suivant.
    stage: String,
    /// Action qui le démarre (`pN`) : annonce puis chapitre, ou chapitre seul.
    entry: String,
}

/// Sous-menu vu comme une option : champs communs à [`MenuEntry::Category`] et
/// [`MenuEntry::Submenu`].
struct Head<'a> {
    name: &'a str,
    entries: &'a [MenuEntry],
    audio: Option<&'a Path>,
    image: Option<&'a PathBuf>,
}

/// Ce que devient un sous-menu.
enum Sub<'c> {
    /// Menu d'un recueil.
    Collection(CollectionEnd),
    /// Livre à chapitres placé dans un recueil.
    Book(ChapterSettings),
    /// Partie d'un livre : ses chapitres s'ajoutent à ceux du livre.
    Part {
        announce: bool,
        chapters: &'c mut Vec<Chapter>,
    },
}

/// Option en construction : image, transition `ok`, libellé, titre lu.
type OptionParts = (String, Value, String, Option<String>);

impl<'a> Graph<'a> {
    fn build(menu: &'a [MenuEntry], mode: MenuMode, limits: Limits) -> Result<Self, ArchiveError> {
        let mut g = Graph {
            total_episodes: count_episodes(menu),
            limits,
            ..Graph::default()
        };
        if menu.is_empty() {
            return Err(ArchiveError::NoEpisode);
        }
        // Scène d'entrée : passage (sans audio, autoplay) vers le menu racine, ou vers le
        // premier chapitre d'un livre (`debut = chapitre`).
        g.stages.insert(
            START_STAGE.into(),
            json!({
                "image": null,
                "audio": null,
                "ok": trans(ROOT_MENU, 0),
                "home": null,
                "control": { "ok": false, "home": false, "autoplay": true },
            }),
        );
        g.actions
            .insert(START_ACTION.into(), json!([{ "stage": START_STAGE }]));
        match mode {
            MenuMode::Collection { end } => {
                g.menu(menu, ROOT_MENU, 1, None, end)?;
            }
            MenuMode::Chapters(settings) => {
                let (_, start) = g.book(menu, ROOT_MENU, 1, None, settings)?;
                if let Some(st) = g.stages.get_mut(START_STAGE) {
                    st["ok"] = start;
                }
            }
        }
        if g.episodes.is_empty() {
            return Err(ArchiveError::NoEpisode);
        }
        if g.stages.len() > g.limits.max_stages {
            return Err(g.limits.too_many_stages(g.stages.len()));
        }
        Ok(g)
    }

    /// Bornes d'un menu (niveau `depth`, racine = 1).
    fn check_menu(&self, entries: &[MenuEntry], depth: usize) -> Result<(), ArchiveError> {
        if depth > self.limits.max_menu_depth {
            return Err(self.limits.too_deep(depth));
        }
        if entries.len() > self.limits.max_options_per_menu {
            return Err(self.limits.too_many_options(entries.len()));
        }
        Ok(())
    }

    /// Enregistre un épisode ou un chapitre : rang `n` (à partir de 1), image, libellé, titre lu.
    fn episode(
        &mut self,
        ep: &'a PackEpisode,
        fallback: &str,
    ) -> Result<(usize, String, String, Option<String>), ArchiveError> {
        if self.episodes.len() >= self.limits.max_episodes {
            return Err(self.limits.too_many_episodes(self.total_episodes));
        }
        self.episodes.push(ep);
        let n = self.episodes.len();
        let text = label(&ep.title, &format!("{fallback} {n}"));
        let spoken = ep.title_audio.as_deref().map(|mp3| {
            let file = episode_title_file(n);
            self.title_audios.push((file.clone(), mp3));
            file
        });
        Ok((n, episode_file(n, "png"), text, spoken))
    }

    /// Image d'une option de sous-menu (N5 c) : le fichier de l'épisode choisi s'il est dans le
    /// sous-menu (rangs d'archive `first + 1` et suivants), sinon `auto` (N5 a).
    fn chosen_image(&self, chosen: Option<&PathBuf>, first: usize, auto: String) -> String {
        chosen
            .and_then(|png| {
                self.episodes[first..]
                    .iter()
                    .position(|ep| ep.image.as_ref() == Some(png))
            })
            .map_or(auto, |p| episode_file(first + p + 1, "png"))
    }

    /// Option qui ouvre un sous-menu, placée en `parent` (action du menu, position) au niveau
    /// `depth`.
    fn submenu(
        &mut self,
        head: Head<'a>,
        parent: (&str, usize),
        depth: usize,
        kind: Sub<'_>,
    ) -> Result<OptionParts, ArchiveError> {
        self.categories += 1;
        let k = self.categories;
        let fallback = format!("Catégorie {k}");
        if head.entries.is_empty() {
            return Err(ArchiveError::EmptyCategory(one_line(head.name)));
        }
        // Nom lu noté avant le sous-menu : ordre du parcours (la catégorie, puis ses options).
        let spoken = head.audio.map(|mp3| {
            let file = category_title_file(k);
            self.title_audios.push((file.clone(), mp3));
            file
        });
        let sub = format!("m{k}");
        let first = self.episodes.len();
        let (image, ok) = match kind {
            Sub::Collection(end) => (
                self.menu(head.entries, &sub, depth + 1, Some(parent), end)?,
                trans(&sub, 0),
            ),
            Sub::Book(settings) => {
                self.book(head.entries, &sub, depth + 1, Some(parent), settings)?
            }
            Sub::Part { announce, chapters } => {
                let home = trans(parent.0, parent.1);
                let image = self.chapter_menu(
                    head.entries,
                    &sub,
                    depth + 1,
                    Some(&home),
                    announce,
                    chapters,
                )?;
                (image, trans(&sub, 0))
            }
        };
        let image = self.chosen_image(head.image, first, image);
        Ok((image, ok, label(head.name, &fallback), spoken))
    }

    /// Construit le menu de recueil `action` (niveau `depth`, racine = 1) ; `parent` = action
    /// du menu parent et position de la catégorie ; `end` = réglage `fin` de ce menu. Rend le
    /// nom de l'image du premier épisode (N5 a).
    fn menu(
        &mut self,
        entries: &'a [MenuEntry],
        action: &str,
        depth: usize,
        parent: Option<(&str, usize)>,
        end: CollectionEnd,
    ) -> Result<String, ArchiveError> {
        self.check_menu(entries, depth)?;
        self.menus += 1;
        let mut options = Vec::with_capacity(entries.len());
        // (scène de l'épisode, action « jouer », position de son option), dans l'ordre du menu
        let mut played: Vec<(String, String, usize)> = Vec::new();
        let mut first_image: Option<String> = None;
        for (i, entry) in entries.iter().enumerate() {
            self.options += 1;
            let option = format!("o{}", self.options);
            let (image, ok, text, spoken) = match entry {
                MenuEntry::Episode(ep) => {
                    let (n, image, text, spoken) = self.episode(ep, "Épisode")?;
                    let stage = format!("e{n}");
                    let play = format!("p{n}");
                    self.stages.insert(
                        stage.clone(),
                        json!({
                            "image": image,
                            "audio": episode_file(n, "mp3"),
                            "ok": trans(action, i), // remplacé plus bas si un suivant existe
                            "home": trans(action, i),
                            "control": playing_control(),
                        }),
                    );
                    self.notes.insert(stage.clone(), json!({ "title": text }));
                    self.actions
                        .insert(play.clone(), json!([{ "stage": stage }]));
                    played.push((stage, play.clone(), i));
                    (image, trans(&play, 0), text, spoken)
                }
                MenuEntry::Category {
                    name,
                    entries,
                    audio,
                    image,
                } => self.submenu(
                    Head {
                        name,
                        entries,
                        audio: audio.as_deref(),
                        image: image.as_ref(),
                    },
                    (action, i),
                    depth,
                    Sub::Collection(end),
                )?,
                MenuEntry::Submenu {
                    name,
                    entries,
                    audio,
                    image,
                    mode,
                } => {
                    let kind = match *mode {
                        MenuMode::Collection { end } => Sub::Collection(end),
                        MenuMode::Chapters(settings) => Sub::Book(settings),
                    };
                    self.submenu(
                        Head {
                            name,
                            entries,
                            audio: audio.as_deref(),
                            image: image.as_ref(),
                        },
                        (action, i),
                        depth,
                        kind,
                    )?
                }
            };
            if first_image.is_none() {
                first_image = Some(image.clone());
            }
            let (home, control_home) = match parent {
                Some((pa, pi)) => (trans(pa, pi), true),
                None => (Value::Null, false),
            };
            self.insert_option(&option, image, spoken, ok, home, control_home, text);
            options.push(json!({ "stage": option }));
        }
        // Fin d'un épisode : `suivante` (P8, N4 a) = épisode suivant du même menu ; `menu` = ce
        // menu, focalisé sur l'épisode suivant. Le dernier garde le retour à ce menu, focalisé
        // sur lui, posé à la création.
        for pair in played.windows(2) {
            let (stage, _, _) = &pair[0];
            let (_, next_play, next_option) = &pair[1];
            let ok = match end {
                CollectionEnd::Next => trans(next_play, 0),
                CollectionEnd::Menu => trans(action, *next_option),
            };
            if let Some(st) = self.stages.get_mut(stage) {
                st["ok"] = ok;
            }
        }
        self.actions
            .insert(action.to_string(), Value::Array(options));
        // Menu non vide (vérifié par l'appelant) : au moins une option, donc une image.
        first_image.ok_or(ArchiveError::NoEpisode)
    }

    /// Scène d'option d'un menu.
    #[allow(clippy::too_many_arguments)]
    fn insert_option(
        &mut self,
        option: &str,
        image: String,
        spoken: Option<String>,
        ok: Value,
        home: Value,
        control_home: bool,
        text: String,
    ) {
        self.stages.insert(
            option.to_string(),
            json!({
                "image": image,
                "audio": spoken,
                "ok": ok,
                "home": home,
                "control": { "ok": true, "home": control_home, "autoplay": false },
            }),
        );
        self.notes
            .insert(option.to_string(), json!({ "title": text }));
    }

    /// Construit le livre à chapitres dont le menu des chapitres est `action` (niveau
    /// `depth`) ; `parent` = menu du recueil qui le contient et position de son option, `None`
    /// pour le livre racine. Rend l'image du premier chapitre (N5 a) et la transition qui
    /// démarre le livre (`debut`).
    fn book(
        &mut self,
        entries: &'a [MenuEntry],
        action: &str,
        depth: usize,
        parent: Option<(&str, usize)>,
        settings: ChapterSettings,
    ) -> Result<(String, Value), ArchiveError> {
        // Retour depuis le premier niveau du menu des chapitres : le recueil qui contient le
        // livre (focalisé sur lui) ; pour le livre racine, retour par défaut du moteur (menu
        // Start, sortie de l'histoire), comme depuis le menu racine d'un recueil.
        let home = parent.map(|(pa, pi)| trans(pa, pi));
        let mut chapters = Vec::new();
        let image = self.chapter_menu(
            entries,
            action,
            depth,
            home.as_ref(),
            settings.announce,
            &mut chapters,
        )?;
        // Enchaînement en profondeur : les parties sont traversées sans arrêt.
        for pair in chapters.windows(2) {
            if let Some(st) = self.stages.get_mut(&pair[0].stage) {
                st["ok"] = trans(&pair[1].entry, 0);
            }
        }
        let (Some(first), Some(last)) = (chapters.first(), chapters.last()) else {
            return Err(ArchiveError::NoEpisode);
        };
        let end = match settings.end {
            BookEnd::Menu => trans(action, 0),
            BookEnd::End => Value::Null,
        };
        let start = match settings.start {
            ChapterStart::Chapter => trans(&first.entry, 0),
            ChapterStart::Menu => trans(action, 0),
        };
        if let Some(st) = self.stages.get_mut(&last.stage) {
            st["ok"] = end;
        }
        Ok((image, start))
    }

    /// Un niveau du menu des chapitres (`action`, niveau `depth`) : une option par chapitre,
    /// un sous-menu par partie. `home` = retour depuis ses options (`None` : retour par défaut
    /// du moteur) ; `announce` = réglage `annoncer` hérité.
    /// Les chapitres sont ajoutés à `chapters` dans l'ordre de lecture ; leur `ok` est posé par
    /// [`Graph::book`].
    #[allow(clippy::too_many_arguments)]
    fn chapter_menu(
        &mut self,
        entries: &'a [MenuEntry],
        action: &str,
        depth: usize,
        home: Option<&Value>,
        announce: bool,
        chapters: &mut Vec<Chapter>,
    ) -> Result<String, ArchiveError> {
        self.check_menu(entries, depth)?;
        self.menus += 1;
        let mut options = Vec::with_capacity(entries.len());
        let mut first_image: Option<String> = None;
        for (i, entry) in entries.iter().enumerate() {
            self.options += 1;
            let option = format!("o{}", self.options);
            let (image, ok, text, spoken) = match entry {
                MenuEntry::Episode(ep) => {
                    let (n, image, text, spoken) = self.episode(ep, "Chapitre")?;
                    let stage = format!("e{n}");
                    let entry_action = format!("p{n}");
                    let st = json!({
                        "image": image,
                        "audio": episode_file(n, "mp3"),
                        "ok": null, // posé par `book`
                        "home": trans(action, i),
                        "control": playing_control(),
                    });
                    self.stages.insert(stage.clone(), st);
                    self.notes.insert(stage.clone(), json!({ "title": text }));
                    match spoken.as_ref().filter(|_| announce) {
                        // Annonce : titre lu sur l'image du chapitre, puis le chapitre.
                        Some(title) => {
                            let intro = format!("n{n}");
                            let then = format!("q{n}");
                            let st = json!({
                                "image": image,
                                "audio": title,
                                "ok": trans(&then, 0),
                                "home": trans(action, i),
                                "control": playing_control(),
                            });
                            self.stages.insert(intro.clone(), st);
                            self.actions
                                .insert(entry_action.clone(), json!([{ "stage": intro }]));
                            self.actions.insert(then, json!([{ "stage": stage }]));
                        }
                        None => {
                            self.actions
                                .insert(entry_action.clone(), json!([{ "stage": stage }]));
                        }
                    }
                    chapters.push(Chapter {
                        stage,
                        entry: entry_action.clone(),
                    });
                    (image, trans(&entry_action, 0), text, spoken)
                }
                MenuEntry::Category {
                    name,
                    entries,
                    audio,
                    image,
                } => self.submenu(
                    Head {
                        name,
                        entries,
                        audio: audio.as_deref(),
                        image: image.as_ref(),
                    },
                    (action, i),
                    depth,
                    Sub::Part {
                        announce,
                        chapters: &mut *chapters,
                    },
                )?,
                MenuEntry::Submenu {
                    name,
                    entries,
                    audio,
                    image,
                    mode,
                } => {
                    let MenuMode::Chapters(settings) = mode else {
                        return Err(ArchiveError::CollectionInBook(one_line(name)));
                    };
                    self.submenu(
                        Head {
                            name,
                            entries,
                            audio: audio.as_deref(),
                            image: image.as_ref(),
                        },
                        (action, i),
                        depth,
                        Sub::Part {
                            announce: settings.announce,
                            chapters: &mut *chapters,
                        },
                    )?
                }
            };
            if first_image.is_none() {
                first_image = Some(image.clone());
            }
            let (home, control_home) = match home {
                Some(h) => (h.clone(), true),
                None => (Value::Null, false),
            };
            self.insert_option(&option, image, spoken, ok, home, control_home, text);
            options.push(json!({ "stage": option }));
        }
        self.actions
            .insert(action.to_string(), Value::Array(options));
        first_image.ok_or(ArchiveError::NoEpisode)
    }
}

fn check_text(what: &'static str, s: &str, max_chars: usize) -> Result<(), ArchiveError> {
    let len = s.chars().count();
    if len > max_chars {
        return Err(ArchiveError::TextTooLong {
            what,
            len,
            max: max_chars,
        });
    }
    Ok(())
}

fn check_metadata(m: &PackMetadata) -> Result<(), ArchiveError> {
    if m.title.trim().is_empty() {
        return Err(ArchiveError::EmptyTitle);
    }
    check_text("titre", &m.title, MAX_TITLE_CHARS)?;
    check_text("sous-titre", &m.subtitle, MAX_TITLE_CHARS)?;
    if m.description.len() > MAX_DESCRIPTION_BYTES {
        return Err(ArchiveError::TextTooLong {
            what: "description (octets)",
            len: m.description.len(),
            max: MAX_DESCRIPTION_BYTES,
        });
    }
    Ok(())
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

/// Taille d'un fichier ordinaire, avec son en-tête (`head.len()` octets au plus).
fn probe(path: &Path, head: &mut [u8]) -> io::Result<(u64, usize)> {
    let mut f = File::open(path)?;
    let meta = f.metadata()?;
    if !meta.is_file() {
        return Err(io::Error::other("pas un fichier ordinaire"));
    }
    let mut n = 0;
    while n < head.len() {
        match f.read(&mut head[n..]) {
            Ok(0) => break,
            Ok(read) => n += read,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok((meta.len(), n))
}

/// MP3 : non vide, borné, commençant par une étiquette ID3 ou une synchronisation de trame.
fn check_audio(path: &Path, max_bytes: u64) -> Result<u64, ArchiveError> {
    let bad = |reason: String| ArchiveError::InvalidAudio {
        path: shown(path),
        reason,
    };
    let mut head = [0u8; 3];
    let (len, n) = probe(path, &mut head).map_err(|e| bad(e.to_string()))?;
    if len == 0 {
        return Err(bad("fichier vide".into()));
    }
    if n < 2 {
        return Err(bad("pas un fichier MP3".into()));
    }
    if len > max_bytes {
        return Err(bad(format!("{len} octets (au plus {max_bytes})")));
    }
    let id3 = n == 3 && &head == b"ID3";
    let sync = head[0] == 0xFF && head[1] & 0xE0 == 0xE0;
    if !id3 && !sync {
        return Err(bad("pas un fichier MP3".into()));
    }
    Ok(len)
}

/// PNG : signature, taille bornée. Rend les octets.
fn read_png(path: &Path) -> Result<Vec<u8>, ArchiveError> {
    let bad = |reason: String| ArchiveError::InvalidImage {
        path: shown(path),
        reason,
    };
    let f = File::open(path).map_err(|e| bad(e.to_string()))?;
    let meta = f.metadata().map_err(|e| bad(e.to_string()))?;
    if !meta.is_file() {
        return Err(bad("pas un fichier ordinaire".into()));
    }
    let mut bytes = Vec::new();
    // Une lecture de plus que la borne : un fichier qui a grossi depuis `metadata` est refusé.
    f.take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| bad(e.to_string()))?;
    if bytes.len() as u64 > MAX_IMAGE_BYTES {
        return Err(bad(format!("plus de {MAX_IMAGE_BYTES} octets")));
    }
    if !bytes.starts_with(&PNG_SIGNATURE) {
        return Err(bad("pas une image PNG".into()));
    }
    Ok(bytes)
}

/// Vérifie les fichiers d'entrée (avant toute écriture) : MP3, PNG, doublons (N9 a), bornes.
///
/// Le doublon (N9 a) est détecté sur le **chemin tel que reçu**, sans canonicalisation : deux
/// écritures différentes du même fichier (casse, `..`, lien) ne sont pas vues. L'appelant (#217)
/// génère les chemins des MP3 dans le dossier de travail de la tâche, un par épisode, ce
/// contrôle ne garde donc que d'une erreur de construction du pack.
fn check_files(pack: &Pack, graph: &Graph<'_>) -> Result<(), ArchiveError> {
    let limits = &graph.limits;
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for ep in &graph.episodes {
        if !seen.insert(ep.audio.as_path()) {
            return Err(ArchiveError::DuplicateEpisode(one_line(&ep.title)));
        }
        total += check_audio(&ep.audio, limits.max_audio_bytes)?;
        if total > limits.max_total_audio_bytes {
            return Err(limits.too_large(total));
        }
        if let Some(img) = &ep.image {
            read_png(img)?;
        }
    }
    // Titres lus (§ 12) : mêmes contrôles, comptés dans le total ; un même fichier ne sert
    // jamais deux fois (erreur de construction du pack).
    let titles = graph
        .title_audios
        .iter()
        .map(|(_, path)| *path)
        .chain(pack.title_audio.as_deref());
    for path in titles {
        if !seen.insert(path) {
            return Err(ArchiveError::InvalidAudio {
                path: shown(path),
                reason: "fichier utilisé deux fois".into(),
            });
        }
        total += check_audio(path, limits.max_audio_bytes)?;
        if total > limits.max_total_audio_bytes {
            return Err(limits.too_large(total));
        }
    }
    if let Some(cover) = &pack.cover {
        read_png(cover)?;
    }
    Ok(())
}

fn metadata_json(pack: &Pack) -> Value {
    let m = &pack.metadata;
    let mut title = translit(m.title.trim());
    if title.is_empty() {
        // Titre fait uniquement d'emoji ou de symboles : jamais de titre vide dans l'archive.
        title = "Podcast".to_string();
    }
    json!({
        "title": title,
        "uuid": pack.uuid.hyphenated().to_string(),
        "image": COVER_NAME,
        "version": 1,
        "category": translit(m.subtitle.trim()),
        "description": m.description,
        "age": m.age,
    })
}

/// Temporaire voisin unique (par processus et par appel).
fn temp_path(dest: &Path) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let parent = dest
        .parent()
        .ok_or_else(|| io::Error::other("chemin sans dossier parent"))?;
    let name = dest
        .file_name()
        .ok_or_else(|| io::Error::other("chemin sans nom de fichier"))?;
    Ok(parent.join(format!(
        ".{}.{}-{}.tmp",
        name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    )))
}

/// Écrit l'archive Telmi d'un pack de podcasts à `dest`, de façon atomique : recueil
/// `fin = suivante`, bornes des podcasts ([`ArchiveOptions::default`]). `cancelled` est
/// consulté entre deux entrées et entre deux blocs d'audio.
pub fn write_archive(
    pack: &Pack,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ArchiveSummary, ArchiveError> {
    write_archive_with(pack, &ArchiveOptions::default(), dest, cancelled)
}

/// [`write_archive`] avec le mode du menu racine et les bornes choisis par l'appelant.
pub fn write_archive_with(
    pack: &Pack,
    options: &ArchiveOptions,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ArchiveSummary, ArchiveError> {
    check_metadata(&pack.metadata)?;
    let graph = Graph::build(&pack.menu, options.mode, options.limits)?;
    check_files(pack, &graph)?;
    if cancelled() {
        return Err(ArchiveError::Cancelled);
    }
    let tmp = temp_path(dest)?;
    let result = (|| {
        let file = File::create_new(&tmp)?;
        let bytes = write_zip(pack, &graph, BufWriter::new(file), cancelled)?;
        crate::storage::rename_with_retry(&tmp, dest)?;
        Ok(bytes)
    })();
    match result {
        Ok(bytes) => Ok(ArchiveSummary {
            stages: graph.stages.len(),
            actions: graph.actions.len(),
            episodes: graph.episodes.len(),
            menus: graph.menus,
            bytes,
        }),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Contenu de l'archive ; rend sa taille une fois synchronisée sur le disque.
fn write_zip(
    pack: &Pack,
    graph: &Graph<'_>,
    out: BufWriter<File>,
    cancelled: &dyn Fn() -> bool,
) -> Result<u64, ArchiveError> {
    // Stockées sans compression (MP3 et PNG déjà compressés) ; date fixe (1980) par défaut.
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(false);
    let mut zip = ZipWriter::new(out);
    let check = || {
        if cancelled() {
            Err(ArchiveError::Cancelled)
        } else {
            Ok(())
        }
    };
    let nodes = json!({
        "startAction": trans(START_ACTION, 0),
        "stages": Value::Object(graph.stages.clone()),
        "actions": Value::Object(graph.actions.clone()),
    });
    let jsons = [
        ("metadata.json", metadata_json(pack)),
        ("nodes.json", nodes),
        ("notes.json", Value::Object(graph.notes.clone())),
    ];
    for (name, value) in jsons {
        zip.start_file(name, options)?;
        zip.write_all(&serde_json::to_vec_pretty(&value).map_err(io::Error::other)?)?;
    }
    check()?;
    let cover = match &pack.cover {
        Some(path) => read_png(path)?,
        None => Artwork::fallback(ArtworkKind::Cover, FALLBACK_COLOR)?.png,
    };
    zip.start_file(COVER_NAME, options)?;
    zip.write_all(&cover)?;
    let mut fallback: Option<Vec<u8>> = None;
    let mut buf = vec![0u8; COPY_CHUNK];
    let mut total = 0u64;
    for (i, ep) in graph.episodes.iter().enumerate() {
        let n = i + 1;
        check()?;
        let png = match &ep.image {
            Some(path) => read_png(path)?,
            None => match &fallback {
                Some(png) => png.clone(),
                None => {
                    let png = Artwork::fallback(ArtworkKind::Episode, FALLBACK_COLOR)?.png;
                    fallback = Some(png.clone());
                    png
                }
            },
        };
        zip.start_file(format!("images/{}", episode_file(n, "png")), options)?;
        zip.write_all(&png)?;
        zip.start_file(format!("audios/{}", episode_file(n, "mp3")), options)?;
        copy_audio(
            &ep.audio,
            &mut zip,
            &mut buf,
            &mut total,
            &graph.limits,
            &check,
        )?;
    }
    for (name, path) in &graph.title_audios {
        check()?;
        zip.start_file(format!("audios/{name}"), options)?;
        copy_audio(path, &mut zip, &mut buf, &mut total, &graph.limits, &check)?;
    }
    if let Some(path) = &pack.title_audio {
        check()?;
        zip.start_file(TITLE_AUDIO_NAME, options)?;
        copy_audio(path, &mut zip, &mut buf, &mut total, &graph.limits, &check)?;
    }
    check()?;
    let mut out = zip.finish()?;
    out.flush()?;
    let file = out.into_inner().map_err(|e| e.into_error())?;
    file.sync_all()?;
    let mut file = file;
    Ok(file.stream_position()?)
}

/// Copie un MP3 par blocs ; refuse un fichier qui a dépassé la borne depuis la vérification.
fn copy_audio<W: Write>(
    path: &Path,
    out: &mut W,
    buf: &mut [u8],
    total: &mut u64,
    limits: &Limits,
    check: &dyn Fn() -> Result<(), ArchiveError>,
) -> Result<(), ArchiveError> {
    let mut src = File::open(path).map_err(|e| ArchiveError::InvalidAudio {
        path: shown(path),
        reason: e.to_string(),
    })?;
    let mut copied = 0u64;
    loop {
        check()?;
        let n = match src.read(buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e.into()),
        };
        copied += n as u64;
        if copied > limits.max_audio_bytes {
            return Err(ArchiveError::InvalidAudio {
                path: shown(path),
                reason: format!("plus de {} octets", limits.max_audio_bytes),
            });
        }
        *total += n as u64;
        if *total > limits.max_total_audio_bytes {
            return Err(limits.too_large(*total));
        }
        out.write_all(&buf[..n])?;
    }
    if copied == 0 {
        return Err(ArchiveError::InvalidAudio {
            path: shown(path),
            reason: "fichier vide".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
