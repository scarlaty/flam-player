//! Tests de la préparation Lunii : packs clairs construits à la main (format de la note,
//! § 5.2), clés de test inventées, faux système de fichiers pour l'injection de panne.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::*;
use crate::device::carrier::CarrierKeys;
use crate::device::crypto::tests::{test_keys, TEST_IV, TEST_KEY, TEST_SERIAL};
use crate::device::prepare::{cleanup_stale, STAGING_PREFIX};

const UUID_BYTES: [u8; 16] = [
    0x0a, 0x1b, 0x2c, 0x3d, 0x4e, 0x5f, 0x60, 0x71, 0x82, 0x93, 0xa4, 0xb5, 0xc6, 0xd7, 0xe8, 0xf9,
];
const UUID_STR: &str = "0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9";
const INFO_TEXT: &[u8] = b"Histoire inventee\nHistoire inventee\n0\nHistoire inventee\nNo authors.";

/// Nœud de `ni` (note, § 3.2).
#[derive(Clone, Copy)]
struct Node {
    image: i32,
    sound: i32,
    ok: (i32, i32, i32),
    home: (i32, i32, i32),
    controls: [u16; 5],
}

const NONE: (i32, i32, i32) = (-1, -1, -1);

fn ni_bytes(nodes: &[Node], images: u32, sounds: u32) -> Vec<u8> {
    let mut out = vec![0u8; 0x200];
    out[0..2].copy_from_slice(&1u16.to_le_bytes());
    out[2..4].copy_from_slice(&3u16.to_le_bytes());
    out[4..8].copy_from_slice(&0x200u32.to_le_bytes());
    out[8..12].copy_from_slice(&0x2Cu32.to_le_bytes());
    out[12..16].copy_from_slice(&(nodes.len() as u32).to_le_bytes());
    out[16..20].copy_from_slice(&images.to_le_bytes());
    out[20..24].copy_from_slice(&sounds.to_le_bytes());
    out[0x18] = 1;
    for n in nodes {
        for v in [
            n.image, n.sound, n.ok.0, n.ok.1, n.ok.2, n.home.0, n.home.1, n.home.2,
        ] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for c in n.controls {
            out.extend_from_slice(&c.to_le_bytes());
        }
        out.extend_from_slice(&[0, 0]);
    }
    out
}

fn li_bytes(entries: &[i32]) -> Vec<u8> {
    let mut out: Vec<u8> = entries.iter().flat_map(|v| v.to_le_bytes()).collect();
    if out.len() < 8 {
        out.resize(8, 0);
    }
    out
}

fn asset_index(names: &[&str]) -> Vec<u8> {
    names
        .iter()
        .flat_map(|n| [b"000\\".as_slice(), n.as_bytes()].concat())
        .collect()
}

/// Graphe inventé : 3 nœuds, 2 nœuds d'action (`[1, 2]` puis `[0]`), 2 images, 1 son.
fn nodes() -> Vec<Node> {
    vec![
        Node {
            image: 0,
            sound: 0,
            ok: (0, 2, 0),
            home: NONE,
            controls: [1, 1, 0, 0, 0],
        },
        Node {
            image: 1,
            sound: -1,
            ok: (2, 1, 0),
            home: (0, 2, 1),
            controls: [0, 1, 1, 1, 0],
        },
        Node {
            image: -1,
            sound: 0,
            ok: NONE,
            home: (2, 1, 0),
            controls: [0, 0, 1, 1, 1],
        },
    ]
}

fn image_a() -> Vec<u8> {
    (0..600u32).map(|i| (i * 3 % 251) as u8).collect()
}

fn image_b() -> Vec<u8> {
    vec![0x42; 100]
}

fn sound_c() -> Vec<u8> {
    (0..200_000u32).map(|i| (i * 7 % 253) as u8).collect()
}

type Entries = Vec<(String, Vec<u8>)>;

fn base_entries() -> Entries {
    let e = |n: &str, d: Vec<u8>| (n.to_string(), d);
    vec![
        e("uuid.bin", UUID_BYTES.to_vec()),
        e("_metadata.json", b"{\"title\":\"x\"}".to_vec()),
        e("_thumbnail.png", b"png".to_vec()),
        e("ni", ni_bytes(&nodes(), 2, 1)),
        e("nm", Vec::new()),
        e("li", li_bytes(&[1, 2, 0])),
        e("ri", asset_index(&["AAAAAAAA", "0000000B"])),
        e("si", asset_index(&["CCCCCCCC"])),
        e("info", INFO_TEXT.to_vec()),
        e("img/", Vec::new()),
        e("img/thumbnail.lif", vec![7u8; 700]),
        e("rf/000/AAAAAAAA", image_a()),
        e("rf/000/0000000b", image_b()),
        e("sf/000/CCCCCCCC", sound_c()),
    ]
}

