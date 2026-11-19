use std::fs;

use super::*;
use crate::authoring::settings::Fin;

/// Crée `rel` (et ses dossiers) sous `root`, avec `len` octets.
fn touch(root: &Path, rel: &str, len: usize) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, vec![0u8; len]).unwrap();
}

fn write(root: &Path, rel: &str, text: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn scan_ok(root: &Path) -> Scan {
    scan(root, &Limits::default(), &[]).unwrap()
}

/// Ids des options du menu `id`.
fn ids(scan: &Scan, id: &str) -> Vec<String> {
    scan.root
        .find(id)
        .unwrap_or_else(|| panic!("{id} absent"))
        .children()
        .iter()
        .map(|n| n.id().to_owned())
        .collect()
}

fn codes(scan: &Scan) -> Vec<(&'static str, String)> {
    scan.problems
        .iter()
        .map(|p| (p.code, p.path.clone()))
        .collect()
}

fn has(scan: &Scan, code: &str, path: &str) -> bool {
    scan.problems
        .iter()
        .any(|p| p.code == code && p.path == path)
}

fn menu_fields(node: &Node) -> (Mode, Fin, bool, Option<&str>, Option<&str>) {
    match node {
        Node::Menu {
            mode,
            fin,
            annoncer,
            image,
            title_audio,
            ..
        } => (
            *mode,
            *fin,
            *annoncer,
            image.as_deref(),
            title_audio.as_deref(),
        ),
        Node::Story { .. } => panic!("pas un menu"),
    }
}

fn story_fields(node: &Node) -> (Option<&str>, Option<&str>) {
    match node {
        Node::Story {
            image, title_audio, ..
        } => (image.as_deref(), title_audio.as_deref()),
        Node::Menu { .. } => panic!("pas une histoire"),
    }
}

// --- Tri et titres ---

#[test]
fn tri_naturel_sans_casse_ni_accents() {
    let mut names = vec![
        "10 b.mp3",
        "2 a.mp3",
        "Eu.mp3",
        "1 c.mp3",
        "Été.mp3",
        "ete 2.mp3",
        "002 z.mp3",
    ];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(
        names,
        [
            "1 c.mp3",
            "2 a.mp3",
            "002 z.mp3",
            "10 b.mp3",
            "ete 2.mp3",
            "Été.mp3",
            "Eu.mp3"
        ]
    );
    assert_eq!(natural_cmp("a", "A"), "a".cmp("A"));
    assert_eq!(natural_cmp("x2", "x10"), Ordering::Less);
}

#[test]
fn prefixe_de_tri_retire() {
    for (name, title) in [
        ("01 - Le loup", "Le loup"),
        ("02_Chat", "Chat"),
        ("3) Trois", "Trois"),
        ("4.Quatre", "Quatre"),
        ("05Cinq", "Cinq"),
        ("-5 degrés", "-5 degrés"),
        ("Sans numéro", "Sans numéro"),
        ("12", ""),
    ] {
        assert_eq!(strip_sort_prefix(name), title, "{name}");
    }
    assert_eq!(title_from_name("01 - Le loup.mp3", true), "Le loup");
    assert_eq!(title_from_name("07.mp3", true), "07");
    assert_eq!(title_from_name("01 Animaux", false), "Animaux");
    assert_eq!(title_from_name("2.5 Vol.1", false), "5 Vol.1");
    assert_eq!(
        title_from_name("Le\u{202E} loup  \t bleu.mp3", true),
        "Le loup bleu"
    );
}

// --- Lecture du dossier ---

#[test]
fn arbre_ordre_naturel_et_titres_deduits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "10 Dix.mp3", 10);
    touch(root, "2 Deux.m4a", 20);
    touch(root, "01 Animaux/02 Le chat.ogg", 5);
    touch(root, "01 Animaux/01 - Le loup.mp3", 5);
    let scan = scan_ok(root);
    assert_eq!(ids(&scan, ""), ["01 Animaux", "2 Deux.m4a", "10 Dix.mp3"]);
    assert_eq!(
        ids(&scan, "01 Animaux"),
        ["01 Animaux/01 - Le loup.mp3", "01 Animaux/02 Le chat.ogg"]
    );
    assert_eq!(scan.root.find("01 Animaux").unwrap().title(), "Animaux");
    assert_eq!(
        scan.root
            .find("01 Animaux/01 - Le loup.mp3")
            .unwrap()
            .title(),
        "Le loup"
    );
    assert_eq!(scan.story_count, 4);
    assert_eq!(scan.audio_bytes, 40);
    // Titre de la racine : nom du dossier.
    let root_name = root.file_name().unwrap().to_string_lossy();
    assert_eq!(scan.root.title(), title_from_name(&root_name, false));
    assert!(!scan.is_blocked(), "{:?}", scan.problems);
    assert!(scan.problems.is_empty(), "{:?}", scan.problems);
}

