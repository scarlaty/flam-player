import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { SelectField } from "./SelectField";

const options = [
  { value: "a", label: "Option A" },
  { value: "b", label: "Option B" },
];

it("relie le libellé à la liste et signale les changements", () => {
  const onChange = vi.fn();
  render(<SelectField label="Tri" options={options} value="a" onChange={onChange} />);
  const select = screen.getByLabelText("Tri");
  expect(screen.getAllByRole("option")).toHaveLength(2);
  fireEvent.change(select, { target: { value: "b" } });
  expect(onChange).toHaveBeenCalledTimes(1);
});

it("est accessible", async () => {
  const { container } = render(
    <SelectField label="Tri" options={options} defaultValue="b" />,
  );
  await expectComponentAccessible(container);
});
