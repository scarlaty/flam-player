//! Assemblage en mémoire du paquet FLAM : la partie de `_convert` (`tools/telmi2flam/telmi2flam.py`)
//! qui va de la lecture de `metadata.json` à la liste ordonnée des fichiers de sortie. L'écriture
//! du `.plain.pk` et du dossier `.plain` est dans [`crate::output`].
//!
//! Ordre des étapes identique à Python, pour que la première erreur et les avertissements
//! soient les mêmes : `metadata.json`, `nodes.json`, `notes.json`, titre, sous-titre et UUID,
//! graphe (assets demandés à [`StoryAssets`]), couverture et `title.mp3`, caractères hors police,
//! assets manquants, `startAction`, `nodes.lua`, puis les fichiers fixes (moteur, runtime,
//! `img/empty.lif`, `sounds/silent.mp3`, `info.plain`, `version`, `uuid.bin`).
//!
//! Invariants : aucune panique ; noms de sortie relatifs et sûrs (préfixes fixes et
//! `[A-Za-z0-9_-]`, runtime vérifié par `build.rs`), revérifiés en fin d'assemblage par la garde
//! zip-slip de Python ([`check_names`], `invalid_content`) ; coût linéaire en la taille de l'archive
//! (chaque image est décodée une fois, la couverture deux fois comme en Python) ; annulation
//! consultée avant chaque asset et dans les boucles de lignes des images.
//!
//! Progression ([`build_package_with`]) : phases `images` et `audio` annoncées après la
//! validation de `stages` et `inventory` (totaux : assets distincts référencés), `scripts`
//! après la génération de `nodes.lua` (`nodes.lua`, moteur, runtime), avertissements émis dans
//! l'ordre de Python.
//!
//! Mémoire (#355) : [`build_package_streamed`] ne garde pas les MP3 recopiés tels quels, mais
//! une référence à leur entrée dans l'archive ([`Data::Source`] : nom, taille, CRC calculés à la
//! lecture faite pour `mp3map`) ; [`crate::output::write_package_from`] les relit un à un, dans
//! l'ordre canonique, et refuse une entrée dont la taille ou le CRC a changé. Les octets écrits
//! sont ceux du paquet en mémoire de [`build_package_with`].
//!
//! Options ([`Options`]) : `keep_size`, `allow_missing`, sélecteur des choix et mode de la
//! vignette (`thumb_fit` : `bandes`, `flou`, `rogner`, comme `--thumb-fit` ; #282), transmis à
//! `png_to_thumb_lif` ; la couverture et les images de scène ne dépendent pas du mode.
//!
//! Audio : un MP3 hors 44 100 Hz est rééchantillonné par le ffmpeg externe de [`StoryAssets`]
//! (#354 ; `FLAM_FFMPEG`, sinon `PATH`) ; sans ffmpeg, ou si sa sortie est inutilisable, il est
//! recopié tel quel avec l'avertissement `audio_samplerate`, comme Python.
//!
//! Parité avec Python : `tests/package_parity.rs` compare les octets du `.plain.pk`, les codes
//! et les messages des avertissements, avec `FLAM_FFMPEG` vide des deux côtés (Python :
//! `tests/data/gen_package_cases.py`) ; `tests/ffmpeg.rs` la vérifie avec un faux ffmpeg.
//! Écarts assumés (listés dans `package_parity.rs`, `assumed_divergence`) : formats délégués à
//! Python (`unsupported_format`, D4), JSON hors limites, entrées chiffrées, noms non UTF-8 et
//! positions d'en-tête négatives, où le Rust échoue avec un code propre au port. Les lignes
//! d'information `  i …` et le compte des MP3 rééchantillonnés ([`StoryAssets::infos`],
//! [`StoryAssets::resampled`]) ne sont pas encore affichés ni émis (CLI, #133).

use std::collections::HashSet;
use std::io::{Read, Seek};

use crate::archive::{check_asset_name, find_root, Archive};
use crate::assets::{image_error, Missing, StoryAssets};
use crate::error::ConvError;
use crate::graph::{
    convert_graph, load_notes, nodes_lua, truthy, Header, Sources, Start, Warning, MAIN_LUA,
};
use crate::image::{png_to_fit_lif, png_to_thumb_lif, Align};
use crate::json::{expect, load_json, Kind, Object, Value};
use crate::lua::py_float_repr;
use crate::output::check_names;
use crate::png::PngError;
use crate::progress::{Phase, Progress};
use crate::runtime::{RUNTIME, STORY_LUA};
use crate::text::{describe_replaced, py_strip, text, Replaced};
use crate::{lif, md5, mp3map};

