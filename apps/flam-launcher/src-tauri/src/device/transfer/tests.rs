//! Tests du transfert sur faux appareil (dossier temporaire) avec injection de panne.
//!
//! Les aides `pub(crate)` (faux appareil [`device`], `.mdf` synthétique [`mdf`], instantané
//! [`snapshot`], index [`listed`]) servent aussi aux tests d'envoi depuis la bibliothèque des
//! autres modules (#556).

use std::cell::Cell;
use std::collections::BTreeMap;
use std::io::Write;

use super::*;
use crate::device::crypto::tests::{test_keys, TEST_IV, TEST_KEY};
use crate::device::crypto::FirmwareFamily;
use crate::device::guard::tests::{key_v2, mdf_v1_real, mdf_with, story, test_ctx, wrong_info};
use crate::device::guard::KNOWN_HEADER;
use crate::device::index::serialize;

pub(crate) const NEW: &str = "0a1b2c3d-4e5f-6071-8293-a4b5c6d7e8f9";
pub(crate) const OTHER: &str = "11111111-2222-3333-4444-555555555555";
const BIG: usize = 2 * CHUNK + 12_345;
const PLENTY: u64 = 1 << 40;

pub(crate) fn id(s: &str) -> StoryId {
    StoryId::parse(s).unwrap()
}

/// Autorisation de test pour les étapes d'écriture elles-mêmes (la garde est testée à part et
/// par les envois depuis la bibliothèque).
fn permit() -> WritePermit {
    WritePermit::for_tests(FirmwareFamily::V1, &mdf_v1_real())
}

/// Histoire officielle 2.x inventée (hors bibliothèque) : sa présence prouve les clés de test.
const OFFICIAL_V2: &str = "abababab-cdcd-4efe-8a8a-121212121212";

/// Arborescence complète (fichiers et dossiers vides) avec contenu, relative à `root`.
pub(crate) fn snapshot(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
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

pub(crate) fn write(path: &Path, data: &[u8]) {
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
pub(crate) fn device(base: &Path, old: bool) -> PathBuf {
    let root = base.join("flam");
    write(&root.join(MDF_FILE), &mdf_v1_real());
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
pub(crate) struct FaultyFs {
    free: u64,
    ops: Cell<usize>,
    crash_at: usize,
    /// Octets encore inscriptibles avant « disque plein ».
    budget: Cell<Option<u64>>,
    /// Fichier dont le premier octet écrit est altéré.
    pub(crate) corrupt: Option<&'static str>,
    /// Espace libre rendu une fois la corbeille (`.flam-trash/`) absente de la racine (#577) ;
    /// `free` tant qu'elle y est.
    free_without_trash: Option<u64>,
    /// Réponse forcée de `same_directory`.
    same_dir: bool,
    /// Dossier dont la suppression échoue (verrouillé) : le chemin contient ce texte.
    locked: Option<&'static str>,
    /// `sync_dir` échoue (renommage non forcé sur le support, #609).
    pub(crate) sync_dir_fails: bool,
    /// La relecture est retombée sur le cache du système (#609).
    pub(crate) cached_verify: bool,
    /// Lectures de `.mdf` faites (comptées à partir de 0).
    mdf_reads: Cell<usize>,
    /// Lectures de `.mdf` refusées par un verrou passager : (première, nombre).
    mdf_locked: Option<(usize, usize)>,
    /// À partir de cette lecture de `.mdf` : celui d'une autre Flam (`true`) ou absent (`false`).
    mdf_replaced_from: Option<(usize, bool)>,
    /// Attentes entre deux essais (comptées, pas faites).
    sleeps: Cell<usize>,
    /// Appelé à chaque `sync_dir`, donc juste après la promotion (changement d'appareil simulé).
    on_sync_dir: Option<Box<dyn Fn()>>,
}

impl FaultyFs {
    pub(crate) fn new() -> Self {
        Self {
            free: PLENTY,
            ops: Cell::new(0),
            crash_at: usize::MAX,
            budget: Cell::new(None),
            corrupt: None,
            free_without_trash: None,
            same_dir: false,
            locked: None,
            sync_dir_fails: false,
            cached_verify: false,
            mdf_reads: Cell::new(0),
            mdf_locked: None,
            mdf_replaced_from: None,
            sleeps: Cell::new(0),
            on_sync_dir: None,
        }
    }
    pub(crate) fn crash_at(n: usize) -> Self {
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
            ..RealFs::default()
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
    fn free_space(&self, root: &Path) -> io::Result<u64> {
        self.tick()?;
        match self.free_without_trash {
            Some(free) if !root.join(".flam-trash").exists() => Ok(free),
            _ => Ok(self.free),
        }
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
        if self
            .locked
            .is_some_and(|l| path.to_string_lossy().contains(l))
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "dossier verrouillé",
            ));
        }
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
    fn sync_dir(&self, _path: &Path) -> io::Result<()> {
        if let Some(hook) = &self.on_sync_dir {
            hook();
        }
        if self.sync_dir_fails {
            Err(io::Error::other("synchronisation refusée (test)"))
        } else {
            Ok(())
        }
    }
    fn verified_uncached(&self) -> bool {
        !self.cached_verify
    }
    fn read_mdf_once(&self, root: &Path) -> Result<zeroize::Zeroizing<Vec<u8>>, TransferError> {
        let n = self.mdf_reads.get();
        self.mdf_reads.set(n + 1);
        if self
            .mdf_locked
            .is_some_and(|(first, count)| (first..first + count).contains(&n))
        {
            return Err(TransferError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "verrou passager (test)",
            )));
        }
        match self.mdf_replaced_from {
            Some((from, true)) if n >= from => Ok(zeroize::Zeroizing::new(
                crate::device::guard::tests::mdf_v2_known(),
            )),
            Some((from, false)) if n >= from => Err(TransferError::DeviceNotFound),
            _ => read_mdf(root),
        }
    }
    fn retry_sleep(&self, _delay: Duration) {
        self.sleeps.set(self.sleeps.get() + 1);
    }
}

/// `replace` : remplacement demandé pour l'histoire préparée elle-même (`NEW`).
fn send(
    fs: &dyn DeviceFs,
    root: &Path,
    p: &PreparedStory,
    replace: bool,
) -> Result<TransferReport, TransferError> {
    send_to(fs, root, p, replace.then_some(NEW))
}

fn send_to(
    fs: &dyn DeviceFs,
    root: &Path,
    p: &PreparedStory,
    replace: Option<&str>,
) -> Result<TransferReport, TransferError> {
    send_with(
        fs,
        &permit(),
        root,
        p,
        replace,
        &AtomicBool::new(false),
        &mut |_| {},
    )
}

/// Redémarrage : reprise des transferts puis des index, avec un appareil sain.
fn restart(root: &Path) {
    recover_with(&FaultyFs::new(), &permit(), root).unwrap();
    for kind in IndexKind::ALL {
        index::recover(root, kind).unwrap();
    }
}

