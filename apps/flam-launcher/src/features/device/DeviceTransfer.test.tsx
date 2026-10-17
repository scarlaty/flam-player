import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type {
  DeviceStatus,
  DeviceStories,
  DeviceStory,
  EjectOutcome,
} from "../../api/device";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { DeviceProvider } from "./DeviceProvider";
import { DeviceScreen } from "./DeviceScreen";
import { POLL_INTERVAL_MS } from "./useDeviceStatus";
import { SendToDeviceButton } from "./SendToDeviceButton";
import { TransferPanel } from "./TransferPanel";

tolerateLoadInAsyncUtils();

const MB = 1024 * 1024;
const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const C = "33333333-3333-4333-8333-333333333333";
const D = "44444444-4444-4444-8444-444444444444";

const library = [
  { uuid: A, title: "Capi", size: 40 * MB },
  { uuid: B, title: "Les trois ours", size: 40 * MB },
  { uuid: C, title: "Déjà là", size: 10 * MB },
  { uuid: D, title: "Énorme", size: 900 * MB },
];

const absent: DeviceStatus = {
  detected: false,
  hint: null,
  device: null,
  device_count: 0,
};

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
    },
  };
}

function deviceStory(uuid: string, title: string | null = null): DeviceStory {
  return {
    uuid,
    short_id: uuid.slice(0, 8),
    placement: "visible",
    title,
    in_library: false,
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

type Deferred = { resolve: (value: unknown) => void; reject: (reason: unknown) => void };

let status: DeviceStatus;
let onDevice: DeviceStory[];
let calls: { cmd: string; args: Record<string, unknown> }[];
let sends: Map<string, Deferred>;
let ejectOutcome: EjectOutcome | { kind: string; message: string };
let removeFailure: { kind: string; message: string } | null;
let recoverFailure: { kind: string; message: string } | null;
let carrierConsent: { text_version: number; accepted_at: string } | null;
let storedKeys: boolean;
let carrierCandidates: {
  uuid: string;
  title: string;
  hidden: boolean;
  with_stored: boolean;
}[];

const WARNING =
  "Fonction expérimentale, non officielle. Pour installer une histoire sur une Flam en firmware 1.x, flam-launcher réutilise les clés d'une histoire officielle déjà présente sur cette Flam et achetée sur votre compte. Aucune clé n'est fournie par flam-launcher, aucune histoire officielle n'est copiée ni exportée. Si cette histoire officielle est retirée de la Flam ou de votre compte, les histoires installées ainsi peuvent devenir illisibles. Usage strictement personnel, à vos risques ; fonction ni approuvée ni prise en charge par Lunii.";
const OFFICIAL = "99999999-9999-4999-8999-999999999999";

const names = () => calls.map((c) => c.cmd);
const count = (cmd: string) => calls.filter((c) => c.cmd === cmd).length;

beforeEach(() => {
  window.localStorage.clear();
  status = detected();
  onDevice = [deviceStory(C, "Déjà là")];
  calls = [];
  sends = new Map();
  ejectOutcome = { status: "ejected", message: "Vous pouvez débrancher la Flam." };
  removeFailure = null;
  recoverFailure = null;
  carrierConsent = null;
  storedKeys = false;
  carrierCandidates = [
    { uuid: OFFICIAL, title: "Histoire officielle", hidden: false, with_stored: false },
  ];
  mockIPC(
    (cmd, payload) => {
      const args = (payload ?? {}) as Record<string, unknown>;
      calls.push({ cmd, args });
      switch (cmd) {
        case "device_status":
          return status;
        case "device_recover_transfers":
          if (recoverFailure) throw recoverFailure;
          return {
            incomplete_removed: 0,
            previous_restored: 0,
            updates_finished: 0,
            left_alone: 0,
          };
        case "device_list_stories":
          return report(onDevice);
        case "device_send_story":
          return new Promise((resolve, reject) => {
            sends.set(String(args.uuid), { resolve, reject });
          });
        case "device_cancel_transfer":
          return true;
        case "device_remove_story":
          if (removeFailure) throw removeFailure;
          onDevice = onDevice.filter((s) => s.uuid !== args.id);
          return {
            id: args.id,
            index_updated: true,
            folders_removed: 1,
            cleanup_pending: false,
          };
        case "device_eject":
          if ("kind" in ejectOutcome) throw ejectOutcome;
          return ejectOutcome;
        case "library_thumbnail":
          return "data:image/png;base64,AAAA";
        case "device_carrier_state":
          return {
            warning_text: WARNING,
            warning_version: 1,
            consent: carrierConsent,
            firmware: "1.15.14",
            carrier_needed: true,
            stored_keys: storedKeys,
            // Comme le backend : rien n'est lu avant le consentement.
            candidates: carrierConsent ? carrierCandidates : [],
          };
        case "device_carrier_accept":
          if (args.understood !== true) throw { kind: "validation", message: "case" };
          carrierConsent = { text_version: 1, accepted_at: "2026-10-17T10:00:00Z" };
          return carrierConsent;
        case "device_carrier_forget":
          return storedKeys ? 1 : 0;
        case "plugin:dialog|open":
          return "D:\\sauvegarde\\bt";
        default:
          throw { kind: "unknown", message: cmd };
      }
    },
    { shouldMockEvents: true },
  );
});

afterEach(() => {
  clearMocks();
  vi.useRealTimers();
});

function Harness() {
  return (
    <ToastProvider>
      <DeviceProvider>
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

/** Attend que la Flam et ses histoires soient lues (les boutons connaissent alors leur état). */
async function ready() {
  await screen.findByRole("heading", { name: "Histoires sur la Flam (1)" });
  await waitFor(() =>
    expect(sendButton("Capi")).toHaveAttribute("aria-disabled", "false"),
  );
}

function acknowledgeWarning() {
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
}

async function resolveSend(uuid: string, value: unknown = { id: uuid }) {
  await waitFor(() => expect(sends.has(uuid)).toBe(true));
  await act(async () => {
    sends.get(uuid)?.resolve(value);
  });
}

it("désactive le bouton avec la raison : Flam non détectée", async () => {
  status = absent;
  const { unmount } = render(
    <ToastProvider>
      <DeviceProvider>
        <SendToDeviceButton {...library[0]!} />
      </DeviceProvider>
    </ToastProvider>,
  );
  await waitFor(() =>
    expect(
      screen.getAllByRole("button", { name: /Envoyer sur la Flam/ })[0],
    ).toHaveAttribute("aria-disabled", "true"),
  );
  const button = screen.getAllByRole("button", { name: /Envoyer sur la Flam/ })[0]!;
  expect(button).toHaveAccessibleDescription("Flam non détectée");
  fireEvent.click(button);
  expect(count("device_send_story")).toBe(0);
  unmount();
});

it("désactive le bouton avec la raison : déjà présente, espace insuffisant", async () => {
  render(<Harness />);
  await ready();
  const present = sendButton("Déjà là");
  expect(present).toHaveAttribute("aria-disabled", "true");
  expect(present).toHaveAccessibleDescription("Déjà sur la Flam");
  const big = sendButton("Énorme");
  expect(big).toHaveAttribute("aria-disabled", "true");
  expect(big).toHaveAccessibleDescription("Espace insuffisant sur la Flam");
  expect(sendButton("Capi")).toHaveAttribute("aria-disabled", "false");

  fireEvent.click(present);
  fireEvent.click(big);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  expect(count("device_send_story")).toBe(0);
});

it("indique la lecture de la Flam tant que ses histoires ne sont pas connues", async () => {
  render(<Harness />);
  await screen.findByRole("heading", { name: "Flam détectée" });
  const button = sendButton("Capi");
  expect(button).toHaveAttribute("aria-disabled", "true");
  expect(button).toHaveAccessibleDescription("Lecture de la Flam…");
  await ready();
});

it("avertit une seule fois que la fonction est expérimentale", async () => {
  render(<Harness />);
  await ready();

  sendButton("Capi").focus();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog", {
    name: "Fonction expérimentale, non officielle",
  });
  expect(within(dialog).getByRole("button", { name: "Annuler" })).toHaveFocus();
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(count("device_send_story")).toBe(0);
  expect(sendButton("Capi")).toHaveFocus();

  fireEvent.click(sendButton("Capi"));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "J'ai compris, envoyer",
    }),
  );
  await waitFor(() => expect(count("device_send_story")).toBe(1));
  expect(window.localStorage.getItem("flam-launcher.flamSendWarningAck")).toBe("1");
  await resolveSend(A);

  fireEvent.click(sendButton("Les trois ours"));
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();
  await waitFor(() => expect(count("device_send_story")).toBe(2));
  await resolveSend(B);
});

