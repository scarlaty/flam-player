import type { AppError } from "../../api/error";
import type { FolderProblem } from "../../api/authoring";
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

/** Erreur d'une commande : ce qui s'est passé et quoi faire. */
export function errorText(error: AppError): string {
  const message = error.message;
  if (error.kind === "not_found") return t.authoring.folderMissing;
  if (error.kind === "validation") {
    if (message.includes("en cours de création")) return t.authoring.busy;
    if (message.includes("ne peut pas être le dossier") || message.includes("lien"))
      return t.authoring.folderRefused;
  }
  return t.authoring.failed;
}
