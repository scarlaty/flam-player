import { useEffect, useRef, useState, type DragEvent } from "react";
import type { EpisodeRef, PackNode } from "../../../api/pack";
import { Badge, Button, IconButton, SelectField, TextField } from "../../../components";
import { t } from "../../../i18n";
import { CategoryImageChoice } from "./CategoryImageChoice";
import {
  MAX_LABEL_CHARS,
  append,
  canPlace,
  categoryNode,
  destinationsFor,
  episodesOf,
  menuAt,
  move,
  nodeAt,
  pathOf,
  removeAt,
  renameAt,
  setCategoryImage,
  shift,
  type Menu,
  type Path,
} from "./tree";

type Action = "up" | "down" | "move" | "name";

/** Élément à focaliser après le rendu qui suit une modification. */
interface FocusRequest {
  path: Path;
  action: Action;
}

export interface PackTreeProps {
  menu: Menu;
  /** Titre d'un épisode ; `null` : inconnu (flux indisponible). */
  titleOf: (ref: EpisodeRef) => string | null;
  isUnavailable: (ref: EpisodeRef) => boolean;
  /** L'épisode a une pochette (choix de l'image d'une catégorie, N5 c) ; absent : aucune. */
  hasImage?: (ref: EpisodeRef) => boolean;
  /** Reçoit la nouvelle arborescence et le message à annoncer. */
  onChange: (next: PackNode[], announcement: string) => void;
  /** Annonce sans modification (déplacement refusé). */
  onRefuse: (announcement: string) => void;
}

const pathKey = (path: Path) => path.join(".");

/** Nom affiché d'un menu à `path` : « Racine > Contes > Hiver ». */
export function menuLabel(root: Menu, path: Path): string {
  const names: string[] = [t.podcasts.pack.tree.root];
  let menu = root;
  for (const index of path) {
    const node = menu[index];
    if (node?.kind !== "category") break;
    names.push(
      node.name.trim() === "" ? t.podcasts.pack.tree.untitled : node.name.trim(),
    );
    menu = node.entries;
  }
  return names.join(" > ");
}

/**
 * Arborescence du pack : chaque élément se règle au clavier (monter, descendre, déplacer vers
 * un menu, retirer) ; le glisser-déposer à la souris n'est qu'un complément.
 */
