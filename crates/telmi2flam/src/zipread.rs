//! Lecteur ZIP qui reproduit `zipfile` de Python 3.14.8 (`ZipFile(path, "r")`, `namelist()`,
//! `getinfo()`, `read()`), y compris ses refus et ses tolérances. Les archives Telmi ne sont pas
//! fiables : aucune entrée n'est extraite sur disque, rien n'est indexé au-delà de ce que
//! `zipfile` lit, et aucune borne n'est ajoutée à celles de Python (décision D5).
//!
//! Pourquoi pas la crate `zip` : elle décode les noms UTF-8 invalides avec pertes (Python
//! échoue), cherche la fin du répertoire central autrement, ajoute ses propres contrôles
//! (AES, champs supplémentaires) et ne garde qu'une entrée par nom. Ici, chaque étape suit le
//! code de `Lib/zipfile/__init__.py` de Python 3.14.8 (version de l'oracle, épinglée dans
//! `ORACLE-VERSIONS.txt`), cité en commentaire.

use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use flate2::{Decompress, FlushDecompress, Status};

use crate::unbzip2::Bzip2Decoder;
use crate::unlzma::LzmaDecoder;
use crate::unzstd::ZstdDecoder;
use crate::zipcodec::{Halt, StreamDecoder};

const SIZE_END_CENT_DIR: u64 = 22;
const SIZE_END_CENT_DIR64_LOCATOR: i128 = 20;
const SIZE_END_CENT_DIR64: i128 = 56;
const SIZE_CENTRAL_DIR: usize = 46;
const SIZE_FILE_HEADER: usize = 30;
const ZIP_MAX_COMMENT: u64 = (1 << 16) - 1;
const MAX_EXTRACT_VERSION: u8 = 63;

const MASK_ENCRYPTED: u16 = 1 << 0;
const MASK_COMPRESSED_PATCH: u16 = 1 << 5;
const MASK_STRONG_ENCRYPTION: u16 = 1 << 6;
const MASK_UTF_FILENAME: u16 = 1 << 11;

const ZIP_STORED: u16 = 0;
const ZIP_DEFLATED: u16 = 8;
/// bzip2 (`bz2`), LZMA (`lzma`) et Zstandard (`compression.zstd`) : lus par Python 3.14 et
/// par le Rust (#458, décodeurs de `zipcodec`).
const ZIP_BZIP2: u16 = 12;
const ZIP_LZMA: u16 = 14;
const ZIP_ZSTANDARD: u16 = 93;

/// `ZipExtFile.MAX_N` (`(1 << 31) - 1` depuis Python 3.14 ; `1 << 30` en 3.12) : taille d'un tour.
const MAX_N: usize = (1 << 31) - 1;
/// `ZipExtFile.MIN_READ_SIZE`.
const MIN_READ_SIZE: usize = 4096;
/// bzip2, LZMA, Zstandard : un tour de `MAX_N` octets compressés est lu et décodé par
/// morceaux de cette taille (Python le lit d'un bloc ; le résultat est le même).
const CODEC_PIECE: usize = 1 << 20;

/// Échec de lecture, classé selon ce que l'ancien convertisseur Python en faisait.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ZipError {
    /// `BadZipFile`, `NotImplementedError`, `EOFError`, `zlib.error` : code 3 en Python.
    Corrupt(String),
    /// Bit 0 (chiffrement) : `RuntimeError` en Python (code 6), code 3 en Rust (§3).
    Encrypted(String),
    /// Nom non décodable (`UnicodeDecodeError`), position négative (`OSError`), flux bzip2,
    /// LZMA ou Zstandard invalide (`OSError`, `LZMAError`, `ZstdError`) : Python échoue en
    /// `internal_error` (code 6) ; le Rust rend code 3, comme pour le chiffrement.
    PythonFailsToo(String),
}

impl std::fmt::Display for ZipError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ZipError::Corrupt(m) | ZipError::Encrypted(m) | ZipError::PythonFailsToo(m) => {
                f.write_str(m)
            }
        }
    }
}

fn corrupt(message: impl Into<String>) -> ZipError {
    ZipError::Corrupt(message.into())
}

fn io_corrupt(e: io::Error) -> ZipError {
    ZipError::Corrupt(e.to_string())
}

/// Une entrée du répertoire central (`ZipInfo`).
#[derive(Debug, Clone)]
struct Entry {
    /// `ZipInfo.filename` : nom assaini (NUL, `\` sous Windows), éventuellement remplacé par le
    /// champ 0x7075.
    filename: String,
    /// `ZipInfo.orig_filename` : nom décodé du répertoire central, sans assainissement.
    orig_filename: String,
    flag_bits: u16,
    compress_type: u16,
    crc: u32,
    compress_size: u64,
    file_size: u64,
    /// Entier signé non borné en Python : `concat` peut être négatif.
    header_offset: i128,
    /// `ZipInfo._end_offset` : début de l'entrée suivante (garde « Overlapped entries »).
    end_offset: i128,
}

/// Archive ouverte en lecture.
pub struct ZipReader<R> {
    reader: R,
    entries: Vec<Entry>,
    /// `NameToInfo` : la dernière entrée d'un nom gagne.
    name_to_info: HashMap<String, usize>,
}

/// Lit au plus `n` octets (comme `file.read(n)` : moins à la fin du fichier, jamais d'erreur
/// pour un fichier trop court). N'alloue que ce qui est réellement lu.
fn read_up_to<R: Read>(reader: &mut R, n: u64) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    reader.take(n).read_to_end(&mut out)?;
    Ok(out)
}

