import { afterEach, expect, it, vi } from "vitest";
import type { DeviceStoryThumbnail } from "../../api/device";
import { DETAILS_RETRY_MS, THUMB_CONCURRENCY, ThumbnailLoader } from "./storyDetails";

afterEach(() => {
  vi.useRealTimers();
});

const PNG = "data:image/png;base64,QUJD";

/** Récupérateur contrôlé : chaque appel attend `answer`. */
function controlled() {
  const pending: Array<{ uuid: string; answer: (r: DeviceStoryThumbnail) => void }> = [];
  const fetcher = (_root: string, uuid: string) =>
    new Promise<DeviceStoryThumbnail>((resolve) =>
      pending.push({ uuid, answer: resolve }),
    );
  return { pending, fetcher };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

it("limite les lectures simultanées et garde les résultats", async () => {
  const { pending, fetcher } = controlled();
  const loader = new ThumbnailLoader("E:\\", fetcher);
  const results = ["a", "b", "c", "d"].map((uuid) => loader.request(uuid));
  // Même vignette demandée deux fois : une seule lecture.
  const again = loader.request("a");
  expect(pending.map((p) => p.uuid)).toEqual(["a", "b"].slice(0, THUMB_CONCURRENCY));
  pending[0]!.answer({ thumb: PNG, busy: false });
  await flush();
  expect(pending).toHaveLength(THUMB_CONCURRENCY + 1);
  for (const p of pending.slice(1)) p.answer({ thumb: null, busy: false });
  await flush();
  for (const p of pending.slice(THUMB_CONCURRENCY + 1))
    p.answer({ thumb: null, busy: false });
  await flush();
  expect(await results[0]).toBe(PNG);
  expect(await again).toBe(PNG);
  expect(await results[1]).toBeNull();
  expect(loader.peek("a")).toBe(PNG);
  expect(loader.peek("z")).toBeUndefined();
  // Déjà connue : aucune nouvelle lecture.
  const before = pending.length;
  expect(await loader.request("a")).toBe(PNG);
  expect(pending).toHaveLength(before);
});

it("refuse une réponse qui n'est pas une image et traite l'échec comme absente", async () => {
  const loader = new ThumbnailLoader("E:\\", (_root, uuid) =>
    uuid === "x"
      ? Promise.resolve({ thumb: "javascript:alert(1)", busy: false })
      : Promise.reject(new Error("boum")),
  );
  expect(await loader.request("x")).toBeNull();
  expect(await loader.request("y")).toBeNull();
});

it("redemande plus tard quand la Flam est occupée", async () => {
  vi.useFakeTimers();
  let calls = 0;
  const loader = new ThumbnailLoader("E:\\", () => {
    calls += 1;
    return Promise.resolve(
      calls === 1 ? { thumb: null, busy: true } : { thumb: PNG, busy: false },
    );
  });
  const result = loader.request("a");
  await vi.advanceTimersByTimeAsync(DETAILS_RETRY_MS - 10);
  expect(calls).toBe(1);
  await vi.advanceTimersByTimeAsync(20);
  expect(await result).toBe(PNG);
  expect(calls).toBe(2);
});

it("suspend les lectures pendant une opération et abandonne au changement de Flam", async () => {
  const { pending, fetcher } = controlled();
  const loader = new ThumbnailLoader("E:\\", fetcher);
  loader.setPaused(true);
  const result = loader.request("a");
  expect(pending).toHaveLength(0);
  loader.setPaused(false);
  expect(pending).toHaveLength(1);
  const waiting = loader.request("b");
  const queued = loader.request("c");
  loader.dispose();
  expect(await queued).toBeNull();
  pending[0]!.answer({ thumb: PNG, busy: false });
  expect(await waiting).toBeNull();
  await flush();
  // Après abandon, aucune nouvelle lecture.
  expect(pending).toHaveLength(THUMB_CONCURRENCY);
  void result;
});
