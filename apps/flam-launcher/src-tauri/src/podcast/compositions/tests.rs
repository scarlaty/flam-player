use super::*;
use crate::podcast::pack::{CoverChoice, EpisodeRef, PackMetadataInput, PackNode};

fn ep(source: &str, id: &str) -> PackNode {
    PackNode::Episode(EpisodeRef {
        source_id: source.into(),
        episode_id: id.into(),
    })
}

fn comp(title: &str, ids: &[&str]) -> PackComposition {
    PackComposition {
        metadata: PackMetadataInput {
            title: title.into(),
            subtitle: None,
            age: 3,
            description: None,
        },
        cover: CoverChoice::Default,
        menu: vec![
            ep("a", ids[0]),
            PackNode::Category {
                name: "Soir".into(),
                entries: ids[1..].iter().map(|id| ep("b", id)).collect(),
            },
        ],
        ..PackComposition::default()
    }
}

fn backups(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.starts_with(COMPOSITIONS_FILE) && n.ends_with(".bak"))
        .collect();
    names.sort();
    names
}

fn leftovers(dir: &Path) -> usize {
    fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
        .count()
}

fn names(dir: &Path) -> Vec<String> {
    Store::new(dir)
        .read()
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect()
}

/// Disque plein simulé : un temporaire partiel est écrit puis supprimé, l'erreur remonte
/// (comportement de `write_atomic` quand `write_all` échoue).
fn disk_full(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes[..bytes.len() / 2])?;
    fs::remove_file(&tmp)?;
    Err(io::Error::from_raw_os_error(112))
}

#[test]
fn fichier_absent_liste_vide_sans_ecriture() {
    let dir = tempfile::tempdir().unwrap();
    let read = Store::new(dir.path()).read().unwrap();
    assert!(read.entries.is_empty());
    assert_eq!(read.notice, None);
    assert!(!dir.path().join(COMPOSITIONS_FILE).exists());
}

#[test]
fn conservation_puis_relecture_sans_perte() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let c = comp("Contes", &["e1", "e2", "e3"]);
    let r = store.record(&c, "Contes", 100).unwrap();
    assert!(r.new);
    assert_eq!(r.name, "Contes");
    assert_eq!(r.notice, None);
    let read = store.read().unwrap();
    assert_eq!(read.entries.len(), 1);
    let e = &read.entries[0];
    assert_eq!(
        (e.id.as_str(), e.created_at, e.last_used_at, e.packs),
        (r.id.as_str(), 100, 100, 1)
    );
    assert_eq!(e.composition, c);
    let raw: Value =
        serde_json::from_str(&fs::read_to_string(dir.path().join(COMPOSITIONS_FILE)).unwrap())
            .unwrap();
    assert_eq!(raw["schema_version"], COMPOSITIONS_SCHEMA);
    assert_eq!(leftovers(dir.path()), 0);
}

#[test]
fn refaire_met_a_jour_l_entree_deriver_en_cree_une() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let c = comp("Contes", &["e1", "e2"]);
    let first = store.record(&c, "Contes", 100).unwrap();
    // Refaire : composition identique.
    let again = store.record(&c, "Contes", 200).unwrap();
    assert!(!again.new);
    assert_eq!(again.id, first.id);
    let read = store.read().unwrap().entries;
    assert_eq!(read.len(), 1);
    assert_eq!(
        (read[0].created_at, read[0].last_used_at, read[0].packs),
        (100, 200, 2)
    );
    // Dériver : un épisode de plus, même titre.
    let derived = comp("Contes", &["e1", "e2", "e3"]);
    let d = store.record(&derived, "Contes", 300).unwrap();
    assert!(d.new);
    assert_ne!(d.id, first.id);
    assert_eq!(d.name, "Contes (2)");
    let read = store.read().unwrap().entries;
    assert_eq!(read.len(), 2);
    assert_eq!(read[0].composition, c, "l'originale reste intacte");
}

#[test]
fn noms_uniques_bornes_et_nettoyes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let long = "é".repeat(80);
    for (i, title) in [
        long.as_str(),
        long.as_str(),
        "  \u{7}Tab\tulé ",
        "",
        "CONTES",
        "contes",
    ]
    .iter()
    .enumerate()
    {
        store
            .record(&comp(title, &[&format!("x{i}"), "y"]), title, 1)
            .unwrap();
    }
    let n = names(dir.path());
    assert_eq!(n[0], "é".repeat(MAX_NAME_CHARS));
    assert_eq!(n[1], format!("{} (2)", "é".repeat(MAX_NAME_CHARS - 4)));
    assert_eq!(n[2], "Tab ulé");
    assert_eq!(n[3], FALLBACK_NAME);
    assert_eq!(n[4], "CONTES");
    assert_eq!(n[5], "contes (2)", "unicité sans tenir compte de la casse");
    assert!(n.iter().all(|s| s.chars().count() <= MAX_NAME_CHARS));
}

#[test]
fn limite_atteinte_refusee_sans_suppression() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    for i in 0..MAX_COMPOSITIONS {
        store
            .record(&comp("P", &[&format!("e{i}"), "z"]), "P", i as u64)
            .unwrap();
    }
    let before = fs::read(dir.path().join(COMPOSITIONS_FILE)).unwrap();
    let err = store
        .record(&comp("P", &["nouveau", "z"]), "P", 99)
        .unwrap_err();
    assert!(matches!(err, AppError::Validation(ref m) if m.contains("supprimez")));
    assert_eq!(
        fs::read(dir.path().join(COMPOSITIONS_FILE)).unwrap(),
        before
    );
    // Refaire une composition existante reste possible à la limite.
    assert!(
        !store
            .record(&comp("P", &["e0", "z"]), "P", 100)
            .unwrap()
            .new
    );
}

