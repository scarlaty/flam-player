import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * Réduit la fenêtre de la surcouche (pendant la lecture). Meilleur effort : un échec (fenêtre
 * indisponible, permission refusée) n'empêche jamais la lecture ; renvoie vrai si réduite.
 */
export async function minimizeWindow(): Promise<boolean> {
  try {
    await getCurrentWindow().minimize();
    return true;
  } catch {
    return false;
  }
}

/** Rétablit la fenêtre réduite par `minimizeWindow` et lui rend le focus (meilleur effort). */
export async function restoreWindow(): Promise<void> {
  try {
    const window = getCurrentWindow();
    await window.unminimize();
    await window.setFocus();
  } catch {
    // Rien à faire : l'utilisateur peut rouvrir la fenêtre depuis la barre des tâches.
  }
}

/**
 * Appelle `handler` (attendu) quand la fenêtre va se fermer, avant la fermeture. Meilleur
 * effort : renvoie une fonction de désinscription (sans effet si la fenêtre est indisponible).
 */
export async function onWindowCloseRequested(
  handler: () => Promise<void>,
): Promise<() => void> {
  try {
    return await getCurrentWindow().onCloseRequested(async () => {
      try {
        await handler();
      } catch {
        // La fermeture ne doit jamais dépendre du gestionnaire.
      }
    });
  } catch {
    return () => {};
  }
}
