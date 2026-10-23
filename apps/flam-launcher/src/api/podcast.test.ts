import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import { addPodcast, getPodcastFeed, refreshPodcast } from "./podcast";

afterEach(() => clearMocks());

function mockBackend(result: unknown) {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return result;
  });
  return calls;
}

it("addPodcast envoie l'adresse et le nom à podcast_add", async () => {
  const calls = mockBackend({});
  await addPodcast("https://exemple.org/f.xml", "Mon flux");
  await addPodcast("https://exemple.org/f.xml");
  expect(calls).toEqual([
    { cmd: "podcast_add", args: { url: "https://exemple.org/f.xml", name: "Mon flux" } },
    { cmd: "podcast_add", args: { url: "https://exemple.org/f.xml", name: null } },
  ]);
});

it("getPodcastFeed et refreshPodcast désignent la source", async () => {
  const calls = mockBackend({});
  await getPodcastFeed("s1");
  await refreshPodcast("s1");
  expect(calls).toEqual([
    { cmd: "podcast_feed", args: { sourceId: "s1" } },
    { cmd: "podcast_refresh", args: { sourceId: "s1" } },
  ]);
});
