import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { Badge } from "./Badge";

it("affiche le texte du badge avec son ton", () => {
  render(
    <Badge tone="success" icon="check">
      Installée
    </Badge>,
  );

  expect(screen.getByText("Installée")).toHaveClass("fl-badge", "fl-badge--success");
});

it("utilise le ton neutre par défaut", () => {
  render(<Badge>Disponible</Badge>);
  expect(screen.getByText("Disponible")).toHaveClass("fl-badge--neutral");
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <Badge tone="danger" icon="alert">
        Incompatible
      </Badge>
      <Badge tone="info">Compatible</Badge>
    </div>,
  );
  await expectComponentAccessible(container);
});

it("avec description : badge focalisable, info-bulle lue une seule fois, fermable par Échap", async () => {
  const { container } = render(
    <Badge tone="info" description="Le fichier a changé.">
      Mise à jour
    </Badge>,
  );
  const badge = screen.getByText("Mise à jour");
  expect(badge).toHaveAttribute("tabindex", "0");
  expect(badge).not.toHaveAttribute("title");
  // Le nom du badge ne contient pas la description : pas de lecture en double.
  expect(badge).toHaveTextContent(/^Mise à jour$/);
  const tooltip = document.body.querySelector('[role="tooltip"]');
  expect(badge).toHaveAttribute("aria-describedby", tooltip?.id);
  expect(tooltip).toHaveTextContent("Le fichier a changé.");
  expect(tooltip).not.toBeVisible();
  // Hors de la carte survolée (transformée) : directement sous le body.
  expect(tooltip?.parentElement).toBe(document.body);

  fireEvent.focus(badge);
  expect(tooltip).toBeVisible();
  fireEvent.keyDown(badge, { key: "Escape" });
  expect(tooltip).not.toBeVisible();

  fireEvent.mouseEnter(badge);
  expect(tooltip).toBeVisible();
  fireEvent.mouseLeave(badge);
  expect(tooltip).not.toBeVisible();
  await expectComponentAccessible(container);
});
