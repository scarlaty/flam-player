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
import { useEffect, useMemo, useState, type ReactNode } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { CompatReport } from "../../api/compat";
import type { DeviceStatus, DeviceStory, LibraryCopy } from "../../api/device";
import type { InstallTask } from "../../api/install";
import type { LibraryEntry } from "../../api/library";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
import { ToastProvider } from "../../components";
import { PAGE_LEVEL_RULES, expectNoAxeViolations } from "../../test/axe";
import { makeEntry } from "../../test/entries";
import { firmwareV1, firmwareV2 } from "../../test/firmware";
import { slowIt, tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { CompatProvider } from "../compat/CompatProvider";
import { DeviceProvider } from "../device/DeviceProvider";
import { TransferPanel } from "../device/TransferPanel";
import { LibraryProvider } from "../library/LibraryProvider";
import { useStories } from "./StoriesContext";
import { StoriesLibrary } from "./StoriesLibrary";
import { StoriesProvider } from "./StoriesProvider";
import { StoriesTaskPanel } from "./StoriesTaskPanel";

// « Installer et envoyer » (#609) : l'installation habituelle, puis l'envoi par la file d'envoi,
// seulement sur la Flam branchée au moment du clic.

tolerateLoadInAsyncUtils();

const MB = 1024 * 1024;

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

const ours = makeEntry({
  source_id: "store",
  entry_id: "store:ours",
  uuid: "ours",
  version: 1,
  title: "Les trois ours",
  size: 3 * MB,
});
const capi = makeEntry({
  source_id: "store",
  entry_id: "store:capi",
  uuid: "capi",
  version: 1,
  title: "Capi",
  size: 3 * MB,
});
const pack = makeEntry({
  source_id: "dossier",
  entry_id: "dossier:pack",
  uuid: "pack",
  version: 1,
  title: "Pack STUdio",
  size: 3 * MB,
  kind: "studio_zip",
  download_url: "C:\\Contes\\pack.zip",
});

function installedEntry(entry: StoryEntry): LibraryEntry {
  return {
    uuid: entry.uuid,
    version: entry.version,
    title: entry.title,
    file: `${entry.uuid}-${entry.version}.${entry.kind === "studio_zip" ? "lunii" : "plain"}.pk`,
    format: entry.kind === "studio_zip" ? "lunii" : "flam",
    size: 2 * MB,
    installed_at: 1_700_000_000,
    converter: null,
    source: entry,
  };
}

function detected(over: Partial<NonNullable<DeviceStatus["device"]>> = {}): DeviceStatus {
  return {
    detected: true,
    hint: null,
    device_count: 1,
    device: {
      root: "E:\\",
      volume_label: "FLAM",
      file_system: "FAT32",
      space: { free: 500 * MB, total: 4096 * MB },
      warnings: [],
      mdf_problem: null,
      backup_failed: false,
      firmware: "2.4.2",
      firmware_status: firmwareV2(),
      ...over,
    },
  };
}
const absent: DeviceStatus = {
  detected: false,
  hint: null,
  device: null,
  device_count: 0,
};

function onFlam(uuid: string): DeviceStory {
  return {
    uuid,
    short_id: uuid.slice(0, 8),
    placement: "visible",
    title: null,
    in_library: false,
    missing: false,
    library_uuids: [],
  } as unknown as DeviceStory;
}

let catalogEntries: StoryEntry[];
let scanEntries: StoryEntry[];
let installed: LibraryEntry[];
let report: CompatReport;
let started: InstallTask[];
let status: DeviceStatus;
let deviceStories: DeviceStory[];
let libraryCopy: LibraryCopy | null;
let sendGate: Promise<void> | null;
// Issue forcée d'un envoi, par histoire : erreur du backend, ou champs ajoutés au rapport.
let sendOutcomes: Map<
  string,
  { error: { kind: string; message: string } } | { report: object }
>;
// L'histoire envoyée figure ensuite dans la liste de la Flam.
let sendAddsToFlam: boolean;
// Retient les réponses de `device_status`.
let statusGate: Promise<void> | null;
// Retient la prochaine réponse de `library_list`.
let listGate: Promise<void> | null;
// Fenêtre masquée (`document.hidden`) et lecture incrustée (scrutation en pause).
let windowHidden: boolean;
const PAUSE_EVENT = "test:pause";
const setPaused = (paused: boolean) =>
  window.dispatchEvent(new CustomEvent(PAUSE_EVENT, { detail: paused }));
let calls: { cmd: string; args: Record<string, any> }[];

const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);
const sent = () => callsOf("device_send_story").map((c) => c.args.uuid as string);

