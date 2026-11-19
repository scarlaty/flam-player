//! Uuid et version (§ 8) : création, actualisation, dossier non inscriptible, dossier copié ou
//! déplacé, uuid d'une autre source.

use std::fs;

use super::*;
use crate::authoring::build::story_entry;
use crate::podcast::pack::ResolvedMetadata;

const UUID: &str = "5d1c2b3a-4e5f-4a6b-8c7d-9e0f1a2b3c4d";

fn story(dir: &Path) -> Loaded<StorySettings> {
    settings::load_story(dir)
}

fn installed(uuid: &str, version: u32, source_id: &str, folder: &Path) -> LibraryEntry {
    let meta = ResolvedMetadata {
        title: "Installée".into(),
        subtitle: String::new(),
        age: 0,
        description: None,
    };
    let mut source = story_entry(uuid, version, &meta, folder);
    source.source_id = source_id.into();
    LibraryEntry {
        uuid: uuid.into(),
        version: Some(version),
        title: "Installée".into(),
        file: format!("{uuid}-{version}.plain.pk"),
        size: 1,
        installed_at: 0,
        converter: None,
        source,
        thumb: None,
        source_stamp: None,
    }
}

fn json(dir: &Path) -> serde_json::Value {
    serde_json::from_slice(&fs::read(dir.join(settings::STORY_FILE)).unwrap()).unwrap()
}

#[test]
fn creation_uuid_neuf_version_1_ecrit_dans_le_dossier() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join(settings::STORY_FILE),
        r#"{"titre": "Mon titre", "perso": 42}"#,
    )
    .unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    let identity = inspect(&folder, &story(&folder), None, &[]);
    assert_eq!(identity, Identity::default());
    let decision = decide(&identity, None).unwrap();
    assert!(decision.created);
    assert_eq!(decision.version, 1);
    assert!(is_usable_uuid(&decision.uuid));
    assert_eq!(
        commit(&folder, &decision, &registry).unwrap(),
        SavedIn::Folder
    );
    let written = json(&folder);
    assert_eq!(written["uuid"], decision.uuid.as_str());
    assert_eq!(written["version"], 1);
    // Rien de perdu : titre et clé inconnue gardés.
    assert_eq!(written["titre"], "Mon titre");
    assert_eq!(written["perso"], 42);
    assert!(registry.load().is_empty());
}

#[test]
fn actualisation_meme_uuid_version_suivante() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join(settings::STORY_FILE),
        format!(r#"{{"uuid": "{UUID}", "version": 3}}"#),
    )
    .unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    // Installée en version 4 (une actualisation précédente dont l'écriture a été perdue).
    let library = [installed(
        UUID,
        4,
        crate::authoring::build::SOURCE_ID,
        &folder,
    )];
    let identity = inspect(&folder, &story(&folder), None, &library);
    assert_eq!(identity.uuid.as_deref(), Some(UUID));
    assert_eq!(identity.installed_version, Some(4));
    assert_eq!(identity.conflict, None);
    let decision = decide(&identity, None).unwrap();
    assert_eq!(
        decision,
        Decision {
            uuid: UUID.into(),
            version: 5,
            created: false
        }
    );
    commit(&folder, &decision, &registry).unwrap();
    assert_eq!(json(&folder)["version"], 5);
    // Et la suivante.
    let again = decide(&inspect(&folder, &story(&folder), None, &library), None).unwrap();
    assert_eq!(again.version, 6);
}

#[test]
fn dossier_non_inscriptible_uuid_garde_par_le_launcher() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    // Réglages abîmés : jamais réécrits sans sauvegarde.
    let damaged = "{ pas du json";
    fs::write(folder.join(settings::STORY_FILE), damaged).unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    let identity = inspect(
        &folder,
        &story(&folder),
        registry.find(&folder).as_ref(),
        &[],
    );
    let decision = decide(&identity, None).unwrap();
    assert_eq!(
        commit(&folder, &decision, &registry).unwrap(),
        SavedIn::Launcher
    );
    assert_eq!(
        fs::read_to_string(folder.join(settings::STORY_FILE)).unwrap(),
        damaged
    );
    // Relu par chemin (casse indifférente) : actualisation avec le même uuid.
    let upper = PathBuf::from(folder.to_string_lossy().to_uppercase());
    let remembered = registry.find(if cfg!(windows) { &upper } else { &folder });
    let identity = inspect(&folder, &story(&folder), remembered.as_ref(), &[]);
    assert!(identity.remembered);
    assert_eq!(identity.uuid.as_deref(), Some(decision.uuid.as_str()));
    let next = decide(&identity, None).unwrap();
    assert_eq!(
        (next.uuid.as_str(), next.version),
        (decision.uuid.as_str(), 2)
    );
    commit(&folder, &next, &registry).unwrap();
    assert_eq!(registry.find(&folder).unwrap().version, 2);
}

