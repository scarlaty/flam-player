//! Modes recueil et chapitres (`docs/creation-dossier.md` § 5, lot 3) : graphe produit,
//! réglages `fin`, `debut`, `annoncer`, bornes en paramètre.

use super::*;

fn sub(name: &str, entries: Vec<MenuEntry>, mode: MenuMode) -> MenuEntry {
    MenuEntry::Submenu {
        name: name.to_string(),
        entries,
        audio: None,
        image: None,
        mode,
    }
}

fn chapters(start: ChapterStart, end: BookEnd, announce: bool) -> MenuMode {
    MenuMode::Chapters(ChapterSettings {
        start,
        end,
        announce,
    })
}

fn book() -> MenuMode {
    MenuMode::Chapters(ChapterSettings::default())
}

fn collection(end: CollectionEnd) -> MenuMode {
    MenuMode::Collection { end }
}

fn options(mode: MenuMode) -> ArchiveOptions {
    ArchiveOptions {
        mode,
        limits: Limits::PODCAST,
    }
}

/// Écrit le pack avec ces réglages ; rend les entrées de l'archive, après [`assert_graph`].
fn build(fx: &Fixture, menu: Vec<MenuEntry>, opts: ArchiveOptions) -> BTreeMap<String, Vec<u8>> {
    write_archive_with(&pack(menu, None), &opts, &fx.out(), &never).unwrap();
    let files = read_zip(&fx.out());
    assert_graph(&json_of(&files, "nodes.json"));
    files
}

