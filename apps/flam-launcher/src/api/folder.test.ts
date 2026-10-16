import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { folderThumbnail, scanFolder, type FolderScan } from "./folder";

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
