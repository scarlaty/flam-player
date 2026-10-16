import type { InstallTask } from "../../api/install";
import { isActiveState } from "./taskView";

/**
 * Événements d'installation reçus pour une tâche pas encore connue (avant le retour de
 * `install_start`) : mémorisés, puis appliqués et **oubliés** quand la tâche est ajoutée.
 */
export class EarlyEvents {
  private readonly patches = new Map<string, Partial<InstallTask>>();

  get size(): number {
    return this.patches.size;
  }

  /** Mémorise `patch` ; un retour à un état actif après une fin est ignoré. */
  record(taskId: string, patch: Partial<InstallTask>): void {
    const previous = this.patches.get(taskId);
    const stale =
      previous?.state !== undefined &&
      !isActiveState(previous.state) &&
      patch.state !== undefined &&
      isActiveState(patch.state);
    if (!stale) this.patches.set(taskId, { ...previous, ...patch });
  }

  /**
   * Applique les événements mémorisés à `task` et les purge. À appeler hors d'une fonction de
   * mise à jour d'état (elle peut être rejouée).
   */
  take(task: InstallTask): InstallTask {
    const patch = this.patches.get(task.id);
    if (!patch) return task;
    this.patches.delete(task.id);
    return { ...task, ...patch };
  }
}
