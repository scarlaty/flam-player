import { createContext, useContext } from "react";
import type { InstallTask } from "../../api/install";
import type { LibraryEntry } from "../../api/library";
import type { PlayerSession } from "../../api/player";
import type { StoryEntry } from "../../api/story";
import type { Task } from "../../components";
import type { StoryProgress, StoryState } from "../library/storyState";

export interface InstallOptions {
  /** Lance l'histoire dès que l'installation est terminée (« Télécharger et jouer »). */
  thenPlay?: boolean;
  /** Tâche inactive à retirer du panneau une fois la nouvelle tâche acceptée (« Réessayer »). */
  replaces?: string;
  /** Source de la fiche d'où l'action est lancée : on y revient à la fin de la lecture. */
  fromSourceId?: string;
}

export interface StoriesApi {
  /** Histoires installées (`library_list`), triées par titre. */
  installed: LibraryEntry[];
  /** Change à chaque relecture de la bibliothèque (pour recharger « Mes histoires »). */
  installedRevision: number;
  /** Player en cours d'exécution, ou `null`. */
  session: PlayerSession | null;
  /** Tâches affichées dans le panneau (actives, terminées, en échec ou annulées). */
  panelTasks: Task[];
  /** Tâche active (en attente ou en cours) pour cette histoire. */
  activeTaskFor: (entry: StoryEntry) => InstallTask | undefined;
  /** Dernière tâche en échec ou annulée de cette histoire, tant qu'elle n'est pas retirée. */
  failedTaskFor: (entry: StoryEntry) => InstallTask | undefined;
  /** Avancement de la tâche active de cette histoire. */
  progressFor: (entry: StoryEntry) => StoryProgress | undefined;
  /** Version installée de cette histoire (même `uuid`). */
  installedFor: (entry: StoryEntry) => LibraryEntry | undefined;
  /** Une version plus récente que celle installée est proposée par `entry`. */
  updateAvailable: (entry: StoryEntry) => boolean;
  /**
   * Installe l'histoire ; si elle est marquée incompatible, demande d'abord confirmation
   * (`IncompatibleInstallDialog`). Les erreurs sont signalées par une notification.
   */
  install: (entry: StoryEntry, options?: InstallOptions) => void;
  /** Lance une histoire installée. */
  play: (entry: StoryEntry, fromSourceId?: string) => void;
  /** Lance directement un `.plain` / `.plain.pk` d'une source dossier, sans installation. */
  playFromFolder: (entry: StoryEntry, fromSourceId?: string) => void;
  /**
   * Désinstalle l'histoire ; renvoie vrai si elle a été supprimée. `purgeSaves` efface aussi la
   * progression de l'enfant (conservée par défaut).
   */
  remove: (uuid: string, title: string, purgeSaves?: boolean) => Promise<boolean>;
  /**
   * Fiche à rouvrir (fin d'une lecture lancée depuis cette fiche), ou `null`. La vue de la
   * source concernée l'ouvre puis appelle `consumeReopen`.
   */
  reopen: { sourceId: string; entryId: string } | null;
  consumeReopen: () => void;
  cancel: (taskId: string) => void;
  retry: (taskId: string) => void;
  dismiss: (taskId: string) => void;
  /** Une confirmation d'installation (histoire incompatible) est affichée. */
  confirming: boolean;
}

/** Archive à télécharger (catalogue) plutôt que fichier local (dossier). */
export function isRemote(entry: StoryEntry): boolean {
  return /^https?:\/\//i.test(entry.download_url);
}

/** État d'une histoire d'après les installations ; `undefined` : celui de la source. */
export function storyStateOf(
  stories: StoriesApi,
  entry: StoryEntry,
): StoryState | undefined {
  if (stories.activeTaskFor(entry)) return "in_progress";
  if (!stories.installedFor(entry)) return undefined;
  return stories.updateAvailable(entry) ? "update_available" : "installed";
}

export const StoriesContext = createContext<StoriesApi | null>(null);

/** Accès aux installations et au lancement ; nécessite un `StoriesProvider` parent. */
export function useStories(): StoriesApi {
  const api = useContext(StoriesContext);
  if (!api) throw new Error("useStories doit être utilisé sous un StoriesProvider");
  return api;
}
