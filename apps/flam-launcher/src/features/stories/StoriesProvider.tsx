import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { toAppError } from "../../api/error";
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
  // `uuid` des histoires à lancer à la fin de leur installation ; conservé après un échec pour
  // que « Réessayer » reprenne « Télécharger et jouer ».
  const playAfter = useRef(new Set<string>());
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
      const list = await listInstalls();
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
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refreshTasks();
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
      if (!tasksRef.current.some((task) => task.id === taskId)) void refreshTasks();
    },
    [refreshTasks],
  );

  const launch = useCallback(
    async (start: () => Promise<PlayerSession>) => {
      try {
        setSession(await start());
      } catch (caught) {
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
          void launch(() => launchPlayer(uuid));
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
  useListener(onPlayerStarted, setSession);
  useListener(
    onPlayerExited,
    useCallback(
      (event) => {
        setSession(null);
        const title = event.session.title;
        if (event.success) {
          toast.show({ message: t.stories.toast.finished(title) });
        } else {
          toast.show({
            message: t.stories.toast.playerFailed(title, event.stderr.trim()),
            tone: "danger",
          });
        }
      },
      [toast],
    ),
  );

  const start = useCallback(
    async (entry: StoryEntry, options: InstallOptions) => {
      if (options.thenPlay) playAfter.current.add(entry.uuid);
      else playAfter.current.delete(entry.uuid);
      try {
        const task = await startInstall(entry.source_id, entry.entry_id);
        // Une tâche déjà connue (événement arrivé avant ce retour) garde son état le plus récent.
        setTasks((current) =>
          current.some((other) => other.id === task.id) ? current : [task, ...current],
        );
        setDismissed((all) => {
          if (!all.has(task.id)) return all;
          const next = new Set(all);
          next.delete(task.id);
          return next;
        });
      } catch (caught) {
        playAfter.current.delete(entry.uuid);
        toast.show({ message: toAppError(caught).message, tone: "danger" });
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
      if (task) playAfter.current.delete(task.entry.uuid);
      cancelInstall(taskId).catch((caught: unknown) =>
        toast.show({ message: toAppError(caught).message, tone: "danger" }),
      );
    },
    [toast],
  );

  const dismiss = useCallback((taskId: string) => {
    const task = tasksRef.current.find((other) => other.id === taskId);
    if (task && !isActiveState(task.state)) playAfter.current.delete(task.entry.uuid);
    setDismissed((all) => new Set(all).add(taskId));
  }, []);

  const retry = useCallback(
    (taskId: string) => {
      const task = tasksRef.current.find((other) => other.id === taskId);
      if (!task) return;
      setDismissed((all) => new Set(all).add(taskId));
      install(task.entry, { thenPlay: playAfter.current.has(task.entry.uuid) });
    },
    [install],
  );

  const remove = useCallback(
    async (uuid: string, title: string) => {
      try {
        await removeFromLibrary(uuid);
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
      play: (entry) => void launch(() => launchPlayer(entry.uuid)),
      playFromFolder: (entry) =>
        void launch(() => launchFolderStory(entry.source_id, entry.entry_id)),
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
    install,
    launch,
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
