/** Écran de la Flam, en pixels. */
export const SCREEN_WIDTH = 320;
export const SCREEN_HEIGHT = 240;

/**
 * Agrandissement maximal de l'écran incrusté, en px CSS (×2 : 640×480) : les images de la Flam
 * sont en basse résolution, un fort agrandissement les rend laides.
 */
export const MAX_SCREEN_FACTOR = 2;

/**
 * Taille (px CSS) de l'écran Flam incrusté dans un espace `width`×`height` (px CSS), pour un
 * rapport px physiques / px CSS `ratio` (`devicePixelRatio`).
 *
 * L'écran garde ses proportions 4:3, ne dépasse ni l'espace ni ×2 en px CSS, et occupe un nombre
 * entier de px physiques par pixel de la Flam (pixels nets, sans pixel doublé de-ci de-là). Si
 * même ×1 physique ne tient pas, l'écran est réduit à l'espace disponible (facteur non entier).
 */
export function fitScreen(
  width: number,
  height: number,
  ratio: number,
): { width: number; height: number } {
  const dpr = Number.isFinite(ratio) && ratio > 0 ? ratio : 1;
  const room = (v: number) => (Number.isFinite(v) && v > 0 ? v : 0);
  // Facteur le plus grand, en px physiques par pixel de la Flam.
  const fit = Math.min(
    (room(width) * dpr) / SCREEN_WIDTH,
    (room(height) * dpr) / SCREEN_HEIGHT,
    MAX_SCREEN_FACTOR * dpr,
  );
  // Tolérance aux arrondis flottants (à 185 %, 480 / 1,85 × 1,85 / 240 = 1,9999999999999998).
  const whole = Math.floor(fit + 1e-9);
  const factor = whole >= 1 ? whole : fit;
  return {
    width: (SCREEN_WIDTH * factor) / dpr,
    height: (SCREEN_HEIGHT * factor) / dpr,
  };
}