pub(crate) fn listed(root: &Path) -> Vec<StoryId> {
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
    assert!(
        !snapshot(root).keys().any(|k| k
            .rsplit('/')
            .next()
            .unwrap_or("")
            .starts_with(PROBE_PREFIX)),
        "fichier sonde résiduel (étape {step})"
    );
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
        &permit(),
        &root,
        &p,
        None,
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
            carrier_readback: None,
            carrier_stored: None,
            recovery_failures: Vec::new(),
            library_copy: None,
            dir_synced: cfg!(windows),
            verified_uncached: true,
            device_changed: false,
            installed_files: p
                .files
                .iter()
                .map(|f| (f.clone(), std::fs::metadata(p.dir.join(f)).unwrap().len()))
                .collect(),
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
            let result = send_with(
                &FaultyFs::new(),
                &permit(),
                &root,
                &p,
                Some(NEW),
                &cancel,
                &mut |e| {
                    if when(&e, total) {
                        cancel.store(true, Ordering::Relaxed);
                    }
                },
            );
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
    let before = snapshot(&root);
    let err = send(&FaultyFs::new(), &root, &p, true).unwrap_err();
    // Dossier masqué présent mais dans aucun index (orphelin masqué, visible dans « Ma Flam ») :
    // toujours refusé, avec la marche à suivre (#577).
    assert!(matches!(err, TransferError::HiddenOrphan { .. }), "{err:?}");
    assert!(err.to_string().contains("retirez d'abord"), "{err}");
    assert!(err.to_string().contains("Ma Flam"), "{err}");
    let app: AppError = err.into();
    assert_eq!(app.kind(), "validation");
    assert_eq!(snapshot(&root), before);
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
    write(&root.join(MDF_FILE), &mdf_v1_real());
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
    let report = recover_with(&FaultyFs::new(), &permit(), &root).unwrap();
    assert_eq!(report.previous_restored, 1);
    assert_eq!(report.incomplete_removed, 1);
    assert_eq!(files_of(&root.join("str").join(NEW)), old);
    assert!(!t.exists());

    // Nouvelle version en place, ancienne restée : supprimée, cache supprimé.
    write(&t.join(format!("{NEW}.old")).join("info"), b"ancienne");
    let report = recover_with(&FaultyFs::new(), &permit(), &root).unwrap();
    assert_eq!(report.updates_finished, 1);
    assert!(!root.join(LIBRARY_CACHE).exists());
    assert_eq!(files_of(&root.join("str").join(NEW)), old);
    assert!(!t.exists());

    // Entrée inconnue : laissée.
    write(&t.join("inconnu").join("x"), b"x");
    let report = recover_with(&FaultyFs::new(), &permit(), &root).unwrap();
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
    let report = recover_with(&fs, &permit(), &root).unwrap();
    assert_eq!(report.left_alone, 2);
    assert!(t.join(format!("{NEW}.old")).exists());
    assert!(t.join(format!("{NEW}.in")).exists());
    // Un transfert refuse alors de démarrer, sans rien supprimer.
    let before = snapshot(&root);
    let p = prepared(tmp.path(), 11);
    let err = send(&fs, &root, &p, true).unwrap_err();
    assert!(matches!(err, TransferError::Io(_)), "{err:?}");
    assert!(err.to_string().contains("chkdsk"), "{err}");
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
pub(crate) fn mdf(version: &str) -> Vec<u8> {
    // En-tête `01 00` : seule valeur mesurée (Flam 1.15.14), reprise pour la 2.x supposée.
    let mut m = KNOWN_HEADER.to_vec();
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
    assert_eq!(m.len(), crate::device::crypto::MDF_LEN);
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
    let mut garbage = mdf("2.4.2");
    garbage[2..50].fill(b'x');
    assert!(matches!(
        keys_from_mdf(&garbage),
        Err(TransferError::KeyUnavailable(_))
    ));
    let mut bad_serial = mdf("2.4.2");
    bad_serial[MDF_SERIAL_OFFSET] = b'z';
    assert!(matches!(
        keys_from_mdf(&bad_serial),
        Err(TransferError::KeyUnavailable(_))
    ));
}

/// Disposition observée sur une Flam 1.x (#269) : en-tête `01 00`, puis la version nue sans
/// « main: », complétée d'octets nuls.
fn mdf_bare(version: &str) -> Vec<u8> {
    let mut m = mdf("2.4.2");
    m[0] = 1;
    let mut text = format!("{version}-2d356995").into_bytes();
    text.resize(48, 0);
    m[2..50].copy_from_slice(&text);
    m
}

#[test]
fn version_nue_lue_dans_mdf() {
    assert_eq!(
        firmware_from_mdf(&mdf_bare("1.15.14")).as_deref(),
        Some("1.15.14")
    );
    assert!(matches!(
        keys_from_mdf(&mdf_bare("1.15.14")),
        Err(TransferError::UnsupportedFirmware(v)) if v == "1.15.14"
    ));
    let keys = keys_from_mdf(&mdf_bare("2.4.2")).unwrap();
    assert_eq!(&keys.key_file()[..], &test_keys().key_file()[..]);
}

const PK_INFO: &[u8] = b"Titre\nSous-titre\n000000\nTitre\n";
const PK_MAIN: &[u8] = b"function setup() end";

/// `.plain.pk` de l'histoire `NEW` dans `dir`.
fn plain_pk(dir: &Path) -> PathBuf {
    plain_pk_with(dir, &[9u8; 3000])
}

/// `.plain.pk` de l'histoire `NEW` (version 1) dont le son `sounds/a.mp3` vaut `sound`.
pub(crate) fn plain_pk_with(dir: &Path, sound: &[u8]) -> PathBuf {
    use zip::write::SimpleFileOptions;
    use zip::{CompressionMethod, ZipWriter};

    let pk = dir.join("histoire.plain.pk");
    let mut zip = ZipWriter::new(File::create(&pk).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    let uuid_bytes = uuid::Uuid::parse_str(NEW).unwrap().into_bytes();
    for (name, data) in [
        ("uuid.bin", &uuid_bytes[..]),
        ("info.plain", PK_INFO),
        ("main.lua", PK_MAIN),
        ("version", b"1"),
        ("sounds/a.mp3", sound),
    ] {
        zip.start_file(name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    zip.finish().unwrap();
    pk
}

#[test]
fn envoi_depuis_un_plain_pk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    write(&root.join(MDF_FILE), &mdf("2.4.2"));
    // Flam 2.x : aucune confirmation demandée (#326).
    let ctx = test_ctx();
    let pk = plain_pk(tmp.path());

    let work = tmp.path().join("cache").join(WORK_SUBDIR);
    let report = send_from_library_with(
        &FaultyFs::new(),
        &root,
        &pk,
        &work,
        None,
        None,
        &ctx,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.carrier_readback, None);
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

    // Firmware non pris en charge (3.x), ou 1.x sans demande carrier : rien n'est préparé ni
    // écrit.
    for (mdf, carrier_needed) in [
        (mdf("3.0.0"), false),
        (mdf_with(KNOWN_HEADER, "1.2.0-abcdef"), true),
    ] {
        write(&root.join(MDF_FILE), &mdf);
        let before = snapshot(&root);
        let err = send_from_library_with(
            &FaultyFs::new(),
            &root,
            &pk,
            &work,
            Some(NEW),
            None,
            &ctx,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        if carrier_needed {
            assert!(
                matches!(err, TransferError::Carrier(CarrierError::CarrierRequired)),
                "{err:?}"
            );
        } else {
            assert!(
                matches!(
                    err,
                    TransferError::Refused(WriteRefused::UnsupportedFirmware { .. })
                ),
                "{err:?}"
            );
        }
        assert_eq!(snapshot(&root), before);
        assert!(!work.join("str").join(NEW).exists());
    }
}

/// #288 : une histoire reconstruite **à version égale** (même uuid, même `version`, contenu
/// différent) se remplace sur la Flam par le chemin « replace » : sans lui, refus « déjà sur la
/// Flam » et rien n'est écrit ; avec lui, le nouveau contenu remplace l'ancien.
#[test]
fn remplacement_a_version_egale_contenu_different() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    write(&root.join(MDF_FILE), &mdf("2.4.2"));
    // Flam 2.x : aucune confirmation demandée (#326).
    let ctx = test_ctx();
    let work = tmp.path().join("cache").join(WORK_SUBDIR);
    let send_pk = |pk: &Path, replace: bool| {
        send_from_library_with(
            &FaultyFs::new(),
            &root,
            pk,
            &work,
            replace.then_some(NEW),
            None,
            &ctx,
            &AtomicBool::new(false),
            &mut |_| {},
        )
    };
    let first = tmp.path().join("v1");
    std::fs::create_dir_all(&first).unwrap();
    send_pk(&plain_pk_with(&first, &[1u8; 3000]), false).unwrap();
    let before = files_of(&root.join("str").join(NEW));

    let rebuilt = tmp.path().join("v1-reconstruite");
    std::fs::create_dir_all(&rebuilt).unwrap();
    let pk = plain_pk_with(&rebuilt, &[2u8; 3000]);
    let snapshot_before = snapshot(&root);
    let err = send_pk(&pk, false).unwrap_err();
    assert!(
        matches!(err, TransferError::AlreadyInstalled { hidden: false, .. }),
        "{err:?}"
    );
    assert_eq!(snapshot(&root), snapshot_before, "rien n'est écrit");

    let report = send_pk(&pk, true).unwrap();
    assert!(report.replaced && !report.index_updated);
    let after = files_of(&root.join("str").join(NEW));
    assert_eq!(before["version"], after["version"], "même version");
    assert_ne!(
        before["sounds/a.mp3"], after["sounds/a.mp3"],
        "contenu remplacé"
    );
    assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
    assert!(!root.join(TRANSFER_DIR).exists());
}

// --- Mode carrier (firmware 1.x, #234) ---------------------------------------------------------

mod carrier_flow {
    use super::*;
    use crate::device::carrier::tests::{
        assert_no_leak, capture_logs, encrypted_info, fake_store, foreign_bt, hex_of,
        official_story, test_bt, test_key_file, FakeProtector, INFO_PLAIN, OFFICIAL,
    };
    use crate::device::carrier::{CarrierConsent, EnteredBt, WARNING_VERSION};
    use crate::device::crypto::decrypt_blocks;

    fn consent() -> CarrierConsent {
        CarrierConsent {
            text_version: WARNING_VERSION,
            accepted_at: "2026-10-17T10:00:00Z".into(),
        }
    }

    /// Faux appareil 1.x : `.mdf` 1.15.14, une histoire officielle (`OFFICIAL`, avec `bt`)
    /// listée, cache du firmware.
    fn device_v1(base: &Path) -> PathBuf {
        let root = base.join("flam");
        write(&root.join(MDF_FILE), &mdf_v1_real());
        official_story(&root, "str", OFFICIAL, true);
        write(&root.join(LIBRARY_CACHE), b"cache");
        write(&root.join("etc/library/list"), &serialize(&[id(OFFICIAL)]));
        write(&root.join("etc/library/list.hidden"), b"");
        root
    }

    struct Env {
        _tmp: tempfile::TempDir,
        root: PathBuf,
        pk: PathBuf,
        work: PathBuf,
        store: CarrierStore,
        keys_dir: PathBuf,
    }

    fn env() -> Env {
        let tmp = tempfile::tempdir().unwrap();
        let root = device_v1(tmp.path());
        let pk = plain_pk(tmp.path());
        let work = tmp.path().join("cache").join(WORK_SUBDIR);
        let keys_dir = tmp.path().join("cles");
        let store = fake_store(&keys_dir);
        Env {
            _tmp: tmp,
            root,
            pk,
            work,
            store,
            keys_dir,
        }
    }

    fn send_carrier(
        fs: &dyn DeviceFs,
        e: &Env,
        consent: Option<&CarrierConsent>,
        choice: CarrierChoice,
        remember: bool,
    ) -> Result<TransferReport, TransferError> {
        let request = CarrierRequest {
            consent,
            choice,
            remember,
            store: &e.store,
        };
        send_from_library_with(
            fs,
            &e.root,
            &e.pk,
            &e.work,
            None,
            Some(&request),
            &test_ctx(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    fn device_choice() -> CarrierChoice {
        CarrierChoice::Device {
            uuid: OFFICIAL.into(),
        }
    }

    fn decrypt_with_bt(data: &[u8]) -> Vec<u8> {
        let bt = test_bt();
        let mut out = data.to_vec();
        decrypt_blocks(
            &bt[..16].try_into().unwrap(),
            &bt[16..].try_into().unwrap(),
            &mut out,
        )
        .unwrap();
        out
    }

    #[test]
    fn envoi_carrier_de_bout_en_bout() {
        let e = env();
        let c = consent();
        let logs = capture_logs(|| {
            let report =
                send_carrier(&FaultyFs::new(), &e, Some(&c), device_choice(), true).unwrap();
            assert_eq!(report.carrier_readback, Some(true));
            assert_no_leak(&serde_json::to_string(&report).unwrap(), "rapport");
            assert_no_leak(&format!("{report:?}"), "rapport");
        });
        assert_no_leak(&logs, "journaux");
        let installed = files_of(&e.root.join("str").join(NEW));
        assert_eq!(
            installed["key"],
            test_key_file().to_vec(),
            "key de l'officielle"
        );
        assert!(!installed.contains_key("bt"), "bt jamais copié");
        assert!(decrypt_with_bt(&installed["info"]).starts_with(PK_INFO));
        assert!(decrypt_with_bt(&installed["main.lsf"]).starts_with(PK_MAIN));
        assert_eq!(installed["sounds/a.mp3"], vec![9u8; 3000]);
        assert_eq!(listed(&e.root), vec![id(OFFICIAL), id(NEW)]);
        // Histoire officielle intacte.
        let official = files_of(&e.root.join("str").join(OFFICIAL));
        assert_eq!(official["bt"], test_bt().to_vec());
        assert_eq!(official["info"], encrypted_info(&test_bt(), INFO_PLAIN));
        // Couple conservé, relisible ; puis réutilisé depuis la conservation.
        let id_flam = crate::device::carrier::DeviceId::from_mdf(&mdf_v1_real()).unwrap();
        assert!(e.store.has(&id_flam));
        std::fs::remove_dir_all(e.root.join("str").join(NEW)).unwrap();
        write(
            &e.root.join("etc/library/list"),
            &serialize(&[id(OFFICIAL)]),
        );
        let again =
            send_carrier(&FaultyFs::new(), &e, Some(&c), CarrierChoice::Stored, false).unwrap();
        assert_eq!(again.carrier_readback, Some(true));
        // Aucune clé dans le dossier de conservation en clair (protecteur de test : XOR).
        for entry in std::fs::read_dir(&e.keys_dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert_no_leak(&name, "nom de fichier");
        }
    }

    #[test]
    fn aucune_copie_sans_consentement() {
        let e = env();
        let before = snapshot(&e.root);
        let old = CarrierConsent {
            text_version: WARNING_VERSION + 1,
            ..consent()
        };
        for given in [None, Some(&old)] {
            for choice in [device_choice(), CarrierChoice::Stored] {
                let err = send_carrier(&FaultyFs::new(), &e, given, choice, true).unwrap_err();
                assert!(
                    matches!(err, TransferError::Carrier(CarrierError::ConsentRequired)),
                    "{err:?}"
                );
                assert_no_leak(&err.to_string(), "erreur");
            }
        }
        assert_eq!(snapshot(&e.root), before, "Flam inchangée");
        assert!(!e.work.join("str").exists(), "rien de préparé");
        assert!(!e.keys_dir.exists(), "rien de conservé");
    }

    #[test]
    fn couple_invalide_refuse_avant_copie() {
        let e = env();
        let before = snapshot(&e.root);
        // Couple d'un autre compte fourni par fichier.
        let other = e._tmp.path().join("autre-compte");
        write(&other.join("bt"), &foreign_bt());
        write(&other.join("key"), &test_key_file());
        // `key` d'un autre compte avec le bon bt.
        let other_key = e._tmp.path().join("autre-key");
        write(&other_key.join("bt"), &test_bt());
        write(&other_key.join("key"), &[0x33u8; 32]);
        let c = consent();
        let logs = capture_logs(|| {
            for dir in [&other, &other_key] {
                let choice = CarrierChoice::File {
                    path: dir.join("bt").display().to_string(),
                };
                let err = send_carrier(&FaultyFs::new(), &e, Some(&c), choice, true).unwrap_err();
                assert!(
                    matches!(err, TransferError::Carrier(CarrierError::Mismatch)),
                    "{err:?}"
                );
                let shown = crate::AppError::from(err);
                assert_eq!(shown.kind(), "validation");
                assert_no_leak(&format!("{shown} {shown:?}"), "erreur");
            }
            // Aucune clé conservée : demande échouée.
            let err = send_carrier(&FaultyFs::new(), &e, Some(&c), CarrierChoice::Stored, false)
                .unwrap_err();
            assert!(matches!(
                err,
                TransferError::Carrier(CarrierError::NoStoredKeys)
            ));
        });
        assert_no_leak(&logs, "journaux");
        assert_eq!(snapshot(&e.root), before, "rien écrit sur la Flam");
        assert!(!e.work.join("str").exists(), "rien de préparé");
        assert!(!e.keys_dir.exists(), "couple invalide jamais conservé");
    }

    #[test]
    fn panne_de_conservation_n_empeche_pas_l_envoi() {
        let mut e = env();
        e.store = CarrierStore::new(e.keys_dir.clone(), Box::new(FakeProtector { fail: true }));
        let c = consent();
        let report = send_carrier(&FaultyFs::new(), &e, Some(&c), device_choice(), true).unwrap();
        assert_eq!(report.carrier_readback, Some(true));
        assert_eq!(
            report.carrier_stored,
            Some(false),
            "échec de conservation signalé"
        );
        assert!(!e.keys_dir.exists());
    }

    #[test]
    fn relecture_d_info_illisible_signalee() {
        let e = env();
        let c = consent();
        // Une altération pendant la copie est déjà refusée par la vérification d'empreinte :
        // la relecture est éprouvée sur un `info` altéré après l'installation.
        let report = send_carrier(&FaultyFs::new(), &e, Some(&c), device_choice(), false).unwrap();
        assert_eq!(report.carrier_readback, Some(true));
        let info = e.root.join("str").join(NEW).join("info");
        std::fs::write(&info, [0u8; 32]).unwrap();
        let keys = crate::device::carrier::read_device_carrier(&e.root, OFFICIAL, || None).unwrap();
        assert!(!crate::device::carrier::readback(&e.root, &id(NEW), &keys));
    }

    /// Coupure à chaque opération d'écriture d'un envoi carrier, puis redémarrage : la Flam
    /// reste cohérente (aucun index vers du vide, histoire absente ou complète avec le `key`
    /// officiel), l'histoire officielle est intacte.
    #[test]
    fn coupure_a_chaque_etape_carrier() {
        let c = consent();
        let mut crash_at = 0;
        loop {
            let e = env();
            let official_before = files_of(&e.root.join("str").join(OFFICIAL));
            let result = send_carrier(
                &FaultyFs::crash_at(crash_at),
                &e,
                Some(&c),
                device_choice(),
                false,
            );
            restart(&e.root);
            assert!(
                !e.root.join(TRANSFER_DIR).exists(),
                "résidu (étape {crash_at})"
            );
            let entries = listed(&e.root);
            assert!(
                index::missing_stories(&e.root, &entries).is_empty(),
                "index vers du vide (étape {crash_at})"
            );
            assert_eq!(
                files_of(&e.root.join("str").join(OFFICIAL)),
                official_before,
                "officielle modifiée (étape {crash_at})"
            );
            let target = e.root.join("str").join(NEW);
            if target.exists() {
                let installed = files_of(&target);
                assert_eq!(installed.len(), 5, "incomplet (étape {crash_at})");
                assert_eq!(installed["key"], test_key_file().to_vec());
            }
            if result.is_ok() {
                assert!(entries.contains(&id(NEW)));
                break;
            }
            crash_at += 1;
            assert!(crash_at < 500, "l'envoi n'aboutit jamais");
        }
        assert!(crash_at > 5, "pannes injectées : {crash_at}");
    }

    // --- Saisie du bt (#271) ------------------------------------------------------------------

    /// Flam 1.x telle qu'observée : l'histoire officielle n'a pas de `bt`.
    fn env_without_bt() -> Env {
        let e = env();
        std::fs::remove_file(e.root.join("str").join(OFFICIAL).join("bt")).unwrap();
        e
    }

    fn entered(bt: &[u8]) -> CarrierChoice {
        CarrierChoice::Entered {
            bt: EnteredBt::new(hex_of(bt)),
            confirmed: true,
        }
    }

    #[test]
    fn envoi_avec_bt_saisi() {
        let e = env_without_bt();
        let c = consent();
        let logs = capture_logs(|| {
            let report =
                send_carrier(&FaultyFs::new(), &e, Some(&c), entered(&test_bt()), true).unwrap();
            assert_eq!(report.carrier_readback, Some(true));
            assert_eq!(report.carrier_stored, Some(true));
            assert_no_leak(&serde_json::to_string(&report).unwrap(), "rapport");
        });
        assert_no_leak(&logs, "journaux");
        let installed = files_of(&e.root.join("str").join(NEW));
        assert_eq!(installed["key"], test_key_file().to_vec());
        assert!(!installed.contains_key("bt"), "bt jamais copié");
        assert!(decrypt_with_bt(&installed["info"]).starts_with(PK_INFO));
        // Couple conservé : réutilisable sans nouvelle saisie.
        let id_flam = crate::device::carrier::DeviceId::from_mdf(&mdf_v1_real()).unwrap();
        assert!(e.store.has(&id_flam));
        for entry in std::fs::read_dir(&e.keys_dir).unwrap() {
            let name = entry.unwrap().file_name().to_string_lossy().into_owned();
            assert_no_leak(&name, "nom de fichier");
        }
    }

    #[test]
    fn bt_saisi_refuse_ou_annule_rien_n_est_ecrit() {
        let e = env_without_bt();
        let before = snapshot(&e.root);
        let c = consent();
        let unknown: [u8; 32] = std::array::from_fn(|i| 0x99 ^ i as u8);
        let mut short = hex_of(&test_bt());
        short.truncate(63);
        let logs = capture_logs(|| {
            for (choice, expected) in [
                (entered(&unknown), "validation"),
                (entered(&foreign_bt()), "validation"),
                (
                    CarrierChoice::Entered {
                        bt: EnteredBt::new(short.clone()),
                        confirmed: true,
                    },
                    "validation",
                ),
                // Titre non confirmé : rien n'est écrit.
                (
                    CarrierChoice::Entered {
                        bt: EnteredBt::new(hex_of(&test_bt())),
                        confirmed: false,
                    },
                    "validation",
                ),
            ] {
                let err = send_carrier(&FaultyFs::new(), &e, Some(&c), choice, true).unwrap_err();
                let shown = crate::AppError::from(err);
                assert_eq!(shown.kind(), expected);
                assert_no_leak(&format!("{shown} {shown:?}"), "erreur");
            }
            // Sans consentement.
            let err =
                send_carrier(&FaultyFs::new(), &e, None, entered(&test_bt()), true).unwrap_err();
            assert!(matches!(
                err,
                TransferError::Carrier(CarrierError::ConsentRequired)
            ));
            // Annulation pendant la recherche de l'histoire.
            let request = CarrierRequest {
                consent: Some(&c),
                choice: entered(&test_bt()),
                remember: true,
                store: &e.store,
            };
            let err = send_from_library_with(
                &FaultyFs::new(),
                &e.root,
                &e.pk,
                &e.work,
                None,
                Some(&request),
                &test_ctx(),
                &AtomicBool::new(true),
                &mut |_| {},
            )
            .unwrap_err();
            assert!(matches!(err, TransferError::Cancelled), "{err:?}");
            assert_eq!(crate::AppError::from(err).kind(), "cancelled");
        });
        assert_no_leak(&logs, "journaux");
        assert_eq!(snapshot(&e.root), before, "rien écrit sur la Flam");
        assert!(!e.work.join("str").exists(), "rien de préparé");
        assert!(!e.keys_dir.exists(), "rien conservé sur le PC");
    }

    /// #619 : après un envoi carrier réel (copie, relecture sans cache, renommage forcé, index,
    /// relecture de `info`), une mise à jour, puis les lectures de « Ma Flam » (liste, titres,
    /// vignettes, carriers), le launcher ne garde aucune poignée sur la Flam. Sonde : Windows
    /// refuse de renommer un dossier dont un fichier ou un sous-dossier est encore ouvert.
    /// Portée : poignées ouvertes par ce code Rust, chemin de succès, sur NTFS ; ni poignée de
    /// volume, ni dossier courant du processus, ni WebView2, ni surveillance `notify`.
    #[cfg(windows)]
    #[test]
    fn aucune_poignee_ne_reste_ouverte_sur_la_flam_apres_envoi_et_lectures() {
        use crate::device::details::{read_titles, story_thumbnail};
        use crate::device::index::scan_stories;

        let e = env();
        let renamed = e.root.with_file_name("flam-renommee");
        // La sonde détecte bien une poignée de fichier ou de dossier restée ouverte.
        {
            let _open = File::open(e.root.join(MDF_FILE)).unwrap();
            assert!(
                std::fs::rename(&e.root, &renamed).is_err(),
                "sonde (fichier)"
            );
        }
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
            let _dir = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
                .open(e.root.join("str"))
                .unwrap();
            assert!(
                std::fs::rename(&e.root, &renamed).is_err(),
                "sonde (dossier)"
            );
        }

        let fs = FaultyFs::new().real();
        let c = consent();
        let report = send_carrier(&fs, &e, Some(&c), device_choice(), true).unwrap();
        assert_eq!(report.carrier_readback, Some(true));
        assert!(report.dir_synced && report.verified_uncached, "{report:?}");
        // Mise à jour (ancienne version mise de côté puis supprimée).
        let request = CarrierRequest {
            consent: Some(&c),
            choice: CarrierChoice::Stored,
            remember: false,
            store: &e.store,
        };
        let again = send_from_library_with(
            &fs,
            &e.root,
            &e.pk,
            &e.work,
            Some(NEW),
            Some(&request),
            &test_ctx(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        assert!(again.replaced, "{again:?}");

        // Lectures de « Ma Flam ».
        let visible = read_index(&e.root, IndexKind::Visible);
        let _ = read_index(&e.root, IndexKind::Hidden);
        let _ = index::missing_stories(&e.root, &visible.entries);
        let _ = scan_stories(&e.root).unwrap();
        let uuids: Vec<String> = visible.entries.iter().map(ToString::to_string).collect();
        let _ = read_titles(&e.root, &uuids, None, &|| false);
        let thumbs = e._tmp.path().join("vignettes");
        for uuid in &uuids {
            let _ = story_thumbnail(&e.root, uuid, &thumbs);
        }
        let _ = carrier::list_candidates(&e.root, None);
        let _ = read_mdf(&e.root).unwrap();

        // Nouveaux essais pendant environ 5 s : un antivirus peut ouvrir un instant un fichier
        // tout juste écrit ; une poignée oubliée par le launcher, elle, tient au-delà.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match std::fs::rename(&e.root, &renamed) {
                Ok(()) => break,
                Err(_) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                Err(err) => panic!(
                    "une poignée sur la Flam est restée ouverte après l'envoi ou les lectures : {err}"
                ),
            }
        }
        std::fs::rename(&renamed, &e.root).unwrap();
    }
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
    recover_with(&counter, &permit(), &setup(&tmp.path().join("compte"))).unwrap();
    let total_ops = counter.ops.get();
    for crash_at in 0..total_ops {
        let root = setup(&tmp.path().join(format!("r{crash_at}")));
        // Première reprise coupée, deuxième coupée plus tôt encore, puis reprise saine.
        let _ = recover_with(&FaultyFs::crash_at(crash_at), &permit(), &root);
        let _ = recover_with(&FaultyFs::crash_at(crash_at / 2), &permit(), &root);
        let lock = crate::device::lock::DeviceLock::new();
        let guard = lock.try_begin(Operation::Transfer).unwrap();
        recover_device(&guard, &root, &test_ctx()).unwrap();
        assert!(!root.join(TRANSFER_DIR).exists(), "étape {crash_at}");
        assert_eq!(
            files_of(&root.join("str").join(NEW)),
            old,
            "étape {crash_at}"
        );
        assert!(index::missing_stories(&root, &listed(&root)).is_empty());
    }
}

/// Une entrée de reprise qui échoue (dossier verrouillé) n'empêche ni la reprise des autres, ni
/// l'envoi d'une autre histoire ; `recover_with` rend toujours l'erreur.
#[test]
fn reprise_continue_apres_une_entree_verrouillee() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let transfer = root.join(TRANSFER_DIR);
    write(
        &transfer
            .join(format!("{OTHER}{INCOMING_SUFFIX}"))
            .join("info"),
        b"x",
    );
    let third = "22222222-3333-4444-5555-666666666666";
    write(
        &transfer
            .join(format!("{third}{INCOMING_SUFFIX}"))
            .join("info"),
        b"y",
    );
    let fs = FaultyFs {
        locked: Some(OTHER),
        ..FaultyFs::new()
    };
    let (report, failures) = recover_collect(&fs, &permit(), &root).unwrap();
    assert_eq!(report.incomplete_removed, 1, "la troisième est reprise");
    assert_eq!(failures.len(), 1);
    assert!(failures[0].0.starts_with(OTHER));
    assert!(!transfer.join(format!("{third}{INCOMING_SUFFIX}")).exists());
    assert!(recover_with(&fs, &permit(), &root).is_err());

    // L'envoi d'une autre histoire n'est pas bloqué par le résidu verrouillé, mais le rapport
    // le signale (#309, M-c).
    let p = prepared(tmp.path(), 1);
    let report = send(&fs, &root, &p, false).unwrap();
    assert!(root.join("str").join(NEW).join("info").is_file());
    assert_eq!(report.recovery_failures.len(), 1, "{report:?}");
    assert!(
        report.recovery_failures[0].starts_with(&format!("{OTHER}{INCOMING_SUFFIX} : ")),
        "{report:?}"
    );
    // Reprise réussie ensuite : plus rien à signaler.
    let report = send(&FaultyFs::new(), &root, &p, true).unwrap();
    assert!(report.recovery_failures.is_empty(), "{report:?}");
}

/// #309 (N1) : un résidu de la même histoire que la reprise n'a pas pu supprimer (dossier
/// verrouillé) est refusé sans renvoyer vers `chkdsk`, et rien n'est écrit.
#[test]
fn residu_verrouille_de_la_meme_histoire_sans_chkdsk() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    write(
        &root
            .join(TRANSFER_DIR)
            .join(format!("{NEW}{INCOMING_SUFFIX}"))
            .join("info"),
        b"x",
    );
    let fs = FaultyFs {
        locked: Some(NEW),
        ..FaultyFs::new()
    };
    let before = snapshot(&root);
    let p = prepared(tmp.path(), 2);
    let err = send(&fs, &root, &p, false).unwrap_err();
    let text = err.to_string();
    assert!(matches!(err, TransferError::Io(_)), "{err:?}");
    assert!(
        !text.contains("chkdsk") && text.contains("réessayez"),
        "{text}"
    );
    assert_eq!(snapshot(&root), before);
}

/// #309 (M-b) : « Remplacer » ne s'applique qu'à l'histoire visée. Une archive dont l'UUID
/// diffère de celui que l'utilisateur a choisi de remplacer est refusée sans rien écrire, même
/// si son UUID est déjà sur la Flam ; l'UUID visé est comparé en forme simple ou à tirets.
#[test]
fn remplacement_lie_a_l_uuid_vise() {
    let tmp = tempfile::tempdir().unwrap();
    let p = prepared(tmp.path(), 12);
    let braced = format!("{{{NEW}}}");
    let urn = format!("urn:uuid:{NEW}");
    for update in [true, false] {
        let root = device(&tmp.path().join(format!("refus-{update}")), update);
        // Une reprise en attente : le refus la précède, rien n'est touché.
        write(
            &root
                .join(TRANSFER_DIR)
                .join(format!("{OTHER}{INCOMING_SUFFIX}"))
                .join("info"),
            b"partiel",
        );
        let before = snapshot(&root);
        for target in [
            OTHER,
            "11111111222233334444555555555555",
            "",
            "pas un uuid",
            braced.as_str(),
            urn.as_str(),
        ] {
            let err = send_to(&FaultyFs::new(), &root, &p, Some(target)).unwrap_err();
            assert!(
                matches!(&err, TransferError::ReplaceMismatch { id } if id.as_str() == NEW),
                "{target:?} : {err:?}"
            );
            assert_eq!(
                crate::AppError::from(err).kind(),
                "validation",
                "{target:?}"
            );
            assert_eq!(snapshot(&root), before, "rien n'est écrit ({target:?})");
        }
    }
    // Forme simple (source dossier) et majuscules : même UUID, remplacement accepté.
    let simple = NEW.replace('-', "");
    for (n, target) in [simple.clone(), simple.to_uppercase(), NEW.to_uppercase()]
        .iter()
        .enumerate()
    {
        let root = device(&tmp.path().join(format!("ok-{n}")), true);
        let report = send_to(&FaultyFs::new(), &root, &p, Some(target)).unwrap();
        assert!(report.replaced, "{target}");
        assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
        assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
    }
}

#[test]
fn racine_hors_des_lecteurs_detectes_refusee() {
    let roots = [PathBuf::from("D:"), PathBuf::from("E:/")];
    for ok in ["D:/", "d:\\", "e:/", "E:\\"] {
        ensure_root_among(Path::new(ok), &roots).unwrap_or_else(|e| panic!("{ok} : {e}"));
    }
    // #577 : `E:` sans séparateur final désigne le dossier courant du lecteur, pas sa racine
    // (`E:` joint à `.mdf` donne `E:.mdf`).
    for bad in [
        "",
        "C:",
        "D:/dossier",
        "/",
        "F:",
        "D:",
        "d:",
        "e:",
        r"\\?\E:",
    ] {
        assert!(
            matches!(
                ensure_root_among(Path::new(bad), &roots),
                Err(TransferError::InvalidRoot)
            ),
            "{bad}"
        );
    }
}

#[test]
fn mdf_de_plus_de_64_kio_refuse_comme_a_la_detection() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("flam");
    write(&root.join(MDF_FILE), &vec![0u8; MDF_MAX_SIZE as usize + 1]);
    assert!(matches!(
        read_mdf(&root),
        Err(TransferError::KeyUnavailable(_))
    ));
    write(&root.join(MDF_FILE), &vec![0u8; MDF_MAX_SIZE as usize]);
    assert_eq!(read_mdf(&root).unwrap().len() as u64, MDF_MAX_SIZE);
}

/// Garde commune (#317) à l'envoi : firmware refusé, clés 2.x triviales, `.mdf` tronqué : rien
/// n'est préparé ni écrit, pas même la reprise d'un envoi interrompu.
#[test]
fn envoi_refuse_par_la_garde_du_firmware() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let pk = plain_pk(tmp.path());
    let work = tmp.path().join("cache").join(WORK_SUBDIR);
    // Reste d'un envoi interrompu : la reprise ne doit pas tourner.
    write(
        &root.join(TRANSFER_DIR).join(format!("{NEW}.in")).join("x"),
        b"x",
    );
    let mut zero_key = mdf("2.4.2");
    zero_key[crate::device::crypto::MDF_KEY_OFFSET..crate::device::crypto::MDF_KEY_OFFSET + 16]
        .fill(0);
    type Expect = fn(&WriteRefused) -> bool;
    let cases: [(Vec<u8>, Expect); 3] = [
        (mdf("3.0.0"), |e| {
            matches!(e, WriteRefused::UnsupportedFirmware { .. })
        }),
        (zero_key, |e| matches!(e, WriteRefused::KeysTrivial { .. })),
        (mdf("2.4.2")[..MDF_MIN_LEN - 1].to_vec(), |e| {
            matches!(e, WriteRefused::Truncated { .. })
        }),
    ];
    for (mdf, expected) in cases {
        write(&root.join(MDF_FILE), &mdf);
        let before = snapshot(&root);
        let err = send_from_library_with(
            &FaultyFs::new(),
            &root,
            &pk,
            &work,
            None,
            None,
            &test_ctx(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap_err();
        match &err {
            TransferError::Refused(reason) => assert!(expected(reason), "{reason}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(crate::AppError::from(err).kind(), "validation");
        assert_eq!(snapshot(&root), before, "rien écrit");
        assert!(!work.join("str").join(NEW).exists(), "rien préparé");
    }
}

/// 2.x de disposition non standard, version sans « main: » : envoi sans confirmation (#326),
/// avec les clés de `.mdf` et le fichier `key` déduit du numéro de série. Ni une histoire
/// installée par un autre outil au format Lunii, ni une contradiction de cohérence (histoire au
/// format Flam au fichier `key` déduit mais `info` illisible : simple avertissement) n'empêchent
/// l'envoi.
#[test]
fn envoi_2x_non_standard_sans_confirmation_malgre_une_contradiction() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let mut mdf = mdf_with([0, 0], "2.4.2-nue");
    mdf.extend([0u8; 10]);
    write(&root.join(MDF_FILE), &mdf);
    let lunii = root.join("str").join(OFFICIAL_V2);
    write(&lunii.join("key"), &key_v2());
    write(&lunii.join("ni"), b"ni");
    write(&lunii.join("info"), &[7u8; 32]);
    story(
        &root,
        "77777777-6666-4555-8444-333333333333",
        &key_v2(),
        &wrong_info(),
    );
    let pk = plain_pk(tmp.path());
    let work = tmp.path().join("cache").join(WORK_SUBDIR);
    let ctx = test_ctx();
    // La contradiction est bien vue par « Ma Flam » (avertissement), sans refus.
    let status = crate::device::guard::firmware_status(&root, &mdf, &ctx, false);
    assert!(
        matches!(
            status.coherence,
            crate::device::guard::Coherence::Contradiction { .. }
        ),
        "{:?}",
        status.coherence
    );
    assert_eq!(status.write_block, None);
    let report = send_from_library_with(
        &FaultyFs::new(),
        &root,
        &pk,
        &work,
        None,
        None,
        &ctx,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.id, NEW);
    let installed = files_of(&root.join("str").join(NEW));
    assert_eq!(installed["key"], key_v2());
    assert_eq!(
        test_keys().title_of_info(&installed["info"]).as_deref(),
        Some("Titre")
    );
}

/// La reprise au branchement passe par la garde : refusée sur un firmware non pris en charge.
#[test]
fn reprise_refusee_par_la_garde_du_firmware() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    write(&root.join(MDF_FILE), &mdf("3.0.0"));
    write(
        &root.join(TRANSFER_DIR).join(format!("{NEW}.in")).join("x"),
        b"x",
    );
    let before = snapshot(&root);
    let guard = crate::device::lock::DeviceLock::new();
    let guard = guard.try_begin(Operation::Transfer).unwrap();
    let err = recover_device(&guard, &root, &test_ctx()).unwrap_err();
    assert!(matches!(err, TransferError::Refused(_)), "{err:?}");
    assert_eq!(snapshot(&root), before);
    // Flam 1.15.14 réelle : la reprise a lieu.
    write(&root.join(MDF_FILE), &mdf_v1_real());
    recover_device(&guard, &root, &test_ctx()).unwrap();
    assert!(!root.join(TRANSFER_DIR).exists());
    // Flam 2.x : la reprise a lieu sans confirmation (#326).
    write(&root.join(MDF_FILE), &mdf("2.4.2"));
    write(
        &root.join(TRANSFER_DIR).join(format!("{NEW}.in")).join("x"),
        b"x",
    );
    recover_device(&guard, &root, &test_ctx()).unwrap();
    assert!(!root.join(TRANSFER_DIR).exists());
}

#[test]
fn comm_avant_main_et_version_nue() {
    let m = mdf_with(KNOWN_HEADER, "comm: 7.7.7\nmain: 2.4.2-x");
    assert_eq!(firmware_from_mdf(&m).as_deref(), Some("2.4.2"));
    assert_eq!(
        firmware_field(&mdf_v1_real()),
        Some(("1.15.14".to_string(), VersionForm::Bare))
    );
}

/// #577 : la garde refuse `Z:` (relatif au dossier courant du lecteur) même quand `Z:\` est
/// détecté, avant tout accès au lecteur.
#[test]
fn garde_refuse_une_lettre_de_lecteur_sans_separateur() {
    let session = crate::device::guard::KeySession::default();
    let ctx = WriteContext::new(
        &session,
        Some(vec![PathBuf::from("Z:\\")]),
        Default::default(),
    );
    let kind = crate::device::guard::WriteKind::Other;
    assert_eq!(
        crate::device::guard::check_write(Path::new("Z:"), &ctx, kind).unwrap_err(),
        WriteRefused::InvalidRoot
    );
}

/// #577 : la Flam A est débranchée pendant la préparation et une Flam B (autre `.mdf`) prend la
/// même lettre : l'envoi est refusé avant toute écriture, reprise comprise, avec le permis de A.
#[test]
fn autre_flam_branchee_pendant_la_preparation_rien_n_est_ecrit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), true);
    write(&root.join(MDF_FILE), &mdf("2.4.2"));
    // Résidu d'un envoi interrompu sur B : la reprise n'y touche pas non plus.
    write(
        &root
            .join(TRANSFER_DIR)
            .join(format!("{OTHER}{INCOMING_SUFFIX}"))
            .join("info"),
        b"x",
    );
    write(&root.join("etc/library/list.new"), b"partiel");
    let before = snapshot(&root);
    let p = prepared(tmp.path(), 1);
    let result = send(&FaultyFs::new(), &root, &p, true);
    assert!(
        matches!(result, Err(TransferError::DeviceChanged)),
        "{result:?}"
    );
    assert_eq!(snapshot(&root), before);
    let message = AppError::from(result.unwrap_err()).to_string();
    assert!(message.contains("rien n'a été écrit"), "{message}");
}

