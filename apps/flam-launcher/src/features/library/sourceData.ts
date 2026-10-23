import { getCatalogEntries, refreshCatalog, type CatalogView } from "../../api/catalog";
import type { AppError } from "../../api/error";
import { scanFolder } from "../../api/folder";
import { listLibrary } from "../../api/library";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
import type { SourceStatus } from "../../components";

/**
 * Entrée affichée : celle de la source, plus, pour « Mes histoires », la vignette conservée par
 * la bibliothèque (`LibraryEntry.thumb`, unique par installation). Elle sert à la clé de cache
 * des vignettes (`useThumbnail`) : une réinstallation rafraîchit l'image sans remonter la carte.
 */
export type ListedEntry = StoryEntry & { library_thumb?: string | null };

/** Contenu d'une source prêt à afficher. */
export interface SourceData {
  entries: ListedEntry[];
  /** Éléments ignorés à la lecture (un par ligne, déjà lisibles). */
  warnings: string[];
  /** Catalogue injoignable : les entrées viennent du cache. */
  offline: boolean;
  /** Dernière vérification réussie d'un catalogue, en secondes Unix. */
  updatedAt: number | null;
  /** Échec du dernier rafraîchissement, quand des données en cache sont tout de même servies. */
  problem: AppError | null;
}

const EMPTY: SourceData = {
  entries: [],
  warnings: [],
  offline: false,
  updatedAt: null,
  problem: null,
};

function fromCatalog(view: CatalogView): SourceData {
  return {
    entries: view.entries,
    warnings: view.warnings,
    offline: view.status.offline,
    updatedAt: view.status.updated_at,
    problem: view.status.error,
  };
}

/**
 * Charge le contenu d'une source. Un catalogue est lu dans le cache, puis téléchargé s'il ne l'a
 * jamais été ou si `refresh` est demandé ; un dossier est toujours réanalysé. « Mes histoires »
 * liste les histoires installées.
 */
export async function loadSourceData(
  source: Source,
  refresh: boolean,
): Promise<SourceData> {
  switch (source.type) {
    case "catalog": {
      if (refresh) return fromCatalog(await refreshCatalog(source.id));
      const cached = await getCatalogEntries(source.id);
      if (cached.status.updated_at !== null) return fromCatalog(cached);
      return fromCatalog(await refreshCatalog(source.id));
    }
    case "folder": {
      const scan = await scanFolder(source.id);
      return {
        ...EMPTY,
        entries: scan.entries,
        warnings: scan.warnings.map((w) => `${w.path} : ${w.message}`),
      };
    }
    case "library":
      // Chaque histoire installée garde l'entrée de sa source d'origine (métadonnées, lien),
      // complétée de la vignette conservée (clé de cache, voir `useThumbnail`).
      return {
        ...EMPTY,
        entries: (await listLibrary()).map((item) => ({
          ...item.source,
          library_thumb: item.thumb ?? null,
        })),
      };
    case "podcast":
      // Un flux podcast n'a pas d'histoires à installer : ses épisodes entrent dans un pack
      // (écran de sélection : #212).
      return EMPTY;
  }
}

/** Résumé d'une source pour la barre latérale. */
export interface SourceSummary {
  status: SourceStatus;
  count?: number;
}

export function summarize(data: SourceData): SourceSummary {
  return {
    status: data.offline ? "offline" : data.problem ? "error" : "ok",
    count: data.entries.length,
  };
}
