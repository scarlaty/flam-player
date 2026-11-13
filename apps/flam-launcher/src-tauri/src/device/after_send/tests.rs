//! Retrait de « Mes histoires » après l'envoi (#609) : vraie bibliothèque dans un dossier
//! temporaire, envoi réel (`send_from_library_with`) sur un faux appareil en firmware 2.x, pannes
//! injectées (`FaultyFs`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use super::*;
use crate::device::detect::MDF_FILE;
use crate::device::guard::tests::test_ctx;
use crate::device::transfer::tests::{
    device, listed, mdf, plain_pk_with, snapshot, write, FaultyFs, NEW, OTHER,
};
use crate::device::transfer::{send_from_library_with, DeviceFs, WORK_SUBDIR};
use crate::library::tests::story;

/// Histoire de la bibliothèque (son `uuid.bin` désigne `NEW` sur la Flam).
const A: &str = "histoire-a";
/// Seconde histoire de la bibliothèque.
const B: &str = "histoire-b";

struct Env {
    tmp: tempfile::TempDir,
    cache: PathBuf,
    root: PathBuf,
    registry: SentRegistry,
}

impl Env {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let cache = tmp.path().join("cache");
        let root = device(tmp.path(), false);
        write(&root.join(MDF_FILE), &mdf("2.4.2"));
        Self {
            tmp,
            cache,
            root,
            registry: SentRegistry::new(),
        }
    }

    fn library(&self) -> Library {
        Library::new(&self.cache)
    }

    /// Installe (ou réinstalle) `uuid` dans la bibliothèque, à la date `now`.
    fn install(&self, uuid: &str, now: u64, sound: &[u8]) {
        let built = self.tmp.path().join("construit");
        std::fs::create_dir_all(&built).unwrap();
        let pk = plain_pk_with(&built, sound);
        let staged = self.library().staging_dir().unwrap().join("t.plain.pk");
        std::fs::rename(pk, &staged).unwrap();
        self.library()
            .install(&staged, &story(uuid, Some(1)), None, now)
            .unwrap();
    }

    fn installed(&self, uuid: &str) -> bool {
        self.library().playable(uuid).is_ok()
    }

    fn library_file(&self, uuid: &str) -> PathBuf {
        self.library().playable(uuid).unwrap().0
    }

    /// Retrait réel par la bibliothèque, vérification comprise (le créneau du player et la purge
    /// de `library::remove_story` sont hors de ce test).
    fn remover<'a>(&'a self, uuid: &'a str) -> impl FnOnce(Check<'_>) -> AppResult<()> + 'a {
        move |check| {
            self.library()
                .remove_checked(uuid, Duration::from_secs(5), check, || Ok(()))
        }
    }

    /// Envoi de `uuid` par `fs`, comme `device_send_story` ; `before_send` s'exécute entre la
    /// prise d'empreinte et l'envoi, `after_send` juste après l'envoi.
    #[allow(clippy::too_many_arguments)]
    fn send_with_hooks(
        &self,
        uuid: &str,
        fs: &dyn DeviceFs,
        auto: bool,
        installing: bool,
        cancel: &AtomicBool,
        cancel_on_progress: bool,
        before_send: impl FnOnce(),
        after_send: impl FnOnce(),
    ) -> AppResult<TransferReport> {
        send_then_settle(
            &self.registry,
            &self.cache,
            &self.root,
            uuid,
            auto,
            &|| installing,
            |plain_pk| {
                before_send();
                let result = send_from_library_with(
                    fs,
                    &self.root,
                    plain_pk,
                    &self.cache.join(WORK_SUBDIR),
                    None,
                    None,
                    &test_ctx(),
                    cancel,
                    &mut |_| {
                        if cancel_on_progress {
                            cancel.store(true, Ordering::Relaxed);
                        }
                    },
                );
                after_send();
                Ok(result?)
            },
            self.remover(uuid),
        )
    }

    fn send(&self, uuid: &str, fs: &dyn DeviceFs, auto: bool) -> AppResult<TransferReport> {
        self.send_with_hooks(
            uuid,
            fs,
            auto,
            false,
            &AtomicBool::new(false),
            false,
            || {},
            || {},
        )
    }
}

fn on_device(root: &Path) -> bool {
    root.join("str").join(NEW).join("info").is_file() && listed(root).contains(&id(NEW))
}

