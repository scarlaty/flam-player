use std::sync::Arc;

use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;

/// Fragment secret de l'URL du flux (et d'une enclosure) : ne doit apparaître nulle part.
const SECRET: &str = "SECRET42";
const FEED_PATH: &str = "/prive/flux.xml";

const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
<channel>
  <title>Contes inventés</title>
  <copyright>Autrice Fictive</copyright>
  <itunes:image href="https://img.example.test/c.png"/>
  <item><title>Ancien</title><guid>g1</guid><pubDate>Mon, 01 Jan 2024 10:00:00 GMT</pubDate>
    <enclosure url="https://cdn.example.test/a.mp3?jeton=SECRET42" type="audio/mpeg" length="1000"/></item>
  <item><title>Sans date</title>
    <enclosure url="https://cdn.example.test/c.mp3?jeton=SECRET42" type="audio/mpeg"/></item>
  <item><title>Récent</title><guid>g2</guid><pubDate>Tue, 02 Jan 2024 10:00:00 GMT</pubDate>
    <enclosure url="https://cdn.example.test/b.mp3" type="audio/mpeg"/></item>
</channel>
</rss>"#;

const FEED_V2: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Contes inventés</title>
  <item><title>Nouveau</title><guid>g3</guid>
    <enclosure url="https://cdn.example.test/n.mp3" type="audio/mpeg"/></item>
</channel></rss>"#;

const NO_AUDIO: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>Vide</title>
  <item><title>Vidéo</title><enclosure url="https://cdn.example.test/v.mp4" type="video/mp4"/></item>
</channel></rss>"#;

fn service() -> PodcastService {
    PodcastService::new(NetPolicy::loopback_for_tests(), Arc::default()).unwrap()
}

fn feed_url(server: &MockServer) -> String {
    format!("{}{FEED_PATH}?jeton={SECRET}", server.uri())
}

async fn serve(server: &MockServer, body: &str) {
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(FEED_PATH))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("ETag", "\"v1\"")
                .set_body_string(body),
        )
        .mount(server)
        .await;
}

async fn serve_status(server: &MockServer, status: u16) {
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(status))
        .mount(server)
        .await;
}

/// Ajout d'une source : téléchargement puis écriture du cache (génération 0).
async fn add(svc: &PodcastService, root: &Path, id: &str, url: &str) -> PodcastView {
    let fetched = svc.fetch(url).await.unwrap();
    svc.store_fetched(0, root, id, url, fetched, 100)
        .await
        .unwrap()
}

fn titles(view: &PodcastView) -> Vec<String> {
    view.feed
        .as_ref()
        .map(|f| f.episodes.iter().map(|e| e.title.clone()).collect())
        .unwrap_or_default()
}

fn assert_no_secret(text: &str, url: &str) {
    assert!(!text.contains(SECRET), "secret présent : {text}");
    assert!(!text.contains(FEED_PATH), "chemin du flux présent : {text}");
    assert!(!text.contains(url), "URL présente : {text}");
}

use crate::testlog::Logs;

#[test]
fn le_filtre_de_traces_desactive_reqwest_en_debug() {
    let logs = Logs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_env_filter(crate::log_filter(Some("debug")))
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!(target: "reqwest::connect", "ROUGE reqwest");
        tracing::debug!(target: "flam_launcher::x", "VERT flam");
    });
    let text = logs.text();
    assert!(!text.contains("ROUGE"), "reqwest en debug : {text}");
    assert!(text.contains("VERT"), "{text}");
    // Sans RUST_LOG : `info` (le debug de l'application est coupé, `reqwest` reste à `info`
    // même si l'on demande `trace`).
    let logs = Logs::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(logs.clone())
        .with_ansi(false)
        .with_env_filter(crate::log_filter(None))
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::debug!(target: "flam_launcher::x", "ROUGE debug");
        tracing::trace!(target: "reqwest::connect", "ROUGE reqwest trace");
        tracing::info!(target: "flam_launcher::x", "VERT info");
    });
    let text = logs.text();
    assert!(!text.contains("ROUGE"), "filtre par défaut : {text}");
    assert!(text.contains("VERT"), "{text}");
}

// ---------------------------------------------------------------------------------------
// Validation et indice
// ---------------------------------------------------------------------------------------

