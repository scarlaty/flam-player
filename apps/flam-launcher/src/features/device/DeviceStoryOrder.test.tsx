import { clearMocks, mockIPC } from "@tauri-apps/api/mocks";
import { firmwareV2 } from "../../test/firmware";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { DeviceStatus, DeviceStories, DeviceStory } from "../../api/device";
import { ToastProvider } from "../../components";
import { expectNoAxeViolations } from "../../test/axe";
import { SLOW_TEST_MS, tolerateLoadInAsyncUtils } from "../../test/timeouts";
import { DeviceContext, type DeviceApi } from "./DeviceContext";
import { DeviceProvider } from "./DeviceProvider";
import { DeviceScreen } from "./DeviceScreen";
import { DeviceStoryOrder } from "./DeviceStoryOrder";
import { SendToDeviceButton } from "./SendToDeviceButton";

tolerateLoadInAsyncUtils();

const A = "11111111-1111-4111-8111-111111111111";
const B = "22222222-2222-4222-8222-222222222222";
const C = "33333333-3333-4333-8333-333333333333";
const H = "44444444-4444-4444-8444-444444444444";
const E = "55555555-5555-4555-8555-555555555555";

const TITLES: Record<string, string> = {
  [A]: "Capi",
  [B]: "Les trois ours",
  [C]: "Le loup",
  [H]: "Cachée",
};

function story(uuid: string, extra: Partial<DeviceStory> = {}): DeviceStory {
  return {
    uuid,
    short_id: uuid.slice(0, 8),
    placement: "visible",
    title: TITLES[uuid] ?? null,
    in_library: false,
    library_uuids: [],
    missing: false,
    ...extra,
  };
}

function report(list: DeviceStory[], extra: Partial<DeviceStories> = {}): DeviceStories {
  return {
    stories: list,
    visible_index: "valid",
    hidden_index: "valid",
    str_unreadable: false,
    truncated: false,
    limit: 5000,
    ...extra,
  };
}

const status: DeviceStatus = {
  detected: true,
  hint: null,
  device_count: 1,
  device: {
    root: "E:\\",
    volume_label: "FLAM",
    file_system: "FAT32",
    space: { free: 500 * 1024 * 1024, total: 4096 * 1024 * 1024 },
    warnings: [],
    mdf_problem: null,
    backup_failed: false,
    firmware: "2.4.2",
    firmware_status: firmwareV2(),
  },
};

type Deferred = { resolve: (value: unknown) => void; reject: (reason: unknown) => void };

/** Ordre de `list` sur la fausse Flam (histoires visibles). */
let visible: string[];
let extraStories: DeviceStory[];
let listExtra: Partial<DeviceStories>;
let calls: { cmd: string; args: Record<string, unknown> }[];
let reorderFailure: { kind: string; message: string } | null;
let pendingReorder: Deferred | null;
let holdReorder: boolean;
let sends: Map<string, Deferred>;

const reorderCalls = () => calls.filter((c) => c.cmd === "device_reorder_stories");

beforeEach(() => {
  window.localStorage.setItem("flam-launcher.flamSendWarningAck", "1");
  visible = [A, B, C];
  extraStories = [story(H, { placement: "hidden" })];
  listExtra = {};
  calls = [];
  reorderFailure = null;
  pendingReorder = null;
  holdReorder = false;
  sends = new Map();
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
          return report(
            [...visible.map((uuid) => story(uuid)), ...extraStories],
            listExtra,
          );
        case "device_story_titles":
          return { titles: [], interrupted: false, keys: false };
        case "device_story_thumbnail":
          return { thumb: null, busy: false };
        case "device_story_details_cancel":
          return null;
        case "device_reorder_stories": {
          if (reorderFailure) throw reorderFailure;
          const order = args.order as string[];
          const apply = () => {
            const changed = order.join() !== visible.join();
            visible = [...order];
            return { changed, cache_removed: changed, count: order.length };
          };
          if (!holdReorder) return apply();
          return new Promise((resolve, reject) => {
            pendingReorder = { resolve: () => resolve(apply()), reject };
          });
        }
        case "device_send_story":
          return new Promise((resolve, reject) => {
            sends.set(String(args.uuid), { resolve, reject });
          });
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
  window.localStorage.clear();
});