/// Envoi dont le `.mdf` est modifié par `swap` une fois la copie relue, juste avant la
/// promotion (mise à jour de `NEW`).
fn send_swapping_mdf_before_promotion(
    root: &Path,
    p: &PreparedStory,
    swap: &dyn Fn(&Path),
) -> Result<TransferReport, TransferError> {
    let mdf_path = root.join(MDF_FILE);
    send_with(
        &FaultyFs::new(),
        &permit(),
        root,
        p,
        Some(NEW),
        &AtomicBool::new(false),
        &mut |e: Progress| {
            if e.phase == Phase::Verify && e.done == e.total {
                swap(&mdf_path);
            }
        },
    )
}

/// Changement de `.mdf` simulé : nom et action.
type MdfSwap = (&'static str, fn(&Path));

/// #577 : `.mdf` remplacé (ou disparu) entre la copie et la promotion : rien n'est installé,
/// l'ancienne version, l'index et le cache sont intacts ; la copie temporaire est laissée à la
/// reprise, qui la supprime au retour de la Flam.
#[test]
fn autre_flam_branchee_avant_la_promotion_rien_n_est_installe() {
    let swaps: [MdfSwap; 2] = [
        ("remplacé", |path: &Path| write(path, &mdf("2.4.2"))),
        // Appelé deux fois (dernier bloc relu, puis fin de la relecture).
        ("retiré", |path: &Path| {
            let _ = std::fs::remove_file(path);
        }),
    ];
    for (n, (what, swap)) in swaps.into_iter().enumerate() {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(&tmp.path().join(format!("{n}")), true);
        let old = files_of(&root.join("str").join(NEW));
        let list_before = std::fs::read(root.join("etc/library/list")).unwrap();
        let p = prepared(tmp.path(), 1);
        let result = send_swapping_mdf_before_promotion(&root, &p, &swap);
        assert!(
            matches!(result, Err(TransferError::DeviceChangedDuringSend)),
            "{what} : {result:?}"
        );
        assert_eq!(files_of(&root.join("str").join(NEW)), old, "{what}");
        assert_eq!(
            std::fs::read(root.join("etc/library/list")).unwrap(),
            list_before,
            "{what}"
        );
        assert!(root.join(LIBRARY_CACHE).exists(), "{what}");
        assert!(!previous_dir(&root, &id(NEW)).exists(), "{what}");
        assert_eq!(
            files_of(&incoming_dir(&root, &id(NEW))),
            files_of(&p.dir),
            "{what} : copie temporaire laissée à la reprise"
        );
        // Retour de la Flam A : la reprise supprime la copie temporaire.
        write(&root.join(MDF_FILE), &mdf_v1_real());
        restart(&root);
        assert!(!root.join(TRANSFER_DIR).exists(), "{what}");
        assert_eq!(files_of(&root.join("str").join(NEW)), old, "{what}");
    }
}

/// #577 : une entrée de `.flam-transfer` en échec (verrouillée) n'empêche pas la reprise des
/// index ; la première erreur est rendue ensuite.
#[test]
fn reprise_des_index_faite_malgre_une_entree_en_echec() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let locked = root
        .join(TRANSFER_DIR)
        .join(format!("{OTHER}{INCOMING_SUFFIX}"));
    write(&locked.join("info"), b"x");
    write(&root.join("etc/library/list.new"), b"partiel");
    let fs = FaultyFs {
        locked: Some(OTHER),
        ..FaultyFs::new()
    };
    let result = recover_device_with(&fs, &permit(), &root);
    assert!(
        matches!(&result, Err(TransferError::Io(e)) if e.kind() == io::ErrorKind::PermissionDenied),
        "{result:?}"
    );
    assert!(
        !root.join("etc/library/list.new").exists(),
        "reprise de l'index sautée"
    );
    assert!(locked.join("info").is_file());
}

