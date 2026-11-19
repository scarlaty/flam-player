use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::authoring::tree::Node;

// ---------------------------------------------------------------------------
// Aides
// ---------------------------------------------------------------------------

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn read(root: &Path, rel: &str) -> Vec<u8> {
    fs::read(root.join(rel)).unwrap_or_else(|e| panic!("{rel} : {e}"))
}

fn exists(root: &Path, rel: &str) -> bool {
    fs::symlink_metadata(root.join(rel)).is_ok()
}

fn json(root: &Path, rel: &str) -> serde_json::Value {
    serde_json::from_slice(&read(root, rel)).unwrap()
}

fn open(root: &Path) -> Editor {
    Editor::open(
        root,
        ForbiddenRoots::default(),
        Vec::new(),
        Limits::default(),
    )
    .unwrap()
}

fn target(ed: &Editor, id: &str) -> Target {
    Target {
        id: id.to_owned(),
        stamp: ed.stamp(id).unwrap(),
    }
}

/// Contenu du dossier (hors `.flam/`) : chemin relatif → octets (`None` : dossier).
fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(dir: &Path, rel: &str, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            if rel.is_empty() && name == EDITOR_DIR {
                continue;
            }
            let child = join(rel, &name);
            let meta = fs::symlink_metadata(entry.path()).unwrap();
            if meta.is_dir() && !meta.file_type().is_symlink() {
                out.insert(child.clone(), None);
                walk(&entry.path(), &child, out);
            } else {
                out.insert(child, Some(fs::read(entry.path()).unwrap_or_default()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, "", &mut out);
    out
}

/// Fichiers de la corbeille : nom (sans la place) → octets.
fn trash(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let dir = root.join(TRASH_DIR);
    let Ok(slots) = fs::read_dir(&dir) else {
        return out;
    };
    for slot in slots {
        let slot = slot.unwrap().path();
        for (rel, bytes) in snapshot(&slot) {
            if let Some(bytes) = bytes {
                out.push((rel, bytes));
            }
        }
    }
    out.sort();
    out
}

fn trash_names(root: &Path) -> Vec<String> {
    trash(root).into_iter().map(|(n, _)| n).collect()
}

fn ids(scan: &Scan, id: &str) -> Vec<String> {
    scan.root
        .find(id)
        .unwrap_or_else(|| panic!("{id} absent"))
        .children()
        .iter()
        .map(|n| n.id().to_owned())
        .collect()
}

fn junction(link: &Path, target: &Path) {
    // Jonction (sans privilège) ; séparateurs Windows pour `mklink`.
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link.to_string_lossy().replace('/', "\\"))
        .arg(target.to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(status.status.success(), "mklink /J a échoué");
}

// ---------------------------------------------------------------------------
// Identifiants et noms
// ---------------------------------------------------------------------------

#[test]
fn identifiants_refuses() {
    for bad in [
        "..",
        ".",
        "a/../b",
        "a/./b",
        r"a\b",
        "C:",
        "C:/x",
        "a.mp3:flux",
        ".flam",
        ".flam/corbeille",
        "a/",
        "/a",
        "a//b",
        "con",
        "nul.mp3",
        "a.",
        "a ",
        "a?b",
        "a\u{7}b",
    ] {
        assert!(
            matches!(parse_id(bad), Err(OpError::InvalidId(_))),
            "{bad:?} accepté"
        );
    }
    for good in ["", "a.mp3", "01 Animaux/Le loup.mp3", "été/à.jpg"] {
        assert_eq!(parse_id(good).unwrap(), good);
    }
}

#[test]
fn noms_choisis_rendus_surs() {
    assert_eq!(sanitize_name("Le loup").as_deref(), Some("Le loup"));
    assert_eq!(sanitize_name("a/b:c?").as_deref(), Some("a b c"));
    assert_eq!(sanitize_name("../x").as_deref(), Some("x"));
    assert_eq!(sanitize_name("fin...").as_deref(), Some("fin"));
    assert_eq!(sanitize_name("CON").as_deref(), Some("CON_"));
    assert_eq!(sanitize_name(" ... "), None);
    assert_eq!(sanitize_name("::"), None);
}

#[test]
fn reglages_partiels_distinguent_absent_et_efface() {
    let patch: StoryPatch =
        serde_json::from_str(r#"{"titre": null, "age": 6, "mode": "chapitres"}"#).unwrap();
    assert_eq!(patch.titre, Some(None));
    assert_eq!(patch.age, Some(Some(6)));
    assert_eq!(patch.description, None);
    assert_eq!(patch.folder.mode, Some(Some(Mode::Chapitres)));
    assert_eq!(patch.folder.fin, None);
}

// ---------------------------------------------------------------------------
// Ouverture et racine
// ---------------------------------------------------------------------------

#[test]
fn ouverture_n_ecrit_rien_et_refuse_fichier_et_lien() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    let ed = open(dir.path());
    assert!(!exists(dir.path(), EDITOR_DIR));
    assert!(!ed.can_undo());
    let file = dir.path().join("a.mp3");
    assert!(matches!(
        Editor::open(&file, ForbiddenRoots::default(), vec![], Limits::default()),
        Err(OpError::Root(ScanError::NotADirectory(_)))
    ));
    let link = dir.path().join("lien");
    let real = tempfile::tempdir().unwrap();
    junction(&link, real.path());
    assert!(matches!(
        Editor::open(&link, ForbiddenRoots::default(), vec![], Limits::default()),
        Err(OpError::Root(ScanError::Link(_)))
    ));
}

#[test]
fn racine_remplacee_par_une_jonction() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("histoire");
    write(&root, "a.mp3", b"a");
    let mut ed = open(&root);
    let t = target(&ed, "a.mp3");
    // Le dossier est remplacé par une jonction vers une copie identique.
    let moved = base.path().join("ailleurs");
    fs::rename(&root, &moved).unwrap();
    junction(&root, &moved);
    let err = ed.delete(&t).unwrap_err();
    assert!(
        matches!(err, OpError::RootChanged | OpError::Root(_)),
        "{err:?}"
    );
    assert!(exists(&moved, "a.mp3"));
    assert!(!exists(&moved, EDITOR_DIR));
}

#[test]
fn racines_interdites_par_identite_via_partage_administratif() {
    let base = tempfile::tempdir().unwrap();
    let interdit = base.path().join("interdit");
    let story = interdit.join("histoire");
    write(&story, "a.mp3", b"a");
    let text = fs::canonicalize(&story).unwrap();
    let text = crate::authoring::root::simplify(text)
        .to_string_lossy()
        .into_owned();
    let Some(rest) = text.strip_prefix(r"C:\") else {
        eprintln!("dossier temporaire hors de C: : test sauté");
        return;
    };
    let unc = PathBuf::from(format!(r"\\localhost\C$\{rest}"));
    if fs::symlink_metadata(&unc).is_err() {
        eprintln!("partage administratif inaccessible : test sauté");
        return;
    }
    // Même dossier : les deux formes ont la même identité.
    let trees = ForbiddenRoots {
        profile: vec![],
        trees: vec![interdit.clone()],
    };
    assert!(matches!(
        Editor::open(&story, trees.clone(), vec![], Limits::default()),
        Err(OpError::Root(ScanError::Forbidden { .. }))
    ));
    assert!(matches!(
        Editor::open(&unc, trees, vec![], Limits::default()),
        Err(OpError::Root(ScanError::Forbidden { .. }))
    ));
    // Profil : son parent (par le partage) est refusé, un sous-dossier permis.
    let profile = ForbiddenRoots {
        profile: vec![story.clone()],
        trees: vec![],
    };
    let unc_parent = unc.parent().unwrap().to_path_buf();
    assert!(matches!(
        Editor::open(&unc_parent, profile.clone(), vec![], Limits::default()),
        Err(OpError::Root(ScanError::Forbidden { .. }))
    ));
    fs::create_dir(story.join("sous")).unwrap();
    assert!(Editor::open(&unc.join("sous"), profile, vec![], Limits::default()).is_ok());
}

#[test]
fn dossier_interdit_absent_ne_refuse_pas_ses_voisins() {
    let base = tempfile::tempdir().unwrap();
    let story = base.path().join("histoire");
    write(&story, "a.mp3", b"a");
    let roots = ForbiddenRoots {
        profile: vec![],
        trees: vec![base.path().join("launcher-absent")],
    };
    assert!(Editor::open(&story, roots, vec![], Limits::default()).is_ok());
}

#[test]
fn epinglage_empeche_de_renommer_le_dossier() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "A/B/x.mp3", b"x");
    let ed = open(dir.path());
    let pins = ed.pin_dir("A/B").unwrap();
    assert!(fs::rename(dir.path().join("A/B"), dir.path().join("A/C")).is_err());
    assert!(fs::rename(dir.path().join("A"), dir.path().join("Z")).is_err());
    // Les fichiers du dossier épinglé restent déplaçables (par l'éditeur).
    fsx::rename_noreplace(&dir.path().join("A/B/x.mp3"), &dir.path().join("A/B/y.mp3")).unwrap();
    drop(pins);
    fs::rename(dir.path().join("A/B"), dir.path().join("A/C")).unwrap();
}

