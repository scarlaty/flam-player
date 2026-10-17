import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeviceStatus, DeviceStories, DeviceStory } from "../../api/device";
import { expectNoAxeViolations } from "../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { DeviceScreen } from "./DeviceScreen";
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

let status: DeviceStatus;
let stories: DeviceStories | null;
let failures: Record<string, { kind: string; message: string } | undefined>;
let calls: string[];

beforeEach(() => {
  status = absent;
  stories = report([]);
  failures = {};
  calls = [];
  mockIPC((cmd) => {
    calls.push(cmd);
    const failure = failures[cmd];
    if (failure) throw failure;
    switch (cmd) {
      case "device_status":
        return status;
      case "device_list_stories":
        return stories;
      case "library_thumbnail":
        return "data:image/png;base64,AAAA";
      default:
        throw { kind: "unknown", message: cmd };
    }
  });
});

afterEach(() => {
  clearMocks();
  vi.useRealTimers();
});

const count = (cmd: string) => calls.filter((c) => c === cmd).length;

it("invite à activer le mode USB quand la Flam n'est pas détectée", async () => {
  const { container } = render(<DeviceScreen />);
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

it(
  "affiche la Flam détectée, son espace et ses histoires",
  async () => {
    status = detected();
    stories = report([
      story(A, { title: "Capi et la rivière", in_library: true }),
      story(B, { missing: true }),
      story(C, { placement: "hidden" }),
      story("44444444-4444-4444-8444-444444444444", { placement: "unlisted" }),
    ]);
    const { container } = render(<DeviceScreen />);

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
  render(<DeviceScreen />);
  expect(await screen.findByText("Aucune histoire sur la Flam")).toBeInTheDocument();
  expect(screen.getByRole("status")).toHaveTextContent("0 histoire sur la Flam");
  expect(
    screen.getByRole("heading", { name: "Histoires sur la Flam (0)" }),
  ).toBeInTheDocument();
});

it("affiche les avertissements de la Flam et des index", async () => {
  status = detected({
    warnings: [{ kind: "missing_str_dir" }, { kind: "unexpected_mdf_size", size: 130 }],
    mdf_problem: { kind: "truncated", size: 10, expected_min: 126 },
    backup_failed: true,
  });
  status.device_count = 2;
  stories = report([story(A)], {
    visible_index: "damaged",
    hidden_index: "unreadable",
    str_unreadable: true,
  });
  const { container } = render(<DeviceScreen />);
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
  render(<DeviceScreen />);
  expect(await screen.findByText("Espace disque indisponible")).toBeInTheDocument();
  expect(screen.getByText("(sans nom) (E:\\)")).toBeInTheDocument();
  expect(screen.queryByRole("progressbar")).toBeNull();
});

it("affiche l'erreur de détection avec un bouton Réessayer", async () => {
  failures.device_status = { kind: "io", message: "disque en panne" };
  const { container } = render(<DeviceScreen />);
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
  render(<DeviceScreen />);
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
  render(<DeviceScreen />);
  await screen.findByText("Aucune histoire sur la Flam");
  stories = report([story(A)]);
  fireEvent.click(screen.getByRole("button", { name: "Actualiser la liste" }));
  expect(
    await screen.findByRole("heading", { name: "Histoires sur la Flam (1)" }),
  ).toBeInTheDocument();
  expect(count("device_list_stories")).toBe(2);
});

it("rend le focus au titre quand « Réessayer » trouve la Flam", async () => {
  render(<DeviceScreen />);
  const retry = await screen.findByRole("button", { name: "Réessayer" });
  status = detected();
  fireEvent.click(retry);
  const heading = await screen.findByRole("heading", { name: "Flam détectée" });
  await waitFor(() => expect(heading).toHaveFocus());
  expect(screen.getByRole("status")).toHaveTextContent(/Flam détectée|histoire/);
});

it("garde le focus dans la liste quand on actualise ou réessaie", async () => {
  status = detected();
  render(<DeviceScreen />);
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
  render(<DeviceScreen />);
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
  render(<DeviceScreen />);
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

it("prévoit les emplacements des actions d'envoi et de retrait", async () => {
  status = detected();
  stories = report([story(A, { title: "Capi" })]);
  render(
    <DeviceScreen
      toolbar={<button type="button">Envoyer</button>}
      storyActions={(s) => <button type="button">{`Retirer ${s.short_id}`}</button>}
    />,
  );
  expect(await screen.findByRole("button", { name: "Envoyer" })).toBeInTheDocument();
  expect(
    await screen.findByRole("button", { name: "Retirer 11111111" }),
  ).toBeInTheDocument();
});

it("ne sollicite que des commandes de lecture", async () => {
  status = detected();
  stories = report([story(A, { in_library: true, title: "Capi" })]);
  render(<DeviceScreen />);
  await screen.findByText("Capi");
  await waitFor(() => expect(count("library_thumbnail")).toBe(1));
  expect(new Set(calls)).toEqual(
    new Set(["device_status", "device_list_stories", "library_thumbnail"]),
  );
});

it("scrute toutes les 2 s et constate le débranchement", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  status = detected();
  render(<DeviceScreen />);
  await screen.findByRole("heading", { name: "Flam détectée" });
  expect(count("device_status")).toBe(1);

  status = absent;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
  });
  expect(count("device_status")).toBe(2);
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
});

it("arrête la scrutation au démontage", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  status = detected();
  const { unmount } = render(<DeviceScreen />);
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
  render(<DeviceScreen />);
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
  render(<DeviceScreen />);
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

it("annonce ensemble la détection et le nombre d'histoires", async () => {
  status = detected();
  stories = report([story(A), story(B)]);
  render(<DeviceScreen />);
  await screen.findByRole("heading", { name: "Histoires sur la Flam (2)" });
  expect(screen.getByRole("status")).toHaveTextContent(
    "Flam détectée. 2 histoires sur la Flam",
  );
});

it("signale une liste tronquée", async () => {
  status = detected();
  stories = report([story(A)], { truncated: true, limit: 5000 });
  render(<DeviceScreen />);
  expect(
    await screen.findByText(/seules les 5000 premières histoires/),
  ).toBeInTheDocument();
});