function renderScreen() {
  return render(
    <ToastProvider>
      <DeviceProvider>
        <main>
          <DeviceScreen
            libraryPanel={
              <SendToDeviceButton uuid={E} title="Nouvelle" size={1024 * 1024} />
            }
          />
        </main>
      </DeviceProvider>
    </ToastProvider>,
  );
}

const orderList = () =>
  screen.getByRole("list", { name: "Ordre des histoires visibles" });

const orderNames = () =>
  within(orderList())
    .getAllByRole("listitem")
    .map((li) => li.querySelector(".fl-device-order__name")?.textContent);

const saveButton = () => screen.getByRole("button", { name: /^Enregistre/ });

async function openOrder() {
  const edit = await screen.findByRole("button", { name: "Modifier l'ordre" });
  fireEvent.click(edit);
  await screen.findByRole("heading", { name: "Ordre des histoires sur la Flam" });
}

it(
  "réordonne au clavier, garde le focus et enregistre l'ordre complet en une écriture",
  async () => {
    const { container } = renderScreen();
    await openOrder();
    expect(
      screen.getByRole("heading", { name: "Ordre des histoires sur la Flam" }),
    ).toHaveFocus();
    expect(orderNames()).toEqual(["Capi", "Les trois ours", "Le loup"]);
    // Les histoires masquées ne font pas partie de l'ordre.
    expect(within(orderList()).queryByText("Cachée")).toBeNull();
    expect(
      screen.getByText(/masquées ou non listées ne font pas partie/),
    ).toBeInTheDocument();
    expect(saveButton()).toHaveAttribute("aria-disabled", "true");
    expect(saveButton()).toHaveAccessibleDescription(
      "Déplacez au moins une histoire pour enregistrer un nouvel ordre.",
    );
    expect(screen.getByRole("button", { name: "Monter « Capi »" })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Descendre « Le loup »" })).toBeDisabled();

    const down = screen.getByRole("button", { name: "Descendre « Capi »" });
    down.focus();
    fireEvent.click(down);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Descendre « Capi »" })).toHaveFocus(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Descendre « Capi »" }));
    // En bas de la liste, « Descendre » est désactivé : le focus passe à « Monter ».
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Monter « Capi »" })).toHaveFocus(),
    );
    expect(orderNames()).toEqual(["Les trois ours", "Le loup", "Capi"]);
    expect(
      screen
        .getAllByRole("status")
        .some((s) => /« Capi » déplacée en position 3 sur 3/.test(s.textContent ?? "")),
    ).toBe(true);
    // Aucune écriture tant que l'ordre n'est pas enregistré.
    expect(reorderCalls()).toHaveLength(0);
    await expectNoAxeViolations(container, ["region"]);

    expect(saveButton()).not.toHaveAttribute("aria-disabled");
    fireEvent.click(saveButton());
    await waitFor(() => expect(reorderCalls()).toHaveLength(1));
    expect(reorderCalls()[0]?.args).toEqual({ root: "E:\\", order: [B, C, A] });
    expect(
      await screen.findByText(/Ordre enregistré sur la Flam\. Redémarrez la Flam/),
    ).toBeInTheDocument();
    // Retour à la liste, relue dans le nouvel ordre ; le focus revient au bouton.
    await waitFor(() => {
      const rows = within(
        screen.getByRole("list", { name: "Histoires installées sur la Flam" }),
      ).getAllByRole("listitem");
      expect(
        rows.map((r) => r.querySelector(".fl-device-row__name")?.textContent),
      ).toEqual(["Les trois ours", "Le loup", "Capi", "Cachée"]);
    });
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Modifier l'ordre" })).toHaveFocus(),
    );
  },
  SLOW_TEST_MS,
);