#[test]
fn une_url_valide_est_normalisee_sans_fragment() {
    assert_eq!(
        validate_feed_url("  https://Exemple.org/flux.xml?a=1#ancre ").unwrap(),
        "https://exemple.org/flux.xml?a=1"
    );
    assert_eq!(
        validate_feed_url("http://exemple.org").unwrap(),
        "http://exemple.org/"
    );
}

#[test]
fn une_url_invalide_ou_avec_identifiants_est_refusee_sans_etre_recopiee() {
    let long = format!("https://exemple.org/{SECRET}/{}", "a".repeat(2048));
    let cases = [
        String::new(),
        "   ".into(),
        format!("pas une url {SECRET}"),
        format!("ftp://exemple.org/{SECRET}.xml"),
        format!("file:///C:/{SECRET}.xml"),
        "https://".into(),
        format!("https://moi:{SECRET}@exemple.org/f.xml"),
        format!("https://{SECRET}@exemple.org/f.xml"),
        long,
    ];
    for bad in cases {
        let err = validate_feed_url(&bad).unwrap_err();
        assert_eq!(err.kind(), "validation", "{bad}");
        assert!(!err.to_string().contains(SECRET), "{err}");
    }
    let err = validate_feed_url(&format!("https://moi:{SECRET}@exemple.org/f.xml")).unwrap_err();
    assert!(err.to_string().contains("identifiants"), "{err}");
}

#[test]
fn l_identifiant_d_episode_est_opaque_et_stable() {
    let id = episode_id("https://cdn.example.test/a.mp3?jeton=SECRET42");
    assert_eq!(id.len(), 32);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
    assert!(!id.contains(SECRET));
    assert_eq!(
        id,
        episode_id("https://cdn.example.test/a.mp3?jeton=SECRET42")
    );
    assert_ne!(id, episode_id("g1"));
}

#[test]
fn un_identifiant_de_source_n_accepte_que_des_minuscules() {
    let root = Path::new("cache");
    assert!(source_dir(root, "3f2a-b_9").is_ok());
    for bad in ["", "ABC", "aBc", "a/b", "..", "a.b", &"a".repeat(65)] {
        assert!(source_dir(root, bad).is_err(), "{bad}");
    }
}

#[test]
fn la_vue_d_episode_expose_explicit_avec_heritage_du_canal() {
    let xml = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd"><channel>
  <title>T</title><itunes:explicit>yes</itunes:explicit>
  <item><title>Hérite</title><guid>a</guid>
    <enclosure url="https://c.example.test/a.mp3" type="audio/mpeg"/></item>
  <item><title>Propre</title><guid>b</guid><itunes:explicit>false</itunes:explicit>
    <enclosure url="https://c.example.test/b.mp3" type="audio/mpeg"/></item>
</channel></rss>"#;
    let view = FeedView::from(&parse_feed(xml.as_bytes()).unwrap());
    let by_title = |t: &str| {
        view.episodes
            .iter()
            .find(|e| e.title == t)
            .unwrap()
            .explicit
    };
    assert_eq!(by_title("Hérite"), Some(true));
    assert_eq!(by_title("Propre"), Some(false));
    let json = serde_json::to_value(&view).unwrap();
    assert_eq!(json["episodes"][0]["explicit"], serde_json::json!(true));
    let none = FeedView::from(&parse_feed(FEED.as_bytes()).unwrap());
    assert!(none.episodes.iter().all(|e| e.explicit.is_none()));
    assert!(serde_json::to_value(&none).unwrap()["episodes"][0]["explicit"].is_null());
}

#[test]
fn le_nom_par_defaut_est_le_titre_sur_une_ligne() {
    let mut feed = parse_feed(FEED.as_bytes()).unwrap();
    assert_eq!(default_name(&feed), "Contes inventés");
    feed.title = "  Deux\tlignes\n ici ".into();
    assert_eq!(default_name(&feed), "Deux lignes ici");
    feed.title = "é".repeat(150);
    assert_eq!(default_name(&feed).chars().count(), 100);
    feed.title = " \u{7}\n ".into();
    assert_eq!(default_name(&feed), "Podcast");
}