#[test]
fn composition_trop_volumineuse_refusee() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = comp("P", &["e", "f"]);
    c.metadata.description = Some("d".repeat(MAX_ENTRY_BYTES));
    let err = Store::new(dir.path()).record(&c, "P", 1).unwrap_err();
    assert!(matches!(err, AppError::Validation(_)));
    assert!(!dir.path().join(COMPOSITIONS_FILE).exists());
}

#[test]
fn renommer() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let a = store.record(&comp("A", &["1", "2"]), "A", 1).unwrap();
    let b = store.record(&comp("B", &["3", "4"]), "B", 2).unwrap();
    store.rename(&a.id, "  Histoires du soir  ").unwrap();
    assert_eq!(names(dir.path()), ["Histoires du soir", "B"]);
    // Changement de casse de son propre nom : accepté.
    store.rename(&a.id, "HISTOIRES DU SOIR").unwrap();
    // Nom pris par une autre (casse ignorée), vide, trop long, avec contrôle : refusés.
    for bad in ["b", "   ", &"x".repeat(MAX_NAME_CHARS + 1), "a\nb"] {
        let err = store.rename(&a.id, bad).unwrap_err();
        assert!(matches!(err, AppError::Validation(_)), "{bad:?}");
    }
    assert_eq!(names(dir.path()), ["HISTOIRES DU SOIR", "B"]);
    let err = store.rename("inconnue", "Z").unwrap_err();
    assert!(matches!(err, AppError::NotFound(_)));
    // Renommer ne change pas la composition.
    store.rename(&b.id, "C").unwrap();
    assert_eq!(
        store.read().unwrap().entries[1].composition,
        comp("B", &["3", "4"])
    );
}

#[test]
fn supprimer_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let a = store.record(&comp("A", &["1", "2"]), "A", 1).unwrap();
    store.record(&comp("B", &["3", "4"]), "B", 2).unwrap();
    store.delete(&a.id).unwrap();
    assert_eq!(names(dir.path()), ["B"]);
    store.delete(&a.id).unwrap();
    store.delete("inconnue").unwrap();
    assert_eq!(names(dir.path()), ["B"]);
}

#[test]
fn ouvrir_copie_dans_le_brouillon_a_la_revision_suivante() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let c = comp("A", &["1", "2"]);
    let a = store.record(&c, "A", 1).unwrap();
    // Brouillon en cours, révision 7.
    pack::save_draft(dir.path(), &comp("Autre", &["9", "8"]), 7).unwrap();
    let outcome = store.open(&a.id).unwrap();
    assert_eq!(
        outcome,
        Opened {
            saved: true,
            revision: 8,
            notice: None
        }
    );
    let draft = pack::read_draft(dir.path()).unwrap();
    assert_eq!((draft.composition, draft.revision), (Some(c.clone()), 8));
    // Une sauvegarde automatique en retard (révision 8) ne l'écrase pas.
    assert!(
        !pack::save_draft(dir.path(), &comp("Autre", &["9", "8"]), 8)
            .unwrap()
            .saved
    );
    // Sans brouillon : révision 1.
    let empty = tempfile::tempdir().unwrap();
    let s2 = Store::new(empty.path());
    let id = s2.record(&c, "A", 1).unwrap().id;
    assert_eq!(s2.open(&id).unwrap().revision, 1);
    // Composition disparue.
    assert!(matches!(store.open("inconnue"), Err(AppError::NotFound(_))));
    // Révision épuisée : refus, brouillon intact.
    pack::save_draft(dir.path(), &c, MAX_REVISION).unwrap();
    assert!(matches!(store.open(&a.id), Err(AppError::Validation(_))));
}

#[test]
fn ouvrir_sur_brouillon_illisible_le_met_de_cote() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let c = comp("A", &["1", "2"]);
    let a = store.record(&c, "A", 1).unwrap();
    fs::write(dir.path().join(pack::DRAFT_FILE), b"{\"revision\": 3, tron").unwrap();
    let opened = store.open(&a.id).unwrap();
    assert!(opened.saved);
    // Le message de mise de côté du brouillon est rendu (#484, revue).
    assert!(opened.notice.unwrap().contains("brouillon"));
    assert_eq!(pack::read_draft(dir.path()).unwrap().composition, Some(c));
    let draft_backups = fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            n.starts_with(pack::DRAFT_FILE) && n.ends_with(".bak")
        })
        .count();
    assert_eq!(draft_backups, 1);
}

#[test]
fn source_supprimee_marquee_dans_la_liste() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.record(&comp("A", &["1", "2"]), "A", 1).unwrap();
    let mut c = comp("B", &["3", "4"]);
    c.cover = CoverChoice::Feed {
        source_id: "b".into(),
    };
    store.record(&c, "B", 5).unwrap();
    let podcasts: HashSet<String> = ["a".to_string()].into();
    let v = view(store.read().unwrap(), Some(&podcasts));
    assert_eq!(v.max, MAX_COMPOSITIONS);
    // Dernière utilisée d'abord.
    let got: Vec<_> = v
        .compositions
        .iter()
        .map(|s| (s.name.as_str(), s.episodes, s.feeds, s.missing_feeds))
        .collect();
    assert_eq!(got, [("B", 2, 2, 1), ("A", 2, 2, 1)]);
    let all: HashSet<String> = ["a".to_string(), "b".to_string()].into();
    assert!(view(store.read().unwrap(), Some(&all))
        .compositions
        .iter()
        .all(|s| s.missing_feeds == 0));
}

