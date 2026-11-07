//! Contrôle des fichiers audio d'une histoire STUdio avant copie sur l'appareil : un MP3 conforme
//! est copié tel quel (étiquettes retirées), tout le reste est transcodé par ffmpeg (que le
//! launcher possède déjà, `apps/flam-launcher/src-tauri/src/ffmpeg.rs`). Le transcodage n'est donc
//! pas dans cette crate : elle expose seulement la décision ([`transcoding_required`]), les
//! paramètres cibles ([`TARGET_SAMPLE_RATE_HZ`], [`TARGET_CHANNELS`], [`FFMPEG_TRANSCODE_ARGS`]…)
//! et le retrait des étiquettes ([`mp3_tag_cleanup`]).
//!
//! Comportement repris de Lunii.QT (`convert_audio.py`, qui s'appuie sur la bibliothèque
//! `mutagen`), réécrit en Rust pur : analyse bornée des en-têtes de trame, aucune dépendance.
//!
//! # Un MP3 est accepté tel quel (pas de transcodage) quand
//!
//! | Critère | Valeur acceptée | Origine |
//! |---|---|---|
//! | Nom du fichier | se termine par `.mp3` (casse ignorée) | Lunii.QT |
//! | Première trame | une trame MPEG valide est trouvée (1 Mio explorés, 1499 synchros au plus) | `mutagen` |
//! | Couche | III | **plus strict** que Lunii.QT (il accepte aussi les couches I et II) |
//! | Fréquence | `>= 44100 Hz` (donc 44,1 et 48 kHz ; 32 kHz, 24, 22,05, 16… sont transcodés) | Lunii.QT |
//! | Mode de débit | inconnu (CBR présumé), CBR ou VBR ; **ABR refusé** (en-tête LAME) | Lunii.QT |
//! | Canaux | mono (mode de canal 3) ; stéréo, joint stéréo, double canal sont transcodés | Lunii.QT |
//!
//! Le débit lui-même n'est pas un critère. Un fichier dont aucune trame n'est trouvée (Lunii.QT
//! lève alors une exception) est déclaré « à transcoder » : ffmpeg tranchera.
//!
//! # Paramètres du transcodage
//!
//! Ceux de Lunii.QT (`audio_to_mp3`), à passer à ffmpeg après `-i`, la sortie allant sur la sortie
//! standard : voir [`FFMPEG_TRANSCODE_ARGS`].
//!
//! | Paramètre ffmpeg | Valeur | Constante |
//! |---|---|---|
//! | `-map` | `0:a` (flux audio seulement) | |
//! | `-ar` | 44100 | [`TARGET_SAMPLE_RATE_HZ`] |
//! | `-ac` | 1 (mono) | [`TARGET_CHANNELS`] |
//! | `-codec:a` | `libmp3lame` | |
//! | `-aq` | 5 (VBR qualité 5, environ 130 kb/s ; surtout pas `-b:a`, Lunii.QT : « NOOO CBR ») | [`TARGET_MP3_VBR_QUALITY`] |
//! | `-map_metadata` | `-1` (aucune métadonnée copiée) | |
//! | `-write_xing` | 0 (pas d'en-tête Xing/Info) | |
//! | `-id3v2_version` | 0 (pas d'ID3v2) | |
//! | `-f` | `mp3` | |
//!
//! # Étiquettes retirées ([`mp3_tag_cleanup`])
//!
//! | Étiquette | Emplacement | Retrait |
//! |---|---|---|
//! | ID3v2.2 / 2.3 / 2.4 | début, `ID3`, 10 octets d'en-tête + taille synchsafe (+ 10 si pied `3DI`, drapeau `0x10`) | oui, répétée si plusieurs étiquettes sont empilées (16 au plus) |
//! | ID3v1 / v1.1 | 128 derniers octets, `TAG` | oui |
//! | APEv2 | pied `APETAGEX` de 32 octets en fin de fichier (avant l'ID3v1), en-tête éventuel | oui |
//!
//! Écarts avec `mutagen` : il ne retire pas l'APEv2, ni le pied ID3v2.4, ni les étiquettes
//! ID3v2 empilées ; et [`tags_removal_required`] est vrai dès qu'une étiquette existe, alors que
//! `audio.tags` de `mutagen` est faux pour une étiquette sans aucune trame (nettoyage sans effet
//! sur le son, donc sans risque).

use std::ops::Range;

