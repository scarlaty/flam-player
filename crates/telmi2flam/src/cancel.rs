//! Jeton d'annulation du binaire (#132, PORT-RUST.md §2.2 « Annulation sans Python ») : un
//! drapeau global, levé par la fermeture de stdin (`--cancel-on-stdin-eof`, protocole du
//! launcher), par `Ctrl+C` et `CTRL_BREAK` sous Windows, par `SIGINT` et `SIGTERM` ailleurs
//! (`_install_cancel_handlers` de Python), et lu par la conversion ([`is_cancelled`], passé
//! comme `cancelled` à [`crate::convert::convert`]).
//!
//! Une fois levé, le drapeau ne retombe pas : la conversion s'arrête au prochain point de
//! contrôle (entre deux assets, à chaque ligne d'image, avant chaque entrée écrite et avant les
//! publications), supprime ses `.part` et rend [`crate::error::ConvError::cancelled`].

use std::io::{ErrorKind, Read};
use std::sync::atomic::{AtomicBool, Ordering};

static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Vrai si l'annulation a été demandée.
pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// Demande l'annulation. Async-signal-safe (un `store` atomique).
pub fn request() {
    CANCELLED.store(true, Ordering::SeqCst);
}

/// `--cancel-on-stdin-eof` : un fil lit stdin jusqu'à la fin (ou une erreur de lecture) puis
/// demande l'annulation ; les octets lus sont ignorés. Le fil n'empêche pas le processus de se
/// terminer : sortir de `main` (ou `process::exit`) l'arrête même bloqué dans la lecture.
///
/// Un stdin absent ou invalide se lit comme une fin immédiate : l'option est réservée à un
/// appelant qui garde stdin ouvert en tube (le launcher) et le ferme pour annuler.
pub fn cancel_on_stdin_eof() -> std::io::Result<()> {
    std::thread::Builder::new()
        .name("stdin-eof".to_owned())
        .stack_size(64 << 10)
        .spawn(|| {
            let mut stdin = std::io::stdin();
            let mut buf = [0u8; 256];
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            request();
        })
        .map(drop)
}

/// Installe les gestionnaires de signaux console. Faux si l'installation a échoué (le
/// processus garde alors le comportement par défaut : arrêt sans nettoyage).
pub fn install_signal_handlers() -> bool {
    sys::install()
}

#[cfg(windows)]
mod sys {
    type HandlerRoutine = unsafe extern "system" fn(u32) -> i32;

    #[link(name = "kernel32")]
    extern "system" {
        fn SetConsoleCtrlHandler(handler: Option<HandlerRoutine>, add: i32) -> i32;
    }

    const CTRL_C_EVENT: u32 = 0;
    const CTRL_BREAK_EVENT: u32 = 1;

    /// Appelé par Windows dans un fil créé pour l'occasion. `Ctrl+C` et `CTRL_BREAK` : jeton
    /// levé et événement traité (pas d'arrêt par défaut). Fermeture de la console, déconnexion
    /// et arrêt du système : comportement par défaut (Windows arrête le processus).
    unsafe extern "system" fn handler(ctrl: u32) -> i32 {
        match ctrl {
            CTRL_C_EVENT | CTRL_BREAK_EVENT => {
                super::request();
                1
            }
            _ => 0,
        }
    }

    pub fn install() -> bool {
        // SAFETY : `handler` est une fonction `'static` de la bonne signature.
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) != 0 }
    }
}

#[cfg(unix)]
mod sys {
    use std::os::raw::c_int;

    extern "C" {
        fn signal(sig: c_int, handler: usize) -> usize;
    }

    const SIGINT: c_int = 2;
    const SIGTERM: c_int = 15;
    /// `SIG_ERR` : `(void (*)(int)) -1`.
    const SIG_ERR: usize = usize::MAX;

    extern "C" fn handler(_sig: c_int) {
        super::request();
    }

    pub fn install() -> bool {
        let h = handler as extern "C" fn(c_int) as usize;
        // SAFETY : le gestionnaire n'exécute qu'un `store` atomique (async-signal-safe).
        unsafe { signal(SIGINT, h) != SIG_ERR && signal(SIGTERM, h) != SIG_ERR }
    }
}

#[cfg(not(any(windows, unix)))]
mod sys {
    pub fn install() -> bool {
        false
    }
}