fn set(entries: &mut Entries, name: &str, data: Vec<u8>) {
    let slot = entries.iter_mut().find(|(n, _)| n == name).unwrap();
    slot.1 = data;
}

fn without(entries: &Entries, name: &str) -> Entries {
    entries.iter().filter(|(n, _)| n != name).cloned().collect()
}

fn write_pk(path: &Path, entries: &Entries) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, data) in entries {
        if name.ends_with('/') {
            zip.add_directory(name.as_str(), options).unwrap();
        } else {
            zip.start_file(name.as_str(), options).unwrap();
            zip.write_all(data).unwrap();
        }
    }
    zip.finish().unwrap();
}

fn pk_with(dir: &Path, entries: &Entries) -> PathBuf {
    let path = dir.join(format!("{UUID_STR}-3{PACK_SUFFIX}"));
    write_pk(&path, entries);
    path
}

fn run(pk: &Path, work: &Path) -> Result<PreparedStory, PrepareError> {
    prepare(
        pk,
        &test_keys(),
        work,
        &Limits::default(),
        &AtomicBool::new(false),
    )
}

fn list_files(root: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(root, &p, out);
            } else {
                let rel = p.strip_prefix(root).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut out = Vec::new();
    if root.exists() {
        walk(root, root, &mut out);
    }
    out.sort();
    out
}

fn contents(root: &Path) -> BTreeMap<String, Vec<u8>> {
    list_files(root)
        .into_iter()
        .map(|f| {
            let data = std::fs::read(root.join(&f)).unwrap();
            (f, data)
        })
        .collect()
}

/// Aucun fichier, aucun chantier dans `work`.
fn assert_nothing_left(work: &Path) {
    assert!(list_files(work).is_empty(), "{:?}", list_files(work));
    if let Ok(entries) = std::fs::read_dir(work) {
        for e in entries {
            let name = e.unwrap().file_name();
            assert!(!name.to_string_lossy().starts_with(STAGING_PREFIX));
        }
    }
}

const EXPECTED: [&str; 11] = [
    "img/thumbnail.lif",
    "info",
    "key",
    "li",
    "ni",
    "nm",
    "rf/000/0000000B",
    "rf/000/AAAAAAAA",
    "ri",
    "sf/000/CCCCCCCC",
    "si",
];

#[test]
fn arborescence_produite_et_fichiers_dechiffrables() {
    let tmp = tempfile::tempdir().unwrap();
    let entries = base_entries();
    let pk = pk_with(tmp.path(), &entries);
    let work = tmp.path().join("travail");
    let prepared = run(&pk, &work).unwrap();
    assert_eq!(prepared.uuid, UUID_STR);
    assert_eq!(prepared.dir, work.join("str").join(UUID_STR));
    assert_eq!(prepared.files, EXPECTED);
    assert_eq!(list_files(&prepared.dir), EXPECTED);
    assert_eq!(
        list_files(&work).len(),
        EXPECTED.len(),
        "rien hors str/<uuid>"
    );

    let keys = test_keys();
    let read = |name: &str| std::fs::read(prepared.dir.join(name)).unwrap();
    let source = |name: &str| entries.iter().find(|(n, _)| n == name).unwrap().1.clone();
    // En clair, copiés tels quels.
    for name in ["ni", "nm", "info", "img/thumbnail.lif"] {
        assert_eq!(read(name), source(name), "{name}");
    }
    assert_eq!(read("key"), keys.key_file().to_vec());
    // Chiffrés : différents de la source, déchiffrables, bourrage nul sous 512 octets.
    for (name, plain) in [
        ("li", source("li")),
        ("ri", source("ri")),
        ("si", source("si")),
        ("rf/000/AAAAAAAA", image_a()),
        ("rf/000/0000000B", image_b()),
        ("sf/000/CCCCCCCC", sound_c()),
    ] {
        let data = read(name);
        let n = plain.len().min(16);
        assert_ne!(data[..n], plain[..n], "{name} chiffré");
        let clear = keys.decrypt_lunii_file(name, &data).unwrap();
        assert_eq!(&clear[..plain.len()], &plain[..], "{name}");
        assert!(clear[plain.len()..].iter().all(|&b| b == 0), "{name}");
        let len = if plain.len() < 512 {
            crate::device::crypto::zero_padded_len(plain.len())
        } else {
            plain.len()
        };
        assert_eq!(data.len(), len, "{name}");
        // Égal au chiffrement du fichier entier (lecture par blocs sans effet).
        assert_eq!(
            data,
            keys.encrypt_lunii_file(name, &plain).unwrap(),
            "{name}"
        );
    }
    // Au-delà de 512 octets : la suite est en clair.
    assert_eq!(read("rf/000/AAAAAAAA")[512..], image_a()[512..]);
    assert_eq!(read("sf/000/CCCCCCCC")[512..], sound_c()[512..]);
    let total: u64 = EXPECTED.iter().map(|f| read(f).len() as u64).sum();
    assert_eq!(prepared.total_bytes, total);
}

