import {
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import {
  type DeviceStories,
  type DeviceStory,
  type DeviceSummary,
  type IndexHealth,
} from "../../api/device";
import { Badge, Banner, Button, EmptyState, Icon, ProgressBar } from "../../components";
import { t } from "../../i18n";
import { ErrorBanner } from "../ErrorBanner";
import { formatBytes } from "../validation";
import { useDevice } from "./DeviceContext";
import { DeviceStoryOrder } from "./DeviceStoryOrder";
import { EjectButton } from "./EjectButton";
import { ForgetCarrierKeysButton } from "./ForgetCarrierKeysButton";
import { RemoveStoryButton } from "./RemoveStoryButton";
import {
  ThumbnailLoader,
  useDeviceStoryTitles,
  useDeviceThumbnail,
} from "./storyDetails";
import "./DeviceScreen.css";

const text = t.deviceScreen;

export interface DeviceScreenProps {
  /** Section affichée sous la liste des histoires de la Flam (envoi depuis « Mes histoires »). */
  libraryPanel?: ReactNode;
}

/** Écran « Ma Flam » : état de la Flam branchée en USB, ses histoires, envoi, retrait et éjection. */
export function DeviceScreen({ libraryPanel }: DeviceScreenProps) {
  const { status, error, retry, device, stories, recoveryNote, ejectNote } = useDevice();
  const count =
    device && stories.phase === "ready" ? stories.report.stories.length : null;

  // Annonces : l'état de la Flam puis le nombre d'histoires, dites ensemble pour que la
  // seconde ne couvre pas la première.
  const [stateNote, setStateNote] = useState("");
  const countNote = count === null ? "" : text.announce.stories(count);
  const announcement = [stateNote, countNote].filter(Boolean).join(". ");
  const wasDetected = useRef<boolean | null>(null);
  const retried = useRef(false);
  const headingRef = useRef<HTMLHeadingElement>(null);

  // Annonce les changements d'état (pas l'état initial « non détecté », déjà visible).
  const detected = device !== null;
  const known = status !== null;
  useEffect(() => {
    if (!known) return;
    const before = wasDetected.current;
    wasDetected.current = detected;
    if (before === detected || (before === null && !detected)) return;
    setStateNote(detected ? text.announce.detected : text.announce.lost);
    if (detected && retried.current) headingRef.current?.focus();
  }, [known, detected]);

  // « Réessayer » ne réclame le focus que pour le résultat de l'appel qui suit.
  useEffect(() => {
    retried.current = false;
  }, [status, error]);

  function onRetry() {
    retried.current = true;
    retry();
  }

  return (
    <div className="fl-device-screen">
      <p className="fl-device-screen__intro">{text.intro}</p>
      <p role="status" className="fl-visually-hidden">
        {announcement}
      </p>
      {ejectNote && (
        <Banner
          tone={ejectNote.tone}
          role={ejectNote.tone === "danger" ? "alert" : "status"}
        >
          {ejectNote.message}
        </Banner>
      )}
      {error && (
        <ErrorBanner
          error={error}
          action={<Button onClick={onRetry}>{t.errors.retry}</Button>}
        />
      )}
      {!status && !error && <p>{text.searching}</p>}
      {status && !device && (
        <EmptyState
          icon="device"
          title={text.notDetected.title}
          description={text.notDetected.description}
          action={
            <Button variant="primary" icon="refresh" onClick={onRetry}>
              {text.notDetected.retry}
            </Button>
          }
        />
      )}
      {status && device && (
        <>
          <DeviceCard
            device={device}
            count={status.device_count}
            headingRef={headingRef}
          />
          {recoveryNote && (
            <Banner tone={recoveryNote.tone} role="status">
              {recoveryNote.message}
            </Banner>
          )}
          <div className="fl-device-screen__toolbar">
            <EjectButton />
            <ForgetCarrierKeysButton />
          </div>
          <DeviceStoriesList key={`${device.root}|${device.volume_label}`} />
          {libraryPanel}
        </>
      )}
    </div>
  );
}

function warningTexts(device: DeviceSummary): string[] {
  const messages = device.warnings.map((warning) => {
    switch (warning.kind) {
      case "missing_str_dir":
        return text.warnings.missingStr;
      case "missing_library_dir":
        return text.warnings.missingLibrary;
      case "unexpected_mdf_size":
        return text.warnings.unexpectedMdf(warning.size);
    }
  });
  if (device.mdf_problem?.kind === "truncated") messages.push(text.warnings.mdfTruncated);
  if (device.mdf_problem?.kind === "too_large") messages.push(text.warnings.mdfTooLarge);
  if (device.backup_failed) messages.push(text.warnings.backupFailed);
  return messages;
}

function DeviceCard({
  device,
  count,
  headingRef,
}: {
  device: DeviceSummary;
  count: number;
  headingRef: RefObject<HTMLHeadingElement | null>;
}) {
  const warnings = warningTexts(device);
  const space = device.space;
  const percent =
    space && space.total > 0 ? ((space.total - space.free) / space.total) * 100 : 0;
  return (
    <section className="fl-device-card" aria-labelledby="fl-device-title">
      <h2
        id="fl-device-title"
        ref={headingRef}
        tabIndex={-1}
        className="fl-device-card__title"
      >
        {text.detected.title}
      </h2>
      <dl className="fl-device-card__facts">
        <div>
          <dt>{text.detected.volume}</dt>
          <dd>
            {device.volume_label || text.detected.unnamedVolume} ({device.root})
          </dd>
        </div>
        {device.file_system && (
          <div>
            <dt>{text.detected.fileSystem}</dt>
            <dd>{device.file_system}</dd>
          </div>
        )}
      </dl>
      {space ? (
        <div className="fl-device-card__space">
          <ProgressBar value={percent} label={text.space.label} showValue />
          <p className="fl-device-card__space-text">
            {text.space.summary(formatBytes(space.free), formatBytes(space.total))}
          </p>
        </div>
      ) : (
        <p className="fl-device-card__space-text">{text.space.unavailable}</p>
      )}
      {count > 1 && (
        <Banner tone="info" role="status">
          {text.detected.several(count)}
        </Banner>
      )}
      {warnings.length > 0 && (
        <Banner tone="warning" title={text.warnings.title}>
          <ul className="fl-device-card__warnings">
            {warnings.map((message) => (
              <li key={message}>{message}</li>
            ))}
          </ul>
        </Banner>
      )}
    </section>
  );
}

type Notice = { tone: "warning" | "danger"; message: string };

const VISIBLE_NOTICES: Record<IndexHealth, string | null> = {
  valid: null,
  absent: text.index.visibleAbsent,
  damaged: text.index.visibleDamaged,
  unreadable: text.index.visibleUnreadable,
};

const HIDDEN_NOTICES: Record<IndexHealth, string | null> = {
  valid: null,
  absent: null,
  damaged: text.index.hiddenDamaged,
  unreadable: text.index.hiddenUnreadable,
};

function indexNotices(report: DeviceStories): Notice[] {
  const notices: Notice[] = [];
  const visible = VISIBLE_NOTICES[report.visible_index];
  if (visible) {
    notices.push({
      tone: report.visible_index === "unreadable" ? "danger" : "warning",
      message: visible,
    });
  }
  const hidden = HIDDEN_NOTICES[report.hidden_index];
  if (hidden) {
    notices.push({
      tone: report.hidden_index === "unreadable" ? "danger" : "warning",
      message: hidden,
    });
  }
  if (report.truncated) {
    notices.push({ tone: "warning", message: text.index.truncated(report.limit) });
  }
  if (report.str_unreadable) {
    notices.push({ tone: "warning", message: text.index.strUnreadable });
  }
  return notices;
}

/** Ordre modifiable : index sûrs et liste complète d'au moins deux histoires visibles. */
function canReorder(report: DeviceStories | null): report is DeviceStories {
  return (
    report !== null &&
    report.visible_index === "valid" &&
    (report.hidden_index === "valid" || report.hidden_index === "absent") &&
    !report.truncated &&
    report.stories.filter((s) => s.placement === "visible").length >= 2
  );
}

/**
 * Liste des histoires de la Flam (lues par le `DeviceProvider`), avec retrait par histoire et
 * modification de l'ordre.
 */
function DeviceStoriesList() {
  const { stories: state, reloadStories, busy, device } = useDevice();
  const root = device?.root ?? null;
  const report = state.phase === "ready" ? state.report : null;
  const uuids = useMemo(
    () => report?.stories.filter((s) => !s.missing).map((s) => s.uuid) ?? [],
    [report],
  );
  // Titres et vignettes de la Flam : chargés après l'affichage de la liste, suspendus pendant
  // un envoi ou un retrait.
  const titles = useDeviceStoryTitles(root, uuids, busy);
  const loader = useMemo(() => (root ? new ThumbnailLoader(root) : null), [root]);
  useEffect(() => () => loader?.dispose(), [loader]);
  useEffect(() => loader?.setPaused(busy), [loader, busy]);
  const wrapperRef = useRef<HTMLDivElement>(null);
  const orderButtonRef = useRef<HTMLButtonElement>(null);
  const [ordering, setOrdering] = useState(false);
  const reorderable = canReorder(report);
  // Liste relue non modifiable (index abîmé, tronquée…) : l'édition de l'ordre se ferme.
  if (ordering && !reorderable) setOrdering(false);
  const showOrder = ordering && reorderable;
  const orderHintId = useId();
  // Après un retrait, la ligne (donc le bouton) disparaît à la relecture : le focus passe à la
  // liste dès que la nouvelle liste est affichée.
  const focusAfterReload = useRef(false);
  const [removing, setRemoving] = useState<string | null>(null);
  useEffect(() => {
    if (focusAfterReload.current && state.phase === "ready") {
      focusAfterReload.current = false;
      wrapperRef.current?.focus();
    }
  }, [state]);

  // Fin de l'édition de l'ordre : le focus revient au bouton « Modifier l'ordre » réaffiché.
  const focusOrderButton = useRef(false);
  useEffect(() => {
    if (showOrder || !focusOrderButton.current) return;
    focusOrderButton.current = false;
    (orderButtonRef.current ?? wrapperRef.current)?.focus();
  }, [showOrder]);

  function closeOrder() {
    focusOrderButton.current = true;
    setOrdering(false);
  }

  const nameOf = (story: DeviceStory) =>
    titles.get(story.uuid) ?? story.title ?? text.stories.untitled(story.short_id);

  function refresh() {
    // Le bouton cliqué disparaît pendant le chargement : le focus passe à un élément stable.
    wrapperRef.current?.focus();
    reloadStories();
  }

  const renderStories = () => {
    if (state.phase === "error") {
      return (
        <ErrorBanner
          error={{
            ...state.error,
            message: `${text.stories.loadFailed} ${state.error.message}`,
          }}
          action={<Button onClick={refresh}>{t.errors.retry}</Button>}
        />
      );
    }
    if (state.phase === "loading") return <p>{text.stories.loading}</p>;

    const { report } = state;
    return (
      <section className="fl-device-stories" aria-labelledby="fl-device-stories-title">
        <div className="fl-device-stories__head">
          <h2 id="fl-device-stories-title" className="fl-device-card__title">
            {text.stories.title(report.stories.length)}
          </h2>
          {!showOrder && (
            <div className="fl-device-row__actions">
              {reorderable && (
                <span className="fl-send">
                  <Button
                    ref={orderButtonRef}
                    variant="secondary"
                    size="sm"
                    icon="sliders"
                    className="fl-send__button"
                    aria-disabled={busy || undefined}
                    aria-describedby={busy ? orderHintId : undefined}
                    onClick={() => {
                      if (!busy) setOrdering(true);
                    }}
                  >
                    {text.order.edit}
                  </Button>
                  {busy && (
                    <span id={orderHintId} className="fl-send__reason">
                      {text.order.busy}
                    </span>
                  )}
                </span>
              )}
              <Button variant="secondary" size="sm" icon="refresh" onClick={refresh}>
                {text.stories.refresh}
              </Button>
            </div>
          )}
        </div>
        {indexNotices(report).map(({ tone, message }) => (
          <Banner key={message} tone={tone} role="status">
            {message}
          </Banner>
        ))}
        {showOrder ? (
          <DeviceStoryOrder
            stories={report.stories.filter((s) => s.placement === "visible")}
            nameOf={nameOf}
            hasOthers={report.stories.some((s) => s.placement !== "visible")}
            busy={busy}
            onClose={closeOrder}
          />
        ) : report.stories.length === 0 ? (
          <EmptyState
            icon="book"
            headingLevel={3}
            title={text.stories.emptyTitle}
            description={text.stories.emptyDescription}
          />
        ) : (
          <ul className="fl-device-rows" aria-label={text.stories.listLabel}>
            {report.stories.map((story) => (
              <DeviceStoryRow
                key={story.uuid}
                story={story}
                deviceTitle={titles.get(story.uuid)}
                loader={story.missing ? null : loader}
                removing={removing === story.uuid}
                actions={
                  <RemoveStoryButton
                    story={story}
                    busy={busy}
                    locked={removing !== null && removing !== story.uuid}
                    onWorkingChange={(working) =>
                      setRemoving((current) =>
                        working ? story.uuid : current === story.uuid ? null : current,
                      )
                    }
                    onRemoved={() => {
                      focusAfterReload.current = true;
                    }}
                  />
                }
              />
            ))}
          </ul>
        )}
      </section>
    );
  };

  return (
    <div ref={wrapperRef} tabIndex={-1} className="fl-device-stories-wrap">
      {renderStories()}
    </div>
  );
}

/**
 * Ligne d'une histoire. Titre : celui déchiffré sur la Flam (`deviceTitle`), sinon celui de la
 * bibliothèque, sinon l'UUID court. Vignette : celle de la Flam, sinon l'illustration par défaut.
 */
function DeviceStoryRow({
  story,
  deviceTitle,
  loader,
  actions,
  removing = false,
}: {
  story: DeviceStory;
  deviceTitle?: string | null;
  loader: ThumbnailLoader | null;
  actions?: ReactNode;
  /** Retrait en cours : la ligne est grisée. */
  removing?: boolean;
}) {
  const rowRef = useRef<HTMLLIElement>(null);
  const thumb = useDeviceThumbnail(loader, story.uuid, rowRef);
  const title = deviceTitle ?? story.title;
  const name = title ?? text.stories.untitled(story.short_id);
  return (
    <li ref={rowRef} className="fl-device-row" aria-busy={removing || undefined}>
      <span className="fl-device-row__thumb">
        {thumb ? <img src={thumb} alt="" /> : <Icon name="book" size={20} />}
      </span>
      <div className="fl-device-row__text">
        <span className="fl-device-row__name">{name}</span>
        {title && (
          <span className="fl-device-row__detail">
            {text.stories.idDetail(story.short_id)}
          </span>
        )}
      </div>
      {story.placement === "hidden" && (
        <Badge tone="secondary">{text.stories.hidden}</Badge>
      )}
      {story.placement === "unlisted" && (
        <Badge tone="warning" icon="alert">
          {text.stories.unlisted}
        </Badge>
      )}
      {story.missing && (
        <Badge tone="danger" icon="alert">
          {text.stories.missing}
        </Badge>
      )}
      {actions && <div className="fl-device-row__actions">{actions}</div>}
    </li>
  );
}
