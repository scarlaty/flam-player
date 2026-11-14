import type { DeviceSummary, ExpectedDevice } from "../../api/device";

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
 * distinguerait pas ces deux Flam, comme avant l'empreinte.
 *
 * Flam détectée sans empreinte (`.mdf` de plus de 64 Kio, que le backend ne lit pas ; un `.mdf`
 * qui ne s'ouvre pas rend la Flam non détectée) : clé d'une Flam sans identité, différente de
 * celle de toute Flam lisible, et pour laquelle aucune écriture n'est acceptée.
 */
export function deviceKeyOf(device: DeviceSummary | null): string | null {
  if (!device) return null;
  return [device.root, device.volume_label, device.fingerprint ?? ""].join(SEPARATOR);
}

/**
 * Flam désignée par une clé de {@link deviceKeyOf}, à transmettre à une commande d'écriture
 * pour une demande faite quand cette clé était affichée. Une clé sans empreinte (ou `null` :
 * aucune Flam affichée) donne `{ fingerprint: null }`, que le backend refuse sur toute Flam : il
 * ne retombe jamais sur « aucune comparaison », sans quoi une autre Flam, lisible, qui
 * reprendrait la lettre recevrait l'écriture.
 */
export function expectedOfKey(key: string | null | undefined): ExpectedDevice {
  return { fingerprint: key?.split(SEPARATOR)[2] || null };
}
