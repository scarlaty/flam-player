import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import {
  cancelClose,
  cancelDeviceStoryDetails,
  getCloseVerdict,
  getWriteInProgress,
  getDeviceStatus,
  getDeviceStoryThumbnail,
  getDeviceStoryTitles,
  listDeviceStories,
} from "./device";

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

it("transmet la racine et les UUID aux commandes de titres et de vignettes", async () => {
  const calls: string[] = [];
  mockIPC((cmd, args) => {
    calls.push(`${cmd}:${JSON.stringify(args ?? {})}`);
    if (cmd === "device_story_titles")
      return { titles: [], interrupted: false, keys: false };
    if (cmd === "device_story_thumbnail") return { thumb: null, busy: false };
    return null;
  });
  await getDeviceStoryTitles("E:\\", ["a"]);
  await getDeviceStoryThumbnail("E:\\", "a");
  await cancelDeviceStoryDetails();
  expect(calls).toEqual([
    'device_story_titles:{"root":"E:\\\\","uuids":["a"]}',
    'device_story_thumbnail:{"root":"E:\\\\","uuid":"a"}',
    "device_story_details_cancel:{}",
  ]);
});

it("transmet la racine et la Flam attendue à device_list_stories, rien sans racine (#577)", async () => {
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    calls.push([cmd, args]);
    return null;
  });
  await listDeviceStories("E:\\", { fingerprint: "ab" });
  await listDeviceStories();
  expect(calls).toEqual([
    ["device_list_stories", { root: "E:\\", expected: { fingerprint: "ab" } }],
    ["device_list_stories", {}],
  ]);
});

it("appelle les commandes de fermeture retenue (#577)", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    if (cmd === "device_close_verdict") return "retain";
    if (cmd === "device_write_in_progress") return "transfer";
    return true;
  });
  expect(await getCloseVerdict()).toBe("retain");
  expect(await getWriteInProgress()).toBe("transfer");
  expect(await cancelClose()).toBe(true);
  expect(calls).toEqual([
    "device_close_verdict",
    "device_write_in_progress",
    "device_cancel_close",
  ]);
});
