import type { DeviceStories, DeviceSummary } from "../../api/device";

/** Marge d'espace libre exigée par le backend après le transfert (`SPACE_MARGIN`). */
export const SPACE_MARGIN_BYTES = 32 * 1024 * 1024;

/** Débit d'estimation : environ 80 Mo en 4 minutes en USB. */
export const BYTES_PER_MINUTE = (80 * 1024 * 1024) / 4;

/** Raison pour laquelle une histoire ne peut pas être envoyée maintenant. */
export type SendBlock =
  "queued" | "no_device" | "firmware" | "checking" | "present" | "no_space";

export interface SendContext {
  device: DeviceSummary | null;
  /** Histoires de la Flam, une fois lues (`null` : pas encore). */
  deviceStories: DeviceStories | null;
  firmwareBlocked: boolean;
  /** Histoires en file d'envoi, en cours d'envoi ou envoyées pendant cette session. */
  queuedOrSent: ReadonlySet<string>;
}

/**
 * Première raison qui interdit l'envoi de l'histoire `uuid` (taille `size` en octets, `null` :
 * inconnue), ou `null` si elle peut partir. Le backend revérifie tout : ces règles ne servent
 * qu'à désactiver le bouton avec une raison lisible.
 */
export function sendBlock(
  { device, deviceStories, firmwareBlocked, queuedOrSent }: SendContext,
  uuid: string,
  size: number | null,
): SendBlock | null {
  if (!device) return "no_device";
  if (deviceStories?.stories.some((story) => story.uuid === uuid)) return "present";
  if (queuedOrSent.has(uuid)) return "queued";
  if (firmwareBlocked) return "firmware";
  if (!deviceStories) return "checking";
  const free = device.space?.free;
  if (size !== null && free !== undefined && free < size + SPACE_MARGIN_BYTES) {
    return "no_space";
  }
  return null;
}

/** Minutes restantes (arrondies au-dessus), `null` si une taille manque. */
export function estimateMinutes(remainingBytes: number | null): number | null {
  if (remainingBytes === null) return null;
  return Math.max(1, Math.ceil(remainingBytes / BYTES_PER_MINUTE));
}
