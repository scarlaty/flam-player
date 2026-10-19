//! Tests des titres et vignettes de « Ma Flam » (#276). Clés inventées (aucune vraie clé),
//! faux appareils dans des dossiers temporaires, dispositions 1.x et 2.x.

use std::cell::Cell;

use sha2::{Digest, Sha256};

use super::*;
use crate::device::carrier::tests::{
    assert_no_leak, encrypted_info, fake_store, foreign_bt, foreign_key_file, real_mdf, real_story,
    test_bt, test_key_file, write,
};
use crate::device::carrier::{CarrierKeys, WARNING_VERSION};
use crate::device::crypto::tests::{TEST_IV, TEST_KEY};
use crate::device::crypto::{encrypt_zero_padded, MDF_LEN, SERIAL_FIELD_LEN};

const A: &str = "a701e27e-8f3a-248b-72bd-ff65fa5d440e";
const B: &str = "22222222-2222-4222-8222-222222222222";
const H: &str = "33333333-3333-4333-8333-333333333333";

/// `.mdf` 2.x synthétique (clé et IV de test inventés).
fn mdf_v2() -> Vec<u8> {
    let mut m = vec![0u8; 2];
    let mut text = b"main: 2.4.2-test\ncomm: x".to_vec();
    text.resize(48, 0);
    m.extend(text);
    let mut serial = b"00123456789abc".to_vec();
    serial.resize(SERIAL_FIELD_LEN, 0);
    m.extend(serial);
    m.extend([0u8; 4]);
    m.extend(TEST_KEY);
    m.extend(TEST_IV);
    m.extend([0u8; 16]);
    assert_eq!(m.len(), MDF_LEN);
    m
}

fn info_v2(title: &str) -> Vec<u8> {
    let plain = format!("{title}\nSous-titre\n000000\n{title}\n");
    encrypt_zero_padded(&TEST_KEY, &TEST_IV, plain.as_bytes())
}

/// Flam 2.x : `A` visible (titre accentué), `H` masquée, `B` sans `info`.
fn device_v2() -> tempfile::TempDir {
    let dev = tempfile::tempdir().unwrap();
    let root = dev.path();
    write(&root.join(".mdf"), &mdf_v2());
    write(
        &root.join(format!("str/{A}/info")),
        &info_v2("Capi et la rivière"),
    );
    write(
        &root.join(format!("str.hidden/{H}/info")),
        &info_v2("Histoire masquée"),
    );
    std::fs::create_dir_all(root.join(format!("str/{B}"))).unwrap();
    dev
}

/// Flam 1.x de la disposition réelle (en-tête `01 00`, version nue, histoires sans `bt`).
fn device_v1() -> tempfile::TempDir {
    let dev = tempfile::tempdir().unwrap();
    let root = dev.path();
    write(&root.join(".mdf"), &real_mdf());
    real_story(
        root,
        "str",
        A,
        &test_bt(),
        &test_key_file(),
        "Capi et la rivière",
    );
    real_story(
        root,
        "str",
        B,
        &foreign_bt(),
        &foreign_key_file(),
        "Autre compte",
    );
    dev
}

fn consent() -> CarrierConsent {
    CarrierConsent {
        text_version: WARNING_VERSION,
        accepted_at: "2026-10-19T00:00:00Z".into(),
    }
}

fn never() -> bool {
    false
}

fn titles_of(root: &Path, keys: Option<&DeviceKeys>, uuids: &[&str]) -> Vec<Option<String>> {
    let uuids: Vec<String> = uuids.iter().map(|u| u.to_string()).collect();
    read_titles(root, &uuids, keys, &never)
        .titles
        .into_iter()
        .map(|t| t.title)
        .collect()
}

/// Empreinte de toute l'arborescence : chemins relatifs, types et octets.
fn fingerprint(root: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = fs::read_dir(dir).unwrap().map(|e| e.unwrap()).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            if path.is_dir() {
                out.push(format!("D {rel}"));
                walk(&path, root, out);
            } else {
                let hash = Sha256::digest(fs::read(&path).unwrap());
                out.push(format!("F {rel} {hash:x}"));
            }
        }
    }
    let mut lines = Vec::new();
    walk(root, root, &mut lines);
    lines
}

