import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeviceStatus, DeviceStory, LibraryCopy } from "../../api/device";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { firmwareV2 } from "../../test/firmware";
import { tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { useDevice } from "./DeviceContext";
import { DeviceProvider } from "./DeviceProvider";
import { SendToDeviceButton } from "./SendToDeviceButton";
import { TransferPanel } from "./TransferPanel";

// « Retirer du PC » après un envoi réussi (#609) : proposé d'après le rapport du backend
// (`library_copy`), jamais après un échec ou une annulation.

tolerateLoadInAsyncUtils();

const MB = 1024 * 1024;
const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const library = [
  { uuid: A, title: "Capi", size: 4 * MB },
  { uuid: B, title: "Les trois ours", size: 4 * MB },
];

const HINT =
  "L'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller. Avant de débrancher la Flam, utilisez « Éjecter la Flam ».";
/** Bouton « Retirer du PC » (nom propre à l'histoire). */
const PC = /^Retirer du PC/;
const REMOVED =
  "Retirée de Mes histoires : l'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller.";

function detected(): DeviceStatus {
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
    },
  };
}

type Outcome =
  | { ok: LibraryCopy | null | undefined; readback?: boolean }
  | { error: { kind: string; message: string } };

let outcomes: Map<string, Outcome>;
let removeSent: { kind: string; message: string } | null;
let removeGate: Promise<void> | null;
let sendGate: Promise<void> | null;
let calls: { cmd: string; args: Record<string, unknown> }[];
let status: DeviceStatus;
const count = (cmd: string) => calls.filter((c) => c.cmd === cmd).length;

beforeEach(() => {
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  outcomes = new Map();
  removeSent = null;
  removeGate = null;
  sendGate = null;
  calls = [];
  status = detected();
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
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
          return {
            stories: [] as DeviceStory[],
            visible_index: "valid",
            hidden_index: "valid",
            str_unreadable: false,
            truncated: false,
            limit: 5000,
          };
        case "device_send_story":
          return (sendGate ?? Promise.resolve()).then(() => {
            const outcome = outcomes.get(String(args.uuid));
            if (!outcome) throw { kind: "io", message: "non prévu" };
            if ("error" in outcome) throw outcome.error;
            return {
              id: args.uuid,
              bytes: 1,
              files: 1,
              replaced: false,
              index_updated: true,
              cache_removed: true,
              carrier_readback: outcome.readback ?? null,
              carrier_stored: null,
              recovery_failures: [],
              library_copy: outcome.ok,
            };
          });
        case "device_remove_story":
          return {
            id: args.id,
            index_updated: true,
            folders_removed: 1,
            cleanup_pending: false,
          };
        case "library_remove_sent":
          return (removeGate ?? Promise.resolve()).then(() => {
            if (removeSent) throw removeSent;
            return null;
          });
        default:
          throw { kind: "unknown", message: cmd };
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  clearMocks();
  window.localStorage.clear();
});

const onPcCopyChange = vi.fn();

/** Retrait de la Flam (« Ma Flam »), sans dialogue. */
function RemoveFromFlam({ uuid }: { uuid: string }) {
  const { removeFromDevice, deviceKey } = useDevice();
  return (
    <button type="button" onClick={() => void removeFromDevice(uuid, deviceKey)}>
      Retirer de la Flam
    </button>
  );
}

function renderPanel() {
  onPcCopyChange.mockClear();
  return render(
    <ToastProvider>
      <DeviceProvider onPcCopyChange={onPcCopyChange} intervalMs={50}>
        <main>
          <RemoveFromFlam uuid={A} />
          <ul aria-label="Bibliothèque">
            {library.map((story) => (
              <li key={story.uuid}>
                <SendToDeviceButton {...story} />
              </li>
            ))}
          </ul>
        </main>
        <aside>
          <TransferPanel />
        </aside>
      </DeviceProvider>
    </ToastProvider>,
  );
}

async function send(title: string) {
  const button = await screen.findByRole("button", {
    name: `Envoyer sur la Flam « ${title} »`,
  });
  await waitFor(() => expect(button).toHaveAttribute("aria-disabled", "false"));
  fireEvent.click(button);
}

const row = (title: string) =>
  Array.from(
    screen.getByRole("list", { name: "File d'envoi" }).querySelectorAll("li"),
  ).find((li) => li.textContent?.includes(title)) as HTMLElement;

