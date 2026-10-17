//! Tests sur faux appareil (dossier temporaire) : lecture tolérante, écriture, coupure
//! simulée après chaque étape puis reprise, états laissés par un renommage FAT interrompu.

use super::*;

const A: &str = "4639e3ea-13b4-4550-91c9-f930a4df6b93";
const B: &str = "5fb70290-6246-419e-a764-bca1635fc850";
const C: &str = "76b88912-f3dc-41f4-b4a7-6aa809d57ad7";
const D: &str = "9a8521bf-0ae0-40de-8f8b-c22376bad8fd";

fn id(text: &str) -> StoryId {
    StoryId::parse(text).unwrap()
}

fn ids(texts: &[&str]) -> Vec<StoryId> {
    texts.iter().map(|t| id(t)).collect()
}

fn lines(texts: &[&str]) -> Vec<u8> {
    serialize(&ids(texts))
}

/// Faux appareil : dossiers `str/<uuid>` pour A à D, cache du firmware, index facultatif.
struct Device {
    dir: tempfile::TempDir,
}

impl Device {
    fn new(list: Option<&[u8]>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for uuid in [A, B, C, D] {
            fs::create_dir_all(root.join(STORIES_DIR).join(uuid)).unwrap();
        }
        fs::create_dir_all(root.join(LIBRARY_DIR)).unwrap();
        fs::create_dir_all(root.join("usr/0")).unwrap();
        fs::write(root.join(LIBRARY_CACHE), b"cache").unwrap();
        let device = Self { dir };
        if let Some(bytes) = list {
            device.put("list", bytes);
        }
        device
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root().join(LIBRARY_DIR).join(name)
    }

    fn put(&self, name: &str, bytes: &[u8]) {
        fs::write(self.path(name), bytes).unwrap();
    }

    fn get(&self, name: &str) -> Option<Vec<u8>> {
        fs::read(self.path(name)).ok()
    }

    fn cache_present(&self) -> bool {
        self.root().join(LIBRARY_CACHE).exists()
    }

    /// Aucun fichier de travail ne reste après une reprise.
    fn assert_no_work_files(&self, base: &str) {
        for suffix in ["new", "bak", "restore"] {
            let name = format!("{base}.{suffix}");
            assert!(self.get(&name).is_none(), "{name} doit avoir disparu");
        }
    }
}

// ---------------------------------------------------------------- lecture tolérante

#[test]
fn index_absent() {
    let dev = Device::new(None);
    let read = read_index(dev.root(), IndexKind::Visible);
    assert_eq!(read.state, IndexState::Absent);
    assert!(read.entries.is_empty());
}

#[test]
fn index_vide_valide() {
    let read = parse(b"");
    assert_eq!(read.state, IndexState::Valid);
    assert!(read.entries.is_empty());
    assert_eq!(parse(b"\n\r\n  \n").state, IndexState::Valid);
}

#[test]
fn fins_de_ligne_mixtes_espaces_bom_et_majuscules() {
    let text = format!(
        "\u{feff}{A}\r\n\n  {B}  \n{}\r\n\r\n{D}",
        C.to_ascii_uppercase()
    );
    let read = parse(text.as_bytes());
    assert_eq!(read.state, IndexState::Valid);
    assert_eq!(read.entries, ids(&[A, B, C, D]));
}

#[test]
fn doublons_ignores_et_signales() {
    let text = format!("{A}\n{B}\n{}\n{B}\n", A.to_ascii_uppercase());
    let read = parse(text.as_bytes());
    assert_eq!(read.state, IndexState::Damaged);
    assert_eq!(read.entries, ids(&[A, B]));
    assert_eq!(
        read.issues,
        vec![
            LineIssue::Duplicate { line: 3, id: id(A) },
            LineIssue::Duplicate { line: 4, id: id(B) },
        ]
    );
}

