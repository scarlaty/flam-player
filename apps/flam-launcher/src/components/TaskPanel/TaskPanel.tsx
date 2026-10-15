import { useId, useState } from "react";
import { t } from "../../i18n";
import { Badge, type BadgeTone } from "../Badge/Badge";
import { Icon, type IconName } from "../Icon";
import { IconButton } from "../IconButton/IconButton";
import { ProgressBar } from "../ProgressBar/ProgressBar";
import "./TaskPanel.css";

export type TaskKind = "download" | "convert" | "install";
export type TaskState = "queued" | "running" | "done" | "error" | "cancelled";

export interface Task {
  id: string;
  /** Titre de l'histoire concernée. */
  title: string;
  kind: TaskKind;
  state: TaskState;
  /** Pourcentage 0-100 ; absent pendant `running` = indéterminé. */
  progress?: number;
  /** Étape en cours, affichée sous le titre (ex. « Extraction des images »). */
  phase?: string;
  /** Précision affichée à côté de l'état (message d'erreur, débit…). */
  detail?: string;
}

export interface TaskPanelProps {
  tasks: Task[];
  /** Annuler une tâche en attente ou en cours. */
  onCancel?: (id: string) => void;
  /** Relancer une tâche en échec ou annulée. */
  onRetry?: (id: string) => void;
  /** Retirer une tâche terminée, en échec ou annulée de la liste. */
  onDismiss?: (id: string) => void;
}

const KIND_ICONS: Record<TaskKind, IconName> = {
  download: "download",
  convert: "refresh",
  install: "install",
};

const STATE_TONES: Record<TaskState, BadgeTone> = {
  queued: "neutral",
  running: "info",
  done: "success",
  error: "danger",
  cancelled: "neutral",
};

function isActive(state: TaskState): boolean {
  return state === "queued" || state === "running";
}

/** Texte à annoncer pour les tâches dont l'état a changé (ou nouvelles). */
export function describeChanges(previous: Task[], current: Task[]): string {
  const before = new Map(previous.map((task) => [task.id, task.state]));
  return current
    .filter((task) => before.get(task.id) !== task.state)
    .map((task) =>
      t.tasks.announce(
        task.title,
        t.tasks.kind[task.kind],
        t.tasks.state[task.state],
        task.state === "error" ? task.detail : undefined,
      ),
    )
    .join(" ");
}

/**
 * Panneau des téléchargements, conversions et installations. Les changements d'état
 * (dont les échecs) sont annoncés poliment aux lecteurs d'écran ; la progression ne
 * l'est pas, pour ne pas saturer l'annonce.
 */
export function TaskPanel({ tasks, onCancel, onRetry, onDismiss }: TaskPanelProps) {
  const titleId = useId();
  // Mémorise la liste précédente pour calculer l'annonce pendant le rendu
  // (modèle React « stocker les props précédentes »), sans effet.
  const [previous, setPrevious] = useState(tasks);
  const [announcement, setAnnouncement] = useState("");
  if (previous !== tasks) {
    setPrevious(tasks);
    const changes = describeChanges(previous, tasks);
    if (changes) setAnnouncement(changes);
  }

  return (
    <section className="fl-tasks" aria-labelledby={titleId}>
      <h2 id={titleId} className="fl-tasks__title">
        {t.tasks.title}
      </h2>
      <p className="fl-visually-hidden" role="status">
        {announcement}
      </p>
      {tasks.length === 0 ? (
        <p className="fl-tasks__empty">{t.tasks.empty}</p>
      ) : (
        <ul className="fl-tasks__list">
          {tasks.map((task) => {
            const active = isActive(task.state);
            const retryable = task.state === "error" || task.state === "cancelled";
            return (
              <li key={task.id} className={`fl-task fl-task--${task.state}`}>
                <div className="fl-task__head">
                  <span className="fl-task__icon">
                    <Icon name={KIND_ICONS[task.kind]} size={18} />
                  </span>
                  <div className="fl-task__text">
                    <p className="fl-task__name">{task.title}</p>
                    <p className="fl-task__kind">
                      {t.tasks.kind[task.kind]}
                      {task.phase && ` · ${task.phase}`}
                    </p>
                  </div>
                  <div className="fl-task__actions">
                    {retryable && onRetry && (
                      <IconButton
                        icon="refresh"
                        size="sm"
                        label={t.tasks.retry(task.title)}
                        onClick={() => onRetry(task.id)}
                      />
                    )}
                    {active && onCancel && (
                      <IconButton
                        icon="close"
                        size="sm"
                        label={t.tasks.cancel(task.title)}
                        onClick={() => onCancel(task.id)}
                      />
                    )}
                    {!active && onDismiss && (
                      <IconButton
                        icon="close"
                        size="sm"
                        label={t.tasks.dismiss(task.title)}
                        onClick={() => onDismiss(task.id)}
                      />
                    )}
                  </div>
                </div>
                {task.state === "running" && (
                  <ProgressBar
                    value={task.progress}
                    label={t.tasks.progress(task.title)}
                    showValue
                  />
                )}
                <div className="fl-task__foot">
                  <Badge
                    tone={STATE_TONES[task.state]}
                    icon={task.state === "error" ? "alert" : undefined}
                  >
                    {t.tasks.state[task.state]}
                  </Badge>
                  {task.detail && <span className="fl-task__detail">{task.detail}</span>}
                </div>
              </li>
            );
          })}
        </ul>
      )}
    </section>
  );
}
