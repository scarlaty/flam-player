//! Tests du transfert sur faux appareil (dossier temporaire) avec injection de panne.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Write;

use super::*;
use crate::device::crypto::tests::{test_keys, TEST_IV, TEST_KEY};
use crate::device::index::serialize;

const NEW: &str = "0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9";
const OTHER: &str = "11111111-2222-3333-4444-555555555555";
const BIG: usize = 2 * CHUNK + 12_345;
const PLENTY: u64 = 1 << 40;

fn id(s: &str) -> StoryId {
    StoryId::parse(s).unwrap()
}

/// Arborescence complète (fichiers et dossiers vides) avec contenu, relative à `root`.
fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, Option<Vec<u8>>>) {
        for e in std::fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if p.is_dir() {
                out.insert(format!("{rel}/"), None);
                walk(root, &p, out);
            } else {
                out.insert(rel, Some(std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    if root.exists() {
        walk(root, root, &mut out);
    }
    out
}

/// Contenu des fichiers d'un dossier (relatif, `/`).
fn files_of(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    snapshot(dir)
        .into_iter()
        .filter_map(|(k, v)| v.map(|v| (k, v)))
        .collect()
}

fn write(path: &Path, data: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, data).unwrap();
}

/// Histoire préparée (hors appareil) : petits fichiers et un gros sur plusieurs blocs.
fn prepared(base: &Path, seed: u8) -> PreparedStory {
    let dir = base.join("travail").join("str").join(NEW);
    let mut files = BTreeMap::new();
    files.insert("info", vec![seed; 32]);
    files.insert("key", vec![seed.wrapping_add(1); 32]);
    files.insert("main.lsf", vec![seed.wrapping_add(2); 48]);
    files.insert("version", b"1".to_vec());
    files.insert("img/thumbnail.lif", vec![seed.wrapping_add(3); 700]);
    files.insert(
        "sounds/a.mp3",
        (0..BIG).map(|i| (i % 251) as u8 ^ seed).collect(),
    );
    files.insert("vide", Vec::new());
    for (name, data) in &files {
        write(&dir.join(name), data);
    }
    let total_bytes = files.values().map(|d| d.len() as u64).sum();
    PreparedStory {
        uuid: NEW.to_string(),
        dir,
        files: files.keys().map(|k| k.to_string()).collect(),
        total_bytes,
    }
}

/// Faux appareil : `.mdf`, une autre histoire listée, cache du firmware ; `old` : ancienne
/// version de `NEW` listée.
fn device(base: &Path, old: bool) -> PathBuf {
    let root = base.join("flam");
    write(&root.join(MDF_FILE), &[0u8; 126]);
    write(&root.join("str").join(OTHER).join("info"), b"autre");
    write(&root.join(LIBRARY_CACHE), b"cache");
    let mut list = vec![id(OTHER)];
    if old {
        write(&root.join("str").join(NEW).join("info"), b"ancienne");
        write(
            &root.join("str").join(NEW).join("sounds/ancien.mp3"),
            &[5u8; 5000],
        );
        list.push(id(NEW));
    }
    write(&root.join("etc/library/list"), &serialize(&list));
    write(&root.join("etc/library/list.hidden"), b"");
    root
}

/// Opérations de l'appareil avec pannes injectées. À partir de l'opération `crash_at`,
/// toutes échouent : l'appareil a disparu (débranchement, coupure).
struct FaultyFs {
    free: u64,
    ops: Cell<usize>,
    crash_at: usize,
    /// Octets encore inscriptibles avant « disque plein ».
    budget: Cell<Option<u64>>,
    /// Fichier dont le premier octet écrit est altéré.
    corrupt: Option<&'static str>,
    /// Réponse forcée de `same_directory`.
    same_dir: bool,
}

impl FaultyFs {
    fn new() -> Self {
        Self {
            free: PLENTY,
            ops: Cell::new(0),
            crash_at: usize::MAX,
            budget: Cell::new(None),
            corrupt: None,
            same_dir: false,
        }
    }
    fn crash_at(n: usize) -> Self {
        Self {
            crash_at: n,
            ..Self::new()
        }
    }
    fn tick(&self) -> io::Result<()> {
        let n = self.ops.get();
        self.ops.set(n + 1);
        if n >= self.crash_at {
            Err(io::Error::other("coupure simulée"))
        } else {
            Ok(())
        }
    }
    fn real(&self) -> RealFs {
        RealFs {
            free_space: |_| Some(PLENTY),
        }
    }
}

struct FaultyWriter<'a> {
    fs: &'a FaultyFs,
    inner: File,
    corrupt_next: bool,
}

impl Write for FaultyWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.fs.tick()?;
        if let Some(left) = self.fs.budget.get() {
            if buf.len() as u64 > left {
                return Err(io::Error::new(io::ErrorKind::StorageFull, "disque plein"));
            }
            self.fs.budget.set(Some(left - buf.len() as u64));
        }
        if self.corrupt_next && !buf.is_empty() {
            self.corrupt_next = false;
            let mut altered = buf.to_vec();
            altered[0] ^= 0xff;
            self.inner.write_all(&altered)?;
            return Ok(buf.len());
        }
        self.inner.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.fs.tick()?;
        self.inner.flush()
    }
}

