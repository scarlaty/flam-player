/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { useEffect } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { InstallTask } from "../../../api/install";
import type { DraftView, PackComposition, ValidationReport } from "../../../api/pack";
import type { Episode, PodcastView } from "../../../api/podcast";
import { expectNoAxeViolations } from "../../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../../test/timeouts";
import type { Task } from "../../../components";
import { ToastContext, type ToastOptions } from "../../../components/Toast/ToastContext";
import { StoriesContext, type StoriesApi } from "../../stories/StoriesContext";
import type { SelectedEpisode } from "../selection";
import { PodcastSelectionProvider, usePodcastSelection } from "../SelectionContext";
import { resetCreations } from "./creations";
import { PackEditor } from "./PackEditor";

tolerateLoadInAsyncUtils();

vi.mock("../../../api/window", () => ({
  onWindowCloseRequested: () => Promise.resolve(() => {}),
}));

const RIGHTS_KEY = "flam-launcher.podcastPackRightsAck";
const RIGHTS_TEXT =
  "Les podcasts appartiennent à leurs auteurs. Les histoires créées ici sont réservées à un usage personnel, dans le cadre familial : ne les partagez pas et ne les redistribuez pas. L'audio est téléchargé tel que l'éditeur le diffuse, publicité comprise.";

const episode = (id: string): Episode => ({
  id,
  title: `Épisode ${id}`,
  description: null,
  pub_date: null,
  duration: 600,
  author: null,
  enclosure_type: "audio/mpeg",
  enclosure_length: 10_000_000,
  has_image: false,
  explicit: false,
});

const composition: PackComposition = {
  metadata: { title: "Contes", subtitle: null, age: 0, description: null },
  cover: { kind: "default" },
  menu: [{ kind: "episode", source_id: "a", episode_id: "1" }],
};

const okReport: ValidationReport = {
  valid: true,
  problems: [],
  unavailable: [],
  episodes: 1,
  feeds: 1,
  estimated_bytes: 10_000_000,
  unknown_size: 0,
  metadata: null,
  cover: null,
};

const task: InstallTask = {
  id: "t1",
  entry: { title: "Contes" } as InstallTask["entry"],
  state: "queued",
  percent: null,
  error: null,
  created_at: 1,
  pack: { episodes: 1, dropped: [] },
};

let calls: { cmd: string; args: Record<string, any> }[];
let draft: DraftView;
let created: (() => unknown) | null;
let tracked: InstallTask[];
let toasts: ToastOptions[];
let panelTasks: Task[];

const picked = (id: string): SelectedEpisode => ({
  id,
  sourceId: "a",
  sourceName: "Contes",
  title: `Épisode ${id}`,
  bytes: 10_000_000,
  explicit: false,
});

/** Remplit la sélection au montage et en affiche les identifiants. */
function SelectionProbe({ ids }: { ids: string[] }) {
  const selection = usePodcastSelection();
  const { addMany } = selection;
  useEffect(() => addMany(ids.map(picked)), [addMany, ids]);
  return <p data-testid="selection">{selection.selected.map((s) => s.id).join(",")}</p>;
}

beforeEach(() => {
  resetCreations();
  toasts = [];
  panelTasks = [];
  calls = [];
  tracked = [];
  created = null;
  window.localStorage.clear();
  window.location.hash = "#/podcasts/pack";
  draft = { composition, revision: 4, notice: null, report: okReport };
  const view: PodcastView = {
    source_id: "a",
    feed: {
      title: "Contes",
      description: null,
      copyright: null,
      has_image: false,
      episodes: [episode("1")],
      warnings: [],
    },
    status: { offline: false, updated_at: 1, error: null },
  };
  mockIPC((cmd, args) => {
    calls.push({ cmd, args: (args ?? {}) as Record<string, any> });
    switch (cmd) {
      case "pack_draft_read":
        return draft;
      case "pack_draft_save":
        return { saved: true, revision: (args as any).revision };
      case "pack_validate":
        return okReport;
      case "podcast_feed":
        return view;
      case "pack_create":
        return created ? created() : { task, report: okReport };
    }
    throw new Error(`commande inattendue : ${cmd}`);
  });
});

afterEach(() => clearMocks());

const NO_IDS: string[] = [];

function editorTree(ids: string[] = NO_IDS) {
  const stories = {
    trackTask: (t: InstallTask) => tracked.push(t),
    panelTasks,
  } as unknown as StoriesApi;
  return (
    <ToastContext.Provider
      value={{
        show: (options) => toasts.push(options),
        dismiss: () => {},
      }}
    >
      <StoriesContext.Provider value={stories}>
        <PodcastSelectionProvider>
          <SelectionProbe ids={ids} />
          <PackEditor saveDelayMs={20} validateDelayMs={20} />
        </PodcastSelectionProvider>
      </StoriesContext.Provider>
    </ToastContext.Provider>
  );
}

function renderEditor(ids: string[] = NO_IDS) {
  return render(editorTree(ids));
}

