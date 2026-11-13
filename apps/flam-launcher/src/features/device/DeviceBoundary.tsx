import { useCallback, useMemo, type ReactNode } from "react";
import { useToast } from "../../components";
import { t } from "../../i18n";
import { useRoute } from "../../router";
import { useStories } from "../stories/StoriesContext";
import type { PcCopyNotice } from "./DeviceContext";
import { DeviceProvider } from "./DeviceProvider";

/** Scrutation lente hors de l'écran « Ma Flam » (les boutons d'envoi n'ont pas besoin de plus). */
export const BACKGROUND_POLL_MS = 6000;

/**
 * Branche le `DeviceProvider` sur l'application : scrutation arrêtée pendant la lecture
 * incrustée, ralentie hors de l'écran « Ma Flam ».
 */
export function DeviceBoundary({ children }: { children: ReactNode }) {
  const route = useRoute();
  const toast = useToast();
  const { session, installed, installedRevision, refreshInstalled } = useStories();
  const libraryUuids = useMemo(
    () => new Set(installed.map((item) => item.uuid)),
    [installed],
  );
  // Histoire envoyée retirée de « Mes histoires », gardée faute d'écriture confirmée, ou retrait
  // impossible (#609).
  const onPcCopyChange = useCallback(
    (title: string, outcome: PcCopyNotice) => {
      const announce = t.deviceTransfer.panel.announce;
      if (outcome.status === "removed") {
        toast.show({ message: announce.removedFromPc(title), tone: "success" });
        void refreshInstalled();
      } else if (outcome.status === "kept_unconfirmed") {
        toast.show({ message: announce.keptUnconfirmed(title) });
      } else {
        toast.show({
          message: announce.removeFromPcFailed(title, outcome.message),
          tone: "danger",
        });
      }
    },
    [toast, refreshInstalled],
  );
  return (
    <DeviceProvider
      paused={session?.embedded === true}
      intervalMs={route === "device" ? undefined : BACKGROUND_POLL_MS}
      // Tant que la bibliothèque n'a pas été lue, rien n'est retiré de la file.
      libraryUuids={installedRevision > 0 ? libraryUuids : undefined}
      onPcCopyChange={onPcCopyChange}
    >
      {children}
    </DeviceProvider>
  );
}