/// Sélecteur des choix (`--selector`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Selector {
    #[default]
    Carousel,
    Image,
}

impl Selector {
    pub fn as_str(self) -> &'static str {
        match self {
            Selector::Carousel => "carousel",
            Selector::Image => "image",
        }
    }
}

/// Options de conversion qui changent le contenu du paquet.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// `--keep-size` : images à leur taille native (bornée à 2047 px).
    pub keep_size: bool,
    /// `--allow-missing` : assets manquants remplacés au lieu d'un échec.
    pub allow_missing: bool,
    pub selector: Selector,
    /// `--thumb-fit` : remplissage de la vignette (défaut `flou`, comme Python).
    pub thumb_fit: crate::image::ThumbFit,
}

/// Contenu d'une entrée du paquet assemblé par [`build_package_streamed`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    /// Octets produits par la conversion.
    Bytes(Vec<u8>),
    /// MP3 recopié tel quel : entrée `name` (nom exact) de l'archive d'entrée, relue à
    /// l'écriture et refusée si sa taille ou son CRC-32 diffère.
    Source { name: String, size: u64, crc: u32 },
}

impl Data {
    /// Taille de l'entrée en octets.
    pub fn size(&self) -> u64 {
        match self {
            Data::Bytes(b) => b.len() as u64,
            Data::Source { size, .. } => *size,
        }
    }
}

impl From<Vec<u8>> for Data {
    fn from(bytes: Vec<u8>) -> Self {
        Data::Bytes(bytes)
    }
}

/// Paquet assemblé, prêt à écrire : tout en mémoire (`Vec<u8>`, [`build_package_with`]) ou
/// MP3 référencés dans l'archive ([`Data`], [`build_package_streamed`]).
#[derive(Debug, Clone)]
pub struct Package<D = Vec<u8>> {
    /// Fichiers dans l'ordre canonique de Python : `info.plain`, `version`, `main.lua`,
    /// `script/nodes.lua`, puis les autres triés, `uuid.bin` en dernier.
    pub entries: Vec<(String, D)>,
    /// Avertissements dans l'ordre d'émission (codes stables de l'événement `warning`).
    pub warnings: Vec<Warning>,
    /// Titre affiché (nom de sortie par défaut).
    pub title: String,
    /// Contenu de `uuid.bin` (nom de sortie par défaut : 4 premiers octets).
    pub uuid: [u8; 16],
    /// Compteurs du résumé humain.
    pub stats: Stats,
}

/// Compteurs du résumé humain de `_convert` (`stats`, `lua_stages`, `lua_actions`, `missing`
/// et `bad` de Python).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub stages: usize,
    pub actions: usize,
    /// Images écrites (converties, couverture et vignette, `empty.lif`).
    pub img: usize,
    /// Audios écrits (`title.mp3` compris).
    pub audio: usize,
    pub mp3map: usize,
    /// MP3 rééchantillonnés par ffmpeg (compté par fichier, cache compris).
    pub resampled: usize,
    /// Scripts du runtime (`.lua`).
    pub lib: usize,
    /// Icônes du runtime (`.lif`).
    pub ui: usize,
    /// Assets manquants (`--allow-missing`).
    pub missing: usize,
    /// Assets illisibles remplacés.
    pub bad: usize,
}

/// En-tête du MP3 silencieux : MPEG1 Layer III, 128 kbit/s, 44 100 Hz, stéréo.
const SILENT_FRAME_HEADER: [u8; 4] = [0xFF, 0xFB, 0x90, 0x00];
/// Trame complète : 417 octets, données nulles.
const SILENT_FRAME_LEN: usize = 417;
/// Nombre de trames (environ 0,6 s).
const SILENT_FRAMES: usize = 24;

/// `sounds/silent.mp3` : trame silencieuse répétée (entrées de menu sans audio).
pub fn silent_mp3() -> Vec<u8> {
    let mut frame = vec![0u8; SILENT_FRAME_LEN];
    frame[..4].copy_from_slice(&SILENT_FRAME_HEADER);
    frame.repeat(SILENT_FRAMES)
}

