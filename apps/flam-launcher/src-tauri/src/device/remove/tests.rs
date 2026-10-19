//! Tests sur faux appareil : retrait, refus, coupure simulée après chaque étape puis reprise.

use std::cell::Cell;

use super::*;
use crate::device::index::{serialize, LIBRARY_CACHE, LIBRARY_DIR};

const A: &str = "4639e3ea-13b4-4550-91c9-f930a4df6b93";
const B: &str = "5fb70290-6246-419e-a764-bca1635fc850";
const C: &str = "76b88912-f3dc-41f4-b4a7-6aa809d57ad7";
const H: &str = "9a8521bf-0ae0-40de-8f8b-c22376bad8fd";

fn id(text: &str) -> StoryId {
    StoryId::parse(text).unwrap()
}

/// Faux appareil : `.mdf`, histoires A, B, C visibles et H masquée, chacune avec des fichiers
/// et un sous-dossier, sauvegardes `usr/0/`, cache du firmware.
struct Device {
    dir: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let device = Self { dir };
        let root = device.root();
        fs::write(root.join(MDF_FILE), [7u8; 126]).unwrap();
        for uuid in [A, B, C] {
            device.story(STORIES_DIR, uuid);
        }
        device.story(HIDDEN_STORIES_DIR, H);
        fs::create_dir_all(root.join(LIBRARY_DIR)).unwrap();
        device.put_index("list", &[A, B, C]);
        device.put_index("list.hidden", &[H]);
        fs::create_dir_all(root.join("usr/0").join(A)).unwrap();
        fs::write(root.join("usr/0").join(format!("{A}.save")), b"save").unwrap();
        fs::write(root.join(LIBRARY_CACHE), b"cache").unwrap();
        device
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn story(&self, dir: &str, uuid: &str) {
        let story = self.root().join(dir).join(uuid);
        fs::create_dir_all(story.join("rf/000")).unwrap();
        fs::write(story.join("ni"), b"ni").unwrap();
        fs::write(story.join("rf/000/ABCD1234"), [1u8; 4096]).unwrap();
    }

    fn put_index(&self, name: &str, uuids: &[&str]) {
        let entries: Vec<StoryId> = uuids.iter().map(|u| id(u)).collect();
        fs::write(
            self.root().join(LIBRARY_DIR).join(name),
            serialize(&entries),
        )
        .unwrap();
    }

    fn index(&self, kind: IndexKind) -> Vec<String> {
        let read = read_index(self.root(), kind);
        assert!(
            matches!(read.state, IndexState::Valid | IndexState::Absent),
            "index {kind:?} : {:?}",
            read.state
        );
        read.entries.iter().map(|e| e.to_string()).collect()
    }

    fn has_story(&self, dir: &str, uuid: &str) -> bool {
        let story = self.root().join(dir).join(uuid);
        story.join("ni").is_file() && story.join("rf/000/ABCD1234").is_file()
    }

    fn trash_entries(&self) -> usize {
        fs::read_dir(self.root().join(TRASH_DIR))
            .map(|it| it.count())
            .unwrap_or(0)
    }

    /// Les autres histoires et les sauvegardes sont intactes.
    fn assert_others_intact(&self, removed: &str) {
        for (dir, uuid) in [
            (STORIES_DIR, A),
            (STORIES_DIR, B),
            (STORIES_DIR, C),
            (HIDDEN_STORIES_DIR, H),
        ] {
            if uuid != removed {
                assert!(self.has_story(dir, uuid), "{uuid} touchée");
            }
        }
        assert!(self.root().join("usr/0").join(A).is_dir());
        assert!(self
            .root()
            .join("usr/0")
            .join(format!("{A}.save"))
            .is_file());
        assert!(self.root().join(MDF_FILE).is_file());
    }
}

fn remove(device: &Device, uuid: &str) -> Result<RemoveReport, RemoveError> {
    remove_story(device.root(), &DeviceLock::new(), uuid, true)
}

#[test]
fn retire_une_histoire_visible() {
    let device = Device::new();
    let report = remove(&device, B).unwrap();
    assert_eq!(
        report,
        RemoveReport {
            id: B.into(),
            index_updated: true,
            folders_removed: 1,
            cleanup_pending: false,
        }
    );
    assert_eq!(device.index(IndexKind::Visible), [A, C]);
    assert_eq!(device.index(IndexKind::Hidden), [H]);
    assert!(!device.root().join(STORIES_DIR).join(B).exists());
    assert!(!device.root().join(TRASH_DIR).exists());
    assert!(!device.root().join(LIBRARY_CACHE).exists());
    device.assert_others_intact(B);
}

