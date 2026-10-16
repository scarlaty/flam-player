import { useCompat } from "../compat/CompatContext";
import { LibraryScreen } from "../library/LibraryScreen";
import { StoryActions } from "./StoryActions";
import { storyStateOf, useStories } from "./StoriesContext";
import { StoryDetail } from "./StoryDetail";

/**
 * Bibliothèque reliée à l'installation, au lancement et à la compatibilité : badges, barre de
 * progression et actions rapides sur les cartes, fiche détaillée au clic sur le titre.
 */
export function StoriesLibrary() {
  const { compatFor } = useCompat();
  const stories = useStories();
  return (
    <LibraryScreen
      compatFor={compatFor}
      progressFor={stories.progressFor}
      reloadKey={stories.installedRevision}
      resolveState={(entry) => storyStateOf(stories, entry)}
      renderActions={(entry) => <StoryActions entry={entry} />}
      reopenEntry={stories.reopen ?? undefined}
      onReopened={stories.consumeReopen}
      renderDetail={(props) => <StoryDetail {...props} />}
    />
  );
}
