import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { afterEach, beforeEach, expect } from "vitest";
import type { EditorNode, EditorUpdate, EditorView, Stamp } from "../../../api/editor";
import { expectNoAxeViolations } from "../../../test/axe";
import { slowIt, tolerateLoadInAsyncUtils } from "../../../test/timeouts";
import { EditorScreen } from "./EditorScreen";
import { clearThumbnailCache } from "./Thumbnail";

tolerateLoadInAsyncUtils();

const stamp = (len: number): Stamp => ({ dir: false, len, modifiedMs: 1, modifiedNs: 0 });
const dirStamp: Stamp = { dir: true, len: 0, modifiedMs: 0, modifiedNs: 0 };

function makeView(patch: Partial<EditorView> = {}, root?: EditorNode): EditorView {
  const tree: EditorNode = root ?? {
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
            image: "Animaux/01 Le chat.png",
            title_audio: null,
            bytes: 2048,
          },
          {
            kind: "story",
            id: "Animaux/02 Le chien.mp3",
            file: "02 Le chien.mp3",
            title: "Le chien",
            image: null,
            title_audio: "Animaux/02 Le chien.titre.mp3",
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
      root: tree,
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
      "Animaux/01 Le chat.png": stamp(10),
      "Animaux/02 Le chien.mp3": stamp(4096),
      "Animaux/02 Le chien.titre.mp3": stamp(20),
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
    ...patch,
  };
}

type Call = { cmd: string; args: Record<string, unknown> };
let calls: Call[] = [];

/** Backend simulé : `view` rendue par chaque commande, `fail` pour une commande en échec. */
function backend(
  view: EditorView,
  fail: Record<string, { kind: string; message: string }> = {},
) {
  calls = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
    if (fail[cmd]) throw fail[cmd];
    if (cmd === "authoring_editor_view") return view;
    if (cmd === "authoring_editor_thumbnail") return "data:image/png;base64,AA==";
    if (cmd.startsWith("authoring_editor_")) {
      const update: EditorUpdate = {
        label: "Opération",
        ids: [],
        skipped: [],
        trashed: null,
        view,
      };
      return update;
    }
    return null;
  });
}

const called = (cmd: string) => calls.filter((c) => c.cmd === cmd);

beforeEach(() => clearThumbnailCache());
afterEach(() => clearMocks());

async function open(view = makeView()) {
  backend(view);
  render(<EditorScreen />);
  return screen.findByRole("tree", { name: "Contenu de l'histoire" });
}

