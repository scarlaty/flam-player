import type { InstallTask, TaskState as InstallState } from "../../api/install";
import type { Task, TaskKind, TaskState } from "../../components";
import { t } from "../../i18n";
import type { StoryProgress } from "../library/storyState";
import { formatBytes } from "../validation";

/** Détail de progression reçu par `install://progress` (hors pourcentage). */
export interface TaskDetail {
  received: number | null;
  total: number | null;
  phase: string | null;
}

export const NO_DETAIL: TaskDetail = { received: null, total: null, phase: null };

/** Vrai tant que la tâche n'est ni terminée, ni en échec, ni annulée. */
export function isActiveState(state: InstallState): boolean {
  return state !== "done" && state !== "failed" && state !== "cancelled";
}

const KINDS: Record<InstallState, TaskKind> = {
  queued: "install",
  downloading: "download",
  converting: "convert",
  installing: "install",
  done: "install",
  failed: "install",
  cancelled: "install",
};

const STATES: Record<InstallState, TaskState> = {
  queued: "queued",
  downloading: "running",
  converting: "running",
  installing: "running",
  done: "done",
  failed: "error",
  cancelled: "cancelled",
};

/** Étape en cours, en toutes lettres (« Conversion : audio », « Téléchargement : 3 Mo sur 12 Mo »). */
export function describePhase(
  state: InstallState,
  detail: TaskDetail = NO_DETAIL,
): string | undefined {
  const text = t.stories.progress;
  switch (state) {
    case "queued":
      return text.queued;
    case "downloading":
      return detail.received !== null && detail.total !== null && detail.total > 0
        ? text.downloadingSize(formatBytes(detail.received), formatBytes(detail.total))
        : text.downloading;
    case "converting": {
      const phase = detail.phase ? (text.phase[detail.phase] ?? detail.phase) : null;
      return phase ? text.convertingPhase(phase) : text.converting;
    }
    case "installing":
      return text.installing;
    default:
      return undefined;
  }
}

/** Avancement d'une tâche active pour la carte de l'histoire (`StoryHooks.progressFor`). */
export function taskProgress(task: InstallTask, detail?: TaskDetail): StoryProgress {
  return {
    value: task.percent === null ? "indeterminate" : task.percent,
    text: describePhase(task.state, detail),
  };
}

/** Tâche d'installation mise au format du `TaskPanel`. */
export function toPanelTask(task: InstallTask, detail?: TaskDetail): Task {
  const state = STATES[task.state];
  return {
    id: task.id,
    title: task.entry.title,
    kind: KINDS[task.state],
    state,
    progress: task.percent === null ? undefined : task.percent,
    phase: state === "running" ? describePhase(task.state, detail) : undefined,
    detail: task.error?.message,
  };
}