it("Flam 2.x non confirmée : ordre non enregistrable, raison à l'appui (#326)", async () => {
  const before = status.device!.firmware_status;
  status.device!.firmware_status = firmwareV2({
    trust: "unconfirmed",
    write_block: { kind: "confirmation_required", version: "2.4.2", standard: true },
  });
  try {
    renderScreen();
    await openOrder();
    expect(saveButton()).toHaveAttribute("aria-disabled", "true");
    expect(saveButton()).toHaveAccessibleDescription(
      "Ordre non modifiable : écriture sur cette Flam bloquée ou à confirmer (voir l'état du firmware).",
    );
    expect(screen.getByRole("button", { name: "Descendre « Capi »" })).toBeDisabled();
    fireEvent.click(saveButton());
    expect(reorderCalls()).toHaveLength(0);
  } finally {
    status.device!.firmware_status = before;
  }
});

it("Flam 2.x aux clés incohérentes : les histoires en cause sont désignées (#326)", async () => {
  const before = status.device!.firmware_status;
  const counts = {
    examined: 23,
    key_matches: 23,
    decrypted: 0,
    decrypted_other_key: 0,
    lunii_format: 0,
    clear_info: 0,
  };
  status.device!.firmware_status = firmwareV2({
    coherence: { kind: "contradiction", counts, stories: [A, B], total: 3 },
    write_block: { kind: "keys_contradiction", version: "2.4.2" },
  });
  try {
    renderScreen();
    expect(
      await screen.findByText(`Histoires en cause : ${A}, ${B} (et 1 autre).`),
    ).toBeInTheDocument();
  } finally {
    status.device!.firmware_status = before;
  }
});

it("réordonne par glisser-déposer", async () => {
  renderScreen();
  await openOrder();
  const rows = within(orderList()).getAllByRole("listitem");
  expect(rows[2]).toHaveAttribute("draggable", "true");
  fireEvent.dragStart(rows[2]!);
  fireEvent.dragOver(rows[0]!);
  fireEvent.drop(rows[0]!);
  fireEvent.dragEnd(rows[2]!);
  expect(orderNames()).toEqual(["Le loup", "Capi", "Les trois ours"]);
  expect(reorderCalls()).toHaveLength(0);

  fireEvent.click(saveButton());
  await waitFor(() => expect(reorderCalls()).toHaveLength(1));
  expect(reorderCalls()[0]?.args.order).toEqual([C, A, B]);
});

const lastAnnouncement = (re: RegExp) =>
  screen.getAllByRole("status").some((s) => re.test(s.textContent ?? ""));

it("Tout en haut et Tout en bas à la souris : focus gardé, annonce, extrémités désactivées", async () => {
  visible = [A, B, C];
  renderScreen();
  await openOrder();
  expect(screen.getByRole("button", { name: "Tout en haut « Capi »" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Tout en bas « Le loup »" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Tout en bas « Capi »" })).toBeEnabled();

  const bottom = screen.getByRole("button", { name: "Tout en bas « Capi »" });
  bottom.focus();
  fireEvent.click(bottom);
  expect(orderNames()).toEqual(["Les trois ours", "Le loup", "Capi"]);
  expect(lastAnnouncement(/« Capi » déplacée en position 3 sur 3/)).toBe(true);
  // « Tout en bas » est désormais désactivé : le focus passe à un bouton actif de la ligne.
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Monter « Capi »" })).toHaveFocus(),
  );
  expect(screen.getByRole("button", { name: "Tout en bas « Capi »" })).toBeDisabled();

  fireEvent.click(screen.getByRole("button", { name: "Tout en haut « Capi »" }));
  expect(orderNames()).toEqual(["Capi", "Les trois ours", "Le loup"]);
  expect(lastAnnouncement(/« Capi » déplacée en position 1 sur 3/)).toBe(true);
  expect(reorderCalls()).toHaveLength(0);

  fireEvent.click(screen.getByRole("button", { name: "Tout en haut « Le loup »" }));
  expect(orderNames()).toEqual(["Le loup", "Capi", "Les trois ours"]);
  fireEvent.click(saveButton());
  await waitFor(() => expect(reorderCalls()).toHaveLength(1));
  expect(reorderCalls()[0]?.args.order).toEqual([C, A, B]);
});