/// Invariants du graphe pour `story.lua`, sur tout `nodes.json` :
/// - `startAction`, chaque `ok` et chaque `home` mènent à une action existante, avec un
///   `index` dans ses bornes ou un `indexItem` qui désigne un objet de l'inventaire ;
/// - chaque entrée d'action désigne une scène existante ;
/// - un `home` n'est jamais ignoré par `homeOf` (`control.home` vrai, action cible qui ne
///   contient pas la scène) ;
/// - seule une scène de lecture (`eN`) peut avoir `ok: null` (fin d'un livre `fin = fin`) ;
///   une option a toujours une suite ;
/// - chaque scène est atteinte par au moins une action.
pub(super) fn assert_graph(nodes: &Value) {
    let stages = nodes["stages"].as_object().expect("stages");
    let actions = nodes["actions"].as_object().expect("actions");
    let inventory = nodes
        .get("inventory")
        .map_or(0, |i| i.as_array().unwrap().len());
    let check = |t: &Value, what: &str| {
        let action = t["action"]
            .as_str()
            .unwrap_or_else(|| panic!("{what} : {t}"));
        let list = actions[action]
            .as_array()
            .unwrap_or_else(|| panic!("{what} : action {action} absente"));
        assert!(!list.is_empty(), "{what} : action {action} vide");
        match (t.get("index"), t.get("indexItem")) {
            (Some(i), None) => {
                assert!((i.as_u64().unwrap() as usize) < list.len(), "{what} : {t}")
            }
            (None, Some(i)) => {
                assert!((i.as_u64().unwrap() as usize) < inventory, "{what} : {t}")
            }
            _ => panic!("{what} : index et indexItem exclusifs : {t}"),
        }
        list.iter()
            .map(|e| e["stage"].as_str().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    check(&nodes["startAction"], "startAction");
    let mut reached = HashSet::new();
    for (aid, list) in actions {
        for e in list.as_array().unwrap() {
            let sid = e["stage"].as_str().unwrap();
            assert!(
                stages.contains_key(sid),
                "action {aid} : scène {sid} absente"
            );
            reached.insert(sid.to_string());
        }
    }
    for (sid, st) in stages {
        assert!(reached.contains(sid), "scène {sid} jamais atteinte");
        match &st["ok"] {
            Value::Null => assert!(
                sid.starts_with('e') && st["audio"].is_string(),
                "{sid} : ok null hors fin de livre"
            ),
            ok => {
                check(ok, &format!("{sid}.ok"));
            }
        }
        if !st["home"].is_null() {
            assert_eq!(st["control"]["home"], true, "{sid} : home inactif");
            let targets = check(&st["home"], &format!("{sid}.home"));
            assert!(
                !targets.iter().any(|t| t == sid),
                "{sid} : home vers une action qui la contient (ignoré par homeOf)"
            );
        }
    }
}

impl Nodes {
    /// Transition `ok` ou `home` brute (objet ou `null`).
    fn raw(&self, stage: &str, key: &str) -> &Value {
        &self.stage(stage)[key]
    }
    /// Scène de départ, après la scène d'entrée : (action, index).
    fn start(&self) -> (String, usize) {
        let action = self.0["startAction"]["action"].as_str().unwrap();
        let start = &self.action(action)[0];
        self.target(start, "ok")
    }
    /// Scène jouée en entrant dans `action` (action à une scène).
    fn single(&self, action: &str) -> String {
        let list = self.action(action);
        assert_eq!(list.len(), 1, "{action} : une scène");
        list[0].clone()
    }
    /// Chapitre (ou annonce) qui suit `stage` : scène jouée par son `ok`.
    fn next(&self, stage: &str) -> String {
        let (action, index) = self.target(stage, "ok");
        assert_eq!(index, 0);
        self.single(&action)
    }
}

/// Option du premier niveau d'un livre racine : retour par défaut du moteur (sortie).
fn assert_exit(nodes: &Nodes, option: &str) {
    assert!(nodes.raw(option, "home").is_null(), "{option}");
    assert_eq!(nodes.stage(option)["control"]["home"], false, "{option}");
}

/// Graphe sans inventaire ni objet posé par une scène.
fn assert_no_inventory(nodes: &Nodes) {
    assert!(nodes.0.get("inventory").is_none());
    for (sid, st) in nodes.0["stages"].as_object().unwrap() {
        assert!(st.get("items").is_none(), "{sid}");
    }
}

// --- recueil -------------------------------------------------------------------------------------

/// Racine : A, X[B, C], D.
fn small_collection(fx: &mut Fixture) -> Vec<MenuEntry> {
    let a = fx.episode("A");
    let b = fx.episode("B");
    let c = fx.episode("C");
    let d = fx.episode("D");
    vec![a, cat("X", vec![b, c]), d]
}

#[test]
fn recueil_fin_menu_revient_au_menu_sur_l_histoire_suivante() {
    let mut fx = Fixture::new();
    let menu = small_collection(&mut fx);
    let files = build(&fx, menu, options(collection(CollectionEnd::Menu)));
    let nodes = Nodes::of(&files);
    let root = nodes.menu_of_root();
    let r = nodes.action(&root);
    let (a, d) = (nodes.played_by(&r[0]), nodes.played_by(&r[2]));
    // A → menu racine focalisé sur D (la catégorie X est sautée) ; D, dernier, sur lui-même.
    assert_eq!(nodes.after(&a), Err((root.clone(), 2)));
    assert_eq!(nodes.after(&d), Err((root.clone(), 2)));
    // La catégorie hérite du réglage.
    let (x, _) = nodes.target(&r[1], "ok");
    let xo = nodes.action(&x);
    let (b, c) = (nodes.played_by(&xo[0]), nodes.played_by(&xo[1]));
    assert_eq!(nodes.after(&b), Err((x.clone(), 1)));
    assert_eq!(nodes.after(&c), Err((x.clone(), 1)));
    // Retours inchangés : épisode → son option, sous-menu → catégorie dans le parent.
    assert_eq!(nodes.target(&b, "home"), (x.clone(), 0));
    assert_eq!(nodes.target(&xo[0], "home"), (root.clone(), 1));
    assert_eq!(nodes.stage(&r[0])["control"]["home"], false);
    assert!(nodes.0.get("inventory").is_none());
}

#[test]
fn recueil_fin_suivante_explicite_identique_aux_podcasts() {
    let mut fx = Fixture::new();
    let mut menu = small_collection(&mut fx);
    speak_all(&mut fx, &mut menu, &[]);
    let p = pack(menu, None);
    write_archive(&p, &fx.out(), &never).unwrap();
    let before = read_zip(&fx.out());
    write_archive_with(
        &p,
        &options(collection(CollectionEnd::Next)),
        &fx.out(),
        &never,
    )
    .unwrap();
    let after = read_zip(&fx.out());
    assert_eq!(before, after);
    assert_eq!(ArchiveOptions::default(), options(MenuMode::PODCAST));
}

#[test]
fn sous_recueil_avec_son_propre_reglage_fin() {
    let mut fx = Fixture::new();
    let (a, b, c, d) = (
        fx.episode("A"),
        fx.episode("B"),
        fx.episode("C"),
        fx.episode("D"),
    );
    // Racine `suivante`, sous-menu `menu`.
    let menu = vec![a, b, sub("X", vec![c, d], collection(CollectionEnd::Menu))];
    let files = build(&fx, menu, options(MenuMode::PODCAST));
    let nodes = Nodes::of(&files);
    let root = nodes.menu_of_root();
    let r = nodes.action(&root);
    let (a, b) = (nodes.played_by(&r[0]), nodes.played_by(&r[1]));
    assert_eq!(nodes.after(&a), Ok(b.clone()));
    let (x, _) = nodes.target(&r[2], "ok");
    let xo = nodes.action(&x);
    let c = nodes.played_by(&xo[0]);
    assert_eq!(nodes.after(&c), Err((x.clone(), 1)));
    assert_eq!(nodes.target(&xo[0], "home"), (root, 2));
}

// --- livre racine --------------------------------------------------------------------------------

#[test]
fn livre_simple_debut_chapitre_fin_menu() {
    let mut fx = Fixture::new();
    let menu = fx.episodes(3);
    let files = build(&fx, menu, options(book()));
    let nodes = Nodes::of(&files);
    // Démarrage : premier chapitre directement.
    let (p1, _) = nodes.start();
    let e1 = nodes.single(&p1);
    assert_eq!(nodes.stage(&e1)["audio"], "e001.mp3");
    let e2 = nodes.next(&e1);
    let e3 = nodes.next(&e2);
    assert_eq!(nodes.stage(&e3)["audio"], "e003.mp3");
    // Fin du livre : menu des chapitres, sur le premier.
    let menu = ROOT_MENU.to_string();
    assert_eq!(nodes.target(&e3, "ok"), (menu.clone(), 0));
    let o = nodes.action(&menu);
    assert_eq!(o.len(), 3);
    for (i, (opt, ch)) in o.iter().zip([&e1, &e2, &e3]).enumerate() {
        // Option → ce chapitre, puis la suite.
        assert_eq!(&nodes.played_by(opt), ch);
        // Retour pendant un chapitre : son option, focalisée.
        assert_eq!(nodes.target(ch, "home"), (menu.clone(), i));
        assert_eq!(
            nodes.stage(ch)["control"],
            json!({"ok": true, "home": true, "autoplay": true})
        );
        // Retour depuis le menu des chapitres : sortie de l'histoire (menu Start).
        assert_exit(&nodes, opt);
        // Rôles lus par le moteur : chapitre repris à sa position, option hors jauge.
        assert_eq!(nodes.stage(ch)["role"], "chapter");
        assert_eq!(nodes.stage(opt)["role"], "title");
    }
    assert_no_inventory(&nodes);
    // Entrée + 3 options + 3 chapitres, rien d'autre.
    assert_eq!(nodes.0["stages"].as_object().unwrap().len(), 7);
    let notes = json_of(&files, "notes.json");
    assert_eq!(notes[&o[0]]["title"], "Épisode n°1");
}

#[test]
fn livre_debut_menu_fin_d_histoire() {
    let mut fx = Fixture::new();
    let menu = fx.episodes(2);
    let files = build(
        &fx,
        menu,
        options(chapters(ChapterStart::Menu, BookEnd::End, false)),
    );
    let nodes = Nodes::of(&files);
    assert_eq!(nodes.start(), (ROOT_MENU.to_string(), 0));
    let o = nodes.action(ROOT_MENU);
    // Retour sur le menu d'ouverture : sortie, pas de chapitre lancé.
    for opt in &o {
        assert_exit(&nodes, opt);
    }
    let (e1, e2) = (nodes.played_by(&o[0]), nodes.played_by(&o[1]));
    assert_eq!(nodes.next(&e1), e2);
    // Fin du livre : fin d'histoire.
    assert!(nodes.raw(&e2, "ok").is_null());
    assert_eq!(nodes.stage(&e2)["control"]["ok"], true);
}

/// Racine : P, X[A, B], Y[C, Z[D]].
fn parts(fx: &mut Fixture, z_mode: Option<MenuMode>) -> Vec<MenuEntry> {
    let p = fx.episode("P");
    let a = fx.episode("A");
    let b = fx.episode("B");
    let c = fx.episode("C");
    let d = fx.episode("D");
    let z = match z_mode {
        Some(mode) => sub("Z", vec![d], mode),
        None => cat("Z", vec![d]),
    };
    vec![p, cat("X", vec![a, b]), cat("Y", vec![c, z])]
}

#[test]
fn livre_en_parties_lecture_en_profondeur() {
    let mut fx = Fixture::new();
    let menu = parts(&mut fx, None);
    let files = build(&fx, menu, options(book()));
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let title = |stage: &str| notes[stage]["title"].as_str().unwrap().to_string();

    // Lecture : P, A, B, C, D, les parties traversées sans arrêt.
    let (p1, _) = nodes.start();
    let mut order = vec![nodes.single(&p1)];
    while let Some(t) = nodes.raw(order.last().unwrap(), "ok").get("action") {
        let t = t.as_str().unwrap();
        if t == ROOT_MENU {
            break;
        }
        order.push(nodes.single(t));
    }
    assert_eq!(
        order.iter().map(|s| title(s)).collect::<Vec<_>>(),
        ["P", "A", "B", "C", "D"]
    );
    let last = order.last().unwrap();
    assert_eq!(nodes.target(last, "ok"), (ROOT_MENU.to_string(), 0));
    assert_no_inventory(&nodes);

    // Menu des chapitres : un niveau par partie.
    let r = nodes.action(ROOT_MENU);
    assert_eq!(
        r.iter().map(|o| title(o)).collect::<Vec<_>>(),
        ["P", "X", "Y"]
    );
    for o in &r {
        assert_exit(&nodes, o);
    }
    let (x, xi) = nodes.target(&r[1], "ok");
    assert_eq!(xi, 0);
    let xo = nodes.action(&x);
    for o in &xo {
        // Retour depuis une partie : niveau du dessus, focalisé sur elle.
        assert_eq!(nodes.target(o, "home"), (ROOT_MENU.to_string(), 1));
    }
    // A : retour vers son option dans sa partie ; valider B la démarre.
    assert_eq!(nodes.target(&order[1], "home"), (x.clone(), 0));
    assert_eq!(nodes.played_by(&xo[1]), order[2]);
    let (y, _) = nodes.target(&r[2], "ok");
    let yo = nodes.action(&y);
    let (z, _) = nodes.target(&yo[1], "ok");
    let zo = nodes.action(&z);
    assert_eq!(nodes.target(&zo[0], "home"), (y.clone(), 1));
    assert_eq!(nodes.target(&order[4], "home"), (z, 0));
    // Image d'une partie : son premier chapitre (N5 a).
    assert_eq!(nodes.stage(&r[1])["image"], "e002.png");
    assert_eq!(nodes.stage(&r[2])["image"], "e004.png");
}

#[test]
fn sous_livre_dans_un_livre_est_une_partie() {
    let mut fx = Fixture::new();
    // Z déclare son propre mode chapitres (debut menu, fin fin) : seul `annoncer` compte.
    let menu = parts(
        &mut fx,
        Some(chapters(ChapterStart::Menu, BookEnd::End, false)),
    );
    let with_sub = build(&fx, menu, options(book()));
    let mut fx2 = Fixture::new();
    let menu = parts(&mut fx2, None);
    let with_cat = build(&fx2, menu, options(book()));
    assert_eq!(
        json_of(&with_sub, "nodes.json"),
        json_of(&with_cat, "nodes.json")
    );
}

#[test]
fn recueil_dans_un_livre_est_refuse() {
    let mut fx = Fixture::new();
    let (a, b) = (fx.episode("A"), fx.episode("B"));
    let menu = vec![a, sub("Bonus", vec![b], MenuMode::PODCAST)];
    let err =
        write_archive_with(&pack(menu, None), &options(book()), &fx.out(), &never).unwrap_err();
    assert!(
        matches!(&err, ArchiveError::CollectionInBook(n) if n == "Bonus"),
        "{err:?}"
    );
    assert!(!fx.out().exists());
}

// --- livre dans un recueil (Q1) ------------------------------------------------------------------

/// Racine (recueil, podcasts) : E1, Livre[C1, Partie[C2, C3]], E2.
fn book_in_collection(fx: &mut Fixture, mode: MenuMode) -> Vec<MenuEntry> {
    let e1 = fx.episode("E1");
    let c1 = fx.episode("C1");
    let c2 = fx.episode("C2");
    let c3 = fx.episode("C3");
    let e2 = fx.episode("E2");
    vec![
        e1,
        sub("Livre", vec![c1, cat("Partie", vec![c2, c3])], mode),
        e2,
    ]
}

#[test]
fn livre_dans_un_recueil_debut_chapitre() {
    let mut fx = Fixture::new();
    let menu = book_in_collection(&mut fx, book());
    let files = build(&fx, menu, options(MenuMode::PODCAST));
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let title = |stage: &str| notes[stage]["title"].as_str().unwrap().to_string();
    let root = nodes.menu_of_root();
    let r = nodes.action(&root);
    // Recueil inchangé autour du livre : E1 enchaîne sur E2 (le livre est sauté).
    let (e1, e2) = (nodes.played_by(&r[0]), nodes.played_by(&r[2]));
    assert_eq!(nodes.after(&e1), Ok(e2));
    // L'option du livre démarre son premier chapitre.
    let c1 = nodes.played_by(&r[1]);
    assert_eq!(title(&c1), "C1");
    let c2 = nodes.next(&c1);
    let c3 = nodes.next(&c2);
    assert_eq!(title(&c3), "C3");
    // Menu des chapitres du livre : action de la partie de C1.
    let (bm, i) = nodes.target(&c1, "home");
    assert_eq!(i, 0);
    assert_eq!(nodes.target(&c3, "ok"), (bm.clone(), 0));
    let bo = nodes.action(&bm);
    for o in &bo {
        // Retour depuis le menu des chapitres : le recueil, focalisé sur le livre.
        assert_eq!(nodes.target(o, "home"), (root.clone(), 1));
    }
    let (pm, _) = nodes.target(&bo[1], "ok");
    for o in nodes.action(&pm) {
        assert_eq!(nodes.target(&o, "home"), (bm.clone(), 1));
    }
    assert_no_inventory(&nodes);
    // Rôles : sur les scènes du livre seulement, jamais sur celles du recueil.
    assert_eq!(nodes.stage(&c1)["role"], "chapter");
    assert_eq!(nodes.stage(&bo[0])["role"], "title");
    for st in [&r[0], &r[1], &r[2], &nodes.played_by(&r[0])] {
        assert!(nodes.stage(st).get("role").is_none(), "{st}");
    }
    // Image de l'option du livre : son premier chapitre.
    assert_eq!(nodes.stage(&r[1])["image"], "e002.png");
}

#[test]
fn livre_dans_un_recueil_debut_menu_fin_fin() {
    let mut fx = Fixture::new();
    let menu = book_in_collection(&mut fx, chapters(ChapterStart::Menu, BookEnd::End, false));
    let files = build(&fx, menu, options(MenuMode::PODCAST));
    let nodes = Nodes::of(&files);
    let r = nodes.action(&nodes.menu_of_root());
    let (bm, i) = nodes.target(&r[1], "ok");
    assert_eq!(i, 0);
    let bo = nodes.action(&bm);
    assert!(bo.len() == 2, "C1 et la partie");
    let c1 = nodes.played_by(&bo[0]);
    let c3 = nodes.next(&nodes.next(&c1));
    assert!(nodes.raw(&c3, "ok").is_null(), "fin d'histoire");
}

// --- annonce -------------------------------------------------------------------------------------

#[test]
fn annoncer_joue_le_titre_lu_avant_le_chapitre() {
    let mut fx = Fixture::new();
    let mut menu = fx.episodes(3);
    // Chapitre 2 sans titre lu : pas d'annonce.
    speak_all(&mut fx, &mut menu, &["Épisode n°2"]);
    let files = build(
        &fx,
        menu,
        options(chapters(ChapterStart::Chapter, BookEnd::Menu, true)),
    );
    let nodes = Nodes::of(&files);
    let o = nodes.action(ROOT_MENU);
    // Démarrage : annonce du chapitre 1, puis le chapitre.
    let (p1, _) = nodes.start();
    let n1 = nodes.single(&p1);
    let st = nodes.stage(&n1);
    assert_eq!(st["role"], "title", "annonce hors jauge");
    assert_eq!(st["audio"], "t001.mp3");
    assert_eq!(st["image"], "e001.png");
    assert_eq!(
        st["control"],
        json!({"ok": true, "home": true, "autoplay": true})
    );
    assert_eq!(nodes.target(&n1, "home"), (ROOT_MENU.to_string(), 0));
    let e1 = nodes.next(&n1);
    assert_eq!(nodes.stage(&e1)["audio"], "e001.mp3");
    // L'option mène au chapitre sans l'annonce (son titre lu vient d'être entendu).
    assert_eq!(nodes.played_by(&o[0]), e1);
    assert_eq!(nodes.played_by(&o[2]), "e3");
    // Chapitre 2 : pas de titre lu, joué directement ; chapitre 3 : annoncé.
    let e2 = nodes.next(&e1);
    assert_eq!(nodes.stage(&e2)["audio"], "e002.mp3");
    let n3 = nodes.next(&e2);
    assert_eq!(nodes.stage(&n3)["audio"], "t003.mp3");
    let e3 = nodes.next(&n3);
    assert_eq!(nodes.stage(&e3)["audio"], "e003.mp3");
    // Le titre lu sert à l'option et à l'annonce : une seule copie dans l'archive.
    assert_eq!(nodes.stage(&o[0])["audio"], "t001.mp3");
    let audios: Vec<&String> = files.keys().filter(|k| k.starts_with("audios/")).collect();
    assert_eq!(audios.len(), 3 + 2, "{audios:?}");
}

#[test]
fn sans_annoncer_le_titre_lu_reste_sur_l_option() {
    let mut fx = Fixture::new();
    let mut menu = fx.episodes(2);
    speak_all(&mut fx, &mut menu, &[]);
    let files = build(&fx, menu, options(book()));
    let nodes = Nodes::of(&files);
    let (p1, _) = nodes.start();
    assert_eq!(nodes.stage(&nodes.single(&p1))["audio"], "e001.mp3");
    let o = nodes.action(ROOT_MENU);
    assert_eq!(nodes.stage(&o[0])["audio"], "t001.mp3");
    assert!(nodes.0["stages"]
        .as_object()
        .unwrap()
        .keys()
        .all(|k| !k.starts_with('n')));
}

#[test]
fn annoncer_par_partie() {
    let mut fx = Fixture::new();
    let (a, b) = (fx.episode("A"), fx.episode("B"));
    let mut menu = vec![
        a,
        sub(
            "Partie",
            vec![b],
            chapters(ChapterStart::Chapter, BookEnd::Menu, true),
        ),
    ];
    speak_all(&mut fx, &mut menu, &[]);
    let files = build(&fx, menu, options(book()));
    let nodes = Nodes::of(&files);
    let (p1, _) = nodes.start();
    let a = nodes.single(&p1);
    assert_eq!(nodes.stage(&a)["audio"], "e001.mp3", "A : pas d'annonce");
    let nb = nodes.next(&a);
    assert_eq!(nodes.stage(&nb)["audio"], "t002.mp3", "B : annoncé");
}

// --- cas limites ---------------------------------------------------------------------------------

#[test]
fn livre_d_un_seul_chapitre() {
    for end in [BookEnd::Menu, BookEnd::End] {
        let mut fx = Fixture::new();
        let menu = fx.episodes(1);
        let files = build(
            &fx,
            menu,
            options(chapters(ChapterStart::Chapter, end, false)),
        );
        let nodes = Nodes::of(&files);
        let (p1, _) = nodes.start();
        let e1 = nodes.single(&p1);
        // Menu à une entrée : le moteur montre l'option seule (scène qui attend OK).
        let o = nodes.action(ROOT_MENU);
        assert_eq!(o.len(), 1);
        assert_eq!(nodes.played_by(&o[0]), e1);
        assert_exit(&nodes, &o[0]);
        match end {
            BookEnd::Menu => assert_eq!(nodes.target(&e1, "ok"), (ROOT_MENU.to_string(), 0)),
            BookEnd::End => assert!(nodes.raw(&e1, "ok").is_null()),
        }
    }
}

#[test]
fn recueil_fin_menu_d_un_seul_episode() {
    let mut fx = Fixture::new();
    let menu = fx.episodes(1);
    let files = build(&fx, menu, options(collection(CollectionEnd::Menu)));
    let nodes = Nodes::of(&files);
    let root = nodes.menu_of_root();
    let e = nodes.played_by(&nodes.action(&root)[0]);
    assert_eq!(nodes.after(&e), Err((root, 0)));
}

#[test]
fn sous_menu_vide_refuse() {
    for (mode, opts) in [
        (book(), options(MenuMode::PODCAST)),
        (book(), options(book())),
        (MenuMode::PODCAST, options(MenuMode::PODCAST)),
    ] {
        let mut fx = Fixture::new();
        let menu = vec![fx.episode("A"), sub("Vide", vec![], mode)];
        let err = write_archive_with(&pack(menu, None), &opts, &fx.out(), &never).unwrap_err();
        assert!(
            matches!(&err, ArchiveError::EmptyCategory(n) if n == "Vide"),
            "{err:?}"
        );
        assert!(!fx.out().exists());
    }
}

#[test]
fn image_choisie_d_une_partie_et_d_un_livre() {
    let mut fx = Fixture::new();
    // Racine (recueil) : E1, Livre[C1, Partie[C2, C3]], E2 ; rangs 1 à 5.
    let mut menu = book_in_collection(&mut fx, book());
    let (c1, c3) = (episode_image(&menu, 2), episode_image(&menu, 4));
    if let MenuEntry::Submenu { image, entries, .. } = &mut menu[1] {
        *image = Some(c3);
        // Image de la partie : C1, hors de la partie → automatique (C2).
        if let MenuEntry::Category { image, .. } = &mut entries[1] {
            *image = Some(c1);
        }
    }
    let files = build(&fx, menu, options(MenuMode::PODCAST));
    let nodes = Nodes::of(&files);
    let r = nodes.action(&nodes.menu_of_root());
    assert_eq!(nodes.stage(&r[1])["image"], "e004.png");
    let (bm, _) = nodes.target(&nodes.played_by(&r[1]), "home");
    let bo = nodes.action(&bm);
    assert_eq!(nodes.stage(&bo[1])["image"], "e003.png");
}

#[test]
fn livre_dans_un_recueil_a_la_profondeur_maximale() {
    // Recueil (1) → livre (2) → partie (3) : accepté ; une sous-partie (4) : refusée.
    let mut fx = Fixture::new();
    let (a, b) = (fx.episode("A"), fx.episode("B"));
    let menu = vec![sub("Livre", vec![a, cat("Partie", vec![b])], book())];
    build(&fx, menu, options(MenuMode::PODCAST));
    let c = fx.episode("C");
    let deep = vec![sub(
        "Livre",
        vec![cat("Partie", vec![cat("Sous-partie", vec![c])])],
        book(),
    )];
    let err = write_archive_with(
        &pack(deep, None),
        &options(MenuMode::PODCAST),
        &fx.out(),
        &never,
    )
    .unwrap_err();
    assert!(matches!(err, ArchiveError::TooDeep), "{err:?}");
}

// --- bornes en paramètre -------------------------------------------------------------------------

fn nested(fx: &mut Fixture, levels: usize) -> Vec<MenuEntry> {
    let mut menu = vec![fx.episode("x")];
    for l in 1..levels {
        menu = vec![cat(&format!("n{l}"), menu)];
    }
    menu
}

#[test]
fn bornes_parametrees() {
    let folder = Limits {
        max_menu_depth: 5,
        max_options_per_menu: 50,
        max_episodes: 300,
        max_stages: 1000,
        max_audio_bytes: MAX_AUDIO_BYTES,
        max_total_audio_bytes: 3 * 1024 * 1024 * 1024,
    };
    let with = |mode, limits| ArchiveOptions { mode, limits };
    let mut fx = Fixture::new();

    // Profondeur : 5 niveaux acceptés par ces bornes, refusés par celles des podcasts.
    let deep = nested(&mut fx, 5);
    let p = pack(deep, None);
    let err = write_archive(&p, &fx.out(), &never).unwrap_err();
    assert!(matches!(err, ArchiveError::TooDeep), "{err:?}");
    write_archive_with(&p, &with(book(), folder), &fx.out(), &never).unwrap();
    let deeper = pack(nested(&mut fx, 6), None);
    let err = write_archive_with(&deeper, &with(MenuMode::PODCAST, folder), &fx.out(), &never)
        .unwrap_err();
    assert!(
        matches!(
            err,
            ArchiveError::OverLimit {
                count: 6,
                max: 5,
                ..
            }
        ),
        "{err:?}"
    );
    assert_eq!(err.to_string(), "trop de niveaux de menus : 6 (au plus 5)");

    // 300 chapitres en 6 parties de 50 : acceptés ; les podcasts s'arrêtent à 50 épisodes.
    let mut fx = Fixture::new();
    let parts: Vec<MenuEntry> = (0..6)
        .map(|k| cat(&format!("Partie {k}"), fx.episodes(50)))
        .collect();
    let p = pack(parts, None);
    let err = write_archive(&p, &fx.out(), &never).unwrap_err();
    assert!(matches!(err, ArchiveError::TooManyEpisodes(300)), "{err:?}");
    let summary = write_archive_with(&p, &with(book(), folder), &fx.out(), &never).unwrap();
    assert_eq!(summary.episodes, 300);
    // entrée + 300 chapitres + 300 options + 6 parties
    assert_eq!(summary.stages, 607);
    let nodes = Nodes::of(&read_zip(&fx.out()));
    assert_graph(&nodes.0);
    assert_eq!(nodes.action(ROOT_MENU).len(), 6);

    // Chaque borne reçue est appliquée, avec son message.
    let tight = |f: &dyn Fn(&mut Limits)| {
        let mut l = folder;
        f(&mut l);
        with(book(), l)
    };
    let cases: Vec<(ArchiveOptions, &str)> = vec![
        (
            tight(&|l| l.max_episodes = 299),
            "trop d'histoires ou de chapitres : 300 (au plus 299)",
        ),
        (
            tight(&|l| l.max_options_per_menu = 5),
            "trop d'options dans un menu : 6 (au plus 5)",
        ),
        (
            tight(&|l| l.max_stages = 606),
            "trop de scènes : 607 (au plus 606)",
        ),
    ];
    for (opts, msg) in cases {
        let err = write_archive_with(&p, &opts, &fx.out(), &never).unwrap_err();
        assert!(matches!(err, ArchiveError::OverLimit { .. }), "{err:?}");
        assert_eq!(err.to_string(), msg);
    }
    // Audio : taille d'un fichier et total.
    let mut fx = Fixture::new();
    let p = pack(fx.episodes(3), None);
    let one = fs::metadata(fx.path("in1.mp3")).unwrap().len();
    let err = write_archive_with(
        &p,
        &tight(&|l| l.max_total_audio_bytes = one),
        &fx.out(),
        &never,
    )
    .unwrap_err();
    assert!(
        matches!(err, ArchiveError::OverLimit { max, .. } if max == one),
        "{err:?}"
    );
    let err = write_archive_with(&p, &tight(&|l| l.max_audio_bytes = one), &fx.out(), &never)
        .unwrap_err();
    assert!(matches!(err, ArchiveError::InvalidAudio { .. }), "{err:?}");
    assert!(!fx.out().exists());
    // Bornes des podcasts passées explicitement : erreurs historiques.
    let err = write_archive_with(
        &pack(nested(&mut fx, 4), None),
        &with(book(), Limits::PODCAST),
        &fx.out(),
        &never,
    )
    .unwrap_err();
    assert!(matches!(err, ArchiveError::TooDeep), "{err:?}");
    assert_eq!(Limits::default(), Limits::PODCAST);
}

// --- conversion ----------------------------------------------------------------------------------

/// Paquet produit par la crate `telmi2flam` (fil à pile de 8 Mo, comme le test des podcasts).
fn convert(out: PathBuf) -> telmi2flam::package::Package {
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut archive = telmi2flam::archive::Archive::open(&out).unwrap();
            telmi2flam::package::build_package(
                &mut archive,
                &telmi2flam::package::Options::default(),
                &|| false,
            )
            .unwrap()
        })
        .unwrap()
        .join()
        .unwrap()
}

