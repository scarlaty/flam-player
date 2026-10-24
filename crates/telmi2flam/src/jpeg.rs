//! Décodage JPEG vers RGBA avec la sémantique de `_open_rgba` (`tools/telmi2flam/telmi2flam.py`)
//! exécuté avec **Pillow 12.3.0** (libjpeg-turbo 3.1.4.1) : `Image.open`, `load()` puis
//! `convert("RGBA")` (#459, décision D7 de `PORT-RUST.md`).
//!
//! Deux étages :
//! 1. **Verdict** (image lue ou illisible), reproduit à la main : analyse de `JpegImageFile._open`
//!    (marqueurs lus en Python jusqu'au premier `SOS`, exceptions comprises), garde anti-bombe de
//!    `Image.open`, refus fatals de libjpeg-turbo sur la structure (`SOF` en double ou non pris en
//!    charge, marqueur réservé, `SOI` répété, longueurs de `DHT`, `DQT`, `DAC`, `DRI` et `SOS`,
//!    index et valeurs de `DAC`, index et validité des tables de Huffman utilisées, table de
//!    quantification absente (arithmétique compris ; pas en sans perte, comme libjpeg),
//!    composante inconnue, paramètres de progression, échantillonnage hors 1..4 ou non entier,
//!    côté au-delà de 65 500 px) et fin des données (fichier tronqué : `LOAD_TRUNCATED_IMAGES`
//!    reste faux dans `telmi2flam.py`). Les erreurs de libjpeg avant le premier `SOS` ne passent
//!    qu'après toute l'analyse de `_open` (elles surviennent au décodage). Une exception de
//!    `_open` rattrapée par `Image.open` sur des données qu'un greffon sans `_accept` peut lire
//!    (SPIDER, ou PCD : `PCD_` à l'octet 2048) est déléguée. Règle de fin, mesurée sur
//!    l'oracle : une image à un seul balayage est lue si un marqueur suit ses données (les blocs
//!    manquants sont mis à zéro par libjpeg) ; sans marqueur, illisible si des données manquent,
//!    lue si au moins 8 octets inutiles terminent le flux, déléguée entre les deux (libjpeg
//!    demande de 0 à 7 octets au-delà des données utiles selon l'image) ; un segment de tables
//!    coupé par la fin du fichier suspend libjpeg (image lue), sauf erreur dans ses octets
//!    présents. Une image à plusieurs balayages (progressive, ou composantes en balayages
//!    séparés) n'est lue que si `EOI` est atteint : `jpeg_start_decompress` consomme tout le
//!    fichier. En séquentiel, les tables de Huffman 0 et 1 absentes sont les tables
//!    standard (annexe K de T.81), comme `std_huff_tables` de libjpeg-turbo ; pas en progressif.
//! 2. **Pixels** : `zune-jpeg` (scalaire : fonctions `x86` et `neon` non compilées quand la crate
//!    est construite seule, `-p telmi2flam`, contrôlé par la CI ; `use_unsafe` ne coupe pas
//!    l'AVX2) décode
//!    un flux réécrit (seulement `SOF`, `DQT`, `DHT`, `DRI`, `SOS` et données, puis `EOI` ;
//!    paramètres de progression d'un `SOS` séquentiel remis à 0/63/0, que libjpeg ignore) en
//!    composantes brutes suréchantillonnées. La conversion de couleur suit le choix de
//!    libjpeg-turbo (`default_decompress_parms` : JFIF, puis Adobe, puis identifiants des
//!    composantes) et celle de Pillow (`CMYK;I` : polarité Adobe toujours supposée, puis
//!    `cmyk2rgb`). L'IDCT et le suréchantillonnage de `zune-jpeg` ne sont pas ceux de
//!    libjpeg-turbo : **écart borné à ±4 par composante** (D7, mesuré sur les vecteurs
//!    `tests/vectors/jpeg/`), seul écart admis par `parity.py` (règle `jpeg_bounded`).
//!
//! **Délégués** ([`PngError::Unsupported`], code 7, D4) : ce que libjpeg-turbo lit (ou peut lire)
//! mais pas le Rust — codages arithmétique (après le verdict de la structure) et sans perte,
//! refus de `zune-jpeg` (données corrompues, table inutilisée invalide, facteur 3, `DQT` de
//! précision 2…), plus de [`MAX_SCANS`] balayages, flux de tables seules suivi d'une image,
//! balayage unique sans marqueur final au verdict incertain, marqueur invalide dans une image à
//! redémarrages, polyglotte PCD ou SPIDER. Écarts assumés (PORT-RUST.md §4.4) : pixels des données
//! corrompues ou coupées puis terminées par un marqueur (libjpeg les met en gris, `zune-jpeg`
//! non) ; EXIF et MPO
//! malformés (exceptions de Pillow non reproduites). Annulation consultée avant et après chaque
//! décodage de `zune-jpeg` (qui ne rend pas la main pendant le décodage), puis à chaque ligne
//! de la conversion.

use crate::png::{PngError, Rgba, MAX_IMAGE_PIXELS};
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

type Result<T> = std::result::Result<T, PngError>;

/// `JPEG_MAX_DIMENSION` de libjpeg-turbo : au-delà, `JERR_IMAGE_TOO_BIG`.
const JPEG_MAX_DIMENSION: u32 = 65500;

const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const DQT: u8 = 0xDB;
const DHT: u8 = 0xC4;
const DRI: u8 = 0xDD;
const DNL: u8 = 0xDC;
const DAC: u8 = 0xCC;
const COM: u8 = 0xFE;
/// `APP0` « AVI1 » : `zune-jpeg` complète alors les tables de Huffman 0 et 1 absentes par les
/// tables standard (comme libjpeg-turbo en séquentiel).
const AVI1: &[u8] = b"\xff\xe0\x00\x07AVI1\x00";

/// Balayages au plus (premier compris) : au-delà, délégué (`JPEG refusé par zune-jpeg`, D4).
/// `zune-jpeg` ne rend pas la main pendant un décodage et libjpeg-turbo ne borne pas le nombre
/// de balayages : un JPEG progressif fait de milliers de balayages vides tiendrait le décodage
/// (et l'annulation) en otage. Seuil généreux : un JPEG progressif courant en compte une
/// dizaine ; seul un script extrême (un balayage par coefficient et par bit, jusqu'à quelque
/// 3 000 balayages) le dépasse, délégué alors (sens sûr).
const MAX_SCANS: usize = 1000;

fn unreadable<T>(why: &'static str) -> Result<T> {
    Err(PngError::Unreadable(why))
}