#[test]
fn jonction_dans_le_chemin_refusee_au_moment_d_agir() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(dir.path(), "A/x.mp3", b"x");
    write(outside.path(), "y.mp3", b"y");
    let mut ed = open(dir.path());
    let src = tempfile::tempdir().unwrap();
    write(src.path(), "z.mp3", b"z");
    ed.import("A", &[src.path().join("z.mp3")]).unwrap();
    // Entre l'opération et son annulation, « A » devient une jonction vers un dossier qui
    // contient un fichier de même nom.
    fs::rename(dir.path().join("A"), dir.path().join("A0")).unwrap();
    fs::copy(dir.path().join("A0/z.mp3"), outside.path().join("z.mp3")).unwrap();
    junction(&dir.path().join("A"), outside.path());
    let err = ed.undo().unwrap_err();
    assert!(matches!(err, OpError::NotUndoable { .. }), "{err:?}");
    assert!(
        exists(outside.path(), "z.mp3"),
        "cible de la jonction intacte"
    );
    assert!(exists(outside.path(), "y.mp3"));
    assert!(matches!(ed.pin_dir("A"), Err(OpError::Link(_))));
    assert!(matches!(
        ed.stamp("A/x.mp3"),
        Ok(_) | Err(OpError::NotFound(_))
    ));
}

