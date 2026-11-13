//! Lecture et validation du `story.json` d'une archive STUdio.
//!
//! Le launcher lit `story.json` dans l'archive (taille déjà bornée par lui) et passe ses octets à
//! [`parse_story`]. Le résultat, [`Story`], est un graphe typé et cohérent : toutes les
//! références sont résolues en indices, les noms d'assets sont sûrs, les nombres tiennent dans
//! les champs binaires de l'index. [`crate::index`] le transforme ensuite en `ni`/`li`/`ri`/`si`.
//!
//! # Champs lus
//!
//! Références : note de conception `docs/studio-lunii.md` (§ 2.2, § 6) et `StudioStory.load` de
//! Lunii.QT (commit c8afe43). Les autres champs de STUdio (`name`, `type`, `position`,
//! `groupId`…) sont ignorés, comme chez Lunii.QT.
//!
//! | Champ JSON                          | Type attendu                         | Devient                                 |
//! |-------------------------------------|--------------------------------------|-----------------------------------------|
//! | `format`                            | `"v1"` seulement                     | [`Story::format_version`] (1)           |
//! | `version`                           | entier 0..=65535                     | [`Story::pack_version`] (`u16`)         |
//! | `title`, `description`              | chaîne, `null` ou absent             | chaîne (vide si absent)                 |
//! | `nightModeAvailable`                | booléen, `null` ou absent            | [`Story::night_mode`] (faux si absent)  |
//! | `stageNodes[]`                      | tableau non vide                     | [`StageNode`], dans l'ordre             |
//! | `stageNodes[].uuid`                 | UUID canonique (36 caractères)       | [`StageNode::uuid`]                     |
//! | `stageNodes[].squareOne`            | booléen, `null` ou absent            | nœud d'entrée, placé en tête            |
//! | `stageNodes[].image` / `.audio`     | chaîne, `""`, `null` ou absent       | `Some(nom)`, ou `None` si vide/absent   |
//! | `stageNodes[].okTransition`         | objet, `null` ou absent              | [`Transition`] ou `None`                |
//! | `stageNodes[].homeTransition`       | objet, `null` ou absent              | [`Transition`] ou `None`                |
//! | `…Transition.actionNode`            | `id` d'un actionNode                 | [`Transition::action_node`] (indice)    |
//! | `…Transition.optionIndex`           | entier de 0 à options - 1 (`-1` si 1 option) | [`Transition::option_index`]    |
//! | `stageNodes[].controlSettings`      | objet de 5 booléens, obligatoire     | [`StageNode::controls`]                 |
//! | `actionNodes[]`                     | tableau (peut être vide)             | [`ActionNode`], dans l'ordre            |
//! | `actionNodes[].id`                  | chaîne unique                        | [`ActionNode::id`]                      |
//! | `actionNodes[].options[]`           | UUID de stageNodes, au moins un      | [`ActionNode::options`] (indices)       |
//!
//! Nœud d'entrée (note § 2.2) : au plus un `stageNode` porte `squareOne: true` ; s'il n'est pas
//! en tête, il y est déplacé (les autres gardent leur ordre, tous les indices sont calculés
//! après ce déplacement) ; sans `squareOne`, le premier nœud est l'entrée, comme Lunii.QT.
//! L'UUID de l'histoire est celui du nœud d'entrée, en minuscules, comme `StudioStory.uuid`.
//!
//! # Écarts choisis avec Lunii.QT (note § 2.4, § 3.2, § 6.3, Q5)
//!
//! - `format` autre que `"v1"` refusé (Lunii.QT recopie n'importe quel nombre) ;
//! - `controlSettings` obligatoire (Lunii.QT remplit le nœud de `0xAA`) ;
//! - `optionIndex` à `-1` : accepté (converti en `0`) si l'actionNode n'a qu'une option, refusé
//!   sinon tant que son effet sur la Flam n'est pas mesuré (L11) ;
//! - nœud `squareOne` placé en tête (Lunii.QT prend toujours le premier nœud) ;
//! - toute référence inconnue refusée (Lunii.QT écrit `-1` dans `li` ou lève une exception).
//!
//! # Garde-fous contre une archive hostile
//!
//! Toute incohérence est une [`StoryError`], avant toute production d'octets :
//!
//! - bornes du § 6.2 de la note : taille du JSON ([`MAX_STORY_JSON_BYTES`]), nœuds
//!   ([`MAX_STAGE_NODES`], [`MAX_ACTION_NODES`]), entrées de `li` ([`MAX_OPTIONS`]), images et
//!   sons distincts ([`MAX_ASSETS`] chacun) ; les tableaux `stageNodes`, `actionNodes` et
//!   `options` sont lus avec leur plafond (éléments excédentaires comptés, jamais alloués) ;
//!   numéros de nœud des erreurs comptés depuis 1 ; profondeur d'imbrication bornée par `serde_json`
//!   (128) ; clé en double refusée pour les champs lus ;
//! - plusieurs `squareOne` ;
//! - UUID de stageNode uniques (comparés sans la casse), identifiants d'actionNode uniques ;
//! - transition vers un actionNode inexistant, option vers un stageNode inexistant (comparaison
//!   exacte des chaînes, comme Lunii.QT), actionNode sans option ;
//! - `optionIndex` hors de `0..options` (`-1` toléré avec une seule option), `controlSettings`
//!   absent ;
//! - `format` autre que `"v1"`, `version` hors des 16 bits de son champ dans `ni` ;
//! - noms d'assets dangereux : blanc, plus de [`MAX_ASSET_NAME_BYTES`] octets, `..`, `/`, `\`,
//!   `:`, caractère de contrôle ; un même nom employé comme image et comme son (Lunii.QT n'en
//!   copierait que l'image). Aucun nom d'asset ne devient un chemin de sortie : les noms du
//!   pack sont produits par [`crate::index`].
//!
//! Les cycles du graphe sont permis : STUdio en produit pour tout retour au menu ou à l'accueil,
//! et l'appareil les suit à l'exécution. Ni Lunii.QT ni ce module ne parcourent le graphe : les
//! références sont résolues en indices par tables de hachage, en une passe, donc sans risque de
//! boucle infinie ni de récursion.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::marker::PhantomData;