/// Exception de `JpegImageFile._open` que `Image.open` rattrape (`SyntaxError`, `IndexError`,
/// `struct.error`) : Pillow essaie alors les autres greffons. Pour des données commençant par
/// `FF D8 FF`, seuls SPIDER et PCD (sans `_accept` ; IM, IMT, IPTC et TGA rejettent `FF D8`)
/// peuvent les ouvrir : délégué comme un autre format (D4), sinon illisible.
fn open_error<T>(data: &[u8], why: &'static str) -> Result<T> {
    if crate::identify::spider_signature(data) || crate::identify::pcd_signature(data) {
        Err(PngError::Unsupported("pas un PNG"))
    } else {
        unreadable(why)
    }
}

/// Erreur fatale de libjpeg-turbo avant le premier `SOS` : elle ne survient qu'au décodage,
/// après toute l'analyse de `_open` (dont les exceptions priment) ; la première est retenue.
fn defer(fatal: &mut Option<PngError>, r: Result<()>) {
    if let Err(e) = r {
        fatal.get_or_insert(e);
    }
}

/// Espace de couleur du flux choisi par libjpeg-turbo (`jpeg_color_space`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Space {
    Gray,
    YCbCr,
    Rgb,
    Cmyk,
    Ycck,
}

/// Lecture séquentielle façon `BytesIO.read` (lectures courtes en fin de données).
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    /// Segment à longueur : `n = i16(read(2)) - 2` (`struct.error` sous 2 octets) puis
    /// `ImageFile._safe_read(fp, n)` : rien si `n <= 0` (flux non avancé), sinon exactement `n`
    /// octets ou `OSError("Truncated File Read")`. Rend la longueur lue et le corps.
    fn segment(&mut self) -> Result<(u16, &'a [u8])> {
        let Some(s) = self.data.get(self.pos..self.pos + 2) else {
            return open_error(self.data, "longueur de segment tronquée");
        };
        self.pos += 2;
        let len = u16::from_be_bytes([s[0], s[1]]);
        if len <= 2 {
            return Ok((len, &[]));
        }
        let end = self.pos + usize::from(len) - 2;
        let Some(s) = self.data.get(self.pos..end) else {
            return unreadable("Truncated File Read");
        };
        self.pos = end;
        Ok((len, s))
    }
}

/// Composante de la trame (`SOF`) : identifiant, échantillonnages, table de quantification.
#[derive(Debug, Clone, Copy)]
struct Component {
    id: u8,
    h: u8,
    v: u8,
    quant: u8,
}

/// Table de Huffman définie : codes valides (aucun code tout à un, `jpeg_make_d_derived_tbl`)
/// et symboles de 0 à 15 (exigé des tables DC).
#[derive(Debug, Clone, Copy)]
struct Huffman {
    codes: bool,
    dc_symbols: bool,
}

/// Tables définies pour libjpeg-turbo.
#[derive(Debug, Default)]
struct Tables {
    dc: [Option<Huffman>; 4],
    ac: [Option<Huffman>; 4],
    quant: [bool; 4],
}

impl Tables {
    // Les segments de tables sont lus par libjpeg octet par octet dans `avail` (tout ce qui suit
    // les deux octets de longueur, jusqu'à la fin du fichier) : une erreur trouvée dans ce qui
    // est présent passe d'abord, puis la fin du fichier suspend la lecture (`Ok(None)` ;
    // `Ok(Some(()))` quand le segment est complet).

    /// `get_dht` : comptes, index, longueur exacte.
    fn dht(&mut self, len: u16, avail: &[u8]) -> Result<Option<()>> {
        let mut length = i32::from(len) - 2;
        let mut s = avail;
        while length > 16 {
            let Some(head) = s.get(..17) else {
                return Ok(None);
            };
            let (index, bits) = (head[0], &head[1..]);
            let count: usize = bits.iter().map(|&b| usize::from(b)).sum();
            length -= 17;
            if count > 256 || count as i32 > length {
                return unreadable("JERR_BAD_HUFF_TABLE");
            }
            let Some(vals) = s.get(17..17 + count) else {
                return Ok(None);
            };
            s = &s[17 + count..];
            length -= count as i32;
            let (slots, index) = if index & 0x10 != 0 {
                (&mut self.ac, index - 0x10)
            } else {
                (&mut self.dc, index)
            };
            if index >= 4 {
                return unreadable("JERR_DHT_INDEX");
            }
            // jpeg_make_d_derived_tbl : après les codes de longueur l, code < 2^l
            let mut code = 0u32;
            let mut codes = true;
            for (l, &n) in bits.iter().enumerate() {
                code += u32::from(n);
                if n > 0 && code >= 1 << (l + 1) {
                    codes = false;
                }
                code <<= 1;
            }
            slots[usize::from(index)] = Some(Huffman {
                codes,
                dc_symbols: vals.iter().all(|&v| v <= 15),
            });
        }
        if length != 0 {
            return unreadable("JERR_BAD_LENGTH (DHT)");
        }
        Ok(Some(()))
    }

    /// `get_dqt` : index de la table, puis 64 valeurs toujours lues (2 octets dès que la
    /// précision n'est pas nulle, `Pq` = 2 compris), au-delà du segment s'il le faut : un reste
    /// plus court qu'une table donne `JERR_BAD_LENGTH` si le fichier contient ces 64 valeurs.
    fn dqt(&mut self, len: u16, avail: &[u8]) -> Result<Option<()>> {
        let mut length = i32::from(len) - 2;
        let mut s = avail;
        while length > 0 {
            let Some(&n) = s.first() else {
                return Ok(None);
            };
            if n & 15 >= 4 {
                return unreadable("JERR_DQT_INDEX");
            }
            let size = if n >> 4 != 0 { 128 } else { 64 };
            let Some(rest) = s.get(1 + size..) else {
                return Ok(None);
            };
            self.quant[usize::from(n & 15)] = true;
            s = rest;
            length -= 1 + size as i32;
        }
        if length != 0 {
            return unreadable("JERR_BAD_LENGTH (DQT)");
        }
        Ok(Some(()))
    }

    /// `get_dac` (lu quel que soit le codage) : paires index, valeur (lues toutes deux avant
    /// les contrôles, au-delà du segment pour une longueur impaire) ; index < 32 et, pour une
    /// table DC (index < 16), borne basse ≤ borne haute ; longueur paire.
    fn dac(len: u16, avail: &[u8]) -> Result<Option<()>> {
        let mut length = i32::from(len) - 2;
        let mut s = avail;
        while length > 0 {
            let Some(&[index, v]) = s.first_chunk::<2>() else {
                return Ok(None);
            };
            s = &s[2..];
            length -= 2;
            if index >= 32 {
                return unreadable("JERR_DAC_INDEX");
            }
            if index < 16 && v & 15 > v >> 4 {
                return unreadable("JERR_DAC_VALUE");
            }
        }
        if length != 0 {
            return unreadable("JERR_BAD_LENGTH (DAC)");
        }
        Ok(Some(()))
    }