#[test]
fn info_au_format_de_la_note_et_sans_nm_ni_vignette() {
    let tmp = tempfile::tempdir().unwrap();
    let mut entries = without(&without(&base_entries(), "nm"), "img/thumbnail.lif");
    entries.retain(|(n, _)| n != "img/");
    let prepared = run(&pk_with(tmp.path(), &entries), &tmp.path().join("travail")).unwrap();
    assert!(!prepared
        .files
        .iter()
        .any(|f| f == "nm" || f.starts_with("img")));
    assert_eq!(list_files(&prepared.dir), prepared.files);
    let info = std::fs::read(prepared.dir.join("info")).unwrap();
    assert_eq!(info, INFO_TEXT);
    assert!(info.is_ascii() && !info.contains(&b'\r'));
}

#[test]
fn preparation_deterministe_et_remplacement() {
    let tmp = tempfile::tempdir().unwrap();
    let pk = pk_with(tmp.path(), &base_entries());
    let work = tmp.path().join("travail");
    let first = contents(&run(&pk, &work).unwrap().dir);
    std::fs::write(work.join("str").join(UUID_STR).join("ancien"), b"x").unwrap();
    let prepared = run(&pk, &work).unwrap();
    assert_eq!(contents(&prepared.dir), first);
    assert_eq!(cleanup_stale(&work).unwrap(), 0);
}

#[test]
fn firmware_1x_refuse_avant_toute_lecture_ou_ecriture() {
    let tmp = tempfile::tempdir().unwrap();
    let carrier = CarrierKeys::from_parts(&[0x33; 32], &[0x44; 32]).unwrap();
    let keys = DeviceKeys::with_carrier("1.15.14", &carrier).unwrap();
    let work = tmp.path().join("travail");
    let cancel = AtomicBool::new(false);
    let pk = pk_with(tmp.path(), &base_entries());
    let before = list_files(tmp.path());
    for pack in [pk.clone(), tmp.path().join("absent.lunii.pk")] {
        let err = prepare(&pack, &keys, &work, &Limits::default(), &cancel).unwrap_err();
        assert!(matches!(err, PrepareError::LuniiNeedsV2), "{err:?}");
        assert!(err.to_string().contains("2.x"), "{err}");
    }
    assert!(!work.exists());
    assert_eq!(list_files(tmp.path()), before);
}

#[test]
fn aucune_cle_dans_les_erreurs_ni_en_clair_dans_les_fichiers() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("travail");
    let prepared = run(&pk_with(tmp.path(), &base_entries()), &work).unwrap();
    let mut bad = base_entries();
    bad.push(("../evil".into(), b"x".to_vec()));
    let err = run(&pk_with(tmp.path(), &bad), &tmp.path().join("t2")).unwrap_err();
    let texts = [err.to_string(), format!("{err:?}"), format!("{prepared:?}")];
    let reversed_key = crate::device::crypto::reverse_words(&TEST_KEY);
    let reversed_iv = crate::device::crypto::reverse_words(&TEST_IV);
    let secrets: [&[u8]; 5] = [
        &TEST_KEY,
        &TEST_IV,
        &TEST_SERIAL,
        &reversed_key[..],
        &reversed_iv[..],
    ];
    for secret in secrets {
        let as_hex: String = secret.iter().map(|b| format!("{b:02x}")).collect();
        for text in &texts {
            assert!(!text.to_lowercase().contains(&as_hex), "{text}");
        }
        if secret.len() == 16 {
            for file in &prepared.files {
                let data = std::fs::read(prepared.dir.join(file)).unwrap();
                assert!(!data.windows(16).any(|w| w == secret), "{file}");
            }
        }
    }
}

fn expect_invalid(entries: &Entries, needle: &str) {
    let tmp = tempfile::tempdir().unwrap();
    let pk = pk_with(tmp.path(), entries);
    let work = tmp.path().join("travail");
    match run(&pk, &work) {
        Err(PrepareError::LuniiInvalid(m)) => assert!(m.contains(needle), "{needle} : {m}"),
        other => panic!("attendu LuniiInvalid({needle}), obtenu {other:?}"),
    }
    assert!(!work.exists(), "rien d'écrit ({needle})");
}

