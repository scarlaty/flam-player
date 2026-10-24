use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use super::*;

/// Trame MP3 silencieuse (MPEG1 Layer III, 128 kbit/s, 44 100 Hz, stéréo), comme celle de
/// `telmi2flam` : un MP3 d'essai conforme (§ 3.1), sans rééchantillonnage.
fn mp3_bytes(frames: usize) -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    frame.repeat(frames)
}

struct Fixture {
    dir: tempfile::TempDir,
    next: usize,
}

impl Fixture {
    fn new() -> Self {
        Fixture {
            dir: tempfile::tempdir().unwrap(),
            next: 0,
        }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// Épisode d'essai : MP3 et PNG 640 × 480 propres (couleur différente à chaque fois).
    fn episode(&mut self, title: &str) -> MenuEntry {
        self.next += 1;
        let n = self.next;
        let audio = self.path(&format!("in{n}.mp3"));
        fs::write(&audio, mp3_bytes(20 + n)).unwrap();
        let image = self.path(&format!("in{n}.png"));
        let color = [(n * 5) as u8, 0x40, 0x80];
        fs::write(
            &image,
            Artwork::fallback(ArtworkKind::Episode, color).unwrap().png,
        )
        .unwrap();
        MenuEntry::Episode(PackEpisode {
            title: title.to_string(),
            audio,
            image: Some(image),
            title_audio: None,
        })
    }

    /// MP3 de titre lu (§ 12), distinct à chaque appel.
    fn title_mp3(&mut self) -> PathBuf {
        self.next += 1;
        let path = self.path(&format!("title{}.mp3", self.next));
        fs::write(&path, mp3_bytes(3 + self.next % 5)).unwrap();
        path
    }

    fn episodes(&mut self, count: usize) -> Vec<MenuEntry> {
        (1..=count)
            .map(|i| self.episode(&format!("Épisode n°{i}")))
            .collect()
    }

    fn cover(&self) -> PathBuf {
        let cover = self.path("cover-in.png");
        fs::write(
            &cover,
            Artwork::fallback(ArtworkKind::Cover, [0xAA, 0x10, 0x10])
                .unwrap()
                .png,
        )
        .unwrap();
        cover
    }

    fn out(&self) -> PathBuf {
        self.path("pack.zip")
    }
}

fn cat(name: &str, entries: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry::Category {
        name: name.to_string(),
        entries,
        audio: None,
    }
}

fn pack(menu: Vec<MenuEntry>, cover: Option<PathBuf>) -> Pack {
    Pack {
        uuid: new_pack_uuid(),
        metadata: PackMetadata {
            title: "Mon pack".into(),
            subtitle: "© Radio Exemple".into(),
            age: 5,
            description: default_description(&menu),
        },
        cover,
        menu,
        title_audio: None,
    }
}

fn never() -> bool {
    false
}

fn read_zip(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        assert_eq!(f.compression(), CompressionMethod::Stored, "{}", f.name());
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        out.insert(f.name().to_string(), bytes);
    }
    out
}

fn json_of(files: &BTreeMap<String, Vec<u8>>, name: &str) -> Value {
    serde_json::from_slice(&files[name]).unwrap()
}

/// Lecture du graphe produit, à la manière du moteur (`story.lua`).
struct Nodes(Value);

impl Nodes {
    fn of(files: &BTreeMap<String, Vec<u8>>) -> Self {
        Nodes(json_of(files, "nodes.json"))
    }
    fn stage(&self, id: &str) -> &Value {
        let st = &self.0["stages"][id];
        assert!(st.is_object(), "scène {id} absente");
        st
    }
    fn action(&self, id: &str) -> Vec<String> {
        self.0["actions"][id]
            .as_array()
            .unwrap_or_else(|| panic!("action {id} absente"))
            .iter()
            .map(|e| e["stage"].as_str().unwrap().to_string())
            .collect()
    }
    /// Cible d'une transition `ok`/`home` : (action, index).
    fn target(&self, stage: &str, key: &str) -> (String, usize) {
        let t = &self.stage(stage)[key];
        (
            t["action"].as_str().unwrap().to_string(),
            t["index"].as_u64().unwrap() as usize,
        )
    }
    /// Épisode joué par une option (action « jouer » à une seule scène).
    fn played_by(&self, option: &str) -> String {
        let (action, _) = self.target(option, "ok");
        let list = self.action(&action);
        assert_eq!(list.len(), 1, "action « jouer » {action} : une scène");
        list[0].clone()
    }
    /// Après un épisode : scène suivante jouée directement, ou retour à un menu (action, index).
    fn after(&self, episode: &str) -> Result<String, (String, usize)> {
        let (action, index) = self.target(episode, "ok");
        let list = self.action(&action);
        if list.len() == 1 && list[0].starts_with('e') {
            Ok(list[0].clone())
        } else {
            Err((action, index))
        }
    }
    fn menu_of_root(&self) -> String {
        let (start_action, _) = (
            self.0["startAction"]["action"]
                .as_str()
                .unwrap()
                .to_string(),
            0,
        );
        let start = &self.action(&start_action)[0];
        let st = self.stage(start);
        assert!(st["audio"].is_null() && st["control"]["autoplay"] == true);
        self.target(start, "ok").0
    }
}

