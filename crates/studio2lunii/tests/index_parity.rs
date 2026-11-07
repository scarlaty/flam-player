//! Parité octet pour octet avec Lunii.QT (commit c8afe43) des index `ni`, `li`, `ri`, `si` et
//! des noms cibles des assets.
//!
//! Production des attendus (oracle hors dépôt) : un script Python 3.12 lancé en mode isolé
//! (`python -I`) importe `StudioStory` d'une copie non modifiée de `pkg/api/stories.py` de
//! Lunii.QT (ses imports sans rapport — Crypto, PySide6, py7zr… — remplacés par des modules
//! vides), extrait `FlamDevice.__get_lunii_ciphered_name` de `device_flam.py` par `ast`, puis,
//! pour chaque histoire synthétique de `tests/data/studio/*.json` (écrite par ce même script,
//! `casse.json` avec un BOM UTF-8), consigne dans `tests/data/studio/expected.json` :
//! `get_ni_data()`, `get_li_data()`, `get_ri_data()`, `get_si_data()` en hexadécimal, `uuid`,
//! `nm`, et pour chaque entrée de `ri` puis de `si` le triplet
//! (`assets/<nom>`, nom cible avec `/`, nature). Aucune histoire du commerce n'est employée.
//!
//! Deux cas suivent une règle de la note de conception (`docs/studio-lunii.md`) au lieu de
//! Lunii.QT seul : `square_one` (nœud `squareOne` en 3e position) a pour attendus la sortie de
//! Lunii.QT sur le même JSON dont ce nœud est déplacé en tête (§ 2.2) ; `noms_par_rang` (noms
//! courts) a `ni`/`li` de Lunii.QT et `ri`/`si`/noms cibles calculés par la règle du rang
//! (§ 3.4), Lunii.QT y produisant des entrées de moins de 12 octets.

use serde_json::Value;
use studio2lunii::index::{build_index, AssetKind, Naming};
use studio2lunii::story::parse_story;

const CASES: &[(&str, &[u8])] = &[
    ("lineaire", include_bytes!("data/studio/lineaire.json")),
    ("menu", include_bytes!("data/studio/menu.json")),
    (
        "boucle_accueil",
        include_bytes!("data/studio/boucle_accueil.json"),
    ),
    ("mode_nuit", include_bytes!("data/studio/mode_nuit.json")),
    (
        "assets_partages",
        include_bytes!("data/studio/assets_partages.json"),
    ),
    ("casse", include_bytes!("data/studio/casse.json")),
    ("minimal", include_bytes!("data/studio/minimal.json")),
    ("une_option", include_bytes!("data/studio/une_option.json")),
    ("square_one", include_bytes!("data/studio/square_one.json")),
    (
        "noms_par_rang",
        include_bytes!("data/studio/noms_par_rang.json"),
    ),
];

fn hex(s: &str) -> Vec<u8> {
    assert!(s.len().is_multiple_of(2));
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn parite_lunii_qt() {
    let expected: Value =
        serde_json::from_slice(include_bytes!("data/studio/expected.json")).unwrap();
    let table = expected.as_object().unwrap();
    assert_eq!(table.len(), CASES.len(), "cas de l'oracle non couverts");
    for (name, json) in CASES {
        let exp = &table[*name];
        let story = parse_story(json).unwrap_or_else(|e| panic!("{name} : {e}"));
        let index = build_index(&story);
        let naming = match index.naming {
            Naming::LuniiQt => "lunii",
            Naming::Rank(_) => "rank",
        };
        assert_eq!(naming, exp["naming"].as_str().unwrap(), "{name} : nommage");
        assert_eq!(story.uuid(), exp["uuid"].as_str().unwrap(), "{name} : uuid");
        assert_eq!(
            index.night_mode,
            exp["night_mode"].as_bool().unwrap(),
            "{name} : nm"
        );
        for (file, got) in [
            ("ni", &index.ni),
            ("li", &index.li),
            ("ri", &index.ri),
            ("si", &index.si),
        ] {
            assert_eq!(
                got,
                &hex(exp[file].as_str().unwrap()),
                "{name} : {file} diffère de Lunii.QT"
            );
        }
        let assets: Vec<(String, String, &str)> = index
            .assets
            .iter()
            .map(|a| {
                let kind = match a.kind {
                    AssetKind::Image => "image",
                    AssetKind::Audio => "audio",
                };
                assert_eq!(a.archive_path, format!("assets/{}", a.name));
                (a.archive_path.clone(), a.pack_path.clone(), kind)
            })
            .collect();
        let exp_assets: Vec<(String, String, &str)> = exp["assets"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                (
                    t[0].as_str().unwrap().to_owned(),
                    t[1].as_str().unwrap().to_owned(),
                    t[2].as_str().unwrap(),
                )
            })
            .collect();
        assert_eq!(assets, exp_assets, "{name} : table des assets");
    }
}

#[test]
fn disposition_ni_lisible() {
    // Contrôle indépendant de l'oracle sur le cas linéaire : en-tête et premier nœud.
    let story = parse_story(include_bytes!("data/studio/lineaire.json")).unwrap();
    let ni = build_index(&story).ni;
    assert_eq!(ni.len(), 0x200 + 4 * 0x2C);
    assert_eq!(&ni[0..4], &[1, 0, 1, 0]); // format v1, version 1
    assert_eq!(&ni[4..8], &0x200u32.to_le_bytes());
    assert_eq!(&ni[8..12], &0x2Cu32.to_le_bytes());
    assert_eq!(&ni[12..16], &4u32.to_le_bytes()); // stageNodes
    assert_eq!(&ni[16..20], &1u32.to_le_bytes()); // images
    assert_eq!(&ni[20..24], &4u32.to_le_bytes()); // sons
    assert_eq!(ni[24], 1);
    assert!(ni[25..0x200].iter().all(|&b| b == 0));
    let n0 = &ni[0x200..0x200 + 0x2C];
    assert_eq!(&n0[0..8], &[0, 0, 0, 0, 0, 0, 0, 0]); // image 0, son 0
    assert_eq!(&n0[8..20], &[1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]); // ok -> li[1], 1 option, 0
    assert_eq!(&n0[20..32], &[0xFF; 12]); // pas de transition maison
    assert_eq!(&n0[32..44], &[1, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
}
