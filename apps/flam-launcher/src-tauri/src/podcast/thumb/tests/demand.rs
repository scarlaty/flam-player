//! Téléchargement à la demande et cache des vignettes (#508), puis suivis de la revue de #506
//! (lecture seule de `cached_image`, source purgée, `find_episode`).

use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;

/// Téléchargement simulé : écrit `body` (ou échoue si `None`) et compte les appels.
async fn fake_download(calls: &AtomicUsize, body: Option<Vec<u8>>, dest: PathBuf) -> bool {
    calls.fetch_add(1, Ordering::SeqCst);
    match body {
        Some(b) => std::fs::write(dest, b).is_ok(),
        None => false,
    }
}

fn thumbs_dir(f: &Fixture) -> PathBuf {
    crate::podcast::source::source_dir(&f.root(), "s1")
        .unwrap()
        .join(THUMBS_DIR)
}

fn names(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|d| {
            d.flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn pochette_telechargee_mise_en_cache_puis_relue_sans_reseau() {
    let f = Fixture::new();
    let ep = episode("d1");
    let key = EntryKey::of("s1", &ep);
    let calls = AtomicUsize::new(0);
    let first = load(
        &f.access(),
        &key,
        |dest| fake_download(&calls, Some(png(500, 500)), dest),
        in_process,
    )
    .await
    .expect("vignette");
    assert!(first.starts_with("data:image/png;base64,"));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // Seule la vignette reste : ni téléchargement ni fichier temporaire.
    let kept = names(&thumbs_dir(&f));
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert!(kept[0].ends_with(".png"));
    // Deuxième demande : servie par le cache, sans téléchargement.
    let second = load(
        &f.access(),
        &key,
        |dest| fake_download(&calls, None, dest),
        in_process,
    )
    .await;
    assert_eq!(second.as_deref(), Some(first.as_str()));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cache_des_mp3_prioritaire_sur_le_telechargement() {
    let f = Fixture::new();
    let ep = episode("d2");
    f.store(&ep, Some(&png(300, 200))).await;
    let calls = AtomicUsize::new(0);
    let url = load(
        &f.access(),
        &EntryKey::of("s1", &ep),
        |dest| fake_download(&calls, None, dest),
        in_process,
    )
    .await;
    assert!(url.is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn echec_reseau_ou_image_refusee_pas_d_apercu_ni_reste() {
    let f = Fixture::new();
    let calls = AtomicUsize::new(0);
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec();
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00;".to_vec();
    let cas: [(&str, Option<Vec<u8>>); 4] = [
        ("e1", None),
        ("e2", Some(svg)),
        ("e3", Some(gif)),
        ("e4", Some(Vec::new())),
    ];
    for (guid, body) in cas {
        let key = EntryKey::of("s1", &episode(guid));
        let out = load(
            &f.access(),
            &key,
            |dest| fake_download(&calls, body, dest),
            in_process,
        )
        .await;
        assert_eq!(out, None, "{guid}");
    }
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    // Aucun échec n'est mis en cache et rien ne traîne dans le dossier.
    assert!(names(&thumbs_dir(&f)).is_empty());
}

#[tokio::test]
async fn image_trop_grosse_refusee_et_supprimee() {
    let f = Fixture::new();
    let key = EntryKey::of("s1", &episode("big"));
    let big = vec![0u8; MAX_INPUT_BYTES + 1];
    let calls = AtomicUsize::new(0);
    let out = load(
        &f.access(),
        &key,
        |dest| fake_download(&calls, Some(big), dest),
        in_process,
    )
    .await;
    assert_eq!(out, None);
    assert!(names(&thumbs_dir(&f)).is_empty());
}

#[tokio::test]
async fn source_purgee_ni_cache_ni_telechargement() {
    let f = Fixture::new();
    let key = EntryKey::of("s1", &episode("p1"));
    let access = f.access();
    // Purge après la lecture de la génération (suppression de la source pendant la résolution
    // de son adresse) : rien n'est téléchargé, rien n'est recréé.
    f.locks.purge(&f.root(), "s1");
    let calls = AtomicUsize::new(0);
    let out = load(
        &access,
        &key,
        |dest| fake_download(&calls, Some(png(64, 64)), dest),
        in_process,
    )
    .await;
    assert_eq!(out, None);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!thumbs_dir(&f).exists());
}

#[tokio::test]
async fn les_vignettes_partent_avec_la_source() {
    let f = Fixture::new();
    let key = EntryKey::of("s1", &episode("p2"));
    let calls = AtomicUsize::new(0);
    assert!(load(
        &f.access(),
        &key,
        |dest| fake_download(&calls, Some(png(64, 64)), dest),
        in_process,
    )
    .await
    .is_some());
    assert!(thumbs_dir(&f).is_dir());
    f.locks.purge(&f.root(), "s1");
    assert!(!thumbs_dir(&f).exists());
}

#[tokio::test]
async fn image_changee_dans_le_flux_nouvelle_vignette() {
    let f = Fixture::new();
    let ep = episode("c1");
    let calls = AtomicUsize::new(0);
    for url in [
        "https://img.example.test/a.png",
        "https://img.example.test/b.png",
    ] {
        let mut changed = ep.clone();
        changed.image = Some(url.into());
        let out = load(
            &f.access(),
            &EntryKey::of("s1", &changed),
            |dest| fake_download(&calls, Some(png(40, 40)), dest),
            in_process,
        )
        .await;
        assert!(out.is_some());
    }
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(names(&thumbs_dir(&f)).len(), 2);
}

#[tokio::test]
async fn vignette_en_cache_alteree_ou_trop_grosse_est_ignoree() {
    let f = Fixture::new();
    let key = EntryKey::of("s1", &episode("a1"));
    let name = thumb_name(&key).unwrap();
    let dir = thumbs_dir(&f);
    std::fs::create_dir_all(&dir).unwrap();
    let calls = AtomicUsize::new(0);
    for bad in [b"pas un png".to_vec(), vec![0x89; MAX_THUMB_BYTES + 1]] {
        std::fs::write(dir.join(&name), bad).unwrap();
        let out = load(
            &f.access(),
            &key,
            |dest| fake_download(&calls, Some(png(32, 32)), dest),
            in_process,
        )
        .await;
        assert!(out.is_some());
    }
    // Chaque fichier invalide a été retéléchargé puis remplacé.
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(read_thumb(&dir, &name).is_some());
}

#[test]
fn restes_anciens_supprimes_restes_recents_gardes() {
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("x.dl");
    let recent = dir.path().join("y.dl");
    let kept = dir.path().join("z.png");
    for p in [&old, &recent, &kept] {
        std::fs::write(p, b"x").unwrap();
    }
    let long_ago = std::time::SystemTime::now() - STALE_AFTER - Duration::from_secs(60);
    for p in [&old, &kept] {
        std::fs::File::options()
            .write(true)
            .open(p)
            .unwrap()
            .set_modified(long_ago)
            .unwrap();
    }
    sweep_stale(dir.path());
    assert!(!old.exists());
    assert!(recent.exists());
    assert!(kept.exists(), "une vignette n'est jamais balayée");
}

#[test]
fn nom_de_vignette_refuse_les_identifiants_inattendus() {
    let mut key = EntryKey::of("s1", &episode("n1"));
    assert!(thumb_name(&key).is_some());
    key.episode_id = "../evil".into();
    assert_eq!(thumb_name(&key), None);
    let mut key = EntryKey::of("s1", &episode("n2"));
    key.image_fingerprint = None;
    assert_eq!(thumb_name(&key), None);
}

#[tokio::test]
async fn telechargements_simultanes_bornes() {
    let f = Fixture::new();
    let running = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let access = Arc::new(f.access());
    let mut tasks = Vec::new();
    for i in 0..(DOWNLOADS * 3) {
        let (running, peak, access) = (running.clone(), peak.clone(), access.clone());
        tasks.push(tokio::spawn(async move {
            let key = EntryKey::of("s1", &episode(&format!("w{i}")));
            load(
                &access,
                &key,
                |dest| async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(30)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    std::fs::write(dest, png(30, 30)).is_ok()
                },
                in_process,
            )
            .await
        }));
    }
    for t in tasks {
        assert!(t.await.unwrap().is_some());
    }
    let peak = peak.load(Ordering::SeqCst);
    assert!((2..=DOWNLOADS).contains(&peak), "{peak}");
}

#[tokio::test]
async fn le_telechargement_reel_refuse_les_adresses_locales() {
    let dir = tempfile::tempdir().unwrap();
    for url in [
        "http://127.0.0.1:9/a.png",
        "http://localhost/a.png",
        "http://192.168.1.1/a.png",
        "file:///etc/passwd",
        "pas une adresse",
    ] {
        let dest = dir.path().join("x.dl");
        assert!(!download_from_web(url.into(), dest.clone()).await, "{url}");
        assert!(!dest.exists());
    }
}

// --- Suivis de la revue de #506 ---------------------------------------------------------

#[tokio::test]
async fn lecture_du_cache_sans_effet_de_bord() {
    let f = Fixture::new();
    let ep = episode("r1");
    f.store(&ep, Some(&png(64, 64))).await;
    let key = EntryKey::of("s1", &ep);
    let mp3 = crate::podcast::source::source_dir(&f.root(), "s1")
        .unwrap()
        .join("mp3");
    let snapshot = |dir: &Path| {
        let mut all: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| (e.file_name(), std::fs::read(e.path()).unwrap()))
            .collect();
        all.sort();
        all
    };
    let before = snapshot(&mp3);
    assert!(f.access().cached_image(&key, 1 << 20).await.is_some());
    // Borne trop basse, puis adresse changée : ni suppression ni réécriture.
    assert!(f.access().cached_image(&key, 1).await.is_none());
    let mut changed = ep.clone();
    changed.image = Some("https://img.example.test/autre.png".into());
    assert!(f
        .access()
        .cached_image(&EntryKey::of("s1", &changed), 1 << 20)
        .await
        .is_none());
    assert_eq!(snapshot(&mp3), before);
}

#[tokio::test]
async fn source_purgee_apres_la_creation_de_l_acces_pas_de_pochette() {
    let f = Fixture::new();
    let ep = episode("r2");
    f.store(&ep, Some(&png(64, 64))).await;
    let access = f.access();
    f.locks.purge(&f.root(), "s1");
    assert!(access
        .cached_image(&EntryKey::of("s1", &ep), 1 << 20)
        .await
        .is_none());
}

const FEED_XML: &str = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel>
  <title>Contes</title>
  <item><title>Un</title><guid>g1</guid>
    <enclosure url="https://cdn.example.test/a.mp3" type="audio/mpeg" length="1000"/>
    <itunes:image href="https://img.example.test/a.png"/></item>
</channel></rss>"#;

#[tokio::test]
async fn find_episode_retrouve_l_episode_du_flux_en_cache() {
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FEED_XML))
        .mount(&server)
        .await;
    let url = format!("{}/flux.xml", server.uri());
    let config_dir = tempfile::tempdir().unwrap();
    let cache = config_dir.path().join("cache");
    let store = SettingsStore::new(config_dir.path());
    let mut settings = store.load().unwrap();
    settings.cache_dir = Some(cache.to_string_lossy().into_owned());
    store.update(settings).unwrap();
    let config = ConfigState::new(config_dir.path().to_path_buf());
    SourcesStore::new(config_dir.path())
        .add_podcast("p1".into(), "Contes".into(), url.clone())
        .unwrap();
    let service = PodcastService::new(
        crate::podcast::net::NetPolicy::loopback_for_tests(),
        Arc::default(),
    )
    .unwrap();
    let wanted = EpisodeRef {
        source_id: "p1".into(),
        episode_id: episode_id("g1"),
    };

    // Flux jamais obtenu : pas d'épisode.
    assert!(find_episode(&config, &service, &wanted)
        .await
        .unwrap()
        .is_none());

    let fetched = service.fetch(&url).await.unwrap();
    let root = SettingsStore::new(config_dir.path())
        .resolved_cache_dir(&Environment::current())
        .unwrap();
    service
        .store_fetched(0, &root, "p1", &url, fetched, 100)
        .await
        .unwrap();

    let (found_root, found) = find_episode(&config, &service, &wanted)
        .await
        .unwrap()
        .expect("épisode");
    assert_eq!(found_root, root);
    assert_eq!(found.guid, "g1");
    assert_eq!(
        found.image.as_deref(),
        Some("https://img.example.test/a.png")
    );

    // Épisode inconnu, source inconnue : rien.
    for (s, e) in [("p1", episode_id("absent")), ("inconnue", episode_id("g1"))] {
        let r = EpisodeRef {
            source_id: s.into(),
            episode_id: e,
        };
        assert!(find_episode(&config, &service, &r).await.unwrap().is_none());
    }
}
