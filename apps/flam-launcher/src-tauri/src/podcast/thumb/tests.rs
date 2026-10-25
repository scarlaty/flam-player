//! Aperçu des pochettes (#504) : vignette bornée, pochette absente ou altérée, aucune URL.

use std::path::PathBuf;
use std::sync::Arc;

use super::*;
use crate::podcast::mp3cache::tests::{episode, TestLocks};
use crate::podcast::mp3cache::Store;

mod demand;

/// PNG `w` × `h` uni.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_pixel(w, h, image::Rgb([200, 30, 30]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

/// Décodage dans le processus de test (la production passe par le processus enfant isolé).
fn in_process(b: &[u8], kind: ArtworkKind) -> Result<Artwork, ArtworkError> {
    Artwork::convert(b, kind)
}

fn decode(url: &str) -> (thumbnail::RgbaImage, usize) {
    use base64::Engine;
    let b64 = url
        .strip_prefix("data:image/png;base64,")
        .expect("URL data PNG");
    let png = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .unwrap();
    let len = png.len();
    (thumbnail::decode_png(&png).unwrap(), len)
}

#[test]
fn vignette_png_bornee_en_taille() {
    let url = thumbnail_data_url(&png(1200, 900), in_process).expect("vignette");
    let (img, len) = decode(&url);
    assert!(img.width <= THUMB_DIM && img.height <= THUMB_DIM);
    assert!(img.width == THUMB_DIM || img.height == THUMB_DIM);
    assert!(len <= MAX_THUMB_BYTES, "{len}");
}

#[test]
fn image_illisible_ou_vide_donne_pas_d_apercu() {
    assert_eq!(thumbnail_data_url(b"pas une image", in_process), None);
    assert_eq!(thumbnail_data_url(b"", in_process), None);
}

#[test]
fn png_invalide_du_convertisseur_refuse() {
    // Un PNG illisible rendu par le convertisseur ne donne pas d'aperçu.
    let convert = |_: &[u8], _| {
        Ok(Artwork {
            width: 1,
            height: 1,
            png: b"pas un png".to_vec(),
        })
    };
    assert_eq!(thumbnail_data_url(b"x", convert), None);
}

struct Fixture {
    dir: tempfile::TempDir,
    locks: Arc<TestLocks>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            locks: Arc::new(TestLocks::default()),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("cache")
    }

    fn access(&self) -> CacheAccess {
        CacheAccess::new(self.locks.clone(), self.root(), ["s1".to_string()])
    }

    async fn store(&self, ep: &PodcastEpisode, image: Option<&[u8]>) {
        let work = self.dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let mp3 = work.join(format!("{}.mp3", ep.guid));
        std::fs::write(&mp3, b"mp3").unwrap();
        let img = image.map(|b| {
            let p = work.join(format!("{}.img", ep.guid));
            std::fs::write(&p, b).unwrap();
            p
        });
        let n = self
            .access()
            .store(
                vec![Store {
                    key: EntryKey::of("s1", ep),
                    mp3,
                    image: img,
                    image_only: false,
                }],
                100,
            )
            .await;
        assert_eq!(n, 1);
    }
}

#[tokio::test]
async fn pochette_en_cache_donne_une_vignette() {
    let f = Fixture::new();
    let ep = episode("g1");
    f.store(&ep, Some(&png(640, 480))).await;
    let bytes = f
        .access()
        .cached_image(&EntryKey::of("s1", &ep), MAX_INPUT_BYTES as u64)
        .await
        .expect("pochette en cache");
    let url = thumbnail_data_url(&bytes, in_process).expect("vignette");
    assert!(url.starts_with("data:image/png;base64,"));
    assert!(!url.contains("secret"));
}

#[tokio::test]
async fn sans_pochette_ou_sans_entree_pas_d_apercu() {
    let f = Fixture::new();
    let sans = episode("g2");
    f.store(&sans, None).await;
    let key = EntryKey::of("s1", &sans);
    assert!(f.access().cached_image(&key, 1 << 20).await.is_none());
    // Épisode jamais mis en cache.
    let jamais = EntryKey::of("s1", &episode("g3"));
    assert!(f.access().cached_image(&jamais, 1 << 20).await.is_none());
    // Source inconnue de l'accès.
    let autre = EntryKey::of("s9", &sans);
    assert!(f.access().cached_image(&autre, 1 << 20).await.is_none());
}

#[tokio::test]
async fn pochette_alteree_trop_grosse_ou_perimee_pas_d_apercu() {
    let f = Fixture::new();
    let ep = episode("g4");
    let image = png(64, 64);
    f.store(&ep, Some(&image)).await;
    let key = EntryKey::of("s1", &ep);
    // Au-delà de la borne.
    assert!(f
        .access()
        .cached_image(&key, image.len() as u64 - 1)
        .await
        .is_none());
    // Adresse de pochette changée dans le flux : empreinte différente.
    let mut changed = ep.clone();
    changed.image = Some("https://img.example.test/autre.png".into());
    assert!(f
        .access()
        .cached_image(&EntryKey::of("s1", &changed), 1 << 20)
        .await
        .is_none());
    // Fichier altéré sur le disque (même taille, autre contenu).
    let path = crate::podcast::source::source_dir(&f.root(), "s1")
        .unwrap()
        .join("mp3")
        .join(format!("{}.img", key.episode_id));
    let mut bad = image.clone();
    bad[10] ^= 0xff;
    std::fs::write(path, bad).unwrap();
    assert!(f.access().cached_image(&key, 1 << 20).await.is_none());
}
