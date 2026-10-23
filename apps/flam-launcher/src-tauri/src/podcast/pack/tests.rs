use super::*;

/// Fragment des URL (enclosures, images) : ne doit apparaître dans aucun rapport.
const SECRET: &str = "SECRET42";

fn ep(guid: &str, length: Option<u64>, duration: Option<u32>) -> PodcastEpisode {
    PodcastEpisode {
        guid: guid.into(),
        title: format!("Titre {guid}"),
        description: None,
        pub_date: None,
        duration,
        author: None,
        image: Some(format!("https://img.example/{SECRET}/{guid}.jpg")),
        enclosure_url: format!("https://audio.example/{SECRET}/{guid}.mp3"),
        enclosure_type: "audio/mpeg".into(),
        enclosure_length: length,
        explicit: None,
    }
}

fn feed(title: &str, guids: &[&str]) -> PodcastFeed {
    PodcastFeed {
        title: title.into(),
        description: None,
        image: Some(format!("https://img.example/{SECRET}/canal.jpg")),
        link: None,
        copyright: Some(format!("© {title}")),
        explicit: None,
        episodes: guids
            .iter()
            .map(|g| ep(g, Some(10_000_000), None))
            .collect(),
        warnings: Vec::new(),
    }
}

fn r(source: &str, guid: &str) -> EpisodeRef {
    EpisodeRef {
        source_id: source.into(),
        episode_id: episode_id(guid),
    }
}

fn e(source: &str, guid: &str) -> PackNode {
    PackNode::Episode(r(source, guid))
}

fn cat(name: &str, entries: Vec<PackNode>) -> PackNode {
    PackNode::Category {
        name: name.into(),
        entries,
    }
}

fn comp(menu: Vec<PackNode>) -> PackComposition {
    PackComposition {
        menu,
        ..PackComposition::default()
    }
}

fn guids(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("g{i}")).collect()
}

/// Flux « a » de `n` épisodes g0…g(n-1) et flux « b » (h0, h1).
fn feeds(n: usize) -> Feeds {
    let g = guids(n);
    let g: Vec<&str> = g.iter().map(String::as_str).collect();
    let mut f = Feeds::new();
    f.insert("a".into(), CachedFeed::Ready(feed("Contes", &g)));
    f.insert(
        "b".into(),
        CachedFeed::Ready(feed("Comptines", &["h0", "h1"])),
    );
    f
}

fn codes(report: &ValidationReport) -> Vec<ProblemCode> {
    report.problems.iter().map(|p| p.code).collect()
}

fn assert_no_secret(report: &ValidationReport) {
    let json = serde_json::to_string(report).unwrap();
    assert!(!json.contains(SECRET), "URL dans le rapport : {json}");
}

// ----- Validation -----

#[test]
fn composition_simple_valide_avec_defauts_d16() {
    let c = comp(vec![e("a", "g0"), cat("Soir", vec![e("a", "g1")])]);
    let report = validate(&c, &feeds(2));
    assert!(report.valid, "{:?}", report.problems);
    assert_eq!(report.episodes, 2);
    assert_eq!(report.feeds, 1);
    assert_eq!(report.estimated_bytes, 20_000_000);
    let meta = report.metadata.clone().unwrap();
    assert_eq!(meta.title, "Contes");
    assert_eq!(meta.subtitle, "© Contes");
    assert_eq!(meta.description, None);
    assert_eq!(
        report.cover,
        Some(CoverChoice::Feed {
            source_id: "a".into()
        })
    );
    assert_no_secret(&report);
}