/// Positionne le lecteur comme `file.seek(pos)` de Python : position négative refusée
/// (`OSError`, code 6 en Python), position au-delà de la fin acceptée (les lectures rendent 0).
fn seek_to<R: Seek>(reader: &mut R, pos: i128) -> Result<(), ZipError> {
    if pos < 0 {
        return Err(ZipError::PythonFailsToo(format!(
            "position négative dans l'archive : {pos}"
        )));
    }
    let pos = u64::try_from(pos).unwrap_or(u64::MAX);
    reader.seek(SeekFrom::Start(pos)).map_err(io_corrupt)?;
    Ok(())
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

fn u64_at(b: &[u8], i: usize) -> u64 {
    let mut a = [0u8; 8];
    a.copy_from_slice(&b[i..i + 8]);
    u64::from_le_bytes(a)
}

/// Table cp437 de Python pour 0x80..=0xFF (0x00..=0x7F : ASCII).
const CP437_HIGH: [char; 128] = [
    '\u{00c7}', '\u{00fc}', '\u{00e9}', '\u{00e2}', '\u{00e4}', '\u{00e0}', '\u{00e5}', '\u{00e7}',
    '\u{00ea}', '\u{00eb}', '\u{00e8}', '\u{00ef}', '\u{00ee}', '\u{00ec}', '\u{00c4}', '\u{00c5}',
    '\u{00c9}', '\u{00e6}', '\u{00c6}', '\u{00f4}', '\u{00f6}', '\u{00f2}', '\u{00fb}', '\u{00f9}',
    '\u{00ff}', '\u{00d6}', '\u{00dc}', '\u{00a2}', '\u{00a3}', '\u{00a5}', '\u{20a7}', '\u{0192}',
    '\u{00e1}', '\u{00ed}', '\u{00f3}', '\u{00fa}', '\u{00f1}', '\u{00d1}', '\u{00aa}', '\u{00ba}',
    '\u{00bf}', '\u{2310}', '\u{00ac}', '\u{00bd}', '\u{00bc}', '\u{00a1}', '\u{00ab}', '\u{00bb}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{2561}', '\u{2562}', '\u{2556}',
    '\u{2555}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255d}', '\u{255c}', '\u{255b}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252c}', '\u{251c}', '\u{2500}', '\u{253c}', '\u{255e}', '\u{255f}',
    '\u{255a}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256c}', '\u{2567}',
    '\u{2568}', '\u{2564}', '\u{2565}', '\u{2559}', '\u{2558}', '\u{2552}', '\u{2553}', '\u{256b}',
    '\u{256a}', '\u{2518}', '\u{250c}', '\u{2588}', '\u{2584}', '\u{258c}', '\u{2590}', '\u{2580}',
    '\u{03b1}', '\u{00df}', '\u{0393}', '\u{03c0}', '\u{03a3}', '\u{03c3}', '\u{00b5}', '\u{03c4}',
    '\u{03a6}', '\u{0398}', '\u{03a9}', '\u{03b4}', '\u{221e}', '\u{03c6}', '\u{03b5}', '\u{2229}',
    '\u{2261}', '\u{00b1}', '\u{2265}', '\u{2264}', '\u{2320}', '\u{2321}', '\u{00f7}', '\u{2248}',
    '\u{00b0}', '\u{2219}', '\u{00b7}', '\u{221a}', '\u{207f}', '\u{00b2}', '\u{25a0}', '\u{00a0}',
];

/// `bytes.decode("cp437")` : ne peut pas échouer.
pub fn decode_cp437(raw: &[u8]) -> String {
    raw.iter()
        .map(|&b| {
            if b < 0x80 {
                char::from(b)
            } else {
                CP437_HIGH[usize::from(b - 0x80)]
            }
        })
        .collect()
}

/// Nom d'entrée selon le drapeau UTF-8 (bit 11) : UTF-8 strict, sinon cp437.
fn decode_name(raw: &[u8], flags: u16) -> Result<String, ZipError> {
    if flags & MASK_UTF_FILENAME != 0 {
        String::from_utf8(raw.to_vec())
            .map_err(|e| ZipError::PythonFailsToo(format!("nom d'entrée UTF-8 invalide : {e}")))
    } else {
        Ok(decode_cp437(raw))
    }
}

/// `_sanitize_filename` : coupe au premier NUL ; sous Windows (`os.sep == "\\"`), `\` devient
/// `/`. Le comportement suit donc le système, comme Python.
fn sanitize_filename(name: &str) -> String {
    sanitize_filename_for(name, cfg!(windows))
}

/// `_sanitize_filename` pour un système donné (`\` séparateur si `backslash_is_sep`), afin
/// de tester les deux branches quel que soit le système d'exécution.
fn sanitize_filename_for(name: &str, backslash_is_sep: bool) -> String {
    let cut = match name.find('\0') {
        Some(i) => &name[..i],
        None => name,
    };
    if backslash_is_sep {
        cut.replace('\\', "/")
    } else {
        cut.to_string()
    }
}

/// Fin du répertoire central, après lecture éventuelle de l'enregistrement ZIP64.
struct EndRec {
    size_cd: u64,
    offset_cd: u64,
    /// `_ECD_LOCATION`.
    location: i128,
}

/// `_EndRecData64`.
fn end_rec_data64<R: Read + Seek>(
    r: &mut R,
    offset: i128,
    endrec: EndRec,
) -> Result<EndRec, ZipError> {
    let mut offset = offset - SIZE_END_CENT_DIR64_LOCATOR;
    if offset < 0 {
        return Ok(endrec);
    }
    seek_to(r, offset)?;
    let data = read_up_to(r, 20).map_err(io_corrupt)?;
    if data.len() != 20 {
        return Err(corrupt("File is not a zip file"));
    }
    if &data[0..4] != b"PK\x06\x07" {
        return Ok(endrec);
    }
    let diskno = u32_at(&data, 4);
    let reloff = i128::from(u64_at(&data, 8));
    let disks = u32_at(&data, 16);
    if diskno != 0 || disks > 1 {
        return Err(corrupt(
            "zipfiles that span multiple disks are not supported",
        ));
    }
    offset -= SIZE_END_CENT_DIR64;
    if reloff > offset {
        return Err(corrupt("Corrupt zip64 end of central directory locator"));
    }
    seek_to(r, reloff)?;
    let mut extrasz = offset - reloff;
    let mut data = read_up_to(r, 56).map_err(io_corrupt)?;
    if data.len() != 56 {
        return Err(corrupt("File is not a zip file"));
    }
    if !data.starts_with(b"PK\x06\x06") && reloff != offset {
        seek_to(r, offset)?;
        extrasz = 0;
        data = read_up_to(r, 56).map_err(io_corrupt)?;
        if data.len() != 56 {
            return Err(corrupt("File is not a zip file"));
        }
    }
    if !data.starts_with(b"PK\x06\x06") {
        return Err(corrupt("Zip64 end of central directory record not found"));
    }
    let sz = i128::from(u64_at(&data, 4));
    let dirsize = u64_at(&data, 40);
    let diroffset = u64_at(&data, 48);
    if i128::from(diroffset) + i128::from(dirsize) != reloff
        || sz + 12 != SIZE_END_CENT_DIR64 + extrasz
    {
        return Err(corrupt("Corrupt zip64 end of central directory record"));
    }
    Ok(EndRec {
        size_cd: dirsize,
        offset_cd: diroffset,
        location: offset - extrasz,
    })
}

fn parse_endrec(rec: &[u8], location: i128) -> EndRec {
    EndRec {
        size_cd: u64::from(u32_at(rec, 12)),
        offset_cd: u64::from(u32_at(rec, 16)),
        location,
    }
}

/// `_EndRecData` : `None` si aucune fin de répertoire central n'est trouvée.
fn end_rec_data<R: Read + Seek>(r: &mut R) -> Result<Option<EndRec>, ZipError> {
    let filesize = r.seek(SeekFrom::End(0)).map_err(io_corrupt)?;
    // fpin.seek(-sizeEndCentDir, 2) lève OSError sur un fichier plus court : None.
    if filesize < SIZE_END_CENT_DIR {
        return Ok(None);
    }
    r.seek(SeekFrom::Start(filesize - SIZE_END_CENT_DIR))
        .map_err(io_corrupt)?;
    let data = read_up_to(r, SIZE_END_CENT_DIR).map_err(io_corrupt)?;
    if data.len() == 22 && &data[0..4] == b"PK\x05\x06" && data[20..22] == [0, 0] {
        let location = i128::from(filesize - SIZE_END_CENT_DIR);
        let endrec = parse_endrec(&data, location);
        return end_rec_data64(r, location, endrec).map(Some);
    }
    let max_comment_start = filesize.saturating_sub(ZIP_MAX_COMMENT + SIZE_END_CENT_DIR);
    r.seek(SeekFrom::Start(max_comment_start))
        .map_err(io_corrupt)?;
    let data = read_up_to(r, ZIP_MAX_COMMENT + SIZE_END_CENT_DIR).map_err(io_corrupt)?;
    // data.rfind(stringEndArchive)
    let Some(start) = data.windows(4).rposition(|w| w == b"PK\x05\x06") else {
        return Ok(None);
    };
    let rec = &data[start..data.len().min(start + 22)];
    if rec.len() != 22 {
        return Ok(None);
    }
    let location = i128::from(max_comment_start) + start as i128;
    let endrec = parse_endrec(rec, location);
    end_rec_data64(r, location, endrec).map(Some)
}

/// Curseur sur le répertoire central lu en mémoire (`io.BytesIO(data)` : lectures courtes
/// silencieuses).
struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn read(&mut self, n: usize) -> &'a [u8] {
        let end = self.pos.saturating_add(n).min(self.data.len());
        let out = &self.data[self.pos..end];
        self.pos = end;
        out
    }
}

