import { expect, it } from "vitest";
import {
  estimateSize,
  acceptedForShortcut,
  shortcutOutcome,
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

it("un même identifiant dans deux flux désigne deux épisodes distincts", () => {
  const other = { ...item("a", 1), sourceId: "autre" };
  expect(refusalFor([item("a", 1)], other)).toBeNull();
  expect(refusalFor([item("a", 1), other], other)).toBe("duplicate");
});

it("raccourci : garde les épisodes acceptés par les plafonds, saute explicites et doublons", () => {
  const selected = [item("a", 10)];
  const candidates = [
    item("a", 10),
    { ...item("x", 10), explicit: true },
    item("big", MAX_EPISODE_BYTES + 1),
    item("b", 10),
  ];
  expect(acceptedForShortcut(selected, candidates).map((e) => e.id)).toEqual(["b"]);
});

it("raccourci : distingue déjà cochés, explicites et plafonds", () => {
  const outcome = shortcutOutcome(
    [item("a", 10)],
    [
      item("a", 10),
      { ...item("x", 10), explicit: true },
      item("big", MAX_EPISODE_BYTES + 1),
      item("b", 10),
    ],
  );
  expect(outcome.accepted.map((e) => e.id)).toEqual(["b"]);
  expect([outcome.already, outcome.explicit, outcome.capped]).toEqual([1, 1, 1]);
});

it("raccourci : tient compte du plafond total au fil des ajouts", () => {
  const big = 190_000_000;
  const candidates = [..."abcde"].map((id) => item(id, big));
  candidates.push(item("f", big), item("g", 1));
  const accepted = acceptedForShortcut([], candidates);
  expect(accepted.map((e) => e.id)).toEqual(["a", "b", "c", "d", "e", "g"]);
});

it("raccourci : s'arrête au plafond du nombre d'épisodes", () => {
  const selected = Array.from({ length: MAX_EPISODES - 2 }, (_, i) => item(`s${i}`, 1));
  const accepted = acceptedForShortcut(selected, [
    item("a", 1),
    item("b", 1),
    item("c", 1),
  ]);
  expect(accepted.map((e) => e.id)).toEqual(["a", "b"]);
});
