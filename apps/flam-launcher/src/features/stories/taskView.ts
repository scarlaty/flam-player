import type {
  InstallTask,
  TaskError,
  TaskState as InstallState,
} from "../../api/install";
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
  preparing: "download",
  prepared: "convert",
  downloading: "download",
  converting: "convert",
  installing: "install",
  done: "install",
  failed: "install",
  cancelled: "install",
};

const STATES: Record<InstallState, TaskState> = {
  queued: "queued",
  preparing: "running",
  prepared: "queued",
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
    case "preparing": {
      const phase = detail.phase ? (text.preparePhase[detail.phase] ?? null) : null;
      return phase ? text.preparingPhase(phase) : text.preparing;
    }
    case "prepared":
      return text.prepared;
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
    detail: task.error ? taskErrorMessage(task.error) : packDetail(task),
  };
}

/**
 * Message d'une tâche en échec. Un code du convertisseur connu (`unsupported_format` : format que
 * le launcher ne sait pas convertir, sans repli depuis #138), ou un `kind` connu (titres lus,
 * `tts_*`, #462), est traduit en un texte clair et non technique ; sinon le message du backend.
 */
export function taskErrorMessage(error: TaskError): string {
  // Le code vient du convertisseur : texte libre, jamais une clé héritée (`constructor`, `__proto__`).
  const text =
    error.kind === "conversion" && error.code
      ? conversionText(error.code)
      : errorKindText(error.kind);
  return text ?? error.message;
}

/** Texte traduit d'un `kind` d'erreur de tâche, seulement s'il est propre à la table. */
export function errorKindText(kind: string): string | undefined {
  const table = t.errors.task;
  return Object.hasOwn(table, kind) ? table[kind] : undefined;
}

/** Texte traduit d'un code de conversion, seulement s'il est propre à la table (`Object.hasOwn`). */
function conversionText(code: string): string | undefined {
  const table = t.errors.conversion;
  return Object.hasOwn(table, code) ? table[code] : undefined;
}

/** Avertissements des titres lus montrés en entier dans le panneau ; les suivants sont comptés. */
export const MAX_SHOWN_WARNINGS = 3;

/**
 * Pack de podcasts : épisodes retirés (P7) et avertissements des titres lus (#462 : voix de
 * repli, titres restés muets ; textes du backend, sans nom de fichier), signalés dans le panneau
 * des tâches.
 */
function packDetail(task: InstallTask): string | undefined {
  const dropped = task.pack?.dropped ?? [];
  const warnings = task.pack?.warnings ?? [];
  const parts: string[] = [];
  if (dropped.length > 0) parts.push(t.stories.progress.packDropped(dropped));
  if (warnings.length > 0) {
    parts.push(
      t.speech.warnings(
        warnings.slice(0, MAX_SHOWN_WARNINGS),
        Math.max(0, warnings.length - MAX_SHOWN_WARNINGS),
      ),
    );
  }
  return parts.length > 0 ? parts.join(" ") : undefined;
}
