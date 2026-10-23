import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactElement } from "react";
import { afterEach, beforeEach, expect, it, onTestFinished, vi } from "vitest";
import type { DeviceStatus, DeviceStories, DeviceStory } from "../../api/device";
import { expectNoAxeViolations } from "../../test/axe";
import { firmwareV1, firmwareV2 } from "../../test/firmware";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { ToastProvider } from "../../components";
import { DeviceProvider } from "./DeviceProvider";
import { DeviceScreen } from "./DeviceScreen";
import { StrictMode } from "react";
import { resetOfficialThumbnails } from "./storyDetails";
import { POLL_INTERVAL_MS } from "./useDeviceStatus";

tolerateLoadInAsyncUtils();

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const C = "33333333-3333-4333-8333-333333333333";

const absent: DeviceStatus = {
  detected: false,
  hint: null,
  device: null,
  device_count: 0,
};

function detected(
  overrides: Partial<DeviceStatus["device"] & object> = {},
): DeviceStatus {
  return {
    detected: true,
    hint: null,
    device_count: 1,
    device: {
      root: "E:\\",
      volume_label: "FLAM",
      file_system: "FAT32",
      space: { free: 250 * 1024 * 1024, total: 1024 * 1024 * 1024 },
      warnings: [],
      mdf_problem: null,
      backup_failed: false,
      firmware: "2.4.2",
      firmware_status: firmwareV2(),
      ...overrides,
    },
  };
}

function story(uuid: string, extra: Partial<DeviceStory> = {}): DeviceStory {
  return {
    uuid,
    short_id: uuid.slice(0, 8),
    placement: "visible",
    title: null,
    in_library: false,
    library_uuids: [],
    missing: false,
    ...extra,
  };
}

function report(
  stories: DeviceStory[],
  extra: Partial<DeviceStories> = {},
): DeviceStories {
  return {
    stories,
    visible_index: "valid",
    hidden_index: "valid",
    str_unreadable: false,
    truncated: false,
    limit: 5000,
    ...extra,
  };
}

function renderScreen(ui: ReactElement = <DeviceScreen />) {
  return render(
    <ToastProvider>
      <DeviceProvider>{ui}</DeviceProvider>
    </ToastProvider>,
  );
}

let status: DeviceStatus;
/** Si défini, `device_status` renvoie ce que rend cette fonction (réponse retenue). */
let statusGate: (() => Promise<DeviceStatus>) | null;
let stories: DeviceStories | null;
let failures: Record<string, { kind: string; message: string } | undefined>;
let calls: string[];
/** Titres déchiffrés renvoyés par `device_story_titles` (absent : `null`). */
let deviceTitles: Record<string, string>;
/** Vignettes renvoyées par `device_story_thumbnail` (absent : `null`). */
let deviceThumbs: Record<string, string>;
let titleRequests: string[][];
/** Titres du catalogue officiel renvoyés par `device_official_titles` (absent : inconnu). */
let officialTitles: Record<string, { title: string; has_image: boolean }>;
/** Images renvoyées par `device_official_thumbnail` (absent : `null`). */
let officialThumbs: Record<string, string>;
let officialRequests: string[][];

