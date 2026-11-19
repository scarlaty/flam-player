/*
 * Géométrie du glisser-déposer de l'éditeur (lot 8, § 4.3) : fonctions pures, sans DOM, pour
 * retrouver l'élément sous le pointeur ou sous un dépôt de fichiers et la place d'insertion.
 */

/** Rectangle en pixels CSS de la page (comme `getBoundingClientRect`). */
export interface Rect {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

export interface Point {
  x: number;
  y: number;
}

/** Place d'un dépôt par rapport à une ligne de l'arbre. */
export type Slot = "before" | "inside" | "after";

/** Ligne de l'arbre mesurée. */
export interface RowBox {
  id: string;
  rect: Rect;
  /** Menu (peut recevoir « dans »). */
  menu: boolean;
}

/** Vrai si `point` est dans `rect` (bords compris). */
export function contains(rect: Rect, point: Point): boolean {
  return (
    point.x >= rect.left &&
    point.x <= rect.right &&
    point.y >= rect.top &&
    point.y <= rect.bottom
  );
}

const area = (rect: Rect) =>
  Math.max(0, rect.right - rect.left) * Math.max(0, rect.bottom - rect.top);

/**
 * Zone sous `point` : la plus petite qui le contient (les zones imbriquées, une ligne dans
 * l'arbre, l'emportent sur leur conteneur) ; `null` hors de toute zone.
 */
export function hitZone<T extends { rect: Rect }>(
  zones: readonly T[],
  point: Point,
): T | null {
  let best: T | null = null;
  for (const zone of zones) {
    if (!contains(zone.rect, point)) continue;
    if (best === null || area(zone.rect) < area(best.rect)) best = zone;
  }
  return best;
}

/**
 * Place d'un dépôt à la hauteur `y` sur la ligne `row` : une histoire se partage en deux
 * (avant, après) ; un menu en trois (un quart avant, la moitié dans, un quart après) ; la
 * racine (id vide) reçoit toujours « dans ».
 */
export function slotAt(row: RowBox, y: number): Slot {
  if (row.id === "") return "inside";
  const height = Math.max(1, row.rect.bottom - row.rect.top);
  const ratio = (y - row.rect.top) / height;
  if (!row.menu) return ratio < 0.5 ? "before" : "after";
  if (ratio < 0.25) return "before";
  if (ratio > 0.75) return "after";
  return "inside";
}

/**
 * Ligne visée à la hauteur `y` (lignes dans l'ordre affiché) : celle qui la contient ; sous la
 * dernière, la dernière ; au-dessus de la première, la première. `null` sans ligne.
 */
export function rowAt(rows: readonly RowBox[], y: number): RowBox | null {
  if (rows.length === 0) return null;
  for (const row of rows) {
    if (y >= row.rect.top && y <= row.rect.bottom) return row;
  }
  const first = rows[0];
  const last = rows[rows.length - 1];
  if (first && y < first.rect.top) return first;
  if (last && y > last.rect.bottom) return last;
  // Entre deux lignes (marges) : la plus proche.
  let best: RowBox | null = null;
  let distance = Infinity;
  for (const row of rows) {
    const d = Math.min(Math.abs(y - row.rect.top), Math.abs(y - row.rect.bottom));
    if (d < distance) {
      distance = d;
      best = row;
    }
  }
  return best;
}

/** Place visée dans l'arbre : ligne et place sur cette ligne. */
export interface Spot {
  id: string;
  slot: Slot;
}

/** Place visée à la hauteur `y` ; `null` sans ligne. */
export function spotAt(rows: readonly RowBox[], y: number): Spot | null {
  const row = rowAt(rows, y);
  if (!row) return null;
  // Sous la dernière ligne : après elle (ou dans la racine si c'est la seule).
  const slot =
    y > row.rect.bottom && row.id !== ""
      ? "after"
      : y < row.rect.top && row.id !== ""
        ? "before"
        : slotAt(row, y);
  return { id: row.id, slot };
}

/**
 * Défilement automatique près des bords de `rect` : pas en pixels (négatif vers le haut), nul
 * hors des marges ; plus rapide quand le pointeur s'approche du bord ou le dépasse.
 */
export function edgeScroll(rect: Rect, y: number, margin = 32, maxStep = 18): number {
  const top = y - rect.top;
  const bottom = rect.bottom - y;
  if (top < margin) return -Math.ceil(maxStep * Math.min(1, (margin - top) / margin));
  if (bottom < margin)
    return Math.ceil(maxStep * Math.min(1, (margin - bottom) / margin));
  return 0;
}

/** Vrai si le pointeur s'est assez éloigné de son départ pour commencer un glissement. */
export function pastThreshold(start: Point, now: Point, threshold = 5): boolean {
  return Math.hypot(now.x - start.x, now.y - start.y) >= threshold;
}

/** Position d'un événement de dépôt (pixels physiques de la vue) en pixels CSS. */
export function physicalToCss(position: Point, ratio: number): Point {
  const r = ratio > 0 && Number.isFinite(ratio) ? ratio : 1;
  return { x: position.x / r, y: position.y / r };
}

/** Partie de `rect` visible dans `bounds` ; `null` si elle est vide (élément défilé hors vue). */
export function clip(rect: Rect, bounds: Rect | null): Rect | null {
  if (bounds === null) return rect;
  const out = {
    left: Math.max(rect.left, bounds.left),
    top: Math.max(rect.top, bounds.top),
    right: Math.min(rect.right, bounds.right),
    bottom: Math.min(rect.bottom, bounds.bottom),
  };
  return out.right > out.left && out.bottom > out.top ? out : null;
}
