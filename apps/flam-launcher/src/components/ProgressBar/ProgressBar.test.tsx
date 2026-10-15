import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { ProgressBar } from "./ProgressBar";

it("expose les valeurs ARIA d'une progression déterminée", () => {
  render(<ProgressBar value={42.6} label="Téléchargement" showValue />);

  const bar = screen.getByRole("progressbar", { name: "Téléchargement" });
  expect(bar).toHaveAttribute("aria-valuemin", "0");
  expect(bar).toHaveAttribute("aria-valuemax", "100");
  expect(bar).toHaveAttribute("aria-valuenow", "43");
  expect(screen.getByText("43 %")).toBeInTheDocument();
});

it.each([
  [-12, "0"],
  [0, "0"],
  [100, "100"],
  [142.4, "100"],
])("borne %s à %s", (value, expected) => {
  render(<ProgressBar value={value} label="Copie" />);
  expect(screen.getByRole("progressbar")).toHaveAttribute("aria-valuenow", expected);
});

it("omet aria-valuenow et le pourcentage quand la progression est indéterminée", () => {
  const { container } = render(<ProgressBar label="Conversion" showValue />);

  const bar = screen.getByRole("progressbar", { name: "Conversion" });
  expect(bar).not.toHaveAttribute("aria-valuenow");
  expect(bar).toHaveClass("fl-progress__track--indeterminate");
  expect(container).not.toHaveTextContent("%");
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <ProgressBar value={40} label="Téléchargement" showValue />
      <ProgressBar label="Conversion" />
    </div>,
  );
  await expectComponentAccessible(container);
});
