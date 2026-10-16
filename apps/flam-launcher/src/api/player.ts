import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { invokeCommand } from "./error";

/** Lancement en cours (miroir de `PlayerSession` dans `src-tauri/src/player.rs`). */
export interface PlayerSession {
  /** `library` (histoire installée) ou `folder` (fichier d'une source dossier). */
  origin: "library" | "folder";
  /** Identifiant de l'histoire (bibliothèque : `uuid` ; dossier : `entry_id`). */
  uuid: string;
  title: string;
  pid: number;
  /** Démarrage (secondes Unix). */
  started_at: number;
}

/** Charge de `player://exited` (miroir de `PlayerExited`). */
export interface PlayerExited {
  session: PlayerSession;
  /** Code de sortie ; `null` si le processus a été tué. */
  code: number | null;
  /** Vrai si le player s'est terminé avec le code 0. */
  success: boolean;
  duration_ms: number;
  /** Dernières lignes de la sortie d'erreur du player (vide si rien). */
  stderr: string;
}

export const PLAYER_STARTED_EVENT = "player://started";
export const PLAYER_EXITED_EVENT = "player://exited";

/**
 * `player_launch` : lance l'histoire installée `uuid` dans flam-player. Erreurs : `not_found`
 * (non installée), `story_file_missing`, `player_unavailable` (player introuvable ou non
 * exécutable), `player_busy` (un player tourne déjà : un seul à la fois).
 */
export function launchPlayer(uuid: string): Promise<PlayerSession> {
  return invokeCommand<PlayerSession>("player_launch", { uuid });
}

/**
 * `player_launch_folder` : lance directement une histoire `.plain` / `.plain.pk` du dernier
 * scan d'une source dossier (sans installation). Une archive Telmi est refusée (`validation`).
 */
export function launchFolderStory(
  sourceId: string,
  entryId: string,
): Promise<PlayerSession> {
  return invokeCommand<PlayerSession>("player_launch_folder", { sourceId, entryId });
}

/** `player_status` : player en cours d'exécution, ou `null`. */
export function getPlayerStatus(): Promise<PlayerSession | null> {
  return invokeCommand<PlayerSession | null>("player_status");
}

/** Écoute `player://started`. Renvoie la fonction de désabonnement. */
export function onPlayerStarted(
  handler: (event: PlayerSession) => void,
): Promise<UnlistenFn> {
  return listen<PlayerSession>(PLAYER_STARTED_EVENT, (e) => handler(e.payload));
}

/** Écoute `player://exited` (succès, échec ou processus tué). */
export function onPlayerExited(
  handler: (event: PlayerExited) => void,
): Promise<UnlistenFn> {
  return listen<PlayerExited>(PLAYER_EXITED_EVENT, (e) => handler(e.payload));
}
