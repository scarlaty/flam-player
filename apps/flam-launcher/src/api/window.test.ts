import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { minimizeWindow, onWindowCloseRequested, restoreWindow } from "./window";

// Fenêtre factice : le gestionnaire de fermeture enregistré est capturé, `destroy` espionné.
const fake = vi.hoisted(() => ({
  handler: null as null | ((event: { preventDefault: () => void }) => Promise<void>),
  destroy: vi.fn(() => Promise.resolve()),
}));
vi.mock("@tauri-apps/api/window", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tauri-apps/api/window")>()),
  getCurrentWindow: () => ({
    onCloseRequested: (
      handler: (event: { preventDefault: () => void }) => Promise<void>,
    ) => {
      fake.handler = handler;
      return Promise.resolve(() => {
        fake.handler = null;
      });
    },
    destroy: fake.destroy,
    minimize: () =>
      import("@tauri-apps/api/core").then((m) => m.invoke("plugin:window|minimize")),
    unminimize: () =>
      import("@tauri-apps/api/core").then((m) => m.invoke("plugin:window|unminimize")),
    setFocus: () =>
      import("@tauri-apps/api/core").then((m) => m.invoke("plugin:window|set_focus")),
  }),
}));

let calls: string[];
let failing: string | null;

beforeEach(() => {
  fake.handler = null;
  fake.destroy.mockClear();
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

/** Fermeture demandée : rend vrai si le gestionnaire a empêché la fermeture par défaut. */
async function requestClose(): Promise<boolean> {
  let prevented = false;
  await fake.handler?.({ preventDefault: () => (prevented = true) });
  return prevented;
}

// #577 : un seul point de décision (le backend). Le gestionnaire empêche toujours la fermeture
// par défaut de Tauri (qui détruirait la fenêtre au milieu d'une écriture), travaille, puis suit
// le verdict.
it("ne détruit pas la fenêtre tant qu'une écriture est en cours (verdict retain)", async () => {
  const order: string[] = [];
  mockIPC((cmd) => {
    order.push(cmd);
    return cmd === "device_close_verdict" ? "retain" : null;
  });
  await onWindowCloseRequested(async () => {
    order.push("brouillon");
  });
  expect(await requestClose()).toBe(true);
  expect(order).toEqual(["brouillon", "device_close_verdict"]);
  expect(fake.destroy).not.toHaveBeenCalled();
});

it("vide le brouillon puis ferme quand aucune écriture n'est en cours (verdict allow)", async () => {
  const order: string[] = [];
  mockIPC((cmd) => {
    order.push(cmd);
    return cmd === "device_close_verdict" ? "allow" : null;
  });
  fake.destroy.mockImplementation(() => {
    order.push("destroy");
    return Promise.resolve();
  });
  await onWindowCloseRequested(async () => {
    order.push("brouillon");
  });
  expect(await requestClose()).toBe(true);
  expect(order).toEqual(["brouillon", "device_close_verdict", "destroy"]);
});

it("ferme comme avant si le verdict est indisponible ou si le brouillon échoue", async () => {
  mockIPC((cmd) => {
    if (cmd === "device_close_verdict") throw new Error("indisponible");
    return null;
  });
  await onWindowCloseRequested(() => Promise.reject(new Error("disque")));
  await requestClose();
  expect(fake.destroy).toHaveBeenCalledTimes(1);
});

it("la désinscription retire le gestionnaire", async () => {
  mockIPC(() => "allow");
  const off = await onWindowCloseRequested(() => Promise.resolve());
  expect(fake.handler).not.toBeNull();
  off();
  expect(fake.handler).toBeNull();
});
