import { createContext, useContext } from "react";
import type { StoryEntry } from "../../api/story";
import type { CompatIssue } from "../../api/compat";
import type { CompatInfo } from "./compatInfo";

export interface CompatApi {
  /** Problème de lecture du fichier de statuts (les histoires sont alors non testées). */
  issue: CompatIssue | null;
  /** Statut de `entry` ; `undefined` tant que le rapport n'est pas chargé. */
  compatFor: (entry: StoryEntry) => CompatInfo | undefined;
  /** Relit le rapport (après une installation, par exemple). */
  reload: () => void;
}

export const CompatContext = createContext<CompatApi | null>(null);

/** Accès au statut de compatibilité ; nécessite un `CompatProvider` parent. */
export function useCompat(): CompatApi {
  const api = useContext(CompatContext);
  if (!api) throw new Error("useCompat doit être utilisé sous un CompatProvider");
  return api;
}