// ---------------------------------------------------------------------------------------
// Ajout, cache, rafraîchissement
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn l_ajout_ecrit_un_cache_sans_url_et_une_vue_sans_url() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    let view = add(&svc, root.path(), "p1", &url).await;

    // Tri par date décroissante, épisode sans date à la fin.
    assert_eq!(titles(&view), ["Récent", "Ancien", "Sans date"]);
    let feed = view.feed.as_ref().unwrap();
    assert_eq!(feed.copyright.as_deref(), Some("Autrice Fictive"));
    assert!(feed.has_image && feed.episodes.iter().all(|e| e.has_image));
    assert_eq!(view.status.updated_at, Some(100));
    let json = serde_json::to_string(&view).unwrap();
    assert_no_secret(&json, &url);
    assert!(!json.contains("cdn.example.test"), "{json}");

    let dir = root.path().join("podcasts/p1");
    assert_eq!(std::fs::read_to_string(dir.join("feed.xml")).unwrap(), FEED);
    let meta = std::fs::read_to_string(dir.join("meta.json")).unwrap();
    assert_no_secret(&meta, &url);
    assert!(meta.contains(&digest(&url)));

    // Relecture sans réseau.
    server.reset().await;
    let cached = svc.cached(root.path(), "p1", &url).await.unwrap();
    assert_eq!(cached, view);
    assert!(svc
        .cached_feed(root.path(), "p1", &url)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn l_ajout_refuse_un_flux_invalide_vide_trop_grand_ou_injoignable() {
    let server = MockServer::start().await;
    let svc = service();
    let url = feed_url(&server);

    serve(&server, "<html><body>pas un flux</body></html>").await;
    let err = svc.fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "feed_invalid");

    serve(&server, NO_AUDIO).await;
    let err = svc.fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "feed_invalid");
    assert!(err.to_string().contains("aucun épisode audio"), "{err}");

    serve(&server, &"x".repeat(MAX_FEED_BYTES + 1)).await;
    let err = svc.fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "feed_invalid");
    assert!(err.to_string().contains("trop volumineux"), "{err}");

    serve_status(&server, 404).await;
    let err = svc.fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "network");
    assert!(err.to_string().contains("404"), "{err}");
    assert_no_secret(&err.to_string(), &url);

    let unreachable = format!("http://127.0.0.1:1{FEED_PATH}?jeton={SECRET}");
    let err = svc.fetch(&unreachable).await.err().unwrap();
    assert_eq!(err.kind(), "network");
    assert_no_secret(&err.to_string(), &unreachable);
}

#[tokio::test]
async fn le_rafraichissement_remplace_le_cache_puis_utilise_l_etag() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "p1", &url).await;

    serve(&server, FEED_V2).await;
    let view = svc.refresh(0, root.path(), "p1", &url, 200).await.unwrap();
    assert_eq!(titles(&view), ["Nouveau"]);
    assert_eq!(view.status.updated_at, Some(200));

    // 304 à la requête conditionnelle : cache gardé, date renouvelée.
    server.reset().await;
    Mock::given(method("GET"))
        .and(header("if-none-match", "\"v1\""))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;
    let view = svc.refresh(0, root.path(), "p1", &url, 300).await.unwrap();
    assert_eq!(titles(&view), ["Nouveau"]);
    assert_eq!(view.status.updated_at, Some(300));
    assert!(!view.status.offline && view.status.error.is_none());
    let cached = svc.cached(root.path(), "p1", &url).await.unwrap();
    assert_eq!(cached.status.updated_at, Some(300));
}

/// 304 alors qu'aucun en-tête conditionnel n'a été envoyé (pas de cache) : nouvelle requête.
#[tokio::test]
async fn un_304_sans_cache_relance_la_requete() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(304))
        .up_to_n_times(1)
        .with_priority(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string(FEED))
        .mount(&server)
        .await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    let view = svc.refresh(0, root.path(), "p1", &url, 5).await.unwrap();
    assert_eq!(titles(&view).len(), 3);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

/// Critère d'acceptation : un flux indisponible affiche le cache hors ligne ; ni l'erreur ni
/// le journal ne contiennent l'URL.
#[tokio::test]
async fn un_flux_indisponible_sert_le_cache_hors_ligne_sans_journaliser_l_url() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "journal-p1", &url).await;
    let logs = Logs::capture();

    serve_status(&server, 503).await;
    let view = svc
        .refresh(0, root.path(), "journal-p1", &url, 200)
        .await
        .unwrap();
    assert!(view.status.offline);
    assert_eq!(view.status.error.as_ref().unwrap().kind, "network");
    assert_eq!(titles(&view), ["Récent", "Ancien", "Sans date"]);
    assert_eq!(view.status.updated_at, Some(100));
    assert_no_secret(&serde_json::to_string(&view).unwrap(), &url);

    // Serveur arrêté : même repli.
    drop(server);
    let view = svc
        .refresh(0, root.path(), "journal-p1", &url, 300)
        .await
        .unwrap();
    assert!(view.status.offline);
    assert_eq!(titles(&view).len(), 3);

    let text = logs.text();
    assert!(
        text.contains("journal-p1"),
        "le journal cite la source : {text}"
    );
    assert_no_secret(&text, &url);
}

