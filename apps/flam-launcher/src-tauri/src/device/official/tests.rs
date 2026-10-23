//! Tests du catalogue officiel (#290) : serveur HTTP local (wiremock), aucun accès à Internet.
//! Les UUID, titres et jetons sont inventés.

use serde_json::json;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::thumbnail::RgbaImage;

const A: &str = "a701e27e-8f3a-248b-72bd-ff65fa5d440e";
const B: &str = "22222222-2222-4222-8222-222222222222";
const C: &str = "33333333-3333-4333-8333-333333333333";
const D: &str = "44444444-4444-4444-8444-444444444444";
const TOKEN: &str = "jeton-de-test-0123";
/// Numéro de série de la Flam de test : ne doit jamais apparaître dans une requête.
const SERIAL: &str = "00123456789abc";

fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn client() -> reqwest::Client {
    no_redirect_client(Duration::from_secs(2), Duration::from_secs(5)).unwrap()
}

fn service(server: &MockServer) -> OfficialCatalog {
    OfficialCatalog::with_endpoints(
        client(),
        format!("{}/guest/create", server.uri()),
        format!("{}/v2/packs", server.uri()),
        server.uri(),
        false,
    )
}

fn packs_body() -> serde_json::Value {
    json!({ "response": {
        "k1": { "uuid": A.to_uppercase(), "title": "Totally Spies!",
                "localized_infos": { "fr_FR": { "image": { "image_url": "/public/images/packs/a.png" } } } },
        "k2": { "uuid": B, "title": "  La Quête du Micro d'Or \n",
                "localized_infos": { "fr_FR": { "title": "autre" } } },
    }})
}

fn token_body() -> serde_json::Value {
    json!({ "response": { "token": { "server": TOKEN } } })
}

/// Monte le jeton et le catalogue ; `calls` : nombre d'appels attendus de chacun.
async fn mount_api(server: &MockServer, packs: ResponseTemplate, calls: u64) {
    Mock::given(method("GET"))
        .and(path("/guest/create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body()))
        .expect(calls)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/packs"))
        .and(header("X-AUTH-TOKEN", TOKEN))
        .and(header("Application-Sender", "luniistore_desktop"))
        .and(header("Accept", "application/json"))
        .respond_with(packs)
        .expect(calls)
        .mount(server)
        .await;
}

fn ok_packs() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(packs_body())
}

fn ids(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn write_cache(dir: &Path, fetched_at: u64, entries: &[(&str, &str, Option<&str>)]) {
    let file = CacheFile {
        version: CACHE_VERSION,
        fetched_at,
        entries: entries
            .iter()
            .map(|(u, t, i)| {
                (
                    u.to_string(),
                    Entry {
                        title: t.to_string(),
                        image: i.map(str::to_string),
                    },
                )
            })
            .collect(),
    };
    write_atomic(&cache_path(dir), &serde_json::to_vec(&file).unwrap()).unwrap();
}

fn titles_of(reply: &OfficialTitles) -> Vec<(&str, &str)> {
    reply
        .titles
        .iter()
        .map(|t| (t.uuid.as_str(), t.title.as_str()))
        .collect()
}

fn png() -> Vec<u8> {
    let img = RgbaImage {
        width: 4,
        height: 4,
        rgba: vec![200; 4 * 4 * 4],
    };
    thumbnail::encode_png(&img).unwrap()
}

// --- Quand l'API est appelée -----------------------------------------------------------

#[tokio::test]
async fn aucun_appel_sans_histoire_sans_titre() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 0).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&server);
    assert_eq!(
        svc.titles(dir.path(), &[], now()).await,
        OfficialTitles::default()
    );
    // UUID invalides : comme une liste vide.
    let reply = svc
        .titles(dir.path(), &ids(&["pas-un-uuid", ""]), now())
        .await;
    assert!(reply.titles.is_empty());
    assert!(!cache_path(dir.path()).exists());
}

