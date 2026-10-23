import { describe, expect, it } from "vitest";
import type { PackNode } from "../../../api/pack";
import {
  canPlace,
  categoryNode,
  destinationsFor,
  episodeNode,
  episodesOf,
  menuHeight,
  move,
  nodeAt,
  removeAt,
  removeEpisode,
  renameAt,
  shift,
  tryAppend,
  MAX_OPTIONS_PER_MENU,
} from "./tree";

const ep = (id: string): PackNode => episodeNode({ source_id: "s", episode_id: id });
const cat = (name: string, ...entries: PackNode[]): PackNode => ({
  kind: "category",
  name,
  entries,
});
const ids = (menu: readonly PackNode[]): string[] =>
  menu.map((n) => (n.kind === "episode" ? n.episode_id : `[${n.name}]`));

describe("arborescence du pack", () => {
  it("monte, descend et ne dépasse pas les bords", () => {
    const menu = [ep("a"), ep("b"), ep("c")];
    expect(ids(shift(menu, [1], -1))).toEqual(["b", "a", "c"]);
    expect(ids(shift(menu, [1], 1))).toEqual(["a", "c", "b"]);
    expect(ids(shift(menu, [0], -1))).toEqual(["a", "b", "c"]);
    expect(ids(shift(menu, [2], 1))).toEqual(["a", "b", "c"]);
  });

  it("renomme et supprime dans une catégorie imbriquée", () => {
    const menu = [cat("A", cat("B", ep("x"), ep("y")))];
    const renamed = renameAt(menu, [0, 0], "Neuf");
    expect(nodeAt(renamed, [0, 0])).toMatchObject({ name: "Neuf" });
    expect(
      ids((nodeAt(removeAt(menu, [0, 0, 0]), [0, 0]) as { entries: PackNode[] }).entries),
    ).toEqual(["y"]);
  });

  it("déplace un épisode dans une catégorie, puis avant un autre nœud", () => {
    const menu = [ep("a"), cat("C", ep("x")), ep("b")];
    const into = move(menu, [0], [1])!;
    expect(ids(into)).toEqual(["[C]", "b"]);
    expect(ids((into[0] as { entries: PackNode[] }).entries)).toEqual(["x", "a"]);
    const before = move(menu, [2], [], 0)!;
    expect(ids(before)).toEqual(["b", "a", "[C]"]);
    const after = move(menu, [0], [], 3)!;
    expect(ids(after)).toEqual(["[C]", "b", "a"]);
  });

  it("recule l'indice de destination quand le nœud précède sa catégorie cible", () => {
    const menu = [cat("A", ep("x")), cat("B", ep("y"))];
    // Déplace la catégorie A... impossible dans B ? Non : profondeur 2 sur 3, permis.
    const moved = move(menu, [0], [1])!;
    expect(ids(moved)).toEqual(["[B]"]);
    const inner = moved[0] as { entries: PackNode[] };
    expect(ids(inner.entries)).toEqual(["y", "[A]"]);
  });

  it("refuse de déplacer une catégorie dans elle-même ou son descendant", () => {
    const menu = [cat("A", cat("B", ep("x")))];
    expect(move(menu, [0], [0])).toBeNull();
    expect(move(menu, [0], [0, 0])).toBeNull();
  });

  it("refuse un quatrième niveau de menus (N2 b)", () => {
    const menu = [cat("A", cat("B")), cat("C", ep("x"))];
    expect(menuHeight(menu)).toBe(3);
    // Catégorie à un niveau 3 : refusée.
    expect(tryAppend(menu, [0, 0], categoryNode("D"))).toBeNull();
    // Épisode au niveau 3 : accepté.
    expect(tryAppend(menu, [0, 0], ep("z"))).not.toBeNull();
    // Déplacer C (hauteur 2) dans B (niveau 3) : refusé.
    expect(move(menu, [1], [0, 0])).toBeNull();
    // Mais A (hauteur 3) dans C (niveau 2) : refusé aussi.
    expect(canPlace(menu, [1], menu[0]!, [0])).toBe(false);
  });

  it("refuse la 51e option d'un menu mais autorise le réordonnancement dans un menu plein", () => {
    const full = Array.from({ length: MAX_OPTIONS_PER_MENU }, (_, i) => ep(`e${i}`));
    expect(tryAppend(full, [], ep("de-plus"))).toBeNull();
    expect(move(full, [0], [], 10)).not.toBeNull();
  });

  it("liste les destinations valides d'un nœud", () => {
    const menu = [ep("a"), cat("A", cat("B")), cat("")];
    const labels = destinationsFor(menu, [0], "Racine", "(sans nom)").map((d) => d.label);
    expect(labels).toEqual(["Racine > A", "Racine > A > B", "Racine > (sans nom)"]);
    // Catégorie A (hauteur 2) : seule la racine (son menu actuel est exclu) -> aucune.
    expect(destinationsFor(menu, [1], "Racine", "(sans nom)")).toEqual([]);
  });

  it("retrouve et retire les épisodes", () => {
    const menu = [ep("a"), cat("A", ep("b"), ep("a"))];
    expect(episodesOf(menu).map((e) => e.episode_id)).toEqual(["a", "b", "a"]);
    expect(
      episodesOf(removeEpisode(menu, { source_id: "s", episode_id: "a" })),
    ).toHaveLength(1);
  });
});
