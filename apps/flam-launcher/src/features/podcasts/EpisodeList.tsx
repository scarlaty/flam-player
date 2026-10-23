import { useEffect, useId, useRef, useState } from "react";
import type { Episode } from "../../api/podcast";
import { Button, CheckboxField, EmptyState } from "../../components";
import { t } from "../../i18n";
import { formatBytes } from "../validation";
import { usePodcastSelection } from "./SelectionContext";
import {
  estimateSize,
  MAX_EPISODES,
  type Refusal,
  type SelectedEpisode,
} from "./selection";

/** Épisodes affichés d'un coup (un flux peut en compter des milliers). */
export const EPISODES_PAGE_SIZE = 50;

function formatDate(iso: string | null): string {
  if (iso === null) return t.podcasts.episodes.unknownDate;
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return t.podcasts.episodes.unknownDate;
  return date.toLocaleDateString("fr-FR", { dateStyle: "long", timeZone: "UTC" });
}

function refusalText(refusal: Refusal): string {
  const text = t.podcasts.refusal;
  switch (refusal) {
    case "count":
      return text.count(MAX_EPISODES);
    case "episodeTooBig":
      return text.episodeTooBig(t.podcasts.limits.episode);
    case "total":
      return text.total(t.podcasts.limits.pack);
    case "duplicate":
      return text.duplicate;
  }
}

/** Récapitulatif de la sélection ; région `status` annoncée poliment à chaque changement. */
export function SelectionSummary() {
  const { totals, clear } = usePodcastSelection();
  const text = t.podcasts.summary;
  return (
    <div className="fl-podcasts__summary">
      <div role="status" aria-label={text.title}>
        {totals.count === 0 ? (
          <p>{text.empty}</p>
        ) : (
          <>
            <p>{text.count(totals.count, MAX_EPISODES)}</p>
            <p>{text.size(formatBytes(totals.bytes), t.podcasts.limits.pack)}</p>
            {totals.unknownSize > 0 && <p>{text.unknown(totals.unknownSize)}</p>}
          </>
        )}
      </div>
      <Button variant="ghost" size="sm" disabled={totals.count === 0} onClick={clear}>
        {text.clear}
      </Button>
    </div>
  );
}

interface EpisodeListProps {
  sourceId: string;
  sourceName: string;
  episodes: readonly Episode[];
}

/** Épisodes d'un flux avec cases à cocher ; aucun n'est coché au départ (D8 : explicites compris). */
export function EpisodeList({ sourceId, sourceName, episodes }: EpisodeListProps) {
  const selection = usePodcastSelection();
  const [shown, setShown] = useState(EPISODES_PAGE_SIZE);
  const list = useRef<HTMLUListElement>(null);
  const focusFrom = useRef<number | null>(null);
  const capId = useId();
  const text = t.podcasts.episodes;

  // « Afficher plus » disparaît parfois avec le focus : il passe au premier épisode ajouté.
  useEffect(() => {
    if (focusFrom.current === null) return;
    const boxes =
      list.current?.querySelectorAll<HTMLInputElement>("input[type=checkbox]");
    boxes?.[focusFrom.current]?.focus();
    focusFrom.current = null;
  }, [shown]);

  if (episodes.length === 0) {
    return (
      <EmptyState
        icon="catalog"
        headingLevel={3}
        title={text.empty.title}
        description={text.empty.description}
      />
    );
  }

  const visible = episodes.slice(0, shown);
  // Plafonds du pack : un seul message pour toutes les cases refusées, pas un par case.
  const capRefusals = new Set<Refusal>();
  const rows = visible.map((episode) => {
    const size = estimateSize(episode);
    const candidate: SelectedEpisode = {
      id: episode.id,
      sourceId,
      sourceName,
      title: episode.title,
      bytes: size.bytes,
      explicit: episode.explicit === true,
    };
    const refusal = selection.refusal(candidate);
    if (refusal === "count" || refusal === "total") capRefusals.add(refusal);
    return { episode, size, candidate, refusal };
  });
  return (
    <>
      <div role="status" id={capId} className="fl-podcasts__cap">
        {[...capRefusals].map((refusal) => (
          <p key={refusal}>{refusalText(refusal)}</p>
        ))}
      </div>
      <ul ref={list} className="fl-podcasts__episodes" aria-label={text.list}>
        {rows.map(({ episode, size, candidate, refusal }) => {
          const perEpisode = refusal === "episodeTooBig";
          const details = [
            formatDate(episode.pub_date),
            episode.duration !== null && episode.duration > 0
              ? text.duration(Math.max(1, Math.round(episode.duration / 60)))
              : null,
            size.bytes === null
              ? text.unknownSize
              : text.size(formatBytes(size.bytes), size.approximate),
            candidate.explicit ? text.explicit : null,
            perEpisode ? refusalText(refusal) : null,
          ].filter((part): part is string => part !== null);
          return (
            <li key={episode.id} className="fl-podcasts__episode">
              {/* aria-disabled et non disabled : la case reste atteignable au clavier. */}
              <CheckboxField
                label={episode.title}
                hint={details.join(" · ")}
                describedBy={
                  refusal === "count" || refusal === "total" ? capId : undefined
                }
                checked={selection.isSelected(sourceId, episode.id)}
                aria-disabled={refusal !== null ? true : undefined}
                onChange={() => {
                  if (refusal === null || selection.isSelected(sourceId, episode.id)) {
                    selection.toggle(candidate);
                  }
                }}
              />
            </li>
          );
        })}
      </ul>
      {shown < episodes.length && (
        <Button
          variant="secondary"
          onClick={() => {
            focusFrom.current = shown;
            setShown((n) => n + EPISODES_PAGE_SIZE);
          }}
        >
          {text.showMore(episodes.length - shown)}
        </Button>
      )}
    </>
  );
}
