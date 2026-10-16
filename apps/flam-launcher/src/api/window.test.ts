import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, it } from "vitest";
import { minimizeWindow, restoreWindow } from "./window";

let calls: string[];
let failing: string | null;

beforeEach(() => {
  calls = [];
  failing = null;
  mockWindows("main");
  mockIPC((cmd) => {
    calls.push(cmd);
    if (failing === cmd) throw new Error("refusé");
    return null;
  });
});

afterEach(() => clearMocks());

it("réduit la fenêtre", async () => {
  expect(await minimizeWindow()).toBe(true);
  expect(calls).toEqual(["plugin:window|minimize"]);
});

it("signale un échec de réduction sans lever d'erreur", async () => {
  failing = "plugin:window|minimize";
  expect(await minimizeWindow()).toBe(false);
});

it("rétablit la fenêtre et lui rend le focus", async () => {
  await restoreWindow();
  expect(calls).toEqual(["plugin:window|unminimize", "plugin:window|set_focus"]);
});

it("ne lève pas d'erreur si le rétablissement échoue", async () => {
  failing = "plugin:window|unminimize";
  await expect(restoreWindow()).resolves.toBeUndefined();
});
