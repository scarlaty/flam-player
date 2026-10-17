import { useCallback, useEffect, useRef, useState } from "react";
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
 * depuis la fin de l'appel précédent ; un appel en vol n'est jamais doublé : « Réessayer » et
 * le retour au premier plan demandent un tour immédiat, exécuté dès la fin de l'appel en cours.
 * La fenêtre masquée ne scrute pas.
 */
export function useDeviceStatus(): DeviceStatusState {
  const [status, setStatus] = useState<DeviceStatus | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const pollNow = useRef<() => void>(() => undefined);

  useEffect(() => {
    let active = true;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let inFlight = false;
    let again = false;
    const poll = async () => {
      clearTimeout(timer);
      if (inFlight) {
        again = true;
        return;
      }
      inFlight = true;
      try {
        if (!document.hidden) {
          const next = await getDeviceStatus();
          if (active) {
            setStatus(next);
            setError(null);
          }
        }
      } catch (caught) {
        if (active) setError(toAppError(caught));
      } finally {
        inFlight = false;
      }
      if (!active) return;
      if (again) {
        again = false;
        void poll();
        return;
      }
      timer = setTimeout(() => void poll(), POLL_INTERVAL_MS);
    };
    pollNow.current = () => void poll();
    // Au retour au premier plan, la scrutation reprend aussitôt.
    const onVisible = () => {
      if (!document.hidden) pollNow.current();
    };
    document.addEventListener("visibilitychange", onVisible);
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, []);

  const retry = useCallback(() => pollNow.current(), []);
  return { status, error, retry };
}
