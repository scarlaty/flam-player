import {
  AUDIO_EXTENSIONS,
  IMAGE_EXTENSIONS,
  type EditorMenu,
  type EditorNode,
} from "../../../../api/editor";
import { entryName, findNode, isWithin, parentOf } from "../model";
import {
  hitZone,
  slotAt,
  type Point,
  type Rect,
  type RowBox,
  type Spot,
} from "./geometry";

/*
 * Ce que ferait un dépôt (lot 8, § 4.2 et 4.3) : fonctions pures, de l'arbre affiché à
 * l'opération proposée au backend. Le backend revalide tout (formats, bornes, racine) : ce
 * plan ne sert qu'à choisir l'opération et à prévenir avant.
 */

/** Menu qui recevrait un dépôt à la place `spot`, et l'option devant laquelle insérer. */
export interface Destination {
  menu: EditorMenu;
  /** Id de l'option suivante ; `null` : à la fin. */
  beforeId: string | null;
}

/**
 * Destination d'un dépôt à la place `spot` : « dans » un menu, à sa fin ; avant ou après une
 * option, dans son menu. `open` : ids des menus dépliés (« après » un menu déplié qui a des
 * options, c'est en tête de ses options, là où le trait est dessiné).
 */
export function destinationOf(
  root: EditorNode,
  spot: Spot,
  isOpen: (id: string) => boolean = () => false,
): Destination | null {
  const node = findNode(root, spot.id);
  if (!node) return null;
  if (node.kind === "menu" && (spot.slot === "inside" || node.id === "")) {
    return { menu: node, beforeId: null };
  }
  if (
    node.kind === "menu" &&
    spot.slot === "after" &&
    isOpen(node.id) &&
    node.children.length > 0
  ) {
    return { menu: node, beforeId: node.children[0]?.id ?? null };
  }
  const parent = parentOf(root, node.id);
  if (!parent) return null;
  if (spot.slot === "after") {
    const index = parent.children.findIndex((child) => child.id === node.id);
    return { menu: parent, beforeId: parent.children[index + 1]?.id ?? null };
  }
  return { menu: parent, beforeId: node.id };
}

/** Déplacement par le pointeur dans l'arbre. */
export type TreeMovePlan =
  /** Même place : rien à faire. */
  | { kind: "none" }
  /** Interdit : la racine, ou un menu dans lui-même ou sa descendance. */
  | { kind: "invalid" }
  | { kind: "reorder"; menu: string; menuTitle: string; order: string[] }
  | { kind: "move"; dest: string; destTitle: string; before: string | null };

/** Opération qui ferait aller l'élément `dragId` à la place `spot`. */
export function planTreeMove(
  root: EditorNode,
  dragId: string,
  spot: Spot | null,
  isOpen: (id: string) => boolean = () => false,
): TreeMovePlan {
  if (dragId === "") return { kind: "invalid" };
  const dragged = findNode(root, dragId);
  const from = parentOf(root, dragId);
  if (!dragged || !from || !spot) return { kind: "none" };
  if (spot.id === dragId) return { kind: "none" };
  const dest = destinationOf(root, spot, isOpen);
  if (!dest) return { kind: "none" };
  if (isWithin(dest.menu.id, dragId)) return { kind: "invalid" };
  if (dest.menu.id === from.id) {
    const current = from.children.map(entryName);
    const others = from.children.filter((child) => child.id !== dragId);
    const at =
      dest.beforeId === null
        ? others.length
        : others.findIndex((child) => child.id === dest.beforeId);
    if (at < 0) return { kind: "none" };
    const order = others.map(entryName);
    order.splice(at, 0, entryName(dragged));
    if (order.every((name, index) => name === current[index])) return { kind: "none" };
    return { kind: "reorder", menu: from.id, menuTitle: from.title, order };
  }
  const before = dest.beforeId === null ? null : findNode(root, dest.beforeId);
  return {
    kind: "move",
    dest: dest.menu.id,
    destTitle: dest.menu.title,
    before: before ? entryName(before) : null,
  };
}

/** Sorte d'un chemin déposé, d'après son nom (le backend revérifie le contenu). */
export type PathKind = "audio" | "image" | "other" | "folder";

/** Dernier composant d'un chemin Windows ou POSIX. */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const cut = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  return trimmed.slice(cut + 1);
}

/**
 * Sorte d'un chemin : extension audio ou image acceptée ; autre extension de fichier
 * (`.txt`, `.docx`…) : refusée ; sans extension : un dossier probable, laissé au backend.
 */
export function pathKind(path: string): PathKind {
  const name = baseName(path);
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
  if (AUDIO_EXTENSIONS.includes(ext)) return "audio";
  if (IMAGE_EXTENSIONS.includes(ext)) return "image";
  // Une extension de fichier : 1 à 5 caractères, au moins une lettre, sans espace.
  if (/^(?=.*[a-z])[a-z0-9]{1,5}$/.test(ext)) return "other";
  return "folder";
}

