import type { DeviceSummary } from "../../api/device";

/** Séparateur des champs de la clé : absent d'une racine, d'un nom de volume et d'une empreinte. */
const SEPARATOR = "\u0000";

/**
 * Identité de la Flam détectée pour l'interface (`null` sans Flam) : racine, nom de volume et
 * empreinte opaque rendue par `device_status` (#609). Le nom de volume est le même sur toutes les
 * Flam : sans l'empreinte, la clé se réduirait à la lettre du lecteur, et une autre Flam qui la
 * reprend entre deux relevés ne serait pas distinguée. Avec elle, une autre Flam sous la même
 * lettre est une autre clé : envois prévus perdus, file annulée, dialogues remis à zéro.
 *
 * Hypothèse : deux Flam différentes ont des `.mdf` différents, donc des empreintes différentes.
 * Elle n'est pas confirmée sur deux appareils réels ; si elle était fausse, la clé ne
 * distinguerait pas ces deux Flam, comme avant l'empreinte. `.mdf` illisible : pas d'empreinte,
 * la clé est celle d'une Flam sans identité (aucune écriture n'y est alors possible), différente
 * de celle de la même Flam redevenue lisible.
 */
export function deviceKeyOf(device: DeviceSummary | null): string | null {
  if (!device) return null;
  return [device.root, device.volume_label, device.fingerprint ?? ""].join(SEPARATOR);
}

/**
 * Empreinte de la Flam désignée par une clé de {@link deviceKeyOf} (`null` : aucune Flam, ou
 * `.mdf` illisible) : celle à transmettre aux commandes d'écriture pour une demande faite quand
 * cette clé était affichée.
 */
export function fingerprintOfKey(key: string | null | undefined): string | null {
  if (!key) return null;
  return key.split(SEPARATOR)[2] || null;
}
