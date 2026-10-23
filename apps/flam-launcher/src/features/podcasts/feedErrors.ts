import type { AppError } from "../../api/error";
import { t } from "../../i18n";

/**
 * Titre distinct selon la cause d'un échec d'ajout. Le backend fournit déjà un message sans
 * l'adresse du flux (D11) : il est affiché tel quel sous ce titre. Le classement s'appuie sur
 * `kind`, puis sur le texte du message pour les causes qui partagent un même `kind`.
 */
export function feedErrorTitle(error: AppError): string {
  const text = t.podcasts.error;
  const message = error.message.toLowerCase();
  switch (error.kind) {
    case "network":
      return message.includes("adresse privée") ? text.blocked : text.network;
    case "feed_invalid":
      if (message.includes("aucun épisode")) return text.noAudio;
      if (message.includes("trop volumineux")) return text.tooLarge;
      return text.invalidFeed;
    case "validation":
      if (message.includes("déjà")) return text.duplicate;
      return message.includes("adresse") ? text.invalidUrl : t.errors.title(error.kind);
    default:
      return t.errors.title(error.kind);
  }
}
