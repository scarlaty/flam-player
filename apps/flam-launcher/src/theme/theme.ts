/** Choix de thème : suivre le système ou forcer clair/sombre. */
export type ThemePreference = "system" | "light" | "dark";

export const THEME_PREFERENCES: readonly ThemePreference[] = ["system", "light", "dark"];

const STORAGE_KEY = "flam-launcher.theme";

function isPreference(value: unknown): value is ThemePreference {
  return THEME_PREFERENCES.includes(value as ThemePreference);
}

/** Dernier thème affiché, mémorisé localement pour l'appliquer avant le chargement des paramètres. */
export function readRememberedTheme(): ThemePreference | null {
  try {
    const value = window.localStorage.getItem(STORAGE_KEY);
    return isPreference(value) ? value : null;
  } catch {
    return null;
  }
}

function rememberTheme(preference: ThemePreference): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, preference);
  } catch {
    // Stockage indisponible : le thème sera simplement appliqué après le chargement.
  }
}

/**
 * Applique le thème sur l'élément racine. Les jetons (tokens.css) réagissent à
 * l'attribut `data-theme` ; sans attribut, `prefers-color-scheme` décide.
 * Le thème est aussi mémorisé localement (démarrage sans flash) ; la référence reste
 * celle des paramètres de l'application.
 */
export function applyTheme(
  preference: ThemePreference,
  root: HTMLElement = document.documentElement,
): void {
  if (preference === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", preference);
  }
  rememberTheme(preference);
}

/** Applique le thème mémorisé, s'il y en a un (à appeler avant le premier rendu). */
export function applyRememberedTheme(root: HTMLElement = document.documentElement): void {
  const preference = readRememberedTheme();
  if (preference) applyTheme(preference, root);
}
