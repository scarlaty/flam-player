import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { Banner } from "./Banner";

it("annonce une erreur avec le rôle alert", () => {
  render(<Banner tone="danger">Échec</Banner>);
  expect(screen.getByRole("alert")).toHaveTextContent("Échec");
});

it("utilise le rôle status pour un avertissement et affiche titre et action", () => {
  render(
    <Banner tone="warning" title="Attention" action={<button>OK</button>}>
      Détail
    </Banner>,
  );

  const banner = screen.getByRole("status");
  expect(banner).toHaveClass("fl-banner--warning");
  expect(banner).toHaveTextContent("Attention");
  expect(banner).toHaveTextContent("Détail");
  expect(screen.getByRole("button", { name: "OK" })).toBeInTheDocument();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <Banner tone="info">Info</Banner>
      <Banner tone="danger" title="Erreur">
        Détail
      </Banner>
    </div>,
  );
  await expectComponentAccessible(container);
});

it('role="none" : aucun rôle quand une autre région vivante annonce le message', () => {
  render(
    <Banner tone="success" role="none">
      Déjà annoncé
    </Banner>,
  );
  expect(screen.queryByRole("status")).toBeNull();
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByText("Déjà annoncé")).toBeVisible();
});
