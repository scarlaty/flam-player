import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import {
  cancelDeviceStoryDetails,
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
    if (cmd === "device_story_titles") return { titles: [], interrupted: false };
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
