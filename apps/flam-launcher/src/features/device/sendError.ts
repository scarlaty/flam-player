import { LUNII_NEEDS_V2 } from "../../api/device";
import type { AppError } from "../../api/error";
import { t } from "../../i18n";

/**
 * Message affiché pour l'échec d'un envoi sur la Flam : texte du front pour les `kind` qu'il
 * connaît (histoire STUdio sur une Flam 1.x), message du backend sinon.
 */
export function sendErrorMessage(failure: AppError): string {
  if (failure.kind === LUNII_NEEDS_V2) return t.deviceTransfer.errors.luniiNeedsV2;
  return failure.message;
}