it("après un envoi réussi : « Retirer du PC », puis l'histoire reste sur la Flam", async () => {
  outcomes.set(A, { ok: { status: "kept" } });
  renderPanel();
  await send("Capi");
  const button = await screen.findByRole("button", { name: PC });
  expect(button).toHaveAccessibleDescription(HINT);
  // Nom propre à l'histoire, distinct du retrait de la file.
  expect(button).toHaveAccessibleName("Retirer du PC : « Capi »");
  expect(within(row("Capi")).getByText("Envoyée")).toBeVisible();
  // Rien n'est retiré sans clic (réglage désactivé).
  expect(count("library_remove_sent")).toBe(0);

  button.focus();
  fireEvent.click(button);
  await waitFor(() => expect(count("library_remove_sent")).toBe(1));
  expect(calls.find((c) => c.cmd === "library_remove_sent")?.args).toEqual({
    root: "E:\\",
    uuid: A,
  });
  const note = await screen.findByText(REMOVED);
  expect(screen.queryByRole("button", { name: PC })).toBeNull();
  // Le bouton a disparu : le focus passe au texte de la ligne, pas au corps du document.
  await waitFor(() => expect(note).toHaveFocus());
  expect(onPcCopyChange).toHaveBeenCalledWith("Capi", { status: "removed" });
});

it("pas de « Retirer du PC » après un échec, une annulation, ou sans proposition du backend", async () => {
  outcomes.set(A, { error: { kind: "io", message: "Flam débranchée" } });
  outcomes.set(B, { ok: null });
  renderPanel();
  await send("Capi");
  await screen.findByText("Flam débranchée");
  await send("Les trois ours");
  await waitFor(() =>
    expect(within(row("Les trois ours")).getByText("Envoyée")).toBeVisible(),
  );
  expect(screen.queryByRole("button", { name: PC })).toBeNull();

  // Annulation.
  outcomes.set(A, { error: { kind: "cancelled", message: "annulé" } });
  await send("Capi");
  await waitFor(() => expect(within(row("Capi")).getByText("Annulée")).toBeVisible());
  expect(screen.queryByRole("button", { name: PC })).toBeNull();
  expect(onPcCopyChange).not.toHaveBeenCalled();
});

it("relecture carrier illisible : échec affiché, rien n'est proposé", async () => {
  outcomes.set(A, { ok: undefined, readback: false });
  renderPanel();
  await send("Capi");
  await waitFor(() => expect(within(row("Capi")).getByText("Échec")).toBeVisible());
  expect(screen.queryByRole("button", { name: PC })).toBeNull();
});

it("deux envois : chaque histoire a son propre état", async () => {
  outcomes.set(A, { error: { kind: "io", message: "coupure" } });
  outcomes.set(B, { ok: { status: "kept" } });
  let release!: () => void;
  sendGate = new Promise((resolve) => (release = resolve));
  renderPanel();
  // Les deux histoires dans la même file : la première échoue, la seconde réussit.
  await send("Capi");
  await send("Les trois ours");
  await act(async () => release());
  await within(row("Les trois ours")).findByRole("button", { name: PC });
  expect(within(row("Capi")).queryByRole("button", { name: PC })).toBeNull();
  await expectNoAxeViolations(document.body);
});

it("retrait automatique (réglage) : la ligne le dit, l'application est prévenue", async () => {
  outcomes.set(A, { ok: { status: "removed" } });
  renderPanel();
  await send("Capi");
  expect(await screen.findByText(REMOVED)).toBeVisible();
  expect(screen.queryByRole("button", { name: PC })).toBeNull();
  expect(onPcCopyChange).toHaveBeenCalledWith("Capi", { status: "removed" });
  expect(count("library_remove_sent")).toBe(0);
});

it("retrait automatique impossible, puis refus du backend : plus de bouton, message gardé", async () => {
  const failure = {
    status: "remove_failed",
    message: "« Capi » est en cours de lecture",
  } as const;
  outcomes.set(A, { ok: failure });
  renderPanel();
  await send("Capi");
  expect(
    await screen.findByText(
      "L'histoire est envoyée, mais n'a pas pu être retirée du PC : « Capi » est en cours de lecture",
    ),
  ).toBeVisible();
  expect(within(row("Capi")).getByText("Envoyée")).toBeVisible();
  expect(onPcCopyChange).toHaveBeenCalledWith("Capi", failure);

  // Nouvel essai refusé (histoire plus sur la Flam) : le bouton disparaît, le message reste et
  // prend le focus.
  removeSent = {
    kind: "not_removable",
    message:
      "l'histoire n'a pas été retrouvée sur la Flam : elle n'a pas été retirée du PC",
  };
  let release!: () => void;
  removeGate = new Promise((resolve) => (release = resolve));
  const button = screen.getByRole("button", { name: PC });
  button.focus();
  fireEvent.click(button);
  const busy = await screen.findByRole("button", { name: "Retrait du PC…" });
  expect(busy).toHaveAttribute("aria-busy", "true");
  // Second clic pendant le retrait : ignoré.
  fireEvent.click(busy);
  await act(async () => release());
  const note = await screen.findByText(
    "L'histoire n'a pas été retirée du PC : l'histoire n'a pas été retrouvée sur la Flam : elle n'a pas été retirée du PC",
  );
  expect(count("library_remove_sent")).toBe(1);
  expect(screen.queryByRole("button", { name: PC })).toBeNull();
  await waitFor(() => expect(note).toHaveFocus());
});

