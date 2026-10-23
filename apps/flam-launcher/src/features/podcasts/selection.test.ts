import { expect, it } from "vitest";
import {
  estimateSize,
  latestIds,
  MAX_EPISODE_BYTES,
  MAX_EPISODES,
  MAX_PACK_BYTES,
  refusalFor,
  totalsOf,
  type SelectedEpisode,
} from "./selection";

function item(id: string, bytes: number | null): SelectedEpisode {
  return { id, sourceId: "s", sourceName: "S", title: id, bytes, explicit: false };
}

it("estime la taille : annoncée, sinon durée à 192 kbit/s, sinon inconnue", () => {
  expect(estimateSize({ enclosure_length: 5_000_000, duration: 60 })).toEqual({
    bytes: 5_000_000,
    approximate: false,
  });
  expect(estimateSize({ enclosure_length: null, duration: 3600 })).toEqual({
    bytes: 86_400_000,
    approximate: true,
  });
  expect(estimateSize({ enclosure_length: 0, duration: null }).bytes).toBeNull();
});

it("totalise les tailles connues et compte les inconnues", () => {
  expect(totalsOf([item("a", 10), item("b", null), item("c", 5)])).toEqual({
    count: 3,
    bytes: 15,
    unknownSize: 1,
  });
});

it("refuse un épisode trop gros, un doublon, le 51e épisode et le dépassement du total", () => {
  expect(refusalFor([], item("a", MAX_EPISODE_BYTES + 1))).toBe("episodeTooBig");
  expect(refusalFor([], item("a", MAX_EPISODE_BYTES))).toBeNull();
  expect(refusalFor([item("a", 1)], item("a", 1))).toBe("duplicate");
  const full = Array.from({ length: MAX_EPISODES }, (_, i) => item(`e${i}`, 1));
  expect(refusalFor(full, item("x", 1))).toBe("count");
  const heavy = Array.from({ length: 5 }, (_, i) => item(`h${i}`, MAX_EPISODE_BYTES));
  const room = MAX_PACK_BYTES - 5 * MAX_EPISODE_BYTES;
  expect(refusalFor(heavy, item("y", room + 1))).toBe("total");
  expect(refusalFor(heavy, item("y", room))).toBeNull();
});

it("latestIds prend les n premiers épisodes", () => {
  const episodes = [{ id: "1" }, { id: "2" }, { id: "3" }];
  expect(latestIds(episodes, 2)).toEqual(["1", "2"]);
  expect(latestIds(episodes, -1)).toEqual([]);
});
