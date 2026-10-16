import type { StoryEntry } from "../../api/story";

export type SortKey = "title" | "recent";

export interface LibraryFilters {
  query: string;
  /** Âge de l'enfant : garde les histoires recommandées dès cet âge ou avant. `null` : tous. */
  age: number | null;
  category: string | null;
  sort: SortKey;
}

export const DEFAULT_FILTERS: LibraryFilters = {
  query: "",
  age: null,
  category: null,
  sort: "title",
};

/** Minuscules sans accents, pour une recherche tolérante. */
export function normalizeText(text: string): string {
  return text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase().trim();
}

export function filtersAreDefault(filters: LibraryFilters): boolean {
  return filters.query.trim() === "" && filters.age === null && filters.category === null;
}

/** Âges recommandés présents dans les entrées, du plus petit au plus grand. */
export function availableAges(entries: readonly StoryEntry[]): number[] {
  const ages = new Set<number>();
  for (const entry of entries) {
    if (entry.age !== null) ages.add(entry.age);
  }
  return [...ages].sort((a, b) => a - b);
}

/** Catégories présentes dans les entrées, triées alphabétiquement. */
export function availableCategories(entries: readonly StoryEntry[]): string[] {
  const categories = new Set<string>();
  for (const entry of entries) {
    const category = entry.category?.trim();
    if (category) categories.add(category);
  }
  return [...categories].sort((a, b) =>
    a.localeCompare(b, "fr", { sensitivity: "base" }),
  );
}

function matchesQuery(entry: StoryEntry, words: string[]): boolean {
  if (words.length === 0) return true;
  const haystack = normalizeText(
    [entry.title, entry.author, entry.category, entry.description]
      .filter(Boolean)
      .join(" "),
  );
  return words.every((word) => haystack.includes(word));
}

/** Date de mise à jour en millisecondes ; 0 si absente ou illisible. */
function timeOf(entry: StoryEntry): number {
  if (!entry.updated_at) return 0;
  const time = Date.parse(entry.updated_at);
  return Number.isNaN(time) ? 0 : time;
}

const byTitle = (a: StoryEntry, b: StoryEntry) =>
  a.title.localeCompare(b.title, "fr", { sensitivity: "base", numeric: true });

/**
 * Recherche, filtres puis tri. La recherche ignore casse et accents et exige tous les mots.
 * Une histoire sans âge connu reste visible quand un âge est choisi.
 */
export function applyFilters(
  entries: readonly StoryEntry[],
  filters: LibraryFilters,
): StoryEntry[] {
  const words = normalizeText(filters.query).split(/\s+/).filter(Boolean);
  const kept = entries.filter(
    (entry) =>
      matchesQuery(entry, words) &&
      (filters.age === null || entry.age === null || entry.age <= filters.age) &&
      (filters.category === null || entry.category?.trim() === filters.category),
  );
  return kept.sort(
    filters.sort === "recent"
      ? (a, b) => timeOf(b) - timeOf(a) || byTitle(a, b)
      : byTitle,
  );
}