#[test]
fn ordre_et_titres_des_reglages() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a.mp3", 1);
    touch(root, "b.mp3", 1);
    touch(root, "c.mp3", 1);
    touch(root, "Bonus/x.mp3", 1);
    touch(root, "Bonus/y.mp3", 1);
    touch(root, "Bonus/z.mp3", 1);
    write(
        root,
        "_histoire.json",
        r#"{ "titre": "Mon histoire", "ordre": ["C.MP3", "disparu.mp3", "Bonus"],
             "titres": { "Bonus": "Les bonus", "b.mp3": "Le B" } }"#,
    );
    write(
        root,
        "Bonus/_dossier.json",
        r#"{ "ordre": ["z.mp3"], "titres": { "y.mp3": "Le Y" } }"#,
    );
    let scan = scan_ok(root);
    // `ordre` d'abord (sans casse, entrée disparue ignorée), puis tri naturel.
    assert_eq!(ids(&scan, ""), ["c.mp3", "Bonus", "a.mp3", "b.mp3"]);
    assert_eq!(
        ids(&scan, "Bonus"),
        ["Bonus/z.mp3", "Bonus/x.mp3", "Bonus/y.mp3"]
    );
    assert_eq!(scan.root.title(), "Mon histoire");
    assert_eq!(scan.root.find("Bonus").unwrap().title(), "Les bonus");
    assert_eq!(scan.root.find("b.mp3").unwrap().title(), "Le B");
    assert_eq!(scan.root.find("Bonus/y.mp3").unwrap().title(), "Le Y");
    assert!(scan.problems.is_empty(), "{:?}", scan.problems);
}

#[test]
fn appariement_images_et_titres_lus() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "_couverture.png", 1);
    touch(root, "_titre.mp3", 1);
    touch(root, "01 Le loup.mp3", 1);
    touch(root, "01 Le loup.JPG", 1);
    touch(root, "01 Le loup.titre.mp3", 1);
    touch(root, "Autre.png", 1);
    touch(root, "Seul.titre.mp3", 1);
    touch(root, "M/_image.png", 1);
    touch(root, "M/_image.jpg", 1);
    touch(root, "M/_titre.wav", 1);
    touch(root, "M/a.flac", 1);
    let scan = scan_ok(root);

    let (_, _, _, cover, title) = menu_fields(&scan.root);
    assert_eq!(cover, Some("_couverture.png"));
    assert_eq!(title, Some("_titre.mp3"));
    let loup = scan.root.find("01 Le loup.mp3").unwrap();
    assert_eq!(
        story_fields(loup),
        (Some("01 Le loup.JPG"), Some("01 Le loup.titre.mp3"))
    );
    // Titre lu sans fichier audio du même nom : une histoire ordinaire.
    let seul = scan.root.find("Seul.titre.mp3").unwrap();
    assert_eq!(seul.title(), "Seul.titre");
    assert_eq!(scan.story_count, 3);
    let (_, _, _, image, title) = menu_fields(scan.root.find("M").unwrap());
    assert_eq!(image, Some("M/_image.jpg"));
    assert_eq!(title, Some("M/_titre.wav"));

    assert!(
        has(&scan, "orphan_image", "Autre.png"),
        "{:?}",
        codes(&scan)
    );
    assert!(has(&scan, "duplicate_special_file", "M/_image.png"));
    assert_eq!(scan.problems.len(), 2, "{:?}", codes(&scan));
    assert!(!scan.is_blocked());
}

