import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeviceStatus } from "../../api/device";
import type { LibraryEntry } from "../../api/library";
import { ToastProvider } from "../../components";
import { firmwareV2, TEST_FINGERPRINT } from "../../test/firmware";
import { makeEntry } from "../../test/entries";
import { StoriesContext, type StoriesApi } from "../stories/StoriesContext";
import { DeviceLibrarySend } from "../stories/DeviceLibrarySend";
import type { PlannedSend } from "./DeviceContext";
import { DeviceProvider } from "./DeviceProvider";
import { PlannedLine } from "./TransferPanel";

// Suppression d'une histoire en cours (#581) : « Envoyer sur la Flam » ne part pas, ni depuis
// « Ma Flam » (`DeviceLibrarySend`) ni depuis la ligne « envoi à confirmer » (`PlannedLine`).

const MB = 1024 * 1024;

const status: DeviceStatus = {
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
    fingerprint: TEST_FINGERPRINT,
  },
};

const source = makeEntry({ uuid: "ours", title: "Les trois ours" });
const ours: LibraryEntry = {
  uuid: "ours",
  version: 1,
  title: "Les trois ours",
  file: "ours-1.plain.pk",
  format: "flam",
  size: 2 * MB,
  installed_at: 1_700_000_000,
  converter: null,
  source,
};
const other: LibraryEntry = {
  ...ours,
  uuid: "autre",
  title: "Capi",
  file: "autre-1.plain.pk",
};

let sends: string[];

beforeEach(() => {
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  sends = [];
  mockIPC(
    (cmd, payload) => {
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
            stories: [],
            visible_index: "valid",
            hidden_index: "valid",
            str_unreadable: false,
            truncated: false,
            limit: 5000,
          };
        case "device_send_story":
          sends.push((payload as { uuid: string }).uuid);
          return new Promise(() => {});
        default:
          throw { kind: "unknown", message: cmd };
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  clearMocks();
});

// Seule la suppression de « Les trois ours » est en cours.
const stories = {
  installed: [ours, other],
  sourceChanged: () => false,
  removing: (uuid: string) => uuid === "ours",
} as unknown as StoriesApi;

function wrap(children: React.ReactNode) {
  return (
    <ToastProvider>
      <StoriesContext.Provider value={stories}>
        <DeviceProvider>{children}</DeviceProvider>
      </StoriesContext.Provider>
    </ToastProvider>
  );
}

const sendButton = (title: string) =>
  screen.findByRole("button", { name: `Envoyer sur la Flam « ${title} »` });

it("« Ma Flam » : l'envoi de l'histoire en cours de suppression ne part pas (#581)", async () => {
  render(wrap(<DeviceLibrarySend />));
  const free = await sendButton("Capi");
  await waitFor(() => expect(free).toHaveAttribute("aria-disabled", "false"));
  const locked = await sendButton("Les trois ours");
  expect(locked).toHaveAttribute("aria-disabled", "true");
  expect(locked).toHaveAccessibleDescription(/Suppression en cours\./);
  fireEvent.click(locked);
  fireEvent.click(free);
  await waitFor(() => expect(sends).toEqual(["autre"]));
});

function plan(uuid: string, title: string): PlannedSend {
  return {
    uuid,
    title,
    format: "flam",
    deviceKey: "E:\\u0000FLAM",
    state: "ready",
    size: 2 * MB,
    lost: false,
    unwatched: true,
  };
}

it("envoi à confirmer : l'histoire en cours de suppression ne part pas (#581)", async () => {
  const onConfirm = vi.fn();
  render(
    wrap(
      <ul>
        <PlannedLine
          plan={plan("autre", "Capi")}
          onDrop={() => {}}
          onConfirm={() => onConfirm("autre")}
        />
        <PlannedLine
          plan={plan("ours", "Les trois ours")}
          onDrop={() => {}}
          onConfirm={() => onConfirm("ours")}
        />
      </ul>,
    ),
  );
  const free = await sendButton("Capi");
  await waitFor(() => expect(free).toHaveAttribute("aria-disabled", "false"));
  const locked = await sendButton("Les trois ours");
  expect(locked).toHaveAttribute("aria-disabled", "true");
  expect(locked).toHaveAccessibleDescription(/Suppression en cours\./);
  fireEvent.click(locked);
  fireEvent.click(free);
  expect(onConfirm.mock.calls).toEqual([["autre"]]);
  const rows = screen.getAllByRole("listitem");
  expect(within(rows[1]!).getByText("Suppression en cours.")).toBeVisible();
});
