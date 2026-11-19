import type { FolderProblem } from "../../../api/authoring";
import type { EditorMenu, EditorNode, EditorView, Target } from "../../../api/editor";

/** Longueur d'un libellé sur la Flam (`MAX_LABEL_CHARS` des podcasts). */
export const MAX_LABEL_CHARS = 40;

/** Nom de l'élément sur le disque (dossier ou fichier). */
export function entryName(node: EditorNode): string {
  return node.kind === "menu" ? node.dir : node.file;
}

/** Élément d'id `id` sous `root`. */
export function findNode(root: EditorNode, id: string): EditorNode | null {
  if (root.id === id) return root;
  if (root.kind !== "menu") return null;
  for (const child of root.children) {
    const found = findNode(child, id);
    if (found) return found;
  }
  return null;
}

/** Menu qui contient l'élément `id` (`null` pour la racine ou un id inconnu). */
export function parentOf(root: EditorNode, id: string): EditorMenu | null {
  if (root.kind !== "menu") return null;
  for (const child of root.children) {
    if (child.id === id) return root;
    const found = parentOf(child, id);
    if (found) return found;
  }
  return null;
}

/** Cible d'une opération : l'id et l'état vu par l'éditeur ; `null` si l'élément a disparu. */
export function targetOf(view: EditorView, id: string): Target | null {
  const stamp = view.stamps[id];
  return stamp ? { id, stamp } : null;
}

/**
 * Noms des options du menu après avoir décalé l'élément `id` de `delta` places ; `null` s'il
 * est déjà au bord.
 */
export function shiftedOrder(
  menu: EditorMenu,
  id: string,
  delta: -1 | 1,
): string[] | null {
  const names = menu.children.map(entryName);
  const index = menu.children.findIndex((child) => child.id === id);
  const here = names[index];
  const there = names[index + delta];
  if (index < 0 || here === undefined || there === undefined) return null;
  names[index] = there;
  names[index + delta] = here;
  return names;
}

/** Menus de l'arbre avec leur chemin affiché (« Mon histoire > Animaux »). */
export function menusOf(
  root: EditorNode,
  trail: string[] = [],
): { id: string; label: string }[] {
  if (root.kind !== "menu") return [];
  const path = [...trail, root.title];
  return [
    { id: root.id, label: path.join(" > ") },
    ...root.children.flatMap((child) => menusOf(child, path)),
  ];
}

/** Vrai si `id` est `ancestor` ou se trouve dessous. */
export function isWithin(id: string, ancestor: string): boolean {
  return ancestor === "" || id === ancestor || id.startsWith(`${ancestor}/`);
}

/** Élément de l'arbre le plus proche d'un chemin du rapport (lui-même, sinon un parent). */
export function nodeForPath(root: EditorNode, path: string): EditorNode {
  let current = path;
  for (;;) {
    const found = findNode(root, current);
    if (found) return found;
    // Une image ou un titre lu mène à son histoire ; sinon au dossier qui le contient.
    const owner = ownerOf(root, current);
    if (owner) return owner;
    const slash = current.lastIndexOf("/");
    if (slash < 0) return root;
    current = current.slice(0, slash);
  }
}

/** Élément dont `id` est l'image ou le titre lu. */
function ownerOf(root: EditorNode, id: string): EditorNode | null {
  if (root.image === id || root.title_audio === id) return root;
  if (root.kind !== "menu") return null;
  for (const child of root.children) {
    const found = ownerOf(child, id);
    if (found) return found;
  }
  return null;
}

/** Problèmes du rapport rattachés à l'élément `id`. */
export function problemsOf(view: EditorView, id: string): FolderProblem[] {
  return view.report.problems.filter(
    (p) => nodeForPath(view.report.root, p.path).id === id,
  );
}

/** Première image d'histoire sous `node`, dans l'ordre affiché (complément d'un menu, § 7). */
export function firstStoryImage(node: EditorNode): string | null {
  if (node.kind === "story") return node.image;
  for (const child of node.children) {
    const found = firstStoryImage(child);
    if (found) return found;
  }
  return null;
}

/**
 * Image affichée pour un élément : la sienne, ou le complément automatique (`automatic`) ;
 * `keepable` : « Garder » peut l'écrire dans le dossier (menu ou couverture).
 */
export function imageOf(node: EditorNode): {
  id: string | null;
  automatic: boolean;
  keepable: boolean;
} {
  if (node.image) return { id: node.image, automatic: false, keepable: false };
  if (node.kind === "menu") {
    const first = firstStoryImage(node);
    return { id: first, automatic: true, keepable: first !== null };
  }
  return { id: null, automatic: true, keepable: false };
}

/** Nombre d'histoires sous `node`. */
export function storyCount(node: EditorNode): number {
  return node.kind === "story"
    ? 1
    : node.children.reduce((sum, child) => sum + storyCount(child), 0);
}

/** Taille lisible (« 3,2 Mo »). */
export function formatSize(bytes: number): string {
  const units = ["o", "Ko", "Mo", "Go"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 10 ? 0 : 1;
  return `${value.toLocaleString("fr-FR", { maximumFractionDigits: digits })} ${units[unit]}`;
}

/** Ids visibles de l'arbre, dans l'ordre (menus repliés : enfants cachés). */
export function visibleIds(root: EditorNode, collapsed: ReadonlySet<string>): string[] {
  const out = [root.id];
  if (root.kind === "menu" && !collapsed.has(root.id)) {
    for (const child of root.children) out.push(...visibleIds(child, collapsed));
  }
  return out;
}
