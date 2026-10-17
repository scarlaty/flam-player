//! Fenêtre d'accueil du player incrusté (#201).
//!
//! Sous Windows, le launcher crée dans sa fenêtre principale une fenêtre enfant native
//! (`STATIC`, `WS_CHILD | WS_CLIPSIBLINGS`, au-dessus de la WebView2) et passe son HWND au
//! player (`--parent-hwnd`). Le player remplit la zone cliente de cette fenêtre et suit sa
//! taille ; c'est le launcher qui la place sur la zone de l'écran Lecture
//! ([`EmbedHost::layout`], rectangle en px CSS converti en px physiques par [`to_physical`]).
//!
//! Toutes les opérations Win32 ont lieu sur le **thread principal** (celui de la boucle de
//! messages qui possède la fenêtre du launcher), via `run_on_main_thread`. Seule la création
//! attend sa réponse (bornée) ; déplacement et destruction sont envoyés sans attente, et
//! s'exécutent donc dans l'ordre d'envoi. Une opération sur une fenêtre d'accueil déjà
//! détruite est ignorée (HWND comparé à la fenêtre vivante), pour qu'un HWND réutilisé par
//! Windows ne soit jamais touché.

use serde::Deserialize;

/// Rectangle de la zone d'accueil en px CSS, relatif au coin haut gauche de la WebView (qui
/// couvre la zone cliente de la fenêtre).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct CssRect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Rectangle en px physiques (coordonnées clientes de la fenêtre du launcher).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// Convertit un rectangle CSS en px physiques : bords arrondis séparément (pas de trou d'un
/// pixel entre la zone et la fenêtre), facteur d'échelle invalide ramené à 1, valeurs non
/// finies à 0, taille au moins 1×1 (un swapchain de taille nulle est refusé par le pilote).
pub fn to_physical(rect: CssRect, scale: f64) -> PhysicalRect {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let px = |v: f64| {
        let v = (v * scale).round();
        if v.is_finite() {
            v.clamp(-1_000_000.0, 1_000_000.0) as i32
        } else {
            0
        }
    };
    let left = px(rect.x);
    let top = px(rect.y);
    let right = px(rect.x + rect.width.max(0.0));
    let bottom = px(rect.y + rect.height.max(0.0));
    PhysicalRect {
        x: left,
        y: top,
        width: (right - left).max(1),
        height: (bottom - top).max(1),
    }
}

/// Fenêtre d'accueil du player (abstraite pour tester `PlayerService` sans Tauri).
pub trait EmbedHost: Send + Sync {
    /// Crée la fenêtre d'accueil (cachée) et renvoie son HWND, ou `None` si l'incrustation
    /// est impossible (hors Windows, fenêtre principale introuvable, échec Win32).
    fn create(&self) -> Option<isize>;
    /// Place la fenêtre d'accueil sur `rect` (px CSS) et l'affiche ; `None` la cache.
    fn layout(&self, hwnd: isize, rect: Option<CssRect>);
    /// Détruit la fenêtre d'accueil (sans effet si elle l'est déjà).
    fn destroy(&self, hwnd: isize);
    /// Donne le clavier à la fenêtre du player incrustée dans la fenêtre d'accueil.
    fn focus(&self, hwnd: isize);
}

/// Pas d'incrustation : le player s'ouvre dans sa fenêtre séparée.
pub struct NoEmbed;

impl EmbedHost for NoEmbed {
    fn create(&self) -> Option<isize> {
        None
    }
    fn layout(&self, _hwnd: isize, _rect: Option<CssRect>) {}
    fn destroy(&self, _hwnd: isize) {}
    fn focus(&self, _hwnd: isize) {}
}

/// Incrustation désactivée par `FLAM_LAUNCHER_NO_EMBED=1` (dépannage).
pub fn disabled_by_env() -> bool {
    std::env::var_os("FLAM_LAUNCHER_NO_EMBED").is_some_and(|v| v == "1")
}

#[cfg(windows)]
pub use windows_host::TauriEmbed;

