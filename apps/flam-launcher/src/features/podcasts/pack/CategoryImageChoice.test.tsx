import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { EpisodeRef, PackNode } from "../../../api/pack";
import { expectComponentAccessible } from "../../../test/axe";
import { CategoryImageChoice } from "./CategoryImageChoice";
import { PackTree } from "./PackTree";

/** PNG 1 × 1 transparent. */
const PIXEL =
  "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
const ref = (id: string): EpisodeRef => ({ source_id: "s", episode_id: id });
/** Vignettes affichées : décoratives (`alt=""`), donc sans rôle `img`. */
const thumbs = () => document.querySelectorAll<HTMLImageElement>(".fl-pack__thumb img");
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
        thumbnailOf={() => Promise.resolve(PIXEL)}
      />,
    );
    await waitFor(() => expect(thumbs()).toHaveLength(3));
    await expectComponentAccessible(container);
  });

  it("distingue deux épisodes de même titre par leur rang", () => {
    render(
      <CategoryImageChoice
        categoryLabel="Saison"
        candidates={[
          { ref: ref("a"), title: "Conte" },
          { ref: ref("b"), title: "Autre" },
          { ref: ref("c"), title: "Conte" },
        ]}
        value={null}
        onChange={vi.fn()}
        thumbnailOf={() => Promise.resolve(null)}
      />,
    );
    expect(screen.getByRole("radio", { name: "Conte (n° 1)" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Conte (n° 3)" })).toBeInTheDocument();
    expect(screen.getByRole("radio", { name: "Autre" })).toBeInTheDocument();
  });

  describe("aperçus des pochettes", () => {
    it("affiche la vignette de chaque épisode, décorative (alt vide)", async () => {
      const load = vi.fn((r: EpisodeRef) =>
        Promise.resolve(r.episode_id === "b" ? null : PIXEL),
      );
      render(
        <CategoryImageChoice
          categoryLabel="Saison"
          candidates={candidates}
          value={null}
          onChange={vi.fn()}
          thumbnailOf={load}
        />,
      );
      await waitFor(() => expect(thumbs()).toHaveLength(2));
      for (const img of thumbs()) {
        expect(img).toHaveAttribute("src", PIXEL);
        // Décorative : le libellé du radio porte le titre (pas de « Premier Premier »).
        expect(img).toHaveAttribute("alt", "");
        expect(img).not.toHaveAttribute("loading");
      }
      expect(screen.queryByRole("img")).toBeNull();
      // Sans aperçu : le libellé seul, le choix reste possible.
      expect(screen.getByRole("radio", { name: "Premier" })).toBeInTheDocument();
      expect(screen.getByRole("radio", { name: "Deuxième" })).toBeInTheDocument();
      expect(load).toHaveBeenCalledTimes(3);
    });

    it("ne demande aucun aperçu pour un épisode hors écran", async () => {
      const observed: IntersectionObserverCallback[] = [];
      class FakeObserver {
        constructor(callback: IntersectionObserverCallback) {
          observed.push(callback);
        }
        observe() {}
        disconnect() {}
      }
      vi.stubGlobal("IntersectionObserver", FakeObserver);
      try {
        const load = vi.fn(() => Promise.resolve(PIXEL));
        render(
          <CategoryImageChoice
            categoryLabel="Saison"
            candidates={candidates}
            value={null}
            onChange={vi.fn()}
            thumbnailOf={load}
          />,
        );
        await waitFor(() => expect(observed).toHaveLength(3));
        expect(load).not.toHaveBeenCalled();
        // Le premier approche de l'écran : seul lui est demandé.
        observed[0]!(
          [{ isIntersecting: true } as IntersectionObserverEntry],
          {} as IntersectionObserver,
        );
        await waitFor(() => expect(load).toHaveBeenCalledTimes(1));
        expect(load).toHaveBeenCalledWith(ref("a"));
      } finally {
        vi.unstubAllGlobals();
      }
    });

    it("replie sur le libellé seul quand la lecture de l'aperçu échoue", async () => {
      const load = vi.fn(() => Promise.reject(new Error("boum")));
      render(
        <CategoryImageChoice
          categoryLabel="Saison"
          candidates={candidates}
          value={null}
          onChange={vi.fn()}
          thumbnailOf={load}
        />,
      );
      await waitFor(() => expect(load).toHaveBeenCalledTimes(3));
      await waitFor(() =>
        expect(document.querySelectorAll(".fl-pack__thumb")).toHaveLength(0),
      );
      expect(screen.queryByRole("img")).toBeNull();
      expect(screen.getAllByRole("radio")).toHaveLength(4);
    });

    it("garde les boutons radio natifs et le clavier avec les vignettes", async () => {
      const onChange = vi.fn();
      render(
        <CategoryImageChoice
          categoryLabel="Saison"
          candidates={candidates}
          value={null}
          onChange={onChange}
          thumbnailOf={() => Promise.resolve(PIXEL)}
        />,
      );
      await waitFor(() => expect(thumbs()).toHaveLength(3));
      const radios = screen.getAllByRole<HTMLInputElement>("radio");
      expect(radios.every((r) => r.type === "radio")).toBe(true);
      // Une seule tabulation pour le groupe, focus possible sur chaque radio.
      expect(radios.every((r) => r.tabIndex === 0)).toBe(true);
      radios[1]!.focus();
      expect(radios[1]).toHaveFocus();
      fireEvent.click(radios[1]!);
      expect(onChange).toHaveBeenLastCalledWith(ref("a"));
      // Cliquer sur la vignette (dans le libellé) choisit l'épisode.
      fireEvent.click(thumbs()[2]!);
      expect(onChange).toHaveBeenLastCalledWith(ref("c"));
    });
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
