import type { AppError } from "../../api/error";
import { authoringCode, type FolderProblem } from "../../api/authoring";
import { t } from "../../i18n";

/** Dernier composant d'un chemin relatif en `/`. */
export function fileName(path: string): string {
  return path.split("/").pop() ?? "";
}

/** Nom montré d'un élément : dernier composant du chemin, sans extension pour un fichier. */
export function shownName(path: string): string {
  const last = fileName(path);
  const dot = last.lastIndexOf(".");
  return dot > 0 ? last.slice(0, dot) : last;
}

/**
 * Phrase courte d'après le code du problème et l'élément en cause (le message du backend,
 * plus technique, reste dans les journaux).
 */
export function problemText(problem: FolderProblem): string {
  const text = t.authoring.problem;
  const name = shownName(problem.path);
  switch (problem.code) {
    case "no_story":
      return text.no_story;
    case "too_many_stories":
      return text.too_many_stories;
    case "too_many_options":
      return text.too_many_options(name);
    case "too_deep":
      return text.too_deep(name);
    case "too_many_entries":
      return text.too_many_entries;
    case "audio_too_large":
      return text.audio_too_large;
    case "unreadable_folder":
      return text.unreadable_folder(name);
    case "collection_in_book":
      return text.collection_in_book(name);
    case "orphan_image":
      return text.orphan_image(fileName(problem.path));
    case "empty_menu":
      return text.empty_menu(name);
    case "title_too_long":
      return text.title_too_long(name);
    case "duplicate_story_name":
      return t.authoringFolder.problem.duplicate_story_name(fileName(problem.path));
    case "fin_ignored":
      return t.authoringFolder.problem.fin_ignored;
    case "unsupported_file":
    case "misplaced_special_file":
    case "duplicate_special_file":
    case "link_ignored":
    case "invalid_name":
      return text.ignored(fileName(problem.path) || problem.path);
    case "settings_unreadable":
    case "settings_invalid_value":
    case "settings_newer_format":
      return text.settings;
    default:
      return text.other;
  }
}

/** Erreur d'une commande : ce qui s'est passé et quoi faire, d'après son code. */
export function errorText(error: AppError): string {
  const errors = t.authoringFolder.errors;
  switch (authoringCode(error)) {
    case "folder_missing":
      return t.authoring.folderMissing;
    case "folder_refused":
      return t.authoring.folderRefused;
    case "busy":
      return t.authoring.busy;
    case "blocked":
      return errors.blocked;
    case "conflict_copied":
      return errors.conflictCopied;
    case "conflict_foreign":
      return errors.conflictForeign;
    case "version_overflow":
      return errors.versionOverflow;
    case "retry_unavailable":
      return errors.retryUnavailable;
    case "folder_changed":
      return errors.folderChanged;
    case "uuid_mismatch":
      return errors.uuidMismatch;
    case "read_only_folder":
      return errors.readOnlyFolder;
    case "not_from_folder":
      return errors.notFromFolder;
    default:
      return t.authoring.failed;
  }
}