#[cfg(windows)]
mod windows_host {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::channel;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    use tauri::{AppHandle, Manager};
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetParent, GetWindow, SetWindowPos, ShowWindow, GW_CHILD,
        HWND_TOP, SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, WS_CHILD, WS_CLIPSIBLINGS,
    };

    use super::{to_physical, CssRect, EmbedHost};

    /// Attente maximale de la création sur le thread principal.
    const CREATE_TIMEOUT: Duration = Duration::from_secs(3);

    /// Fenêtre d'accueil enfant de la fenêtre `main` du launcher.
    pub struct TauriEmbed {
        app: AppHandle,
        /// Fenêtre d'accueil vivante (lue et écrite sur le thread principal seulement).
        live: Arc<Mutex<Option<isize>>>,
    }

    impl TauriEmbed {
        pub fn new(app: AppHandle) -> Self {
            Self {
                app,
                live: Arc::default(),
            }
        }
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    impl EmbedHost for TauriEmbed {
        fn create(&self) -> Option<isize> {
            let (tx, rx) = channel();
            let app = self.app.clone();
            let live = self.live.clone();
            // Posé si la réponse n'arrive pas à temps : la création, si elle n'a pas encore eu
            // lieu, est alors abandonnée.
            let abandoned = Arc::new(AtomicBool::new(false));
            let gave_up = abandoned.clone();
            let sent = self.app.run_on_main_thread(move || {
                if gave_up.load(Ordering::SeqCst) {
                    return;
                }
                let parent = app
                    .get_webview_window("main")
                    .and_then(|window| window.hwnd().ok())
                    .map(|hwnd| hwnd.0 as HWND);
                let Some(parent) = parent else {
                    let _ = tx.send(None);
                    return;
                };
                let class = wide("STATIC");
                // SAFETY : appel Win32 sur le thread qui possède `parent` ; chaînes terminées
                // par un zéro et vivantes pendant l'appel.
                let host = unsafe {
                    CreateWindowExW(
                        0,
                        class.as_ptr(),
                        std::ptr::null(),
                        WS_CHILD | WS_CLIPSIBLINGS,
                        0,
                        0,
                        320,
                        240,
                        parent,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null(),
                    )
                };
                if host.is_null() {
                    let _ = tx.send(None);
                    return;
                }
                let id = host as isize;
                *live.lock().unwrap_or_else(PoisonError::into_inner) = Some(id);
                let _ = tx.send(Some(id));
            });
            if sent.is_err() {
                return None;
            }
            match rx.recv_timeout(CREATE_TIMEOUT) {
                Ok(id) => id,
                Err(_) => {
                    tracing::warn!("création de la fenêtre d'accueil du player sans réponse");
                    abandoned.store(true, Ordering::SeqCst);
                    // Si la création a eu lieu entre-temps, la fenêtre est détruite : posté
                    // après la création (ordre d'envoi), avant toute création suivante (le
                    // créneau du player est tenu par l'appelant jusqu'à notre retour).
                    let live = self.live.clone();
                    let _ = self.app.run_on_main_thread(move || {
                        let orphan = live.lock().unwrap_or_else(PoisonError::into_inner).take();
                        if let Some(hwnd) = orphan {
                            // SAFETY : fenêtre créée par ce thread et jamais remise au player.
                            unsafe {
                                DestroyWindow(hwnd as HWND);
                            }
                        }
                    });
                    None
                }
            }
        }

        fn layout(&self, hwnd: isize, rect: Option<CssRect>) {
            let live = self.live.clone();
            let _ = self.app.run_on_main_thread(move || {
                if *live.lock().unwrap_or_else(PoisonError::into_inner) != Some(hwnd) {
                    return;
                }
                let host = hwnd as HWND;
                // SAFETY : `host` est la fenêtre d'accueil vivante, créée par ce thread.
                unsafe {
                    match rect {
                        None => {
                            ShowWindow(host, SW_HIDE);
                        }
                        Some(rect) => {
                            let parent = GetParent(host);
                            let dpi = if parent.is_null() {
                                96
                            } else {
                                GetDpiForWindow(parent)
                            };
                            let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
                            let r = to_physical(rect, scale);
                            SetWindowPos(
                                host,
                                HWND_TOP,
                                r.x,
                                r.y,
                                r.width,
                                r.height,
                                SWP_NOACTIVATE | SWP_SHOWWINDOW,
                            );
                        }
                    }
                }
            });
        }

        fn focus(&self, hwnd: isize) {
            let live = self.live.clone();
            let _ = self.app.run_on_main_thread(move || {
                if *live.lock().unwrap_or_else(PoisonError::into_inner) != Some(hwnd) {
                    return;
                }
                // SAFETY : `hwnd` est la fenêtre d'accueil vivante ; son enfant est la fenêtre
                // du player, dont la file d'entrée est liée à ce thread (fenêtre enfant).
                unsafe {
                    let player = GetWindow(hwnd as HWND, GW_CHILD);
                    if !player.is_null() {
                        SetFocus(player);
                    }
                }
            });
        }

        fn destroy(&self, hwnd: isize) {
            let live = self.live.clone();
            let _ = self.app.run_on_main_thread(move || {
                let mut current = live.lock().unwrap_or_else(PoisonError::into_inner);
                if *current != Some(hwnd) {
                    return;
                }
                *current = None;
                // SAFETY : fenêtre d'accueil vivante, détruite par le thread qui l'a créée.
                unsafe {
                    DestroyWindow(hwnd as HWND);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> CssRect {
        CssRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn conversion_a_l_echelle_1() {
        assert_eq!(
            to_physical(rect(10.0, 20.0, 320.0, 240.0), 1.0),
            PhysicalRect {
                x: 10,
                y: 20,
                width: 320,
                height: 240
            }
        );
    }

    #[test]
    fn conversion_a_150_pourcent_arrondit_les_bords() {
        // 10,5 × 1,5 = 15,75 → 16 ; bord droit (10,5 + 100,25) × 1,5 = 166,125 → 166.
        assert_eq!(
            to_physical(rect(10.5, 7.0, 100.25, 50.0), 1.5),
            PhysicalRect {
                x: 16,
                y: 11,
                width: 150,
                height: 75
            }
        );
    }

    #[test]
    fn facteur_invalide_et_tailles_nulles() {
        let r = to_physical(rect(5.0, 5.0, 0.0, -3.0), 0.0);
        assert_eq!((r.x, r.y, r.width, r.height), (5, 5, 1, 1));
        let r = to_physical(rect(f64::NAN, 1.0, 10.0, 10.0), f64::INFINITY);
        assert_eq!((r.x, r.y), (0, 1));
        assert!(r.width >= 1 && r.height >= 1);
    }

    #[test]
    fn sans_incrustation_rien_n_est_cree() {
        assert_eq!(NoEmbed.create(), None);
    }
}
