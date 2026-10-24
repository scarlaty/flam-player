//! Cache des MP3 convertis (#460) : clé, lecture, écriture atomique, invalidation, éviction,
//! injection de pannes (fichier tronqué ou altéré, métadonnées illisibles, disque plein, purge
//! de la source pendant une création).

use std::sync::Mutex;

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
fn disk_full(src: &Path, dest: &Path) -> io::Result<()> {
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
    let copy_only: Arc<Placer> =
        Arc::new(|src: &Path, dest: &Path| std::fs::copy(src, dest).map(|_| ()));
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