#[test]
fn chemins_hostiles_et_entrees_inattendues_refuses() {
    for bad in [
        "../evil",
        "/abs",
        "rf/000/../../x",
        "rf/000/CON",
        "img/a:b.lif",
        "fin.",
    ] {
        let mut entries = base_entries();
        entries.push((bad.into(), b"x".to_vec()));
        expect_invalid(&entries, "refusé");
    }
    for bad in [
        "key",
        "main.lua",
        "version",
        "bt",
        "li.plain",
        "rf/001/AAAAAAAA",
        "rf/000/sub/AAAAAAAA",
        "rf/000/AAAAAAA",
        "rf/000/AAAAAAAA.bmp",
        "rf/000/AAAA-AAA",
        "sf/000",
        "img/autre.lif",
    ] {
        let mut entries = base_entries();
        entries.push((bad.into(), b"x".to_vec()));
        expect_invalid(&entries, "inattendue");
    }
    // Doublons à la casse près (FAT ignore la casse).
    for dup in ["NI", "rf/000/aaaaaaaa", "RF/000/AAAAAAAA"] {
        let mut entries = base_entries();
        entries.push((dup.into(), b"x".to_vec()));
        expect_invalid(&entries, "double");
    }
}

#[test]
fn lien_symbolique_refuse() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("lien.lunii.pk");
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, data) in base_entries() {
        if !name.ends_with('/') {
            zip.start_file(name.as_str(), options).unwrap();
            zip.write_all(&data).unwrap();
        }
    }
    zip.add_symlink("img/lien.lif", "../../../etc", options)
        .unwrap();
    zip.finish().unwrap();
    let work = tmp.path().join("travail");
    match run(&path, &work) {
        Err(PrepareError::LuniiInvalid(m)) => assert!(m.contains("lien"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(!work.exists());
}

#[test]
fn fichier_obligatoire_ou_asset_absent() {
    for required in ["uuid.bin", "ni", "li", "ri", "si", "info"] {
        expect_invalid(
            &without(&base_entries(), required),
            &format!("{required} absent"),
        );
    }
    expect_invalid(
        &without(&base_entries(), "rf/000/AAAAAAAA"),
        "rf/000/AAAAAAAA absent",
    );
    expect_invalid(
        &without(&base_entries(), "sf/000/CCCCCCCC"),
        "sf/000/CCCCCCCC absent",
    );
    // Fichier présent mais non référencé par `ri`.
    let mut entries = base_entries();
    entries.push(("rf/000/DDDDDDDD".into(), vec![1; 10]));
    expect_invalid(&entries, "non référencé");
    for size in [0usize, 15, 17] {
        let mut entries = base_entries();
        set(&mut entries, "uuid.bin", vec![1; size]);
        expect_invalid(&entries, "uuid.bin");
    }
}

#[test]
fn ni_li_ri_si_incoherents_refuses() {
    let mut cases: Vec<(Entries, &str)> = Vec::new();
    let with = |name: &str, data: Vec<u8>| {
        let mut e = base_entries();
        set(&mut e, name, data);
        e
    };
    let node_with = |f: &dyn Fn(&mut Node)| {
        let mut n = nodes();
        f(&mut n[1]);
        ni_bytes(&n, 2, 1)
    };
    // En-tête.
    let mut ni = ni_bytes(&nodes(), 2, 1);
    ni[8] = 0x2D;
    cases.push((with("ni", ni), "en-tête"));
    cases.push((with("ni", vec![0; 0x100]), "en-tête incomplet"));
    let mut ni = ni_bytes(&nodes(), 2, 1);
    ni.pop();
    cases.push((with("ni", ni), "pour 3 nœuds"));
    cases.push((with("ni", ni_bytes(&[], 2, 1)), "0 nœuds"));
    cases.push((with("ni", ni_bytes(&nodes(), 3, 1)), "annoncés"));
    cases.push((with("ni", ni_bytes(&nodes(), 2, 0)), "annoncés"));
    // Indices de nœud.
    cases.push((with("ni", node_with(&|n| n.image = 2)), "image 2"));
    cases.push((with("ni", node_with(&|n| n.sound = -2)), "son -2"));
    cases.push((
        with("ni", node_with(&|n| n.ok = (2, 2, 0))),
        "transition OK",
    ));
    cases.push((
        with("ni", node_with(&|n| n.ok = (0, 2, 2))),
        "transition OK",
    ));
    // -1 est le tirage au hasard (#608) ; toute autre valeur négative reste refusée, et -1 ne
    // dispense pas de contrôler la position et le nombre d'options.
    cases.push((
        with("ni", node_with(&|n| n.ok = (0, 2, -2))),
        "transition OK",
    ));
    cases.push((
        with("ni", node_with(&|n| n.ok = (2, 2, -1))),
        "transition OK",
    ));
    cases.push((
        with("ni", node_with(&|n| n.home = (0, 0, -1))),
        "transition Maison",
    ));
    cases.push((
        with("ni", node_with(&|n| n.home = (-1, 2, -1))),
        "transition Maison",
    ));
    cases.push((
        with("ni", node_with(&|n| n.ok = (-1, 2, 0))),
        "transition OK",
    ));
    cases.push((
        with("ni", node_with(&|n| n.home = (0, 0, 0))),
        "transition Maison",
    ));
    cases.push((with("ni", node_with(&|n| n.controls[2] = 2)), "commandes"));
    // `li`.
    cases.push((with("li", li_bytes(&[1, 3, 0])), "nœud 3"));
    cases.push((with("li", li_bytes(&[1, -1, 0])), "nœud -1"));
    cases.push((with("li", vec![0; 13]), "multiple de 4"));
    // `ri`, `si`.
    cases.push((
        with("ri", asset_index(&["AAAAAAAA", "AAAAAAAA"])),
        "en double",
    ));
    cases.push((
        with("ri", asset_index(&["AAAAAAAA", "aaaaaaaa"])),
        "en double",
    ));
    cases.push((
        with("ri", b"000/AAAAAAAA000\\0000000B".to_vec()),
        "mal formée",
    ));
    cases.push((
        with("ri", b"000\\AAAA.AAA000\\0000000B".to_vec()),
        "mal formée",
    ));
    cases.push((with("si", b"000\\CCCCCCCC0".to_vec()), "multiple de 12"));
    // `nm`, `info`.
    cases.push((with("nm", vec![0]), "nm"));
    cases.push((with("info", b"Titre\r\nTitre".to_vec()), "info"));
    cases.push((with("info", "Titr\u{e9}\nx".as_bytes().to_vec()), "info"));
    cases.push((with("info", b"\nTitre".to_vec()), "info"));
    cases.push((with("info", Vec::new()), "info"));
    for (entries, needle) in cases {
        expect_invalid(&entries, needle);
    }
}

#[test]
fn tirage_au_hasard_accepte() {
    // `optionIndex` à -1 (tirage au hasard de STUdio, #608) sur les transitions OK et maison :
    // `ni` copié tel quel, en clair.
    let tmp = tempfile::tempdir().unwrap();
    let mut n = nodes();
    n[0].ok = (0, 2, -1);
    n[1].home = (0, 2, -1);
    let ni = ni_bytes(&n, 2, 1);
    let mut entries = base_entries();
    set(&mut entries, "ni", ni.clone());
    let prepared = run(&pk_with(tmp.path(), &entries), &tmp.path().join("travail")).unwrap();
    assert_eq!(std::fs::read(prepared.dir.join("ni")).unwrap(), ni);
}

#[test]
fn histoire_sans_image_ni_son_et_cycles_acceptes() {
    // Un seul nœud qui revient sur lui-même ; `ri` et `si` vides (à confirmer sur appareil,
    // L12) : la préparation l'accepte.
    let tmp = tempfile::tempdir().unwrap();
    let node = Node {
        image: -1,
        sound: -1,
        ok: (0, 1, 0),
        home: NONE,
        controls: [0, 1, 0, 0, 0],
    };
    let mut entries: Entries = base_entries()
        .into_iter()
        .filter(|(n, _)| !n.starts_with("rf/") && !n.starts_with("sf/"))
        .collect();
    set(&mut entries, "ni", ni_bytes(&[node], 0, 0));
    set(&mut entries, "li", li_bytes(&[0]));
    set(&mut entries, "ri", Vec::new());
    set(&mut entries, "si", Vec::new());
    let prepared = run(&pk_with(tmp.path(), &entries), &tmp.path().join("travail")).unwrap();
    assert_eq!(std::fs::read(prepared.dir.join("ri")).unwrap(), b"");
    assert!(!prepared.files.iter().any(|f| f.starts_with("rf/")));
}

#[test]
fn bornes_respectees() {
    let tmp = tempfile::tempdir().unwrap();
    let pk = pk_with(tmp.path(), &base_entries());
    let work = tmp.path().join("travail");
    let cancel = AtomicBool::new(false);
    let declared: u64 = base_entries()
        .iter()
        .filter(|(n, _)| !n.starts_with('_') && n != "uuid.bin")
        .map(|(_, d)| d.len() as u64)
        .sum();
    // Tailles déclarées : refus avant tout chantier.
    let limits = Limits {
        max_total_bytes: declared - 1,
        ..Limits::default()
    };
    match prepare(&pk, &test_keys(), &work, &limits, &cancel) {
        Err(PrepareError::LuniiTooLarge(m)) => assert!(m.contains("déclarés"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(!work.exists());
    for limits in [
        Limits {
            max_entries: 5,
            ..Limits::default()
        },
        // Accepté d'après l'annuaire, dépassé à l'écriture (`key`, bourrage).
        Limits {
            max_total_bytes: declared + 1,
            ..Limits::default()
        },
        Limits {
            max_encrypted_bytes: 100,
            ..Limits::default()
        },
    ] {
        let err = prepare(&pk, &test_keys(), &work, &limits, &cancel).unwrap_err();
        assert!(matches!(err, PrepareError::LuniiTooLarge(_)), "{err:?}");
        assert_nothing_left(&work);
    }
    // `info` borné à part.
    let mut entries = base_entries();
    set(&mut entries, "info", vec![b'a'; 64 * 1024 + 1]);
    let pk = pk_with(tmp.path(), &entries);
    assert!(matches!(
        run(&pk, &work),
        Err(PrepareError::LuniiTooLarge(_))
    ));
    assert_nothing_left(&work);
}

#[test]
fn archive_corrompue_ou_tronquee() {
    let tmp = tempfile::tempdir().unwrap();
    let work = tmp.path().join("travail");
    let garbage = tmp.path().join("garbage.lunii.pk");
    std::fs::write(&garbage, b"pas un zip du tout").unwrap();
    assert!(matches!(
        run(&garbage, &work),
        Err(PrepareError::LuniiInvalid(_))
    ));
    let pk = pk_with(tmp.path(), &base_entries());
    let bytes = std::fs::read(&pk).unwrap();
    for cut in [bytes.len() / 2, bytes.len() - 10, 30] {
        let truncated = tmp.path().join("tronque.lunii.pk");
        std::fs::write(&truncated, &bytes[..cut]).unwrap();
        assert!(matches!(
            run(&truncated, &work),
            Err(PrepareError::LuniiInvalid(_))
        ));
        assert_nothing_left(&work);
    }
    // Son altéré après les 512 premiers octets (CRC faux) : détecté pendant la copie.
    let mut corrupt = bytes.clone();
    // Suite propre au son (pas de 7, l'image de test a un pas de 3).
    let marker: Vec<u8> = sound_c()[1000..1064].to_vec();
    let pos = corrupt.windows(64).position(|w| w == marker).unwrap();
    corrupt[pos + 50_000] ^= 0xff;
    let altered = tmp.path().join("altere.lunii.pk");
    std::fs::write(&altered, &corrupt).unwrap();
    assert!(matches!(
        run(&altered, &work),
        Err(PrepareError::LuniiInvalid(_))
    ));
    assert_nothing_left(&work);
}

#[test]
fn annulation_avant_le_debut() {
    let tmp = tempfile::tempdir().unwrap();
    let pk = pk_with(tmp.path(), &base_entries());
    let work = tmp.path().join("travail");
    let cancel = AtomicBool::new(true);
    let err = prepare(&pk, &test_keys(), &work, &Limits::default(), &cancel).unwrap_err();
    assert!(matches!(err, PrepareError::Cancelled));
    assert!(!work.exists());
}

/// Faux système de fichiers : panne ou annulation à la n-ième opération d'écriture.
struct FaultySink {
    ops: Cell<usize>,
    fail_at: usize,
    mode: Fault,
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Fault {
    Error,
    Cancel,
    None,
}

impl FaultySink {
    fn new(fail_at: usize, mode: Fault, cancel: Arc<AtomicBool>) -> Self {
        Self {
            ops: Cell::new(0),
            fail_at,
            mode,
            cancel,
        }
    }
    fn tick(&self) -> io::Result<()> {
        let n = self.ops.get();
        self.ops.set(n + 1);
        if n == self.fail_at {
            match self.mode {
                Fault::Error => return Err(io::Error::other("panne simulée")),
                Fault::Cancel => self.cancel.store(true, Ordering::Relaxed),
                Fault::None => {}
            }
        }
        Ok(())
    }
}

struct FaultyWriter<'a> {
    sink: &'a FaultySink,
    inner: File,
}

impl Write for FaultyWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.sink.tick()?;
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.sink.tick()?;
        self.inner.flush()
    }
}

impl Sink for FaultySink {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        RealSink.create_dir_all(path)
    }
    fn create(&self, path: &Path) -> io::Result<Box<dyn Write + '_>> {
        self.tick()?;
        Ok(Box::new(FaultyWriter {
            sink: self,
            inner: File::create_new(path)?,
        }))
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.tick()?;
        RealSink.rename(from, to)
    }
    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        RealSink.remove_dir_all(path)
    }
}