#[test]
fn menus_de_1_10_et_50_episodes_enchainement_et_retour() {
    for count in [1usize, 10, 50] {
        let mut fx = Fixture::new();
        let menu = fx.episodes(count);
        let summary = write_archive(&pack(menu, Some(fx.cover())), &fx.out(), &never).unwrap();
        assert_eq!(summary.episodes, count);
        // entrée + options + épisodes
        assert_eq!(summary.stages, 1 + 2 * count);
        let files = read_zip(&fx.out());
        let nodes = Nodes::of(&files);
        let root = nodes.menu_of_root();
        let options = nodes.action(&root);
        assert_eq!(options.len(), count);
        let episodes: Vec<String> = options.iter().map(|o| nodes.played_by(o)).collect();
        for (i, ep) in episodes.iter().enumerate() {
            let st = nodes.stage(ep);
            assert_eq!(st["control"]["autoplay"], true);
            assert_eq!(st["audio"], format!("e{:03}.mp3", i + 1));
            assert!(files.contains_key(&format!("audios/e{:03}.mp3", i + 1)));
            assert!(files.contains_key(&format!("images/e{:03}.png", i + 1)));
            // retour : vers ce menu, focalisé sur l'épisode
            assert_eq!(nodes.target(ep, "home"), (root.clone(), i));
            match nodes.after(ep) {
                Ok(next) => assert_eq!(next, episodes[i + 1]),
                Err(menu) => {
                    assert_eq!(i, count - 1, "seul le dernier retourne au menu");
                    assert_eq!(menu, (root.clone(), i));
                }
            }
            // option du menu racine : pas de retour propre (menu Start du moteur)
            assert_eq!(nodes.stage(&options[i])["control"]["home"], false);
            assert!(nodes.stage(&options[i])["audio"].is_null());
        }
    }
}

/// Racine : A, X[B, Y[C, D], E], F.
fn three_levels(fx: &mut Fixture) -> Vec<MenuEntry> {
    let a = fx.episode("A");
    let b = fx.episode("B");
    let c = fx.episode("C");
    let d = fx.episode("D");
    let e = fx.episode("E");
    let f = fx.episode("F");
    vec![a, cat("X", vec![b, cat("Y", vec![c, d]), e]), f]
}

#[test]
fn sous_menus_sur_trois_niveaux() {
    let mut fx = Fixture::new();
    let menu = three_levels(&mut fx);
    let summary = write_archive(&pack(menu, None), &fx.out(), &never).unwrap();
    assert_eq!(summary.episodes, 6);
    assert_eq!(summary.menus, 3);
    let files = read_zip(&fx.out());
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let title = |stage: &str| notes[stage]["title"].as_str().unwrap().to_string();

    let root = nodes.menu_of_root();
    let r = nodes.action(&root);
    assert_eq!(
        r.iter().map(|o| title(o)).collect::<Vec<_>>(),
        ["A", "X", "F"]
    );
    let (a, f) = (nodes.played_by(&r[0]), nodes.played_by(&r[2]));
    // A enchaîne sur F (la catégorie X est sautée), F retourne à la racine sur F.
    assert_eq!(nodes.after(&a), Ok(f.clone()));
    assert_eq!(nodes.after(&f), Err((root.clone(), 2)));

    let (x, xi) = nodes.target(&r[1], "ok");
    assert_eq!(xi, 0);
    let xo = nodes.action(&x);
    assert_eq!(
        xo.iter().map(|o| title(o)).collect::<Vec<_>>(),
        ["B", "Y", "E"]
    );
    for o in &xo {
        // retour d'une option du sous-menu : menu parent, focalisé sur la catégorie
        assert_eq!(nodes.target(o, "home"), (root.clone(), 1));
        assert_eq!(nodes.stage(o)["control"]["home"], true);
    }
    let (b, e) = (nodes.played_by(&xo[0]), nodes.played_by(&xo[2]));
    assert_eq!(nodes.after(&b), Ok(e.clone()));
    assert_eq!(nodes.after(&e), Err((x.clone(), 2)));
    assert_eq!(nodes.target(&b, "home"), (x.clone(), 0));

    let (y, _) = nodes.target(&xo[1], "ok");
    let yo = nodes.action(&y);
    let (c, d) = (nodes.played_by(&yo[0]), nodes.played_by(&yo[1]));
    assert_eq!(nodes.after(&c), Ok(d.clone()));
    assert_eq!(nodes.after(&d), Err((y.clone(), 1)));
    assert_eq!(nodes.target(&yo[0], "home"), (x.clone(), 1));

    // Fichiers dans l'ordre de l'arborescence ; image d'une catégorie = premier épisode (N5 a).
    for (ep, n) in [(&a, 1), (&b, 2), (&c, 3), (&d, 4), (&e, 5), (&f, 6)] {
        assert_eq!(nodes.stage(ep)["image"], format!("e{n:03}.png"));
    }
    assert_eq!(nodes.stage(&r[1])["image"], "e002.png");
    assert_eq!(nodes.stage(&xo[1])["image"], "e003.png");
}

