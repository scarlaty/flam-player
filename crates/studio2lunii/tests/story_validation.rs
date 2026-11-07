//! Validation de `story.json` face à une archive hostile ou incohérente, et cas limites admis.

use serde_json::{json, Value};
use studio2lunii::index::{build_index, IndexError, Naming};
use studio2lunii::story::{
    parse_story, StoryError, MAX_ASSETS, MAX_ASSET_NAME_BYTES, MAX_OPTIONS, MAX_STAGE_NODES,
    MAX_STORY_JSON_BYTES,
};

const U0: &str = "00000000-0000-4000-8000-000000000000";
const U1: &str = "00000000-0000-4000-8000-000000000001";

/// Histoire minimale valide : couverture -> nœud 1 -> retour à la couverture (cycle).
fn base() -> Value {
    json!({
        "format": "v1",
        "version": 1,
        "title": "t",
        "stageNodes": [
            {
                "uuid": U0, "image": "imgcouv1.png", "audio": "soncouv1.mp3",
                "okTransition": {"actionNode": "a", "optionIndex": 0},
                "homeTransition": null,
                "controlSettings": {"wheel": true, "ok": true, "home": false,
                                    "pause": false, "autoplay": false}
            },
            {
                "uuid": U1, "audio": "sonhist1.mp3",
                "okTransition": {"actionNode": "retour", "optionIndex": 0},
                "homeTransition": {"actionNode": "retour", "optionIndex": 0},
                "controlSettings": {"wheel": false, "ok": true, "home": true,
                                    "pause": true, "autoplay": true}
            }
        ],
        "actionNodes": [
            {"id": "a", "options": [U1]},
            {"id": "retour", "options": [U0]}
        ]
    })
}

fn parse(v: &Value) -> Result<studio2lunii::story::Story, StoryError> {
    parse_story(&serde_json::to_vec(v).unwrap())
}

fn err(v: &Value) -> StoryError {
    parse(v).expect_err("devait être refusé")
}

#[test]
fn base_valide_avec_cycle() {
    let story = parse(&base()).unwrap();
    assert_eq!(story.uuid(), U0);
    assert_eq!(story.title(), "t");
    assert_eq!(story.description(), "");
    assert!(!story.night_mode());
    let n1 = &story.stage_nodes()[1];
    assert_eq!(n1.image, None);
    assert!(n1.controls.autoplay && !n1.controls.wheel);
    assert_eq!(n1.ok_transition.unwrap().action_node, 1);
    assert_eq!(story.action_nodes()[1].options, vec![0]);
    assert_eq!(build_index(&story).naming, Naming::LuniiQt);
}

#[test]
fn square_one_place_en_tete() {
    let mut v = base();
    v["stageNodes"][1]["squareOne"] = json!(true);
    v["stageNodes"][0]["squareOne"] = json!(false);
    let story = parse(&v).unwrap();
    assert_eq!(story.uuid(), U1);
    assert_eq!(story.stage_nodes()[0].uuid, U1);
    assert_eq!(story.stage_nodes()[1].uuid, U0);
    // Indices recalculés : l'option de « a » (U1) est maintenant le nœud 0.
    assert_eq!(story.action_nodes()[0].options, vec![0]);
    assert_eq!(story.action_nodes()[1].options, vec![1]);

    let mut v = base();
    v["stageNodes"][0]["squareOne"] = json!(true);
    v["stageNodes"][1]["squareOne"] = json!(true);
    assert_eq!(
        err(&v),
        StoryError::PlusieursSquareOne {
            premier: 1,
            second: 2
        }
    );
}

#[test]
fn controles_obligatoires() {
    for absent in [None, Some(Value::Null)] {
        let mut v = base();
        match absent {
            None => {
                v["stageNodes"][1]
                    .as_object_mut()
                    .unwrap()
                    .remove("controlSettings");
            }
            Some(n) => v["stageNodes"][1]["controlSettings"] = n,
        }
        assert_eq!(
            err(&v),
            StoryError::ControlesAbsents {
                uuid: U1.to_owned()
            }
        );
    }
    let mut v = base();
    v["stageNodes"][1]["controlSettings"] = json!({});
    assert!(matches!(err(&v), StoryError::Json(_)));
}

#[test]
fn bom_et_uuid_en_majuscules() {
    let mut v = base();
    let up = U0.to_uppercase().replace('0', "A");
    v["stageNodes"][0]["uuid"] = json!(up);
    v["actionNodes"][1]["options"] = json!([up]);
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(serde_json::to_vec(&v).unwrap());
    let story = parse_story(&bytes).unwrap();
    assert_eq!(story.uuid(), up.to_lowercase());
}