it("Fin et Début déplacent la ligne focalisée tout en bas / tout en haut", async () => {
  renderScreen();
  await openOrder();
  const down = screen.getByRole("button", { name: "Descendre « Les trois ours »" });
  down.focus();
  fireEvent.keyDown(down, { key: "End" });
  expect(orderNames()).toEqual(["Capi", "Le loup", "Les trois ours"]);
  expect(lastAnnouncement(/« Les trois ours » déplacée en position 3 sur 3/)).toBe(true);
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Monter « Les trois ours »" }),
    ).toHaveFocus(),
  );
  // Déjà en bas : Fin ne change rien.
  fireEvent.keyDown(document.activeElement!, { key: "End" });
  expect(orderNames()).toEqual(["Capi", "Le loup", "Les trois ours"]);

  fireEvent.keyDown(document.activeElement!, { key: "Home" });
  expect(orderNames()).toEqual(["Les trois ours", "Capi", "Le loup"]);
  expect(lastAnnouncement(/« Les trois ours » déplacée en position 1 sur 3/)).toBe(true);
  await waitFor(() =>
    expect(
      screen.getByRole("button", { name: "Descendre « Les trois ours »" }),
    ).toHaveFocus(),
  );
  // Début en haut, avec un modificateur, ou sur une autre touche : sans effet.
  fireEvent.keyDown(document.activeElement!, { key: "Home" });
  fireEvent.keyDown(document.activeElement!, { key: "End", ctrlKey: true });
  fireEvent.keyDown(document.activeElement!, { key: "ArrowDown" });
  expect(orderNames()).toEqual(["Les trois ours", "Capi", "Le loup"]);
});

it("Début / Fin ne sont traités que depuis un bouton de la ligne", async () => {
  renderScreen();
  await openOrder();
  const row = within(orderList()).getAllByRole("listitem")[1]!;
  fireEvent.keyDown(row, { key: "End" });
  const field = document.createElement("input");
  row.appendChild(field);
  fireEvent.keyDown(field, { key: "End" });
  field.remove();
  expect(orderNames()).toEqual(["Capi", "Les trois ours", "Le loup"]);
});

it("le focus retombe sur le titre si la ligne déplacée disparaît à la relecture", async () => {
  reorderFailure = { kind: "validation", message: "liste périmée" };
  renderScreen();
  await openOrder();
  const down = screen.getByRole("button", { name: "Descendre « Capi »" });
  down.focus();
  fireEvent.click(down);
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Descendre « Capi »" })).toHaveFocus(),
  );
  // Capi est retirée de la Flam entre-temps ; l'enregistrement échoue et la liste repart.
  visible = [B, C];
  fireEvent.click(saveButton());
  await screen.findByRole("alert");
  await waitFor(() => expect(orderNames()).toEqual(["Les trois ours", "Le loup"]));
  expect(screen.getByRole("heading", { name: /ordre/i, level: 3 })).toHaveFocus();
});

it("un dépôt sur soi-même ne change rien", async () => {
  renderScreen();
  await openOrder();
  const rows = within(orderList()).getAllByRole("listitem");
  fireEvent.dragStart(rows[1]!);
  fireEvent.drop(rows[1]!);
  expect(orderNames()).toEqual(["Capi", "Les trois ours", "Le loup"]);
  expect(saveButton()).toHaveAttribute("aria-disabled", "true");
  fireEvent.click(saveButton());
  expect(reorderCalls()).toHaveLength(0);
});

