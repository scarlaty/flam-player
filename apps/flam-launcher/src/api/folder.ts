import { invokeCommand } from "./error";
import type { StoryEntry } from "./story";

/** Fichier ou archive ignoré pendant l'analyse (miroir de `ScanWarning`). */
export interface ScanWarning {
  path: string;
  message: string;
}

/** Réponse de `folder_scan` (miroir de `FolderScan` dans `src-tauri/src/folder.rs`). */
export interface FolderScan {
  entries: StoryEntry[];
  warnings: ScanWarning[];
}

/**
 * `folder_scan` : analyse une source de type dossier. Non récursif par défaut.
 * Erreur `folder_unavailable` si le dossier a disparu.
 */
export function scanFolder(sourceId: string, recursive = false): Promise<FolderScan> {
  return invokeCommand<FolderScan>("folder_scan", { sourceId, recursive });
}
