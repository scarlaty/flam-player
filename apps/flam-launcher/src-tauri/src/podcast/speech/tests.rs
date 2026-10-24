use super::testing::{sine_wav, voice, wav_bytes, Simulated, SECS_PER_CHAR};
use super::*;
use crate::ffmpeg::MANIFEST_FILE;

const SHORT: Duration = Duration::from_secs(5);

fn no_op() {}

// --- texte -------------------------------------------------------------------------------------

#[test]
fn le_texte_est_nettoye_sur_une_ligne() {
    assert_eq!(
        prepare_text("  Épisode   3 :\tL'été \n deuxième ligne ").as_deref(),
        Some("Épisode 3 : L'été")
    );
    assert_eq!(
        prepare_text("\n\n  Titre\r\nsuite").as_deref(),
        Some("Titre")
    );
    assert_eq!(
        prepare_text("Ti\u{0007}tre\u{0000}").as_deref(),
        Some("Titre")
    );
}

#[test]
fn les_titres_sans_lettre_restent_muets() {
    for raw in ["", "   ", "\n\n", "🎧", "#12", "12 - 34", "!!!"] {
        assert_eq!(prepare_text(raw), None, "{raw:?}");
    }
    assert_eq!(
        prepare_text("#12 Le retour").as_deref(),
        Some("#12 Le retour")
    );
}

#[test]
fn le_texte_long_est_coupe_proprement() {
    let long = "mot ".repeat(60);
    let text = prepare_text(&long).unwrap();
    assert!(text.chars().count() <= MAX_TEXT_CHARS);
    assert!(text.ends_with("mot"), "coupe sur un mot : {text:?}");
    // Un seul très long mot : coupé net à 120 caractères.
    let word = "é".repeat(300);
    assert_eq!(prepare_text(&word).unwrap().chars().count(), MAX_TEXT_CHARS);
    // Exactement 120 caractères : conservé.
    let exact = "a".repeat(MAX_TEXT_CHARS);
    assert_eq!(prepare_text(&exact).unwrap(), exact);
}

// --- voix --------------------------------------------------------------------------------------

#[test]
fn seules_les_voix_francaises_locales_sont_gardees_et_triees() {
    let synth = Simulated {
        default_id: Some("en".into()),
        ..Simulated::new(vec![
            voice("en", "Microsoft David", "en-US"),
            voice("fr-paul", "Microsoft Paul", "fr-FR"),
            voice("ca", "Microsoft Claude", "fr-CA"),
            voice("fr-julie", "microsoft Julie", "fr-FR"),
            voice("on", "Microsoft Denise Online (Natural)", "fr-FR"),
            voice("on2", "Voix en ligne", "fr-FR"),
            voice("fr", "Générique", "fr"),
            voice("frx", "Faux ami", "frx-XX"),
            voice("de", "Microsoft Hedda", "de-DE"),
        ])
    };
    let list = french_voices(&synth).unwrap();
    let ids: Vec<_> = list.voices.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, ["fr", "ca", "fr-julie", "fr-paul"]);
    // Le défaut du système n'est pas français : première voix triée.
    assert_eq!(list.default_id.as_deref(), Some("fr"));
}

#[test]
fn le_defaut_du_systeme_est_garde_s_il_est_francais() {
    let synth = Simulated {
        default_id: Some("fr-paul".into()),
        ..Simulated::new(vec![
            voice("fr-julie", "Microsoft Julie", "fr-FR"),
            voice("fr-paul", "Microsoft Paul", "fr-FR"),
        ])
    };
    let list = french_voices(&synth).unwrap();
    assert_eq!(list.default_id.as_deref(), Some("fr-paul"));
    assert_eq!(list.voices[0].id, "fr-julie");
}

#[test]
fn les_doublons_d_identifiant_sont_fusionnes() {
    let synth = Simulated::new(vec![
        voice("a", "Microsoft Julie", "fr-FR"),
        voice("a", "Microsoft Julie", "fr-FR"),
    ]);
    assert_eq!(french_voices(&synth).unwrap().voices.len(), 1);
}