#[test]
fn corbeille_en_jonction_refusee() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    junction(&dir.path().join(EDITOR_DIR), outside.path());
    let mut ed = open(dir.path());
    let t = target(&ed, "a.mp3");
    let err = ed.delete(&t).unwrap_err();
    assert!(matches!(err, OpError::Link(_)), "{err:?}");
    assert!(exists(dir.path(), "a.mp3"));
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
}

// ---------------------------------------------------------------------------
// Copie
// ---------------------------------------------------------------------------

#[test]
fn copie_avec_image_et_titre_lu_sous_un_nom_libre() {
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    write(dir.path(), "Loup.mp3", b"ancien");
    for (name, bytes) in [
        ("Loup.mp3", &b"nouveau"[..]),
        ("Loup.jpg", b"img"),
        ("Loup.titre.mp3", b"titre"),
        ("notes.txt", b"txt"),
    ] {
        write(src.path(), name, bytes);
    }
    let before_src = snapshot(src.path());
    let mut ed = open(dir.path());
    let sources: Vec<PathBuf> = ["Loup.mp3", "Loup.jpg", "Loup.titre.mp3", "notes.txt"]
        .iter()
        .map(|n| src.path().join(n))
        .collect();
    let report = ed.import("", &sources).unwrap();
    assert_eq!(
        report.done.ids,
        ["Loup (2).mp3", "Loup (2).jpg", "Loup (2).titre.mp3"]
    );
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, "format non pris en charge");
    assert_eq!(read(dir.path(), "Loup.mp3"), b"ancien");
    assert_eq!(read(dir.path(), "Loup (2).mp3"), b"nouveau");
    assert_eq!(snapshot(src.path()), before_src, "originaux intacts");
    let scan = ed.scan().unwrap();
    match scan.root.find("Loup (2).mp3").unwrap() {
        Node::Story {
            image, title_audio, ..
        } => {
            assert_eq!(image.as_deref(), Some("Loup (2).jpg"));
            assert_eq!(title_audio.as_deref(), Some("Loup (2).titre.mp3"));
        }
        other => panic!("{other:?}"),
    }
    assert!(match scan.root.find("Loup.mp3").unwrap() {
        Node::Story { image, .. } => image.is_none(),
        _ => false,
    });

    // Annuler : les copies vont dans la corbeille ; rétablir : elles reviennent.
    ed.undo().unwrap();
    assert!(!exists(dir.path(), "Loup (2).mp3"));
    assert_eq!(
        trash_names(dir.path()),
        ["Loup (2).jpg", "Loup (2).mp3", "Loup (2).titre.mp3"]
    );
    ed.redo().unwrap();
    assert_eq!(read(dir.path(), "Loup (2).titre.mp3"), b"titre");
    assert!(trash(dir.path()).is_empty());
    assert!(!exists(dir.path(), ".flam/tmp/x"));
}

