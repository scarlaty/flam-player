import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { SidebarSourceList, type SidebarSource } from "./SidebarSourceList";

const sources: SidebarSource[] = [
  { id: "cat", label: "Catalogue A", kind: "catalog", count: 1 },
  { id: "lib", label: "Mes histoires", kind: "library", count: 12 },
  { id: "dir", label: "Clé USB", kind: "folder", unavailable: true },
];

it("regroupe les sources par type, bibliothèque d'abord", () => {
  render(<SidebarSourceList sources={sources} onSelect={() => {}} />);

  const headings = screen.getAllByRole("heading").map((h) => h.textContent);
  expect(headings).toEqual(["Bibliothèque", "Catalogue", "Dossier local"]);
  expect(screen.getByRole("button", { name: /Mes histoires/ })).toHaveAccessibleName(
    "Mes histoires, 12 histoires",
  );
  expect(screen.getByRole("button", { name: /Catalogue A/ })).toHaveAccessibleName(
    "Catalogue A, 1 histoire",
  );
  expect(screen.getByRole("button", { name: /Clé USB/ })).toHaveAccessibleName(
    "Clé USB, Indisponible",
  );
});

it("signale la source sélectionnée et notifie la sélection", () => {
  const onSelect = vi.fn();
  render(<SidebarSourceList sources={sources} selectedId="lib" onSelect={onSelect} />);

  expect(screen.getByRole("button", { name: /Mes histoires/ })).toHaveAttribute(
    "aria-current",
    "page",
  );
  fireEvent.click(screen.getByRole("button", { name: /Catalogue A/ }));
  expect(onSelect).toHaveBeenCalledWith("cat");
});

it("déplace le focus avec les flèches, Début et Fin", () => {
  render(<SidebarSourceList sources={sources} onSelect={() => {}} />);
  const [library, catalog, folder] = ["Mes histoires", "Catalogue A", "Clé USB"].map(
    (name) => screen.getByRole("button", { name: new RegExp(name) }),
  );

  library?.focus();
  fireEvent.keyDown(library!, { key: "ArrowDown" });
  expect(catalog).toHaveFocus();
  fireEvent.keyDown(catalog!, { key: "End" });
  expect(folder).toHaveFocus();
  fireEvent.keyDown(folder!, { key: "ArrowDown" });
  expect(folder).toHaveFocus();
  fireEvent.keyDown(folder!, { key: "Home" });
  expect(library).toHaveFocus();
  fireEvent.keyDown(library!, { key: "ArrowUp" });
  expect(library).toHaveFocus();
});

it("propose l'ajout d'une source si demandé", () => {
  const onAdd = vi.fn();
  render(<SidebarSourceList sources={[]} onSelect={() => {}} onAdd={onAdd} />);

  fireEvent.click(screen.getByRole("button", { name: "Ajouter une source" }));
  expect(onAdd).toHaveBeenCalledOnce();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <SidebarSourceList
      sources={sources}
      selectedId="lib"
      onSelect={() => {}}
      onAdd={() => {}}
    />,
  );
  await expectComponentAccessible(container);
});