it("annuler abandonne les déplacements sans rien écrire", async () => {
  renderScreen();
  await openOrder();
  fireEvent.click(screen.getByRole("button", { name: "Descendre « Capi »" }));
  expect(orderNames()).toEqual(["Les trois ours", "Capi", "Le loup"]);
  fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
  expect(
    screen.queryByRole("heading", { name: "Ordre des histoires sur la Flam" }),
  ).toBeNull();
  const rows = within(
    screen.getByRole("list", { name: "Histoires installées sur la Flam" }),
  ).getAllByRole("listitem");
  expect(rows.map((r) => r.querySelector(".fl-device-row__name")?.textContent)).toEqual([
    "Capi",
    "Les trois ours",
    "Le loup",
    "Cachée",
  ]);
  expect(reorderCalls()).toHaveLength(0);
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Modifier l'ordre" })).toHaveFocus(),
  );
  // Rouvrir repart de l'ordre de la Flam.
  await openOrder();
  expect(orderNames()).toEqual(["Capi", "Les trois ours", "Le loup"]);
});

it("affiche l'enregistrement en cours et bloque les déplacements", async () => {
  holdReorder = true;
  renderScreen();
  await openOrder();
  fireEvent.click(screen.getByRole("button", { name: "Monter « Le loup »" }));
  fireEvent.click(saveButton());
  await waitFor(() => expect(pendingReorder).not.toBeNull());
  expect(saveButton()).toHaveTextContent("Enregistrement de l'ordre…");
  expect(saveButton()).toHaveAttribute("aria-disabled", "true");
  expect(screen.getByRole("button", { name: "Annuler" })).toHaveAttribute(
    "aria-disabled",
    "true",
  );
  expect(screen.getByRole("button", { name: "Monter « Capi »" })).toBeDisabled();
  expect(within(orderList()).getAllByRole("listitem")[0]).toHaveAttribute(
    "draggable",
    "false",
  );
  // Un second clic n'envoie rien de plus.
  fireEvent.click(saveButton());
  fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
  expect(reorderCalls()).toHaveLength(1);
  await act(async () => {
    pendingReorder?.resolve(undefined);
  });
  expect(await screen.findByText(/Ordre enregistré sur la Flam/)).toBeInTheDocument();
  expect(visible).toEqual([A, C, B]);
});

it("signale un refus et garde l'ordre choisi pour réessayer", async () => {
  reorderFailure = {
    kind: "validation",
    message:
      "la Flam est occupée (un transfert) : réessayez quand l'opération est terminée",
  };
  renderScreen();
  await openOrder();
  fireEvent.click(screen.getByRole("button", { name: "Descendre « Capi »" }));
  fireEvent.click(saveButton());
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent(
    "Impossible d'enregistrer l'ordre : la Flam est occupée (un transfert)",
  );
  expect(orderNames()).toEqual(["Les trois ours", "Capi", "Le loup"]);
  expect(saveButton()).not.toHaveAttribute("aria-disabled");

  reorderFailure = null;
  fireEvent.click(saveButton());
  expect(await screen.findByText(/Ordre enregistré sur la Flam/)).toBeInTheDocument();
  expect(visible).toEqual([B, A, C]);
});

it("repart de l'ordre de la Flam si sa liste a changé", async () => {
  reorderFailure = {
    kind: "validation",
    message: "l'histoire x n'est pas dans l'index de la Flam : actualisez la liste",
  };
  renderScreen();
  await openOrder();
  fireEvent.click(screen.getByRole("button", { name: "Descendre « Capi »" }));
  // Entre-temps, une histoire a été retirée de la Flam.
  visible = [A, C];
  fireEvent.click(saveButton());
  await screen.findByRole("alert");
  await waitFor(() => expect(orderNames()).toEqual(["Capi", "Le loup"]));
  expect(
    screen
      .getAllByRole("status")
      .some((s) => /La liste de la Flam a changé/.test(s.textContent ?? "")),
  ).toBe(true);
});

