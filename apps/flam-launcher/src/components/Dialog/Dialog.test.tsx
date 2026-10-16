import { fireEvent, render, screen } from "@testing-library/react";
import { useRef, useState } from "react";
import { expect, it, vi } from "vitest";
import { expectNoAxeViolations, PAGE_LEVEL_RULES } from "../../test/axe";
import { Dialog, type DialogProps } from "./Dialog";

type HarnessProps = Partial<Pick<DialogProps, "role" | "description">> & {
  onClose?: () => void;
  focusCancel?: boolean;
};

function Harness({ onClose = () => {}, focusCancel = false, ...props }: HarnessProps) {
  const [open, setOpen] = useState(false);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const close = () => {
    onClose();
    setOpen(false);
  };
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>
        Ouvrir
      </button>
      <button type="button">Dehors</button>
      <Dialog
        {...props}
        open={open}
        onClose={close}
        title="Installer l'histoire ?"
        initialFocus={focusCancel ? cancelRef : undefined}
        footer={
          <>
            <button type="button" ref={cancelRef} onClick={close}>
              Annuler
            </button>
            <button type="button" onClick={close}>
              Installer
            </button>
          </>
        }
      >
        <p>Texte explicatif.</p>
      </Dialog>
    </>
  );
}

const open = () => fireEvent.click(screen.getByRole("button", { name: "Ouvrir" }));

it("ne rend rien tant qu'il est fermé", () => {
  render(<Harness />);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("s'ouvre avec le focus sur la première action et un nom accessible", () => {
  render(<Harness />);
  open();

  const dialog = screen.getByRole("dialog", { name: "Installer l'histoire ?" });
  expect(dialog).toHaveAttribute("aria-modal", "true");
  expect(dialog).not.toHaveAttribute("aria-describedby");
  expect(screen.getByRole("button", { name: "Annuler" })).toHaveFocus();
});

it("relie uniquement la description fournie", () => {
  render(<Harness description="Elle sera convertie." />);
  open();

  expect(screen.getByRole("dialog")).toHaveAccessibleDescription("Elle sera convertie.");
});

it("propose la variante alertdialog et un focus initial choisi", () => {
  render(<Harness role="alertdialog" focusCancel />);
  open();

  expect(
    screen.getByRole("alertdialog", { name: "Installer l'histoire ?" }),
  ).toBeVisible();
  expect(screen.getByRole("button", { name: "Annuler" })).toHaveFocus();
});

it("confine Tab dans le dialogue", () => {
  render(<Harness />);
  open();
  const close = screen.getByRole("button", { name: "Fermer" });
  const install = screen.getByRole("button", { name: "Installer" });

  install.focus();
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

it("se ferme sur Échap même si le focus a quitté le panneau", () => {
  const onClose = vi.fn();
  render(<Harness onClose={onClose} />);
  open();

  const outside = screen.getByRole("button", { name: "Dehors", hidden: true });
  outside.focus();
  fireEvent.keyDown(outside, { key: "Escape" });
  expect(onClose).toHaveBeenCalledOnce();
});

it("bloque le défilement de la page tant qu'il est ouvert", () => {
  render(<Harness />);
  open();
  expect(document.body.style.overflow).toBe("hidden");

  fireEvent.keyDown(document, { key: "Escape" });
  expect(document.body.style.overflow).toBe("");
});

it("se ferme au clic sur le fond, pas sur le panneau", () => {
  const onClose = vi.fn();
  render(<Harness onClose={onClose} />);
  open();

  fireEvent.mouseDown(screen.getByRole("dialog"));
  expect(onClose).not.toHaveBeenCalled();
  fireEvent.mouseDown(screen.getByRole("dialog").parentElement!);
  expect(onClose).toHaveBeenCalledOnce();
});

it("n'a aucune violation axe une fois ouvert", async () => {
  render(<Harness role="alertdialog" description="Elle sera convertie." />);
  open();
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
});