#[tokio::test]
async fn appel_quand_un_titre_manque_et_mise_en_cache() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&server);
    let reply = svc
        .titles(dir.path(), &ids(&[&A.to_uppercase(), B, C]), now())
        .await;
    // UUID insensible à la casse ; titre nettoyé ; C inconnu du catalogue.
    assert_eq!(
        titles_of(&reply),
        [(A, "Totally Spies!"), (B, "La Quête du Micro d'Or")]
    );
    assert!(reply.titles[0].has_image);
    assert!(!reply.titles[1].has_image);
    let cache = read_cache(&cache_path(dir.path())).unwrap();
    assert_eq!(cache.fetched_at, 1_800_000_000);
    assert_eq!(cache.entries.len(), 2);
    // C est absent mais le cache est frais : pas de nouvel appel (le serveur n'en attend qu'un).
    let again = svc.titles(dir.path(), &ids(&[C]), now()).await;
    assert!(again.titles.is_empty());
}

#[tokio::test]
async fn cache_complet_sans_appel_meme_ancien() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 0).await;
    let dir = tempfile::tempdir().unwrap();
    write_cache(dir.path(), 1_000, &[(A, "En cache", None)]);
    let reply = service(&server).titles(dir.path(), &ids(&[A]), now()).await;
    assert_eq!(titles_of(&reply), [(A, "En cache")]);
}

#[tokio::test]
async fn cache_frais_utilise_sans_appel() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 0).await;
    let dir = tempfile::tempdir().unwrap();
    let fresh = unix_secs(now()) - 3600;
    write_cache(dir.path(), fresh, &[(A, "En cache", None)]);
    let reply = service(&server)
        .titles(dir.path(), &ids(&[A, B]), now())
        .await;
    assert_eq!(titles_of(&reply), [(A, "En cache")]);
}

#[tokio::test]
async fn cache_perime_rafraichi() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let old = unix_secs(now()) - CACHE_MAX_AGE.as_secs() - 1;
    write_cache(dir.path(), old, &[(A, "Ancien titre", None)]);
    let reply = service(&server)
        .titles(dir.path(), &ids(&[A, B]), now())
        .await;
    assert_eq!(
        titles_of(&reply),
        [(A, "Totally Spies!"), (B, "La Quête du Micro d'Or")]
    );
    assert_eq!(
        read_cache(&cache_path(dir.path())).unwrap().fetched_at,
        unix_secs(now())
    );
}

#[tokio::test]
async fn cache_date_du_futur_lointain_est_perime() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    write_cache(
        dir.path(),
        unix_secs(now()) + 10 * 86_400,
        &[(A, "x", None)],
    );
    let reply = service(&server).titles(dir.path(), &ids(&[B]), now()).await;
    assert_eq!(titles_of(&reply), [(B, "La Quête du Micro d'Or")]);
}

// --- Replis ----------------------------------------------------------------------------

#[tokio::test]
async fn serveur_injoignable_repli_sur_le_cache_sans_erreur() {
    // Port local fermé : connexion refusée.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap().port()
    };
    let base = format!("http://127.0.0.1:{closed}");
    let svc = OfficialCatalog::with_endpoints(
        client(),
        format!("{base}/guest/create"),
        format!("{base}/v2/packs"),
        base,
        false,
    );
    let dir = tempfile::tempdir().unwrap();
    write_cache(dir.path(), 1_000, &[(A, "En cache", None)]);
    let reply = svc.titles(dir.path(), &ids(&[A, B]), now()).await;
    assert_eq!(titles_of(&reply), [(A, "En cache")]);
    assert!(svc.session_failed());
    // Sans cache : réponse vide, sans erreur.
    let empty = tempfile::tempdir().unwrap();
    assert!(svc
        .titles(empty.path(), &ids(&[A]), now())
        .await
        .titles
        .is_empty());
}