#[test]
fn trois_niveaux_acceptes_quatre_refuses() {
    let ok = comp(vec![cat("N2", vec![cat("N3", vec![e("a", "g0")])])]);
    assert!(validate(&ok, &feeds(1)).valid);
    let deep = comp(vec![cat(
        "N2",
        vec![cat("N3", vec![cat("N4", vec![e("a", "g0")])])],
    )]);
    let report = validate(&deep, &feeds(1));
    assert_eq!(codes(&report), vec![ProblemCode::TooDeep]);
    assert_eq!(report.problems[0].path, Some(vec![0, 0, 0]));

    // Catégorie de niveau 3 contenant plusieurs catégories : chacune est signalée, une fois.
    let wide = comp(vec![cat(
        "N2",
        vec![cat(
            "N3",
            vec![
                cat("N4a", vec![cat("N5", vec![e("a", "g0")])]),
                cat("N4b", vec![e("a", "g1")]),
            ],
        )],
    )]);
    let report = validate(&wide, &feeds(2));
    assert_eq!(codes(&report), vec![ProblemCode::TooDeep; 2]);
    let paths: Vec<_> = report.problems.iter().map(|p| p.path.clone()).collect();
    assert_eq!(paths, vec![Some(vec![0, 0, 0]), Some(vec![0, 0, 1])]);
}

#[test]
fn estimation_d5_taille_annoncee_prioritaire() {
    // Taille annoncée présente : utilisée telle quelle, même au-dessus de durée × 192 kbps.
    let announced = ep("x", Some(300_000_000), Some(3600));
    assert_eq!(estimate_bytes(&announced), Some(300_000_000));
    assert_eq!(
        estimate_bytes(&ep("y", None, Some(3600))),
        Some(3600 * FALLBACK_BYTES_PER_SECOND)
    );
    assert_eq!(estimate_bytes(&ep("z", Some(0), Some(10))), Some(240_000));
    assert_eq!(estimate_bytes(&ep("w", None, None)), None);
}

#[test]
fn description_saisie_bornee_avec_reserve() {
    let mut c = comp(vec![e("a", "g0")]);
    c.metadata.description = Some("d".repeat(MAX_DESCRIPTION_INPUT_BYTES));
    assert!(validate(&c, &feeds(1)).valid);
    c.metadata.description = Some("d".repeat(MAX_DESCRIPTION_INPUT_BYTES + 1));
    let report = validate(&c, &feeds(1));
    assert_eq!(codes(&report), vec![ProblemCode::TextTooLong]);
    assert_eq!(report.problems[0].field, Some("description"));
    // La réserve couvre 50 échecs aux titres longs de caractères de 4 octets.
    let failures: Vec<String> = (0..50).map(|_| "😀".repeat(300)).collect();
    let full =
        crate::podcast::archive::with_failures(&"d".repeat(MAX_DESCRIPTION_INPUT_BYTES), &failures);
    assert!(full.len() <= MAX_DESCRIPTION_BYTES, "{}", full.len());
}

#[test]
fn liste_de_problemes_plafonnee() {
    let menu: Vec<PackNode> = (0..MAX_PROBLEMS + 10)
        .map(|i| cat(&format!("c{i}"), vec![]))
        .collect();
    let report = validate(&comp(menu), &feeds(1));
    assert!(!report.valid);
    assert_eq!(report.problems.len(), MAX_PROBLEMS);
}

#[test]
fn cinquante_options_acceptees_cinquante_et_une_refusees() {
    let g = guids(50);
    let menu: Vec<PackNode> = g.iter().map(|g| e("a", g)).collect();
    assert!(validate(&comp(menu.clone()), &feeds(50)).valid);
    let mut wide = menu;
    wide.push(cat("Encore", vec![e("b", "h0")]));
    let mut c = comp(wide);
    c.metadata.title = "Mélange".into();
    let report = validate(&c, &feeds(50));
    assert!(codes(&report).contains(&ProblemCode::TooManyOptions));
    assert!(codes(&report).contains(&ProblemCode::TooManyEpisodes));
}

#[test]
fn cinquante_et_un_episodes_refuses_meme_repartis() {
    let g = guids(51);
    let menu = vec![
        cat("Un", g[..26].iter().map(|g| e("a", g)).collect()),
        cat("Deux", g[26..].iter().map(|g| e("a", g)).collect()),
    ];
    let report = validate(&comp(menu), &feeds(51));
    assert_eq!(codes(&report), vec![ProblemCode::TooManyEpisodes]);
}

