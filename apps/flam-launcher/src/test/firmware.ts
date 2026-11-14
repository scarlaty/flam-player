import type { FirmwareStatus } from "../api/device";

/**
 * Empreinte opaque inventée d'une Flam de test (`DeviceSummary.fingerprint`, #609) : le backend
 * la rend pour toute Flam dont `.mdf` a été lu.
 */
export const TEST_FINGERPRINT = "5a".repeat(32);

/** État du firmware d'une Flam 2.x (écriture autorisée, sans contradiction). */
export function firmwareV2(overrides: Partial<FirmwareStatus> = {}): FirmwareStatus {
  return {
    version: "2.4.2",
    family: "v2",
    version_form: "main",
    mdf_size: 126,
    header: "01 00",
    layout_standard: true,
    usb_ids: "303a:819e",
    coherence: {
      kind: "no_evidence",
      counts: {
        examined: 0,
        key_matches: 0,
        decrypted: 0,
        decrypted_other_key: 0,
        lunii_format: 0,
        clear_info: 0,
      },
    },
    write_block: null,
    ...overrides,
  };
}

/** État du firmware de la Flam 1.15.14 mesurée (mode carrier). */
export function firmwareV1(): FirmwareStatus {
  return {
    version: "1.15.14",
    family: "v1",
    version_form: "bare",
    mdf_size: 126,
    header: "01 00",
    layout_standard: true,
    usb_ids: "303a:819e",
    coherence: { kind: "not_applicable" },
    write_block: null,
  };
}
