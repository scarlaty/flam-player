//! Table de recherche `.mp3map` (format Lunii/Flam), port de l'ancien `mp3map.py` (#140).
//!
//! Format (voir `src/formats/mp3map_parser.c`), little-endian :
//! - en-tête de 12 octets : `total_units` (échantillons × 2, cadence interne 88 200 Hz),
//!   `id3_offset` (position de la première trame audio), puis 4 octets à zéro ;
//! - puis une entrée de 8 octets par seconde : position absolue de la trame, position cumulée
//!   en unités.
//!
//! Les octets viennent d'archives non fiables : toutes les lectures passent par `get`, les
//! positions par des additions saturées, et la seule allocation proportionnelle à l'entrée
//! (liste des trames) est bornée par la taille du fichier (une trame fait au moins 24 octets).
//! La sortie est bornée par l'en-tête : `total_units` tient sur 32 bits, donc au plus
//! `u32::MAX / 88200` entrées.
//!
//! Coût mémoire : la liste des trames occupe 16 octets par trame (position et unités cumulées)
//! et une trame fait au moins 24 octets, soit au plus ~0,67 fois la taille du MP3 (comme la
//! liste de tuples du Python, mais en plus compact), hors pic transitoire de réallocation du
//! `Vec` (jusqu'à ~1,3 fois). Le plafond est donc proportionnel à la taille de l'entrée,
//! que l'appelant borne ; pas de parcours en flux, la table de seek a besoin de la liste
//! complète pour interpoler les entrées par seconde.
//!
//! Sortie identique à `mp3map.build` à l'octet près, y compris les
//! erreurs (même message que l'exception Python).

use std::collections::BTreeSet;
use std::fmt;

/// Cadence interne des unités (2 × 44 100 Hz).
pub const INTERNAL_RATE: u32 = 88_200;

/// Nombre de trames suivantes valides exigées pour accepter une synchronisation.
const SYNC_CHAIN: usize = 2;

/// Débits (kbit/s) par indice, pour MPEG1 puis MPEG2/2.5, Layer I, II, III.
const BITRATES_MPEG1: [[u32; 15]; 3] = [
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
const BITRATES_MPEG2: [[u32; 15]; 3] = [
    [
        0, 32, 48, 56, 64, 80, 96, 112, 128, 144, 160, 176, 192, 224, 256,
    ],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

/// Résultat de [`build`].
#[derive(Debug, Clone, PartialEq)]
pub struct Mp3Map {
    /// Contenu du fichier `.mp3map`.
    pub data: Vec<u8>,
    /// Durée en secondes (`total_units / 88200`).
    pub duration_s: f64,
    /// Nombre d'entrées (une par seconde).
    pub num_records: u32,
    /// Nombre de trames MPEG retenues (`info["frames"]`).
    pub frames: usize,
    /// Fréquences d'échantillonnage rencontrées (`info["samplerates"]`).
    pub samplerates: BTreeSet<u32>,
}

/// Échec de [`build`] ; `Display` reprend le message de l'exception Python.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mp3MapError {
    /// Aucune trame valide suivie de deux trames valides (`ValueError`).
    NoFrame,
    /// Trames couvrant moins de la moitié des données après l'ID3 (`ValueError`).
    NotMpeg { frames: usize, percent: u64 },
    /// Valeur au-delà de 32 bits dans la sortie (`OverflowError` de `int.to_bytes`).
    Overflow,
}

impl fmt::Display for Mp3MapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrame => f.write_str("aucun frame MPEG audio valide"),
            Self::NotMpeg { frames, percent } => write!(
                f,
                "donnees non MPEG ({frames} frames couvrant {percent}% du fichier)"
            ),
            Self::Overflow => f.write_str("int too big to convert"),
        }
    }
}

impl std::error::Error for Mp3MapError {}

/// En-tête de trame décodé.
#[derive(Clone, Copy)]
struct Frame {
    len: usize,
    samples: u32,
    rate: u32,
}

