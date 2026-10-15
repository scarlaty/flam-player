import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { scanFolder, type FolderScan } from "./folder";

afterEach(() => clearMocks());

const scan: FolderScan = {
  entries: [
    {
      source_id: "s",
      uuid: "u",
      version: null,
      title: "Histoire",
      description: "",
      age: null,
      category: "",
      author: "",
      voice: "",
      license: "",
      thumb_url: null,
      download_url: "C:\\Histoires\\a.zip",
      size: 12,
      updated_at: "2026-10-15T08:00:00Z",
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
