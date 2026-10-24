//! Décodage GIF vers RGBA avec la sémantique de `_open_rgba` (`tools/telmi2flam/telmi2flam.py`)
//! exécuté avec **Pillow 12.3.0** (#459) : première image seulement, mêmes pixels, et image
//! illisible exactement quand Python lève une exception.
//!
//! - `GifImageFile._open` et `_seek(0)` : écran logique, palette globale (absente ou « inutile »,
//!   c'est-à-dire l'identité grise : mode `L`), extensions (transparence du GCE), descripteur
//!   d'image (cadre qui agrandit l'écran s'il déborde), palette locale ;
//! - lectures courtes de Python reproduites : un champ manquant lève `IndexError` ou
//!   `struct.error`, donc image illisible ;
//! - `load_prepare` : fond rempli de l'index transparent du GCE (sinon 0) ;
//! - décodeur LZW de `GifDecode.c` porté tel quel (codes invalides, entrelacement, fin de
//!   l'image au dernier pixel, code de fin prématuré suivi de données) ;
//! - `convert("RGBA")` : palette (noir opaque au-delà), index transparent à alpha 0 ; en `L`,
//!   `convert_transparent`.
//! - garde anti-bombe de Pillow seulement (`2 × MAX_IMAGE_PIXELS`), mémoire allouée quand les
//!   données arrivent.
//!
//! Code porté (tag `12.3.0` de <https://github.com/python-pillow/Pillow>) : `src/libImaging/GifDecode.c`
//! (`ImagingGifDecode`), décision D3 de `PORT-RUST.md` (code de Pillow sous licence HPND).
//!
//! ---
//!
//! Ce module est un travail dérivé de Pillow, distribué sous la licence MIT-CMU (HPND) :
//!
//! The Python Imaging Library (PIL) is
//!
//! Copyright © 1997-2011 by Secret Labs AB
//! Copyright © 1995-2011 by Fredrik Lundh and contributors
//!
//! Pillow is the friendly PIL fork. It is
//!
//! Copyright © 2010 by Jeffrey 'Alex' Clark and contributors
//!
//! Like PIL, Pillow is licensed under the open source MIT-CMU License:
//!
//! By obtaining, using, and/or copying this software and/or its associated
//! documentation, you agree that you have read, understood, and will comply
//! with the following terms and conditions:
//!
//! Permission to use, copy, modify and distribute this software and its
//! documentation for any purpose and without fee is hereby granted,
//! provided that the above copyright notice appears in all copies, and that
//! both that copyright notice and this permission notice appear in supporting
//! documentation, and that the name of Secret Labs AB or the author not be
//! used in advertising or publicity pertaining to distribution of the software
//! without specific, written prior permission.
//!
//! SECRET LABS AB AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS
//! SOFTWARE, INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS.
//! IN NO EVENT SHALL SECRET LABS AB OR THE AUTHOR BE LIABLE FOR ANY SPECIAL,
//! INDIRECT OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM
//! LOSS OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE
//! OR OTHER TORTIOUS ACTION, ARISING OUT OF OR IN CONNECTION WITH THE USE OR
//! PERFORMANCE OF THIS SOFTWARE.
//!
//! (Texte aussi dans `crates/telmi2flam/LICENSE-Pillow.txt`, repris dans les notices.)

use crate::png::{PngError, Rgba, MAX_IMAGE_PIXELS};

type Result<T> = std::result::Result<T, PngError>;

fn unreadable<T>(why: &'static str) -> Result<T> {
    Err(PngError::Unreadable(why))
}

/// `fp.read(n)` de Python sur des octets en mémoire.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn read(&mut self, n: usize) -> &'a [u8] {
        let start = self.pos.min(self.data.len());
        let end = start.saturating_add(n).min(self.data.len());
        self.pos = end;
        &self.data[start..end]
    }

    /// `GifImageFile.data` : sous-bloc suivant, `None` pour un bloc nul ou la fin du fichier.
    fn sub_block(&mut self) -> Option<&'a [u8]> {
        match self.read(1) {
            [n] if *n != 0 => Some(self.read(usize::from(*n))),
            _ => None,
        }
    }
}