#[test]
fn json_invalide() {
    assert!(matches!(parse_story(b"{"), Err(StoryError::Json(_))));
    assert!(matches!(
        parse_story(b"\xFF\xFE{}"),
        Err(StoryError::Json(_))
    ));
    assert!(matches!(parse_story(b"[]"), Err(StoryError::Json(_))));
    let mut v = base();
    v.as_object_mut().unwrap().remove("stageNodes");
    assert!(matches!(err(&v), StoryError::Json(_)));
    let mut v = base();
    v["stageNodes"][0]["controlSettings"]["ok"] = json!(1);
    assert!(matches!(err(&v), StoryError::Json(_)));
    let mut v = base();
    v["stageNodes"][0]["image"] = json!(5);
    assert!(matches!(err(&v), StoryError::Json(_)));
    let mut v = base();
    v["version"] = json!(1.5);
    assert!(matches!(err(&v), StoryError::Json(_)));
    // Imbrication profonde : bornée par serde_json, sans débordement de pile.
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    assert!(matches!(
        parse_story(deep.as_bytes()),
        Err(StoryError::Json(_))
    ));
}

#[test]
fn taille_maximale() {
    let big = vec![b' '; MAX_STORY_JSON_BYTES + 1];
    assert_eq!(
        parse_story(&big),
        Err(StoryError::TropVolumineux {
            taille: MAX_STORY_JSON_BYTES + 1,
            max: MAX_STORY_JSON_BYTES
        })
    );
}

#[test]
fn format_et_version_hors_bornes() {
    for f in ["1", "v", "v2", "v70000", "v-1", "x1", "V1"] {
        let mut v = base();
        v["format"] = json!(f);
        assert_eq!(err(&v), StoryError::Format(f.to_owned()));
    }
    for n in [-1i64, 65_536, i64::MAX] {
        let mut v = base();
        v["version"] = json!(n);
        assert_eq!(err(&v), StoryError::Version(n));
    }
}

#[test]
fn aucun_ou_trop_de_noeuds() {
    let mut v = base();
    v["stageNodes"] = json!([]);
    v["actionNodes"] = json!([]);
    assert_eq!(err(&v), StoryError::AucunStageNode);

    let nodes: Vec<Value> = (0..=MAX_STAGE_NODES)
        .map(|i| json!({"uuid": format!("00000000-0000-4000-8000-{i:012x}")}))
        .collect();
    let mut v = base();
    v["stageNodes"] = Value::Array(nodes);
    v["actionNodes"] = json!([]);
    assert!(matches!(
        err(&v),
        StoryError::TropDeNoeuds {
            quoi: "stageNodes",
            ..
        }
    ));

    let mut v = base();
    v["actionNodes"] = json!([
        {"id": "a", "options": vec![U1; MAX_OPTIONS / 2 + 1]},
        {"id": "retour", "options": vec![U0; MAX_OPTIONS / 2]}
    ]);
    assert!(matches!(err(&v), StoryError::TropDeNoeuds { .. }));
}