#[tokio::test]
async fn un_echec_n_est_pas_retente_pendant_la_session() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/guest/create"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let svc = service(&server);
    assert!(svc
        .titles(dir.path(), &ids(&[A]), now())
        .await
        .titles
        .is_empty());
    assert!(svc
        .titles(dir.path(), &ids(&[B]), now())
        .await
        .titles
        .is_empty());
}

#[tokio::test]
async fn formats_inattendus_gardent_le_cache() {
    let bodies = [
        json!({ "response": [] }),
        json!({ "response": {} }),
        json!({ "response": { "k": { "uuid": "pas-un-uuid", "title": "x" } } }),
        json!({ "autre": 1 }),
    ];
    for body in bodies {
        let server = MockServer::start().await;
        mount_api(&server, ResponseTemplate::new(200).set_body_json(body), 1).await;
        let dir = tempfile::tempdir().unwrap();
        write_cache(dir.path(), 1_000, &[(A, "En cache", None)]);
        let svc = service(&server);
        let reply = svc.titles(dir.path(), &ids(&[A, B]), now()).await;
        assert_eq!(titles_of(&reply), [(A, "En cache")]);
        // Le cache n'a pas été remplacé (ni sa date).
        assert_eq!(
            read_cache(&cache_path(dir.path())).unwrap().fetched_at,
            1_000
        );
        assert!(svc.session_failed());
    }
    // Pas du JSON du tout.
    let server = MockServer::start().await;
    mount_api(
        &server,
        ResponseTemplate::new(200).set_body_string("<html>"),
        1,
    )
    .await;
    let dir = tempfile::tempdir().unwrap();
    assert!(service(&server)
        .titles(dir.path(), &ids(&[A]), now())
        .await
        .titles
        .is_empty());
}

#[tokio::test]
async fn reponse_trop_grosse_refusee() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let mut svc = service(&server);
    svc.max_packs_bytes = 64;
    let reply = svc.titles(dir.path(), &ids(&[A]), now()).await;
    assert!(reply.titles.is_empty());
    assert!(!cache_path(dir.path()).exists());
    assert!(svc.session_failed());
}

#[tokio::test]
async fn jeton_absent_ou_invalide_est_un_echec_silencieux() {
    for body in [
        json!({ "response": {} }),
        json!({ "response": { "token": { "server": "a b" } } }),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/guest/create"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let dir = tempfile::tempdir().unwrap();
        assert!(service(&server)
            .titles(dir.path(), &ids(&[A]), now())
            .await
            .titles
            .is_empty());
    }
}

#[tokio::test]
async fn https_exige_hors_tests() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 0).await;
    let mut svc = service(&server);
    svc.https_only = true;
    let dir = tempfile::tempdir().unwrap();
    assert!(svc
        .titles(dir.path(), &ids(&[A]), now())
        .await
        .titles
        .is_empty());
    // Les adresses de production sont bien en HTTPS.
    for url in [TOKEN_URL, PACKS_URL, IMAGE_BASE] {
        assert_eq!(Url::parse(url).unwrap().scheme(), "https");
    }
}

// --- Rien de la Flam dans les requêtes -------------------------------------------------

#[tokio::test]
async fn aucune_donnee_d_appareil_dans_les_requetes() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    service(&server)
        .titles(dir.path(), &ids(&[A, B, C]), now())
        .await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    let allowed = [
        "host",
        "accept",
        "application-sender",
        "x-auth-token",
        "user-agent",
    ];
    for request in &requests {
        assert_eq!(request.method.as_str(), "GET");
        assert!(request.url.query().is_none(), "paramètre : {}", request.url);
        assert!(request.body.is_empty());
        for (name, _) in request.headers.iter() {
            assert!(
                allowed.contains(&name.as_str()),
                "en-tête inattendu : {name}"
            );
        }
        let dump = format!("{request:?}").to_lowercase();
        for secret in [A, B, C, SERIAL] {
            assert!(
                !dump.contains(secret),
                "donnée de la Flam envoyée : {secret}"
            );
        }
    }
    // Le jeton n'est envoyé qu'au serveur de catalogue (second appel).
    assert!(requests[0].headers.get("x-auth-token").is_none());
}