use serde::de::{IgnoredAny, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

/// Taille maximale acceptée pour `story.json` (le launcher borne déjà la lecture).
///
/// Les tableaux sont lus avec un plafond (voir [`MAX_STAGE_NODES`], [`MAX_ACTION_NODES`],
/// [`MAX_OPTIONS`]) : au-delà, les éléments sont comptés sans être alloués, donc la mémoire
/// transitoire reste de l'ordre de quelques fois la taille du JSON (chaînes copiées, au plus
/// 20 001 stageNodes, 20 001 actionNodes et 100 001 options conservés), et non plus de dizaines
/// de fois. Hypothèse : une grande histoire STUdio réelle fait quelques centaines de Kio, 16 Mio
/// laissent une très large marge ; ce plafond n'est donc pas abaissé.
pub const MAX_STORY_JSON_BYTES: usize = 16 * 1024 * 1024;
/// Nombre maximal de `stageNodes`.
pub const MAX_STAGE_NODES: usize = 20_000;
/// Nombre maximal d'`actionNodes`.
pub const MAX_ACTION_NODES: usize = 20_000;
/// Nombre maximal d'options, tous `actionNodes` confondus (entrées de `li`).
pub const MAX_OPTIONS: usize = 100_000;
/// Nombre maximal d'images distinctes, et de sons distincts.
pub const MAX_ASSETS: usize = 10_000;
/// Longueur maximale d'un nom d'asset, en octets.
pub const MAX_ASSET_NAME_BYTES: usize = 255;

/// Erreur de lecture ou de validation de `story.json`.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StoryError {
    #[error("story.json trop volumineux : {taille} octets (maximum {max})")]
    TropVolumineux { taille: usize, max: usize },
    #[error("story.json n'est pas un JSON STUdio valide : {0}")]
    Json(String),
    #[error("format STUdio {0:?} non pris en charge (seul « v1 » l'est)")]
    Format(String),
    #[error("champ « version » hors bornes : {0} (attendu 0 à 65535)")]
    Version(i64),
    #[error("l'histoire ne contient aucun stageNode")]
    AucunStageNode,
    #[error("{quoi} : {nombre}, au-delà du maximum de {max}")]
    TropDeNoeuds {
        quoi: &'static str,
        nombre: usize,
        max: usize,
    },
    /// Numéros de stageNode comptés depuis 1.
    #[error("plusieurs stageNodes portent squareOne (n° {premier} et n° {second})")]
    PlusieursSquareOne { premier: usize, second: usize },
    #[error("stageNode {uuid} : controlSettings absent")]
    ControlesAbsents { uuid: String },
    #[error("{quoi} : plus que le maximum de {max}")]
    TropDAssets { quoi: &'static str, max: usize },
    #[error("stageNode n° {noeud} : UUID invalide {uuid:?}")]
    UuidInvalide { noeud: usize, uuid: String },
    #[error("stageNode n° {noeud} : UUID {uuid} déjà employé par un autre stageNode")]
    UuidDuplique { noeud: usize, uuid: String },
    #[error("actionNode n° {action} : identifiant {id:?} déjà employé")]
    ActionDupliquee { action: usize, id: String },
    #[error("actionNode {id:?} sans option")]
    ActionSansOption { id: String },
    #[error("actionNode {id:?} : l'option {uuid:?} ne désigne aucun stageNode")]
    OptionInconnue { id: String, uuid: String },
    #[error(
        "stageNode {uuid} : la transition {transition} vise l'actionNode inexistant {action:?}"
    )]
    ActionInconnue {
        uuid: String,
        transition: &'static str,
        action: String,
    },
    #[error(
        "stageNode {uuid} : la transition {transition} a un optionIndex {index} hors bornes \
         (actionNode à {options} option(s))"
    )]
    OptionIndexHorsBornes {
        uuid: String,
        transition: &'static str,
        index: i64,
        options: usize,
    },
    #[error(
        "stageNode {uuid} : la transition {transition} demande un choix au hasard parmi {options} options (optionIndex -1) ; ce n'est pas encore pris en charge sur la Flam"
    )]
    OptionIndexAleatoire {
        uuid: String,
        transition: &'static str,
        options: usize,
    },
    #[error("stageNode {uuid} : nom d'asset refusé {nom:?} ({raison})")]
    NomAsset {
        uuid: String,
        nom: String,
        raison: &'static str,
    },
    #[error("l'asset {nom:?} sert à la fois d'image et de son")]
    AssetImageEtSon { nom: String },
}