/// `ZipInfo._decodeExtra` : ZIP64 (0x0001) et chemin Unicode d'Info-ZIP (0x7075).
fn decode_extra(entry: &mut Entry, extra: &[u8], filename_crc: u32) -> Result<(), ZipError> {
    let mut extra = extra;
    while extra.len() >= 4 {
        let tp = u16_at(extra, 0);
        let ln = usize::from(u16_at(extra, 2));
        if ln + 4 > extra.len() {
            return Err(corrupt(format!("Corrupt extra field {tp:04x} (size={ln})")));
        }
        let data = &extra[4..ln + 4];
        if tp == 0x0001 {
            let mut data = data;
            let mut take = |field: &str| -> Result<u64, ZipError> {
                if data.len() < 8 {
                    return Err(corrupt(format!(
                        "Corrupt zip64 extra field. {field} not found."
                    )));
                }
                let v = u64_at(data, 0);
                data = &data[8..];
                Ok(v)
            };
            if entry.file_size == 0xFFFF_FFFF_FFFF_FFFF || entry.file_size == 0xFFFF_FFFF {
                entry.file_size = take("File size")?;
            }
            if entry.compress_size == 0xFFFF_FFFF {
                entry.compress_size = take("Compress size")?;
            }
            if entry.header_offset == 0xFFFF_FFFF {
                entry.header_offset = i128::from(take("Header offset")?);
            }
        } else if tp == 0x7075 {
            if data.len() < 5 {
                return Err(corrupt("Corrupt unicode path extra field (0x7075)"));
            }
            let up_version = data[0];
            let up_name_crc = u32_at(data, 1);
            if up_version == 1 && up_name_crc == filename_crc {
                let name = std::str::from_utf8(&data[5..]).map_err(|_| {
                    corrupt("Corrupt unicode path extra field (0x7075): invalid utf-8 bytes")
                })?;
                // Vide : simple avertissement en Python, le nom d'origine reste.
                if !name.is_empty() {
                    entry.filename = sanitize_filename(name);
                }
            }
        }
        extra = &extra[ln + 4..];
    }
    Ok(())
}

impl<R: Read + Seek> ZipReader<R> {
    /// `ZipFile(file, "r")` puis `_RealGetContents`.
    pub fn new(mut reader: R) -> Result<Self, ZipError> {
        let endrec = end_rec_data(&mut reader)?.ok_or_else(|| corrupt("File is not a zip file"))?;
        let size_cd = endrec.size_cd;
        let offset_cd = endrec.offset_cd;
        // "concat" est nul, sauf si l'archive suit d'autres données (exécutable autoextractible).
        let concat = endrec.location - i128::from(size_cd) - i128::from(offset_cd);
        let start_dir = i128::from(offset_cd) + concat;
        if start_dir < 0 {
            return Err(corrupt("Bad offset for central directory"));
        }
        seek_to(&mut reader, start_dir)?;
        // fp.read(size_cd) : n'alloue que ce que contient le fichier.
        let data = read_up_to(&mut reader, size_cd).map_err(io_corrupt)?;
        let mut fp = Cursor {
            data: &data,
            pos: 0,
        };
        let mut entries: Vec<Entry> = Vec::new();
        let mut name_to_info = HashMap::new();
        let mut total: u128 = 0;
        while total < u128::from(size_cd) {
            let centdir = fp.read(SIZE_CENTRAL_DIR);
            if centdir.len() != SIZE_CENTRAL_DIR {
                return Err(corrupt("Truncated central directory"));
            }
            if &centdir[0..4] != b"PK\x01\x02" {
                return Err(corrupt("Bad magic number for central directory"));
            }
            let extract_version = centdir[6];
            let flag_bits = u16_at(centdir, 8);
            let compress_type = u16_at(centdir, 10);
            let crc = u32_at(centdir, 16);
            let compress_size = u64::from(u32_at(centdir, 20));
            let file_size = u64::from(u32_at(centdir, 24));
            let name_len = u16_at(centdir, 28);
            let extra_len = u16_at(centdir, 30);
            let comment_len = u16_at(centdir, 32);
            let header_offset = i128::from(u32_at(centdir, 42));

            let raw_name = fp.read(usize::from(name_len));
            let filename_crc = crc32fast::hash(raw_name);
            let orig_filename = decode_name(raw_name, flag_bits)?;
            let extra = fp.read(usize::from(extra_len));
            let _comment = fp.read(usize::from(comment_len));
            if extract_version > MAX_EXTRACT_VERSION {
                return Err(corrupt(format!(
                    "zip file version {:.1}",
                    f64::from(extract_version) / 10.0
                )));
            }
            let mut entry = Entry {
                filename: sanitize_filename(&orig_filename),
                orig_filename,
                flag_bits,
                compress_type,
                crc,
                compress_size,
                file_size,
                header_offset,
                end_offset: 0,
            };
            decode_extra(&mut entry, extra, filename_crc)?;
            entry.header_offset += concat;
            name_to_info.insert(entry.filename.clone(), entries.len());
            entries.push(entry);
            total += SIZE_CENTRAL_DIR as u128
                + u128::from(name_len)
                + u128::from(extra_len)
                + u128::from(comment_len);
        }
        // for zinfo in reversed(sorted(filelist, key=header_offset)) : tri stable, puis inverse.
        let mut order: Vec<usize> = (0..entries.len()).collect();
        order.sort_by_key(|&i| entries[i].header_offset);
        let mut end_offset = start_dir;
        for &i in order.iter().rev() {
            entries[i].end_offset = end_offset;
            end_offset = entries[i].header_offset;
        }
        Ok(ZipReader {
            reader,
            entries,
            name_to_info,
        })
    }

