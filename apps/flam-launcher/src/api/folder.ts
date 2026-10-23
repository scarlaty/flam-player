import { listen, type UnlistenFn } from "@tauri-apps/api/event";
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

/** Nom de l'événement émis quand le contenu d'un dossier surveillé change. */
export const FOLDER_CHANGED_EVENT = "folder://changed";

/** Charge de `folder://changed` (miroir de `FolderChanged` dans `src-tauri/src/watch.rs`). */
export interface FolderChanged {
  source_id: string;
}

/**
 * `folder_watch` : démarre (`enabled`) ou arrête la surveillance du dossier d'une source.
 * Les rafales de modifications donnent un seul événement `folder://changed`. Erreur
 * `folder_unavailable` si le dossier a disparu.
 */
export function watchFolder(sourceId: string, enabled: boolean): Promise<void> {
  // Appels exécutés l'un après l'autre : un démontage rapide de l'écran (arrêt) ne doit pas
  // doubler le démarrage correspondant, ni un remontage (démarrage) précéder l'arrêt précédent.
  const run = watchQueue.then(() =>
    invokeCommand<void>("folder_watch", { sourceId, enabled }),
  );
  watchQueue = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

let watchQueue: Promise<void> = Promise.resolve();

/** Écoute `folder://changed`. Renvoie la fonction de désabonnement. */
export function onFolderChanged(
  handler: (event: FolderChanged) => void,
): Promise<UnlistenFn> {
  return listen<FolderChanged>(FOLDER_CHANGED_EVENT, (e) => handler(e.payload));
}
