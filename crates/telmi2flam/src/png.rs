//! Décodage PNG vers RGBA avec la sémantique de `_open_rgba` (`tools/telmi2flam/telmi2flam.py`)
//! exécuté avec **Pillow 12.3.0** : mêmes pixels, et image illisible exactement quand Python lève
//! une exception (avertissement `image_unreadable`).
//!
//! Décodeur écrit à la main au-dessus de `miniz_oxide` (et non la crate `png`) pour reproduire les
//! choix de Pillow sur des entrées non fiables :
//! - CRC vérifié seulement pour les chunks **avant** le premier IDAT ; ni pour les IDAT ni après ;
//! - lecture arrêtée dès la dernière ligne décodée : données en trop, chunks suivants, IEND
//!   absent sans effet ; seuls les gestionnaires de chunks de Pillow qui lèvent une exception
//!   (chunk tronqué, `gAMA` court, `iCCP` sans profil…) rendent l'image illisible ;
//! - somme Adler-32 vérifiée seulement si elle arrive dans le même bloc de 64 Kio que la fin
//!   de l'image (comportement de zlib dans `ZipDecode.c`) ;
//! - conversions de `_open_rgba` : 16 → 8 bits par l'octet de poids fort, clés `tRNS` brutes
//!   (gris 1/2/4/16 bits, RGB 16 bits) ou réduites à l'octet bas (gris et RGB 8 bits, comme
//!   `rgbT2rgba`), palette complétée par du noir opaque ;
//! - garde anti-bombe de Pillow seulement : plus de `2 × MAX_IMAGE_PIXELS` pixels, image
//!   illisible (décision D5 de `PORT-RUST.md`) ; un manque de mémoire rend aussi l'image
//!   illisible, comme le `MemoryError` attrapé par Python.
//!
//! Les cas que ce module ne modélise pas sont **délégués** à Python ([`PngError::Unsupported`],
//! code 7, décision D4) : autre format que PNG que Pillow pourrait lire (voir [`crate::identify`] ;
//! des données qu'aucun greffon de Pillow n'accepte sont illisibles), APNG (`acTL`, `fcTL`, `fdAT`), `IHDR` absent en
//! tête ou répété, `tRNS` répété, placé après l'IDAT ou de longueur inattendue.

use std::fmt;

use miniz_oxide::inflate::core::inflate_flags::{
    TINFL_FLAG_HAS_MORE_INPUT, TINFL_FLAG_PARSE_ZLIB_HEADER,
    TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF,
};
use miniz_oxide::inflate::core::{decompress, DecompressorOxide};
use miniz_oxide::inflate::TINFLStatus;

/// `PIL.Image.MAX_IMAGE_PIXELS` : au-delà du double, Pillow refuse l'image.
pub const MAX_IMAGE_PIXELS: u64 = 1024 * 1024 * 1024 / 4 / 3;
/// `ImageFile.MAXBLOCK` : taille des lectures de données d'image de Pillow.
const MAX_BLOCK: usize = 65536;
/// `PngImagePlugin.MAX_TEXT_CHUNK` et `MAX_TEXT_MEMORY`.
const MAX_TEXT_CHUNK: usize = 1024 * 1024;
const MAX_TEXT_MEMORY: usize = 64 * MAX_TEXT_CHUNK;
const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

/// Image décodée, RGBA8888 par lignes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Échec du décodage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PngError {
    /// Cas non modélisé (autre format, APNG…) : à déléguer à Python (`unsupported_format`).
    Unsupported(&'static str),
    /// Pillow lève une exception sur ce fichier : avertissement `image_unreadable`.
    Unreadable(&'static str),
    /// Jeton d'annulation levé pendant le décodage.
    Cancelled,
}

impl fmt::Display for PngError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PngError::Unsupported(why) => write!(f, "format non pris en charge : {why}"),
            PngError::Unreadable(why) => write!(f, "image illisible : {why}"),
            PngError::Cancelled => f.write_str("annulé"),
        }
    }
}

impl std::error::Error for PngError {}

type Result<T> = std::result::Result<T, PngError>;

fn unreadable<T>(why: &'static str) -> Result<T> {
    Err(PngError::Unreadable(why))
}

/// Modes de Pillow (`PngImagePlugin._MODES`) pour (profondeur, type de couleur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Bit,     // gris 1 bit -> « 1 »
    Gray,    // gris 2/4/8 bits -> « L »
    Gray16,  // gris 16 bits -> « I;16 »
    Rgb,     // RGB 8/16 bits
    Palette, // palette 1/2/4/8 bits -> « P »
    GrayA,   // gris + alpha 8 bits -> « LA »
    GrayA16, // gris + alpha 16 bits -> « RGBA » (brut « LA;16B »)
    Rgba4,   // RGBA 8/16 bits
}

