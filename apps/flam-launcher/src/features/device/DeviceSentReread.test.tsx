import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import type { DeviceStatus, DeviceStories, DeviceStory } from "../../api/device";
import { ToastProvider } from "../../components";
import { firmwareV2, TEST_FINGERPRINT } from "../../test/firmware";
import { tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { DeviceProvider } from "./DeviceProvider";
import { DeviceScreen } from "./DeviceScreen";
import { SendToDeviceButton } from "./SendToDeviceButton";

tolerateLoadInAsyncUtils();

// Après « Remplacer sur la Flam » (l'UUID de la Flam est gardé), titre et vignette lus avant
// l'envoi sont relus : câblage `sentStamps` (clé `uuid:gen`) puis `useDeviceStoryTitles` et
// `ThumbnailLoader.forgetSent`, depuis `DeviceScreen` (#581).

const MB = 1024 * 1024;
const LIB = "22222222-2222-4222-8222-222222222222";
const ON_FLAM = "55555555-5555-4555-8555-555555555555";

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

function story(): DeviceStory {
  return {
    uuid: ON_FLAM,
    short_id: ON_FLAM.slice(0, 8),
    placement: "visible",
    title: null,
    in_library: true,
    library_uuids: [LIB],
    missing: false,
  };
}

const report: DeviceStories = {
  stories: [story()],
  visible_index: "valid",
  hidden_index: "valid",
  str_unreadable: false,
  truncated: false,
  limit: 5000,
};

let title: string;
let thumb: string;
let titleReads: string[][];
let thumbReads: number;

beforeEach(() => {
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  title = "Ancien titre";
  thumb = "data:image/png;base64,QUJD";
  titleReads = [];
  thumbReads = 0;
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
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
          return report;
        case "device_story_titles": {
          const uuids = args.uuids as string[];
          titleReads.push(uuids);
          return {
            titles: uuids.map((uuid) => ({ uuid, title, format: "flam" })),
            interrupted: false,
            keys: true,
            keys_problem: null,
          };
        }
        case "device_story_thumbnail":
          thumbReads += 1;
          return { thumb, busy: false };
        case "device_send_story":
          // Le remplacement garde l'UUID de la Flam ; la Flam a maintenant un autre contenu.
          title = "Nouveau titre";
          thumb = "data:image/png;base64,REVG";
          return { id: ON_FLAM, replaced: true };
        case "device_official_titles":
          return { titles: [] };
        case "library_thumbnail":
          return "data:image/png;base64,AAAA";
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

it("relit le titre et la vignette de la Flam après « Remplacer sur la Flam » (#581)", async () => {
  render(
    <ToastProvider>
      <DeviceProvider>
        <DeviceScreen
          libraryPanel={
            <SendToDeviceButton uuid={LIB} title="Les trois ours" size={40 * MB} />
          }
        />
      </DeviceProvider>
    </ToastProvider>,
  );
  const list = await screen.findByRole("list", {
    name: "Histoires installées sur la Flam",
  });
  expect(await within(list).findByText("Ancien titre")).toBeInTheDocument();
  await waitFor(() => expect(list.querySelector("img")).not.toBeNull());
  expect(list.querySelector("img")).toHaveAttribute("src", "data:image/png;base64,QUJD");
  expect(titleReads).toHaveLength(1);
  const thumbsBefore = thumbReads;

  const replace = await screen.findByRole("button", {
    name: "Remplacer sur la Flam « Les trois ours »",
  });
  await waitFor(() => expect(replace).toHaveAttribute("aria-disabled", "false"));
  fireEvent.click(replace);

  expect(await within(list).findByText("Nouveau titre")).toBeInTheDocument();
  expect(titleReads.flat().filter((uuid) => uuid === ON_FLAM)).toHaveLength(2);
  await waitFor(() =>
    expect(list.querySelector("img")).toHaveAttribute(
      "src",
      "data:image/png;base64,REVG",
    ),
  );
  expect(thumbReads).toBeGreaterThan(thumbsBefore);
});
