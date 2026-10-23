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
    let dir = tempfile::tempdir().unwrap();
    let progress = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&progress);
    let report = fetcher()
        .fetch_all(
            &[
                item("e1", "feedA", &base, "/a.mp3", Some("/cover.jpg")),
                item("e2", "feedB", &base, "/b.mp3", None),
            ],
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
    let report = fetcher()
        .fetch_all(
            &[
                item("ok", "f", &base, "/a.mp3", Some("/absente.jpg")),
                item("ko", "f", &base, "/absent.mp3", None),
            ],
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
    let err = fetcher()
        .fetch_all(
            &[
                item("x", "f", &base, "/x", None),
                item("y", "f", &base, "/y", None),
            ],
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
    let report = fetcher()
        .fetch_all(
            &[item("e", "f", &base, "/e.mp3", None)],
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
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
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
    let strict = Fetcher::new(NetPolicy::strict()).unwrap();
    for host in ["127.0.0.1", "localhost", "[::1]"] {
        let base = format!("http://{host}:{port}");
        let err = strict
            .fetch_all(
                &[item("x", "f", &base, "/a", None)],
                dir.path(),
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
async fn nom_resolu_vers_bouclage_refuse_par_le_client_garde() {
    // Le nom `localhost` passe `check_url` (nom de domaine) : c'est la résolution DNS du
    // client qui le refuse, avec une erreur reconnaissable.
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
    let dir = tempfile::tempdir().unwrap();
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
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
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
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
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
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
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
            dir.path(),
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
    let dir = tempfile::tempdir().unwrap();
    let f = fetcher();
    let err = f
        .fetch_all(
            &[],
            dir.path(),
            &Limits::default(),
            never_cancel(),
            no_progress(),
        )
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "validation");
    let many: Vec<Item> = (0..=MAX_EPISODES)
        .map(|i| item(&format!("e{i}"), "f", "http://127.0.0.1:9", "/x", None))
        .collect();
    let err = f
        .fetch_all(
            &many,
            dir.path(),
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
}

#[test]
fn dossier_de_travail_supprime_apres_publication() {
    let base = tempfile::tempdir().unwrap();
    let wd = Workdir::create(base.path()).unwrap();
    std::fs::write(wd.path().join("ep-0000.audio"), b"x").unwrap();
    let path = wd.path().to_path_buf();
    wd.finish().unwrap();
    assert!(!path.exists());
    // Abandon : supprimé aussi.
    let wd = Workdir::create(base.path()).unwrap();
    let path = wd.path().to_path_buf();
    drop(wd);
    assert!(!path.exists());
}
