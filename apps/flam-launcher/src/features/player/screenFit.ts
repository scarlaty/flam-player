/** Écran de la Flam, en pixels. */
export const SCREEN_WIDTH = 320;
export const SCREEN_HEIGHT = 240;

/**
 * Agrandissement maximal de l'écran incrusté, en px CSS (×2 : 640×480) : les images de la Flam
 * sont en basse résolution, un fort agrandissement les rend laides.
 */
export const MAX_SCREEN_FACTOR = 2;

/**
 * Part minimale de la taille cible qu'un facteur entier (en px physiques) doit atteindre pour
 * être préféré : en deçà, mieux vaut un facteur non entier qu'un écran trop petit.
 */
export const MIN_WHOLE_SHARE = 0.8;

/**
 * Taille (px CSS) de l'écran Flam incrusté dans un espace `width`×`height` (px CSS), pour un
 * rapport px physiques / px CSS `ratio` (`devicePixelRatio`).
 *
 * Cible : la plus grande taille 4:3 qui tient dans l'espace, plafonnée à ×2 en px CSS (640×480).
 * Si un nombre entier de px physiques par pixel de la Flam (pixels nets, sans pixel doublé de-ci
 * de-là) donne au moins 80 % de la cible, il est retenu ; sinon la cible est gardée (facteur non
 * entier) : un espace à peine trop court ne fait pas retomber l'écran à ×1.
 */
export function fitScreen(
  width: number,
  height: number,
  ratio: number,
): { width: number; height: number } {
  const dpr = Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
  const room = (v: number) => (Number.isFinite(v) && v > 0 ? v : 0);
  // Facteur cible, en px physiques par pixel de la Flam.
  const fit = Math.min(
    (room(width) * dpr) / SCREEN_WIDTH,
    (room(height) * dpr) / SCREEN_HEIGHT,
    MAX_SCREEN_FACTOR * dpr,
  );
  // Tolérance aux arrondis flottants (à 185 %, 480 / 1,85 × 1,85 / 240 = 1,9999999999999998).
  const whole = Math.floor(fit + 1e-9);
  const factor = whole >= 1 && whole + 1e-9 >= MIN_WHOLE_SHARE * fit ? whole : fit;
  return {
    width: (SCREEN_WIDTH * factor) / dpr,
    height: (SCREEN_HEIGHT * factor) / dpr,
  };
}