    /// `namelist()` : tous les noms, doublons compris, dans l'ordre du répertoire central.
    pub fn namelist(&self) -> Vec<String> {
        self.entries.iter().map(|e| e.filename.clone()).collect()
    }

    /// Le nom existe-t-il (`name in NameToInfo`) ?
    pub fn contains(&self, name: &str) -> bool {
        self.name_to_info.contains_key(name)
    }

    /// `read(name)` : contenu complet de l'entrée retenue par `getinfo` (la dernière du nom),
    /// CRC vérifié. `Ok(None)` si le nom n'existe pas (`KeyError`).
    pub fn read(&mut self, name: &str) -> Result<Option<Vec<u8>>, ZipError> {
        let Some(&index) = self.name_to_info.get(name) else {
            return Ok(None);
        };
        let entry = self.entries[index].clone();
        self.read_entry(&entry).map(Some)
    }

    /// [`ZipReader::read`] par tours d'au plus `chunk` octets décompressés, passés à `sink` un
    /// à un (la mémoire reste bornée par `chunk`) ; le CRC est vérifié après le dernier tour,
    /// donc **après** que `sink` a reçu toutes les données. `Ok(false)` si le nom n'existe pas.
    /// Mêmes contrôles d'en-tête que `read` ; seule la taille des tours diffère.
    pub fn read_chunks<E>(
        &mut self,
        name: &str,
        chunk: usize,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<bool, ChunkStop<E>> {
        let Some(&index) = self.name_to_info.get(name) else {
            return Ok(false);
        };
        let entry = self.entries[index].clone();
        self.open_entry(&entry)?.pump(&entry, chunk.max(1), sink)?;
        Ok(true)
    }

    /// `ZipFile.open(name)` puis `ZipExtFile.read()`.
    fn read_entry(&mut self, zinfo: &Entry) -> Result<Vec<u8>, ZipError> {
        self.open_entry(zinfo)?.read_all(zinfo, MAX_N)
    }

    /// `ZipFile.open(name)` : contrôles de l'en-tête local, lecteur placé sur les données.
    fn open_entry(&mut self, zinfo: &Entry) -> Result<ExtFile<'_, R>, ZipError> {
        let r = &mut self.reader;
        seek_to(r, zinfo.header_offset)?;
        let fheader = read_up_to(r, SIZE_FILE_HEADER as u64).map_err(io_corrupt)?;
        if fheader.len() != SIZE_FILE_HEADER {
            return Err(corrupt("Truncated file header"));
        }
        if &fheader[0..4] != b"PK\x03\x04" {
            return Err(corrupt("Bad magic number for file header"));
        }
        let local_flags = u16_at(&fheader, 6);
        let fname_len = u16_at(&fheader, 26);
        let extra_len = u16_at(&fheader, 28);
        let fname = read_up_to(r, u64::from(fname_len)).map_err(io_corrupt)?;
        // zef_file.tell() après seek(extra, whence=1) : position atteinte, même hors fichier.
        let data_start = zinfo.header_offset
            + SIZE_FILE_HEADER as i128
            + fname.len() as i128
            + i128::from(extra_len);

        if zinfo.flag_bits & MASK_COMPRESSED_PATCH != 0 {
            return Err(corrupt("compressed patched data (flag bit 5)"));
        }
        if zinfo.flag_bits & MASK_STRONG_ENCRYPTION != 0 {
            return Err(corrupt("strong encryption (flag bit 6)"));
        }
        let fname_str = decode_name(&fname, local_flags)?;
        if fname_str != zinfo.orig_filename {
            return Err(corrupt(format!(
                "File name in directory {:?} and header {:?} differ.",
                zinfo.orig_filename, fname_str
            )));
        }
        // Entrées qui se chevauchent : erreur, sauf deux entrées au même en-tête (avertissement).
        if data_start + i128::from(zinfo.compress_size) > zinfo.end_offset
            && zinfo.end_offset != zinfo.header_offset
        {
            return Err(corrupt(format!(
                "Overlapped entries: {:?} (possible zip bomb)",
                zinfo.orig_filename
            )));
        }
        if zinfo.flag_bits & MASK_ENCRYPTED != 0 {
            return Err(ZipError::Encrypted(format!(
                "File {:?} is encrypted, password required for extraction",
                zinfo.filename
            )));
        }
        // _get_decompressor
        let codec: Option<Box<dyn StreamDecoder>> = match zinfo.compress_type {
            ZIP_STORED | ZIP_DEFLATED => None,
            ZIP_BZIP2 => Some(Box::new(Bzip2Decoder::new())),
            ZIP_LZMA => Some(Box::new(LzmaDecoder::new())),
            ZIP_ZSTANDARD => Some(Box::new(ZstdDecoder::new())),
            _ => return Err(corrupt("That compression method is not supported")),
        };
        seek_to(r, data_start)?;
        Ok(ExtFile {
            reader: r,
            deflate: zinfo.compress_type == ZIP_DEFLATED,
            compress_left: zinfo.compress_size,
            left: zinfo.file_size,
            decompressor: Decompress::new(false),
            tail: Vec::new(),
            codec,
        })
    }
}

/// Arrêt de [`ZipReader::read_chunks`] : entrée illisible, ou refus du destinataire.
#[derive(Debug)]
pub enum ChunkStop<E> {
    Zip(ZipError),
    Sink(E),
}

impl<E> From<ZipError> for ChunkStop<E> {
    fn from(e: ZipError) -> Self {
        ChunkStop::Zip(e)
    }
}

