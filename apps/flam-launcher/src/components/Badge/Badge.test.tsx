import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { ProgressBar } from "../ProgressBar/ProgressBar";
import { Badge } from "./Badge";

it("affiche le texte du badge avec son ton", () => {
  render(
    <Badge tone="success" icon="check">
      Installée
    </Badge>,
  );

  expect(screen.getByText("Installée")).toHaveClass("fl-badge", "fl-badge--success");
});

it("expose une barre de progression bornée entre 0 et 100", () => {
  render(<ProgressBar value={142.4} label="Téléchargement" showValue />);

  const bar = screen.getByRole("progressbar", { name: "Téléchargement" });
  expect(bar).toHaveAttribute("aria-valuenow", "100");
  expect(screen.getByText("100 %")).toBeInTheDocument();
});

it("omet aria-valuenow quand la progression est indéterminée", () => {
  render(<ProgressBar label="Conversion" showValue />);

  const bar = screen.getByRole("progressbar", { name: "Conversion" });
  expect(bar).not.toHaveAttribute("aria-valuenow");
  expect(screen.queryByText(/%/)).not.toBeInTheDocument();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <Badge tone="danger">Incompatible</Badge>
      <ProgressBar value={40} label="Téléchargement" />
      <ProgressBar label="Conversion" />
    </div>,
  );
  await expectComponentAccessible(container);
});
