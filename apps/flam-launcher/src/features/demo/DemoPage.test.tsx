import { fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it } from "vitest";
import { expectNoAxeViolations } from "../../test/axe";
import { DemoPage } from "./DemoPage";

// axe parcourt toute la page dans jsdom : plus lent qu'un test de composant.
const AXE_TIMEOUT = 30_000;

afterEach(() => {
  document.documentElement.removeAttribute("data-theme");
});

it(
  "présente tous les composants sans violation axe",
  async () => {
    render(<DemoPage />);

    expect(
      screen.getByRole("heading", { level: 1, name: "Démonstration des composants" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Tâches en cours" })).toBeInTheDocument();
    await expectNoAxeViolations();
  },
  AXE_TIMEOUT,
);

it(
  "reste accessible avec le dialogue et une notification ouverts",
  async () => {
    render(<DemoPage />);

    fireEvent.click(screen.getByRole("button", { name: "Afficher une notification" }));
    fireEvent.click(screen.getByRole("button", { name: "Ouvrir un dialogue" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    await expectNoAxeViolations();
  },
  AXE_TIMEOUT,
);

it("applique le thème choisi sur la racine du document", () => {
  render(<DemoPage />);

  fireEvent.click(screen.getByRole("radio", { name: "Sombre" }));
  expect(document.documentElement).toHaveAttribute("data-theme", "dark");
  fireEvent.click(screen.getByRole("radio", { name: "Système" }));
  expect(document.documentElement).not.toHaveAttribute("data-theme");
});