#[test]
fn retire_une_histoire_masquee() {
    let device = Device::new();
    let report = remove(&device, H).unwrap();
    assert!(report.index_updated);
    assert_eq!(report.folders_removed, 1);
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
    assert!(device.index(IndexKind::Hidden).is_empty());
    assert!(!device.root().join(HIDDEN_STORIES_DIR).join(H).exists());
    device.assert_others_intact(H);
}

#[test]
fn uuid_en_majuscules_accepte_sous_forme_canonique() {
    let device = Device::new();
    let report = remove(&device, &B.to_uppercase()).unwrap();
    assert_eq!(report.id, B);
    assert_eq!(device.index(IndexKind::Visible), [A, C]);
}

#[test]
fn entree_d_index_sans_dossier_est_retiree() {
    let device = Device::new();
    fs::remove_dir_all(device.root().join(STORIES_DIR).join(C)).unwrap();
    let report = remove(&device, C).unwrap();
    assert!(report.index_updated);
    assert_eq!(report.folders_removed, 0);
    assert_eq!(device.index(IndexKind::Visible), [A, B]);
}

#[test]
fn sans_confirmation_rien_n_est_touche() {
    let device = Device::new();
    let err = remove_story(device.root(), &DeviceLock::new(), B, false).unwrap_err();
    assert!(matches!(err, RemoveError::NotConfirmed));
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
    assert!(device.has_story(STORIES_DIR, B));
    assert!(device.root().join(LIBRARY_CACHE).exists());
}

#[test]
fn uuid_malforme_ou_chemin_ne_supprime_rien() {
    let device = Device::new();
    // Dossiers qu'un chemin construit naïvement pourrait viser.
    fs::create_dir_all(device.root().join("etc/wifi")).unwrap();
    for bad in [
        "",
        "..",
        "../etc",
        "..\\etc",
        "str",
        "etc/wifi",
        "C:\\Windows",
        "/etc",
        "4639e3ea-13b4-4550-91c9-f930a4df6b9",
        "4639e3ea-13b4-4550-91c9-f930a4df6b93/..",
        "4639e3ea-13b4-4550-91c9-f930a4df6b9/",
        "{4639e3ea-13b4-4550-91c9-f930a4df6b93}",
        " 4639e3ea-13b4-4550-91c9-f930a4df6b93",
        "4639e3ea_13b4_4550_91c9_f930a4df6b93",
        "4639e3ea-13b4-4550-91c9-f930a4df6g93",
    ] {
        let err = remove(&device, bad).unwrap_err();
        assert!(matches!(err, RemoveError::InvalidId), "{bad:?} : {err}");
    }
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
    assert_eq!(device.index(IndexKind::Hidden), [H]);
    assert!(device.root().join("etc/wifi").is_dir());
    device.assert_others_intact("");
    assert!(device.root().join(LIBRARY_CACHE).exists());
}

#[test]
fn histoire_inconnue_refusee() {
    let device = Device::new();
    let unknown = "00000000-0000-4000-8000-000000000000";
    let err = remove(&device, unknown).unwrap_err();
    assert!(matches!(err, RemoveError::NotFound(_)));
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
    assert!(device.root().join(LIBRARY_CACHE).exists());
}

#[test]
fn racine_sans_mdf_refusee() {
    let device = Device::new();
    fs::remove_file(device.root().join(MDF_FILE)).unwrap();
    let err = remove(&device, B).unwrap_err();
    assert!(matches!(err, RemoveError::NotADevice));
    assert!(device.has_story(STORIES_DIR, B));
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
}

#[test]
fn appareil_occupe_refuse() {
    let device = Device::new();
    let lock = DeviceLock::new();
    let _transfer = lock.try_begin(Operation::Transfer).unwrap();
    let err = remove_story(device.root(), &lock, B, true).unwrap_err();
    assert!(matches!(err, RemoveError::Busy(Busy(Operation::Transfer))));
    assert!(device.has_story(STORIES_DIR, B));
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
}