#[test]
fn categorie_vide_et_pack_vide_refuses() {
    let report = validate(&comp(vec![e("a", "g0"), cat("Vide", vec![])]), &feeds(1));
    assert_eq!(codes(&report), vec![ProblemCode::EmptyCategory]);
    assert_eq!(report.problems[0].path, Some(vec![1]));
    // Catégorie ne contenant qu'une catégorie vide : signalée par la feuille.
    let nested = comp(vec![e("a", "g0"), cat("A", vec![cat("B", vec![])])]);
    assert_eq!(
        codes(&validate(&nested, &feeds(1))),
        vec![ProblemCode::EmptyCategory]
    );
    let empty = validate(&comp(vec![]), &feeds(1));
    assert!(codes(&empty).contains(&ProblemCode::NoEpisode));
}

#[test]
fn noms_de_categorie_bornes() {
    let long = "x".repeat(MAX_LABEL_CHARS);
    assert!(validate(&comp(vec![cat(&long, vec![e("a", "g0")])]), &feeds(1)).valid);
    for bad in ["  ", &"é".repeat(MAX_LABEL_CHARS + 1), "a\u{7}b"] {
        let report = validate(&comp(vec![cat(bad, vec![e("a", "g0")])]), &feeds(1));
        assert_eq!(
            codes(&report),
            vec![ProblemCode::InvalidCategoryName],
            "{bad:?}"
        );
    }
}

#[test]
fn episode_en_double_refuse_meme_dans_une_autre_categorie() {
    let c = comp(vec![
        e("a", "g0"),
        cat("Bis", vec![e("a", "g0"), e("a", "g1")]),
    ]);
    let report = validate(&c, &feeds(2));
    assert_eq!(codes(&report), vec![ProblemCode::DuplicateEpisode]);
    assert_eq!(report.problems[0].path, Some(vec![1, 0]));
    assert_eq!(
        report.estimated_bytes, 20_000_000,
        "doublon compté une fois"
    );
}

#[test]
fn identifiants_invalides_refuses() {
    let bad = PackNode::Episode(EpisodeRef {
        source_id: "a".into(),
        episode_id: "x".repeat(MAX_ID_LEN + 1),
    });
    let empty = PackNode::Episode(EpisodeRef {
        source_id: String::new(),
        episode_id: episode_id("g0"),
    });
    let report = validate(&comp(vec![bad, empty, e("a", "g0")]), &feeds(1));
    assert_eq!(
        codes(&report),
        vec![ProblemCode::InvalidId, ProblemCode::InvalidId]
    );
}

#[test]
fn plafonds_de_taille_d1() {
    let mut f = feeds(0);
    let mut big = feed("Long", &[]);
    big.episodes = vec![
        ep("ok", Some(MAX_EPISODE_BYTES), None),
        ep("trop", Some(MAX_EPISODE_BYTES + 1), None),
        // Sans taille : durée × 192 kbps (3 h = 259,2 Mo).
        ep("duree", None, Some(3 * 3600)),
        ep("inconnu", None, None),
    ];
    f.insert("l".into(), CachedFeed::Ready(big));
    let c = comp(vec![
        e("l", "ok"),
        e("l", "trop"),
        e("l", "duree"),
        e("l", "inconnu"),
    ]);
    let report = validate(&c, &f);
    assert_eq!(
        codes(&report),
        vec![ProblemCode::EpisodeTooLarge, ProblemCode::EpisodeTooLarge]
    );
    assert_eq!(report.unknown_size, 1);
    assert_no_secret(&report);

    // Total : 6 × 199 Mo > 1 Gio, chaque épisode sous le plafond.
    let mut total = feed("Total", &[]);
    total.episodes = (0..6)
        .map(|i| ep(&format!("t{i}"), Some(199_000_000), None))
        .collect();
    f.insert("t".into(), CachedFeed::Ready(total));
    let c = comp((0..6).map(|i| e("t", &format!("t{i}"))).collect());
    let report = validate(&c, &f);
    assert_eq!(codes(&report), vec![ProblemCode::PackTooLarge]);
    assert_eq!(report.estimated_bytes, 1_194_000_000);
}

