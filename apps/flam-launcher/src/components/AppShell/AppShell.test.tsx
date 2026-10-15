import { fireEvent, render, screen } from "@testing-library/react";
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

it("focalise le contenu via le lien d'évitement sans toucher au hash", () => {
  window.location.hash = "#/demo";
  renderShell(false);

  const skip = screen.getByRole("link", { name: "Aller au contenu" });
  const notPrevented = fireEvent.click(skip);
  expect(notPrevented).toBe(false);
  expect(screen.getByRole("main")).toHaveFocus();
  expect(window.location.hash).toBe("#/demo");
  expect(screen.queryByRole("complementary")).not.toBeInTheDocument();
  window.location.hash = "";
});

it("n'a aucune violation axe", async () => {
  renderShell(true);
  await expectNoAxeViolations();
});
