import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { TextField } from "./TextField";

it("relie le libellé, l'aide et l'erreur au champ", () => {
  render(<TextField label="Nom" hint="Obligatoire" error="Le nom est vide" />);

  const input = screen.getByRole("textbox", { name: "Nom" });
  expect(input).toHaveAttribute("aria-invalid", "true");
  expect(input).toHaveAccessibleDescription("Obligatoire Le nom est vide");
});

it("n'est pas invalide sans erreur", () => {
  render(<TextField label="Nom" />);
  const input = screen.getByRole("textbox", { name: "Nom" });
  expect(input).not.toHaveAttribute("aria-invalid");
  expect(input).not.toHaveAttribute("aria-describedby");
});

it("transmet la saisie et affiche les actions", () => {
  const onChange = vi.fn();
  render(
    <TextField label="Nom" onChange={onChange} actions={<button>Parcourir</button>} />,
  );

  fireEvent.change(screen.getByRole("textbox", { name: "Nom" }), {
    target: { value: "ab" },
  });

  expect(onChange).toHaveBeenCalledTimes(1);
  expect(screen.getByRole("button", { name: "Parcourir" })).toBeInTheDocument();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <div>
      <TextField label="Nom" hint="Aide" />
      <TextField label="URL" error="Invalide" defaultValue="x" />
    </div>,
  );
  await expectComponentAccessible(container);
});
