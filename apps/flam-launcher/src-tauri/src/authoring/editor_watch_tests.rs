use std::fs;
use std::sync::Mutex as StdMutex;

use super::*;
use crate::authoring::ops::{Editor, Target};
use crate::authoring::ops_commands::{run_op, EditorState, Op, Registered};
use crate::authoring::root::ForbiddenRoots;
use crate::authoring::tree::Limits;

const QUIET: Duration = Duration::from_millis(150);

fn timing() -> Timing {
    Timing {
        debounce: QUIET,
        max_wait: Duration::from_secs(3),
        // Large : un runner chargé peut livrer les événements de l'éditeur avec retard.
        grace: Duration::from_millis(1000),
        poll: Duration::from_millis(100),
    }
}

/// Signaux reçus.
type Seen = Arc<StdMutex<Vec<Signal>>>;

fn recorder() -> (Seen, impl Fn(Signal) + Send + 'static) {
    let seen: Seen = Arc::default();
    let s = seen.clone();
    (seen, move |signal| s.lock().unwrap().push(signal))
}

fn count(seen: &Seen, signal: Signal) -> usize {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|s| **s == signal)
        .count()
}

/// Attend (au plus 10 s) que `cond` soit vraie.
fn wait_for(cond: impl Fn() -> bool) -> bool {
    let end = Instant::now() + Duration::from_secs(10);
    while Instant::now() < end {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    cond()
}

/// Laisse passer tout signal en attente (anti-rebond, grâce, présence).
fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

fn write(root: &Path, rel: &str, bytes: &[u8]) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn creation_suppression_et_renommage_dans_un_sous_dossier_sont_signales() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "A/B/x.mp3", b"x");
    let (seen, on) = recorder();
    let _w = EditorWatch::start(dir.path(), timing(), on).unwrap();

    write(dir.path(), "A/B/y.mp3", b"y");
    assert!(wait_for(|| count(&seen, Signal::Changed) >= 1), "création");
    settle();
    let n = count(&seen, Signal::Changed);

    fs::rename(dir.path().join("A/B/y.mp3"), dir.path().join("A/B/z.mp3")).unwrap();
    assert!(wait_for(|| count(&seen, Signal::Changed) > n), "renommage");
    settle();
    let n = count(&seen, Signal::Changed);

    fs::remove_file(dir.path().join("A/B/z.mp3")).unwrap();
    assert!(
        wait_for(|| count(&seen, Signal::Changed) > n),
        "suppression"
    );
    assert_eq!(count(&seen, Signal::Missing), 0);
}

