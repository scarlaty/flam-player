import { expect, it } from "vitest";
import type { InstallTask } from "../../api/install";
import { makeEntry } from "../../test/entries";
import { EarlyEvents } from "./earlyEvents";

const base: InstallTask = {
  id: "t1",
  entry: makeEntry({ title: "Les trois ours" }),
  state: "queued",
  percent: null,
  error: null,
  created_at: 1,
};

it("applique les événements mémorisés puis les oublie", () => {
  const early = new EarlyEvents();
  early.record("t1", { state: "downloading", percent: 40 });
  early.record("t1", { percent: 60 });
  expect(early.size).toBe(1);

  expect(early.take(base)).toMatchObject({ state: "downloading", percent: 60 });
  expect(early.size).toBe(0);
  // Appliqués une seule fois : la tâche est ensuite laissée telle quelle.
  expect(early.take(base)).toBe(base);
});

it("laisse intacte une tâche sans événement et garde les autres", () => {
  const early = new EarlyEvents();
  early.record("autre", { state: "failed" });
  expect(early.take(base)).toBe(base);
  expect(early.size).toBe(1);
});

it("ignore un retour à un état actif après une fin", () => {
  const early = new EarlyEvents();
  early.record("t1", { state: "done", percent: 100 });
  early.record("t1", { state: "downloading", percent: 10 });
  expect(early.take(base)).toMatchObject({ state: "done", percent: 100 });
});
