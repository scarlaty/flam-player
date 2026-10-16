import { expect, it } from "vitest";
import type { StoryEntry } from "../../api/story";
import { makeEntry } from "../../test/entries";
import {
  applyFilters,
  availableAges,
  availableCategories,
  DEFAULT_FILTERS,
  filtersAreDefault,
  normalizeText,
} from "./filters";

const entries = [
  makeEntry({ title: "Zébulon le dragon", age: 5, category: "Aventure" }),
  makeEntry({
    title: "Éléonore et la lune",
    age: 3,
    category: "Contes",
    author: "Marie Dupré",
    updated_at: "2025-03-01T00:00:00Z",
  }),
  makeEntry({
    title: "Alice 10",
    age: 8,
    category: "Contes",
    updated_at: "2025-06-01T00:00:00Z",
  }),
  makeEntry({ title: "Alice 2", description: "Un voyage étonnant" }),
];

const titles = (list: StoryEntry[]) => list.map((e) => e.title);

it("normalise casse et accents", () => {
  expect(normalizeText("  ÉLÉonore Çà ")).toBe("eleonore ca");
});

it("trie par titre sans tenir compte des accents ni de la casse, chiffres dans l'ordre", () => {
  expect(titles(applyFilters(entries, DEFAULT_FILTERS))).toEqual([
    "Alice 2",
    "Alice 10",
    "Éléonore et la lune",
    "Zébulon le dragon",
  ]);
});

it("trie par date de mise à jour décroissante, sans date en dernier", () => {
  expect(titles(applyFilters(entries, { ...DEFAULT_FILTERS, sort: "recent" }))).toEqual([
    "Alice 10",
    "Éléonore et la lune",
    "Alice 2",
    "Zébulon le dragon",
  ]);
});

it("cherche dans le titre, l'auteur, la catégorie et la description", () => {
  const search = (query: string) =>
    titles(applyFilters(entries, { ...DEFAULT_FILTERS, query }));
  expect(search("eleonore")).toEqual(["Éléonore et la lune"]);
  expect(search("DUPRE")).toEqual(["Éléonore et la lune"]);
  expect(search("aventure")).toEqual(["Zébulon le dragon"]);
  expect(search("étonnant voyage")).toEqual(["Alice 2"]);
  expect(search("lune dragon")).toEqual([]);
  expect(search("   ")).toHaveLength(4);
});

it("filtre par âge en gardant les histoires sans âge", () => {
  const result = applyFilters(entries, { ...DEFAULT_FILTERS, age: 5 });
  expect(titles(result)).toEqual(["Alice 2", "Éléonore et la lune", "Zébulon le dragon"]);
});

it("filtre par catégorie et combine les filtres", () => {
  expect(
    titles(applyFilters(entries, { ...DEFAULT_FILTERS, category: "Contes" })),
  ).toEqual(["Alice 10", "Éléonore et la lune"]);
  expect(
    titles(applyFilters(entries, { ...DEFAULT_FILTERS, category: "Contes", age: 3 })),
  ).toEqual(["Éléonore et la lune"]);
});

it("liste âges et catégories disponibles", () => {
  expect(availableAges(entries)).toEqual([3, 5, 8]);
  expect(availableCategories(entries)).toEqual(["Aventure", "Contes"]);
});

it("reconnaît les filtres par défaut (le tri ne compte pas)", () => {
  expect(filtersAreDefault({ ...DEFAULT_FILTERS, sort: "recent" })).toBe(true);
  expect(filtersAreDefault({ ...DEFAULT_FILTERS, query: "x" })).toBe(false);
  expect(filtersAreDefault({ ...DEFAULT_FILTERS, age: 3 })).toBe(false);
});
