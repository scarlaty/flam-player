import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeviceStatus, DeviceStory, LibraryCopy } from "../../api/device";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { firmwareV2 } from "../../test/firmware";
import { tolerateLoadInAsyncUtils } from "../../test/timeouts";
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
  "L'histoire reste sur la Flam. Pour la renvoyer plus tard, il faudra la réinstaller.";
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
const count = (cmd: string) => calls.filter((c) => c.cmd === cmd).length;

beforeEach(() => {
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  outcomes = new Map();
  removeSent = null;
  removeGate = null;
  sendGate = null;
  calls = [];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      switch (cmd) {
        case "device_status":
          return detected();
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

function renderPanel() {
  onPcCopyChange.mockClear();
  return render(
    <ToastProvider>
      <DeviceProvider onPcCopyChange={onPcCopyChange}>
        <main>
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
  const button = await screen.findByRole("button", { name: "Retirer du PC" });
  expect(button).toHaveAccessibleDescription(HINT);
  expect(within(row("Capi")).getByText("Envoyée")).toBeVisible();
  // Rien n'est retiré sans clic (réglage désactivé).
  expect(count("library_remove_sent")).toBe(0);

  button.focus();
  fireEvent.click(button);
  await waitFor(() => expect(count("library_remove_sent")).toBe(1));
  expect(calls.find((c) => c.cmd === "library_remove_sent")?.args).toEqual({ uuid: A });
  const note = await screen.findByText(REMOVED);
  expect(screen.queryByRole("button", { name: "Retirer du PC" })).toBeNull();
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
  expect(screen.queryByRole("button", { name: "Retirer du PC" })).toBeNull();

  // Annulation.
  outcomes.set(A, { error: { kind: "cancelled", message: "annulé" } });
  await send("Capi");
  await waitFor(() => expect(within(row("Capi")).getByText("Annulée")).toBeVisible());
  expect(screen.queryByRole("button", { name: "Retirer du PC" })).toBeNull();
  expect(onPcCopyChange).not.toHaveBeenCalled();
});

it("relecture carrier illisible : échec affiché, rien n'est proposé", async () => {
  outcomes.set(A, { ok: undefined, readback: false });
  renderPanel();
  await send("Capi");
  await waitFor(() => expect(within(row("Capi")).getByText("Échec")).toBeVisible());
  expect(screen.queryByRole("button", { name: "Retirer du PC" })).toBeNull();
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
  await within(row("Les trois ours")).findByRole("button", { name: "Retirer du PC" });
  expect(within(row("Capi")).queryByRole("button", { name: "Retirer du PC" })).toBeNull();
  await expectNoAxeViolations(document.body);
});

it("retrait automatique (réglage) : la ligne le dit, l'application est prévenue", async () => {
  outcomes.set(A, { ok: { status: "removed" } });
  renderPanel();
  await send("Capi");
  expect(await screen.findByText(REMOVED)).toBeVisible();
  expect(screen.queryByRole("button", { name: "Retirer du PC" })).toBeNull();
  expect(onPcCopyChange).toHaveBeenCalledWith("Capi", { status: "removed" });
  expect(count("library_remove_sent")).toBe(0);
});

it("retrait automatique impossible : envoi réussi, erreur affichée, bouton reproposé", async () => {
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

  // Nouvel essai en échec : le bouton reste et garde le focus.
  removeSent = { kind: "validation", message: "installation en cours" };
  let release!: () => void;
  removeGate = new Promise((resolve) => (release = resolve));
  const button = screen.getByRole("button", { name: "Retirer du PC" });
  button.focus();
  fireEvent.click(button);
  const busy = await screen.findByRole("button", { name: "Retrait du PC…" });
  expect(busy).toHaveAttribute("aria-busy", "true");
  // Second clic pendant le retrait : ignoré.
  fireEvent.click(busy);
  await act(async () => release());
  expect(
    await screen.findByText(
      "L'histoire est envoyée, mais n'a pas pu être retirée du PC : installation en cours",
    ),
  ).toBeVisible();
  expect(count("library_remove_sent")).toBe(1);
  expect(screen.getByRole("button", { name: "Retirer du PC" })).toHaveFocus();
});