beforeEach(() => {
  status = absent;
  statusGate = null;
  stories = report([]);
  failures = {};
  calls = [];
  deviceTitles = {};
  deviceThumbs = {};
  titleRequests = [];
  officialTitles = {};
  officialThumbs = {};
  officialRequests = [];
  mockIPC(
    (cmd, args) => {
      calls.push(cmd);
      const failure = failures[cmd];
      if (failure) throw failure;
      switch (cmd) {
        case "device_status":
          return statusGate ? statusGate() : status;
        case "device_recover_transfers":
          return {
            incomplete_removed: 0,
            previous_restored: 0,
            updates_finished: 0,
            left_alone: 0,
          };
        case "device_list_stories":
          return stories;
        case "device_confirm_firmware": {
          const confirmed = firmwareV2({ trust: "confirmed", write_block: null });
          if (status.device) status.device.firmware_status = confirmed;
          return confirmed;
        }
        case "device_story_titles": {
          const { uuids } = args as { uuids: string[] };
          titleRequests.push(uuids);
          return {
            titles: uuids.map((uuid) => ({ uuid, title: deviceTitles[uuid] ?? null })),
            interrupted: false,
            keys: true,
          };
        }
        case "device_story_thumbnail": {
          const { uuid } = args as { uuid: string };
          return { thumb: deviceThumbs[uuid] ?? null, busy: false };
        }
        case "device_official_titles": {
          const { uuids } = args as { uuids: string[] };
          officialRequests.push(uuids);
          return {
            titles: uuids.flatMap((uuid) =>
              officialTitles[uuid] ? [{ uuid, ...officialTitles[uuid] }] : [],
            ),
          };
        }
        case "device_official_thumbnail": {
          const { uuid } = args as { uuid: string };
          return officialThumbs[uuid] ?? null;
        }
        case "device_story_details_cancel":
          return null;
        default:
          throw { kind: "unknown", message: cmd };
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  resetOfficialThumbnails();
  clearMocks();
  vi.useRealTimers();
});

const count = (cmd: string) => calls.filter((c) => c === cmd).length;

it("invite à activer le mode USB quand la Flam n'est pas détectée", async () => {
  const { container } = renderScreen();
  expect(screen.getByText("Recherche de la Flam…")).toBeInTheDocument();
  expect(
    await screen.findByRole("heading", { name: "Aucune Flam détectée" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText("Activez le mode connexion USB sur la Flam"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Réessayer" })).toBeInTheDocument();
  expect(count("device_list_stories")).toBe(0);
  await expectNoAxeViolations(container);
});

it("affiche le bandeau expérimental, avec le lien des risques, même sans Flam", async () => {
  const { container } = renderScreen();
  expect(
    await screen.findByRole("heading", { name: "Aucune Flam détectée" }),
  ).toBeInTheDocument();
  expect(
    screen.getByText("Fonction expérimentale et non officielle"),
  ).toBeInTheDocument();
  expect(screen.getByText(/flam-transfert\.md#risques/)).toBeInTheDocument();
  expect(screen.queryByText(/Firmware 2\.x : en test/)).not.toBeInTheDocument();
  await expectNoAxeViolations(container);
});

it("propose « Copier le diagnostic » sans Flam et quand l'écriture est bloquée", async () => {
  const first = renderScreen();
  expect(
    await screen.findByRole("heading", { name: "Aucune Flam détectée" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Copier le diagnostic" }),
  ).toBeInTheDocument();
  first.unmount();

  status = detected({
    firmware_status: firmwareV2({
      write_block: { kind: "keys_trivial", version: "2.4.2" },
    }),
  });
  const { container } = renderScreen();
  expect(
    await screen.findByRole("heading", { name: "Flam détectée" }),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Copier le diagnostic" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("heading", { name: "Retour de test" })).toBeInTheDocument();
  await expectNoAxeViolations(container);
});

it("ajoute la mention « firmware 2.x » pour une Flam en 2.x seulement", async () => {
  status = detected({ firmware: "2.4.2" });
  const first = renderScreen();
  expect(await screen.findByText(/Firmware 2\.x : en test/)).toBeInTheDocument();
  expect(screen.getByText(/sauvegardez votre Flam avant/)).toBeInTheDocument();
  first.unmount();

  status = detected({ firmware: "1.15.14", firmware_status: firmwareV1() });
  renderScreen();
  expect(
    await screen.findByRole("heading", { name: "Flam détectée" }),
  ).toBeInTheDocument();
  expect(screen.queryByText(/Firmware 2\.x : en test/)).not.toBeInTheDocument();
});

it("copie le lien des risques dans le presse-papiers", async () => {
  const writeText = vi.fn().mockResolvedValue(undefined);
  const original = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {
    value: { writeText },
    configurable: true,
  });
  onTestFinished(() => {
    if (original) Object.defineProperty(navigator, "clipboard", original);
    else Reflect.deleteProperty(navigator, "clipboard");
  });
  renderScreen();
  fireEvent.click(
    await screen.findByRole("button", { name: "Copier le lien des risques" }),
  );
  expect(writeText).toHaveBeenCalledWith(
    expect.stringContaining("apps/flam-launcher/docs/flam-transfert.md#risques"),
  );
  expect(await screen.findByText(/Lien copié/)).toBeInTheDocument();

  // Second clic : le message est vidé puis remis, pour être annoncé de nouveau.
  let release: () => void = () => {};
  writeText.mockReturnValueOnce(new Promise<void>((resolve) => (release = resolve)));
  fireEvent.click(screen.getByRole("button", { name: "Copier le lien des risques" }));
  expect(screen.queryByText(/Lien copié/)).not.toBeInTheDocument();
  await act(async () => release());
  expect(await screen.findByText(/Lien copié/)).toBeInTheDocument();
});

it("affiche le firmware sur la carte, « illisible » s'il est inconnu", async () => {
  status = detected({ firmware: "1.15.14", firmware_status: firmwareV1() });
  const first = renderScreen();
  expect(
    await screen.findByText("1.15.14 (1.x) · pris en charge (mode carrier)"),
  ).toBeInTheDocument();
  first.unmount();
  status = detected({
    firmware: null,
    firmware_status: firmwareV2({
      version: null,
      family: "unreadable",
      version_form: "unreadable",
      trust: "not_applicable",
      coherence: { kind: "not_checked" },
      write_block: { kind: "version_unreadable" },
    }),
  });
  renderScreen();
  expect(await screen.findByText("illisible · écriture bloquée")).toBeInTheDocument();
});

it(
  "affiche la Flam détectée, son espace et ses histoires",
  async () => {
    status = detected();
    deviceThumbs = { [A]: "data:image/png;base64,AAAA" };
    stories = report([
      story(A, { title: "Capi et la rivière", in_library: true }),
      story(B, { missing: true }),
      story(C, { placement: "hidden" }),
      story("44444444-4444-4444-8444-444444444444", { placement: "unlisted" }),
    ]);
    const { container } = renderScreen();

    expect(
      await screen.findByRole("heading", { name: "Flam détectée" }),
    ).toBeInTheDocument();
    expect(screen.getByText("FLAM (E:\\)")).toBeInTheDocument();
    expect(screen.getByText("FAT32")).toBeInTheDocument();
    const bar = screen.getByRole("progressbar", { name: "Espace utilisé sur la Flam" });
    expect(bar).toHaveAttribute("aria-valuenow", "76");
    expect(screen.getByText("250 Mo libres sur 1 Go")).toBeInTheDocument();

    expect(
      await screen.findByRole("heading", { name: "Histoires sur la Flam (4)" }),
    ).toBeInTheDocument();
    const list = screen.getByRole("list", { name: "Histoires installées sur la Flam" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(4);
    expect(within(rows[0]!).getByText("Capi et la rivière")).toBeInTheDocument();
    expect(within(rows[0]!).getByText("Identifiant 11111111")).toBeInTheDocument();
    await waitFor(() => expect(rows[0]!.querySelector("img")).not.toBeNull());
    expect(within(rows[1]!).getByText("Histoire 22222222")).toBeInTheDocument();
    expect(within(rows[1]!).getByText("Dossier manquant")).toBeInTheDocument();
    expect(within(rows[2]!).getByText("Masquée")).toBeInTheDocument();
    expect(within(rows[3]!).getByText("Non listée")).toBeInTheDocument();
    expect(rows[1]!.querySelector("img")).toBeNull();
    await expectNoAxeViolations(container);
  },
  SLOW_TEST_MS,
);

it("annonce le nombre d'histoires et affiche un état vide", async () => {
  status = detected();
  renderScreen();
  expect(await screen.findByText("Aucune histoire sur la Flam")).toBeInTheDocument();
  await waitFor(() =>
    expect(screen.getByRole("status")).toHaveTextContent("0 histoire sur la Flam"),
  );
  expect(
    screen.getByRole("heading", { name: "Histoires sur la Flam (0)" }),
  ).toBeInTheDocument();
});

it("affiche les avertissements de la Flam et des index", async () => {
  status = detected({
    warnings: [{ kind: "missing_str_dir" }, { kind: "unexpected_mdf_size", size: 130 }],
    mdf_problem: { kind: "truncated", size: 10, expected_min: 126 },
    backup_failed: true,
    firmware: "2.4.2",
  });
  status.device_count = 2;
  stories = report([story(A)], {
    visible_index: "damaged",
    hidden_index: "unreadable",
    str_unreadable: true,
  });
  const { container } = renderScreen();
  expect(
    await screen.findByText(/Le dossier des histoires \(str\) est introuvable/),
  ).toBeInTheDocument();
  expect(screen.getByText(/taille inattendue \(130 octets\)/)).toBeInTheDocument();
  expect(screen.getByText(/est tronqué ou vide/)).toBeInTheDocument();
  expect(screen.getByText(/copie de sauvegarde/)).toBeInTheDocument();
  expect(screen.getByText(/2 Flam sont branchées/)).toBeInTheDocument();
  expect(
    await screen.findByText(/lignes invalides ou en double : elles sont ignorées\.$/),
  ).toBeInTheDocument();
  expect(
    screen.getByText("L'index des histoires masquées est illisible."),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/dossier des histoires \(str\) est illisible/),
  ).toBeInTheDocument();
  await expectNoAxeViolations(container);
});

it("affiche l'espace indisponible sans mesure", async () => {
  status = detected({ space: null, volume_label: "" });
  renderScreen();
  expect(await screen.findByText("Espace disque indisponible")).toBeInTheDocument();
  expect(screen.getByText("(sans nom) (E:\\)")).toBeInTheDocument();
  expect(screen.queryByRole("progressbar")).toBeNull();
});

it("affiche l'erreur de détection avec un bouton Réessayer", async () => {
  failures.device_status = { kind: "io", message: "disque en panne" };
  const { container } = renderScreen();
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("disque en panne");
  failures.device_status = undefined;
  fireEvent.click(within(alert).getByRole("button", { name: "Réessayer" }));
  expect(
    await screen.findByRole("heading", { name: "Aucune Flam détectée" }),
  ).toBeInTheDocument();
  expect(screen.queryByRole("alert")).toBeNull();
  await expectNoAxeViolations(container);
});

it("signale l'échec de lecture des histoires et permet de réessayer", async () => {
  status = detected();
  failures.device_list_stories = { kind: "io", message: "illisible" };
  renderScreen();
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(
    "Impossible de lire les histoires de la Flam. illisible",
  );
  failures.device_list_stories = undefined;
  stories = report([story(A)]);
  fireEvent.click(within(alert).getByRole("button", { name: "Réessayer" }));
  expect(
    await screen.findByRole("heading", { name: "Histoires sur la Flam (1)" }),
  ).toBeInTheDocument();
});

it("actualise la liste à la demande", async () => {
  status = detected();
  renderScreen();
  await screen.findByText("Aucune histoire sur la Flam");
  stories = report([story(A)]);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser la liste" }));
  expect(
    await screen.findByRole("heading", { name: "Histoires sur la Flam (1)" }),
  ).toBeInTheDocument();
  expect(count("device_list_stories")).toBe(2);
});

it(
  "rend le focus au titre quand « Réessayer » trouve la Flam",
  async () => {
    renderScreen();
    const retry = await screen.findByRole("button", { name: "Réessayer" });
    status = detected();
    fireEvent.click(retry);
    const heading = await screen.findByRole("heading", { name: "Flam détectée" });
    await waitFor(() => expect(heading).toHaveFocus());
    expect(screen.getByRole("status")).toHaveTextContent(/Flam détectée|histoire/);
  },
  SLOW_TEST_MS,
);

it(
  "rend le focus au titre même si un appel en vol répond avant celui de « Réessayer »",
  async () => {
    renderScreen();
    const retry = await screen.findByRole("button", { name: "Réessayer" });
    // Un appel est déjà en vol (retour au premier plan) quand on clique : sa réponse, périmée
    // (« non détectée »), arrive et est affichée avant celle de l'appel lancé par le clic.
    const held: ((value: DeviceStatus) => void)[] = [];
    const answer = (index: number, value: DeviceStatus) => held[index]?.(value);
    statusGate = () => new Promise<DeviceStatus>((resolve) => held.push(resolve));
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(held).toHaveLength(1));
    fireEvent.click(retry);
    await act(async () => {
      answer(0, { ...absent });
    });
    await waitFor(() => expect(held).toHaveLength(2));
    await act(async () => {
      answer(1, detected());
    });
    statusGate = null;
    const heading = await screen.findByRole("heading", { name: "Flam détectée" });
    await waitFor(() => expect(heading).toHaveFocus());
  },
  SLOW_TEST_MS,
);

it(
  "rend le focus au titre même si l'appel en vol qui précède « Réessayer » trouve déjà la Flam",
  async () => {
    renderScreen();
    const retry = await screen.findByRole("button", { name: "Réessayer" });
    // Un appel est déjà en vol (retour au premier plan) quand on clique : sa réponse, périmée
    // (déjà « détectée »), arrive et est affichée avant celle de l'appel lancé par le clic.
    const held: ((value: DeviceStatus) => void)[] = [];
    const answer = (index: number, value: DeviceStatus) => held[index]?.(value);
    statusGate = () => new Promise<DeviceStatus>((resolve) => held.push(resolve));
    document.dispatchEvent(new Event("visibilitychange"));
    await waitFor(() => expect(held).toHaveLength(1));
    fireEvent.click(retry);
    await act(async () => {
      answer(0, detected());
    });
    await waitFor(() => expect(held).toHaveLength(2));
    await act(async () => {
      answer(1, detected());
    });
    statusGate = null;
    const heading = await screen.findByRole("heading", { name: "Flam détectée" });
    await waitFor(() => expect(heading).toHaveFocus());
  },
  SLOW_TEST_MS,
);

it("garde le focus dans la liste quand on actualise ou réessaie", async () => {
  status = detected();
  renderScreen();
  const refresh = await screen.findByRole("button", { name: "Actualiser la liste" });
  refresh.focus();
  fireEvent.click(refresh);
  await screen.findByRole("heading", { name: "Histoires sur la Flam (0)" });
  expect(document.body).not.toHaveFocus();
  expect(document.activeElement?.className).toBe("fl-device-stories-wrap");
});

it("sort de l'attente quand la Flam disparaît entre la détection et la lecture", async () => {
  status = detected();
  stories = null;
  renderScreen();
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("La Flam n'a pas répondu");
  stories = report([story(A)]);
  fireEvent.click(within(alert).getByRole("button", { name: "Réessayer" }));
  expect(
    await screen.findByRole("heading", { name: "Histoires sur la Flam (1)" }),
  ).toBeInTheDocument();
});

it("ne scrute pas fenêtre masquée et reprend au retour", async () => {
  const hidden = vi.spyOn(document, "hidden", "get").mockReturnValue(true);
  renderScreen();
  await act(async () => {
    await new Promise((r) => setTimeout(r, 50));
  });
  expect(count("device_status")).toBe(0);
  hidden.mockReturnValue(false);
  await act(async () => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await screen.findByRole("heading", { name: "Aucune Flam détectée" });
  expect(count("device_status")).toBe(1);
  hidden.mockRestore();
});

it("affiche le panneau d'envoi fourni sous la liste", async () => {
  status = detected();
  stories = report([story(A, { title: "Capi" })]);
  renderScreen(<DeviceScreen libraryPanel={<p>Panneau d'envoi</p>} />);
  expect(await screen.findByText("Panneau d'envoi")).toBeInTheDocument();
  expect(
    await screen.findByRole("button", { name: "Retirer « Capi »" }),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Éjecter la Flam" })).toBeInTheDocument();
});

it("affiche le titre déchiffré et la vignette de la Flam, avec repli", async () => {
  status = detected();
  deviceTitles = { [A]: "Titre de la Flam" };
  deviceThumbs = { [A]: "data:image/png;base64,QUJD" };
  stories = report([
    story(A, { title: "Titre de la bibliothèque", in_library: true }),
    story(B, { title: "Capi (bibliothèque)", in_library: true }),
    story(C),
  ]);
  const { container } = renderScreen();
  const list = await screen.findByRole("list", {
    name: "Histoires installées sur la Flam",
  });
  const rows = within(list).getAllByRole("listitem");
  // Le titre déchiffré remplace celui de la bibliothèque.
  expect(await within(rows[0]!).findByText("Titre de la Flam")).toBeInTheDocument();
  expect(within(rows[0]!).queryByText("Titre de la bibliothèque")).toBeNull();
  await waitFor(() =>
    expect(rows[0]!.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,QUJD",
    ),
  );
  // Sans titre déchiffré : titre de la bibliothèque, puis UUID court.
  expect(within(rows[1]!).getByText("Capi (bibliothèque)")).toBeInTheDocument();
  expect(within(rows[2]!).getByText("Histoire 33333333")).toBeInTheDocument();
  expect(within(rows[2]!).queryByText(/^Identifiant/)).toBeNull();
  // Sans vignette : illustration par défaut, pas d'image.
  await waitFor(() => expect(count("device_story_thumbnail")).toBe(3));
  expect(rows[1]!.querySelector("img")).toBeNull();
  expect(rows[1]!.querySelector(".fl-device-row__thumb svg")).not.toBeNull();
  await expectNoAxeViolations(container);
});

it(
  "demande les titres par lots de 100",
  async () => {
    status = detected();
    const many = Array.from({ length: 150 }, (_, i) =>
      story(`00000000-0000-4000-8000-${i.toString(16).padStart(12, "0")}`),
    );
    stories = report([...many, story(B, { missing: true })]);
    deviceTitles = { [many[149]!.uuid]: "Dernière" };
    renderScreen();
    expect(
      await screen.findByRole("heading", { name: "Histoires sur la Flam (151)" }),
    ).toBeInTheDocument();
    expect(await screen.findByText("Dernière")).toBeInTheDocument();
    expect(titleRequests.map((r) => r.length)).toEqual([100, 50]);
    // Une histoire dont le dossier manque n'est pas lue.
    expect(titleRequests.flat()).not.toContain(B);
  },
  SLOW_TEST_MS,
);

it("redemande un lot interrompu par une opération sur la Flam", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  status = detected();
  stories = report([story(A), story(B)]);
  let first = true;
  mockIPC(
    (cmd, args) => {
      calls.push(cmd);
      switch (cmd) {
        case "device_status":
          return status;
        case "device_recover_transfers":
          return {
            incomplete_removed: 0,
            previous_restored: 0,
            updates_finished: 0,
            left_alone: 0,
          };
        case "device_list_stories":
          return stories;
        case "device_story_titles": {
          const { uuids } = args as { uuids: string[] };
          titleRequests.push(uuids);
          if (first) {
            first = false;
            return {
              titles: [{ uuid: A, title: "Premier" }],
              interrupted: true,
              keys: true,
            };
          }
          return {
            titles: uuids.map((uuid) => ({ uuid, title: "Second" })),
            interrupted: false,
            keys: true,
          };
        }
        case "device_story_thumbnail":
          return { thumb: null, busy: false };
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  renderScreen();
  expect(await screen.findByText("Premier")).toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(2100);
  });
  expect(await screen.findByText("Second")).toBeInTheDocument();
  expect(titleRequests).toEqual([[A, B], [B]]);
});

it("interrompt le lot de titres en cours quand l'écran est quitté", async () => {
  status = detected();
  stories = report([story(A)]);
  let release: (() => void) | undefined;
  mockIPC(
    async (cmd) => {
      calls.push(cmd);
      switch (cmd) {
        case "device_status":
          return status;
        case "device_recover_transfers":
          return {
            incomplete_removed: 0,
            previous_restored: 0,
            updates_finished: 0,
            left_alone: 0,
          };
        case "device_list_stories":
          return stories;
        case "device_story_titles":
          await new Promise<void>((resolve) => {
            release = resolve;
          });
          return { titles: [], interrupted: true, keys: true };
        case "device_story_thumbnail":
          return { thumb: null, busy: false };
        default:
          return null;
      }
    },
    { shouldMockEvents: true },
  );
  const { unmount } = renderScreen();
  await screen.findByText("Histoire 11111111");
  await waitFor(() => expect(count("device_story_titles")).toBe(1));
  unmount();
  expect(count("device_story_details_cancel")).toBe(1);
  release?.();
});

it("titre : déchiffré, puis catalogue officiel, puis bibliothèque ; l'image du catalogue en repli", async () => {
  status = detected();
  deviceTitles = { [A]: "Déchiffré" };
  deviceThumbs = { [C]: "data:image/png;base64,RkxBTQ==" };
  officialTitles = {
    [B]: { title: "Catalogue B", has_image: true },
    [C]: { title: "Catalogue C", has_image: true },
  };
  officialThumbs = {
    [B]: "data:image/png;base64,Q0FUQQ==",
    [C]: "data:image/png;base64,Q0FUQw==",
  };
  stories = report([
    story(A, { title: "Bibliothèque A", in_library: true }),
    story(B, { title: "Bibliothèque B", in_library: true }),
    story(C),
    story("44444444-4444-4444-8444-444444444444", {
      title: "Bibliothèque D",
      in_library: true,
    }),
  ]);
  renderScreen();
  const list = await screen.findByRole("list", {
    name: "Histoires installées sur la Flam",
  });
  const rows = within(list).getAllByRole("listitem");
  await waitFor(() =>
    expect(within(rows[1]!).getByText("Catalogue B")).toBeInTheDocument(),
  );
  expect(within(rows[0]!).getByText("Déchiffré")).toBeInTheDocument();
  expect(within(rows[2]!).getByText("Catalogue C")).toBeInTheDocument();
  // Hors catalogue : repli sur la bibliothèque.
  expect(within(rows[3]!).getByText("Bibliothèque D")).toBeInTheDocument();
  // Seules les histoires sans titre déchiffré sont demandées, en une fois.
  expect(officialRequests).toEqual([[B, C, "44444444-4444-4444-8444-444444444444"]]);
  // Image : celle de la Flam d'abord, celle du catalogue en repli.
  await waitFor(() =>
    expect(rows[1]!.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,Q0FUQQ==",
    ),
  );
  await waitFor(() =>
    expect(rows[2]!.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,RkxBTQ==",
    ),
  );
  expect(rows[0]!.querySelector("img")).toBeNull();
});

it("n'interroge pas le catalogue officiel quand tous les titres sont connus", async () => {
  status = detected();
  deviceTitles = { [A]: "Un", [B]: "Deux" };
  stories = report([story(A), story(B)]);
  renderScreen();
  expect(await screen.findByText("Deux")).toBeInTheDocument();
  await waitFor(() => expect(count("device_story_titles")).toBe(1));
  await new Promise((resolve) => setTimeout(resolve, 50));
  expect(count("device_official_titles")).toBe(0);
  expect(count("device_official_thumbnail")).toBe(0);
});

it("charge les vignettes en StrictMode (double montage)", async () => {
  status = detected();
  deviceThumbs = { [A]: "data:image/png;base64,QUJD" };
  stories = report([story(A)]);
  render(
    <StrictMode>
      <ToastProvider>
        <DeviceProvider>
          <DeviceScreen />
        </DeviceProvider>
      </ToastProvider>
    </StrictMode>,
  );
  const list = await screen.findByRole("list", {
    name: "Histoires installées sur la Flam",
  });
  await waitFor(() =>
    expect(within(list).getByRole("listitem").querySelector("img")).not.toBeNull(),
  );
});

it("ne modifie rien au simple affichage (lecture et reprise seulement)", async () => {
  status = detected();
  stories = report([story(A, { in_library: true, title: "Capi" })]);
  renderScreen();
  await screen.findByText("Capi");
  await waitFor(() => expect(count("device_story_thumbnail")).toBe(1));
  await waitFor(() => expect(count("device_story_titles")).toBe(1));
  // Titre non déchiffré : une demande (lecture seule) au catalogue officiel.
  await waitFor(() => expect(count("device_official_titles")).toBe(1));
  expect(new Set(calls)).toEqual(
    new Set([
      "device_status",
      "device_recover_transfers",
      "device_list_stories",
      "device_story_titles",
      "device_story_thumbnail",
      "device_official_titles",
    ]),
  );
});

it(
  "scrute toutes les 2 s et constate le débranchement",
  async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    status = detected();
    renderScreen();
    await screen.findByRole("heading", { name: "Flam détectée" });
    expect(count("device_status")).toBe(1);
    await waitFor(() => expect(count("device_list_stories")).toBe(1));

    status = absent;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
    });
    await waitFor(() => expect(count("device_status")).toBe(2));
    expect(
      await screen.findByRole("heading", { name: "Aucune Flam détectée" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("Flam débranchée ou mode connexion USB désactivé"),
    ).toBeInTheDocument();

    status = detected();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
    });
    expect(
      await screen.findByRole("heading", { name: "Flam détectée" }),
    ).toBeInTheDocument();
    await waitFor(() => expect(count("device_list_stories")).toBe(2));
  },
  SLOW_TEST_MS,
);

it(
  "un autre appareil (autre volume) remplace le premier sans passer par l'état absent",
  async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    status = detected();
    stories = report([story(A, { title: "Capi" })]);
    renderScreen();
    await screen.findByRole("heading", { name: "Histoires sur la Flam (1)" });
    expect(await screen.findByText("Capi")).toBeInTheDocument();
    await waitFor(() => expect(count("device_list_stories")).toBe(1));

    // Le premier appareil est remplacé entre deux scrutations : même lettre, autre volume.
    status = detected({ volume_label: "FLAM2" });
    stories = report([story(B, { title: "Loup" }), story(C, { title: "Ours" })]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
    });
    expect(
      await screen.findByRole("heading", { name: "Histoires sur la Flam (2)" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Capi")).toBeNull();
    expect(screen.getByText("Loup")).toBeInTheDocument();
    expect(count("device_list_stories")).toBe(2);
    // Jamais passé par « Aucune Flam détectée ».
    expect(screen.queryByRole("heading", { name: "Aucune Flam détectée" })).toBeNull();
  },
  SLOW_TEST_MS,
);

it("arrête la scrutation au démontage", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  status = detected();
  const { unmount } = renderScreen();
  await screen.findByRole("heading", { name: "Flam détectée" });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
  });
  expect(count("device_status")).toBe(2);

  unmount();
  expect(vi.getTimerCount()).toBe(0);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 5);
  });
  expect(count("device_status")).toBe(2);
});

it("ne lance pas deux détections en même temps", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  let release: (() => void) | undefined;
  let running = 0;
  let overlap = false;
  mockIPC(async (cmd) => {
    if (cmd !== "device_status") return null;
    running += 1;
    if (running > 1) overlap = true;
    calls.push(cmd);
    await new Promise<void>((resolve) => {
      release = resolve;
    });
    running -= 1;
    return absent;
  });
  renderScreen();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS * 3);
  });
  expect(count("device_status")).toBe(1);
  await act(async () => {
    release?.();
  });
  expect(overlap).toBe(false);
});

it("ne double pas un appel en vol quand on réessaie", async () => {
  let running = 0;
  let maxRunning = 0;
  const releases: (() => void)[] = [];
  mockIPC(async (cmd) => {
    if (cmd !== "device_status") return null;
    calls.push(cmd);
    running += 1;
    maxRunning = Math.max(maxRunning, running);
    await new Promise<void>((resolve) => releases.push(resolve));
    running -= 1;
    return absent;
  });
  renderScreen();
  await waitFor(() => expect(releases).toHaveLength(1));
  // Le premier appel n'est pas fini : un retour au premier plan demande un tour immédiat.
  await act(async () => {
    document.dispatchEvent(new Event("visibilitychange"));
  });
  expect(count("device_status")).toBe(1);
  await act(async () => {
    releases[0]?.();
  });
  await waitFor(() => expect(releases).toHaveLength(2));
  expect(count("device_status")).toBe(2);
  await act(async () => {
    releases[1]?.();
  });
  expect(maxRunning).toBe(1);
});

it("ne double pas un appel en vol quand l'écran est remonté", async () => {
  let running = 0;
  let maxRunning = 0;
  const releases: (() => void)[] = [];
  mockIPC(async (cmd) => {
    if (cmd !== "device_status") return null;
    calls.push(cmd);
    running += 1;
    maxRunning = Math.max(maxRunning, running);
    await new Promise<void>((resolve) => releases.push(resolve));
    running -= 1;
    return absent;
  });
  const first = renderScreen();
  await waitFor(() => expect(releases).toHaveLength(1));
  // Retour rapide sur l'écran : le premier appel n'est pas fini.
  first.unmount();
  renderScreen();
  await act(async () => {
    await Promise.resolve();
  });
  expect(count("device_status")).toBe(1);
  await act(async () => {
    releases[0]?.();
  });
  expect(await screen.findByText(/mode connexion USB/)).toBeInTheDocument();
  expect(maxRunning).toBe(1);
});

it("annonce ensemble la détection et le nombre d'histoires", async () => {
  status = detected();
  stories = report([story(A), story(B)]);
  renderScreen();
  await screen.findByRole("heading", { name: "Histoires sur la Flam (2)" });
  expect(screen.getByRole("status")).toHaveTextContent(
    "Flam détectée. 2 histoires sur la Flam",
  );
});

it("signale une liste tronquée", async () => {
  status = detected();
  stories = report([story(A)], { truncated: true, limit: 5000 });
  renderScreen();
  expect(
    await screen.findByText(/seules les 5000 premières histoires/),
  ).toBeInTheDocument();
});

it("affiche la version et l'état du firmware sur la carte de la Flam", async () => {
  status = detected();
  renderScreen();
  expect(
    await screen.findByText("2.4.2 (2.x) · en test, sauvegarde confirmée"),
  ).toBeInTheDocument();
});

it("affiche un refus d'écrire avec sa raison et la version", async () => {
  status = detected({
    firmware: "3.0.0",
    firmware_status: firmwareV2({
      version: "3.0.0",
      family: "unsupported",
      trust: "not_applicable",
      coherence: { kind: "not_checked" },
      write_block: { kind: "unsupported_firmware", version: "3.0.0" },
    }),
  });
  renderScreen();
  expect(
    await screen.findByText("3.0.0 (non pris en charge) · écriture bloquée"),
  ).toBeInTheDocument();
  expect(screen.getByText("Écriture sur cette Flam bloquée")).toBeInTheDocument();
  expect(screen.getByText(/Firmware 3.0.0 non pris en charge/)).toBeInTheDocument();
  // Rien n'est écrit, pas même la reprise.
  expect(calls).not.toContain("device_recover_transfers");
});

it("Flam 2.x non standard : confirmation de sauvegarde renforcée, puis écriture", async () => {
  status = detected({
    firmware_status: firmwareV2({
      mdf_size: 128,
      header: "02 00",
      layout_standard: false,
      trust: "unconfirmed",
      write_block: { kind: "confirmation_required", version: "2.4.2", standard: false },
    }),
  });
  renderScreen();
  expect(
    await screen.findByText(
      "2.4.2 (2.x) · en test, confirmation à donner · fichier .mdf non standard",
    ),
  ).toBeInTheDocument();
  expect(screen.getByText(/128 octets, en-tête 02 00/)).toBeInTheDocument();
  const confirm = screen.getByRole("button", { name: "Confirmer" });
  expect(confirm).toBeDisabled();
  expect(calls).not.toContain("device_recover_transfers");
  fireEvent.click(
    screen.getByLabelText("Firmware 2.x en test : j'ai sauvegardé ma Flam"),
  );
  expect(confirm).toBeEnabled();
  fireEvent.click(confirm);
  expect(
    await screen.findByText("2.4.2 (2.x) · en test, sauvegarde confirmée"),
  ).toBeInTheDocument();
  expect(calls).toContain("device_confirm_firmware");
  // L'écriture devient possible : la reprise au branchement a lieu, puis la liste est relue.
  await waitFor(() => expect(calls).toContain("device_recover_transfers"));
  await waitFor(() =>
    expect(calls.filter((c) => c === "device_list_stories").length).toBeGreaterThan(1),
  );
  expect(screen.queryByRole("button", { name: "Confirmer" })).not.toBeInTheDocument();
});

it("Flam 2.x aux clés incohérentes : envoi bloqué, reprise et retrait possibles", async () => {
  status = detected({
    firmware_status: firmwareV2({
      write_block: { kind: "keys_contradiction", version: "2.4.2" },
    }),
  });
  renderScreen();
  expect(
    await screen.findByText("2.4.2 (2.x) · envoi bloqué (clés incohérentes)"),
  ).toBeInTheDocument();
  expect(screen.getByText("Envoi sur cette Flam bloqué")).toBeInTheDocument();
  await waitFor(() => expect(calls).toContain("device_recover_transfers"));
});

it("Flam 2.x incohérente et non confirmée : la confirmation reste proposée", async () => {
  status = detected({
    firmware_status: firmwareV2({
      trust: "unconfirmed",
      write_block: { kind: "keys_contradiction", version: "2.4.2" },
    }),
  });
  renderScreen();
  expect(await screen.findByText("Envoi sur cette Flam bloqué")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Confirmer" })).toBeDisabled();
  // Rien n'est écrit avant la confirmation, pas même la reprise.
  expect(calls).not.toContain("device_recover_transfers");
  fireEvent.click(
    screen.getByLabelText("Firmware 2.x en test : j'ai sauvegardé ma Flam"),
  );
  fireEvent.click(screen.getByRole("button", { name: "Confirmer" }));
  await waitFor(() => expect(calls).toContain("device_confirm_firmware"));
});
