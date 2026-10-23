import { beforeEach, expect, it } from "vitest";
import {
  forgetFinishedTasks,
  isCreating,
  rememberTask,
  rememberedTaskCount,
  resetCreations,
} from "./creations";

beforeEach(resetCreations);

it("purge les tâches terminées, garde les actives et celles pas encore vues", () => {
  rememberTask("t1", 1);
  rememberTask("t2", 1);
  rememberTask("t3", 2);
  let active = new Set(["t1", "t2"]);
  forgetFinishedTasks((id) => active.has(id));
  expect(rememberedTaskCount()).toBe(3);
  active = new Set(["t2"]);
  forgetFinishedTasks((id) => active.has(id));
  // t1 terminée : purgée ; t3 jamais vue active : conservée.
  expect(rememberedTaskCount()).toBe(2);
  expect(isCreating(1, (id) => active.has(id))).toBe(true);
  active = new Set();
  forgetFinishedTasks((id) => active.has(id));
  expect(rememberedTaskCount()).toBe(1);
  expect(isCreating(1, () => false)).toBe(false);
});
