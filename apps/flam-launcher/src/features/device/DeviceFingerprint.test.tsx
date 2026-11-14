import { firmwareV1, firmwareV2 } from "../../test/firmware";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it } from "vitest";
import type { DeviceStatus, DeviceStories, DeviceStory } from "../../api/device";
import { ToastProvider } from "../../components";
import { tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { DeviceProvider } from "./DeviceProvider";
import { DeviceScreen } from "./DeviceScreen";
import { deviceKeyOf, expectedOfKey } from "./deviceKey";
import { SendToDeviceButton } from "./SendToDeviceButton";
import { TransferPanel } from "./TransferPanel";

// Empreinte de l'appareil (#609) : une écriture ne part que pour la Flam affichée au moment du
// clic. Le nom de volume est le même sur toutes les Flam : sans l'empreinte, une autre Flam qui
// reprend la même lettre de lecteur entre deux relevés ne serait pas distinguée.

tolerateLoadInAsyncUtils();

const MB = 1024 * 1024;
const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const C = "33333333-3333-4333-8333-333333333333";
const D = "44444444-4444-4444-8444-444444444444";

/** Empreintes opaques de deux Flam différentes, toutes deux en `E:\` sous le nom « FLAM ». */
const FLAM_1 = "a1".repeat(32);
const FLAM_2 = "b2".repeat(32);

const library = [
  { uuid: A, title: "Capi", size: 40 * MB },
  { uuid: B, title: "Les trois ours", size: 40 * MB },
];

function detected(fingerprint: string | null): DeviceStatus {
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
      fingerprint,
    },
  };
}

function deviceStory(uuid: string, title: string): DeviceStory {
  return {
    uuid,
    short_id: uuid.slice(0, 8),
    placement: "visible",
    title,
    in_library: false,
    library_uuids: [],
    missing: false,
  };
}

function report(stories: DeviceStory[]): DeviceStories {
  return {
    stories,
    visible_index: "valid",
    hidden_index: "valid",
    str_unreadable: false,
    truncated: false,
    limit: 5000,
  };
}

/** Message du backend quand l'empreinte transmise n'est pas celle de la Flam branchée. */
const REFUSAL =
  "la Flam branchée n'est plus celle affichée au moment de la demande (fichier .mdf différent) : rien n'a été écrit ; vérifiez la Flam branchée, puis recommencez";

/** Ce que l'interface a reçu au dernier relevé. */
let status: DeviceStatus;
/** Retient les réponses de `device_status` (la scrutation ne voit plus rien). */
let statusGate: Promise<void> | null;
/** Empreinte de la Flam réellement branchée : le faux backend refuse toute autre, comme le vrai. */
let plugged: string;
let onDevice: DeviceStory[];
/** Écritures acceptées par le faux backend (commande et histoire). */
let written: string[];
let calls: { cmd: string; args: Record<string, unknown> }[];
/** Relâche les relevés retenus par `swapUnseen` (un relevé en vol est partagé entre les montages). */
let releaseStatus: () => void;

const callsOf = (cmd: string) => calls.filter((c) => c.cmd === cmd);

/**
 * Comme la garde du backend : refus `device_changed` avant toute écriture si la Flam attendue
 * (`expected`) n'est pas celle branchée, y compris une Flam attendue sans empreinte. Plus strict
 * que lui sur un point, pour voir un oubli de l'interface : une écriture sans `expected` est
 * refusée, sauf la reprise qui suit un changement de Flam (`unchecked`), seule écriture que
 * l'interface demande pour la Flam branchée quelle qu'elle soit.
 */
function guard(args: Record<string, unknown>, unchecked = false) {
  const expected = args.expected as { fingerprint: string | null } | undefined;
  if (expected === undefined) {
    if (unchecked) return;
    throw { kind: "validation", message: "écriture demandée sans Flam attendue" };
  }
  if (expected.fingerprint !== plugged) {
    throw { kind: "device_changed", message: REFUSAL };
  }
}

