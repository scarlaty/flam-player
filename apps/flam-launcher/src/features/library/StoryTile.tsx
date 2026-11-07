import { memo } from "react";
import type { Source } from "../../api/sources";
import type { ListedEntry } from "./sourceData";
import { StoryCard, type StoryBadge } from "../../components";
import { t } from "../../i18n";
import { compatBadge } from "../compat/compatInfo";
import {
  defaultStoryState,
  STATE_APPEARANCE,
  type StoryHooks,
  type StoryState,
} from "./storyState";
import { useThumbnail } from "./useThumbnail";

export interface StoryTileProps extends StoryHooks {
  source: Source;
  entry: ListedEntry;
}

function badgeFor(state: StoryState): StoryBadge {
  return {
    id: state,
    label: t.library.state[state],
    ...STATE_APPEARANCE[state],
    ...(state === "update_available" && { description: t.stories.actions.updateHelp }),
    ...(state === "studio" && { description: t.stories.studioNotice }),
  };
}

/**
 * Badge de format d'une histoire STUdio dont l'état n'est pas déjà « Format STUdio » (installée,
 * en cours…) : elle ne se lit pas sur le PC et ne part que sur une Flam 2.x.
 */
function formatBadge(entry: ListedEntry, state: StoryState): StoryBadge[] {
  if (entry.kind !== "studio_zip" || state === "studio") return [];
  return [
    {
      id: "format",
      label: t.stories.luniiFormat,
      tone: "neutral",
      icon: "info",
      description: t.stories.studioNotice,
    },
  ];
}

/** Carte d'une histoire : vignette chargée à la demande, âge et état dans la source. */
export const StoryTile = memo(function StoryTile({
  source,
  entry,
  resolveState,
  compatFor,
  progressFor,
  renderActions,
  onOpenStory,
  selectedEntryId,
}: StoryTileProps) {
  const { url, observe } = useThumbnail(source, entry);
  const progress = progressFor?.(entry);
  const compat = compatFor?.(entry);
  const state: StoryState = progress
    ? "in_progress"
    : (resolveState?.(entry) ?? defaultStoryState(source, entry));

  return (
    <li ref={observe} className="fl-library__item" data-entry-id={entry.entry_id}>
      <StoryCard
        title={entry.title}
        coverUrl={url}
        ageMin={entry.age ?? undefined}
        badges={[
          badgeFor(state),
          ...formatBadge(entry, state),
          ...(compat ? [compatBadge(compat)] : []),
        ]}
        selected={selectedEntryId === entry.entry_id}
        onOpen={onOpenStory ? () => onOpenStory(entry) : undefined}
        progress={progress?.value}
        progressText={progress?.text}
        actions={renderActions?.(entry)}
      />
    </li>
  );
});