impl SyncWrite for FaultyWriter<'_> {
    fn sync(&mut self) -> io::Result<()> {
        self.fs.tick()?;
        self.inner.sync_all()
    }
}

struct FaultyReader<'a> {
    fs: &'a FaultyFs,
    inner: Box<dyn Read + 'a>,
}

impl Read for FaultyReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.fs.tick()?;
        self.inner.read(buf)
    }
}

impl DeviceFs for FaultyFs {
    fn free_space(&self, _root: &Path) -> io::Result<u64> {
        self.tick()?;
        Ok(self.free)
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        self.real().create_dir_all(path)
    }
    fn create_file(&self, path: &Path) -> io::Result<Box<dyn SyncWrite + '_>> {
        self.tick()?;
        let corrupt_next = self
            .corrupt
            .is_some_and(|name| path.to_string_lossy().replace('\\', "/").ends_with(name));
        Ok(Box::new(FaultyWriter {
            fs: self,
            inner: File::create_new(path)?,
            corrupt_next,
        }))
    }
    fn open_verify(&self, path: &Path) -> io::Result<Box<dyn Read + '_>> {
        self.tick()?;
        let inner = Box::new(File::open(path)?);
        Ok(Box::new(FaultyReader { fs: self, inner }))
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.tick()?;
        self.real().rename(from, to)
    }
    fn remove_dir_all(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        self.real().remove_dir_all(path)
    }
    fn remove_empty_dir(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        self.real().remove_empty_dir(path)
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.tick()?;
        self.real().remove_file(path)
    }
    fn same_directory(&self, a: &Path, b: &Path) -> io::Result<bool> {
        self.tick()?;
        if self.same_dir {
            return Ok(true);
        }
        self.real().same_directory(a, b)
    }
    fn write_index(&self, root: &Path, entries: &[StoryId]) -> Result<WriteOutcome, IndexError> {
        self.tick().map_err(|source| IndexError::Io {
            file: "list".into(),
            source,
        })?;
        self.real().write_index(root, entries)
    }
}

fn send(
    fs: &dyn DeviceFs,
    root: &Path,
    p: &PreparedStory,
    replace: bool,
) -> Result<TransferReport, TransferError> {
    send_with(fs, root, p, replace, &AtomicBool::new(false), &mut |_| {})
}

/// Redémarrage : reprise des transferts puis des index, avec un appareil sain.
fn restart(root: &Path) {
    recover_with(&FaultyFs::new(), root).unwrap();
    for kind in IndexKind::ALL {
        index::recover(root, kind).unwrap();
    }
}

fn listed(root: &Path) -> Vec<StoryId> {
    let read = read_index(root, IndexKind::Visible);
    assert_eq!(read.state, IndexState::Valid);
    read.entries
}