#[test]
fn deux_audios_du_meme_nom_partagent_l_image() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a.mp3", 1);
    touch(root, "a.m4a", 1);
    touch(root, "a.png", 1);
    touch(root, "a.bmp", 1);
    let scan = scan_ok(root);
    for id in ["a.mp3", "a.m4a"] {
        assert_eq!(story_fields(scan.root.find(id).unwrap()).0, Some("a.bmp"));
        assert!(has(&scan, "duplicate_story_name", id));
    }
    // Doublon d'image signalé une seule fois.
    let dups = scan
        .problems
        .iter()
        .filter(|p| p.code == "duplicate_special_file")
        .count();
    assert_eq!(dups, 1, "{:?}", codes(&scan));
}

#[test]
fn fichiers_speciaux_mal_places() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a.mp3", 1);
    write(root, "_dossier.json", r#"{ "mode": "chapitres" }"#);
    touch(root, "_image.png", 1);
    touch(root, "M/b.mp3", 1);
    write(root, "M/_histoire.json", "{}");
    touch(root, "M/_couverture.png", 1);
    let scan = scan_ok(root);
    for path in [
        "_dossier.json",
        "_image.png",
        "M/_histoire.json",
        "M/_couverture.png",
    ] {
        assert!(
            has(&scan, "misplaced_special_file", path),
            "{path} : {:?}",
            codes(&scan)
        );
    }
    // `_dossier.json` de la racine non lu : mode proposé.
    assert_eq!(scan.story.settings.folder.mode, None);
    assert_eq!(scan.suggested_mode, Some(Mode::Recueil));
}

#[test]
fn ignores_en_silence_ou_signales() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a.mp3", 1);
    touch(root, ".flam/corbeille/vieux.mp3", 1);
    touch(root, ".cache.mp3", 1);
    touch(root, "desktop.ini", 1);
    touch(root, "Thumbs.db", 1);
    touch(root, ".DS_Store", 1);
    touch(root, "M/DESKTOP.INI", 1);
    touch(root, "M/b.mp3", 1);
    touch(root, "notes.txt", 1);
    touch(root, "M/video.mp4", 1);
    touch(root, "_autre.json", 1);
    let scan = scan_ok(root);
    assert_eq!(ids(&scan, ""), ["a.mp3", "M"]);
    assert_eq!(ids(&scan, "M"), ["M/b.mp3"]);
    let mut found = codes(&scan);
    found.sort();
    assert_eq!(
        found,
        [
            ("unsupported_file", "M/video.mp4".to_owned()),
            ("unsupported_file", "_autre.json".to_owned()),
            ("unsupported_file", "notes.txt".to_owned()),
        ]
    );
}

#[cfg(windows)]
#[test]
fn fichier_cache_windows_ignore() {
    use std::os::windows::ffi::OsStrExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    touch(root, "a.mp3", 1);
    touch(root, "cache.mp3", 1);
    let wide: Vec<u16> = root
        .join("cache.mp3")
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // SAFETY : chaîne terminée par un zéro, vivante pendant l'appel.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
            wide.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
        )
    };
    assert_ne!(ok, 0);
    let scan = scan_ok(root);
    assert_eq!(ids(&scan, ""), ["a.mp3"]);
    assert!(scan.problems.is_empty());
}

