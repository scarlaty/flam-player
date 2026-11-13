//! Index binaires d'un pack Lunii (`ni`, `li`, `ri`, `si`) et noms cibles des assets, produits à
//! partir d'une [`Story`] validée.
//!
//! Références : note de conception `docs/studio-lunii.md` (§ 3.2 à 3.4, § 3.7, Q3) et Lunii.QT
//! (`StudioStory.get_*_data`, `FlamDevice.__get_lunii_ciphered_name`, commit c8afe43), dont les
//! octets sont reproduits à l'identique quand la règle de nommage de Lunii.QT est sûre.
//!
//! Tous les entiers sont en petit-boutiste. Les octets rendus sont **en clair** : le chiffrement
//! est fait par le launcher (#523), voir [Chiffrement](#chiffrement).
//!
//! # Assets : ordre et noms
//!
//! Les images (resp. sons) sont numérotées dans l'ordre de leur **première** apparition en
//! parcourant les stageNodes dans l'ordre (nœud d'entrée en tête) ; un asset partagé par
//! plusieurs nœuds n'a qu'un numéro. Le nom cible `XXXXXXXX` (8 caractères, sans extension) est
//! choisi pour **tout le pack** par l'une de deux règles ([`Naming`]) :
//!
//! 1. **Règle de Lunii.QT** ([`Naming::LuniiQt`]) : les 8 derniers caractères du nom sans son
//!    extension (`splitext` de Python : dernier `.`, sauf s'il n'y a que des points avant), en
//!    majuscules. Appliquée seulement si, pour **chaque** asset, ces 8 caractères existent et
//!    sont des lettres ou chiffres ASCII, et si deux images (resp. deux sons) n'aboutissent
//!    jamais au même nom.
//! 2. **Rang** ([`Naming::Rank`]) sinon : `00000000`, `00000001`… (numéro en 8 chiffres
//!    hexadécimaux majuscules) dans l'ordre de `ri` et, séparément, de `si`.
//!
//! | Nom dans `story.json`               | Règle de Lunii.QT | Entrée `ri`/`si` | Fichier du pack     |
//! |-------------------------------------|-------------------|------------------|---------------------|
//! | `0123…cdef00000001.png` (image)     | `00000001`        | `000\00000001`   | `rf/000/00000001`   |
//! | `son-de-Couverture.Mp3` (son)       | `UVERTURE`        | `000\UVERTURE`   | `sf/000/UVERTURE`   |
//! | `Couverture.Finale.AbCdEf12.PNG`    | `ABCDEF12`        | `000\ABCDEF12`   | `rf/000/ABCDEF12`   |
//!
//! Lunii.QT met le chemin en minuscules puis le nom de base en majuscules s'il fait au moins 8
//! caractères : avec 8 caractères exactement, le répertoire est toujours `rf/000/` ou `sf/000/`
//! et le nom en majuscules. Dans le pack, l'image est le BMP de [`crate::bitmap`] et le son le
//! MP3 de [`crate::audio`]. Aucun nom de l'archive ne devient un chemin de sortie.
//!
//! Pourquoi la seconde règle : Lunii.QT, avec un nom de moins de 8 caractères, écrit une entrée
//! `ri`/`si` de moins de 12 octets qui décale les suivantes, et, avec deux noms de même fin,
//! écrase le premier fichier par le second.
//!
//! # `ni` : en-tête puis un enregistrement par stageNode
//!
//! En-tête de [`NI_HEADER_SIZE`] (0x200) octets :
//!
//! | Position | Taille | Contenu                                                   |
//! |----------|--------|-----------------------------------------------------------|
//! | 0x00     | 2      | version de format : 1 (`"v1"`)                           |
//! | 0x02     | 2      | version du pack (`version`)                               |
//! | 0x04     | 4      | position des nœuds : 0x200                                |
//! | 0x08     | 4      | taille d'un nœud : 0x2C                                   |
//! | 0x0C     | 4      | nombre de stageNodes                                      |
//! | 0x10     | 4      | nombre d'images (entrées de `ri`)                         |
//! | 0x14     | 4      | nombre de sons (entrées de `si`)                          |
//! | 0x18     | 1      | 1 (constante de Lunii.QT, sens inconnu)                   |
//! | 0x19     | 0x1E7  | zéros                                                     |
//!
//! Puis, pour chaque stageNode dans l'ordre, [`NODE_SIZE`] (0x2C) octets à
//! `0x200 + n × 0x2C` :
//!
//! | Position | Taille | Contenu                                                           |
//! |----------|--------|-------------------------------------------------------------------|
//! | 0x00     | 4 (s)  | numéro de l'image dans `ri`, -1 si aucune                         |
//! | 0x04     | 4 (s)  | numéro du son dans `si`, -1 si aucun                              |
//! | 0x08     | 4 (s)  | OK : position dans `li` de la 1re option de l'actionNode visé     |
//! | 0x0C     | 4 (s)  | OK : nombre d'options de cet actionNode                           |
//! | 0x10     | 4 (s)  | OK : `optionIndex`, ou -1 pour un tirage au hasard                |
//! | 0x14     | 12     | maison : mêmes trois champs                                       |
//! | 0x20     | 2 × 5  | contrôles `wheel`, `ok`, `home`, `pause`, `autoplay` (0 ou 1)     |
//! | 0x2A     | 2      | zéros                                                             |
//!
//! Sans transition, ses trois champs valent `FF FF FF FF`. Un tirage au hasard
//! ([`OptionChoice::Aleatoire`], actionNode d'au moins deux options) ne met `FF FF FF FF` que
//! dans le champ d'option, les deux autres restant renseignés : c'est l'encodage de Lunii.QT
//! (`optionIndex` recopié en entier signé de 32 bits). Son effet sur la Flam 2.x n'est pas
//! mesuré (L11 de la note). `controlSettings` étant obligatoire
//! ([`crate::story`]), le remplissage `0xAA` de Lunii.QT n'apparaît jamais.
//!
//! # `li` : options des actionNodes
//!
//! Pour chaque actionNode dans l'ordre, pour chaque option dans l'ordre : numéro du stageNode
//! (4 octets signés). La position d'un actionNode dans `li` (champ 0x08 de `ni`) est la somme
//! des nombres d'options des actionNodes qui le précèdent. Si le tout fait moins de 8 octets,
//! il est complété de zéros jusqu'à 8.
//!
//! # `ri` et `si` : noms des images et des sons
//!
//! Concaténation, dans l'ordre des numéros, de `000\` suivi du nom cible : 12 octets ASCII par
//! entrée, sans séparateur ni terminateur (vide s'il n'y a aucune image ou aucun son).
//!
//! # `nm`
//!
//! Si [`Story::night_mode`] est vrai, le pack contient un fichier `nm` vide
//! ([`PackIndex::night_mode`]).
//!
//! # Chiffrement
//!
//! À faire par le launcher (#523, note § 3.7), comme `__get_ciphered_data` de Lunii.QT :
//!
//! | Fichier                         | Chiffré ?                                             |
//! |---------------------------------|-------------------------------------------------------|
//! | `ni`, `nm`                      | non                                                   |
//! | `li`, `ri`, `si`                | oui : AES-CBC sur les 0x200 premiers octets au plus   |
//! | `rf/000/*`, `sf/000/*`          | oui : AES-CBC sur les 0x200 premiers octets au plus   |
//!
//! Clé et IV : ceux de l'appareil, inversés par mots de 4 octets. Un fichier de moins de 0x200
//! octets est chiffré en entier, après bourrage de zéros jusqu'au multiple de 16 suivant (sa
//! taille change) ; un fichier vide (`ri` ou `si` sans entrée) reste vide. Le choix se fait par
//! nature de fichier, jamais par suffixe de nom (Lunii.QT teste `endswith("ni")` sur le chemin
//! source, ce qui laisserait en clair un asset dont le nom finirait par `ni`).