/// Invariants après reprise : aucun résidu, aucun index vers un dossier absent ou incomplet,
/// l'ancienne version (si `old`) ou la nouvelle en place, l'autre histoire intacte.
fn check_invariants(
    root: &Path,
    new: &BTreeMap<String, Vec<u8>>,
    old: Option<&BTreeMap<String, Vec<u8>>>,
    step: usize,
) {
    assert!(!root.join(TRANSFER_DIR).exists(), "résidu (étape {step})");
    let entries = listed(root);
    assert!(
        index::missing_stories(root, &entries).is_empty(),
        "index vers du vide (étape {step})"
    );
    assert_eq!(
        files_of(&root.join("str").join(OTHER)).len(),
        1,
        "étape {step}"
    );
    let target = root.join("str").join(NEW);
    match old {
        Some(old) => {
            let now = files_of(&target);
            assert!(
                now == *old || now == *new,
                "version mêlée ou perdue (étape {step})"
            );
            assert!(entries.contains(&id(NEW)), "étape {step}");
        }
        None => {
            if target.exists() {
                assert_eq!(files_of(&target), *new, "dossier incomplet (étape {step})");
            }
        }
    }
}

#[test]
fn chemin_nominal_de_bout_en_bout() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let p = prepared(tmp.path(), 1);
    let mut events = Vec::new();
    let report = send_with(
        &FaultyFs::new(),
        &root,
        &p,
        false,
        &AtomicBool::new(false),
        &mut |e| events.push(e),
    )
    .unwrap();
    assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
    assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
    assert!(!root.join(LIBRARY_CACHE).exists());
    assert!(!root.join(TRANSFER_DIR).exists());
    assert_eq!(
        report,
        TransferReport {
            id: NEW.into(),
            bytes: p.total_bytes,
            files: p.files.len(),
            replaced: false,
            index_updated: true,
            cache_removed: true,
        }
    );

    // Progression monotone, cohérente avec la taille, copie puis vérification.
    let total = 2 * p.total_bytes;
    assert!(events.iter().all(|e| e.total == total && e.done <= total));
    assert!(events.windows(2).all(|w| w[0].done <= w[1].done));
    assert_eq!(events.first().unwrap().done, 0);
    assert_eq!(events.last().unwrap().done, total);
    let first_verify = events
        .iter()
        .position(|e| e.phase == Phase::Verify)
        .unwrap();
    assert!(events[..first_verify]
        .iter()
        .all(|e| e.phase == Phase::Copy));
    assert!(events[first_verify..]
        .iter()
        .all(|e| e.phase == Phase::Verify));
    assert_eq!(events[first_verify - 1].done, p.total_bytes);
}

#[test]
fn mise_a_jour_d_une_histoire_existante() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), true);
    let p = prepared(tmp.path(), 2);
    let before = std::fs::read(root.join("etc/library/list")).unwrap();
    let err = send(&FaultyFs::new(), &root, &p, false).unwrap_err();
    assert!(
        matches!(err, TransferError::AlreadyInstalled { hidden: false, .. }),
        "{err:?}"
    );
    let report = send(&FaultyFs::new(), &root, &p, true).unwrap();
    assert!(report.replaced && !report.index_updated && report.cache_removed);
    assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
    assert_eq!(
        std::fs::read(root.join("etc/library/list")).unwrap(),
        before
    );
    assert!(!root.join(TRANSFER_DIR).exists());
}

