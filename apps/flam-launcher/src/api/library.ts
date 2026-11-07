import { invokeCommand } from "./error";
import type { StoryEntry, StoryKind } from "./story";

/**
 * Format d'une histoire installée (miroir de `StoryFormat` dans `src-tauri/src/library.rs`) :
 * `flam` (`.plain.pk`, jouable sur le PC) ou `lunii` (`.lunii.pk`, histoire STUdio pour une
 * Flam 2.x, jamais jouable sur le PC).
 */
export type LibraryFormat = "flam" | "lunii";

/** Format produit par l'installation d'une archive de nature `kind`. */
export function formatOfKind(kind: StoryKind): LibraryFormat {
  return kind === "studio_zip" ? "lunii" : "flam";
}

/** Histoire installée (miroir de `LibraryItem` dans `src-tauri/src/library.rs`). */
export interface LibraryEntry {
  uuid: string;
  /** Version installée (`null` : inconnue). */
  version: number | null;
  title: string;
  /** Nom du fichier sous `<cache>/library/` (`<uuid>-<version>.plain.pk` ou `.lunii.pk`). */
  file: string;
  /** Format, déduit du nom du fichier par le backend. */
  format: LibraryFormat;
  /** Taille du fichier en octets. */
  size: number;
  /** Date d'installation (secondes Unix). */
  installed_at: number;
  /** Version de `telmi2flam` utilisée (champ `version` de l'événement `start`, repli `sha256:<12 hex>`), `null` sans conversion. */
  converter: string | null;
  /** Entrée d'origine (source, lien, métadonnées). */
  source: StoryEntry;
  /** Vignette conservée sous `<cache>/library/thumbs/` (nom de fichier), `null` sans vignette. */
  thumb?: string | null;
  /**
   * Empreinte de la source dossier à l'installation (#288), `null` : source distante, ou
   * installation antérieure (comparée autrement, voir `library_source.rs`).
   */
  source_stamp?: SourceStamp | null;
}

/** Empreinte d'une source locale (miroir de `SourceStamp` dans `src-tauri/src/library_source.rs`). */
export interface SourceStamp {
  /** Pré-filtre (taille et date, ou liste d'un dossier `.plain`). */
  quick: string;
  /** `sha256:<hex>` du contenu installé. */
  sha256: string;
}

/**
 * État de la source dossier d'une histoire installée (miroir de `SourceState`) : `changed`
 * (modifiée depuis l'installation, même à version égale), `missing` (disparue : la copie est
 * conservée), `unknown` (non vérifiable).
 */
export type SourceState = "unchanged" | "changed" | "missing" | "unknown";

/** Réponse de `library_source_status` pour une histoire (miroir de `SourceCheck`). */
export interface SourceCheck {
  uuid: string;
  state: SourceState;
}

/**
 * `library_source_status` : état de la source des histoires installées depuis une source
 * dossier (lecture seule ; la source n'est relue que si sa taille ou sa date a changé).
 */
export function librarySourceStatus(): Promise<SourceCheck[]> {
  return invokeCommand<SourceCheck[]>("library_source_status");
}

/** `library_list` : histoires installées, triées par titre. */
export function listLibrary(): Promise<LibraryEntry[]> {
  return invokeCommand<LibraryEntry[]>("library_list");
}

/**
 * `library_thumbnail` : URL `data:` de la vignette conservée à l'installation (`not_found` si
 * l'histoire n'en a pas : installation antérieure, vignette illisible ou disparue).
 */
export function libraryThumbnail(uuid: string): Promise<string> {
  return invokeCommand<string>("library_thumbnail", { uuid });
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