/// LIF valide de `w` × `h` (dégradé).
fn lif(w: u32, h: u32) -> Vec<u8> {
    let rgba: Vec<u8> = (0..w * h)
        .flat_map(|i| [(i % 251) as u8, (i % 13) as u8 * 19, 200, 255])
        .collect();
    telmi2flam::lif::encode(&rgba, w, h).unwrap()
}

fn put_thumb(root: &Path, sub: &str, uuid: &str, data: &[u8]) {
    write(&root.join(sub).join(uuid).join("img/thumbnail.lif"), data);
}

// --- Titres ---------------------------------------------------------------------------------

#[test]
fn titre_2x_dechiffre_avec_les_cles_de_mdf() {
    let dev = device_v2();
    let store_dir = tempfile::tempdir().unwrap();
    let keys = title_keys(dev.path(), None, &fake_store(store_dir.path())).unwrap();
    assert_eq!(
        titles_of(dev.path(), Some(&keys), &[A, H, B]),
        vec![
            Some("Capi et la rivière".to_string()),
            Some("Histoire masquée".to_string()),
            None,
        ]
    );
}

#[test]
fn titre_1x_avec_la_cle_carrier_conservee() {
    let dev = device_v1();
    let store_dir = tempfile::tempdir().unwrap();
    let store = fake_store(store_dir.path());
    let id = DeviceId::from_mdf(&real_mdf()).unwrap();
    store
        .save(
            &id,
            &CarrierKeys::from_parts(&test_bt(), &test_key_file()).unwrap(),
        )
        .unwrap();
    let keys = title_keys(dev.path(), Some(&consent()), &store).unwrap();
    // L'histoire d'un autre compte ne se déchiffre pas : repli, pas d'erreur.
    assert_eq!(
        titles_of(dev.path(), Some(&keys), &[A, B]),
        vec![Some("Capi et la rivière".to_string()), None]
    );
}

#[test]
fn titre_1x_sans_cle_conservee_ou_sans_consentement_se_replie() {
    let dev = device_v1();
    let store_dir = tempfile::tempdir().unwrap();
    let store = fake_store(store_dir.path());
    // Aucune clé conservée : rien n'est demandé, aucun titre.
    assert!(title_keys(dev.path(), Some(&consent()), &store).is_none());
    let id = DeviceId::from_mdf(&real_mdf()).unwrap();
    store
        .save(
            &id,
            &CarrierKeys::from_parts(&test_bt(), &test_key_file()).unwrap(),
        )
        .unwrap();
    // Clé conservée, mais consentement absent ou périmé.
    assert!(title_keys(dev.path(), None, &store).is_none());
    let stale = CarrierConsent {
        text_version: WARNING_VERSION + 1,
        accepted_at: "2026-01-01T00:00:00Z".into(),
    };
    assert!(title_keys(dev.path(), Some(&stale), &store).is_none());
    // Clé conservée pour une autre Flam : ignorée.
    let mut other_mdf = real_mdf();
    other_mdf[50..64].copy_from_slice(b"00aaaaaaaaaaaa");
    write(&dev.path().join(".mdf"), &other_mdf);
    assert!(title_keys(dev.path(), Some(&consent()), &store).is_none());
    // Sans clé, aucun `info` n'est lu : les titres sont tous absents.
    assert_eq!(titles_of(dev.path(), None, &[A, B]), vec![None, None]);
}

