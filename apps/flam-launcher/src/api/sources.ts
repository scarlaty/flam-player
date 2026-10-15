import { invokeCommand } from "./error";
import type { Recovery } from "./recovery";

/** Identifiant de la source implicite « Mes histoires » (non modifiable, toujours première). */
export const LIBRARY_SOURCE_ID = "library";

/** Source d'histoires (miroir de `Source` dans `src-tauri/src/sources.rs`). */
export type Source =
  | { type: "library"; id: string; name: string }
  | { type: "catalog"; id: string; name: string; url: string }
  | { type: "folder"; id: string; name: string; path: string };

/** Données d'une source à créer ou à modifier (le type ne peut pas changer). */
export type SourceInput =
  | { type: "catalog"; name: string; url: string }
  | { type: "folder"; name: string; path: string };

/** Réponse de `sources_list`. */
export interface SourcesView {
  /** « Mes histoires » en tête, puis les sources dans l'ordre choisi. */
  sources: Source[];
  /** Présent tant que la restauration de `sources.json` n'est pas acquittée (`ackRecovery`). */
  recovery: Recovery | null;
}

/** `sources_list` : sources ordonnées et restauration éventuelle à signaler. */
export function listSources(): Promise<SourcesView> {
  return invokeCommand<SourcesView>("sources_list");
}

/** `sources_add` : ajoute une source en fin de liste. */
export function addSource(input: SourceInput): Promise<Source> {
  return invokeCommand<Source>("sources_add", { input });
}

/** `sources_update` : modifie nom et URL/chemin d'une source existante. */
export function updateSource(id: string, input: SourceInput): Promise<Source> {
  return invokeCommand<Source>("sources_update", { id, input });
}

/** `sources_remove` : supprime une source (« Mes histoires » est refusée). */
export function removeSource(id: string): Promise<void> {
  return invokeCommand<void>("sources_remove", { id });
}

/**
 * `sources_reorder` : `ids` est la permutation complète des sources enregistrées
 * (sans « Mes histoires »). Renvoie la liste complète.
 */
export function reorderSources(ids: string[]): Promise<Source[]> {
  return invokeCommand<Source[]>("sources_reorder", { ids });
}
