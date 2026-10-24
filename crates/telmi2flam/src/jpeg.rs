//! Décodage JPEG vers RGBA avec la sémantique de `_open_rgba` (`tools/telmi2flam/telmi2flam.py`)
//! exécuté avec **Pillow 12.3.0** (libjpeg-turbo 3.1.4.1) : `Image.open`, `load()` puis
//! `convert("RGBA")` (#459, décision D7 de `PORT-RUST.md`).
//!
//! Deux étages :
//! 1. **Verdict** (image lue ou illisible), reproduit à la main : analyse de `JpegImageFile._open`
//!    (marqueurs lus en Python jusqu'au premier `SOS`, exceptions comprises), garde anti-bombe de
//!    `Image.open`, refus fatals de libjpeg-turbo sur la structure (`SOF` en double ou non pris en
//!    charge, marqueur réservé, `SOI` répété, longueurs de `DHT`, `DQT`, `DRI` et `SOS`, index et
//!    validité des tables de Huffman utilisées, table de quantification absente, composante
//!    inconnue, paramètres de progression, échantillonnage hors 1..4 ou non entier, côté au-delà
//!    de 65 500 px) et fin des données (fichier tronqué : `LOAD_TRUNCATED_IMAGES` reste faux dans
//!    `telmi2flam.py`). Règle de fin, mesurée sur l'oracle : une image à un seul balayage est lue
//!    si un marqueur suit ses données (les blocs manquants sont mis à zéro par libjpeg) ; sans
//!    marqueur, libjpeg attend des octets qui ne viennent pas et Pillow lève `image file is
//!    truncated`. Une image à plusieurs balayages (progressive, ou composantes en balayages
//!    séparés) n'est lue que si `EOI` est atteint : `jpeg_start_decompress` consomme tout le
//!    fichier. En séquentiel, les tables de Huffman 0 et 1 absentes sont les tables standard
//!    (annexe K de T.81), comme `std_huff_tables` de libjpeg-turbo ; pas en progressif.
//! 2. **Pixels** : `zune-jpeg` (scalaire : ni AVX2 ni NEON compilés, `use_unsafe` faux) décode
//!    un flux réécrit (seulement `SOF`, `DQT`, `DHT`, `DRI`, `SOS` et données, puis `EOI` ;
//!    paramètres de progression d'un `SOS` séquentiel remis à 0/63/0, que libjpeg ignore) en
//!    composantes brutes suréchantillonnées. La conversion de couleur suit le choix de
//!    libjpeg-turbo (`default_decompress_parms` : JFIF, puis Adobe, puis identifiants des
//!    composantes) et celle de Pillow (`CMYK;I` : polarité Adobe toujours supposée, puis
//!    `cmyk2rgb`). L'IDCT et le suréchantillonnage de `zune-jpeg` ne sont pas ceux de
//!    libjpeg-turbo : **écart borné à ±4 par composante** (D7, mesuré sur les vecteurs
//!    `tests/vectors/jpeg/`), seul écart admis par `parity.py` (règle `jpeg_bounded`).
//!
//! Écarts assumés (PORT-RUST.md §4.4) : données corrompues ou coupées puis terminées par un
//! marqueur (libjpeg avertit et continue, ses pixels ne sont pas reproduits ; un refus de
//! `zune-jpeg` donne une image illisible) ; flux sans marqueur final que libjpeg lit quand
//! assez d'octets suivent les dernières données ; flux de tables seules suivi d'une image ;
//! codages arithmétique et sans perte, lus par libjpeg-turbo mais pas par `zune-jpeg`,
//! **délégués** ([`PngError::Unsupported`], code 7) ; EXIF et MPO malformés (exceptions de
//! Pillow non reproduites). Annulation consultée avant et après le décodage de `zune-jpeg` (qui
//! ne rend pas la main pendant le décodage), puis à chaque ligne de la conversion.

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
const COM: u8 = 0xFE;
/// `APP0` « AVI1 » : `zune-jpeg` complète alors les tables de Huffman 0 et 1 absentes par les
/// tables standard (comme libjpeg-turbo en séquentiel).
const AVI1: &[u8] = b"\xff\xe0\x00\x07AVI1\x00";