fn id(s: &str) -> crate::device::index::StoryId {
    crate::device::index::StoryId::parse(s).unwrap()
}

#[test]
#[cfg_attr(not(windows), ignore = "retrait automatique : Windows seulement")]
fn retrait_automatique_apres_un_envoi_reussi() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let file = env.library_file(A);

    let report = env.send(A, &FaultyFs::new(), true).unwrap();
    assert_eq!(report.library_copy, Some(LibraryCopy::Removed));
    assert!(on_device(&env.root), "l'histoire est sur la Flam, listée");
    assert!(!env.installed(A), "retirée de Mes histoires");
    assert!(!file.exists(), "fichier de la bibliothèque supprimé");
    assert!(!env.registry.knows(A), "empreinte consommée");
    // L'autre histoire de la Flam est intacte.
    assert_eq!(listed(&env.root), vec![id(OTHER), id(NEW)]);
}

#[test]
fn reglage_desactive_rien_n_est_retire_et_le_retrait_manuel_reste_possible() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);

    let report = env.send(A, &FaultyFs::new(), false).unwrap();
    assert_eq!(
        report.library_copy,
        Some(LibraryCopy::Kept { unconfirmed: false })
    );
    assert!(on_device(&env.root));
    assert!(env.installed(A), "gardée");
    assert!(env.registry.knows(A));

    // « Retirer du PC » : même vérification, puis retrait.
    remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap();
    assert!(!env.installed(A));
    assert!(on_device(&env.root), "toujours sur la Flam");
    // Second clic : plus rien d'enregistré.
    let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
    assert_eq!(err.kind(), "not_found");
}

#[test]
fn aucun_retrait_ni_enregistrement_apres_un_echec_une_annulation_ou_un_envoi_partiel() {
    // Coupure (appareil débranché) à plusieurs moments de la copie et de la relecture.
    for crash_at in [0, 3, 10, 25] {
        let env = Env::new();
        env.install(A, 10, &[1u8; 3000]);
        let library_before = snapshot(&env.cache.join("library"));
        let err = env
            .send(A, &FaultyFs::crash_at(crash_at), true)
            .unwrap_err();
        assert_ne!(err.kind(), "cancelled", "{crash_at}");
        assert!(env.installed(A), "{crash_at} : gardée après un échec");
        assert_eq!(
            snapshot(&env.cache.join("library")),
            library_before,
            "{crash_at} : bibliothèque intacte"
        );
        assert!(!env.registry.knows(A), "{crash_at}");
        assert!(!on_device(&env.root), "{crash_at} : rien d'installé");
    }

    // Annulation avant le début, puis pendant la copie.
    for during in [false, true] {
        let env = Env::new();
        env.install(A, 10, &[1u8; 3000]);
        let cancel = AtomicBool::new(!during);
        let err = env
            .send_with_hooks(
                A,
                &FaultyFs::new(),
                true,
                false,
                &cancel,
                during,
                || {},
                || {},
            )
            .unwrap_err();
        assert_eq!(err.kind(), "cancelled", "{during}");
        assert!(env.installed(A), "{during} : gardée après une annulation");
        assert!(!env.registry.knows(A));
    }

    // Copie altérée : la relecture la détecte, rien n'est installé ni retiré.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let mut corrupt = FaultyFs::new();
    corrupt.corrupt = Some("main.lsf");
    assert!(env.send(A, &corrupt, true).is_err());
    assert!(env.installed(A));
    assert!(!env.registry.knows(A));
}

#[test]
#[cfg_attr(not(windows), ignore = "retrait automatique : Windows seulement")]
fn deux_histoires_chacune_jugee_a_part() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    env.install(B, 11, &[2u8; 3000]);

    // A échoue (coupure), puis B réussit : seule B est retirée.
    assert!(env.send(A, &FaultyFs::crash_at(5), true).is_err());
    let report = env.send(B, &FaultyFs::new(), true).unwrap();
    assert_eq!(report.library_copy, Some(LibraryCopy::Removed));
    assert!(env.installed(A), "l'échec de A ne la retire pas");
    assert!(!env.installed(B));

    // A désigne maintenant une histoire déjà sur la Flam : refus « déjà présente », gardée.
    let err = env.send(A, &FaultyFs::new(), true).unwrap_err();
    assert!(err.to_string().contains("déjà sur la Flam"), "{err}");
    assert!(env.installed(A));
    assert!(!env.registry.knows(A));
}

