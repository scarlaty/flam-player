import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invokeCommand } from "./error";
import type { LibraryEntry } from "./library";
import type { StoryEntry } from "./story";

/** État d'une tâche d'installation (miroir de `TaskState` dans `src-tauri/src/install/mod.rs`). */
export type TaskState =
  | "queued"
  | "downloading"
  | "converting"
  | "installing"
  | "done"
  | "failed"
  | "cancelled";

/**
 * Erreur d'une tâche : `kind` stable (`network`, `download_invalid`, `converter_unavailable`,
 * `conversion`, `cancelled`…) ; `code` est le code de `telmi2flam` (`missing_asset`,
 * `invalid_content`…) pour `kind = "conversion"`.
 */
export interface TaskError {
  kind: string;
  message: string;
  code: string | null;
}

/** Tâche d'installation (miroir de `InstallTask`). */
export interface InstallTask {
  id: string;
  entry: StoryEntry;
  state: TaskState;
  /** Avancement de l'étape en cours (0 à 100), si connu. */
  percent: number | null;
  error: TaskError | null;
  /** Création (secondes Unix). */
  created_at: number;
}

/** Charge de `install://progress` : changement d'état ou avancement d'une tâche. */
export interface InstallProgress {
  task_id: string;
  state: TaskState;
  percent: number | null;
  /** Téléchargement : octets reçus et taille annoncée. */
  received: number | null;
  total: number | null;
  /** Conversion : phase de `telmi2flam` (`images`, `audio`, `scripts`, `zip`). */
  phase: string | null;
}

/** Charge de `install://done`. */
export interface InstallDone {
  task_id: string;
  entry: LibraryEntry;
  /** Avertissements de la conversion. */
  warnings: number;
}

/** Charge de `install://error` (`state` : `failed` ou `cancelled`). */
export interface InstallFailed {
  task_id: string;
  state: TaskState;
  error: TaskError;
}

export const INSTALL_PROGRESS_EVENT = "install://progress";
export const INSTALL_DONE_EVENT = "install://done";
export const INSTALL_ERROR_EVENT = "install://error";

/**
 * `install_start` : installe l'histoire `entryId` de la source `sourceId` (catalogue : entrée du
 * catalogue en cache ; dossier : entrée du dernier `scanFolder`, le dossier étant analysé s'il
 * ne l'a pas encore été). Une tâche déjà active pour le même `uuid` est renvoyée telle quelle.
 * `uuid` : histoire attendue (`not_found` si l'entrée en propose une autre).
 */
export function startInstall(
  sourceId: string,
  entryId: string,
  uuid?: string,
): Promise<InstallTask> {
  return invokeCommand<InstallTask>(
    "install_start",
    uuid === undefined ? { sourceId, entryId } : { sourceId, entryId, uuid },
  );
}

/** `install_cancel` : annule une tâche (sans effet si elle est terminée). */
export function cancelInstall(taskId: string): Promise<void> {
  return invokeCommand<void>("install_cancel", { taskId });
}

/** `install_list` : tâches de la session (actives et terminées). */
export function listInstalls(): Promise<InstallTask[]> {
  return invokeCommand<InstallTask[]>("install_list");
}

/** Écoute `install://progress`. Renvoie la fonction de désabonnement. */
export function onInstallProgress(
  handler: (event: InstallProgress) => void,
): Promise<UnlistenFn> {
  return listen<InstallProgress>(INSTALL_PROGRESS_EVENT, (e) => handler(e.payload));
}

/** Écoute `install://done`. */
export function onInstallDone(
  handler: (event: InstallDone) => void,
): Promise<UnlistenFn> {
  return listen<InstallDone>(INSTALL_DONE_EVENT, (e) => handler(e.payload));
}

/** Écoute `install://error` (échec ou annulation). */
export function onInstallError(
  handler: (event: InstallFailed) => void,
): Promise<UnlistenFn> {
  return listen<InstallFailed>(INSTALL_ERROR_EVENT, (e) => handler(e.payload));
}
