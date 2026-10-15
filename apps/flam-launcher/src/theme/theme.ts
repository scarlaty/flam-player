/** Choix de thème : suivre le système ou forcer clair/sombre. */
export type ThemePreference = "system" | "light" | "dark";

export const THEME_PREFERENCES: readonly ThemePreference[] = ["system", "light", "dark"];

/**
 * Applique le thème sur l'élément racine. Les jetons (tokens.css) réagissent à
 * l'attribut `data-theme` ; sans attribut, `prefers-color-scheme` décide.
 * La mémorisation du choix relève des paramètres de l'application.
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
}
