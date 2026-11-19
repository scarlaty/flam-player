use std::fs;

use super::*;

fn is_forbidden(result: Result<PathBuf, ScanError>) -> bool {
    matches!(result, Err(ScanError::Forbidden { .. }))
}

/// Dossier de test : `<base>/Profil utilisateur long` (profil) et `<base>/Systeme` (arbre).
struct Fixture {
    _dir: tempfile::TempDir,
    base: PathBuf,
    profile: PathBuf,
    system: PathBuf,
    roots: ForbiddenRoots,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let base = dir.path().join("base");
    let profile = base.join("Profil utilisateur long");
    let system = base.join("Systeme");
    fs::create_dir_all(profile.join("Histoires")).unwrap();
    fs::create_dir_all(system.join("Sous")).unwrap();
    let roots = ForbiddenRoots {
        profile: vec![profile.clone()],
        trees: vec![system.clone()],
    };
    Fixture {
        _dir: dir,
        base,
        profile,
        system,
        roots,
    }
}

#[test]
fn profil_parents_refuses_sous_dossiers_permis() {
    let f = fixture();
    assert!(is_forbidden(check_root(&f.profile, &f.roots)));
    assert!(is_forbidden(check_root(&f.base, &f.roots)));
    let upper = PathBuf::from(f.profile.to_string_lossy().to_uppercase());
    if cfg!(windows) {
        assert!(is_forbidden(check_root(&upper, &f.roots)));
    }
    assert!(check_root(&f.profile.join("Histoires"), &f.roots).is_ok());
}

#[test]
fn arbres_interdits_avec_leurs_sous_dossiers() {
    let f = fixture();
    assert!(is_forbidden(check_root(&f.system, &f.roots)));
    assert!(is_forbidden(check_root(&f.system.join("Sous"), &f.roots)));
    // Paramètre de l'appelant (dossier du launcher, lecteur de la Flam).
    let extra = f.roots.clone().with_trees(&[f.profile.join("Histoires")]);
    assert!(is_forbidden(check_root(
        &f.profile.join("Histoires"),
        &extra
    )));
}

#[test]
fn formes_refusees_sans_acces_au_disque() {
    let none = ForbiddenRoots::default();
    assert!(matches!(
        check_root(Path::new("relatif"), &none),
        Err(ScanError::NotAbsolute(_))
    ));
    if cfg!(windows) {
        assert!(matches!(
            check_root(Path::new(r"C:\a\..\b"), &none),
            Err(ScanError::NotAbsolute(_))
        ));
        for path in [
            r"C:\",
            r"\\serveur\partage",
            r"\\?\GLOBALROOT\Device\HarddiskVolume1\Users",
            r"\\.\C:\Users",
            r"\\?\UNC\serveur\partage\x",
            r"C:\Users\moi.",
            r"C:\Users\moi ",
            r"C:\Users\moi.\Documents",
        ] {
            assert!(is_forbidden(check_root(Path::new(path), &none)), "{path}");
        }
    } else {
        assert!(is_forbidden(check_root(Path::new("/"), &none)));
        assert!(is_forbidden(check_root(Path::new("/home/moi."), &none)));
    }
}

#[test]
fn point_final_refuse_meme_si_le_dossier_existe() {
    let f = fixture();
    let dotted = PathBuf::from(format!("{}.", f.profile.to_string_lossy()));
    assert!(is_forbidden(check_root(&dotted, &f.roots)));
}

#[cfg(windows)]
#[test]
fn prefixe_verbatim_resolu() {
    let f = fixture();
    let verbatim = PathBuf::from(format!(r"\\?\{}", f.profile.to_string_lossy()));
    assert!(is_forbidden(check_root(&verbatim, &f.roots)));
    let ok = PathBuf::from(format!(
        r"\\?\{}",
        f.profile.join("Histoires").to_string_lossy()
    ));
    let resolved = check_root(&ok, &f.roots).unwrap();
    assert!(
        !resolved.to_string_lossy().starts_with(r"\\?\"),
        "{resolved:?}"
    );
}

#[cfg(windows)]
fn junction(link: &Path, target: &Path) {
    // Jonction (sans privilège) ; séparateurs Windows pour `mklink`.
    let status = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link.to_string_lossy().replace('/', "\\"))
        .arg(target.to_string_lossy().replace('/', "\\"))
        .output()
        .unwrap();
    assert!(status.status.success(), "mklink /J a échoué");
}

#[cfg(windows)]
#[test]
fn jonction_vers_un_dossier_interdit_resolue() {
    let f = fixture();
    // Jonction vers le profil (comme `C:\Documents and Settings\<user>`).
    let alias = f.base.join("Alias du profil");
    junction(&alias, &f.profile);
    assert!(is_forbidden(check_root(&alias, &f.roots)));
    // Jonction sur un parent : la racine désigne en fait le dossier système.
    let parent = f.base.join("Alias de base");
    junction(&parent, &f.base);
    assert!(is_forbidden(check_root(&parent.join("Systeme"), &f.roots)));
    assert!(is_forbidden(check_root(
        &parent.join("Profil utilisateur long"),
        &f.roots
    )));
    assert!(check_root(
        &parent.join("Profil utilisateur long").join("Histoires"),
        &f.roots
    )
    .is_ok());
}

#[cfg(windows)]
#[test]
fn nom_court_8_3_resolu() {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    let f = fixture();
    let wide: Vec<u16> = f.profile.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0u16; 1024];
    // SAFETY : chaîne terminée par un zéro ; tampon de la taille annoncée.
    let len = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(
            wide.as_ptr(),
            buffer.as_mut_ptr(),
            buffer.len() as u32,
        )
    } as usize;
    let short = (len > 0 && len < buffer.len())
        .then(|| PathBuf::from(std::ffi::OsString::from_wide(&buffer[..len])));
    match short {
        Some(short) if short.file_name() != f.profile.file_name() => {
            assert!(is_forbidden(check_root(&short, &f.roots)), "{short:?}");
        }
        _ => eprintln!(
            "nom_court_8_3_resolu : pas de nom 8.3 sur ce volume, cas non vérifié ({:?})",
            f.profile
        ),
    }
}

#[test]
fn dossiers_du_systeme() {
    let roots = ForbiddenRoots::system();
    if let Some(home) = dirs::home_dir() {
        assert!(is_forbidden(check_root(&home, &roots)));
        if let Some(parent) = home.parent().filter(|p| p.parent().is_some()) {
            assert!(is_forbidden(check_root(parent, &roots)));
        }
    }
    if cfg!(windows) {
        if let Some(windir) = std::env::var_os("SystemRoot").map(PathBuf::from) {
            assert!(is_forbidden(check_root(&windir.join("System32"), &roots)));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles").map(PathBuf::from) {
            if let Some(child) = fs::read_dir(&program_files)
                .ok()
                .and_then(|mut d| d.find_map(|e| e.ok().filter(|e| e.path().is_dir())))
            {
                assert!(is_forbidden(check_root(&child.path(), &roots)));
            }
        }
    }
}

#[test]
fn racine_absente() {
    let f = fixture();
    assert!(matches!(
        check_root(&f.base.join("absent"), &f.roots),
        Err(ScanError::NotFound(_))
    ));
}