#[test]
fn un_meme_identifiant_sous_deux_noms_n_apparait_qu_une_fois() {
    // Les deux entrées ne sont pas adjacentes une fois triées par nom.
    let synth = Simulated::new(vec![
        voice("a", "Microsoft Julie", "fr-FR"),
        voice("b", "Microsoft Marie", "fr-FR"),
        voice("a", "Microsoft Paul", "fr-FR"),
    ]);
    let list = french_voices(&synth).unwrap();
    let names: Vec<_> = list.voices.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["Microsoft Julie", "Microsoft Marie"]);
}

#[test]
fn aucune_voix_francaise_donne_une_liste_vide() {
    let only_english = Simulated::new(vec![voice("en", "Microsoft David", "en-US")]);
    let list = french_voices(&only_english).unwrap();
    assert!(list.voices.is_empty());
    assert_eq!(list.default_id, None);
    assert_eq!(
        resolve_voice(&list, None).unwrap_err(),
        SpeechError::NoVoice
    );
    assert_eq!(
        resolve_voice(&list, Some("en")).unwrap_err(),
        SpeechError::NoVoice
    );
}

#[test]
fn la_voix_est_resolue_ou_declaree_disparue() {
    let synth = Simulated::new(vec![
        voice("a", "Microsoft Julie", "fr-FR"),
        voice("b", "Microsoft Paul", "fr-FR"),
    ]);
    let list = french_voices(&synth).unwrap();
    assert_eq!(resolve_voice(&list, None).unwrap().id, "a");
    assert_eq!(resolve_voice(&list, Some("b")).unwrap().id, "b");
    assert_eq!(
        resolve_voice(&list, Some("disparue")).unwrap_err(),
        SpeechError::VoiceGone
    );
}

#[test]
fn le_service_indisponible_est_une_liste_vide_pour_l_interface() {
    let synth = Simulated {
        unavailable: true,
        ..Simulated::french()
    };
    assert_eq!(french_voices(&synth).unwrap_err(), SpeechError::Unavailable);
    let list = voices_for_front(&synth).unwrap();
    assert!(list.voices.is_empty());
    assert_eq!(list.default_id, None);
    let none = voices_for_front(&NoSpeech).unwrap();
    assert!(none.voices.is_empty());
}

#[test]
fn le_moteur_du_systeme_est_borne_hors_windows() {
    if cfg!(windows) {
        return;
    }
    let sys = system();
    assert_eq!(sys.voices().unwrap_err(), SpeechError::Unavailable);
    assert_eq!(sys.default_voice_id(), None);
    assert_eq!(
        sys.synthesize("Bonjour", "x").unwrap_err(),
        SpeechError::Unavailable
    );
}

// --- WAV ---------------------------------------------------------------------------------------

#[test]
fn un_wav_valide_est_decrit() {
    let wav = Wav::parse(sine_wav(1.0)).unwrap();
    assert_eq!(wav.sample_rate, 22_050);
    assert_eq!(wav.channels, 1);
    assert!((wav.duration_secs - 1.0).abs() < 0.001);
    let stereo = Wav::parse(wav_bytes(44_100, 2, 16, &[0u8; 4 * 44_100])).unwrap();
    assert_eq!(stereo.channels, 2);
    assert!((stereo.duration_secs - 1.0).abs() < 0.001);
}

