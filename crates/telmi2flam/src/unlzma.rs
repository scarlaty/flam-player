//! Décodeur LZMA (méthode ZIP 14), écrit d'après la spécification publique de LZMA (décodeur
//! de référence `LzmaSpec.cpp` d'Igor Pavlov, domaine public). Il reproduit
//! `zipfile.LZMADecompressor` de Python 3.14 : en-tête ZIP (version sur 2 octets, taille des
//! propriétés sur 2 octets, propriétés), puis un flux LZMA1 brut décodé comme liblzma
//! (`FORMAT_RAW`, `FILTER_LZMA1`, taille inconnue) : marqueur de fin facultatif, premier octet
//! du décodeur d'intervalle nul, `lc + lp <= 4`, distances bornées par la sortie et le
//! dictionnaire, état du décodeur nul après le marqueur de fin. Un flux tronqué rend tous les
//! symboles complets, sans erreur.
//!
//! Pourquoi pas `lzma-rs` (MIT) : sans taille connue il exige le marqueur de fin (Python
//! accepte un flux sans marqueur), il ne signale pas la fin du flux à l'appelant en mode
//! incrémental, et il ne fait pas les contrôles de liblzma ci-dessus.
//!
//! Écart connu (voir PORT-RUST.md §3) : le dictionnaire grandit avec la sortie au lieu d'être
//! alloué d'emblée ; un dictionnaire annoncé démesuré (jusqu'à 4 Gio) que liblzma ne
//! parviendrait pas à allouer (`MemoryError`) ne fait donc pas échouer le Rust.

use crate::zipcodec::{data, Halt, Out, OutBuf, StreamDecoder};

const TOP: u32 = 1 << 24;
/// Au-delà de ce nombre d'octets disponibles, un symbole ne peut pas manquer d'entrée : un
/// symbole lit au plus un octet par bit décodé, soit au plus 49 octets (correspondance à
/// distance maximale, puis normalisation du marqueur de fin).
const SAFE_INPUT: usize = 64;

fn corrupt() -> Halt {
    data("Corrupt input data")
}

fn options() -> Halt {
    data("Invalid or unsupported options")
}

/// Manque d'octets (symbole incomplet) ou arrêt.
enum Stop {
    Need,
    Halt(Halt),
}

impl From<Halt> for Stop {
    fn from(h: Halt) -> Self {
        Stop::Halt(h)
    }
}

struct Input<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Input<'_> {
    #[inline]
    fn byte(&mut self) -> Result<u8, Stop> {
        let b = *self.data.get(self.pos).ok_or(Stop::Need)?;
        self.pos += 1;
        Ok(b)
    }
}

/// Probabilités d'un décodeur de longueur.
#[derive(Clone)]
struct LenProbs {
    choice: u16,
    choice2: u16,
    low: [u16; 16 * 8],
    mid: [u16; 16 * 8],
    high: [u16; 256],
}

impl LenProbs {
    fn new() -> Self {
        LenProbs {
            choice: 1024,
            choice2: 1024,
            low: [1024; 16 * 8],
            mid: [1024; 16 * 8],
            high: [1024; 256],
        }
    }
}

/// Tout ce qui change pendant le décodage d'un symbole, sauf la fenêtre : copié avant chaque
/// symbole proche de la fin de l'entrée disponible, pour reprendre à l'identique quand la
/// suite arrive.
#[derive(Clone)]
struct State {
    range: u32,
    code: u32,
    state: usize,
    reps: [u32; 4],
    is_match: [u16; 192],
    is_rep: [u16; 12],
    is_rep0: [u16; 12],
    is_rep1: [u16; 12],
    is_rep2: [u16; 12],
    is_rep0_long: [u16; 192],
    dist_slot: [u16; 256],
    dist_special: [u16; 115],
    align: [u16; 16],
    len: LenProbs,
    rep_len: LenProbs,
    literal: Vec<u16>,
}

