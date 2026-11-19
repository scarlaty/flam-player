use std::fs;
use std::path::Path;

use super::*;
use crate::authoring::tree::Limits;

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn editor(root: &Path) -> Editor {
    Editor::open(
        root,
        ForbiddenRoots::default(),
        Vec::new(),
        Limits::default(),
    )
    .unwrap()
}

fn target(state: &EditorState, label: &str, id: &str) -> Target {
    let session = state.get(label).unwrap();
    let stamp = lock(&session).editor.stamp(id).unwrap();
    Target {
        id: id.to_owned(),
        stamp,
    }
}

fn new_label(state: &EditorState, root: &Path) -> String {
    match state.register(editor(root)) {
        Registered::New(label) => label,
        other => panic!("{other:?}"),
    }
}

#[test]
fn une_seule_fenetre_par_dossier() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let state = EditorState::default();
    let first = new_label(&state, a.path());
    assert!(first.starts_with(EDITOR_LABEL_PREFIX));
    assert_eq!(
        state.register(editor(a.path())),
        Registered::Existing(first.clone())
    );
    let second = new_label(&state, b.path());
    assert_ne!(first, second);

    state.remove(&first);
    assert_eq!(state.get(&first).unwrap_err().kind, CLOSED);
    // Fenêtre fermée : le dossier peut être rouvert, sous un nouveau libellé.
    let again = new_label(&state, a.path());
    assert_ne!(again, first);
    assert_eq!(state.labels().len(), 2);
}

#[test]
fn fermee_pendant_une_operation_le_dossier_ne_se_rouvre_qu_a_sa_fin() {
    let dir = tempfile::tempdir().unwrap();
    let state = EditorState::default();
    let label = new_label(&state, dir.path());
    // Opération en cours : la session est encore tenue.
    let running = state.get(&label).unwrap();
    state.remove(&label);
    assert_eq!(state.register(editor(dir.path())), Registered::Closing);
    drop(running);
    assert!(matches!(
        state.register(editor(dir.path())),
        Registered::New(_)
    ));
}

#[test]
fn ouvrir_un_dossier_n_attend_pas_l_operation_d_un_autre() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let state = EditorState::default();
    let label = new_label(&state, a.path());
    let session = state.get(&label).unwrap();
    let _busy = lock(&session);
    // Le verrou de la session A est tenu : enregistrer B ne doit pas l'attendre.
    assert!(matches!(
        state.register(editor(b.path())),
        Registered::New(_)
    ));
}

#[test]
fn une_fenetre_sans_editeur_est_refusee() {
    let state = EditorState::default();
    let err = run_op(&state, "main", Op::Undo).unwrap_err();
    assert_eq!(err.kind, CLOSED);
}

#[test]
fn operations_et_journal_par_la_fenetre() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    let state = EditorState::default();
    let label = new_label(&state, dir.path());

    let done = run_op(
        &state,
        &label,
        Op::NewMenu {
            parent: String::new(),
            name: "Contes".into(),
        },
    )
    .unwrap()
    .done;
    assert_eq!(done.ids, ["Contes"]);
    assert!(dir.path().join("Contes").is_dir());
    let journal = journal_of(&lock(&state.get(&label).unwrap()).editor);
    assert!(journal.can_undo);
    assert!(!journal.can_redo);

    run_op(&state, &label, Op::Undo).unwrap();
    assert!(!dir.path().join("Contes").exists());
    run_op(&state, &label, Op::Redo).unwrap();
    assert!(dir.path().join("Contes").is_dir());

    // Codes stables des erreurs.
    let err = run_op(
        &state,
        &label,
        Op::Delete {
            target: Target {
                id: "../x".into(),
                stamp: target(&state, &label, "a.mp3").stamp,
            },
        },
    )
    .unwrap_err();
    assert_eq!(err.kind, "invalid_id");
    let stale = target(&state, &label, "a.mp3");
    write(dir.path(), "a.mp3", b"changed!");
    let err = run_op(&state, &label, Op::Delete { target: stale }).unwrap_err();
    assert_eq!(err.kind, "changed");
    assert!(dir.path().join("a.mp3").exists());
}

