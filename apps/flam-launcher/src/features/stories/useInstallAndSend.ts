import { formatOfKind } from "../../api/library";
import type { StoryEntry } from "../../api/story";
import { useDevice } from "../device/DeviceContext";
import { useSendBlock } from "../device/SendToDeviceButton";
import { useStories } from "./StoriesContext";

/**
 * « Installer et envoyer » (#609) pour une histoire non installée : l'action, ou `null` quand
 * elle n'est pas proposée. Elle ne l'est que si « Envoyer sur la Flam » serait possible
 * maintenant pour cette histoire (Flam branchée et lue, firmware accepté, format lisible par
 * cette Flam, histoire absente de la Flam, espace suffisant d'après la taille annoncée) : mêmes
 * règles que le bouton d'envoi, revérifiées à la fin de l'installation puis par le backend.
 *
 * L'action n'ajoute aucun chemin : elle lance l'installation habituelle, puis l'envoi par la file
 * d'envoi (`planSend`), qui ne part que sur la Flam branchée au moment du clic.
 */
export function useInstallAndSend(entry: StoryEntry): (() => void) | null {
  const stories = useStories();
  const { planSend, fulfilPlannedSend, dropPlannedSend, plannedSends } = useDevice();
  const format = formatOfKind(entry.kind);
  const block = useSendBlock(entry.uuid, entry.size, format);
  if (block !== null || plannedSends.some((plan) => plan.uuid === entry.uuid))
    return null;
  return () =>
    planSend({ uuid: entry.uuid, title: entry.title, format }, () =>
      stories.install(entry, {
        onSettled: (outcome) => {
          if (outcome.status === "installed")
            fulfilPlannedSend(entry.uuid, outcome.entry.size);
          else dropPlannedSend(entry.uuid);
        },
      }),
    );
}
