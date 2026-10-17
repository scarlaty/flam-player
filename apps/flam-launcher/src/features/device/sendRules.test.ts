import { expect, it } from "vitest";
import type { DeviceStories, DeviceSummary } from "../../api/device";
import {
  estimateMinutes,
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
};

const stories: DeviceStories = {
  stories: [
    {
      uuid: "present",
      short_id: "present",
      placement: "hidden",
      title: null,
      in_library: false,
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

it("estime environ 4 minutes pour 80 Mo, au moins une minute", () => {
  expect(estimateMinutes(80 * MB)).toBe(4);
  expect(estimateMinutes(160 * MB)).toBe(8);
  expect(estimateMinutes(1)).toBe(1);
  expect(estimateMinutes(null)).toBeNull();
});