/// #577 : un verrou passager sur `.mdf` (antivirus) à l'un des trois contrôles (entrée, avant la
/// promotion, avant l'index) est réessayé : l'envoi réussit.
#[test]
fn verrou_passager_sur_mdf_reessaye() {
    for first in 0..3 {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        let p = prepared(tmp.path(), 1);
        let fs = FaultyFs {
            mdf_locked: Some((first, 3)),
            ..FaultyFs::new()
        };
        let report = send(&fs, &root, &p, false)
            .unwrap_or_else(|e| panic!("lecture {first} verrouillée : {e:?}"));
        assert!(report.index_updated, "lecture {first}");
        assert_eq!(listed(&root), vec![id(OTHER), id(NEW)], "lecture {first}");
        assert_eq!(fs.sleeps.get(), 3, "lecture {first}");
        assert_eq!(fs.mdf_reads.get(), 6, "lecture {first}");
    }
}

/// #577 : `.mdf` toujours illisible après les nouveaux essais : refus selon l'instant.
/// Entrée : erreur d'entrée/sortie, rien d'écrit. Avant la promotion : rien d'installé, copie
/// temporaire laissée. Avant l'index : histoire en place hors index, index et cache intacts.
#[test]
fn mdf_illisible_apres_les_essais_refuse_selon_l_instant() {
    for first in 0..3 {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        let before = snapshot(&root);
        let list_before = std::fs::read(root.join("etc/library/list")).unwrap();
        let p = prepared(tmp.path(), 1);
        let fs = FaultyFs {
            mdf_locked: Some((first, 100)),
            ..FaultyFs::new()
        };
        let result = send(&fs, &root, &p, false);
        match first {
            0 => {
                assert!(matches!(result, Err(TransferError::Io(_))), "{result:?}");
                assert_eq!(snapshot(&root), before);
            }
            1 => {
                assert!(
                    matches!(result, Err(TransferError::DeviceChangedDuringSend)),
                    "{result:?}"
                );
                assert!(!root.join("str").join(NEW).exists());
                assert!(incoming_dir(&root, &id(NEW)).exists());
            }
            _ => {
                assert!(
                    matches!(result, Err(TransferError::DeviceChangedBeforeIndex)),
                    "{result:?}"
                );
                assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
            }
        }
        assert_eq!(
            std::fs::read(root.join("etc/library/list")).unwrap(),
            list_before,
            "lecture {first}"
        );
        assert!(root.join(LIBRARY_CACHE).exists(), "lecture {first}");
    }
}

