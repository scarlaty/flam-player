import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invokeCommand } from "./error";
import type { LibraryEntry } from "./library";
import type { PackComposition, ValidationReport } from "./pack";
import type { StoryEntry } from "./story";

/** État d'une tâche d'installation (miroir de `TaskState` dans `src-tauri/src/install/mod.rs`). */
export type TaskState =
  | "queued"
  | "preparing"
  | "prepared"
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
  /** Pack de podcasts (#217) ; `null` pour une histoire d'une source. */
  pack: PackStatus | null;
}

/** État d'un pack de podcasts dans la file (miroir de `PackStatus`, `install/podcast.rs`). */
export interface PackStatus {
  /** Épisodes de la composition. */
  episodes: number;
  /** Titres des épisodes retirés du pack (échec de téléchargement ou de conversion, P7). */
  dropped: string[];
  /**
   * Avertissements des titres lus (#462) : voix remplacée par la voix par défaut, titres restés
   * muets. Vide sans titres lus.
   */
  warnings: string[];
}

/** Résultat de `pack_create` : `task` vaut `null` si la composition est refusée. */
export interface PackCreated {
  task: InstallTask | null;
  report: ValidationReport;
  /** Composition conservée après la création de la tâche (#461) ; `null` sans tâche. */
  composition?: CompositionRecord | null;
}

/** Conservation de la composition (miroir de `CompositionRecord`, `podcast/compositions.rs`). */
export interface CompositionRecord {
  /** Nom de la composition conservée ; `null` : non conservée (voir `error`). */
  name: string | null;
  /** `false` : composition déjà conservée (pack refait). */
  new: boolean;
  error: string | null;
  /** Fichier des compositions mis de côté ou entrées écartées. */
  notice: string | null;
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
  /** Pack de podcasts : épisodes retirés (P7) ; `null` pour une histoire d'une source. */
  pack: PackStatus | null;
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

/**
 * `pack_create` : revalide la composition dans le backend (épisodes retrouvés dans les flux en
 * cache) et, si elle est valide, ajoute la création du pack à la file d'installation. Sinon,
 * `task` vaut `null` et `report.problems` dit pourquoi. `draftRevision` : dernière révision du
 * brouillon, effacé après la publication du pack s'il n'a pas changé entre-temps.
 */
export function createPack(
  composition: PackComposition,
  draftRevision?: number,
): Promise<PackCreated> {
  return invokeCommand<PackCreated>(
    "pack_create",
    draftRevision === undefined ? { composition } : { composition, draftRevision },
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