#[test]
fn uuid_invalides_ignores_et_signales() {
    let text = format!("{A}\npas-un-uuid\n{{{B}}}\n{C}\n");
    let read = parse(text.as_bytes());
    assert_eq!(read.state, IndexState::Damaged);
    assert_eq!(read.entries, ids(&[A, C]));
    assert_eq!(
        read.issues,
        vec![
            LineIssue::Invalid {
                line: 2,
                text: "pas-un-uuid".into()
            },
            LineIssue::Invalid {
                line: 3,
                text: format!("{{{B}}}")
            },
        ]
    );
}

#[test]
fn index_tronque_derniere_ligne_signalee() {
    let full = lines(&[A, B, C]);
    let read = parse(&full[..full.len() - 10]);
    assert_eq!(read.state, IndexState::Damaged);
    assert_eq!(read.entries, ids(&[A, B]));
    assert!(matches!(
        read.issues[..],
        [LineIssue::Invalid { line: 3, .. }]
    ));
}

#[test]
fn texte_invalide_long_tronque_dans_le_signalement() {
    let long = "x".repeat(500);
    let read = parse(format!("{A}\n{long}\n").as_bytes());
    match &read.issues[..] {
        [LineIssue::Invalid { text, .. }] => assert_eq!(text.len(), ISSUE_TEXT_MAX),
        other => panic!("signalement inattendu : {other:?}"),
    }
}

#[test]
fn contenu_illisible() {
    assert!(matches!(
        parse(b"n'importe quoi\n").state,
        IndexState::Unreadable(_)
    ));
    let mut nul = lines(&[A]);
    nul.extend_from_slice(&[0, 0, 0]);
    assert!(matches!(parse(&nul).state, IndexState::Unreadable(_)));
    assert!(matches!(
        parse(&[0xff, 0xfe, 0x80, 0x81]).state,
        IndexState::Unreadable(_)
    ));
}

#[test]
fn fichier_trop_gros_illisible() {
    let dev = Device::new(None);
    dev.put("list", &vec![b'\n'; MAX_INDEX_BYTES as usize + 1]);
    let read = read_index(dev.root(), IndexKind::Visible);
    assert!(matches!(read.state, IndexState::Unreadable(_)));
}

#[test]
fn aller_retour_identique_sur_index_normalise() {
    // Format observé sur l'appareil : 10 UUID, LF final, 370 octets.
    let original = lines(&[
        A,
        B,
        C,
        D,
        "a6c67c68-2db7-4713-9cf9-0d0e33336bc8",
        "b0b3657c-9f0b-420a-969f-ffe7670e73d9",
        "b2794eb7-82ba-4971-a567-e9f2880674b2",
        "c103c95c-a852-4d8f-9c03-82f3814b572d",
        "d2ccf774-b623-4e3c-908a-ae1bb78dcebf",
        "f3f67d0d-79a6-4511-9cb2-389ac40001b9",
    ]);
    assert_eq!(original.len(), 370);
    let dev = Device::new(Some(&original));
    let read = read_index(dev.root(), IndexKind::Visible);
    assert_eq!(read.state, IndexState::Valid);
    let outcome = write_index(
        dev.root(),
        IndexKind::Visible,
        &read.entries,
        Consent::IfValid,
    )
    .unwrap();
    assert_eq!(outcome, WriteOutcome::Unchanged);
    assert_eq!(dev.get("list").unwrap(), original);
    assert!(dev.cache_present(), "aucune écriture : cache gardé");
}

#[test]
fn aller_retour_normalise_la_forme() {
    let messy = format!("{}\r\n\r\n{B}", A.to_ascii_uppercase());
    let read = parse(messy.as_bytes());
    assert_eq!(serialize(&read.entries), format!("{A}\n{B}\n").into_bytes());
    assert_eq!(parse(&serialize(&read.entries)), {
        let mut again = read.clone();
        again.issues.clear();
        again
    });
}

#[test]
fn serialize_retire_les_doublons() {
    assert_eq!(serialize(&ids(&[A, B, A])), lines(&[A, B]));
    assert!(serialize(&[]).is_empty());
}

// ---------------------------------------------------------------- écriture

