import { expect, it } from "vitest";
import type { InstallTask } from "../../api/install";
import { makeEntry } from "../../test/entries";
import { describePhase, isActiveState, taskProgress, toPanelTask } from "./taskView";

function task(over: Partial<InstallTask>): InstallTask {
  return {
    id: "t",
    entry: makeEntry({ title: "Conte" }),
    state: "queued",
    percent: null,
    error: null,
    created_at: 0,
    pack: null,
    ...over,
  };
}

it("décrit l'étape en cours", () => {
  expect(describePhase("queued")).toBe("En attente");
  expect(describePhase("downloading")).toBe("Téléchargement");
  expect(describePhase("downloading", { received: 1024, total: 2048, phase: null })).toBe(
    "Téléchargement : 1 Ko sur 2 Ko",
  );
  expect(describePhase("converting", { received: null, total: null, phase: "zip" })).toBe(
    "Conversion : archive",
  );
  expect(describePhase("converting", { received: null, total: null, phase: "x" })).toBe(
    "Conversion : x",
  );
  expect(describePhase("installing")).toBe("Installation");
  expect(describePhase("done")).toBeUndefined();
});

it("distingue les tâches actives des tâches terminées", () => {
  expect(
    ["queued", "downloading", "converting", "installing"].every((s) =>
      isActiveState(s as never),
    ),
  ).toBe(true);
  expect(["done", "failed", "cancelled"].some((s) => isActiveState(s as never))).toBe(
    false,
  );
});

it("met une tâche au format du panneau", () => {
  expect(toPanelTask(task({ state: "downloading", percent: 12 }))).toMatchObject({
    title: "Conte",
    kind: "download",
    state: "running",
    progress: 12,
    phase: "Téléchargement",
  });
  expect(toPanelTask(task({ state: "converting" }))).toMatchObject({
    kind: "convert",
    state: "running",
    progress: undefined,
  });
  const failed = toPanelTask(
    task({
      state: "failed",
      error: { kind: "conversion", message: "Image manquante.", code: "missing_asset" },
    }),
  );
  expect(failed).toMatchObject({ state: "error", detail: "Image manquante." });
  expect(failed.phase).toBeUndefined();
  expect(toPanelTask(task({ state: "cancelled" })).state).toBe("cancelled");
});

it("donne l'avancement affiché sur la carte", () => {
  expect(taskProgress(task({ state: "converting", percent: 30 }))).toEqual({
    value: 30,
    text: "Conversion",
  });
  expect(taskProgress(task({ state: "queued" })).value).toBe("indeterminate");
});

it("décrit la préparation d'un pack et signale les épisodes retirés", () => {
  expect(describePhase("preparing")).toBe("Préparation du pack");
  expect(
    describePhase("preparing", { received: null, total: null, phase: "audio" }),
  ).toBe("Préparation du pack : conversion audio");
  expect(describePhase("prepared")).toBe("Pack préparé, en attente de conversion");
  const panel = toPanelTask(
    task({ state: "done", pack: { episodes: 3, dropped: ["Deux"] } }),
  );
  expect(panel.detail).toBe("1 épisode non inclus : Deux");
  expect(toPanelTask(task({ state: "preparing", percent: 40 })).state).toBe("running");
});
