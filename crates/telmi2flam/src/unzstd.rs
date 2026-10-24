//! Décodeur Zstandard (méthode ZIP 93) : reproduit `compression.zstd.ZstdDecompressor` de
//! Python 3.14 (libzstd 1.5.7, réglages par défaut) au-dessus du décodeur de blocs de
//! `ruzstd` (Rust pur, MIT). Le découpage en trames et en blocs, la somme de contrôle XXH64
//! et les contrôles de libzstd que `ruzstd` ne fait pas sont faits ici :
//!
//! - une seule trame : la suite est ignorée (`unused_data`) ; une trame « skippable » termine
//!   le flux sans rien rendre (comme Python) ;
//! - nombre magique contrôlé dès les premiers octets, bit réservé nul, fenêtre d'au plus
//!   `2^27 + 1` octets (`ZSTD_d_windowLogMax` par défaut), dictionnaire refusé ;
//! - taille annoncée (`Frame_Content_Size`) égale à la sortie, somme de contrôle vérifiée ;
//! - un bloc n'est passé à `ruzstd` qu'une fois entièrement reçu : un flux tronqué rend les
//!   blocs complets, sans erreur.
//!
//! Écart connu (voir PORT-RUST.md §3) : libzstd rend au fil de l'eau le début d'un bloc brut
//! (`Raw_Block`) tronqué, le Rust attend le bloc entier.

use std::hash::Hasher;

use ruzstd::decoding::{BlockDecodingStrategy, FrameDecoder};
use twox_hash::XxHash64;

use crate::zipcodec::{data, Halt, Out, StreamDecoder};

const MAGIC: u32 = 0xFD2F_B528;
const SKIPPABLE: u32 = 0x184D_2A50;
const SKIPPABLE_MASK: u32 = 0xFFFF_FFF0;
/// `ZSTD_MAXWINDOWSIZE_DEFAULT` : `(1 << ZSTD_WINDOWLOG_LIMIT_DEFAULT) + 1`.
const MAX_WINDOW: u64 = (1 << 27) + 1;

fn corruption() -> Halt {
    data("Unable to decompress Zstandard data: Data corruption detected")
}

enum Stage {
    Header,
    Skip(u64),
    Block,
    Checksum,
    Done,
}

/// `compression.zstd.ZstdDecompressor`.
pub struct ZstdDecoder {
    buf: Vec<u8>,
    start: usize,
    stage: Stage,
    frame: Option<Box<FrameDecoder>>,
    checksum: bool,
    content_size: Option<u64>,
    hasher: XxHash64,
    total: u64,
}

impl Default for ZstdDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ZstdDecoder {
    pub fn new() -> Self {
        ZstdDecoder {
            buf: Vec::new(),
            start: 0,
            stage: Stage::Header,
            frame: None,
            checksum: false,
            content_size: None,
            hasher: XxHash64::with_seed(0),
            total: 0,
        }
    }

    /// En-tête de trame ; `Ok(false)` s'il manque des octets.
    fn header(&mut self) -> Result<bool, Halt> {
        let avail = &self.buf[self.start..];
        // Les premiers octets doivent commencer un nombre magique connu (ZSTD_getFrameHeader).
        let n = avail.len().min(4);
        let mut m = MAGIC.to_le_bytes();
        m[..n].copy_from_slice(&avail[..n]);
        let mut s = SKIPPABLE.to_le_bytes();
        s[..n].copy_from_slice(&avail[..n]);
        if u32::from_le_bytes(m) != MAGIC && u32::from_le_bytes(s) & SKIPPABLE_MASK != SKIPPABLE {
            return Err(data(
                "Unable to decompress Zstandard data: Unknown frame descriptor",
            ));
        }
        if avail.len() < 4 {
            return Ok(false);
        }
        if u32::from_le_bytes(m) != MAGIC {
            if avail.len() < 8 {
                return Ok(false);
            }
            let size = u32::from_le_bytes([avail[4], avail[5], avail[6], avail[7]]);
            self.start += 8;
            self.stage = Stage::Skip(u64::from(size));
            return Ok(true);
        }
        let Some(&fhd) = avail.get(4) else {
            return Ok(false);
        };
        let single = fhd & 0x20 != 0;
        let fcs_len = match fhd >> 6 {
            0 => usize::from(single),
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let dict_len = [0usize, 1, 2, 4][usize::from(fhd & 3)];
        let size = 5 + usize::from(!single) + dict_len + fcs_len;
        if avail.len() < size {
            return Ok(false);
        }
        if fhd & 0x08 != 0 {
            return Err(data(
                "Unable to decompress Zstandard data: Unsupported frame parameter",
            ));
        }
        self.checksum = fhd & 0x04 != 0;
        self.content_size = (fcs_len > 0).then(|| {
            let raw = &avail[size - fcs_len..size];
            let mut v = 0u64;
            for (i, &b) in raw.iter().enumerate() {
                v |= u64::from(b) << (8 * i);
            }
            if fcs_len == 2 {
                v + 256
            } else {
                v
            }
        });
        // La somme de contrôle est lue et vérifiée ici : `ruzstd` ne la voit pas.
        let mut header = avail[..size].to_vec();
        header[4] &= !0x04;
        let mut frame = Box::new(FrameDecoder::new());
        frame.set_max_window_size(MAX_WINDOW);
        frame
            .reset(&header[..])
            .map_err(|e| data(format!("Unable to decompress Zstandard data: {e}")))?;
        self.frame = Some(frame);
        self.start += size;
        self.stage = Stage::Block;
        Ok(true)
    }

    fn emit(&mut self, bytes: &[u8], out: &mut Out) -> Result<(), Halt> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.hasher.write(bytes);
        self.total += bytes.len() as u64;
        out(bytes).map_err(|()| Halt::Sink)
    }

