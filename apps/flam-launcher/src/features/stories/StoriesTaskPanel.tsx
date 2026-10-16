import { TaskPanel } from "../../components";
import { useStories } from "./StoriesContext";

/** Panneau des tâches d'installation (annuler, réessayer, retirer). */
export function StoriesTaskPanel() {
  const { panelTasks, cancel, retry, dismiss } = useStories();
  return (
    <TaskPanel tasks={panelTasks} onCancel={cancel} onRetry={retry} onDismiss={dismiss} />
  );
}
