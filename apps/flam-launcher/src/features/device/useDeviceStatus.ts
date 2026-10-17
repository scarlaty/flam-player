import { useCallback, useEffect, useState } from "react";
import { toAppError, type AppError } from "../../api/error";
import { getDeviceStatus, type DeviceStatus } from "../../api/device";

/** Intervalle de scrutation de la Flam, tant que l'écran est ouvert. */
export const POLL_INTERVAL_MS = 2000;

export interface DeviceStatusState {
  /** Dernier état connu ; `null` avant la première réponse. */
  status: DeviceStatus | null;
  /** Échec du dernier appel (effacé au premier succès suivant). */
  error: AppError | null;
  /** Relance aussitôt la scrutation (bouton « Réessayer »). */
  retry: () => void;
}

/**
 * Scrute l'état de la Flam toutes les `POLL_INTERVAL_MS` tant que le composant est monté : la
 * scrutation s'arrête au démontage (changement d'écran, lecture incrustée). Le délai court
 * depuis la fin de l'appel précédent, donc deux appels ne se chevauchent jamais ; la fenêtre
 * masquée ne scrute pas.
 */
export function useDeviceStatus(): DeviceStatusState {
  const [status, setStatus] = useState<DeviceStatus | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const [run, setRun] = useState(0);

  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      if (!document.hidden) {
        try {
          const next = await getDeviceStatus();
          if (!active) return;
          setStatus(next);
          setError(null);
        } catch (caught) {
          if (!active) return;
          setError(toAppError(caught));
        }
      }
      if (active) timer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
    };
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, [run]);

  const retry = useCallback(() => setRun((n) => n + 1), []);
  return { status, error, retry };
}
