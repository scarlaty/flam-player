import type { AppError } from "../../../api/error";
import { t } from "../../../i18n";
import { errorText } from "../problemText";

/** Message d'une erreur de l'éditeur, d'après son code stable (`OpError::code`, `AppError`). */
export function editorErrorText(error: AppError): string {
  const text = t.authoringEditor.error;
  switch (error.kind) {
    case "changed":
      return text.changed;
    case "not_undoable":
      return text.not_undoable;
    case "nothing_to_undo":
      return text.nothing_to_undo;
    case "nothing_to_redo":
      return text.nothing_to_redo;
    case "invalid_id":
    case "not_found":
      return text.gone;
    case "link":
      return text.link;
    case "source_refused":
      return text.source_refused;
    case "too_large":
      return text.too_large;
    case "settings":
      return text.settings;
    case "no_recycle_bin":
      return text.no_recycle_bin;
    case "root_changed":
    case "root_refused":
    case "editor_closed":
      return text.closed;
    case "refused":
    case "validation":
      // Messages du backend déjà rédigés pour l'utilisateur (« titre vide : rien à renommer »).
      return text.refused(error.message);
    case "authoring":
      // Création lancée depuis l'éditeur : texte d'après le code (`busy`, `conflict_copied`…).
      return errorText(error);
    case "inconsistent":
      return text.inconsistent;
    case "io":
      return text.io;
    default:
      return text.other;
  }
}

/** Message d'un échec d'ouverture de l'éditeur (fenêtre principale). */
export function openErrorText(error: AppError): string {
  const text = t.authoringEditor;
  if (error.kind === "editor_closing") return text.closing;
  // Dossier refusé ou disparu : reconnu par son code (`folder_missing`, `folder_refused`).
  if (error.code === "folder_missing" || error.kind === "not_found")
    return text.error.folderMissing;
  if (
    error.code === "folder_refused" ||
    error.kind === "root_refused" ||
    error.kind === "link"
  )
    return text.error.folderRefused;
  return text.openFailed;
}
