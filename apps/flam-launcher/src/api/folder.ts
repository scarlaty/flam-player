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

/**
 * Les `thumb_url` des entrées d'un dossier sont des références (`entry_id`), pas des URL
 * affichables. `folder_thumbnail` renvoie l'URL `data:image/png;base64,…` de la vignette d'une
 * histoire du dernier scan de la source (erreur `not_found` pour une entrée inconnue).
 */
export function folderThumbnail(sourceId: string, entryId: string): Promise<string> {
  return invokeCommand<string>("folder_thumbnail", { sourceId, entryId });
}