use std::collections::HashMap;

use thiserror::Error;

use crate::story::{OptionChoice, StageNode, Story, Transition};

/// Taille de l'en-tête de `ni`, qui est aussi la position du premier nœud.
pub const NI_HEADER_SIZE: usize = 0x200;
/// Taille d'un enregistrement de stageNode dans `ni`.
pub const NODE_SIZE: usize = 0x2C;
/// Longueur d'un nom cible.
pub const ASSET_NAME_LEN: usize = 8;
/// Préfixe de chaque entrée de `ri`/`si`.
const ENTRY_PREFIX: &[u8] = b"000\\";

/// Raison pour laquelle la règle de nommage de Lunii.QT n'est pas sûre.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum IndexError {
    #[error("asset {nom:?} : nom sans extension de moins de {ASSET_NAME_LEN} caractères")]
    NomCourt { nom: String },
    #[error(
        "asset {nom:?} : les {ASSET_NAME_LEN} derniers caractères du nom sans extension ne sont \
         pas tous des lettres ou chiffres ASCII"
    )]
    NomInvalide { nom: String },
    #[error("assets {premier:?} et {second:?} : même nom cible {cible}")]
    Collision {
        premier: String,
        second: String,
        cible: String,
    },
}

/// Règle de nommage retenue pour le pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Naming {
    /// Règle de Lunii.QT (8 derniers caractères, majuscules) : octets identiques à Lunii.QT.
    LuniiQt,
    /// Nommage par rang, avec la raison (premier asset fautif) qui a écarté la règle de Lunii.QT.
    Rank(IndexError),
}