#[test]
fn le_verrou_est_libere_apres_le_retrait() {
    let device = Device::new();
    let lock = DeviceLock::new();
    remove_story(device.root(), &lock, B, true).unwrap();
    assert_eq!(lock.current(), None);
    remove_story(device.root(), &lock, "nimporte", true).unwrap_err();
    assert_eq!(lock.current(), None);
}

#[test]
fn index_abime_refuse_sans_rien_supprimer() {
    let device = Device::new();
    let mut bytes = serialize(&[id(A), id(B)]);
    bytes.extend_from_slice(b"pas un uuid\n");
    fs::write(device.root().join(LIBRARY_DIR).join("list"), &bytes).unwrap();
    let err = remove(&device, B).unwrap_err();
    assert!(matches!(err, RemoveError::IndexNeedsUser), "{err}");
    assert_eq!(
        fs::read(device.root().join(LIBRARY_DIR).join("list")).unwrap(),
        bytes
    );
    assert!(device.has_story(STORIES_DIR, B));
}

/// `list.hidden` abîmé alors que `list` est valide : rien n'est retiré (l'appartenance n'est pas
/// sûre).
#[test]
fn list_hidden_abime_refuse_sans_rien_supprimer() {
    let device = Device::new();
    let hidden = device.root().join(LIBRARY_DIR).join("list.hidden");
    fs::write(&hidden, b"\x01\x02 illisible").unwrap();
    let err = remove(&device, B).unwrap_err();
    assert!(matches!(err, RemoveError::IndexNeedsUser), "{err}");
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
    assert!(device.has_story(STORIES_DIR, B));
    assert_eq!(fs::read(&hidden).unwrap(), b"\x01\x02 illisible");
}

/// Une jonction nommée comme une entrée de corbeille n'est jamais suivie : sa cible reste.
#[cfg(windows)]
#[test]
fn jonction_dans_la_corbeille_n_est_pas_suivie() {
    let device = Device::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("precieux"), b"x").unwrap();
    let trash = device.root().join(TRASH_DIR);
    fs::create_dir_all(&trash).unwrap();
    let junction = trash.join(format!("{A}-{}", "0123456789abcdef0123456789abcdef"));
    let made = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction)
        .arg(outside.path())
        .output()
        .is_ok_and(|o| o.status.success());
    if !made {
        eprintln!("jonction impossible : test sans objet");
        return;
    }
    let _ = purge_trash(device.root());
    assert!(outside.path().join("precieux").is_file(), "cible suivie");
}

/// La corbeille créée par un retrait est cachée (Explorateur) sous Windows.
#[cfg(windows)]
#[test]
fn corbeille_cachee_si_la_suppression_echoue() {
    use std::os::windows::fs::MetadataExt;
    let device = Device::new();
    let mut calls = 0;
    // Coupure après le déplacement : la corbeille reste en place.
    let _ = remove_with(device.root(), &DeviceLock::new(), B, true, &mut |step| {
        calls += 1;
        if step == Step::MovedToTrash {
            Err(io::Error::other("coupure simulée"))
        } else {
            Ok(())
        }
    });
    assert!(calls > 0);
    let attributes = fs::metadata(device.root().join(TRASH_DIR))
        .unwrap()
        .file_attributes();
    assert_ne!(attributes & 0x2, 0, "attribut FILE_ATTRIBUTE_HIDDEN absent");
}

#[test]
fn orphelin_signale_puis_supprime_sur_confirmation() {
    let device = Device::new();
    device.put_index("list", &[A, C]);
    assert_eq!(
        find_orphans(device.root()).unwrap(),
        [Orphan {
            id: B.into(),
            hidden: false
        }]
    );
    let err = remove_story(device.root(), &DeviceLock::new(), B, false).unwrap_err();
    assert!(matches!(err, RemoveError::NotConfirmed));
    assert!(device.has_story(STORIES_DIR, B));

    fs::write(device.root().join(LIBRARY_CACHE), b"cache").unwrap();
    let report = remove(&device, B).unwrap();
    assert!(!report.index_updated);
    assert_eq!(report.folders_removed, 1);
    assert!(!device.root().join(STORIES_DIR).join(B).exists());
    assert_eq!(device.index(IndexKind::Visible), [A, C]);
    // Index inchangé : le cache du firmware reste valide.
    assert!(device.root().join(LIBRARY_CACHE).exists());
    assert!(find_orphans(device.root()).unwrap().is_empty());
    device.assert_others_intact(B);
}