#[test]
fn injection_de_panne_et_d_annulation_a_chaque_etape() {
    let tmp = tempfile::tempdir().unwrap();
    let pk = pk_with(tmp.path(), &base_entries());
    let work = tmp.path().join("travail");
    let cancel = Arc::new(AtomicBool::new(false));
    let counter = FaultySink::new(usize::MAX, Fault::None, cancel.clone());
    let expected = contents(
        &prepare_with(
            &counter,
            &pk,
            &test_keys(),
            &work,
            &Limits::default(),
            &cancel,
        )
        .unwrap()
        .dir,
    );
    let total_ops = counter.ops.get();
    assert!(total_ops > 20, "{total_ops}");

    for mode in [Fault::Error, Fault::Cancel] {
        for fail_at in 0..total_ops {
            let _ = std::fs::remove_dir_all(&work);
            let cancel = Arc::new(AtomicBool::new(false));
            let sink = FaultySink::new(fail_at, mode, cancel.clone());
            let result = prepare_with(&sink, &pk, &test_keys(), &work, &Limits::default(), &cancel);
            // Les 2 dernières opérations (dossier `str`, renommage final) suivent le dernier
            // contrôle d'annulation.
            let tail = fail_at + 2 >= total_ops;
            let final_dir = work.join("str").join(UUID_STR);
            match result {
                Err(PrepareError::Io(_)) if mode == Fault::Error => {
                    assert!(!final_dir.exists(), "étape {fail_at}");
                }
                Err(PrepareError::Cancelled) if mode == Fault::Cancel && !tail => {
                    assert!(!final_dir.exists(), "étape {fail_at}");
                }
                Ok(p) if mode == Fault::Cancel && tail => {
                    assert_eq!(contents(&p.dir), expected, "étape {fail_at}");
                }
                other => panic!("{mode:?} {fail_at} : résultat inattendu {other:?}"),
            }
            for e in std::fs::read_dir(&work).into_iter().flatten() {
                let name = e.unwrap().file_name();
                assert!(
                    !name.to_string_lossy().starts_with(STAGING_PREFIX),
                    "chantier laissé ({mode:?}, étape {fail_at})"
                );
            }
        }
    }
}

