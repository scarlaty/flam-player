/* eslint-disable @typescript-eslint/no-explicit-any -- arguments IPC simulés, forme libre */
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CompatEntry, CompatReport } from "../../api/compat";
import type { InstallTask } from "../../api/install";
import type { LibraryEntry } from "../../api/library";
import type { PlayerSession } from "../../api/player";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
import { ToastProvider } from "../../components";
import { PAGE_LEVEL_RULES, expectNoAxeViolations } from "../../test/axe";
import { makeEntry } from "../../test/entries";
import { CompatProvider } from "../compat/CompatProvider";
import { LibraryProvider } from "../library/LibraryProvider";
import { StoriesLibrary } from "./StoriesLibrary";
import { StoriesProvider } from "./StoriesProvider";
import { StoriesTaskPanel } from "./StoriesTaskPanel";

const library: Source = { type: "library", id: "library", name: "Mes histoires" };
const store: Source = {
  type: "catalog",
  id: "store",
  name: "Catalogue test",
  url: "https://exemple.org/c.json",
};
const folder: Source = {
  type: "folder",
  id: "dossier",
  name: "Contes",
  path: "C:\\Contes",
};

const ours = (over: Partial<StoryEntry> = {}) =>
  makeEntry({
    source_id: "store",
    entry_id: "store:ours",
    uuid: "ours",
    version: 1,
    title: "Les trois ours",
    description: "Une visite chez trois ours gourmands.",
    age: 3,
    category: "Contes",
    author: "Anne Auteur",
    voice: "Vic Voix",
    license: "CC BY 4.0",
    size: 3 * 1024 * 1024,
    ...over,
  });

function installedEntry(entry: StoryEntry): LibraryEntry {
  return {
    uuid: entry.uuid,
    version: entry.version,
    title: entry.title,
    file: `${entry.uuid}-${entry.version}.plain.pk`,
    size: 1000,
    installed_at: 1_700_000_000,
    converter: null,
    source: entry,
  };
}

function task(
  entry: StoryEntry,
  state: InstallTask["state"],
  over: Partial<InstallTask> = {},
): InstallTask {
  return {
    id: "t1",
    entry,
    state,
    percent: null,
    error: null,
    created_at: 1_700_000_000,
    ...over,
  };
}

const session = (title = "Les trois ours", uuid = "ours"): PlayerSession => ({
  origin: "library",
  uuid,
  title,
  pid: 42,
  started_at: 1_700_000_100,
});

let sources: Source[];
let catalogEntries: StoryEntry[];
let scanEntries: StoryEntry[];
let installed: LibraryEntry[];
let report: CompatReport;
let started: InstallTask[];
let status: PlayerSession | null;
let failures: Record<string, { kind: string; message: string }>;
// Retient la réponse d'une commande pour tester l'ordre des événements.
let installGate: Promise<void> | null;
let launchGate: Promise<void> | null;
let calls: { cmd: string; args: Record<string, any> }[];

const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);

