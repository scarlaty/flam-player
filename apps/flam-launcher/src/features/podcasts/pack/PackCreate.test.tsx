/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { InstallTask } from "../../../api/install";
import type { DraftView, PackComposition, ValidationReport } from "../../../api/pack";
import type { Episode, PodcastView } from "../../../api/podcast";
import { expectNoAxeViolations } from "../../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../../test/timeouts";
import { StoriesContext, type StoriesApi } from "../../stories/StoriesContext";
import { PodcastSelectionProvider } from "../SelectionContext";
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

beforeEach(() => {
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

function renderEditor() {
  const stories = {
    trackTask: (t: InstallTask) => tracked.push(t),
  } as unknown as StoriesApi;
  return render(
    <StoriesContext.Provider value={stories}>
      <PodcastSelectionProvider>
        <PackEditor saveDelayMs={20} validateDelayMs={20} />
      </PodcastSelectionProvider>
    </StoriesContext.Provider>,
  );
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
    const accept = within(dialog).getByRole("button", { name: "J'ai compris" });
    accept.focus();
    fireEvent.keyDown(dialog, { key: "Tab" });
    expect(dialog.contains(document.activeElement)).toBe(true);
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

it("le bouton est désactivé pendant l'envoi : un seul appel", async () => {
  window.localStorage.setItem(RIGHTS_KEY, "1");
  let release!: (value: unknown) => void;
  const gate = new Promise((resolve) => (release = resolve));
  created = () => gate;
  renderEditor();
  fireEvent.click(await readyButton());
  const busy = await screen.findByRole("button", { name: "Création en cours…" });
  expect(busy).toBeDisabled();
  fireEvent.click(busy);
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