beforeEach(() => {
  window.localStorage.clear();
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  status = detected(FLAM_1);
  statusGate = null;
  releaseStatus = () => undefined;
  plugged = FLAM_1;
  onDevice = [deviceStory(C, "Déjà là"), deviceStory(D, "Le loup")];
  written = [];
  calls = [];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      switch (cmd) {
        case "device_status": {
          const gate = statusGate;
          return gate ? gate.then(() => status) : status;
        }
        case "device_recover_transfers":
          guard(args, true);
          return {
            incomplete_removed: 0,
            previous_restored: 0,
            updates_finished: 0,
            left_alone: 0,
          };
        case "device_list_stories":
          return report(onDevice);
        case "device_send_story":
          guard(args);
          written.push(`envoi ${String(args.uuid)}`);
          return {
            id: args.uuid,
            bytes: 1,
            files: 1,
            replaced: false,
            index_updated: true,
            cache_removed: true,
            carrier_readback: null,
            carrier_stored: null,
            recovery_failures: [],
            library_copy: null,
          };
        case "device_remove_story":
          guard(args);
          written.push(`retrait ${String(args.id)}`);
          onDevice = onDevice.filter((s) => s.uuid !== args.id);
          return {
            id: args.id,
            index_updated: true,
            folders_removed: 1,
            cleanup_pending: false,
          };
        case "device_reorder_stories":
          guard(args);
          written.push("ordre");
          return { changed: true, cache_removed: true, count: onDevice.length };
        case "device_carrier_state":
          return {
            warning_text: "Avertissement du mode carrier",
            warning_version: 1,
            consent: { text_version: 1, accepted_at: "2026-10-17T10:00:00Z" },
            firmware: "1.15.14",
            carrier_needed: true,
            stored_keys: false,
            candidates: [
              {
                uuid: C,
                title: "Histoire officielle",
                hidden: false,
                with_stored: false,
              },
            ],
          };
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
  releaseStatus();
  clearMocks();
  window.localStorage.clear();
});

function Harness() {
  return (
    <ToastProvider>
      <DeviceProvider intervalMs={20}>
        <main>
          <DeviceScreen
            libraryPanel={
              <ul aria-label="Bibliothèque">
                {library.map((story) => (
                  <li key={story.uuid}>
                    <SendToDeviceButton {...story} />
                  </li>
                ))}
              </ul>
            }
          />
        </main>
        <aside>
          <TransferPanel />
        </aside>
      </DeviceProvider>
    </ToastProvider>
  );
}

const sendButton = (title: string) =>
  screen.getByRole("button", { name: `Envoyer sur la Flam « ${title} »` });

async function ready() {
  await screen.findByRole("heading", { name: "Histoires sur la Flam (2)" });
  await waitFor(() =>
    expect(sendButton("Capi")).toHaveAttribute("aria-disabled", "false"),
  );
}

/** Laisse passer quelques scrutations et les suites des promesses. */
async function settle(ms = 120) {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, ms));
  });
}

/**
 * Une autre Flam prend la place de la première sous la même lettre, entre deux relevés : la
 * scrutation est retenue, l'interface affiche toujours la première. Rend de quoi la relâcher.
 */
function swapUnseen(): () => void {
  let release!: () => void;
  statusGate = new Promise<void>((resolve) => {
    release = resolve;
  });
  plugged = FLAM_2;
  status = detected(FLAM_2);
  releaseStatus = () => {
    statusGate = null;
    release();
  };
  return releaseStatus;
}

it("la clé d'appareil porte l'empreinte : même lettre et même nom, autre Flam, autre clé", () => {
  const first = deviceKeyOf(detected(FLAM_1).device);
  const second = deviceKeyOf(detected(FLAM_2).device);
  expect(first).not.toBeNull();
  expect(first).toBe(deviceKeyOf(detected(FLAM_1).device));
  expect(second).not.toBe(first);
  expect(expectedOfKey(first)).toEqual({ fingerprint: FLAM_1 });
  expect(expectedOfKey(second)).toEqual({ fingerprint: FLAM_2 });
  // `.mdf` non lu : pas d'empreinte, et une clé qui n'est celle d'aucune Flam lisible. La Flam
  // attendue est alors « sans empreinte », jamais « aucune comparaison ».
  const unreadable = deviceKeyOf(detected(null).device);
  expect(unreadable).not.toBe(first);
  expect(expectedOfKey(unreadable)).toEqual({ fingerprint: null });
  expect(deviceKeyOf(null)).toBeNull();
  expect(expectedOfKey(null)).toEqual({ fingerprint: null });
  expect(expectedOfKey(undefined)).toEqual({ fingerprint: null });
});