#[test]
fn format_plus_recent_jamais_reecrit() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    let text = r#"{"format": 9, "titre": "Futur"}"#;
    fs::write(folder.join(settings::STORY_FILE), text).unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    let decision = decide(&inspect(&folder, &story(&folder), None, &[]), None).unwrap();
    assert_eq!(
        commit(&folder, &decision, &registry).unwrap(),
        SavedIn::Launcher
    );
    assert_eq!(
        fs::read_to_string(folder.join(settings::STORY_FILE)).unwrap(),
        text
    );
}

#[test]
fn dossier_redevenu_inscriptible_l_uuid_y_est_ecrit_et_oublie_du_launcher() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    registry.remember(&folder, UUID, 2, None).unwrap();
    let identity = inspect(
        &folder,
        &story(&folder),
        registry.find(&folder).as_ref(),
        &[],
    );
    let decision = decide(&identity, None).unwrap();
    assert_eq!((decision.uuid.as_str(), decision.version), (UUID, 3));
    assert_eq!(
        commit(&folder, &decision, &registry).unwrap(),
        SavedIn::Folder
    );
    assert!(registry.find(&folder).is_none());
    assert_eq!(json(&folder)["uuid"], UUID);
}

#[test]
fn dossier_copie_choix_remplacer_ou_nouvelle_histoire() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("Original");
    let copy = dir.path().join("Copie");
    for folder in [&original, &copy] {
        fs::create_dir_all(folder).unwrap();
        fs::write(
            folder.join(settings::STORY_FILE),
            format!(r#"{{"uuid": "{UUID}", "version": 2}}"#),
        )
        .unwrap();
    }
    let library = [installed(
        UUID,
        2,
        crate::authoring::build::SOURCE_ID,
        &original,
    )];
    let identity = inspect(&copy, &story(&copy), None, &library);
    assert_eq!(
        identity.conflict,
        Some(Conflict::Copied {
            other_folder: original.to_string_lossy().into_owned(),
            title: "Installée".into(),
        })
    );
    // Sans choix : refus.
    assert!(decide(&identity, None).is_err());
    let replace = decide(&identity, Some(ConflictChoice::ReplaceOther)).unwrap();
    assert_eq!((replace.uuid.as_str(), replace.version), (UUID, 3));
    let fresh = decide(&identity, Some(ConflictChoice::NewStory)).unwrap();
    assert_ne!(fresh.uuid, UUID);
    assert_eq!(fresh.version, 1);
    assert!(fresh.created);
    // Le dossier d'origine lui-même n'est pas en conflit.
    assert_eq!(
        inspect(&original, &story(&original), None, &library).conflict,
        None
    );
}

#[test]
fn dossier_deplace_n_est_pas_un_conflit() {
    let dir = tempfile::tempdir().unwrap();
    let moved = dir.path().join("Nouveau");
    fs::create_dir_all(&moved).unwrap();
    fs::write(
        moved.join(settings::STORY_FILE),
        format!(r#"{{"uuid": "{UUID}", "version": 1}}"#),
    )
    .unwrap();
    // L'ancien chemin n'existe plus.
    let library = [installed(
        UUID,
        1,
        crate::authoring::build::SOURCE_ID,
        &dir.path().join("Ancien"),
    )];
    let identity = inspect(&moved, &story(&moved), None, &library);
    assert_eq!(identity.conflict, None);
    assert_eq!(decide(&identity, None).unwrap().version, 2);
}

#[test]
fn uuid_d_une_autre_source_seulement_nouvelle_histoire() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join(settings::STORY_FILE),
        format!(r#"{{"uuid": "{UUID}"}}"#),
    )
    .unwrap();
    let library = [installed(
        UUID,
        7,
        "telmi",
        Path::new("https://exemple.org/a.zip"),
    )];
    let identity = inspect(&folder, &story(&folder), None, &library);
    assert!(matches!(identity.conflict, Some(Conflict::Foreign { .. })));
    assert!(decide(&identity, None).is_err());
    assert!(decide(&identity, Some(ConflictChoice::ReplaceOther)).is_err());
    let fresh = decide(&identity, Some(ConflictChoice::NewStory)).unwrap();
    assert_ne!(fresh.uuid, UUID);
}

#[test]
fn uuid_nul_vaut_absent() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join(settings::STORY_FILE),
        r#"{"uuid": "00000000-0000-0000-0000-000000000000", "version": 4}"#,
    )
    .unwrap();
    let identity = inspect(&folder, &story(&folder), None, &[]);
    assert_eq!(identity.uuid, None);
    assert!(decide(&identity, None).unwrap().created);
}