#[tokio::test]
async fn sans_cache_un_echec_est_une_erreur() {
    let server = MockServer::start().await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    serve_status(&server, 500).await;
    let err = svc
        .refresh(0, root.path(), "p1", &url, 1)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "network");
    serve(&server, "<rss").await;
    let err = svc
        .refresh(0, root.path(), "p1", &url, 1)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "feed_invalid");
    assert!(!root.path().join("podcasts/p1/meta.json").exists());
}

#[tokio::test]
async fn un_flux_devenu_invalide_ou_vide_garde_le_cache() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "p1", &url).await;

    for body in ["<!DOCTYPE rss><rss/>", NO_AUDIO, "pas du xml"] {
        serve(&server, body).await;
        let view = svc.refresh(0, root.path(), "p1", &url, 200).await.unwrap();
        assert!(!view.status.offline, "{body}");
        assert_eq!(view.status.error.as_ref().unwrap().kind, "feed_invalid");
        assert_eq!(titles(&view).len(), 3, "{body}");
    }
    let stored = std::fs::read_to_string(root.path().join("podcasts/p1/feed.xml")).unwrap();
    assert_eq!(stored, FEED);
}

// ---------------------------------------------------------------------------------------
// Injection de pannes sur le cache
// ---------------------------------------------------------------------------------------

/// `feed.xml` tronqué (écriture interrompue) : illisible, il n'est jamais servi.
#[tokio::test]
async fn un_flux_en_cache_tronque_n_est_pas_servi() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "p1", &url).await;
    let file = root.path().join("podcasts/p1/feed.xml");
    // Coupé avant le premier épisode : illisible ou sans épisode audio.
    std::fs::write(&file, &FEED.as_bytes()[..120]).unwrap();

    assert!(svc
        .cached(root.path(), "p1", &url)
        .await
        .unwrap()
        .feed
        .is_none());
    serve_status(&server, 500).await;
    let err = svc
        .refresh(0, root.path(), "p1", &url, 2)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "network");
}

/// `feed.xml` modifié mais lisible : servi, mais sans requête conditionnelle.
#[tokio::test]
async fn un_flux_en_cache_modifie_n_envoie_pas_d_etag() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "p1", &url).await;
    let file = root.path().join("podcasts/p1/feed.xml");
    std::fs::write(&file, format!("{FEED}\n\n")).unwrap();
    assert_eq!(
        titles(&svc.cached(root.path(), "p1", &url).await.unwrap()).len(),
        3
    );

    serve(&server, FEED_V2).await;
    let view = svc.refresh(0, root.path(), "p1", &url, 2).await.unwrap();
    assert_eq!(titles(&view), ["Nouveau"]);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("if-none-match"));
}

#[tokio::test]
async fn un_cache_d_une_autre_url_ou_sans_meta_est_ignore() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "p1", &url).await;
    let other = format!("{}/autre.xml", server.uri());
    assert!(svc
        .cached(root.path(), "p1", &other)
        .await
        .unwrap()
        .feed
        .is_none());

    // Purge interrompue après la suppression de `meta.json` : le reste n'est pas relu.
    std::fs::remove_file(root.path().join("podcasts/p1/meta.json")).unwrap();
    assert!(svc
        .cached(root.path(), "p1", &url)
        .await
        .unwrap()
        .feed
        .is_none());
    // `meta.json` illisible : idem.
    std::fs::write(root.path().join("podcasts/p1/meta.json"), "{tronqu").unwrap();
    assert!(svc
        .cached(root.path(), "p1", &url)
        .await
        .unwrap()
        .feed
        .is_none());
}

// ---------------------------------------------------------------------------------------
// Purge (suppression de la source)
// ---------------------------------------------------------------------------------------