it(
  "envoie la file séquentiellement, avec progression, puis rappelle de redémarrer la Flam",
  async () => {
    acknowledgeWarning();
    const { container } = render(<Harness />);
    await ready();

    fireEvent.click(sendButton("Capi"));
    fireEvent.click(sendButton("Les trois ours"));
    expect(sendButton("Capi")).toHaveAccessibleDescription("Dans la file d'envoi");
    expect(sendButton("Les trois ours")).toHaveAccessibleDescription(
      "Dans la file d'envoi",
    );

    const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
    await waitFor(() => expect(count("device_send_story")).toBe(1));
    expect(calls.find((c) => c.cmd === "device_send_story")?.args).toEqual({
      root: "E:\\",
      uuid: A,
    });
    expect(within(panel).getByText("Histoire 1 sur 2")).toBeInTheDocument();
    expect(within(panel).getByText("En attente")).toBeInTheDocument();
    // Un seul transfert à la fois : B n'est pas lancée.
    expect(sends.has(B)).toBe(false);
    expect(within(panel).getByText(/Environ 4 minutes restantes/)).toBeInTheDocument();

    await act(async () => {
      await emit("device://transfer-progress", {
        uuid: A,
        phase: "copy",
        done: 40 * MB,
        total: 80 * MB,
      });
    });
    await waitFor(() =>
      expect(
        within(panel).getByRole("progressbar", { name: "Progression de « Capi »" }),
      ).toHaveAttribute("aria-valuenow", "50"),
    );
    expect(
      within(panel).getByRole("progressbar", { name: "Progression de l'envoi" }),
    ).toHaveAttribute("aria-valuenow", "25");
    expect(within(panel).getByText("Copie sur la Flam")).toBeInTheDocument();
    // Les événements de progression d'une autre histoire sont ignorés.
    await act(async () => {
      await emit("device://transfer-progress", {
        uuid: B,
        phase: "verify",
        done: 80 * MB,
        total: 80 * MB,
      });
    });
    expect(
      within(panel).getByRole("progressbar", { name: "Progression de « Capi »" }),
    ).toHaveAttribute("aria-valuenow", "50");
    await expectNoAxeViolations(container, ["region", "landmark-one-main"]);

    onDevice = [deviceStory(C, "Déjà là"), deviceStory(A, "Capi")];
    await resolveSend(A);
    await waitFor(() => expect(sends.has(B)).toBe(true));
    expect(count("device_send_story")).toBe(2);
    expect(within(panel).getByText("Histoire 2 sur 2")).toBeInTheDocument();
    await waitFor(() =>
      expect(sendButton("Capi")).toHaveAccessibleDescription("Déjà sur la Flam"),
    );

    onDevice = [...onDevice, deviceStory(B, "Les trois ours")];
    await resolveSend(B);
    await waitFor(() =>
      expect(
        within(panel).getByText(/Redémarrez la Flam pour qu'elle reconstruise/),
      ).toBeInTheDocument(),
    );
    expect(within(panel).getAllByText("Envoyée")).toHaveLength(2);
    expect(
      within(panel).queryByRole("button", { name: "Annuler l'envoi" }),
    ).not.toBeInTheDocument();
    expect(
      within(panel).getByRole("button", { name: "Éjecter la Flam" }),
    ).toBeInTheDocument();
    // La liste de la Flam est relue : les deux histoires y figurent désormais.
    await screen.findByRole("heading", { name: "Histoires sur la Flam (3)" });
    await waitFor(() =>
      expect(sendButton("Capi")).toHaveAccessibleDescription("Déjà sur la Flam"),
    );

    fireEvent.click(within(panel).getByRole("button", { name: "Fermer la liste" }));
    expect(
      screen.queryByRole("region", { name: "Envoi sur la Flam" }),
    ).not.toBeInTheDocument();
  },
  SLOW_TEST_MS,
);

it("annule l'envoi en cours et les histoires en attente", async () => {
  acknowledgeWarning();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  fireEvent.click(sendButton("Les trois ours"));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  await waitFor(() => expect(sends.has(A)).toBe(true));

  fireEvent.click(within(panel).getByRole("button", { name: "Annuler l'envoi" }));
  await waitFor(() => expect(count("device_cancel_transfer")).toBe(1));
  expect(
    within(panel).getByRole("button", { name: "Annulation en cours…" }),
  ).toHaveAttribute("aria-disabled", "true");

  await act(async () => {
    sends.get(A)?.reject({ kind: "cancelled", message: "installation annulée" });
  });
  await waitFor(() => expect(within(panel).getAllByText("Annulée")).toHaveLength(2));
  // B n'a jamais été lancée ; les deux histoires peuvent être renvoyées.
  expect(count("device_send_story")).toBe(1);
  expect(
    within(panel).queryByRole("button", { name: "Annuler l'envoi" }),
  ).not.toBeInTheDocument();
  expect(sendButton("Capi")).toHaveAttribute("aria-disabled", "false");
  expect(sendButton("Les trois ours")).toHaveAttribute("aria-disabled", "false");
});

it("retire une histoire en attente de la file", async () => {
  acknowledgeWarning();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  fireEvent.click(sendButton("Les trois ours"));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  fireEvent.click(
    within(panel).getByRole("button", { name: "Retirer « Les trois ours » de la file" }),
  );
  expect(within(panel).queryByText("Les trois ours")).not.toBeInTheDocument();
  await resolveSend(A);
  await waitFor(() => expect(within(panel).getByText("Envoyée")).toBeInTheDocument());
  expect(count("device_send_story")).toBe(1);
});

it("affiche l'erreur d'une histoire et poursuit la file", async () => {
  acknowledgeWarning();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  fireEvent.click(sendButton("Les trois ours"));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  await waitFor(() => expect(sends.has(A)).toBe(true));
  await act(async () => {
    sends.get(A)?.reject({
      kind: "validation",
      message: "espace insuffisant sur la Flam : rien n'a été installé",
    });
  });
  expect(
    await within(panel).findByText(
      /espace insuffisant sur la Flam : rien n'a été installé/,
    ),
  ).toBeInTheDocument();
  expect(within(panel).getByText("Échec")).toBeInTheDocument();
  // L'annonce dit l'échec, sans pourcentage.
  expect(
    screen
      .getAllByRole("status")
      .map((s) => s.textContent)
      .join(" "),
  ).toContain("Échec de l'envoi de « Capi »");
  await waitFor(() => expect(sends.has(B)).toBe(true));
  await resolveSend(B);
  await waitFor(() => expect(within(panel).getByText("Envoyée")).toBeInTheDocument());
  // Une histoire en échec peut être renvoyée.
  expect(sendButton("Capi")).toHaveAttribute("aria-disabled", "false");
});

it("bloque les envois quand le firmware n'est pas pris en charge", async () => {
  acknowledgeWarning();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  fireEvent.click(sendButton("Les trois ours"));
  const panel = await screen.findByRole("region", { name: "Envoi sur la Flam" });
  await waitFor(() => expect(sends.has(A)).toBe(true));
  await act(async () => {
    sends.get(A)?.reject({
      kind: "validation",
      message: "firmware de la Flam non pris en charge : « 1.4 » (seul le 2.x l'est)",
    });
  });
  expect(
    await within(panel).findByText(/firmware de cette Flam n'est pas pris en charge/),
  ).toBeInTheDocument();
  // Les histoires suivantes ne sont pas tentées.
  expect(count("device_send_story")).toBe(1);
  expect(sendButton("Les trois ours")).toHaveAttribute("aria-disabled", "true");
  expect(sendButton("Les trois ours")).toHaveAccessibleDescription(
    "Firmware de la Flam non pris en charge",
  );
});

it("retire une histoire de la Flam après confirmation", async () => {
  render(<Harness />);
  await ready();

  const remove = screen.getByRole("button", { name: "Retirer « Déjà là »" });
  remove.focus();
  fireEvent.click(remove);
  const dialog = await screen.findByRole("alertdialog", {
    name: "Retirer « Déjà là » de la Flam ?",
  });
  expect(within(dialog).getByRole("button", { name: "Annuler" })).toHaveFocus();
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
  expect(count("device_remove_story")).toBe(0);
  expect(screen.getByRole("button", { name: "Retirer « Déjà là »" })).toHaveFocus();

  fireEvent.click(screen.getByRole("button", { name: "Retirer « Déjà là »" }));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Retirer de la Flam",
    }),
  );
  await waitFor(() => expect(count("device_remove_story")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_remove_story")?.args).toEqual({
    root: "E:\\",
    id: C,
    confirmed: true,
  });
  expect(await screen.findByText("« Déjà là » retirée de la Flam.")).toBeInTheDocument();
  await screen.findByRole("heading", { name: "Histoires sur la Flam (0)" });
  // La liste est relue, le focus ne reste pas sur un élément disparu.
  await waitFor(() =>
    expect(document.querySelector(".fl-device-stories-wrap")).toHaveFocus(),
  );
  await waitFor(() =>
    expect(sendButton("Déjà là")).toHaveAttribute("aria-disabled", "false"),
  );
});