// --- Images ----------------------------------------------------------------------------

#[tokio::test]
async fn image_telechargee_puis_servie_depuis_le_cache() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/public/images/packs/a.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(png()))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    write_cache(
        dir.path(),
        unix_secs(now()),
        &[(A, "T", Some("/public/images/packs/a.png"))],
    );
    let svc = service(&server);
    let first = svc.thumbnail(dir.path(), &A.to_uppercase()).await.unwrap();
    assert!(first.starts_with("data:image/png;base64,"));
    // Deuxième demande : cache disque, sans nouvelle requête (le serveur n'en attend qu'une).
    assert_eq!(svc.thumbnail(dir.path(), A).await, Some(first));
}

#[tokio::test]
async fn image_refusee_si_inconnue_trop_grosse_ou_pas_png() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/public/images/packs/svg.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<svg xmlns='x'/>".to_vec()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/public/images/packs/gros.png"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; MAX_THUMB_BYTES + 1]))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/public/images/packs/404.png"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    write_cache(
        dir.path(),
        unix_secs(now()),
        &[
            (A, "T", Some("/public/images/packs/svg.png")),
            (B, "T", Some("/public/images/packs/gros.png")),
            (C, "T", Some("/public/images/packs/404.png")),
        ],
    );
    let svc = service(&server);
    for uuid in [A, B, C] {
        assert_eq!(svc.thumbnail(dir.path(), uuid).await, None, "{uuid}");
    }
    // UUID absent du cache du catalogue : aucune requête.
    let before = server.received_requests().await.unwrap().len();
    assert_eq!(svc.thumbnail(dir.path(), D).await, None);
    assert_eq!(svc.thumbnail(dir.path(), "../../x").await, None);
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

// --- Lecture tolérante -----------------------------------------------------------------

#[test]
fn lecture_tolerante_du_catalogue() {
    let body = json!({ "response": {
        "a": { "uuid": A, "localized_infos": { "en_GB": { "title": "Titre local",
                "image": { "image_url": "/public/images/x.png" } } } },
        "b": { "uuid": B, "title": "Titre B", "localized_infos": {
                "fr_FR": { "image": { "image_url": "https://exemple.test/x.png" } },
                "en_GB": { "image": { "image_url": "/public/images/en.png" } } } },
        "c": { "uuid": C, "title": "   " },
        "d": "pas un objet",
        "e": { "title": "sans uuid" },
        "f": { "uuid": D, "title": "T",
               "localized_infos": { "fr_FR": { "image": { "image_url": "/public/images/../secret.png" } } } },
    }});
    let entries = parse_packs(body.to_string().as_bytes()).unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[A].title, "Titre local");
    assert_eq!(entries[A].image.as_deref(), Some("/public/images/x.png"));
    assert_eq!(entries[B].image.as_deref(), Some("/public/images/en.png"));
    assert_eq!(entries[D].image, None);
}

#[test]
fn chemins_d_image_acceptes() {
    assert!(valid_image_path("/public/images/packs/a-b_c.fr_FR.1.png"));
    for bad in [
        "",
        "/other/x.png",
        "/public/images/../x.png",
        "/public/images//x.png",
        "/public/images/x.png?a=1",
        "/public/images/x y.png",
        "/public/images/é.png",
        "http://h/public/images/x.png",
    ] {
        assert!(!valid_image_path(bad), "{bad}");
    }
}

#[test]
fn titres_nettoyes_et_bornes() {
    assert_eq!(clean_title("  a\u{0}b\n "), Some("ab".into()));
    assert_eq!(clean_title(" \n"), None);
    assert_eq!(
        clean_title(&"x".repeat(500)).unwrap().chars().count(),
        MAX_TITLE_CHARS
    );
}

