import { invokeCommand } from "./error";

/** Paramètres enregistrés (miroir de `Settings` dans `src-tauri/src/settings.rs`). */
export interface Settings {
  schema_version: number;
  /** Chemin de `flam-player.exe` ; `null` : détection automatique. */
  player_path: string | null;
  /** Chemin de Python ; `null` : `python` du PATH. */
  python_path: string | null;
  /** Dossier de cache ; `null` : dossier par défaut. */
  cache_dir: string | null;
  /** URL du catalogue proposé par défaut. */
  default_catalog_url: string;
}

/** Valeurs effectives après détection automatique (`null` : introuvable). */
export interface ResolvedSettings {
  player_path: string | null;
  python_path: string | null;
  cache_dir: string;
}

export interface SettingsView {
  settings: Settings;
  resolved: ResolvedSettings;
}

/** Appelle `settings_get` : paramètres enregistrés et valeurs effectives. */
export function getSettings(): Promise<SettingsView> {
  return invokeCommand<SettingsView>("settings_get");
}

/**
 * Appelle `settings_set` : valide et enregistre. Rejette avec une `AppError`
 * de `kind` `validation` si une valeur est refusée.
 */
export function setSettings(settings: Settings): Promise<SettingsView> {
  return invokeCommand<SettingsView>("settings_set", { settings });
}
