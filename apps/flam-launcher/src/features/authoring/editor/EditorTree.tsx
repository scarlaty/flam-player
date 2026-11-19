import { useEffect, useId, useMemo, useRef, type KeyboardEvent } from "react";
import type { EditorNode, EditorView } from "../../../api/editor";
import { Badge } from "../../../components";
import { t } from "../../../i18n";
import { findNode, formatSize, nodeForPath, parentOf, visibleIds } from "./model";
import { Thumbnail } from "./Thumbnail";

/** Gravité la plus forte des problèmes d'un élément. */
type Flag = "blocking" | "warning";

export interface EditorTreeProps {
  view: EditorView;
  selectedId: string;
  onSelect: (id: string) => void;
  collapsed: ReadonlySet<string>;
  onToggle: (id: string) => void;
  /** Alt+Flèche : monter (-1) ou descendre (+1) l'élément dans son menu. */
  onShift: (id: string, delta: -1 | 1) => void;
  /** Suppr : mettre l'élément à la corbeille (après confirmation). */
  onDelete: (id: string) => void;
  /** F2 : renommer (focus sur le titre du panneau). */
  onRename: (id: string) => void;
}

const domId = (base: string, id: string) =>
  `${base}-${Array.from(id, (c) => c.charCodeAt(0).toString(36)).join("_")}`;

/**
 * Arbre de l'histoire (motif ARIA « tree ») : un seul élément focalisable à la fois (celui
 * choisi), flèches pour parcourir, ouvrir et fermer les menus, Alt+Flèche haut ou bas pour
 * réordonner, Suppr pour mettre à la corbeille, F2 pour renommer.
 */
export function EditorTree(props: EditorTreeProps) {
  const { view, selectedId, onSelect, collapsed, onToggle } = props;
  const text = t.authoringEditor.tree;
  const base = useId();
  const helpId = `${base}-help`;
  const root = view.report.root;
  const tree = useRef<HTMLUListElement>(null);
  const focusPending = useRef(false);

  const flags = useMemo(() => {
    const out = new Map<string, Flag>();
    for (const problem of view.report.problems) {
      const id = nodeForPath(root, problem.path).id;
      if (problem.severity === "blocking" || !out.has(id))
        out.set(id, problem.severity === "blocking" ? "blocking" : "warning");
    }
    return out;
  }, [view, root]);

  // Le focus suit la sélection faite au clavier.
  useEffect(() => {
    if (!focusPending.current) return;
    focusPending.current = false;
    const row = tree.current?.querySelector<HTMLElement>(
      `#${CSS.escape(domId(base, selectedId))}`,
    );
    row?.focus();
  }, [selectedId, base]);

  const move = (id: string | undefined) => {
    if (id === undefined) return;
    focusPending.current = true;
    onSelect(id);
  };

  function onKeyDown(event: KeyboardEvent<HTMLUListElement>) {
    const ids = visibleIds(root, collapsed);
    const index = ids.indexOf(selectedId);
    const node = findNode(root, selectedId);
    if (index < 0 || node === null) return;
    const isMenu = node.kind === "menu";
    const open = isMenu && !collapsed.has(node.id);
    let handled = true;
    if (event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
      if (node.id !== "") props.onShift(node.id, event.key === "ArrowUp" ? -1 : 1);
    } else if (event.altKey || event.ctrlKey || event.metaKey) {
      handled = false;
    } else if (event.key === "ArrowDown") {
      if (index + 1 < ids.length) move(ids[index + 1]);
    } else if (event.key === "ArrowUp") {
      if (index > 0) move(ids[index - 1]);
    } else if (event.key === "Home") {
      move(ids[0]);
    } else if (event.key === "End") {
      move(ids[ids.length - 1]);
    } else if (event.key === "ArrowRight") {
      if (isMenu && !open) onToggle(node.id);
      else if (open) move(node.children[0]?.id);
    } else if (event.key === "ArrowLeft") {
      if (open && node.id !== "") onToggle(node.id);
      else {
        const parent = parentOf(root, node.id);
        if (parent) move(parent.id);
      }
    } else if (event.key === "Delete") {
      if (node.id !== "") props.onDelete(node.id);
    } else if (event.key === "F2") {
      props.onRename(node.id);
    } else {
      handled = false;
    }
    if (handled) event.preventDefault();
  }

  return (
    <>
      <p id={helpId} className="fl-visually-hidden">
        {text.help}
      </p>
      <ul
        ref={tree}
        role="tree"
        aria-label={text.label}
        aria-describedby={helpId}
        className="fl-editor-tree"
        onKeyDown={onKeyDown}
      >
        <Row
          node={root}
          level={1}
          view={view}
          flags={flags}
          base={base}
          selectedId={selectedId}
          collapsed={collapsed}
          onSelect={onSelect}
          onToggle={onToggle}
        />
      </ul>
    </>
  );
}