impl State {
    fn new(lc: u32, lp: u32) -> Self {
        State {
            range: 0xFFFF_FFFF,
            code: 0,
            state: 0,
            reps: [0; 4],
            is_match: [1024; 192],
            is_rep: [1024; 12],
            is_rep0: [1024; 12],
            is_rep1: [1024; 12],
            is_rep2: [1024; 12],
            is_rep0_long: [1024; 192],
            dist_slot: [1024; 256],
            dist_special: [1024; 115],
            align: [1024; 16],
            len: LenProbs::new(),
            rep_len: LenProbs::new(),
            literal: vec![1024; 0x300 << (lc + lp)],
        }
    }
}

/// Décodeur d'intervalle de liblzma (normalisation avant chaque bit).
struct Rc<'s, 'i, 'd> {
    range: &'s mut u32,
    code: &'s mut u32,
    input: &'i mut Input<'d>,
}

impl Rc<'_, '_, '_> {
    #[inline]
    fn normalize(&mut self) -> Result<(), Stop> {
        if *self.range < TOP {
            *self.range <<= 8;
            *self.code = (*self.code << 8) | u32::from(self.input.byte()?);
        }
        Ok(())
    }

    #[inline]
    fn bit(&mut self, p: &mut u16) -> Result<u32, Stop> {
        self.normalize()?;
        let bound = (*self.range >> 11) * u32::from(*p);
        if *self.code < bound {
            *self.range = bound;
            *p += (2048 - *p) >> 5;
            Ok(0)
        } else {
            *self.range -= bound;
            *self.code -= bound;
            *p -= *p >> 5;
            Ok(1)
        }
    }

    /// Bit direct (`rc_direct` de liblzma, arithmétique modulo 2^32 comprise).
    #[inline]
    fn direct(&mut self) -> Result<u32, Stop> {
        self.normalize()?;
        *self.range >>= 1;
        *self.code = self.code.wrapping_sub(*self.range);
        let mask = 0u32.wrapping_sub(*self.code >> 31);
        *self.code = self.code.wrapping_add(*self.range & mask);
        Ok(mask.wrapping_add(1))
    }

    fn tree(&mut self, probs: &mut [u16], bits: u32) -> Result<u32, Stop> {
        let mut m = 1usize;
        for _ in 0..bits {
            m = (m << 1) + self.bit(&mut probs[m])? as usize;
        }
        Ok(m as u32 - (1 << bits))
    }

    fn reverse(&mut self, probs: &mut [u16], base: usize, bits: u32) -> Result<u32, Stop> {
        let mut m = 1usize;
        let mut sym = 0u32;
        for i in 0..bits {
            let b = self.bit(&mut probs[base + m])?;
            m = (m << 1) + b as usize;
            sym |= b << i;
        }
        Ok(sym)
    }

    fn len(&mut self, p: &mut LenProbs, pos_state: usize) -> Result<u32, Stop> {
        if self.bit(&mut p.choice)? == 0 {
            return self.tree(&mut p.low[pos_state * 8..pos_state * 8 + 8], 3);
        }
        if self.bit(&mut p.choice2)? == 0 {
            return Ok(8 + self.tree(&mut p.mid[pos_state * 8..pos_state * 8 + 8], 3)?);
        }
        Ok(16 + self.tree(&mut p.high, 8)?)
    }
}

/// Fenêtre (dictionnaire) : grandit avec la sortie jusqu'à la taille du dictionnaire.
struct Window {
    buf: Vec<u8>,
    cap: usize,
    pos: usize,
    total: u64,
}

impl Window {
    #[inline]
    fn push(&mut self, b: u8) {
        if self.buf.len() < self.cap {
            if self.buf.len() == self.buf.capacity() {
                // Croissance par doublement, plafonnée à la taille du dictionnaire.
                let more = self.buf.len().max(4096).min(self.cap - self.buf.len());
                self.buf.reserve_exact(more);
            }
            self.buf.push(b);
        } else {
            self.buf[self.pos] = b;
        }
        self.pos += 1;
        if self.pos == self.cap {
            self.pos = 0;
        }
        self.total += 1;
    }

