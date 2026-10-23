//! Tests du téléchargement par lot, contre un serveur HTTP local de test (bouclage autorisé
//! par `NetPolicy::loopback_for_tests`, jamais en production).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use super::*;

#[derive(Debug, Clone)]
struct Req {
    path: String,
    range_start: Option<u64>,
}

enum Resp {
    Ok(Vec<u8>),
    /// Réponse complète, avec `ETag`, honorant `Range` (206).
    Ranged(Vec<u8>),
    /// Annonce la taille complète mais coupe après `sent` octets (avec `ETag`).
    Cut(Vec<u8>, usize),
    /// Envoie 10 octets puis se tait longtemps.
    Hang,
    /// Annonce 100 000 octets, en envoie un toutes les 30 ms.
    Trickle,
    /// Corps « chunked » interrompu (pas de chunk final) puis fermeture.
    ChunkedCut,
    /// Comme `Ranged`, mais la partie demandée est envoyée octet par octet, un toutes les
    /// `Duration`.
    RangedSlow(Vec<u8>, Duration),
    Status(u16),
    Redirect(String),
}

type Handler = Arc<dyn Fn(&Req) -> Resp + Send + Sync>;

async fn serve(handler: Handler) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let handler = Arc::clone(&handler);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut tmp = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut tmp).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let path = text
                    .lines()
                    .next()
                    .and_then(|l| l.split(' ').nth(1))
                    .unwrap_or("/")
                    .to_string();
                let range_start = text.lines().find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    if !k.eq_ignore_ascii_case("range") {
                        return None;
                    }
                    v.trim()
                        .strip_prefix("bytes=")?
                        .split('-')
                        .next()?
                        .parse()
                        .ok()
                });
                let resp = handler(&Req { path, range_start });
                let head = |status: &str, extra: &str, len: usize| {
                    format!("HTTP/1.1 {status}\r\n{extra}Content-Length: {len}\r\nConnection: close\r\n\r\n")
                };
                match resp {
                    Resp::Ok(body) => {
                        let _ = socket
                            .write_all(head("200 OK", "", body.len()).as_bytes())
                            .await;
                        let _ = socket.write_all(&body).await;
                    }
                    Resp::Ranged(body) => match range_start {
                        Some(start) => {
                            let start = start as usize;
                            let part = &body[start..];
                            let extra = format!(
                                "ETag: \"v1\"\r\nContent-Range: bytes {start}-{}/{}\r\n",
                                body.len() - 1,
                                body.len()
                            );
                            let _ = socket
                                .write_all(
                                    head("206 Partial Content", &extra, part.len()).as_bytes(),
                                )
                                .await;
                            let _ = socket.write_all(part).await;
                        }
                        None => {
                            let _ = socket
                                .write_all(
                                    head("200 OK", "ETag: \"v1\"\r\n", body.len()).as_bytes(),
                                )
                                .await;
                            let _ = socket.write_all(&body).await;
                        }
                    },
                    Resp::Cut(body, sent) => {
                        let _ = socket
                            .write_all(head("200 OK", "ETag: \"v1\"\r\n", body.len()).as_bytes())
                            .await;
                        let _ = socket.write_all(&body[..sent]).await;
                        let _ = socket.flush().await;
                    }
                    Resp::Hang => {
                        let _ = socket.write_all(head("200 OK", "", 1000).as_bytes()).await;
                        let _ = socket.write_all(&[7u8; 10]).await;
                        let _ = socket.flush().await;
                        tokio::time::sleep(Duration::from_secs(60)).await;
                    }
                    Resp::Trickle => {
                        let _ = socket
                            .write_all(head("200 OK", "", 100_000).as_bytes())
                            .await;
                        for _ in 0..200 {
                            if socket.write_all(&[9u8]).await.is_err() {
                                return;
                            }
                            let _ = socket.flush().await;
                            tokio::time::sleep(Duration::from_millis(30)).await;
                        }
                    }
                    Resp::ChunkedCut => {
                        let _ = socket
                            .write_all(
                                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\nhello\r\n",
                            )
                            .await;
                        let _ = socket.flush().await;
                    }
                    Resp::RangedSlow(body, every) => {
                        let start = range_start.unwrap_or(0) as usize;
                        let part = &body[start..];
                        let extra = format!(
                            "ETag: \"v1\"\r\nContent-Range: bytes {start}-{}/{}\r\n",
                            body.len() - 1,
                            body.len()
                        );
                        let status = if start > 0 {
                            "206 Partial Content"
                        } else {
                            "200 OK"
                        };
                        let _ = socket
                            .write_all(head(status, &extra, part.len()).as_bytes())
                            .await;
                        for byte in part {
                            if socket.write_all(&[*byte]).await.is_err() {
                                return;
                            }
                            let _ = socket.flush().await;
                            tokio::time::sleep(every).await;
                        }
                    }
                    Resp::Status(code) => {
                        let _ = socket
                            .write_all(head(&format!("{code} X"), "", 0).as_bytes())
                            .await;
                    }
                    Resp::Redirect(to) => {
                        let extra = format!("Location: {to}\r\n");
                        let _ = socket
                            .write_all(head("302 Found", &extra, 0).as_bytes())
                            .await;
                    }
                }
            });
        }
    });
    port
}

