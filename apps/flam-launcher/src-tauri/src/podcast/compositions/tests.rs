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
        SaveOutcome {
            saved: true,
            revision: 8
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
    assert!(store.open(&a.id).unwrap().saved);
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
    let v = view(store.read().unwrap(), &podcasts);
    assert_eq!(v.max, MAX_COMPOSITIONS);
    // Dernière utilisée d'abord.
    let got: Vec<_> = v
        .compositions
        .iter()
        .map(|s| (s.name.as_str(), s.episodes, s.feeds, s.missing_feeds))
        .collect();
    assert_eq!(got, [("B", 2, 2, 1), ("A", 2, 2, 1)]);
    let all: HashSet<String> = ["a".to_string(), "b".to_string()].into();
    assert!(view(store.read().unwrap(), &all)
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
    assert!(read.notice.unwrap().contains("4 composition(s)"));
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
fn schema_plus_recent_mis_de_cote_jamais_ecrase() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(COMPOSITIONS_FILE);
    let future = format!(
        r#"{{"schema_version": {}, "compositions": []}}"#,
        COMPOSITIONS_SCHEMA + 1
    );
    fs::write(&path, &future).unwrap();
    let read = Store::new(dir.path()).read().unwrap();
    assert!(read.notice.unwrap().contains("plus récente"));
    let saved = backups(dir.path());
    assert_eq!(
        fs::read_to_string(dir.path().join(&saved[0])).unwrap(),
        future
    );
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
    let read = Store::with_writer(dir.path(), disk_full).read().unwrap();
    // Entrées lisibles servies, fichier d'origine intact et copié.
    assert_eq!(read.entries.len(), 1);
    assert!(read.notice.is_some());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(backups(dir.path()).len(), 1);
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
    let json = serde_json::to_string(&view(store.read().unwrap(), &HashSet::new())).unwrap();
    assert!(!json.contains("http"));
    assert!(!json.contains(&dir.path().display().to_string()));
}
