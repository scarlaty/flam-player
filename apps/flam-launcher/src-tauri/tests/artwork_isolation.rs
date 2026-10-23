//! Décodage des pochettes dans le vrai binaire du launcher relancé avec `--decode-artwork`
//! (#263) : bout en bout du processus enfant, `main.rs` compris.

use std::process::Command;
use std::time::Duration;

use flam_launcher_lib::podcast::artwork::{
    convert_isolated_with, Artwork, ArtworkError, ArtworkKind, CHILD_TIMEOUT, DECODE_ARG,
};

const JPEG: &[u8] = include_bytes!("fixtures/progressif.jpg");

fn launcher() -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_flam-launcher"));
    cmd.arg(DECODE_ARG);
    cmd
}

#[test]
fn l_enfant_decode_comme_l_appelant() {
    let art = convert_isolated_with(launcher(), JPEG, ArtworkKind::Episode, None, CHILD_TIMEOUT)
        .expect("décodage par l'enfant");
    let direct = Artwork::convert(JPEG, ArtworkKind::Episode).unwrap();
    assert_eq!(art, direct);
    assert_eq!((art.width, art.height), (640, 480));
}

#[test]
fn l_enfant_transmet_les_refus() {
    let r = convert_isolated_with(
        launcher(),
        b"ceci n'est pas une image",
        ArtworkKind::Cover,
        None,
        Duration::from_secs(60),
    );
    assert!(matches!(r, Err(ArtworkError::UnsupportedFormat)), "{r:?}");
}