fn i16le(s: &[u8], o: usize) -> Result<u16> {
    match s.get(o..o + 2) {
        Some(b) => Ok(u16::from_le_bytes([b[0], b[1]])),
        None => unreadable("champ GIF tronqué"),
    }
}

fn byte(s: &[u8], o: usize) -> Result<u8> {
    s.get(o)
        .copied()
        .map_or_else(|| unreadable("champ GIF tronqué"), Ok)
}

/// `_is_palette_needed` : vrai sauf pour la palette identité grise (`i // 3 == p[i] == p[i + 1]
/// == p[i + 2]`, évalué paresseusement : `IndexError` seulement si les comparaisons
/// précédentes sont vraies).
fn palette_needed(p: &[u8]) -> Result<bool> {
    for i in (0..p.len()).step_by(3) {
        let idx = i / 3;
        if idx != usize::from(p[i]) {
            return Ok(true);
        }
        if p[i] != byte(p, i + 1)? {
            return Ok(true);
        }
        if byte(p, i + 1)? != byte(p, i + 2)? {
            return Ok(true);
        }
    }
    Ok(false)
}

const GIFBITS: i32 = 12;
const GIFTABLE: usize = 1 << GIFBITS;
const GIFBUFFER: usize = 1 << GIFBITS;
/// `ImageFile.MAXBLOCK` : taille des lectures d'`ImageFile.load`.
const MAX_BLOCK: usize = 65536;

/// Retour d'un appel à [`Lzw::decode`] (`ImagingGifDecode`).
enum Step {
    /// Octets consommés ; le décodeur attend la suite (sous-bloc incomplet, ou code de fin :
    /// `break` puis `return`, l'état est conservé et Pillow le relance sur la suite du fichier).
    Consumed(usize),
    /// Dernier pixel écrit (`return -1`, `errcode` nul).
    Done,
    /// `IMAGING_CODEC_BROKEN`, `CONFIG` ou `OVERRUN` : exception.
    Broken,
}

/// Image d'index (mode `P` ou `L`) : `Image.core.fill` de `load_prepare`, allouée au premier
/// pixel écrit.
struct Canvas {
    width: usize,
    height: usize,
    fill: u8,
    pixels: Vec<u8>,
}

impl Canvas {
    fn put(&mut self, x: usize, y: usize, v: u8) -> Result<()> {
        if self.pixels.is_empty() {
            let n = self.width * self.height;
            if self.pixels.try_reserve_exact(n).is_err() {
                return unreadable("mémoire insuffisante");
            }
            self.pixels.resize(n, self.fill);
        }
        self.pixels[y * self.width + x] = v;
        Ok(())
    }
}

/// État de `GIFDECODERSTATE` et de `ImagingCodecState` (contexte remis à zéro par Pillow).
struct Lzw {
    bits: i32,
    xoff: usize,
    yoff: usize,
    xsize: usize,
    ysize: usize,
    x: usize,
    y: usize,
    /// 0 : à initialiser ; 1 : réinitialisation de la table ; 2 : premier symbole ; 3.
    state: u8,
    interlace: u8,
    step: usize,
    clear: i32,
    end: i32,
    next: i32,
    codesize: i32,
    codemask: i32,
    bitbuffer: i64,
    bitcount: i32,
    blocksize: usize,
    lastcode: i32,
    lastdata: u8,
    bufferindex: usize,
    data: Vec<u8>,
    link: Vec<u16>,
    buffer: Vec<u8>,
}