it("signale l'échec d'un retrait", async () => {
  removeFailure = { kind: "validation", message: "la Flam est occupée" };
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Retirer « Déjà là »" }));
  fireEvent.click(
    within(await screen.findByRole("alertdialog")).getByRole("button", {
      name: "Retirer de la Flam",
    }),
  );
  expect(
    await screen.findByText("Impossible de retirer l'histoire : la Flam est occupée"),
  ).toBeInTheDocument();
  expect(
    screen.getByRole("heading", { name: "Histoires sur la Flam (1)" }),
  ).toBeInTheDocument();
});

it("refuse le retrait et l'éjection pendant un envoi, raison à l'appui", async () => {
  acknowledgeWarning();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  await waitFor(() => expect(sends.has(A)).toBe(true));

  const remove = screen.getByRole("button", { name: "Retirer « Déjà là »" });
  expect(remove).toHaveAttribute("aria-disabled", "true");
  expect(remove).toHaveAccessibleDescription("Retrait indisponible pendant un envoi.");
  fireEvent.click(remove);
  expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument();

  const eject = screen.getAllByRole("button", { name: "Éjecter la Flam" })[0]!;
  expect(eject).toHaveAttribute("aria-disabled", "true");
  fireEvent.click(eject);
  expect(count("device_eject")).toBe(0);
  await resolveSend(A);
});