#[test]
fn coupure_a_chaque_etape_installation_et_mise_a_jour() {
    for update in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let p = prepared(tmp.path(), 3);
        let new = files_of(&p.dir);
        // Nombre d'opérations d'un transfert réussi.
        let counter = FaultyFs::new();
        send(
            &counter,
            &device(&tmp.path().join("compte"), update),
            &p,
            true,
        )
        .unwrap();
        let total_ops = counter.ops.get();
        assert!(total_ops > 20, "{total_ops}");

        for crash_at in 0..=total_ops {
            let base = tmp.path().join(format!("c{crash_at}"));
            let root = device(&base, update);
            let old = update.then(|| files_of(&root.join("str").join(NEW)));
            let result = send(&FaultyFs::crash_at(crash_at), &root, &p, true);
            if crash_at < total_ops {
                // Les dernières opérations (cache, ancienne version, dossier vide) sont faites
                // au mieux : leur échec ne fait pas échouer le transfert.
                if let Ok(report) = &result {
                    assert!(crash_at + 4 >= total_ops, "étape {crash_at} : {report:?}");
                }
            } else {
                result.unwrap();
            }
            restart(&root);
            check_invariants(&root, &new, old.as_ref(), crash_at);
            // Un nouvel envoi aboutit toujours.
            send(&FaultyFs::new(), &root, &p, true).unwrap();
            assert_eq!(files_of(&root.join("str").join(NEW)), new);
            let entries = listed(&root);
            assert_eq!(entries.iter().filter(|e| **e == id(NEW)).count(), 1);
            assert!(!root.join(TRANSFER_DIR).exists());
            std::fs::remove_dir_all(&base).unwrap();
        }
    }
}

#[test]
fn espace_insuffisant_rien_n_est_ecrit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let p = prepared(tmp.path(), 4);
    let before = snapshot(&root);
    let fs = FaultyFs {
        free: SPACE_MARGIN + p.total_bytes,
        ..FaultyFs::new()
    };
    match send(&fs, &root, &p, false) {
        Err(TransferError::NotEnoughSpace { needed, free }) => {
            assert!(needed > free);
            assert_eq!(free, SPACE_MARGIN + p.total_bytes);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(snapshot(&root), before);
}

#[test]
fn disque_plein_pendant_la_copie() {
    for update in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), update);
        let p = prepared(tmp.path(), 5);
        let before = snapshot(&root);
        let fs = FaultyFs::new();
        fs.budget.set(Some(CHUNK as u64 + 10));
        let err = send(&fs, &root, &p, true).unwrap_err();
        assert!(
            matches!(err, TransferError::NotEnoughSpace { .. }),
            "{err:?}"
        );
        assert_eq!(snapshot(&root), before);
    }
}

#[test]
fn une_corruption_est_detectee() {
    let tmp = tempfile::tempdir().unwrap();
    for (n, name) in ["sounds/a.mp3", "info"].into_iter().enumerate() {
        let root = device(&tmp.path().join(n.to_string()), true);
        let p = prepared(&tmp.path().join(n.to_string()), 6);
        let before = snapshot(&root);
        let fs = FaultyFs {
            corrupt: Some(name),
            ..FaultyFs::new()
        };
        match send(&fs, &root, &p, true) {
            Err(TransferError::Verify(file)) => assert_eq!(file, name),
            other => panic!("{other:?}"),
        }
        assert_eq!(snapshot(&root), before);
    }
}

/// Moment d'annulation : `(événement, total)` -> annuler.
type Moment = fn(&Progress, u64) -> bool;

#[test]
fn annulation_a_trois_moments() {
    let moments: [(&str, Moment); 3] = [
        ("pendant la copie", |e, _| {
            e.phase == Phase::Copy && e.done > CHUNK as u64
        }),
        ("pendant la vérification", |e, _| e.phase == Phase::Verify),
        ("avant le renommage", |e, total| e.done == total),
    ];
    for update in [false, true] {
        for (what, when) in moments {
            let tmp = tempfile::tempdir().unwrap();
            let root = device(tmp.path(), update);
            let p = prepared(tmp.path(), 7);
            let before = snapshot(&root);
            let cancel = AtomicBool::new(false);
            let total = 2 * p.total_bytes;
            let result = send_with(&FaultyFs::new(), &root, &p, true, &cancel, &mut |e| {
                if when(&e, total) {
                    cancel.store(true, Ordering::Relaxed);
                }
            });
            assert!(
                matches!(result, Err(TransferError::Cancelled)),
                "{what} : {result:?}"
            );
            assert_eq!(snapshot(&root), before, "{what}");
        }
    }
}

