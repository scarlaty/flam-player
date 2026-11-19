import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { emit as emitEvent } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect, vi } from "vitest";
import {
  FOLDER_CHANGED_EVENT,
  FOLDER_MISSING_EVENT,
  type EditorNode,
  type EditorUpdate,
  type EditorView,
  type Stamp,
} from "../../../../api/editor";
import { slowIt, tolerateLoadInAsyncUtils } from "../../../../test/timeouts";
import { EditorScreen } from "../EditorScreen";
import { clearThumbnailCache } from "../Thumbnail";

tolerateLoadInAsyncUtils();

/** Événements de dépôt de la vue Tauri, simulés. */
type DropPayload =
  | { type: "enter"; paths: string[]; position: { x: number; y: number } }
  | { type: "over"; position: { x: number; y: number } }
  | { type: "drop"; paths: string[]; position: { x: number; y: number } }
  | { type: "leave" };
const drops = vi.hoisted(() => ({
  handler: null as ((event: { payload: unknown }) => void) | null,
  unlistened: 0,
}));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: () => ({
    onDragDropEvent: (handler: (event: { payload: unknown }) => void) => {
      drops.handler = handler;
      return Promise.resolve(() => {
        drops.unlistened += 1;
      });
    },
  }),
}));

async function emit(payload: DropPayload) {
  await waitFor(() => expect(drops.handler).not.toBeNull());
  act(() => drops.handler?.({ payload }));
}

const stamp = (len: number): Stamp => ({ dir: false, len, modifiedMs: 1, modifiedNs: 0 });
const dirStamp: Stamp = { dir: true, len: 0, modifiedMs: 0, modifiedNs: 0 };

