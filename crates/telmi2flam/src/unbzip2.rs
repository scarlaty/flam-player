//! Décodeur bzip2 (méthode ZIP 12), écrit d'après la description publique du format (en-tête
//! `BZh1`..`BZh9`, blocs Burrows-Wheeler + move-to-front + Huffman, CRC de bloc et de flux).
//! Il reproduit `bz2.BZ2Decompressor` de Python 3.14 (libbzip2 1.0.8) : mêmes contrôles et
//! même granularité de sortie (un bloc n'est rendu qu'une fois entièrement décodé), un seul
//! flux (les octets qui suivent la fin du flux sont ignorés, comme `unused_data`).
//!
//! Pourquoi pas une crate : `bzip2-sys` et `libbz2-rs-sys` sont sous licence bzip2-1.0.6 (hors
//! `ALLOWED_LICENSES`), et `bzip2-rs` (MIT/Apache-2.0) échoue sur un bloc compressé plus grand
//! que la taille de bloc annoncée, ce qui arrive avec des données incompressibles (MP3) : ce
//! cas est couvert par le test `donnees_incompressibles`.
//!
//! Écarts connus (flux corrompus ou incomplets seulement, voir PORT-RUST.md §3) : les blocs
//! « randomised » (bzip2 antérieur à 0.9.5, 1999) sont refusés ; un flux sans marqueur de fin
//! rend son dernier bloc en entier, alors que Python peut en perdre la fin selon la
//! croissance de son tampon de sortie.

use crate::zipcodec::{data, Halt, Out, OutBuf, StreamDecoder};

/// `BZ_MAX_SELECTORS` de libbzip2 1.0.8 : les sélecteurs au-delà sont lus puis ignorés.
const MAX_SELECTORS: usize = 18002;
/// `BZ_MAX_ALPHA_SIZE`.
const MAX_ALPHA_SIZE: usize = 258;
/// `BZ_MAX_CODE_LEN`.
const MAX_CODE_LEN: usize = 23;
/// Taille d'un groupe de symboles partageant une table de Huffman (`BZ_G_SIZE`).
const GROUP_SIZE: u32 = 50;
/// Octets compressés à attendre au minimum avant de tenter un bloc, hors fin de tour.
const RETRY_MIN: usize = 64 * 1024;

/// Table du CRC-32 de bzip2 (polynôme 0x04C11DB7, bit de poids fort d'abord).
const CRC_TABLE: [u32; 256] = {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = (i as u32) << 24;
        let mut k = 0;
        while k < 8 {
            c = if c & 0x8000_0000 != 0 {
                (c << 1) ^ 0x04C1_1DB7
            } else {
                c << 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
};

#[inline]
fn crc_update(crc: u32, b: u8) -> u32 {
    (crc << 8) ^ CRC_TABLE[((crc >> 24) as u8 ^ b) as usize]
}

/// Manque d'octets (bloc incomplet) ou arrêt.
enum Stop {
    Need,
    Halt(Halt),
}

impl From<Halt> for Stop {
    fn from(h: Halt) -> Self {
        Stop::Halt(h)
    }
}

fn bad(message: &str) -> Stop {
    Stop::Halt(data(format!("Invalid data stream : {message}")))
}

/// Lecteur de bits, poids fort d'abord.
struct Bits<'a> {
    data: &'a [u8],
    pos: u64,
}

impl Bits<'_> {
    #[inline]
    fn bit(&mut self) -> Result<u32, Stop> {
        let byte = *self.data.get((self.pos >> 3) as usize).ok_or(Stop::Need)?;
        let v = (byte >> (7 - (self.pos & 7))) & 1;
        self.pos += 1;
        Ok(u32::from(v))
    }

    #[inline]
    fn bits(&mut self, n: u32) -> Result<u32, Stop> {
        let mut v = 0;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }
}

/// Table de décodage d'un groupe (`BZ2_hbCreateDecodeTables`).
struct Table {
    limit: [i32; MAX_CODE_LEN],
    base: [i32; MAX_CODE_LEN],
    perm: [u16; MAX_ALPHA_SIZE],
    /// Nombre d'entrées de `perm` remplies : au-delà, libbzip2 lirait une mémoire non
    /// initialisée (code incomplet) ; ici, erreur.
    count: usize,
    min_len: u32,
}

