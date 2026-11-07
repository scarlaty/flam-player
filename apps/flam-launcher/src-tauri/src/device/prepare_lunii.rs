//! Préparation hors appareil d'une histoire au format Lunii pour une Flam en firmware 2.x
//! (#530, épopée #517 ; note de conception `docs/studio-lunii.md`, § 3, 5.2, 5.4 et 6).
//!
//! À partir d'un pack clair `library/<uuid>-<version>.lunii.pk` (écrit à l'installation, sans
//! clé), construit dans le dossier de travail l'arborescence `str/<uuid>/` chiffrée pour
//! l'appareil. Le [`PreparedStory`] produit passe tel quel par `transfer::send_with`.
//! **Rien n'est écrit sur l'appareil ici.**
//!
//! | Entrée du `.lunii.pk`             | Dans `str/<uuid>/`         | Traitement                      |
//! |-----------------------------------|----------------------------|---------------------------------|
//! | `uuid.bin` (16 octets)            | (aucune)                   | donne le nom du dossier         |
//! | `_metadata.json`, `_thumbnail.png`| (aucune)                   | ignorées                        |
//! | `ni` (obligatoire)                | `ni`                       | contrôlé, copié en clair        |
//! | `nm` (facultatif, vide)           | `nm`                       | contrôlé, copié (vide)          |
//! | `li`, `ri`, `si` (obligatoires)   | mêmes noms                 | contrôlés, en-tête chiffré      |
//! | `info` (obligatoire)              | `info`                     | contrôlé (ASCII, LF), en clair  |
//! | `img/thumbnail.lif` (facultatif)  | `img/thumbnail.lif`        | copié en clair                  |
//! | `rf/000/<nom>`, `sf/000/<nom>`    | `rf/000/<NOM>`, `sf/000/<NOM>` | en-tête chiffré (512 octets) |
//! | (aucune)                          | `key`                      | déduit de l'appareil            |
//! | toute autre entrée                | —                          | refus du pack                   |
//!
//! Chiffrement : [`DeviceKeys::encrypt_lunii_file`] (#523). Les fichiers d'images et de sons
//! sont lus par blocs : seuls leurs 512 premiers octets passent par le chiffrement, la suite
//! est recopiée ; le résultat est identique au chiffrement du fichier entier.
//!
//! # Invariants
//!
//! 1. Firmware 1.x (clés « carrier ») : refus [`PrepareError::LuniiNeedsV2`] **avant toute
//!    lecture ou écriture** (pas même le dossier de travail).
//! 2. Tout est contrôlé avant d'écrire : noms, entrées attendues, cohérence de `ni`, `li`,
//!    `ri`, `si` et des fichiers de `rf/000/`, `sf/000/`, `info`, `nm`, UUID.
//! 3. Aucun nom de l'archive ne devient un chemin de sortie : les noms produits sont des
//!    constantes, ou un nom de `ri`/`si` contrôlé (8 caractères ASCII alphanumériques, mis en
//!    majuscules).
//! 4. Le dossier final `<travail>/str/<uuid>/` est absent ou complet : chantier
//!    `.partiel-<uuid>-<aléa>` renommé à la fin ([`super::prepare::build_in_staging`], partagé
//!    avec les histoires Flam) ; chantier supprimé en cas d'erreur ou d'annulation.
//! 5. Coût borné et linéaire : bornes de [`Limits`] sur les tailles **lues**, bornes de la note
//!    (§ 6.2) sur les nœuds, les entrées de `li` et les assets ; aucun parcours du graphe.
//! 6. Aucune clé journalisée ni écrite ailleurs que dans les fichiers chiffrés et `key`.
//!
//! # Pannes (reprises de la note, § 6.3, pour cette partie)
//!
//! | Situation                                                     | Comportement                                |
//! |---------------------------------------------------------------|---------------------------------------------|
//! | Firmware 1.x                                                  | `LuniiNeedsV2`, rien d'écrit                |
//! | Chemin hostile (`..`, absolu, `\`, nom réservé, doublon de casse), lien symbolique, entrée chiffrée | `LuniiInvalid`, rien d'écrit |
//! | Entrée inattendue ou obligatoire absente                      | `LuniiInvalid`, rien d'écrit                |
//! | `ni`, `li`, `ri`, `si` incohérents (tailles, compteurs, indices, transitions) | `LuniiInvalid`, rien d'écrit |
//! | Asset de `ri`/`si` absent, ou fichier de `rf/`/`sf/` non référencé | `LuniiInvalid`, rien d'écrit           |
//! | Trop d'entrées, tailles déclarées ou lues trop grandes        | `LuniiTooLarge`, chantier supprimé          |
//! | Contenu altéré (CRC faux), archive tronquée                   | `LuniiInvalid`, chantier supprimé           |
//! | Erreur d'écriture (disque plein…)                             | `Io`, chantier supprimé                     |
//! | Annulation                                                    | `Cancelled`, chantier supprimé              |
//! | Arrêt brutal pendant la préparation                           | chantier supprimé au prochain envoi (`cleanup_stale`) |
//!
//! Le contenu des images (BMP) et des sons (MP3) n'est pas relu : il a été produit et contrôlé
//! à l'installation ; un pack modifié à la main reste borné et ne peut écrire que sous
//! `str/<uuid>/`.

