import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { clearCache, getCacheUsage } from "./cache";

afterEach(() => clearMocks());

it("getCacheUsage appelle cache_usage", async () => {
  mockIPC((cmd) => {
    expect(cmd).toBe("cache_usage");
    return { path: "C:\\cache", bytes: 1234 };
  });
  await expect(getCacheUsage()).resolves.toEqual({ path: "C:\\cache", bytes: 1234 });
});

it("clearCache appelle cache_clear", async () => {
  mockIPC((cmd) => {
    expect(cmd).toBe("cache_clear");
    return { freed_bytes: 99 };
  });
  await expect(clearCache()).resolves.toEqual({ freed_bytes: 99 });
});

it("propage une erreur typée", async () => {
  mockIPC(() => {
    throw { kind: "io", message: "accès refusé" };
  });
  await expect(clearCache()).rejects.toEqual({ kind: "io", message: "accès refusé" });
});