    /// Un bloc ; `Ok(false)` s'il manque des octets.
    fn block(&mut self, out: &mut Out) -> Result<bool, Halt> {
        let avail = &self.buf[self.start..];
        if avail.len() < 3 {
            return Ok(false);
        }
        let h = u32::from(avail[0]) | u32::from(avail[1]) << 8 | u32::from(avail[2]) << 16;
        let last = h & 1 != 0;
        let kind = (h >> 1) & 3;
        if kind == 3 {
            return Err(corruption());
        }
        let need = 3 + if kind == 1 { 1 } else { (h >> 3) as usize };
        if avail.len() < need {
            return Ok(false);
        }
        let frame = self.frame.as_mut().expect("trame ouverte");
        let mut src = &avail[..need];
        frame
            .decode_blocks(&mut src, BlockDecodingStrategy::UptoBlocks(1))
            .map_err(|e| data(format!("Unable to decompress Zstandard data: {e}")))?;
        if !src.is_empty() {
            return Err(data("erreur interne du décodeur Zstandard (bloc)"));
        }
        self.start += need;
        let collected = frame.collect().unwrap_or_default();
        self.emit(&collected, out)?;
        if last {
            if !self.frame.as_ref().is_some_and(|f| f.is_finished()) {
                return Err(data("erreur interne du décodeur Zstandard (fin de trame)"));
            }
            let rest = self
                .frame
                .as_mut()
                .and_then(|f| f.collect())
                .unwrap_or_default();
            self.emit(&rest, out)?;
            if self.content_size.is_some_and(|n| n != self.total) {
                return Err(corruption());
            }
            self.stage = if self.checksum {
                Stage::Checksum
            } else {
                Stage::Done
            };
        }
        Ok(true)
    }
}

impl StreamDecoder for ZstdDecoder {
    /// Trame tronquée : libzstd a déjà rendu chaque bloc décodé (le Rust en retient une
    /// fenêtre dans `ruzstd`) et le début d'un bloc brut incomplet. La trame est close par un
    /// bloc brut vide marqué dernier pour vider `ruzstd`, puis le début du bloc brut est rendu.
    fn finish(&mut self, out: &mut Out) -> Result<(), Halt> {
        if !matches!(self.stage, Stage::Block) {
            return Ok(());
        }
        self.stage = Stage::Done;
        if let Some(frame) = self.frame.as_mut() {
            let mut src: &[u8] = &[0x01, 0, 0];
            frame
                .decode_blocks(&mut src, BlockDecodingStrategy::UptoBlocks(1))
                .map_err(|e| data(format!("Unable to decompress Zstandard data: {e}")))?;
            let rest = frame.collect().unwrap_or_default();
            self.emit(&rest, out)?;
        }
        let avail = &self.buf[self.start..];
        if avail.len() > 3 {
            let h = u32::from(avail[0]) | u32::from(avail[1]) << 8 | u32::from(avail[2]) << 16;
            if (h >> 1) & 3 == 0 {
                let k = (avail.len() - 3).min((h >> 3) as usize);
                let partial = avail[3..3 + k].to_vec();
                self.emit(&partial, out)?;
            }
        }
        Ok(())
    }

    fn feed(&mut self, input: &[u8], _flush: bool, out: &mut Out) -> Result<bool, Halt> {
        if matches!(self.stage, Stage::Done) {
            return Ok(true);
        }
        if self.start > 0 && self.start >= self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(input);
        loop {
            let progressed = match self.stage {
                Stage::Header => self.header()?,
                Stage::Skip(n) => {
                    let k = n.min((self.buf.len() - self.start) as u64);
                    self.start += k as usize;
                    if k == n {
                        self.stage = Stage::Done;
                    } else {
                        self.stage = Stage::Skip(n - k);
                    }
                    k == n
                }
                Stage::Block => self.block(out)?,
                Stage::Checksum => {
                    let avail = &self.buf[self.start..];
                    if avail.len() < 4 {
                        false
                    } else {
                        let stored = u32::from_le_bytes([avail[0], avail[1], avail[2], avail[3]]);
                        self.start += 4;
                        if self.hasher.finish() as u32 != stored {
                            return Err(data(
                                "Unable to decompress Zstandard data: Restored data doesn't match checksum",
                            ));
                        }
                        self.stage = Stage::Done;
                        true
                    }
                }
                Stage::Done => {
                    self.buf = Vec::new();
                    self.start = 0;
                    return Ok(true);
                }
            };
            if !progressed {
                return Ok(false);
            }
        }
    }
}