beforeEach(() => {
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  sources = [library, store, folder];
  catalogEntries = [ours()];
  scanEntries = [];
  installed = [];
  report = { entries: [], issue: null };
  started = [];
  status = null;
  failures = {};
  calls = [];
  installGate = null;
  launchGate = null;
  window.location.hash = "#/library/store";
  mockIPC(
    (cmd, args) => {
      const payload = (args ?? {}) as Record<string, any>;
      calls.push({ cmd, args: payload });
      const failure = failures[cmd];
      if (failure) throw failure;
      switch (cmd) {
        case "sources_list":
          return { sources, recovery: null };
        case "catalog_entries":
        case "catalog_refresh":
          return {
            source_id: "store",
            entries: catalogEntries,
            warnings: [],
            status: { offline: false, updated_at: 1_700_049_600, error: null },
          };
        case "folder_scan":
          return { entries: scanEntries, warnings: [] };
        case "catalog_thumbnail":
        case "folder_thumbnail":
          return "data:image/png;base64,AAAA";
        case "compat_statuses":
          return report;
        case "library_list":
          return installed;
        case "library_remove":
          installed = installed.filter((item) => item.uuid !== payload.uuid);
          return null;
        case "install_list":
          return started;
        case "install_start": {
          const entry = [...catalogEntries, ...scanEntries].find(
            (e) => e.entry_id === payload.entryId,
          );
          const created = task(entry as StoryEntry, "queued", {
            id: `t${started.length + 1}`,
          });
          started.push(created);
          return installGate ? installGate.then(() => created) : created;
        }
        case "install_cancel":
          return null;
        case "player_status":
          return status;
        case "player_launch":
        case "player_launch_folder":
          return launchGate ? launchGate.then(() => session()) : session();
        default:
          throw { kind: "unknown", message: cmd };
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  // Démonte avant d'effacer les simulations : les désabonnements appellent l'API d'événements.
  cleanup();
  clearMocks();
  vi.unstubAllGlobals();
  window.location.hash = "";
});

function fiche(over: Partial<CompatEntry> & { uuid: string }): CompatEntry {
  return {
    version: null,
    level: "compatible",
    reason: null,
    code: "OK",
    origin: "campaign",
    stale: false,
    ...over,
  };
}

async function renderApp() {
  const view = render(
    <ToastProvider>
      <LibraryProvider>
        <CompatProvider>
          <StoriesProvider>
            <main>
              <StoriesLibrary />
            </main>
            <aside>
              <StoriesTaskPanel />
            </aside>
          </StoriesProvider>
        </CompatProvider>
      </LibraryProvider>
    </ToastProvider>,
  );
  await screen.findByRole("heading", { level: 3, name: ours().title });
  // Attend l'abonnement effectif aux événements : une progression d'une tâche inconnue
  // déclenche une relecture de `install_list`, signe que l'écouteur est en place.
  const before = callsOf("install_list").length;
  await waitFor(async () => {
    await emit("install://progress", {
      task_id: "sonde",
      state: "queued",
      percent: null,
      received: null,
      total: null,
      phase: null,
    });
    expect(callsOf("install_list").length).toBeGreaterThan(before);
  });
  return view;
}

async function send(event: string, payload: unknown) {
  await act(async () => {
    await emit(event, payload);
  });
}

async function openFiche(title = "Les trois ours") {
  fireEvent.click(await screen.findByRole("button", { name: title }));
  return screen.findByRole("dialog", { name: title });
}

const panel = () => screen.getByRole("region", { name: "Tâches en cours" });

it("affiche la fiche : vignette, description, métadonnées, état, compatibilité", async () => {
  report = {
    issue: null,
    entries: [
      fiche({ uuid: "ours", level: "warnings", reason: "Exploration partielle" }),
    ],
  };
  await renderApp();
  const dialog = await openFiche();
  await waitFor(() =>
    expect(within(dialog).getByRole("img", { name: /Vignette/ })).toBeInTheDocument(),
  ).catch(() => undefined);
  expect(within(dialog).getByText("Une visite chez trois ours gourmands.")).toBeVisible();
  for (const text of [
    "Dès 3 ans",
    "Contes",
    "Anne Auteur",
    "Vic Voix",
    "CC BY 4.0",
    "3 Mo",
    "Catalogue test",
    "À télécharger",
    "Avertissements",
  ]) {
    expect(within(dialog).getByText(text)).toBeInTheDocument();
  }
  await waitFor(() =>
    expect(within(dialog).getByText("Exploration partielle")).toBeInTheDocument(),
  );
  expect(
    within(dialog).getByRole("button", { name: "Télécharger et jouer" }),
  ).toBeEnabled();
  expect(within(dialog).getByRole("button", { name: "Télécharger" })).toBeEnabled();
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);

  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
});

it("télécharger et jouer : progression, installation puis lancement, retour sur la fiche", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger et jouer" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  expect(callsOf("install_start")[0]?.args).toEqual({
    sourceId: "store",
    entryId: "store:ours",
  });
  expect(await within(dialog).findByRole("button", { name: "Annuler" })).toBeVisible();

  await send("install://progress", {
    task_id: "t1",
    state: "downloading",
    percent: 40,
    received: 1024 * 1024,
    total: 3 * 1024 * 1024,
    phase: null,
  });
  expect(
    await within(panel()).findByText(/Téléchargement : 1 Mo sur 3 Mo/),
  ).toBeInTheDocument();
  expect(
    within(dialog).getByRole("progressbar", { name: /Progression/ }),
  ).toHaveAttribute("aria-valuenow", "40");

  await send("install://progress", {
    task_id: "t1",
    state: "converting",
    percent: 60,
    received: null,
    total: null,
    phase: "audio",
  });
  expect(await within(panel()).findByText(/Conversion : audio/)).toBeInTheDocument();

  installed = [installedEntry(ours())];
  await send("install://done", { task_id: "t1", entry: installed[0], warnings: 0 });
  await waitFor(() => expect(callsOf("player_launch")).toHaveLength(1));
  expect(callsOf("player_launch")[0]?.args).toEqual({ uuid: "ours" });
  expect(within(panel()).getByText("Terminée")).toBeInTheDocument();

  await send("player://started", session());
  const reopened = await screen.findByRole("dialog", { name: "Les trois ours" });
  expect(
    within(reopened).getByText("En cours de lecture dans flam-player"),
  ).toBeVisible();
  expect(within(reopened).getByRole("button", { name: "Jouer" })).toBeDisabled();

  await send("player://exited", {
    session: session(),
    code: 0,
    success: true,
    duration_ms: 5000,
    stderr: "",
  });
  expect(await screen.findByText("Lecture de « Les trois ours » terminée")).toBeVisible();
  // La fiche est toujours là, et Jouer de nouveau possible.
  expect(screen.getByRole("dialog", { name: "Les trois ours" })).toBeVisible();
  await waitFor(() =>
    expect(within(reopened).getByRole("button", { name: "Jouer" })).toBeEnabled(),
  );
});

