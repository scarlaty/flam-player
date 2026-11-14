import { getCurrentWindow } from "@tauri-apps/api/window";
import { getCloseVerdict } from "./device";

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
 *
 * Avec un gestionnaire JavaScript, Tauri laisse celui-ci détruire la fenêtre sauf s'il appelle
 * `preventDefault()` : il l'appelle donc toujours, fait son travail, puis demande le verdict du
 * backend (`device_close_verdict`, seul point de décision : une écriture en cours sur la Flam
 * retient la fermeture, #577) et ne détruit la fenêtre que s'il est « allow ». Verdict
 * indisponible (échec de la commande) : la fenêtre est fermée, la sortie ne dépend jamais de lui.
 */
export async function onWindowCloseRequested(
  handler: () => Promise<void>,
): Promise<() => void> {
  try {
    const current = getCurrentWindow();
    return await current.onCloseRequested(async (event) => {
      event.preventDefault();
      try {
        await handler();
      } catch {
        // La fermeture ne doit jamais dépendre du gestionnaire.
      }
      if (await closeAllowed()) {
        try {
          await current.destroy();
        } catch {
          // Fenêtre déjà détruite.
        }
      }
    });
  } catch {
    return () => {};
  }
}

/** Verdict du backend : faux seulement s'il retient la fermeture (écriture en cours). */
async function closeAllowed(): Promise<boolean> {
  try {
    return (await getCloseVerdict()) !== "retain";
  } catch {
    return true;
  }
}