/// Faux appareil dont le `.mdf` devient celui d'une autre Flam juste après la promotion.
fn swapping_after_promotion(root: &Path) -> FaultyFs {
    let mdf_path = root.join(MDF_FILE);
    FaultyFs {
        on_sync_dir: Some(Box::new(move || write(&mdf_path, &mdf("2.4.2")))),
        ..FaultyFs::new()
    }
}

/// #577 : Flam changée entre la promotion et l'écriture de l'index (nouvelle histoire, ou
/// dossier orphelin remplacé) : l'index n'est pas écrit (la liste lue sur A n'est jamais écrite
/// sur B). Sur A, l'histoire est complète hors index ; la reprise termine la mise de côté et un
/// nouvel envoi de la même histoire l'ajoute à l'index, sans perte.
#[test]
fn autre_flam_branchee_avant_l_index_rien_n_est_liste() {
    for orphan in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        if orphan {
            write(&root.join("str").join(NEW).join("info"), b"orphelin");
        }
        let list_before = std::fs::read(root.join("etc/library/list")).unwrap();
        let p = prepared(tmp.path(), 1);
        let result = send(&swapping_after_promotion(&root), &root, &p, false);
        assert!(
            matches!(result, Err(TransferError::DeviceChangedBeforeIndex)),
            "orphelin {orphan} : {result:?}"
        );
        assert_eq!(
            std::fs::read(root.join("etc/library/list")).unwrap(),
            list_before
        );
        assert!(root.join(LIBRARY_CACHE).exists());
        assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
        assert_eq!(previous_dir(&root, &id(NEW)).exists(), orphan);

        // Retour de la Flam A : reprise, puis nouvel envoi de la même histoire.
        write(&root.join(MDF_FILE), &mdf_v1_real());
        restart(&root);
        assert!(!root.join(TRANSFER_DIR).exists());
        assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
        let report = send(&FaultyFs::new(), &root, &p, false).unwrap();
        assert!(report.replaced && report.index_updated, "{report:?}");
        assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
        assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
        assert!(!root.join(TRANSFER_DIR).exists());
    }
}