#[test]
fn cle_conservee_illisible_ou_flam_inconnue_se_replient() {
    let dev = device_v1();
    let store_dir = tempfile::tempdir().unwrap();
    let store = fake_store(store_dir.path());
    let id = DeviceId::from_mdf(&real_mdf()).unwrap();
    store
        .save(
            &id,
            &CarrierKeys::from_parts(&test_bt(), &test_key_file()).unwrap(),
        )
        .unwrap();
    // Fichier conservé abîmé.
    for entry in fs::read_dir(store_dir.path()).unwrap() {
        fs::write(entry.unwrap().path(), b"abime").unwrap();
    }
    assert!(title_keys(dev.path(), Some(&consent()), &store).is_none());
    // `.mdf` absent, tronqué, firmware 3.x ou illisible : aucune clé.
    let mut v3 = mdf_v2();
    v3[2..18].copy_from_slice(b"main: 3.0.0-test");
    for mdf in [
        None,
        Some(mdf_v2()[..60].to_vec()),
        Some(v3),
        Some(vec![0u8; MDF_LEN]),
    ] {
        let dev = device_v2();
        match mdf {
            Some(bytes) => write(&dev.path().join(".mdf"), &bytes),
            None => fs::remove_file(dev.path().join(".mdf")).unwrap(),
        }
        assert!(title_keys(dev.path(), None, &store).is_none());
    }
}

#[test]
fn info_illisible_donne_un_repli() {
    let dev = device_v2();
    let root = dev.path();
    let store_dir = tempfile::tempdir().unwrap();
    let keys = title_keys(root, None, &fake_store(store_dir.path())).unwrap();
    let cases: Vec<(&str, Vec<u8>)> = vec![
        // Chiffré avec une autre clé.
        (
            "44444444-4444-4444-8444-444444444441",
            encrypted_info(&foreign_bt(), b"Titre\nSous-titre\n"),
        ),
        // Bruit.
        (
            "44444444-4444-4444-8444-444444444442",
            (0..64u8).map(|i| i.wrapping_mul(37)).collect(),
        ),
        // Trop court pour un bloc.
        ("44444444-4444-4444-8444-444444444443", b"court".to_vec()),
        // Vide.
        ("44444444-4444-4444-8444-444444444444", Vec::new()),
        // Clair sans aucune ligne non vide.
        (
            "44444444-4444-4444-8444-444444444445",
            encrypt_zero_padded(&TEST_KEY, &TEST_IV, b"  \n \n   \n\n"),
        ),
    ];
    for (uuid, data) in &cases {
        write(&root.join(format!("str/{uuid}/info")), data);
    }
    // `info` qui est un dossier : repli aussi.
    fs::create_dir_all(root.join("str/44444444-4444-4444-8444-444444444446/info")).unwrap();
    let mut uuids: Vec<&str> = cases.iter().map(|(u, _)| *u).collect();
    uuids.push("44444444-4444-4444-8444-444444444446");
    assert!(titles_of(root, Some(&keys), &uuids)
        .iter()
        .all(Option::is_none));
}

#[test]
fn titre_aux_espaces_multiples_resserre() {
    let dev = device_v2();
    let store_dir = tempfile::tempdir().unwrap();
    let keys = title_keys(dev.path(), None, &fake_store(store_dir.path())).unwrap();
    write(
        &dev.path().join(format!("str/{B}/info")),
        &info_v2("Tobie,            Livre 1,      La Vie"),
    );
    assert_eq!(
        titles_of(dev.path(), Some(&keys), &[B]),
        vec![Some("Tobie, Livre 1, La Vie".to_string())]
    );
}

#[test]
fn info_long_est_lu_en_partie_seulement() {
    let dev = device_v2();
    let root = dev.path();
    let store_dir = tempfile::tempdir().unwrap();
    let keys = title_keys(root, None, &fake_store(store_dir.path())).unwrap();
    let mut plain = b"Titre long\n".to_vec();
    plain.extend(std::iter::repeat_n(b'x', 64 * 1024));
    write(
        &root.join(format!("str/{B}/info")),
        &encrypt_zero_padded(&TEST_KEY, &TEST_IV, &plain),
    );
    assert_eq!(
        titles_of(root, Some(&keys), &[B]),
        vec![Some("Titre long".to_string())]
    );
}