impl Lzw {
    fn new(bits: i32, interlaced: bool, tile: (usize, usize, usize, usize)) -> Self {
        Lzw {
            bits,
            xoff: tile.0,
            yoff: tile.1,
            xsize: tile.2,
            ysize: tile.3,
            x: 0,
            y: 0,
            state: 0,
            interlace: u8::from(interlaced),
            step: 0,
            clear: 0,
            end: 0,
            next: 0,
            codesize: 0,
            codemask: 0,
            bitbuffer: 0,
            bitcount: 0,
            blocksize: 0,
            lastcode: 0,
            lastdata: 0,
            bufferindex: 0,
            data: vec![0; GIFTABLE],
            link: vec![0; GIFTABLE],
            buffer: vec![0; GIFBUFFER],
        }
    }

    /// Macro `NEWLINE` : vrai quand l'image est complète.
    fn newline(&mut self) -> bool {
        self.x = 0;
        self.y += self.step;
        while self.y >= self.ysize {
            match self.interlace {
                1 => {
                    self.y = 4;
                    self.interlace = 2;
                }
                2 => {
                    self.step = 4;
                    self.y = 2;
                    self.interlace = 3;
                }
                3 => {
                    self.step = 2;
                    self.y = 1;
                    self.interlace = 0;
                }
                _ => return true,
            }
        }
        false
    }

