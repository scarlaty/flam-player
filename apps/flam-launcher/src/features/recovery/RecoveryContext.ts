import { createContext, useContext } from "react";
import type { Recovery } from "../../api/recovery";

export interface RecoveryApi {
  /** Restaurations à signaler (une par fichier au plus). */
  recoveries: Recovery[];
  /** Mémorise (ou efface, avec `null`) la restauration signalée par une lecture de `file`. */
  report: (file: string, recovery: Recovery | null) => void;
  /** Acquitte la restauration de `file` auprès du backend puis la retire. */
  acknowledge: (file: string) => Promise<void>;
}

export const RecoveryContext = createContext<RecoveryApi | null>(null);

/** Accès aux restaurations à signaler ; nécessite un `RecoveryProvider` parent. */
export function useRecovery(): RecoveryApi {
  const api = useContext(RecoveryContext);
  if (!api) {
    throw new Error("useRecovery doit être utilisé sous un RecoveryProvider");
  }
  return api;
}
