import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { getCatalogEntries, getCatalogThumbnail, refreshCatalog } from "./catalog";
import { isAppError } from "./error";

afterEach(() => clearMocks());

it("refreshCatalog appelle catalog_refresh avec l'identifiant de la source", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return {
      source_id: "telmi-store",
      entries: [],
      warnings: [],
      status: { offline: false },
    };
  });
  const view = await refreshCatalog("telmi-store");
  expect(calls).toEqual([{ cmd: "catalog_refresh", args: { sourceId: "telmi-store" } }]);
  expect(view.source_id).toBe("telmi-store");
});

it("getCatalogEntries appelle catalog_entries", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return { source_id: "s", entries: [], warnings: [], status: {} };
  });
  await getCatalogEntries("s");
  expect(calls).toEqual([{ cmd: "catalog_entries", args: { sourceId: "s" } }]);
});

it("getCatalogThumbnail renvoie l'URL data:", async () => {
  mockIPC((cmd, args) => {
    expect(cmd).toBe("catalog_thumbnail");
    expect(args).toEqual({ sourceId: "s", thumbUrl: "https://h.example.test/t.png" });
    return "data:image/png;base64,AAAA";
  });
  await expect(getCatalogThumbnail("s", "https://h.example.test/t.png")).resolves.toBe(
    "data:image/png;base64,AAAA",
  );
});

it("une erreur réseau du backend reste une AppError", async () => {
  mockIPC(() => {
    throw { kind: "network", message: "catalogue inaccessible : délai dépassé" };
  });
  const error = await refreshCatalog("s").catch((e: unknown) => e);
  expect(isAppError(error)).toBe(true);
  expect(error).toMatchObject({ kind: "network" });
});