// ----- Injection de panne -----

#[test]
fn fichier_tronque_mis_de_cote_puis_nouveau_depart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.record(&comp("A", &["1", "2"]), "A", 1).unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, &text[..text.len() / 2]).unwrap();
    let read = store.read().unwrap();
    assert!(read.entries.is_empty());
    assert!(read.notice.unwrap().contains("illisible"));
    let saved = backups(dir.path());
    assert_eq!(saved.len(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join(&saved[0])).unwrap(),
        text[..text.len() / 2]
    );
    // La conservation suivante repart d'un fichier neuf.
    let r = store.record(&comp("B", &["3", "4"]), "B", 2).unwrap();
    assert_eq!(r.notice, None);
    assert_eq!(names(dir.path()), ["B"]);
}

#[test]
fn fichier_corrompu_sous_toutes_ses_formes() {
    for content in [
        &b"\xff\xfe\x00 pas de l'UTF-8"[..],
        b"[1, 2, 3]",
        b"{\"compositions\": 3}",
        b"{\"schema_version\": \"un\"}",
        b"",
    ] {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(COMPOSITIONS_FILE), content).unwrap();
        // Rencontré pendant une conservation : la composition est conservée, le message suit.
        let r = Store::new(dir.path())
            .record(&comp("A", &["1", "2"]), "A", 1)
            .unwrap();
        assert!(r.notice.is_some(), "{content:?}");
        assert_eq!(backups(dir.path()).len(), 1, "{content:?}");
        assert_eq!(
            fs::read(dir.path().join(&backups(dir.path())[0])).unwrap(),
            content
        );
        assert_eq!(names(dir.path()), ["A"]);
    }
}

#[test]
fn entrees_illisibles_ecartees_apres_copie_integrale() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let good = store.record(&comp("Bon", &["1", "2"]), "Bon", 1).unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let mut raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    let valid = raw["compositions"][0].clone();
    let list = raw["compositions"].as_array_mut().unwrap();
    // Type d'option inconnu, champ manquant, identifiant en double, nom vide.
    let mut unknown = valid.clone();
    unknown["id"] = "x1".into();
    unknown["name"] = "X".into();
    unknown["composition"]["menu"] = serde_json::json!([{ "kind": "inconnu" }]);
    list.push(unknown);
    list.push(serde_json::json!({ "id": "x2", "name": "Y" }));
    let mut dup = valid.clone();
    dup["name"] = "Autre".into();
    list.push(dup);
    let mut empty = valid.clone();
    empty["id"] = "x3".into();
    empty["name"] = " ".into();
    list.push(empty);
    let original = serde_json::to_string(&raw).unwrap();
    fs::write(&path, &original).unwrap();

    let read = store.read().unwrap();
    assert_eq!(read.entries.len(), 1);
    assert_eq!(read.entries[0].id, good.id);
    let notice = read.notice.unwrap();
    assert!(notice.contains("4 composition(s)"), "{notice}");
    // Raisons distinguées (#484) : une entrée en double n'est pas dite illisible.
    assert!(notice.contains("3 illisible(s)"), "{notice}");
    assert!(notice.contains("1 en double"), "{notice}");
    let saved = backups(dir.path());
    assert_eq!(saved.len(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join(&saved[0])).unwrap(),
        original
    );
    // Fichier réécrit : le message ne revient pas.
    let again = store.read().unwrap();
    assert_eq!((again.entries.len(), again.notice), (1, None));
}

#[test]
fn schema_plus_recent_erreur_fichier_intact() {
    // Comme `settings.json` (#484) : ni mis de côté ni réécrit, la liste ne repart pas de zéro.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let future = format!(
        r#"{{"schema_version": {}, "compositions": [{{"id": "f1", "name": "Futur"}}]}}"#,
        COMPOSITIONS_SCHEMA + 1
    );
    fs::write(&path, &future).unwrap();
    let store = Store::new(dir.path());
    let err = store.read().unwrap_err();
    assert!(matches!(err, AppError::ConfigVersion { .. }));
    // Aucun chemin dans le message (le nom du fichier seulement).
    let message = err.to_string();
    assert!(message.contains(COMPOSITIONS_FILE));
    assert!(!message.contains(&dir.path().display().to_string()));
    assert!(matches!(
        store.record(&comp("A", &["1", "2"]), "A", 1),
        Err(AppError::ConfigVersion { .. })
    ));
    assert!(matches!(
        store.rename("f1", "Z"),
        Err(AppError::ConfigVersion { .. })
    ));
    assert!(matches!(
        store.delete("f1"),
        Err(AppError::ConfigVersion { .. })
    ));
    assert!(matches!(
        store.open("f1"),
        Err(AppError::ConfigVersion { .. })
    ));
    assert!(matches!(
        list_view(dir.path()),
        Err(AppError::ConfigVersion { .. })
    ));
    // La création du pack continue, la raison est dite.
    let r = record_after_create(dir.path(), &comp("A", &["1", "2"]), "A");
    assert_eq!(r.name, None);
    assert!(r.error.unwrap().contains("plus récente"));
    assert_eq!(fs::read_to_string(&path).unwrap(), future);
    assert!(backups(dir.path()).is_empty());
    assert_eq!(leftovers(dir.path()), 0);
}

