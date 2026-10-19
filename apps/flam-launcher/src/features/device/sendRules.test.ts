import { expect, it } from "vitest";
import { firmwareV2 } from "../../test/firmware";
import type { DeviceStories, DeviceSummary } from "../../api/device";
import {
  estimateMinutes,
  findOnDevice,
  replaceBlock,
  sendBlock,
  SPACE_MARGIN_BYTES,
  type SendContext,
} from "./sendRules";

const MB = 1024 * 1024;

const device: DeviceSummary = {
  root: "E:\\",
  volume_label: "FLAM",
  file_system: "FAT32",
  space: { free: 500 * MB, total: 1024 * MB },
  warnings: [],
  mdf_problem: null,
  backup_failed: false,
  firmware: "2.4.2",
  firmware_status: firmwareV2(),
};

const stories: DeviceStories = {
  stories: [
    {
      uuid: "present",
      short_id: "present",
      placement: "hidden",
      title: null,
      in_library: false,
      library_uuids: [],
      missing: false,
    },
  ],
  visible_index: "valid",
  hidden_index: "valid",
  str_unreadable: false,
  truncated: false,
  limit: 5000,
};

function context(overrides: Partial<SendContext> = {}): SendContext {
  return {
    device,
    deviceStories: stories,
    firmwareBlocked: false,
    removing: false,
    queuedOrSent: new Set(),
    ...overrides,
  };
}

it("autorise une histoire absente avec assez de place", () => {
  expect(sendBlock(context(), "new", 100 * MB)).toBeNull();
  expect(sendBlock(context(), "new", null)).toBeNull();
});

it.each([
  ["Flam non détectée", context({ device: null }), "present", "no_device"],
  ["retrait en cours sur la Flam", context({ removing: true }), "new", "removing"],
  ["firmware non pris en charge", context({ firmwareBlocked: true }), "new", "firmware"],
  ["histoires pas encore lues", context({ deviceStories: null }), "new", "checking"],
  ["déjà présente (même masquée)", context(), "present", "present"],
  ["déjà dans la file", context({ queuedOrSent: new Set(["new"]) }), "new", "queued"],
] as const)("raison : %s", (_label, ctx, uuid, expected) => {
  expect(sendBlock(ctx, uuid, 1 * MB)).toBe(expected);
});

it("refuse quand la taille et la marge dépassent l'espace libre", () => {
  const free = 500 * MB;
  expect(sendBlock(context(), "new", free - SPACE_MARGIN_BYTES)).toBeNull();
  expect(sendBlock(context(), "new", free - SPACE_MARGIN_BYTES + 1)).toBe("no_space");
});

it("ne refuse pas pour l'espace quand il est inconnu", () => {
  expect(
    sendBlock(context({ device: { ...device, space: null } }), "new", 10_000 * MB),
  ).toBeNull();
});

it("donne la priorité à l'absence de Flam, puis à la présence, puis à la file", () => {
  expect(
    sendBlock(context({ device: null, queuedOrSent: new Set(["new"]) }), "new", 1),
  ).toBe("no_device");
  expect(sendBlock(context({ queuedOrSent: new Set(["present"]) }), "present", 1)).toBe(
    "present",
  );
  expect(
    sendBlock(
      context({ queuedOrSent: new Set(["new"]), firmwareBlocked: true }),
      "new",
      1,
    ),
  ).toBe("queued");
});

it("remplacement : seulement d'une histoire présente et visible, mêmes règles sinon", () => {
  const visible = (uuid: string) => ({
    ...stories.stories[0]!,
    uuid,
    short_id: uuid,
    placement: "visible" as const,
  });
  const deviceStories = { ...stories, stories: [...stories.stories, visible("capi")] };
  const ctx = (overrides: Partial<SendContext> = {}) =>
    context({ deviceStories, ...overrides });
  expect(replaceBlock(ctx(), "capi", 10 * MB)).toBeNull();
  expect(replaceBlock(ctx(), "absente", 10 * MB)).toBe("absent");
  // Masquée : le backend refuse le remplacement.
  expect(replaceBlock(ctx(), "present", 10 * MB)).toBe("absent");
  expect(replaceBlock(context({ deviceStories: null }), "capi", 1)).toBe("absent");
  expect(replaceBlock(ctx({ queuedOrSent: new Set(["capi"]) }), "capi", 1)).toBe(
    "queued",
  );
  expect(replaceBlock(ctx({ firmwareBlocked: true }), "capi", 1)).toBe("firmware");
  expect(replaceBlock(ctx(), "capi", 500 * MB)).toBe("no_space");
});

it("reconnaît une histoire de source dossier ou Telmi par l'UUID de son archive (#309)", () => {
  const onDevice = {
    ...stories.stories[0]!,
    uuid: "a701e27e-8f3a-248b-72bd-ff65fa5d440e",
    short_id: "a701e27e",
    placement: "visible" as const,
    in_library: true,
    library_uuids: ["a701e27e8f3a248b72bdff65fa5d440e", "ffffff-199a63526c4"],
  };
  const deviceStories = { ...stories, stories: [...stories.stories, onDevice] };
  const ctx = context({ deviceStories });
  for (const libraryUuid of onDevice.library_uuids) {
    expect(findOnDevice(deviceStories, libraryUuid)?.uuid).toBe(onDevice.uuid);
    expect(sendBlock(ctx, libraryUuid, 1 * MB)).toBe("present");
    expect(replaceBlock(ctx, libraryUuid, 1 * MB)).toBeNull();
  }
  // L'UUID de la Flam lui-même reste reconnu (histoire officielle).
  expect(findOnDevice(deviceStories, onDevice.uuid)?.uuid).toBe(onDevice.uuid);
  expect(findOnDevice(deviceStories, "autre")).toBeUndefined();
  expect(findOnDevice(null, onDevice.uuid)).toBeUndefined();
});

it("estime environ 4 minutes pour 80 Mo, au moins une minute", () => {
  expect(estimateMinutes(80 * MB)).toBe(4);
  expect(estimateMinutes(160 * MB)).toBe(8);
  expect(estimateMinutes(1)).toBe(1);
  expect(estimateMinutes(null)).toBeNull();
});