#[test]
fn lot_borne_uuid_invalides_omis_et_arret() {
    let dev = device_v2();
    let root = dev.path();
    let uuids: Vec<String> = ["pas-un-uuid", "../../etc", A]
        .iter()
        .map(|s| s.to_string())
        .chain((0..MAX_BATCH + 5).map(|i| format!("00000000-0000-4000-8000-{i:012x}")))
        .collect();
    let report = read_titles(root, &uuids, None, &never);
    assert!(!report.interrupted);
    assert_eq!(
        report.titles.len(),
        MAX_BATCH - 2,
        "bornée au lot, invalides omis"
    );
    assert_eq!(report.titles[0].uuid, A);
    // Arrêt demandé après deux histoires : lot interrompu, partiel.
    let calls = Cell::new(0);
    let stop = || {
        calls.set(calls.get() + 1);
        calls.get() > 3
    };
    let report = read_titles(root, &uuids, None, &stop);
    assert!(report.interrupted);
    assert_eq!(report.titles.len(), 1);
}

#[test]
fn uuid_en_majuscules_normalise() {
    let dev = device_v2();
    let store_dir = tempfile::tempdir().unwrap();
    let keys = title_keys(dev.path(), None, &fake_store(store_dir.path())).unwrap();
    let report = read_titles(dev.path(), &[A.to_uppercase()], Some(&keys), &never);
    assert_eq!(report.titles[0].uuid, A);
    assert_eq!(
        report.titles[0].title.as_deref(),
        Some("Capi et la rivière")
    );
}

// --- Vignettes ------------------------------------------------------------------------------

