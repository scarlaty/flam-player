//! Parité de la lecture d'archive avec `zipfile` et `telmi2flam.py` : vecteurs produits par
//! `tests/vectors/make_archive_vectors.py` (Python 3.14.8 sous Windows, version de l'oracle).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;
use telmi2flam::archive::{find_root, Archive};
use telmi2flam::info::read_info;

fn vectors_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vectors/archive")
}

fn expected() -> Vec<Value> {
    let raw = fs::read_to_string(vectors_dir().join("expected.json")).unwrap();
    serde_json::from_str::<Value>(&raw)
        .unwrap()
        .as_array()
        .unwrap()
        .clone()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Code d'erreur attendu côté Rust pour une catégorie d'échec Python.
fn rust_code(category: &str) -> &'static str {
    match category {
        "corrupt" | "encrypted" | "python_fails_too" => "archive_unreadable",
        other => panic!("catégorie inconnue {other}"),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest;
    hex(&sha2::Sha256::digest(bytes))
}

/// Les noms avec `\` dépendent du système (`_sanitize_filename`) : vecteurs produits sous
/// Windows.
fn applicable(case: &Value) -> bool {
    cfg!(windows) || case["id"] != "names_backslash_nul"
}

#[test]
fn ouverture_noms_lectures_et_recherche_comme_python() {
    let cases = expected();
    assert!(cases.len() >= 40);
    for case in cases.iter().filter(|c| applicable(c)) {
        let id = case["id"].as_str().unwrap();
        let path = vectors_dir().join(format!("{id}.zip"));
        let opened = Archive::open(&path);
        let open = case["open"].as_str().unwrap();
        if open != "ok" {
            let e = opened
                .err()
                .unwrap_or_else(|| panic!("{id} : ouverture acceptée"));
            assert_eq!(e.code, rust_code(open), "{id}");
            assert_eq!(e.exit_code, 3, "{id}");
            continue;
        }
        let mut archive = opened.unwrap_or_else(|e| panic!("{id} : {e}"));
        let names: Vec<String> = case["namelist"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_str().unwrap().to_string())
            .collect();
        assert_eq!(archive.names(), names.as_slice(), "{id} : namelist");
        for read in case["reads"].as_array().unwrap() {
            let name = read["name"].as_str().unwrap();
            let got = archive.read(name);
            // Écart admis (PORT-RUST §3) : le Rust refuse ce que Python lit.
            let divergence = read.get("rust_error").and_then(Value::as_str);
            let readable = read.get("hex").is_some() || read.get("sha256").is_some();
            if divergence.is_none() && readable {
                let content = got
                    .unwrap_or_else(|e| panic!("{id} / {name:?} : {e}"))
                    .unwrap_or_else(|| panic!("{id} / {name:?} : absent"));
                if let Some(h) = read.get("hex") {
                    assert_eq!(hex(&content), h.as_str().unwrap(), "{id} / {name:?}");
                } else {
                    // Gros contenus (bzip2, LZMA, Zstandard) : taille et empreinte.
                    assert_eq!(content.len() as u64, read["len"].as_u64().unwrap(), "{id}");
                    assert_eq!(
                        sha256_hex(&content),
                        read["sha256"].as_str().unwrap(),
                        "{id}"
                    );
                }
            } else {
                let e = got.expect_err(&format!("{id} / {name:?} : lecture acceptée"));
                let want = divergence.unwrap_or_else(|| read["error"].as_str().unwrap());
                assert_eq!(e.code, rust_code(want), "{id} / {name:?} : {e}");
            }
        }
        for probe in case["lookup"].as_array().unwrap() {
            let p = probe[0].as_str().unwrap();
            let want = probe[1].as_str();
            assert_eq!(archive.lookup(p).as_deref(), want, "{id} : lookup {p:?}");
        }
        match find_root(archive.names()) {
            Ok(root) => assert_eq!(root, case["root"].as_str().unwrap(), "{id}"),
            Err(e) => {
                let want = case["root_error"].as_array().unwrap();
                assert_eq!(e.code, want[0].as_str().unwrap(), "{id}");
                assert_eq!(u64::from(e.exit_code), want[1].as_u64().unwrap(), "{id}");
            }
        }
    }
}

/// `read_info` (bibliothèque) et `telmi2flam <archive> --info` (binaire) contre Python.
fn check_info(case: &Value, path: &Path) {
    let id = case["id"].as_str().unwrap();
    let exit = case["info_exit"].as_u64().unwrap();
    let stdout = case["info_stdout"].as_str().unwrap().trim_end();
    match read_info(path) {
        Ok(info) => {
            assert_eq!(exit, 0, "{id} : --info accepté");
            assert_eq!(info.to_json(), stdout, "{id}");
        }
        Err(e) => assert_eq!(u64::from(e.exit_code), exit, "{id} : {e}"),
    }
    let out = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
        .arg(path)
        .arg("--info")
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(exit as i32),
        "{id} : code du binaire"
    );
    if exit == 0 {
        assert_eq!(
            String::from_utf8(out.stdout).unwrap().trim_end(),
            stdout,
            "{id}"
        );
    } else {
        assert!(out.stdout.is_empty(), "{id}");
    }
}