    /// `get_dri` : longueur 4 exigée avant la lecture de l'intervalle.
    fn dri(len: u16, avail: &[u8]) -> Result<Option<u16>> {
        if len != 4 {
            return unreadable("JERR_BAD_LENGTH (DRI)");
        }
        Ok(avail.first_chunk::<2>().map(|&v| u16::from_be_bytes(v)))
    }

    /// `std_huff_tables` (séquentiel) : tables 0 et 1 absentes remplacées par celles de T.81.
    fn standard(&mut self) {
        let std = Huffman {
            codes: true,
            dc_symbols: true,
        };
        for slot in self.dc[..2].iter_mut().chain(self.ac[..2].iter_mut()) {
            slot.get_or_insert(std);
        }
    }

    /// Table `tbl` d'une classe utilisée par un balayage : présente et valide.
    fn require(&self, dc: bool, tbl: u8) -> Result<()> {
        let slot = if dc { &self.dc } else { &self.ac };
        match slot.get(usize::from(tbl)).copied().flatten() {
            None => unreadable("JERR_NO_HUFF_TABLE"),
            Some(t) if !t.codes || (dc && !t.dc_symbols) => unreadable("JERR_BAD_HUFF_TABLE"),
            Some(_) => Ok(()),
        }
    }
}

/// Ce que `_open` de Pillow et l'en-tête de libjpeg-turbo retiennent jusqu'au premier `SOS`.
#[derive(Debug, Default)]
struct Header {
    width: u32,
    height: u32,
    /// Marqueur `SOF` (octet bas) et composantes, de libjpeg (un seul `SOF` admis).
    sof: Option<u8>,
    components: Vec<Component>,
    /// `saw_JFIF_marker` et `Adobe_transform` de libjpeg (le dernier `APP14` gagne).
    jfif: bool,
    adobe: Option<u8>,
    /// Intervalle de redémarrage en vigueur (dernier `DRI`).
    restart: u16,
    tables: Tables,
    /// Flux pour `zune-jpeg` jusqu'au premier balayage compris (en-tête du `SOS`).
    stream: Vec<u8>,
    /// Composantes du premier balayage et position des données qui le suivent.
    scan_components: usize,
    entropy: usize,
    /// Balayages vus (`SOS`), premier compris.
    scans: usize,
}

impl Header {
    /// Progressif, Huffman (`SOF2`) ou arithmétique (`SOF10`) : mêmes contrôles de progression.
    fn progressive(&self) -> bool {
        matches!(self.sof, Some(0xC2 | 0xCA))
    }

    /// Codage de Huffman à DCT (`SOF0` à `SOF2`), seul lu par `zune-jpeg`.
    fn huffman(&self) -> bool {
        matches!(self.sof, Some(0xC0..=0xC2))
    }
}

/// `APP` de Pillow : JFIF et Adobe lisent `i16(s, 5)` (`struct.error` sous 7 octets) ; bloc
/// Photoshop parcouru (seul `struct.error` y est rattrapé : un `IndexError` remonte).
fn pillow_app(data: &[u8], marker: u8, s: &[u8]) -> Result<()> {
    if (marker == 0xE0 && s.starts_with(b"JFIF") || marker == 0xEE && s.starts_with(b"Adobe"))
        && s.len() < 7
    {
        return open_error(data, "segment JFIF ou Adobe trop court");
    }
    if marker == 0xED && s.starts_with(b"Photoshop 3.0\0") {
        let mut offset = 14usize;
        while s.get(offset..).and_then(|t| t.get(..4)) == Some(b"8BIM") {
            offset += 4;
            let Some(code) = s.get(offset..offset + 2) else {
                break; // i16 : struct.error rattrapé
            };
            let code = u16::from_be_bytes([code[0], code[1]]);
            offset += 2;
            let Some(&name_len) = s.get(offset) else {
                return open_error(data, "bloc Photoshop tronqué"); // IndexError
            };
            offset += 1 + usize::from(name_len);
            offset += offset & 1;
            let Some(size) = s.get(offset..offset + 4) else {
                break; // i32 : struct.error rattrapé
            };
            let size = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
            offset += 4;
            // `ResolutionInfo` (0x03ED) lu jusqu'à l'octet 14 : court, struct.error rattrapé
            // par le `try` extérieur, qui arrête la boucle
            if code == 0x03ED && size.min(s.len().saturating_sub(offset)) < 14 {
                break;
            }
            let Some(next) = offset.checked_add(size) else {
                break;
            };
            offset = next.saturating_add(next & 1);
        }
    }
    Ok(())
}