beforeEach(() => {
  class FakeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  }
  vi.stubGlobal("IntersectionObserver", FakeObserver);
  catalogEntries = [ours, capi];
  scanEntries = [pack];
  installed = [];
  report = { entries: [], issue: null };
  started = [];
  status = detected();
  deviceStories = [];
  libraryCopy = { status: "kept" };
  sendGate = null;
  sendOutcomes = new Map();
  sendAddsToFlam = false;
  listGate = null;
  statusGate = null;
  windowHidden = false;
  Object.defineProperty(document, "hidden", {
    configurable: true,
    get: () => windowHidden,
  });
  calls = [];
  window.localStorage.clear();
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  window.location.hash = "#/library/store";
  mockIPC(
    (cmd, args) => {
      const payload = (args ?? {}) as Record<string, any>;
      calls.push({ cmd, args: payload });
      switch (cmd) {
        case "sources_list":
          return { sources: [library, store, folder], recovery: null };
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
        case "library_list": {
          const snapshot = installed;
          const gate = listGate;
          listGate = null;
          return gate ? gate.then(() => snapshot) : snapshot;
        }
        case "library_remove":
          installed = installed.filter((item) => item.uuid !== payload.uuid);
          return null;
        case "library_source_status":
          return [];
        case "install_list":
          return started;
        case "install_start": {
          const entry = [...catalogEntries, ...scanEntries].find(
            (e) => e.entry_id === payload.entryId,
          ) as StoryEntry;
          const created: InstallTask = {
            id: `t-${entry.uuid}`,
            entry,
            state: "queued",
            percent: null,
            error: null,
            created_at: 1_700_000_000,
            pack: null,
          };
          started.push(created);
          return created;
        }
        case "install_cancel":
          return null;
        case "player_status":
          return null;
        case "device_status":
          return statusGate ? statusGate.then(() => status) : status;
        case "device_recover_transfers":
          return {
            incomplete_removed: 0,
            previous_restored: 0,
            updates_finished: 0,
            left_alone: 0,
          };
        case "device_list_stories":
          return {
            stories: deviceStories,
            visible_index: "valid",
            hidden_index: "valid",
            str_unreadable: false,
            truncated: false,
            limit: 5000,
          };
        case "device_carrier_state":
          return {
            warning_text: "Avertissement du mode carrier",
            warning_version: 1,
            consent: null,
            firmware: "1.15.14",
            carrier_needed: true,
            stored_keys: false,
            candidates: [],
          };
        case "device_send_story":
          return (sendGate ?? Promise.resolve()).then(() => {
            const forced = sendOutcomes.get(payload.uuid);
            if (forced && "error" in forced) throw forced.error;
            if (sendAddsToFlam) deviceStories = [...deviceStories, onFlam(payload.uuid)];
            return {
              ...(forced?.report ?? {}),
              id: payload.uuid,
              bytes: 1,
              files: 1,
              replaced: false,
              index_updated: true,
              cache_removed: true,
              carrier_readback: null,
              carrier_stored: null,
              recovery_failures: [],
              library_copy: libraryCopy,
            };
          });
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
  window.localStorage.clear();
  window.location.hash = "";
  delete (document as { hidden?: boolean }).hidden;
});

/** Comme `DeviceBoundary` : bibliothèque connue du `DeviceProvider`, pause de la scrutation. */
function Boundary({ children }: { children: ReactNode }) {
  const { installed: stories, installedRevision } = useStories();
  const [paused, setPausedState] = useState(false);
  useEffect(() => {
    const onPause = (event: Event) =>
      setPausedState((event as CustomEvent).detail === true);
    window.addEventListener(PAUSE_EVENT, onPause);
    return () => window.removeEventListener(PAUSE_EVENT, onPause);
  }, []);
  const libraryUuids = useMemo(
    () => new Set(stories.map((item) => item.uuid)),
    [stories],
  );
  return (
    <DeviceProvider
      intervalMs={20}
      paused={paused}
      libraryUuids={installedRevision > 0 ? libraryUuids : undefined}
      onPcCopyChange={onPcCopyChange}
    >
      {children}
    </DeviceProvider>
  );
}

const onPcCopyChange = vi.fn();

async function renderApp(first = "Les trois ours") {
  onPcCopyChange.mockClear();
  const view = render(
    <ToastProvider>
      <LibraryProvider>
        <CompatProvider>
          <StoriesProvider>
            <Boundary>
              <main>
                <StoriesLibrary />
              </main>
              <aside>
                <StoriesTaskPanel />
                <TransferPanel />
              </aside>
            </Boundary>
          </StoriesProvider>
        </CompatProvider>
      </LibraryProvider>
    </ToastProvider>,
  );
  await screen.findByRole("heading", { level: 3, name: first });
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

const SEND = (title: string) => `Télécharger et envoyer « ${title} »`;
const PLAIN = (title: string) => `Télécharger « ${title} »`;

/** Bouton « Télécharger et envoyer » de la carte, une fois la Flam lue. */
const sendButton = (title: string) => screen.findByRole("button", { name: SEND(title) });

/** L'installation de `entry` se termine avec succès (bibliothèque à jour, puis événement). */
async function installDone(entry: StoryEntry) {
  const item = installedEntry(entry);
  installed = [...installed.filter((other) => other.uuid !== entry.uuid), item];

  await act(async () => {
    await emit("install://done", {
      task_id: `t-${entry.uuid}`,
      entry: item,
      warnings: 0,
      pack: null,
    });
  });
}

async function installFailed(
  entry: StoryEntry,
  state: "failed" | "cancelled" = "failed",
) {
  await act(async () => {
    await emit("install://error", {
      task_id: `t-${entry.uuid}`,
      state,
      error: { kind: "conversion", message: "conversion impossible", code: null },
    });
  });
}

/** Laisse passer quelques scrutations de la Flam et les suites des promesses. */
async function settle(ms = 120) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

const planned = () =>
  screen.queryByRole("list", { name: "Histoires à envoyer après leur installation" });
const queueRow = (title: string) =>
  Array.from(
    screen.getByRole("list", { name: "File d'envoi" }).querySelectorAll("li"),
  ).find((li) => li.textContent?.includes(title)) as HTMLElement;

/** Textes des régions vivantes (annonces du panneau d'envoi comprises). */
const announced = () => screen.getAllByRole("status").map((node) => node.textContent);

const CHANGED =
  "L'histoire est installée dans Mes histoires, mais elle n'a pas été envoyée : la Flam a été débranchée ou a changé pendant l'installation. Vérifiez la Flam branchée, puis utilisez « Envoyer sur la Flam ».";
const NOT_SENT = (reason: string) =>
  `L'histoire est installée dans Mes histoires, mais elle n'a pas été envoyée : ${reason}`;

slowIt(
  "proposé seulement quand une Flam est prête ; « Télécharger » reste là, inchangé",
  async () => {
    status = absent;
    await renderApp();
    await settle();
    expect(screen.getByRole("button", { name: PLAIN("Les trois ours") })).toBeVisible();
    expect(screen.queryByRole("button", { name: SEND("Les trois ours") })).toBeNull();

    // La Flam est branchée : l'action apparaît, à côté de « Télécharger ».
    status = detected();
    const button = await sendButton("Les trois ours");
    expect(button).toHaveAccessibleDescription(/puis l'envoie sur la Flam branchée/);
    expect(screen.getByRole("button", { name: PLAIN("Les trois ours") })).toBeVisible();
    await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);

    // « Télécharger » seul : installation sans envoi.
    fireEvent.click(screen.getByRole("button", { name: PLAIN("Les trois ours") }));
    await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
    expect(planned()).toBeNull();
    await installDone(ours);
    await settle();
    expect(sent()).toEqual([]);
  },
);

slowIt("installe, puis envoie sur la Flam du clic, une seule fois", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  // Installation en cours : l'envoi est annoncé, rien n'est encore envoyé.
  const list = await waitFor(() => {
    const found = planned();
    expect(found).not.toBeNull();
    return found as HTMLElement;
  });
  expect(within(list).getByText("Les trois ours")).toBeVisible();
  expect(within(list).getByText("Installation…")).toBeVisible();
  expect(announced()).toContain(
    "« Les trois ours » sera envoyée sur la Flam à la fin de son installation",
  );
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
  await settle();
  expect(sent()).toEqual([]);

  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
  expect(callsOf("device_send_story")[0]?.args).toEqual({ root: "E:\\", uuid: "ours" });
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText("Envoyée")).toBeVisible(),
  );
  expect(planned()).toBeNull();
  // Un second événement de fin (doublon) ne renvoie rien.
  await installDone(ours);
  await settle();
  expect(sent()).toEqual(["ours"]);
});

