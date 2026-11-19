use std::fs;

use super::*;

fn write(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).unwrap();
}

fn raw(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

#[test]
fn absent_donne_les_valeurs_par_defaut_sans_avertissement() {
    let dir = tempfile::tempdir().unwrap();
    let loaded = load_folder(dir.path());
    assert!(!loaded.exists && !loaded.damaged);
    assert!(loaded.warnings.is_empty());
    assert_eq!(loaded.settings.format, FORMAT);
    let story = load_story(dir.path());
    assert!(!story.exists);
    assert_eq!(story.settings.folder.format, FORMAT);
}

#[test]
fn lit_les_champs_de_la_note() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        STORY_FILE,
        r#"{
          "format": 1, "uuid": "8C1E0000-0000-4000-8000-000000000001", "version": 3,
          "titre": "Le Petit Prince", "sousTitre": "Livre audio", "description": "Lu par…",
          "age": 6, "mode": "chapitres", "debut": "chapitre", "fin": "menu",
          "annoncer": false, "titresLus": { "voix": "Hortense" }
        }"#,
    );
    let loaded = load_story(dir.path());
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    let s = loaded.settings;
    assert_eq!(
        s.uuid.as_deref(),
        Some("8c1e0000-0000-4000-8000-000000000001")
    );
    assert_eq!(s.version, Some(3));
    assert_eq!(s.titre.as_deref(), Some("Le Petit Prince"));
    assert_eq!(s.sous_titre.as_deref(), Some("Livre audio"));
    assert_eq!(s.age, Some(6));
    assert_eq!(s.debut, Some(Debut::Chapitre));
    assert_eq!(s.folder.mode, Some(Mode::Chapitres));
    assert_eq!(s.folder.fin, Some(Fin::Menu));
    assert_eq!(s.folder.annoncer, Some(false));
    assert_eq!(s.titres_lus.unwrap()["voix"], "Hortense");
    assert!(s.folder.extra.is_empty());

    write(
        dir.path(),
        FOLDER_FILE,
        r#"{ "format": 1, "ordre": ["02 Le chat.m4a", "01 Le loup.mp3", "Bonus"],
             "titres": { "01 Le loup.mp3": "Le grand méchant loup" },
             "mode": "recueil", "fin": "suivante" }"#,
    );
    let folder = load_folder(dir.path()).settings;
    assert_eq!(folder.ordre, ["02 Le chat.m4a", "01 Le loup.mp3", "Bonus"]);
    assert_eq!(
        folder.title_of("01 LE LOUP.MP3"),
        Some("Le grand méchant loup")
    );
    assert_eq!(folder.mode, Some(Mode::Recueil));
    assert_eq!(folder.fin, Some(Fin::Suivante));
}

#[test]
fn enumerations_lues_sans_casse() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        STORY_FILE,
        r#"{ "mode": "Chapitres", "fin": " FIN ", "debut": "Menu" }"#,
    );
    let loaded = load_story(dir.path());
    assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
    assert_eq!(loaded.settings.folder.mode, Some(Mode::Chapitres));
    assert_eq!(loaded.settings.folder.fin, Some(Fin::Fin));
    assert_eq!(loaded.settings.debut, Some(Debut::Menu));
    // Réécrites en minuscules.
    write_story(dir.path(), &loaded.settings).unwrap();
    assert_eq!(raw(&dir.path().join(STORY_FILE))["mode"], "chapitres");
}

#[test]
fn json_abime_donne_les_valeurs_par_defaut_et_un_avertissement() {
    for text in ["{ pas du json", "[1, 2]", "\"texte\"", ""] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), FOLDER_FILE, text);
        let loaded = load_folder(dir.path());
        assert!(loaded.exists && loaded.damaged, "{text}");
        assert_eq!(loaded.warnings.len(), 1, "{text}");
        assert_eq!(loaded.warnings[0].code, CODE_UNREADABLE);
        assert_eq!(loaded.settings, FolderSettings::default_new());
    }
}

#[test]
fn bom_utf8_tolere() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join(FOLDER_FILE),
        b"\xEF\xBB\xBF{\"mode\":\"chapitres\"}",
    )
    .unwrap();
    let loaded = load_folder(dir.path());
    assert!(loaded.warnings.is_empty());
    assert_eq!(loaded.settings.mode, Some(Mode::Chapitres));
}