#[test]
fn heritage_du_mode_de_la_fin_et_de_l_annonce() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "_histoire.json",
        r#"{ "mode": "recueil", "fin": "suivante" }"#,
    );
    touch(root, "a.mp3", 1);
    write(
        root,
        "Livre/_dossier.json",
        r#"{ "mode": "chapitres", "annoncer": true }"#,
    );
    touch(root, "Livre/Partie 1/01.mp3", 1);
    write(root, "Livre/Partie 2/_dossier.json", r#"{ "fin": "fin" }"#);
    touch(root, "Livre/Partie 2/02.mp3", 1);
    write(root, "Autre/_dossier.json", r#"{ "fin": "fin" }"#);
    touch(root, "Autre/b.mp3", 1);
    let scan = scan_ok(root);
    assert_eq!(scan.suggested_mode, None);
    let top = menu_fields(&scan.root);
    assert_eq!((top.0, top.1, top.2), (Mode::Recueil, Fin::Suivante, false));
    let livre = menu_fields(scan.root.find("Livre").unwrap());
    // `suivante` héritée n'a pas de sens en chapitres : `menu`, sans avertissement.
    assert_eq!(
        (livre.0, livre.1, livre.2),
        (Mode::Chapitres, Fin::Menu, true)
    );
    let p1 = menu_fields(scan.root.find("Livre/Partie 1").unwrap());
    assert_eq!((p1.0, p1.1, p1.2), (Mode::Chapitres, Fin::Menu, true));
    let p2 = menu_fields(scan.root.find("Livre/Partie 2").unwrap());
    assert_eq!((p2.0, p2.1, p2.2), (Mode::Chapitres, Fin::Fin, true));
    // `fin` réglée sans rapport avec le mode : signalée.
    let autre = menu_fields(scan.root.find("Autre").unwrap());
    assert_eq!((autre.0, autre.1), (Mode::Recueil, Fin::Menu));
    assert_eq!(
        codes(&scan),
        [("fin_ignored", "Autre/_dossier.json".to_owned())]
    );
}

#[test]
fn json_abime_signale_sans_bloquer() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "_histoire.json", "{ abîmé");
    touch(root, "M/a.mp3", 1);
    write(root, "M/_dossier.json", r#"{ "mode": 3, "ordre": "x" }"#);
    let scan = scan_ok(root);
    assert!(scan.story.damaged);
    assert!(has(&scan, "settings_unreadable", "_histoire.json"));
    assert!(has(&scan, "settings_invalid_value", "M/_dossier.json"));
    assert!(!scan.is_blocked());
    assert_eq!(ids(&scan, "M"), ["M/a.mp3"]);
}

// --- Mode proposé ---

#[test]
fn mode_propose() {
    let cases: [(&[&str], Mode); 5] = [
        (
            &["01 Début.mp3", "02 Suite.mp3", "10 Fin.mp3"],
            Mode::Chapitres,
        ),
        (
            &["Partie 1/01.mp3", "Partie 2/01.mp3", "Partie 2/02.mp3"],
            Mode::Chapitres,
        ),
        (&["01 Début.mp3", "Bonus.mp3"], Mode::Recueil),
        (&["01 a.mp3", "Partie/b.mp3"], Mode::Recueil),
        (&["Le loup.mp3"], Mode::Recueil),
    ];
    for (files, expected) in cases {
        let dir = tempfile::tempdir().unwrap();
        for file in files {
            touch(dir.path(), file, 1);
        }
        // Images, titres lus et dossiers vides ne changent rien.
        touch(dir.path(), "_couverture.png", 1);
        fs::create_dir(dir.path().join("Vide")).unwrap();
        let scan = scan_ok(dir.path());
        assert_eq!(scan.suggested_mode, Some(expected), "{files:?}");
        assert_eq!(menu_fields(&scan.root).0, expected, "{files:?}");
    }
    // Mode réglé : rien n'est proposé.
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "01.mp3", 1);
    write(dir.path(), "_histoire.json", r#"{ "mode": "recueil" }"#);
    let scan = scan_ok(dir.path());
    assert_eq!(scan.suggested_mode, None);
    assert_eq!(menu_fields(&scan.root).0, Mode::Recueil);
}

// --- Limites ---

#[test]
fn dossier_vide_et_menu_vide() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("Vide")).unwrap();
    let scan = scan_ok(dir.path());
    assert!(has(&scan, "no_story", ""));
    assert!(has(&scan, "empty_menu", "Vide"));
    assert!(scan.is_blocked());
}