fn nodes_lua(package: &telmi2flam::package::Package) -> String {
    let lua = &package
        .entries
        .iter()
        .find(|(n, _)| n == "script/nodes.lua")
        .unwrap()
        .1;
    String::from_utf8_lossy(lua).into_owned()
}

/// Livre racine en parties, annoncé, puis livre dans un recueil : convertis par le port Rust
/// du convertisseur sans avertissement.
#[test]
fn livres_convertis_par_la_crate_telmi2flam_sans_avertissement() {
    let mut fx = Fixture::new();
    let mut menu = parts(&mut fx, None);
    speak_all(&mut fx, &mut menu, &["B"]);
    let p = pack(menu, Some(fx.cover()));
    write_archive_with(
        &p,
        &options(chapters(ChapterStart::Chapter, BookEnd::Menu, true)),
        &fx.out(),
        &never,
    )
    .unwrap();
    let package = convert(fx.out());
    assert!(
        package.warnings.is_empty(),
        "avertissements : {:?}",
        package.warnings
    );
    let lua = nodes_lua(&package);
    assert!(!lua.contains("inventory = {"), "{lua}");
    // `role` transmis au moteur ; l'annonce et les options hors de la jauge : 5 chapitres.
    assert!(lua.contains("role = \"chapter\""), "{lua}");
    assert!(lua.contains("role = \"title\""), "{lua}");
    assert!(lua.contains("totalChapters = 5,"), "{lua}");
    if let Some(dir) = std::env::var_os("FLAM_DUMP_NODES_LUA") {
        fs::write(Path::new(&dir).join("livre_racine.lua"), &lua).unwrap();
    }

    let mut fx = Fixture::new();
    let menu = book_in_collection(&mut fx, chapters(ChapterStart::Menu, BookEnd::End, false));
    write_archive_with(
        &pack(menu, None),
        &options(collection(CollectionEnd::Menu)),
        &fx.out(),
        &never,
    )
    .unwrap();
    let package = convert(fx.out());
    assert!(
        package.warnings.is_empty(),
        "avertissements : {:?}",
        package.warnings
    );
    if let Some(dir) = std::env::var_os("FLAM_DUMP_NODES_LUA") {
        fs::write(
            Path::new(&dir).join("livre_dans_recueil.lua"),
            nodes_lua(&package),
        )
        .unwrap();
    }
}

