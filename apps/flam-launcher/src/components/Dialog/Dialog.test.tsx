import { fireEvent, render, screen, within } from "@testing-library/react";
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

it("`wide` élargit le panneau (fiche détaillée)", () => {
  render(<Dialog wide open onClose={() => {}} title="Fiche" />);
  expect(screen.getByRole("dialog")).toHaveClass("fl-dialog", "fl-dialog--wide");
});

it("Maj+Tab depuis le panneau lui-même va au dernier élément", () => {
  render(<Harness />);
  open();
  const panel = screen.getByRole("dialog");
  panel.focus();
  fireEvent.keyDown(panel, { key: "Tab", shiftKey: true });
  expect(screen.getByRole("button", { name: "Installer" })).toHaveFocus();
});

it("Tab ramène dans le dialogue quand le focus est tombé sur body", () => {
  render(<Harness />);
  open();
  (document.activeElement as HTMLElement).blur();
  expect(document.body).toHaveFocus();
  fireEvent.keyDown(document.body, { key: "Tab" });
  expect(screen.getByRole("button", { name: "Fermer" })).toHaveFocus();

  (document.activeElement as HTMLElement).blur();
  fireEvent.keyDown(document.body, { key: "Tab", shiftKey: true });
  expect(screen.getByRole("button", { name: "Installer" })).toHaveFocus();
});

it("Tab hors du dialogue y revient, et Tab au milieu reste au navigateur", () => {
  render(<Harness />);
  open();
  const outside = screen.getByRole("button", { name: "Dehors", hidden: true });
  outside.focus();
  fireEvent.keyDown(outside, { key: "Tab" });
  expect(screen.getByRole("button", { name: "Fermer" })).toHaveFocus();

  const cancel = screen.getByRole("button", { name: "Annuler" });
  cancel.focus();
  const notPrevented = fireEvent.keyDown(cancel, { key: "Tab" });
  expect(notPrevented).toBe(true);
});

function Hidden() {
  return (
    <Dialog
      open
      onClose={() => {}}
      title="Réglages"
      footer={
        <>
          <button type="button">Premier</button>
          <button type="button">Dernier</button>
          <button type="button" hidden>
            Caché
          </button>
          <div hidden>
            <button type="button">Dans un parent caché</button>
          </div>
          <button type="button" style={{ visibility: "hidden" }}>
            Invisible
          </button>
        </>
      }
    />
  );
}