/// Le `PreparedStory` passe tel quel par `transfer::send_with` sur un faux appareil 2.x.
#[test]
fn envoi_tel_quel_par_send_with() {
    use crate::device::crypto::FirmwareFamily;
    use crate::device::detect::MDF_FILE;
    use crate::device::guard::tests::mdf_v2_known;
    use crate::device::guard::WritePermit;
    use crate::device::index::{serialize, StoryId};
    use crate::device::transfer::{send_with, RealFs};

    let tmp = tempfile::tempdir().unwrap();
    let prepared = run(
        &pk_with(tmp.path(), &base_entries()),
        &tmp.path().join("travail"),
    )
    .unwrap();
    let root = tmp.path().join("flam");
    let mdf = mdf_v2_known();
    std::fs::create_dir_all(root.join("etc/library")).unwrap();
    std::fs::write(root.join(MDF_FILE), &mdf).unwrap();
    std::fs::write(root.join("etc/library/list"), serialize(&[])).unwrap();
    std::fs::write(root.join("etc/library/list.hidden"), b"").unwrap();
    let fs = RealFs {
        free_space: |_| Some(1 << 40),
        ..RealFs::default()
    };
    let report = send_with(
        &fs,
        &WritePermit::for_tests(FirmwareFamily::V2, &mdf),
        &root,
        &prepared,
        None,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.files, EXPECTED.len());
    let installed = root.join("str").join(UUID_STR);
    assert_eq!(contents(&installed), contents(&prepared.dir));
    let list = crate::device::index::read_index(&root, crate::device::index::IndexKind::Visible);
    assert_eq!(list.entries, vec![StoryId::parse(UUID_STR).unwrap()]);
}