#[test]
fn titre_obligatoire_si_plusieurs_flux_n1() {
    let mut c = comp(vec![e("a", "g0"), e("b", "h0")]);
    let report = validate(&c, &feeds(1));
    assert_eq!(codes(&report), vec![ProblemCode::TitleRequired]);
    assert_eq!(report.metadata, None);
    assert_eq!(report.feeds, 2);

    c.metadata.title = "  Ma sélection ".into();
    let report = validate(&c, &feeds(1));
    assert!(report.valid);
    let meta = report.metadata.unwrap();
    assert_eq!(meta.title, "Ma sélection");
    assert_eq!(meta.subtitle, "", "sous-titre vide en multi-flux");
}

#[test]
fn textes_trop_longs_refuses() {
    let mut c = comp(vec![e("a", "g0")]);
    c.metadata.title = "t".repeat(MAX_TITLE_CHARS + 1);
    c.metadata.subtitle = Some("s".repeat(MAX_TITLE_CHARS + 1));
    c.metadata.description = Some("d".repeat(MAX_DESCRIPTION_BYTES + 1));
    let report = validate(&c, &feeds(1));
    assert_eq!(codes(&report), vec![ProblemCode::TextTooLong; 3]);
}

#[test]
fn episodes_indisponibles_signales_sans_perte_du_reste() {
    let mut f = feeds(1);
    f.insert("jamais".into(), CachedFeed::Never);
    f.insert("abime".into(), CachedFeed::Unreadable);
    let mut c = comp(vec![
        e("a", "g0"),
        e("a", "retire"),
        e("supprimee", "x"),
        e("jamais", "y"),
        e("abime", "z"),
    ]);
    c.metadata.title = "Pack".into();
    let report = validate(&c, &f);
    assert_eq!(
        report.unavailable,
        vec![
            r("a", "retire"),
            r("supprimee", "x"),
            r("jamais", "y"),
            r("abime", "z")
        ]
    );
    assert_eq!(codes(&report), vec![ProblemCode::EpisodeUnavailable; 4]);
    let messages: Vec<&str> = report.problems.iter().map(|p| p.message.as_str()).collect();
    assert!(messages[0].contains("retiré du flux"));
    assert!(messages[1].contains("source supprimée"));
    assert!(messages[2].contains("jamais obtenu"));
    assert!(messages[3].contains("cache illisible"));
    assert_eq!(report.estimated_bytes, 10_000_000, "le reste est compté");
}

#[test]
fn titre_par_defaut_absent_si_flux_unique_indisponible() {
    let report = validate(&comp(vec![e("absent", "g0")]), &feeds(1));
    assert!(codes(&report).contains(&ProblemCode::TitleRequired));
}

#[test]
fn couverture_n6() {
    // Défaut : premier flux utilisé (ordre de l'arborescence) ayant une image.
    let mut f = feeds(1);
    if let Some(CachedFeed::Ready(a)) = f.get_mut("a") {
        a.image = None;
    }
    let mut c = comp(vec![e("a", "g0"), e("b", "h0")]);
    c.metadata.title = "Pack".into();
    let report = validate(&c, &f);
    assert_eq!(
        report.cover,
        Some(CoverChoice::Feed {
            source_id: "b".into()
        })
    );

    c.cover = CoverChoice::Episode(r("a", "g0"));
    let report = validate(&c, &f);
    assert!(report.valid);
    assert_eq!(report.cover, Some(CoverChoice::Episode(r("a", "g0"))));

    for bad in [
        CoverChoice::Feed {
            source_id: "a".into(), // flux sans image
        },
        CoverChoice::Feed {
            source_id: "z".into(), // hors du pack
        },
        CoverChoice::Episode(r("b", "h1")), // épisode hors du pack
    ] {
        c.cover = bad.clone();
        let report = validate(&c, &f);
        assert_eq!(
            codes(&report),
            vec![ProblemCode::CoverUnavailable],
            "{bad:?}"
        );
        assert_eq!(report.cover, None);
    }
}