#[test]
fn profondeur_bornee() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "1/2/3/4/a.mp3", 1);
    touch(dir.path(), "1/2/3/4/5/b.mp3", 1);
    touch(dir.path(), "1/2/3/4/5/6/7/c.mp3", 1);
    let scan = scan_ok(dir.path());
    // Racine = niveau 1 : « 1/2/3/4 » est le niveau 5, « 5 » ne peut pas être lu.
    assert!(scan.root.find("1/2/3/4/a.mp3").is_some());
    assert!(scan.root.find("1/2/3/4/5").is_none());
    assert_eq!(codes(&scan), [("too_deep", "1/2/3/4/5".to_owned())]);
    assert!(scan.is_blocked());
    assert_eq!(scan.story_count, 1);
}

#[test]
fn dossier_trop_profond_sans_audio_seulement_signale() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "1/2/3/4/a.mp3", 1);
    touch(dir.path(), "1/2/3/4/Scans/6/couverture.png", 1);
    let scan = scan_ok(dir.path());
    assert_eq!(codes(&scan), [("too_deep", "1/2/3/4/Scans".to_owned())]);
    assert_eq!(scan.problems[0].severity, Severity::Warning);
    assert!(!scan.is_blocked());
}

#[test]
fn chaine_de_dossiers_vides_signalee_une_fois() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "a.mp3", 1);
    fs::create_dir_all(dir.path().join("Vide/Encore/Toujours")).unwrap();
    fs::create_dir_all(dir.path().join("Autre vide")).unwrap();
    let scan = scan_ok(dir.path());
    let mut found = codes(&scan);
    found.sort();
    assert_eq!(
        found,
        [
            ("empty_menu", "Autre vide".to_owned()),
            ("empty_menu", "Vide".to_owned()),
        ]
    );
}

#[test]
fn limites_metier_bloquantes() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..4 {
        touch(dir.path(), &format!("{i}.mp3"), 100);
    }
    touch(dir.path(), "M/a.mp3", 100);
    let limits = Limits {
        max_stories: 4,
        max_options_per_menu: 4,
        max_audio_bytes: 499,
        ..Limits::default()
    };
    let scan = scan(dir.path(), &limits, &[]).unwrap();
    let mut found = codes(&scan);
    found.sort();
    assert_eq!(
        found,
        [
            ("audio_too_large", String::new()),
            ("too_many_options", String::new()),
            ("too_many_stories", String::new()),
        ]
    );
    assert!(scan
        .problems
        .iter()
        .all(|p| p.severity == Severity::Blocking));
    // L'arbre reste complet.
    assert_eq!(scan.story_count, 5);
    assert!(!scan.truncated);
}

#[test]
fn parcours_borne_par_le_nombre_d_entrees() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..5 {
        touch(dir.path(), &format!("A/{i}.mp3"), 1);
        touch(dir.path(), &format!("B/{i}.mp3"), 1);
    }
    let limits = Limits {
        max_entries: 8,
        ..Limits::default()
    };
    let scan = scan(dir.path(), &limits, &[]).unwrap();
    assert!(scan.truncated);
    assert_eq!(scan.entries_seen, 8);
    assert!(has(&scan, "too_many_entries", "B"), "{:?}", codes(&scan));
    assert!(scan.is_blocked());
    // 2 entrées à la racine + 5 dans A + 1 dans B.
    assert_eq!(scan.story_count, 6);
}

// --- Sécurité ---

#[test]
fn racines_interdites_par_l_appelant() {
    // Les cas de forme et de résolution sont testés dans `authoring::root`.
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "Launcher/Histoire/a.mp3", 1);
    let launcher = dir.path().join("Launcher");
    for root in [
        launcher.clone(),
        launcher.join("Histoire"),
        dir.path().to_path_buf(),
    ] {
        assert!(
            matches!(
                scan(&root, &Limits::default(), std::slice::from_ref(&launcher)),
                Err(ScanError::Forbidden { .. })
            ),
            "{root:?}"
        );
    }
    assert!(scan(&launcher.join("Histoire"), &Limits::default(), &[]).is_ok());
}