#[test]
fn deja_present_ou_masque() {
    let tmp = tempfile::tempdir().unwrap();
    let p = prepared(tmp.path(), 8);

    let root = device(&tmp.path().join("a"), false);
    write(
        &root.join("etc/library/list.hidden"),
        format!("{NEW}\n").as_bytes(),
    );
    let before = snapshot(&root);
    let err = send(&FaultyFs::new(), &root, &p, true).unwrap_err();
    assert!(
        matches!(err, TransferError::AlreadyInstalled { hidden: true, .. }),
        "{err:?}"
    );
    assert_eq!(snapshot(&root), before);

    let root = device(&tmp.path().join("b"), false);
    write(&root.join("str.hidden").join(NEW).join("info"), b"x");
    let err = send(&FaultyFs::new(), &root, &p, true).unwrap_err();
    assert!(
        matches!(err, TransferError::AlreadyInstalled { hidden: true, .. }),
        "{err:?}"
    );
}

#[test]
fn un_dossier_orphelin_est_remplace() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    write(&root.join("str").join(NEW).join("reste"), b"orphelin");
    let p = prepared(tmp.path(), 9);
    let report = send(&FaultyFs::new(), &root, &p, false).unwrap();
    assert!(report.replaced && report.index_updated);
    assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
}

#[test]
fn index_illisible_ou_appareil_absent() {
    let tmp = tempfile::tempdir().unwrap();
    let p = prepared(tmp.path(), 10);
    let root = device(tmp.path(), false);
    write(&root.join("etc/library/list"), b"\0\0pas un index");
    let before = snapshot(&root);
    let err = send(&FaultyFs::new(), &root, &p, false).unwrap_err();
    assert!(matches!(err, TransferError::IndexNeedsUser), "{err:?}");
    assert_eq!(snapshot(&root), before);

    std::fs::remove_file(root.join(MDF_FILE)).unwrap();
    let err = send(&FaultyFs::new(), &root, &p, false).unwrap_err();
    assert!(matches!(err, TransferError::DeviceNotFound), "{err:?}");

    let mut bad = p.clone();
    bad.uuid = NEW.to_uppercase();
    write(&root.join(MDF_FILE), &[0u8; 126]);
    let err = send(&FaultyFs::new(), &root, &bad, false).unwrap_err();
    assert!(matches!(err, TransferError::InvalidId), "{err:?}");
    let mut bad = p.clone();
    bad.files.push("../evil".into());
    assert!(send(&FaultyFs::new(), &root, &bad, false).is_err());
    assert!(!root.join(TRANSFER_DIR).exists());
}

#[test]
fn reprise_des_etats_partiels() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), true);
    let old = files_of(&root.join("str").join(NEW));
    let t = root.join(TRANSFER_DIR);

    // Ancienne version mise de côté, nouvelle pas encore en place : restaurée.
    std::fs::create_dir_all(&t).unwrap();
    std::fs::rename(root.join("str").join(NEW), t.join(format!("{NEW}.old"))).unwrap();
    write(&t.join(format!("{NEW}.in")).join("info"), b"partiel");
    let report = recover_with(&FaultyFs::new(), &root).unwrap();
    assert_eq!(report.previous_restored, 1);
    assert_eq!(report.incomplete_removed, 1);
    assert_eq!(files_of(&root.join("str").join(NEW)), old);
    assert!(!t.exists());

    // Nouvelle version en place, ancienne restée : supprimée, cache supprimé.
    write(&t.join(format!("{NEW}.old")).join("info"), b"ancienne");
    let report = recover_with(&FaultyFs::new(), &root).unwrap();
    assert_eq!(report.updates_finished, 1);
    assert!(!root.join(LIBRARY_CACHE).exists());
    assert_eq!(files_of(&root.join("str").join(NEW)), old);
    assert!(!t.exists());

    // Entrée inconnue : laissée.
    write(&t.join("inconnu").join("x"), b"x");
    let report = recover_with(&FaultyFs::new(), &root).unwrap();
    assert_eq!(report.left_alone, 1);
    assert!(t.join("inconnu").exists());
    std::fs::remove_dir_all(&t).unwrap();

    // Deux noms du même dossier (incohérence FAT) : rien n'est supprimé.
    write(&t.join(format!("{NEW}.old")).join("info"), b"ancienne");
    write(&t.join(format!("{NEW}.in")).join("info"), b"nouvelle");
    let fs = FaultyFs {
        same_dir: true,
        ..FaultyFs::new()
    };
    let report = recover_with(&fs, &root).unwrap();
    assert_eq!(report.left_alone, 2);
    assert!(t.join(format!("{NEW}.old")).exists());
    assert!(t.join(format!("{NEW}.in")).exists());
    // Un transfert refuse alors de démarrer, sans rien supprimer.
    let before = snapshot(&root);
    let p = prepared(tmp.path(), 11);
    let err = send(&fs, &root, &p, true).unwrap_err();
    assert!(matches!(err, TransferError::Io(_)), "{err:?}");
    assert_eq!(snapshot(&root), before);

    // La sonde réelle distingue deux dossiers différents.
    let real = FaultyFs::new().real();
    assert!(!real
        .same_directory(&t.join(format!("{NEW}.old")), &root.join("str").join(NEW))
        .unwrap());
    assert!(real
        .same_directory(&root.join("str").join(NEW), &root.join("str").join(NEW))
        .unwrap());
    assert_eq!(
        files_of(&t.join(format!("{NEW}.old"))).len(),
        1,
        "sonde supprimée"
    );
}

