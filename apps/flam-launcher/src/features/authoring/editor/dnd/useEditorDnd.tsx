import { useCallback, useMemo, useRef, useState, type ReactNode } from "react";
import {
  importFiles,
  moveItem,
  reorder,
  setImage,
  setTitleAudio,
  type EditorNode,
  type EditorUpdate,
  type EditorView,
} from "../../../../api/editor";
import { t } from "../../../../i18n";
import { targetOf } from "../model";
import type { Point } from "./geometry";
import {
  fileZoneAt,
  planFileDrop,
  type DropHint,
  type FileDropPlan,
  type TreeMovePlan,
  type ZoneBox,
} from "./plan";
import { useFileDrop, type FileDragSubscribe } from "./useFileDrop";
import { measureRows, useTreeDrag, type TreeDragState } from "./useTreeDrag";
import "./Dnd.css";

/** Attribut d'une zone de dépôt nommée (`tree`, `image`, `titleAudio`). */
export const ZONE_ATTR = "data-dnd-zone";

/** Ce que l'arbre reçoit du glisser-déposer. */
export interface TreeDnd {
  onPointerDown: ReturnType<typeof useTreeDrag>["onPointerDown"];
  /** Place surlignée (pointeur ou fichiers). */
  hint: Extract<DropHint, { kind: "row" }> | null;
  /** Place refusée (un menu dans lui-même). */
  invalid: boolean;
  /** Élément en cours de déplacement. */
  draggingId: string | null;
  disabled: boolean;
}

export interface EditorDndOptions {
  view: EditorView | null;
  selectedId: string;
  collapsed: ReadonlySet<string>;
  busy: boolean;
  run: (
    action: () => Promise<EditorUpdate>,
    say?: (label: string) => string,
  ) => Promise<boolean>;
  select: (id: string) => void;
  /** Le focus revient sur la ligne choisie de l'arbre. */
  refocusTree: () => void;
  announce: (message: string) => void;
  setError: (message: string | null) => void;
  /** Abonnement aux dépôts de fichiers (tests : simulé). */
  subscribe?: FileDragSubscribe;
}

export interface EditorDnd {
  /** À poser sur la racine de l'éditeur (zones mesurées dessous). */
  attachRoot: (el: HTMLElement | null) => void;
  tree: TreeDnd;
  /** Zone du panneau surlignée pendant un dépôt de fichiers. */
  panelZone: "image" | "titleAudio" | null;
  /** Des fichiers survolent la fenêtre. */
  filesOver: boolean;
  /** Aperçu du déplacement et bandeau du dépôt. */
  overlay: ReactNode;
}

/** Zones de dépôt mesurées sous `root`. */
export function measureZones(root: HTMLElement): ZoneBox[] {
  const zones: ZoneBox[] = [];
  const tree = root.querySelector<HTMLElement>('[role="tree"]');
  if (tree)
    for (const row of measureRows(tree)) zones.push({ kind: "row", rect: row.rect, row });
  for (const el of root.querySelectorAll<HTMLElement>(`[${ZONE_ATTR}]`)) {
    const kind = el.getAttribute(ZONE_ATTR);
    if (kind !== "tree" && kind !== "image" && kind !== "titleAudio") continue;
    const r = el.getBoundingClientRect();
    zones.push({
      kind,
      rect: { left: r.left, top: r.top, right: r.right, bottom: r.bottom },
    });
  }
  return zones;
}

function refuseText(plan: Extract<FileDropPlan, { kind: "refuse" }>): string {
  return t.authoringDnd.refuse[plan.reason];
}

/** Texte du bandeau pendant le survol de fichiers. */
function hoverText(plan: FileDropPlan): string {
  const text = t.authoringDnd;
  switch (plan.kind) {
    case "import":
      return text.dropHere.import(plan.menuTitle);
    case "image":
      return text.dropHere.image(plan.title);
    case "titleAudio":
      return text.dropHere.titleAudio(plan.title);
    case "refuse":
      return refuseText(plan);
  }
}

/**
 * Glisser-déposer de l'éditeur (§ 4.3) : fichiers venus de l'Explorateur (copie, image, titre
 * lu) et déplacement des éléments de l'arbre au pointeur. Le plan n'est qu'une proposition :
 * le backend revalide chaque opération (formats, bornes, racine, état vu).
 */