#[test]
fn copie_de_deux_fichiers_de_meme_nom() {
    let dir = tempfile::tempdir().unwrap();
    let (s1, s2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(dir.path(), "M/x.txt", b"");
    write(s1.path(), "a.mp3", b"1");
    write(s2.path(), "a.mp3", b"2");
    let mut ed = open(dir.path());
    let report = ed
        .import("M", &[s1.path().join("a.mp3"), s2.path().join("a.mp3")])
        .unwrap();
    assert_eq!(report.done.ids, ["M/a.mp3", "M/a (2).mp3"]);
    assert_eq!(read(dir.path(), "M/a (2).mp3"), b"2");
}

#[test]
fn copie_d_un_dossier_formats_acceptes_seulement() {
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    write(dir.path(), "Contes/z.mp3", b"deja");
    let contes = src.path().join("Contes");
    write(&contes, "a.mp3", b"a");
    write(&contes, "_image.png", b"p");
    write(&contes, "sous/b.mp3", b"b");
    write(&contes, "sous/x.txt", b"x");
    write(&contes, "_dossier.json", b"{}");
    fs::create_dir_all(contes.join("vide/encore")).unwrap();
    write(&contes, "rien/y.txt", b"y");
    let mut ed = open(dir.path());
    let report = ed.import("", std::slice::from_ref(&contes)).unwrap();
    assert_eq!(report.done.ids, ["Contes (2)"]);
    let got: Vec<String> = snapshot(&dir.path().join("Contes (2)"))
        .into_keys()
        .collect();
    assert_eq!(got, ["_image.png", "a.mp3", "sous", "sous/b.mp3"]);
    let reasons: Vec<&str> = report.skipped.iter().map(|s| s.reason).collect();
    assert_eq!(reasons.len(), 3, "{reasons:?}");
    ed.undo().unwrap();
    assert!(!exists(dir.path(), "Contes (2)"));
    assert_eq!(read(dir.path(), "Contes/z.mp3"), b"deja");
}

#[test]
fn copie_bornee_refusee_sans_rien_ecrire() {
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    write(dir.path(), "z.mp3", b"z");
    write(src.path(), "D/a.mp3", b"aaaa");
    write(src.path(), "D/b.mp3", b"bbbb");
    write(src.path(), "D/s1/s2/c.mp3", b"c");
    let before = snapshot(dir.path());
    let cases = [
        ImportLimits {
            max_files: 2,
            ..ImportLimits::default()
        },
        ImportLimits {
            max_bytes: 6,
            ..ImportLimits::default()
        },
        ImportLimits {
            max_depth: 2,
            ..ImportLimits::default()
        },
        ImportLimits {
            max_entries: 3,
            ..ImportLimits::default()
        },
    ];
    for limits in cases {
        let mut ed = open(dir.path()).with_import_limits(limits.clone());
        let err = ed.import("", &[src.path().join("D")]).unwrap_err();
        assert!(matches!(err, OpError::TooLarge(_)), "{limits:?} : {err:?}");
        assert_eq!(snapshot(dir.path()), before);
        assert!(!ed.can_undo());
    }
}

#[test]
fn copie_refuse_les_sources_dangereuses() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("histoire");
    write(&root, "M/a.mp3", b"a");
    let outside = tempfile::tempdir().unwrap();
    write(outside.path(), "b.mp3", b"b");
    let link = base.path().join("lien");
    junction(&link, outside.path());
    let mut ed = open(&root);
    let before = snapshot(&root);
    for (src, reason) in [
        (link.clone(), "lien, jonction ou fichier « à la demande »"),
        (root.join("M"), "déjà dans le dossier de l'histoire"),
        (root.join("M/a.mp3"), "déjà dans le dossier de l'histoire"),
        (
            base.path().to_path_buf(),
            "contient le dossier de l'histoire",
        ),
        (PathBuf::from("relatif.mp3"), "chemin relatif"),
        (base.path().join("absent.mp3"), "introuvable"),
    ] {
        match ed.import("", std::slice::from_ref(&src)) {
            Err(OpError::Source { reason: got, .. }) => assert_eq!(got, reason, "{src:?}"),
            other => panic!("{src:?} : {other:?}"),
        }
        assert_eq!(snapshot(&root), before);
    }
    // Une jonction dans un dossier déposé est ignorée, sa cible n'est pas copiée.
    let dropped = base.path().join("depot");
    write(&dropped, "c.mp3", b"c");
    junction(&dropped.join("lien"), outside.path());
    let report = ed.import("", &[dropped]).unwrap();
    assert_eq!(report.done.ids, ["depot"]);
    assert!(!exists(&root, "depot/lien"));
    assert!(report.skipped.iter().any(|s| s.path.ends_with("lien")));
}

// ---------------------------------------------------------------------------
// Image
// ---------------------------------------------------------------------------

#[test]
fn remplacer_une_image_garde_l_ancienne_dans_la_corbeille() {
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    write(dir.path(), "a.png", b"ancienne");
    write(dir.path(), "M/b.mp3", b"b");
    write(src.path(), "Nouvelle.JPG", b"nouvelle");
    let mut ed = open(dir.path());

    let done = ed
        .set_image(&target(&ed, "a.mp3"), &src.path().join("Nouvelle.JPG"))
        .unwrap();
    assert_eq!(done.ids, ["a.jpg"]);
    assert_eq!(read(dir.path(), "a.jpg"), b"nouvelle");
    assert!(!exists(dir.path(), "a.png"));
    assert_eq!(
        trash(dir.path()),
        [("a.png".to_owned(), b"ancienne".to_vec())]
    );

    ed.set_image(&target(&ed, "M"), &src.path().join("Nouvelle.JPG"))
        .unwrap();
    assert!(exists(dir.path(), "M/_image.jpg"));
    ed.set_image(&target(&ed, ""), &src.path().join("Nouvelle.JPG"))
        .unwrap();
    assert!(exists(dir.path(), "_couverture.jpg"));

    ed.undo().unwrap();
    ed.undo().unwrap();
    ed.undo().unwrap();
    assert_eq!(read(dir.path(), "a.png"), b"ancienne");
    assert!(!exists(dir.path(), "a.jpg"));
    assert!(!exists(dir.path(), "M/_image.jpg"));
    assert!(!exists(dir.path(), "_couverture.jpg"));
    ed.redo().unwrap();
    assert_eq!(read(dir.path(), "a.jpg"), b"nouvelle");
    assert!(!exists(dir.path(), "a.png"));
    assert!(exists(src.path(), "Nouvelle.JPG"));
}

#[test]
fn image_refusee_si_pas_une_image() {
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    write(src.path(), "x.txt", b"x");
    let mut ed = open(dir.path());
    let err = ed
        .set_image(&target(&ed, "a.mp3"), &src.path().join("x.txt"))
        .unwrap_err();
    assert!(matches!(err, OpError::Source { .. }));
}