export function PackTree(props: PackTreeProps) {
  const { menu, onChange, onRefuse } = props;
  const text = t.podcasts.pack.tree;
  const root = useRef<HTMLDivElement>(null);
  const focus = useRef<FocusRequest | null>(null);
  const dragFrom = useRef<Path | null>(null);

  // Après une modification, le focus suit l'élément (les boutons gardent leur place dans le DOM
  // mais plus leur élément).
  useEffect(() => {
    const request = focus.current;
    if (request === null) return;
    focus.current = null;
    const target = (action: Action) =>
      root.current?.querySelector<HTMLElement>(
        `[data-path="${pathKey(request.path)}"][data-action="${action}"]`,
      );
    const order: Action[] =
      request.action === "up"
        ? ["up", "down", "move"]
        : request.action === "down"
          ? ["down", "up", "move"]
          : request.action === "move"
            ? ["move", "up", "down"]
            : [request.action];
    for (const action of order) {
      const element = target(action);
      if (element && !(element as HTMLButtonElement).disabled) {
        element.focus();
        return;
      }
    }
    root.current?.focus();
  }, [menu]);

  function labelOf(node: PackNode): string {
    if (node.kind === "category") {
      return node.name.trim() === "" ? text.untitled : node.name.trim();
    }
    return props.titleOf(node) ?? text.unknownTitle;
  }

  function shiftNode(path: Path, delta: -1 | 1) {
    const siblings = menuAt(menu, path.slice(0, -1))!;
    const index = path[path.length - 1]!;
    const target = index + delta;
    if (target < 0 || target >= siblings.length) return;
    const node = siblings[index]!;
    focus.current = {
      path: [...path.slice(0, -1), target],
      action: delta < 0 ? "up" : "down",
    };
    onChange(
      shift(menu, path, delta),
      t.podcasts.pack.announce.moved(labelOf(node), target + 1, siblings.length),
    );
  }

  function moveTo(path: Path, parent: Path) {
    const node = nodeAt(menu, path)!;
    const next = move(menu, path, parent);
    if (next === null) {
      onRefuse(t.podcasts.pack.announce.refused);
      return;
    }
    // Le focus suit l'élément déplacé (le même objet dans la nouvelle arborescence).
    const moved = pathOf(next, node);
    focus.current = moved
      ? { path: moved, action: "move" }
      : { path: [], action: "name" };
    onChange(
      next,
      t.podcasts.pack.announce.movedTo(labelOf(node), menuLabel(menu, parent)),
    );
  }

  function remove(path: Path) {
    const node = nodeAt(menu, path)!;
    const next = removeAt(menu, path);
    const parent = path.slice(0, -1);
    const siblings = menuAt(next, parent)!;
    // Le focus passe à l'élément voisin (celui qui prend la place, sinon le précédent), à
    // défaut à la catégorie parente.
    focus.current =
      siblings.length > 0
        ? {
            path: [...parent, Math.min(path[path.length - 1]!, siblings.length - 1)],
            action: "move",
          }
        : { path: parent, action: "name" };
    onChange(next, t.podcasts.pack.announce.removed(labelOf(node)));
  }

  function addCategory(parent: Path) {
    const node = categoryNode(text.newCategory);
    if (!canPlace(menu, parent, node)) {
      onRefuse(t.podcasts.pack.announce.refused);
      return;
    }
    const target = menuAt(menu, parent)!;
    focus.current = { path: [...parent, target.length], action: "name" };
    onChange(
      append(menu, parent, node),
      t.podcasts.pack.announce.added(text.newCategory, menuLabel(menu, parent)),
    );
  }

  function drop(event: DragEvent, path: Path) {
    const from = dragFrom.current;
    dragFrom.current = null;
    if (from === null) return;
    event.preventDefault();
    event.stopPropagation();
    const node = nodeAt(menu, path);
    if (node === null || path.join(".") === from.join(".")) return;
    // Sur une catégorie : dedans, à la fin ; sur un épisode : juste avant lui.
    const parent = node.kind === "category" ? path : path.slice(0, -1);
    const before = node.kind === "category" ? undefined : path[path.length - 1];
    const next = move(menu, from, parent, before);
    if (next === null) {
      onRefuse(t.podcasts.pack.announce.refused);
      return;
    }
    const moved = nodeAt(menu, from)!;
    const landed = pathOf(next, moved);
    if (landed) focus.current = { path: landed, action: "move" };
    onChange(
      next,
      t.podcasts.pack.announce.movedTo(labelOf(moved), menuLabel(menu, parent)),
    );
  }

  function renderMenu(entries: Menu, parent: Path) {
    const parentLabel = menuLabel(menu, parent);
    const canAddCategory = canPlace(menu, parent, categoryNode(text.newCategory));
    return (
      <>
        <ul className="fl-pack__menu" aria-label={parentLabel}>
          {entries.map((node, index) => {
            const path = [...parent, index];
            return renderRow(path, node, entries);
          })}
          {entries.length === 0 && parent.length > 0 && (
            <li className="fl-pack__empty">{text.menuEmpty}</li>
          )}
        </ul>
        <Button
          variant="ghost"
          size="sm"
          icon="plus"
          disabled={!canAddCategory}
          onClick={() => addCategory(parent)}
        >
          {parent.length === 0 ? text.addCategory : text.addCategoryIn(parentLabel)}
        </Button>
      </>
    );
  }

  function renderRow(path: Path, node: PackNode, entries: Menu) {
    const index = path[path.length - 1]!;
    const label = labelOf(node);
    const key = pathKey(path);
    const destinations = destinationsFor(menu, path, text.root, text.untitled);
    const unavailable = node.kind === "episode" && props.isUnavailable(node);
    return (
      <li
        key={key}
        className="fl-pack__node"
        onDragOver={(event) => {
          if (dragFrom.current !== null) {
            event.preventDefault();
            event.stopPropagation();
          }
        }}
        onDrop={(event) => drop(event, path)}
      >
        <div className="fl-pack__row">
          <span
            className="fl-pack__grip"
            aria-hidden="true"
            draggable
            onDragStart={(event) => {
              event.stopPropagation();
              dragFrom.current = path;
              if (event.dataTransfer) {
                event.dataTransfer.effectAllowed = "move";
                event.dataTransfer.setData("text/plain", "");
                const row = event.currentTarget.closest("li");
                if (row) event.dataTransfer.setDragImage(row, 0, 0);
              }
            }}
            onDragEnd={() => {
              dragFrom.current = null;
            }}
          >
            ⠿
          </span>
          {node.kind === "category" ? (
            <TextField
              className="fl-pack__name"
              label={text.categoryName(path.map((i) => i + 1).join("."))}
              value={node.name}
              maxLength={MAX_LABEL_CHARS}
              data-path={key}
              data-action="name"
              onChange={(event) =>
                props.onChange(renameAt(menu, path, event.target.value), "")
              }
            />
          ) : (
            <p className="fl-pack__label">
              <Badge tone="neutral">{text.episode}</Badge> <span>{label}</span>
              {unavailable && (
                <>
                  {" "}
                  <Badge tone="warning" icon="alert">
                    {text.unavailableBadge}
                  </Badge>
                </>
              )}
            </p>
          )}
          <div className="fl-pack__tools">
            <IconButton
              icon="arrowUp"
              size="sm"
              label={text.up(label)}
              disabled={index === 0}
              data-path={key}
              data-action="up"
              onClick={() => shiftNode(path, -1)}
            />
            <IconButton
              icon="arrowDown"
              size="sm"
              label={text.down(label)}
              disabled={index === entries.length - 1}
              data-path={key}
              data-action="down"
              onClick={() => shiftNode(path, 1)}
            />
            <MoveControl
              label={label}
              dataPath={key}
              destinations={destinations.map((d) => d.label)}
              onMove={(i) => {
                const chosen = destinations[i];
                if (chosen) moveTo(path, chosen.path);
              }}
            />
            <IconButton
              icon="trash"
              size="sm"
              variant="danger-quiet"
              label={
                node.kind === "category" ? text.removeCategory(label) : text.remove(label)
              }
              data-path={key}
              data-action="remove"
              onClick={() => remove(path)}
            />
          </div>
        </div>
        {node.kind === "category" && (
          <div className="fl-pack__children">
            <CategoryImageChoice
              categoryLabel={label}
              candidates={episodesOf(node.entries)
                .filter((ref) => props.hasImage?.(ref) === true)
                .map((ref) => ({ ref, title: props.titleOf(ref) ?? text.unknownTitle }))}
              value={node.image ?? null}
              onChange={(image) =>
                props.onChange(
                  setCategoryImage(menu, path, image),
                  t.podcasts.pack.announce.categoryImage(label, image !== null),
                )
              }
            />
            {renderMenu(node.entries, path)}
          </div>
        )}
      </li>
    );
  }

  return (
    <div ref={root} tabIndex={-1} className="fl-pack__tree" aria-label={text.label}>
      {renderMenu(menu, [])}
    </div>
  );
}

interface MoveControlProps {
  label: string;
  dataPath: string;
  destinations: readonly string[];
  onMove: (index: number) => void;
}

/** Choix d'un menu puis validation explicite : parcourir la liste au clavier ne déplace rien. */
function MoveControl({ label, dataPath, destinations, onMove }: MoveControlProps) {
  const text = t.podcasts.pack.tree;
  const [choice, setChoice] = useState("");
  return (
    <div className="fl-pack__move">
      <SelectField
        label={text.moveTo(label)}
        value={choice}
        disabled={destinations.length === 0}
        data-path={dataPath}
        data-action="move"
        options={[
          { value: "", label: text.movePlaceholder },
          ...destinations.map((d, i) => ({ value: String(i), label: d })),
        ]}
        onChange={(event) => setChoice(event.target.value)}
      />
      <Button
        variant="secondary"
        size="sm"
        disabled={choice === ""}
        aria-label={text.moveConfirm(label)}
        onClick={() => {
          onMove(Number(choice));
          setChoice("");
        }}
      >
        {text.moveButton}
      </Button>
    </div>
  );
}