/// Position des trames audio : saute un tag ID3v2 s'il existe (`_skip_id3v2`).
pub(crate) fn skip_id3v2(data: &[u8]) -> usize {
    match data {
        [b'I', b'D', b'3', _, _, flags, s0, s1, s2, s3, ..] => {
            let size = (usize::from(s0 & 0x7F) << 21)
                | (usize::from(s1 & 0x7F) << 14)
                | (usize::from(s2 & 0x7F) << 7)
                | usize::from(s3 & 0x7F);
            let footer = if flags & 0x10 != 0 { 10 } else { 0 };
            10 + size + footer
        }
        _ => 0,
    }
}

/// En-tête de trame valide à `off`, ou `None` (`_parse_frame_header`).
fn parse_frame_header(data: &[u8], off: usize) -> Option<Frame> {
    let &[b0, b1, b2, _] = data.get(off..off.checked_add(4)?)? else {
        return None;
    };
    if b0 != 0xFF || b1 & 0xE0 != 0xE0 {
        return None;
    }
    let version = (b1 >> 3) & 0x03; // 0 = 2.5, 2 = 2, 3 = 1 (1 : réservé)
    let layer = (b1 >> 1) & 0x03; // 1 = III, 2 = II, 3 = I (0 : réservé)
    if version == 1 || layer == 0 {
        return None;
    }
    let bitrate_index = usize::from((b2 >> 4) & 0x0F);
    let sr_index = usize::from((b2 >> 2) & 0x03);
    let padding = u32::from((b2 >> 1) & 0x01);
    if bitrate_index == 0 || bitrate_index == 15 || sr_index == 3 {
        return None;
    }
    let table = if version == 3 {
        &BITRATES_MPEG1
    } else {
        &BITRATES_MPEG2
    };
    let bitrate = table[usize::from(3 - layer)][bitrate_index] * 1000;
    let rate = match version {
        3 => [44100, 48000, 32000][sr_index],
        2 => [22050, 24000, 16000][sr_index],
        _ => [11025, 12000, 8000][sr_index],
    };
    let len = if layer == 3 {
        // Layer I.
        (12 * bitrate / rate + padding) * 4
    } else {
        // Layer II : 1152 échantillons dans toutes les versions (coefficient 144) ; Layer III
        // MPEG2/2.5 : 576 échantillons (coefficient 72).
        let coef = if version == 3 || layer == 2 { 144 } else { 72 };
        coef * bitrate / rate + padding
    };
    if len == 0 {
        return None;
    }
    let samples = match layer {
        1 if version == 3 => 1152,
        1 => 576,
        2 => 1152,
        _ => 384,
    };
    Some(Frame {
        len: len as usize,
        samples,
        rate,
    })
}

/// Vrai si les [`SYNC_CHAIN`] trames suivantes sont valides, ou si on atteint la fin des
/// données (`_chain_ok`) : écarte les fausses synchronisations.
fn chain_ok(data: &[u8], mut pos: usize, mut frame: Frame) -> bool {
    for _ in 0..SYNC_CHAIN {
        pos = pos.saturating_add(frame.len);
        if pos >= data.len() {
            return true;
        }
        match parse_frame_header(data, pos) {
            Some(next) => frame = next,
            None => return false,
        }
    }
    true
}

/// Prochaine trame valide suivie de [`SYNC_CHAIN`] trames valides, à partir de `pos`
/// (`_resync`).
fn resync(data: &[u8], mut pos: usize) -> Option<(usize, Frame)> {
    while pos < data.len() {
        pos += data[pos..].iter().position(|&b| b == 0xFF)?;
        if let Some(frame) = parse_frame_header(data, pos) {
            if chain_ok(data, pos, frame) {
                return Some((pos, frame));
            }
        }
        pos += 1;
    }
    None
}

/// Entier sur 4 octets little-endian, ou [`Mp3MapError::Overflow`] (`int.to_bytes(4, …)`).
fn le32(out: &mut Vec<u8>, v: u64) -> Result<(), Mp3MapError> {
    let v = u32::try_from(v).map_err(|_| Mp3MapError::Overflow)?;
    out.extend_from_slice(&v.to_le_bytes());
    Ok(())
}