it("envoi manuel : l'empreinte de la Flam affichée au clic accompagne l'envoi et la reprise au branchement", async () => {
  render(<Harness />);
  await ready();
  expect(callsOf("device_recover_transfers")[0]?.args).toEqual({
    root: "E:\\",
    expected: { fingerprint: FLAM_1 },
  });
  fireEvent.click(sendButton("Capi"));
  await waitFor(() => expect(written).toEqual([`envoi ${A}`]));
  expect(callsOf("device_send_story")[0]?.args).toEqual({
    root: "E:\\",
    uuid: A,
    expected: { fingerprint: FLAM_1 },
  });
});

it("envoi manuel, autre Flam sous la même lettre entre le clic et l'envoi : rien n'est écrit, la file s'arrête", async () => {
  render(<Harness />);
  await ready();
  const release = swapUnseen();
  // L'interface affiche encore la première Flam : les clics valent pour elle.
  fireEvent.click(sendButton("Capi"));
  fireEvent.click(sendButton("Les trois ours"));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  expect(await within(panel).findByText(REFUSAL)).toBeInTheDocument();
  await waitFor(() => expect(within(panel).getByText("Annulée")).toBeInTheDocument());
  // Une seule demande, avec l'empreinte du clic ; la seconde histoire n'est pas tentée.
  expect(callsOf("device_send_story").map((c) => c.args)).toEqual([
    { root: "E:\\", uuid: A, expected: { fingerprint: FLAM_1 } },
  ]);
  expect(written).toEqual([]);
  // La reprise qui suit vaut pour la Flam branchée, quelle qu'elle soit : sans empreinte.
  await waitFor(() =>
    expect(callsOf("device_recover_transfers").at(-1)?.args).toEqual({ root: "E:\\" }),
  );
  release();
  await settle();
  expect(written).toEqual([]);
  expect(callsOf("device_send_story")).toHaveLength(1);
});

it("avertissement resté ouvert pendant que la Flam change : l'envoi garde l'empreinte du clic, pas celle du relevé courant", async () => {
  window.localStorage.clear();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog");
  // Pendant que le dialogue est ouvert, l'interface voit l'autre Flam sous la même lettre.
  plugged = FLAM_2;
  status = detected(FLAM_2);
  const polls = callsOf("device_status").length;
  await waitFor(() => expect(callsOf("device_status").length).toBeGreaterThan(polls + 2));
  await settle();
  fireEvent.click(within(dialog).getByRole("button", { name: "J'ai compris, envoyer" }));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  expect(await within(panel).findByText(REFUSAL)).toBeInTheDocument();
  expect(callsOf("device_send_story").map((c) => c.args)).toEqual([
    { root: "E:\\", uuid: A, expected: { fingerprint: FLAM_1 } },
  ]);
  expect(written).toEqual([]);
});

it("autre Flam vue sous la même lettre et le même nom : nouvelle reprise, liste relue, comme pour une autre lettre", async () => {
  render(<Harness />);
  await ready();
  const recoveries = callsOf("device_recover_transfers").length;
  const lists = callsOf("device_list_stories").length;
  plugged = FLAM_2;
  status = detected(FLAM_2);
  await waitFor(() =>
    expect(callsOf("device_recover_transfers")).toHaveLength(recoveries + 1),
  );
  expect(callsOf("device_recover_transfers").at(-1)?.args).toEqual({
    root: "E:\\",
    expected: { fingerprint: FLAM_2 },
  });
  await waitFor(() =>
    expect(callsOf("device_list_stories").length).toBeGreaterThan(lists),
  );
  // Les envois suivants portent l'empreinte de la Flam désormais affichée.
  await ready();
  fireEvent.click(sendButton("Capi"));
  await waitFor(() => expect(written).toEqual([`envoi ${A}`]));
  expect(callsOf("device_send_story")[0]?.args.expected).toEqual({ fingerprint: FLAM_2 });
});

it("retrait : l'empreinte de la Flam affichée à la demande accompagne le retrait ; une autre Flam n'est pas touchée", async () => {
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Retirer « Déjà là »" }));
  const dialog = await screen.findByRole("alertdialog");
  // L'autre Flam prend la lettre pendant la confirmation, sans que l'interface le voie : elle
  // peut porter une histoire de même UUID.
  const release = swapUnseen();
  fireEvent.click(within(dialog).getByRole("button", { name: "Retirer de la Flam" }));
  await waitFor(() => expect(callsOf("device_remove_story")).toHaveLength(1));
  expect(callsOf("device_remove_story")[0]?.args).toEqual({
    root: "E:\\",
    id: C,
    confirmed: true,
    expected: { fingerprint: FLAM_1 },
  });
  expect(await screen.findByText(new RegExp("rien n'a été écrit"))).toBeInTheDocument();
  expect(written).toEqual([]);
  release();
  await settle();
  expect(written).toEqual([]);
});