/// `ZipExtFile` réduit à `read()` (lecture jusqu'au bout, puis contrôle du CRC).
struct ExtFile<'a, R> {
    reader: &'a mut R,
    deflate: bool,
    compress_left: u64,
    left: u64,
    decompressor: Decompress,
    /// `unconsumed_tail` du décompresseur.
    tail: Vec<u8>,
    /// bzip2, LZMA, Zstandard : décompresseur incrémental (`_get_decompressor`).
    codec: Option<Box<dyn StreamDecoder>>,
}

impl<R: Read> ExtFile<'_, R> {
    /// `_read2` : au plus `n` octets compressés (au moins `MIN_READ_SIZE`, au plus ce qui
    /// reste) ; `EOFError` si le fichier s'arrête avant.
    fn read2(&mut self, n: usize) -> Result<Vec<u8>, ZipError> {
        if self.compress_left == 0 {
            return Ok(Vec::new());
        }
        let n = (n.max(MIN_READ_SIZE) as u64).min(self.compress_left);
        let data = read_up_to(self.reader, n).map_err(io_corrupt)?;
        self.compress_left -= data.len() as u64;
        if data.is_empty() {
            return Err(corrupt("EOFError : fin de fichier dans une entrée"));
        }
        Ok(data)
    }

    /// `decompressobj.decompress(data, max_length)` : décompresse jusqu'à la fin du flux, la
    /// fin de `data` ou `max_length` octets produits. Ne garde que les `keep` premiers octets
    /// (les suivants seraient tronqués par `data[:self._left]`), mais compte tout pour
    /// détecter les erreurs au même endroit que zlib. Rend (sortie, consommé, fin du flux,
    /// produit).
    fn inflate(
        &mut self,
        input: &[u8],
        max_length: usize,
        keep: usize,
    ) -> Result<(Vec<u8>, usize, bool, usize), ZipError> {
        let mut out = Vec::new();
        let mut buf = vec![0u8; 64 * 1024];
        let mut pos = 0usize;
        let mut produced = 0usize;
        while produced < max_length {
            let room = buf.len().min(max_length - produced);
            let before_in = self.decompressor.total_in();
            let before_out = self.decompressor.total_out();
            let status = self
                .decompressor
                .decompress(&input[pos..], &mut buf[..room], FlushDecompress::Sync)
                .map_err(|e| corrupt(format!("Error while decompressing data: {e}")))?;
            let used = (self.decompressor.total_in() - before_in) as usize;
            let made = (self.decompressor.total_out() - before_out) as usize;
            pos += used;
            if out.len() < keep {
                let k = made.min(keep - out.len());
                out.extend_from_slice(&buf[..k]);
            }
            produced += made;
            if status == Status::StreamEnd {
                return Ok((out, pos, true, produced));
            }
            if used == 0 && made == 0 {
                break;
            }
        }
        Ok((out, pos, false, produced))
    }

    /// `ZipExtFile.read()` ; `max_n` est `MAX_N` (paramétré pour tester les bornes de tour
    /// sans produire 2 Gio).
    fn read_all(self, zinfo: &Entry, max_n: usize) -> Result<Vec<u8>, ZipError> {
        let mut out = Vec::new();
        self.pump(zinfo, max_n, &mut |chunk: &[u8]| {
            out.extend_from_slice(chunk);
            Ok::<(), ZipError>(())
        })
        .map_err(|e| match e {
            ChunkStop::Zip(e) | ChunkStop::Sink(e) => e,
        })?;
        Ok(out)
    }

    /// Boucle de `ZipExtFile.read()` : chaque tour (au plus `max_n` octets) est passé à
    /// `sink`, puis le CRC est vérifié.
    fn pump<E>(
        mut self,
        zinfo: &Entry,
        max_n: usize,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<(), ChunkStop<E>> {
        if self.codec.is_some() {
            return self.pump_codec(zinfo, MAX_N, max_n, sink);
        }
        let mut crc = crc32fast::Hasher::new();
        let mut eof = false;
        while !eof {
            // _read1(max_n)
            let mut chunk = if self.deflate {
                let mut data = std::mem::take(&mut self.tail);
                if max_n > data.len() {
                    let more = self.read2(max_n - data.len())?;
                    data.extend_from_slice(&more);
                }
                let keep = usize::try_from(self.left).unwrap_or(usize::MAX);
                let (chunk, used, stream_end, produced) = self.inflate(&data, max_n, keep)?;
                if !stream_end && produced >= max_n {
                    self.tail = data[used..].to_vec();
                }
                // Le flush() final ne rend rien de plus : chaque tour vide déjà la sortie
                // (Z_SYNC_FLUSH).
                // Tour plein sans fin de flux : miniz_oxide peut avoir consommé toute l'entrée
                // en gardant de la sortie en attente (zlib laisserait ces octets dans
                // `unconsumed_tail`) ; un tour de plus la vide.
                eof = stream_end
                    || (self.compress_left == 0 && self.tail.is_empty() && produced < max_n);
                chunk
            } else {
                let data = self.read2(max_n)?;
                eof = self.compress_left == 0;
                data
            };
            let n = (chunk.len() as u64).min(self.left);
            chunk.truncate(n as usize);
            self.left -= n;
            if self.left == 0 {
                eof = true;
            }
            crc.update(&chunk);
            if !chunk.is_empty() {
                sink(&chunk).map_err(ChunkStop::Sink)?;
            }
        }
        if crc.finalize() != zinfo.crc {
            return Err(corrupt(format!("Bad CRC-32 for file {:?}", zinfo.filename)).into());
        }
        Ok(())
    }

    /// bzip2, LZMA, Zstandard : boucle de `ZipExtFile.read()`, où `_read1` se réduit à
    /// `decompressor.decompress(self._read2(n))`. Chaque tour lit `round` octets compressés
    /// (`MAX_N`, au moins `MIN_READ_SIZE`, au plus ce qui reste ; `EOFError` si le fichier n'en
    /// donne aucun) et les décode **tous**, même au-delà de `file_size` (Python aussi : une
    /// erreur du flux y est détectée), mais seuls les `file_size` premiers octets sont gardés.
    /// Les tours s'arrêtent à la fin du flux, à la fin des données compressées ou quand
    /// `file_size` octets sont rendus ; puis le CRC est vérifié. La mémoire reste bornée : le
    /// tour est lu par morceaux de `CODEC_PIECE` octets et la sortie passée à `sink` par
    /// morceaux d'au plus `out_chunk` octets.
    fn pump_codec<E>(
        mut self,
        zinfo: &Entry,
        round: usize,
        out_chunk: usize,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), E>,
    ) -> Result<(), ChunkStop<E>> {
        let mut codec = self.codec.take().expect("décompresseur présent");
        let out_chunk = out_chunk.max(1);
        let mut crc = crc32fast::Hasher::new();
        let mut left = self.left;
        let mut sink_err: Option<E> = None;
        loop {
            let n = (round.max(MIN_READ_SIZE) as u64).min(self.compress_left);
            let mut got = 0u64;
            let mut eof;
            let mut short;
            loop {
                let want = (CODEC_PIECE as u64).min(n - got);
                let piece = read_up_to(self.reader, want).map_err(io_corrupt)?;
                got += piece.len() as u64;
                self.compress_left -= piece.len() as u64;
                if piece.is_empty() && got == 0 && n > 0 {
                    return Err(corrupt("EOFError : fin de fichier dans une entrée").into());
                }
                // Dernier morceau du tour : tour complet, ou fin du fichier.
                short = (piece.len() as u64) < want;
                let last = got == n || short;
                let r = codec.feed(&piece, last, &mut |chunk: &[u8]| {
                    deliver(
                        chunk,
                        &mut left,
                        &mut crc,
                        out_chunk,
                        &mut *sink,
                        &mut sink_err,
                    )
                });
                eof = codec_result(r, &mut sink_err, zinfo)?;
                // Après la fin du flux, le reste du tour est ignoré (`unused_data`).
                if eof || last {
                    break;
                }
            }
            if !eof && left > 0 && (self.compress_left == 0 || short) {
                // Plus rien à lire (fin des données ou du fichier) : ce que le décodeur retenait encore est rendu, comme les
                // bibliothèques C le rendent au fil de l'eau (flux tronqué).
                let r = codec.finish(&mut |chunk: &[u8]| {
                    deliver(
                        chunk,
                        &mut left,
                        &mut crc,
                        out_chunk,
                        &mut *sink,
                        &mut sink_err,
                    )
                });
                codec_result(r, &mut sink_err, zinfo)?;
            }
            if eof || self.compress_left == 0 || left == 0 {
                break;
            }
        }
        if crc.finalize() != zinfo.crc {
            return Err(corrupt(format!("Bad CRC-32 for file {:?}", zinfo.filename)).into());
        }
        Ok(())
    }
}