#[test]
fn un_wav_invalide_est_refuse() {
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("vide", Vec::new()),
        ("pas de RIFF", b"OggS____WAVEfmt ".to_vec()),
        ("sans données", wav_bytes(22_050, 1, 16, &[])),
        ("8 bits", wav_bytes(22_050, 1, 8, &[0; 100])),
        ("24 bits", wav_bytes(22_050, 1, 24, &[0; 300])),
        ("canaux", wav_bytes(22_050, 6, 16, &[0; 1200])),
        ("fréquence", wav_bytes(100, 1, 16, &[0; 200])),
        ("une demi-trame", wav_bytes(22_050, 2, 16, &[0; 3])),
    ];
    for (what, bytes) in cases {
        assert!(
            matches!(Wav::parse(bytes), Err(SpeechError::Failed(_))),
            "{what}"
        );
    }
    // En-tête tronqué au milieu du bloc fmt, et données avant fmt.
    let mut truncated = wav_bytes(22_050, 1, 16, &[0; 100]);
    truncated.truncate(30);
    assert!(matches!(Wav::parse(truncated), Err(SpeechError::Failed(_))));
    let mut data_first = b"RIFF\0\0\0\0WAVEdata".to_vec();
    data_first.extend_from_slice(&4u32.to_le_bytes());
    data_first.extend_from_slice(&[0; 4]);
    assert!(matches!(
        Wav::parse(data_first),
        Err(SpeechError::Failed(_))
    ));
}

/// WAV `WAVE_FORMAT_EXTENSIBLE` (bloc `fmt ` de 40 octets) dont le sous-format commence par
/// `subformat` (1 : PCM, 3 : flottant).
fn extensible_wav(subformat: u32) -> Vec<u8> {
    let mut bytes = wav_bytes(22_050, 1, 16, &[1; 400]);
    let mut fmt = bytes[20..36].to_vec();
    fmt[0..2].copy_from_slice(&0xFFFEu16.to_le_bytes());
    fmt.extend_from_slice(&22u16.to_le_bytes());
    fmt.extend_from_slice(&16u16.to_le_bytes());
    fmt.extend_from_slice(&4u32.to_le_bytes());
    fmt.extend_from_slice(&subformat.to_le_bytes());
    fmt.extend_from_slice(&[
        0x00, 0x00, 0x10, 0x00, 0x80, 0x00, 0x00, 0xAA, 0x00, 0x38, 0x9B, 0x71,
    ]);
    let data = bytes.split_off(36);
    bytes.truncate(16);
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&fmt);
    bytes.extend_from_slice(&data);
    bytes
}

#[test]
fn le_sous_format_d_un_wav_extensible_est_controle() {
    assert_eq!(Wav::parse(extensible_wav(1)).unwrap().sample_rate, 22_050);
    assert!(matches!(
        Wav::parse(extensible_wav(3)),
        Err(SpeechError::Failed(_))
    ));
}

#[test]
fn un_wav_trop_long_est_refuse() {
    let long = sine_wav(MAX_AUDIO_SECS + 1.0);
    assert_eq!(Wav::parse(long).unwrap_err(), SpeechError::TooLong);
    assert!(Wav::parse(sine_wav(MAX_AUDIO_SECS - 0.5)).is_ok());
    let huge = vec![0u8; MAX_WAV_BYTES + 1];
    assert_eq!(Wav::parse(huge).unwrap_err(), SpeechError::TooLong);
}

#[test]
fn la_taille_de_donnees_annoncee_trop_grande_est_ramenee_au_contenu() {
    let mut bytes = wav_bytes(22_050, 1, 16, &[1; 2_000]);
    bytes[40..44].copy_from_slice(&u32::MAX.to_le_bytes());
    let wav = Wav::parse(bytes).unwrap();
    assert!((wav.duration_secs - 1_000.0 / 22_050.0).abs() < 1e-9);
}

#[test]
fn un_bloc_inconnu_avant_les_donnees_est_saute() {
    let canonical = wav_bytes(22_050, 1, 16, &[1; 400]);
    let mut bytes = canonical[..36].to_vec();
    // Bloc `LIST` de taille impaire (rembourrage d'un octet), puis `data`.
    bytes.extend_from_slice(b"LIST");
    bytes.extend_from_slice(&3u32.to_le_bytes());
    bytes.extend_from_slice(&[9, 9, 9, 0]);
    bytes.extend_from_slice(&canonical[36..]);
    assert_eq!(Wav::parse(bytes).unwrap().sample_rate, 22_050);
}

// --- synthèse ----------------------------------------------------------------------------------

