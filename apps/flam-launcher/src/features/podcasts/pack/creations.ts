/**
 * Créations de pack en cours, par révision du brouillon (#429). Le brouillon n'est effacé qu'à la
 * publication : tant que la tâche tourne, « Créer l'histoire » relancerait un pack identique.
 * L'état vit hors du composant, car l'éditeur peut être quitté puis rouvert pendant la création ;
 * l'activité de la tâche est lue dans le panneau des tâches (`StoriesProvider`).
 */

/** Révisions dont l'appel `pack_create` est en cours (la tâche n'existe pas encore). */
const inFlight = new Map<number, number>();

/** Tâches créées à partir d'une révision du brouillon : identifiant de tâche -> révision. */
const tasks = new Map<string, number>();

/** Tâches vues actives au moins une fois : seules celles-là sont purgées une fois terminées. */
const seenActive = new Set<string>();

const listeners = new Set<() => void>();
let version = 0;
const notify = () => {
  version += 1;
  listeners.forEach((listener) => listener());
};

/** Abonnement aux changements des envois en cours (pour `useSyncExternalStore`). */
export function subscribeCreations(listener: () => void): () => void {
  listeners.add(listener);
  return () => void listeners.delete(listener);
}

/** Change à chaque début ou fin d'envoi : l'éditeur rouvert se met alors à jour. */
export const creationsVersion = () => version;

/** Marque l'envoi de la révision `revision` ; renvoie la fonction qui lève la marque. */
export function beginCreation(revision: number): () => void {
  inFlight.set(revision, (inFlight.get(revision) ?? 0) + 1);
  notify();
  return () => {
    const left = (inFlight.get(revision) ?? 1) - 1;
    if (left <= 0) inFlight.delete(revision);
    else inFlight.set(revision, left);
    notify();
  };
}

/** Retient la tâche créée à partir de la révision `revision`. */
export function rememberTask(taskId: string, revision: number): void {
  tasks.set(taskId, revision);
}

/**
 * Une création issue de cette révision est en cours : envoi en cours, ou tâche encore active
 * d'après `isActive` (tâches du panneau ; absentes hors de l'application, où seul l'envoi compte).
 */
export function isCreating(revision: number, isActive: (taskId: string) => boolean) {
  if (inFlight.has(revision)) return true;
  for (const [taskId, taskRevision] of tasks) {
    if (taskRevision === revision && isActive(taskId)) return true;
  }
  return false;
}

/**
 * Purge les tâches terminées (#435) : une tâche vue active puis devenue inactive ne bloque plus
 * rien. Une tâche jamais vue active (pas encore dans le panneau) est conservée.
 */
export function forgetFinishedTasks(isActive: (taskId: string) => boolean): void {
  for (const taskId of [...tasks.keys()]) {
    if (isActive(taskId)) seenActive.add(taskId);
    else if (seenActive.delete(taskId)) tasks.delete(taskId);
  }
}

/** Nombre de tâches retenues (tests). */
export const rememberedTaskCount = () => tasks.size;

/** Remise à zéro (tests). */
export function resetCreations(): void {
  inFlight.clear();
  tasks.clear();
  seenActive.clear();
  notify();
}
