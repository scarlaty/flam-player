import { useContext, useEffect } from "react";
import { onEditorTask } from "../../../api/editor";
import { StoriesContext } from "../../stories/StoriesContext";

/**
 * Fenêtre principale : une création lancée depuis un éditeur rejoint le panneau des tâches
 * (événement `authoring:task`), comme une création lancée ici.
 */
export function EditorTaskBridge() {
  const stories = useContext(StoriesContext);
  const track = stories?.trackTask;

  useEffect(() => {
    if (!track) return;
    let stop: (() => void) | null = null;
    let alive = true;
    void onEditorTask(track).then((unlisten) => {
      if (alive) stop = unlisten;
      else unlisten();
    });
    return () => {
      alive = false;
      stop?.();
    };
  }, [track]);

  return null;
}