#[test]
fn la_voix_simulee_produit_une_duree_proportionnelle_au_texte() {
    let synth = Simulated::french().shared();
    let short = synthesize_wav(&synth, "Bonjour", "fr-hortense", SHORT).unwrap();
    let long = synthesize_wav(&synth, "Bonjour tout le monde", "fr-hortense", SHORT).unwrap();
    assert!((short.duration_secs - 7.0 * SECS_PER_CHAR).abs() < 0.01);
    assert!(long.duration_secs > short.duration_secs);
    // Déterministe.
    let again = synthesize_wav(&synth, "Bonjour", "fr-hortense", SHORT).unwrap();
    assert_eq!(again, short);
}

#[test]
fn la_synthese_lit_le_texte_prepare() {
    let synth = Simulated {
        failing: vec!["Titre".into()],
        ..Simulated::french()
    }
    .shared();
    // Le texte est préparé avant d'atteindre le moteur : « Titre » est bien celui qui échoue.
    assert!(matches!(
        synthesize_wav(&synth, "  Titre \n suite", "fr-hortense", SHORT),
        Err(SpeechError::Failed(_))
    ));
    assert_eq!(
        synthesize_wav(&synth, "🎧", "fr-hortense", SHORT).unwrap_err(),
        SpeechError::NothingToSay
    );
}

#[test]
fn les_pannes_de_la_voix_sont_distinguees() {
    let failing = Simulated {
        failing: vec!["Titre".into()],
        ..Simulated::french()
    }
    .shared();
    assert!(matches!(
        synthesize_wav(&failing, "Titre", "fr-hortense", SHORT),
        Err(SpeechError::Failed(_))
    ));
    assert_eq!(
        synthesize_wav(&failing, "Titre", "disparue", SHORT).unwrap_err(),
        SpeechError::VoiceGone
    );
    let unavailable = Simulated {
        unavailable: true,
        ..Simulated::french()
    }
    .shared();
    assert_eq!(
        synthesize_wav(&unavailable, "Titre", "fr-hortense", SHORT).unwrap_err(),
        SpeechError::Unavailable
    );
}

#[test]
fn une_voix_qui_ne_rend_pas_la_main_tombe_en_delai() {
    let slow = Simulated {
        delay: Duration::from_millis(600),
        ..Simulated::french()
    }
    .shared();
    let err = synthesize_wav(&slow, "Titre", "fr-hortense", Duration::from_millis(50)).unwrap_err();
    assert_eq!(err, SpeechError::Timeout);
}

#[test]
fn un_wav_vide_corrompu_ou_trop_long_est_un_echec() {
    let bad = |wav: Vec<u8>| {
        Simulated {
            wav: Some(wav),
            ..Simulated::french()
        }
        .shared()
    };
    for wav in [
        Vec::new(),
        b"pas du WAV du tout".to_vec(),
        wav_bytes(22_050, 1, 16, &[]),
    ] {
        let synth = bad(wav);
        assert!(matches!(
            synthesize_wav(&synth, "Titre", "fr-hortense", SHORT),
            Err(SpeechError::Failed(_))
        ));
    }
    let long = bad(sine_wav(MAX_AUDIO_SECS + 2.0));
    assert_eq!(
        synthesize_wav(&long, "Titre", "fr-hortense", SHORT).unwrap_err(),
        SpeechError::TooLong
    );
}

#[test]
fn un_moteur_qui_panique_est_un_echec_pas_un_blocage() {
    struct Panics;
    impl Synthesizer for Panics {
        fn voices(&self) -> Result<Vec<Voice>, SpeechError> {
            Ok(Vec::new())
        }
        fn default_voice_id(&self) -> Option<String> {
            None
        }
        fn synthesize(&self, _: &str, _: &str) -> Result<Vec<u8>, SpeechError> {
            panic!("moteur en panne");
        }
    }
    let synth: Arc<dyn Synthesizer> = Arc::new(Panics);
    assert!(matches!(
        synthesize_wav(&synth, "Titre", "x", SHORT),
        Err(SpeechError::Failed(_))
    ));
}

