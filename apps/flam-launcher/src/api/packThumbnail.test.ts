import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, it } from "vitest";
import { clearThumbnailMemo, episodeThumbnail } from "./pack";

const ref = (id: string) => ({ source_id: "s", episode_id: id });
const PIXEL = "data:image/png;base64,AAAA";

beforeEach(() => clearThumbnailMemo());
afterEach(() => clearMocks());

it("n'envoie que des identifiants et mémorise la vignette obtenue", async () => {
  const calls: unknown[] = [];
  mockIPC((cmd, args) => {
    expect(cmd).toBe("pack_episode_thumbnail");
    calls.push(args);
    return PIXEL;
  });
  await expect(episodeThumbnail(ref("a"))).resolves.toBe(PIXEL);
  await expect(episodeThumbnail(ref("a"))).resolves.toBe(PIXEL);
  expect(calls).toEqual([{ sourceId: "s", episodeId: "a" }]);
  // Un autre épisode est demandé à part.
  await episodeThumbnail(ref("b"));
  expect(calls).toHaveLength(2);
});

it("regroupe les demandes simultanées du même épisode", async () => {
  let calls = 0;
  mockIPC(() => {
    calls += 1;
    return PIXEL;
  });
  const [x, y] = await Promise.all([
    episodeThumbnail(ref("a")),
    episodeThumbnail(ref("a")),
  ]);
  expect([x, y]).toEqual([PIXEL, PIXEL]);
  expect(calls).toBe(1);
});

it("ne mémorise ni l'absence d'aperçu ni l'erreur (réessai possible)", async () => {
  let calls = 0;
  mockIPC(() => {
    calls += 1;
    if (calls === 2) throw { kind: "io", message: "boum" };
    return calls === 1 ? null : PIXEL;
  });
  await expect(episodeThumbnail(ref("a"))).resolves.toBeNull();
  await expect(episodeThumbnail(ref("a"))).rejects.toEqual({
    kind: "io",
    message: "boum",
  });
  await expect(episodeThumbnail(ref("a"))).resolves.toBe(PIXEL);
  expect(calls).toBe(3);
});