/// Fréquence d'échantillonnage cible du transcodage (`-ar`).
pub const TARGET_SAMPLE_RATE_HZ: u32 = 44_100;
/// Nombre de canaux cible du transcodage (`-ac`) : mono.
pub const TARGET_CHANNELS: u8 = 1;
/// Qualité VBR de `libmp3lame` (`-aq`) : 5, soit environ 130 kb/s. Pas de débit constant.
pub const TARGET_MP3_VBR_QUALITY: u8 = 5;
/// Fréquence minimale d'un MP3 accepté sans transcodage (strictement moins : transcodé).
pub const MIN_ACCEPTED_SAMPLE_RATE_HZ: u32 = 44_100;

/// Arguments ffmpeg de sortie du transcodage, à placer entre l'entrée (`-i`) et la sortie
/// (`pipe:1` ou un fichier). Équivalent de `audio_to_mp3` de Lunii.QT.
pub const FFMPEG_TRANSCODE_ARGS: &[&str] = &[
    "-map",
    "0:a",
    "-ar",
    "44100",
    "-ac",
    "1",
    "-codec:a",
    "libmp3lame",
    "-aq",
    "5",
    "-map_metadata",
    "-1",
    "-write_xing",
    "0",
    "-id3v2_version",
    "0",
    "-f",
    "mp3",
];

/// Octets explorés après les étiquettes ID3v2 pour trouver une synchronisation (comme `mutagen`).
const MAX_SYNC_SCAN: usize = 1024 * 1024;
/// Nombre maximal de synchronisations candidates essayées (comme `mutagen`).
const MAX_SYNCS: u32 = 1500;
/// Trames consécutives contrôlées pour valider une synchronisation.
const ENOUGH_FRAMES: usize = 4;
/// Trames consécutives minimales pour retenir une synchronisation « douteuse ».
const MIN_FRAMES: usize = 2;
/// Nombre maximal d'étiquettes ID3v2 empilées retirées.
const MAX_STACKED_ID3V2: usize = 16;
const ID3V1_LEN: usize = 128;
const APE_FOOTER_LEN: usize = 32;

/// Erreurs du contrôle audio.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AudioError {
    /// Aucune trame MPEG valide dans le fichier.
    #[error("aucune trame MP3 valide trouvée")]
    NoFrame,
    /// La taille annoncée par une étiquette dépasse le fichier.
    #[error("étiquette {kind} tronquée : {declared} octets annoncés, {available} disponibles")]
    TruncatedTag {
        /// Type d'étiquette (`ID3v2`).
        kind: &'static str,
        /// Taille annoncée par l'étiquette.
        declared: usize,
        /// Octets réellement disponibles.
        available: usize,
    },
}

/// Version MPEG d'une trame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MpegVersion {
    /// MPEG-1 (32, 44,1 et 48 kHz).
    V1,
    /// MPEG-2 (16, 22,05 et 24 kHz).
    V2,
    /// MPEG-2.5 (8, 11,025 et 12 kHz).
    V25,
}

/// Mode de débit, déduit des en-têtes Xing / Info / LAME / VBRI (comme `mutagen`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitrateMode {
    /// Pas d'en-tête de débit : probablement constant.
    Unknown,
    /// Débit constant.
    Cbr,
    /// Débit variable.
    Vbr,
    /// Débit moyen (variante de VBR) : refusé sans transcodage.
    Abr,
}

/// Ce qu'on lit de la première trame d'un MP3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mp3Info {
    /// Version MPEG.
    pub version: MpegVersion,
    /// Couche (1, 2 ou 3).
    pub layer: u8,
    /// Fréquence d'échantillonnage en Hz.
    pub sample_rate: u32,
    /// Mode de canal brut : 0 stéréo, 1 joint stéréo, 2 double canal, 3 mono.
    pub channel_mode: u8,
    /// Mode de débit.
    pub bitrate_mode: BitrateMode,
    /// Vrai si la synchronisation n'est pas confirmée (moins de 4 trames consécutives).
    pub sketchy: bool,
}

impl Mp3Info {
    /// Nombre de canaux : 1 en mono, 2 sinon.
    pub fn channels(&self) -> u8 {
        if self.channel_mode == 3 {
            1
        } else {
            2
        }
    }
}

/// Faut-il transcoder ce fichier ? `filename` est le nom dans l'archive STUdio (seule l'extension
/// compte). Voir le tableau des critères en en-tête de module.
pub fn transcoding_required(filename: &str, bytes: &[u8]) -> bool {
    if !filename.to_ascii_lowercase().ends_with(".mp3") {
        return true;
    }
    let Ok(info) = probe_mp3(bytes) else {
        return true;
    };
    info.layer != 3
        || info.sample_rate < MIN_ACCEPTED_SAMPLE_RATE_HZ
        || info.bitrate_mode == BitrateMode::Abr
        || info.channels() != TARGET_CHANNELS
}