// ---------------------------------------------------------------------------
// Ordre, déplacement, renommage
// ---------------------------------------------------------------------------

#[test]
fn reordonner_ecrit_ordre_puis_annuler_met_le_reglage_a_la_corbeille() {
    let dir = tempfile::tempdir().unwrap();
    for n in ["1.mp3", "2.mp3", "3.mp3"] {
        write(dir.path(), &format!("M/{n}"), b"x");
    }
    let mut ed = open(dir.path());
    ed.reorder("M", &["3.MP3".to_owned()]).unwrap();
    assert_eq!(
        json(dir.path(), "M/_dossier.json")["ordre"],
        serde_json::json!(["3.mp3", "1.mp3", "2.mp3"])
    );
    assert_eq!(
        ids(&ed.scan().unwrap(), "M"),
        ["M/3.mp3", "M/1.mp3", "M/2.mp3"]
    );
    ed.undo().unwrap();
    assert!(!exists(dir.path(), "M/_dossier.json"));
    assert_eq!(trash_names(dir.path()), ["_dossier.json"]);
    ed.redo().unwrap();
    assert!(exists(dir.path(), "M/_dossier.json"));

    for bad in [
        vec!["4.mp3".to_owned()],
        vec!["1.mp3".into(), "1.MP3".into()],
    ] {
        assert!(matches!(ed.reorder("M", &bad), Err(OpError::Refused(_))));
    }
}

#[test]
fn reordonner_garde_les_cles_inconnues_et_annuler_rend_les_octets() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    write(dir.path(), "b.mp3", b"b");
    let original = b"{ \"format\": 1, \"inconnu\": [1, 2], \"ordre\": [\"a.mp3\"] }";
    write(dir.path(), STORY_FILE_NAME, original);
    let mut ed = open(dir.path());
    ed.reorder("", &["b.mp3".to_owned()]).unwrap();
    let value = json(dir.path(), STORY_FILE_NAME);
    assert_eq!(value["inconnu"], serde_json::json!([1, 2]));
    assert_eq!(value["ordre"], serde_json::json!(["b.mp3", "a.mp3"]));
    ed.undo().unwrap();
    assert_eq!(read(dir.path(), STORY_FILE_NAME), original);
    assert!(trash(dir.path()).is_empty(), "capture effacée");
}

const STORY_FILE_NAME: &str = "_histoire.json";
const FOLDER_FILE_NAME: &str = "_dossier.json";

#[test]
fn deplacer_une_histoire_avec_image_titre_lu_et_reglages() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "A/x.mp3", b"x");
    write(r, "A/x.jpg", b"xi");
    write(r, "A/x.titre.mp3", b"xt");
    write(r, "A/y.mp3", b"y");
    let a_settings =
        br#"{"format":1,"ordre":["x.mp3","y.mp3"],"titres":{"x.mp3":"Le X"},"autre":true}"#;
    write(r, "A/_dossier.json", a_settings);
    write(r, "B/x.mp3", b"bx");
    write(r, "B/w.mp3", b"bw");
    let before = snapshot(r);
    let mut ed = open(r);
    let done = ed
        .move_item(&target(&ed, "A/x.mp3"), "B", Some("x.mp3"))
        .unwrap();
    assert_eq!(done.ids, ["B/x (2).mp3"]);
    assert_eq!(read(r, "B/x (2).jpg"), b"xi");
    assert_eq!(read(r, "B/x (2).titre.mp3"), b"xt");
    assert!(!exists(r, "A/x.mp3") && !exists(r, "A/x.jpg") && !exists(r, "A/x.titre.mp3"));
    let b = json(r, "B/_dossier.json");
    assert_eq!(
        b["ordre"],
        serde_json::json!(["w.mp3", "x (2).mp3", "x.mp3"])
    );
    assert_eq!(b["titres"], serde_json::json!({"x (2).mp3": "Le X"}));
    let a = json(r, "A/_dossier.json");
    assert_eq!(a["ordre"], serde_json::json!(["y.mp3"]));
    assert!(a.get("titres").is_none());
    assert_eq!(a["autre"], serde_json::json!(true));
    let scan = ed.scan().unwrap();
    assert_eq!(scan.root.find("B/x (2).mp3").unwrap().title(), "Le X");

    ed.undo().unwrap();
    assert_eq!(snapshot(r), before, "annuler rend l'état exact");
    ed.redo().unwrap();
    assert!(exists(r, "B/x (2).mp3"));
    ed.undo().unwrap();
    assert_eq!(snapshot(r), before);
}

#[test]
fn deplacer_un_menu() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "A/S/s.mp3", b"s");
    write(r, "B/b.mp3", b"b");
    let mut ed = open(r);
    let before = snapshot(r);
    for (dest, id) in [
        ("A/S", "A"),
        ("A", "A"),
        ("A/S", "A/S/s.mp3"),
        ("A/S/s.mp3", "B"),
    ] {
        let t = target(&ed, id);
        let err = ed.move_item(&t, dest, None).unwrap_err();
        assert!(
            matches!(err, OpError::Refused(_)),
            "{id} → {dest} : {err:?}"
        );
    }
    assert!(matches!(
        ed.move_item(&target(&ed, ""), "B", None),
        Err(OpError::Refused(_))
    ));
    assert_eq!(snapshot(r), before);
    let done = ed.move_item(&target(&ed, "A/S"), "B", None).unwrap();
    assert_eq!(done.ids, ["B/S"]);
    assert_eq!(read(r, "B/S/s.mp3"), b"s");
    ed.undo().unwrap();
    assert_eq!(snapshot(r), before);
}