it("éjecte la Flam et affiche le message du backend", async () => {
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Éjecter la Flam" }));
  expect(await screen.findByText("Vous pouvez débrancher la Flam.")).toBeInTheDocument();
  expect(calls.find((c) => c.cmd === "device_eject")?.args).toEqual({ root: "E:\\" });
});

it("affiche le message de repli quand l'éjection logicielle échoue", async () => {
  ejectOutcome = {
    status: "fallback",
    reason: "fichier ouvert",
    message: "Retirez la Flam en toute sécurité : éjectez-la depuis l'Explorateur.",
  };
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Éjecter la Flam" }));
  expect(await screen.findByText(/éjectez-la depuis l'Explorateur/)).toBeInTheDocument();
});

it("signale un refus d'éjecter", async () => {
  ejectOutcome = { kind: "validation", message: "la Flam est occupée" };
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Éjecter la Flam" }));
  expect(
    await screen.findByText("Impossible d'éjecter la Flam : la Flam est occupée"),
  ).toBeInTheDocument();
});

it("reprend les transferts une seule fois par branchement, avant de lire la Flam", async () => {
  vi.useFakeTimers({ shouldAdvanceTime: true });
  render(<Harness />);
  await ready();
  expect(count("device_recover_transfers")).toBe(1);
  expect(calls.find((c) => c.cmd === "device_recover_transfers")?.args).toEqual({
    root: "E:\\",
  });
  expect(names().indexOf("device_recover_transfers")).toBeLessThan(
    names().indexOf("device_list_stories"),
  );

  // Plusieurs scrutations de la même Flam : pas de nouvelle reprise.
  for (let i = 0; i < 3; i += 1) {
    await act(async () => {
      await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
    });
  }
  expect(count("device_status")).toBeGreaterThanOrEqual(4);
  expect(count("device_recover_transfers")).toBe(1);

  // Débranchée puis rebranchée : une nouvelle reprise.
  status = absent;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
  });
  await screen.findByRole("heading", { name: "Aucune Flam détectée" });
  expect(count("device_recover_transfers")).toBe(1);
  status = detected();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(POLL_INTERVAL_MS + 50);
  });
  await waitFor(() => expect(count("device_recover_transfers")).toBe(2));
  await ready();
  expect(count("device_recover_transfers")).toBe(2);
});

