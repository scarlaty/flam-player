//! Parité avec les modules Python : vecteurs produits par `gen_vectors.py` (supprimé)
//! (`lif.encode`, `lif.quantize_rgba`, `mp3map.build`), comparés à l'octet près.

use std::path::{Path, PathBuf};

use telmi2flam::{lif, mp3map};

fn dir(sub: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/vectors")
        .join(sub)
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{} : {e}", path.display()))
}

/// Lignes d'un manifeste, sans commentaires, découpées sur les tabulations.
fn rows(path: &Path) -> Vec<Vec<String>> {
    String::from_utf8(read(path))
        .unwrap()
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split('\t').map(str::to_owned).collect())
        .collect()
}

fn unhex(s: &str) -> Vec<u8> {
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
        .collect()
}

/// Même représentation que `digest` de `gen_vectors.py`.
fn digest(out: &[u8]) -> String {
    if out.len() <= 256 {
        return out.iter().map(|b| format!("{b:02x}")).collect();
    }
    let mut h: u64 = 0xCBF2_9CE4_8422_2325;
    for &b in out {
        h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01B3);
    }
    format!("fnv1a64:{h:016x}:{}", out.len())
}

#[test]
fn lif_identique_a_python() {
    let d = dir("lif");
    let manifest = rows(&d.join("manifest.tsv"));
    assert!(manifest.len() >= 10);
    let mut transposed_seen = 0;
    for row in manifest {
        let (name, w, h, transposed) = match row.as_slice() {
            [name, w, h] => (name, w, h, false),
            [name, w, h, flag] if flag == "transposee" => (name, w, h, true),
            _ => panic!("ligne invalide : {row:?}"),
        };
        let (w, h): (u32, u32) = (w.parse().unwrap(), h.parse().unwrap());
        let rgba = read(&d.join(format!("{name}.rgba")));
        let expected = read(&d.join(format!("{name}.lif")));
        let quantized = read(&d.join(format!("{name}.q.rgba")));

        assert_eq!(lif::encode(&rgba, w, h).unwrap(), expected, "encode {name}");
        assert_eq!(
            lif::quantize_rgba(&rgba, w, h).unwrap(),
            quantized,
            "quantize_rgba {name}"
        );

        // Aller-retour : le décodeur restitue l'image quantifiée, sauf image vide (refusée)
        // et image marquée `transposee` (voir ci-dessous).
        if w > 0 && h > 0 {
            let img = lif::decode(&expected).unwrap();
            assert_eq!((img.width, img.height), (w, h), "decode {name}");
            if transposed {
                // Heuristique du décodeur C : stockée en colonnes, l'image est transposée.
                // Python `lif.decode` ne transpose pas : la référence est `.t.rgba`.
                let want = read(&d.join(format!("{name}.t.rgba")));
                assert_eq!(img.rgba, want, "decode transposé {name}");
                assert_ne!(img.rgba, quantized, "la transposition doit changer {name}");
                transposed_seen += 1;
            } else {
                assert_eq!(img.rgba, quantized, "decode {name}");
            }
        }

        // Décodage de chaque préfixe et de chaque octet altéré : jamais de panique.
        for len in 0..expected.len() {
            let _ = lif::decode(&expected[..len]);
        }
        let mut damaged = expected.clone();
        for i in 13..damaged.len() {
            damaged[i] ^= 0xA5;
            let _ = lif::decode(&damaged);
            damaged[i] ^= 0xA5;
        }
    }
    assert_eq!(
        transposed_seen, 1,
        "un vecteur doit déclencher la transposition"
    );
}

/// Compare `mp3map::build(data)` aux colonnes de résultat de Python.
fn check_mp3map(label: &str, data: &[u8], expected: &[String]) {
    let got = match mp3map::build(data) {
        Ok(map) => {
            let rates: Vec<String> = map.samplerates.iter().map(u32::to_string).collect();
            vec![
                "ok".to_owned(),
                digest(&map.data),
                map.num_records.to_string(),
                format!("{:016x}", map.duration_s.to_bits()),
                map.frames.to_string(),
                rates.join(","),
            ]
        }
        Err(e) => vec!["err".to_owned(), e.to_string()],
    };
    assert_eq!(got, expected, "{label}");
}

#[test]
fn mp3map_identique_a_python() {
    let d = dir("mp3map");
    let manifest = rows(&d.join("manifest.tsv"));
    assert!(manifest.len() >= 20);
    let mut overflow_seen = false;
    for row in manifest {
        let (name, input, expected) = (&row[0], &row[1], &row[2..]);
        let data = match input.split(':').collect::<Vec<_>>().as_slice() {
            ["file"] => read(&d.join(format!("{name}.mp3"))),
            ["repeat", frame, count] => unhex(frame).repeat(count.parse().unwrap()),
            _ => panic!("entrée inconnue : {input}"),
        };
        overflow_seen |= expected[0] == "err" && expected[1] == "int too big to convert";
        check_mp3map(name, &data, expected);
    }
    assert!(
        overflow_seen,
        "le cas de dépassement 32 bits doit être couvert"
    );
}

#[test]
fn mp3map_troncatures_identiques_a_python() {
    let d = dir("mp3map");
    let base = read(&d.join("troncature.mp3"));
    let ranges = rows(&d.join("truncations.tsv"));
    let mut next = 0usize;
    for row in ranges {
        let (lo, hi): (usize, usize) = (row[0].parse().unwrap(), row[1].parse().unwrap());
        assert_eq!(lo, next, "plages contiguës");
        for len in lo..=hi {
            check_mp3map(&format!("troncature à {len}"), &base[..len], &row[2..]);
        }
        next = hi + 1;
    }
    assert_eq!(next, base.len() + 1, "toutes les longueurs sont couvertes");
}

#[test]
fn mp3map_prefixes_des_vecteurs_sans_panique() {
    let d = dir("mp3map");
    for row in rows(&d.join("manifest.tsv")) {
        if row[1] != "file" {
            continue;
        }
        let data = read(&d.join(format!("{}.mp3", row[0])));
        for len in (0..=data.len()).step_by(7) {
            let _ = mp3map::build(&data[..len]);
            let _ = mp3map::build(&data[data.len() - len..]);
        }
    }
}