/// Nature d'un asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// Image, à convertir en BMP ([`crate::bitmap`]), rangée sous `rf/000/`.
    Image,
    /// Son MP3 ([`crate::audio`]), rangé sous `sf/000/`.
    Audio,
}

impl AssetKind {
    fn dir(self) -> &'static str {
        match self {
            AssetKind::Image => "rf/000/",
            AssetKind::Audio => "sf/000/",
        }
    }
}

/// Correspondance entre un asset de l'archive STUdio et son fichier dans le pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetTarget {
    /// Nature de l'asset.
    pub kind: AssetKind,
    /// Nom tel que dans `story.json`.
    pub name: String,
    /// Chemin dans l'archive : `assets/<name>` (l'asset doit y être, sinon refus).
    pub archive_path: String,
    /// Chemin relatif au répertoire du pack : `rf/000/XXXXXXXX` ou `sf/000/XXXXXXXX`.
    pub pack_path: String,
}

/// Index du pack, en clair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackIndex {
    /// `ni` (jamais chiffré).
    pub ni: Vec<u8>,
    /// `li` (à chiffrer).
    pub li: Vec<u8>,
    /// `ri` (à chiffrer).
    pub ri: Vec<u8>,
    /// `si` (à chiffrer).
    pub si: Vec<u8>,
    /// Images dans l'ordre de `ri`, puis sons dans l'ordre de `si`.
    pub assets: Vec<AssetTarget>,
    /// Règle de nommage retenue.
    pub naming: Naming,
    /// Le pack doit contenir un fichier `nm` vide (jamais chiffré).
    pub night_mode: bool,
}

/// Assets d'une nature, numérotés dans l'ordre de première apparition.
struct Table<'a> {
    kind: AssetKind,
    /// Nom dans `story.json` → numéro.
    number: HashMap<&'a str, u32>,
    /// Noms dans l'ordre des numéros.
    order: Vec<&'a str>,
}

impl<'a> Table<'a> {
    fn new(kind: AssetKind) -> Self {
        Table {
            kind,
            number: HashMap::new(),
            order: Vec::new(),
        }
    }

    fn add(&mut self, name: &'a str) {
        if !self.number.contains_key(name) {
            // Au plus MAX_ASSETS noms distincts (story) : tient dans un u32.
            let n = u32::try_from(self.order.len()).expect("borné par story");
            self.number.insert(name, n);
            self.order.push(name);
        }
    }

    fn number_of(&self, name: Option<&str>) -> i32 {
        name.and_then(|n| self.number.get(n))
            .map_or(-1, |&i| i32::try_from(i).expect("borné par story"))
    }

