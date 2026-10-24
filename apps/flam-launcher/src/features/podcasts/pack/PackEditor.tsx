import {
  useCallback,
  useContext,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { toAppError, type AppError } from "../../../api/error";
import { createPack } from "../../../api/install";
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
  Dialog,
  EmptyState,
  SelectField,
  TextField,
  type SelectOption,
} from "../../../components";
import { ToastContext } from "../../../components/Toast/ToastContext";
import { t } from "../../../i18n";
import { libraryHash, ROUTE_HASHES } from "../../../router";
import { ErrorBanner } from "../../ErrorBanner";
import { getPackRightsAck, setPackRightsAck } from "../../settings/preferences";
import { StoriesContext } from "../../stories/StoriesContext";
import { taskErrorMessage } from "../../stories/taskView";
import { formatBytes } from "../../validation";
import { MAX_EPISODES } from "../selection";
import { usePodcastSelection } from "../SelectionContext";
import {
  settlePendingSave,
  useDebouncedValidation,
  useDraftAutosave,
  useFeedInfo,
} from "./hooks";
import {
  beginCreation,
  creationsVersion,
  forgetFinishedTasks,
  isCreating,
  rememberTask,
  subscribeCreations,
} from "./creations";
import { PackTree, menuLabel } from "./PackTree";
import { TitlesAudioField } from "./TitlesAudioField";
import {
  MAX_MENU_DEPTH,
  MAX_LABEL_CHARS,
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
        return text.invalidCategoryName(MAX_LABEL_CHARS);
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
  const announceToggle = useRef(false);
  // Un message identique au précédent (refus répétés) n'est pas relu par le lecteur d'écran :
  // une espace insécable en fin de message, alternée, force la nouvelle annonce.
  const announce = useCallback((message: string) => {
    announceToggle.current = !announceToggle.current;
    setAnnouncement(announceToggle.current ? message : `${message}\u00a0`);
  }, []);
  const {
    status,
    init: initDraft,
    schedule,
    retry,
    settle,
  } = useDraftAutosave(saveDelayMs);
  const toast = useContext(ToastContext);
  // Contextes absents hors de l'application (éditeur rendu seul) : la tâche apparaît alors avec ses
  // premiers événements.
  const stories = useContext(StoriesContext);
  const trackTask = stories?.trackTask;
  const panelTasks = stories?.panelTasks;
  const [creating, setCreating] = useState(false);
  const [createError, setCreateError] = useState<AppError | null>(null);
  // Problèmes renvoyés par le backend à la création (et la composition qu'ils décrivent).
  const [refused, setRefused] = useState<{
    problems: Problem[];
    menu: PackNode[];
  } | null>(null);
  const [rightsOpen, setRightsOpen] = useState(false);
  // Relit l'état des envois en cours quand une autre instance de l'éditeur les termine.
  useSyncExternalStore(subscribeCreations, creationsVersion);
  const [loadedRevision, setLoadedRevision] = useState<number | null>(null);
  // `edited` : modifié depuis la lecture, donc plus le brouillon d'une création en cours (#429).
  // Il passe à vrai à toute modification, même annulée ensuite : les contenus ne sont pas comparés.
  const [edited, setEdited] = useState(false);
  const createLock = useRef(false);
  const isTaskActive = (taskId: string) =>
    panelTasks?.some(
      (task) =>
        task.id === taskId && (task.state === "queued" || task.state === "running"),
    ) ?? false;
  // Les tâches terminées sortent du registre des créations (#435).
  useEffect(() => {
    forgetFinishedTasks(
      (taskId) =>
        panelTasks?.some(
          (task) =>
            task.id === taskId &&
            (task.state === "done" ||
              task.state === "error" ||
              task.state === "cancelled"),
        ) ?? false,
    );
  }, [panelTasks]);
  const validation = useDebouncedValidation(composition, validateDelayMs);
  const seedValidation = validation.seed;

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        // Un éditeur fermé à l'instant a pu laisser une sauvegarde en cours : la lire d'abord.
        await settlePendingSave();
        if (cancelled) return;
        const view = await readPackDraft();
        if (cancelled) return;
        initDraft(view.revision);
        setLoadedRevision(view.revision);
        setEdited(false);
        const loaded =
          view.composition ??
          emptyComposition(
            initialSelection.current.map((s) =>
              episodeNode({ source_id: s.sourceId, episode_id: s.id }),
            ),
          );
        if (view.composition && view.report)
          seedValidation(view.composition, view.report);
        setComposition(loaded);
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
  }, [attempt, initDraft, seedValidation]);

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
      (feeds.has(ref.source_id) ? null : t.podcasts.pack.tree.loadingTitle),
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
    setEdited(true);
    setRefused(null);
    setCreateError(null);
    schedule(next);
    if (message !== "") announce(message);
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
    return problem
      ? problemText(problem, validation.reportFor?.menu ?? current.menu)
      : null;
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
  // Brouillon inchangé dont la création tourne encore (éditeur quitté puis rouvert, #429).
  const alreadyCreating =
    !edited && loadedRevision !== null && isCreating(loadedRevision, isTaskActive);
  const canCreate =
    !creating &&
    !alreadyCreating &&
    validation.error === null &&
    !validation.pending &&
    report !== null &&
    report.problems.length === 0;

  /** Création : première fois, le message de droits (D14) passe d'abord. */
  function requestCreate() {
    if (!canCreate) return;
    if (getPackRightsAck()) void create();
    else setRightsOpen(true);
  }

  async function create() {
    if (createLock.current) return;
    createLock.current = true;
    setCreating(true);
    setCreateError(null);
    setRefused(null);
    let endCreation: (() => void) | undefined;
    try {
      // Une modification faite pendant l'enregistrement fait partie de la révision renvoyée :
      // c'est elle qui part, pas la composition affichée au moment du clic.
      const saved = await settle();
      const submitted = saved.composition ?? current;
      // Verrou durable : une autre instance de l'éditeur (quitté puis rouvert) peut déjà créer
      // ce brouillon.
      // Sortie sans message : une autre instance crée déjà ce brouillon (le bouton le dit).
      if (isCreating(saved.revision, isTaskActive)) return;
      endCreation = beginCreation(saved.revision);
      const created = await createPack(submitted, saved.revision);
      if (created.task === null) {
        setRefused({ problems: created.report.problems, menu: submitted.menu });
        return;
      }
      rememberTask(created.task.id, saved.revision);
      trackTask?.(created.task);
      selection.removeMany(
        episodesOf(submitted.menu).map((e) => ({
          sourceId: e.source_id,
          id: e.episode_id,
        })),
      );
      // Composition conservée sous un nom (#461) : dite dans le même message.
      const record = created.composition;
      const kept = record?.name
        ? ` ${text.create.compositionSaved(record.name, record.new)}`
        : record?.error
          ? ` ${text.create.compositionNotSaved(record.error)}`
          : "";
      const notice = record?.notice ? ` ${record.notice}` : "";
      // Composition non conservée ou fichier mis de côté : le message reste jusqu'à
      // fermeture (#484), il ne disparaît pas pendant le passage à la bibliothèque.
      const lasting = Boolean(record?.error || record?.notice);
      toast?.show({
        message: `${text.create.started(created.task.entry.title)}${kept}${notice}`,
        ...(lasting ? { tone: "info", duration: 0 } : { tone: "success" }),
      });
      window.location.hash = libraryHash();
    } catch (caught) {
      setCreateError(toAppError(caught));
    } finally {
      endCreation?.();
      createLock.current = false;
      setCreating(false);
    }
  }
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
        <div>
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
          <Banner
            tone="danger"
            action={
              <Button variant="secondary" size="sm" onClick={retry}>
                {t.errors.retry}
              </Button>
            }
          >
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
                <li key={i}>
                  {problemText(problem, validation.reportFor?.menu ?? current.menu)}
                </li>
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
        <p className="fl-pack__hint">{text.tree.hint(MAX_LABEL_CHARS)}</p>
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
          hasImage={(ref) =>
            feeds.get(ref.source_id)?.episodes.get(ref.episode_id)?.has_image === true
          }
          onChange={(next, message) => {
            edit({ ...current, menu: next }, message);
          }}
          onRefuse={announce}
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
          hint={text.metadata.titleHint(single?.title ?? null, feedIds.length > 1)}
          value={current.metadata.title}
          maxLength={MAX_TITLE_CHARS}
          error={fieldError("title")}
          onChange={(event) => editMetadata({ title: event.target.value })}
        />
        <TextField
          label={text.metadata.subtitle}
          hint={text.metadata.subtitleHint(
            current.metadata.subtitle === null && Boolean(single?.copyright?.trim()),
          )}
          value={
            current.metadata.subtitle ??
            [...(single?.copyright?.trim() ?? "")].slice(0, MAX_TITLE_CHARS).join("")
          }
          maxLength={MAX_TITLE_CHARS}
          error={fieldError("subtitle")}
          onChange={(event) =>
            editMetadata({
              // Vide avec un copyright par défaut : sous-titre explicitement vide (D16), sans
              // quoi le défaut serait réimposé ; sans copyright, `null` et "" sont équivalents.
              subtitle:
                event.target.value === "" && !single?.copyright
                  ? null
                  : event.target.value,
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
          onChange={(value) =>
            editMetadata({ description: value.trim() === "" ? null : value })
          }
        />
        <TitlesAudioField
          checked={current.titles_audio === true}
          voice={current.titles_voice ?? null}
          previewText={
            current.metadata.title.trim() ||
            (report?.metadata?.title ?? single?.title ?? "")
          }
          onChange={(checked) => edit({ ...current, titles_audio: checked })}
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
        <Button
          variant="primary"
          disabled={!canCreate}
          aria-describedby={canCreate || creating ? undefined : "fl-pack-create-hint"}
          onClick={requestCreate}
        >
          {creating ? text.create.creating : text.create.label}
        </Button>
        {!canCreate && !creating && (
          <p id="fl-pack-create-hint" className="fl-pack__hint">
            {alreadyCreating
              ? text.create.inProgress
              : validation.error
                ? text.create.checkFailed
                : report === null || validation.pending
                  ? text.create.checking
                  : text.create.blocked}
          </p>
        )}
        {createError && (
          <ErrorBanner
            error={{
              ...createError,
              message: taskErrorMessage({
                kind: createError.kind,
                message: createError.message,
                code: null,
              }),
            }}
          />
        )}
        {refused && (
          <Banner tone="danger">
            <p>{text.create.refused}</p>
            <ul className="fl-pack__problems">
              {refused.problems.map((problem, i) => (
                <li key={i}>{problemText(problem, refused.menu)}</li>
              ))}
            </ul>
          </Banner>
        )}
      </section>
      <Dialog
        open={rightsOpen}
        onClose={() => setRightsOpen(false)}
        title={text.rights.title}
        footer={
          <>
            <Button variant="secondary" onClick={() => setRightsOpen(false)}>
              {text.rights.cancel}
            </Button>
            <Button
              variant="primary"
              onClick={() => {
                setPackRightsAck();
                setRightsOpen(false);
                void create();
              }}
            >
              {text.rights.accept}
            </Button>
          </>
        }
      >
        <p>{text.rights.text}</p>
      </Dialog>
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
        aria-describedby={error ? `${id}-hint ${id}-error` : `${id}-hint`}
        aria-invalid={error ? true : undefined}
        onChange={(event) => onChange(event.target.value)}
      />
      <p id={`${id}-hint`} className="fl-field__hint">
        {text.descriptionHint}
      </p>
      <div aria-live="polite">
        {error && (
          <p id={`${id}-error`} className="fl-field__error">
            {error}
          </p>
        )}
      </div>
    </div>
  );
}