/// En-tête d'un `SOS` (corps sans la longueur) : `get_sos` de libjpeg (longueur, composantes
/// connues et distinctes), progression (`start_pass_phuff_decoder`), tables de Huffman utilisées
/// et tables de quantification des composantes vues pour la première fois. Copie l'en-tête dans
/// `out` (paramètres de progression remis à 0/63/0 en séquentiel). Rend le nombre de composantes.
fn scan_header(
    h: &Header,
    len: u16,
    s: &[u8],
    latched: &mut [bool],
    out: &mut Vec<u8>,
) -> Result<usize> {
    let n = usize::from(*s.first().unwrap_or(&0));
    if usize::from(len) != n * 2 + 6 || !(1..=4).contains(&n) {
        return unreadable("JERR_BAD_LENGTH (SOS)");
    }
    let (ss, se, ah, al) = (
        s[1 + 2 * n],
        s[2 + 2 * n],
        s[3 + 2 * n] >> 4,
        s[3 + 2 * n] & 15,
    );
    let progressive = h.progressive();
    if progressive {
        let dc_band = ss == 0;
        let bad = if dc_band {
            se != 0
        } else {
            ss > se || se >= 64 || n != 1
        } || ah != 0 && al != ah.wrapping_sub(1)
            || al > 13;
        if bad {
            return unreadable("JERR_BAD_PROGRESSION");
        }
    }
    let mut seen = Vec::with_capacity(n);
    for c in s[1..1 + 2 * n].as_chunks::<2>().0 {
        let Some(k) = h.components.iter().position(|k| k.id == c[0]) else {
            return unreadable("JERR_BAD_COMPONENT_ID");
        };
        if seen.contains(&k) {
            return unreadable("JERR_BAD_COMPONENT_ID");
        }
        seen.push(k);
        let (dc, ac) = (c[1] >> 4, c[1] & 15);
        // sans perte (délégué) : ni tables de quantification (`latch_quant_tables` sauté par
        // libjpeg), ni tables de Huffman vérifiées ici
        if matches!(h.sof, Some(0xC3 | 0xCB)) {
            continue;
        }
        // arithmétique : tables 0 à 15 toutes valables (`NUM_ARITH_TBLS`), rien à vérifier
        if h.huffman() {
            if !progressive {
                h.tables.require(true, dc)?;
                h.tables.require(false, ac)?;
            } else if ss == 0 && ah == 0 {
                h.tables.require(true, dc)?;
            } else if ss != 0 {
                h.tables.require(false, ac)?;
            }
        }
        // `latch_quant_tables`
        if !latched[k] {
            if !h.tables.quant[usize::from(h.components[k].quant & 3)] || h.components[k].quant >= 4
            {
                return unreadable("JERR_NO_QUANT_TABLE");
            }
            latched[k] = true;
        }
    }
    // `per_scan_setup` : au plus 10 blocs par MCU d'un balayage entrelacé (JERR_BAD_MCU_SIZE)
    let blocks: usize = seen
        .iter()
        .map(|&k| usize::from(h.components[k].h) * usize::from(h.components[k].v))
        .sum();
    if n > 1 && blocks > 10 {
        return unreadable("JERR_BAD_MCU_SIZE");
    }
    out.extend_from_slice(&[0xFF, SOS]);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&s[..1 + 2 * n]);
    if progressive {
        out.extend_from_slice(&s[1 + 2 * n..]);
    } else {
        out.extend_from_slice(&[0, 63, 0]);
    }
    Ok(n)
}

/// Analyse des marqueurs jusqu'au premier `SOS` : `JpegImageFile._open` de Pillow (exceptions
/// comprises), puis contrôles fatals de libjpeg-turbo sur les mêmes segments.
fn parse_header(data: &[u8], latched: &mut Vec<bool>) -> Result<Header> {
    if !data.starts_with(b"\xff\xd8\xff") {
        return unreadable("not a JPEG file");
    }
    let mut r = Reader { data, pos: 3 };
    let mut fatal: Option<PngError> = None;
    let mut h = Header::default();
    h.stream.extend_from_slice(&[0xFF, SOI]);
    // Profils ICC vus depuis le dernier SOF (`self.icclist`).
    let mut icc: Vec<&[u8]> = Vec::new();
    let mut pillow_size: Option<(u32, u32)> = None;
    let mut cur = Some(0xFFu8); // s = b"\xff"
    loop {
        // `s[0]` sur `b""` : IndexError
        let Some(b0) = cur else {
            return open_error(data, "fin des données avant SOS");
        };
        if b0 != 0xFF {
            cur = r.byte(); // octets parasites
            continue;
        }
        // `i16(s + read(1))` : struct.error en fin de données
        let Some(m) = r.byte() else {
            return open_error(data, "fin des données dans un marqueur");
        };
        match m {
            0xFF => {
                cur = Some(0xFF); // remplissage
                continue;
            }
            0x00 => {
                cur = r.byte();
                continue;
            }
            0x01..=0xBF => return open_error(data, "no marker found"),
            _ => {}
        }
        let start = r.pos - 2;
        match m {
            // SOFn et DHP : gestionnaire `SOF` de Pillow
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xDE => {
                let (_, s) = r.segment()?;
                if s.len() < 6 {
                    return open_error(data, "SOF trop court"); // struct.error / IndexError
                }
                let height = u32::from(u16::from_be_bytes([s[1], s[2]]));
                let width = u32::from(u16::from_be_bytes([s[3], s[4]]));
                pillow_size = Some((width, height));
                if s[0] != 8 {
                    return open_error(data, "cannot handle non 8-bit layers");
                }
                let layers = usize::from(s[5]);
                if ![1, 3, 4].contains(&layers) {
                    return open_error(data, "cannot handle this number of layers");
                }
                // `self.icclist[0][13]` après tri : IndexError si le plus petit est court
                if icc.iter().min().is_some_and(|first| first.len() <= 13) {
                    return open_error(data, "profil ICC tronqué");
                }
                icc.clear();
                // `t[1]`, `t[2]` d'un triplet incomplet : IndexError
                if (s.len() - 6) % 3 != 0 {
                    return open_error(data, "composante de SOF incomplète");
                }
                // libjpeg-turbo : un seul SOF, types pris en charge, longueur exacte
                let refused = if !matches!(m, 0xC0..=0xC3 | 0xC9..=0xCB) {
                    unreadable("JERR_SOF_UNSUPPORTED")
                } else if h.sof.is_some() {
                    unreadable("JERR_SOF_DUPLICATE")
                } else if s.len() != 6 + 3 * layers {
                    unreadable("JERR_BAD_LENGTH (SOF)")
                } else {
                    Ok(())
                };
                if refused.is_err() {
                    defer(&mut fatal, refused);
                    cur = r.byte();
                    continue;
                }
                h.sof = Some(m);
                h.width = width;
                h.height = height;
                h.components = s[6..]
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|t| Component {
                        id: t[0],
                        h: t[1] >> 4,
                        v: t[1] & 15,
                        quant: t[2],
                    })
                    .collect();
                h.stream.extend_from_slice(&data[start..r.pos]);
            }
            DHT => {
                let (len, _) = r.segment()?;
                defer(
                    &mut fatal,
                    h.tables.dht(len, &data[start + 4..]).map(|_| ()),
                );
                h.stream.extend_from_slice(&data[start..r.pos]);
            }
            DQT => {
                let (len, s) = r.segment()?;
                // `DQT` de Pillow : table plus courte que sa précision
                let mut t = s;
                while let Some(&v) = t.first() {
                    let n = 1 + if v >> 4 == 0 { 64 } else { 128 };
                    if t.len() < n {
                        return open_error(data, "bad quantization table marker");
                    }
                    t = &t[n..];
                }
                defer(
                    &mut fatal,
                    h.tables.dqt(len, &data[start + 4..]).map(|_| ()),
                );
                h.stream.extend_from_slice(&data[start..r.pos]);
            }
            DRI => {
                let (len, s) = r.segment()?;
                if len != 4 {
                    defer(&mut fatal, unreadable("JERR_BAD_LENGTH (DRI)"));
                } else {
                    h.restart = u16::from_be_bytes([s[0], s[1]]);
                    h.stream.extend_from_slice(&data[start..r.pos]);
                }
            }
            0xE0..=0xEF => {
                let (_, s) = r.segment()?;
                pillow_app(data, m, s)?;
                // libjpeg : `examine_app0` (14 octets) et `examine_app14` (12 octets)
                if m == 0xE0 && s.len() >= 14 && s.starts_with(b"JFIF\0") {
                    h.jfif = true;
                }
                if m == 0xE2 && s.starts_with(b"ICC_PROFILE\0") {
                    icc.push(s);
                }
                if m == 0xEE && s.len() >= 12 && s.starts_with(b"Adobe") {
                    h.adobe = Some(s[11]);
                }
            }
            COM | DNL => {
                r.segment()?;
            }
            DAC => {
                let (len, _) = r.segment()?;
                defer(&mut fatal, Tables::dac(len, &data[start + 4..]).map(|_| ()));
            }
            // EXP : réservé pour libjpeg
            0xDF => {
                r.segment()?;
                defer(&mut fatal, unreadable("JERR_UNKNOWN_MARKER"));
            }
            SOS => {
                let (len, s) = r.segment()?;
                let Some((width, height)) = pillow_size else {
                    return open_error(data, "SOS sans SOF");
                };
                // `Image.open` : mode vide ou taille nulle, puis garde anti-bombe
                if width == 0 || height == 0 {
                    return open_error(data, "not identified by this driver");
                }
                if u64::from(width) * u64::from(height) > 2 * MAX_IMAGE_PIXELS {
                    return unreadable("décompression bombe (plus de 2 x MAX_IMAGE_PIXELS)");
                }
                if let Some(e) = fatal {
                    return Err(e);
                }
                if h.sof.is_none() {
                    return unreadable("JERR_SOF_BEFORE");
                }
                if !h.progressive() {
                    h.tables.standard();
                }
                latched.resize(h.components.len(), false);
                let mut stream = std::mem::take(&mut h.stream);
                h.scan_components = scan_header(&h, len, s, latched, &mut stream)?;
                h.stream = stream;
                h.entropy = r.pos;
                h.scans = 1;
                return Ok(h);
            }
            // libjpeg : SOI répété (JERR_SOI_DUPLICATE), JPG et JPGn réservés ; EOI avant
            // l'image (flux de tables seules suivi d'une autre image, que libjpeg lit) : non
            // pris en charge, délégué. Pillow passe ces marqueurs sans paramètre.
            SOI => defer(&mut fatal, unreadable("JERR_SOI_DUPLICATE")),
            // une nouvelle image doit suivre immédiatement (`first_marker` : JERR_NO_SOI)
            EOI if data[r.pos..].starts_with(b"\xff\xd8") => defer(
                &mut fatal,
                Err(PngError::Unsupported("JPEG à tables séparées")),
            ),
            EOI => defer(&mut fatal, unreadable("JERR_NO_SOI")),
            0xC8 | 0xF0..=0xFD => defer(&mut fatal, unreadable("JERR_UNKNOWN_MARKER")),
            // RSTn hors balayage : sans paramètre, ignoré par les deux
            _ => {}
        }
        cur = r.byte();
    }
}