#[test]
fn histoire_reinstallee_pendant_l_envoi_jamais_retiree() {
    // Réinstallée avant la lecture de son fichier, ou juste après l'envoi (date d'installation
    // différente), ou au même instant avec un autre contenu (taille du fichier différente).
    for (when, now, sound) in [
        ("avant", 20, vec![3u8; 3000]),
        ("après", 20, vec![3u8; 3000]),
        ("même date", 10, vec![3u8; 4000]),
    ] {
        for auto in [true, false] {
            let env = Env::new();
            env.install(A, 10, &[1u8; 3000]);
            let reinstall = || env.install(A, now, &sound);
            type Hook<'a> = Box<dyn FnOnce() + 'a>;
            let (before, after): (Hook<'_>, Hook<'_>) = if when == "avant" {
                (Box::new(reinstall), Box::new(|| {}))
            } else {
                (Box::new(|| {}), Box::new(reinstall))
            };
            let report = env
                .send_with_hooks(
                    A,
                    &FaultyFs::new(),
                    auto,
                    false,
                    &AtomicBool::new(false),
                    false,
                    before,
                    after,
                )
                .unwrap();
            let expected = auto.then(|| LibraryCopy::RemoveFailed {
                message: CHANGED_MESSAGE.into(),
            });
            assert_eq!(report.library_copy, expected, "{when} {auto}");
            assert!(env.installed(A), "{when} {auto} : nouvelle version gardée");
            assert!(!env.registry.knows(A), "{when} {auto}");
        }
    }

    // Envoi réussi et gardé, puis réinstallée avant le clic sur « Retirer du PC » : refus.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let report = env.send(A, &FaultyFs::new(), false).unwrap();
    assert_eq!(
        report.library_copy,
        Some(LibraryCopy::Kept { unconfirmed: false })
    );
    env.install(A, 30, &[4u8; 3000]);
    let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
    assert_eq!(err.to_string(), CHANGED_MESSAGE);
    assert!(env.installed(A));
    assert!(!env.registry.knows(A), "empreinte périmée oubliée");
}

#[test]
#[cfg_attr(not(windows), ignore = "retrait automatique : Windows seulement")]
fn echec_du_retrait_rapporte_sans_faire_echouer_l_envoi() {
    // Installation de la même histoire en cours : refus vérifié sous le verrou.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let report = env
        .send_with_hooks(
            A,
            &FaultyFs::new(),
            true,
            true,
            &AtomicBool::new(false),
            false,
            || {},
            || {},
        )
        .unwrap();
    assert_eq!(
        report.library_copy,
        Some(LibraryCopy::RemoveFailed {
            message: INSTALLING_MESSAGE.into()
        })
    );
    assert!(on_device(&env.root), "envoi réussi");
    assert!(env.installed(A));
    // L'empreinte reste : « Retirer du PC » pourra être retenté.
    assert!(env.registry.knows(A));

    // Refus du chemin de suppression (lecture en cours) : rapporté, rien n'est retiré, et un
    // nouvel essai réussit une fois la lecture finie.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let report = send_then_settle(
        &env.registry,
        &env.cache,
        &env.root,
        A,
        true,
        &|| false,
        |plain_pk| {
            Ok(send_from_library_with(
                &FaultyFs::new(),
                &env.root,
                plain_pk,
                &env.cache.join(WORK_SUBDIR),
                None,
                None,
                &test_ctx(),
                &AtomicBool::new(false),
                &mut |_| {},
            )?)
        },
        |_| Err(AppError::PlayerBusy("en cours de lecture".into())),
    )
    .unwrap();
    assert_eq!(
        report.library_copy,
        Some(LibraryCopy::RemoveFailed {
            message: "en cours de lecture".into()
        })
    );
    assert!(env.installed(A));
    remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap();
    assert!(!env.installed(A));
}