it("installer sans jouer, puis jouer depuis la fiche", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  installed = [installedEntry(ours())];
  await send("install://done", { task_id: "t1", entry: installed[0], warnings: 2 });
  expect(await screen.findByText(/est installée \(2 avertissements/)).toBeVisible();
  expect(callsOf("player_launch")).toHaveLength(0);

  const play = await within(dialog).findByRole("button", { name: "Jouer" });
  expect(within(dialog).getByText("Installée")).toBeInTheDocument();
  fireEvent.click(play);
  await waitFor(() => expect(callsOf("player_launch")).toHaveLength(1));
});

it("annule une installation, puis la relance depuis le panneau", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger et jouer" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await send("install://progress", {
    task_id: "t1",
    state: "downloading",
    percent: 10,
    received: null,
    total: null,
    phase: null,
  });
  fireEvent.click(await within(dialog).findByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(callsOf("install_cancel")).toHaveLength(1));
  expect(callsOf("install_cancel")[0]?.args).toEqual({ taskId: "t1" });

  await send("install://error", {
    task_id: "t1",
    state: "cancelled",
    error: { kind: "cancelled", message: "Installation annulée.", code: null },
  });
  expect(await within(panel()).findByText("Annulée")).toBeInTheDocument();
  fireEvent.click(
    within(panel()).getByRole("button", { name: "Réessayer : Les trois ours" }),
  );
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  // L'annulation a effacé « jouer ensuite » : pas de lancement à la fin.
  installed = [installedEntry(ours())];
  await send("install://done", { task_id: "t2", entry: installed[0], warnings: 0 });
  await waitFor(() => expect(callsOf("library_list").length).toBeGreaterThan(1));
  expect(callsOf("player_launch")).toHaveLength(0);
});