#[test]
fn format_json_du_modele_stable() {
    let c = comp(vec![e("a", "g0"), cat("Soir", vec![])]);
    let json = serde_json::to_value(&c).unwrap();
    assert_eq!(json["menu"][0]["kind"], "episode");
    assert_eq!(json["menu"][0]["source_id"], "a");
    assert_eq!(json["menu"][1]["kind"], "category");
    assert_eq!(json["cover"]["kind"], "default");
    let back: PackComposition = serde_json::from_value(json).unwrap();
    assert_eq!(back, c);
    // Champs absents tolérés (front plus ancien).
    let minimal: PackComposition = serde_json::from_str(r#"{"menu":[]}"#).unwrap();
    assert_eq!(minimal, PackComposition::default());
}

// ----- Brouillon -----

fn sample() -> PackComposition {
    let mut c = comp(vec![
        e("a", "g0"),
        cat("Soir", vec![e("a", "g1"), cat("Dodo", vec![e("b", "h0")])]),
    ]);
    c.metadata = PackMetadataInput {
        title: "Mon pack é ✓".into(),
        subtitle: Some(String::new()),
        age: 4,
        description: Some("ligne 1\nligne 2".into()),
    };
    c.cover = CoverChoice::Episode(r("b", "h0"));
    c
}

fn backups(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(DRAFT_FILE) && n.ends_with(".bak"))
        .collect()
}

#[test]
fn brouillon_absent_vide_sans_message() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(read_draft(dir.path()).unwrap(), DraftRead::default());
}

#[test]
fn brouillon_aller_retour_sans_perte() {
    let dir = tempfile::tempdir().unwrap();
    let c = sample();
    assert!(save_draft(dir.path(), &c, 1).unwrap().saved);
    let read = read_draft(dir.path()).unwrap();
    assert_eq!(read.composition, Some(c));
    assert_eq!(read.revision, 1);
    assert_eq!(read.notice, None);
    let raw: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(DRAFT_FILE)).unwrap()).unwrap();
    assert_eq!(raw["schema_version"], DRAFT_SCHEMA);
}

#[test]
fn sauvegarde_en_retard_ignoree() {
    let dir = tempfile::tempdir().unwrap();
    let mut newer = sample();
    newer.metadata.title = "récent".into();
    assert!(save_draft(dir.path(), &newer, 5).unwrap().saved);
    assert_eq!(
        save_draft(dir.path(), &sample(), 4).unwrap(),
        SaveOutcome {
            saved: false,
            revision: 5
        }
    );
    assert!(!save_draft(dir.path(), &sample(), 5).unwrap().saved);
    assert_eq!(read_draft(dir.path()).unwrap().composition, Some(newer));
}

#[test]
fn brouillon_efface_apres_creation_et_non_ressuscite() {
    let dir = tempfile::tempdir().unwrap();
    assert!(save_draft(dir.path(), &sample(), 3).unwrap().saved);
    // Le front a émis la révision 4, encore en vol au moment de la création.
    clear_draft(dir.path(), 4).unwrap();
    let read = read_draft(dir.path()).unwrap();
    assert_eq!(read.composition, None);
    assert_eq!(read.revision, 4);
    assert!(
        !save_draft(dir.path(), &sample(), 4).unwrap().saved,
        "autosave en retard"
    );
    assert_eq!(read_draft(dir.path()).unwrap().composition, None);
    // Nouvelle composition : révisions suivantes acceptées.
    assert!(save_draft(dir.path(), &sample(), 5).unwrap().saved);
    assert!(read_draft(dir.path()).unwrap().composition.is_some());
    // Effacement sans brouillon : sans erreur.
    let empty = tempfile::tempdir().unwrap();
    clear_draft(empty.path(), 0).unwrap();
    assert_eq!(read_draft(empty.path()).unwrap().composition, None);
}

