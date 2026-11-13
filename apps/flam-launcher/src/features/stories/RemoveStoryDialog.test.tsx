import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { RemoveStoryDialog } from "./RemoveStoryDialog";

function ui(busy: boolean, onCancel = () => {}, onConfirm = () => {}) {
  return (
    <RemoveStoryDialog
      open
      title="Capi"
      busy={busy}
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}

it("annule sur Annuler et sur Échap hors suppression", () => {
  const onCancel = vi.fn();
  render(ui(false, onCancel));
  fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
  fireEvent.keyDown(document, { key: "Escape" });
  expect(onCancel).toHaveBeenCalledTimes(2);
});

it("pendant la suppression : « Annuler » devient « Fermer », le statut est annoncé, « Supprimer » refusé (#581)", () => {
  const onCancel = vi.fn();
  const onConfirm = vi.fn();
  render(ui(true, onCancel, onConfirm));
  expect(screen.queryByRole("button", { name: "Annuler" })).toBeNull();
  expect(screen.getAllByRole("button", { name: "Fermer" })).toHaveLength(2);
  expect(screen.getByRole("status")).toHaveTextContent("Suppression en cours.");
  const remove = screen.getByRole("button", { name: "Supprimer" });
  expect(remove).toHaveAccessibleDescription("Suppression en cours.");
  expect(remove).toHaveAttribute("aria-disabled", "true");
  expect(remove).toHaveAttribute("aria-busy", "true");
  fireEvent.click(remove);
  expect(onConfirm).not.toHaveBeenCalled();
});

it("pendant la suppression, on peut toujours fermer : Fermer, Échap et fond (#581)", () => {
  const onCancel = vi.fn();
  render(ui(true, onCancel));
  for (const button of screen.getAllByRole("button", { name: "Fermer" }))
    fireEvent.click(button);
  fireEvent.keyDown(document, { key: "Escape" });
  fireEvent.mouseDown(screen.getByRole("alertdialog").parentElement!);
  expect(onCancel).toHaveBeenCalledTimes(4);
});

it("hors suppression : « Annuler », aucun statut annoncé", () => {
  render(ui(false));
  expect(screen.getByRole("button", { name: "Annuler" })).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("");
  expect(screen.getByRole("button", { name: "Supprimer" })).not.toHaveAttribute(
    "aria-describedby",
  );
});
