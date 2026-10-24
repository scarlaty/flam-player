//! Cache des MP3 convertis (#460) : clé, lecture, écriture atomique, invalidation, éviction,
//! injection de pannes (fichier tronqué ou altéré, métadonnées illisibles, disque plein, purge
//! de la source pendant une création, annulation, jonction sur le chemin du cache).

use std::sync::Mutex;
use std::time::Duration;

use super::*;

/// Verrous de test : générations incrémentées par [`TestLocks::purge`].
#[derive(Default)]
pub(crate) struct TestLocks {
    gens: Mutex<HashMap<String, u64>>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

impl TestLocks {
    /// Simule `PodcastService::purge_source` : génération incrémentée, dossier supprimé.
    pub(crate) fn purge(&self, cache_root: &Path, source_id: &str) {
        *self
            .gens
            .lock()
            .unwrap()
            .entry(source_id.to_string())
            .or_default() += 1;
        let _ = std::fs::remove_dir_all(source_dir(cache_root, source_id).unwrap());
    }
}

impl SourceLocks for TestLocks {
    fn generation(&self, source_id: &str) -> u64 {
        self.gens
            .lock()
            .unwrap()
            .get(source_id)
            .copied()
            .unwrap_or_default()
    }

    fn lock<'a>(
        &'a self,
        source_id: &'a str,
        generation: u64,
    ) -> BoxFuture<'a, Option<OwnedMutexGuard<()>>> {
        Box::pin(async move {
            let mutex = self
                .locks
                .lock()
                .unwrap()
                .entry(source_id.to_string())
                .or_default()
                .clone();
            let guard = mutex.lock_owned().await;
            (self.generation(source_id) == generation).then_some(guard)
        })
    }
}

pub(crate) fn episode(guid: &str) -> PodcastEpisode {
    PodcastEpisode {
        guid: guid.into(),
        title: format!("Titre {guid}"),
        description: None,
        pub_date: None,
        duration: Some(60),
        author: None,
        image: Some(format!("https://img.example.test/{guid}.png?token=secret")),
        enclosure_url: format!("https://audio.example.test/{guid}.mp3?token=secret"),
        enclosure_type: "audio/mpeg".into(),
        enclosure_length: Some(1000),
        explicit: None,
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    locks: Arc<TestLocks>,
}

impl Fixture {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
            locks: Arc::new(TestLocks::default()),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().join("cache")
    }

    fn work(&self) -> PathBuf {
        let w = self.dir.path().join("work");
        std::fs::create_dir_all(&w).unwrap();
        w
    }

    fn access(&self) -> CacheAccess {
        CacheAccess::new(
            self.locks.clone(),
            self.root(),
            ["s1".to_string(), "s2".to_string()],
        )
    }

    fn mp3_dir(&self, source: &str) -> PathBuf {
        source_dir(&self.root(), source).unwrap().join(CACHE_DIR)
    }

    /// MP3 converti et pochette dans le dossier de travail.
    fn converted(&self, name: &str, body: &[u8]) -> (PathBuf, PathBuf) {
        let mp3 = self.work().join(format!("{name}.mp3"));
        std::fs::write(&mp3, body).unwrap();
        let img = self.work().join(format!("{name}.img"));
        std::fs::write(&img, b"\x89PNG image").unwrap();
        (mp3, img)
    }

    async fn store(&self, access: &CacheAccess, source: &str, ep: &PodcastEpisode, body: &[u8]) {
        let (mp3, img) = self.converted(&ep.guid, body);
        let n = access
            .store(
                vec![Store {
                    key: EntryKey::of(source, ep),
                    mp3,
                    image: Some(img),
                }],
                100,
            )
            .await;
        assert_eq!(n, 1);
    }

    fn wanted(&self, rank: usize, source: &str, ep: &PodcastEpisode) -> Wanted {
        Wanted {
            rank,
            key: EntryKey::of(source, ep),
            mp3_dest: self.work().join(format!("cache-{rank:04}.mp3")),
            image_dest: self.work().join(format!("cache-{rank:04}.img")),
        }
    }

    fn files(&self, source: &str) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.mp3_dir(source))
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }
}