// --- titres lus (§ 12, #462) -------------------------------------------------------------------

/// Donne un titre lu à chaque option de l'arborescence dont le libellé n'est pas dans `mute`.
fn speak_all(fx: &mut Fixture, menu: &mut [MenuEntry], mute: &[&str]) {
    for entry in menu {
        match entry {
            MenuEntry::Episode(ep) => {
                if !mute.contains(&ep.title.as_str()) {
                    ep.title_audio = Some(fx.title_mp3());
                }
            }
            MenuEntry::Category {
                name,
                entries,
                audio,
            } => {
                if !mute.contains(&name.as_str()) {
                    *audio = Some(fx.title_mp3());
                }
                speak_all(fx, entries, mute);
            }
        }
    }
}

/// Options de l'arborescence (parcours en profondeur) : libellé, audio de la scène d'option.
fn option_audios(nodes: &Nodes, notes: &Value, action: &str, out: &mut Vec<(String, Value)>) {
    for option in nodes.action(action) {
        let st = nodes.stage(&option);
        out.push((
            notes[&option]["title"].as_str().unwrap().to_string(),
            st["audio"].clone(),
        ));
        let (target, _) = nodes.target(&option, "ok");
        let list = nodes.action(&target);
        // Sous-menu : plusieurs options ou une option qui n'est pas un épisode.
        if !(list.len() == 1 && list[0].starts_with('e')) {
            option_audios(nodes, notes, &target, out);
        }
    }
}