/// Critère d'acceptation : la purge supprime le cache de la source et rien d'autre (histoires
/// installées, autres sources, autres sous-dossiers du cache).
#[tokio::test]
async fn la_purge_ne_touche_que_le_cache_de_la_source() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    add(&svc, root.path(), "p1", &url).await;
    add(&svc, root.path(), "p2", &url).await;
    let story = root.path().join("library/histoire/story.plain.pk");
    std::fs::create_dir_all(story.parent().unwrap()).unwrap();
    std::fs::write(&story, b"histoire").unwrap();
    let episode = root.path().join("podcasts/p1/episodes/e.mp3");
    std::fs::create_dir_all(episode.parent().unwrap()).unwrap();
    std::fs::write(&episode, b"audio").unwrap();

    svc.purge_source(root.path(), "p1").await.unwrap();

    assert!(!root.path().join("podcasts/p1").exists());
    assert!(root.path().join("podcasts/p2/feed.xml").is_file());
    assert_eq!(std::fs::read(&story).unwrap(), b"histoire");
    // Purger une source sans cache n'est pas une erreur.
    svc.purge_source(root.path(), "p1").await.unwrap();
    assert!(svc.purge_source(root.path(), "../p2").await.is_err());
    assert!(root.path().join("podcasts/p2/feed.xml").is_file());
}

/// Un rafraîchissement ou une écriture de cache qui a lu la génération avant la purge
/// abandonne : la purge ne laisse jamais un cache recréé derrière elle.
#[tokio::test]
async fn la_purge_retire_le_verrou_mais_garde_la_generation() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    add(&svc, root.path(), "p1", &feed_url(&server)).await;
    assert_eq!(lock(&svc.locks).len(), 1);
    svc.purge_source(root.path(), "p1").await.unwrap();
    assert!(lock(&svc.locks).is_empty());
    // Un rafraîchissement lancé avant la purge reste abandonné.
    assert_eq!(svc.generation("p1"), 1);
    let err = svc
        .refresh(0, root.path(), "p1", &feed_url(&server), 200)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "not_found");
}

#[tokio::test]
async fn apres_la_purge_aucun_cache_n_est_recree() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let root = tempfile::tempdir().unwrap();
    let svc = service();
    let url = feed_url(&server);
    let generation = svc.generation("p1");
    let fetched = svc.fetch(&url).await.unwrap();

    svc.purge_source(root.path(), "p1").await.unwrap();

    let err = svc
        .refresh(generation, root.path(), "p1", &url, 1)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "not_found");
    let err = svc
        .store_fetched(generation, root.path(), "p1", &url, fetched, 1)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "not_found");
    assert!(!root.path().join("podcasts/p1").exists());
}

#[tokio::test]
async fn la_purge_attend_le_portillon_du_vidage() {
    let gate: Arc<tokio::sync::RwLock<()>> = Arc::default();
    let svc = PodcastService::new(NetPolicy::loopback_for_tests(), gate.clone()).unwrap();
    let root = tempfile::tempdir().unwrap();
    let paused = gate.write().await;
    let blocked = tokio::time::timeout(
        Duration::from_millis(100),
        svc.purge_source(root.path(), "p1"),
    )
    .await;
    assert!(
        blocked.is_err(),
        "la purge doit attendre le vidage du cache"
    );
    drop(paused);
    svc.purge_source(root.path(), "p1").await.unwrap();
}

// ---------------------------------------------------------------------------------------
// Adresses privées (D10)
// ---------------------------------------------------------------------------------------

/// Politique stricte (production) : le serveur de test, sur 127.0.0.1, est refusé avant toute
/// connexion ; avec la politique de test, les autres plages privées restent refusées.
#[tokio::test]
async fn un_flux_sur_une_adresse_privee_litterale_est_refuse() {
    let server = MockServer::start().await;
    serve(&server, FEED).await;
    let strict = PodcastService::new(NetPolicy::strict(), Arc::default()).unwrap();
    let url = feed_url(&server);
    let err = strict.fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "network");
    assert_eq!(err.to_string(), BLOCKED_MESSAGE);
    assert_no_secret(&err.to_string(), &url);
    assert!(server.received_requests().await.unwrap().is_empty());

    let svc = service();
    for private in [
        format!("http://10.0.0.1{FEED_PATH}?jeton={SECRET}"),
        format!("http://169.254.169.254{FEED_PATH}"),
        format!("http://[::ffff:192.168.1.1]{FEED_PATH}"),
    ] {
        let err = svc.fetch(&private).await.err().unwrap();
        assert_eq!(err.to_string(), BLOCKED_MESSAGE, "{private}");
    }
    // Rafraîchissement avec un cache : adresse refusée, cache servi hors ligne.
    let root = tempfile::tempdir().unwrap();
    add(&svc, root.path(), "p1", &url).await;
    let err = strict.refresh(0, root.path(), "p1", &url, 2).await.unwrap();
    assert!(err.status.offline);
    assert_eq!(err.status.error.unwrap().message, BLOCKED_MESSAGE);
}