it("écriture non confirmée : gardée, bouton proposé, l'application prévenue", async () => {
  outcomes.set(A, { ok: { status: "kept", unconfirmed: true, auto: true } });
  renderPanel();
  await send("Capi");
  expect(await screen.findByRole("button", { name: PC })).toBeVisible();
  expect(
    screen.getByText(
      "L'histoire est bien sur la Flam. Sa copie est gardée sur ce PC par prudence : si vous la retirez, éjectez ensuite la Flam avec « Éjecter la Flam » avant de la débrancher.",
    ),
  ).toBeVisible();
  expect(onPcCopyChange).toHaveBeenCalledWith("Capi", { status: "kept_unconfirmed" });
});

it("histoire retirée de la Flam : « Retirer du PC » n'est plus proposé", async () => {
  outcomes.set(A, { ok: { status: "kept" } });
  renderPanel();
  await send("Capi");
  await screen.findByRole("button", { name: PC });
  fireEvent.click(screen.getByRole("button", { name: "Retirer de la Flam" }));
  await waitFor(() => expect(count("device_remove_story")).toBe(1));
  await waitFor(() => expect(screen.queryByRole("button", { name: PC })).toBeNull());
  expect(within(row("Capi")).getByText("Envoyée")).toBeVisible();
  expect(count("library_remove_sent")).toBe(0);
});

it("autre Flam branchée : « Retirer du PC » n'est plus proposé", async () => {
  outcomes.set(A, { ok: { status: "kept" } });
  renderPanel();
  await send("Capi");
  await screen.findByRole("button", { name: PC });
  const other = detected();
  if (other.device) other.device.volume_label = "AUTRE";
  status = other;
  await waitFor(() => expect(screen.queryByRole("button", { name: PC })).toBeNull());
  expect(count("library_remove_sent")).toBe(0);
});

it("refus passager (lecture, installation, Flam occupée) : bouton reproposé avec le message", async () => {
  outcomes.set(A, { ok: { status: "kept" } });
  renderPanel();
  await send("Capi");
  for (const [kind, message] of [
    [
      "player_busy",
      "« Capi » est en cours de lecture : fermez flam-player avant de la désinstaller",
    ],
    [
      "validation",
      "une installation de cette histoire est en cours : elle n'a pas été retirée du PC",
    ],
    [
      "validation",
      "la Flam est occupée (un transfert) : réessayez quand l'opération est terminée",
    ],
  ] as const) {
    removeSent = { kind, message };
    const button = await screen.findByRole("button", { name: PC });
    button.focus();
    fireEvent.click(button);
    expect(
      await screen.findByText(
        `L'histoire est envoyée, mais n'a pas pu être retirée du PC : ${message}`,
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: PC })).toHaveFocus();
  }
  // Une fois la lecture finie, le retrait réussit.
  removeSent = null;
  fireEvent.click(screen.getByRole("button", { name: PC }));
  expect(await screen.findByText(REMOVED)).toBeVisible();
  expect(count("library_remove_sent")).toBe(4);
});

it("refus définitif (autre Flam, envoi inconnu) : plus de bouton", async () => {
  for (const kind of ["not_removable", "not_found"]) {
    outcomes.set(A, { ok: { status: "kept" } });
    const { unmount } = renderPanel();
    await send("Capi");
    removeSent = { kind, message: "refus définitif" };
    fireEvent.click(await screen.findByRole("button", { name: PC }));
    expect(
      await screen.findByText("L'histoire n'a pas été retirée du PC : refus définitif"),
    ).toBeVisible();
    expect(screen.queryByRole("button", { name: PC })).toBeNull();
    unmount();
  }
});

it("réglage désactivé, écriture non confirmée : consigne d'éjection sur la ligne", async () => {
  outcomes.set(A, { ok: { status: "kept", unconfirmed: true } });
  renderPanel();
  await send("Capi");
  expect(await screen.findByRole("button", { name: PC })).toBeVisible();
  expect(
    within(row("Capi")).getByText(/si vous la retirez, éjectez ensuite la Flam/),
  ).toBeVisible();
  // Réglage désactivé : pas de notification.
  expect(onPcCopyChange).not.toHaveBeenCalled();
});
