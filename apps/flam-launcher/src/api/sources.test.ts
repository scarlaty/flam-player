import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, expect, it } from "vitest";
import {
  addSource,
  listSources,
  removeSource,
  reorderSources,
  updateSource,
  type Source,
} from "./sources";

afterEach(() => clearMocks());

const library: Source = { type: "library", id: "library", name: "Mes histoires" };
const catalog: Source = {
  type: "catalog",
  id: "a",
  name: "A",
  url: "https://exemple.org/a.json",
};

/** Simule le backend et enregistre commande et arguments reçus. */
function mockBackend(result: unknown) {
  const calls: { cmd: string; args: unknown }[] = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args });
    return result;
  });
  return calls;
}

it("listSources appelle sources_list", async () => {
  const view = {
    sources: [library, catalog],
    recovery: { file: "sources.json", backup: "sources.json.20261015-101010.bak" },
  };
  const calls = mockBackend(view);
  await expect(listSources()).resolves.toEqual(view);
  expect(calls).toEqual([{ cmd: "sources_list", args: {} }]);
});

it("addSource envoie l'entrée à sources_add", async () => {
  const calls = mockBackend(catalog);
  const input = {
    type: "catalog",
    name: "A",
    url: "https://exemple.org/a.json",
  } as const;
  await expect(addSource(input)).resolves.toEqual(catalog);
  expect(calls).toEqual([{ cmd: "sources_add", args: { input } }]);
});

it("updateSource envoie l'identifiant et l'entrée à sources_update", async () => {
  const calls = mockBackend(catalog);
  const input = { type: "folder", name: "D", path: "C:\\histoires" } as const;
  await updateSource("a", input);
  expect(calls).toEqual([{ cmd: "sources_update", args: { id: "a", input } }]);
});

it("removeSource appelle sources_remove", async () => {
  const calls = mockBackend(null);
  await removeSource("a");
  expect(calls).toEqual([{ cmd: "sources_remove", args: { id: "a" } }]);
});

it("reorderSources envoie les identifiants dans l'ordre voulu", async () => {
  const calls = mockBackend([library, catalog]);
  await expect(reorderSources(["b", "a"])).resolves.toEqual([library, catalog]);
  expect(calls).toEqual([{ cmd: "sources_reorder", args: { ids: ["b", "a"] } }]);
});

it("les erreurs du backend gardent leur kind", async () => {
  mockIPC(() => {
    throw { kind: "not_found", message: "source introuvable : x" };
  });
  await expect(removeSource("x")).rejects.toMatchObject({ kind: "not_found" });
});
