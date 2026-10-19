import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { openExternal } from "./external";

afterEach(() => clearMocks());

const URL_OK = "https://github.com/scarlaty/flam-player/issues/new";

it("appelle la commande open_external avec l'adresse", async () => {
  const calls: Array<{ cmd: string; args: unknown }> = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return null;
  });
  await expect(openExternal(URL_OK)).resolves.toBeUndefined();
  expect(calls).toEqual([{ cmd: "open_external", args: { url: URL_OK } }]);
});

it("normalise l'erreur du backend (adresse refusée)", async () => {
  mockIPC(() => {
    throw { kind: "validation", message: "adresse refusée" };
  });
  await expect(openExternal("https://example.com/")).rejects.toEqual({
    kind: "validation",
    message: "adresse refusée",
  });
});