function makeView(): EditorView {
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
        children: [
          {
            kind: "story",
            id: "Animaux/01 Le chat.mp3",
            file: "01 Le chat.mp3",
            title: "Le chat",
            image: null,
            title_audio: null,
            bytes: 2048,
          },
          {
            kind: "story",
            id: "Animaux/02 Le chien.mp3",
            file: "02 Le chien.mp3",
            title: "Le chien",
            image: null,
            title_audio: null,
            bytes: 4096,
          },
        ],
      },
    ],
  };
  return {
    report: {
      folder: "C:\\Histoires\\Loup",
      title: "Le loup",
      root,
      problems: [],
      blocked: false,
      story_count: 2,
      audio_bytes: 6144,
      mode: "recueil",
      suggested_mode: null,
      uuid: null,
      installed_version: null,
      remembered: false,
      conflict: null,
    },
    folder_name: "Loup",
    stamps: {
      "": dirStamp,
      Animaux: dirStamp,
      "Animaux/01 Le chat.mp3": stamp(2048),
      "Animaux/02 Le chien.mp3": stamp(4096),
    },
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

type Call = { cmd: string; args: Record<string, unknown> };
let calls: Call[] = [];
const called = (cmd: string) => calls.filter((c) => c.cmd === cmd);

/** Id rendu par une copie (un élément existant de la vue simulée). */
const importedId = "Animaux/01 Le chat.mp3";

/** Relecture du dossier suspendue (lot 9) : la vue n'est rendue qu'à `release()`. */
let viewGate: Promise<void> | null = null;

function backend(view: EditorView) {
  calls = [];
  viewGate = null;
  mockWindows("editor-1");
  mockIPC(
    async (cmd, args) => {
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
      if (cmd === "authoring_editor_view") {
        if (viewGate) await viewGate;
        return view;
      }
      if (cmd.startsWith("authoring_editor_")) {
        // Copie : le backend rend les éléments copiés (rien pour un format refusé).
        const sources =
          cmd === "authoring_editor_import"
            ? ((args as { sources?: string[] }).sources ?? [])
            : [];
        const update: EditorUpdate = {
          label: "Opération",
          ids: sources.some((s) => s.endsWith(".mp3")) ? [importedId] : [],
          skipped: [],
          trashed: null,
          view,
        };
        return update;
      }
      return null;
    },
    { shouldMockEvents: true },
  );
}

/*
 * Mise en page simulée (jsdom ne calcule rien) : lignes de l'arbre de 40 px empilées dans
 * l'ordre affiché, arbre à gauche (0–300), image (400–800, 0–200) et titre lu (200–400) du
 * panneau à droite.
 */
function rect(left: number, top: number, right: number, bottom: number): DOMRect {
  return {
    left,
    top,
    right,
    bottom,
    x: left,
    y: top,
    width: right - left,
    height: bottom - top,
    toJSON: () => ({}),
  };
}

function layout(this: HTMLElement): DOMRect {
  if (this.hasAttribute("data-dnd-row")) {
    const rows = Array.from(document.querySelectorAll("[data-dnd-row]"));
    const index = rows.indexOf(this);
    return rect(0, index * 40, 300, index * 40 + 40);
  }
  if (this.getAttribute("role") === "tree") return rect(0, 0, 300, 1000);
  switch (this.getAttribute("data-dnd-zone")) {
    case "tree":
      return rect(0, 0, 300, 1000);
    case "image":
      return rect(400, 0, 800, 200);
    case "titleAudio":
      return rect(400, 200, 800, 400);
    default:
      return rect(0, 0, 0, 0);
  }
}

beforeEach(() => {
  clearThumbnailCache();
  drops.handler = null;
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(layout);
});
afterEach(() => {
  // Démonter avant de retirer les simulations : les écouteurs de la fenêtre se désabonnent.
  cleanup();
  clearMocks();
  vi.restoreAllMocks();
});

async function open() {
  backend(makeView());
  render(<EditorScreen />);
  return screen.findByRole("tree", { name: "Contenu de l'histoire" });
}

/** Lignes : racine (0–40), Animaux (40–80), Le chat (80–120), Le chien (120–160). */
function grip(title: string): HTMLElement {
  const row = screen.getByText(title, { selector: ".fl-editor-tree__title" });
  const found = row
    .closest("[data-dnd-row]")
    ?.querySelector<HTMLElement>("[data-dnd-grip]");
  if (!found) throw new Error(`poignée de ${title} absente`);
  return found;
}

function rowOf(title: string): HTMLElement {
  const row = screen
    .getByText(title, { selector: ".fl-editor-tree__title" })
    .closest<HTMLElement>("[data-dnd-row]");
  if (!row) throw new Error(title);
  return row;
}

function drag(from: HTMLElement, y0: number, path: number[]) {
  fireEvent.pointerDown(from, { button: 0, pointerId: 1, clientX: 10, clientY: y0 });
  for (const y of path)
    fireEvent.pointerMove(window, { pointerId: 1, clientX: 10, clientY: y });
}

const status = () => document.querySelector(".fl-editor__status");

slowIt("pointeur : réordonner dans le même menu, focus gardé sur l'élément", async () => {
  await open();
  drag(grip("Le chien"), 140, [130, 84]);
  // Aperçu et place d'insertion pendant le glissement.
  expect(document.querySelector(".fl-editor-dnd__ghost")).toHaveTextContent("Le chien");
  expect(rowOf("Le chat")).toHaveClass("fl-editor-tree__row--drop-before");
  expect(rowOf("Le chien")).toHaveClass("fl-editor-tree__row--dragging");
  await waitFor(() => expect(status()).toHaveTextContent(/Déplacement de « Le chien »/));
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 84 });
  await waitFor(() => expect(called("authoring_editor_reorder")).toHaveLength(1));
  expect(called("authoring_editor_reorder")[0]?.args).toEqual({
    menu: "Animaux",
    order: ["02 Le chien.mp3", "01 Le chat.mp3"],
  });
  expect(document.querySelector(".fl-editor-dnd__ghost")).toBeNull();
  await waitFor(() =>
    expect(status()).toHaveTextContent(
      "« Le chien » rangé à sa nouvelle place dans « Animaux ».",
    ),
  );
  const chien = screen.getByRole("treeitem", { name: /Le chien/ });
  await waitFor(() => expect(chien).toHaveFocus());
  expect(chien).toHaveAttribute("aria-selected", "true");
});

slowIt("pointeur : vers un autre menu, déplacement", async () => {
  await open();
  drag(grip("Le chat"), 100, [90, 20]);
  expect(rowOf("Le loup")).toHaveClass("fl-editor-tree__row--drop-inside");
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 20 });
  await waitFor(() => expect(called("authoring_editor_move")).toHaveLength(1));
  expect(called("authoring_editor_move")[0]?.args).toMatchObject({
    target: { id: "Animaux/01 Le chat.mp3" },
    dest: "",
    before: null,
  });
});