#[test]
fn vignette_valide_decodee_en_png_et_mise_en_cache() {
    let dev = device_v2();
    let cache = tempfile::tempdir().unwrap();
    // Dimensions observées sur une Flam : 104 × 136 (officielles), 128 × 96 (converties).
    put_thumb(dev.path(), "str", A, &lif(104, 136));
    put_thumb(dev.path(), "str.hidden", H, &lif(128, 96));
    let url = story_thumbnail(dev.path(), A, cache.path()).unwrap();
    assert!(url.starts_with("data:image/png;base64,"));
    assert!(story_thumbnail(dev.path(), H, cache.path()).is_some());
    let cached: Vec<_> = fs::read_dir(cache.path().join("thumbs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(cached.len(), 2);
    // Nom du cache : empreinte seule (ni UUID ni chemin).
    for name in &cached {
        assert!(
            !name.contains("a701e27e") && name.ends_with(".png"),
            "{name}"
        );
    }
    // Second appel : servi par le cache, même résultat.
    assert_eq!(story_thumbnail(dev.path(), A, cache.path()).unwrap(), url);
    // PNG réduit à 96 px au plus.
    let png = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        url.trim_start_matches("data:image/png;base64,"),
    )
    .unwrap();
    let image = crate::thumbnail::decode_png(&png).unwrap();
    assert!(image.width <= 96 && image.height <= 96);
    assert_eq!(image.height, 96);
}

#[test]
fn vignette_modifiee_change_de_cle_de_cache() {
    let dev = device_v2();
    let cache = tempfile::tempdir().unwrap();
    put_thumb(dev.path(), "str", A, &lif(20, 20));
    let first = story_thumbnail(dev.path(), A, cache.path()).unwrap();
    put_thumb(dev.path(), "str", A, &lif(30, 10));
    let second = story_thumbnail(dev.path(), A, cache.path()).unwrap();
    assert_ne!(first, second);
}

#[test]
fn vignette_absente_corrompue_ou_trop_grosse_donne_none() {
    let dev = device_v2();
    let root = dev.path();
    let cache = tempfile::tempdir().unwrap();
    // Absente (dossier sans img/), histoire absente, UUID invalide.
    assert!(story_thumbnail(root, B, cache.path()).is_none());
    assert!(story_thumbnail(root, "55555555-5555-4555-8555-555555555555", cache.path()).is_none());
    assert!(story_thumbnail(root, "../str", cache.path()).is_none());
    // Corrompue : signature, troncature, vide.
    for (uuid, data) in [
        (
            "66666666-6666-4666-8666-666666666661",
            b"PNG pas un lif".to_vec(),
        ),
        (
            "66666666-6666-4666-8666-666666666662",
            lif(40, 40)[..20].to_vec(),
        ),
        ("66666666-6666-4666-8666-666666666663", Vec::new()),
    ] {
        put_thumb(root, "str", uuid, &data);
        assert!(
            story_thumbnail(root, uuid, cache.path()).is_none(),
            "{uuid}"
        );
    }
    // Trop grosse : refusée sans être lue.
    let big = "66666666-6666-4666-8666-666666666664";
    let mut data = lif(8, 8);
    data.resize(MAX_LIF_BYTES as usize + 1, 0);
    put_thumb(root, "str", big, &data);
    assert!(story_thumbnail(root, big, cache.path()).is_none());
    // Un dossier à la place du fichier.
    fs::create_dir_all(root.join(format!("str/{A}/img/thumbnail.lif"))).unwrap();
    assert!(story_thumbnail(root, A, cache.path()).is_none());
    // Rien n'est mis en cache pour une vignette invalide.
    assert!(!cache.path().join("thumbs").exists());
}

// --- Lecture seule et secrets ----------------------------------------------------------------

#[test]
fn aucune_ecriture_sur_la_flam() {
    for dev in [device_v1(), device_v2()] {
        let root = dev.path();
        put_thumb(root, "str", A, &lif(104, 136));
        put_thumb(root, "str", B, b"corrompu");
        write(&root.join("usr/0/library.cache"), b"cache");
        write(&root.join("etc/library/list"), format!("{A}\n").as_bytes());
        let before = fingerprint(root);
        let store_dir = tempfile::tempdir().unwrap();
        let store = fake_store(store_dir.path());
        let id = DeviceId::from_mdf(&fs::read(root.join(".mdf")).unwrap()).unwrap();
        store
            .save(
                &id,
                &CarrierKeys::from_parts(&test_bt(), &test_key_file()).unwrap(),
            )
            .unwrap();
        let cache = tempfile::tempdir().unwrap();
        for _ in 0..2 {
            let keys = title_keys(root, Some(&consent()), &store);
            let _ = titles_of(root, keys.as_ref(), &[A, B, H]);
            for uuid in [A, B, H] {
                let _ = story_thumbnail(root, uuid, cache.path());
            }
        }
        assert_eq!(fingerprint(root), before);
    }
}

#[test]
fn aucune_cle_dans_les_reponses_ni_le_debug() {
    let dev = device_v1();
    let root = dev.path();
    put_thumb(root, "str", A, &lif(16, 16));
    let store_dir = tempfile::tempdir().unwrap();
    let store = fake_store(store_dir.path());
    let id = DeviceId::from_mdf(&real_mdf()).unwrap();
    store
        .save(
            &id,
            &CarrierKeys::from_parts(&test_bt(), &test_key_file()).unwrap(),
        )
        .unwrap();
    let keys = title_keys(root, Some(&consent()), &store).unwrap();
    let titles = read_titles(root, &[A.to_string(), B.to_string()], Some(&keys), &never);
    let cache = tempfile::tempdir().unwrap();
    let thumb = StoryThumbnail {
        thumb: story_thumbnail(root, A, cache.path()),
        busy: false,
    };
    let text = format!(
        "{} {} {keys:?} {titles:?}",
        serde_json::to_string(&titles).unwrap(),
        serde_json::to_string(&thumb).unwrap(),
    );
    assert_no_leak(&text, "réponses");
    // Champs exposés au front : rien d'autre.
    let value = serde_json::to_value(&titles).unwrap();
    let mut keys_json: Vec<_> = value["titles"][0].as_object().unwrap().keys().collect();
    keys_json.sort();
    assert_eq!(keys_json, ["title", "uuid"]);
    // Clés 2.x de `.mdf` non plus.
    let dev = device_v2();
    let keys = title_keys(dev.path(), None, &store).unwrap();
    let debug = format!("{keys:?}");
    for secret in [&TEST_KEY[..], &TEST_IV[..]] {
        let hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        assert!(!debug.contains(&hex));
    }
}