/// #577 : mise à jour d'une histoire déjà listée, Flam changée après la promotion, renommages
/// forcés sur le support : l'envoi est réussi (rien à écrire dans l'index) mais signalé
/// (`device_changed`) ; ni le cache ni la mise de côté ne sont supprimés sur l'autre Flam ; la
/// reprise de la Flam d'origine s'en charge. Hors Windows, renommages jamais réputés forcés :
/// refus (voir le test suivant).
#[test]
fn mise_a_jour_reussie_sans_nettoyage_sur_une_autre_flam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), true);
    let p = prepared(tmp.path(), 1);
    let result = send(&swapping_after_promotion(&root), &root, &p, true);
    if !cfg!(windows) {
        assert!(
            matches!(result, Err(TransferError::DeviceChangedAfterUpdate)),
            "{result:?}"
        );
        return;
    }
    let report = result.unwrap();
    assert!(report.device_changed, "{report:?}");
    assert!(report.replaced && !report.index_updated && !report.cache_removed);
    assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
    assert!(root.join(LIBRARY_CACHE).exists());
    assert!(previous_dir(&root, &id(NEW)).exists());
    write(&root.join(MDF_FILE), &mdf_v1_real());
    restart(&root);
    assert!(!root.join(TRANSFER_DIR).exists());
    assert!(!root.join(LIBRARY_CACHE).exists());
    assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
}

/// #577 : mise à jour listée, Flam changée après la promotion et renommages NON confirmés sur le
/// support (`sync_dir` en échec) : la mise à jour a pu ne pas atteindre la Flam d'origine ; refus
/// `DeviceChangedAfterUpdate`, jamais « envoyée ». Sur A, la reprise termine l'état laissé.
#[test]
fn mise_a_jour_non_confirmee_sur_une_autre_flam_refusee() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), true);
    let p = prepared(tmp.path(), 1);
    let mut fs = swapping_after_promotion(&root);
    fs.sync_dir_fails = true;
    let result = send(&fs, &root, &p, true);
    assert!(
        matches!(result, Err(TransferError::DeviceChangedAfterUpdate)),
        "{result:?}"
    );
    let message = AppError::from(result.unwrap_err());
    assert_eq!(message.kind(), "device_changed");
    assert!(message.to_string().contains("peut-être faite"), "{message}");
    // Ici le renommage a bien eu lieu : nouvelle version en place, `.old` et cache laissés.
    assert_eq!(files_of(&root.join("str").join(NEW)), files_of(&p.dir));
    assert!(previous_dir(&root, &id(NEW)).exists());
    assert!(root.join(LIBRARY_CACHE).exists());
    write(&root.join(MDF_FILE), &mdf_v1_real());
    restart(&root);
    assert!(!root.join(TRANSFER_DIR).exists());
    assert!(!root.join(LIBRARY_CACHE).exists());
    assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
}

/// #577 : Flam changée après une mise à jour (`device_changed`) : la relecture carrier n'est pas
/// faite (elle lirait l'autre Flam) ; la conservation du couple reste rapportée.
#[test]
fn pas_de_relecture_carrier_sur_une_autre_flam() {
    let resolved = carrier::Resolved {
        keys: carrier::CarrierKeys::from_parts(&[1u8; 32], &[2u8; 32]).unwrap(),
        stored: true,
    };
    for changed in [false, true] {
        let mut report = TransferReport {
            id: NEW.into(),
            bytes: 1,
            files: 1,
            replaced: true,
            index_updated: false,
            cache_removed: false,
            carrier_readback: None,
            carrier_stored: None,
            recovery_failures: Vec::new(),
            library_copy: None,
            dir_synced: true,
            verified_uncached: true,
            device_changed: changed,
            installed_files: Vec::new(),
        };
        let called = Cell::new(false);
        carrier_outcome(&mut report, Some(&resolved), |_, _| {
            called.set(true);
            true
        });
        assert_eq!(called.get(), !changed, "changée {changed}");
        assert_eq!(report.carrier_readback, (!changed).then_some(true));
        assert_eq!(report.carrier_stored, Some(true));
    }
}

/// #577 : une copie complète `<uuid>.in` écrite pour la Flam A et retrouvée sur la Flam B est
/// supprimée par la reprise de B (permis de B), jamais promue ni listée.
#[test]
fn reprise_sur_b_d_une_copie_ecrite_pour_a_supprimee_jamais_promue() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let mdf_b = mdf("2.4.2");
    write(&root.join(MDF_FILE), &mdf_b);
    let p = prepared(tmp.path(), 1);
    for (name, data) in files_of(&p.dir) {
        write(&incoming_dir(&root, &id(NEW)).join(name), &data);
    }
    let permit_b = WritePermit::for_tests(FirmwareFamily::V2, &mdf_b);
    let report = recover_device_with(&FaultyFs::new(), &permit_b, &root).unwrap();
    assert_eq!(report.incomplete_removed, 1);
    assert!(!root.join("str").join(NEW).exists());
    assert!(!root.join(TRANSFER_DIR).exists());
    assert_eq!(listed(&root), vec![id(OTHER)]);
}

/// Un permis obtenu pour une Flam ne permet aucune écriture sur une autre (#317).
#[test]
fn reprise_refusee_avec_le_permis_d_une_autre_flam() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let other = tempfile::tempdir().unwrap();
    let other_root = device(other.path(), false);
    write(
        &other_root
            .join(TRANSFER_DIR)
            .join(format!("{NEW}.in"))
            .join("x"),
        b"x",
    );
    let permit = crate::device::guard::check_write(
        &root,
        &test_ctx(),
        crate::device::guard::WriteKind::Other,
    )
    .unwrap();
    let before = snapshot(&other_root);
    assert!(matches!(
        recover_with(&FaultyFs::new(), &permit, &other_root),
        Err(TransferError::InvalidRoot)
    ));
    assert_eq!(snapshot(&other_root), before);
}

#[test]
fn le_dossier_de_cache_scrute_suit_les_reglages() {
    // #326 : `device_status` ne relit les réglages que s'ils ont changé.
    let config_dir = tempfile::tempdir().unwrap();
    let first = config_dir.path().join("cache-a");
    let second = config_dir.path().join("cache-bb");
    let store = SettingsStore::new(config_dir.path());
    let set = |dir: &Path| {
        let mut settings = store.load().unwrap();
        settings.cache_dir = Some(dir.to_string_lossy().into_owned());
        store.update(settings).unwrap();
    };
    set(&first);
    let config = ConfigState::new(config_dir.path().to_path_buf());
    assert_eq!(polled_cache_root(&config), Some(first.clone()));
    assert!(first.is_dir());
    // Réglages inchangés : résolution réutilisée (le dossier supprimé n'est pas recréé). Il
    // contient les sous-dossiers marqués créés par `resolved_cache_dir` (#570).
    fs::remove_dir_all(&first).unwrap();
    assert_eq!(polled_cache_root(&config), Some(first.clone()));
    assert!(!first.exists());
    // Réglages enregistrés : résolu à neuf.
    set(&second);
    assert_eq!(polled_cache_root(&config), Some(second.clone()));
    assert_eq!(launcher_cache_root(&config), Some(second));
}

// --- Histoires STUdio au format Lunii (#542) ---------------------------------------------------

mod lunii {
    use super::*;
    use crate::device::carrier::tests::{fake_store, official_story, OFFICIAL};
    use crate::device::carrier::{CarrierConsent, WARNING_VERSION};
    use crate::device::prepare_lunii::PACK_SUFFIX;

    const INFO_TEXT: &[u8] =
        b"Histoire inventee\nHistoire inventee\n0\nHistoire inventee\nNo authors.";

    /// Nœud de `ni` : image, son, transitions « ok » et « home », commandes.
    type Node = (i32, i32, [i32; 3], [i32; 3], [u16; 5]);

    /// `ni` : en-tête de 512 octets puis les nœuds (note, § 3.2).
    fn ni_bytes(nodes: &[Node], images: u32, sounds: u32) -> Vec<u8> {
        let mut out = vec![0u8; 0x200];
        out[0..2].copy_from_slice(&1u16.to_le_bytes());
        out[2..4].copy_from_slice(&3u16.to_le_bytes());
        out[4..8].copy_from_slice(&0x200u32.to_le_bytes());
        out[8..12].copy_from_slice(&0x2Cu32.to_le_bytes());
        out[12..16].copy_from_slice(&(nodes.len() as u32).to_le_bytes());
        out[16..20].copy_from_slice(&images.to_le_bytes());
        out[20..24].copy_from_slice(&sounds.to_le_bytes());
        out[0x18] = 1;
        for (image, sound, ok, home, controls) in nodes {
            for v in [*image, *sound].iter().chain(ok).chain(home) {
                out.extend_from_slice(&v.to_le_bytes());
            }
            for c in controls {
                out.extend_from_slice(&c.to_le_bytes());
            }
            out.extend_from_slice(&[0, 0]);
        }
        out
    }

    fn asset_index(names: &[&str]) -> Vec<u8> {
        names
            .iter()
            .flat_map(|n| [b"000\\".as_slice(), n.as_bytes()].concat())
            .collect()
    }

    fn sound() -> Vec<u8> {
        (0..200_000u32).map(|i| (i * 7 % 253) as u8).collect()
    }

    /// Entrées d'un pack clair inventé de l'histoire `NEW` : 3 nœuds, 2 images, 1 son.
    fn entries() -> Vec<(&'static str, Vec<u8>)> {
        let none = [-1, -1, -1];
        let nodes: [Node; 3] = [
            (0, 0, [0, 2, 0], none, [1, 1, 0, 0, 0]),
            (1, -1, [2, 1, 0], [0, 2, 1], [0, 1, 1, 1, 0]),
            (-1, 0, none, [2, 1, 0], [0, 0, 1, 1, 1]),
        ];
        let li: Vec<u8> = [1i32, 2, 0].iter().flat_map(|v| v.to_le_bytes()).collect();
        vec![
            (
                "uuid.bin",
                uuid::Uuid::parse_str(NEW).unwrap().into_bytes().to_vec(),
            ),
            ("_metadata.json", b"{\"title\":\"x\"}".to_vec()),
            ("_thumbnail.png", b"png".to_vec()),
            ("ni", ni_bytes(&nodes, 2, 1)),
            ("nm", Vec::new()),
            ("li", li),
            ("ri", asset_index(&["AAAAAAAA", "0000000B"])),
            ("si", asset_index(&["CCCCCCCC"])),
            ("info", INFO_TEXT.to_vec()),
            ("img/thumbnail.lif", vec![7u8; 700]),
            (
                "rf/000/AAAAAAAA",
                (0..600u32).map(|i| (i * 3 % 251) as u8).collect(),
            ),
            ("rf/000/0000000B", vec![0x42; 100]),
            ("sf/000/CCCCCCCC", sound()),
        ]
    }

    /// Pack `<NEW>-1.lunii.pk` dans `dir`, sans les entrées `skip`.
    fn lunii_pk(dir: &Path, skip: &[&str]) -> PathBuf {
        use zip::write::SimpleFileOptions;
        use zip::{CompressionMethod, ZipWriter};

        let pk = dir.join(format!("{}-1{PACK_SUFFIX}", id(NEW)));
        let mut zip = ZipWriter::new(File::create(&pk).unwrap());
        let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
        for (name, data) in entries() {
            if skip.contains(&name) {
                continue;
            }
            zip.start_file(name, options).unwrap();
            zip.write_all(&data).unwrap();
        }
        zip.finish().unwrap();
        pk
    }

