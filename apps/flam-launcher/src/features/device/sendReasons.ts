import { t } from "../../i18n";
import type { SendBlock } from "./sendRules";

const text = t.deviceTransfer.reasons;

/** Raison lisible de chaque blocage de l'envoi (bouton « Envoyer sur la Flam », envoi prévu). */
export const REASON_TEXT: Record<SendBlock, string> = {
  queued: text.queued,
  no_device: text.noDevice,
  firmware: text.firmware,
  lunii_needs_v2: text.luniiNeedsV2,
  checking: text.checking,
  present: text.present,
  no_space: text.noSpace,
  removing: text.removing,
};