/// Histoire STUdio validée ; ne se construit que par [`parse_story`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Story {
    uuid: String,
    title: String,
    description: String,
    format_version: u16,
    pack_version: u16,
    night_mode: bool,
    stage_nodes: Vec<StageNode>,
    action_nodes: Vec<ActionNode>,
}

/// Nœud de scène : une image et un son facultatifs, deux transitions, des contrôles.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageNode {
    /// UUID tel qu'écrit dans `story.json`.
    pub uuid: String,
    /// Nom de l'image dans `assets/`, `None` si absente ou vide.
    pub image: Option<String>,
    /// Nom du son dans `assets/`, `None` si absent ou vide.
    pub audio: Option<String>,
    /// Transition sur OK (ou en fin de son en lecture automatique).
    pub ok_transition: Option<Transition>,
    /// Transition sur le bouton maison.
    pub home_transition: Option<Transition>,
    /// Contrôles (`controlSettings`, obligatoire).
    pub controls: Controls,
}

/// Transition vers une option d'un actionNode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    /// Indice de l'actionNode dans [`Story::action_nodes`].
    pub action_node: usize,
    /// Option choisie, toujours `< options.len()`.
    pub option_index: u32,
}

/// Nœud d'action : liste ordonnée de stageNodes proposés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionNode {
    /// Identifiant tel qu'écrit dans `story.json`.
    pub id: String,
    /// Indices des stageNodes dans [`Story::stage_nodes`] (au moins un).
    pub options: Vec<usize>,
}