/// Fin des données compressées d'un balayage commençant à `pos` : position du `0xFF` et octet
/// bas du marqueur qui la termine (`RSTn` sautés si `skip_rst`), ou `None` en fin de données.
fn entropy_end(data: &[u8], mut pos: usize, skip_rst: bool, resync: bool) -> Option<(usize, u8)> {
    // Avec redémarrages, un marqueur invalide (< SOF0) est écarté par `jpeg_resync_to_restart`
    // au redémarrage suivant : il ne termine les données que si aucun `RSTn` ne le suit (il
    // reste alors à lire après le balayage, et libjpeg le refuse).
    let mut pending: Option<(usize, u8)> = None;
    loop {
        let Some(ff) = data
            .get(pos..)
            .and_then(|t| t.iter().position(|&b| b == 0xFF))
            .map(|i| pos + i)
        else {
            return pending;
        };
        let mut q = ff + 1;
        while data.get(q) == Some(&0xFF) {
            q += 1; // remplissage
        }
        let Some(&m) = data.get(q) else {
            return pending;
        };
        if m == 0x00 {
            pos = q + 1;
        } else if skip_rst && (0xD0..=0xD7).contains(&m) {
            pending = None;
            pos = q + 1;
        } else if resync && m < 0xC0 {
            pending = pending.or(Some((q - 1, m)));
            pos = q + 1;
        } else {
            return pending.or(Some((q - 1, m)));
        }
    }
}

/// Recopie des données compressées `data[start..end]` pour `zune-jpeg`, normalisées comme les
/// lit libjpeg : `FF` de remplissage retirés (`FF FF 00` est l'octet `FF`, `FF FF D0` un
/// `RST0`, des `FF` en fin de plage précèdent le marqueur final), marqueurs invalides (< SOF0,
/// hors `FF 00`) retirés avec leurs `FF`, libjpeg les écartant en resynchronisant et
/// `zune-jpeg` les refusant (pixels de ces images corrompues non comparés).
fn copy_entropy(data: &[u8], start: usize, end: usize, out: &mut Vec<u8>) {
    let mut i = start;
    while i < end {
        let b = data[i];
        if b != 0xFF {
            out.push(b);
            i += 1;
            continue;
        }
        let mut q = i + 1;
        while q < end && data[q] == 0xFF {
            q += 1;
        }
        match data[..end].get(q) {
            // remplissage avant le marqueur final, ou marqueur invalide
            None | Some(0x01..=0xBF) => {}
            // FF 00 ou RSTn
            Some(&m) => out.extend_from_slice(&[0xFF, m]),
        }
        i = q + 1;
    }
}

/// Marqueur de segment suivant (`next_marker` de libjpeg : octets parasites sautés) : position
/// juste après le marqueur et son octet bas, ou `None` en fin de données.
fn segment_marker(data: &[u8], mut pos: usize) -> Option<(usize, u8)> {
    loop {
        let ff = pos + data.get(pos..)?.iter().position(|&b| b == 0xFF)?;
        let mut q = ff + 1;
        while *data.get(q)? == 0xFF {
            q += 1;
        }
        if data[q] != 0x00 {
            return Some((q + 1, data[q]));
        }
        pos = q + 1;
    }
}