fn unreadable<T>(why: &'static str) -> Result<T> {
    Err(PngError::Unreadable(why))
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
            return unreadable("longueur de segment tronquée");
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
    /// `get_dht` : comptes, index, longueur exacte.
    fn dht(&mut self, len: u16, body: &[u8]) -> Result<()> {
        if len < 2 {
            return unreadable("JERR_BAD_LENGTH (DHT)");
        }
        let mut s = body;
        while s.len() > 16 {
            let index = s[0];
            let bits = &s[1..17];
            let count: usize = bits.iter().map(|&b| usize::from(b)).sum();
            s = &s[17..];
            if count > 256 || count > s.len() {
                return unreadable("JERR_BAD_HUFF_TABLE");
            }
            let vals = &s[..count];
            s = &s[count..];
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
        if !s.is_empty() {
            return unreadable("JERR_BAD_LENGTH (DHT)");
        }
        Ok(())
    }

    /// `get_dqt` : index des tables (longueur négative : `JERR_BAD_LENGTH`).
    fn dqt(&mut self, len: u16, body: &[u8]) -> Result<()> {
        if len < 2 {
            return unreadable("JERR_BAD_LENGTH (DQT)");
        }
        let mut s = body;
        while let Some(&n) = s.first() {
            if n & 15 >= 4 {
                return unreadable("JERR_DQT_INDEX");
            }
            self.quant[usize::from(n & 15)] = true;
            let size = if n >> 4 != 0 { 128 } else { 64 };
            s = &s[(1 + size).min(s.len())..];
        }
        Ok(())
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
}

impl Header {
    fn progressive(&self) -> bool {
        self.sof == Some(0xC2)
    }
}

/// `APP` de Pillow : JFIF et Adobe lisent `i16(s, 5)` (`struct.error` sous 7 octets) ; bloc
/// Photoshop parcouru (seul `struct.error` y est rattrapé : un `IndexError` remonte).
fn pillow_app(marker: u8, s: &[u8]) -> Result<()> {
    if (marker == 0xE0 && s.starts_with(b"JFIF") || marker == 0xEE && s.starts_with(b"Adobe"))
        && s.len() < 7
    {
        return unreadable("segment JFIF ou Adobe trop court");
    }
    if marker == 0xED && s.starts_with(b"Photoshop 3.0\0") {
        let mut offset = 14usize;
        while s.get(offset..offset + 4) == Some(b"8BIM") {
            offset += 4;
            if s.len() < offset + 2 {
                break; // i16 : struct.error rattrapé
            }
            offset += 2;
            let Some(&name_len) = s.get(offset) else {
                return unreadable("bloc Photoshop tronqué"); // IndexError
            };
            offset += 1 + usize::from(name_len);
            offset += offset & 1;
            let Some(size) = s.get(offset..offset + 4) else {
                break; // i32 : struct.error rattrapé
            };
            let size = u32::from_be_bytes([size[0], size[1], size[2], size[3]]) as usize;
            // `ResolutionInfo` court : struct.error, rattrapé aussi
            offset = (offset + 4).saturating_add(size);
            offset += offset & 1;
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
        if !progressive {
            h.tables.require(true, dc)?;
            h.tables.require(false, ac)?;
        } else if ss == 0 && ah == 0 {
            h.tables.require(true, dc)?;
        } else if ss != 0 {
            h.tables.require(false, ac)?;
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
    let mut h = Header::default();
    h.stream.extend_from_slice(&[0xFF, SOI]);
    // Profils ICC vus depuis le dernier SOF (`self.icclist`).
    let mut icc: Vec<&[u8]> = Vec::new();
    let mut pillow_size: Option<(u32, u32)> = None;
    let mut cur = Some(0xFFu8); // s = b"\xff"
    loop {
        // `s[0]` sur `b""` : IndexError
        let Some(b0) = cur else {
            return unreadable("fin des données avant SOS");
        };
        if b0 != 0xFF {
            cur = r.byte(); // octets parasites
            continue;
        }
        // `i16(s + read(1))` : struct.error en fin de données
        let Some(m) = r.byte() else {
            return unreadable("fin des données dans un marqueur");
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
            0x01..=0xBF => return unreadable("no marker found"),
            _ => {}
        }
        let start = r.pos - 2;
        match m {
            // SOFn et DHP : gestionnaire `SOF` de Pillow
            0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF | 0xDE => {
                let (_, s) = r.segment()?;
                if s.len() < 6 {
                    return unreadable("SOF trop court"); // struct.error / IndexError
                }
                let height = u32::from(u16::from_be_bytes([s[1], s[2]]));
                let width = u32::from(u16::from_be_bytes([s[3], s[4]]));
                pillow_size = Some((width, height));
                if s[0] != 8 {
                    return unreadable("cannot handle non 8-bit layers");
                }
                let layers = usize::from(s[5]);
                if ![1, 3, 4].contains(&layers) {
                    return unreadable("cannot handle this number of layers");
                }
                // `self.icclist[0][13]` après tri : IndexError si le plus petit est court
                if icc.iter().min().is_some_and(|first| first.len() <= 13) {
                    return unreadable("profil ICC tronqué");
                }
                icc.clear();
                // `t[1]`, `t[2]` d'un triplet incomplet : IndexError
                if (s.len() - 6) % 3 != 0 {
                    return unreadable("composante de SOF incomplète");
                }
                // libjpeg-turbo : un seul SOF, types pris en charge, longueur exacte
                if !matches!(m, 0xC0..=0xC3 | 0xC9..=0xCB) {
                    return unreadable("JERR_SOF_UNSUPPORTED");
                }
                if h.sof.is_some() {
                    return unreadable("JERR_SOF_DUPLICATE");
                }
                if s.len() != 6 + 3 * layers {
                    return unreadable("JERR_BAD_LENGTH (SOF)");
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
                let (len, s) = r.segment()?;
                h.tables.dht(len, s)?;
                h.stream.extend_from_slice(&data[start..r.pos]);
            }
            DQT => {
                let (len, s) = r.segment()?;
                // `DQT` de Pillow : table plus courte que sa précision
                let mut t = s;
                while let Some(&v) = t.first() {
                    let n = 1 + if v >> 4 == 0 { 64 } else { 128 };
                    if t.len() < n {
                        return unreadable("bad quantization table marker");
                    }
                    t = &t[n..];
                }
                h.tables.dqt(len, s)?;
                h.stream.extend_from_slice(&data[start..r.pos]);
            }
            DRI => {
                let (len, s) = r.segment()?;
                if len != 4 {
                    return unreadable("JERR_BAD_LENGTH (DRI)");
                }
                h.restart = u16::from_be_bytes([s[0], s[1]]);
                h.stream.extend_from_slice(&data[start..r.pos]);
            }
            0xE0..=0xEF => {
                let (_, s) = r.segment()?;
                pillow_app(m, s)?;
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
            // DAC n'a de sens qu'en arithmétique (délégué) ; EXP est réservé pour libjpeg
            COM | DNL | 0xCC => {
                r.segment()?;
            }
            0xDF => {
                r.segment()?;
                return unreadable("JERR_UNKNOWN_MARKER");
            }
            SOS => {
                let (len, s) = r.segment()?;
                let Some((width, height)) = pillow_size else {
                    return unreadable("SOS sans SOF");
                };
                // `Image.open` : mode vide ou taille nulle, puis garde anti-bombe
                if width == 0 || height == 0 {
                    return unreadable("not identified by this driver");
                }
                if u64::from(width) * u64::from(height) > 2 * MAX_IMAGE_PIXELS {
                    return unreadable("décompression bombe (plus de 2 x MAX_IMAGE_PIXELS)");
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
                return Ok(h);
            }
            // libjpeg : SOI répété (JERR_SOI_DUPLICATE), JPG et JPGn réservés ; EOI avant
            // l'image (flux de tables seules suivi d'une autre image) : non pris en charge,
            // illisible.
            SOI => return unreadable("JERR_SOI_DUPLICATE"),
            EOI => return unreadable("EOI avant l'image"),
            0xC8 | 0xF0..=0xFD => return unreadable("JERR_UNKNOWN_MARKER"),
            // RSTn hors balayage : sans paramètre, ignoré par les deux
            _ => {}
        }
        cur = r.byte();
    }
}

/// Fin des données compressées d'un balayage commençant à `pos` : position du `0xFF` et octet
/// bas du marqueur qui la termine (`RSTn` sautés si `skip_rst`), ou `None` en fin de données.
fn entropy_end(data: &[u8], mut pos: usize, skip_rst: bool) -> Option<(usize, u8)> {
    loop {
        let ff = pos + data.get(pos..)?.iter().position(|&b| b == 0xFF)?;
        let mut q = ff + 1;
        while *data.get(q)? == 0xFF {
            q += 1; // remplissage
        }
        let m = data[q];
        if m == 0x00 || (skip_rst && (0xD0..=0xD7).contains(&m)) {
            pos = q + 1;
            continue;
        }
        return Some((q - 1, m));
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
/// et flux pour `zune-jpeg` (balayages et tables, sans `APPn` ni `COM`).
fn walk_scans(data: &[u8], h: &mut Header, latched: &mut [bool], multi_scan: bool) -> Result<()> {
    let truncated = || unreadable("image file is truncated");
    // premier balayage : données jusqu'au marqueur qui le termine (RSTn attendus sautés)
    let skip_rst = multi_scan || h.restart != 0;
    let Some((end, mut m)) = entropy_end(data, h.entropy, skip_rst) else {
        return truncated();
    };
    h.stream.extend_from_slice(&data[h.entropy..end]);
    let mut pos = end + 2;
    loop {
        match m {
            EOI => return Ok(()),
            SOS if !multi_scan => return unreadable("JERR_EOI_EXPECTED"),
            SOS | DHT | DQT | DRI | DNL | COM | 0xE0..=0xEF => {
                let mut r = Reader { data, pos };
                let (len, s) = match r.segment() {
                    Ok(seg) => seg,
                    // un seul balayage : toutes les lignes sont lues, libjpeg attend en vain la
                    // fin du fichier et Pillow s'en contente
                    Err(_) if !multi_scan => return Ok(()),
                    Err(_) => return truncated(),
                };
                let copy = multi_scan && matches!(m, DHT | DQT | DRI);
                match m {
                    DHT => h.tables.dht(len, s)?,
                    DQT => h.tables.dqt(len, s)?,
                    DRI if len != 4 => return unreadable("JERR_BAD_LENGTH (DRI)"),
                    _ => {}
                }
                if copy {
                    h.stream.extend_from_slice(&data[pos - 2..r.pos]);
                }
                pos = r.pos;
                if m == SOS {
                    let mut stream = std::mem::take(&mut h.stream);
                    let scanned = scan_header(h, len, s, latched, &mut stream);
                    h.stream = stream;
                    scanned?;
                    let Some((end, next)) = entropy_end(data, pos, true) else {
                        return truncated();
                    };
                    h.stream.extend_from_slice(&data[pos..end]);
                    pos = end + 2;
                    m = next;
                    continue;
                }
            }
            0xD0..=0xD7 => {} // sans paramètre : ignoré
            _ => return unreadable("marqueur inattendu après le premier balayage"),
        }
        match segment_marker(data, pos) {
            Some((at, next)) => {
                pos = at;
                m = next;
            }
            None if multi_scan => return truncated(),
            None => return Ok(()),
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
    match h.sof {
        Some(0xC9..=0xCB) => return Err(PngError::Unsupported("JPEG arithmétique")),
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
    walk_scans(data, &mut h, &mut latched, multi_scan)?;
    h.stream.extend_from_slice(&[0xFF, EOI]);
    if !h.progressive() {
        // tables standard de zune-jpeg pour les tables 0 et 1 absentes, juste après SOI
        h.stream.splice(2..2, AVI1.iter().copied());
    }
    if cancelled() {
        return Err(PngError::Cancelled);
    }

    let space = libjpeg_space(&h);
    let (w, ht) = (h.width as usize, h.height as usize);
    let channels = h.components.len();
    let raw = decode_raw(&std::mem::take(&mut h.stream), channels, w, ht)?;
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
/// `zune-jpeg` en scalaire ; un refus de `zune-jpeg` rend l'image illisible (écart assumé :
/// libjpeg-turbo poursuit souvent sur des données corrompues).
fn decode_raw(stream: &[u8], channels: usize, w: usize, h: usize) -> Result<Vec<u8>> {
    let options = DecoderOptions::default()
        .set_use_unsafe(false)
        .set_strict_mode(false)
        .set_max_width(usize::from(u16::MAX))
        .set_max_height(usize::from(u16::MAX))
        // libjpeg-turbo ne limite pas le nombre de balayages
        .jpeg_set_max_scans(usize::MAX);
    let mut decoder = JpegDecoder::new_with_options(ZCursor::new(stream), options);
    let refused = |_| PngError::Unreadable("JPEG refusé par zune-jpeg");
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
        return unreadable("JPEG : sortie de zune-jpeg inattendue");
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
    fn annulation_avant_tout() {
        assert_eq!(
            decode_rgba(b"\xff\xd8\xff", &|| true),
            Err(PngError::Cancelled)
        );
    }
}
