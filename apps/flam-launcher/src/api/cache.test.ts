import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { clearCache, getCacheUsage, getMp3CacheUsage } from "./cache";

afterEach(() => clearMocks());

it("getCacheUsage appelle cache_usage", async () => {
  mockIPC((cmd) => {
    expect(cmd).toBe("cache_usage");
    return { path: "C:\\cache", bytes: 1234 };
  });
  await expect(getCacheUsage()).resolves.toEqual({ path: "C:\\cache", bytes: 1234 });
});

it("getMp3CacheUsage appelle podcast_mp3_cache_size", async () => {
  mockIPC((cmd) => {
    expect(cmd).toBe("podcast_mp3_cache_size");
    return { bytes: 4096 };
  });
  await expect(getMp3CacheUsage()).resolves.toEqual({ bytes: 4096 });
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