#[test]
fn deplacement_defait_si_un_reglage_est_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "A/x.mp3", b"x");
    write(r, "A/x.png", b"xi");
    write(r, "B/b.mp3", b"b");
    write(r, "B/_dossier.json", br#"{"format": 99}"#);
    let before = snapshot(r);
    let mut ed = open(r);
    let err = ed
        .move_item(&target(&ed, "A/x.mp3"), "B", None)
        .unwrap_err();
    assert!(matches!(err, OpError::Settings(_)), "{err:?}");
    assert_eq!(snapshot(r), before);
    assert!(trash(r).is_empty());
    assert!(!ed.can_undo());
}

#[test]
fn renommer_le_titre_puis_le_fichier() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "M/01 Le loup.mp3", b"l");
    write(r, "M/01 Le loup.png", b"li");
    write(r, "M/02 Autre.mp3", b"a");
    write(
        r,
        "M/_dossier.json",
        br#"{"ordre":["02 Autre.mp3","01 Le loup.mp3"]}"#,
    );
    let before = snapshot(r);
    let mut ed = open(r);

    ed.rename(&target(&ed, "M/01 Le loup.mp3"), "Le grand loup", false)
        .unwrap();
    assert_eq!(
        json(r, "M/_dossier.json")["titres"],
        serde_json::json!({"01 Le loup.mp3": "Le grand loup"})
    );
    let done = ed
        .rename(&target(&ed, "M/01 Le loup.mp3"), "Le grand loup", true)
        .unwrap();
    assert_eq!(done.ids, ["M/01 Le grand loup.mp3"]);
    assert_eq!(read(r, "M/01 Le grand loup.png"), b"li");
    let m = json(r, "M/_dossier.json");
    assert_eq!(
        m["ordre"],
        serde_json::json!(["02 Autre.mp3", "01 Le grand loup.mp3"])
    );
    assert!(m.get("titres").is_none(), "titre déduit du nom : {m}");

    // Nom pris : suffixe ; titre avec caractère interdit : gardé dans `titres`.
    write(r, "M/02 Le chat.mp3", b"c");
    ed.rename(&target(&ed, "M/02 Autre.mp3"), "Le chat ?", true)
        .unwrap();
    assert!(exists(r, "M/02 Le chat (2).mp3"));
    assert_eq!(
        json(r, "M/_dossier.json")["titres"],
        serde_json::json!({"02 Le chat (2).mp3": "Le chat ?"})
    );
    fs::remove_file(r.join("M/02 Le chat.mp3")).unwrap();
    ed.undo().unwrap();
    ed.undo().unwrap();
    ed.undo().unwrap();
    assert_eq!(snapshot(r), before);

    // Racine : titre de l'histoire.
    ed.rename(&target(&ed, ""), "Mon histoire", false).unwrap();
    assert_eq!(json(r, STORY_FILE_NAME)["titre"], "Mon histoire");
    assert!(matches!(
        ed.rename(&target(&ed, ""), "x", true),
        Err(OpError::Refused(_))
    ));
}

#[test]
fn renommer_un_menu() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "1 Animaux/a.mp3", b"a");
    let mut ed = open(r);
    let done = ed.rename(&target(&ed, "1 Animaux"), "Bêtes", true).unwrap();
    assert_eq!(done.ids, ["1 Bêtes"]);
    assert!(exists(r, "1 Bêtes/a.mp3"));
    assert!(
        !exists(r, STORY_FILE_NAME),
        "rien à écrire dans les réglages"
    );
    ed.undo().unwrap();
    assert!(exists(r, "1 Animaux/a.mp3"));
}

// ---------------------------------------------------------------------------
// Nouveau menu, suppression
// ---------------------------------------------------------------------------

#[test]
fn nouveau_menu_nom_libre_et_annulation() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.mp3", b"a");
    let mut ed = open(r);
    assert_eq!(ed.new_menu("", "Bonus").unwrap().ids, ["Bonus"]);
    assert_eq!(ed.new_menu("", "bonus").unwrap().ids, ["bonus (2)"]);
    assert_eq!(ed.new_menu("", "CON").unwrap().ids, ["CON_"]);
    assert_eq!(ed.new_menu("Bonus", "../x").unwrap().ids, ["Bonus/x"]);
    assert!(matches!(ed.new_menu("", " .. "), Err(OpError::Refused(_))));
    assert!(matches!(
        ed.new_menu("a.mp3", "x"),
        Err(OpError::Refused(_))
    ));
    ed.undo().unwrap();
    assert!(!exists(r, "Bonus/x"));
    assert!(exists(r, "Bonus"));
}