#[test]
fn titres_lus_un_audio_par_option_et_title_mp3() {
    let mut fx = Fixture::new();
    let mut menu = three_levels(&mut fx);
    speak_all(&mut fx, &mut menu, &[]);
    let mut p = pack(menu, None);
    let title = fx.title_mp3();
    p.title_audio = Some(title.clone());
    write_archive(&p, &fx.out(), &never).unwrap();
    let files = read_zip(&fx.out());
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let mut audios = Vec::new();
    option_audios(&nodes, &notes, &nodes.menu_of_root(), &mut audios);
    // Épisodes A..F = 1..6 ; catégories X = 1, Y = 2 (ordre du parcours).
    let expected = [
        ("A", "t001.mp3"),
        ("X", "c001.mp3"),
        ("B", "t002.mp3"),
        ("Y", "c002.mp3"),
        ("C", "t003.mp3"),
        ("D", "t004.mp3"),
        ("E", "t005.mp3"),
        ("F", "t006.mp3"),
    ];
    assert_eq!(
        audios,
        expected
            .iter()
            .map(|(l, a)| (l.to_string(), Value::from(*a)))
            .collect::<Vec<_>>()
    );
    // Chaque audio cité est dans l'archive, copie exacte de son MP3.
    let sources: Vec<&Path> = {
        fn walk<'a>(menu: &'a [MenuEntry], out: &mut Vec<(&'a str, &'a Path)>) {
            for e in menu {
                match e {
                    MenuEntry::Episode(ep) => {
                        out.push((&ep.title, ep.title_audio.as_deref().unwrap()))
                    }
                    MenuEntry::Category {
                        name,
                        entries,
                        audio,
                    } => {
                        out.push((name, audio.as_deref().unwrap()));
                        walk(entries, out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&p.menu, &mut out);
        out.into_iter().map(|(_, path)| path).collect()
    };
    for ((_, file), src) in expected.iter().zip(sources) {
        assert_eq!(
            files[&format!("audios/{file}")],
            fs::read(src).unwrap(),
            "{file}"
        );
    }
    assert_eq!(files["title.mp3"], fs::read(&title).unwrap());
    // Scènes d'épisode et d'entrée inchangées : pas d'annonce avant l'épisode.
    let r = nodes.action(&nodes.menu_of_root());
    assert_eq!(nodes.stage(&nodes.played_by(&r[0]))["audio"], "e001.mp3");
    let audio_entries = files.keys().filter(|k| k.starts_with("audios/")).count();
    assert_eq!(audio_entries, 6 + 8, "épisodes et titres lus, rien d'autre");
}

#[test]
fn titres_lus_partiels_les_autres_options_restent_muettes() {
    let mut fx = Fixture::new();
    let mut menu = three_levels(&mut fx);
    // B et Y muets (échec ou rien à lire) : `"audio": null` (silent.mp3 par telmi2flam).
    speak_all(&mut fx, &mut menu, &["B", "Y"]);
    write_archive(&pack(menu, None), &fx.out(), &never).unwrap();
    let files = read_zip(&fx.out());
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let mut audios = Vec::new();
    option_audios(&nodes, &notes, &nodes.menu_of_root(), &mut audios);
    for (label, audio) in &audios {
        if label == "B" || label == "Y" {
            assert!(audio.is_null(), "{label} : {audio}");
        } else {
            let name = audio.as_str().unwrap_or_else(|| panic!("{label} muet"));
            assert!(files.contains_key(&format!("audios/{name}")), "{name}");
        }
    }
    assert!(!files.contains_key("audios/t002.mp3"));
    assert!(!files.contains_key("audios/c002.mp3"));
    assert!(!files.contains_key("title.mp3"), "pas de titre lu du pack");
}

#[test]
fn sans_titre_lu_l_archive_est_inchangee() {
    let mut fx = Fixture::new();
    let menu = three_levels(&mut fx);
    write_archive(&pack(menu, None), &fx.out(), &never).unwrap();
    let files = read_zip(&fx.out());
    assert!(!files.contains_key("title.mp3"));
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let mut audios = Vec::new();
    option_audios(&nodes, &notes, &nodes.menu_of_root(), &mut audios);
    assert_eq!(audios.len(), 8);
    assert!(audios.iter().all(|(_, a)| a.is_null()), "{audios:?}");
    assert!(files
        .keys()
        .filter(|k| k.starts_with("audios/"))
        .all(|k| k.starts_with("audios/e")));
}

#[test]
fn un_titre_lu_invalide_ou_reutilise_est_refuse_sans_rien_ecrire() {
    let mut fx = Fixture::new();
    let mut menu = fx.episodes(2);
    let bad = fx.path("bad.mp3");
    fs::write(&bad, b"pas un mp3").unwrap();
    if let MenuEntry::Episode(ep) = &mut menu[0] {
        ep.title_audio = Some(bad);
    }
    let err = write_archive(&pack(menu.clone(), None), &fx.out(), &never).unwrap_err();
    assert!(matches!(err, ArchiveError::InvalidAudio { .. }), "{err}");
    assert!(!fx.out().exists());
    // Même MP3 pour le titre lu de l'épisode et celui du pack.
    let shared = fx.title_mp3();
    if let MenuEntry::Episode(ep) = &mut menu[0] {
        ep.title_audio = Some(shared.clone());
    }
    let mut p = pack(menu, None);
    p.title_audio = Some(shared);
    let err = write_archive(&p, &fx.out(), &never).unwrap_err();
    assert!(err.to_string().contains("deux fois"), "{err}");
    assert!(!fx.out().exists());
}

#[test]
fn metadonnees_uuid_aleatoire_et_version_1() {
    let mut fx = Fixture::new();
    let menu = fx.episodes(2);
    let p1 = pack(menu.clone(), Some(fx.cover()));
    let p2 = pack(menu, Some(fx.cover()));
    assert_ne!(p1.uuid, p2.uuid, "deux packs, deux uuid");
    let (o1, o2) = (fx.path("1.zip"), fx.path("2.zip"));
    write_archive(&p1, &o1, &never).unwrap();
    write_archive(&p2, &o2, &never).unwrap();
    let (m1, m2) = (
        json_of(&read_zip(&o1), "metadata.json"),
        json_of(&read_zip(&o2), "metadata.json"),
    );
    assert_ne!(m1["uuid"], m2["uuid"]);
    for (m, p) in [(&m1, &p1), (&m2, &p2)] {
        let u = m["uuid"].as_str().unwrap();
        assert_eq!(u, p.uuid.hyphenated().to_string());
        assert_eq!(u.len(), 36);
        assert!(u
            .chars()
            .all(|c| c == '-' || c.is_ascii_digit() || c.is_ascii_lowercase()));
        assert_eq!(Uuid::parse_str(u).unwrap().get_version_num(), 4);
        assert_eq!(m["version"], 1);
        assert_eq!(m["title"], "Mon pack");
        assert_eq!(m["category"], "© Radio Exemple");
        assert_eq!(m["age"], 5);
        assert_eq!(m["image"], "cover.png");
        assert_eq!(m["description"], "- Épisode n°1\n- Épisode n°2");
    }
}

#[test]
fn libelles_translitteres_et_tronques() {
    let long = "Épisode « spécial » — l’œuvre de Mikuláš 🎉 et un titre bien trop long";
    let l = label(long, "repli");
    assert!(l.chars().count() <= LABEL_MAX, "{l}");
    assert!(l.ends_with("..."), "{l}");
    assert_eq!(l, "Épisode « spécial » - l'oeuvre de Mik...");
    assert_eq!(label("Mikuláš", "repli"), "Mikulás");
    assert_eq!(label("Łódź – Đakovo", "repli"), "Lódz - Dakovo");
    // Sans équivalent latin : `?`, comme le convertisseur.
    assert_eq!(label("Ωmega", "repli"), "?mega");
    for c in l.chars() {
        let o = c as u32;
        assert!(
            (0x20..=0x7E).contains(&o) || (0xA0..=0xFF).contains(&o),
            "{c:?}"
        );
    }
    assert_eq!(label("  Court\ttitre\n ", "repli"), "Court titre");
    assert_eq!(label("🎉", "Épisode 3"), "Épisode 3");
    assert_eq!(label(&"a".repeat(40), "x"), "a".repeat(40));
    assert_eq!(
        label(&"a".repeat(41), "x"),
        format!("{}...", "a".repeat(37))
    );

    // Dans l'archive : libellés des options et des catégories.
    let mut fx = Fixture::new();
    let ep = fx.episode(long);
    let menu = vec![cat("🎧", vec![ep]), fx.episode("Łukasz")];
    write_archive(&pack(menu, None), &fx.out(), &never).unwrap();
    let files = read_zip(&fx.out());
    let nodes = Nodes::of(&files);
    let notes = json_of(&files, "notes.json");
    let root = nodes.action(&nodes.menu_of_root());
    assert_eq!(notes[&root[0]]["title"], "Catégorie 1");
    assert_eq!(notes[&root[1]]["title"], "Lukasz");
    let (sub, _) = nodes.target(&root[0], "ok");
    assert_eq!(notes[&nodes.action(&sub)[0]]["title"], l.as_str());
}

#[test]
fn noms_d_entrees_surs_et_images_de_repli() {
    let mut fx = Fixture::new();
    let mut menu = fx.episodes(3);
    if let MenuEntry::Episode(e) = &mut menu[1] {
        e.image = None;
        e.title = "../../evil\\name:<>".into();
    }
    write_archive(&pack(menu, None), &fx.out(), &never).unwrap();
    let files = read_zip(&fx.out());
    let names: Vec<&str> = files.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        [
            "audios/e001.mp3",
            "audios/e002.mp3",
            "audios/e003.mp3",
            "cover.png",
            "images/e001.png",
            "images/e002.png",
            "images/e003.png",
            "metadata.json",
            "nodes.json",
            "notes.json",
        ]
    );
    let dims = |bytes: &[u8]| {
        let reader = png::Decoder::new(Cursor::new(bytes)).read_info().unwrap();
        (reader.info().width, reader.info().height)
    };
    assert_eq!(dims(&files["cover.png"]), (512, 512));
    assert_eq!(dims(&files["images/e002.png"]), (640, 480));
    assert_eq!(
        files["images/e002.png"],
        Artwork::fallback(ArtworkKind::Episode, FALLBACK_COLOR)
            .unwrap()
            .png
    );
    // audio et image copiés à l'identique
    assert_eq!(
        files["audios/e003.mp3"],
        fs::read(fx.path("in3.mp3")).unwrap()
    );
    assert_eq!(
        files["images/e003.png"],
        fs::read(fx.path("in3.png")).unwrap()
    );
}

fn assert_nothing_written(fx: &Fixture, previous: Option<&[u8]>) {
    match previous {
        Some(bytes) => assert_eq!(fs::read(fx.out()).unwrap(), bytes),
        None => assert!(!fx.out().exists()),
    }
    let tmp: Vec<_> = fs::read_dir(fx.dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(tmp.is_empty(), "temporaires restants : {tmp:?}");
}

#[test]
fn bornes_refusees_sans_rien_ecrire() {
    let mut fx = Fixture::new();
    let check = |fx: &Fixture, p: Pack, expect: fn(&ArchiveError) -> bool| {
        let err = write_archive(&p, &fx.out(), &never).unwrap_err();
        assert!(expect(&err), "erreur inattendue : {err:?}");
        assert_nothing_written(fx, None);
    };
    // profondeur : 4 niveaux de menus
    let e = fx.episode("x");
    let deep = vec![cat("1", vec![cat("2", vec![cat("3", vec![e])])])];
    check(&fx, pack(deep, None), |e| {
        matches!(e, ArchiveError::TooDeep)
    });
    // 3 niveaux : accepté (vérifié ailleurs) ; 51 options dans un menu
    let wide = fx.episodes(51);
    check(&fx, pack(wide, None), |e| {
        matches!(e, ArchiveError::TooManyOptions(51))
    });
    // 60 épisodes répartis dans deux catégories : le message donne le total, pas « 51 »
    let (a, b) = (fx.episodes(35), fx.episodes(25));
    check(&fx, pack(vec![cat("a", a), cat("b", b)], None), |e| {
        matches!(e, ArchiveError::TooManyEpisodes(60))
    });
    check(&fx, pack(vec![], None), |e| {
        matches!(e, ArchiveError::NoEpisode)
    });
    let e = fx.episode("x");
    check(
        &fx,
        pack(vec![e, cat("vide", vec![])], None),
        |e| matches!(e, ArchiveError::EmptyCategory(n) if n == "vide"),
    );
    // même épisode deux fois (N9 a)
    let e = fx.episode("x");
    check(&fx, pack(vec![e.clone(), cat("c", vec![e])], None), |e| {
        matches!(e, ArchiveError::DuplicateEpisode(_))
    });
    // métadonnées
    let mut p = pack(fx.episodes(1), None);
    p.metadata.title = "  ".into();
    check(&fx, p, |e| matches!(e, ArchiveError::EmptyTitle));
    let mut p = pack(fx.episodes(1), None);
    p.metadata.subtitle = "s".repeat(MAX_TITLE_CHARS + 1);
    check(&fx, p, |e| matches!(e, ArchiveError::TextTooLong { .. }));
    let mut p = pack(fx.episodes(1), None);
    p.metadata.description = "d".repeat(MAX_DESCRIPTION_BYTES + 1);
    check(&fx, p, |e| matches!(e, ArchiveError::TextTooLong { .. }));
}

/// Titre et sous-titre du pack translittérés dans `metadata.json` (U+2019, emoji), sans
/// titre vide si tout est supprimé ; la validation garde le texte saisi.
#[test]
fn metadonnees_translitterees() {
    let mut fx = Fixture::new();
    let mut p = pack(fx.episodes(1), None);
    p.metadata.title = " L’œuvre d’art 🎉 ".into();
    p.metadata.subtitle = "© Radio d’Exemple".into();
    let m = metadata_json(&p);
    assert_eq!(m["title"], "L'oeuvre d'art");
    assert_eq!(m["category"], "© Radio d'Exemple");
    p.metadata.title = "🎉".into();
    assert_eq!(metadata_json(&p)["title"], "Podcast");
}

#[test]
fn fichiers_d_entree_invalides() {
    let mut fx = Fixture::new();
    let with = |fx: &mut Fixture, f: &dyn Fn(&mut PackEpisode)| {
        let mut e = fx.episode("x");
        if let MenuEntry::Episode(ep) = &mut e {
            f(ep);
        }
        pack(vec![e], None)
    };
    let not_mp3 = fx.path("faux.mp3");
    fs::write(&not_mp3, b"RIFF....WAVEfmt ").unwrap();
    let empty = fx.path("vide.mp3");
    fs::write(&empty, b"").unwrap();
    let jpeg = fx.path("faux.png");
    fs::write(&jpeg, [0xFF, 0xD8, 0xFF, 0xE0, 0, 0, 0, 0]).unwrap();
    type Expect = fn(&ArchiveError) -> bool;
    let cases: Vec<(Pack, Expect)> = vec![
        (with(&mut fx, &|e| e.audio = not_mp3.clone()), |e| {
            matches!(e, ArchiveError::InvalidAudio { .. })
        }),
        (with(&mut fx, &|e| e.audio = empty.clone()), |e| {
            matches!(e, ArchiveError::InvalidAudio { .. })
        }),
        (
            with(&mut fx, &|e| e.audio = PathBuf::from("absent.mp3")),
            |e| matches!(e, ArchiveError::InvalidAudio { .. }),
        ),
        (with(&mut fx, &|e| e.image = Some(jpeg.clone())), |e| {
            matches!(e, ArchiveError::InvalidImage { .. })
        }),
        (
            {
                let mut p = with(&mut fx, &|_| ());
                p.cover = Some(jpeg.clone());
                p
            },
            |e| matches!(e, ArchiveError::InvalidImage { .. }),
        ),
    ];
    // Une archive précédente reste intacte après chaque échec.
    fs::write(fx.out(), b"ancienne archive").unwrap();
    for (p, expect) in cases {
        let err = write_archive(&p, &fx.out(), &never).unwrap_err();
        assert!(expect(&err), "erreur inattendue : {err:?}");
        assert_nothing_written(&fx, Some(b"ancienne archive"));
    }
    // L'ID3 en tête est accepté.
    let id3 = fx.path("id3.mp3");
    let mut bytes = b"ID3\x04\x00\x00\x00\x00\x00\x00".to_vec();
    bytes.extend(mp3_bytes(4));
    fs::write(&id3, bytes).unwrap();
    let p = with(&mut fx, &|e| e.audio = id3.clone());
    write_archive(&p, &fx.out(), &never).unwrap();
}

#[test]
fn annulation_et_remplacement_atomique() {
    let mut fx = Fixture::new();
    let p = pack(fx.episodes(5), None);
    fs::write(fx.out(), b"ancienne archive").unwrap();
    // Annulation à chaque point de contrôle possible : rien n'est laissé.
    for stop_at in 1..40 {
        let calls = std::cell::Cell::new(0);
        let cancel = || {
            calls.set(calls.get() + 1);
            calls.get() >= stop_at
        };
        match write_archive(&p, &fx.out(), &cancel) {
            Err(ArchiveError::Cancelled) => {
                assert_nothing_written(&fx, Some(b"ancienne archive"));
            }
            Ok(_) => {
                // plus de points de contrôle que `stop_at` : l'archive a remplacé l'ancienne
                assert!(stop_at > 5, "annulation trop tôt ignorée ({stop_at})");
                assert_nothing_written(&fx, Some(&fs::read(fx.out()).unwrap()));
                read_zip(&fx.out());
                return;
            }
            Err(e) => panic!("{e:?}"),
        }
    }
    panic!("l'écriture n'a jamais abouti");
}

#[test]
fn description_par_defaut_et_echecs() {
    let mut fx = Fixture::new();
    let menu = vec![fx.episode("Un\ntitre"), cat("c", vec![fx.episode("Deux")])];
    let d = default_description(&menu);
    assert_eq!(d, "- Un titre\n- Deux");
    assert_eq!(with_failures(&d, &[]), d);
    // titre RSS démesuré : ligne tronquée
    let long = with_failures("", &["x".repeat(10_000)]);
    let last = long.lines().last().unwrap();
    assert_eq!(last.chars().count(), 2 + 200);
    assert!(last.ends_with('…'));
    assert_eq!(
        with_failures(&d, &["Trois".into()]),
        "- Un titre\n- Deux\n\nÉpisodes non inclus (échec du téléchargement ou de la conversion) :\n- Trois"
    );
    assert_eq!(
        with_failures("  ", &["Trois".into()]),
        "Épisodes non inclus (échec du téléchargement ou de la conversion) :\n- Trois"
    );
}

/// L'archive est acceptée par le port Rust du convertisseur, sans avertissement.
#[test]
fn convertie_par_la_crate_telmi2flam_sans_avertissement() {
    let mut fx = Fixture::new();
    let menu = three_levels(&mut fx);
    let p = pack(menu, Some(fx.cover()));
    write_archive(&p, &fx.out(), &never).unwrap();
    let out = fx.out();
    // Conversion complète : fil à pile de 8 Mo (voir la documentation de la crate).
    let package = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut archive = telmi2flam::archive::Archive::open(&out).unwrap();
            let info = telmi2flam::info::read_info(&out).unwrap();
            let package = telmi2flam::package::build_package(
                &mut archive,
                &telmi2flam::package::Options::default(),
                &|| false,
            )
            .unwrap();
            (info, package)
        })
        .unwrap()
        .join()
        .unwrap();
    let (info, package) = package;
    use telmi2flam::info::Scalar;
    assert_eq!(info.title, Some(Scalar::Str("Mon pack".into())));
    assert_eq!(info.version, Some(Scalar::Int(1)));
    assert_eq!(
        info.uuid,
        Some(Scalar::Str(p.uuid.hyphenated().to_string()))
    );
    assert!(info.cover);
    assert!(
        package.warnings.is_empty(),
        "avertissements : {:?}",
        package.warnings
    );
    let names: Vec<&str> = package.entries.iter().map(|(n, _)| n.as_str()).collect();
    for n in [
        "sounds/e001.mp3",
        "sounds/e006.mp3map",
        "img/e006.lif",
        "img/thumbnail.lif",
    ] {
        assert!(names.contains(&n), "{n} absent du paquet");
    }
    let nodes = &package
        .entries
        .iter()
        .find(|(n, _)| n == "script/nodes.lua")
        .unwrap()
        .1;
    let nodes = String::from_utf8_lossy(nodes);
    assert!(nodes.contains("selector = \"carousel\""), "{nodes}");
}

// ---------------------------------------------------------------------------------------------
// Binaire `telmi2flam.exe` livré et `validate.py` (outil Python de développement) : critère de #216
// ---------------------------------------------------------------------------------------------

/// `telmi2flam.exe` livré (`build/telmi2flam`, après `npm run build:converter`) ou
/// `FLAM_TEST_RUST_CONVERTER`.
fn converter() -> Option<PathBuf> {
    let exe = std::env::var_os("FLAM_TEST_RUST_CONVERTER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../build/telmi2flam")
                .join("telmi2flam.exe")
        });
    exe.is_file().then_some(exe)
}