    /// `ImagingGifDecode(im, state, buf, bytes)`.
    fn decode(
        &mut self,
        buf: &[u8],
        canvas: &mut Canvas,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Step> {
        if self.state == 0 {
            if !(0..=12).contains(&self.bits) {
                return Ok(Step::Broken);
            }
            self.clear = 1 << self.bits;
            self.end = self.clear + 1;
            if self.interlace != 0 {
                self.interlace = 1;
                self.step = 8;
            } else {
                self.step = 1;
            }
            self.state = 1;
        }
        let mut ptr = 0usize;
        loop {
            if self.state == 1 {
                self.next = self.clear + 2;
                self.codesize = self.bits + 1;
                self.codemask = (1 << self.codesize) - 1;
                self.bufferindex = GIFBUFFER;
                self.state = 2;
            }
            // chaîne à copier : reste du tampon (rempli par la droite), ou premier octet
            let (start, len, single);
            if self.bufferindex < GIFBUFFER {
                start = self.bufferindex;
                len = GIFBUFFER - self.bufferindex;
                single = None;
                self.bufferindex = GIFBUFFER;
            } else {
                while self.bitcount < self.codesize {
                    if self.blocksize > 0 {
                        let c = buf[ptr];
                        ptr += 1;
                        self.blocksize -= 1;
                        self.bitbuffer |= i64::from(c) << self.bitcount;
                        self.bitcount += 8;
                    } else {
                        // nouveau sous-bloc, décodé seulement s'il est entier
                        let Some(&c) = buf.get(ptr) else {
                            return Ok(Step::Consumed(ptr));
                        };
                        if buf.len() - ptr < usize::from(c) + 1 {
                            return Ok(Step::Consumed(ptr));
                        }
                        self.blocksize = usize::from(c);
                        ptr += 1;
                    }
                }
                let mut c = (self.bitbuffer as i32) & self.codemask;
                self.bitbuffer >>= self.codesize;
                self.bitcount -= self.codesize;
                if c == self.clear {
                    if self.state != 2 {
                        self.state = 1;
                    }
                    continue;
                }
                if c == self.end {
                    return Ok(Step::Consumed(ptr));
                }
                if self.state == 2 {
                    if c > self.clear {
                        return Ok(Step::Broken);
                    }
                    self.lastdata = c as u8;
                    self.lastcode = c;
                    self.state = 3;
                } else {
                    let thiscode = c;
                    if c > self.next {
                        return Ok(Step::Broken);
                    }
                    if c == self.next {
                        if self.bufferindex == 0 {
                            return Ok(Step::Broken);
                        }
                        self.bufferindex -= 1;
                        self.buffer[self.bufferindex] = self.lastdata;
                        c = self.lastcode;
                    }
                    while c >= self.clear {
                        if self.bufferindex == 0 || c as usize >= GIFTABLE {
                            return Ok(Step::Broken);
                        }
                        self.bufferindex -= 1;
                        self.buffer[self.bufferindex] = self.data[c as usize];
                        c = i32::from(self.link[c as usize]);
                    }
                    self.lastdata = c as u8;
                    if (self.next as usize) < GIFTABLE {
                        self.data[self.next as usize] = c as u8;
                        self.link[self.next as usize] = self.lastcode as u16;
                        if self.next == self.codemask && self.codesize < GIFBITS {
                            self.codesize += 1;
                            self.codemask = (1 << self.codesize) - 1;
                        }
                        self.next += 1;
                    }
                    self.lastcode = thiscode;
                }
                start = 0;
                len = 1;
                single = Some(self.lastdata);
            }
            if self.y >= self.ysize {
                return Ok(Step::Broken);
            }
            // copie pixel par pixel : les raccourcis de Pillow donnent le même résultat sans
            // transparence (première image : `transparency = -1`)
            for k in 0..len {
                let v = single.unwrap_or_else(|| self.buffer[start + k]);
                if self.x == 0 && cancelled() {
                    return Err(PngError::Cancelled);
                }
                canvas.put(self.xoff + self.x, self.yoff + self.y, v)?;
                self.x += 1;
                if self.x >= self.xsize && self.newline() {
                    return Ok(Step::Done);
                }
            }
        }
    }
}

/// Décode un GIF en RGBA comme `_open_rgba(gif_bytes)` avec Pillow 12.3.0.
pub fn decode_rgba(data: &[u8], cancelled: &dyn Fn() -> bool) -> Result<Rgba> {
    let mut r = Reader { data, pos: 0 };
    // `_open`
    let s = r.read(13);
    if !(s.starts_with(b"GIF87a") || s.starts_with(b"GIF89a")) {
        return unreadable("pas un GIF");
    }
    let (mut width, mut height) = (u32::from(i16le(s, 6)?), u32::from(i16le(s, 8)?));
    let flags = byte(s, 10)?;
    let mut global = None;
    if flags & 128 != 0 {
        byte(s, 11)?; // `info["background"]`
        let p = r.read(3 << ((flags & 7) + 1));
        if palette_needed(p)? {
            global = Some(p);
        }
    }
    // `_seek(0)`
    let mut s = r.read(1);
    if s.is_empty() || s == b";" {
        return unreadable("pas d'image dans le GIF");
    }
    let mut transparency = None;
    // (x0, y0, x1, y1), entrelacement, palette locale (`Some(None)` : identité grise), bits
    let mut frame = None;
    loop {
        if s.is_empty() {
            s = r.read(1);
        }
        if s.is_empty() || s == b";" {
            break;
        }
        if s == b"!" {
            let label = r.read(1);
            let block = r.sub_block();
            let label = byte(label, 0)?;
            match (label, block) {
                (249, Some(b)) => {
                    // extension de contrôle graphique
                    let f = byte(b, 0)?;
                    if f & 1 != 0 {
                        transparency = Some(byte(b, 3)?);
                    }
                    i16le(b, 1)?; // `info["duration"]`
                }
                (254, mut b) => {
                    // commentaire : sous-blocs jusqu'au bloc nul, sans la boucle finale
                    while b.is_some() {
                        b = r.sub_block();
                    }
                    s = b"";
                    continue;
                }
                (255, Some(b)) if b.starts_with(b"NETSCAPE2.0") => {
                    r.sub_block();
                }
                _ => {}
            }
            while r.sub_block().is_some() {}
        } else if s == b"," {
            let d = r.read(9);
            let (x0, y0) = (u32::from(i16le(d, 0)?), u32::from(i16le(d, 2)?));
            let (x1, y1) = (x0 + u32::from(i16le(d, 4)?), y0 + u32::from(i16le(d, 6)?));
            if x1 > width || y1 > height {
                width = width.max(x1);
                height = height.max(y1);
                if u64::from(width) * u64::from(height) > 2 * MAX_IMAGE_PIXELS {
                    return unreadable("bombe de décompression");
                }
            }
            let f = byte(d, 8)?;
            let interlaced = f & 64 != 0;
            let mut local = None;
            if f & 128 != 0 {
                let p = r.read(3 << ((f & 7) + 1));
                local = Some(palette_needed(p)?.then_some(p));
            }
            let bits = byte(r.read(1), 0)?;
            frame = Some(((x0, y0, x1, y1), interlaced, local, bits, r.pos));
            break;
        }
        s = b"";
    }
    let Some(((x0, y0, x1, y1), interlaced, local, bits, offset)) = frame else {
        return unreadable("pas d'image dans le GIF");
    };
    // palette de l'image : locale, sinon globale ; mode `P`, ou `L` sans palette
    // (palette locale identité : mode `L`, mais `self.palette` reprend la palette globale,
    // que `Image.load` pose sur le cœur gris : `convert("RGBA")` la traverse ; avec une
    // transparence, `convert_transparent` refuse ce cœur `P`)
    let palette = match local {
        Some(Some(p)) => Some(p),
        Some(None) if global.is_some() && transparency.is_some() => {
            return unreadable("transparence sur un gris à palette");
        }
        Some(None) | None => global,
    };
    if u64::from(width) * u64::from(height) > 2 * MAX_IMAGE_PIXELS {
        return unreadable("bombe de décompression");
    }
    // `decoder.setimage(im, extents)` : zone non vide (le cadre est toujours dans l'écran)
    if x1 <= x0 || y1 <= y0 {
        return unreadable("zone vide");
    }
    let mut canvas = Canvas {
        width: width as usize,
        height: height as usize,
        fill: transparency.unwrap_or(0),
        pixels: Vec::new(),
    };
    let mut lzw = Lzw::new(
        i32::from(bits),
        interlaced,
        (
            x0 as usize,
            y0 as usize,
            (x1 - x0) as usize,
            (y1 - y0) as usize,
        ),
    );
    // boucle d'`ImageFile.load` : lectures de `MAX_BLOCK` octets, reste non consommé gardé
    let mut pos = offset;
    let mut pending: Vec<u8> = Vec::new();
    loop {
        let chunk = &data[pos.min(data.len())..(pos.saturating_add(MAX_BLOCK)).min(data.len())];
        if chunk.is_empty() {
            return unreadable("image tronquée");
        }
        pos += chunk.len();
        pending.extend_from_slice(chunk);
        match lzw.decode(&pending, &mut canvas, cancelled)? {
            Step::Done => break,
            Step::Broken => return unreadable("flux LZW corrompu"),
            Step::Consumed(n) => {
                pending.drain(..n);
            }
        }
    }
    if canvas.pixels.is_empty() {
        let n = canvas.width * canvas.height;
        if canvas.pixels.try_reserve_exact(n).is_err() {
            return unreadable("mémoire insuffisante");
        }
        canvas.pixels.resize(n, canvas.fill);
    }
    // `convert("RGBA")`
    let mut table = [[0u8, 0, 0, 255]; 256];
    match palette {
        Some(p) => {
            for (e, rgb) in table.iter_mut().zip(p.as_chunks::<3>().0) {
                e[..3].copy_from_slice(rgb);
            }
        }
        None => {
            for (i, e) in table.iter_mut().enumerate() {
                *e = [i as u8, i as u8, i as u8, 255];
            }
        }
    }
    if let Some(t) = transparency {
        table[usize::from(t)][3] = 0;
    }
    let n = canvas.pixels.len();
    let mut rgba = Vec::new();
    if rgba.try_reserve_exact(n * 4).is_err() {
        return unreadable("mémoire insuffisante");
    }
    for &i in &canvas.pixels {
        rgba.extend_from_slice(&table[usize::from(i)]);
    }
    Ok(Rgba {
        width,
        height,
        rgba,
    })
}
