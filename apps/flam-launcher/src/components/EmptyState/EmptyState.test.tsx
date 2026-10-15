import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { EmptyState } from "./EmptyState";

it("affiche titre, description et action", () => {
  render(
    <EmptyState
      title="Aucune histoire ici"
      description="Ajoutez une source."
      action={<button type="button">Ajouter</button>}
    />,
  );

  expect(
    screen.getByRole("heading", { level: 2, name: "Aucune histoire ici" }),
  ).toBeVisible();
  expect(screen.getByText("Ajoutez une source.")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Ajouter" })).toBeInTheDocument();
});

it("adapte le niveau de titre", () => {
  render(<EmptyState title="Vide" headingLevel={3} />);
  expect(screen.getByRole("heading", { level: 3, name: "Vide" })).toBeInTheDocument();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(<EmptyState icon="folder" title="Vide" />);
  await expectComponentAccessible(container);
});