it("indique que l'ordre de la Flam était déjà celui-ci", async () => {
  renderScreen();
  await openOrder();
  fireEvent.click(screen.getByRole("button", { name: "Descendre « Capi »" }));
  // Un autre appareil ou une autre fenêtre a déjà appliqué cet ordre.
  visible = [B, A, C];
  fireEvent.click(saveButton());
  expect(
    await screen.findByText(/était déjà celui-ci : rien n'a été écrit/),
  ).toBeInTheDocument();
});

it("désactive la modification de l'ordre pendant un envoi", async () => {
  renderScreen();
  await openOrder();
  fireEvent.click(screen.getByRole("button", { name: "Descendre « Capi »" }));
  fireEvent.click(
    screen.getByRole("button", { name: "Envoyer sur la Flam « Nouvelle »" }),
  );
  await waitFor(() => expect(sends.has(E)).toBe(true));

  expect(saveButton()).toHaveAttribute("aria-disabled", "true");
  expect(saveButton()).toHaveAccessibleDescription(
    "Ordre non modifiable pendant un envoi.",
  );
  expect(screen.getByRole("button", { name: "Monter « Capi »" })).toBeDisabled();
  fireEvent.click(saveButton());
  expect(reorderCalls()).toHaveLength(0);

  // Hors édition, le bouton d'entrée est lui aussi indisponible.
  fireEvent.click(screen.getByRole("button", { name: "Annuler" }));
  const edit = screen.getByRole("button", { name: "Modifier l'ordre" });
  expect(edit).toHaveAttribute("aria-disabled", "true");
  expect(edit).toHaveAccessibleDescription("Ordre non modifiable pendant un envoi.");
  fireEvent.click(edit);
  expect(
    screen.queryByRole("heading", { name: "Ordre des histoires sur la Flam" }),
  ).toBeNull();

  await act(async () => {
    sends.get(E)?.resolve({ id: E });
  });
});

it("ne propose pas de modifier l'ordre d'un index non sûr ou trop court", async () => {
  for (const [list, extra] of [
    [[A, B], { visible_index: "damaged" }],
    [[A, B], { hidden_index: "unreadable" }],
    [[A, B], { truncated: true }],
    [[A], {}],
  ] as const) {
    visible = [...list];
    listExtra = extra;
    const view = renderScreen();
    await screen.findByRole("list", { name: "Histoires installées sur la Flam" });
    expect(screen.queryByRole("button", { name: "Modifier l'ordre" })).toBeNull();
    view.unmount();
  }
  // `list.hidden` absent : l'ordre reste modifiable.
  visible = [A, B];
  listExtra = { hidden_index: "absent" };
  renderScreen();
  expect(
    await screen.findByRole("button", { name: "Modifier l'ordre" }),
  ).toBeInTheDocument();
});

it("Flam débranchée : ordre non modifiable, sans raison « firmware » (#345)", () => {
  const api = {
    reorderOnDevice: vi.fn(),
    deviceKey: null,
    canMaintain: false,
    device: null,
  } as unknown as DeviceApi;
  render(
    <ToastProvider>
      <DeviceContext.Provider value={api}>
        <DeviceStoryOrder
          stories={[story(A), story(B)]}
          nameOf={(s) => s.title ?? s.uuid}
          hasOthers={false}
          busy={false}
          onClose={() => {}}
        />
      </DeviceContext.Provider>
    </ToastProvider>,
  );
  expect(
    screen.getByRole("button", { name: "Monter « Les trois ours »" }),
  ).toBeDisabled();
  expect(screen.getByRole("button", { name: "Descendre « Capi »" })).toBeDisabled();
  expect(screen.queryByText(/écriture sur cette Flam bloquée/)).not.toBeInTheDocument();
  expect(api.reorderOnDevice).not.toHaveBeenCalled();
});