    const EXPECTED: [&str; 11] = [
        "img/thumbnail.lif",
        "info",
        "key",
        "li",
        "ni",
        "nm",
        "rf/000/0000000B",
        "rf/000/AAAAAAAA",
        "ri",
        "sf/000/CCCCCCCC",
        "si",
    ];

    fn consent() -> CarrierConsent {
        CarrierConsent {
            text_version: WARNING_VERSION,
            accepted_at: "2026-11-07T10:00:00Z".into(),
        }
    }

    fn send(
        root: &Path,
        pk: &Path,
        work: &Path,
        replace: Option<&str>,
        carrier: Option<&CarrierRequest<'_>>,
    ) -> Result<TransferReport, TransferError> {
        send_from_library_with(
            &FaultyFs::new(),
            root,
            pk,
            work,
            replace,
            carrier,
            &test_ctx(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
    }

    /// Règle unique de la bibliothèque (#556) : suffixe exact, sensible à la casse, d'après le
    /// seul nom du fichier ; un nom inconnu (`None`) est envoyé comme une histoire Flam.
    #[test]
    fn format_choisi_d_apres_le_nom_du_fichier() {
        use crate::library::StoryFormat;
        for (name, expected) in [
            ("abc-1.lunii.pk", Some(StoryFormat::Lunii)),
            ("é.lunii.pk", Some(StoryFormat::Lunii)),
            ("abc-1.plain.pk", Some(StoryFormat::Flam)),
            ("abc-1.lunii.pk.plain.pk", Some(StoryFormat::Flam)),
            ("ABC-1.LUNII.PK", None),
            ("abc-1.Lunii.pk", None),
            (".lunii.pk", None),
            ("histoire.pk", None),
            ("éé", None),
        ] {
            assert_eq!(
                StoryFormat::of_path(&Path::new("bib").join(name)),
                expected,
                "{name}"
            );
        }
        // Seul le nom du fichier compte, pas le dossier.
        assert_eq!(
            StoryFormat::of_path(&Path::new("x.lunii.pk").join("a.plain.pk")),
            Some(StoryFormat::Flam)
        );
        assert_eq!(StoryFormat::of_path(Path::new("x.lunii.pk/..")), None);
    }

    #[test]
    fn envoi_d_un_pack_lunii_sur_une_flam_2x() {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        write(&root.join(MDF_FILE), &mdf("2.4.2"));
        // Copie interrompue d'un envoi précédent : reprise faite avant l'envoi (2.x).
        write(
            &incoming_dir(&root, &id(NEW)).join("ni"),
            b"copie interrompue",
        );
        let pk = lunii_pk(tmp.path(), &[]);
        let work = tmp.path().join("cache").join(WORK_SUBDIR);

        let report = send(&root, &pk, &work, None, None).unwrap();
        assert_eq!(report.id, NEW);
        assert_eq!(report.files, EXPECTED.len());
        assert_eq!(report.carrier_readback, None);
        assert_eq!(report.carrier_stored, None);
        assert!(report.index_updated && report.cache_removed && !report.replaced);
        assert!(report.recovery_failures.is_empty());

        let installed = files_of(&root.join("str").join(NEW));
        let names: Vec<&str> = installed.keys().map(String::as_str).collect();
        assert_eq!(names, EXPECTED);
        let source: BTreeMap<&str, Vec<u8>> = entries().into_iter().collect();
        for name in ["ni", "nm", "info", "img/thumbnail.lif"] {
            assert_eq!(installed[name], source[name], "{name} en clair");
        }
        let keys = test_keys();
        assert_eq!(installed["key"], keys.key_file().to_vec());
        for name in ["li", "ri", "si", "rf/000/AAAAAAAA", "rf/000/0000000B"] {
            let plain = &source[name];
            let n = plain.len().min(16);
            assert_ne!(installed[name][..n], plain[..n], "{name} chiffré");
            let clear = keys.decrypt_lunii_file(name, &installed[name]).unwrap();
            assert_eq!(&clear[..plain.len()], &plain[..], "{name}");
        }
        let clear = keys
            .decrypt_lunii_file("sf/000/CCCCCCCC", &installed["sf/000/CCCCCCCC"])
            .unwrap();
        assert_eq!(clear, sound());

        assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
        assert!(!root.join(LIBRARY_CACHE).exists(), "cache supprimé");
        assert!(
            !root.join(TRANSFER_DIR).exists(),
            "reprise et nettoyage faits"
        );
        assert!(
            !work.join("str").join(NEW).exists(),
            "préparation supprimée"
        );
        assert_eq!(
            files_of(&root.join("str").join(OTHER))["info"],
            b"autre".to_vec(),
            "autre histoire intacte"
        );
    }

    /// § 5.8 : la version Lunii d'une histoire remplace sa version Flam (même UUID) seulement
    /// par « Remplacer ».
    #[test]
    fn la_version_lunii_remplace_la_version_flam_sur_demande() {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), true);
        write(&root.join(MDF_FILE), &mdf("2.4.2"));
        let pk = lunii_pk(tmp.path(), &[]);
        let work = tmp.path().join("cache").join(WORK_SUBDIR);

        let before = snapshot(&root);
        let err = send(&root, &pk, &work, None, None).unwrap_err();
        assert!(
            matches!(err, TransferError::AlreadyInstalled { hidden: false, .. }),
            "{err:?}"
        );
        assert_eq!(snapshot(&root), before, "rien n'est écrit");

        let report = send(&root, &pk, &work, Some(NEW), None).unwrap();
        assert!(report.replaced && !report.index_updated);
        let installed = files_of(&root.join("str").join(NEW));
        let names: Vec<&str> = installed.keys().map(String::as_str).collect();
        assert_eq!(names, EXPECTED, "ancienne version Flam retirée");
        assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
        assert!(!root.join(TRANSFER_DIR).exists());
    }

    /// Flam 1.x : refus avant toute écriture, reprise comprise, sans jamais consulter le mode
    /// carrier, même avec un consentement et une histoire officielle utilisables.
    #[test]
    fn refus_en_firmware_1x_sans_ecriture_ni_carrier() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("flam");
        write(&root.join(MDF_FILE), &mdf_v1_real());
        official_story(&root, "str", OFFICIAL, true);
        write(&root.join(LIBRARY_CACHE), b"cache");
        write(&root.join("etc/library/list"), &serialize(&[id(OFFICIAL)]));
        write(&root.join("etc/library/list.hidden"), b"");
        // Résidus de transferts interrompus : la reprise ne doit pas y toucher.
        write(
            &incoming_dir(&root, &id(NEW)).join("ni"),
            b"copie interrompue",
        );
        write(
            &previous_dir(&root, &id(OTHER)).join("info"),
            b"ancienne version",
        );
        let pk = lunii_pk(tmp.path(), &[]);
        let work = tmp.path().join("cache").join(WORK_SUBDIR);
        let keys_dir = tmp.path().join("cles");
        let store = fake_store(&keys_dir);
        let c = consent();
        let before = snapshot(&root);

        let choices = [
            None,
            Some(CarrierChoice::Device {
                uuid: OFFICIAL.into(),
            }),
            Some(CarrierChoice::Stored),
        ];
        for choice in choices {
            let request = choice.map(|choice| CarrierRequest {
                consent: Some(&c),
                choice,
                remember: true,
                store: &store,
            });
            for replace in [None, Some(NEW)] {
                let err = send(&root, &pk, &work, replace, request.as_ref()).unwrap_err();
                assert!(matches!(err, TransferError::LuniiNeedsV2), "{err:?}");
                let app = AppError::from(err);
                assert_eq!(app.kind(), "lunii_needs_v2");
                assert_eq!(app.to_string(), LUNII_NEEDS_V2_MESSAGE);
                assert_eq!(snapshot(&root), before, "Flam inchangée, reprise comprise");
                assert!(!work.exists(), "rien de préparé");
                assert!(!keys_dir.exists(), "aucun couple carrier conservé");
            }
        }

        // Une histoire Flam sur la même Flam passe toujours par le mode carrier (non-régression).
        let flam_pk = plain_pk(tmp.path());
        let err = send(&root, &flam_pk, &work, None, None).unwrap_err();
        assert!(
            matches!(err, TransferError::Carrier(CarrierError::CarrierRequired)),
            "{err:?}"
        );
    }

    /// Firmware non pris en charge (3.x) : la garde commune refuse d'abord, comme pour une
    /// histoire Flam.
    #[test]
    fn firmware_3x_refuse_par_la_garde() {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        write(&root.join(MDF_FILE), &mdf("3.0.0"));
        let pk = lunii_pk(tmp.path(), &[]);
        let work = tmp.path().join("cache").join(WORK_SUBDIR);
        let before = snapshot(&root);
        let err = send(&root, &pk, &work, None, None).unwrap_err();
        assert!(
            matches!(
                err,
                TransferError::Refused(WriteRefused::UnsupportedFirmware { .. })
            ),
            "{err:?}"
        );
        assert_eq!(snapshot(&root), before);
    }

    /// Pack de la bibliothèque abîmé : refus à la préparation, message fixe vers l'interface
    /// (sans nom d'entrée), Flam inchangée, reprise non faite.
    #[test]
    fn pack_lunii_invalide_rien_n_est_ecrit() {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        write(&root.join(MDF_FILE), &mdf("2.4.2"));
        write(
            &incoming_dir(&root, &id(NEW)).join("ni"),
            b"copie interrompue",
        );
        let pk = lunii_pk(tmp.path(), &["sf/000/CCCCCCCC"]);
        let work = tmp.path().join("cache").join(WORK_SUBDIR);
        let before = snapshot(&root);
        let err = send(&root, &pk, &work, None, None).unwrap_err();
        assert!(
            matches!(err, TransferError::Prepare(PrepareError::LuniiInvalid(_))),
            "{err:?}"
        );
        let app = AppError::from(err);
        assert_eq!(app.kind(), "validation");
        let message = app.to_string();
        assert!(message.contains("histoire STUdio"), "{message}");
        assert!(!message.contains("CCCCCCCC"), "{message}");
        assert_eq!(snapshot(&root), before);
        assert!(!work.join("str").join(NEW).exists());

        let too_large = AppError::from(TransferError::Prepare(PrepareError::LuniiTooLarge(
            "rf/000/AAAAAAAA".into(),
        )));
        assert_eq!(too_large.kind(), "validation");
        assert!(!too_large.to_string().contains("AAAAAAAA"));
        // Défense en profondeur : un refus 1.x venu de la préparation garde son type.
        assert_eq!(
            AppError::from(TransferError::Prepare(PrepareError::LuniiNeedsV2)).kind(),
            "lunii_needs_v2"
        );
    }
}

