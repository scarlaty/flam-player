//! Archive Telmi synthétique d'un pack de podcasts (#216, spécification
//! `docs/podcasts.md` § 6).
//!
//! Un pack est une **nouvelle histoire** (P3) : uuid aléatoire tiré une fois par l'appelant
//! ([`new_pack_uuid`]), version fixe 1. L'arborescence de menus (N2 b) devient un graphe Telmi
//! que `telmi2flam` convertit sans modification :
//!
//! - `startAction` → scène d'entrée (passage sans audio) → menu racine ;
//! - un menu est une action à plusieurs entrées (carrousel) ; chaque option est une scène
//!   image sans audio dont `ok` mène à l'action « jouer l'épisode » (action à une entrée) ou à
//!   l'action du sous-menu ;
//! - un épisode est une scène `autoplay` : à sa fin, l'épisode **suivant du même menu** (les
//!   catégories sont sautées) ; la fin du dernier retourne à **ce menu**, focalisé sur l'épisode
//!   joué (P8, N4 a) ;
//! - retour (`home`) : d'un épisode vers son menu, d'une option de sous-menu vers le menu
//!   parent (focalisé sur la catégorie) ; le menu racine garde le retour par défaut du moteur ;
//! - libellés (`notes.json`) : titres translittérés pour les polices du player
//!   (`telmi2flam::text`), 40 caractères au plus (A6) ;
//! - images : celle de l'épisode, sinon l'image unie de repli (D13) ; image d'une catégorie =
//!   image de son premier épisode (N5 a) ; couverture choisie par l'appelant (N6 b), sinon
//!   repli.
//!
//! Module pur (ni réseau, ni Tauri) : les MP3 (#214) et PNG (#215) sont déjà sur le disque.
//!
//! # Invariants
//!
//! - **Écriture atomique** : temporaire voisin unique, `sync_all`, renommage ; tout échec ou
//!   annulation supprime le temporaire, la destination n'est jamais partielle.
//! - **Noms d'entrées sûrs** : seulement des noms générés ici, jamais tirés d'un titre ni d'un
//!   chemin reçu.
//! - **Bornes** vérifiées avant toute écriture : voir les constantes `MAX_*`.
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
/// 2 catégories par épisode sur 3 niveaux), soit 201. Garde-fou pour le moteur Lua.
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
}

/// Option d'un menu : un épisode ou une catégorie (sous-menu).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Episode(PackEpisode),
    Category {
        name: String,
        entries: Vec<MenuEntry>,
    },
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
            MenuEntry::Category { entries, .. } => count_episodes(entries),
        })
        .sum()
}