const BODY: &[u8] = b"ID3 audio converti";

#[test]
fn la_cle_depend_de_l_enclosure_et_jamais_de_l_url_en_clair() {
    let base = episode("g1");
    let key = EntryKey::of("s1", &base);
    assert_eq!(key.episode_id, episode_id("g1"));
    assert!(!format!("{key:?}").contains("example.test"));
    for changed in [
        PodcastEpisode {
            enclosure_url: "https://audio.example.test/autre.mp3".into(),
            ..base.clone()
        },
        PodcastEpisode {
            enclosure_length: Some(1001),
            ..base.clone()
        },
        PodcastEpisode {
            enclosure_length: None,
            ..base.clone()
        },
        PodcastEpisode {
            enclosure_type: "audio/aac".into(),
            ..base.clone()
        },
    ] {
        let other = EntryKey::of("s1", &changed);
        assert_eq!(other.episode_id, key.episode_id);
        assert_ne!(other.fingerprint, key.fingerprint, "{changed:?}");
    }
    // La pochette a sa propre empreinte.
    let no_image = EntryKey::of(
        "s1",
        &PodcastEpisode {
            image: None,
            ..base.clone()
        },
    );
    assert_eq!(no_image.fingerprint, key.fingerprint);
    assert_eq!(no_image.image_fingerprint, None);
    assert!(key.image_fingerprint.is_some());
}