impl Table {
    fn new(lens: &[u8]) -> Table {
        let min_len = u32::from(*lens.iter().min().unwrap_or(&1));
        let max_len = u32::from(*lens.iter().max().unwrap_or(&1));
        let mut perm = [0u16; MAX_ALPHA_SIZE];
        let mut count = 0;
        for i in min_len..=max_len {
            for (j, &l) in lens.iter().enumerate() {
                if u32::from(l) == i {
                    perm[count] = j as u16;
                    count += 1;
                }
            }
        }
        let mut base = [0i32; MAX_CODE_LEN];
        for &l in lens {
            base[usize::from(l) + 1] += 1;
        }
        for i in 1..MAX_CODE_LEN {
            base[i] += base[i - 1];
        }
        let mut limit = [0i32; MAX_CODE_LEN];
        let mut vec = 0i32;
        for i in min_len as usize..=max_len as usize {
            vec += base[i + 1] - base[i];
            limit[i] = vec - 1;
            vec <<= 1;
        }
        for i in min_len as usize + 1..=max_len as usize {
            base[i] = ((limit[i - 1] + 1) << 1) - base[i];
        }
        Table {
            limit,
            base,
            perm,
            count,
            min_len,
        }
    }

    /// Un symbole (`GET_MTF_VAL`, hors changement de groupe).
    fn decode(&self, r: &mut Bits) -> Result<u16, Stop> {
        let mut zn = self.min_len;
        let mut zvec = r.bits(zn)? as i32;
        loop {
            if zn > 20 {
                return Err(bad("code de Huffman trop long"));
            }
            if zvec <= self.limit[zn as usize] {
                break;
            }
            zn += 1;
            zvec = (zvec << 1) | r.bit()? as i32;
        }
        let idx = zvec - self.base[zn as usize];
        if idx < 0 || idx as usize >= MAX_ALPHA_SIZE || idx as usize >= self.count {
            return Err(bad("code de Huffman invalide"));
        }
        Ok(self.perm[idx as usize])
    }
}

/// `bz2.BZ2Decompressor`.
pub struct Bzip2Decoder {
    /// Octets compressés non consommés, depuis le début du bloc en cours.
    buf: Vec<u8>,
    /// Position (en bits) du premier bit non consommé dans `buf`.
    bit: u64,
    /// Taille de bloc annoncée (1..=9) ; 0 tant que l'en-tête n'est pas lu.
    level: u32,
    done: bool,
    combined_crc: u32,
    /// Taille de `buf` à atteindre avant une nouvelle tentative (hors fin de tour).
    retry_at: usize,
    tt: Vec<u32>,
    out: OutBuf,
}

impl Default for Bzip2Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Bzip2Decoder {
    pub fn new() -> Self {
        Bzip2Decoder {
            buf: Vec::new(),
            bit: 0,
            level: 0,
            done: false,
            combined_crc: 0,
            retry_at: 0,
            tt: Vec::new(),
            out: OutBuf::new(),
        }
    }

    /// En-tête `BZh1`..`BZh9`, contrôlé octet par octet dès réception (comme libbzip2).
    fn header(&mut self) -> Result<bool, Halt> {
        for (i, &b) in self.buf.iter().take(4).enumerate() {
            let ok = match i {
                0 => b == b'B',
                1 => b == b'Z',
                2 => b == b'h',
                _ => (b'1'..=b'9').contains(&b),
            };
            if !ok {
                return Err(data("Invalid data stream : en-tête bzip2 invalide"));
            }
        }
        if self.buf.len() < 4 {
            return Ok(false);
        }
        self.level = u32::from(self.buf[3] - b'0');
        self.buf.drain(..4);
        self.bit = 0;
        Ok(true)
    }
}