#[test]
fn fichier_trop_gros_mis_de_cote_sans_lecture() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    fs::write(&path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
    let read = Store::new(dir.path()).read().unwrap();
    assert!(read.notice.unwrap().contains("trop volumineux"));
    assert_eq!(backups(dir.path()).len(), 1);
    assert!(!path.exists());
}

#[test]
fn ancien_schema_sans_version_relu() {
    let dir = tempfile::tempdir().unwrap();
    let old = serde_json::json!({
        "compositions": [{
            "id": "abc-1",
            "name": "Ancien",
            "composition": { "menu": [{ "kind": "episode", "source_id": "a", "episode_id": "e" }] }
        }]
    });
    fs::write(dir.path().join(COMPOSITIONS_FILE), old.to_string()).unwrap();
    let read = Store::new(dir.path()).read().unwrap();
    assert_eq!(read.notice, None);
    let e = &read.entries[0];
    assert_eq!((e.name.as_str(), e.packs, e.created_at), ("Ancien", 0, 0));
    assert_eq!(e.composition.cover, CoverChoice::Default);
}

#[test]
fn disque_plein_ancien_fichier_intact_et_erreur() {
    let dir = tempfile::tempdir().unwrap();
    Store::new(dir.path())
        .record(&comp("A", &["1", "2"]), "A", 1)
        .unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let before = fs::read(&path).unwrap();
    let full = Store::with_writer(dir.path(), disk_full);
    let id = full.read().unwrap().entries[0].id.clone();
    assert!(matches!(
        full.record(&comp("B", &["3", "4"]), "B", 2),
        Err(AppError::Io(_))
    ));
    assert!(matches!(full.rename(&id, "Z"), Err(AppError::Io(_))));
    assert!(matches!(full.delete(&id), Err(AppError::Io(_))));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(leftovers(dir.path()), 0);
    assert!(backups(dir.path()).is_empty());
}

#[test]
fn disque_plein_pendant_la_reecriture_des_entrees_lisibles() {
    let dir = tempfile::tempdir().unwrap();
    Store::new(dir.path())
        .record(&comp("A", &["1", "2"]), "A", 1)
        .unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let mut raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    raw["compositions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "id": "x", "name": "Y" }));
    let original = serde_json::to_string(&raw).unwrap();
    fs::write(&path, &original).unwrap();
    let full = Store::with_writer(dir.path(), disk_full);
    // Lectures répétées tant que le fichier reste impossible à réécrire (#484) : entrées
    // lisibles servies, fichier d'origine intact, et aucune copie accumulée (l'original, intact,
    // en tient lieu).
    for _ in 0..3 {
        let read = full.read().unwrap();
        assert_eq!(read.entries.len(), 1);
        assert!(read.notice.unwrap().contains("n'a pas pu être réécrit"));
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(backups(dir.path()).is_empty());
        assert_eq!(leftovers(dir.path()), 0);
    }
    // Le disque revenu : la réécriture se fait, avec une copie de l'original.
    let read = Store::new(dir.path()).read().unwrap();
    assert!(read.notice.unwrap().contains("copié"));
    let saved = backups(dir.path());
    assert_eq!(saved.len(), 1);
    assert_eq!(
        fs::read_to_string(dir.path().join(&saved[0])).unwrap(),
        original
    );
}

#[test]
fn record_after_create_ne_remonte_jamais_d_erreur() {
    let dir = tempfile::tempdir().unwrap();
    let c = comp("A", &["1", "2"]);
    let r = record_after_create(dir.path(), &c, "A");
    assert_eq!((r.name.as_deref(), r.new, r.error), (Some("A"), true, None));
    let again = record_after_create(dir.path(), &c, "A");
    assert_eq!((again.name.as_deref(), again.new), (Some("A"), false));
    // Fichier impossible à créer (un dossier porte son nom) : erreur rapportée, pas levée.
    let blocked = tempfile::tempdir().unwrap();
    fs::create_dir(blocked.path().join(COMPOSITIONS_FILE)).unwrap();
    let r = record_after_create(blocked.path(), &c, "A");
    assert_eq!(r.name, None);
    assert!(r.error.is_some());
}

#[test]
fn aucune_url_ni_chemin_dans_la_vue() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.record(&comp("A", &["1", "2"]), "A", 1).unwrap();
    let json = serde_json::to_string(&view(store.read().unwrap(), Some(&HashSet::new()))).unwrap();
    assert!(!json.contains("http"));
    assert!(!json.contains(&dir.path().display().to_string()));
}

// ----- Suivis de la revue (#484) -----