it("ignore les éléments masqués dans la boucle de Tab (#581)", () => {
  render(<Hidden />);
  const last = screen.getByRole("button", { name: "Dernier" });
  last.focus();
  fireEvent.keyDown(last, { key: "Tab" });
  expect(screen.getByRole("button", { name: "Fermer" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: "Tab", shiftKey: true });
  expect(last).toHaveFocus();
});

it("ignore un dernier élément masqué par `display: none` (#581)", () => {
  render(
    <Dialog
      open
      onClose={() => {}}
      title="Réglages"
      footer={
        <>
          <button type="button">Visible</button>
          <button type="button" style={{ display: "none" }}>
            Absent
          </button>
        </>
      }
    />,
  );
  const visible = screen.getByRole("button", { name: "Visible" });
  visible.focus();
  fireEvent.keyDown(visible, { key: "Tab" });
  expect(screen.getByRole("button", { name: "Fermer" })).toHaveFocus();
});

it("laisse Ctrl+Tab, Alt+Tab et Méta+Tab au navigateur (#581)", () => {
  render(<Harness />);
  open();
  const install = screen.getByRole("button", { name: "Installer" });
  install.focus();
  for (const modifier of ["ctrlKey", "altKey", "metaKey"] as const) {
    const notPrevented = fireEvent.keyDown(install, { key: "Tab", [modifier]: true });
    expect(notPrevented).toBe(true);
    expect(install).toHaveFocus();
  }
});

it("ne ré-enregistre pas les écouteurs du document quand onClose change à chaque rendu (#581)", () => {
  const add = vi.spyOn(document, "addEventListener");
  const calls: string[] = [];
  const { rerender } = render(
    <Dialog open onClose={() => calls.push("a")} title="Fiche" />,
  );
  const registered = add.mock.calls.filter(([type]) => type === "keydown").length;
  rerender(<Dialog open onClose={() => calls.push("b")} title="Fiche" />);
  expect(add.mock.calls.filter(([type]) => type === "keydown")).toHaveLength(registered);
  // C'est bien le dernier `onClose` qui répond.
  fireEvent.keyDown(document, { key: "Escape" });
  expect(calls).toEqual(["b"]);
  add.mockRestore();
});

it("avec deux dialogues empilés, seul le dernier piège le focus (#581)", () => {
  render(
    <>
      <Dialog
        open
        onClose={() => {}}
        title="Dessous"
        footer={<button type="button">Bas</button>}
      />
      <Dialog
        open
        onClose={() => {}}
        title="Dessus"
        footer={<button type="button">Haut</button>}
      />
    </>,
  );
  const top = screen.getByRole("dialog", { name: "Dessus" });
  const haut = screen.getByRole("button", { name: "Haut" });
  // Le dialogue du dessous ne doit pas toucher au focus.
  const lowerClose = within(
    screen.getByRole("dialog", { name: "Dessous", hidden: true }),
  ).getByRole("button", { name: "Fermer", hidden: true });
  const lowerFocus = vi.spyOn(lowerClose, "focus");
  haut.focus();
  fireEvent.keyDown(haut, { key: "Tab" });
  expect(lowerFocus).not.toHaveBeenCalled();
  // Un seul piège agit : le focus va au premier élément du dialogue du dessus.
  expect(top).toContainElement(document.activeElement as HTMLElement);
  expect(document.activeElement).toBe(
    within(top).getByRole("button", { name: "Fermer" }),
  );
});

it("ignore un élément `inert` ou dans un parent `inert` dans la boucle de Tab (#581)", () => {
  render(
    <Dialog
      open
      onClose={() => {}}
      title="Réglages"
      footer={
        <>
          <button type="button">Utile</button>
          <button type="button" inert>
            Inerte
          </button>
          <div inert>
            <button type="button">Dans un parent inerte</button>
          </div>
        </>
      }
    />,
  );
  const useful = screen.getByRole("button", { name: "Utile" });
  useful.focus();
  fireEvent.keyDown(useful, { key: "Tab" });
  expect(screen.getByRole("button", { name: "Fermer" })).toHaveFocus();
  fireEvent.keyDown(document.activeElement!, { key: "Tab", shiftKey: true });
  expect(useful).toHaveFocus();
});

it("le focus initial saute un élément masqué (#581)", () => {
  render(
    <Dialog
      open
      onClose={() => {}}
      title="Réglages"
      footer={
        <>
          <button type="button" hidden>
            Caché
          </button>
          <button type="button" style={{ display: "none" }}>
            Absent
          </button>
          <button type="button">Visible</button>
        </>
      }
    >
      <button type="button" style={{ visibility: "hidden" }}>
        Invisible
      </button>
    </Dialog>,
  );
  expect(screen.getByRole("button", { name: "Visible" })).toHaveFocus();
});

it("avec deux dialogues empilés, Échap ne ferme que celui du dessus (#652)", () => {
  function Stack() {
    const [lower, setLower] = useState(true);
    const [upper, setUpper] = useState(true);
    return (
      <>
        <Dialog open={lower} onClose={() => setLower(false)} title="Dessous" />
        <Dialog open={upper} onClose={() => setUpper(false)} title="Dessus" />
      </>
    );
  }
  render(<Stack />);
  expect(screen.getByRole("dialog", { name: "Dessus" })).toBeInTheDocument();

  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog", { name: "Dessus" })).not.toBeInTheDocument();
  expect(screen.getByRole("dialog", { name: "Dessous" })).toBeInTheDocument();

  // Un second Échap ferme alors le dialogue resté dessous.
  fireEvent.keyDown(document, { key: "Escape" });
  expect(screen.queryByRole("dialog", { name: "Dessous" })).not.toBeInTheDocument();
});
