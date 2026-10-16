import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { RadioGroup } from "./RadioGroup";

const options = [
  { value: "a", label: "Choix A", hint: "Précision" },
  { value: "b", label: "Choix B" },
] as const;

it("expose un groupe nommé avec le choix courant", () => {
  render(
    <RadioGroup legend="Mon choix" options={options} value="b" onChange={vi.fn()} />,
  );

  const group = screen.getByRole("group", { name: "Mon choix" });
  expect(group).toBeInTheDocument();
  expect(screen.getByRole("radio", { name: /Choix B/ })).toBeChecked();
  expect(screen.getByRole("radio", { name: /Choix A/ })).not.toBeChecked();
});

it("signale le choix sélectionné", () => {
  const onChange = vi.fn();
  render(
    <RadioGroup legend="Mon choix" options={options} value="b" onChange={onChange} />,
  );

  fireEvent.click(screen.getByRole("radio", { name: /Choix A/ }));

  expect(onChange).toHaveBeenCalledWith("a");
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <RadioGroup legend="Mon choix" options={options} value="a" onChange={vi.fn()} />,
  );
  await expectComponentAccessible(container);
});
