//! Tests sur faux appareil : validation de l'ordre demandé, écriture et relecture, refus sans
//! écriture, coupure simulée à chaque étape de l'écriture de l'index puis reprise.

use std::collections::BTreeMap;
use std::path::PathBuf;

use super::*;
use crate::device::index::{self, serialize, Step, LIBRARY_CACHE, LIBRARY_DIR, STORIES_DIR};

const A: &str = "4639e3ea-13b4-4550-91c9-f930a4df6b93";
const B: &str = "5fb70290-6246-419e-a764-bca1635fc850";
const C: &str = "76b88912-f3dc-41f4-b4a7-6aa809d57ad7";
const D: &str = "0b2c1d3e-4f5a-4b6c-8d7e-9f0a1b2c3d4e";
const H: &str = "9a8521bf-0ae0-40de-8f8b-c22376bad8fd";

fn id(text: &str) -> StoryId {
    StoryId::parse(text).unwrap()
}

fn order(uuids: &[&str]) -> Vec<String> {
    uuids.iter().map(|u| u.to_string()).collect()
}

fn lines(uuids: &[&str]) -> Vec<u8> {
    serialize(&uuids.iter().map(|u| id(u)).collect::<Vec<_>>())
}

/// Faux appareil : `.mdf`, histoires A, B, C, D visibles (dans cet ordre) et H masquée, avec
/// fichiers, sauvegardes `usr/0/`, réglages et cache du firmware.
struct Device {
    dir: tempfile::TempDir,
}

impl Device {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let device = Self { dir };
        let root = device.root();
        fs::write(root.join(MDF_FILE), [7u8; 126]).unwrap();
        for uuid in [A, B, C, D] {
            device.story(STORIES_DIR, uuid);
        }
        device.story("str.hidden", H);
        fs::create_dir_all(root.join(LIBRARY_DIR)).unwrap();
        device.put("list", &lines(&[A, B, C, D]));
        device.put("list.hidden", &lines(&[H]));
        fs::create_dir_all(root.join("usr/0").join(A)).unwrap();
        fs::write(root.join("usr/0").join(format!("{A}.save")), b"save").unwrap();
        fs::create_dir_all(root.join("etc/wifi")).unwrap();
        fs::write(root.join("etc/wifi/conf"), b"wifi").unwrap();
        fs::write(root.join(LIBRARY_CACHE), b"cache").unwrap();
        device
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn story(&self, dir: &str, uuid: &str) {
        let story = self.root().join(dir).join(uuid);
        fs::create_dir_all(story.join("img")).unwrap();
        fs::write(story.join("info"), b"info").unwrap();
        fs::write(story.join("img/thumbnail.lif"), [1u8; 512]).unwrap();
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

    /// Toute l'arborescence (chemin relatif → contenu, `None` pour un dossier), sauf `list`, ses
    /// fichiers de travail et `library.cache`, seuls fichiers qu'un réordonnancement peut toucher.
    fn others(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
            for entry in fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if path.is_dir() {
                    out.insert(rel, None);
                    walk(&path, root, out);
                } else {
                    out.insert(rel, Some(fs::read(&path).unwrap()));
                }
            }
        }
        let mut all = BTreeMap::new();
        walk(self.root(), self.root(), &mut all);
        all.retain(|rel, _| {
            !matches!(
                rel.as_str(),
                "etc/library/list"
                    | "etc/library/list.new"
                    | "etc/library/list.bak"
                    | "etc/library/list.restore"
                    | "usr/0/library.cache"
            )
        });
        all
    }

    fn assert_no_work_files(&self) {
        for name in ["list.new", "list.bak", "list.restore"] {
            assert!(self.get(name).is_none(), "{name} doit avoir disparu");
        }
    }
}

fn reorder(dev: &Device, uuids: &[&str]) -> Result<ReorderReport, ReorderError> {
    reorder_stories(dev.root(), &DeviceLock::new(), &order(uuids))
}

