import { describe, expect, it } from "vitest";
import {
  clip,
  edgeScroll,
  hitZone,
  pastThreshold,
  physicalToCss,
  rowAt,
  slotAt,
  spotAt,
  type Rect,
  type RowBox,
} from "./geometry";

const rect = (left: number, top: number, right: number, bottom: number): Rect => ({
  left,
  top,
  right,
  bottom,
});

const row = (id: string, index: number, menu = false): RowBox => ({
  id,
  menu,
  rect: rect(0, index * 40, 300, index * 40 + 40),
});

describe("hitZone", () => {
  it("rend la plus petite zone qui contient le point", () => {
    const zones = [
      { name: "arbre", rect: rect(0, 0, 300, 1000) },
      { name: "ligne", rect: rect(0, 40, 300, 80) },
      { name: "image", rect: rect(400, 0, 800, 200) },
    ];
    expect(hitZone(zones, { x: 10, y: 50 })?.name).toBe("ligne");
    expect(hitZone(zones, { x: 10, y: 500 })?.name).toBe("arbre");
    expect(hitZone(zones, { x: 500, y: 100 })?.name).toBe("image");
    expect(hitZone(zones, { x: 350, y: 100 })).toBeNull();
    expect(hitZone([], { x: 0, y: 0 })).toBeNull();
  });
});

describe("slotAt", () => {
  it("histoire : moitié haute avant, moitié basse après", () => {
    const story = row("a.mp3", 1);
    expect(slotAt(story, 41)).toBe("before");
    expect(slotAt(story, 59)).toBe("before");
    expect(slotAt(story, 61)).toBe("after");
  });

  it("menu : un quart avant, la moitié dans, un quart après", () => {
    const menu = row("M", 1, true);
    expect(slotAt(menu, 45)).toBe("before");
    expect(slotAt(menu, 60)).toBe("inside");
    expect(slotAt(menu, 75)).toBe("after");
  });

  it("racine : toujours dans", () => {
    const root = row("", 0, true);
    expect(slotAt(root, 1)).toBe("inside");
    expect(slotAt(root, 39)).toBe("inside");
  });
});

describe("rowAt et spotAt", () => {
  const rows = [row("", 0, true), row("M", 1, true), row("M/a.mp3", 2)];

  it("trouve la ligne sous le pointeur, ou la plus proche hors des lignes", () => {
    expect(rowAt(rows, 50)?.id).toBe("M");
    expect(rowAt(rows, -20)?.id).toBe("");
    expect(rowAt(rows, 500)?.id).toBe("M/a.mp3");
    expect(rowAt([], 10)).toBeNull();
    const gap = [row("a", 0), { ...row("b", 1), rect: rect(0, 50, 300, 90) }];
    expect(rowAt(gap, 44)?.id).toBe("a");
    expect(rowAt(gap, 48)?.id).toBe("b");
  });

  it("sous la dernière ligne : après elle", () => {
    expect(spotAt(rows, 500)).toEqual({ id: "M/a.mp3", slot: "after" });
    expect(spotAt(rows, 85)).toEqual({ id: "M/a.mp3", slot: "before" });
    expect(spotAt(rows, -5)).toEqual({ id: "", slot: "inside" });
    expect(spotAt([], 5)).toBeNull();
  });
});

describe("edgeScroll", () => {
  const box = rect(0, 100, 300, 500);
  it("défile près des bords, plus vite au bord", () => {
    expect(edgeScroll(box, 300)).toBe(0);
    expect(edgeScroll(box, 120)).toBeLessThan(0);
    expect(edgeScroll(box, 480)).toBeGreaterThan(0);
    expect(edgeScroll(box, 101)).toBeLessThan(edgeScroll(box, 125));
    expect(edgeScroll(box, 50)).toBe(-18);
    expect(edgeScroll(box, 600)).toBe(18);
  });
});

describe("clip", () => {
  it("garde la partie visible ; rien si l'élément est défilé hors vue", () => {
    const view = rect(0, 100, 300, 500);
    expect(clip(rect(0, 80, 300, 120), view)).toEqual(rect(0, 100, 300, 120));
    expect(clip(rect(0, 20, 300, 60), view)).toBeNull();
    expect(clip(rect(0, 500, 300, 540), view)).toBeNull();
    expect(clip(rect(0, 20, 300, 60), null)).toEqual(rect(0, 20, 300, 60));
  });
});

describe("pastThreshold et physicalToCss", () => {
  it("le glissement commence au-delà de quelques pixels", () => {
    expect(pastThreshold({ x: 0, y: 0 }, { x: 3, y: 3 })).toBe(false);
    expect(pastThreshold({ x: 0, y: 0 }, { x: 0, y: 6 })).toBe(true);
  });

  it("convertit les pixels physiques en pixels CSS", () => {
    expect(physicalToCss({ x: 300, y: 150 }, 1.5)).toEqual({ x: 200, y: 100 });
    expect(physicalToCss({ x: 30, y: 15 }, 0)).toEqual({ x: 30, y: 15 });
    expect(physicalToCss({ x: 30, y: 15 }, Number.NaN)).toEqual({ x: 30, y: 15 });
  });
});