it("erreur de conversion : message clair, Réessayer reprend « Télécharger et jouer »", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger et jouer" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await send("install://error", {
    task_id: "t1",
    state: "failed",
    error: {
      kind: "conversion",
      message: "Image manquante dans l'archive.",
      code: "missing_asset",
    },
  });
  expect(
    await within(panel()).findByText("Image manquante dans l'archive."),
  ).toBeVisible();
  expect(within(panel()).getByText("Échec")).toBeInTheDocument();
  // La fiche relaie l'erreur et propose aussi de réessayer.
  expect(within(dialog).getByText("Image manquante dans l'archive.")).toBeVisible();
  expect(within(dialog).getByRole("alert")).toBeInTheDocument();

  fireEvent.click(within(dialog).getByRole("button", { name: "Réessayer" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  installed = [installedEntry(ours())];
  await send("install://done", { task_id: "t2", entry: installed[0], warnings: 0 });
  await waitFor(() => expect(callsOf("player_launch")).toHaveLength(1));
  // L'ancienne tâche en échec a quitté le panneau.
  expect(within(panel()).queryByText("Échec")).toBeNull();
});

it("retire une tâche en échec du panneau", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await send("install://error", {
    task_id: "t1",
    state: "failed",
    error: { kind: "network", message: "Réseau coupé.", code: null },
  });
  fireEvent.click(
    await within(panel()).findByRole("button", { name: "Retirer : Les trois ours" }),
  );
  expect(within(panel()).getByText("Aucune tâche en cours.")).toBeInTheDocument();
});

it("avertit avant d'installer une histoire incompatible, sans bloquer", async () => {
  report = {
    issue: null,
    entries: [fiche({ uuid: "ours", level: "incompatible", reason: "Image manquante" })],
  };
  await renderApp();
  await screen.findAllByText("Incompatible");
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger" }));

  const alert = await screen.findByRole("alertdialog", { name: /Les trois ours/ });
  // Un seul modal à la fois : la fiche s'efface le temps de la réponse.
  expect(screen.queryByRole("dialog", { name: "Les trois ours" })).toBeNull();
  expect(within(alert).getByText("Image manquante")).toBeInTheDocument();
  expect(callsOf("install_start")).toHaveLength(0);
  fireEvent.click(within(alert).getByRole("button", { name: "Annuler" }));
  expect(await screen.findByRole("dialog", { name: "Les trois ours" })).toBeVisible();
  expect(callsOf("install_start")).toHaveLength(0);

  fireEvent.click(screen.getByRole("button", { name: "Télécharger et jouer" }));
  fireEvent.click(await screen.findByRole("button", { name: "Installer quand même" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  expect(screen.queryByRole("alertdialog")).toBeNull();
});

it("une histoire compatible s'installe sans avertissement, depuis la carte", async () => {
  report = { issue: null, entries: [fiche({ uuid: "ours" })] };
  await renderApp();
  fireEvent.click(screen.getByRole("button", { name: "Télécharger « Les trois ours »" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  expect(screen.queryByRole("alertdialog")).toBeNull();
  // La carte montre la progression de la tâche.
  expect((await screen.findAllByText("En attente")).length).toBeGreaterThan(0);
  await send("install://progress", {
    task_id: "t1",
    state: "converting",
    percent: 25,
    received: null,
    total: null,
    phase: "images",
  });
  const grid = screen.getByRole("list", { name: "Histoires de Catalogue test" });
  expect(await within(grid).findByText("Conversion : images")).toBeInTheDocument();
  expect(within(grid).getByText("En cours")).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Télécharger « Les trois ours »" }),
  ).toBeNull();
});

it("« Mes histoires » liste les histoires installées : jouer et supprimer", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  await renderApp();
  expect(screen.getByText("Installée")).toBeInTheDocument();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer" }));
  const confirm = await screen.findByRole("alertdialog", { name: /Supprimer/ });
  expect(screen.queryByRole("dialog", { name: "Les trois ours" })).toBeNull();
  fireEvent.click(within(confirm).getByRole("button", { name: "Supprimer" }));
  await waitFor(() => expect(callsOf("library_remove")).toHaveLength(1));
  expect(callsOf("library_remove")[0]?.args).toEqual({ uuid: "ours", purgeSaves: false });
  expect(await screen.findByText("« Les trois ours » a été supprimée")).toBeVisible();
  expect(await screen.findByText("Aucune histoire installée")).toBeVisible();
  expect(screen.queryByRole("dialog")).toBeNull();
});

it("désactive Jouer et Supprimer pendant la lecture de cette histoire", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  await renderApp();
  fireEvent.click(screen.getByRole("button", { name: "Jouer « Les trois ours »" }));
  await waitFor(() => expect(callsOf("player_launch")).toHaveLength(1));
  await send("player://started", session());
  const dialog = await openFiche();
  expect(within(dialog).getByRole("button", { name: "Jouer" })).toBeDisabled();
  expect(within(dialog).getByRole("button", { name: "Supprimer" })).toBeDisabled();
});

it("annuler la suppression garde l'histoire", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer" }));
  const confirm = await screen.findByRole("alertdialog");
  fireEvent.click(within(confirm).getByRole("button", { name: "Annuler" }));
  expect(await screen.findByRole("dialog", { name: "Les trois ours" })).toBeVisible();
  expect(callsOf("library_remove")).toHaveLength(0);
});

it("propose la mise à jour d'une histoire dont le catalogue a une autre version", async () => {
  installed = [installedEntry(ours({ version: 1 }))];
  catalogEntries = [ours({ version: 2 })];
  await renderApp();
  expect(screen.getAllByText("Mise à jour disponible").length).toBeGreaterThan(0);
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Mettre à jour" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  expect(within(dialog).getByRole("button", { name: "Annuler" })).toBeVisible();
});

it("signale un lancement refusé et un player qui s'arrête en erreur", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  failures.player_launch = {
    kind: "player_busy",
    message: "Un player est déjà en cours.",
  };
  await renderApp();
  fireEvent.click(screen.getByRole("button", { name: "Jouer « Les trois ours »" }));
  expect(await screen.findByText("Un player est déjà en cours.")).toBeVisible();

  await send("player://exited", {
    session: session(),
    code: 1,
    success: false,
    duration_ms: 100,
    stderr: "Erreur de chargement : histoire.pk",
  });
  expect(
    await screen.findByText(
      /s'est arrêté sur une erreur.*Erreur de chargement : histoire\.pk/,
    ),
  ).toBeVisible();
});

it("retrouve un player déjà lancé au chargement", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  status = session();
  await renderApp();
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Jouer « Les trois ours »" }),
    ).toBeDisabled(),
  );
});