/// Refus sans aucune écriture : arborescence entière identique, cache compris.
fn assert_refused_untouched(dev: &Device, uuids: &[&str]) -> ReorderError {
    let list = dev.get("list");
    let others = dev.others();
    let err = reorder(dev, uuids).unwrap_err();
    assert_eq!(dev.get("list"), list, "list modifié par un refus ({err})");
    assert_eq!(
        dev.others(),
        others,
        "appareil modifié par un refus ({err})"
    );
    assert!(dev.cache_present(), "cache supprimé par un refus ({err})");
    err
}

// ---------------------------------------------------------------- écriture

#[test]
fn ordre_ecrit_relu_et_cache_supprime() {
    let dev = Device::new();
    let others = dev.others();
    let report = reorder(&dev, &[C, A, D, B]).unwrap();
    assert_eq!(
        report,
        ReorderReport {
            changed: true,
            cache_removed: true,
            count: 4,
        }
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[C, A, D, B]));
    let read = read_index(dev.root(), IndexKind::Visible);
    assert_eq!(read.state, IndexState::Valid);
    assert_eq!(read.entries, [C, A, D, B].map(id));
    assert!(!dev.cache_present());
    dev.assert_no_work_files();
    // Aucune autre écriture : histoires, list.hidden, sauvegardes, réglages identiques.
    assert_eq!(dev.others(), others);
}

#[test]
fn ordre_identique_n_ecrit_rien() {
    let dev = Device::new();
    let others = dev.others();
    let report = reorder(&dev, &[A, B, C, D]).unwrap();
    assert!(!report.changed);
    assert!(!report.cache_removed);
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B, C, D]));
    assert!(dev.cache_present());
    assert_eq!(dev.others(), others);
}

#[test]
fn majuscules_acceptees_et_ecrites_en_minuscules() {
    let dev = Device::new();
    let upper = B.to_ascii_uppercase();
    reorder(&dev, &[&upper, A, C, D]).unwrap();
    assert_eq!(dev.get("list").unwrap(), lines(&[B, A, C, D]));
}

#[test]
fn histoire_listee_sans_dossier_garde_sa_place() {
    let dev = Device::new();
    fs::remove_dir_all(dev.root().join(STORIES_DIR).join(D)).unwrap();
    reorder(&dev, &[D, C, B, A]).unwrap();
    assert_eq!(dev.get("list").unwrap(), lines(&[D, C, B, A]));
}

#[test]
fn list_hidden_absent_accepte_et_non_cree() {
    let dev = Device::new();
    fs::remove_file(dev.path("list.hidden")).unwrap();
    reorder(&dev, &[B, A, C, D]).unwrap();
    assert_eq!(dev.get("list").unwrap(), lines(&[B, A, C, D]));
    assert!(dev.get("list.hidden").is_none());
}

#[test]
fn reprend_une_ecriture_interrompue_avant_de_valider() {
    // Écriture précédente coupée après la sauvegarde : `list` est l'ancien ordre.
    let dev = Device::new();
    dev.put("list.new", &lines(&[D, C, B, A]));
    dev.put("list.bak", &lines(&[A, B, C, D]));
    reorder(&dev, &[B, A, C, D]).unwrap();
    assert_eq!(dev.get("list").unwrap(), lines(&[B, A, C, D]));
    dev.assert_no_work_files();
}

// ---------------------------------------------------------------- validation

#[test]
fn ordre_vide_refuse() {
    let dev = Device::new();
    let err = assert_refused_untouched(&dev, &[]);
    assert!(matches!(err, ReorderError::Empty), "{err}");
}

#[test]
fn uuid_invalide_refuse_avec_sa_position() {
    let dev = Device::new();
    for bad in ["", "pas-un-uuid", "../etc", &A[..35], &format!("{{{A}}}")] {
        let err = assert_refused_untouched(&dev, &[A, B, bad, D]);
        assert!(matches!(err, ReorderError::InvalidId(3)), "{bad} : {err}");
    }
}