/// Le MP3 porte-t-il une étiquette (ID3v2, ID3v1 ou APEv2) à retirer ?
pub fn tags_removal_required(bytes: &[u8]) -> bool {
    id3v2_len_at(bytes, 0).is_some() || id3v1_present(bytes) || ape_range(bytes).is_some()
}

/// Intervalle des octets audio, une fois les étiquettes écartées. Évite la copie quand l'appelant
/// peut travailler sur une tranche.
pub fn mp3_audio_range(bytes: &[u8]) -> Result<Range<usize>, AudioError> {
    let mut start = 0;
    for _ in 0..MAX_STACKED_ID3V2 {
        let Some(len) = id3v2_len_at(&bytes[start..], 0) else {
            break;
        };
        if start + len > bytes.len() {
            return Err(AudioError::TruncatedTag {
                kind: "ID3v2",
                declared: len,
                available: bytes.len() - start,
            });
        }
        start += len;
    }
    let mut end = bytes.len();
    if id3v1_present(&bytes[start..end]) {
        end -= ID3V1_LEN;
    }
    if let Some(r) = ape_range(&bytes[start..end]) {
        end = start + r.start;
    }
    Ok(start..end)
}

/// Copie du MP3 sans étiquette ID3v2, ID3v1 ni APEv2 (équivalent de `mp3_tag_cleanup`).
pub fn mp3_tag_cleanup(bytes: &[u8]) -> Result<Vec<u8>, AudioError> {
    let r = mp3_audio_range(bytes)?;
    Ok(bytes[r].to_vec())
}

/// Lit la première trame d'un MP3 (après les étiquettes ID3v2 de tête).
pub fn probe_mp3(bytes: &[u8]) -> Result<Mp3Info, AudioError> {
    let mut start = 0;
    // « WMP écrit plusieurs ID3 » : `mutagen` en saute autant qu'il en trouve. Une étiquette dont
    // la taille déborde du fichier arrête simplement le saut.
    while start < bytes.len() {
        match id3v2_len_at(&bytes[start..], 0) {
            Some(len) if start + len <= bytes.len() => start += len,
            _ => break,
        }
    }
    let end = bytes.len().min(start.saturating_add(MAX_SYNC_SCAN));

    let mut syncs = MAX_SYNCS;
    let mut fallback: Option<Frame> = None;
    let mut i = start;
    while i + 1 < end {
        if !(bytes[i] == 0xFF && bytes[i + 1] & 0xE0 == 0xE0) {
            i += 1;
            continue;
        }
        syncs -= 1;
        if syncs == 0 {
            break;
        }
        let mut frames: Vec<Frame> = Vec::with_capacity(ENOUGH_FRAMES);
        let mut pos = i;
        for _ in 0..ENOUGH_FRAMES {
            let Some(frame) = parse_frame(bytes, pos) else {
                break;
            };
            pos += frame.length;
            let confirmed = !frame.sketchy;
            frames.push(frame);
            if confirmed {
                break;
            }
        }
        if frames.len() >= MIN_FRAMES && fallback.is_none() {
            fallback = Some(frames[0]);
        }
        if let Some(last) = frames.last().filter(|f| !f.sketchy) {
            return Ok(last.info(false));
        }
        if frames.len() >= ENOUGH_FRAMES {
            return Ok(frames[0].info(false));
        }
        i += 1;
    }
    fallback.map(|f| f.info(true)).ok_or(AudioError::NoFrame)
}

#[derive(Debug, Clone, Copy)]
struct Frame {
    version: MpegVersion,
    layer: u8,
    mode: u8,
    sample_rate: u32,
    length: usize,
    sketchy: bool,
    bitrate_mode: BitrateMode,
}

impl Frame {
    fn info(&self, sketchy: bool) -> Mp3Info {
        Mp3Info {
            version: self.version,
            layer: self.layer,
            sample_rate: self.sample_rate,
            channel_mode: self.mode,
            bitrate_mode: self.bitrate_mode,
            sketchy,
        }
    }
}

const BITRATE_V1: [[u32; 15]; 3] = [
    [
        0, 32, 64, 96, 128, 160, 192, 224, 256, 288, 320, 352, 384, 416, 448,
    ],
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384,
    ],
    [
        0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320,
    ],
];
const BITRATE_V2_L1: [u32; 15] = [
    0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
];
const BITRATE_V2_L23: [u32; 15] = [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160];