/// Contrôles d'un stageNode (`controlSettings` de STUdio).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Controls {
    pub wheel: bool,
    pub ok: bool,
    pub home: bool,
    pub pause: bool,
    pub autoplay: bool,
}

impl Story {
    /// UUID de l'histoire (celui du nœud d'entrée), forme canonique en minuscules.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }
    /// Titre (vide si absent).
    pub fn title(&self) -> &str {
        &self.title
    }
    /// Description (vide si absente).
    pub fn description(&self) -> &str {
        &self.description
    }
    /// Version de format : toujours 1 (`"v1"`).
    pub fn format_version(&self) -> u16 {
        self.format_version
    }
    /// Version du pack (`version`).
    pub fn pack_version(&self) -> u16 {
        self.pack_version
    }
    /// `nightModeAvailable` : le pack doit alors contenir un fichier `nm` vide.
    pub fn night_mode(&self) -> bool {
        self.night_mode
    }
    /// stageNodes, dans l'ordre de `story.json`, le nœud d'entrée en tête.
    pub fn stage_nodes(&self) -> &[StageNode] {
        &self.stage_nodes
    }
    /// actionNodes, dans l'ordre de `story.json`.
    pub fn action_nodes(&self) -> &[ActionNode] {
        &self.action_nodes
    }
}

// --- Modèle brut désérialisé -----------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStory {
    format: String,
    version: i64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    night_mode_available: Option<bool>,
    stage_nodes: Capped<RawStage, MAX_STAGE_NODES>,
    action_nodes: Capped<RawAction, MAX_ACTION_NODES>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawStage {
    uuid: String,
    #[serde(default)]
    image: Option<String>,
    #[serde(default)]
    audio: Option<String>,
    #[serde(default)]
    ok_transition: Option<RawTransition>,
    #[serde(default)]
    home_transition: Option<RawTransition>,
    #[serde(default)]
    control_settings: Option<Controls>,
    #[serde(default)]
    square_one: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawTransition {
    action_node: String,
    option_index: i64,
}

#[derive(Deserialize)]
struct RawAction {
    id: String,
    options: Options,
}

/// Tableau JSON dont on ne garde que les `MAX + 1` premiers éléments : la mémoire reste bornée
/// quelle que soit la taille du tableau, le reste est parcouru sans rien allouer
/// ([`IgnoredAny`]) et seulement compté. Un dépassement est signalé ensuite, avec le nombre exact
/// d'éléments, par [`check_count`].
struct Capped<T, const MAX: usize> {
    items: Vec<T>,
    total: usize,
}

impl<'de, T: Deserialize<'de>, const MAX: usize> Deserialize<'de> for Capped<T, MAX> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T, const MAX: usize>(PhantomData<T>);
        impl<'de, T: Deserialize<'de>, const MAX: usize> Visitor<'de> for V<T, MAX> {
            type Value = Capped<T, MAX>;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("un tableau")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
                let mut items = Vec::new();
                let mut total = 0usize;
                while total <= MAX {
                    match seq.next_element::<T>()? {
                        Some(x) => items.push(x),
                        None => return Ok(Capped { items, total }),
                    }
                    total += 1;
                }
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    total += 1;
                }
                Ok(Capped { items, total })
            }
        }
        d.deserialize_seq(V::<T, MAX>(PhantomData))
    }
}