fn pillow_mode(depth: u8, color: u8) -> Option<Mode> {
    Some(match (depth, color) {
        (1, 0) => Mode::Bit,
        (2 | 4 | 8, 0) => Mode::Gray,
        (16, 0) => Mode::Gray16,
        (8 | 16, 2) => Mode::Rgb,
        (1 | 2 | 4 | 8, 3) => Mode::Palette,
        (8, 4) => Mode::GrayA,
        (16, 4) => Mode::GrayA16,
        (8 | 16, 6) => Mode::Rgba4,
        _ => return None,
    })
}

fn channels(color: u8) -> u32 {
    match color {
        2 => 3,
        4 => 2,
        6 => 4,
        _ => 1,
    }
}

/// Lecture d'un fichier en mémoire, à la manière d'un `fp.read` de Python (lectures courtes
/// en fin de fichier, position au-delà de la fin permise).
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    /// `fp.read(n)` : au plus `n` octets.
    fn read(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.data.len());
        let end = start.saturating_add(n).min(self.data.len());
        self.pos = self.pos.saturating_add(n);
        &self.data[start..end]
    }

    /// `ImageFile._safe_read` : exactement `n` octets, sinon exception.
    fn safe_read(&mut self, n: usize) -> Result<&'a [u8]> {
        let s = self.read(n);
        if s.len() < n {
            return unreadable("chunk tronqué");
        }
        Ok(s)
    }

    /// `ChunkStream.read` : longueur et type du chunk suivant. `Ok(None)` : en-tête incomplet
    /// (`struct.error`) ou type invalide (`SyntaxError`), traités par l'appelant.
    fn chunk_header(&mut self) -> Option<([u8; 4], usize)> {
        let s = self.read(8);
        if s.len() < 8 {
            return None;
        }
        let kind = [s[4], s[5], s[6], s[7]];
        if !kind.iter().all(|c| c.is_ascii_alphanumeric() || *c == b'_') {
            return None;
        }
        Some((kind, u32::from_be_bytes([s[0], s[1], s[2], s[3]]) as usize))
    }
}

/// Décompresseur zlib dans un tampon sans retour circulaire : comme zlib (et non comme le mode
/// circulaire de miniz), une distance qui remonte avant le début des données est une erreur.
///
/// Le tampon grandit avec ce que miniz produit réellement (croissance géométrique plafonnée à
/// `max`) : une taille annoncée par l'IHDR n'engage aucune mémoire sans données derrière.
struct Inflater {
    state: Box<DecompressorOxide>,
    out: Vec<u8>,
    /// Octets produits dans `out`.
    pos: usize,
    done: bool,
    /// Taille maximale de `out`.
    max: usize,
}

/// Résultat d'un appel à [`Inflater::run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// Octets d'entrée consommés.
    Consumed(usize),
    /// Flux zlib invalide (`zlib.error`, ou `IMAGING_CODEC_BROKEN` de `ZipDecode.c`).
    Broken,
}

/// Taille de départ du tampon de sortie.
const INFLATE_START: usize = 1024;

impl Inflater {
    fn new(max: usize) -> Self {
        Inflater {
            state: Box::default(),
            out: Vec::new(),
            pos: 0,
            done: false,
            max,
        }
    }

    /// Réutilise le décompresseur pour un nouveau flux (textes : un seul état pour tous les
    /// chunks). Les anciens octets du tampon ne sont pas lisibles : miniz refuse toute distance
    /// qui remonte avant la position de départ.
    fn reset(&mut self) {
        self.state.init();
        self.pos = 0;
        self.done = false;
    }

    /// Agrandit `out` (au moins d'un octet, au plus jusqu'à `limit`).
    fn grow(&mut self, limit: usize) -> Result<()> {
        let want = (self.out.len() * 2).max(INFLATE_START).min(limit);
        if self.out.try_reserve_exact(want - self.out.len()).is_err() {
            return unreadable("mémoire insuffisante");
        }
        self.out.resize(want, 0);
        Ok(())
    }

    /// Décompresse `input` sans dépasser `limit` octets produits au total.
    fn run(&mut self, input: &[u8], limit: usize) -> Result<Step> {
        let limit = limit.min(self.max);
        let flags = TINFL_FLAG_PARSE_ZLIB_HEADER
            | TINFL_FLAG_HAS_MORE_INPUT
            | TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
        let mut consumed = 0;
        while !self.done {
            if self.out.len() <= self.pos && self.out.len() < limit {
                self.grow(limit)?;
            }
            let end = limit.min(self.out.len());
            let (status, used, written) = decompress(
                &mut self.state,
                &input[consumed..],
                &mut self.out[..end],
                self.pos,
                flags,
            );
            consumed += used;
            self.pos += written;
            match status {
                TINFLStatus::Done => self.done = true,
                TINFLStatus::NeedsMoreInput => break,
                TINFLStatus::HasMoreOutput if self.out.len() < limit => self.grow(limit)?,
                TINFLStatus::HasMoreOutput => break,
                _ => return Ok(Step::Broken),
            }
        }
        Ok(Step::Consumed(consumed))
    }