fn bytes(seed: u8, len: usize) -> Vec<u8> {
    (0..len)
        .map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed))
        .collect()
}

fn sha(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn item(id: &str, feed: &str, base: &str, audio: &str, image: Option<&str>) -> Item {
    Item {
        id: id.into(),
        feed: feed.into(),
        audio_url: format!("{base}{audio}"),
        image_url: image.map(|i| format!("{base}{i}")),
    }
}

fn no_progress() -> Arc<dyn Fn(Progress) + Send + Sync> {
    Arc::new(|_| {})
}

fn fetcher() -> Fetcher {
    Fetcher::new(NetPolicy::loopback_for_tests()).unwrap()
}

fn files(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
        .collect()
}

fn never_cancel() -> watch::Receiver<bool> {
    let (tx, rx) = watch::channel(false);
    std::mem::forget(tx);
    rx
}

#[tokio::test(flavor = "multi_thread")]
async fn lot_de_deux_flux_complet_taille_et_empreinte() {
    let a = bytes(1, 50_000);
    let b = bytes(2, 70_000);
    let img = bytes(3, 4_000);
    let (a2, b2, img2) = (a.clone(), b.clone(), img.clone());
    let port = serve(Arc::new(move |r| match r.path.as_str() {
        "/a.mp3" => Resp::Ok(a2.clone()),
        "/b.mp3" => Resp::Ok(b2.clone()),
        "/cover.jpg" => Resp::Ok(img2.clone()),
        _ => Resp::Status(404),
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&progress);
    let report = fetcher()
        .fetch_all(
            &[
                item("e1", "feedA", &base, "/a.mp3", Some("/cover.jpg")),
                item("e2", "feedB", &base, "/b.mp3", None),
            ],
            &dir,
            &Limits::default(),
            never_cancel(),
            Arc::new(move |p| sink.lock().unwrap().push(p)),
        )
        .await
        .unwrap();
    assert_eq!(report.fetched.len(), 2);
    assert!(report.dropped.is_empty() && report.image_failures.is_empty());
    assert_eq!(report.fetched[0].feed, "feedA");
    assert_eq!(report.fetched[1].feed, "feedB");
    let first = std::fs::read(&report.fetched[0].audio).unwrap();
    assert_eq!(first.len(), a.len());
    assert_eq!(sha(&first), sha(&a));
    assert_eq!(report.fetched[0].audio_bytes, a.len() as u64);
    assert_eq!(
        sha(&std::fs::read(report.fetched[0].image.as_ref().unwrap()).unwrap()),
        sha(&img)
    );
    assert_eq!(
        sha(&std::fs::read(&report.fetched[1].audio).unwrap()),
        sha(&b)
    );
    assert!(report.fetched[1].image.is_none());
    assert!(!files(dir.path()).iter().any(|f| f.ends_with(".part")));
    let last = *progress.lock().unwrap().last().unwrap();
    assert_eq!(last.items_total, 2);
    assert_eq!(last.items_done, 2);
    assert_eq!(last.bytes, (a.len() + b.len() + img.len()) as u64);
}

#[tokio::test(flavor = "multi_thread")]
async fn episode_en_404_retire_et_signale_pochette_en_404_ignoree() {
    let a = bytes(1, 1000);
    let a2 = a.clone();
    let port = serve(Arc::new(move |r| match r.path.as_str() {
        "/a.mp3" => Resp::Ok(a2.clone()),
        _ => Resp::Status(404),
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let report = fetcher()
        .fetch_all(
            &[
                item("ok", "f", &base, "/a.mp3", Some("/absente.jpg")),
                item("ko", "f", &base, "/absent.mp3", None),
            ],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.fetched.len(), 1);
    assert_eq!(report.fetched[0].id, "ok");
    assert!(report.fetched[0].image.is_none());
    assert_eq!(report.image_failures, vec!["ok".to_string()]);
    assert_eq!(
        report.dropped,
        vec![Dropped {
            id: "ko".into(),
            feed: "f".into(),
            reason: DropReason::Unreachable
        }]
    );
    assert_eq!(files(dir.path()), vec!["ep-0000.audio".to_string()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn echec_global_si_aucun_episode_recuperable() {
    let port = serve(Arc::new(|_| Resp::Status(404))).await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let err = fetcher()
        .fetch_all(
            &[
                item("x", "f", &base, "/x", None),
                item("y", "f", &base, "/y", None),
            ],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "download_invalid");
    assert!(files(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn coupure_puis_reprise_complete_le_fichier() {
    let full = bytes(9, 100_000);
    let full2 = full.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let ranges = Arc::new(Mutex::new(Vec::new()));
    let (c2, r2) = (Arc::clone(&calls), Arc::clone(&ranges));
    let port = serve(Arc::new(move |r| {
        r2.lock().unwrap().push(r.range_start);
        if c2.fetch_add(1, Ordering::SeqCst) == 0 {
            Resp::Cut(full2.clone(), 40_000)
        } else {
            Resp::Ranged(full2.clone())
        }
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let report = fetcher()
        .fetch_all(
            &[item("e", "f", &base, "/e.mp3", None)],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    let got = std::fs::read(&report.fetched[0].audio).unwrap();
    assert_eq!(sha(&got), sha(&full));
    let ranges = ranges.lock().unwrap();
    assert_eq!(ranges.len(), 2);
    assert_eq!(ranges[0], None);
    assert!(
        ranges[1].is_some_and(|s| s > 0 && s <= 40_000),
        "{ranges:?}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn adresses_privees_refusees_litterales_et_apres_redirection() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&hits);
    let port = serve(Arc::new(move |r| {
        h2.fetch_add(1, Ordering::SeqCst);
        match r.path.as_str() {
            "/ok" => Resp::Ok(vec![1; 10]),
            "/to-meta" => Resp::Redirect("http://169.254.169.254/latest/meta-data".into()),
            "/to-lan" => Resp::Redirect("http://192.168.1.1/admin".into()),
            "/to-v6" => Resp::Redirect("http://[fd00::1]/x".into()),
            "/to-mapped" => Resp::Redirect("http://[::ffff:10.0.0.1]/x".into()),
            "/to-cgnat" => Resp::Redirect("http://100.64.0.1/x".into()),
            "/to-ftp" => Resp::Redirect("ftp://example.org/x".into()),
            _ => Resp::Status(404),
        }
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let items = [
        item("meta", "f", &base, "/to-meta", None),
        item("lan", "f", &base, "/to-lan", None),
        item("v6", "f", &base, "/to-v6", None),
        item("mapped", "f", &base, "/to-mapped", None),
        item("cgnat", "f", &base, "/to-cgnat", None),
        item("ftp", "f", &base, "/to-ftp", None),
        Item {
            id: "literal".into(),
            feed: "f".into(),
            audio_url: "http://10.0.0.7/a.mp3".into(),
            image_url: None,
        },
        item("fine", "f", &base, "/ok", None),
    ];
    let report = fetcher()
        .fetch_all(
            &items,
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.fetched.len(), 1);
    assert_eq!(report.fetched[0].id, "fine");
    assert_eq!(report.dropped.len(), 7);
    for d in &report.dropped {
        assert_eq!(d.reason, DropReason::Blocked, "{}", d.id);
    }
    // Six redirections lues + « fine » : aucune requête n'a atteint les cibles privées.
    assert_eq!(hits.load(Ordering::SeqCst), 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn politique_stricte_refuse_le_bouclage_sans_requete() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&hits);
    let port = serve(Arc::new(move |_| {
        h2.fetch_add(1, Ordering::SeqCst);
        Resp::Ok(vec![1; 10])
    }))
    .await;
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let strict = Fetcher::new(NetPolicy::strict()).unwrap();
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let base = format!("http://{host}:{port}");
        let err = strict
            .fetch_all(
                &[item("x", "f", &base, "/a", None)],
                &dir,
                &Limits::default(),
                never_cancel(),
                no_progress(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "download_invalid", "{host}");
    }
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "aucune connexion ne doit aboutir"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn nom_local_refuse_par_le_resolveur_du_client_garde() {
    // Requête directe sur le client (sans `check_url`) : son résolveur refuse le nom local
    // avant toute résolution, avec une erreur reconnaissable. Le filtrage des adresses
    // résolues est testé sur `net::resolve_host`.
    let client = guarded_client(NetPolicy::strict(), CONNECT_TIMEOUT, READ_TIMEOUT).unwrap();
    let err = client.get("http://localhost:9/").send().await.unwrap_err();
    assert!(crate::podcast::net::is_blocked(&err), "{err:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn annulation_supprime_les_fichiers_partiels() {
    let port = serve(Arc::new(|r| match r.path.as_str() {
        "/fast" => Resp::Ok(vec![1; 100]),
        _ => Resp::Hang,
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let (tx, rx) = watch::channel(false);
    let cancel = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(600)).await;
        tx.send(true).unwrap();
    });
    let err = fetcher()
        .fetch_all(
            &[
                item("a", "f", &base, "/fast", None),
                item("b", "f", &base, "/slow1", None),
                item("c", "f", &base, "/slow2", None),
            ],
            &dir,
            &Limits::default(),
            rx,
            no_progress(),
        )
        .await
        .unwrap_err();
    cancel.await.unwrap();
    assert_eq!(err.kind(), "cancelled");
    assert!(files(dir.path()).is_empty(), "{:?}", files(dir.path()));
}

#[tokio::test(flavor = "multi_thread")]
async fn plafond_total_arrete_le_lot_et_supprime_les_fichiers() {
    let port = serve(Arc::new(|_| Resp::Ok(vec![5; 80]))).await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let limits = Limits {
        max_total_bytes: 100,
        ..Limits::default()
    };
    let err = fetcher()
        .fetch_all(
            &[
                item("a", "f", &base, "/1", None),
                item("b", "f", &base, "/2", None),
                item("c", "f", &base, "/3", None),
            ],
            &dir,
            &limits,
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "download_invalid");
    assert!(err.to_string().contains("plafond"), "{err}");
    assert!(files(dir.path()).is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn plafond_par_episode_retire_l_episode() {
    let port = serve(Arc::new(|r| match r.path.as_str() {
        "/big" => Resp::Ok(vec![1; 300]),
        _ => Resp::Ok(vec![2; 50]),
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let limits = Limits {
        max_episode_bytes: 100,
        ..Limits::default()
    };
    let report = fetcher()
        .fetch_all(
            &[
                item("big", "f", &base, "/big", None),
                item("small", "f", &base, "/small", None),
            ],
            &dir,
            &limits,
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.fetched.len(), 1);
    assert_eq!(report.dropped[0].id, "big");
    assert_eq!(report.dropped[0].reason, DropReason::TooLarge);
    assert_eq!(files(dir.path()), vec!["ep-0001.audio".to_string()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn delai_depasse_retire_l_episode() {
    let port = serve(Arc::new(|r| match r.path.as_str() {
        "/slow" => Resp::Hang,
        _ => Resp::Ok(vec![2; 50]),
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let limits = Limits {
        file_timeout: Duration::from_millis(400),
        ..Limits::default()
    };
    let report = fetcher()
        .fetch_all(
            &[
                item("slow", "f", &base, "/slow", None),
                item("fast", "f", &base, "/fast", None),
            ],
            &dir,
            &limits,
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.dropped[0].reason, DropReason::TimedOut);
    assert_eq!(report.fetched[0].id, "fast");
    assert_eq!(files(dir.path()), vec!["ep-0001.audio".to_string()]);
}

#[tokio::test]
async fn trop_d_episodes_ou_lot_vide_refuses() {
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let f = fetcher();
    let err = f
        .fetch_all(&[], &dir, &Limits::default(), never_cancel(), no_progress())
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "validation");
    let many: Vec<Item> = (0..=MAX_EPISODES)
        .map(|i| item(&format!("e{i}"), "f", "http://127.0.0.1:9", "/x", None))
        .collect();
    let err = f
        .fetch_all(
            &many,
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "validation");
}

#[test]
fn le_debug_d_un_item_n_affiche_pas_les_url() {
    let it = Item {
        id: "e".into(),
        feed: "f".into(),
        audio_url: "https://h.example/SECRET-TOKEN.mp3".into(),
        image_url: Some("https://h.example/SECRET-IMG.jpg".into()),
    };
    assert!(!format!("{it:?}").contains("SECRET"));
    let it = Item {
        id: "https://h.example/SECRET-GUID".into(),
        feed: "https://h.example/f?key=SECRET".into(),
        audio_url: String::new(),
        image_url: None,
    };
    let shown = format!("{it:?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(shown.contains(NEUTRAL_FEED_LABEL));
}

#[tokio::test]
async fn dossier_de_travail_supprime_apres_publication() {
    let base = tempfile::tempdir().unwrap();
    let wd = Workdir::create(base.path()).unwrap();
    std::fs::write(wd.path().join("ep-0000.audio"), b"x").unwrap();
    let path = wd.path().to_path_buf();
    wd.finish().await.unwrap();
    assert!(!path.exists());
}

#[test]
fn dossier_de_travail_supprime_a_l_abandon() {
    let base = tempfile::tempdir().unwrap();
    let wd = Workdir::create(base.path()).unwrap();
    let path = wd.path().to_path_buf();
    drop(wd);
    assert!(!path.exists());
}

#[tokio::test]
async fn id_qui_est_une_url_refuse_sans_la_recopier() {
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    for (id, feed) in [
        ("https://h.example/SECRET-GUID.mp3?t=SECRET", "f"),
        ("ftp://SECRET@h.example/x", "f"),
    ] {
        let err = fetcher()
            .fetch_all(
                &[item(id, feed, "http://127.0.0.1:9", "/a", None)],
                &dir,
                &Limits::default(),
                never_cancel(),
                no_progress(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "validation", "{id}");
        assert!(!err.to_string().contains("SECRET"), "{err}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn identifiants_dans_l_url_d_enclosure_refuses_sans_requete() {
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&hits);
    let port = serve(Arc::new(move |_| {
        h2.fetch_add(1, Ordering::SeqCst);
        Resp::Ok(vec![1; 10])
    }))
    .await;
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let base = format!("http://127.0.0.1:{port}");
    let report = fetcher()
        .fetch_all(
            &[
                Item {
                    id: "cred".into(),
                    feed: "f".into(),
                    audio_url: format!("http://user:pass@127.0.0.1:{port}/a.mp3"),
                    image_url: None,
                },
                item("ok", "f", &base, "/b.mp3", None),
            ],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.dropped.len(), 1);
    assert_eq!(report.dropped[0].reason, DropReason::Blocked);
    assert_eq!(hits.load(Ordering::SeqCst), 1, "seul « ok » a été demandé");
}

#[tokio::test(flavor = "multi_thread")]
async fn debit_minimal_retire_l_episode_lent() {
    let port = serve(Arc::new(|r| match r.path.as_str() {
        "/slow" => Resp::Trickle,
        _ => Resp::Ok(vec![2; 50]),
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    // ~33 octets/s reçus ; minimum exigé : 1000 octets/s sur 500 ms.
    let limits = Limits {
        min_rate_bytes_per_sec: 1000,
        min_rate_window: Duration::from_millis(500),
        ..Limits::default()
    };
    let started = std::time::Instant::now();
    let report = fetcher()
        .fetch_all(
            &[
                item("slow", "f", &base, "/slow", None),
                item("fast", "f", &base, "/fast", None),
            ],
            &dir,
            &limits,
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(report.dropped.len(), 1);
    assert_eq!(report.dropped[0].id, "slow");
    assert_eq!(report.dropped[0].reason, DropReason::TimedOut);
    assert_eq!(report.fetched[0].id, "fast");
    assert_eq!(files(dir.path()), vec!["ep-0001.audio".to_string()]);
}

#[tokio::test(flavor = "multi_thread")]
async fn debit_minimal_inactif_si_fenetre_nulle() {
    // Fenêtre nulle : pas de contrôle de débit ; le délai du fichier reste le seul garde-fou.
    let port = serve(Arc::new(|_| Resp::Trickle)).await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let limits = Limits {
        min_rate_window: Duration::ZERO,
        file_timeout: Duration::from_millis(500),
        ..Limits::default()
    };
    let started = std::time::Instant::now();
    let err = fetcher()
        .fetch_all(
            &[item("a", "f", &base, "/a", None)],
            &dir,
            &limits,
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "download_invalid");
    assert!(started.elapsed() >= Duration::from_millis(450));
}

#[test]
fn debit_minimal_par_defaut_1_kio_s_sur_2_minutes() {
    let l = Limits::default();
    assert_eq!(l.min_rate_bytes_per_sec, 1024);
    assert_eq!(l.min_rate_window, Duration::from_secs(120));
}

#[tokio::test(flavor = "multi_thread")]
async fn corps_chunked_coupe_sans_longueur_est_detecte() {
    let port = serve(Arc::new(|_| Resp::ChunkedCut)).await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let err = fetcher()
        .fetch_all(
            &[item("a", "f", &base, "/a", None)],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "download_invalid");
    assert!(files(dir.path()).is_empty(), "{:?}", files(dir.path()));
}

#[tokio::test(flavor = "multi_thread")]
async fn flux_qui_est_une_url_remplace_par_un_libelle_neutre() {
    let port = serve(Arc::new(|_| Resp::Ok(vec![4; 20]))).await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let report = fetcher()
        .fetch_all(
            &[
                item("e1", "https://h.example/feed?key=SECRET", &base, "/a", None),
                item("e2", "Mon flux", &base, "/b", None),
                item("e3", "http://SECRET.example/x", &base, "/c", None),
            ],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.fetched.len(), 3);
    assert_eq!(report.fetched[0].feed, NEUTRAL_FEED_LABEL);
    assert_eq!(report.fetched[1].feed, "Mon flux");
    assert_eq!(report.fetched[2].feed, NEUTRAL_FEED_LABEL);
    assert!(!format!("{report:?}").contains("SECRET"));
}

#[tokio::test(flavor = "multi_thread")]
async fn noms_locaux_refuses_sans_requete() {
    // Politique de test (bouclage permis) : sans le refus des noms locaux, `a.localhost`
    // pourrait joindre le serveur, et un nom inconnu donnerait `Unreachable` (échec DNS) au
    // lieu de `Blocked`.
    let hits = Arc::new(AtomicUsize::new(0));
    let h2 = Arc::clone(&hits);
    let port = serve(Arc::new(move |_| {
        h2.fetch_add(1, Ordering::SeqCst);
        Resp::Ok(vec![1; 10])
    }))
    .await;
    let ok_base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let hosts = [
        "nas",
        "box",
        "nas.local",
        "x.lan",
        "x.home.arpa",
        "a.internal",
        "a.localhost",
        "serveur.test",
        "x.invalid",
    ];
    let mut items: Vec<Item> = hosts
        .iter()
        .map(|h| item(h, "f", &format!("http://{h}:{port}"), "/a.mp3", None))
        .collect();
    items.push(item("ok", "f", &ok_base, "/a.mp3", None));
    let report = fetcher()
        .fetch_all(
            &items,
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(report.fetched.len(), 1);
    assert_eq!(report.dropped.len(), hosts.len());
    for d in &report.dropped {
        assert_eq!(d.reason, DropReason::Blocked, "{}", d.id);
    }
    assert_eq!(hits.load(Ordering::SeqCst), 1, "seul « ok » a été demandé");
}

#[tokio::test(flavor = "multi_thread")]
async fn delai_du_lot_arrete_tout_et_supprime_les_fichiers() {
    // Chaque fichier respecte son délai (30 min) et le débit minimal est désactivé : seul le
    // délai du lot arrête le serveur muet.
    let port = serve(Arc::new(|r| match r.path.as_str() {
        "/fast" => Resp::Ok(vec![1; 100]),
        _ => Resp::Hang,
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let limits = Limits {
        min_rate_window: Duration::ZERO,
        batch_timeout: Duration::from_millis(500),
        ..Limits::default()
    };
    let started = std::time::Instant::now();
    let fetcher = fetcher();
    let items = [
        item("fast", "f", &base, "/fast", None),
        item("hang", "f", &base, "/hang", None),
    ];
    let batch = fetcher.fetch_all(&items, &dir, &limits, never_cancel(), no_progress());
    // Garde-fou : sans délai du lot, le test échoue au lieu de bloquer 30 min.
    let err = tokio::time::timeout(Duration::from_secs(10), batch)
        .await
        .expect("le délai du lot n'a pas arrêté le lot")
        .unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(err.kind(), "download_invalid");
    assert!(err.to_string().contains("délai du lot"), "{err}");
    assert!(files(dir.path()).is_empty(), "{:?}", files(dir.path()));
}

#[test]
fn delai_du_lot_par_defaut_2_heures() {
    assert_eq!(Limits::default().batch_timeout, Duration::from_secs(7200));
}

#[tokio::test(flavor = "multi_thread")]
async fn debit_minimal_apres_reprise_ignore_les_octets_deja_recus() {
    // 2 000 octets reçus puis coupure ; la reprise n'envoie ensuite qu'un octet toutes les
    // 80 ms (20 octets en 1,6 s). Minimum : 100 octets/s sur 1 s. Si les 2 000 octets du
    // `.part` comptaient dans la première fenêtre, l'épisode passerait (fin à 1,6 s).
    let full = bytes(5, 2_020);
    let full2 = full.clone();
    let calls = Arc::new(AtomicUsize::new(0));
    let c2 = Arc::clone(&calls);
    let port = serve(Arc::new(move |r| match r.path.as_str() {
        "/ok" => Resp::Ok(vec![3; 10]),
        _ if c2.fetch_add(1, Ordering::SeqCst) == 0 => Resp::Cut(full2.clone(), 2_000),
        _ => Resp::RangedSlow(full2.clone(), Duration::from_millis(80)),
    }))
    .await;
    let base = format!("http://127.0.0.1:{port}");
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    let limits = Limits {
        min_rate_bytes_per_sec: 100,
        min_rate_window: Duration::from_secs(1),
        ..Limits::default()
    };
    let report = fetcher()
        .fetch_all(
            &[
                item("lent", "f", &base, "/lent", None),
                item("ok", "f", &base, "/ok", None),
            ],
            &dir,
            &limits,
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 2, "une reprise a eu lieu");
    assert_eq!(report.dropped.len(), 1, "{report:?}");
    assert_eq!(report.dropped[0].id, "lent");
    assert_eq!(report.dropped[0].reason, DropReason::TimedOut);
}

#[tokio::test]
async fn guid_sans_hote_est_accepte() {
    let base_dir = tempfile::tempdir().unwrap();
    let dir = Workdir::create(base_dir.path()).unwrap();
    // `tag:` et `urn:` ne sont pas des URL réseau : pas de refus de validation (le lot échoue
    // ensuite pour une autre raison : serveur injoignable).
    let err = fetcher()
        .fetch_all(
            &[item(
                "tag:soundcloud,2010:tracks/1",
                "Mythes:Grecs",
                "http://127.0.0.1:9",
                "/a",
                None,
            )],
            &dir,
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "download_invalid");
}