/// Générateur du paquet d'essai sur la Flam (reprise d'un chapitre à sa position) : livre de
/// 2 parties × 3 chapitres, annoncés. `FLAM_ESSAI_DIR` contient `c1..c6.mp3/png` (chapitres),
/// `t1..t6.mp3` (titres lus), `p1/p2.mp3` (parties), `titre.mp3`, `couverture.png` ; l'archive
/// Telmi est écrite à côté (`livre-essai.zip`), à convertir avec `telmi2flam`.
/// `cargo test paquet_d_essai_flam -- --ignored`.
#[test]
#[ignore = "générateur manuel (FLAM_ESSAI_DIR)"]
fn paquet_d_essai_flam() {
    let dir = PathBuf::from(std::env::var_os("FLAM_ESSAI_DIR").expect("FLAM_ESSAI_DIR"));
    let chapter = |n: usize| {
        MenuEntry::Episode(PackEpisode {
            title: format!("Chapitre {n}"),
            audio: dir.join(format!("c{n}.mp3")),
            image: Some(dir.join(format!("c{n}.png"))),
            title_audio: Some(dir.join(format!("t{n}.mp3"))),
        })
    };
    let part = |p: usize| MenuEntry::Category {
        name: format!("Partie {p}"),
        entries: (3 * p - 2..=3 * p).map(chapter).collect(),
        audio: Some(dir.join(format!("p{p}.mp3"))),
        image: None,
    };
    let menu = vec![part(1), part(2)];
    let p = Pack {
        uuid: Uuid::parse_str("5d0c7a1e-8b2f-4c3d-9e4f-a1b2c3d4e5f6").unwrap(),
        metadata: PackMetadata {
            title: "Essai chapitres".into(),
            subtitle: "Reprise à la position".into(),
            age: 3,
            description: default_description(&menu),
        },
        cover: Some(dir.join("couverture.png")),
        menu,
        title_audio: Some(dir.join("titre.mp3")),
    };
    let out = dir.parent().unwrap().join("livre-essai.zip");
    write_archive_with(
        &p,
        &options(chapters(ChapterStart::Chapter, BookEnd::Menu, true)),
        &out,
        &never,
    )
    .unwrap();
    assert_graph(&json_of(&read_zip(&out), "nodes.json"));
}