use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::atomic::AtomicBool;

use super::crypto::{CryptoError, DeviceKeys, LUNII_CIPHERED_LEN};
use super::prepare::{
    build_in_staging, check_cancel, copy_chunks, for_each_entry, invalid, open_archive, read_uuid,
    write_all_to, Archive, Limits, PrepareError, PreparedStory, RealSink, Sink, KEY_FILE,
    UUID_ENTRY,
};

/// Extension des packs au format Lunii de la bibliothèque.
pub const PACK_SUFFIX: &str = ".lunii.pk";

const NI: &str = "ni";
const NM: &str = "nm";
const LI: &str = "li";
const RI: &str = "ri";
const SI: &str = "si";
const INFO: &str = "info";
const THUMBNAIL: &str = "img/thumbnail.lif";
const IMAGE_DIR: &str = "rf/000";
const SOUND_DIR: &str = "sf/000";
/// Entrées propres à la bibliothèque, jamais copiées sur la Flam (casse indifférente).
const IGNORED: [&str; 2] = ["_metadata.json", "_thumbnail.png"];

/// Taille de l'en-tête de `ni`.
const NI_HEADER_LEN: usize = 0x200;
/// Taille d'un nœud de `ni`.
const NI_NODE_LEN: usize = 0x2C;
/// Taille d'une entrée de `ri` ou `si` : `000\` puis 8 caractères.
const ASSET_ENTRY_LEN: usize = 12;
const ASSET_PREFIX: &[u8; 4] = b"000\\";
const ASSET_NAME_LEN: usize = 8;
/// Bornes de la note (§ 6.2).
const MAX_NODES: usize = 20_000;
const MAX_LI_ENTRIES: usize = 100_000;
const MAX_ASSETS: usize = 10_000;
/// Taille maximale de `info` (titre répété, auteur).
const MAX_INFO_BYTES: u64 = 64 * 1024;

