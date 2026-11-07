//! Tests de la conversion STUdio → `.lunii.pk` : zips STUdio synthétiques (PNG et JPEG
//! produits en mémoire, MP3 synthétiques des fixtures de `crates/studio2lunii`), transcodeur
//! simulé, puis relecture du pack par `device::prepare_lunii` avec des clés de test.

use std::io::{Cursor, Write};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use studio2lunii::audio::{mp3_tag_cleanup, transcoding_required};
use zip::write::SimpleFileOptions;
use zip::ZipArchive;

use super::*;
use crate::device::crypto::tests::test_keys;
use crate::device::prepare::Limits as PrepareLimits;
use crate::device::prepare_lunii;

// --- Fixtures ----------------------------------------------------------------------------------

const UUID_A: &str = "0a1b2c3d-4e5f-4071-8293-a4b5c6d7e8f9";
const UUID_B: &str = "1b2c3d4e-5f60-4182-93a4-b5c6d7e8f90a";
const UUID_C: &str = "2c3d4e5f-6071-4293-a4b5-c6d7e8f90a1b";

const IMG_A: &str = "0123456789abcdef0123456789abcdef0000000a.png";
const IMG_B: &str = "0123456789abcdef0123456789abcdef0000000b.jpg";
const SND_A: &str = "0123456789abcdef0123456789abcdef000000a1.mp3";
const SND_B: &str = "0123456789abcdef0123456789abcdef000000b1.mp3";
const SND_C: &str = "0123456789abcdef0123456789abcdef000000c1.mp3";

/// MP3 synthétique des tests de `studio2lunii` (`tests/data/audio/<nom>.mp3`).
fn fixture(name: &str) -> Vec<u8> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../crates/studio2lunii/tests/data/audio")
        .join(format!("{name}.mp3"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// MP3 conforme (gardé tel quel).
fn mp3_mono() -> Vec<u8> {
    fixture("mono_44100_cbr")
}

/// MP3 conforme avec étiquette ID3v2.3 (gardé, étiquette retirée).
fn mp3_tagged() -> Vec<u8> {
    fixture("tag_id3v23")
}

/// MP3 stéréo (à transcoder).
fn mp3_stereo() -> Vec<u8> {
    fixture("stereo_44100")
}

fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, y| {
        let shade = ((x + y) % 7) as u8 * 9;
        image::Rgba([
            rgba[0].wrapping_add(shade),
            rgba[1],
            rgba[2].wrapping_sub(shade),
            rgba[3],
        ])
    });
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .unwrap();
    out
}

fn jpeg(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 3) as u8, (y * 5) as u8, 90]));
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Jpeg)
        .unwrap();
    out
}

fn controls() -> Value {
    json!({"wheel": true, "ok": true, "home": true, "pause": false, "autoplay": false})
}

/// Graphe inventé : A (entrée, image PNG, son conforme) → menu [B, C] ; B (image JPEG, son
/// étiqueté) → retour [A] ; C (sans image, son stéréo) → maison vers retour. Cycle permis.
fn story(title: &str, night: bool) -> Value {
    json!({
        "format": "v1",
        "version": 3,
        "title": title,
        "description": "Une\taventure\u{0007} inventée\n pour les tests.",
        "nightModeAvailable": night,
        "stageNodes": [
            {"uuid": UUID_B, "image": IMG_B, "audio": SND_B, "squareOne": false,
             "okTransition": {"actionNode": "retour", "optionIndex": 0},
             "homeTransition": null, "controlSettings": controls(), "name": "B"},
            {"uuid": UUID_A, "image": IMG_A, "audio": SND_A, "squareOne": true,
             "okTransition": {"actionNode": "menu", "optionIndex": 1},
             "homeTransition": null, "controlSettings": controls(), "name": "A"},
            {"uuid": UUID_C, "image": null, "audio": SND_C,
             "okTransition": null,
             "homeTransition": {"actionNode": "retour", "optionIndex": 0},
             "controlSettings": controls()}
        ],
        "actionNodes": [
            {"id": "menu", "options": [UUID_B, UUID_C], "name": "Menu"},
            {"id": "retour", "options": [UUID_A]}
        ]
    })
}

type Entries = Vec<(String, Vec<u8>)>;

fn entry(name: &str, data: Vec<u8>) -> (String, Vec<u8>) {
    (name.to_string(), data)
}

fn story_entries(story: &Value) -> Entries {
    vec![
        entry("story.json", serde_json::to_vec_pretty(story).unwrap()),
        entry("thumbnail.png", png(300, 300, [200, 40, 40, 255])),
        entry(
            &format!("assets/{IMG_A}"),
            png(640, 480, [10, 120, 200, 255]),
        ),
        entry(&format!("assets/{IMG_B}"), jpeg(320, 240)),
        entry(&format!("assets/{SND_A}"), mp3_mono()),
        entry(&format!("assets/{SND_B}"), mp3_tagged()),
        entry(&format!("assets/{SND_C}"), mp3_stereo()),
        // Asset présent mais non référencé : ignoré.
        entry("assets/inutile.png", png(4, 4, [0, 0, 0, 255])),
    ]
}

fn base_entries() -> Entries {
    story_entries(&story("Léa et l’Œuf d’été — épisode 1…", true))
}

fn set(entries: &mut Entries, name: &str, data: Vec<u8>) {
    entries.iter_mut().find(|(n, _)| n == name).unwrap().1 = data;
}

fn without(entries: &Entries, name: &str) -> Entries {
    entries.iter().filter(|(n, _)| n != name).cloned().collect()
}

fn write_zip(path: &Path, entries: &Entries) {
    let mut zip = ZipWriter::new(File::create(path).unwrap());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, data) in entries {
        if name.ends_with('/') {
            zip.add_directory(name.as_str(), options).unwrap();
        } else {
            zip.start_file(name.as_str(), options).unwrap();
            zip.write_all(data).unwrap();
        }
    }
    zip.finish().unwrap();
}

/// Dossiers d'un test : `src/histoire.zip` et `out/` (dossier de sortie, vide).
struct Dirs {
    _tmp: tempfile::TempDir,
    input: PathBuf,
    out_dir: PathBuf,
    output: PathBuf,
}