/// Parcours des données après le premier `SOS` : verdict de libjpeg-turbo sur la suite du flux
/// et flux pour `zune-jpeg` (balayages et tables, sans `APPn` ni `COM`). Rend `true` pour un
/// balayage unique sans aucun marqueur après ses données (recopiées jusqu'à la fin) : libjpeg
/// ne le lit que si assez d'octets suivent les dernières données utiles (`unterminated`).
fn walk_scans(data: &[u8], h: &mut Header, latched: &mut [bool], multi_scan: bool) -> Result<bool> {
    let truncated = || unreadable("image file is truncated");
    // premier balayage : données jusqu'au marqueur qui le termine (RSTn attendus sautés)
    let skip_rst = multi_scan || h.restart != 0;
    let Some((end, mut m)) = entropy_end(data, h.entropy, skip_rst, h.restart != 0) else {
        if multi_scan {
            return truncated();
        }
        copy_entropy(data, h.entropy, data.len(), &mut h.stream);
        return Ok(true);
    };
    copy_entropy(data, h.entropy, end, &mut h.stream);
    let mut pos = end + 2;
    loop {
        match m {
            EOI => return Ok(false),
            SOS if !multi_scan => return unreadable("JERR_EOI_EXPECTED"),
            // tables : lues par libjpeg octet par octet, erreurs dans ce qui est présent d'abord
            DHT | DQT | DRI | DAC => {
                // fin du fichier : un seul balayage, toutes les lignes sont lues, libjpeg attend
                // en vain la suite et Pillow s'en contente ; sinon tronqué
                let suspended = || if multi_scan { truncated() } else { Ok(false) };
                let Some(&l) = data.get(pos..).and_then(|t| t.first_chunk::<2>()) else {
                    return suspended();
                };
                let len = u16::from_be_bytes(l);
                let avail = &data[pos + 2..];
                let done = match m {
                    DHT => h.tables.dht(len, avail)?,
                    DQT => h.tables.dqt(len, avail)?,
                    DAC => Tables::dac(len, avail)?,
                    _ => Tables::dri(len, avail)?.map(|restart| h.restart = restart),
                };
                if done.is_none() {
                    return suspended();
                }
                // segment complet : longueur ≥ 2 et tous ses octets présents
                let end = pos + usize::from(len);
                if multi_scan && m != DAC {
                    h.stream.extend_from_slice(&data[pos - 2..end]);
                }
                pos = end;
            }
            SOS | DNL | COM | 0xE0..=0xEF => {
                let mut r = Reader { data, pos };
                let (len, s) = match r.segment() {
                    Ok(seg) => seg,
                    // un seul balayage : toutes les lignes sont lues, libjpeg attend en vain la
                    // fin du fichier et Pillow s'en contente
                    Err(_) if !multi_scan => return Ok(false),
                    Err(_) => return truncated(),
                };
                pos = r.pos;
                if m == SOS {
                    h.scans += 1;
                    let mut stream = std::mem::take(&mut h.stream);
                    let scanned = scan_header(h, len, s, latched, &mut stream);
                    h.stream = stream;
                    scanned?;
                    let Some((end, next)) = entropy_end(data, pos, true, h.restart != 0) else {
                        return truncated();
                    };
                    copy_entropy(data, pos, end, &mut h.stream);
                    pos = end + 2;
                    m = next;
                    continue;
                }
            }
            0xD0..=0xD7 | 0x01 => {} // RSTn et TEM, sans paramètre : ignorés
            // marqueur invalide après le dernier RSTn : libjpeg le lit comme un redémarrage
            // (resynchronisation) s'il tombe en fin d'intervalle, sinon le refuse après le
            // balayage ; sans décodage entropique, seul libjpeg tranche (délégué, D4)
            0x02..=0xBF if h.restart != 0 => {
                return Err(PngError::Unsupported("JPEG corrompu (redémarrage)"))
            }
            _ => return unreadable("marqueur inattendu après le premier balayage"),
        }
        match segment_marker(data, pos) {
            Some((at, next)) => {
                pos = at;
                m = next;
            }
            None if multi_scan => return truncated(),
            None => return Ok(false),
        }
    }
}

/// YCbCr → RGB de JFIF en virgule fixe 16 bits, arrondi au plus proche.
fn ycc_to_rgb(y: u8, cb: u8, cr: u8) -> [u8; 3] {
    const ONE_HALF: i32 = 1 << 15;
    let (y, cb, cr) = (i32::from(y), i32::from(cb) - 128, i32::from(cr) - 128);
    let clamp = |v: i32| v.clamp(0, 255) as u8;
    [
        clamp(y + ((91881 * cr + ONE_HALF) >> 16)),
        clamp(y + ((-22554 * cb - 46802 * cr + ONE_HALF) >> 16)),
        clamp(y + ((116130 * cb + ONE_HALF) >> 16)),
    ]
}

/// `MULDIV255` de Pillow.
fn muldiv255(a: i32, b: i32) -> i32 {
    let t = a * b + 128;
    ((t >> 8) + t) >> 8
}

/// CMYK de libjpeg (polarité Adobe) → RGBA : `CMYK;I` (inversion) puis `cmyk2rgb` de Pillow.
fn cmyk_to_rgba([c, m, y, k]: [u8; 4]) -> [u8; 4] {
    let nk = i32::from(k); // 255 - (255 - k)
    let f = |v: u8| (nk - muldiv255(255 - i32::from(v), nk)).clamp(0, 255) as u8;
    [f(c), f(m), f(y), 255]
}

/// Espace de couleur de libjpeg-turbo (`default_decompress_parms`).
fn libjpeg_space(h: &Header) -> Space {
    match h.components.len() {
        1 => Space::Gray,
        3 if h.jfif => Space::YCbCr,
        3 => match h.adobe {
            Some(0) => Space::Rgb,
            Some(_) => Space::YCbCr,
            None if h.components.iter().map(|c| c.id).eq([82, 71, 66]) => Space::Rgb,
            None => Space::YCbCr,
        },
        _ => match h.adobe {
            Some(0) | None => Space::Cmyk,
            Some(_) => Space::Ycck,
        },
    }
}

