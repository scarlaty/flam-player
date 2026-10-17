import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { getDeviceStatus, listDeviceStories } from "./device";

afterEach(() => clearMocks());

it("appelle device_status sans argument", async () => {
  const calls: string[] = [];
  mockIPC((cmd, args) => {
    calls.push(`${cmd}:${JSON.stringify(args ?? {})}`);
    return { detected: false, hint: null, device: null, device_count: 0 };
  });
  expect((await getDeviceStatus()).detected).toBe(false);
  expect(calls).toEqual(["device_status:{}"]);
});

it("appelle device_list_stories et rend null sans appareil", async () => {
  mockIPC(() => null);
  expect(await listDeviceStories()).toBeNull();
});

it("normalise l'erreur du backend", async () => {
  mockIPC(() => {
    throw { kind: "io", message: "boum" };
  });
  await expect(getDeviceStatus()).rejects.toEqual({ kind: "io", message: "boum" });
});