/// Fichier avec une entrée lisible (« Bon ») et une entrée illisible : la lecture qui le
/// découvre produit un message (une fois : le fichier est alors réécrit).
fn file_with_rejected_entry(dir: &Path) -> String {
    let good = Store::new(dir)
        .record(&comp("Bon", &["1", "2"]), "Bon", 1)
        .unwrap();
    let path = dir.join(COMPOSITIONS_FILE);
    let mut raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    raw["compositions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "id": "x", "name": "Y" }));
    fs::write(&path, serde_json::to_string(&raw).unwrap()).unwrap();
    good.id
}

#[test]
fn rang_des_copies() {
    let f = COMPOSITIONS_FILE;
    let rank = |name: &str| backup_rank(OsStr::new(name));
    assert_eq!(
        rank(&format!("{f}.20261024-101500.bak")),
        Some(("20261024-101500".into(), 0))
    );
    assert_eq!(
        rank(&format!("{f}.20261024-101500-12.bak")),
        Some(("20261024-101500".into(), 12))
    );
    for other in [
        f.to_string(),
        format!("{f}.tmp"),
        format!("{f}.bak"),
        format!("{f}.2026-bad.bak"),
        format!("{f}.20261024-101500-.bak"),
        format!("{f}.20261024-101500x.bak"),
        format!("{f}.20261024-1015ab.bak"),
        format!("{f}.2026é024-101500.bak"),
        format!("{f}.ééééééé-101500.bak"),
        format!("{}.20261024-101500.bak", pack::DRAFT_FILE),
        format!("{f}.20261024-101500.bak.txt"),
    ] {
        assert_eq!(rank(&other), None, "{other}");
    }
}

#[test]
fn copies_bornees_aux_plus_recentes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    // Copies anciennes et fichiers voisins qui ne sont pas des copies.
    let old = format!("{COMPOSITIONS_FILE}.20200101-000000.bak");
    let older = format!("{COMPOSITIONS_FILE}.20190101-000000-3.bak");
    let foreign = format!("{COMPOSITIONS_FILE}.a-garder.bak");
    let draft_bak = format!("{}.20000101-000000.bak", pack::DRAFT_FILE);
    for name in [&old, &older, &foreign, &draft_bak] {
        fs::write(dir.path().join(name), b"ancien").unwrap();
    }
    // Cinq mises de côté successives (même seconde : suffixes -1, -2…).
    for i in 0..5 {
        fs::write(&path, format!("tronqué {i}")).unwrap();
        let read = Store::new(dir.path()).read().unwrap();
        assert!(read.notice.is_some());
    }
    let saved = backups(dir.path());
    // Les 3 plus récentes, plus la copie au nom inconnu (jamais supprimée).
    assert_eq!(saved.len(), MAX_BACKUPS + 1, "{saved:?}");
    assert!(saved.contains(&foreign));
    assert!(!saved.contains(&old) && !saved.contains(&older));
    let mut kept: Vec<String> = saved
        .iter()
        .filter(|n| **n != foreign)
        .map(|n| fs::read_to_string(dir.path().join(n)).unwrap())
        .collect();
    kept.sort();
    assert_eq!(kept, ["tronqué 2", "tronqué 3", "tronqué 4"]);
    assert!(
        dir.path().join(&draft_bak).exists(),
        "copie du brouillon intacte"
    );
}

#[test]
fn copies_bornees_aussi_pour_les_entrees_ecartees() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..(MAX_BACKUPS + 2) {
        let name = format!("{COMPOSITIONS_FILE}.2020010{i}-000000.bak");
        fs::write(dir.path().join(name), b"ancien").unwrap();
    }
    file_with_rejected_entry(dir.path());
    let read = Store::new(dir.path()).read().unwrap();
    assert!(read.notice.unwrap().contains("copié"));
    let saved = backups(dir.path());
    assert_eq!(saved.len(), MAX_BACKUPS);
    // La copie qui vient d'être faite est gardée (la plus récente).
    assert!(saved.iter().any(|n| fs::read_to_string(dir.path().join(n))
        .unwrap()
        .contains("\"x\"")));
}

#[test]
fn message_transmis_par_record_en_echec() {
    // Limite atteinte et fichier contenant une entrée écartée : erreur ET message (#484).
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    for i in 0..MAX_COMPOSITIONS {
        store
            .record(&comp("P", &[&format!("e{i}"), "z"]), "P", 1)
            .unwrap();
    }
    let path = dir.path().join(COMPOSITIONS_FILE);
    let mut raw: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    raw["compositions"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({ "id": "x", "name": "Y" }));
    fs::write(&path, serde_json::to_string(&raw).unwrap()).unwrap();
    let r = record_after_create(dir.path(), &comp("P", &["nouveau", "z"]), "P");
    assert_eq!(r.name, None);
    assert!(r.error.unwrap().contains("supprimez"));
    assert!(r.notice.unwrap().contains("1 illisible(s)"));

    // Composition trop volumineuse sur un fichier tronqué : idem.
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join(COMPOSITIONS_FILE), b"{\"compositions\": [").unwrap();
    let mut big = comp("G", &["1", "2"]);
    big.metadata.description = Some("d".repeat(MAX_ENTRY_BYTES));
    let r = record_after_create(dir.path(), &big, "G");
    assert!(r.error.unwrap().contains("trop volumineuse"));
    assert!(r.notice.unwrap().contains("mis de côté"));
    // `Store::record` : le message suit l'erreur.
    fs::write(dir.path().join(COMPOSITIONS_FILE), b"{\"compositions\": [").unwrap();
    let err = Store::new(dir.path()).record(&big, "G", 1).unwrap_err();
    assert!(matches!(err, AppError::Validation(ref m) if m.contains("mis de côté")));
}