/// `repr(s)` de Python pour une chaîne dont les caractères sont tous en dessous de U+0100 ;
/// `None` au-delà (le caractère imprimable ou non dépend des tables Unicode de Python).
/// Le résultat est ajouté à `out` (pas de copie par niveau d'imbrication).
fn py_repr_str(s: &str, out: &mut String) -> Option<()> {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    out.push(quote);
    for c in s.chars() {
        let cp = c as u32;
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ if c == quote => {
                out.push('\\');
                out.push(c);
            }
            // Non imprimables : contrôles C0/C1, espace insécable (Zs), tiret conditionnel (Cf).
            _ if cp < 0x20 || (0x7f..=0xa0).contains(&cp) || cp == 0xad => {
                out.push_str(&format!("\\x{cp:02x}"));
            }
            _ if cp < 0x100 => out.push(c),
            _ => return None,
        }
    }
    out.push(quote);
    Some(())
}

/// `repr(v)` de Python pour une valeur JSON (dans une liste ou un objet), ajouté à `out`.
fn py_repr(v: &Value, out: &mut String) -> Option<()> {
    match v {
        Value::Str(s) => py_repr_str(s, out)?,
        Value::List(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                py_repr(item, out)?;
            }
            out.push(']');
        }
        Value::Object(map) => {
            out.push('{');
            for (i, (k, v)) in map.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                py_repr_str(k, out)?;
                out.push_str(": ");
                py_repr(v, out)?;
            }
            out.push('}');
        }
        other => out.push_str(&py_str(other)?),
    }
    Some(())
}

/// `str(v)` de Python pour une valeur JSON. Récursion bornée par la profondeur maximale du
/// JSON ([`crate::json::MAX_DEPTH`]). `None` si une chaîne imbriquée contient un caractère
/// au-delà de U+00FF (voir [`py_repr_str`]).
fn py_str(v: &Value) -> Option<String> {
    Some(match v {
        Value::Null => "None".to_owned(),
        Value::Bool(true) => "True".to_owned(),
        Value::Bool(false) => "False".to_owned(),
        Value::Int(i) => i.to_string(),
        Value::UInt(u) => u.to_string(),
        Value::Float(f) => py_float_repr(*f),
        Value::Str(s) => s.clone(),
        Value::List(_) | Value::Object(_) => {
            let mut out = String::new();
            py_repr(v, &mut out)?;
            out
        }
    })
}

/// `_text(str(v))`, ou délégation à Python si `str(v)` n'est pas reproductible ici (titre ou
/// catégorie qui sont une liste ou un objet contenant des caractères au-delà de U+00FF).
fn display_text(v: &Value, what: &str, replaced: &mut Replaced) -> Result<String, ConvError> {
    let s = py_str(v).ok_or_else(|| {
        ConvError::unsupported_format(format!(
            "metadata.json {what} : liste ou objet non pris en charge"
        ))
    })?;
    Ok(text(&s, Some(replaced)))
}

/// Ordre canonique des entrées (`order` de Python) ; les noms sont uniques (clés du `BTreeMap`,
/// déjà triées par points de code comme `sorted()`).
fn canonical<D>(mut files: std::collections::BTreeMap<String, D>) -> Vec<(String, D)> {
    let mut entries = Vec::with_capacity(files.len());
    for head in ["info.plain", "version", "main.lua", "script/nodes.lua"] {
        if let Some(data) = files.remove(head) {
            entries.push((head.to_owned(), data));
        }
    }
    let uuid = files.remove("uuid.bin");
    entries.extend(files);
    if let Some(data) = uuid {
        entries.push(("uuid.bin".to_owned(), data));
    }
    entries
}

/// Lit une entrée dont `find_root` a garanti la présence.
fn read_exact<R: Read + Seek>(archive: &mut Archive<R>, name: &str) -> Result<Vec<u8>, ConvError> {
    archive
        .read(name)?
        .ok_or_else(|| ConvError::internal_error(format!("{name} : entree introuvable")))
}

/// Totaux des phases `images` et `audio` : assets distincts référencés par les stages
/// (image, audio) et l'inventaire (image), noms non vides seulement (`imgs_todo` /
/// `audios_todo` de Python). Une même image en scène et en inventaire compte deux fois.
fn asset_totals(src: &Sources<'_>) -> (u64, u64) {
    fn name<'v>(obj: &'v Object, key: &str) -> Option<&'v str> {
        match obj.get(key) {
            Some(Value::Str(s)) if !s.is_empty() => Some(s),
            _ => None,
        }
    }
    let mut images: HashSet<(&str, bool)> = HashSet::new();
    let mut audios: HashSet<&str> = HashSet::new();
    for st in src.stages.into_iter().flat_map(|m| m.values()) {
        if let Value::Object(st) = st {
            images.extend(name(st, "image").map(|n| (n, false)));
            audios.extend(name(st, "audio"));
        }
    }
    for it in src.inventory {
        if let Value::Object(it) = it {
            images.extend(name(it, "image").map(|n| (n, true)));
        }
    }
    (images.len() as u64, audios.len() as u64)
}