it("signale une reprise échouée sans bloquer l'écran", async () => {
  recoverFailure = { kind: "validation", message: "la Flam est occupée" };
  render(<Harness />);
  expect(
    await screen.findByText(/La reprise d'un éventuel transfert interrompu a échoué/),
  ).toBeInTheDocument();
  await ready();
});

function detectedV1(): DeviceStatus {
  const value = detected();
  value.device!.firmware = "1.15.14";
  return value;
}

it("Flam 1.x : aucun envoi sans consentement ni choix des clés", async () => {
  acknowledgeWarning();
  status = detectedV1();
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog", {
    name: "Flam en firmware 1.x : mode carrier",
  });
  // Texte affiché tel quel ; « Continuer » bloqué tant que la case n'est pas cochée.
  expect(await within(dialog).findByText(WARNING)).toBeInTheDocument();
  const next = within(dialog).getByRole("button", { name: "Continuer" });
  expect(next).toBeDisabled();
  fireEvent.click(next);
  expect(count("device_carrier_accept")).toBe(0);
  // Annuler : rien n'est envoyé.
  fireEvent.click(within(dialog).getByRole("button", { name: "Annuler" }));
  await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
  expect(count("device_send_story")).toBe(0);
  expect(count("device_carrier_accept")).toBe(0);

  // Consentement puis choix de l'histoire officielle.
  fireEvent.click(sendButton("Capi"));
  const again = await screen.findByRole("alertdialog");
  fireEvent.click(await within(again).findByRole("checkbox", { name: "J'ai compris" }));
  fireEvent.click(within(again).getByRole("button", { name: "Continuer" }));
  await waitFor(() => expect(count("device_carrier_accept")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_carrier_accept")?.args).toEqual({
    textVersion: 1,
    understood: true,
  });
  const official = await within(again).findByRole("radio", {
    name: "Histoire officielle « Histoire officielle »",
  });
  expect(official).toBeChecked();
  expect(count("device_send_story")).toBe(0);
  await expectNoAxeViolations(again);
  fireEvent.click(within(again).getByRole("button", { name: "Vérifier et envoyer" }));
  await waitFor(() => expect(count("device_send_story")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_send_story")?.args).toEqual({
    root: "E:\\",
    uuid: A,
    carrier: { kind: "device", uuid: OFFICIAL },
    remember: true,
  });
  await resolveSend(A, { id: A, carrier_readback: true });

  // Envoi suivant : clés conservées, sans redemander.
  fireEvent.click(sendButton("Les trois ours"));
  await waitFor(() => expect(count("device_send_story")).toBe(2));
  expect(calls.filter((c) => c.cmd === "device_send_story")[1]?.args).toEqual({
    root: "E:\\",
    uuid: B,
    carrier: { kind: "stored" },
    remember: false,
  });
  expect(screen.queryByRole("alertdialog")).toBeNull();
});