#[test]
fn une_rafale_ne_produit_qu_un_signal() {
    let dir = tempfile::tempdir().unwrap();
    let (seen, on) = recorder();
    // Anti-rebond long : un runner chargé peut suspendre le thread entre deux écritures. On
    // vérifie la coalescence (bien moins de signaux que d'écritures), pas un compte exact.
    let timing = Timing {
        debounce: Duration::from_millis(1500),
        ..timing()
    };
    let _w = EditorWatch::start(dir.path(), timing, on).unwrap();
    for i in 0..10 {
        write(dir.path(), &format!("S/{i}.mp3"), b"x");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(wait_for(|| count(&seen, Signal::Changed) >= 1));
    std::thread::sleep(Duration::from_millis(3000));
    let n = count(&seen, Signal::Changed);
    assert!((1..=3).contains(&n), "{n} signaux pour 10 écritures");
}

#[test]
fn flam_et_temporaires_sont_ignores() {
    let dir = tempfile::tempdir().unwrap();
    let (seen, on) = recorder();
    let _w = EditorWatch::start(dir.path(), timing(), on).unwrap();
    write(dir.path(), ".flam/corbeille/1/x.mp3", b"x");
    write(dir.path(), ".flam/tmp/0/y.mp3", b"y");
    write(dir.path(), "A/.x.mp3.12-3.tmp", b"t");
    write(dir.path(), "A/._dossier.json.12-4.flam-tmp", b"t");
    // La création de « A » elle-même compte : on attend qu'elle soit passée.
    assert!(wait_for(|| count(&seen, Signal::Changed) >= 1));
    settle();
    let n = count(&seen, Signal::Changed);
    write(dir.path(), ".flam/corbeille/2/z.mp3", b"z");
    write(dir.path(), "A/.y.mp3.12-5.tmp", b"t");
    fs::remove_file(dir.path().join("A/.y.mp3.12-5.tmp")).unwrap();
    settle();
    assert_eq!(
        count(&seen, Signal::Changed),
        n,
        "rien de neuf hors .flam et temporaires"
    );
    // Un fichier visible est bien signalé : la surveillance fonctionne.
    write(dir.path(), "A/v.mp3", b"v");
    assert!(wait_for(|| count(&seen, Signal::Changed) > n));
}

#[test]
fn les_changements_faits_pendant_un_silence_sont_ignores() {
    let dir = tempfile::tempdir().unwrap();
    let (seen, on) = recorder();
    let w = EditorWatch::start(dir.path(), timing(), on).unwrap();
    {
        let _q = w.quiet();
        write(dir.path(), "a.mp3", b"a");
        fs::create_dir(dir.path().join("M")).unwrap();
        // Deux silences imbriqués : le premier libéré ne rouvre pas la surveillance.
        let q2 = w.quiet();
        drop(q2);
        write(dir.path(), "M/b.mp3", b"b");
    }
    settle();
    assert_eq!(count(&seen, Signal::Changed), 0);
    // Fin du délai de grâce : un changement d'ailleurs est signalé.
    write(dir.path(), "c.mp3", b"c");
    assert!(wait_for(|| count(&seen, Signal::Changed) >= 1));
}

#[test]
fn abandonner_la_surveillance_l_arrete() {
    let dir = tempfile::tempdir().unwrap();
    let (seen, on) = recorder();
    let w = EditorWatch::start(dir.path(), timing(), on).unwrap();
    assert!(!w.finished());
    let probe = w.quiet();
    drop(w);
    // Un garde encore tenu ne garde pas le thread vivant.
    drop(probe);
    write(dir.path(), "a.mp3", b"a");
    fs::remove_dir_all(dir.path()).ok();
    settle();
    assert!(
        seen.lock().unwrap().is_empty(),
        "{:?}",
        seen.lock().unwrap()
    );
}

#[test]
fn le_thread_finit_avec_drop() {
    let dir = tempfile::tempdir().unwrap();
    let (_seen, on) = recorder();
    let mut w = EditorWatch::start(dir.path(), timing(), on).unwrap();
    let thread = w.thread.take().unwrap();
    drop(w);
    assert!(wait_for(|| thread.is_finished()), "thread arrêté");
}

#[test]
fn dossier_supprime_puis_revenu() {
    let parent = tempfile::tempdir().unwrap();
    let base = parent.path().join("Histoire");
    write(&base, "a.mp3", b"a");
    let (seen, on) = recorder();
    let _w = EditorWatch::start(&base, timing(), on).unwrap();
    fs::remove_dir_all(&base).unwrap();
    assert!(wait_for(|| count(&seen, Signal::Missing) == 1), "disparu");
    settle();
    assert_eq!(count(&seen, Signal::Missing), 1, "signalé une seule fois");
    let n = count(&seen, Signal::Changed);
    // Clé rebranchée : le dossier revient, la surveillance reprend.
    write(&base, "a.mp3", b"a");
    assert!(wait_for(|| count(&seen, Signal::Changed) > n), "revenu");
    settle();
    let n = count(&seen, Signal::Changed);
    write(&base, "b.mp3", b"b");
    assert!(
        wait_for(|| count(&seen, Signal::Changed) > n),
        "surveillé à nouveau"
    );
}

#[test]
fn dossier_renomme_signale_disparu() {
    let parent = tempfile::tempdir().unwrap();
    let base = parent.path().join("Histoire");
    write(&base, "a.mp3", b"a");
    let (seen, on) = recorder();
    let _w = EditorWatch::start(&base, timing(), on).unwrap();
    fs::rename(&base, parent.path().join("Autre nom")).unwrap();
    assert!(wait_for(|| count(&seen, Signal::Missing) == 1));
}

#[test]
fn dossier_absent_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let (_seen, on) = recorder();
    assert!(EditorWatch::start(&dir.path().join("absent"), timing(), on).is_err());
}

#[test]
fn filtre_des_chemins() {
    use notify::event::{AccessKind, CreateKind};
    let base = Path::new("/h");
    let ev = |p: &str| Event::new(EventKind::Create(CreateKind::Any)).add_path(p.into());
    assert!(is_relevant(&ev("/h/a.mp3"), base));
    assert!(is_relevant(&ev("/h/A/b.png"), base));
    assert!(is_relevant(&ev("/h"), base), "la racine elle-même");
    assert!(!is_relevant(&ev("/h/.flam/tmp/1/x"), base));
    assert!(!is_relevant(&ev("/h/A/.x.mp3.1-2.tmp"), base));
    assert!(!is_relevant(&ev("/h/A/Desktop.ini"), base));
    assert!(!is_relevant(&ev("/h/Thumbs.db"), base));
    let mixed = ev("/h/.flam/x").add_path("/h/A/y.mp3".into());
    assert!(is_relevant(&mixed, base));
    let access = Event::new(EventKind::Access(AccessKind::Any)).add_path("/h/a".into());
    assert!(!is_relevant(&access, base));
    assert!(
        is_relevant(&Event::new(EventKind::Any), base),
        "sans chemin"
    );
}

