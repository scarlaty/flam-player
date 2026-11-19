import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
} from "react";
import type { EditorNode } from "../../../../api/editor";
import { findNode } from "../model";
import {
  edgeScroll,
  pastThreshold,
  spotAt,
  type Point,
  type RowBox,
  type Spot,
} from "./geometry";
import { planTreeMove, type TreeMovePlan } from "./plan";

/*
 * Réordonner et déplacer dans l'arbre avec le pointeur (§ 4.3) : événements de pointeur, pas
 * de glisser-déposer HTML5 (incompatible avec le dépôt natif de fichiers sous Windows). On
 * saisit la poignée d'une ligne ; le glissement commence au-delà de quelques pixels ; un aperçu
 * suit le pointeur, un trait montre la place (ou le menu surligné : « dans ») ; l'arbre défile
 * près de ses bords ; Échap annule.
 */

/** Attribut de la poignée d'une ligne. */
export const GRIP_ATTR = "data-dnd-grip";
/** Attribut d'une ligne de l'arbre (valeur : id de l'élément). */
export const ROW_ATTR = "data-dnd-row";
/** Attribut « menu » d'une ligne (`"true"`). */
export const ROW_MENU_ATTR = "data-dnd-menu";

/** Glissement en cours (pour l'affichage). */
export interface TreeDragState {
  id: string;
  title: string;
  spot: Spot | null;
  plan: TreeMovePlan;
}

export interface TreeDragOptions {
  /** Élément de l'arbre (`role="tree"`). */
  getTree: () => HTMLElement | null;
  root: EditorNode;
  isOpen: (id: string) => boolean;
  disabled: boolean;
  onStart: (id: string, title: string) => void;
  onCancel: (id: string, title: string) => void;
  onDrop: (id: string, title: string, plan: TreeMovePlan) => void;
}

export interface TreeDrag {
  onPointerDown: (event: ReactPointerEvent<HTMLElement>) => void;
  drag: TreeDragState | null;
  ghost: ReactNode;
}

/** Lignes de l'arbre mesurées, dans l'ordre affiché. */
export function measureRows(tree: HTMLElement): RowBox[] {
  return Array.from(tree.querySelectorAll<HTMLElement>(`[${ROW_ATTR}]`)).map((el) => {
    const r = el.getBoundingClientRect();
    return {
      id: el.getAttribute(ROW_ATTR) ?? "",
      menu: el.getAttribute(ROW_MENU_ATTR) === "true",
      rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
    };
  });
}

/** Premier parent qui défile verticalement. */
function scrollerOf(el: HTMLElement): HTMLElement | null {
  for (let node = el.parentElement; node; node = node.parentElement) {
    const overflow = window.getComputedStyle(node).overflowY;
    if (overflow === "auto" || overflow === "scroll") return node;
  }
  return null;
}

interface Session {
  pointerId: number;
  id: string;
  title: string;
  start: Point;
  last: Point;
  started: boolean;
  scrollTimer: number | undefined;
  cleanup: () => void;
}