#[test]
fn supprimer_annuler_retablir() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.mp3", b"a");
    write(r, "a.jpg", b"ai");
    write(r, "a.titre.mp3", b"at");
    write(r, "M/m.mp3", b"m");
    write(r, "M/_image.png", b"mi");
    let before = snapshot(r);
    let mut ed = open(r);

    ed.delete(&target(&ed, "a.mp3")).unwrap();
    assert_eq!(trash_names(r), ["a.jpg", "a.mp3", "a.titre.mp3"]);
    ed.delete(&target(&ed, "M/_image.png")).unwrap();
    ed.delete(&target(&ed, "M")).unwrap();
    assert!(!exists(r, "M"));
    assert_eq!(ed.undo_label(), Some("Suppression de « M »"));
    ed.undo().unwrap();
    ed.undo().unwrap();
    ed.undo().unwrap();
    assert_eq!(snapshot(r), before);
    assert!(trash(r).is_empty());
    assert!(
        fs::read_dir(r.join(TRASH_DIR)).unwrap().next().is_none(),
        "places vides retirées"
    );
    ed.redo().unwrap();
    assert!(!exists(r, "a.mp3"));
    assert!(matches!(
        ed.delete(&Target {
            id: ".flam".into(),
            stamp: ed.stamp("M").unwrap()
        }),
        Err(OpError::InvalidId(_))
    ));
    assert!(matches!(
        ed.delete(&target(&ed, "")),
        Err(OpError::Refused(_))
    ));
}

#[test]
fn image_partagee_par_deux_histoires_reste_en_place() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.mp3", b"a");
    write(r, "a.m4a", b"b");
    write(r, "a.jpg", b"i");
    let mut ed = open(r);
    ed.delete(&target(&ed, "a.mp3")).unwrap();
    assert!(exists(r, "a.jpg"));
    assert_eq!(trash_names(r), ["a.mp3"]);
}

// ---------------------------------------------------------------------------
// Réglages
// ---------------------------------------------------------------------------

#[test]
fn reglages_de_l_histoire_et_d_un_dossier() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "M/a.mp3", b"a");
    write(
        r,
        STORY_FILE_NAME,
        br#"{"format":1,"inconnu":42,"titre":"A"}"#,
    );
    write(r, "M/_dossier.json", br#"{"mode":"roman","x":1}"#);
    let mut ed = open(r);
    let patch: StoryPatch = serde_json::from_str(
        r#"{"description":"  Lu   par moi ","age":6,"titre":null,"mode":"chapitres"}"#,
    )
    .unwrap();
    ed.set_story_settings(&patch).unwrap();
    let story = json(r, STORY_FILE_NAME);
    assert_eq!(story["inconnu"], 42);
    assert_eq!(story["description"], "Lu par moi");
    assert_eq!(story["age"], 6);
    assert_eq!(story["mode"], "chapitres");
    assert!(story.get("titre").is_none());

    // Effacer `mode` retire aussi la valeur brute invalide.
    let clear: FolderPatch = serde_json::from_str(r#"{"mode":null,"fin":"suivante"}"#).unwrap();
    ed.set_folder_settings("M", &clear).unwrap();
    let m = json(r, "M/_dossier.json");
    assert!(m.get("mode").is_none(), "{m}");
    assert_eq!(m["fin"], "suivante");
    assert_eq!(m["x"], 1);

    let bad: StoryPatch = serde_json::from_str(r#"{"age":120}"#).unwrap();
    assert!(matches!(
        ed.set_story_settings(&bad),
        Err(OpError::Refused(_))
    ));
    // Rien à changer : rien d'écrit, rien dans le journal.
    let label = ed.undo_label().map(str::to_owned);
    ed.set_folder_settings("M", &FolderPatch::default())
        .unwrap();
    assert_eq!(ed.undo_label().map(str::to_owned), label);
}

#[test]
fn reglage_abime_sauve_dans_la_corbeille_avant_reecriture() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "M/a.mp3", b"a");
    write(r, "M/b.mp3", b"b");
    let damaged = b"{ pas du json";
    write(r, "M/_dossier.json", damaged);
    let mut ed = open(r);
    ed.reorder("M", &["b.mp3".to_owned()]).unwrap();
    assert_eq!(
        json(r, "M/_dossier.json")["ordre"],
        serde_json::json!(["b.mp3", "a.mp3"])
    );
    assert_eq!(trash(r), [(FOLDER_FILE_NAME.to_owned(), damaged.to_vec())]);
    ed.undo().unwrap();
    assert_eq!(read(r, "M/_dossier.json"), damaged);
}

