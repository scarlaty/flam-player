import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { EpisodeRef, PackNode } from "../../../api/pack";
import { expectComponentAccessible } from "../../../test/axe";
import { CategoryImageChoice } from "./CategoryImageChoice";
import { PackTree } from "./PackTree";

const ref = (id: string): EpisodeRef => ({ source_id: "s", episode_id: id });
const candidates = [
  { ref: ref("a"), title: "Premier" },
  { ref: ref("b"), title: "Deuxième" },
  { ref: ref("c"), title: "Troisième" },
];

describe("CategoryImageChoice", () => {
  it("propose Automatique, coché par défaut, puis chaque épisode", () => {
    render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates}
        value={null}
        onChange={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("group", { name: "Image de la catégorie « Saison »" }),
    ).toBeInTheDocument();
    expect(screen.getAllByRole("radio")).toHaveLength(4);
    expect(screen.getByRole("radio", { name: /Automatique/ })).toBeChecked();
  });

  it("signale l'épisode choisi, puis le retour à Automatique", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates}
        value={null}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("radio", { name: "Deuxième" }));
    expect(onChange).toHaveBeenLastCalledWith(ref("b"));
    rerender(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates}
        value={ref("b")}
        onChange={onChange}
      />,
    );
    expect(screen.getByRole("radio", { name: "Deuxième" })).toBeChecked();
    fireEvent.click(screen.getByRole("radio", { name: /Automatique/ }));
    expect(onChange).toHaveBeenLastCalledWith(null);
  });

  it("est un groupe de boutons radio natifs : une tabulation, au clavier", () => {
    render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates}
        value={ref("c")}
        onChange={vi.fn()}
      />,
    );
    const radios = screen.getAllByRole<HTMLInputElement>("radio");
    // Même groupe : les flèches changent de choix, la tabulation n'en visite qu'un.
    expect(new Set(radios.map((r) => r.name)).size).toBe(1);
    radios[3]!.focus();
    expect(radios[3]).toHaveFocus();
    expect(radios[3]).toBeChecked();
    expect(radios.every((r) => r.tabIndex === 0)).toBe(true);
  });

  it("traite un choix périmé comme Automatique", () => {
    render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates}
        value={ref("disparu")}
        onChange={vi.fn()}
      />,
    );
    expect(screen.getByRole("radio", { name: /Automatique/ })).toBeChecked();
  });

  it("ne s'affiche pas quand il n'y a rien à choisir", () => {
    const { container } = render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates.slice(0, 1)}
        value={null}
        onChange={vi.fn()}
      />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("n'a aucune violation axe", async () => {
    const { container } = render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={candidates}
        value={ref("a")}
        onChange={vi.fn()}
      />,
    );
    await expectComponentAccessible(container);
  });
});

describe("choix d'image dans l'arborescence", () => {
  const episode = (id: string): PackNode => ({
    kind: "episode",
    source_id: "s",
    episode_id: id,
  });
  const menu: PackNode[] = [
    episode("z"),
    { kind: "category", name: "Saison", entries: [episode("a"), episode("b")] },
  ];

  function renderTree(onChange: (next: PackNode[], message: string) => void) {
    render(
      <PackTree
        menu={menu}
        titleOf={(r) => `Titre ${r.episode_id}`}
        isUnavailable={() => false}
        hasImage={(r) => r.episode_id !== "z"}
        onChange={onChange}
        onRefuse={vi.fn()}
      />,
    );
  }

  it("enregistre le choix dans la catégorie et l'annonce", () => {
    const onChange = vi.fn();
    renderTree(onChange);
    fireEvent.click(screen.getByRole("radio", { name: "Titre b" }));
    const [next, message] = onChange.mock.calls[0]!;
    expect(next[1]).toEqual({
      kind: "category",
      name: "Saison",
      entries: [episode("a"), episode("b")],
      image: ref("b"),
    });
    expect(message).toContain("Saison");
  });

  it("n'affiche aucun choix sans pochette connue", () => {
    render(
      <PackTree
        menu={menu}
        titleOf={() => "Titre"}
        isUnavailable={() => false}
        onChange={vi.fn()}
        onRefuse={vi.fn()}
      />,
    );
    expect(screen.queryByRole("radio")).toBeNull();
  });
});