    /// Noms selon la règle de Lunii.QT, ou la raison qui l'écarte.
    fn lunii_names(&self) -> Result<Vec<String>, IndexError> {
        let mut first: HashMap<String, &str> = HashMap::with_capacity(self.order.len());
        let mut names = Vec::with_capacity(self.order.len());
        for &name in &self.order {
            let norm = lunii_asset_name(name)?;
            if let Some(premier) = first.insert(norm.clone(), name) {
                return Err(IndexError::Collision {
                    premier: premier.to_owned(),
                    second: name.to_owned(),
                    cible: format!("{}{norm}", self.kind.dir()),
                });
            }
            names.push(norm);
        }
        Ok(names)
    }

    fn rank_names(&self) -> Vec<String> {
        (0..self.order.len()).map(|i| format!("{i:08X}")).collect()
    }

    fn index_bytes(names: &[String]) -> Vec<u8> {
        let mut out = Vec::with_capacity(names.len() * (ENTRY_PREFIX.len() + ASSET_NAME_LEN));
        for n in names {
            out.extend_from_slice(ENTRY_PREFIX);
            out.extend_from_slice(n.as_bytes());
        }
        out
    }

    fn targets<'t>(&'t self, names: &'t [String]) -> impl Iterator<Item = AssetTarget> + 't {
        self.order
            .iter()
            .zip(names)
            .map(|(name, norm)| AssetTarget {
                kind: self.kind,
                name: (*name).to_owned(),
                archive_path: format!("assets/{name}"),
                pack_path: format!("{}{norm}", self.kind.dir()),
            })
    }
}

/// Nom cible d'un asset selon la règle de Lunii.QT : les 8 derniers caractères du nom sans
/// extension, en majuscules ; erreur si la règle n'est pas sûre pour ce nom.
pub fn lunii_asset_name(name: &str) -> Result<String, IndexError> {
    let stem = strip_extension(name);
    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < ASSET_NAME_LEN {
        return Err(IndexError::NomCourt {
            nom: name.to_owned(),
        });
    }
    let tail = &chars[chars.len() - ASSET_NAME_LEN..];
    if !tail.iter().all(char::is_ascii_alphanumeric) {
        return Err(IndexError::NomInvalide {
            nom: name.to_owned(),
        });
    }
    Ok(tail.iter().map(char::to_ascii_uppercase).collect())
}

/// `os.path.splitext(name)[0]` pour un nom sans séparateur : coupe au dernier `.`, sauf si ce
/// qui le précède n'est fait que de points (fichier « caché » sans extension).
fn strip_extension(name: &str) -> &str {
    match name.rfind('.') {
        Some(dot) if name[..dot].bytes().any(|b| b != b'.') => &name[..dot],
        _ => name,
    }
}

/// Produit `ni`, `li`, `ri`, `si` et la table des assets d'une histoire validée.
pub fn build_index(story: &Story) -> PackIndex {
    let nodes = story.stage_nodes();
    let mut images = Table::new(AssetKind::Image);
    let mut sounds = Table::new(AssetKind::Audio);
    for node in nodes {
        if let Some(n) = node.image.as_deref() {
            images.add(n);
        }
        if let Some(n) = node.audio.as_deref() {
            sounds.add(n);
        }
    }
    let (image_names, sound_names, naming) = match images
        .lunii_names()
        .and_then(|i| Ok((i, sounds.lunii_names()?)))
    {
        Ok((i, s)) => (i, s, Naming::LuniiQt),
        Err(raison) => (
            images.rank_names(),
            sounds.rank_names(),
            Naming::Rank(raison),
        ),
    };

    // Position de chaque actionNode dans li, puis li lui-même.
    let actions = story.action_nodes();
    let mut li_start = Vec::with_capacity(actions.len());
    let mut li = Vec::new();
    for action in actions {
        li_start.push(li.len() / 4);
        for &option in &action.options {
            put_i32(&mut li, to_i32(option));
        }
    }
    if li.len() < 8 {
        li.resize(8, 0);
    }

    let mut ni = Vec::with_capacity(NI_HEADER_SIZE + nodes.len() * NODE_SIZE);
    put_u16(&mut ni, story.format_version());
    put_u16(&mut ni, story.pack_version());
    put_u32(&mut ni, NI_HEADER_SIZE as u32);
    put_u32(&mut ni, NODE_SIZE as u32);
    put_u32(&mut ni, to_u32(nodes.len()));
    put_u32(&mut ni, to_u32(images.order.len()));
    put_u32(&mut ni, to_u32(sounds.order.len()));
    ni.push(1);
    ni.resize(NI_HEADER_SIZE, 0);
    for node in nodes {
        put_node(&mut ni, node, story, &li_start, &images, &sounds);
    }

    let assets = images
        .targets(&image_names)
        .chain(sounds.targets(&sound_names))
        .collect();
    PackIndex {
        ni,
        li,
        ri: Table::index_bytes(&image_names),
        si: Table::index_bytes(&sound_names),
        assets,
        naming,
        night_mode: story.night_mode(),
    }
}

