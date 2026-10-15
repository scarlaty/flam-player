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

const liveRegion = (text: string) => screen.getByText(text).closest("[aria-live]");
const advance = (ms: number) => act(() => vi.advanceTimersByTime(ms));

it("annonce les infos dans une région polie, sans rôle supplémentaire", () => {
  renderWith({ message: "Histoire prête.", tone: "success" });

  expect(screen.getByRole("region", { name: "Notifications" })).toBeInTheDocument();
  expect(liveRegion("Histoire prête.")).toHaveAttribute("aria-live", "polite");
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByRole("status")).toBeNull();
});

it("annonce les erreurs dans une région assertive séparée", () => {
  renderWith({ message: "Catalogue illisible.", tone: "danger" });
  expect(liveRegion("Catalogue illisible.")).toHaveAttribute("aria-live", "assertive");
});

it("disparaît après la durée par défaut", () => {
  vi.useFakeTimers();
  renderWith({ message: "Éphémère" });

  advance(DEFAULT_TOAST_DURATION - 1);
  expect(screen.getByText("Éphémère")).toBeInTheDocument();
  advance(1);
  expect(screen.queryByText("Éphémère")).not.toBeInTheDocument();
});

it("garde les erreurs jusqu'à fermeture, quelle que soit la durée demandée", () => {
  vi.useFakeTimers();
  renderWith({ message: "Erreur durable", tone: "danger", duration: 1000 });

  advance(DEFAULT_TOAST_DURATION * 4);
  expect(screen.getByText("Erreur durable")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Fermer" }));
  expect(screen.queryByText("Erreur durable")).not.toBeInTheDocument();
});

it("met la minuterie en pause au survol", () => {
  vi.useFakeTimers();
  renderWith({ message: "Survolée", duration: 1000 });
  const toast = screen.getByText("Survolée").closest(".fl-toast")!;

  advance(600);
  fireEvent.mouseEnter(toast);
  advance(5000);
  expect(screen.getByText("Survolée")).toBeInTheDocument();
  fireEvent.mouseLeave(toast);
  advance(399);
  expect(screen.getByText("Survolée")).toBeInTheDocument();
  advance(1);
  expect(screen.queryByText("Survolée")).not.toBeInTheDocument();
});

it("met la minuterie en pause tant que le focus est dans la notification", () => {
  vi.useFakeTimers();
  renderWith({ message: "Focalisée", duration: 1000 });
  const close = screen.getByRole("button", { name: "Fermer" });

  act(() => close.focus());
  advance(5000);
  expect(screen.getByText("Focalisée")).toBeInTheDocument();
  act(() => close.blur());
  advance(1000);
  expect(screen.queryByText("Focalisée")).not.toBeInTheDocument();
});

it("refuse useToast hors d'un ToastProvider", () => {
  vi.spyOn(console, "error").mockImplementation(() => {});
  expect(() => render(<Trigger message="x" />)).toThrow(/ToastProvider/);
});

it("n'a aucune violation axe", async () => {
  renderWith({ message: "Histoire prête.", tone: "info" });
  fireEvent.click(screen.getByRole("button", { name: "Notifier" }));
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});