#[test]
fn ecriture_remplace_et_supprime_le_cache() {
    let dev = Device::new(Some(&lines(&[A, B])));
    let outcome = write_index(
        dev.root(),
        IndexKind::Visible,
        &ids(&[B, A, C]),
        Consent::IfValid,
    )
    .unwrap();
    assert_eq!(outcome, WriteOutcome::Written);
    assert_eq!(dev.get("list").unwrap(), lines(&[B, A, C]));
    assert!(!dev.cache_present());
    dev.assert_no_work_files("list");
}

#[test]
fn ecriture_refusee_sans_accord_si_index_absent_abime_ou_illisible() {
    let cases: [Option<Vec<u8>>; 3] = [
        None,
        Some(format!("{A}\nabîmé\n").into_bytes()),
        Some(b"\x01\x02 illisible".to_vec()),
    ];
    for list in cases {
        let dev = Device::new(list.as_deref());
        let err =
            write_index(dev.root(), IndexKind::Visible, &ids(&[A]), Consent::IfValid).unwrap_err();
        assert!(matches!(err, IndexError::NeedsConsent), "{err}");
        assert_eq!(dev.get("list"), list, "index intact");
        assert!(dev.cache_present());
        dev.assert_no_work_files("list");

        // Avec l'accord explicite : reconstruction depuis `str/`.
        let rebuilt = scan_stories(dev.root()).unwrap();
        write_index(dev.root(), IndexKind::Visible, &rebuilt, Consent::Overwrite).unwrap();
        assert_eq!(dev.get("list").unwrap(), lines(&[A, B, C, D]));
        assert!(!dev.cache_present());
    }
}

#[test]
fn list_hidden_absent_ecrit_sans_accord() {
    let dev = Device::new(Some(&lines(&[A])));
    write_index(dev.root(), IndexKind::Hidden, &ids(&[B]), Consent::IfValid).unwrap();
    assert_eq!(dev.get("list.hidden").unwrap(), lines(&[B]));
    assert_eq!(dev.get("list").unwrap(), lines(&[A]), "list intact");
    dev.assert_no_work_files("list.hidden");
}

#[test]
fn ecriture_d_une_liste_vide() {
    let dev = Device::new(Some(&lines(&[A])));
    write_index(dev.root(), IndexKind::Visible, &[], Consent::IfValid).unwrap();
    assert_eq!(dev.get("list").unwrap(), b"");
    dev.assert_no_work_files("list");
}

#[test]
fn echec_de_suppression_du_cache_garde_le_marqueur_puis_reprise() {
    let dev = Device::new(Some(&lines(&[A])));
    // Un dossier à la place du cache : sa suppression comme fichier échoue.
    let cache = dev.root().join(LIBRARY_CACHE);
    fs::remove_file(&cache).unwrap();
    fs::create_dir(&cache).unwrap();
    let err = write_index(
        dev.root(),
        IndexKind::Visible,
        &ids(&[A, B]),
        Consent::IfValid,
    )
    .unwrap_err();
    assert!(matches!(err, IndexError::CacheNotRemoved(_)), "{err}");
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B]), "index écrit");
    assert!(
        dev.get("list.bak").is_some(),
        "marqueur gardé pour réessayer"
    );

    fs::remove_dir(&cache).unwrap();
    fs::write(&cache, b"cache").unwrap();
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledForward
    );
    assert!(!dev.cache_present());
    dev.assert_no_work_files("list");
}

// ---------------------------------------------------------------- coupures simulées

const STEPS: [Step; 5] = [
    Step::NewWritten,
    Step::BackupWritten,
    Step::Replaced,
    Step::Verified,
    Step::CacheRemoved,
];

