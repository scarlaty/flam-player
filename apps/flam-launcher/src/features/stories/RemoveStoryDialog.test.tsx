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

it("pendant la suppression, seul « Supprimer » est refusé : on peut toujours fermer (#581)", () => {
  const onCancel = vi.fn();
  const onConfirm = vi.fn();
  render(ui(true, onCancel, onConfirm));
  fireEvent.click(screen.getByRole("button", { name: "Supprimer" }));
  expect(onConfirm).not.toHaveBeenCalled();
  expect(screen.getByRole("button", { name: "Supprimer" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
  fireEvent.keyDown(document, { key: "Escape" });
  fireEvent.click(screen.getByRole("button", { name: "Fermer" }));
  expect(onCancel).toHaveBeenCalledTimes(3);
});