export function useEditorDnd(options: EditorDndOptions): EditorDnd {
  const {
    view,
    selectedId,
    collapsed,
    busy,
    run,
    select,
    refocusTree,
    announce,
    setError,
  } = options;
  const text = t.authoringDnd;
  const rootEl = useRef<HTMLElement | null>(null);
  const attachRoot = useCallback((el: HTMLElement | null) => {
    rootEl.current = el;
  }, []);
  const isOpen = useCallback((id: string) => !collapsed.has(id), [collapsed]);
  const [hover, setHover] = useState<{ plan: FileDropPlan; point: Point } | null>(null);

  const planAt = useCallback(
    (paths: string[], point: Point): FileDropPlan | null => {
      const root = rootEl.current;
      if (!view || !root) return null;
      const zone = fileZoneAt(measureZones(root), point, selectedId);
      return planFileDrop(view.report.root, zone, paths, isOpen);
    },
    [view, selectedId, isOpen],
  );

  const dropFiles = useCallback(
    (paths: string[], point: Point) => {
      const plan = planAt(paths, point);
      if (!plan || !view) return;
      if (busy) {
        setError(text.refuse.busy);
        return;
      }
      switch (plan.kind) {
        case "refuse":
          setError(refuseText(plan));
          return;
        case "import": {
          const { menu, menuTitle, before, sources, rejected } = plan;
          const extra = rejected.length > 0 ? ` ${text.rejected(rejected.length)}` : "";
          void run(
            () => importFiles(menu, sources, before),
            () => `${text.copied(menuTitle)}${extra}`,
          );
          return;
        }
        case "image":
        case "titleAudio": {
          const at = targetOf(view, plan.id);
          if (!at) {
            setError(text.refuse.gone);
            return;
          }
          const source = plan.source;
          void run(() =>
            plan.kind === "image" ? setImage(at, source) : setTitleAudio(at, source),
          );
          return;
        }
      }
    },
    [planAt, view, busy, run, setError, text],
  );

  useFileDrop(
    {
      onHover: (drag) => {
        const plan = drag ? planAt(drag.paths, drag.point) : null;
        setHover(drag && plan ? { plan, point: drag.point } : null);
      },
      onDrop: dropFiles,
    },
    options.subscribe,
  );

  const onTreeDrop = useCallback(
    (id: string, title: string, plan: TreeMovePlan) => {
      if (!view) return;
      switch (plan.kind) {
        case "none":
          announce(text.stayed(title));
          refocusTree();
          return;
        case "invalid":
          setError(text.refuse.invalid);
          announce(text.refuse.invalid);
          refocusTree();
          return;
        case "reorder": {
          const { menu, order, menuTitle } = plan;
          select(id);
          void run(
            () => reorder(menu, order),
            () => text.reordered(title, menuTitle),
          ).then(refocusTree);
          return;
        }
        case "move": {
          const at = targetOf(view, id);
          if (!at) {
            setError(text.refuse.gone);
            return;
          }
          const { dest, before, destTitle } = plan;
          // `run` choisit l'élément à sa nouvelle place (id rendu par le backend).
          void run(
            () => moveItem(at, dest, before),
            () => text.moved(title, destTitle),
          ).then(refocusTree);
          return;
        }
      }
    },
    [view, run, select, refocusTree, announce, setError, text],
  );

  const getTree = useCallback(
    () => rootEl.current?.querySelector<HTMLElement>('[role="tree"]') ?? null,
    [],
  );
  const treeDrag = useTreeDrag({
    getTree,
    root: view?.report.root ?? EMPTY_ROOT,
    isOpen,
    disabled: busy || view === null,
    onStart: (id, title) => {
      setError(null);
      select(id);
      announce(text.started(title));
    },
    onCancel: (_id, title) => {
      announce(text.cancelled(title));
      refocusTree();
    },
    onDrop: onTreeDrop,
  });

  const drag: TreeDragState | null = treeDrag.drag;
  const tree = useMemo<TreeDnd>(() => {
    let hint: TreeDnd["hint"] = null;
    let invalid = false;
    if (drag) {
      if (drag.spot && drag.plan.kind !== "none") hint = { kind: "row", ...drag.spot };
      invalid = drag.plan.kind === "invalid";
    } else if (hover?.plan.hint?.kind === "row") {
      hint = hover.plan.hint;
      invalid = hover.plan.kind === "refuse";
    }
    return {
      onPointerDown: treeDrag.onPointerDown,
      hint,
      invalid,
      draggingId: drag?.id ?? null,
      disabled: busy,
    };
  }, [drag, hover, treeDrag.onPointerDown, busy]);

  const hintKind = hover?.plan.hint?.kind;
  const panelZone = hintKind === "image" || hintKind === "titleAudio" ? hintKind : null;

  const overlay = (
    <>
      {treeDrag.ghost}
      {hover && (
        <div
          className={`fl-editor-dnd__banner${hover.plan.kind === "refuse" ? " fl-editor-dnd__banner--refused" : ""}`}
          aria-hidden="true"
        >
          {hoverText(hover.plan)}
        </div>
      )}
    </>
  );

  return { attachRoot, tree, panelZone, filesOver: hover !== null, overlay };
}

const EMPTY_ROOT: EditorNode = {
  kind: "menu",
  id: "",
  dir: "",
  title: "",
  mode: "recueil",
  fin: "menu",
  annoncer: false,
  image: null,
  title_audio: null,
  children: [],
};
