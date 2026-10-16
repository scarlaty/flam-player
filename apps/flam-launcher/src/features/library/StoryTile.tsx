import { memo } from "react";
import type { Source } from "../../api/sources";
import type { StoryEntry } from "../../api/story";
import { StoryCard, type StoryBadge } from "../../components";
import { t } from "../../i18n";
import {
  defaultStoryState,
  STATE_APPEARANCE,
  type StoryHooks,
  type StoryState,
} from "./storyState";
import { useThumbnail } from "./useThumbnail";

export interface StoryTileProps extends StoryHooks {
  source: Source;
  entry: StoryEntry;
}

function badgeFor(state: StoryState): StoryBadge {
  return { id: state, label: t.library.state[state], ...STATE_APPEARANCE[state] };
}

/** Carte d'une histoire : vignette chargée à la demande, âge et état dans la source. */
export const StoryTile = memo(function StoryTile({
  source,
  entry,
  resolveState,
  progressFor,
  renderActions,
  onOpenStory,
  selectedEntryId,
}: StoryTileProps) {
  const { url, observe } = useThumbnail(source, entry);
  const progress = progressFor?.(entry);
  const state: StoryState = progress
    ? "in_progress"
    : (resolveState?.(entry) ?? defaultStoryState(source, entry));

  return (
    <li ref={observe} className="fl-library__item">
      <StoryCard
        title={entry.title}
        coverUrl={url}
        ageMin={entry.age ?? undefined}
        badges={[badgeFor(state)]}
        selected={selectedEntryId === entry.entry_id}
        onOpen={onOpenStory ? () => onOpenStory(entry) : undefined}
        progress={progress?.value}
        progressText={progress?.text}
        actions={renderActions?.(entry)}
      />
    </li>
  );
});
