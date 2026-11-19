import { useEffect, useLayoutEffect, useRef } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { physicalToCss, type Point } from "./geometry";

/*
 * Dépôt de fichiers depuis l'Explorateur (§ 4.3) : la fenêtre de l'éditeur a
 * `dragDropEnabled: true`, les fichiers arrivent par `onDragDropEvent` de Tauri (chemins
 * absolus, position en pixels physiques). Les autres fenêtres gardent le dépôt natif coupé
 * (#300) : ce module n'est utilisé que par l'éditeur.
 */

/** Événement de dépôt réduit à ce qu'utilise l'éditeur (position en pixels CSS). */
export type FileDragEvent =
  | { type: "enter"; paths: string[]; point: Point }
  | { type: "over"; point: Point }
  | { type: "drop"; paths: string[]; point: Point }
  | { type: "leave" };

/** Abonnement aux dépôts ; rend la fonction de désabonnement. */
export type FileDragSubscribe = (
  handler: (event: FileDragEvent) => void,
) => Promise<() => void>;

/** Abonnement réel : `getCurrentWebview().onDragDropEvent` (rien hors de Tauri). */
export const tauriFileDrag: FileDragSubscribe = async (handler) => {
  try {
    return await getCurrentWebview().onDragDropEvent((event) => {
      const payload = event.payload;
      const ratio = window.devicePixelRatio;
      if (payload.type === "leave") handler({ type: "leave" });
      else if (payload.type === "over")
        handler({ type: "over", point: physicalToCss(payload.position, ratio) });
      else
        handler({
          type: payload.type,
          paths: [...payload.paths],
          point: physicalToCss(payload.position, ratio),
        });
    });
  } catch {
    return () => {};
  }
};

export interface FileDropHandlers {
  /** Survol : fichiers en cours de glissement et position ; `null` : sortie ou fin. */
  onHover: (drag: { paths: string[]; point: Point } | null) => void;
  /** Fichiers lâchés à `point`. */
  onDrop: (paths: string[], point: Point) => void;
}

/** Écoute les dépôts de fichiers sur la fenêtre tant que le composant est monté. */
export function useFileDrop(
  handlers: FileDropHandlers,
  subscribe: FileDragSubscribe = tauriFileDrag,
): void {
  const latest = useRef(handlers);
  useLayoutEffect(() => {
    latest.current = handlers;
  });
  useEffect(() => {
    let paths: string[] = [];
    let unlisten: (() => void) | null = null;
    let alive = true;
    void subscribe((event) => {
      if (!alive) return;
      const current = latest.current;
      switch (event.type) {
        case "enter":
          paths = event.paths;
          current.onHover({ paths, point: event.point });
          break;
        case "over":
          if (paths.length > 0) current.onHover({ paths, point: event.point });
          break;
        case "drop":
          current.onHover(null);
          current.onDrop(event.paths.length > 0 ? event.paths : paths, event.point);
          paths = [];
          break;
        case "leave":
          paths = [];
          current.onHover(null);
          break;
      }
    }).then((stop) => {
      if (alive) unlisten = stop;
      else stop();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, [subscribe]);
}
