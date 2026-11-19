import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect } from "vitest";
import {
  FOLDER_CHANGED_EVENT,
  FOLDER_MISSING_EVENT,
  type EditorNode,
  type EditorView,
} from "../../../api/editor";
import { t } from "../../../i18n";
import { slowIt, tolerateLoadInAsyncUtils } from "../../../test/timeouts";
import { EditorScreen } from "./EditorScreen";
import { clearThumbnailCache } from "./Thumbnail";

tolerateLoadInAsyncUtils();

const text = t.authoringEditor;

const story = (id: string, title: string): EditorNode => ({
  kind: "story",
  id,
  file: id.split("/").pop()!,
  title,
  image: null,
  title_audio: null,
  bytes: 10,
});

function makeView(stories: EditorNode[]): EditorView {
  const root: EditorNode = {
    kind: "menu",
    id: "",
    dir: "Loup",
    title: "Le loup",
    mode: "recueil",
    fin: "menu",
    annoncer: false,
    image: null,
    title_audio: null,
    children: [
      {
        kind: "menu",
        id: "Animaux",
        dir: "Animaux",
        title: "Animaux",
        mode: "recueil",
        fin: "menu",
        annoncer: false,
        image: null,
        title_audio: null,
        children: stories,
      },
    ],
  };
  const stamp = { dir: false, len: 10, modifiedMs: 1, modifiedNs: 0 };
  const dir = { dir: true, len: 0, modifiedMs: 0, modifiedNs: 0 };
  return {
    report: {
      folder: "C:\\Histoires\\Loup",
      title: "Le loup",
      root,
      problems: [],
      blocked: false,
      story_count: stories.length,
      audio_bytes: 10 * stories.length,
      mode: "recueil",
      suggested_mode: null,
      uuid: null,
      installed_version: null,
      remembered: false,
      conflict: null,
    },
    folder_name: "Loup",
    stamps: Object.fromEntries([
      ["", dir],
      ["Animaux", dir],
      ...stories.map((s) => [s.id, stamp]),
    ]),
    story: {
      titre: null,
      sous_titre: null,
      description: null,
      age: null,
      debut: null,
      mode: null,
      fin: null,
      annoncer: null,
    },
    folders: { Animaux: { mode: null, fin: null, annoncer: null } },
    journal: { can_undo: false, can_redo: false, undo_label: null, redo_label: null },
  };
}

const chat = story("Animaux/01 Le chat.mp3", "Le chat");
const chien = story("Animaux/02 Le chien.mp3", "Le chien");

let current: EditorView;
let calls: string[] = [];
/** Fenêtre rendue par `authoring_editor_open`. */
let openedIn = "editor-2";

beforeEach(() => {
  clearThumbnailCache();
  current = makeView([chat, chien]);
  calls = [];
  openedIn = "editor-2";
  mockWindows("editor-1");
  mockIPC(
    (cmd) => {
      calls.push(cmd);
      if (cmd === "authoring_editor_view") return current;
      if (cmd === "plugin:dialog|open") return "D:\\Histoires\\Loup";
      if (cmd === "authoring_editor_open")
        return { already_open: false, label: openedIn };
      return null;
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  cleanup();
  clearMocks();
});

async function openOn(title: string) {
  render(<EditorScreen />);
  fireEvent.click(await screen.findByRole("treeitem", { name: new RegExp(title) }));
  await screen.findByRole("heading", { level: 2, name: title });
  // Abonnement aux événements installé.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

slowIt("changement ailleurs : la sélection reste sur l'élément s'il existe", async () => {
  await openOn("Le chat");
  current = makeView([chat, chien, story("Animaux/03 La souris.mp3", "La souris")]);
  await act(() => emit(FOLDER_CHANGED_EVENT));
  expect(await screen.findByRole("treeitem", { name: /La souris/ })).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 2, name: "Le chat" })).toBeInTheDocument();
  expect(await screen.findByText(text.watch.changed)).toBeInTheDocument();
});

slowIt("élément supprimé ailleurs : la sélection passe à son menu", async () => {
  await openOn("Le chat");
  current = makeView([chien]);
  await act(() => emit(FOLDER_CHANGED_EVENT));
  expect(
    await screen.findByRole("heading", { level: 2, name: "Animaux" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("treeitem", { name: /Le chat/ })).toBeNull();
});

slowIt(
  "dossier disparu : lecture seule, « Choisir le dossier » rouvre ailleurs",
  async () => {
    await openOn("Le chat");
    await act(() => emit(FOLDER_MISSING_EVENT));
    expect(await screen.findByText(text.watch.missingTitle)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: text.tree.addFiles })).toBeDisabled();
    expect(screen.getByRole("button", { name: text.tree.newMenu })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: text.watch.choose }));
    await waitFor(() => expect(calls).toContain("plugin:window|close"));
    expect(calls).toContain("authoring_editor_open");
  },
);

slowIt("le même dossier revenu entre-temps : la fenêtre reste ouverte", async () => {
  await openOn("Le chat");
  await act(() => emit(FOLDER_MISSING_EVENT));
  openedIn = "editor-1";
  fireEvent.click(await screen.findByRole("button", { name: text.watch.choose }));
  await waitFor(() => expect(calls).toContain("authoring_editor_open"));
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
  expect(calls).not.toContain("plugin:window|close");
});