it("Flam 1.x : clés choisies dans un fichier, sans conservation", async () => {
  acknowledgeWarning();
  status = detectedV1();
  carrierConsent = { text_version: 1, accepted_at: "2026-10-17T10:00:00Z" };
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog");
  fireEvent.click(
    await within(dialog).findByRole("radio", { name: /Choisir un fichier/ }),
  );
  const confirm = within(dialog).getByRole("button", { name: "Vérifier et envoyer" });
  expect(confirm).toBeDisabled();
  fireEvent.click(within(dialog).getByRole("button", { name: "Parcourir…" }));
  expect(await within(dialog).findByText(/Fichier choisi/)).toBeInTheDocument();
  fireEvent.click(
    within(dialog).getByRole("checkbox", { name: "Conserver ces clés sur ce PC" }),
  );
  fireEvent.click(confirm);
  await waitFor(() => expect(count("device_send_story")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_send_story")?.args).toMatchObject({
    carrier: { kind: "file", path: "D:\\sauvegarde\\bt" },
    remember: false,
  });
  // Couple refusé par le backend : l'échec est affiché et le choix sera redemandé.
  await waitFor(() => expect(sends.has(A)).toBe(true));
  await act(async () => {
    sends.get(A)?.reject({ kind: "validation", message: "couple invalide" });
  });
  expect(await screen.findAllByText(/couple invalide/)).not.toHaveLength(0);
  fireEvent.click(sendButton("Les trois ours"));
  expect(await screen.findByRole("alertdialog")).toBeInTheDocument();
});