export function useTreeDrag(options: TreeDragOptions): TreeDrag {
  const latest = useRef(options);
  useEffect(() => {
    latest.current = options;
  });
  const [drag, setDrag] = useState<TreeDragState | null>(null);
  const session = useRef<Session | null>(null);
  const ghostRef = useRef<HTMLDivElement | null>(null);
  const ghostAt = useRef<Point>({ x: 0, y: 0 });

  const placeGhost = useCallback((point: Point) => {
    ghostAt.current = point;
    const ghost = ghostRef.current;
    if (ghost) ghost.style.transform = `translate(${point.x + 12}px, ${point.y + 8}px)`;
  }, []);
  // L'aperçu est placé hors du rendu (il suit chaque mouvement sans re-rendre l'arbre).
  const attachGhost = useCallback(
    (el: HTMLDivElement | null) => {
      ghostRef.current = el;
      if (el) placeGhost(ghostAt.current);
    },
    [placeGhost],
  );

  /** Recalcule la place visée au point `point`. */
  const update = useCallback((point: Point) => {
    const s = session.current;
    const tree = latest.current.getTree();
    if (!s || !tree) return;
    const { root, isOpen } = latest.current;
    const spot = spotAt(measureRows(tree), point.y);
    const plan = planTreeMove(root, s.id, spot, isOpen);
    setDrag((current) =>
      current &&
      current.spot?.id === spot?.id &&
      current.spot?.slot === spot?.slot &&
      current.plan.kind === plan.kind
        ? current
        : { id: s.id, title: s.title, spot, plan },
    );
  }, []);

  const finish = useCallback((commit: boolean) => {
    const s = session.current;
    if (!s) return;
    session.current = null;
    window.clearInterval(s.scrollTimer);
    s.cleanup();
    setDrag(null);
    if (!s.started) return;
    // Le clic qui suit le relâchement ne doit pas choisir la ligne survolée.
    const swallow = (event: MouseEvent) => {
      event.stopPropagation();
      event.preventDefault();
    };
    window.addEventListener("click", swallow, { capture: true, once: true });
    window.setTimeout(
      () => window.removeEventListener("click", swallow, { capture: true }),
      0,
    );
    const { root, isOpen, getTree } = latest.current;
    if (!commit) {
      latest.current.onCancel(s.id, s.title);
      return;
    }
    const tree = getTree();
    const spot = tree ? spotAt(measureRows(tree), s.last.y) : null;
    latest.current.onDrop(s.id, s.title, planTreeMove(root, s.id, spot, isOpen));
  }, []);

  // Démonté pendant un glissement : écouteurs retirés.
  useEffect(
    () => () => {
      const s = session.current;
      if (!s) return;
      session.current = null;
      window.clearInterval(s.scrollTimer);
      s.cleanup();
    },
    [],
  );

  const onPointerDown = useCallback(
    (event: ReactPointerEvent<HTMLElement>) => {
      if (event.button !== 0 || latest.current.disabled || session.current) return;
      const target = event.target as HTMLElement | null;
      const grip = target?.closest?.(`[${GRIP_ATTR}]`);
      const row = grip?.closest?.(`[${ROW_ATTR}]`);
      const id = row?.getAttribute(ROW_ATTR);
      if (!grip || id === null || id === undefined || id === "") return;
      const node = findNode(latest.current.root, id);
      if (!node) return;
      // Pas de sélection de texte ni de glisser natif depuis la poignée.
      event.preventDefault();
      const start = { x: event.clientX, y: event.clientY };
      const pointerId = event.pointerId;
      try {
        (grip as HTMLElement).setPointerCapture?.(pointerId);
      } catch {
        // Capture impossible : les écouteurs de la fenêtre suffisent.
      }

      const onMove = (e: PointerEvent) => {
        const s = session.current;
        if (!s || (e.pointerId !== undefined && e.pointerId !== s.pointerId)) return;
        const point = { x: e.clientX, y: e.clientY };
        s.last = point;
        if (!s.started) {
          if (!pastThreshold(s.start, point)) return;
          s.started = true;
          ghostAt.current = point;
          latest.current.onStart(s.id, s.title);
          setDrag({ id: s.id, title: s.title, spot: null, plan: { kind: "none" } });
        }
        placeGhost(point);
        update(point);
        // Défilement automatique près des bords de l'arbre.
        const tree = latest.current.getTree();
        const scroller = tree ? scrollerOf(tree) : null;
        window.clearInterval(s.scrollTimer);
        s.scrollTimer = undefined;
        if (scroller) {
          const r = scroller.getBoundingClientRect();
          const step = edgeScroll(
            { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
            point.y,
          );
          if (step !== 0) {
            s.scrollTimer = window.setInterval(() => {
              scroller.scrollTop += step;
              update(s.last);
            }, 16);
          }
        }
      };
      const onUp = (e: PointerEvent) => {
        const s = session.current;
        if (!s || (e.pointerId !== undefined && e.pointerId !== s.pointerId)) return;
        s.last = { x: e.clientX, y: e.clientY };
        finish(true);
      };
      const onCancelPointer = () => finish(false);
      const onKey = (e: KeyboardEvent) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        e.stopPropagation();
        finish(false);
      };
      const onBlur = () => finish(false);
      window.addEventListener("pointermove", onMove);
      window.addEventListener("pointerup", onUp);
      window.addEventListener("pointercancel", onCancelPointer);
      window.addEventListener("keydown", onKey, { capture: true });
      window.addEventListener("blur", onBlur);
      session.current = {
        pointerId,
        id,
        title: node.title,
        start,
        last: start,
        started: false,
        scrollTimer: undefined,
        cleanup: () => {
          window.removeEventListener("pointermove", onMove);
          window.removeEventListener("pointerup", onUp);
          window.removeEventListener("pointercancel", onCancelPointer);
          window.removeEventListener("keydown", onKey, { capture: true });
          window.removeEventListener("blur", onBlur);
          try {
            (grip as HTMLElement).releasePointerCapture?.(pointerId);
          } catch {
            // Déjà relâchée.
          }
        },
      };
    },
    [finish, update, placeGhost],
  );

  const ghost = drag ? (
    <div ref={attachGhost} className="fl-editor-dnd__ghost" aria-hidden="true">
      {drag.title}
    </div>
  ) : null;

  return { onPointerDown, drag, ghost };
}
