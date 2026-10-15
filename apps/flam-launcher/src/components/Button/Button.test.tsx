import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { IconButton } from "../IconButton/IconButton";
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

it("donne à IconButton un libellé accessible et une infobulle", () => {
  render(<IconButton icon="refresh" label="Actualiser" />);

  const button = screen.getByRole("button", { name: "Actualiser" });
  expect(button).toHaveAttribute("title", "Actualiser");
  expect(button.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <Button variant="danger" icon="trash">
        Supprimer
      </Button>
      <IconButton icon="sliders" variant="secondary" label="Paramètres" />
    </div>,
  );
  await expectComponentAccessible(container);
});