/// Comme `binaire_convertit_et_valide_l_archive`, pour un livre à chapitres en parties :
/// binaire `telmi2flam.exe` livré, puis `validate.py`. Ignoré sans eux (voir ce test).
#[test]
fn binaire_convertit_et_valide_un_livre() {
    let required = std::env::var_os("FLAM_TEST_REQUIRE_CONVERTER").is_some_and(|v| v == "1");
    let (Some(exe), Some(python)) = (converter(), python()) else {
        assert!(
            !required,
            "FLAM_TEST_REQUIRE_CONVERTER=1 : telmi2flam.exe ou FLAM_TEST_PYTHON introuvable"
        );
        eprintln!("telmi2flam.exe ou Python de validate.py absent : test ignoré");
        return;
    };
    let mut fx = Fixture::new();
    let mut menu = parts(&mut fx, None);
    speak_all(&mut fx, &mut menu, &[]);
    let p = pack(menu, Some(fx.cover()));
    write_archive_with(
        &p,
        &options(chapters(ChapterStart::Chapter, BookEnd::Menu, true)),
        &fx.out(),
        &never,
    )
    .unwrap();
    let archive = fx.out();
    let work = fx.dir.path();
    let pk = fx.path("livre.plain.pk");
    let (code, stdout, stderr) = run(
        &exe,
        false,
        work,
        &[
            archive.as_os_str(),
            "-o".as_ref(),
            pk.as_os_str(),
            "--progress-json".as_ref(),
        ],
    );
    assert_eq!(code, 0, "conversion : {stderr}\n{stdout}");
    let events: Vec<Value> = stdout
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .collect();
    assert!(
        events.iter().all(|v| v["event"] != "warning"),
        "avertissements : {stdout}"
    );
    assert!(events.iter().any(|v| v["event"] == "done"), "{stdout}");
    let validate = validate_script();
    let (code, stdout, stderr) = run(&python, true, work, &[validate.as_os_str(), pk.as_os_str()]);
    assert_eq!(code, 0, "validate.py :\n{stdout}\n{stderr}");
    let warns: Vec<&str> = stdout
        .lines()
        .filter(|l| l.trim_start().starts_with("[WARN]"))
        .filter(|l| !l.contains("syntaxe Lua non verifiee"))
        .collect();
    assert!(warns.is_empty(), "validate.py :\n{stdout}");
}