it("échec de l'installation : rien n'est envoyé, l'échec s'affiche comme d'habitude", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(planned()).not.toBeNull());
  await installFailed(ours);
  await waitFor(() => expect(planned()).toBeNull());
  const tasks = screen.getByRole("region", { name: "Tâches en cours" });
  expect(within(tasks).getByText("conversion impossible")).toBeVisible();
  // « Réessayer » relance une installation simple : sa réussite n'envoie rien.
  fireEvent.click(within(tasks).getByRole("button", { name: /Réessayer/ }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
  expect(screen.queryByRole("list", { name: "File d'envoi" })).toBeNull();
});

it("installation annulée : pas d'envoi, même si elle se termine avant de voir l'annulation", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(planned()).not.toBeNull());
  const tasks = screen.getByRole("region", { name: "Tâches en cours" });
  fireEvent.click(within(tasks).getByRole("button", { name: /Annuler/ }));
  await waitFor(() => expect(callsOf("install_cancel")).toHaveLength(1));
  await waitFor(() => expect(planned()).toBeNull());
  // L'annulation arrive trop tard : l'installation se termine quand même.
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
});

it("Flam débranchée puis rebranchée pendant l'installation : histoire installée, rien n'est envoyé", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(planned()).not.toBeNull());

  status = absent;
  await waitFor(() =>
    expect(
      within(planned() as HTMLElement).getByText("Ne sera pas envoyée"),
    ).toBeVisible(),
  );
  expect(announced()).toContain(
    "La Flam a été débranchée ou a changé : l'histoire en cours d'installation ne sera pas envoyée",
  );
  // Une Flam revient sous la même lettre et le même libellé : ce peut être une autre.
  status = detected();
  await settle();
  expect(within(planned() as HTMLElement).getByText("Ne sera pas envoyée")).toBeVisible();

  await installDone(ours);
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText(CHANGED)).toBeVisible(),
  );
  expect(within(queueRow("Les trois ours")).getByText("Échec")).toBeVisible();
  expect(planned()).toBeNull();
  await settle();
  expect(sent()).toEqual([]);
  // L'histoire est installée : l'envoi reste possible, à la main.
  expect(
    await screen.findByRole("button", { name: "Envoyer sur la Flam « Les trois ours »" }),
  ).toHaveAttribute("aria-disabled", "false");
});