fn dirs(entries: &Entries) -> Dirs {
    let tmp = tempfile::tempdir().unwrap();
    let src = tmp.path().join("src");
    let out_dir = tmp.path().join("out");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&out_dir).unwrap();
    let input = src.join("histoire.zip");
    write_zip(&input, entries);
    let output = out_dir.join(format!("{UUID_A}-3.lunii.pk"));
    Dirs {
        _tmp: tmp,
        input,
        out_dir,
        output,
    }
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Rien ne reste dans le dossier de sortie (ni sortie, ni dossier temporaire).
fn assert_nothing_left(d: &Dirs) {
    assert_eq!(names_in(&d.out_dir), Vec::<String>::new());
}

/// Transcodeur simulé.
enum Fake {
    /// Écrit ces octets comme sortie.
    Output(Vec<u8>),
    /// Échoue comme ffmpeg.
    Fail,
    /// Demande l'annulation pendant le transcodage.
    Cancel(Arc<watch::Sender<bool>>),
    /// Ne rend jamais la main (ffmpeg bloqué).
    Slow,
    /// Contrôle anticipé en échec (ffmpeg absent ou empreinte fausse).
    Unavailable,
}

struct FakeTranscoder {
    mode: Fake,
    calls: AtomicUsize,
    prepares: AtomicUsize,
    inputs: Mutex<Vec<Vec<u8>>>,
}

impl FakeTranscoder {
    fn new(mode: Fake) -> Self {
        Self {
            mode,
            calls: AtomicUsize::new(0),
            prepares: AtomicUsize::new(0),
            inputs: Mutex::default(),
        }
    }
}

impl Transcoder for FakeTranscoder {
    fn prepare<'a>(&'a self, _cancel: &'a watch::Receiver<bool>) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            self.prepares.fetch_add(1, Ordering::SeqCst);
            match self.mode {
                Fake::Unavailable => Err(ffmpeg_unavailable("absent (simulé)")),
                _ => Ok(()),
            }
        })
    }

    fn transcode<'a>(
        &'a self,
        input: &'a Path,
        output: &'a Path,
        _cancel: &'a watch::Receiver<bool>,
    ) -> BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.inputs.lock().unwrap().push(std::fs::read(input)?);
            match &self.mode {
                Fake::Output(bytes) => {
                    std::fs::write(output, bytes)?;
                    Ok(())
                }
                Fake::Fail => Err(conversion(FFMPEG_FAILED, "échec simulé")),
                Fake::Cancel(tx) => {
                    tx.send_replace(true);
                    Err(AppError::Cancelled)
                }
                Fake::Slow => {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    Ok(())
                }
                Fake::Unavailable => unreachable!("contrôle anticipé en échec"),
            }
        })
    }
}

fn good_transcoder() -> FakeTranscoder {
    FakeTranscoder::new(Fake::Output(mp3_mono()))
}

struct Run {
    result: AppResult<StudioPack>,
    steps: Vec<(String, u64, u64, f64)>,
}

async fn convert_dirs(d: &Dirs, transcoder: Option<&dyn Transcoder>) -> Run {
    let (_tx, rx) = watch::channel(false);
    convert_rx(d, transcoder, &rx).await
}

async fn convert_rx(
    d: &Dirs,
    transcoder: Option<&dyn Transcoder>,
    rx: &watch::Receiver<bool>,
) -> Run {
    convert_budget(d, transcoder, FFMPEG_TOTAL_BUDGET, rx).await
}

async fn convert_budget(
    d: &Dirs,
    transcoder: Option<&dyn Transcoder>,
    budget: Duration,
    rx: &watch::Receiver<bool>,
) -> Run {
    let mut steps = Vec::new();
    let mut on_step = |s: &Step| steps.push((s.phase.clone(), s.done, s.total, s.percent()));
    let result = convert_with(
        &d.input,
        &d.output,
        ThumbFit::default(),
        transcoder,
        budget,
        rx,
        &mut on_step,
    )
    .await;
    Run { result, steps }
}

fn read_pack(path: &Path) -> Entries {
    let mut zip = ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut e = zip.by_index(i).unwrap();
            assert_eq!(e.compression(), CompressionMethod::Stored, "{}", e.name());
            let mut data = Vec::new();
            e.read_to_end(&mut data).unwrap();
            (e.name().to_string(), data)
        })
        .collect()
}

fn code_of(err: &AppError) -> &str {
    match err {
        AppError::Conversion { code, .. } => code,
        other => panic!("erreur de conversion attendue : {other:?}"),
    }
}

fn expect_code(run: Run, code: &str) -> String {
    let err = run.result.unwrap_err();
    assert_eq!(code_of(&err), code, "{err}");
    err.to_string()
}

// --- Pack complet et relecture par prepare_lunii ------------------------------------------------