    /// Ce que zlib fait encore de `input` une fois la sortie pleine : il traite la fin de bloc
    /// et vérifie l'Adler-32, mais s'arrête au premier octet à produire. `Ok(Some(true))` : il
    /// reste un octet à produire ; `Ok(None)` : flux invalide.
    fn probe(&mut self, input: &[u8]) -> Result<Option<bool>> {
        let before = self.pos;
        let step = self.run(input, before + 1)?;
        if self.pos > before {
            // zlib s'arrête sur cet octet : ce que miniz lit ensuite (Adler-32…) ne compte pas
            return Ok(Some(true));
        }
        Ok(match step {
            Step::Broken => None,
            Step::Consumed(_) => Some(false),
        })
    }
}

/// `_safe_zlib_decompress` : au plus `MAX_TEXT_CHUNK` octets (`ValueError` au-delà). Un flux
/// tronqué n'est pas une erreur. Rend le nombre d'octets décompressés (dans `z.out`), `None`
/// pour un `zlib.error` (ignoré par les gestionnaires de Pillow).
fn safe_zlib_decompress(z: &mut Inflater, input: &[u8]) -> Result<Option<usize>> {
    z.reset();
    let used = match z.run(input, MAX_TEXT_CHUNK)? {
        Step::Broken => return Ok(None),
        Step::Consumed(n) => n,
    };
    if z.pos == MAX_TEXT_CHUNK && !z.done {
        // sortie pleine : `unconsumed_tail` non vide si zlib a encore un octet à produire
        match z.probe(&input[used..])? {
            Some(true) => return unreadable("texte compressé trop grand"),
            None => return Ok(None),
            Some(false) => {}
        }
    }
    Ok(Some(z.pos.min(MAX_TEXT_CHUNK)))
}

/// Clé de transparence retenue par Pillow et `_trns_mask`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Trns {
    Gray(u16),
    Rgb(u16, u16, u16),
    /// Palette : alpha de chaque entrée (`putpalettealphas`), ou une seule entrée
    /// transparente (`putpalettealpha`, motif `^\xff*\x00\xff*$`).
    Alphas(Vec<u8>),
    Index(usize),
}

/// État lu par `PngStream` avant le premier IDAT.
struct Header {
    width: u32,
    height: u32,
    depth: u8,
    color: u8,
    mode: Option<Mode>,
    interlace: bool,
    palette: Option<Vec<u8>>,
    trns: Option<Trns>,
    text_memory: usize,
    /// Décompresseur des chunks de texte, réutilisé d'un chunk à l'autre.
    text: Inflater,
}

impl Header {
    fn add_text(&mut self, n: usize) -> Result<()> {
        self.text_memory = self.text_memory.saturating_add(n);
        if self.text_memory > MAX_TEXT_MEMORY {
            return unreadable("trop de texte");
        }
        Ok(())
    }

    /// Gestionnaires des chunks ancillaires de Pillow (avant ou après l'IDAT) : seules leurs
    /// exceptions comptent, leurs valeurs ne changent pas les pixels.
    fn ancillary(&mut self, kind: &[u8; 4], s: &[u8]) -> Result<()> {
        match kind {
            b"gAMA" if s.len() < 4 => unreadable("gAMA court"),
            b"cHRM" if !s.len().is_multiple_of(4) => unreadable("cHRM de longueur invalide"),
            b"sRGB" if s.is_empty() => unreadable("sRGB vide"),
            b"pHYs" if s.len() < 9 => unreadable("pHYs court"),
            b"tEXt" => {
                let (k, v) = split_nul(s);
                if !k.is_empty() {
                    self.add_text(v.len())?;
                }
                Ok(())
            }
            b"zTXt" => {
                let (k, v) = split_nul(s);
                if v.first().copied().unwrap_or(0) != 0 {
                    return unreadable("méthode zTXt inconnue");
                }
                let n = safe_zlib_decompress(&mut self.text, v.get(1..).unwrap_or(&[]))?;
                let n = n.unwrap_or(0);
                if !k.is_empty() {
                    self.add_text(n)?;
                }
                Ok(())
            }
            b"iTXt" => self.itxt(s),
            b"iCCP" => {
                let at = s.iter().position(|&c| c == 0).map_or(0, |i| i + 1);
                match s.get(at) {
                    None => unreadable("iCCP sans méthode"),
                    Some(0) => safe_zlib_decompress(&mut self.text, &s[at + 1..]).map(|_| ()),
                    Some(_) => unreadable("méthode iCCP inconnue"),
                }
            }
            _ => Ok(()),
        }
    }