#[tokio::test]
async fn ecriture_puis_lecture_place_le_mp3_et_la_pochette() {
    let fx = Fixture::new();
    let access = fx.access();
    let ep = episode("g1");
    fx.store(&access, "s1", &ep, BODY).await;
    let id = episode_id("g1");
    assert_eq!(
        fx.files("s1"),
        [
            format!("{id}.img"),
            format!("{id}.json"),
            format!("{id}.mp3")
        ]
    );
    // Aucune URL sur le disque (D11).
    let meta = std::fs::read_to_string(fx.mp3_dir("s1").join(format!("{id}.json"))).unwrap();
    assert!(!meta.contains("example.test") && !meta.contains("token"));

    let hits = access.lookup(vec![fx.wanted(3, "s1", &ep)], 200).await;
    assert_eq!(hits.len(), 1);
    let hit = &hits[0];
    assert_eq!(hit.rank, 3);
    assert_eq!(hit.mp3_bytes, BODY.len() as u64);
    assert_eq!(std::fs::read(&hit.mp3).unwrap(), BODY);
    assert_eq!(
        std::fs::read(hit.image.as_ref().unwrap()).unwrap(),
        b"\x89PNG image"
    );
    // Date d'usage renouvelée.
    let meta: EntryMeta = serde_json::from_slice(
        &std::fs::read(fx.mp3_dir("s1").join(format!("{id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!((meta.stored_at, meta.used_at), (100, 200));
    // Supprimer le fichier du dossier de travail ne touche pas au cache (lien physique).
    std::fs::remove_file(&hit.mp3).unwrap();
    assert_eq!(
        access
            .lookup(vec![fx.wanted(3, "s1", &ep)], 300)
            .await
            .len(),
        1
    );
    // Autre source, autre épisode : absents.
    assert!(access
        .lookup(
            vec![fx.wanted(0, "s2", &ep), fx.wanted(1, "s1", &episode("g9"))],
            300
        )
        .await
        .is_empty());
}

#[tokio::test]
async fn episode_modifie_invalide_l_entree() {
    let fx = Fixture::new();
    let access = fx.access();
    let ep = episode("g1");
    fx.store(&access, "s1", &ep, BODY).await;
    let changed = PodcastEpisode {
        enclosure_url: "https://audio.example.test/g1-v2.mp3".into(),
        ..ep.clone()
    };
    assert!(access
        .lookup(vec![fx.wanted(0, "s1", &changed)], 200)
        .await
        .is_empty());
    assert!(fx.files("s1").is_empty(), "entrée périmée supprimée");
    // Remise en cache avec la nouvelle empreinte : servie.
    fx.store(&access, "s1", &changed, b"ID3 v2").await;
    let hits = access.lookup(vec![fx.wanted(0, "s1", &changed)], 300).await;
    assert_eq!(std::fs::read(&hits[0].mp3).unwrap(), b"ID3 v2");
}

#[tokio::test]
async fn pochette_changee_seule_retiree() {
    let fx = Fixture::new();
    let access = fx.access();
    let ep = episode("g1");
    fx.store(&access, "s1", &ep, BODY).await;
    let new_image = PodcastEpisode {
        image: Some("https://img.example.test/nouvelle.png".into()),
        ..ep.clone()
    };
    let hits = access
        .lookup(vec![fx.wanted(0, "s1", &new_image)], 200)
        .await;
    assert_eq!(hits.len(), 1, "le MP3 reste servi");
    assert_eq!(hits[0].image, None);
    let id = episode_id("g1");
    assert_eq!(fx.files("s1"), [format!("{id}.json"), format!("{id}.mp3")]);
}

/// Panne : MP3 tronqué, altéré (même taille), métadonnées illisibles ou d'un autre format,
/// MP3 absent : l'entrée n'est jamais servie et elle est supprimée.
#[tokio::test]
async fn fichier_tronque_ou_altere_jamais_servi() {
    let id = episode_id("g1");
    type Damage = fn(&Path, &str);
    let damages: [(&str, Damage); 5] = [
        ("tronqué", |dir, id| {
            let f = std::fs::OpenOptions::new()
                .write(true)
                .open(dir.join(format!("{id}.mp3")))
                .unwrap();
            f.set_len(5).unwrap();
        }),
        ("altéré", |dir, id| {
            let path = dir.join(format!("{id}.mp3"));
            let mut bytes = std::fs::read(&path).unwrap();
            bytes[4] ^= 0xff;
            std::fs::write(&path, bytes).unwrap();
        }),
        ("json illisible", |dir, id| {
            std::fs::write(dir.join(format!("{id}.json")), b"{ pas du json").unwrap();
        }),
        ("autre format", |dir, id| {
            let path = dir.join(format!("{id}.json"));
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("\"format\": 1", "\"format\": 99");
            std::fs::write(&path, text).unwrap();
        }),
        ("mp3 absent", |dir, id| {
            std::fs::remove_file(dir.join(format!("{id}.mp3"))).unwrap();
        }),
    ];
    for (what, damage) in damages {
        let fx = Fixture::new();
        let access = fx.access();
        let ep = episode("g1");
        fx.store(&access, "s1", &ep, BODY).await;
        damage(&fx.mp3_dir("s1"), &id);
        let wanted = fx.wanted(0, "s1", &ep);
        let dest = wanted.mp3_dest.clone();
        assert!(
            access.lookup(vec![wanted], 200).await.is_empty(),
            "{what} : servi"
        );
        assert!(!dest.exists(), "{what} : rien placé");
        // Éviction : les restes d'une entrée sans métadonnées valides sont supprimés.
        access.evict(HashSet::new()).await;
        assert!(fx.files("s1").is_empty(), "{what} : {:?}", fx.files("s1"));
    }
}

/// Placeur qui écrit un début de fichier puis échoue comme un disque plein.
fn disk_full(src: &Path, dest: &Path, _: &Cancel) -> io::Result<()> {
    let bytes = std::fs::read(src)?;
    std::fs::write(dest, &bytes[..bytes.len() / 2])?;
    Err(io::Error::new(io::ErrorKind::StorageFull, "disque plein"))
}

#[tokio::test]
async fn disque_plein_a_l_ecriture_ne_laisse_rien() {
    let fx = Fixture::new();
    let ep = episode("g1");
    // Une ancienne entrée existe : elle est retirée avant l'écriture (jamais servie à moitié).
    fx.store(&fx.access(), "s1", &ep, b"ID3 ancien").await;
    let failing = fx.access().with_placer(Arc::new(disk_full));
    let (mp3, img) = fx.converted("nouveau", BODY);
    let n = failing
        .store(
            vec![Store {
                key: EntryKey::of("s1", &ep),
                mp3: mp3.clone(),
                image: Some(img),
            }],
            200,
        )
        .await;
    assert_eq!(n, 0);
    assert!(fx.files("s1").is_empty(), "{:?}", fx.files("s1"));
    assert_eq!(std::fs::read(&mp3).unwrap(), BODY, "source intacte");
    assert!(fx
        .access()
        .lookup(vec![fx.wanted(0, "s1", &ep)], 300)
        .await
        .is_empty());
}

#[tokio::test]
async fn disque_plein_a_la_lecture_donne_un_episode_absent() {
    let fx = Fixture::new();
    let ep = episode("g1");
    fx.store(&fx.access(), "s1", &ep, BODY).await;
    let failing = fx.access().with_placer(Arc::new(disk_full));
    let wanted = fx.wanted(0, "s1", &ep);
    let dest = wanted.mp3_dest.clone();
    assert!(failing.lookup(vec![wanted], 200).await.is_empty());
    assert!(!dest.exists(), "copie partielle supprimée");
    // L'entrée, intacte, reste servie ensuite.
    assert_eq!(
        fx.access()
            .lookup(vec![fx.wanted(0, "s1", &ep)], 300)
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn copie_sans_lien_physique_fonctionne() {
    let fx = Fixture::new();
    let copy_only: Arc<Placer> = Arc::new(copy_file);
    let access = fx.access().with_placer(copy_only);
    let ep = episode("g1");
    fx.store(&access, "s1", &ep, BODY).await;
    let hits = access.lookup(vec![fx.wanted(0, "s1", &ep)], 200).await;
    assert_eq!(std::fs::read(&hits[0].mp3).unwrap(), BODY);
}

#[tokio::test]
async fn source_purgee_pendant_la_creation_jamais_recreee() {
    let fx = Fixture::new();
    let ep = episode("g1");
    fx.store(&fx.access(), "s1", &ep, BODY).await;
    // Générations lues au début de la préparation, puis la source est supprimée.
    let access = fx.access();
    fx.locks.purge(&fx.root(), "s1");
    assert!(access
        .lookup(vec![fx.wanted(0, "s1", &ep)], 200)
        .await
        .is_empty());
    let (mp3, img) = fx.converted("g1", BODY);
    let n = access
        .store(
            vec![Store {
                key: EntryKey::of("s1", &ep),
                mp3,
                image: Some(img),
            }],
            200,
        )
        .await;
    assert_eq!(n, 0);
    access.evict(HashSet::new()).await;
    assert!(
        !source_dir(&fx.root(), "s1").unwrap().exists(),
        "dossier de la source recréé"
    );
    // L'autre source n'est pas concernée.
    fx.store(&access, "s2", &ep, BODY).await;
}

#[tokio::test]
async fn eviction_lru_respecte_le_plafond_et_les_protections() {
    let fx = Fixture::new();
    let body = [b'I'; 100];
    let image_bytes = b"\x89PNG image".len() as u64;
    let entry_bytes = 100 + image_bytes;
    // Trois entrées, utilisées aux instants 10, 20, 30.
    for (n, guid) in ["a", "b", "c"].into_iter().enumerate() {
        let access = fx.access();
        let (mp3, img) = fx.converted(guid, &body);
        access
            .store(
                vec![Store {
                    key: EntryKey::of("s1", &episode(guid)),
                    mp3,
                    image: Some(img),
                }],
                10 * (n as u64 + 1),
            )
            .await;
    }
    // Restes : `.part`, temporaire, fichier sans métadonnées.
    let dir = fx.mp3_dir("s1");
    let orphan = episode_id("orphelin");
    std::fs::write(dir.join(format!("{orphan}.mp3")), b"x").unwrap();
    std::fs::write(dir.join(format!("{}.mp3.part", episode_id("p"))), b"x").unwrap();
    std::fs::write(dir.join(".x.json.1-2.tmp"), b"x").unwrap();

    // Plafond de deux entrées, « a » (la plus ancienne) protégée : « b » part.
    let access = fx.access().with_max_bytes(2 * entry_bytes);
    let protect = HashSet::from([("s1".to_string(), episode_id("a"))]);
    access.evict(protect).await;
    let mut expected: Vec<String> = ["a", "c"]
        .iter()
        .flat_map(|g| {
            let id = episode_id(g);
            [
                format!("{id}.img"),
                format!("{id}.json"),
                format!("{id}.mp3"),
            ]
        })
        .collect();
    expected.sort();
    assert_eq!(fx.files("s1"), expected);

    // Sans protection, plafond d'une entrée : la moins récemment utilisée (« a ») part.
    fx.access()
        .with_max_bytes(entry_bytes)
        .evict(HashSet::new())
        .await;
    let id = episode_id("c");
    assert_eq!(
        fx.files("s1"),
        [
            format!("{id}.img"),
            format!("{id}.json"),
            format!("{id}.mp3")
        ]
    );
}

#[test]
fn victimes_toutes_sources_confondues() {
    let e = |s: &str, id: &str, bytes, used_at| Scanned {
        source_id: s.into(),
        episode_id: id.into(),
        bytes,
        used_at,
    };
    let entries = [
        e("s1", "x", 10, 5),
        e("s2", "y", 10, 1),
        e("s1", "z", 10, 9),
    ];
    let v = victims(&entries, 15, &HashSet::new());
    assert_eq!(v, [e("s2", "y", 10, 1), e("s1", "x", 10, 5)]);
    assert!(victims(&entries, 30, &HashSet::new()).is_empty());
}

#[tokio::test]
async fn verrou_d_une_source_purgee_refuse() {
    let dir = tempfile::tempdir().unwrap();
    let service = super::super::source::PodcastService::new(
        super::super::net::NetPolicy::strict(),
        Arc::default(),
    )
    .unwrap();
    let generation = service.generation("s1");
    assert!(service.lock_source("s1", generation).await.is_some());
    service.purge_source(dir.path(), "s1").await.unwrap();
    assert!(service.lock_source("s1", generation).await.is_none());
    assert!(service
        .lock_source("s1", service.generation("s1"))
        .await
        .is_some());
}

// --- Suivis de la revue (#466) --------------------------------------------------------

/// Lien de dossier sans privilège : jonction (`mklink /J`) sous Windows, lien symbolique
/// ailleurs.
fn link_dir(link: &Path, target: &Path) {
    #[cfg(windows)]
    {
        let out = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(link.to_string_lossy().replace('/', "\\"))
            .arg(target.to_string_lossy().replace('/', "\\"))
            .output()
            .unwrap();
        assert!(out.status.success(), "mklink /J : {out:?}");
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
}

/// Noms des fichiers d'un dossier, triés.
fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Panne : `podcasts/<id>/` ou `podcasts/` remplacé par une jonction vers un dossier hors du
/// cache. Rien n'y est lu, écrit ni supprimé, même au-delà du plafond.
#[tokio::test]
async fn jonction_sur_le_chemin_du_cache_jamais_suivie() {
    for linked in ["source", "podcasts"] {
        let fx = Fixture::new();
        let ep = episode("g1");
        fx.store(&fx.access(), "s1", &ep, BODY).await;
        // Reste qui serait supprimé dans un vrai dossier.
        std::fs::write(
            fx.mp3_dir("s1")
                .join(format!("{}.mp3.part", episode_id("p"))),
            b"x",
        )
        .unwrap();
        // Le dossier part hors du cache ; une jonction prend sa place.
        let outside = fx.dir.path().join("dehors");
        std::fs::create_dir_all(&outside).unwrap();
        let moved = match linked {
            "source" => source_dir(&fx.root(), "s1").unwrap(),
            _ => fx.root().join(CACHE_SUBDIR),
        };
        let target = outside.join("cible");
        std::fs::rename(&moved, &target).unwrap();
        link_dir(&moved, &target);
        let real_mp3 = match linked {
            "source" => target.join(CACHE_DIR),
            _ => target.join("s1").join(CACHE_DIR),
        };
        let before = names(&real_mp3);
        assert_eq!(before.len(), 4, "{linked} : {before:?}");

        let access = fx.access().with_max_bytes(0);
        assert!(
            access
                .lookup(vec![fx.wanted(0, "s1", &ep)], 200)
                .await
                .is_empty(),
            "{linked} : servi à travers une jonction"
        );
        let (mp3_src, img) = fx.converted("g2", BODY);
        let n = access
            .store(
                vec![Store {
                    key: EntryKey::of("s1", &episode("g2")),
                    mp3: mp3_src,
                    image: Some(img),
                }],
                200,
            )
            .await;
        assert_eq!(n, 0, "{linked} : écrit à travers une jonction");
        access.evict(HashSet::new()).await;
        assert_eq!(
            names(&real_mp3),
            before,
            "{linked} : dossier extérieur modifié"
        );
        // Date d'usage non renouvelée hors du cache.
        let meta: EntryMeta = serde_json::from_slice(
            &std::fs::read(real_mp3.join(format!("{}.json", episode_id("g1")))).unwrap(),
        )
        .unwrap();
        assert_eq!(meta.used_at, 100, "{linked}");
        // L'autre source (vrai dossier) reste utilisable quand seule `s1` est liée.
        if linked == "source" {
            fx.store(&fx.access(), "s2", &ep, BODY).await;
        }
    }
}

/// Le contrôle refuse aussi un fichier à la place d'un dossier, et ne crée les dossiers
/// manquants qu'en écriture.
#[test]
fn chemin_du_cache_controle_dossier_par_dossier() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cache");
    assert!(real_mp3_dir(&root, "s1", false).unwrap().is_none());
    assert!(!root.exists(), "rien n'est créé en lecture");
    let mp3 = real_mp3_dir(&root, "s1", true).unwrap().unwrap();
    assert!(is_real_dir(&mp3));
    assert_eq!(real_mp3_dir(&root, "s1", false).unwrap(), Some(mp3));
    // Un fichier à la place de `podcasts/s2/`.
    std::fs::write(root.join(CACHE_SUBDIR).join("s2"), b"x").unwrap();
    assert!(real_mp3_dir(&root, "s2", true).is_err());
    assert!(real_mp3_dir(&root, "s2", false).is_err());
    // Identifiant invalide.
    assert!(real_mp3_dir(&root, "../s1", true).is_err());
}

/// Placeur qui place le fichier puis lève l'annulation (annulation pendant la copie).
fn cancel_after_place(tx: Arc<watch::Sender<bool>>) -> Arc<Placer> {
    Arc::new(move |src: &Path, dest: &Path, cancel: &Cancel| {
        let placed = link_or_copy(src, dest, cancel);
        tx.send_replace(true);
        placed
    })
}

/// Panne : annulation pendant la lecture. L'épisode en cours n'est pas servi, les suivants ne
/// sont pas lus, et aucune entrée n'est supprimée : toutes restent servies ensuite.
#[tokio::test]
async fn lecture_annulee_ne_supprime_aucune_entree() {
    let fx = Fixture::new();
    let (g1, g2) = (episode("g1"), episode("g2"));
    fx.store(&fx.access(), "s1", &g1, BODY).await;
    fx.store(&fx.access(), "s1", &g2, BODY).await;
    let before = fx.files("s1");

    // Annulation déjà demandée : rien n'est lu.
    let (tx, rx) = watch::channel(true);
    let access = fx.access().with_cancel(rx);
    let wanted = || vec![fx.wanted(0, "s1", &g1), fx.wanted(1, "s1", &g2)];
    assert!(access.lookup(wanted(), 200).await.is_empty());
    assert_eq!(fx.files("s1"), before);

    // Annulation levée pendant la copie du premier MP3 : sa pochette n'est pas vérifiée
    // (hachage interrompu), il n'est pas servi, le second n'est pas lu.
    tx.send_replace(false);
    let tx = Arc::new(tx);
    let access = fx
        .access()
        .with_cancel(tx.subscribe())
        .with_placer(cancel_after_place(tx.clone()));
    assert!(access.lookup(wanted(), 200).await.is_empty());
    assert_eq!(fx.files("s1"), before, "entrée supprimée sur annulation");
    assert!(!fx.work().join("cache-0000.mp3").exists(), "copie laissée");

    // Hachage interrompu dès le premier bloc : entrée non jugée, gardée.
    tx.send_replace(true);
    let cancel = Cancel::new(tx.subscribe());
    let op = Op {
        now: 300,
        place: &link_or_copy,
        cancel: &cancel,
        max_entry_bytes: u64::MAX,
    };
    assert!(lookup_one(&fx.mp3_dir("s1"), &fx.wanted(0, "s1", &g1), &op).is_none());
    assert_eq!(fx.files("s1"), before);

    // Sans annulation, tout est servi.
    assert_eq!(fx.access().lookup(wanted(), 400).await.len(), 2);
}

/// Panne : annulation pendant l'écriture. Rien de l'entrée ne reste (ni `.part`, ni
/// métadonnées) ; une annulation demandée avant le début garde l'ancienne entrée.
#[tokio::test]
async fn ecriture_annulee_ne_laisse_rien() {
    let fx = Fixture::new();
    let ep = episode("g1");
    let tx = Arc::new(watch::channel(false).0);
    let access = fx
        .access()
        .with_cancel(tx.subscribe())
        .with_placer(cancel_after_place(tx.clone()));
    let (mp3, img) = fx.converted("g1", BODY);
    let store = || {
        vec![Store {
            key: EntryKey::of("s1", &ep),
            mp3: mp3.clone(),
            image: Some(img.clone()),
        }]
    };
    assert_eq!(access.store(store(), 100).await, 0);
    assert!(fx.files("s1").is_empty(), "{:?}", fx.files("s1"));

    // Ancienne entrée, puis écriture d'une nouvelle version déjà annulée : l'ancienne reste.
    fx.store(&fx.access(), "s1", &ep, b"ID3 ancien").await;
    let before = fx.files("s1");
    let (_tx, rx) = watch::channel(true);
    assert_eq!(fx.access().with_cancel(rx).store(store(), 200).await, 0);
    assert_eq!(fx.files("s1"), before);
    let hits = fx.access().lookup(vec![fx.wanted(0, "s1", &ep)], 300).await;
    assert_eq!(std::fs::read(&hits[0].mp3).unwrap(), b"ID3 ancien");
}

/// Copie de repli (sans lien physique) et hachage : interrompus par l'annulation, la copie
/// partielle est supprimée.
#[test]
fn copie_et_hachage_annulables() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src.mp3");
    std::fs::write(&src, vec![7u8; 3 * CHUNK + 1]).unwrap();
    let dest = dir.path().join("dest.mp3");
    let (tx, rx) = watch::channel(true);
    let cancel = Cancel::new(rx);
    let err = copy_file(&src, &dest, &cancel).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    assert!(!dest.exists());
    let err = measure(&src, &cancel).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::Interrupted);
    tx.send_replace(false);
    copy_file(&src, &dest, &cancel).unwrap();
    assert_eq!(
        measure(&dest, &cancel).unwrap(),
        measure(&src, &Cancel::default()).unwrap()
    );
}

/// Panne : entrée plus grande que la borne d'un épisode (borne abaissée depuis l'écriture) :
/// jamais servie, supprimée.
#[tokio::test]
async fn entree_trop_volumineuse_invalidee() {
    let fx = Fixture::new();
    let ep = episode("g1");
    fx.store(&fx.access(), "s1", &ep, BODY).await;
    let access = fx.access().with_max_entry_bytes(BODY.len() as u64 - 1);
    assert!(access
        .lookup(vec![fx.wanted(0, "s1", &ep)], 200)
        .await
        .is_empty());
    assert!(fx.files("s1").is_empty(), "{:?}", fx.files("s1"));
}

/// Seuls `<id>.mp3`, `<id>.img` et `<id>.json` comptent comme fichiers d'entrée : une autre
/// extension est un reste, supprimé à l'éviction sans toucher à l'entrée.
#[tokio::test]
async fn extension_inconnue_est_un_reste() {
    let fx = Fixture::new();
    let ep = episode("g1");
    fx.store(&fx.access(), "s1", &ep, BODY).await;
    let before = fx.files("s1");
    let dir = fx.mp3_dir("s1");
    let id = episode_id("g1");
    std::fs::write(dir.join(format!("{id}.exe")), b"x").unwrap();
    std::fs::write(dir.join(format!("{}.txt", episode_id("seul"))), b"x").unwrap();
    std::fs::write(dir.join(format!("{id}.mp3.bak")), b"x").unwrap();
    let survey = scan(&fx.root());
    assert_eq!(survey.entries.len(), 1);
    assert_eq!(
        survey.entries[0].bytes,
        (BODY.len() + b"\x89PNG image".len()) as u64
    );
    fx.access().evict(HashSet::new()).await;
    assert_eq!(fx.files("s1"), before);
    assert_eq!(
        fx.access()
            .lookup(vec![fx.wanted(0, "s1", &ep)], 200)
            .await
            .len(),
        1
    );
}

/// Les constantes de la règle D6 sont figées avec `AUDIO_PROFILE` : en changer une impose
/// d'incrémenter le profil (toutes les entrées deviennent invalides) et de mettre ce test à
/// jour.
#[test]
fn regle_d6_figee_avec_le_profil() {
    use crate::podcast::audio::{MAX_KBPS, MIN_KBPS, TARGET_SAMPLE_RATE};
    assert_eq!(
        (AUDIO_PROFILE, TARGET_SAMPLE_RATE, MIN_KBPS, MAX_KBPS),
        ("d6-1", 44_100, 64, 192),
        "règle D6 changée : incrémenter AUDIO_PROFILE"
    );
}

/// Verrous réels : le cache prend le verrou de la source du `PodcastService` (celui des
/// rafraîchissements) et respecte sa génération (purge).
#[tokio::test]
async fn verrous_du_service_partages_avec_le_cache() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("cache");
    let service = Arc::new(
        super::super::source::PodcastService::new(
            super::super::net::NetPolicy::strict(),
            Arc::default(),
        )
        .unwrap(),
    );
    let access = CacheAccess::new(service.clone(), root.clone(), ["s1".to_string()]);
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mp3 = work.join("a.mp3");
    std::fs::write(&mp3, BODY).unwrap();
    let ep = episode("g1");
    let store = || {
        vec![Store {
            key: EntryKey::of("s1", &ep),
            mp3: mp3.clone(),
            image: None,
        }]
    };
    // Rafraîchissement en cours (verrou tenu) : l'écriture attend.
    let held = service
        .lock_source("s1", service.generation("s1"))
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), access.store(store(), 100))
            .await
            .is_err(),
        "écriture faite pendant un rafraîchissement"
    );
    drop(held);
    assert_eq!(access.store(store(), 100).await, 1);
    // Source purgée : plus rien n'est écrit ni recréé.
    service.purge_source(&root, "s1").await.unwrap();
    assert_eq!(access.store(store(), 200).await, 0);
    assert!(!source_dir(&root, "s1").unwrap().exists());
}

/// Absence d'interblocage : la tâche d'installation tient le portillon du cache en lecture et
/// un « Vider le cache » attend l'écriture ; le verrou de la source reste obtenu.
#[tokio::test]
async fn verrou_de_source_obtenu_sous_portillon_avec_vidage_en_attente() {
    let gate: Arc<tokio::sync::RwLock<()>> = Arc::default();
    let service = super::super::source::PodcastService::new(
        super::super::net::NetPolicy::strict(),
        gate.clone(),
    )
    .unwrap();
    let installing = gate.clone().read_owned().await;
    let clearing = tokio::spawn({
        let gate = gate.clone();
        async move {
            let _write = gate.write().await;
        }
    });
    // Le vidage est en attente : une nouvelle prise en lecture serait bloquée.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(50), gate.read())
            .await
            .is_err(),
        "vidage pas encore en attente"
    );
    let guard = tokio::time::timeout(
        Duration::from_secs(5),
        service.lock_source("s1", service.generation("s1")),
    )
    .await
    .expect("interblocage : verrou de source jamais obtenu");
    assert!(guard.is_some());
    drop(guard);
    drop(installing);
    tokio::time::timeout(Duration::from_secs(5), clearing)
        .await
        .unwrap()
        .unwrap();
}