#[test]
fn racine_absente_ou_fichier() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        scan(&dir.path().join("absent"), &Limits::default(), &[]),
        Err(ScanError::NotFound(_))
    ));
    touch(dir.path(), "a.mp3", 1);
    assert!(matches!(
        scan(&dir.path().join("a.mp3"), &Limits::default(), &[]),
        Err(ScanError::NotADirectory(_))
    ));
}

#[test]
fn liens_symboliques_ignores() {
    let dir = tempfile::tempdir().unwrap();
    let real = tempfile::tempdir().unwrap();
    touch(real.path(), "cible.mp3", 1);
    touch(real.path(), "sous/b.mp3", 1);
    touch(dir.path(), "a.mp3", 1);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(
        real.path().join("cible.mp3"),
        dir.path().join("lien.mp3"),
    )
    .and(std::os::windows::fs::symlink_dir(
        real.path().join("sous"),
        dir.path().join("lien-dossier"),
    ));
    #[cfg(unix)]
    let linked =
        std::os::unix::fs::symlink(real.path().join("cible.mp3"), dir.path().join("lien.mp3")).and(
            std::os::unix::fs::symlink(real.path().join("sous"), dir.path().join("lien-dossier")),
        );
    if let Err(e) = linked {
        // Le cas des dossiers est couvert sans privilège par `jonction_ignoree`.
        eprintln!("liens_symboliques_ignores : lien symbolique impossible ({e}), test sauté");
        return;
    }
    let scan = scan_ok(dir.path());
    assert_eq!(ids(&scan, ""), ["a.mp3"]);
    assert!(has(&scan, "link_ignored", "lien.mp3"));
    assert!(has(&scan, "link_ignored", "lien-dossier"));
    // Racine en lien : refusée.
    assert!(matches!(
        super::scan(&dir.path().join("lien-dossier"), &Limits::default(), &[]),
        Err(ScanError::Link(_))
    ));
}

#[cfg(windows)]
#[test]
fn jonction_ignoree() {
    let dir = tempfile::tempdir().unwrap();
    let real = tempfile::tempdir().unwrap();
    touch(real.path(), "b.mp3", 1);
    touch(dir.path(), "a.mp3", 1);
    let link = dir.path().join("jonction");
    // Jonction (sans privilège) ; séparateurs Windows pour `mklink`.
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link.to_string_lossy().replace('/', "\\"))
        .arg(real.path().to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(status.status.success(), "mklink /J a échoué");
    let scan = scan_ok(dir.path());
    assert_eq!(ids(&scan, ""), ["a.mp3"]);
    assert_eq!(codes(&scan), [("link_ignored", "jonction".to_owned())]);
    assert!(matches!(
        super::scan(&link, &Limits::default(), &[]),
        Err(ScanError::Link(_))
    ));
}

// --- Divers ---

#[test]
fn titre_trop_long_signale() {
    let dir = tempfile::tempdir().unwrap();
    let long = "x".repeat(LABEL_MAX + 1);
    touch(dir.path(), &format!("{long}.mp3"), 1);
    touch(dir.path(), &format!("{}.mp3", "y".repeat(LABEL_MAX)), 1);
    let scan = scan_ok(dir.path());
    assert_eq!(codes(&scan), [("title_too_long", format!("{long}.mp3"))]);
}

#[test]
fn serialisation_de_l_arbre() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "M/01 a.mp3", 3);
    let scan = scan_ok(dir.path());
    let json = serde_json::to_value(&scan.root).unwrap();
    assert_eq!(json["kind"], "menu");
    assert_eq!(json["mode"], "chapitres");
    assert_eq!(json["fin"], "menu");
    let story = &json["children"][0]["children"][0];
    assert_eq!(story["kind"], "story");
    assert_eq!(story["id"], "M/01 a.mp3");
    assert_eq!(story["file"], "01 a.mp3");
    assert_eq!(story["title"], "a");
    assert_eq!(story["bytes"], 3);
    let problem = serde_json::to_value(Problem {
        code: "no_story",
        severity: Severity::Blocking,
        path: String::new(),
        message: String::new(),
    })
    .unwrap();
    assert_eq!(problem["severity"], "blocking");
}