it("Flam remplacée par une autre ou absente à la fin de l'installation : rien n'est envoyé", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));

  // Une autre Flam (autre lettre de lecteur) prend la place de la première.
  status = detected({ root: "F:\\" });
  await waitFor(() =>
    expect(
      within(planned() as HTMLElement).getAllByText("Ne sera pas envoyée"),
    ).toHaveLength(2),
  );
  await installDone(ours);
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText(CHANGED)).toBeVisible(),
  );
  // Plus de Flam du tout à la fin de la seconde installation.
  status = absent;
  await settle();
  await installDone(capi);
  await waitFor(() => expect(within(queueRow("Capi")).getByText(CHANGED)).toBeVisible());
  // Le premier message reste lisible.
  expect(within(queueRow("Les trois ours")).getByText(CHANGED)).toBeVisible();
  expect(sent()).toEqual([]);
});

it("histoire déjà installée : « Envoyer sur la Flam » seul, pas de réinstallation proposée", async () => {
  installed = [installedEntry(ours)];
  await renderApp();
  const send = await screen.findByRole("button", {
    name: "Envoyer sur la Flam « Les trois ours »",
  });
  await waitFor(() => expect(send).toHaveAttribute("aria-disabled", "false"));
  expect(screen.queryByRole("button", { name: SEND("Les trois ours") })).toBeNull();
  // L'autre histoire, non installée, garde l'action.
  expect(await sendButton("Capi")).toBeVisible();
});

it("envoi déjà connu comme impossible : l'action n'est pas proposée", async () => {
  // Déjà sur la Flam.
  deviceStories = [onFlam("ours")];
  const view = await renderApp();
  expect(await sendButton("Capi")).toBeVisible();
  expect(screen.queryByRole("button", { name: SEND("Les trois ours") })).toBeNull();
  view.unmount();

  // Espace insuffisant (taille annoncée et marge).
  deviceStories = [];
  status = detected({ space: { free: 10 * MB, total: 4096 * MB } });
  const small = await renderApp();
  await settle();
  expect(screen.queryByRole("button", { name: SEND("Les trois ours") })).toBeNull();
  expect(screen.getByRole("button", { name: PLAIN("Les trois ours") })).toBeVisible();
  small.unmount();

  // Écriture bloquée par la garde du firmware.
  status = detected({
    firmware_status: firmwareV2({ write_block: "unsupported" } as any),
  });
  await renderApp();
  await settle();
  expect(screen.queryByRole("button", { name: SEND("Les trois ours") })).toBeNull();
});

it("histoire STUdio : proposée sur une Flam 2.x, jamais sur une Flam 1.x", async () => {
  window.location.hash = "#/library/dossier";
  const view = await renderApp("Pack STUdio");
  expect(
    await screen.findByRole("button", { name: "Installer et envoyer « Pack STUdio »" }),
  ).toBeVisible();
  view.unmount();

  status = detected({ firmware: "1.15.14", firmware_status: firmwareV1() });
  await renderApp("Pack STUdio");
  await settle();
  expect(
    screen.queryByRole("button", { name: "Installer et envoyer « Pack STUdio »" }),
  ).toBeNull();
  expect(screen.getByRole("button", { name: "Installer « Pack STUdio »" })).toBeVisible();
});

it("envoi devenu impossible pendant l'installation : même refus que « Envoyer », rien n'est envoyé", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));

  // La Flam se remplit pendant l'installation (même Flam, même lettre).
  status = detected({ space: { free: 10 * MB, total: 4096 * MB } });
  await settle();
  await installDone(ours);
  await waitFor(() =>
    expect(
      within(queueRow("Les trois ours")).getByText(
        NOT_SENT("Espace insuffisant sur la Flam"),
      ),
    ).toBeVisible(),
  );
  expect(sent()).toEqual([]);
});

it("plusieurs demandes : envois dans l'ordre des clics, sans doublon", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));

  // La seconde installation se termine la première : son envoi attend celui de la première.
  await installDone(capi);
  await waitFor(() =>
    expect(
      within(planned() as HTMLElement).getByText("Installée, envoi en attente"),
    ).toBeVisible(),
  );
  await settle();
  expect(sent()).toEqual([]);

  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours", "capi"]));
  await waitFor(() =>
    expect(within(queueRow("Capi")).getByText("Envoyée")).toBeVisible(),
  );
  expect(within(queueRow("Les trois ours")).getByText("Envoyée")).toBeVisible();
  await settle();
  expect(sent()).toEqual(["ours", "capi"]);
});

it("fenêtre rechargée pendant l'installation : pas d'envoi au redémarrage", async () => {
  const view = await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(planned()).not.toBeNull());
  view.unmount();

  // Nouvelle fenêtre : le backend a repris la tâche, qui se termine.
  await renderApp();
  expect(planned()).toBeNull();
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
});

