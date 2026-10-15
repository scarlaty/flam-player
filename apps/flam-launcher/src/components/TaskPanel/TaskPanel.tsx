import { useId } from "react";
import { t } from "../../i18n";
import { Badge, type BadgeTone } from "../Badge/Badge";
import { Icon, type IconName } from "../Icon";
import { IconButton } from "../IconButton/IconButton";
import { ProgressBar } from "../ProgressBar/ProgressBar";
import "./TaskPanel.css";

export type TaskKind = "download" | "convert" | "install";
export type TaskState = "queued" | "running" | "done" | "error";

export interface Task {
  id: string;
  /** Titre de l'histoire concernée. */
  title: string;
  kind: TaskKind;
  state: TaskState;
  /** Pourcentage 0-100 ; absent pendant `running` = indéterminé. */
  progress?: number;
  /** Précision affichée sous le titre (message d'erreur, débit…). */
  detail?: string;
}

export interface TaskPanelProps {
  tasks: Task[];
  /** Annuler une tâche en attente ou en cours. */
  onCancel?: (id: string) => void;
  /** Retirer une tâche terminée ou en échec de la liste. */
  onDismiss?: (id: string) => void;
}

const KIND_ICONS: Record<TaskKind, IconName> = {
  download: "download",
  convert: "refresh",
  install: "check",
};

const STATE_TONES: Record<TaskState, BadgeTone> = {
  queued: "neutral",
  running: "info",
  done: "success",
  error: "danger",
};

/** Panneau des téléchargements, conversions et installations. */
export function TaskPanel({ tasks, onCancel, onDismiss }: TaskPanelProps) {
  const titleId = useId();

  return (
    <section className="fl-tasks" aria-labelledby={titleId}>
      <h2 id={titleId} className="fl-tasks__title">
        {t.tasks.title}
      </h2>
      {tasks.length === 0 ? (
        <p className="fl-tasks__empty">{t.tasks.empty}</p>
      ) : (
        <ul className="fl-tasks__list">
          {tasks.map((task) => {
            const active = task.state === "queued" || task.state === "running";
            return (
              <li key={task.id} className="fl-task">
                <div className="fl-task__head">
                  <span className="fl-task__icon">
                    <Icon name={KIND_ICONS[task.kind]} size={18} />
                  </span>
                  <div className="fl-task__text">
                    <p className="fl-task__name">{task.title}</p>
                    <p className="fl-task__kind">{t.tasks.kind[task.kind]}</p>
                  </div>
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
                {task.state === "running" && (
                  <ProgressBar
                    value={task.progress}
                    label={t.tasks.progress(task.title)}
                    showValue
                  />
                )}
                <div className="fl-task__foot">
                  <Badge tone={STATE_TONES[task.state]}>
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