// --- Corrections de revue ---

#[cfg(windows)]
fn hide(path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY : chaîne terminée par un zéro, vivante pendant l'appel.
    let ok = unsafe {
        windows_sys::Win32::Storage::FileSystem::SetFileAttributesW(
            wide.as_ptr(),
            windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN,
        )
    };
    assert_ne!(ok, 0);
}

#[cfg(windows)]
#[test]
fn noms_finissant_par_un_point_ou_une_espace_sans_id() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "a.mp3", 1);
    // Seule la forme verbatim crée ces noms (Win32 retire le point ou l'espace final).
    let verbatim = |name: &str| PathBuf::from(format!(r"\\?\{}\{name}", dir.path().display()));
    fs::write(verbatim("b.mp3."), b"x").unwrap();
    fs::write(verbatim("c.mp3 "), b"x").unwrap();
    fs::create_dir(verbatim("Menu.")).unwrap();
    let scan = scan_ok(dir.path());
    assert_eq!(ids(&scan, ""), ["a.mp3"]);
    let mut found = codes(&scan);
    found.sort();
    assert_eq!(
        found,
        [
            ("invalid_name", "Menu.".to_owned()),
            ("invalid_name", "b.mp3.".to_owned()),
            ("invalid_name", "c.mp3 ".to_owned()),
        ]
    );
    // Nettoyage : `tempfile` passe par Win32 et ne saurait pas effacer ces noms.
    fs::remove_file(verbatim("b.mp3.")).unwrap();
    fs::remove_file(verbatim("c.mp3 ")).unwrap();
    fs::remove_dir(verbatim("Menu.")).unwrap();
}

#[cfg(windows)]
#[test]
fn reglages_caches_lus_sous_leur_nom_reel() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "a.mp3", 1);
    touch(dir.path(), "M/b.mp3", 1);
    write(dir.path(), "_Histoire.JSON", r#"{ "titre": "Caché" }"#);
    write(dir.path(), "M/_DOSSIER.JSON", r#"{ "fin": "fin" }"#);
    hide(&dir.path().join("_Histoire.JSON"));
    hide(&dir.path().join("M/_DOSSIER.JSON"));
    let scan = scan_ok(dir.path());
    assert_eq!(scan.root.title(), "Caché");
    // `fin` du sous-dossier lue (puis ignorée en recueil), signalée sous le nom réel.
    assert_eq!(
        codes(&scan),
        [("fin_ignored", "M/_DOSSIER.JSON".to_owned())]
    );
}

#[test]
fn reglages_en_lien_vus_par_le_scan() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    write(other.path(), "_dossier.json", r#"{ "mode": "chapitres" }"#);
    touch(dir.path(), "M/b.mp3", 1);
    let link = dir.path().join("M").join("_dossier.json");
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(other.path().join("_dossier.json"), &link);
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(other.path().join("_dossier.json"), &link);
    if let Err(e) = linked {
        eprintln!(
            "reglages_en_lien_vus_par_le_scan : lien symbolique impossible ({e}), test sauté"
        );
        return;
    }
    let scan = scan_ok(dir.path());
    assert!(
        has(&scan, "link_ignored", "M/_dossier.json"),
        "{:?}",
        codes(&scan)
    );
    assert_eq!(menu_fields(scan.root.find("M").unwrap()).0, Mode::Recueil);
}

#[test]
fn noms_compares_en_nfc_sans_casse() {
    let dir = tempfile::tempdir().unwrap();
    // Fichier audio en NFD, image et titre lu en NFC, réglages en NFC majuscules.
    touch(dir.path(), "e\u{301}te\u{301}.mp3", 1);
    touch(dir.path(), "\u{e9}t\u{e9}.png", 1);
    touch(dir.path(), "\u{c9}T\u{c9}.titre.mp3", 1);
    touch(dir.path(), "zz.mp3", 1);
    write(
        dir.path(),
        "_histoire.json",
        r#"{ "ordre": ["ZZ.MP3", "ÉTÉ.MP3"], "titres": { "ÉTÉ.MP3": "L'été" } }"#,
    );
    let scan = scan_ok(dir.path());
    assert_eq!(ids(&scan, ""), ["zz.mp3", "e\u{301}te\u{301}.mp3"]);
    let ete = scan.root.find("e\u{301}te\u{301}.mp3").unwrap();
    assert_eq!(ete.title(), "L'été");
    assert_eq!(
        story_fields(ete),
        (Some("\u{e9}t\u{e9}.png"), Some("\u{c9}T\u{c9}.titre.mp3"))
    );
    assert!(scan.problems.is_empty(), "{:?}", codes(&scan));
}

#[test]
fn le_scan_ne_garde_pas_les_cles_inconnues() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "a.mp3", 1);
    write(
        dir.path(),
        "_histoire.json",
        r#"{ "futur": 1, "mode": "roman" }"#,
    );
    let scan = scan_ok(dir.path());
    assert!(scan.story.settings.folder.extra.is_empty());
    assert!(has(&scan, "settings_invalid_value", "_histoire.json"));
}