it("avertissement « expérimentale » demandé au clic : refusé, rien n'est installé", async () => {
  window.localStorage.removeItem("flam-launcher.flamSendWarningAck");
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  const dialog = await screen.findByRole("alertdialog", {
    name: "Fonction expérimentale, non officielle",
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(callsOf("install_start")).toHaveLength(0);
  expect(planned()).toBeNull();

  // Accepté : l'installation démarre, l'envoi suivra sans nouvelle question.
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "J'ai compris, installer et envoyer",
    }),
  );
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
  expect(screen.queryByRole("alertdialog")).toBeNull();
});

it("« Ne pas envoyer » retire l'envoi prévu : l'installation continue, rien n'est envoyé", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  const drop = await screen.findByRole("button", {
    name: "Ne pas envoyer « Les trois ours » après l'installation",
  });
  drop.focus();
  fireEvent.click(drop);
  await waitFor(() => expect(planned()).toBeNull());
  expect(callsOf("install_cancel")).toHaveLength(0);
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
});

it("installation d'une histoire incompatible refusée au dialogue : aucun envoi prévu", async () => {
  report = {
    issue: null,
    entries: [
      {
        uuid: "ours",
        version: null,
        level: "incompatible",
        reason: "Format non pris en charge",
        code: "KO",
        origin: "campaign",
        stale: false,
      },
    ],
  };
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(callsOf("install_start")).toHaveLength(0);
  expect(planned()).toBeNull();
  // La demande est oubliée : l'action est de nouveau proposée.
  expect(await sendButton("Les trois ours")).toBeVisible();
});

it("« Annuler l'envoi » oublie aussi les envois prévus", async () => {
  let release: () => void = () => undefined;
  sendGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours"]));

  fireEvent.click(await screen.findByRole("button", { name: "Annuler l'envoi" }));
  await waitFor(() => expect(planned()).toBeNull());
  release();
  await installDone(capi);
  await settle();
  expect(sent()).toEqual(["ours"]);
});

it("fiche de l'histoire : même action, et rappel de l'envoi prévu pendant l'installation", async () => {
  await renderApp();
  await sendButton("Les trois ours");
  fireEvent.click(screen.getByRole("button", { name: "Les trois ours" }));
  const dialog = await screen.findByRole("dialog", { name: "Les trois ours" });
  expect(within(dialog).getByRole("button", { name: "Télécharger" })).toBeVisible();
  fireEvent.click(within(dialog).getByRole("button", { name: "Télécharger et envoyer" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  expect(
    await within(dialog).findByText(
      "Elle sera envoyée sur la Flam à la fin de l'installation.",
    ),
  ).toBeVisible();
  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
});

it("réglage de retrait actif : le retrait suit l'envoi par le mécanisme existant, avec son rappel", async () => {
  libraryCopy = { status: "removed" };
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
  expect(
    await screen.findByText(
      "Retirée de Mes histoires : l'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller.",
    ),
  ).toBeVisible();
  expect(onPcCopyChange).toHaveBeenCalledWith("Les trois ours", { status: "removed" });
  // Le front n'appelle aucun retrait lui-même : le backend l'a fait pendant l'envoi.
  expect(callsOf("library_remove_sent")).toHaveLength(0);
  expect(callsOf("library_remove")).toHaveLength(0);
});

it("Flam 1.x : les clés sont demandées une histoire à la fois, aucune demande n'est écrasée", async () => {
  status = detected({ firmware: "1.15.14", firmware_status: firmwareV1() });
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(ours);
  await installDone(capi);

  // Dialogue des clés pour la première ; la seconde attend, toujours prévue.
  const CARRIER = "Flam en firmware 1.x : mode carrier";
  const first = await screen.findByRole("alertdialog", { name: CARRIER });
  await settle();
  const list = planned() as HTMLElement;
  expect(within(list).getByText("Capi")).toBeInTheDocument();
  expect(within(list).queryByText("Les trois ours")).toBeNull();
  expect(within(list).getByText("Installée, envoi en attente")).toBeInTheDocument();

  // Refusé pour la première : le dialogue revient pour la seconde, refusée aussi.
  fireEvent.click(within(first).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(planned()).toBeNull());
  const second = await screen.findByRole("alertdialog", { name: CARRIER });
  fireEvent.click(within(second).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  await settle();
  expect(sent()).toEqual([]);
});

// --- Revue adversariale (#636) ------------------------------------------------------------

/** La fenêtre est masquée ou réaffichée (`visibilitychange`). */
async function setWindowHidden(hidden: boolean) {
  windowHidden = hidden;
  await act(async () => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
}

const MANUAL = (title: string) => `Envoyer sur la Flam « ${title} »`;

/** Deux demandes ; « Capi » est installée et envoyée, « Les trois ours » s'installe encore. */
async function capiSentWhileOursInstalls() {
  await renderApp();
  fireEvent.click(await sendButton("Capi"));
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  await waitFor(() => expect(sent()).toEqual(["capi"]));
}

it("Flam changée pendant un envoi (refus `device_changed`) : les envois prévus sont abandonnés", async () => {
  sendOutcomes.set("capi", {
    error: { kind: "device_changed", message: "la Flam a été débranchée ou remplacée" },
  });
  await capiSentWhileOursInstalls();
  // Même lettre, même libellé : la clé d'appareil du front n'a rien vu.
  await waitFor(() =>
    expect(
      within(planned() as HTMLElement).getByText("Ne sera pas envoyée"),
    ).toBeVisible(),
  );
  await installDone(ours);
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText(CHANGED)).toBeVisible(),
  );
  await settle();
  expect(sent()).toEqual(["capi"]);
});

it("Flam changée juste après un envoi (rapport `device_changed`) : les envois prévus sont abandonnés", async () => {
  sendOutcomes.set("capi", { report: { device_changed: true } });
  await capiSentWhileOursInstalls();
  await waitFor(() =>
    expect(
      within(planned() as HTMLElement).getByText("Ne sera pas envoyée"),
    ).toBeVisible(),
  );
  await installDone(ours);
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText(CHANGED)).toBeVisible(),
  );
  await settle();
  expect(sent()).toEqual(["capi"]);
});

it("histoire envoyée à la main pendant l'attente, puis retirée de la Flam : elle n'est pas renvoyée", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  await waitFor(() =>
    expect(
      within(planned() as HTMLElement).getByText("Installée, envoi en attente"),
    ).toBeVisible(),
  );
  // L'utilisateur n'attend pas : il envoie « Capi » lui-même.
  const manual = await screen.findByRole("button", { name: MANUAL("Capi") });
  await waitFor(() => expect(manual).toHaveAttribute("aria-disabled", "false"));
  fireEvent.click(manual);
  await waitFor(() => expect(sent()).toEqual(["capi"]));
  await waitFor(() =>
    expect(within(queueRow("Capi")).getByText("Envoyée")).toBeVisible(),
  );
  // Son envoi prévu est oublié ; la liste de la Flam ne la contient pas (retirée depuis).
  expect(within(planned() as HTMLElement).queryByText("Capi")).toBeNull();
  await settle();

  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["capi", "ours"]));
  await settle();
  expect(sent()).toEqual(["capi", "ours"]);
});

