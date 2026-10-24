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
//! PNG atypiques (#459), avec le comportement exact de Pillow, erreurs comprises :
//! - APNG : première image seulement (`acTL`, séquences de `fcTL`/`fdAT`, zone du `fcTL` lu
//!   avant les données, reste de l'image au pixel nul du mode ; en entrelacé, `ZipDecode.c`
//!   ignore le décalage de la zone), données continuées en `fdAT`, arrêt de `load_end` au `fcTL`
//!   suivant d'un APNG animé ;
//! - `IHDR` absent en tête ou répété : taille du dernier, mode du dernier couple valide,
//!   entrelacement collant ; `PLTE` gardé seulement sous un mode palette ;
//! - transparence : valeur de Pillow rangée selon le mode courant à chaque `tRNS` (le dernier
//!   gagne, y compris après l'image), clé brute de `_png_trns_key` (premier `tRNS` de 2 ou 6
//!   octets avant l'IDAT) et `IHDR` à position fixe de `_png_ihdr` pour `_trns_mask`, puis
//!   conversions de Pillow (`convert("L")`, `"I"`, `"RGB"`, `convert_transparent`) quel que
//!   soit le mode réellement décodé.
//!
//! Les JPEG (signature `FF D8 FF`) et les GIF sont confiés à [`crate::jpeg`] (#459, D7) et
//! [`crate::gif`]. Seul cas encore **délégué** ici ([`PngError::Unsupported`], code 7, décision
//! D4) : un autre format que PNG, JPEG et GIF que Pillow pourrait lire (voir [`crate::identify`] ; des données qu'aucun greffon de
//! Pillow n'accepte sont illisibles).

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
    /// Autre format que PNG que Pillow pourrait lire : à déléguer à Python
    /// (`unsupported_format`).
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
    /// Sortie pleine avec un octet en attente : zlib s'arrête là (rien de plus n'est lu).
    halted: bool,
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
/// Distance maximale de deflate.
const DEFLATE_WINDOW: usize = 32768;
const INFLATE_FLAGS: u32 = TINFL_FLAG_PARSE_ZLIB_HEADER
    | TINFL_FLAG_HAS_MORE_INPUT
    | TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;