thread_local! {
    /// Options d'actionNodes vues pendant la lecture en cours (voir [`Options`]).
    static OPTIONS_SEEN: Cell<usize> = const { Cell::new(0) };
}

/// `options[]` d'un actionNode : le plafond [`MAX_OPTIONS`] vaut pour **tous** les actionNodes
/// ensemble, d'où un compteur partagé (remis à zéro par [`parse_story`]) ; au-delà, les options
/// sont comptées mais plus gardées.
struct Options {
    items: Vec<String>,
}

impl<'de> Deserialize<'de> for Options {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Options;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("un tableau de chaînes")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Options, A::Error> {
                let mut items = Vec::new();
                while OPTIONS_SEEN.get() <= MAX_OPTIONS {
                    match seq.next_element::<String>()? {
                        Some(x) => items.push(x),
                        None => return Ok(Options { items }),
                    }
                    OPTIONS_SEEN.set(OPTIONS_SEEN.get() + 1);
                }
                while seq.next_element::<IgnoredAny>()?.is_some() {
                    OPTIONS_SEEN.set(OPTIONS_SEEN.get() + 1);
                }
                Ok(Options { items })
            }
        }
        d.deserialize_seq(V)
    }
}

// --- Lecture ---------------------------------------------------------------------------------

/// Lit et valide `story.json` à partir de ses octets (UTF-8, BOM toléré).
pub fn parse_story(bytes: &[u8]) -> Result<Story, StoryError> {
    if bytes.len() > MAX_STORY_JSON_BYTES {
        return Err(StoryError::TropVolumineux {
            taille: bytes.len(),
            max: MAX_STORY_JSON_BYTES,
        });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    OPTIONS_SEEN.set(0);
    let parsed = serde_json::from_slice::<RawStory>(bytes);
    let options_seen = OPTIONS_SEEN.replace(0);
    let mut raw = parsed.map_err(|e| StoryError::Json(e.to_string()))?;

    let format_version = parse_format(&raw.format)?;
    let pack_version = u16::try_from(raw.version).map_err(|_| StoryError::Version(raw.version))?;

    if raw.stage_nodes.total == 0 {
        return Err(StoryError::AucunStageNode);
    }
    check_count("stageNodes", raw.stage_nodes.total, MAX_STAGE_NODES)?;
    check_count("actionNodes", raw.action_nodes.total, MAX_ACTION_NODES)?;
    check_count("options d'actionNodes", options_seen, MAX_OPTIONS)?;

    // UUID des stageNodes : forme canonique, uniques sans la casse ; les options sont résolues
    // par égalité exacte des chaînes, comme Lunii.QT.
    let mut seen_uuid = HashSet::with_capacity(raw.stage_nodes.items.len());
    for (i, s) in raw.stage_nodes.items.iter().enumerate() {
        if !is_canonical_uuid(&s.uuid) {
            return Err(StoryError::UuidInvalide {
                noeud: i + 1,
                uuid: s.uuid.clone(),
            });
        }
        if !seen_uuid.insert(s.uuid.to_ascii_lowercase()) {
            return Err(StoryError::UuidDuplique {
                noeud: i + 1,
                uuid: s.uuid.clone(),
            });
        }
    }

    // Nœud d'entrée : squareOne unique, déplacé en tête.
    let mut square_one = None;
    for (i, s) in raw.stage_nodes.items.iter().enumerate() {
        if s.square_one == Some(true) {
            if let Some(premier) = square_one {
                return Err(StoryError::PlusieursSquareOne {
                    premier: premier + 1,
                    second: i + 1,
                });
            }
            square_one = Some(i);
        }
    }
    if let Some(k) = square_one.filter(|&k| k > 0) {
        let entry = raw.stage_nodes.items.remove(k);
        raw.stage_nodes.items.insert(0, entry);
    }
    let mut stage_by_uuid: HashMap<&str, usize> =
        HashMap::with_capacity(raw.stage_nodes.items.len());
    for (i, s) in raw.stage_nodes.items.iter().enumerate() {
        stage_by_uuid.insert(&s.uuid, i);
    }

    let mut action_by_id: HashMap<&str, usize> =
        HashMap::with_capacity(raw.action_nodes.items.len());
    let mut action_nodes = Vec::with_capacity(raw.action_nodes.items.len());
    for (i, a) in raw.action_nodes.items.iter().enumerate() {
        if action_by_id.insert(&a.id, i).is_some() {
            return Err(StoryError::ActionDupliquee {
                action: i + 1,
                id: a.id.clone(),
            });
        }
        if a.options.items.is_empty() {
            return Err(StoryError::ActionSansOption { id: a.id.clone() });
        }
        let options =
            a.options
                .items
                .iter()
                .map(|o| {
                    stage_by_uuid.get(o.as_str()).copied().ok_or_else(|| {
                        StoryError::OptionInconnue {
                            id: a.id.clone(),
                            uuid: o.clone(),
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
        action_nodes.push(ActionNode {
            id: a.id.clone(),
            options,
        });
    }

    let mut image_names: HashSet<&str> = HashSet::new();
    let mut audio_names: HashSet<&str> = HashSet::new();
    let mut stage_nodes = Vec::with_capacity(raw.stage_nodes.items.len());
    for s in &raw.stage_nodes.items {
        let image = asset_name(&s.uuid, s.image.as_deref())?;
        let audio = asset_name(&s.uuid, s.audio.as_deref())?;
        if let Some(n) = image {
            image_names.insert(n);
        }
        if let Some(n) = audio {
            audio_names.insert(n);
        }
        let controls = s
            .control_settings
            .ok_or_else(|| StoryError::ControlesAbsents {
                uuid: s.uuid.clone(),
            })?;
        let ok_transition = transition(
            &s.uuid,
            "okTransition",
            s.ok_transition.as_ref(),
            &action_by_id,
            &action_nodes,
        )?;
        let home_transition = transition(
            &s.uuid,
            "homeTransition",
            s.home_transition.as_ref(),
            &action_by_id,
            &action_nodes,
        )?;
        stage_nodes.push(StageNode {
            uuid: s.uuid.clone(),
            image: image.map(str::to_owned),
            audio: audio.map(str::to_owned),
            ok_transition,
            home_transition,
            controls,
        });
    }
    for (quoi, names) in [
        ("images distinctes", &image_names),
        ("sons distincts", &audio_names),
    ] {
        if names.len() > MAX_ASSETS {
            return Err(StoryError::TropDAssets {
                quoi,
                max: MAX_ASSETS,
            });
        }
    }
    // Parcours dans l'ordre des stageNodes : message d'erreur stable.
    if let Some(n) = stage_nodes
        .iter()
        .filter_map(|s| s.image.as_deref())
        .find(|n| audio_names.contains(n))
    {
        return Err(StoryError::AssetImageEtSon { nom: n.to_owned() });
    }

    Ok(Story {
        uuid: stage_nodes[0].uuid.to_ascii_lowercase(),
        title: raw.title.unwrap_or_default(),
        description: raw.description.unwrap_or_default(),
        format_version,
        pack_version,
        night_mode: raw.night_mode_available.unwrap_or(false),
        stage_nodes,
        action_nodes,
    })
}

fn check_count(quoi: &'static str, nombre: usize, max: usize) -> Result<(), StoryError> {
    if nombre > max {
        return Err(StoryError::TropDeNoeuds { quoi, nombre, max });
    }
    Ok(())
}

/// Seul `"v1"` est pris en charge (note § 2.4) ; Lunii.QT écrit `int(format[1:])` dans `ni`.
fn parse_format(format: &str) -> Result<u16, StoryError> {
    if format == "v1" {
        Ok(1)
    } else {
        Err(StoryError::Format(format.to_owned()))
    }
}

/// UUID `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx`, chiffres hexadécimaux de casse quelconque.
fn is_canonical_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, &c)| match i {
            8 | 13 | 18 | 23 => c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Nom d'asset : `None` si absent ou vide (comme `if image:` de Lunii.QT), sinon contrôlé.
fn asset_name<'a>(uuid: &str, name: Option<&'a str>) -> Result<Option<&'a str>, StoryError> {
    let Some(name) = name.filter(|n| !n.is_empty()) else {
        return Ok(None);
    };
    let refuse = |raison| {
        Err(StoryError::NomAsset {
            uuid: uuid.to_owned(),
            nom: name.to_owned(),
            raison,
        })
    };
    if name.trim().is_empty() {
        return refuse("nom blanc");
    }
    if name.len() > MAX_ASSET_NAME_BYTES {
        return refuse("nom trop long");
    }
    if name.contains("..") {
        return refuse("« .. » interdit");
    }
    if name.contains(['/', '\\', ':']) {
        return refuse("séparateur de chemin ou « : » interdit");
    }
    if name.chars().any(char::is_control) {
        return refuse("caractère de contrôle");
    }
    Ok(Some(name))
}

fn transition(
    uuid: &str,
    which: &'static str,
    raw: Option<&RawTransition>,
    action_by_id: &HashMap<&str, usize>,
    action_nodes: &[ActionNode],
) -> Result<Option<Transition>, StoryError> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let action =
        *action_by_id
            .get(raw.action_node.as_str())
            .ok_or_else(|| StoryError::ActionInconnue {
                uuid: uuid.to_owned(),
                transition: which,
                action: raw.action_node.clone(),
            })?;
    let options = action_nodes[action].options.len();
    let out_of_range = || StoryError::OptionIndexHorsBornes {
        uuid: uuid.to_owned(),
        transition: which,
        index: raw.option_index,
        options,
    };
    // -1 est le port « Random option » de l'éditeur STUdio : writer.js l'écrit quand la
    // transition vise ce port, reader.js le relit comme ce port, et le simulateur
    // (ActionNodeModel.js) tire alors l'option au hasard, uniformément. Établi pour STUdio ;
    // le comportement de la Flam devant un -1 dans `ni` reste inconnu (L11).
    // Avec une seule option, il ne peut désigner que
    // l'option 0, quel que soit le comportement de la Flam : on le convertit en 0. On n'écrit
    // jamais -1 dans `ni` pour une transition présente (-1 y signifie « sans transition »).
    // Avec plusieurs options, son effet sur la Flam n'est pas mesuré : refusé (note, Q5 et L11).
    if raw.option_index == -1 {
        if options == 1 {
            return Ok(Some(Transition {
                action_node: action,
                option_index: 0,
            }));
        }
        return Err(StoryError::OptionIndexAleatoire {
            uuid: uuid.to_owned(),
            transition: which,
            options,
        });
    }
    let option_index = u32::try_from(raw.option_index)
        .ok()
        .filter(|&i| (i as usize) < options)
        .ok_or_else(out_of_range)?;
    Ok(Some(Transition {
        action_node: action,
        option_index,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_v1_seulement() {
        assert_eq!(parse_format("v1"), Ok(1));
        for bad in ["1", "v", "v2", "v01", "V1", "v1.0", "v 1", " v1", "v١"] {
            assert!(parse_format(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn uuid_canonique() {
        assert!(is_canonical_uuid("0123abcd-0000-4000-8000-ABCDEF012345"));
        assert!(!is_canonical_uuid("0123abcd00004000800-0ABCDEF012345"));
        assert!(!is_canonical_uuid("{0123abcd-0000-4000-8000-abcdef01234}"));
        assert!(!is_canonical_uuid("0123abcg-0000-4000-8000-abcdef012345"));
    }
}