it("histoire envoyée à la main pendant l'attente : sa ligne « Envoyée » et « Retirer du PC » restent", async () => {
  sendAddsToFlam = true;
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  const manual = await screen.findByRole("button", { name: MANUAL("Capi") });
  await waitFor(() => expect(manual).toHaveAttribute("aria-disabled", "false"));
  fireEvent.click(manual);
  await waitFor(() => expect(sent()).toEqual(["capi"]));
  await screen.findByRole("button", { name: "Retirer du PC : « Capi »" });

  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["capi", "ours"]));
  await settle();
  expect(within(queueRow("Capi")).getByText("Envoyée")).toBeVisible();
  expect(within(queueRow("Capi")).queryByText("Échec")).toBeNull();
  expect(screen.getByRole("button", { name: "Retirer du PC : « Capi »" })).toBeVisible();
});

it("histoire supprimée de « Mes histoires » pendant l'attente : son envoi prévu est oublié", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  fireEvent.click(await screen.findByRole("button", { name: "Supprimer « Capi »" }));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(within(dialog).getByRole("button", { name: "Supprimer" }));
  await waitFor(() => expect(callsOf("library_remove")).toHaveLength(1));
  await waitFor(() =>
    expect(within(planned() as HTMLElement).queryByText("Capi")).toBeNull(),
  );

  await installDone(ours);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
  await settle();
  expect(sent()).toEqual(["ours"]);
  expect(queueRow("Capi")).toBeUndefined();
});

const TO_CONFIRM = "Installée, envoi à confirmer";

it("installation terminée fenêtre masquée : rien ne part seul, l'envoi est à confirmer", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  // Fenêtre réduite : la Flam n'est plus surveillée, une autre peut prendre sa lettre.
  await setWindowHidden(true);
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);

  await setWindowHidden(false);
  await settle();
  expect(sent()).toEqual([]);
  const list = planned() as HTMLElement;
  expect(within(list).getByText(TO_CONFIRM)).toBeVisible();
  await expectNoAxeViolations(document.body, PAGE_LEVEL_RULES);
  // L'utilisateur confirme : l'envoi part, par le bouton d'envoi habituel.
  const confirm = within(list).getByRole("button", { name: MANUAL("Les trois ours") });
  await waitFor(() => expect(confirm).toHaveAttribute("aria-disabled", "false"));
  fireEvent.click(confirm);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
  await waitFor(() => expect(planned()).toBeNull());
});

it("installation terminée pendant la lecture incrustée (scrutation en pause) : envoi à confirmer", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  act(() => setPaused(true));
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
  act(() => setPaused(false));
  await settle();
  expect(sent()).toEqual([]);
  const list = planned() as HTMLElement;
  expect(within(list).getByText(TO_CONFIRM)).toBeVisible();
  // « Ne pas envoyer » : l'histoire reste installée, rien n'est envoyé.
  fireEvent.click(
    within(list).getByRole("button", {
      name: "Ne pas envoyer « Les trois ours » après l'installation",
    }),
  );
  await waitFor(() => expect(planned()).toBeNull());
  await settle();
  expect(sent()).toEqual([]);
});

