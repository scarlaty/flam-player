import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import {
  hasUpdate,
  libraryThumbnail,
  listLibrary,
  removeFromLibrary,
  type LibraryEntry,
} from "./library";
import type { StoryEntry } from "./story";

afterEach(() => clearMocks());

const story: StoryEntry = {
  source_id: "telmi-store",
  entry_id: "telmi-store:abc",
  uuid: "abc",
  version: 3,
  title: "Histoire",
  description: null,
  age: null,
  category: null,
  author: null,
  voice: null,
  license: null,
  thumb_url: null,
  download_url: "https://exemple.org/abc.zip",
  size: null,
  updated_at: null,
  kind: "telmi_zip",
};

const installed: LibraryEntry = {
  uuid: "abc",
  version: 2,
  title: "Histoire",
  file: "abc-2.plain.pk",
  size: 10,
  installed_at: 1,
  converter: "sha256:0123456789ab",
  source: { ...story, version: 2 },
};

it("listLibrary et removeFromLibrary appellent leurs commandes", async () => {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return cmd === "library_list" ? [installed] : null;
  });
  await expect(listLibrary()).resolves.toEqual([installed]);
  await removeFromLibrary("abc");
  await removeFromLibrary("abc", true);
  expect(calls).toEqual([
    { cmd: "library_list", args: {} },
    // Par défaut la progression de l'enfant est conservée.
    { cmd: "library_remove", args: { uuid: "abc", purgeSaves: false } },
    { cmd: "library_remove", args: { uuid: "abc", purgeSaves: true } },
  ]);
});

it("hasUpdate compare les versions connues d'un même uuid", () => {
  expect(hasUpdate(installed, story)).toBe(true);
  expect(hasUpdate(installed, { ...story, version: 2 })).toBe(false);
  expect(hasUpdate(installed, { ...story, version: null })).toBe(false);
  expect(hasUpdate(installed, { ...story, uuid: "autre" })).toBe(false);
  expect(hasUpdate(undefined, story)).toBe(false);
});

it("libraryThumbnail appelle library_thumbnail avec l'uuid", async () => {
  let received: unknown;
  mockIPC((cmd, args) => {
    received = { cmd, args };
    return "data:image/png;base64,AA";
  });
  await expect(libraryThumbnail("abc")).resolves.toBe("data:image/png;base64,AA");
  expect(received).toEqual({ cmd: "library_thumbnail", args: { uuid: "abc" } });
});
