import { render, screen } from "@testing-library/react";
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
