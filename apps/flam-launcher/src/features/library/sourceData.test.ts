import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import type { Source } from "../../api/sources";
import { loadSourceData, summarize } from "./sourceData";

afterEach(() => clearMocks());

it("une source podcast donne une vue vide, sans appel au backend (#211)", async () => {
  const calls: string[] = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    throw { kind: "unknown", message: cmd };
  });
  const podcast: Source = {
    type: "podcast",
    id: "p1",
    name: "Contes du soir",
    url: "https://flux.exemple.org/contes.xml",
    feed_hint: "https://flux.exemple.org/…",
  };
  for (const refresh of [false, true]) {
    const data = await loadSourceData(podcast, refresh);
    expect(data).toEqual({
      entries: [],
      warnings: [],
      offline: false,
      updatedAt: null,
      problem: null,
    });
    expect(summarize(data)).toEqual({ status: "ok", count: 0 });
  }
  expect(calls).toEqual([]);
});