#[test]
fn info_identique_a_python() {
    for case in expected().iter().filter(|c| applicable(c)) {
        let id = case["id"].as_str().unwrap();
        check_info(case, &vectors_dir().join(format!("{id}.zip")));
    }
}

#[test]
fn chemins_windows_non_ascii_et_longs() {
    let cases = expected();
    let case = cases.iter().find(|c| c["id"] == "story_stored").unwrap();
    let src = vectors_dir().join("story_stored.zip");
    let base = Path::new(env!("CARGO_TARGET_TMPDIR")).join("archive_chemins");
    let _ = fs::remove_dir_all(&base);

    let unicode = base.join("été 漢字 😀");
    fs::create_dir_all(&unicode).unwrap();
    let target = unicode.join("histoire ÉTÉ 🐻.zip");
    fs::copy(&src, &target).unwrap();
    check_info(case, &target);

    // Plus de 260 caractères : la bibliothèque standard ajoute le préfixe \\?\ elle-même.
    let mut long = base.clone();
    for i in 0..12 {
        long = long.join(format!("dossier-très-long-{i:02}-ééééééééé"));
    }
    fs::create_dir_all(&long).unwrap();
    let target = long.join("histoire.zip");
    fs::copy(&src, &target).unwrap();
    assert!(target.as_os_str().len() > 260);
    check_info(case, &target);

    // Archive absente, dossier : archive_unreadable (code 3), comme Python.
    for missing in [unicode.join("absente.zip"), unicode.clone()] {
        let e = read_info(&missing).unwrap_err();
        assert_eq!((e.code, e.exit_code), ("archive_unreadable", 3));
    }
    let _ = fs::remove_dir_all(&base);
}

/// Lit toutes les entrées et `read_info` d'une archive altérée : ni panique ni boucle.
fn exercise(id: &str, v: Vec<u8>) {
    let Ok(mut archive) = Archive::new(std::io::Cursor::new(v), id.to_string()) else {
        return;
    };
    let names = archive.names().to_vec();
    for n in &names {
        let _ = archive.read(n);
    }
    let _ = telmi2flam::info::read_info_from(&mut archive);
}

/// Variantes ciblées : à chaque signature `PK`, chaque mot de 16 et 32 bits des 46 octets
/// suivants (en-têtes local, central, fin de répertoire ; champs ZIP64 de 8 octets non couverts) mis à 0, à tout-à-1, +1 et -1.
fn targeted_mutations(data: &[u8]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    let sigs: Vec<usize> = (0..data.len().saturating_sub(3))
        .filter(|&i| data[i] == b'P' && data[i + 1] == b'K' && data[i + 2] < 8 && data[i + 3] < 8)
        .collect();
    for s in sigs {
        for off in (s + 4..(s + 46).min(data.len())).step_by(2) {
            for width in [2usize, 4] {
                if off + width > data.len() {
                    continue;
                }
                let old = data[off..off + width]
                    .iter()
                    .rev()
                    .fold(0u64, |a, b| (a << 8) | u64::from(*b));
                let mask = if width == 2 { 0xFFFF } else { 0xFFFF_FFFF };
                for new in [
                    0,
                    mask,
                    old.wrapping_add(1) & mask,
                    old.wrapping_sub(1) & mask,
                ] {
                    let mut v = data.to_vec();
                    for (k, b) in v[off..off + width].iter_mut().enumerate() {
                        *b = (new >> (8 * k)) as u8;
                    }
                    out.push(v);
                }
            }
        }
    }
    out
}

/// Vecteurs bzip2/LZMA/Zstandard de #458 (hors les trois historiques, petits) : leur
/// décodage complet coûte cher en debug (bombe de 5 Mo), d'où un test à budget réduit.
fn codec_case(id: &str) -> bool {
    ["bzip2_", "lzma_", "zstd_"]
        .iter()
        .any(|p| id.starts_with(p))
        && !["bzip2_image", "lzma_metadata", "zstd_image"].contains(&id)
}