// --- erreurs -----------------------------------------------------------------------------------

#[test]
fn les_erreurs_ont_des_codes_stables_sans_texte_ni_voix() {
    let kind = |e: SpeechError| AppError::from(e).kind();
    assert_eq!(kind(SpeechError::Unavailable), "tts_unavailable");
    assert_eq!(kind(SpeechError::NoVoice), "tts_no_voice");
    assert_eq!(kind(SpeechError::VoiceGone), "tts_no_voice");
    assert_eq!(kind(SpeechError::Timeout), "tts_failed");
    assert_eq!(kind(SpeechError::TooLong), "tts_failed");
    assert_eq!(kind(SpeechError::Failed("x".into())), "tts_failed");
    assert_eq!(kind(SpeechError::NothingToSay), "validation");
    let msg = AppError::from(SpeechError::NoVoice).to_string();
    assert!(msg.contains("Aucune voix française"), "{msg}");
}

// --- aperçu ------------------------------------------------------------------------------------

#[test]
fn l_apercu_joue_le_wav_avec_la_voix_choisie() {
    let synth = Simulated::french().shared();
    let generation = AtomicU64::new(0);
    let mut played = None;
    let info = preview_with(
        &generation,
        &synth,
        "Bonjour",
        None,
        no_op,
        |wav, current| {
            assert!(current(), "aucun aperçu plus récent");
            played = Some(wav);
            Ok(true)
        },
    )
    .unwrap();
    assert!(info.played);
    assert_eq!(info.duration_ms, 350);
    assert!(Wav::parse(played.unwrap()).is_ok());
}

#[test]
fn l_apercu_sans_voix_ou_sans_service_echoue_sans_rien_jouer() {
    let generation = AtomicU64::new(0);
    let never = |_: Vec<u8>, _: &dyn Fn() -> bool| -> Result<bool, SpeechError> {
        panic!("rien ne doit être joué")
    };
    let none = Simulated::none().shared();
    assert_eq!(
        preview_with(&generation, &none, "Bonjour", None, no_op, never).unwrap_err(),
        SpeechError::NoVoice
    );
    let sys: Arc<dyn Synthesizer> = Arc::new(NoSpeech);
    assert_eq!(
        preview_with(&generation, &sys, "Bonjour", None, no_op, never).unwrap_err(),
        SpeechError::Unavailable
    );
    let french = Simulated::french().shared();
    assert_eq!(
        preview_with(&generation, &french, "Bonjour", Some("autre"), no_op, never).unwrap_err(),
        SpeechError::VoiceGone
    );
    assert_eq!(
        preview_with(&generation, &french, "🎧", None, no_op, never).unwrap_err(),
        SpeechError::NothingToSay
    );
}

/// Moteur dont la synthèse attend un signal : l'ordre des événements ne dépend d'aucune durée.
struct Gated {
    started: std::sync::Mutex<mpsc::Sender<()>>,
    release: std::sync::Mutex<mpsc::Receiver<()>>,
}

impl Synthesizer for Gated {
    fn voices(&self) -> Result<Vec<Voice>, SpeechError> {
        Ok(vec![voice("fr-hortense", "Microsoft Hortense", "fr-FR")])
    }
    fn default_voice_id(&self) -> Option<String> {
        Some("fr-hortense".into())
    }
    fn synthesize(&self, _: &str, _: &str) -> Result<Vec<u8>, SpeechError> {
        self.started.lock().unwrap().send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(sine_wav(0.5))
    }
}

