import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { toAppError, type AppError } from "../../../api/error";
import {
  readPackDraft,
  type EpisodeRef,
  type PackComposition,
  type PackNode,
  type Problem,
} from "../../../api/pack";
import {
  Banner,
  Button,
  EmptyState,
  SelectField,
  TextField,
  type SelectOption,
} from "../../../components";
import { t } from "../../../i18n";
import { ROUTE_HASHES } from "../../../router";
import { ErrorBanner } from "../../ErrorBanner";
import { formatBytes } from "../../validation";
import { MAX_EPISODES } from "../selection";
import { usePodcastSelection } from "../SelectionContext";
import { useDebouncedValidation, useDraftAutosave, useFeedInfo } from "./hooks";
import { PackTree, menuLabel } from "./PackTree";
import {
  MAX_MENU_DEPTH,
  MAX_OPTIONS_PER_MENU,
  append,
  canPlace,
  containsEpisode,
  episodeNode,
  episodesOf,
  menuAt,
  menuHeight,
  sameEpisode,
} from "./tree";
import "./PackEditor.css";

/** Titre et sous-titre, en caractères (miroir de `MAX_TITLE_CHARS`). */
const MAX_TITLE_CHARS = 256;
const MAX_AGE = 18;
const AGES = Array.from({ length: MAX_AGE + 1 }, (_, age) => age);

const emptyComposition = (menu: PackNode[]): PackComposition => ({
  metadata: { title: "", subtitle: null, age: 0, description: null },
  cover: { kind: "default" },
  menu,
});

function problemText(problem: Problem, menu: readonly PackNode[]): string {
  const text = t.podcasts.pack.problems;
  const limits = t.podcasts.limits;
  const message = ((): string => {
    switch (problem.code) {
      case "no_episode":
        return text.noEpisode;
      case "too_many_episodes":
        return text.tooManyEpisodes(MAX_EPISODES);
      case "too_deep":
        return text.tooDeep(MAX_MENU_DEPTH);
      case "too_many_options":
        return text.tooManyOptions(MAX_OPTIONS_PER_MENU);
      case "empty_category":
        return text.emptyCategory;
      case "invalid_category_name":
        return text.invalidCategoryName(40);
      case "duplicate_episode":
        return text.duplicateEpisode;
      case "invalid_id":
        return text.invalidId;
      case "title_required":
        return text.titleRequired;
      case "text_too_long":
        return text.textTooLong;
      case "episode_unavailable":
        return text.episodeUnavailable;
      case "episode_too_large":
        return text.episodeTooLarge(limits.episode);
      case "pack_too_large":
        return text.packTooLarge(limits.pack);
      case "cover_unavailable":
        return text.coverUnavailable;
      default:
        return problem.message;
    }
  })();
  return problem.path === null
    ? message
    : `${message} ${text.at(menuLabel(menu, categoryPrefix(menu, problem.path)))}`;
}

/** Plus long préfixe du chemin qui ne traverse que des catégories (le menu concerné). */
function categoryPrefix(menu: readonly PackNode[], path: readonly number[]): number[] {
  const out: number[] = [];
  let level = menu;
  for (const index of path) {
    const node = level[index];
    if (node?.kind !== "category") break;
    out.push(index);
    level = node.entries;
  }
  return out;
}

const refKey = (ref: EpisodeRef) => `${ref.source_id}\u0000${ref.episode_id}`;

export interface PackEditorProps {
  /** Délai de la sauvegarde automatique du brouillon, en ms. */
  saveDelayMs?: number;
  /** Délai entre la dernière modification et l'appel à `pack_validate`, en ms. */
  validateDelayMs?: number;
}

/**
 * Éditeur de pack (#363) : arborescence libre des épisodes sélectionnés (#212), titre,
 * sous-titre, âge, description, couverture, brouillon enregistré automatiquement (#362).
 */
