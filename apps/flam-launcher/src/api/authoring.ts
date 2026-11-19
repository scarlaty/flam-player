import { invokeCommand, type AppError } from "./error";
import type { InstallTask } from "./install";

/** `source_id` d'une histoire faite d'un dossier (miroir de `authoring::build::SOURCE_ID`). */
export const AUTHORING_SOURCE_ID = "authoring-folder";

/** Organisation d'un dossier (miroir de `Mode`, `authoring/settings.rs`). */
export type FolderMode = "recueil" | "chapitres";

/** Ligne du rapport de lecture (miroir de `Problem`, `authoring/tree.rs`). */
export interface FolderProblem {
  /** Code stable (`no_story`, `too_many_stories`, `orphan_image`…). */
  code: string;
  severity: "blocking" | "warning";
  /** Chemin relatif à la racine, en `/` (racine : `""`). */
  path: string;
  /** Explication technique (journaux) : l'écran utilise `code` et `path`. */
  message: string;
}

/** Élément de l'arbre (miroir de `Node`). */
export type FolderNode =
  | {
      kind: "menu";
      id: string;
      dir: string;
      title: string;
      mode: FolderMode;
      children: FolderNode[];
    }
  | {
      kind: "story";
      id: string;
      file: string;
      title: string;
      bytes: number;
    };

/** Conflit d'identifiant (miroir de `Conflict`, `authoring/identity.rs`). */
export type FolderConflict =
  | { kind: "copied"; other_folder: string; title: string }
  | { kind: "foreign"; title: string };

/** Réponse à un conflit (miroir de `ConflictChoice`). */
export type ConflictChoice = "replace_other" | "new_story";

/** Résultat de `authoring_read` (miroir de `FolderReport`, `authoring/commands.rs`). */
export interface FolderReport {
  folder: string;
  title: string;
  root: FolderNode;
  problems: FolderProblem[];
  blocked: boolean;
  story_count: number;
  audio_bytes: number;
  mode: FolderMode;
  suggested_mode: FolderMode | null;
  /** Identifiant connu : l'histoire existe déjà (actualisation). */
  uuid: string | null;
  installed_version: number | null;
  remembered: boolean;
  conflict: FolderConflict | null;
}

/** Résultat de `authoring_create` (miroir de `FolderCreated`). */
export interface FolderCreated {
  task: InstallTask;
  uuid: string;
  version: number;
  created: boolean;
  saved_in: "folder" | "launcher";
}

/** `authoring_read` : lit le dossier `path` (arbre et problèmes) sans rien écrire. */
export function readStoryFolder(path: string): Promise<FolderReport> {
  return invokeCommand<FolderReport>("authoring_read", { path });
}

/**
 * `authoring_create` : crée (ou met à jour) l'histoire du dossier `path` ; `choice` répond au
 * conflit signalé par `readStoryFolder`. La tâche rejoint la file d'installation.
 */
export function createStoryFromFolder(
  path: string,
  choice: ConflictChoice | null = null,
): Promise<FolderCreated> {
  return invokeCommand<FolderCreated>("authoring_create", { path, choice });
}

/**
 * Code d'une erreur `kind = "authoring"` (miroir de `authoring::commands::code`) : le texte
 * montré en dépend, jamais le message.
 */
export type AuthoringErrorCode =
  | "folder_missing"
  | "folder_refused"
  | "blocked"
  | "busy"
  | "conflict_copied"
  | "conflict_foreign"
  | "version_overflow"
  | "retry_unavailable"
  | "folder_changed"
  | "uuid_mismatch"
  | "not_from_folder";

/** Code de création porté par `error` (`kind = "authoring"`), sinon `null`. */
export function authoringCode(error: AppError): AuthoringErrorCode | null {
  return error.kind === "authoring" && typeof error.code === "string"
    ? (error.code as AuthoringErrorCode)
    : null;
}

/**
 * `authoring_retry` : relance la création en échec (ou annulée) `taskId` depuis le même dossier,
 * sous le même identifiant et avec la même réponse au conflit.
 */
export function retryFolderTask(taskId: string): Promise<FolderCreated> {
  return invokeCommand<FolderCreated>("authoring_retry", { taskId });
}

/**
 * `authoring_relocate` : « Choisir le dossier » de l'histoire `uuid` dont le dossier est
 * introuvable. Le dossier `path` doit porter le même identifiant ; rend le chemin retenu.
 */
export function relocateStoryFolder(uuid: string, path: string): Promise<string> {
  return invokeCommand<string>("authoring_relocate", { uuid, path });
}
