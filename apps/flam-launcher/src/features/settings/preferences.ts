/**
 * Préférences d'interface propres à ce poste, gardées dans le stockage local de la fenêtre
 * (pas dans `settings.json`, qui est celui du backend). Lecture et écriture sans exception :
 * stockage indisponible, la valeur par défaut s'applique.
 */
const MINIMIZE_DURING_PLAY_KEY = "flam-launcher.minimizeDuringPlay";

/** Réduire la fenêtre de la surcouche pendant la lecture (désactivé par défaut). */
export function getMinimizeDuringPlay(): boolean {
  try {
    return window.localStorage.getItem(MINIMIZE_DURING_PLAY_KEY) === "1";
  } catch {
    return false;
  }
}

export function setMinimizeDuringPlay(value: boolean): void {
  try {
    window.localStorage.setItem(MINIMIZE_DURING_PLAY_KEY, value ? "1" : "0");
  } catch {
    // Préférence non conservée : sans conséquence sur la lecture.
  }
}

const FLAM_SEND_ACK_KEY = "flam-launcher.flamSendWarningAck";

/** L'avertissement « fonction expérimentale, non officielle » de l'envoi a déjà été accepté. */
export function getFlamSendWarningAck(): boolean {
  try {
    return window.localStorage.getItem(FLAM_SEND_ACK_KEY) === "1";
  } catch {
    return false;
  }
}

export function setFlamSendWarningAck(): void {
  try {
    window.localStorage.setItem(FLAM_SEND_ACK_KEY, "1");
  } catch {
    // Non mémorisé : l'avertissement sera simplement redemandé.
  }
}
