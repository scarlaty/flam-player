//! Entrées hostiles pour `audio` : étiquettes aux tailles mensongères, trames tronquées, en-têtes
//! Xing / VBRI / LAME coupés, octets aléatoires. Aucune ne doit paniquer ni durer.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use studio2lunii::audio::{
    mp3_tag_cleanup, probe_mp3, tags_removal_required, transcoding_required, AudioError,
    FFMPEG_TRANSCODE_ARGS, TARGET_CHANNELS, TARGET_MP3_VBR_QUALITY, TARGET_SAMPLE_RATE_HZ,
};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/data/audio")
            .join(format!("{name}.mp3")),
    )
    .unwrap()
}

#[test]
fn id3v2_larger_than_the_file_is_an_error_not_a_panic() {
    let mut v = b"ID3\x03\x00\x00\x7f\x7f\x7f\x7f".to_vec(); // ~256 Mio annoncés
    v.extend(fixture("mono_44100_cbr"));
    assert!(matches!(
        mp3_tag_cleanup(&v),
        Err(AudioError::TruncatedTag { .. })
    ));
    assert!(tags_removal_required(&v));
    assert!(transcoding_required("a.mp3", &v));
}

#[test]
fn invalid_id3v2_headers_are_not_tags() {
    // Octets de taille non synchsafe, version inconnue, en-tête de moins de 10 octets.
    for head in [
        &b"ID3\x03\x00\x00\xff\xff\xff\xff"[..],
        &b"ID3\x09\x00\x00\x00\x00\x00\x10"[..],
        &b"ID3\x03\x00"[..],
    ] {
        let mut v = head.to_vec();
        v.extend(fixture("mono_44100_cbr"));
        assert!(!tags_removal_required(&v));
        assert_eq!(mp3_tag_cleanup(&v).unwrap(), v);
    }
}

#[test]
fn ape_footer_claiming_more_than_the_file_is_ignored() {
    let mut v = fixture("mono_44100_cbr");
    let mut foot = b"APETAGEX".to_vec();
    foot.extend_from_slice(&2000u32.to_le_bytes());
    foot.extend_from_slice(&u32::MAX.to_le_bytes()); // taille énorme
    foot.extend_from_slice(&1u32.to_le_bytes());
    foot.extend_from_slice(&0x8000_0000u32.to_le_bytes());
    foot.extend_from_slice(&[0; 8]);
    v.extend(foot);
    assert!(mp3_tag_cleanup(&v).is_ok());
}

#[test]
fn stacked_id3v2_bomb_is_bounded() {
    // 10 000 étiquettes ID3v2 vides empilées : au plus 16 sont retirées, sans boucle longue.
    let v: Vec<u8> = b"ID3\x04\x00\x00\x00\x00\x00\x00".repeat(10_000);
    let t = Instant::now();
    let _ = mp3_tag_cleanup(&v);
    let _ = transcoding_required("a.mp3", &v);
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn sync_bomb_is_bounded() {
    // 8 Mio de 0xFF : des millions de synchronisations candidates, 1 Mio et 1499 essais au plus.
    let v = vec![0xFFu8; 8 * 1024 * 1024];
    let t = Instant::now();
    assert!(probe_mp3(&v).is_err());
    assert!(transcoding_required("a.mp3", &v));
    assert!(t.elapsed() < Duration::from_secs(5));
}

#[test]
fn truncated_and_corrupted_files_never_panic() {
    for name in [
        "xing_vbr_mono",
        "lame_abr_mono",
        "vbri_mono",
        "tag_with_xing",
        "tag_ape_and_v1",
    ] {
        let full = fixture(name);
        for cut in 0..full.len().min(700) {
            let _ = transcoding_required("a.mp3", &full[..cut]);
            let _ = tags_removal_required(&full[..cut]);
            let _ = mp3_tag_cleanup(&full[..cut]);
        }
        for seed in 0..300usize {
            let mut b = full.clone();
            for k in 0..4 {
                let at = (seed * 7919 + k * 104_729) % b.len();
                b[at] ^= 1 << ((seed + k) % 8);
            }
            let _ = transcoding_required("a.mp3", &b);
            let _ = mp3_tag_cleanup(&b);
        }
    }
}

#[test]
fn random_bytes_are_never_kept() {
    let mut x = 0x1234_5678u32;
    for len in [0usize, 1, 3, 4, 10, 417, 5000] {
        let v: Vec<u8> = (0..len)
            .map(|_| {
                x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (x >> 24) as u8
            })
            .collect();
        assert!(transcoding_required("a.mp3", &v), "{len} octets");
    }
}

#[test]
fn trailing_garbage_or_a_foreign_frame_forces_transcoding() {
    let mut v = fixture("mono_44100_cbr");
    assert!(!transcoding_required("a.mp3", &v));
    v.extend_from_slice(b"not an mp3 frame at all");
    assert!(transcoding_required("a.mp3", &v));
    // Une trame stéréo au milieu d'un flux mono.
    let mut w = fixture("mono_44100_cbr");
    w[182 * 5 + 3] &= 0x3F; // mode de canal 0 (stéréo) sur la 6e trame (182 octets chacune)
    assert!(transcoding_required("a.mp3", &w));
}

#[test]
fn transcoding_target_is_documented_and_consistent() {
    assert_eq!(TARGET_SAMPLE_RATE_HZ, 44_100);
    assert_eq!(TARGET_CHANNELS, 1);
    assert_eq!(TARGET_MP3_VBR_QUALITY, 5);
    let args = FFMPEG_TRANSCODE_ARGS;
    let value_of = |flag: &str| {
        let i = args.iter().position(|a| *a == flag).unwrap();
        args[i + 1]
    };
    assert_eq!(value_of("-ar"), TARGET_SAMPLE_RATE_HZ.to_string());
    assert_eq!(value_of("-ac"), TARGET_CHANNELS.to_string());
    assert_eq!(value_of("-aq"), TARGET_MP3_VBR_QUALITY.to_string());
    assert_eq!(value_of("-codec:a"), "libmp3lame");
    assert_eq!(value_of("-write_xing"), "0");
    assert_eq!(value_of("-id3v2_version"), "0");
    assert_eq!(value_of("-map_metadata"), "-1");
    assert!(
        !args.contains(&"-b:a"),
        "Lunii.QT interdit le débit constant"
    );
}