/// Assemble le paquet d'une archive Telmi ouverte, sans suivi de progression (voir
/// [`build_package_with`]).
pub fn build_package<R: Read + Seek>(
    archive: &mut Archive<R>,
    options: &Options,
    cancelled: &dyn Fn() -> bool,
) -> Result<Package, ConvError> {
    build_package_with(archive, options, cancelled, &mut ())
}

/// Assemble le paquet d'une archive Telmi ouverte. `cancelled` est consulté avant chaque
/// asset et pendant le traitement des images ([`ConvError::cancelled`]) ; `progress` reçoit
/// les phases `images`, `audio`, `scripts` et les avertissements.
pub fn build_package_with<R: Read + Seek>(
    archive: &mut Archive<R>,
    options: &Options,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
) -> Result<Package, ConvError> {
    let pkg = build(archive, options, cancelled, progress, false)?;
    let mut entries = Vec::with_capacity(pkg.entries.len());
    for (name, data) in pkg.entries {
        match data {
            Data::Bytes(b) => entries.push((name, b)),
            Data::Source { .. } => {
                return Err(ConvError::internal_error(format!(
                    "{name} : entree non chargee"
                )))
            }
        }
    }
    Ok(Package {
        entries,
        warnings: pkg.warnings,
        title: pkg.title,
        uuid: pkg.uuid,
        stats: pkg.stats,
    })
}

/// [`build_package_with`] sans garder en mémoire les MP3 recopiés tels quels : leurs entrées
/// sont des [`Data::Source`] de `archive`, à écrire avec
/// [`crate::output::write_package_from`] et la même archive. Progression, avertissements,
/// erreurs et octets écrits identiques.
pub fn build_package_streamed<R: Read + Seek>(
    archive: &mut Archive<R>,
    options: &Options,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
) -> Result<Package<Data>, ConvError> {
    build(archive, options, cancelled, progress, true)
}