slowIt("pointeur : même place, sous le seuil, ou Échap : rien n'est fait", async () => {
  await open();
  // Sous le seuil : un clic, pas de glissement.
  drag(grip("Le chien"), 140, [142]);
  expect(document.querySelector(".fl-editor-dnd__ghost")).toBeNull();
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 142 });

  // Lâché à sa place.
  drag(grip("Le chien"), 140, [150, 155]);
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 155 });
  await waitFor(() =>
    expect(status()).toHaveTextContent("« Le chien » est resté à sa place."),
  );

  // Échap annule.
  drag(grip("Le chien"), 140, [130, 84]);
  fireEvent.keyDown(window, { key: "Escape" });
  expect(document.querySelector(".fl-editor-dnd__ghost")).toBeNull();
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 84 });
  await waitFor(() =>
    expect(status()).toHaveTextContent("Déplacement de « Le chien » annulé."),
  );
  expect(called("authoring_editor_reorder")).toHaveLength(0);
  expect(called("authoring_editor_move")).toHaveLength(0);
});

slowIt("pointeur : un menu ne va pas dans sa descendance", async () => {
  await open();
  drag(grip("Animaux"), 60, [70, 100]);
  expect(rowOf("Le chat")).toHaveClass("fl-editor-tree__row--drop-invalid");
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 100 });
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Un menu ne peut pas aller dans lui-même",
  );
  expect(called("authoring_editor_move")).toHaveLength(0);
});

slowIt(
  "fichiers : copie dans un menu, zone surlignée, puis à la place du dépôt",
  async () => {
    await open();
    const mp3 = "C:\\Sources\\Le loup.mp3";
    await emit({ type: "enter", paths: [mp3], position: { x: 10, y: 60 } });
    expect(document.querySelector(".fl-editor")).toHaveClass("fl-editor--files-over");
    expect(rowOf("Animaux")).toHaveClass("fl-editor-tree__row--drop-inside");
    expect(document.querySelector(".fl-editor-dnd__banner")).toHaveTextContent(
      "Copier ici, dans « Animaux »",
    );
    await emit({ type: "over", position: { x: 10, y: 123 } });
    expect(rowOf("Le chien")).toHaveClass("fl-editor-tree__row--drop-before");
    await emit({ type: "drop", paths: [mp3], position: { x: 10, y: 123 } });
    expect(document.querySelector(".fl-editor-dnd__banner")).toBeNull();
    await waitFor(() => expect(called("authoring_editor_import")).toHaveLength(1));
    expect(called("authoring_editor_import")[0]?.args).toEqual({
      menu: "Animaux",
      sources: [mp3],
      before: "02 Le chien.mp3",
    });
    await waitFor(() => expect(status()).toHaveTextContent("Copié dans « Animaux »."));

    await emit({ type: "drop", paths: [mp3], position: { x: 10, y: 60 } });
    await waitFor(() => expect(called("authoring_editor_import")).toHaveLength(2));
    expect(called("authoring_editor_import")[1]?.args).toEqual({
      menu: "Animaux",
      sources: [mp3],
    });
  },
);

slowIt("fichiers : formats refusés, message clair et rien de copié", async () => {
  await open();
  // Le backend décide (et ne copie rien) : le message le dit.
  await emit({ type: "drop", paths: ["C:\\a.txt"], position: { x: 10, y: 60 } });
  expect(await screen.findByRole("alert")).toHaveTextContent("Formats non acceptés");
  expect(called("authoring_editor_import")).toHaveLength(1);
  // Hors de toute zone : bandeau d'avertissement, aucun appel.
  await emit({ type: "enter", paths: ["C:\\a.mp3"], position: { x: 350, y: 60 } });
  expect(document.querySelector(".fl-editor-dnd__banner")).toHaveClass(
    "fl-editor-dnd__banner--refused",
  );
  await emit({ type: "drop", paths: ["C:\\a.mp3"], position: { x: 350, y: 60 } });
  expect(await screen.findByRole("alert")).toHaveTextContent("Déposez sur un menu");
  await emit({ type: "leave" });
  expect(called("authoring_editor_import")).toHaveLength(1);
});

slowIt(
  "fichiers : copie dans un menu replié, la ligne copiée devient visible",
  async () => {
    await open();
    const animaux = screen.getByRole("treeitem", { name: /Animaux/ });
    fireEvent.click(animaux.querySelector(".fl-editor-tree__twisty")!);
    expect(screen.queryByRole("treeitem", { name: /Le chat/ })).toBeNull();
    await emit({ type: "drop", paths: ["C:\\x.mp3"], position: { x: 10, y: 60 } });
    await waitFor(() => expect(called("authoring_editor_import")).toHaveLength(1));
    expect(
      await screen.findByRole("treeitem", { name: /Le chat/, selected: true }),
    ).toBeInTheDocument();
  },
);