#[test]
fn un_apercu_depasse_par_le_suivant_n_est_pas_joue() {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let synth: Arc<dyn Synthesizer> = Arc::new(Gated {
        started: std::sync::Mutex::new(started_tx),
        release: std::sync::Mutex::new(release_rx),
    });
    let generation = Arc::new(AtomicU64::new(0));
    let first = {
        let (synth, generation) = (Arc::clone(&synth), Arc::clone(&generation));
        std::thread::spawn(move || {
            preview_with(&generation, &synth, "Premier", None, no_op, |_, _| {
                panic!("l'aperçu dépassé ne doit pas être joué")
            })
        })
    };
    // La synthèse du premier est en cours : un second aperçu (ou un arrêt) prend la main, puis
    // la synthèse est libérée.
    started_rx.recv().unwrap();
    generation.fetch_add(1, Ordering::SeqCst);
    release_tx.send(()).unwrap();
    let info = first.join().unwrap().unwrap();
    assert_eq!(
        info,
        PreviewInfo {
            played: false,
            duration_ms: 0
        }
    );
}

#[test]
fn un_arret_arrive_entre_la_synthese_et_le_demarrage_du_son_l_emporte() {
    // `play` refait le test de génération (sous le verrou de lecture) : s'il échoue, rien n'est
    // joué et l'aperçu est signalé comme non joué.
    let synth = Simulated::french().shared();
    let generation = AtomicU64::new(0);
    let info = preview_with(&generation, &synth, "Bonjour", None, no_op, |_, _| {
        generation.fetch_add(1, Ordering::SeqCst);
        Ok(false)
    })
    .unwrap();
    assert_eq!(
        info,
        PreviewInfo {
            played: false,
            duration_ms: 0
        }
    );
}

#[test]
fn l_apercu_coupe_la_lecture_en_cours_avant_de_synthetiser() {
    let synth = Simulated::french().shared();
    let generation = AtomicU64::new(0);
    let mut order = Vec::new();
    let order_cell = std::cell::RefCell::new(&mut order);
    preview_with(
        &generation,
        &synth,
        "Bonjour",
        None,
        || order_cell.borrow_mut().push("stop"),
        |_, _| {
            order_cell.borrow_mut().push("play");
            Ok(true)
        },
    )
    .unwrap();
    assert_eq!(order, ["stop", "play"]);
}

// --- MP3 ---------------------------------------------------------------------------------------

#[test]
fn les_arguments_ffmpeg_sont_fixes_mono_et_prefixes() {
    let args: Vec<String> = mp3_args(Path::new("in.wav"), Path::new("-out.mp3"))
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let joined = args.join(" ");
    assert!(joined.contains("-nostdin"));
    assert!(joined.contains("-protocol_whitelist file"));
    assert!(joined.contains("-i file:in.wav"));
    assert!(joined.contains("-c:a libmp3lame -ar 44100 -ac 1 -b:a 64k"));
    assert!(joined.contains("-map_metadata -1"));
    // Sortie préfixée : un nom commençant par « - » n'est jamais une option.
    assert_eq!(args.last().map(String::as_str), Some("file:-out.mp3"));
    assert_eq!(args[args.len() - 2], "mp3");
}

fn leftovers(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn l_annulation_avant_ffmpeg_ne_laisse_aucun_fichier() {
    let dir = tempfile::tempdir().unwrap();
    let ff = Ffmpeg::in_dir(dir.path().join("absent"));
    let wav = Wav::parse(sine_wav(0.5)).unwrap();
    let (tx, mut rx) = watch::channel(false);
    tx.send(true).unwrap();
    let err = wav_to_mp3(&ff, &wav, &dir.path().join("t001.mp3"), &mut rx)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Cancelled), "{err:?}");
    assert!(
        leftovers(dir.path()).is_empty(),
        "{:?}",
        leftovers(dir.path())
    );
}

#[tokio::test]
async fn un_ffmpeg_absent_ne_laisse_aucun_fichier() {
    let dir = tempfile::tempdir().unwrap();
    let ff = Ffmpeg::in_dir(dir.path().join("absent"));
    let wav = Wav::parse(sine_wav(0.5)).unwrap();
    let (_tx, mut rx) = watch::channel(false);
    let output = dir.path().join("t001.mp3");
    assert!(wav_to_mp3(&ff, &wav, &output, &mut rx).await.is_err());
    assert!(
        leftovers(dir.path()).is_empty(),
        "{:?}",
        leftovers(dir.path())
    );
}

