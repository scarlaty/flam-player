import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { toAppError } from "../../api/error";
import { minimizeWindow, restoreWindow } from "../../api/window";
import { libraryHash } from "../../router";
import { getMinimizeDuringPlay } from "../settings/preferences";
import {
  cancelInstall,
  listInstalls,
  onInstallDone,
  onInstallError,
  onInstallProgress,
  startInstall,
  type InstallTask,
} from "../../api/install";
import {
  hasUpdate,
  listLibrary,
  removeFromLibrary,
  type LibraryEntry,
} from "../../api/library";
import {
  getPlayerStatus,
  launchFolderStory,
  launchPlayer,
  onPlayerExited,
  onPlayerUpdated,
  onPlayerStarted,
  type PlayerSession,
} from "../../api/player";
import type { StoryEntry } from "../../api/story";
import { useToast } from "../../components";
import { t } from "../../i18n";
import { useCompat } from "../compat/CompatContext";
import {
  IncompatibleInstallDialog,
  needsInstallWarning,
} from "../compat/IncompatibleInstallDialog";
import { EarlyEvents } from "./earlyEvents";
import { StoriesContext, type InstallOptions, type StoriesApi } from "./StoriesContext";
import {
  isActiveState,
  NO_DETAIL,
  taskProgress,
  toPanelTask,
  type TaskDetail,
} from "./taskView";

