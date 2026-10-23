//! MP3 écrits en flux (#355) : le paquet de [`build_package_streamed`] ne garde pas les MP3
//! recopiés tels quels, [`write_package_from`] les relit dans l'archive d'entrée.
//!
//! - parité : sur tous les vecteurs de `package_cases.zip`, le `.plain.pk` écrit en flux a le
//!   SHA-256 de Python (comme `package_parity.rs` pour le chemin en mémoire) ;
//! - mémoire : sur une grosse histoire synthétique, le pic d'allocation du chemin en flux est de
//!   l'ordre d'un MP3, celui du chemin en mémoire de la somme des MP3 (mesures affichées).

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde_json::Value;
use sha2::{Digest, Sha256};
use telmi2flam::archive::Archive;
use telmi2flam::image::ThumbFit;
use telmi2flam::output::{write_package, write_package_from, write_zip, Target, ZipLimits};
use telmi2flam::package::{build_package_streamed, build_package_with, Data, Options, Selector};

/// Allocateur qui mesure le pic d'octets alloués.
struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
/// Les tests de ce fichier s'exécutent l'un après l'autre (pic global au processus).
static SERIAL: Mutex<()> = Mutex::new(());

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            let now = CURRENT.fetch_add(layout.size(), Ordering::SeqCst) + layout.size();
            PEAK.fetch_max(now, Ordering::SeqCst);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size(), Ordering::SeqCst);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

const VECTORS: &[u8] = include_bytes!("data/package_cases.zip");