#[tokio::test]
async fn un_dossier_de_sortie_absent_est_une_erreur_d_ecriture() {
    let dir = tempfile::tempdir().unwrap();
    let ff = Ffmpeg::in_dir(dir.path());
    let wav = Wav::parse(sine_wav(0.5)).unwrap();
    let (_tx, mut rx) = watch::channel(false);
    let output = dir.path().join("absent").join("t001.mp3");
    let err = wav_to_mp3(&ff, &wav, &output, &mut rx).await.unwrap_err();
    assert!(matches!(err, AppError::Io(_)), "{err:?}");
}

// Fin de conversion (`finish_mp3`) avec les sorties qu'un ffmpeg défaillant peut rendre : le
// processus lui-même est couvert par le vrai ffmpeg ci-dessous (ignoré s'il n'est pas construit).

fn ffmpeg_output(code: Option<i32>) -> crate::ffmpeg::Output {
    crate::ffmpeg::Output {
        code,
        stdout: Vec::new(),
        stderr: String::new(),
    }
}

/// Dossier de sortie et temporaires, avec un MP3 de `mp3` octets déjà « produit ».
fn finished_scratch(dir: &Path, mp3: &[u8]) -> (Scratch, PathBuf) {
    let output = dir.join("t001.mp3");
    let scratch = Scratch::beside(&output);
    std::fs::write(&scratch.wav, b"wav").unwrap();
    std::fs::write(&scratch.mp3, mp3).unwrap();
    (scratch, output)
}

#[tokio::test]
async fn une_sortie_non_nulle_de_ffmpeg_est_un_echec_sans_fichier() {
    let dir = tempfile::tempdir().unwrap();
    let (scratch, output) = finished_scratch(dir.path(), b"mp3");
    let (_tx, rx) = watch::channel(false);
    for code in [Some(1), None] {
        let err = finish_mp3(&ffmpeg_output(code), &scratch, &output, &rx)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), "tts_failed", "{code:?}");
        assert!(!output.exists());
    }
    drop(scratch);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn un_mp3_vide_est_un_echec_sans_fichier() {
    let dir = tempfile::tempdir().unwrap();
    let (scratch, output) = finished_scratch(dir.path(), b"");
    let (_tx, rx) = watch::channel(false);
    let err = finish_mp3(&ffmpeg_output(Some(0)), &scratch, &output, &rx)
        .await
        .unwrap_err();
    assert_eq!(err.kind(), "tts_failed");
    assert!(!output.exists());
    drop(scratch);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn une_annulation_pendant_ffmpeg_n_installe_rien() {
    let dir = tempfile::tempdir().unwrap();
    let (scratch, output) = finished_scratch(dir.path(), b"mp3");
    let (tx, rx) = watch::channel(false);
    tx.send(true).unwrap();
    let err = finish_mp3(&ffmpeg_output(Some(0)), &scratch, &output, &rx)
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Cancelled), "{err:?}");
    assert!(!output.exists());
    drop(scratch);
    assert!(leftovers(dir.path()).is_empty());
}

#[tokio::test]
async fn un_mp3_produit_est_renomme_et_ne_laisse_aucun_temporaire() {
    let dir = tempfile::tempdir().unwrap();
    let (scratch, output) = finished_scratch(dir.path(), b"mp3 factice");
    let (_tx, rx) = watch::channel(false);
    let size = finish_mp3(&ffmpeg_output(Some(0)), &scratch, &output, &rx)
        .await
        .unwrap();
    assert_eq!(size, 11);
    assert_eq!(std::fs::read(&output).unwrap(), b"mp3 factice");
    drop(scratch);
    assert_eq!(leftovers(dir.path()), ["t001.mp3"]);
}