#[test]
fn cache_au_pire_cas_tient_dans_la_borne_de_taille() {
    // Entrée la plus lourde possible : titre de 200 caractères de 4 octets, image de 256 octets.
    let worst = Entry {
        title: "\u{1F600}".repeat(MAX_TITLE_CHARS),
        image: Some(format!("/public/images/{}", "a".repeat(256 - 15))),
    };
    let entries = (0..MAX_ENTRIES)
        .map(|i| {
            (
                format!("{i:08x}-0000-4000-8000-000000000000"),
                worst.clone(),
            )
        })
        .collect();
    let cache = CacheFile {
        version: CACHE_VERSION,
        fetched_at: 1,
        entries,
    };
    let size = serde_json::to_vec(&cache).unwrap().len() as u64;
    assert!(size <= MAX_CACHE_BYTES, "{size} octets");
}

#[test]
fn titre_du_cache_nettoye_a_la_relecture() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    let long = "x".repeat(MAX_TITLE_CHARS + 50);
    let text = format!(
        r#"{{"version":1,"fetched_at":1,"entries":{{"{A}":{{"t":"  a{}b "}},"{B}":{{"t":"{long}"}},"{C}":{{"t":"  "}}}}}}"#,
        "\\u0001"
    );
    write_atomic(&path, text.as_bytes()).unwrap();
    let cache = read_cache(&path).unwrap();
    assert_eq!(cache.entries[A].title, "ab");
    assert_eq!(cache.entries[B].title.chars().count(), MAX_TITLE_CHARS);
    assert!(!cache.entries.contains_key(C));
}

#[test]
fn cache_corrompu_ou_d_une_autre_version_ignore() {
    let dir = tempfile::tempdir().unwrap();
    let path = cache_path(dir.path());
    write_atomic(&path, b"{ pas du json").unwrap();
    assert!(read_cache(&path).is_none());
    write_atomic(&path, br#"{"version":99,"fetched_at":1,"entries":{}}"#).unwrap();
    assert!(read_cache(&path).is_none());
    // Entrées douteuses écartées à la relecture.
    let text = format!(
        r#"{{"version":1,"fetched_at":1,"entries":{{"{A}":{{"t":"ok","i":"/etc/passwd"}},"XYZ":{{"t":"x"}},"{B}":{{"t":""}}}}}}"#
    );
    write_atomic(&path, text.as_bytes()).unwrap();
    let cache = read_cache(&path).unwrap();
    assert_eq!(cache.entries.len(), 1);
    assert_eq!(cache.entries[A].image, None);
}

#[tokio::test]
async fn aucune_redirection_suivie_et_jeton_non_transmis() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/guest/create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_body()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v2/packs"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("Location", format!("{}/vol", other.uri())),
        )
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let svc = OfficialCatalog::with_endpoints(
        no_redirect_client(Duration::from_secs(2), Duration::from_secs(5)).unwrap(),
        format!("{}/guest/create", server.uri()),
        format!("{}/v2/packs", server.uri()),
        server.uri(),
        false,
    );
    assert!(svc
        .titles(dir.path(), &ids(&[A]), now())
        .await
        .titles
        .is_empty());
    assert!(other.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn cache_non_ecrit_arrete_les_telechargements_de_la_session() {
    let server = MockServer::start().await;
    mount_api(&server, ok_packs(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    // `official` est un fichier : le dossier du cache ne peut pas être créé.
    std::fs::write(dir.path().join(CACHE_DIR), b"x").unwrap();
    let svc = service(&server);
    let reply = svc.titles(dir.path(), &ids(&[A]), now()).await;
    assert_eq!(titles_of(&reply), [(A, "Totally Spies!")]);
    assert!(svc.session_failed());
    // Pas de second téléchargement (le serveur n'en attend qu'un).
    svc.titles(dir.path(), &ids(&[B]), now()).await;
}