/// Décode un JPEG en RGBA comme `_open_rgba(data)` avec Pillow 12.3.0 (à l'écart D7 près).
pub fn decode_rgba(data: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    if cancelled() {
        return Err(PngError::Cancelled);
    }
    let mut latched = Vec::new();
    let mut h = parse_header(data, &mut latched)?;
    // sans perte : délégué dès l'en-tête (contrôles de libjpeg propres à ce mode non reproduits)
    match h.sof {
        Some(0xCB) => return Err(PngError::Unsupported("JPEG arithmétique")),
        Some(0xC3) => return Err(PngError::Unsupported("JPEG sans perte")),
        _ => {}
    }
    if h.width > JPEG_MAX_DIMENSION || h.height > JPEG_MAX_DIMENSION {
        return unreadable("JERR_IMAGE_TOO_BIG");
    }
    // libjpeg : facteurs 1..4 (`JERR_BAD_SAMPLING`), rapports entiers (`int_upsample`)
    let (hmax, vmax) = h
        .components
        .iter()
        .fold((0, 0), |(a, b), c| (a.max(c.h), b.max(c.v)));
    for c in &h.components {
        if !(1..=4).contains(&c.h) || !(1..=4).contains(&c.v) {
            return unreadable("JERR_BAD_SAMPLING");
        }
        if h.components.len() > 1 && (hmax % c.h != 0 || vmax % c.v != 0) {
            return unreadable("JERR_FRACT_SAMPLE_NOTIMPL");
        }
    }
    let multi_scan = h.progressive() || h.scan_components < h.components.len();
    let mut entropy_start = h.stream.len();
    let unterminated = walk_scans(data, &mut h, &mut latched, multi_scan)?;
    // arithmétique : verdict de la structure rendu, pixels hors de portée de `zune-jpeg`
    if !h.huffman() {
        return Err(PngError::Unsupported("JPEG arithmétique"));
    }
    if h.scans > MAX_SCANS {
        return Err(PngError::Unsupported("JPEG refusé par zune-jpeg"));
    }
    if !h.progressive() {
        // tables standard de zune-jpeg pour les tables 0 et 1 absentes, juste après SOI
        h.stream.splice(2..2, AVI1.iter().copied());
        entropy_start += AVI1.len();
    }
    let stream = std::mem::take(&mut h.stream);
    if cancelled() {
        return Err(PngError::Cancelled);
    }

    let space = libjpeg_space(&h);
    let (w, ht) = (h.width as usize, h.height as usize);
    let channels = h.components.len();
    // flux privé de ses `cut` derniers octets de données, suivi de `fill` puis EOI, décodé en
    // mode strict ou non (`None` : rien à couper)
    let decode_with = |cut: usize, fill: &[u8], strict: bool| -> Option<Result<Vec<u8>>> {
        let end = stream
            .len()
            .checked_sub(cut)
            .filter(|&e| e >= entropy_start)?;
        let mut s = Vec::new();
        if s.try_reserve_exact(end + fill.len() + 2).is_err() {
            return Some(unreadable("mémoire insuffisante (flux JPEG)"));
        }
        s.extend_from_slice(&stream[..end]);
        s.extend_from_slice(fill);
        s.extend_from_slice(&[0xFF, EOI]);
        Some(decode_raw(&s, channels, w, ht, strict))
    };
    let raw = decode_with(0, &[], false).unwrap_or_else(|| unreadable("flux JPEG vide"))?;
    let same = |r: Option<Result<Vec<u8>>>| matches!(r, Some(Ok(r)) if r == raw);
    if unterminated {
        // Balayage unique sans marqueur final : libjpeg-turbo attend des octets au-delà des
        // données utiles (0 à 7 mesurés sur l'oracle, selon l'image), sinon Pillow lève « image
        // file is truncated ». Données manquantes (image changée par des bits 1 au lieu des 0
        // de zune-jpeg après la fin) : tronqué ; 8 derniers octets inutiles : lu ; entre les
        // deux, seul libjpeg tranche (délégué, D4). Inutiles : image inchangée qu'on les
        // remplace par des bits 0, ou par des bits 1 en mode strict (des octets utiles à bits
        // nuls décodés depuis des bits 1 donnent un code de Huffman invalide, que le mode
        // tolérant passe en laissant des blocs vides, parfois identiques).
        let ones = [0xFF, 0x00].repeat(8);
        if cancelled() {
            return Err(PngError::Cancelled);
        }
        if !same(decode_with(0, &ones, false)) {
            return unreadable("image file is truncated");
        }
        for (fill, strict) in [(&[][..], false), (&ones[..], true)] {
            if cancelled() {
                return Err(PngError::Cancelled);
            }
            if !same(decode_with(8, fill, strict)) {
                return Err(PngError::Unsupported("JPEG sans marqueur final"));
            }
        }
    }
    drop(stream);
    if cancelled() {
        return Err(PngError::Cancelled);
    }
    let mut rgba = Vec::new();
    rgba.try_reserve_exact(w * ht * 4)
        .map_err(|_| PngError::Unreadable("mémoire insuffisante (RGBA)"))?;
    for row in raw.chunks_exact(w * channels) {
        if cancelled() {
            return Err(PngError::Cancelled);
        }
        match space {
            Space::Gray => rgba.extend(row.iter().flat_map(|&l| [l, l, l, 255])),
            Space::Rgb => rgba.extend(
                row.as_chunks::<3>()
                    .0
                    .iter()
                    .flat_map(|&[r, g, b]| [r, g, b, 255]),
            ),
            Space::YCbCr => rgba.extend(row.as_chunks::<3>().0.iter().flat_map(|&[y, cb, cr]| {
                let [r, g, b] = ycc_to_rgb(y, cb, cr);
                [r, g, b, 255]
            })),
            Space::Cmyk => {
                rgba.extend(row.as_chunks::<4>().0.iter().flat_map(|&p| cmyk_to_rgba(p)))
            }
            Space::Ycck => rgba.extend(row.as_chunks::<4>().0.iter().flat_map(|&[y, cb, cr, k]| {
                // `ycck_cmyk_convert` de libjpeg : C = 255 - R, M = 255 - G, Y = 255 - B
                let [r, g, b] = ycc_to_rgb(y, cb, cr);
                cmyk_to_rgba([255 - r, 255 - g, 255 - b, k])
            })),
        }
    }
    Ok(Rgba {
        width: h.width,
        height: h.height,
        rgba,
    })
}