#[test]
fn asset_de_512_octets_exactement_et_asset_vide() {
    let tmp = tempfile::tempdir().unwrap();
    let mut entries = base_entries();
    let exact: Vec<u8> = (0..512u32).map(|i| (i * 5 % 241) as u8).collect();
    set(&mut entries, "rf/000/0000000b", exact.clone());
    set(&mut entries, "sf/000/CCCCCCCC", Vec::new());
    let prepared = run(&pk_with(tmp.path(), &entries), &tmp.path().join("travail")).unwrap();
    let keys = test_keys();
    let image = std::fs::read(prepared.dir.join("rf/000/0000000B")).unwrap();
    assert_eq!(image.len(), 512);
    assert_ne!(image, exact);
    assert_eq!(
        keys.decrypt_lunii_file("rf/000/0000000B", &image).unwrap(),
        exact
    );
    assert_eq!(
        std::fs::read(prepared.dir.join("sf/000/CCCCCCCC")).unwrap(),
        b""
    );
}

#[test]
fn bornes_de_la_note_sur_les_index() {
    let ri = asset_index(&["AAAAAAAA"]).repeat(MAX_ASSETS + 1);
    assert!(matches!(
        asset_names(&ri, RI),
        Err(PrepareError::TooLarge(_))
    ));
    let li = vec![0u8; 4 * (MAX_LI_ENTRIES + 1)];
    assert!(matches!(check_li(&li, 1), Err(PrepareError::TooLarge(_))));
    let mut ni = ni_bytes(&nodes(), 2, 1);
    ni[12..16].copy_from_slice(&((MAX_NODES + 1) as u32).to_le_bytes());
    assert!(matches!(
        check_ni(&ni, &li_bytes(&[1, 2, 0]), 2, 1),
        Err(PrepareError::Invalid(m)) if m.contains("nœuds")
    ));
}

