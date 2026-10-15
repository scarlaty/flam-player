import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { IconButton } from "../IconButton/IconButton";
import { placeholderTone, StoryCard } from "./StoryCard";

it("ouvre la fiche au clic sur le titre", () => {
  const onOpen = vi.fn();
  render(<StoryCard title="Pipo le petit nuage" ageMin={2} ageMax={4} onOpen={onOpen} />);

  fireEvent.click(screen.getByRole("button", { name: "Pipo le petit nuage" }));
  expect(onOpen).toHaveBeenCalledOnce();
  expect(screen.getByText("2-4 ans")).toBeInTheDocument();
});

it("affiche les badges, l'âge minimal et l'état sélectionné", () => {
  render(
    <StoryCard
      title="Le grand voyage de Mila"
      ageMin={5}
      selected
      onOpen={() => {}}
      badges={[
        { id: "a", label: "Compatible", tone: "warning" },
        { id: "b", label: "Compatible" },
      ]}
    />,
  );

  expect(screen.getByText("Dès 5 ans")).toBeInTheDocument();
  // Deux badges de même libellé : les clés stables évitent toute collision.
  expect(screen.getAllByRole("listitem")).toHaveLength(2);
  expect(screen.getByRole("button", { name: "Le grand voyage de Mila" })).toHaveAttribute(
    "aria-current",
    "true",
  );
});

it("utilise la vignette fournie comme image décorative", () => {
  const { container } = render(<StoryCard title="Avec vignette" coverUrl="data:," />);

  expect(container.querySelector("img")).toHaveAttribute("alt", "");
  expect(container.querySelector(".fl-story-card__placeholder")).toBeNull();
});

it("se replie sur l'illustration si la vignette ne charge pas", () => {
  const { container } = render(<StoryCard title="Image cassée" coverUrl="data:x" />);

  fireEvent.error(container.querySelector("img")!);
  expect(container.querySelector("img")).toBeNull();
  expect(container.querySelector(".fl-story-card__placeholder")).not.toBeNull();
});

it("affiche la progression d'une opération en cours", () => {
  const { rerender } = render(
    <StoryCard title="Mila" progress={40} progressText="Téléchargement…" />,
  );

  expect(screen.getByRole("progressbar", { name: "Progression : Mila" })).toHaveAttribute(
    "aria-valuenow",
    "40",
  );
  expect(screen.getByText("Téléchargement…")).toBeInTheDocument();

  rerender(<StoryCard title="Mila" progress="indeterminate" />);
  expect(screen.getByRole("progressbar")).not.toHaveAttribute("aria-valuenow");

  rerender(<StoryCard title="Mila" />);
  expect(screen.queryByRole("progressbar")).toBeNull();
});

it("choisit une couleur d'illustration stable pour un titre", () => {
  expect(placeholderTone("Toc toc")).toBe(placeholderTone("Toc toc"));
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <StoryCard
      title="Bonne nuit, petit ours"
      ageMin={2}
      onOpen={() => {}}
      progress={10}
      badges={[{ id: "installed", label: "Installée", tone: "success", icon: "check" }]}
      actions={<IconButton icon="play" label="Jouer : Bonne nuit, petit ours" />}
    />,
  );
  await expectComponentAccessible(container);
});
