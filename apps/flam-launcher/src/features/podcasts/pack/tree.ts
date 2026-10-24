import type { EpisodeRef, PackNode } from "../../../api/pack";

/** Bornes de l'arborescence (N2 b ; miroir de `pack.rs`). */
export const MAX_MENU_DEPTH = 3;
export const MAX_OPTIONS_PER_MENU = 50;
export const MAX_LABEL_CHARS = 40;

/** Position d'un nœud : indices depuis le menu racine. */
export type Path = readonly number[];

export type Menu = readonly PackNode[];

export const sameEpisode = (a: EpisodeRef, b: EpisodeRef) =>
  a.source_id === b.source_id && a.episode_id === b.episode_id;

export const episodeNode = (ref: EpisodeRef): PackNode => ({
  kind: "episode",
  source_id: ref.source_id,
  episode_id: ref.episode_id,
});

export const categoryNode = (name: string): PackNode => ({
  kind: "category",
  name,
  entries: [],
});

/** Menu contenu à `path` (`[]` : racine) ; `null` si le chemin ne désigne pas une catégorie. */
export function menuAt(root: Menu, path: Path): Menu | null {
  let menu = root;
  for (const index of path) {
    const node = menu[index];
    if (node?.kind !== "category") return null;
    menu = node.entries;
  }
  return menu;
}

export function nodeAt(root: Menu, path: Path): PackNode | null {
  if (path.length === 0) return null;
  const parent = menuAt(root, path.slice(0, -1));
  return parent?.[path[path.length - 1]!] ?? null;
}

/** Chemin du nœud `target` (même objet) dans `root` ; `null` s'il n'y figure pas. */
export function pathOf(root: Menu, target: PackNode): Path | null {
  for (let i = 0; i < root.length; i++) {
    const node = root[i]!;
    if (node === target) return [i];
    if (node.kind === "category") {
      const inner = pathOf(node.entries, target);
      if (inner !== null) return [i, ...inner];
    }
  }
  return null;
}

/** Remplace le menu situé à `parent` par `change(menu)`. */
function updateMenu(root: Menu, parent: Path, change: (menu: Menu) => Menu): PackNode[] {
  if (parent.length === 0) return [...change(root)];
  const [head, ...rest] = parent as [number, ...number[]];
  return root.map((node, i) =>
    i === head && node.kind === "category"
      ? { ...node, entries: updateMenu(node.entries, rest, change) }
      : node,
  );
}

/** Ajoute `node` à la fin du menu `parent`. */
export function append(root: Menu, parent: Path, node: PackNode): PackNode[] {
  return updateMenu(root, parent, (menu) => [...menu, node]);
}

/** Retrait brut : n'efface aucune image choisie (utilisé par `move`, qui réinsère le nœud). */
function removeAtRaw(root: Menu, path: Path): PackNode[] {
  return updateMenu(root, path.slice(0, -1), (menu) =>
    menu.filter((_, i) => i !== path[path.length - 1]),
  );
}

export function removeAt(root: Menu, path: Path): PackNode[] {
  return pruneImages(removeAtRaw(root, path));
}

/**
 * Efface, sans rien dire, l'image choisie d'une catégorie (N5 c) dont l'épisode n'en fait plus
 * partie (retiré ou déplacé ailleurs) : retour à l'image automatique. Les nœuds inchangés
 * gardent leur identité (le focus suit un nœud déplacé par référence).
 */
export function pruneImages(menu: Menu): PackNode[] {
  return menu.map((node) => {
    if (node.kind !== "category") return node;
    const entries = pruneImages(node.entries);
    const changed = entries.some((e, i) => e !== node.entries[i]);
    const stale = node.image !== undefined && !containsEpisode(entries, node.image);
    if (!changed && !stale) return node;
    const { image, ...rest } = node;
    return stale || image === undefined
      ? { ...rest, entries }
      : { ...rest, entries, image };
  });
}

/** Choisit l'image de la catégorie à `path` ; `null` : automatique. */
export function setCategoryImage(
  root: Menu,
  path: Path,
  image: EpisodeRef | null,
): PackNode[] {
  return updateMenu(root, path.slice(0, -1), (menu) =>
    menu.map((node, i) => {
      if (i !== path[path.length - 1] || node.kind !== "category") return node;
      const { image: _previous, ...rest } = node;
      void _previous;
      return image === null
        ? rest
        : {
            ...rest,
            image: { source_id: image.source_id, episode_id: image.episode_id },
          };
    }),
  );
}

export function renameAt(root: Menu, path: Path, name: string): PackNode[] {
  return updateMenu(root, path.slice(0, -1), (menu) =>
    menu.map((node, i) =>
      i === path[path.length - 1] && node.kind === "category" ? { ...node, name } : node,
    ),
  );
}

/** Décale le nœud d'une position (`-1` : monter, `1` : descendre) ; sans effet aux bords. */
export function shift(root: Menu, path: Path, delta: -1 | 1): PackNode[] {
  const index = path[path.length - 1]!;
  const target = index + delta;
  return updateMenu(root, path.slice(0, -1), (menu) => {
    if (target < 0 || target >= menu.length) return menu;
    const copy = [...menu];
    [copy[index], copy[target]] = [copy[target]!, copy[index]!];
    return copy;
  });
}

