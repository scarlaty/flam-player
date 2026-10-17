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
  /**
   * Vrai si le player est incrusté dans la fenêtre du launcher (écran Lecture) ; faux s'il
   * s'ouvre dans sa propre fenêtre (hors Windows, incrustation impossible ou désactivée).
   */
  embedded: boolean;
}

/** Zone d'accueil du player incrusté, en px CSS relatifs à la page (`getBoundingClientRect`). */
export interface EmbedRect {
  x: number;
  y: number;
  width: number;
  height: number;
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
  /**
   * Raison pour laquelle la progression de l'enfant n'a pas pu être conservée telle quelle
   * (sauvegardes au-delà des limites, mises de côté ; rapatriement en échec), `null` sinon.
   * À afficher : sans cela, la progression est perdue sans que personne le sache.
   */
  saves_error: string | null;
}

export const PLAYER_STARTED_EVENT = "player://started";
export const PLAYER_EXITED_EVENT = "player://exited";
export const PLAYER_UPDATED_EVENT = "player://updated";
export const PLAYER_FOCUS_HOST_EVENT = "player://focus-host";

/**
 * `player_launch` : lance l'histoire installée `uuid` dans flam-player. Erreurs : `not_found`
 * (non installée), `story_file_missing`, `player_unavailable` (player introuvable ou non
 * exécutable), `player_busy` (un player tourne déjà : un seul à la fois).
 * Si le lancement échoue après la préparation (`story_file_missing`, `player_unavailable`), le
 * message peut se terminer par « Information sur la progression : … » (sauvegardes mises de
 * côté) : à afficher tel quel.
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

/**
 * `player_status` : player en cours d'exécution, ou `null` (aussi pendant la préparation d'un
 * lancement ou un vidage du cache : le démarrage est annoncé par `player://started`).
 */
export function getPlayerStatus(): Promise<PlayerSession | null> {
  return invokeCommand<PlayerSession | null>("player_status");
}

/**
 * `player_stop` : demande au player de s'arrêter proprement (il sauvegarde puis se termine ;
 * arrêt forcé après 5 s). La fin est annoncée par `player://exited`. Renvoie faux si aucun
 * player ne tourne.
 */
export function stopPlayer(): Promise<boolean> {
  return invokeCommand<boolean>("player_stop");
}

/**
 * `player_embed_layout` : place la fenêtre du player incrusté sur `rect` (px CSS), ou la cache
 * (`null`). Sans effet si aucun player incrusté ne tourne.
 */
export function setPlayerEmbedLayout(rect: EmbedRect | null): Promise<void> {
  // Un appel n'est envoyé qu'une fois le précédent terminé : les commandes asynchrones ne sont
  // pas garanties de s'exécuter dans l'ordre d'envoi, et le dernier rectangle doit l'emporter.
  const result = layoutQueue.then(() =>
    invokeCommand<void>("player_embed_layout", { rect }),
  );
  layoutQueue = result.catch(() => undefined);
  return result;
}

let layoutQueue: Promise<unknown> = Promise.resolve();

/**
 * `player_embed_focus` : donne le clavier au player incrusté (accès sans souris). Sans effet si
 * aucun player incrusté ne tourne.
 */
export function focusEmbeddedPlayer(): Promise<void> {
  return invokeCommand<void>("player_embed_focus");
}

/**
 * Écoute `player://updated` : la session a changé en cours de partie (le player n'a pas pu
 * s'incruster et s'est ouvert dans sa fenêtre : `embedded` devient faux).
 */
export function onPlayerUpdated(
  handler: (event: PlayerSession) => void,
): Promise<UnlistenFn> {
  return listen<PlayerSession>(PLAYER_UPDATED_EVENT, (e) => handler(e.payload));
}

/**
 * Écoute `player://focus-host` : le player incrusté a rendu le clavier au launcher (touche F6
 * dans le player). L'écran Lecture replace alors le focus sur « Clavier au lecteur ».
 */
export function onPlayerFocusHost(handler: () => void): Promise<UnlistenFn> {
  return listen(PLAYER_FOCUS_HOST_EVENT, () => handler());
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