#[test]
fn brouillon_tronque_mis_de_cote_avec_message() {
    let dir = tempfile::tempdir().unwrap();
    save_draft(dir.path(), &sample(), 1).unwrap();
    let path = dir.path().join(DRAFT_FILE);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, &text[..text.len() / 2]).unwrap();
    let read = read_draft(dir.path()).unwrap();
    assert_eq!(read.composition, None);
    assert!(read.notice.unwrap().contains("illisible"));
    assert!(!path.exists());
    assert_eq!(backups(dir.path()).len(), 1, "contenu conservé en .bak");
    // La sauvegarde suivante repart proprement.
    assert!(save_draft(dir.path(), &sample(), 1).unwrap().saved);
    assert!(read_draft(dir.path()).unwrap().composition.is_some());
}

#[test]
fn brouillon_corrompu_sous_toutes_ses_formes() {
    for content in [
        &b"\xff\xfe\x00 pas de l'UTF-8"[..],
        b"[1, 2, 3]",
        b"{\"composition\": {\"menu\": [{\"kind\": \"inconnu\"}]}}",
        b"{\"schema_version\": \"un\"}",
        b"",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(DRAFT_FILE), content).unwrap();
        let read = read_draft(dir.path()).unwrap();
        assert_eq!(read.composition, None);
        assert!(read.notice.is_some(), "{content:?}");
        assert_eq!(backups(dir.path()).len(), 1);
    }
}

#[test]
fn brouillon_trop_gros_mis_de_cote_sans_lecture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DRAFT_FILE);
    fs::write(&path, vec![b' '; MAX_DRAFT_BYTES as usize + 1]).unwrap();
    let read = read_draft(dir.path()).unwrap();
    assert!(read.notice.unwrap().contains("trop volumineux"));
    assert_eq!(backups(dir.path()).len(), 1);
}

#[test]
fn brouillon_trop_gros_refuse_a_l_ecriture_ancien_intact() {
    let dir = tempfile::tempdir().unwrap();
    save_draft(dir.path(), &sample(), 1).unwrap();
    let g = guids(20_000);
    let huge = comp(g.iter().map(|g| e("a", g)).collect());
    let err = save_draft(dir.path(), &huge, 2).unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    let read = read_draft(dir.path()).unwrap();
    assert_eq!((read.composition, read.revision), (Some(sample()), 1));
}

#[test]
fn ancien_schema_migre() {
    let dir = tempfile::tempdir().unwrap();
    // Version 0 : sans `schema_version`.
    let old = serde_json::json!({
        "revision": 7,
        "composition": {
            "metadata": { "title": "Ancien", "age": 3 },
            "menu": [{ "kind": "episode", "source_id": "a", "episode_id": "abc" }]
        }
    });
    fs::write(dir.path().join(DRAFT_FILE), old.to_string()).unwrap();
    let read = read_draft(dir.path()).unwrap();
    assert_eq!(read.revision, 7);
    assert_eq!(read.notice, None);
    let c = read.composition.unwrap();
    assert_eq!(c.metadata.title, "Ancien");
    assert_eq!(c.cover, CoverChoice::Default);
    assert_eq!(
        c.menu,
        vec![PackNode::Episode(EpisodeRef {
            source_id: "a".into(),
            episode_id: "abc".into()
        })]
    );
}

#[test]
fn schema_plus_recent_mis_de_cote_jamais_ecrase() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(DRAFT_FILE);
    let future = format!(
        r#"{{"schema_version": {}, "revision": 9}}"#,
        DRAFT_SCHEMA + 1
    );
    fs::write(&path, &future).unwrap();
    let read = read_draft(dir.path()).unwrap();
    assert!(read.notice.unwrap().contains("plus récente"));
    assert_eq!(read.composition, None);
    let saved = backups(dir.path());
    assert_eq!(saved.len(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join(&saved[0])).unwrap(),
        future
    );
}

#[test]
fn ecriture_interrompue_ancien_brouillon_intact() {
    let dir = tempfile::tempdir().unwrap();
    save_draft(dir.path(), &sample(), 1).unwrap();
    // Arrêt brutal pendant une écriture : temporaire voisin abandonné, destination intacte.
    fs::write(
        dir.path().join(format!(".{DRAFT_FILE}.1234-0.tmp")),
        b"{\"revision\": 99, \"compo",
    )
    .unwrap();
    let read = read_draft(dir.path()).unwrap();
    assert_eq!((read.composition, read.revision), (Some(sample()), 1));
    assert!(save_draft(dir.path(), &sample(), 2).unwrap().saved);
}

