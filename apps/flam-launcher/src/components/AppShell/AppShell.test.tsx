import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { expectNoAxeViolations } from "../../test/axe";
import { AppShell } from "./AppShell";

function renderShell(withAside: boolean) {
  return render(
    <AppShell
      title="Bibliothèque"
      sidebar={<p>Sources</p>}
      headerActions={<button type="button">Actualiser</button>}
      aside={withAside ? <p>Tâches</p> : undefined}
    >
      <p>Contenu</p>
    </AppShell>,
  );
}

it("structure la page en zones repérables", () => {
  renderShell(true);

  expect(
    screen.getByRole("navigation", { name: "Sources d'histoires" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("banner")).toContainElement(
    screen.getByRole("heading", { level: 1, name: "Bibliothèque" }),
  );
  expect(screen.getByRole("main")).toHaveTextContent("Contenu");
  expect(screen.getByRole("complementary")).toHaveTextContent("Tâches");
});

it("propose un lien d'évitement vers le contenu", () => {
  renderShell(false);

  const skip = screen.getByRole("link", { name: "Aller au contenu" });
  expect(skip).toHaveAttribute("href", `#${screen.getByRole("main").id}`);
  expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
});

it("n'a aucune violation axe", async () => {
  renderShell(true);
  await expectNoAxeViolations();
});
