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
