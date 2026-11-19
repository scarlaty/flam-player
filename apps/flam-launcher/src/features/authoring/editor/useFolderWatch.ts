import { useEffect, useRef, useState } from "react";
import {
  editorView,
  onFolderChanged,
  onFolderMissing,
  type EditorView,
} from "../../../api/editor";
import { t } from "../../../i18n";

export interface FolderWatchOptions {
  /** Vrai pendant une opération de l'éditeur : la relecture attend sa fin (lot 9). */
  busy: boolean;
  /** Vue relue après un changement fait ailleurs (Explorateur, autre programme). */
  onView: (view: EditorView) => void;
  /** Message discret à annoncer. */
  onNotice: (message: string) => void;
}

export interface FolderWatch {
  /** Le dossier a disparu (supprimé, renommé, clé USB retirée) : éditeur en lecture seule. */
  missing: boolean;
}

/**
 * Surveillance du dossier de l'éditeur (`docs/creation-dossier.md` § 4.5) : à chaque
 * changement fait ailleurs (événement `authoring:folder-changed`, envoyé à cette seule fenêtre
 * par `editor_watch.rs`), la vue est relue et transmise à `onView`, avec un message discret.
 *
 * - Pendant une opération de l'éditeur, la relecture attend sa fin ; une relecture en cours
 *   quand une opération commence est jetée puis refaite (elle pourrait être antérieure à la
 *   vue rendue par l'opération).
 * - Plusieurs changements rapprochés ne donnent qu'une relecture.
 * - `authoring:folder-missing` : `missing` passe à vrai ; si le dossier revient (clé
 *   rebranchée), le backend signale un changement et une relecture réussie le remet à faux.
 */
export function useFolderWatch({
  busy,
  onView,
  onNotice,
}: FolderWatchOptions): FolderWatch {
  const [missing, setMissing] = useState(false);
  /** Changements signalés et pas encore relus (0 : vue à jour). */
  const [pending, setPending] = useState(0);
  const missingRef = useRef(false);
  const latest = useRef({ onView, onNotice });
  useEffect(() => {
    latest.current = { onView, onNotice };
  });

  useEffect(() => {
    let disposed = false;
    const stops: (() => void)[] = [];
    const keep = (stop: () => void) => {
      if (disposed) stop();
      else stops.push(stop);
    };
    void onFolderChanged(() => setPending((n) => n + 1)).then(keep);
    void onFolderMissing(() => {
      missingRef.current = true;
      setMissing(true);
      setPending(0);
      latest.current.onNotice(t.authoringEditor.watch.missing);
    }).then(keep);
    return () => {
      disposed = true;
      stops.forEach((stop) => stop());
    };
  }, []);

  useEffect(() => {
    if (pending === 0 || busy) return;
    // Jeté si une opération commence ou si un autre changement arrive entre-temps : l'effet
    // est alors relancé et relit de nouveau.
    let current = true;
    editorView().then(
      (view) => {
        if (!current) return;
        const back = missingRef.current;
        missingRef.current = false;
        setMissing(false);
        setPending(0);
        latest.current.onView(view);
        const text = t.authoringEditor.watch;
        latest.current.onNotice(back ? text.back : text.changed);
      },
      () => {
        // Dossier illisible à l'instant (il disparaît ?) : l'événement dédié suivra.
        if (current) setPending(0);
      },
    );
    return () => {
      current = false;
    };
  }, [pending, busy]);

  return { missing };
}