    fn itxt(&mut self, s: &[u8]) -> Result<()> {
        let Some(nul) = s.iter().position(|&c| c == 0) else {
            return Ok(());
        };
        let r = &s[nul + 1..];
        if r.len() < 2 {
            return Ok(());
        }
        let (compressed, method, r) = (r[0], r[1], &r[2..]);
        let mut parts = r.splitn(3, |&c| c == 0);
        let (Some(lang), Some(tk), Some(v)) = (parts.next(), parts.next(), parts.next()) else {
            return Ok(());
        };
        let owned;
        let v = if compressed != 0 {
            if method != 0 {
                return Ok(());
            }
            match safe_zlib_decompress(&mut self.text, v)? {
                Some(n) => {
                    owned = self.text.out[..n].to_vec();
                    &owned[..]
                }
                None => return Ok(()),
            }
        } else {
            v
        };
        let decoded = (
            std::str::from_utf8(lang),
            std::str::from_utf8(tk),
            std::str::from_utf8(v),
        );
        if let (Ok(_), Ok(_), Ok(text)) = decoded {
            self.add_text(text.chars().count())?;
        }
        Ok(())
    }
}

fn split_nul(s: &[u8]) -> (&[u8], &[u8]) {
    match s.iter().position(|&c| c == 0) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, &[]),
    }
}

fn crc_ok(kind: &[u8; 4], data: &[u8], stored: &[u8]) -> bool {
    let mut crc = crc32fast::Hasher::new();
    crc.update(kind);
    crc.update(data);
    stored.len() == 4
        && crc.finalize() == u32::from_be_bytes([stored[0], stored[1], stored[2], stored[3]])
}

/// `PngImageFile._open` : chunks jusqu'au premier IDAT. Rend l'en-tête et la longueur de ce
/// premier IDAT (le curseur est sur ses données).
fn read_header(cur: &mut Cursor, cancelled: &dyn Fn() -> bool) -> Result<(Header, usize)> {
    let mut h = Header {
        width: 0,
        height: 0,
        depth: 0,
        color: 0,
        mode: None,
        interlace: false,
        palette: None,
        trns: None,
        text_memory: 0,
        text: Inflater::new(MAX_TEXT_CHUNK + 1),
    };
    let mut first = true;
    let mut idat = None;
    loop {
        if cancelled() {
            return Err(PngError::Cancelled);
        }
        let Some((kind, len)) = cur.chunk_header() else {
            return unreadable("en-tête de chunk invalide");
        };
        let s: &[u8] = match &kind {
            b"IDAT" => {
                idat = Some(len);
                break;
            }
            b"IEND" => break,
            b"acTL" | b"fcTL" | b"fdAT" => return Err(PngError::Unsupported("APNG")),
            b"IHDR" => {
                if !first {
                    return Err(PngError::Unsupported("IHDR absent en tête ou répété"));
                }
                let s = cur.safe_read(len)?;
                if s.len() < 13 {
                    return unreadable("IHDR court");
                }
                h.width = u32::from_be_bytes([s[0], s[1], s[2], s[3]]);
                h.height = u32::from_be_bytes([s[4], s[5], s[6], s[7]]);
                h.depth = s[8];
                h.color = s[9];
                h.mode = pillow_mode(s[8], s[9]);
                h.interlace = s[12] != 0;
                if s[11] != 0 {
                    return unreadable("méthode de filtrage inconnue");
                }
                s
            }
            b"PLTE" => {
                let s = cur.safe_read(len)?;
                if h.mode == Some(Mode::Palette) {
                    h.palette = Some(s.to_vec());
                }
                s
            }
            b"tRNS" => {
                let s = cur.safe_read(len)?;
                read_trns(&mut h, s)?;
                s
            }
            _ => {
                let s = cur.safe_read(len)?;
                h.ancillary(&kind, s)?;
                s
            }
        };
        if !crc_ok(&kind, s, cur.read(4)) {
            return unreadable("CRC faux");
        }
        first = false;
    }
    if h.mode.is_none() || h.width == 0 || h.height == 0 {
        return unreadable("mode ou taille invalide");
    }
    if u64::from(h.width) * u64::from(h.height) > 2 * MAX_IMAGE_PIXELS {
        return unreadable("bombe de décompression");
    }
    match idat {
        Some(len) => Ok((h, len)),
        None => unreadable("pas d'IDAT"),
    }
}