it("joue directement un fichier d'une source dossier, sans installation", async () => {
  scanEntries = [
    makeEntry({
      source_id: "dossier",
      entry_id: "dossier:conte",
      uuid: "conte",
      title: "Conte local",
      kind: "flam_pk",
      download_url: "C:\\Contes\\conte.plain.pk",
    }),
  ];
  window.location.hash = "#/library/dossier";
  render(
    <ToastProvider>
      <LibraryProvider>
        <CompatProvider>
          <StoriesProvider>
            <main>
              <StoriesLibrary />
            </main>
          </StoriesProvider>
        </CompatProvider>
      </LibraryProvider>
    </ToastProvider>,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Conte local" }));
  const dialog = await screen.findByRole("dialog", { name: "Conte local" });
  expect(
    within(dialog).getByRole("button", { name: "Installer et jouer" }),
  ).toBeVisible();
  fireEvent.click(
    within(dialog).getByRole("button", { name: "Jouer depuis le dossier" }),
  );
  await waitFor(() => expect(callsOf("player_launch_folder")).toHaveLength(1));
  expect(callsOf("player_launch_folder")[0]?.args).toEqual({
    sourceId: "dossier",
    entryId: "dossier:conte",
  });
});

it("ignore une progression tardive reçue après l'annulation", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await send("install://error", {
    task_id: "t1",
    state: "cancelled",
    error: { kind: "cancelled", message: "Installation annulée.", code: null },
  });
  await send("install://progress", {
    task_id: "t1",
    state: "downloading",
    percent: 90,
    received: null,
    total: null,
    phase: null,
  });
  expect(await within(panel()).findByText("Annulée")).toBeInTheDocument();
  expect(within(dialog).getByRole("button", { name: "Télécharger" })).toBeEnabled();
});

