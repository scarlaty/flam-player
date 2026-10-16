import { createContext, useContext } from "react";
import type { AppError } from "../../api/error";
import type { Source } from "../../api/sources";
import type { SourceSummary } from "./sourceData";

export interface LibraryApi {
  /** Sources dans l'ordre de `sources.json`, « Mes histoires » en tête ; `null` avant le chargement. */
  sources: Source[] | null;
  /** Échec du chargement de la liste des sources. */
  error: AppError | null;
  /** État et nombre d'histoires connus par source (barre latérale). */
  summaries: Record<string, SourceSummary>;
  /** Mémorise le résumé d'une source après chargement de son contenu. */
  report: (sourceId: string, summary: SourceSummary) => void;
  /** Recharge la liste des sources. */
  reload: () => void;
}

export const LibraryContext = createContext<LibraryApi | null>(null);

/** Accès aux sources et à leur état ; nécessite un `LibraryProvider` parent. */
export function useLibrary(): LibraryApi {
  const api = useContext(LibraryContext);
  if (!api) {
    throw new Error("useLibrary doit être utilisé sous un LibraryProvider");
  }
  return api;
}
