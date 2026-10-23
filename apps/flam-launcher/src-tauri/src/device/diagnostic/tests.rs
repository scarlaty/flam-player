//! Tests du diagnostic copiable (#318) : contenu attendu, diagnostic réduit sans Flam, aucune
//! écriture sur la Flam et, surtout, aucune fuite (clé, IV, `bt`, numéro de série, empreinte de
//! `.mdf`, UUID et titres d'histoires, profil et nom d'utilisateur).

use std::collections::BTreeMap;
use std::path::PathBuf;

use base64::Engine;
use sha2::{Digest, Sha256};

use super::*;
use crate::device::carrier::tests::{official_story, test_bt, test_key_file, OFFICIAL};
use crate::device::crypto::tests::{TEST_IV, TEST_KEY, TEST_SERIAL};
use crate::device::crypto::MDF_LEN;
use crate::device::detect::VolumeInfo;
use crate::device::guard::tests::{info_v2, key_v2, mdf_with, story, wrong_info, TEST_SERIAL_HEX};
use crate::device::guard::{KeySession, TrustStore, KNOWN_HEADER};

/// Titre de l'histoire officielle des fixtures (`INFO_PLAIN`).
const OFFICIAL_TITLE: &str = "Histoire officielle";
/// Histoire installée par ce launcher.
const MINE: &str = "11111111-2222-4333-8444-555555555555";
const MINE_TITLE: &str = "Mon histoire perso";
/// Histoire listée sans dossier.
const GHOST: &str = "99999999-8888-4777-8666-555555555555";
/// Dossier hors index.
const LOOSE: &str = "abcdefab-cdef-4abc-8def-abcdefabcdef";
/// Octets 0x6E..0x7E de `.mdf` (rôle inconnu) : jamais affichés non plus.
const TAIL: [u8; 16] = [
    0xc3, 0x1f, 0x77, 0x2b, 0x9d, 0x04, 0xe8, 0x5a, 0x61, 0xb2, 0x3e, 0xf0, 0x48, 0x8c, 0x17, 0xd9,
];
const PROFILE: &str = r"C:\Users\jdupont";
const USERNAME: &str = "jdupont";
/// Dossier de cache réglé hors du profil.
const CACHE: &str = r"D:\Jean\FlamCache";

fn write(path: &Path, data: &[u8]) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, data).unwrap();
}

fn fake_volume(_: &Path) -> Option<VolumeInfo> {
    Some(VolumeInfo {
        label: "FLAM DE JDUPONT".into(),
        fs: "FAT32".into(),
    })
}

fn fake_space(_: &Path) -> Option<Space> {
    Some(Space {
        free: 3 * 1024 * 1024 * 1024,
        total: 7 * 1024 * 1024 * 1024,
    })
}

fn env(roots: Vec<PathBuf>) -> Environment {
    Environment {
        roots,
        volume_info: fake_volume,
        space: fake_space,
    }
}

/// `.mdf` de fixture : numéro de série, clé et IV connus, fin connue.
fn fixture_mdf() -> Vec<u8> {
    let mut mdf = mdf_with(KNOWN_HEADER, "main: 2.4.2-test\ncomm: x");
    mdf[MDF_LEN - 16..].copy_from_slice(&TAIL);
    mdf
}

/// Flam 2.x de fixture : histoire officielle avec `bt` (dans `str/` et `str.hidden/`), histoire
/// installée par ce launcher, index CRLF avec un UUID en majuscules et une entrée sans dossier,
/// dossier hors index, fichiers de travail, `library.cache` contenant les titres.
fn fixture_device() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(&root.join(detect::MDF_FILE), &fixture_mdf());
    official_story(root, "str", OFFICIAL, true);
    official_story(
        root,
        "str.hidden",
        "f0e1d2c3-0000-4000-8000-5a4b3c2d1e0f",
        true,
    );
    story(root, MINE, &key_v2(), &info_v2(MINE_TITLE));
    story(root, LOOSE, &key_v2(), &info_v2("Histoire hors index"));
    write(
        &root.join("etc/library/list"),
        format!("{}\r\n{MINE}\r\n{GHOST}\r\n", OFFICIAL.to_uppercase()).as_bytes(),
    );
    write(
        &root.join("etc/library/list.hidden"),
        b"f0e1d2c3-0000-4000-8000-5a4b3c2d1e0f\n",
    );
    write(&root.join("etc/library/list.bak"), OFFICIAL.as_bytes());
    write(&root.join("etc/library/list.corrupt"), b"zzz");
    write(
        &root.join(format!(".flam-transfer/{MINE}.in/info")),
        &info_v2(MINE_TITLE),
    );
    let mut cache = Vec::new();
    for title in [OFFICIAL_TITLE, MINE_TITLE] {
        cache.push(title.len() as u8);
        cache.extend(title.as_bytes());
        cache.push(0);
    }
    write(&root.join(LIBRARY_CACHE), &cache);
    dir
}