#[test]
fn doublon_refuse() {
    let dev = Device::new();
    let err = assert_refused_untouched(&dev, &[A, B, C, D, B]);
    assert!(
        matches!(&err, ReorderError::Duplicate(d) if *d == id(B)),
        "{err}"
    );
    // Doublon à la casse près.
    let upper = A.to_ascii_uppercase();
    let err = assert_refused_untouched(&dev, &[A, &upper, B, C, D]);
    assert!(
        matches!(&err, ReorderError::Duplicate(d) if *d == id(A)),
        "{err}"
    );
}

#[test]
fn uuid_inconnu_refuse() {
    let dev = Device::new();
    let unknown = "11111111-2222-4333-8444-555555555555";
    let err = assert_refused_untouched(&dev, &[A, B, C, D, unknown]);
    assert!(
        matches!(&err, ReorderError::Unknown(u) if *u == id(unknown)),
        "{err}"
    );
    // Même nombre d'histoires, mais une remplacée.
    let err = assert_refused_untouched(&dev, &[A, B, C, unknown]);
    assert!(
        matches!(&err, ReorderError::Unknown(u) if *u == id(unknown)),
        "{err}"
    );
}

#[test]
fn histoire_absente_de_la_demande_refusee() {
    let dev = Device::new();
    let err = assert_refused_untouched(&dev, &[D, C, A]);
    assert!(
        matches!(&err, ReorderError::Missing(m) if *m == id(B)),
        "{err}"
    );
}

#[test]
fn histoire_masquee_refusee_et_list_hidden_inchange() {
    let dev = Device::new();
    let err = assert_refused_untouched(&dev, &[A, B, C, D, H]);
    assert!(
        matches!(&err, ReorderError::Hidden(h) if *h == id(H)),
        "{err}"
    );
    let err = assert_refused_untouched(&dev, &[H, B, C, D]);
    assert!(
        matches!(&err, ReorderError::Hidden(h) if *h == id(H)),
        "{err}"
    );
}

#[test]
fn index_non_sur_refuse() {
    let damaged = {
        let mut bytes = lines(&[A, B, C, D]);
        bytes.extend_from_slice(b"ligne invalide\n");
        bytes
    };
    let cases: [(&str, Option<&[u8]>); 5] = [
        ("list", None),
        ("list", Some(&damaged)),
        ("list", Some(b"\0\0\0")),
        ("list.hidden", Some(b"ligne invalide\n")),
        ("list.hidden", Some(b"\0")),
    ];
    for (name, content) in cases {
        let dev = Device::new();
        match content {
            Some(bytes) => dev.put(name, bytes),
            None => fs::remove_file(dev.path(name)).unwrap(),
        }
        let err = assert_refused_untouched(&dev, &[B, A, C, D]);
        assert!(
            matches!(err, ReorderError::IndexNeedsUser),
            "{name} {content:?} : {err}"
        );
    }
}

#[test]
fn reprise_sans_candidat_refusee_sans_rien_toucher() {
    let dev = Device::new();
    // `bak` illisible et `list` abîmé : la reprise demande l'utilisateur.
    dev.put("list", b"illisible\n");
    dev.put("list.bak", b"\0");
    let err = assert_refused_untouched(&dev, &[B, A, C, D]);
    assert!(matches!(err, ReorderError::IndexNeedsUser), "{err}");
    assert_eq!(dev.get("list.bak").unwrap(), b"\0");
}

#[test]
fn pas_une_flam_refuse() {
    let dev = Device::new();
    fs::remove_file(dev.root().join(MDF_FILE)).unwrap();
    let err = reorder(&dev, &[B, A, C, D]).unwrap_err();
    assert!(matches!(err, ReorderError::NotADevice), "{err}");
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B, C, D]));
    let relative = reorder_stories(Path::new("relatif"), &DeviceLock::new(), &order(&[A]));
    assert!(matches!(relative, Err(ReorderError::NotADevice)));
}

