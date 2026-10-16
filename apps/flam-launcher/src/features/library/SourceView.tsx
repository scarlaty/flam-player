import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { toAppError, type AppError } from "../../api/error";
import type { Source } from "../../api/sources";
import {
  Badge,
  Banner,
  Button,
  EmptyState,
  SelectField,
  TextField,
  useToast,
} from "../../components";
import { t } from "../../i18n";
import { ROUTE_HASHES } from "../../router";
import { ErrorBanner } from "../ErrorBanner";
import { useLibrary } from "./LibraryContext";
import {
  applyFilters,
  availableAges,
  availableCategories,
  DEFAULT_FILTERS,
  filtersAreDefault,
  type LibraryFilters,
  type SortKey,
} from "./filters";
import { loadSourceData, summarize, type SourceData } from "./sourceData";
import { StoryTile } from "./StoryTile";
import type { StoryHooks } from "./storyState";
import "./SourceView.css";

/** Nombre de cartes affichées d'un coup (une source peut compter des milliers d'histoires). */
export const PAGE_SIZE = 60;
/** Éléments ignorés détaillés dans le bandeau d'avertissement. */
const MAX_WARNINGS_SHOWN = 20;

function formatDate(seconds: number | null): string | null {
  if (seconds === null) return null;
  return new Date(seconds * 1000).toLocaleDateString("fr-FR", { dateStyle: "long" });
}

const ALL = "";
/** Délai avant d'annoncer le nombre de résultats, pour ne pas lire chaque frappe. */
export const COUNT_ANNOUNCE_DELAY_MS = 400;

/**
 * Texte annoncé : la première valeur non vide l'est aussitôt, les suivantes seulement
 * après `delay` ms sans changement.
 */