#[test]
fn les_etats_vus_couvrent_images_et_titres_lus() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "M/a.mp3", b"a");
    write(dir.path(), "M/a.png", b"png");
    write(dir.path(), "M/a.titre.mp3", b"t");
    write(dir.path(), "_couverture.jpg", b"jpg");
    let ed = editor(dir.path());
    let scan = ed.scan().unwrap();
    let stamps = stamps_of(&ed, &scan.root);
    for id in [
        "",
        "M",
        "M/a.mp3",
        "M/a.png",
        "M/a.titre.mp3",
        "_couverture.jpg",
    ] {
        assert!(stamps.contains_key(id), "{id}");
    }
    assert!(stamps[""].dir);
    assert_eq!(stamps["M/a.png"].len, 3);
}

#[test]
fn garder_l_image_automatique_d_un_menu() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "M/01 a.mp3", b"a");
    write(dir.path(), "M/02 b.mp3", b"b");
    write(dir.path(), "M/02 b.png", b"image b");
    write(dir.path(), "N/c.mp3", b"c");
    write(dir.path(), "N/_image.png", b"propre");
    let ed = editor(dir.path());
    let root = ed.scan().unwrap().root;
    assert_eq!(keep_source(&root, "M").as_deref(), Some("M/02 b.png"));
    // Couverture : première image d'histoire.
    assert_eq!(keep_source(&root, "").as_deref(), Some("M/02 b.png"));
    // Menu qui a déjà son image, histoire : rien à garder.
    assert_eq!(keep_source(&root, "N"), None);
    assert_eq!(keep_source(&root, "M/01 a.mp3"), None);

    let state = EditorState::default();
    let label = new_label(&state, dir.path());
    let done = run_op(
        &state,
        &label,
        Op::Keep {
            target: target(&state, &label, "M"),
        },
    )
    .unwrap()
    .done;
    assert_eq!(done.ids, ["M/_image.png"]);
    assert_eq!(
        fs::read(dir.path().join("M/_image.png")).unwrap(),
        b"image b"
    );
    assert_eq!(fs::read(dir.path().join("M/02 b.png")).unwrap(), b"image b");
    let err = run_op(
        &state,
        &label,
        Op::Keep {
            target: target(&state, &label, "N"),
        },
    )
    .unwrap_err();
    assert_eq!(err.kind, "refused");
    run_op(&state, &label, Op::Undo).unwrap();
    assert!(!dir.path().join("M/_image.png").exists());
}

#[test]
fn vignette_seulement_pour_une_image_de_l_histoire() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.mp3", b"a");
    write(dir.path(), "a.png", b"png");
    write(dir.path(), ".flam/x.png", b"png");
    let base = dir.path();
    assert!(image_path(base, "a.png").is_ok());
    for id in ["a.mp3", "", "../a.png", ".flam/x.png", "C:/x.png", "a.png/"] {
        assert!(image_path(base, id).is_err(), "{id}");
    }
    assert_eq!(image_path(base, "../a.png").unwrap_err().kind, "invalid_id");
}

#[test]
fn sources_absolues_seulement() {
    assert!(sources(Vec::new()).is_err());
    let err = sources(vec!["relatif.mp3".into()]).unwrap_err();
    assert_eq!(err.kind, "source_refused");
    let abs = std::env::temp_dir().join("x.mp3");
    assert_eq!(
        sources(vec![abs.to_string_lossy().into_owned()]).unwrap(),
        [abs]
    );
}

#[test]
fn l_erreur_a_la_forme_d_app_error() {
    let err = EditorError::from(OpError::NothingToUndo);
    let json = serde_json::to_value(&err).unwrap();
    assert_eq!(
        json,
        serde_json::json!({ "kind": "nothing_to_undo", "message": "rien à annuler" })
    );
    let app = EditorError::from(AppError::NotFound("x".into()));
    assert_eq!(app.kind, "not_found");
}

/// #300 : le dépôt de fichiers natif n'est actif que sur le modèle de la fenêtre de
/// l'éditeur, qui n'est pas créé au démarrage.
#[test]
fn seul_le_modele_de_l_editeur_accepte_le_depot_de_fichiers() {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    let windows = raw["app"]["windows"].as_array().unwrap();
    let editor = windows
        .iter()
        .find(|w| w["label"] == EDITOR_TEMPLATE)
        .expect("modèle de la fenêtre de l'éditeur");
    assert_eq!(editor["dragDropEnabled"], true);
    assert_eq!(editor["create"], false);
    for window in windows.iter().filter(|w| w["label"] != EDITOR_TEMPLATE) {
        assert_eq!(window["dragDropEnabled"], false, "{window}");
    }
}