#[test]
fn lien_a_la_place_du_dossier_n_est_pas_suivi() {
    let device = Device::new();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("precieux"), b"x").unwrap();
    let link = device.root().join(STORIES_DIR).join(B);
    fs::remove_dir_all(&link).unwrap();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(outside.path(), &link);
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(outside.path(), &link);
    if made.is_err() {
        eprintln!("liens symboliques interdits (mode développeur) : test sans objet");
        return;
    }
    let report = remove(&device, B).unwrap();
    assert_eq!(report.folders_removed, 0);
    assert!(outside.path().join("precieux").is_file());
}

#[test]
fn corbeille_videe_au_debut_d_une_operation() {
    let device = Device::new();
    let trash = device.root().join(TRASH_DIR);
    let leftover = trash.join(format!("{A}-{}", "0123456789abcdef0123456789abcdef"));
    fs::create_dir_all(leftover.join("rf")).unwrap();
    fs::write(leftover.join("rf/x"), b"x").unwrap();
    assert_eq!(purge_trash(device.root()).unwrap(), 1);
    assert!(!trash.exists());
    assert_eq!(purge_trash(device.root()).unwrap(), 0);
}

#[test]
fn entree_inconnue_de_la_corbeille_conservee() {
    let device = Device::new();
    let trash = device.root().join(TRASH_DIR);
    fs::create_dir_all(trash.join("photos")).unwrap();
    fs::write(trash.join(format!("{A}-reste")), b"x").unwrap();
    assert_eq!(purge_trash(device.root()).unwrap(), 0);
    assert!(trash.join("photos").is_dir());
    assert!(trash.join(format!("{A}-reste")).is_file());
}

#[test]
fn corbeille_inutilisable_rien_n_est_modifie() {
    let device = Device::new();
    fs::write(device.root().join(TRASH_DIR), b"fichier").unwrap();
    let err = remove(&device, B).unwrap_err();
    assert!(matches!(err, RemoveError::TrashUnavailable(_)), "{err}");
    assert_eq!(device.index(IndexKind::Visible), [A, B, C]);
    assert!(device.has_story(STORIES_DIR, B));
    assert!(device.root().join(LIBRARY_CACHE).exists());
}

#[test]
fn list_absent_orphelin_refuse() {
    let device = Device::new();
    fs::remove_file(device.root().join(LIBRARY_DIR).join("list")).unwrap();
    assert!(matches!(
        find_orphans(device.root()).unwrap_err(),
        RemoveError::IndexNeedsUser
    ));
    let err = remove(&device, B).unwrap_err();
    assert!(matches!(err, RemoveError::IndexNeedsUser), "{err}");
    assert!(device.has_story(STORIES_DIR, B));
}

#[test]
fn racine_relative_refusee() {
    let err = remove_story(Path::new("."), &DeviceLock::new(), B, true).unwrap_err();
    assert!(matches!(err, RemoveError::NotADevice));
}

#[test]
fn histoire_dans_les_deux_index_et_les_deux_dossiers() {
    let device = Device::new();
    device.put_index("list.hidden", &[H, B]);
    device.story(HIDDEN_STORIES_DIR, B);
    let report = remove(&device, B).unwrap();
    assert_eq!(report.folders_removed, 2);
    assert_eq!(device.index(IndexKind::Visible), [A, C]);
    assert_eq!(device.index(IndexKind::Hidden), [H]);
    assert!(!device.root().join(STORIES_DIR).join(B).exists());
    assert!(!device.root().join(HIDDEN_STORIES_DIR).join(B).exists());
    device.assert_others_intact(B);
}