#[test]
fn relecture_carrier_illisible_ou_empreinte_absente_rien_n_est_propose() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let stamp = SentStamp::capture(&env.cache, A);
    assert!(stamp.is_some());
    let report = TransferReport {
        id: NEW.into(),
        bytes: 1,
        files: 1,
        replaced: false,
        index_updated: true,
        cache_removed: true,
        carrier_readback: Some(false),
        carrier_stored: Some(false),
        recovery_failures: Vec::new(),
        library_copy: None,
        dir_synced: true,
        verified_uncached: true,
        installed_files: vec![("info".into(), 1)],
    };
    let removed = std::cell::Cell::new(false);
    let copy = settle(
        &env.registry,
        &env.cache,
        &env.root,
        A,
        stamp,
        &report,
        true,
        true,
        &|| false,
        |_| {
            removed.set(true);
            Ok(())
        },
    );
    assert_eq!(copy, None);
    assert!(!removed.get());
    assert!(env.installed(A));
    assert!(!env.registry.knows(A));

    // Histoire absente de la bibliothèque : pas d'empreinte, rien à retirer.
    assert_eq!(SentStamp::capture(&env.cache, "inconnue"), None);
    let copy = settle(
        &env.registry,
        &env.cache,
        &env.root,
        "inconnue",
        None,
        &TransferReport {
            carrier_readback: None,
            ..report
        },
        true,
        true,
        &|| false,
        |_| {
            removed.set(true);
            Ok(())
        },
    );
    assert_eq!(copy, None);
    assert!(!removed.get());
}

#[test]
#[cfg_attr(not(windows), ignore = "retrait automatique : Windows seulement")]
fn le_retrait_ne_touche_que_l_entree_envoyee() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    env.install(B, 11, &[2u8; 3000]);
    // Fichier source de l'utilisateur, hors de la bibliothèque.
    let source = env.tmp.path().join("source").join("histoire.plain.pk");
    write(&source, b"source");
    let b_file = env.library_file(B);

    env.send(A, &FaultyFs::new(), true).unwrap();
    assert!(!env.installed(A));
    assert!(env.installed(B), "l'autre histoire reste");
    assert!(b_file.is_file());
    assert_eq!(std::fs::read(&source).unwrap(), b"source");
}

/// Envoi gardé (réglage désactivé) : « Retirer du PC » est enregistré.
fn kept(env: &Env) {
    let report = env.send(A, &FaultyFs::new(), false).unwrap();
    assert_eq!(
        report.library_copy,
        Some(LibraryCopy::Kept { unconfirmed: false })
    );
    assert!(env.registry.knows(A));
}

#[test]
fn retirer_du_pc_refuse_si_l_histoire_a_quitte_la_flam() {
    // Retirée de la Flam hors du launcher (dossier et index) : refus, rien n'est retiré.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    kept(&env);
    std::fs::remove_dir_all(env.root.join("str").join(NEW)).unwrap();
    write(
        &env.root.join("etc/library/list"),
        &crate::device::index::serialize(&[id(OTHER)]),
    );
    let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
    assert_eq!(err.to_string(), NOT_ON_DEVICE_MESSAGE);
    assert!(env.installed(A), "toujours dans Mes histoires");
    assert!(!env.registry.knows(A), "entrée oubliée");

    // Encore listée, mais un fichier manque ou n'a plus sa taille : refus.
    for damage in ["manque", "taille"] {
        let env = Env::new();
        env.install(A, 10, &[1u8; 3000]);
        kept(&env);
        let sound = env.root.join("str").join(NEW).join("sounds/a.mp3");
        if damage == "manque" {
            std::fs::remove_file(&sound).unwrap();
        } else {
            write(&sound, b"court");
        }
        let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
        assert_eq!(err.to_string(), NOT_ON_DEVICE_MESSAGE, "{damage}");
        assert!(env.installed(A), "{damage}");
    }

    // Retirée par « Ma Flam » (`device_remove_story` oublie l'histoire de la Flam) : plus rien
    // d'enregistré, quelle que soit la forme de l'identifiant.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    kept(&env);
    env.registry
        .forget_story(&NEW.replace('-', "").to_uppercase());
    assert!(!env.registry.knows(A));
    let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
    assert_eq!(err.kind(), "not_found");
    assert!(env.installed(A));
}

