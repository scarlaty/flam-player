import { expect, it } from "vitest";
import type { EditorMenu, EditorNode } from "../../../api/editor";
import {
  findNode,
  formatSize,
  imageOf,
  menusOf,
  nodeForPath,
  parentOf,
  shiftedOrder,
  visibleIds,
} from "./model";

function story(id: string, image: string | null = null): EditorNode {
  const file = id.split("/").pop() ?? id;
  return { kind: "story", id, file, title: file, image, title_audio: null, bytes: 10 };
}

function menu(
  id: string,
  children: EditorNode[],
  image: string | null = null,
): EditorMenu {
  return {
    kind: "menu",
    id,
    dir: id.split("/").pop() || "Racine",
    title: id === "" ? "Mon histoire" : (id.split("/").pop() ?? id),
    mode: "recueil",
    fin: "menu",
    annoncer: false,
    image,
    title_audio: null,
    children,
  };
}

const root = menu("", [
  menu("Animaux", [
    story("Animaux/loup.mp3"),
    story("Animaux/chat.mp3", "Animaux/chat.png"),
  ]),
  story("ours.mp3"),
]);

it("retrouve un élément, son menu et les menus de l'arbre", () => {
  expect(findNode(root, "Animaux/chat.mp3")?.title).toBe("chat.mp3");
  expect(parentOf(root, "Animaux/chat.mp3")?.id).toBe("Animaux");
  expect(parentOf(root, "")).toBeNull();
  expect(menusOf(root)).toEqual([
    { id: "", label: "Mon histoire" },
    { id: "Animaux", label: "Mon histoire > Animaux" },
  ]);
});

it("monter ou descendre change l'ordre des noms, sauf au bord", () => {
  const animaux = parentOf(root, "Animaux/chat.mp3")!;
  expect(shiftedOrder(animaux, "Animaux/chat.mp3", -1)).toEqual(["chat.mp3", "loup.mp3"]);
  expect(shiftedOrder(animaux, "Animaux/chat.mp3", 1)).toBeNull();
  expect(shiftedOrder(animaux, "Animaux/loup.mp3", -1)).toBeNull();
});

it("une ligne du rapport mène à l'élément, à son histoire ou au dossier", () => {
  expect(nodeForPath(root, "Animaux/chat.png").id).toBe("Animaux/chat.mp3");
  expect(nodeForPath(root, "Animaux/notes.txt").id).toBe("Animaux");
  expect(nodeForPath(root, "inconnu/x.txt").id).toBe("");
  expect(nodeForPath(root, "").id).toBe("");
});

it("image automatique d'un menu : celle de sa première histoire, à garder", () => {
  expect(imageOf(findNode(root, "Animaux")!)).toEqual({
    id: "Animaux/chat.png",
    automatic: true,
    keepable: true,
  });
  expect(imageOf(findNode(root, "ours.mp3")!)).toEqual({
    id: null,
    automatic: true,
    keepable: false,
  });
  expect(imageOf(findNode(root, "Animaux/chat.mp3")!).automatic).toBe(false);
});

it("menus repliés : leurs enfants ne sont plus parcourus", () => {
  expect(visibleIds(root, new Set())).toEqual([
    "",
    "Animaux",
    "Animaux/loup.mp3",
    "Animaux/chat.mp3",
    "ours.mp3",
  ]);
  expect(visibleIds(root, new Set(["Animaux"]))).toEqual(["", "Animaux", "ours.mp3"]);
});

it("taille lisible", () => {
  expect(formatSize(500)).toBe("500 o");
  expect(formatSize(1536)).toBe("1,5 Ko");
  expect(formatSize(25 * 1024 * 1024)).toBe("25 Mo");
});