/// Renommage final impossible (destination en lecture seule, refusée par Windows) : erreur,
/// ancien brouillon intact, aucun temporaire laissé.
#[cfg(windows)]
#[test]
fn ecriture_impossible_erreur_ancien_intact() {
    let dir = tempfile::tempdir().unwrap();
    save_draft(dir.path(), &sample(), 1).unwrap();
    let path = dir.path().join(DRAFT_FILE);
    let before = fs::read(&path).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_readonly(true);
    fs::set_permissions(&path, perms.clone()).unwrap();
    let mut other = sample();
    other.metadata.title = "autre".into();
    let result = save_draft(dir.path(), &other, 2);
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    fs::set_permissions(&path, perms).unwrap();
    assert!(result.is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    let leftovers: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty());
}

#[test]
fn sauvegarde_sur_fichier_corrompu_non_lu_le_met_de_cote() {
    for content in [
        &b"{\"revision\": 3, tronqu"[..],
        b"{\"schema_version\": 99, \"revision\": 3}",
        // En-tête valide, composition invalide.
        b"{\"schema_version\": 1, \"revision\": 3, \"composition\": {\"menu\": [{\"kind\": \"x\"}]}}",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(DRAFT_FILE);
        fs::write(&path, content).unwrap();
        assert!(save_draft(dir.path(), &sample(), 1).unwrap().saved);
        assert_eq!(backups(dir.path()).len(), 1, "{content:?}");
        let bak = dir.path().join(&backups(dir.path())[0]);
        assert_eq!(fs::read(bak).unwrap(), content);
        // Même chose pour l'effacement.
        fs::write(&path, content).unwrap();
        clear_draft(dir.path(), 2).unwrap();
        assert_eq!(backups(dir.path()).len(), 2);
        assert_eq!(read_draft(dir.path()).unwrap().revision, 2);
    }
}

#[test]
fn revision_bornee_aux_entiers_exacts_javascript() {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        save_draft(dir.path(), &sample(), MAX_REVISION)
            .unwrap()
            .saved
    );
    for bad in [MAX_REVISION + 1, u64::MAX] {
        let err = save_draft(dir.path(), &sample(), bad).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
        let err = clear_draft(dir.path(), bad).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)));
    }
    assert_eq!(read_draft(dir.path()).unwrap().revision, MAX_REVISION);
}

#[test]
fn flux_illisibles_le_brouillon_reste_transmis() {
    let draft = DraftRead {
        composition: Some(sample()),
        revision: 7,
        notice: None,
    };
    let view = draft_view(draft.clone(), Some(Err(AppError::ConfigDir)));
    assert_eq!(view.draft.composition, draft.composition);
    assert_eq!(view.draft.revision, 7);
    assert!(view.draft.notice.unwrap().contains("pas pu être vérifiés"));
    assert_eq!(view.report, None);
    let view = draft_view(draft, Some(Ok(feeds(2))));
    assert!(view.report.is_some());
    assert_eq!(view.draft.notice, None);
}

#[test]
fn problemes_de_metadonnees_designent_leur_champ() {
    let mut c = comp(vec![e("a", "g0"), e("b", "h0")]);
    c.metadata.subtitle = Some("s".repeat(MAX_TITLE_CHARS + 1));
    c.metadata.description = Some("d".repeat(MAX_DESCRIPTION_BYTES + 1));
    c.cover = CoverChoice::Feed {
        source_id: "z".into(),
    };
    let report = validate(&c, &feeds(1));
    let fields: Vec<_> = report.problems.iter().map(|p| p.field).collect();
    assert_eq!(
        fields,
        vec![
            Some("title"),
            Some("subtitle"),
            Some("description"),
            Some("cover")
        ]
    );
}
