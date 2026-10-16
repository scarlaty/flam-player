import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { getSettings, setSettings, type Settings, type SettingsView } from "./settings";

afterEach(() => clearMocks());

const settings: Settings = {
  schema_version: 1,
  player_path: null,
  python_path: "C:\\Python\\python.exe",
  cache_dir: null,
  default_catalog_url: "https://exemple.org/c.json",
  theme: "dark",
};
const view: SettingsView = {
  settings,
  resolved: {
    player_path: "C:\\app\\flam-player.exe",
    python_path: "C:\\Python\\python.exe",
    cache_dir: "C:\\cache",
  },
  issues: [{ field: "player_path", kind: "not_found", message: "disparu" }],
  recovery: null,
};

it("getSettings appelle settings_get", async () => {
  mockIPC((cmd) => {
    expect(cmd).toBe("settings_get");
    return view;
  });
  await expect(getSettings()).resolves.toEqual(view);
});

it("setSettings envoie les paramètres à settings_set", async () => {
  let received: unknown;
  mockIPC((cmd, args) => {
    expect(cmd).toBe("settings_set");
    received = args;
    return view;
  });
  await expect(setSettings(settings)).resolves.toEqual(view);
  expect(received).toEqual({ settings });
});

it("setSettings propage une erreur de validation typée", async () => {
  mockIPC(() => {
    throw { kind: "validation", message: "URL invalide" };
  });
  await expect(setSettings(settings)).rejects.toEqual({
    kind: "validation",
    message: "URL invalide",
  });
});