#[test]
fn message_transmis_par_renommer_supprimer_ouvrir() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let id = file_with_rejected_entry(dir.path());
    let notice = store.rename(&id, "Renommée").unwrap();
    assert!(notice.unwrap().contains("1 illisible(s)"));
    assert_eq!(
        store.rename(&id, "Encore").unwrap(),
        None,
        "message dit une fois"
    );

    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let id = file_with_rejected_entry(dir.path());
    assert!(store.delete(&id).unwrap().unwrap().contains("écartée"));

    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let id = file_with_rejected_entry(dir.path());
    let opened = store.open(&id).unwrap();
    assert_eq!((opened.saved, opened.revision), (true, 1));
    assert!(opened.notice.unwrap().contains("écartée"));

    // En échec (fichier tronqué : la composition a disparu) : message joint à l'erreur, genre
    // inchangé.
    let truncated = || {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(COMPOSITIONS_FILE), b"{\"compositions\": [").unwrap();
        dir
    };
    let dir = truncated();
    let err = Store::new(dir.path()).rename("abc", "Z").unwrap_err();
    assert!(
        matches!(err, AppError::NotFound(ref m) if m.contains("introuvable") && m.contains("mis de côté")),
        "{err}"
    );
    let dir = truncated();
    let err = Store::new(dir.path()).open("abc").unwrap_err();
    assert!(
        matches!(err, AppError::NotFound(ref m) if m.contains("mis de côté")),
        "{err}"
    );
    // Supprimer une absente : rien à faire, mais le message est rendu.
    let dir = truncated();
    assert!(Store::new(dir.path())
        .delete("abc")
        .unwrap()
        .unwrap()
        .contains("mis de côté"));

    // Disque plein pendant le renommage : erreur d'E/S qui porte le message, fichier intact.
    let dir = tempfile::tempdir().unwrap();
    let id = file_with_rejected_entry(dir.path());
    let before = fs::read(dir.path().join(COMPOSITIONS_FILE)).unwrap();
    let err = Store::with_writer(dir.path(), disk_full)
        .rename(&id, "Z")
        .unwrap_err();
    assert!(
        matches!(err, AppError::Io(ref e) if e.to_string().contains("écartée")),
        "{err}"
    );
    assert_eq!(
        fs::read(dir.path().join(COMPOSITIONS_FILE)).unwrap(),
        before
    );
}

#[test]
fn liste_servie_si_les_sources_sont_illisibles() {
    let dir = tempfile::tempdir().unwrap();
    file_with_rejected_entry(dir.path());
    // Un dossier porte le nom du fichier des sources : sa lecture échoue.
    fs::create_dir(dir.path().join("sources.json")).unwrap();
    assert!(SourcesStore::new(dir.path()).list().is_err());
    let v = list_view(dir.path()).unwrap();
    assert_eq!(v.compositions.len(), 1);
    assert_eq!(
        v.compositions[0].missing_feeds, 0,
        "inconnu : rien n'est marqué"
    );
    assert!(v.notice.unwrap().contains("écartée"));
    // Sources lisibles (aucune source podcast) : flux comptés supprimés.
    let ok = tempfile::tempdir().unwrap();
    Store::new(ok.path())
        .record(&comp("A", &["1", "2"]), "A", 1)
        .unwrap();
    assert_eq!(
        list_view(ok.path()).unwrap().compositions[0].missing_feeds,
        2
    );
}

#[test]
fn champs_inconnus_conserves_a_la_reecriture() {
    // Fichier écrit par une version plus récente sans changement de schéma : ses champs
    // ajoutés (premier niveau, entrée, composition) survivent à une réécriture (#484).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let original = serde_json::json!({
        "schema_version": COMPOSITIONS_SCHEMA,
        "reglages_futurs": { "tri": "nom" },
        "compositions": [{
            "id": "abc-1",
            "name": "Futur",
            "created_at": 5,
            "last_used_at": 6,
            "packs": 2,
            "couleur": "bleu",
            "composition": {
                "metadata": { "title": "T", "subtitle": null, "age": 3, "description": null },
                "cover": { "kind": "default" },
                "menu": [{ "kind": "episode", "source_id": "a", "episode_id": "e" }],
                "voix_des_titres": { "voice_id": "fr-1" }
            }
        }]
    });
    fs::write(&path, serde_json::to_string_pretty(&original).unwrap()).unwrap();
    let store = Store::new(dir.path());
    let read = store.read().unwrap();
    assert_eq!(read.notice, None);
    assert_eq!(read.entries[0].extra["couleur"], "bleu");
    assert_eq!(
        read.entries[0].composition_extra["voix_des_titres"]["voice_id"],
        "fr-1"
    );
    // Renommage : réécriture complète, aller-retour sans perte.
    store.rename("abc-1", "Renommée").unwrap();
    let mut expected = original.clone();
    expected["compositions"][0]["name"] = "Renommée".into();
    let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written, expected);
    // La même composition sans le champ inconnu n'est pas « identique » (#484, revue) :
    // nouvelle entrée, l'entrée aux champs inconnus reste intacte ; une autre conservation ne
    // les perd pas non plus.
    let mut known_value = original["compositions"][0]["composition"].clone();
    known_value
        .as_object_mut()
        .unwrap()
        .remove("voix_des_titres");
    let known: PackComposition = serde_json::from_value(known_value).unwrap();
    let derived = store.record(&known, "T", 9).unwrap();
    assert!(derived.new);
    assert_eq!(derived.name, "T");
    store.record(&comp("B", &["1", "2"]), "B", 10).unwrap();
    let written: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["reglages_futurs"], original["reglages_futurs"]);
    assert_eq!(written["compositions"][0]["couleur"], "bleu");
    assert_eq!(
        written["compositions"][0]["composition"]["voix_des_titres"],
        original["compositions"][0]["composition"]["voix_des_titres"]
    );
    assert_eq!(written["compositions"][0]["packs"], 2);
    assert_eq!(written["compositions"][1]["name"], "T");
    assert!(written["compositions"][1]["composition"]
        .get("voix_des_titres")
        .is_none());
    assert!(written["compositions"][1].get("couleur").is_none());
    assert!(written.get("extra").is_none());
    assert!(written["compositions"][1].get("extra").is_none());
    assert!(written["compositions"][1]
        .get("composition_extra")
        .is_none());
    // Ouvrir copie la composition connue dans le brouillon (dont le schéma n'a pas de champ
    // inconnu).
    assert!(store.open("abc-1").unwrap().saved);
    assert_eq!(
        pack::read_draft(dir.path()).unwrap().composition,
        Some(known)
    );
}

