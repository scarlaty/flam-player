import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  FOLDER_CHANGED_EVENT,
  FOLDER_MISSING_EVENT,
  type EditorNode,
  type EditorView,
} from "../../../api/editor";
import { t } from "../../../i18n";
import { useFolderWatch } from "./useFolderWatch";

const text = t.authoringEditor.watch;

function view(children: string[]): EditorView {
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
    children: children.map((id) => ({
      kind: "story",
      id,
      file: id,
      title: id,
      image: null,
      title_audio: null,
      bytes: 1,
    })),
  };
  return {
    report: {
      folder: "C:\\Histoires\\Loup",
      title: "Le loup",
      root,
      problems: [],
      blocked: false,
      story_count: children.length,
      audio_bytes: children.length,
      mode: "recueil",
      suggested_mode: null,
      uuid: null,
      installed_version: null,
      remembered: false,
      conflict: null,
    },
    folder_name: "Loup",
    stamps: {},
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
    folders: {},
    journal: { can_undo: false, can_redo: false, undo_label: null, redo_label: null },
  };
}

/** Vue rendue par `authoring_editor_view` (modifiable entre deux événements). */
let current: EditorView;
let reads = 0;
/** Lecture retenue : résolue à la main (relecture lente). */
let hold: (() => void) | null = null;
let holding = false;

beforeEach(() => {
  current = view(["a.mp3"]);
  reads = 0;
  hold = null;
  holding = false;
  mockWindows("editor-1");
  mockIPC(
    async (cmd) => {
      if (cmd === "authoring_editor_view") {
        reads += 1;
        if (holding) await new Promise<void>((resolve) => (hold = resolve));
        return current;
      }
      return null;
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  // Démonter avant de retirer les simulations : les désabonnements passent par elles.
  cleanup();
  clearMocks();
});

function setup(busy = false) {
  const onView = vi.fn<(v: EditorView) => void>();
  const onNotice = vi.fn<(m: string) => void>();
  const hook = renderHook(
    (props: { busy: boolean }) => useFolderWatch({ busy: props.busy, onView, onNotice }),
    { initialProps: { busy } },
  );
  return { onView, onNotice, hook };
}

/** Laisse l'abonnement aux événements s'installer. */
async function subscribed() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

it("un changement ailleurs relit la vue et l'annonce", async () => {
  const { onView, onNotice } = setup();
  await subscribed();
  current = view(["a.mp3", "b.mp3"]);
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await waitFor(() => expect(onView).toHaveBeenCalledTimes(1));
  expect(onView.mock.lastCall?.[0].report.story_count).toBe(2);
  expect(onNotice).toHaveBeenCalledWith(text.changed);
});

it("pendant une opération, la relecture attend sa fin", async () => {
  const { onView, hook } = setup(true);
  await subscribed();
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await subscribed();
  expect(reads).toBe(0);
  hook.rerender({ busy: false });
  await waitFor(() => expect(onView).toHaveBeenCalledTimes(1));
  expect(reads).toBe(1);
});

it("une relecture commencée avant une opération est jetée puis refaite", async () => {
  const { onView, hook } = setup();
  await subscribed();
  holding = true;
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await waitFor(() => expect(reads).toBe(1));
  // Une opération commence : la lecture en cours est périmée.
  hook.rerender({ busy: true });
  holding = false;
  await act(async () => hold?.());
  await subscribed();
  expect(onView).not.toHaveBeenCalled();
  hook.rerender({ busy: false });
  await waitFor(() => expect(onView).toHaveBeenCalledTimes(1));
  expect(reads).toBe(2);
});

it("plusieurs changements rapprochés ne donnent qu'une vue appliquée", async () => {
  const { onView } = setup();
  await subscribed();
  holding = true;
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await waitFor(() => expect(reads).toBe(1));
  const first = hold;
  holding = false;
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await waitFor(() => expect(reads).toBe(2));
  await act(async () => first?.());
  await subscribed();
  expect(onView).toHaveBeenCalledTimes(1);
});

it("dossier disparu : lecture seule, puis retour quand il revient", async () => {
  const { onView, onNotice, hook } = setup();
  await subscribed();
  await act(() => emit(FOLDER_MISSING_EVENT));
  await waitFor(() => expect(hook.result.current.missing).toBe(true));
  expect(onNotice).toHaveBeenCalledWith(text.missing);
  expect(onView).not.toHaveBeenCalled();
  // Clé rebranchée : le backend signale un changement.
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await waitFor(() => expect(hook.result.current.missing).toBe(false));
  expect(onView).toHaveBeenCalledTimes(1);
  expect(onNotice).toHaveBeenLastCalledWith(text.back);
});

it("une relecture en échec n'est pas répétée", async () => {
  clearMocks();
  mockWindows("editor-1");
  mockIPC(
    (cmd) => {
      if (cmd === "authoring_editor_view") {
        reads += 1;
        throw { kind: "root_refused", message: "dossier introuvable" };
      }
      return null;
    },
    { shouldMockEvents: true },
  );
  const { onView } = setup();
  await subscribed();
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await waitFor(() => expect(reads).toBe(1));
  await subscribed();
  expect(reads).toBe(1);
  expect(onView).not.toHaveBeenCalled();
});

it("démonté, plus aucune relecture", async () => {
  const { onView, hook } = setup();
  await subscribed();
  hook.unmount();
  await act(() => emit(FOLDER_CHANGED_EVENT));
  await subscribed();
  expect(reads).toBe(0);
  expect(onView).not.toHaveBeenCalled();
});
