import { beforeEach, expect, it } from "vitest";
import {
  forgetFinishedTasks,
  isCreating,
  rememberTask,
  rememberedTaskCount,
  resetCreations,
} from "./creations";

beforeEach(resetCreations);

it("purge les tâches terminées présentes, garde les actives et les absentes", () => {
  rememberTask("t1", 1);
  rememberTask("t2", 1);
  rememberTask("t3", 2);
  const finished = new Set(["t1"]);
  forgetFinishedTasks((id) => finished.has(id));
  // t1 terminée : purgée ; t2 active et t3 absente du panneau : conservées.
  expect(rememberedTaskCount()).toBe(2);
  expect(isCreating(1, (id) => id === "t2")).toBe(true);
  finished.add("t2");
  forgetFinishedTasks((id) => finished.has(id));
  expect(rememberedTaskCount()).toBe(1);
  expect(isCreating(1, () => false)).toBe(false);
});