/** Abonne `subscribe` pour la durée de vie du composant, même si la promesse se résout tard. */
function useListener<T>(
  subscribe: (handler: (event: T) => void) => Promise<() => void>,
  handler: (event: T) => void,
) {
  const latest = useRef(handler);
  useEffect(() => {
    latest.current = handler;
  });
  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | undefined;
    subscribe((event) => latest.current(event))
      .then((off) => {
        if (disposed) off();
        else unlisten = off;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [subscribe]);
}

/** Fiche d'une histoire : source qui la porte et identifiant de l'entrée. */
export interface FicheTarget {
  sourceId: string;
  entryId: string;
}

/** Délai avant le retrait automatique d'une installation réussie du panneau des tâches. */
export const DONE_TASK_TTL_MS = 8000;

interface PendingInstall {
  entry: StoryEntry;
  options: InstallOptions;
}

/**
 * Tient les installations (file de tâches, progression), les histoires installées et le player
 * en cours, et expose les actions Installer / Jouer / Mettre à jour / Supprimer / Annuler /
 * Réessayer. L'installation d'une histoire incompatible passe par `IncompatibleInstallDialog`.
 */
export function StoriesProvider({ children }: { children: ReactNode }) {
  const toast = useToast();
  const compat = useCompat();
  const [installed, setInstalled] = useState<LibraryEntry[]>([]);
  const [installedRevision, setInstalledRevision] = useState(0);
  const [tasks, setTasks] = useState<InstallTask[]>([]);
  const [details, setDetails] = useState<Record<string, TaskDetail>>({});
  const [dismissed, setDismissed] = useState<ReadonlySet<string>>(new Set());
  const [session, setSession] = useState<PlayerSession | null>(null);
  const [pending, setPending] = useState<PendingInstall | null>(null);
  // Fiche à rouvrir à la fin de la lecture (voir `returnTo`).
  const [reopen, setReopen] = useState<FicheTarget | null>(null);
  // Fiche d'où une lecture a été demandée, par identifiant de session (`uuid` d'une histoire
  // installée, `entry_id` d'un fichier de dossier) : on y revient à la fin de la lecture,
  // même si l'utilisateur a changé d'écran entre-temps.
  const returnTo = useRef(new Map<string, FicheTarget>());
  // La fenêtre est réduite par cette lecture : elle est rétablie à sa fin. Les opérations sont
  // chaînées pour que le rétablissement ne passe jamais avant une réduction encore en cours.
  const minimized = useRef(false);
  const windowOps = useRef<Promise<unknown>>(Promise.resolve());
  // `uuid` des histoires à lancer à la fin de leur installation ; conservé après un échec pour
  // que « Réessayer » reprenne « Télécharger et jouer ».
  const playAfter = useRef(new Set<string>());
  // Événements reçus pour une tâche pas encore connue (avant le retour de `install_start`) :
  // appliqués dès que la tâche est ajoutée à la liste.
  const early = useRef(new EarlyEvents());
  const tasksRef = useRef(tasks);
  useEffect(() => {
    tasksRef.current = tasks;
  });
  const compatReload = compat.reload;

  const refreshInstalled = useCallback(async () => {
    try {
      setInstalled(await listLibrary());
      setInstalledRevision((n) => n + 1);
    } catch (caught) {
      toast.show({ message: toAppError(caught).message, tone: "danger" });
    }
  }, [toast]);

  const refreshTasks = useCallback(async () => {
    try {
      // Événements mémorisés appliqués (et purgés) ici, hors de la fonction de mise à jour.
      const list = (await listInstalls()).map((task) => early.current.take(task));
      setTasks((current) => {
        // Les événements font foi pour les tâches déjà connues : le cliché du backend ne
        // ramène jamais une tâche à un état antérieur.
        const known = new Map(current.map((task) => [task.id, task]));
        return list.map((task) => known.get(task.id) ?? task);
      });
    } catch {
      // La liste des tâches est un confort : les événements suffisent à suivre l'avancement.
    }
  }, []);

  useEffect(() => {
    // Chargement initial depuis le backend.
    void refreshTasks();
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refreshInstalled();
    getPlayerStatus()
      // Un `player://started` déjà reçu l'emporte sur ce cliché.
      .then((status) => setSession((current) => current ?? status))
      .catch(() => undefined);
  }, [refreshTasks, refreshInstalled]);

  const patchTask = useCallback(
    (taskId: string, patch: Partial<InstallTask>) => {
      setTasks((current) =>
        current.map((task) => {
          if (task.id !== taskId) return task;
          // Un événement de progression tardif ne rouvre pas une tâche terminée.
          if (!isActiveState(task.state) && patch.state && isActiveState(patch.state)) {
            return task;
          }
          return { ...task, ...patch };
        }),
      );
      // Événement reçu avant le retour de `install_start`, ou tâche reprise au démarrage.
      if (!tasksRef.current.some((task) => task.id === taskId)) {
        early.current.record(taskId, patch);
        void refreshTasks();
      }
    },
    [refreshTasks],
  );

  // Pose (ou, sans `target`, retire) le retour à la fiche de la clé `key` pour un lancement et
  // renvoie de quoi défaire cet appel seul : un lancement refusé (`player_busy`) ne doit pas
  // effacer le retour de la lecture en cours.
  const claimReturn = useCallback((key: string, target: FicheTarget | undefined) => {
    const previous = returnTo.current.get(key);
    if (target) returnTo.current.set(key, target);
    else returnTo.current.delete(key);
    return () => {
      if (returnTo.current.get(key) !== target) return;
      if (previous) returnTo.current.set(key, previous);
      else returnTo.current.delete(key);
    };
  }, []);

  const launch = useCallback(
    async (start: () => Promise<PlayerSession>, rollback?: () => void) => {
      try {
        // La session vient de `player://started` : la réponse peut arriver après
        // `player://exited` et ne doit pas ressusciter un player déjà terminé.
        await start();
      } catch (caught) {
        rollback?.();
        toast.show({ message: toAppError(caught).message, tone: "danger" });
      }
    },
    [toast],
  );

  useListener(
    onInstallProgress,
    useCallback(
      (event) => {
        patchTask(event.task_id, { state: event.state, percent: event.percent });
        setDetails((all) => ({
          ...all,
          [event.task_id]: {
            received: event.received,
            total: event.total,
            phase: event.phase,
          },
        }));
      },
      [patchTask],
    ),
  );
  useListener(
    onInstallDone,
    useCallback(
      (event) => {
        patchTask(event.task_id, { state: "done", percent: 100, error: null });
        compatReload();
        void refreshInstalled();
        const { uuid, title } = event.entry;
        toast.show({
          message:
            event.warnings > 0
              ? t.stories.toast.installedWarnings(title, event.warnings)
              : t.stories.toast.installed(title),
          tone: "success",
        });
        if (playAfter.current.delete(uuid)) {
          void launch(
            () => launchPlayer(uuid),
            () => returnTo.current.delete(uuid),
          );
        }
      },
      [patchTask, compatReload, refreshInstalled, toast, launch],
    ),
  );
  useListener(
    onInstallError,
    useCallback(
      (event) => {
        patchTask(event.task_id, { state: event.state, error: event.error });
        // Une conversion en échec est enregistrée dans le rapport de compatibilité local.
        compatReload();
      },
      [patchTask, compatReload],
    ),
  );
  useListener(
    onPlayerStarted,
    useCallback((started: PlayerSession) => {
      setSession(started);
      // Player incrusté : la fenêtre du launcher le contient, elle n'est jamais réduite.
      if (!started.embedded && getMinimizeDuringPlay()) {
        minimized.current = true;
        windowOps.current = windowOps.current.then(minimizeWindow);
      }
    }, []),
  );
  useListener(
    onPlayerUpdated,
    useCallback((updated: PlayerSession) => {
      // Même partie (pid), le player s'est ouvert dans sa fenêtre : écran Lecture quitté.
      setSession((current) =>
        current && current.pid === updated.pid ? updated : current,
      );
    }, []),
  );
  useListener(
    onPlayerExited,
    useCallback(
      (event) => {
        setSession(null);
        const title = event.session.title;
        if (minimized.current) {
          minimized.current = false;
          windowOps.current = windowOps.current.then(restoreWindow);
        }
        const target = returnTo.current.get(event.session.uuid);
        returnTo.current.delete(event.session.uuid);
        // Une fiche ou un dialogue déjà ouvert (autre histoire, confirmation…) n'est pas
        // remplacé : l'utilisateur est en train de s'en servir.
        if (target && !document.querySelector('[aria-modal="true"]')) {
          setReopen(target);
          window.location.hash = libraryHash(target.sourceId);
        }
        if (event.success) {
          toast.show({ message: t.stories.toast.finished(title) });
        } else {
          toast.show({
            message: t.stories.toast.playerFailed(title, event.stderr.trim()),
            tone: "danger",
          });
        }
        if (event.saves_error) {
          toast.show({
            message: t.stories.toast.savesLost(title, event.saves_error),
            tone: "danger",
          });
        }
      },
      [toast],
    ),
  );

  const start = useCallback(
    async (entry: StoryEntry, options: InstallOptions) => {
      if (options.thenPlay) {
        playAfter.current.add(entry.uuid);
        if (options.fromSourceId !== undefined) {
          returnTo.current.set(entry.uuid, {
            sourceId: options.fromSourceId,
            entryId: entry.entry_id,
          });
        }
      } else {
        playAfter.current.delete(entry.uuid);
        returnTo.current.delete(entry.uuid);
      }
      try {
        const task = await startInstall(entry.source_id, entry.entry_id);
        // La nouvelle tâche remplace les tâches inactives (échec, annulation) de la même histoire.
        const stale = tasksRef.current
          .filter(
            (other) =>
              other.id !== task.id &&
              other.entry.uuid === entry.uuid &&
              !isActiveState(other.state),
          )
          .map((other) => other.id);
        if (options.replaces) stale.push(options.replaces);
        // Une tâche déjà connue (événement arrivé avant ce retour) garde son état le plus récent.
        const added = early.current.take(task);
        setTasks((current) =>
          current.some((other) => other.id === task.id) ? current : [added, ...current],
        );
        setDismissed((all) => {
          const next = new Set(all);
          next.delete(task.id);
          for (const id of stale) next.add(id);
          return next;
        });
      } catch (caught) {
        const error = toAppError(caught);
        // Relance d'une tâche dont l'entrée a disparu du catalogue ou du dernier scan : la
        // tâche en échec reste affichée, « Réessayer » pourra servir après un rafraîchissement.
        const gone = error.kind === "not_found";
        if (!options.replaces) {
          playAfter.current.delete(entry.uuid);
          returnTo.current.delete(entry.uuid);
        }
        toast.show({
          message: gone ? t.stories.toast.entryGone(entry.title) : error.message,
          tone: "danger",
        });
      }
    },
    [toast],
  );

  const compatFor = compat.compatFor;
  const install = useCallback(
    (entry: StoryEntry, options: InstallOptions = {}) => {
      if (needsInstallWarning(compatFor(entry))) {
        setPending({ entry, options });
        return;
      }
      void start(entry, options);
    },
    [compatFor, start],
  );

  const cancel = useCallback(
    (taskId: string) => {
      const task = tasksRef.current.find((other) => other.id === taskId);
      if (task) {
        playAfter.current.delete(task.entry.uuid);
        returnTo.current.delete(task.entry.uuid);
      }
      cancelInstall(taskId).catch((caught: unknown) =>
        toast.show({ message: toAppError(caught).message, tone: "danger" }),
      );
    },
    [toast],
  );

  const dismiss = useCallback((taskId: string) => {
    const task = tasksRef.current.find((other) => other.id === taskId);
    // Retirer à la main une ligne terminée ne retire pas le lancement ni le retour à la fiche
    // d'une lecture en cours ; seul l'abandon d'un échec ou d'une annulation les retire.
    if (task && (task.state === "failed" || task.state === "cancelled")) {
      playAfter.current.delete(task.entry.uuid);
      returnTo.current.delete(task.entry.uuid);
    }
    setDismissed((all) => new Set(all).add(taskId));
  }, []);

  // Une installation réussie n'a plus rien à proposer : sa ligne se retire d'elle-même.
  // Les échecs et annulations restent (message, « Réessayer »).
  const dismissTimers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  useEffect(() => {
    const timers = dismissTimers.current;
    for (const task of tasks) {
      if (task.state !== "done" || timers.has(task.id)) continue;
      timers.set(
        task.id,
        setTimeout(() => {
          timers.delete(task.id);
          setDismissed((all) => new Set(all).add(task.id));
        }, DONE_TASK_TTL_MS),
      );
    }
  }, [tasks]);
  useEffect(() => {
    const timers = dismissTimers.current;
    const ops = windowOps;
    const reduced = minimized;
    return () => {
      for (const timer of timers.values()) clearTimeout(timer);
      timers.clear();
      // Filet de sécurité : la fenêtre réduite pour une lecture ne reste pas réduite si le
      // provider disparaît avant la fin du player.
      if (reduced.current) {
        reduced.current = false;
        ops.current = ops.current.then(restoreWindow);
      }
    };
  }, []);

  const retry = useCallback(
    (taskId: string) => {
      const task = tasksRef.current.find((other) => other.id === taskId);
      if (!task) return;
      // L'ancienne tâche n'est retirée qu'une fois la nouvelle acceptée (pas si l'utilisateur
      // renonce à l'installation d'une histoire incompatible).
      install(task.entry, {
        thenPlay: playAfter.current.has(task.entry.uuid),
        replaces: taskId,
      });
    },
    [install],
  );

  const remove = useCallback(
    async (uuid: string, title: string, purgeSaves = false) => {
      try {
        await removeFromLibrary(uuid, purgeSaves);
      } catch (caught) {
        toast.show({ message: toAppError(caught).message, tone: "danger" });
        return false;
      }
      toast.show({ message: t.stories.toast.removed(title) });
      await refreshInstalled();
      return true;
    },
    [toast, refreshInstalled],
  );

  const api = useMemo<StoriesApi>(() => {
    const visible = tasks.filter((task) => !dismissed.has(task.id));
    const activeFor = (entry: StoryEntry) =>
      tasks.find((task) => task.entry.uuid === entry.uuid && isActiveState(task.state));
    const installedFor = (entry: StoryEntry) =>
      installed.find((item) => item.uuid === entry.uuid);
    return {
      installed,
      installedRevision,
      session,
      forgetSession: (pid) =>
        setSession((current) => (current && current.pid === pid ? null : current)),
      panelTasks: visible.map((task) => toPanelTask(task, details[task.id])),
      activeTaskFor: activeFor,
      failedTaskFor: (entry) =>
        visible.find(
          (task) =>
            task.entry.uuid === entry.uuid &&
            (task.state === "failed" || task.state === "cancelled"),
        ),
      progressFor: (entry) => {
        const task = activeFor(entry);
        return task ? taskProgress(task, details[task.id] ?? NO_DETAIL) : undefined;
      },
      installedFor,
      updateAvailable: (entry) => hasUpdate(installedFor(entry), entry),
      install,
      play: (entry, fromSourceId) => {
        const target =
          fromSourceId === undefined
            ? undefined
            : { sourceId: fromSourceId, entryId: entry.entry_id };
        void launch(() => launchPlayer(entry.uuid), claimReturn(entry.uuid, target));
      },
      playFromFolder: (entry, fromSourceId) => {
        const target =
          fromSourceId === undefined
            ? undefined
            : { sourceId: fromSourceId, entryId: entry.entry_id };
        void launch(
          () => launchFolderStory(entry.source_id, entry.entry_id),
          claimReturn(entry.entry_id, target),
        );
      },
      reopen,
      consumeReopen: () => setReopen(null),
      remove,
      cancel,
      retry,
      dismiss,
      confirming: pending !== null,
    };
  }, [
    tasks,
    dismissed,
    details,
    installed,
    installedRevision,
    session,
    reopen,
    install,
    launch,
    claimReturn,
    remove,
    cancel,
    retry,
    dismiss,
    pending,
  ]);

  const info = pending ? compatFor(pending.entry) : undefined;
  return (
    <StoriesContext.Provider value={api}>
      {children}
      {pending && info && (
        <IncompatibleInstallDialog
          open
          story={pending.entry.title}
          info={info}
          onCancel={() => setPending(null)}
          onConfirm={() => {
            const { entry, options } = pending;
            setPending(null);
            void start(entry, options);
          }}
        />
      )}
    </StoriesContext.Provider>
  );
}