    /// Octet à la distance `dist` (1 = le dernier écrit) ; `dist` déjà validée.
    #[inline]
    fn get(&self, dist: usize) -> u8 {
        if self.buf.len() < self.cap {
            self.buf[self.buf.len() - dist]
        } else if self.pos >= dist {
            self.buf[self.pos - dist]
        } else {
            self.buf[self.pos + self.cap - dist]
        }
    }

    fn prev(&self) -> u8 {
        if self.total == 0 {
            0
        } else {
            self.get(1)
        }
    }
}

/// Symbole décodé, appliqué à la fenêtre une fois tous ses bits lus.
enum Symbol {
    Literal(u8),
    Copy { dist: usize, len: usize },
    End,
}

struct Core {
    lc: u32,
    lp_mask: u64,
    pb_mask: u64,
    /// Distance maximale (`dict.size` de liblzma : taille annoncée, au moins 4096, arrondie au
    /// multiple de 16 supérieur).
    dict_size: u64,
    /// Octets d'initialisation du décodeur d'intervalle déjà lus (5 au total).
    init: u8,
    st: State,
    win: Window,
}

impl Core {
    fn new(props: &[u8]) -> Result<Core, Halt> {
        if props.len() != 5 || props[0] > (4 * 5 + 4) * 9 + 8 {
            return Err(options());
        }
        let d = u32::from(props[0]);
        let lc = d % 9;
        let lp = (d / 9) % 5;
        let pb = d / 45;
        if lc + lp > 4 {
            return Err(options());
        }
        let dict = u32::from_le_bytes([props[1], props[2], props[3], props[4]]);
        let dict_size = (u64::from(dict.max(4096)) + 15) & !15;
        Ok(Core {
            lc,
            lp_mask: (1 << lp) - 1,
            pb_mask: (1 << pb) - 1,
            dict_size,
            init: 0,
            st: State::new(lc, lp),
            win: Window {
                buf: Vec::new(),
                cap: usize::try_from(dict_size).unwrap_or(usize::MAX),
                pos: 0,
                total: 0,
            },
        })
    }

