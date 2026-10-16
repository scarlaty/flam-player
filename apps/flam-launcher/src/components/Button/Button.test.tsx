import { fireEvent, render, screen } from "@testing-library/react";
import { createRef } from "react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { Button } from "./Button";

it("rend un bouton de type button avec sa variante", () => {
  const onClick = vi.fn();
  render(
    <Button variant="primary" icon="play" onClick={onClick}>
      Jouer
    </Button>,
  );

  const button = screen.getByRole("button", { name: "Jouer" });
  expect(button).toHaveAttribute("type", "button");
  expect(button).toHaveClass("fl-button--primary");
  fireEvent.click(button);
  expect(onClick).toHaveBeenCalledOnce();
});

it("n'appelle pas onClick quand il est désactivé", () => {
  const onClick = vi.fn();
  render(
    <Button disabled onClick={onClick}>
      Installer
    </Button>,
  );

  fireEvent.click(screen.getByRole("button", { name: "Installer" }));
  expect(onClick).not.toHaveBeenCalled();
});

it("transmet la ref au bouton natif", () => {
  const ref = createRef<HTMLButtonElement>();
  render(<Button ref={ref}>Annuler</Button>);
  expect(ref.current).toBe(screen.getByRole("button", { name: "Annuler" }));
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <Button variant="danger" icon="trash">
        Supprimer
      </Button>
      <Button variant="ghost">Annuler</Button>
    </div>,
  );
  await expectComponentAccessible(container);
});

it("applique la variante accent (jaune) au bouton Jouer", async () => {
  const { container } = render(
    <Button variant="accent" icon="play">
      Jouer
    </Button>,
  );

  expect(screen.getByRole("button", { name: "Jouer" })).toHaveClass("fl-button--accent");
  await expectComponentAccessible(container);
});
