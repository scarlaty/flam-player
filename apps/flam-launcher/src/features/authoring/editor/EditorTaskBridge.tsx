import { useCallback, useContext, useEffect } from "react";
import { onEditorRelocated, onEditorTask } from "../../../api/editor";
import { StoriesContext } from "../../stories/StoriesContext";

/** Abonnement à un événement des éditeurs, retiré au démontage. */
function useEditorEvent<T>(
  subscribe: (handler: (value: T) => void) => Promise<() => void>,
  handler: ((value: T) => void) | undefined,
) {
  useEffect(() => {
    if (!handler) return;
    let stop: (() => void) | null = null;
    let alive = true;
    void subscribe(handler).then((unlisten) => {
      if (alive) stop = unlisten;
      else unlisten();
    });
    return () => {
      alive = false;
      stop?.();
    };
  }, [subscribe, handler]);
}

/**
 * Fenêtre principale : une création lancée depuis un éditeur rejoint le panneau des tâches
 * (événement `authoring:task`), comme une création lancée ici ; une histoire qui a retenu un
 * nouveau dossier depuis un éditeur (`authoring:relocated`, lot 9) fait relire Mes histoires.
 */
export function EditorTaskBridge() {
  const stories = useContext(StoriesContext);
  useEditorEvent(onEditorTask, stories?.trackTask);
  const refresh = stories?.refreshInstalled;
  const reread = useCallback(() => void refresh?.(), [refresh]);
  useEditorEvent<void>(onEditorRelocated, refresh ? reread : undefined);
  return null;
}