/// Composantes brutes suréchantillonnées (`channels` octets par pixel) décodées par
/// `zune-jpeg` en scalaire. Un refus de `zune-jpeg` est **délégué** (D4) : la structure a passé
/// les contrôles de Pillow et de libjpeg, et libjpeg-turbo poursuit souvent là où `zune-jpeg`
/// s'arrête (données corrompues, table inutilisée invalide, facteur 3…). Mode strict
/// seulement pour un contrôle de fin de flux (`decode_rgba`).
fn decode_raw(stream: &[u8], channels: usize, w: usize, h: usize, strict: bool) -> Result<Vec<u8>> {
    let options = DecoderOptions::default()
        .set_use_unsafe(false)
        .set_strict_mode(strict)
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX))
        // nombre de balayages déjà borné par `MAX_SCANS` (compté comme libjpeg)
        .jpeg_set_max_scans(usize::MAX);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(stream), options);
    let refused = |_| PngError::Unsupported("JPEG refusé par zune-jpeg");
    decoder.decode_headers().map_err(refused)?;
    // sortie sans conversion de couleur : l'espace d'entrée de zune-jpeg lui-même
    let raw_space = match (channels, decoder.input_colorspace()) {
        (1, _) => ColorSpace::Luma,
        (3, Some(ColorSpace::RGB)) => ColorSpace::RGB,
        (3, _) => ColorSpace::YCbCr,
        _ => ColorSpace::CMYK,
    };
    decoder.set_options(options.jpeg_set_out_colorspace(raw_space));
    let raw = decoder.decode().map_err(refused)?;
    if raw.len() != w * h * channels || decoder.dimensions() != Some((w, h)) {
        return Err(PngError::Unsupported("JPEG refusé par zune-jpeg"));
    }
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never() -> bool {
        false
    }

    #[test]
    fn conversions_de_couleur() {
        assert_eq!(ycc_to_rgb(128, 128, 128), [128, 128, 128]);
        // R = 255 + 178 écrêté, G = 255 - 47, B = 255 - 227 (décalages arithmétiques)
        assert_eq!(ycc_to_rgb(255, 0, 255), [255, 208, 28]);
        // CMYK Adobe : k = 255 (aucune encre noire), c = 255 (aucun cyan) → blanc
        assert_eq!(cmyk_to_rgba([255, 255, 255, 255]), [255, 255, 255, 255]);
        assert_eq!(cmyk_to_rgba([0, 0, 0, 0]), [0, 0, 0, 255]);
    }

    #[test]
    fn entetes_refuses_comme_pillow() {
        for d in [
            &b"\xff\xd8\xff"[..],
            b"\xff\xd8\xff\xe0",
            b"\xff\xd8\xff\xe0\x00\x10JFIF",
            b"\xff\xd8\xff\x01\x00\x00",
            b"\xff\xd8\xff\xd9",
        ] {
            assert!(
                matches!(decode_rgba(d, &never), Err(PngError::Unreadable(_))),
                "{d:?}"
            );
        }
    }

    #[test]
    fn tables_de_huffman_comme_libjpeg() {
        let mut t = Tables::default();
        // deux codes de longueur 1 : le second est « tout à un »
        let mut bad = vec![0x00, 2];
        bad.extend([0; 15]);
        bad.extend([0, 1]);
        t.dht(2 + bad.len() as u16, &bad).unwrap();
        assert!(t.require(true, 0).is_err());
        // symbole DC > 15 : refusé en DC seulement
        let mut ac = vec![0x10, 1];
        ac.extend([0; 15]);
        ac.push(0x20);
        t.dht(2 + ac.len() as u16, &ac).unwrap();
        assert!(t.require(false, 0).is_ok());
        assert!(t.dht(2 + 2, &[0x04, 0]).is_err()); // longueur résiduelle
        let mut idx = vec![0x14];
        idx.extend([0; 16]);
        assert!(t.dht(2 + idx.len() as u16, &idx).is_err()); // index 4
        t.standard();
        assert!(t.require(true, 1).is_ok() && t.require(true, 2).is_err());
    }

    #[test]
    fn donnees_normalisees_comme_libjpeg() {
        let copy = |d: &[u8]| {
            let mut out = Vec::new();
            copy_entropy(d, 0, d.len(), &mut out);
            out
        };
        // remplissage et marqueur invalide retirés ensemble, `FF FF 00` = octet FF
        assert_eq!(copy(b"\x12\xff\xff\x05\x34"), b"\x12\x34");
        assert_eq!(copy(b"\x12\xff\xff\x00\x34"), b"\x12\xff\x00\x34");
        assert_eq!(copy(b"\x12\xff\xff\xd0\x34"), b"\x12\xff\xd0\x34");
        // remplissage en fin de plage (avant le marqueur final)
        assert_eq!(copy(b"\x12\xff\xff"), b"\x12");
    }

    #[test]
    fn dqt_et_dac_comme_libjpeg() {
        let mut t = Tables::default();
        let table = |n: u8, size: usize| [vec![n], vec![1; size]].concat();
        let (complete, suspended) = (Ok(Some(())), Ok(None));
        assert_eq!(t.dqt(2 + 65, &table(0x00, 64)), complete);
        // Pq = 2 : 16 bits
        assert_eq!(t.dqt(2 + 129, &table(0x21, 128)), complete);
        assert!(t.quant[1]);
        // reste d'un octet : 64 valeurs lues au-delà du segment, s'il y en a
        let rest = [table(0x02, 64), vec![0x00]].concat();
        assert!(t.dqt(2 + 66, &[&rest[..], &[1; 64]].concat()).is_err());
        assert_eq!(t.dqt(2 + 66, &[&rest[..], &[1; 10]].concat()), suspended);
        assert!(t
            .dqt(2 + 66, &[table(0x02, 64), vec![0x04]].concat())
            .is_err()); // reste : index 4
        assert!(t
            .dqt(2 + 128, &[table(0x13, 127), vec![1; 9]].concat())
            .is_err());
        assert!(t.dqt(2 + 65, &table(0x04, 64)).is_err());
        assert!(t.dqt(1, &table(0x00, 64)).is_err());
        assert_eq!(Tables::dac(2 + 4, b"\x00\x10\x1f\xff"), complete);
        assert!(Tables::dac(2 + 2, b"\x00\x01").is_err()); // L > U
        assert!(Tables::dac(2 + 2, b"\x20\x00").is_err()); // index 32
                                                           // longueur impaire : valeur lue après le segment, puis longueur négative
        assert!(Tables::dac(2 + 3, b"\x10\x05\x00\xff").is_err());
        assert_eq!(Tables::dac(2 + 3, b"\x10\x05\x00"), suspended);
        assert!(Tables::dac(1, b"\x10\x05").is_err());
        assert!(Tables::dri(5, b"\x00\x01\x00").is_err());
        assert_eq!(Tables::dri(4, b"\x00"), Ok(None));
        let mut t = Tables::default();
        assert_eq!(t.dht(2 + 17, &[0x00; 10]), suspended);
        assert!(t.dht(2 + 17, &[&[0x00, 5][..], &[0; 15]].concat()).is_err()); // compte > longueur
    }

    #[test]
    fn annulation_avant_tout() {
        assert_eq!(
            decode_rgba(b"\xff\xd8\xff", &|| true),
            Err(PngError::Cancelled)
        );
    }
}
