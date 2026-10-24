import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type RefObject,
} from "react";
import { toAppError, type AppError } from "../../api/error";
import {
  getPodcastFeed,
  refreshPodcast,
  type PodcastAdded,
  type PodcastView,
} from "../../api/podcast";
import { listSources, type Source } from "../../api/sources";
import { Banner, Button, EmptyState, useToast } from "../../components";
import { t } from "../../i18n";
import { PACK_HASH } from "../../router";
import { ErrorBanner } from "../ErrorBanner";
import { AddFeedForm } from "./AddFeedForm";
import { CompositionsSection } from "./CompositionsSection";
import { EpisodeList, SelectionSummary } from "./EpisodeList";
import "./PodcastsScreen.css";

type PodcastSource = Extract<Source, { type: "podcast" }>;

const isPodcast = (source: Source): source is PodcastSource => source.type === "podcast";

function formatDate(seconds: number | null): string | null {
  if (seconds === null) return null;
  return new Date(seconds * 1000).toLocaleDateString("fr-FR", { dateStyle: "long" });
}

/**
 * Flux de la source choisie : version enregistrée d'abord, puis actualisation réseau. `seed` porte
 * la vue qu'un ajout vient de rapporter : elle est affichée telle quelle, sans seconde requête.
 */
function useFeed(sourceId: string | null, seedRef: RefObject<PodcastView | null>) {
  const [view, setView] = useState<PodcastView | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const [refreshError, setRefreshError] = useState<AppError | null>(null);

  useEffect(() => {
    if (sourceId === null) return;
    let cancelled = false;
    const seeded = seedRef.current?.source_id === sourceId ? seedRef.current : null;
    // Chargement du flux choisi depuis le backend.
    if (seeded?.feed) {
      setView(seeded);
      setError(null);
      setRefreshError(null);
      setLoading(false);
      return;
    }
    setView(null);
    setError(null);
    setRefreshError(null);
    setLoading(true);
    void (async () => {
      let cached: PodcastView | null = null;
      try {
        cached = await getPodcastFeed(sourceId);
        if (cancelled) return;
        if (cached.feed) setView(cached);
      } catch (caught) {
        if (cancelled) return;
        setError(toAppError(caught));
      }
      try {
        const fresh = await refreshPodcast(sourceId);
        if (cancelled) return;
        setView(fresh);
        setError(null);
      } catch (caught) {
        if (cancelled) return;
        if (cached?.feed) setRefreshError(toAppError(caught));
        else setError(toAppError(caught));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [sourceId, seedRef]);

  return { view, loading, error, refreshError };
}

/**
 * Écran Podcasts : ajout d'un flux, liste de ses épisodes avec cases à cocher et plafonds du
 * pack (D1). La sélection est tenue par `PodcastSelectionProvider` pour l'éditeur de pack.
 */
export function PodcastsScreen() {
  const toast = useToast();
  const [feeds, setFeeds] = useState<PodcastSource[] | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [chosenId, setChosenId] = useState<string | null>(null);
  const episodesHeading = useRef<HTMLHeadingElement>(null);
  const focusOnLoad = useRef(false);
  const addedView = useRef<PodcastView | null>(null);

  const load = useCallback(async () => {
    setLoadError(null);
    try {
      const view = await listSources();
      const podcasts = view.sources.filter(isPodcast);
      setFeeds(podcasts);
      setChosenId((id) => id ?? podcasts[0]?.id ?? null);
    } catch (caught) {
      setLoadError(toAppError(caught));
    }
  }, []);

  useEffect(() => {
    // Chargement initial depuis le backend.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load();
  }, [load]);

  const chosen = feeds?.find((f) => f.id === chosenId) ?? null;
  const { view, loading, error, refreshError } = useFeed(chosen?.id ?? null, addedView);

  // Après un choix explicite, le focus va au titre de la liste d'épisodes. Effet de mise en page :
  // le focus est donné dans le rendu même qui montre la liste, jamais après (#489).
  useLayoutEffect(() => {
    if (focusOnLoad.current && view?.feed) {
      focusOnLoad.current = false;
      episodesHeading.current?.focus();
    }
  }, [view]);

  function onAdded(added: PodcastAdded) {
    const source = added.source;
    if (!isPodcast(source)) return;
    setFeeds((list) => [...(list ?? []).filter((f) => f.id !== source.id), source]);
    if (source.id !== chosenId) {
      focusOnLoad.current = true;
      addedView.current = added.view;
      setChosenId(source.id);
    }
    toast.show({ message: t.podcasts.add.added(source.name), tone: "success" });
  }

  const text = t.podcasts;
  return (
    <div className="fl-podcasts">
      <p className="fl-podcasts__intro">{text.intro}</p>
      <p>
        <a href={PACK_HASH}>{text.pack.link}</a>
      </p>

      <CompositionsSection />

      <AddFeedForm onAdded={onAdded} />

      <section className="fl-podcasts__section" aria-labelledby="fl-podcasts-feeds">
        <h2 id="fl-podcasts-feeds" className="fl-podcasts__heading">
          {text.feeds.title}
        </h2>
        {loadError && (
          <ErrorBanner
            error={loadError}
            action={
              <Button variant="secondary" size="sm" onClick={() => void load()}>
                {t.errors.retry}
              </Button>
            }
          />
        )}
        {feeds !== null && feeds.length === 0 && (
          <EmptyState
            icon="catalog"
            headingLevel={3}
            title={text.feeds.empty.title}
            description={text.feeds.empty.description}
          />
        )}
        {feeds !== null && feeds.length > 0 && (
          <ul className="fl-podcasts__feeds">
            {feeds.map((feed) => (
              <li key={feed.id} className="fl-podcasts__feed">
                <Button
                  variant={feed.id === chosenId ? "primary" : "secondary"}
                  aria-pressed={feed.id === chosenId}
                  onClick={() => {
                    // Le flux déjà choisi ne se recharge pas : le focus ne doit pas bouger.
                    if (feed.id === chosenId) return;
                    focusOnLoad.current = true;
                    addedView.current = null;
                    setChosenId(feed.id);
                  }}
                >
                  {feed.name}
                </Button>
                <span className="fl-podcasts__url">
                  <span className="fl-podcasts__url-label">{text.feeds.urlLabel}</span>{" "}
                  {feed.url}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      {chosen && (
        <section className="fl-podcasts__section" aria-labelledby="fl-podcasts-episodes">
          <h2
            id="fl-podcasts-episodes"
            ref={episodesHeading}
            tabIndex={-1}
            className="fl-podcasts__heading"
          >
            {text.episodes.title(view?.feed?.title ?? chosen.name)}
          </h2>
          <div role="status">{loading ? text.episodes.loading : ""}</div>
          {error && <ErrorBanner error={error} />}
          {(refreshError || view?.status.error) && !view?.status.offline && (
            <Banner tone="warning">
              <p>{text.episodes.refreshFailed}</p>
              {view?.status.error && <p>{view.status.error.message}</p>}
            </Banner>
          )}
          {view?.status.offline && (
            <Banner tone="info">
              <p>{text.episodes.offline(formatDate(view.status.updated_at))}</p>
            </Banner>
          )}
          {view?.feed && view.feed.warnings.length > 0 && (
            <Banner tone="warning">
              <p>{text.episodes.warnings(view.feed.warnings.length)}</p>
            </Banner>
          )}
          {/* Hors de la condition : la sélection multi-flux reste visible si ce flux est en erreur. */}
          <SelectionSummary />
          {view?.feed && (
            <EpisodeList
              key={chosen.id}
              sourceId={chosen.id}
              sourceName={chosen.name}
              episodes={view.feed.episodes}
            />
          )}
        </section>
      )}
    </div>
  );
}
