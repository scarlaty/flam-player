import { useState } from "react";
import type { StoryEntry } from "../../api/story";
import { Button, IconButton } from "../../components";
import { t } from "../../i18n";
import { SendToDeviceButton } from "../device/SendToDeviceButton";
import { isRemote, useStories } from "./StoriesContext";
import { RemoveStoryDialog } from "./RemoveStoryDialog";

/**
 * Actions rapides d'une carte : « Jouer », « Envoyer sur la Flam » et « Supprimer » (bouton icône
 * compact, libellé et info-bulle) si l'histoire est installée, sinon « Télécharger » ou
 * « Installer » ; rien pendant qu'une tâche est active (la carte montre sa progression).
 */
export function StoryActions({ entry }: { entry: StoryEntry }) {
  const stories = useStories();
  const text = t.stories.actions;
  const [confirmRemove, setConfirmRemove] = useState(false);
  const [removing, setRemoving] = useState(false);
  if (stories.activeTaskFor(entry)) return null;
  const installed = stories.installedFor(entry);
  if (installed) {
    const { session } = stories;
    const playingThis =
      session !== null &&
      session.uuid === (session.origin === "library" ? entry.uuid : entry.entry_id);
    return (
      <>
        <Button
          size="sm"
          variant="accent"
          icon="play"
          disabled={stories.session !== null}
          aria-label={text.forStory(text.play, entry.title)}
          onClick={() => stories.play(entry)}
        >
          {text.play}
        </Button>
        <SendToDeviceButton
          uuid={installed.uuid}
          title={entry.title}
          size={installed.size}
          stale={stories.sourceChanged(installed.uuid)}
        />
        <IconButton
          size="sm"
          variant="danger-quiet"
          icon="trash"
          disabled={playingThis || removing}
          aria-busy={removing || undefined}
          label={text.forStory(text.remove, entry.title)}
          onClick={() => setConfirmRemove(true)}
        />
        <RemoveStoryDialog
          open={confirmRemove}
          title={entry.title}
          onCancel={() => setConfirmRemove(false)}
          onConfirm={(purge) => {
            setConfirmRemove(false);
            setRemoving(true);
            void stories
              .remove(installed.uuid, entry.title, purge)
              .finally(() => setRemoving(false));
          }}
        />
      </>
    );
  }
  const label = isRemote(entry) ? text.download : text.install;
  return (
    <Button
      size="sm"
      icon="download"
      aria-label={text.forStory(label, entry.title)}
      onClick={() => stories.install(entry)}
    >
      {label}
    </Button>
  );
}
