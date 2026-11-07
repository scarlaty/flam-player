//! Index binaires d'un pack Lunii (`ni`, `li`, `ri`, `si`) et noms cibles des assets, produits à
//! partir d'une [`Story`] validée, octet pour octet comme Lunii.QT (`StudioStory.get_*_data` et
//! `FlamDevice.__get_lunii_ciphered_name`, commit c8afe43).
//!
//! Tous les entiers sont en petit-boutiste. Les octets rendus sont **en clair** : le chiffrement
//! est fait par le launcher (#523), voir [Chiffrement](#chiffrement).
//!
//! # Assets : ordre, nom normalisé, nom cible
//!
//! Les images (resp. sons) sont numérotées dans l'ordre de leur **première** apparition en
//! parcourant les stageNodes dans l'ordre ; un asset partagé par plusieurs nœuds n'a qu'un
//! numéro. Le nom normalisé est fait des 8 derniers caractères du nom sans son extension
//! (`splitext` de Python : dernier `.`, sauf s'il n'y a que des points avant), passés en
//! majuscules :
//!
//! | Nom dans `story.json`               | Nom normalisé | Entrée `ri`/`si` | Fichier du pack     |
//! |-------------------------------------|---------------|------------------|---------------------|
//! | `0123…cdef00000001.png` (image)     | `00000001`    | `000\00000001`   | `rf/000/00000001`   |
//! | `son-de-Couverture.Mp3` (son)       | `UVERTURE`    | `000\UVERTURE`   | `sf/000/UVERTURE`   |
//! | `Couverture.Finale.AbCdEf12.PNG`    | `ABCDEF12`    | `000\ABCDEF12`   | `rf/000/ABCDEF12`   |
//!
//! Dans le pack, l'image est le BMP produit par [`crate::bitmap`] et le son le MP3 de
//! [`crate::audio`], sans extension. Lunii.QT met le chemin en minuscules puis le nom de base en
//! majuscules s'il fait au moins 8 caractères : comme ce module exige exactement 8 caractères,
//! le répertoire est toujours en minuscules (`rf/000/`, `sf/000/`) et le nom en majuscules.
//!
//! Écarts assumés avec Lunii.QT, qui produirait un pack cassé sans le signaler :
//!
//! - nom sans extension de moins de 8 caractères : refusé ([`IndexError::NomCourt`]) ; Lunii.QT
//!   écrirait une entrée `ri`/`si` de moins de 12 octets, qui décale toutes les suivantes ;
//! - 8 derniers caractères hors `A-Z a-z 0-9 - _` : refusés ([`IndexError::NomInvalide`]) ;
//! - deux assets de même nature au même nom normalisé : refusés ([`IndexError::Collision`]) ;
//!   Lunii.QT écraserait le premier fichier par le second.
//!
//! # `ni` : en-tête puis un enregistrement par stageNode
//!
//! En-tête de [`NI_HEADER_SIZE`] (0x200) octets :
//!
//! | Position | Taille | Contenu                                                   |
//! |----------|--------|-----------------------------------------------------------|
//! | 0x00     | 2      | version de format (`format` sans le `v`)                  |
//! | 0x02     | 2      | version du pack (`version`)                               |
//! | 0x04     | 4      | position des nœuds : 0x200                                |
//! | 0x08     | 4      | taille d'un nœud : 0x2C                                   |
//! | 0x0C     | 4      | nombre de stageNodes                                      |
//! | 0x10     | 4      | nombre d'images (entrées de `ri`)                         |
//! | 0x14     | 4      | nombre de sons (entrées de `si`)                          |
//! | 0x18     | 1      | 1 (constante de Lunii.QT : pack non usine)                |
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
//! | 0x10     | 4 (s)  | OK : `optionIndex` (-1 : au hasard)                               |
//! | 0x14     | 12     | maison : mêmes trois champs                                       |
//! | 0x20     | 2 × 5  | contrôles `wheel`, `ok`, `home`, `pause`, `autoplay` (0 ou 1)     |
//! | 0x2A     | 2      | zéros                                                             |
//!
//! Sans transition, les trois champs valent `FF FF FF FF`. Sans `controlSettings`, les 12
//! octets à partir de 0x20 valent `0xAA` (remplissage de Lunii.QT, conservé tel quel).
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
//! Concaténation, dans l'ordre des numéros, de `000\` suivi du nom normalisé : 12 octets ASCII
//! par entrée, sans séparateur ni terminateur (vide s'il n'y a aucune image ou aucun son).
//!
//! # `nm`
//!
//! Si [`Story::night_mode`] est vrai, Lunii.QT ajoute au pack un fichier `nm` vide
//! ([`PackIndex::night_mode`]).
//!
//! # Chiffrement
//!
//! À faire par le launcher (#523), comme `__get_ciphered_data` de Lunii.QT pour un pack Lunii :
//!
//! | Fichier                         | Chiffré ?                                             |
//! |---------------------------------|-------------------------------------------------------|
//! | `ni`, `nm`                      | non                                                   |
//! | `li`, `ri`, `si`                | oui : AES-CBC sur les 0x200 premiers octets au plus   |
//! | `rf/000/*`, `sf/000/*`          | oui : AES-CBC sur les 0x200 premiers octets au plus   |
//!
//! Clé et IV : ceux de l'histoire, octets inversés. Si la partie chiffrée n'est pas un multiple
//! de 16, Lunii.QT complète le fichier de zéros jusqu'au multiple de 16 suivant (le fichier
//! grossit). Le fichier `key` (autorisation) est aussi à écrire par le launcher.

