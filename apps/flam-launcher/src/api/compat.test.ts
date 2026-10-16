import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { findCompat, getCompatReport, type CompatEntry } from "./compat";

afterEach(() => clearMocks());

function fiche(over: Partial<CompatEntry>): CompatEntry {
  return {
    uuid: "a",
    version: "1",
    level: "compatible",
    reason: null,
    code: "OK",
    origin: "campaign",
    stale: false,
    ...over,
  };
}

it("getCompatReport appelle compat_statuses sans argument", async () => {
  mockIPC((cmd, args) => {
    expect(cmd).toBe("compat_statuses");
    expect(args ?? {}).toEqual({});
    return { entries: [], issue: null };
  });
  await expect(getCompatReport()).resolves.toEqual({ entries: [], issue: null });
});

it("findCompat associe par uuid puis version", () => {
  const v1 = fiche({ version: "1" });
  const v2 = fiche({ version: "2", level: "warnings" });
  const local = fiche({ uuid: "b", version: "local" });
  const inconnue = fiche({ uuid: "c", version: null });
  const all = [v1, v2, local, inconnue];
  expect(findCompat(all, { uuid: "a", version: 2 })).toBe(v2);
  expect(findCompat(all, { uuid: "a", version: 1 })).toBe(v1);
  expect(findCompat(all, { uuid: "a", version: 3 })).toBeUndefined();
  expect(findCompat(all, { uuid: "a", version: null })).toBe(v1);
  expect(findCompat(all, { uuid: "b", version: 7 })).toBe(local);
  expect(findCompat(all, { uuid: "c", version: 4 })).toBe(inconnue);
  expect(findCompat(all, { uuid: "zz", version: null })).toBeUndefined();
});