/// Écrit `new` sur un appareil dont l'index vaut `old`, coupe après `step`, vérifie l'état
/// laissé, reprend, puis vérifie les invariants.
fn cut_then_recover(kind: IndexKind, old: Option<&[&str]>, new: &[&str], step: Step) {
    let base = kind.file_name();
    let dev = Device::new(None);
    if let Some(old) = old {
        dev.put(base, &lines(old));
    }
    let old_bytes = old.map(lines);
    let new_bytes = lines(new);
    let consent = if old.is_some() || kind == IndexKind::Hidden {
        Consent::IfValid
    } else {
        Consent::Overwrite
    };

    let err = write_with(dev.root(), kind, &ids(new), consent, &mut |s| {
        if s == step {
            Err(io::Error::other("coupure"))
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert!(matches!(err, IndexError::Io { .. }), "{step:?} : {err}");

    // Pendant la coupure : l'index est l'ancien ou le nouveau, jamais vide ni tronqué.
    let during = dev.get(base);
    assert!(
        during == old_bytes || during.as_deref() == Some(&new_bytes[..]),
        "{step:?} : index pendant la coupure = {during:?}"
    );
    let replaced = matches!(step, Step::Replaced | Step::Verified | Step::CacheRemoved);
    assert_eq!(
        dev.cache_present(),
        step != Step::CacheRemoved,
        "{step:?} : cache supprimé seulement après l'écriture vérifiée"
    );

    // Index absent au départ et `bak` posé : impossible à distinguer d'un index perdu par un
    // renommage FAT interrompu ; `new` étant complet, la transaction est terminée.
    let replaced = replaced || (old.is_none() && step == Step::BackupWritten);
    let recovery = recover(dev.root(), kind).unwrap();
    let expected = if replaced {
        Recovery::RolledForward
    } else {
        Recovery::RolledBack
    };
    assert_eq!(recovery, expected, "{step:?}");
    let after = dev.get(base);
    if replaced {
        assert_eq!(after.as_deref(), Some(&new_bytes[..]), "{step:?}");
        assert!(!dev.cache_present(), "{step:?} : cache invalidé");
    } else {
        assert_eq!(after, old_bytes, "{step:?} : index d'origine gardé");
        assert!(dev.cache_present(), "{step:?} : cache inchangé");
    }
    dev.assert_no_work_files(base);
    let read = read_index(dev.root(), kind);
    assert!(
        missing_stories(dev.root(), &read.entries).is_empty(),
        "{step:?} : l'index pointe vers des dossiers existants"
    );
    assert_eq!(recover(dev.root(), kind).unwrap(), Recovery::Clean);

    // L'appareil reste utilisable : une nouvelle écriture aboutit.
    write_index(dev.root(), kind, &ids(new), consent)
        .map(|_| ())
        .unwrap();
    assert_eq!(dev.get(base).as_deref(), Some(&new_bytes[..]));
    dev.assert_no_work_files(base);
}

#[test]
fn coupure_a_chaque_etape_ajout() {
    for step in STEPS {
        cut_then_recover(IndexKind::Visible, Some(&[A, B, C]), &[A, B, C, D], step);
    }
}

#[test]
fn coupure_a_chaque_etape_retrait() {
    for step in STEPS {
        cut_then_recover(IndexKind::Visible, Some(&[A, B, C]), &[A, C], step);
    }
}

#[test]
fn coupure_a_chaque_etape_index_absent_au_depart() {
    for step in STEPS {
        cut_then_recover(IndexKind::Visible, None, &[A, B], step);
    }
}

#[test]
fn coupure_a_chaque_etape_list_hidden() {
    for step in STEPS {
        cut_then_recover(IndexKind::Hidden, Some(&[]), &[D], step);
        cut_then_recover(IndexKind::Hidden, None, &[D], step);
    }
}

// ---------------------------------------------------------------- états FAT et reprise

/// Appareil dans un état donné (fichiers `list*` posés à la main).
fn state(files: &[(&str, &[u8])]) -> Device {
    let dev = Device::new(None);
    for (name, bytes) in files {
        dev.put(name, bytes);
    }
    dev
}

#[test]
fn reprise_sans_fichier_de_travail() {
    let dev = state(&[("list", &lines(&[A]))]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::Clean
    );
    assert!(dev.cache_present());
}

#[test]
fn reprise_new_partiel_sans_bak() {
    let full = lines(&[A, B, C]);
    let dev = state(&[("list", &lines(&[A, B])), ("list.new", &full[..40])]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledBack
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B]));
    assert!(dev.cache_present());
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_bak_partiel_index_intact() {
    let old = lines(&[A, B, C]);
    let dev = state(&[
        ("list", &old),
        ("list.new", &lines(&[A])),
        ("list.bak", &old[..50]),
    ]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledBack
    );
    assert_eq!(dev.get("list").unwrap(), old);
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_renommage_fat_interrompu_cible_supprimee() {
    // MoveFileEx sur FAT : cible supprimée, source pas encore renommée.
    let dev = state(&[
        ("list.new", &lines(&[A, B, D])),
        ("list.bak", &lines(&[A, B])),
    ]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledForward
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B, D]));
    assert!(!dev.cache_present());
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_renommage_fat_entree_source_restee() {
    let new = lines(&[A, D]);
    let dev = state(&[
        ("list", &new),
        ("list.new", &new),
        ("list.bak", &lines(&[A])),
    ]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledForward
    );
    assert_eq!(dev.get("list").unwrap(), new);
    assert!(!dev.cache_present());
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_index_perdu_restaure_depuis_bak() {
    let dev = state(&[("list.bak", &lines(&[A, B]))]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RestoredFromBackup
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B]));
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_index_illisible_new_invalide_restaure_depuis_bak() {
    let dev = state(&[
        ("list", b"\0\0\0\0"),
        ("list.new", b"\0\0garbage"),
        ("list.bak", &lines(&[C])),
    ]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RestoredFromBackup
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[C]));
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_restauration_interrompue_reprend_depuis_bak() {
    let old = lines(&[A, B, C]);
    let dev = state(&[("list.bak", &old), ("list.restore", &old[..37])]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RestoredFromBackup
    );
    assert_eq!(
        dev.get("list").unwrap(),
        old,
        "le résidu tronqué n'est pas promu"
    );
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_bak_seul_index_present_termine_la_transaction() {
    let dev = state(&[("list", &lines(&[A, B])), ("list.bak", &lines(&[A]))]);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledForward
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B]));
    assert!(!dev.cache_present());
    dev.assert_no_work_files("list");
}