use std::collections::HashMap;

use thiserror::Error;

use crate::story::{StageNode, Story, Transition};

/// Taille de l'en-tête de `ni`, qui est aussi la position du premier nœud.
pub const NI_HEADER_SIZE: usize = 0x200;
/// Taille d'un enregistrement de stageNode dans `ni`.
pub const NODE_SIZE: usize = 0x2C;
/// Longueur d'un nom normalisé (et du nom de fichier cible).
pub const ASSET_NAME_LEN: usize = 8;
/// Préfixe de chaque entrée de `ri`/`si`.
const ENTRY_PREFIX: &str = "000\\";

/// Erreur de production des index.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum IndexError {
    #[error(
        "asset {nom:?} : nom sans extension de moins de {ASSET_NAME_LEN} caractères, \
         incompatible avec les index du pack"
    )]
    NomCourt { nom: String },
    #[error(
        "asset {nom:?} : les {ASSET_NAME_LEN} derniers caractères du nom doivent être des \
         lettres ou chiffres ASCII, « - » ou « _ »"
    )]
    NomInvalide { nom: String },
    #[error("assets {premier:?} et {second:?} : même fichier cible {cible}")]
    Collision {
        premier: String,
        second: String,
        cible: String,
    },
}

/// Nature d'un asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetKind {
    /// Image, à convertir en BMP ([`crate::bitmap`]), rangée sous `rf/000/`.
    Image,
    /// Son MP3 ([`crate::audio`]), rangé sous `sf/000/`.
    Audio,
}

/// Correspondance entre un asset de l'archive STUdio et son fichier dans le pack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetTarget {
    /// Nature de l'asset.
    pub kind: AssetKind,
    /// Nom tel que dans `story.json`.
    pub name: String,
    /// Chemin dans l'archive : `assets/<name>`.
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
    /// Le pack doit contenir un fichier `nm` vide (jamais chiffré).
    pub night_mode: bool,
}

/// Assets d'une nature, numérotés dans l'ordre de première apparition.
struct Table<'a> {
    kind: AssetKind,
    /// Nom dans `story.json` → (numéro, nom normalisé).
    by_name: HashMap<&'a str, (u32, String)>,
    /// Noms dans l'ordre des numéros.
    order: Vec<&'a str>,
    /// Nom normalisé → premier nom qui l'a pris.
    by_target: HashMap<String, &'a str>,
}

impl<'a> Table<'a> {
    fn new(kind: AssetKind) -> Self {
        Table {
            kind,
            by_name: HashMap::new(),
            order: Vec::new(),
            by_target: HashMap::new(),
        }
    }

