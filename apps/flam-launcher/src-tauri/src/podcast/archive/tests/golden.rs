//! Non-régression des packs de podcasts : `metadata.json`, `nodes.json` et `notes.json` sont
//! identiques, octet pour octet, à ceux produits avant l'ajout des modes recueil / chapitres
//! (lot 3 de `docs/creation-dossier.md`). Les fichiers de référence de `golden/` ont été
//! produits par le code d'avant ce lot ; `FLAM_UPDATE_GOLDEN=1` les réécrit (à ne faire que
//! pour un changement voulu du format des podcasts).

use super::*;

/// uuid fixe : le seul élément aléatoire d'un pack.
const UUID: &str = "0b5e0c1d-3f6a-4b7e-9c2d-5a6b7c8d9e0f";

/// Les trois JSON de l'archive, à la suite, chacun précédé de son nom, puis la liste des
/// entrées de l'archive avec leur taille.
fn snapshot(out: &Path) -> String {
    let files = read_zip(out);
    let mut s = String::new();
    for name in ["metadata.json", "nodes.json", "notes.json"] {
        s.push_str(&format!("== {name} ==\n"));
        s.push_str(std::str::from_utf8(&files[name]).unwrap());
        s.push('\n');
    }
    s.push_str("== entrées ==\n");
    for (name, bytes) in &files {
        s.push_str(&format!("{name} {}\n", bytes.len()));
    }
    s
}

fn check(name: &str, p: &Pack, fx: &Fixture) {
    write_archive(p, &fx.out(), &never).unwrap();
    super::chapters::assert_graph(&json_of(&read_zip(&fx.out()), "nodes.json"));
    let got = snapshot(&fx.out());
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/podcast/archive/tests/golden")
        .join(name);
    if std::env::var_os("FLAM_UPDATE_GOLDEN").is_some_and(|v| v == "1") {
        fs::write(&path, &got).unwrap();
    }
    let want = fs::read_to_string(&path).unwrap().replace("\r\n", "\n");
    assert!(got == want, "{name} : différent de la référence\n{got}");
}

fn fixed(mut p: Pack) -> Pack {
    p.uuid = Uuid::parse_str(UUID).unwrap();
    p
}

/// Trois niveaux, titres lus partiels, image choisie d'une catégorie, épisode sans image,
/// titre lu du pack, couverture.
#[test]
fn podcast_trois_niveaux_json_identique() {
    let mut fx = Fixture::new();
    let mut menu = three_levels(&mut fx);
    speak_all(&mut fx, &mut menu, &["B", "Y"]);
    let d = episode_image(&menu, 4);
    choose_image(&mut menu, "X", Some(d));
    if let MenuEntry::Episode(e) = &mut menu[2] {
        e.image = None;
        e.title = "L’œuvre — 🎉 un très long titre d’épisode qui déborde".into();
    }
    let mut p = fixed(pack(menu, Some(fx.cover())));
    p.title_audio = Some(fx.title_mp3());
    check("podcast_trois_niveaux.txt", &p, &fx);
}

/// Menu plat de 12 épisodes, sans titre lu ni couverture.
#[test]
fn podcast_menu_plat_json_identique() {
    let mut fx = Fixture::new();
    let menu = fx.episodes(12);
    let p = fixed(pack(menu, None));
    check("podcast_menu_plat.txt", &p, &fx);
}