fn collect_titles<'a>(menu: &'a [MenuEntry], out: &mut Vec<&'a str>) {
    for entry in menu {
        match entry {
            MenuEntry::Episode(e) => out.push(&e.title),
            MenuEntry::Category { entries, .. } => collect_titles(entries, out),
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

/// Graphe Telmi construit à partir de l'arborescence (aucune entrée/sortie).
#[derive(Debug, Default)]
struct Graph<'a> {
    stages: Map<String, Value>,
    actions: Map<String, Value>,
    notes: Map<String, Value>,
    /// Épisodes dans l'ordre de l'arborescence (parcours en profondeur) : le n-ième (à partir
    /// de 1) a pour fichiers `eNNN.mp3` et `eNNN.png`.
    episodes: Vec<&'a PackEpisode>,
    /// Épisodes de tout le pack (calculé avant le parcours), pour le message de dépassement.
    total_episodes: usize,
    menus: usize,
    options: usize,
    categories: usize,
}

/// Transition Telmi `{action, index}`.
fn trans(action: &str, index: usize) -> Value {
    json!({ "action": action, "index": index })
}

impl<'a> Graph<'a> {
    fn build(menu: &'a [MenuEntry]) -> Result<Self, ArchiveError> {
        let mut g = Graph {
            total_episodes: count_episodes(menu),
            ..Graph::default()
        };
        if menu.is_empty() {
            return Err(ArchiveError::NoEpisode);
        }
        // Scène d'entrée : passage (sans audio, autoplay) vers le menu racine.
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
        g.menu(menu, ROOT_MENU, 1, None)?;
        if g.episodes.is_empty() {
            return Err(ArchiveError::NoEpisode);
        }
        if g.stages.len() > MAX_STAGES {
            return Err(ArchiveError::TooManyStages(g.stages.len()));
        }
        Ok(g)
    }

    /// Construit le menu `action` (niveau `depth`, racine = 1) ; `parent` = action du menu
    /// parent et position de la catégorie. Rend le nom de l'image du premier épisode (N5 a).
    fn menu(
        &mut self,
        entries: &'a [MenuEntry],
        action: &str,
        depth: usize,
        parent: Option<(&str, usize)>,
    ) -> Result<String, ArchiveError> {
        if depth > MAX_MENU_DEPTH {
            return Err(ArchiveError::TooDeep);
        }
        if entries.len() > MAX_OPTIONS_PER_MENU {
            return Err(ArchiveError::TooManyOptions(entries.len()));
        }
        self.menus += 1;
        let mut options = Vec::with_capacity(entries.len());
        // (scène de l'épisode, action « jouer »), dans l'ordre du menu
        let mut played: Vec<(String, String)> = Vec::new();
        let mut first_image: Option<String> = None;
        for (i, entry) in entries.iter().enumerate() {
            self.options += 1;
            let option = format!("o{}", self.options);
            let (image, ok, text) = match entry {
                MenuEntry::Episode(ep) => {
                    if self.episodes.len() >= MAX_EPISODES {
                        return Err(ArchiveError::TooManyEpisodes(self.total_episodes));
                    }
                    self.episodes.push(ep);
                    let n = self.episodes.len();
                    let stage = format!("e{n}");
                    let play = format!("p{n}");
                    let image = episode_file(n, "png");
                    self.stages.insert(
                        stage.clone(),
                        json!({
                            "image": image,
                            "audio": episode_file(n, "mp3"),
                            "ok": trans(action, i), // remplacé plus bas si un suivant existe
                            "home": trans(action, i),
                            "control": { "ok": true, "home": true, "autoplay": true },
                        }),
                    );
                    let text = label(&ep.title, &format!("Épisode {n}"));
                    self.notes.insert(stage.clone(), json!({ "title": text }));
                    self.actions
                        .insert(play.clone(), json!([{ "stage": stage }]));
                    played.push((stage, play.clone()));
                    (image, trans(&play, 0), text)
                }
                MenuEntry::Category { name, entries } => {
                    self.categories += 1;
                    let fallback = format!("Catégorie {}", self.categories);
                    if entries.is_empty() {
                        return Err(ArchiveError::EmptyCategory(one_line(name)));
                    }
                    let sub = format!("m{}", self.categories);
                    let image = self.menu(entries, &sub, depth + 1, Some((action, i)))?;
                    (image, trans(&sub, 0), label(name, &fallback))
                }
            };
            if first_image.is_none() {
                first_image = Some(image.clone());
            }
            let (home, control_home) = match parent {
                Some((pa, pi)) => (trans(pa, pi), true),
                None => (Value::Null, false),
            };
            self.stages.insert(
                option.clone(),
                json!({
                    "image": image,
                    "audio": null,
                    "ok": ok,
                    "home": home,
                    "control": { "ok": true, "home": control_home, "autoplay": false },
                }),
            );
            self.notes.insert(option.clone(), json!({ "title": text }));
            options.push(json!({ "stage": option }));
        }
        // Enchaînement (P8, N4 a) : épisode suivant du même menu ; le dernier garde le retour
        // à ce menu posé à la création.
        for pair in played.windows(2) {
            let (stage, _) = &pair[0];
            let (_, next_play) = &pair[1];
            if let Some(st) = self.stages.get_mut(stage) {
                st["ok"] = trans(next_play, 0);
            }
        }
        self.actions
            .insert(action.to_string(), Value::Array(options));
        // Menu non vide (vérifié par l'appelant) : au moins une option, donc une image.
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
fn check_audio(path: &Path) -> Result<u64, ArchiveError> {
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
    if len > MAX_AUDIO_BYTES {
        return Err(bad(format!("{len} octets (au plus {MAX_AUDIO_BYTES})")));
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
fn check_files(pack: &Pack, episodes: &[&PackEpisode]) -> Result<(), ArchiveError> {
    let mut seen = HashSet::new();
    let mut total = 0u64;
    for ep in episodes {
        if !seen.insert(ep.audio.as_path()) {
            return Err(ArchiveError::DuplicateEpisode(one_line(&ep.title)));
        }
        total += check_audio(&ep.audio)?;
        if total > MAX_TOTAL_AUDIO_BYTES {
            return Err(ArchiveError::TooLarge(total));
        }
        if let Some(img) = &ep.image {
            read_png(img)?;
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

/// Écrit l'archive Telmi du pack à `dest`, de façon atomique. `cancelled` est consulté entre
/// deux entrées et entre deux blocs d'audio.
pub fn write_archive(
    pack: &Pack,
    dest: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<ArchiveSummary, ArchiveError> {
    check_metadata(&pack.metadata)?;
    let graph = Graph::build(&pack.menu)?;
    check_files(pack, &graph.episodes)?;
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
        copy_audio(&ep.audio, &mut zip, &mut buf, &mut total, &check)?;
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
        if copied > MAX_AUDIO_BYTES {
            return Err(ArchiveError::InvalidAudio {
                path: shown(path),
                reason: format!("plus de {MAX_AUDIO_BYTES} octets"),
            });
        }
        *total += n as u64;
        if *total > MAX_TOTAL_AUDIO_BYTES {
            return Err(ArchiveError::TooLarge(*total));
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
