import type { StoryEntry } from "../../api/story";
import { Button } from "../../components";
import { t } from "../../i18n";
import { SendToDeviceButton } from "../device/SendToDeviceButton";
import { isRemote, useStories } from "./StoriesContext";

/**
 * Action rapide d'une carte : « Jouer » si l'histoire est installée, sinon « Télécharger » ou
 * « Installer » ; rien pendant qu'une tâche est active (la carte montre sa progression).
 */
export function StoryActions({ entry }: { entry: StoryEntry }) {
  const stories = useStories();
  const text = t.stories.actions;
  if (stories.activeTaskFor(entry)) return null;
  const installed = stories.installedFor(entry);
  if (installed) {
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
