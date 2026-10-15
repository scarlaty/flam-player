import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { expect, it, vi } from "vitest";
import { expectNoAxeViolations, PAGE_LEVEL_RULES } from "../../test/axe";
import { Dialog } from "./Dialog";

function Harness({ onClose = () => {} }: { onClose?: () => void }) {
  const [open, setOpen] = useState(false);
  const close = () => {
    onClose();
    setOpen(false);
  };
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Ouvrir
      </button>
      <Dialog
        open={open}
        onClose={close}
        title="Installer l'histoire ?"
        footer={
          <button type="button" onClick={close}>
            Installer
          </button>
        }
      >
        <p>Texte explicatif.</p>
      </Dialog>
    </>
  );
}

it("ne rend rien tant qu'il est fermé", () => {
  render(<Harness />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("s'ouvre avec le focus à l'intérieur et un nom accessible", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Ouvrir" }));

  const dialog = screen.getByRole("dialog", { name: "Installer l'histoire ?" });
  expect(dialog).toHaveAttribute("aria-modal", "true");
  expect(dialog).toHaveAccessibleDescription("Texte explicatif.");
  expect(screen.getByRole("button", { name: "Installer" })).toHaveFocus();
});

it("confine Tab dans le dialogue", () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Ouvrir" }));
  const close = screen.getByRole("button", { name: "Fermer" });
  const install = screen.getByRole("button", { name: "Installer" });

  fireEvent.keyDown(install, { key: "Tab" });
  expect(close).toHaveFocus();
  fireEvent.keyDown(close, { key: "Tab", shiftKey: true });
  expect(install).toHaveFocus();
});

it("se ferme sur Échap et rend le focus au déclencheur", () => {
  const onClose = vi.fn();
  render(<Harness onClose={onClose} />);
  const trigger = screen.getByRole("button", { name: "Ouvrir" });
  trigger.focus();
  fireEvent.click(trigger);

  fireEvent.keyDown(screen.getByRole("dialog"), { key: "Escape" });
  expect(onClose).toHaveBeenCalledOnce();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(trigger).toHaveFocus();
});

it("se ferme au clic sur le fond, pas sur le panneau", () => {
  const onClose = vi.fn();
  render(<Harness onClose={onClose} />);
  fireEvent.click(screen.getByRole("button", { name: "Ouvrir" }));

  fireEvent.mouseDown(screen.getByRole("dialog"));
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.mouseDown(screen.getByRole("dialog").parentElement!);
  expect(onClose).toHaveBeenCalledOnce();
});

it("n'a aucune violation axe une fois ouvert", async () => {
  render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "Ouvrir" }));
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});