export function PackEditor({
  saveDelayMs = 800,
  validateDelayMs = 700,
}: PackEditorProps) {
  const text = t.podcasts.pack;
  const selection = usePodcastSelection();
  const initialSelection = useRef(selection.selected);
  const [attempt, setAttempt] = useState(0);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<AppError | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [composition, setComposition] = useState<PackComposition | null>(null);
  const [announcement, setAnnouncement] = useState("");
  const { status, init: initDraft, schedule } = useDraftAutosave(saveDelayMs);
  const validation = useDebouncedValidation(composition, validateDelayMs);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const view = await readPackDraft();
        if (cancelled) return;
        initDraft(view.revision);
        setComposition(
          view.composition ??
            emptyComposition(
              initialSelection.current.map((s) =>
                episodeNode({ source_id: s.sourceId, episode_id: s.id }),
              ),
            ),
        );
        setNotice(view.notice);
        setLoadError(null);
      } catch (caught) {
        if (!cancelled) setLoadError(toAppError(caught));
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [attempt, initDraft]);

  const menu = composition?.menu;
  const packEpisodes = useMemo(() => episodesOf(menu ?? []), [menu]);
  const sourceIds = useMemo(
    () => [
      ...new Set([
        ...packEpisodes.map((e) => e.source_id),
        ...selection.selected.map((s) => s.sourceId),
      ]),
    ],
    [packEpisodes, selection.selected],
  );
  const feeds = useFeedInfo(sourceIds);

  const titleOf = useCallback(
    (ref: EpisodeRef): string | null =>
      feeds.get(ref.source_id)?.episodes.get(ref.episode_id)?.title ??
      selection.selected.find(
        (s) => s.sourceId === ref.source_id && s.id === ref.episode_id,
      )?.title ??
      null,
    [feeds, selection.selected],
  );
  const unavailable = useMemo(
    () => new Set((validation.report?.unavailable ?? []).map(refKey)),
    [validation.report],
  );

  if (loading) return <p role="status">{text.loading}</p>;
  if (loadError || !composition) {
    return (
      <div className="fl-pack">
        <ErrorBanner
          error={loadError ?? { kind: "unknown", message: text.loadFailed }}
          action={
            <Button
              variant="secondary"
              size="sm"
              onClick={() => {
                setLoading(true);
                setAttempt((n) => n + 1);
              }}
            >
              {t.errors.retry}
            </Button>
          }
        />
      </div>
    );
  }

  function edit(next: PackComposition, message = "") {
    setComposition(next);
    schedule(next);
    if (message !== "") setAnnouncement(message);
  }
  const current = composition;
  const editMetadata = (change: Partial<PackComposition["metadata"]>) =>
    edit({ ...current, metadata: { ...current.metadata, ...change } });

  const pool = selection.selected.filter(
    (s) => !containsEpisode(current.menu, { source_id: s.sourceId, episode_id: s.id }),
  );
  const root = menuAt(current.menu, [])!;
  const rootFull = !canPlace(
    current.menu,
    [],
    episodeNode({ source_id: "", episode_id: "" }),
  );

  function addToRoot(selected: (typeof pool)[number]) {
    const ref = { source_id: selected.sourceId, episode_id: selected.id };
    if (!canPlace(current.menu, [], episodeNode(ref))) return;
    edit(
      { ...current, menu: append(current.menu, [], episodeNode(ref)) },
      text.announce.added(selected.title, text.tree.root),
    );
  }

  function addAllToRoot() {
    let next = current.menu as readonly PackNode[];
    for (const s of pool) {
      const node = episodeNode({ source_id: s.sourceId, episode_id: s.id });
      if (!canPlace(next, [], node)) break;
      next = append(next, [], node);
    }
    edit({ ...current, menu: [...next] });
  }

  // Défauts D16 : repris du flux quand le pack n'en utilise qu'un.
  const feedIds = [...new Set(packEpisodes.map((e) => e.source_id))];
  const single = feedIds.length === 1 ? (feeds.get(feedIds[0]!) ?? null) : null;
  const problemsOf = (field: Problem["field"]) =>
    validation.report?.problems.find((p) => p.field === field) ?? null;
  const fieldError = (field: Problem["field"]) => {
    const problem = problemsOf(field);
    return problem ? problemText(problem, current.menu) : null;
  };

  // Couverture (N6 b) : images des flux et des épisodes du pack.
  const coverRefs = packEpisodes.filter(
    (e, i, all) =>
      feeds.get(e.source_id)?.episodes.get(e.episode_id)?.has_image === true &&
      all.findIndex((o) => sameEpisode(o, e)) === i,
  );
  const coverFeeds = feedIds.filter((id) => feeds.get(id)?.hasImage === true);
  const coverOptions: SelectOption[] = [
    { value: "default", label: text.cover.default },
    ...coverFeeds.map((id) => ({
      value: `feed:${id}`,
      label: text.cover.feed(feeds.get(id)!.title),
    })),
    ...coverRefs.map((ref, i) => ({
      value: `episode:${i}`,
      label: text.cover.episode(titleOf(ref) ?? t.podcasts.pack.tree.unknownTitle),
    })),
  ];
  const cover = current.cover;
  let coverValue = "default";
  if (cover.kind === "feed") coverValue = `feed:${cover.source_id}`;
  if (cover.kind === "episode") {
    const i = coverRefs.findIndex((r) => sameEpisode(r, cover));
    coverValue = i >= 0 ? `episode:${i}` : "missing";
  }
  if (!coverOptions.some((o) => o.value === coverValue)) {
    coverOptions.push({ value: "missing", label: text.cover.missing });
    coverValue = "missing";
  }

  const report = validation.report;
  const levels = menuHeight(current.menu);
  const descriptionError = fieldError("description");

  return (
    <div className="fl-pack">
      <p className="fl-pack__intro">{text.intro}</p>
      <p>
        <a href={ROUTE_HASHES.podcasts}>{text.back}</a>
      </p>
      {notice && (
        <Banner tone="warning">
          <p>{notice}</p>
        </Banner>
      )}
      <p className="fl-pack__limits">
        {text.limits(MAX_OPTIONS_PER_MENU, MAX_MENU_DEPTH)}
      </p>

      <section className="fl-pack__section" aria-labelledby="fl-pack-summary">
        <h2 id="fl-pack-summary" className="fl-pack__heading">
          {text.summary.title}
        </h2>
        <div role="status">
          <p>{text.summary.episodes(packEpisodes.length, MAX_EPISODES)}</p>
          <p>{text.summary.levels(levels, MAX_MENU_DEPTH)}</p>
          {report && (
            <>
              <p>
                {text.summary.size(
                  formatBytes(report.estimated_bytes),
                  t.podcasts.limits.pack,
                )}
              </p>
              {report.unknown_size > 0 && (
                <p>{text.summary.unknown(report.unknown_size)}</p>
              )}
            </>
          )}
        </div>
        <p className="fl-pack__status">
          {status === "saving" && text.draftSaving}
          {status === "saved" && text.draftSaved}
        </p>
        {status === "error" && (
          <Banner tone="danger">
            <p>{text.draftFailed}</p>
          </Banner>
        )}
      </section>

      {report && report.unavailable.length > 0 && (
        <Banner tone="warning">
          <p>{text.unavailableBanner(report.unavailable.length)}</p>
        </Banner>
      )}

      <section className="fl-pack__section" aria-labelledby="fl-pack-problems">
        <h2 id="fl-pack-problems" className="fl-pack__heading">
          {text.problems.title}
        </h2>
        <div aria-live="polite">
          {validation.error ? (
            <p>{text.validateFailed}</p>
          ) : report === null ? (
            <p>{text.checking}</p>
          ) : report.problems.length === 0 ? (
            <p>{validation.pending ? text.checking : text.problems.ok}</p>
          ) : (
            <ul className="fl-pack__problems">
              {report.problems.map((problem, i) => (
                <li key={i}>{problemText(problem, current.menu)}</li>
              ))}
            </ul>
          )}
        </div>
      </section>

      {pool.length > 0 && (
        <section className="fl-pack__section" aria-labelledby="fl-pack-pool">
          <h2 id="fl-pack-pool" className="fl-pack__heading">
            {text.pool.title}
          </h2>
          <ul className="fl-pack__pool">
            {pool.map((s) => (
              <li key={`${s.sourceId}/${s.id}`} className="fl-pack__pool-item">
                <span>{s.title}</span>
                <Button
                  variant="secondary"
                  size="sm"
                  icon="plus"
                  disabled={rootFull}
                  aria-label={text.pool.addRoot(s.title)}
                  onClick={() => addToRoot(s)}
                >
                  {text.pool.addRoot(s.title)}
                </Button>
              </li>
            ))}
          </ul>
          <Button variant="ghost" size="sm" disabled={rootFull} onClick={addAllToRoot}>
            {text.pool.addAll}
          </Button>
        </section>
      )}

      <section className="fl-pack__section" aria-labelledby="fl-pack-tree">
        <h2 id="fl-pack-tree" className="fl-pack__heading">
          {text.tree.title}
        </h2>
        <p className="fl-pack__hint">{text.tree.hint}</p>
        {root.length === 0 && (
          <EmptyState
            icon="catalog"
            headingLevel={3}
            title={text.tree.empty.title}
            description={text.tree.empty.description}
          />
        )}
        <PackTree
          menu={current.menu}
          titleOf={titleOf}
          isUnavailable={(ref) => unavailable.has(refKey(ref))}
          onChange={(next, message) => {
            edit({ ...current, menu: next }, message);
          }}
          onRefuse={setAnnouncement}
        />
        <div role="status" className="fl-pack__announce">
          {announcement}
        </div>
      </section>

      <section className="fl-pack__section" aria-labelledby="fl-pack-meta">
        <h2 id="fl-pack-meta" className="fl-pack__heading">
          {text.metadata.title}
        </h2>
        <TextField
          label={text.metadata.storyTitle}
          hint={text.metadata.titleHint(
            feedIds.length > 1 ? null : (single?.title ?? null),
          )}
          value={current.metadata.title}
          maxLength={MAX_TITLE_CHARS}
          error={fieldError("title")}
          onChange={(event) => editMetadata({ title: event.target.value })}
        />
        <TextField
          label={text.metadata.subtitle}
          hint={text.metadata.subtitleHint(single?.copyright ?? null)}
          value={current.metadata.subtitle ?? ""}
          maxLength={MAX_TITLE_CHARS}
          error={fieldError("subtitle")}
          onChange={(event) =>
            editMetadata({
              subtitle: event.target.value === "" ? null : event.target.value,
            })
          }
        />
        <SelectField
          label={text.metadata.age}
          value={String(current.metadata.age)}
          options={(AGES.includes(current.metadata.age)
            ? AGES
            : [...AGES, current.metadata.age]
          ).map((age) => ({ value: String(age), label: text.metadata.ageOption(age) }))}
          onChange={(event) => editMetadata({ age: Number(event.target.value) })}
        />
        <DescriptionField
          value={current.metadata.description ?? ""}
          error={descriptionError}
          onChange={(value) => editMetadata({ description: value === "" ? null : value })}
        />
      </section>

      <section className="fl-pack__section" aria-labelledby="fl-pack-cover">
        <h2 id="fl-pack-cover" className="fl-pack__heading">
          {text.cover.title}
        </h2>
        <p className="fl-pack__hint">{text.cover.hint}</p>
        <SelectField
          label={text.cover.label}
          value={coverValue}
          options={coverOptions}
          onChange={(event) => {
            const value = event.target.value;
            if (value === "default") edit({ ...current, cover: { kind: "default" } });
            else if (value.startsWith("feed:"))
              edit({ ...current, cover: { kind: "feed", source_id: value.slice(5) } });
            else if (value.startsWith("episode:")) {
              const ref = coverRefs[Number(value.slice(8))];
              if (ref) edit({ ...current, cover: { kind: "episode", ...ref } });
            }
          }}
        />
      </section>

      <section className="fl-pack__section fl-pack__create">
        {/* TODO #217 : brancher ce bouton sur la création de l'histoire (installation). */}
        <Button variant="primary" disabled aria-describedby="fl-pack-create-hint">
          {text.create.label}
        </Button>
        <p id="fl-pack-create-hint" className="fl-pack__hint">
          {text.create.unavailable}
        </p>
      </section>
    </div>
  );
}

interface DescriptionFieldProps {
  value: string;
  error: string | null;
  onChange: (value: string) => void;
}

function DescriptionField({ value, error, onChange }: DescriptionFieldProps) {
  const id = useId();
  const text = t.podcasts.pack.metadata;
  return (
    <div className="fl-field">
      <label className="fl-field__label" htmlFor={id}>
        {text.description}
      </label>
      <textarea
        id={id}
        className="fl-field__input fl-pack__description"
        rows={4}
        value={value}
        aria-describedby={`${id}-hint`}
        aria-invalid={error ? true : undefined}
        onChange={(event) => onChange(event.target.value)}
      />
      <p id={`${id}-hint`} className="fl-field__hint">
        {text.descriptionHint}
      </p>
      <div aria-live="polite">{error && <p className="fl-field__error">{error}</p>}</div>
    </div>
  );
}
