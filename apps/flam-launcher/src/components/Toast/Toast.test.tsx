import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { expectNoAxeViolations, PAGE_LEVEL_RULES } from "../../test/axe";
import { DEFAULT_TOAST_DURATION, ToastProvider } from "./Toast";
import { useToast, type ToastOptions } from "./ToastContext";

afterEach(() => {
  vi.useRealTimers();
});

function Trigger(options: ToastOptions) {
  const toast = useToast();
  return (
    <button type="button" onClick={() => toast.show(options)}>
      Notifier
    </button>
  );
}

function renderWith(options: ToastOptions) {
  render(
    <ToastProvider>
      <Trigger {...options} />
    </ToastProvider>,
  );
  fireEvent.click(screen.getByRole("button", { name: "Notifier" }));
}

it("affiche la notification dans une zone annoncée poliment", () => {
  renderWith({ message: "Histoire prête.", tone: "success" });

  const region = screen.getByRole("region", { name: "Notifications" });
  expect(region).toHaveAttribute("aria-live", "polite");
  expect(region).toHaveTextContent("Histoire prête.");
});

it("annonce les erreurs comme alertes", () => {
  renderWith({ message: "Catalogue illisible.", tone: "danger" });
  expect(screen.getByRole("alert")).toHaveTextContent("Catalogue illisible.");
});

it("disparaît après la durée par défaut", () => {
  vi.useFakeTimers();
  renderWith({ message: "Éphémère" });

  expect(screen.getByText("Éphémère")).toBeInTheDocument();
  act(() => vi.advanceTimersByTime(DEFAULT_TOAST_DURATION));
  expect(screen.queryByText("Éphémère")).not.toBeInTheDocument();
});

it("reste affichée avec une durée nulle jusqu'à fermeture", () => {
  vi.useFakeTimers();
  renderWith({ message: "Persistante", duration: 0 });

  act(() => vi.advanceTimersByTime(DEFAULT_TOAST_DURATION * 4));
  expect(screen.getByText("Persistante")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Fermer" }));
  expect(screen.queryByText("Persistante")).not.toBeInTheDocument();
});

it("refuse useToast hors d'un ToastProvider", () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  expect(() => render(<Trigger message="x" />)).toThrow(/ToastProvider/);
});

it("n'a aucune violation axe", async () => {
  renderWith({ message: "Histoire prête.", tone: "info" });
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});