/// `chunk_tRNS` de Pillow, et clé brute de `_png_trns_key` pour `_trns_mask`.
fn read_trns(h: &mut Header, s: &[u8]) -> Result<()> {
    let Some(mode) = h.mode else {
        return Ok(());
    };
    let i16 = |o: usize| u16::from_be_bytes([s[o], s[o + 1]]);
    let trns = match mode {
        Mode::Palette => {
            // `_simple_palette = re.compile(b"^\xff*\x00\xff*$")` : un seul zéro entouré de
            // 0xFF ; le `$` de Python accepte aussi un saut de ligne final
            let body = s.strip_suffix(b"\n").unwrap_or(s);
            let simple = body.iter().filter(|&&c| c == 0).count() == 1
                && body.iter().all(|&c| c == 0 || c == 0xFF);
            if simple {
                Trns::Index(body.iter().position(|&c| c == 0).unwrap_or(0))
            } else {
                Trns::Alphas(s.to_vec())
            }
        }
        Mode::Bit | Mode::Gray | Mode::Gray16 => {
            if s.len() < 2 {
                return unreadable("tRNS court");
            }
            if s.len() != 2 {
                return Err(PngError::Unsupported("tRNS de longueur inattendue"));
            }
            Trns::Gray(i16(0))
        }
        Mode::Rgb => {
            if s.len() < 6 {
                return unreadable("tRNS court");
            }
            if s.len() != 6 {
                return Err(PngError::Unsupported("tRNS de longueur inattendue"));
            }
            Trns::Rgb(i16(0), i16(2), i16(4))
        }
        Mode::GrayA | Mode::GrayA16 | Mode::Rgba4 => return Ok(()),
    };
    if h.trns.is_some() {
        return Err(PngError::Unsupported("tRNS répété"));
    }
    h.trns = Some(trns);
    Ok(())
}

/// Table RGBA de 256 entrées de Pillow : `PLTE` (au plus 256 entrées), complété par du noir
/// opaque (palette absente comprise), puis les alphas de `tRNS`, même au-delà du `PLTE`.
fn palette_table(h: &Header) -> Result<Vec<[u8; 4]>> {
    let mut table = vec![[0u8, 0, 0, 255]; 256];
    if let Some(p) = &h.palette {
        if p.len() / 3 > 256 {
            return unreadable("palette trop longue");
        }
        for (entry, rgb) in table.iter_mut().zip(p.as_chunks::<3>().0) {
            entry[..3].copy_from_slice(rgb);
        }
    }
    match &h.trns {
        Some(Trns::Alphas(a)) => {
            if a.len() > 256 {
                return unreadable("tRNS plus long que 256");
            }
            for (entry, &alpha) in table.iter_mut().zip(a) {
                entry[3] = alpha;
            }
        }
        Some(Trns::Index(i)) => match table.get_mut(*i) {
            Some(entry) => entry[3] = 0,
            None => return unreadable("index tRNS hors palette"),
        },
        _ => {}
    }
    Ok(table)
}

/// Flux des données d'image, servi comme `PngImageFile.load_read` : blocs d'au plus
/// `MAX_BLOCK` octets dans chaque IDAT (ou `DDAT`) consécutif, CRC non vérifiés. Les lignes
/// brutes (octet de filtre compris) sont décompressées dans `z.out`.
struct IdatStream<'a> {
    cur: Cursor<'a>,
    /// Octets restant à lire dans le chunk courant (longueur annoncée).
    remaining: usize,
    /// Bloc courant et position dans ce bloc.
    block: &'a [u8],
    used: usize,
    z: Inflater,
    /// Taille totale des lignes brutes de l'image.
    total: usize,
}

impl<'a> IdatStream<'a> {
    fn next_block(&mut self) -> Result<()> {
        while self.remaining == 0 {
            self.cur.read(4);
            match self.cur.chunk_header() {
                Some((kind, len)) if &kind == b"IDAT" || &kind == b"DDAT" => self.remaining = len,
                Some((kind, _)) if &kind == b"fdAT" => {
                    return Err(PngError::Unsupported("APNG"));
                }
                _ => return unreadable("données d'image tronquées"),
            }
        }
        let n = self.remaining.min(MAX_BLOCK);
        self.remaining -= n;
        self.block = self.cur.read(n);
        self.used = 0;
        if self.block.is_empty() {
            return unreadable("données d'image tronquées");
        }
        Ok(())
    }

    /// Décompresse jusqu'à disposer des `n` premiers octets bruts.
    fn ensure(&mut self, n: usize) -> Result<()> {
        while self.z.pos < n {
            if self.z.done {
                // flux terminé avant la dernière ligne : Pillow lit jusqu'au bout et échoue
                return unreadable("données d'image incomplètes");
            }
            if self.used >= self.block.len() {
                self.next_block()?;
            }
            match self.z.run(&self.block[self.used..], self.total)? {
                Step::Broken => return unreadable("flux zlib corrompu"),
                Step::Consumed(k) => self.used += k,
            }
        }
        Ok(())
    }

    /// Fin de l'image : zlib a encore vu le reste du bloc courant (fin de flux, Adler-32).
    fn finish(&mut self) -> Result<()> {
        match self.z.probe(&self.block[self.used..])? {
            None => unreadable("flux zlib corrompu"),
            Some(_) => Ok(()),
        }
    }