#[test]
fn retirer_du_pc_refuse_flam_debranchee_ou_autre_flam() {
    // Débranchée : plus de `.mdf` à cette racine.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    kept(&env);
    std::fs::remove_file(env.root.join(MDF_FILE)).unwrap();
    let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
    assert_eq!(err.to_string(), OTHER_DEVICE_MESSAGE);
    assert!(env.installed(A));
    assert!(!env.registry.knows(A));

    // Autre Flam à la même racine (autre `.mdf`), ou Flam à une autre racine : refus, même si
    // elle porte la même histoire.
    for other_root in [false, true] {
        let env = Env::new();
        env.install(A, 10, &[1u8; 3000]);
        kept(&env);
        let root = if other_root {
            let copy = env.tmp.path().join("autre");
            for (rel, data) in snapshot(&env.root) {
                if let Some(data) = data {
                    write(&copy.join("flam").join(&rel), &data);
                }
            }
            copy.join("flam")
        } else {
            write(&env.root.join(MDF_FILE), &mdf("2.4.3"));
            env.root.clone()
        };
        let err = remove_sent(&env.registry, &root, A, &|| false, env.remover(A)).unwrap_err();
        assert_eq!(err.to_string(), OTHER_DEVICE_MESSAGE, "{other_root}");
        assert!(env.installed(A), "{other_root}");
        assert!(!env.registry.knows(A), "{other_root}");
    }
}

#[test]
fn un_nouvel_envoi_oublie_l_ancien_meme_s_il_echoue() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    kept(&env);
    // Renvoi (remplacement refusé faute de `replace` : « déjà sur la Flam ») : en échec, l'ancien
    // envoi n'est plus utilisable.
    assert!(env.send(A, &FaultyFs::new(), false).is_err());
    assert!(!env.registry.knows(A));
    let err = remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap_err();
    assert_eq!(err.kind(), "not_found");
    assert!(env.installed(A));

    // Renvoi en échec (coupure) d'une histoire gardée.
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    kept(&env);
    std::fs::remove_dir_all(env.root.join("str").join(NEW)).unwrap();
    write(
        &env.root.join("etc/library/list"),
        &crate::device::index::serialize(&[id(OTHER)]),
    );
    assert!(env.send(A, &FaultyFs::crash_at(8), false).is_err());
    assert!(!env.registry.knows(A));
    assert!(env.installed(A));
}

#[test]
fn ecriture_non_confirmee_pas_de_retrait_automatique() {
    for (sync_fails, cached) in [(true, false), (false, true), (true, true)] {
        let env = Env::new();
        env.install(A, 10, &[1u8; 3000]);
        let mut fs = FaultyFs::new();
        fs.sync_dir_fails = sync_fails;
        fs.cached_verify = cached;
        let report = env.send(A, &fs, true).unwrap();
        assert_eq!(
            report.library_copy,
            Some(LibraryCopy::Kept { unconfirmed: true }),
            "{sync_fails} {cached}"
        );
        assert!(!report.dir_synced || !report.verified_uncached);
        assert!(on_device(&env.root));
        assert!(env.installed(A), "gardée");
        // « Retirer du PC » reste possible, après revérification sur la Flam.
        remove_sent(&env.registry, &env.root, A, &|| false, env.remover(A)).unwrap();
        assert!(!env.installed(A));
    }
}

#[test]
#[cfg_attr(not(windows), ignore = "retrait automatique : Windows seulement")]
fn ecriture_confirmee_retrait_automatique() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let report = env.send(A, &FaultyFs::new(), true).unwrap();
    assert!(report.dir_synced && report.verified_uncached);
    assert_eq!(report.library_copy, Some(LibraryCopy::Removed));
    assert!(!env.installed(A));
}

#[test]
fn jamais_de_retrait_automatique_hors_windows() {
    let env = Env::new();
    env.install(A, 10, &[1u8; 3000]);
    let report = env.send(A, &FaultyFs::new(), false).unwrap();
    let confirmed = TransferReport {
        dir_synced: true,
        verified_uncached: true,
        ..report.clone()
    };
    assert!(durable_on(&confirmed, true));
    assert!(!durable_on(&confirmed, false), "hors Windows");
    for (dir_synced, verified_uncached) in [(false, true), (true, false)] {
        let partial = TransferReport {
            dir_synced,
            verified_uncached,
            ..report.clone()
        };
        assert!(!durable_on(&partial, true));
    }
    // Rapport hors Windows (`dir_synced` toujours faux) : jamais de retrait automatique.
    let removed = std::cell::Cell::new(false);
    let copy = settle(
        &env.registry,
        &env.cache,
        &env.root,
        A,
        SentStamp::capture(&env.cache, A),
        &report,
        true,
        durable_on(&confirmed, false),
        &|| false,
        |_| {
            removed.set(true);
            Ok(())
        },
    );
    assert_eq!(copy, Some(LibraryCopy::Kept { unconfirmed: true }));
    assert!(!removed.get());
    assert!(env.installed(A));
}