slowIt(
  "montre l'arbre, l'en-tête et le rapport, sans défaut d'accessibilité",
  async () => {
    const tree = await open(
      makeView({
        report: {
          ...makeView().report,
          problems: [
            {
              code: "orphan_image",
              severity: "warning",
              path: "Animaux/03 La souris.png",
              message: "technique",
            },
          ],
        },
      }),
    );
    expect(
      screen.getByRole("heading", { level: 1, name: "Le loup" }),
    ).toBeInTheDocument();
    expect(within(tree).getByRole("treeitem", { name: /Le chat/ })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Créer l'histoire" })).toBeEnabled();
    // Aucun identifiant technique à l'écran.
    expect(screen.queryByText(/technique/)).toBeNull();
    await expectNoAxeViolations(document.body);
  },
);

slowIt("une ligne du rapport mène à l'élément", async () => {
  await open(
    makeView({
      report: {
        ...makeView().report,
        problems: [
          {
            code: "title_too_long",
            severity: "warning",
            path: "Animaux/02 Le chien.mp3",
            message: "",
          },
        ],
      },
    }),
  );
  fireEvent.click(screen.getByRole("button", { name: /Titre « 02 Le chien » coupé/ }));
  expect(
    await screen.findByRole("heading", { level: 2, name: "Le chien" }),
  ).toBeInTheDocument();
});

slowIt("clavier : flèches pour parcourir, Alt+Flèche pour réordonner", async () => {
  const tree = await open();
  const rootItem = within(tree).getAllByRole("treeitem")[0]!;
  rootItem.focus();
  fireEvent.keyDown(rootItem, { key: "ArrowDown" });
  fireEvent.keyDown(rootItem, { key: "ArrowDown" });
  const chat = await screen.findByRole("treeitem", { name: /Le chat/, selected: true });
  await waitFor(() => expect(chat).toHaveFocus());
  fireEvent.keyDown(chat, { key: "ArrowDown", altKey: true });
  await waitFor(() => expect(called("authoring_editor_reorder")).toHaveLength(1));
  expect(called("authoring_editor_reorder")[0]?.args).toEqual({
    menu: "Animaux",
    order: ["02 Le chien.mp3", "01 Le chat.mp3"],
  });
});

slowIt("renommer envoie l'id et l'état vu, jamais un chemin", async () => {
  const tree = await open();
  fireEvent.click(within(tree).getByRole("treeitem", { name: /Le chat/ }));
  const field = await screen.findByLabelText("Titre affiché");
  fireEvent.change(field, { target: { value: "Le gros chat" } });
  fireEvent.click(screen.getByRole("button", { name: "Enregistrer le titre" }));
  await waitFor(() => expect(called("authoring_editor_rename")).toHaveLength(1));
  expect(called("authoring_editor_rename")[0]?.args).toEqual({
    target: { id: "Animaux/01 Le chat.mp3", stamp: stamp(2048) },
    title: "Le gros chat",
    renameFile: false,
  });
  expect(await screen.findByText("Fait : Opération.")).toBeInTheDocument();
});

slowIt("supprimer demande confirmation puis met à la corbeille", async () => {
  const tree = await open();
  const chat = within(tree).getByRole("treeitem", { name: /Le chat/ });
  fireEvent.click(chat);
  fireEvent.keyDown(chat, { key: "Delete" });
  const dialog = await screen.findByRole("alertdialog");
  expect(called("authoring_editor_delete")).toHaveLength(0);
  fireEvent.click(within(dialog).getByRole("button", { name: "Mettre à la corbeille" }));
  await waitFor(() => expect(called("authoring_editor_delete")).toHaveLength(1));
  expect(called("authoring_editor_delete")[0]?.args).toEqual({
    target: { id: "Animaux/01 Le chat.mp3", stamp: stamp(2048) },
  });
});

slowIt("Ctrl+Z annule hors des champs, pas dans un champ de saisie", async () => {
  await open(
    makeView({
      journal: {
        can_undo: true,
        can_redo: false,
        undo_label: "Nouveau menu",
        redo_label: null,
      },
    }),
  );
  const field = screen.getByLabelText("Titre de l'histoire");
  fireEvent.keyDown(field, { key: "z", ctrlKey: true });
  expect(called("authoring_editor_undo")).toHaveLength(0);
  fireEvent.keyDown(document.body, { key: "z", ctrlKey: true });
  await waitFor(() => expect(called("authoring_editor_undo")).toHaveLength(1));
  expect(await screen.findByText("Annulé : Opération.")).toBeInTheDocument();
  // Rien à rétablir : Ctrl+Y ne fait rien.
  fireEvent.keyDown(document.body, { key: "y", ctrlKey: true });
  expect(called("authoring_editor_redo")).toHaveLength(0);
});

slowIt("dossier changé ailleurs : message clair et vue relue", async () => {
  const view = makeView();
  backend(view, {
    authoring_editor_new_menu: { kind: "changed", message: "« Animaux » a changé" },
  });
  render(<EditorScreen />);
  fireEvent.click(await screen.findByRole("button", { name: "Nouveau menu" }));
  const dialog = await screen.findByRole("dialog");
  fireEvent.change(within(dialog).getByLabelText("Nom du nouveau menu"), {
    target: { value: "Contes" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Créer" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Le dossier a changé entre-temps",
  );
  expect(called("authoring_editor_new_menu")[0]?.args).toEqual({
    parent: "",
    name: "Contes",
  });
  expect(called("authoring_editor_view").length).toBeGreaterThanOrEqual(2);
});

slowIt(
  "un dossier à corriger ne peut pas être créé ; sinon la création est lancée",
  async () => {
    await open(
      makeView({
        report: {
          ...makeView().report,
          blocked: true,
          problems: [{ code: "no_story", severity: "blocking", path: "", message: "" }],
        },
      }),
    );
    expect(screen.getByRole("button", { name: "Créer l'histoire" })).toBeDisabled();
    expect(screen.getByText("À corriger avant de créer l'histoire")).toBeInTheDocument();
  },
);

slowIt("mettre à jour une histoire déjà créée", async () => {
  const view = makeView({
    report: { ...makeView().report, uuid: "u", installed_version: 2 },
  });
  calls = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
    if (cmd === "authoring_editor_view") return view;
    if (cmd === "authoring_editor_create") return { created: {}, view };
    return null;
  });
  render(<EditorScreen />);
  fireEvent.click(
    await screen.findByRole("button", { name: "Mettre à jour l'histoire" }),
  );
  await waitFor(() => expect(called("authoring_editor_create")).toHaveLength(1));
  expect(called("authoring_editor_create")[0]?.args).toEqual({ choice: null });
  expect(await screen.findByText(/Création lancée/)).toBeInTheDocument();
});

slowIt(
  "ajouter des fichiers : sélecteur natif puis copie dans le menu choisi",
  async () => {
    const view = makeView();
    calls = [];
    mockIPC((cmd, args) => {
      calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
      if (cmd === "authoring_editor_view") return view;
      if (cmd === "plugin:dialog|open")
        return ["C:\\Musique\\a.mp3", "C:\\Musique\\b.mp3"];
      if (cmd === "authoring_editor_import")
        return {
          label: "Ajout",
          ids: ["Animaux/a.mp3"],
          skipped: [{ path: "x", reason: "r" }],
          trashed: null,
          view,
        };
      return null;
    });
    render(<EditorScreen />);
    const tree = await screen.findByRole("tree");
    fireEvent.click(within(tree).getByRole("treeitem", { name: /Le chat/ }));
    fireEvent.click(screen.getByRole("button", { name: "Ajouter des fichiers" }));
    await waitFor(() => expect(called("authoring_editor_import")).toHaveLength(1));
    expect(called("authoring_editor_import")[0]?.args).toEqual({
      menu: "Animaux",
      sources: ["C:\\Musique\\a.mp3", "C:\\Musique\\b.mp3"],
    });
    expect(await screen.findByText(/1 fichier n'a pas été copié/)).toBeInTheDocument();
  },
);

slowIt("choisir une image garde la sélection sur l'histoire", async () => {
  const view = makeView();
  // Vue relue : l'histoire a maintenant son image.
  const after = JSON.parse(JSON.stringify(view)) as EditorView;
  const animaux =
    after.report.root.kind === "menu" ? after.report.root.children[0] : null;
  const chien = animaux?.kind === "menu" ? animaux.children[1] : null;
  if (chien) chien.image = "Animaux/02 Le chien.png";
  calls = [];
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, unknown> });
    if (cmd === "authoring_editor_view") return view;
    if (cmd === "plugin:dialog|open") return "C:\\Images\\chien.png";
    if (cmd === "authoring_editor_set_image")
      return {
        label: "Image",
        ids: ["Animaux/02 Le chien.png"],
        skipped: [],
        trashed: null,
        view: after,
      };
    return null;
  });
  render(<EditorScreen />);
  const tree = await screen.findByRole("tree");
  fireEvent.click(within(tree).getByRole("treeitem", { name: /Le chien/ }));
  fireEvent.click(await screen.findByRole("button", { name: "Choisir une image…" }));
  await waitFor(() => expect(called("authoring_editor_set_image")).toHaveLength(1));
  expect(await screen.findByText("Fait : Image.")).toBeInTheDocument();
  expect(screen.getByRole("heading", { level: 2, name: "Le chien" })).toBeInTheDocument();
});

slowIt("les réglages du dossier ne s'écrivent qu'avec « Appliquer »", async () => {
  const tree = await open();
  fireEvent.click(within(tree).getByRole("treeitem", { name: /Animaux/ }));
  fireEvent.click(await screen.findByLabelText(/Livre à chapitres/));
  fireEvent.click(screen.getByLabelText(/Annoncer le titre/));
  expect(called("authoring_editor_folder_settings")).toHaveLength(0);
  fireEvent.click(screen.getByRole("button", { name: "Appliquer les réglages" }));
  await waitFor(() => expect(called("authoring_editor_folder_settings")).toHaveLength(1));
  expect(called("authoring_editor_folder_settings")[0]?.args).toEqual({
    menu: "Animaux",
    patch: { mode: "chapitres", annoncer: true },
  });
});