/// Marque l'entrée `name` comme chiffrée par mot de passe (bit 0 des drapeaux, en-tête local
/// et répertoire central) sans toucher aux données : `zip` n'écrit pas de ZipCrypto.
fn mark_password_protected(pk: &Path, name: &str) {
    let mut bytes = std::fs::read(pk).unwrap();
    let needle = name.as_bytes();
    let mut marked = 0;
    for p in 0..bytes.len().saturating_sub(needle.len()) {
        if &bytes[p..p + needle.len()] != needle {
            continue;
        }
        if p >= 30 && bytes[p - 30..p - 26] == [0x50, 0x4b, 0x03, 0x04] {
            bytes[p - 30 + 6] |= 1;
            marked += 1;
        } else if p >= 46 && bytes[p - 46..p - 42] == [0x50, 0x4b, 0x01, 0x02] {
            bytes[p - 46 + 8] |= 1;
            marked += 1;
        }
    }
    assert_eq!(marked, 2, "{name} : en-tête local et central");
    std::fs::write(pk, bytes).unwrap();
}

#[test]
fn entree_chiffree_par_mot_de_passe_refusee_avant_toute_ecriture() {
    // #540 : vignette et assets compris (avant, refusés seulement dans le chantier).
    for name in [
        "img/thumbnail.lif",
        "rf/000/AAAAAAAA",
        "sf/000/CCCCCCCC",
        "ni",
        "info",
        "uuid.bin",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let pk = pk_with(tmp.path(), &base_entries());
        mark_password_protected(&pk, name);
        let work = tmp.path().join("travail");
        match run(&pk, &work) {
            Err(PrepareError::LuniiInvalid(m)) => {
                assert!(m.contains(name) && m.contains("mot de passe"), "{m}")
            }
            other => panic!("{name} : attendu LuniiInvalid, obtenu {other:?}"),
        }
        assert!(!work.exists(), "rien d'écrit ({name})");
    }
    // Une entrée ignorée n'est jamais lue : son chiffrement ne gêne pas.
    let tmp = tempfile::tempdir().unwrap();
    let pk = pk_with(tmp.path(), &base_entries());
    mark_password_protected(&pk, "_thumbnail.png");
    run(&pk, &tmp.path().join("travail")).unwrap();
}

#[test]
fn en_tete_de_ni_controle_entierement() {
    let with_ni = |f: &dyn Fn(&mut Vec<u8>)| {
        let mut ni = ni_bytes(&nodes(), 2, 1);
        f(&mut ni);
        let mut e = base_entries();
        set(&mut e, "ni", ni);
        e
    };
    // En-tête documenté (note § 3.2, `studio2lunii::index::build_index`) : accepté, quelle
    // que soit la version du pack en 0x02.
    let tmp = tempfile::tempdir().unwrap();
    let entries = with_ni(&|ni| ni[2..4].copy_from_slice(&0xFFFFu16.to_le_bytes()));
    run(&pk_with(tmp.path(), &entries), &tmp.path().join("travail")).unwrap();
    // Version de format autre que 1.
    for v in [0u16, 2, 0x0100] {
        expect_invalid(
            &with_ni(&|ni| ni[0..2].copy_from_slice(&v.to_le_bytes())),
            "version de format",
        );
    }
    // Octet 0x18 autre que 1, octets non nuls de 0x19 à 0x1FF.
    for (at, value) in [(0x18, 0u8), (0x18, 2), (0x19, 1), (0x100, 0xAA), (0x1FF, 1)] {
        expect_invalid(&with_ni(&|ni| ni[at] = value), "octet 0x18");
    }
}

#[test]
fn li_vide_accepte_sans_transition_qui_le_reference() {
    // Comportement figé en attendant la mesure sur l'appareil (L12, #540) : un `li` vide passe
    // si toutes les transitions sont absentes, et est refusé dès qu'une le référence.
    let node = |ok| Node {
        image: 0,
        sound: 0,
        ok,
        home: NONE,
        controls: [1, 1, 0, 0, 0],
    };
    let entries = |ok| {
        let mut e: Entries = base_entries()
            .into_iter()
            .filter(|(n, _)| n != "rf/000/0000000b")
            .collect();
        set(&mut e, "ni", ni_bytes(&[node(ok)], 1, 1));
        set(&mut e, "li", Vec::new());
        set(&mut e, "ri", asset_index(&["AAAAAAAA"]));
        e
    };
    let tmp = tempfile::tempdir().unwrap();
    let prepared = run(
        &pk_with(tmp.path(), &entries(NONE)),
        &tmp.path().join("travail"),
    )
    .unwrap();
    assert!(prepared.files.iter().any(|f| f == "li"));
    expect_invalid(&entries((0, 1, 0)), "transition OK");
}
