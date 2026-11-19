import { describe, expect, it } from "vitest";
import type { EditorNode } from "../../../../api/editor";
import {
  baseName,
  destinationOf,
  fileZoneAt,
  pathKind,
  planFileDrop,
  planTreeMove,
  type ZoneBox,
} from "./plan";

const story = (id: string, title = id): EditorNode => ({
  kind: "story",
  id,
  file: id.split("/").pop() ?? id,
  title,
  image: null,
  title_audio: null,
  bytes: 1,
});

const menu = (
  id: string,
  children: EditorNode[],
  title = id || "Racine",
): EditorNode => ({
  kind: "menu",
  id,
  dir: id.split("/").pop() ?? id,
  title,
  mode: "recueil",
  fin: "menu",
  annoncer: false,
  image: null,
  title_audio: null,
  children,
});

// Racine : A (menu : A/1, A/2, A/B (menu : A/B/x)), r.mp3
const root = menu("", [
  menu("A", [story("A/1.mp3"), story("A/2.mp3"), menu("A/B", [story("A/B/x.mp3")])]),
  story("r.mp3"),
]);
const allOpen = () => true;

describe("destinationOf", () => {
  it("dans un menu : à sa fin ; avant ou après une option : dans son menu", () => {
    expect(destinationOf(root, { id: "A", slot: "inside" })).toMatchObject({
      menu: { id: "A" },
      beforeId: null,
    });
    expect(destinationOf(root, { id: "A/2.mp3", slot: "before" })).toMatchObject({
      menu: { id: "A" },
      beforeId: "A/2.mp3",
    });
    expect(destinationOf(root, { id: "A/1.mp3", slot: "after" })).toMatchObject({
      menu: { id: "A" },
      beforeId: "A/2.mp3",
    });
    expect(destinationOf(root, { id: "r.mp3", slot: "after" })).toMatchObject({
      menu: { id: "" },
      beforeId: null,
    });
    expect(destinationOf(root, { id: "", slot: "before" })).toMatchObject({
      menu: { id: "" },
      beforeId: null,
    });
    expect(destinationOf(root, { id: "absent", slot: "before" })).toBeNull();
  });

  it("après un menu déplié : en tête de ses options ; replié : après lui", () => {
    expect(destinationOf(root, { id: "A", slot: "after" }, allOpen)).toMatchObject({
      menu: { id: "A" },
      beforeId: "A/1.mp3",
    });
    expect(destinationOf(root, { id: "A", slot: "after" })).toMatchObject({
      menu: { id: "" },
      beforeId: "r.mp3",
    });
  });
});