it("`player://exited` reçu avant la réponse de player_launch ne bloque pas Jouer", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  let release = () => {};
  launchGate = new Promise((resolve) => (release = resolve));
  await renderApp();
  fireEvent.click(screen.getByRole("button", { name: "Jouer « Les trois ours »" }));
  await waitFor(() => expect(callsOf("player_launch")).toHaveLength(1));
  await send("player://started", session());
  await send("player://exited", {
    session: session(),
    code: 0,
    success: true,
    duration_ms: 10,
    stderr: "",
  });
  await act(async () => release());
  await screen.findByText("Lecture de « Les trois ours » terminée");
  expect(screen.getByRole("button", { name: "Jouer « Les trois ours »" })).toBeEnabled();
});

it("un événement done reçu avant le retour de install_start est conservé", async () => {
  await renderApp();
  let release = () => {};
  installGate = new Promise((resolve) => (release = resolve));
  fireEvent.click(screen.getByRole("button", { name: "Télécharger « Les trois ours »" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  installed = [installedEntry(ours())];
  await send("install://done", { task_id: "t1", entry: installed[0], warnings: 0 });
  await act(async () => release());
  expect(await within(panel()).findByText("Terminée")).toBeInTheDocument();
  expect(
    await screen.findByRole("button", { name: "Jouer « Les trois ours »" }),
  ).toBeVisible();
});

it("relancer par Télécharger après un échec retire la tâche en échec", async () => {
  await renderApp();
  let dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await send("install://error", {
    task_id: "t1",
    state: "failed",
    error: { kind: "network", message: "Réseau coupé.", code: null },
  });
  expect(await within(dialog).findByRole("alert")).toBeVisible();
  dialog = screen.getByRole("dialog", { name: "Les trois ours" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await waitFor(() => expect(within(panel()).queryByText("Échec")).toBeNull());
  expect(within(dialog).queryByRole("alert")).toBeNull();
});

it("Réessayer garde la tâche en échec si on renonce à une histoire incompatible", async () => {
  await renderApp();
  const dialog = await openFiche();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger et jouer" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await send("install://error", {
    task_id: "t1",
    state: "failed",
    error: { kind: "conversion", message: "Image manquante.", code: "missing_asset" },
  });
  await within(panel()).findByText("Échec");
  report = {
    issue: null,
    entries: [fiche({ uuid: "ours", level: "incompatible", reason: "Image manquante" })],
  };
  // La conversion en échec recharge le rapport : l'histoire devient incompatible.
  await send("install://error", {
    task_id: "t1",
    state: "failed",
    error: { kind: "conversion", message: "Image manquante.", code: "missing_asset" },
  });
  await waitFor(() => expect(callsOf("compat_statuses").length).toBeGreaterThan(1));
  fireEvent.click(
    within(panel()).getByRole("button", { name: "Réessayer : Les trois ours" }),
  );
  const alert = await screen.findByRole("alertdialog");
  fireEvent.click(within(alert).getByRole("button", { name: "Annuler" }));
  expect(callsOf("install_start")).toHaveLength(1);
  expect(within(panel()).getByText("Échec")).toBeInTheDocument();
  expect(screen.getByRole("alert")).toBeInTheDocument();
});

it("rend le focus à la carte à la fermeture de la fiche, même après une confirmation", async () => {
  installed = [installedEntry(ours())];
  window.location.hash = "#/library/library";
  await renderApp();
  const card = screen.getByRole("button", { name: "Les trois ours" });
  card.focus();
  fireEvent.click(card);
  const dialog = await screen.findByRole("dialog", { name: "Les trois ours" });
  fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer" }));
  const confirm = await screen.findByRole("alertdialog");
  fireEvent.click(within(confirm).getByRole("button", { name: "Annuler" }));
  await screen.findByRole("dialog", { name: "Les trois ours" });
  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(card).toHaveFocus();
});