it("fenêtre masquée puis réaffichée avant la fin de l'installation : envoi à confirmer, les demandes suivantes partent", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await setWindowHidden(true);
  await setWindowHidden(false);
  await settle();
  // Demande faite après la reprise : la Flam a été surveillée sans interruption depuis.
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));

  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
  expect(within(planned() as HTMLElement).getByText(TO_CONFIRM)).toBeVisible();
  // L'envoi à confirmer ne retient pas la demande suivante.
  await installDone(capi);
  await waitFor(() => expect(sent()).toEqual(["capi"]));
  expect(within(planned() as HTMLElement).getByText("Les trois ours")).toBeVisible();
});

it("demande faite à la reprise, avant un relevé neuf de la Flam : envoi à confirmer", async () => {
  await renderApp();
  await sendButton("Les trois ours");
  await setWindowHidden(true);
  // Au retour, le relevé demandé ne répond pas encore : l'écran montre l'état d'avant.
  let release: () => void = () => undefined;
  statusGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  await setWindowHidden(false);
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  statusGate = null;
  await act(async () => {
    release();
  });
  await installDone(ours);
  await settle();
  expect(sent()).toEqual([]);
  expect(within(planned() as HTMLElement).getByText(TO_CONFIRM)).toBeVisible();
});

it("confirmation d'installation remplacée par une autre : la première demande est abandonnée", async () => {
  // « Capi » est installée dans l'autre format : confirmation demandée après relecture.
  installed = [{ ...installedEntry(capi), format: "lunii" }];
  report = {
    issue: null,
    entries: [
      {
        uuid: "ours",
        version: null,
        level: "incompatible",
        reason: "Format non pris en charge",
        code: "KO",
        origin: "campaign",
        stale: false,
      },
    ],
  };
  await renderApp();
  await sendButton("Les trois ours");
  let release: () => void = () => undefined;
  listGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  // « Capi » : relecture de la bibliothèque retenue ; « Les trois ours » : dialogue aussitôt.
  fireEvent.click(await sendButton("Capi"));
  fireEvent.click(await sendButton("Les trois ours"));
  await screen.findByRole("alertdialog", { name: /Les trois ours/ });
  await act(async () => {
    release();
  });
  // Le dialogue de « Capi » remplace celui de « Les trois ours ».
  const dialog = await screen.findByRole("alertdialog", { name: /Capi/ });
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(callsOf("install_start")).toHaveLength(0);
  // Rien n'est resté prévu : l'action est de nouveau proposée pour les deux histoires.
  expect(await sendButton("Les trois ours")).toBeVisible();
  expect(await sendButton("Capi")).toBeVisible();
  expect(planned()).toBeNull();
});

it("rien n'est affiché comme « Installation… » tant qu'un dialogue de confirmation est ouvert", async () => {
  report = {
    issue: null,
    entries: [
      {
        uuid: "ours",
        version: null,
        level: "incompatible",
        reason: "Format non pris en charge",
        code: "KO",
        origin: "campaign",
        stale: false,
      },
    ],
  };
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  const dialog = await screen.findByRole("alertdialog");
  await settle();
  expect(planned()).toBeNull();
  // Confirmé : l'installation démarre, l'envoi prévu apparaît.
  const buttons = within(dialog).getAllByRole("button");
  fireEvent.click(buttons[buttons.length - 1] as HTMLElement);
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await waitFor(() =>
    expect(within(planned() as HTMLElement).getByText("Installation…")).toBeVisible(),
  );
});

it("demande de tête qui n'aboutit jamais : « Ne pas envoyer » débloque la suivante", async () => {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  await settle();
  expect(sent()).toEqual([]);
  fireEvent.click(
    screen.getByRole("button", {
      name: "Ne pas envoyer « Les trois ours » après l'installation",
    }),
  );
  await waitFor(() => expect(sent()).toEqual(["capi"]));
});

it("la liste ne garde que les lignes utiles quand un envoi prévu s'ajoute", async () => {
  // Aucune suite proposée par le backend : la ligne « Envoyée » n'a plus d'action.
  libraryCopy = null;
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await installDone(ours);
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText("Envoyée")).toBeVisible(),
  );
  await settle();
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  await waitFor(() =>
    expect(within(queueRow("Capi")).getByText("Envoyée")).toBeVisible(),
  );
  expect(queueRow("Les trois ours")).toBeUndefined();
});

it("avertissement ouvert par « Installer et envoyer » : le bouton dit que l'installation vient d'abord", async () => {
  window.localStorage.removeItem("flam-launcher.flamSendWarningAck");
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  const dialog = await screen.findByRole("alertdialog");
  expect(
    within(dialog).getByRole("button", { name: "J'ai compris, installer et envoyer" }),
  ).toBeVisible();
});