impl Inflater {
    fn new(max: usize) -> Self {
        Inflater {
            state: Box::default(),
            out: Vec::new(),
            pos: 0,
            done: false,
            halted: false,
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
        self.halted = false;
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
    ///
    /// Sortie pleine, zlib s'arrête au premier octet à écrire : il décode encore les codes
    /// (un code invalide est une erreur), mais ne contrôle une distance trop lointaine qu'en
    /// écrivant la copie. miniz la refuse dès son décodage : un échec survenu sortie pleine
    /// est rejoué depuis l'état d'avant l'appel, dans un tampon jetable à historique fictif
    /// (le décodage des codes ne dépend pas du contenu produit). S'il produit l'octet suivant,
    /// zlib se serait arrêté là sans erreur (`halted`).
    fn run(&mut self, input: &[u8], limit: usize) -> Result<Step> {
        let limit = limit.min(self.max);
        let mut consumed = 0;
        while !self.done && !self.halted {
            if self.out.len() <= self.pos && self.out.len() < limit {
                self.grow(limit)?;
            }
            let end = limit.min(self.out.len());
            // Instantané (environ 10 Kio) à chaque appel une fois le tampon à sa taille finale :
            // mesuré (#473, n1) sur un PNG de 1600 × 1200 en 194 IDAT de 8 Kio, l'écart reste
            // dans le bruit de la mesure (moins de 2 Mio copiés pour 5,8 Mio décompressés).
            let before = (end == limit).then(|| (self.state.clone(), self.pos));
            let (status, used, written) = decompress(
                &mut self.state,
                &input[consumed..],
                &mut self.out[..end],
                self.pos,
                INFLATE_FLAGS,
            );
            self.pos += written;
            match status {
                TINFLStatus::Done => self.done = true,
                TINFLStatus::NeedsMoreInput => {}
                TINFLStatus::HasMoreOutput if self.out.len() < limit => self.grow(limit)?,
                TINFLStatus::HasMoreOutput => self.halted = true,
                _ => match before {
                    Some((state, pos)) if self.pos == limit => {
                        self.state = state;
                        if !self.replay(&input[consumed..], limit - pos)? {
                            return Ok(Step::Broken);
                        }
                        self.halted = true;
                    }
                    _ => return Ok(Step::Broken),
                },
            }
            consumed += used;
            if status == TINFLStatus::NeedsMoreInput {
                break;
            }
        }
        Ok(Step::Consumed(consumed))
    }

    /// Rejoue `input` depuis l'état courant dans un tampon jetable dont la position vaut la
    /// fenêtre de deflate : vrai si `n` octets puis un de plus sont produits sans erreur.
    ///
    /// Tampon circulaire de 64 Kio (mode « wrapping » de miniz : aucune distance n'y est trop
    /// lointaine), sans engager la taille de ce qui est produit.
    fn replay(&mut self, input: &[u8], n: usize) -> Result<bool> {
        const RING: usize = 2 * DEFLATE_WINDOW;
        let mut ring = Vec::new();
        if ring.try_reserve_exact(RING).is_err() {
            return unreadable("mémoire insuffisante");
        }
        ring.resize(RING, 0);
        let flags = TINFL_FLAG_PARSE_ZLIB_HEADER | TINFL_FLAG_HAS_MORE_INPUT;
        let (mut consumed, mut produced, mut pos) = (0usize, 0usize, 0usize);
        loop {
            let (status, used, written) =
                decompress(&mut self.state, &input[consumed..], &mut ring, pos, flags);
            consumed += used;
            produced += written;
            pos = (pos + written) % RING;
            if produced > n {
                return Ok(true);
            }
            if status != TINFLStatus::HasMoreOutput {
                return Ok(false);
            }
        }
    }

    /// Ce que zlib fait encore de `input` une fois la sortie pleine : il traite la fin de bloc
    /// et vérifie l'Adler-32, mais s'arrête au premier octet à produire. `Ok(Some(true))` : il
    /// reste un octet à produire ; `Ok(None)` : flux invalide.
    ///
    /// zlib ne contrôle une distance trop lointaine qu'au moment d'écrire la copie (état
    /// `MATCH`, après le test de place libre), alors que miniz la refuse dès son décodage : la
    /// sonde décompresse donc dans un tampon jetable dont la position vaut la fenêtre maximale
    /// de deflate (32 Kio), où aucune distance n'est trop lointaine. Le décompresseur ne sert
    /// plus ensuite (fin d'image, ou `reset` avant le texte suivant).
    fn probe(&mut self, input: &[u8]) -> Result<Option<bool>> {
        if self.halted {
            return Ok(Some(true));
        }
        if self.done || input.is_empty() {
            // Sans entrée, rien ne peut être produit : ni arrêté (`halted`) ni fini, le dernier
            // appel a rendu `NeedsMoreInput`, donc sans copie ni littéral en attente.
            return Ok(Some(false));
        }
        let mut scratch = Vec::new();
        if scratch.try_reserve_exact(DEFLATE_WINDOW + 1).is_err() {
            return unreadable("mémoire insuffisante");
        }
        scratch.resize(DEFLATE_WINDOW + 1, 0);
        let (status, _, written) = decompress(
            &mut self.state,
            input,
            &mut scratch,
            DEFLATE_WINDOW,
            INFLATE_FLAGS,
        );
        if written > 0 {
            // zlib s'arrête sur cet octet : ce que miniz lit ensuite (Adler-32…) ne compte pas
            return Ok(Some(true));
        }
        Ok(match status {
            TINFLStatus::Done | TINFLStatus::NeedsMoreInput | TINFLStatus::HasMoreOutput => {
                Some(false)
            }
            _ => None,
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
        // Sortie pleine : `unconsumed_tail` non vide s'il reste de l'entrée là où zlib s'arrête.
        // Il lit encore le symbole qui produirait l'octet suivant (littéral, ou copie avec sa
        // distance, contrôlée seulement à l'écriture), mais pas un octet de bloc stocké. miniz
        // s'arrête au même point et rend la même consommation (octets entiers de son tampon de
        // bits restitués ; toute l'entrée si elle finit avant ou sur ce symbole, #473). Écart
        // connu, dans le sens permissif : si le premier octet d'un bloc stocké était déjà dans
        // le tampon de bits (lecture anticipée après un bloc dynamique), miniz le garde
        // (`RawStoreFirstByte`) ; une entrée qui finit sur cet octet est alors lue, où Python
        // lève `ValueError` (PORT-RUST.md §4.4).
        if z.halted {
            if used < input.len() {
                return unreadable("texte compressé trop grand");
            }
            return Ok(Some(MAX_TEXT_CHUNK));
        }
        match z.probe(&input[used..])? {
            Some(true) => return unreadable("texte compressé trop grand"),
            None => return Ok(None),
            Some(false) => {}
        }
    }
    Ok(Some(z.pos.min(MAX_TEXT_CHUNK)))
}

/// Valeur de `info["transparency"]` de Pillow, typée comme `chunk_tRNS` l'a rangée selon le
/// mode **courant** au moment du chunk (un `IHDR` répété peut changer ce mode ensuite).
#[derive(Debug, Clone, PartialEq, Eq)]
enum PilTrns {
    /// Entier : gris (`"1"` : 0 ou 255, `"L"`, `"I;16"` : `i16(s)`), ou index unique d'une
    /// palette (motif `^\xff*\x00\xff*$`, `putpalettealpha`).
    Int(u32),
    /// Triplet RGB (`i16(s)`, `i16(s, 2)`, `i16(s, 4)`).
    Tuple([u16; 3]),
    /// Octets : alpha de chaque entrée d'une palette (`putpalettealphas`).
    Bytes(Vec<u8>),
}

/// `chunk_tRNS` : valeur rangée selon le mode courant ; `Ok(None)` : le mode ignore le chunk
/// (valeur précédente conservée). Un chunk trop court pour `i16` lève `struct.error`.
fn pil_trns(mode: Option<Mode>, s: &[u8]) -> Result<Option<PilTrns>> {
    let i16 = |o: usize| -> Result<u16> {
        match s.get(o..o + 2) {
            Some(b) => Ok(u16::from_be_bytes([b[0], b[1]])),
            None => unreadable("tRNS court"),
        }
    };
    Ok(Some(match mode {
        Some(Mode::Palette) => {
            // `_simple_palette = re.compile(b"^\xff*\x00\xff*$")` : un seul zéro entouré de
            // 0xFF ; le `$` de Python accepte aussi un saut de ligne final
            let body = s.strip_suffix(b"\n").unwrap_or(s);
            let simple = body.iter().filter(|&&c| c == 0).count() == 1
                && body.iter().all(|&c| c == 0 || c == 0xFF);
            if simple {
                PilTrns::Int(body.iter().position(|&c| c == 0).unwrap_or(0) as u32)
            } else {
                PilTrns::Bytes(s.to_vec())
            }
        }
        Some(Mode::Bit) => PilTrns::Int(if i16(0)? != 0 { 255 } else { 0 }),
        Some(Mode::Gray | Mode::Gray16) => PilTrns::Int(u32::from(i16(0)?)),
        Some(Mode::Rgb) => PilTrns::Tuple([i16(0)?, i16(2)?, i16(4)?]),
        Some(Mode::GrayA | Mode::GrayA16 | Mode::Rgba4) | None => return Ok(None),
    }))
}

/// Mode, profondeur et type de couleur retenus par le dernier `IHDR` valide (`_MODES`) :
/// `chunk_IHDR` garde le mode précédent si le couple est inconnu (`KeyError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PixFormat {
    mode: Mode,
    depth: u8,
    color: u8,
}

/// Zone décodée (`tile`) : `bbox` du dernier `fcTL` avant les données, sinon toute l'image.
type Extent = (u64, u64, u64, u64);

/// État de `PngStream` : lu avant les données d'image, puis complété par `load_end`.
struct Header {
    /// `im_size` : taille du dernier `IHDR` lu (même de mode inconnu).
    size: (u32, u32),
    format: Option<PixFormat>,
    /// `im_info["interlace"]` : posé par un `IHDR` entrelacé, jamais retiré.
    interlace: bool,
    palette: Option<Vec<u8>>,
    trns: Option<PilTrns>,
    /// APNG : `im_n_frames`, `_seq_num`, `im_info["bbox"]` et `im_info["default_image"]`.
    n_frames: Option<u32>,
    seq: Option<u64>,
    bbox: Option<Extent>,
    default_image: bool,
    text_memory: usize,
    /// Décompresseur des chunks de texte, réutilisé d'un chunk à l'autre.
    text: Inflater,
}

impl Header {
    /// `chunk_IHDR` (avant ou après les données).
    fn ihdr(&mut self, s: &[u8]) -> Result<()> {
        if s.len() < 13 {
            return unreadable("IHDR court");
        }
        self.size = (
            u32::from_be_bytes([s[0], s[1], s[2], s[3]]),
            u32::from_be_bytes([s[4], s[5], s[6], s[7]]),
        );
        if let Some(mode) = pillow_mode(s[8], s[9]) {
            self.format = Some(PixFormat {
                mode,
                depth: s[8],
                color: s[9],
            });
        }
        if s[12] != 0 {
            self.interlace = true;
        }
        if s[11] != 0 {
            return unreadable("méthode de filtrage inconnue");
        }
        Ok(())
    }

    /// `chunk_tRNS` (avant ou après les données), selon le mode courant.
    fn trns(&mut self, s: &[u8]) -> Result<()> {
        if let Some(t) = pil_trns(self.format.map(|f| f.mode), s)? {
            self.trns = Some(t);
        }
        Ok(())
    }

    /// `chunk_acTL` (la valeur ne sert qu'à `is_animated`, lu avant les données).
    fn actl(&mut self, s: &[u8]) -> Result<()> {
        if s.len() < 8 {
            return unreadable("acTL court");
        }
        if self.n_frames.is_some() {
            self.n_frames = None;
            return Ok(());
        }
        let n = u32::from_be_bytes([s[0], s[1], s[2], s[3]]);
        if n != 0 && n <= 0x8000_0000 {
            self.n_frames = Some(n);
        }
        Ok(())
    }

    /// `chunk_fcTL` : numéro de séquence et cadre dans `im_size`.
    fn fctl(&mut self, s: &[u8]) -> Result<()> {
        if s.len() < 26 {
            return unreadable("fcTL court");
        }
        let at = |o: usize| u64::from(u32::from_be_bytes([s[o], s[o + 1], s[o + 2], s[o + 3]]));
        let seq = at(0);
        let ok = match self.seq {
            None => seq == 0,
            Some(n) => n + 1 == seq,
        };
        if !ok {
            return unreadable("séquence APNG");
        }
        self.seq = Some(seq);
        let (w, h, x, y) = (at(4), at(8), at(12), at(16));
        if x + w > u64::from(self.size.0) || y + h > u64::from(self.size.1) {
            return unreadable("cadre APNG hors de l'image");
        }
        self.bbox = Some((x, y, x + w, y + h));
        Ok(())
    }

    /// Début de `chunk_fdAT` : longueur et numéro de séquence (4 octets lus).
    fn fdat_seq(&mut self, cur: &mut Cursor, len: usize) -> Result<()> {
        if len < 4 {
            return unreadable("fdAT court");
        }
        let s = cur.safe_read(4)?;
        let seq = u64::from(u32::from_be_bytes([s[0], s[1], s[2], s[3]]));
        if self.seq.is_none_or(|n| n + 1 != seq) {
            return unreadable("séquence APNG");
        }
        self.seq = Some(seq);
        Ok(())
    }

    /// `chunk_IDAT` : zone à décoder (`im_tile`).
    fn tile(&mut self) -> Extent {
        match self.bbox {
            Some(b) => b,
            None => {
                if self.n_frames.is_some() {
                    self.default_image = true;
                }
                (0, 0, u64::from(self.size.0), u64::from(self.size.1))
            }
        }
    }

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

/// `PngImageFile._open` : chunks jusqu'aux premières données d'image (`IDAT`, ou `fdAT` d'un
/// APNG dont le numéro de séquence suit). Rend l'en-tête, la zone à décoder et la longueur des
/// données restantes du chunk (le curseur est sur ces données).
fn read_header(cur: &mut Cursor, cancelled: &dyn Fn() -> bool) -> Result<(Header, Extent, usize)> {
    let mut h = Header {
        size: (0, 0),
        format: None,
        interlace: false,
        palette: None,
        trns: None,
        n_frames: None,
        seq: None,
        bbox: None,
        default_image: false,
        text_memory: 0,
        text: Inflater::new(MAX_TEXT_CHUNK),
    };
    let mut data = None;
    loop {
        if cancelled() {
            return Err(PngError::Cancelled);
        }
        let Some((kind, len)) = cur.chunk_header() else {
            return unreadable("en-tête de chunk invalide");
        };
        let s: &[u8] = match &kind {
            b"IDAT" => {
                data = Some((h.tile(), len));
                break;
            }
            b"fdAT" => {
                // `chunk_fdAT` : séquence, puis `chunk_IDAT` (fin de l'en-tête, sans CRC)
                h.fdat_seq(cur, len)?;
                data = Some((h.tile(), len - 4));
                break;
            }
            b"IEND" => break,
            b"IHDR" => {
                let s = cur.safe_read(len)?;
                h.ihdr(s)?;
                s
            }
            b"PLTE" => {
                let s = cur.safe_read(len)?;
                if h.format.is_some_and(|f| f.mode == Mode::Palette) {
                    h.palette = Some(s.to_vec());
                }
                s
            }
            b"tRNS" => {
                let s = cur.safe_read(len)?;
                h.trns(s)?;
                s
            }
            b"acTL" => {
                let s = cur.safe_read(len)?;
                h.actl(s)?;
                s
            }
            b"fcTL" => {
                let s = cur.safe_read(len)?;
                h.fctl(s)?;
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
    }
    let (w, ht) = h.size;
    if h.format.is_none() || w == 0 || ht == 0 {
        return unreadable("mode ou taille invalide");
    }
    if u64::from(w) * u64::from(ht) > 2 * MAX_IMAGE_PIXELS {
        return unreadable("bombe de décompression");
    }
    let Some((tile, len)) = data else {
        return unreadable("pas d'IDAT");
    };
    // `decoder.setimage(im, extents)` : zone non vide, dans l'image (un `IHDR` lu après le
    // `fcTL` peut avoir réduit l'image)
    let (x0, y0, x1, y1) = tile;
    if x1 <= x0 || y1 <= y0 || x1 > u64::from(w) || y1 > u64::from(ht) {
        return unreadable("zone hors de l'image");
    }
    Ok((h, tile, len))
}

/// `_png_ihdr` de `telmi2flam.py` : (profondeur, type de couleur) lus à position fixe, si le
/// premier chunk est un `IHDR`.
fn fixed_ihdr(data: &[u8]) -> Option<(u8, u8)> {
    (data.len() >= 26 && &data[12..16] == b"IHDR").then(|| (data[24], data[25]))
}

/// `_png_trns_key` de `telmi2flam.py` : premier `tRNS` avant `IDAT`/`IEND`, parcouru naïvement
/// depuis l'octet 8. `Err` : `struct.error` sur un corps tronqué (l'image est illisible).
fn raw_trns_key(data: &[u8]) -> Result<Option<PilTrns>> {
    let mut pos = 8usize;
    while pos.checked_add(8).is_some_and(|e| e <= data.len()) {
        let n = u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]);
        let n = n as usize;
        let kind = &data[pos + 4..pos + 8];
        let start = pos + 8;
        let body = &data[start.min(data.len())..start.saturating_add(n).min(data.len())];
        if kind == b"tRNS" {
            let i16 = |o: usize| -> Result<u16> {
                match body.get(o..o + 2) {
                    Some(b) => Ok(u16::from_be_bytes([b[0], b[1]])),
                    None => unreadable("tRNS tronqué"),
                }
            };
            return Ok(match n {
                2 => Some(PilTrns::Int(u32::from(i16(0)?))),
                6 => Some(PilTrns::Tuple([i16(0)?, i16(2)?, i16(4)?])),
                _ => None,
            });
        }
        if kind == b"IDAT" || kind == b"IEND" {
            return Ok(None);
        }
        pos = match pos.checked_add(12).and_then(|p| p.checked_add(n)) {
            Some(p) => p,
            None => return Ok(None),
        };
    }
    Ok(None)
}

/// Table RGB(A) de 256 entrées de Pillow : `PLTE` (au plus 256 entrées), complété par du noir
/// opaque (palette absente comprise). Avec `alphas`, la transparence de `convert("RGBA")`
/// (`putpalettealphas`, même au-delà du `PLTE`, ou `putpalettealpha` d'un index).
fn palette_table(palette: Option<&[u8]>, alphas: Option<&PilTrns>) -> Result<Vec<[u8; 4]>> {
    let mut table = vec![[0u8, 0, 0, 255]; 256];
    if let Some(p) = palette {
        if p.len() / 3 > 256 {
            return unreadable("palette trop longue");
        }
        for (entry, rgb) in table.iter_mut().zip(p.as_chunks::<3>().0) {
            entry[..3].copy_from_slice(rgb);
        }
    }
    match alphas {
        Some(PilTrns::Bytes(a)) => {
            if a.len() > 256 {
                return unreadable("tRNS plus long que 256");
            }
            for (entry, &alpha) in table.iter_mut().zip(a) {
                entry[3] = alpha;
            }
        }
        Some(PilTrns::Int(i)) => match table.get_mut(*i as usize) {
            Some(entry) => entry[3] = 0,
            None => return unreadable("index tRNS hors palette"),
        },
        Some(PilTrns::Tuple(_)) => return unreadable("transparence de palette invalide"),
        None => {}
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
    fn next_block(&mut self, h: &mut Header) -> Result<()> {
        while self.remaining == 0 {
            self.cur.read(4);
            match self.cur.chunk_header() {
                Some((kind, len)) if &kind == b"IDAT" || &kind == b"DDAT" => self.remaining = len,
                Some((kind, len)) if &kind == b"fdAT" => {
                    // suite des données d'un APNG : `chunk_fdAT` vérifie la séquence
                    h.fdat_seq(&mut self.cur, len)?;
                    self.remaining = len - 4;
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
    fn ensure(&mut self, n: usize, h: &mut Header) -> Result<()> {
        while self.z.pos < n {
            if self.z.done {
                // flux terminé avant la dernière ligne : Pillow lit jusqu'au bout et échoue
                return unreadable("données d'image incomplètes");
            }
            if self.used >= self.block.len() {
                self.next_block(h)?;
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
    /// gestionnaires comptent, et la transparence (`tRNS`), que `convert("RGBA")` lit ensuite.
    /// Un APNG animé s'arrête au `fcTL` de l'image suivante.
    fn load_end(
        mut self,
        h: &mut Header,
        animated: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
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
                b"fcTL" if animated => return Ok(()),
                b"fcTL" => h.fctl(self.cur.safe_read(len)?)?,
                b"acTL" => h.actl(self.cur.safe_read(len)?)?,
                b"fdAT" => {
                    // séquence, puis `chunk_IDAT` : `EOFError`, données sautées
                    h.fdat_seq(&mut self.cur, len)?;
                    self.cur.safe_read(len - 4)?;
                }
                b"tRNS" => h.trns(self.cur.safe_read(len)?)?,
                b"IHDR" => h.ihdr(self.cur.safe_read(len)?)?,
                // la palette de l'image est copiée à l'ouverture : un `PLTE` tardif est sans
                // effet
                b"PLTE" => {
                    self.cur.safe_read(len)?;
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

/// Pixel tel que Pillow le range en mémoire après le décodage (mode de l'image).
#[derive(Debug, Clone, Copy)]
enum PilPixel {
    /// `"1"` (0 ou 255) et `"L"` (remis à l'échelle 8 bits).
    L(u8),
    I16(u16),
    Rgb([u8; 3]),
    P(u8),
    La(u8, u8),
    Rgba([u8; 4]),
}

/// `rgb2l` de Pillow (`Convert.c`) : luminance ITU-R 601-2 en virgule fixe.
fn luma(c: [u8; 3]) -> u8 {
    ((u32::from(c[0]) * 19595 + u32::from(c[1]) * 38470 + u32::from(c[2]) * 7471 + 0x8000) >> 16)
        as u8
}

/// Alpha posé par `convert("RGBA")` à partir de `info["transparency"]` (`convert_transparent`
/// pour les gris et le RGB, palette à part).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Native {
    None,
    /// Gris 8 bits égal à l'octet bas de la clé (`rgbT2rgba`).
    Gray(u8),
    /// RGB 8 bits égal aux octets bas de la clé.
    Rgb([u8; 3]),
}

/// Masque de `_trns_mask` (clé brute de `_png_trns_key`, ou valeur de Pillow).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mask {
    /// Gris 1/2/4 bits : `convert("L")` égal à `k`.
    L(u32),
    /// Gris 16 bits : `convert("I")` égal à `k`.
    I(u32),
    /// RGB 16 bits : chaque canal de `convert("RGB")` égal à l'octet haut de la clé.
    Rgb([u32; 3]),
}

/// Conversion d'une ligne brute vers RGBA, selon `_open_rgba`.
struct Converter {
    mode: Mode,
    depth: u8,
    /// Palette RGBA (alpha de `convert("RGBA")` compris si aucun masque ne s'applique).
    palette: Vec<[u8; 4]>,
    native: Native,
    mask: Option<Mask>,
    /// `PLTE` lu sous un mode palette puis `IHDR` gris (`L`) ou `LA` : `Image.load` pose la
    /// palette sur le cœur de l'image, que `convert("RGBA")` traverse alors (valeur grise
    /// utilisée comme index, alpha du canal A en `LA`).
    gray_palette: bool,
}

impl Converter {
    /// Règles de `_open_rgba` pour l'image décodée (`format`), la transparence finale de Pillow
    /// (`trns`, après `load_end`) et le fichier (`data`, relu par `_trns_mask`).
    fn new(
        format: PixFormat,
        palette: Option<&[u8]>,
        trns: Option<&PilTrns>,
        data: &[u8],
    ) -> Result<Self> {
        let mode = format.mode;
        // `_trns_mask` : seulement si `info["transparency"]` existe
        let mask = match trns {
            None => None,
            Some(t) => {
                let raw = raw_trns_key(data)?;
                let key = raw.as_ref().unwrap_or(t);
                match (fixed_ihdr(data), key) {
                    (Some((bd @ (1 | 2 | 4), 0)), PilTrns::Int(k)) => {
                        // `k` peut être un index de palette (non borné) : jamais atteint au-delà
                        let k = u64::from(*k) * 255 / ((1u64 << bd) - 1);
                        Some(Mask::L(u32::try_from(k).unwrap_or(u32::MAX)))
                    }
                    (Some((16, 0)), PilTrns::Int(k)) => Some(Mask::I(*k)),
                    (Some((16, 2)), PilTrns::Tuple(k)) => {
                        Some(Mask::Rgb(k.map(|v| u32::from(v >> 8))))
                    }
                    _ => None,
                }
            }
        };
        // `convert("RGBA")` sans masque : `convert_transparent` (gris, RGB) ou palette ;
        // `TypeError` pour des octets hors palette, `ValueError` pour un triplet sur une palette
        let mut native = Native::None;
        let mut alphas = None;
        if mask.is_none() {
            match (mode, trns) {
                (_, None) | (Mode::GrayA | Mode::GrayA16 | Mode::Rgba4, _) => {}
                (Mode::Palette, t) => alphas = t,
                (_, Some(PilTrns::Bytes(_))) => return unreadable("transparence en octets"),
                (Mode::Rgb, Some(PilTrns::Int(k))) => native = Native::Rgb([*k as u8, 0, 0]),
                (Mode::Rgb, Some(PilTrns::Tuple(k))) => native = Native::Rgb(k.map(|v| v as u8)),
                (_, Some(PilTrns::Int(k))) => native = Native::Gray(*k as u8),
                (_, Some(PilTrns::Tuple(k))) => native = Native::Gray(k[0] as u8),
            }
        } else if matches!(mask, Some(Mask::Rgb(_)))
            && mode == Mode::Gray
            && matches!(trns, Some(PilTrns::Tuple(_)))
        {
            // `convert("RGB")` d'une image `"L"` : `putpixel` d'un triplet sur un pixel gris
            // (`TypeError`)
            return unreadable("transparence en triplet sur du gris");
        }
        let gray_palette = palette.is_some() && matches!(mode, Mode::Gray | Mode::GrayA);
        if gray_palette && mode == Mode::Gray && trns.is_some() {
            // `convert_transparent` refuse un cœur `P` ; `_trns_mask` sur `L` : `illegal image
            // mode` (en `LA`, tous les masques passent : voir `pixel`)
            return unreadable("transparence sur un gris à palette");
        }
        Ok(Converter {
            mode,
            depth: format.depth,
            palette: if mode == Mode::Palette {
                palette_table(palette, alphas)?
            } else if gray_palette {
                palette_table(palette, None)?
            } else {
                Vec::new()
            },
            native,
            mask,
            gray_palette,
        })
    }

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

    /// Pixel `x` d'une ligne brute, rangé comme Pillow le décode.
    fn pil_pixel(&self, row: &[u8], x: usize) -> PilPixel {
        let hi = |v: u16| (v >> 8) as u8;
        let s8 = |i: usize| {
            let v = self.sample(row, i);
            if self.depth == 16 {
                hi(v)
            } else {
                v as u8
            }
        };
        match self.mode {
            Mode::Bit | Mode::Gray => {
                let max = (1u32 << self.depth) - 1;
                PilPixel::L((u32::from(self.sample(row, x)) * 255 / max) as u8)
            }
            Mode::Gray16 => PilPixel::I16(self.sample(row, x)),
            Mode::Rgb => PilPixel::Rgb([s8(3 * x), s8(3 * x + 1), s8(3 * x + 2)]),
            Mode::Palette => PilPixel::P(self.sample(row, x) as u8),
            Mode::GrayA => PilPixel::La(row[2 * x], row[2 * x + 1]),
            Mode::GrayA16 => {
                let l = s8(2 * x);
                PilPixel::Rgba([l, l, l, s8(2 * x + 1)])
            }
            Mode::Rgba4 => PilPixel::Rgba([s8(4 * x), s8(4 * x + 1), s8(4 * x + 2), s8(4 * x + 3)]),
        }
    }

    /// `convert("RGB")` de Pillow (sans transparence).
    fn to_rgb(&self, p: PilPixel) -> [u8; 3] {
        match p {
            PilPixel::L(l) | PilPixel::La(l, _) => [l, l, l],
            PilPixel::I16(v) => [v.min(255) as u8; 3],
            PilPixel::Rgb(c) => c,
            PilPixel::P(i) => {
                let e = self.palette[usize::from(i)];
                [e[0], e[1], e[2]]
            }
            PilPixel::Rgba(c) => [c[0], c[1], c[2]],
        }
    }

    /// `convert("L")` de Pillow (`I;16` écrêté à 255).
    fn to_l(&self, p: PilPixel) -> u8 {
        match p {
            PilPixel::L(l) | PilPixel::La(l, _) => l,
            PilPixel::I16(v) => v.min(255) as u8,
            _ => luma(self.to_rgb(p)),
        }
    }

    /// `convert("I")` de Pillow.
    fn to_i(&self, p: PilPixel) -> u32 {
        match p {
            PilPixel::I16(v) => u32::from(v),
            _ => u32::from(self.to_l(p)),
        }
    }

    fn pixel(&self, row: &[u8], x: usize) -> [u8; 4] {
        let p = self.pil_pixel(row, x);
        let mut p = p;
        if self.gray_palette {
            match p {
                PilPixel::L(l) => return self.palette[usize::from(l)],
                // masque de `_trns_mask` : `convert("L")` du cœur `PA` = luminance de la palette
                PilPixel::La(l, _) if self.mask.is_some() => {
                    let e = self.palette[usize::from(l)];
                    let rgb = [e[0], e[1], e[2]];
                    if let Some(Mask::Rgb(k)) = self.mask {
                        // masque RGB : `convert("RGB")` du cœur `PA` = couleur de la palette
                        let l = luma(rgb);
                        let alpha = if rgb.map(u32::from) == k { 0 } else { 255 };
                        return [l, l, l, alpha];
                    }
                    p = PilPixel::L(luma(rgb));
                }
                PilPixel::La(l, a) => {
                    let e = self.palette[usize::from(l)];
                    return [e[0], e[1], e[2], a];
                }
                _ => {}
            }
        }
        if let Some(mask) = self.mask {
            // `_trns_mask` puis `convert("RGB" si RGB, sinon "L")` (après `point(v / 256)`
            // pour `I;16`), `convert("RGBA")` et `putalpha(mask)`
            let hit = match mask {
                Mask::L(k) => u32::from(self.to_l(p)) == k,
                Mask::I(k) => self.to_i(p) == k,
                Mask::Rgb(k) => self.to_rgb(p).map(u32::from) == k,
            };
            let alpha = if hit { 0 } else { 255 };
            return match p {
                PilPixel::Rgb([r, g, b]) => [r, g, b, alpha],
                PilPixel::I16(v) => {
                    let l = (v >> 8) as u8;
                    [l, l, l, alpha]
                }
                _ => {
                    let l = self.to_l(p);
                    [l, l, l, alpha]
                }
            };
        }
        let gray = |l: u8| {
            let opaque = self.native != Native::Gray(l);
            [l, l, l, if opaque { 255 } else { 0 }]
        };
        match p {
            PilPixel::L(l) => gray(l),
            // `I;16` : `point(v / 256).convert("L")` d'abord
            PilPixel::I16(v) => gray((v >> 8) as u8),
            PilPixel::Rgb([r, g, b]) => {
                let opaque = self.native != Native::Rgb([r, g, b]);
                [r, g, b, if opaque { 255 } else { 0 }]
            }
            PilPixel::P(i) => self.palette[usize::from(i)],
            PilPixel::La(l, a) => [l, l, l, a],
            PilPixel::Rgba(c) => c,
        }
    }
}

/// Décode un PNG en RGBA comme `_open_rgba(png_bytes)` avec Pillow 12.3.0.
///
/// `cancelled` est consulté avant chaque chunk (avant et après l'image) et avant chaque ligne
/// décodée : [`PngError::Cancelled`] dès qu'il rend `true`.
pub fn decode_rgba(data: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    if data.starts_with(b"\xff\xd8\xff") {
        // JPEG (#459, D7) : `_accept` de `JpegImagePlugin`
        return crate::jpeg::decode_rgba(data, cancelled);
    }
    if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
        // GIF (#459) : décodé comme Pillow, première image
        return crate::gif::decode_rgba(data, cancelled);
    }
    if data.len() < SIGNATURE.len() || &data[..8] != SIGNATURE {
        // Autre format : délégué si Pillow pourrait le lire, sinon illisible pour Pillow aussi
        // (`UnidentifiedImageError`).
        return Err(if crate::identify::pillow_may_open(data) {
            PngError::Unsupported("pas un PNG")
        } else {
            PngError::Unreadable("cannot identify image file")
        });
    }
    // La transparence de `convert("RGBA")` est celle de la fin de `load_end` : un `tRNS` placé
    // après les données la change. Premier passage avec celle de l'en-tête ; s'il diffère, second
    // passage (même lecture, mêmes erreurs) avec la valeur finale.
    let (img, last) = decode_pass(data, cancelled, None)?;
    let Some(last) = last else {
        return Ok(img);
    };
    // image du premier passage libérée avant le second (pic de mémoire)
    drop(img);
    match decode_pass(data, cancelled, Some(last))? {
        (img, None) => Ok(img),
        (_, Some(_)) => unreadable("transparence finale instable"),
    }
}

/// Un passage de décodage. `final_trns` : transparence à appliquer (sinon celle de l'en-tête).
/// Rend aussi la transparence finale si elle diffère de celle appliquée.
#[allow(clippy::type_complexity)]
fn decode_pass(
    data: &[u8],
    cancelled: &dyn Fn() -> bool,
    final_trns: Option<Option<PilTrns>>,
) -> Result<(Rgba, Option<Option<PilTrns>>)> {
    let mut cur = Cursor { data, pos: 8 };
    let (mut h, tile, first_data) = read_header(&mut cur, cancelled)?;
    let Some(format) = h.format else {
        return unreadable("mode invalide");
    };
    // `n_frames` et `is_animated` de `_open`, figés avant le chargement
    let n_frames =
        u64::from(h.n_frames.unwrap_or(1)) + u64::from(h.n_frames.is_some() && h.default_image);
    let animated = n_frames > 1;
    let applied = final_trns.unwrap_or_else(|| h.trns.clone());
    // les erreurs de `_open_rgba` liées à la transparence viennent après le chargement : au
    // premier passage, une transparence d'en-tête invalide peut encore être remplacée
    let rules = Converter::new(format, h.palette.as_deref(), applied.as_ref(), data);
    let fallback;
    let conv = match &rules {
        Ok(c) => c,
        Err(_) => {
            fallback = Converter::new(format, h.palette.as_deref(), None, data)?;
            &fallback
        }
    };
    let (w, ht) = h.size;
    let size = (w as usize)
        .checked_mul(ht as usize)
        .and_then(|n| n.checked_mul(4))
        .map_or_else(|| unreadable("taille"), Ok)?;
    // zone décodée (`fcTL` d'un APNG) ; le reste de l'image garde les zéros de
    // `Image.core.new`, convertis comme un pixel brut nul
    // (en entrelacé, `ZipDecode.c` ignore le décalage : la zone part du coin haut gauche)
    let (ox, oy) = if h.interlace {
        (0, 0)
    } else {
        (tile.0 as u32, tile.1 as u32)
    };
    let (tw, th) = ((tile.2 - tile.0) as u32, (tile.3 - tile.1) as u32);
    let partial = (tw, th) != (w, ht);
    let bits = u64::from(channels(format.color)) * u64::from(format.depth);
    let bpp = ((bits / 8) as usize).max(1);
    let passes: &[(u32, u32, u32, u32)] = if h.interlace { &ADAM7 } else { &[(0, 0, 1, 1)] };
    // (passe, largeur, hauteur, octets par ligne sans l'octet de filtre)
    let mut layout = Vec::new();
    let mut total: u64 = 0;
    for &(x0, y0, dx, dy) in passes {
        if x0 >= tw || y0 >= th {
            continue;
        }
        let pw = (tw - x0).div_ceil(dx);
        let ph = (th - y0).div_ceil(dy);
        let row = (u64::from(pw) * bits).div_ceil(8);
        total += (row + 1) * u64::from(ph);
        layout.push(((x0, y0, dx, dy), pw, ph, row as usize));
    }
    let total = usize::try_from(total).map_or_else(|_| unreadable("taille"), Ok)?;
    let mut stream = IdatStream {
        cur,
        remaining: first_data,
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
            stream.ensure(at + 1 + row, &mut h)?;
            if out.is_empty() {
                out = alloc_zeroed(size)?;
                // (comme `Image.core.new`, la taille entière est engagée dès la première ligne
                // d'une zone partielle, bornée par la garde anti-bombe)
                let zero = conv.pixel(&[0u8; 8], 0);
                if partial && zero != [0; 4] {
                    for line in out.chunks_mut(4 * w as usize) {
                        if cancelled() {
                            return Err(PngError::Cancelled);
                        }
                        for px in line.as_chunks_mut::<4>().0 {
                            *px = zero;
                        }
                    }
                }
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
            let y = (oy + y0 + r * dy) as usize;
            for i in 0..pw as usize {
                let x = (ox + x0) as usize + i * dx as usize;
                let o = (y * w as usize + x) * 4;
                out[o..o + 4].copy_from_slice(&conv.pixel(&line, i));
            }
            std::mem::swap(&mut line, &mut prev);
            at += 1 + row;
        }
    }
    stream.finish()?;
    stream.load_end(&mut h, animated, cancelled)?;
    if h.palette.is_some() && !matches!(format.mode, Mode::Palette | Mode::Gray | Mode::GrayA) {
        // `PLTE` lu sous un mode palette, puis `IHDR` d'un autre mode que `L`/`LA` :
        // `Image.load` pose la palette sur l'image (`putpalette`, mode refusé)
        return unreadable("palette sur une image sans palette");
    }
    let img = Rgba {
        width: w,
        height: ht,
        rgba: out,
    };
    if h.trns == applied {
        rules?;
        Ok((img, None))
    } else {
        Ok((img, Some(h.trns)))
    }
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
        for data in [&b"BM\0\0"[..], b"\x89PNG\r\n"] {
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