/// ffmpeg embarqué (`npm run build:ffmpeg`), sinon `None` (test ignoré) ; exigé si
/// `FLAM_REQUIRE_FFMPEG` est défini.
fn real_ffmpeg() -> Option<Ffmpeg> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("build")
        .join("ffmpeg");
    if !dir.join(Tool::Ffmpeg.file_name()).is_file()
        || !dir.join(Tool::Ffprobe.file_name()).is_file()
        || !dir.join(MANIFEST_FILE).is_file()
    {
        assert!(
            std::env::var_os("FLAM_REQUIRE_FFMPEG").is_none(),
            "ffmpeg embarqué non construit alors que FLAM_REQUIRE_FFMPEG est défini"
        );
        eprintln!("ffmpeg embarqué non construit : test ignoré");
        return None;
    }
    Some(Ffmpeg::in_dir(dir))
}

#[tokio::test]
async fn le_vrai_ffmpeg_produit_un_mp3_mono_sans_temporaire() {
    let Some(ff) = real_ffmpeg() else { return };
    let dir = tempfile::tempdir().unwrap();
    let wav = Wav::parse(sine_wav(2.0)).unwrap();
    let output = dir.path().join("t001.mp3");
    let (_tx, mut rx) = watch::channel(false);
    let size = wav_to_mp3(&ff, &wav, &output, &mut rx).await.unwrap();
    assert_eq!(std::fs::metadata(&output).unwrap().len(), size);
    // 2 s à 64 kbit/s : environ 16 Kio.
    assert!((8_000..40_000).contains(&size), "{size}");
    assert_eq!(leftovers(dir.path()), ["t001.mp3"]);
    // Sonde : mono, 44 100 Hz.
    let (_tx2, mut rx2) = watch::channel(false);
    let info = crate::podcast::audio::probe(&ff, &output, &mut rx2)
        .await
        .unwrap();
    assert_eq!(info.sample_rate, Some(44_100));
    assert_eq!(info.channels, Some(1));
    assert_eq!(info.codec, "mp3");
}

// --- test réel, manuel -------------------------------------------------------------------------

/// Synthèse réelle avec les voix du poste : `cargo test -p flam-launcher speech::tests::voix_reelle
/// -- --ignored --nocapture`. Sans voix française, le test le dit et réussit. Variables :
/// `FLAM_TTS_OUT=<dossier>` écrit le WAV (à convertir à la main) ; `FLAM_TTS_PLAY=1` le joue.
#[test]
#[ignore = "voix du poste : test manuel (cargo test ... -- --ignored --nocapture)"]
#[cfg(windows)]
fn voix_reelle_du_poste() {
    let synth = system();
    let all = synth.voices().expect("liste des voix");
    eprintln!("voix installées ({}) :", all.len());
    for v in &all {
        eprintln!("  {} | {} | {}", v.id, v.name, v.language);
    }
    let list = french_voices(synth.as_ref()).unwrap();
    eprintln!(
        "voix françaises locales : {:?}, défaut : {:?}",
        list.voices.iter().map(|v| &v.name).collect::<Vec<_>>(),
        list.default_id
    );
    if list.voices.is_empty() {
        eprintln!("aucune voix française : synthèse non testée");
        return;
    }
    for v in &list.voices {
        let started = std::time::Instant::now();
        let wav = synthesize_wav(&synth, "Bonjour", &v.id, Duration::from_secs(30)).unwrap();
        eprintln!(
            "{} : {} Hz, {} canal(aux), {:.2} s d'audio, {} octets, synthèse en {:?}",
            v.name,
            wav.sample_rate,
            wav.channels,
            wav.duration_secs,
            wav.bytes.len(),
            started.elapsed()
        );
        assert!(wav.duration_secs > 0.2);
        if let Some(dir) = std::env::var_os("FLAM_TTS_OUT") {
            let name: String = v
                .name
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                .collect();
            std::fs::write(Path::new(&dir).join(format!("{name}.wav")), &wav.bytes).unwrap();
        }
        if std::env::var_os("FLAM_TTS_PLAY").is_some() {
            playback::play(wav.bytes, &|| true).unwrap();
            std::thread::sleep(Duration::from_millis(
                (wav.duration_secs * 1000.0) as u64 + 300,
            ));
        }
    }
}