slowIt("fichiers : refusés tant qu'un dialogue est ouvert", async () => {
  await open();
  fireEvent.click(screen.getAllByRole("button", { name: "Nouveau menu" })[0]!);
  await emit({ type: "enter", paths: ["C:\\x.mp3"], position: { x: 10, y: 60 } });
  expect(document.querySelector(".fl-editor-dnd__banner")).toBeNull();
  await emit({ type: "drop", paths: ["C:\\x.mp3"], position: { x: 10, y: 60 } });
  await waitFor(() =>
    expect(document.body).toHaveTextContent("Fermez d'abord la fenêtre ouverte"),
  );
  expect(called("authoring_editor_import")).toHaveLength(0);
});

slowIt("pointeur : relâché hors de l'arbre, rien ne bouge", async () => {
  await open();
  drag(grip("Le chien"), 140, [130, 84, 90]);
  fireEvent.pointerMove(window, { pointerId: 1, clientX: 500, clientY: 90 });
  expect(rowOf("Le chat")).not.toHaveClass("fl-editor-tree__row--drop-before");
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 500, clientY: 90 });
  await waitFor(() =>
    expect(status()).toHaveTextContent("« Le chien » est resté à sa place."),
  );
  expect(called("authoring_editor_reorder")).toHaveLength(0);
});

slowIt("fichiers : image et titre lu sur les zones du panneau", async () => {
  await open();
  const png = "C:\\Images\\loup.png";
  await emit({ type: "enter", paths: [png], position: { x: 500, y: 100 } });
  const zone = document.querySelector('[data-dnd-zone="image"]');
  expect(zone).toHaveClass("fl-editor-dnd__zone--active");
  await emit({ type: "drop", paths: [png], position: { x: 500, y: 100 } });
  await waitFor(() => expect(called("authoring_editor_set_image")).toHaveLength(1));
  expect(called("authoring_editor_set_image")[0]?.args).toMatchObject({
    target: { id: "" },
    source: png,
  });

  const mp3 = "C:\\Sons\\titre.mp3";
  await emit({ type: "drop", paths: [mp3], position: { x: 500, y: 300 } });
  await waitFor(() => expect(called("authoring_editor_set_title_audio")).toHaveLength(1));
  expect(called("authoring_editor_set_title_audio")[0]?.args).toMatchObject({
    target: { id: "" },
    source: mp3,
  });

  // Une image déposée sur une ligne : l'image de cet élément.
  await emit({ type: "drop", paths: [png], position: { x: 10, y: 100 } });
  await waitFor(() => expect(called("authoring_editor_set_image")).toHaveLength(2));
  expect(called("authoring_editor_set_image")[1]?.args).toMatchObject({
    target: { id: "Animaux/01 Le chat.mp3" },
  });
  const tree = screen.getByRole("tree");
  expect(within(tree).getAllByRole("treeitem").length).toBeGreaterThan(0);
});

/** Laisse les abonnements aux événements de la fenêtre s'installer. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

slowIt("dossier disparu : dépôts et glissements refusés", async () => {
  await open();
  await settle();
  await act(() => emitEvent(FOLDER_MISSING_EVENT));
  await waitFor(() => expect(grip("Le chien")).toHaveAttribute("data-disabled", "true"));
  await emit({ type: "drop", paths: ["C:\\x.mp3"], position: { x: 10, y: 60 } });
  await waitFor(() =>
    expect(document.body).toHaveTextContent("Le dossier de l'histoire est introuvable"),
  );
  drag(grip("Le chien"), 140, [130, 84]);
  expect(document.querySelector(".fl-editor-dnd__ghost")).toBeNull();
  fireEvent.pointerUp(window, { pointerId: 1, clientX: 10, clientY: 84 });
  expect(called("authoring_editor_import")).toHaveLength(0);
  expect(called("authoring_editor_reorder")).toHaveLength(0);
});

slowIt("relecture du dossier en attente : dépôt refusé, puis accepté", async () => {
  await open();
  await settle();
  let release = () => {};
  viewGate = new Promise((resolve) => {
    release = resolve;
  });
  await act(() => emitEvent(FOLDER_CHANGED_EVENT));
  await emit({ type: "drop", paths: ["C:\\x.mp3"], position: { x: 10, y: 60 } });
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Une opération est en cours",
  );
  expect(called("authoring_editor_import")).toHaveLength(0);
  await act(async () => release());
  await waitFor(() => expect(grip("Le chien")).toHaveAttribute("data-disabled", "false"));
  await emit({ type: "drop", paths: ["C:\\x.mp3"], position: { x: 10, y: 60 } });
  await waitFor(() => expect(called("authoring_editor_import")).toHaveLength(1));
});