#[test]
fn budget_cumule_des_reglages() {
    let dir = tempfile::tempdir().unwrap();
    let big = format!("{{\"x\":\"{}\"}}", "a".repeat(1_000_000));
    for i in 0..18 {
        touch(dir.path(), &format!("M{i:02}/a.mp3"), 1);
        write(dir.path(), &format!("M{i:02}/_dossier.json"), &big);
    }
    let scan = scan_ok(dir.path());
    let unreadable: Vec<_> = scan
        .problems
        .iter()
        .filter(|p| p.code == "settings_unreadable")
        .map(|p| p.path.clone())
        .collect();
    // 16 Mio de budget : 16 fichiers de ~1 Mo lus, les suivants non.
    assert_eq!(unreadable, ["M16/_dossier.json", "M17/_dossier.json"]);
    assert!(!scan.is_blocked());
}

#[test]
fn coup_d_oeil_des_dossiers_trop_profonds_borne_globalement() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "a.mp3", 1);
    touch(dir.path(), "A/1.png", 1);
    touch(dir.path(), "A/2.png", 1);
    touch(dir.path(), "B/1.png", 1);
    touch(dir.path(), "B/2.png", 1);
    let limits = Limits {
        max_menu_depth: 1,
        max_peek_entries: 3,
        ..Limits::default()
    };
    let scan = scan(dir.path(), &limits, &[]).unwrap();
    let deep: Vec<_> = scan
        .problems
        .iter()
        .filter(|p| p.code == "too_deep")
        .map(|p| (p.path.as_str(), p.severity))
        .collect();
    // A vu en entier (2 entrées, sans audio) ; budget épuisé dans B : supposé avec audio.
    assert_eq!(deep, [("A", Severity::Warning), ("B", Severity::Blocking)]);
}

#[test]
fn base_resolue_exposee() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "Histoire/a.mp3", 1);
    let scan = scan_ok(&dir.path().join("Histoire"));
    let expected =
        crate::authoring::root::simplify(fs::canonicalize(dir.path().join("Histoire")).unwrap());
    assert_eq!(scan.base, expected);
    assert!(!scan.base.to_string_lossy().starts_with(r"\\?\"));
}

#[cfg(windows)]
#[test]
fn base_resolue_a_travers_une_jonction_parente() {
    let dir = tempfile::tempdir().unwrap();
    touch(dir.path(), "reel/Histoire/a.mp3", 1);
    let alias = dir.path().join("alias");
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(alias.to_string_lossy().replace('/', "\\"))
        .arg(dir.path().join("reel").to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(status.status.success(), "mklink /J a échoué");
    let scan = scan_ok(&alias.join("Histoire"));
    let expected = crate::authoring::root::simplify(
        fs::canonicalize(dir.path().join("reel").join("Histoire")).unwrap(),
    );
    assert_eq!(scan.base, expected);
    assert_eq!(ids(&scan, ""), ["a.mp3"]);
}