/// Sortie d'un décodeur bzip2/LZMA/Zstandard : `data[:self._left]`, CRC, puis `sink` par
/// morceaux d'au plus `out_chunk` octets. Un refus de `sink` est noté dans `err`.
fn deliver<E>(
    chunk: &[u8],
    left: &mut u64,
    crc: &mut crc32fast::Hasher,
    out_chunk: usize,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), E>,
    err: &mut Option<E>,
) -> Result<(), ()> {
    let k = (chunk.len() as u64).min(*left) as usize;
    let kept = &chunk[..k];
    *left -= k as u64;
    crc.update(kept);
    for part in kept.chunks(out_chunk) {
        if let Err(e) = sink(part) {
            *err = Some(e);
            return Err(());
        }
    }
    Ok(())
}

/// Erreur d'un décodeur bzip2/LZMA/Zstandard : flux invalide (Python échoue aussi, code 3 en
/// Rust) ou refus du destinataire noté par [`deliver`].
fn codec_result<T, E>(
    r: Result<T, Halt>,
    sink_err: &mut Option<E>,
    zinfo: &Entry,
) -> Result<T, ChunkStop<E>> {
    match r {
        Ok(v) => Ok(v),
        Err(Halt::Sink) => Err(ChunkStop::Sink(
            sink_err.take().expect("refus du destinataire noté"),
        )),
        Err(Halt::Data(m)) => {
            Err(ZipError::PythonFailsToo(format!("{:?} : {m}", zinfo.filename)).into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor as IoCursor;

    #[test]
    fn nettoyage_des_noms_selon_le_systeme() {
        // Windows (`os.sep == "\\"`) : `\` devient `/`, coupure au premier NUL.
        assert_eq!(sanitize_filename_for("a\\b/c\\d", true), "a/b/c/d");
        assert_eq!(sanitize_filename_for("a\\b\0c\\d", true), "a/b");
        // Autres systèmes : `\` est un caractère de nom ordinaire, coupure au NUL seule.
        assert_eq!(sanitize_filename_for("a\\b/c\\d", false), "a\\b/c\\d");
        assert_eq!(sanitize_filename_for("a\\b\0c\\d", false), "a\\b");
        assert_eq!(sanitize_filename_for("\0x", false), "");
        let here = if cfg!(windows) { "a/b" } else { "a\\b" };
        assert_eq!(sanitize_filename("a\\b"), here);
    }

    /// Archive *stored* d'une entrée, avec champ ZIP64 dans le répertoire central et
    /// enregistrements de fin ZIP64 (comme une archive de plus de 4 Gio, en miniature).
    fn zip64(content: &[u8], prefix: &[u8]) -> Vec<u8> {
        let name = b"metadata.json";
        let crc = crc32fast::hash(content);
        let mut z = prefix.to_vec();
        let local = z.len() as u64 - prefix.len() as u64;
        z.extend_from_slice(b"PK\x03\x04");
        z.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&(content.len() as u32).to_le_bytes());
        z.extend_from_slice(&(content.len() as u32).to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z.extend_from_slice(name);
        z.extend_from_slice(content);
        let cd_start = z.len() as u64 - prefix.len() as u64;
        z.extend_from_slice(b"PK\x01\x02");
        z.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&crc.to_le_bytes());
        z.extend_from_slice(&u32::MAX.to_le_bytes());
        z.extend_from_slice(&u32::MAX.to_le_bytes());
        z.extend_from_slice(&(name.len() as u16).to_le_bytes());
        z.extend_from_slice(&28u16.to_le_bytes());
        z.extend_from_slice(&[0; 8]);
        z.extend_from_slice(&[0; 2]);
        z.extend_from_slice(&u32::MAX.to_le_bytes());
        z.extend_from_slice(name);
        z.extend_from_slice(&1u16.to_le_bytes());
        z.extend_from_slice(&24u16.to_le_bytes());
        z.extend_from_slice(&(content.len() as u64).to_le_bytes());
        z.extend_from_slice(&(content.len() as u64).to_le_bytes());
        z.extend_from_slice(&local.to_le_bytes());
        let cd_size = z.len() as u64 - prefix.len() as u64 - cd_start;
        let eocd64 = z.len() as u64 - prefix.len() as u64;
        z.extend_from_slice(b"PK\x06\x06");
        z.extend_from_slice(&44u64.to_le_bytes());
        z.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
        z.extend_from_slice(&1u64.to_le_bytes());
        z.extend_from_slice(&1u64.to_le_bytes());
        z.extend_from_slice(&cd_size.to_le_bytes());
        z.extend_from_slice(&cd_start.to_le_bytes());
        z.extend_from_slice(b"PK\x06\x07");
        z.extend_from_slice(&0u32.to_le_bytes());
        z.extend_from_slice(&eocd64.to_le_bytes());
        z.extend_from_slice(&1u32.to_le_bytes());
        z.extend_from_slice(b"PK\x05\x06");
        z.extend_from_slice(&[0, 0, 0, 0, 0xFF, 0xFF, 0xFF, 0xFF]);
        z.extend_from_slice(&u32::MAX.to_le_bytes());
        z.extend_from_slice(&u32::MAX.to_le_bytes());
        z.extend_from_slice(&0u16.to_le_bytes());
        z
    }

    /// Sortie de taille exactement égale à un tour (`MAX_N`), à un multiple, ou à ±1 : le flux
    /// se termine pile à la borne (NIT de #161 : « 1 Gio pile en un tour »). `MAX_N` vaut
    /// `2^31 - 1` : on teste la même logique avec un tour de 5000 octets.
    #[test]
    fn sortie_aux_bornes_d_un_tour() {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;

        const TOUR: usize = 5000;
        let mut seed = 7u32;
        let mut noise = |n: usize| -> Vec<u8> {
            (0..n)
                .map(|_| {
                    seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                    (seed >> 16) as u8
                })
                .collect()
        };
        for size in [
            TOUR - 1,
            TOUR,
            TOUR + 1,
            2 * TOUR - 1,
            2 * TOUR,
            2 * TOUR + 1,
            3 * TOUR,
        ] {
            for content in [noise(size), vec![b'a'; size]] {
                let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
                enc.write_all(&content).unwrap();
                let comp = enc.finish().unwrap();
                let crc = crc32fast::hash(&content);
                // Avec et sans octets après la fin du flux (cas où `tail` est non vide).
                for extra in [0usize, 1, 100] {
                    let mut data = comp.clone();
                    data.extend(std::iter::repeat_n(0x55u8, extra));
                    // Taille compressée de l'en-tête : flux et octets en trop, comme dans une
                    // archive (`compress_left` part de la même valeur).
                    let entry = Entry {
                        filename: "x".into(),
                        orig_filename: "x".into(),
                        flag_bits: 0,
                        compress_type: ZIP_DEFLATED,
                        crc,
                        compress_size: data.len() as u64,
                        file_size: size as u64,
                        header_offset: 0,
                        end_offset: 0,
                    };
                    let mut cursor = IoCursor::new(data.clone());
                    let ext = ExtFile {
                        reader: &mut cursor,
                        deflate: true,
                        compress_left: data.len() as u64,
                        left: size as u64,
                        decompressor: Decompress::new(false),
                        tail: Vec::new(),
                        codec: None,
                    };
                    let got = ext
                        .read_all(&entry, TOUR)
                        .unwrap_or_else(|e| panic!("taille {size}, {extra} octets en trop : {e}"));
                    assert!(got == content, "taille {size}, {extra} octets en trop");
                }
            }
        }
    }

    /// Lecture par tours (#402) : tours d'au plus `chunk` octets, contenu identique à `read`,
    /// CRC faux détecté après le dernier tour, refus du destinataire rendu tel quel.
    #[test]
    fn lecture_par_tours() {
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;

        let content: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(&content).unwrap();
        let comp = enc.finish().unwrap();
        for (deflate, data) in [(false, content.clone()), (true, comp)] {
            for crc in [crc32fast::hash(&content), 0] {
                let entry = Entry {
                    filename: "x".into(),
                    orig_filename: "x".into(),
                    flag_bits: 0,
                    compress_type: if deflate { ZIP_DEFLATED } else { ZIP_STORED },
                    crc,
                    compress_size: data.len() as u64,
                    file_size: content.len() as u64,
                    header_offset: 0,
                    end_offset: 0,
                };
                for chunk in [1usize, 4096, 4097, 7000, 1 << 20] {
                    let mut cursor = IoCursor::new(data.clone());
                    let ext = ExtFile {
                        reader: &mut cursor,
                        deflate,
                        compress_left: data.len() as u64,
                        left: content.len() as u64,
                        decompressor: Decompress::new(false),
                        tail: Vec::new(),
                        codec: None,
                    };
                    let mut got = Vec::new();
                    let r = ext.pump(&entry, chunk, &mut |c: &[u8]| {
                        assert!(!c.is_empty() && c.len() <= chunk.max(MIN_READ_SIZE));
                        got.extend_from_slice(c);
                        Ok::<(), ()>(())
                    });
                    let what = format!("deflate {deflate}, crc {crc}, tour {chunk}");
                    // Tout est transmis avant le contrôle du CRC.
                    assert!(got == content, "{what}");
                    match r {
                        Ok(()) => assert_ne!(crc, 0, "{what}"),
                        Err(ChunkStop::Zip(ZipError::Corrupt(m))) => {
                            assert_eq!(crc, 0, "{what}");
                            assert!(m.starts_with("Bad CRC-32"), "{what} : {m}");
                        }
                        Err(e) => panic!("{what} : {e:?}"),
                    }
                }
            }
        }
        // Refus du destinataire au deuxième tour : rendu tel quel, lecture arrêtée.
        let mut zr = ZipReader::new(IoCursor::new(zip64(&content, b""))).unwrap();
        let mut calls = 0;
        let r = zr.read_chunks("metadata.json", 5000, &mut |_| {
            calls += 1;
            if calls == 2 {
                Err("stop")
            } else {
                Ok(())
            }
        });
        assert!(matches!(r, Err(ChunkStop::Sink("stop"))));
        assert_eq!(calls, 2);
        let mut all = Vec::new();
        let found = zr
            .read_chunks("metadata.json", 5000, &mut |c: &[u8]| {
                all.extend_from_slice(c);
                Ok::<(), ()>(())
            })
            .unwrap();
        assert!(found && all == content);
        assert!(!zr
            .read_chunks("absent", 5000, &mut |_| Ok::<(), ()>(()))
            .unwrap());
    }

    /// bzip2, LZMA, Zstandard (#458) : même contenu en tours de `MIN_READ_SIZE` octets
    /// compressés qu'en un seul tour, et décodeur alimenté par petits morceaux (reprise d'un
    /// bloc ou d'un symbole incomplet) ; refus du destinataire rendu tel quel.
    #[test]
    fn bzip2_lzma_zstd_par_tours_et_par_morceaux() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/archive/");
        for id in [
            "bzip2_multiblock",
            "bzip2_runs",
            "bzip2_empty",
            "lzma_runs",
            "lzma_no_eos",
            "lzma_truncated_partial",
            "zstd_level19",
            "zstd_noise",
            "zstd_checksum",
            "zstd_raw_truncated_partial",
            "zstd_multiblock_truncated_partial",
        ] {
            let data = std::fs::read(format!("{dir}{id}.zip")).unwrap();
            let mut zr = ZipReader::new(IoCursor::new(data)).unwrap();
            let want = zr.read("images/c.png").unwrap().unwrap();
            let entry = zr.entries[zr.name_to_info["images/c.png"]].clone();
            let ext = zr.open_entry(&entry).unwrap();
            let mut got = Vec::new();
            ext.pump_codec(&entry, MIN_READ_SIZE, 1000, &mut |c: &[u8]| {
                assert!(!c.is_empty() && c.len() <= 1000);
                got.extend_from_slice(c);
                Ok::<(), ()>(())
            })
            .unwrap_or_else(|e| panic!("{id} : {e:?}"));
            assert!(got == want, "{id} : tours de {MIN_READ_SIZE} octets");

            // Décodeur seul, par morceaux, sans `flush` avant le dernier.
            let mut ext = zr.open_entry(&entry).unwrap();
            let comp = read_up_to(ext.reader, entry.compress_size).unwrap();
            let mut codec = ext.codec.take().unwrap();
            let step = if comp.len() < 5000 { 1 } else { 61 };
            let mut out = Vec::new();
            let mut eof = false;
            let parts: Vec<&[u8]> = comp.chunks(step).collect();
            for (i, p) in parts.iter().enumerate() {
                let last = i + 1 == parts.len();
                eof = codec
                    .feed(p, last, &mut |c: &[u8]| {
                        out.extend_from_slice(c);
                        Ok(())
                    })
                    .unwrap_or_else(|e| panic!("{id} : {e:?}"));
                if eof {
                    break;
                }
            }
            if !eof {
                codec
                    .finish(&mut |c: &[u8]| {
                        out.extend_from_slice(c);
                        Ok(())
                    })
                    .unwrap();
            }
            out.truncate(want.len());
            assert!(out == want, "{id} : morceaux de {step} octets");

            // Refus du destinataire : rendu tel quel, lecture arrêtée.
            if !want.is_empty() {
                let ext = zr.open_entry(&entry).unwrap();
                let r = ext.pump_codec(&entry, MAX_N, 1000, &mut |_| Err("stop"));
                assert!(matches!(r, Err(ChunkStop::Sink("stop"))), "{id}");
            }
        }
    }

    /// Mutations directes des flux bzip2/LZMA/Zstandard (décodeurs seuls, sans le CRC de
    /// l'entrée) : aucune panique, chaque décodage se termine. `TELMI2FLAM_FUZZ_ITER` règle le
    /// nombre de mutations par flux (500 par défaut ; davantage en local, en release).
    #[test]
    fn decodeurs_sous_mutations() {
        let iters: usize = std::env::var("TELMI2FLAM_FUZZ_ITER")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(500);
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vectors/archive/");
        let mut seed: u64 = 0xD1B5_4A32_D192_ED03;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for id in [
            "bzip2_text",
            "bzip2_runs",
            "lzma_text",
            "lzma_no_eos",
            "zstd_text",
            "zstd_checksum",
            "zstd_level19",
        ] {
            let data = std::fs::read(format!("{dir}{id}.zip")).unwrap();
            let mut zr = ZipReader::new(IoCursor::new(data)).unwrap();
            let entry = zr.entries[zr.name_to_info["images/c.png"]].clone();
            let mut ext = zr.open_entry(&entry).unwrap();
            let comp = read_up_to(ext.reader, entry.compress_size).unwrap();
            let method = entry.compress_type;
            drop(ext.codec.take());
            for _ in 0..iters {
                let mut v = comp.clone();
                for _ in 0..1 + next() % 4 {
                    let i = (next() % v.len() as u64) as usize;
                    v[i] = (next() & 0xFF) as u8;
                }
                if next() % 4 == 0 {
                    v.truncate((next() % v.len() as u64) as usize);
                }
                let mut codec: Box<dyn StreamDecoder> = match method {
                    ZIP_BZIP2 => Box::new(Bzip2Decoder::new()),
                    ZIP_LZMA => Box::new(LzmaDecoder::new()),
                    _ => Box::new(ZstdDecoder::new()),
                };
                let mut n = 0usize;
                let step = 1 + (next() % 5000) as usize;
                let parts: Vec<&[u8]> = v.chunks(step).collect();
                let mut ended = false;
                for (k, p) in parts.iter().enumerate() {
                    match codec.feed(p, k + 1 == parts.len(), &mut |c: &[u8]| {
                        n += c.len();
                        Ok(())
                    }) {
                        Ok(true) | Err(_) => {
                            ended = true;
                            break;
                        }
                        Ok(false) => {}
                    }
                }
                if !ended {
                    let _ = codec.finish(&mut |c: &[u8]| {
                        n += c.len();
                        Ok(())
                    });
                }
                // Borne grossière : aucune expansion démesurée sur ces petits flux.
                assert!(n < 1 << 28, "{id}");
            }
        }
    }

    #[test]
    fn zip64_lu_comme_python() {
        for prefix in [&b""[..], b"prefixe autoextractible"] {
            let mut zr = ZipReader::new(IoCursor::new(zip64(b"{\"a\": 1}", prefix))).unwrap();
            assert_eq!(zr.namelist(), vec!["metadata.json".to_string()]);
            assert_eq!(zr.read("metadata.json").unwrap().unwrap(), b"{\"a\": 1}");
            assert_eq!(zr.read("absent").unwrap(), None);
        }
    }

    #[test]
    fn zip64_corrompu() {
        let good = zip64(b"x", b"");
        // Champ ZIP64 trop court pour les trois tailles annoncées.
        let mut short = good.clone();
        let i = short.windows(4).position(|w| w == [1, 0, 24, 0]).unwrap();
        short[i + 2] = 8;
        assert!(matches!(
            ZipReader::new(IoCursor::new(short)),
            Err(ZipError::Corrupt(_))
        ));
        // Plusieurs disques.
        let mut disks = good.clone();
        let i = disks.windows(4).rposition(|w| w == b"PK\x06\x07").unwrap();
        disks[i + 16] = 2;
        assert!(matches!(
            ZipReader::new(IoCursor::new(disks)),
            Err(ZipError::Corrupt(_))
        ));
    }

    #[test]
    fn cp437_complet() {
        let all: Vec<u8> = (0..=255).collect();
        let s = decode_cp437(&all);
        assert_eq!(s.chars().count(), 256);
        assert!(s.starts_with('\0'));
        assert_eq!(s.chars().nth(0x82), Some('é'));
        assert_eq!(s.chars().nth(0xFF), Some('\u{a0}'));
    }
}
