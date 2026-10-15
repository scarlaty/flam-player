import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { IconButton } from "../IconButton/IconButton";
import { SidebarSourceList, type SidebarSource } from "./SidebarSourceList";

const sources: SidebarSource[] = [
  { id: "cat", label: "Catalogue A", kind: "catalog", count: 1 },
  { id: "lib", label: "Mes histoires", kind: "library", count: 12 },
  { id: "off", label: "Catalogue B", kind: "catalog", status: "offline", count: 4 },
  { id: "dir", label: "Clé USB", kind: "folder", status: "error" },
];

const sourceButton = (name: string) =>
  screen.getByRole("button", { name: new RegExp(`^${name},`) });

it("respecte l'ordre fourni par l'appelant", () => {
  render(<SidebarSourceList sources={sources} onSelect={() => {}} />);

  const labels = screen
    .getAllByRole("listitem")
    .map((item) => item.querySelector(".fl-source__label")?.textContent);
  expect(labels).toEqual(["Catalogue A", "Mes histoires", "Catalogue B", "Clé USB"]);
  expect(screen.getByRole("heading", { name: "Sources" })).toBeInTheDocument();
});

it("annonce type, nombre d'histoires et état de chaque source", () => {
  render(<SidebarSourceList sources={sources} onSelect={() => {}} />);

  expect(sourceButton("Mes histoires")).toHaveAccessibleName(
    "Mes histoires, Bibliothèque, 12 histoires",
  );
  expect(sourceButton("Catalogue A")).toHaveAccessibleName(
    "Catalogue A, Catalogue, 1 histoire",
  );
  expect(sourceButton("Catalogue B")).toHaveAccessibleName(
    "Catalogue B, Catalogue, Hors ligne",
  );
  expect(sourceButton("Clé USB")).toHaveAccessibleName("Clé USB, Dossier local, Erreur");
  expect(screen.getByTitle("Hors ligne")).toBeInTheDocument();
  expect(screen.getByTitle("Erreur")).toBeInTheDocument();
});

it("signale la source sélectionnée et notifie la sélection", () => {
  const onSelect = vi.fn();
  render(<SidebarSourceList sources={sources} selectedId="lib" onSelect={onSelect} />);

  expect(sourceButton("Mes histoires")).toHaveAttribute("aria-current", "page");
  fireEvent.click(sourceButton("Catalogue A"));
  expect(onSelect).toHaveBeenCalledWith("cat");
});

it("déplace le focus entre les sources avec les flèches, Début et Fin", () => {
  render(
    <SidebarSourceList
      sources={sources}
      onSelect={() => {}}
      renderActions={(source) => (
        <IconButton icon="trash" label={`Supprimer ${source.id}`} />
      )}
    />,
  );
  const [first, second, , last] = [
    "Catalogue A",
    "Mes histoires",
    "Catalogue B",
    "Clé USB",
  ].map(sourceButton);

  first!.focus();
  fireEvent.keyDown(first!, { key: "ArrowDown" });
  expect(second).toHaveFocus();
  fireEvent.keyDown(second!, { key: "End" });
  expect(last).toHaveFocus();
  fireEvent.keyDown(last!, { key: "ArrowDown" });
  expect(last).toHaveFocus();
  fireEvent.keyDown(last!, { key: "Home" });
  expect(first).toHaveFocus();
  fireEvent.keyDown(first!, { key: "ArrowUp" });
  expect(first).toHaveFocus();
});

it("affiche les actions propres à chaque source", () => {
  const onDelete = vi.fn();
  render(
    <SidebarSourceList
      sources={sources}
      onSelect={() => {}}
      renderActions={(source) =>
        source.kind === "library" ? null : (
          <IconButton
            icon="trash"
            label={`Supprimer ${source.label}`}
            onClick={() => onDelete(source.id)}
          />
        )
      }
    />,
  );

  expect(screen.getAllByRole("button", { name: /^Supprimer/ })).toHaveLength(3);
  fireEvent.click(screen.getByRole("button", { name: "Supprimer Clé USB" }));
  expect(onDelete).toHaveBeenCalledWith("dir");
});

it("montre le libellé complet en infobulle seulement s'il est tronqué", () => {
  render(<SidebarSourceList sources={sources} onSelect={() => {}} />);
  const button = sourceButton("Catalogue A");
  const label = button.querySelector(".fl-source__label")!;

  Object.defineProperty(label, "clientWidth", { configurable: true, value: 100 });
  Object.defineProperty(label, "scrollWidth", { configurable: true, value: 100 });
  fireEvent.mouseEnter(button);
  expect(button).not.toHaveAttribute("title");

  Object.defineProperty(label, "scrollWidth", { configurable: true, value: 180 });
  fireEvent.focus(button);
  expect(button).toHaveAttribute("title", "Catalogue A");
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
      renderActions={(source) => (
        <IconButton icon="trash" label={`Supprimer ${source.id}`} />
      )}
    />,
  );
  await expectComponentAccessible(container);
});