const INVALID_STORY: &str = r#"{ "format": "un", "uuid": "pas-un-uuid", "version": 0, "age": 300,
     "titre": 12, "mode": "roman", "fin": true, "debut": "milieu",
     "annoncer": "oui", "titresLus": [], "ordre": "a",
     "titres": { "a.mp3": 3, "../x": "X", "b.mp3": "  B  " },
     "inconnu": { "garde": true } }"#;

#[test]
fn valeurs_invalides_lues_comme_defaut_avec_avertissement() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), STORY_FILE, INVALID_STORY);
    let loaded = load_story(dir.path());
    assert!(!loaded.damaged);
    let keys: Vec<_> = loaded
        .warnings
        .iter()
        .map(|w| (w.code, w.key.clone().unwrap()))
        .collect();
    for key in [
        "format",
        "uuid",
        "version",
        "age",
        "titre",
        "mode",
        "fin",
        "debut",
        "annoncer",
        "titresLus",
        "ordre",
        "titres",
    ] {
        assert!(
            keys.contains(&(CODE_INVALID_VALUE, key.to_owned())),
            "{key} : {keys:?}"
        );
    }
    let s = loaded.settings;
    assert_eq!(s.uuid, None);
    assert_eq!(s.version, None);
    assert_eq!(s.age, None);
    assert_eq!(s.titre, None);
    assert_eq!(s.folder.mode, None);
    assert_eq!(s.folder.fin, None);
    assert_eq!(s.folder.annoncer, None);
    assert!(s.folder.ordre.is_empty());
    assert_eq!(s.folder.titres.len(), 1);
    assert_eq!(s.folder.titres["b.mp3"], "B");
    assert_eq!(s.folder.extra["inconnu"]["garde"], true);
    assert_eq!(s.folder.extra["mode"], "roman");
}

#[test]
fn valeurs_invalides_survivent_a_la_reecriture() {
    let dir = tempfile::tempdir().unwrap();
    // `format` invalide interdirait toute écriture (`format_invalide_jamais_ecrase`).
    let text = INVALID_STORY.replace(r#""format": "un", "#, "");
    assert_ne!(text, INVALID_STORY);
    write(dir.path(), STORY_FILE, &text);
    let mut story = load_story(dir.path()).settings;
    // L'appelant pose une valeur pour `version` et `mode` seulement.
    story.version = Some(2);
    story.folder.mode = Some(Mode::Recueil);
    write_story(dir.path(), &story).unwrap();
    let raw = raw(&dir.path().join(STORY_FILE));
    assert_eq!(raw["version"], 2);
    assert_eq!(raw["mode"], "recueil");
    assert_eq!(raw["format"], 1);
    for (key, value) in [
        ("uuid", serde_json::json!("pas-un-uuid")),
        ("age", serde_json::json!(300)),
        ("titre", serde_json::json!(12)),
        ("fin", serde_json::json!(true)),
        ("debut", serde_json::json!("milieu")),
        ("annoncer", serde_json::json!("oui")),
        ("titresLus", serde_json::json!([])),
        ("ordre", serde_json::json!("a")),
        ("inconnu", serde_json::json!({ "garde": true })),
    ] {
        assert_eq!(raw[key], value, "{key}");
    }
    // Seules les entrées invalides de `titres` sont abandonnées.
    assert_eq!(raw["titres"], serde_json::json!({ "b.mp3": "B" }));
}

#[test]
fn ordre_sans_doublon_ni_chemin() {
    let (ordre, rejected) = parse_order_items(
        serde_json::json!(["a.mp3", "A.MP3", "x/y", "..", "", 3, "b", "e\u{301}", "\u{e9}"])
            .as_array()
            .unwrap()
            .clone(),
    );
    assert_eq!(ordre, ["a.mp3", "b", "e\u{301}"]);
    assert_eq!(rejected, 4);
}

#[test]
fn cle_de_nom_nfc_et_minuscules() {
    assert_eq!(
        name_key("E\u{301}TE\u{301}.mp3"),
        name_key("\u{e9}t\u{e9}.MP3")
    );
    let settings = FolderSettings {
        titres: [("Été.mp3".to_owned(), "T".to_owned())]
            .into_iter()
            .collect(),
        ..FolderSettings::default()
    };
    assert_eq!(settings.title_of("E\u{301}TE\u{301}.MP3"), Some("T"));
    assert_eq!(
        settings.title_table().get("e\u{301}te\u{301}.mp3"),
        Some("T")
    );
}

#[test]
fn format_plus_recent_lu_au_mieux_jamais_reecrit() {
    let dir = tempfile::tempdir().unwrap();
    let text = r#"{ "format": 7, "mode": "recueil" }"#;
    write(dir.path(), FOLDER_FILE, text);
    let loaded = load_folder(dir.path());
    assert_eq!(loaded.warnings[0].code, CODE_NEWER_FORMAT);
    assert_eq!(loaded.settings.mode, Some(Mode::Recueil));
    let err = write_folder(dir.path(), &loaded.settings).unwrap_err();
    assert!(err.to_string().contains(CODE_NEWER_FORMAT), "{err}");
    assert_eq!(
        fs::read_to_string(dir.path().join(FOLDER_FILE)).unwrap(),
        text
    );
}

#[test]
fn format_invalide_jamais_ecrase() {
    for text in [
        r#"{ "format": "2" }"#,
        r#"{ "format": 0 }"#,
        r#"{ "format": -1 }"#,
    ] {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), FOLDER_FILE, text);
        let loaded = load_folder(dir.path());
        assert_eq!(loaded.warnings[0].code, CODE_INVALID_VALUE, "{text}");
        let err = write_folder(dir.path(), &loaded.settings).unwrap_err();
        assert!(
            err.to_string().contains(CODE_NEWER_FORMAT),
            "{text} : {err}"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join(FOLDER_FILE)).unwrap(),
            text
        );
    }
}