    /// Décode un symbole (sans toucher à la fenêtre).
    fn symbol(&mut self, input: &mut Input) -> Result<Symbol, Stop> {
        let win = &self.win;
        let st = &mut self.st;
        let mut rc = Rc {
            range: &mut st.range,
            code: &mut st.code,
            input,
        };
        let pos_state = (win.total & self.pb_mask) as usize;
        let s = st.state;
        if rc.bit(&mut st.is_match[(s << 4) + pos_state])? == 0 {
            let lit = (((win.total & self.lp_mask) << self.lc) as usize)
                + (usize::from(win.prev()) >> (8 - self.lc));
            let probs = &mut st.literal[lit * 0x300..lit * 0x300 + 0x300];
            let mut sym = 1usize;
            if s >= 7 {
                let mut match_byte = usize::from(win.get(st.reps[0] as usize + 1));
                while sym < 0x100 {
                    let match_bit = (match_byte >> 7) & 1;
                    match_byte <<= 1;
                    let b = rc.bit(&mut probs[((1 + match_bit) << 8) + sym])? as usize;
                    sym = (sym << 1) | b;
                    if match_bit != b {
                        break;
                    }
                }
            }
            while sym < 0x100 {
                sym = (sym << 1) | rc.bit(&mut probs[sym])? as usize;
            }
            st.state = if s < 4 {
                0
            } else if s < 10 {
                s - 3
            } else {
                s - 6
            };
            return Ok(Symbol::Literal((sym - 0x100) as u8));
        }
        let len;
        if rc.bit(&mut st.is_rep[s])? == 1 {
            // Répétition d'une distance récente : le dictionnaire ne doit pas être vide.
            if win.total == 0 {
                return Err(corrupt().into());
            }
            if rc.bit(&mut st.is_rep0[s])? == 0 {
                if rc.bit(&mut st.is_rep0_long[(s << 4) + pos_state])? == 0 {
                    st.state = if s < 7 { 9 } else { 11 };
                    return Ok(Symbol::Copy {
                        dist: st.reps[0] as usize + 1,
                        len: 1,
                    });
                }
            } else {
                let dist;
                if rc.bit(&mut st.is_rep1[s])? == 0 {
                    dist = st.reps[1];
                } else {
                    if rc.bit(&mut st.is_rep2[s])? == 0 {
                        dist = st.reps[2];
                    } else {
                        dist = st.reps[3];
                        st.reps[3] = st.reps[2];
                    }
                    st.reps[2] = st.reps[1];
                }
                st.reps[1] = st.reps[0];
                st.reps[0] = dist;
            }
            len = rc.len(&mut st.rep_len, pos_state)?;
            st.state = if s < 7 { 8 } else { 11 };
        } else {
            st.reps[3] = st.reps[2];
            st.reps[2] = st.reps[1];
            st.reps[1] = st.reps[0];
            len = rc.len(&mut st.len, pos_state)?;
            st.state = if s < 7 { 7 } else { 10 };
            let len_state = len.min(3) as usize;
            let slot = rc.tree(&mut st.dist_slot[len_state * 64..len_state * 64 + 64], 6)?;
            let dist = if slot < 4 {
                slot
            } else {
                let direct = (slot >> 1) - 1;
                let base = (2 | (slot & 1)) << direct;
                if slot < 14 {
                    base + rc.reverse(&mut st.dist_special, (base - slot) as usize, direct)?
                } else {
                    let mut v = 0u32;
                    for _ in 0..direct - 4 {
                        v = (v << 1) | rc.direct()?;
                    }
                    base + (v << 4) + rc.reverse(&mut st.align, 0, 4)?
                }
            };
            st.reps[0] = dist;
            if dist == u32::MAX {
                // Marqueur de fin : une normalisation, puis l'état doit être nul.
                rc.normalize()?;
                if *rc.code != 0 {
                    return Err(corrupt().into());
                }
                return Ok(Symbol::End);
            }
            if u64::from(dist) >= win.total.min(self.dict_size) {
                return Err(corrupt().into());
            }
        }
        Ok(Symbol::Copy {
            dist: st.reps[0] as usize + 1,
            len: len as usize + 2,
        })
    }
}

/// `zipfile.LZMADecompressor`.
pub struct LzmaDecoder {
    /// Octets reçus avant que l'en-tête ZIP soit complet (`_unconsumed`).
    header: Vec<u8>,
    core: Option<Box<Core>>,
    /// Octets compressés pas encore consommés (symbole incomplet).
    pending: Vec<u8>,
    eof: bool,
    out: OutBuf,
}

impl Default for LzmaDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl LzmaDecoder {
    pub fn new() -> Self {
        LzmaDecoder {
            header: Vec::new(),
            core: None,
            pending: Vec::new(),
            eof: false,
            out: OutBuf::new(),
        }
    }
}

