import { invokeCommand } from "./error";
import type { StoryEntry } from "./story";

/** Fichier ou archive ignoré pendant l'analyse (miroir de `ScanWarning`). */
export interface ScanWarning {
  path: string;
  message: string;
}

/**
 * Histoire d'un dossier. `entry_id` (hash du chemin) est stable et unique dans la source : à
 * utiliser comme clé d'affichage. Un `.plain` et un `.plain.pk` de même `uuid` donnent deux
 * entrées. `thumb_url` est une référence opaque (égale à `entry_id`) ou `null`, à passer à
 * `folderThumbnail` : ce n'est pas une URL affichable.
 */
export type FolderEntry = StoryEntry & { entry_id: string };

/** Réponse de `folder_scan` (miroir de `FolderScan` dans `src-tauri/src/folder.rs`). */
export interface FolderScan {
  entries: FolderEntry[];
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
 * `folder_thumbnail` : URL `data:image/png;base64,…` de la vignette d'une histoire du dernier
 * scan de la source. Erreur `not_found` pour une entrée inconnue.
 */
export function folderThumbnail(sourceId: string, entryId: string): Promise<string> {
  return invokeCommand<string>("folder_thumbnail", { sourceId, entryId });
}