/** Niveaux de menus de `menu`, lui compris (menu sans catégorie : 1). */
export function menuHeight(menu: Menu): number {
  let deepest = 0;
  for (const node of menu) {
    if (node.kind === "category") deepest = Math.max(deepest, menuHeight(node.entries));
  }
  return 1 + deepest;
}

const isPrefix = (prefix: Path, path: Path) =>
  prefix.length <= path.length && prefix.every((v, i) => path[i] === v);

const samePath = (a: Path, b: Path) => a.length === b.length && isPrefix(a, b);

/**
 * `true` si le nœud `node` peut être inséré dans le menu `parent` (niveau `parent.length + 1`)
 * sans dépasser les bornes : profondeur et nombre d'options (N2 b). `from` : position actuelle
 * du nœud (déplacement), qui libère sa place dans son menu d'origine.
 */
export function canPlace(
  root: Menu,
  parent: Path,
  node: PackNode,
  from: Path | null = null,
): boolean {
  if (from !== null && isPrefix(from, parent)) return false; // dans lui-même ou un descendant
  const target = menuAt(root, parent);
  if (target === null) return false;
  const staysInPlace = from !== null && samePath(from.slice(0, -1), parent);
  if (!staysInPlace && target.length >= MAX_OPTIONS_PER_MENU) return false;
  // Une catégorie ouvre un sous-menu au niveau `parent.length + 2`.
  if (
    node.kind === "category" &&
    parent.length + 1 + menuHeight(node.entries) > MAX_MENU_DEPTH
  ) {
    return false;
  }
  return true;
}

/** Ajoute à `parent` si les bornes le permettent ; `null` sinon. */
export function tryAppend(root: Menu, parent: Path, node: PackNode): PackNode[] | null {
  return canPlace(root, parent, node) ? append(root, parent, node) : null;
}

/**
 * Déplace le nœud `from` à la fin de `parent`, ou avant `before` (indice dans `parent`, tel
 * qu'avant le déplacement). `null` si les bornes l'interdisent.
 */
export function move(
  root: Menu,
  from: Path,
  parent: Path,
  before?: number,
): PackNode[] | null {
  const node = nodeAt(root, from);
  if (node === null || !canPlace(root, parent, node, from)) return null;
  const fromParent = from.slice(0, -1);
  const fromIndex = from[from.length - 1]!;
  // Dans l'arbre privé du nœud, un indice de même niveau que `from` et supérieur à lui recule.
  const adjusted = parent.map((v, i) =>
    i === fromParent.length && isPrefix(fromParent, parent) && v > fromIndex ? v - 1 : v,
  );
  const without = removeAtRaw(root, from);
  const sameMenu = samePath(fromParent, parent);
  const insertAt =
    before === undefined
      ? undefined
      : sameMenu && before > fromIndex
        ? before - 1
        : before;
  return pruneImages(
    updateMenu(without, adjusted, (menu) => {
      const copy = [...menu];
      copy.splice(insertAt ?? copy.length, 0, node);
      return copy;
    }),
  );
}

export interface Destination {
  path: Path;
  /** Chemin lisible : « Racine > Contes > Hiver ». */
  label: string;
}

/** Menus où le nœud `from` peut être déplacé (racine comprise), hors son menu actuel. */
export function destinationsFor(
  root: Menu,
  from: Path,
  rootLabel: string,
  untitled: string,
): Destination[] {
  const node = nodeAt(root, from);
  if (node === null) return [];
  const fromParent = from.slice(0, -1);
  const out: Destination[] = [];
  const visit = (menu: Menu, path: Path, label: string) => {
    if (!samePath(path, fromParent) && canPlace(root, path, node, from)) {
      out.push({ path, label });
    }
    menu.forEach((child, i) => {
      if (child.kind === "category") {
        const name = child.name.trim() === "" ? untitled : child.name.trim();
        visit(child.entries, [...path, i], `${label} > ${name}`);
      }
    });
  };
  visit(root, [], rootLabel);
  return out;
}

/** Épisodes de l'arborescence, dans l'ordre, doublons compris. */
export function episodesOf(menu: Menu): EpisodeRef[] {
  const out: EpisodeRef[] = [];
  for (const node of menu) {
    if (node.kind === "episode") out.push(node);
    else out.push(...episodesOf(node.entries));
  }
  return out;
}

export function containsEpisode(menu: Menu, ref: EpisodeRef): boolean {
  return episodesOf(menu).some((e) => sameEpisode(e, ref));
}

/** Retire tout nœud épisode désignant `ref`. */
export function removeEpisode(menu: Menu, ref: EpisodeRef): PackNode[] {
  return pruneImages(removeEpisodeNodes(menu, ref));
}

function removeEpisodeNodes(menu: Menu, ref: EpisodeRef): PackNode[] {
  return menu
    .filter((node) => node.kind !== "episode" || !sameEpisode(node, ref))
    .map((node) =>
      node.kind === "category"
        ? { ...node, entries: removeEpisodeNodes(node.entries, ref) }
        : node,
    );
}