/// `.mdf` synthétique (clés de test inventées).
fn mdf(version: &str) -> Vec<u8> {
    let mut m = vec![0u8; 2];
    let mut text = format!("main: {version}-test\ncomm: x").into_bytes();
    text.resize(48, 0);
    m.extend(text);
    let mut serial = b"00123456789abc".to_vec();
    serial.resize(SERIAL_FIELD_LEN, 0);
    m.extend(serial);
    m.extend([0u8; 4]);
    m.extend(TEST_KEY);
    m.extend(TEST_IV);
    m.extend([0u8; 16]);
    assert_eq!(m.len(), MDF_LEN);
    m
}

#[test]
fn cles_lues_dans_mdf() {
    let keys = keys_from_mdf(&mdf("2.4.2")).unwrap();
    assert_eq!(&keys.key_file()[..], &test_keys().key_file()[..]);
    assert_eq!(keys.encrypt(b"abc"), test_keys().encrypt(b"abc"));
    assert!(matches!(
        keys_from_mdf(&mdf("1.9.0")),
        Err(TransferError::UnsupportedFirmware(v)) if v == "1.9.0"
    ));
    assert!(matches!(
        keys_from_mdf(&mdf("3.0.0")),
        Err(TransferError::UnsupportedFirmware(_))
    ));
    assert!(matches!(
        keys_from_mdf(&mdf("2.4.2")[..100]),
        Err(TransferError::KeyUnavailable(_))
    ));
    let mut no_version = mdf("2.4.2");
    no_version[2..50].fill(0);
    assert!(matches!(
        keys_from_mdf(&no_version),
        Err(TransferError::KeyUnavailable(_))
    ));
    let mut bad_serial = mdf("2.4.2");
    bad_serial[MDF_SERIAL_OFFSET] = b'z';
    assert!(matches!(
        keys_from_mdf(&bad_serial),
        Err(TransferError::KeyUnavailable(_))
    ));
}