it("Flam 1.x sans bt : clé saisie, aucun envoi tant qu'elle est invalide", async () => {
  acknowledgeWarning();
  status = detectedV1();
  carrierConsent = { text_version: 1, accepted_at: "2026-10-17T10:00:00Z" };
  carrierCandidates = [];
  const bt = "0123456789ABCDEF".repeat(4);
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog");
  expect(
    await within(dialog).findByText(
      "Aucune histoire de cette Flam ne contient de fichier bt : saisissez la clé ou choisissez un fichier.",
    ),
  ).toBeInTheDocument();
  // Saisie proposée d'office ; champ masqué.
  expect(
    within(dialog).getByRole("radio", { name: /Saisir la clé \(bt\)/ }),
  ).toBeChecked();
  const field = within(dialog).getByLabelText("Clé bt");
  expect(field).toHaveAttribute("type", "password");
  const confirm = within(dialog).getByRole("button", { name: "Vérifier et envoyer" });
  expect(confirm).toBeDisabled();
  // Caractère non hexadécimal, longueur incorrecte : refusés.
  fireEvent.change(field, { target: { value: `${bt.slice(0, 63)}g` } });
  expect(within(dialog).getByText(/Seuls les chiffres 0 à 9/)).toBeInTheDocument();
  expect(confirm).toBeDisabled();
  fireEvent.change(field, { target: { value: bt.slice(0, 60) } });
  expect(
    within(dialog).getByText("60 caractères hexadécimaux sur 64 attendus."),
  ).toBeInTheDocument();
  expect(confirm).toBeDisabled();
  fireEvent.change(field, { target: { value: `${bt}0` } });
  expect(confirm).toBeDisabled();
  fireEvent.click(confirm);
  expect(count("device_send_story")).toBe(0);
  // « Afficher la clé ».
  fireEvent.click(within(dialog).getByRole("checkbox", { name: "Afficher la clé" }));
  expect(field).toHaveAttribute("type", "text");
  // Espaces et majuscules tolérés : envoyée compacte, en minuscules.
  fireEvent.change(field, {
    target: { value: ` ${bt.slice(0, 32)} \n ${bt.slice(32)} ` },
  });
  expect(confirm).toBeEnabled();
  await expectNoAxeViolations(dialog);
  fireEvent.click(
    within(dialog).getByRole("checkbox", { name: "Conserver ces clés sur ce PC" }),
  );
  fireEvent.click(confirm);
  await waitFor(() => expect(count("device_send_story")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_send_story")?.args).toEqual({
    root: "E:\\",
    uuid: A,
    carrier: { kind: "entered", bt: bt.toLowerCase() },
    remember: false,
  });
  await resolveSend(A, { id: A, carrier_readback: true });
  // Clé saisie non conservée : redemandée à l'envoi suivant.
  fireEvent.click(sendButton("Les trois ours"));
  const again = await screen.findByRole("alertdialog");
  expect(within(again).getByLabelText("Clé bt")).toHaveValue("");
  expect(count("device_send_story")).toBe(1);
});