impl StreamDecoder for Bzip2Decoder {
    fn feed(&mut self, input: &[u8], flush: bool, out: &mut Out) -> Result<bool, Halt> {
        if self.done {
            return Ok(true);
        }
        self.buf.extend_from_slice(input);
        if self.level == 0 && !self.header()? {
            return Ok(false);
        }
        loop {
            let avail = self.buf.len();
            if !flush && avail < self.retry_at {
                return Ok(false);
            }
            let mut r = Bits {
                data: &self.buf,
                pos: self.bit,
            };
            let res = block(
                &mut r,
                self.level,
                &mut self.tt,
                &mut self.out,
                &mut self.combined_crc,
                out,
            );
            let pos = r.pos;
            match res {
                Ok(end) => {
                    self.out.flush(out)?;
                    if end {
                        self.done = true;
                        self.buf = Vec::new();
                        return Ok(true);
                    }
                    let bytes = (pos >> 3) as usize;
                    self.buf.drain(..bytes);
                    self.bit = pos & 7;
                    self.retry_at = 0;
                }
                Err(Stop::Need) => {
                    // Bloc incomplet : rien n'a été rendu (la sortie ne commence qu'après le
                    // décodage complet du bloc). Nouvelle tentative avec deux fois plus
                    // d'octets, ou en fin de tour.
                    self.retry_at = avail.saturating_mul(2).max(RETRY_MIN);
                    return Ok(false);
                }
                Err(Stop::Halt(h)) => return Err(h),
            }
        }
    }
}