/**
 * Deux clics dans un même `act` : React n'a pas encore rendu le bouton désactivé au second, c'est
 * donc le verrou (`useRef`) qui doit l'arrêter. Passe par de vrais événements, sans toucher aux
 * propriétés internes de React.
 */
function clickTwice(element: HTMLElement) {
  act(() => {
    fireEvent.click(element);
    fireEvent.click(element);
  });
}

async function readyButton() {
  const button = await screen.findByRole("button", { name: "Créer l'histoire" });
  await waitFor(() => expect(button).toBeEnabled());
  return button;
}
const createCalls = () => calls.filter((c) => c.cmd === "pack_create");

it("première création : le message de droits passe d'abord, « J'ai compris » crée et mémorise", async () => {
  renderEditor();
  fireEvent.click(await readyButton());
  const dialog = await screen.findByRole("dialog", {
    name: "Avant de créer votre premier pack",
  });
  expect(within(dialog).getByText(RIGHTS_TEXT)).toBeInTheDocument();
  expect(createCalls()).toHaveLength(0);
  fireEvent.click(within(dialog).getByRole("button", { name: "J'ai compris" }));
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  expect(createCalls()[0]!.args).toMatchObject({ draftRevision: 4 });
  expect(createCalls()[0]!.args.composition.metadata.title).toBe("Contes");
  expect(window.localStorage.getItem(RIGHTS_KEY)).toBe("1");
  await waitFor(() => expect(tracked).toEqual([task]));
  expect(window.location.hash).toBe("#/library");
});

it("Échap ferme le message de droits sans créer ni mémoriser", async () => {
  renderEditor();
  fireEvent.click(await readyButton());
  await screen.findByRole("dialog");
  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(createCalls()).toHaveLength(0);
  expect(window.localStorage.getItem(RIGHTS_KEY)).toBeNull();
  expect(window.location.hash).toBe("#/podcasts/pack");
});

it(
  "le message de droits est accessible et garde le focus",
  async () => {
    renderEditor();
    fireEvent.click(await readyButton());
    const dialog = await screen.findByRole("dialog");
    await expectNoAxeViolations(dialog);
    // jsdom ne déplace pas le focus avec Tab (et `user-event` n'est pas installé) : on teste
    // la logique du piège, aux deux extrémités du dialogue.
    const buttons = within(dialog).getAllByRole("button");
    const first = buttons[0]!;
    const last = within(dialog).getByRole("button", { name: "J'ai compris" });
    expect(buttons[buttons.length - 1]).toBe(last);
    last.focus();
    fireEvent.keyDown(last, { key: "Tab" });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(last);
    // Au milieu, le navigateur gère : le piège n'intervient pas.
    const cancel = within(dialog).getByRole("button", { name: "Annuler" });
    cancel.focus();
    fireEvent.keyDown(cancel, { key: "Tab" });
    expect(document.activeElement).toBe(cancel);
  },
  SLOW_TEST_MS,
);

it("message déjà acquitté : création directe, sans dialogue", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  renderEditor();
  fireEvent.click(await readyButton());
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("le bouton est désactivé pendant l'envoi", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  let release!: (value: unknown) => void;
  created = () => new Promise((resolve) => (release = resolve));
  renderEditor();
  fireEvent.click(await readyButton());
  expect(
    await screen.findByRole("button", { name: "Création en cours…" }),
  ).toBeDisabled();
  release({ task, report: okReport });
  await waitFor(() => expect(tracked).toHaveLength(1));
});

it("verrou : le gestionnaire appelé pendant l'envoi ne relance pas la création", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  let release!: (value: unknown) => void;
  created = () => new Promise((resolve) => (release = resolve));
  renderEditor();
  const button = await readyButton();
  clickTwice(button);
  await screen.findByRole("button", { name: "Création en cours…" });
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  await act(async () => {});
  expect(createCalls()).toHaveLength(1);
  release({ task, report: okReport });
  await waitFor(() => expect(tracked).toHaveLength(1));
  expect(createCalls()).toHaveLength(1);
});

it("verrou : « J'ai compris » appelé deux fois ne crée qu'une fois", async () => {
  let release!: (value: unknown) => void;
  created = () => new Promise((resolve) => (release = resolve));
  renderEditor();
  fireEvent.click(await readyButton());
  const dialog = await screen.findByRole("dialog");
  clickTwice(within(dialog).getByRole("button", { name: "J'ai compris" }));
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  await act(async () => {});
  expect(createCalls()).toHaveLength(1);
  release({ task, report: okReport });
  await waitFor(() => expect(tracked).toHaveLength(1));
});

it("après création : notification de démarrage, seuls les épisodes du pack quittent la sélection", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  renderEditor(["1", "2"]);
  await waitFor(() => expect(screen.getByTestId("selection")).toHaveTextContent("1,2"));
  fireEvent.click(await readyButton());
  await waitFor(() => expect(tracked).toHaveLength(1));
  expect(toasts).toEqual([
    {
      message:
        "La création de « Contes » a démarré : suivez-la dans le panneau des tâches.",
      tone: "success",
    },
  ]);
  // L'épisode 2 était coché mais pas placé dans le pack : il reste coché.
  expect(screen.getByTestId("selection")).toHaveTextContent(/^2$/);
});

