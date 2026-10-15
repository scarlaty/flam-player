import { invokeCommand } from "./error";

/**
 * Restauration d'un fichier de configuration corrompu aux valeurs par défaut
 * (miroir de `Recovery` dans `src-tauri/src/storage.rs`). Signalée par `settings_get` et
 * `sources_list` tant que l'utilisateur ne l'a pas acquittée.
 */
export interface Recovery {
  /** Fichier restauré : `settings.json` ou `sources.json`. */
  file: string;
  /** Nom de la sauvegarde `*.bak` créée à côté du fichier. */
  backup: string;
}

/**
 * `config_ack_recovery` : acquitte la restauration de `file`, ou de tous les fichiers si
 * `file` est omis. À appeler une fois l'avertissement montré à l'utilisateur.
 */
export function ackRecovery(file?: string): Promise<void> {
  return invokeCommand<void>("config_ack_recovery", { file: file ?? null });
}