    fn add(&mut self, name: &'a str) -> Result<(), IndexError> {
        if self.by_name.contains_key(name) {
            return Ok(());
        }
        let norm = normalized_asset_name(name)?;
        if let Some(premier) = self.by_target.get(&norm) {
            return Err(IndexError::Collision {
                premier: (*premier).to_owned(),
                second: name.to_owned(),
                cible: self.pack_path(&norm),
            });
        }
        // Au plus un asset par stageNode, donc ≤ MAX_STAGE_NODES : tient dans un u32.
        let number = u32::try_from(self.order.len()).expect("nombre d'assets borné par story");
        self.by_target.insert(norm.clone(), name);
        self.by_name.insert(name, (number, norm));
        self.order.push(name);
        Ok(())
    }

    fn number(&self, name: Option<&str>) -> i32 {
        name.and_then(|n| self.by_name.get(n))
            .map_or(-1, |&(i, _)| i32::try_from(i).expect("borné par story"))
    }

    fn pack_path(&self, norm: &str) -> String {
        match self.kind {
            AssetKind::Image => format!("rf/000/{norm}"),
            AssetKind::Audio => format!("sf/000/{norm}"),
        }
    }

    fn index_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.order.len() * (ENTRY_PREFIX.len() + ASSET_NAME_LEN));
        for name in &self.order {
            out.extend_from_slice(ENTRY_PREFIX.as_bytes());
            out.extend_from_slice(self.by_name[name].1.as_bytes());
        }
        out
    }

    fn targets(&self) -> impl Iterator<Item = AssetTarget> + '_ {
        self.order.iter().map(|name| AssetTarget {
            kind: self.kind,
            name: (*name).to_owned(),
            archive_path: format!("assets/{name}"),
            pack_path: self.pack_path(&self.by_name[name].1),
        })
    }
}

/// Nom normalisé d'un asset : les 8 derniers caractères du nom sans extension, en majuscules.
pub fn normalized_asset_name(name: &str) -> Result<String, IndexError> {
    let stem = strip_extension(name);
    let chars: Vec<char> = stem.chars().collect();
    if chars.len() < ASSET_NAME_LEN {
        return Err(IndexError::NomCourt {
            nom: name.to_owned(),
        });
    }
    let tail = &chars[chars.len() - ASSET_NAME_LEN..];
    if !tail
        .iter()
        .all(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
    {
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
pub fn build_index(story: &Story) -> Result<PackIndex, IndexError> {
    let nodes = story.stage_nodes();
    let mut images = Table::new(AssetKind::Image);
    let mut sounds = Table::new(AssetKind::Audio);
    for node in nodes {
        if let Some(n) = node.image.as_deref() {
            images.add(n)?;
        }
        if let Some(n) = node.audio.as_deref() {
            sounds.add(n)?;
        }
    }

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

    let ri = images.index_bytes();
    let si = sounds.index_bytes();
    let assets = images.targets().chain(sounds.targets()).collect();
    Ok(PackIndex {
        ni,
        li,
        ri,
        si,
        assets,
        night_mode: story.night_mode(),
    })
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
    put_i32(ni, images.number(node.image.as_deref()));
    put_i32(ni, sounds.number(node.audio.as_deref()));
    for t in [node.ok_transition, node.home_transition] {
        put_transition(ni, t, story, li_start);
    }
    match node.controls {
        Some(c) => {
            for flag in [c.wheel, c.ok, c.home, c.pause, c.autoplay] {
                put_u16(ni, u16::from(flag));
            }
            put_u16(ni, 0);
        }
        None => ni.resize(start + NODE_SIZE, 0xAA),
    }
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
            put_i32(ni, t.option_index);
        }
        None => ni.extend_from_slice(&[0xFF; 12]),
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
    fn nom_normalise() {
        assert_eq!(normalized_asset_name("x/abcdefgh.mp3").unwrap(), "ABCDEFGH");
        assert_eq!(
            normalized_asset_name("prefixe_ab-d1234.Png").unwrap(),
            "AB-D1234"
        );
        assert!(matches!(
            normalized_asset_name("court.png"),
            Err(IndexError::NomCourt { .. })
        ));
        assert!(matches!(
            normalized_asset_name("abcdéfgh.png"),
            Err(IndexError::NomInvalide { .. })
        ));
        assert!(matches!(
            normalized_asset_name("abcd efgh.png"),
            Err(IndexError::NomInvalide { .. })
        ));
    }
}
