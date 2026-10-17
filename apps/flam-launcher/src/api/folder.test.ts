import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import {
  folderThumbnail,
  onFolderChanged,
  scanFolder,
  watchFolder,
  type FolderChanged,
  type FolderScan,
} from "./folder";

afterEach(() => clearMocks());

const scan: FolderScan = {
  entries: [
    {
      entry_id: "s:00ff00ff00ff00ff",
      source_id: "s",
      uuid: "u",
      version: null,
      title: "Histoire",
      description: null,
      age: null,
      category: null,
      author: null,
      voice: null,
      license: null,
      thumb_url: "s:00ff00ff00ff00ff",
      download_url: "C:\\Histoires\\a.zip",
      size: 12,
      updated_at: "2026-10-15T08:00:00Z",
      download_count: null,
      kind: "telmi_zip",
    },
  ],
  warnings: [{ path: "C:\\Histoires\\b.zip", message: "ignoré : archive zip invalide" }],
};

it("scanFolder appelle folder_scan, non récursif par défaut", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return scan;
  });
  await expect(scanFolder("s")).resolves.toEqual(scan);
  await scanFolder("s", true);
  expect(calls).toEqual([
    { cmd: "folder_scan", args: { sourceId: "s", recursive: false } },
    { cmd: "folder_scan", args: { sourceId: "s", recursive: true } },
  ]);
});

it("scanFolder normalise les erreurs du backend", async () => {
  mockIPC(() => {
    throw { kind: "folder_unavailable", message: "dossier introuvable" };
  });
  await expect(scanFolder("s")).rejects.toEqual({
    kind: "folder_unavailable",
    message: "dossier introuvable",
  });
});

it("folderThumbnail appelle folder_thumbnail", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return "data:image/png;base64,AAAA";
  });
  await expect(folderThumbnail("s", "e1")).resolves.toBe("data:image/png;base64,AAAA");
  expect(calls).toEqual([
    { cmd: "folder_thumbnail", args: { sourceId: "s", entryId: "e1" } },
  ]);
});

it("watchFolder appelle folder_watch", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return null;
  });
  await watchFolder("s", true);
  await watchFolder("s", false);
  expect(calls).toEqual([
    { cmd: "folder_watch", args: { sourceId: "s", enabled: true } },
    { cmd: "folder_watch", args: { sourceId: "s", enabled: false } },
  ]);
});

it("watchFolder exécute les appels dans l'ordre, sans chevauchement", async () => {
  const events: string[] = [];
  mockIPC(async (_cmd, args) => {
    const { enabled } = args as { enabled: boolean };
    events.push(`début ${enabled}`);
    // Le démarrage est plus lent que l'arrêt : sans file, l'arrêt le doublerait.
    await new Promise((resolve) => setTimeout(resolve, enabled ? 30 : 0));
    events.push(`fin ${enabled}`);
    return null;
  });
  await Promise.all([watchFolder("s", true), watchFolder("s", false)]);
  expect(events).toEqual(["début true", "fin true", "début false", "fin false"]);
});

it("watchFolder continue la file après une erreur", async () => {
  let n = 0;
  mockIPC(() => {
    if (n++ === 0) throw { kind: "folder_unavailable", message: "absent" };
    return null;
  });
  const first = watchFolder("s", true);
  const second = watchFolder("s", false);
  await expect(first).rejects.toMatchObject({ kind: "folder_unavailable" });
  await expect(second).resolves.toBeNull();
});

it("watchFolder normalise l'erreur d'un dossier disparu", async () => {
  mockIPC(() => {
    throw { kind: "folder_unavailable", message: "dossier introuvable" };
  });
  await expect(watchFolder("s", true)).rejects.toEqual({
    kind: "folder_unavailable",
    message: "dossier introuvable",
  });
});

it("onFolderChanged reçoit la charge de folder://changed", async () => {
  mockIPC(() => null, { shouldMockEvents: true });
  const seen: FolderChanged[] = [];
  const off = await onFolderChanged((e) => seen.push(e));
  await emit("folder://changed", { source_id: "s" });
  off();
  await emit("folder://changed", { source_id: "autre" });
  expect(seen).toEqual([{ source_id: "s" }]);
});