/** Zone visée par un dépôt de fichiers. */
export type FileZone =
  | { kind: "row"; spot: Spot }
  /** Zone vide de l'arbre : à la fin de la racine. */
  | { kind: "tree" }
  /** Image de l'élément `id` (panneau). */
  | { kind: "image"; id: string }
  /** Titre lu de l'élément `id` (panneau). */
  | { kind: "titleAudio"; id: string };

/** Pourquoi un dépôt de fichiers ne fait rien. */
export type RefuseReason =
  "nowhere" | "formats" | "notImage" | "oneImage" | "notAudio" | "oneAudio";

/** Ce qu'il faut surligner pendant le survol. */
export type DropHint =
  | { kind: "row"; id: string; slot: "before" | "inside" | "after" }
  | { kind: "image" }
  | { kind: "titleAudio" };

export type FileDropPlan =
  | {
      kind: "import";
      menu: string;
      menuTitle: string;
      before: string | null;
      sources: string[];
      rejected: string[];
      hint: DropHint;
    }
  | { kind: "image"; id: string; title: string; source: string; hint: DropHint }
  | { kind: "titleAudio"; id: string; title: string; source: string; hint: DropHint }
  | { kind: "refuse"; reason: RefuseReason; hint: DropHint | null };

/** Opération proposée pour des fichiers `paths` déposés sur `zone`. */
export function planFileDrop(
  root: EditorNode,
  zone: FileZone | null,
  paths: readonly string[],
  isOpen: (id: string) => boolean = () => false,
): FileDropPlan {
  if (zone === null) return { kind: "refuse", reason: "nowhere", hint: null };
  const kinds = paths.map((path) => ({ path, kind: pathKind(path) }));
  const of = (kind: PathKind) => kinds.filter((k) => k.kind === kind).map((k) => k.path);
  const images = of("image");
  const audio = of("audio");

  if (zone.kind === "image" || zone.kind === "titleAudio") {
    const node = findNode(root, zone.id);
    const wanted = zone.kind === "image" ? images : audio;
    const hint: DropHint = { kind: zone.kind };
    if (!node) return { kind: "refuse", reason: "nowhere", hint: null };
    const [source] = wanted;
    if (source === undefined || paths.length === 0) {
      return {
        kind: "refuse",
        reason: zone.kind === "image" ? "notImage" : "notAudio",
        hint,
      };
    }
    if (paths.length > 1) {
      return {
        kind: "refuse",
        reason: zone.kind === "image" ? "oneImage" : "oneAudio",
        hint,
      };
    }
    return { kind: zone.kind, id: node.id, title: node.title, source, hint };
  }

  const spot: Spot = zone.kind === "tree" ? { id: "", slot: "inside" } : zone.spot;
  const node = findNode(root, spot.id);
  if (!node) return { kind: "refuse", reason: "nowhere", hint: null };

  // Des images seules sur un élément : son image (une seule à la fois).
  if (images.length > 0 && images.length === paths.length) {
    const hint: DropHint = { kind: "row", id: node.id, slot: "inside" };
    const [source] = images;
    if (images.length > 1 || source === undefined)
      return { kind: "refuse", reason: "oneImage", hint };
    return { kind: "image", id: node.id, title: node.title, source, hint };
  }

  const sources = kinds.filter((k) => k.kind !== "other").map((k) => k.path);
  const rejected = of("other");
  const dest = destinationOf(root, spot, isOpen);
  if (!dest) return { kind: "refuse", reason: "nowhere", hint: null };
  const hint: DropHint =
    dest.beforeId === null && spot.slot !== "after"
      ? { kind: "row", id: dest.menu.id, slot: "inside" }
      : { kind: "row", id: spot.id, slot: spot.slot };
  if (audio.length === 0 && of("folder").length === 0) {
    return { kind: "refuse", reason: "formats", hint };
  }
  const before = dest.beforeId === null ? null : findNode(root, dest.beforeId);
  return {
    kind: "import",
    menu: dest.menu.id,
    menuTitle: dest.menu.title,
    before: before ? entryName(before) : null,
    sources,
    rejected,
    hint,
  };
}

/** Zone mesurée de la fenêtre : une ligne de l'arbre, ou une zone nommée. */
export type ZoneBox =
  | { kind: "row"; rect: Rect; row: RowBox }
  | { kind: "tree" | "image" | "titleAudio"; rect: Rect };

/**
 * Zone visée par un dépôt de fichiers au point `point` : la plus petite zone qui le contient.
 * L'image et le titre lu du panneau sont ceux de l'élément choisi `selectedId`.
 */
export function fileZoneAt(
  zones: readonly ZoneBox[],
  point: Point,
  selectedId: string,
): FileZone | null {
  const hit = hitZone(zones, point);
  if (!hit) return null;
  switch (hit.kind) {
    case "row":
      return { kind: "row", spot: { id: hit.row.id, slot: slotAt(hit.row, point.y) } };
    case "tree":
      return { kind: "tree" };
    case "image":
    case "titleAudio":
      return { kind: hit.kind, id: selectedId };
  }
}