/// Analyse l'en-tête de trame à `pos` (mêmes rejets que `mutagen`) et, pour la couche III,
/// cherche un en-tête Xing / Info / VBRI qui rend la trame « sûre » (non douteuse).
fn parse_frame(data: &[u8], pos: usize) -> Option<Frame> {
    let h = data.get(pos..pos.checked_add(4)?)?;
    if h[0] != 0xFF || h[1] & 0xE0 != 0xE0 {
        return None;
    }
    let version_bits = (h[1] >> 3) & 3;
    let layer_bits = (h[1] >> 1) & 3;
    let bitrate_idx = usize::from(h[2] >> 4);
    let rate_idx = usize::from((h[2] >> 2) & 3);
    let padding = u32::from((h[2] >> 1) & 1);
    let mode = (h[3] >> 6) & 3;
    if version_bits == 1
        || layer_bits == 0
        || rate_idx == 3
        || bitrate_idx == 15
        || bitrate_idx == 0
    {
        return None;
    }
    let version = match version_bits {
        0 => MpegVersion::V25,
        2 => MpegVersion::V2,
        _ => MpegVersion::V1,
    };
    let layer = 4 - layer_bits;
    let kbps = match (version, layer) {
        (MpegVersion::V1, l) => BITRATE_V1[usize::from(l) - 1][bitrate_idx],
        (_, 1) => BITRATE_V2_L1[bitrate_idx],
        _ => BITRATE_V2_L23[bitrate_idx],
    };
    let sample_rate = match (version, rate_idx) {
        (MpegVersion::V1, 0) => 44_100,
        (MpegVersion::V1, 1) => 48_000,
        (MpegVersion::V1, _) => 32_000,
        (MpegVersion::V2, 0) => 22_050,
        (MpegVersion::V2, 1) => 24_000,
        (MpegVersion::V2, _) => 16_000,
        (MpegVersion::V25, 0) => 11_025,
        (MpegVersion::V25, 1) => 12_000,
        (MpegVersion::V25, _) => 8_000,
    };
    let (samples, slot) = if layer == 1 {
        (384, 4)
    } else if version != MpegVersion::V1 && layer == 3 {
        (576, 1)
    } else {
        (1152, 1)
    };
    let length = (((samples / 8 * kbps * 1000) / sample_rate + padding) * slot) as usize;

    let mut frame = Frame {
        version,
        layer,
        mode,
        sample_rate,
        length,
        sketchy: true,
        bitrate_mode: BitrateMode::Unknown,
    };
    if layer == 3 {
        let xing_offset = match (version, mode) {
            (MpegVersion::V1, 3) => 21,
            (MpegVersion::V1, _) => 36,
            (_, 3) => 13,
            (_, _) => 21,
        };
        if let Some(bitrate_mode) = parse_xing(data, pos + xing_offset) {
            frame.sketchy = false;
            frame.bitrate_mode = bitrate_mode;
        } else if parse_vbri(data, pos + 36) {
            frame.sketchy = false;
            frame.bitrate_mode = BitrateMode::Vbr;
        }
    }
    Some(frame)
}