/// Prépare le pack Lunii clair `pack` pour l'appareil `keys` dans `work_dir` (créé au besoin).
/// Une préparation précédente de la même histoire est remplacée. Bloquant : à appeler hors du
/// runtime asynchrone. `cancel` est consulté entre chaque fichier et chaque bloc écrit.
///
/// Erreur [`PrepareError::LuniiNeedsV2`] si `keys` ne sont pas celles d'une Flam en 2.x, avant
/// toute lecture ou écriture. Un seul appel à la fois par histoire et par dossier de travail
/// (sérialisé par l'appelant).
pub fn prepare(
    pack: &Path,
    keys: &DeviceKeys,
    work_dir: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<PreparedStory, PrepareError> {
    prepare_with(&RealSink, pack, keys, work_dir, limits, cancel)
}

fn crypto_error(err: CryptoError) -> PrepareError {
    match err {
        CryptoError::LuniiFormatNeedsV2 => PrepareError::LuniiNeedsV2,
        other => PrepareError::Io(io::Error::other(other.to_string())),
    }
}

/// Messages des fonctions partagées avec les histoires Flam, rapportés au pack Lunii.
fn lunii_error(err: PrepareError) -> PrepareError {
    match err {
        PrepareError::Invalid(m) => PrepareError::LuniiInvalid(m),
        PrepareError::TooLarge(m) => PrepareError::LuniiTooLarge(m),
        other => other,
    }
}

pub(crate) fn prepare_with(
    sink: &dyn Sink,
    pack: &Path,
    keys: &DeviceKeys,
    work_dir: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<PreparedStory, PrepareError> {
    // Invariant 1 : refus du 1.x avant toute lecture ou écriture.
    keys.encrypt_lunii_file(NI, &[]).map_err(crypto_error)?;
    prepare_checked(sink, pack, keys, work_dir, limits, cancel).map_err(lunii_error)
}

/// Indices des entrées de l'archive, d'après leur nom.
#[derive(Default)]
struct Entries {
    uuid: Option<usize>,
    ni: Option<usize>,
    nm: Option<usize>,
    li: Option<usize>,
    ri: Option<usize>,
    si: Option<usize>,
    info: Option<usize>,
    thumbnail: Option<usize>,
    /// Fichiers de `rf/000/`, par nom en majuscules.
    images: HashMap<String, usize>,
    /// Fichiers de `sf/000/`, par nom en majuscules.
    sounds: HashMap<String, usize>,
    declared: u64,
}

/// Nom d'asset valide : 8 caractères ASCII alphanumériques (nom court 8.3 sans extension).
fn is_asset_name(name: &[u8]) -> bool {
    name.len() == ASSET_NAME_LEN && name.iter().all(u8::is_ascii_alphanumeric)
}

fn classify(entries: &mut Entries, index: usize, relative: &str) -> Result<(), PrepareError> {
    let slot = match relative {
        UUID_ENTRY => &mut entries.uuid,
        NI => &mut entries.ni,
        NM => &mut entries.nm,
        LI => &mut entries.li,
        RI => &mut entries.ri,
        SI => &mut entries.si,
        INFO => &mut entries.info,
        THUMBNAIL => &mut entries.thumbnail,
        _ => {
            for (dir, map) in [
                (IMAGE_DIR, &mut entries.images),
                (SOUND_DIR, &mut entries.sounds),
            ] {
                if let Some(name) = relative
                    .strip_prefix(dir)
                    .and_then(|rest| rest.strip_prefix('/'))
                {
                    if is_asset_name(name.as_bytes()) {
                        // Doublons à la casse près déjà refusés par `for_each_entry`.
                        map.insert(name.to_ascii_uppercase(), index);
                        return Ok(());
                    }
                }
            }
            return Err(invalid(format!("entrée inattendue : {relative:?}")));
        }
    };
    // Noms exacts uniques : déjà garanti par `for_each_entry`.
    *slot = Some(index);
    Ok(())
}

/// Lit entièrement l'entrée `index` (au plus `max` octets, d'après les octets **lus**).
fn read_entry(
    zip: &mut Archive,
    index: usize,
    name: &str,
    max: u64,
) -> Result<Vec<u8>, PrepareError> {
    let mut entry = zip
        .by_index(index)
        .map_err(|e| invalid(format!("{name} : {e}")))?;
    let mut data = Vec::new();
    (&mut entry)
        .take(max.saturating_add(1))
        .read_to_end(&mut data)
        .map_err(|e| invalid(format!("{name} : {e}")))?;
    if data.len() as u64 > max {
        return Err(PrepareError::TooLarge(format!(
            "{name} dépasse {max} octets"
        )));
    }
    Ok(data)
}

fn u32_at(data: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(data[at..at + 4].try_into().expect("4 octets"))
}

fn i32_at(data: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(data[at..at + 4].try_into().expect("4 octets"))
}

/// Noms (en majuscules) des entrées de `ri` ou `si` : `000\` puis 8 caractères ASCII
/// alphanumériques, sans doublon.
fn asset_names(data: &[u8], file: &str) -> Result<Vec<String>, PrepareError> {
    if !data.len().is_multiple_of(ASSET_ENTRY_LEN) {
        return Err(invalid(format!(
            "{file} : {} octets, pas un multiple de {ASSET_ENTRY_LEN}",
            data.len()
        )));
    }
    let count = data.len() / ASSET_ENTRY_LEN;
    if count > MAX_ASSETS {
        return Err(invalid(format!(
            "{file} : {count} entrées (au plus {MAX_ASSETS})"
        )));
    }
    let mut names = Vec::with_capacity(count);
    let mut seen = HashSet::with_capacity(count);
    for (i, entry) in data.as_chunks::<ASSET_ENTRY_LEN>().0.iter().enumerate() {
        let (prefix, name) = entry.split_at(ASSET_PREFIX.len());
        if prefix != ASSET_PREFIX || !is_asset_name(name) {
            return Err(invalid(format!("{file} : entrée {i} mal formée")));
        }
        let name = String::from_utf8_lossy(name).to_ascii_uppercase();
        if !seen.insert(name.clone()) {
            return Err(invalid(format!("{file} : {name} en double")));
        }
        names.push(name);
    }
    Ok(names)
}

/// Contrôle `li` : entiers signés 32 bits, chacun indice d'un nœud de scène. Rend le nombre
/// d'entrées.
fn check_li(li: &[u8], nodes: usize) -> Result<usize, PrepareError> {
    if !li.len().is_multiple_of(4) {
        return Err(invalid(format!(
            "li : {} octets, pas un multiple de 4",
            li.len()
        )));
    }
    let count = li.len() / 4;
    if count > MAX_LI_ENTRIES {
        return Err(invalid(format!(
            "li : {count} entrées (au plus {MAX_LI_ENTRIES})"
        )));
    }
    for i in 0..count {
        let node = i32_at(li, 4 * i);
        if usize::try_from(node).map_or(true, |n| n >= nodes) {
            return Err(invalid(format!(
                "li : entrée {i} vers le nœud {node} ({nodes} nœuds)"
            )));
        }
    }
    Ok(count)
}

/// Contrôle une transition (position dans `li`, nombre de choix, choix) : les trois à `-1`
/// (absente), ou une plage de `li` existante et un choix dans `[0, nombre)`.
fn check_transition(
    node: &[u8],
    at: usize,
    li_entries: usize,
    what: &str,
    i: usize,
) -> Result<(), PrepareError> {
    let (pos, count, option) = (i32_at(node, at), i32_at(node, at + 4), i32_at(node, at + 8));
    if (pos, count, option) == (-1, -1, -1) {
        return Ok(());
    }
    let ok = pos >= 0
        && count >= 1
        && option >= 0
        && option < count
        && i64::from(pos) + i64::from(count) <= li_entries as i64;
    if ok {
        Ok(())
    } else {
        Err(invalid(format!(
            "ni : nœud {i}, transition {what} incohérente ({pos}, {count}, {option})"
        )))
    }
}

/// Indice d'image ou de son : `-1` (aucun) ou dans `[0, count)`.
fn check_asset_index(value: i32, count: usize, what: &str, i: usize) -> Result<(), PrepareError> {
    if value == -1 || usize::try_from(value).is_ok_and(|v| v < count) {
        Ok(())
    } else {
        Err(invalid(format!(
            "ni : nœud {i}, {what} {value} hors de [0, {count})"
        )))
    }
}

/// Contrôle `ni` (note, § 3.2) avec `li`, et les nombres d'images et de sons.
fn check_ni(ni: &[u8], li: &[u8], images: usize, sounds: usize) -> Result<(), PrepareError> {
    if ni.len() < NI_HEADER_LEN {
        return Err(invalid(format!(
            "ni : {} octets, en-tête incomplet",
            ni.len()
        )));
    }
    if u32_at(ni, 0x04) as usize != NI_HEADER_LEN || u32_at(ni, 0x08) as usize != NI_NODE_LEN {
        return Err(invalid(
            "ni : en-tête inattendu (début ou taille des nœuds)",
        ));
    }
    let nodes = u32_at(ni, 0x0C) as usize;
    if nodes == 0 || nodes > MAX_NODES {
        return Err(invalid(format!("ni : {nodes} nœuds (de 1 à {MAX_NODES})")));
    }
    if ni.len() != NI_HEADER_LEN + nodes * NI_NODE_LEN {
        return Err(invalid(format!(
            "ni : {} octets pour {nodes} nœuds",
            ni.len()
        )));
    }
    let (declared_images, declared_sounds) = (u32_at(ni, 0x10) as usize, u32_at(ni, 0x14) as usize);
    if declared_images != images || declared_sounds != sounds {
        return Err(invalid(format!(
            "ni : {declared_images} images et {declared_sounds} sons annoncés, \
             {images} et {sounds} dans ri et si"
        )));
    }
    let li_entries = check_li(li, nodes)?;
    for (i, node) in ni[NI_HEADER_LEN..]
        .as_chunks::<NI_NODE_LEN>()
        .0
        .iter()
        .enumerate()
    {
        check_asset_index(i32_at(node, 0x00), images, "image", i)?;
        check_asset_index(i32_at(node, 0x04), sounds, "son", i)?;
        check_transition(node, 0x08, li_entries, "OK", i)?;
        check_transition(node, 0x14, li_entries, "Maison", i)?;
        let controls_ok = node[0x20..0x2A]
            .as_chunks::<2>()
            .0
            .iter()
            .all(|c| *c == [0, 0] || *c == [1, 0]);
        if !controls_ok || node[0x2A..] != [0, 0] {
            return Err(invalid(format!("ni : nœud {i}, commandes mal formées")));
        }
    }
    Ok(())
}

/// `info` : texte ASCII imprimable, lignes séparées par LF, titre (première ligne) non vide
/// (note, § 3.8 et Q4).
fn check_info(info: &[u8]) -> Result<(), PrepareError> {
    let printable = info
        .iter()
        .all(|&b| b == b'\n' || (0x20..0x7F).contains(&b));
    let title = info.split(|&b| b == b'\n').next().unwrap_or_default();
    if !printable || title.iter().all(|&b| b == b' ') {
        return Err(invalid(
            "info : attendu un titre en ASCII, lignes séparées par LF",
        ));
    }
    Ok(())
}

/// Associe chaque nom de `ri` (ou `si`) à son fichier de `rf/000/` (ou `sf/000/`) ; tout
/// fichier non référencé est refusé.
fn match_assets(
    names: &[String],
    files: &HashMap<String, usize>,
    dir: &str,
    index_file: &str,
) -> Result<Vec<(String, usize)>, PrepareError> {
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let index = files
            .get(name)
            .ok_or_else(|| invalid(format!("{dir}/{name} absent (référencé par {index_file})")))?;
        out.push((format!("{dir}/{name}"), *index));
    }
    if files.len() != names.len() {
        let mut extra: Vec<&String> = files.keys().filter(|f| !names.contains(f)).collect();
        extra.sort();
        return Err(invalid(format!(
            "entrée inattendue : {dir}/{} non référencé par {index_file}",
            extra.first().map_or("?", |s| s.as_str())
        )));
    }
    Ok(out)
}

/// Contenu contrôlé du pack, prêt à écrire.
struct Checked {
    uuid: String,
    ni: Vec<u8>,
    nm: bool,
    li: Vec<u8>,
    ri: Vec<u8>,
    si: Vec<u8>,
    info: Vec<u8>,
    thumbnail: Option<usize>,
    /// Nom produit (`rf/000/<NOM>`, `sf/000/<NOM>`) et indice de l'entrée, dans l'ordre de
    /// `ri` puis de `si`.
    assets: Vec<(String, usize)>,
}

/// Plan et contrôles : rien n'est écrit.
fn check_pack(zip: &mut Archive, limits: &Limits) -> Result<Checked, PrepareError> {
    let mut entries = Entries::default();
    for_each_entry(zip, |index, relative, size| {
        if IGNORED.iter().any(|i| i.eq_ignore_ascii_case(&relative)) {
            return Ok(());
        }
        classify(&mut entries, index, &relative)?;
        if relative != UUID_ENTRY {
            entries.declared = entries.declared.saturating_add(size);
        }
        Ok(())
    })?;
    // Refus précoce d'après les tailles déclarées ; les tailles lues restent contrôlées.
    if entries.declared > limits.max_total_bytes {
        return Err(PrepareError::TooLarge(format!(
            "plus de {} octets déclarés",
            limits.max_total_bytes
        )));
    }
    let required =
        |slot: Option<usize>, name: &str| slot.ok_or_else(|| invalid(format!("{name} absent")));
    let uuid_index = required(entries.uuid, UUID_ENTRY)?;
    let ni_index = required(entries.ni, NI)?;
    let li_index = required(entries.li, LI)?;
    let ri_index = required(entries.ri, RI)?;
    let si_index = required(entries.si, SI)?;
    let info_index = required(entries.info, INFO)?;

    let uuid = read_uuid(zip, uuid_index)?;
    let max = limits.max_encrypted_bytes;
    let ni = read_entry(zip, ni_index, NI, max)?;
    let li = read_entry(zip, li_index, LI, max)?;
    let ri = read_entry(zip, ri_index, RI, max)?;
    let si = read_entry(zip, si_index, SI, max)?;
    let info = read_entry(zip, info_index, INFO, MAX_INFO_BYTES.min(max))?;
    if let Some(nm) = entries.nm {
        if !read_entry(zip, nm, NM, max)?.is_empty() {
            return Err(invalid("nm : fichier non vide"));
        }
    }
    let images = asset_names(&ri, RI)?;
    let sounds = asset_names(&si, SI)?;
    check_ni(&ni, &li, images.len(), sounds.len())?;
    check_info(&info)?;
    let mut assets = match_assets(&images, &entries.images, IMAGE_DIR, RI)?;
    assets.extend(match_assets(&sounds, &entries.sounds, SOUND_DIR, SI)?);
    Ok(Checked {
        uuid,
        ni,
        nm: entries.nm.is_some(),
        li,
        ri,
        si,
        info,
        thumbnail: entries.thumbnail,
        assets,
    })
}

/// Ajoute `n` octets à `total`, borné par [`Limits::max_total_bytes`].
fn add_total(total: &mut u64, n: usize, limits: &Limits) -> Result<(), PrepareError> {
    *total += n as u64;
    if *total > limits.max_total_bytes {
        return Err(PrepareError::TooLarge(format!(
            "plus de {} octets",
            limits.max_total_bytes
        )));
    }
    Ok(())
}

/// Écrit l'asset `index` sous `target` : 512 premiers octets chiffrés, suite recopiée par blocs.
#[allow(clippy::too_many_arguments)]
fn write_asset(
    sink: &dyn Sink,
    zip: &mut Archive,
    index: usize,
    target: &str,
    keys: &DeviceKeys,
    staging: &Path,
    total: &mut u64,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<(), PrepareError> {
    let mut entry = zip
        .by_index(index)
        .map_err(|e| invalid(format!("{target} : {e}")))?;
    let mut head = Vec::with_capacity(LUNII_CIPHERED_LEN);
    (&mut entry)
        .take(LUNII_CIPHERED_LEN as u64)
        .read_to_end(&mut head)
        .map_err(|e| invalid(format!("{target} : {e}")))?;
    let head = keys
        .encrypt_lunii_file(target, &head)
        .map_err(crypto_error)?;
    add_total(total, head.len(), limits)?;
    let mut out = sink.create(&staging.join(target))?;
    out.write_all(&head)?;
    copy_chunks(&mut entry, &mut out, target, total, limits, cancel)?;
    out.flush()?;
    Ok(())
}

fn prepare_checked(
    sink: &dyn Sink,
    pack: &Path,
    keys: &DeviceKeys,
    work_dir: &Path,
    limits: &Limits,
    cancel: &AtomicBool,
) -> Result<PreparedStory, PrepareError> {
    check_cancel(cancel)?;
    let mut zip = open_archive(pack, limits)?;
    let checked = check_pack(&mut zip, limits)?;
    check_cancel(cancel)?;

    let mut files: Vec<String> = [NI, LI, RI, SI, INFO, KEY_FILE]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if checked.nm {
        files.push(NM.to_string());
    }
    if checked.thumbnail.is_some() {
        files.push(THUMBNAIL.to_string());
    }
    files.extend(checked.assets.iter().map(|(name, _)| name.clone()));

    let (dir, total_bytes) = build_in_staging(sink, work_dir, &checked.uuid, cancel, |staging| {
        let mut total = 0u64;
        // Fichiers en clair (`ni`, `nm`, `info`) puis index chiffrés (`li`, `ri`, `si`).
        let mut small: Vec<(&str, Vec<u8>)> = vec![(NI, checked.ni.clone())];
        if checked.nm {
            small.push((NM, Vec::new()));
        }
        small.push((INFO, checked.info.clone()));
        for (name, plain) in [(LI, &checked.li), (RI, &checked.ri), (SI, &checked.si)] {
            small.push((
                name,
                keys.encrypt_lunii_file(name, plain).map_err(crypto_error)?,
            ));
        }
        for (name, data) in &small {
            check_cancel(cancel)?;
            add_total(&mut total, data.len(), limits)?;
            write_all_to(sink, &staging.join(name), data)?;
        }
        if let Some(index) = checked.thumbnail {
            check_cancel(cancel)?;
            let target = staging.join(THUMBNAIL);
            sink.create_dir_all(target.parent().expect("img/"))?;
            let mut entry = zip
                .by_index(index)
                .map_err(|e| invalid(format!("{THUMBNAIL} : {e}")))?;
            let mut out = sink.create(&target)?;
            copy_chunks(&mut entry, &mut out, THUMBNAIL, &mut total, limits, cancel)?;
            out.flush()?;
        }
        for dir in [IMAGE_DIR, SOUND_DIR] {
            if checked.assets.iter().any(|(name, _)| name.starts_with(dir)) {
                sink.create_dir_all(&staging.join(dir))?;
            }
        }
        for (target, index) in &checked.assets {
            check_cancel(cancel)?;
            write_asset(
                sink, &mut zip, *index, target, keys, staging, &mut total, limits, cancel,
            )?;
        }
        check_cancel(cancel)?;
        let key = keys.key_file();
        add_total(&mut total, key.len(), limits)?;
        write_all_to(sink, &staging.join(KEY_FILE), &key[..])?;
        Ok(total)
    })?;
    files.sort();
    tracing::info!(uuid = %checked.uuid, fichiers = files.len(), octets = total_bytes,
        "histoire au format Lunii préparée pour la Flam");
    Ok(PreparedStory {
        uuid: checked.uuid,
        dir,
        files,
        total_bytes,
    })
}

#[cfg(test)]
mod tests;