#[test]
fn verrou_occupe_refuse_sans_rien_lire_ni_ecrire() {
    let dev = Device::new();
    // Écriture interrompue laissée en place : même la reprise n'est pas lancée.
    dev.put("list.new", &lines(&[D, C, B, A]));
    let others = dev.others();
    let lock = DeviceLock::new();
    let _transfer = lock.try_begin(Operation::Transfer).unwrap();
    let err = reorder_stories(dev.root(), &lock, &order(&[B, A, C, D])).unwrap_err();
    assert!(
        matches!(err, ReorderError::Busy(Busy(Operation::Transfer))),
        "{err}"
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B, C, D]));
    assert_eq!(dev.get("list.new").unwrap(), lines(&[D, C, B, A]));
    assert_eq!(dev.others(), others);
    assert!(dev.cache_present());
}

#[test]
fn verrou_tenu_pendant_l_ecriture_puis_libere() {
    let dev = Device::new();
    let lock = DeviceLock::new();
    let mut seen = None;
    reorder_with(
        dev.root(),
        &lock,
        &order(&[B, A, C, D]),
        &mut |root, entries| {
            seen = lock.current();
            write_index(root, IndexKind::Visible, entries, Consent::IfValid)
        },
    )
    .unwrap();
    assert_eq!(seen, Some(Operation::Reorder));
    assert_eq!(lock.current(), None);
}

#[test]
fn cache_non_supprime_signale_et_repris() {
    let dev = Device::new();
    let report = reorder_with(
        dev.root(),
        &DeviceLock::new(),
        &order(&[B, A, C, D]),
        &mut |root, entries| {
            // Index écrit, puis échec de la suppression du cache (fichier verrouillé).
            index::write_with(
                root,
                IndexKind::Visible,
                entries,
                Consent::IfValid,
                &mut |s| {
                    if s == Step::Verified {
                        Err(io::Error::other("cache verrouillé"))
                    } else {
                        Ok(())
                    }
                },
            )
            .map_err(|_| IndexError::CacheNotRemoved(io::Error::other("cache verrouillé")))
        },
    )
    .unwrap();
    assert!(report.changed);
    assert!(!report.cache_removed);
    assert_eq!(dev.get("list").unwrap(), lines(&[B, A, C, D]));
    // La reprise suivante termine la transaction.
    assert_eq!(
        recover(dev.root(), IndexKind::Visible).unwrap(),
        Recovery::RolledForward
    );
    assert!(!dev.cache_present());
    dev.assert_no_work_files();
}

// ---------------------------------------------------------------- coupures

const STEPS: [Step; 5] = [
    Step::NewWritten,
    Step::BackupWritten,
    Step::Replaced,
    Step::Verified,
    Step::CacheRemoved,
];

/// Dégât supplémentaire laissé par la coupure (écriture FAT partielle).
#[derive(Debug, Clone, Copy)]
enum Torn {
    /// Aucun : coupure juste après l'étape.
    None,
    /// Coupure pendant l'écriture de `list.new` (contenu tronqué, pas de `bak`).
    PartialNew,
    /// Coupure pendant l'écriture de `list.bak` (contenu tronqué).
    PartialBak,
    /// Coupure pendant le renommage : la cible `list` a disparu, `list.new` reste.
    LostTarget,
}