interface RowProps {
  node: EditorNode;
  level: number;
  view: EditorView;
  flags: Map<string, Flag>;
  base: string;
  selectedId: string;
  collapsed: ReadonlySet<string>;
  onSelect: (id: string) => void;
  onToggle: (id: string) => void;
}

function Row(props: RowProps) {
  const { node, level, view, flags, base, selectedId, collapsed } = props;
  const text = t.authoringEditor.tree;
  const isMenu = node.kind === "menu";
  const open = isMenu && !collapsed.has(node.id);
  const selected = node.id === selectedId;
  const flag = flags.get(node.id);
  const meta = isMenu
    ? text.items(node.children.length)
    : text.size(formatSize(node.bytes));
  const imageId = node.image;
  const rowId = domId(base, node.id);
  return (
    <li
      id={rowId}
      role="treeitem"
      aria-labelledby={`${rowId}-t`}
      aria-describedby={`${rowId}-m ${rowId}-b`}
      aria-level={level}
      aria-selected={selected}
      aria-expanded={isMenu && node.children.length > 0 ? open : undefined}
      tabIndex={selected ? 0 : -1}
      className="fl-editor-tree__item"
      onClick={(event) => {
        event.stopPropagation();
        props.onSelect(node.id);
      }}
    >
      <span
        className={`fl-editor-tree__row${selected ? " fl-editor-tree__row--selected" : ""}`}
        style={{
          paddingInlineStart: `calc(${level - 1} * var(--space-4) + var(--space-2))`,
        }}
      >
        {isMenu && node.children.length > 0 ? (
          <span
            className="fl-editor-tree__twisty"
            aria-hidden="true"
            onClick={(event) => {
              event.stopPropagation();
              props.onToggle(node.id);
            }}
          >
            {open ? "▾" : "▸"}
          </span>
        ) : (
          <span className="fl-editor-tree__twisty" aria-hidden="true" />
        )}
        <Thumbnail
          id={imageId}
          stamp={imageId ? view.stamps[imageId] : undefined}
          size={40}
          alt=""
          fallback={isMenu ? "folder" : "play"}
        />
        <span className="fl-editor-tree__text">
          <span id={`${rowId}-t`} className="fl-editor-tree__title">
            {node.title}
          </span>
          <span id={`${rowId}-m`} className="fl-editor-tree__meta">
            {meta}
          </span>
        </span>
        <span id={`${rowId}-b`} className="fl-editor-tree__badges">
          {node.title_audio && <Badge tone="success">{text.titleAudio}</Badge>}
          {!node.image && <Badge tone="neutral">{text.autoImage}</Badge>}
          {flag && (
            <Badge tone={flag === "blocking" ? "danger" : "warning"} icon="alert">
              {flag === "blocking" ? text.blocking : text.problem}
            </Badge>
          )}
        </span>
      </span>
      {isMenu && open && node.children.length > 0 && (
        <ul role="group" className="fl-editor-tree__group">
          {node.children.map((child) => (
            <Row key={child.id} {...props} node={child} level={level + 1} />
          ))}
        </ul>
      )}
    </li>
  );
}