impl StreamDecoder for LzmaDecoder {
    fn feed(&mut self, mut input: &[u8], _flush: bool, out: &mut Out) -> Result<bool, Halt> {
        if self.eof {
            return Ok(true);
        }
        // En-tête : au moins 5 octets, puis au moins 4 + taille des propriétés + 1 (le décodeur
        // brut n'est créé, et les propriétés contrôlées, qu'avec un octet du flux).
        while self.core.is_none() {
            let need = if self.header.len() < 5 {
                5
            } else {
                5 + usize::from(u16::from_le_bytes([self.header[2], self.header[3]]))
            };
            let take = need.saturating_sub(self.header.len()).min(input.len());
            self.header.extend_from_slice(&input[..take]);
            input = &input[take..];
            if self.header.len() < need {
                return Ok(false);
            }
            if need > 5 || u16::from_le_bytes([self.header[2], self.header[3]]) == 0 {
                let props = &self.header[4..need - 1];
                self.core = Some(Box::new(Core::new(props)?));
                self.pending.push(self.header[need - 1]);
                self.header = Vec::new();
            }
        }
        let owned;
        let src: &[u8] = if self.pending.is_empty() {
            input
        } else {
            self.pending.extend_from_slice(input);
            owned = std::mem::take(&mut self.pending);
            &owned
        };
        let core = self.core.as_mut().expect("décodeur créé");
        let mut inp = Input { data: src, pos: 0 };
        // Initialisation du décodeur d'intervalle : premier octet nul, puis 4 octets.
        while core.init < 5 {
            let Some(&b) = src.get(inp.pos) else {
                return Ok(false);
            };
            if core.init == 0 && b != 0 {
                return Err(corrupt());
            }
            core.st.code = (core.st.code << 8) | u32::from(b);
            core.init += 1;
            inp.pos += 1;
        }
        loop {
            let start = inp.pos;
            let near_end = src.len() - start < SAFE_INPUT;
            let snapshot = near_end.then(|| core.st.clone());
            match core.symbol(&mut inp) {
                Ok(Symbol::Literal(b)) => {
                    core.win.push(b);
                    self.out.push(b, out)?;
                }
                Ok(Symbol::Copy { dist, len }) => {
                    for _ in 0..len {
                        let b = core.win.get(dist);
                        core.win.push(b);
                        self.out.push(b, out)?;
                    }
                }
                Ok(Symbol::End) => {
                    self.eof = true;
                    self.out.flush(out)?;
                    return Ok(true);
                }
                Err(Stop::Need) => {
                    let Some(snap) = snapshot else {
                        return Err(data("erreur interne du décodeur LZMA (entrée)"));
                    };
                    core.st = snap;
                    self.pending = src[start..].to_vec();
                    self.out.flush(out)?;
                    return Ok(false);
                }
                Err(Stop::Halt(h)) => {
                    self.out.flush(out)?;
                    return Err(h);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `zipfile.LZMACompressor` de Python sur b"" et sur b"hello world " * 100 (voir
    /// `exp_lzma.py` dans la PR #458) : en-tête ZIP 09 04 05 00, propriétés 5d 00 00 80 00.
    const EMPTY: &[u8] =
        b"\x09\x04\x05\x00\x5d\x00\x00\x80\x00\x00\x83\xff\xfb\xff\xff\xc0\x00\x00\x00";

    fn decode(chunks: &[&[u8]]) -> (Result<bool, Halt>, Vec<u8>) {
        let mut d = LzmaDecoder::new();
        let mut got = Vec::new();
        let mut res = Ok(false);
        for c in chunks {
            res = d.feed(c, true, &mut |b: &[u8]| {
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
    fn flux_vide() {
        assert_eq!(decode(&[EMPTY]), (Ok(true), vec![]));
        let parts: Vec<&[u8]> = EMPTY.chunks(1).collect();
        assert_eq!(decode(&parts), (Ok(true), vec![]));
        // En-tête seul (même avec des propriétés invalides) : rien, sans erreur.
        assert_eq!(decode(&[&EMPTY[..9]]), (Ok(false), vec![]));
        let mut bad = EMPTY[..9].to_vec();
        bad[4] = 255;
        assert_eq!(decode(&[&bad]), (Ok(false), vec![]));
        // Avec un octet du flux, les propriétés sont contrôlées.
        bad.push(0);
        assert!(matches!(decode(&[&bad]).0, Err(Halt::Data(_))));
    }

    #[test]
    fn proprietes_refusees_comme_liblzma() {
        for (i, v) in [(4usize, (2 * 5 + 2) * 9 + 3), (4, 225), (2, 6)] {
            let mut b = EMPTY.to_vec();
            b[i] = v as u8;
            assert!(matches!(decode(&[&b]).0, Err(Halt::Data(_))), "{i} {v}");
        }
        // Premier octet du décodeur d'intervalle non nul.
        let mut b = EMPTY.to_vec();
        b[9] = 1;
        assert!(matches!(decode(&[&b]).0, Err(Halt::Data(_))));
        // État non nul après le marqueur de fin.
        let mut b = EMPTY.to_vec();
        let n = b.len();
        b[n - 1] ^= 0xFF;
        assert!(matches!(decode(&[&b]).0, Err(Halt::Data(_))));
    }
}