#[test]
fn envoi_depuis_un_plain_pk() {
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    write(&root.join(MDF_FILE), &mdf("2.4.2"));
    let pk = tmp.path().join("histoire.plain.pk");
    let mut zip = ZipWriter::new(File::create(&pk).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let uuid_bytes = uuid::Uuid::parse_str(NEW).unwrap().into_bytes();
    for (name, data) in [
        ("uuid.bin", &uuid_bytes[..]),
        ("info.plain", b"Titre\nSous-titre\n000000\nTitre\n"),
        ("main.lua", b"function setup() end"),
        ("version", b"1"),
        ("sounds/a.mp3", &[9u8; 3000]),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();

    let work = tmp.path().join("cache").join(WORK_SUBDIR);
    let report = send_from_library_with(
        &FaultyFs::new(),
        &root,
        &pk,
        &work,
        false,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.id, NEW);
    let installed = files_of(&root.join("str").join(NEW));
    let names: Vec<&str> = installed.keys().map(String::as_str).collect();
    assert_eq!(
        names,
        ["info", "key", "main.lsf", "sounds/a.mp3", "version"]
    );
    assert_eq!(installed["key"], test_keys().key_file().to_vec());
    assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
    // Préparation supprimée.
    assert!(!work.join("str").join(NEW).exists());

    // Firmware non pris en charge : rien n'est préparé ni écrit.
    write(&root.join(MDF_FILE), &mdf("1.2.0"));
    let before = snapshot(&root);
    let err = send_from_library_with(
        &FaultyFs::new(),
        &root,
        &pk,
        &work,
        true,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err();
    assert!(
        matches!(err, TransferError::UnsupportedFirmware(_)),
        "{err:?}"
    );
    assert_eq!(snapshot(&root), before);
}

#[test]
fn erreurs_vers_l_interface() {
    let cases: [(TransferError, &str); 4] = [
        (TransferError::Cancelled, "cancelled"),
        (
            TransferError::NotEnoughSpace { needed: 2, free: 1 },
            "validation",
        ),
        (
            TransferError::AlreadyInstalled {
                id: id(NEW),
                hidden: true,
            },
            "validation",
        ),
        (TransferError::Verify("info".into()), "io"),
    ];
    for (err, kind) in cases {
        assert_eq!(AppError::from(err).kind(), kind);
    }
    let msg = TransferError::AlreadyInstalled {
        id: id(NEW),
        hidden: true,
    }
    .to_string();
    assert!(msg.contains("(masquée)"), "{msg}");
}

#[test]
fn la_relecture_reelle_rend_le_contenu_exact() {
    let tmp = tempfile::tempdir().unwrap();
    for len in [0usize, 1, 4095, 4096, CHUNK, 3 * CHUNK + 123] {
        let path = tmp.path().join(format!("f{len}"));
        let data: Vec<u8> = (0..len).map(|i| (i % 253) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let mut read = Vec::new();
        RealFs::default()
            .open_verify(&path)
            .unwrap()
            .read_to_end(&mut read)
            .unwrap();
        assert_eq!(read, data, "{len} octets");
    }
}

#[test]
fn coupures_repetees_pendant_la_reprise() {
    let tmp = tempfile::tempdir().unwrap();
    // État : mise à jour coupée entre la mise de côté et le renommage final.
    let setup = |base: &Path| {
        let root = device(base, true);
        let t = root.join(TRANSFER_DIR);
        std::fs::create_dir_all(&t).unwrap();
        std::fs::rename(root.join("str").join(NEW), t.join(format!("{NEW}.old"))).unwrap();
        write(
            &t.join(format!("{NEW}.in")).join("sounds/a.mp3"),
            &[1u8; 10],
        );
        write(&t.join(format!("{NEW}.in")).join("info"), b"partiel");
        root
    };
    let old = files_of(&device(&tmp.path().join("ref"), true).join("str").join(NEW));
    let counter = FaultyFs::new();
    recover_with(&counter, &setup(&tmp.path().join("compte"))).unwrap();
    let total_ops = counter.ops.get();
    for crash_at in 0..total_ops {
        let root = setup(&tmp.path().join(format!("r{crash_at}")));
        // Première reprise coupée, deuxième coupée plus tôt encore, puis reprise saine.
        let _ = recover_with(&FaultyFs::crash_at(crash_at), &root);
        let _ = recover_with(&FaultyFs::crash_at(crash_at / 2), &root);
        let lock = crate::device::lock::DeviceLock::new();
        let guard = lock.try_begin(Operation::Transfer).unwrap();
        recover_device(&guard, &root).unwrap();
        assert!(!root.join(TRANSFER_DIR).exists(), "étape {crash_at}");
        assert_eq!(
            files_of(&root.join("str").join(NEW)),
            old,
            "étape {crash_at}"
        );
        assert!(index::missing_stories(&root, &listed(&root)).is_empty());
    }
}