it("double création : éditeur rouvert pendant la tâche, la création est bloquée", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  const first = renderEditor();
  fireEvent.click(await readyButton());
  await waitFor(() => expect(tracked).toHaveLength(1));
  first.unmount();
  panelTasks = [{ id: "t1", title: "Contes", kind: "download", state: "running" }];
  const again = renderEditor();
  const button = await screen.findByRole("button", { name: "Créer l'histoire" });
  await screen.findByText(/La création de ce pack est déjà en cours/);
  expect(button).toBeDisabled();
  expect(createCalls()).toHaveLength(1);
  // Tâche en échec, brouillon conservé : on peut relancer.
  panelTasks = [{ id: "t1", title: "Contes", kind: "download", state: "error" }];
  again.rerender(editorTree());
  await waitFor(() => expect(button).toBeEnabled());
  expect(screen.queryByText(/La création de ce pack est déjà en cours/)).toBeNull();
});

it("double création : modifier le pack débloque la création d'un autre pack", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  const first = renderEditor();
  fireEvent.click(await readyButton());
  await waitFor(() => expect(tracked).toHaveLength(1));
  first.unmount();
  panelTasks = [{ id: "t1", title: "Contes", kind: "download", state: "queued" }];
  renderEditor();
  const button = await screen.findByRole("button", { name: "Créer l'histoire" });
  await screen.findByText(/La création de ce pack est déjà en cours/);
  fireEvent.change(screen.getByLabelText("Titre"), { target: { value: "Autre pack" } });
  await waitFor(() => expect(button).toBeEnabled());
});

it("double création : l'envoi de l'éditeur quitté échoue, l'éditeur rouvert se débloque", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  let fail!: () => void;
  created = () =>
    new Promise(
      (_, reject) => (fail = () => reject({ kind: "io", message: "disque plein" })),
    );
  const first = renderEditor();
  fireEvent.click(await readyButton());
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  first.unmount();
  renderEditor();
  await screen.findByText(/La création de ce pack est déjà en cours/);
  fail();
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Créer l'histoire" })).toBeEnabled(),
  );
});

it("double création : éditeur rouvert pendant l'envoi, la création est bloquée", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  let release!: (value: unknown) => void;
  created = () => new Promise((resolve) => (release = resolve));
  const first = renderEditor();
  fireEvent.click(await readyButton());
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  first.unmount();
  renderEditor();
  await screen.findByText(/La création de ce pack est déjà en cours/);
  expect(screen.getByRole("button", { name: "Créer l'histoire" })).toBeDisabled();
  release({ task, report: okReport });
  await waitFor(() => expect(tracked).toHaveLength(1));
  expect(createCalls()).toHaveLength(1);
});

it("un rapport sans tâche affiche les problèmes et ne redirige pas", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  created = () => ({
    task: null,
    report: {
      ...okReport,
      valid: false,
      problems: [
        {
          code: "episode_unavailable",
          message: "indisponible",
          path: null,
          episode: null,
          field: null,
        },
      ],
    },
  });
  renderEditor();
  fireEvent.click(await readyButton());
  expect(await screen.findByText(/Le pack n'a pas été créé/)).toBeInTheDocument();
  expect(
    screen.getByText("Un épisode n'est plus disponible : retirez-le du pack."),
  ).toBeInTheDocument();
  expect(tracked).toHaveLength(0);
  expect(window.location.hash).toBe("#/podcasts/pack");
  await readyButton();
});

it("une erreur du backend est affichée et le bouton redevient actif", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  created = () => {
    throw { kind: "io", message: "disque plein" };
  };
  renderEditor();
  fireEvent.click(await readyButton());
  expect(await screen.findByText("disque plein")).toBeInTheDocument();
  await readyButton();
});

it("création désactivée tant que la validation signale des problèmes", async () => {
  draft = {
    ...draft,
    report: {
      ...okReport,
      valid: false,
      problems: [
        { code: "no_episode", message: "x", path: null, episode: null, field: null },
      ],
    },
  };
  renderEditor();
  const button = await screen.findByRole("button", { name: "Créer l'histoire" });
  expect(button).toBeDisabled();
});

it("une modification récente est enregistrée avant la création, avec sa révision", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  renderEditor();
  const button = await readyButton();
  fireEvent.change(screen.getByLabelText("Titre"), {
    target: { value: "Nouveau titre" },
  });
  // La validation de la nouvelle composition est en attente : attendre qu'elle soit prête.
  await waitFor(() => expect(button).toBeEnabled());
  fireEvent.click(button);
  await waitFor(() => expect(createCalls()).toHaveLength(1));
  const order = calls.map((c) => c.cmd);
  expect(order.lastIndexOf("pack_draft_save")).toBeLessThan(order.indexOf("pack_create"));
  expect(createCalls()[0]!.args.draftRevision).toBe(5);
  expect(createCalls()[0]!.args.composition.metadata.title).toBe("Nouveau titre");
});