/// Un bloc ou la fin du flux (`true`). Rien n'est rendu avant que le bloc soit entièrement lu :
/// un manque d'octets ([`Stop::Need`]) laisse l'état intact.
fn block(
    r: &mut Bits,
    level: u32,
    tt: &mut Vec<u32>,
    outbuf: &mut OutBuf,
    combined: &mut u32,
    out: &mut Out,
) -> Result<bool, Stop> {
    let m0 = r.bits(8)?;
    if m0 == 0x17 {
        for want in [0x72, 0x45, 0x38, 0x50, 0x90] {
            if r.bits(8)? != want {
                return Err(bad("marqueur de fin de flux"));
            }
        }
        let stored = r.bits(32)?;
        if stored != *combined {
            return Err(bad("CRC du flux"));
        }
        return Ok(true);
    }
    if m0 != 0x31 {
        return Err(bad("marqueur de bloc"));
    }
    for want in [0x41, 0x59, 0x26, 0x53, 0x59] {
        if r.bits(8)? != want {
            return Err(bad("marqueur de bloc"));
        }
    }
    let stored_crc = r.bits(32)?;
    if r.bit()? == 1 {
        return Err(Stop::Halt(data(
            "bloc bzip2 « randomised » (bzip2 antérieur à 0.9.5) non pris en charge",
        )));
    }
    let orig_ptr = r.bits(24)?;
    if orig_ptr > 10 + 100_000 * level {
        return Err(bad("origPtr"));
    }

    // Octets utilisés.
    let in_use16 = r.bits(16)?;
    let mut seq_to_unseq = [0u8; 256];
    let mut n_in_use = 0usize;
    for i in 0..16 {
        if in_use16 & (0x8000 >> i) != 0 {
            let bits16 = r.bits(16)?;
            for j in 0..16 {
                if bits16 & (0x8000 >> j) != 0 {
                    seq_to_unseq[n_in_use] = (i * 16 + j) as u8;
                    n_in_use += 1;
                }
            }
        }
    }
    if n_in_use == 0 {
        return Err(bad("aucun octet utilisé"));
    }
    let alpha_size = n_in_use + 2;

    // Sélecteurs (move-to-front sur les numéros de table).
    let n_groups = r.bits(3)? as usize;
    if !(2..=6).contains(&n_groups) {
        return Err(bad("nombre de tables de Huffman"));
    }
    let n_selectors = r.bits(15)? as usize;
    if n_selectors < 1 {
        return Err(bad("aucun sélecteur"));
    }
    let mut selectors = Vec::with_capacity(n_selectors.min(MAX_SELECTORS));
    let mut pos: [u8; 6] = [0, 1, 2, 3, 4, 5];
    for i in 0..n_selectors {
        let mut j = 0usize;
        while r.bit()? == 1 {
            j += 1;
            if j >= n_groups {
                return Err(bad("sélecteur hors limites"));
            }
        }
        if i < MAX_SELECTORS {
            let v = pos[j];
            pos.copy_within(0..j, 1);
            pos[0] = v;
            selectors.push(v);
        }
    }

    // Tables de Huffman (longueurs codées en delta).
    let mut tables = Vec::with_capacity(n_groups);
    let mut lens = [0u8; MAX_ALPHA_SIZE];
    for _ in 0..n_groups {
        let mut curr = r.bits(5)? as i32;
        for len in lens.iter_mut().take(alpha_size) {
            loop {
                if !(1..=20).contains(&curr) {
                    return Err(bad("longueur de code hors limites"));
                }
                if r.bit()? == 0 {
                    break;
                }
                if r.bit()? == 0 {
                    curr += 1;
                } else {
                    curr -= 1;
                }
            }
            *len = curr as u8;
        }
        tables.push(Table::new(&lens[..alpha_size]));
    }

    // Symboles MTF/RLE2 jusqu'à la fin de bloc.
    let eob = (n_in_use + 1) as u16;
    let nblock_max = 100_000 * level as usize;
    let mut unzftab = [0u32; 256];
    let mut mtf: [u8; 256] = std::array::from_fn(|i| i as u8);
    let mut group_no = 0usize;
    let mut group_pos = 0u32;
    let mut table = &tables[0];
    tt.clear();
    let mut next_sym = |r: &mut Bits, group_no: &mut usize, group_pos: &mut u32| {
        if *group_pos == 0 {
            if *group_no >= selectors.len() {
                return Err(bad("sélecteurs épuisés"));
            }
            table = &tables[usize::from(selectors[*group_no])];
            *group_no += 1;
            *group_pos = GROUP_SIZE;
        }
        *group_pos -= 1;
        table.decode(r)
    };
    let mut sym = next_sym(r, &mut group_no, &mut group_pos)?;
    loop {
        if sym == eob {
            break;
        }
        if sym <= 1 {
            // RUNA / RUNB : répétition de l'octet en tête de liste.
            let mut es: u64 = 0;
            let mut n: u64 = 1;
            loop {
                if n >= 2 * 1024 * 1024 {
                    return Err(bad("répétition trop longue"));
                }
                es += if sym == 0 { n } else { 2 * n };
                n *= 2;
                sym = next_sym(r, &mut group_no, &mut group_pos)?;
                if sym > 1 {
                    break;
                }
            }
            let uc = seq_to_unseq[usize::from(mtf[0])];
            if tt.len() as u64 + es > nblock_max as u64 {
                return Err(bad("bloc trop grand"));
            }
            unzftab[usize::from(uc)] += es as u32;
            tt.resize(tt.len() + es as usize, u32::from(uc));
            continue;
        }
        if tt.len() >= nblock_max {
            return Err(bad("bloc trop grand"));
        }
        let nn = usize::from(sym - 1);
        let v = mtf[nn];
        mtf.copy_within(0..nn, 1);
        mtf[0] = v;
        let uc = seq_to_unseq[usize::from(v)];
        unzftab[usize::from(uc)] += 1;
        tt.push(u32::from(uc));
        sym = next_sym(r, &mut group_no, &mut group_pos)?;
    }
    let nblock = tt.len();
    if orig_ptr as usize >= nblock {
        return Err(bad("origPtr hors du bloc"));
    }

    // Transformée de Burrows-Wheeler inverse.
    let mut cftab = [0u32; 256];
    let mut sum = 0u32;
    for (c, &n) in cftab.iter_mut().zip(unzftab.iter()) {
        *c = sum;
        sum += n;
    }
    for i in 0..nblock {
        let uc = (tt[i] & 0xFF) as usize;
        let j = cftab[uc] as usize;
        tt[j] |= (i as u32) << 8;
        cftab[uc] += 1;
    }

    // Sortie : décodage RLE1 (4 octets égaux puis un compte), CRC du bloc.
    let mut t_pos = tt[orig_ptr as usize] >> 8;
    let get = |t_pos: &mut u32| -> Result<u8, Stop> {
        let e = *tt.get(*t_pos as usize).ok_or_else(|| bad("index BWT"))?;
        *t_pos = e >> 8;
        Ok(e as u8)
    };
    let end = nblock + 1;
    let mut crc = 0xFFFF_FFFFu32;
    let mut k0 = get(&mut t_pos)?;
    let mut used = 1usize;
    loop {
        if used > end {
            return Err(bad("répétition au-delà de la fin du bloc"));
        }
        if used == end {
            break;
        }
        let ch = k0;
        let mut len = 1usize;
        let k1 = get(&mut t_pos)?;
        used += 1;
        if k1 != k0 {
            k0 = k1;
        } else if used != end {
            len = 2;
            let k1 = get(&mut t_pos)?;
            used += 1;
            if used != end {
                if k1 != k0 {
                    k0 = k1;
                } else {
                    len = 3;
                    let k1 = get(&mut t_pos)?;
                    used += 1;
                    if used != end {
                        if k1 != k0 {
                            k0 = k1;
                        } else {
                            let k1 = get(&mut t_pos)?;
                            used += 1;
                            len = usize::from(k1) + 4;
                            k0 = get(&mut t_pos)?;
                            used += 1;
                        }
                    }
                }
            }
        }
        for _ in 0..len {
            crc = crc_update(crc, ch);
            outbuf.push(ch, out)?;
        }
    }
    let crc = !crc;
    if crc != stored_crc {
        return Err(bad("CRC de bloc"));
    }
    *combined = combined.rotate_left(1) ^ crc;
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(chunks: &[&[u8]], flush_each: bool) -> (Result<bool, Halt>, Vec<u8>) {
        let mut d = Bzip2Decoder::new();
        let mut got = Vec::new();
        let mut res = Ok(false);
        for (i, c) in chunks.iter().enumerate() {
            let flush = flush_each || i + 1 == chunks.len();
            res = d.feed(c, flush, &mut |b: &[u8]| {
                got.extend_from_slice(b);
                Ok(())
            });
            if res.is_err() {
                break;
            }
        }
        (res, got)
    }

    #[test]
    fn crc_de_reference() {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in b"123456789" {
            crc = crc_update(crc, b);
        }
        assert_eq!(!crc, 0xFC89_1918);
    }

    #[test]
    fn flux_vide_et_en_tete() {
        // bz2.compress(b"") de Python.
        let empty = b"BZh9\x17\x72\x45\x38\x50\x90\x00\x00\x00\x00";
        assert_eq!(decode(&[empty], false), (Ok(true), vec![]));
        // Octet par octet : même résultat.
        let parts: Vec<&[u8]> = empty.chunks(1).collect();
        assert_eq!(decode(&parts, true), (Ok(true), vec![]));
        // En-tête partiel ou fin de flux partielle : pas d'erreur, pas de fin.
        assert_eq!(decode(&[b"B"], true), (Ok(false), vec![]));
        assert_eq!(decode(&[b"BZh9\x17"], true), (Ok(false), vec![]));
        // Octet invalide dès qu'il arrive.
        assert!(matches!(decode(&[b"X"], true).0, Err(Halt::Data(_))));
        assert!(matches!(decode(&[b"BZh0"], true).0, Err(Halt::Data(_))));
        assert!(matches!(decode(&[b"BZh9\x18"], true).0, Err(Halt::Data(_))));
        // CRC du flux faux.
        let mut bad_crc = empty.to_vec();
        bad_crc[13] = 1;
        assert!(matches!(decode(&[&bad_crc], true).0, Err(Halt::Data(_))));
        // Après la fin du flux, la suite est ignorée.
        let mut two = empty.to_vec();
        two.extend_from_slice(b"garbage");
        assert_eq!(decode(&[&two], true), (Ok(true), vec![]));
    }

    #[test]
    fn destinataire_qui_refuse() {
        // Un bloc de "abc" (bz2.compress(b"abc" * 30000, 1) de Python, tronqué au besoin) : on
        // vérifie seulement que le refus du destinataire arrête le décodage.
        let data = std::fs::read(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/vectors/archive/bzip2_image.zip"
        ))
        .unwrap();
        let start = data.windows(4).position(|w| w == b"BZh9").unwrap();
        let mut d = Bzip2Decoder::new();
        let r = d.feed(&data[start..], true, &mut |_| Err(()));
        assert_eq!(r, Err(Halt::Sink));
    }
}
