import { invokeCommand } from "./error";
import type { StoryEntry } from "./story";

export type { StoryEntry, StoryKind } from "./story";

/** Problème du dernier rafraîchissement (`kind` : `network` ou `catalog_invalid`). */
export interface CatalogProblem {
  kind: string;
  message: string;
}

/** Fraîcheur des données (miroir de `CatalogStatus` dans `src-tauri/src/catalog.rs`). */
export interface CatalogStatus {
  /** Rafraîchissement impossible faute de réseau : les données viennent du cache. */
  offline: boolean;
  /** Dernière vérification réussie, en secondes Unix ; `null` si jamais obtenu. */
  updated_at: number | null;
  error: CatalogProblem | null;
}

/** Entrées d'un catalogue et leur état (miroir de `CatalogView`). */
export interface CatalogView {
  source_id: string;
  entries: StoryEntry[];
  /** Entrées ignorées à la lecture du catalogue. */
  warnings: string[];
  status: CatalogStatus;
}

/** `catalog_refresh` : télécharge le catalogue (cache hors ligne en cas d'échec). */
export function refreshCatalog(sourceId: string): Promise<CatalogView> {
  return invokeCommand<CatalogView>("catalog_refresh", { sourceId });
}

/** `catalog_entries` : dernier catalogue en cache, sans réseau (vide si jamais téléchargé). */
export function getCatalogEntries(sourceId: string): Promise<CatalogView> {
  return invokeCommand<CatalogView>("catalog_entries", { sourceId });
}

/**
 * `catalog_thumbnail` : vignette d'une entrée sous forme d'URL `data:` utilisable dans un
 * `<img src>` (téléchargée à la première demande, puis lue sur le disque).
 */
export function getCatalogThumbnail(sourceId: string, thumbUrl: string): Promise<string> {
  return invokeCommand<string>("catalog_thumbnail", { sourceId, thumbUrl });
}