describe("planTreeMove", () => {
  it("même place : rien", () => {
    expect(planTreeMove(root, "A/1.mp3", { id: "A/1.mp3", slot: "after" })).toEqual({
      kind: "none",
    });
    expect(planTreeMove(root, "A/1.mp3", { id: "A/2.mp3", slot: "before" })).toEqual({
      kind: "none",
    });
    expect(planTreeMove(root, "A/2.mp3", { id: "A/1.mp3", slot: "after" })).toEqual({
      kind: "none",
    });
    expect(planTreeMove(root, "A/1.mp3", null)).toEqual({ kind: "none" });
    expect(planTreeMove(root, "absent", { id: "A", slot: "inside" })).toEqual({
      kind: "none",
    });
  });

  it("même menu : nouvel ordre", () => {
    expect(planTreeMove(root, "A/1.mp3", { id: "A/2.mp3", slot: "after" })).toEqual({
      kind: "reorder",
      menu: "A",
      menuTitle: "A",
      order: ["2.mp3", "1.mp3", "B"],
    });
    expect(planTreeMove(root, "A/B", { id: "A/1.mp3", slot: "before" })).toEqual({
      kind: "reorder",
      menu: "A",
      menuTitle: "A",
      order: ["B", "1.mp3", "2.mp3"],
    });
    // « Dans » son propre menu : à la fin.
    expect(planTreeMove(root, "A/1.mp3", { id: "A", slot: "inside" })).toEqual({
      kind: "reorder",
      menu: "A",
      menuTitle: "A",
      order: ["2.mp3", "B", "1.mp3"],
    });
  });

  it("autre menu : déplacement, avant l'option visée (nom) ou à la fin", () => {
    expect(planTreeMove(root, "r.mp3", { id: "A/2.mp3", slot: "before" })).toEqual({
      kind: "move",
      dest: "A",
      destTitle: "A",
      before: "2.mp3",
    });
    expect(planTreeMove(root, "A/1.mp3", { id: "A/B", slot: "inside" })).toEqual({
      kind: "move",
      dest: "A/B",
      destTitle: "A/B",
      before: null,
    });
    expect(planTreeMove(root, "A/B/x.mp3", { id: "", slot: "inside" })).toEqual({
      kind: "move",
      dest: "",
      destTitle: "Racine",
      before: null,
    });
  });

  it("un menu ne va ni dans lui-même ni dans sa descendance ; la racine ne bouge pas", () => {
    expect(planTreeMove(root, "A", { id: "A/B", slot: "inside" })).toEqual({
      kind: "invalid",
    });
    expect(planTreeMove(root, "A", { id: "A/B/x.mp3", slot: "before" })).toEqual({
      kind: "invalid",
    });
    expect(planTreeMove(root, "A", { id: "A", slot: "after" }, allOpen)).toEqual({
      kind: "none",
    });
    expect(planTreeMove(root, "A", { id: "A/1.mp3", slot: "before" })).toEqual({
      kind: "invalid",
    });
    expect(planTreeMove(root, "", { id: "A", slot: "inside" })).toEqual({
      kind: "invalid",
    });
    // Un menu voisin dont le nom commence pareil n'est pas sa descendance.
    const twins = menu("", [menu("A", []), menu("AB", [])]);
    expect(planTreeMove(twins, "A", { id: "AB", slot: "inside" })).toMatchObject({
      kind: "move",
      dest: "AB",
    });
  });
});

describe("pathKind", () => {
  it("reconnaît audio, images, autres fichiers et dossiers probables", () => {
    expect(pathKind("C:\\Musique\\Loup.MP3")).toBe("audio");
    expect(pathKind("C:\\Images\\loup.jpeg")).toBe("image");
    expect(pathKind("C:\\Docs\\note.txt")).toBe("other");
    expect(pathKind("C:\\Docs\\livre.docx")).toBe("other");
    expect(pathKind("C:\\Histoires\\Contes")).toBe("folder");
    expect(pathKind("C:\\Histoires\\Vol. 1")).toBe("folder");
    expect(pathKind("C:\\Histoires\\Contes\\")).toBe("folder");
    expect(pathKind("/home/x/.cache")).toBe("folder");
    expect(baseName("C:\\a\\b/c.mp3")).toBe("c.mp3");
  });
});