/// Lance `work` dans un fil, avec un garde-fou de durée (boucle ou lenteur anormale).
fn with_deadline(work: impl FnOnce() + Send + 'static) {
    let worker = std::thread::spawn(work);
    let start = std::time::Instant::now();
    while !worker.is_finished() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(300),
            "robustesse : plus de 300 s (boucle ou lenteur anormale)"
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    worker.join().expect("panique sur une archive altérée");
}

/// Injection de pannes dans les flux bzip2, LZMA et Zstandard (#458) : flux coupés à des
/// positions réparties (suite à zéro), octets du flux remplacés au hasard, `file_size` et CRC
/// altérés dans le répertoire central. Aucune panique, chaque lecture se termine.
#[test]
fn injection_de_pannes_bzip2_lzma_zstd() {
    with_deadline(|| {
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut n = 0;
        for case in expected() {
            let id = case["id"].as_str().unwrap().to_string();
            if !codec_case(&id) {
                continue;
            }
            n += 1;
            let data = fs::read(vectors_dir().join(format!("{id}.zip"))).unwrap();
            // Le flux compressé de images/c.png : de l'en-tête local au répertoire central.
            let name = b"images/c.png";
            let local = data.windows(name.len()).position(|w| w == name).unwrap();
            let start = local + name.len();
            let end = data.windows(4).position(|w| w == b"PK\x01\x02").unwrap();
            let bomb = id.contains("bomb");
            let tries = if bomb { 3 } else { 12 };
            for k in 0..tries {
                // Flux coupé à `cut` et complété par des zéros (en-têtes intacts) ; les
                // troncatures avec contenu partiel sont dans les vecteurs `*_truncated*`.
                let cut = start + (end - start) * k / tries;
                let mut v = data[..cut].to_vec();
                v.extend(std::iter::repeat_n(0u8, end - cut));
                v.extend_from_slice(&data[end..]);
                exercise(&id, v);
                if end > start {
                    let mut v = data.clone();
                    for _ in 0..1 + next() % 3 {
                        let i = start + (next() % (end - start) as u64) as usize;
                        v[i] = (next() & 0xFF) as u8;
                    }
                    exercise(&id, v);
                }
            }
            // file_size (central +24) et CRC (central +16) de l'entrée altérés.
            let central = data.windows(name.len()).rposition(|w| w == name).unwrap() - 46;
            assert_eq!(&data[central..central + 4], b"PK\x01\x02", "{id}");
            for (off, val) in [(24usize, 0u32), (24, 1), (24, u32::MAX), (16, 0), (16, 1)] {
                let mut v = data.clone();
                v[central + off..central + off + 4].copy_from_slice(&val.to_le_bytes());
                exercise(&id, v);
            }
        }
        assert!(n >= 60, "{n} vecteurs bzip2/LZMA/Zstandard");
    });
}

/// Entrées hostiles : troncatures, octets aléatoires et mutations ciblées des en-têtes de
/// chaque vecteur. Aucune panique, et chaque lecture se termine (succès ou erreur de
/// conversion) ; un garde-fou de durée échoue au lieu de laisser le test tourner sans fin.
#[test]
fn aucune_panique_sur_archive_alteree() {
    with_deadline(|| {
        let mut seed: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for case in expected() {
            let id = case["id"].as_str().unwrap().to_string();
            if codec_case(&id) {
                continue; // budget réduit : injection_de_pannes_bzip2_lzma_zstd
            }
            let data = fs::read(vectors_dir().join(format!("{id}.zip"))).unwrap();
            // Les gros vecteurs (deflate de plus de 64 Kio) : pas de troncature à chaque 7e octet.
            let step = (data.len() / 1500).max(7);
            for cut in (0..data.len()).step_by(step) {
                exercise(&id, data[..cut].to_vec());
            }
            if data.is_empty() {
                continue;
            }
            for _ in 0..(300 * 2000 / data.len().max(2000)).max(20) {
                let mut v = data.clone();
                for _ in 0..1 + next() % 4 {
                    let i = (next() % v.len() as u64) as usize;
                    v[i] = (next() & 0xFF) as u8;
                }
                exercise(&id, v);
            }
            for v in targeted_mutations(&data) {
                exercise(&id, v);
            }
        }
    });
}

/// `--info` avec la sortie standard fermée côté lecteur : pas de panique (`println!` en
/// aurait une sur un tube fermé).
#[test]
fn info_sortie_fermee_sans_panique() {
    let path = vectors_dir().join("story_stored.zip");
    for _ in 0..5 {
        let mut child = Command::new(env!("CARGO_BIN_EXE_telmi2flam"))
            .arg(&path)
            .arg("--info")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let out = child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!stderr.contains("panicked"), "{stderr}");
        assert_eq!(out.status.code(), Some(0), "{stderr}");
    }
}
