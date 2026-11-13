import { createContext, useContext } from "react";
import type { InstallTask } from "../../api/install";
import type { LibraryEntry } from "../../api/library";
import type { PlayerSession } from "../../api/player";
import type { StoryEntry } from "../../api/story";
import type { Task } from "../../components";
import type { StoryProgress, StoryState } from "../library/storyState";

/**
 * Issue d'une installation demandée avec `onSettled` : `installed` (entrée publiée dans « Mes
 * histoires ») ou `abandoned` (échec, annulation, refus du backend, confirmation refusée, ou
 * nouvelle demande d'installation de la même histoire).
 */
export type InstallOutcome =
  { status: "installed"; entry: Omit<LibraryEntry, "format"> } | { status: "abandoned" };

export interface InstallOptions {
  /** Lance l'histoire dès que l'installation est terminée (« Télécharger et jouer »). */
  thenPlay?: boolean;
  /** Tâche inactive à retirer du panneau une fois la nouvelle tâche acceptée (« Réessayer »). */
  replaces?: string;
  /** Source de la fiche d'où l'action est lancée : on y revient à la fin de la lecture. */
  fromSourceId?: string;
  /**
   * Remplacement déjà confirmé de la même histoire installée dans l'autre format (Telmi ↔
   * STUdio, Q10) : sans lui, `install` demande d'abord confirmation.
   */
  replaceFormat?: boolean;
  /**
   * Appelé une seule fois, à l'issue de cette installation (« Installer et envoyer », #609).
   * Gardé en mémoire seulement : après un rechargement de la fenêtre, il n'est jamais appelé.
   * « Réessayer » après un échec relance une installation simple, sans lui.
   */
  onSettled?: (outcome: InstallOutcome) => void;
  /**
   * Appelé quand la tâche d'installation est créée (confirmations passées, demande acceptée par
   * le backend) ; jamais si elle est refusée ou abandonnée avant.
   */
  onStarted?: () => void;
}

export interface StoriesApi {
  /** Histoires installées (`library_list`), triées par titre. */
  installed: LibraryEntry[];
  /** Change à chaque relecture de la bibliothèque (pour recharger « Mes histoires »). */
  installedRevision: number;
  /**
   * Relit la liste des histoires installées (`library_list`). À appeler quand la bibliothèque
   * change hors des installations : cache vidé, dossier de cache modifié (#570).
   */
  refreshInstalled: () => Promise<void>;
  /** Player en cours d'exécution, ou `null`. */
  session: PlayerSession | null;
  /**
   * Oublie la session : le backend a confirmé qu'aucun player ne tourne (fin de partie
   * manquée), l'écran Lecture ne doit pas rester affiché. Sans effet si la session courante
   * n'est plus celle du processus `pid`.
   */
  forgetSession: (pid: number) => void;
  /** Tâches affichées dans le panneau (actives, terminées, en échec ou annulées). */
  panelTasks: Task[];
  /** Tâche active (en attente ou en cours) pour cette histoire. */
  activeTaskFor: (entry: StoryEntry) => InstallTask | undefined;
  /** Dernière tâche en échec ou annulée de cette histoire, tant qu'elle n'est pas retirée. */
  failedTaskFor: (entry: StoryEntry) => InstallTask | undefined;
  /** Avancement de la tâche active de cette histoire. */
  progressFor: (entry: StoryEntry) => StoryProgress | undefined;
  /**
   * Version installée de cette histoire : même `uuid` **et** même format (une histoire Telmi
   * et son pack STUdio de même `uuid` ne sont pas « la même installation »).
   */
  installedFor: (entry: StoryEntry) => LibraryEntry | undefined;
  /**
   * `entry` propose une autre version que celle installée, ou c'est la source dossier de
   * l'histoire installée et elle a changé depuis l'installation (même à version égale, #288).
   */
  updateAvailable: (entry: StoryEntry) => boolean;
  /**
   * La copie de bibliothèque de l'histoire `uuid` est plus ancienne que sa source dossier
   * (modifiée depuis l'installation) : « Mettre à jour » la réinstalle depuis la source.
   */
  sourceChanged: (uuid: string) => boolean;
  /**
   * Installe l'histoire ; si elle est marquée incompatible (`IncompatibleInstallDialog`) ou
   * si elle remplace la même histoire dans l'autre format (`FormatReplaceDialog`), demande
   * d'abord confirmation. Les erreurs sont signalées par une notification.
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
  /** Une suppression de cette histoire (par UUID) est en cours. */
  removing: (uuid: string) => boolean;
  /**
   * Fiche à rouvrir (fin d'une lecture lancée depuis cette fiche), ou `null`. La vue de la
   * source concernée l'ouvre puis appelle `consumeReopen`.
   */
  reopen: { sourceId: string; entryId: string } | null;
  consumeReopen: () => void;
  cancel: (taskId: string) => void;
  retry: (taskId: string) => void;
  dismiss: (taskId: string) => void;
  /**
   * Ajoute au panneau une tâche créée hors du provider (création d'un pack de podcasts, #217),
   * sans attendre son premier événement.
   */
  trackTask: (task: InstallTask) => void;
  /** Une confirmation d'installation (incompatible, changement de format) est affichée. */
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