fn be32(d: &[u8], at: usize) -> Option<u32> {
    let b = d.get(at..at.checked_add(4)?)?;
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// En-tête Xing / Info à `at` : renvoie le mode de débit, ou `None` si absent ou tronqué.
fn parse_xing(d: &[u8], at: usize) -> Option<BitrateMode> {
    let tag = d.get(at..at.checked_add(4)?)?;
    let is_info = match tag {
        b"Xing" => false,
        b"Info" => true,
        _ => return None,
    };
    let flags = be32(d, at + 4)?;
    let mut p = at + 8;
    if flags & 0x1 != 0 {
        be32(d, p)?;
        p += 4;
    }
    if flags & 0x2 != 0 {
        be32(d, p)?;
        p += 4;
    }
    if flags & 0x4 != 0 {
        d.get(p..p.checked_add(100)?)?;
        p += 100;
    }
    let mut vbr_scale = None;
    if flags & 0x8 != 0 {
        vbr_scale = Some(be32(d, p)?);
        p += 4;
    }
    let (lame_vbr_method, has_version) = parse_lame(d, p);
    match lame_vbr_method {
        Some(1 | 8) => return Some(BitrateMode::Cbr),
        Some(2 | 9) => return Some(BitrateMode::Abr),
        Some(3..=6) => return Some(BitrateMode::Vbr),
        _ => {}
    }
    if is_info {
        return Some(BitrateMode::Cbr);
    }
    if vbr_scale.is_some() || has_version {
        return Some(BitrateMode::Vbr);
    }
    Some(BitrateMode::Unknown)
}

/// Chaîne de version LAME à `p` puis, si elle annonce l'en-tête étendu, sa méthode de débit.
/// Renvoie (méthode de débit, chaîne de version reconnue).
fn parse_lame(d: &[u8], p: usize) -> (Option<u8>, bool) {
    let Some(raw) = p.checked_add(20).and_then(|e| d.get(p..e)) else {
        return (None, false);
    };
    if !(raw.starts_with(b"LAME") || raw.starts_with(b"L3.99")) {
        return (None, false);
    }
    let mut s: &[u8] = raw;
    while let [b'E' | b'M' | b'A' | b'L', rest @ ..] = s {
        s = rest;
    }
    let Some((&major_b, rest)) = s.split_first() else {
        return (None, false);
    };
    let mut rest = rest;
    while let [b'.', r @ ..] = rest {
        rest = r;
    }
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();
    if !major_b.is_ascii_digit() || digits == 0 {
        return (None, false);
    }
    let major = u32::from(major_b - b'0');
    let Some(minor) = std::str::from_utf8(&rest[..digits])
        .ok()
        .and_then(|t| t.parse::<u32>().ok())
    else {
        return (None, false);
    };
    let tail = &rest[digits..];
    // Version antérieure à 3.90 (ou « 3.90 (alpha) ») : pas d'en-tête étendu.
    let old = (major, minor) < (3, 90)
        || ((major, minor) == (3, 90) && tail.len() >= 11 && tail[tail.len() - 11] == b'(');
    if old {
        return (None, true);
    }
    if tail.len() < 11 {
        return (None, false);
    }
    // En-tête étendu : 27 octets, 9 octets après le début de la chaîne de version.
    let Some(ext) = p.checked_add(9).and_then(|s| d.get(s..s.checked_add(27)?)) else {
        return (None, true);
    };
    if ext[0] >> 4 != 0 {
        return (None, true);
    }
    (Some(ext[0] & 0x0F), true)
}

/// En-tête VBRI (Fraunhofer) à `at`, validé comme `mutagen`.
fn parse_vbri(d: &[u8], at: usize) -> bool {
    let Some(h) = at.checked_add(26).and_then(|e| d.get(at..e)) else {
        return false;
    };
    if !h.starts_with(b"VBRI") || u16::from_be_bytes([h[4], h[5]]) != 1 {
        return false;
    }
    let entries = usize::from(u16::from_be_bytes([h[18], h[19]]));
    let entry_size = usize::from(u16::from_be_bytes([h[22], h[23]]));
    if entry_size != 2 && entry_size != 4 {
        return false;
    }
    at + 26 + entries * entry_size <= d.len()
}

/// Longueur totale (en-tête, corps et pied éventuel) de l'étiquette ID3v2 à `at`, si l'en-tête
/// est plausible : `ID3`, version majeure 2 à 4, octets de taille synchsafe.
fn id3v2_len_at(d: &[u8], at: usize) -> Option<usize> {
    let h = d.get(at..at.checked_add(10)?)?;
    if &h[..3] != b"ID3" || !(2..=4).contains(&h[3]) || h[6..10].iter().any(|b| b & 0x80 != 0) {
        return None;
    }
    let size = h[6..10]
        .iter()
        .fold(0usize, |acc, &b| (acc << 7) | usize::from(b));
    let footer = if h[3] == 4 && h[5] & 0x10 != 0 { 10 } else { 0 };
    Some(10 + size + footer)
}

fn id3v1_present(d: &[u8]) -> bool {
    d.len() >= ID3V1_LEN && d[d.len() - ID3V1_LEN..].starts_with(b"TAG")
}

/// Étiquette APEv2 collée en fin de données : intervalle qu'elle occupe, en-tête compris. Le
/// pied de 32 octets doit se trouver à la toute fin (l'appelant a déjà écarté l'ID3v1).
fn ape_range(d: &[u8]) -> Option<Range<usize>> {
    let footer = d.get(d.len().checked_sub(APE_FOOTER_LEN)?..)?;
    if &footer[..8] != b"APETAGEX" {
        return None;
    }
    let size = u32::from_le_bytes([footer[12], footer[13], footer[14], footer[15]]) as usize;
    let flags = u32::from_le_bytes([footer[20], footer[21], footer[22], footer[23]]);
    // Le pied d'un APEv2 n'a pas le bit « est un en-tête » ; `size` couvre les éléments et le pied.
    if flags & 0x2000_0000 != 0 || size < APE_FOOTER_LEN {
        return None;
    }
    let header = if flags & 0x8000_0000 != 0 {
        APE_FOOTER_LEN
    } else {
        0
    };
    let total = size.checked_add(header)?;
    let start = d.len().checked_sub(total)?;
    Some(start..d.len())
}