// ---------------------------------------------------------------------------
// Avec l'éditeur (`ops_commands`)
// ---------------------------------------------------------------------------

fn editor(root: &Path) -> Editor {
    Editor::open(
        root,
        ForbiddenRoots::default(),
        Vec::new(),
        Limits::default(),
    )
    .unwrap()
}

/// Éditeur enregistré avec sa surveillance (comme `authoring_editor_open`).
fn watched(root: &Path) -> (EditorState, String, Seen) {
    let state = EditorState::default();
    let ed = editor(root);
    let base = ed.base().to_path_buf();
    let Registered::New(label) = state.register(ed) else {
        panic!("déjà ouvert");
    };
    let (seen, on) = recorder();
    state.set_watch(&label, EditorWatch::start(&base, timing(), on).unwrap());
    (state, label, seen)
}

fn target(state: &EditorState, label: &str, root: &Path, id: &str) -> Target {
    let _ = state.get(label).unwrap();
    Target {
        id: id.to_owned(),
        stamp: editor(root).stamp(id).unwrap(),
    }
}

#[test]
fn les_operations_de_l_editeur_ne_sont_pas_signalees() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "A/x.mp3", b"x");
    let (state, label, seen) = watched(dir.path());
    let menu = Op::NewMenu {
        parent: String::new(),
        name: "Nouveau".into(),
    };
    run_op(&state, &label, menu).unwrap();
    let t = target(&state, &label, dir.path(), "A/x.mp3");
    let rename = Op::Rename {
        target: t,
        title: "Le chat".into(),
        rename_file: true,
    };
    run_op(&state, &label, rename).unwrap();
    run_op(&state, &label, Op::Undo).unwrap();
    let t = target(&state, &label, dir.path(), "A");
    run_op(&state, &label, Op::Delete { target: t }).unwrap();
    settle();
    assert!(
        seen.lock().unwrap().is_empty(),
        "{:?}",
        seen.lock().unwrap()
    );
}

#[test]
fn fenetre_fermee_surveillance_arretee() {
    let dir = tempfile::tempdir().unwrap();
    let (state, label, seen) = watched(dir.path());
    state.remove(&label);
    assert!(state.quiet(&label).is_none());
    write(dir.path(), "a.mp3", b"a");
    settle();
    assert!(seen.lock().unwrap().is_empty());
    // Posée après la fermeture (course avec `authoring_editor_open`) : abandonnée aussitôt.
    let (late, on) = recorder();
    state.set_watch(
        &label,
        EditorWatch::start(dir.path(), timing(), on).unwrap(),
    );
    write(dir.path(), "b.mp3", b"b");
    settle();
    assert!(late.lock().unwrap().is_empty());
}

#[test]
fn journal_coherent_apres_un_changement_externe() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "A/x.mp3", b"x");
    write(dir.path(), "A/y.mp3", b"y");
    let (state, label, seen) = watched(dir.path());
    // Deux opérations dans le journal.
    let t = target(&state, &label, dir.path(), "A/x.mp3");
    let rename = Op::Rename {
        target: t,
        title: "Chat".into(),
        rename_file: true,
    };
    run_op(&state, &label, rename).unwrap();
    let new_menu = Op::NewMenu {
        parent: String::new(),
        name: "M".into(),
    };
    run_op(&state, &label, new_menu).unwrap();
    settle();
    assert!(seen.lock().unwrap().is_empty());

    // Ailleurs, dans l'Explorateur : le fichier renommé est modifié.
    let renamed = fs::read_dir(dir.path().join("A"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .find(|n| n.contains("Chat"))
        .expect("fichier renommé");
    write(dir.path(), &format!("A/{renamed}"), b"modifie ailleurs");
    assert!(wait_for(|| count(&seen, Signal::Changed) >= 1), "signalé");

    // La dernière opération (nouveau menu) se défait ; celle dont le fichier a changé dit
    // « impossible de défaire », ne fait rien et quitte le journal.
    run_op(&state, &label, Op::Undo).unwrap();
    let err = run_op(&state, &label, Op::Undo).unwrap_err();
    assert_eq!(err.kind, "not_undoable");
    assert_eq!(
        fs::read(dir.path().join("A").join(&renamed)).unwrap(),
        b"modifie ailleurs",
        "rien n'a été fait"
    );
    let err = run_op(&state, &label, Op::Undo).unwrap_err();
    assert_eq!(err.kind, "nothing_to_undo");
    // Une opération visant un état périmé est refusée comme « changé ».
    let stale = Target {
        id: "A/y.mp3".into(),
        stamp: editor(dir.path()).stamp("A/y.mp3").unwrap(),
    };
    write(dir.path(), "A/y.mp3", b"plus long qu'avant");
    let err = run_op(&state, &label, Op::Delete { target: stale }).unwrap_err();
    assert_eq!(err.kind, "changed");
}
