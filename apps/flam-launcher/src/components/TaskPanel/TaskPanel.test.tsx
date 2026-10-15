import { fireEvent, render, screen } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { expectComponentAccessible } from "../../test/axe";
import { describeChanges, TaskPanel, type Task } from "./TaskPanel";

const tasks: Task[] = [
  { id: "a", title: "Histoire A", kind: "download", state: "running", progress: 40 },
  { id: "b", title: "Histoire B", kind: "convert", state: "queued" },
  { id: "c", title: "Histoire C", kind: "install", state: "done" },
  {
    id: "d",
    title: "Histoire D",
    kind: "convert",
    state: "error",
    phase: "Lecture de l'archive",
    detail: "Archive illisible",
  },
  { id: "e", title: "Histoire E", kind: "download", state: "cancelled" },
];

it("indique l'absence de tâche", () => {
  render(<TaskPanel tasks={[]} />);

  expect(screen.getByRole("region", { name: "Tâches en cours" })).toHaveTextContent(
    "Aucune tâche en cours.",
  );
});

it("affiche type, phase, état, progression et détail de chaque tâche", () => {
  render(<TaskPanel tasks={tasks} />);

  expect(screen.getAllByRole("listitem")).toHaveLength(5);
  expect(
    screen.getByRole("progressbar", { name: "Progression : Histoire A" }),
  ).toHaveAttribute("aria-valuenow", "40");
  expect(screen.getAllByRole("progressbar")).toHaveLength(1);
  expect(screen.getByText("Conversion · Lecture de l'archive")).toBeInTheDocument();
  for (const state of ["En attente", "Terminée", "Échec", "Annulée"]) {
    expect(screen.getByText(state)).toBeInTheDocument();
  }
  expect(screen.getByText("Archive illisible")).toBeInTheDocument();
});

it("distingue l'icône d'installation de l'état terminé", () => {
  const { container } = render(<TaskPanel tasks={tasks} />);
  const installIcon = container.querySelector(".fl-task--done .fl-task__icon svg");
  const doneBadge = container.querySelector(".fl-task--done .fl-badge");

  expect(installIcon?.innerHTML).not.toContain("m5 12.5 4.5 4.5L19 7.5");
  expect(doneBadge).toHaveTextContent("Terminée");
});

it("propose Annuler, Réessayer et Retirer selon l'état", () => {
  const onCancel = vi.fn();
  const onRetry = vi.fn();
  const onDismiss = vi.fn();
  render(
    <TaskPanel
      tasks={tasks}
      onCancel={onCancel}
      onRetry={onRetry}
      onDismiss={onDismiss}
    />,
  );

  fireEvent.click(screen.getByRole("button", { name: "Annuler : Histoire B" }));
  fireEvent.click(screen.getByRole("button", { name: "Réessayer : Histoire D" }));
  fireEvent.click(screen.getByRole("button", { name: "Réessayer : Histoire E" }));
  fireEvent.click(screen.getByRole("button", { name: "Retirer : Histoire C" }));
  expect(onCancel).toHaveBeenCalledWith("b");
  expect(onRetry.mock.calls).toEqual([["d"], ["e"]]);
  expect(onDismiss).toHaveBeenCalledWith("c");
  expect(screen.queryByRole("button", { name: "Réessayer : Histoire C" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Retirer : Histoire A" })).toBeNull();
  expect(screen.queryByRole("button", { name: "Annuler : Histoire C" })).toBeNull();
});

it("annonce poliment les changements d'état et les échecs, pas la progression", () => {
  const first: Task[] = [
    { id: "a", title: "Mila", kind: "download", state: "running", progress: 10 },
  ];
  const { rerender } = render(<TaskPanel tasks={first} />);
  const status = screen.getByRole("status");
  expect(status).toHaveTextContent("");

  rerender(<TaskPanel tasks={[{ ...first[0]!, progress: 50 }]} />);
  expect(status).toHaveTextContent("");

  rerender(
    <TaskPanel tasks={[{ ...first[0]!, state: "error", detail: "Réseau coupé" }]} />,
  );
  expect(status).toHaveTextContent("Téléchargement « Mila » : Échec. Réseau coupé");
});

it("décrit les tâches nouvelles ou modifiées", () => {
  const before: Task[] = [{ id: "a", title: "A", kind: "install", state: "running" }];
  const after: Task[] = [
    { id: "a", title: "A", kind: "install", state: "done" },
    { id: "b", title: "B", kind: "convert", state: "queued" },
  ];
  expect(describeChanges(before, after)).toBe(
    "Installation « A » : Terminée Conversion « B » : En attente",
  );
  expect(describeChanges(after, after)).toBe("");
});

it("n'a aucune violation axe", async () => {
  const { container } = render(
    <TaskPanel
      tasks={tasks}
      onCancel={() => {}}
      onRetry={() => {}}
      onDismiss={() => {}}
    />,
  );
  await expectComponentAccessible(container);
});
