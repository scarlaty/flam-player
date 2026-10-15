import { invokeCommand } from "./error";
import type { Recovery } from "./recovery";

/** Paramètres enregistrés (miroir de `Settings` dans `src-tauri/src/settings.rs`). */
export interface Settings {
  schema_version: number;
  /** Chemin absolu local de `flam-player.exe` ; `null` : détection automatique. */
  player_path: string | null;
  /** Chemin absolu local d'un `.exe` Python ; `null` : détection dans le PATH. */
  python_path: string | null;
  /** Dossier de cache ; `null` : dossier par défaut. */
  cache_dir: string | null;
  /** URL du catalogue proposé par défaut. */
  default_catalog_url: string;
}

/**
 * Valeurs effectives après détection automatique (`null` : introuvable).
 *
 * Ce n'est PAS une garantie : le fichier peut disparaître après le calcul. Le code qui
 * lance un exécutable doit revérifier juste avant (issue #16).
 */
export interface ResolvedSettings {
  player_path: string | null;
  python_path: string | null;
  cache_dir: string;
}

/**
 * Problème détecté sur un paramètre :
 * - `invalid_path` : valeur configurée refusée (extension, chemin réseau, relatif…) ;
 * - `not_found` : fichier configuré disparu ;
 * - `store_alias` : seul un alias Python du Microsoft Store a été trouvé (ignoré).
 *
 * Pour les deux premiers, `resolved` utilise la détection automatique à la place.
 */
export interface SettingsIssue {
  field: "player_path" | "python_path" | "cache_dir";
  kind: "invalid_path" | "not_found" | "store_alias";
  message: string;
}

export interface SettingsView {
  settings: Settings;
  resolved: ResolvedSettings;
  issues: SettingsIssue[];
  /** Présent tant que la restauration de `settings.json` n'est pas acquittée (`ackRecovery`). */
  recovery: Recovery | null;
}

/** Appelle `settings_get` : paramètres enregistrés, valeurs effectives et problèmes. */
export function getSettings(): Promise<SettingsView> {
  return invokeCommand<SettingsView>("settings_get");
}

/**
 * Appelle `settings_set` : valide puis REMPLACE l'objet complet (pas de fusion partielle :
 * un champ à `null` repasse en automatique). Partir de `getSettings().settings`, modifier les
 * champs voulus et renvoyer l'objet entier. Rejette avec une `AppError` de `kind`
 * `validation` si une valeur est refusée.
 */
export function setSettings(settings: Settings): Promise<SettingsView> {
  return invokeCommand<SettingsView>("settings_set", { settings });
}