function useDebouncedAnnouncement(value: string, delay: number): string {
  const [announced, setAnnounced] = useState(value);
  if (announced === "" && value !== "") setAnnounced(value);
  useEffect(() => {
    const timer = setTimeout(() => setAnnounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return announced;
}

export interface SourceViewProps extends StoryHooks {
  source: Source;
}

/**
 * Contenu d'une source : grille de cartes, recherche, filtres (âge, catégorie), tri,
 * états de chargement, d'erreur, hors ligne et de liste vide, rafraîchissement manuel.
 */
export function SourceView({ source, ...hooks }: SourceViewProps) {
  const { renderDetail, reloadKey } = hooks;
  const toast = useToast();
  const { report } = useLibrary();
  const [data, setData] = useState<SourceData | null>(null);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [loading, setLoading] = useState(true);
  const [filters, setFilters] = useState<LibraryFilters>(DEFAULT_FILTERS);
  const [shown, setShown] = useState<{ key: string; count: number }>({
    key: "",
    count: PAGE_SIZE,
  });
  const [openId, setOpenId] = useState<string | null>(null);
  const latest = useRef(0);
  const gridRef = useRef<HTMLUListElement>(null);
  const focusFrom = useRef<number | null>(null);
  // Le chargement ne dépend que de l'identité de la source, pas de l'objet entier
  // (qui change de référence quand la liste des sources est rechargée).
  const sourceRef = useRef(source);
  useEffect(() => {
    sourceRef.current = source;
  });
  const { id: sourceId, type: sourceType } = source;

  const load = useCallback(
    async (refresh: boolean) => {
      const source = sourceRef.current;
      const request = ++latest.current;
      setLoading(true);
      setLoadError(null);
      try {
        const result = await loadSourceData(source, refresh);
        if (request !== latest.current) return;
        setData(result);
        report(source.id, summarize(result));
        if (refresh) {
          toast.show(
            result.offline
              ? { message: t.library.refreshedOffline(source.name) }
              : { message: t.library.refreshed(source.name), tone: "success" },
          );
        }
      } catch (caught) {
        if (request !== latest.current) return;
        const error = toAppError(caught);
        setLoadError(error);
        report(source.id, { status: "error" });
        if (refresh) toast.show({ message: error.message, tone: "danger" });
      } finally {
        if (request === latest.current) setLoading(false);
      }
    },
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sourceId, sourceType, report, toast, sourceType === "library" ? reloadKey : 0],
  );

  useEffect(() => {
    // Chargement initial depuis le backend.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void load(false);
    const counter = latest;
    return () => {
      counter.current++;
    };
  }, [load]);

  const entries = data?.entries;
  const openEntry =
    openId === null ? undefined : entries?.find((e) => e.entry_id === openId);
  // Avec une fiche détaillée, un clic sur le titre d'une carte l'ouvre.
  const tileHooks: StoryHooks = renderDetail
    ? {
        ...hooks,
        onOpenStory: (entry) => setOpenId(entry.entry_id),
        selectedEntryId: openEntry?.entry_id,
      }
    : hooks;
  const ages = useMemo(() => availableAges(entries ?? []), [entries]);
  const categories = useMemo(() => availableCategories(entries ?? []), [entries]);
  const visible = useMemo(() => applyFilters(entries ?? [], filters), [entries, filters]);

  const filterKey = JSON.stringify(filters);
  const limit = shown.key === filterKey ? shown.count : PAGE_SIZE;
  const page = visible.slice(0, limit);

  const countText = entries ? t.library.resultCount(visible.length, entries.length) : "";
  const announced = useDebouncedAnnouncement(countText, COUNT_ANNOUNCE_DELAY_MS);

  useEffect(() => {
    // Après « Afficher plus », le focus passe à la première nouvelle carte.
    const from = focusFrom.current;
    if (from === null) return;
    focusFrom.current = null;
    const item = gridRef.current?.children[from];
    item?.querySelector<HTMLElement>("button, a[href], [tabindex]")?.focus();
  }, [page.length]);

  function update(patch: Partial<LibraryFilters>) {
    setFilters((current) => ({ ...current, ...patch }));
  }

  const refreshButton = (
    <Button
      icon="refresh"
      disabled={loading}
      onClick={() => void load(true)}
      aria-busy={loading || undefined}
    >
      {loading ? t.library.refreshing : t.library.refresh}
    </Button>
  );

  return (
    <section className="fl-library" aria-labelledby="fl-library-title">
      <div className="fl-library__head">
        <h2 id="fl-library-title" className="fl-library__title">
          {source.name}
        </h2>
        <Badge>{t.sources.kind[source.type]}</Badge>
        {source.type !== "library" && refreshButton}
      </div>

      {loadError && (
        <ErrorBanner
          error={loadError}
          action={<Button onClick={() => void load(false)}>{t.errors.retry}</Button>}
        />
      )}
      {!data && loading && <p>{t.library.loading}</p>}

      {data && <Notices data={data} />}

      {data && data.entries.length > 0 && (
        <>
          <form
            className="fl-library__filters"
            role="search"
            aria-label={t.library.filtersLabel}
            onSubmit={(event) => event.preventDefault()}
          >
            <TextField
              className="fl-library__search"
              type="search"
              label={t.library.search}
              placeholder={t.library.searchPlaceholder}
              value={filters.query}
              onChange={(event) => update({ query: event.target.value })}
            />
            <SelectField
              label={t.library.age}
              value={filters.age === null ? ALL : String(filters.age)}
              onChange={(event) =>
                update({
                  age: event.target.value === ALL ? null : Number(event.target.value),
                })
              }
              options={[
                { value: ALL, label: t.library.allAges },
                ...ages.map((age) => ({
                  value: String(age),
                  label: t.library.ageOption(age),
                })),
              ]}
            />
            <SelectField
              label={t.library.category}
              value={filters.category ?? ALL}
              onChange={(event) =>
                update({
                  category: event.target.value === ALL ? null : event.target.value,
                })
              }
              options={[
                { value: ALL, label: t.library.allCategories },
                ...categories.map((category) => ({ value: category, label: category })),
              ]}
            />
            <SelectField
              label={t.library.sort}
              value={filters.sort}
              onChange={(event) => update({ sort: event.target.value as SortKey })}
              options={[
                { value: "title", label: t.library.sortOptions.title },
                { value: "recent", label: t.library.sortOptions.recent },
              ]}
            />
          </form>
          <p role="status" className="fl-library__count">
            {announced}
          </p>
        </>
      )}

      {data && data.entries.length === 0 && <EmptyForSource source={source} />}

      {data && data.entries.length > 0 && visible.length === 0 && (
        <EmptyState
          icon="sparkle"
          headingLevel={3}
          title={t.library.empty.noResults.title}
          description={t.library.empty.noResults.description}
          action={
            !filtersAreDefault(filters) && (
              <Button onClick={() => setFilters(DEFAULT_FILTERS)}>
                {t.library.resetFilters}
              </Button>
            )
          }
        />
      )}

      {page.length > 0 && (
        <>
          <ul
            ref={gridRef}
            className="fl-library__grid"
            aria-label={t.library.gridLabel(source.name)}
          >
            {page.map((entry) => (
              <StoryTile
                key={entry.entry_id}
                source={source}
                entry={entry}
                {...tileHooks}
              />
            ))}
          </ul>
          {visible.length > page.length && (
            <div className="fl-library__more">
              <Button
                onClick={() => {
                  focusFrom.current = page.length;
                  setShown({ key: filterKey, count: limit + PAGE_SIZE });
                }}
              >
                {t.library.showMore(visible.length - page.length)}
              </Button>
            </div>
          )}
        </>
      )}
      {openEntry &&
        renderDetail?.({ source, entry: openEntry, onClose: () => setOpenId(null) })}
    </section>
  );
}

/** Bandeaux d'état du contenu : hors ligne, échec du rafraîchissement, éléments ignorés. */
function Notices({ data }: { data: SourceData }) {
  return (
    <>
      {data.offline && (
        <Banner tone="info" title={t.library.offlineTitle}>
          <p>{t.library.offlineBody(formatDate(data.updatedAt))}</p>
        </Banner>
      )}
      {data.problem && !data.offline && (
        <Banner tone="warning" title={t.library.problemTitle}>
          <p>{data.problem.message}</p>
        </Banner>
      )}
      {data.warnings.length > 0 && (
        <Banner tone="warning" title={t.library.warningsTitle(data.warnings.length)}>
          <details>
            <summary>{t.library.warningsHint}</summary>
            <ul className="fl-library__warnings">
              {data.warnings.slice(0, MAX_WARNINGS_SHOWN).map((warning, index) => (
                <li key={index}>{warning}</li>
              ))}
            </ul>
          </details>
        </Banner>
      )}
    </>
  );
}

function EmptyForSource({ source }: { source: Source }) {
  const text = t.library.empty[source.type];
  return (
    <EmptyState
      icon={source.type === "library" ? "library" : source.type}
      headingLevel={3}
      title={text.title}
      description={text.description}
      action={
        source.type === "library" && (
          <a className="fl-library__link" href={ROUTE_HASHES.sources}>
            {t.library.empty.library.action}
          </a>
        )
      }
    />
  );
}