#[test]
fn format_plus_recent_jamais_reecrit() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.mp3", b"a");
    write(r, STORY_FILE_NAME, br#"{"format":2,"ordre":["a.mp3"]}"#);
    let before = snapshot(r);
    let mut ed = open(r);
    let patch: StoryPatch = serde_json::from_str(r#"{"age":3}"#).unwrap();
    assert!(matches!(
        ed.set_story_settings(&patch),
        Err(OpError::Settings(_))
    ));
    assert_eq!(snapshot(r), before);
}

// ---------------------------------------------------------------------------
// État attendu et journal
// ---------------------------------------------------------------------------

#[test]
fn etat_change_rien_n_est_fait() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.mp3", b"a");
    write(r, "M/m.mp3", b"m");
    let mut ed = open(r);
    let stale = target(&ed, "a.mp3");
    write(r, "a.mp3", b"modifie");
    let before = snapshot(r);
    for err in [
        ed.delete(&stale).unwrap_err(),
        ed.move_item(&stale, "M", None).unwrap_err(),
        ed.rename(&stale, "x", true).unwrap_err(),
    ] {
        assert!(matches!(err, OpError::Changed(_)), "{err:?}");
    }
    let gone = Target {
        id: "absent.mp3".into(),
        stamp: stale.stamp,
    };
    assert!(matches!(ed.delete(&gone), Err(OpError::Changed(_))));
    let dir_as_file = Target {
        id: "M".into(),
        stamp: stale.stamp,
    };
    assert!(matches!(ed.delete(&dir_as_file), Err(OpError::Changed(_))));
    assert_eq!(snapshot(r), before);
    assert!(!ed.can_undo());
}

#[test]
fn annulation_impossible_si_le_fichier_a_change_depuis() {
    let dir = tempfile::tempdir().unwrap();
    let src = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "M/a.mp3", b"a");
    write(r, "M/b.mp3", b"b");
    write(src.path(), "c.mp3", b"c");
    let mut ed = open(r);
    ed.reorder("M", &["b.mp3".to_owned()]).unwrap();
    ed.import("M", &[src.path().join("c.mp3")]).unwrap();

    // La copie a été modifiée : son annulation ne fait rien et sort du journal.
    write(r, "M/c.mp3", b"modifie");
    let err = ed.undo().unwrap_err();
    assert!(matches!(err, OpError::NotUndoable { .. }), "{err:?}");
    assert_eq!(read(r, "M/c.mp3"), b"modifie");
    assert_eq!(ed.undo_label(), Some("Ordre de « M »"));

    // Le réglage a été modifié à la main : idem, le fichier n'est pas touché.
    let hand = br#"{"ordre":["a.mp3"],"main":true}"#;
    write(r, "M/_dossier.json", hand);
    let err = ed.undo().unwrap_err();
    assert!(matches!(err, OpError::NotUndoable { .. }), "{err:?}");
    assert_eq!(read(r, "M/_dossier.json"), hand);
    assert!(!ed.can_undo());
    assert!(matches!(ed.undo(), Err(OpError::NothingToUndo)));
}

#[test]
fn nouvelle_operation_vide_retablir() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "a.mp3", b"a");
    let mut ed = open(r);
    ed.new_menu("", "A").unwrap();
    ed.undo().unwrap();
    assert!(ed.can_redo());
    assert_eq!(ed.redo_label(), Some("Nouveau menu « A »"));
    ed.new_menu("", "B").unwrap();
    assert!(!ed.can_redo());
    assert!(matches!(ed.redo(), Err(OpError::NothingToRedo)));
}

#[test]
fn json_capture_jamais_d_ecrasement_d_un_contenu_inattendu() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "M/a.mp3", b"a");
    write(r, "M/_dossier.json", b"{\"vu\":1}");
    let ed = open(r);
    let action = Action::Json {
        path: "M/_dossier.json".into(),
        before: b"{\"autre\":1}".to_vec(),
        after: b"{}".to_vec(),
    };
    assert!(matches!(ed.execute(&action), Err(OpError::Changed(_))));
    assert_eq!(read(r, "M/_dossier.json"), b"{\"vu\":1}");
}

// ---------------------------------------------------------------------------
// Corbeille Windows
// ---------------------------------------------------------------------------

#[test]
fn corbeille_windows_refusee_hors_disque_fixe() {
    assert!(fsx::recycle_bin_refusal(Path::new(r"\\serveur\partage\x")).is_some());
    assert!(fsx::recycle_bin_refusal(Path::new(r"\\?\C:\x")).is_some());
    let dir = tempfile::tempdir().unwrap();
    let base = crate::authoring::root::simplify(fs::canonicalize(dir.path()).unwrap());
    // Le dossier temporaire est sur un disque fixe.
    assert_eq!(fsx::recycle_bin_refusal(&base), None);
}

/// Envoie un vrai fichier dans la Corbeille Windows de l'utilisateur : lancé à la main.
#[test]
#[ignore = "écrit dans la Corbeille Windows de l'utilisateur"]
fn vider_la_corbeille_vers_la_corbeille_windows() {
    let dir = tempfile::tempdir().unwrap();
    let r = dir.path();
    write(r, "flam-test-corbeille.mp3", b"a");
    let mut ed = open(r);
    ed.delete(&target(&ed, "flam-test-corbeille.mp3")).unwrap();
    assert_eq!(ed.empty_trash().unwrap(), 1);
    assert!(trash(r).is_empty());
    assert!(matches!(ed.undo(), Err(OpError::NotUndoable { .. })));
}