/// Les éléments au-delà du plafond ne sont jamais désérialisés : ici des valeurs qui ne sont
/// pas des stageNodes (ni même des objets) après les `MAX + 1` premiers, et le nombre exact
/// d'éléments est quand même rapporté.
#[test]
fn tableaux_geants_comptes_sans_etre_materialises() {
    let node = |i: usize| format!(r#"{{"uuid":"00000000-0000-4000-8000-{i:012x}"}}"#);
    let mut json = String::from(r#"{"format":"v1","version":1,"stageNodes":["#);
    for i in 0..=MAX_STAGE_NODES {
        json.push_str(&node(i));
        json.push(',');
    }
    json.push_str(&"7,".repeat(100_000));
    json.push_str(r#"7],"actionNodes":[]}"#);
    assert_eq!(
        parse_story(json.as_bytes()),
        Err(StoryError::TropDeNoeuds {
            quoi: "stageNodes",
            nombre: MAX_STAGE_NODES + 1 + 100_001,
            max: MAX_STAGE_NODES
        })
    );

    let mut json = format!(
        r#"{{"format":"v1","version":1,"stageNodes":[{}],"actionNodes":[{{"id":"a","options":["#,
        node(0)
    );
    json.push_str(&format!(r#""{U0}","#).repeat(MAX_OPTIONS / 2 + 1));
    json.push_str(&format!(r#""{U0}","#).repeat(MAX_OPTIONS / 2));
    json.push_str(&"7,".repeat(5_000));
    json.push_str(r#"7]}]}"#);
    assert_eq!(
        parse_story(json.as_bytes()),
        Err(StoryError::TropDeNoeuds {
            quoi: "options d'actionNodes",
            nombre: MAX_OPTIONS + 1 + 5_001,
            max: MAX_OPTIONS
        })
    );
}

#[test]
fn numeros_de_noeud_comptes_depuis_un() {
    let mut v = base();
    v["stageNodes"][0]["squareOne"] = json!(true);
    v["stageNodes"][1]["squareOne"] = json!(true);
    let e = err(&v);
    assert_eq!(
        e,
        StoryError::PlusieursSquareOne {
            premier: 1,
            second: 2
        }
    );
    assert!(e.to_string().contains("n° 1 et n° 2"), "{e}");

    let mut v = base();
    v["stageNodes"][0]["uuid"] = json!("pas-un-uuid");
    let e = err(&v);
    assert!(e.to_string().contains("stageNode n° 1 :"), "{e}");

    let mut v = base();
    v["actionNodes"][1]["id"] = json!("a");
    let e = err(&v);
    assert!(e.to_string().contains("actionNode n° 2 :"), "{e}");
}

#[test]
fn uuid_invalides_ou_dupliques() {
    for bad in [
        "",
        "pas-un-uuid",
        "{00000000-0000-4000-8000-000000000000}",
        "../../x",
    ] {
        let mut v = base();
        v["stageNodes"][1]["uuid"] = json!(bad);
        assert!(
            matches!(err(&v), StoryError::UuidInvalide { noeud: 2, .. }),
            "{bad}"
        );
    }
    let mut v = base();
    v["stageNodes"][0]["uuid"] = json!("abcdef00-0000-4000-8000-000000000000");
    v["stageNodes"][1]["uuid"] = json!("ABCDEF00-0000-4000-8000-000000000000");
    assert!(matches!(err(&v), StoryError::UuidDuplique { noeud: 2, .. }));
}

#[test]
fn references_inexistantes() {
    let mut v = base();
    v["actionNodes"][0]["options"] = json!(["00000000-0000-4000-8000-0000000000ff"]);
    assert!(matches!(err(&v), StoryError::OptionInconnue { .. }));

    // Comparaison exacte, comme Lunii.QT : une casse différente ne résout pas.
    let mut v = base();
    v["stageNodes"][1]["uuid"] = json!("00000000-0000-4000-8000-00000000000A");
    v["actionNodes"][0]["options"] = json!(["00000000-0000-4000-8000-00000000000a"]);
    assert!(matches!(err(&v), StoryError::OptionInconnue { .. }));

    let mut v = base();
    v["stageNodes"][0]["okTransition"]["actionNode"] = json!("absent");
    assert_eq!(
        err(&v),
        StoryError::ActionInconnue {
            uuid: U0.to_owned(),
            transition: "okTransition",
            action: "absent".to_owned()
        }
    );
    let mut v = base();
    v["stageNodes"][1]["homeTransition"]["actionNode"] = json!("absent");
    assert!(matches!(
        err(&v),
        StoryError::ActionInconnue {
            transition: "homeTransition",
            ..
        }
    ));

    let mut v = base();
    v["actionNodes"][1]["id"] = json!("a");
    assert!(matches!(
        err(&v),
        StoryError::ActionDupliquee { action: 2, .. }
    ));

    let mut v = base();
    v["actionNodes"][0]["options"] = json!([]);
    assert!(matches!(err(&v), StoryError::ActionSansOption { .. }));
}

#[test]
fn option_index() {
    // -1 (hasard ?) refusé tant que son effet n'est pas mesuré (note, Q5).
    for bad in [1i64, -1, -2, i64::from(u32::MAX) + 1, i64::MIN, i64::MAX] {
        let mut v = base();
        v["stageNodes"][0]["okTransition"]["optionIndex"] = json!(bad);
        assert!(
            matches!(err(&v), StoryError::OptionIndexHorsBornes { index, options: 1, .. } if index == bad),
            "{bad}"
        );
    }
}

#[test]
fn noms_d_assets_dangereux() {
    let long = "a".repeat(MAX_ASSET_NAME_BYTES + 1);
    for bad in [
        "../../evil.mp3",
        "a..b.mp3",
        "/etc/passwd",
        "sous/dossier.mp3",
        "C:evil.mp3",
        "c:\\x.mp3",
        "\\\\serveur\\x.mp3",
        "nul\u{0}.mp3",
        "ligne\n.mp3",
        "   ",
        long.as_str(),
    ] {
        for field in ["image", "audio"] {
            let mut v = base();
            v["stageNodes"][1][field] = json!(bad);
            assert!(
                matches!(err(&v), StoryError::NomAsset { .. }),
                "{field} {bad:?}"
            );
        }
    }
    // Vide ou null : pas d'asset, comme `if image:` de Lunii.QT.
    let mut v = base();
    v["stageNodes"][0]["image"] = json!("");
    v["stageNodes"][0]["audio"] = Value::Null;
    let s = parse(&v).unwrap();
    assert_eq!(s.stage_nodes()[0].image, None);
    assert_eq!(s.stage_nodes()[0].audio, None);
}

#[test]
fn asset_image_et_son() {
    let mut v = base();
    v["stageNodes"][1]["image"] = json!("soncouv1.mp3");
    assert_eq!(
        err(&v),
        StoryError::AssetImageEtSon {
            nom: "soncouv1.mp3".to_owned()
        }
    );
}

#[test]
fn trop_d_assets() {
    let nodes: Vec<Value> = (0..=MAX_ASSETS)
        .map(|i| {
            json!({
                "uuid": format!("00000000-0000-4000-8000-{i:012x}"),
                "image": format!("{i:08}.png"),
                "controlSettings": {"wheel": true, "ok": true, "home": true,
                                    "pause": true, "autoplay": true}
            })
        })
        .collect();
    let mut v = base();
    v["stageNodes"] = Value::Array(nodes);
    v["actionNodes"] = json!([]);
    assert_eq!(
        err(&v),
        StoryError::TropDAssets {
            quoi: "images distinctes",
            max: MAX_ASSETS
        }
    );
}

#[test]
fn nommage_par_rang_si_regle_lunii_non_sure() {
    let rank = |field: &str, name: &str| {
        let mut v = base();
        v["stageNodes"][1][field] = json!(name);
        let index = build_index(&parse(&v).unwrap());
        match index.naming.clone() {
            Naming::Rank(raison) => (raison, index),
            Naming::LuniiQt => panic!("{name} : règle de Lunii.QT gardée"),
        }
    };
    let (raison, index) = rank("audio", "court.mp3");
    assert!(matches!(raison, IndexError::NomCourt { .. }));
    // Tout le pack passe au rang, images comprises.
    assert_eq!(index.ri, b"000\\00000000");
    assert_eq!(index.si, b"000\\00000000000\\00000001");
    let paths: Vec<_> = index.assets.iter().map(|a| a.pack_path.as_str()).collect();
    assert_eq!(
        paths,
        ["rf/000/00000000", "sf/000/00000000", "sf/000/00000001"]
    );

    for bad in ["accentué.png", "image_ab-cdefg.png"] {
        assert!(matches!(
            rank("image", bad).0,
            IndexError::NomInvalide { .. }
        ));
    }
    // Même nom cible pour deux images distinctes (à la casse près) : Lunii.QT écraserait la
    // première.
    assert_eq!(
        rank("image", "autre_IMGCOUV1.jpg").0,
        IndexError::Collision {
            premier: "imgcouv1.png".to_owned(),
            second: "autre_IMGCOUV1.jpg".to_owned(),
            cible: "rf/000/IMGCOUV1".to_owned()
        }
    );
    // Image et son au même nom cible : répertoires distincts, règle de Lunii.QT gardée.
    let mut v = base();
    v["stageNodes"][1]["image"] = json!("soncouv1.png");
    let index = build_index(&parse(&v).unwrap());
    assert_eq!(index.naming, Naming::LuniiQt);
    assert_eq!(index.ri, b"000\\IMGCOUV1000\\SONCOUV1");
}

#[test]
fn rang_au_dela_de_255_assets() {
    // 300 images aux noms courts : rangs hexadécimaux sur 8 chiffres, entrées de 12 octets.
    let nodes: Vec<Value> = (0..300)
        .map(|i| {
            json!({
                "uuid": format!("00000000-0000-4000-8000-{i:012x}"),
                "image": format!("i{i}.png"),
                "controlSettings": {"wheel": true, "ok": true, "home": true,
                                    "pause": true, "autoplay": true}
            })
        })
        .collect();
    let mut v = base();
    v["stageNodes"] = Value::Array(nodes);
    v["actionNodes"] = json!([]);
    let index = build_index(&parse(&v).unwrap());
    assert_eq!(index.ri.len(), 300 * 12);
    assert_eq!(&index.ri[299 * 12..], b"000\\0000012B");
    assert_eq!(index.assets[299].pack_path, "rf/000/0000012B");
}