it("ordre : l'empreinte de la Flam dont l'ordre était affiché accompagne l'enregistrement", async () => {
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Modifier l'ordre" }));
  fireEvent.click(await screen.findByRole("button", { name: "Descendre « Déjà là »" }));
  const release = swapUnseen();
  fireEvent.click(screen.getByRole("button", { name: "Enregistrer l'ordre" }));
  await waitFor(() => expect(callsOf("device_reorder_stories")).toHaveLength(1));
  expect(callsOf("device_reorder_stories")[0]?.args).toEqual({
    root: "E:\\",
    order: [D, C],
    expected: { fingerprint: FLAM_1 },
  });
  await settle();
  expect(written).toEqual([]);
  release();
});

it("Flam affichée sans empreinte, puis une autre, lisible, sous la même lettre : l'envoi est refusé, jamais « sans comparaison »", async () => {
  // La Flam affichée n'a pas d'empreinte (son `.mdf` n'a pas été lu) ; l'état du firmware du
  // relevé laisse ici le clic possible, pour éprouver le dernier rempart.
  status = detected(null);
  render(<Harness />);
  await ready();
  // La reprise au branchement elle-même désigne une Flam sans empreinte.
  expect(callsOf("device_recover_transfers")[0]?.args).toEqual({
    root: "E:\\",
    expected: { fingerprint: null },
  });
  const release = swapUnseen();
  fireEvent.click(sendButton("Capi"));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  expect(await within(panel).findByText(REFUSAL)).toBeInTheDocument();
  expect(callsOf("device_send_story").map((c) => c.args)).toEqual([
    { root: "E:\\", uuid: A, expected: { fingerprint: null } },
  ]);
  expect(written).toEqual([]);
  release();
  await settle();
  expect(written).toEqual([]);
});

/** Flam 1.x (mode carrier), d'empreinte donnée. */
function detectedV1(fingerprint: string): DeviceStatus {
  const value = detected(fingerprint);
  value.device!.firmware = "1.15.14";
  value.device!.firmware_status = firmwareV1();
  return value;
}

/** Ouvre le dialogue des clés carrier pour « Capi », histoire officielle déjà proposée. */
async function openCarrierDialog() {
  status = detectedV1(FLAM_1);
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog", {
    name: "Flam en firmware 1.x : mode carrier",
  });
  await within(dialog).findByRole("radio", {
    name: "Histoire officielle « Histoire officielle »",
  });
  return dialog;
}

it("dialogue des clés carrier ouvert, autre Flam sous la même lettre entre deux relevés : l'envoi garde l'empreinte du clic, rien n'est écrit", async () => {
  const dialog = await openCarrierDialog();
  const release = swapUnseen();
  fireEvent.click(within(dialog).getByRole("button", { name: "Vérifier et envoyer" }));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  expect(await within(panel).findByText(REFUSAL)).toBeInTheDocument();
  expect(callsOf("device_send_story").map((c) => c.args)).toEqual([
    {
      root: "E:\\",
      uuid: A,
      carrier: { kind: "device", uuid: C },
      remember: true,
      expected: { fingerprint: FLAM_1 },
    },
  ]);
  expect(written).toEqual([]);
  release();
  await settle();
  expect(written).toEqual([]);
});

it("dialogue des clés carrier ouvert, autre Flam vue sous la même lettre : le dialogue se ferme, rien n'est envoyé", async () => {
  await openCarrierDialog();
  plugged = FLAM_2;
  status = detectedV1(FLAM_2);
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  await settle();
  expect(callsOf("device_send_story")).toHaveLength(0);
  expect(written).toEqual([]);
  // Un nouveau clic redemande les clés, pour la Flam désormais affichée.
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog", {
    name: "Flam en firmware 1.x : mode carrier",
  });
  fireEvent.click(
    await within(dialog).findByRole("button", { name: "Vérifier et envoyer" }),
  );
  await waitFor(() => expect(written).toEqual([`envoi ${A}`]));
  expect(callsOf("device_send_story")[0]?.args.expected).toEqual({ fingerprint: FLAM_2 });
});