/** Installe « Les trois ours » avec une suite (`onSettled`) et compte ses appels. */
function SettleProbe({ onSettled }: { onSettled: (status: string) => void }) {
  const stories = useStories();
  return (
    <button
      type="button"
      onClick={() =>
        stories.install(ours, { onSettled: (outcome) => onSettled(outcome.status) })
      }
    >
      Sonde
    </button>
  );
}

it("la suite d'une installation n'est appelée qu'une fois, même si la fin est signalée deux fois", async () => {
  const onSettled = vi.fn();
  render(
    <ToastProvider>
      <LibraryProvider>
        <CompatProvider>
          <StoriesProvider>
            <SettleProbe onSettled={onSettled} />
          </StoriesProvider>
        </CompatProvider>
      </LibraryProvider>
    </ToastProvider>,
  );
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
  fireEvent.click(screen.getByRole("button", { name: "Sonde" }));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await installDone(ours);
  await installDone(ours);
  expect(onSettled.mock.calls).toEqual([["installed"]]);
});

// --- Second tour de revue (#636) ------------------------------------------------------------

it("un envoi prévu qui démarre garde l'échec non lu d'un envoi manuel", async () => {
  installed = [installedEntry(pack)];
  sendOutcomes.set("pack", {
    error: { kind: "io", message: "écriture impossible sur la Flam" },
  });
  await renderApp();
  // Envoi manuel d'une histoire installée, en échec.
  window.location.hash = "#/library/dossier";
  const manual = await screen.findByRole("button", { name: MANUAL("Pack STUdio") });
  await waitFor(() => expect(manual).toHaveAttribute("aria-disabled", "false"));
  fireEvent.click(manual);
  await waitFor(() =>
    expect(
      within(queueRow("Pack STUdio")).getByText("écriture impossible sur la Flam"),
    ).toBeVisible(),
  );
  // La file est terminée ; un nouvel envoi prévu arrive seul.
  window.location.hash = "#/library/store";
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await installDone(capi);
  await waitFor(() => expect(sent()).toEqual(["pack", "capi"]));
  await waitFor(() => expect(queueRow("Capi")).toBeDefined());
  // L'échec de l'envoi manuel reste lisible.
  expect(
    within(queueRow("Pack STUdio")).getByText("écriture impossible sur la Flam"),
  ).toBeVisible();
});

it("un envoi prévu qui démarre garde la ligne d'une histoire retirée du PC", async () => {
  libraryCopy = { status: "removed" };
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await installDone(ours);
  const REMOVED =
    "Retirée de Mes histoires : l'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller.";
  await waitFor(() =>
    expect(within(queueRow("Les trois ours")).getByText(REMOVED)).toBeVisible(),
  );
  await settle();
  fireEvent.click(await sendButton("Capi"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(2));
  await installDone(capi);
  await waitFor(() => expect(sent()).toEqual(["ours", "capi"]));
  await waitFor(() => expect(queueRow("Capi")).toBeDefined());
  expect(within(queueRow("Les trois ours")).getByText(REMOVED)).toBeVisible();
});

/** « Les trois ours » installée fenêtre masquée : sa ligne est « à confirmer ». */
async function oursToConfirm() {
  await renderApp();
  fireEvent.click(await sendButton("Les trois ours"));
  await waitFor(() => expect(callsOf("install_start")).toHaveLength(1));
  await setWindowHidden(true);
  await installDone(ours);
  await setWindowHidden(false);
  const list = await waitFor(() => {
    const found = planned() as HTMLElement;
    expect(within(found).getByText(TO_CONFIRM)).toBeVisible();
    return found;
  });
  const confirm = within(list).getByRole("button", { name: MANUAL("Les trois ours") });
  await waitFor(() => expect(confirm).toHaveAttribute("aria-disabled", "false"));
  return confirm;
}

it("envoi à confirmer, Flam 1.x : dialogue des clés annulé, la ligne « à confirmer » reste", async () => {
  status = detected({ firmware: "1.15.14", firmware_status: firmwareV1() });
  const confirm = await oursToConfirm();
  confirm.focus();
  fireEvent.click(confirm);
  const dialog = await screen.findByRole("alertdialog", {
    name: "Flam en firmware 1.x : mode carrier",
  });
  // Tant que rien n'est mis en file, la ligne reste.
  expect(within(planned() as HTMLElement).getByText(TO_CONFIRM)).toBeInTheDocument();
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  await settle();
  const list = planned() as HTMLElement;
  expect(within(list).getByText(TO_CONFIRM)).toBeVisible();
  expect(
    within(list).getByRole("button", { name: MANUAL("Les trois ours") }),
  ).toBeVisible();
  expect(sent()).toEqual([]);
});

it("envoi à confirmer envoyé : la ligne disparaît, le focus passe au titre du panneau", async () => {
  // L'envoi reste en cours : le focus ne doit pas attendre la fin de la file.
  let release: () => void = () => undefined;
  sendGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const confirm = await oursToConfirm();
  confirm.focus();
  fireEvent.click(confirm);
  await waitFor(() => expect(sent()).toEqual(["ours"]));
  await waitFor(() => expect(planned()).toBeNull());
  expect(screen.getByRole("heading", { name: "Envoi sur la Flam" })).toHaveFocus();
  await act(async () => {
    release();
  });
});