fn never() -> bool {
    false
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Dossier de test propre au processus, supprimé à la fin.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scratch(name: &str) -> Scratch {
    let d = std::env::temp_dir().join(format!("t2f-mp3-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    Scratch(d)
}

fn options(case: &Value) -> Options {
    Options {
        keep_size: case["keep_size"].as_bool().unwrap(),
        allow_missing: case["allow_missing"].as_bool().unwrap(),
        selector: match case["selector"].as_str().unwrap() {
            "carousel" => Selector::Carousel,
            _ => Selector::Image,
        },
        thumb_fit: match case.get("thumb_fit").and_then(Value::as_str) {
            Some("flou") => ThumbFit::Flou,
            Some("rogner") => ThumbFit::Rogner,
            _ => ThumbFit::Bandes,
        },
    }
}

/// Conversion en flux de `data` vers `out` ; `Err` : code d'erreur.
fn convert_streamed(data: &[u8], opts: &Options, out: &Path) -> Result<usize, String> {
    let mut archive =
        Archive::new(Cursor::new(data), "in.zip".into()).map_err(|e| e.code.to_owned())?;
    let pkg = build_package_streamed(&mut archive, opts, &never, &mut ())
        .map_err(|e| e.code.to_owned())?;
    let refs = pkg
        .entries
        .iter()
        .filter(|(_, d)| matches!(d, Data::Source { .. }))
        .count();
    let target = Target::new(Some(out), false, &pkg);
    write_package_from(&pkg, archive, &target, &never, &mut ()).map_err(|e| e.code.to_owned())?;
    Ok(refs)
}

/// Le chemin en flux écrit le `.plain.pk` de Python, à l'octet près, sur tous les vecteurs
/// (sans ffmpeg, comme les vecteurs : un MP3 hors 44 100 Hz est référencé aussi).
#[test]
fn flux_identique_a_python() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("FLAM_FFMPEG", "");
    let mut zip = Archive::new(Cursor::new(VECTORS), "package_cases.zip".into()).unwrap();
    let raw = zip.read("expected.json").unwrap().unwrap();
    let v: Value = serde_json::from_slice(&raw).unwrap();
    let cases = v["cases"].as_array().unwrap().clone();
    let d = scratch("parite");
    let (mut checked, mut refs) = (0, 0);
    let mut failures = Vec::new();
    for case in &cases {
        let Some(want) = case.get("pk_sha256").and_then(Value::as_str) else {
            continue;
        };
        let id = case["id"].as_str().unwrap().to_owned();
        let data = zip.read(&format!("cases/{id}.zip")).unwrap().unwrap();
        let opts = options(case);
        let out = d.0.join(format!("{id}.plain.pk"));
        let out2 = out.clone();
        // Pile de 8 Mo, comme le binaire.
        let r = std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || convert_streamed(&data, &opts, &out2))
            .unwrap()
            .join()
            .unwrap();
        match r {
            Ok(n) => {
                refs += n;
                let have = hex(&Sha256::digest(std::fs::read(&out).unwrap()));
                if have != want {
                    failures.push(format!("{id} : SHA-256 Python {want}, Rust {have}"));
                }
                checked += 1;
            }
            // Écarts assumés (délégation à Python, limites du port) : même erreur qu'en
            // mémoire (`package_parity.rs` les vérifie).
            Err(code) => {
                let data = zip.read(&format!("cases/{id}.zip")).unwrap().unwrap();
                let opts = options(case);
                let mem = std::thread::Builder::new()
                    .stack_size(8 << 20)
                    .spawn(move || {
                        let mut a = Archive::new(Cursor::new(&data[..]), "in.zip".into())?;
                        build_package_with(&mut a, &opts, &never, &mut ()).map(|_| ())
                    })
                    .unwrap()
                    .join()
                    .unwrap();
                if mem.map_err(|e| e.code) != Err(code.as_str()) {
                    failures.push(format!("{id} : erreur {code} en flux seulement"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked > 80, "{checked} cas");
    assert!(refs > 50, "{refs} MP3 référencés seulement");
}

/// MP3 de `frames` trames silencieuses (MPEG1 Layer III, 128 kbit/s, 44 100 Hz), avec un octet
/// distinct par fichier pour des contenus différents.
fn mp3(frames: usize, tag: u8) -> Vec<u8> {
    let mut frame = vec![0u8; 417];
    frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x00]);
    frame[100] = tag;
    frame.repeat(frames)
}

/// Histoire de `n` scènes, chacune avec son MP3 d'environ `frames` × 417 octets.
fn big_story(n: usize, frames: usize) -> Vec<u8> {
    let stages: Vec<String> = (0..n)
        .map(|i| {
            format!(
                "\"s{i}\": {{\"audio\": \"s{i}.mp3\", \"ok\": null, \"home\": null, \
                 \"control\": {{\"ok\": true, \"home\": false, \"autoplay\": true}}}}"
            )
        })
        .collect();
    let actions: Vec<String> = (0..n).map(|i| format!("{{\"stage\": \"s{i}\"}}")).collect();
    let nodes = format!(
        "{{\"startAction\": {{\"action\": \"a0\", \"index\": 0}}, \"stages\": {{{}}}, \
         \"actions\": {{\"a0\": [{}]}}}}",
        stages.join(", "),
        actions.join(", ")
    );
    let mut entries = vec![
        (
            "metadata.json".to_owned(),
            b"{\"title\": \"Grosse\", \"uuid\": \"u-355\"}".to_vec(),
        ),
        ("nodes.json".to_owned(), nodes.into_bytes()),
    ];
    for i in 0..n {
        entries.push((format!("audios/s{i}.mp3"), mp3(frames, i as u8)));
    }
    let mut out = Vec::new();
    write_zip(&mut out, &entries, ZipLimits::PYTHON, &never, &mut |_| {
        Ok(())
    })
    .unwrap();
    out
}

/// Pic d'allocation de `f`, au-dessus de l'état courant.
fn peak_of<T>(f: impl FnOnce() -> T) -> (T, usize) {
    let base = CURRENT.load(Ordering::SeqCst);
    PEAK.store(base, Ordering::SeqCst);
    let r = f();
    (r, PEAK.load(Ordering::SeqCst).saturating_sub(base))
}

/// 12 MP3 de 8 Mo : pic du chemin en mémoire > somme des MP3, pic du chemin en flux borné par
/// quelques MP3 ; pendant l'écriture seule, moins d'un MP3 (copie par tours, #402) ; les deux
/// `.plain.pk` sont identiques.
#[test]
fn pic_memoire_d_un_mp3_et_non_de_la_somme() {
    let _serial = SERIAL.lock().unwrap_or_else(|p| p.into_inner());
    std::env::set_var("FLAM_FFMPEG", "");
    // MP3 de 8 Mo : l'écriture en flux (tampon 1 Mio + tour 256 Kio + tampons de lecture, ~1,6 Mio) reste très loin
    // sous un MP3 entier (#416, la marge était mince avec 2 Mo).
    const N: usize = 12;
    const FRAMES: usize = 20_000; // 8 340 000 octets
    let mp3_size = 417 * FRAMES;
    let d = scratch("memoire");
    let input = d.0.join("grosse.zip");
    std::fs::write(&input, big_story(N, FRAMES)).unwrap();
    let opts = Options::default();
    let run = |streamed: bool, out: std::path::PathBuf| {
        let input = input.clone();
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn(move || {
                peak_of(|| {
                    let mut a = Archive::open(&input).unwrap();
                    if streamed {
                        let pkg = build_package_streamed(&mut a, &opts, &never, &mut ()).unwrap();
                        let t = Target::new(Some(&out), false, &pkg);
                        // Pic de l'écriture seule, puis pic global rétabli.
                        let build_peak = PEAK.load(Ordering::SeqCst);
                        let (_, write_peak) = peak_of(|| {
                            write_package_from(&pkg, a, &t, &never, &mut ()).unwrap();
                        });
                        PEAK.fetch_max(build_peak, Ordering::SeqCst);
                        write_peak
                    } else {
                        let pkg = build_package_with(&mut a, &opts, &never, &mut ()).unwrap();
                        let t = Target::new(Some(&out), false, &pkg);
                        write_package(&pkg, &t, &never).unwrap();
                        0
                    }
                })
            })
            .unwrap()
            .join()
            .unwrap()
    };
    let (mem_out, stream_out) = (d.0.join("mem.plain.pk"), d.0.join("flux.plain.pk"));
    let (_, before) = run(false, mem_out.clone());
    let (write, after) = run(true, stream_out.clone());
    eprintln!(
        "pic mémoire, {N} MP3 de {mp3_size} octets : en mémoire {before} octets, \
         en flux {after} octets, dont écriture {write} octets"
    );
    assert_eq!(
        std::fs::read(&mem_out).unwrap(),
        std::fs::read(&stream_out).unwrap()
    );
    assert!(before > N * mp3_size, "en mémoire : {before}");
    assert!(after < 2 * mp3_size + (8 << 20), "en flux : {after}");
    // Tampon d'écriture (1 Mio) et un tour de relecture (256 Kio), jamais un MP3 entier.
    assert!(write < mp3_size / 3, "écriture en flux : {write}");
}
