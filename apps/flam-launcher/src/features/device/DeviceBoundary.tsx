import { useMemo, type ReactNode } from "react";
import { useRoute } from "../../router";
import { useStories } from "../stories/StoriesContext";
import { DeviceProvider } from "./DeviceProvider";

/** Scrutation lente hors de l'écran « Ma Flam » (les boutons d'envoi n'ont pas besoin de plus). */
export const BACKGROUND_POLL_MS = 6000;

/**
 * Branche le `DeviceProvider` sur l'application : scrutation arrêtée pendant la lecture
 * incrustée, ralentie hors de l'écran « Ma Flam ».
 */
export function DeviceBoundary({ children }: { children: ReactNode }) {
  const route = useRoute();
  const { session, installed, installedRevision } = useStories();
  const libraryUuids = useMemo(
    () => new Set(installed.map((item) => item.uuid)),
    [installed],
  );
  return (
    <DeviceProvider
      paused={session?.embedded === true}
      intervalMs={route === "device" ? undefined : BACKGROUND_POLL_MS}
      // Tant que la bibliothèque n'a pas été lue, rien n'est retiré de la file.
      libraryUuids={installedRevision > 0 ? libraryUuids : undefined}
    >
      {children}
    </DeviceProvider>
  );
}