#[test]
fn champs_inconnus_disque_plein_fichier_intact() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let original = serde_json::json!({
        "schema_version": COMPOSITIONS_SCHEMA,
        "futur": 1,
        "compositions": [{
            "id": "abc-1", "name": "Futur", "couleur": "bleu",
            "composition": { "menu": [], "voix": "x" }
        }]
    })
    .to_string();
    fs::write(&path, &original).unwrap();
    let full = Store::with_writer(dir.path(), disk_full);
    assert!(matches!(full.rename("abc-1", "Z"), Err(AppError::Io(_))));
    assert!(matches!(full.delete("abc-1"), Err(AppError::Io(_))));
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(leftovers(dir.path()), 0);
}

#[test]
fn taille_mesuree_en_json_indente() {
    let dir = tempfile::tempdir().unwrap();
    let mut c = comp("P", &["e1", "e2", "e3", "e4"]);
    c.metadata.description = Some(String::new());
    let compact = serde_json::to_vec(&c).unwrap().len();
    let pretty = serde_json::to_vec_pretty(&c).unwrap().len();
    let delta = pretty - compact;
    assert!(delta >= 2);
    // Compact sous la limite, indenté au-dessus : refusée.
    c.metadata.description = Some("d".repeat(MAX_ENTRY_BYTES - compact - delta / 2));
    assert!(serde_json::to_vec(&c).unwrap().len() <= MAX_ENTRY_BYTES);
    assert!(serde_json::to_vec_pretty(&c).unwrap().len() > MAX_ENTRY_BYTES);
    let err = Store::new(dir.path()).record(&c, "P", 1).unwrap_err();
    assert!(matches!(err, AppError::Validation(ref m) if m.contains("trop volumineuse")));
    // Indentée à la limite exacte : acceptée.
    c.metadata.description = Some("d".repeat(MAX_ENTRY_BYTES - pretty));
    assert_eq!(
        serde_json::to_vec_pretty(&c).unwrap().len(),
        MAX_ENTRY_BYTES
    );
    assert!(Store::new(dir.path()).record(&c, "P", 1).is_ok());
}

#[test]
fn surnombre_et_doublons_distingues_des_illisibles() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let entry = |i: usize, name: &str| {
        serde_json::json!({
            "id": format!("id-{i}"),
            "name": name,
            "composition": { "menu": [] }
        })
    };
    let mut list: Vec<Value> = (0..MAX_COMPOSITIONS + 2)
        .map(|i| entry(i, &format!("N{i}")))
        .collect();
    list.push(entry(999, "n0"));
    fs::write(
        &path,
        serde_json::json!({ "schema_version": 1, "compositions": list }).to_string(),
    )
    .unwrap();
    let read = Store::new(dir.path()).read().unwrap();
    assert_eq!(read.entries.len(), MAX_COMPOSITIONS);
    let notice = read.notice.unwrap();
    assert!(notice.contains("3 composition(s)"), "{notice}");
    assert!(
        notice.contains(&format!("2 au-delà de {MAX_COMPOSITIONS}")),
        "{notice}"
    );
    assert!(notice.contains("1 en double"), "{notice}");
    assert!(!notice.contains("illisible"), "{notice}");
}