/// Coupure après chaque étape, puis reprise (nouvelle connexion) : l'index reste valide et
/// ne contient plus l'UUID ou le contient avec son dossier intact ; le dossier est soit
/// entier dans `str/` (orphelin signalé), soit dans la corbeille (vidée) ; les autres
/// histoires sont intactes.
#[test]
fn coupure_a_chaque_etape_puis_reprise() {
    for (uuid, dir) in [(B, STORIES_DIR), (H, HIDDEN_STORIES_DIR)] {
        let mut step_count = 0;
        loop {
            let device = Device::new();
            let calls = Cell::new(0);
            let mut hook = |_step: Step| {
                calls.set(calls.get() + 1);
                if calls.get() > step_count {
                    Err(io::Error::other("coupure"))
                } else {
                    Ok(())
                }
            };
            let lock = DeviceLock::new();
            let result = remove_with(device.root(), &lock, uuid, true, &mut hook);
            let finished = result.is_ok();
            assert_eq!(lock.current(), None);

            // Reprise : nouvelle connexion de l'appareil.
            for kind in IndexKind::ALL {
                recover(device.root(), kind).unwrap();
            }
            purge_trash(device.root()).unwrap();
            let visible = device.index(IndexKind::Visible);
            let hidden = device.index(IndexKind::Hidden);
            let indexed = visible.iter().chain(&hidden).any(|e| e == uuid);
            let folder = device.root().join(dir).join(uuid);
            if indexed {
                // Index jamais orphelin d'un dossier : l'histoire est entière.
                assert!(device.has_story(dir, uuid), "étape {step_count}");
            } else if folder.exists() {
                assert!(device.has_story(dir, uuid), "étape {step_count}");
                let orphans = find_orphans(device.root()).unwrap();
                assert_eq!(orphans.len(), 1, "étape {step_count}");
                assert_eq!(orphans[0].id, uuid);
            }
            assert_eq!(device.trash_entries(), 0, "étape {step_count}");
            device.assert_others_intact(uuid);
            for other in [A, B, C] {
                if other != uuid {
                    assert!(visible.iter().any(|e| e == other), "étape {step_count}");
                }
            }
            if uuid != H {
                assert_eq!(hidden, [H]);
            }

            // Le retrait relancé termine toujours le travail.
            if !finished {
                let present = indexed || folder.exists();
                let again = remove(&device, uuid);
                if present {
                    again.unwrap();
                } else {
                    assert!(matches!(again.unwrap_err(), RemoveError::NotFound(_)));
                }
                assert!(!folder.exists());
                assert!(!device.index(IndexKind::Visible).iter().any(|e| e == uuid));
                assert!(!device.index(IndexKind::Hidden).iter().any(|e| e == uuid));
                device.assert_others_intact(uuid);
            }
            if finished {
                break;
            }
            step_count += 1;
        }
        assert!(step_count >= 3, "{uuid} : {step_count} étapes");
    }
}

/// Suppression interrompue au milieu de `remove_dir_all` : les restes partiels sont dans la
/// corbeille, jamais dans `str/`, et sont vidés à la reprise.
#[test]
fn suppression_partielle_reprise_depuis_la_corbeille() {
    let device = Device::new();
    let lock = DeviceLock::new();
    let mut hook = |step: Step| {
        if step == Step::MovedToTrash {
            // Simule une suppression commencée : un fichier retiré, puis coupure.
            let entry = fs::read_dir(device.root().join(TRASH_DIR))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            fs::remove_file(entry.join("ni")).unwrap();
            return Err(io::Error::other("coupure"));
        }
        Ok(())
    };
    remove_with(device.root(), &lock, B, true, &mut hook).unwrap_err();
    assert!(!device.root().join(STORIES_DIR).join(B).exists());
    assert_eq!(device.trash_entries(), 1);
    assert!(find_orphans(device.root()).unwrap().is_empty());
    // L'opération suivante vide la corbeille.
    remove(&device, C).unwrap();
    assert_eq!(device.trash_entries(), 0);
    assert_eq!(device.index(IndexKind::Visible), [A]);
}

/// Dossier verrouillé (renommage impossible) : l'index est réécrit, le dossier reste entier
/// et devient un orphelin signalé.
#[cfg(windows)]
#[test]
fn dossier_verrouille_devient_orphelin() {
    use std::os::windows::fs::OpenOptionsExt;
    let device = Device::new();
    // Fichier ouvert sans partage de suppression : le dossier parent ne peut être renommé.
    let _open = fs::OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(device.root().join(STORIES_DIR).join(B).join("ni"))
        .unwrap();
    let err = remove(&device, B).unwrap_err();
    assert!(matches!(err, RemoveError::FolderLeft(_)), "{err}");
    assert_eq!(device.index(IndexKind::Visible), [A, C]);
    assert!(device.has_story(STORIES_DIR, B));
    assert_eq!(
        find_orphans(device.root()).unwrap(),
        [Orphan {
            id: B.into(),
            hidden: false
        }]
    );
}