it("Flam 1.x : histoires sans bt proposées avec la clé conservée", async () => {
  acknowledgeWarning();
  status = detectedV1();
  carrierConsent = { text_version: 1, accepted_at: "2026-10-17T10:00:00Z" };
  storedKeys = true;
  carrierCandidates = [
    { uuid: OFFICIAL, title: "Sans bt", hidden: true, with_stored: true },
  ];
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog");
  expect(
    await within(dialog).findByText("Histoire masquée · Avec la clé conservée sur ce PC"),
  ).toBeInTheDocument();
  expect(within(dialog).queryByText(/ne contient de fichier bt/)).toBeNull();
  fireEvent.click(
    within(dialog).getByRole("radio", { name: /Histoire officielle « Sans bt »/ }),
  );
  fireEvent.click(within(dialog).getByRole("button", { name: "Vérifier et envoyer" }));
  await waitFor(() => expect(count("device_send_story")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_send_story")?.args).toMatchObject({
    carrier: { kind: "device", uuid: OFFICIAL },
  });
});

it("Flam 1.x : relecture d'info illisible signalée", async () => {
  acknowledgeWarning();
  status = detectedV1();
  carrierConsent = { text_version: 1, accepted_at: "2026-10-17T10:00:00Z" };
  render(<Harness />);
  await ready();
  fireEvent.click(sendButton("Capi"));
  const dialog = await screen.findByRole("alertdialog");
  await within(dialog).findByRole("radio", { name: /Histoire officielle/ });
  fireEvent.click(within(dialog).getByRole("button", { name: "Vérifier et envoyer" }));
  await resolveSend(A, { id: A, carrier_readback: false });
  expect(await screen.findAllByText(/risque d'être illisible/)).not.toHaveLength(0);
});

it("Flam 1.x : « Oublier les clés »", async () => {
  status = detectedV1();
  storedKeys = true;
  render(<Harness />);
  await ready();
  fireEvent.click(screen.getByRole("button", { name: "Oublier les clés" }));
  expect(
    await screen.findByText("Clés de cette Flam oubliées sur ce PC."),
  ).toBeInTheDocument();
  expect(calls.find((c) => c.cmd === "device_carrier_forget")?.args).toEqual({
    root: "E:\\",
  });
});

it("Flam 2.x : ni dialogue carrier ni bouton « Oublier les clés »", async () => {
  acknowledgeWarning();
  render(<Harness />);
  await ready();
  expect(screen.queryByRole("button", { name: "Oublier les clés" })).toBeNull();
  fireEvent.click(sendButton("Capi"));
  await waitFor(() => expect(count("device_send_story")).toBe(1));
  expect(calls.find((c) => c.cmd === "device_send_story")?.args).toEqual({
    root: "E:\\",
    uuid: A,
  });
  expect(count("device_carrier_state")).toBe(0);
});
