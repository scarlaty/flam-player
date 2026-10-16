import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { pickExecutable, pickFolder } from "./dialog";

afterEach(() => clearMocks());

it("pickFolder ouvre le sélecteur de dossier et renvoie le chemin", async () => {
  let options: Record<string, unknown> = {};
  mockIPC((cmd, args) => {
    expect(cmd).toBe("plugin:dialog|open");
    options = (args as { options: Record<string, unknown> }).options;
    return "C:\\Histoires";
  });

  await expect(pickFolder("Choisir")).resolves.toBe("C:\\Histoires");
  expect(options).toMatchObject({ directory: true, multiple: false, title: "Choisir" });
});

it("pickExecutable filtre sur les .exe", async () => {
  let options: Record<string, unknown> = {};
  mockIPC((_cmd, args) => {
    options = (args as { options: Record<string, unknown> }).options;
    return "C:\\app\\flam-player.exe";
  });

  await expect(pickExecutable()).resolves.toBe("C:\\app\\flam-player.exe");
  expect(options).toMatchObject({
    directory: false,
    filters: [{ extensions: ["exe"] }],
  });
});

it("renvoie null quand l'utilisateur annule", async () => {
  mockIPC(() => null);
  await expect(pickFolder()).resolves.toBeNull();
});

it("normalise les erreurs", async () => {
  mockIPC(() => {
    throw "refusé";
  });
  await expect(pickFolder()).rejects.toEqual({ kind: "unknown", message: "refusé" });
});