/// #591 : les restes d'un envoi interrompu par un arrêt brutal (chantier `.partiel-*` et
/// préparation terminée de `str/`) sont supprimés au prochain envoi, mais seulement dans un
/// dossier de travail que l'application possède (marqueur) ; un homonyme sans marqueur n'est pas
/// nettoyé.
#[test]
fn les_restes_d_un_arret_brutal_sont_nettoyes_au_prochain_envoi_si_le_dossier_est_marque() {
    for owned in [true, false] {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        write(&root.join(MDF_FILE), &mdf("2.4.2"));
        let pk = plain_pk(tmp.path());
        let cache = tmp.path().join("cache");
        let work = if owned {
            crate::cache::ensure_subdir(&cache, WORK_SUBDIR).unwrap()
        } else {
            cache.join(WORK_SUBDIR)
        };
        let staging = work.join(".partiel-abandonne");
        let prepared = work
            .join("str")
            .join("11111111-2222-3333-4444-555555555555");
        write(&staging.join("info"), b"x");
        write(&prepared.join("key"), b"k");
        // Restes d'un arrêt brutal, anciens ; un autre, récent, peut être celui d'une autre
        // session qui prépare dans le même dossier de cache : jamais touché.
        let aged = crate::device::prepare::LEFTOVER_MIN_AGE + std::time::Duration::from_secs(60);
        let young = work.join(".partiel-en-cours");
        write(&young.join("info"), b"x");
        crate::device::prepare::backdate(&staging, aged);
        crate::device::prepare::backdate(&prepared, aged);

        send_from_library_with(
            &FaultyFs::new(),
            &root,
            &pk,
            &work,
            None,
            None,
            &test_ctx(),
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();

        assert_eq!(staging.exists(), !owned, "chantier, marqué = {owned}");
        assert_eq!(prepared.exists(), !owned, "préparation, marqué = {owned}");
        assert!(young.is_dir(), "reste récent gardé");
        assert!(!work.join("str").join(NEW).exists(), "envoi nettoyé");
        assert_eq!(crate::cache::is_owned(&work), owned);
    }
}

/// #609 : l'interface transmet l'empreinte de la Flam affichée au moment du clic. Une autre
/// Flam sous la même lettre : refus `device_changed` avant toute écriture. Rien n'est créé,
/// modifié ni supprimé sur la Flam branchée (ni la reprise d'un transfert interrompu, ni celle
/// d'un index), ni dans le dossier de travail (restes anciens d'un arrêt brutal, qu'un envoi
/// accepté supprimerait). Avec l'empreinte de la Flam branchée, l'envoi se fait comme avant.
#[test]
fn une_autre_flam_que_celle_du_clic_est_refusee_avant_toute_ecriture() {
    use crate::device::guard::device_fingerprint;
    let tmp = tempfile::tempdir().unwrap();
    // Flam du clic (A), puis une autre (B) sous la même racine : autre numéro de série.
    let mdf_a = mdf("2.4.2");
    let mut mdf_b = mdf_a.clone();
    mdf_b[MDF_SERIAL_OFFSET + 5] ^= 0x01;
    let clicked = device_fingerprint(&mdf_a).as_str().to_string();
    let root = device(tmp.path(), false);
    write(&root.join(MDF_FILE), &mdf_b);
    // Restes sur B : transfert interrompu, écriture d'index interrompue.
    write(
        &root.join(TRANSFER_DIR).join(format!("{NEW}.in")).join("x"),
        b"x",
    );
    write(
        &root
            .join(TRANSFER_DIR)
            .join(format!("{OTHER}.old"))
            .join("y"),
        b"y",
    );
    write(&root.join("etc/library/list.new"), b"x");
    let pk = plain_pk(tmp.path());
    let cache = tmp.path().join("cache");
    let work = crate::cache::ensure_subdir(&cache, WORK_SUBDIR).unwrap();
    let staging = work.join(".partiel-abandonne");
    write(&staging.join("info"), b"x");
    let aged = crate::device::prepare::LEFTOVER_MIN_AGE + std::time::Duration::from_secs(60);
    crate::device::prepare::backdate(&staging, aged);

    let send = |expected: ExpectedDevice, replace: Option<&str>| {
        send_from_library_with(
            &FaultyFs::new(),
            &root,
            &pk,
            &work,
            replace,
            None,
            &test_ctx().expecting(expected),
            &AtomicBool::new(false),
            &mut |_| {},
        )
    };
    let (device_before, work_before) = (snapshot(&root), snapshot(&work));
    // Flam du clic affichée avec son empreinte, ou affichée sans empreinte : même refus.
    for (expected, replace) in [
        (ExpectedDevice::Fingerprint(clicked.clone()), None),
        (ExpectedDevice::Fingerprint(clicked.clone()), Some(NEW)),
        (ExpectedDevice::Unidentified, None),
    ] {
        let err = send(expected, replace).unwrap_err();
        assert!(
            matches!(err, TransferError::Refused(WriteRefused::DeviceChanged)),
            "{err:?}"
        );
        let message = crate::AppError::from(err);
        assert_eq!(message.kind(), "device_changed");
        assert!(message.to_string().contains("rien n'a été écrit"));
        assert!(!message.to_string().contains(&clicked), "{message}");
        assert_eq!(snapshot(&root), device_before, "Flam branchée intacte");
        assert_eq!(snapshot(&work), work_before, "dossier de travail intact");
    }
    // La reprise au branchement reçoit la même empreinte : rien n'est repris sur B.
    let lock = crate::device::lock::DeviceLock::new();
    let guard = lock.try_begin(Operation::Transfer).unwrap();
    let ctx = test_ctx().expecting(ExpectedDevice::Fingerprint(clicked.clone()));
    let err = recover_device(&guard, &root, &ctx).unwrap_err();
    assert_eq!(crate::AppError::from(err).kind(), "device_changed");
    let ctx = test_ctx().expecting(ExpectedDevice::Unidentified);
    let err = recover_device(&guard, &root, &ctx).unwrap_err();
    assert_eq!(crate::AppError::from(err).kind(), "device_changed");
    assert_eq!(snapshot(&root), device_before);
    drop(guard);

    // Empreinte de la Flam branchée : l'envoi se fait (reprise et nettoyage compris).
    let own = device_fingerprint(&mdf_b).as_str().to_string();
    std::fs::remove_file(root.join("etc/library/list.new")).unwrap();
    let report = send(ExpectedDevice::Fingerprint(own), None).unwrap();
    assert_eq!(report.id, NEW);
    assert!(!report.device_changed);
    assert_eq!(listed(&root), vec![id(OTHER), id(NEW)]);
    assert!(!root.join(TRANSFER_DIR).exists(), "reprise faite");
    assert!(!staging.exists(), "restes du dossier de travail supprimés");
}

/// Entrée de corbeille d'un retrait terminé : nommée comme `remove` les nomme.
fn leftover_trash(root: &Path, name: &str) {
    let trash = root.join(crate::device::remove::TRASH_DIR);
    write(&trash.join(name).join("ni"), &[7u8; 4096]);
}

const TRASH_ENTRY: &str = "11111111-2222-3333-4444-555555555555-00000000000000000000000000000000";

/// #577 : la corbeille d'un retrait précédent occupe la place : elle est vidée avant le refus,
/// l'espace relu, et l'envoi réussit. Aucune histoire ni l'index n'est touché.
#[test]
fn corbeille_videe_pour_liberer_la_place() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let p = prepared(tmp.path(), 4);
    leftover_trash(&root, TRASH_ENTRY);
    let other = snapshot(&root.join("str").join(OTHER));
    let fs = FaultyFs {
        free: 0,
        free_without_trash: Some(PLENTY),
        ..FaultyFs::new()
    };
    let report = send(&fs, &root, &p, false).unwrap();
    assert!(report.index_updated);
    assert!(!root.join(crate::device::remove::TRASH_DIR).exists());
    assert_eq!(snapshot(&root.join("str").join(OTHER)), other);
    assert_eq!(listed(&root), [id(OTHER), id(NEW)]);
}

/// #577 : espace toujours insuffisant après le vidage (entrée inconnue laissée, ou place
/// vraiment manquante) : refus `NotEnoughSpace`. Seule l'entrée reconnue de la corbeille a été
/// supprimée ; index, histoires et entrée inconnue sont intacts.
#[test]
fn espace_toujours_insuffisant_apres_le_vidage_de_la_corbeille() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let p = prepared(tmp.path(), 4);
    leftover_trash(&root, TRASH_ENTRY);
    leftover_trash(&root, "inconnu");
    let mut expected = snapshot(&root);
    expected.retain(|k, _| !k.contains(TRASH_ENTRY));
    let fs = FaultyFs {
        free: 0,
        free_without_trash: Some(PLENTY),
        ..FaultyFs::new()
    };
    let err = send(&fs, &root, &p, false).unwrap_err();
    assert!(
        matches!(err, TransferError::NotEnoughSpace { .. }),
        "{err:?}"
    );
    assert_eq!(snapshot(&root), expected);
}

/// #577 : la corbeille n'est pas vidée quand l'espace suffit (durée de l'envoi inchangée).
#[test]
fn corbeille_laissee_quand_la_place_suffit() {
    let tmp = tempfile::tempdir().unwrap();
    let root = device(tmp.path(), false);
    let p = prepared(tmp.path(), 4);
    leftover_trash(&root, TRASH_ENTRY);
    send(&FaultyFs::new(), &root, &p, false).unwrap();
    assert!(root
        .join(crate::device::remove::TRASH_DIR)
        .join(TRASH_ENTRY)
        .is_dir());
}

/// #577 : `.mdf` relu juste avant le vidage. Une autre Flam ou un `.mdf` absent : refus
/// `device_changed` au message exact, rien n'est supprimé. Un `.mdf` illisible après les nouveaux
/// essais (verrou persistant) : erreur d'entrée/sortie, pas un changement de Flam.
#[test]
fn flam_echangee_avant_le_vidage_de_la_corbeille_rien_n_est_supprime() {
    for replaced in [Some((1, true)), Some((1, false)), None] {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        let p = prepared(tmp.path(), 4);
        leftover_trash(&root, TRASH_ENTRY);
        let before = snapshot(&root);
        // Lecture 0 : entrée de l'envoi ; lecture 1 : avant le vidage.
        let fs = FaultyFs {
            free: 0,
            free_without_trash: Some(PLENTY),
            mdf_replaced_from: replaced,
            mdf_locked: replaced.is_none().then_some((1, 100)),
            ..FaultyFs::new()
        };
        let err = send(&fs, &root, &p, false).unwrap_err();
        match replaced {
            Some(_) => {
                assert!(
                    matches!(err, TransferError::DeviceChangedBeforeTrash),
                    "{err:?}"
                );
                assert!(!err.to_string().contains("rien n'a été écrit"), "{err}");
                let app: AppError = err.into();
                assert_eq!(app.kind(), "device_changed");
            }
            None => {
                assert!(matches!(err, TransferError::Io(_)), "{err:?}");
                let app: AppError = err.into();
                assert_eq!(app.kind(), "io");
            }
        }
        assert_eq!(snapshot(&root), before);
    }
}

/// #577 : sans entrée reconnue dans la corbeille, l'espace insuffisant est refusé tout de suite,
/// sans relire `.mdf` (un `.mdf` illisible ne devient pas un faux changement de Flam).
#[test]
fn espace_insuffisant_sans_corbeille_ne_relit_pas_mdf() {
    for unrecognized in [false, true] {
        let tmp = tempfile::tempdir().unwrap();
        let root = device(tmp.path(), false);
        let p = prepared(tmp.path(), 4);
        if unrecognized {
            leftover_trash(&root, "inconnu");
        }
        let before = snapshot(&root);
        let fs = FaultyFs {
            free: 0,
            mdf_locked: Some((1, 100)),
            ..FaultyFs::new()
        };
        let err = send(&fs, &root, &p, false).unwrap_err();
        assert!(
            matches!(err, TransferError::NotEnoughSpace { .. }),
            "{err:?}"
        );
        assert_eq!(fs.mdf_reads.get(), 1, "seule la lecture d'entrée");
        assert_eq!(snapshot(&root), before);
    }
}
