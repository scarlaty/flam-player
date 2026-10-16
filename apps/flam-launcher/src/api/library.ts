import { invokeCommand } from "./error";
import type { StoryEntry } from "./story";

/** Histoire installée (miroir de `LibraryEntry` dans `src-tauri/src/library.rs`). */
export interface LibraryEntry {
  uuid: string;
  /** Version installée (`null` : inconnue). */
  version: number | null;
  title: string;
  /** Nom du fichier sous `<cache>/library/` (`<uuid>-<version>.plain.pk`). */
  file: string;
  /** Taille du `.plain.pk` en octets. */
  size: number;
  /** Date d'installation (secondes Unix). */
  installed_at: number;
  /** Version de `telmi2flam` utilisée (`sha256:<12 hex>`), `null` sans conversion. */
  converter: string | null;
  /** Entrée d'origine (source, lien, métadonnées). */
  source: StoryEntry;
}

/** `library_list` : histoires installées, triées par titre. */
export function listLibrary(): Promise<LibraryEntry[]> {
  return invokeCommand<LibraryEntry[]>("library_list");
}

/**
 * `library_remove` : désinstalle l'histoire `uuid` (`not_found` si absente ; `player_busy` si le
 * player la lit). La progression de l'enfant est **conservée par défaut** (retrouvée si
 * l'histoire est réinstallée) ; `purgeSaves = true` l'efface aussi : à ne proposer que sur
 * choix explicite de l'utilisateur.
 */
export function removeFromLibrary(uuid: string, purgeSaves = false): Promise<void> {
  return invokeCommand<void>("library_remove", { uuid, purgeSaves });
}

/**
 * Vrai si `candidate` propose une autre version que celle installée (même règle que
 * `LibraryEntry::is_outdated_by` côté Rust) : une version inconnue n'est jamais une mise à jour.
 */
export function hasUpdate(
  installed: LibraryEntry | undefined,
  candidate: StoryEntry,
): boolean {
  return (
    installed !== undefined &&
    installed.uuid === candidate.uuid &&
    installed.version !== null &&
    candidate.version !== null &&
    installed.version !== candidate.version
  );
}
