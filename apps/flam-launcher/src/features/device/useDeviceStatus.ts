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
 * Appel `device_status` en vol, partagé entre toutes les instances du hook : un remontage
 * (StrictMode, retour rapide sur l'écran) pendant un appel le rejoint au lieu d'en lancer un
 * second sur la même Flam.
 */
let shared: Promise<DeviceStatus> | null = null;

function fetchStatus(): Promise<DeviceStatus> {
  shared ??= getDeviceStatus().finally(() => {
    shared = null;
  });
  return shared;
}

/**
 * (`enabled` faux : pas de scrutation, l'état reste celui d'avant ; `intervalMs` : délai entre
 * deux appels, `POLL_INTERVAL_MS` par défaut.)
 *
 * Scrute l'état de la Flam toutes les `POLL_INTERVAL_MS` tant que le composant est monté : la
 * scrutation s'arrête au démontage (changement d'écran, lecture incrustée). Le délai court
 * depuis la fin de l'appel précédent ; un appel en vol n'est jamais doublé, même entre deux
 * montages : « Réessayer » et le retour au premier plan demandent un tour immédiat, exécuté dès
 * la fin de l'appel en cours. La fenêtre masquée ne scrute pas.
 */
export function useDeviceStatus(
  options: { enabled?: boolean; intervalMs?: number } = {},
): DeviceStatusState {
  const { enabled = true, intervalMs = POLL_INTERVAL_MS } = options;
  const interval = useRef(intervalMs);
  useEffect(() => {
    interval.current = intervalMs;
  });
  const [status, setStatus] = useState<DeviceStatus | null>(null);
  const [error, setError] = useState<AppError | null>(null);
  const pollNow = useRef<() => void>(() => undefined);

  useEffect(() => {
    if (!enabled) return;
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
          const next = await fetchStatus();
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
      timer = setTimeout(() => void poll(), interval.current);
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
  }, [enabled]);

  const retry = useCallback(() => pollNow.current(), []);
  return { status, error, retry };
}