thread_local! {
    /// Écritures encore à faire échouer par [`flaky`] (par fil : les tests tournent en
    /// parallèle).
    static FAILS_LEFT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Écrivain qui échoue (disque plein) `FAILS_LEFT` fois puis écrit normalement : verrou
/// transitoire, place libérée entre deux écritures.
fn flaky(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let fail = FAILS_LEFT.with(|f| {
        let n = f.get();
        f.set(n.saturating_sub(1));
        n > 0
    });
    if fail {
        disk_full(path, bytes)
    } else {
        storage::write_atomic(path, bytes)
    }
}

/// Copies contenant l'entrée écartée `"x"` de [`file_with_rejected_entry`].
fn copies_with_rejected(dir: &Path) -> usize {
    backups(dir)
        .iter()
        .filter(|n| {
            fs::read_to_string(dir.join(n))
                .unwrap()
                .contains("\"id\":\"x\"")
        })
        .count()
}

#[test]
fn reecriture_en_echec_puis_action_reussie_copie_d_abord() {
    // Revue #484 (B1) : la lecture n'a pas pu réécrire (copie retirée), l'action suivante
    // réussit : elle copie d'abord, l'entrée écartée n'est jamais perdue sans copie.
    for action in ["rename", "delete", "record"] {
        let dir = tempfile::tempdir().unwrap();
        let id = file_with_rejected_entry(dir.path());
        let store = Store::with_writer(dir.path(), flaky);
        FAILS_LEFT.with(|f| f.set(1));
        // #489 : le message dit le nouvel état (fichier copié, copie nommée), plus l'échec de
        // la réécriture faite à la lecture.
        let notice = match action {
            "rename" => store.rename(&id, "Z").unwrap(),
            "delete" => store.delete(&id).unwrap(),
            _ => {
                store
                    .record(&comp("N", &["7", "8"]), "N", 2)
                    .unwrap()
                    .notice
            }
        }
        .unwrap();
        let text = fs::read_to_string(dir.path().join(COMPOSITIONS_FILE)).unwrap();
        assert!(
            !text.contains("\"x\""),
            "{action} : fichier réécrit sans l'entrée"
        );
        assert_eq!(
            copies_with_rejected(dir.path()),
            1,
            "{action} : copie faite"
        );
        let copy = backups(dir.path()).pop().unwrap();
        assert!(
            notice.contains("1 composition(s) enregistrée(s) écartée(s) (1 illisible(s))")
                && notice.contains(&format!("copié ({copy})"))
                && !notice.contains("n'a pas pu"),
            "{action} : {notice}"
        );
        assert_eq!(leftovers(dir.path()), 0);
    }
}

#[test]
fn reecriture_et_action_en_echec_fichier_intact_sans_copie() {
    let dir = tempfile::tempdir().unwrap();
    let id = file_with_rejected_entry(dir.path());
    let path = dir.path().join(COMPOSITIONS_FILE);
    let before = fs::read(&path).unwrap();
    FAILS_LEFT.with(|f| f.set(2));
    let err = Store::with_writer(dir.path(), flaky)
        .rename(&id, "Z")
        .unwrap_err();
    assert!(matches!(err, AppError::Io(_)));
    assert_eq!(fs::read(&path).unwrap(), before);
    assert!(backups(dir.path()).is_empty());
    assert_eq!(leftovers(dir.path()), 0);
    // Plus tard, le disque revenu : copie puis réécriture.
    FAILS_LEFT.with(|f| f.set(0));
    Store::with_writer(dir.path(), flaky)
        .rename(&id, "Z")
        .unwrap();
    assert_eq!(copies_with_rejected(dir.path()), 1);
}

#[test]
fn copie_neuve_gardee_meme_si_l_horloge_a_recule() {
    // Revue #484 (M1) : des copies datées du futur ne font pas supprimer la copie neuve.
    let dir = tempfile::tempdir().unwrap();
    let future: Vec<String> = (1..=3)
        .map(|i| format!("{COMPOSITIONS_FILE}.2099010{i}-000000.bak"))
        .collect();
    for name in &future {
        fs::write(dir.path().join(name), b"futur").unwrap();
    }
    fs::write(dir.path().join(COMPOSITIONS_FILE), b"tronque").unwrap();
    assert!(Store::new(dir.path()).read().unwrap().notice.is_some());
    let saved = backups(dir.path());
    assert_eq!(saved.len(), MAX_BACKUPS);
    let contents: Vec<String> = saved
        .iter()
        .map(|n| fs::read_to_string(dir.path().join(n)).unwrap())
        .collect();
    assert!(contents.iter().any(|c| c == "tronque"), "{saved:?}");
    // Entrées écartées : idem.
    let dir = tempfile::tempdir().unwrap();
    for name in &future {
        fs::write(dir.path().join(name), b"futur").unwrap();
    }
    file_with_rejected_entry(dir.path());
    Store::new(dir.path()).read().unwrap();
    assert_eq!(backups(dir.path()).len(), MAX_BACKUPS);
    assert_eq!(copies_with_rejected(dir.path()), 1);
}

#[test]
fn copies_comparees_sur_le_nom_brut() {
    // #489 : un nom qui n'est pas de l'Unicode valide n'est jamais confondu avec une copie (ni
    // avec la copie gardée) après une conversion avec perte : jamais supprimé.
    let dir = tempfile::tempdir().unwrap();
    for i in 1..=4 {
        let name = format!("{COMPOSITIONS_FILE}.2020010{i}-000000.bak");
        fs::write(dir.path().join(name), b"ancien").unwrap();
    }
    let keep = format!("{COMPOSITIONS_FILE}.20200105-000000.bak");
    fs::write(dir.path().join(&keep), b"neuve").unwrap();
    #[cfg(windows)]
    let odd = {
        use std::os::windows::ffi::OsStringExt;
        // Demi-paire de substitution isolée : U+FFFD une fois converti avec perte.
        let mut wide: Vec<u16> = format!("{COMPOSITIONS_FILE}.20200101-000000")
            .encode_utf16()
            .collect();
        wide.push(0xD800);
        wide.extend(".bak".encode_utf16());
        std::ffi::OsString::from_wide(&wide)
    };
    #[cfg(unix)]
    let odd = {
        use std::os::unix::ffi::OsStringExt;
        let mut bytes = format!("{COMPOSITIONS_FILE}.20200101-000000").into_bytes();
        bytes.push(0xFF);
        bytes.extend(b".bak");
        std::ffi::OsString::from_vec(bytes)
    };
    assert_eq!(backup_rank(&odd), None);
    fs::write(dir.path().join(&odd), b"etrange").unwrap();
    prune_backups(dir.path(), OsStr::new(&keep));
    assert!(dir.path().join(&keep).exists(), "copie neuve gardée");
    assert!(
        dir.path().join(&odd).exists(),
        "nom non Unicode jamais supprimé"
    );
    let saved = backups(dir.path());
    assert_eq!(saved.len(), MAX_BACKUPS + 1, "{saved:?}");
    assert!(saved.contains(&format!("{COMPOSITIONS_FILE}.20200104-000000.bak")));
    assert!(saved.contains(&format!("{COMPOSITIONS_FILE}.20200103-000000.bak")));
    // `keep` égal à la forme convertie du nom non Unicode : aucun nom brut ne lui est égal,
    // rien n'est protégé à tort, le nom non Unicode reste.
    let lossy = odd.to_string_lossy().into_owned();
    prune_backups(dir.path(), OsStr::new(&lossy));
    assert!(dir.path().join(&odd).exists());
    assert_eq!(backups(dir.path()).len(), MAX_BACKUPS);
}