/// Python de développement pour `validate.py` (bibliothèque standard, `lupa` pour le contrôle
/// de syntaxe Lua) : `FLAM_TEST_PYTHON`. Le launcher ne livre pas de Python (#138).
fn python() -> Option<PathBuf> {
    std::env::var_os("FLAM_TEST_PYTHON").map(PathBuf::from)
}

fn validate_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tools/telmi2flam/validate.py")
}

/// Lance `program args` dans `cwd`, sans `FLAM_FFMPEG` (pas de rééchantillonnage) ; `isolated`
/// : options d'isolement de Python (`-E -s -B -X utf8`) en tête. Renvoie (code, stdout, stderr).
fn run(program: &Path, isolated: bool, cwd: &Path, args: &[&OsStr]) -> (i32, String, String) {
    let mut cmd = Command::new(program);
    if isolated {
        cmd.args(["-E", "-s", "-B", "-X", "utf8"]);
    }
    let out = cmd
        .args(args)
        .current_dir(cwd)
        .env_remove("FLAM_FFMPEG")
        .output()
        .expect("lancement du programme");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Critère de #216 : l'archive est lue (`--info`) et convertie par le binaire `telmi2flam.exe`
/// livré (processus enfant, comme le launcher), sans avertissement, et le paquet passe
/// `validate.py`. Ignoré sans binaire ni `FLAM_TEST_PYTHON`, sauf avec
/// `FLAM_TEST_REQUIRE_CONVERTER=1` (CI, après `npm run build:converter`).
#[test]
fn binaire_convertit_et_valide_l_archive() {
    let required = std::env::var_os("FLAM_TEST_REQUIRE_CONVERTER").is_some_and(|v| v == "1");
    let (Some(exe), Some(python)) = (converter(), python()) else {
        assert!(
            !required,
            "FLAM_TEST_REQUIRE_CONVERTER=1 : telmi2flam.exe ou FLAM_TEST_PYTHON introuvable"
        );
        eprintln!("telmi2flam.exe ou Python de validate.py absent : test ignoré");
        return;
    };
    let validate = validate_script();
    assert!(validate.is_file(), "{}", validate.display());

    let mut fx = Fixture::new();
    let mut menu = three_levels(&mut fx);
    // Titre hors police et sans image : translittération et repli.
    if let MenuEntry::Episode(e) = &mut menu[0] {
        e.title = "L’œuvre — 🎉 un très long titre d’épisode qui déborde".into();
        e.image = None;
    }
    let mut p = pack(menu, Some(fx.cover()));
    // Titre et sous-titre du pack avec une apostrophe typographique (U+2019) : translittérés
    // dans `metadata.json`, sinon avertissement `font_replaced`.
    p.metadata.title = "L’heure du conte".into();
    p.metadata.subtitle = "© Radio d’Exemple".into();
    write_archive(&p, &fx.out(), &never).unwrap();
    let archive = fx.out();
    let work = fx.dir.path();

    // --info
    let (code, stdout, stderr) = run(&exe, false, work, &["--info".as_ref(), archive.as_os_str()]);
    assert_eq!(code, 0, "--info : {stderr}");
    let info: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(info["uuid"], p.uuid.hyphenated().to_string());
    assert_eq!(info["version"], 1);
    assert_eq!(info["title"], "L'heure du conte");
    assert_eq!(info["category"], "© Radio d'Exemple");

    // conversion, événements machine : aucun avertissement
    let pk = fx.path("pack.plain.pk");
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
    let warnings: Vec<&Value> = events.iter().filter(|v| v["event"] == "warning").collect();
    assert!(warnings.is_empty(), "avertissements : {warnings:?}");
    // Sans événement `done`, une sortie illisible ou vide passerait pour « aucun avertissement ».
    assert!(
        events.iter().any(|v| v["event"] == "done"),
        "aucun événement done : {stdout}"
    );

    // uuid.bin = MD5 de la chaîne uuid (§ 3.4), calculé indépendamment par Python
    let pk_files = read_zip_any(&pk);
    let (code, md5, stderr) = run(
        &python,
        true,
        work,
        &[
            "-c".as_ref(),
            "import hashlib,sys;print(hashlib.md5(sys.argv[1].encode()).hexdigest())".as_ref(),
            p.uuid.hyphenated().to_string().as_ref(),
        ],
    );
    assert_eq!(code, 0, "{stderr}");
    let hex: String = pk_files["uuid.bin"]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(hex, md5.trim());
    assert_eq!(pk_files["version"], b"1");
    let info_plain = String::from_utf8(pk_files["info.plain"].clone()).unwrap();
    assert!(
        info_plain.starts_with("L'heure du conte\n© Radio d'Exemple\n"),
        "{info_plain}"
    );

    // validate.py
    let (code, stdout, stderr) = run(&python, true, work, &[validate.as_os_str(), pk.as_os_str()]);
    assert_eq!(code, 0, "validate.py :\n{stdout}\n{stderr}");
    // Aucun `[WARN]` (régression de focus N4 a, de retour…), sauf le contrôle Lua ignoré faute
    // de `lupa` ou d'interpréteur.
    let warns: Vec<&str> = stdout
        .lines()
        .filter(|l| l.trim_start().starts_with("[WARN]"))
        .filter(|l| !l.contains("syntaxe Lua non verifiee"))
        .collect();
    assert!(warns.is_empty(), "validate.py :\n{stdout}");
}

/// Entrées d'un zip quelconque (méthode non vérifiée).
fn read_zip_any(path: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).unwrap();
        out.insert(f.name().to_string(), bytes);
    }
    out
}