/// Une redirection vers une adresse privée est refusée par le client gardé, sans la suivre.
#[tokio::test]
async fn une_redirection_vers_une_adresse_privee_est_refusee() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(302).insert_header(
            "Location",
            format!("http://10.0.0.1/flux.xml?jeton={SECRET}"),
        ))
        .mount(&server)
        .await;
    let svc = service();
    let url = feed_url(&server);
    let logs = Logs::capture();
    let err = svc.fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "network");
    assert_eq!(err.to_string(), BLOCKED_MESSAGE);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_no_secret(&logs.text(), &url);
}

/// Serveur HTTP/1.1 minimal répondant en `Transfer-Encoding: chunked` (donc sans
/// `Content-Length`) avec `chunks` morceaux de 1 Mio d'espaces.
fn chunked_server(chunks: usize) -> (String, std::thread::JoinHandle<()>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/flux.xml", listener.local_addr().unwrap());
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = [0u8; 4096];
        let _ = stream.read(&mut request);
        let _ = stream.write_all(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
        );
        let block = vec![b' '; 1024 * 1024];
        for _ in 0..chunks {
            // Le client coupe dès le plafond atteint : les erreurs d'écriture sont normales.
            if stream.write_all(b"100000\r\n").is_err()
                || stream.write_all(&block).is_err()
                || stream.write_all(b"\r\n").is_err()
            {
                return;
            }
        }
        let _ = stream.write_all(b"0\r\n\r\n");
    });
    (url, handle)
}

#[tokio::test]
async fn un_corps_en_morceaux_sans_longueur_est_plafonne() {
    let max_mib = MAX_FEED_BYTES / (1024 * 1024);
    let (url, handle) = chunked_server(max_mib + 2);
    let err = service().fetch(&url).await.err().unwrap();
    assert_eq!(err.kind(), "feed_invalid", "{err}");
    assert!(err.to_string().contains("volumineux"), "{err}");
    // Joint hors du runtime : `fetch` a rendu la main, donc la connexion est fermée et le fil
    // serveur (erreur d'écriture) se termine.
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        tokio::task::spawn_blocking(move || handle.join()),
    )
    .await
    .expect("le fil du serveur chunked ne se termine pas")
    .unwrap()
    .expect("le fil du serveur chunked ne doit pas paniquer");
}

// ---------------------------------------------------------------------------------------
// Diagnostic copiable (D11)
// ---------------------------------------------------------------------------------------

/// Le diagnostic copiable (`device::diagnostic`) ne lit ni `sources.json` ni les podcasts, et
/// une erreur de flux qui y serait enregistrée ne contient pas l'URL.
#[tokio::test]
async fn le_diagnostic_ne_reprend_jamais_l_url_du_flux() {
    let diagnostic = include_str!("../../device/diagnostic.rs");
    for needle in ["SourcesStore", "sources.json", "podcast", "Source::"] {
        assert!(!diagnostic.contains(needle), "le diagnostic lit {needle}");
    }
    let server = MockServer::start().await;
    let svc = service();
    let url = feed_url(&server);
    let strict = PodcastService::new(NetPolicy::strict(), Arc::default()).unwrap();
    let mut errors = vec![strict.fetch(&url).await.err().unwrap()];
    serve_status(&server, 404).await;
    errors.push(svc.fetch(&url).await.err().unwrap());
    serve(&server, "<rss").await;
    errors.push(svc.fetch(&url).await.err().unwrap());
    errors.push(validate_feed_url(&format!("ftp://h/{SECRET}")).unwrap_err());
    // Vérification pure (sans l'état global des erreurs de la session) : ce que le diagnostic
    // garderait d'une erreur, et le message tel qu'il serait nettoyé à l'enregistrement.
    let redactor = crate::device::diagnostic::Redactor::current();
    for err in &errors {
        assert_no_secret(&crate::device::diagnostic::session_message(err), &url);
        assert_no_secret(&redactor.clean_message(&err.to_string()), &url);
    }
}