/// `int(round(x))` de Python pour un flottant fini positif (arrondi au pair).
fn round_units(x: f64) -> u64 {
    // Conversion saturante : `x` est positif et bien en deçà de 2^64 (au plus ~88 unités par
    // octet d'entrée).
    x.round_ties_even() as u64
}

/// Construit le `.mp3map` d'un fichier MP3 (`mp3map.build`).
///
/// Une entrée par seconde, positions absolues ; position cumulée d'une trame en unités
/// 88 200 Hz (`samples × 88200 / samplerate`).
pub fn build(data: &[u8]) -> Result<Mp3Map, Mp3MapError> {
    let n = data.len();
    let start = skip_id3v2(data);

    let (first_frame, _) = resync(data, start).ok_or(Mp3MapError::NoFrame)?;
    let mut pos = first_frame;

    // (position, unités cumulées avant la trame) : au plus une entrée par 24 octets d'entrée.
    let mut frames: Vec<(usize, u64)> = Vec::new();
    let mut covered: u64 = 0;
    let mut samplerates = BTreeSet::new();
    let mut units = 0.0f64;
    while pos < n {
        let frame = match parse_frame_header(data, pos) {
            Some(frame) => frame,
            None => match resync(data, pos + 1) {
                Some((p, frame)) => {
                    pos = p;
                    frame
                }
                None => break,
            },
        };
        samplerates.insert(frame.rate);
        frames.push((pos, round_units(units)));
        covered += frame.len.min(n - pos) as u64;
        units += f64::from(frame.samples * INTERNAL_RATE) / f64::from(frame.rate);
        pos = pos.saturating_add(frame.len);
    }

    let nf = frames.len();
    // Données majoritairement non MPEG (WAV ou OGG renommé en .mp3, par exemple) : refus.
    let body = (n - start) as u64;
    if covered * 2 < body {
        return Err(Mp3MapError::NotMpeg {
            frames: nf,
            percent: 100 * covered / body.max(1),
        });
    }
    let Some(last) = nf.checked_sub(1) else {
        // Inatteignable : `resync` a validé la trame à `first_frame`.
        return Err(Mp3MapError::NoFrame);
    };
    // Durée moyenne d'une trame, puis correction du délai du décodeur (gapless), empirique
    // comme le mp3map-tool de référence : `x` demi-trames.
    let fu = units / nf as f64;
    let x = if nf > 100 { 34.0 } else { 22.0 };
    let total = (units - x * fu / 2.0).round_ties_even();
    let total_units = if total > 0.0 { total as u64 } else { 0 };

    let mut out = Vec::new();
    le32(&mut out, total_units)?;
    le32(&mut out, first_frame as u64)?;
    le32(&mut out, 0)?;
    // `total_units` tient sur 32 bits : au plus 48 695 entrées.
    let num_records = (total_units / u64::from(INTERNAL_RATE)) as u32;
    out.reserve(8 * num_records as usize);

    // Une entrée par seconde : première trame dont la position atteint k × 88200.
    let mut fi = 0;
    for k in 1..=u64::from(num_records) {
        let target = k * u64::from(INTERNAL_RATE);
        while fi < last && frames[fi].1 < target {
            fi += 1;
        }
        let (byte_off, upos) = frames[fi];
        le32(&mut out, byte_off as u64)?;
        le32(&mut out, upos)?;
    }

    Ok(Mp3Map {
        data: out,
        duration_s: total_units as f64 / f64::from(INTERNAL_RATE),
        num_records,
        frames: nf,
        samplerates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Trame MPEG1 Layer III 128 kbit/s 44,1 kHz (celle de `silent.mp3`) : 417 octets.
    fn silent_frame() -> Vec<u8> {
        let mut f = vec![0xFF, 0xFB, 0x90, 0x00];
        f.resize(417, 0);
        f
    }

    #[test]
    fn longueurs_de_trame_des_tables() {
        let len = |b1: u8, b2: u8| parse_frame_header(&[0xFF, b1, b2, 0], 0).map(|f| f.len);
        assert_eq!(len(0xFB, 0x90), Some(417)); // MPEG1 L3 128k 44,1k
        assert_eq!(len(0xFB, 0x92), Some(418)); // + padding
        assert_eq!(len(0xF3, 0x14), Some(24)); // MPEG2 L3 8k 24k : la plus courte
        assert_eq!(len(0xE5, 0x18), Some(144)); // MPEG2.5 L2 8k 8k
        assert_eq!(len(0xFF, 0xE4), Some(448)); // MPEG1 L1 448k 48k : (12×448000/48000)×4
        assert_eq!(len(0xFD, 0xE8), Some(1728)); // MPEG1 L2 384k 32k : la plus longue
                                                 // Réservés : version 1, layer 0, débit 0 ou 15, fréquence 3.
        assert_eq!(len(0xEB, 0x90), None);
        assert_eq!(len(0xF9, 0x90), None);
        assert_eq!(len(0xFB, 0x00), None);
        assert_eq!(len(0xFB, 0xF0), None);
        assert_eq!(len(0xFB, 0x9C), None);
    }

    #[test]
    fn silent_mp3_du_convertisseur() {
        let data = silent_frame().repeat(24);
        let map = build(&data).unwrap();
        assert_eq!(map.frames, 24);
        assert_eq!(map.num_records, 0);
        assert_eq!(map.samplerates, BTreeSet::from([44100]));
        // 24 × 2304 − 11 × 2304 unités.
        assert_eq!(
            map.data,
            [&(13u32 * 2304).to_le_bytes()[..], &[0; 8]].concat()
        );
    }

    #[test]
    fn entrees_vides_ou_sans_trame() {
        assert_eq!(build(&[]), Err(Mp3MapError::NoFrame));
        assert_eq!(build(b"ID3"), Err(Mp3MapError::NoFrame));
        assert_eq!(build(&[0xFF; 4096]), Err(Mp3MapError::NoFrame));
        assert_eq!(build(&[0; 4096]), Err(Mp3MapError::NoFrame));
    }

    #[test]
    fn id3_de_taille_demesuree() {
        // Taille syncsafe maximale (≈ 256 Mio) et pied de tag : position au-delà des données.
        let mut data = b"ID3\x04\x00\x10\x7F\x7F\x7F\x7F".to_vec();
        data.extend(silent_frame().repeat(4));
        assert_eq!(build(&data), Err(Mp3MapError::NoFrame));
        // En-tête ID3 coupé (moins de 10 octets) : pas de tag, la trame qui suit n'est pas vue
        // comme un tag.
        assert_eq!(build(b"ID3\x04\x00\x00\x00"), Err(Mp3MapError::NoFrame));
    }

    #[test]
    fn trame_annoncee_au_dela_de_la_fin() {
        // Une trame valide dont la longueur dépasse les données : acceptée (fin des données),
        // couverture limitée à ce qui existe.
        let data = vec![0xFF, 0xFD, 0xE8, 0x00, 0, 0, 0, 0];
        let map = build(&data).unwrap();
        assert_eq!(map.frames, 1);
        assert_eq!(&map.data[4..8], &0u32.to_le_bytes());
    }

    #[test]
    fn donnees_pseudo_aleatoires_sans_panique() {
        // Générateur congruentiel : déterministe, sans dépendance.
        let mut state = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u8
        };
        for len in [0usize, 1, 3, 4, 9, 10, 11, 100, 1000, 5000] {
            for _ in 0..20 {
                let mut data: Vec<u8> = (0..len).map(|_| next()).collect();
                // Beaucoup d'octets de synchronisation pour exercer `resync` et `chain_ok`.
                for b in data.iter_mut().step_by(3) {
                    *b |= 0xE0;
                }
                let _ = build(&data);
            }
        }
    }

    #[test]
    fn messages_identiques_a_python() {
        assert_eq!(
            Mp3MapError::NoFrame.to_string(),
            "aucun frame MPEG audio valide"
        );
        assert_eq!(
            Mp3MapError::NotMpeg {
                frames: 3,
                percent: 12
            }
            .to_string(),
            "donnees non MPEG (3 frames couvrant 12% du fichier)"
        );
        assert_eq!(Mp3MapError::Overflow.to_string(), "int too big to convert");
    }
}