describe("planFileDrop", () => {
  const audio = "C:\\Src\\loup.mp3";
  const image = "C:\\Src\\loup.png";

  it("audio sur un menu : copie à sa fin", () => {
    expect(
      planFileDrop(root, { kind: "row", spot: { id: "A", slot: "inside" } }, [audio]),
    ).toEqual({
      kind: "import",
      menu: "A",
      menuTitle: "A",
      before: null,
      sources: [audio],
      rejected: [],
      hint: { kind: "row", id: "A", slot: "inside" },
    });
  });

  it("entre deux éléments : copie à cette place ; formats refusés laissés de côté", () => {
    const plan = planFileDrop(
      root,
      { kind: "row", spot: { id: "A/2.mp3", slot: "before" } },
      [audio, image, "C:\\Src\\Dossier", "C:\\Src\\note.txt"],
    );
    expect(plan).toEqual({
      kind: "import",
      menu: "A",
      menuTitle: "A",
      before: "2.mp3",
      sources: [audio, image, "C:\\Src\\Dossier"],
      rejected: ["C:\\Src\\note.txt"],
      hint: { kind: "row", id: "A/2.mp3", slot: "before" },
    });
  });

  it("zone vide de l'arbre : fin de la racine", () => {
    expect(planFileDrop(root, { kind: "tree" }, ["C:\\Src\\Contes"])).toMatchObject({
      kind: "import",
      menu: "",
      before: null,
    });
  });

  it("une image sur un élément : son image ; plusieurs : refus", () => {
    expect(
      planFileDrop(root, { kind: "row", spot: { id: "A/1.mp3", slot: "after" } }, [
        image,
      ]),
    ).toEqual({
      kind: "image",
      id: "A/1.mp3",
      title: "A/1.mp3",
      source: image,
      hint: { kind: "row", id: "A/1.mp3", slot: "inside" },
    });
    expect(
      planFileDrop(root, { kind: "row", spot: { id: "A", slot: "inside" } }, [
        image,
        image,
      ]),
    ).toMatchObject({ kind: "refuse", reason: "oneImage" });
  });

  it("rien d'accepté : refus, rien à copier", () => {
    expect(
      planFileDrop(root, { kind: "row", spot: { id: "A", slot: "inside" } }, [
        "C:\\a.txt",
        "C:\\b.pdf",
      ]),
    ).toMatchObject({ kind: "refuse", reason: "formats" });
    expect(planFileDrop(root, null, [audio])).toEqual({
      kind: "refuse",
      reason: "nowhere",
      hint: null,
    });
    expect(
      planFileDrop(root, { kind: "row", spot: { id: "absent", slot: "inside" } }, [
        audio,
      ]),
    ).toMatchObject({ kind: "refuse", reason: "nowhere" });
  });

  it("zones du panneau : une image, un titre lu, un fichier à la fois", () => {
    expect(planFileDrop(root, { kind: "image", id: "A" }, [image])).toMatchObject({
      kind: "image",
      id: "A",
      source: image,
      hint: { kind: "image" },
    });
    expect(planFileDrop(root, { kind: "image", id: "A" }, [audio])).toMatchObject({
      kind: "refuse",
      reason: "notImage",
    });
    expect(planFileDrop(root, { kind: "image", id: "A" }, [image, image])).toMatchObject({
      kind: "refuse",
      reason: "oneImage",
    });
    expect(
      planFileDrop(root, { kind: "titleAudio", id: "r.mp3" }, [audio]),
    ).toMatchObject({
      kind: "titleAudio",
      id: "r.mp3",
      source: audio,
    });
    expect(
      planFileDrop(root, { kind: "titleAudio", id: "r.mp3" }, [image]),
    ).toMatchObject({ kind: "refuse", reason: "notAudio" });
    expect(
      planFileDrop(root, { kind: "titleAudio", id: "r.mp3" }, [audio, audio]),
    ).toMatchObject({ kind: "refuse", reason: "oneAudio" });
  });
});

describe("fileZoneAt", () => {
  const zones: ZoneBox[] = [
    { kind: "tree", rect: { left: 0, top: 0, right: 300, bottom: 1000 } },
    {
      kind: "row",
      rect: { left: 0, top: 40, right: 300, bottom: 80 },
      row: { id: "A", menu: true, rect: { left: 0, top: 40, right: 300, bottom: 80 } },
    },
    { kind: "image", rect: { left: 400, top: 0, right: 800, bottom: 200 } },
    { kind: "titleAudio", rect: { left: 400, top: 200, right: 800, bottom: 400 } },
  ];

  it("ligne, zone vide de l'arbre, zones du panneau de l'élément choisi", () => {
    expect(fileZoneAt(zones, { x: 10, y: 60 }, "r.mp3")).toEqual({
      kind: "row",
      spot: { id: "A", slot: "inside" },
    });
    expect(fileZoneAt(zones, { x: 10, y: 42 }, "r.mp3")).toEqual({
      kind: "row",
      spot: { id: "A", slot: "before" },
    });
    expect(fileZoneAt(zones, { x: 10, y: 900 }, "r.mp3")).toEqual({ kind: "tree" });
    expect(fileZoneAt(zones, { x: 500, y: 100 }, "r.mp3")).toEqual({
      kind: "image",
      id: "r.mp3",
    });
    expect(fileZoneAt(zones, { x: 500, y: 300 }, "r.mp3")).toEqual({
      kind: "titleAudio",
      id: "r.mp3",
    });
    expect(fileZoneAt(zones, { x: 350, y: 300 }, "r.mp3")).toBeNull();
  });
});
