import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { IconButton } from "./IconButton";

it("tire son nom accessible et son infobulle du libellé", () => {
  const onClick = vi.fn();
  render(<IconButton icon="refresh" label="Actualiser" onClick={onClick} />);

  const button = screen.getByRole("button", { name: "Actualiser" });
  expect(button).toHaveAttribute("type", "button");
  expect(button).toHaveAttribute("title", "Actualiser");
  expect(button.querySelector("svg")).toHaveAttribute("aria-hidden", "true");
  fireEvent.click(button);
  expect(onClick).toHaveBeenCalledOnce();
});

it("garde le libellé comme nom même avec une infobulle différente", () => {
  render(<IconButton icon="trash" label="Supprimer la source" title="Supprimer" />);
  expect(screen.getByRole("button", { name: "Supprimer la source" })).toHaveAttribute(
    "title",
    "Supprimer",
  );
});

it("exige un libellé à la compilation", () => {
  // @ts-expect-error `label` est obligatoire : un bouton icône sans nom est refusé.
  const element = <IconButton icon="close" />;
  expect(element).toBeTruthy();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <IconButton icon="sliders" variant="secondary" label="Paramètres" />
      <IconButton icon="play" variant="primary" size="sm" label="Jouer" />
    </div>,
  );
  await expectComponentAccessible(container);
});