fn redactor() -> Redactor {
    Redactor {
        profile: Some(PROFILE.into()),
        username: Some(USERNAME.into()),
        cache: Some(CACHE.into()),
    }
}

fn error(operation: &'static str, message: &str) -> SessionError {
    SessionError {
        at: Instant::now(),
        operation,
        message: message.into(),
    }
}

fn diagnostic(roots: Vec<PathBuf>, errors: Vec<SessionError>) -> String {
    diagnostic_busy(roots, errors, false)
}

fn diagnostic_busy(roots: Vec<PathBuf>, errors: Vec<SessionError>, busy: bool) -> String {
    let session: &'static KeySession = Box::leak(Box::default());
    let trust: &'static TrustStore = Box::leak(Box::default());
    let launcher = HashSet::from([MINE.to_string()]);
    let ctx = WriteContext::new(session, trust, None, launcher.clone());
    build(&Inputs {
        env: &env(roots),
        ctx: &ctx,
        busy,
        launcher_stories: &launcher,
        os: "Windows 11 (build 26100.1, 24H2), x86_64".into(),
        errors,
        redactor: &redactor(),
        now: Instant::now(),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn b64(bytes: &[u8]) -> Vec<String> {
    use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE};
    vec![
        STANDARD.encode(bytes),
        STANDARD_NO_PAD.encode(bytes),
        URL_SAFE.encode(bytes),
    ]
}

/// Valeurs qui ne doivent jamais apparaître, sous toutes leurs formes.
fn secrets() -> Vec<(String, String)> {
    let mdf = fixture_mdf();
    let bt = test_bt();
    let mut prefixed = Sha256::new();
    prefixed.update(b"flam-launcher/mdf-fingerprint/v1/");
    prefixed.update(&mdf);
    let binaries: Vec<(&str, Vec<u8>)> = vec![
        ("numéro de série (octets)", TEST_SERIAL.to_vec()),
        ("numéro de série (texte)", TEST_SERIAL_HEX.to_vec()),
        ("clé", TEST_KEY.to_vec()),
        ("IV", TEST_IV.to_vec()),
        ("clé + IV", [TEST_KEY, TEST_IV].concat()),
        ("bt", bt.to_vec()),
        ("clé du bt", bt[..16].to_vec()),
        ("IV du bt", bt[16..].to_vec()),
        ("fichier key 2.x", key_v2()),
        ("fichier key officiel", test_key_file().to_vec()),
        ("fin de .mdf", TAIL.to_vec()),
        ("SHA-256 de .mdf", Sha256::digest(&mdf).to_vec()),
        ("empreinte de .mdf", prefixed.finalize().to_vec()),
        (".mdf entier", mdf.clone()),
    ];
    let mut out = Vec::new();
    for (name, bytes) in binaries {
        let h = hex(&bytes);
        out.push((format!("{name} (hex minuscule)"), h.clone()));
        out.push((format!("{name} (hex majuscule)"), h.to_uppercase()));
        for encoded in b64(&bytes) {
            out.push((format!("{name} (base64)"), encoded));
        }
    }
    let sha = hex(&Sha256::digest(&mdf));
    out.push((
        "début du SHA-256 (nom de sauvegarde)".into(),
        sha[..16].into(),
    ));
    out.push((
        "numéro de série (texte, majuscules)".into(),
        String::from_utf8(TEST_SERIAL_HEX.to_vec())
            .unwrap()
            .to_uppercase(),
    ));
    for uuid in [
        OFFICIAL,
        "f0e1d2c3-0000-4000-8000-5a4b3c2d1e0f",
        MINE,
        GHOST,
        LOOSE,
    ] {
        out.push((format!("UUID {uuid}"), uuid.into()));
        out.push((format!("UUID {uuid} (majuscules)"), uuid.to_uppercase()));
        out.push((format!("UUID {uuid} (simple)"), uuid.replace('-', "")));
        out.push((format!("UUID {uuid} (court)"), uuid[..8].into()));
    }
    for title in [OFFICIAL_TITLE, MINE_TITLE, "Histoire hors index"] {
        out.push((format!("titre {title}"), title.into()));
    }
    out.push(("profil".into(), PROFILE.into()));
    out.push(("profil (/)".into(), PROFILE.replace('\\', "/")));
    out.push(("nom d'utilisateur".into(), USERNAME.into()));
    out.push(("nom du volume".into(), "FLAM DE JDUPONT".into()));
    out.push(("dossier de cache".into(), CACHE.into()));
    out.push(("dossier de cache (/)".into(), CACHE.replace('\\', "/")));
    out
}

fn assert_no_leak(text: &str) {
    let lower = text.to_lowercase();
    for (name, value) in secrets() {
        assert!(
            !text.contains(&value) && !lower.contains(&value.to_lowercase()),
            "fuite : {name} ({value}) dans le diagnostic :\n{text}"
        );
    }
}

/// Erreur qui contient tout ce qu'il ne faut pas publier (message d'une couche basse).
fn leaky_error() -> SessionError {
    let bt = test_bt();
    error(
        "envoi",
        &format!(
            "échec sur {PROFILE}\\AppData\\Local\\flam\\x.tmp et C:/Users/{USERNAME}/Desktop (utilisateur {USERNAME}), \
             cache {CACHE}\\work\\y.tmp : bt {} / {} / {}, série {} {}, clé {}, histoire {} {}",
            hex(&bt),
            hex(&bt).to_uppercase(),
            b64(&bt)[0],
            String::from_utf8(TEST_SERIAL_HEX.to_vec()).unwrap(),
            b64(TEST_SERIAL_HEX)[0],
            b64(&TEST_KEY)[0],
            OFFICIAL.to_uppercase(),
            OFFICIAL.replace('-', ""),
        ),
    )
}

/// Arborescence complète avec contenu.
fn snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                out.insert(path.clone(), None);
                walk(&path, out);
            } else {
                out.insert(path.clone(), Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, &mut out);
    out
}

#[test]
fn aucune_cle_bt_numero_de_serie_ni_empreinte_dans_le_diagnostic() {
    let dev = fixture_device();
    let text = diagnostic(vec![dev.path().to_path_buf()], vec![leaky_error()]);
    assert_no_leak(&text);
    // L'erreur reste lisible, profil remplacé.
    assert!(
        text.contains("%USERPROFILE%\\AppData\\Local\\flam\\x.tmp"),
        "{text}"
    );
    assert!(text.contains("et %USERPROFILE%/Desktop"), "{text}");
    assert!(text.contains("(utilisateur %USERNAME%)"), "{text}");
    assert!(text.contains("cache %CACHE%\\work\\y.tmp"), "{text}");
    assert!(text.contains("histoire <uuid>"), "{text}");
}

#[test]
fn le_diagnostic_decrit_la_flam_sans_rien_ecrire() {
    let dev = fixture_device();
    let before = snapshot(dev.path());
    let text = diagnostic(vec![dev.path().to_path_buf()], Vec::new());
    assert_eq!(snapshot(dev.path()), before, "aucune écriture sur la Flam");
    for expected in [
        &format!("- Launcher : {}", env!("CARGO_PKG_VERSION")),
        "- Système : Windows 11 (build 26100.1, 24H2), x86_64",
        "- Flam détectées : 1",
        "- Système de fichiers : FAT32",
        "- Espace : 3,00 Gio (3221225472 octets) libres sur 7,00 Gio (7516192768 octets)",
        "- Version : 2.4.2 (famille 2.x, préfixe « main: »)",
        "- .mdf : 126 octets, en-tête 01 00, disposition standard : oui",
        "- VID:PID : 303a:819e",
        "- Confiance : vérifié par la Flam (flam_verified)",
        "- Écriture : autorisée",
        "| Entrées | 3 | 1 |",
        "| Fins de ligne | CRLF | LF |",
        "| UUID non canoniques | 1 | 0 |",
        "| Entrées sans dossier | 1 | 0 |",
        "- Dossiers hors index : str/ 1, str.hidden/ 0",
        "- Histoires (UUID distincts, index et dossiers) : 5 (installées par ce launcher : 1 ; autres, officielles ou d'autres outils : 4)",
        "- .flam-transfer : présent (1 entrée(s))",
        "- .flam-trash : absent",
        "- etc/library : list.bak (36 octets), list.corrupt (3 octets)",
        "- usr/0/library.cache : présent, 41 octets",
        "### Erreurs de la session\n\n- aucune",
    ] {
        assert!(text.contains(expected), "attendu « {expected} » dans :\n{text}");
    }
    assert!(!text.contains("FLAM DE"), "pas de nom de volume");
}

#[test]
fn sans_flam_le_diagnostic_est_reduit() {
    let empty = tempfile::tempdir().unwrap();
    let text = diagnostic(
        vec![empty.path().to_path_buf()],
        vec![error("reprise", "la Flam est occupée")],
    );
    assert!(text.contains("- Flam détectées : 0"));
    assert!(text.contains("Aucune Flam détectée : diagnostic réduit."));
    assert!(text.contains("- reprise (il y a 0 s) : la Flam est occupée"));
    assert!(!text.contains("### Firmware"));
}

#[test]
fn une_flam_refusee_par_la_garde_a_quand_meme_son_diagnostic() {
    let dev = fixture_device();
    // Clé vide : la garde refuse toute écriture.
    let mut mdf = fixture_mdf();
    mdf[0x4E..0x5E].fill(0);
    write(&dev.path().join(detect::MDF_FILE), &mdf);
    let text = diagnostic(vec![dev.path().to_path_buf()], Vec::new());
    assert!(text.contains("- Écriture : refusée (keys_trivial) : clés de la Flam invalides"));
    assert_no_leak(&text);
    // `.mdf` tronqué : refus, diagnostic toujours produit.
    write(&dev.path().join(detect::MDF_FILE), &mdf[..40]);
    let text = diagnostic(vec![dev.path().to_path_buf()], Vec::new());
    assert!(text.contains("- Écriture : refusée (truncated)"), "{text}");
    assert!(text.contains("fichier .mdf tronqué (40 octets"), "{text}");
}

#[test]
fn le_masquage_garde_le_texte_utile() {
    let r = redactor();
    assert_eq!(
        r.scrub("usr/0/library.cache, str.hidden/, flam-launcher 0.1.0-beta.1, 303a:819e, 01 00"),
        "usr/0/library.cache, str.hidden/, flam-launcher 0.1.0-beta.1, 303a:819e, 01 00"
    );
    assert_eq!(r.scrub("série 0123456789abcd fin"), "série <masqué> fin");
    assert_eq!(r.scrub("court 0123456789abc"), "court 0123456789abc");
    assert_eq!(r.scrub("b64 oKGio6Slpqeoqaqr0w== fin"), "b64 <masqué> fin");
    assert_eq!(r.scrub("C:\\Users\\Public\\x"), "C:\\Users\\Public\\x");
    assert_eq!(r.scrub("D:\\users\\Autre\\x"), "D:\\users\\%USERNAME%\\x");
    // Le nom d'utilisateur n'est remplacé que comme mot entier.
    assert_eq!(
        r.clean_message("jdupontx, jdupont, JDUPONT\nfin"),
        "jdupontx, %USERNAME%, %USERNAME% fin"
    );
    let long = "é".repeat(MAX_ERROR_LEN + 5);
    assert_eq!(r.clean_message(&long).chars().count(), MAX_ERROR_LEN + 1);
}

#[test]
fn les_erreurs_de_la_session_sont_bornees() {
    for i in 0..MAX_SESSION_ERRORS + 3 {
        record_error("envoi", None, &format!("erreur {i}"));
    }
    let errors = session_errors();
    assert_eq!(errors.len(), MAX_SESSION_ERRORS);
    assert_eq!(
        errors.last().unwrap().message,
        format!("erreur {}", MAX_SESSION_ERRORS + 2)
    );
    // Revue #329 : nettoyage à l'enregistrement avec le profil réel (`Redactor::current`) et le
    // dossier de cache. Dans le même test, la liste des erreurs étant globale.
    let cache = Path::new(r"D:\Jean Dupont\FlamCache");
    let profile = Redactor::current().profile;
    let message = format!(
        r"{}\AppData\x.tmp et {}\work\y.tmp",
        profile.as_deref().unwrap_or(r"C:\Users\Jean Dupont"),
        cache.display()
    );
    record_error("reprise", Some(cache), &message);
    let recorded = session_errors().last().unwrap().message.clone();
    assert!(recorded.ends_with(r"et %CACHE%\work\y.tmp"), "{recorded}");
    assert!(!recorded.contains("Jean"), "{recorded}");
    if let Some(profile) = profile {
        assert!(!recorded.contains(&profile), "{recorded}");
    }
}

#[test]
fn les_fins_de_ligne_sont_reconnues() {
    assert_eq!(line_endings(b""), (LineEndings::None, false));
    assert_eq!(line_endings(b"a\nb\n"), (LineEndings::Lf, true));
    assert_eq!(line_endings(b"a\r\nb"), (LineEndings::Crlf, false));
    assert_eq!(line_endings(b"a\rb\r"), (LineEndings::Cr, true));
    assert_eq!(line_endings(b"a\r\nb\n"), (LineEndings::Mixed, true));
}

#[test]
fn la_description_du_systeme_n_est_jamais_vide() {
    let os = os_description();
    assert!(os.ends_with(std::env::consts::ARCH));
    assert!(os.len() > std::env::consts::ARCH.len() + 2);
}

#[test]
#[ignore = "lecteurs réels : nécessite Windows, résultat selon le matériel branché (lecture seule)"]
fn diagnostic_reel() {
    let session: &'static KeySession = Box::leak(Box::default());
    let trust: &'static TrustStore = Box::leak(Box::default());
    let ctx = WriteContext::new(session, trust, None, HashSet::new());
    let text = build(&Inputs {
        env: &Environment::current(),
        ctx: &ctx,
        busy: false,
        launcher_stories: &HashSet::new(),
        os: os_description(),
        errors: Vec::new(),
        redactor: &Redactor::current(),
        now: Instant::now(),
    });
    println!("{text}");
}

/// Revue #318 (M1) : une erreur de la bibliothèque qui cite le titre de l'histoire n'en garde que
/// le code.
#[test]
fn une_erreur_qui_cite_un_titre_n_est_pas_reprise() {
    let missing = AppError::StoryFileMissing(format!(
        "le fichier de « {MINE_TITLE} » a disparu de la bibliothèque"
    ));
    let message = session_message(&missing);
    assert_eq!(
        message,
        "fichier de l'histoire absent de la bibliothèque du launcher"
    );
    let other = AppError::FolderUnavailable(format!("dossier de {OFFICIAL_TITLE} absent"));
    assert_eq!(
        session_message(&other),
        "erreur de type « folder_unavailable » (message non repris)"
    );
    let dev = fixture_device();
    let text = diagnostic(
        vec![dev.path().to_path_buf()],
        vec![
            error("envoi", &message),
            error("envoi", &session_message(&other)),
            error(
                "reprise",
                &session_message(&AppError::Validation("refus".into())),
            ),
        ],
    );
    assert_no_leak(&text);
    assert!(text.contains("- reprise (il y a 0 s) : refus"), "{text}");
}

#[test]
fn un_uuid_colle_a_un_mot_est_masque() {
    let r = Redactor::default();
    let upper = OFFICIAL.to_uppercase();
    assert_eq!(
        r.scrub(&format!("str/{OFFICIAL}/info x={upper}_y")),
        "str/<uuid>/info x=<uuid>_y"
    );
}

/// Revue #326 (I3) : les UUID des histoires en contradiction ne sont jamais repris, même avant
/// le filet (la ligne de cohérence ne contient aucun `<uuid>`).
#[test]
fn une_contradiction_ne_cite_aucun_uuid() {
    const BAD: &str = "77777777-6666-4555-8444-333333333333";
    let dev = tempfile::tempdir().unwrap();
    write(&dev.path().join(detect::MDF_FILE), &fixture_mdf());
    story(dev.path(), BAD, &key_v2(), &wrong_info());
    let text = diagnostic(vec![dev.path().to_path_buf()], Vec::new());
    let line = text
        .lines()
        .find(|l| l.starts_with("- Cohérence des clés"))
        .unwrap_or_else(|| panic!("ligne de cohérence absente :\n{text}"));
    assert!(line.contains("contradiction :"), "{line}");
    assert!(!line.contains("<uuid>") && !line.contains(BAD), "{line}");
    assert!(!text.to_lowercase().contains(BAD), "{text}");
}

/// Revue #329 : un nom de compte avec espace ou apostrophe est masqué en entier, même quand le
/// chemin n'est pas celui du profil courant.
#[test]
fn un_profil_avec_espace_ou_apostrophe_est_masque_en_entier() {
    let r = Redactor {
        profile: Some(r"C:\Users\Jean Dupont".into()),
        username: Some("Jean Dupont".into()),
        cache: None,
    };
    assert_eq!(
        r.clean_message(r"échec C:\Users\Jean Dupont\x, D:\Users\Marie O'Brien\y (fin)"),
        r"échec %USERPROFILE%\x, D:\Users\%USERNAME%\y (fin)"
    );
    assert_eq!(
        r.scrub("C:/Users/Marie O'Brien: refus\nC:\\Users\\Paul Martin\nsuite"),
        "C:/Users/%USERNAME%: refus\nC:\\Users\\%USERNAME%\nsuite"
    );
    assert_eq!(r.scrub(r#""C:\Users\Ana Lee""#), r#""C:\Users\%USERNAME%""#);
}

/// Revue #329 : filet durci (paires hexadécimales séparées, base64 sans chiffre, UUID collé).
#[test]
fn le_filet_masque_hex_separe_base64_sans_chiffre_et_uuid_colle() {
    let r = Redactor::default();
    let pairs: Vec<String> = TAIL.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        r.scrub(&format!("clé {} fin", pairs.join(" "))),
        "clé <masqué> fin"
    );
    assert_eq!(
        r.scrub(&format!("clé {} fin", pairs.join(":").to_uppercase())),
        "clé <masqué> fin"
    );
    // Moins de 8 paires (adresse MAC, en-tête, VID:PID, mots courts) : gardé.
    for kept in ["aa:bb:cc:dd:ee:ff", "01 00", "303a:819e", "de ce fa be"] {
        assert_eq!(r.scrub(kept), kept);
    }
    // Base64 sans chiffre (avec ou sans remplissage) ; identifiant CamelCase gardé.
    assert_eq!(r.scrub("k AbCdEfGhIjKlMnOpQrStUv fin"), "k <masqué> fin");
    assert_eq!(r.scrub("k AbcdEfghIjklMnop== fin"), "k <masqué> fin");
    assert_eq!(
        r.scrub("StoryFileMissingWarning"),
        "StoryFileMissingWarning"
    );
    // Chemin à dossier en majuscules : gardé.
    assert_eq!(
        r.scrub("D:/FLAMCACHE/work/stories"),
        "D:/FLAMCACHE/work/stories"
    );
    // Cache réglé sur un parent du profil : le profil reste masqué.
    let parent = Redactor {
        profile: Some(PROFILE.into()),
        username: None,
        cache: Some(r"C:\Users".into()),
    };
    assert_eq!(
        parent.clean_message(&format!(r"{PROFILE}\x")),
        r"%USERPROFILE%\x"
    );
    // UUID collé à des caractères hexadécimaux.
    assert_eq!(r.scrub(&format!("cafe{OFFICIAL}0")), "cafe<uuid>0");
    assert_eq!(
        r.scrub(&format!("x {}f", OFFICIAL.to_uppercase())),
        "x <uuid>f"
    );
}

/// Revue #329 : pendant une écriture, ni index, ni dossiers, ni fichiers de travail ne sont
/// parcourus.
#[test]
fn pendant_une_ecriture_les_balayages_sont_sautes() {
    let dev = fixture_device();
    let text = diagnostic_busy(vec![dev.path().to_path_buf()], Vec::new(), true);
    assert!(text.contains("### Firmware"), "{text}");
    assert!(
        text.contains("- non examinés : écriture en cours sur la Flam"),
        "{text}"
    );
    for absent in [
        "| Entrées",
        "Dossiers hors index",
        ".flam-transfer",
        "- etc/library",
    ] {
        assert!(!text.contains(absent), "« {absent} » inattendu :\n{text}");
    }
}

/// Revue #329 : le suffixe libre d'un `list.corrupt*` n'est pas affiché.
#[test]
fn le_suffixe_d_un_index_abime_n_est_pas_affiche() {
    let dev = fixture_device();
    write(
        &dev.path()
            .join("etc/library/list.hidden.corrupt-Mon histoire perso"),
        b"zz",
    );
    let text = diagnostic(vec![dev.path().to_path_buf()], Vec::new());
    assert!(
        text.contains("list.corrupt (3 octets), list.hidden.corrupt… (2 octets)"),
        "{text}"
    );
    assert_no_leak(&text);
}

/// #342 : un cache réglé sur `C:\Users` ne fait pas passer le nom d'un autre compte pour un
/// sous-dossier du cache.
#[test]
fn un_cache_regle_sur_users_ne_laisse_pas_fuiter_un_autre_compte() {
    for cache in [r"C:\Users", r"C:\Users\", "c:/users"] {
        let r = Redactor {
            profile: Some(PROFILE.into()),
            username: None,
            cache: Some(cache.into()),
        };
        assert_eq!(
            r.clean_message(r"échec C:\Users\Marie\x et C:\Users\jdupont\y"),
            r"échec C:\Users\%USERNAME%\x et %USERPROFILE%\y",
            "cache {cache}"
        );
    }
}

/// #342 : la casse Unicode à longueur variable (`İ` → `i̇`) ne fait pas rater le nom, ne décale
/// ni la suite du texte ni les frontières de mot.
#[test]
fn la_casse_unicode_a_longueur_variable_est_geree() {
    assert_eq!(replace_ci("a İSTANBUL b", "i̇stanbul", "X"), "a X b");
    assert_eq!(replace_ci("a İSTANBUL b", "İstanbul", "X"), "a X b");
    // Le motif ne peut pas s'arrêter au milieu d'un caractère d'origine.
    assert_eq!(replace_ci("İ", "i", "X"), "İ");
    assert_eq!(replace_word_ci("İbob, bob", "bob", "X"), "İbob, X");
    assert_eq!(replace_word_ci("xİ bob İ", "bob", "N"), "xİ N İ");
}

/// #342 : plusieurs Flam, toutes décrites.
#[test]
fn plusieurs_flam_sont_toutes_decrites() {
    let a = fixture_device();
    let b = fixture_device();
    let text = diagnostic(
        vec![a.path().to_path_buf(), b.path().to_path_buf()],
        Vec::new(),
    );
    assert!(text.contains("- Flam détectées : 2"), "{text}");
    assert!(text.contains("**Flam 1 sur 2**"), "{text}");
    assert!(text.contains("**Flam 2 sur 2**"), "{text}");
    assert_eq!(text.matches("### Firmware").count(), 2, "{text}");
    // Une seule Flam : pas d'intertitre.
    let one = diagnostic(vec![a.path().to_path_buf()], Vec::new());
    assert!(!one.contains("**Flam 1 sur"), "{one}");
}
