import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { TaskPanel, type Task } from "./TaskPanel";

const tasks: Task[] = [
  { id: "a", title: "Histoire A", kind: "download", state: "running", progress: 40 },
  { id: "b", title: "Histoire B", kind: "convert", state: "queued" },
  { id: "c", title: "Histoire C", kind: "install", state: "done" },
  {
    id: "d",
    title: "Histoire D",
    kind: "convert",
    state: "error",
    detail: "Archive illisible",
  },
];

it("indique l'absence de tâche", () => {
  render(<TaskPanel tasks={[]} />);

  expect(screen.getByRole("region", { name: "Tâches en cours" })).toHaveTextContent(
    "Aucune tâche en cours.",
  );
});

it("affiche type, état, progression et détail de chaque tâche", () => {
  render(<TaskPanel tasks={tasks} />);

  expect(screen.getAllByRole("listitem")).toHaveLength(4);
  expect(
    screen.getByRole("progressbar", { name: "Progression : Histoire A" }),
  ).toHaveAttribute("aria-valuenow", "40");
  expect(screen.getAllByRole("progressbar")).toHaveLength(1);
  expect(screen.getByText("Téléchargement")).toBeInTheDocument();
  expect(screen.getByText("En attente")).toBeInTheDocument();
  expect(screen.getByText("Terminée")).toBeInTheDocument();
  expect(screen.getByText("Échec")).toBeInTheDocument();
  expect(screen.getByText("Archive illisible")).toBeInTheDocument();
});

it("propose Annuler pour les tâches actives et Retirer pour les autres", () => {
  const onCancel = vi.fn();
  const onDismiss = vi.fn();
  render(<TaskPanel tasks={tasks} onCancel={onCancel} onDismiss={onDismiss} />);

  fireEvent.click(screen.getByRole("button", { name: "Annuler : Histoire B" }));
  fireEvent.click(screen.getByRole("button", { name: "Retirer : Histoire D" }));
  expect(onCancel).toHaveBeenCalledWith("b");
  expect(onDismiss).toHaveBeenCalledWith("d");
  expect(screen.queryByRole("button", { name: "Retirer : Histoire A" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Annuler : Histoire C" })).toBeNull();
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <TaskPanel tasks={tasks} onCancel={() => {}} onDismiss={() => {}} />,
  );
  await expectComponentAccessible(container);
});