fn build<R: Read + Seek>(
    archive: &mut Archive<R>,
    options: &Options,
    cancelled: &dyn Fn() -> bool,
    progress: &mut dyn Progress,
    stream: bool,
) -> Result<Package<Data>, ConvError> {
    let check = || {
        if cancelled() {
            Err(ConvError::cancelled())
        } else {
            Ok(())
        }
    };
    check()?;
    let root = find_root(archive.names())?;
    let metadata = load_json(
        &read_exact(archive, &format!("{root}metadata.json"))?,
        "metadata.json",
    )?;
    let nodes_raw = read_exact(archive, &format!("{root}nodes.json"))?;
    let nodes = load_json(&nodes_raw, "nodes.json")?;
    expect(Some(&metadata), &[Kind::Object], "metadata.json", false)?;
    expect(Some(&nodes), &[Kind::Object], "nodes.json", false)?;

    let mut assets = StoryAssets::new(archive, &root, options.keep_size, cancelled, progress);
    assets.stream = stream;
    let notes_raw = assets.read("notes.json")?;
    let (notes, notes_warning) = load_notes(notes_raw.as_deref());
    drop(notes_raw);
    if let Some(w) = notes_warning {
        assets.warn(w.code, w.message);
    }

    // Titre, sous-titre (catégorie) et UUID.
    let mut replaced = Replaced::new();
    let title_src = metadata.get("title").filter(|v| truthy(Some(v)));
    let mut title = match title_src {
        Some(v) => display_text(v, "title", &mut replaced)?,
        None => String::new(),
    };
    if title.is_empty() {
        title = "Histoire".to_owned();
    }
    let subtitle = match metadata.get("category").filter(|v| truthy(Some(v))) {
        // `isinstance(v, (str, int, float))` : booléens compris (sous-classe de `int`).
        Some(v @ (Value::Str(_) | Value::Int(_) | Value::UInt(_) | Value::Float(_)))
        | Some(v @ Value::Bool(_)) => display_text(v, "category", &mut replaced)?,
        _ => String::new(),
    };
    let uuid = match metadata.get("uuid") {
        Some(Value::Str(u)) if !py_strip(u).is_empty() => md5::digest(&[u.as_bytes()]),
        _ => {
            // Sans uuid : empreinte du contenu (et non du titre, pour ne pas partager la
            // sauvegarde entre deux histoires de même titre).
            assets.warn(
                "uuid_derived",
                "metadata.json sans uuid : UUID derive du contenu de nodes.json \
                 (change si l'histoire est modifiee)"
                    .to_owned(),
            );
            md5::digest(&[b"telmi2flam:nodes.json:", &nodes_raw])
        }
    };
    drop(nodes_raw);

    // Graphe : stages, actions, inventaire (assets convertis à la demande).
    let src = Sources::new(&nodes)?;
    let (images_total, audio_total) = asset_totals(&src);
    assets.progress().begin(Phase::Images, images_total);
    assets.progress().begin(Phase::Audio, audio_total);
    let graph = convert_graph(&src, &notes, &mut replaced, &mut assets)?;

    // Titre illustré et vignette : `image` de metadata.json, sinon title.png, sinon cover.png
    // (un fichier vide passe au suivant, comme `or` en Python).
    check()?;
    let cover_name = match metadata.get("image") {
        Some(Value::Str(s)) if !s.is_empty() => Some(s.as_str()),
        _ => None,
    };
    if let Some(name) = cover_name {
        check_asset_name(name, "metadata.json image")?;
    }
    let mut cover = match cover_name {
        Some(name) => assets.read(name)?,
        None => None,
    };
    for fallback in ["title.png", "cover.png"] {
        if cover.as_ref().is_some_and(|c| !c.is_empty()) {
            break;
        }
        cover = assets.read(fallback)?;
    }
    let mut title_audio = None;
    if let Some((source, mp3)) = assets.read_named("title.mp3")? {
        assets.add_mp3map("sounds/title.mp3", mp3, "title.mp3", &source)?;
        assets.stats.audio += 1;
        title_audio = Some("title.mp3");
    }
    let mut title_image = None;
    if let Some(cover) = cover {
        check()?;
        // title-card place la couverture à taille native dans un emplacement d'environ
        // 89 × 120, ancré à gauche : une couverture portrait est complétée à droite.
        let converted = png_to_fit_lif(&cover, 89, 120, Align::Left, cancelled)
            .and_then(|t| Ok((t, png_to_thumb_lif(&cover, options.thumb_fit, cancelled)?)));
        match converted {
            Ok((title_lif, thumb_lif)) => {
                assets
                    .out
                    .insert("img/title.lif".to_owned(), title_lif.into());
                assets
                    .out
                    .insert("img/thumbnail.lif".to_owned(), thumb_lif.into());
                assets.stats.img += 2;
                title_image = Some("title.lif");
            }
            Err(PngError::Unreadable(why)) => {
                assets.warn(
                    "cover_unreadable",
                    format!("couverture illisible ({why}) : pas de titre illustre ni de vignette"),
                );
                assets
                    .bad
                    .push(cover_name.unwrap_or("title.png").to_owned());
            }
            Err(e) => return Err(image_error(cover_name.unwrap_or("title.png"), e)),
        }
    }

    if !replaced.is_empty() {
        assets.warn(
            "font_replaced",
            format!(
                "{} caractere(s) hors police (ASCII + Latin-1) remplace(s) dans le titre, \
                 le sous-titre ou les labels : {}",
                replaced.len(),
                describe_replaced(&replaced, 12)
            ),
        );
    }

    if !assets.missing.is_empty() {
        let shown: Vec<String> = assets
            .missing
            .iter()
            .take(5)
            .map(|Missing { kind, name }| format!("{kind} {name}"))
            .collect();
        let more = if assets.missing.len() > 5 { " ..." } else { "" };
        let msg = format!(
            "{} asset(s) manquant(s) : {}{more}",
            assets.missing.len(),
            shown.join(", ")
        );
        if !options.allow_missing {
            return Err(ConvError::missing_asset(format!(
                "{msg} (utiliser --allow-missing pour convertir quand meme)"
            )));
        }
        assets.warn(
            "assets_missing_allowed",
            format!("{msg} (--allow-missing : images omises, audios remplaces par silent.mp3)"),
        );
    }

    let start = Start::new(&src)?;
    let header = Header {
        title: &title,
        subtitle: &subtitle,
        title_image,
        title_audio,
        selector: options.selector.as_str(),
    };
    let nodes_lua = nodes_lua(&header, &graph, &start)?;
    let mut stats = Stats {
        stages: graph.stages.len(),
        actions: graph.actions.len(),
        img: assets.stats.img,
        audio: assets.stats.audio,
        mp3map: assets.stats.mp3map,
        resampled: assets.resampled,
        lib: RUNTIME.iter().filter(|f| f.name.ends_with(".lua")).count(),
        ui: RUNTIME.iter().filter(|f| f.name.ends_with(".lif")).count(),
        missing: assets.missing.len(),
        bad: assets.bad.len(),
    };
    drop(graph);

    let StoryAssets {
        out: mut files,
        warnings,
        progress,
        ..
    } = assets;
    // `scripts` : nodes.lua, main.lua et story.lua (comptés 3), puis chaque fichier du runtime.
    progress.begin(Phase::Scripts, 3 + RUNTIME.len() as u64);
    files.insert("script/nodes.lua".to_owned(), nodes_lua.into());
    progress.tick(Phase::Scripts);
    files.insert("main.lua".to_owned(), MAIN_LUA.to_vec().into());
    files.insert("script/story.lua".to_owned(), STORY_LUA.to_vec().into());
    progress.tick(Phase::Scripts);
    progress.tick(Phase::Scripts);
    for f in RUNTIME {
        files.insert(f.name.to_owned(), f.data.to_vec().into());
        progress.tick(Phase::Scripts);
    }
    // Couverture par défaut de audio-player : petite image transparente.
    let empty = lif::encode(&[0u8; 8 * 8 * 4], 8, 8)
        .map_err(|e| ConvError::internal_error(format!("img/empty.lif : {e}")))?;
    files.insert("img/empty.lif".to_owned(), empty.into());
    stats.img += 1;
    let silent = silent_mp3();
    if let Ok(map) = mp3map::build(&silent) {
        files.insert("sounds/silent.mp3map".to_owned(), map.data.into());
    }
    files.insert("sounds/silent.mp3".to_owned(), silent.into());
    // Métadonnées FLAM (titre / sous-titre / id / titre), lues ligne par ligne.
    let info_subtitle = if subtitle.is_empty() {
        &title
    } else {
        &subtitle
    };
    files.insert(
        "info.plain".to_owned(),
        format!("{title}\n{info_subtitle}\n000000\n{title}")
            .into_bytes()
            .into(),
    );
    files.insert("version".to_owned(), b"1".to_vec().into());
    files.insert("uuid.bin".to_owned(), uuid.to_vec().into());

    let entries = canonical(files);
    check_names(entries.iter().map(|(n, _)| n.as_str()))?;
    Ok(Package {
        entries,
        warnings,
        title,
        uuid,
        stats,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::Object;

    #[test]
    fn str_de_python() {
        let v = |s: &str| load_json(s.as_bytes(), "t").unwrap();
        for (json, expected) in [
            ("5", "5"),
            ("-0.0", "-0.0"),
            ("1e16", "1e+16"),
            ("true", "True"),
            ("[1, \"a'b\", null]", "[1, \"a'b\", None]"),
            ("{\"k\": [\"\\u00e9\\u00a0\\n\"]}", "{'k': ['é\\xa0\\n']}"),
            ("[\"\\\"'\"]", "['\"\\'']"),
            ("[]", "[]"),
        ] {
            assert_eq!(py_str(&v(json)).as_deref(), Some(expected), "{json}");
        }
        assert_eq!(py_str(&v("[\"\\u0100\"]")), None);
        assert_eq!(py_str(&v("\"\\u0100\"")).as_deref(), Some("\u{100}"));
    }

    #[test]
    fn ordre_canonique() {
        let mut files = std::collections::BTreeMap::new();
        for n in [
            "uuid.bin",
            "sounds/a.mp3",
            "version",
            "img/b.lif",
            "script/nodes.lua",
            "info.plain",
            "main.lua",
            "script/story.lua",
        ] {
            files.insert(n.to_owned(), Vec::<u8>::new());
        }
        let names: Vec<String> = canonical(files).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            [
                "info.plain",
                "version",
                "main.lua",
                "script/nodes.lua",
                "img/b.lif",
                "script/story.lua",
                "sounds/a.mp3",
                "uuid.bin"
            ]
        );
    }

    #[test]
    fn mp3_silencieux_et_sa_table() {
        let s = silent_mp3();
        assert_eq!(s.len(), 417 * 24);
        let map = mp3map::build(&s).unwrap();
        assert!(map.samplerates.iter().all(|&r| r == 44100));
        let _ = Object::new();
    }
}