#[test]
fn reprise_sans_candidat_ne_touche_a_rien() {
    let files: [(&str, &[u8]); 2] = [("list.new", b"\0\0"), ("list.bak", b"\0\0garbage")];
    let dev = state(&files);
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::NeedsUser
    );
    for (name, bytes) in files {
        assert_eq!(dev.get(name).as_deref(), Some(bytes), "{name} intact");
    }
    assert!(dev.get("list").is_none());
    assert!(dev.cache_present());

    let err =
        write_index(dev.root(), IndexKind::Visible, &ids(&[A]), Consent::IfValid).unwrap_err();
    assert!(matches!(err, IndexError::NeedsConsent));

    let rebuilt = scan_stories(dev.root()).unwrap();
    write_index(dev.root(), IndexKind::Visible, &rebuilt, Consent::Overwrite).unwrap();
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B, C, D]));
    dev.assert_no_work_files("list");
}

// ---------------------------------------------------------------- dossiers d'histoires

#[test]
fn scan_et_histoires_manquantes() {
    let dev = Device::new(None);
    fs::create_dir(dev.root().join(STORIES_DIR).join("pas-un-uuid")).unwrap();
    fs::write(
        dev.root()
            .join(STORIES_DIR)
            .join("b2794eb7-82ba-4971-a567-e9f2880674b2"),
        b"fichier, pas dossier",
    )
    .unwrap();
    assert_eq!(scan_stories(dev.root()).unwrap(), ids(&[A, B, C, D]));
    let orphan = "f0662db2-f32e-428c-81ee-2e65625aad3b";
    assert_eq!(
        missing_stories(dev.root(), &ids(&[A, orphan])),
        ids(&[orphan])
    );
}