fn put_node(
    ni: &mut Vec<u8>,
    node: &StageNode,
    story: &Story,
    li_start: &[usize],
    images: &Table<'_>,
    sounds: &Table<'_>,
) {
    let start = ni.len();
    put_i32(ni, images.number_of(node.image.as_deref()));
    put_i32(ni, sounds.number_of(node.audio.as_deref()));
    for t in [node.ok_transition, node.home_transition] {
        put_transition(ni, t, story, li_start);
    }
    let c = node.controls;
    for flag in [c.wheel, c.ok, c.home, c.pause, c.autoplay] {
        put_u16(ni, u16::from(flag));
    }
    put_u16(ni, 0);
    debug_assert_eq!(ni.len() - start, NODE_SIZE);
}

fn put_transition(ni: &mut Vec<u8>, t: Option<Transition>, story: &Story, li_start: &[usize]) {
    match t {
        Some(t) => {
            put_i32(ni, to_i32(li_start[t.action_node]));
            put_i32(
                ni,
                to_i32(story.action_nodes()[t.action_node].options.len()),
            );
            put_i32(ni, option_value(t.option));
        }
        None => ni.extend_from_slice(&[0xFF; 12]),
    }
}

/// Champ « optionIndex » d'une transition présente : l'indice, ou `-1` pour un tirage au hasard
/// (valeur de STUdio recopiée par Lunii.QT, entier signé de 32 bits).
fn option_value(option: OptionChoice) -> i32 {
    match option {
        OptionChoice::Index(i) => i32::try_from(i).expect("borné par story"),
        OptionChoice::Aleatoire => -1,
    }
}

// Les nombres viennent d'une Story bornée (MAX_STAGE_NODES, MAX_OPTIONS) : la conversion ne
// peut pas échouer.
fn to_i32(n: usize) -> i32 {
    i32::try_from(n).expect("borné par story")
}

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).expect("borné par story")
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_comme_splitext() {
        assert_eq!(strip_extension("abc.png"), "abc");
        assert_eq!(strip_extension("a.b.c"), "a.b");
        assert_eq!(strip_extension(".cache"), ".cache");
        assert_eq!(strip_extension("..x"), "..x");
        assert_eq!(strip_extension("sans"), "sans");
        assert_eq!(strip_extension("fin."), "fin");
    }

    #[test]
    fn nom_lunii_qt() {
        assert_eq!(lunii_asset_name("abcdefgh.mp3").unwrap(), "ABCDEFGH");
        assert_eq!(
            lunii_asset_name("prefixe-ab0d1234.Png").unwrap(),
            "AB0D1234"
        );
        assert!(matches!(
            lunii_asset_name("court.png"),
            Err(IndexError::NomCourt { .. })
        ));
        for bad in [
            "abcdéfgh.png",
            "abcd efgh.png",
            "abc_efgh.png",
            "abc-efgh.png",
        ] {
            assert!(
                matches!(lunii_asset_name(bad), Err(IndexError::NomInvalide { .. })),
                "{bad}"
            );
        }
    }
}