#[test]
fn titres_la_cle_exacte_prime() {
    let settings = FolderSettings {
        titres: [
            ("A.mp3".to_owned(), "Majuscule".to_owned()),
            ("a.mp3".to_owned(), "Minuscule".to_owned()),
        ]
        .into_iter()
        .collect(),
        ..FolderSettings::default()
    };
    let table = settings.title_table();
    for (name, expected) in [
        ("a.mp3", "Minuscule"),
        ("A.mp3", "Majuscule"),
        // Ni l'une ni l'autre exactement : la première dans l'ordre stable (« A » < « a »).
        ("A.MP3", "Majuscule"),
    ] {
        assert_eq!(table.get(name), Some(expected), "{name}");
        assert_eq!(settings.title_of(name), Some(expected), "{name}");
    }
}

#[test]
fn cles_inconnues_conservees_a_la_reecriture() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        STORY_FILE,
        r#"{ "uuid": "8c1e0000-0000-4000-8000-000000000001", "version": 1,
             "titresLus": { "voix": "A", "vitesse": 2 }, "titre": "Avant",
             "futur": [1, 2, 3] }"#,
    );
    let mut story = load_story(dir.path()).settings;
    story.version = Some(2);
    story.folder.mode = Some(Mode::Recueil);
    // Un champ valide vidé par l'appelant disparaît.
    story.titre = None;
    write_story(dir.path(), &story).unwrap();

    let raw = raw(&dir.path().join(STORY_FILE));
    assert_eq!(raw["futur"], serde_json::json!([1, 2, 3]));
    assert_eq!(raw["titresLus"]["vitesse"], 2);
    assert_eq!(raw["version"], 2);
    assert_eq!(raw["mode"], "recueil");
    assert_eq!(raw["format"], 1);
    assert!(raw.get("titre").is_none() && raw.get("ordre").is_none());
    let again = load_story(dir.path());
    assert!(again.warnings.is_empty(), "{:?}", again.warnings);
    assert_eq!(again.settings, story);
    // Aucun temporaire laissé.
    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [STORY_FILE]);
}

#[test]
fn ecriture_compacte_si_l_indentation_depasse_la_borne() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = FolderSettings::default_new();
    // Beaucoup de petites valeurs : l'indentation ajoute environ 5 octets par élément.
    let items: Vec<Value> = (0..200_000).map(|i| Value::from(i % 10)).collect();
    settings.extra.insert("liste".into(), Value::Array(items));
    let compact = serde_json::to_vec(&Value::Object(folder_object(&settings).unwrap())).unwrap();
    assert!((compact.len() as u64) < MAX_SETTINGS_BYTES);
    write_folder(dir.path(), &settings).unwrap();
    let written = fs::read(dir.path().join(FOLDER_FILE)).unwrap();
    assert_eq!(written.len(), compact.len() + 1);
}