#[tokio::test]
async fn pack_conforme_au_contrat_et_relu_par_prepare_lunii() {
    let entries = base_entries();
    let d = dirs(&entries);
    let fake = good_transcoder();
    let run = convert_dirs(&d, Some(&fake)).await;
    let pack = run.result.unwrap();
    assert_eq!(names_in(&d.out_dir), vec![format!("{UUID_A}-3.lunii.pk")]);

    // Résumé.
    assert_eq!(pack.uuid, UUID_A);
    assert_eq!(pack.version, 3);
    assert_eq!(pack.title, "Léa et l’Œuf d’été — épisode 1…");
    assert_eq!(pack.description, "Une aventure inventée pour les tests.");
    assert!(pack.night_mode && pack.thumbnail);
    assert_eq!((pack.images, pack.sounds, pack.transcoded), (2, 3, 1));
    assert!(pack.warnings.is_empty(), "{:?}", pack.warnings);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_eq!(fake.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(fake.inputs.lock().unwrap()[0], mp3_stereo());
    assert_eq!(pack.bytes, std::fs::metadata(&d.output).unwrap().len());

    // Contenu, dans l'ordre du contrat.
    let story = parse_story(&serde_json::to_vec(&story("x", true)).unwrap()).unwrap();
    let index = build_index(&story);
    let files = read_pack(&d.output);
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    let mut expected = vec![
        "uuid.bin",
        "_metadata.json",
        "_thumbnail.png",
        "ni",
        "nm",
        "li",
        "ri",
        "si",
        "info",
        "img/thumbnail.lif",
    ];
    let targets: Vec<String> = index.assets.iter().map(|a| a.pack_path.clone()).collect();
    expected.extend(targets.iter().map(String::as_str));
    assert_eq!(names, expected);
    let get = |name: &str| files.iter().find(|(n, _)| n == name).unwrap().1.clone();
    let target = |asset: &str| {
        index
            .assets
            .iter()
            .find(|a| a.name == asset)
            .unwrap()
            .pack_path
            .clone()
    };

    assert_eq!(
        get("uuid.bin"),
        uuid::Uuid::parse_str(UUID_A).unwrap().as_bytes()
    );
    let meta: Value = serde_json::from_slice(&get("_metadata.json")).unwrap();
    assert_eq!(meta["source_format"], "studio");
    assert_eq!(meta["uuid"], UUID_A);
    assert_eq!(meta["version"], 3);
    assert_eq!(meta["title"], pack.title);
    assert_eq!(meta["description"], pack.description);
    assert_eq!(get("_thumbnail.png"), get_entry(&entries, "thumbnail.png"));
    assert_eq!(get("ni"), index.ni);
    assert_eq!(get("li"), index.li);
    assert_eq!(get("ri"), index.ri);
    assert_eq!(get("si"), index.si);
    assert!(get("nm").is_empty());
    assert_eq!(
        get("info"),
        b"Lea et l'OEuf d'ete - episode 1...\nLea et l'OEuf d'ete - episode 1...\n0\n\
          Lea et l'OEuf d'ete - episode 1...\nNo authors."
    );
    let lif = telmi2flam::lif::decode(&get("img/thumbnail.lif")).unwrap();
    assert_eq!((lif.width, lif.height), (104, 136));

    let img_a = get_entry(&entries, &format!("assets/{IMG_A}"));
    let img_b = get_entry(&entries, &format!("assets/{IMG_B}"));
    assert_eq!(get(&target(IMG_A)), to_lunii_bmp(&img_a).unwrap());
    assert_eq!(get(&target(IMG_B)), to_lunii_bmp(&img_b).unwrap());
    assert_eq!(get(&target(SND_A)), mp3_mono());
    assert_eq!(get(&target(SND_B)), mp3_tag_cleanup(&mp3_tagged()).unwrap());
    assert_ne!(get(&target(SND_B)), mp3_tagged());
    assert_eq!(get(&target(SND_C)), mp3_mono());
    for snd in [SND_A, SND_B, SND_C] {
        assert!(!transcoding_required("x.mp3", &get(&target(snd))));
    }

    // Relecture par la préparation de l'envoi : acceptée, déchiffrement identique.
    let work = d._tmp.path().join("travail");
    let keys = test_keys();
    let prepared = prepare_lunii::prepare(
        &d.output,
        &keys,
        &work,
        &PrepareLimits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
    assert_eq!(prepared.uuid, UUID_A);
    for (name, data) in &files {
        if matches!(
            name.as_str(),
            "uuid.bin" | "_metadata.json" | "_thumbnail.png"
        ) {
            continue;
        }
        let on_device = std::fs::read(prepared.dir.join(name)).unwrap();
        if matches!(name.as_str(), "ni" | "nm" | "info" | "img/thumbnail.lif") {
            assert_eq!(&on_device, data, "{name} en clair");
        } else {
            let clear = keys.decrypt_lunii_file(name, &on_device).unwrap();
            assert_eq!(&clear[..data.len()], &data[..], "{name}");
            assert!(clear[data.len()..].iter().all(|&b| b == 0), "{name}");
        }
    }

    // Progression : trois phases, jusqu'à 100 %.
    let phases: Vec<&str> = run.steps.iter().map(|s| s.0.as_str()).collect();
    assert_eq!(phases.first(), Some(&"images"));
    assert!(phases.contains(&"audio"));
    let last = run.steps.last().unwrap();
    assert_eq!((last.0.as_str(), last.3), ("zip", 100.0));
    assert!(run.steps.windows(2).all(|w| w[0].3 <= w[1].3));
}

fn get_entry(entries: &Entries, name: &str) -> Vec<u8> {
    entries.iter().find(|(n, _)| n == name).unwrap().1.clone()
}

#[tokio::test]
async fn conversion_deterministe_et_sortie_remplacee() {
    let entries = base_entries();
    let d = dirs(&entries);
    std::fs::write(&d.output, b"ancienne version").unwrap();
    let fake = good_transcoder();
    convert_dirs(&d, Some(&fake)).await.result.unwrap();
    let first = std::fs::read(&d.output).unwrap();
    std::fs::remove_file(&d.output).unwrap();
    convert_dirs(&d, Some(&fake)).await.result.unwrap();
    assert_eq!(std::fs::read(&d.output).unwrap(), first);
    assert_eq!(names_in(&d.out_dir).len(), 1);
}

#[tokio::test]
async fn histoire_sans_son_a_transcoder_ni_vignette_sans_ffmpeg() {
    let mut s = story("Titre", false);
    s["stageNodes"][2]["audio"] = json!(SND_A);
    s["stageNodes"][0]["image"] = Value::Null;
    s["stageNodes"][1]["image"] = Value::Null;
    let entries = without(
        &without(&story_entries(&s), "thumbnail.png"),
        &format!("assets/{SND_C}"),
    );
    let d = dirs(&entries);
    let pack = convert_dirs(&d, None).await.result.unwrap();
    assert_eq!((pack.images, pack.sounds, pack.transcoded), (0, 2, 0));
    assert!(!pack.thumbnail && !pack.night_mode);
    let names: Vec<String> = read_pack(&d.output).into_iter().map(|(n, _)| n).collect();
    assert!(!names.iter().any(|n| n == "nm"
        || n.starts_with("img/")
        || n == "_thumbnail.png"
        || n.starts_with("rf/")));
    // `ri` vide, accepté par la préparation.
    prepare_lunii::prepare(
        &d.output,
        &test_keys(),
        &d._tmp.path().join("travail"),
        &PrepareLimits::default(),
        &AtomicBool::new(false),
    )
    .unwrap();
}

#[tokio::test]
async fn dossier_unique_a_la_racine_et_fichiers_macos_ignores() {
    let mut entries: Entries = base_entries()
        .into_iter()
        .map(|(n, d)| (format!("Mon histoire/{n}"), d))
        .collect();
    entries.push(entry("__MACOSX/Mon histoire/._story.json", b"x".to_vec()));
    entries.push(entry("Mon histoire/assets/._truc.png", b"x".to_vec()));
    entries.push(entry("Mon histoire/assets/", Vec::new()));
    let d = dirs(&entries);
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert_eq!(pack.uuid, UUID_A);
}

// --- Vignette ------------------------------------------------------------------------------------

#[tokio::test]
async fn vignette_de_l_image_de_depart_png_ou_jpeg() {
    // Sans thumbnail.png : image du nœud d'entrée (PNG), pas de `_thumbnail.png`.
    let d = dirs(&without(&base_entries(), "thumbnail.png"));
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert!(pack.thumbnail && pack.warnings.is_empty());
    let files = read_pack(&d.output);
    assert!(!files.iter().any(|(n, _)| n == "_thumbnail.png"));
    let lif = &files
        .iter()
        .find(|(n, _)| n == "img/thumbnail.lif")
        .unwrap()
        .1;
    let lif = telmi2flam::lif::decode(lif).unwrap();
    assert_eq!((lif.width, lif.height), (104, 136));

    // Image de départ JPEG, thumbnail.png illisible : avertissement puis repli.
    let mut s = story("Titre", false);
    s["stageNodes"][1]["image"] = json!(IMG_B);
    let mut entries = story_entries(&s);
    set(&mut entries, "thumbnail.png", b"pas un png".to_vec());
    let d = dirs(&entries);
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert!(pack.thumbnail);
    assert_eq!(pack.warnings.len(), 1, "{:?}", pack.warnings);
    assert!(!read_pack(&d.output)
        .iter()
        .any(|(n, _)| n == "_thumbnail.png"));
}

// --- Refus ---------------------------------------------------------------------------------------

#[tokio::test]
async fn asset_reference_absent_refuse_sans_rien_ecrire() {
    let d = dirs(&without(&base_entries(), &format!("assets/{SND_B}")));
    let fake = good_transcoder();
    let msg = expect_code(convert_dirs(&d, Some(&fake)).await, MISSING_ASSET);
    assert!(msg.contains(SND_B), "{msg}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_nothing_left(&d);
}

#[tokio::test]
async fn archives_hostiles_refusees() {
    let fake = good_transcoder();
    let cases: Vec<(&str, Entries)> = vec![
        ("remontée ..", {
            let mut e = base_entries();
            e.push(entry("../evil.png", b"x".to_vec()));
            e
        }),
        ("chemin absolu", {
            let mut e = base_entries();
            e.push(entry("/etc/passwd", b"x".to_vec()));
            e
        }),
        ("doublon à la casse près", {
            let mut e = base_entries();
            e.push(entry(
                &format!("assets/{}", IMG_A.to_uppercase()),
                b"x".to_vec(),
            ));
            e
        }),
        ("nom réservé Windows", {
            let mut e = base_entries();
            e.push(entry("assets/CON.png", b"x".to_vec()));
            e
        }),
        ("story.json hors racine", {
            base_entries()
                .into_iter()
                .map(|(n, d)| {
                    let dir = if n == "story.json" { "a" } else { "b" };
                    (format!("{dir}/{n}"), d)
                })
                .collect()
        }),
        ("story.json absent", without(&base_entries(), "story.json")),
        ("archive ambiguë", {
            let mut e = base_entries();
            e.push(entry("uuid.bin", vec![0; 16]));
            e
        }),
    ];
    for (what, entries) in cases {
        let d = dirs(&entries);
        let run = convert_dirs(&d, Some(&fake)).await;
        let err = run.result.unwrap_err();
        assert_eq!(code_of(&err), INVALID_ARCHIVE, "{what} : {err}");
        assert_nothing_left(&d);
    }
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);

    // Ce n'est pas un zip.
    let tmp = tempfile::tempdir().unwrap();
    let d = Dirs {
        input: tmp.path().join("faux.zip"),
        out_dir: tmp.path().join("out"),
        output: tmp.path().join("out").join("x.lunii.pk"),
        _tmp: tmp,
    };
    std::fs::write(&d.input, b"pas un zip").unwrap();
    std::fs::create_dir(&d.out_dir).unwrap();
    expect_code(convert_dirs(&d, None).await, INVALID_ARCHIVE);
    assert_nothing_left(&d);
}

#[tokio::test]
async fn story_json_invalide_refuse() {
    for mutate in [
        |s: &mut Value| s["format"] = json!("v2"),
        |s: &mut Value| s["stageNodes"][0]["okTransition"]["actionNode"] = json!("inconnu"),
        |s: &mut Value| s["stageNodes"][1]["image"] = json!("../../evil.png"),
        |s: &mut Value| s["stageNodes"][2]["squareOne"] = json!(true),
    ] {
        let mut s = story("Titre", false);
        mutate(&mut s);
        let d = dirs(&story_entries(&s));
        expect_code(
            convert_dirs(&d, Some(&good_transcoder())).await,
            INVALID_STORY,
        );
        assert_nothing_left(&d);
    }
}

#[tokio::test]
async fn bombe_zip_et_image_illisible_refusees() {
    // 33 Mio de zéros (quelques dizaines de Kio compressés) : au-delà de la borne des images.
    let mut entries = base_entries();
    set(
        &mut entries,
        &format!("assets/{IMG_A}"),
        vec![0u8; MAX_IMAGE_BYTES + 1024 * 1024],
    );
    let d = dirs(&entries);
    expect_code(convert_dirs(&d, Some(&good_transcoder())).await, TOO_LARGE);
    assert_nothing_left(&d);

    let mut entries = base_entries();
    set(
        &mut entries,
        &format!("assets/{IMG_B}"),
        b"pas une image".to_vec(),
    );
    let d = dirs(&entries);
    let msg = expect_code(
        convert_dirs(&d, Some(&good_transcoder())).await,
        UNREADABLE_IMAGE,
    );
    assert!(msg.contains(IMG_B), "{msg}");
    assert_nothing_left(&d);
}

// --- ffmpeg --------------------------------------------------------------------------------------

#[tokio::test]
async fn ffmpeg_absent_refuse_seulement_si_un_son_doit_etre_transcode() {
    // Détecté par la pré-analyse (MP3 stéréo), avant la conversion des images.
    let d = dirs(&base_entries());
    let run = convert_dirs(&d, None).await;
    let err = run.result.unwrap_err();
    assert!(matches!(err, AppError::ConverterUnavailable(_)), "{err:?}");
    assert!(err.to_string().contains(SND_C), "{err}");
    assert!(run.steps.is_empty(), "{:?}", run.steps);
    assert_nothing_left(&d);

    // Son au nom autre que `.mp3` : détecté à l'extension.
    let mut s = story("Titre", false);
    s["stageNodes"][0]["audio"] = json!("son.ogg");
    s["stageNodes"][2]["audio"] = json!(SND_A);
    let mut entries = story_entries(&s);
    entries.push(entry("assets/son.ogg", b"OggS factice".to_vec()));
    let d = dirs(&entries);
    let run = convert_dirs(&d, None).await;
    let err = run.result.unwrap_err();
    assert!(err.to_string().contains("son.ogg"), "{err}");
    assert!(run.steps.is_empty(), "{:?}", run.steps);
    assert_nothing_left(&d);

    // ffmpeg présent mais inutilisable : refusé avant toute conversion, jamais lancé.
    let d = dirs(&base_entries());
    let fake = FakeTranscoder::new(Fake::Unavailable);
    let run = convert_dirs(&d, Some(&fake)).await;
    let err = run.result.unwrap_err();
    assert!(matches!(err, AppError::ConverterUnavailable(_)), "{err:?}");
    assert!(run.steps.is_empty(), "{:?}", run.steps);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_nothing_left(&d);
}

#[tokio::test]
async fn son_non_mp3_copie_en_flux_pour_ffmpeg() {
    let mut s = story("Titre", false);
    s["stageNodes"][2]["audio"] = json!("son.wav");
    let mut entries = without(&story_entries(&s), &format!("assets/{SND_C}"));
    let wav: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    entries.push(entry("assets/son.wav", wav.clone()));
    let d = dirs(&entries);
    let fake = good_transcoder();
    let pack = convert_dirs(&d, Some(&fake)).await.result.unwrap();
    assert_eq!((pack.sounds, pack.transcoded), (3, 1));
    assert_eq!(fake.prepares.load(Ordering::SeqCst), 1);
    assert_eq!(*fake.inputs.lock().unwrap(), vec![wav]);
}

#[tokio::test]
async fn histoire_sans_son_a_transcoder_ne_controle_pas_ffmpeg() {
    let mut s = story("Titre", false);
    s["stageNodes"][2]["audio"] = json!(SND_A);
    let entries = without(&story_entries(&s), &format!("assets/{SND_C}"));
    let d = dirs(&entries);
    let fake = FakeTranscoder::new(Fake::Unavailable);
    let pack = convert_dirs(&d, Some(&fake)).await.result.unwrap();
    assert_eq!(pack.transcoded, 0);
    assert_eq!(fake.prepares.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn budget_global_de_ffmpeg_depasse() {
    // Faux transcodeur bloqué : arrêté au bout du budget, rien ne reste.
    let d = dirs(&base_entries());
    let fake = FakeTranscoder::new(Fake::Slow);
    let (_tx, rx) = watch::channel(false);
    let started = Instant::now();
    let run = convert_budget(&d, Some(&fake), Duration::from_millis(200), &rx).await;
    assert!(started.elapsed() < Duration::from_secs(60));
    let err = run.result.unwrap_err();
    match &err {
        AppError::Io(e) => assert_eq!(e.kind(), std::io::ErrorKind::TimedOut, "{e}"),
        other => panic!("délai attendu : {other:?}"),
    }
    assert!(err.to_string().contains("budget"), "{err}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_nothing_left(&d);

    // Budget épuisé : le transcodeur n'est même pas lancé.
    let fake = good_transcoder();
    let run = convert_budget(&d, Some(&fake), Duration::ZERO, &rx).await;
    assert!(matches!(run.result, Err(AppError::Io(_))));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_nothing_left(&d);
}

#[test]
fn ffmpeg_limite_ses_demuxeurs_en_entree() {
    let at = |arg: &str| FFMPEG_INPUT_ARGS.iter().position(|a| *a == arg).unwrap();
    assert_eq!(FFMPEG_INPUT_ARGS.last(), Some(&"-i"));
    assert_eq!(
        FFMPEG_INPUT_ARGS[at("-format_whitelist") + 1],
        FFMPEG_INPUT_FORMATS
    );
    assert_eq!(FFMPEG_INPUT_ARGS[at("-protocol_whitelist") + 1], "file");
    let formats: Vec<&str> = FFMPEG_INPUT_FORMATS.split(',').collect();
    assert_eq!(formats, ["mp3", "ogg", "wav", "flac", "aac", "mov"]);
}

#[tokio::test]
async fn echec_ou_sortie_non_conforme_de_ffmpeg_refuses() {
    let d = dirs(&base_entries());
    let fake = FakeTranscoder::new(Fake::Fail);
    expect_code(convert_dirs(&d, Some(&fake)).await, FFMPEG_FAILED);
    assert_nothing_left(&d);

    for bad in [mp3_stereo(), Vec::new(), b"pas un mp3".to_vec()] {
        let fake = FakeTranscoder::new(Fake::Output(bad));
        let msg = expect_code(convert_dirs(&d, Some(&fake)).await, FFMPEG_FAILED);
        assert!(msg.contains("pas conforme"), "{msg}");
        assert_nothing_left(&d);
    }
}

// --- Annulation et écriture ----------------------------------------------------------------------

#[tokio::test]
async fn annulation_avant_le_debut_et_pendant_le_transcodage() {
    let d = dirs(&base_entries());
    let (tx, rx) = watch::channel(true);
    let fake = good_transcoder();
    let err = convert_rx(&d, Some(&fake), &rx).await.result.unwrap_err();
    assert!(matches!(err, AppError::Cancelled), "{err:?}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_nothing_left(&d);
    drop(tx);

    let (tx, rx) = watch::channel(false);
    let fake = FakeTranscoder::new(Fake::Cancel(Arc::new(tx)));
    let err = convert_rx(&d, Some(&fake), &rx).await.result.unwrap_err();
    assert!(matches!(err, AppError::Cancelled), "{err:?}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 1);
    assert_nothing_left(&d);
}

#[tokio::test]
async fn echec_d_ecriture_nettoie_le_dossier_temporaire() {
    // La sortie est un dossier : le renommage final échoue, rien d'autre ne reste.
    let d = dirs(&base_entries());
    std::fs::create_dir(&d.output).unwrap();
    std::fs::write(d.output.join("garde"), b"x").unwrap();
    let err = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap_err();
    assert!(matches!(err, AppError::Io(_)), "{err:?}");
    assert_eq!(names_in(&d.out_dir), vec![format!("{UUID_A}-3.lunii.pk")]);
    assert_eq!(names_in(&d.output), vec!["garde".to_string()]);

    // Dossier de sortie absent.
    let mut d = dirs(&base_entries());
    d.output = d.out_dir.join("absent").join("x.lunii.pk");
    let err = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap_err();
    assert!(matches!(err, AppError::Io(_)), "{err:?}");
    assert_nothing_left(&d);
}

// --- Textes --------------------------------------------------------------------------------------

#[test]
fn titre_et_description_assainis() {
    assert_eq!(clean_line("  a\u{0}b\t\tc\r\n d \u{a0} ", 100), "a b c d");
    assert_eq!(clean_line("abcdef", 3), "abc");
    assert_eq!(
        ascii_text("Ça « déjà » vu ! ß œ Æ ﬁn ½"),
        "Ca \" deja \" vu ! ss oe AE fin 1?2"
    );
    assert_eq!(ascii_text("日本"), "??");
    assert_eq!(info_bytes("Léa"), b"Lea\nLea\n0\nLea\nNo authors.".to_vec());
    // Rien de lisible : titre de repli.
    assert_eq!(
        info_bytes("日本"),
        b"Sans titre\nSans titre\n0\nSans titre\nNo authors.".to_vec()
    );
    // Toujours ASCII imprimable et LF : le contrôle de `prepare_lunii` l'accepte.
    for title in ["", "\u{7}\u{1b}[31m", "a\nb\rc", "Ünïcödé — “ok”"] {
        let info = info_bytes(&clean_line(title, 200));
        assert!(info
            .iter()
            .all(|&b| b == b'\n' || (0x20..0x7f).contains(&b)));
        assert_eq!(info.iter().filter(|&&b| b == b'\n').count(), 4);
    }
}

#[test]
fn caracteres_de_format_retires() {
    // Retournement bidi (« Trojan Source »), espaces sans chasse, BOM, étiquettes.
    assert_eq!(
        clean_line(
            "\u{202E}Conte\u{202C} \u{2066}du\u{2069}\u{200B} soir\u{FEFF}",
            100
        ),
        "Conte du soir"
    );
    assert_eq!(
        clean_line("a\u{200E}\u{200F}\u{061C}b\u{00AD}c\u{E0041}", 100),
        "abc"
    );
    // Seul : titre vide, donc repli.
    assert_eq!(clean_line("\u{202E}\u{2067}", 100), "");
    assert_eq!(ascii_text("L\u{202E}éa\u{200D}"), "Lea");
    // ZWNJ et ZWJ gardés dans le texte UTF-8 (écriture, émojis composés).
    assert_eq!(clean_line("می\u{200C}خواهم", 100), "می\u{200C}خواهم");
}

#[test]
fn pre_analyse_prudente_sur_une_longue_etiquette() {
    // ID3v2 de 100 000 octets : déborde du préfixe, aucune conclusion.
    let mut head = b"ID3\x03\x00\x00".to_vec();
    let size = 100_000u32;
    head.extend([21, 14, 7, 0].iter().map(|s| ((size >> s) & 0x7f) as u8));
    head.resize(PROBE_BYTES as usize, 0xFF);
    assert!(id3v2_overflows(&head));
    assert!(!id3v2_overflows(&mp3_tagged()));
    assert!(!id3v2_overflows(&mp3_mono()));
}

#[tokio::test]
async fn titre_et_description_sans_caracteres_bidi() {
    let mut s = story("\u{202E}Le loup\u{202C}", false);
    s["description"] = json!("Une \u{2066}histoire\u{2069}\u{200B}.");
    let d = dirs(&story_entries(&s));
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert_eq!(pack.title, "Le loup");
    assert_eq!(pack.description, "Une histoire.");
    let files = read_pack(&d.output);
    let info = &files.iter().find(|(n, _)| n == "info").unwrap().1;
    assert!(info.starts_with(b"Le loup\n"), "{info:?}");

    let s = story("\u{202E}\u{200B}", false);
    let d = dirs(&story_entries(&s));
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert_eq!(pack.title, "Sans titre");
}

#[tokio::test]
async fn titre_absent_ou_illisible() {
    let mut s = story("", false);
    s.as_object_mut().unwrap().remove("description");
    let d = dirs(&story_entries(&s));
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert_eq!(pack.title, "Sans titre");
    assert_eq!(pack.description, "");
}

/// Tests avec le vrai ffmpeg embarqué (`npm run build:ffmpeg`) : ignorés sans build, exigés si
/// `FLAM_REQUIRE_FFMPEG` est défini (la CI les lance avec le filtre `ffmpeg::`).
mod ffmpeg {
    use super::*;
    use crate::ffmpeg::MANIFEST_FILE;

    fn real_ffmpeg() -> Option<Ffmpeg> {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("build")
            .join("ffmpeg");
        if !dir.join(Tool::Ffmpeg.file_name()).is_file() || !dir.join(MANIFEST_FILE).is_file() {
            assert!(
                std::env::var_os("FLAM_REQUIRE_FFMPEG").is_none(),
                "ffmpeg embarqué non construit alors que FLAM_REQUIRE_FFMPEG est défini"
            );
            eprintln!("ffmpeg embarqué non construit : test ignoré");
            return None;
        }
        Some(Ffmpeg::in_dir(dir))
    }

    /// Son de 1 s produit par le ffmpeg embarqué (`args` : format de sortie).
    fn make_sound(ff: &Ffmpeg, out: &Path, args: &[&str]) -> Vec<u8> {
        let status = std::process::Command::new(ff.dir().join(Tool::Ffmpeg.file_name()))
            .args([
                "-hide_banner",
                "-nostdin",
                "-v",
                "error",
                "-y",
                "-f",
                "lavfi",
            ])
            .args(["-i", "sine=frequency=440:duration=1:sample_rate=48000"])
            .args(args)
            .arg(out)
            .status()
            .expect("lancement de ffmpeg");
        assert!(status.success(), "ffmpeg n'a pas produit {}", out.display());
        std::fs::read(out).unwrap()
    }

    #[tokio::test]
    async fn transcodage_reel_par_le_ffmpeg_embarque() {
        let Some(ff) = real_ffmpeg() else {
            return;
        };
        let tmp = tempfile::tempdir().unwrap();
        let stereo = make_sound(
            &ff,
            &tmp.path().join("s.mp3"),
            &["-ac", "2", "-c:a", "libmp3lame"],
        );
        let wav = make_sound(&ff, &tmp.path().join("s.wav"), &["-ac", "1"]);
        let mut s = story("Titre", false);
        s["stageNodes"][1]["audio"] = json!("son.wav");
        let mut entries = story_entries(&s);
        set(&mut entries, &format!("assets/{SND_C}"), stereo);
        entries.push(entry("assets/son.wav", wav));
        let d = dirs(&entries);
        let (_tx, rx) = watch::channel(false);
        let pack = convert(
            &StudioJob {
                input: &d.input,
                output: &d.output,
                ffmpeg: Some(&ff),
                thumb_fit: ThumbFit::default(),
            },
            &rx,
            &mut |_: &Step| {},
        )
        .await
        .unwrap();
        assert_eq!(pack.transcoded, 2);
        let story = parse_story(&serde_json::to_vec(&s).unwrap()).unwrap();
        let index = build_index(&story);
        let files = read_pack(&d.output);
        for asset in index.assets.iter().filter(|a| a.kind == AssetKind::Audio) {
            let data = &files.iter().find(|(n, _)| *n == asset.pack_path).unwrap().1;
            assert!(!transcoding_required("x.mp3", data), "{}", asset.name);
            assert!(!data.starts_with(b"ID3"), "{}", asset.name);
        }
        prepare_lunii::prepare(
            &d.output,
            &test_keys(),
            &d._tmp.path().join("travail"),
            &PrepareLimits::default(),
            &AtomicBool::new(false),
        )
        .unwrap();
    }

    #[tokio::test]
    async fn ffmpeg_introuvable_ou_empreinte_fausse() {
        let d = dirs(&base_entries());
        let absent = Ffmpeg::in_dir(d._tmp.path().join("absent"));
        let (_tx, rx) = watch::channel(false);
        let job = StudioJob {
            input: &d.input,
            output: &d.output,
            ffmpeg: Some(&absent),
            thumb_fit: ThumbFit::default(),
        };
        let err = convert(&job, &rx, &mut |_: &Step| {}).await.unwrap_err();
        assert!(matches!(err, AppError::ConverterUnavailable(_)), "{err:?}");
        assert!(err.to_string().contains("ffmpeg"), "{err}");
        assert_nothing_left(&d);

        // Binaire présent mais empreinte fausse : jamais lancé.
        let dir = d._tmp.path().join("faux");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join(Tool::Ffmpeg.file_name()), b"faux").unwrap();
        std::fs::write(
            dir.join(MANIFEST_FILE),
            format!("{}  {}\n", "0".repeat(64), Tool::Ffmpeg.file_name()),
        )
        .unwrap();
        let fake = Ffmpeg::in_dir(&dir);
        let job = StudioJob {
            ffmpeg: Some(&fake),
            ..job
        };
        let err = convert(&job, &rx, &mut |_: &Step| {}).await.unwrap_err();
        assert!(err.to_string().contains("empreinte"), "{err}");
        assert_nothing_left(&d);
    }
}

// --- Suivis de revue -----------------------------------------------------------------------------

/// CRC-32 (PNG) écrit à la main pour les PNG forgés.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// PNG forgé : en-tête d'une image `w` × `h` sans données (bombe de surface).
fn png_header_only(w: u32, h: u32) -> Vec<u8> {
    let mut out = PNG_SIGNATURE.to_vec();
    let mut chunk = |kind: &[u8], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let body = [kind, data].concat();
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IEND", &[]);
    out
}

#[tokio::test]
async fn vignette_bombe_de_surface_ou_deguisee_ignoree() {
    // 13000 × 13000 annoncés : refusée avant décodage, repli sur l'image de départ.
    let mut entries = base_entries();
    set(
        &mut entries,
        "thumbnail.png",
        png_header_only(13_000, 13_000),
    );
    let d = dirs(&entries);
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert!(pack.thumbnail);
    assert_eq!(pack.warnings.len(), 1, "{:?}", pack.warnings);
    assert!(
        pack.warnings[0].contains("thumbnail.png"),
        "{:?}",
        pack.warnings
    );
    assert!(!read_pack(&d.output)
        .iter()
        .any(|(n, _)| n == "_thumbnail.png"));

    // JPEG nommé thumbnail.png : vignette produite, mais pas de `_thumbnail.png` (pas un PNG).
    let mut entries = base_entries();
    set(&mut entries, "thumbnail.png", jpeg(200, 260));
    let d = dirs(&entries);
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert!(
        pack.thumbnail && pack.warnings.is_empty(),
        "{:?}",
        pack.warnings
    );
    assert!(!read_pack(&d.output)
        .iter()
        .any(|(n, _)| n == "_thumbnail.png"));

    // Vignette trop lourde : avertissement, pas un refus.
    let mut entries = base_entries();
    set(
        &mut entries,
        "thumbnail.png",
        vec![0u8; MAX_IMAGE_BYTES + 1],
    );
    let d = dirs(&entries);
    let pack = convert_dirs(&d, Some(&good_transcoder()))
        .await
        .result
        .unwrap();
    assert!(pack.thumbnail);
    assert_eq!(pack.warnings.len(), 1, "{:?}", pack.warnings);
}

#[tokio::test]
async fn sortie_identique_a_la_source_refusee() {
    let d = dirs(&base_entries());
    let before = std::fs::read(&d.input).unwrap();
    let (_tx, rx) = watch::channel(false);
    let err = convert_with(
        &d.input,
        &d.input,
        ThumbFit::default(),
        Some(&good_transcoder()),
        FFMPEG_TOTAL_BUDGET,
        &rx,
        &mut |_: &Step| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Validation(_)), "{err:?}");
    assert_eq!(std::fs::read(&d.input).unwrap(), before);
    assert_eq!(
        names_in(d.input.parent().unwrap()),
        vec!["histoire.zip".to_string()]
    );
}

#[test]
fn le_futur_de_conversion_est_send() {
    fn assert_send<T: Send>(_: &T) {}
    let (_tx, rx) = watch::channel(false);
    let mut on_step = |_: &Step| {};
    let job = StudioJob {
        input: Path::new("a.zip"),
        output: Path::new("b.lunii.pk"),
        ffmpeg: None,
        thumb_fit: ThumbFit::default(),
    };
    let future = convert(&job, &rx, &mut on_step);
    assert_send(&future);
}

// --- Archives forgées (#553) ---------------------------------------------------------------------

/// Modifie l'entrée `name` d'un zip écrit : `patch` reçoit les octets du fichier, la position de
/// son en-tête central et celle de son en-tête local.
fn patch_entry(path: &Path, name: &str, patch: impl FnOnce(&mut [u8], usize, usize)) {
    let mut bytes = std::fs::read(path).unwrap();
    let u16_at = |b: &[u8], i: usize| usize::from(u16::from_le_bytes([b[i], b[i + 1]]));
    let mut at = 0;
    let (central, local) = loop {
        let pos = bytes[at..]
            .windows(4)
            .position(|w| w == b"PK\x01\x02")
            .map(|p| p + at)
            .expect("entrée absente de l'annuaire");
        let len = u16_at(&bytes, pos + 28);
        if &bytes[pos + 46..pos + 46 + len] == name.as_bytes() {
            let local = u32::from_le_bytes(bytes[pos + 42..pos + 46].try_into().unwrap());
            break (pos, local as usize);
        }
        at = pos + 4;
    };
    assert_eq!(&bytes[local..local + 4], b"PK\x03\x04");
    patch(&mut bytes, central, local);
    std::fs::write(path, bytes).unwrap();
}

fn set_u32(bytes: &mut [u8], at: usize, value: u32) {
    bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

#[tokio::test]
async fn entree_chiffree_refusee() {
    let d = dirs(&base_entries());
    // Drapeau « chiffré » (bit 0) dans l'en-tête central et l'en-tête local.
    patch_entry(&d.input, &format!("assets/{IMG_A}"), |b, central, local| {
        b[central + 8] |= 1;
        b[local + 6] |= 1;
    });
    let fake = good_transcoder();
    let msg = expect_code(convert_dirs(&d, Some(&fake)).await, INVALID_ARCHIVE);
    assert!(msg.contains("mot de passe"), "{msg}");
    assert_eq!(fake.calls.load(Ordering::SeqCst), 0);
    assert_nothing_left(&d);
}

#[tokio::test]
async fn lien_symbolique_refuse() {
    let d = dirs(&base_entries());
    let mut zip = ZipWriter::new(File::create(&d.input).unwrap());
    let options = SimpleFileOptions::default();
    for (name, data) in base_entries() {
        zip.start_file(name.as_str(), options).unwrap();
        zip.write_all(&data).unwrap();
    }
    zip.add_symlink("assets/lien.png", "../../secret.png", options)
        .unwrap();
    zip.finish().unwrap();
    let msg = expect_code(
        convert_dirs(&d, Some(&good_transcoder())).await,
        INVALID_ARCHIVE,
    );
    assert!(msg.contains("lien symbolique"), "{msg}");
    assert_nothing_left(&d);
}

#[tokio::test]
async fn trop_d_entrees_refusees() {
    let d = dirs(&base_entries());
    let mut zip = ZipWriter::new(std::io::BufWriter::new(File::create(&d.input).unwrap()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
    for (name, data) in base_entries() {
        zip.start_file(name.as_str(), options).unwrap();
        zip.write_all(&data).unwrap();
    }
    for i in 0..=MAX_ENTRIES - base_entries().len() {
        zip.start_file(format!("assets/v{i}"), options).unwrap();
    }
    zip.finish().unwrap();
    let msg = expect_code(convert_dirs(&d, Some(&good_transcoder())).await, TOO_LARGE);
    assert!(msg.contains(&MAX_ENTRIES.to_string()), "{msg}");
    assert_nothing_left(&d);
}

#[tokio::test]
async fn taille_declaree_mensongere_refusee() {
    // Annoncée minuscule, lue au-delà de la borne des images : arrêt de la lecture.
    let mut entries = base_entries();
    let big = format!("assets/{IMG_A}");
    set(&mut entries, &big, vec![0u8; MAX_IMAGE_BYTES + 1024 * 1024]);
    let d = dirs(&entries);
    patch_entry(&d.input, &big, |b, central, local| {
        set_u32(b, central + 24, 1000);
        set_u32(b, local + 22, 1000);
    });
    expect_code(convert_dirs(&d, Some(&good_transcoder())).await, TOO_LARGE);
    assert_nothing_left(&d);

    // Annoncée énorme : total déclaré au-delà de 4 Gio, refus avant toute lecture.
    let d = dirs(&base_entries());
    for name in [format!("assets/{SND_A}"), format!("assets/{SND_B}")] {
        patch_entry(&d.input, &name, |b, central, local| {
            set_u32(b, central + 24, 0xF000_0000);
            set_u32(b, local + 22, 0xF000_0000);
        });
    }
    let msg = expect_code(convert_dirs(&d, Some(&good_transcoder())).await, TOO_LARGE);
    assert!(msg.contains("annoncés"), "{msg}");
    assert_nothing_left(&d);
}