    /// `PngImageFile.load_end` : chunks après l'image, sans CRC ; seules les exceptions des
    /// gestionnaires comptent.
    fn load_end(mut self, h: &mut Header, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.cur.read(self.remaining);
        loop {
            if cancelled() {
                return Err(PngError::Cancelled);
            }
            self.cur.read(4);
            let Some((kind, len)) = self.cur.chunk_header() else {
                return Ok(());
            };
            match &kind {
                b"IEND" => return Ok(()),
                b"acTL" | b"fcTL" | b"fdAT" => return Err(PngError::Unsupported("APNG")),
                b"tRNS" => return Err(PngError::Unsupported("tRNS après l'IDAT")),
                b"IHDR" => {
                    let s = self.cur.safe_read(len)?;
                    if s.len() < 13 {
                        return unreadable("IHDR court");
                    }
                    if s[11] != 0 {
                        return unreadable("méthode de filtrage inconnue");
                    }
                }
                _ => {
                    let s = self.cur.safe_read(len)?;
                    h.ancillary(&kind, s)?;
                }
            }
        }
    }
}

/// Passes Adam7 : (x0, y0, dx, dy).
const ADAM7: [(u32, u32, u32, u32); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

fn alloc(n: usize) -> Result<Vec<u8>> {
    let mut v = Vec::new();
    if v.try_reserve_exact(n).is_err() {
        return unreadable("mémoire insuffisante");
    }
    v.resize(n, 0);
    Ok(v)
}

/// Tampon de `n` zéros par `alloc_zeroed` (calloc) : pages remises à zéro par le système à
/// la première écriture, et échec d'allocation rendu en `Unreadable` (`MemoryError` de Python).
fn alloc_zeroed(n: usize) -> Result<Vec<u8>> {
    let Ok(layout) = std::alloc::Layout::array::<u8>(n) else {
        return unreadable("taille");
    };
    if n == 0 {
        return Ok(Vec::new());
    }
    // SAFETY: `layout` est non nul ; la mémoire rendue (n octets à zéro, alignement 1) vient
    // de l'allocateur global avec ce même layout, comme l'exige `Vec::from_raw_parts`.
    let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
    if ptr.is_null() {
        return unreadable("mémoire insuffisante");
    }
    // SAFETY: voir ci-dessus ; capacité et longueur valent n, tous les octets sont initialisés.
    Ok(unsafe { Vec::from_raw_parts(ptr, n, n) })
}

fn unfilter(filter: u8, row: &mut [u8], prev: Option<&[u8]>, bpp: usize) -> Result<()> {
    let up = |i: usize| prev.map_or(0, |p| p[i]);
    match filter {
        0 => {}
        1 => {
            for i in bpp..row.len() {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for (i, v) in row.iter_mut().enumerate() {
                *v = v.wrapping_add(up(i));
            }
        }
        3 => {
            for i in 0..row.len() {
                let a = if i >= bpp { row[i - bpp] } else { 0 };
                row[i] = row[i].wrapping_add(((u16::from(a) + u16::from(up(i))) / 2) as u8);
            }
        }
        4 => {
            for i in 0..row.len() {
                let (a, c) = if i >= bpp {
                    (row[i - bpp], up(i - bpp))
                } else {
                    (0, 0)
                };
                row[i] = row[i].wrapping_add(paeth(a, up(i), c));
            }
        }
        _ => return unreadable("filtre de ligne inconnu"),
    }
    Ok(())
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Conversion d'une ligne brute vers RGBA, selon `_open_rgba`.
struct Converter {
    mode: Mode,
    depth: u8,
    trns: Option<Trns>,
    palette: Vec<[u8; 4]>,
}

impl Converter {
    /// Échantillon `i` (de `depth` bits) d'une ligne.
    fn sample(&self, row: &[u8], i: usize) -> u16 {
        match self.depth {
            16 => u16::from_be_bytes([row[2 * i], row[2 * i + 1]]),
            8 => u16::from(row[i]),
            d => {
                let bit = i * usize::from(d);
                let shift = 8 - usize::from(d) - bit % 8;
                u16::from((row[bit / 8] >> shift) & ((1u8 << d) - 1))
            }
        }
    }

    fn pixel(&self, row: &[u8], x: usize) -> [u8; 4] {
        let hi = |v: u16| (v >> 8) as u8;
        match self.mode {
            Mode::Bit | Mode::Gray => {
                let v = self.sample(row, x);
                let max = (1u16 << self.depth) - 1;
                let l = (u32::from(v) * 255 / u32::from(max)) as u8;
                let alpha = match self.trns {
                    // gris 8 bits : `rgbT2rgba`, octet bas de la clé
                    Some(Trns::Gray(k)) if self.depth == 8 => l != (k & 0xFF) as u8,
                    // gris 1/2/4 bits : `_trns_mask`, clé brute remise à l'échelle
                    Some(Trns::Gray(k)) => u32::from(l) != u32::from(k) * 255 / u32::from(max),
                    _ => true,
                };
                [l, l, l, if alpha { 255 } else { 0 }]
            }
            Mode::Gray16 => {
                let v = self.sample(row, x);
                let opaque = !matches!(self.trns, Some(Trns::Gray(k)) if k == v);
                [hi(v), hi(v), hi(v), if opaque { 255 } else { 0 }]
            }
            Mode::Rgb => {
                let (r, g, b) = (
                    self.sample(row, 3 * x),
                    self.sample(row, 3 * x + 1),
                    self.sample(row, 3 * x + 2),
                );
                if self.depth == 16 {
                    let opaque = !matches!(self.trns, Some(Trns::Rgb(kr, kg, kb))
                        if hi(r) == hi(kr) && hi(g) == hi(kg) && hi(b) == hi(kb));
                    [hi(r), hi(g), hi(b), if opaque { 255 } else { 0 }]
                } else {
                    let opaque = !matches!(self.trns, Some(Trns::Rgb(kr, kg, kb))
                        if r == kr & 0xFF && g == kg & 0xFF && b == kb & 0xFF);
                    [r as u8, g as u8, b as u8, if opaque { 255 } else { 0 }]
                }
            }
            Mode::Palette => self.palette[usize::from(self.sample(row, x))],
            Mode::GrayA => {
                let (l, a) = (row[2 * x], row[2 * x + 1]);
                [l, l, l, a]
            }
            Mode::GrayA16 => {
                let (l, a) = (hi(self.sample(row, 2 * x)), hi(self.sample(row, 2 * x + 1)));
                [l, l, l, a]
            }
            Mode::Rgba4 => {
                let mut px = [0u8; 4];
                for (c, v) in px.iter_mut().enumerate() {
                    let s = self.sample(row, 4 * x + c);
                    *v = if self.depth == 16 { hi(s) } else { s as u8 };
                }
                px
            }
        }
    }
}

/// Décode un PNG en RGBA comme `_open_rgba(png_bytes)` avec Pillow 12.3.0.
///
/// `cancelled` est consulté avant chaque chunk (avant et après l'image) et avant chaque ligne
/// décodée : [`PngError::Cancelled`] dès qu'il rend `true`.
pub fn decode_rgba(data: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    if data.len() < SIGNATURE.len() || &data[..8] != SIGNATURE {
        // Autre format : délégué si Pillow pourrait le lire, sinon illisible pour Pillow aussi
        // (`UnidentifiedImageError`).
        return Err(if crate::identify::pillow_may_open(data) {
            PngError::Unsupported("pas un PNG")
        } else {
            PngError::Unreadable("cannot identify image file")
        });
    }
    let mut cur = Cursor { data, pos: 8 };
    let (mut h, first_idat) = read_header(&mut cur, cancelled)?;
    let mode = h.mode.unwrap_or(Mode::Bit);
    let conv = Converter {
        mode,
        depth: h.depth,
        trns: h.trns.clone(),
        palette: if mode == Mode::Palette {
            palette_table(&h)?
        } else {
            Vec::new()
        },
    };
    let (w, ht) = (h.width, h.height);
    let size = (w as usize)
        .checked_mul(ht as usize)
        .and_then(|n| n.checked_mul(4))
        .map_or_else(|| unreadable("taille"), Ok)?;
    let bits = u64::from(channels(h.color)) * u64::from(h.depth);
    let bpp = ((bits / 8) as usize).max(1);
    let passes: &[(u32, u32, u32, u32)] = if h.interlace { &ADAM7 } else { &[(0, 0, 1, 1)] };
    // (passe, largeur, hauteur, octets par ligne sans l'octet de filtre)
    let mut layout = Vec::new();
    let mut total: u64 = 0;
    for &(x0, y0, dx, dy) in passes {
        if x0 >= w || y0 >= ht {
            continue;
        }
        let pw = (w - x0).div_ceil(dx);
        let ph = (ht - y0).div_ceil(dy);
        let row = (u64::from(pw) * bits).div_ceil(8);
        total += (row + 1) * u64::from(ph);
        layout.push(((x0, y0, dx, dy), pw, ph, row as usize));
    }
    let total = usize::try_from(total).map_or_else(|_| unreadable("taille"), Ok)?;
    let mut stream = IdatStream {
        cur,
        remaining: first_idat,
        block: &[],
        used: 0,
        z: Inflater::new(
            total
                .checked_add(1)
                .map_or_else(|| unreadable("taille"), Ok)?,
        ),
        total,
    };
    // tampons alloués seulement quand une ligne brute est disponible : une taille annoncée
    // sans données derrière n'engage pas de mémoire
    let mut out = Vec::new();
    let mut at = 0usize;
    for ((x0, y0, dx, dy), pw, ph, row) in layout {
        // lignes défiltrées hors du tampon de miniz, qui sert d'historique aux références
        let (mut line, mut prev) = (Vec::new(), Vec::new());
        for r in 0..ph {
            if cancelled() {
                return Err(PngError::Cancelled);
            }
            stream.ensure(at + 1 + row)?;
            if out.is_empty() {
                out = alloc_zeroed(size)?;
            }
            if r == 0 {
                line = alloc(row)?;
                prev = alloc(row)?;
            }
            line.copy_from_slice(&stream.z.out[at + 1..at + 1 + row]);
            // ligne précédente de la même passe (zéros pour la première)
            unfilter(
                stream.z.out[at],
                &mut line,
                (r > 0).then_some(&prev[..]),
                bpp,
            )?;
            let y = (y0 + r * dy) as usize;
            for i in 0..pw as usize {
                let x = x0 as usize + i * dx as usize;
                let o = (y * w as usize + x) * 4;
                out[o..o + 4].copy_from_slice(&conv.pixel(&line, i));
            }
            std::mem::swap(&mut line, &mut prev);
            at += 1 + row;
        }
    }
    stream.finish()?;
    stream.load_end(&mut h, cancelled)?;
    Ok(Rgba {
        width: w,
        height: ht,
        rgba: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn never() -> bool {
        false
    }

    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut crc = crc32fast::Hasher::new();
        crc.update(kind);
        crc.update(data);
        let mut v = (data.len() as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(data);
        v.extend_from_slice(&crc.finalize().to_be_bytes());
        v
    }

    /// PNG gris 8 bits de `w × h`, lignes non filtrées.
    fn gray_png(w: u32, h: u32) -> Vec<u8> {
        let mut raw = Vec::new();
        for y in 0..h {
            raw.push(0);
            raw.extend((0..w).map(|x| (x + y) as u8));
        }
        let zdata = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 0, 0, 0, 0]);
        let mut png = SIGNATURE.to_vec();
        png.extend(chunk(b"IHDR", &ihdr));
        png.extend(chunk(b"IDAT", &zdata));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn decode_un_png_simple() {
        let img = decode_rgba(&gray_png(3, 2), &never).unwrap();
        assert_eq!((img.width, img.height), (3, 2));
        assert_eq!(&img.rgba[..8], &[0, 0, 0, 255, 1, 1, 1, 255]);
    }

    #[test]
    fn annulation_verifiee_avant_chaque_ligne() {
        let png = gray_png(4, 10);
        let calls = Cell::new(0);
        let count = || {
            calls.set(calls.get() + 1);
            false
        };
        decode_rgba(&png, &count).unwrap();
        // IHDR et IDAT (avant l'image), 10 lignes, IEND (après)
        assert_eq!(calls.get(), 2 + 10 + 1);

        // annulation au milieu : arrêt à la 4e ligne, sans lire la suite
        let calls = Cell::new(0);
        let stop = || {
            calls.set(calls.get() + 1);
            calls.get() > 2 + 3
        };
        assert_eq!(decode_rgba(&png, &stop), Err(PngError::Cancelled));
        assert_eq!(calls.get(), 2 + 4);
    }

    #[test]
    fn garde_anti_bombe_au_double_de_max_image_pixels() {
        let header = |w: u32, h: u32| {
            let mut ihdr = w.to_be_bytes().to_vec();
            ihdr.extend_from_slice(&h.to_be_bytes());
            ihdr.extend_from_slice(&[1, 0, 0, 0, 0]);
            let mut png = SIGNATURE.to_vec();
            png.extend(chunk(b"IHDR", &ihdr));
            png.extend(chunk(b"IDAT", &[]));
            png
        };
        let limit = 2 * MAX_IMAGE_PIXELS;
        assert_eq!(limit, 178_956_970);
        let mut cur = Cursor {
            data: &header(limit as u32 + 1, 1),
            pos: 8,
        };
        assert_eq!(
            read_header(&mut cur, &never).err(),
            Some(PngError::Unreadable("bombe de décompression"))
        );
        // à la limite : accepté (Pillow avertit seulement)
        let png = header(limit as u32, 1);
        let mut cur = Cursor { data: &png, pos: 8 };
        assert!(read_header(&mut cur, &never).is_ok());
        let png = header(u32::MAX, u32::MAX);
        let mut cur = Cursor { data: &png, pos: 8 };
        assert!(read_header(&mut cur, &never).is_err());
    }

    #[test]
    fn autres_formats_delegues() {
        for data in [&b"GIF89a"[..], b"\xff\xd8\xff\xe0", b"\x89PNG\r\n"] {
            assert!(matches!(
                decode_rgba(data, &never),
                Err(PngError::Unsupported(_))
            ));
        }
        // Aucun greffon de Pillow ne les accepte : illisibles, comme pour Python.
        for data in [&b""[..], b"garbage", b"\x89PNGgarbage"] {
            assert!(matches!(
                decode_rgba(data, &never),
                Err(PngError::Unreadable(_))
            ));
        }
    }

    #[test]
    fn aucune_panique_sur_les_prefixes_et_octets_alteres() {
        let png = gray_png(9, 5);
        for n in 0..png.len() {
            let _ = decode_rgba(&png[..n], &never);
        }
        for i in 0..png.len() {
            for x in [0x01, 0x80, 0xFF] {
                let mut d = png.clone();
                d[i] ^= x;
                let _ = decode_rgba(&d, &never);
            }
        }
    }
}