#[test]
fn ecriture_bornee() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = FolderSettings::default_new();
    settings
        .extra
        .insert("gros".into(), Value::String("x".repeat(2 * 1024 * 1024)));
    assert!(matches!(
        write_folder(dir.path(), &settings),
        Err(AppError::Validation(_))
    ));
    assert!(!dir.path().join(FOLDER_FILE).exists());
}

#[test]
fn lecture_bornee() {
    let dir = tempfile::tempdir().unwrap();
    let big = format!("{{\"x\":\"{}\"}}", "a".repeat(MAX_SETTINGS_BYTES as usize));
    write(dir.path(), FOLDER_FILE, &big);
    let loaded = load_folder(dir.path());
    assert!(loaded.damaged);
    assert_eq!(loaded.warnings[0].code, CODE_UNREADABLE);
}

#[test]
fn budget_cumule_de_lecture() {
    let dir = tempfile::tempdir().unwrap();
    let text = r#"{ "mode": "chapitres" }"#;
    write(dir.path(), FOLDER_FILE, text);
    let path = dir.path().join(FOLDER_FILE);
    let mut budget = text.len() as u64 + 3;
    let first = load_folder_file(&path, &mut budget);
    assert!(first.warnings.is_empty());
    assert_eq!(budget, 3);
    let second = load_folder_file(&path, &mut budget);
    // Non lu faute de budget : pas abîmé (rien à mettre à la corbeille).
    assert!(second.skipped && !second.damaged);
    assert_eq!(second.warnings[0].code, CODE_UNREADABLE);
    assert_eq!(second.settings.mode, None);
}

#[test]
fn heritage_mode_fin_annoncer() {
    let root = FolderSettings {
        mode: Some(Mode::Recueil),
        fin: Some(Fin::Suivante),
        ..FolderSettings::default()
    };
    let top = Inherited::root(&root, Mode::Chapitres);
    assert_eq!(top.mode, Mode::Recueil);
    assert_eq!(top.effective_fin(), Fin::Suivante);
    assert!(!top.annoncer);

    let livre = FolderSettings {
        mode: Some(Mode::Chapitres),
        annoncer: Some(true),
        ..FolderSettings::default()
    };
    let child = top.child(&livre);
    assert_eq!(child.mode, Mode::Chapitres);
    assert!(child.annoncer);
    // `suivante` héritée n'a pas de sens en chapitres : fin effective `menu`.
    assert_eq!(child.fin, Some(Fin::Suivante));
    assert_eq!(child.effective_fin(), Fin::Menu);
    let partie = child.child(&FolderSettings::default());
    assert_eq!(partie, child);

    let sans_reglage = Inherited::root(&FolderSettings::default(), Mode::Chapitres);
    assert_eq!(sans_reglage.mode, Mode::Chapitres);
    assert_eq!(sans_reglage.effective_fin(), Fin::Menu);
}

#[test]
fn prune_retire_les_noms_disparus() {
    let mut settings = FolderSettings {
        ordre: vec!["a.mp3".into(), "B.mp3".into(), "c".into()],
        titres: [
            ("b.MP3".to_owned(), "B".to_owned()),
            ("z".into(), "Z".into()),
        ]
        .into_iter()
        .collect(),
        ..FolderSettings::default()
    };
    let existing: HashSet<String> = [name_key("b.mp3"), name_key("c")].into_iter().collect();
    settings.prune(&existing);
    assert_eq!(settings.ordre, ["B.mp3", "c"]);
    assert_eq!(settings.titres.keys().collect::<Vec<_>>(), ["b.MP3"]);
}

#[test]
fn reglage_en_lien_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    write(other.path(), FOLDER_FILE, r#"{ "mode": "chapitres" }"#);
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(
        other.path().join(FOLDER_FILE),
        dir.path().join(FOLDER_FILE),
    );
    #[cfg(unix)]
    let linked =
        std::os::unix::fs::symlink(other.path().join(FOLDER_FILE), dir.path().join(FOLDER_FILE));
    if let Err(e) = linked {
        eprintln!("reglage_en_lien_refuse : lien symbolique impossible ({e}), test sauté");
        return;
    }
    let loaded = load_folder(dir.path());
    assert!(loaded.damaged);
    assert_eq!(loaded.settings.mode, None);
}