#[test]
fn registre_borne_et_relu() {
    let dir = tempfile::tempdir().unwrap();
    let registry = Registry::new(dir.path());
    for n in 0..(MAX_REMEMBERED + 5) {
        registry
            .remember(&dir.path().join(format!("d{n}")), UUID, 1, None)
            .unwrap();
    }
    let all = registry.load();
    assert_eq!(all.len(), MAX_REMEMBERED);
    assert!(registry.find(&dir.path().join("d0")).is_none());
    assert!(registry
        .find(&dir.path().join(format!("d{}", MAX_REMEMBERED + 4)))
        .is_some());
    registry
        .forget(&dir.path().join(format!("d{}", MAX_REMEMBERED + 4)))
        .unwrap();
    assert_eq!(registry.load().len(), MAX_REMEMBERED - 1);
}

#[test]
fn seuls_uuid_et_version_changent_dans_le_fichier() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    // Valeurs que la lecture tolérante nettoierait : description sur plusieurs lignes, entrée
    // invalide dans `ordre`, casse d'une énumération, espaces d'un titre.
    let text = "{\"description\": \"Ligne 1\\nLigne 2\", \"ordre\": [\"a.mp3\", \"x/y\", 3], \
                \"mode\": \"Chapitres\", \"titre\": \"  Titre  \"}";
    fs::write(folder.join(settings::STORY_FILE), text).unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    let decision = decide(&inspect(&folder, &story(&folder), None, &[]), None).unwrap();
    assert_eq!(
        commit(&folder, &decision, &registry).unwrap(),
        SavedIn::Folder
    );
    let mut expected: serde_json::Value = serde_json::from_str(text).unwrap();
    expected["uuid"] = decision.uuid.as_str().into();
    expected["version"] = 1.into();
    assert_eq!(json(&folder), expected);
}

#[test]
fn dossier_copie_non_inscriptible_nouvelle_histoire_puis_actualisation() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("Original");
    let copy = dir.path().join("Copie");
    fs::create_dir_all(&original).unwrap();
    fs::write(
        original.join(settings::STORY_FILE),
        format!(r#"{{"uuid": "{UUID}", "version": 2}}"#),
    )
    .unwrap();
    // La copie a un `_histoire.json` au format plus récent : jamais réécrit.
    fs::create_dir_all(&copy).unwrap();
    fs::write(
        copy.join(settings::STORY_FILE),
        format!(r#"{{"format": 9, "uuid": "{UUID}", "version": 2}}"#),
    )
    .unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    let source = crate::authoring::build::SOURCE_ID;
    let mut library = vec![installed(UUID, 2, source, &original)];
    let identity = inspect(
        &copy,
        &story(&copy),
        registry.find(&copy).as_ref(),
        &library,
    );
    assert!(matches!(identity.conflict, Some(Conflict::Copied { .. })));
    let fresh = decide(&identity, Some(ConflictChoice::NewStory)).unwrap();
    assert_eq!(commit(&copy, &fresh, &registry).unwrap(), SavedIn::Launcher);
    library.push(installed(&fresh.uuid, 1, source, &copy));
    // Actualisation : l'uuid de la nouvelle histoire (configuration) l'emporte sur celui du
    // fichier, sans conflit ni doublon.
    let identity = inspect(
        &copy,
        &story(&copy),
        registry.find(&copy).as_ref(),
        &library,
    );
    assert_eq!(identity.uuid.as_deref(), Some(fresh.uuid.as_str()));
    assert_eq!(identity.conflict, None);
    let next = decide(&identity, None).unwrap();
    assert_eq!((next.uuid.as_str(), next.version), (fresh.uuid.as_str(), 2));
}

#[test]
fn entree_perimee_du_launcher_le_fichier_l_emporte() {
    // Le launcher a retenu U2 pour ce chemin quand le fichier portait UUID (dossier non
    // inscriptible) ; au même chemin, le fichier porte maintenant un autre uuid (autre clé USB
    // sous la même lettre, ou uuid écrit à la main) : il l'emporte.
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Histoire");
    fs::create_dir_all(&folder).unwrap();
    let other = "11111111-2222-4333-8444-555555555555";
    fs::write(
        folder.join(settings::STORY_FILE),
        format!(r#"{{"uuid": "{other}", "version": 4}}"#),
    )
    .unwrap();
    let registry = Registry::new(&dir.path().join("config"));
    let u2 = "22222222-3333-4444-8555-666666666666";
    registry.remember(&folder, u2, 3, Some(UUID)).unwrap();
    let identity = inspect(
        &folder,
        &story(&folder),
        registry.find(&folder).as_ref(),
        &[],
    );
    assert_eq!(identity.uuid.as_deref(), Some(other));
    assert!(!identity.remembered);
    let decision = decide(&identity, None).unwrap();
    assert_eq!((decision.uuid.as_str(), decision.version), (other, 5));
    // Écrit dans le fichier : l'entrée périmée est oubliée.
    assert_eq!(
        commit(&folder, &decision, &registry).unwrap(),
        SavedIn::Folder
    );
    assert!(registry.find(&folder).is_none());
}