/// Réordonne de `[A, B, C, D]` vers `[D, B, A, C]`, coupe après `step` (sans aucun nettoyage,
/// comme un débranchement), applique `torn`, puis redémarre (reprise). L'index doit être
/// exactement l'ancien ou le nouvel ordre, sans fichier de travail, le reste de l'appareil
/// intact ; une nouvelle demande aboutit ensuite.
fn cut_then_recover(step: Step, torn: Torn) {
    let old = [A, B, C, D];
    let new = [D, B, A, C];
    let dev = Device::new();
    let others = dev.others();
    let cut = |s: Step| {
        if s == step {
            Err(io::Error::other("coupure simulée"))
        } else {
            Ok(())
        }
    };
    let err = reorder_with(
        dev.root(),
        &DeviceLock::new(),
        &order(&new),
        &mut |root, entries| {
            index::write_with(
                root,
                IndexKind::Visible,
                entries,
                Consent::IfValid,
                &mut |s| cut(s),
            )
        },
    )
    .unwrap_err();
    assert!(matches!(err, ReorderError::Index(_)), "{step:?} : {err}");

    match torn {
        Torn::None => {}
        Torn::PartialNew => {
            let new_bytes = dev.get("list.new").unwrap();
            dev.put("list.new", &new_bytes[..new_bytes.len() / 2]);
        }
        Torn::PartialBak => {
            let bak = dev.get("list.bak").unwrap();
            dev.put("list.bak", &bak[..bak.len() / 2 + 3]);
        }
        Torn::LostTarget => fs::remove_file(dev.path("list")).unwrap(),
    }

    // Redémarrage : reprise des index (branchement suivant).
    for kind in IndexKind::ALL {
        let outcome = recover(dev.root(), kind).unwrap();
        assert_ne!(outcome, Recovery::NeedsUser, "{step:?} {torn:?}");
    }
    let list = dev.get("list").expect("list présent après reprise");
    let replaced = matches!(step, Step::Replaced | Step::Verified | Step::CacheRemoved)
        || matches!(torn, Torn::LostTarget);
    let expected = if replaced { lines(&new) } else { lines(&old) };
    assert_eq!(list, expected, "{step:?} {torn:?}");
    assert_eq!(
        read_index(dev.root(), IndexKind::Visible).state,
        IndexState::Valid
    );
    dev.assert_no_work_files();
    assert_eq!(dev.others(), others, "{step:?} {torn:?} : appareil modifié");
    if replaced {
        assert!(!dev.cache_present(), "{step:?} {torn:?} : cache périmé");
    }

    // Une nouvelle demande aboutit.
    let current: Vec<&str> = if replaced { new.to_vec() } else { old.to_vec() };
    let mut target = current.clone();
    target.reverse();
    reorder(&dev, &target).unwrap();
    assert_eq!(dev.get("list").unwrap(), lines(&target));
    dev.assert_no_work_files();
    assert_eq!(dev.others(), others);
}

#[test]
fn coupure_apres_chaque_etape_puis_reprise() {
    for step in STEPS {
        cut_then_recover(step, Torn::None);
    }
}

#[test]
fn coupure_pendant_l_ecriture_de_list_new() {
    cut_then_recover(Step::NewWritten, Torn::PartialNew);
}

#[test]
fn coupure_pendant_l_ecriture_de_list_bak() {
    cut_then_recover(Step::BackupWritten, Torn::PartialBak);
}

#[test]
fn coupure_pendant_le_renommage_cible_perdue() {
    cut_then_recover(Step::BackupWritten, Torn::LostTarget);
}

#[test]
fn echec_d_ecriture_reel_laisse_l_ancien_ordre() {
    // `list.new` occupé par un dossier : l'écriture échoue dès l'étape 1 (sans coupure),
    // `write_index` reprend aussitôt ; l'ancien ordre reste.
    let dev = Device::new();
    fs::create_dir_all(dev.path("list.new")).unwrap();
    let err = reorder(&dev, &[B, A, C, D]).unwrap_err();
    assert!(
        matches!(err, ReorderError::Index(_) | ReorderError::IndexNeedsUser),
        "{err}"
    );
    assert_eq!(dev.get("list").unwrap(), lines(&[A, B, C, D]));
    assert!(dev.cache_present());
}
